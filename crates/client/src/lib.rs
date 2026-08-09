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
// Join links — `scry://join/gates/host:port`. Unconditional for the reason
// `shardlist` is: it parses a string a stranger wrote and handed to a friend,
// which is the input in this client that most needs a test in the code tier.
pub mod deeplink;
pub mod scry;
pub mod shardlist;
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

/// The dev-trust endpoint. Same posture as `server::botclient::bot_endpoint`
/// and for the same reason: shards we run, self-signed certs. A shipping
/// client validates, and that is a `DECISIONS.md` row before anything is
/// published, not a default to drift into.
pub fn client_endpoint() -> Result<Endpoint<Client>, String> {
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
        Endpoint::client(
            ClientConfig::builder()
                .with_bind_config(ip)
                .with_no_cert_validation()
                .build(),
        )
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
    datagrams: tokio::sync::mpsc::Receiver<Vec<u8>>,
    snapshots: u64,
    input_buf: [u8; DATAGRAM_BUDGET_BYTES],
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
        sign: impl FnOnce(&str) -> Option<protocol::Signature>,
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
                return Err(format!("refused: code {}", r.code));
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
            let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
            let n = protocol::siwe_message(
                domain,
                &address,
                &challenge.nonce,
                challenge.issued_at,
                &mut text,
            );
            let text = core::str::from_utf8(&text[..n.min(protocol::SIWE_MESSAGE_MAX)])
                .map_err(|_| "the challenge message is not utf-8".to_string())?;
            match sign(text) {
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
                return Err(format!("refused: code {}", r.code));
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
        // the pump must never await the network mid-frame.
        let (dg_tx, datagrams) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        let dg_conn = connection.clone();
        tokio::spawn(async move {
            while let Ok(dgram) = dg_conn.receive_datagram().await {
                if dg_tx.send(dgram.to_vec()).await.is_err() {
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
        let dg_gone = drain_lane(&mut self.datagrams, |dgram| {
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
    use super::drain_lane;

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
}
