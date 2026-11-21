# Architecture

This document describes the implemented MVP (Phase 0–5 of the original plan).
Deviations from the plan are recorded in [`docs/adr/`](adr/).

## Crate boundaries

```text
tftpd-r-gui-slint ──┐
                    ├──> tftpd-r-core
tftpd-r-daemon ────┘

tftpd-r-gui-slint ──┐  (build-dependencies only)
tftpd-r-daemon ────┴──> tftpd-r-build-support
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
- **`tftpd-r-build-support`** is build-time only and is not linked into either
  binary. It turns `crates/tftpd-r-gui-slint/ui/icons/app-icon.ico` into a
  Windows `.res` resource: MSVC links the `.res` directly, GNU targets convert
  it to a COFF `.rsrc` object with `windres`. On non-Windows targets it is a
  no-op, so Linux builds do not need a resource compiler.

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

1. Validates `ServerConfig` (including root directory existence and the bind
   address list).
2. Canonicalises the root with `Sandbox::new`.
3. Binds one UDP socket per effective bind address on the shared `port` (port 0
   is allowed for tests: the first listener resolves the ephemeral port and the
   rest reuse it). Binding is all-or-nothing; a failure drops the sockets bound
   so far and returns the classified error.
4. Spawns one accept loop per listener and returns a `ServerHandle` whose
   `local_addrs()` reports the actual bound addresses.

Status transitions: `Starting → Running { local_addrs } → Stopping → Stopped`,
or `Failed` if the server task fails. Binding/validation failures are returned
synchronously from `start`, so no half-started listener exists.

`ServerHandle::stop()` is idempotent: it sends `Stopping`, cancels the
`CancellationToken`, waits for the supervisor (which joins every accept loop),
then awaits the join handle. Each accept loop cancels its sessions, drains them
for `shutdown_grace_secs` in parallel with the other loops, aborts the remainder
and releases its socket. Dropping a handle without `stop()` still cancels the
token (sessions finish on their cancellation path), but does not wait for them.

## Accept loops and sessions

- One accept loop owns one listening socket. Each loop receives RRQ/WRQ and
  malformed datagrams on its own address; request datagrams larger than
  `MAX_REQUEST_BYTES` (1024) are rejected with ERROR 4.
- The peer→session map, the per-peer session count, the global session
  semaphore and the session-id counter are shared by all listeners. A duplicate
  RRQ from a peer that already has a session does **not** create a second
  session, even when the retry arrives on another listening address: the loop
  sends a `SessionSignal::Resend` to the existing session, which retransmits its
  current packet. This keeps the TID stable for the client and prevents
  task/port exhaustion from retried requests.
- Each session gets its own ephemeral UDP socket (the TID), an event session ID,
  a `JoinSet` slot and a semaphore permit. A specific listener address is reused
  as the session socket address, so a multi-homed client sees DATA from the
  address it contacted; wildcard listeners bind `0.0.0.0`/`::` and let the OS
  pick the source address.
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

## GUI structure and state

The GUI crate stays deliberately small:

- `ui/app.slint` composes the window from `titlebar.slint`, `toolbar.slint`,
  `logs.slint`, `transfers.slint`, `statusbar.slint` and three overlays:
  `root-menu.slint` (recent root directories), `settings.slint` and
  `about.slint`.
- The window is frameless (`no-frame: true`). `titlebar.slint` draws the title
  and minimize / maximize-restore / close controls, and uses `WindowMoveArea`
  for dragging. `resize-border-width` keeps the native edge/corner resize
  behaviour, and a one-pixel dark frame drawn by `app.slint` bounds the client
  area. The drag area stops before the window buttons so pressing a button
  cannot also start a window move.
- `ui/widgets.slint` holds the icon-over-label toolbar button, compact buttons,
  the text field and panel primitives; `ui/theme.slint` centralises the palette;
  `ui/icons.slint` embeds Material Design icon path data, so no image decoder or
  extra image files are needed at runtime.
- The service status is a dedicated chip at the far left of the toolbar; the
  status bar keeps the read-only hint or the last error, the bound address(es)
  and the clickable root selector (last path component, full path in the
  settings dialog and recent-directory menu).
- The settings dialog edits `port`, the multi-select bind address list, the
  root, the block size, the timeout and the maximum session count. The address
  picker always offers `0.0.0.0` ("全部网络") and `127.0.0.1` ("本地回环"), merges
  the discovered interface addresses and never drops a selected address that
  discovery did not report. "全部网络" is exclusive and sticky: selecting it
  clears the other addresses, selecting another address clears it, and it cannot
  be unchecked (unchecking the last specific address selects it again), so the
  bind list is never empty. The root field has a 浏览… button backed by a native
  directory dialog. When no valid `config.toml` exists, the panel preselects
  `0.0.0.0`; the core and daemon defaults stay loopback. On the first start of a
  profile without a valid configuration the dialog opens automatically, once
  (see `src/user_state.rs`).
- Saving the dialog, or selecting another recent root, compares the resulting
  `ServerConfig` with the configuration of the running server. When the server
  is running (`Starting`/`Running`) and no transfer is active, the change is
  applied with an asynchronous `EmbeddedController::restart`; with active
  transfers it is only stored and the log asks for a manual restart. A stop or
  restart that is already in progress (`Stopping`) is never turned back into a
  start. `poll_status` no longer reports a stale `Running` after the handle was
  taken by `stop()`.
- `src/main.rs` owns the Slint models (`LogRow`, `TransferRow`, `RecentDir`,
  `AddressOption`), translates `TftpEvent`s and `ServerStatus` into UI state,
  and keeps the UI thread free of blocking calls. Window controls call
  `slint::Window` methods (`set_minimized`, `set_maximized`) on the UI thread;
  close requests quit the event loop so the normal window-close shutdown path
  runs.
- `src/netif.rs` discovers interface addresses on a background thread; the UI
  timer merges the result into the picker. It asks the platform runtime for the
  list (Windows PowerShell + .NET `NetworkInterface`, `hostname -I` on Unix) and
  parses only IP literals; failure is not fatal.
- `src/folder_picker.rs` runs the native directory dialog in a helper process on
  a background thread and validates that the result is a directory; the UI timer
  applies it to the settings form. Both helpers are fixed, absolute commands
  without a shell, with the initial directory passed via the environment.
- `src/recent.rs` persists at most eight canonicalised root directories
  (most-recent first). The drop-down's 清空 button removes the state file; the
  configured/running root is not history and is unaffected. It is local UI
  state, not part of `ServerConfig`, and write failures are ignored so a
  read-only profile still runs the panel.
- `src/user_state.rs` locates the per-user state directory and stores the
  one-time setup flag (`ui-state.txt`). The recent-root file uses the same
  directory helper. Both files are advisory: a missing or unwritable profile
  only means the panel asks for setup again / keeps no history.
- Selecting a recent root while the server runs either restarts it
  automatically (no active transfer) or updates the configuration for the next
  manual restart; the status bar keeps showing the root of the running server.
  The same rule applies to saving the settings dialog.
- The GUI is a windowed-subsystem binary on Windows (`windows_subsystem =
  "windows"`), so starting the panel does not allocate a console window.

Dialogs and the drop-downs (recent roots, bind address picker) are sibling
rectangles inside the same Slint window rather than separate native windows;
this keeps the software renderer and DPI handling simple and guarantees they
render above the main layout. The layout relies on explicit positions for sized
boxes that are not managed by a layout (borders and progress fills), because a
sized child without `x`/`y` is centred by the renderer.

## GUI threading

- Slint runs on the main/UI thread. All UI property access happens in callbacks
  or in the Slint `Timer` closure.
- The embedded server runs on the controller's Tokio runtime. No Tokio task
  touches UI properties directly; communication is pull-based through the event
  bus and a `watch` channel for status.
- `stop()` spawns the potentially slow `ServerHandle::stop()` and returns; the
  timer observes `Stopping → Stopped`. Window close calls
  `EmbeddedController::shutdown`, which blocks only after the UI loop has exited.
- `restart()` queues the new configuration, stops the running handle and starts
  the replacement from `poll_status` once the old server reaches `Stopped`. While
  a restart is pending a stale `Running` watch value is reported as `Stopping`,
  and a failed replacement is surfaced through `take_error()` instead of leaving
  a half-started server.
- Log rows are capped at 500 in the model; the event ring is independently
  bounded. Logs are displayed newest-first so the latest event stays visible
  without programmatic scrolling.
- The title-bar maximize icon follows `Window::is_maximized()` on the UI timer;
  the OS window state is authoritative rather than a UI-side toggle. Minimize,
  maximize and close are plain UI-thread calls and never wait for the server.

## Configuration and validation

`ServerConfig` is serde-derived with `deny_unknown_fields` and safe defaults. The
same `validate()` is used by the daemon, the GUI and tests, so a configuration
accepted by one front-end is accepted by the other. CLI overrides are applied and
re-validated. TOML syntax errors include the file path via `ConfigLoadError`.

The listen configuration is `port` plus `bind_addresses` (a list of IP
addresses). Exact duplicates are dropped and a wildcard (`0.0.0.0` / `::`)
collapses the other addresses of its family in `effective_bind_addresses()`;
multicast/broadcast entries and lists longer than 16 are rejected. The 0.1.0
`bind_address` key is gone, so an old file fails with an unknown-field error
instead of silently binding something else. Daemon `--bind` replaces the whole
list with one address and port. `ServerConfig::default()` stays on loopback; only
the GUI's no-config form preselects `0.0.0.0` (ADR 0009), and the GUI does not
write `config.toml`.

## Logging

- Library code logs operational events at `debug`/`warn` through `tracing`; it
  does not print to stdout.
- The daemon initialises a global `tracing-subscriber` (filter from `--log` or
  `TFTPD_R_LOG`, default `info`) and logs lifecycle events.
- Error text sent to clients is a fixed, sanitised string per error class; local
  paths and OS error messages stay in local logs (and even there only the file
  name and error kind are used for client-visible paths).
