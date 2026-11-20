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
| `cargo test --workspace` | **147 passed, 0 failed** |
| `cargo build --workspace --release` | Pass (59.6 s in the final run) |

The workspace was renamed from `rust-tftpd`/`tftp-*` to TftpdR/`tftpd-r-*` on
2026-11-20. Every command in this section was re-run on 2026-11-20 for `0.2.0`;
the results shown here are from that run (offline, GNU toolchain).
The GUI crate now contributes 39 tests (24 before the first-run / auto-restart
work): `product_icon_pixels_are_a_valid_rounded_tile` guards the embedded
window-icon pixels (size, transparent corners, `#263238` tile, `#fafafa`
glyph); `SetupState` guards the one-time setup flag; the address-toggle and
`RestartDisposition` tests cover the picker rules and the idle auto-restart
decision, and one test performs a real embedded-server restart.

The `0.2.0` release artifacts built on the verification machine (informational
only; not an acceptance criterion and not a promise for other
machines/toolchains):
`target/release/tftpd-r.exe` ≈ 2.07 MiB (2,174,976 bytes), `target/release/tftpd-r-gui.exe`
≈ 12.81 MiB (13,430,272 bytes; debug symbols stripped by the release profile).

The suite also runs on `ubuntu-latest` in CI. One core unit test
(`prefix_confusion_is_rejected_by_component_comparison`) therefore has a
Windows and a Unix variant, so the component-comparison invariant is checked on
both platforms instead of only where `C:\` paths are meaningful.

## CI and release verification (GitHub Actions)

The repository is public at
[github.com/greyskysoul/tftpd-r](https://github.com/greyskysoul/tftpd-r).
`.github/workflows/ci.yml` runs on every push; `.github/workflows/release.yml`
runs on a `v*` tag, or manually without publishing.

| Run | Result |
| --- | --- |
| CI on 2026-11-20 ([44681897220](https://github.com/greyskysoul/tftpd-r/actions/runs/44681897220)) | Pass: fmt, clippy `-D warnings` and `cargo test --workspace` on `windows-latest` (MSVC) and `ubuntu-latest` (GNU); GUI `cargo check` on Windows |
| Release dry run ([44681934004](https://github.com/greyskysoul/tftpd-r/actions/runs/44681934004)) | Pass: the same test gate, then all four targets built and packaged; the publish job was correctly skipped for a non-tag run |
| `v0.1.0` release ([44684633030](https://github.com/greyskysoul/tftpd-r/actions/runs/44684633030)) | Pass: [release published](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.1.0) with four archives plus `SHA256SUMS` |

What CI proves, and what it does not:

- The suite passes with the **MSVC** toolchain (`x86_64-pc-windows-msvc`) and on
  Linux (GNU). The hand-verified build documented above is still the GNU one.
- `aarch64-pc-windows-msvc` and `aarch64-unknown-linux-gnu` are built on native
  arm64 runners and packaged, but no test is executed there and no arm64 binary
  has ever been run.
- Archive contents were inspected for a Windows and a Linux artifact: each holds
  `tftpd-r`, `tftpd-r-gui`, `config.example.toml`, `README.md`, `LICENSE-MIT`
  and `LICENSE-APACHE`.
- The published `SHA256SUMS` was verified against a downloaded asset
  (`tftpd-r-0.1.0-aarch64-pc-windows-msvc.zip`, SHA-256 starting `8ee455c6`); it
  matched.

Archive and binary sizes in the `v0.1.0` release: Windows x64 zip 7.09 MiB,
Windows arm64 zip 6.76 MiB, Linux x64 tar.gz 9.19 MiB, Linux arm64 tar.gz
8.77 MiB. Uncompressed MSVC x64 binaries are `tftpd-r.exe` 2.28 MiB and
`tftpd-r-gui.exe` 13.36 MiB; Linux arm64 `tftpd-r` is 1.94 MiB and
`tftpd-r-gui` 15.76 MiB. They differ from the GNU numbers recorded above because
the toolchain and platform differ.

Test breakdown from `cargo test --workspace`:

| Target | Tests | What is covered |
| --- | --- | --- |
| `tftpd-r-core` unit tests (`src/lib.rs` target) | 63 | codec round-trips and malformed/oversized packets; option negotiation ranges, duplicates and clamping; configuration defaults/ranges/TOML errors, bind list emptiness/limit/multicast/broadcast rejection, wildcard collapsing and duplicate removal, shared-port summary; privileged-port bind classification and the non-forcing hint; path sandbox traversal, device names, legacy root-prefix mapping and root-directory rejection, symlink/reparse rejection, size limits; event ring drop/sequence/clear; error-code mapping |
| `tftpd-r-core` integration `tests/rrq.rs` | 29 | real loopback UDP server and client: small/exact-multiple/empty files, missing file, traversal, legacy `/file` and `\file` requests, root-prefixed traversal rejection, `blksize`/`tsize`/`timeout` negotiation, clamping, invalid option, duplicate ACK, retransmission, unknown TID ERROR 5, duplicate RRQ resend without a new session, WRQ rejection, netascii rejection, unknown mode/malformed/oversized datagrams, size limit, concurrent transfers, lifecycle events, shutdown cancellation, idempotent stop, retry exhaustion, **block-number wrap 65535 → 0** with `blksize = 8` (65,537 blocks); multiple listeners on one resolved port (IPv6 checked when available), wildcard collapsing to one listener, and an unbindable address aborting the whole start |
| `tftpd-r-daemon` unit tests | 10 | CLI parsing/defaults/overrides, embedded example template validity, config loading errors, unknown fields, override validation, status-channel failure path |
| `tftpd-r-daemon` integration `tests/cli.rs` | 5 | binary `print-default-config` equals `config.example.toml`; `check-config` success/missing file/uploads-rejected exit code 2; `serve` refuses invalid config with exit code 1; `serve` stays running with a valid config |
| `tftpd-r-gui-slint` unit tests | 39 | embedded controller start/stop, double-start rejection, invalid root, asynchronous restart (old server → new config), failed-restart error reporting and the no-stale-`Running`-after-stop guarantee; recent-root ordering, de-duplication, cap, persistence, canonicalisation, stale-directory cleanup and clearing the state file; one-time setup-flag persistence and read-only-profile tolerance; interface-address parsing/filtering/sorting; folder-selection validation; address picker model (two permanent entries, selected-but-undiscovered addresses preserved, duplicate removal, summaries) and the exclusive/sticky/fallback "全部网络" toggle rules; first-run all-network defaults; idle auto-restart with a real embedded server, deferral while a transfer is tracked, unchanged configuration and a deliberate stop; status-bar address summary and leaf-directory name |
| `tftpd-r-core` doc test | 1 | crate-level usage example compiles |

The block-wrap test transfers 65,537 blocks in roughly 3–4 seconds on the
verification machine.

### Deliberate limitations of the automated tests

- Timeouts use real timers (typically 1 s), not Tokio time pausing, so tests are
  slower than ideal and could be flaky on extremely loaded machines.
- Packet loss is not injected with a proxy; retransmission behaviour is tested by
  withholding ACKs deterministically.
- The GUI tests cover the controller, setup state, recent-root state, address
  toggles and the restart decision, not window rendering or real user
  interaction. Rendering and interaction were checked manually as
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

### 0.2.0 changes (2026-11-20, second run)

- **No console window.** Both binaries were checked at the PE-header level:
  `tftpd-r-gui.exe` has subsystem 2 (Windows GUI), `tftpd-r.exe` has subsystem 3
  (console). Debug and release `--smoke-test` still exit 0 with empty stderr, so
  the windowed subsystem did not break startup or shutdown.
- **Multi-listener daemon.** A temporary config with `port = 0` and
  `bind_addresses = ["127.0.0.1", "192.168.3.17"]` started the daemon; the log
  reported both listeners on the same resolved ephemeral port; a raw UDP client
  downloaded a 5,000-byte file as `/image.bin` from **each** address with
  identical bytes; Ctrl-Break produced `TFTP server stopped` and exit code 0.
- **GUI status bar / settings / picker.** `PrintWindow` captures of the panel at
  2× DPI showed: the stray horizontal line no longer crosses the status bar (the
  border is pinned with explicit `x`/`y`), the status bar shows `tftp-root` (last
  path component) instead of the absolute path, and the settings dialog with the
  new port field, 绑定地址 picker, 浏览… button and transfer limits. Opening the
  picker (`TFTPD_R_GUI_PREVIEW=picker`, a development-only environment variable)
  showed `全部网络 (0.0.0.0)`, `本地回环 (127.0.0.1)`, `192.168.3.17` and two
  global IPv6 addresses discovered through the .NET `NetworkInterface` API.
  `Get-NetIPAddress` is denied in this agent sandbox (WMI/CIM access), which is
  why discovery uses the .NET API instead.
- **Progress bar and transfer log.** The progress fill and the status-bar border
  shared the same centring bug; both now pin `x`/`y`. The table was rendered with
  two synthetic rows (`TFTPD_R_GUI_PREVIEW=transfer`) and the capture shows the
  blue fill anchored at the left edge (55 %) and an indeterminate row with an
  empty track and `—`, so the fill no longer grows from the middle. A new real
  transfer in the panel was not run in this session (the earlier run above
  already recorded the live progress row and the `开始传输` / `传输完成` log
  lines). Completion logging was additionally made unconditional so a finished
  transfer always leaves a log line.
- **Not re-verified:** clicking with a real mouse, the native folder dialog, the
  bind-failure hint in the GUI, and a real multi-NIC selection. Synthetic mouse
  input does not reach this winit window under the agent sandbox, so the dialogs
  were opened through the preview variable instead of a click.

### Product icon and header palette (2026-11-20, third run)

- **Icon artwork.** New master `crates/tftpd-r-gui-slint/ui/icons/app-icon.svg`:
  the Material `dns` glyph in `#fafafa` on a `#263238` rounded square (radius
  56/256) with a soft top-light sheen. The `dns` path data is the same string as
  `Icons.dns`. Edge/Chromium cannot start inside the agent sandbox (its
  multi-process model needs named pipes), so the SVG was rasterised in-process
  with WPF; the rasteriser asserted the parsed glyph bounds (3,3)–(21,21) so a
  mangled path could not silently produce a wrong icon. Exports: `app-icon.png`
  (256 px), `app-icon.ico` (16–256 px) and `app-icon-128.rgba` (raw pixels for
  the window icon).
- **Window and task-bar icon.** The panel uploads the embedded RGBA buffer
  through `Window.icon` and the running window was probed with `WM_GETICON`:
  ICON_SMALL and ICON_SMALL2 return a **128x128** icon whose pixels match
  `app-icon.png` (ICON_BIG stays 0 for this window, which is why the probe reads
  the small icon too).
- **Header palette.** `PrintWindow` captures at 1x show the title bar and the
  toolbar as one flat `#37474f` block with a `#2b3a41` divider between the rows,
  the `#263238`/`#fafafa` brand mark at 22 px on the left of the title bar
  (previously a 20 px `#2563eb` badge), the toolbar icons/labels on the
  blue-grey tints and the About dialog's 54 px mark. Panel surfaces carry no
  gradients; only the exported icon files do.
- **Failure mode found and removed in this run.** Embedding the icon with
  `slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer` makes the Slint
  compiler pre-render glyphs at build time, which left every Chinese string and
  every runtime log line blank in the panel. The build keeps the default embed
  mode and passes the icon as a pixel buffer instead
  (`crates/tftpd-r-gui-slint/build.rs`, `app_icon()` in `src/main.rs`).
- **Smoke tests.** `tftpd-r-gui --smoke-test` exits 0 with empty stderr and empty
  stdout in both the debug and the release build after the change.
- **Not verified in this session:** the window controls (minimize / maximize /
  close) are not visible in any capture here — the same holds for the unmodified
  `main` build captured in the same session, so it is a property of this agent
  desktop, not of the change: it reports 200 % scaling on a 1560x1040 physical
  screen, so the 1120x780 *logical* window is laid out larger than its window and
  the right/bottom edges are clipped. Captures therefore used
  `SLINT_SCALE_FACTOR=1` plus a forced window size. Real mouse interaction with
  the new palette, other DPI scales and the Windows Explorer `.exe` icon were not
  exercised.

### GUI first-run, address picker, history clear and idle auto-restart (2026-11-20, fourth run)

The panel was built in debug mode and run with an isolated `APPDATA` so every
scenario started from a fresh profile. `SLINT_SCALE_FACTOR=1` was set because
this agent desktop reports 200 % scaling; captures are `PrintWindow` images.

- **First run.** With no `config.toml` in the working directory, the settings
  dialog opened automatically on startup. The bind summary showed
  `全部网络 (0.0.0.0)`, the status bar showed `0.0.0.0:69`, and the activity log
  contained the first-run hint. `ui-state.txt` was created with
  `setup-shown=true`; a second launch with the same profile showed the main
  window without the dialog. A fresh profile with a **valid** `config.toml`
  started on the main window and did not create `ui-state.txt`.
- **Address picker.** Clicks were posted as window messages (the method used in
  the earlier runs; this time they reached the window). Starting from
  "全部网络" selected: clicking `192.168.3.17` cleared the wildcard; clicking
  `127.0.0.1` then added it (summary `本地回环 (127.0.0.1)、192.168.3.17`);
  clicking "全部网络" cleared both; clicking "全部网络" again kept it checked;
  unchecking the last specific address (127.0.0.1) selected the wildcard again.
  Every state was checked in a capture; no capture showed an empty selection.
- **Clear history.** With two seeded recent roots, the drop-down showed the new
  清空 button next to the close button. Clicking it removed
  `recent-dirs.txt`, logged `最近使用的根目录已清空。`, rendered the empty-state
  hint and disabled the button. The configured root in the status bar did not
  change.
- **Idle auto-restart.** A temporary `config.toml` (`port = 56971`,
  `bind_addresses = ["127.0.0.1"]`, `root_dir = "share1"`) plus two directories
  was used. Starting the server and then selecting `share2` in the drop-down
  logged `根目录已切换为 …share2；当前无进行中的传输，正在自动重启服务。`
  followed by `服务已就绪：监听 127.0.0.1:56971，根目录 …share2。`. A raw UDP
  client downloaded `two.txt` (5 bytes) from `share2`; `one.txt` (only in
  `share1`) returned `ERROR File not found`. The status bar showed the new root
  and the toolbar returned to 运行中.
- **No restart during a transfer.** A slow raw client (512-byte blocks, 20 ms
  between ACKs, 256 KiB file) kept one transfer active. Selecting `share2`
  logged the warning `根目录已切换为 …share2；有 1 个进行中的传输，未自动重启，
  稍后请手动重启。`; the server stayed on `share1` and the transfer row kept
  progressing (80.3 % captured after the switch, 90 % later). The test client
  itself timed out near the end of that deliberately slow transfer; the panel
  never restarted or dropped the session during the observation window.
- **Not verified in this run:** real mouse hardware (the posted messages are
  synthetic), keyboard/IME editing of the dialog fields, other DPI scales, and
  the native folder dialog. The file transfer used the in-repo raw Python client,
  not an external TFTP implementation.

## Not verified (and therefore not claimed)
- **External TFTP client interoperability** (tftp-hpa, Tftpd64, PXE firmware,
  U-Boot, etc.). Only the in-repo test client and the raw Python client were
  used.
- **MSVC runtime behaviour.** CI compiles and tests `x86_64-pc-windows-msvc` and
  compiles `aarch64-pc-windows-msvc`, but no MSVC binary has been run by hand on
  a target machine; only the GNU build above was.
- **Linux/macOS runtime.** CI runs the full suite on Linux, but the GUI has never
  been run on a Linux desktop, and macOS is untested.
- **LAN/deployment behaviour:** binding `0.0.0.0`, Windows Defender Firewall
  rules for TID ports, RDP sessions, concurrent real-world clients. Port 69 was
  actually bound by the daemon on this Windows machine (Windows normally allows
  low ports), so the "permission denied" hint was exercised only by unit tests,
  not by a real failed bind; the Linux root/`CAP_NET_BIND_SERVICE` path is
  untested.
- **GUI helper features:** the native folder dialog was not interacted with, and
  address discovery was exercised only through the .NET API on this machine
  (not on Linux, not with VPN adapters appearing/disappearing, and not through
  the `hostname -I` fallback).
- **Release artifacts:** sizes and checksums are recorded above; they are
  informational for the recorded build and are not a guarantee for other
  machines or toolchains.
- **Executable icon.** Neither `.exe` embeds the product icon as a PE resource
  yet, so Windows Explorer shows the generic executable icon. The running
  window/task-bar icon and the `app-icon.ico` shipped in the archives carry the
  artwork; adding a resource compiler belongs to the packaging work (Phase 8).
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
   address and the root *name* in the status bar, and that Chinese text renders
   without missing glyphs. Confirm that no console window appears.
3. Start the server, download a file with a real TFTP client, confirm the file
   hash matches and the transfer appears/disappears in the 传输列表 with a live
   progress bar.
4. Use 重启 while a transfer is running or immediately after one; confirm the
   client sees the expected disconnect/retry behaviour and the UI reaches
   `运行中` again without freezing.
5. Stop the server during a large transfer; confirm the client sees an error and
   the UI reaches `已停止` without freezing.
6. Open the settings dialog and type into every field (including Chinese IME
   composition); select several bind addresses (including a real LAN address),
   use 浏览… to pick a root directory, save, then restart and confirm the new
   values are used. Try port 69 without administrator/root and confirm the
   failure hint appears without any automatic fallback; then switch to an
   unprivileged port.
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
