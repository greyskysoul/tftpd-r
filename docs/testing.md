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
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass (no warnings) |
| `cargo test --workspace` | **97 passed, 0 failed** |
| `cargo build --workspace --release` | Pass (~1 min 36 s including Slint) |

Release artifacts on the verification machine (informational only; not an
acceptance criterion and not a promise for other machines/toolchains):
`target/release/tftp-daemon.exe` ≈ 2.03 MiB, `target/release/tftp-gui-slint.exe`
≈ 12.5 MiB (debug symbols stripped by the release profile).

Test breakdown from `cargo test --workspace`:

| Target | Tests | What is covered |
| --- | --- | --- |
| `tftp-core` unit tests (`src/lib.rs` target) | 53 | codec round-trips and malformed/oversized packets; option negotiation ranges, duplicates and clamping; configuration defaults/ranges/TOML errors; path sandbox traversal, device names, legacy root-prefix mapping and root-directory rejection, symlink/reparse rejection, size limits; event ring drop/sequence/clear; error-code mapping |
| `tftp-core` integration `tests/rrq.rs` | 26 | real loopback UDP server and client: small/exact-multiple/empty files, missing file, traversal, legacy `/file` and `\file` requests, root-prefixed traversal rejection, `blksize`/`tsize`/`timeout` negotiation, clamping, invalid option, duplicate ACK, retransmission, unknown TID ERROR 5, duplicate RRQ resend without a new session, WRQ rejection, netascii rejection, unknown mode/malformed/oversized datagrams, size limit, concurrent transfers, lifecycle events, shutdown cancellation, idempotent stop, retry exhaustion, **block-number wrap 65535 → 0** with `blksize = 8` (65,537 blocks) |
| `tftp-daemon` unit tests | 10 | CLI parsing/defaults/overrides, embedded example template validity, config loading errors, unknown fields, override validation, status-channel failure path |
| `tftp-daemon` integration `tests/cli.rs` | 5 | binary `print-default-config` equals `config.example.toml`; `check-config` success/missing file/uploads-rejected exit code 2; `serve` refuses invalid config with exit code 1; `serve` stays running with a valid config |
| `tftp-gui-slint` controller tests | 2 | embedded `ServerHandle` start/stop within a Tokio runtime, double-start rejection, invalid root does not start |
| `tftp-core` doc test | 1 | crate-level usage example compiles |

The block-wrap test transfers 65,537 blocks in roughly 3–4 seconds on the
verification machine.

### Deliberate limitations of the automated tests

- Timeouts use real timers (typically 1 s), not Tokio time pausing, so tests are
  slower than ideal and could be flaky on extremely loaded machines.
- Packet loss is not injected with a proxy; retransmission behaviour is tested by
  withholding ACKs deterministically.
- The GUI tests cover the controller, not window rendering or user interaction.

## Manual end-to-end verification performed

1. **Daemon + real UDP client.**
   - Built `target/debug/tftp-daemon.exe`.
   - Started `tftp-daemon serve --config <temp config>` with
     `bind_address = "127.0.0.1:0"` and a 5,000-byte file `image.bin`.
   - A raw UDP TFTP client (Python, 512-byte `octet` RRQ, sequential ACKs)
     downloaded the file **as `/image.bin`**, i.e. with the legacy root prefix
     that the user's old client sends, and the bytes matched exactly.
   - Sent `CTRL_BREAK_EVENT` to the daemon process group: it logged
     `shutdown requested signal=Ctrl-Break` and `TFTP server stopped`, and exited
     with code 0.
   - This exercises CLI parsing, config loading, socket binding, RRQ, events,
     logging, signal handling and graceful shutdown together.
2. **GUI smoke run.**
   - `target/debug/tftp-gui-slint.exe --smoke-test` creates the Slint window,
     runs the 200 ms UI timer, and quits after 1.5 s.
   - Observed: process exited with code 0 after ~3.2 s, no stderr output or panic.
     This verifies the winit/software-renderer backend initialises and the
     generated UI compiles and instantiates on this machine.
   - It is **not** a substitute for interactive acceptance: visually checking
     layout, Chinese font rendering, button behaviour and window-close cleanup
     on a target desktop remains to be done.

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
- **Windows Service, IPC, uploads** — not implemented in this release.
- **Fuzz/property testing** of the codec; tests are table-driven and
  boundary-focused instead.

## Manual acceptance checklist for a target machine

1. `cargo test --workspace` and `cargo build --workspace --release` on the target
   toolchain (MSVC and, if used, GNU).
2. Launch the GUI with a real `config.toml`; check the dashboard shows the
   configured bind address and root.
3. Start the server, download a file with a real TFTP client, confirm the file
   hash matches and the transfer appears/disappears in the Active transfers list.
4. Stop the server during a large transfer; confirm the client sees an error and
   the UI reaches `Stopped` without freezing.
5. Try `../`, a root-prefixed traversal (`/../file`), a drive path, a symlink
   inside the root and an oversized file; confirm the client receives an error
   and the server log shows a rejection. Also request `/file.bin` with the old
   client and confirm it is served from the configured root.
6. Flood events (large file, small `blksize`) and confirm the UI stays
   responsive and log/transfer lists stay bounded.
7. Close the window during an active transfer; confirm the process exits and no
   server task keeps running.
8. Check Chinese text rendering and font fallback on the target (the UI uses
   system fonts; no font is bundled).
9. Verify firewall behaviour for the listening port and ephemeral TID ports.

## Reproducing the checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release

# GUI smoke test (opens a window for ~1.5 s)
cargo run -p tftp-gui-slint -- --smoke-test
```
