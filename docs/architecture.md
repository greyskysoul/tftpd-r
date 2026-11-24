# Architecture

This document describes the implemented MVP (Phase 0–5 of the original plan).
Deviations from the plan are recorded in [`docs/adr/`](adr/).

## Crate boundaries

```text
tftpd-r-gui-slint ──┐
                    ├──> tftpd-r-core
tftpd-r-daemon ────┘

tftpd-r-gui-slint ──┐  (build-dependencies only)
tftpd-r-daemon ────┴──> tftpd-r-build-support
```

- **`tftpd-r-core`** owns the TFTP protocol codec, option negotiation, the RRQ
  session state machine, the filesystem sandbox, configuration validation and
  the event bus. It depends on Tokio, serde/toml, thiserror and tracing, but not
  on Slint, clap, tracing-subscriber or any IPC transport.
- **`tftpd-r-daemon`** is a thin front-end: clap CLI, TOML file loading, tracing
  logging, signal handling and process exit codes. It builds the
  `tftpd-r-server` executable.
- **`tftpd-r-gui-slint`** is a second front-end: a Slint window plus an embedded
  controller that owns a Tokio runtime and a `tftpd-r-core` server handle. It
  does not talk to the daemon (no IPC in this release) and builds the `tftpd-r`
  executable.
- **`tftpd-r-build-support`** is build-time only and is not linked into either
  binary. It turns `crates/tftpd-r-gui-slint/ui/icons/app-icon.ico` into a
  Windows `.res` resource: MSVC links the `.res` directly, GNU targets convert
  it to a COFF `.rsrc` object with `windres`. On non-Windows targets it is a
  no-op, so Linux builds do not need a resource compiler.

There is no `tftpd-r-control-protocol` crate yet because embedded mode does not
need shared DTOs. When IPC is added, put versioned DTOs in a separate crate so
`tftpd-r-core` is not coupled to the transport.

## Runtime ownership

`tftpd-r-core` uses Tokio primitives but **never creates a runtime**. The
front-end owns the runtime:

- The daemon builds a multi-thread runtime in `main` and `block_on`s
  `serve_async`; Ctrl-C, Ctrl-Break (Windows console events) and SIGINT/SIGTERM
  (Unix) request a graceful stop.
- The GUI builds a two-worker runtime in `EmbeddedController::new`, uses
  `block_on` for the (fast) `TftpServer::start` call and spawns shutdown work so
  the UI thread is never blocked for the full grace period.

`ServerHandle` therefore carries an implicit requirement: the runtime that
started it must stay alive. See [`adr/0001-runtime-ownership.md`](adr/0001-runtime-ownership.md).

## Server lifecycle

`TftpServer::start(config, event_sink)`:

1. Validates `ServerConfig` (including root directory existence and the bind
   address list).
2. Canonicalises the root with `Sandbox::new`.
3. Binds one UDP socket per effective bind address on the shared `port` (port 0
   is allowed for tests: the first listener resolves the ephemeral port and the
   rest reuse it). Binding is all-or-nothing; a failure drops the sockets bound
   so far and returns the classified error.
4. Spawns one accept loop per listener and returns a `ServerHandle` whose
   `local_addrs()` reports the actual bound addresses.

Status transitions: `Starting → Running { local_addrs } → Stopping → Stopped`,
or `Failed` if the server task fails. Binding/validation failures are returned
synchronously from `start`, so no half-started listener exists.

`ServerHandle::stop()` is idempotent: it sends `Stopping`, cancels the
`CancellationToken`, waits for the supervisor (which joins every accept loop),
then awaits the join handle. Each accept loop cancels its sessions, drains them
for `shutdown_grace_secs` in parallel with the other loops, aborts the remainder
and releases its socket. Dropping a handle without `stop()` still cancels the
token (sessions finish on their cancellation path), but does not wait for them.

## Accept loops and sessions

- One accept loop owns one listening socket. Each loop receives RRQ/WRQ and
  malformed datagrams on its own address; request datagrams larger than
  `MAX_REQUEST_BYTES` (1024) are rejected with ERROR 4.
- The peer→session map, the per-peer session count, the global session
  semaphore and the session-id counter are shared by all listeners. A duplicate
  RRQ from a peer that already has a session does **not** create a second
  session, even when the retry arrives on another listening address: the loop
  sends a `SessionSignal::Resend` to the existing session, which retransmits its
  current packet. This keeps the TID stable for the client and prevents
  task/port exhaustion from retried requests.
- Each session gets its own ephemeral UDP socket (the TID), an event session ID,
  a `JoinSet` slot and a semaphore permit. A specific listener address is reused
  as the session socket address, so a multi-homed client sees DATA from the
  address it contacted; wildcard listeners bind `0.0.0.0`/`::` and let the OS
  pick the source address.
- Global concurrency is bounded by `max_sessions`; a per-IP cap of 8 further
  limits one peer. Rejected requests receive a small ERROR packet and an
  event/log entry.
- The session socket only accepts packets from the expected `peer` address.
  Other sources get ERROR 5 (unknown TID), capped at 8 responses per block
  exchange. Packets from the peer that are not the expected ACK are ignored, so
  duplicates and reordering do not corrupt state.

## RRQ state machine

For each session:

1. Open the file through the sandbox (size limit applied).
2. Negotiate options (`blksize` capped by config, `timeout`, `tsize`) or fail
   with ERROR 8.
3. Emit `Started` with the file size.
4. If options were negotiated: send OACK and wait for ACK 0.
5. Send DATA blocks starting at 1, waiting for the matching ACK. The exact
   encoded DATA packet is retransmitted on timeout (bounded by `max_retries`);
   RRQ retransmissions also trigger an immediate resend.
6. A block shorter than the negotiated size ends the transfer; a full final
   block is followed by a zero-length DATA block, as required by RFC 1350.
7. Emit `Progress` at most every 200 ms and `Completed`/`Failed` at the end.

Block numbers use 16-bit `wrapping_add`, tested across the 65535 → 0 boundary
with `blksize = 8`.

The transfer deadline (`max_transfer_duration_secs`) is checked before sending
each block and while waiting for ACKs.

## Filesystem sandbox

`Sandbox` canonicalises the root once and validates every request syntactically
before touching the filesystem. A leading `/` or `\` is mapped to the root for
legacy clients (`/firmware.bin` → `<root>/firmware.bin`); after that mapping,
`..`, drive letters/`:`, device names, alternate data streams and reparse points
are rejected. The file is then opened through a component walk, and the final
canonical path is compared component-wise with the canonical root. Details and
residual risks are in [`security.md`](security.md) and
[`adr/0003-path-sandbox.md`](adr/0003-path-sandbox.md).

## Events

`EventSink` is a small synchronous trait. The daemon's `TracingSink` maps events
to `tracing` records; the GUI uses `EventBus`, a bounded ring guarded by a
`Mutex`.

- `EventBus::emit` assigns a monotonic `seq` and drops the oldest event when the
  ring is full, incrementing `dropped`. Lifecycle events are bounded in practice
  by session limits; the ring size is configurable (`event_buffer`).
- The GUI polls `drain_since(last_seq)` from a Slint timer every 200 ms, updates
  models and never blocks the server on a slow subscriber.
- `Progress` events are throttled per session so a fast transfer cannot flood the
  ring or the UI.

## GUI structure and state

The GUI crate stays deliberately small:

- `ui/app.slint` composes the window from `titlebar.slint`, `toolbar.slint`,
  `logs.slint`, `transfers.slint`, `statusbar.slint` and four overlays:
  `root-menu.slint` (recent root directories), `settings.slint`,
  `about.slint` and `close-prompt.slint` (the close-time three-way choice,
  drawn last so it covers the whole window). `ui/single-instance.slint` is a
  separate `InstancePromptWindow` for a second launch and `ui/tray.slint` is the
  notification-area icon; neither is a child of `AppWindow`.
- The window is frameless (`no-frame: true`). `titlebar.slint` draws the title
  and minimize / maximize-restore / close controls, and uses `WindowMoveArea`
  for dragging. `resize-border-width` keeps the native edge/corner resize
  behaviour, and a one-pixel dark frame drawn by `app.slint` bounds the client
  area. The drag area stops before the window buttons so pressing a button
  cannot also start a window move. The title bar and the toolbar share the same
  flat `#37474f` fill and draw no hairline between them, so the two rows read as
  one header block; the dividing rules that remain are the vertical ones inside
  each row.
- `ui/widgets.slint` holds the icon-over-label toolbar button, compact buttons,
  the drop-down trigger, the text field and panel primitives; `ui/theme.slint`
  centralises the palette (both schemes, every colour token); `ui/icons.slint`
  embeds Material Design icon path data, so no image decoder or extra image
  files are needed at runtime.
- **Language.** Static UI text is written in English and wrapped in `@tr(...)`;
  `translations/zh_CN/LC_MESSAGES/tftpd-r-gui-slint.po` is bundled by
  `build.rs` (`slint_build::CompilerConfiguration::with_bundled_translations`
  plus `with_default_translation_context(DefaultTranslationContext::None)`, so
  the catalogue has no `msgctxt`). `src/i18n.rs` resolves the persisted
  `LanguagePreference` (default: follow the system locale) to `Language::En` or
  `Language::ZhCn` before the window is created and calls
  `slint::select_bundled_translation` afterwards, which also overrides Slint's
  own (language-part-only) locale guess. The settings dialog's Appearance page
  stores a new preference in `ui-prefs.txt`, but the running process keeps its
  startup language: the activity log holds rendered strings and the native
  dialogs captured their titles, so the new language appears on the next start.
  The same module's `Text` renders every string the Rust side builds (log lines,
  validation errors, status text, address labels, export header, native dialog
  titles), so both halves of the panel always use one language. See
  [ADR 0015](adr/0015-gui-language-and-theme.md).
- **Theme.** `src/theme.rs` owns `ThemeMode` (follow the system / light / dark),
  the persisted choice and the platform probe: Windows reads
  `HKCU\...\Themes\Personalize\AppsUseLightTheme` through `reg.exe`, Unix asks
  `gsettings` and then `GTK_THEME`. `SystemThemeWatcher` reads the value once on
  the UI thread at startup (so the first frame is correct) and then every 30 s on
  a worker thread; the probe is disabled while light/dark is forced and stopped
  at shutdown. The resolved boolean is pushed into the Slint window, which copies
  it into the `Theme` global and into `Palette.color-scheme` (the standard
  widgets the panel embeds are `ScrollView`s, whose scrollbars follow that
  palette). Every colour in `ui/theme.slint` is a function of that flag, so the
  two schemes cannot drift apart per component.
- The service status is a dedicated chip at the far left of the toolbar; the
  status bar keeps the read-only hint or the last error, the bound address(es)
  and the clickable root selector (last path component, full path in the
  settings dialog and recent-directory menu). A panel without a selected root
  shows `No root directory` (`未指定根目录` in Chinese); there is no placeholder
  path. The recent-root drop-down also offers Select…, which opens the native
  folder dialog and applies the chosen directory immediately.
- The settings dialog has a left navigation bar with two pages. **Server** edits
  `port`, the multi-select bind address list, the root, the block size, the
  timeout and the maximum session count; **Appearance** holds the theme and
  language choices and applies to the panel only. The address picker always
  offers `0.0.0.0` ("All networks") and `127.0.0.1` ("Loopback"), merges
  the discovered interface addresses and never drops a selected address that
  discovery did not report. "All networks" is exclusive and sticky: selecting it
  clears the other addresses, selecting another address clears it, and it cannot
  be unchecked (unchecking the last specific address selects it again), so the
  bind list is never empty. The root field has a Browse… button backed by a
  native directory dialog and a history button that opens the recent-root list;
  the helper behind the dialog is pre-warmed when the settings dialog (or the
  recent-root menu) opens, so the click does not pay for PowerShell startup (see
  [ADR 0014](adr/0014-prewarmed-folder-picker-helper.md)). Picking a directory
  (through Browse… or the history list) only edits the form (Cancel must be able
  to restore the active value), so the form shows an inline "click Save settings
  to apply" reminder; Save settings applies and remembers it. The Appearance page
  is the only place whose controls apply independently: the theme applies
  immediately, the language is stored for the next start, and neither restarts
  the server. When no valid `config.toml` exists, the panel preselects `0.0.0.0`;
  the core and daemon defaults stay loopback. On the first start of a profile
  without a valid configuration the dialog opens automatically, once (see
  `src/user_state.rs`).
- The GUI uses an **empty root path** as its "no root selected" state (a
  GUI-only convention; `ServerConfig::default()` and the daemon keep
  `tftp-root`). The status bar then shows `未指定根目录` and the settings field is
  empty; saving or starting validates the field before `ServerConfig::validate`,
  opens the dialog and reports `请先选择根目录。`, so an empty path is never passed
  to the server. On startup the root is chosen in this order: a valid
  `config.toml` (`root_dir`), otherwise the remembered root of the previous
  start (`last-root.txt`, only when it still is a directory), otherwise unset.
  The remembered root is written on a successful start and on an explicit save
  or recent-root selection; it is not part of the recent history, so 清空 does
  not forget it. A failed write keeps the value for the current session but logs
  a WARN (the next launch will not restore it); a stored entry that is no longer
  a directory is reported as a WARN on the next launch, and a valid
  `config.toml` that overrides it is reported too.
- **Second launch.** `src/single_instance.rs` owns an advisory lock file per
  running panel (`instance-<token>.lock`, refreshed from the UI timer every 3 s,
  stale after 15 s) plus a `activate-request-*.req` / `activate-ack-*.ack`
  handshake in the per-user state directory. A launch that finds a fresh lock
  shows the standalone `InstancePromptWindow` (`ui/single-instance.slint`)
  instead of registering its own lock; Activate writes the request and exits when
  the owner acknowledges (the owner raises itself with `set_minimized(false)` +
  `Window::show()`), New instance shows the main panel, registers a lock and
  continues, Cancel exits without a lock. The main panel is created but never
  shown while the prompt is open, and `--new-instance` / `--activate-only` are
  the scriptable forms. The files are advisory UI state, not an IPC channel; see
  [ADR 0017](adr/0017-single-instance-prompt.md) and
  [ADR 0018](adr/0018-tray-icon-and-close-choice.md).
- **Close choice and notification area.** With the server `Starting`/`Running`,
  the title-bar close button and `Window::on_close_requested` open the
  `close-prompt.slint` overlay: close-and-exit hides the window and quits,
  run-in-background keeps serving, cancel restores the panel. Background mode
  creates `ui/tray.slint` (Slint `SystemTrayIcon`; the `system-tray` feature is
  enabled workspace-wide and selects `Shell_NotifyIconW`, `ksni` or AppKit at
  compile time) on first use, shows the icon and hides the window on the next
  200 ms tick. The tray menu and left-click raise the window; its Quit entry
  stops the event loop. The same `show_main_window` helper is used when another
  launch or `--activate-only` raises a background panel, so the hidden window
  reappears and the tray icon is removed. A desktop without a StatusNotifier
  host (Linux/BSD) shows no icon; the last-window rule then makes the process
  exit instead of leaving an unreachable background process, and a later launch
  can still reach the panel through the single-instance prompt. See
  [ADR 0018](adr/0018-tray-icon-and-close-choice.md).
- Saving the dialog, or selecting another recent root, compares the resulting
  `ServerConfig` with the configuration of the running server. When the server
  is running (`Starting`/`Running`) and no transfer is active, the change is
  applied with an asynchronous `EmbeddedController::restart`; with active
  transfers it is only stored and the log asks for a manual restart. A stop or
  restart that is already in progress (`Stopping`) is never turned back into a
  start. `poll_status` no longer reports a stale `Running` after the handle was
  taken by `stop()`.
- `src/main.rs` owns the Slint models (`LogRow`, `TransferRow`, `RecentDir`,
  `AddressOption`), translates `TftpEvent`s and `ServerStatus` into UI state,
  and keeps the UI thread free of blocking calls. Window controls call
  `slint::Window` methods (`set_minimized`, `set_maximized`) on the UI thread;
  close requests route through the close-time prompt (or quit directly when the
  server is not active) so the normal window-close shutdown path runs. The event
  loop keeps running while the prompt window or a visible tray icon exists; the
  main window may be hidden without stopping the process.
- `src/logging.rs` owns the bounded in-memory activity log: at most 500 entries
  (newest first), local wall-clock timestamps, `Info`/`Warn`/`Error` levels, an
  exact-level display filter selected from a drop-down that never drops entries,
  single-line message normalisation and the plain-text export format.
  `src/log_export.rs` runs the native save dialog in a helper process on a
  background thread; the UI timer writes the snapshot and reports success or an
  actionable failure. The log table is rebuilt only when entries or the filter
  change, so the scroll position survives the 200 ms refresh. Opening a
  folder/save picker is not logged; only the chosen directory, export result and
  real errors are.
- The recent-root drop-down model (`RecentDir`) is attached once and replaced
  only when its visible rows (path, leaf name, current marker) actually change.
  `VecModel::set_vec` is a model reset: Slint destroys and recreates every
  repeated row, which clears `has-hover` and discards a press that spans a UI
  tick. The former unconditional rebuild on every 200 ms tick made the entries
  look like they were constantly refreshing: hover disappeared immediately and
  only a fast click selected a directory.
- `src/folder_picker.rs` owns the native directory dialog. On Windows one
  PowerShell + WinForms helper process is kept warm for the session: a
  background worker starts it the first time the settings dialog or the
  recent-root menu opens, waits for its `READY` line and then sends one UTF-8
  request line (the initial directory) per click, receiving `PICK:<path>` or
  `PICK:` back. Reusing the helper keeps PowerShell/.NET startup out of the
  click path (about 0.2 s for a warm dialog instead of about 1.6 s). If the
  helper cannot be started or dies, the worker falls back to the one-shot
  helper for the rest of the session; `shutdown()` kills a helper (and an open
  dialog) when the panel closes. On Unix the worker keeps the one-shot
  `zenity`/`kdialog` behaviour. `src/log_export.rs` still runs a one-shot
  PowerShell `SaveFileDialog` (or `zenity`/`kdialog`) for the log export.
- Both dialogs use fixed, absolute commands without a shell. The folder
  picker's initial directory and selected path travel over stdin/stdout as
  UTF-8 — a Windows path cannot contain a line break — so a non-ASCII directory
  name survives the round trip; the save dialog receives its default file name
  through `TFTPD_R_LOG_NAME`. Their titles and the save dialog's file-type
  filter are translated and travel the same way (`TFTPD_R_PICKER_TITLE`,
  `TFTPD_R_SAVE_TITLE`, `TFTPD_R_SAVE_FILTER`, or fixed `--title` arguments on
  Unix), so no message text ever becomes script text. The directory is accepted
  only if it still is an existing directory; the save path does not have to
  exist yet.
- `src/netif.rs` discovers interface addresses on a background thread; the UI
  timer merges the result into the picker. It asks the platform runtime for the
  list (Windows PowerShell + .NET `NetworkInterface`, `hostname -I` on Unix) and
  parses only IP literals; failure is not fatal.
- `src/recent.rs` persists at most eight canonicalised root directories
  (most-recent first). The drop-down's 清空 button removes the state file; the
  configured/running root is not history and is unaffected. It is local UI
  state, not part of `ServerConfig`, and write failures are ignored so a
  read-only profile still runs the panel.
- `src/user_state.rs` selects the state directory and stores the one-time setup
  flag (`ui-state.txt`), the remembered root (`last-root.txt`, one canonical
  path written on a successful start or an explicit selection) and the
  appearance choice (`ui-prefs.txt`, `theme-mode=system|light|dark`). The
  recent-root file uses the same directory helper. The per-user configuration
  directory is
  preferred; if it cannot be created (restricted profile or a
  low-integrity/sandboxed launch), a portable `.tftpd-r` directory beside the
  executable, then in the working directory, is used instead and the panel logs
  a WARN. All four files are advisory: a missing or unwritable profile only
  means the panel asks for setup again / keeps no history / forgets the last
  root / falls back to following the system theme. A UTF-8 BOM a text editor may
  add is stripped before parsing. The `last-root.txt` path is the exception to
  "write failures are ignored": `LastRoot::record` returns a `Result` and the
  panel logs a WARN, because silently losing the remembered root is
  indistinguishable from a broken setting; a failed `UiPrefs` write logs a WARN
  as well.
- Selecting a recent root, or choosing 选择… in the same menu, while the server
  runs either restarts it automatically (no active transfer) or updates the
  configuration for the next manual restart; the status bar keeps showing the
  root of the running server. The same rule applies to saving the settings
  dialog.
- The GUI is a windowed-subsystem binary on Windows (`windows_subsystem =
  "windows"`), so starting the panel does not allocate a console window.

Dialogs and the drop-downs (recent roots, bind address picker) are sibling
rectangles inside the same Slint window rather than separate native windows;
this keeps the software renderer and DPI handling simple and guarantees they
render above the main layout. The activity-log **level** list is the one nested
overlay: it is a sibling of the log panel's layout (inside the panel's clip), so
it draws over the log rows without a second native window. The layout relies on
explicit positions for sized boxes that are not managed by a layout (borders and
progress fills), because a sized child without `x`/`y` is centred by the
renderer.

## GUI threading

- Slint runs on the main/UI thread. All UI property access happens in callbacks
  or in the Slint `Timer` closure.
- The embedded server runs on the controller's Tokio runtime. No Tokio task
  touches UI properties directly; communication is pull-based through the event
  bus and a `watch` channel for status.
- `stop()` spawns the potentially slow `ServerHandle::stop()` and returns; the
  timer observes `Stopping → Stopped`. Window close calls
  `EmbeddedController::shutdown`, which blocks only after the UI loop has exited.
- `restart()` queues the new configuration, stops the running handle and starts
  the replacement from `poll_status` once the old server reaches `Stopped`. While
  a restart is pending a stale `Running` watch value is reported as `Stopping`,
  and a failed replacement is surfaced through `take_error()` instead of leaving
  a half-started server.
- Log rows are capped at 500 in the model; the event ring is independently
  bounded. Logs are displayed newest-first so the latest event stays visible
  without programmatic scrolling. Log text is rendered in the startup language;
  changing the language preference takes effect on the next start, which keeps
  existing rows and the native dialog titles consistent (ADR 0015).
- `SystemThemeWatcher` runs one worker thread that re-reads the operating-system
  theme every 30 s while the panel follows the system; the UI timer copies the
  newest value into the state and re-applies the palette on the UI thread. The
  worker is detached on shutdown and never blocks the event loop.
- The title-bar maximize icon follows `Window::is_maximized()` on the UI timer;
  the OS window state is authoritative rather than a UI-side toggle. Minimize,
  maximize and close are plain UI-thread calls and never wait for the server.

## Configuration and validation

`ServerConfig` is serde-derived with `deny_unknown_fields` and safe defaults. The
same `validate()` is used by the daemon, the GUI and tests, so a configuration
accepted by one front-end is accepted by the other. CLI overrides are applied and
re-validated. TOML syntax errors include the file path via `ConfigLoadError`.

The listen configuration is `port` plus `bind_addresses` (a list of IP
addresses). Exact duplicates are dropped and a wildcard (`0.0.0.0` / `::`)
collapses the other addresses of its family in `effective_bind_addresses()`;
multicast/broadcast entries and lists longer than 16 are rejected. The 0.1.0
`bind_address` key is gone, so an old file fails with an unknown-field error
instead of silently binding something else. Daemon `--bind` replaces the whole
list with one address and port. `ServerConfig::default()` stays on loopback; only
the GUI's no-config form preselects `0.0.0.0` (ADR 0009), and the GUI does not
write `config.toml`.

## Logging

- Library code logs operational events at `debug`/`warn` through `tracing`; it
  does not print to stdout.
- The daemon initialises a global `tracing-subscriber` and logs lifecycle
  events. The filter comes from `--log` or `TFTPD_R_LOG` and defaults to `info`;
  `logging.rs` parses the level-based subset itself (a level, `target=level`, or
  a bare `target` meaning `trace`) into a `Targets` filter instead of enabling
  `env-filter`, which would link the `regex` engine into the daemon (ADR 0016).
  A target-only filter keeps the `info` default for other targets, and invalid
  directives are reported and ignored rather than aborting startup.
- Error text sent to clients is a fixed, sanitised string per error class; local
  paths and OS error messages stay in local logs (and even there only the file
  name and error kind are used for client-visible paths).
