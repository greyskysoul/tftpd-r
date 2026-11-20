# ADR 0007: Default bind port 69, with an actionable privilege hint

- **Status:** accepted
- **Date:** 2026-11-20
- **Security review:** recorded in [`security.md`](../security.md)

## Context

`0.1.0` defaults to the unprivileged port `6969` so a checkout runs without
administrator/root rights. The standard TFTP port is **69**, and clients and
firewall guidance assume it. Requirement R1 in [`roadmap.md`](../roadmap.md)
asks for port 69 as the default *and* for a helpful message when the process
lacks the privileges to bind it.

Port 69 is a low port: Linux requires root or `CAP_NET_BIND_SERVICE`, while
Windows normally lets a normal user bind it unless the port is reserved or
excluded by the system. Silently falling back to another port would hide a
deployment mistake and change behaviour behind the operator's back;
auto-elevating would violate the project's "never escalate" rule.

## Decision

1. The default bind address becomes **`127.0.0.1:69`** (loopback only, standard
   TFTP port). Loopback remains the default; LAN exposure is still opt-in.
2. A bind failure is classified in `tftpd-r-core`: a permission error on a
   privileged port (`1..=1023`) becomes
   `ServerError::PrivilegedPort`. Its message names the address, states that
   administrator/root is required, suggests an unprivileged port such as
   `6969`, and explicitly says the configuration is not changed automatically.
3. The GUI shows the message in the status bar and activity log; the daemon
   prints it and exits non-zero. Neither retries, falls back, elevates nor
   rewrites the configuration.
4. The classification checks `io::ErrorKind::PermissionDenied` plus the raw
   `EACCES`/`WSAEACCES` codes instead of matching error text.

## Consequences

- On a platform that restricts low ports (Linux without `CAP_NET_BIND_SERVICE`
  or a reserved Windows port), a first run now fails visibly by default. That
  is the intended trade-off: the standard port is the product default, the
  failure is actionable, and an unprivileged trial run uses
  `--bind 127.0.0.1:6969` (daemon) or changes the port in the GUI settings.
- The example configuration, README quick start, security defaults and the
  daemon config tests change together.
- Tests cannot bind port 69 (that would require privileges and would conflict
  with a real service); the classification is unit-tested, while integration
  tests keep using port `0`.
- A future IPC/service wrapper must not treat this hint as permission to
  elevate. The daemon remains a foreground process.
