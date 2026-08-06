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

pub mod bodies;
pub mod capture;
pub mod clutter;
pub mod hud;
pub mod input;
pub mod menu;
pub mod props;
pub mod rig;
pub mod sky;
pub mod terrain_mesh;
pub mod textures;
pub mod tree;
pub mod ui;

pub use menu::{Menu, Rt, Screen};

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

/// How the app starts.
///
/// **`WorldId` is no longer built here**, and that is the structural change
/// the menu forced: it needs the seed, the seed comes from the welcome, and
/// the welcome does not exist until something has connected. It is inserted
/// on entering `Screen::InWorld` instead — by `menu::poll_connect` when the
/// player picked a shard, or by `gates.rs` when a `--server`/`--capture` run
/// already connected before the window opened.
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
    /// `gates.rs` already connected, so open in the world. The probe harness
    /// and a launcher join both arrive this way: the first must not wait on
    /// a click, the second has already chosen.
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
            .init_resource::<bodies::Bodies>()
            .init_resource::<menu::Picked>()
            .insert_non_send_resource(menu::Connecting::default());

        // `Menu` is inserted either way, because a system that reads it must
        // not care which door the app came through — and a disconnect
        // returning to the menu is the obvious next slice (`NOW.md` §0v).
        app.insert_state(if self.start.connected {
            Screen::InWorld
        } else {
            Screen::Menu
        })
        .insert_resource(Menu::new(
            &self.start.direct,
            self.start.servers_url.clone(),
        ));

        // Textures load at Startup rather than on entering the world: they
        // are wanted whichever screen comes first, and warming them while a
        // player reads the menu is free time the old shape did not have.
        app.add_systems(Startup, textures::load);

        // ---- the menu ------------------------------------------------
        app.add_systems(OnEnter(Screen::Menu), (menu::begin_fetch, menu::setup))
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

        // ---- the world -----------------------------------------------
        // Every one of these reads `WorldId` or `Net`, neither of which
        // exists before the welcome. `OnEnter` is what makes that safe:
        // as `Startup` systems they would have run against a resource that
        // was not there yet and panicked on the first frame of the menu.
        app.add_systems(
            OnEnter(Screen::InWorld),
            (rig::setup, terrain_mesh::setup_water),
        )
        // The HUD's viewmodel is parented to the camera, so it must be
        // built after the rig has spawned one.
        .add_systems(OnEnter(Screen::InWorld), hud::setup.after(rig::setup))
        // The cloud deck hangs on the camera, so it waits for the rig too.
        .add_systems(OnEnter(Screen::InWorld), sky::setup.after(rig::setup))
        .add_systems(
            Update,
            (
                input::gather,
                input::place_eye,
                (
                    terrain_mesh::stream,
                    props::stream,
                    props::harvest,
                    clutter::stream,
                    bodies::stream,
                    rig::follow_eye,
                    hud::update,
                )
                    .in_set(Stream),
            )
                .chain()
                .run_if(in_state(Screen::InWorld)),
        );

        // ---- the menus -----------------------------------------------
        // **Not on a capture run**, and that is a rule rather than a
        // convenience: a probe harness that could open a panel is a visual
        // gate whose frames depend on which key was last pressed. The cost
        // is that nothing photographs these panels — the same missing menu
        // vantage `NOW.md` §0v already names, now owed twice.
        if self.capture.is_none() {
            ui::register(app);
        }

        if let Some(dir) = &self.capture {
            let _ = std::fs::create_dir_all(dir);
            app.insert_resource(capture::Capture::new(dir.clone()));
            // Ahead of `gather`, because it owns the view on a capture run
            // and `gather` must not fight it for the same frame. Gated on
            // the world state like everything else it races with — a capture
            // run enters `InWorld` on frame one, so this costs it nothing.
            app.add_systems(
                Update,
                capture::drive
                    .before(input::gather)
                    .run_if(in_state(Screen::InWorld)),
            );
        }
    }
}
