# ADR 0018: Tray icon, close choice, and a standalone single-instance prompt

- **Status:** accepted
- **Date:** 2026-11-24
- **Amends:** [ADR 0017](0017-single-instance-prompt.md) (the prompt is now a
  separate small window; the activation protocol itself is unchanged)

## Context

The panel embeds the only TFTP server, so closing its only window stops
serving. Operators asked for the usual desktop behaviour of a server app:

- a notification-area (system tray) icon while the panel is in the background;
- a close-time choice between *close and exit*, *run in the background* and
  *cancel* when the server is still running;
- the existing duplicate-launch prompt must cooperate with the background
  process, and "Show program window" must also raise a window that was hidden
  to the tray;
- the duplicate-launch prompt itself should be a small standalone window, not
  the full panel (ADR 0017 had deliberately used an in-window overlay).

Constraints:

- Slint 1.19 ships a native `SystemTrayIcon` element (`Shell_NotifyIconW` on
  Windows, `ksni`/StatusNotifierItem on Linux/BSD, AppKit on macOS). The
  `system-tray` feature is required for the native backends; without it the
  element compiles against a dummy backend.
- The Linux build must not gain a GTK/AppIndicator build dependency. `ksni` is
  pure Rust, but a StatusNotifierItem host is still required at runtime and this
  environment cannot validate one.
- `unsafe_code = "deny"` remains in force for hand-written code. A direct
  Win32 tray implementation is therefore not available; the Slint element is
  the supported path.
- The panel event loop is `slint::run_event_loop()`, which stops when the last
  window or visible tray icon disappears. Keeping the process alive in the
  background must not depend on a race between "hide window" and "tray icon
  registered".
- ADR 0017's lock/heartbeat/request/ack handshake stays the only cross-process
  channel; it must not become a control channel.

## Decision

### Tray icon

- Enable Slint's `system-tray` feature in the workspace `slint` dependency.
  Windows gets the native `Shell_NotifyIconW` backend; Linux/BSD compiles
  `ksni` (pure Rust StatusNotifierItem, no GTK build dependency); macOS uses
  AppKit. The Linux backend is compiled but its desktop-host behaviour is
  unverified.
- `ui/tray.slint` is a `SystemTrayIcon`-rooted component with a *Show program
  window* / *Quit* menu. Its inherited `icon`, `tooltip` and `visible`
  properties are mirrored through component-level `tray-icon`,
  `tray-tooltip` and `tray-visible` properties because Slint's generated tray
  root does not expose inherited property setters.
- The tray component is created lazily on the first "Run in background"
  choice. Slint creates the platform handle on the first event-loop turn and
  does not retry after a failed icon conversion or `NIM_ADD`; creating it at
  startup would either flash an icon or fail before the panel is shown. The
  icon and tooltip are assigned before the first turn.
- **Visibility before hiding:** selecting background sets the tray visible and
  records `pending_background`; the 200 ms UI timer hides the window on the
  next tick. This gives Slint's delayed visibility tracker one turn to register
  the icon (and the event-loop keep-alive) before the last window disappears.
  If the tray handle cannot be created, hiding the last window makes the
  backend stop the process instead of leaving an unreachable background
  process. A desktop without a StatusNotifier host therefore degrades to
  "the panel exits" rather than an invisible background process; starting the
  panel again re-opens the single-instance prompt.
- The tray callbacks keep a weak handle of the tray component, never a strong
  one: the component owns its own callbacks, and a strong self-reference would
  leak the icon. Left-click and the menu entry both raise the window and hide
  the tray; *Quit* calls `quit_event_loop()` and the normal shutdown path stops
  the embedded server.
- On exit (`Close and exit`, tray *Quit*, prompt cancel/ack, or the main
  function's cleanup) the icon is hidden and the component dropped.

### Close choice

- `ui/close-prompt.slint` is an in-window modal overlay (scrim + card) with
  *Close and exit*, *Run in background* and *Cancel*.
- Both the custom title-bar close button and `Window::on_close_requested`
  (Alt+F4 / OS close) call the same helper. If the server is `Starting` or
  `Running`, the helper opens the prompt and returns
  `CloseRequestResponse::KeepWindowShown`; `Stopping`, `Stopped` and `Failed`
  exit directly.
- *Close and exit* hides the window, calls `quit_event_loop()` and lets
  `EmbeddedController::shutdown()` drain the server within
  `shutdown_grace_secs`. *Cancel* only clears the prompt.
- The background button is offered on every platform because the tray backend
  is compiled everywhere. If creating the tray component fails, the panel logs
  a warning and keeps the window instead of hiding it.

### A standalone single-instance prompt

- ADR 0017's overlay is replaced by `InstancePromptWindow` in
  `ui/single-instance.slint`: a fixed 560 × 340 frameless window with its own
  compact title bar and the same three choices. It is not part of
  `AppWindow`'s visual tree.
- `AppWindow` is still constructed (it holds the server controller, models and
  callbacks), but it is not shown while the prompt is open. Choosing *New
  instance* shows the main window **before** hiding the prompt, so the event
  loop always has at least one visible surface during the switch. *Cancel* and
  *Show program window* quit without ever showing the full panel.
- The prompt's own `Theme` global is initialised from the same resolved
  light/dark state; the startup language selection is global. The prompt is
  updated if the system theme changes while it is open.
- `service_single_instance` raises a hidden/background window through the same
  `show_main_window` helper used by the tray. It restores
  `set_minimized(false)`, calls `Window::show()`, clears the close prompt and
  hides the tray icon. `--activate-only` therefore also wakes a background
  panel.

### Text and tests

- New tappable strings are English `@tr(...)` sources in the two `.slint`
  files; Chinese goes to `tftpd-r-gui-slint.po`. Rust-side messages
  (`background_running`, `background_unavailable`, `tray_tooltip`,
  `close_prompt_server_running`, `window_show_failed`, `window_hide_failed`)
  live in `i18n::Text`.
- Unit tests pin `server_is_active` to `Starting`/`Running`, the tray
  menu/show/quit shape, the close prompt's three choices and the fact that the
  duplicate-launch prompt is a `Window` and no longer an `AppWindow` overlay.
  Translation catalogue coverage is enforced by the existing `i18n` tests.

## Alternatives considered

- **Direct Win32 `Shell_NotifyIconW` implementation.** Rejected: hand-written
  `unsafe` is forbidden, and the Slint element already implements the same
  native backend.
- **PowerShell/WinForms tray helper.** Rejected: it adds a persistent
  PowerShell process per background panel, a second event protocol and large
  memory overhead, and would not work on Unix.
- **Windows-only `system-tray` feature.** Rejected: Cargo unifies features
  across the graph, so a target-specific dependency still pulls `ksni` into the
  Linux build; enabling the feature globally makes that dependency intentional
  and gives Linux a StatusNotifierItem backend instead of an unused one. Its
  desktop-host behaviour remains unverified and is documented as such.
- **Create the tray at startup and keep it hidden.** Rejected: Slint creates
  the native handle on the first event-loop turn even for a hidden tray, so an
  empty icon fails permanently (observed `CreateIconIndirect`/`NIM_ADD`
  errors) and a real icon would flash at startup.
- **Use `run_event_loop_until_quit()` so background mode never depends on the
  tray.** Rejected: if the platform tray fails, the process would live with no
  visible UI. The default last-window rule plus the next-tick hide is a safe
  failure (the process exits).
- **Always show the tray icon while the panel is open.** Rejected for now: the
  requested behaviour is a background icon; an always-present icon adds a
  second always-visible surface and more state to keep in sync.
- **Keep the prompt as an `AppWindow` overlay (ADR 0017).** Superseded by the
  explicit request for a small separate window; the cost is a second Slint
  window and its own theme bootstrap, which this ADR accepts.

## Consequences

- On Windows the operator can close the window while the server runs, keep it
  serving from the notification area, and raise it again from the tray, a
  second launch's prompt, or `--activate-only`. Linux/BSD builds also compile
  the tray backend, but a StatusNotifierItem host (and, on GNOME, an AppIndicator
  extension) is required; the icon is not visible without one.
- The process exits instead of silently hiding when the tray cannot be
  registered; the operator sees the window disappear (and a warning is logged
  when component creation itself fails).
- New dependencies (`system-tray`, `ksni`, `async-channel` and their
  transitive crates) are in the lockfile for every platform even though the
  native backends are platform-specific. The GUI binary grows by about 76 KiB
  on the GNU x64 release build (measured 9,233,408 → 9,309,184 B, +75,776 B,
  +0.82 %) and the daemon is unchanged. Linux binary size was not measured
  because the Linux GUI was not built or run in this environment.
- Manual desktop verification of the tray menu/click, Alt+F4, background
  restoration and the small prompt window is still required; CI can only
  compile and run the unit tests.
