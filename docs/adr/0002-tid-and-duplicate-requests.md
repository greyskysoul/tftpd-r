# ADR 0002: Transfer IDs and duplicate requests

- **Status:** accepted
- **Date:** 2026-11-20

## Context

RFC 1350 requires the server to answer a request from a new UDP port (the
transfer ID, TID) and to ignore packets that do not come from the client's TID.
Clients retransmit their RRQ when the first response is lost, which naively
creates duplicate server sessions, duplicate ephemeral ports and duplicate file
reads.

## Decision

- The listener binds `bind_address`; each accepted session binds a fresh
  ephemeral socket in the listener's address family and uses it for all
  DATA/ERROR packets of that transfer.
- The accept loop keeps a `peer SocketAddr -> session` map. A second RRQ from a
  peer that already has a live session is **not** spawned as a new session: the
  accept loop sends `SessionSignal::Resend` over a bounded channel, and the
  session immediately retransmits its current OACK/DATA packet.
- The map entry is removed by a guard when the session task ends, and only if
  the entry still belongs to that session ID (a newer session may have replaced
  it).
- Datagrams from any address other than the expected peer receive a
  rate-limited ERROR 5 (maximum 8 per block exchange); packets from the expected
  peer that are not the awaited ACK are ignored.

## Consequences

- Retried requests keep the client's expected TID and do not consume extra
  session slots.
- A duplicate RRQ can still cause one extra retransmission that is already in
  flight when the ACK arrives; clients must tolerate duplicate DATA (RFC 1350
  permits it). Integration tests cover this.
- Spoofed sources can still trigger ERROR 5 responses (inherent UDP reflection);
  the cap limits amplification.
