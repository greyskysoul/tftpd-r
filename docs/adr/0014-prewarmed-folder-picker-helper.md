# ADR 0014: Pre-warmed folder-picker helper

- **Status:** accepted
- **Date:** 2026-11-21

## Context

User feedback on the `0.2.1` panel: pressing 浏览… in the settings dialog (or
选择… in the recent-root menu) took roughly 1–2 s before the native folder
dialog appeared, which made the button feel unresponsive.

The panel deliberately has no file-dialog dependency and no hand-written FFI
([ADR 0004](0004-embedded-only.md)); the dialog is shown by a helper process
running Windows PowerShell 5.1 + WinForms. That helper was started **per click**,
so the click path paid for the whole PowerShell/.NET startup:

- `powershell.exe -NoLogo -NoProfile -Command exit` on the verification machine:
  about **1.1 s**;
- plus `Add-Type -AssemblyName System.Windows.Forms` /
  `System.Drawing` and dialog construction: about **1.6 s** in total.

The dialog itself then added another ~0.2 s before it became visible. Reusing a
warm helper was measured at **~0.37 s** from the request to a visible dialog on
the first click, and **~0.06–0.1 s** on later clicks in the same process.

A second, independent defect surfaced during the measurement: PowerShell writes
`[Console]::Out` in the console/OEM code page (for example GBK on a Chinese
Windows). The Rust side decoded stdout as UTF-8, so a directory whose name
contains non-ASCII characters arrived as replacement characters and was silently
rejected as "not a directory". Picking `D:\固件\共享` therefore did nothing on a
zh-CN system.

## Decision

### One warm helper per panel session

- `src/folder_picker.rs` gains a `FolderPicker` service: a background worker
  thread that owns an optional, persistent Windows helper process. On Unix the
  worker keeps the existing one-shot `zenity`/`kdialog` behaviour.
- The helper is a single `powershell.exe -NoLogo -NoProfile -STA -Command …`
  process. It loads WinForms once, writes `READY`, then handles one dialog per
  request line:
  - the initial directory is one UTF-8 line on stdin (an empty line means "no
    initial directory");
  - the result is one UTF-8 line on stdout, `PICK:<path>` for a selection and
    `PICK:` for cancel.
  A Windows path cannot contain a line break, so the line protocol is
  unambiguous. A UTF-8 `StreamReader`/`StreamWriter` over the standard
  handles avoids the OEM code-page problem in both the warm helper and the
  one-shot fallback.
- The UI already allowed only one pending folder dialog. The worker serialises
  requests, so a second click cannot interleave with the open dialog.
- The helper is started by `FolderPicker::prewarm()` the first time the settings
  dialog or the recent-root menu is opened (checked from the existing 200 ms UI
  timer). It is **not** started at panel launch: users who never open these
  overlays pay neither process memory nor CPU.

### Failure handling, lifecycle and shutdown

- If the helper cannot be started or dies, the worker marks it failed and uses
  the original one-shot helper for every later request in that session. The
  feature keeps working, just at the old latency; a request that was interrupted
  reports a cancelled dialog.
- The helper is killed when the panel shuts down. Closing stdin alone would
  leave an open dialog alive, so `FolderPicker::shutdown()` also kills the child
  through a shared handle before the event loop's normal cleanup. Closing stdin
  still lets an idle helper exit cleanly (exit code 0).
- Commands remain fixed and absolute, no shell is involved, and the initial
  directory travels over stdin instead of the `TFTPD_R_INITIAL_DIR` environment
  variable; it can never become script text. The one-shot fallback keeps the
  environment-variable path for compatibility.

### Feedback while warming up

The settings form shows `正在打开文件夹选择器…` immediately on a click and
restores the previous inline hint if the dialog is cancelled. Once a directory
is chosen, the existing `已选择目录，点击“保存设置”后生效。` reminder replaces
it. No log entry is written for opening or cancelling a picker
([ADR 0013](0013-gui-activity-log.md)).

## Alternatives considered

- **Pre-warm at panel launch.** This would make even the very first click fast
  (~0.2 s), but keeps a hidden PowerShell + WinForms process alive for the whole
  session: about **73 MB working set / 49 MB private** on the verification
  machine, even when the operator never browses. Rejected as the default for a
  lightweight server panel; it is a one-line change to the pre-warm trigger if a
  deployment prefers guaranteed first-click latency.
- **`cscript`/VBScript with `Shell.Application.BrowseForFolder`.** One-shot
  startup is ~0.1–0.2 s with no idle process, but the API sets the dialog
  **root**, not just the initial selection, so the current-directory preselection
  is lost; it also needs a generated `.vbs` file and depends on Windows Script
  Host, which Microsoft is phasing out. Rejected.
- **A native dialog through `rfd`/`IFileOpenDialog` or hand-written FFI.** A
  modern dialog in ~0.1 s, but it adds a GUI dependency or `unsafe` code and
  crosses the project's no-toolkit/no-FFI boundary; on Linux it would also add
  GTK/portal requirements. Rejected without a separate ADR and dependency
  review.
- **Only add "opening…" feedback.** Cheap, but does not remove the 1–2 s wait.
  Kept as a complement, not as the fix.

## Consequences

- A warm folder dialog appears in ~0.37 s on the first click and ~0.06–0.1 s on
  later clicks on the verification machine, instead of ~1.6 s + dialog.
- A panel that never opens the settings dialog or the recent-root menu starts no
  helper process. Once a picker has been opened, the helper stays alive for the
  rest of the session; its memory cost is bounded and documented here, not
  configurable.
- Non-ASCII directory names now survive the helper round trip (UTF-8), fixing a
  latent silent failure on non-English Windows installations.
- The helper lifecycle is more complex than one-shot spawning: a shared child
  handle, a `READY` handshake with a 10 s timeout, one-shot fallback, and
  shutdown while a dialog is open. `the_warm_helper_closes_its_dialog_on_shutdown`
  (an ignored test, run manually on a desktop session) covers the start/open/
  shutdown path; parsing and UTF-8 handling are unit-tested.
- The real selection still needs target-machine acceptance: the harness cannot
  click a folder and press 确定 inside the native dialog. This is recorded as a
  remaining manual check in [docs/testing.md](../testing.md).
