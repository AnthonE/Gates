//! What the fx stage can stage, by name (`examples/fx_stage.rs`, `ci/fx.sh`,
//! the `fx` skill): every blow the table knows, `<weapon>-<matter>`, and the
//! blast. Each fires through [`Fx::impact`] from a real [`Contact`], so the
//! stage shows exactly what a blow in the game throws.

use bevy::prelude::*;

use super::Fx;
use crate::render::impact::{Contact, Matter, Weapon};

/// One effect the stage can fire.
#[derive(Clone, Debug)]
pub struct Staged {
    pub name: String,
    /// What it is, for a person reading the list.
    pub what: String,
    /// Roughly how far it reaches from where it fires, metres: the stage
    /// frames it by this (`fx::stage::Framing`).
    pub radius: f32,
    /// How long it lasts, seconds: the strip's frames span this.
    pub span_s: f32,
    pub weapon: Weapon,
    pub matter: Matter,
}

pub const WEAPONS: [(Weapon, &str); 3] = [
    (Weapon::Melee, "melee"),
    (Weapon::Arrow, "arrow"),
    (Weapon::Bullet, "bullet"),
];

pub const MATTERS: [(Matter, &str); 9] = [
    (Matter::Wood, "wood"),
    (Matter::Stone, "stone"),
    (Matter::Metal, "metal"),
    (Matter::Flesh, "flesh"),
    (Matter::Plant, "plant"),
    (Matter::Dirt, "dirt"),
    (Matter::Sand, "sand"),
    (Matter::Grass, "grass"),
    (Matter::Water, "water"),
];

/// Every effect the stage can fire, in a stable order.
pub fn list() -> Vec<Staged> {
    let mut out = Vec::new();
    for (w, wn) in WEAPONS {
        for (m, mn) in MATTERS {
            // How long the longest layer of it lives: dust and leaves hang,
            // sparks and blood are gone in half a second.
            let span_s = match m {
                Matter::Plant => 1.6,
                Matter::Flesh => 0.8,
                Matter::Water => 1.0,
                _ => 1.4,
            };
            out.push(Staged {
                name: format!("{wn}-{mn}"),
                what: format!("{wn} on {mn}"),
                radius: 0.35,
                span_s,
                weapon: w,
                matter: m,
            });
        }
    }
    out.push(Staged {
        name: "blast".into(),
        what: "a satchel going off: flash, fireball, smoke, dust, sparks".into(),
        radius: 2.2,
        span_s: 3.0,
        weapon: Weapon::Blast,
        matter: Matter::Stone,
    });
    out
}

/// The staged effect called `name`.
pub fn find(name: &str) -> Option<Staged> {
    list().into_iter().find(|s| s.name == name)
}

/// Fire `s` at `at`, thrown along `dir` (out of the surface it struck), seen
/// from `eye`. Returns the lit chips the blow would throw with it (the stage
/// draws particles only).
pub fn fire(fx: &mut Fx, s: &Staged, at: Vec3, dir: Vec3, eye: Vec3) -> usize {
    let c = Contact {
        at,
        away: dir,
        normal: dir,
        matter: s.matter,
        weapon: s.weapon,
        ..Contact::default()
    };
    fx.impact(&c, eye)
}
