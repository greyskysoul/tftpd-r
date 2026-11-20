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

## R1 — Default bind port 69, with a non-forcing hint

- The default bind port becomes **69**, the standard TFTP port (currently
  `6969`, because 69 needs elevated privileges).
- Port 69 is privileged: on Windows it requires an administrator token, on
  Linux a root-owned process or `CAP_NET_BIND_SERVICE`. When binding fails for
  that reason, the GUI and the daemon must tell the user that the port needs
  privileges and suggest an unprivileged port (for example `6969`).
- The hint must **not** be forcing: never silently fall back to another port,
  never auto-elevate, never rewrite the saved configuration. A failed start has
  to stay a visible failure, with the suggestion attached to it.

Implementation notes:

- Distinguish "privileged port, no permission" from other bind failures instead
  of matching on error text; the current error mapping in `tftpd-r-core` is
  coarse. `docs/security.md` must state that the server is never started with
  elevated rights by the application itself.
- Changing the default port changes a documented security-relevant default, so
  it needs a new ADR and a security review entry (see `AGENTS.md` §3.1/§3.3).
- Tests: bind failure on a privileged port maps to the new hint; an
  unprivileged port still binds; the configuration itself is not mutated.

## R2 — Multi-select bind addresses

- The GUI replaces the single `bind_address` text field with a **multi-select
  drop-down**.
- The list is populated automatically from the local network interfaces (all IP
  addresses found on the machine).
- Two special entries always appear:
  - **全部网络 (0.0.0.0)** — bind every interface;
  - **本地回环 (127.0.0.1)** — loopback only.
- The user may select **one or more** entries; the server listens on exactly the
  selected set.

Implementation notes:

- This is a protocol/server change, not only a UI change: `ServerHandle` and the
  accept loop currently assume a single socket. Multiple listeners need their
  own shutdown, per-listener bind reporting (one listener may fail while others
  succeed) and an aggregate status for the toolbar chip.
- The configuration field has to become a list, and the semantics must be
  documented for: `0.0.0.0` combined with a specific address (redundant or an
  error), an IPv6 equivalent of both special entries, duplicate/overlapping
  addresses, and an interface address that disappeared between configuration
  and start.
- Limits are required as everywhere else in this codebase: a bound on the number
  of selected addresses and on the total number of open listeners.

Open questions to settle before implementation:

- Is `0.0.0.0` mutually exclusive with the specific addresses, or accepted and
  de-duplicated?
- Are both special entries also offered for IPv6 (`::`, `::1`)?
- What does the GUI show when only some of the selected addresses bound
  successfully (partial success)?


