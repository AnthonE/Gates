//! Gate: the animal the client DRAWS is the animal the sim lets you hit.
//!
//! Melee aim v1 (2026-09-05) made a swing a ray, so an animal needs a volume
//! — `MobDef::{body_r_cm, body_h_cm}`, off `content/mobs.toml`. That volume is
//! authored from what this client draws (`render::mobs`: a pig is 1.5 m long
//! and 0.78 m high), and the two live in different crates with a TOML file
//! between them, which is exactly the shape `CLAUDE.md` warns about twice: a
//! hand-kept mirror of another crate's surface goes stale. So read both and
//! compare, rather than trusting the comment in either.
//!
//! **What is pinned and what is not.** The HEIGHT is an equality: the sim's
//! cylinder must be the drawn animal's shoulder height, because that is the
//! number that decides whether a level swing from a 1.6 m eye passes over the
//! back of a pig, and a client that drew one height while the server hit
//! another would be a hunter aiming at a picture. The RADIUS is a band: a
//! cylinder is round and a body is long, so `body_r_cm` covers the drawn
//! width with room and stays inside the drawn length — a radius past the
//! nose is an animal you hit by aiming at the air beside it.
//!
//! Reads the shipped `content/` through the real loader and the real bake,
//! for `viewmodel_arms.rs`'s reason: a number typed into a gate is a second
//! copy of the thing under test.

use client::render::mobs::{PIG_H_M, PIG_LEN_M, WOLF_H_M, WOLF_LEN_M};
use sim_core::mob::{MOB_PIG, MOB_WOLF};

/// The baked roster, off `content/`.
fn mobs() -> sim_core::mob::MobContent {
    let content = content::Content::load_dir(std::path::Path::new("../../content"))
        .unwrap_or_else(|e| panic!("content/ does not load: {e}"));
    content
        .bake_mobs()
        .unwrap_or_else(|e| panic!("bake_mobs: {e}"))
}

/// A drawn body's width — the massing is longer than it is wide, and the
/// client draws both off one pair of constants, so the width the cylinder
/// has to cover is the smaller dimension in plan. Kept here rather than
/// exported from `render::mobs` because it is this gate's reading of the
/// massing, not a number the renderer uses.
const PLAN_ASPECT: f32 = 0.45;

#[test]
fn the_hit_cylinder_is_the_animal_the_client_draws() {
    let mc = mobs();
    for (kind, name, h_m, len_m) in [
        (MOB_PIG, "pig", PIG_H_M, PIG_LEN_M),
        (MOB_WOLF, "wolf", WOLF_H_M, WOLF_LEN_M),
    ] {
        let def = mc.def(kind);
        let h = f32::from(def.body_h_cm) * 0.01;
        let r = f32::from(def.body_r_cm) * 0.01;
        // Height: an equality, at the centimetre the content file can say.
        assert!(
            (h - h_m).abs() < 0.005,
            "{name}: content stands it {h:.2} m tall and the client draws it \
             {h_m:.2} m — a level swing decides on this number, so they cannot \
             differ (content/mobs.toml `body_h_cm`, render/mobs.rs)"
        );
        // Radius: covers the drawn width, stays inside the drawn length.
        let width = len_m * PLAN_ASPECT;
        assert!(
            r >= width * 0.5,
            "{name}: a {r:.2} m radius does not cover a {width:.2} m body — \
             a swing at its flank would pass through it"
        );
        assert!(
            r <= len_m * 0.5,
            "{name}: a {r:.2} m radius reaches past the {len_m:.2} m body — \
             a swing at the air beside it would land"
        );
        println!("{name}: drawn {len_m:.2} x {h_m:.2} m, hit cylinder r {r:.2} h {h:.2}");
    }
}

/// **Both species are hittable at all**, which is the failure a zeroed row
/// would make invisible: `MobDef::EMPTY` carries zeros, and a bake that
/// dropped the two columns would leave every animal a ghost the ray passes
/// straight through with every other gate green.
#[test]
fn no_shipped_species_is_a_ghost() {
    let mc = mobs();
    for (kind, name) in [(MOB_PIG, "pig"), (MOB_WOLF, "wolf")] {
        let def = mc.def(kind);
        assert!(
            def.body_r_cm > 0 && def.body_h_cm > 0,
            "{name} has no hit volume ({} x {} cm) — nothing can be swung at it",
            def.body_r_cm,
            def.body_h_cm
        );
    }
}
