//! Building a home: the starter base (`sim_core::bots::STARTER`) read as
//! data and put up one op at a time, the way a player puts one up.
//!
//! A player builds from inside the base: stand in the core, pull out the
//! building plan, the hammer or the deployable (crafted first, from the
//! pack), look at the spot, press, and read what the server said. So does
//! [`Builder`]. Each op waits for exactly one verdict, fanned out to it by
//! `Survivor::event_with` (the one reader of the client's rings): a
//! placement broadcast at the address it asked for, a build or deploy
//! refusal, or three seconds of nothing.
//!
//! A starter costs thousands of wood and stone, far more than one trip
//! gathers, so the work comes in milestones ([`Milestone`]): the cupboard
//! and a twig shell; the wooden doors, a sleeping bag and a box inside;
//! the stone core; the storey above; the wood grades. The job pauses when
//! a milestone is done or the pack runs short, and the mind sends it off
//! for what the next one needs ([`Survey::needs`]). Once the starter
//! stands come its stations: a workbench on a foundation behind the core,
//! where `E` takes it, and a furnace on the upstairs floor, both in reach
//! of the stand spot, where a station's recipes are crafted. Then code locks on both doors and
//! the cupboard, armed with its own code ([`LockCode`]), and metal doors in
//! place of the wooden ones when the fragments are spare.
//!
//! After the base comes the gear a player works through at its benches
//! ([`Milestone::MetalTools`] on): blueprints learned ([`Op::Learn`]: the
//! tech tree's unlock at a bench, or paper read), then made ([`Op::Make`]),
//! and the bench itself swapped for the second rung on the way. The same
//! verdict-at-a-time machinery runs them from the stand spot.
//!
//! What it knows is its own: the plot it chose, what it built there (the
//! client's mirror of its own base, which it stands in), and the game's
//! prices from the tables the server sent. Getting in and out through its
//! own doors is [`Passage`], a fixed walk through the airlock that opens
//! each door on the way and shuts it behind.

use crate::agent::hands::Hands;
use crate::agent::home::{Home, HOLD_TICKS};
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::lock::LockCode;
use crate::agent::oven::Keep;
use crate::agent::route::{Route, Step};
use crate::agent::site::{self, Seen};
use crate::agent::stash::Ledger;
use crate::mind::{Why, BAG_ITEM};
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::STARTER_STAND_M;
use sim_core::bots::{op_addr, part_shape, BaseOp, BasePlan, Kit, OpAddr, Part, STARTER};
use sim_core::build::{
    anchor, column_floor_y, level_y, row_of, BUILD_CELL_M, BUILD_REACH_M, LOC_DIAG_A, LOC_DIAG_B,
    LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER_ZLO, MAT_STONE, MAT_TWIG, REFUSE_B_CLAIM,
    REFUSE_B_COST, REFUSE_B_REACH, REFUSE_B_SPOT, REFUSE_B_SUPPORT, REFUSE_B_TERRAIN,
    REFUSE_B_TIER,
};
use sim_core::craft::{
    STATION_FURNACE, STATION_NONE, STATION_RADIUS_M, STATION_WORKBENCH1, STATION_WORKBENCH2,
};
use sim_core::deploy::{
    arch_is_door, bench_tier, lockable, ACCESS_OP_ENTER, ACCESS_OP_SET_CODE, REFUSE_D_AUTH_FULL,
    REFUSE_D_CLAIM, REFUSE_D_COST, REFUSE_D_HAS_LOCK, REFUSE_D_LOCKOUT, REFUSE_D_OVERLAP,
    REFUSE_D_OWNER, REFUSE_D_REACH, REFUSE_D_SPOT, REFUSE_D_SUPPORT, REFUSE_D_TERRAIN,
};
use sim_core::limits::{
    CRAFT_COUNT_MAX, CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_ITEM_DEFS, TICK_HZ,
};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::{self, Haven};

/// How far the base has come, in the order a player builds a starter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Milestone {
    /// The cupboard, the foundations, the walls and doorways, in twig.
    #[default]
    Shell,
    /// Wooden doors on both doorways, a sleeping bag and a box inside.
    Doors,
    /// The ground floor graded to stone.
    Stone,
    /// The stairs, the floor over the cupboard, its walls and the roof.
    Upstairs,
    /// Whatever stone missed, graded to wood; the upper storey graded.
    Wood,
    /// A workbench behind the core, on a stone foundation of its own: its
    /// 100 fragments are what the first loot runs are for.
    Bench,
    /// A furnace on the floor over the cupboard, crafted at the bench; ore
    /// smelts there.
    Furnace,
    /// Code locks on both doors and the cupboard, armed with its code; a
    /// metal front door, and the inner one in metal whenever the fragments
    /// are spare.
    Locks,
    /// Metal hatchet and pickaxe (blueprints, learned at the bench) and a
    /// metal spear.
    MetalTools,
    /// A crossbow and metal arrows for it.
    Crossbow,
    /// Burlap hood and tunic, worn.
    Burlap,
    /// Medkits (a blueprint), on the belt.
    Medkits,
    /// The second bench rung, in the first one's place.
    Bench2,
    /// Gunpowder, from charcoal and sulfur smelted at the furnace.
    Gunpowder,
    /// The revolver and its rounds (blueprints, at the second rung).
    Revolver,
    /// A roadsign vest (a blueprint), worn.
    Roadsign,
    /// Everything this body builds and makes for itself.
    Done,
}

impl Milestone {
    pub const ALL: [Milestone; 17] = [
        Milestone::Shell,
        Milestone::Doors,
        Milestone::Stone,
        Milestone::Upstairs,
        Milestone::Wood,
        Milestone::Bench,
        Milestone::Furnace,
        Milestone::Locks,
        Milestone::MetalTools,
        Milestone::Crossbow,
        Milestone::Burlap,
        Milestone::Medkits,
        Milestone::Bench2,
        Milestone::Gunpowder,
        Milestone::Revolver,
        Milestone::Roadsign,
        Milestone::Done,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Milestone::Shell => "cupboard_and_twig_shell",
            Milestone::Doors => "doors_bag_and_box",
            Milestone::Stone => "stone_core",
            Milestone::Upstairs => "upstairs",
            Milestone::Bench => "workbench",
            Milestone::Furnace => "furnace",
            Milestone::Wood => "wood_grades",
            Milestone::Locks => "code_locks",
            Milestone::MetalTools => "metal_tools",
            Milestone::Crossbow => "crossbow",
            Milestone::Burlap => "burlap_armor",
            Milestone::Medkits => "medkits",
            Milestone::Bench2 => "workbench_2",
            Milestone::Gunpowder => "gunpowder",
            Milestone::Revolver => "revolver",
            Milestone::Roadsign => "roadsign_armor",
            Milestone::Done => "done",
        }
    }

    /// A milestone past the base: gear made at its benches.
    pub fn gear(self) -> bool {
        self >= Milestone::MetalTools && self != Milestone::Done
    }
}

/// Deployables by the catalog name a player knows them by: the rows are
/// the server's, found by the item each one places.
pub const HEARTH_ITEM: &str = "Hearth";
pub const DOOR_ITEM: &str = "Wooden Door";
pub const BOX_ITEM: &str = "Small Box";
pub const BENCH_ITEM: &str = "Workbench";
pub const BENCH2_ITEM: &str = "Workbench-2";
pub const FURNACE_ITEM: &str = "Furnace";
pub const LOCK_ITEM: &str = "Code Lock";
pub const METAL_DOOR_ITEM: &str = "Metal Door";
/// What a player holds to place a piece, and to grade one.
pub const PLAN_ITEM: &str = "Building Plan";
pub const HAMMER_ITEM: &str = "Hammer";

/// What the agent adds to the blueprint for its own use, as `(item, dx,
/// dz, level, milestone)`: the bag it wakes on and the box it keeps things
/// in, both behind the front door from the second milestone on. The box
/// stands in the stair cell, the one plane inside the room the cupboard
/// does not take (a box is solid, and the airlock's walk passes beside
/// it); the bag in the airlock, where a walk-over mat blocks nobody. Then
/// the furnace, within `craft::STATION_RADIUS_M` of the stand spot (the
/// gate is planar, so the floor above counts), on the floor over the
/// cupboard, the blueprint's own box spot: it is worked as a station and
/// never opened, so that `E` there takes the cupboard costs nothing. A
/// workbench is opened (`E` shows its tree), so each rung stands where `E`
/// takes it from the stand spot, on a stone foundation of its own behind
/// the core (`LATER`): the first on `YARD`, the second on `ANNEX`. The plot
/// is chosen to take both foundations.
const EXTRAS: [(&str, i8, i8, u8, Milestone); 3] = [
    (BOX_ITEM, 1, 0, 0, Milestone::Doors),
    (BAG_ITEM, 1, 1, 0, Milestone::Doors),
    (FURNACE_ITEM, 0, 0, 1, Milestone::Furnace),
];
/// Where in its cell a kit item goes (free placement). Everything stands at
/// its cell's centre but the bag: the airlock is a triangle foundation (the
/// NW half, `bots::BASE` at (1, 1)) with a diagonal wall on its hypotenuse
/// through the centre, so the bag lies inside the half, along the wall,
/// clear of it and of both doorways.
pub fn kit_pose(name: &str) -> sim_core::footprint::Pose {
    if name == BAG_ITEM {
        sim_core::footprint::Pose {
            ox: -32,
            oz: -32,
            yaw: 32,
        }
    } else {
        sim_core::footprint::Pose::CENTRE
    }
}

/// The cell behind the core the first bench's foundation takes, from the
/// plot.
pub const YARD: (i8, i8) = (0, -1);
/// The cell behind the stair cell the second bench's foundation takes.
pub const ANNEX: (i8, i8) = (1, -1);
/// The plot's cells: the core's three by two, and the yard and annex cells
/// behind it (not the rest of that row, which may be a neighbour's).
fn on_plot(plan: &BasePlan, cx: u16, cz: u16) -> bool {
    let at = |(dx, dz): (i8, i8)| {
        (
            plan.cx.wrapping_add_signed(i16::from(dx)),
            plan.cz.wrapping_add_signed(i16::from(dz)),
        )
    };
    (cx.wrapping_sub(plan.cx) <= 2 && cz.wrapping_sub(plan.cz) <= 1)
        || (cx, cz) == at(YARD)
        || (cx, cz) == at(ANNEX)
}

/// What it does after the starter and its stations, in order, as `(op, dx,
/// dz, level, loc, milestone, optional)`: the first bench on its stone
/// foundation behind the core (`YARD`); the cupboard's lock and code
/// (the doors' are the blueprint's own), the inner door swapped for metal
/// like the front one, the second bench rung on a stone foundation of its
/// own behind the stair cell (`ANNEX`: every plane inside is taken, and
/// the first rung stays, its recipes made at either); then the gear,
/// each blueprint learned before it is made, a parent before its child
/// (pistol rounds before the revolver: the tree's own edge). An optional op
/// holds no milestone back: it is done when the pack pays for it on top of
/// what the milestone in hand costs.
const LATER: [(Op, i8, i8, u8, u8, Milestone, bool); 27] = [
    (
        Op::Piece(Part::Foundation),
        YARD.0,
        YARD.1,
        0,
        LOC_PLANE,
        Milestone::Bench,
        false,
    ),
    (
        Op::Grade(MAT_STONE),
        YARD.0,
        YARD.1,
        0,
        LOC_PLANE,
        Milestone::Bench,
        false,
    ),
    (
        Op::Kit(BENCH_ITEM),
        YARD.0,
        YARD.1,
        0,
        LOC_PLANE,
        Milestone::Bench,
        false,
    ),
    (Op::Lock, 0, 0, 0, LOC_PLANE, Milestone::Locks, false),
    (Op::Code, 0, 0, 0, LOC_PLANE, Milestone::Locks, false),
    (
        Op::Swap(DOOR_ITEM, METAL_DOOR_ITEM),
        INNER.0,
        INNER.1,
        INNER.2,
        INNER.3,
        Milestone::Locks,
        true,
    ),
    (
        Op::Learn("Metal Hatchet"),
        0,
        0,
        0,
        0,
        Milestone::MetalTools,
        false,
    ),
    (
        Op::Make("Metal Hatchet", 1),
        0,
        0,
        0,
        0,
        Milestone::MetalTools,
        false,
    ),
    (
        Op::Learn("Metal Pickaxe"),
        0,
        0,
        0,
        0,
        Milestone::MetalTools,
        false,
    ),
    (
        Op::Make("Metal Pickaxe", 1),
        0,
        0,
        0,
        0,
        Milestone::MetalTools,
        false,
    ),
    (
        Op::Make("Metal Spear", 1),
        0,
        0,
        0,
        0,
        Milestone::MetalTools,
        false,
    ),
    (
        Op::Make("Crossbow", 1),
        0,
        0,
        0,
        0,
        Milestone::Crossbow,
        false,
    ),
    (
        Op::Make("Metal Arrow", 20),
        0,
        0,
        0,
        0,
        Milestone::Crossbow,
        false,
    ),
    (
        Op::Make("Burlap Hood", 1),
        0,
        0,
        0,
        0,
        Milestone::Burlap,
        false,
    ),
    (
        Op::Make("Burlap Tunic", 1),
        0,
        0,
        0,
        0,
        Milestone::Burlap,
        false,
    ),
    (Op::Learn("Medkit"), 0, 0, 0, 0, Milestone::Medkits, false),
    (Op::Make("Medkit", 2), 0, 0, 0, 0, Milestone::Medkits, false),
    (
        Op::Piece(Part::Foundation),
        ANNEX.0,
        ANNEX.1,
        0,
        LOC_PLANE,
        Milestone::Bench2,
        false,
    ),
    (
        Op::Grade(MAT_STONE),
        ANNEX.0,
        ANNEX.1,
        0,
        LOC_PLANE,
        Milestone::Bench2,
        false,
    ),
    (
        Op::Kit(BENCH2_ITEM),
        ANNEX.0,
        ANNEX.1,
        0,
        LOC_PLANE,
        Milestone::Bench2,
        false,
    ),
    (
        Op::Make("Gunpowder", 30),
        0,
        0,
        0,
        0,
        Milestone::Gunpowder,
        false,
    ),
    (
        Op::Learn("Pistol Round"),
        0,
        0,
        0,
        0,
        Milestone::Revolver,
        false,
    ),
    (
        Op::Learn("Revolver"),
        0,
        0,
        0,
        0,
        Milestone::Revolver,
        false,
    ),
    (
        Op::Make("Revolver", 1),
        0,
        0,
        0,
        0,
        Milestone::Revolver,
        false,
    ),
    (
        Op::Make("Pistol Round", 24),
        0,
        0,
        0,
        0,
        Milestone::Revolver,
        false,
    ),
    (
        Op::Learn("Roadsign Vest"),
        0,
        0,
        0,
        0,
        Milestone::Roadsign,
        false,
    ),
    (
        Op::Make("Roadsign Vest", 1),
        0,
        0,
        0,
        0,
        Milestone::Roadsign,
        false,
    ),
];

/// Every op the builder knows: the blueprint's, then its own, then what
/// comes after.
pub const OPS: usize = STARTER.len() + EXTRAS.len() + LATER.len();
const _: () = assert!(OPS <= 128, "op sets are u128 masks");

/// An op's answer comes within this long (`explorer::VERDICT_SECS`).
const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// Tries at one op (no answer, or a refusal that may pass later) before it
/// is given up.
pub const MAX_FAILS: u8 = 3;
/// The builder stands this close to its spot in the core.
pub const STAND_M: f32 = 0.25;
/// A route to the stand spot ends this close (`Route::to`'s least); the
/// rest is walked straight.
const ROUTE_STOP_M: f32 = 0.5;
/// A walk through the airlock counts a waypoint reached this close.
pub const WAYPOINT_M: f32 = 0.1;
/// A waypoint the body has got no closer to in this long is not going to
/// be reached.
pub const PASSAGE_STALL_TICKS: u32 = 2 * TICK_HZ;
/// How far off the aim line `E` still takes a thing: the human client's
/// `interact::AIM_RADIUS_M`.
const E_AIM_RADIUS_M: f32 = 1.0;
/// Plots refused by the ground or a claim, remembered so as not to be
/// chosen again.
pub const BAD_PLOTS: usize = 4;
/// Distinct items one bill of materials can name.
pub const BILL_ROWS: usize = 8;
/// Shortfalls the survey reports.
pub const NEED_ROWS: usize = 3;

/// What an op does, independent of where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Lay a twig piece: the plan in hand.
    Piece(Part),
    /// Put a deployable down: the item itself in hand, named.
    Kit(&'static str),
    /// Grade the piece at the address to a material: the hammer in hand.
    Grade(u8),
    /// Bolt a code lock onto what stands at the address: the lock in hand.
    Lock,
    /// Set this body's code on the lock at the address (the keypad), which
    /// arms it.
    Code,
    /// Take the first deployable at the address back up and put the
    /// second there in its place: a metal door for a wooden one, the
    /// second bench rung for the first.
    Swap(&'static str, &'static str),
    /// Learn the blueprint of the named item: at a bench, for the tree's
    /// junk, or by reading paper for it.
    Learn(&'static str),
    /// Hold this many of the named item, made at its station.
    Make(&'static str, u32),
}

#[derive(Clone, Copy, Debug)]
struct Spec {
    op: Op,
    dx: i8,
    dz: i8,
    level: u8,
    loc: u8,
    /// `None`: not this body's to build (the feed, the blueprint's upstairs
    /// box, which the box inside replaces).
    stage: Option<Milestone>,
    /// Done when the pack has the spare for it; holds no milestone back.
    optional: bool,
}

impl Spec {
    /// Where the op is worked from, as the chain point the walk through the
    /// doors goes by and the spot itself, relative to the plot's corner.
    /// Everything is worked from the stand spot in the core but a door's
    /// lock and a door's swap: those from beside the door, on the side
    /// the cupboard is not (from the core `E` takes the cupboard first),
    /// where that door is the one `E` and the take-down's key take, the
    /// front from outside, the inner from the room.
    fn work_spot(&self) -> (usize, [f32; 2]) {
        let door = matches!(self.op, Op::Lock | Op::Code | Op::Swap(..));
        let here = (self.dx, self.dz, self.level, self.loc);
        if door && here == FRONT {
            (CHAIN.len() - 1, CHAIN[CHAIN.len() - 1])
        } else if door && here == INNER {
            (INNER_LEG, INNER_SPOT)
        } else {
            (0, CHAIN[0])
        }
    }
}

/// Where the inner door is worked from: in the room, closer to it than to
/// the box or the stairs, so it is the nearest thing standing there.
const INNER_SPOT: [f32; 2] = [1.5, -0.45];

/// Op `i`: the blueprint's in order, then [`EXTRAS`], then [`LATER`].
fn spec(i: usize) -> Spec {
    if let Some(&(name, dx, dz, level, stage)) =
        i.checked_sub(STARTER.len()).and_then(|k| EXTRAS.get(k))
    {
        return Spec {
            op: Op::Kit(name),
            dx,
            dz,
            level,
            loc: LOC_PLANE,
            stage: Some(stage),
            optional: false,
        };
    }
    if let Some(&(op, dx, dz, level, loc, stage, optional)) = i
        .checked_sub(STARTER.len() + EXTRAS.len())
        .and_then(|k| LATER.get(k))
    {
        return Spec {
            op,
            dx,
            dz,
            level,
            loc,
            stage: Some(stage),
            optional,
        };
    }
    let none = |dx, dz, level, loc| Spec {
        op: Op::Grade(0),
        dx,
        dz,
        level,
        loc,
        stage: None,
        optional: false,
    };
    match STARTER[i] {
        BaseOp::Place(part, dx, dz, level, loc) => Spec {
            op: Op::Piece(part),
            dx,
            dz,
            level,
            loc,
            stage: Some(if level == 0 && part != Part::Stairs {
                Milestone::Shell
            } else {
                Milestone::Upstairs
            }),
            optional: false,
        },
        BaseOp::Deploy(kit, dx, dz, level, loc) => {
            let (op, stage, optional) = match kit {
                Kit::Hearth => (Op::Kit(HEARTH_ITEM), Some(Milestone::Shell), false),
                Kit::Door => (Op::Kit(DOOR_ITEM), Some(Milestone::Doors), false),
                Kit::Box => (Op::Kit(BOX_ITEM), None, false),
                // The metal front door: the one a raid comes through first,
                // part of the locks milestone.
                Kit::MetalDoor => (
                    Op::Swap(DOOR_ITEM, METAL_DOOR_ITEM),
                    Some(Milestone::Locks),
                    false,
                ),
                Kit::Lock => (Op::Lock, Some(Milestone::Locks), false),
            };
            Spec {
                op,
                dx,
                dz,
                level,
                loc,
                stage,
                optional,
            }
        }
        BaseOp::Upgrade(dx, dz, level, loc, material) => Spec {
            op: Op::Grade(material),
            dx,
            dz,
            level,
            loc,
            stage: Some(
                if material == MAT_STONE && level == 0 && loc != LOC_RISER_ZLO {
                    Milestone::Stone
                } else {
                    Milestone::Wood
                },
            ),
            optional: false,
        },
        BaseOp::Code(dx, dz, level, loc) => Spec {
            op: Op::Code,
            dx,
            dz,
            level,
            loc,
            stage: Some(Milestone::Locks),
            optional: false,
        },
        BaseOp::Feed(dx, dz, level) => none(dx, dz, level, LOC_PLANE),
    }
}

fn addr(plan: &BasePlan, s: &Spec) -> OpAddr {
    let (cx, cz) = plan.cell(s.dx, s.dz);
    OpAddr {
        cx,
        cz,
        level: s.level,
        loc: s.loc,
    }
}

/// The piece shape laid at a grade's address.
fn shape_at(s: &Spec) -> Option<u8> {
    place_of(s).and_then(|i| match spec(i).op {
        Op::Piece(part) => Some(part_shape(part)),
        _ => None,
    })
}

/// The op that lays the piece at a grade's address.
fn place_of(s: &Spec) -> Option<usize> {
    (0..OPS).find(|&i| {
        let o = spec(i);
        matches!(o.op, Op::Piece(_)) && (o.dx, o.dz, o.level, o.loc) == (s.dx, s.dz, s.level, s.loc)
    })
}

const fn bit(i: usize) -> u128 {
    1 << i
}

/// Up to [`BILL_ROWS`] items and the units of each.
#[derive(Clone, Copy, Debug, Default)]
struct Bill {
    rows: [(u16, u32); BILL_ROWS],
    n: usize,
}

impl Bill {
    fn add(&mut self, item: u16, units: u32) {
        if let Some(r) = self.rows[..self.n].iter_mut().find(|r| r.0 == item) {
            r.1 = r.1.saturating_add(units);
        } else if self.n < BILL_ROWS {
            self.rows[self.n] = (item, units);
            self.n += 1;
        }
    }

    fn add_bill(&mut self, other: &Bill) {
        self.add_times(other, 1);
    }

    /// `other`, `times` over.
    fn add_times(&mut self, other: &Bill, times: u32) {
        if times == 0 {
            return;
        }
        for &(item, units) in &other.rows[..other.n] {
            self.add(item, units.saturating_mul(times));
        }
    }

    fn paid_by(&self, core: &ClientCore) -> bool {
        self.rows[..self.n]
            .iter()
            .all(|&(item, units)| count(core, item) >= units)
    }

    /// The pack pays for this on top of what `reserve` keeps of the same
    /// items: it has the spare.
    fn spare_in(&self, core: &ClientCore, reserve: &Bill) -> bool {
        self.rows[..self.n].iter().all(|&(item, units)| {
            let kept = reserve.rows[..reserve.n]
                .iter()
                .filter(|r| r.0 == item)
                .map(|r| r.1)
                .sum::<u32>();
            count(core, item) >= units.saturating_add(kept)
        })
    }
}

/// Units of an item in the pack and belt.
fn count(core: &ClientCore, item: u16) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// Empty slots in the pack and belt.
fn free_slots(core: &ClientCore) -> usize {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count == 0)
        .count()
}

/// What `units` crafts of `recipe` make fits the pack: room for it as it
/// stands, or a slot its inputs, taken at the queue in slot order
/// (`craft::inv_take`), leave empty.
pub fn craft_fits(core: &ClientCore, recipe: u16, units: u16) -> bool {
    let Some(def) = core.recipes.recipes.get(usize::from(recipe)) else {
        return false;
    };
    if room_for(core, def.output) {
        return true;
    }
    let mut inv = core.inv;
    for &(input, per) in &def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())] {
        sim_core::craft::inv_take(&mut inv, input, u32::from(per) * u32::from(units));
    }
    inv[..INV_SLOTS].iter().any(|s| s.count == 0)
}

/// One more `item` fits the pack: an empty slot, or a stack of it with room.
fn room_for(core: &ClientCore, item: u16) -> bool {
    let max = core.catalog.row(usize::from(item)).stack_max;
    core.inv[..INV_SLOTS]
        .iter()
        .any(|s| s.count == 0 || (s.item == item && s.count < max))
}

/// The item a catalog name means on this server.
pub fn item_named(core: &ClientCore, name: &str) -> Option<u16> {
    (0..usize::from(core.catalog.count).min(MAX_ITEM_DEFS))
        .find(|&i| core.catalog.name(i) == name.as_bytes())
        .map(|i| i as u16)
}

/// The crafting stations this body has standing in its base, where the
/// mirror still shows them: what the craft gate (`craft::enqueue`) asks
/// for, a deployable of the station's kind within `STATION_RADIUS_M`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stations {
    /// Its workbench, its second-rung bench and its furnace, where they
    /// stand.
    pub bench: Option<[f32; 2]>,
    pub bench2: Option<[f32; 2]>,
    pub furnace: Option<[f32; 2]>,
    /// Its best bench's rung (`craft::STATION_WORKBENCH*`), 0 with none: a
    /// bench crafts its own rung's recipes and every rung below.
    pub tier: u8,
}

impl Stations {
    pub const NONE: Self = Self {
        bench: None,
        bench2: None,
        furnace: None,
        tier: 0,
    };

    /// Where the station a recipe names stands, if this body has one: no
    /// station needs nothing ([`Self::usable`]); a bench recipe takes a
    /// bench of its rung or higher.
    pub fn spot(&self, station: u8) -> Option<[f32; 2]> {
        match station {
            STATION_WORKBENCH1 => self.bench.or(self.bench2),
            STATION_WORKBENCH2 => self.bench2,
            STATION_FURNACE => self.furnace,
            _ => None,
        }
    }

    /// A recipe at this station can be crafted here, somewhere.
    pub fn usable(&self, station: u8) -> bool {
        station == STATION_NONE || self.spot(station).is_some()
    }

    /// A crafter standing at `at` passes the station gate for this recipe,
    /// with a little to spare for the body's drift.
    pub fn in_reach(&self, station: u8, at: [f32; 2]) -> bool {
        station == STATION_NONE
            || self.spot(station).is_some_and(|[x, z]| {
                (x - at[0]).hypot(z - at[1]) <= STATION_RADIUS_M - STATION_SLACK_M
            })
    }
}

/// How far inside `STATION_RADIUS_M` a crafter keeps.
pub const STATION_SLACK_M: f32 = 0.5;

/// The recipe that makes `item` with no blueprint missing, at no station
/// or one this body has: `(recipe, ticks per unit, station)`.
pub fn recipe_for(core: &ClientCore, item: u16, has: &Stations) -> Option<(u16, u32, u8)> {
    if core.recipes_have < core.recipes.recipe_count {
        return None;
    }
    let known = core.known();
    (0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len())).find_map(|r| {
        let def = core.recipes.recipes[r];
        let usable = def.out_count > 0
            && def.output == item
            && has.usable(def.station)
            && (!def.blueprint || (r < 64 && known & (1 << r) != 0));
        usable.then_some((r as u16, def.ticks, def.station))
    })
}

/// Ticks until this player's own craft queue, as the server last announced
/// it, has paid out: the head unit's countdown and every unit behind it,
/// at the recipe's full time (a bench rebate only makes it sooner).
pub fn queue_wait(core: &ClientCore) -> u32 {
    let live = &core.jobs[..usize::from(core.jobs_count).min(core.jobs.len())];
    let unit = |r: u8| {
        core.recipes
            .recipes
            .get(usize::from(r))
            .map_or(0, |d| d.ticks)
    };
    let behind: u32 = live
        .iter()
        .enumerate()
        .map(|(i, &(r, n))| unit(r) * u32::from(n).saturating_sub(u32::from(i == 0)))
        .sum();
    u32::from(core.craft_eta_ticks) + behind
}

/// The announced queue holds a job of `recipe`: what was asked is coming,
/// however long the jobs ahead of it take.
pub fn queued(core: &ClientCore, recipe: u16) -> bool {
    core.jobs[..usize::from(core.jobs_count).min(core.jobs.len())]
        .iter()
        .any(|&(r, n)| u16::from(r) == recipe && n > 0)
}

/// What crafting one `item` takes from the pack.
fn recipe_bill(core: &ClientCore, item: u16, has: &Stations) -> Option<Bill> {
    let (r, ..) = recipe_for(core, item, has)?;
    let def = core.recipes.recipes[usize::from(r)];
    let mut bill = Bill::default();
    for &(input, need) in &def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())] {
        bill.add(input, u32::from(need));
    }
    Some(bill)
}

/// The recipe that makes `item` at any station, known or not, and how
/// many one craft pays: what the gear is priced by before its blueprint
/// is learned or its bench stands.
fn price(core: &ClientCore, item: u16) -> Option<(Bill, u32)> {
    let r = recipe_any(core, item)?;
    let def = core.recipes.recipes[usize::from(r)];
    let mut bill = Bill::default();
    for &(input, need) in &def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())] {
        bill.add(input, u32::from(need));
    }
    Some((bill, u32::from(def.out_count)))
}

/// The recipe index that makes `item`, whatever it needs.
fn recipe_any(core: &ClientCore, item: u16) -> Option<u16> {
    if core.recipes_have < core.recipes.recipe_count {
        return None;
    }
    (0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len()))
        .find(|&r| {
            let def = core.recipes.recipes[r];
            def.out_count > 0 && def.output == item
        })
        .map(|r| r as u16)
}

/// Whether this body may make `item`: its recipe takes no blueprint, or
/// that blueprint is learned. `None` before the tables are in.
fn learned(core: &ClientCore, item: u16) -> Option<bool> {
    let r = recipe_any(core, item)?;
    let def = core.recipes.recipes[usize::from(r)];
    Some(!def.blueprint || sim_core::research::knows(core.known(), r))
}

/// The tech tree's node for `item`'s recipe, as the server sent it: what
/// it costs (`(coin, units)`), its parent recipe, and the bench rung it is
/// unlocked at.
fn tree_row(core: &ClientCore, item: u16) -> Option<(u16, u16, u16, u8)> {
    let r = recipe_any(core, item)?;
    let rc = &core.research;
    if core.research_have < rc.row_count {
        return None;
    }
    let row = rc.row_for_recipe(r)?;
    let station = core.recipes.recipes[usize::from(r)].station;
    Some((
        rc.coin,
        row.cost,
        row.requires,
        sim_core::research::node_tier(station),
    ))
}

/// A pack slot holding paper that teaches `item`.
fn paper_slot(core: &ClientCore, item: u16) -> Option<u8> {
    (0..INV_SLOTS)
        .find(|&i| sim_core::research::blueprint_target(&core.research, core.inv[i]) == Some(item))
        .map(|i| i as u8)
}

/// Every catalog name has arrived.
fn catalog_complete(core: &ClientCore) -> bool {
    let n = usize::from(core.catalog.count).min(MAX_ITEM_DEFS);
    n > 0 && (0..n).all(|i| core.catalog.lens[i] > 0)
}

/// The deployable row that places `item`.
fn kit_row(core: &ClientCore, item: u16) -> Option<u16> {
    let defs = &core.deploy_defs;
    (0..core.deploy_defs_have.min(defs.def_count)).find(|&row| {
        let def = defs.defs[usize::from(row)];
        def.hp > 0 && def.item == item
    })
}

/// A piece row, if the server has sent it.
fn piece_row(core: &ClientCore, shape: u8, material: u8) -> Option<u16> {
    row_of(&core.piece_defs, shape, material).filter(|&r| r < core.piece_defs_have)
}

fn piece_bill(core: &ClientCore, row: u16) -> Bill {
    let def = core.piece_defs.pieces[usize::from(row)];
    let mut bill = Bill::default();
    for &(item, units) in &def.costs[..usize::from(def.n_costs).min(def.costs.len())] {
        bill.add(item, u32::from(units));
    }
    bill
}

/// Where a body is in its own base: the room with the cupboard, the
/// airlock between the doors, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Room,
    Airlock,
    Outside,
}

/// The airlock walk, relative to the corner the core, the stair cell and
/// the airlock share (the plot cell's far corner): the stand spot in the
/// core, round the cupboard and the box, through the inner door, across
/// the airlock and out of the front door. Each point keeps a body's
/// capsule clear of the walls, the posts, the cupboard and the box.
///
/// The stand spot sits 0.45 m from the cupboard's side, a capsule and a
/// hair; it is walked to straight from the wall side, never past the
/// cupboard's corner.
const CHAIN: [[f32; 2]; 9] = [
    [
        STARTER_STAND_M.0 - BUILD_CELL_M,
        STARTER_STAND_M.1 - BUILD_CELL_M,
    ],
    [STARTER_STAND_M.0 - BUILD_CELL_M, -BUILD_CELL_M + 0.58],
    [-0.3, -BUILD_CELL_M + 0.58],
    [-0.3, -0.65],
    [1.5, -0.65],
    [1.5, 0.65],
    [0.65, 0.65],
    [0.65, 1.5],
    [-1.0, 1.5],
];
/// Chain legs a door stands across: from the room to the airlock (the
/// inner door), from the airlock to the outside (the front door).
const INNER_LEG: usize = 4;
const FRONT_LEG: usize = 7;
/// Where a walk that begins in the airlock joins the chain.
const AIRLOCK_JOIN: usize = 6;
/// The walk's points that are inside the base ([`Builder::stand_at`]).
pub const STANDS: usize = CHAIN.len() - 1;
/// The two doors, as the blueprint addresses them from the plot.
const FRONT: (i8, i8, u8, u8) = (1, 1, 0, LOC_EDGE_XLO);
const INNER: (i8, i8, u8, u8) = (1, 1, 0, LOC_EDGE_ZLO);

/// Which way through the airlock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    In,
    Out,
}

/// What the walk through the airlock wants next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pass {
    Go(Intent),
    /// Press use on the door at this address, holding this intent.
    Use(OpAddr, Intent),
    /// Enter this body's code at the lock on the door at this address: the
    /// lock did not know it (a new session, say), and its door would not
    /// swing.
    Enter(OpAddr, Intent),
    Done,
    Fail(Why),
}

/// Walking in or out through its own doors: open the one ahead, walk the
/// airlock's fixed points, shut each behind as soon as it is passed.
///
/// A door is shut from the chain point just past it, looking back at it:
/// from there it is the one a player's `E` picks (the other door of the
/// cell is behind the eye or well off the aim line), and the eye does not
/// look through a leaf already shut.
#[derive(Clone, Copy, Debug, Default)]
pub struct Passage {
    way: Option<Way>,
    /// The chain point walked to next; `CHAIN.len()` is "on the way to the
    /// front door from outside".
    at: usize,
    /// Where the walk began on the chain.
    start: usize,
    /// The door just passed, to shut before walking on.
    shut: Option<(i8, i8, u8, u8)>,
    /// The last point is reached, and the walk ends once it is shut.
    out: bool,
    /// The chain point the walk ends at, short of its own end (the airlock,
    /// where the doors' locks are worked).
    stop: Option<usize>,
    /// A use sent at this door, wanting it open or shut, and when.
    door: Option<(OpAddr, bool, u32)>,
    use_ready: bool,
    /// Its own lock refused this door's use: the code goes in first...
    knock: Option<OpAddr>,
    /// ...and was entered here, at this tick, awaiting the lock's answer.
    entering: Option<(OpAddr, u32)>,
    held: Option<u32>,
    tries: u8,
    best: Option<(f32, u32)>,
}

impl Passage {
    const IDLE: Passage = Passage {
        way: None,
        at: 0,
        start: 0,
        shut: None,
        out: false,
        stop: None,
        door: None,
        use_ready: false,
        knock: None,
        entering: None,
        held: None,
        tries: 0,
        best: None,
    };

    /// Mid-walk: a door may stand open, and switching now would leave it.
    pub fn busy(&self) -> bool {
        self.way.is_some()
    }

    /// A use went out at `tick`.
    fn sent(&mut self, tick: u32) {
        if let Some(d) = self.door.as_mut() {
            d.2 = tick;
        }
        self.use_ready = false;
    }

    /// A deploy refusal: the use in flight was refused by the door's lock
    /// (`REFUSE_D_OWNER`), so the code goes in before the next; or the code
    /// entered was not taken. A lock that has locked this body out, or
    /// whose list has no room for it, will not take the code however often
    /// it goes in: the walk gives up at its next step.
    fn on_refused(&mut self, reason: u8) {
        if self.entering.is_some() {
            self.entering = None;
            self.knock = None;
            self.tries = match u32::from(reason) {
                REFUSE_D_LOCKOUT | REFUSE_D_AUTH_FULL => MAX_FAILS,
                _ => self.tries.saturating_add(1),
            };
            return;
        }
        if u32::from(reason) == REFUSE_D_OWNER {
            if let Some((door, ..)) = self.door.take() {
                self.knock = Some(door);
                self.held = None;
            }
        }
    }

    /// A lock let this body through: the code entered at that door was
    /// right, and its use is pressed again.
    fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        let ours = self
            .entering
            .is_some_and(|(d, _)| (d.cx, d.cz, d.level, d.loc) == (cx, cz, level, loc));
        if ours {
            self.entering = None;
            self.knock = None;
        }
        ours
    }

    /// A door that would not answer, or that `E` would not pick from
    /// here: one more try, and the walk fails after `MAX_FAILS`.
    fn tried(&mut self) -> Option<Pass> {
        self.tries = self.tries.saturating_add(1);
        (self.tries >= MAX_FAILS).then(|| {
            self.way = None;
            Pass::Fail(Why::Refused)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        plan: &BasePlan,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        way: Way,
        stop: Option<usize>,
        tick: u32,
    ) -> Pass {
        let corner = corner(plan);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.way != Some(way) || self.stop != stop {
            *self = Passage {
                way: Some(way),
                stop,
                ..Passage::IDLE
            };
            let rel = [x - corner[0], z - corner[1]];
            let nearest = |range: std::ops::RangeInclusive<usize>| {
                range
                    .min_by(|&a, &b| {
                        let d = |i: usize| (CHAIN[i][0] - rel[0]).hypot(CHAIN[i][1] - rel[1]);
                        d(a).total_cmp(&d(b))
                    })
                    .unwrap_or(0)
            };
            self.at = match (way, region_at(rel)) {
                (Way::In, Region::Outside) => CHAIN.len(),
                (_, Region::Airlock) => AIRLOCK_JOIN,
                (_, Region::Room) => nearest(0..=INNER_LEG),
                (Way::Out, Region::Outside) => {
                    self.way = None;
                    return Pass::Done;
                }
            };
            self.start = self.at;
        }
        // Its own lock would not take the code: no more tries this walk.
        if self.tries >= MAX_FAILS {
            self.way = None;
            return Pass::Fail(Why::Refused);
        }
        // Its own lock did not know it: the code, then the use again.
        if let Some((door, at)) = self.entering {
            if tick.wrapping_sub(at) < VERDICT_TICKS {
                return Pass::Go(look_at(seed, haven, core, door));
            }
            self.entering = None;
            self.knock = None;
            if let Some(fail) = self.tried() {
                return fail;
            }
        }
        if let Some(door) = self.knock {
            let intent = look_at(seed, haven, core, door);
            let held = *self.held.get_or_insert(tick);
            if tick.wrapping_sub(held) >= HOLD_TICKS
                && hands.settled()
                && e_picks(core, x, z, hands.view().0, door)
            {
                self.held = None;
                self.entering = Some((door, tick));
                return Pass::Enter(door, intent);
            }
            if tick.wrapping_sub(held) >= VERDICT_TICKS {
                self.held = None;
                self.knock = None;
                if let Some(fail) = self.tried() {
                    return fail;
                }
            }
            return Pass::Go(intent);
        }
        // A use in flight: the mirror's leaf says when it swung.
        if let Some((door, open, at)) = self.door {
            match door_open(core, door) {
                Some(now) if now == open => {
                    self.door = None;
                    self.tries = 0;
                }
                None => self.door = None,
                Some(_) if self.use_ready => {}
                Some(_) if tick.wrapping_sub(at) < VERDICT_TICKS => {
                    return Pass::Go(look_at(seed, haven, core, door));
                }
                Some(_) => {
                    self.door = None;
                    if let Some(fail) = self.tried() {
                        return fail;
                    }
                }
            }
        }
        if let Some(d) = self.shut {
            let door = door_addr(plan, d);
            if door_open(core, door) == Some(true) {
                return self.press(seed, haven, core, body, hands, door, false, tick);
            }
            self.shut = None;
            self.held = None;
        }
        if self.out {
            self.way = None;
            return Pass::Done;
        }
        if self.at == CHAIN.len() {
            let p0 = at_corner(corner, CHAIN[CHAIN.len() - 1]);
            match route.to(core, body, p0, 0.5, true, tick) {
                step @ Step::Walk { .. } => return Pass::Go(step.walk().unwrap_or(Intent::IDLE)),
                Step::Wait => return Pass::Go(Intent::walk(yaw_toward(p0[0] - x, p0[1] - z))),
                Step::Blocked => {
                    self.way = None;
                    return Pass::Fail(Why::Stuck);
                }
                Step::Arrived => {
                    self.at = CHAIN.len() - 1;
                    self.best = None;
                }
            }
        }
        // The door across the leg ahead opens before the body walks it; a
        // leg this walk began past is not crossed.
        let (leg, crosses) = match way {
            // Heading for point `at` from `at + 1`.
            Way::In => (self.at, self.start > self.at),
            // Heading for point `at` from `at - 1`.
            Way::Out => (
                self.at.wrapping_sub(1),
                self.at >= 1 && self.start < self.at,
            ),
        };
        let door = crosses.then(|| door_across(leg)).flatten();
        if let Some(d) = door {
            let door = door_addr(plan, d);
            if door_open(core, door) == Some(false) {
                return self.press(seed, haven, core, body, hands, door, true, tick);
            }
        }
        let target = at_corner(corner, CHAIN[self.at]);
        let left = (target[0] - x).hypot(target[1] - z);
        if left <= WAYPOINT_M {
            self.best = None;
            // The leg just walked had its door open: shut it from here.
            self.shut = door;
            if self.stop == Some(self.at) {
                // Where this walk was for: done once the door behind is
                // shut.
                self.out = true;
                return Pass::Go(Intent::IDLE);
            }
            match way {
                Way::In => {
                    if self.at == 0 {
                        self.way = None;
                        return Pass::Done;
                    }
                    self.at -= 1;
                }
                Way::Out => {
                    if self.at + 1 == CHAIN.len() {
                        self.out = true;
                    } else {
                        self.at += 1;
                    }
                }
            }
            return Pass::Go(Intent::IDLE);
        }
        match self.best {
            Some((best, since)) if left > best - 0.05 => {
                if tick.wrapping_sub(since) >= PASSAGE_STALL_TICKS {
                    self.way = None;
                    return Pass::Fail(Why::Stuck);
                }
            }
            _ => self.best = Some((left, tick)),
        }
        Pass::Go(Intent::walk(yaw_toward(target[0] - x, target[1] - z)))
    }

    /// Eyes on the door, then the press once they have settled on it and
    /// the door is what a player's `E` would take from here.
    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        seed: u64,
        haven: &Haven,
        core: &ClientCore,
        body: &EntityState,
        hands: &Hands,
        door: OpAddr,
        open: bool,
        tick: u32,
    ) -> Pass {
        let intent = look_at(seed, haven, core, door);
        let held = *self.held.get_or_insert(tick);
        if tick.wrapping_sub(held) >= HOLD_TICKS && hands.settled() {
            let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
            if e_picks(core, x, z, hands.view().0, door) {
                self.held = None;
                self.door = Some((door, open, tick));
                self.use_ready = true;
                return Pass::Use(door, intent);
            }
        }
        if tick.wrapping_sub(held) >= VERDICT_TICKS {
            // Settled on it this long and `E` still takes something else.
            self.held = None;
            if let Some(fail) = self.tried() {
                return fail;
            }
        }
        Pass::Go(intent)
    }
}

/// The door across chain leg `leg`, if any.
fn door_across(leg: usize) -> Option<(i8, i8, u8, u8)> {
    match leg {
        INNER_LEG => Some(INNER),
        FRONT_LEG => Some(FRONT),
        _ => None,
    }
}

/// What the human client's structure keys (`R` to repair, `X` to plant a
/// charge, `Backspace`) would take from a body standing at (`x`, `z`): the
/// structure whose anchor is nearest the feet within the build reach, over
/// both stores, a tie going to the later record and a deployable over a
/// piece (`client::ui::structure::nearest`). `true` for a deployable.
pub(crate) fn nearest_structure(core: &ClientCore, x: f32, z: f32) -> Option<(bool, OpAddr)> {
    let mut best: Option<(bool, OpAddr)> = None;
    let mut best_d2 = BUILD_REACH_M * BUILD_REACH_M;
    let pieces = core
        .pieces
        .entries()
        .iter()
        .map(|r| (false, r.cx, r.cz, r.level, r.loc));
    let deploys = core
        .deploys
        .entries()
        .iter()
        .map(|r| (true, r.cx, r.cz, r.level, r.loc));
    for (deploy, cx, cz, level, loc) in pieces.chain(deploys) {
        let (ax, az) = anchor(cx, cz, loc);
        let d2 = (ax - x) * (ax - x) + (az - z) * (az - z);
        if d2 > best_d2 {
            continue;
        }
        best_d2 = d2;
        best = Some((deploy, OpAddr { cx, cz, level, loc }));
    }
    best
}

/// Would a player standing at (`x`, `z`) and facing wire yaw `yaw` press
/// `E` on this door (or the box or cupboard on this plane)? The human
/// client's pick (`client::ui::interact::
/// resolve`), asked conservatively: the door is on the aim line and nearer
/// than anything else there that `E` could take (every deployable in
/// reach, a door scored where it hangs and the rest at its cell's centre,
/// and every backpack). A dead tie counts as not picked.
pub(crate) fn e_picks(core: &ClientCore, x: f32, z: f32, yaw: u16, door: OpAddr) -> bool {
    e_picks_by(core, x, z, yaw, door, 0.0)
}

/// [`e_picks`] with `slack` metres to spare: the thing stays on the aim
/// line, and nothing nearer comes onto it, if the view drifts that far
/// sideways. A cupboard at arm's length sits right on the edge of what is
/// in front of the eye, where a degree decides whether `E` takes it.
pub(crate) fn e_picks_by(
    core: &ClientCore,
    x: f32,
    z: f32,
    yaw: u16,
    door: OpAddr,
    slack: f32,
) -> bool {
    let (fx, fz) = sim_core::yaw_dir(yaw);
    let reach2 = BUILD_REACH_M * BUILD_REACH_M;
    // (aimed, squared distance) of a point from here: `give` metres
    // narrower (the target) or wider (a rival) than the aim really is.
    let score = |px: f32, pz: f32, give: f32| {
        let (dx, dz) = (px - x, pz - z);
        let t = dx * fx + dz * fz;
        let (ox, oz) = (dx - t * fx, dz - t * fz);
        let r = (E_AIM_RADIUS_M + give).max(0.0);
        (t > -give && ox * ox + oz * oz <= r * r, dx * dx + dz * dz)
    };
    let in_reach = |cx: u16, cz: u16| {
        let (cx, cz) = sim_core::deploy::cell_center(cx, cz);
        (cx - x) * (cx - x) + (cz - z) * (cz - z) <= reach2
    };
    if !in_reach(door.cx, door.cz) {
        return false;
    }
    let (ax, az) = anchor(door.cx, door.cz, door.loc);
    let (aimed, mine) = score(ax, az, -slack);
    if !aimed || mine > reach2 {
        return false;
    }
    let rank = |row: u8| deploy_def(core, row).map_or(u8::MAX, |def| e_rank(def.arch));
    let own = deploy_rec(core, door).map_or(u8::MAX, |d| rank(d.row));
    let deploys = core.deploys.entries().iter().filter(|d| {
        (d.cx, d.cz, d.level, d.loc) != (door.cx, door.cz, door.level, door.loc)
            && in_reach(d.cx, d.cz)
    });
    let rivals = deploys
        .map(|d| {
            let (x, z) = anchor(d.cx, d.cz, d.loc);
            (x, z, rank(d.row))
        })
        .chain(
            core.bags
                .entries()
                .iter()
                .map(|b| (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q, E_RANK_BAG)),
        );
    for (px, pz, r) in rivals {
        // A thing `E` does not take is no rival; one scored exactly the
        // same (a bench on the floor over the cupboard) loses to the
        // better tiebreak, as the human client's pick has it.
        let (aimed, d2) = score(px, pz, slack);
        let tie_lost = d2 == mine && r > own;
        if r != u8::MAX && aimed && d2 <= mine && d2 <= reach2 && !tie_lost {
            return false;
        }
    }
    true
}

/// A bag's place in `E`'s tiebreak.
const E_RANK_BAG: u8 = 2;

/// `E`'s tiebreak between two things scored the same, as the human
/// client's pick orders them (`client::ui::interact::Verb::tie`): a door,
/// a bag, a box, the cupboard, an oven, a recycler, a research table, a
/// bench. `u8::MAX` for what `E` does not take at all.
fn e_rank(arch: u8) -> u8 {
    use sim_core::deploy::{
        ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_GARAGE_DOOR, ARCH_HEARTH, ARCH_RECYCLER,
        ARCH_RESEARCH, ARCH_WINDOW_SHUTTER, ARCH_WORKBENCH, ARCH_WORKBENCH2, ARCH_WORKBENCH3,
    };
    match arch {
        ARCH_DOOR | ARCH_GARAGE_DOOR | ARCH_WINDOW_SHUTTER => 1,
        ARCH_BOX => 3,
        ARCH_HEARTH => 4,
        ARCH_FIRE | ARCH_FURNACE => 5,
        ARCH_RECYCLER => 6,
        ARCH_RESEARCH => 7,
        ARCH_WORKBENCH | ARCH_WORKBENCH2 | ARCH_WORKBENCH3 => 9,
        _ => u8::MAX,
    }
}

/// Sideways slides of the eyes off a thing's middle, metres, tried in
/// turn: within `E`'s metre either way.
const AIM_OFFSETS_M: [f32; 11] = [0.0, -0.2, 0.2, -0.35, 0.35, -0.5, 0.5, -0.6, 0.6, -0.7, 0.7];
/// The drift of the view an aim point has to survive...
const AIM_SLACK_M: f32 = 0.2;
/// ...and the drift left at the press, once the eyes have settled.
pub(crate) const PRESS_SLACK_M: f32 = 0.1;

/// Where to look to work `at` from (`x`, `z`): its look point, slid
/// sideways across the line of sight by the least of [`AIM_OFFSETS_M`]
/// that `E` takes it from with [`AIM_SLACK_M`] to spare. The look point
/// itself when none does; the press then waits, and misses.
pub(crate) fn aim_point(
    core: &ClientCore,
    seed: u64,
    haven: &Haven,
    at: OpAddr,
    x: f32,
    z: f32,
) -> [f32; 3] {
    let [px, py, pz] = look_point(seed, haven, core, at);
    let (dx, dz) = (px - x, pz - z);
    let d = (dx * dx + dz * dz).sqrt();
    if d <= f32::EPSILON {
        return [px, py, pz];
    }
    let (sx, sz) = (-dz / d, dx / d);
    AIM_OFFSETS_M
        .iter()
        .map(|&o| [px + sx * o, py, pz + sz * o])
        .find(|p| {
            let yaw = yaw_toward(p[0] - x, p[2] - z);
            e_picks_by(core, x, z, yaw, at, AIM_SLACK_M)
        })
        .unwrap_or([px, py, pz])
}

/// The plot cell's far corner, where the core, the stair cell and the
/// airlock meet.
fn corner(plan: &BasePlan) -> [f32; 2] {
    [
        (f32::from(plan.cx) + 1.0) * BUILD_CELL_M,
        (f32::from(plan.cz) + 1.0) * BUILD_CELL_M,
    ]
}

fn at_corner(corner: [f32; 2], p: [f32; 2]) -> [f32; 2] {
    [corner[0] + p[0], corner[1] + p[1]]
}

fn region_at(rel: [f32; 2]) -> Region {
    let [x, z] = rel;
    let span = BUILD_CELL_M;
    if x > -span && x < span && z > -span && z < 0.0 {
        Region::Room
    } else if x > 0.0 && z > 0.0 && x + z < span {
        Region::Airlock
    } else {
        Region::Outside
    }
}

fn door_addr(plan: &BasePlan, (dx, dz, level, loc): (i8, i8, u8, u8)) -> OpAddr {
    op_addr(plan, BaseOp::Deploy(Kit::Door, dx, dz, level, loc))
}

/// Is the door at this address open, per the mirror; `None` when no door
/// hangs there.
fn door_open(core: &ClientCore, at: OpAddr) -> Option<bool> {
    let defs = &core.deploy_defs;
    core.deploys
        .entries()
        .iter()
        .find(|d| (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc))
        .filter(|d| {
            u16::from(d.row) < core.deploy_defs_have.min(defs.def_count)
                && arch_is_door(defs.defs[usize::from(d.row)].arch)
        })
        .map(|d| d.open)
}

/// Where the eyes go to work an address: the piece's middle, a door's
/// handle height, a plane's top.
pub(crate) fn look_point(seed: u64, haven: &Haven, core: &ClientCore, at: OpAddr) -> [f32; 3] {
    let (x, z) = anchor(at.cx, at.cz, at.loc);
    let floor = match core.pieces.cols().plate(at.cx, at.cz) {
        Some(plate) => column_floor_y(seed, haven, at.cx, at.cz, plate),
        None => terrain::ground(seed, haven, x, z),
    };
    let up = match at.loc {
        LOC_EDGE_XLO | LOC_EDGE_ZLO | LOC_DIAG_A | LOC_DIAG_B => 1.1,
        LOC_PLANE if at.level == 0 => 0.1,
        _ => 0.6,
    };
    [x, floor + level_y(at.level) + up, z]
}

pub(crate) fn look_at(seed: u64, haven: &Haven, core: &ClientCore, at: OpAddr) -> Intent {
    Intent {
        look: Look::Point(look_point(seed, haven, core, at)),
        ..Intent::IDLE
    }
}

/// What the builder wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Go(Intent),
    /// Craft what the next op needs (`encode_action_craft`): one, or a
    /// batch of gear.
    Craft {
        recipe: u16,
        count: u16,
    },
    /// Move a stack to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Lay a twig piece, the plan in hand (`encode_action_place`).
    Place {
        row: u16,
        at: OpAddr,
        intent: Intent,
    },
    /// Put a deployable down, it in hand (`encode_action_deploy`); `bag`
    /// when it is a sleeping bag, which home keeps a list of.
    Deploy {
        row: u16,
        at: OpAddr,
        /// Where in the cell it stands and which way it faces (free
        /// placement) — [`kit_pose`].
        pose: sim_core::footprint::Pose,
        bag: bool,
        intent: Intent,
    },
    /// Grade a piece, the hammer in hand (`encode_action_upgrade`).
    Upgrade {
        at: OpAddr,
        material: u8,
        intent: Intent,
    },
    /// Open or shut a door (`encode_action_use`).
    Use {
        at: OpAddr,
        intent: Intent,
    },
    /// A keypad op on the lock at this address (`encode_action_access`):
    /// set this body's code, or enter it.
    Access {
        at: OpAddr,
        op: u8,
        code: LockCode,
        intent: Intent,
    },
    /// Take the deployable at this address back up (`encode_action_demolish`).
    Demolish {
        at: OpAddr,
        intent: Intent,
    },
    /// Learn a recipe at the bench's tree (`encode_action_unlock`).
    Unlock {
        recipe: u16,
    },
    /// Read the blueprint in this pack slot (`encode_action_research`).
    Read {
        slot: u8,
    },
    Done,
    Fail(Why),
}

/// Which of the server's refusal rings answers an op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ring {
    Build,
    Deploy,
    Research,
}

/// What went out for an op, and so what answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sent {
    Place,
    Grade,
    Deploy,
    Lock,
    Code,
    Demolish,
    Unlock,
    Read,
}

impl Sent {
    fn ring(self) -> Ring {
        match self {
            Sent::Place | Sent::Grade => Ring::Build,
            Sent::Deploy | Sent::Lock | Sent::Code | Sent::Demolish => Ring::Deploy,
            Sent::Unlock | Sent::Read => Ring::Research,
        }
    }
}

/// What the server said about the op in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Yes,
    /// A refusal, and which ring it came from.
    No {
        ring: Ring,
        reason: u8,
    },
}

/// What the builder is waiting on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Await {
    /// A craft of `item` (by `recipe`) for op `op`, with this many in the
    /// pack before, done by then.
    Craft {
        op: usize,
        item: u16,
        recipe: u16,
        before: u32,
        until: u32,
    },
    /// A stack of `item` on its way to the belt, for op `op`.
    Belt { op: usize, item: u16, since: u32 },
    /// Op `op` at this address, holding this intent until the answer.
    Op {
        op: usize,
        at: OpAddr,
        sent: Sent,
        since: u32,
        intent: Intent,
    },
}

/// The next milestone, and what it still needs from the pack. Cached once
/// a second; the mind reads it through the summary.
#[derive(Clone, Copy, Debug, Default)]
pub struct Survey {
    pub milestone: Milestone,
    /// Items short for the rest of the milestone, and how many.
    pub needs: [(u16, u32); NEED_ROWS],
    pub needs_len: u8,
    /// Some op of the milestone can go now.
    pub ready: bool,
    /// Its own cupboard stands on the plot.
    pub hearth: bool,
    /// Everything the rest of the milestone costs from the pack: what a
    /// visit to the box keeps in the pack, or takes out of the box.
    pub bill: [(u16, u32); BILL_ROWS],
    pub bill_len: u8,
    /// The pack is short of something the box held when last opened.
    pub take_out: bool,
}

impl Survey {
    pub fn needs(&self) -> &[(u16, u32)] {
        &self.needs[..usize::from(self.needs_len)]
    }

    pub fn bill(&self) -> &[(u16, u32)] {
        &self.bill[..usize::from(self.bill_len)]
    }
}

/// What building has come to.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildStats {
    pub sites: u64,
    pub abandoned: u64,
    pub placed: u64,
    pub deployed: u64,
    pub graded: u64,
    pub refusals: u64,
    pub no_answer: u64,
    pub given_up: u64,
    pub crafted: u64,
    pub uses: u64,
    /// Locks bolted on and codes set; deployables swapped for better ones;
    /// blueprints learned; gear made (units).
    pub locks: u64,
    pub codes: u64,
    pub swapped: u64,
    pub learned: u64,
    pub made: u64,
    /// Codes entered at its own locks, to be let through.
    pub entered: u64,
}

/// The base this body is building, and the op it is on. Outlives goals:
/// the plot and what stands on it carry over from one build goal to the
/// next, the op in hand does not.
#[derive(Clone, Copy, Debug)]
pub struct Builder {
    plan: Option<BasePlan>,
    done: u128,
    /// Deployables the server said it put down for this body: the mirror
    /// shows where every deployable stands, not whose it is.
    mine: u128,
    /// Refused for want of support: tried again once something else stands.
    deferred: u128,
    /// Tried `MAX_FAILS` times, or out of reach of what this body can do.
    given_up: u128,
    fails: [u8; OPS],
    bad: [Option<(u16, u16)>; BAD_PLOTS],
    waiting: Option<Await>,
    /// The op the last `Act` asked to send, until `sent` confirms it.
    want: Option<Await>,
    verdict: Option<Verdict>,
    held: Option<(usize, u32)>,
    /// Walk back onto the stand spot itself before the next op (a reach
    /// refusal).
    restand: bool,
    /// The last straight walk onto the stand spot: the nearest it came,
    /// and when.
    approach: Option<(f32, u32)>,
    /// An op that got no answer in time, whose answer may still come: its
    /// kind, address, and when it was given up on. A refusal of that kind
    /// is its, not the next op's.
    late: Option<(usize, Ring, OpAddr, u32)>,
    /// A swap whose first deployable is down and whose second is not up
    /// yet: it is finished before anything else, and the op that put the
    /// first one there counts as standing meanwhile.
    swapping: Option<usize>,
    /// This body's lock code, once it has one.
    code: Option<LockCode>,
    /// The box as its panel last showed it: gear kept there counts as held.
    stored: Ledger,
    /// The milestone a build goal began on; it ends when that one does.
    began: Option<Milestone>,
    passage: Passage,
    /// The last move asked for a door's use.
    use_out: bool,
    survey: Survey,
    pub stats: BuildStats,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub const fn new() -> Self {
        Self {
            plan: None,
            done: 0,
            mine: 0,
            deferred: 0,
            given_up: 0,
            fails: [0; OPS],
            bad: [None; BAD_PLOTS],
            waiting: None,
            want: None,
            verdict: None,
            held: None,
            restand: false,
            approach: None,
            late: None,
            swapping: None,
            code: None,
            stored: Ledger::EMPTY,
            began: None,
            passage: Passage::IDLE,
            use_out: false,
            survey: Survey {
                milestone: Milestone::Shell,
                needs: [(0, 0); NEED_ROWS],
                needs_len: 0,
                ready: false,
                hearth: false,
                bill: [(0, 0); BILL_ROWS],
                bill_len: 0,
                take_out: false,
            },
            stats: BuildStats {
                sites: 0,
                abandoned: 0,
                placed: 0,
                deployed: 0,
                graded: 0,
                refusals: 0,
                no_answer: 0,
                given_up: 0,
                crafted: 0,
                uses: 0,
                locks: 0,
                codes: 0,
                swapped: 0,
                learned: 0,
                made: 0,
                entered: 0,
            },
        }
    }

    /// A new session: the plot is chosen again (the mirror still shows
    /// what stands, so what was built is not built twice).
    pub fn reset(&mut self) {
        let (stats, code) = (self.stats, self.code);
        *self = Self::new();
        self.stats = stats;
        self.code = code;
    }

    /// The code its locks are set to and opened with. Never printed.
    pub fn set_code(&mut self, code: LockCode) {
        self.code = Some(code);
    }

    /// Stop whatever op or walk was in hand; the base stays.
    pub fn halt(&mut self) {
        self.waiting = None;
        self.want = None;
        self.verdict = None;
        self.held = None;
        self.restand = false;
        self.approach = None;
        self.began = None;
        self.passage = Passage::IDLE;
        self.use_out = false;
    }

    /// The plot, once chosen.
    pub fn plan(&self) -> Option<BasePlan> {
        self.plan
    }

    /// A plot chosen already (a unit test's scene).
    #[cfg(test)]
    pub(crate) fn set_plan(&mut self, plan: BasePlan) {
        self.plan = Some(plan);
    }

    pub fn survey(&self) -> &Survey {
        &self.survey
    }

    /// Ops the server has seen stand (or that stood already).
    pub fn done_ops(&self) -> u32 {
        self.done.count_ones()
    }

    /// Nothing would be lost by switching now: no op waiting on its answer,
    /// no craft or belt move under way, no door left open mid-walk.
    pub fn at_checkpoint(&self) -> bool {
        self.waiting.is_none() && self.want.is_none() && !self.passage.busy()
    }

    /// Is this cell on its plot or next to it: somewhere a fire pit put
    /// down would be in the way of its doors and its yard.
    pub fn near_plot(&self, cx: u16, cz: u16) -> bool {
        self.plan.is_some_and(|p| {
            let (dx, dz) = (
                i32::from(cx) - i32::from(p.cx),
                i32::from(cz) - i32::from(p.cz),
            );
            (-1..=3).contains(&dx) && (-2..=2).contains(&dz)
        })
    }

    /// Is this address part of its own base (the furnace's yard cell too)?
    pub fn owns(&self, cx: u16, cz: u16, _level: u8, _loc: u8) -> bool {
        self.plan.is_some_and(|p| on_plot(&p, cx, cz))
    }

    /// Where the body stands in its base.
    pub fn region(&self, body: &EntityState) -> Region {
        self.region_of([body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q])
    }

    /// Where a spot on the ground (x, z) is in its base.
    pub fn region_of(&self, at: [f32; 2]) -> Region {
        let Some(plan) = self.plan else {
            return Region::Outside;
        };
        let c = corner(&plan);
        region_at([at[0] - c[0], at[1] - c[1]])
    }

    /// The spot in the core every op is in reach of.
    pub fn stand(&self) -> Option<[f32; 2]> {
        self.plan.map(|p| at_corner(corner(&p), CHAIN[0]))
    }

    /// The front doorway stands: in and out goes through the airlock.
    pub fn walled(&self, core: &ClientCore) -> bool {
        self.plan.is_some_and(|p| {
            let at = door_addr(&p, FRONT);
            piece_at(core, at).is_some()
        })
    }

    /// Would a walk from here out into the island have to pass its doors?
    pub fn must_exit(&self, core: &ClientCore, body: &EntityState) -> bool {
        self.region(body) != Region::Outside && self.walled(core)
    }

    /// Does one of its doors stand open, per the mirror?
    pub fn door_open(&self, core: &ClientCore) -> bool {
        self.plan.is_some_and(|p| {
            [FRONT, INNER]
                .iter()
                .any(|&d| door_open(core, door_addr(&p, d)) == Some(true))
        })
    }

    /// A walk through the doors is under way.
    pub fn passing(&self) -> bool {
        self.passage.busy()
    }

    /// A walk out through the doors is under way.
    pub fn leaving(&self) -> bool {
        self.passage.way == Some(Way::Out)
    }

    /// Where a body may stand in its own base: the points of the walk
    /// through its doors, the stand spot first, the airlock's last
    /// ([`STANDS`] of them; the walk's end outside the front door is not
    /// one).
    pub fn stand_at(&self, i: usize) -> Option<[f32; 2]> {
        let plan = self.plan.filter(|_| i < STANDS)?;
        Some(at_corner(corner(&plan), CHAIN[i]))
    }

    /// Walk to stand point `stop` ([`Builder::stand_at`]) through whatever
    /// doors lie between, shutting each behind: in from outside, or along
    /// the chain from where the body is inside.
    #[allow(clippy::too_many_arguments)]
    pub fn pass_to(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        stop: usize,
        tick: u32,
    ) -> Act {
        let Some(plan) = self.plan.filter(|_| stop < STANDS) else {
            return Act::Fail(Why::NotFound);
        };
        let way = match self.passage.way {
            Some(way) if self.passage.stop == Some(stop) => way,
            _ => {
                let c = corner(&plan);
                let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
                let rel = [x - c[0], z - c[1]];
                let from = match region_at(rel) {
                    Region::Outside => usize::MAX,
                    Region::Airlock => AIRLOCK_JOIN,
                    Region::Room => (0..=INNER_LEG)
                        .min_by(|&a, &b| {
                            let d = |i: usize| (CHAIN[i][0] - rel[0]).hypot(CHAIN[i][1] - rel[1]);
                            d(a).total_cmp(&d(b))
                        })
                        .unwrap_or(0),
                };
                if from == usize::MAX || stop < from {
                    Way::In
                } else {
                    Way::Out
                }
            }
        };
        let pass = self.passage.step(
            core,
            seed,
            haven,
            &plan,
            body,
            hands,
            route,
            way,
            Some(stop),
            tick,
        );
        let act = self.pass_act(pass, Act::Done);
        self.use_out = matches!(act, Act::Use { .. });
        act
    }

    /// Walk in or out through the doors (`GoHome`, or leaving for work).
    #[allow(clippy::too_many_arguments)]
    pub fn pass(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        way: Way,
        tick: u32,
    ) -> Act {
        let Some(plan) = self.plan else {
            return Act::Fail(Why::NotFound);
        };
        let pass = self.passage.step(
            core, seed, haven, &plan, body, hands, route, way, None, tick,
        );
        let act = self.pass_act(pass, Act::Done);
        self.use_out = matches!(act, Act::Use { .. });
        act
    }

    /// What a step of the walk through the doors asks to send; `done` when
    /// it is over.
    fn pass_act(&self, pass: Pass, done: Act) -> Act {
        match pass {
            Pass::Go(i) => Act::Go(i),
            Pass::Use(at, intent) => Act::Use { at, intent },
            Pass::Enter(at, intent) => match self.code {
                Some(code) => Act::Access {
                    at,
                    op: ACCESS_OP_ENTER,
                    code,
                    intent,
                },
                None => Act::Fail(Why::Refused),
            },
            Pass::Done => done,
            Pass::Fail(why) => Act::Fail(why),
        }
    }

    /// The last `Act` went out on the wire.
    pub fn sent(&mut self, tick: u32) {
        if std::mem::take(&mut self.use_out) {
            self.passage.sent(tick);
            self.stats.uses += 1;
            return;
        }
        let Some(mut w) = self.want.take() else {
            return;
        };
        match &mut w {
            Await::Craft { until, .. } => *until = until.wrapping_add(tick),
            Await::Belt { since, .. } | Await::Op { since, .. } => *since = tick,
        }
        self.verdict = None;
        self.waiting = Some(w);
    }

    /// A placement broadcast: the answer when it is the address asked.
    pub fn on_placed(&mut self, cx: u16, cz: u16, level: u8, loc: u8, deploy: bool) {
        let ring = if deploy { Ring::Deploy } else { Ring::Build };
        if let Some((op, r, at, _)) = self.late {
            if (at.cx, at.cz, at.level, at.loc) == (cx, cz, level, loc) && r == ring {
                self.late = None;
                if deploy {
                    self.mine |= bit(op);
                }
            }
        }
        if let Some(Await::Op { at, sent, .. }) = self.waiting {
            if (at.cx, at.cz, at.level, at.loc) == (cx, cz, level, loc) && sent.ring() == ring {
                self.verdict = Some(Verdict::Yes);
            }
        }
    }

    /// A build or deploy refusal: the answer when that kind is in flight
    /// (the rings carry only this player's own), unless an op of that kind
    /// timed out and its answer is still owed: a refusal carries no
    /// address, and this one is most likely the late op's. A door that
    /// would not swing for this body (its lock does not know it) is the
    /// walk's to answer, with the code.
    pub fn on_refused(&mut self, deploy: bool, reason: u8) {
        if deploy {
            self.passage.on_refused(reason);
        }
        self.refusal(if deploy { Ring::Deploy } else { Ring::Build }, reason);
    }

    /// A research or tree refusal (`research::REFUSE_R_*`).
    pub fn on_research_refused(&mut self, reason: u8) {
        self.refusal(Ring::Research, reason);
    }

    fn refusal(&mut self, ring: Ring, reason: u8) {
        if self.late.is_some_and(|(_, r, _, _)| r == ring) {
            self.late = None;
            return;
        }
        if let Some(Await::Op { sent, .. }) = self.waiting {
            if sent.ring() == ring && self.verdict.is_none() {
                self.verdict = Some(Verdict::No { ring, reason });
            }
        }
    }

    /// A lock let this body through (`EV_AUTH`): the code it entered at its
    /// own door was right.
    pub fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        if self.passage.on_auth(cx, cz, level, loc) {
            self.stats.entered += 1;
        }
    }

    /// The craft queue said no.
    pub fn on_craft_refused(&mut self) {
        if let Some(Await::Craft { until, .. }) = self.waiting.as_mut() {
            *until = 0;
        }
    }

    /// Bring `done` up to what the mirror shows standing on the plot, and
    /// work out the next milestone and what it needs: what the pack and the
    /// box (as `stored` last showed it) are short of between them. Once a
    /// second.
    pub fn survey_now(&mut self, core: &ClientCore, stored: &Ledger) {
        self.stored = *stored;
        // Before a plot is chosen the work is the whole blueprint, and its
        // price does not depend on where.
        let plan = self.plan.unwrap_or(BasePlan::new(0, 0, 0));
        if self.plan.is_some() {
            self.reconcile(core, &plan);
        }
        let milestone = self.milestone();
        let mut needs = [(0u16, 0u32); NEED_ROWS];
        let mut n = 0;
        let bill = self.with_spare(core, &plan, milestone, self.bill(core, milestone, stored));
        let mut take_out = false;
        for &(item, units) in &bill.rows[..bill.n] {
            let carried = count(core, item);
            let boxed = stored.units(item);
            take_out |= carried < units && boxed > 0;
            let short = units.saturating_sub(carried.saturating_add(boxed));
            if short > 0 && n < NEED_ROWS {
                needs[n] = (item, short);
                n += 1;
            }
        }
        let hearth = self.plan.is_some() && self.hearth_stands(core, &plan);
        self.survey = Survey {
            milestone,
            needs,
            needs_len: n as u8,
            // An op refused for want of support counts: only a build goal
            // tries it again, once something else stands. Past the last
            // milestone, an optional op the pack has the spare for.
            ready: self
                .pick_among(core, &plan, milestone, 0)
                .is_some_and(|i| self.tool_fits(core, &spec(i))),
            hearth,
            bill: bill.rows,
            bill_len: bill.n as u8,
            take_out,
        };
    }

    /// Its own cupboard, standing on the plot.
    pub fn hearth_addr(&self, core: &ClientCore) -> Option<OpAddr> {
        self.own_kit(core, HEARTH_ITEM)
    }

    /// Where its cupboard goes on the plot, standing or not: a removal
    /// there is its own cupboard coming down.
    pub fn hearth_spot(&self) -> Option<OpAddr> {
        let plan = self.plan?;
        (0..OPS).find_map(|i| {
            let s = spec(i);
            (s.stage.is_some() && s.op == Op::Kit(HEARTH_ITEM)).then(|| addr(&plan, &s))
        })
    }

    /// Its own box, standing in the base.
    pub fn box_addr(&self, core: &ClientCore) -> Option<OpAddr> {
        self.own_kit(core, BOX_ITEM)
    }

    /// Its workbench and furnace, standing in the base.
    pub fn stations(&self, core: &ClientCore) -> Stations {
        let at = |a: OpAddr| {
            let (x, z) = sim_core::deploy::cell_center(a.cx, a.cz);
            [x, z]
        };
        let rung = |a: Option<OpAddr>| {
            a.and_then(|b| deploy_rec(core, b))
                .and_then(|d| deploy_def(core, d.row))
                .map_or(0, |def| bench_tier(def.arch))
        };
        let (bench, bench2) = (
            self.own_kit(core, BENCH_ITEM),
            self.own_kit(core, BENCH2_ITEM),
        );
        let (one, two) = (rung(bench), rung(bench2));
        Stations {
            bench: bench.filter(|_| one > 0).map(at),
            bench2: bench2.filter(|_| two > 0).map(at),
            furnace: self.own_kit(core, FURNACE_ITEM).map(at),
            tier: one.max(two),
        }
    }

    /// The deployable this body put down for the op that places `name`,
    /// where the mirror still shows one.
    fn own_kit(&self, core: &ClientCore, name: &'static str) -> Option<OpAddr> {
        let plan = self.plan?;
        (0..OPS).find_map(|i| {
            let s = spec(i);
            let at = addr(&plan, &s);
            (s.stage.is_some()
                && s.op == Op::Kit(name)
                && self.mine & bit(i) != 0
                && deploy_at(core, at).is_some())
            .then_some(at)
        })
    }

    /// Grades that pay upkeep, counted once the stone core stands: until
    /// then every stone the pack holds is going into those walls, and a
    /// fresh grade has an upkeep period before anything rots.
    pub fn charged(&self) -> u32 {
        if self.survey.milestone > Milestone::Stone {
            self.graded()
        } else {
            0
        }
    }

    /// Grades that stand: what upkeep is charged on (twig never is).
    pub fn graded(&self) -> u32 {
        (0..OPS)
            .filter(|&i| self.done & bit(i) != 0 && matches!(spec(i).op, Op::Grade(_)))
            .count() as u32
    }

    fn hearth_stands(&self, core: &ClientCore, plan: &BasePlan) -> bool {
        (0..STARTER.len()).any(|i| {
            let s = spec(i);
            s.op == Op::Kit(HEARTH_ITEM)
                && self.mine & bit(i) != 0
                && deploy_at(core, addr(plan, &s)).is_some()
        })
    }

    /// The lowest milestone with work left. An optional op holds none back.
    fn milestone(&self) -> Milestone {
        (0..OPS)
            .filter(|&i| (self.done | self.given_up) & bit(i) == 0)
            .map(spec)
            .filter(|s| !s.optional)
            .filter_map(|s| s.stage)
            .min()
            .unwrap_or(Milestone::Done)
    }

    /// What stands on the plot is what is done: a piece at its address,
    /// its own deployable at its, a grade the piece has reached, a lock on
    /// its door and the door locked, the better deployable in a swap's
    /// place. The gear is done when the blueprint is known and the pack,
    /// the body and the box hold what was wanted. Worked out afresh from
    /// the mirror each time, never only added to: twig rots within the
    /// upkeep hour it went down in, and what rots or is broken is built
    /// again (with its grades after it); gear used up is made again.
    fn reconcile(&mut self, core: &ClientCore, plan: &BasePlan) {
        // One pass over each mirror, keeping what stands on the plot: the
        // mirror is the island's, the plot a few cells of it.
        let on_plot = |cx: u16, cz: u16| on_plot(plan, cx, cz);
        let mut stands = 0u128;
        for p in core.pieces.entries().iter().filter(|p| on_plot(p.cx, p.cz)) {
            let material = (u16::from(p.row) < core.piece_defs_have)
                .then(|| core.piece_defs.pieces[usize::from(p.row)].material);
            let here = OpAddr {
                cx: p.cx,
                cz: p.cz,
                level: p.level,
                loc: p.loc,
            };
            for i in 0..OPS {
                let s = spec(i);
                if s.stage.is_none() || addr(plan, &s) != here {
                    continue;
                }
                let done = match s.op {
                    Op::Piece(_) => true,
                    Op::Grade(m) => material.is_some_and(|have| have >= m),
                    _ => false,
                };
                if done {
                    stands |= bit(i);
                }
            }
        }
        // Only what it put down itself (the mirror does not say whose a
        // deployable is): a stranger's bag in the airlock is not its bag.
        for d in core
            .deploys
            .entries()
            .iter()
            .filter(|d| on_plot(d.cx, d.cz))
        {
            let here = OpAddr {
                cx: d.cx,
                cz: d.cz,
                level: d.level,
                loc: d.loc,
            };
            for i in 0..OPS {
                let s = spec(i);
                if s.stage.is_some()
                    && matches!(s.op, Op::Kit(_))
                    && self.mine & bit(i) != 0
                    && addr(plan, &s) == here
                {
                    stands |= bit(i);
                }
            }
        }
        // The locks' bits and the swaps are the mirror's to say; the gear,
        // the known mask's and the pack's.
        for i in 0..OPS {
            let s = spec(i);
            if s.stage.is_none() {
                continue;
            }
            let at = addr(plan, &s);
            let done = match s.op {
                Op::Lock => deploy_rec(core, at).is_some_and(|d| d.has_lock),
                Op::Code => deploy_rec(core, at).is_some_and(|d| d.locked),
                Op::Swap(_, to) => {
                    item_named(core, to).is_some_and(|t| item_at(core, at) == Some(t))
                }
                Op::Learn(name) => item_named(core, name)
                    .and_then(|item| learned(core, item))
                    .unwrap_or(false),
                Op::Make(name, want) => {
                    item_named(core, name).is_some_and(|item| self.held(core, item) >= want)
                }
                _ => continue,
            };
            if done {
                stands |= bit(i);
            }
        }
        // A swap half done: what it took down still counts as standing, so
        // the old door is not hung again in the new one's place.
        if let Some(j) = self.swapping {
            if let Op::Swap(from, _) = spec(j).op {
                let at = addr(plan, &spec(j));
                for i in 0..OPS {
                    let s = spec(i);
                    if s.op == Op::Kit(from) && addr(plan, &s) == at {
                        stands |= bit(i);
                    }
                }
            }
        }
        self.done = stands;
        // A grade whose piece was given up never comes.
        let open = !(stands | self.given_up);
        let tables = core.recipes_have >= core.recipes.recipe_count && catalog_complete(core);
        for i in (0..OPS).filter(|&i| open & bit(i) != 0) {
            let s = spec(i);
            if matches!(s.op, Op::Grade(_))
                && place_of(&s).is_some_and(|p| self.given_up & bit(p) != 0)
            {
                self.given_up |= bit(i);
            }
            // Nor a station or a piece of gear made only at one whose own
            // op was given up (the furnace, at a bench that never stood).
            if let Some(item) = self.made_item(core, &s) {
                if count(core, item) == 0 && self.unmakeable(core, item) {
                    self.given_up |= bit(i);
                }
            }
            // Gear this server has no recipe for never comes; nor what is
            // made from a blueprint whose learning was given up.
            if let Op::Learn(name) | Op::Make(name, _) = s.op {
                let none = item_named(core, name)
                    .and_then(|item| recipe_any(core, item))
                    .is_none();
                let unlearned =
                    (0..OPS).any(|j| spec(j).op == Op::Learn(name) && self.given_up & bit(j) != 0);
                if (tables && none) || unlearned {
                    self.given_up |= bit(i);
                }
            }
        }
    }

    /// Units of `item` this body holds: the pack and belt, what it wears,
    /// and the box as its panel last showed it.
    fn held(&self, core: &ClientCore, item: u16) -> u32 {
        let worn: u32 = core
            .worn
            .iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum();
        count(core, item)
            .saturating_add(worn)
            .saturating_add(self.stored.units(item))
    }

    /// What a recycle leaves whole: the units of each `salvage` item the
    /// gear still to be made takes, this milestone's or a later one's.
    pub fn kept_whole(&self, core: &ClientCore, salvage: impl Fn(u16) -> bool) -> Keep {
        let mut keep = Keep::NONE;
        for i in 0..OPS {
            let Op::Make(name, want) = spec(i).op else {
                continue;
            };
            if (self.done | self.given_up) & bit(i) != 0 {
                continue;
            }
            let Some(item) = item_named(core, name) else {
                continue;
            };
            let short = want.saturating_sub(self.held(core, item));
            let Some((r, out)) = price(core, item).filter(|_| short > 0) else {
                continue;
            };
            let times = short.div_ceil(out.max(1));
            for &(input, units) in r.rows[..r.n].iter().filter(|r| salvage(r.0)) {
                keep.add(input, units.saturating_mul(times));
            }
        }
        keep
    }

    /// The item an op makes or puts down, when it has one to make.
    fn made_item(&self, core: &ClientCore, s: &Spec) -> Option<u16> {
        match s.op {
            Op::Kit(name) | Op::Swap(_, name) | Op::Make(name, _) => item_named(core, name),
            Op::Lock => item_named(core, LOCK_ITEM),
            _ => None,
        }
    }

    /// Every recipe for `item` is made at a station of its own base whose op
    /// was given up. Nothing is concluded before the table has arrived.
    fn unmakeable(&self, core: &ClientCore, item: u16) -> bool {
        if core.recipes_have < core.recipes.recipe_count {
            return false;
        }
        let lost = |op: Op| (0..OPS).any(|j| spec(j).op == op && self.given_up & bit(j) != 0);
        let station_lost = |station: u8| match station {
            STATION_WORKBENCH1 => lost(Op::Kit(BENCH_ITEM)),
            STATION_WORKBENCH2 => lost(Op::Kit(BENCH2_ITEM)),
            STATION_FURNACE => lost(Op::Kit(FURNACE_ITEM)),
            _ => false,
        };
        let mut recipes = core.recipes.recipes
            [..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len())]
            .iter()
            .filter(|d| d.out_count > 0 && d.output == item)
            .peekable();
        recipes.peek().is_some() && recipes.all(|d| station_lost(d.station))
    }

    /// Everything the rest of `milestone` costs from the pack: pieces and
    /// grades at their price, a deployable itself where the pack or the box
    /// has one and at its recipe's price where neither does, the plan or
    /// hammer if it is missing, the junk a blueprint costs at the tree and
    /// the inputs of the gear still to make. A wood grade that a stone
    /// grade of the same piece would make pointless is not counted, nor is
    /// an optional op.
    fn bill(&self, core: &ClientCore, milestone: Milestone, stored: &Ledger) -> Bill {
        let mut bill = Bill::default();
        let mut kits: [(&str, u32); 6] = [("", 0); 6];
        let (mut plan_needed, mut hammer_needed) = (false, false);
        let mut kit = |name: &'static str| {
            if let Some(k) = kits.iter_mut().find(|k| k.0 == name || k.0.is_empty()) {
                k.0 = name;
                k.1 += 1;
            }
        };
        for i in 0..OPS {
            let s = spec(i);
            if s.stage != Some(milestone) || s.optional || (self.done | self.given_up) & bit(i) != 0
            {
                continue;
            }
            match s.op {
                Op::Piece(part) => {
                    plan_needed = true;
                    if let Some(row) = piece_row(core, part_shape(part), MAT_TWIG) {
                        bill.add_bill(&piece_bill(core, row));
                    }
                }
                Op::Grade(material) => {
                    if self.superseded(&s) {
                        continue;
                    }
                    hammer_needed = true;
                    if let Some(row) = shape_at(&s).and_then(|sh| piece_row(core, sh, material)) {
                        bill.add_bill(&piece_bill(core, row));
                    }
                }
                Op::Kit(name) | Op::Swap(_, name) => kit(name),
                Op::Lock => kit(LOCK_ITEM),
                Op::Code => {}
                Op::Learn(name) => {
                    if let Some((coin, cost, ..)) = item_named(core, name)
                        .filter(|&item| paper_slot(core, item).is_none())
                        .and_then(|item| tree_row(core, item))
                    {
                        bill.add(coin, u32::from(cost));
                    }
                }
                Op::Make(name, want) => {
                    let Some(item) = item_named(core, name) else {
                        continue;
                    };
                    let short = want.saturating_sub(self.held(core, item));
                    if let Some((r, out)) = price(core, item) {
                        bill.add_times(&r, short.div_ceil(out.max(1)));
                    }
                }
            }
        }
        for (name, wanted) in kits {
            let Some(item) = (!name.is_empty()).then(|| item_named(core, name)).flatten() else {
                continue;
            };
            let have = count(core, item).saturating_add(stored.units(item));
            if have > 0 {
                bill.add(item, wanted.min(have));
            }
            let missing = wanted.saturating_sub(have);
            if let Some((r, _)) = price(core, item) {
                bill.add_times(&r, missing);
            }
        }
        let has = self.stations(core);
        for (needed, name) in [(plan_needed, PLAN_ITEM), (hammer_needed, HAMMER_ITEM)] {
            if let Some(item) = needed.then(|| item_named(core, name)).flatten() {
                if count(core, item) == 0 {
                    if let Some(r) = recipe_bill(core, item, &has) {
                        bill.add_bill(&r);
                    }
                }
            }
        }
        bill
    }

    /// `bill`, and the optional ops of this milestone and those before it
    /// that the pack and the box between them have the spare for (a metal
    /// door's fragments, say): what a visit to the box takes out for them,
    /// and never a shortfall.
    fn with_spare(
        &self,
        core: &ClientCore,
        plan: &BasePlan,
        milestone: Milestone,
        mut bill: Bill,
    ) -> Bill {
        for i in 0..OPS {
            let s = spec(i);
            if !s.optional
                || s.stage.is_none_or(|m| m > milestone)
                || (self.done | self.given_up) & bit(i) != 0
                || !self.can_do(core, plan, &s, i)
            {
                continue;
            }
            let Some(item) = self.made_item(core, &s) else {
                continue;
            };
            let cost = if self.held(core, item) > 0 {
                let mut one = Bill::default();
                one.add(item, 1);
                one
            } else {
                match price(core, item) {
                    Some((r, _)) => r,
                    None => continue,
                }
            };
            let mut total = bill;
            total.add_bill(&cost);
            let spare = total.rows[..total.n]
                .iter()
                .all(|&(it, units)| self.held(core, it) >= units);
            if spare {
                bill = total;
            }
        }
        bill
    }

    /// A grade a higher grade of the same piece, still to come, makes
    /// pointless: wood goes where stone did not.
    fn superseded(&self, s: &Spec) -> bool {
        let Op::Grade(material) = s.op else {
            return false;
        };
        (0..STARTER.len()).any(|j| {
            let o = spec(j);
            matches!(o.op, Op::Grade(m) if m > material)
                && (o.dx, o.dz, o.level, o.loc) == (s.dx, s.dz, s.level, s.loc)
                && (self.done | self.given_up) & bit(j) == 0
        })
    }

    /// The item an op is done with in hand: the plan, the hammer, the
    /// deployable. The keypad, the tree and the crafts take none.
    fn tool(&self, core: &ClientCore, s: &Spec) -> Option<u16> {
        match s.op {
            Op::Piece(_) => item_named(core, PLAN_ITEM),
            Op::Grade(_) => item_named(core, HAMMER_ITEM),
            Op::Kit(name) | Op::Swap(_, name) => item_named(core, name),
            Op::Lock => item_named(core, LOCK_ITEM),
            Op::Code | Op::Learn(_) | Op::Make(..) => None,
        }
    }

    /// An op that is done with something in hand.
    fn handed(s: &Spec) -> bool {
        !matches!(s.op, Op::Code | Op::Learn(_) | Op::Make(..))
    }

    /// What op `s` is done with is in the pack, or would fit it once made;
    /// what it makes would fit it. A craft into a full pack is not sent
    /// (`Self::craft`).
    fn tool_fits(&self, core: &ClientCore, s: &Spec) -> bool {
        match s.op {
            Op::Make(name, _) => item_named(core, name).is_some_and(|it| {
                room_for(core, it)
                    || recipe_for(core, it, &self.stations(core))
                        .is_some_and(|(r, ..)| craft_fits(core, r, 1))
            }),
            Op::Code | Op::Learn(_) => true,
            _ => self.tool(core, s).is_some_and(|t| {
                count(core, t) > 0
                    || room_for(core, t)
                    || recipe_for(core, t, &self.stations(core))
                        .is_some_and(|(r, ..)| craft_fits(core, r, 1))
            }),
        }
    }

    /// What one op costs from the pack, including crafting what it is done
    /// with if that is not in the pack; `None` when it cannot be done here
    /// at all yet (no recipe at a station it has, a blueprint's parent not
    /// learned, no bench of the rung).
    fn op_bill(&self, core: &ClientCore, s: &Spec, has: &Stations) -> Option<Bill> {
        let mut bill = match s.op {
            Op::Piece(part) => piece_bill(core, piece_row(core, part_shape(part), MAT_TWIG)?),
            Op::Grade(material) => piece_bill(core, piece_row(core, shape_at(s)?, material)?),
            Op::Kit(_) | Op::Lock | Op::Swap(..) => Bill::default(),
            Op::Code => return Some(Bill::default()),
            Op::Learn(name) => {
                let item = item_named(core, name)?;
                if paper_slot(core, item).is_some() {
                    return Some(Bill::default());
                }
                let (coin, cost, parent, tier) = tree_row(core, item)?;
                let parent_known = parent == sim_core::research::NO_RECIPE
                    || sim_core::research::knows(core.known(), parent);
                if !parent_known || has.tier < tier {
                    return None;
                }
                let mut bill = Bill::default();
                bill.add(coin, u32::from(cost));
                return Some(bill);
            }
            Op::Make(name, _) => return recipe_bill(core, item_named(core, name)?, has),
        };
        let tool = self.tool(core, s)?;
        if count(core, tool) == 0 {
            bill.add_bill(&recipe_bill(core, tool, has)?);
        }
        Some(bill)
    }

    /// What must stand for op `i` to go, beyond its price: a grade's piece,
    /// a lock's door (its own, without one), a code's lock, a swap's first
    /// deployable (or the swap half done).
    fn can_do(&self, core: &ClientCore, plan: &BasePlan, s: &Spec, i: usize) -> bool {
        let at = addr(plan, s);
        match s.op {
            Op::Grade(_) => !self.superseded(s) && piece_at(core, at).is_some(),
            Op::Lock => {
                self.ours_at(plan, at)
                    && deploy_rec(core, at).is_some_and(|d| {
                        !d.has_lock && deploy_def(core, d.row).is_some_and(|def| lockable(def.arch))
                    })
            }
            Op::Code => {
                self.code.is_some()
                    && self.ours_at(plan, at)
                    && deploy_rec(core, at).is_some_and(|d| d.has_lock && !d.locked)
            }
            // What comes back up (the old door, and its lock) needs the
            // room: a full pack drops it at the feet.
            Op::Swap(from, _) => {
                self.swapping == Some(i)
                    || (self.ours_at(plan, at)
                        && item_named(core, from).is_some_and(|f| item_at(core, at) == Some(f))
                        && free_slots(core)
                            > usize::from(deploy_rec(core, at).is_some_and(|d| d.has_lock)))
            }
            _ => true,
        }
    }

    /// Something it put down stands at this address: a lock and a code go
    /// on its own doors and cupboard only.
    fn ours_at(&self, plan: &BasePlan, at: OpAddr) -> bool {
        (0..OPS).any(|j| {
            let o = spec(j);
            matches!(o.op, Op::Kit(_)) && self.done & bit(j) != 0 && addr(plan, &o) == at
        })
    }

    /// The first op of `milestone` not done, not waiting for support, that
    /// can go now and the pack pays for; else an optional op of it or one
    /// before it that the pack has the spare for, past what the milestone
    /// still costs.
    fn pick(&self, core: &ClientCore, plan: &BasePlan, milestone: Milestone) -> Option<usize> {
        self.pick_among(core, plan, milestone, self.deferred)
    }

    /// [`Self::pick`], passing over the ops in `skip` as well as those done
    /// or given up.
    fn pick_among(
        &self,
        core: &ClientCore,
        plan: &BasePlan,
        milestone: Milestone,
        skip: u128,
    ) -> Option<usize> {
        let has = self.stations(core);
        let open = |i: usize| (self.done | self.given_up | skip) & bit(i) == 0;
        let own = (0..OPS).find(|&i| {
            let s = spec(i);
            s.stage == Some(milestone)
                && !s.optional
                && open(i)
                && self.can_do(core, plan, &s, i)
                && self
                    .op_bill(core, &s, &has)
                    .is_some_and(|b| b.paid_by(core))
        });
        if own.is_some() {
            return own;
        }
        let reserve = self.bill(core, milestone, &self.stored);
        (0..OPS).find(|&i| {
            let s = spec(i);
            s.optional
                && s.stage.is_some_and(|m| m <= milestone)
                && open(i)
                && self.can_do(core, plan, &s, i)
                && self
                    .op_bill(core, &s, &has)
                    .is_some_and(|b| b.spare_in(core, &reserve))
        })
    }

    /// Craft `units` of `item` for op `i` (one, or a batch of the gear). A
    /// full pack drops what is made at the crafter's feet and spends the
    /// inputs either way: not sent, and no fault of the op's, so the op is
    /// not given up over it.
    fn craft(&mut self, core: &ClientCore, i: usize, item: u16, units: u16) -> Act {
        let Some((recipe, ticks, _)) = recipe_for(core, item, &self.stations(core)) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        if !craft_fits(core, recipe, units) {
            return Act::Fail(Why::PackFull);
        }
        // A full queue refuses whatever is asked: it drains on its own.
        if usize::from(core.jobs_count) >= CRAFT_QUEUE {
            return Act::Go(Intent::IDLE);
        }
        // Behind a smelt batch the unit starts when the queue drains.
        self.want = Some(Await::Craft {
            op: i,
            item,
            recipe,
            before: count(core, item),
            until: queue_wait(core) + ticks + VERDICT_TICKS,
        });
        Act::Craft {
            recipe,
            count: units.max(1),
        }
    }

    fn fail(&mut self, i: usize) {
        self.fails[i] = self.fails[i].saturating_add(1);
        if self.fails[i] >= MAX_FAILS {
            self.given_up |= bit(i);
            self.deferred &= !bit(i);
            self.stats.given_up += 1;
            if self.swapping == Some(i) {
                self.swapping = None;
            }
        }
    }

    fn succeed(&mut self, i: usize, deploy: bool, grade: bool) {
        self.done |= bit(i);
        if deploy {
            self.mine |= bit(i);
        }
        // Something new stands: what lacked support may have it now.
        self.deferred = 0;
        if grade {
            self.stats.graded += 1;
        } else if deploy {
            self.stats.deployed += 1;
        } else {
            self.stats.placed += 1;
        }
    }

    /// Op `i`'s answer was yes.
    fn answered(&mut self, i: usize, sent: Sent) {
        self.fails[i] = 0;
        match sent {
            Sent::Place => self.succeed(i, false, false),
            Sent::Grade => self.succeed(i, false, true),
            Sent::Deploy => {
                self.succeed(i, true, false);
                if self.swapping == Some(i) {
                    self.swapping = None;
                    self.stats.swapped += 1;
                }
            }
            Sent::Lock => {
                self.done |= bit(i);
                self.stats.locks += 1;
            }
            Sent::Code => {
                self.done |= bit(i);
                self.stats.codes += 1;
            }
            // The first half of a swap: the second goes up before anything
            // else is done.
            Sent::Demolish => self.swapping = Some(i),
            Sent::Unlock | Sent::Read => {
                self.done |= bit(i);
                self.stats.learned += 1;
            }
        }
    }

    /// Give this plot up: its ground or somebody's claim refused it.
    fn abandon(&mut self) {
        if let Some(p) = self.plan.take() {
            if let Some(slot) = self.bad.iter_mut().find(|b| b.is_none()) {
                *slot = Some((p.cx, p.cz));
            } else {
                self.bad.rotate_left(1);
                self.bad[BAD_PLOTS - 1] = Some((p.cx, p.cz));
            }
        }
        self.done = 0;
        self.mine = 0;
        self.deferred = 0;
        self.given_up = 0;
        self.fails = [0; OPS];
        self.swapping = None;
        self.stats.abandoned += 1;
    }

    /// Choose a plot near here: the site rules, clear of its own bags,
    /// of what it has seen others build, of trees and rocks, and of plots
    /// that were refused.
    #[allow(clippy::too_many_arguments)]
    fn choose(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        seen: &Seen,
        home: &Home,
        wood: Option<[f32; 2]>,
        stone: Option<[f32; 2]>,
    ) -> Option<BasePlan> {
        let from = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        let bad = self.bad;
        let (_, island) = core.island();
        let table = island.table;
        let avoid = |cx: u16, cz: u16| {
            bad.contains(&Some((cx, cz)))
                || (0..=2).any(|dx| (0..=1).any(|dz| home.owns(cx + dx, cz + dz, 0, LOC_PLANE)))
                || scatter_in(seed, table, haven, cx, cz)
                || !yard_goes(seed, haven, cx, cz)
        };
        let (cx, cz) = site::pick_plot(seed, haven, seen, from, wood, stone, avoid)?;
        self.stats.sites += 1;
        Some(BasePlan::new(0, cx, cz))
    }

    /// The build goal's next move.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        seen: &Seen,
        home: &Home,
        wood: Option<[f32; 2]>,
        stone: Option<[f32; 2]>,
        tick: u32,
    ) -> Act {
        let act = self.next(
            core, seed, haven, body, hands, route, seen, home, wood, stone, tick,
        );
        self.use_out = matches!(act, Act::Use { .. });
        act
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        seen: &Seen,
        home: &Home,
        wood: Option<[f32; 2]>,
        stone: Option<[f32; 2]>,
        tick: u32,
    ) -> Act {
        // A move not sent last frame is asked for again below.
        self.want = None;
        // A late answer that has not come in twice its time is not coming.
        if self
            .late
            .is_some_and(|(_, _, _, at)| tick.wrapping_sub(at) >= VERDICT_TICKS)
        {
            self.late = None;
        }
        // What the last move is waiting on.
        match self.waiting {
            Some(Await::Craft {
                op,
                item,
                recipe,
                before,
                until,
            }) => {
                let late = until == 0 || tick.wrapping_sub(until) < u32::MAX / 2;
                if count(core, item) > before {
                    self.waiting = None;
                    self.stats.crafted += 1;
                    if matches!(spec(op).op, Op::Make(..)) {
                        self.stats.made += u64::from(count(core, item) - before);
                    }
                } else if late && until != 0 && queued(core, recipe) {
                    // Still in the queue, behind longer jobs: it is coming,
                    // and no fault of the op's.
                    if let Some(Await::Craft { until, .. }) = self.waiting.as_mut() {
                        *until = tick.wrapping_add(VERDICT_TICKS);
                    }
                    return Act::Go(Intent::IDLE);
                } else if late {
                    self.waiting = None;
                    self.stats.no_answer += 1;
                    self.fail(op);
                    return Act::Fail(Why::NoAnswer);
                } else {
                    return Act::Go(Intent::IDLE);
                }
            }
            Some(Await::Belt { op, item, since }) => {
                if belt_slot(core, item).is_some() {
                    self.waiting = None;
                } else if tick.wrapping_sub(since) >= VERDICT_TICKS {
                    self.waiting = None;
                    self.fail(op);
                    return Act::Fail(Why::NoAnswer);
                } else {
                    return Act::Go(Intent::IDLE);
                }
            }
            Some(Await::Op {
                op,
                at,
                sent,
                since,
                intent,
            }) => {
                // A lock, a code, a take-down and a blueprint are answered
                // by what the mirror shows (the door's bits, the known
                // mask), not by a placement.
                if self.verdict.is_none() && self.mirror_answers(core, op, at, sent) {
                    self.verdict = Some(Verdict::Yes);
                }
                match self.verdict.take() {
                    Some(Verdict::Yes) => {
                        self.waiting = None;
                        self.answered(op, sent);
                    }
                    Some(Verdict::No { ring, reason }) => {
                        self.waiting = None;
                        self.stats.refusals += 1;
                        if let Some(end) = self.refused(core, op, ring, u32::from(reason)) {
                            return end;
                        }
                    }
                    None if tick.wrapping_sub(since) >= VERDICT_TICKS => {
                        self.waiting = None;
                        self.stats.no_answer += 1;
                        self.late = Some((op, sent.ring(), at, tick));
                        self.fail(op);
                    }
                    // The answer is on its way; the plan stays in hand.
                    None => return Act::Go(intent),
                }
            }
            None => {}
        }
        let plan = match self.plan {
            Some(p) => p,
            None => match self.choose(core, seed, haven, body, seen, home, wood, stone) {
                Some(p) => {
                    self.plan = Some(p);
                    p
                }
                None => return Act::Fail(Why::NoSpot),
            },
        };
        self.reconcile(core, &plan);
        let milestone = self.milestone();
        let began = *self.began.get_or_insert(milestone);
        if milestone > began {
            // A milestone done is where a build goal stops: the next one
            // wants another trip for materials. Not offered again until the
            // next survey says what it needs. (Back down the list, a raid's
            // or the rot's work, it builds again what it can.)
            self.survey.milestone = milestone;
            self.survey.ready = false;
            return Act::Done;
        }
        // A swap half done comes first: a doorway may stand open. One that
        // can no longer be finished (the new door lost with a death, say)
        // is let go, and the old door is hung again.
        let swap = self.swapping.filter(|&j| {
            let has = self.stations(core);
            self.op_bill(core, &spec(j), &has)
                .is_some_and(|b| b.paid_by(core))
        });
        if swap.is_none() {
            self.swapping = None;
        }
        let Some(i) = swap
            .or_else(|| self.pick(core, &plan, milestone))
            .or_else(|| {
                // Everything left was refused for support: once more each, now
                // that more stands.
                let deferred = self.deferred;
                (deferred != 0)
                    .then(|| {
                        for i in 0..OPS {
                            if deferred & bit(i) != 0 {
                                self.fail(i);
                            }
                        }
                        self.deferred = 0;
                        self.pick(core, &plan, milestone)
                    })
                    .flatten()
            })
        else {
            if milestone == Milestone::Done {
                self.survey.ready = false;
                return Act::Done;
            }
            // The pack is short: the mind sends it for materials.
            return Act::Fail(Why::MissingInputs);
        };
        let s = spec(i);
        let at = addr(&plan, &s);
        let tool = self.tool(core, &s);
        if Self::handed(&s) && tool.is_none() {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        }
        // What the op is done with, crafted from the pack first; what a
        // station makes, on the stand spot, which its stations are in
        // reach of.
        if let Some(t) = tool.filter(|&t| count(core, t) == 0) {
            let at_station = recipe_for(core, t, &self.stations(core))
                .is_some_and(|(.., station)| station != STATION_NONE);
            if !at_station {
                return self.craft(core, i, t, 1);
            }
            // A door's lock or its metal door is worked from beside the
            // door, out of a bench's reach: made on the stand spot first.
            let stand = Spec {
                op: Op::Grade(0),
                ..s
            };
            if let Some(act) =
                self.walk_to(core, seed, haven, &plan, body, hands, route, &stand, tick)
            {
                return act;
            }
            return self.craft(core, i, t, 1);
        }
        // Every op is worked from the stand spot in the core, or for a
        // door's lock from the airlock.
        if let Some(act) = self.walk_to(core, seed, haven, &plan, body, hands, route, &s, tick) {
            return act;
        }
        if let Some(t) = tool {
            if count(core, t) == 0 {
                return self.craft(core, i, t, 1);
            }
        }
        match s.op {
            Op::Make(name, want) => return self.make(core, i, name, want),
            Op::Learn(name) => {
                return self.learn(core, seed, haven, body, hands, i, name, tick);
            }
            Op::Code => return self.press(core, seed, haven, body, hands, i, at, Sent::Code, tick),
            // The first half of a swap: the old one taken back up, the new
            // one already in the pack.
            Op::Swap(from, _)
                if item_named(core, from).is_some_and(|f| item_at(core, at) == Some(f)) =>
            {
                return self.press(core, seed, haven, body, hands, i, at, Sent::Demolish, tick);
            }
            _ => {}
        }
        let Some(tool) = tool else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        // In hand, from the belt.
        let Some(slot) = belt_slot(core, tool) else {
            let Some((from, to, count)) = belt_move(core, tool) else {
                self.fail(i);
                return Act::Fail(Why::MissingInputs);
            };
            self.want = Some(Await::Belt {
                op: i,
                item: tool,
                since: 0,
            });
            return Act::Belt { from, to, count };
        };
        let intent = Intent {
            sel: Some(slot as u8),
            ..look_at(seed, haven, core, at)
        };
        let held = match self.held {
            Some((op, since)) if op == i => since,
            _ => {
                self.held = Some((i, tick));
                tick
            }
        };
        if tick.wrapping_sub(held) < HOLD_TICKS || !hands.settled() {
            return Act::Go(intent);
        }
        self.held = None;
        let act = match s.op {
            Op::Piece(part) => piece_row(core, part_shape(part), MAT_TWIG)
                .map(|row| (Act::Place { row, at, intent }, Sent::Place)),
            Op::Grade(material) => Some((
                Act::Upgrade {
                    at,
                    material,
                    intent,
                },
                Sent::Grade,
            )),
            Op::Kit(name) | Op::Swap(_, name) => kit_row(core, tool).map(|row| {
                (
                    Act::Deploy {
                        row,
                        at,
                        pose: kit_pose(name),
                        bag: name == BAG_ITEM,
                        intent,
                    },
                    Sent::Deploy,
                )
            }),
            Op::Lock => kit_row(core, tool).map(|row| {
                (
                    Act::Deploy {
                        row,
                        at,
                        pose: sim_core::footprint::Pose::CENTRE,
                        bag: false,
                        intent,
                    },
                    Sent::Lock,
                )
            }),
            Op::Code | Op::Learn(_) | Op::Make(..) => None,
        };
        let Some((act, sent)) = act else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        self.want = Some(Await::Op {
            op: i,
            at,
            sent,
            since: 0,
            intent,
        });
        act
    }

    /// Onto the spot op `s` is worked from ([`Spec::work_spot`]): through
    /// the doors by the airlock's chain to the point by it when it is on
    /// the doors' other side or round the cupboard and the box, then
    /// straight on. `None` once there.
    #[allow(clippy::too_many_arguments)]
    fn walk_to(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        plan: &BasePlan,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        s: &Spec,
        tick: u32,
    ) -> Option<Act> {
        let c = corner(plan);
        let (via, rel) = s.work_spot();
        let spot = at_corner(c, rel);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let far = (spot[0] - x).hypot(spot[1] - z);
        // After a reach refusal, onto the spot itself rather than near it.
        let close = if self.restand { WAYPOINT_M } else { STAND_M };
        if far <= close {
            // On the spot: any walk in has ended here (the chain's points
            // are the spots, and the builder stops short of the chain's
            // own radius).
            self.restand = false;
            self.approach = None;
            if self.passage.busy() {
                self.passage = Passage::IDLE;
            }
            return None;
        }
        self.held = None;
        let pointed = at_corner(c, CHAIN[via]);
        let at_via = (pointed[0] - x).hypot(pointed[1] - z) <= STAND_M;
        let region = self.region(body);
        let same_side = region == region_at(rel) && (region == Region::Outside || at_via);
        if far > STAND_M && self.walled(core) && (!same_side || self.passage.busy()) {
            // Through the airlock, or round the cupboard and the box
            // inside, by the chain, to the point by the spot.
            let last = CHAIN.len() - 1;
            let way = match region {
                Region::Outside => Way::In,
                _ => {
                    let rel_here = [x - c[0], z - c[1]];
                    let here = if region == Region::Airlock {
                        AIRLOCK_JOIN
                    } else {
                        (0..=INNER_LEG)
                            .min_by(|&a, &b| {
                                let d = |i: usize| {
                                    (CHAIN[i][0] - rel_here[0]).hypot(CHAIN[i][1] - rel_here[1])
                                };
                                d(a).total_cmp(&d(b))
                            })
                            .unwrap_or(0)
                    };
                    if via > here {
                        Way::Out
                    } else {
                        Way::In
                    }
                }
            };
            let stop = (via != 0 && via != last).then_some(via);
            let pass = self
                .passage
                .step(core, seed, haven, plan, body, hands, route, way, stop, tick);
            if pass != Pass::Done {
                return Some(self.pass_act(pass, Act::Go(Intent::IDLE)));
            }
        }
        // Routed round what stands (a wall between the body and the spot)
        // to the route's closest, then straight on.
        if far > ROUTE_STOP_M {
            match route.to(core, body, spot, ROUTE_STOP_M, true, tick) {
                step @ Step::Walk { .. } => {
                    self.approach = None;
                    return Some(Act::Go(step.walk().unwrap_or(Intent::IDLE)));
                }
                Step::Blocked => return Some(Act::Fail(Why::Stuck)),
                Step::Wait => return Some(Act::Go(Intent::IDLE)),
                Step::Arrived => {}
            }
        }
        let walk = Act::Go(Intent::walk(yaw_toward(spot[0] - x, spot[1] - z)));
        match self.approach {
            Some((best, since)) if far > best - 0.05 => {
                if tick.wrapping_sub(since) < PASSAGE_STALL_TICKS {
                    return Some(walk);
                }
                self.approach = None;
                if !self.restand {
                    return Some(Act::Fail(Why::Stuck));
                }
                // Near enough to work from, if not onto it.
                self.restand = false;
                None
            }
            _ => {
                self.approach = Some((far, tick));
                Some(walk)
            }
        }
    }

    /// Eyes on the thing at `at`, then the press once they have settled on
    /// it and it is what a player's `E` (the keypad's `L`, the take-down's
    /// key) would take from here: the code set on its lock, or the old
    /// deployable of a swap taken back up.
    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        i: usize,
        at: OpAddr,
        sent: Sent,
        tick: u32,
    ) -> Act {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let intent = Intent {
            look: Look::Point(aim_point(core, seed, haven, at, x, z)),
            ..Intent::IDLE
        };
        let held = match self.held {
            Some((op, since)) if op == i => since,
            _ => {
                self.held = Some((i, tick));
                tick
            }
        };
        let waited = tick.wrapping_sub(held);
        if waited >= HOLD_TICKS
            && hands.settled()
            && e_picks_by(core, x, z, hands.view().0, at, PRESS_SLACK_M)
        {
            let act = match (sent, self.code) {
                (Sent::Code, Some(code)) => Act::Access {
                    at,
                    op: ACCESS_OP_SET_CODE,
                    code,
                    intent,
                },
                (Sent::Demolish, _) => Act::Demolish { at, intent },
                _ => {
                    self.fail(i);
                    return Act::Fail(Why::Refused);
                }
            };
            self.held = None;
            self.want = Some(Await::Op {
                op: i,
                at,
                sent,
                since: 0,
                intent,
            });
            return act;
        }
        if waited >= VERDICT_TICKS {
            // Settled on it this long and `E` still takes something else.
            self.held = None;
            self.fail(i);
        }
        Act::Go(intent)
    }

    /// Learn a blueprint: read the paper for it when the pack holds some,
    /// else unlock it at a bench's tree. The tree is the panel a player's
    /// `E` on a bench opens, at that bench's rung: so a bench of the node's
    /// tier, eyes on it until `E` would take it from here.
    #[allow(clippy::too_many_arguments)]
    fn learn(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        i: usize,
        name: &'static str,
        tick: u32,
    ) -> Act {
        let Some(item) = item_named(core, name) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        let plan = self.plan.unwrap_or(BasePlan::new(0, 0, 0));
        let at = addr(&plan, &spec(i));
        if let Some(slot) = paper_slot(core, item) {
            self.want = Some(Await::Op {
                op: i,
                at,
                sent: Sent::Read,
                since: 0,
                intent: Intent::IDLE,
            });
            return Act::Read { slot };
        }
        let tier = tree_row(core, item).map_or(1, |(.., tier)| tier.max(1));
        let rung = |at: OpAddr| {
            deploy_rec(core, at)
                .and_then(|d| deploy_def(core, d.row))
                .map_or(0, |def| bench_tier(def.arch))
        };
        let bench = [BENCH_ITEM, BENCH2_ITEM]
            .iter()
            .filter_map(|name| self.own_kit(core, name))
            .find(|&at| rung(at) >= tier);
        let (Some(recipe), Some(bench)) = (recipe_any(core, item), bench) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let intent = Intent {
            look: Look::Point(aim_point(core, seed, haven, bench, x, z)),
            ..Intent::IDLE
        };
        let held = match self.held {
            Some((op, since)) if op == i => since,
            _ => {
                self.held = Some((i, tick));
                tick
            }
        };
        let waited = tick.wrapping_sub(held);
        if waited < HOLD_TICKS
            || !hands.settled()
            || !e_picks_by(core, x, z, hands.view().0, bench, PRESS_SLACK_M)
        {
            if waited >= VERDICT_TICKS {
                // Settled on it this long and `E` still takes something
                // else: no panel to buy the node from.
                self.held = None;
                self.fail(i);
            }
            return Act::Go(intent);
        }
        self.held = None;
        self.want = Some(Await::Op {
            op: i,
            at,
            sent: Sent::Unlock,
            since: 0,
            intent,
        });
        Act::Unlock { recipe }
    }

    /// Make gear at its station: as many as are still wanted in one go, as
    /// far as the pack pays and the queue takes. What the queue holds is
    /// coming, and waited for.
    fn make(&mut self, core: &ClientCore, i: usize, name: &'static str, want: u32) -> Act {
        let Some(item) = item_named(core, name) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        let Some((recipe, ..)) = recipe_for(core, item, &self.stations(core)) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        if queued(core, recipe) {
            return Act::Go(Intent::IDLE);
        }
        let def = core.recipes.recipes[usize::from(recipe)];
        let short = want.saturating_sub(self.held(core, item));
        let crafts = short.div_ceil(u32::from(def.out_count.max(1)));
        let paid = def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())]
            .iter()
            .filter(|&&(_, need)| need > 0)
            .map(|&(input, need)| count(core, input) / u32::from(need))
            .min()
            .unwrap_or(0);
        let units = crafts.min(paid).min(u32::from(CRAFT_COUNT_MAX)).max(1);
        self.craft(core, i, item, units as u16)
    }

    /// What the mirror shows for an op it answers: a lock on the door, the
    /// door locked, the old deployable gone, the blueprint known.
    fn mirror_answers(&self, core: &ClientCore, op: usize, at: OpAddr, sent: Sent) -> bool {
        match sent {
            Sent::Lock => deploy_rec(core, at).is_some_and(|d| d.has_lock),
            Sent::Code => deploy_rec(core, at).is_some_and(|d| d.locked),
            Sent::Demolish => deploy_rec(core, at).is_none(),
            Sent::Unlock | Sent::Read => match spec(op).op {
                Op::Learn(name) => item_named(core, name)
                    .and_then(|item| learned(core, item))
                    .unwrap_or(false),
                _ => false,
            },
            Sent::Place | Sent::Grade | Sent::Deploy => false,
        }
    }

    /// What a refusal of op `i` means for the job; `Some` ends the goal.
    fn refused(&mut self, core: &ClientCore, i: usize, ring: Ring, reason: u32) -> Option<Act> {
        use sim_core::research::{REFUSE_R_BENCH, REFUSE_R_COST, REFUSE_R_KNOWN};
        if ring == Ring::Research {
            match reason {
                REFUSE_R_COST => return Some(Act::Fail(Why::MissingInputs)),
                REFUSE_R_KNOWN => {
                    if let Some(plan) = self.plan {
                        self.reconcile(core, &plan);
                    }
                }
                REFUSE_R_BENCH => {
                    self.restand = true;
                    self.fail(i);
                }
                _ => self.fail(i),
            }
            return None;
        }
        let deploy = ring == Ring::Deploy;
        let (spot, reach, support, cost, claim, terrain) = if deploy {
            (
                reason == REFUSE_D_SPOT,
                reason == REFUSE_D_REACH,
                reason == REFUSE_D_SUPPORT,
                reason == REFUSE_D_COST,
                reason == REFUSE_D_CLAIM || reason == REFUSE_D_OVERLAP,
                reason == REFUSE_D_TERRAIN,
            )
        } else {
            (
                // A grade to where the piece already is counts as there.
                reason == REFUSE_B_SPOT || reason == REFUSE_B_TIER,
                reason == REFUSE_B_REACH,
                reason == REFUSE_B_SUPPORT,
                reason == REFUSE_B_COST,
                reason == REFUSE_B_CLAIM,
                reason == REFUSE_B_TERRAIN,
            )
        };
        if spot || (deploy && reason == REFUSE_D_HAS_LOCK) {
            // Taken. Done if what stands there is this body's own (the next
            // reconcile counts it); somebody else's is one more failure.
            if let Some(plan) = self.plan {
                self.reconcile(core, &plan);
            }
            if self.done & bit(i) == 0 {
                self.fail(i);
            }
        } else if reach {
            self.restand = true;
            self.fail(i);
        } else if support {
            self.deferred |= bit(i);
        } else if cost {
            return Some(Act::Fail(Why::MissingInputs));
        } else if claim || terrain {
            // Before its cupboard stands the plot is not yet this body's: the
            // ground or somebody's claim says to build elsewhere. After, it
            // is one op's trouble.
            let claimed = (0..STARTER.len())
                .any(|j| spec(j).op == Op::Kit(HEARTH_ITEM) && self.done & bit(j) != 0);
            if !claimed {
                self.abandon();
                return Some(Act::Fail(Why::NoSpot));
            }
            self.fail(i);
        } else {
            self.fail(i);
        }
        None
    }
}

/// The piece row standing at an address, per the mirror.
fn piece_at(core: &ClientCore, at: OpAddr) -> Option<u8> {
    core.pieces
        .entries()
        .iter()
        .find(|p| (p.cx, p.cz, p.level, p.loc) == (at.cx, at.cz, at.level, at.loc))
        .map(|p| p.row)
}

fn deploy_at(core: &ClientCore, at: OpAddr) -> Option<u8> {
    deploy_rec(core, at).map(|d| d.row)
}

/// The deployable standing at an address, per the mirror.
fn deploy_rec(core: &ClientCore, at: OpAddr) -> Option<&sim_core::deploy::DeployRec> {
    core.deploys
        .entries()
        .iter()
        .find(|d| (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc))
}

/// A deployable row's definition, once the server has sent it.
fn deploy_def(core: &ClientCore, row: u8) -> Option<sim_core::deploy::DeployDef> {
    let defs = &core.deploy_defs;
    (u16::from(row) < core.deploy_defs_have.min(defs.def_count))
        .then(|| defs.defs[usize::from(row)])
}

/// The item the deployable at an address places, per the mirror.
fn item_at(core: &ClientCore, at: OpAddr) -> Option<u16> {
    deploy_rec(core, at)
        .and_then(|d| deploy_def(core, d.row))
        .map(|def| def.item)
}

fn belt_slot(core: &ClientCore, item: u16) -> Option<usize> {
    (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item)
}

/// The whole-stack move that puts `item` on the belt: an empty belt slot,
/// else the last one (the stacks swap).
fn belt_move(core: &ClientCore, item: u16) -> Option<(u8, u8, u16)> {
    let from =
        (HOTBAR_SLOTS..INV_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item)?;
    let to = (0..HOTBAR_SLOTS)
        .find(|&i| core.inv[i].count == 0)
        .unwrap_or(HOTBAR_SLOTS - 1);
    Some((from as u8, to as u8, core.inv[from].count))
}

/// The ground behind the core takes the benches' foundations: the
/// foundation's terrain rule on both cells, clear of the edge.
fn yard_goes(seed: u64, haven: &Haven, cx: u16, cz: u16) -> bool {
    [YARD, ANNEX].iter().all(|&(dx, dz)| {
        match (
            cx.checked_add_signed(i16::from(dx)),
            cz.checked_add_signed(i16::from(dz)),
        ) {
            (Some(x), Some(z)) => site::foundation_goes(seed, haven, x, z),
            _ => false,
        }
    })
}

/// Does a tree, a rock or a bush stand where the base, or its yard behind
/// it, would? The map's scatter, which a player sees standing there.
fn scatter_in(
    seed: u64,
    table: &sim_core::terrain::ScatterTable,
    haven: &Haven,
    cx: u16,
    cz: u16,
) -> bool {
    let x0 = f32::from(cx) * BUILD_CELL_M;
    let z0 = (f32::from(cz) + f32::from(YARD.1)) * BUILD_CELL_M;
    let (x1, z1) = (
        x0 + 3.0 * BUILD_CELL_M,
        f32::from(cz) * BUILD_CELL_M + 2.0 * BUILD_CELL_M,
    );
    let pad = 1.5;
    let cell = |v: f32| (v / terrain::CELL_SIZE).floor() as i32;
    for tz in cell(z0 - pad)..=cell(z1 + pad) {
        for tx in cell(x0 - pad)..=cell(x1 + pad) {
            let slot = terrain::scatter(seed, table, haven, tx, tz);
            if slot.occupant != terrain::Occupant::None
                && (x0 - pad..=x1 + pad).contains(&slot.x)
                && (z0 - pad..=z1 + pad).contains(&slot.z)
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn the_milestones_cover_the_blueprint_in_building_order() {
        let mut last = Milestone::Shell;
        let mut seen = [0usize; Milestone::ALL.len()];
        for i in 0..OPS {
            let s = spec(i);
            let Some(m) = s.stage else {
                // Only the rent and the box the one inside replaces are
                // passed over.
                assert!(
                    matches!(
                        STARTER.get(i),
                        Some(BaseOp::Feed(..) | BaseOp::Deploy(Kit::Box, ..))
                    ),
                    "op {i} {:?} is never built",
                    STARTER.get(i)
                );
                continue;
            };
            seen[m as usize] += 1;
            // The blueprint hangs its locks and its metal door right after
            // the doors; this body does once the bench that makes them
            // stands.
            if m >= Milestone::Locks {
                assert!(
                    i >= STARTER.len()
                        || matches!(
                            STARTER[i],
                            BaseOp::Code(..) | BaseOp::Deploy(Kit::Lock | Kit::MetalDoor, ..)
                        ),
                    "op {i} {:?} waits for {m:?}",
                    STARTER.get(i)
                );
                if i >= STARTER.len() + EXTRAS.len() {
                    assert!(m >= last, "op {i} goes back to {m:?} after {last:?}");
                    last = m;
                }
                continue;
            }
            if i < STARTER.len() && m < last && m != Milestone::Stone {
                // Only the ground floor's stone grades come back down the
                // list: the blueprint grades before it grows.
                panic!("op {i} {:?} goes back to {m:?} after {last:?}", STARTER[i]);
            }
            last = last.max(m);
        }
        // The shell: two foundations, a triangle, five walls, two doorways,
        // the diagonal, and the cupboard. Two doors, the box and the bag.
        assert_eq!(seen[Milestone::Shell as usize], 12);
        assert_eq!(seen[Milestone::Doors as usize], 4);
        assert_eq!(seen[Milestone::Stone as usize], 11);
        assert!(seen[Milestone::Upstairs as usize] >= 10);
        // Its stations in reach of the stand spot, the bench on a stone
        // foundation of its own.
        assert_eq!(seen[Milestone::Bench as usize], 3);
        assert_eq!(seen[Milestone::Furnace as usize], 1);
        // A lock and its code on both doors and the cupboard, the front
        // door in metal, and the inner one when the fragments are spare.
        assert_eq!(seen[Milestone::Locks as usize], 8);
        let optional: Vec<(Op, i8, i8, u8, u8)> = (0..OPS)
            .map(spec)
            .filter(|s| s.optional)
            .map(|s| (s.op, s.dx, s.dz, s.level, s.loc))
            .collect();
        let (dx, dz, level, loc) = INNER;
        assert_eq!(
            optional,
            [(Op::Swap(DOOR_ITEM, METAL_DOOR_ITEM), dx, dz, level, loc)]
        );
        // The second bench, on a stone foundation of its own.
        assert_eq!(seen[Milestone::Bench2 as usize], 3);
        assert_eq!(seen[Milestone::Done as usize], 0);
        // Every gear milestone has work, and a blueprint is learned before
        // what it teaches is made.
        for m in Milestone::ALL.iter().filter(|m| m.gear()) {
            assert!(seen[*m as usize] > 0, "{m:?} has nothing to do");
        }
        for i in 0..OPS {
            if let Op::Make(name, _) = spec(i).op {
                if let Some(j) = (0..OPS).find(|&j| spec(j).op == Op::Learn(name)) {
                    assert!(j < i, "{name} made before it is learned");
                }
            }
        }
        let plan = BasePlan::new(0, 100, 100);
        let stand = at_corner(corner(&plan), CHAIN[0]);
        for name in [BENCH_ITEM, FURNACE_ITEM, BENCH2_ITEM] {
            let s = (0..OPS).map(spec).find(|s| s.op == Op::Kit(name)).unwrap();
            let at = addr(&plan, &s);
            let (x, z) = sim_core::deploy::cell_center(at.cx, at.cz);
            let d = (x - stand[0]).hypot(z - stand[1]);
            assert!(d <= STATION_RADIUS_M - STATION_SLACK_M, "{name} {d} m off");
            assert!(d <= BUILD_REACH_M, "{name} out of reach");
        }
    }

    #[test]
    fn the_airlock_walk_keeps_to_its_regions() {
        let plan = BasePlan::new(0, 100, 100);
        let c = corner(&plan);
        let stand = at_corner(c, CHAIN[0]);
        let (sx, sz) = (
            f32::from(plan.cx) * BUILD_CELL_M + STARTER_STAND_M.0,
            f32::from(plan.cz) * BUILD_CELL_M + STARTER_STAND_M.1,
        );
        assert!((stand[0] - sx).abs() < 1e-4 && (stand[1] - sz).abs() < 1e-4);
        for (i, p) in CHAIN.iter().enumerate() {
            let want = if i <= INNER_LEG {
                Region::Room
            } else if i < FRONT_LEG + 1 {
                Region::Airlock
            } else {
                Region::Outside
            };
            assert_eq!(region_at(*p), want, "chain point {i}");
        }
        // Each door stands across its leg: the inner one's edge between
        // points 3 and 4, the front one's between 6 and 7.
        assert!(CHAIN[INNER_LEG][1] < 0.0 && CHAIN[INNER_LEG + 1][1] > 0.0);
        assert!(CHAIN[FRONT_LEG][0] > 0.0 && CHAIN[FRONT_LEG + 1][0] < 0.0);
        // Both doors' cell is in use reach of both ends of each leg.
        let (dx, dz) = anchor(plan.cx + 1, plan.cz + 1, LOC_PLANE);
        for i in [INNER_LEG, INNER_LEG + 1, FRONT_LEG, FRONT_LEG + 1] {
            let p = at_corner(c, CHAIN[i]);
            assert!((p[0] - dx).hypot(p[1] - dz) < sim_core::build::BUILD_REACH_M - 1.0);
        }
    }

    fn stream(
        core: &mut ClientCore,
        encode: impl FnOnce(&mut [u8]) -> Result<usize, protocol::WireError>,
    ) {
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = encode(&mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
    }

    /// Twig rots within the upkeep hour it went down in, and a raid breaks
    /// what it likes: what is gone from the mirror is not done any more,
    /// and the base goes back to the milestone that builds it.
    #[test]
    fn what_rots_away_is_built_again() {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let plan = BasePlan::new(0, 100, 100);
        let mut b = Builder::new();
        b.plan = Some(plan);
        for i in (0..OPS).filter(|&i| spec(i).stage == Some(Milestone::Shell)) {
            let at = addr(&plan, &spec(i));
            if matches!(spec(i).op, Op::Kit(_)) {
                b.succeed(i, true, false);
                let rec = sim_core::deploy::DeployRec {
                    cx: at.cx,
                    cz: at.cz,
                    level: at.level,
                    loc: at.loc,
                    ..Default::default()
                };
                stream(&mut core, |buf| {
                    protocol::event::encode_event_deploy_placed(&rec, buf)
                });
            } else {
                let rec = sim_core::build::PieceRec {
                    cx: at.cx,
                    cz: at.cz,
                    level: at.level,
                    loc: at.loc,
                    ..Default::default()
                };
                stream(&mut core, |buf| {
                    protocol::event::encode_event_piece_placed(&rec, buf)
                });
            }
        }
        b.survey_now(&core, &Ledger::EMPTY);
        assert_eq!(b.survey().milestone, Milestone::Doors);
        assert!(b.survey().hearth);
        let wall = (0..OPS)
            .find(|&i| spec(i).op == Op::Piece(Part::Wall))
            .unwrap();
        let at = addr(&plan, &spec(wall));
        stream(&mut core, |buf| {
            protocol::event::encode_event_removed(true, at.cx, at.cz, at.level, at.loc, buf)
        });
        b.survey_now(&core, &Ledger::EMPTY);
        assert_eq!(b.survey().milestone, Milestone::Shell);
        assert_eq!(b.done & bit(wall), 0, "the wall is built again");

        // A deployable at the bag's address that this body did not put
        // down is not its bag.
        let bag = STARTER.len() + 1;
        assert_eq!(spec(bag).op, Op::Kit(BAG_ITEM));
        let at = addr(&plan, &spec(bag));
        let rec = sim_core::deploy::DeployRec {
            cx: at.cx,
            cz: at.cz,
            level: at.level,
            loc: at.loc,
            ..Default::default()
        };
        stream(&mut core, |buf| {
            protocol::event::encode_event_deploy_placed(&rec, buf)
        });
        b.survey_now(&core, &Ledger::EMPTY);
        assert_eq!(b.done & bit(bag), 0);
    }

    /// A craft into a full pack spills what it makes and spends the inputs:
    /// it is not sent, and the op is not given up over it. Room in the pack
    /// (an empty slot) sends it.
    #[test]
    fn a_full_pack_crafts_nothing_and_gives_up_on_nothing() {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let tool = 9;
        core.catalog.count = 10;
        core.catalog
            .set(
                usize::from(tool),
                b"Hammer",
                protocol::ItemRow {
                    stack_max: 1,
                    ..protocol::ItemRow::EMPTY
                },
            )
            .unwrap();
        (core.recipes.recipe_count, core.recipes_have) = (1, 1);
        core.recipes.recipes[0] = sim_core::craft::RecipeDef {
            output: tool,
            out_count: 1,
            ticks: 10,
            station: STATION_NONE,
            blueprint: false,
            n_inputs: 1,
            inputs: [(5, 100), (0, 0), (0, 0), (0, 0)],
        };
        core.inv.fill(sim_core::gather::ItemStack {
            item: 6,
            count: 1,
            cond: 0,
            skin: 0,
        });
        let mut b = Builder::new();
        for _ in 0..MAX_FAILS + 1 {
            assert!(matches!(
                b.craft(&core, 0, tool, 1),
                Act::Fail(Why::PackFull)
            ));
        }
        assert_eq!((b.fails[0], b.given_up), (0, 0));
        assert!(b.want.is_none(), "nothing sent, nothing awaited");
        // A full pack whose inputs empty a slot when the queue takes them
        // has the room after all; one whose inputs leave every slot full,
        // not.
        core.inv[3] = sim_core::gather::ItemStack {
            item: 5,
            count: 150,
            cond: 0,
            skin: 0,
        };
        assert!(!craft_fits(&core, 0, 1), "50 of 150 left in the slot");
        core.inv[3].count = 100;
        assert!(craft_fits(&core, 0, 1), "the slot the inputs leave empty");
        core.inv[3] = core.inv[4];
        core.inv[INV_SLOTS - 1].count = 0;
        assert!(matches!(
            b.craft(&core, 0, tool, 1),
            Act::Craft {
                recipe: 0,
                count: 1
            }
        ));
        assert!(matches!(b.want, Some(Await::Craft { op: 0, .. })));
    }

    /// The furnace is made at the bench: once the bench is given up, so is
    /// the furnace, and the milestones move past it rather than stall.
    #[test]
    fn a_station_made_at_a_bench_given_up_is_given_up_too() {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let furnace_item = 9;
        core.catalog.count = 10;
        core.catalog
            .set(
                usize::from(furnace_item),
                FURNACE_ITEM.as_bytes(),
                protocol::ItemRow {
                    stack_max: 1,
                    ..protocol::ItemRow::EMPTY
                },
            )
            .unwrap();
        (core.recipes.recipe_count, core.recipes_have) = (1, 1);
        core.recipes.recipes[0] = sim_core::craft::RecipeDef {
            output: furnace_item,
            out_count: 1,
            ticks: 10,
            station: STATION_WORKBENCH1,
            blueprint: false,
            n_inputs: 1,
            inputs: [(5, 100), (0, 0), (0, 0), (0, 0)],
        };
        let plan = BasePlan::new(0, 100, 100);
        let mut b = Builder::new();
        b.plan = Some(plan);
        let op = |name| (0..OPS).find(|&i| spec(i).op == Op::Kit(name)).unwrap();
        let (bench, furnace) = (op(BENCH_ITEM), op(FURNACE_ITEM));
        b.reconcile(&core, &plan);
        assert_eq!(b.given_up & bit(furnace), 0, "the bench may still come");
        b.given_up |= bit(bench);
        b.reconcile(&core, &plan);
        assert_ne!(b.given_up & bit(furnace), 0);
        assert_ne!(b.milestone(), Milestone::Furnace);
    }

    /// A refusal that comes after its op was given up on (three seconds of
    /// nothing) is that op's, not the next one's of the same kind.
    #[test]
    fn a_late_refusal_is_not_the_next_ops() {
        let mut b = Builder::new();
        let at = OpAddr {
            cx: 1,
            cz: 1,
            level: 0,
            loc: LOC_PLANE,
        };
        b.late = Some((1, Ring::Deploy, at, 0));
        b.waiting = Some(Await::Op {
            op: 2,
            at,
            sent: Sent::Deploy,
            since: 10,
            intent: Intent::IDLE,
        });
        b.on_refused(true, REFUSE_D_SPOT as u8);
        assert_eq!(b.verdict, None, "the late op's");
        assert_eq!(b.late, None);
        b.on_refused(true, REFUSE_D_SPOT as u8);
        assert!(matches!(b.verdict, Some(Verdict::No { .. })), "this op's");
    }

    /// Doors are pressed only where a player's `E` would take that door:
    /// from each point the airlock walk presses one, the other door of the
    /// cell is never the pick.
    #[test]
    fn the_airlock_doors_are_pressed_where_e_picks_them() {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let plan = BasePlan::new(0, 100, 100);
        let c = corner(&plan);
        for d in [FRONT, INNER] {
            let at = door_addr(&plan, d);
            let rec = sim_core::deploy::DeployRec {
                cx: at.cx,
                cz: at.cz,
                level: at.level,
                loc: at.loc,
                ..Default::default()
            };
            stream(&mut core, |buf| {
                protocol::event::encode_event_deploy_placed(&rec, buf)
            });
        }
        let yaw_at = |from: [f32; 2], to: (f32, f32)| {
            let (dx, dz) = (to.0 - from[0], to.1 - from[1]);
            yaw_toward(dx, dz)
        };
        // Opened from one end of its leg and shut from the other.
        for (door, points) in [
            (INNER, [INNER_LEG, INNER_LEG + 1]),
            (FRONT, [FRONT_LEG, FRONT_LEG + 1]),
        ] {
            let at = door_addr(&plan, door);
            let (ax, az) = anchor(at.cx, at.cz, at.loc);
            for i in points {
                let p = at_corner(c, CHAIN[i]);
                let yaw = yaw_at(p, (ax, az));
                assert!(
                    e_picks(&core, p[0], p[1], yaw, at),
                    "{door:?} from point {i}"
                );
                let other = door_addr(&plan, if door == FRONT { INNER } else { FRONT });
                assert!(!e_picks(&core, p[0], p[1], yaw, other));
            }
        }
    }

    /// The bench on the floor over the cupboard scores exactly where the
    /// cupboard does: `E` takes the cupboard, as the human client's
    /// tiebreak has it, and never the bench.
    #[test]
    fn the_cupboard_under_the_bench_is_still_e_s_pick() {
        use sim_core::deploy::{ARCH_HEARTH, ARCH_WORKBENCH};
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        for (row, arch) in [(0usize, ARCH_HEARTH), (1, ARCH_WORKBENCH)] {
            core.deploy_defs.defs[row].arch = arch;
            core.deploy_defs.defs[row].hp = 100;
        }
        core.deploy_defs.def_count = 2;
        core.deploy_defs_have = 2;
        let at = |level: u8| OpAddr {
            cx: 100,
            cz: 100,
            level,
            loc: LOC_PLANE,
        };
        for (level, row) in [(0u8, 0u8), (1, 1)] {
            let a = at(level);
            let rec = sim_core::deploy::DeployRec {
                cx: a.cx,
                cz: a.cz,
                level: a.level,
                loc: a.loc,
                row,
                ..Default::default()
            };
            stream(&mut core, |buf| {
                protocol::event::encode_event_deploy_placed(&rec, buf)
            });
        }
        let (x, z) = sim_core::deploy::cell_center(100, 100);
        let (px, pz) = (x, z - 1.0);
        let yaw = yaw_toward(x - px, z - pz);
        assert!(e_picks(&core, px, pz, yaw, at(0)), "the cupboard");
        assert!(!e_picks(&core, px, pz, yaw, at(1)), "never the bench");
    }

    #[test]
    fn every_op_is_in_reach_of_the_stand_spot() {
        let plan = BasePlan::new(0, 100, 100);
        let stand = at_corner(corner(&plan), CHAIN[0]);
        for i in 0..OPS {
            let s = spec(i);
            if s.stage.is_none() {
                continue;
            }
            let at = addr(&plan, &s);
            let (ax, az) = match s.op {
                Op::Kit(_) => sim_core::deploy::cell_center(at.cx, at.cz),
                _ => anchor(at.cx, at.cz, at.loc),
            };
            let d = (ax - stand[0]).hypot(az - stand[1]);
            assert!(
                d + STAND_M < sim_core::build::BUILD_REACH_M,
                "op {i} at {d} m"
            );
        }
    }
}
