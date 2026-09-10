//! **The browser's `read_exact`, gated on a box with no browser.**
//!
//! Code tier: no `render` feature, no GPU, no socket, no wasm.
//!
//! `net::frame::FrameBuf` is the adapter between a `ReadableStream`, which
//! hands a page whatever bytes happened to arrive, and the reliable lane,
//! which is a `u16` LE length prefix followed by exactly that many bytes.
//! `findings/web-build-20260909.md` §10.6 named it *"the single largest piece
//! of real code the web transport owes"*, and the reason it is written with no
//! transport type and no `async` is precisely so this file can exist: the
//! off-by-one lives in the chunking, and the chunking can be enumerated here
//! in microseconds instead of being chased in a browser.
//!
//! **The central property is chunk-independence.** One byte stream, split
//! every way from whole-in-one-go down to one byte at a time, must yield the
//! same frames — because the split is the network's choice and nothing about
//! the protocol may depend on it. That is what `the_chunking_cannot_change_the_frames`
//! asserts, and it is the assertion the whole design exists to satisfy.
//!
//! **Proven by mutation** (2026-09-10, `CLAUDE.md`'s rule for a new gate — run
//! the mutants, because a gate that shares a code path with what it checks is
//! checking that path against itself). Four were applied to
//! `net/frame.rs` and all four redden this file:
//!
//! | mutant | what it would have shipped |
//! |---|---|
//! | `from_le_bytes` → `from_be_bytes` | every length read byte-swapped |
//! | `declared > N` → `declared >= N` | a frame of exactly the ceiling refused |
//! | `Ok((used, ..))` → `Ok((chunk.len(), ..))` | the residue silently dropped |
//! | `take()` stops resetting `hdr_len` | the next frame's length read from the wrong two bytes |
//!
//! The third is the one worth naming twice: it is the bug this whole type
//! exists to prevent, it costs nothing on a stream that happens to arrive one
//! frame per chunk, and in play it would present as a rare missing event at
//! join.

use client::net::frame::{encode_into, FrameBuf, FrameError, LEN_PREFIX_BYTES};

const N: usize = 320;

/// Encode a list of payloads into one byte stream, the way a shard's stream
/// lane looks on the wire.
fn stream(payloads: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in payloads {
        let mut scratch = vec![0u8; LEN_PREFIX_BYTES + p.len()];
        let n = encode_into(&mut scratch, p).expect("payload fits");
        out.extend_from_slice(&scratch[..n]);
    }
    out
}

/// Feed `bytes` through a `FrameBuf` in chunks of `chunk` and collect every
/// frame that comes out. This is the loop `net/web.rs::FrameReader::next`
/// runs, with the await taken out.
fn frames_at(bytes: &[u8], chunk: usize) -> Result<Vec<Vec<u8>>, FrameError> {
    let mut buf = FrameBuf::<N>::new();
    let mut out = Vec::new();
    for piece in bytes.chunks(chunk) {
        let mut at = 0;
        while at < piece.len() {
            let (used, done) = buf.push(&piece[at..])?;
            at += used;
            if done {
                out.push(buf.frame().to_vec());
                buf.take();
            }
        }
    }
    Ok(out)
}

fn sample() -> Vec<Vec<u8>> {
    vec![
        vec![7u8],           // the shortest legal frame
        (0..64u8).collect(), // an ordinary one
        vec![0xABu8; N],     // exactly the ceiling
        vec![1, 2, 3, 4, 5],
        (0..200u8).collect(),
    ]
}

/// **The property the whole type exists for.**
///
/// Red under a mutant that drops the residue after a completed frame — the
/// single most likely bug here, and the one that would present in play as a
/// rare missing event at join rather than as a crash.
#[test]
fn the_chunking_cannot_change_the_frames() {
    let payloads = sample();
    let refs: Vec<&[u8]> = payloads.iter().map(|p| p.as_slice()).collect();
    let bytes = stream(&refs);

    for chunk in 1..=bytes.len() {
        let got = frames_at(&bytes, chunk).expect("a well-formed stream");
        assert_eq!(
            got, payloads,
            "chunked at {chunk} bytes, the lane produced different frames. The split is the \
             network's choice and no protocol fact may depend on it."
        );
    }
}

/// A header split across two reads is the ordinary case, not the exotic one:
/// two bytes is the smallest thing on this lane and a datagram boundary can
/// land between them.
#[test]
fn a_header_split_across_reads_still_reads_one_length() {
    let bytes = stream(&[&[9, 8, 7][..]]);
    let mut buf = FrameBuf::<N>::new();
    let (used, done) = buf.push(&bytes[..1]).expect("half a header");
    assert_eq!((used, done), (1, false));
    let (_, done) = buf.push(&bytes[1..]).expect("the rest");
    assert!(
        done,
        "the frame completed once its second length byte arrived"
    );
    assert_eq!(buf.frame(), &[9, 8, 7]);
}

/// **Wall 4, and it is satisfied by construction rather than by a cap check.**
/// The storage is `[u8; N]`, so a declared length above the ceiling has
/// nowhere to go — it is refused at the header, before a byte is copied, and
/// the session ends. There is no truncate-and-continue arm because the
/// reliable lane drops nothing.
#[test]
fn a_frame_over_the_ceiling_is_refused_at_its_header() {
    let mut bytes = vec![0u8; LEN_PREFIX_BYTES];
    bytes[..LEN_PREFIX_BYTES].copy_from_slice(&((N + 1) as u16).to_le_bytes());
    let mut buf = FrameBuf::<N>::new();
    assert_eq!(
        buf.push(&bytes),
        Err(FrameError::TooLong {
            declared: N + 1,
            ceiling: N
        })
    );
    // And the boundary is not off by one: the ceiling itself is legal, which
    // `the_chunking_cannot_change_the_frames` also exercises through `sample`.
    let ok = stream(&[&vec![3u8; N][..]]);
    assert_eq!(frames_at(&ok, 17).expect("N is legal").len(), 1);
}

/// A zero-length frame is refused rather than delivered empty — identically to
/// the desktop reader (`lib.rs::read_frame`), because every message on this
/// lane carries a kind byte and an empty one cannot be dispatched.
#[test]
fn a_zero_length_frame_is_refused() {
    let mut buf = FrameBuf::<N>::new();
    assert_eq!(buf.push(&[0, 0]), Err(FrameError::Empty));
}

/// The drain contract, stated as behaviour: a push against an undrained frame
/// consumes nothing and says so, rather than overwriting it. That turns a
/// caller's missing `take()` into a visible stall instead of silent loss.
#[test]
fn an_undrained_frame_is_never_overwritten() {
    let bytes = stream(&[&[1, 2][..], &[3, 4][..]]);
    let mut buf = FrameBuf::<N>::new();
    let (used, done) = buf.push(&bytes).expect("first frame");
    assert!(done);
    assert_eq!(buf.frame(), &[1, 2]);
    assert_eq!(
        buf.push(&bytes[used..]).expect("refused politely"),
        (0, true),
        "a push against a complete frame must consume nothing"
    );
    assert_eq!(buf.frame(), &[1, 2], "and must not have overwritten it");
}

/// **The encoder is one function, so the two transports cannot drift.**
/// Natively the prefix and the payload go out as two `write_all`s and on the
/// web as one chunk; the bytes are `encode_into`'s either way. This pins the
/// layout the server's own `read_frame` expects — `u16` LE, then the payload,
/// nothing else.
#[test]
fn the_encoding_is_a_little_endian_u16_and_then_the_bytes() {
    let mut out = [0u8; 8];
    let n = encode_into(&mut out, &[0xDE, 0xAD, 0xBE]).expect("fits");
    assert_eq!(n, 5);
    assert_eq!(&out[..n], &[3, 0, 0xDE, 0xAD, 0xBE]);

    // Refusals, both of which are caller bugs rather than wire conditions.
    assert_eq!(
        encode_into(&mut out, &[]),
        None,
        "an empty frame has no kind"
    );
    assert_eq!(
        encode_into(&mut out[..4], &[1, 2, 3]),
        None,
        "a buffer that cannot hold the frame is refused, never truncated"
    );
}

/// **The browser's certificate pin, parsed strictly.**
///
/// A page cannot skip validation, so `--cert-hash` is the *only* way to reach
/// a self-signed dev shard from a tab (`net/web.rs::open`). That makes this
/// parser the dev flow's whole trust decision, and a lenient one is how a
/// truncated paste becomes a shorter secret.
#[test]
fn a_certificate_pin_is_thirty_two_bytes_of_dotted_hex_or_nothing() {
    use client::net::parse_cert_digest;

    let good: String = (0..32)
        .map(|i| format!("{:02x}", i as u8))
        .collect::<Vec<_>>()
        .join(":");
    let parsed = parse_cert_digest(&good).expect("32 groups of two hex digits");
    assert_eq!(parsed[0], 0);
    assert_eq!(parsed[31], 31);
    assert!(
        parse_cert_digest(&format!("  {good}  ")).is_some(),
        "a copied line carries whitespace at its ends"
    );

    // Everything short of exactly right is nothing.
    let bad = [
        "",
        "aa",
        &good[..good.len() - 3],       // 31 groups
        &format!("{good}:ff"),         // 33 groups
        &good.replace(':', ""),        // no separators
        &good.replacen("00", "0", 1),  // a one-digit group
        &good.replacen("00", "+f", 1), // `from_str_radix` accepts `+`
        &good.replacen("00", "gg", 1), // not hex
        &good.replacen(':', "::", 1),  // an empty group
    ];
    for case in bad {
        assert!(
            parse_cert_digest(case).is_none(),
            "a pin is a security control: {case:?} must not parse"
        );
    }
}

/// **The client encodes a length prefix in exactly one place.**
///
/// `src/lib.rs`'s `write_frame` hand-rolled its own `to_le_bytes` pair until
/// 2026-09-10 while two doc comments already claimed the layout was
/// byte-identical across the two transports — three copies agreeing by
/// inspection, described as an enforcement. It calls `encode_into` now, and
/// this is what keeps that true.
///
/// **State the limit, because a proxy that reads as a proof is worse than no
/// gate.** This is a file-scoped scan for one spelling. It is green under
/// `to_ne_bytes`, under a hand-rolled `[n as u8, (n >> 8) as u8]`, and under a
/// second writer added to `net/native.rs`, which is outside its scope. What
/// would actually hold the invariant is lifting the encoder into `protocol`
/// beside the rest of the wire and gating the call sites — `frame.rs`'s own
/// header says that is the signal to watch for, and a third copy appearing is
/// it. Until then this catches the one regression anybody is likely to make:
/// re-inlining the prefix at the site that had it.
#[test]
fn the_length_prefix_is_encoded_in_one_place() {
    let path = std::path::Path::new("src/lib.rs");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let code: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("to_le_bytes"),
        "src/lib.rs encodes a length prefix of its own. The stream lane's framing is \
         `net::frame::encode_into` on both transports — that is what makes the two \
         `write_frame`s provably the same bytes rather than the same by inspection, and \
         it is what refuses an empty frame and a payload past 65,535 instead of \
         truncating it into a permanent desync."
    );
}
