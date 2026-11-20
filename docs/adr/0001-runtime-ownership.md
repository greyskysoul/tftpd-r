# ADR 0001: Runtime ownership

- **Status:** accepted
- **Date:** 2026-11-20

## Context

`tftp-core` uses Tokio sockets, timers, channels and tasks. Creating a runtime
inside a library is surprising, can deadlock when called from an async context,
and makes shutdown ownership unclear. The GUI and the daemon have different
runtime needs.

## Decision

`tftp-core` never creates a Tokio runtime and never calls `block_on`. The
front-end owns the runtime:

- `tftp-daemon` builds a multi-thread runtime in `main`, runs `serve_async` on
  it, and drops it after `ServerHandle::stop()` completes.
- `tftp-gui-slint` builds a two-worker multi-thread runtime inside
  `EmbeddedController`. `start` uses `block_on` because binding + validation are
  fast and non-blocking; `stop` is spawned and polled so the UI thread is never
  blocked for the grace period. `shutdown` (after the event loop exits) blocks
  until the stop task completes, with a 10 s safety timeout.

`ServerHandle` must be used from a runtime that stays alive; dropping the
runtime aborts server tasks.

## Consequences

- Embedders choose their runtime configuration and can share it with other work.
- Tests use `#[tokio::test]` / a manual runtime and can therefore assert task
  cleanup deterministically.
- Async trait object-safety is not needed; the controller is a concrete type.
