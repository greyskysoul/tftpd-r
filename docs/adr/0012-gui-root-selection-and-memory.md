# ADR 0012: GUI root selection, the unset state and the remembered last root

- **Status:** accepted
- **Date:** 2026-11-21
- **Revised:** 2026-11-21 (visible save reminder, write/stale-entry diagnostics,
  portable state fallback)

## Context

User feedback on the `0.2.1` panel reported two related root-directory problems:

1. After selecting a root directory and starting the server, a new launch forgot
   the choice. Because the panel deliberately does not write `config.toml`
   (ADR 0009), the root came from `ServerConfig::default()` again — the relative
   path `tftp-root`.
2. When no root had ever been selected, the status bar and the settings field
   showed `tftp-root` as if it were a configured directory. That directory
   usually does not exist in the working directory, so 启动 failed with a
   `root_dir` validation error and the operator could not tell that the root was
   simply unset.

The requested behaviour is: show **未指定根目录** instead of the placeholder,
leave the settings field empty, and automatically select the root chosen on the
previous run when the panel has been used before.

## Decision

### "No root selected" is an explicit GUI state

- The panel represents "no root selected" as an **empty `PathBuf`** in its own
  `ServerConfig`/`UiState`. `gui_default_config()` sets it; this is a GUI-only
  convention. `ServerConfig::default()` and the daemon keep their documented
  `tftp-root` path, and `config.example.toml` is unchanged.
- The status bar shows `未指定根目录`, and the settings root field is empty (the
  placeholder is `请选择根目录`). No placeholder path is added to the recent
  list or written anywhere as if it were configuration.
- `read_config` trims the field and rejects an empty value with
  `请先选择根目录。` **before** `ServerConfig::validate`, so the empty path never
  reaches `TftpServer`. The manual start path opens the settings dialog and shows
  the message inline; the dialog itself refuses to save an empty root.
- The 浏览… button passes no initial directory when neither the typed value nor
  the configured root is a directory.

### Recent-root menu picker

- The recent-root drop-down (status-bar root selector) gains a **选择…** button
  in its header, next to 清空.
- It closes the menu and starts the same native folder picker, using the current
  configured root as the initial directory when it exists. The helper shows the
  dialog with a tiny, invisible, top-most owner form, so it is activated above
  the panel and other windows instead of opening behind them (2026-11-21
  follow-up; see [docs/testing.md](../testing.md)).
- Unlike the settings dialog's 浏览… button, a directory chosen here is applied
  **immediately**: it becomes the configured root, is recorded in the recent list
  and `last-root.txt`, updates the status bar/settings form, and restarts a
  running server when no transfer is active (same rules as selecting a recent
  entry: deferred with a WARN while transfers exist, stored only while stopped).
- Opening or cancelling the picker is internal UI activity and writes no log
  line; a chosen directory, a restart outcome or an invalid path is logged. See
  [ADR 0013](0013-gui-activity-log.md).

### Remembering the root of the previous run

- A new per-user state file, `last-root.txt`, stores one canonical directory
  path. It is written when a start succeeds and when the operator explicitly
  saves the settings dialog or selects a recent root. Like the other GUI state,
  a failed write does not stop the panel, but it is no longer silent: the panel
  logs a WARN that the next launch will not restore the root.
- The settings dialog's 浏览… button only edits the form. After a pick the form
  shows an inline reminder ("已选择目录，点击“保存设置”后生效。") because the
  activity log is behind the modal dialog; 保存设置 applies and remembers the
  directory. The recent-root menu's 选择… continues to apply immediately.
- Startup precedence for the root is:
  1. a valid `config.toml` (`root_dir`), because that file is the operator's
     explicit, documented configuration and the panel never rewrites it;
  2. otherwise `last-root.txt`, but only while that directory still exists;
  3. otherwise unset (`未指定根目录`).
- A missing or unreadable remembered path opens the panel unset. A stored path
  that is no longer a directory is dropped and reported as a WARN on startup, so
  "no root restored" has a visible reason. When a valid `config.toml` supplies a
  root while `last-root.txt` also exists, the log states that the file wins.
- Restoring the root only fills the form/configuration; the server still starts
  only after an explicit 启动.
- 清空 in the recent-root drop-down removes history only and does **not** forget
  `last-root.txt`. The last active configuration is not history.

### State directory fallback

- All three state files (`recent-dirs.txt`, `last-root.txt`, `ui-state.txt`) use
  the same directory: the per-user configuration directory
  (`%APPDATA%\TftpdR`, or the XDG config directory) when it can be created.
- A normal user profile is writable, but a restricted profile, a low-integrity
  process or a sandboxed launch may not be able to create that directory. On
  Windows this is an integrity-level (MIC) restriction, not an ACL or
  installation issue: a Low-integrity token cannot write a Medium-integrity
  profile directory even when the account has full control of it. Code signing
  is not the cause either: a signature affects SmartScreen reputation and
  WDAC/AppLocker allowlisting, not the permission to write `%APPDATA%`. The
  selection is made per process, not per executable path: the same binary
  writes the per-user directory when launched from a normal Medium-integrity
  desktop and falls back to `.tftpd-r` when launched from a low-integrity or
  sandboxed terminal. Moving the executable does not change permissions by
  itself; the launcher does. The two locations are independent, so root memory
  is not automatically shared between them. When the per-user directory is
  unusable, the panel uses `.tftpd-r` beside the executable, then `.tftpd-r` in
  the working directory, and logs a WARN naming the chosen directory. This keeps
  the remembered root, the recent list and the one-time setup flag working
  instead
  of silently losing them.
- The fallback is only selected while the per-user directory is unusable. If a
  later launch can use the per-user directory, it is preferred and the portable
  files are not read; moving the portable files there is left to the operator.
- The fallback is still advisory local UI state. It can contain local paths, but
  no credentials, and it is never read by the TFTP server. A writable executable
  directory/working directory already implies control over the binary and
  `config.toml`, so this does not add a meaningful new trust boundary.

## Alternatives considered

- **Reuse the first entry of `recent-dirs.txt`.** Rejected: browsing a directory
  records it in the recent list even when the dialog is then cancelled, so the
  "last used root" could be a directory the operator never applied; and 清空
  would silently forget the last root.
- **Write `config.toml` from the panel.** Rejected in ADR 0009 and still
  rejected here: it would require atomic writes, preservation of operator
  comments/unknown fields, and a decision about which values the GUI owns.
  Persisted file configuration remains a daemon/TOML responsibility.
- **Keep displaying `tftp-root` and rely on the validation error.** Rejected:
  the user explicitly asked for an honest unset state, and a non-existent
  relative path is not a usable default for the panel.

## Consequences

- A fresh profile cannot start and does not pretend to have a root; the first
  useful action is choosing a directory in the automatically opened settings
  dialog.
- A returning user sees the previous root in the status bar and the settings
  form without having to create `config.toml`.
- Deployments that do pin `config.toml` keep file authority: changing the root
  in the panel remains session-only, exactly as before.
- One more advisory state file exists (one path). A read-only profile still
  works, but it now logs a WARN and the next launch opens unset instead of
  pretending the root was remembered. If the whole per-user directory is
  unusable, the portable `.tftpd-r` fallback keeps the state working and the
  startup WARN explains where it went.
- Tests cover `LastRoot` persistence/stale/read-only handling (including the
  changed flag and write errors), the state-directory selection/fallback, the
  startup precedence, empty-field rejection and the status text. Manual
  `PrintWindow` captures (isolated `APPDATA`, synthetic window messages,
  prefilled `config.toml`) verify the unset first run, the restored
  field/status, a save followed by a relaunch, the stale-entry WARN, the
  write-failure WARN and the portable fallback with the real (unwritable)
  `APPDATA`; they are not a substitute for a real mouse/desktop acceptance run.
