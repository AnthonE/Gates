//! What a graphics tier actually costs, in Bevy components.
//!
//! **`config::Quality` is the choice and this is the table.** The split is
//! `config.rs`'s own rule — that module must build without Bevy, because its
//! parse and serialize are pure arithmetic and belong in the tier of the test
//! suite that runs without a renderer. The names are the file format; every
//! type below is a Bevy one.
//!
//! ## Why one knob instead of six
//!
//! The settings screen carried three `Row::Fact` lines saying this was not
//! adjustable — *"SMAA, always on"*, *"SSAO medium, always on"*, and a render
//! distance *"fixed by the streaming budget"*. Two of those are now false and
//! the third still is not, which is the shape of this slice: one ladder a
//! player can walk, not a panel of switches nobody can reason about. Six
//! independent toggles is six ways to build a frame nobody has ever looked
//! at, and this repo's visual gate is a person looking.
//!
//! ## What is deliberately NOT here
//!
//! **A render scale.** It is the biggest single lever on a weak GPU and it is
//! not a knob: Bevy renders to the window's surface, so a scaled path means
//! an off-screen `Image` target and a blit, which is a slice with its own
//! design rather than a row in the table below. `NOW.md` carries it.
//!
//! **The clutter and prop rings.** `CLUTTER_RING` and `NEAR_RADIUS` are read
//! by the streamers to decide which tiles exist, so moving them at runtime is
//! a streaming change, not a draw change — and `ART.md` rule 4 (no bare
//! ground inside 15 m) is a floor a tier may not cross. Also `NOW.md`.
//!
//! ## The one thing this must never do
//!
//! **[`Quality::High`] is the frame the client shipped before tiers existed,
//! exactly.** Every value in the `High` column below was a literal in
//! `rig.rs` or a constant in `tree.rs` the day before, and
//! `crates/client/tests/quality.rs` holds the column against those literals
//! written out again. A tier system that quietly re-tuned the default frame
//! would be a visual change nobody chose, arriving as a side effect of a
//! performance feature — which `CLAUDE.md`'s trap list already names once,
//! about `COVER_FULL`.

use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

use crate::config::Quality;

use super::props::Fellable;
use super::rig::{EyeCam, Sun};
use super::tree::TreeLod;

/// What one tier asks the renderer for. Plain data, so the table is readable
/// in one screen and testable without a GPU.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tier {
    /// `None` removes the component. The two prepasses SSAO requires stay
    /// either way, and that is correct rather than leftover: `ForwardDecal`
    /// needs `DepthPrepass` + `NormalPrepass` too (`decal.rs`), so the low
    /// tier drops the occlusion pass and keeps the depth the decals read.
    pub ssao: Option<ScreenSpaceAmbientOcclusionQualityLevel>,
    pub smaa: bool,
    pub bloom: bool,
    /// Shadow cascades and how far the last one reaches, metres. Each cascade
    /// is a full re-raster of every caster in it, so this is the row with the
    /// most GPU behind it.
    pub cascades: usize,
    pub shadow_m: f32,
    /// One cascade's shadow map, in texels a side. Bevy's default is 2048 and
    /// nothing in this client had ever set it.
    pub shadow_map_px: usize,
    /// Where a tree stops being its own geometry (`tree::TREE_LOD_SWAP_M`).
    pub tree_lod_swap_m: f32,
}

/// The ladder.
///
/// **Read down a column, not across a row.** Every value in `High` is what
/// shipped; `Medium` and `Low` are the same frame with work removed, and the
/// order of removal is what the arithmetic says is expensive rather than what
/// is easiest to switch off — a shadow cascade is a whole extra rasterization
/// of the forest, and the tree LOD distance decides how much forest that is.
pub fn tier(q: Quality) -> Tier {
    match q {
        Quality::High => Tier {
            ssao: Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
            smaa: true,
            bloom: true,
            cascades: 4,
            shadow_m: 200.0,
            shadow_map_px: 2048,
            tree_lod_swap_m: super::tree::TREE_LOD_SWAP_M,
        },
        Quality::Medium => Tier {
            // Low rather than off: `ART.md` §4 pays for the ambient fill with
            // occlusion, and a frame with the fill and no AO is the washed
            // one that measurement rejected (`RENDER.md` §0).
            ssao: Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
            smaa: true,
            bloom: true,
            cascades: 3,
            shadow_m: 140.0,
            shadow_map_px: 2048,
            tree_lod_swap_m: 55.0,
        },
        Quality::Low => Tier {
            ssao: None,
            // SMAA is a post-process resolve and the cheapest thing here, so
            // it goes last — but it does go: on the tier that exists for a
            // machine that cannot hold the frame, a pass is a pass.
            smaa: false,
            bloom: false,
            cascades: 2,
            shadow_m: 90.0,
            shadow_map_px: 1024,
            tree_lod_swap_m: 35.0,
        },
    }
}

impl Tier {
    /// The sun's cascade config for this tier. The two shape parameters are
    /// NOT tiered — they are how the cascades divide the distance, and
    /// re-splitting them per tier would move where the shadow resolution
    /// steps, which is a look rather than a budget.
    pub fn cascades(&self) -> CascadeShadowConfig {
        CascadeShadowConfigBuilder {
            num_cascades: self.cascades,
            minimum_distance: super::rig::CASCADE_MIN_M,
            maximum_distance: self.shadow_m,
            first_cascade_far_bound: super::rig::CASCADE_FIRST_M,
            overlap_proportion: super::rig::CASCADE_OVERLAP,
        }
        .build()
    }
}

/// Push the chosen tier into the components that carry it.
///
/// **It RECONCILES rather than reacting, and the difference is a bug this
/// slice shipped for about ten minutes.** The obvious shape is "run when
/// `Settings` changed" — and a settings file is loaded at plugin build while
/// `rig::setup` spawns the camera on `OnEnter(Screen::Loading)`, several
/// frames later. Nothing "changes" in between, so a player who had chosen LOW
/// would boot into HIGH and stay there until they touched the knob again.
/// Watching `Added<EyeCam>` as well as the settings is what closes it, and it
/// is also why `rig::setup` may go on spawning a static default-tier bundle:
/// a bundle cannot express the rung that omits a component, and it does not
/// have to.
///
/// The `Local` is the other half: `Settings` changes when a volume slider
/// moves, and re-inserting `Bloom` and a `CascadeShadowConfig` on every drag
/// would re-extract the camera and rebuild the shadow config for nothing. So
/// the trigger is the tier the player is actually on, compared against the
/// tier this system last wrote.
pub fn apply(
    mut commands: Commands,
    settings: Res<super::Settings>,
    // `Ref` rather than a second `Added<EyeCam>` query: the marker's own
    // change ticks answer "did this camera just appear" without a parameter,
    // and this system is already at clippy's argument ceiling.
    cam: Query<(Entity, Ref<EyeCam>)>,
    sun: Query<Entity, With<Sun>>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut lod: ResMut<TreeLod>,
    mut last: Local<Option<Quality>>,
) {
    let fresh = cam.iter().any(|(_, marker)| marker.is_added());
    let moved = *last != Some(settings.quality);
    if !moved && !fresh {
        return;
    }
    *last = Some(settings.quality);
    let t = tier(settings.quality);

    if let Ok((cam, _)) = cam.single() {
        let mut e = commands.entity(cam);
        match t.ssao {
            Some(quality_level) => e.insert(ScreenSpaceAmbientOcclusion {
                quality_level,
                ..default()
            }),
            None => e.remove::<ScreenSpaceAmbientOcclusion>(),
        };
        if t.smaa {
            e.insert(bevy::anti_alias::smaa::Smaa::default());
        } else {
            e.remove::<bevy::anti_alias::smaa::Smaa>();
        }
        if t.bloom {
            e.insert(Bloom::NATURAL);
        } else {
            e.remove::<Bloom>();
        }
    }
    if let Ok(sun) = sun.single() {
        commands.entity(sun).insert(t.cascades());
    }
    if shadow_map.size != t.shadow_map_px {
        shadow_map.size = t.shadow_map_px;
    }
    let want = TreeLod::at(t.tree_lod_swap_m);
    if *lod != want {
        *lod = want;
    }
}

/// Re-band every tree already in the ring when the swap distance moves.
///
/// **A separate system on a changed `TreeLod` rather than part of [`apply`]**,
/// because the two have different triggers: `apply` reacts to a settings
/// change, and this has to run for trees that stream in later as well as the
/// ones standing now — `props::stream` reads the same resource at spawn, so
/// this is only the catch-up for what is already spawned.
///
/// `FellPart` is the band: the hull is `Far` and everything else that carries
/// a range is near. That is the same table `spawn_slot` writes from, so the
/// two cannot disagree about which entity is which.
pub fn reband_trees(
    lod: Res<TreeLod>,
    mut trees: Query<(&Fellable, &mut bevy::camera::visibility::VisibilityRange)>,
) {
    if !lod.is_changed() || lod.is_added() {
        return;
    }
    for (f, mut range) in trees.iter_mut() {
        let want = lod.for_part(f.part);
        // `VisibilityRange` is not `Copy` and a write marks it changed, which
        // re-extracts the entity — so a tier that did not move this band
        // leaves it alone.
        if let Some(want) = want {
            if *range != *want {
                *range = want.clone();
            }
        }
    }
}
