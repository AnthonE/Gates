//! The stream lane's framing, as a state machine over arbitrary chunks.
//!
//! **Why this exists, and why it is pure.** The reliable lane is a `u16` LE
//! length prefix followed by that many bytes — byte-identical on both sides
//! (`server::net::{read_frame, write_frame}`) and, natively, read with
//! `read_exact`, which wtransport gives us for free. **A browser has no
//! `read_exact`.** `ReadableStreamDefaultReader::read()` resolves to whatever
//! the transport happened to have: half a header, three frames and a tail, a
//! single byte. `findings/web-build-20260909.md` §10.6 named the buffering
//! adapter that closes that gap as *"the single largest piece of real code the
//! web transport owes"*, and this is it.
//!
//! It is written with **no transport type, no `async` and no target cfg** so
//! that the hard part — the part with an off-by-one in it — is testable on the
//! box you are sitting at rather than only in a browser. `tests/framing.rs` is
//! the gate, and it works by feeding one byte stream in every chunking from
//! whole-in-one-go down to one byte at a time and requiring the same frames
//! out of all of them.
//!
//! **Wall 4 is satisfied by construction rather than by a cap check.** The
//! storage is `[u8; 2]` plus `[u8; N]` — fixed at compile time, sized by the
//! caller's own `MAX_*_MSG_BYTES` — so there is nowhere for a hostile stream
//! to grow. A declared length above `N` cannot be stored and is refused as
//! [`FrameError::TooLong`]; the session ends. That is the stated overflow
//! policy: the reliable lane drops nothing, so a frame it cannot hold is a
//! protocol violation and not a value to skip.

/// The length prefix, in bytes. `u16` LE, and the reason `N` above is a
/// `usize` the caller states rather than 65,535: the protocol's own ceilings
/// (`MAX_STREAM_MSG_BYTES` = 128, `MAX_EVENT_MSG_BYTES` = 320) are far below
/// what the prefix can express, and the smaller number is the one worth
/// enforcing.
pub const LEN_PREFIX_BYTES: usize = 2;

/// Why a stream cannot be read any further. Both arms end the session: this
/// is the *reliable* lane, where nothing is droppable, so a frame that cannot
/// be delivered is a desync and not a hiccup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The peer declared a frame longer than this lane's ceiling. Never a
    /// recoverable state — the bytes after it cannot be located without
    /// trusting the length that was just refused.
    TooLong { declared: usize, ceiling: usize },
    /// A zero-length frame. The native reader refuses it identically
    /// (`lib.rs::read_frame`), and it must stay refused rather than becoming
    /// an empty delivery: every message this lane carries has a kind byte.
    Empty,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooLong { declared, ceiling } => {
                write!(
                    f,
                    "stream frame declared {declared} bytes over a {ceiling}-byte lane"
                )
            }
            FrameError::Empty => write!(f, "stream frame declared zero bytes"),
        }
    }
}

/// Reassembles length-prefixed frames of at most `N` payload bytes from
/// chunks of any size.
///
/// One frame at a time on purpose: [`push`](Self::push) stops the moment a
/// frame is complete and tells the caller how much of the chunk it used, so
/// the caller delivers that frame before the next byte is looked at. The
/// alternative — buffering every frame in a chunk and handing back a list —
/// would need a queue, and a queue would need a cap, and the cap is the thing
/// this design does not have to argue about.
pub struct FrameBuf<const N: usize> {
    hdr: [u8; LEN_PREFIX_BYTES],
    hdr_len: usize,
    body: [u8; N],
    body_len: usize,
    /// The declared payload length, once the header is whole. `None` means
    /// the header is still arriving.
    want: Option<usize>,
}

impl<const N: usize> Default for FrameBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FrameBuf<N> {
    pub const fn new() -> Self {
        Self {
            hdr: [0; LEN_PREFIX_BYTES],
            hdr_len: 0,
            body: [0; N],
            body_len: 0,
            want: None,
        }
    }

    /// Whether a whole frame is sitting in [`frame`](Self::frame) waiting to
    /// be [`take`](Self::take)n.
    pub fn complete(&self) -> bool {
        matches!(self.want, Some(w) if self.body_len == w)
    }

    /// The assembled payload. Only meaningful while [`complete`](Self::complete);
    /// a partial frame reads as the bytes that have arrived so far, which is
    /// why the caller is handed `bool` by `push` rather than being trusted to
    /// ask.
    pub fn frame(&self) -> &[u8] {
        &self.body[..self.body_len]
    }

    /// Finish with the current frame and arm for the next one. Returns how
    /// many payload bytes it held, so a caller that has already copied out of
    /// [`frame`](Self::frame) can assert on the same number.
    pub fn take(&mut self) -> usize {
        let len = self.body_len;
        self.hdr_len = 0;
        self.body_len = 0;
        self.want = None;
        len
    }

    /// Consume bytes from the front of `chunk` until one whole frame is
    /// assembled or the chunk runs out.
    ///
    /// Returns `(consumed, complete)`. **The caller must call
    /// [`take`](Self::take) when `complete` is true before pushing again** —
    /// a push against a complete frame consumes nothing and answers
    /// `(0, true)` rather than overwriting it, which turns "I forgot to
    /// drain" into a stall the caller can see instead of silent data loss.
    ///
    /// Progress is guaranteed for the loop that reads this: with a non-empty
    /// chunk and no complete frame pending, at least one byte is always
    /// consumed. There is no third case in which it returns `(0, false)`.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(usize, bool), FrameError> {
        if self.complete() {
            return Ok((0, true));
        }
        let mut used = 0;

        if self.want.is_none() {
            let need = LEN_PREFIX_BYTES - self.hdr_len;
            let take = need.min(chunk.len());
            self.hdr[self.hdr_len..self.hdr_len + take].copy_from_slice(&chunk[..take]);
            self.hdr_len += take;
            used += take;
            if self.hdr_len < LEN_PREFIX_BYTES {
                return Ok((used, false));
            }
            let declared = u16::from_le_bytes(self.hdr) as usize;
            // Refused here, at the header, rather than after N bytes have
            // been copied somewhere: the same order `lib.rs::read_frame`
            // checks in, and the only order in which the refusal costs
            // nothing.
            if declared == 0 {
                return Err(FrameError::Empty);
            }
            if declared > N {
                return Err(FrameError::TooLong {
                    declared,
                    ceiling: N,
                });
            }
            self.want = Some(declared);
        }

        let want = self.want.expect("set immediately above or on a prior push");
        let need = want - self.body_len;
        let take = need.min(chunk.len() - used);
        self.body[self.body_len..self.body_len + take].copy_from_slice(&chunk[used..used + take]);
        self.body_len += take;
        used += take;
        Ok((used, self.body_len == want))
    }
}

/// Write one frame's bytes into `out`, answering how many it wrote.
///
/// The encoder half, shared for the same reason the decoder is: natively
/// `write_frame` puts the prefix and the payload on the wire as two
/// `write_all`s, and a browser writer wants **one** `Uint8Array` — two writes
/// on a `WritableStream` is two chunks and a needless round through the
/// queue. Same bytes either way, and now that is a property of one function
/// rather than of two that happen to agree.
///
/// `None` when the payload does not fit, which is a caller bug rather than a
/// wire condition: every encoder in `protocol` bounds its own output.
pub fn encode_into(out: &mut [u8], payload: &[u8]) -> Option<usize> {
    let total = LEN_PREFIX_BYTES + payload.len();
    if payload.is_empty() || payload.len() > u16::MAX as usize || out.len() < total {
        return None;
    }
    out[..LEN_PREFIX_BYTES].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[LEN_PREFIX_BYTES..total].copy_from_slice(payload);
    Some(total)
}
