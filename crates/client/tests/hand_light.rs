//! Gate: the torch in your hand lights the ground, nothing else in the hand
//! does, and the number it lights it with sits where the ladder says.
//!
//! **The gap this closes was ranked first by a judge**
//! (`findings/pass-20260829-153230-03-judge.md`): night is a pressure the
//! sim spends real effort on — `world::is_night` drives `mob::think`'s
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
//!    `torch < campfire < sun`, not the literal 600.
//! 4. **The geometry.** `flame_m` derives the emitter's offset from the
//!    mesh so it cannot drift when the mesh is regenerated —
//!    `held_assets.rs` makes exactly that argument about `grip_m`. Here it
//!    is measured off the built mesh, so a torch whose head moves takes its
//!    light with it or this goes red.
//!
//! **What is NOT gated, said plainly:** whether any of it looks right.
//! Nobody has seen a night frame in this repo — `rig::CAPTURE_DAY_FRAC`
//! pins every capture to noon, so no vantage any visual judge has ever
//! scored was shot after dark. `CLAUDE.md` makes a person the visual gate
//! and forbids building a pixel one; `NOW.md` §0tl is that ask.

#![cfg(feature = "render")]

use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::heldgen;
use client::render::structures::fire_lumens;
use client::render::viewmodel::{apply_hand_light, HandLight};
use client::ui::hold::{pool_radius_m, HeldSrc, HELD_MODELS, TORCH_LIGHT};

/// `rig`'s night, read from the renderer rather than restated. The coupling
/// is the point: if night gets darker or brighter, every claim below about
/// what a flame is worth re-reads.
use client::render::rig::NIGHT_AMBIENT_LUX;

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

fn read(app: &mut App) -> (f32, f32, f32) {
    let (light, tf) = app
        .world_mut()
        .query::<(&PointLight, &Transform)>()
        .iter(app.world())
        .next()
        .expect("no hand light");
    (light.intensity, light.range, tf.translation.y)
}

fn drive(app: &mut App, want: Option<usize>) {
    app.world_mut()
        .run_system_once(
            move |q: Query<(&mut PointLight, &mut Transform), With<HandLight>>| {
                apply_hand_light(want, q);
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
        (0.0, 0.0, 0.0),
        "an empty hand is emitting light"
    );

    drive(&mut app, Some(torch));
    let row = &HELD_MODELS[torch];
    assert_eq!(
        read(&mut app),
        (TORCH_LIGHT.lumens, TORCH_LIGHT.range_m, row.flame_m()),
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
            (0.0, 0.0, 0.0),
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
        (0.0, 0.0, 0.0),
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

/// The ladder. `sun > campfire > torch > night ambient`, which is the rule
/// the emission skill states and the only thing about these lumens that is
/// not a taste call.
#[test]
fn the_torch_sits_under_the_campfire_and_over_the_night() {
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
        "the torch row is carrying a light that is not `TORCH_LIGHT` — the \
         knob in `DECISIONS.md` and the number in the table have parted"
    );
    assert!(
        lit.lumens < fire_lumens(),
        "the torch ({} lm) is brighter than a campfire ({} lm). Physically a \
         fire pit is many times a burning rag, so this reads as backwards the \
         moment a player stands next to one — and the emission rule in \
         `.claude/skills/threejs-procedural-vfx` is that the HIERARCHY is the \
         invariant, not the numbers",
        lit.lumens,
        fire_lumens()
    );

    // The other end of the ladder: it has to beat the ambient somewhere, or
    // it is a decoration. The radius it beats it at is small — see below —
    // but it is not zero.
    let pool = pool_radius_m(lit.lumens, NIGHT_AMBIENT_LUX);
    assert!(
        pool > 0.5,
        "the torch beats night's {NIGHT_AMBIENT_LUX} lux only within {pool:.2} \
         m of the flame, which does not reach the ground the player stands on"
    );
    assert!(
        pool < 3.0,
        "the torch beats the night ambient out to {pool:.2} m. That is a \
         floodlight in a fist, and it would have to be far over the campfire \
         to do it"
    );
}

/// `pool_radius_m` is the arithmetic every number above rests on, so it is
/// checked against its own DEFINING property rather than against a second
/// copy of itself: at the radius it returns, the point source's illuminance
/// equals the ambient. `CLAUDE.md`'s naive-rebuild trap is the reason —
/// re-deriving `sqrt(L / 4πA)` in the test would carry any mutant the
/// function carried.
#[test]
fn the_pool_radius_is_where_the_flame_equals_the_night() {
    for (lumens, ambient) in [(600.0f32, 60.0f32), (900.0, 60.0), (12.6, 0.25)] {
        let r = pool_radius_m(lumens, ambient);
        // Illuminance of a point source: candela / d², candela = lm / 4π.
        let at_r = lumens / (4.0 * std::f32::consts::PI) / (r * r);
        assert!(
            (at_r - ambient).abs() < ambient * 1e-3,
            "at the returned {r:.4} m a {lumens} lm source delivers {at_r:.4} \
             lux, not the {ambient} lux it was solved for"
        );
        // Inside is brighter, outside is dimmer. Cheap, and it is what
        // catches a sign or a reciprocal.
        let inside = lumens / (4.0 * std::f32::consts::PI) / (0.5 * r * (0.5 * r));
        assert!(inside > ambient, "the flame is dimmer at half the radius");
    }
    assert_eq!(
        pool_radius_m(600.0, 0.0),
        0.0,
        "a zero ambient returned a radius rather than the stated 0.0 — an \
         infinity here would propagate into a range"
    );
    assert_eq!(pool_radius_m(0.0, 60.0), 0.0, "a dark source has a pool");
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
