# Roadmap and planned requirements

This file records requirements that are **accepted but not implemented**. It is
not a statement about the current release: `0.1.0` ships TOML configuration, a
single `bind_address` defaulting to `127.0.0.1:6969`, and a GUI that loads
`config.toml` from the working directory.

Anything listed here is unimplemented until the code, tests, ADR and docs say
otherwise.

The configuration format stays **TOML**. An INI configuration with fixed
locations (`~/.tftpdr.ini`) was considered for `0.2.0` and then dropped; the
daemon keeps `--config <path>` with TOML and the GUI keeps loading
`config.toml` from the working directory.

## Implemented on main (unreleased, target `0.2.0`)

The two requirements below are implemented on `main`; they are kept here as the
accepted requirement record until `0.2.0` is released. See the linked ADRs for
the decisions and deviations.

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

## Next

- Phase 6 (local IPC and GUI-managed daemon) and Phase 7 (WRQ uploads) remain
  unimplemented. Their prerequisites and security notes are in
  [`AGENTS.md`](../AGENTS.md) §6.
