//! Gate: the graphics ladder only ever gets cheaper, and its top rung is the
//! frame that shipped before it existed.
//!
//! **The second half is the load-bearing one.** A tier system is a
//! performance feature, and the way one goes wrong is not by failing to save
//! anything — it is by quietly re-tuning the default frame while doing so.
//! `CLAUDE.md`'s trap list already carries that shape once (`COVER_FULL`, a
//! unit conversion that would have arrived as a tuning change nobody chose),
//! and here it would be worse: every value in `Quality::High` was a literal in
//! `rig.rs` or a constant in `tree.rs` the day before, and a one-character
//! slip while moving them into a table changes what every player sees with no
//! gate in this repo able to notice. So the column is written out again here,
//! by hand, as a golden — the shape `test_protocol_golden` uses for the wire.
//!
//! The rest is arithmetic about the ladder plus one headless run of the
//! applier, because a table nothing reads is a table that is always correct.

#![cfg(feature = "render")]

use bevy::anti_alias::smaa::Smaa;
use bevy::asset::AssetPlugin;
use bevy::camera::visibility::VisibilityRange;
use bevy::light::{DirectionalLight, DirectionalLightShadowMap};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel as Q};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use client::config::{Ao, Quality};
use client::render::props::{FellPart, Fellable, Topple};
use client::render::quality::{
    apply, components, effective, far_texel_m, max_cascades, max_shadow_map_px, preset,
    reband_trees, shadow_vram_mb, tier, Gfx, SHADOW_M_LADDER, SHADOW_PX_LADDER, TREE_LOD_LADDER,
};
use client::render::rig::CASCADE_FIRST_M;
use client::render::rig::{EyeCam, Sun};
use client::render::settings::Knob;
use client::render::tree::{TreeLod, TREE_LOD_SWAP_M};
use client::render::Settings;

/// **The golden.** Every one of these was a literal in the client on
/// 2026-08-20, before `render/quality.rs` existed:
///
///   * `rig.rs` spawned `ScreenSpaceAmbientOcclusion { quality_level: Medium }`
///     and `Smaa::default()` and `Bloom::NATURAL` on the camera;
///   * its `CascadeShadowConfigBuilder` read `num_cascades: 4` and
///     `maximum_distance: 200.0`;
///   * nothing anywhere set `DirectionalLightShadowMap`, so it was Bevy's own
///     default of 2048 — which is why that row is the one to check against
///     the engine rather than against us;
///   * `tree.rs` swapped to the far hull at `TREE_LOD_SWAP_M`.
///
/// A change to any of them is a change to the shipped frame. That is allowed;
/// doing it by accident while editing a table is not.
#[test]
fn the_top_rung_is_the_frame_that_shipped() {
    let t = tier(Quality::High);
    assert_eq!(t.ssao, Some(Q::Medium), "SSAO was medium");
    assert!(t.smaa, "SMAA was on");
    assert!(t.bloom, "bloom was on");
    assert_eq!(t.cascades, 4, "the sun had four cascades");
    assert_eq!(t.shadow_m, 200.0, "they reached 200 m");
    assert_eq!(
        t.shadow_map_px,
        DirectionalLightShadowMap::default().size,
        "nothing set a shadow map size before this, so the top rung must be \
         whatever Bevy's default is — reading it from the engine rather than \
         writing 2048 here is what makes that still true after an upgrade"
    );
    assert_eq!(
        t.tree_lod_swap_m, TREE_LOD_SWAP_M,
        "the top rung must be the registered knob itself, not a copy of its \
         value — `ci/knob_registry.mjs` pins that constant to DECISIONS.md, \
         and a tier holding a second number would leave the registry pinning \
         the one nobody reads"
    );

    assert_eq!(
        Settings::default().quality,
        Quality::High,
        "a fresh install must draw the frame that shipped"
    );
}

/// Down the ladder, everything gets cheaper or stays — never dearer.
///
/// **A rung that raised one cost to lower another would be a look, not a
/// budget**, and it is the easy mistake to make: "low can afford sharper
/// shadows because it has fewer cascades" is a sentence somebody will write.
/// The player asked for less work, so every axis has to answer that.
#[test]
fn every_rung_down_is_cheaper_on_every_axis() {
    // Bevy's own ordering, plus `None` below all of it. `Custom` carries a
    // sample count this table does not use and cannot be ranked against the
    // named levels, so it fails loudly rather than sorting as anything —
    // ranking it as zero would let a tier smuggle 32 samples in under LOW.
    let rank = |s: Option<Q>| match s {
        None => 0,
        Some(Q::Low) => 1,
        Some(Q::Medium) => 2,
        Some(Q::High) => 3,
        Some(Q::Ultra) => 4,
        Some(other) => panic!(
            "a tier asks for {other:?}, which this ladder cannot rank — give \
             it a rank here rather than letting it sort as the cheapest thing"
        ),
    };
    for pair in Quality::LADDER.windows(2) {
        let (lo, hi) = (tier(pair[0]), tier(pair[1]));
        let (ln, hn) = (pair[0], pair[1]);
        assert!(
            rank(lo.ssao) <= rank(hi.ssao),
            "{ln:?} asks for more ambient occlusion than {hn:?}"
        );
        assert!(
            !(lo.smaa && !hi.smaa),
            "{ln:?} anti-aliases where {hn:?} does not"
        );
        assert!(
            !(lo.bloom && !hi.bloom),
            "{ln:?} blooms where {hn:?} does not"
        );
        assert!(
            lo.cascades <= hi.cascades,
            "{ln:?} draws {} shadow cascades against {hn:?}'s {} — a cascade \
             is a whole extra rasterization of everything that casts",
            lo.cascades,
            hi.cascades
        );
        assert!(
            lo.shadow_m <= hi.shadow_m,
            "{ln:?} shadows further than {hn:?}"
        );
        assert!(
            lo.shadow_map_px <= hi.shadow_map_px,
            "{ln:?} wants a bigger shadow map than {hn:?}"
        );
        assert!(
            lo.tree_lod_swap_m <= hi.tree_lod_swap_m,
            "{ln:?} keeps full-detail trees out to {} m against {hn:?}'s {} — \
             the swap distance decides how much forest is real geometry",
            lo.tree_lod_swap_m,
            hi.tree_lod_swap_m
        );
        assert!(
            lo != hi,
            "{ln:?} and {hn:?} resolve to the same tier — a rung that costs \
             what the one above it costs is a row on a screen that does \
             nothing"
        );
        assert!(
            lo.cascades >= 1 && lo.shadow_map_px > 0,
            "{ln:?} turned shadows off entirely, which is a look and not a \
             budget — ART.md §8 asks for visible contact shadowing"
        );
    }
}

/// The knob walks the ladder and stops at both ends.
///
/// ⚠ **Asserted at every step, not at the end, and the first draft was not.**
/// It stepped five times and checked where it landed — and five steps on a
/// three-rung ladder wraps back to the same answer, so a `rem_euclid` in
/// place of the clamp passed it. A gate whose count happens to be a multiple
/// of the cycle is a gate that cannot see the cycle.
#[test]
fn the_knob_steps_and_does_not_wrap() {
    let rung = |q: Quality| Quality::LADDER.iter().position(|x| *x == q).unwrap() as i32;
    for dir in [-1i32, 1] {
        let mut s = Settings::default();
        if dir > 0 {
            // Start at the bottom so the upward walk has somewhere to go.
            for _ in 0..Quality::LADDER.len() {
                s.adjust(client::render::settings::Knob::Quality, -1);
            }
        }
        let mut prev = rung(s.quality);
        for step in 0..Quality::LADDER.len() * 2 {
            s.adjust(client::render::settings::Knob::Quality, dir);
            let now = rung(s.quality);
            let moved = now - prev;
            assert!(
                moved == dir || moved == 0,
                "step {step} in direction {dir} moved the tier by {moved} — a \
                 click must move one rung or none. A wrap puts a player who \
                 clicked once too many on the far end of the ladder from \
                 where they were looking, which for this knob means the frame \
                 changing completely"
            );
            prev = now;
        }
        let end = if dir < 0 { Quality::Low } else { Quality::High };
        assert_eq!(
            s.quality, end,
            "the walk must rest against the end it ran at"
        );
    }
}

/// A tier the player chose LAST session reaches a camera that spawns later.
///
/// **This is a bug the slice shipped and this gate is why it did not stay.**
/// `crate::config` loads the settings file at plugin build; `rig::setup`
/// spawns the camera on `OnEnter(Screen::Loading)`, several frames later.
/// An applier keyed on "the settings changed" sees nothing happen in between
/// — so a player who had chosen LOW would boot into HIGH and stay there until
/// they opened the menu and touched the knob. Every other assertion in this
/// file passed while that was true, because they all move the knob first.
#[test]
fn a_tier_chosen_last_session_reaches_a_camera_that_spawns_later() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<TreeLod>();
    app.init_resource::<DirectionalLightShadowMap>();
    // The file said LOW, and it said so before anything was drawn.
    //
    // `with_preset` rather than `Settings { quality, ..default() }`: the
    // preset NAME and the graphics ROWS are two fields now, and that struct
    // update would have built a settings object that says LOW and draws HIGH
    // — which this gate would then have failed for the wrong reason.
    app.insert_resource(Settings::with_preset(Quality::Low));
    app.add_systems(Update, apply);

    // Frames with no camera at all — the menu, the shard list, the loading
    // screen. The tier must survive them.
    app.update();
    app.update();

    // `rig::setup` runs, spawning the DEFAULT tier's bundle because a bundle
    // is static.
    let cam = app
        .world_mut()
        .spawn((
            EyeCam,
            ScreenSpaceAmbientOcclusion::default(),
            Smaa::default(),
            Bloom::NATURAL,
        ))
        .id();
    app.update();

    let e = app.world().entity(cam);
    assert!(
        !e.contains::<ScreenSpaceAmbientOcclusion>()
            && !e.contains::<Smaa>()
            && !e.contains::<Bloom>(),
        "the camera booted wearing the default tier's post chain under a \
         persisted LOW — a saved setting that only takes effect after the \
         player opens the menu again is a setting that did not save"
    );
    assert!(
        *app.world().resource::<TreeLod>() == TreeLod::at(tier(Quality::Low).tree_lod_swap_m),
        "…and the tree LOD resource never left the default, so every chunk \
         streamed before the first knob-touch swaps at the wrong distance"
    );
}

/// …and the table actually reaches the ECS.
///
/// **The lesson `tests/tree_lod.rs` was written for, one file over.** Every
/// assertion above is about a `Tier` struct, and a `Tier` nothing reads is a
/// struct that is always correct. This runs the real applier against a real
/// `World` holding a camera, a sun and a standing tree, and reads back what
/// changed.
#[test]
fn flipping_the_knob_rebuilds_the_camera_the_sun_and_the_forest() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_resource::<Settings>();
    app.init_resource::<TreeLod>();
    app.init_resource::<DirectionalLightShadowMap>();

    let cam = app
        .world_mut()
        .spawn((
            EyeCam,
            ScreenSpaceAmbientOcclusion::default(),
            Smaa::default(),
            Bloom::NATURAL,
        ))
        .id();
    let sun = app
        .world_mut()
        .spawn((Sun, DirectionalLight::default()))
        .id();
    // One tree's worth of parts, banded as `spawn_slot` bands them.
    let lod = TreeLod::default();
    let near = app
        .world_mut()
        .spawn((
            Fellable {
                key: 1,
                variant: 0,
                base_y: 0.0,
                yaw: 0.0,
                part: FellPart::Trunk,
                felled: false,
            },
            Topple { t: -1.0 },
            lod.near.clone(),
        ))
        .id();
    let far = app
        .world_mut()
        .spawn((
            Fellable {
                key: 1,
                variant: 0,
                base_y: 0.0,
                yaw: 0.0,
                part: FellPart::Far,
                felled: false,
            },
            Topple { t: -1.0 },
            lod.far.clone(),
        ))
        .id();

    // **Through the schedule, not `run_system_once`.** Both systems guard on
    // `is_changed() && !is_added()`, and a one-shot run initializes the
    // system with a zero `last_run` — so everything in the world looks both
    // changed and newly added to it, and the guard that makes a fresh boot a
    // no-op would make every one-shot run a no-op too. Registering them the
    // way `render/mod.rs` does is also the only way to exercise the ordering
    // it declares.
    app.add_systems(Update, (apply, reband_trees.after(apply)));

    // Frame one: the default tier is already what everything above wears, and
    // the resource is newly added. Nothing may move — that is the guarantee
    // that a fresh boot draws `rig::setup`'s frame and not the applier's idea
    // of it.
    app.update();
    // What this can see is that nothing was TAKEN AWAY; it cannot see a
    // re-insert of the same value, and does not claim to. The guard exists so
    // the applier does not write over what `rig::setup` spawned before a
    // player has touched anything.
    assert!(
        app.world()
            .entity(cam)
            .contains::<ScreenSpaceAmbientOcclusion>(),
        "the applier removed ambient occlusion on the frame the settings \
         were inserted — a fresh boot must not be a settings change"
    );

    // Frame two: the bottom rung. The whole resource, not just its `quality`
    // label — the applier reads `gfx`, and writing the label alone is exactly
    // the disagreement `Settings::with_preset` exists to make impossible.
    *app.world_mut().resource_mut::<Settings>() = Settings::with_preset(Quality::Low);
    app.update();

    let low = tier(Quality::Low);
    let e = app.world().entity(cam);
    assert!(
        !e.contains::<ScreenSpaceAmbientOcclusion>(),
        "LOW must drop SSAO — it is the pass this tier exists to stop paying"
    );
    assert!(!e.contains::<Smaa>(), "LOW must drop SMAA");
    assert!(!e.contains::<Bloom>(), "LOW must drop bloom");
    let cascades = app
        .world()
        .entity(sun)
        .get::<bevy::light::CascadeShadowConfig>()
        .expect("the applier must write a cascade config onto the sun");
    assert_eq!(
        cascades.bounds.len(),
        low.cascades,
        "the sun kept {} cascades where LOW asks for {}",
        cascades.bounds.len(),
        low.cascades
    );
    assert_eq!(
        app.world().resource::<DirectionalLightShadowMap>().size,
        low.shadow_map_px,
        "the shadow map size did not follow the tier"
    );

    // The forest: the resource moved, and both standing parts followed it.
    let want = TreeLod::at(low.tree_lod_swap_m);
    assert!(
        *app.world().resource::<TreeLod>() == want,
        "the tree LOD resource did not follow the tier, so every chunk that \
         streams in from here still swaps at the old distance"
    );
    assert!(
        app.world().entity(near).get::<VisibilityRange>() == Some(&want.near),
        "a tree already standing kept its old near band — the trees in the \
         ring when the knob moved would be the only ones not obeying it"
    );
    assert!(
        app.world().entity(far).get::<VisibilityRange>() == Some(&want.far),
        "…and the same for the hull, which would leave a gap between the two \
         bands: the near half vanishing at 35 m and the hull not appearing \
         until 80"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// The rows, and the engine ceiling that made them necessary
// (2026-09-16, "shadow stuff is kinda garbage with distance? mayb its web?")
// ─────────────────────────────────────────────────────────────────────────

/// **The bug this slice exists to fix, stated as a property: the renderer
/// draws exactly the cascades we asked for.**
///
/// It did not. `bevy_pbr::render::light::MAX_CASCADES_PER_LIGHT` is 4 — and
/// **1** under `all(feature = "webgl", target_arch = "wasm32",
/// not(feature = "webgpu"))`, which is precisely what this client builds for
/// the browser. Bevy does not refuse an over-long request and does not scale
/// it: `prepare_lights` takes `.min(MAX_CASCADES_PER_LIGHT)` of the bounds
/// list, keeping cascade **0**, whose far bound is `CASCADE_FIRST_M`. So the
/// browser tier's "two cascades to 90 m" drew shadows to **12 m** and nothing
/// beyond, with a one-line `warn!` into a console nobody reads and every gate
/// in this repo green.
///
/// The property below is target-independent by construction — it is the same
/// code either side of the `cfg`, with a different ceiling — so a native run
/// proves the mechanism, which is the half that was broken.
#[test]
fn every_cascade_asked_for_is_a_cascade_the_engine_will_draw() {
    for q in Quality::LADDER {
        let g = effective(preset(q));
        assert!(
            g.cascades >= 1 && g.cascades <= max_cascades(),
            "{q:?} resolves to {} cascades where this target draws at most {} \
             — the engine silently keeps the first ones and drops the rest, \
             which is a shadow distance nobody chose",
            g.cascades,
            max_cascades()
        );
        let bounds = components(g).cascades().bounds;
        assert_eq!(
            bounds.len(),
            g.cascades,
            "{q:?} asked for {} cascades and the builder produced {} bounds",
            g.cascades,
            bounds.len()
        );
        // The last bound IS the shadow distance. This is what "shadows stop
        // at 12 m" looked like from here: the list was right and only its
        // first entry was ever read.
        assert!(
            (bounds[bounds.len() - 1] - g.shadow_m).abs() < 1e-3,
            "{q:?}'s last cascade ends at {} m, not at the {} m the row says",
            bounds[bounds.len() - 1],
            g.shadow_m
        );
    }
}

/// A one-cascade config covers the WHOLE distance, which is the browser fix.
///
/// Written against an explicit [`Gfx`] rather than against whatever this
/// target resolves to, so the browser's shape is gated on a desktop: with one
/// cascade `calculate_cascade_bounds` short-circuits to `[maximum_distance]`
/// and `first_cascade_far_bound` is ignored entirely. Asking for two and
/// getting one is the failure; asking for one and getting the whole distance
/// is the fix, and the difference is visible here without a browser.
#[test]
fn one_cascade_reaches_the_whole_shadow_distance() {
    let g = Gfx {
        cascades: 1,
        shadow_m: 90.0,
        ..preset(Quality::Low)
    };
    let bounds = components(g).cascades().bounds;
    assert_eq!(bounds.len(), 1);
    assert!(
        (bounds[0] - 90.0).abs() < 1e-3,
        "one cascade must reach the full distance, got {bounds:?}"
    );
    // And the shape that shipped: two cascades, of which a WebGL2 browser
    // reads one — the first, ending at `CASCADE_FIRST_M`.
    let two = Gfx {
        cascades: 2,
        shadow_m: 90.0,
        ..preset(Quality::Low)
    };
    let split = components(two).cascades().bounds;
    assert!(
        (split[0] - CASCADE_FIRST_M).abs() < 1e-3,
        "the first of two cascades ends at CASCADE_FIRST_M ({CASCADE_FIRST_M} m), \
         which is the distance every shadow in the browser build stopped at — \
         got {split:?}"
    );
}

/// [`far_texel_m`]'s cascade bounds are **Bevy's**, not a second guess at
/// them.
///
/// `CLAUDE.md`: a naive rebuild that calls the function under test is a
/// rebuild of nothing. The readout re-derives the geometric split so it can
/// run with no camera in the room, and the half of that which can drift under
/// an engine upgrade is the split itself — which Bevy publishes, on
/// `CascadeShadowConfig::bounds`. So it is checked against the engine rather
/// than against itself, and only the frustum-corner arithmetic below is ours
/// to hold with a golden.
#[test]
fn the_far_edge_readout_splits_the_cascades_the_way_bevy_does() {
    for q in Quality::LADDER {
        let g = effective(preset(q));
        let bounds = components(g).cascades().bounds;
        // The readout's own far bound, recovered by inverting the divide:
        // `texel = ceil(diameter) / px`, and the diameter is monotone in the
        // far bound, so a disagreement about where the last cascade ENDS
        // shows up as a different texel. Asserting on the bounds directly is
        // the stronger statement and is what this gate is for.
        let want_far = bounds[bounds.len() - 1];
        assert!(
            (want_far - g.shadow_m).abs() < 1e-3,
            "{q:?}: Bevy ends the last cascade at {want_far} and the row says \
             {}",
            g.shadow_m
        );
        // Positive, finite and coarser than the near field — the three things
        // that must be true of any answer at all.
        let cm = far_texel_m(g, 75.0) * 100.0;
        assert!(
            cm.is_finite() && cm > 0.0,
            "{q:?} produced a far-edge texel of {cm} cm"
        );
    }
}

/// **The golden for the readout**, computed independently and written by hand.
///
/// These are the numbers that answer the question this slice was asked, at
/// `fov_deg = 75` and 16:9 (`quality::READOUT_ASPECT`):
///
///   * `High` — four cascades to 200 m at 2048 px — puts the last cascade
///     from 62.6 m to 200 m inside a **627 m** ortho box: **30.6 cm** of world
///     per shadow texel, against 1.86 cm in the first cascade. A shadow edge
///     cannot be finer than one texel, so that is the size of the stair-step
///     out there.
///   * `Low` at 1024 px is 27.5 cm, and a browser's single cascade over the
///     same 90 m at 2048 px is **13.8 cm** — twice as fine as the tier it
///     replaces, and reaching 90 m instead of 12.
///   * The same `High` config at 4096 px is 15.3 cm, which is what the top of
///     `SHADOW_PX_LADDER` buys and why it is on the ladder.
///
/// **Tolerance is 0.005 cm, and it was chosen by running the mutants.** At
/// half a centimetre — the first draft — one mutant survived: dropping the
/// `.ceil()` Bevy applies to the cascade diameter, which moves these readings
/// by 0.013–0.046 cm and would have left an assertion in this file that could
/// not fail for the reason it names. `f32` noise on a `tan` at this magnitude
/// is four orders of magnitude below the bound, so the tight one is free.
#[test]
fn the_far_edge_readout_matches_its_measurement() {
    let cases: [(&str, Gfx, f32); 4] = [
        ("High 4x200 @2048", preset(Quality::High), 30.6152),
        (
            // Every field that enters the arithmetic written out, because
            // `preset(Low)`'s shadow map is the one value in that table that
            // differs by target (2048 in a browser, where the cascade clamp
            // hands the budget back). A `..preset(Quality::Low)` here would
            // be a golden that reads 13.77 on one target and 27.54 on the
            // other while claiming to be a statement about arithmetic.
            "Low 2x90 @1024",
            Gfx {
                cascades: 2,
                shadow_m: 90.0,
                shadow_map_px: 1024,
                ..preset(Quality::Low)
            },
            27.5391,
        ),
        (
            "browser 1x90 @2048",
            Gfx {
                cascades: 1,
                shadow_m: 90.0,
                shadow_map_px: 2048,
                ..preset(Quality::Low)
            },
            13.7695,
        ),
        (
            "High 4x200 @4096",
            Gfx {
                shadow_map_px: 4096,
                ..preset(Quality::High)
            },
            15.3076,
        ),
    ];
    for (name, g, want_cm) in cases {
        // Deliberately NOT through `effective`: these are statements about
        // the arithmetic, and a browser run must check the same numbers a
        // desktop does.
        let got = far_texel_m(g, 75.0) * 100.0;
        assert!(
            (got - want_cm).abs() < 0.005,
            "{name}: the far-edge texel reads {got:.4} cm against a measured \
             {want_cm:.4} cm"
        );
    }
    // A wider field of view is a bigger frustum and therefore a coarser
    // shadow, which is worth pinning because it is the one coupling on this
    // screen a player would never guess: the FIELD OF VIEW row moves the
    // shadow detail row.
    let g = preset(Quality::High);
    assert!(
        far_texel_m(g, 110.0) > far_texel_m(g, 60.0) * 1.5,
        "widening the fov must coarsen the shadows — 60 deg reads {:.2} cm \
         and 110 deg reads {:.2} cm",
        far_texel_m(g, 60.0) * 100.0,
        far_texel_m(g, 110.0) * 100.0
    );
}

/// What the shadow maps cost, so the screen can say it before a player picks.
#[test]
fn the_shadow_memory_readout_is_the_texture_bevy_allocates() {
    // One `Depth32Float` array, `px` square, one layer per cascade.
    let mib = |px: usize, n: usize| (px * px * n * 4) as f32 / (1024.0 * 1024.0);
    assert!((shadow_vram_mb(preset(Quality::High)) - mib(2048, 4)).abs() < 1e-3);
    assert!((shadow_vram_mb(preset(Quality::High)) - 64.0).abs() < 1e-3);
    let top = Gfx {
        shadow_map_px: 4096,
        ..preset(Quality::High)
    };
    assert!(
        (shadow_vram_mb(top) - 256.0).abs() < 1e-3,
        "the top of both ladders is a quarter of a gigabyte and the screen \
         has to say so, got {}",
        shadow_vram_mb(top)
    );
    let off = Gfx {
        shadows: false,
        ..preset(Quality::High)
    };
    assert_eq!(
        shadow_vram_mb(off),
        0.0,
        "shadows off must cost no shadow map"
    );
}

/// Every preset lands ON its ladder, so a stepper can get back to it.
///
/// **A preset the rows cannot reproduce is a one-way trip**: a player who
/// nudges the shadow distance off HIGH could never step back onto it, and the
/// QUALITY row would read CUSTOM forever with no way home but the preset
/// button. The ladders must also ascend, which is what [`step_ladder`]'s
/// "nearest rung at or above" search assumes.
///
/// [`step_ladder`]: client::render::settings
#[test]
fn every_preset_is_reachable_from_the_steppers() {
    let ascends = |name: &str, xs: &[f32]| {
        for w in xs.windows(2) {
            assert!(w[0] < w[1], "{name} does not ascend: {xs:?}");
        }
    };
    ascends("SHADOW_M_LADDER", &SHADOW_M_LADDER);
    ascends("TREE_LOD_LADDER", &TREE_LOD_LADDER);
    for w in SHADOW_PX_LADDER.windows(2) {
        assert!(w[0] < w[1], "SHADOW_PX_LADDER does not ascend");
    }
    // Powers of two, because `calculate_cascade` divides an integer cascade
    // diameter by this and says in as many words that a power of two is what
    // keeps the texel size exactly representable — an off-power size makes
    // shadow edges crawl as the camera moves.
    for px in SHADOW_PX_LADDER {
        assert!(
            px.is_power_of_two(),
            "{px} is not a power of two, which is what stops shadow edges \
             crawling under a moving camera"
        );
    }
    // **The ladder's floor against `CASCADE_FIRST_M` is structural and lives
    // in `quality.rs` as a `const _: () = assert!(..)`** — a relation between
    // two constants can only ever be constant-folded here, so it is checked
    // at BUILD time instead, which is the right severity for "the settings
    // screen can hand the engine an input it panics on". What this file can
    // usefully add is the consequence: every rung actually builds.
    for m in SHADOW_M_LADDER {
        for n in 1..=max_cascades() {
            let g = Gfx {
                cascades: n,
                shadow_m: m,
                ..preset(Quality::High)
            };
            let bounds = components(g).cascades().bounds;
            assert_eq!(bounds.len(), n, "{n} cascades to {m} m built {bounds:?}");
        }
    }
    for q in Quality::LADDER {
        let g = preset(q);
        assert!(
            SHADOW_M_LADDER.contains(&g.shadow_m),
            "{q:?}'s {} m shadow distance is not a rung of SHADOW_M_LADDER",
            g.shadow_m
        );
        assert!(
            SHADOW_PX_LADDER.contains(&g.shadow_map_px),
            "{q:?}'s {}px shadow map is not a rung of SHADOW_PX_LADDER",
            g.shadow_map_px
        );
        assert!(
            TREE_LOD_LADDER.contains(&g.tree_lod_swap_m),
            "{q:?}'s {} m tree swap is not a rung of TREE_LOD_LADDER",
            g.tree_lod_swap_m
        );
        assert!(
            Ao::LADDER.contains(&g.ao),
            "{q:?}'s ambient occlusion is not on Ao::LADDER"
        );
        assert!(
            g.cascades <= max_cascades() || max_cascades() < 4,
            "{q:?} asks for more cascades than this target can draw"
        );
    }
}

/// The QUALITY row reads CUSTOM when — and only when — a row has moved.
///
/// **Derived, never flagged.** A boolean "the player customized it" is a
/// second copy of the truth and the copy is what goes stale: a player who
/// pulls a row off HIGH and puts it back is on HIGH, with nothing to reset.
#[test]
fn a_moved_row_reads_custom_and_moving_it_back_does_not() {
    for q in Quality::LADDER {
        let mut s = Settings::with_preset(q);
        assert_eq!(
            s.preset_name(),
            Some(q),
            "{q:?} built from its own preset must read as that preset"
        );
        // One click down the shadow distance, then one back up.
        s.adjust(Knob::ShadowDistance, -1);
        assert_ne!(
            s.gfx.shadow_m,
            preset(q).shadow_m,
            "the stepper did not move the shadow distance off {q:?}"
        );
        assert_eq!(
            s.preset_name(),
            None,
            "{q:?} with the shadow distance pulled in still claims to be a \
             preset — the screen would be stating a frame the renderer is \
             not drawing"
        );
        s.adjust(Knob::ShadowDistance, 1);
        assert_eq!(
            s.preset_name(),
            Some(q),
            "stepping back onto {q:?}'s own rung must be {q:?} again, with \
             nothing to reset"
        );
    }
    // And the preset row still writes every row below it.
    let mut s = Settings::with_preset(Quality::High);
    s.adjust(Knob::Bloom, 0);
    s.adjust(Knob::Ambient, -1);
    assert_eq!(s.preset_name(), None);
    s.adjust(Knob::Quality, -1);
    assert_eq!(
        s.gfx,
        effective(preset(Quality::Medium)),
        "one click of the QUALITY row must rewrite every row below it"
    );
    assert_eq!(s.preset_name(), Some(Quality::Medium));
}

/// Every stepper stops at both ends of its ladder and never leaves it.
///
/// The `MEDIUM`/`UNCAPPED` shape of bug: a row that wrapped would put the far
/// end of the ladder one click from where the player was looking, and a row
/// that walked off the end would hand `CascadeShadowConfigBuilder` an input
/// it asserts on — a panic, from a settings screen.
#[test]
fn no_stepper_can_walk_off_its_ladder() {
    for dir in [-1i32, 1] {
        let mut s = Settings::with_preset(Quality::High);
        for _ in 0..12 {
            s.adjust(Knob::ShadowDistance, dir);
            s.adjust(Knob::ShadowCascades, dir);
            s.adjust(Knob::ShadowMapPx, dir);
            s.adjust(Knob::Ambient, dir);
            s.adjust(Knob::TreeLod, dir);
            assert!(
                SHADOW_M_LADDER.contains(&s.gfx.shadow_m),
                "the shadow distance left its ladder at {}",
                s.gfx.shadow_m
            );
            assert!(
                SHADOW_PX_LADDER.contains(&s.gfx.shadow_map_px),
                "the shadow map size left its ladder at {}",
                s.gfx.shadow_map_px
            );
            assert!(
                TREE_LOD_LADDER.contains(&s.gfx.tree_lod_swap_m),
                "the tree swap left its ladder at {}",
                s.gfx.tree_lod_swap_m
            );
            assert!(
                s.gfx.cascades >= 1 && s.gfx.cascades <= max_cascades(),
                "the cascade stepper reached {}, which this target cannot \
                 draw",
                s.gfx.cascades
            );
            assert!(s.gfx.shadow_map_px <= max_shadow_map_px());
            // The whole point of staying on the ladder: whatever the walk
            // reached must still build.
            let _ = components(effective(s.gfx)).cascades();
        }
        let end = if dir < 0 {
            0
        } else {
            SHADOW_M_LADDER.len() - 1
        };
        assert_eq!(
            s.gfx.shadow_m, SHADOW_M_LADDER[end],
            "the walk must rest against the end it ran at"
        );
    }
}

/// [`effective`] is idempotent, which is what lets a preset be STORED already
/// resolved.
///
/// `default_gfx`, `Settings::with_preset`, the QUALITY stepper and the file
/// loader all write `effective(preset(q))`, and `preset_name` then looks the
/// stored value up by comparing against the same expression. If applying the
/// clamp twice moved anything, a fresh install on the clamped target would
/// open reading CUSTOM — before the player had touched a thing.
#[test]
fn the_target_clamp_is_idempotent() {
    let mut seen = Vec::new();
    for q in Quality::LADDER {
        seen.push(preset(q));
    }
    // Plus the corners a hand-edited file can reach, so the fixed point is
    // checked somewhere other than on the three rungs.
    for px in SHADOW_PX_LADDER {
        for n in 1..=6usize {
            seen.push(Gfx {
                cascades: n,
                shadow_map_px: px,
                tree_lod_swap_m: TREE_LOD_LADDER[0],
                ..preset(Quality::High)
            });
        }
    }
    for g in seen {
        let once = effective(g);
        assert_eq!(
            effective(once),
            once,
            "applying the target clamp twice moved {g:?}"
        );
    }
}

/// A value BETWEEN two rungs steps onto the rung it is between, not past it.
///
/// Only a hand-edited settings file can produce one (the loader clamps the
/// ends but honours the middle, `settings::gfx_from_file`), and the obvious
/// index-step — from "the nearest rung at or above" — sends `+` from 120 m to
/// **200**, skipping the 140 it is sitting just below. The player watching
/// that number would have no idea what they did.
#[test]
fn a_stepper_lands_on_the_rung_it_is_between() {
    // 120 m is not a rung: SHADOW_M_LADDER goes 90, 140.
    let between = |dir: i32| {
        let mut s = Settings::with_preset(Quality::High);
        s.gfx.shadow_m = 120.0;
        s.adjust(Knob::ShadowDistance, dir);
        s.gfx.shadow_m
    };
    assert_eq!(between(1), 140.0, "+ from 120 m must reach 140, not 200");
    assert_eq!(between(-1), 90.0, "- from 120 m must reach 90");

    // And both ends still hold against an off-ladder value outside them.
    let mut past = Settings::with_preset(Quality::High);
    past.gfx.shadow_m = 9000.0;
    past.adjust(Knob::ShadowDistance, 1);
    assert_eq!(
        past.gfx.shadow_m,
        SHADOW_M_LADDER[SHADOW_M_LADDER.len() - 1],
        "stepping up from past the top must rest on the top rung"
    );
}

/// Turning shadows off costs no cascades, and turning them back on costs no
/// distance.
#[test]
fn the_shadow_toggle_keeps_the_distance_it_was_given() {
    let mut s = Settings::with_preset(Quality::High);
    s.adjust(Knob::ShadowDistance, -1);
    let kept = s.gfx.shadow_m;
    s.adjust(Knob::Shadows, 0);
    assert!(!s.gfx.shadows, "the toggle did not turn shadows off");
    assert_eq!(
        s.gfx.shadow_m, kept,
        "turning shadows off must not cost the distance the player picked — \
         a row that forgot it would reset to the preset on the way back"
    );
    s.adjust(Knob::Shadows, 0);
    assert!(s.gfx.shadows);
    assert_eq!(s.gfx.shadow_m, kept);
}
