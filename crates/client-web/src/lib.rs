//! Gates in a tab: the module a page loads, and nothing else.
//!
//! **This is the browser's `main.rs`, and it is deliberately the same slice
//! that file was.** The native client's first cut (DECISIONS.md 2026-08-05)
//! connected over the real wire, drove the real `ClientCore` and reported what
//! the world said — no window, no renderer — because the risky claim was never
//! *"Rust can draw a triangle"*, it was that the transport, the handshake,
//! both lanes and the predictor run against an unmodified shard. The risky
//! claim here is identical with one word changed, and so is the answer.
//!
//! What it is NOT: the game. There is no canvas. Bevy on WebGL2 is the next
//! slice and the largest unknown left (`findings/web-build-20260909.md` §8,
//! §10.1 — the dependency stack was proved to compile for this target and
//! nothing has drawn a frame). What this proves is the half nobody could
//! prove from a build log: that a `WebTransport` in a browser completes our
//! handshake against a real shard and that snapshots arrive.
//!
//! **The server needs no change for any of this** and that is the finding the
//! whole port rests on: `crates/server/src/net.rs` already serves
//! browser-shaped certificates, and `NETCODE.md` §2.2's P-256 and 14-day rules
//! are WebTransport spec rules, written for browsers, that our shard has
//! satisfied since before there was a browser client to want them.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;

/// One joined session, handed to the page.
///
/// The page owns the frame loop and calls [`pump`](Gates::pump) from
/// `requestAnimationFrame`, exactly as `bin/gates.rs` calls it from a Bevy
/// system — the pump is synchronous, drains both lanes without blocking and
/// never awaits, which is what makes it callable from a frame at all
/// (`client::Session::pump`).
#[wasm_bindgen]
pub struct Gates {
    session: client::Session,
    /// The last frame's snapshot count, kept here so [`snapshots`](Gates::snapshots)
    /// can be a getter.
    ///
    /// **Not derived by calling `pump` again**, which is what the first draft
    /// did and is a real bug rather than a style point: `pump` DRAINS both
    /// lanes, so a getter that called it would consume snapshots and events
    /// outside the frame loop and hand them to nobody. Same family as
    /// `CLAUDE.md`'s destructive-read trap — a queue with a single-consumer
    /// contract needs one owner, and here the owner is `pump`.
    last: client::Frame,
}

#[wasm_bindgen]
impl Gates {
    /// Join `server` (`host:port`) and return once the shard has said welcome.
    ///
    /// `cert_hash` is the dotted-hex SHA-256 a shard prints at boot, and on
    /// this platform it is **required for a dev shard** rather than optional:
    /// a page cannot ask a browser to skip certificate validation, so the
    /// loopback carve-out the desktop client has does not exist here
    /// (`client::net::web::open` has the table). A public shard with a real
    /// chain needs nothing — the browser trusts it outright.
    ///
    /// `identity` is `0x…` and is accepted so the shape is there, but it is
    /// **not yet load-bearing**: with no launcher in a page there is nothing
    /// to sign the challenge, so a declared address joins as a guest with a
    /// claim nobody checked. That is the same posture `--identity` has
    /// natively when the launcher is absent, and the honest one until a
    /// browser wallet lands (`client::elo::sign_siwe`, the wasm arm).
    ///
    /// ⚠ A shard with `require_auth` will answer `REFUSE_AUTH` to a guest, and
    /// the message must read *this shard needs an account* rather than a login
    /// failure — the trap `CLAUDE.md` records for the vendored launcher, which
    /// lands two hops from its cause.
    pub async fn join(
        server: String,
        cert_hash: Option<String>,
        identity: Option<String>,
    ) -> Result<Gates, JsValue> {
        let address = match identity.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => protocol::Address::GUEST,
            Some(a) => protocol::Address::from_hex(a.as_bytes()).ok_or_else(|| {
                JsValue::from_str(
                    "identity is not an Ethereum address — it must be 0x followed by 40 hex digits",
                )
            })?,
        };
        let session = client::Session::connect(
            &server,
            cert_hash.as_deref(),
            address,
            client::elo::sign_siwe,
        )
        .await
        .map_err(|e| JsValue::from_str(&e))?;
        let last = client::Frame {
            tick: 0,
            snapshots: 0,
        };
        Ok(Gates { session, last })
    }

    /// Advance one frame. `dt_ms` is the page's frame time; the core owns the
    /// 30 Hz tick and this only tells it how much wall time passed.
    ///
    /// Returns how many sim steps that frame drove — 0 or 1 is the correct
    /// reading at 60 Hz against a 30 Hz tick, not a stalled clock.
    pub fn pump(&mut self, dt_ms: f64) -> u32 {
        self.last = self.session.pump(dt_ms);
        self.last.tick
    }

    /// Snapshots accepted since the join. **This is the number that says the
    /// transport works**: it climbs only when a datagram arrived over
    /// `WebTransport`, decoded, and was applied by the same `ClientCore` the
    /// desktop client runs. `f64` because JS has no `u64`.
    #[wasm_bindgen(getter)]
    pub fn snapshots(&self) -> f64 {
        self.last.snapshots as f64
    }

    #[wasm_bindgen(getter)]
    pub fn player_id(&self) -> u32 {
        self.session.welcome.player_id
    }

    /// The world seed, as the string the shard's own logs print it as. A
    /// `u64` does not survive a JS number, and this is for a human to compare
    /// against a shard, so it travels as text rather than as a rounded float.
    #[wasm_bindgen(getter)]
    pub fn seed(&self) -> String {
        self.session.welcome.seed.to_string()
    }

    /// Whether the shard has hung up. Sticky: a connection does not come back,
    /// and the page must stop drawing a live world over a dead wire.
    #[wasm_bindgen(getter)]
    pub fn closed(&self) -> bool {
        self.session.closed()
    }
}

/// Route Rust panics to the console instead of an opaque `unreachable`.
///
/// **Called by the page before anything else**, and it is not a nicety: a
/// panic in wasm aborts with no message at all by default, so the first real
/// bug in a browser would present as a module that simply stopped. This is the
/// same reasoning `bin/gates.rs` gives for installing its panic hook first,
/// before anything can panic.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("gates: panic: {info}")));
    }));
}
