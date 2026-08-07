//! Timed satchel charges — the raid verb (`DESIGN.md` §2, `CONTENT.md` §3).
//!
//! Everything defensive this sim has grown — `build::repair`,
//! `deploy::decay_of`, upkeep every `UPKEEP_PERIOD_TICKS`, the four
//! upgrade tiers, door locks, ownership — defends against a breach. Until
//! this module the only breach was a hatchet at hatchet rates, so
//! `content/balance.toml`'s anchor 1 (`raid_ratio_stone_pct`, "satchels to
//! break a stone wall in farm-minutes") divided by a number no player
//! could spend. This is the verb that spends it.
//!
//! **The shape, and why.** A charge is *planted at an address*, not thrown
//! along an arc. The address is the same `(deploy, cx, cz, level, loc)`
//! tuple `build::repair` takes, and it means the same thing — the bit
//! picks the store, because a door and the doorway it stands in share one
//! address. A ballistic throw needs M2's rewound raycasts to say what it
//! hit; planting needs nothing the sim does not already have, and it is
//! the *placement* that the raid ratio prices. Where the charge lands is
//! not what makes a raid a decision; the fuse is.
//!
//! **The fuse is the whole mechanic.** The seconds between planting and
//! the blast are what make a raid a commitment rather than a click: you
//! are standing at the wall you are breaking, holding nothing, and the
//! defender gets exactly the same seconds you do. It comes from content
//! (`fuse_s`, baked to ticks) and never from a literal here.
//!
//! **What v0 deliberately does not do**, registered in `DECISIONS.md`
//! §open ("satchel fuse v0"): no blast radius — a charge damages the one
//! address it was planted on and nothing around it, so a raider pays per
//! wall and the anchor's arithmetic is exactly `piece hp ÷ structure`; no
//! damage to players standing in it; no dud chance; no defusing. Each of
//! those is a separate verb and each would want its own knob.

use crate::build::{
    anchor, BuildContent, Pieces, BUILD_REACH_M, REFUSE_B_COST, REFUSE_B_FULL, REFUSE_B_PIECE,
    REFUSE_B_REACH, REFUSE_B_SPOT,
};
use crate::combat::held_item;
use crate::craft::{inv_count, inv_take};
use crate::deploy::{damage_deploy, damage_piece, DeployContent, Deploys};
use crate::limits::MAX_LIVE_CHARGES;
use crate::world::{EventQueue, Player, EV_BUILD_REFUSED, EV_CHARGE_PLACED, STRUCT_DEPLOY_BIT};

/// One planted charge, burning.
///
/// `structure` is copied off the throwable's row **at plant time** rather
/// than read again when the fuse runs out, and that is deliberate: what a
/// charge takes off a wall is decided by what was planted, not by what the
/// raider happens to be holding ten seconds later. The alternative also
/// breaks a rule — the content table is construction input, and reading it
/// at detonation would make a live charge's damage depend on a table swap
/// mid-session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChargeRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Which store the address names — `build::repair`'s bit, for
    /// `build::repair`'s reason.
    pub deploy: bool,
    /// Damage the blast takes off its target.
    pub structure: u16,
    /// The tick the fuse runs out. Absolute, never a countdown that is
    /// decremented: a decrement is state that a dropped tick can corrupt,
    /// and an absolute deadline against `world.tick` cannot drift.
    pub fires_at: u64,
    /// Who planted it. Not used to refuse anything — see `place` — but it
    /// is on the event so a client can tell its own charge from a
    /// stranger's, and a kill-credit rule later has somewhere to read.
    pub owner: u32,
}

/// The live charge store — sim state, hashed. Dense and insertion-ordered,
/// rewritten by swap-remove, exactly like `Pieces` and the box list; the
/// order is deterministic because every insert and every removal is a
/// command's or a tick's consequence, replayed in the same order.
#[derive(Clone, Debug)]
pub struct Charges {
    entries: [ChargeRec; MAX_LIVE_CHARGES],
    len: usize,
}

impl Default for Charges {
    fn default() -> Self {
        Self::new()
    }
}

impl Charges {
    pub const fn new() -> Self {
        Self {
            entries: [ChargeRec {
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
                deploy: false,
                structure: 0,
                fires_at: 0,
                owner: 0,
            }; MAX_LIVE_CHARGES],
            len: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn entries(&self) -> &[ChargeRec] {
        &self.entries[..self.len]
    }

    /// Replace the store from a decoded world save. Boot-only
    /// (`worldsave.rs`).
    ///
    /// A fuse is restored with its **absolute** `fires_at`, not a
    /// remainder, because the world resumes at the tick it was saved on.
    /// That is the whole reason the tick counter is persisted: every
    /// deadline in this world — a fuse, a bag's despawn, a tree's respawn,
    /// a bag's cooldown — is an absolute tick, and rebasing them all
    /// against zero would be four chances to get the arithmetic wrong for
    /// no gain.
    pub(crate) fn restore(&mut self, recs: &[ChargeRec]) {
        self.len = recs.len().min(MAX_LIVE_CHARGES);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
    }

    /// Push one charge, or refuse when the store is full (wall 4 — the
    /// caller turns `false` into `REFUSE_B_FULL`, and refuses *before*
    /// taking the item so nothing is charged for a plant that did not
    /// happen).
    #[inline]
    fn push(&mut self, rec: ChargeRec) -> bool {
        if self.len >= MAX_LIVE_CHARGES {
            return false;
        }
        self.entries[self.len] = rec;
        self.len += 1;
        true
    }

    #[inline]
    fn remove_at(&mut self, i: usize) {
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        self.entries[self.len] = ChargeRec::default();
    }
}

/// Plant the held throwable against the structure at the address.
///
/// Refusal-first, then check-all-then-take-all, `build::place`'s split for
/// `build::place`'s reason: a half-paid plant leaves the client's
/// predicted inventory and the server's disagreeing, which is the
/// container-divergence class `CLAUDE.md`'s trap list names.
///
/// **There is no claim check here, and its absence is the verb.**
/// `build::repair` refuses on `deploys.foreign_claim` — you do not mend a
/// stranger's wall. Raiding a stranger's wall is the entire point, so a
/// claim check would make this verb work only on your own base. That is
/// worth saying out loud, because every other address-taking verb in this
/// crate has one and a later pass copying the pattern would reintroduce it
/// as a "fix".
#[allow(clippy::too_many_arguments)]
pub fn place(
    bc: &BuildContent,
    dc: &DeployContent,
    cc: &crate::combat::CombatContent,
    charges: &mut Charges,
    deploys: &Deploys,
    pieces: &Pieces,
    p: &mut Player,
    tick: u64,
    deploy: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    // The hand decides what is planted and pays for it. An empty hand, a
    // hatchet, a stack of wood — none of them are a charge, and all of
    // them read here as "you cannot pay", which is what `REFUSE_B_COST`
    // means. A refusal code of its own would have to cross to
    // `web/src/refusals.js`, which this lane may not touch.
    let held = held_item(p);
    let Some(def) = cc.held_throw(held) else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_COST, 0);
        return;
    };
    let found = if deploy {
        deploys.find_index(cx, cz, level, loc)
    } else {
        pieces.find_index(cx, cz, level, loc)
    };
    let Some(i) = found else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0);
        return;
    };
    // An inert row is refused rather than charged against, `repair`'s
    // check for `repair`'s reason: a content table swapped under a live
    // store must refuse, never index out of bounds (wall 5).
    let row = if deploy {
        let rec = deploys.entries()[i];
        if rec.row as u16 >= dc.def_count || dc.defs[rec.row as usize].hp == 0 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
            return;
        }
        rec.row
    } else {
        let rec = pieces.entries()[i];
        if rec.row as u16 >= bc.piece_count || bc.pieces[rec.row as usize].hp == 0 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
            return;
        }
        rec.row
    };
    // Reach to the same anchor everything else measures to, so what you
    // can build and mend you can also breach. The throwable's own
    // `range_m` is the distance — a content number, not a literal — but it
    // is clamped to the build reach, because a charge that could be
    // planted from further than a wall can be built is a raider standing
    // outside the tool cupboard's reach with no way for the owner to
    // answer. Content sets the arm's length; the sim sets the ceiling.
    let reach_m = (def.reach_cm as f32 * 0.01).min(BUILD_REACH_M);
    let (ax, az) = anchor(cx, cz, loc);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > reach_m * reach_m {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_REACH, 0);
        return;
    }
    // Check the store, check the pocket, then spend both. `push` is
    // checked before `inv_take` runs so a full store never eats a charge.
    if charges.len() >= MAX_LIVE_CHARGES {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_FULL, 0);
        return;
    }
    if inv_count(&p.inv, held) == 0 {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_COST, 0);
        return;
    }
    inv_take(&mut p.inv, held, 1);
    let fires_at = tick + def.fuse_ticks as u64;
    charges.push(ChargeRec {
        cx,
        cz,
        level,
        loc,
        deploy,
        structure: def.structure,
        fires_at,
        owner: p.id,
    });
    events.push(
        EV_CHARGE_PLACED,
        crate::gather::cell_key(cx, cz),
        // `EV_STRUCT_HIT`'s packing, bit for bit, because it addresses the
        // same thing: the store bit at 24, then level, loc and row in
        // 8-bit fields below it. A client that can draw a hit on a wall
        // can draw a charge stuck to it without learning a second layout.
        if deploy { STRUCT_DEPLOY_BIT } else { 0 }
            | ((level as u32) << 16)
            | ((loc as u32) << 8)
            | row as u32,
        // Fuse ticks remaining, not the absolute deadline: a client that
        // joined mid-fuse has no shared tick origin to subtract from, and
        // a countdown is what it draws either way.
        def.fuse_ticks as u32,
    );
}

/// Run every burning fuse one tick, detonating what is due.
///
/// Called once a tick, after the player loop that can light one and before
/// the sweeps — a charge planted this tick with a zero fuse could not
/// exist (`validate` refuses `fuse_s = 0`), so nothing detonates on the
/// tick it was planted.
///
/// `budget` is the tick's shared `MAX_REMOVALS_PER_TICK`, taken by
/// reference for the reason `combat::raid` takes it: a blast that brings a
/// wall down spends the same structural-removal allowance a swing does,
/// and a tick that has spent it leaves the wall standing at one hp for the
/// next one. Wall 4 does not get a second allowance because the damage
/// arrived on a fuse.
// The arity allow `place` and `build::repair` carry, for their reason: two
// content tables, two stores, the charge list, the clock, the budget and
// the ring are six distinct owners, and bundling them into a context struct
// here would put a second definition of "the tick's mutable world" beside
// the one `World` already is.
#[allow(clippy::too_many_arguments)]
pub fn tick_fuses(
    bc: &BuildContent,
    dc: &DeployContent,
    charges: &mut Charges,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    tick: u64,
    budget: &mut usize,
    events: &mut EventQueue,
) {
    let mut i = 0;
    while i < charges.len {
        let c = charges.entries[i];
        if c.fires_at > tick {
            i += 1;
            continue;
        }
        // The target is resolved *now*, not remembered as a store index:
        // between the plant and the blast the store has been swap-removed
        // by every other removal in the world, so a remembered index would
        // point at a stranger's wall. An address cannot go stale that way
        // — it either still names something or it does not.
        let found = if c.deploy {
            deploys.find_index(c.cx, c.cz, c.level, c.loc)
        } else {
            pieces.find_index(c.cx, c.cz, c.level, c.loc)
        };
        // Nothing there any more — somebody else broke it first, or it
        // decayed. The charge is spent either way and the raider is out an
        // item: a charge that survived its target and waited for a rebuild
        // would be a trap with no counter-play, and one that refunded
        // would make planting free whenever a wall was already falling.
        // No event: the client drops the charge when the piece it was
        // stuck to leaves on `EV_PIECE_REMOVED`.
        if let Some(t) = found {
            if c.deploy {
                damage_deploy(dc, pieces, deploys, t, c.structure, events);
            } else {
                damage_piece(dc, bc, pieces, deploys, t, c.structure, budget, events);
            }
        }
        // Swap-remove without advancing: the entry now at `i` is the one
        // that was last, and it has not been tested yet.
        charges.remove_at(i);
    }
}
