# TftpdR

[![CI](https://github.com/greyskysoul/tftpd-r/actions/workflows/ci.yml/badge.svg)](https://github.com/greyskysoul/tftpd-r/actions/workflows/ci.yml)

TftpdR is a read-only TFTP (RFC 1350) server written in Rust, with a headless
daemon and a native [Slint](https://slint.dev/) desktop panel.

> **Status: MVP (Phase 0–5 vertical slice).** RRQ downloads, option negotiation,
> the path sandbox, the daemon CLI and the embedded-mode GUI are implemented and
> tested. WRQ uploads, daemon IPC and Windows Service packaging are explicitly
> **not** part of this release. See [Scope](#scope-and-non-goals).

## Downloads

Prebuilt archives for Windows and Linux on x64 and arm64 are attached to the
[latest release](https://github.com/greyskysoul/tftpd-r/releases/latest). Each
archive contains the `tftpd-r` daemon, the `tftpd-r-gui` panel,
`config.example.toml`, `README.md` and both license files. Both Windows
executables embed the product icon as a PE resource, and the Windows archives
also ship `tftpd-r-gui.ico` for tools that want the file; `SHA256SUMS` covers
every asset.

The protocol engine and the daemon are portable; the desktop panel needs a
graphical session (an X11 or Wayland session on Linux).

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
- One or more UDP listen addresses sharing a single port (default
  `127.0.0.1:69`, the standard TFTP port). Binding is all-or-nothing; a
  permission error on a privileged port produces a non-forcing hint to choose
  an unprivileged port.
- Bounded concurrency (global and per-peer), per-transfer deadline, file-size
  limit and bounded event ring.
- Headless daemon: TOML configuration, `check-config`, `print-default-config`,
  structured logs and Ctrl-C / Ctrl-Break / SIGTERM graceful shutdown.
- Slint GUI (embedded mode): frameless window with a custom dark title bar and
  window controls; toolbar with a prominent service-status chip plus
  start/restart, stop, settings and about; status bar with bind address and a
  clickable recently-used root selector; activity log and active transfers;
  settings validation. The panel starts with no root directory until one is
  chosen (the status bar says 未指定根目录 and the settings field is empty) and
  restores the root of the previous start on the next launch. The header is one
  flat `#37474f` block carrying the product mark; the same artwork
  (`crates/tftpd-r-gui-slint/ui/icons/app-icon.svg`, Material `dns` glyph) is the
  window and task-bar icon and is embedded in both Windows executables as their
  PE icon. Layout and icons are in `crates/tftpd-r-gui-slint/ui/`.

Not implemented in this release (deliberate, tracked in `docs/adr/`):

- **WRQ uploads.** `allow_upload = true` is rejected by configuration
  validation; the server answers WRQ with ERROR 2 (access violation). Uploads
  need a separate security review and atomic-temp-file design first.
- **IPC / GUI-managed daemon.** The GUI embeds the server in its own process
  (`docs/adr/0004-embedded-only.md`). There is no named-pipe protocol yet.
- **Windows Service registration**, system tray, automatic updates, remote
  management and user/role management.
- `netascii` mode (`docs/adr/0006-octet-only.md`).

Port 69 as the default (with a non-forcing hint when privileges are missing) and
multi-select bind addresses shipped in `0.2.0` and are recorded in
[docs/roadmap.md](docs/roadmap.md) and
[docs/adr/0007](docs/adr/0007-default-port-69.md) /
[docs/adr/0008](docs/adr/0008-multiple-bind-addresses.md); the GUI first-run,
history-clear and idle auto-restart behaviour is in
[docs/adr/0009](docs/adr/0009-gui-first-run-and-auto-restart.md). They are **not**
part of 0.1.0. Configuration stays TOML; an INI format with a fixed location was
considered and dropped.

The GUI root-selection rules (unset state, remembered last root, recent-menu
选择…) are recorded in [ADR 0012](docs/adr/0012-gui-root-selection-and-memory.md),
and the activity-log timestamps, display filter and explicit text export in
[ADR 0013](docs/adr/0013-gui-activity-log.md). The native folder dialog is served
by a background helper that is pre-warmed when the settings dialog or the
recent-root menu opens, so the click does not pay for PowerShell startup
([ADR 0014](docs/adr/0014-prewarmed-folder-picker-helper.md)).

## Requirements

- Rust **1.100.0** is the toolchain used to develop and verify this workspace.
  An MSRV is not yet claimed; older compilers have not been tested.
- Windows 10/11 is the primary target. The protocol engine is platform-neutral
  and the full test suite also runs on Linux in CI; the desktop panel has only
  been run by hand on Windows so far.
- On Windows, a linker is required: the MSVC Build Tools (`link.exe`) for the
  default `x86_64-pc-windows-msvc` toolchain, or MinGW-w64 for
  `x86_64-pc-windows-gnu`. GNU builds also use `windres` from MinGW-w64 binutils
  to compile the product icon into the executable (MSVC links the generated
  `.res` directly). The verification described in
  [docs/testing.md](docs/testing.md) used the GNU toolchain with MinGW-w64.

## Build

```powershell
cargo build --workspace            # debug
cargo build --workspace --release  # release binaries in target\release
```

Release builds use a size-first profile (`opt-level = "z"`, fat LTO,
`panic = "abort"` and symbol stripping). On the GNU x64 verification build this
gives about 1.29 MiB for the daemon and 8.50 MiB for the GUI, including the
~110 KiB Windows icon resource. `panic = "abort"` is a deliberate fail-fast
choice: a panic in a spawned task terminates the process instead of being
contained by Tokio. Debug and test builds keep the default unwind strategy. See
[ADR 0010](docs/adr/0010-size-first-release-profile.md) and
[ADR 0011](docs/adr/0011-windows-executable-icon.md).

The workspace contains three runtime crates and one build-time helper:

| Crate | Purpose |
| --- | --- |
| `tftpd-r-core` | Protocol, session engine, sandbox, configuration, events. No GUI/daemon/IPC dependency. |
| `tftpd-r-daemon` | Headless server binary: CLI, TOML config, logging, graceful shutdown. |
| `tftpd-r-gui-slint` | Native Slint desktop panel; embeds `tftpd-r-core` (no daemon required). |
| `tftpd-r-build-support` | Build-time only: embeds the product icon in both Windows executables. Not linked into either binary. |

## Quick start (daemon)

```powershell
# 1. Create the exported directory (the example config points here).
New-Item -ItemType Directory -Force tftp-root | Out-Null
Set-Content tftp-root\hello.txt "hello over tftp"

# 2. Validate the configuration (does not bind a socket).
Copy-Item config.example.toml config.toml
cargo run -p tftpd-r-daemon -- check-config --config config.toml

# 3. Run the server in the foreground; Ctrl-C (or Ctrl-Break) stops it gracefully.
#    Port 69 needs root/CAP_NET_BIND_SERVICE on Linux; Windows normally permits
#    a standard user unless the port is reserved. For a high-port trial run:
cargo run -p tftpd-r-daemon -- serve --config config.toml --bind 127.0.0.1:6969
```

Then point any TFTP client at `127.0.0.1:69` (or `127.0.0.1:6969` with the
override) and request `hello.txt`. The server is read-only: uploads are
refused. Binding port 69 without the required privileges fails with a message
that suggests an unprivileged port; the server never falls back or elevates on
its own.

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

The GUI loads `config.toml` from the current directory when present and never
rewrites it. On the first start without a usable configuration it opens the
settings dialog automatically (once per user profile) so the root directory and
bind addresses can be filled in; the form preselects "全部网络 (0.0.0.0)". The
daemon and `ServerConfig::default()` keep their conservative loopback default.

The panel has **no default root directory**. Until one is chosen, the status bar
says `未指定根目录` and the settings field is empty; 启动/保存 asks for a
directory instead of trying a placeholder such as `tftp-root`. The root of the
last successful start (or the last root saved in the dialog) is remembered in
`%APPDATA%\TftpdR\last-root.txt` (XDG config directory elsewhere) and restored
on the next launch when no valid `config.toml` provides one. A valid
`config.toml` always wins, so the file remains the authoritative way to pin the
root for a deployment. The panel is in Chinese and is organized as follows:

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
  left; the bind address(es) and the current root directory (last path
  component, or `未指定根目录`) on the right. Click the root name to choose
  another directory from the recently used list; the drop-down has a 选择…
  button that opens the native folder dialog and applies the chosen directory
  immediately (restarting an idle running server), and a 清空 button that forgets
  the history (the configured/running root is not affected).
- **Settings dialog**: validated with the same `ServerConfig::validate` rules as
  the daemon. A port field plus a multi-select address picker (always offering
  "全部网络 (0.0.0.0)" and "本地回环 (127.0.0.1)" and the addresses discovered on
  this machine), a directory picker button for the root, and the transfer
  limits. The root field is empty when no directory has been selected and must
  not be left empty when saving. Choosing a directory with 浏览… only fills the
  field; the form then shows an inline reminder and the selection is applied and
  remembered by 保存设置. (The recent-root menu's 选择… applies immediately
  instead.) Every selected address shares the single port field. The wildcard is
  exclusive: selecting "全部网络" clears the specific addresses, selecting a
  specific address clears the wildcard, and the wildcard cannot be unchecked
  (unchecking the last specific address selects 全部网络 again), so at least one
  bind address is always selected. **About** shows the
  version, feature summary and third-party icon attribution.
- **Automatic restart**: saving settings, or switching the root while the server
  is running, restarts it automatically when no transfer is active. With active
  transfers the new values are stored and a warning explains that a manual
  restart is required; the running downloads are never interrupted for a
  configuration change.
- **Activity log**: every transfer logs a start line and a completion or failure
  line (file, peer, size and duration), even when the start event was already
  evicted from the bounded event ring. Lines carry the local wall-clock time and
  are single-line. The header filters the table by exact level
  (全部 / INFO / WARN / ERROR); the filter only hides rows, so 全部 restores
  them and the export stays complete. 导出 opens a native save dialog and writes
  all retained entries (up to 500, oldest first) with the version, export time
  and row count. Opening a picker is not logged; exported files contain local
  paths and peer addresses and are only created by this explicit action.

Recently used roots are kept (most recent first, at most 8) in
`%APPDATA%\TftpdR\recent-dirs.txt` on Windows and
`$XDG_CONFIG_HOME/tftpd-r/recent-dirs.txt` (or `~/.config/tftpd-r/`) elsewhere.
The root of the last successful start is stored next to it in `last-root.txt`,
and the one-time first-run flag in `ui-state.txt`. If the per-user directory
cannot be created (restricted profile or a low-integrity/sandboxed launch), the
panel uses `.tftpd-r` beside `tftpd-r-gui.exe`, then `.tftpd-r` in the working
directory, and logs a WARN naming the location. This is a Windows integrity-level
restriction, not an installation or ACL problem: a low-integrity process cannot
write the medium-integrity profile directory even though the account owns it.
Code signing is unrelated: it affects SmartScreen reputation and
WDAC/AppLocker allowlisting, not the right to write `%APPDATA%`. The state
location is chosen **per process, not per executable path**: the same binary
writes `%APPDATA%\TftpdR` when started from a normal Medium-integrity desktop
and uses `.tftpd-r` when started from a low-integrity/sandboxed terminal.
Moving the executable to the desktop does not change permissions by itself;
launching it from Explorer does. The two locations are independent, so a root
remembered in one is not visible in the other.
Clearing the recent list does not forget `last-root.txt`. These files only store
local UI state; the panel still works in a read-only profile, but a failed
`last-root.txt` write is now reported as WARN (and a stored root that no longer
exists is reported on the next launch) instead of being silently forgotten.

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
| `port` | `69` | UDP port shared by every listen address. On Linux port 69 needs root (`CAP_NET_BIND_SERVICE`); Windows normally allows it for a normal user unless the port is reserved. A process that may not bind it gets an actionable failure and must choose another port (for example `6969`); nothing changes automatically. |
| `bind_addresses` | `["127.0.0.1"]` | One or more interface addresses (at most 16). `0.0.0.0`/`::` listen on every interface of that family; other addresses of the same family are then redundant and ignored. Multicast/broadcast are rejected. Keep loopback unless intentionally serving a LAN. |
| `root_dir` | `tftp-root` | Must exist and be a directory. This is the TOML/daemon default and the value used by `config.example.toml`; the GUI panel itself starts with no root selected until one is chosen. |
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
Configurations written for 0.1.0 still use `bind_address = "127.0.0.1:6969"`;
that key was replaced by `port` plus `bind_addresses` and is now rejected with
an unknown-field error. See [docs/roadmap.md](docs/roadmap.md) and
[docs/adr/0008](docs/adr/0008-multiple-bind-addresses.md) for the rationale.

## Ports and firewall

- The server listens on **UDP** on every configured `bind_addresses` entry at
  `port` (default `127.0.0.1:69`). Port `69` is the product default: Linux
  requires root or `CAP_NET_BIND_SERVICE`, while Windows normally allows a normal
  user to bind it unless it is reserved by the system. A process that may not
  bind the port gets a hint naming an unprivileged port (for example `6969`) but
  nothing is changed automatically. If several addresses bind the same port, a
  firewall rule is needed for each reachable address.
- Each transfer uses an additional **ephemeral UDP port** (the transfer ID) in
  the same address family as the listener that received the request. A firewall
  must allow inbound UDP to the listening port *and* the return traffic from
  those ephemeral ports. On Windows Defender Firewall this normally follows
  automatically for stateful/loopback traffic, but a restrictive policy may
  require an explicit rule; this was not tested in this repository.
- Binding `0.0.0.0` (or `[::]`) exposes the service to the network; there is no
  authentication in TFTP, so only the exported tree and read-only access limit
  what a client can reach. Binding a list of specific addresses never exposes
  more interfaces than the listed ones, and a partial bind failure fails the
  whole start instead of quietly serving fewer addresses.

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
29 loopback-UDP RRQ integration tests (including block-number wraparound,
shutdown cancellation, legacy `/file` requests, multiple listeners on one port
and all-or-nothing binding), daemon CLI/config integration tests and GUI tests
for the embedded controller (start/stop, asynchronous restart, failed restart),
the recent-root persistence and clearing, the one-time setup flag,
interface-address parsing, folder-selection validation and the pre-warmed
folder-picker protocol/UTF-8 handling, the exclusive address-picker model and the
idle/deferred auto-restart decision. See [docs/testing.md](docs/testing.md) for
what was actually executed and what still needs manual/target-machine
verification.

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

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT) at your option (see `Cargo.toml`). The bundled
Material Design icon path data is Apache-2.0; its license and per-icon sources
are under `crates/tftpd-r-gui-slint/ui/icons/`.
