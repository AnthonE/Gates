//! Gate: the browser's tree LOD swap does what the desktop's `VisibilityRange`
//! does, minus the fade.
//!
//! WebGL2 cannot bind the table `VisibilityRange` dithers by — Bevy 0.18.1's
//! view layout keeps a 16-byte `min_binding_size` on a 1,024-byte uniform
//! fallback, and wgpu refuses the first tree's pipeline
//! (`findings/web-build-20260909.md` §15.6) — so on wasm32 no tree part
//! carries the component and `tree::swap_by_distance` toggles `Visibility`
//! by hand. It is compiled on every target and registered on that one, which
//! is what lets this suite drive it here. Four things a wrong version does
//! silently, because a wrong `Visibility` is a tree that is missing or doubled
//! and nothing else:
//!
//!  1. the pair and the hull swap at `TreeLod::swap_m()`, in both directions;
//!  2. it swaps where the desktop swaps — `swap_m()` is the near band's end
//!     margin start, which `tests/tree.rs` holds contiguous with the far one;
//!  3. the stump and a vanishing prop are never written — they are the fell
//!     system's — and neither is a part that already has the right answer,
//!     because a `Visibility` write re-extracts the entity;
//!  4. the desktop still carries the component and a browser build does not.
//!
//! Headless — `MinimalPlugins`, no GPU, no transform propagation (the system
//! reads `GlobalTransform`, so it is spawned directly).

#![cfg(feature = "render")]

use bevy::ecs::change_detection::DetectChanges;
use bevy::prelude::*;

use client::render::props::{FellPart, Fellable};
use client::render::tree::{self, TreeLod, TREE_LOD_SWAP_M};
use client::render::Eye;

fn app(eye_x: f32) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<TreeLod>();
    app.insert_resource(Eye {
        pos: Vec3::new(eye_x, 0.0, 0.0),
        ..default()
    });
    app.add_systems(Update, tree::swap_by_distance);
    app
}

fn part(app: &mut App, part: FellPart, x: f32) -> Entity {
    app.world_mut()
        .spawn((
            Fellable {
                key: 1,
                variant: 0,
                base_y: 0.0,
                yaw: 0.0,
                part,
                felled: false,
            },
            GlobalTransform::from_xyz(x, 0.0, 0.0),
            Visibility::Inherited,
        ))
        .id()
}

fn vis(app: &App, e: Entity) -> Visibility {
    *app.world().entity(e).get::<Visibility>().unwrap()
}

// ── 1 + 2. The swap, at the desktop's distance, both ways ──────────────────

#[test]
fn the_pair_shows_inside_the_swap_and_the_hull_outside_it() {
    let swap = TreeLod::default().swap_m();
    assert!(
        (swap - TREE_LOD_SWAP_M).abs() < 1e-6,
        "the shipped swap is TREE_LOD_SWAP_M = {TREE_LOD_SWAP_M} m, swap_m() says {swap}"
    );
    // Where the desktop's bands change over: the near band's end margin
    // begins where the far band's start margin does.
    let lod = TreeLod::default();
    assert_eq!(lod.near.end_margin.start, lod.far.start_margin.start);

    let mut app = app(0.0);
    // A tree just inside the swap and one just outside it, three parts each.
    let inside = swap - 1.0;
    let outside = swap + 1.0;
    let near_in = [
        part(&mut app, FellPart::Trunk, inside),
        part(&mut app, FellPart::Canopy, inside),
    ];
    let hull_in = part(&mut app, FellPart::Far, inside);
    let near_out = [
        part(&mut app, FellPart::Trunk, outside),
        part(&mut app, FellPart::Canopy, outside),
    ];
    let hull_out = part(&mut app, FellPart::Far, outside);
    app.update();

    for e in near_in {
        assert_eq!(
            vis(&app, e),
            Visibility::Inherited,
            "the near pair inside the swap draws"
        );
    }
    assert_eq!(
        vis(&app, hull_in),
        Visibility::Hidden,
        "the hull inside the swap is hidden"
    );
    for e in near_out {
        assert_eq!(
            vis(&app, e),
            Visibility::Hidden,
            "the near pair outside the swap is hidden"
        );
    }
    assert_eq!(
        vis(&app, hull_out),
        Visibility::Inherited,
        "the hull outside the swap draws"
    );

    // Walk away: the eye moves, everything swaps the other way.
    app.world_mut().resource_mut::<Eye>().pos.x = -2.5;
    app.update();
    for e in near_in {
        assert_eq!(
            vis(&app, e),
            Visibility::Hidden,
            "walked out of range: the pair hides"
        );
    }
    assert_eq!(
        vis(&app, hull_in),
        Visibility::Inherited,
        "walked out of range: the hull shows"
    );

    // And back, past the far tree.
    app.world_mut().resource_mut::<Eye>().pos.x = outside;
    app.update();
    for e in near_out {
        assert_eq!(
            vis(&app, e),
            Visibility::Inherited,
            "standing at the far tree: its pair draws"
        );
    }
    assert_eq!(vis(&app, hull_out), Visibility::Hidden);
}

// ── 3. Untouched parts, and no write without a change ──────────────────────

#[test]
fn the_stump_and_a_vanishing_prop_are_the_fell_systems_alone() {
    let mut app = app(0.0);
    // A stump hidden by the fell system (its spawn state) far away, and a
    // vanished rock near by: neither answer is this system's to change.
    let stump = part(&mut app, FellPart::Stump, 1_000.0);
    let rock = part(&mut app, FellPart::Vanish, 1.0);
    app.world_mut().entity_mut(stump).insert(Visibility::Hidden);
    app.world_mut().entity_mut(rock).insert(Visibility::Hidden);
    let hull = part(&mut app, FellPart::Far, 1_000.0);
    app.update();
    assert_eq!(
        vis(&app, stump),
        Visibility::Hidden,
        "the stump was written"
    );
    assert_eq!(
        vis(&app, rock),
        Visibility::Hidden,
        "the vanished prop was written"
    );
    assert_eq!(vis(&app, hull), Visibility::Inherited);

    // A second frame with nothing moved writes nothing: `Visibility` is
    // change-detected by extraction, and a write per part per frame would
    // re-extract every tree in the ring every frame.
    app.update();
    let world = app.world_mut();
    let tick = world.change_tick();
    let last = world.last_change_tick();
    for e in [stump, rock, hull] {
        let v = world.entity(e).get_ref::<Visibility>().unwrap();
        assert!(
            !v.last_changed().is_newer_than(last, tick),
            "entity {e:?} had its Visibility written on a frame nothing changed"
        );
    }
}

// ── 4. Which target carries the component ──────────────────────────────────

#[test]
fn the_desktop_carries_the_range_and_a_browser_does_not() {
    // `lod_band` is the one function both spawn sites go through: on this target
    // it hands back the range itself. The browser half is a `cfg` this test
    // cannot run, so its shape is checked as source instead.
    // This suite is type-checked for wasm32 too (the browser-renderer gate
    // runs clippy with `--all-targets`), where `lod_band` returns nothing —
    // so the desktop comparison sits behind the same `cfg` as the desktop
    // half. `assert!`, not `assert_eq!`: `VisibilityRange` has no `Debug`.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let lod = TreeLod::default();
        let native = tree::lod_band(&lod.near);
        assert!(
            native == lod.near,
            "the desktop's lod_band must hand back the range itself"
        );
    }
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/tree.rs"))
        .expect("tree.rs");
    assert!(
        src.contains(
            "#[cfg(target_arch = \"wasm32\")]\npub fn lod_band(_range: &VisibilityRange) {}"
        ),
        "the wasm32 `lod_band` must hand back nothing — a tree part carrying a \
         VisibilityRange in a browser is the refused pipeline of findings §15.6"
    );
    let props =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/props.rs"))
            .expect("props.rs");
    assert_eq!(
        props.matches("tree::lod_band(&lod.").count(),
        3,
        "every tree part's band goes through `tree::lod_band` — trunk, canopy, hull"
    );
    assert!(
        !props.contains("lod.near.clone()") && !props.contains("lod.far.clone()"),
        "a spawn site inserts a VisibilityRange directly, which a browser cannot bind"
    );
}
