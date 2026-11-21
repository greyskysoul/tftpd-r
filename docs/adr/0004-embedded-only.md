# ADR 0004: Embedded-only GUI in this release

- **Status:** accepted (deviation from the original phased plan)
- **Date:** 2026-11-20

## Context

At the time this decision was made, the implementation plan called the embedded
GUI Phase 5 and local IPC Phase 6. The roadmap has since reordered the remaining
work: current Phase 6 is target-machine validation; the IPC scope in this ADR is
now current Phase 7. IPC still requires a versioned protocol, platform-specific
transport (named pipes / Unix sockets), authentication/ACLs, framing limits,
reconnect logic and a new crate for shared DTOs.

## Decision

Deliver Phase 5 only in this release: the Slint GUI embeds `tftpd-r-core` in its own
Tokio runtime (`EmbeddedController`). There is no IPC endpoint, no daemon
process control, and no `tftpd-r-control-protocol` crate yet. The daemon is a
separate, standalone front-end.

## Consequences

- The GUI is a fully functional local server panel without any IPC attack
  surface or platform transport code.
- The GUI cannot attach to an already-running daemon; closing the GUI stops its
  server. Operators who need a background service must run `tftpd-r`.
- The IPC work (current Phase 7; formerly Phase 6) remains open. When implemented,
  DTOs must live in a separate crate, the protocol must be versioned and
  length-prefixed, local clients must be authenticated (pipe ACLs / socket
  permissions), and commands must be split by privilege (read status vs
  start/stop vs change configuration).
