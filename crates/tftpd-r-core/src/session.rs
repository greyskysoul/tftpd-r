//! One RRQ transfer: option negotiation, DATA/ACK exchange, retransmission and
//! event reporting.
//!
//! The session owns its own UDP socket (transfer ID). All datagrams from a
//! different address get RFC 1350 ERROR 5 (rate limited); packets from the
//! expected peer that are out of order are ignored so retransmissions stay
//! idempotent.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::config::ServerConfig;
use crate::error::ErrorCode;
use crate::events::{sanitize_display_name, EventKind, EventSink, SessionId, TftpEvent};
use crate::filesystem::Sandbox;
use crate::protocol::{self, options, Mode, Packet, Request};

/// Interval between progress events for one transfer.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// Maximum number of ERROR 5 responses sent for one block exchange.
const MAX_UNKNOWN_TID_RESPONSES: u32 = 8;

/// Signal from the accept loop to a running session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionSignal {
    /// The client retransmitted its request; resend the current packet.
    Resend,
}

/// Immutable per-session context supplied by the server.
pub(crate) struct SessionContext {
    pub(crate) id: SessionId,
    pub(crate) peer: SocketAddr,
    pub(crate) config: Arc<ServerConfig>,
    pub(crate) sandbox: Arc<Sandbox>,
    pub(crate) events: Arc<dyn EventSink>,
    pub(crate) cancel: CancellationToken,
}

/// A failed transfer, with both a client-visible message and an internal reason.
struct Failure {
    code: ErrorCode,
    message: &'static str,
    reason: String,
    notify_peer: bool,
}

impl Failure {
    fn timeout() -> Self {
        Self {
            code: ErrorCode::Undefined,
            message: "Transfer timed out",
            reason: "timed out waiting for acknowledgement".to_string(),
            notify_peer: true,
        }
    }

    fn cancelled() -> Self {
        Self {
            code: ErrorCode::Undefined,
            message: "Server shutting down",
            reason: "cancelled by server shutdown".to_string(),
            notify_peer: true,
        }
    }

    fn peer(code: ErrorCode) -> Self {
        Self {
            code,
            message: "Peer reported an error",
            reason: format!("peer reported TFTP error {}", code.as_u16()),
            notify_peer: false,
        }
    }

    fn socket(error: &io::Error) -> Self {
        Self {
            code: ErrorCode::Undefined,
            message: "Transfer failed",
            reason: format!("socket error ({:?})", error.kind()),
            notify_peer: false,
        }
    }

    fn file(error: io::Error) -> Self {
        Self {
            code: ErrorCode::Undefined,
            message: "Transfer failed",
            reason: format!("file read error ({:?})", error.kind()),
            notify_peer: true,
        }
    }

    fn internal(reason: &'static str) -> Self {
        Self {
            code: ErrorCode::Undefined,
            message: "Transfer failed",
            reason: reason.to_string(),
            notify_peer: true,
        }
    }
}

/// Drive one RRQ transfer to completion.
pub(crate) async fn run_rrq(
    socket: UdpSocket,
    request: Request,
    context: SessionContext,
    mut signals: mpsc::Receiver<SessionSignal>,
) {
    let peer = context.peer;
    let display_name = sanitize_display_name(&request.filename);

    if request.mode != Mode::Octet {
        let message = match request.mode {
            Mode::NetAscii => "netascii mode is not supported",
            Mode::Mail => "mail mode is not supported",
            Mode::Octet => unreachable!(),
        };
        reject(
            &socket,
            peer,
            ErrorCode::Undefined,
            message,
            &context,
            display_name,
        )
        .await;
        return;
    }

    let opened = match context.sandbox.open_for_read(&request.filename).await {
        Ok(opened) => opened,
        Err(error) => {
            let reason = error.to_string();
            reject_with_reason(
                &socket,
                peer,
                error.error_code(),
                error.client_message(),
                reason,
                &context,
                display_name,
            )
            .await;
            return;
        }
    };
    let total = opened.size;
    let display_name = opened.display_name;
    let mut file = opened.file;

    let negotiated = match options::negotiate(
        &request.options,
        context.config.max_block_size(),
        context.config.timeout(),
        Some(total),
    ) {
        Ok(negotiated) => negotiated,
        Err(error) => {
            reject_with_reason(
                &socket,
                peer,
                error.error_code(),
                "Option negotiation failed",
                error.to_string(),
                &context,
                display_name,
            )
            .await;
            return;
        }
    };

    let started = Instant::now();
    let deadline = started + context.config.max_transfer_duration();
    let timeout = negotiated.timeout;
    let block_size = negotiated.block_size;
    let max_block_size = context.config.max_block_size();

    context.events.emit(TftpEvent::new(
        context.id,
        peer,
        display_name.clone(),
        EventKind::Started,
        0,
        Some(total),
    ));

    let mut unknown_tid_responses = 0u32;
    let mut exchange = Exchange {
        socket: &socket,
        peer,
        timeout,
        max_retries: context.config.max_retries,
        deadline,
        max_block_size,
        cancel: &context.cancel,
        signals: &mut signals,
        signals_open: true,
        unknown_tid_responses: &mut unknown_tid_responses,
    };

    if negotiated.needs_oack() {
        let oack = Packet::Oack(negotiated.oack.clone());
        let encoded = match protocol::encode(&oack) {
            Ok(encoded) => encoded,
            Err(error) => {
                finish_failure(
                    &socket,
                    peer,
                    Failure::internal("internal encoding failure"),
                    &context,
                    display_name,
                    Some(total),
                    0,
                )
                .await;
                debug!(session = context.id, %error, "failed to encode OACK");
                return;
            }
        };
        if let Err(failure) = exchange.send_and_wait(&encoded, 0).await {
            finish_failure(
                &socket,
                peer,
                failure,
                &context,
                display_name,
                Some(total),
                0,
            )
            .await;
            return;
        }
    }

    let mut block: u16 = 1;
    let mut bytes_sent: u64 = 0;
    let mut last_progress = Instant::now();

    loop {
        if Instant::now() >= deadline {
            finish_failure(
                &socket,
                peer,
                Failure::timeout(),
                &context,
                display_name,
                Some(total),
                bytes_sent,
            )
            .await;
            return;
        }
        let payload = match read_block(&mut file, block_size).await {
            Ok(payload) => payload,
            Err(error) => {
                finish_failure(
                    &socket,
                    peer,
                    Failure::file(error),
                    &context,
                    display_name,
                    Some(total),
                    bytes_sent,
                )
                .await;
                return;
            }
        };
        let payload_len = payload.len();
        let is_final = payload_len < block_size;
        let packet = Packet::Data {
            block,
            data: payload,
        };
        let encoded = match protocol::encode(&packet) {
            Ok(encoded) => encoded,
            Err(error) => {
                debug!(session = context.id, %error, "failed to encode DATA");
                finish_failure(
                    &socket,
                    peer,
                    Failure::internal("internal encoding failure"),
                    &context,
                    display_name,
                    Some(total),
                    bytes_sent,
                )
                .await;
                return;
            }
        };
        if let Err(failure) = exchange.send_and_wait(&encoded, block).await {
            finish_failure(
                &socket,
                peer,
                failure,
                &context,
                display_name,
                Some(total),
                bytes_sent,
            )
            .await;
            return;
        }
        bytes_sent += u64::try_from(payload_len).unwrap_or(0);
        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            context.events.emit(TftpEvent::new(
                context.id,
                peer,
                display_name.clone(),
                EventKind::Progress,
                bytes_sent,
                Some(total),
            ));
            last_progress = Instant::now();
        }
        if is_final {
            break;
        }
        block = block.wrapping_add(1);
    }

    context.events.emit(TftpEvent::new(
        context.id,
        peer,
        display_name,
        EventKind::Completed,
        bytes_sent,
        Some(total),
    ));
    debug!(session = context.id, %peer, bytes = bytes_sent, "transfer completed");
}

/// DATA/ACK exchange for one block, including retransmission.
struct Exchange<'a> {
    socket: &'a UdpSocket,
    peer: SocketAddr,
    timeout: Duration,
    max_retries: u32,
    deadline: Instant,
    max_block_size: usize,
    cancel: &'a CancellationToken,
    signals: &'a mut mpsc::Receiver<SessionSignal>,
    signals_open: bool,
    unknown_tid_responses: &'a mut u32,
}

impl Exchange<'_> {
    async fn send_and_wait(&mut self, encoded: &[u8], expected_ack: u16) -> Result<(), Failure> {
        let mut attempts: u32 = 0;
        let mut buffer = vec![0u8; 65_536];
        loop {
            if Instant::now() >= self.deadline {
                return Err(Failure::timeout());
            }
            self.socket
                .send_to(encoded, self.peer)
                .await
                .map_err(|error| Failure::socket(&error))?;
            loop {
                if Instant::now() >= self.deadline {
                    return Err(Failure::timeout());
                }
                let wait = self
                    .timeout
                    .min(self.deadline.saturating_duration_since(Instant::now()));
                tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => return Err(Failure::cancelled()),
                    () = tokio::time::sleep(wait) => break,
                    signal = self.signals.recv(), if self.signals_open => {
                        if signal.is_none() {
                            self.signals_open = false;
                        }
                        break;
                    }
                    received = self.socket.recv_from(&mut buffer) => {
                        let (length, from) = match received {
                            Ok(value) => value,
                            Err(error) => {
                                if is_transient_udp_error(&error) {
                                    continue;
                                }
                                return Err(Failure::socket(&error));
                            }
                        };
                        if from != self.peer {
                            self.reject_unknown_tid(from).await;
                            continue;
                        }
                        match protocol::decode(&buffer[..length], self.max_block_size) {
                            Ok(Packet::Ack { block }) if block == expected_ack => return Ok(()),
                            Ok(Packet::Ack { .. }) => {
                                // Duplicate or stale ACK: ignore and keep waiting.
                            }
                            Ok(Packet::Error { code, .. }) => return Err(Failure::peer(code)),
                            Ok(Packet::Rrq(_)) => break,
                            Ok(_) => {
                                // DATA/OACK/WRQ sent to a transfer socket is not
                                // meaningful; ignore it rather than creating an
                                // error-packet loop.
                            }
                            Err(_) => {
                                // Malformed datagrams are ignored; the retry
                                // timer provides a bounded escape.
                            }
                        }
                    }
                }
            }
            attempts = attempts.saturating_add(1);
            if attempts > self.max_retries {
                warn!(
                    peer = %self.peer,
                    expected_ack,
                    attempts,
                    "giving up after retransmissions"
                );
                return Err(Failure::timeout());
            }
        }
    }

    async fn reject_unknown_tid(&mut self, from: SocketAddr) {
        if *self.unknown_tid_responses >= MAX_UNKNOWN_TID_RESPONSES {
            return;
        }
        *self.unknown_tid_responses += 1;
        let packet = Packet::Error {
            code: ErrorCode::UnknownTransferId,
            message: ErrorCode::UnknownTransferId.default_message().to_string(),
        };
        if let Ok(encoded) = protocol::encode(&packet) {
            let _ = self.socket.send_to(&encoded, from).await;
        }
    }
}

fn is_transient_udp_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused
    )
}

async fn read_block(file: &mut tokio::fs::File, block_size: usize) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0u8; block_size];
    let mut filled = 0;
    while filled < block_size {
        let read = file.read(&mut buffer[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    Ok(buffer)
}

async fn reject(
    socket: &UdpSocket,
    peer: SocketAddr,
    code: ErrorCode,
    message: &'static str,
    context: &SessionContext,
    display_name: String,
) {
    reject_with_reason(
        socket,
        peer,
        code,
        message,
        message.to_string(),
        context,
        display_name,
    )
    .await;
}

async fn reject_with_reason(
    socket: &UdpSocket,
    peer: SocketAddr,
    code: ErrorCode,
    message: &'static str,
    reason: String,
    context: &SessionContext,
    display_name: String,
) {
    send_error(socket, peer, code, message).await;
    context.events.emit(TftpEvent::new(
        context.id,
        peer,
        display_name,
        EventKind::Rejected { reason },
        0,
        None,
    ));
}

async fn send_error(socket: &UdpSocket, peer: SocketAddr, code: ErrorCode, message: &str) {
    let packet = Packet::Error {
        code,
        message: message.to_string(),
    };
    if let Ok(encoded) = protocol::encode(&packet) {
        let _ = socket.send_to(&encoded, peer).await;
    }
}

async fn finish_failure(
    socket: &UdpSocket,
    peer: SocketAddr,
    failure: Failure,
    context: &SessionContext,
    display_name: String,
    total: Option<u64>,
    bytes_sent: u64,
) {
    if failure.notify_peer {
        let packet = Packet::Error {
            code: failure.code,
            message: failure.message.to_string(),
        };
        if let Ok(encoded) = protocol::encode(&packet) {
            // Best effort: never let a dead peer stall shutdown.
            let _ =
                tokio::time::timeout(Duration::from_millis(200), socket.send_to(&encoded, peer))
                    .await;
        }
    }
    context.events.emit(TftpEvent::new(
        context.id,
        peer,
        display_name,
        EventKind::Failed {
            reason: failure.reason,
        },
        bytes_sent,
        total,
    ));
}
