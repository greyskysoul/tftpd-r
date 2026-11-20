# Roadmap and planned requirements

This file records what shipped in the current release and the requirements that
are **accepted but not implemented**. `0.2.0` ships TOML configuration with
`port` + `bind_addresses` defaulting to `127.0.0.1:69`, a GUI that loads
`config.toml` from the working directory, and the desktop-panel interaction
changes linked below. `0.1.0` used a single `bind_address` defaulting to
`127.0.0.1:6969`.

Anything in the "Next" section is unimplemented until the code, tests, ADR and
docs say otherwise.

The configuration format stays **TOML**. An INI configuration with fixed
locations (`~/.tftpdr.ini`) was considered for `0.2.0` and then dropped; the
daemon keeps `--config <path>` with TOML and the GUI keeps loading
`config.toml` from the working directory.

## Implemented in `0.2.0`

The requirements below shipped in
[`v0.2.0`](https://github.com/greyskysoul/tftpd-r/releases/tag/v0.2.0); they are
kept here as the accepted requirement record. See the linked ADRs for the
decisions and deviations.

### R1 — Default bind port 69, with a non-forcing hint — implemented

- The default port is **69**, the standard TFTP port.
- A permission failure on a privileged port produces an actionable message that
  suggests an unprivileged port (for example `6969`); the server never falls
  back, never elevates and never rewrites the configuration.
- ADR: [`adr/0007-default-port-69.md`](adr/0007-default-port-69.md).
- Tests: bind-failure classification unit tests; the configuration and example
  files assert port 69. Port 69 itself is not bound in tests (privileges and
  port conflicts).

### R2 — Multi-select bind addresses — implemented

- `ServerConfig` has `port` plus `bind_addresses: Vec<IpAddr>`; the GUI offers a
  multi-select picker with "全部网络 (0.0.0.0)", "本地回环 (127.0.0.1)" and the
  addresses discovered on the machine.
- The server runs one accept loop per address, shares the peer/session limits
  across listeners and binds all addresses or fails the start as a whole.
- ADR: [`adr/0008-multiple-bind-addresses.md`](adr/0008-multiple-bind-addresses.md).
- Tests: configuration normalisation/validation unit tests, UDP integration
  tests for multiple listeners, one shared resolved port, wildcard collapsing
  and an unbindable address aborting the start; GUI unit tests for the picker
  model and summaries.

### GUI first-run setup, clearable history and idle auto-restart — implemented

Additional interaction requirements accepted after the R2 review:

- The first start without a valid `config.toml` opens the settings dialog once
  per user profile; the unconfigured form preselects "全部网络 (0.0.0.0)" while
  the daemon and core defaults stay loopback.
- "全部网络" is exclusive in the bind picker, cannot be unchecked and is the
  fallback when the last specific address is removed, so the bind list is never
  empty.
- The recent-root drop-down has a 清空 button.
- Saving settings or changing the root while the server runs restarts it
  automatically when no transfer is active; with active transfers the change is
  only stored and the log asks for a manual restart.
- ADR: [`adr/0009-gui-first-run-and-auto-restart.md`](adr/0009-gui-first-run-and-auto-restart.md).
- Tests: address-toggle transitions, first-run defaults, setup-state
  persistence, recent-history clearing and the idle/deferred restart decision
  (including a real embedded-server restart) in the GUI crate.

## Next

- Phase 6 (local IPC and GUI-managed daemon) and Phase 7 (WRQ uploads) remain
  unimplemented. Their prerequisites and security notes are in
  [`AGENTS.md`](../AGENTS.md) §6.
