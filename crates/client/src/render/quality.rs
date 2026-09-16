//! What the graphics settings actually cost, in Bevy components.
//!
//! Three types, and keeping them apart is the whole design:
//!
//!   * [`Gfx`] — what the player chose. Plain numbers, no Bevy, one field per
//!     row on the settings screen.
//!   * [`effective`] — what this TARGET can actually do with that choice. A
//!     browser cannot run four shadow cascades whatever the screen says, and
//!     that is a capability, not a preference.
//!   * [`Tier`] — the Bevy components an effective [`Gfx`] resolves to.
//!
//! `config::Quality` is still here and still means what it meant: it is a
//! **preset**, a named column of [`preset`]'s table that writes every row of
//! [`Gfx`] at once.
//!
//! ## Why six toggles now, when this file argued for one knob
//!
//! It argued for one knob and the operator scored it from the player's side
//! (2026-09-16: *"we need more graphics options in the setting and make them
//! actually work please"*). `CLAUDE.md`: the operator's word beats any doc
//! including this one, and it is recorded in `DECISIONS.md` the same day.
//!
//! The old argument was that six independent toggles is six ways to build a
//! frame nobody has ever looked at. That risk is real and it is answered
//! rather than ignored: the ladder did not go away, it became the **preset**,
//! so LOW / MEDIUM / HIGH still name three frames that were built on purpose
//! and a fresh install still lands on one of them. What is new is that a row
//! can be moved off its preset, and the QUALITY row then reads CUSTOM —
//! because it is true, and a screen that kept saying HIGH while the player
//! had pulled the shadows in would be the most confidently wrong thing in the
//! game.
//!
//! ## What is deliberately NOT a row
//!
//! **A render scale.** It is the biggest single lever on a weak GPU and it is
//! not a knob: Bevy renders to the window's surface, so a scaled path means
//! an off-screen `Image` target and a blit, which is a slice with its own
//! design rather than a row in the table below. `NOW.md` carries it.
//!
//! **The clutter and prop rings.** `CLUTTER_RING` and `NEAR_RADIUS` are read
//! by the streamers to decide which tiles exist, so moving them at runtime is
//! a streaming change, not a draw change — and `ART.md` rule 4 (no bare
//! ground inside 15 m) is a floor a row may not cross. Also `NOW.md`.
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

use crate::config::{Ao, Quality};

use super::props::Fellable;
use super::rig::{EyeCam, Sun};
use super::tree::TreeLod;

/// What the player chose, one field per row on the GRAPHICS tab.
///
/// **Plain data on purpose.** `render/settings.rs` holds one of these, the
/// settings file writes one key per field, and nothing here is a Bevy type —
/// so the whole of "what did the player ask for" is comparable with `==`,
/// which is what makes [`Settings::preset_name`] a one-liner and what lets
/// `apply` skip its work on a frame where a volume slider moved.
///
/// [`Settings::preset_name`]: super::settings::Settings::preset_name
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gfx {
    pub ao: Ao,
    pub smaa: bool,
    pub bloom: bool,
    /// Whether the sun casts at all.
    ///
    /// **Separate from [`Self::shadow_m`] rather than folded into it as a
    /// zero**, so that turning shadows off and back on does not cost the
    /// player the distance they had picked — and so that
    /// `CascadeShadowConfigBuilder`, which asserts on its inputs, is never
    /// handed a degenerate one.
    pub shadows: bool,
    /// How far the last cascade reaches, metres.
    pub shadow_m: f32,
    /// How many cascades the distance is cut into. See [`max_cascades`] for
    /// the ceiling, which is the engine's and not ours.
    pub cascades: usize,
    /// One cascade's shadow map, in texels a side.
    pub shadow_map_px: usize,
    /// Where a tree stops being its own geometry (`tree::TREE_LOD_SWAP_M`).
    pub tree_lod_swap_m: f32,
}

/// The shadow distances the screen offers, metres, ascending.
///
/// **The floor is above `rig::CASCADE_FIRST_M`** and that is structural, not
/// taste: `CascadeShadowConfigBuilder::build` asserts `minimum_distance <
/// first_cascade_far_bound` for any multi-cascade config, and a maximum
/// distance below the first cascade's own bound would put the whole geometric
/// split in the wrong order. `tests/quality.rs` holds the rungs against that
/// constant rather than against a literal.
pub const SHADOW_M_LADDER: [f32; 6] = [40.0, 60.0, 90.0, 140.0, 200.0, 300.0];

/// The floor above, asserted where it cannot be skipped — at compile time.
///
/// A test would be the usual home, and clippy is right that it does not
/// belong in one: the relation is between two constants, so a runtime
/// assertion about it can only ever be constant-folded. Here it fails the
/// BUILD, which is the correct severity for "the settings screen can hand the
/// engine an input it panics on".
const _: () = assert!(
    SHADOW_M_LADDER[0] > super::rig::CASCADE_FIRST_M
        && super::rig::CASCADE_FIRST_M > super::rig::CASCADE_MIN_M,
    "the shortest shadow distance the screen offers must stay above      rig::CASCADE_FIRST_M: CascadeShadowConfigBuilder::build asserts      minimum_distance < first_cascade_far_bound, and a maximum distance under      the first cascade's own bound puts the geometric split in the wrong order"
);

/// The shadow map sizes the screen offers, texels a side.
///
/// Powers of two, and Bevy asks for that in as many words: `calculate_cascade`
/// divides an integer cascade diameter by this, and notes that a power of two
/// keeps the texel size exactly representable, which is what stops a shadow
/// edge crawling as the camera moves.
///
/// The top rung is 4096 and it is not free — see [`shadow_vram_mb`], which the
/// screen prints beside the row precisely because four cascades at 4096 is a
/// quarter of a gigabyte and nothing else in this client would tell you.
pub const SHADOW_PX_LADDER: [usize; 4] = [512, 1024, 2048, 4096];

/// The tree hull-swap distances the screen offers, metres.
///
/// Named constants where there are any: 35 is `Quality::Low`'s, 55 is
/// [`MEDIUM_TREE_LOD_SWAP_M`] and 80 is `tree::TREE_LOD_SWAP_M`, the
/// registered knob. `tests/quality.rs` asserts every preset's value is ON
/// this ladder — a preset a stepper cannot return to would be a one-way trip.
pub const TREE_LOD_LADDER: [f32; 5] = [35.0, MEDIUM_TREE_LOD_SWAP_M, 80.0, 120.0, 160.0];

/// `Medium`'s tree swap distance, metres — named because a browser borrows it
/// (see [`effective`]), so the two uses cannot drift apart.
pub const MEDIUM_TREE_LOD_SWAP_M: f32 = 55.0;

/// **How many cascades this target can actually draw**, which is Bevy's
/// number and not ours.
///
/// `bevy_pbr::render::light::MAX_CASCADES_PER_LIGHT` is 4 — and **1** under
/// `all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu"))`,
/// which is exactly what this client builds for the browser: `bevy`'s
/// `default_platform` feature set turns on `webgl2`, we take it, and we do not
/// take `webgpu`.
///
/// **This was a shipped bug and it is the reason this slice exists.** The
/// browser tier asked for two cascades from a `first_cascade_far_bound` of
/// `rig::CASCADE_FIRST_M` (12 m). Bevy does not fail that request and does not
/// scale it: `prepare_lights` takes `.min(MAX_CASCADES_PER_LIGHT)` of the
/// bounds list, so the browser got cascade **0 only** — every shadow in the
/// world stopped dead 12 m from the player, with the ground beyond it lit as
/// if nothing stood on it. The engine `warn!`s once, into a console nobody
/// reads, and every gate in this repo was green over it.
///
/// So the fix is not a bigger number, it is asking for **one** cascade on that
/// target, which makes `calculate_cascade_bounds` return `[maximum_distance]`
/// and ignore the first-bound split entirely. One cascade over 90 m is a
/// coarser shadow than four over 200 m; no shadow at all past 12 m is not a
/// shadow system.
pub const fn max_cascades() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        4
    }
}

/// The largest shadow map this target is guaranteed to be able to allocate,
/// texels a side.
///
/// 2048 in a browser because that is WebGL 2's own floor for
/// `MAX_TEXTURE_SIZE` — most machines do far better, but a setting that
/// works on the operator's laptop and fails to allocate on a player's is the
/// worst shape this file can ship, and the rung above buys 13.8 cm of texel
/// against 27.5 (see [`far_texel_m`]) rather than anything anyone would call
/// the difference between a game and not.
pub const fn max_shadow_map_px() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        2048
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        4096
    }
}

/// The preset table: what one named column of the ladder means.
///
/// **Read down a column, not across a row.** Every value in `High` is what
/// shipped; `Medium` and `Low` are the same frame with work removed, and the
/// order of removal is what the arithmetic says is expensive rather than what
/// is easiest to switch off — a shadow cascade is a whole extra rasterization
/// of the forest, and the tree LOD distance decides how much forest that is.
pub fn preset(q: Quality) -> Gfx {
    match q {
        Quality::High => Gfx {
            ao: Ao::Medium,
            smaa: true,
            bloom: true,
            shadows: true,
            cascades: 4,
            shadow_m: 200.0,
            shadow_map_px: 2048,
            tree_lod_swap_m: super::tree::TREE_LOD_SWAP_M,
        },
        Quality::Medium => Gfx {
            // Low rather than off: `ART.md` §4 pays for the ambient fill with
            // occlusion, and a frame with the fill and no AO is the washed
            // one that measurement rejected (`RENDER.md` §0).
            ao: Ao::Low,
            smaa: true,
            bloom: true,
            shadows: true,
            cascades: 3,
            shadow_m: 140.0,
            shadow_map_px: 2048,
            tree_lod_swap_m: MEDIUM_TREE_LOD_SWAP_M,
        },
        Quality::Low => Gfx {
            ao: Ao::Off,
            // SMAA is a post-process resolve and the cheapest thing here, so
            // it goes last — but it does go: on the tier that exists for a
            // machine that cannot hold the frame, a pass is a pass.
            smaa: false,
            bloom: false,
            shadows: true,
            cascades: 2,
            shadow_m: 90.0,
            // **The one value in this table that differs by target, and it is
            // not a second opinion about the tier — it is the same budget
            // spent where the target left it.** `Low` asks for two cascades
            // at 1024; a browser is given ONE whatever it asks
            // ([`max_cascades`]), so half that allocation is handed back, and
            // a single cascade covering the whole 90 m is the one place where
            // the map size is the ONLY resolution lever left. 1024 over 90 m
            // is 27.5 cm of world per shadow texel; 2048 is 13.8, for
            // 16 MiB against the 8 MiB two cascades at 1024 would have cost.
            //
            // In `preset` rather than in [`effective`] deliberately: it is a
            // budget and not a capability, so a browser player who wants the
            // 12 MiB back can still step this row down to 1024 — which a
            // clamp would have taken away from them.
            #[cfg(target_arch = "wasm32")]
            shadow_map_px: 2048,
            #[cfg(not(target_arch = "wasm32"))]
            shadow_map_px: 1024,
            tree_lod_swap_m: 35.0,
        },
    }
}

/// What a fresh install gets **on this target**.
///
/// Native takes the top rung, which is the frame that shipped. A browser
/// takes the bottom one, which is what it has been drawing since the page
/// existed. **Which RUNG a target opens on is decided here and nowhere
/// else**; `preset`'s `Low` row carries one `cfg` of its own, and that one is
/// about how that rung spends its shadow budget when the engine forces it to
/// one cascade, not about which rung a browser gets.
///
/// **Through [`effective`], and that is not belt-and-braces.** A browser's
/// `Low` asks for two cascades and gets one; if the settings screen HELD the
/// two, then a fresh browser install would open on a QUALITY row reading
/// CUSTOM and an "on this machine" line listing what it was refused — before
/// the player had touched anything. What a preset resolves to on this target
/// IS the preset here, so `preset_name` finds it and the screen names it.
/// Every other write of a preset goes through the same call
/// (`Settings::with_preset`, `Knob::Quality`, `gfx_from_file`), which is what
/// makes that true rather than nearly true.
pub fn default_gfx() -> Gfx {
    #[cfg(target_arch = "wasm32")]
    let g = preset(Quality::Low);
    #[cfg(not(target_arch = "wasm32"))]
    let g = preset(Quality::default());
    effective(g)
}

/// What this target can actually do with a choice.
///
/// **Only capabilities and one measured budget belong here.** A clamp is a
/// statement that the hardware or the engine refuses the request; a
/// preference belongs in [`preset`] or in the player's hands. The settings
/// screen draws the chosen value AND this one, and says which row was pinned
/// and why (`settings::rows`) — a setting that silently does nothing is worse
/// than one that is honestly greyed.
///
/// **It is idempotent, and `tests/quality.rs` says so.** Every term is a
/// clamp, a floor or a constant, so `effective(effective(g)) == effective(g)`
/// — which is what lets a preset be stored already-resolved (see
/// [`default_gfx`]) without the screen then reading it as customized.
pub fn effective(g: Gfx) -> Gfx {
    let g = Gfx {
        cascades: g.cascades.clamp(1, max_cascades()),
        shadow_map_px: g.shadow_map_px.min(max_shadow_map_px()),
        ..g
    };
    // ⚠ **WebGL2 cannot run SSAO at all, and the failure is a PANIC rather
    // than a degradation** — so this is a clamp on the row and not a default
    // a player may override.
    //
    // Measured 2026-09-11 in a real browser, on the first frame:
    //
    //     In Device::create_bind_group_layout,
    //       label = 'mesh_view_layout_depth_normal_atmosphere'
    //       Too many bindings of type StorageBuffers in Stage FRAGMENT,
    //       limit is 0, count was 1
    //
    // The chain is indirect, which is why it is written down. `ao != Off`
    // inserts `ScreenSpaceAmbientOcclusion`, which carries
    // `#[require(DepthPrepass, NormalPrepass)]`. Those prepasses build a
    // mesh-view layout that wants one storage buffer in the fragment stage,
    // and `downlevel_webgl2_defaults()` sets
    // `max_storage_buffers_per_shader_stage` to **zero**. Bevy's own SSAO
    // plugin already declines to load on this backend (it needs storage
    // TEXTURES, also zero) and says so politely in the log — but the
    // `#[require]` still fires, so the row pays for two prepasses that feed
    // nothing and then dies building their layout.
    //
    // **The tree rung is a budget floor, not a capability, and it is the
    // browser's existing behaviour written down where it can be read.**
    // `Low`'s 35 m is sized for a machine that cannot hold the frame, which a
    // browser on an ordinary desktop is not; a hull is ~55× cheaper than the
    // tree it stands in for, so the page keeps the next rung up. A player who
    // asks for more still gets more — it is a floor, not a pin.
    //
    // ⚠ That rung was first moved on 2026-09-13 because the first real
    // browser session "read as a forest of hulls right up to the player", and
    // that reading was wrong about the CAUSE: a hull at arm's length is
    // inside any swap distance, so no distance can put it there. Those were
    // the outer ring's hulls, left standing in every chunk the player walked
    // into (`props::stream`'s hand-off, fixed the same day —
    // `tests/ring_handoff.rs`). The distance stays on the argument above, not
    // on that frame.
    #[cfg(target_arch = "wasm32")]
    let g = Gfx {
        ao: Ao::Off,
        tree_lod_swap_m: g.tree_lod_swap_m.max(MEDIUM_TREE_LOD_SWAP_M),
        ..g
    };
    g
}

/// What one effective [`Gfx`] asks the renderer for, in Bevy's own types.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tier {
    /// `None` removes the component. The two prepasses SSAO requires stay
    /// either way, and that is correct rather than leftover: `ForwardDecal`
    /// needs `DepthPrepass` + `NormalPrepass` too (`decal.rs`), so a frame
    /// with the occlusion pass off keeps the depth the decals read.
    pub ssao: Option<ScreenSpaceAmbientOcclusionQualityLevel>,
    pub smaa: bool,
    pub bloom: bool,
    pub shadows: bool,
    /// Shadow cascades and how far the last one reaches, metres. Each cascade
    /// is a full re-raster of every caster in it, so this is the row with the
    /// most GPU behind it.
    pub cascades: usize,
    pub shadow_m: f32,
    /// One cascade's shadow map, in texels a side. Bevy's default is 2048.
    pub shadow_map_px: usize,
    /// Where a tree stops being its own geometry (`tree::TREE_LOD_SWAP_M`).
    pub tree_lod_swap_m: f32,
}

/// The Bevy components one effective [`Gfx`] resolves to.
pub fn components(g: Gfx) -> Tier {
    Tier {
        ssao: match g.ao {
            Ao::Off => None,
            Ao::Low => Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
            Ao::Medium => Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
            Ao::High => Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
            Ao::Ultra => Some(ScreenSpaceAmbientOcclusionQualityLevel::Ultra),
        },
        smaa: g.smaa,
        bloom: g.bloom,
        shadows: g.shadows,
        cascades: g.cascades,
        shadow_m: g.shadow_m,
        shadow_map_px: g.shadow_map_px,
        tree_lod_swap_m: g.tree_lod_swap_m,
    }
}

/// A named preset, straight through [`effective`] and [`components`] — what
/// that column of the table actually reaches the renderer as on this target.
pub fn tier(q: Quality) -> Tier {
    components(effective(preset(q)))
}

impl Tier {
    /// The sun's cascade config for this tier. The two shape parameters are
    /// NOT per-row — they are how the cascades divide the distance, and
    /// letting a player re-split them would move where the shadow resolution
    /// steps, which is a look rather than a budget.
    ///
    /// **With one cascade the split does not happen at all**, which is the
    /// whole of the browser fix: `calculate_cascade_bounds` short-circuits to
    /// `[maximum_distance]` and `first_cascade_far_bound` is ignored. See
    /// [`max_cascades`].
    pub fn cascades(&self) -> CascadeShadowConfig {
        CascadeShadowConfigBuilder {
            num_cascades: self.cascades.max(1),
            minimum_distance: super::rig::CASCADE_MIN_M,
            maximum_distance: self.shadow_m,
            first_cascade_far_bound: super::rig::CASCADE_FIRST_M,
            overlap_proportion: super::rig::CASCADE_OVERLAP,
        }
        .build()
    }
}

/// The aspect ratio [`far_texel_m`] reports against.
///
/// **A stated assumption, not a measurement**, and the screen says so in the
/// row: the real number comes from the window, which this function has no
/// business reading — it is called to draw a settings row and to be gated
/// headlessly, and a readout that changed when you resized the window would
/// be noise rather than information. 16:9 is the shape of every frame this
/// project has judged.
pub const READOUT_ASPECT: f32 = 16.0 / 9.0;

/// **How coarse the shadows get at the far edge**, metres per shadow-map
/// texel — the one number that answers "why do shadows look like that over
/// there", and the reason this slice was asked for.
///
/// It is Bevy's own arithmetic, restated: `calculate_cascade` fits the last
/// cascade's ortho projection to the larger of the frustum slice's **body**
/// diagonal and its **far-plane** diagonal, rounds that up to an integer, and
/// divides by the shadow map size. Nothing about the sun's direction enters
/// it, which is why this can be printed on a settings screen with no camera
/// and no GPU in the room.
///
/// The measurement that made this worth printing (2026-09-16, `fov` 75,
/// 16:9): `Quality::High`'s four cascades to 200 m at 2048 px put the last
/// one from 62.6 m to 200 m inside a **627 m** ortho box — **30.6 cm** of
/// world per texel, against 1.86 cm in the first cascade. A browser's single
/// cascade to 90 m at 1024 px is 27.5 cm. Those are the numbers behind
/// "shadow stuff is kinda garbage with distance"; a shadow edge cannot be
/// finer than one texel, and 30 cm of stair-step at 80 m is several screen
/// pixels.
///
/// ⚠ **`cascades` here is the EFFECTIVE count**, so pass a [`Gfx`] that has
/// been through [`effective`] — asking this about four cascades in a browser
/// would print a number that target cannot produce.
pub fn far_texel_m(g: Gfx, fov_deg: f32) -> f32 {
    let n = g.cascades.max(1);
    // `calculate_cascade_bounds`: a geometric progression from the first
    // cascade's far bound to the maximum distance, or just the maximum when
    // there is one cascade.
    let (near, far) = if n == 1 {
        (super::rig::CASCADE_MIN_M, g.shadow_m)
    } else {
        let first = super::rig::CASCADE_FIRST_M;
        let base = (g.shadow_m / first).powf(1.0 / (n - 1) as f32);
        let prev = first * base.powf((n - 2) as f32);
        ((1.0 - super::rig::CASCADE_OVERLAP) * prev, g.shadow_m)
    };
    let t = (fov_deg.to_radians() * 0.5).tan();
    // Corner order is Bevy's: near bottom-right .. far bottom-left, looking
    // down -z. It compares corner 0 against corner 6 (near BR to far TL) and
    // corner 4 against corner 6 (far BR to far TL).
    let corner = |d: f32, sx: f32, sy: f32| Vec3::new(sx * d * t * READOUT_ASPECT, sy * d * t, -d);
    let near_br = corner(near, 1.0, -1.0);
    let far_br = corner(far, 1.0, -1.0);
    let far_tl = corner(far, -1.0, 1.0);
    let diameter = near_br.distance(far_tl).max(far_br.distance(far_tl)).ceil();
    diameter / g.shadow_map_px.max(1) as f32
}

/// What the shadow maps cost in video memory, mebibytes.
///
/// One `Depth32Float` texture array, `shadow_map_px` square, one layer per
/// cascade (`CORE_3D_DEPTH_FORMAT`, `bevy_pbr`'s
/// `directional_light_depth_texture`). Four bytes a texel, and the top of
/// both ladders is **256 MiB** — which is the number a player is entitled to
/// see before they pick it, and which nothing else in this client would tell
/// them.
pub fn shadow_vram_mb(g: Gfx) -> f32 {
    if !g.shadows {
        return 0.0;
    }
    let texels = (g.shadow_map_px * g.shadow_map_px * g.cascades.max(1)) as f32;
    texels * 4.0 / (1024.0 * 1024.0)
}

/// Push the chosen settings into the components that carry them.
///
/// **It RECONCILES rather than reacting, and the difference is a bug this
/// slice shipped for about ten minutes.** The obvious shape is "run when
/// `Settings` changed" — and a settings file is loaded at plugin build while
/// `rig::setup` spawns the camera on `OnEnter(Screen::Loading)`, several
/// frames later. Nothing "changes" in between, so a player who had chosen LOW
/// would boot into HIGH and stay there until they touched the knob again.
/// Watching `Added<EyeCam>` as well as the settings is what closes it, and it
/// is also why `rig::setup` may go on spawning a static default bundle: a
/// bundle cannot express the rung that omits a component, and it does not
/// have to.
///
/// The `Local` is the other half: `Settings` changes when a volume slider
/// moves, and re-inserting `Bloom` and a `CascadeShadowConfig` on every drag
/// would re-extract the camera and rebuild the shadow config for nothing. So
/// the trigger is the effective [`Gfx`] the player is actually on, compared
/// against the one this system last wrote.
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
    mut last: Local<Option<Gfx>>,
) {
    let fresh = cam.iter().any(|(_, marker)| marker.is_added());
    let want = effective(settings.gfx);
    let moved = *last != Some(want);
    if !moved && !fresh {
        return;
    }
    *last = Some(want);
    let t = components(want);

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
    // `shadows_enabled` is NOT written here: `rig::day_night` owns that field
    // — it turns the sun off at night — and two writers of one field is the
    // shape `CLAUDE.md`'s own trap list calls a clean merge that is not a
    // correct one. It reads this same table instead.
    if shadow_map.size != t.shadow_map_px {
        shadow_map.size = t.shadow_map_px;
    }
    // The chosen distance, not the bands: `tree::cap_swap` may be holding the
    // bands under it this frame, and that is not a reason to rewrite them.
    // A distance that did move resets the bands to it, and the cap pulls them
    // in again on its next run if the stand still needs it.
    if lod.tier_m != t.tree_lod_swap_m {
        *lod = TreeLod::at(t.tree_lod_swap_m);
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
