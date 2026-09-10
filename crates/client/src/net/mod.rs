//! What a session needs from a transport, and the lane plumbing that needs
//! nothing from one.
//!
//! Split out of `lib.rs` on 2026-09-09 for the browser client
//! (`DECISIONS.md`; `findings/web-build-20260909.md` §10.3). The point of the
//! split is a measurement rather than a taste: across every METHOD on
//! `Session`, **exactly one call touches the transport** — `pump`'s
//! `send_datagram`. Everything else in this file is portable as written,
//! because it was already built out of `tokio::sync` (a feature tokio
//! supports on wasm) and a plain `Mutex` over an array.
//!
//! *Methods*, precisely: the three long-lived tasks `connect` spawns do keep
//! touching the transport, and they are not covered by that count. They own
//! their own halves of the connection and outlive the call that made them, so
//! they belong to whatever built them — which is why `connect`, not this
//! file, was the half a web build still had to write.
//!
//! So [`Wire`] has one method, and that is the honest size of the seam rather
//! than a stub of a larger one. **The prediction it was written on held**
//! (2026-09-10): [`web`] fills the same [`DatagramRx`] ring from a
//! `ReadableStream` reader and pushes events into the same `mpsc`, and the one
//! thing it could not share was the send. What the measurement did NOT
//! predict is the other cost of a browser — that a `ReadableStream` has no
//! `read_exact`, so the stream lane owes a framing state machine that
//! wtransport gave the desktop client for free. That is [`frame`], and it is
//! the largest piece of code in this directory.
//!
//! **Every method here is synchronous, and that is load-bearing.**
//! `Session`'s whole per-frame surface refuses to await on purpose — a frame
//! that awaits the network is a dropped frame — so no lane operation needs
//! `async`, which means no `async-trait` and, more importantly, no `Send`
//! bound to reconcile: `tokio::spawn` requires a `Send` future and
//! `wasm_bindgen_futures::spawn_local` must not have one, and an `async fn`
//! in a trait cannot express "Send on one target, not the other". Connecting
//! is the one async act, and it deliberately stays a free function in
//! `lib.rs` rather than a trait method.

/// The join handshake with the transport taken out of it — pure, and
/// compiled on every target because a web `connect` drives the same three
/// functions rather than copying them.
pub mod handshake;

/// The stream lane's framing, with no transport under it. Shared by both
/// builds — natively `read_exact` makes the decoder half unnecessary, but the
/// ENCODER is the same bytes on both and now that is one function rather than
/// two that agree by inspection.
pub mod frame;

#[cfg(feature = "native")]
pub mod native;

#[cfg(target_arch = "wasm32")]
pub mod web;

use sim_core::limits::DATAGRAM_BUDGET_BYTES;

/// The transport, reduced to the one thing a connected session asks of it.
///
/// One method is not an under-specified trait — it is the measured surface.
/// The reliable C→S lane is an `mpsc::Sender` the writer task owns, both
/// inbound lanes are drained by the free functions below, and closed-ness is
/// derived from those drains, so none of them belong here.
pub trait Wire {
    /// Send one unreliable C→S datagram. **Never blocks and never awaits**,
    /// and a congestion stall must cost freshness rather than latency
    /// (`CLAUDE.md` traps: `send_datagram`, never `send_datagram_wait`).
    ///
    /// Returns nothing on purpose: the caller has no recovery for a datagram
    /// that did not go, and the next tick's input supersedes it anyway.
    fn send_datagram(&self, payload: &[u8]);
}

/// The `Wire` this build actually uses.
///
/// A cfg-selected **concrete type**, never `Box<dyn Wire>` and never a
/// generic parameter on `Session`. Both alternatives would cost something
/// real: a trait object cannot take `impl FnMut` drains and would force an
/// allocation on a hot path, and a generic would leak a type parameter into
/// `render::Net` and every signature that names a session.
///
/// **The two arms are selected by different things, and that is deliberate.**
/// `native` is a cargo feature, because `wtransport` is an optional dependency
/// that a `--no-default-features` build turns off; `wasm32` is a target,
/// because a page's transport is a property of the platform and not of a flag.
/// The `compile_error!` pair in `lib.rs` is what proves exactly one of them
/// holds, so this alias is total without a fallback arm that could quietly win
/// on a build nobody meant to make.
#[cfg(feature = "native")]
pub type ActiveWire = native::NativeWire;

#[cfg(all(target_arch = "wasm32", not(feature = "native")))]
pub type ActiveWire = web::WebWire;

/// Parse the dotted hex SHA-256 the shard prints at boot (`aa:bb:cc:...`)
/// into the 32 bytes a certificate pin is made of.
///
/// **Ours because the browser build cannot borrow wtransport's.** Natively
/// this is `"…".parse::<wtransport::tls::Sha256Digest>()`, one line inside
/// `client_endpoint`; a page has no wtransport, and `serverCertificateHashes`
/// wants the raw bytes. Written here rather than in `net/web.rs` for one
/// reason: it is pure, so putting it on every target is what lets
/// `tests/framing.rs` run it on the box you are sitting at instead of only in
/// a browser.
///
/// Deliberately strict — exactly 32 groups of exactly two hex digits,
/// separated by single colons, nothing else. A pin is a security control and
/// a lenient parser for one is how a truncated paste becomes a shorter
/// secret. Whitespace at the ends is trimmed because a copied line carries
/// it; nothing inside is.
pub fn parse_cert_digest(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    let mut out = [0u8; 32];
    let mut groups = 0usize;
    for (i, group) in text.split(':').enumerate() {
        if i >= out.len() || group.len() != 2 {
            return None;
        }
        // `from_str_radix` accepts a leading `+`, which is not a hex digit
        // and would make `+f` parse as 15. Refused explicitly rather than
        // trusted, because the whole value of this function is that it is
        // strict.
        if !group.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        out[i] = u8::from_str_radix(group, 16).ok()?;
        groups = i + 1;
    }
    (groups == out.len()).then_some(out)
}

/// Why an action could not be queued. Both arms are states a panel draws
/// rather than errors it swallows: a full lane means the sim is behind and
/// the move has NOT been sent, and a closed one means the session is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    Full,
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Full => write!(f, "the server is behind - try again"),
            SendError::Closed => write!(f, "disconnected"),
        }
    }
}
/// Drain one lane without blocking, handing each message to `each`; answer
/// whether the lane's sender has hung up.
///
/// **The buffered last words land before the hangup is reported** — tokio's
/// mpsc yields everything queued ahead of the drop before it answers
/// `Disconnected`, and `lib.rs`'s
/// `the_last_words_land_before_the_hangup_is_reported` pins that (it stayed
/// with the session it describes when this moved), because the whole disconnect
/// path leans on it: the facts the server sent in its final flush (a kick's
/// toast, the last snapshot) must reach the core before the client declares
/// the session over, or the reason for the hangup is the first thing lost.
pub(crate) fn drain_lane(
    rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut each: impl FnMut(&[u8]),
) -> bool {
    use tokio::sync::mpsc::error::TryRecvError;
    loop {
        match rx.try_recv() {
            Ok(bytes) => each(&bytes),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
}

/// The shared handle to the datagram ring — see [`datagram_lane`].
pub(crate) type DatagramRx = std::sync::Arc<std::sync::Mutex<DgRing>>;

/// The datagram ring's storage: `CLIENT_DG_RING` reusable buffers, oldest
/// first from `head`. Payload `Vec`s are cleared and refilled in place, so
/// after warm-up neither side of the lane allocates.
pub(crate) struct DgRing {
    bufs: [Vec<u8>; sim_core::limits::CLIENT_DG_RING],
    head: usize,
    len: usize,
    /// Snapshots overwritten before a frame drained them — wall 4's
    /// stated policy (drop-oldest), counted rather than silent.
    pub(crate) dropped: u64,
    pub(crate) closed: bool,
}

impl DgRing {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let cap = sim_core::limits::CLIENT_DG_RING;
        if self.len == cap {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
            self.dropped += 1;
        }
        let at = (self.head + self.len) % cap;
        self.bufs[at].clear();
        self.bufs[at].extend_from_slice(bytes);
        self.len += 1;
    }
}

/// The datagram lane: **a bounded ring, drained whole every frame**
/// (netcode v2 S2; `limits::CLIENT_DG_RING`, drop-oldest, counted).
///
/// This was a depth-one latest-wins `watch`, and that shape was honest
/// about exactly one consumer: the OWN body, whose snapshot genuinely
/// replaces its predecessor. Every OTHER body is reconstructed by the
/// interpolator from the samples in between, so each collapsed pair was a
/// hole in someone's motion — at ≤ 30 fps against 30 Hz snapshots, half
/// the stream never reached `client-core` at all — and S5's arrival-jitter
/// estimator cannot measure a network through a slot that conflates
/// arrival with the render loop. Delivery is in arrival order; dropping
/// the OLDEST under a stall keeps the freshest world, which is the half of
/// latest-wins that was worth keeping.
///
/// **Why dropping old snapshots remains safe** (the watch's argument,
/// re-checked): the server deltas only against an ACKED snapshot
/// (`server::client::baseline` takes `newest_acked`), removed ids ride
/// every snapshot until acked, and `nudge`/`last_executed_seq`/the v60
/// gauges are levels in every header, not edges. Nothing on this lane is
/// exactly-once.
pub(crate) fn datagram_lane() -> DatagramRx {
    std::sync::Arc::new(std::sync::Mutex::new(DgRing {
        bufs: std::array::from_fn(|_| Vec::with_capacity(DATAGRAM_BUDGET_BYTES)),
        head: 0,
        len: 0,
        dropped: 0,
        closed: false,
    }))
}

/// Hand over every pending datagram, oldest first, and answer whether the
/// lane is finished — closed AND empty, so the last snapshots land in (or
/// before) the same drain that reports the hangup, the property
/// `the_last_words_land_before_the_hangup_is_reported` pins on the
/// reliable lane. A poisoned lock reads as gone: the reader task panicking
/// mid-push is a dead lane, not a recoverable state.
///
/// **The payloads are swapped out, not copied, and processed after the
/// lock drops.** `each` here is `ClientCore::on_datagram` — a whole
/// snapshot decode — and holding the ring's mutex across it would block
/// the reader task for the span (the watch this replaced documented the
/// same rule for its read guard). The swap trades `Vec` pointers with the
/// session-owned `scratch`, so the frame path allocates nothing after
/// warm-up: the ring refills the swapped-in buffers in place.
pub(crate) fn drain_datagram(
    rx: &DatagramRx,
    scratch: &mut [Vec<u8>],
    mut each: impl FnMut(&[u8]),
) -> bool {
    let cap = sim_core::limits::CLIENT_DG_RING;
    let (n, gone) = {
        let Ok(mut r) = rx.lock() else {
            return true;
        };
        let n = r.len.min(scratch.len());
        for (k, s) in scratch.iter_mut().enumerate().take(n) {
            let i = (r.head + k) % cap;
            std::mem::swap(&mut r.bufs[i], s);
        }
        r.head = (r.head + n) % cap;
        r.len -= n;
        (n, r.closed && r.len == 0)
    };
    for b in scratch.iter().take(n) {
        each(b);
    }
    gone
}
