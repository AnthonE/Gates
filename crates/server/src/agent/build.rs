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
//! for what the next one needs ([`Survey::needs`]). Locks, metal doors and
//! the upkeep feed wait for a workbench (lane C) and are passed over.
//!
//! What it knows is its own: the plot it chose, what it built there (the
//! client's mirror of its own base, which it stands in), and the game's
//! prices from the tables the server sent. Getting in and out through its
//! own doors is [`Passage`], a fixed walk through the airlock that opens
//! each door on the way and shuts it behind.

use crate::agent::hands::Hands;
use crate::agent::home::{Home, HOLD_TICKS};
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::site::{self, Seen};
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
use sim_core::craft::STATION_NONE;
use sim_core::deploy::{
    arch_is_door, REFUSE_D_CLAIM, REFUSE_D_COST, REFUSE_D_OVERLAP, REFUSE_D_REACH, REFUSE_D_SPOT,
    REFUSE_D_SUPPORT, REFUSE_D_TERRAIN,
};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, MAX_ITEM_DEFS, TICK_HZ};
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
    /// Everything this body can build without a workbench.
    Done,
}

impl Milestone {
    pub const ALL: [Milestone; 6] = [
        Milestone::Shell,
        Milestone::Doors,
        Milestone::Stone,
        Milestone::Upstairs,
        Milestone::Wood,
        Milestone::Done,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Milestone::Shell => "cupboard_and_twig_shell",
            Milestone::Doors => "doors_bag_and_box",
            Milestone::Stone => "stone_core",
            Milestone::Upstairs => "upstairs",
            Milestone::Wood => "wood_grades",
            Milestone::Done => "done",
        }
    }
}

/// Deployables by the catalog name a player knows them by: the rows are
/// the server's, found by the item each one places.
pub const HEARTH_ITEM: &str = "Hearth";
pub const DOOR_ITEM: &str = "Wooden Door";
pub const BOX_ITEM: &str = "Small Box";
/// What a player holds to place a piece, and to grade one.
pub const PLAN_ITEM: &str = "Building Plan";
pub const HAMMER_ITEM: &str = "Hammer";

/// What the agent adds to the blueprint for its own use: the bag it wakes
/// on and the box it keeps things in, both behind the front door from the
/// second milestone on. The box stands in the stair cell, the one plane
/// inside the room the cupboard does not take (a box is solid, and the
/// airlock's walk passes beside it); the bag in the airlock, where a
/// walk-over mat blocks nobody.
const EXTRAS: [(&str, i8, i8); 2] = [(BOX_ITEM, 1, 0), (BAG_ITEM, 1, 1)];
/// Every op the builder knows: the blueprint's, then its own.
pub const OPS: usize = STARTER.len() + EXTRAS.len();
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
pub const BILL_ROWS: usize = 6;
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
}

#[derive(Clone, Copy, Debug)]
struct Spec {
    op: Op,
    dx: i8,
    dz: i8,
    level: u8,
    loc: u8,
    /// `None`: not this body's to build yet (locks, metal doors, the feed,
    /// the blueprint's upstairs box, which the box inside replaces).
    stage: Option<Milestone>,
}

/// Op `i`: the blueprint's in order, then [`EXTRAS`].
fn spec(i: usize) -> Spec {
    if let Some(&(name, dx, dz)) = i.checked_sub(STARTER.len()).and_then(|k| EXTRAS.get(k)) {
        return Spec {
            op: Op::Kit(name),
            dx,
            dz,
            level: 0,
            loc: LOC_PLANE,
            stage: Some(Milestone::Doors),
        };
    }
    let none = |dx, dz, level, loc| Spec {
        op: Op::Grade(0),
        dx,
        dz,
        level,
        loc,
        stage: None,
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
        },
        BaseOp::Deploy(kit, dx, dz, level, loc) => {
            let (name, stage) = match kit {
                Kit::Hearth => (HEARTH_ITEM, Some(Milestone::Shell)),
                Kit::Door => (DOOR_ITEM, Some(Milestone::Doors)),
                Kit::Box => (BOX_ITEM, None),
                Kit::MetalDoor | Kit::Lock => ("", None),
            };
            Spec {
                op: Op::Kit(name),
                dx,
                dz,
                level,
                loc,
                stage,
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
        },
        BaseOp::Code(dx, dz, level, loc) => none(dx, dz, level, loc),
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

/// The blueprint's piece shape at an address, for a grade.
fn shape_at(s: &Spec) -> Option<u8> {
    STARTER.iter().find_map(|op| match *op {
        BaseOp::Place(part, dx, dz, level, loc)
            if (dx, dz, level, loc) == (s.dx, s.dz, s.level, s.loc) =>
        {
            Some(part_shape(part))
        }
        _ => None,
    })
}

/// The blueprint's Place op at a grade's address.
fn place_of(s: &Spec) -> Option<usize> {
    STARTER.iter().position(|op| {
        matches!(*op, BaseOp::Place(_, dx, dz, level, loc)
            if (dx, dz, level, loc) == (s.dx, s.dz, s.level, s.loc))
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
        for &(item, units) in &other.rows[..other.n] {
            self.add(item, units);
        }
    }

    fn paid_by(&self, core: &ClientCore) -> bool {
        self.rows[..self.n]
            .iter()
            .all(|&(item, units)| count(core, item) >= units)
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

/// The item a catalog name means on this server.
pub fn item_named(core: &ClientCore, name: &str) -> Option<u16> {
    (0..usize::from(core.catalog.count).min(MAX_ITEM_DEFS))
        .find(|&i| core.catalog.name(i) == name.as_bytes())
        .map(|i| i as u16)
}

/// The recipe that makes `item` with no station and no blueprint missing,
/// and how long one takes.
fn recipe_for(core: &ClientCore, item: u16) -> Option<(u16, u32)> {
    if core.recipes_have < core.recipes.recipe_count {
        return None;
    }
    let known = core.known();
    (0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len())).find_map(|r| {
        let def = core.recipes.recipes[r];
        let usable = def.out_count > 0
            && def.output == item
            && def.station == STATION_NONE
            && (!def.blueprint || (r < 64 && known & (1 << r) != 0));
        usable.then_some((r as u16, def.ticks))
    })
}

/// What crafting one `item` takes from the pack.
fn recipe_bill(core: &ClientCore, item: u16) -> Option<Bill> {
    let (r, _) = recipe_for(core, item)?;
    let def = core.recipes.recipes[usize::from(r)];
    let mut bill = Bill::default();
    for &(input, need) in &def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())] {
        bill.add(input, u32::from(need));
    }
    Some(bill)
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
    /// Out: the last point is reached, and the walk ends once it is shut.
    out: bool,
    /// A use sent at this door, wanting it open or shut, and when.
    door: Option<(OpAddr, bool, u32)>,
    use_ready: bool,
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
        door: None,
        use_ready: false,
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

    /// A door that would not answer, or that `E` would not pick from
    /// here: one more try, and the walk fails after `MAX_FAILS`.
    fn tried(&mut self) -> Option<Pass> {
        self.tries += 1;
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
        tick: u32,
    ) -> Pass {
        let corner = corner(plan);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.way != Some(way) {
            *self = Passage {
                way: Some(way),
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

/// Would a player standing at (`x`, `z`) and facing wire yaw `yaw` press
/// `E` on this door? The human client's pick (`client::ui::interact::
/// resolve`), asked conservatively: the door is on the aim line and nearer
/// than anything else there that `E` could take (every deployable in
/// reach, a door scored where it hangs and the rest at its cell's centre,
/// and every backpack). A dead tie counts as not picked.
fn e_picks(core: &ClientCore, x: f32, z: f32, yaw: u16, door: OpAddr) -> bool {
    let (fx, fz) = sim_core::yaw_dir(yaw);
    let reach2 = BUILD_REACH_M * BUILD_REACH_M;
    // (aimed, squared distance) of a point, from here.
    let score = |px: f32, pz: f32| {
        let (dx, dz) = (px - x, pz - z);
        let t = dx * fx + dz * fz;
        let (ox, oz) = (dx - t * fx, dz - t * fz);
        (
            t > 0.0 && ox * ox + oz * oz <= E_AIM_RADIUS_M * E_AIM_RADIUS_M,
            dx * dx + dz * dz,
        )
    };
    let in_reach = |cx: u16, cz: u16| {
        let (cx, cz) = sim_core::deploy::cell_center(cx, cz);
        (cx - x) * (cx - x) + (cz - z) * (cz - z) <= reach2
    };
    if !in_reach(door.cx, door.cz) {
        return false;
    }
    let (ax, az) = anchor(door.cx, door.cz, door.loc);
    let (aimed, mine) = score(ax, az);
    if !aimed || mine > reach2 {
        return false;
    }
    let deploys = core.deploys.entries().iter().filter(|d| {
        (d.cx, d.cz, d.level, d.loc) != (door.cx, door.cz, door.level, door.loc)
            && in_reach(d.cx, d.cz)
    });
    let rivals = deploys.map(|d| anchor(d.cx, d.cz, d.loc)).chain(
        core.bags
            .entries()
            .iter()
            .map(|b| (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q)),
    );
    for (px, pz) in rivals {
        let (aimed, d2) = score(px, pz);
        if aimed && d2 <= mine && d2 <= reach2 {
            return false;
        }
    }
    true
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
fn look_point(seed: u64, haven: &Haven, core: &ClientCore, at: OpAddr) -> [f32; 3] {
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

fn look_at(seed: u64, haven: &Haven, core: &ClientCore, at: OpAddr) -> Intent {
    Intent {
        look: Look::Point(look_point(seed, haven, core, at)),
        ..Intent::IDLE
    }
}

/// What the builder wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Go(Intent),
    /// Craft one of what the next op needs (`encode_action_craft`).
    Craft {
        recipe: u16,
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
    Done,
    Fail(Why),
}

/// What the server said about the op in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Yes,
    /// A refusal, and whether it came from the deploy ring.
    No {
        deploy: bool,
        reason: u8,
    },
}

/// What the builder is waiting on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Await {
    /// A craft of `item` for op `op`, with this many in the pack before,
    /// done by then.
    Craft {
        op: usize,
        item: u16,
        before: u32,
        until: u32,
    },
    /// A stack of `item` on its way to the belt, for op `op`.
    Belt { op: usize, item: u16, since: u32 },
    /// Op `op` at this address, holding this intent until the answer.
    Op {
        op: usize,
        at: OpAddr,
        deploy: bool,
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
}

impl Survey {
    pub fn needs(&self) -> &[(u16, u32)] {
        &self.needs[..usize::from(self.needs_len)]
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
    late: Option<(usize, bool, OpAddr, u32)>,
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
            began: None,
            passage: Passage::IDLE,
            use_out: false,
            survey: Survey {
                milestone: Milestone::Shell,
                needs: [(0, 0); NEED_ROWS],
                needs_len: 0,
                ready: false,
                hearth: false,
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
            },
        }
    }

    /// A new session: the plot is chosen again (the mirror still shows
    /// what stands, so what was built is not built twice).
    pub fn reset(&mut self) {
        let stats = self.stats;
        *self = Self::new();
        self.stats = stats;
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

    /// Is this address part of its own base?
    pub fn owns(&self, cx: u16, cz: u16, _level: u8, _loc: u8) -> bool {
        self.plan
            .is_some_and(|p| (p.cx..=p.cx + 2).contains(&cx) && (p.cz..=p.cz + 1).contains(&cz))
    }

    /// Where the body stands in its base.
    pub fn region(&self, body: &EntityState) -> Region {
        let Some(plan) = self.plan else {
            return Region::Outside;
        };
        let c = corner(&plan);
        region_at([
            body.qx as f32 * POS_XZ_Q - c[0],
            body.qz as f32 * POS_XZ_Q - c[1],
        ])
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

    /// A walk through the doors is under way.
    pub fn passing(&self) -> bool {
        self.passage.busy()
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
        let act = match self
            .passage
            .step(core, seed, haven, &plan, body, hands, route, way, tick)
        {
            Pass::Go(i) => Act::Go(i),
            Pass::Use(at, intent) => Act::Use { at, intent },
            Pass::Done => Act::Done,
            Pass::Fail(why) => Act::Fail(why),
        };
        self.use_out = matches!(act, Act::Use { .. });
        act
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
        if let Some((op, d, at, _)) = self.late {
            if (at.cx, at.cz, at.level, at.loc) == (cx, cz, level, loc) && d == deploy {
                self.late = None;
                if deploy {
                    self.mine |= bit(op);
                }
            }
        }
        if let Some(Await::Op { at, deploy: d, .. }) = self.waiting {
            if (at.cx, at.cz, at.level, at.loc) == (cx, cz, level, loc) && d == deploy {
                self.verdict = Some(Verdict::Yes);
            }
        }
    }

    /// A build or deploy refusal: the answer when that kind is in flight
    /// (the rings carry only this player's own), unless an op of that kind
    /// timed out and its answer is still owed: a refusal carries no
    /// address, and this one is most likely the late op's.
    pub fn on_refused(&mut self, deploy: bool, reason: u8) {
        if self.late.is_some_and(|(_, d, _, _)| d == deploy) {
            self.late = None;
            return;
        }
        if let Some(Await::Op { deploy: d, .. }) = self.waiting {
            if d == deploy && self.verdict.is_none() {
                self.verdict = Some(Verdict::No { deploy, reason });
            }
        }
    }

    /// The craft queue said no.
    pub fn on_craft_refused(&mut self) {
        if let Some(Await::Craft { until, .. }) = self.waiting.as_mut() {
            *until = 0;
        }
    }

    /// Bring `done` up to what the mirror shows standing on the plot, and
    /// work out the next milestone and what it needs. Once a second.
    pub fn survey_now(&mut self, core: &ClientCore) {
        // Before a plot is chosen the work is the whole blueprint, and its
        // price does not depend on where.
        let plan = self.plan.unwrap_or(BasePlan::new(0, 0, 0));
        if self.plan.is_some() {
            self.reconcile(core, &plan);
        }
        let milestone = self.milestone();
        let mut needs = [(0u16, 0u32); NEED_ROWS];
        let mut n = 0;
        let bill = self.bill(core, milestone);
        for &(item, units) in &bill.rows[..bill.n] {
            let short = units.saturating_sub(count(core, item));
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
            // tries it again, once something else stands.
            ready: milestone != Milestone::Done
                && self.pick_among(core, &plan, milestone, 0).is_some(),
            hearth,
        };
    }

    fn hearth_stands(&self, core: &ClientCore, plan: &BasePlan) -> bool {
        (0..STARTER.len()).any(|i| {
            let s = spec(i);
            s.op == Op::Kit(HEARTH_ITEM)
                && self.mine & bit(i) != 0
                && deploy_at(core, addr(plan, &s)).is_some()
        })
    }

    /// The lowest milestone with work left.
    fn milestone(&self) -> Milestone {
        (0..OPS)
            .filter(|&i| (self.done | self.given_up) & bit(i) == 0)
            .filter_map(|i| spec(i).stage)
            .min()
            .unwrap_or(Milestone::Done)
    }

    /// What stands on the plot is what is done: a piece at its address,
    /// its own deployable at its, a grade the piece has reached. Worked out
    /// afresh from the mirror each time, never only added to: twig rots
    /// within the upkeep hour it went down in, and what rots or is broken
    /// is built again (with its grades after it).
    fn reconcile(&mut self, core: &ClientCore, plan: &BasePlan) {
        // One pass over each mirror, keeping what stands on the plot: the
        // mirror is the island's, the plot a few cells of it.
        let on_plot =
            |cx: u16, cz: u16| cx.wrapping_sub(plan.cx) <= 2 && cz.wrapping_sub(plan.cz) <= 1;
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
                    Op::Kit(_) => false,
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
        self.done = stands;
        // A grade whose piece was given up never comes.
        let open = !(stands | self.given_up);
        for i in (0..OPS).filter(|&i| open & bit(i) != 0) {
            let s = spec(i);
            if matches!(s.op, Op::Grade(_))
                && place_of(&s).is_some_and(|p| self.given_up & bit(p) != 0)
            {
                self.given_up |= bit(i);
            }
        }
    }

    /// Everything the rest of `milestone` costs from the pack: pieces and
    /// grades at their price, a deployable not in the pack at its recipe's,
    /// and the plan or hammer if it is missing. A wood grade that a stone
    /// grade of the same piece would make pointless is not counted.
    fn bill(&self, core: &ClientCore, milestone: Milestone) -> Bill {
        let mut bill = Bill::default();
        let mut kits: [(&str, u32); 4] = [("", 0); 4];
        let (mut plan_needed, mut hammer_needed) = (false, false);
        for i in 0..OPS {
            let s = spec(i);
            if s.stage != Some(milestone) || (self.done | self.given_up) & bit(i) != 0 {
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
                Op::Kit(name) => {
                    if let Some(k) = kits.iter_mut().find(|k| k.0 == name || k.0.is_empty()) {
                        k.0 = name;
                        k.1 += 1;
                    }
                }
            }
        }
        for (name, wanted) in kits {
            let Some(item) = (!name.is_empty()).then(|| item_named(core, name)).flatten() else {
                continue;
            };
            let missing = wanted.saturating_sub(count(core, item));
            if let Some(r) = recipe_bill(core, item) {
                for _ in 0..missing {
                    bill.add_bill(&r);
                }
            }
        }
        for (needed, name) in [(plan_needed, PLAN_ITEM), (hammer_needed, HAMMER_ITEM)] {
            if let Some(item) = needed.then(|| item_named(core, name)).flatten() {
                if count(core, item) == 0 {
                    if let Some(r) = recipe_bill(core, item) {
                        bill.add_bill(&r);
                    }
                }
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

    /// The item an op is done with in hand.
    fn tool(&self, core: &ClientCore, s: &Spec) -> Option<u16> {
        match s.op {
            Op::Piece(_) => item_named(core, PLAN_ITEM),
            Op::Grade(_) => item_named(core, HAMMER_ITEM),
            Op::Kit(name) => item_named(core, name),
        }
    }

    /// What one op costs from the pack, including crafting what it is done
    /// with if that is not in the pack.
    fn op_bill(&self, core: &ClientCore, s: &Spec) -> Option<Bill> {
        let mut bill = match s.op {
            Op::Piece(part) => piece_bill(core, piece_row(core, part_shape(part), MAT_TWIG)?),
            Op::Grade(material) => piece_bill(core, piece_row(core, shape_at(s)?, material)?),
            Op::Kit(_) => Bill::default(),
        };
        let tool = self.tool(core, s)?;
        if count(core, tool) == 0 {
            bill.add_bill(&recipe_bill(core, tool)?);
        }
        Some(bill)
    }

    /// The first op of `milestone` not done, not waiting for support, that
    /// the pack pays for and whose piece stands (a grade).
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
        (0..OPS).find(|&i| {
            let s = spec(i);
            s.stage == Some(milestone)
                && (self.done | self.given_up | skip) & bit(i) == 0
                && (!matches!(s.op, Op::Grade(_))
                    || (!self.superseded(&s) && piece_at(core, addr(plan, &s)).is_some()))
                && self.op_bill(core, &s).is_some_and(|b| b.paid_by(core))
        })
    }

    fn fail(&mut self, i: usize) {
        self.fails[i] = self.fails[i].saturating_add(1);
        if self.fails[i] >= MAX_FAILS {
            self.given_up |= bit(i);
            self.deferred &= !bit(i);
            self.stats.given_up += 1;
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
                before,
                until,
            }) => {
                if count(core, item) > before {
                    self.waiting = None;
                    self.stats.crafted += 1;
                } else if until == 0 || tick.wrapping_sub(until) < u32::MAX / 2 {
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
                deploy,
                since,
                intent,
            }) => {
                let grade = matches!(spec(op).op, Op::Grade(_));
                match self.verdict.take() {
                    Some(Verdict::Yes) => {
                        self.waiting = None;
                        self.fails[op] = 0;
                        self.succeed(op, deploy, grade);
                    }
                    Some(Verdict::No { deploy, reason }) => {
                        self.waiting = None;
                        self.stats.refusals += 1;
                        if let Some(end) = self.refused(core, op, deploy, u32::from(reason)) {
                            return end;
                        }
                    }
                    None if tick.wrapping_sub(since) >= VERDICT_TICKS => {
                        self.waiting = None;
                        self.stats.no_answer += 1;
                        self.late = Some((op, deploy, at, tick));
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
        if milestone == Milestone::Done || milestone != began {
            // A milestone done is where a build goal stops: the next one
            // wants another trip for materials. Not offered again until the
            // next survey says what it needs.
            self.survey.milestone = milestone;
            self.survey.ready = false;
            return Act::Done;
        }
        let Some(i) = self.pick(core, &plan, milestone).or_else(|| {
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
        }) else {
            // The pack is short: the mind sends it for materials.
            return Act::Fail(Why::MissingInputs);
        };
        let s = spec(i);
        let at = addr(&plan, &s);
        let Some(tool) = self.tool(core, &s) else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        // What the op is done with, crafted from the pack first.
        if count(core, tool) == 0 {
            let Some((recipe, ticks)) = recipe_for(core, tool) else {
                self.fail(i);
                return Act::Fail(Why::NoRecipe);
            };
            self.want = Some(Await::Craft {
                op: i,
                item: tool,
                before: 0,
                until: ticks + VERDICT_TICKS,
            });
            return Act::Craft { recipe };
        }
        // Every op is worked from the stand spot in the core.
        let Some(stand) = self.stand() else {
            return Act::Fail(Why::NoSpot);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let far = (stand[0] - x).hypot(stand[1] - z);
        // After a reach refusal, onto the spot itself rather than near it.
        let close = if self.restand { WAYPOINT_M } else { STAND_M };
        if far > close {
            self.held = None;
            if far > STAND_M && self.walled(core) {
                // Through the airlock, or round the cupboard and the box
                // inside, by the chain.
                return match self.passage.step(
                    core,
                    seed,
                    haven,
                    &plan,
                    body,
                    hands,
                    route,
                    Way::In,
                    tick,
                ) {
                    Pass::Go(intent) => Act::Go(intent),
                    Pass::Use(at, intent) => Act::Use { at, intent },
                    Pass::Done => Act::Go(Intent::IDLE),
                    Pass::Fail(why) => Act::Fail(why),
                };
            }
            // No doorway yet: routed round what stands (a wall between the
            // body and the spot) to the route's closest, then straight on.
            if far > ROUTE_STOP_M {
                match route.to(core, body, stand, ROUTE_STOP_M, true, tick) {
                    step @ Step::Walk { .. } => {
                        self.approach = None;
                        return Act::Go(step.walk().unwrap_or(Intent::IDLE));
                    }
                    Step::Blocked => return Act::Fail(Why::Stuck),
                    Step::Wait => return Act::Go(Intent::IDLE),
                    Step::Arrived => {}
                }
            }
            match self.approach {
                Some((best, since)) if far > best - 0.05 => {
                    if tick.wrapping_sub(since) >= PASSAGE_STALL_TICKS {
                        self.approach = None;
                        if !self.restand {
                            return Act::Fail(Why::Stuck);
                        }
                        // Near enough to work from, if not onto it.
                        self.restand = false;
                    } else {
                        return Act::Go(Intent::walk(yaw_toward(stand[0] - x, stand[1] - z)));
                    }
                }
                _ => {
                    self.approach = Some((far, tick));
                    return Act::Go(Intent::walk(yaw_toward(stand[0] - x, stand[1] - z)));
                }
            }
        }
        // On the spot: any walk in has ended here (the chain's last point is
        // the spot, and the builder stops short of the chain's own radius).
        self.restand = false;
        self.approach = None;
        if self.passage.busy() {
            self.passage = Passage::IDLE;
        }
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
        let deploy = matches!(s.op, Op::Kit(_));
        let act =
            match s.op {
                Op::Piece(part) => piece_row(core, part_shape(part), MAT_TWIG)
                    .map(|row| Act::Place { row, at, intent }),
                Op::Grade(material) => Some(Act::Upgrade {
                    at,
                    material,
                    intent,
                }),
                Op::Kit(name) => kit_row(core, tool).map(|row| Act::Deploy {
                    row,
                    at,
                    bag: name == BAG_ITEM,
                    intent,
                }),
            };
        let Some(act) = act else {
            self.fail(i);
            return Act::Fail(Why::NoRecipe);
        };
        self.want = Some(Await::Op {
            op: i,
            at,
            deploy,
            since: 0,
            intent,
        });
        act
    }

    /// What a refusal of op `i` means for the job; `Some` ends the goal.
    fn refused(&mut self, core: &ClientCore, i: usize, deploy: bool, reason: u32) -> Option<Act> {
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
        if spot {
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
    core.deploys
        .entries()
        .iter()
        .find(|d| (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc))
        .map(|d| d.row)
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

/// Does a tree, a rock or a bush stand where the base would? The map's
/// scatter, which a player sees standing there.
fn scatter_in(
    seed: u64,
    table: &sim_core::terrain::ScatterTable,
    haven: &Haven,
    cx: u16,
    cz: u16,
) -> bool {
    let (x0, z0) = (f32::from(cx) * BUILD_CELL_M, f32::from(cz) * BUILD_CELL_M);
    let (x1, z1) = (x0 + 3.0 * BUILD_CELL_M, z0 + 2.0 * BUILD_CELL_M);
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
        let mut seen = [0usize; 6];
        for i in 0..OPS {
            let s = spec(i);
            let Some(m) = s.stage else {
                // Only what needs a workbench, the rent and the box the
                // one inside replaces are passed over.
                assert!(
                    matches!(
                        STARTER.get(i),
                        Some(
                            BaseOp::Code(..)
                                | BaseOp::Feed(..)
                                | BaseOp::Deploy(Kit::Lock | Kit::MetalDoor | Kit::Box, ..)
                        )
                    ),
                    "op {i} {:?} is never built",
                    STARTER.get(i)
                );
                continue;
            };
            seen[m as usize] += 1;
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
        assert_eq!(seen[Milestone::Done as usize], 0);
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
        b.survey_now(&core);
        assert_eq!(b.survey().milestone, Milestone::Doors);
        assert!(b.survey().hearth);
        let wall = (0..OPS)
            .find(|&i| spec(i).op == Op::Piece(Part::Wall))
            .unwrap();
        let at = addr(&plan, &spec(wall));
        stream(&mut core, |buf| {
            protocol::event::encode_event_removed(true, at.cx, at.cz, at.level, at.loc, buf)
        });
        b.survey_now(&core);
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
        b.survey_now(&core);
        assert_eq!(b.done & bit(bag), 0);
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
        b.late = Some((1, true, at, 0));
        b.waiting = Some(Await::Op {
            op: 2,
            at,
            deploy: true,
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
