# ADR 0005: Uploads (WRQ) remain disabled

- **Status:** accepted (deviation from the original phased plan)
- **Date:** 2026-11-20

## Context

At the time of this decision, Phase 7 covered WRQ uploads. The roadmap now
places target-machine validation in Phase 6 and IPC in Phase 7; WRQ is current
Phase 8. A safe implementation needs path/overwrite checks before creating the
target, a temporary file in the destination directory, platform-specific atomic
replacement semantics, disk-full/cancel cleanup, upload size limits, race/TOCTOU analysis for
creation, and its own interoperability tests.

Shipping a half-reviewed write path would turn a read-only server into a data
loss or path-escape vector.

## Decision

- WRQ is parsed and always answered with ERROR 2 (`Access violation`, message
  "Uploads are disabled").
- `allow_upload = true` and `overwrite = true` are **configuration validation
  errors**, with messages explaining that uploads are not implemented. The GUI
  shows the upload switch disabled.
- The codec, `Mode` parsing and option negotiation already handle WRQ-shaped
  requests, so the protocol groundwork exists.

## Consequences

- No create/replace/write code exists in this release; the sandbox only opens
  files for reading.
- The default and only behaviour is read-only, matching the plan's MVP scope.
- Current Phase 8 (formerly Phase 7) must revisit this ADR, implement atomic
  temp writes plus cleanup, add upload path security tests, and keep
  `allow_upload` opt-in and default false.
