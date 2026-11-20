# ADR 0008: Multiple bind addresses for one server instance

- **Status:** accepted
- **Date:** 2026-11-20

## Context

`0.1.0` has one `bind_address: SocketAddr`. Requirement R2 in
[`roadmap.md`](../roadmap.md) asks for a multi-select address list in the GUI,
populated from the machine's interfaces, with two permanent entries
("全部网络 (0.0.0.0)", "本地回环 (127.0.0.1)"). This is a server change: the
accept loop, session sockets, status reporting, configuration and CLI all
assumed exactly one listener.

## Decision

### Configuration model

- `ServerConfig` replaces `bind_address` with:
  - `port: u16` — one port shared by every listener;
  - `bind_addresses: Vec<IpAddr>` — one or more interface addresses.
- Default: `port = 69`, `bind_addresses = ["127.0.0.1"]` (ADR 0007 explains the
  privileged default and the non-forcing hint).
- Validation: the list must be non-empty and at most 16 entries; multicast,
  broadcast and unspecified-looking invalid forms are rejected; `0.0.0.0` and
  `::` are accepted as wildcards. Exact duplicates are de-duplicated and a
  wildcard collapses the other addresses of its family in
  `effective_bind_addresses()` — accepted and normalised rather than rejected.
- The old `bind_address` key is gone. Because the config uses
  `deny_unknown_fields`, an older file fails loudly with an "unknown field"
  error instead of silently binding the wrong address. `config.example.toml`
  documents the new shape.
- The daemon `--bind <ADDR>` override replaces the whole list with that one
  address and port; there is no partial override.

### Binding semantics

- Binding is **all-or-nothing**: the server validates, then binds every
  effective address; if one fails, the sockets bound so far are dropped and
  `start` returns the classified error (including the privileged-port hint of
  ADR 0007). A start never leaves a listener serving a subset of the configured
  addresses. Partial success was rejected because it silently changes the
  exposed surface and complicates the status model; a missing interface is a
  configuration error the operator can see and fix by deselecting it.
- All listeners share the port. When `port = 0` (tests and temporary
  instances), the first successful bind resolves the ephemeral port and the
  remaining listeners reuse it, so the instance still has one consistent port.
- Every listener owns an accept loop. The peer map (duplicate-RRQ routing),
  the per-peer session cap, the global session semaphore and the session-id
  counter are shared, so a retried request on another address still reaches the
  existing transfer and the limits are global.
- A session socket binds the **specific** address of the listener it arrived on
  (port 0). A multi-homed client that contacted `192.168.1.10:69` therefore
  receives DATA from `192.168.1.10:<ephemeral>` instead of an unrelated local
  address chosen by the routing table. Wildcard listeners keep binding the
  unspecified address and leave source-address selection to the OS.
- `ServerStatus::Running` and `ServerHandle` expose `local_addrs()` (a list)
  instead of a single address.

### GUI

- The address field opens a multi-select picker (drop-down style panel) that
  always contains the two mandated entries and the addresses discovered on this
  machine; selected addresses are never dropped from the list even if discovery
  does not report them (for example a hand-written IPv6 entry).
- Discovery is a local, best-effort operation. The workspace has no native
  interface-enumeration dependency and hand-written FFI is forbidden, so the
  GUI asks the platform runtime for the list: on Windows, Windows PowerShell
  5.1 running the .NET `NetworkInterface` API (no WMI/CIM access, no
  administrator rights); on Unix, `hostname -I`. Only IP literals are parsed,
  the helper runs with a fixed absolute path without a shell, is killed after
  5 s, and a failure simply leaves the two special entries.
- Loopback and link-local addresses are filtered from the discovered list
  (link-local IPv6 cannot be represented without a scope id). The two special
  entries cover loopback and the IPv4 wildcard. `::` is not offered as a GUI
  entry because its dual-stack behaviour differs between Windows and Linux, but
  it is accepted in a hand-written TOML file.
- The settings dialog also gets a **浏览…** button for the root directory. It
  uses the same helper-process approach (`FolderBrowserDialog` on Windows,
  `zenity`/`kdialog` on Unix) on a background thread and validates that the
  result is an existing directory.

### Security review notes

- The helpers execute fixed programs with fixed arguments. No shell is
  involved, the initial directory is passed through an environment variable (so
  a path cannot become script text), the discovered/selected values are parsed
  as `IpAddr`/validated as existing directories, and helpers are given a
  timeout (discovery) or run only while the user waits for a dialog.
- Helper processes are created with `CREATE_NO_WINDOW`; the GUI itself is a
  windowed-subsystem binary, so no console window is allocated.
- No new dependency, no `unsafe` code and no elevated helper is introduced.

## Consequences

- Breaking configuration change for `0.2.0`; README, security notes, example
  file, daemon summary output and tests change together.
- Up to 16 listeners and accept loops; each listener drains its own sessions in
  parallel within the shared `shutdown_grace_secs`.
- The GUI can only add addresses that discovery reports (plus hand-edited
  entries that stay visible). A VPN interface that appears later is picked up
  after a restart.
- IPv6 wildcard semantics remain platform-dependent and are documented as a
  limitation rather than normalised with socket options.
