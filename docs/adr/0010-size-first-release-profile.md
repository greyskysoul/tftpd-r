# ADR 0010: Size-first release profile

- **Status:** accepted
- **Date:** 2026-11-21
- **Security review:** the panic-strategy consequence is recorded below and in
  [`security.md`](../security.md)

## Context

The `0.2.0` GNU x64 release binaries measured about **2.07 MiB** for the daemon
and **12.81 MiB** for the GUI. Most of the GUI's code is fixed by Slint's winit
backend and software renderer: the system-font stack (parley/fontique and the
ICU data it embeds), the image/SVG decoders that the backend enables
unconditionally, clipboard/menu/URL support, and Windows bindings. The release
profile only used thin LTO and symbol stripping, so there was room to cut
code-generation overhead without changing features, dependencies, the UI, or the
protocol.

The panel does not load images or SVG at runtime: its only artwork is a raw RGBA
buffer compiled in and uploaded through `Window.icon`. Its own configuration and
log text is generated at runtime. No runtime requirement depends on the unused
backend features, but they cannot be disabled from this workspace because they
are hard-wired in `i-slint-backend-winit`; removing them would require an
upstream feature change or a fork.

## Decision

Use a size-first `[profile.release]` for both shipped binaries:

```toml
[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

Measured on the verification machine (Windows 11, Rust 1.100.0 GNU x64, MinGW
GCC 15.3) with the same source tree:

| Profile | Daemon | GUI |
| --- | ---: | ---: |
| thin LTO, default optimisation, unwind (0.2.0) | 2,174,976 B (2.07 MiB) | 13,430,272 B (12.81 MiB) |
| `z` + fat LTO + unwind | 1,429,504 B (1.36 MiB) | 9,242,112 B (8.81 MiB) |
| `z` + thin LTO + abort | 1,443,328 B (1.38 MiB) | 9,827,840 B (9.37 MiB) |
| `s` + fat LTO + abort | 1,351,680 B (1.29 MiB) | 9,483,776 B (9.04 MiB) |
| **`z` + fat LTO + abort (chosen)** | **1,245,184 B (1.19 MiB)** | **8,734,208 B (8.33 MiB)** |

The chosen profile reduces the daemon by **42.7 %** and the GUI by **35.0 %**
relative to the 0.2.0 binaries. `strip = "symbols"` removes the COFF symbol
table and debug information; it does not change behaviour.

`panic = "abort"` is a deliberate **fail-fast** choice. With the previous
`unwind` setting, a panic in a Tokio task was contained by the runtime as a
`JoinError` and the process kept serving. Now a panic terminates the process.
The server is a read-only RRQ implementation, and panics on untrusted input are
treated as bugs to be fixed, not as a recoverable state; aborting avoids
continuing with possibly inconsistent state. The fallback for a supervised
deployment (future Windows Service or an external supervisor) is
restart-on-failure, not silently swallowing the panic.

## Consequences

- Both binaries are smaller; the exact numbers differ by toolchain, target and
  optimiser version, so MSVC, Linux and arm64 sizes must be measured after a
  release build instead of being inferred from this table.
- Release builds take longer: the first full build of this profile took 116.4 s
  on the verification machine, compared with 42.46 s recorded for the previous
  profile. Fat LTO also makes rebuilds less incremental. CI/release build time
  limits should be watched; correctness and test gates are unchanged.
- Tests, clippy and debug builds keep the default `unwind` strategy because
  they use the dev/test profiles. Only the shipped release binaries abort.
- `opt-level = "z"` trades instruction-level performance for size. For TFTP the
  hot path is dominated by file I/O and UDP round trips; the release daemon
  downloaded a 5,000-byte file byte-for-byte and shut down gracefully in the
  end-to-end check. No throughput benchmark was run, so no performance claim is
  made.
- A panic no longer leaves a partially working daemon behind. That is the
  intended failure mode, but it must be reconsidered before adding a Windows
  Service or any supervisor that assumes the process never exits on its own.
- Further reductions would require disabling Slint backend features (image/SVG
  decoders, clipboard, native menus), replacing the `ScrollView` std-widget in
  the UI, or a nightly `build-std`/`panic_immediate_abort` setup. Those change
  dependencies or the UI and are not part of this decision; UPX or similar
  packers are not used.
- Debug/test profiles are unchanged, so assertions still unwind in tests. The
  verification gate is `cargo test --workspace` (dev profile), not
  `cargo test --release`.

## Post-decision note: executable icon (2026-11-21)

[ADR 0011](0011-windows-executable-icon.md) embeds the product icon as a Windows
PE resource in both binaries. On the GNU x64 build the resource adds 112,640
bytes: `tftpd-r.exe` 1,245,184 → **1,357,824 B (1.29 MiB)** and
`tftpd-r-gui.exe` 8,734,208 → **8,846,848 B (8.44 MiB)**. The profile comparison
and decisions above are unaffected; the current absolute baseline is recorded in
[testing.md](../testing.md).
