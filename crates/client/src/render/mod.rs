//! The native client's render path (`RENDER.md`). Compiled only under the
//! `render` feature.
//!
//! **Bevy draws; it does not decide.** Every position in here comes from one
//! of two places and there is no third: `ClientCore` (the predictor for the
//! local body, the interpolator for everyone else) or a pure `sim_core`
//! function of the seed. Nothing in this module writes gameplay state, and
//! the only thing it sends back into the sim is `ClientCore::set_input`.
//!
//! What that buys is the reason the pivot is cheap: the browser client
//! reached `terrain::height`, `splat`, `scatter` and `clutter_fill` through a
//! wasm bridge, a worker and a set of typed-array views, and every one of
//! those is a plain function call from here. The world is not ported. It is
//! the same code, called directly.

use bevy::prelude::*;
use sim_core::terrain::{self, Haven, ScatterTable};

use crate::Session;

// The Bevy half of the client's audio. The MODEL is `crate::sound`, which is
// pure and not feature-gated for the same reason `ui` is not: a mixer testable
// only by a windowed run with a sound card is a mixer with no gate.
pub mod audio;
pub mod bodies;
// The boot splash. The window is the first thing a double-click gets now, and
// the launcher handshake and connect happen behind it as states rather than
// before it as preconditions.
pub mod boot;
pub mod capture;
pub mod icons;
// Chat. Not registered on a capture run, like the panels: a gate whose
// frames depend on whether a composer is open is not a gate.
pub mod chat;
pub mod clutter;
pub mod collider_debug;
// The hemisphere sky fill. `rig.rs` owns the light; this owns the arithmetic
// of what a sky-facing face and a ground-facing face each receive, because
// Bevy's `AmbientLight` cannot tell them apart and `ART.md` §4 requires that
// it can.
pub mod fill;
// This frame's own-facts, drained from the core ONCE. Every `pop_*` call in
// the client lives in there — see its header for the merge that made that a
// rule rather than a preference.
pub mod feed;
// The death screen. Dying used to end the session: `dead` was set and read
// by nothing, and `ACT_RESPAWN` had no key.
pub mod death;
// The involuntary disconnect. The shard hanging up mid-play used to leave
// the client in a dead world; `pause::Disconnect` is the verb the PLAYER
// takes, and this is the state for when the shard takes it.
pub mod disconnected;
// The build ghost: the cell being aimed at, and the click that commits it.
pub mod ghost;
// The blue wash over the piece a hammer is aimed at.
pub mod decal;
pub mod highlight;
pub mod tracer;
// The launcher-backed nav entries: the title manifest's fetch, and the click
// that hands NEWS / ITEM STORE / WORKSHOP to the launcher's own window. The
// model is `crate::ui::hub`.
pub mod hub;
pub mod hud;
pub mod input;
pub mod loading;
// The island map. Painted from the same `terrain::splat_from` the ground
// blends by, so the map and the world are one worldgen seen two ways.
pub mod map;
pub mod menu;
pub mod mobs;
// The in-game panels — inventory, crafting, the build wheel. Distinct from
// `ui`, which is the chrome the full-screen MENU screens share: `ui` is what a
// player sees instead of the world, `panels` is what they see on top of it.
pub mod panels;
pub mod pause;
// Discord rich presence: which screen means what, and the handoff to the
// worker. The model — socket, framing, payloads, copy — is `crate::discord`,
// which is pure and unconditional. Dark unless `GATES_DISCORD_APP_ID` is set.
pub mod presence;
pub mod props;
pub mod rig;
pub mod settings;
// The screenshot key. Distinct from `capture`, which is the probe harness:
// this is a player pressing F12 at a moment they chose, so it settles
// nothing and never touches the view. `crate::shot` is the arithmetic half.
pub mod shot;
pub mod sky;
// What players built. Distinct from `props`, which is the world the seed
// makes: this is the world other players made, and it arrives on the wire.
pub mod structures;
pub mod terrain_mesh;
pub mod textures;
// The ground's four identities, each with its own photograph. The first WGSL
// in the tree (`RENDER.md` R4).
pub mod ground_splat;
pub mod tree;
pub mod ui;
// The sea: a graded volume with a swell on it. `reference/WATER.md` is the
// research, `TERRAIN.md` §4 is what it replaces.
pub mod water;
// The in-world keys: what the crosshair is on, and what E/G/H do about it.
pub mod anim;
pub mod verbs;
pub mod viewmodel;

pub use menu::{Menu, Rt, Screen};
pub use settings::Settings;

/// Marks an entity the WORLD owns, as opposed to one a menu owns.
///
/// It exists because leaving a shard is now a state change rather than an
/// `exit(1)`, and a world that is left has to actually go: every ground chunk,
/// every scatter parent, every clutter tile, every remote body, the rig, the
/// sun and the HUD — 81 root entities on the first disconnect this was measured
/// on. Marking the roots is the cheapest complete answer: each module already
/// despawns its own entities when they stream out, and this is the same
/// despawn asked for all of them at once.
///
/// **Roots only.** Everything else hangs off one of them (the viewmodel and
/// the skybox are the camera's, every prop is its chunk parent's), so a
/// recursive despawn of the marked set is the whole world and nothing else.
#[derive(Component)]
pub struct WorldEntity;

/// The connected session.
///
/// A NON-SEND resource on purpose: it owns tokio channel receivers, which are
/// `Send` but not `Sync`, so it cannot be a plain `Resource` — and that is
/// the correct shape anyway. One owner, touched from the main schedule only.
///
/// **The runtime used to live here** and now lives in `menu::Rt`, because a
/// connect attempt has to run on it *before* any session exists — and a
/// failed attempt leaves no `Net` behind to have owned it. `Rt` is inserted
/// once at startup and never removed, so the reader tasks still outlive
/// every session, which was the original reason for holding it.
pub struct Net {
    pub session: Session,
    /// The selected hotbar slot, held here because it is a client-side
    /// latch rather than a per-frame key state.
    pub sel: u8,
}

/// The world's identity, resolved once from the welcome and then read-only.
///
/// `haven` is here because it costs ~1,000 `height` taps to resolve and
/// `scatter` needs it on every cell — `terrain::scatter`'s own doc comment
/// says to hold it rather than re-resolve it, and this is the client's copy
/// of the hold `World` does server-side.
#[derive(Resource)]
pub struct WorldId {
    pub seed: u64,
    pub haven: Haven,
    pub table: ScatterTable,
}

impl WorldId {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            haven: terrain::haven(seed),
            table: ScatterTable::alpha_default(),
        }
    }
}

/// Where the camera is this frame, in world metres. Written by `input`, read
/// by everything that streams around the player, so the ring builders never
/// query the camera transform and never disagree about which frame they are
/// on.
#[derive(Resource, Default)]
pub struct Eye {
    /// **Has the server told us where the player is?**
    ///
    /// False from the welcome until the first snapshot carrying our own
    /// entity, which is what `Predictor::adopt` takes the authoritative
    /// spawn from — the welcome itself carries `player_id`, `seed` and
    /// `tick`, and no position at all.
    ///
    /// It is a flag rather than an `Option<Vec3>` because the failure it
    /// prevents is not a missing value: `Predictor::body` before its first
    /// snapshot is `Body::default()`, whose position is the **world
    /// origin**, and the world origin is a real place on this island. A ring
    /// builder handed it does not fault — it builds a neighbourhood of
    /// somewhere the server never named. See [`crate::ui::load`].
    pub placed: bool,
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// Eye height above the capsule's feet, metres (`DECISIONS.md` §open, client
/// cosmetics — the same 1.6 the browser client stands at).
pub const EYE_HEIGHT: f32 = 1.6;

/// The system set the world-streaming systems run in, after the eye has been
/// placed for the frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Stream;

/// Is there a world to run this frame?
///
/// **Not a state test alone, and that is the whole point.** Every system it
/// gates reads `Net` or `WorldId`, and a missing non-send resource is a panic
/// rather than a skip — so the question has to be "does the world exist",
/// which four of the six screens can answer yes to and two cannot. `Settings`
/// is the case that forces it: opened from the Esc menu there is a live
/// session behind it, opened from the server list there is nothing at all,
/// and it is one state either way.
pub fn world_running(state: Res<State<Screen>>, world: Option<Res<WorldId>>) -> bool {
    world.is_some()
        && !matches!(
            state.get(),
            Screen::Boot | Screen::Menu | Screen::Connecting | Screen::Disconnected
        )
}

/// Has the server told us **what** to load?
///
/// The narrower of the two questions in front of the `Stream` set, and the
/// one `world_running` cannot answer: a `WorldId` exists the moment the
/// welcome names a seed, but a seed is an island and not a place on it. Every
/// streamer in that set reads `Eye::pos` to decide which cells it owes, so
/// running one before the first snapshot builds a ring around the world
/// origin — a real location, silently the wrong one, and evicted whole one
/// packet later.
///
/// Reads `Eye` rather than the session because `Eye` is already the one place
/// the ring builders agree about where the player is (they never query the
/// camera transform, for the same reason). `input::place_eye` is chained
/// ahead of the set, so this sees this frame's answer, not last frame's.
pub fn world_placed(eye: Res<Eye>) -> bool {
    eye.placed
}

/// Drop the world: every entity it spawned, every ring that indexed them, and
/// the two resources that made it a world rather than a menu.
///
/// Runs on entering `Screen::Menu` — both the voluntary disconnect path and
/// the app's first frame — and on entering `Screen::Disconnected`, the
/// involuntary one. On the first frame it finds nothing and does nothing,
/// which is why it needs no "have we ever had a world" flag; the same
/// property is what lets `Disconnected → Menu` run it twice harmlessly.
///
/// **The rings are reset rather than drained.** Each holds a map from a cell
/// to the entity that draws it; a ring that kept its keys after the entities
/// were despawned would report a chunk resident that is not, and the next
/// world's loading screen would sit at a bar it could never fill.
// A parameter per thing the world is: its entities, the five indexes that
// point at them, and the view resources that would otherwise carry the last
// world's eye — or its keypad — into the next one.
#[allow(clippy::too_many_arguments)]
pub fn world_teardown(
    mut commands: Commands,
    entities: Query<Entity, With<WorldEntity>>,
    mut ring: ResMut<terrain_mesh::Ring>,
    mut props: ResMut<props::PropRing>,
    mut clutter: ResMut<clutter::ClutterRing>,
    mut structures: ResMut<structures::StructRing>,
    mut ghost: ResMut<ghost::Ghost>,
    mut highlight: ResMut<highlight::Highlight>,
    mut marks: ResMut<decal::Marks>,
    mut bodies: ResMut<bodies::Bodies>,
    mut herd: ResMut<mobs::Herd>,
    mut eye: ResMut<Eye>,
    mut look: ResMut<input::Look>,
    mut readout: ResMut<hud::Readout>,
    mut pad: ResMut<verbs::Pad>,
) {
    let mut n = 0usize;
    for e in entities.iter() {
        commands.entity(e).despawn();
        n += 1;
    }
    if n == 0 {
        // Nothing was up: the app just started. Say nothing and touch nothing.
        return;
    }
    *ring = terrain_mesh::Ring::default();
    *props = props::PropRing::default();
    *clutter = clutter::ClutterRing::default();
    *structures = structures::StructRing::default();
    // The ghost holds an `Entity` from the world that just went; keeping it
    // would have the next world's first aim insert components onto a dead id.
    *ghost = ghost::Ghost::default();
    // Same reason, same shape: the hammer's wash holds an `Entity` too.
    highlight::forget_in(&mut highlight);
    // And the mark pool, whose entities are NOT `WorldEntity` — spawned at
    // `Startup`, so the despawn above walks past them and last shard's
    // bullet holes would still be standing in the next world.
    decal::forget_in(&mut commands, &mut marks);
    *bodies = bodies::Bodies::default();
    *herd = mobs::Herd::default();
    // The readout holds the LAST wall hit and charge clock — facts about
    // the world that just went, worth up to TOAST_SECS of lies in the next.
    *readout = hud::Readout::default();
    // The keypad addresses a lock in the world that just went; left open it
    // would draw over the next one and eat its digit keys.
    pad.0.close();
    *eye = Eye::default();
    *look = input::Look::default();
    commands.remove_resource::<WorldId>();
    // `Commands` cannot remove a non-send resource, the same asymmetry
    // `menu::poll_connect` goes through the world for on the way in.
    commands.queue(|world: &mut World| {
        world.remove_non_send_resource::<Net>();
    });
    info!("gates: left the world - {n} root entities despawned");
}

/// How the app starts.
///
/// **`WorldId` is no longer built here**, and that is the structural change
/// the menu forced: it needs the seed, the seed comes from the welcome, and
/// the welcome does not exist until something has connected. It is inserted
/// with the session instead — by `menu::poll_connect` when the player picked
/// a shard, or by `gates.rs` when a `--server`/`--capture` run already
/// connected before the window opened. Its presence is also the question
/// `world_running` asks, which is why the two land together.
///
/// A struct rather than a two-variant enum, because the menu's inputs are
/// the same either way: **`connected` changes which screen opens first, not
/// what the menu would be made of.** The enum shape carried `direct` twice
/// and dropped `servers_url` on the connected arm, which would have left a
/// future disconnect (`NOW.md` §0v) showing "no shard list to fetch" to a
/// player who had passed one.
pub struct Start {
    /// The address the "Direct" row carries — always a real one, never empty.
    pub direct: String,
    /// The shard list to fetch, if any.
    pub servers_url: Option<String>,
    /// `gates.rs` already connected before the window, so open on the loading
    /// screen. **Only `--capture` arrives this way now**, and the module doc
    /// on `boot` has the argument: a probe harness must not photograph a
    /// half-finished handshake, so its connect stays a precondition, while a
    /// launcher join became a state (`chosen`) so that a dead shard lands on
    /// the server list instead of on a stderr nobody is reading.
    pub connected: bool,
    /// A shard was named on the command line or by the launcher, and the
    /// player must not be asked to pick again. The splash hands straight to
    /// `Connecting` (`crate::ui::boot::Next`).
    pub chosen: bool,
    /// `--identity`, for the launcher handshake the splash now runs.
    pub identity: Option<String>,
    /// Nothing to ask a launcher — `--no-launcher`, or a start that already
    /// resolved its player before the window.
    pub no_launcher: bool,
    /// `--no-hud`: a capture run that shoots a clean PLATE — no HUD, no
    /// viewmodel, no compass. The menu backdrop is footage
    /// (`ui::backdrop`), and a frame with a hotbar across it is not footage.
    pub no_hud: bool,
}

pub struct GatesRenderPlugin {
    pub start: Start,
    /// Where captures go, if this is a capture run.
    pub capture: Option<std::path::PathBuf>,
}

impl Plugin for GatesRenderPlugin {
    fn build(&self, app: &mut App) {
        // Copied out so the `run_if` closures below capture a `bool` rather
        // than borrowing `self`, which does not outlive `build`.
        let plate = self.start.no_hud;
        // **The probe's hour is pinned; a player's is the server's.** Same
        // rule as the windowed pin two fields down — a capture run takes the
        // defaults wholesale except where the box would otherwise decide what
        // the frame looks like, and until this landed the sun's height was a
        // function of how long the build took (`rig::DayPin`).
        let day_pin = if self.capture.is_some() {
            rig::DayPin::capture()
        } else {
            rig::DayPin::default()
        };
        // The ground's splat material. `MaterialPlugin` is what registers the
        // pipeline and the asset type; without it the ground draws with no
        // material at all, which — as the asset-root trap in `bin/gates.rs`
        // records — is not an error the image shows you.
        app.add_plugins(MaterialPlugin::<ground_splat::GroundMaterial>::default());
        app.insert_resource(day_pin)
            .init_resource::<Eye>()
            .init_resource::<collider_debug::ShowColliders>()
            .init_resource::<input::Look>()
            .init_resource::<terrain_mesh::Ring>()
            .init_resource::<props::PropRing>()
            .init_resource::<clutter::ClutterRing>()
            .init_resource::<structures::StructRing>()
            .init_resource::<bodies::Bodies>()
            .init_resource::<mobs::Herd>()
            .init_resource::<menu::Picked>()
            .init_resource::<menu::Browse>()
            .init_resource::<hub::HubState>()
            .init_resource::<boot::Who>()
            .init_resource::<pause::Chosen>()
            .init_resource::<viewmodel::Motion>()
            .init_resource::<verbs::Aimed>()
            .init_resource::<verbs::Swung>()
            .init_resource::<verbs::InWeak>()
            .init_resource::<verbs::Near>()
            .init_resource::<verbs::Pad>()
            .init_resource::<death::Answer>()
            .init_resource::<disconnected::Reason>()
            .init_resource::<disconnected::Chosen>()
            .init_resource::<ghost::Ghost>()
            .init_resource::<highlight::Highlight>()
            .init_resource::<tracer::Tracers>()
            .init_resource::<decal::Marks>()
            .init_resource::<hud::Toast>()
            .init_resource::<hud::Readout>()
            .init_resource::<feed::Feed>()
            .init_resource::<audio::Sound>()
            .init_resource::<audio::LastHp>()
            .init_resource::<water::Sea>()
            .insert_non_send_resource(menu::Connecting::default());

        // Settings come off disk ONCE, here — before the first frame, so the
        // fov, vsync and volumes a player picked last run are what the first
        // frame applies (`settings::apply_view`/`apply_window` run every
        // Update). A capture run loads nothing and saves nothing: the visual
        // gate's frames must not depend on the box's config file, so it takes
        // the defaults and gets no `Disk` — which is also what makes
        // `save_on_change` a no-op there.
        if self.capture.is_none() {
            let (settings, favourites, disk) = settings::load();
            app.insert_resource(settings);
            // The starred shards come off the same file and land on the
            // browser's own resource — a favourite is not a knob, and
            // `settings::save_on_change` is the one writer for both.
            app.insert_resource(menu::Browse {
                favourites,
                ..default()
            });
            if let Some(disk) = disk {
                app.insert_resource(disk);
            }
        } else {
            app.insert_resource(Settings {
                // The defaults, with ONE pinned: a capture run stays
                // windowed however the shipping default moves. The probe's
                // frame is the visual gate's unit, and
                // `WindowMode::BorderlessFullscreen` would size it to
                // whatever `Xvfb -screen` this box was started with — the
                // same "a frame must not depend on the box" rule that makes
                // the branch above load no file.
                fullscreen: false,
                ..default()
            });
        }

        // `Menu` is inserted either way, because a system that reads it must
        // not care which door the app came through — and the disconnect that
        // returns to it is what `pause` now spends a verb on.
        //
        // **Every start that is not a capture run opens on `Boot`.** The
        // splash is the window a double-click gets, and what it hands off to
        // is one bit — `chosen` — resolved in `crate::ui::boot` rather than
        // here. A capture run still opens on `Loading`, because its connect
        // happened before the window and its rings take ~25 frames to fill:
        // entering `InWorld` on frame one would enter a state whose `OnEnter`
        // no longer builds anything, and photograph an empty world.
        app.insert_state(if self.start.connected {
            Screen::Loading
        } else {
            Screen::Boot
        })
        .insert_resource(Menu::new(
            &self.start.direct,
            self.start.servers_url.clone(),
        ))
        .insert_resource(boot::Direct(self.start.direct.clone()))
        .insert_resource(boot::Warmup::new(
            self.start.chosen,
            self.start.identity.clone(),
            // A capture run has already resolved its player and must not
            // reach for a socket outside the repo — a gate whose result
            // depends on what else is running on the box is not a gate.
            self.start.no_launcher || self.start.connected,
        ));
        // The direct address is also what the loading and pause screens name,
        // and on a capture start nothing has been "picked" — so the field
        // those screens read is seeded here rather than left empty. Every
        // other start fills it in `boot::teardown` on the way out of the
        // splash.
        if self.start.connected {
            if let Some(mut c) = app
                .world_mut()
                .get_non_send_resource_mut::<menu::Connecting>()
            {
                c.addr = self.start.direct.clone();
            }
        }

        // Textures load at Startup rather than on entering the world: they
        // are wanted whichever screen comes first, and warming them while a
        // player reads the menu is free time the old shape did not have.
        app.add_systems(
            Startup,
            (
                textures::load,
                icons::load,
                anim::load,
                mobs::load,
                // The held-item models. Loaded once here rather than per
                // swap: `AssetServer` dedups, but a `load` still walks and
                // hashes a path, and `viewmodel::swap` runs every frame.
                viewmodel::load_models,
                // The tracer pool. Spawned once here so the frame path
                // never spawns an entity for an arrow (`tracer.rs`).
                tracer::setup,
                // The mark pool, for the same reason plus one more: the
                // materials it builds here are what the prewarm draw
                // specializes, and a pipeline compiled mid-fight is the
                // stall `decal.rs`'s `PREWARM_FRAMES` exists to avoid.
                decal::setup,
                // The menu's footage. Wanted on the first screen after the
                // splash, so it warms while everything else does.
                ui::load_backdrop,
            ),
        );
        // The sound bank is generated rather than loaded (`sound/synth.rs`)
        // and is built HERE, not at `Startup`. **`OnEnter(Screen::Loading)`
        // runs before `Startup`** on a connected start — Bevy schedules the
        // first state transition with `insert_startup_before(PreStartup, …)` —
        // so a `Startup` system cannot supply a resource that an `OnEnter`
        // system reads, and `audio::setup` reads the bank. See
        // `audio::build_bank`; the first capture run after the audio slice
        // died on exactly this.
        audio::build_bank(app);
        // The build wheel's rings, rasterised once. Ten images, and the
        // reason they are not made on demand is that the wheel rebuilds every
        // time the pointer crosses a wedge — several times a second while
        // sweeping — for a thing with ten possible states.
        panels::ring::build_rings(app);
        // The two UI faces, for the same reason and at the same moment: the
        // loading screen draws text before `Startup` runs, and a font that is
        // not there yet draws NOTHING — not a fallback glyph. `ui::build_fonts`
        // has the whole argument for compiling them in.
        ui::build_fonts(app);

        // ---- the boot splash -----------------------------------------
        // The first screen a double-click gets. `update` is the only system
        // that can leave it, and it leaves on observable state — see `boot`.
        app.add_systems(OnEnter(Screen::Boot), (boot::begin_greet, boot::setup))
            .add_systems(OnExit(Screen::Boot), boot::teardown)
            .add_systems(Update, boot::update.run_if(in_state(Screen::Boot)));

        // ---- the menu ------------------------------------------------
        // `world_teardown` first: entering the menu from a live world is the
        // disconnect path, and the menu must not be built over a world that
        // is still drawing behind it.
        app.add_systems(
            OnEnter(Screen::Menu),
            (world_teardown, menu::begin_fetch, menu::setup).chain(),
        )
        .add_systems(OnExit(Screen::Menu), menu::teardown)
        .add_systems(
            Update,
            (
                menu::poll_fetch,
                // The title manifest, beside the shard list: both are
                // documents a menu waits on, both raise a dirty flag, and
                // `menu::rebuild` at the end of this chain is the one redraw.
                hub::poll,
                // The count half, after the list half: `poll_fetch` is what
                // creates the rows a poll addresses by index, and both raise
                // the one `dirty` flag `rebuild` below acts on.
                menu::begin_status_poll,
                menu::poll_status,
                menu::click,
                menu::keys,
                menu::take_pick,
                // Last, so a click, a keystroke and a landed fetch all reach
                // the screen on the frame they happen rather than the next.
                menu::rebuild,
            )
                .chain()
                .run_if(in_state(Screen::Menu)),
        )
        .add_systems(
            OnEnter(Screen::Connecting),
            (menu::begin_connect, menu::connecting_screen),
        )
        .add_systems(OnExit(Screen::Connecting), menu::teardown)
        .add_systems(
            Update,
            menu::poll_connect.run_if(in_state(Screen::Connecting)),
        );

        // ---- the loading screen --------------------------------------
        // `loading::update` runs AFTER the streamers (`Stream`), so the bar
        // reports this frame's rings rather than last frame's — and so the
        // frame that finishes the ring is the frame that ends the screen.
        app.add_systems(OnEnter(Screen::Loading), loading::setup)
            .add_systems(OnExit(Screen::Loading), loading::teardown)
            .add_systems(
                Update,
                loading::update
                    .after(Stream)
                    .run_if(in_state(Screen::Loading)),
            );

        // ---- the Esc menu --------------------------------------------
        app.add_systems(
            OnEnter(Screen::Paused),
            (pause::enter, pause::setup).chain(),
        )
        .add_systems(OnExit(Screen::Paused), pause::teardown)
        .add_systems(
            Update,
            (pause::click, pause::keys, pause::act)
                .chain()
                .run_if(in_state(Screen::Paused)),
        )
        .add_systems(Update, pause::open.run_if(in_state(Screen::InWorld)));

        // ---- the death screen ----------------------------------------
        // `watch` runs in `InWorld` and nowhere else: a death that lands
        // while the Esc menu is up raises the screen on resume, which is the
        // right order — two full-screen states cannot both be entered.
        app.add_systems(OnEnter(Screen::Dead), (death::enter, death::setup).chain())
            .add_systems(OnExit(Screen::Dead), death::teardown)
            .add_systems(
                Update,
                (death::click, death::keys, death::act, death::awaken)
                    .chain()
                    .run_if(in_state(Screen::Dead)),
            )
            .add_systems(Update, death::watch.run_if(in_state(Screen::InWorld)));

        // ---- the involuntary disconnect ------------------------------
        // `watch` is ungated: its guard is `Net`'s presence (the module doc
        // says why that is exactly the right set of states), and it runs
        // after `place_eye` so it reads the latch the frame's own pump set
        // rather than last frame's. Entry runs the SAME teardown chain the
        // menu runs — the session under the world is dead, so the world
        // goes before the screen is built, not when the player clicks
        // through — and `setup` follows it in the chain so the reason line
        // it draws was captured by `watch` before `Net` went away.
        app.add_systems(Update, disconnected::watch.after(input::place_eye))
            .add_systems(
                OnEnter(Screen::Disconnected),
                (world_teardown, disconnected::setup).chain(),
            )
            .add_systems(
                OnEnter(Screen::Disconnected),
                audio::teardown.after(world_teardown),
            )
            .add_systems(
                OnEnter(Screen::Disconnected),
                water::teardown.after(world_teardown),
            )
            .add_systems(
                OnEnter(Screen::Disconnected),
                (map::forget, viewmodel::forget),
            )
            .add_systems(OnExit(Screen::Disconnected), disconnected::teardown)
            .add_systems(
                Update,
                (disconnected::click, disconnected::keys, disconnected::act)
                    .chain()
                    .run_if(in_state(Screen::Disconnected)),
            );

        // ---- the map -------------------------------------------------
        // **Hold `G`.** `open` is ordered after the panels and after chat,
        // and that ordering is necessary but was never sufficient — the claim
        // here used to be that a letter typed into a search box or a composer
        // is theirs because "both consume the press before this sees it", and
        // only half of that was ever true. `chat::keys` does consume it: it
        // clears the whole keyboard while the composer is up. `panels::keys`
        // clears **only `Escape`**, and the inventory search box reads
        // `KeyboardInput` messages rather than `ButtonInput`, so the press
        // survives — which is why typing `m` into the crafting search used to
        // open the map. `map::open` carries its own guard now and does not
        // rely on being downstream of anything.
        app.init_resource::<map::Island>()
            .add_systems(OnEnter(Screen::Map), (map::enter, map::setup).chain())
            .add_systems(OnExit(Screen::Map), (map::teardown, map::leave))
            .add_systems(
                Update,
                (map::track, map::keys)
                    .chain()
                    .run_if(in_state(Screen::Map)),
            )
            .add_systems(
                Update,
                map::open
                    .after(pause::open)
                    .run_if(in_state(Screen::InWorld)),
            )
            .add_systems(OnEnter(Screen::Menu), (map::forget, viewmodel::forget));

        // ---- settings ------------------------------------------------
        // The two `apply_*` systems are deliberately NOT gated on the screen
        // being open: a setting is a property of the client, not of the panel
        // that changed it, and the camera it applies to may not exist until
        // two states later. `save_on_change` is ungated for the same reason —
        // it watches the resource, not the screen — and it self-gates on the
        // `Disk` resource, which a capture run never gets.
        app.add_systems(OnEnter(Screen::Settings), settings::setup)
            .add_systems(OnExit(Screen::Settings), settings::teardown)
            .add_systems(
                Update,
                (settings::click, settings::rebuild, settings::keys)
                    .chain()
                    .run_if(in_state(Screen::Settings)),
            )
            .add_systems(
                Update,
                (
                    settings::apply_view,
                    settings::apply_window,
                    settings::save_on_change,
                ),
            )
            // **The frame cap, and it must be `Last` and unconditional.**
            // `Last` because a cap has to be the final thing a frame does —
            // registered in `Update` it would sleep before the render it is
            // pacing. Unconditional because the screen that wasted the most
            // was the MENU: Bevy's focused update mode is `Continuous`, so
            // with vsync off a still image was redrawn as fast as the
            // hardware allowed, and a cap that only ran in-world would have
            // left exactly that case uncapped.
            .init_resource::<settings::FrameDeadline>()
            .add_systems(Last, settings::limit_frames);

        // One hover handler for every screen that has buttons on it.
        app.add_systems(Update, ui::hover);

        // ---- the world -----------------------------------------------
        // Every one of these reads `WorldId` or `Net`, neither of which
        // exists before the welcome. `OnEnter` is what makes that safe:
        // as `Startup` systems they would have run against a resource that
        // was not there yet and panicked on the first frame of the menu.
        //
        // **They hang off `Loading`, not `InWorld`**, because the loading
        // screen is where the world is built — the rig has to be up for the
        // streamers to have somewhere to put chunks, and the 3D pass has to
        // be running for the pipelines to specialize before the player is
        // looking at the result.
        app.add_systems(
            OnEnter(Screen::Loading),
            // The sea replaced `terrain_mesh::setup_water` here: it builds one
            // eye-centred mesh and a ripple map rather than a plane, and it
            // needs nothing the rig owns.
            (rig::setup, water::setup),
        )
        // The HUD's viewmodel is parented to the camera, so it must be
        // built after the rig has spawned one.
        //
        // **A plate run spawns none of it.** Every HUD system reads its
        // entities through a guarded `single()`, so an absent HUD is a set of
        // no-ops rather than a panic — which is what makes `--no-hud` two
        // conditions here instead of a mode inside `capture.rs`. The harness
        // itself is untouched, so the gate's own frames cannot move.
        .add_systems(
            OnEnter(Screen::Loading),
            hud::setup.after(rig::setup).run_if(move || !plate),
        )
        // Both in `Update` and NOT on the `Loading` transition — that
        // transition runs before `Startup`, so `PropMaps` does not exist yet
        // (see `viewmodel::spawn_item`). `animate` runs after `feed::drain`,
        // because the swing is triggered by a fact the drain publishes and
        // the other order reacts a frame late.
        .add_systems(
            Update,
            (
                viewmodel::spawn_item,
                viewmodel::animate
                    .after(feed::drain)
                    .after(viewmodel::spawn_item),
                // What is in the hand. After the spawn for the obvious
                // reason; it writes only handles and visibility where
                // `animate` writes only a transform, so the two never
                // contend for one entity and need no order between them.
                viewmodel::swap.after(viewmodel::spawn_item),
                // The tracer's two halves. `launch` reads the drained feed,
                // so it must follow the drain for the swing's reason —
                // the other order reacts a frame late. `fly` then advances
                // whatever is live, including the shot just claimed, so a
                // tracer's first frame already shows motion.
                tracer::launch.after(feed::drain),
                tracer::fly.after(tracer::launch),
                // The mark's two halves, the tracer's shape exactly.
                // `mark` reads the drained feed so it follows the drain;
                // `fade` then ages everything including the mark just
                // claimed, which is what releases the prewarm slot.
                decal::mark.after(feed::drain),
                decal::fade.after(decal::mark),
            )
                .run_if(world_running)
                .run_if(move || !plate),
        )
        // The rig. `build` runs until the glTF is in and then costs one
        // branch; `bind` catches every `AnimationPlayer` the scene spawner
        // adds, and runs AFTER `Stream` because the body it walks up to is
        // spawned by `bodies::stream` inside that set.
        .add_systems(
            Update,
            (
                anim::build,
                anim::bind.after(Stream),
                anim::reshade.after(Stream),
                anim::drive.after(anim::bind),
            )
                .run_if(world_running),
        )
        // The cloud deck hangs on the camera, so it waits for the rig too.
        .add_systems(OnEnter(Screen::Loading), sky::setup.after(rig::setup))
        // The listener IS the camera, so the ears wait for the rig as well.
        .add_systems(OnEnter(Screen::Loading), audio::setup.after(rig::setup))
        // **The score runs everywhere, which is why it is registered on its
        // own and not with the audio block below.** `sound::music` is a
        // gap-and-intensity director (`reference/AUDIO.md` §8) and the menus
        // are one of the two places it runs: continuous there, four to eight
        // minutes apart in a world. Nothing it touches belongs to a world —
        // no `Net`, no `Eye`, no listener — so it needs no run condition at
        // all, and a music voice is deliberately not a `WorldEntity` so a
        // piece can ring out across the transition.
        .add_systems(Update, audio::music)
        .add_systems(
            OnEnter(Screen::Menu),
            audio::music_mode(crate::sound::music::Mode::Menu),
        )
        .add_systems(
            OnEnter(Screen::Loading),
            audio::music_mode(crate::sound::music::Mode::World),
        )
        // Leaving a shard resets the step odometer and the bed's fade. The
        // bed entity itself is a `WorldEntity` and goes with the rest.
        .add_systems(OnEnter(Screen::Menu), audio::teardown.after(world_teardown))
        // The sea's caches are one island's depths; the next island's would
        // be read off them until the eye happened to cross a snap cell.
        .add_systems(OnEnter(Screen::Menu), water::teardown.after(world_teardown))
        // The swell runs wherever the world runs — it is a surface, not a
        // streamer, and a sea that froze while the Esc menu was up would
        // resume with a visible jump in every wave.
        .add_systems(Update, water::animate.run_if(world_running))
        // Input writes what the sim reads, so it runs on the two screens where
        // the player is still *in* the world and nowhere else: a player
        // reading a settings pane must not be swinging an axe.
        //
        // **`Map` is the second screen, and it has to be.** The map is held
        // rather than toggled now, so the player is running while it is up —
        // and `ClientCore::set_input` is a LATCH that `advance` re-emits every
        // tick. Stop feeding it and the body keeps walking on whatever keys
        // were down when the map opened, forever, until the map closes. That
        // was already a live bug on the `M` toggle (`pause::enter` and
        // `death::enter` both zero the latch on their way in; `map::enter`
        // never did) and a hold would have made it the common case instead of
        // the odd one. Keeping `gather` alive fixes it at the source: the
        // latch stays honest, and letting go of `W` stops the body.
        //
        // Only `gather`. `verbs::keys` and the ghost stay `InWorld`, so no
        // door opens and nothing is placed while the map is up.
        .add_systems(
            Update,
            input::gather
                .before(input::place_eye)
                .run_if(in_state(Screen::InWorld).or(in_state(Screen::Map))),
        )
        // The in-world verbs. `InWorld` for the same reason `gather` is: every
        // one of them spends something, and a player reading a settings pane
        // asked for none of it. `keys` runs AFTER `resolve` so the press acts
        // on the pick the prompt is currently showing — resolving twice is how
        // a prompt and its verb come to disagree.
        .add_systems(
            Update,
            (verbs::resolve, verbs::keys)
                .chain()
                .after(input::place_eye)
                .run_if(in_state(Screen::InWorld)),
        )
        // The build ghost. `track` before `place_key` for the same reason
        // `verbs::resolve` precedes `verbs::keys`: the click commits what is
        // drawn, so the drawing has to be this frame's.
        .add_systems(
            Update,
            (
                ghost::level_keys,
                ghost::track,
                ghost::deploy_track,
                ghost::place_key,
                ghost::deploy_key,
                // After `verbs::resolve` has answered what is aimed at, and
                // before the click that acts on it.
                highlight::track,
            )
                .chain()
                .after(input::place_eye)
                .run_if(in_state(Screen::InWorld)),
        )
        // Everything else runs wherever the world exists — loading, playing,
        // paused, or reading settings from the pause menu. `place_eye` pumps
        // the session, and a session that stops being read is a connection
        // that stalls and then teleports on resume.
        //
        // **`place_eye` pumps unconditionally; everything downstream waits
        // to be placed.** The two halves are gated differently on purpose:
        // pumping is how the snapshot that does the placing arrives, so a
        // condition that stopped it would be a deadlock, while every system
        // in `Stream` reads `Eye::pos` and would otherwise stream the world
        // origin (`world_placed`). The chain is what makes the condition
        // read this frame's pump rather than the previous frame's.
        .add_systems(
            Update,
            (
                input::place_eye,
                (
                    terrain_mesh::stream,
                    // The sea re-centres like a ring does, and for the same
                    // reason: it reads `Eye::pos`, so it belongs where the
                    // other things that read it are.
                    water::stream,
                    props::stream,
                    props::harvest,
                    // After `harvest`, which owns the discrete transition and
                    // arms the topple this integrates. Reversed, a tree would
                    // spend one frame at the pose the previous chop left it.
                    props::fall,
                    clutter::stream,
                    structures::stream,
                    bodies::stream,
                    mobs::stream,
                    // The legs read the gait `mobs::stream` just advanced.
                    mobs::trot,
                    rig::follow_eye,
                    hud::update,
                    // The feedback surface. Under `world_running` rather than
                    // `InWorld`: a refusal that arrived while the Esc menu was
                    // up is still owed to the player, and a ring nobody drains
                    // is a ring that overflows and drops the newest.
                    hud::feedback,
                    // The pinned readout: `Feed`'s second HUD reader,
                    // which the drain architecture exists to make free
                    // (`feed.rs` — a reader borrows, only the drain pops).
                    hud::readout,
                    hud::prompt,
                    // The netcode readout under the build stamp. Reads the
                    // predictor's own counters, which until now were computed
                    // every snapshot and displayed nowhere — see `NetLine`.
                    hud::net_line,
                    // F3: draw what the SIM blocks over what the client
                    // draws. Not a gate and not a probe — it does nothing
                    // until a person presses the key.
                    collider_debug::toggle,
                    collider_debug::draw,
                    // The keypad's small panel, beside the prompt that
                    // goes quiet while it is up. HUD, not `panels::` — it
                    // must not grab the pointer, so it runs on a capture
                    // build too (where the pad simply never opens).
                    hud::pad_overlay,
                )
                    .in_set(Stream)
                    .run_if(world_placed),
            )
                .chain()
                .run_if(world_running),
        )
        // **The one `pop_*` call site in the client.** `hud::feedback`
        // (inside `Stream`) and `audio::feed` (after it) both want this
        // frame's hits, toasts and refusals, and the core hands each fact
        // over exactly ONCE — so when both popped, the HUD drained every ring
        // and the game fell silent, with no conflict and no failing test to
        // say so. `feed::drain` fills a resource both read immutably;
        // `render/feed.rs` has the account. It is gated on `world_running`
        // rather than `world_placed` because a ring nobody drains overflows,
        // which is the reason `hud::feedback` gives for its own placement.
        //
        // **Ordered against the pump explicitly, both edges.** `place_eye`
        // pumps the session (rings filled, `applied` word raised), the drain
        // takes word and rings in one move, `Stream`'s readers see one
        // coherent frame. Until 2026-08-15 only `.before(Stream)` was stated
        // and drain-after-pump held by insertion order alone; the other
        // schedule splits the word from the facts across frames, and a
        // reader latching a stale `applied` bit beside freshly-pumped state
        // reports one fact twice — the consume rings took the data off the
        // latch for exactly that collapse, and the latched facts that remain
        // (`struct_hit`, `charge_placed`, `stock`, `last_drink`) still live
        // on the word being the same frame's as the fields.
        .add_systems(
            Update,
            feed::drain
                .after(input::place_eye)
                .before(Stream)
                .run_if(world_running),
        )
        // The rig follows the server's clock (day/night v0). After the
        // drain so it reads this frame's tick estimate, not last frame's.
        .add_systems(
            Update,
            rig::day_night.after(feed::drain).run_if(world_running),
        )
        // Audio runs AFTER the streamers and `pump` runs last of all: every
        // producer must have had its say before the mixer resolves the frame,
        // or a cue requested by a system scheduled later is heard a frame
        // late. `fell` in particular reads the change detection that
        // `props::harvest` writes inside `Stream`.
        .add_systems(
            Update,
            (
                // `water` first of the audio systems: it resolves the frame's
                // snapshot, and both `bed` and `pump` scale everything they
                // do by it.
                audio::water,
                audio::feed,
                // The second positional cue: placements off the feed's
                // broadcast-only ring (the join-flood guard is the core's).
                audio::place,
                audio::hurt,
                audio::steps,
                // Everyone else's, off the interpolated bodies `Stream`
                // just moved — positional, culled by the mixer's falloff.
                audio::remote_steps,
                audio::fell,
                // The herd's voices, off the animals `mobs::stream` just
                // moved — a snort, a howl or a growl, by species and range.
                audio::voices,
                audio::bed,
                audio::pump,
            )
                .chain()
                .after(Stream)
                .run_if(world_running),
        );

        // ---- the menus -----------------------------------------------
        // **Not on a capture run**, and that is a rule rather than a
        // convenience: a probe harness that could open a panel is a visual
        // gate whose frames depend on which key was last pressed. The cost
        // is that nothing photographs these panels — the same missing menu
        // vantage `NOW.md` §0v already names, now owed twice.
        if self.capture.is_none() {
            panels::register(app);
            chat::register(app);
            // ---- the screenshot key ----------------------------------
            // **Not on a capture run either**, and for the same reason one
            // line up: the probe harness spawns its own `Screenshot`
            // entities on a fixed schedule, and a second writer of the same
            // frame is a gate whose frames depend on which key was pressed.
            //
            // No state gate, unlike almost everything else here. The menu,
            // the map and the death screen are all worth photographing, and
            // a key that works on four screens out of nine is a key a player
            // has to remember the rules for.
            app.init_resource::<shot::Shots>()
                .add_systems(Update, shot::take);

            // ---- discord rich presence -------------------------------
            // **Not on a capture run either**, and this one is the
            // strongest case of the three: the probe would open a socket to
            // whatever Discord happens to be running on the box, which is a
            // gate whose behaviour depends on who is logged in.
            //
            // Registers nothing at all unless `GATES_DISCORD_APP_ID` names
            // an application — dark is the shipping default, because the
            // application is an operator act and there is no id in this
            // tree (`crate::discord`).
            if presence::register(app) {
                info!("discord presence: live");
            }
        }

        if let Some(dir) = &self.capture {
            let _ = std::fs::create_dir_all(dir);
            app.insert_resource(capture::Capture::new(dir.clone()));
            // Ahead of `gather`, because it owns the view on a capture run
            // and `gather` must not fight it for the same frame. Gated on
            // `world_running` rather than on `InWorld`: a capture run now
            // opens on `Loading` like every other connected start, and this
            // has to keep aiming at the first vantage while the rings fill —
            // those are the frames that warm the pipelines the shots are
            // taken on.
            app.add_systems(
                Update,
                capture::drive.before(input::gather).run_if(world_running),
            );
        }
    }
}
