# TftpdR

TftpdR is a read-only TFTP (RFC 1350) server written in Rust, with a headless
daemon and a native [Slint](https://slint.dev/) desktop panel.

> **Status: MVP (Phase 0–5 vertical slice).** RRQ downloads, option negotiation,
> the path sandbox, the daemon CLI and the embedded-mode GUI are implemented and
> tested. WRQ uploads, daemon IPC and Windows Service packaging are explicitly
> **not** part of this release. See [Scope](#scope-and-non-goals).

## Scope and non-goals

Implemented:

- RFC 1350 RRQ (read) transfers over UDP, with a dedicated transfer ID (TID)
  socket per session.
- RFC 2347/2348/2349 option negotiation: `blksize`, `timeout`, `tsize`; `octet`
  mode only. `netascii` and `mail` requests are rejected explicitly instead of
  being silently treated as `octet`.
- Block-number wraparound, zero-length final DATA for exact block multiples,
  bounded retransmission, duplicate-ACK tolerance, unknown-TID ERROR 5 handling
  and duplicate-request resend without spawning a second session.
- Sandboxed root directory: traversal, drive letters, device paths, NTFS
  alternate data streams, Windows reserved device names and reparse points
  (symlinks/junctions) are rejected. A leading `/` or `\` (legacy clients often
  send `get /file.bin`) is mapped to the configured root instead of being
  rejected; it cannot bypass the traversal checks.
- Bounded concurrency (global and per-peer), per-transfer deadline, file-size
  limit and bounded event ring.
- Headless daemon: TOML configuration, `check-config`, `print-default-config`,
  structured logs and Ctrl-C / Ctrl-Break / SIGTERM graceful shutdown.
- Slint GUI (embedded mode): frameless window with a custom dark title bar and
  window controls; toolbar with a prominent service-status chip plus
  start/restart, stop, settings and about; status bar with bind address and a
  clickable recently-used root selector; activity log and active transfers;
  settings validation. Layout and icons are in `crates/tftpd-r-gui-slint/ui/`.

Not implemented in this release (deliberate, tracked in `docs/adr/`):

- **WRQ uploads.** `allow_upload = true` is rejected by configuration
  validation; the server answers WRQ with ERROR 2 (access violation). Uploads
  need a separate security review and atomic-temp-file design first.
- **IPC / GUI-managed daemon.** The GUI embeds the server in its own process
  (`docs/adr/0004-embedded-only.md`). There is no named-pipe protocol yet.
- **Windows Service registration**, system tray, automatic updates, remote
  management and user/role management.
- `netascii` mode (`docs/adr/0006-octet-only.md`).

## Requirements

- Rust **1.100.0** is the toolchain used to develop and verify this workspace.
  An MSRV is not yet claimed; older compilers have not been tested.
- Windows 10/11 is the primary target. The protocol engine is platform-neutral
  and its tests also run on Linux (see CI), but this repository has only been
  executed on Windows so far.
- On Windows, a linker is required: the MSVC Build Tools (`link.exe`) for the
  default `x86_64-pc-windows-msvc` toolchain, or MinGW-w64 for
  `x86_64-pc-windows-gnu`. The verification described in
  [docs/testing.md](docs/testing.md) used the GNU toolchain with MinGW-w64.

## Build

```powershell
cargo build --workspace            # debug
cargo build --workspace --release  # release binaries in target\release
```

The workspace contains three crates:

| Crate | Purpose |
| --- | --- |
| `tftpd-r-core` | Protocol, session engine, sandbox, configuration, events. No GUI/daemon/IPC dependency. |
| `tftpd-r-daemon` | Headless server binary: CLI, TOML config, logging, graceful shutdown. |
| `tftpd-r-gui-slint` | Native Slint desktop panel; embeds `tftpd-r-core` (no daemon required). |

## Quick start (daemon)

```powershell
# 1. Create the exported directory (the example config points here).
New-Item -ItemType Directory -Force tftp-root | Out-Null
Set-Content tftp-root\hello.txt "hello over tftp"

# 2. Validate the configuration (does not bind a socket).
Copy-Item config.example.toml config.toml
cargo run -p tftpd-r-daemon -- check-config --config config.toml

# 3. Run the server in the foreground; Ctrl-C (or Ctrl-Break) stops it gracefully.
cargo run -p tftpd-r-daemon -- serve --config config.toml
```

Then point any TFTP client at `127.0.0.1:6969` and request `hello.txt`. The
server is read-only: uploads are refused.

CLI summary:

```text
tftpd-r serve --config config.toml [--bind 0.0.0.0:6969] [--root DIR]
tftpd-r check-config --config config.toml
tftpd-r print-default-config
```

Exit codes: `0` success, `2` configuration/CLI validation failure, `1` runtime
startup or server failure. `--bind`/`--root` override the file and are validated
again. `TFTPD_R_LOG` (or `--log`) selects the `tracing` filter, e.g.
`TFTPD_R_LOG=debug`.

Old clients that request `get /hello.txt` are supported: a leading `/` (or `\`)
is mapped to the configured root instead of being rejected as an absolute host
path. Traversal such as `/../secret` is still rejected.

## Quick start (GUI)

```powershell
cargo run -p tftpd-r-gui-slint
```

The GUI loads `config.toml` from the current directory when present; otherwise it
uses safe defaults. The panel is in Chinese and is organized as follows:

- **Window**: frameless, with a custom title bar on the same dark background as
  the toolbar. The title bar holds the app name, a drag area
  (`WindowMoveArea`) and minimize / maximize-restore / close controls; the
  window keeps a resize border (`resize-border-width`) and a one-pixel dark
  frame so it stays visually contained.
- **Toolbar**: the service status chip (服务状态: 已停止 / 启动中 / 运行中 /
  停止中 / 启动失败) is at the far left, followed by 启动 / 停止 / 设置 / 关于,
  each with a large icon above its label. 停止 is disabled until the server
  starts. While the server is running, 启动 turns into 重启; it stops the
  current instance and starts a new one with the saved settings (the stop is
  asynchronous, so the UI never blocks).
- **Main area**: the activity log (newest event first) above the active transfer
  table (file, client, progress, size, state).
- **Status bar**: a read-only hint, or the last error when there is one, on the
  left; bind address and the current, absolute root directory on the right.
  Click the root path to choose another directory from the recently used list.
- **Settings dialog**: validated with the same `ServerConfig::validate` rules as
  the daemon. Changes are stored for the next start or restart; the running
  server is not reconfigured in place. **About** shows the version, feature
  summary and third-party icon attribution.

Recently used roots are kept (most recent first, at most 8) in
`%APPDATA%\TftpdR\recent-dirs.txt` on Windows and
`$XDG_CONFIG_HOME/tftpd-r/recent-dirs.txt` (or `~/.config/tftpd-r/`) elsewhere.
The file only stores local paths; write failures are ignored so the panel still
works in read-only profiles.

Closing the window stops the embedded server and waits for active transfers,
bounded by `shutdown_grace_secs`.

### Icons

Toolbar, panel and dialog icons come from
[google/material-design-icons](https://github.com/google/material-design-icons)
(24×24 `materialicons` style, Apache License 2.0). Only the vector path data is
embedded as Slint `Path` commands, so the icons stay sharp at any scale. Source
URLs per icon and the full license text are in
[`crates/tftpd-r-gui-slint/ui/icons/README.md`](crates/tftpd-r-gui-slint/ui/icons/README.md)
and [`LICENSE`](crates/tftpd-r-gui-slint/ui/icons/LICENSE).

## Configuration

`config.example.toml` is the authoritative annotated example and is embedded in
the daemon (`print-default-config` prints exactly that file). Fields:

| Field | Default | Notes |
| --- | --- | --- |
| `bind_address` | `127.0.0.1:6969` | UDP. Keep loopback unless intentionally serving a LAN. Port 69 needs administrator/root. |
| `root_dir` | `tftp-root` | Must exist and be a directory. |
| `allow_upload` | `false` | Must stay `false` in this release; `true` is a validation error. |
| `overwrite` | `false` | Requires uploads; rejected in this release. |
| `max_file_bytes` | 64 MiB | Larger files are refused. |
| `block_size` | 512 | Maximum negotiated `blksize` (8..=65464). Raise for faster LAN transfers. |
| `timeout_secs` | 3 | Default retransmission timeout (1..=255); a client `timeout` option can override it. |
| `max_retries` | 5 | Retransmissions before a transfer fails (1..=20). |
| `max_sessions` | 16 | Concurrent transfers (1..=1024). Also capped at 8 per peer IP. |
| `max_transfer_duration_secs` | 3600 | Hard wall-clock limit per transfer. |
| `shutdown_grace_secs` | 5 | How long shutdown waits for sessions before aborting them (1..=60). |
| `event_buffer` | 1024 | In-memory event ring size (16..=65536). |

Unknown keys are rejected so typos do not silently fall back to defaults.

## Ports and firewall

- The server listens on **UDP** `bind_address` (default `127.0.0.1:6969`). Port
  `69` is privileged on Windows and Linux; use `6969` for unprivileged runs.
- Each transfer uses an additional **ephemeral UDP port** (the transfer ID) in
  the same address family as the listener. A firewall must allow inbound UDP to
  the listening port *and* the return traffic from those ephemeral ports. On
  Windows Defender Firewall this normally follows automatically for
  stateful/loopback traffic, but a restrictive policy may require an explicit
  rule; this was not tested in this repository.
- Binding `0.0.0.0` (or `[::]`) exposes the service to the network; there is no
  authentication in TFTP, so only the exported tree and read-only access limit
  what a client can reach.

## Security defaults

- Read-only: uploads are disabled and refused by validation.
- Loopback binding by default; no `unsafe` code in hand-written crates.
- Path sandbox rejects traversal, drive letters, device names, alternate data
  streams and reparse points; the final canonical path is compared
  component-wise with the canonical root. A leading `/` or `\` from a legacy
  client is treated as the root directory (`/file.bin` → `<root>/file.bin`),
  not as a host-absolute path; `..`, drive letters and reparse points are still
  rejected after that mapping.
- Concurrency, file size, retransmission count, transfer duration and event
  memory are bounded.
- Client-facing ERROR text never includes local paths or raw OS errors; log
  file names are sanitised and length-limited.
- Residual risks (notably a TOCTOU window between path checks and open, and
  spoofed UDP sources) are documented in [docs/security.md](docs/security.md).

## Tests

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

The test suite includes codec boundary tests, configuration/path-sandbox tests,
26 loopback-UDP RRQ integration tests (including block-number wraparound,
shutdown cancellation and legacy `/file` requests), daemon CLI/config
integration tests and GUI tests for the embedded controller (start/stop,
asynchronous restart, failed restart) and the recent-root persistence. See
[docs/testing.md](docs/testing.md) for what was actually executed and what still
needs manual/target-machine verification.

## Runtime dependencies

Both binaries link only against Windows system DLLs and the CRT provided by the
Rust toolchain, so no additional runtime or redistributable has to be installed.
The verification build used the GNU toolchain and therefore links `msvcrt.dll`;
an MSVC build would typically link the Visual C++ runtime instead.

Release-binary sizes measured on the verification machine are recorded in
[docs/testing.md](docs/testing.md). They are informational only and are not an
acceptance criterion.

## Documentation

- [docs/architecture.md](docs/architecture.md) — crate boundaries, lifecycle,
  data flow and threading.
- [docs/security.md](docs/security.md) — threat model, sandbox and limits.
- [docs/testing.md](docs/testing.md) — automated and manual verification.
- [docs/adr/](docs/adr/) — decision records, including deliberate deviations
  from the original implementation plan.
- [AGENTS.md](AGENTS.md) — development guide for follow-up coding agents:
  invariants, workflow, verification commands and roadmap (in Chinese).

## License

MIT OR Apache-2.0 (see `Cargo.toml`; no license files are bundled yet — add them
before distributing binaries). The bundled Material Design icon path data is
Apache-2.0; its license and per-icon sources are under
`crates/tftpd-r-gui-slint/ui/icons/`.
