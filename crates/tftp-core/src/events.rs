//! Transfer events for the GUI and the daemon log.
//!
//! Events are deliberately small and owned; the server never blocks on a slow
//! subscriber. [`EventBus`] keeps a bounded ring that a front-end drains at its
//! own pace (the GUI polls every few hundred milliseconds).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

/// Monotonic identifier for one transfer attempt.
pub type SessionId = u64;

/// Lifecycle of a transfer as observed by a front-end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// Transfer started (file opened, first packet about to be sent).
    Started,
    /// Bytes were sent; emitted at a throttled rate.
    Progress,
    /// Transfer completed successfully.
    Completed,
    /// Transfer failed (timeout, peer error, disk error, cancellation).
    Failed {
        /// Client-safe reason.
        reason: String,
    },
    /// Request rejected before a transfer started.
    Rejected {
        /// Client-safe reason.
        reason: String,
    },
}

/// One server event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TftpEvent {
    /// Monotonic sequence number assigned by [`EventBus`] (0 while unmanaged).
    pub seq: u64,
    /// Session identifier, stable for the lifetime of one transfer.
    pub session_id: SessionId,
    /// Remote peer address (IP and TID port).
    pub peer: SocketAddr,
    /// Sanitised file name (relative, `/` separators, no control characters).
    pub file: String,
    /// Event payload.
    pub kind: EventKind,
    /// Bytes transferred so far.
    pub bytes: u64,
    /// File size when known at start.
    pub total: Option<u64>,
    /// Wall-clock timestamp (only for display; ordering uses `seq`).
    pub at: SystemTime,
}

impl TftpEvent {
    /// Build an event with `seq = 0` and `at = now`.
    #[must_use]
    pub fn new(
        session_id: SessionId,
        peer: SocketAddr,
        file: String,
        kind: EventKind,
        bytes: u64,
        total: Option<u64>,
    ) -> Self {
        Self {
            seq: 0,
            session_id,
            peer,
            file,
            kind,
            bytes,
            total,
            at: SystemTime::now(),
        }
    }
}

/// Anything that can receive server events.
///
/// Implementations must be cheap and non-blocking: the server calls `emit` from
/// async transfer tasks.
pub trait EventSink: Send + Sync {
    /// Record one event. Must not block for long and must not panic.
    fn emit(&self, event: TftpEvent);
}

impl<T: EventSink + ?Sized> EventSink for Arc<T> {
    fn emit(&self, event: TftpEvent) {
        (**self).emit(event);
    }
}

/// Bounded in-memory event ring shared with a GUI.
#[derive(Debug)]
pub struct EventBus {
    capacity: usize,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    next_seq: u64,
    events: VecDeque<TftpEvent>,
    dropped: u64,
}

impl EventBus {
    /// Create a bus that keeps at most `capacity` events (minimum 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Create a bus with the default capacity (1024) wrapped in an `Arc`.
    #[must_use]
    pub fn default_shared() -> Arc<Self> {
        Arc::new(Self::new(1024))
    }

    /// Last sequence number handed out (0 when no events were emitted).
    #[must_use]
    pub fn latest_seq(&self) -> u64 {
        self.lock().next_seq
    }

    /// Number of events dropped because the ring was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    /// Return all retained events with `seq > after_seq`, oldest first.
    ///
    /// The caller should remember the highest `seq` it has seen and pass it back
    /// next time. Events evicted by the bounded ring are lost by design; compare
    /// [`Self::dropped`] against a previous reading to detect that.
    #[must_use]
    pub fn drain_since(&self, after_seq: u64) -> Vec<TftpEvent> {
        let inner = self.lock();
        inner
            .events
            .iter()
            .filter(|event| event.seq > after_seq)
            .cloned()
            .collect()
    }

    /// Snapshot of every retained event (primarily for diagnostics/tests).
    #[must_use]
    pub fn snapshot(&self) -> Vec<TftpEvent> {
        self.lock().events.iter().cloned().collect()
    }

    /// Drop all retained events. Sequence numbers keep increasing so a UI that
    /// clears its view does not immediately re-display old events.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.events.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventSink for EventBus {
    fn emit(&self, mut event: TftpEvent) {
        let mut inner = self.lock();
        inner.next_seq = inner.next_seq.saturating_add(1);
        event.seq = inner.next_seq;
        if inner.events.len() >= self.capacity {
            inner.events.pop_front();
            inner.dropped = inner.dropped.saturating_add(1);
        }
        inner.events.push_back(event);
    }
}

/// Make a file name safe for display/logging: control characters are replaced,
/// backslashes become `/`, and the result is bounded.
#[must_use]
pub fn sanitize_display_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len().min(256));
    for ch in name.chars() {
        if out.len() + ch.len_utf8() > 256 {
            out.push('…');
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else if ch == '\\' {
            out.push('/');
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session_id: SessionId, kind: EventKind) -> TftpEvent {
        TftpEvent::new(
            session_id,
            "127.0.0.1:1234".parse().expect("addr"),
            "a.bin".into(),
            kind,
            0,
            None,
        )
    }

    #[test]
    fn events_get_increasing_sequence_numbers() {
        let bus = EventBus::new(4);
        bus.emit(event(1, EventKind::Started));
        bus.emit(event(1, EventKind::Completed));
        let events = bus.snapshot();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
        assert_eq!(bus.latest_seq(), 2);
    }

    #[test]
    fn drain_since_returns_only_new_events() {
        let bus = EventBus::new(8);
        bus.emit(event(1, EventKind::Started));
        bus.emit(event(1, EventKind::Progress));
        let first = bus.drain_since(0);
        assert_eq!(first.len(), 2);
        assert!(bus.drain_since(first[1].seq).is_empty());
        bus.emit(event(1, EventKind::Completed));
        let last = bus.drain_since(first[1].seq);
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].kind, EventKind::Completed);
    }

    #[test]
    fn ring_drops_oldest_and_counts_drops() {
        let bus = EventBus::new(2);
        for block in 0..5 {
            bus.emit(TftpEvent::new(
                1,
                "127.0.0.1:1".parse().expect("addr"),
                "f".into(),
                EventKind::Progress,
                block,
                Some(10),
            ));
        }
        assert_eq!(bus.snapshot().len(), 2);
        assert_eq!(bus.dropped(), 3);
        assert_eq!(bus.latest_seq(), 5);
        let retained = bus.drain_since(0);
        assert_eq!(
            retained.iter().map(|event| event.seq).collect::<Vec<_>>(),
            vec![4, 5]
        );
    }

    #[test]
    fn clear_keeps_sequence_numbers_monotonic() {
        let bus = EventBus::new(4);
        bus.emit(event(1, EventKind::Started));
        bus.clear();
        assert!(bus.snapshot().is_empty());
        assert_eq!(bus.latest_seq(), 1);
        bus.emit(event(1, EventKind::Completed));
        assert_eq!(bus.drain_since(1).len(), 1);
    }

    #[test]
    fn display_names_are_sanitised() {
        assert_eq!(sanitize_display_name("dir\\file\n.bin"), "dir/file .bin");
        let long = "a".repeat(300);
        assert!(sanitize_display_name(&long).chars().count() <= 257);
    }

    #[test]
    fn arc_event_sink_forwards_events() {
        let bus = Arc::new(EventBus::new(4));
        let sink: Arc<dyn EventSink> = bus.clone();
        sink.emit(event(9, EventKind::Started));
        assert_eq!(bus.latest_seq(), 1);
    }
}
