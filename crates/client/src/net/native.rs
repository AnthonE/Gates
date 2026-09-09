//! The native [`Wire`](super::Wire): wtransport's datagram send, and nothing
//! else.
//!
//! This module exists so that `Session`'s struct definition names no
//! transport type. It is deliberately the smallest file in the crate — the
//! rest of the session's per-frame work is portable and lives one level up
//! (`super`'s header has the measurement).

use super::Wire;
use wtransport::Connection;

/// A connected wtransport session's send half.
///
/// Holds the `Arc<Connection>` the datagram reader task also clones, so the
/// two halves keep the connection alive together and it dies when both are
/// dropped — the same lifetime the field on `Session` had before this split.
pub struct NativeWire {
    connection: std::sync::Arc<Connection>,
}

impl NativeWire {
    pub(crate) fn new(connection: std::sync::Arc<Connection>) -> Self {
        Self { connection }
    }
}

impl Wire for NativeWire {
    fn send_datagram(&self, payload: &[u8]) {
        // `send_datagram`, never `send_datagram_wait` (`CLAUDE.md` traps):
        // drop-oldest, so a congestion stall costs freshness and not latency.
        //
        // The `Err` is dropped here exactly as it was when this line lived in
        // `Session::pump` — there is no recovery for a datagram that did not
        // go, and the next tick's input supersedes it. The connection dying
        // is noticed by the lane drains, not by this.
        let _ = self.connection.send_datagram(payload);
    }
}
