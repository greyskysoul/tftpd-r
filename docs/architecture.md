# Architecture

This document describes the implemented MVP (Phase 0–5 of the original plan).
Deviations from the plan are recorded in [`docs/adr/`](adr/).

## Crate boundaries

```text
tftpd-r-gui-slint ──┐
                    ├──> tftpd-r-core
tftpd-r-daemon ────┘
```

- **`tftpd-r-core`** owns the TFTP protocol codec, option negotiation, the RRQ
  session state machine, the filesystem sandbox, configuration validation and
  the event bus. It depends on Tokio, serde/toml, thiserror and tracing, but not
  on Slint, clap, tracing-subscriber or any IPC transport.
- **`tftpd-r-daemon`** is a thin front-end: clap CLI, TOML file loading, tracing
  logging, signal handling and process exit codes.
- **`tftpd-r-gui-slint`** is a second front-end: a Slint window plus an embedded
  controller that owns a Tokio runtime and a `tftpd-r-core` server handle. It
  does not talk to the daemon (no IPC in this release).

There is no `tftpd-r-control-protocol` crate yet because embedded mode does not
need shared DTOs. When IPC is added, put versioned DTOs in a separate crate so
`tftpd-r-core` is not coupled to the transport.

## Runtime ownership

`tftpd-r-core` uses Tokio primitives but **never creates a runtime**. The
front-end owns the runtime:

- The daemon builds a multi-thread runtime in `main` and `block_on`s
  `serve_async`; Ctrl-C, Ctrl-Break (Windows console events) and SIGINT/SIGTERM
  (Unix) request a graceful stop.
- The GUI builds a two-worker runtime in `EmbeddedController::new`, uses
  `block_on` for the (fast) `TftpServer::start` call and spawns shutdown work so
  the UI thread is never blocked for the full grace period.

`ServerHandle` therefore carries an implicit requirement: the runtime that
started it must stay alive. See [`adr/0001-runtime-ownership.md`](adr/0001-runtime-ownership.md).

## Server lifecycle

`TftpServer::start(config, event_sink)`:

1. Validates `ServerConfig` (including root directory existence).
2. Canonicalises the root with `Sandbox::new`.
3. Binds the main UDP socket (`bind_address`; port 0 is allowed for tests).
4. Spawns the accept loop and returns a `ServerHandle`.

Status transitions: `Starting → Running → Stopping → Stopped`, or `Failed` if the
accept loop cannot report its socket address. Binding/validation failures are
returned synchronously from `start`, so no half-started listener exists.

`ServerHandle::stop()` is idempotent: it sends `Stopping`, cancels the
`CancellationToken`, waits for the accept loop, then awaits the join handle. The
accept loop cancels all sessions, drains them for `shutdown_grace_secs`, aborts
the remainder and marks the server `Stopped`. Dropping a handle without `stop()`
still cancels the token (sessions finish on their cancellation path), but does
not wait for them.

## Accept loop and sessions

- The main socket receives RRQ/WRQ and malformed datagrams. Request datagrams
  larger than `MAX_REQUEST_BYTES` (1024) are rejected with ERROR 4.
- A duplicate RRQ from a peer that already has a session does **not** create a
  second session: the accept loop sends a `SessionSignal::Resend` to the
  existing session, which retransmits its current packet. This keeps the TID
  stable for the client and prevents task/port exhaustion from retried requests.
- Each session gets its own ephemeral UDP socket (the TID) in the listener's
  address family, an event session ID, a `JoinSet` slot and a semaphore permit.
- Global concurrency is bounded by `max_sessions`; a per-IP cap of 8 further
  limits one peer. Rejected requests receive a small ERROR packet and an
  event/log entry.
- The session socket only accepts packets from the expected `peer` address.
  Other sources get ERROR 5 (unknown TID), capped at 8 responses per block
  exchange. Packets from the peer that are not the expected ACK are ignored, so
  duplicates and reordering do not corrupt state.

## RRQ state machine

For each session:

1. Open the file through the sandbox (size limit applied).
2. Negotiate options (`blksize` capped by config, `timeout`, `tsize`) or fail
   with ERROR 8.
3. Emit `Started` with the file size.
4. If options were negotiated: send OACK and wait for ACK 0.
5. Send DATA blocks starting at 1, waiting for the matching ACK. The exact
   encoded DATA packet is retransmitted on timeout (bounded by `max_retries`);
   RRQ retransmissions also trigger an immediate resend.
6. A block shorter than the negotiated size ends the transfer; a full final
   block is followed by a zero-length DATA block, as required by RFC 1350.
7. Emit `Progress` at most every 200 ms and `Completed`/`Failed` at the end.

Block numbers use 16-bit `wrapping_add`, tested across the 65535 → 0 boundary
with `blksize = 8`.

The transfer deadline (`max_transfer_duration_secs`) is checked before sending
each block and while waiting for ACKs.

## Filesystem sandbox

`Sandbox` canonicalises the root once and validates every request syntactically
before touching the filesystem. A leading `/` or `\` is mapped to the root for
legacy clients (`/firmware.bin` → `<root>/firmware.bin`); after that mapping,
`..`, drive letters/`:`, device names, alternate data streams and reparse points
are rejected. The file is then opened through a component walk, and the final
canonical path is compared component-wise with the canonical root. Details and
residual risks are in [`security.md`](security.md) and
[`adr/0003-path-sandbox.md`](adr/0003-path-sandbox.md).

## Events

`EventSink` is a small synchronous trait. The daemon's `TracingSink` maps events
to `tracing` records; the GUI uses `EventBus`, a bounded ring guarded by a
`Mutex`.

- `EventBus::emit` assigns a monotonic `seq` and drops the oldest event when the
  ring is full, incrementing `dropped`. Lifecycle events are bounded in practice
  by session limits; the ring size is configurable (`event_buffer`).
- The GUI polls `drain_since(last_seq)` from a Slint timer every 200 ms, updates
  models and never blocks the server on a slow subscriber.
- `Progress` events are throttled per session so a fast transfer cannot flood the
  ring or the UI.

## GUI threading

- Slint runs on the main/UI thread. All UI property access happens in callbacks
  or in the Slint `Timer` closure.
- The embedded server runs on the controller's Tokio runtime. No Tokio task
  touches UI properties directly; communication is pull-based through the event
  bus and a `watch` channel for status.
- `stop()` spawns the potentially slow `ServerHandle::stop()` and returns; the
  timer observes `Stopping → Stopped`. Window close calls
  `EmbeddedController::shutdown`, which blocks only after the UI loop has exited.
- Log rows are capped at 500 in the model; the event ring is independently
  bounded.

## Configuration and validation

`ServerConfig` is serde-derived with `deny_unknown_fields` and safe defaults. The
same `validate()` is used by the daemon, the GUI and tests, so a configuration
accepted by one front-end is accepted by the other. CLI overrides are applied and
re-validated. TOML syntax errors include the file path via `ConfigLoadError`.

## Logging

- Library code logs operational events at `debug`/`warn` through `tracing`; it
  does not print to stdout.
- The daemon initialises a global `tracing-subscriber` (filter from `--log` or
  `TFTPD_R_LOG`, default `info`) and logs lifecycle events.
- Error text sent to clients is a fixed, sanitised string per error class; local
  paths and OS error messages stay in local logs (and even there only the file
  name and error kind are used for client-visible paths).
