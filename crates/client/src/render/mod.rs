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
pub mod props;
pub mod rig;
pub mod sky;
pub mod terrain_mesh;
pub mod textures;

/// The connected session plus the runtime its reader tasks live on.
///
/// A NON-SEND resource on purpose: it owns tokio channel receivers, which are
/// `Send` but not `Sync`, so it cannot be a plain `Resource` — and that is
/// the correct shape anyway. One owner, touched from the main schedule only.
/// The runtime must outlive the session; dropping it would strand the event
/// and datagram readers.
pub struct Net {
    pub _rt: tokio::runtime::Runtime,
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

pub struct GatesRenderPlugin {
    pub seed: u64,
    /// Where captures go, if this is a capture run.
    pub capture: Option<std::path::PathBuf>,
}

impl Plugin for GatesRenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WorldId::new(self.seed))
            .init_resource::<Eye>()
            .init_resource::<input::Look>()
            .init_resource::<terrain_mesh::Ring>()
            .init_resource::<props::PropRing>()
            .init_resource::<clutter::ClutterRing>()
            .init_resource::<bodies::Bodies>()
            .add_systems(
                Startup,
                (rig::setup, terrain_mesh::setup_water, textures::load),
            )
            // The HUD's viewmodel is parented to the camera, so it must be
            // built after the rig has spawned one.
            .add_systems(Startup, hud::setup.after(rig::setup))
            // The cloud deck hangs on the camera, so it waits for the rig too.
            .add_systems(Startup, sky::setup.after(rig::setup))
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
                    .chain(),
            );

        if let Some(dir) = &self.capture {
            let _ = std::fs::create_dir_all(dir);
            app.insert_resource(capture::Capture::new(dir.clone()));
            // Ahead of `gather`, because it owns the view on a capture run
            // and `gather` must not fight it for the same frame.
            app.add_systems(Update, capture::drive.before(input::gather));
        }
    }
}
