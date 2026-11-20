//! End-to-end RRQ integration tests over loopback UDP.
//!
//! Every test starts a real server on an ephemeral port with a temporary root
//! directory and speaks raw TFTP to it. No external tools or network access are
//! involved.

use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tftp_core::events::{EventBus, EventKind, TftpEvent};
use tftp_core::protocol::{decode, encode, Mode, Packet, Request};
use tftp_core::server::ServerStatus;
use tftp_core::{ErrorCode, ServerConfig, ServerHandle, TftpServer};

struct Fixture {
    handle: ServerHandle,
    events: Arc<EventBus>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn start(files: &[(&str, &[u8])]) -> Self {
        let config = ServerConfig {
            timeout_secs: 1,
            max_retries: 2,
            max_sessions: 4,
            shutdown_grace_secs: 2,
            ..ServerConfig::default()
        };
        Self::start_with(config, files).await
    }

    async fn start_with(mut config: ServerConfig, files: &[(&str, &[u8])]) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        for (name, contents) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create fixture directory");
            }
            fs::write(&path, contents).expect("write fixture file");
        }
        config.root_dir = dir.path().to_path_buf();
        config.bind_address = "127.0.0.1:0".parse().expect("bind address");
        let events = EventBus::default_shared();
        let handle = TftpServer::start(config, events.clone())
            .await
            .expect("start server");
        Self {
            handle,
            events,
            _dir: dir,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.handle.local_addr()
    }

    async fn stop(self) {
        self.handle.stop().await;
    }
}

struct TestClient {
    socket: tokio::net::UdpSocket,
}

impl TestClient {
    async fn new(_server: SocketAddr) -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind client");
        Self { socket }
    }

    async fn send(&self, packet: &Packet, destination: SocketAddr) {
        let bytes = encode(packet).expect("encode packet");
        self.send_raw(&bytes, destination).await;
    }

    async fn send_raw(&self, bytes: &[u8], destination: SocketAddr) {
        self.socket
            .send_to(bytes, destination)
            .await
            .expect("send datagram");
    }

    async fn recv(&self, wait: Duration) -> (Packet, SocketAddr) {
        let (bytes, from) = self.recv_raw(wait).await;
        let packet = decode(&bytes, 65_464).expect("decode server packet");
        (packet, from)
    }

    async fn recv_raw(&self, wait: Duration) -> (Vec<u8>, SocketAddr) {
        let mut buffer = vec![0u8; 65_536];
        let (length, from) = tokio::time::timeout(wait, self.socket.recv_from(&mut buffer))
            .await
            .expect("timed out waiting for server datagram")
            .expect("receive datagram");
        buffer.truncate(length);
        (buffer, from)
    }
}

#[derive(Debug)]
struct Download {
    data: Vec<u8>,
    blocks: Vec<usize>,
    block_size: usize,
    oack: Option<Vec<(String, String)>>,
}

fn rrq(name: &str, mode: Mode, options: &[(&str, &str)]) -> Packet {
    Packet::Rrq(Request {
        filename: name.to_string(),
        mode,
        options: options
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    })
}

async fn download(
    server: SocketAddr,
    name: &str,
    options: &[(&str, &str)],
) -> Result<Download, (ErrorCode, String)> {
    let client = TestClient::new(server).await;
    client.send(&rrq(name, Mode::Octet, options), server).await;
    receive_transfer(&client, 512).await
}

async fn receive_transfer(
    client: &TestClient,
    mut block_size: usize,
) -> Result<Download, (ErrorCode, String)> {
    let mut data = Vec::new();
    let mut blocks = Vec::new();
    let mut expected: u16 = 1;
    let mut tid: Option<SocketAddr> = None;
    let mut oack = None;
    loop {
        let (packet, from) = client.recv(Duration::from_secs(5)).await;
        match tid {
            Some(known) => assert_eq!(known, from, "server changed its transfer ID"),
            None => tid = Some(from),
        }
        match packet {
            Packet::Oack(options) => {
                for (name, value) in &options {
                    if name.eq_ignore_ascii_case("blksize") {
                        block_size = value.parse().expect("numeric blksize");
                    }
                }
                oack = Some(options);
                client.send(&Packet::Ack { block: 0 }, from).await;
            }
            Packet::Data {
                block,
                data: payload,
            } => {
                assert_eq!(block, expected, "server sent an unexpected block number");
                let length = payload.len();
                assert!(
                    length <= block_size,
                    "payload larger than negotiated block size"
                );
                data.extend_from_slice(&payload);
                blocks.push(length);
                client.send(&Packet::Ack { block }, from).await;
                if length < block_size {
                    break;
                }
                expected = expected.wrapping_add(1);
            }
            Packet::Error { code, message } => return Err((code, message)),
            other => panic!("unexpected server packet: {other:?}"),
        }
    }
    Ok(Download {
        data,
        blocks,
        block_size,
        oack,
    })
}

fn pattern(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| u8::try_from(index % 251).expect("u8"))
        .collect()
}

#[tokio::test]
async fn downloads_small_file_byte_for_byte() {
    let payload = pattern(300);
    let fixture = Fixture::start(&[("dir/hello.bin", &payload)]).await;
    let result = download(fixture.addr(), "dir/hello.bin", &[])
        .await
        .expect("download");
    assert_eq!(result.data, payload);
    assert_eq!(result.blocks, vec![300]);
    assert!(result.oack.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn exact_multiple_of_block_size_sends_zero_length_final_block() {
    let payload = pattern(1024);
    let fixture = Fixture::start(&[("exact.bin", &payload)]).await;
    let result = download(fixture.addr(), "exact.bin", &[])
        .await
        .expect("download");
    assert_eq!(result.data, payload);
    assert_eq!(result.blocks, vec![512, 512, 0]);
    fixture.stop().await;
}

#[tokio::test]
async fn empty_file_sends_one_empty_data_block() {
    let fixture = Fixture::start(&[("empty.bin", b"")]).await;
    let result = download(fixture.addr(), "empty.bin", &[])
        .await
        .expect("download");
    assert!(result.data.is_empty());
    assert_eq!(result.blocks, vec![0]);
    fixture.stop().await;
}

#[tokio::test]
async fn missing_file_returns_error_one() {
    let fixture = Fixture::start(&[]).await;
    let error = download(fixture.addr(), "missing.bin", &[])
        .await
        .expect_err("must fail");
    assert_eq!(error.0, ErrorCode::FileNotFound);
    assert_eq!(error.1, "File not found");
    fixture.stop().await;
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let outside = tempfile::tempdir().expect("outside dir");
    fs::write(outside.path().join("secret.txt"), b"top secret").expect("write secret");
    let fixture = Fixture::start(&[]).await;
    for name in [
        "../secret.txt",
        "..\\secret.txt",
        "/etc/passwd",
        "sub/../../secret.txt",
    ] {
        let error = download(fixture.addr(), name, &[])
            .await
            .expect_err("must fail");
        assert_eq!(
            error.0,
            ErrorCode::AccessViolation,
            "unexpected error for {name:?}"
        );
    }
    fixture.stop().await;
}

#[tokio::test]
async fn blksize_and_tsize_are_negotiated() {
    let payload = pattern(5000);
    let config = ServerConfig {
        block_size: 1024,
        ..ServerConfig::default()
    };
    let fixture = Fixture::start_with(config, &[("big.bin", &payload)]).await;
    let result = download(
        fixture.addr(),
        "big.bin",
        &[("blksize", "1024"), ("tsize", "0")],
    )
    .await
    .expect("download");
    assert_eq!(result.block_size, 1024);
    assert_eq!(result.data, payload);
    assert_eq!(result.blocks, vec![1024, 1024, 1024, 1024, 904]);
    let oack = result.oack.expect("OACK expected");
    assert!(oack.contains(&("blksize".to_string(), "1024".to_string())));
    assert!(oack.contains(&("tsize".to_string(), "5000".to_string())));
    fixture.stop().await;
}

#[tokio::test]
async fn blksize_is_clamped_to_server_maximum() {
    let payload = pattern(900);
    let fixture = Fixture::start(&[("f.bin", &payload)]).await;
    let result = download(fixture.addr(), "f.bin", &[("blksize", "1024")])
        .await
        .expect("download");
    assert_eq!(result.block_size, 512);
    assert_eq!(result.data, payload);
    fixture.stop().await;
}

#[tokio::test]
async fn invalid_blksize_is_rejected_with_error_eight() {
    let fixture = Fixture::start(&[("f.bin", b"data")]).await;
    let error = download(fixture.addr(), "f.bin", &[("blksize", "7")])
        .await
        .expect_err("must fail");
    assert_eq!(error.0, ErrorCode::OptionNegotiation);
    fixture.stop().await;
}

#[tokio::test]
async fn timeout_option_is_echoed() {
    let fixture = Fixture::start(&[("f.bin", b"timeout option")]).await;
    let result = download(fixture.addr(), "f.bin", &[("timeout", "1")])
        .await
        .expect("download");
    let oack = result.oack.expect("OACK expected");
    assert!(oack.contains(&("timeout".to_string(), "1".to_string())));
    assert_eq!(result.data, b"timeout option");
    fixture.stop().await;
}

#[tokio::test]
async fn duplicate_ack_is_idempotent() {
    let payload = pattern(1200);
    let fixture = Fixture::start(&[("dup.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("dup.bin", Mode::Octet, &[]), fixture.addr())
        .await;

    let (first, from) = client.recv(Duration::from_secs(5)).await;
    let Packet::Data {
        block: 1,
        data: first_payload,
    } = first
    else {
        panic!("expected DATA 1")
    };
    assert_eq!(first_payload.len(), 512);
    client.send(&Packet::Ack { block: 1 }, from).await;
    client.send(&Packet::Ack { block: 1 }, from).await;

    let (second, _) = client.recv(Duration::from_secs(5)).await;
    let Packet::Data {
        block: 2,
        data: second_payload,
    } = second
    else {
        panic!("expected DATA 2")
    };
    assert_eq!(second_payload.len(), 512);
    client.send(&Packet::Ack { block: 2 }, from).await;

    let (third, _) = client.recv(Duration::from_secs(5)).await;
    let Packet::Data {
        block: 3,
        data: third_payload,
    } = third
    else {
        panic!("expected DATA 3")
    };
    assert_eq!(third_payload.len(), 176);
    client.send(&Packet::Ack { block: 3 }, from).await;

    assert_eq!(
        payload.len(),
        first_payload.len() + second_payload.len() + third_payload.len()
    );
    fixture.stop().await;
}

#[tokio::test]
async fn unacknowledged_data_is_retransmitted() {
    let payload = pattern(700);
    let fixture = Fixture::start(&[("retry.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("retry.bin", Mode::Octet, &[]), fixture.addr())
        .await;

    let (first, from) = client.recv(Duration::from_secs(5)).await;
    let Packet::Data {
        block: 1,
        data: first_payload,
    } = first
    else {
        panic!("expected DATA 1")
    };
    // Deliberately withhold the ACK and wait for the retransmission timer.
    let (repeat, repeat_from) = client.recv(Duration::from_secs(3)).await;
    assert_eq!(
        repeat_from, from,
        "retransmission must come from the same TID"
    );
    let Packet::Data {
        block: 1,
        data: repeat_payload,
    } = repeat
    else {
        panic!("expected retransmitted DATA 1")
    };
    assert_eq!(repeat_payload, first_payload);

    client.send(&Packet::Ack { block: 1 }, from).await;
    let (second, _) = client.recv(Duration::from_secs(5)).await;
    let Packet::Data {
        block: 2,
        data: second_payload,
    } = second
    else {
        panic!("expected DATA 2")
    };
    assert_eq!(second_payload.len(), 188);
    client.send(&Packet::Ack { block: 2 }, from).await;
    assert_eq!(first_payload.len() + second_payload.len(), payload.len());
    fixture.stop().await;
}

#[tokio::test]
async fn packets_from_the_wrong_tid_get_error_five() {
    let payload = pattern(800);
    let fixture = Fixture::start(&[("tid.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("tid.bin", Mode::Octet, &[]), fixture.addr())
        .await;
    let (Packet::Data { block: 1, .. }, server_tid) = client.recv(Duration::from_secs(5)).await
    else {
        panic!("expected DATA 1")
    };

    let intruder = TestClient::new(fixture.addr()).await;
    intruder.send(&Packet::Ack { block: 1 }, server_tid).await;
    let (reply, from) = intruder.recv(Duration::from_secs(2)).await;
    assert_eq!(from, server_tid);
    let Packet::Error { code, .. } = reply else {
        panic!("expected ERROR 5")
    };
    assert_eq!(code, ErrorCode::UnknownTransferId);

    // The legitimate transfer must be unaffected.
    client.send(&Packet::Ack { block: 1 }, server_tid).await;
    let (Packet::Data { block: 2, data }, _) = client.recv(Duration::from_secs(5)).await else {
        panic!("expected DATA 2")
    };
    client.send(&Packet::Ack { block: 2 }, server_tid).await;
    assert_eq!(data.len(), 288);
    fixture.stop().await;
}

#[tokio::test]
async fn retransmitted_request_resends_without_new_session() {
    let payload = pattern(600);
    let fixture = Fixture::start(&[("retry-req.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("retry-req.bin", Mode::Octet, &[]), fixture.addr())
        .await;
    let (Packet::Data { block, data: first }, tid) = client.recv(Duration::from_secs(5)).await
    else {
        panic!("expected DATA 1")
    };
    assert_eq!(block, 1);

    // The client repeats the RRQ from the same port; the server must reuse the
    // existing session/TID instead of creating a competing transfer.
    client
        .send(&rrq("retry-req.bin", Mode::Octet, &[]), fixture.addr())
        .await;
    let (
        Packet::Data {
            block,
            data: repeat,
        },
        tid2,
    ) = client.recv(Duration::from_secs(3)).await
    else {
        panic!("expected repeated DATA 1")
    };
    assert_eq!(tid2, tid);
    assert_eq!(block, 1);
    assert_eq!(repeat, first);

    client.send(&Packet::Ack { block: 1 }, tid).await;
    // The duplicate-request resend may already be in flight, so tolerate stale
    // DATA 1 packets until block 2 arrives (this is normal RFC 1350 behaviour).
    let (data, _) = loop {
        let (packet, from) = client.recv(Duration::from_secs(5)).await;
        match packet {
            Packet::Data { block: 2, data } => break (data, from),
            Packet::Data { block: 1, .. } => client.send(&Packet::Ack { block: 1 }, from).await,
            other => panic!("unexpected packet while waiting for DATA 2: {other:?}"),
        }
    };
    assert_eq!(data.len(), 88);
    client.send(&Packet::Ack { block: 2 }, tid).await;
    fixture.stop().await;
}

#[tokio::test]
async fn wrq_is_rejected_because_uploads_are_disabled() {
    let fixture = Fixture::start(&[]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(
            &Packet::Wrq(Request {
                filename: "upload.bin".into(),
                mode: Mode::Octet,
                options: vec![],
            }),
            fixture.addr(),
        )
        .await;
    let (packet, _) = client.recv(Duration::from_secs(2)).await;
    let Packet::Error { code, .. } = packet else {
        panic!("expected ERROR")
    };
    assert_eq!(code, ErrorCode::AccessViolation);
    fixture.stop().await;
}

#[tokio::test]
async fn netascii_mode_is_rejected_explicitly() {
    let fixture = Fixture::start(&[("f.txt", b"text")]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("f.txt", Mode::NetAscii, &[]), fixture.addr())
        .await;
    let (packet, _) = client.recv(Duration::from_secs(2)).await;
    let Packet::Error { code, message } = packet else {
        panic!("expected ERROR")
    };
    assert_eq!(code, ErrorCode::Undefined);
    assert!(
        message.contains("netascii"),
        "unexpected message: {message}"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn unknown_mode_and_malformed_packets_are_rejected() {
    let fixture = Fixture::start(&[]).await;
    let client = TestClient::new(fixture.addr()).await;

    client
        .send_raw(b"\x00\x01f.txt\x00binary\x00", fixture.addr())
        .await;
    let (packet, _) = client.recv(Duration::from_secs(2)).await;
    assert_eq!(
        packet,
        Packet::Error {
            code: ErrorCode::IllegalOperation,
            message: "Unsupported transfer mode".into()
        }
    );

    client.send_raw(&[0, 99], fixture.addr()).await;
    let (packet, _) = client.recv(Duration::from_secs(2)).await;
    assert_eq!(
        packet,
        Packet::Error {
            code: ErrorCode::IllegalOperation,
            message: "Illegal TFTP operation".into()
        }
    );
    fixture.stop().await;
}

#[tokio::test]
async fn oversized_request_is_rejected() {
    let fixture = Fixture::start(&[]).await;
    let client = TestClient::new(fixture.addr()).await;
    let mut raw = Vec::from(&b"\x00\x01"[..]);
    raw.extend(std::iter::repeat_n(b'a', 1100));
    raw.push(0);
    raw.extend_from_slice(b"octet\x00");
    client.send_raw(&raw, fixture.addr()).await;
    let (packet, _) = client.recv(Duration::from_secs(2)).await;
    let Packet::Error { code, .. } = packet else {
        panic!("expected ERROR")
    };
    assert_eq!(code, ErrorCode::IllegalOperation);
    fixture.stop().await;
}

#[tokio::test]
async fn file_larger_than_limit_is_rejected() {
    let payload = pattern(100);
    let config = ServerConfig {
        max_file_bytes: 10,
        ..ServerConfig::default()
    };
    let fixture = Fixture::start_with(config, &[("large.bin", &payload)]).await;
    let error = download(fixture.addr(), "large.bin", &[])
        .await
        .expect_err("must fail");
    assert_eq!(error.0, ErrorCode::DiskFull);
    fixture.stop().await;
}

#[tokio::test]
async fn concurrent_transfers_are_independent() {
    let one = pattern(1500);
    let two = pattern(700);
    let three = pattern(4000);
    let fixture =
        Fixture::start(&[("one.bin", &one), ("two.bin", &two), ("three.bin", &three)]).await;
    let address = fixture.addr();
    let (first, second, third) = tokio::join!(
        download(address, "one.bin", &[]),
        download(address, "two.bin", &[("blksize", "1024")]),
        download(address, "three.bin", &[("blksize", "512")])
    );
    assert_eq!(first.expect("one").data, one);
    assert_eq!(second.expect("two").data, two);
    assert_eq!(third.expect("three").data, three);
    fixture.stop().await;
}

#[tokio::test]
async fn lifecycle_events_report_start_and_completion() {
    let payload = pattern(2048);
    let fixture = Fixture::start(&[("events.bin", &payload)]).await;
    let result = download(fixture.addr(), "events.bin", &[])
        .await
        .expect("download");
    assert_eq!(result.data, payload);

    let events = wait_for_events(&fixture.events, |events| {
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::Completed))
    })
    .await;
    let started = events
        .iter()
        .find(|event| matches!(event.kind, EventKind::Started))
        .expect("Started event");
    assert_eq!(started.file, "events.bin");
    assert_eq!(started.total, Some(payload.len() as u64));
    let completed = events
        .iter()
        .find(|event| matches!(event.kind, EventKind::Completed))
        .expect("Completed event");
    assert_eq!(completed.bytes, payload.len() as u64);
    assert_eq!(completed.session_id, started.session_id);
    fixture.stop().await;
}

#[tokio::test]
async fn stop_cancels_active_transfer_and_is_bounded() {
    let payload = pattern(4096);
    let fixture = Fixture::start(&[("cancel.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("cancel.bin", Mode::Octet, &[]), fixture.addr())
        .await;
    let (Packet::Data { block: 1, .. }, _) = client.recv(Duration::from_secs(5)).await else {
        panic!("expected DATA 1")
    };

    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), fixture.handle.stop())
        .await
        .expect("stop timed out");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fixture.handle.status(), ServerStatus::Stopped);

    let events = wait_for_events(&fixture.events, |events| {
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::Failed { .. }))
    })
    .await;
    let failed = events
        .iter()
        .find(|event| matches!(event.kind, EventKind::Failed { .. }))
        .expect("Failed event");
    let EventKind::Failed { reason } = &failed.kind else {
        unreachable!()
    };
    assert!(reason.contains("shutdown"), "unexpected reason: {reason}");
}

#[tokio::test]
async fn stop_is_idempotent() {
    let fixture = Fixture::start(&[]).await;
    let started = Instant::now();
    fixture.handle.stop().await;
    fixture.handle.stop().await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fixture.handle.status(), ServerStatus::Stopped);
}

#[tokio::test]
async fn timeout_gives_up_after_configured_retries() {
    let payload = pattern(1024);
    let config = ServerConfig {
        timeout_secs: 1,
        max_retries: 1,
        ..ServerConfig::default()
    };
    let fixture = Fixture::start_with(config, &[("slow.bin", &payload)]).await;
    let client = TestClient::new(fixture.addr()).await;
    client
        .send(&rrq("slow.bin", Mode::Octet, &[]), fixture.addr())
        .await;
    let (Packet::Data { block: 1, .. }, _) = client.recv(Duration::from_secs(5)).await else {
        panic!("expected DATA 1")
    };
    // One retransmission, then the session gives up and sends ERROR 0.
    let (repeat, _) = client.recv(Duration::from_secs(3)).await;
    assert!(matches!(repeat, Packet::Data { block: 1, .. }));
    let (error, _) = client.recv(Duration::from_secs(4)).await;
    let Packet::Error { code, .. } = error else {
        panic!("expected ERROR after retries")
    };
    assert_eq!(code, ErrorCode::Undefined);
    fixture.stop().await;
}

#[tokio::test]
async fn block_numbers_wrap_from_65535_to_zero() {
    // blksize 8 keeps the ~65k-block wrap test small enough for a unit test.
    let payload = pattern(8 * 65_536 + 5);
    let fixture = Fixture::start(&[("wrap.bin", &payload)]).await;
    let started = Instant::now();
    let result = download(fixture.addr(), "wrap.bin", &[("blksize", "8")])
        .await
        .expect("download");
    assert_eq!(
        result.data, payload,
        "payload corrupted across block-number wrap"
    );
    assert_eq!(result.blocks.len(), 65_537);
    assert_eq!(result.blocks[result.blocks.len() - 1], 5);
    eprintln!(
        "wrap test transferred {} blocks in {:?}",
        result.blocks.len(),
        started.elapsed()
    );
    fixture.stop().await;
}

async fn wait_for_events(
    events: &EventBus,
    predicate: impl Fn(&[TftpEvent]) -> bool,
) -> Vec<TftpEvent> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = events.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "event not observed before timeout: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
