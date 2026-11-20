# Security

This document covers the MVP threat model, implemented controls, and residual
risks. It is intentionally specific about what is **not** protected.

## Assumptions and deployment guidance

- TFTP has **no authentication or encryption**. Anyone who can send UDP packets
  to the listening port and reach the export tree can download any file the
  server account can read under the root. Treat the exported directory as public
  to the allowed network.
- Defaults are conservative: loopback binding, read-only, uploads refused,
  modest concurrency, 64 MiB file limit. The **daemon and `ServerConfig`
  defaults** listen on `127.0.0.1:69`. The desktop panel's first-run form
  preselects "全部网络 (0.0.0.0)" when no valid `config.toml` exists, because the
  panel is normally used for LAN sharing; it still only starts after the
  operator presses 启动 and the picker labels the wildcard explicitly. Use the
  loopback entry (or a concrete interface address) for local-only use.
- Bind to loopback unless a LAN deployment is intentional. For LAN use, restrict
  the firewall to the client subnet, run the daemon under a dedicated
  low-privilege account and export a dedicated directory containing only files
  that may be public.
- Do not expose the service directly to the Internet. The protocol has no
  anti-spoofing or authentication; request spoofing and response amplification
  are inherent to UDP/TFTP.
- The default daemon listen address is `127.0.0.1:69`. Linux requires root or
  `CAP_NET_BIND_SERVICE` for port 69; Windows normally allows a normal user
  unless the port is reserved. The application never elevates itself, never
  retries on another port and never rewrites the configuration: a permission
  failure is reported with a hint to choose an unprivileged port such as `6969`.
  Running the service with elevated rights is an explicit operator decision and
  widens the impact of any future vulnerability; preferring a high port or a
  port-forwarding/firewall rule is safer for long-running deployments.

## Threat model

| Threat | Relevant controls |
| --- | --- |
| Unauthorised LAN client downloading files | Read-only by default; sandbox limits to the root; operator must opt into LAN binding; firewall guidance. No authentication exists. |
| Malicious/malformed UDP datagrams | Strict codec with length/field/UTF-8 checks; unknown opcodes and malformed requests answered with ERROR 4; oversized requests rejected; unknown options ignored, malformed known options fail negotiation. |
| Resource exhaustion (sessions, ports, memory, retransmissions) | Global `max_sessions` semaphore; 8-session per-IP cap; bounded request size; bounded retransmissions; hard per-transfer deadline; file size limit; bounded event ring; duplicate-request dedup avoids task/port churn. |
| Directory traversal / symlink escape | Component validation (`..`, drive letters, device names, ADS via `:`), per-component reparse-point rejection, canonical path check after open, component-wise prefix comparison (avoids `C:\root2` confusion). A leading `/`/`\` is mapped to the root for legacy clients and cannot skip these checks. |
| Silent data corruption | Exact-byte RRQ tests, ACK-block validation, retransmission of the exact encoded packet, idempotent duplicate handling, zero-length final block test, block wrap test. |
| Overwriting or uploading data | WRQ rejected (ERROR 2) and `allow_upload = true` rejected at validation. No write path exists. |
| Local IPC abuse | No IPC endpoint exists in this release. A future IPC must implement local authentication/ACL, framing limits and least-privilege commands. |
| Log injection / sensitive data leakage | Client file names are sanitised for control characters and length before events/logs; ERROR text is fixed and path-free; detailed OS errors are not sent to clients. |
| Configuration mistakes | `deny_unknown_fields`, range validation, root existence check, explicit rejection of uploads/overwrite, loopback default in core/daemon, bind list bounded to 16 unicast addresses and bound all-or-nothing. |
| Multiple listen addresses widening exposure | Only the configured addresses are bound; `0.0.0.0`/`::` are explicit wildcards, other same-family entries are collapsed rather than duplicated. A single unbindable address aborts the start instead of serving a silently reduced set. |
| Automatic restart after a setting/root change | Only when the server is `Starting`/`Running` and no transfer is active; active downloads are never aborted. A change with active transfers is stored for a later manual restart. A deliberate stop (`Stopping`) is never turned back into a start. |
| Helper processes used by the GUI | Fixed absolute programs, fixed arguments, no shell; the initial directory is passed via an environment variable and validated; discovery has a 5 s timeout; the GUI never requests elevation. |

## Filesystem sandbox

Implemented in `crates/tftpd-r-core/src/filesystem.rs`:

1. **Name validation** before touching the filesystem:
   - empty/overlong names, NUL and control characters rejected;
   - a leading `/` or `\` is stripped and interpreted as the sandbox root for
     compatibility with old clients (`/firmware.bin` → `<root>/firmware.bin`);
     a name consisting only of separators (the root directory itself) is
     rejected because TFTP cannot transfer a directory;
   - after that mapping, drive letters and any `:` are rejected (also blocks
     NTFS alternate data streams); a UNC-like name such as `\\server\share` is
     interpreted as the relative path `server/share` *inside* the root, never as
     a network share;
   - both `/` and `\` are treated as separators; empty components rejected
     (catches `//`, trailing separators);
   - `.`/`..` components rejected;
   - Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
     `LPT1`–`LPT9`) rejected, including `NUL.txt` forms;
   - components ending in `.` or space rejected (Win32 would strip them);
   - `< > " | ? *` rejected for cross-platform consistency.
2. **Root canonicalisation**: `Sandbox::new` requires the root to exist and be a
   directory and stores its canonical path (Windows verbatim paths are fine).
3. **Per-component walk** with `symlink_metadata`: every intermediate component
   must be a directory; the final component must be a regular file; any reparse
   point (Windows `FILE_ATTRIBUTE_REPARSE_POINT`, Unix symlink) is rejected.
4. **Open, then verify**: size is checked before and after opening; the final
   canonical path is compared component-by-component against the canonical root
   (case-insensitive on Windows). The component-wise comparison prevents
   prefix confusion such as `C:\root` vs `C:\root2`.

### Legacy root-prefix compatibility

Some older TFTP clients issue a download as `get /file.bin`, i.e. they prefix the
file name with the server's root directory. The sandbox therefore strips any
leading `/` or `\` before validation instead of rejecting the request as an
absolute path. This is a compatibility mapping, not a relaxation:

- only **leading** separators are removed; `a//b`, `a/` and trailing separators
  are still rejected;
- `..`, drive letters, `:`, device names and reparse points are still rejected
  after stripping, so `/../secret`, `/C:/Windows/win.ini` and
  `//sub/../../secret` fail;
- a UNC-looking request such as `\\server\share` becomes the relative path
  `server\share` under the root, which is safe (and almost always missing);
- a request for `/` (the root directory itself) is rejected because TFTP cannot
  transfer directories.

Covered by unit tests (`root_prefixed_names_map_to_the_root`,
`rejects_traversal_and_drive_paths`) and by the UDP integration tests
`root_prefixed_filename_is_served` and `root_prefix_does_not_escape_the_sandbox`.

### Residual filesystem risks (accepted, documented)

- **TOCTOU window.** Between the metadata walk and `open`, a local attacker with
  write access inside the root could replace a component with a junction. The
  post-open canonical check narrows this window but cannot eliminate it with
  stable `std` APIs (no `openat`-style relative open with
  `FILE_FLAG_OPEN_REPARSE_POINT`). Protection assumes the export tree is not
  writable by an untrusted local user. A same-user attacker who can modify the
  exported tree can already read those files directly.
- **Hard links.** A hard link inside the root to a file outside cannot be
  distinguished by path checks. Creating one requires write access to the root
  and the target volume.
- **8.3 short names and case/Unicode normalisation** are resolved by
  `canonicalize`; the final component comparison is case-insensitive with an
  ASCII fast path plus a lower-case fallback. Exotic Unicode case-folding is
  not guaranteed to match Win32's internal table exactly. This can only cause a
  *conservative rejection* or a path that is still under the root; it is not a
  known escape vector, but no formal proof exists.
- **Files changed during a transfer** are read as-is; a shrinking file produces
  a shorter final block. No snapshotting is done.

## UDP protocol controls

- Only the first request (RRQ/WRQ) is accepted on the listening socket. DATA,
  ACK, ERROR and OACK arriving there are answered with ERROR 4.
- Each session validates the source address (IP + TID port). Packets from other
  addresses receive ERROR 5, capped at 8 per block exchange to limit reflection
  amplification. Spoofed sources can still receive those packets; TFTP cannot
  prevent this.
- Duplicate ACKs (same or old block) are ignored, never counted twice.
- Unexpected packet types from the peer are ignored rather than answered, to
  avoid error-packet loops.
- Retransmission is bounded by `max_retries` and a per-transfer deadline; there
  is no unbounded exponential backoff.
- A repeated RRQ from the same peer is routed to the existing session and causes
  a resend; it does not create a new session.

Residual: **source spoofing** of the initial RRQ can make the server send DATA to
a victim address (amplification). This is inherent to TFTP over UDP; mitigation
is network ACL/firewall or loopback-only binding, not the protocol implementation.

## Resource limits

| Resource | Bound |
| --- | --- |
| Listen addresses / accept loops | 16 (`MAX_BIND_ADDRESSES`), one socket and accept loop each |
| Concurrent sessions | `max_sessions` (default 16, max 1024) |
| Sessions per peer IP | 8 |
| Request datagram | 1024 bytes |
| DATA payload | negotiated `blksize`, capped by `block_size` (max 65464) |
| File size | `max_file_bytes` (default 64 MiB) |
| Retransmissions | `max_retries` (default 5, max 20) |
| Transfer duration | `max_transfer_duration_secs` (default 1 hour) |
| Event ring | `event_buffer` (default 1024; oldest dropped, counted) |
| GUI log model | 500 rows |
| GUI recent roots | 8 canonicalised paths in a per-user local state file |
| GUI first-run flag | one boolean in a per-user local state file |
| Shutdown drain | `shutdown_grace_secs` (default 5) |

The GUI writes two small local state files under the current user's profile:
`recent-dirs.txt` (up to eight absolute directory paths) and `ui-state.txt` (one
`setup-shown` flag). They contain no credentials and are treated as advisory:
unreadable or unwritable state never prevents the panel from starting. Neither
file is read by the TFTP server; the recent-root list does not affect the
sandbox root unless the operator selects one of those directories.

## GUI helper processes

The desktop panel has no native interface-enumeration or file-dialog dependency
(and no hand-written FFI), so it asks the platform's own runtime:

- Interface discovery runs `powershell.exe` (absolute path) with the .NET
  `NetworkInterface` API, or `hostname -I` on Unix, on a background thread. Only
  IP literals are parsed; the helper is killed after 5 s.
- The root directory picker runs `FolderBrowserDialog` through the same
  PowerShell 5.1 binary, or `zenity`/`kdialog` on Unix, also on a background
  thread. The selected path is accepted only if it is an existing directory.
- Neither helper is started with a shell, with user-controlled arguments or with
  elevation. The initial directory is passed through `TFTPD_R_INITIAL_DIR` so a
  path containing quotes cannot become script text. Helpers are started with
  `CREATE_NO_WINDOW`, and the panel itself is a windowed-subsystem binary.

A machine policy that blocks PowerShell or the dialog tools only degrades these
convenience features; the panel and the server keep working with the two special
address entries and a manually typed root path.

## Logging and error hygiene

- `sanitize_display_name` replaces control characters, normalises backslashes to
  `/`, and truncates to 256 bytes before an event is emitted.
- Client-visible ERROR messages come from fixed constants per error class
  (for example `File not found`, `Access violation`, `Option negotiation
  failed`). They never contain the local path or the underlying OS error text.
- The daemon logs file names and error *kinds*; no credentials exist in the
  design, and configuration is not dumped to logs.
- `tracing` writes to stdout; redirect it to a file/service log as appropriate.

## Explicitly out of scope for this release

- Upload security (atomic temp writes, overwrite policy, path permissions).
- IPC authentication/ACLs and any local control channel.
- Windows Service hardening (service account, SCM recovery, privilege).
- TLS, authentication, rate limiting per IP beyond the concurrent-session cap.
- Audit logging with tamper evidence.
