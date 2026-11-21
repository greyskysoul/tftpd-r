# ADR 0011: Windows executable icon as a build-time resource

- **Status:** accepted
- **Date:** 2026-11-21
- **Verification:** GNU x64 only; the MSVC and aarch64 resource paths are not
  executed on this machine (see [testing.md](../testing.md))

## Context

The product artwork already existed as `app-icon.svg` and its exports under
`crates/tftpd-r-gui-slint/ui/icons/`, including a seven-frame
`app-icon.ico` (16, 24, 32, 48, 64, 128, 256 px). The running panel used
`app-icon-128.rgba` through `Window.icon`, but neither `tftpd-r.exe` nor
`tftpd-r-gui.exe` carried a PE resource, so Explorer and shortcuts showed the
generic executable icon.

A Windows executable icon must be an `RT_GROUP_ICON`/`RT_ICON` resource in the
PE image. Writing it needs either a resource compiler (`rc.exe`, `windres`) or a
`.res` file that the linker converts. The workspace avoids unnecessary
dependencies and generated files in version control, and the GNU build already
requires MinGW-w64 binutils (which provides `windres`). The MSVC toolchain has
`link.exe` but its `rc.exe` lives in the Windows SDK/Visual Studio tool layout,
which a build script would otherwise have to discover.

## Decision

Generate the resource at build time from the committed ICO and link it into both
binaries:

- A new build-time-only workspace crate, `tftpd-r-build-support`, parses the ICO
  and writes a `.res` file to `OUT_DIR`. `build.rs` in both binary crates calls
  `embed_windows_icon`, which emits `rerun-if-changed` for the asset and
  `rustc-link-arg-bins` for the generated file. The helper is a no-op when the
  target OS is not Windows.
- **MSVC targets:** the `.res` file is passed to `link.exe` directly. `LINK`
  documents `.res` as an input and runs `CVTRES`; no `rc.exe` discovery is
  needed.
- **GNU targets:** `windres` (from `PATH`, or the `WINDRES` environment variable)
  converts the same `.res` to a COFF object with a `.rsrc` section. `-F` is
  passed for x86/x86_64 so an x86 build on an x86_64 toolchain gets the right
  machine type; native arm64 uses the toolchain default.
- No `.res`, `.o` or other generated binary is committed. Replacing
  `app-icon.ico` is enough to regenerate the resource.

The generator reproduces, byte for byte, the `.res` that `windres -O res` writes
for `1 ICON "app-icon.ico"` (112,368 bytes, binutils 2.46.1). This is checked in
the session and the generator/parser/res layout is covered by unit tests in
`tftpd-r-build-support`.

## Alternatives considered

- **Commit a prebuilt `.res` (and a COFF object for GNU):** rejected. It adds a
  generated binary to git, the COFF object is architecture-specific, and it can
  silently go stale when the ICO changes. The build-time generator avoids all
  three.
- **Add `winresource`/`embed-resource` as a dependency:** viable in principle,
  but it adds a build dependency and its transitive crates for a small, stable
  format. This environment could not download new crates, and the workspace
  policy prefers not adding dependencies without a clear need. The internal
  helper is small and tested; if maintenance cost grows, replacing it with a
  maintained crate is a contained change.
- **Compile a `.rc` with `rc.exe`:** rejected for MSVC because the build script
  would have to discover the Windows SDK/Visual Studio installation. Linking the
  generated `.res` uses the linker rustc already invokes. GNU still uses
  `windres`, which is already part of the required MinGW-w64 toolchain.
- **Only keep `Window.icon`:** rejected. It affects the running window and task
  bar, not the file icon in Explorer, shortcuts or the archive contents.

## Consequences

- Both Windows executables now contain the seven-frame icon. On the GNU x64
  verification build the resource adds **112,640 bytes** to each file:
  `tftpd-r.exe` 1,245,184 → **1,357,824 B (1.29 MiB)**, `tftpd-r-gui.exe`
  8,734,208 → **8,846,848 B (8.44 MiB)**. The ADR 0010 profile decision and its
  relative size results are unchanged; the absolute baseline moved and is
  recorded in [testing.md](../testing.md).
- GNU Windows builds now require `windres`; MSVC builds do not require `rc.exe`.
  The generated `.res` path is passed as a plain linker input, so MSVC support
  depends on documented `LINK` behaviour rather than on this machine's
  execution.
- The runtime has no new dependency or code path. The panel's window/task-bar
  icon still comes from `app-icon-128.rgba`; the PE resource is the file icon.
- Linux builds are unaffected: the helper returns before reading the icon, and
  the build support crate is never linked into a shipped binary.
- The unit test pins the current seven-frame dimensions and byte sizes; replacing
  the artwork requires updating those assertions as well as regenerating the
  PNG/ICO/RGBA files.
- Not verified here: the MSVC and aarch64 resource paths, Linux (no icon is
  expected), Explorer icon caching, and icon selection at every Windows DPI.
  The release-readiness phase still needs a target-machine check for the MSVC
  and arm64 artifacts.
