# ADR 0003: Filesystem sandbox strategy

- **Status:** accepted (updated 2026-11-20: legacy root-prefix mapping)
- **Date:** 2026-11-20

## Context

The plan explicitly warns that `canonicalize + starts_with` is not a complete
path-traversal defence, and that Windows reparse points and TOCTOU are real
risks. A TFTP name is attacker-controlled and may contain separators, device
names, ADS syntax and Unicode.

## Decision

Defence in depth, implemented in `tftpd-r-core::filesystem`:

1. **Syntactic validation** before any filesystem call: strip a leading `/` or
   `\` (legacy root prefix, see update below), then reject drive letters/`:`
   (also blocks ADS), empty components, `.`/`..`, control characters, trailing
   dots/spaces, invalid Windows characters and reserved device names. Treat both
   `/` and `\` as separators so the meaning does not depend on the platform.
2. **Canonical root** stored at sandbox construction; the root itself may be a
   symlink/junction chosen by the operator (it is canonicalised once).
3. **Component walk** on every open: intermediate components must be real
   directories; the leaf must be a regular file; every component is checked with
   `symlink_metadata` and any reparse point (`FILE_ATTRIBUTE_REPARSE_POINT`) or
   symlink is rejected.
4. **Open, then re-validate**: enforce the size limit before and after opening,
   canonicalise the leaf after the open, and compare the result to the canonical
   root **component by component** (case-insensitive on Windows), not as a
   string prefix.
5. **No writes**: WRQ is rejected, so there is no create/replace path to secure
   in this release.

## Update 2026-11-20: legacy root-prefix mapping

### Context

Old TFTP clients may request a download as `get /firmware.bin`, prefixing the
name with the server's root directory. The first implementation rejected every
leading separator as an absolute path, which broke those clients even though the
request was intended to be root-relative.

### Decision

Strip any leading `/` and `\` before component validation and interpret the
remainder relative to the canonical sandbox root. Only leading separators are
removed; every other rule (empty components, `.`/`..`, `:`, device names, control
characters, reparse points, canonical-prefix check) still applies afterwards. A
name that consists only of separators (`/`, `\\`) refers to the root directory
itself and is rejected as an unusable file name.

### Consequences

- `/firmware.bin`, `//sub/file.bin` and `\firmware.bin` are served from the root;
  this is covered by unit and UDP integration tests.
- `/../secret`, `/C:/Windows/win.ini` and `//sub/../../secret` are still rejected.
- UNC-like names are reduced to a relative path under the root
  (`\\server\share` → `server/share`), which cannot leave the root.
- The display name in events/logs no longer contains the legacy prefix.

## Consequences

- Traversal, symlink/junction escapes, device names, ADS and `C:\root2` prefix
  confusion are covered by tests. The legacy root-prefix mapping has its own
  positive and negative tests (served root-prefixed files, rejected
  root-prefixed traversal).
- A TOCTOU window remains between metadata checks and `open`; stable `std` has
  no openat-style relative open on Windows. The threat model assumes the export
  tree is not writable by an untrusted local user.
- Hard links inside the root are indistinguishable from regular files.
- Unicode case folding in the final prefix check is conservative, not formally
  equivalent to Win32's table; failures to match result in rejection.
- Future upload support must use this same validation plus an atomic temp-file
  strategy and must return to this ADR for a new review.
