//! UDP server: listening sockets, request validation, TID-per-session
//! spawning, concurrency limits and bounded graceful shutdown.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::ServerConfig;
use crate::error::{DecodeError, ErrorCode, ServerError};
use crate::events::{sanitize_display_name, EventKind, EventSink, SessionId, TftpEvent};
use crate::filesystem::Sandbox;
use crate::protocol::{self, Packet};
use crate::session::{self, SessionContext, SessionSignal};

/// Receive buffer size. A TFTP datagram can never exceed the UDP maximum.
pub const MAX_DATAGRAM_BYTES: usize = 65_535;

/// Per-peer concurrent session cap in addition to the global limit.
const MAX_SESSIONS_PER_PEER: usize = 8;

/// Observable state of one server instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    /// A start request is in progress.
    Starting,
    /// Accepting requests on every address in `local_addrs`.
    Running {
        /// Actual bound addresses (port 0 resolved to the OS-assigned port).
        local_addrs: Vec<SocketAddr>,
    },
    /// Shutdown has been requested; active transfers are draining.
    Stopping,
    /// Fully stopped, no tasks remain.
    Stopped,
    /// The server task failed unexpectedly.
    Failed {
        /// Operator-facing message.
        message: String,
    },
}

impl ServerStatus {
    /// Stable label for UI/CLI.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running { .. } => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed { .. } => "failed",
        }
    }

    /// Bound addresses when running, otherwise an empty slice.
    #[must_use]
    pub fn local_addrs(&self) -> &[SocketAddr] {
        match self {
            Self::Running { local_addrs } => local_addrs,
            _ => &[],
        }
    }

    /// Whether the server accepts requests.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// A successfully bound UDP listening socket and its actual address.
struct BoundListener {
    socket: UdpSocket,
    local: SocketAddr,
}

/// Factory for a running server. There is no global state: every instance owns
/// its sockets, tasks and events.
pub struct TftpServer;

impl TftpServer {
    /// Validate the configuration, bind every listen address and spawn the
    /// accept loops.
    ///
    /// Binding is all-or-nothing: if any configured address cannot be bound,
    /// the already bound sockets are dropped and the classified error is
    /// returned, so a start never leaves a listener serving only a subset of the
    /// configured addresses.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when configuration validation fails, the sandbox
    /// root is unusable or a socket cannot be bound.
    pub async fn start(
        config: ServerConfig,
        events: Arc<dyn EventSink>,
    ) -> Result<ServerHandle, ServerError> {
        config.validate()?;
        let sandbox = Arc::new(Sandbox::new(&config.root_dir, config.max_file_bytes)?);
        let mut port = config.port;
        let mut listeners = Vec::new();
        for ip in config.effective_bind_addresses() {
            let addr = SocketAddr::new(ip, port);
            let socket = UdpSocket::bind(addr)
                .await
                .map_err(|source| ServerError::bind(addr, source))?;
            let local = socket
                .local_addr()
                .map_err(|source| ServerError::bind(addr, source))?;
            // Port 0 means "pick one ephemeral port": the first listener
            // resolves it and the remaining listeners reuse that port, so the
            // whole server keeps a single, consistent port.
            if port == 0 {
                port = local.port();
            }
            listeners.push(BoundListener { socket, local });
        }
        let local_addrs: Vec<SocketAddr> =
            listeners.iter().map(|listener| listener.local).collect();
        let config = Arc::new(config);
        // The sockets are already bound, so the initial observable state is
        // `Running`; the supervisor re-announces it once it is scheduled.
        let (status_tx, status_rx) = watch::channel(ServerStatus::Running {
            local_addrs: local_addrs.clone(),
        });
        let cancel = CancellationToken::new();
        let join = tokio::spawn(run_server(
            listeners,
            config.clone(),
            sandbox,
            events.clone(),
            cancel.clone(),
            status_tx.clone(),
        ));
        Ok(ServerHandle {
            status_rx,
            status_tx,
            cancel,
            join: tokio::sync::Mutex::new(Some(join)),
            local_addrs,
            config,
            events,
        })
    }
}

/// Handle to a running server.
///
/// Dropping the handle cancels the accept loops; call [`ServerHandle::stop`] to
/// wait until all sessions have drained (bounded by `shutdown_grace_secs`).
pub struct ServerHandle {
    status_rx: watch::Receiver<ServerStatus>,
    status_tx: watch::Sender<ServerStatus>,
    cancel: CancellationToken,
    join: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    local_addrs: Vec<SocketAddr>,
    config: Arc<ServerConfig>,
    events: Arc<dyn EventSink>,
}

impl ServerHandle {
    /// Current status.
    #[must_use]
    pub fn status(&self) -> ServerStatus {
        self.status_rx.borrow().clone()
    }

    /// Watch channel for status changes.
    #[must_use]
    pub fn status_receiver(&self) -> watch::Receiver<ServerStatus> {
        self.status_rx.clone()
    }

    /// Actual bound addresses (port 0 resolved to the OS-assigned port).
    #[must_use]
    pub fn local_addrs(&self) -> &[SocketAddr] {
        &self.local_addrs
    }

    /// Configuration this server was started with.
    #[must_use]
    pub fn config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }

    /// Event sink supplied at start (same `Arc`).
    #[must_use]
    pub fn events(&self) -> Arc<dyn EventSink> {
        self.events.clone()
    }

    /// Stop accepting requests, cancel/drain sessions and wait for the server
    /// task. Idempotent; concurrent callers serialize on the join handle.
    pub async fn stop(&self) {
        let mut guard = self.join.lock().await;
        if guard.is_none() {
            return;
        }
        let _ = self.status_tx.send(ServerStatus::Stopping);
        self.cancel.cancel();
        if let Some(handle) = guard.take() {
            match handle.await {
                Ok(()) => {
                    let current = self.status_rx.borrow().clone();
                    if matches!(current, ServerStatus::Stopping | ServerStatus::Starting) {
                        let _ = self.status_tx.send(ServerStatus::Stopped);
                    }
                }
                Err(error) => {
                    let _ = self.status_tx.send(ServerStatus::Failed {
                        message: format!("server task failed: {error}"),
                    });
                }
            }
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Cancel the accept loops even when the caller forgot to stop; sessions
        // then finish on their own cancellation path.
        self.cancel.cancel();
    }
}

#[derive(Clone)]
struct PeerSession {
    id: SessionId,
    sender: mpsc::Sender<SessionSignal>,
}

/// Remove the peer->session mapping when the session task ends, but only if the
/// map still points at this session (a newer session may have replaced it).
struct PeerGuard {
    peers: Arc<StdMutex<HashMap<SocketAddr, PeerSession>>>,
    peer: SocketAddr,
    id: SessionId,
}

impl Drop for PeerGuard {
    fn drop(&mut self) {
        let mut guard = lock_peers(&self.peers);
        if guard
            .get(&self.peer)
            .is_some_and(|entry| entry.id == self.id)
        {
            guard.remove(&self.peer);
        }
    }
}

fn lock_peers(
    peers: &Arc<StdMutex<HashMap<SocketAddr, PeerSession>>>,
) -> std::sync::MutexGuard<'_, HashMap<SocketAddr, PeerSession>> {
    peers.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Supervise one accept loop per bound listener.
///
/// The peer map, the session counter and the session semaphore are shared, so a
/// peer that retries an RRQ on another listening address still reaches its
/// existing session, and the global/per-peer limits cover every listener.
async fn run_server(
    listeners: Vec<BoundListener>,
    config: Arc<ServerConfig>,
    sandbox: Arc<Sandbox>,
    events: Arc<dyn EventSink>,
    cancel: CancellationToken,
    status_tx: watch::Sender<ServerStatus>,
) {
    let local_addrs: Vec<SocketAddr> = listeners.iter().map(|listener| listener.local).collect();
    let _ = status_tx.send(ServerStatus::Running {
        local_addrs: local_addrs.clone(),
    });
    info!(
        bind = %join_addrs(&local_addrs),
        root = %config.display_root(),
        max_sessions = config.max_sessions,
        block_size = config.block_size,
        "TFTP server listening (read-only RRQ)"
    );

    let semaphore = Arc::new(Semaphore::new(config.max_sessions));
    let peers: Arc<StdMutex<HashMap<SocketAddr, PeerSession>>> =
        Arc::new(StdMutex::new(HashMap::new()));
    let next_session_id = Arc::new(AtomicU64::new(1));
    let mut accept_loops: JoinSet<()> = JoinSet::new();
    for listener in listeners {
        accept_loops.spawn(accept_loop(
            listener,
            config.clone(),
            sandbox.clone(),
            events.clone(),
            cancel.clone(),
            semaphore.clone(),
            peers.clone(),
            next_session_id.clone(),
        ));
    }
    while let Some(joined) = accept_loops.join_next().await {
        if let Err(error) = joined {
            warn!(%error, "accept loop task ended unexpectedly");
        }
    }
    let _ = status_tx.send(ServerStatus::Stopped);
    info!("TFTP server stopped");
}

/// Accept requests on one listening socket and own its session tasks.
#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    listener: BoundListener,
    config: Arc<ServerConfig>,
    sandbox: Arc<Sandbox>,
    events: Arc<dyn EventSink>,
    cancel: CancellationToken,
    semaphore: Arc<Semaphore>,
    peers: Arc<StdMutex<HashMap<SocketAddr, PeerSession>>>,
    next_session_id: Arc<AtomicU64>,
) {
    let local_addr = listener.local;
    let socket = listener.socket;
    let mut sessions: JoinSet<()> = JoinSet::new();
    let mut buffer = vec![0u8; MAX_DATAGRAM_BYTES];

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            joined = sessions.join_next(), if !sessions.is_empty() => {
                if let Some(Err(error)) = joined {
                    warn!(%error, "transfer task ended unexpectedly");
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let (length, peer) = match received {
                    Ok(value) => value,
                    Err(error) => {
                        warn!(%error, "UDP receive error");
                        continue;
                    }
                };
                if length > protocol::MAX_REQUEST_BYTES {
                    send_error(&socket, peer, ErrorCode::IllegalOperation, "Request too large").await;
                    continue;
                }
                match protocol::decode(&buffer[..length], config.max_block_size()) {
                    Ok(Packet::Rrq(request)) => {
                        let existing = lock_peers(&peers).get(&peer).cloned();
                        if let Some(existing) = existing {
                            let _ = existing.sender.try_send(SessionSignal::Resend);
                            debug!(%peer, "duplicate request; asked the active session to resend");
                            continue;
                        }
                        if !spawn_rrq_session(
                            &socket,
                            &config,
                            &sandbox,
                            &events,
                            &cancel,
                            &semaphore,
                            &peers,
                            &mut sessions,
                            local_addr,
                            peer,
                            request,
                            &next_session_id,
                        ).await {
                            continue;
                        }
                    }
                    Ok(Packet::Wrq(request)) => {
                        let id = next_id(&next_session_id);
                        send_error(&socket, peer, ErrorCode::AccessViolation, "Uploads are disabled").await;
                        emit_rejected(&events, id, peer, request.filename, "uploads are disabled".to_string());
                    }
                    Ok(_) => {
                        // ACK/DATA/ERROR/OACK on a listening port are not part
                        // of any transfer; answer with a protocol error.
                        send_error(&socket, peer, ErrorCode::IllegalOperation, "Illegal TFTP operation").await;
                    }
                    Err(DecodeError::UnsupportedMode(mode)) => {
                        let id = next_id(&next_session_id);
                        send_error(&socket, peer, ErrorCode::IllegalOperation, "Unsupported transfer mode").await;
                        emit_rejected(&events, id, peer, String::new(), format!("unsupported mode {mode}"));
                        debug!(%peer, %mode, "rejected unsupported transfer mode");
                    }
                    Err(error) => {
                        send_error(&socket, peer, ErrorCode::IllegalOperation, "Illegal TFTP operation").await;
                        debug!(%peer, %error, "rejected malformed datagram");
                    }
                }
            }
        }
    }

    // The shared token is cancelled by `stop()`; call it again defensively so
    // this loop's sessions always observe cancellation even if the loop is
    // interrupted for another reason.
    cancel.cancel();
    debug!(%local_addr, "accept loop stopped; draining active transfers");
    let deadline = tokio::time::Instant::now() + config.shutdown_grace();
    loop {
        if sessions.is_empty() {
            break;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            warn!(%local_addr, "shutdown grace period elapsed; aborting remaining transfers");
            break;
        }
        match tokio::time::timeout(remaining, sessions.join_next()).await {
            Ok(Some(Err(error))) => warn!(%error, "transfer task failed during shutdown"),
            Ok(Some(Ok(()))) => {}
            Ok(None) => break,
            Err(_) => {
                warn!(%local_addr, "shutdown grace period elapsed; aborting remaining transfers");
                break;
            }
        }
    }
    sessions.shutdown().await;
}

/// Allocate the next globally unique session id.
fn next_id(next_session_id: &AtomicU64) -> SessionId {
    next_session_id.fetch_add(1, Ordering::Relaxed)
}

#[allow(clippy::too_many_arguments)]
async fn spawn_rrq_session(
    socket: &UdpSocket,
    config: &Arc<ServerConfig>,
    sandbox: &Arc<Sandbox>,
    events: &Arc<dyn EventSink>,
    cancel: &CancellationToken,
    semaphore: &Arc<Semaphore>,
    peers: &Arc<StdMutex<HashMap<SocketAddr, PeerSession>>>,
    sessions: &mut JoinSet<()>,
    listener_addr: SocketAddr,
    peer: SocketAddr,
    request: protocol::Request,
    next_session_id: &AtomicU64,
) -> bool {
    let peer_sessions = lock_peers(peers)
        .keys()
        .filter(|addr| addr.ip() == peer.ip())
        .count();
    if peer_sessions >= MAX_SESSIONS_PER_PEER {
        let id = next_id(next_session_id);
        send_error(
            socket,
            peer,
            ErrorCode::Undefined,
            "Too many sessions from this address",
        )
        .await;
        emit_rejected(
            events,
            id,
            peer,
            request.filename,
            "per-peer session limit reached".to_string(),
        );
        return false;
    }
    let Ok(permit) = semaphore.clone().try_acquire_owned() else {
        let id = next_id(next_session_id);
        send_error(socket, peer, ErrorCode::Undefined, "Server busy").await;
        emit_rejected(
            events,
            id,
            peer,
            request.filename,
            "server session limit reached".to_string(),
        );
        return false;
    };
    let session_socket = match UdpSocket::bind(session_bind_address(listener_addr)).await {
        Ok(socket) => socket,
        Err(error) => {
            warn!(%error, %peer, "could not create transfer socket");
            send_error(socket, peer, ErrorCode::Undefined, "Server busy").await;
            let id = next_id(next_session_id);
            emit_rejected(
                events,
                id,
                peer,
                request.filename,
                "could not allocate a transfer socket".to_string(),
            );
            return false;
        }
    };
    let id = next_id(next_session_id);
    let (sender, receiver) = mpsc::channel(4);
    lock_peers(peers).insert(peer, PeerSession { id, sender });
    let guard = PeerGuard {
        peers: peers.clone(),
        peer,
        id,
    };
    let context = SessionContext {
        id,
        peer,
        config: config.clone(),
        sandbox: sandbox.clone(),
        events: events.clone(),
        cancel: cancel.clone(),
    };
    sessions.spawn(async move {
        let _permit = permit;
        let _guard = guard;
        session::run_rrq(session_socket, request, context, receiver).await;
    });
    true
}

/// Bind the per-transfer socket in the same address family as the listener.
///
/// A specific listening address is reused for the session socket, so a
/// multi-homed server answers from the address the client contacted instead of
/// letting the routing table pick another local address. A wildcard listener
/// keeps binding the unspecified address and leaves the source address choice
/// to the OS.
fn session_bind_address(listener: SocketAddr) -> SocketAddr {
    match listener {
        SocketAddr::V4(addr) if !addr.ip().is_unspecified() => {
            SocketAddr::new(IpAddr::V4(*addr.ip()), 0)
        }
        SocketAddr::V6(addr) if !addr.ip().is_unspecified() => {
            SocketAddr::new(IpAddr::V6(*addr.ip()), 0)
        }
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    }
}

/// Comma-separated address list for logs.
fn join_addrs(addrs: &[SocketAddr]) -> String {
    addrs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
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

fn emit_rejected(
    events: &Arc<dyn EventSink>,
    id: SessionId,
    peer: SocketAddr,
    file: String,
    reason: String,
) {
    events.emit(TftpEvent::new(
        id,
        peer,
        sanitize_display_name(&file),
        EventKind::Rejected { reason },
        0,
        None,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_sockets_reuse_specific_listening_addresses() {
        assert_eq!(
            session_bind_address("127.0.0.1:69".parse().expect("addr")),
            "127.0.0.1:0".parse().expect("addr")
        );
        assert_eq!(
            session_bind_address("192.168.1.5:6969".parse().expect("addr")),
            "192.168.1.5:0".parse().expect("addr")
        );
        assert_eq!(
            session_bind_address("[::1]:69".parse().expect("addr")),
            "[::1]:0".parse().expect("addr")
        );
    }

    #[test]
    fn wildcard_listeners_keep_an_unspecified_session_address() {
        assert_eq!(
            session_bind_address("0.0.0.0:69".parse().expect("addr")),
            "0.0.0.0:0".parse().expect("addr")
        );
        assert_eq!(
            session_bind_address("[::]:69".parse().expect("addr")),
            "[::]:0".parse().expect("addr")
        );
    }
}
