# ADR 0009: GUI first-run setup, exclusive wildcard selection and idle auto-restart

- **Status:** accepted
- **Date:** 2026-11-20

## Context

User feedback on the `0.2.0` desktop panel asked for four related interaction
changes:

1. The multi-select bind addresses accepted "全部网络 (0.0.0.0)" together with
   specific addresses, could be emptied, and started from the loopback default
   even though the panel is normally used for LAN sharing. The requested rules
   are: selecting "全部网络" removes the other addresses; selecting another
   address removes "全部网络"; unchecking "全部网络" re-checks it; the first
   start preselects "全部网络".
2. A fresh installation without `config.toml` opened on the main window; the
   operator had to discover 设置 before the first useful start. The first start
   should open the configuration window.
3. The recent-root history had no way to clear it.
4. After a configuration or root-directory change, a running server kept the old
   values until a manual restart, even when nothing was being transferred. The
   requested behaviour is an automatic restart when no transfer is in progress.

## Decision

### First-run setup

- The panel keeps a one-time `setup-shown` flag in `ui-state.txt` next to
  `recent-dirs.txt` in the per-user profile directory
  (`%APPDATA%\TftpdR\` on Windows, the XDG config directory elsewhere). The
  state is advisory and best effort, like the recent-root list.
- On startup, the settings dialog opens automatically when **no valid
  `config.toml`** was loaded and the flag is absent; the flag is then written so
  the dialog appears once per profile, not on every unconfigured launch. A
  valid `config.toml` means the operator is already configured and is not
  interrupted. `--smoke-test` and the screenshot previews do not consume the
  flag. A profile that cannot be written simply means the prompt reappears on
  the next launch; no server configuration depends on the flag.
- Without a valid `config.toml` the panel's form starts from
  `ServerConfig::default()` with `bind_addresses = ["0.0.0.0"]`, so the first
  start preselects "全部网络". This is a **GUI form default only**:
  `ServerConfig::default()` and the daemon still default to
  `127.0.0.1:69`, and the server never starts without an explicit 启动 from the
  operator. The panel does not write `config.toml`; persisted configuration
  remains a daemon/TOML responsibility.

### Address picker rules

- "全部网络 (0.0.0.0)" is exclusive and sticky:
  - selecting it clears every specific address;
  - selecting a specific address clears it;
  - clicking it while it is selected keeps it selected (and clears any other
    selected address, which can only appear in a hand-written configuration);
  - unchecking the last remaining specific address selects the wildcard again,
    so the bind list is never empty and `read_config` cannot produce the
    `NoBindAddresses` error through the picker.
- Only the IPv4 wildcard gets these rules. The picker does not offer `::`
  (ADR 0008); a `::` entry coming from a hand-written file stays visible and
  behaves like a normal address.

### Clear recent-root history

- The drop-down gets a 清空 button that clears the in-memory list and removes
  `recent-dirs.txt`. The configured/running root is server configuration, not
  history, and is unaffected. The button is disabled when the list is empty.

### Automatic restart after a change

- `UiState` remembers the `ServerConfig` of the running server. Saving the
  settings dialog or selecting another recent root compares the resulting
  configuration with it.
- When the controller reports `Starting` or `Running`, the configuration
  changed, and `state.transfers` is empty, the panel calls the existing
  asynchronous `EmbeddedController::restart`. This keeps the stop/start off the
  UI thread and reuses the queued-restart path (including failure reporting).
  The log and status bar follow the normal `Stopping → Running` transition.
- With at least one active transfer the new configuration is stored but the
  server is **not** restarted; the log warns that a manual restart is required.
  No restart is queued for when the transfer finishes: the requirement is
  conditional on the state at change time, and a later unattended restart could
  surprise an operator who is still using the server.
- A server that is `Stopped` or `Failed` only stores the new configuration; it
  is applied by the next 启动. A deliberate stop (`Stopping`) is never turned
  back into a start by a change.
- `EmbeddedController::poll_status` now reports `Stopping` instead of a stale
  `Running` after `stop()` took the handle. Without this fix an immediate
  configuration change could observe `Running` from the watch channel and
  start a server the operator had just stopped.

## Consequences

- The bind list and the "全部网络" row are no longer independent checkboxes;
  the picker enforces a single, non-empty selection set. Tests cover all toggle
  transitions.
- The first GUI start on a machine without `config.toml` now shows the settings
  dialog once; later starts with the same profile do not. Deleting
  `ui-state.txt` restores the first-run prompt. Because the panel does not write
  `config.toml`, a user who only uses the panel still gets its all-network
  default on the next start unless they create that file.
- The GUI's unconfigured default widens the exposed surface compared with the
  core default. This is deliberate for the LAN-sharing use case and is called
  out in the picker, README and security notes; read-only mode and the explicit
  启动 requirement remain in place. Operators who want loopback must deselect
  "全部网络" and choose "本地回环".
- Automatic restarts happen only for idle servers. A transfer-heavy deployment
  keeps the old configuration until the operator restarts; the WARN log says so.
- The clear button is destructive but only affects local history; it has no
  confirmation dialog, matching the log-view 清空 button.
