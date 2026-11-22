# Testing and verification

This document is the evidence log for checks that were **actually executed** and
the acceptance matrix for work that remains. Historical results are kept
separate from future test recommendations: a listed test idea is not evidence
that the test or feature already exists. The current phase order and test gates
are defined in [roadmap.md](roadmap.md).

## Environment used for verification

| Item | Value |
| --- | --- |
| OS | Windows 11 (build 10.0.26300) |
| Rust | `rustc 1.100.0 (0ef1bc386 2026-11-09)`, host `x86_64-pc-windows-gnu` |
| Cargo | `cargo 1.100.0 (3acc79c0c 2026-11-05)` |
| Linker | MinGW-w64 GCC 15.3.0 (winlibs, msvcrt runtime); `windres` 2.46.1 from the same binutils for the executable icon |
| Slint | 1.19.0, features `std`, `backend-winit`, `renderer-software`, `software-renderer-systemfonts`, `compat-1-2` |
| Release profile | `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"` (ADR 0010; measured 2026-11-21) |
| Date | 2026-11-20 (size-profile, executable-icon and `v0.2.1` release checks added 2026-11-21; log drop-down and unified header added 2026-11-22) |

The development environment used a portable toolchain stored in
`.dsh-toolchain/` (git-ignored) because the agent sandbox could not perform
Schannel TLS directly. This is **not** part of the repository; a normal Rust
installation is sufficient. Dependency downloads were therefore performed once
with network access; subsequent builds/tests ran with `--offline`.

## Automated tests and checks

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass (exit 0) |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | Pass (exit 0, no warnings) |
| `cargo test --workspace --offline` | **177 passed, 0 failed, 1 ignored** (the ignored test opens the native folder dialog; see the eleventh run) |
| `cargo build --workspace --release --locked --offline` | Pass (exit 0, 59.04 s on the 0.2.2 final tree, 51.43 s for the GUI-only increment before the version bump, 49.08 s on the previous tree). The first full size-profile build was 116.4 s. |

The workspace was renamed from `rust-tftpd`/`tftp-*` to TftpdR/`tftpd-r-*` on
2026-11-20. The original `0.2.0` verification ran on 2026-11-20 (offline, GNU toolchain)
with 147 tests. On 2026-11-21, a GUI log-model bound test was added (148 total),
then three redundant test functions were removed by consolidation without
losing their distinct assertions: the no-option RRQ exact-wire case is now part
of the all-packet roundtrip test; generic and legacy unknown TOML key rejection
share one config test; and the tautological comparison of two identical
`include_str!` sources was replaced by assertions on the parsed default template.
That left 145 test functions. The executable-icon work then added 3
`tftpd-r-build-support` tests. The GUI root-memory and activity-log changes added
25 GUI tests, and the portable state-directory fallback added 2 more. The
pre-warmed folder-picker work added 2 GUI tests plus one ignored Windows
integration test, so the current suite has **177 test functions** (3
build-support, 61 core unit, 29 RRQ integration, 9 daemon unit, 5 CLI
integration, 69 GUI, 1 doctest) and one ignored test. GUI coverage includes
`log_model_stays_bounded_and_keeps_newest_entries_first`, a deterministic
501-entry check for the 500-row newest-first cap,
`helper_replies_are_parsed_without_mangling_the_path` /
`a_non_ascii_directory_is_accepted` for the picker's UTF-8 protocol, and the
state-directory selection tests (`the_first_writable_state_directory_is_used`,
`a_blocked_state_directory_falls_back_to_the_portable_one`). The current
workspace-check results are shown in the table above.

The `0.2.0` release artifacts built on the verification machine measured
`tftpd-r.exe` ≈ 2.07 MiB (2,174,976 bytes) and `tftpd-r-gui.exe` ≈ 12.81 MiB
(13,430,272 bytes, thin LTO and symbols stripped). After the size-first release
profile (ADR 0010) the same tree produced `tftpd-r.exe` **1,245,184 bytes
(1.19 MiB)** and `tftpd-r-gui.exe` **8,734,208 bytes (8.33 MiB)** — 42.7 % and
35.0 % smaller. Embedding the Windows icon resource (ADR 0011) adds 112,640
bytes to each. The GUI root-memory/log/state work and the later save hint,
diagnostics and portable fallback bring the current measurements to
**1,357,824 bytes (1.29 MiB)** for the daemon and **8,980,480 bytes (8.56 MiB)**
for the GUI (the pre-warmed picker added 22,016 bytes to the GUI; the 2026-11-22
log-level drop-down and unified header added another 49,152 bytes). These figures
are informational only; they are not an acceptance criterion and not a promise
for other machines/toolchains.

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
| CI for the `0.2.0` release commit ([44719058814](https://github.com/greyskysoul/tftpd-r/actions/runs/44719058814)) | Pass: fmt, clippy `-D warnings`, `cargo test --workspace` and the release build on `windows-latest` (MSVC) and `ubuntu-latest` (GNU); GUI `cargo check` on Windows |
| `v0.2.0` release ([44719989555](https://github.com/greyskysoul/tftpd-r/actions/runs/44719989555)) | Pass: [release published](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.2.0) with four archives plus `SHA256SUMS`; GitHub's SHA-256 digest for every archive matches the attached checksum file |
| `v0.2.1` release ([44882456726](https://github.com/greyskysoul/tftpd-r/actions/runs/44882456726)) | Pass: the MSVC/Linux test gate, all four native builds and the [GitHub Release](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.2.1); the downloaded x64 MSVC archive matches `SHA256SUMS`, and its executables were run (see the seventh run below) |

The first `v0.2.0` tag run ([44718170093](https://github.com/greyskysoul/tftpd-r/actions/runs/44718170093))
failed before building anything: Linux clippy reported `DISCOVERY_TIMEOUT` and
`run_with_timeout` as dead code because `src/netif.rs` used them only in the
Windows discovery path. They are now `#[cfg(windows)]`; the fixed commit passed
the CI run above and the tag was moved to it. The failed run published no
release and no artifacts.

What CI proves, and what it does not:

- The suite passes with the **MSVC** toolchain (`x86_64-pc-windows-msvc`) and on
  Linux (GNU). The hand-verified build documented above is the GNU one; in
  addition, the published x86_64 MSVC `v0.2.1` executables were downloaded and
  exercised for `--version`, `--help`, `--smoke-test` and a 5,000-byte daemon
  transfer (seventh run below).
- `aarch64-pc-windows-msvc` and `aarch64-unknown-linux-gnu` are built on native
  arm64 runners and packaged, but no test is executed there and no arm64 binary
  has ever been run.
- Archive contents were inspected for a `v0.1.0` Windows and a Linux artifact:
  each holds `tftpd-r`, `tftpd-r-gui`, `config.example.toml`, `README.md`,
  `LICENSE-MIT` and `LICENSE-APACHE`.
- The published `v0.1.0` `SHA256SUMS` was verified against a downloaded asset
  (`tftpd-r-0.1.0-aarch64-pc-windows-msvc.zip`, SHA-256 starting `8ee455c6`); it
  matched. For `v0.2.0` the four archives were not downloaded again — large
  asset downloads stalled in the release session — but the SHA-256 digest
  reported by the GitHub API for each of them matches the attached
  `SHA256SUMS`, so the published bytes are the checksummed bytes. The `v0.2.0`
  archive contents and binaries were therefore not re-executed after download.

Archive and binary sizes in the `v0.1.0` release: Windows x64 zip 7.09 MiB,
Windows arm64 zip 6.76 MiB, Linux x64 tar.gz 9.19 MiB, Linux arm64 tar.gz
8.77 MiB. Uncompressed MSVC x64 binaries are `tftpd-r.exe` 2.28 MiB and
`tftpd-r-gui.exe` 13.36 MiB; Linux arm64 `tftpd-r` is 1.94 MiB and
`tftpd-r-gui` 15.76 MiB. They differ from the GNU numbers recorded above because
the toolchain and platform differ. The `v0.2.0` archives are Windows x64 zip
7.27 MiB, Windows arm64 zip 6.92 MiB, Linux x64 tar.gz 9.33 MiB and Linux arm64
tar.gz 8.90 MiB.

Test breakdown from `cargo test --workspace`:

| Target | Tests | What is covered |
| --- | --- | --- |
| `tftpd-r-core` unit tests (`src/lib.rs` target) | 61 | codec round-trips (including the exact no-option RRQ wire form) and malformed/oversized packets; option negotiation ranges, duplicates and clamping; configuration defaults/ranges/TOML errors, unknown and legacy keys, bind list emptiness/limit/multicast/broadcast rejection, wildcard collapsing and duplicate removal, shared-port summary; privileged-port bind classification and the non-forcing hint; path sandbox traversal, device names, legacy root-prefix mapping and root-directory rejection, symlink/reparse rejection, size limits; event ring drop/sequence/clear; error-code mapping |
| `tftpd-r-core` integration `tests/rrq.rs` | 29 | real loopback UDP server and client: small/exact-multiple/empty files, missing file, traversal, legacy `/file` and `\file` requests, root-prefixed traversal rejection, `blksize`/`tsize`/`timeout` negotiation, clamping, invalid option, duplicate ACK, retransmission, unknown TID ERROR 5, duplicate RRQ resend without a new session, WRQ rejection, netascii rejection, unknown mode/malformed/oversized datagrams, size limit, concurrent transfers, lifecycle events, shutdown cancellation, idempotent stop, retry exhaustion, **block-number wrap 65535 → 0** with `blksize = 8` (65,537 blocks); multiple listeners on one resolved port (IPv6 checked when available), wildcard collapsing to one listener, and an unbindable address aborting the whole start |
| `tftpd-r-daemon` unit tests | 9 | CLI parsing/defaults/overrides, parsed embedded example template safety/validity, config loading errors, unknown fields, override validation, status-channel failure path |
| `tftpd-r-daemon` integration `tests/cli.rs` | 5 | binary `print-default-config` equals `config.example.toml`; `check-config` success/missing file/uploads-rejected exit code 2; `serve` refuses invalid config with exit code 1; `serve` stays running with a valid config |
| `tftpd-r-gui-slint` unit tests | 69 | embedded controller start/stop, double-start rejection, invalid root, asynchronous restart (old server → new config), failed-restart error reporting and the no-stale-`Running`-after-stop guarantee; recent-root ordering, de-duplication, cap, persistence, canonicalisation, stale-directory cleanup and clearing the state file; one-time setup-flag persistence and read-only-profile tolerance; interface-address parsing/filtering/sorting; folder-selection validation and the warm picker's `PICK:` reply parsing/UTF-8 path handling; address picker model (two permanent entries, selected-but-undiscovered addresses preserved, duplicate removal, summaries) and the exclusive/sticky/fallback "全部网络" toggle rules; first-run all-network defaults; idle auto-restart with a real embedded server, deferral while a transfer is tracked, unchanged configuration and a deliberate stop; status-bar address summary and leaf-directory name; newest-first 500-row log-model bound under a 501-entry flood |
| `tftpd-r-core` doc test | 1 | crate-level usage example compiles |

The block-wrap test transfers 65,537 blocks in roughly 3–4 seconds on the
verification machine.

### Current automated-test gaps and improvement order

These are known limits of the recorded suite, not claims that a test failed:

1. **Clock control:** timeout tests use real timers (typically 1 s), not paused
   Tokio time. Keep them deterministic; consider a small clock abstraction or
   paused-time tests only where it reduces runtime/flakiness without weakening
   real UDP coverage.
2. **Packet-loss coverage:** packet loss is not injected with a proxy;
   retransmission is tested by withholding ACKs deterministically. A controlled
   loss/reorder harness is a future improvement, not currently implemented.
3. **Input-space exploration:** codec checks are table-driven and boundary
   focused; no fuzz/property-testing harness is recorded. Add one only after
   reviewing its toolchain, CI runtime, dependency/license impact, and seed
   corpus. Preserve deterministic regression cases for every discovered issue.
4. **GUI/event load:** the new regression test calls the log-model insertion path
   501 times and verifies its 500-row cap/order; it does not flood `EventBus`,
   measure renderer/UI-thread responsiveness, or exercise rendering/input end to
   end. Add a bounded event-load test only if it can assert a stable resource or
   latency contract; otherwise retain it as a target-machine acceptance check.
5. **Platform/manual boundaries:** real TFTP clients, GUI mouse/IME, native
   dialogs, DPI/font/accessibility, Linux desktop execution, LAN/firewall, and
   downloaded release-binary execution require the target matrix below. A
   `cargo check` or synthetic window message is not a substitute.

The 147-test result is the original 0.2.0 baseline. The suite briefly reached 148
after the GUI log-bound regression test, then was reduced to 145 by consolidating
or removing three redundant test functions while retaining their unique
assertions. This is not a requirement to add low-value tests merely to increase
the count. New tests should protect an invariant or close a named gap, and their
actual result must be recorded after execution.

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

### Release size optimization (2026-11-21, fifth run)

The release profile was changed to reduce the shipped binary size without
changing features, dependencies, UI code or protocol behaviour (ADR 0010).
Every candidate was a full `cargo build --workspace --release --offline` of the
same tree on the documented GNU x64 toolchain; sizes are file lengths of
`target/release/tftpd-r*.exe`:

| Profile | Daemon | GUI |
| --- | ---: | ---: |
| `thin` LTO, default optimisation, unwind (0.2.0) | 2,174,976 B (2.07 MiB) | 13,430,272 B (12.81 MiB) |
| `opt-level = "z"`, `lto = "fat"`, unwind | 1,429,504 B (1.36 MiB) | 9,242,112 B (8.81 MiB) |
| `opt-level = "z"`, `lto = "thin"`, abort | 1,443,328 B (1.38 MiB) | 9,827,840 B (9.37 MiB) |
| `opt-level = "s"`, `lto = "fat"`, abort | 1,351,680 B (1.29 MiB) | 9,483,776 B (9.04 MiB) |
| **`opt-level = "z"`, `lto = "fat"`, abort (chosen)** | **1,245,184 B (1.19 MiB)** | **8,734,208 B (8.33 MiB)** |

The chosen profile cuts the daemon by 42.7 % and the GUI by 35.0 %. Symbol
stripping was already present in `0.2.0`; the additional gain comes from fat LTO,
`opt-level = "z"` and `panic = "abort"`. `panic = "abort"` is an intentional
fail-fast trade-off: a panic in a Tokio task now terminates the process instead
of being contained as a `JoinError` (see ADR 0010). Tests and debug builds keep
the default unwind strategy.

Verification run with the chosen profile on 2026-11-21 (before the later test-case consolidation; 148 tests at that point):

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: exit 0, no
  warnings.
- `cargo test --workspace --offline`: **148 passed, 0 failed** (core unit 63,
  RRQ UDP integration 29, daemon unit 10, daemon CLI integration 5, GUI unit 40,
  doctest 1). Test targets use the dev/test profile, so they still unwind.
- `target/release/tftpd-r-gui.exe --smoke-test`: exit 0, empty stdout and
  stderr. It created the window, ran the UI timer and exited after 1.5 s, using
  an isolated `APPDATA`; this is the same smoke check as earlier runs, not a new
  interactive visual review.
- Release daemon end-to-end: a temporary config with `port = 0` and
  `bind_addresses = ["127.0.0.1"]` started `target/release/tftpd-r.exe serve`; a
  raw UDP client downloaded a 5,000-byte `image.bin` as `/image.bin`
  byte-for-byte; `CTRL_BREAK_EVENT` logged `TFTP server stopped` and the process
  exited with code 0.
- `target/release/tftpd-r.exe print-default-config` still equals
  `config.example.toml`.

Not re-verified in this run: MSVC/Linux/arm64 binary sizes (only the GNU x64
build was measured), rebuilt release archives, release-mode throughput or CPU
cost of `opt-level = "z"`, interactive GUI rendering after the profile change,
and the CI/release workflows (they have not been rerun since the profile change;
their next run builds from the updated `Cargo.toml`). Those remain open until a
target-machine/release run records them.

### Windows executable icon (2026-11-21, sixth run)

Both Windows binaries now embed `app-icon.ico` as an `RT_GROUP_ICON`/`RT_ICON`
PE resource; the implementation and trade-offs are in
[ADR 0011](adr/0011-windows-executable-icon.md). The resource is generated from
the committed ICO at build time by `tftpd-r-build-support`; no generated file is
committed.

- **Build.** `cargo build --workspace --release --locked --offline` exited 0 in
  50.08 s on the GNU x64 toolchain. The generated
  `target/*/build/*/out/tftpd-r-icon.res` is 112,368 bytes and its SHA-256
  (`29ccb912…d6a1`) is identical to the `.res` that `windres -O res` produces
  from `1 ICON "app-icon.ico"`: the Rust generator and binutils agree byte for
  byte.
- **PE resource.** A `ctypes` probe loaded both debug and release executables
  with `LoadLibraryExW(..., LOAD_LIBRARY_AS_DATAFILE)` and called
  `FindResourceW`/`SizeofResource`. Each executable has `RT_GROUP_ICON` id 1
  with seven frames, and every `RT_ICON` lookup returned exactly the byte count
  listed in the group: 1,128 / 2,440 / 4,264 / 9,640 / 16,936 / 67,624 / 9,944
  bytes for 16 / 24 / 32 / 48 / 64 / 128 / 256 px.
- **Extracted file icon.** `[System.Drawing.Icon]::ExtractAssociatedIcon`
  returned a 32x32 icon for both release binaries; the saved PNG shows the
  `dns` glyph on the `#263238` tile. This is the icon used for the file, not a
  capture of the running window.
- **Smoke tests.** Release `tftpd-r-gui.exe --smoke-test` exited 0 with empty
  stdout and stderr. Release `tftpd-r.exe` served a 5,000-byte `/image.bin`
  byte-for-byte and exited 0 after Ctrl-Break, logging `TFTP server stopped`.
- **Sizes.** `tftpd-r.exe` 1,357,824 B (1.29 MiB) and `tftpd-r-gui.exe`
  8,846,848 B (8.44 MiB); both grew by 112,640 B over the pre-icon build.
- **Automated gates.** `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets --offline -- -D warnings` and `cargo test --workspace --offline`
  all exited 0; the suite is **148 passed** (3 build-support, 61 core unit, 29
  RRQ integration, 9 daemon unit, 5 CLI integration, 40 GUI, 1 doctest).

Not verified in this run: the MSVC resource path (there is no MSVC toolchain or
Windows SDK on this machine, so `link.exe` consuming the generated `.res` was
not executed), aarch64, Explorer's icon cache and shortcut icons, and the
CI/release workflows (not rerun after the change). Only the GNU x64 binaries had
their PE resources inspected. The later `v0.2.1` release run below exercised the
MSVC path through CI and the published MSVC artifact; aarch64 and Explorer
remain open.

### v0.2.1 release artifacts (2026-11-21, seventh run)

The `v0.2.1` release workflow
([44882456726](https://github.com/greyskysoul/tftpd-r/actions/runs/44882456726))
passed the Windows MSVC and Linux GNU test gate, built all four native targets
and published the [GitHub Release](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.2.1).
This is the first release whose CI-built MSVC executables were downloaded and
run.

- **Checksum.** `tftpd-r-0.2.1-x86_64-pc-windows-msvc.zip` is 5,111,579 bytes;
  its SHA-256
  `32b4e58ff180fc448d1d4ca665a985c3c5be67da43c471d2e2e74b2ff05bcf2f`
  matches both `SHA256SUMS` and GitHub's asset digest.
- **Contents.** `tftpd-r.exe` 1,193,984 B, `tftpd-r-gui.exe` 8,513,024 B,
  `tftpd-r-gui.ico`, `config.example.toml`, `README.md`, `LICENSE-MIT` and
  `LICENSE-APACHE`.
- **PE resources.** Both MSVC executables contain `RT_GROUP_ICON` id 1 with the
  same seven frames as the GNU build; every `RT_ICON` byte count matches the
  ICO (1,128 / 2,440 / 4,264 / 9,640 / 16,936 / 67,624 / 9,944).
  `ExtractAssociatedIcon` returns a 32x32 icon for both. This verifies the MSVC
  `.res`/`CVTRES` path that could not be checked on the GNU-only machine.
- **Execution.** `tftpd-r.exe --version` reports `tftpd-r 0.2.1`;
  `tftpd-r-gui.exe --help` reports `tftpd-r-gui 0.2.1` and exits 0;
  `--smoke-test` exits 0 with empty stdout/stderr. The MSVC daemon downloaded a
  5,000-byte `/image.bin` byte-for-byte and exited 0 after Ctrl-Break, logging
  `TFTP server stopped`.
- **Not run.** The x86_64 Linux archive and both aarch64 archives were not
  downloaded or executed; their icon resources were not inspected. The
  interactive MSVC GUI, Explorer's icon cache and shortcut rendering, other
  DPIs, and real/legacy TFTP clients remain unverified.

### GUI root memory, activity log, picker visibility and portable state (2026-11-21, eighth–tenth runs)

This section condenses the verification rounds after the 0.2.1 release. The
automated commands above (`fmt`, `clippy`, `test`, release build) were executed
for each round; only the latest numbers are retained in the checks table.

- **Eighth run — unset root and remembered root (159 tests).** With an isolated
  `APPDATA`, a fresh profile opened the settings dialog with an empty root field
  and the status bar showed `未指定根目录`; no `tftp-root` appeared anywhere. A
  seeded `last-root.txt` was restored into the settings form and the log, and a
  start used that directory (`服务已就绪`). Starting with an empty root opened
  the dialog, showed `请先选择根目录。`, and did not create `last-root.txt`.
  Captures were `PrintWindow` images with synthetic window messages, not a real
  mouse. ADR 0012.
- **Ninth run — recent-root menu button and activity log (173 tests).** The
  root drop-down showed `选择…`; clicking it closed the menu and started the
  PowerShell folder dialog without adding a log row. From the menu, a directory
  chosen while stopped updated the configuration/status bar and logged that it
  would take effect on the next start. The `全部/INFO/WARN/ERROR` filters only
  hid rows (entries retained), the filtered-empty text was
  `当前筛选级别下没有日志。`, timestamps were local wall-clock, and a multi-line
  TOML error was collapsed to one line. The native save dialog for 导出 opened
  but was not completed. A synthetic three-click sequence did not reliably
  deliver the third click. ADR 0012 revision/ADR 0013.
- **Picker visibility follow-up.** A user reported that the folder picker no
  longer appeared. The helper's `FolderBrowserDialog` **was created** (a visible
  `#32770` window titled `浏览文件夹` existed) but could be placed behind the
  panel or the current foreground window. The PowerShell helper now shows both
  `FolderBrowserDialog` and the export `SaveFileDialog` with a 1×1, fully
  transparent, top-most owner form. The production `pick_folder_async` probe put
  `浏览文件夹` at the top of the visible Z-order and made it the foreground
  window; the rebuilt release GUI confirmed the same from 设置 → 浏览…. The
  already-running instance still used the old binary and had to be restarted.
- **Tenth run — save reminder, diagnostics and portable fallback (175 tests).**
  A manual test produced `拒绝访问 (os error 5)` when remembering the root. The
  profile ACL granted `siris` full control, and a non-sandboxed probe created
  `%APPDATA%\TftpdR\probe.txt` successfully. The GUI under test, however, ran
  from this session at **Low mandatory integrity level** (no Explorer process),
  so it could not create the medium-integrity profile directory. This is an
  environment restriction, not an ACL defect.
  - The settings form now shows an inline hint after a pick:
    `已选择目录，点击“保存设置”后生效。` (the activity log is behind the modal).
  - `LastRoot::record` returns `Result<bool, LastRootError>`; a failed write
    keeps the in-memory value and logs a WARN, a changed value logs
    `已记住根目录：…，下次启动将自动恢复。`, a stale entry logs
    `上次使用的根目录已不可用，已忽略：…。`, and `config.toml` precedence is
    stated in the log.
  - `state_file` selects the first directory that can actually be created and
    written: the per-user directory, then `.tftpd-r` beside the executable, then
    `.tftpd-r` in the working directory. The fallback is logged as
    `用户配置目录不可写；状态文件改用便携目录：…。`
  - Verification: release GUI with the real (unwritable) `APPDATA` and a
    prefilled `config.toml`; 设置 → 保存设置 wrote
    `target\release\.tftpd-r\last-root.txt` and logged the fallback plus
    `已记住根目录`; after deleting `config.toml`, the next launch logged
    `已恢复上次使用的根目录：…` and showed `share`. Screenshots:
    `fallback-1-saved.png`, `fallback-2-relaunch.png`. The test state was removed
    afterwards so it cannot affect the operator's next launch. The running
    manual-test instance (PID 19324) was still on the old binary and had to be
    renamed out of the way before the rebuild; it must be closed and restarted
    to load the fix.
  - Not verified: the inline hint layout was compiled but not captured; the
    native picker selection was reported by the user but not driven
    end-to-end here; Unix and a read-only executable/working directory were not
    tested; portable state is not migrated when the per-user directory becomes
    writable again.

### GUI pre-warmed folder picker (2026-11-21, eleventh run)

A user reported that the folder picker appeared about 1–2 s after pressing
浏览…/选择…. The delay was the per-click PowerShell + WinForms startup, not the
dialog. Measurements on this machine (Windows PowerShell 5.1, `x86_64-pc-windows-gnu`):

| Step | Measured |
| --- | --- |
| `powershell.exe -NoLogo -NoProfile -Command exit` | 1.10–1.14 s |
| One-shot helper: `Add-Type System.Windows.Forms` + `System.Drawing` + construct dialog/owner | 1.61–1.64 s |
| Warm helper: `READY` after process start (background, not in the click path) | 1.53 s |
| Warm helper: request → visible `#32770` dialog (first dialog in the process) | 368 ms |
| Warm helper: request → visible dialog (second dialog, reused process) | 62 ms |
| Warm helper memory (WinForms loaded, before a dialog) | ~73 MB working set / ~49 MB private |

Verification performed:

- `cargo test -p tftpd-r-gui-slint --offline -- --ignored` ran
  `the_warm_helper_closes_its_dialog_on_shutdown`: pre-warm, open the real
  dialog, `shutdown()` closes it and reports a cancelled pick (1 passed, 4.02 s).
- A PowerShell probe using the exact persistent script confirmed `READY`, a
  visible `#32770` window after one request, `PICK:` after closing it with
  `WM_CLOSE`, a second request on the same process (62 ms to the dialog) and a
  clean exit (code 0) when stdin was closed.
- A raw-byte probe (bypassing the harness's OEM-encoded `StandardInput`)
  confirmed UTF-8 round-trip: `PICK:C:\srv\固件 共享\` came back unchanged. The
  old one-shot helper was also checked: `[Console]::Out` emitted the Chinese
  path as GBK bytes (`D6 D0 CE C4 …`), which the Rust side decoded as UTF-8 and
  silently rejected; both helpers now write via a UTF-8 `StreamWriter`.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --offline
  -- -D warnings`, `cargo test --workspace --offline` (177 passed, 1 ignored) and
  `cargo build --workspace --release --locked --offline` (49.08 s) passed. The
  GUI grew from 8,909,312 to 8,931,328 bytes.

Not verified here: a real click selecting a directory and confirming the native
dialog inside the Slint panel (the agent session cannot drive the GUI; the
`--smoke-test` process did not exit under this session and was killed), dialog
z-order on a real desktop after the helper change, and non-Chinese/non-GBK
Windows installations. The ignored test covers the Rust worker and the real
dialog lifecycle, not the Slint click wiring. ADR 0014.

### GUI log-level drop-down and unified header (2026-11-22, twelfth run)

Two UI-polish requests were implemented on top of the activity log and header:

- **Level filter as a drop-down.** The four exact-level buttons (`全部`, `INFO`,
  `WARN`, `ERROR`) became one trigger (`全部级别` / `仅 INFO` / `仅 WARN` /
  `仅 ERROR`) with a chevron and a four-row list. The list is an in-panel overlay
  anchored to the trigger, draws above the log rows and is clipped by the panel.
  The Slint keys (`ALL`/`INFO`/`WARN`/`ERROR`), `LogFilter`, the Rust callback and
  the export are unchanged. The chevron was initially top-aligned inside the
  30 px trigger; the layout now sets `cross-axis-alignment: center`.
- **No title-bar/toolbar hairline.** The 1 px `Theme.toolbar-divider` line at the
  bottom of `titlebar.slint` was removed, so the two rows read as one flat
  `#37474f` header block; the vertical dividers inside each row remain.

Automated gates (final tree): `cargo fmt --all -- --check` exit 0;
`cargo clippy --workspace --all-targets --offline -- -D warnings` exit 0;
`cargo test --workspace --offline` **177 passed, 1 ignored**;
`cargo build --workspace --release --locked --offline` exit 0 in 59.04 s,
`tftpd-r.exe` 1,357,824 B and `tftpd-r-gui.exe` 8,980,480 B.

Visual and interaction checks (`PrintWindow`, Windows PowerShell 5.1):

- `TFTPD_R_GUI_PREVIEW=log-filter` (a development-only opener added for this
  check) at 1120x780 and at the 900x600 minimum: the list opens just below the
  trigger, right-aligned with it, marks the active level with a check, stays
  inside the panel and is clamped/scrolled rather than overflowing.
- Synthetic `WM_MOUSEMOVE`/`WM_LBUTTONDOWN`/`WM_LBUTTONUP` sequence on a debug
  build: open the list; select `仅 WARN` (the INFO startup row disappears and
  `当前筛选级别下没有日志。` is shown); reopen (the check is on `仅 WARN`); click
  the log area (list closes, filter unchanged — that capture is byte-identical to
  the post-selection one); reopen and select `全部级别` (the hidden row returns).
- Header capture at 1120x780 shows one continuous dark band with no line between
  the caption row and the toolbar.

Not verified here: a real mouse, keyboard navigation/accessibility for the custom
drop-down (it has no accessible role, like the previous custom buttons), DPI
scaling other than the 100 % forced for captures, Linux/macOS rendering, and an
interactively driven release build (the click test used the debug binary).
ADR 0013 update.

## Not verified (and therefore not claimed)
- **External TFTP client interoperability** (tftp-hpa, Tftpd64, PXE firmware,
  U-Boot, etc.). Only the in-repo test client and the raw Python client were
  used.
- **MSVC runtime coverage.** The x86_64 MSVC release binaries were run for
  `--version`, `--help`, `--smoke-test` and one daemon transfer in the 0.2.1
  check, but the interactive GUI, long-running service behaviour, LAN operation
  and the arm64 MSVC binaries were not run.
- **Linux/macOS runtime.** CI runs the full suite on Linux, but the GUI has never
  been run on a Linux desktop, and macOS is untested.
- **LAN/deployment behaviour:** binding `0.0.0.0`, Windows Defender Firewall
  rules for TID ports, RDP sessions, concurrent real-world clients. Port 69 was
  actually bound by the daemon on this Windows machine (Windows normally allows
  low ports), so the "permission denied" hint was exercised only by unit tests,
  not by a real failed bind; the Linux root/`CAP_NET_BIND_SERVICE` path is
  untested.
- **GUI helper features:** the pre-warmed folder helper was exercised end to end
  by the ignored Rust test (start, open a real dialog, close it on shutdown) and
  its protocol was probed from PowerShell (READY, `PICK:`, reuse, UTF-8), but a
  real click on 浏览…/选择… followed by selecting a directory and confirming the
  dialog inside the Slint panel was not driven here (the agent session cannot run
  the GUI window). Address discovery was exercised only through the .NET API on
  this machine (not on Linux, not with VPN adapters appearing/disappearing, and
  not through the `hostname -I` fallback). The save/remember/restore path was
  verified by saving a prefilled settings form and relaunching, including the
  portable `.tftpd-r` fallback with a real unwritable `APPDATA`; the inline "not
  saved yet" hint was compiled but not visually captured. The native save dialog
  for log export opened but was not completed, so a file produced through the GUI
  remains unverified; its formatting and write path are unit-tested.
- **Release artifacts:** sizes and checksums are recorded for the GNU x64 local
  build and for the downloaded x64 MSVC `v0.2.1` archive; they are not a
  guarantee for other machines or toolchains. The x86_64 Linux archive and both
  aarch64 archives were not downloaded or run, and their icon resources were not
  inspected. No release-mode throughput benchmark was run, so the CPU cost of
  `opt-level = "z"` is unquantified.
- **MSVC/ARM64 executable icon.** The GNU x64 PE resources were verified (see
  the sixth run above); the MSVC path (passing the generated `.res` to
  `link.exe`) and the aarch64 resource were not executed or inspected here.
  Explorer's icon cache, shortcut icons and icon selection at non-default DPI
  were not checked visually on a target desktop.
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
   values are used. When port 69 is denied (for example, Linux without root/CAP_NET_BIND_SERVICE or Windows with a reserved port), confirm that the
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

## Forward test gates for the reordered phases

The following are planned acceptance tests; they have **not** been run unless a
result is later added to the evidence sections above.

### Phase 6 — Current-product validation

- Windows MSVC: full workspace fmt/clippy/test/release-build gate; record runner
  or target OS/toolchain and the exact artifact tested.
- Linux desktop: launch and interact with the Slint GUI in a real graphical
  session; separately record daemon/core test results. Headless CI compilation
  does not establish desktop usability.
- Independent TFTP client: record implementation/version, OS, options, transfer
  size and SHA-256; include legacy `get /file`, exact-block-size transfer,
  missing file and traversal rejection.
- LAN deployment: record client/server interfaces, firewall rules for the
  listener and TID traffic, and outcomes. Do not expose the test server to the
  public Internet.
- GUI hands-on: ordinary typing and IME composition, native directory picker,
  real mouse title-bar/resize controls, first-run/restart/close paths, Chinese
  rendering and a non-default DPI; test RDP only if it is in the declared support
  matrix.
- Release readiness: download each target archive, check its published digest,
  inspect contents and dependencies, execute only on available target hardware,
  and record unavailable targets as unverified. Complete dependency/license
  review and installation/upgrade/rollback documentation.

### Phase 7 — IPC

- Protocol tests cover exact framing limits, truncation, malformed DTOs, version
  negotiation, request IDs, response/event bounds and unsupported commands.
- Native transport tests cover ACL/peer rejection, remote access denial, stale
  endpoints, connection/rate/subscription caps, disconnects, reconnects and
  shutdown cleanup. Run platform-specific security tests on native CI runners.
- Controller contract tests cover status/start/stop, concurrent or duplicate
  commands, daemon restart, protocol mismatch, event resubscription and the GUI
  remaining responsive. Verify IPC cannot change the root or enable uploads.

### Phase 8 — WRQ uploads

- Only after ADR/security approval: test WRQ negotiation and state transitions,
  duplicate/reordered DATA, retries, TID validation, final blocks, wraparound,
  limits, cancellation and sanitized errors.
- Inject write, flush, disk-full, rename/commit and shutdown failures; verify temp
  cleanup and preservation of any pre-existing destination.
- Test traversal/symlink/reparse/overwrite policy and declared atomicity on each
  supported OS, then interoperate with an independent client and verify final
  file hashes. Retest that uploads remain rejected by default.

Every test record should include date, environment, command or manual steps,
expected/observed result, artifact or client identity, and a clear **pass / fail /
not run / blocked** status. Do not convert an unrun checklist into a pass claim.
