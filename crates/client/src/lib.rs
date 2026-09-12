//! The native client's session: one wtransport connection, the handshake,
//! and the pump that drives `client_core::ClientCore`.
//!
//! This crate exists because the client is moving off the browser
//! (DECISIONS.md 2026-08-05). What it deliberately does NOT do is
//! reimplement the client: `ClientCore`, the predictor, the interpolator
//! and the clock are the same code the browser build runs, and they were
//! always pure — `client-core` depends on nothing but `sim-core` and
//! `protocol`, and already builds as an rlib. The browser-shaped half is
//! `bridge.rs`'s raw C ABI, and that is exactly the half a native client
//! does not need.
//!
//! The wire is unchanged and no server change is owed. `wtransport` has a
//! client side, so this speaks the identical transport the browser speaks,
//! against the same shard, at the same `PROTO_VER`.
//!
//! **And since 2026-09-10 there are two builds of it** (operator, 2026-09-09:
//! a browser client; `findings/web-build-20260909.md`). The sentence above
//! turned out to be load-bearing rather than incidental — "the identical
//! transport the browser speaks" was written about wtransport and is now
//! literally true of a `WebTransport` object in a tab. What splits is this
//! file and nothing else in the crate: `client_endpoint` and the wtransport
//! half of `connect` are `#[cfg(feature = "native")]`, the browser's
//! `connect` sits beside them under `#[cfg(target_arch = "wasm32")]`, and
//! everything from `send_action` down — the lanes, the pump, the core — is
//! shared source with no cfg on it at all.
//!
//! Read the two `connect`s as a pair. They are deliberately the same
//! sequence in the same order, they call the same three pure functions out of
//! [`net::handshake`], and they end by building the same struct; a change to
//! one that is not made to the other is a client that can join from a desktop
//! and not from a page, or the reverse.
//!
//! ⚠ **This paragraph used to end "and no gate can see that, because each
//! target only ever compiles one of them", and the second half is true while
//! the first is false.** No *compile* can see it — `cargo test --workspace`
//! never builds the wasm arm at all, since `client-web` is empty natively —
//! but a source scan compiles nothing and reads both arms on any target.
//! `crates/client/tests/connect_twins.rs` is that scan, and it was written the
//! day after this sentence was, having stopped somebody from looking for
//! exactly one day. A plausible impossibility claim costs more than an
//! unmentioned gap.
//!
//! What it checks is what rustc cannot: the handshake sequence, the lane
//! depths, the frame ceilings, and the VALUES the session literal gives each
//! field. The field SET needs no gate — `Session` is one non-cfg'd struct, so
//! a missing field is `E0063` on that arm's own build.

pub mod args;
// The settings file: path, format, version, and the unknown-key policy. NOT
// feature-gated, for `ui`'s reason exactly: parse and serialize are pure, and
// a test behind `--features render` runs in the renderer tier where nobody
// looks at it. `render/settings.rs` is the Bevy half (load once, save on
// change). The server-side precedent is `server/src/config.rs`.
pub mod config;
// Which way is right. Pure and unconditional for the same reason `ui` is:
// the mapping from a keypress to a wire axis is arithmetic, it was WRONG
// (see the module header), and a mapping that lives inside a Bevy system is
// one no test in the code tier can call.
pub mod look;
// The launcher's title manifest: where this game's NEWS, ITEM STORE and
// WORKSHOP live. Unconditional for `shardlist`'s reason exactly — it parses
// bytes off a network from a host named by another host, and one of the
// values in it is handed to the launcher to OPEN on the player's desktop, so
// the scheme allowlist is the part that most needs a test in the code tier.
pub mod manifest;
// Join links — `elo://join/gates/host:port`. Unconditional for the reason
// `shardlist` is: it parses a string a stranger wrote and handed to a friend,
// which is the input in this client that most needs a test in the code tier.
pub mod deeplink;
// Discord rich presence: the local IPC socket, the frames, and the copy.
// Unconditional for `config`'s reason exactly — the framing is two integers
// and the payloads are four fixed shapes, all of it pure, and a test behind
// `--features render` runs in the renderer tier where nobody looks at it.
// `render/presence.rs` is the Bevy half (the `Screen` mapping and the
// once-per-change handoff). Dark unless `GATES_DISCORD_APP_ID` is set.
pub mod discord;
pub mod elo;
pub mod shardlist;
// Where a player's screenshot goes and what it is called. Unconditional for
// `config`'s reason exactly: a path rule and a filename rule are pure, they
// are the part that can be wrong on a box nobody here owns (three platforms,
// two Linux conventions), and a test behind `--features render` runs in the
// renderer tier where nobody looks at it. `render/shot.rs` is the Bevy half.
pub mod shot;
// What a player's bug report IS — the document, its bounds, and the rule that
// a stranger's prose is evidence rather than instructions. Unconditional for
// `shot`'s reason exactly: every rule in it is arithmetic over strings, it is
// the half that has to be right on a box nobody here owns, and the untrusted
// input it bounds is the one input in this crate a stranger writes.
// `render/report.rs` is the Bevy half (one keypress, the live facts, one file).
pub mod report;
// The audio model — which sound, how loud, how many at once, and the
// generated bank itself. NOT feature-gated, for `ui`'s reason exactly: it is
// pure, a mixer is where "Bevy plays, it does not decide" is easiest to break,
// and a test behind `--features render` runs in the renderer tier where nobody
// looks at it. `render/audio.rs` is the Bevy half.
pub mod sound;
// The in-game menus' arithmetic. NOT feature-gated: it is pure, it is what
// the menus actually get wrong, and a test behind `--features render` runs
// in the renderer tier where nobody looks at it (`ui/mod.rs`).
pub mod ui;

// The transport seam (`findings/web-build-20260909.md` §10.3). Unconditional
// for `ui`'s reason exactly: what a connected session asks of a transport is
// one synchronous call, the lane plumbing under it is pure, and both are the
// half a browser build has to re-express — so they belong where every target
// compiles them rather than inside the native session below.
pub mod net;

// The render path. Feature-gated because Bevy is several hundred crates and
// the code tier must not pay for it (`crates/client/Cargo.toml`).
#[cfg(feature = "render")]
pub mod render;

// **Exactly one transport, proved rather than assumed.**
//
// `net::ActiveWire` has two arms and they are chosen by different mechanisms —
// a cargo feature for the desktop one (because `wtransport` is an optional
// dependency) and a target for the browser one (because a page's transport is
// not a flag). Two mechanisms that must agree is a convention, and `CLAUDE.md`
// says a law without a gate is a mood, so here is the gate: it is a
// `compile_error!` rather than a test because the failure it prevents is a
// build that does not exist, and there is nothing to run.
//
// The first arm is the one somebody will actually hit: `cargo build -p client
// --target wasm32-unknown-unknown` with the default features still on. Without
// this, that build spends two minutes and dies in 48 errors inside `mio`, a
// crate we never asked for (it arrives through quinn) — the exact diagnosis
// `findings/web-build-20260909.md` §10.2 had to be written to correct.
#[cfg(all(feature = "native", target_arch = "wasm32"))]
compile_error!(
    "the `native` feature cannot be built for wasm32: it carries wtransport, and quinn owns a \
     UDP socket a page does not have. Build the browser client with \
     `--no-default-features` (see findings/web-build-20260909.md)."
);
#[cfg(not(any(feature = "native", target_arch = "wasm32")))]
compile_error!(
    "this build of `client` has no transport: the `native` feature is off and the target is not \
     wasm32, so `net::ActiveWire` names nothing. Either build for wasm32 or leave the default \
     features on."
);
// The third of the set, and it exists because the failure is otherwise
// illegible. `hot = ["render", "bevy/file_watcher"]` and `bevy_asset`'s file
// watcher is itself a `compile_error!` on wasm32 — so the mistake lands as an
// error inside a dependency, about a feature the author did not name, rather
// than here about the one they did. A dev-loop feature asking to watch a
// filesystem a page does not have is worth one sentence at the door.
#[cfg(all(feature = "hot", target_arch = "wasm32"))]
compile_error!(
    "the `hot` feature cannot be built for wasm32: it is asset hot-reload, which watches a \
     filesystem a page does not have. Build the browser client without it."
);

use client_core::core::{ClientCore, Ingest};
// Moved to `net` on 2026-09-09, re-exported rather than relocated in the
// public API: `SendError` is what `send_action` returns and renaming its path
// would be a breaking change for a refactor that is meant to be invisible.
pub use net::SendError;
// **The join's failure, carrying the shard's own refusal CODE.** Re-exported
// at the crate root beside `SendError` for the same reason: it is what a
// public method returns, and a caller should not have to name a private
// module's path to match on it.
pub use net::handshake::{refusal_sentence, JoinError, LAUNCHER_REACHABLE};
use net::{datagram_lane, drain_datagram, drain_lane, DatagramRx, Wire};
// **Eight names left this list when the handshake moved to `net::handshake`**
// — every decoder and every message kind. What stays is the two frame-size
// bounds, which belong to the framing this file still owns, and `Welcome`,
// which is a field on `Session`. That the import list shrank to exactly the
// I/O half is the evidence the extraction was complete rather than partial.
use protocol::{Welcome, MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES};
use sim_core::limits::{ACTION_RING_CAP, DATAGRAM_BUDGET_BYTES};
#[cfg(feature = "native")]
use wtransport::config::IpBindConfig;
#[cfg(feature = "native")]
use wtransport::endpoint::endpoint_side::Client;
#[cfg(feature = "native")]
use wtransport::{ClientConfig, Endpoint, RecvStream, SendStream};

/// Stream framing: `u16` LE length prefix per message. Byte-identical to
/// `server::net::{read_frame, write_frame}` and to `web/src/net.js`, and
/// reimplemented here rather than imported because the client must never
/// depend on the `server` crate — that would ship the authoritative sim
/// inside the client binary. If a third copy ever appears, that is the
/// signal to lift this into `protocol` where the rest of the wire lives.
#[cfg(feature = "native")]
async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), String> {
    // **One encoder for both transports, which the comment above claimed
    // before it was true.** This wrote its own `to_le_bytes` prefix until
    // 2026-09-10, so the doc line saying the layout is "byte-identical to
    // `server::net::write_frame` and to the browser's" rested on three copies
    // agreeing by inspection — `CLAUDE.md`'s ⚠ about a doc that reads as
    // enforced while nothing checks.
    //
    // It also silently accepted two payloads the browser refuses, and the
    // second is the expensive one: an empty frame went out as `[0, 0]`, and a
    // payload past 65,535 was TRUNCATED by `as u16` into a length prefix that
    // does not describe it — a permanent stream desync rather than a dropped
    // message. `encode_into` refuses both.
    //
    // One `write_all` rather than two: a QUIC stream is a byte stream, so
    // coalescing was always the transport's business, and the browser's
    // `WritableStream` counts each write as a queued chunk. Same bytes on the
    // wire either way; one fewer syscall per action.
    let mut out = [0u8; MAX_STREAM_MSG_BYTES + net::frame::LEN_PREFIX_BYTES];
    let n = net::frame::encode_into(&mut out, payload).ok_or_else(|| {
        format!(
            "frame: {} bytes does not fit the stream lane",
            payload.len()
        )
    })?;
    send.write_all(&out[..n]).await.map_err(|e| e.to_string())
}

/// **The browser has no counterpart to this function and that is the whole
/// asymmetry of the port.** `read_exact` is what a QUIC stream API gives you
/// and a `ReadableStreamDefaultReader` does not: it yields whatever arrived,
/// so the same job over there is a state machine that carries its own residue
/// ([`net::frame::FrameBuf`], pure and gated in `tests/framing.rs`). The two
/// read the same bytes off the same wire; only one of them gets to be four
/// lines.
#[cfg(feature = "native")]
async fn read_frame<const N: usize>(recv: &mut RecvStream) -> Option<([u8; N], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > N {
        return None;
    }
    let mut buf = [0u8; N];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

/// Whether `server` names this machine's own loopback — the one address at
/// which [`client_endpoint`] deliberately skips certificate validation.
///
/// The split mirrors `shardlist::check_addr`'s rule exactly (bracketed IPv6
/// literal, otherwise the LAST colon) and is repeated rather than shared
/// because that function answers a different question — whether an address is
/// *well shaped* — and one of the two must keep working if the other is
/// relaxed. Anything this cannot read is NOT loopback, which is the safe
/// direction: an unparseable host falls through to validation.
///
/// `localhost` counts. An `/etc/hosts` that points it elsewhere is an
/// attacker who already writes files on this box, which is the same person
/// the carve-out below already concedes.
pub fn is_loopback_host(server: &str) -> bool {
    let server = server.trim();
    let host = if let Some(rest) = server.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, _)) => h,
            None => return false,
        }
    } else {
        match server.rsplit_once(':') {
            // A bare IPv6 literal has colons of its own and is not a shape
            // this client accepts at all (`check_addr` refuses it) — so it is
            // not loopback here either, rather than being half-parsed.
            Some((h, _)) if h.contains(':') => return false,
            Some((h, _)) => h,
            None => server,
        }
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The client's transport endpoint, and **the only thing in this stack that
/// asks who the far end is.**
///
/// Three postures, chosen from the address being dialled and the operator's
/// `--cert-hash` (operator, `DECISIONS.md` 2026-08-10):
///
/// | target | pin | trust |
/// |---|---|---|
/// | any | `Some` | that certificate and no other |
/// | loopback | `None` | **anything** — the carve-out |
/// | anything else | `None` | the platform's root store |
///
/// **Why this is not cosmetic, and what it costs to get wrong.** This used
/// to be `with_no_cert_validation()` unconditionally, at all three call
/// sites across both player binaries: `client` (`main.rs`), and the `gates`
/// that `ci/depot.py` ships, which dials from two places — `--server` at
/// boot (`bin/gates.rs`) and the shard picked out of the list
/// (`render/menu.rs`). SIWE has **no channel
/// binding**: the handshake proves the joiner holds the key behind an
/// address and says nothing about which TLS connection that proof arrived
/// on. So an on-path relay terminating the player's QUIC and opening its own
/// to the shard forwards the challenge, forwards the signature, and is
/// admitted as the victim — a live session hijack with the key never leaving
/// the wallet and the nonce perfectly fresh. There is no credential theft to
/// notice afterwards, which is exactly why nothing else in the stack refuses
/// it. `crates/server/tests/tls_posture.rs` gates the three postures — and
/// `crates/client/tests/tls_callsite.rs` gates the *argument*, because a
/// posture chosen from an address is only as good as the address it is
/// asked about, and `tls_posture` stays 4/4 green over a call site pointed
/// at loopback while the real one is dialled.
///
/// **Why loopback stays permissive.** An attacker sitting between two
/// sockets on this machine is already executing code on this machine, so the
/// carve-out costs nothing against the threat above — and it is what keeps
/// every local dev flow working with no flag, including the capture harness,
/// whose invocation lives outside this repo and cannot be edited from
/// inside it. `args::DEFAULT_SERVER` is `127.0.0.1:4433`, so the bare
/// `gates` a developer types is the carved-out case by construction.
///
/// **Why a hash and not a private CA for the dev path.** The shard already
/// self-signs a P-256 certificate with a 14-day validity and already prints
/// its SHA-256 (`server::net::spawn_shard`), and those two properties are
/// precisely what wtransport's `with_server_certificate_hashes` requires of
/// a certificate before it will pin one. So the number the shard has printed
/// since the browser days — vestigial for a year, since nothing read it —
/// becomes the dev path, and no second trust store has to exist.
#[cfg(feature = "native")]
pub fn client_endpoint(server: &str, cert_hash: Option<&str>) -> Result<Endpoint<Client>, String> {
    // Refused here, at endpoint construction, rather than surfacing as a
    // handshake failure ten seconds later: a typo'd digest and a genuinely
    // untrusted shard must not present as the same thing to the operator.
    let pin = match cert_hash.map(str::trim).filter(|h| !h.is_empty()) {
        None => None,
        Some(h) => Some(h.parse::<wtransport::tls::Sha256Digest>().map_err(|_| {
            format!(
                "--cert-hash {h:?} is not a SHA-256 digest — expected the 32-byte \
                 dotted hex the shard prints at boot (`aa:bb:...`)"
            )
        })?),
    };
    let loopback = is_loopback_host(server);

    // **Bind IPv4 first, and fall back to the dual-stack default.**
    // `with_bind_default()` is `INADDR_ANY` dual-stack, which fails outright
    // on a container with no IPv6 — `Address family not supported by protocol
    // (os error 97)`, the exact failure `CLAUDE.md` records for `bot_smoke`
    // on such a box. That was diagnosed there as a missing capability rather
    // than a defect, and it is; but a client that cannot bind cannot draw,
    // and every shard we run and every gate we write is reachable over v4.
    // So: ask for what works, keep the dual-stack path for a v6 shard, and
    // report both failures if neither binds.
    //
    // Confirmed against the packaged desktop build on 2026-08-05: the depot
    // the elo launcher installed died here at startup, before any address
    // was parsed, on a container with IPv6 off.
    let build = |ip: IpBindConfig| {
        let roots = ClientConfig::builder().with_bind_config(ip);
        let cfg = match (pin.clone(), loopback) {
            (Some(digest), _) => roots.with_server_certificate_hashes([digest]).build(),
            (None, true) => roots.with_no_cert_validation().build(),
            (None, false) => roots.with_native_certs().build(),
        };
        Endpoint::client(cfg)
    };
    match build(IpBindConfig::InAddrAnyV4) {
        Ok(e) => Ok(e),
        Err(v4) => build(IpBindConfig::InAddrAnyDual)
            .map_err(|dual| format!("client endpoint: v4 {v4}; dual-stack {dual}")),
    }
}

/// What the session hands a renderer each frame. Deliberately small: the
/// renderer reads the core, it does not own it.
pub struct Frame {
    pub tick: u32,
    pub snapshots: u64,
}

/// One connected session. Owns the core and the connection; a renderer
/// drives it by calling `pump` and reading `core`.
pub struct Session {
    pub core: ClientCore,
    /// Every `APPLIED*` bit `on_stream` raised since the last drain, OR'd.
    ///
    /// **This word used to be discarded** (`let _ = self.core.on_stream(..)`),
    /// and that single `_` is why five decoded facts had no readers: several
    /// of `ClientCore`'s own-facts are LATCHED fields rather than rings —
    /// `struct_hit`, `charge_placed`, `stock` — so the only thing that says
    /// "this one is fresh this frame" is the bit `on_stream` returns. Without
    /// it a reader either redraws the last hit forever or never learns of the
    /// first. Rings do not need this (they empty themselves); latched fields
    /// cannot live without it.
    ///
    /// Accumulated rather than replaced, because `pump` drains the lane in a
    /// `while` loop and two messages in one frame is ordinary — taking only
    /// the last one's word would drop the earlier fact silently.
    pub applied: u32,
    /// The same, for word 1 (`APPLIED2_*`).
    ///
    /// Word 0 is full — bit 30 is the last flag and bit 31 is the C ABI's
    /// error sentinel sharing the return — so newer facts land here.
    /// Accumulated for a second reason on top of `applied`'s: `applied2()` is
    /// documented valid only until the next `on_stream`, so a frame that
    /// drained two messages would keep only the second one's word.
    pub applied2: u32,
    pub welcome: Welcome,
    /// The transport's send half — the ONE thing in a connected session that
    /// a browser cannot share (`net`'s header has the measurement). A
    /// cfg-selected concrete type, so this struct names no transport and
    /// needs no cfg of its own.
    wire: net::ActiveWire,
    /// The C→S half of the bidi stream, held for the life of the session by
    /// a writer task rather than by this struct.
    ///
    /// **Holding it is load-bearing.** Dropping it finishes that direction,
    /// and the server reads the close as the client going away — the join
    /// never resolves to a world slot and `snap sent` stays 0 while inputs
    /// still flow, which is exactly how this presented the first time it
    /// ran. So the task owns it and lives exactly as long as this sender
    /// does; dropping the `Session` closes the channel, the task returns,
    /// and the stream finishes then and not before.
    ///
    /// **Why a task at all.** Writing is `async` and the menus are not: a
    /// panel sends an action from inside a frame, and a frame that awaits
    /// the network is a dropped frame (`CLAUDE.md`: the client is a hot
    /// path too). Handing the write to a task makes [`Session::send_action`]
    /// synchronous and non-blocking, which is what a UI system can call.
    actions: tokio::sync::mpsc::Sender<Vec<u8>>,
    events: tokio::sync::mpsc::Receiver<Vec<u8>>,
    /// The unreliable lane — a bounded drop-oldest ring since netcode v2
    /// S2; see [`datagram_lane`] for the policy and what the watch it
    /// replaced cost the interpolator.
    datagrams: DatagramRx,
    snapshots: u64,
    input_buf: [u8; DATAGRAM_BUDGET_BYTES],
    /// Reused landing buffers the ring's payloads are swapped into, so the
    /// lock is released before any decode runs — see [`drain_datagram`].
    dg_scratch: Vec<Vec<u8>>,
    /// The shard hung up. Latched by [`Session::pump`] when either receive
    /// lane's reader task ends — the event task on a closed or desynced
    /// stream, the datagram task on a dead connection — because a reader
    /// hanging up is the one fact both failure shapes share. Sticky on
    /// purpose: a connection does not come back, and a flag that cleared
    /// itself would let one hopeful frame un-say it.
    closed: bool,
}

/// The desktop half of the session: connecting.
///
/// Split into its own `impl` block rather than cfg'd method-by-method so
/// that the browser twin below reads as a peer of it — two whole join
/// sequences side by side, each complete, rather than one function with
/// arms in it. They must stay in step; a diff that touches one and not the
/// other is the failure this arrangement is trying to make visible.
#[cfg(feature = "native")]
impl Session {
    /// Connect, handshake, and start the event-lane reader.
    ///
    /// `server` is `host:port` and is deliberately NOT resolved first.
    /// `wtransport` resolves a domain itself and then uses the *unresolved*
    /// name as the TLS server name — so handing it a `SocketAddr` throws
    /// away the one piece of information a certificate is checked against.
    /// The public shard is reached by the name its cert is issued for
    /// (`shard-public.toml`), which is why the shard list carries names too.
    /// `address` is who the player claims to be ([`Address::GUEST`] for a
    /// guest), and `sign` is asked to sign the shard's SIWE challenge with
    /// the key behind it — in practice a call into the elo launcher, which
    /// holds the key and shows the player a consent prompt. Returning `None`
    /// connects as a guest, which is what a declined prompt or an absent
    /// launcher should do rather than failing the connection.
    pub async fn connect(
        endpoint: &Endpoint<Client>,
        server: &str,
        address: protocol::Address,
        // `(domain, nonce hex, issued_at)` — the three inert values the
        // launcher needs, never a message this process composed. See the
        // `prove` block below for why that inversion is the whole fix.
        sign: impl FnOnce(&str, &str, u64) -> Option<protocol::Signature>,
    ) -> Result<Self, JoinError> {
        let url = format!("https://{server}");
        let connection = endpoint
            .connect(&url)
            .await
            .map_err(|e| format!("connect: {e}"))?;

        let opening = connection
            .open_bi()
            .await
            .map_err(|e| format!("open_bi: {e}"))?;
        let (mut send, mut recv) = opening.await.map_err(|e| format!("open_bi await: {e}"))?;

        // **The handshake's rules are in `net::handshake` and its SEQUENCE is
        // here**, which is the split that lets a browser share the first
        // without inheriting wtransport with it. Six lines below touch the
        // transport; everything they carry is pure and testable with no
        // socket (`net::handshake`'s header has the measurement).
        let mut msg = [0u8; MAX_STREAM_MSG_BYTES];
        let len = net::handshake::hello(&mut msg)?;
        write_frame(&mut send, &msg[..len]).await?;

        // The challenge: a nonce this shard chose for this connection.
        let (reply, reply_len) = read_frame::<MAX_STREAM_MSG_BYTES>(&mut recv)
            .await
            .ok_or_else(|| "no challenge".to_string())?;
        let len = net::handshake::auth_for(&reply[..reply_len], server, address, sign, &mut msg)?;
        write_frame(&mut send, &msg[..len]).await?;

        let (reply, reply_len) = read_frame::<MAX_STREAM_MSG_BYTES>(&mut recv)
            .await
            .ok_or_else(|| "no handshake reply".to_string())?;
        let welcome = net::handshake::welcome_from(&reply[..reply_len])?;

        // The event lane reads on its own task. NOT in the select! below:
        // a cancelled read drops a half-read frame and desyncs the stream
        // for good — the trap `server::botclient` documents.
        let (tx, events) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        tokio::spawn(async move {
            while let Some((buf, len)) = read_frame::<MAX_EVENT_MSG_BYTES>(&mut recv).await {
                if tx.send(buf[..len].to_vec()).await.is_err() {
                    return;
                }
            }
        });

        let connection = std::sync::Arc::new(connection);

        // Datagrams get their own task for the same reason the events do:
        // the pump must never await the network mid-frame. The ring's
        // push cannot await either — a full ring drops its oldest and
        // counts it (`datagram_lane`), never backpressures.
        let datagrams = datagram_lane();
        let dg_ring = datagrams.clone();
        let dg_conn = connection.clone();
        tokio::spawn(async move {
            while let Ok(dgram) = dg_conn.receive_datagram().await {
                let Ok(mut r) = dg_ring.lock() else { return };
                r.push(&dgram);
            }
            if let Ok(mut r) = dg_ring.lock() {
                r.closed = true;
            }
        });

        // The action lane's writer. Bounded at `ACTION_RING_CAP` — the same
        // depth the server's own action ring runs at, so the client cannot
        // hold more in flight than the sim will accept in a burst — and
        // wall 4's stated overflow policy is that a full queue is REPORTED
        // to the caller, never dropped: nothing on the reliable lane is
        // dropped, so a panel that cannot enqueue must say so rather than
        // draw a move that was never sent.
        let (act_tx, mut act_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(ACTION_RING_CAP);
        tokio::spawn(async move {
            while let Some(payload) = act_rx.recv().await {
                if write_frame(&mut send, &payload).await.is_err() {
                    return;
                }
            }
            // The channel closed: the session is going away. `send` drops
            // here, which finishes the C→S direction — the one moment that
            // is the correct thing to do.
        });

        let core = ClientCore::new(welcome.seed, welcome.player_id, welcome.tick);
        Ok(Self {
            core,
            applied: 0,
            applied2: 0,
            welcome,
            wire: net::native::NativeWire::new(connection),
            actions: act_tx,
            events,
            datagrams,
            snapshots: 0,
            input_buf: [0u8; DATAGRAM_BUDGET_BYTES],
            dg_scratch: (0..sim_core::limits::CLIENT_DG_RING)
                .map(|_| Vec::with_capacity(DATAGRAM_BUDGET_BYTES))
                .collect(),
            closed: false,
        })
    }
}

/// The browser half of the session: connecting.
///
/// **Read this against the `impl` above, not on its own.** It is the same
/// sequence — open, open a bidi stream, hello, challenge, auth, welcome,
/// three tasks, build the struct — driving the same three pure functions out
/// of [`net::handshake`], and it ends by constructing the same fields in the
/// same order. Every difference below is a difference the platform forced,
/// and each one is commented where it happens.
///
/// The signature differs in one place and it is not cosmetic: there is no
/// `endpoint` parameter, because a page has no endpoint to build. Trust is
/// decided inside [`net::web::open`] from `cert_hash` and the browser's own
/// root store, which is why `crates/client/tests/tls_callsite.rs` — a gate
/// whose whole subject is that `client_endpoint`'s address is the address
/// `connect` then dials — has nothing to say about this path. There are not
/// two addresses here to disagree.
#[cfg(target_arch = "wasm32")]
impl Session {
    pub async fn connect(
        server: &str,
        cert_hash: Option<&str>,
        address: protocol::Address,
        // **The page's wallet, or `None` for a guest.** `(text) => Promise<0x…>`
        // — see `elo::sign_siwe_web`, which is the only thing that calls it.
        //
        // The desktop twin takes the three inert values and lets the LAUNCHER
        // compose the sentence, which is what stops a game smuggling text into
        // a signature. `window.ethereum` has no verb for that: a wallet signs
        // text or nothing. So the text is composed on our side, in Rust, by
        // the same `protocol::siwe_message` the shard rebuilds it with — and
        // the wallet renders it for the human, which is a page's version of
        // the same guarantee. `elo::sign_siwe_web` argues it in full.
        sign: Option<&js_sys::Function>,
    ) -> Result<Self, JoinError> {
        use wasm_bindgen_futures::{spawn_local, JsFuture};
        use web_sys::WritableStreamDefaultWriter;

        let transport = net::web::open(server, cert_hash).await?;

        let bidi = JsFuture::from(transport.create_bidirectional_stream())
            .await
            .map_err(|e| net::web::js_err("open_bi", &e))?;
        let writer = WritableStreamDefaultWriter::new(&bidi.writable())
            .map_err(|e| net::web::js_err("stream writer", &e))?;
        // `MAX_STREAM_MSG_BYTES` for the handshake, exactly as the desktop
        // path reads `read_frame::<MAX_STREAM_MSG_BYTES>` — the ceiling is a
        // property of the lane, not of the transport.
        let mut reader = net::web::FrameReader::<MAX_STREAM_MSG_BYTES>::new(&bidi.readable())?;

        let mut msg = [0u8; MAX_STREAM_MSG_BYTES];
        let len = net::handshake::hello(&mut msg)?;
        net::web::write_frame(&writer, &msg[..len]).await?;

        // The challenge: a nonce this shard chose for this connection.
        let reply = reader
            .next()
            .await
            .ok_or_else(|| "no challenge".to_string())?;
        // **`auth_for`'s two halves, with an `await` between them** — the one
        // place this arm cannot use the shared composition, because a wallet
        // signature is a Promise and the nonce only exists mid-handshake, so
        // it cannot be pre-signed. Both halves are the same pure functions the
        // desktop path composes, and `net::handshake`'s
        // `the_two_step_path_and_the_composition_agree` holds the two
        // assemblies to producing byte-identical frames.
        let want = net::handshake::proof_wanted(&reply, server, address)?;
        let signature = match (&want, sign) {
            (Some(proof), Some(f)) => Some(elo::sign_siwe_web(f, address, proof).await?),
            // A guest address, or a page that offered no wallet: nothing is
            // signed and nothing is claimed, exactly as a declined prompt or an
            // absent launcher does on the desktop. A shard with `require_auth`
            // answers `REFUSE_AUTH` at the next step, and `refusal_sentence`
            // turns that into a sentence naming an act a page can perform.
            _ => None,
        };
        let len = net::handshake::auth_frame(address, signature, &mut msg)?;
        net::web::write_frame(&writer, &msg[..len]).await?;

        let reply = reader
            .next()
            .await
            .ok_or_else(|| "no handshake reply".to_string())?;
        let welcome = net::handshake::welcome_from(&reply)?;

        // **The reader is handed on rather than rebuilt, and that is the one
        // place this path can lose data that the desktop one cannot.**
        // `read_exact` stops on the byte it was asked for; a browser read
        // returns whatever arrived, so the chunk that completed `welcome` may
        // already hold the first event frames. `rekey` carries that residue
        // across the change of ceiling (128 -> 320) — building a fresh reader
        // here instead would drop those bytes with nothing to show for it,
        // and the symptom would be a rare missing event at join, which is
        // about the worst bug shape available.
        let (tx, events) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let mut ev_reader = reader.rekey::<MAX_EVENT_MSG_BYTES>();
        spawn_local(async move {
            while let Some(bytes) = ev_reader.next().await {
                if tx.send(bytes).await.is_err() {
                    return;
                }
            }
        });

        // Datagrams: the browser's chunk IS the datagram, so there is no
        // framing on this lane and the ring's policy is untouched — bounded,
        // drop-oldest, counted, the same `DgRing` the desktop client fills.
        let datagrams = datagram_lane();
        net::web::spawn_datagram_reader(transport.datagrams().readable(), datagrams.clone());

        // The action lane's writer, bounded at `ACTION_RING_CAP` for the
        // reason the desktop one is: the client may not hold more in flight
        // than the sim will accept in a burst, and a full queue is REPORTED
        // rather than dropped.
        let (act_tx, act_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(ACTION_RING_CAP);
        net::web::spawn_action_writer(writer, act_rx);

        let wire = net::web::WebWire::new(&transport)?;
        let core = ClientCore::new(welcome.seed, welcome.player_id, welcome.tick);
        Ok(Self {
            core,
            applied: 0,
            applied2: 0,
            welcome,
            wire,
            actions: act_tx,
            events,
            datagrams,
            snapshots: 0,
            input_buf: [0u8; DATAGRAM_BUDGET_BYTES],
            dg_scratch: (0..sim_core::limits::CLIENT_DG_RING)
                .map(|_| Vec::with_capacity(DATAGRAM_BUDGET_BYTES))
                .collect(),
            closed: false,
        })
    }
}

impl Session {
    /// What the browser transport has REFUSED to send, and why.
    ///
    /// `(over_mtu, backpressured)` — datagrams dropped because the payload
    /// exceeded the live `maxDatagramSize`, and because the writer's queue was
    /// already full.
    ///
    /// **This exists because `findings/web-build-20260909.md` §11.3 promised
    /// it and nothing provided it.** The doc said, twice, that the MTU refusal
    /// is *"counted, so a lane that has quietly stopped reaching the shard is
    /// a number somebody can read rather than a player who cannot move"* — and
    /// a whole-repo grep for those counters returned their declarations, their
    /// initialisers, and the two halves of their own increments. Nothing on
    /// any target could read either one. A promise a doc makes and the tree
    /// does not keep is the exact ⚠ `CLAUDE.md` spends a paragraph on, and it
    /// was made here.
    ///
    /// ⚠ **`cfg`'d off natively rather than answering `(0, 0)`**, deliberately.
    /// `NativeWire` has no clamp to count — `wtransport`'s `send_datagram`
    /// returns an `Err` this client discards, `TooLarge` included — so a
    /// permanent zero would read as *measured, and fine*, which is worse than
    /// no number at all. Giving the desktop side a real counter is its own
    /// slice and it is owed.
    #[cfg(target_arch = "wasm32")]
    pub fn wire_counts(&self) -> (u64, u64) {
        (self.wire.over_mtu.get(), self.wire.backpressured.get())
    }

    /// Snapshots the ring dropped before a frame drained them — wall 4's
    /// stated drop-oldest policy, as an observable rather than a comment.
    ///
    /// A poisoned lock answers 0: the reader task having panicked is a dead
    /// session, which `closed()` is the thing to ask about.
    pub fn datagrams_dropped(&self) -> u64 {
        self.datagrams.lock().map(|r| r.dropped).unwrap_or(0)
    }

    /// Whether the shard has hung up on this session. `pump` keeps working
    /// after it turns true — the drains are no-ops and the datagram send goes
    /// nowhere — so a caller may notice at its own cadence; what it must not
    /// do is keep drawing a live world over a dead wire, which is
    /// `render::disconnected`'s job to end.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Queue one already-encoded C→S message for the reliable lane — the
    /// actions (`ACT_*`) and chat the sim is owed rather than allowed to
    /// drop. Callers encode with `protocol`; this owns only the handoff, so
    /// the wire stays the encoder's business and not the transport's.
    ///
    /// **Synchronous and never blocking**, because every caller is a UI
    /// system inside a frame. The cost is that it can refuse: the queue is
    /// bounded at `ACTION_RING_CAP` and a full one answers
    /// [`SendError::Full`] rather than growing or dropping. That refusal is
    /// a real state a panel must draw — the reliable lane backpressures by
    /// design, which is the opposite of `pump`'s datagram send and
    /// deliberately so.
    pub fn send_action(&self, payload: &[u8]) -> Result<(), SendError> {
        use tokio::sync::mpsc::error::TrySendError;
        match self.actions.try_send(payload.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SendError::Full),
            Err(TrySendError::Closed(_)) => Err(SendError::Closed),
        }
    }

    /// Advance one frame: drain both lanes, step the core, send input.
    /// `dt_ms` is the renderer's frame time — the core owns the tick rate,
    /// this only tells it how much wall time passed.
    ///
    /// Both lanes drain non-blockingly. A renderer must never await the
    /// network inside a frame: one slow read would become a dropped frame,
    /// and the client is a hot path too (CLAUDE.md traps).
    pub fn pump(&mut self, dt_ms: f64) -> Frame {
        let core = &mut self.core;
        let snapshots = &mut self.snapshots;
        // Datagrams first: freshest state before we predict on top of it.
        // Every arrival since the last frame, oldest first — the
        // interpolator wants the samples in between, and the view's
        // stale-guard already refuses any datagram a drop reordered past
        // (netcode v2 S2).
        let dg_gone = drain_datagram(&self.datagrams, &mut self.dg_scratch, |dgram| {
            if core.on_datagram(dgram) != Ingest::Error {
                *snapshots += 1;
            }
        });
        let applied = &mut self.applied;
        let applied2 = &mut self.applied2;
        let ev_gone = drain_lane(&mut self.events, |bytes| {
            // A malformed message contributes no flags. The `Err` is dropped
            // here exactly as the retired `let _` dropped it — surfacing a
            // decode error is its own slice and not this one's; what changes
            // is that a SUCCESSFUL decode no longer goes unnoticed.
            if let Ok(flags) = core.on_stream(bytes) {
                *applied |= flags;
                // Read INSIDE the loop, not after it: `applied2()` describes
                // the message just decoded and is overwritten by the next.
                *applied2 |= core.applied2();
            }
        });
        // Either lane's reader ending means the connection under both is
        // done — the tasks only return on a transport-level failure. OR'd
        // into a latch rather than assigned, so one lane outliving the other
        // by a frame cannot flicker the answer back to alive.
        self.closed |= dg_gone || ev_gone;

        let tick = self.core.advance(dt_ms);

        let len = self.core.poll_input(&mut self.input_buf);
        if len > 0 {
            // The drop-oldest rule and the reason for it moved with the call
            // (`net::native`); what stays here is that this is the only line
            // in a frame that touches the transport at all.
            self.wire.send_datagram(&self.input_buf[..len]);
        }

        Frame {
            tick,
            snapshots: self.snapshots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_loopback_host;
    use crate::net::{drain_datagram, drain_lane};

    /// **Which addresses get the carve-out, exhaustively enough to be
    /// evidence.** `tls_posture.rs` proves the two postures over a real
    /// socket at two addresses; this is the code-tier half, and it exists
    /// because the failure mode of a loopback predicate is not a crash —
    /// it is a public address quietly reading as `127.0.0.1` and a client
    /// trusting anything on it, which no wire test would notice because
    /// nobody would think to run one against `127.0.0.1.evil.test`.
    #[test]
    fn only_this_machines_own_loopback_is_carved_out() {
        for yes in [
            "127.0.0.1:4433", // args::DEFAULT_SERVER, the whole dev flow
            "localhost:4433", //
            "LOCALHOST:4433", // a host name is case-insensitive
            "127.9.9.9:1",    // all of 127/8 is loopback
            "[::1]:4433",     // the bracketed v6 literal `check_addr` takes
            "127.0.0.1",      // no port: still this machine
        ] {
            assert!(is_loopback_host(yes), "{yes} should be carved out");
        }
        for no in [
            "192.0.2.2:4433",
            "game.elopros.com:61234",
            "0.0.0.0:4433",             // reachable from off-box, not loopback
            "127.0.0.1.evil.test:4433", // a NAME that merely starts like one
            "localhost.evil.test:4433", // ...and the other spelling of it
            "evil.test:4433",
            "[2001:db8::1]:4433",
            "::1:4433", // a bare v6 literal `check_addr` refuses
            "",
        ] {
            assert!(!is_loopback_host(no), "{no} must be validated, not trusted");
        }
    }

    /// The disconnect detector must not cry wolf: an empty lane whose sender
    /// is alive is a quiet frame, not a dead shard.
    #[test]
    fn a_quiet_lane_is_not_a_dead_one() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let mut got = 0;
        assert!(!drain_lane(&mut rx, |_| got += 1));
        assert_eq!(got, 0);
        tx.try_send(vec![1]).unwrap();
        assert!(!drain_lane(&mut rx, |_| got += 1));
        assert_eq!(got, 1, "a delivered message is handed over exactly once");
        drop(tx);
    }

    /// The property the whole disconnect path leans on: everything the
    /// server managed to send before the hangup is delivered IN the drain
    /// that reports the hangup — the kick's own toast must not be the first
    /// casualty of noticing the kick.
    #[test]
    fn the_last_words_land_before_the_hangup_is_reported() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        tx.try_send(vec![1]).unwrap();
        tx.try_send(vec![2, 2]).unwrap();
        drop(tx); // the reader task ending is exactly this drop
        let mut got: Vec<usize> = Vec::new();
        assert!(
            drain_lane(&mut rx, |b| got.push(b.len())),
            "a dropped sender must be reported as gone"
        );
        assert_eq!(got, [1, 2], "both buffered messages arrive first, in order");
        // And the verdict is repeatable: asking a dead lane again says dead
        // again, which is what lets `pump` run on after the latch is set.
        assert!(drain_lane(&mut rx, |_| unreachable!(
            "nothing left to hand over"
        )));
    }

    /// The unreliable lane queues in arrival order and bounds itself by
    /// dropping the OLDEST (netcode v2 S2; `limits::CLIENT_DG_RING`,
    /// counted). This replaced a depth-one latest-wins `watch` — right for
    /// the own body, a hole in everyone else's motion — so the properties
    /// pinned here are the ring's own: in-order full delivery, drop-oldest
    /// at the cap with the count moving, nothing redelivered, and the last
    /// snapshots landing no later than the drain that reports the hangup.
    #[test]
    fn the_datagram_ring_delivers_in_order_and_drops_oldest() {
        use sim_core::limits::CLIENT_DG_RING;
        let ring = super::datagram_lane();
        let mut scratch: Vec<Vec<u8>> = (0..CLIENT_DG_RING).map(|_| Vec::new()).collect();
        let mut got: Vec<usize> = Vec::new();
        assert!(
            !drain_datagram(&ring, &mut scratch, |_| unreachable!("nothing has arrived")),
            "a quiet lane with a live sender is not a dead one"
        );
        // Six arrive between frames: six hand over, oldest first.
        for n in 1..=6usize {
            ring.lock().unwrap().push(&vec![0u8; n]);
        }
        assert!(!drain_datagram(&ring, &mut scratch, |b| got.push(b.len())));
        assert_eq!(got, [1, 2, 3, 4, 5, 6], "in order, none collapsed");
        got.clear();
        // Nothing is redelivered on a quiet frame.
        assert!(!drain_datagram(&ring, &mut scratch, |_| unreachable!(
            "already handed over"
        )));
        // Two past the cap: the two OLDEST are gone, and counted.
        for n in 1..=CLIENT_DG_RING + 2 {
            ring.lock().unwrap().push(&vec![0u8; n]);
        }
        assert!(!drain_datagram(&ring, &mut scratch, |b| got.push(b.len())));
        let want: Vec<usize> = (3..=CLIENT_DG_RING + 2).collect();
        assert_eq!(got, want, "drop-oldest kept the freshest ring-full");
        assert_eq!(ring.lock().unwrap().dropped, 2, "the drops are counted");
        got.clear();
        // The reader task's last words land in the drain that reports the
        // hangup — closed-with-pending is not yet gone.
        {
            let mut r = ring.lock().unwrap();
            r.push(&[9u8; 7]);
            r.closed = true;
        }
        assert!(
            drain_datagram(&ring, &mut scratch, |b| got.push(b.len())),
            "closed and drained empty is gone"
        );
        assert_eq!(got, [7], "the last snapshot was not eaten by the hangup");
        // Repeatable, which is what lets `pump` run on after the latch.
        assert!(drain_datagram(&ring, &mut scratch, |_| unreachable!(
            "nothing left to hand over"
        )));
    }
}
