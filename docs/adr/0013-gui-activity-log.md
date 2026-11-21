# ADR 0013: GUI activity log — wall-clock times, level filter and explicit export

- **Status:** accepted
- **Date:** 2026-11-21

## Context

The panel's activity log kept 500 rows but had grown unevenly:

1. The time column was `HH:MM:SS` elapsed since the panel started (`00:00:00`),
   which cannot be correlated with external events such as a client failure.
2. There was no way to hide routine INFO lines or to find errors in a long log.
3. There was no way to preserve the log for a bug report: closing the panel
   discarded it.
4. Internal UI actions such as opening the native directory picker wrote
   `已打开系统目录选择器…` into the log, even though the operator had not caused
   a server or transfer event.
5. Configuration/OS errors can contain embedded newlines; such a message
   overflowed the fixed 26 px row and could displace the time/level columns.

The same feedback asked for a 选择… button in the recent-root menu (handled by an
amendment to [ADR 0012](0012-gui-root-selection-and-memory.md)) and for a more
coherent log experience.

## Decision

### Log model

- `src/logging.rs` owns the bounded log: `VecDeque<LogEntry>` with at most
  `MAX_LOG_ROWS = 500` entries, newest first. `LogEntry` carries a local
  wall-clock timestamp (`DateTime<FixedOffset>`), a three-value
  `LogLevel` (`Info`/`Warn`/`Error`) and the message.
- Entries are stamped with `chrono::Local::now()` and displayed as `HH:MM:SS`.
  `chrono` was already part of the build through Slint; it becomes a direct GUI
  dependency so the panel does not format times by hand or invent a UTC offset.
  It is MIT/Apache-2.0 and adds no new crate to `Cargo.lock`.
- `push_log` collapses every whitespace run, including embedded `\n`/`\r`/`\t`,
  into one space. One entry therefore always occupies one line, in the panel and
  in the text export; this also prevents crafted multi-line log messages.
- Messages use the level enum instead of string literals at call sites.

### Filtering

- The log header gains `全部` / `INFO` / `WARN` / `ERROR` buttons. The filter is
  **exact-level**: `WARN` shows warnings only, not errors.
- The filter is display-only. The retained entries are never dropped, the export
  always contains the complete retained log, and selecting 全部 restores
  everything. `清空` still clears all levels.
- The filtered Slint model is rebuilt only when entries or the filter change, so
  an unchanged model is not replaced on every 200 ms tick (which would reset the
  scroll position).
- When a filter hides every row, the empty state says
  `当前筛选级别下没有日志。` instead of claiming that no log exists.

### Noise rules and wording

- Opening a native picker (folder or save dialog) is internal UI activity and is
  **not** logged. Results and real failures are still logged: a chosen directory,
  an invalid path, an export success, a failed export.
- The log header's 清空 action stays silent; a "cleared" row would immediately
  reappear in the emptied panel.
- Startup now records the version, whether `config.toml` was loaded, and a
  restored root. Status, settings, root and transfer messages use a consistent
  `标签：值。` shape and end with a full-width period where applicable.

### Export

- The log header gains an 导出 button (disabled while no entry is retained). It
  opens a native save dialog on a background thread through a helper process:
  Windows PowerShell `SaveFileDialog`, or `zenity`/`kdialog` on Unix. The default
  name is `tftpd-r-log-YYYYMMDD-HHMMSS.txt`. The Windows helper uses the same
  tiny top-most owner form as the folder picker, so the dialog is not created
  behind the panel and is activated immediately (2026-11-21 follow-up).
- The text is snapshotted when the button is pressed, so entries arriving while
  the dialog is open do not change the file. `src/logging.rs` renders it
  oldest-first with a header (product version, export time, row count, a note
  that the display filter does not apply) and one `YYYY-MM-DD HH:MM:SS LEVEL
  message` line per entry.
- The operator chooses the path; the panel writes the file and logs
  `运行日志已导出：<path>。` or an actionable error. There is no automatic log
  file, rotation or upload: the file is created only by this explicit action.

## Alternatives considered

- **Keep elapsed time.** Rejected: the user asked for local timestamps, and
  elapsed time is misleading when reviewing an incident after a restart.
- **Threshold filter (WARN shows WARN + ERROR).** This is common in logging UIs,
  but with only three levels it would make INFO identical to 全部. Exact-level
  filters keep every button meaningful; 全部 is one click away. The trade-off is
  recorded here and can be revisited if a DEBUG level is ever added.
- **Export to a fixed path.** Rejected: it writes files the operator did not
  choose and makes overwrite/error handling surprising. The native save dialog
  matches the existing folder picker and asks before replacing an existing file.
- **Clipboard copy instead of a file.** A cross-platform clipboard would require
  another helper or dependency, and a file is the useful artifact for support.
- **Automatic log persistence.** Rejected for now: the log contains local paths
  and peer addresses, and an always-on file would need rotation, redaction and a
  retention policy. An explicit export keeps that under operator control.

## Consequences

- The panel gains one direct dependency (`chrono`, already transitive through
  Slint) and two small modules (`logging.rs`, `log_export.rs`).
- Embedded newlines from TOML/OS errors are flattened; the log is no longer a
  place for multi-line diagnostics. The full error still reaches the settings
  error box and the operator log in one line.
- Exported files contain local root paths and peer addresses. This is documented
  in [docs/security.md](../security.md); export is user-initiated and the panel
  never writes a log automatically.
- The native save dialog itself needs the same target-machine acceptance as the
  directory picker (real desktop interaction); the text formatting, file writing
  and result polling are covered by automated tests.
