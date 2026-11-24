# ADR 0017: A second panel launch asks before starting another instance

- **Status:** accepted; amended by [ADR 0018](0018-tray-icon-and-close-choice.md)
  (the prompt is now a standalone small window and a background panel can be
  raised from the notification area)
- **Date:** 2026-11-24

## Context

TftpdR is a desktop panel that embeds the TFTP server. Launching it while a
panel was already open started a second full process: a second window, a second
embedded server that could not bind the same UDP port, a second copy of the
activity log, and two processes refreshing the same per-user state files. The
operator asked for the usual desktop behaviour instead: when a launch detects
that a panel is already running, it should ask whether to raise the existing
window, start another instance on purpose, or cancel.

Constraints and related decisions:

- The panel is Slint + winit with the software renderer and `unsafe_code =
  "deny"`; a Win32 `FindWindow`/`SetForegroundWindow` implementation is not
  available without either `unsafe` or a new windowing dependency.
- The repository deliberately has no GUI/daemon IPC yet (ADR 0004, roadmap
  Phase 7). That decision is about controlling the *server* from a daemon; the
  startup handshake below is not a general IPC channel and must not grow into
  one.
- Per-user state already lives in a writable directory with a documented
  portable fallback (`user_state.rs`): `%APPDATA%\TftpdR` on Windows,
  `$XDG_CONFIG_HOME/tftpd-r` or `~/.config/tftpd-r` on Unix.
- A failed or unavailable state directory must never prevent the panel from
  starting; losing root memory or the recent list is already accepted.
- "Arch Linux users may run two panels on purpose" must stay possible: a
  detection that refuses to start a second instance would be a regression.

## Decision

### Detection is an advisory lock file with a heartbeat

`src/single_instance.rs` owns a tiny file protocol in the state directory:

- Each running panel writes `instance-<token>.lock` (token = 32 hex characters,
  derived from the PID, the wall clock and a per-process counter) and refreshes
  the file every 3 s from the UI timer. The content is `pid=`, `token=`,
  `heartbeat=` and `stale-after=` for diagnostics.
- A launch scans the directory. A lock file modified within 15 s means "a panel
  is alive"; older files belong to crashed or frozen panels and are removed.
  The threshold is five heartbeats, so a busy machine cannot produce a false
  "stale" verdict, at the cost of up to 15 s of a stale prompt after a crash.
- The lock is advisory, not exclusive: two panels can coexist (that is the
  "New instance" choice), so the lock cannot be an exclusive file handle. The
  name-token filter (exact prefix, suffix and a 32-hex token) means unrelated
  state files are never deleted; a corrupt lock whose timestamp is still fresh
  is reported as a running panel instead of being removed.
- A missing/unwritable state directory disables the check and the panel starts
  normally (logged as a WARN). This keeps the previous behaviour as the worst
  case, exactly like the other state files.

### "Show program window" is a request/answer handshake, not foreground stealing

The second launch cannot raise the first process's window directly without
Win32 calls. Instead:

1. The second launch writes `activate-request-<token>.req` with its PID and a
   deadline 2 s in the future.
2. The owning panel polls for requests on each 200 ms UI tick, removes the
   request (the successful removal makes the claim exclusive), raises its
   window and writes `activate-ack-<token>.ack`. Raising means
   `set_minimized(false)` + `Window::show()`; the OS may still decline to hand
   over the foreground (Windows then flashes the task-bar button), which is
   documented and accepted.
3. The second launch sees the acknowledgement and exits. If the deadline
   passes, it says the existing window did not respond and leaves the three
   choices available.

An expired request that reaches the owner after the requester gave up is
removed without raising anything, so "Cancel" cannot wake the panel later.

### The prompt is the first (and only) surface of the second launch

- `ui/single-instance.slint` is a modal overlay with exactly three choices:
  *Show program window* (primary), *New instance* and *Cancel*. It is the last
  child of `AppWindow`, so it covers the title bar and every other dialog, and
  the main window is hidden before the event loop starts so a launch that ends
  in Cancel or Show program window never flashes a panel.
- The prompt is shown only for a normal launch. `--new-instance` starts
  directly; `--activate-only` writes the request from a console path without
  creating a window and exits; `--smoke-test` and the screenshot previews skip
  the check so development runs never consume the setup flag or prompt.
- Nothing is registered while the prompt is open: Cancel and Show program window exit
  without leaving a lock, and only "New instance" writes one. A launch that
  finds nothing registers immediately.
- The first-run settings wizard is suppressed while the prompt is open: the
  operator is choosing whether this process exists at all.
- Text follows the existing rules: English source strings with `@tr(...)` in
  the `.slint` file, Chinese in the bundled `.po`, and Rust-side messages in
  `i18n::Text` (log line, failure and "not responding" hints).

## Alternatives considered

- **Reject the second launch automatically (a real single-instance app).**
  Rejected: the request explicitly offers "start a new instance", and some
  operators run several panels with different roots/ports. Detection must ask,
  not forbid.
- **`CreateMutexW` / named mutex.** Rejected: needs Win32 calls (`unsafe` or a
  new dependency) and still does not solve raising the window.
- **Windows named pipe / Unix socket for the handshake.** Rejected for this
  slice: it adds a listening endpoint, ACL work and a new dependency for a
  three-message protocol, and it starts the IPC surface that ADR 0004/Phase 7
  deliberately postponed. Files in the already-selected state directory are
  enough and keep working in the portable `.tftpd-r` fallback.
- **Exclusive lock file (open without sharing on Windows, `flock` on Unix).**
  Rejected: the "New instance" choice requires two panels, and Rust's standard
  library has no portable, safe exclusive-open/lock API.
- **Blocking the UI thread while waiting for the acknowledgement.** Rejected:
  the UI timer polls instead, so the prompt stays responsive and the
  acknowledgement is handled by the normal tick.
- **A separate small prompt-only Slint window.** Rejected for now: it would
  duplicate the theme bootstrap, the translation selection and the window
  configuration for a single dialog; the overlay reuses all three.

## Consequences

- A second launch shows a three-way prompt and never silently starts a second
  server. "Show program window" costs one UI tick (~200 ms) plus the round trip; the
  measured end-to-end time is ~0.2 s.
- Up to 15 s after a crash a new launch still shows the prompt (the lock looks
  fresh). "New instance" still works, so the prompt is never a dead end.
- The state directory now contains transient `instance-*.lock`,
  `activate-request-*.req` and `activate-ack-*.ack` files. They hold a PID, a
  non-secret token and timestamps, never paths or server configuration, and
  stale ones are removed by later launches.
- The heartbeat, request scanning and ack polling are all best effort and never
  fail the panel. A failed write is reported on stderr only (there is no window
  in the `--activate-only` path).
- The startup handshake is local-only and per-user. It must not be extended into
  a server-control channel; that remains Phase 7 with its own ADR.

## Amendment (2026-11-24)

[ADR 0018](0018-tray-icon-and-close-choice.md) replaces the "prompt is the last
child of `AppWindow`" part of this decision: the prompt is now
`InstancePromptWindow`, a standalone small window, and the main panel is not
shown until the operator picks *New instance*. The advisory lock files,
heartbeat, request/ack handshake, deadline and "New instance" registration rules
are unchanged. A panel that was closed to the notification area is raised by the
same request/ack path; the owning process hides its tray icon before showing the
window.
