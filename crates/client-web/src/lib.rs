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
    /// The camera angles this client carries between frames. Owned here and
    /// stepped by `look::control`, so the page hands over pixels and key
    /// states and derives nothing — see [`Gates::input`].
    aim: client::look::Aim,
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
    /// `identity` is the `0x…` address the player is claiming, and `sign` is
    /// the wallet that proves it: `(text: string) => Promise<"0x…">`, which is
    /// the shape of the platform's own `Deck.wallet.sign(message, address)`
    /// (`watchtower/js/deck-shim.js` in `AnthonE/scry-forge`) with the address
    /// already bound. A page on elopros.com passes the wallet it already has.
    ///
    /// **Both or neither.** An address with no wallet is refused here rather
    /// than joining as a guest under a claim nobody checked — that shape is
    /// right on the desktop, where `--identity` with no launcher running means
    /// "I have no signer", and wrong in a page, where the only way to have an
    /// address at all is to have connected the wallet that holds it. Passing
    /// neither is a guest, which a shard that takes guests admits.
    ///
    /// ⚠ A shard with `require_auth` answers `REFUSE_AUTH` to a guest, and the
    /// message must read *this shard needs an account* rather than a login
    /// failure — the trap `CLAUDE.md` records for the vendored launcher, which
    /// lands two hops from its cause. `client::refusal_sentence` owns that
    /// wording and `crates/client/tests/refusals.rs` holds it to the codes the
    /// protocol declares.
    pub async fn join(
        server: String,
        cert_hash: Option<String>,
        identity: Option<String>,
        sign: Option<js_sys::Function>,
    ) -> Result<Gates, JsValue> {
        let declared = identity.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if declared.is_some() != sign.is_some() {
            // Said out loud rather than coerced either way. Silently dropping
            // the address would join as a guest and hit `REFUSE_AUTH` on a
            // locked shard — a refusal whose stated cause is the shard when
            // the real cause is this call. Silently ignoring the wallet would
            // be the same bug wearing the other hat.
            return Err(JsValue::from_str(
                "join needs an address and a wallet together, or neither: an address \
                 with nothing to sign for it is a claim no shard will take, and a \
                 wallet with no address has nothing to prove",
            ));
        }
        let address = match declared {
            None => protocol::Address::GUEST,
            Some(a) => protocol::Address::from_hex(a.as_bytes()).ok_or_else(|| {
                JsValue::from_str(
                    "identity is not an Ethereum address — it must be 0x followed by 40 hex digits",
                )
            })?,
        };
        let session =
            client::Session::connect(&server, cert_hash.as_deref(), address, sign.as_ref())
                .await
                .map_err(join_error)?;
        let last = client::Frame {
            tick: 0,
            snapshots: 0,
        };
        Ok(Gates {
            session,
            aim: client::look::Aim::default(),
            last,
        })
    }

    /// Hand this frame's raw input to the client core.
    ///
    /// **Pixels and key states in — the page derives nothing.** Which way is
    /// right is arithmetic, and `crates/client/src/look.rs` exists because
    /// that arithmetic was wrong on both clients at once: the operator
    /// reported the mouse inverted, and the half nobody had noticed is that
    /// the strafe was mirrored too. A second client re-deriving `(-r, f)` from
    /// a doc comment is that bug's second chance, so `look::control` is the
    /// one named entry and this is a passthrough to it.
    ///
    /// `buttons` is a `BTN_*` bitfield the caller has already decided. What a
    /// click MEANS is modal even here — a page with a panel open must not send
    /// an attack — and that decision is the caller's, exactly as it is the
    /// Bevy client's (`render/input.rs`).
    ///
    /// Call it before [`pump`](Gates::pump): the core takes the level, and
    /// `advance` is what mints a frame from it.
    #[allow(clippy::too_many_arguments)]
    pub fn input(
        &mut self,
        dx_px: f32,
        dy_px: f32,
        forward: bool,
        back: bool,
        left: bool,
        right: bool,
        buttons: u8,
        sel: u8,
        invert_pitch: bool,
    ) {
        let (buttons, yaw, pitch, move_x, move_z, sel) = client::look::control(
            &mut self.aim,
            client::look::Raw {
                dx_px,
                dy_px,
                invert_pitch,
                forward,
                back,
                left,
                right,
                buttons,
                sel,
            },
        );
        self.session
            .core
            .set_input(buttons, yaw, pitch, move_x, move_z, sel);
    }

    /// Where this player's eye is, in world metres — `[x, y, z]`.
    ///
    /// **The readback that makes an operator's one act prove prediction.**
    /// Without it the page can show `snapshots` climbing while the input lane
    /// is silently clamped and the body never moves, which looks identical to
    /// a working client. A number that changes when a key is held is the
    /// cheapest possible evidence that input reached the shard and came back.
    #[wasm_bindgen(getter)]
    pub fn eye(&self) -> Vec<f32> {
        self.session.core.eye_position().to_vec()
    }

    /// `[over_mtu, backpressured]` — datagrams this page's transport REFUSED
    /// to send, and did not.
    ///
    /// Both should be 0 forever: `DATAGRAM_BUDGET_BYTES` is 1,100 against a
    /// 1,200-byte QUIC floor, and this lane writes one small datagram a frame.
    /// They are surfaced because "should be 0 forever" is what was said about
    /// the clamp itself for a year while nothing checked, and because the
    /// failure they describe is invisible from the outside — a player who
    /// cannot move, on a page whose snapshot counter is still climbing.
    #[wasm_bindgen(getter)]
    pub fn refused(&self) -> Vec<f64> {
        let (over_mtu, backpressured) = self.session.wire_counts();
        vec![over_mtu as f64, backpressured as f64]
    }

    /// Snapshots the receive ring dropped before a frame drained them — wall
    /// 4's drop-oldest policy, as a number. Non-zero means this page is not
    /// draining fast enough to keep every sample the interpolator wants.
    #[wasm_bindgen(getter)]
    pub fn dropped(&self) -> f64 {
        self.session.datagrams_dropped() as f64
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

    /// Hand the world to Bevy and start drawing. **Consumes the session.**
    ///
    /// `self` by value because the `Session` moves into the ECS: `Net` owns it
    /// for the life of the app, and a `Gates` left behind holding a copy would
    /// be a second owner of a single-consumer lane. The page's handle is gone
    /// after this call, which is the honest shape — the join is over and the
    /// renderer is the thing that is live.
    ///
    /// **The page must stop pumping.** `Session::pump` drains `ClientCore`'s
    /// own-fact rings destructively and `render/input.rs` now calls it from a
    /// Bevy system every frame; two drivers would each see half the hits,
    /// toasts and refusals. That is the single-consumer defect `CLAUDE.md`
    /// records merging cleanly and breaking silently, so the page's
    /// `requestAnimationFrame` loop is deleted rather than left dormant.
    ///
    /// ⚠ **`run()` does not return on this platform.** Bevy's wasm arm hands
    /// the loop to `requestAnimationFrame` and returns immediately, so the
    /// `App` has to be owned by the loop rather than by this frame — which is
    /// what `run()` arranges. Nothing after it executes.
    pub fn play(self, canvas: String) {
        use bevy::asset::AssetMetaCheck;
        use bevy::prelude::*;
        use bevy::window::WindowResolution;
        use client::render::{GatesRenderPlugin, Net, Start, WorldId};

        let Gates { session, .. } = self;
        let server = session.welcome.seed.to_string();
        let seed = session.welcome.seed;

        // The backing store: the viewport in device pixels, scaled down
        // uniformly under WebGL2's 2048 cap — `client::render::web::fit`,
        // and its header for why the canvas is then STRETCHED back over the
        // viewport with a CSS transform rather than sized to it. Read here,
        // before the window exists, because the first `configure_surface`
        // is the one that must not exceed the cap: winit's `ResizeObserver`
        // sets the backing store from the CSS box times `devicePixelRatio`,
        // so the scale override below has to be that ratio for the box Bevy
        // asks for (`physical / override`) to come back as `physical`.
        let (css_w, css_h, dpr) = client::render::web::viewport().unwrap_or((1280.0, 720.0, 1.0));
        let fit = client::render::web::fit(css_w, css_h, dpr);
        let (pw, ph) = fit.physical;
        client::render::web::stretch_canvas(&canvas, fit.stretch);

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    // **No `.meta` probe.** Bevy's default is `Always`, which
                    // fires a second request for `<path>.meta` before every
                    // asset; there are no `.meta` files in this tree, so on a
                    // page that is one wasted round trip per asset on the
                    // critical path, each answered 404.
                    meta_check: AssetMetaCheck::Never,
                    // Deliberately NOT the desktop's `current_dir()` rewrite
                    // (`bin/gates.rs`): the default is already the relative
                    // string "assets", which the wasm reader joins into a
                    // page-relative URL. A filesystem path here would produce
                    // requests nothing serves.
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        // `bevy_winit` runs `document.query_selector` and
                        // PANICS if it matches nothing, so the element must be
                        // in the document before this call.
                        canvas: Some(canvas.clone()),
                        // ⚠ **The backing store is capped and this is a
                        // no-frame failure, not a quality one.** WebGL2's
                        // `max_texture_dimension_2d` is 2048 and wgpu REFUSES
                        // `configure_surface` above it, while Bevy passes the
                        // resolution straight through with only `.max(1)`. An
                        // ordinary 1080p display at devicePixelRatio 2 asks
                        // for 3840 and gets nothing at all — no error a player
                        // can read, just a canvas that never paints. `fit`
                        // above is the clamp; `web::follow_viewport` keeps it
                        // as the window changes.
                        //
                        // Physical pixels, and the override is `devicePixelRatio`
                        // itself: Bevy asks winit for a CSS box of
                        // `physical / override`, and the observer hands back
                        // `box × devicePixelRatio` — the two cancel exactly
                        // only when the override IS the ratio. It was 1.0 in
                        // the first build, which drew a retina canvas at half
                        // size. Not `fit_canvas_to_parent`: that lets the
                        // observer size the store from the viewport directly,
                        // which is the over-the-cap request in one step.
                        resolution: WindowResolution::new(pw, ph).with_scale_factor_override(dpr),
                        fit_canvas_to_parent: false,
                        ..default()
                    }),
                    ..default()
                }),
        );

        // **Before `add_plugins`, and the order is load-bearing.**
        // `OnEnter(Screen::Loading)` runs before `Startup`, and `sky::setup`
        // takes a non-optional `Res<WorldId>` — so a `WorldId` inserted later
        // does not arrive late, it makes that system silently not run.
        app.insert_resource(WorldId::new(seed));
        // The element winit draws into, for `web::follow_viewport`'s stretch.
        app.insert_resource(client::render::web::Canvas(canvas.clone()));
        app.insert_non_send_resource(Net {
            session,
            sel: 0,
            light: false,
        });

        // `connected: true` is what puts the app in `Screen::Loading` rather
        // than `Screen::Boot` — the plugin already branches on it, so the
        // browser needs no `insert_state` of its own. It is also the truth:
        // the page joined before Bevy existed.
        app.add_plugins(GatesRenderPlugin {
            start: Start {
                direct: server,
                servers_url: None,
                connected: true,
                chosen: true,
                identity: None,
                // There is no launcher in a tab and there never will be.
                no_launcher: true,
                no_hud: false,
            },
            capture: None,
        });
        app.run();
    }
}

/// Turn a join failure into something the page can BRANCH on rather than read.
///
/// **A JS `Error` carrying the shard's own `REFUSE_*` code**, because the
/// alternative is what shipped on 2026-09-10: the page tested
/// `String(e).includes("auth")` against a sentence that says *"sign in through
/// the elo launcher"*, the branch never fired, and a player in a tab was told
/// to open a launcher that cannot exist in one. Matching on prose is the
/// defect; a number the shard sent is not prose.
///
/// `message` is already correct for this platform —
/// `client::refusal_sentence` overrides the shared wording wherever it names
/// an act a page cannot perform — so the page needs no branch at all to say
/// the right thing, and `code` is there for when it wants to say it
/// differently. `crates/client/tests/refusals.rs` gates both halves, including
/// a scan asserting the page still matches on nothing.
///
/// `code` is absent, not zero, on a failure that never reached a shard: `0` is
/// `REFUSE_VERSION`, and a transport error wearing it would be a lie in the
/// one field this exists to make trustworthy.
fn join_error(e: client::JoinError) -> JsValue {
    let err = js_sys::Error::new(&e.to_string());
    if let client::JoinError::Refused(code) = e {
        // Set on the Error rather than thrown as a bare object so the page
        // still gets a stack and `instanceof Error` for everything else.
        let _ = js_sys::Reflect::set(&err, &JsValue::from_str("code"), &JsValue::from(code));
    }
    err.into()
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
