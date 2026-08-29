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
use client::config::Quality;
use client::render::props::{FellPart, Fellable, Topple};
use client::render::quality::{apply, reband_trees, tier};
use client::render::rig::{EyeCam, Sun};
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
    app.insert_resource(Settings {
        quality: Quality::Low,
        ..default()
    });
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

    // Frame two: the bottom rung.
    app.world_mut().resource_mut::<Settings>().quality = Quality::Low;
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
