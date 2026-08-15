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
// Join links — `scry://join/gates/host:port`. Unconditional for the reason
// `shardlist` is: it parses a string a stranger wrote and handed to a friend,
// which is the input in this client that most needs a test in the code tier.
pub mod deeplink;
pub mod scry;
pub mod shardlist;
// Where a player's screenshot goes and what it is called. Unconditional for
// `config`'s reason exactly: a path rule and a filename rule are pure, they
// are the part that can be wrong on a box nobody here owns (three platforms,
// two Linux conventions), and a test behind `--features render` runs in the
// renderer tier where nobody looks at it. `render/shot.rs` is the Bevy half.
pub mod shot;
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

// The render path. Feature-gated because Bevy is several hundred crates and
// the code tier must not pay for it (`crates/client/Cargo.toml`).
#[cfg(feature = "render")]
pub mod render;

use client_core::core::{ClientCore, Ingest};
use protocol::{
    decode_refuse, decode_welcome, encode_hello, peek_kind, Hello, Welcome, KIND_REFUSE,
    KIND_WELCOME, MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER,
};
use sim_core::limits::{ACTION_RING_CAP, DATAGRAM_BUDGET_BYTES};
use wtransport::config::IpBindConfig;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};

/// Stream framing: `u16` LE length prefix per message. Byte-identical to
/// `server::net::{read_frame, write_frame}` and to `web/src/net.js`, and
/// reimplemented here rather than imported because the client must never
/// depend on the `server` crate — that would ship the authoritative sim
/// inside the client binary. If a third copy ever appears, that is the
/// signal to lift this into `protocol` where the rest of the wire lives.
async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), String> {
    let len = (payload.len() as u16).to_le_bytes();
    send.write_all(&len).await.map_err(|e| e.to_string())?;
    send.write_all(payload).await.map_err(|e| e.to_string())?;
    Ok(())
}

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
    // the scry launcher installed died here at startup, before any address
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
    connection: std::sync::Arc<Connection>,
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
    /// The unreliable lane, depth one — see [`datagram_lane`] for why it is a
    /// `watch` and what that costs at a bad frame rate.
    datagrams: DatagramRx,
    snapshots: u64,
    input_buf: [u8; DATAGRAM_BUDGET_BYTES],
    /// Reused landing buffer for the newest datagram, so the watch's read
    /// guard is released before the decode runs — see [`drain_datagram`].
    dg_buf: Vec<u8>,
    /// The shard hung up. Latched by [`Session::pump`] when either receive
    /// lane's reader task ends — the event task on a closed or desynced
    /// stream, the datagram task on a dead connection — because a reader
    /// hanging up is the one fact both failure shapes share. Sticky on
    /// purpose: a connection does not come back, and a flag that cleared
    /// itself would let one hopeful frame un-say it.
    closed: bool,
}

/// Drain one lane without blocking, handing each message to `each`; answer
/// whether the lane's sender has hung up.
///
/// **The buffered last words land before the hangup is reported** — tokio's
/// mpsc yields everything queued ahead of the drop before it answers
/// `Disconnected`, and the test below pins that, because the whole disconnect
/// path leans on it: the facts the server sent in its final flush (a kick's
/// toast, the last snapshot) must reach the core before the client declares
/// the session over, or the reason for the hangup is the first thing lost.
fn drain_lane(rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>, mut each: impl FnMut(&[u8])) -> bool {
    use tokio::sync::mpsc::error::TryRecvError;
    loop {
        match rx.try_recv() {
            Ok(bytes) => each(&bytes),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
}

/// The receiving end of the datagram mailbox — see [`datagram_lane`].
type DatagramRx = tokio::sync::watch::Receiver<Vec<u8>>;

/// The datagram mailbox: **depth one, latest-wins**. A `watch` and not an
/// mpsc, because the lane carries snapshots and a snapshot REPLACES its
/// predecessor rather than adding to it — the policy the server's own writer
/// already runs three files away and states in one line (`server::net`:
/// *"Drain to the newest — snapshots replace, never accumulate"*). This end
/// was a `channel::<Vec<u8>>(256)` with `send().await` behind it, which is
/// backpressure: an INVENTED LITERAL DEPTH with no stated overflow policy,
/// opened in the same function as — and just above — a lane that takes its
/// depth from `ACTION_RING_CAP` and spells its policy out. Wall 4 wants the
/// cap and the policy named; latest-wins is both, and it is the server's,
/// not a new one.
///
/// The initial value is an empty payload that is never delivered: a `watch`
/// hands a fresh receiver its seed already marked seen, so [`drain_datagram`]
/// sees no change until the reader task actually sends.
///
/// **Why dropping older snapshots is safe, and it is a property of the wire
/// rather than a hope** (re-read against the tree on 2026-08-10): the server
/// deltas only against a snapshot the client ACKED (`server::client::baseline`
/// takes `newest_acked`), and an ack can only name a snapshot that was really
/// applied (`client_core::view` writes the ring on the success path and
/// `ack_fields` reads that ring) — so a snapshot dropped here is a baseline
/// the server never offers. Removed entity ids are not exactly-once either:
/// they stay in `pending_removals` and ride EVERY snapshot until an ack
/// clears them (`ClientSlot::on_acks` → `pending_remove`). `nudge` and
/// `last_executed_seq` are levels stamped into every header, not edges. And
/// the shard has exactly one `send_datagram` (`server::net`), so this lane
/// carries snapshots and nothing else.
///
/// **What it costs, stated rather than hidden.** Below ~15 fps two snapshots
/// can land inside one frame and depth one keeps only the second, so the
/// interpolator loses a sample — `INTERP_DELAY_TICKS = 4` over a 16-deep
/// `HISTORY` (`client_core::interp`), i.e. it eats history it would rather
/// have exactly when the frame rate is worst. The trade is deliberate: the
/// alternative is a queue that hands a stalled client old world states to
/// draw, and stale is worse than sparse.
fn datagram_lane() -> (tokio::sync::watch::Sender<Vec<u8>>, DatagramRx) {
    tokio::sync::watch::channel(Vec::new())
}

/// Take whatever is in the mailbox — at most one payload — and answer
/// whether the sender has hung up. The counterpart of [`drain_lane`] for the
/// unreliable lane, and deliberately a SECOND function rather than a change
/// to that one: `drain_lane` is shared with the 64-deep reliable event lane
/// (inventory, container echoes, toasts, refusals, chat), where every message
/// is owed and none may be skipped.
///
/// **The hangup is asked BEFORE the value is read**, which is the whole
/// subtlety here. `has_changed` answers `Err` the instant the sender drops,
/// unseen payload or not, so returning early on it would eat the last
/// snapshot — the property `the_last_words_land_before_the_hangup_is_reported`
/// pins on the other lane, and it has to hold on this one too. The payload
/// outlives its sender in the watch's slot, and its seen-flag with it, so the
/// read below is still correct after the hangup.
///
/// **The payload is copied out before `each` runs, and that is not
/// bookkeeping.** A `watch::Ref` is a read guard over the slot's lock, so
/// holding it across the callback would hold it across a whole snapshot
/// decode — and `each` here is `ClientCore::on_datagram`. The reader task's
/// `send` is synchronous now, so it would block a tokio worker thread for
/// that span; the mpsc this replaced took no lock on the frame path at all,
/// and a change that fixes a queue by adding a lock to the frame path has
/// not paid for itself. `scratch` is owned by the [`Session`] and reused, so
/// the copy costs a `memcpy` and no allocation after the first frame — the
/// per-frame-allocation rule the client is held to, same as the sim thread.
fn drain_datagram(rx: &mut DatagramRx, scratch: &mut Vec<u8>, mut each: impl FnMut(&[u8])) -> bool {
    let gone = rx.has_changed().is_err();
    {
        let newest = rx.borrow_and_update();
        if !newest.has_changed() {
            return gone;
        }
        scratch.clear();
        scratch.extend_from_slice(&newest);
    }
    each(scratch);
    gone
}

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
    /// the key behind it — in practice a call into the scry launcher, which
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
    ) -> Result<Self, String> {
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

        let mut msg = [0u8; MAX_STREAM_MSG_BYTES];
        let len = encode_hello(
            &Hello {
                proto_ver: PROTO_VER,
                ver: protocol::version::VER,
                build: protocol::version::BUILD,
            },
            &mut msg,
        )
        .map_err(|e| format!("encode hello: {e:?}"))?;
        write_frame(&mut send, &msg[..len]).await?;

        // The challenge: a nonce this shard chose for this connection.
        let (reply, reply_len) = read_frame::<MAX_STREAM_MSG_BYTES>(&mut recv)
            .await
            .ok_or_else(|| "no challenge".to_string())?;
        let reply = &reply[..reply_len];
        match peek_kind(reply) {
            Ok(protocol::KIND_CHALLENGE) => {}
            Ok(KIND_REFUSE) => {
                let r = decode_refuse(reply).map_err(|e| format!("refuse: {e:?}"))?;
                return Err(match protocol::refuse_text(r.code) {
                    Some(why) => format!("refused: {why}"),
                    None => format!("refused: code {}", r.code),
                });
            }
            other => return Err(format!("expected a challenge, got {other:?}")),
        }
        let challenge =
            protocol::decode_challenge(reply).map_err(|e| format!("challenge: {e:?}"))?;

        // **The domain is the host we dialled, never anything the server
        // said.** That is the whole of SIWE's domain binding: a signature
        // collected by one shard is not valid at another because the two
        // messages differ, and letting the server name itself would hand
        // that away.
        let domain = server.rsplit_once(':').map(|(h, _)| h).unwrap_or(server);
        let auth = if address.is_guest() {
            protocol::Auth::default()
        } else {
            // **This process no longer composes the message, and that is the
            // fix.** It used to build the SIWE text here and hand it to the
            // launcher's `sign`, which refused every one: `sign` classifies a
            // message by its first line (`scry <family>`) and EIP-4361 begins
            // with a domain. The refusal became `None`, `None` means "connect
            // as a guest", and a `require_auth` shard answered REFUSE_AUTH —
            // so every login failed as if the signature were wrong, when the
            // message was one the launcher would never sign.
            //
            // `prove` is the verb: the LAUNCHER writes every word, which is
            // what stops a game smuggling a sentence into a signature, and it
            // needs no consent prompt for the same reason. We hand over three
            // inert values and the shard rebuilds the text from the ones it
            // already knows (`protocol::siwe_message`).
            let mut hex = [0u8; protocol::NONCE_BYTES * 2];
            for (i, b) in challenge.nonce.iter().enumerate() {
                const H: &[u8; 16] = b"0123456789abcdef";
                hex[i * 2] = H[(b >> 4) as usize];
                hex[i * 2 + 1] = H[(b & 0xf) as usize];
            }
            let nonce = core::str::from_utf8(&hex).map_err(|_| "nonce hex".to_string())?;
            match sign(domain, nonce, challenge.issued_at) {
                Some(signature) => protocol::Auth { address, signature },
                None => protocol::Auth::default(),
            }
        };
        let len =
            protocol::encode_auth(&auth, &mut msg).map_err(|e| format!("encode auth: {e:?}"))?;
        write_frame(&mut send, &msg[..len]).await?;

        let (reply, reply_len) = read_frame::<MAX_STREAM_MSG_BYTES>(&mut recv)
            .await
            .ok_or_else(|| "no handshake reply".to_string())?;
        let reply = &reply[..reply_len];
        let welcome = match peek_kind(reply) {
            Ok(KIND_WELCOME) => decode_welcome(reply).map_err(|e| format!("welcome: {e:?}"))?,
            Ok(KIND_REFUSE) => {
                let r = decode_refuse(reply).map_err(|e| format!("refuse: {e:?}"))?;
                return Err(match protocol::refuse_text(r.code) {
                    Some(why) => format!("refused: {why}"),
                    None => format!("refused: code {}", r.code),
                });
            }
            other => return Err(format!("unexpected handshake reply: {other:?}")),
        };

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
        // the pump must never await the network mid-frame. The mailbox
        // itself is depth one and latest-wins (`datagram_lane`), so the
        // handoff cannot await either — a full queue is not a state this
        // lane can be in.
        let (dg_tx, datagrams) = datagram_lane();
        let dg_conn = connection.clone();
        tokio::spawn(async move {
            while let Ok(dgram) = dg_conn.receive_datagram().await {
                if dg_tx.send(dgram.to_vec()).is_err() {
                    return;
                }
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
            connection,
            actions: act_tx,
            events,
            datagrams,
            snapshots: 0,
            input_buf: [0u8; DATAGRAM_BUDGET_BYTES],
            dg_buf: Vec::with_capacity(DATAGRAM_BUDGET_BYTES),
            closed: false,
        })
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
        // At most one, and it is the newest the wire delivered since the
        // last frame — the lane replaces rather than queues.
        let dg_gone = drain_datagram(&mut self.datagrams, &mut self.dg_buf, |dgram| {
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
            // send_datagram, never send_datagram_wait (CLAUDE.md traps): a
            // congestion stall must cost freshness, not latency.
            let _ = self.connection.send_datagram(&self.input_buf[..len]);
        }

        Frame {
            tick,
            snapshots: self.snapshots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{drain_datagram, drain_lane, is_loopback_host};

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
            "game.moreright.xyz:61234",
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

    /// The unreliable lane replaces, it does not accumulate: six datagrams
    /// arriving between two frames hand over ONE payload, the newest. Six
    /// because the join is where a backlog was really reachable — `bin/gates`
    /// connects before `App::new()`, so the mailbox filled through window,
    /// asset and first-chunk time on every join, and those pre-first-ack
    /// snapshots are zero-state and all apply.
    ///
    /// **Be honest about which half of that this test can defend.** The
    /// keeps-only-the-newest half is enforced by the TYPE — a `watch` slot is
    /// physically unable to accumulate — so no edit to [`drain_datagram`] can
    /// redden the `got == [6]` assertion; it was seen red only against the
    /// mpsc this replaced, and a revert to mpsc would delete this test along
    /// with the code. What the test actually gates is the ordering that IS
    /// hand-written and IS breakable: the seed is never delivered, a seen
    /// payload is never redelivered, and the last snapshot lands in the same
    /// drain that reports the hangup rather than being eaten by it —
    /// `watch::has_changed` answers `Err` on a dropped sender whether or not
    /// a payload is unseen, which is the same property
    /// `the_last_words_land_before_the_hangup_is_reported` pins on the
    /// reliable lane. Each of those three fails on a one-line mutation.
    #[test]
    fn the_datagram_mailbox_keeps_only_the_newest() {
        let (tx, mut rx) = super::datagram_lane();
        let mut buf: Vec<u8> = Vec::new();
        let mut got: Vec<usize> = Vec::new();
        assert!(
            !drain_datagram(&mut rx, &mut buf, |_| unreachable!("nothing has arrived")),
            "a quiet lane with a live sender is not a dead one"
        );
        for n in 1..=6usize {
            tx.send(vec![0u8; n]).unwrap();
        }
        assert!(
            !drain_datagram(&mut rx, &mut buf, |b| got.push(b.len())),
            "the sender is alive"
        );
        assert_eq!(got, [6], "only the newest snapshot is handed over");
        // And a second drain with nothing new is not a redelivery: the
        // payload stays in the slot, but it has been marked seen.
        assert!(!drain_datagram(&mut rx, &mut buf, |_| unreachable!(
            "the newest was already handed over"
        )));

        got.clear();
        tx.send(vec![0u8; 7]).unwrap();
        drop(tx); // the reader task ending is exactly this drop
        assert!(
            drain_datagram(&mut rx, &mut buf, |b| got.push(b.len())),
            "a dropped sender must be reported as gone"
        );
        assert_eq!(
            got,
            [7],
            "the last snapshot lands in the drain that reports the hangup"
        );
        // Repeatable, which is what lets `pump` run on after the latch is set.
        assert!(drain_datagram(&mut rx, &mut buf, |_| unreachable!(
            "nothing left to hand over"
        )));
    }
}
