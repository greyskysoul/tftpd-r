# ADR 0006: Octet mode only

- **Status:** accepted
- **Date:** 2026-11-20

## Context

RFC 1350 defines `netascii`, `octet` and the obsolete `mail` modes. Silent
`netascii`-as-`octet` translation is a classic source of corrupted firmware or
boot images. The plan requires an explicit rejection rather than a silent
fallback.

## Decision

- `octet` is case-insensitively accepted and is the only served mode.
- `netascii` and `mail` requests are parsed, then rejected with ERROR 0 and a
  clear message ("netascii mode is not supported", "mail mode is not
  supported"). The event log records the rejection.
- Unknown mode strings fail codec decoding and are answered with ERROR 4.

## Consequences

- No line-ending translation code exists, eliminating a class of data
  corruption bugs.
- Clients that require `netascii` (rare for boot images) cannot use this server;
  they receive an explicit error rather than silently wrong content.
- Adding `netascii` later requires a dedicated converter with binary/text
  boundary tests and updates to this ADR.
