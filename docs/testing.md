# Testing and verification

This document records what was **actually executed** for this MVP, with the
environment used, and what still requires manual or target-machine verification.
It is not a wish list.

## Environment used for verification

| Item | Value |
| --- | --- |
| OS | Windows 11 (build 10.0.26300) |
| Rust | `rustc 1.100.0 (0ef1bc386 2026-11-09)`, host `x86_64-pc-windows-gnu` |
| Cargo | `cargo 1.100.0 (3acc79c0c 2026-11-05)` |
| Linker | MinGW-w64 GCC 15.3.0 (winlibs, msvcrt runtime) |
| Slint | 1.19.0, features `std`, `backend-winit`, `renderer-software`, `software-renderer-systemfonts`, `compat-1-2` |
| Date | 2026-11-20 |

The development environment used a portable toolchain stored in
`.dsh-toolchain/` (git-ignored) because the agent sandbox could not perform
Schannel TLS directly. This is **not** part of the repository; a normal Rust
installation is sufficient. Dependency downloads were therefore performed once
with network access; subsequent builds/tests ran with `--offline`.

## Automated tests and checks

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass (exit 0, no warnings) |
| `cargo test --workspace` | **105 passed, 0 failed** |
| `cargo build --workspace --release` | Pass (≈22 s incremental in the final run) |

The workspace was renamed from `rust-tftpd`/`tftp-*` to TftpdR/`tftpd-r-*` on
2026-11-20. Every command in this section was re-run on 2026-11-20 after the GUI
redesign; the results shown here are from that run (offline, GNU toolchain).

Release artifacts on the verification machine (informational only; not an
acceptance criterion and not a promise for other machines/toolchains):
`target/release/tftpd-r.exe` ≈ 2.04 MiB, `target/release/tftpd-r-gui.exe`
≈ 12.43 MiB (debug symbols stripped by the release profile).

Test breakdown from `cargo test --workspace`:

| Target | Tests | What is covered |
| --- | --- | --- |
| `tftpd-r-core` unit tests (`src/lib.rs` target) | 53 | codec round-trips and malformed/oversized packets; option negotiation ranges, duplicates and clamping; configuration defaults/ranges/TOML errors; path sandbox traversal, device names, legacy root-prefix mapping and root-directory rejection, symlink/reparse rejection, size limits; event ring drop/sequence/clear; error-code mapping |
| `tftpd-r-core` integration `tests/rrq.rs` | 26 | real loopback UDP server and client: small/exact-multiple/empty files, missing file, traversal, legacy `/file` and `\file` requests, root-prefixed traversal rejection, `blksize`/`tsize`/`timeout` negotiation, clamping, invalid option, duplicate ACK, retransmission, unknown TID ERROR 5, duplicate RRQ resend without a new session, WRQ rejection, netascii rejection, unknown mode/malformed/oversized datagrams, size limit, concurrent transfers, lifecycle events, shutdown cancellation, idempotent stop, retry exhaustion, **block-number wrap 65535 → 0** with `blksize = 8` (65,537 blocks) |
| `tftpd-r-daemon` unit tests | 10 | CLI parsing/defaults/overrides, embedded example template validity, config loading errors, unknown fields, override validation, status-channel failure path |
| `tftpd-r-daemon` integration `tests/cli.rs` | 5 | binary `print-default-config` equals `config.example.toml`; `check-config` success/missing file/uploads-rejected exit code 2; `serve` refuses invalid config with exit code 1; `serve` stays running with a valid config |
| `tftpd-r-gui-slint` unit tests | 10 | embedded controller start/stop, double-start rejection, invalid root, asynchronous restart (old server → new config) and failed-restart error reporting; recent-root ordering, de-duplication, cap, persistence, canonicalisation and stale-directory cleanup |
| `tftpd-r-core` doc test | 1 | crate-level usage example compiles |

The block-wrap test transfers 65,537 blocks in roughly 3–4 seconds on the
verification machine.

### Deliberate limitations of the automated tests

- Timeouts use real timers (typically 1 s), not Tokio time pausing, so tests are
  slower than ideal and could be flaky on extremely loaded machines.
- Packet loss is not injected with a proxy; retransmission behaviour is tested by
  withholding ACKs deterministically.
- The GUI tests cover the controller and recent-root state, not window rendering
  or real user interaction. Rendering and interaction were checked manually as
  described below; that check is not an automated acceptance test.

## Manual end-to-end verification performed

1. **Daemon + real UDP client.**
   - Built `target/debug/tftpd-r.exe`.
   - Started `tftpd-r serve --config <temp config>` with
     `bind_address = "127.0.0.1:0"` and a 5,000-byte file `image.bin`.
   - A raw UDP TFTP client (Python, 512-byte `octet` RRQ, sequential ACKs)
     downloaded the file **as `/image.bin`**, i.e. with the legacy root prefix
     that the user's old client sends, and the bytes matched exactly.
   - Sent `CTRL_BREAK_EVENT` to the daemon process group: it logged
     `shutdown requested signal=Ctrl-Break` and `TFTP server stopped`, and exited
     with code 0.
   - This exercises CLI parsing, config loading, socket binding, RRQ, events,
     logging, signal handling and graceful shutdown together.
2. **GUI smoke run (debug and release).**
   - `target/debug/tftpd-r-gui.exe --smoke-test` and
     `target/release/tftpd-r-gui.exe --smoke-test` create the Slint window, run
     the 200 ms UI timer, and quit after 1.5 s.
   - Observed: both exited with code 0, no stderr output or panic. This verifies
     the winit/software-renderer backend initialises and the generated UI
     compiles and instantiates on this machine.
   - It is **not** a substitute for interactive acceptance by a person.
3. **GUI panel after the redesign (manual, 2026-11-20).**
   - Launched the debug panel on the verification desktop at the system scale
     factor (the frameless client was 1120×780 logical / 2240×1560 physical,
     i.e. 2×). Chinese labels rendered with the system font; the Material icon
     path data rendered as vectors.
   - The frameless window shows the custom title bar integrated with the dark
     toolbar: app name, drag area and minimize / maximize-restore / close
     buttons, plus a one-pixel dark window frame. Maximize and restore through
     the title-bar button were confirmed with `IsZoomed`, minimize with
     `IsIconic(true)` followed by an external `SW_RESTORE`, and the close button
     exited the process with code 0 (the normal window-close shutdown path).
   - Checked the idle and running toolbars: the service-status chip is the first
     element, grey with 已停止 while stopped and green with 运行中 after starting.
     启动 is enabled and 停止 greyed out while stopped; after starting, the
     primary button becomes 重启 with the restart icon and 停止 becomes enabled.
     The bind address and absolute root path are shown in the bottom bar.
   - Occupied UDP 6969 with a separate process, started the server and observed
     the failure state: the chip turns red with 启动失败, the log records the bind
     error, and the bottom-left status bar shows the error detail instead of the
     read-only hint.
   - Opened the settings dialog: all five fields showed the current
     configuration, the focus border appeared on the clicked field, and the
     disabled upload checkbox rendered. The inline error box layout was checked
     by temporarily injecting an error string (the field could not be edited
     with synthetic key messages); the dialog grows from 448 px to 500 px so the
     footer is not clipped. Opened the About dialog: version, feature summary and
     icon attribution are visible. The About copy was trimmed after that run
     (one implementation-focused feature row removed, wording simplified); that
     edit was verified by `cargo build --release` and `--smoke-test` (exit 0)
     only — the card is a fixed 560×440 px with a vertical stretch spacer, so
     the shorter content cannot clip the footer, but no screenshot was re-taken.
   - Seeded two recent roots through a workspace-scoped `APPDATA` (the test
     harness cannot write the real profile directory under the agent sandbox),
     opened the root drop-down from the status bar and selected the second
     directory. The status bar and the log updated, and starting the server
     afterwards served `other.txt` from the new root. Opening, dismissing by
     clicking outside and reopening the drop-down was repeated to confirm the
     press-based open/dismiss handling.
   - Started a raw 512-byte-block TFTP client for a 2 MiB file: the transfer row
     showed file, peer, a live progress bar, `1.09 MiB / 2.00 MiB` and
     `传输中`; after completion the row disappeared and the log recorded
     `传输完成 …（2.00 MiB），耗时 22.6 秒`. A later 22-byte request also
     succeeded after a restart.
   - Clicked 重启 while running: the log recorded `正在重启服务` followed by
     `服务已就绪`, and the server accepted a new download afterwards.
   - The window was inspected through `PrintWindow` captures. Clicks were
     injected as window messages because this agent session has no interactive
     desktop input; this verifies the Slint handlers and state transitions, but
     it is not a substitute for a person clicking with a real mouse.

## Not verified (and therefore not claimed)

- **External TFTP client interoperability** (tftp-hpa, Tftpd64, PXE firmware,
  U-Boot, etc.). Only the in-repo test client and the raw Python client were
  used.
- **MSVC toolchain build.** Only the `x86_64-pc-windows-gnu` host was built and
  tested here. The code contains no `unsafe` and no Windows-API calls outside
  the reparse-point attribute check, so MSVC is expected to work, but that is
  not measured.
- **Linux/macOS runtime.** `.github/workflows/ci.yml` configures Linux and
  Windows runners, but no CI run has been executed for this repository yet.
- **LAN/deployment behaviour:** binding `0.0.0.0`, Windows Defender Firewall
  rules for TID ports, port 69 privileges, RDP sessions, concurrent real-world
  clients.
- **Release artifacts:** see the release build section; binary size and RSS are
  not acceptance criteria and are not published as guarantees.
- **Keyboard/IME input in the settings fields.** The fields received focus and
  rendered correctly, but the synthetic `WM_CHAR` used here was not conclusive
  for normal typing or Chinese IME composition; this needs a person on a target
  desktop.
- **Mouse dragging of the frameless window and edge/corner drag-resize.** The
  `WindowMoveArea` and `resize-border-width` backend paths were not exercised
  with a real mouse drag; only programmatic maximize/restore and the fixed
  initial size were verified. Snap layouts and double-click-to-maximize on the
  custom drag area are also untested.
- **Font rendering beyond this machine:** other DPI scales, Windows locale
  settings, Linux font stacks and screen readers/accessibility roles.
- **Visual sign-off by a designer/user** on the new layout; the screenshots above
  are evidence of rendering, not an approved design review.
- **Windows Service, IPC, uploads** — not implemented in this release.
- **Fuzz/property testing** of the codec; tests are table-driven and
  boundary-focused instead.

## Manual acceptance checklist for a target machine

1. `cargo test --workspace` and `cargo build --workspace --release` on the target
   toolchain (MSVC and, if used, GNU).
2. Launch the GUI with a real `config.toml`; check the custom title bar, the
   window controls (minimize / maximize-restore / close), edge and corner drag
   resizing, the service-status chip at the far left of the toolbar, the bind
   address and root in the status bar, and that Chinese text renders without
   missing glyphs.
3. Start the server, download a file with a real TFTP client, confirm the file
   hash matches and the transfer appears/disappears in the 传输列表 with a live
   progress bar.
4. Use 重启 while a transfer is running or immediately after one; confirm the
   client sees the expected disconnect/retry behaviour and the UI reaches
   `运行中` again without freezing.
5. Stop the server during a large transfer; confirm the client sees an error and
   the UI reaches `已停止` without freezing.
6. Open the settings dialog and type into every field (including Chinese IME
   composition); save, then restart and confirm the new values are used.
7. Click the root path in the status bar and switch between at least two recent
   directories; confirm the status bar, log and served files follow the selected
   directory.
8. Try `../`, a root-prefixed traversal (`/../file`), a drive path, a symlink
   inside the root and an oversized file; confirm the client receives an error
   and the server log shows a rejection. Also request `/file.bin` with the old
   client and confirm it is served from the configured root.
9. Flood events (large file, small `blksize`) and confirm the UI stays
   responsive and log/transfer lists stay bounded.
10. Close the window with the custom close button during an active transfer;
    confirm the process exits and no server task keeps running.
11. Verify firewall behaviour for the listening port and ephemeral TID ports.

## Reproducing the checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release

# GUI smoke test (opens a window for ~1.5 s)
cargo run -p tftpd-r-gui-slint -- --smoke-test
```
