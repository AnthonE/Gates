//! The native client's session: one wtransport connection, the handshake,
//! and the pump that drives `client_wasm::ClientCore`.
//!
//! This crate exists because the client is moving off the browser
//! (DECISIONS.md 2026-08-05). What it deliberately does NOT do is
//! reimplement the client: `ClientCore`, the predictor, the interpolator
//! and the clock are the same code the browser build runs, and they were
//! always pure — `client-wasm` depends on nothing but `sim-core` and
//! `protocol`, and already builds as an rlib. The browser-shaped half is
//! `bridge.rs`'s raw C ABI, and that is exactly the half a native client
//! does not need.
//!
//! The wire is unchanged and no server change is owed. `wtransport` has a
//! client side, so this speaks the identical transport the browser speaks,
//! against the same shard, at the same `PROTO_VER`.

pub mod args;
pub mod scry;
pub mod shardlist;

// The render path. Feature-gated because Bevy is several hundred crates and
// the code tier must not pay for it (`crates/client/Cargo.toml`).
#[cfg(feature = "render")]
pub mod render;

use client_wasm::core::{ClientCore, Ingest};
use protocol::{
    decode_refuse, decode_welcome, encode_hello, peek_kind, Hello, Welcome, KIND_REFUSE,
    KIND_WELCOME, MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER,
};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
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
    pub welcome: Welcome,
    connection: std::sync::Arc<Connection>,
    /// The C→S half of the bidi stream, held for the life of the session.
    /// Dropping it finishes that direction, and the server reads the close
    /// as the client going away — the join never resolves to a world slot
    /// and `snap sent` stays 0 while inputs still flow, which is exactly
    /// how this presented the first time it ran. It is also the action and
    /// chat lane, so it has to live here regardless.
    send: SendStream,
    events: tokio::sync::mpsc::Receiver<Vec<u8>>,
    datagrams: tokio::sync::mpsc::Receiver<Vec<u8>>,
    snapshots: u64,
    input_buf: [u8; DATAGRAM_BUDGET_BYTES],
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
    pub async fn connect(endpoint: &Endpoint<Client>, server: &str) -> Result<Self, String> {
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

        let core = ClientCore::new(welcome.seed, welcome.player_id, welcome.tick);
        Ok(Self {
            core,
            welcome,
            connection,
            send,
            events,
            datagrams,
            snapshots: 0,
            input_buf: [0u8; DATAGRAM_BUDGET_BYTES],
        })
    }

    /// Send one already-encoded C→S message on the reliable lane — the
    /// actions (`ACT_*`) and chat the sim is owed rather than allowed to
    /// drop. Callers encode with `protocol`; this owns only the framing,
    /// so the wire stays the encoder's business and not the transport's.
    ///
    /// Async because the reliable lane backpressures by design
    /// (`ACTION_RING_CAP`, "nothing on the reliable lane is dropped") —
    /// which is the opposite of `pump`'s datagram send, and deliberately.
    pub async fn send_action(&mut self, payload: &[u8]) -> Result<(), String> {
        write_frame(&mut self.send, payload).await
    }

    /// Advance one frame: drain both lanes, step the core, send input.
    /// `dt_ms` is the renderer's frame time — the core owns the tick rate,
    /// this only tells it how much wall time passed.
    ///
    /// Both lanes drain non-blockingly. A renderer must never await the
    /// network inside a frame: one slow read would become a dropped frame,
    /// and the client is a hot path too (CLAUDE.md traps).
    pub fn pump(&mut self, dt_ms: f64) -> Frame {
        // Datagrams first: freshest state before we predict on top of it.
        while let Ok(dgram) = self.datagrams.try_recv() {
            if self.core.on_datagram(&dgram) != Ingest::Error {
                self.snapshots += 1;
            }
        }
        while let Ok(bytes) = self.events.try_recv() {
            let _ = self.core.on_stream(&bytes);
        }

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
