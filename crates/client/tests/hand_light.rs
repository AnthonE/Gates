//! Gate: the torch in your hand lights the ground, nothing else in the hand
//! does, and the number it lights it with sits where the ladder says.
//!
//! **The gap this closes was ranked first by a judge**
//! (`findings/pass-20260829-153230-03-judge.md`): night is a pressure the
//! sim spends real effort on — `world::is_night` drives `brain::sense`'s
//! nocturnal notice radius, `rig::day_night` takes the sun, the environment
//! map and the sky brightness all to zero — and the starter kit has put a
//! torch on hotbar slot 2 since the kit existed, where it did nothing at
//! all. A player's whole answer to nightfall was to stand still.
//!
//! Four halves, and no other gate in this crate can see any of them:
//!
//! 1. **The spawn shape.** Whether the hand gets an emitter is a claim
//!    about a bundle, and `CLAUDE.md` says a spawn is not type-checked.
//!    That claim is checked here as a call site (`the_hand_light_is_hung_*`)
//!    rather than as a value, for `tests/sound.rs`' reason: the defect
//!    would be *where* it is hung, and every value would still be right.
//! 2. **The drive.** `apply_hand_light` is the whole behaviour and it is
//!    one match on a row. Invert the `light.is_some()` and every item in the
//!    game glows except the torch, with the same green suite.
//! 3. **The ladder.** `.claude/skills/threejs-procedural-vfx` states the
//!    rule this pass was designed against — emission is **ordinal**,
//!    "evidence of relative hierarchy inside that scene, not universal
//!    exposure-independent constants", so what has to be gated is
//!    `torch < campfire`, not the literal 600 — and what the fixed exposure
//!    makes of it at night under `rig::flame_gain`, which is the half that
//!    was black.
//! 4. **The geometry.** `viewmodel::flame_at` reads the flame off the pose
//!    the mesh is drawn with, so it cannot drift when the mesh is
//!    regenerated — `held_assets.rs` makes exactly that argument about
//!    `grip_m`. Here it is measured off the built mesh in all three axes, so
//!    a torch whose head moves takes its light with it or this goes red.
//!
//! **What is NOT gated, said plainly:** whether any of it looks right.
//! `gates --capture --hour midnight` shoots the night; `CLAUDE.md` makes a
//! person the visual gate and forbids building a pixel one.

#![cfg(feature = "render")]

use bevy::camera::Exposure;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::heldgen;
use client::render::rig;
use client::render::structures::fire_lumens;
use client::render::viewmodel::{apply_hand_light, flame_at, pose, HandLight, VIEWMODEL_PALM};
use client::ui::hold::{HeldSrc, FLAME_LIFT_M, HELD_MODELS, TORCH_LIGHT};

fn torch_row() -> usize {
    HELD_MODELS
        .iter()
        .position(|m| m.key == "torch")
        .expect("no torch row in HELD_MODELS")
}

/// A world with one emitter in it, in the state `spawn_item` leaves it:
/// dark, at the origin of the hand.
fn app() -> App {
    let mut app = App::new();
    app.world_mut().spawn((
        HandLight,
        PointLight {
            intensity: 0.0,
            range: 0.0,
            ..default()
        },
        Transform::IDENTITY,
    ));
    app.world_mut().flush();
    app
}

fn read(app: &mut App) -> (f32, f32, Vec3) {
    let (light, tf) = app
        .world_mut()
        .query::<(&PointLight, &Transform)>()
        .iter(app.world())
        .next()
        .expect("no hand light");
    (light.intensity, light.range, tf.translation)
}

fn drive(app: &mut App, want: Option<usize>) {
    app.world_mut()
        .run_system_once(
            move |q: Query<(&mut PointLight, &mut Transform), With<HandLight>>| {
                apply_hand_light(want, 1.0, q);
            },
        )
        .unwrap();
}

/// The drive half. A torch lights, an empty hand does not, and neither does
/// anything else a hand can hold.
#[test]
fn only_the_torch_lights_the_ground() {
    let mut app = app();
    let torch = torch_row();

    drive(&mut app, None);
    assert_eq!(
        read(&mut app),
        (0.0, 0.0, Vec3::ZERO),
        "an empty hand is emitting light"
    );

    drive(&mut app, Some(torch));
    let row = &HELD_MODELS[torch];
    assert_eq!(
        read(&mut app),
        (TORCH_LIGHT.lumens, TORCH_LIGHT.range_m, flame_at(row)),
        "the torch in hand is not lighting the ground, or is lighting it from \
         the wrong place — night is the tenth of every cycle this is the only \
         answer to"
    );

    // Every other row, one at a time. This is the assertion that catches an
    // inverted `light.is_some()`, and it catches it on thirteen rows rather
    // than on one.
    for (i, m) in HELD_MODELS.iter().enumerate() {
        if i == torch {
            continue;
        }
        drive(&mut app, Some(i));
        assert_eq!(
            read(&mut app),
            (0.0, 0.0, Vec3::ZERO),
            "{} is casting light and has no `light` on its row — a rock that \
             glows is worse than a torch that does not",
            m.key
        );
    }

    // And back off from the lit state, which is the transition that would
    // leave a light burning for an item no longer in the hand.
    drive(&mut app, Some(torch));
    drive(&mut app, None);
    assert_eq!(
        read(&mut app),
        (0.0, 0.0, Vec3::ZERO),
        "putting the torch away left it burning"
    );
}

/// Exactly one row declares a light, and it is the torch. Separate from the
/// behaviour above because it is a claim about the TABLE: the drive is
/// correct for whatever the table says, and this is what says the table is
/// right.
#[test]
fn exactly_one_held_row_is_a_light_source() {
    let lit: Vec<&str> = HELD_MODELS
        .iter()
        .filter(|m| m.light.is_some())
        .map(|m| m.key)
        .collect();
    assert_eq!(
        lit,
        vec!["torch"],
        "the set of held items that emit light changed. That is allowed — but \
         it is a gameplay claim (a light is a beacon that discloses you) and a \
         cost claim (`DECISIONS.md` §open), so it lands here deliberately or \
         not at all"
    );
}

/// The ladder, `torch < campfire`, and the question the operator asked of
/// it (2026-10-07, *"i cant see it lighting the area"*): what the frame makes
/// of the torch at night.
///
/// **Measured against the camera, not against the ambient.** This used to
/// gate the torch's pool against `rig::NIGHT_AMBIENT_LUX` and passed at
/// 0.89 m while the frame drew the ground under a lit torch at 1/255: the
/// exposure is daylight's and fixed (`rig::DAY_EV100`), so 600 lm is black
/// whatever the ambient says. The night eye (`rig::flame_gain`) is what
/// brings it into the frame, and this holds the ground under a torch held
/// at head height inside a band that reads, at deep night and at noon.
#[test]
fn the_torch_sits_under_the_campfire_and_lights_the_night() {
    // Read through the ROW rather than off `TORCH_LIGHT`. Two reasons: the
    // row is what the client actually drives from, and a comparison between
    // two constants is one clippy folds to a literal `true`, which is an
    // assertion that has stopped being one.
    let lit = HELD_MODELS[torch_row()]
        .light
        .expect("the torch row lost its light");
    assert!(lit.lumens > 0.0, "a light source with no lumens in it");
    assert_eq!(
        lit, TORCH_LIGHT,
        "the torch row is carrying a light that is not `TORCH_LIGHT`"
    );
    assert!(
        lit.lumens < fire_lumens(),
        "the torch ({} lm) is brighter than a campfire ({} lm) — the \
         HIERARCHY is the invariant (`.claude/skills/threejs-procedural-vfx`), \
         and both burn under the same night gain",
        lit.lumens,
        fire_lumens()
    );

    // Linear frame value of grass (albedo 0.25) `d` metres under the flame,
    // straight below it, through the fixed exposure.
    let exposure = Exposure {
        ev100: rig::DAY_EV100,
    }
    .exposure();
    let ground = |lumens: f32, d: f32| {
        0.25 / std::f32::consts::PI * lumens / (4.0 * std::f32::consts::PI * d * d) * exposure
    };
    let midnight = (1.0 + sim_core::limits::DAY_PORTION) * 0.5;
    let noon = sim_core::limits::DAY_PORTION * 0.5;
    let night = ground(lit.lumens * rig::flame_gain(midnight, 0.0), 1.8);
    assert!(
        night > 0.03,
        "at midnight the ground under a lit torch is {night:.5} of white — \
         black on black, the operator's frame"
    );
    assert!(
        night < 0.5,
        "at midnight the ground under a lit torch is {night:.3} of white — a \
         floodlight in a fist"
    );
    // Out past a stride or two it has to fall away, or it is a lamp post.
    let far = ground(lit.lumens * rig::flame_gain(midnight, 0.0), 8.0);
    assert!(
        far < 0.01,
        "8 m from the torch the ground is still {far:.4}"
    );
    assert_eq!(rig::flame_gain(noon, 0.0), 1.0, "the eye opens at noon");
    let day = ground(lit.lumens * rig::flame_gain(noon, 0.0), 1.8);
    assert!(
        day < 0.001,
        "a torch at noon puts {day:.4} on the ground — a second sun"
    );
}

/// The geometry half: the emitter sits above the crown of the mesh it comes
/// out of, measured off the mesh rather than off the table.
///
/// **This is what stops a light drifting inside a head.** A point light does
/// not illuminate the surface it stands on, so an offset that fell a
/// centimetre short would leave the torch's own wrap black while everything
/// around it brightened — a lamp with a dark bulb, and no value anywhere
/// would be wrong.
#[test]
fn the_flame_sits_above_the_head_it_comes_out_of() {
    for m in HELD_MODELS.iter().filter(|m| m.light.is_some()) {
        assert!(
            m.lay == 0.0,
            "{} declares a light and is laid forward. `flame_m` is spent up \
             the hold frame's +Y, which for a laid-forward row is the model's \
             own −Z — the flame would be pushed out of the frame rather than \
             up out of the head",
            m.key
        );
        let HeldSrc::Gen(name) = m.src else {
            // A `.glb` row would be measured off the file, `held_assets.rs`'
            // way. None exists, and the day one does this arm is the work.
            panic!(
                "{} declares a light and is not a generated row — this gate \
                 measures the mesh and only knows how to build one",
                m.key
            );
        };
        let mesh = heldgen::mesh(name);
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("{name} has no Float32x3 POSITION");
        };
        let crown = pos.iter().fold(f32::MIN, |a, p| a.max(p[1])) * m.scale;
        let flame = m.flame_m();
        let above = flame - (crown - m.grip_m());
        assert!(
            above > 0.005,
            "{}'s light sits {above:.4} m above its own crown (flame {flame:.4} \
             m over the fist, crown {crown:.4} m over the model's foot, grip \
             {:.4} m). At or below the crown the head it belongs to is the one \
             surface it cannot light",
            m.key,
            m.grip_m()
        );
        assert!(
            above < 0.15,
            "{}'s light floats {above:.4} m clear of the mesh — a source with \
             no visible cause hanging over the hand",
            m.key
        );
        // **And ON the head, not beside it** — the half this gate missed
        // while the fire burned 13 cm off the torch (operator, 2026-10-07:
        // *"its not near the actual torch"*). The crown's vertices are
        // posed the way `swap` poses the model, palm offset and all, and the
        // flame has to sit over their middle.
        let top: Vec<Vec3> = pos
            .iter()
            .map(|p| Vec3::from_array(*p))
            .filter(|p| p.y * m.scale > crown - 0.03)
            .collect();
        let mid = top.iter().copied().sum::<Vec3>() / top.len() as f32;
        let head = pose(m, VIEWMODEL_PALM).transform_point(mid);
        let at = flame_at(m);
        let off = Vec2::new(at.x - head.x, at.z - head.z).length();
        assert!(
            off < 0.01,
            "{}'s flame sits {off:.3} m beside its own head in the hold frame \
             (flame {at:?}, head {head:?})",
            m.key
        );
        assert!(
            at.y > head.y && at.y - head.y < FLAME_LIFT_M + 0.03,
            "{}'s flame is not just over its head (flame {at:?}, head {head:?})",
            m.key
        );
    }
}

/// The call-site half, `tests/sound.rs`' shape: two facts that are true of
/// *where* the emitter is written and would survive every value assertion
/// above.
///
/// **The parent is the fact that matters.** Hung on `HeldModel` instead of
/// `HeldItem` the light would inherit `swap`'s per-item pose — a `grip_m`
/// slide and a `pose_yaw` that are corrections for where a MESH sits in a
/// fist and are meaningless to a point source — and it would be re-posed
/// every time the hand changed. It reads correctly on the torch either way,
/// because the torch is upright with no yaw, so nothing here would catch it
/// until the second lit row landed.
#[test]
fn the_hand_light_is_hung_on_the_hand_and_burns_the_one_flame_colour() {
    let src = std::fs::read_to_string("src/render/viewmodel.rs").expect("viewmodel.rs");
    let at = src
        .find("HandLight,\n")
        .expect("no `HandLight,` spawn in viewmodel.rs — the emitter is not spawned anywhere");
    let block = &src[at..(at + 400).min(src.len())];
    assert!(
        block.contains("structures::FIRE_COLOR"),
        "the hand light does not take `structures::FIRE_COLOR`. A torch and a \
         fire pit burn the same thing and there is exactly one ember orange in \
         this client; a second literal here is a drift that no frame would \
         report and no value gate could see"
    );
    // The spawn is inside the `item.spawn` run — the children of `HeldItem`
    // — rather than beside the model.
    let head = &src[..at];
    let last_spawn = head
        .rfind("item.spawn((")
        .expect("no item.spawn in viewmodel.rs");
    assert!(
        !head[last_spawn..].contains("HeldModel {"),
        "the hand light is spawned after the `HeldModel` entity's own \
         `item.spawn` opened — check it is a sibling of the model under \
         `HeldItem`, not a child of it"
    );
}
