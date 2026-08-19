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
//! **The blast (satchel blast v0, 2026-08-11).** A detonation is an area
//! now, not an address: everything structural inside `blast_cm` of the
//! planted anchor takes `structure` scaled by linear falloff, and every
//! *body* inside it takes `damage` the same way — the planter included,
//! because standing at your own bomb is the reference's lesson too. The
//! planted wall is simply the target at distance zero, so it still takes
//! the full number and `balance.toml` anchor 1's arithmetic
//! (`piece hp ÷ structure`) is undisturbed. Two consequences of "area,
//! not address" are deliberate: a charge whose wall was broken before
//! the fuse ran out **still detonates** (a bomb is not defused by its
//! excuse disappearing), and standing on a charge is no longer free —
//! `DEATH_BY_CHARGE` is the sixth cause and the death screen names the
//! planter. Still not built, each its own verb: no dud chance, no
//! defusing.

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
    /// Damage the blast takes off a structure at the epicentre; falloff
    /// scales it toward zero at `blast_cm`.
    pub structure: u16,
    /// Damage the blast takes off a *body* at the epicentre, the same
    /// falloff. Copied at plant time for `structure`'s stated reason.
    pub damage: u16,
    /// The blast radius, planar-and-vertical centimetres. Copied at plant
    /// time — the hole a charge clears was decided when it was planted.
    pub blast_cm: u16,
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
                damage: 0,
                blast_cm: 0,
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
        damage: def.damage,
        blast_cm: def.blast_cm,
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

/// One body the blast finished, parked for `World::tick` to lay down —
/// `mob::Bites`' split for `mob::Bites`' reason: `die` needs the whole
/// world and this function holds only its parts.
///
/// **Exactly bounded, no overflow policy owed**: a body dies at most once
/// per tick (`hp == 0` skips it thereafter), so `MAX_PLAYERS` entries
/// cannot be exceeded — wall 4 satisfied by construction rather than by a
/// drop rule.
pub struct BlastKills {
    entries: [(u8, u32, u16); crate::limits::MAX_PLAYERS],
    len: usize,
}

impl Default for BlastKills {
    fn default() -> Self {
        Self::new()
    }
}

impl BlastKills {
    pub const fn new() -> Self {
        Self {
            entries: [(0, 0, 0); crate::limits::MAX_PLAYERS],
            len: 0,
        }
    }

    /// `(victim slot, planter id, range_cm)` per finished body.
    #[inline]
    pub fn entries(&self) -> &[(u8, u32, u16)] {
        &self.entries[..self.len]
    }

    #[inline]
    fn push(&mut self, victim: u8, owner: u32, range_cm: u16) {
        if self.len < self.entries.len() {
            self.entries[self.len] = (victim, owner, range_cm);
            self.len += 1;
        }
    }
}

/// Linear falloff: the number at the epicentre, scaled toward zero at the
/// blast's edge. Integer arithmetic over centimetres, so both ends of the
/// wire would compute the identical value if a client ever predicted one.
#[inline]
fn falloff(full: u16, d_cm: i64, blast_cm: u16) -> u16 {
    if d_cm >= blast_cm as i64 || blast_cm == 0 {
        return 0;
    }
    ((full as i64 * (blast_cm as i64 - d_cm)) / blast_cm as i64) as u16
}

/// Run every burning fuse one tick, detonating what is due.
///
/// Called once a tick, after the player loop that can light one and before
/// the sweeps — a charge planted this tick with a zero fuse could not
/// exist (`validate` refuses `fuse_s = 0`), so nothing detonates on the
/// tick it was planted.
///
/// **A detonation is an area** (satchel blast v0 — the module header has
/// the design). The scan is bounded and deterministic: the 3×3 column
/// ring around the epicentre for pieces (blast_cm ≤ one build cell, the
/// const block below proves the ring covers it), one pass over the deploy
/// store, one over the players — target addresses are collected first and
/// re-resolved one at a time before damage, because `damage_piece`
/// swap-removes and a held index would point at a stranger's wall.
///
/// `budget` is the tick's shared `MAX_REMOVALS_PER_TICK`, taken by
/// reference for the reason `combat::raid` takes it: a blast that brings a
/// wall down spends the same structural-removal allowance a swing does,
/// and a tick that has spent it leaves the wall standing at one hp for the
/// next one. Wall 4 does not get a second allowance because the damage
/// arrived on a fuse.
// The arity allow `place` and `build::repair` carry, for their reason: the
// content tables, the stores, the players, the clock, the budget and the
// ring are distinct owners, and bundling them into a context struct here
// would put a second definition of "the tick's mutable world" beside the
// one `World` already is.
#[allow(clippy::too_many_arguments)]
pub fn tick_fuses(
    seed: u64,
    haven: &crate::terrain::Haven,
    bc: &BuildContent,
    dc: &DeployContent,
    cc: &crate::combat::CombatContent,
    charges: &mut Charges,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    players: &mut [Player; crate::limits::MAX_PLAYERS],
    tick: u64,
    budget: &mut usize,
    kills: &mut BlastKills,
    events: &mut EventQueue,
) {
    let mut i = 0;
    while i < charges.len {
        let c = charges.entries[i];
        if c.fires_at > tick {
            i += 1;
            continue;
        }
        detonate(
            seed, haven, bc, dc, cc, &c, pieces, deploys, players, budget, kills, events,
        );
        // Swap-remove without advancing: the entry now at `i` is the one
        // that was last, and it has not been tested yet.
        charges.remove_at(i);
    }
}

/// The candidate structural targets one blast can touch, collected before
/// any damage lands. 3×3 columns × 8 levels × (1 plane + 1 riser + 2
/// edges) is the exact ceiling; the array is that size, so there is no
/// overflow arm to get wrong.
const BLAST_TARGET_CAP: usize = 9 * crate::limits::MAX_BUILD_LEVELS * 4;

#[allow(clippy::too_many_arguments)]
fn detonate(
    seed: u64,
    haven: &crate::terrain::Haven,
    bc: &BuildContent,
    dc: &DeployContent,
    cc: &crate::combat::CombatContent,
    c: &ChargeRec,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    players: &mut [Player; crate::limits::MAX_PLAYERS],
    budget: &mut usize,
    kills: &mut BlastKills,
    events: &mut EventQueue,
) {
    use crate::build::{LEVEL_H_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER};
    use crate::limits::{MAX_BUILD_COORD, MAX_BUILD_LEVELS};

    let (ax, az) = anchor(c.cx, c.cz, c.loc);
    let ay = crate::collide::col_base_y(seed, haven, c.cx, c.cz) + c.level as f32 * LEVEL_H_M;
    let blast = c.blast_cm;

    // Distance from the epicentre to a point, centimetres. Planar plus
    // vertical in one metric, f32 sqrt (wall 1's list) cast back to the
    // integer centimetres the falloff divides in.
    let dist_cm = |x: f32, y: f32, z: f32| -> i64 {
        let (dx, dy, dz) = (x - ax, y - ay, z - az);
        ((dx * dx + dy * dy + dz * dz).sqrt() * 100.0) as i64
    };

    // --- structures: collect addresses, then re-resolve and damage. The
    // planted target is found by the same scan at distance ~zero, so it
    // takes the full number and needs no special case.
    let mut targets: [(u16, u16, u8, u8, bool, u16); BLAST_TARGET_CAP] =
        [(0, 0, 0, 0, false, 0); BLAST_TARGET_CAP];
    let mut n = 0usize;
    let bcx = c.cx as i32;
    let bcz = c.cz as i32;
    let mut dz = -1i32;
    while dz <= 1 {
        let mut dx = -1i32;
        while dx <= 1 {
            let (cx, cz) = (bcx + dx, bcz + dz);
            dx += 1;
            if cx < 0 || cz < 0 || cx >= MAX_BUILD_COORD as i32 || cz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let (cx, cz) = (cx as u16, cz as u16);
            let m = pieces.cols().get(cx, cz);
            let base = crate::collide::col_base_y(seed, haven, cx, cz);
            for level in 0..MAX_BUILD_LEVELS as u8 {
                let bit = 1u8 << level;
                let ly = base + level as f32 * LEVEL_H_M;
                let mut consider = |loc: u8, present: bool| {
                    if !present || n >= BLAST_TARGET_CAP {
                        return;
                    }
                    let (tx, tz) = anchor(cx, cz, loc);
                    let d = dist_cm(tx, ly, tz);
                    let scaled = falloff(c.structure, d, blast);
                    if scaled > 0 {
                        targets[n] = (cx, cz, level, loc, false, scaled);
                        n += 1;
                    }
                };
                consider(LOC_PLANE, m.planes & bit != 0);
                consider(LOC_RISER, m.stairs & bit != 0);
                consider(LOC_EDGE_XLO, (m.walls_xlo | m.doors_xlo) & bit != 0);
                consider(LOC_EDGE_ZLO, (m.walls_zlo | m.doors_zlo) & bit != 0);
            }
        }
        dz += 1;
    }
    // Deployables: one pass over the store, addresses only — the same
    // full scan `combat::raid` makes per swing.
    for rec in deploys.entries() {
        if n >= BLAST_TARGET_CAP {
            break;
        }
        let (tx, tz) = anchor(rec.cx, rec.cz, rec.loc);
        let ly =
            crate::collide::col_base_y(seed, haven, rec.cx, rec.cz) + rec.level as f32 * LEVEL_H_M;
        let d = dist_cm(tx, ly, tz);
        let scaled = falloff(c.structure, d, blast);
        if scaled > 0 {
            targets[n] = (rec.cx, rec.cz, rec.level, rec.loc, true, scaled);
            n += 1;
        }
    }
    for &(cx, cz, level, loc, is_deploy, scaled) in targets.iter().take(n) {
        // Re-resolve immediately before damaging: an earlier target's
        // removal may have swap-moved this one, or taken it down with a
        // collapsing doorway — an address cannot go stale, an index can.
        if is_deploy {
            if let Some(t) = deploys.find_index(cx, cz, level, loc) {
                damage_deploy(dc, pieces, deploys, t, scaled, events);
            }
        } else if let Some(t) = pieces.find_index(cx, cz, level, loc) {
            damage_piece(dc, bc, pieces, deploys, t, scaled, budget, events);
        }
    }

    // --- bodies. The planter is not exempt: standing at your own bomb is
    // the oldest lesson the reference's raiders learn. Sleepers are hit
    // too — a body is a body, and a wall-adjacent sleeper in a raid was
    // always going to be part of the bill.
    let player_hp = cc.player_hp;
    if c.damage == 0 || player_hp == 0 {
        return; // a wall-only charge, or unarmed combat content
    }
    for (slot, p) in players.iter_mut().enumerate() {
        if !p.active || p.hp == 0 {
            continue;
        }
        let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
        let py = p.body.qy as f32 * crate::movement::POS_Y_Q;
        let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
        let d = dist_cm(px, py, pz);
        let scaled = falloff(c.damage, d, blast);
        if scaled == 0 {
            continue;
        }
        // The funnel, reduced. No `EV_HIT`: a blast has no hitmarker to
        // draw, which is why the funnel does not own the event set.
        let crate::combat::Hurt { left, died, .. } = crate::combat::hurt(cc, p, scaled);
        let victim_id = p.id;
        events.push(
            crate::world::EV_HEALTH,
            victim_id,
            left as u32,
            player_hp as u32,
        );
        if died {
            events.push(crate::world::EV_DEATH, victim_id, c.owner, 0);
            kills.push(slot as u8, c.owner, d.clamp(0, u16::MAX as i64) as u16);
        }
    }
}

const _: () = {
    // The 3×3 piece ring is complete iff no blast can reach a fourth
    // column: the farthest in-cell anchor sits within one cell of the
    // epicentre's column, so the reach is blast + one cell, and the ring
    // covers two. `validate` bounds `blast_m` at the build cell; this is
    // the sim-side restatement so a widened content bound cannot silently
    // outrun the scan.
    assert!(crate::limits::BLAST_MAX_CM as f32 * 0.01 <= crate::build::BUILD_CELL_M);
};
