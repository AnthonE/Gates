//! The browser [`Wire`](super::Wire) and the stream plumbing around it.
//!
//! The desktop twin is `net/native.rs`, and the asymmetry between the two
//! files is the honest shape of this port rather than an accident: natively
//! the transport hands us `read_exact`, a datagram receiver and a socket, and
//! `NativeWire` is 38 lines. A page gets `ReadableStream`s of arbitrary
//! chunks, so the same job costs a framing state machine
//! ([`super::frame`], pure and gated in `tests/framing.rs`) and a reader that
//! carries its own residue.
//!
//! What is NOT here is everything `findings/web-build-20260909.md` §10.3
//! measured as already portable, and that is most of it: the reliable C→S
//! lane is a `tokio::sync::mpsc::Sender` (a feature tokio supports on wasm),
//! both inbound lanes are drained by the free functions in `super`, and
//! closed-ness is derived from those drains. This module fills the *same*
//! `DgRing` and pushes into the *same* channel the desktop client uses.
//!
//! **Nothing in this file may block or await inside a frame.** That rule is
//! stricter in a tab than on a desktop, because the thread a page would block
//! is the one that also runs the compositor: a native stall costs a frame, a
//! browser stall costs the whole document. Every long-lived read and write
//! runs in a `spawn_local` task and talks to the frame through the rings.

use std::cell::Cell;

use js_sys::Uint8Array;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    ReadableStream, ReadableStreamDefaultReader, WebTransport, WebTransportDatagramDuplexStream,
    WritableStreamDefaultWriter,
};

use super::Wire;

/// Turn a rejected promise or a thrown value into a sentence.
///
/// A `JsValue` is not `Display` and its `Debug` is a dump, so every error path
/// in this module funnels through here — a player who cannot connect must be
/// told what refused them, and `JsValue(DOMException)` is not that. Prefers
/// the value's own `message` (every `DOMException` and `WebTransportError`
/// carries one) and falls back to its string coercion.
pub(crate) fn js_err(what: &str, v: &JsValue) -> String {
    let msg = js_sys::Reflect::get(v, &JsValue::from_str("message"))
        .ok()
        .and_then(|m| m.as_string())
        .filter(|s| !s.is_empty())
        .or_else(|| v.as_string())
        .unwrap_or_else(|| String::from(js_sys::Object::from(v.clone()).to_string()));
    format!("{what}: {msg}")
}

/// A connected page's datagram send half.
///
/// Holds the duplex stream as well as the writer because **the MTU has to be
/// read live, per send**, and it hangs off the stream rather than the writer.
pub struct WebWire {
    /// **Held for the same reason `NativeWire` holds its `Arc<Connection>`:
    /// so that the session owns the connection's lifetime.** In JS the
    /// transport would survive on the strength of the streams still
    /// referencing it, which is a garbage collector's opinion about our
    /// disconnect semantics. Naming it here makes "the session is dropped, so
    /// the connection ends" a fact about this struct instead.
    transport: WebTransport,
    datagrams: WebTransportDatagramDuplexStream,
    writer: WritableStreamDefaultWriter,
    /// Datagrams refused because the payload exceeded the live
    /// `maxDatagramSize`. Counted rather than silent — see `send_datagram`.
    pub over_mtu: Cell<u64>,
    /// Datagrams refused because the writer's queue was already full.
    pub backpressured: Cell<u64>,
}

impl WebWire {
    pub(crate) fn new(transport: &WebTransport) -> Result<Self, String> {
        let datagrams = transport.datagrams();
        let writer = WritableStreamDefaultWriter::new(&datagrams.writable())
            .map_err(|e| js_err("datagram writer", &e))?;
        Ok(Self {
            transport: transport.clone(),
            datagrams,
            writer,
            over_mtu: Cell::new(0),
            backpressured: Cell::new(0),
        })
    }
}

impl Wire for WebWire {
    /// **The clamp `CLAUDE.md`'s trap list has demanded since the first
    /// browser client, finally at the one call site that can enforce it.**
    ///
    /// The trap, verbatim: *"A browser datagram write over `maxDatagramSize`
    /// silently succeeds and sends nothing — clamp every send against the
    /// live value."* Silently: the promise resolves, the writer accepts, the
    /// bytes evaporate. There is no error to notice and no counter in the
    /// platform, which is why this refuses rather than trusting the write —
    /// and why the refusal is *counted*, so an input lane that has quietly
    /// stopped reaching the shard is a number somebody can read instead of a
    /// player who cannot move.
    ///
    /// **Refused, never truncated.** Every payload on this lane is an encoded
    /// message with a length its decoder checks; half of one is not a smaller
    /// message, it is garbage that costs the server a parse.
    ///
    /// `DATAGRAM_BUDGET_BYTES` is 1,100 and sits under the 1,200-byte QUIC
    /// floor, so this should never fire on a healthy path. It is here because
    /// "should never fire" is exactly what was said about it for a year while
    /// nothing checked.
    fn send_datagram(&self, payload: &[u8]) {
        let max = self.datagrams.max_datagram_size() as usize;
        if payload.len() > max {
            self.over_mtu.set(self.over_mtu.get() + 1);
            return;
        }
        // Backpressure costs freshness, not latency — the same law as
        // `send_datagram` over `send_datagram_wait` natively. A full queue
        // means the next tick's input supersedes this one anyway, so the
        // drop is the correct answer and awaiting `writer.ready` would be
        // the wrong one.
        //
        // ⚠ **The depth this compares against is the browser's, not ours.**
        // `outgoingHighWaterMark` has a UA-chosen default and nothing here
        // sets it, deliberately: a number written into this file would be an
        // invented knob, and `CLAUDE.md` is explicit that a tunable is either
        // spoken in `DECISIONS.md` or carries a documented default. So the
        // default is the browser's and `backpressured` is how anybody finds
        // out it was wrong — at one datagram per frame against a lane whose
        // writes resolve without an ack, this should never fire, and the
        // counter is what turns "should never" into something checkable.
        if self
            .writer
            .desired_size()
            .ok()
            .flatten()
            .is_some_and(|d| d <= 0.0)
        {
            self.backpressured.set(self.backpressured.get() + 1);
            return;
        }
        // A fresh `Uint8Array` per send, deliberately, rather than a reused
        // view over wasm memory. A view is invalidated by any memory growth
        // and the platform is entitled to hold the chunk past this call; one
        // small JS allocation at 30 Hz is not a cost worth that class of bug.
        let chunk = Uint8Array::new_with_length(payload.len() as u32);
        chunk.copy_from(payload);
        let promise = self.writer.write_with_chunk(&chunk);
        // Awaited in a task purely so a rejection is *handled*. There is no
        // recovery for a datagram that did not go — the caller has none
        // either, which is why `Wire::send_datagram` returns nothing — but an
        // unhandled promise rejection is a console error on every frame of a
        // dying connection, and the lane drains are what actually notice the
        // hangup.
        spawn_local(async move {
            let _ = JsFuture::from(promise).await;
        });
    }
}

/// A frame-at-a-time reader over one `ReadableStream` of arbitrary chunks.
///
/// Owns three things a browser makes the caller own: the reader, the framing
/// state machine, and **the residue** — the tail of the last chunk that was
/// not part of the frame just delivered. That third one is the bug this type
/// exists to make impossible. Natively, `read_exact` stops on the byte the
/// caller asked for and the next call resumes there; here, the read that
/// completes the handshake's `welcome` can perfectly well have carried the
/// first four event frames with it, and a reader that dropped its tail would
/// lose them with nothing to show for it.
pub(crate) struct FrameReader<const N: usize> {
    reader: ReadableStreamDefaultReader,
    buf: super::frame::FrameBuf<N>,
    /// The chunk currently being consumed, and how far into it we are.
    pending: Vec<u8>,
    at: usize,
}

impl<const N: usize> FrameReader<N> {
    pub(crate) fn new(stream: &ReadableStream) -> Result<Self, String> {
        Ok(Self {
            reader: ReadableStreamDefaultReader::new(stream)
                .map_err(|e| js_err("stream reader", &e))?,
            buf: super::frame::FrameBuf::new(),
            pending: Vec::new(),
            at: 0,
        })
    }

    /// Hand the reader and its undelivered residue to a `FrameReader` of a
    /// different ceiling.
    ///
    /// The handshake reads at `MAX_STREAM_MSG_BYTES` and the event lane at
    /// `MAX_EVENT_MSG_BYTES`, which are different `N` and therefore different
    /// types. Only ever called between frames — the framing state is empty at
    /// that point by construction, so nothing but the raw tail has to travel.
    pub(crate) fn rekey<const M: usize>(self) -> FrameReader<M> {
        debug_assert!(
            !self.buf.complete(),
            "rekey with an undrained frame would drop it"
        );
        FrameReader {
            reader: self.reader,
            buf: super::frame::FrameBuf::new(),
            pending: self.pending,
            at: self.at,
        }
    }

    /// The next whole frame, or `None` once the stream ends or refuses.
    ///
    /// `Err` and end-of-stream are one answer on purpose: both mean this lane
    /// is finished, both are latched by the caller as a hangup, and the
    /// distinction has no reader. The reason is logged where it happens.
    pub(crate) async fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            // Drain what is already in hand before asking for more — the
            // residue case above.
            while self.at < self.pending.len() {
                let (used, done) = match self.buf.push(&self.pending[self.at..]) {
                    Ok(x) => x,
                    Err(e) => {
                        web_sys::console::error_1(&JsValue::from_str(&format!(
                            "gates: stream lane refused a frame: {e}"
                        )));
                        return None;
                    }
                };
                self.at += used;
                if done {
                    let out = self.buf.frame().to_vec();
                    self.buf.take();
                    return Some(out);
                }
            }
            self.pending.clear();
            self.at = 0;

            let result = JsFuture::from(self.reader.read()).await.ok()?;
            let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
                .ok()
                .and_then(|d| d.as_bool())
                .unwrap_or(true);
            if done {
                return None;
            }
            let value = js_sys::Reflect::get(&result, &JsValue::from_str("value")).ok()?;
            let chunk: Uint8Array = value.dyn_into().ok()?;
            self.pending.resize(chunk.length() as usize, 0);
            chunk.copy_to(&mut self.pending);
        }
    }
}

/// Read datagrams off `stream` into `ring` until the transport dies.
///
/// The browser's unit here is already a datagram — no framing, one chunk one
/// packet — so this is the whole of what `connect`'s wtransport
/// `receive_datagram` loop does, and it pushes into the identical ring with
/// the identical drop-oldest policy.
pub(crate) fn spawn_datagram_reader(stream: ReadableStream, ring: super::DatagramRx) {
    spawn_local(async move {
        let reader = match ReadableStreamDefaultReader::new(&stream) {
            Ok(r) => r,
            Err(_) => {
                if let Ok(mut r) = ring.lock() {
                    r.closed = true;
                }
                return;
            }
        };
        let mut scratch: Vec<u8> = Vec::new();
        loop {
            let Ok(result) = JsFuture::from(reader.read()).await else {
                break;
            };
            let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
                .ok()
                .and_then(|d| d.as_bool())
                .unwrap_or(true);
            if done {
                break;
            }
            let Ok(value) = js_sys::Reflect::get(&result, &JsValue::from_str("value")) else {
                break;
            };
            let Ok(chunk) = value.dyn_into::<Uint8Array>() else {
                break;
            };
            scratch.resize(chunk.length() as usize, 0);
            chunk.copy_to(&mut scratch);
            let Ok(mut r) = ring.lock() else { break };
            r.push(&scratch);
        }
        if let Ok(mut r) = ring.lock() {
            r.closed = true;
        }
    });
}

/// Pump the reliable C→S lane onto the bidi stream's writable half.
///
/// The desktop twin is the third `tokio::spawn` in `Session::connect`, and it
/// carries the same load-bearing property: **the writer is owned by this task
/// and released only when the channel closes**, which finishes the C→S
/// direction at the one moment that is correct. Dropping it early reads to
/// the server as the client going away — the failure that presented as a join
/// that never resolved the first time the native path ran.
pub(crate) fn spawn_action_writer(
    writer: WritableStreamDefaultWriter,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
) {
    spawn_local(async move {
        // One frame's worth of scratch: the prefix and the payload go out as
        // a single chunk (`frame::encode_into`'s reason), so this is the
        // buffer that makes that one write possible without allocating per
        // action.
        let mut out = [0u8; protocol::MAX_STREAM_MSG_BYTES + super::frame::LEN_PREFIX_BYTES];
        while let Some(payload) = rx.recv().await {
            let Some(n) = super::frame::encode_into(&mut out, &payload) else {
                // A caller handed this lane more than the protocol's own
                // ceiling. Refused rather than split: the reliable lane is
                // exactly-once and half a message is not a smaller one.
                web_sys::console::error_1(&JsValue::from_str(
                    "gates: action lane refused an oversized message",
                ));
                continue;
            };
            let chunk = Uint8Array::new_with_length(n as u32);
            chunk.copy_from(&out[..n]);
            if JsFuture::from(writer.write_with_chunk(&chunk))
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = JsFuture::from(writer.close()).await;
    });
}

/// Write one framed message to the bidi stream's writable half.
///
/// The desktop twin is `lib.rs::write_frame`, and the difference is one
/// chunk versus two: wtransport takes two `write_all`s because a QUIC stream
/// is a byte stream and coalescing is the transport's business, while a
/// `WritableStream` treats each write as a queued chunk. Same bytes, from
/// [`super::frame::encode_into`], so the two cannot drift.
pub(crate) async fn write_frame(
    writer: &WritableStreamDefaultWriter,
    payload: &[u8],
) -> Result<(), String> {
    let mut out = [0u8; protocol::MAX_STREAM_MSG_BYTES + super::frame::LEN_PREFIX_BYTES];
    let n = super::frame::encode_into(&mut out, payload).ok_or_else(|| {
        format!(
            "frame: {} bytes does not fit the stream lane",
            payload.len()
        )
    })?;
    let chunk = Uint8Array::new_with_length(n as u32);
    chunk.copy_from(&out[..n]);
    JsFuture::from(writer.write_with_chunk(&chunk))
        .await
        .map(|_| ())
        .map_err(|e| js_err("stream write", &e))
}

/// Open a `WebTransport` to `server` and wait for it to be usable.
///
/// **The browser's trust posture, and why it has only two arms where
/// `client_endpoint` has three.**
///
/// | target | pin | trust |
/// |---|---|---|
/// | any | `Some` | that certificate and no other |
/// | any | `None` | the *browser's* root store |
///
/// The missing third arm is the loopback carve-out, and its absence is a
/// feature rather than a gap: a page cannot ask to skip validation, there is
/// no `with_no_cert_validation()` to reach for, and so the web client is
/// strictly safer than the desktop one on exactly the axis
/// `client_endpoint`'s doc comment spends a page worrying about. SIWE still
/// has no channel binding, and the relay attack described there is still the
/// threat — it is just that here the platform refuses it for us.
///
/// ⚠ **The consequence lands on the dev flow, not on players.** A local shard
/// self-signs, and a browser will not accept that on a bare `https://` no
/// matter what the address is. So `--cert-hash` is REQUIRED to reach a dev
/// shard from a page, where natively it is optional — and the shard has been
/// printing the digest for exactly this since the browser days
/// (`server::net::spawn_shard`; `NETCODE.md` §2.2's P-256 and 14-day rules are
/// WebTransport spec rules, written for browsers, which is why the shard
/// already satisfies them).
///
/// `require_unreliable` is set because this client cannot function without
/// datagrams: input goes out on that lane and snapshots come back on it. A
/// connection that silently lacked them would present as a player who can see
/// the world and cannot move it, which is the worst shape for the failure to
/// take.
pub(crate) async fn open(server: &str, cert_hash: Option<&str>) -> Result<WebTransport, String> {
    let url = format!("https://{server}");
    let opts = web_sys::WebTransportOptions::new();
    opts.set_require_unreliable(true);

    // Refused here, before dialling, rather than surfacing as a handshake
    // failure — `client_endpoint`'s rule and for its reason: a typo'd digest
    // and a genuinely untrusted shard must not present as the same thing.
    if let Some(text) = cert_hash.map(str::trim).filter(|h| !h.is_empty()) {
        let digest = super::parse_cert_digest(text).ok_or_else(|| {
            format!(
                "--cert-hash {text:?} is not a SHA-256 digest - expected the 32-byte \
                 dotted hex the shard prints at boot (`aa:bb:...`)"
            )
        })?;
        let bytes = Uint8Array::new_with_length(digest.len() as u32);
        bytes.copy_from(&digest);
        let hash = web_sys::WebTransportHash::new();
        hash.set_algorithm("sha-256");
        hash.set_value_u8_array(&bytes);
        opts.set_server_certificate_hashes(&[hash]);
    }

    let transport = WebTransport::new_with_options(&url, &opts)
        .map_err(|e| js_err(&format!("connect {url}"), &e))?;
    JsFuture::from(transport.ready())
        .await
        .map_err(|e| js_err("connect", &e))?;
    Ok(transport)
}

impl Drop for WebWire {
    /// Close the transport when the session goes away.
    ///
    /// The desktop side gets this for free — dropping the last `Arc<Connection>`
    /// closes the connection — and a page does not: a `WebTransport` whose Rust
    /// handle is dropped keeps running until the JS object is collected, which
    /// is neither prompt nor guaranteed. Without this, leaving a shard and
    /// joining another would hold both sessions open on the server, and the
    /// first one would keep receiving snapshots for a player nobody is drawing.
    fn drop(&mut self) {
        self.transport.close();
    }
}
