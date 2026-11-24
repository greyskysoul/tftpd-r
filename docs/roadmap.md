# Roadmap and planned requirements

This document separates shipped behaviour from work that is accepted but not yet
implemented. A phase is complete only when its code (where applicable), tests,
documentation, and acceptance evidence satisfy the stated Definition of Done.
A checklist item marked **not verified** must not be described as passed.

## Current baseline: `0.4.0`

`0.4.0` is a feature release on `0.3.1`. It adds the two-page settings dialog
with a persisted language choice and recent-root history, a duplicate-launch
prompt with window activation, a close-time choice between exiting and running
in the notification area, and a native system-tray icon. The panel can be
raised again from the tray, a later launch’s prompt, or `--activate-only`.
There is no TFTP protocol, TOML schema, read-only behaviour or
security-default change.

`0.3.1` was a packaging and executable-rename patch release on `0.3.0`. The CLI
daemon builds as `tftpd-r-server` and the desktop panel as `tftpd-r`; release
archives no longer include a standalone `.ico` file. Scripts and service
definitions that started the old `tftpd-r` daemon must use `tftpd-r-server`
instead, because `tftpd-r` now launches the GUI.

`0.3.0` was a feature release on `0.2.2`. In addition to the GUI workflow from
0.2.2 (remembered root, recent-root menu, exportable activity log, pre-warmed
folder picker and unified header), that release added bilingual English and
Simplified Chinese UI, follow-system/light/dark themes, and a smaller daemon
log-filter implementation. The protocol, TOML format, read-only behavior, and
server security defaults remain unchanged. Release gates and target-machine gaps
are recorded in [testing.md](testing.md).

### In 0.3.0: panel language and appearance (ADR 0015)

The panel is now bilingual and themed. It ships **English and Simplified
Chinese**, chosen once at startup from the persisted language preference
(default: the system locale; Simplified Chinese locales show Chinese; every
other language, including Traditional Chinese, shows English). Static text is
translated through bundled Slint `@tr()`/`.po` catalogues and the Rust-built text
(log lines, validation errors, native dialog titles) uses the same language.
Settings → Appearance → Theme offers **Follow system** (default), **Light**
and **Dark**; the system value is read from the operating system and re-checked
every 30 s while that mode is active, and the choice is remembered in
`ui-prefs.txt`. The window is English-by-default in source form, so the
`.slint` files no longer contain Chinese literals. No protocol,
configuration-format, daemon or security-default change.

### In 0.4.0: settings navigation, language choice and root history (ADR 0015 revision)

The settings dialog is split by a left navigation bar into **Server** (port,
bind addresses, root, block size, timeout, sessions; validated like the daemon,
with Cancel/Save and the idle-server restart rule) and **Appearance** (theme and
language; applied independently, with a single Close button). The language
choice is **Follow system / English / Simplified Chinese**, stored as
`language=system|en|zh-CN` in `ui-prefs.txt`; the running panel keeps its startup
language and applies the new preference on the next start, so existing log lines
and native dialog titles stay consistent. The server root field now has a
history drop-down backed by `recent-dirs.txt`; selecting an entry only fills the
form and keeps the inline "save the settings" reminder. The page actions are
part of each page’s content (Cancel/Save on Server, Close on Appearance), so
they no longer sit below a card-wide divider that changes with the page. No
protocol, configuration-format, daemon or security-default change. The GNU x64
GUI grows from 9,036,800 B (0.3.1) to 9,310,720 B (0.4.0), including the
duplicate-launch/tray work described below.

### In 0.4.0: duplicate-launch prompt, tray icon and close choice (ADR 0017/0018)

A second launch detects the running panel through advisory files in the per-user
state directory and asks whether to show the existing window, start another
instance or cancel; the prompt is now a standalone small window rather than a
full-panel overlay. Closing the window while the embedded server is
`Starting`/`Running` offers **Close and exit / Run in background / Cancel**.
Background mode hides the panel and shows a notification-area icon
(Slint `SystemTrayIcon`: `Shell_NotifyIcon` on Windows, `ksni`
StatusNotifierItem on Linux/BSD, AppKit on macOS); the tray menu or left-click,
a later launch's *Show program window*, and `--activate-only` all raise the
hidden window. Linux/BSD tray support depends on a StatusNotifierItem host and
has not been run on a Linux desktop; without a visible icon the last-window rule
exits rather than leaving an unreachable background process. The handshake
remains a local UI convenience, not a server-control IPC channel. No protocol,
configuration-format, bind-default or upload change. The GNU x64 GUI grows from
9,036,800 B (0.3.1) to 9,310,720 B (0.4.0, +3.0 %); the Windows
tray/close/settings flow was manually verified by the operator.

### In 0.3.0: daemon log filter size (ADR 0016)

The daemon no longer links `tracing-subscriber`'s `env-filter` (and with it the
`regex` engine) or `tracing-log`; `logging.rs` parses the level-based filter
subset itself (a level, `target=level`, or a bare target) into a `Targets`
filter. The GNU x64 daemon drops from 1,357,824 B to 1,063,936 B (−21.6 %).
Rendered log lines keep their format; a target-only filter keeps the `info`
default for other targets, and invalid directives are reported and ignored. The
GUI, protocol and configuration format are unchanged.

`0.2.2` was the preceding feature release: remembered root selection, activity
log export/filtering, and the pre-warmed folder picker were introduced there.

`0.2.1` was a patch release on `0.2.0`: both Windows executables embed the
product icon as a PE resource ([ADR 0011](adr/0011-windows-executable-icon.md)).

`0.2.0` ships TOML configuration (`port` + `bind_addresses`, defaulting to
`127.0.0.1:69`), the read-only RRQ server, a standalone daemon, and the embedded
Slint panel. The 0.2.0 CI/release workflows completed for Windows/Linux x64 and
native arm64 builds. The latest recorded local verification is 219 passing tests
on Windows 11 with Rust 1.100.0 / GNU; see [testing.md](testing.md) for the exact
commands, CI evidence, manual checks, and gaps.

The configuration format remains **TOML**. An INI configuration at a fixed path
was considered and dropped; do not reintroduce it. The GUI loads `config.toml`
from its working directory and does not write that file.

### Implemented in `0.2.0`

- **R1 — Default port 69.** Daemon/core default to loopback port 69. Binding
  errors provide an actionable, non-forcing hint; there is no automatic
  fallback, elevation, or config rewrite. See
  [ADR 0007](adr/0007-default-port-69.md).
- **R2 — Multiple bind addresses.** `port` is shared by up to 16 bind addresses;
  duplicates and same-family wildcard overlaps are normalised; binding is
  all-or-nothing. GUI picker and directory picker are implemented. See
  [ADR 0008](adr/0008-multiple-bind-addresses.md).
- **GUI first-run and idle restart.** First-run setup, the exclusive/sticky
  all-network picker option, recent-root clearing, and automatic restart only
  when settings change while the server is idle are implemented. See
  [ADR 0009](adr/0009-gui-first-run-and-auto-restart.md).
- **R3 (INI) was withdrawn.** Configuration remains TOML.

### Feature and patch releases

- **0.4.0 — desktop workflow release.** Two-page settings with persisted
  language choice and recent-root history; duplicate-launch detection with
  activation; close choice, background mode and notification-area icon. No
  protocol, configuration or security-default change. See
  [ADR 0017](adr/0017-single-instance-prompt.md) and
  [ADR 0018](adr/0018-tray-icon-and-close-choice.md).
- **0.3.1 — executable rename and archive cleanup.** The CLI daemon builds as
  `tftpd-r-server` and the GUI as `tftpd-r`; release archives contain no
  standalone `.ico`. No protocol, configuration or security-default change.
  Existing scripts must switch from `tftpd-r` to `tftpd-r-server` for the
  daemon.
- **0.3.0 — UI language/theme and daemon size.** English + Simplified Chinese
  selected from the system locale at startup; follow-system/light/dark appearance;
  level-based daemon filter that removes `env-filter` and the `regex` engine
  (GNU x64 daemon 1,357,824 → 1,063,936 B; MSVC x64 daemon 1,193,984 →
  911,872 B). No protocol, configuration or security-default change. See
  [ADR 0015](adr/0015-gui-language-and-theme.md) and
  [ADR 0016](adr/0016-log-filter-without-env-filter.md).
- **0.2.2 — GUI workflow release.** Remembered root directory and recent-root
  menu, activity log with an exact-level drop-down filter and explicit export,
  pre-warmed native folder picker, and one continuous title-bar/toolbar header.
  No protocol or configuration change. See
  [ADR 0012](adr/0012-gui-root-selection-and-memory.md),
  [ADR 0013](adr/0013-gui-activity-log.md) and
  [ADR 0014](adr/0014-prewarmed-folder-picker-helper.md).
- **0.2.1 — Windows executable icon.** `tftpd-r.exe` and `tftpd-r-gui.exe` embed
  `app-icon.ico` as an `RT_GROUP_ICON`/`RT_ICON` resource, generated at build
  time by `tftpd-r-build-support` (MSVC links the `.res` directly; GNU converts
  it with `windres`). No configuration or protocol change. See
  [ADR 0011](adr/0011-windows-executable-icon.md).

## Revised phase order

The remaining work is reordered to reduce risk: validate and harden the already
shipped read-only product before adding a new local control surface, then
consider a write path last.

| New phase | Scope | Status |
| --- | --- | --- |
| **Phase 6** | Target-machine validation, test-gap closure, and release/deployment readiness for the existing read-only product (previously Phase 8) | 🟡 Partially complete; acceptance work remains |
| **Phase 7** | Local IPC and GUI control of a separately running daemon (previously Phase 6) | ⏳ Not started |
| **Phase 8** | Optional WRQ uploads (previously Phase 7) | ⏳ Not started; read-only remains the only supported mode |

The phase renumbering changes future execution order, not release history. ADRs
0004 and 0005 describe decisions made under the former numbering; their current
phase mapping is noted in those records. Windows Service support remains a
separate, unnumbered follow-up and must not be mixed into foreground daemon
lifecycle work.

## Phase 6 — Validate and harden the shipped product

**Goal:** establish reproducible target-machine evidence for the current 0.2.0
read-only product before expanding its attack surface. This phase is primarily
verification and defect correction, not a feature phase.

### Work items

1. **Run and record the build/test matrix.** On a Windows MSVC target, run fmt,
   clippy, the full test suite, and a release build. On Linux, run core/daemon
   tests and launch the GUI on a supported desktop session. On arm64, run the
   available artifacts when real target hardware/VMs are available; compilation
   alone is not runtime verification. Record OS, architecture, toolchain, exact
   commands, exit codes, and artifact identity.
2. **TFTP interoperability.** Test at least one independent client and, where
   available, a legacy client that requests `get /file`. Record client/version,
   server OS, mode/options, file size/hash, and outcome. Cover a normal transfer,
   an exact block-size multiple, a missing file, and a rejected traversal name.
3. **Network/deployment behaviour.** Test loopback and an intentionally selected
   LAN interface, firewall treatment of UDP/69 and transfer TID ports, a client
   on another host, and port-69 failure behaviour on a platform where the
   permission path can actually be exercised. Do not infer Windows behaviour
   from Linux or vice versa; do not test by exposing the server to the public
   Internet.
4. **GUI acceptance.** On a real desktop, verify first-run setup, normal typing
   and Chinese IME input, directory picker, address discovery and selection,
   start/stop/restart, real mouse title-bar/resize interaction, active-transfer
   shutdown, Chinese font rendering, and at least one non-default DPI. Test RDP
   only if it is within the intended support matrix. Run Linux GUI checks only
   with the required graphical session.
5. **Package and dependency review.** Download each published archive and verify
   its checksum, inspect contents, run the included binaries where the target is
   available, inspect runtime dependencies, and perform a dependency/license
   review. Record results and limits rather than extrapolating from CI builds.
6. **Test-suite strengthening.** Keep existing deterministic table-driven and
   loopback tests as the required baseline. Add fuzz/property tests for codec
   inputs only after selecting and reviewing the tool/dependency; add a bounded
   event-flood/UI responsiveness test where it can be deterministic; avoid flaky
   random sleeps. Track any test not yet automated in the manual matrix.
7. **Documentation and packaging.** Publish a support matrix and concise
   install/upgrade/rollback instructions. Correct platform-specific port-69
   guidance and distinguish “built” from “executed”. The Windows executable
   icon resource is embedded at build time for both binaries (ADR 0011);
   confirming the MSVC and arm64 resource paths on target machines remains part
   of this phase.

### Definition of Done

- The target matrix has reproducible evidence, with every unsupported/unavailable
  target explicitly marked unverified rather than inferred as passing.
- Independent-client transfer results include hashes and the legacy `/file`
  case, or a concrete blocker and a follow-up owner/decision.
- GUI, LAN/firewall, port-69, packaging, dependency and license checks are
  recorded in [testing.md](testing.md); found defects are fixed and the relevant
  regression tests are added.
- Support and deployment documentation matches measured evidence; no claim is
  made for untested OSes, architectures, clients, DPI, IME, or runtime DLLs.
- Required workspace checks pass on the edited state and their actual results are
  recorded. No upload, IPC endpoint, service registration, privilege escalation,
  or public-listener default is introduced in this phase.

## Phase 7 — Local IPC and GUI daemon control

**Goal:** let the GUI observe and control a separately running foreground daemon
without coupling the protocol core to IPC or assuming that “local” means trusted.

### Design gate — before implementation

1. Write/update an ADR that fixes the daemon/GUI ownership model, endpoint
   lifecycle, supported platforms, authentication boundary, threat model, and
   failure/reconnect semantics. The first increment should attach to an already
   running daemon; launching or supervising a daemon process from the GUI is a
   separate decision and must not be assumed implicitly.
2. Add a dedicated `tftpd-r-control-protocol` crate for versioned DTOs and
   transport-independent validation. Define a length-prefixed frame, a strict
   maximum frame size, request IDs, protocol version negotiation, unknown-field
   policy, error codes, and bounded response/event payloads. Choose concrete
   limits in the ADR and test their exact boundary values.
3. Select Windows named-pipe and Unix-domain-socket implementations only after
   reviewing maintained crates/platform APIs, licenses, MSRV, and ACL behavior.
   Restrict endpoints to local clients; configure Windows pipe ACLs and reject
   remote clients where the API permits; use user-only Unix socket permissions
   and safe handling of stale socket paths. No shell command or arbitrary
   filesystem path may be controlled by a request.
4. Define least-privilege operations. Initial scope: query status, subscribe to
   bounded events, and start/stop the daemon's already loaded configuration.
   Do not let IPC change `root_dir`, enable uploads, alter overwrite policy,
   elevate privileges, or expose arbitrary logs/files. Any later config mutation
   needs explicit authorization, validation, auditability, restart semantics, and
   a separate decision.

### Implementation slices

1. Protocol crate: DTOs, framing encoder/decoder, version/error handling, input
   limits, and pure unit tests; no sockets yet.
2. Transport: one platform at a time behind a small server/client boundary;
   enforce peer permissions before accepting commands; cap clients, in-flight
   requests, event subscribers, and per-client request rate.
3. Daemon integration: opt-in local endpoint with deterministic shutdown and
   cleanup. IPC failure must not leave the UDP server in a misleading state.
4. GUI integration: an IPC controller alongside the embedded controller; clear
   connection/auth/version errors; bounded reconnect/backoff and event
   resubscription; no IPC work or blocking wait on the Slint UI thread.
5. Compatibility: document protocol evolution and reject unsupported major
   versions safely. Do not let the GUI silently fall back to a different daemon
   or change server settings on connection failure.

### Required tests and Definition of Done

- Unit tests: framing boundaries, truncation, oversized/zero lengths, invalid
  versions, unknown messages/fields, request-ID correlation, malformed DTOs,
  and bounded event delivery.
- Transport/security tests: unauthorized identity/ACL, remote-client rejection,
  stale endpoint handling, connection and request caps, rate limits, disconnect
  during a response, reconnect, and clean shutdown.
- Contract tests: status/start/stop behaviour, duplicate/concurrent commands,
  GUI state after daemon restart, unsupported protocol version, and no stale
  `Running` state. Mock the transport for GUI/controller tests.
- CI: run the relevant protocol tests on all supported OSes; compile/test each
  implemented transport on its native runner. Security-negative tests must run
  in CI where the platform supports them.
- ADR, architecture, security, README, roadmap, and testing docs agree on the
  actual command set and platform support. No access-control claim is made based
  only on a successful loopback connection.

## Phase 8 — Optional WRQ uploads

**Goal:** add uploads only if a product requirement justifies the additional
write-capable attack surface. Until this phase is complete, `allow_upload = true`
and `overwrite = true` remain configuration errors and WRQ remains rejected.

### Design and implementation slices

1. **Security design first.** Revisit [ADR 0005](adr/0005-uploads-disabled.md)
   and [ADR 0003](adr/0003-path-sandbox.md). Define opt-in configuration,
   overwrite default/semantics, size and duration limits, disk-space/error
   handling, permissions, cancellation, crash recovery, and platform-specific
   atomic-commit guarantees. Identify TOCTOU assumptions and choose a safe
   create-within-root approach; do not assume the current read-only `open` path
   is sufficient for writes.
2. **Temporary-file transaction.** Create an unpredictable, exclusive temporary
   file in the destination directory; never follow an existing symlink/reparse
   point; enforce limits while receiving; flush/close and atomically commit only
   after the final valid DATA/ACK sequence. On timeout, malformed traffic,
   cancellation, disk-full, or process-visible failure, remove the temporary
   file and preserve any pre-existing target. If a platform cannot guarantee the
   documented atomic semantics, fail closed and document it.
3. **WRQ state machine.** Implement ACK 0/options, DATA block ordering and
   duplicate/retransmitted DATA handling, block-number wrap, final-block rules,
   peer/TID checks, retry/deadline enforcement, and sanitized client errors.
   `tsize` and other supported options must have explicit upload-side validation.
4. **Configuration/UI.** Keep uploads disabled by default and make opt-in
   unmistakable. Overwrite remains disabled unless separately designed and
   explicitly selected. `validate()` is the single rule source; GUI and daemon
   must reject the same invalid settings. Never let IPC silently enable upload.
5. **Abuse/resource controls.** Enforce per-upload byte limits while streaming,
   session/concurrency limits, timeouts, bounded buffers, disk-full handling,
   and temporary-file cleanup. Do not trust a client-declared `tsize` as the only
   size control.

### Required tests and Definition of Done

- Path tests: traversal, legacy leading slash mapping, ADS/device names,
  symlink/reparse races where testable, existing-file overwrite policy,
  destination-directory changes, and temp-file naming/collision attacks.
- Protocol tests: WRQ negotiation, valid and invalid block order, duplicate DATA,
  packet loss/retransmission, exact-multiple zero-length terminal block,
  wraparound, wrong TID, retry exhaustion, timeout, cancellation, and sanitized
  errors.
- Failure-injection tests: disk full/write/flush/rename errors, interrupted
  transfer, server shutdown, and cleanup with both new and pre-existing targets.
- Real-client interoperability on each supported platform; record client/version,
  configuration, final file hash, overwrite outcome, and cleanup outcome.
- Security review and updated ADR accepted before enabling the feature. The
  default configuration and tests must prove read-only remains the default; a
  negative test must prove uploads cannot happen unless explicitly enabled.
- Update architecture, security, README, config example, testing and release
  notes together. Do not claim the write path is race-free unless the underlying
  platform mechanism and tests support that claim.

## Separate future work: Windows Service

Windows Service registration, recovery policy, service account/ACL, upgrade and
uninstall are not part of Phases 6–8. Keep them separate from the foreground
daemon, require an explicit design/review, and never auto-register or elevate.

## References

- [Architecture](architecture.md)
- [Security and residual risks](security.md)
- [Executed test and acceptance record](testing.md)
- [ADR index](adr/)
- [Developer invariants and workflow](../AGENTS.md)

> The original implementation plan is a local, ignored planning file. It may be
> revised for execution, but must not be added to Git or copied into tracked
> documents; the tracked roadmap above is the public, concise source of current
> phase order and acceptance criteria.

## Historical Phase 6/7/8 mapping

Before this roadmap revision, Phase 6 meant IPC, Phase 7 meant WRQ uploads, and
Phase 8 meant target-machine validation/release preparation. The new order is
intentional: **validate/harden → IPC → optional uploads**. Historical ADR and
verification entries may refer to the former labels; follow this mapping when
reading them.

| Former phase | New phase | Scope |
| --- | --- | --- |
| Phase 8 | Phase 6 | Target-machine validation and release readiness |
| Phase 6 | Phase 7 | Local IPC and GUI daemon control |
| Phase 7 | Phase 8 | Optional WRQ uploads |
