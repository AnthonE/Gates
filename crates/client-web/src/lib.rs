//! Gates in a tab: the module a page loads, and nothing else.
//!
//! The page (`web/app.js`) owns the shard list, the wallet and the join:
//! [`Gates::join`] (or [`Gates::watch`]) connects over `WebTransport` and
//! returns once the shard has said welcome, and [`Gates::play`] hands that
//! session to the same Bevy client the desktop runs, drawing into the page's
//! canvas. From then on Bevy drives the frame loop and the page only watches
//! for it to hand control back (`client::render::web`).
//!
//! **The server needs no change for any of this**: `crates/server/src/net.rs`
//! already serves browser-shaped certificates, and `NETCODE.md` §2.2's P-256
//! and 14-day rules are WebTransport spec rules, written for browsers.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;

/// One joined session, handed to the page until [`Gates::play`] gives it to
/// the renderer.
#[wasm_bindgen]
pub struct Gates {
    session: client::Session,
    /// The `host:port` joined, which the disconnect line names.
    server: String,
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
    /// `identity` is the `0x…` address the player is claiming — or, since v142,
    /// a Solana account's base58 address, whose `sign` answers with its
    /// `signMessage` signature as base58 — and `sign` is
    /// the wallet that proves it: `(text: string) => Promise<"0x…">`, which is
    /// the shape of the platform's own `Deck.wallet.sign(message, address)`
    /// (`watchtower/js/deck-shim.js` in the platform repo) with the address
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
        // An `0x…` address is an Ethereum account; anything else must be a
        // Solana account's base58 address (v142), which signs the Solana form
        // of the same text with its wallet's `signMessage`.
        let solana = declared
            .filter(|a| !a.starts_with("0x") && !a.starts_with("0X"))
            .map(|a| {
                protocol::SolanaKey::from_base58(a.as_bytes()).ok_or_else(|| {
                    JsValue::from_str(
                        "identity is neither an Ethereum address (0x and 40 hex digits) nor a \
                         Solana address (base58)",
                    )
                })
            })
            .transpose()?;
        let address = match declared {
            None => protocol::Address::GUEST,
            Some(_) if solana.is_some() => protocol::Address::GUEST,
            Some(a) => protocol::Address::from_hex(a.as_bytes()).ok_or_else(|| {
                JsValue::from_str(
                    "identity is not an Ethereum address — it must be 0x followed by 40 hex digits",
                )
            })?,
        };
        // **The page signs its own name where it can vouch for the shard.** A
        // wallet compares a sign-in's domain with the page asking, and the
        // page on elopros.com signing `game.elopros.com` is the mismatch it
        // warns about. `page_domain` has the rule (https, not loopback, and
        // the shard is this host or under it); everything else keeps the
        // dialled host, so local dev and other people's shards are unchanged.
        let page = web_sys::window().map(|w| w.location());
        let scheme = page.as_ref().and_then(|l| l.protocol().ok());
        let hostname = page.as_ref().and_then(|l| l.hostname().ok());
        let signs_as = match (&scheme, &hostname) {
            (Some(s), Some(h)) => client::net::handshake::page_domain(s, h, &server),
            _ => None,
        };
        let session = match (solana, sign.as_ref()) {
            (Some(key), Some(f)) => {
                client::Session::connect_solana(&server, cert_hash.as_deref(), key, f, signs_as)
                    .await
            }
            _ => {
                client::Session::connect(
                    &server,
                    cert_hash.as_deref(),
                    address,
                    sign.as_ref(),
                    signs_as,
                )
                .await
            }
        }
        .map_err(join_error)?;
        Ok(Gates { session, server })
    }

    /// Take a **read-only seat** watching `target` on `server` (wire v73;
    /// `NETCODE.md` §2.3), and return once the shard has said whose view it
    /// is. `target` is a wallet address (`0x…`), or absent / `"any"` for any
    /// agent the shard lets you watch.
    ///
    /// **No wallet, and none is asked for**: a watcher is anonymous, signs
    /// nothing and claims nothing. What it may watch is decided by the
    /// TARGET's consent, on the shard — this call cannot widen it. The seat
    /// has no body and no input: [`Gates::play`] draws the watched player's
    /// view from the ordinary snapshot stream, on this machine's GPU.
    ///
    /// A refusal carries the shard's code on `code`, as [`Gates::join`]'s
    /// does (`REFUSE_WATCH` 6, `REFUSE_WATCH_FULL` 7).
    pub async fn watch(
        server: String,
        cert_hash: Option<String>,
        target: Option<String>,
    ) -> Result<Gates, JsValue> {
        let named = target
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("any"));
        let target = match named {
            None => protocol::Address::GUEST,
            Some(t) => protocol::Address::from_hex(t.as_bytes())
                .filter(|a| !a.is_guest())
                .ok_or_else(|| {
                    JsValue::from_str(
                        "spectate must be `any` or a wallet address — 0x followed by 40 hex digits",
                    )
                })?,
        };
        let session = client::Session::watch(&server, cert_hash.as_deref(), target)
            .await
            .map_err(join_error)?;
        Ok(Gates { session, server })
    }

    /// What a seat's label says (`SPECTATING …`), or `undefined` when this
    /// session plays rather than watches.
    #[wasm_bindgen(getter)]
    pub fn watching(&self) -> Option<String> {
        self.session
            .watching
            .as_ref()
            .map(client::ui::spectate::label)
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

    /// Hand the world to Bevy and start drawing. **Consumes the session.**
    ///
    /// `self` by value because the `Session` moves into the ECS: `Net` owns it
    /// for the life of the app, and a `Gates` left behind holding a copy would
    /// be a second owner of a single-consumer lane. The page's handle is gone
    /// after this call, which is the honest shape — the join is over and the
    /// renderer is the thing that is live.
    ///
    /// **Only Bevy pumps the session.** `Session::pump` drains `ClientCore`'s
    /// own-fact rings destructively and `render/input.rs` calls it every
    /// frame; a second driver would see half the hits, toasts and refusals,
    /// which is why this module exports no pump of its own.
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

        let Gates {
            session, server, ..
        } = self;
        let seed = session.welcome.seed;

        // The backing store: the viewport in device pixels, scaled down
        // uniformly under the module's surface cap (WebGL2's 2048, WebGPU's
        // 8192) — `client::render::web::fit_within`, and its header for why the canvas is then STRETCHED back over the
        // viewport with a CSS transform rather than sized to it. Read here,
        // before the window exists, because the first `configure_surface`
        // is the one that must not exceed the cap: winit's `ResizeObserver`
        // sets the backing store from the CSS box times `devicePixelRatio`,
        // so the scale override below has to be that ratio for the box Bevy
        // asks for (`physical / override`) to come back as `physical`.
        let (css_w, css_h, dpr) = client::render::web::viewport().unwrap_or((1280.0, 720.0, 1.0));
        let fit = client::render::web::fit_within(
            css_w,
            css_h,
            dpr,
            client::render::web::surface_cap_px(),
        );
        let (pw, ph) = fit.physical;
        client::render::web::stretch_canvas(&canvas, fit.stretch);

        let mut app = App::new();
        // Before `AssetPlugin`, which keeps a default source already
        // registered: Bevy's own wasm reader panics on a download that breaks
        // mid-body, this one retries it (`render/asset_fetch.rs`).
        app.register_asset_source(
            bevy::asset::io::AssetSourceId::Default,
            client::render::asset_fetch::source("assets"),
        );
        app.add_plugins(
            DefaultPlugins
                .build()
                // **`bevy_audio` off here too, and on this target it was
                // actively costing something.** The engine owns the output
                // through an `AudioWorklet` (`render/audio_web.rs`), so
                // `AudioPlugin` has nothing to mix — no entity has carried an
                // `AudioPlayer` since the cpal seam (2026-09-14). Left
                // enabled it still opened cpal's only wasm host, which is a
                // `setTimeout` loop **on the main thread** allocating a fresh
                // `AudioBuffer` and source node every ~46 ms
                // (`findings/browser-audio-20260913.md` §2) — i.e. exactly the
                // per-period garbage that note blames for the hiccup, running
                // for a mixer with nothing in it. The desktop binaries have
                // disabled it since the seam; this one was missed.
                .disable::<bevy::audio::AudioPlugin>()
                // Bevy's panic plugin REPLACES the hook `start` installed
                // (`console_error_panic_hook`, no chaining), and that hook
                // is the page's only word that a WebGPU module's renderer
                // never started (`window.gatesPanic`). Ours logs the same.
                .disable::<bevy::app::PanicHandlerPlugin>()
                // Nothing in the game reads a gamepad, and on this target
                // gilrs asks `navigator.getGamepads()` for one every frame.
                .disable::<bevy::gilrs::GilrsPlugin>()
                .set(bevy::log::LogPlugin {
                    custom_layer: quiet_frame_span,
                    ..default()
                })
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
        app.insert_resource(WorldId {
            blight_salt: session.welcome.blight_salt,
            ..WorldId::with_haven(seed, *session.core.haven())
        });
        // The element winit draws into, for `web::follow_viewport`'s stretch.
        app.insert_resource(client::render::web::Canvas(canvas.clone()));
        app.insert_non_send_resource(Net {
            session,
            sel: 0,
            light: false,
            farm_sent: None,
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
                pin_hour: None,
                pin_weather: None,
            },
            capture: None,
        });
        app.run();
    }
}

/// Switch off `bevy_render`'s `present_frames` span, the one span Bevy opens
/// every frame without its `trace` feature.
///
/// The wasm logger (`tracing_wasm`, as `LogPlugin` builds it) answers a span's
/// entry with `performance.mark` and its exit with `performance.measure`: two
/// calls into JS a frame, and two User Timing entries a frame that nothing
/// ever clears — about 430,000 an hour of play, held by the tab. A layer that
/// is not interested in a callsite disables it for every layer, so the span
/// is never built at all.
fn quiet_frame_span(_: &mut bevy::app::App) -> Option<bevy::log::BoxedLayer> {
    use bevy::log::tracing::{subscriber::Interest, Metadata, Subscriber};
    use bevy::log::tracing_subscriber::layer::{Context, Layer};

    struct Quiet;
    fn frame_span(m: &Metadata<'_>) -> bool {
        m.is_span() && m.name() == "present_frames"
    }
    impl<S: Subscriber> Layer<S> for Quiet {
        fn register_callsite(&self, m: &'static Metadata<'static>) -> Interest {
            if frame_span(m) {
                Interest::never()
            } else {
                Interest::always()
            }
        }
        fn enabled(&self, m: &Metadata<'_>, _: Context<'_, S>) -> bool {
            !frame_span(m)
        }
    }
    Some(Box::new(Quiet))
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
///
/// It also hands the message to `window.gatesPanic`, if the page installed
/// one: a WebGPU module that panics before its device exists (Bevy's
/// "Unable to find a GPU!") has no error hook yet, and the page is the only
/// thing left that can load the WebGL2 module instead.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        let msg = JsValue::from_str(&format!("gates: panic: {info}"));
        web_sys::console::error_1(&msg);
        let global = js_sys::global();
        if let Ok(f) = js_sys::Reflect::get(&global, &JsValue::from_str("gatesPanic")) {
            if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                let _ = f.call1(&global, &msg);
            }
        }
        // A crash report to the platform, as a beacon that outlives the trap.
        client::render::report::web_crash(info);
    }));
}
