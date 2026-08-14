//! What the player is aiming at, and what `E` therefore does.
//!
//! A port of `web/src/interact.js`'s `resolveInteract`/`promptFor`, kept pure
//! and in `ui/` for this module tree's stated reason: a pick is arithmetic,
//! and arithmetic inside a Bevy system is arithmetic no headless test can
//! call. `crates/client/tests/ui.rs` drives it in the **code** tier.
//!
//! ## Two ranks, and the first always beats the second
//!
//! - **aimed** — the candidate is in FRONT of the player (its projection on
//!   the look direction is positive) and lies within [`AIM_RADIUS_M`] of the
//!   aim line. Among these the nearest wins, which is what a raycast would
//!   answer: a hearth a metre in front of a box is the thing *between* you
//!   and the box, not a worse version of it.
//! - **nearby** — everything else in reach; nearest wins.
//!
//! Nothing is ever excluded for being off-aim. Being off-aim only means
//! losing to something that is not. Standing between a hearth and a box, look
//! at the box and the box wins however much closer the hearth is; turn round
//! and it reverses.
//!
//! **The `t > 0` half of the aimed test is load-bearing**, and the browser
//! learned it by probing rather than by reasoning: without it a hearth whose
//! cell centre is exactly under the player's feet sits at distance zero from
//! the aim line and trumps every box in the room.
//!
//! ## Reach is not a client knob
//!
//! [`REACH_M`] is `sim_core::build::BUILD_REACH_M` by import, because the sim
//! gates a door use, a hearth feed, a box open (`deploy.rs`) and a bag loot
//! (`backpack.rs`, which aliases it `LOOT_REACH_M`) on exactly that number.
//! Picking a target outside it costs a round trip and a refusal — the
//! quantize-both-sides law (`CLAUDE.md`) applied to reach.

use protocol::event::WireBag;
use sim_core::build::BUILD_CELL_M;
use sim_core::deploy::{
    box_key, DeployContent, DeployRec, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE,
    ARCH_HEARTH, ARCH_RECYCLER, ARCH_RESEARCH,
};

pub use sim_core::build::BUILD_REACH_M as REACH_M;

/// How far off the aim line a thing may sit and still count as aimed at.
///
/// PROPOSED — `DECISIONS.md` §open ("interact aim radius v0"), carried over
/// from the browser resolver, which is the one number this pick needed that
/// no doc and no Rust constant already fixed. 1.0 m is the deployable's own
/// scale: a box and a hearth are roughly a metre across in a 3 m build cell,
/// so a crosshair within a metre of the centre is a crosshair on the thing.
pub const AIM_RADIUS_M: f32 = 1.0;

/// What `E` would do. `None` never carries a prompt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Verb {
    #[default]
    None,
    Door,
    Bag,
    Box,
    Hearth,
    /// A fire or a furnace — one verb for both, because they are one
    /// thing in the sim (`sim-core/oven.rs`) and the prompt names a KIND.
    Fire,
    /// A recycler. The same sim class as the two above and deliberately
    /// **not** the same verb: the prompt names a KIND, and "FIRE" over a
    /// machine that burns nothing would teach the wrong noun for the
    /// second key. What it shares with `Fire` is the shape of the
    /// interaction — a container with a switch — and that shows up as the
    /// two arms agreeing everywhere the code asks a question about
    /// containers, never as one variant doing both jobs.
    Recycler,
    /// A research table (research v0). The one verb here that acts on what
    /// is in your HAND rather than on what is at the address — the table
    /// holds nothing — so the prompt names the held item and `E` spends it.
    Research,
    /// An authored world container — the haven pad's crate, a waystation's
    /// cache (`sim-core/worldcont.rs`).
    ///
    /// **The one verb on this enum that is not a deployable.** Every other
    /// variant is born from an `ARCH_*` on a record the deploy sync sent;
    /// this one is born from a terrain `Occupant`, which terrain places as
    /// a pure function of the seed and no packet ever mentions. So it is
    /// resolved by the *second* resolver (`resolve_open`, beside
    /// `resolve_swing`) and folded into the pick by the caller — the
    /// deploy scan cannot see it, because there is nothing there to see.
    ///
    /// One variant for both kinds, `Fire`'s argument exactly: a crate and
    /// a cache are one thing in the sim, differing only in the loot table
    /// their occupant selects, and the prompt names a KIND. What the
    /// player is told apart is what is inside.
    Crate,
}

impl Verb {
    /// The tiebreak order, and the only place the browser chain's ordering
    /// survives.
    ///
    /// Two candidates can score exactly equal — two boxes placed symmetrically
    /// about the aim ray is the ordinary case, and the floats compare exactly
    /// because both sides came out of the same arithmetic. The pick still has
    /// to be a function of its inputs alone, or the prompt drawn on one frame
    /// and the verb run on the keypress could differ while the world stood
    /// still. So a dead tie falls back to the order `E` has always used: a
    /// door is aimed at, a bag is stood on, a box is the durable one, and a
    /// hearth is what you meant if none of those is there.
    fn tie(self) -> u8 {
        match self {
            Verb::None => 0,
            Verb::Door => 1,
            Verb::Bag => 2,
            Verb::Box => 3,
            Verb::Hearth => 4,
            // Last, and it costs nothing to say why: an oven stands on
            // the plane like a box, so a dead tie between the two is a
            // box and an oven in one cell, which placement already
            // forbids. The rung exists so the order is total, not because
            // anything can reach it.
            Verb::Fire => 5,
            Verb::Recycler => 6,
            Verb::Research => 7,
            // Last, and unlike `Fire`'s rung this one is genuinely
            // unreachable: a world container is not a deployable, so it
            // cannot tie with one — the two resolvers scan different
            // things and the caller only consults this one when the
            // deploy scan came back `None`. The rung exists so the order
            // stays total.
            Verb::Crate => 8,
        }
    }

    /// The noun the prompt names. Generic kinds only — `CONTENT.md` owns
    /// item names and these four name a KIND of thing.
    pub fn label(self) -> &'static str {
        match self {
            Verb::None => "",
            Verb::Door => "DOOR",
            Verb::Bag => "BACKPACK",
            Verb::Box => "BOX",
            Verb::Hearth => "HEARTH",
            Verb::Fire => "FIRE",
            Verb::Recycler => "RECYCLER",
            Verb::Research => "RESEARCH TABLE",
            Verb::Crate => "CRATE",
        }
    }
}

/// One resolved pick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pick {
    pub verb: Verb,
    /// The archetype of the record this pick resolved, exactly as the
    /// deploy sync named it (`ARCH_BAG` for a bag, which arrives on its
    /// own lane and has no deploy record). Carried so the access verb can
    /// ask `sim_core::deploy::lockable` — the sim's own predicate — rather
    /// than matching verbs, which would be a second copy of the lockable
    /// set waiting to drift (`ui::keypad::lock_target`).
    pub arch: u8,
    /// The container handle: a bag id, or a box's packed `box_key`.
    pub handle: u32,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Door state, straight off the wire — all three bits (lock v1).
    /// `has_lock` is what lets the prompt tell "bare" from "unlocked",
    /// which are the same to `E` and completely different to `L`.
    pub open: bool,
    pub locked: bool,
    pub has_lock: bool,
    /// Fire state, and the one field of a pick the resolver does not
    /// fill: whether a fire is burning lives in `ClientCore`'s own lit
    /// set rather than on the mirrored deploy record (`core.rs` says
    /// why), so the caller stamps it after resolving. False on every
    /// other verb and on a fire nobody has heard about yet, which reads
    /// as "out" — the honest default, since an unheard fire is one this
    /// client has no news of.
    pub lit: bool,
    /// Squared distance from the player, and from the aim line. Diagnostics
    /// for the gate; nothing draws them.
    pub d2: f32,
    pub perp2: f32,
    /// True when the pick was AIMED at rather than merely nearest in reach.
    pub aimed: bool,
}

impl Pick {
    pub fn is_none(&self) -> bool {
        self.verb == Verb::None
    }

    /// What the centre prompt says, or `""` for nothing in reach.
    ///
    /// The key is named in the text because that is the whole job: the
    /// reference genre's centre-screen hint is how a player learns the island
    /// has verbs at all. A door additionally reports the two bits of state the
    /// wire carries — `open` says whether `E` closes or opens it, and `locked`
    /// is stated without claiming the press will fail, because the wire
    /// carries the lock bit but never the owner and only the server knows
    /// whether this door is yours.
    pub fn prompt(&self) -> String {
        match self.verb {
            Verb::None => String::new(),
            Verb::Door => format!(
                "[E] {} DOOR{}",
                if self.open { "CLOSE" } else { "OPEN" },
                // Three states, three sentences (lock v1): a bare door
                // says nothing extra, a bolted-but-open one advertises
                // its keypad, and a shut one says so. Naming `[L]` on
                // the middle case is the only place a player learns the
                // key exists.
                match (self.has_lock, self.locked) {
                    (false, _) => "",
                    (true, false) => "  ·  [L] KEYPAD",
                    (true, true) => "  ·  LOCKED  ·  [L] KEYPAD",
                }
            ),
            // The box borrows the door's whole lock grammar (locks on
            // boxes, `DOORS.md` §9.8): a bare lid says nothing extra, a
            // bolted one advertises its keypad, an armed one says LOCKED
            // too. `E` stays the open — whether it succeeds against a
            // locked lid is the sim's verdict, never this line's.
            Verb::Box => format!(
                "[E] OPEN BOX{}",
                match (self.has_lock, self.locked) {
                    (false, _) => "",
                    (true, false) => "  ·  [L] KEYPAD",
                    (true, true) => "  ·  LOCKED  ·  [L] KEYPAD",
                }
            ),
            // The crew keys ride the hearth's prompt because there is
            // nowhere else a player would look for them, and `L` is
            // already the access key at a door (hearth crew v1).
            Verb::Hearth => "[E] FEED HEARTH  ·  [L] JOIN CREW  ·  [K] LEAVE".to_string(),
            // Two verbs on one thing, and both named: the panel is where
            // the wood goes and `C` is the match. The state is stated the
            // way a door's is, because it is the same question — which
            // way the second key will move it.
            Verb::Fire => format!(
                "[E] OPEN FIRE  ·  [C] {}",
                if self.lit { "PUT OUT" } else { "LIGHT" }
            ),
            // The same two keys and a different pair of words, because a
            // recycler is switched rather than lit — "LIGHT" over a
            // machine with no fire in it is the prompt lying about the
            // mechanism.
            Verb::Recycler => format!(
                "[E] OPEN RECYCLER  ·  [C] {}",
                if self.lit { "STOP" } else { "START" }
            ),
            // Not "OPEN": there is nothing to open, and a prompt that
            // promised a panel would be teaching the wrong gesture. What
            // `E` does here is spend the held item, so the prompt says so.
            Verb::Research => "[E] RESEARCH HELD ITEM".to_string(),
            v => format!("[E] OPEN {}", v.label()),
        }
    }
}

/// Where the player is and where they are looking, in world XZ.
#[derive(Clone, Copy, Debug)]
pub struct Aim {
    pub x: f32,
    pub z: f32,
    pub fx: f32,
    pub fz: f32,
    pub reach: f32,
    pub radius: f32,
}

impl Aim {
    /// The ordinary aim: feet position, facing, and both defaults.
    pub fn new(x: f32, z: f32, fx: f32, fz: f32) -> Self {
        Self {
            x,
            z,
            fx,
            fz,
            reach: REACH_M,
            radius: AIM_RADIUS_M,
        }
    }
}

/// Best-so-far, as scalars rather than a candidate struct, so the sweep
/// allocates nothing.
struct Best {
    aimed: bool,
    d2: f32,
    tie: u8,
}

impl Best {
    /// Score one candidate at world XZ; answer whether it takes the lead,
    /// filling the shared fields of `out` if it does.
    fn wins(
        &mut self,
        out: &mut Pick,
        aim: &Aim,
        f: (f32, f32),
        verb: Verb,
        x: f32,
        z: f32,
    ) -> bool {
        let (dx, dz) = (x - aim.x, z - aim.z);
        let d2 = dx * dx + dz * dz;
        if d2 > aim.reach * aim.reach {
            return false; // out of the server's reach
        }
        // Projection onto the look direction. Positive means in front; the
        // perpendicular offset is only meaningful there, which is why `aimed`
        // requires it.
        let t = dx * f.0 + dz * f.1;
        let (px, pz) = (dx - t * f.0, dz - t * f.1);
        let perp2 = px * px + pz * pz;
        let aimed = t > 0.0 && perp2 <= aim.radius * aim.radius;
        let tie = verb.tie();
        // Rank first, then distance inside the rank, then the old order.
        if self.aimed && !aimed {
            return false;
        }
        if aimed == self.aimed {
            if d2 > self.d2 {
                return false;
            }
            if d2 == self.d2 && tie >= self.tie {
                return false;
            }
        }
        self.aimed = aimed;
        self.d2 = d2;
        self.tie = tie;
        out.verb = verb;
        out.d2 = d2;
        out.perp2 = perp2;
        out.aimed = aimed;
        true
    }
}

/// Resolve what `E` is pointed at.
///
/// `have` is `ClientCore::deploy_defs_have`: a row past it has not dripped in
/// and its archetype is unknown, so it is skipped rather than read as
/// `DeployDef::INERT`'s bag — offering a verb on a guess is how a prompt ends
/// up naming a thing that is not there.
pub fn resolve(
    aim: Aim,
    deploys: &[DeployRec],
    defs: &DeployContent,
    have: u16,
    bags: &[WireBag],
) -> Pick {
    let mut out = Pick::default();
    let mut best = Best {
        aimed: false,
        d2: f32::INFINITY,
        tie: u8::MAX,
    };
    // Normalise once. A zero-length look direction is not a reason to refuse
    // to answer — the segment collapses to the player's own position and the
    // metric degrades to plain nearest-wins.
    let flen = (aim.fx * aim.fx + aim.fz * aim.fz).sqrt();
    let f = if flen > 0.0 {
        (aim.fx / flen, aim.fz / flen)
    } else {
        (0.0, 0.0)
    };
    let half = BUILD_CELL_M * 0.5;

    for rec in deploys {
        if (rec.row as u16) >= have {
            continue;
        }
        let arch = defs.defs[rec.row as usize].arch;
        let verb = match arch {
            ARCH_DOOR => Verb::Door,
            ARCH_BOX => Verb::Box,
            ARCH_HEARTH => Verb::Hearth,
            ARCH_FIRE | ARCH_FURNACE => Verb::Fire,
            ARCH_RECYCLER => Verb::Recycler,
            ARCH_RESEARCH => Verb::Research,
            _ => continue,
        };
        // A box is addressed by its packed cell. `box_key(0, 0, 0)` is 0 and
        // 0 is the reserved "no container" handle, which is why the SIM
        // refuses to place a box there (`deploy.rs`). A record carrying it is
        // one this client should not have, so it is not offered at all —
        // better than a prompt for a box the key would decline to open.
        let mut handle = 0u32;
        if verb == Verb::Box || verb == Verb::Fire || verb == Verb::Recycler {
            handle = box_key(rec.cx, rec.cz, rec.level);
            if handle == 0 {
                continue;
            }
        }
        // Reach is measured to the cell CENTRE for all three, which is the
        // metric `deploy.rs`'s `box_in_reach` and the door's own gate use.
        let x = rec.cx as f32 * BUILD_CELL_M + half;
        let z = rec.cz as f32 * BUILD_CELL_M + half;
        if !best.wins(&mut out, &aim, f, verb, x, z) {
            continue;
        }
        out.arch = arch;
        out.handle = handle;
        out.cx = rec.cx;
        out.cz = rec.cz;
        out.level = rec.level;
        out.loc = rec.loc;
        out.open = rec.open;
        out.locked = rec.locked;
        out.has_lock = rec.has_lock;
    }

    for bag in bags {
        // A bag carries a world position, not a grid cell — it is dropped
        // where its owner died.
        let x = bag.qx as f32 * sim_core::movement::POS_XZ_Q;
        let z = bag.qz as f32 * sim_core::movement::POS_XZ_Q;
        if !best.wins(&mut out, &aim, f, Verb::Bag, x, z) {
            continue;
        }
        out.arch = ARCH_BAG;
        out.handle = bag.id;
        out.cx = 0;
        out.cz = 0;
        out.level = 0;
        out.loc = 0;
        out.open = false;
        out.locked = false;
        out.has_lock = false;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::deploy::{DeployDef, ARCH_BAG};

    fn defs_with(arches: &[u8]) -> (DeployContent, u16) {
        let mut d = DeployContent::EMPTY;
        for (i, &arch) in arches.iter().enumerate() {
            d.defs[i] = DeployDef {
                arch,
                ..DeployDef::INERT
            };
        }
        d.def_count = arches.len() as u16;
        (d, arches.len() as u16)
    }

    fn rec(cx: u16, cz: u16, row: u8) -> DeployRec {
        DeployRec {
            cx,
            cz,
            level: 0,
            loc: 0,
            row,
            owner: 1,
            hp: 100,
            uh: 0,
            open: false,
            locked: false,
            has_lock: false,
        }
    }

    /// Cell centre of (cx, cz), so a test can stand exactly somewhere.
    fn centre(cx: u16, cz: u16) -> (f32, f32) {
        (
            cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        )
    }

    #[test]
    fn nothing_in_reach_is_no_verb_and_no_prompt() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        let far = [rec(100, 100, 0)];
        let p = resolve(Aim::new(0.0, 0.0, 0.0, 1.0), &far, &defs, have, &[]);
        assert!(p.is_none());
        assert_eq!(p.prompt(), "");
    }

    /// The judge's case, and the whole reason for two ranks: standing between
    /// a hearth and a box, whichever you LOOK at wins.
    #[test]
    fn aim_beats_proximity() {
        let (defs, have) = defs_with(&[ARCH_HEARTH, ARCH_BOX]);
        // Hearth one cell north, box two cells south. Stand between them.
        let recs = [rec(0, 1, 0), rec(0, 3, 1)];
        let (x, _) = centre(0, 2);
        let (_, hz) = centre(0, 1);
        let (_, bz) = centre(0, 3);
        let me = (x, (hz + bz) * 0.5);

        let north = resolve(Aim::new(me.0, me.1, 0.0, -1.0), &recs, &defs, have, &[]);
        assert_eq!(north.verb, Verb::Hearth, "looking north");
        let south = resolve(Aim::new(me.0, me.1, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(south.verb, Verb::Box, "looking south");
    }

    /// A thing behind you with nothing else around still resolves — off-aim
    /// only ever means losing to something that is not off-aim.
    #[test]
    fn a_lone_target_behind_you_still_resolves() {
        let (defs, have) = defs_with(&[ARCH_DOOR]);
        let recs = [rec(0, 0, 0)];
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz + 2.0, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(p.verb, Verb::Door);
        assert!(!p.aimed, "it is behind us, so it is nearby and not aimed");
    }

    /// The `t > 0` guard: a hearth underfoot must not trump a box you are
    /// looking straight at.
    #[test]
    fn a_target_underfoot_does_not_trump_the_one_you_face() {
        let (defs, have) = defs_with(&[ARCH_HEARTH, ARCH_BOX]);
        let recs = [rec(0, 0, 0), rec(0, 1, 1)];
        let (cx, cz) = centre(0, 0);
        // Standing exactly on the hearth's centre, facing the box.
        let p = resolve(Aim::new(cx, cz, 0.0, 1.0), &recs, &defs, have, &[]);
        assert_eq!(p.verb, Verb::Box);
        assert!(p.aimed);
    }

    #[test]
    fn a_bag_resolves_by_world_position() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let bag = WireBag {
            id: 77,
            qx: (2.0 / sim_core::movement::POS_XZ_Q) as i32,
            qy: 0,
            qz: 0,
        };
        let p = resolve(Aim::new(0.0, 0.0, 1.0, 0.0), &[], &defs, have, &[bag]);
        assert_eq!(p.verb, Verb::Bag);
        assert_eq!(p.handle, 77);
        assert!(p.aimed);
    }

    /// Out past `BUILD_REACH_M` is the server's refusal, so the client does
    /// not offer it.
    #[test]
    fn reach_is_the_sims_reach() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let just_past = REACH_M + 0.5;
        let bag = WireBag {
            id: 1,
            qx: (just_past / sim_core::movement::POS_XZ_Q) as i32,
            qy: 0,
            qz: 0,
        };
        let p = resolve(Aim::new(0.0, 0.0, 1.0, 0.0), &[], &defs, have, &[bag]);
        assert!(p.is_none());
    }

    /// A row the def table has not dripped yet is not a bag — it is unknown,
    /// and offering a verb on `INERT` would name a thing that is not there.
    #[test]
    fn an_undripped_row_offers_nothing() {
        let (defs, _) = defs_with(&[ARCH_BOX]);
        let recs = [rec(0, 0, 0)];
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz - 2.0, 0.0, 1.0), &recs, &defs, 0, &[]);
        assert!(p.is_none());
    }

    /// `box_key(0, 0, 0)` is the reserved no-container handle and the sim
    /// refuses to place there. The pick must not offer one either.
    #[test]
    fn the_reserved_box_handle_is_never_offered() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        let recs = [rec(0, 0, 0)];
        assert_eq!(box_key(0, 0, 0), 0);
        let (cx, cz) = centre(0, 0);
        let p = resolve(Aim::new(cx, cz - 1.0, 0.0, 1.0), &recs, &defs, have, &[]);
        assert!(p.is_none());
    }

    #[test]
    fn a_door_prompt_reports_the_state_the_wire_carries() {
        let mut p = Pick {
            verb: Verb::Door,
            ..Pick::default()
        };
        // Bare: nothing but the verb. A door nobody has secured has no
        // keypad to name and is not locked (lock v1).
        assert_eq!(p.prompt(), "[E] OPEN DOOR");
        p.open = true;
        assert_eq!(p.prompt(), "[E] CLOSE DOOR");
        // Bolted but not armed: the keypad exists, the door is not shut.
        // This is the state the two-bit prompt could not say, and the
        // reason `has_lock` is on the wire at all.
        p.has_lock = true;
        assert!(p.prompt().contains("[L] KEYPAD"), "{}", p.prompt());
        assert!(
            !p.prompt().contains("LOCKED"),
            "an unarmed lock is not a locked door: {}",
            p.prompt()
        );
        // Armed: both.
        p.locked = true;
        assert!(p.prompt().contains("LOCKED"), "{}", p.prompt());
        assert!(p.prompt().contains("[L] KEYPAD"), "{}", p.prompt());
    }

    #[test]
    fn every_verb_has_a_prompt() {
        for verb in [Verb::Door, Verb::Bag, Verb::Box, Verb::Hearth] {
            let p = Pick {
                verb,
                ..Pick::default()
            };
            assert!(!p.prompt().is_empty(), "{verb:?} has no prompt");
            assert!(!verb.label().is_empty(), "{verb:?} has no label");
        }
    }

    /// A pick must be a function of its inputs alone — the prompt drawn on
    /// one frame and the verb run on the keypress cannot differ while the
    /// world stands still.
    #[test]
    fn the_pick_is_deterministic_under_a_tie() {
        let (defs, have) = defs_with(&[ARCH_BOX]);
        // Two boxes placed symmetrically about the aim ray.
        let recs = [rec(0, 2, 0), rec(2, 2, 0)];
        let (x0, _) = centre(0, 2);
        let (x1, _) = centre(2, 2);
        let (_, z) = centre(0, 0);
        let aim = Aim::new((x0 + x1) * 0.5, z, 0.0, 1.0);
        let first = resolve(aim, &recs, &defs, have, &[]);
        for _ in 0..8 {
            let again = resolve(aim, &recs, &defs, have, &[]);
            assert_eq!(first.handle, again.handle);
        }
    }
}

// ---------------------------------------------------------------------------
// The second resolver: what a SWING would hit.
//
// `E` above answers for deployables and bags. The left button answers for the
// scatter — a tree, an ore node, a bush, a barrel — and until this landed the
// crosshair named what `E` would do and never what a swing would hit, so the
// most common verb in the game was the one with no prompt.
//
// **Ported from `sim_core::gather::swing`, not from the browser.** The deleted
// `web/src/interact.js` had a `resolveSwing`, but it was itself a hand-mirror
// of the sim's scan, and mirroring a mirror is how the two drift. Every bound
// below is read from `gather` rather than restated: `REACH_M`, `CONE_COS`,
// `DY_MAX_M`, `POINT_BLANK_M2`. Nothing here invents a number.
//
// **One difference from the browser, and it is a simplification the native
// client earned.** `resolveSwing` skipped a cell that was `hidden || fellAt`,
// because three.js animated the fall over `FELL_TICKS + FELL_SINK_TICKS` (93
// ticks, 3.1 s) and a tree the sim had already taken still stood on screen —
// so reading `hidden` alone offered "CHOP TREE" over a stump-to-be and the
// swing whiffed. This client has no such window: `render::props::apply_fell`
// is driven directly off `harvested(key)` and swaps the mesh on the event, so
// the harvested set is the whole truth and there is no second state to test.

use sim_core::backpack::LOOT_REACH_M as OPEN_REACH_M;
use sim_core::fmath::{fabs, floor_i32};
use sim_core::gather::{CONE_COS, DY_MAX_M, POINT_BLANK_M2, REACH_M as SWING_REACH_M};
use sim_core::occupy::{Harvested, SlotCache};
// `scatter` itself is deliberately NOT imported: every read on this path goes
// through `Island::slot`, and `tests/ui.rs`'s call-site gate says so.
use sim_core::terrain::{Haven, Occupant, ScatterTable, Slot, CELL_SIZE};

/// What a swing would land on, or `Occupant::None` for a whiff.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwingPick {
    /// The terrain occupant ordinal — the same value `EV_SLOT_HARVESTED`
    /// names in field `b`, so the prompt and the event speak one vocabulary.
    pub occupant: u8,
    pub cx: u16,
    pub cz: u16,
    /// Planar squared distance. Diagnostics for the gate; nothing draws it.
    pub d2: f32,
}

/// Whether an occupant is something a swing takes.
///
/// `Rock` (6) is deliberately absent and so is the stump: a rock is scenery
/// the sim never harvests, and a stump is the CONSEQUENCE of a harvest rather
/// than a target. `gather::target_index` makes the same split one crate down —
/// nodes 1..=5 plus the barrel at 7 — and this is that predicate, not a
/// second opinion about it.
fn swingable(o: Occupant) -> bool {
    matches!(
        o,
        Occupant::Tree
            | Occupant::StoneNode
            | Occupant::MetalNode
            | Occupant::SulfurNode
            | Occupant::Bush
            | Occupant::BarrelSlot
    )
}

/// The noun the prompt names for a swing pick, or `""` for a whiff.
///
/// `[LMB]` rather than a verb name is the caller's job — these are the labels
/// only, and they name a KIND of thing (`CONTENT.md` owns item names).
pub fn swing_label(occupant: u8) -> &'static str {
    match occupant {
        o if o == Occupant::Tree as u8 => "CHOP TREE",
        o if o == Occupant::StoneNode as u8 => "MINE STONE",
        o if o == Occupant::MetalNode as u8 => "MINE METAL",
        o if o == Occupant::SulfurNode as u8 => "MINE SULFUR",
        o if o == Occupant::Bush as u8 => "PICK BUSH",
        o if o == Occupant::BarrelSlot as u8 => "SMASH BARREL",
        _ => "",
    }
}

/// Where the swing is taken from: feet position, eye height and facing.
///
/// A struct rather than five scalars because the five are one fact — and
/// because `resolve_swing` reached clippy's argument ceiling honestly, which
/// is usually the lint telling you a parameter list has become a bag.
#[derive(Clone, Copy, Debug)]
pub struct SwingAim {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub fx: f32,
    pub fz: f32,
}

/// The island the swing lands on, borrowed rather than copied.
///
/// `harvested` is a trait object for the reason `render::props::apply_fell`
/// takes `&dyn Fn`: the caller's harvest state is `ClientCore`'s in the game
/// and a fixture in the gate, and neither should force a generic through
/// every signature between here and the HUD.
///
/// This is `sim_core::occupy::Occupants` plus the seed, deliberately — the
/// sim bundles the identical four borrows for the identical scan, and
/// `ClientCore::island` hands both halves over in one call so the client's
/// crosshair and the client's predictor share one set of cache lines.
pub struct Island<'a> {
    pub seed: u64,
    pub table: &'a ScatterTable,
    pub haven: &'a Haven,
    pub harvested: &'a dyn Harvested,
    /// **The memo, and the only door to `terrain::scatter` on this path.**
    /// A cold `scatter` costs ~60 `noise2` taps, a 3×3 window nine of them,
    /// and the crosshair resolves that window every frame — so the resolver
    /// reads through here and `tests/ui.rs`'s call-site gate refuses a
    /// direct call that would walk past it.
    pub cache: &'a mut SlotCache,
}

impl Island<'_> {
    /// The slot in cell (`cx`, `cz`), memoized.
    ///
    /// Identical bits to `terrain::scatter(seed, table, haven, cx, cz)` — the
    /// cache is a pure function's memo, so a hit and a miss return the same
    /// answer and eviction can only change how long it took
    /// (`occupy::SlotCache`'s own argument). `tests/ui.rs` proves that
    /// equality over a walk of the island rather than asserting it here.
    pub fn slot(&mut self, cx: i32, cz: i32) -> Slot {
        self.cache.slot(self.seed, self.table, self.haven, cx, cz)
    }
}

/// The nearest standing swingable slot in reach and inside the aim cone.
///
/// The 3×3 cell window around the player's own cell, walked **dz outer, dx
/// inner** — the sim's own order, and the tiebreak depends on it. Two slots
/// at exactly equal `d2` is not a contrived case (the scatter places on a
/// grid), and `d2 < best` is strict so the FIRST cell in that order keeps it,
/// which is what `gather::swing`'s `is_none_or(|b| d2 < b.d2)` does.
///
/// Allocates nothing and is **idempotent**: the caller may run it every frame
/// for the prompt and again on the click without the two disagreeing. It took
/// no `&mut` at all until the memo landed, and the weaker word is the honest
/// one — `island.cache` is written, but only with answers `terrain::scatter`
/// would have recomputed, so nothing observable moves.
pub fn resolve_swing(at: SwingAim, island: &mut Island<'_>) -> SwingPick {
    scan_slots(at, island, swingable, SWING_REACH_M)
}

/// Whether an occupant is something `E` OPENS.
///
/// The mirror of `swingable`, and its complement rather than a subset:
/// nothing on the island is both smashed and opened. A barrel is hit until
/// it bursts and pays into a bag on the ground; a crate is furniture with
/// a lid. That split is the sim's — `gather::target_index` claims the
/// barrel and `worldcont::table_of` claims these two — and this is those
/// two predicates read from the client side, never a third opinion.
fn openable(o: Occupant) -> bool {
    matches!(o, Occupant::CrateSlot | Occupant::CacheSlot)
}

/// What `E` would OPEN in the scatter, or `Occupant::None` for nothing.
///
/// Same scan, same cone, same 3×3 memo as `resolve_swing` — a different
/// predicate and a different reach, because an open is priced at the arm
/// every other container uses (`backpack::LOOT_REACH_M`) and a swing at
/// the longer `gather::REACH_M`. Sharing the body rather than copying it
/// is the point: the two prompts must never disagree about which cell the
/// player is standing at, and two hand-written 3×3 walks would eventually
/// do exactly that.
pub fn resolve_open(at: SwingAim, island: &mut Island<'_>) -> SwingPick {
    scan_slots(at, island, openable, OPEN_REACH_M)
}

/// The 3×3 scatter walk both slot resolvers run: nearest cell whose
/// occupant satisfies `want`, inside `reach`, inside the aim cone or
/// point-blank, not already harvested.
fn scan_slots(
    at: SwingAim,
    island: &mut Island<'_>,
    want: fn(Occupant) -> bool,
    reach: f32,
) -> SwingPick {
    let SwingAim { x, y, z, fx, fz } = at;
    let mut out = SwingPick::default();
    let mut best = f32::INFINITY;

    // A zero-length look is not a refusal to answer: the cone test degrades
    // to point-blank-only, which is the same posture `resolve` takes above.
    let flen = (fx * fx + fz * fz).sqrt();
    let (f0, f1) = if flen > 0.0 {
        (fx / flen, fz / flen)
    } else {
        (0.0, 0.0)
    };

    let pcx = floor_i32(x / CELL_SIZE);
    let pcz = floor_i32(z / CELL_SIZE);

    let mut dz_cell = -1i32;
    while dz_cell <= 1 {
        let mut dx_cell = -1i32;
        while dx_cell <= 1 {
            let cx = pcx + dx_cell;
            let cz = pcz + dz_cell;
            dx_cell += 1;
            let s = island.slot(cx, cz);
            if !want(s.occupant) {
                continue;
            }
            let dx = s.x - x;
            let dy = s.y - y;
            let dz = s.z - z;
            let d2 = dx * dx + dz * dz;
            if d2 > reach * reach {
                continue;
            }
            if fabs(dy) > DY_MAX_M {
                continue;
            }
            let aimed = d2 <= POINT_BLANK_M2 || {
                let dot = dx * f0 + dz * f1;
                dot > CONE_COS * d2.sqrt()
            };
            if !aimed || d2 >= best {
                continue;
            }
            // Negative cells cannot be harvested — the set is keyed on u16,
            // and the sim's own scan casts the same way.
            if cx < 0 || cz < 0 || island.harvested.is_harvested(cx as u16, cz as u16) {
                continue;
            }
            best = d2;
            out = SwingPick {
                occupant: s.occupant as u8,
                cx: cx as u16,
                cz: cz as u16,
                d2,
            };
        }
        dz_cell += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// The weak spot.
//
// `EV_WEAK_MARK` has been decoded into `ClientCore::{mark_cell, mark8,
// mark_weak_hit}` for a long time and read by nothing, so the one mechanic in
// this game that rewards *where you stand* was invisible: the server was
// announcing a bearing every hit and the player had no way to learn it existed
// (`NOW.md` §0x item 6 — the reference's own `OnDispenserBonus`).
//
// **This mirrors `gather::swing`'s sector test and does not restate it.**
// `WEAK_COS` is imported, the bearing goes through the same 256-entry yaw LUT
// the sim uses, and the point-blank exemption is the sim's own — a hit inside
// `POINT_BLANK_M2` has no bearing to judge, so it never bonuses and the HUD
// must not promise that it will. The client cannot *decide* a weak hit (the
// server does, and says so in `mark_weak_hit`); what it can do is tell the
// player whether they are standing where the next one would land, which is
// the whole skill.

use sim_core::gather::{cell_key, NO_CELL, WEAK_COS};
use sim_core::yaw_dir;

/// Whether the player at `(px, pz)` stands in the announced weak sector of
/// the node at `(nx, nz)`.
///
/// `mark8` is the sector's heading over the sim's 256-entry LUT. The offset
/// is node→player, matching `gather::swing`'s `ox`/`oz`, so a mark of 0
/// (north) means *stand north of the node*.
pub fn in_weak_sector(px: f32, pz: f32, nx: f32, nz: f32, mark8: u8) -> bool {
    let ox = px - nx;
    let oz = pz - nz;
    let d2 = ox * ox + oz * oz;
    // Point blank has no bearing to judge. The sim exempts it, so this must
    // too — a prompt that lit up while standing inside the trunk would
    // promise a bonus the server refuses.
    if d2 <= POINT_BLANK_M2 {
        return false;
    }
    let (wx, wz) = yaw_dir((mark8 as u16) << 8);
    ox * wx + oz * wz > WEAK_COS * d2.sqrt()
}

/// Whether the announced mark belongs to the cell this swing would hit.
///
/// The chase is per-node and the server restarts it when the player switches
/// targets, so a mark for the tree behind you says nothing about the one in
/// front. `NO_CELL` is "no chase in progress" and is the state between the
/// first swing at a fresh node and the hit that announces its mark.
pub fn mark_is_for(mark_cell: u32, pick: &SwingPick) -> bool {
    mark_cell != NO_CELL && pick.occupant != 0 && mark_cell == cell_key(pick.cx, pick.cz)
}
