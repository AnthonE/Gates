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
pub mod capture;
pub mod icons;
// Chat. Not registered on a capture run, like the panels: a gate whose
// frames depend on whether a composer is open is not a gate.
pub mod chat;
pub mod clutter;
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
pub mod highlight;
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
pub mod props;
pub mod rig;
pub mod settings;
pub mod sky;
// What players built. Distinct from `props`, which is the world the seed
// makes: this is the world other players made, and it arrives on the wire.
pub mod structures;
pub mod terrain_mesh;
pub mod textures;
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
            Screen::Menu | Screen::Connecting | Screen::Disconnected
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
// Nine parameters because the world is nine things: its entities, the five
// indexes that point at them, and the two view resources that would otherwise
// carry the last world's eye into the next one.
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
    mut bodies: ResMut<bodies::Bodies>,
    mut herd: ResMut<mobs::Herd>,
    mut eye: ResMut<Eye>,
    mut look: ResMut<input::Look>,
    mut readout: ResMut<hud::Readout>,
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
    *bodies = bodies::Bodies::default();
    *herd = mobs::Herd::default();
    // The readout holds the LAST wall hit and charge clock — facts about
    // the world that just went, worth up to TOAST_SECS of lies in the next.
    *readout = hud::Readout::default();
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
    /// `gates.rs` already connected, so skip the server list and open on the
    /// loading screen. The probe harness and a launcher join both arrive this
    /// way: the first must not wait on a click, the second has already chosen.
    pub connected: bool,
}

pub struct GatesRenderPlugin {
    pub start: Start,
    /// Where captures go, if this is a capture run.
    pub capture: Option<std::path::PathBuf>,
}

impl Plugin for GatesRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Eye>()
            .init_resource::<input::Look>()
            .init_resource::<terrain_mesh::Ring>()
            .init_resource::<props::PropRing>()
            .init_resource::<clutter::ClutterRing>()
            .init_resource::<structures::StructRing>()
            .init_resource::<bodies::Bodies>()
            .init_resource::<mobs::Herd>()
            .init_resource::<menu::Picked>()
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
            let (settings, disk) = settings::load();
            app.insert_resource(settings);
            if let Some(disk) = disk {
                app.insert_resource(disk);
            }
        } else {
            app.init_resource::<Settings>();
        }

        // `Menu` is inserted either way, because a system that reads it must
        // not care which door the app came through — and the disconnect that
        // returns to it is what `pause` now spends a verb on.
        //
        // **A connected start opens on `Loading`, not `InWorld`**, and that
        // includes `--capture`: the world's rings take ~25 frames to fill
        // whoever asked for them, and the state that owns that interval is
        // the one that owns the rig. A capture run entering `InWorld` on
        // frame one would enter a state whose `OnEnter` no longer builds
        // anything, and photograph an empty world.
        app.insert_state(if self.start.connected {
            Screen::Loading
        } else {
            Screen::Menu
        })
        .insert_resource(Menu::new(
            &self.start.direct,
            self.start.servers_url.clone(),
        ));
        // The direct address is also what the loading and pause screens name,
        // and on a `--server` start nothing has been "picked" — so the field
        // those screens read is seeded here rather than left empty.
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
            (textures::load, icons::load, anim::load, mobs::load),
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
                menu::rebuild_on_new_rows,
                menu::refresh_status,
                menu::click,
                menu::keys,
                menu::take_pick,
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
            .add_systems(OnEnter(Screen::Disconnected), map::forget)
            .add_systems(OnExit(Screen::Disconnected), disconnected::teardown)
            .add_systems(
                Update,
                (disconnected::click, disconnected::keys, disconnected::act)
                    .chain()
                    .run_if(in_state(Screen::Disconnected)),
            );

        // ---- the map -------------------------------------------------
        // `open` is registered after the panels and after chat, so `M` typed
        // into a search box or a chat composer is theirs — both consume the
        // press before this sees it.
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
            .add_systems(OnEnter(Screen::Menu), map::forget);

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
            );

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
        .add_systems(OnEnter(Screen::Loading), hud::setup.after(rig::setup))
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
            )
                .run_if(world_running),
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
        // Input is the one thing that is `InWorld` and nothing else: it is
        // the only system that writes what the sim reads, and a player
        // reading a settings pane must not be swinging an axe.
        .add_systems(
            Update,
            input::gather
                .before(input::place_eye)
                .run_if(in_state(Screen::InWorld)),
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
                    clutter::stream,
                    structures::stream,
                    bodies::stream,
                    mobs::stream,
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
                )
                    .in_set(Stream)
                    .run_if(world_placed),
            )
                .chain()
                .run_if(world_running),
        )
        // **The one `pop_*` call site in the client, and it runs first.**
        // `hud::feedback` (inside `Stream`) and `audio::feed` (after it) both
        // want this frame's hits, toasts and refusals, and the core hands each
        // fact over exactly ONCE — so when both popped, the HUD drained every
        // ring and the game fell silent, with no conflict and no failing test
        // to say so. `feed::drain` fills a resource both read immutably;
        // `render/feed.rs` has the account. It is gated on `world_running`
        // rather than `world_placed` because a ring nobody drains overflows,
        // which is the reason `hud::feedback` gives for its own placement.
        .add_systems(Update, feed::drain.before(Stream).run_if(world_running))
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
                audio::fell,
                // The pig's voice, off the herd `mobs::stream` just moved.
                audio::pigs,
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
