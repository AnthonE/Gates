//! The team's base as a shared home.
//!
//! What it knows of it is what its eyes found ([`TeamHome::look`]): first a
//! box it saw a teammate standing at (in view, in range, a clear line to it,
//! an awake teammate within [`CHEST_NEAR_M`]), which is where home is; then
//! the other boxes, the cupboard and the stations it sees within [`HOME_M`]
//! of that one. Never the island's list of boxes, nor whose a box is: a
//! player learns where the team keeps its things the same way, by being
//! shown and by looking round. What each box holds is what its panel showed
//! the last time it was open.
//!
//! A trip home ([`HomeJob`]) walks in through the team's doors
//! (`agent::doors`), and does one of three things there: stocks the boxes
//! ([`Trip::Stock`]: every stack past the belt goes in the box it belongs
//! in, [`TeamHome::best_box`], and what the pack lacks comes out, food,
//! meds and a tool, [`TeamHome::wants`]), then walks back out; stays
//! ([`Trip::Stay`]: home, to sleep or to hold it); or walks to a station
//! there to craft at it ([`Trip::Point`]).
//!
//! Where a stack belongs: the box that already holds that item; else one
//! that holds its kind ([`Group`]: food, meds, tools, arms, wearables,
//! salvage, raw stuff); else an empty one; else whichever has room. So the
//! boxes sort themselves the way the team started them.

use crate::agent::build::{aim_point, e_picks_by, Stations};
use crate::agent::doors::{nearest_door, Doors, Through, HOP_M, MAX_DOORS, MAX_TRIES};
use crate::agent::hands::Hands;
use crate::agent::home::HOLD_TICKS;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::lock::LockCode;
use crate::agent::raid::{BOX_SIGHT_M, BOX_STAND_M};
use crate::agent::route::{Route, Step};
use crate::agent::stash::{self, box_open, carried, land, Ledger};
use crate::agent::tracks::{clear_line, eye_m, Sight, Tracks};
use crate::agent::wiki::{Book, Class};
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::deploy::{
    bench_tier, box_key, cell_center, ARCH_BOX, ARCH_FURNACE, ARCH_HEARTH, ARCH_WORKBENCH,
    ARCH_WORKBENCH2, ARCH_WORKBENCH3, REFUSE_D_OWNER,
};
use sim_core::gather::ItemStack;
use sim_core::limits::{BOX_SLOTS, HOTBAR_SLOTS, INV_SLOTS, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::Haven;
use sim_core::yaw_dir;

/// A teammate standing this near a box is at it: home is there.
pub const CHEST_NEAR_M: f32 = 3.0;
/// The eyes look round the team's base this often...
pub const HOME_LOOK_TICKS: u32 = TICK_HZ / 2;
/// ...at most this many of the mirror's records a look, one line of sight.
pub const HOME_SCAN_PER_LOOK: usize = 16;
/// The team's base: what stands this near the first box it was shown.
pub const HOME_M: f32 = 15.0;
/// Boxes of it kept in mind.
pub const HOME_BOXES: usize = 6;
/// What a trip home takes out: a stack of food, of meds, a tool for wood
/// and one for stone.
pub const WANT_ROWS: usize = 4;
/// A trip home that failed is not offered again for this long.
pub const DELIVER_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// A trip that stays (home to sleep or to hold it, or to a station) stops
/// this near what it walked to.
pub const STAY_NEAR_M: f32 = 2.0;
/// An answer to a press comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// The deployable's archetype standing at this address, per the mirror.
fn arch_at(core: &ClientCore, at: OpAddr) -> Option<u8> {
    let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
    core.deploys
        .entries()
        .iter()
        .find(|d| (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc))
        .filter(|d| u16::from(d.row) < have)
        .map(|d| core.deploy_defs.defs[usize::from(d.row)].arch)
}

fn center(at: OpAddr) -> [f32; 2] {
    let (x, z) = cell_center(at.cx, at.cz);
    [x, z]
}

/// What kind of thing an item is, for which box it goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Food,
    Meds,
    Tools,
    Arms,
    Wear,
    Salvage,
    Raw,
}

impl Group {
    pub fn of(core: &ClientCore, book: &Book, item: u16) -> Group {
        let page = book.page(item);
        match page.class {
            Class::Food => Group::Food,
            Class::Med => Group::Meds,
            Class::Tool => Group::Tools,
            Class::Melee | Class::Ranged | Class::Throw => Group::Arms,
            Class::Other if core.catalog.row(usize::from(item)).wear_slot != 0 => Group::Wear,
            Class::Other if page.recycles => Group::Salvage,
            Class::Other => Group::Raw,
        }
    }
}

/// One of the team's boxes, and what its panel last showed.
#[derive(Clone, Copy, Debug)]
pub struct Kept {
    pub at: OpAddr,
    pub ledger: Ledger,
}

/// The team's base as its eyes found it.
#[derive(Clone, Copy, Debug, Default)]
pub struct TeamHome {
    boxes: [Option<Kept>; HOME_BOXES],
    /// Where the first box it was shown stands: home.
    anchor: Option<[f32; 2]>,
    hearth: Option<OpAddr>,
    benches: [Option<(OpAddr, u8)>; 3],
    furnace: Option<OpAddr>,
    looked: Option<u32>,
    cursor: usize,
    /// Its cupboard said it is on the crew (the team's code entered at the
    /// cupboard's lock): it may build, and put its bag down, in the base.
    crewed: bool,
    /// Boxes, cupboards and stations learnt.
    pub learnt: u64,
}

impl TeamHome {
    /// Home: where the first box it was shown stands.
    pub fn anchor(&self) -> Option<[f32; 2]> {
        self.anchor
    }

    pub fn boxes(&self) -> impl Iterator<Item = &Kept> {
        self.boxes.iter().flatten()
    }

    pub fn box_count(&self) -> usize {
        self.boxes().count()
    }

    /// The first box: where a trip that stays goes.
    pub fn first(&self) -> Option<OpAddr> {
        self.boxes().next().map(|k| k.at)
    }

    pub fn hearth(&self) -> Option<OpAddr> {
        self.hearth
    }

    /// The cupboard has it on its crew, as far as it was last told.
    pub fn crewed(&self) -> bool {
        self.crewed
    }

    /// The cupboard carries a code lock, per the mirror: the team's code
    /// entered there is what puts a hand on its crew.
    pub fn hearth_locked(&self, core: &ClientCore) -> bool {
        self.hearth.is_some_and(|at| {
            core.deploys.entries().iter().any(|d| {
                (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc) && d.has_lock
            })
        })
    }

    /// A lock's or a cupboard's answer: at the team's cupboard, whether it
    /// is on the crew now.
    pub fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8, grant: u8) {
        if self.hearth == Some(OpAddr { cx, cz, level, loc }) {
            self.crewed = grant == sim_core::lock::GRANT_FULL;
        }
    }

    pub fn forget(&mut self) {
        *self = Self {
            learnt: self.learnt,
            ..Self::default()
        };
    }

    /// A teammate said which box is the team's (a clan's word, as
    /// `agent::society` shares it): home is there if it knows none yet,
    /// it is on the team, and the mirror shows a box at that address. The
    /// rest of the base it learns by eye, as it would have.
    pub fn tell(&mut self, core: &ClientCore, at: OpAddr) {
        if self.anchor.is_some()
            || core.team().count == 0
            || self.knows(at)
            || arch_at(core, at) != Some(ARCH_BOX)
        {
            return;
        }
        self.learn(at, ARCH_BOX);
    }

    /// The team's stations as a crafter reads them: the best bench of each
    /// rung, and the furnace.
    pub fn stations(&self) -> Stations {
        let mut s = Stations::NONE;
        for (at, tier) in self.benches.iter().flatten() {
            let spot = Some(center(*at));
            if *tier == 1 {
                s.bench = spot;
            } else {
                s.bench2 = spot;
            }
            s.tier = s.tier.max(*tier);
        }
        s.furnace = self.furnace.map(center);
        s
    }

    /// The panel open is one of the team's boxes: what it shows is what it
    /// holds.
    pub fn on_panel(&mut self, core: &ClientCore) {
        for k in self.boxes.iter_mut().flatten() {
            k.ledger
                .on_cont(core, box_key(k.at.cx, k.at.cz, k.at.level, k.at.loc));
        }
    }

    /// What every box held when last seen, item by item, into `out`; the
    /// rows filled.
    pub fn totals(&self, out: &mut [(u16, u32)]) -> usize {
        let mut n = 0;
        for k in self.boxes() {
            let mut rows = [(0u16, 0u32); BOX_SLOTS];
            let m = k.ledger.totals(&mut rows);
            for &(item, units) in &rows[..m] {
                if let Some(r) = out[..n].iter_mut().find(|r| r.0 == item) {
                    r.1 = r.1.saturating_add(units);
                } else if n < out.len() {
                    out[n] = (item, units);
                    n += 1;
                }
            }
        }
        n
    }

    fn knows(&self, at: OpAddr) -> bool {
        self.boxes().any(|k| k.at == at)
            || self.hearth == Some(at)
            || self.furnace == Some(at)
            || self.benches.iter().flatten().any(|b| b.0 == at)
    }

    fn learn(&mut self, at: OpAddr, arch: u8) {
        match arch {
            ARCH_BOX => {
                let Some(slot) = self.boxes.iter_mut().find(|s| s.is_none()) else {
                    return;
                };
                *slot = Some(Kept {
                    at,
                    ledger: Ledger::EMPTY,
                });
                self.anchor.get_or_insert(center(at));
            }
            ARCH_HEARTH => self.hearth = Some(at),
            ARCH_FURNACE => self.furnace = Some(at),
            ARCH_WORKBENCH | ARCH_WORKBENCH2 | ARCH_WORKBENCH3 => {
                let tier = bench_tier(arch);
                let i = usize::from(tier.saturating_sub(1)).min(2);
                self.benches[i] = Some((at, tier));
            }
            _ => return,
        }
        self.learnt += 1;
    }

    /// Records whose deployable is gone from the mirror are forgotten;
    /// with no box left, home is too.
    fn prune(&mut self, core: &ClientCore) {
        for s in self.boxes.iter_mut() {
            if s.is_some_and(|k| arch_at(core, k.at) != Some(ARCH_BOX)) {
                *s = None;
            }
        }
        if self
            .hearth
            .is_some_and(|at| arch_at(core, at) != Some(ARCH_HEARTH))
        {
            self.hearth = None;
            self.crewed = false;
        }
        if self
            .furnace
            .is_some_and(|at| arch_at(core, at) != Some(ARCH_FURNACE))
        {
            self.furnace = None;
        }
        for b in self.benches.iter_mut() {
            if b.is_some_and(|(at, _)| arch_at(core, at).is_none_or(|a| bench_tier(a) == 0)) {
                *b = None;
            }
        }
        if self.boxes.iter().all(Option::is_none) {
            self.anchor = None;
        }
    }

    /// Look round, every [`HOME_LOOK_TICKS`]: a teammate in sight at a box
    /// makes it the team's; with a home, the next box, cupboard or station
    /// near it that stands in the view cone and range, one line of sight a
    /// look. Not one of its own (`own`).
    pub fn look(
        &mut self,
        core: &mut ClientCore,
        haven: &Haven,
        body: &EntityState,
        tracks: &Tracks,
        tick: u32,
        own: impl Fn(u16, u16, u8, u8) -> bool,
    ) {
        self.prune(core);
        if self
            .looked
            .is_some_and(|t| tick.wrapping_sub(t) < HOME_LOOK_TICKS)
        {
            return;
        }
        self.looked = Some(tick);
        let team = *core.team();
        // Off the team: its base is nobody's business now.
        if team.count == 0 {
            self.forget();
            return;
        }
        let feet = [
            body.qx as f32 * POS_XZ_Q,
            body.qy as f32 * POS_Y_Q,
            body.qz as f32 * POS_XZ_Q,
        ];
        let eye = [feet[0], feet[1] + eye_m(body.crouched), feet[2]];
        let (fx, fz) = yaw_dir(body.yaw);
        let in_view = |p: [f32; 2]| {
            let (dx, dz) = (p[0] - eye[0], p[1] - eye[2]);
            let d = dx.hypot(dz);
            d <= BOX_SIGHT_M
                && (d < 0.5 || dx * fx + dz * fz >= d * std::f32::consts::FRAC_1_SQRT_2)
        };
        let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
        let addr = |d: &sim_core::deploy::DeployRec| OpAddr {
            cx: d.cx,
            cz: d.cz,
            level: d.level,
            loc: d.loc,
        };
        // A teammate at a box.
        let mut found: Option<(OpAddr, u8, f32)> = None;
        let mut best = f32::INFINITY;
        for m in team.members() {
            if m.id == core.player_id || m.state != protocol::TEAM_AWAKE {
                continue;
            }
            let Some(mate) = tracks.get(m.id).filter(|t| t.visible) else {
                continue;
            };
            for d in core.deploys.entries() {
                if u16::from(d.row) >= have
                    || core.deploy_defs.defs[usize::from(d.row)].arch != ARCH_BOX
                    || own(d.cx, d.cz, d.level, d.loc)
                {
                    continue;
                }
                let at = addr(d);
                let p = center(at);
                let near = (p[0] - mate.pos[0]).hypot(p[1] - mate.pos[2]);
                if near <= CHEST_NEAR_M && near < best && !self.knows(at) && in_view(p) {
                    best = near;
                    found = Some((at, ARCH_BOX, mate.pos[1]));
                }
            }
        }
        // With a home: the rest of it, a few records a look.
        if let (None, Some(home)) = (found, self.anchor) {
            let n = core.deploys.entries().len();
            for _ in 0..HOME_SCAN_PER_LOOK.min(n) {
                self.cursor = (self.cursor + 1) % n;
                let d = core.deploys.entries()[self.cursor];
                if u16::from(d.row) >= have {
                    continue;
                }
                let arch = core.deploy_defs.defs[usize::from(d.row)].arch;
                let at = addr(&d);
                let p = center(at);
                let kind = matches!(
                    arch,
                    ARCH_BOX
                        | ARCH_HEARTH
                        | ARCH_FURNACE
                        | ARCH_WORKBENCH
                        | ARCH_WORKBENCH2
                        | ARCH_WORKBENCH3
                );
                if kind
                    && !own(d.cx, d.cz, d.level, d.loc)
                    && !self.knows(at)
                    && (p[0] - home[0]).hypot(p[1] - home[1]) <= HOME_M
                    && in_view(p)
                {
                    found = Some((at, arch, feet[1]));
                    break;
                }
            }
        }
        let Some((at, arch, y)) = found else {
            return;
        };
        // A line to just short of it, at the height of a thing on the
        // floor it stands on.
        let p = center(at);
        let (dx, dz) = (p[0] - eye[0], p[1] - eye[2]);
        let d = dx.hypot(dz).max(0.01);
        let target = [p[0] - dx / d * 0.7, y + 0.5, p[1] - dz / d * 0.7];
        if clear_line(core, haven, eye, target, BOX_SIGHT_M) == Sight::Clear {
            self.learn(at, arch);
        }
    }

    /// The box a stack goes in: one already holding that item with room
    /// for it; else one holding its [`Group`]; else an empty one; else one
    /// never opened; else whichever has the most room. `None` when no box
    /// has room, as far as it knows.
    pub fn best_box(&self, core: &ClientCore, book: &Book, stack: ItemStack) -> Option<usize> {
        let group = Group::of(core, book, stack.item);
        let cap = core.catalog.row(usize::from(stack.item)).stack_max;
        let mut best: Option<(u8, usize, usize)> = None;
        for (i, k) in self.boxes.iter().enumerate() {
            let Some(k) = k else {
                continue;
            };
            let slots = k.ledger.slots();
            let empty = slots.iter().filter(|s| s.count == 0).count();
            let room =
                !k.ledger.known() || land(slots, 0..slots.len(), stack, stack.count, cap).is_some();
            if !room {
                continue;
            }
            let score = if !k.ledger.known() {
                1
            } else if k.ledger.units(stack.item) > 0 {
                4
            } else if slots
                .iter()
                .any(|s| s.count > 0 && Group::of(core, book, s.item) == group)
            {
                3
            } else if empty == slots.len() {
                2
            } else {
                0
            };
            if best.is_none_or(|(s, e, _)| score > s || (score == s && empty > e)) {
                best = Some((score, empty, i));
            }
        }
        best.map(|b| b.2)
    }

    /// What the pack lacks that the boxes are known to hold, into `out`:
    /// a stack of food, of meds, and a better tool for wood and for stone
    /// than any carried. The rows filled.
    pub fn wants(
        &self,
        core: &ClientCore,
        book: &Book,
        out: &mut [(u16, u32); WANT_ROWS],
    ) -> usize {
        let mut n = 0;
        let pack = &core.inv[..INV_SLOTS];
        let held = |f: &dyn Fn(u16) -> bool| pack.iter().any(|s| s.count > 0 && f(s.item));
        let mut stored = [(0u16, 0u32); HOME_BOXES * BOX_SLOTS];
        let m = self.totals(&mut stored);
        let stored = &stored[..m];
        // The most of a class the boxes hold: a stack of it.
        for class in [Class::Food, Class::Med] {
            if held(&|i| book.page(i).class == class) {
                continue;
            }
            if let Some(&(item, units)) = stored
                .iter()
                .filter(|r| book.page(r.0).class == class)
                .max_by_key(|r| r.1)
            {
                let cap = u32::from(core.catalog.row(usize::from(item)).stack_max.max(1));
                out[n] = (item, units.min(cap));
                n += 1;
            }
        }
        // The best tool of each ladder the boxes hold, when it is better
        // than any carried.
        for ladder in [crate::explorer::TREE_TOOLS, crate::explorer::NODE_TOOLS] {
            let named = |i: u16, name: &str| core.catalog.name(usize::from(i)) == name.as_bytes();
            let carried_rank = ladder
                .iter()
                .position(|name| held(&|i| named(i, name)))
                .unwrap_or(ladder.len());
            let best = ladder.iter().enumerate().find_map(|(rank, name)| {
                stored
                    .iter()
                    .find(|r| named(r.0, name))
                    .map(|r| (rank, r.0))
            });
            if let Some((rank, item)) = best {
                if n < WANT_ROWS && rank < carried_rank && !out[..n].iter().any(|r| r.0 == item) {
                    out[n] = (item, 1);
                    n += 1;
                }
            }
        }
        n
    }

    /// Does the pack carry something for the boxes, with room for it in
    /// one of them as far as it knows?
    pub fn has_delivery(&self, core: &ClientCore, book: &Book, wants: &[(u16, u32)]) -> bool {
        (0..INV_SLOTS).any(|i| self.spare(core, book, wants, i).is_some())
    }

    /// Is something it wants in one of the boxes?
    pub fn has_wanted(&self, core: &ClientCore, wants: &[(u16, u32)]) -> bool {
        wants.iter().any(|&(item, want)| {
            carried(core, item) < want && self.boxes().any(|k| k.ledger.units(item) > 0)
        })
    }

    /// The pack stack at `i`, how many of it are spare (past the belt's
    /// loadout and what a want keeps), and the box that would take them.
    fn spare(
        &self,
        core: &ClientCore,
        book: &Book,
        wants: &[(u16, u32)],
        i: usize,
    ) -> Option<(ItemStack, u16, usize)> {
        let s = core.inv[i];
        if s.count == 0 {
            return None;
        }
        let wanted = wants
            .iter()
            .filter(|w| w.0 == s.item)
            .map(|w| w.1)
            .sum::<u32>();
        let keep = stash::loadout(core, book, s.item).max(wanted);
        let spare = carried(core, s.item).saturating_sub(keep);
        let n = u32::from(s.count).min(spare) as u16;
        if n == 0 {
            return None;
        }
        let b = self.best_box(core, book, ItemStack { count: n, ..s })?;
        Some((s, n, b))
    }

    /// A box never opened, not among `skip`: the nearest. A stocking trip
    /// looks in each first, so every stack goes where it belongs.
    fn unknown_box(&self, skip: u8, x: f32, z: f32) -> Option<usize> {
        self.boxes
            .iter()
            .enumerate()
            .filter_map(|(i, k)| {
                k.filter(|k| !k.ledger.known() && skip & (1 << i) == 0)
                    .map(|k| (i, k))
            })
            .min_by(|(_, a), (_, b)| {
                let d = |k: &Kept| {
                    let p = center(k.at);
                    (p[0] - x).hypot(p[1] - z)
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    }

    /// What every box holds is known.
    fn all_known(&self) -> bool {
        self.boxes().all(|k| k.ledger.known())
    }

    /// The next box a stocking trip goes to, not yet seen to on it: where a
    /// carried stack belongs, or one holding something wanted; the nearest.
    fn next_box(
        &self,
        core: &ClientCore,
        book: &Book,
        wants: &[(u16, u32)],
        visited: u8,
        x: f32,
        z: f32,
    ) -> Option<usize> {
        let mut pick: Option<(f32, usize)> = None;
        let mut consider = |i: usize| {
            if visited & (1 << i) != 0 {
                return;
            }
            let Some(k) = self.boxes[i] else {
                return;
            };
            let p = center(k.at);
            let d = (p[0] - x).hypot(p[1] - z);
            if pick.is_none_or(|(b, _)| d < b) {
                pick = Some((d, i));
            }
        };
        for i in 0..INV_SLOTS {
            if let Some((.., b)) = self.spare(core, book, wants, i) {
                consider(b);
            }
        }
        for &(item, want) in wants {
            if carried(core, item) >= want {
                continue;
            }
            for (i, k) in self.boxes.iter().enumerate() {
                if k.is_some_and(|k| k.ledger.units(item) > 0) {
                    consider(i);
                }
            }
        }
        pick.map(|p| p.1)
    }
}

/// What a trip home is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trip {
    /// Put the pack away, sorted, take out what it lacks, walk back out.
    Stock,
    /// Home and stay: to sleep there, or to hold it.
    Stay,
    /// To this spot in the base (a station), and stay.
    Point([f32; 2]),
}

/// What the trip wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deliver {
    /// A door on the way, or a step.
    Door(Through),
    /// Open the box's panel.
    Open {
        key: u32,
        intent: Intent,
    },
    /// Pack slot to box slot.
    Put {
        key: u32,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Box slot to pack slot.
    Take {
        key: u32,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Shut the panel.
    Close(Intent),
    Done,
    Fail(Why),
}

impl Deliver {
    pub const IDLE: Self = Deliver::Door(Through::Go(Intent::IDLE));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leg {
    #[default]
    In,
    Chest,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lid {
    Aiming(u32),
    Opening(u32),
    Moving(u32),
    Open,
}

/// One trip into the team's base.
#[derive(Clone, Copy, Debug)]
pub struct HomeJob {
    trip: Trip,
    leg: Leg,
    pub doors: Doors,
    /// The doors it is inside of, outermost first: through one, it goes on
    /// the stack; back through the top one, it comes off. Out is the
    /// stack walked back down.
    passed: [Option<OpAddr>; MAX_DOORS],
    passed_len: u8,
    /// Doors that would not be got to or through.
    tried: [Option<OpAddr>; MAX_DOORS],
    /// The box being seen to, by its place in [`TeamHome`], where it
    /// stands, the boxes seen to on this trip, and the ones only looked in
    /// while others were not known yet.
    target: Option<usize>,
    chest: Option<OpAddr>,
    visited: u8,
    peeked: u8,
    /// This box is only being looked in.
    looking: bool,
    lid: Lid,
    moved: Option<bool>,
    /// The move in flight: units, and whether it went in or came out.
    moving: (u16, bool),
    moves: u8,
    tries: u8,
    /// Why the trip failed inside, said once it is back out.
    failed: Option<Why>,
    /// Units put in the boxes, and taken out.
    pub put: u32,
    pub took: u32,
}

impl Default for HomeJob {
    fn default() -> Self {
        Self::new(Trip::Stock)
    }
}

impl HomeJob {
    pub fn new(trip: Trip) -> Self {
        Self {
            trip,
            leg: Leg::In,
            doors: Doors::default(),
            passed: [None; MAX_DOORS],
            passed_len: 0,
            tried: [None; MAX_DOORS],
            target: None,
            chest: None,
            visited: 0,
            peeked: 0,
            looking: false,
            lid: Lid::Aiming(0),
            moved: None,
            moving: (0, true),
            moves: 0,
            tries: 0,
            failed: None,
            put: 0,
            took: 0,
        }
    }

    pub fn trip(&self) -> Trip {
        self.trip
    }

    /// The step went out on the wire at `tick`.
    pub fn sent(&mut self, what: &Deliver, tick: u32) {
        match *what {
            Deliver::Door(ref door) => self.doors.sent(door, tick),
            Deliver::Open { .. } => self.lid = Lid::Opening(tick),
            Deliver::Put { count, .. } | Deliver::Take { count, .. } => {
                self.lid = Lid::Moving(tick);
                self.moved = None;
                self.moving = (count, matches!(what, Deliver::Put { .. }));
                self.moves = self.moves.saturating_add(1);
            }
            Deliver::Close(_) => {
                // This box is seen to (or only looked in, while others were
                // not known): the next, or out.
                if let Some(i) = self.target.take() {
                    if self.looking {
                        self.peeked |= 1 << i;
                    } else {
                        self.visited |= 1 << i;
                    }
                }
                self.leg = Leg::In;
                self.moves = 0;
            }
            _ => {}
        }
    }

    /// A deploy refusal: a door's, or a lock on the box refusing a move
    /// (`REFUSE_D_OWNER`), when the code goes in there first.
    pub fn on_refused(&mut self, reason: u8) {
        if self.doors.on_refused(reason) || u32::from(reason) != REFUSE_D_OWNER {
            return;
        }
        if let (Lid::Moving(_), Some(chest)) = (self.lid, self.chest) {
            self.lid = Lid::Open;
            self.moved = None;
            self.doors.knock_at(chest);
        }
    }

    pub fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        self.doors.on_auth(cx, cz, level, loc);
    }

    /// A box's panel opened.
    pub fn on_panel(&mut self, handle: u32) {
        if let (Lid::Opening(_), Some(at)) = (self.lid, self.chest) {
            if box_key(at.cx, at.cz, at.level, at.loc) == handle {
                self.lid = Lid::Open;
            }
        }
    }

    /// A move's answer.
    pub fn on_moved(&mut self, refused: bool) {
        if let Lid::Moving(_) = self.lid {
            self.moved = Some(!refused);
            self.lid = Lid::Open;
        }
    }

    /// The trip fails. A stocking trip inside a door it went through walks
    /// back out first, as it came, and says why there.
    fn fail(&mut self, why: Why) -> Deliver {
        if self.leg == Leg::Out || self.passed_len == 0 || self.trip != Trip::Stock {
            return Deliver::Fail(why);
        }
        self.failed = Some(why);
        self.leg = Leg::Out;
        self.doors.clear();
        self.tries = 0;
        Deliver::IDLE
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        home: &TeamHome,
        wants: &[(u16, u32)],
        code: Option<LockCode>,
        book: &Book,
        tick: u32,
    ) -> Deliver {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.tries >= MAX_TRIES {
            return self.fail(Why::Refused);
        }
        if self.doors.busy() {
            return match self
                .doors
                .step(core, seed, haven, body, hands, route, code, tick)
            {
                Through::Passed(at) => {
                    let top = usize::from(self.passed_len).checked_sub(1);
                    let back = top.is_some_and(|t| self.passed[t] == Some(at));
                    if back {
                        // Back out through it.
                        self.passed_len -= 1;
                        self.passed[usize::from(self.passed_len)] = None;
                    } else if usize::from(self.passed_len) < MAX_DOORS {
                        self.passed[usize::from(self.passed_len)] = Some(at);
                        self.passed_len += 1;
                    }
                    // Through a door, it is somewhere else: a door it could
                    // not get to from the other side may be the way from
                    // this one (the inner door of an airlock, given up from
                    // outside the base, is the one in from inside the
                    // airlock). The one it just came back out of is not
                    // tried again straight away.
                    if self.leg != Leg::Out {
                        self.tried = [None; MAX_DOORS];
                        if back {
                            self.tried[0] = Some(at);
                        }
                    }
                    Deliver::IDLE
                }
                // On the way in another door may do; on the way out it is
                // the only way.
                Through::GaveUp(at) => match self.tried.iter_mut().find(|s| s.is_none()) {
                    Some(slot) if self.leg != Leg::Out => {
                        *slot = Some(at);
                        Deliver::IDLE
                    }
                    _ => self.fail(Why::Stuck),
                },
                Through::Fail(why) => self.fail(why),
                step => Deliver::Door(step),
            };
        }
        match self.leg {
            Leg::In => {
                // Where to: the next box to see to, home's first box, or a
                // spot in the base.
                let (goal, stop) = match self.trip {
                    Trip::Stock => {
                        if self.target.is_none() {
                            // A look in each box never opened first, while
                            // there is anything to do.
                            let busy = home.has_delivery(core, book, wants)
                                || home.has_wanted(core, wants);
                            let skip = self.visited | self.peeked;
                            self.looking = false;
                            self.target = match home.unknown_box(skip, x, z).filter(|_| busy) {
                                Some(i) => {
                                    self.looking = true;
                                    Some(i)
                                }
                                None => home.next_box(core, book, wants, self.visited, x, z),
                            };
                        }
                        let Some(i) = self.target else {
                            self.leg = Leg::Out;
                            return Deliver::IDLE;
                        };
                        let Some(k) = home.boxes[i] else {
                            self.visited |= 1 << i;
                            self.target = None;
                            return Deliver::IDLE;
                        };
                        self.chest = Some(k.at);
                        (center(k.at), BOX_STAND_M)
                    }
                    Trip::Stay => match home.first() {
                        Some(at) => (center(at), STAY_NEAR_M),
                        None => return Deliver::Fail(Why::NotFound),
                    },
                    Trip::Point(p) => (p, STAY_NEAR_M),
                };
                let there = (goal[0] - x).hypot(goal[1] - z) <= stop;
                let step = if there {
                    Step::Arrived
                } else {
                    route.to(core, body, goal, stop - 0.2, false, tick)
                };
                match step {
                    step @ Step::Walk { .. } => {
                        Deliver::Door(Through::Go(step.walk().unwrap_or(Intent::IDLE)))
                    }
                    Step::Wait => Deliver::Door(Through::Go(Intent::walk(yaw_toward(
                        goal[0] - x,
                        goal[1] - z,
                    )))),
                    Step::Arrived if self.trip == Trip::Stock => {
                        self.leg = Leg::Chest;
                        self.lid = Lid::Aiming(tick);
                        Deliver::IDLE
                    }
                    Step::Arrived => Deliver::Done,
                    // No way there from here: a door is in the way, the
                    // nearest one near where it is going not tried yet.
                    Step::Blocked => {
                        // Not a door it is inside of; the innermost, which
                        // is the way back out, only when no other will do.
                        // Just through it, it is the nearest door there is,
                        // and taking it again walks back out the way the
                        // trip came in.
                        let mut skip = [None; 2 * MAX_DOORS];
                        skip[..MAX_DOORS].copy_from_slice(&self.passed);
                        skip[MAX_DOORS..].copy_from_slice(&self.tried);
                        let mut door = nearest_door(core, goal, HOP_M, 0, &skip, x, z);
                        if door.is_none() {
                            let inner = usize::from(self.passed_len.saturating_sub(1));
                            skip[inner] = None;
                            door = nearest_door(core, goal, HOP_M, 0, &skip, x, z);
                        }
                        match door {
                            Some(door) => {
                                self.doors.begin_toward(door, goal, tick);
                                Deliver::IDLE
                            }
                            None => self.fail(Why::Stuck),
                        }
                    }
                }
            }
            Leg::Chest => self.at_chest(core, seed, haven, body, hands, home, wants, book, tick),
            Leg::Out => {
                let Some(top) = usize::from(self.passed_len).checked_sub(1) else {
                    return match self.failed {
                        Some(why) => Deliver::Fail(why),
                        None if self.put > 0 || self.took > 0 => Deliver::Done,
                        None => Deliver::Fail(Why::BoxFull),
                    };
                };
                match self.passed[top] {
                    Some(door) => {
                        self.doors.begin(door, x, z, tick);
                        Deliver::IDLE
                    }
                    None => Deliver::Fail(Why::Stuck),
                }
            }
        }
    }

    /// At a box: the panel opened with `E`; what the pack lacks taken out,
    /// then every stack that belongs in this box put in, a stack at a time;
    /// then the panel shut.
    #[allow(clippy::too_many_arguments)]
    fn at_chest(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        home: &TeamHome,
        wants: &[(u16, u32)],
        book: &Book,
        tick: u32,
    ) -> Deliver {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let (Some(i), Some(chest)) = (self.target, self.chest) else {
            self.leg = Leg::In;
            return Deliver::IDLE;
        };
        if arch_at(core, chest) != Some(ARCH_BOX) {
            self.visited |= 1 << i;
            self.target = None;
            self.leg = Leg::In;
            return Deliver::IDLE;
        }
        let key = box_key(chest.cx, chest.cz, chest.level, chest.loc);
        let intent = Intent {
            look: Look::Point(aim_point(core, seed, haven, chest, x, z)),
            ..Intent::IDLE
        };
        let go = Deliver::Door(Through::Go(intent));
        match self.lid {
            Lid::Aiming(since) => {
                let p = center(chest);
                if (p[0] - x).hypot(p[1] - z) > BOX_STAND_M + 0.5 {
                    // Pushed off it: walk back.
                    self.leg = Leg::In;
                    return Deliver::IDLE;
                }
                if tick.wrapping_sub(since) >= HOLD_TICKS
                    && hands.settled()
                    && e_picks_by(core, x, z, hands.view().0, chest, 0.0)
                {
                    return Deliver::Open { key, intent };
                }
                if tick.wrapping_sub(since) >= VERDICT_TICKS {
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                }
                go
            }
            Lid::Opening(sent) => {
                if box_open(core, key) {
                    self.lid = Lid::Open;
                } else if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                }
                go
            }
            Lid::Moving(sent) => {
                if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                    self.lid = Lid::Open;
                    self.moved = Some(false);
                }
                go
            }
            Lid::Open => {
                let moved = self.moved.take();
                if moved == Some(true) {
                    let (n, put) = std::mem::take(&mut self.moving);
                    if put {
                        self.put = self.put.saturating_add(u32::from(n));
                    } else {
                        self.took = self.took.saturating_add(u32::from(n));
                    }
                }
                if !box_open(core, key) {
                    // The panel shut under it.
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                    return go;
                }
                if moved == Some(false) || self.moves >= stash::MAX_MOVES {
                    return Deliver::Close(intent);
                }
                // Once every box is known, this one is seen to properly.
                if home.all_known() {
                    self.looking = false;
                }
                match plan(core, book, home, wants, i, !self.looking) {
                    Some((true, from, to, count)) => Deliver::Put {
                        key,
                        from,
                        to,
                        count,
                        intent,
                    },
                    Some((false, from, to, count)) => Deliver::Take {
                        key,
                        from,
                        to,
                        count,
                        intent,
                    },
                    None => Deliver::Close(intent),
                }
            }
        }
    }
}

/// The next move at box `here`, its panel open: `(put, from, to, count)`.
/// First what the pack wants that this box holds (box to pack); then, with
/// `puts`, the spare stacks whose best box is this one (pack to box). The
/// box's slots are the panel's, which the home's record of it follows.
fn plan(
    core: &ClientCore,
    book: &Book,
    home: &TeamHome,
    wants: &[(u16, u32)],
    here: usize,
    puts: bool,
) -> Option<(bool, u8, u8, u16)> {
    let boxed = &core.cont[..BOX_SLOTS];
    let pack = &core.inv[..INV_SLOTS];
    let cap = |item: u16| core.catalog.row(usize::from(item)).stack_max;
    let into_pack = (HOTBAR_SLOTS..INV_SLOTS).chain(0..HOTBAR_SLOTS);
    for &(item, want) in wants {
        let short = want.saturating_sub(carried(core, item));
        if short == 0 {
            continue;
        }
        for (b, s) in boxed.iter().enumerate() {
            if s.count == 0 || s.item != item {
                continue;
            }
            let n = u32::from(s.count).min(short) as u16;
            if let Some((to, count)) = land(pack, into_pack.clone(), *s, n, cap(item)) {
                return Some((false, b as u8, to, count));
            }
        }
    }
    for i in (0..INV_SLOTS).rev().filter(|_| puts) {
        let Some((s, n, best)) = home.spare(core, book, wants, i) else {
            continue;
        };
        if best != here {
            continue;
        }
        if let Some((to, count)) = land(boxed, 0..boxed.len(), s, n, cap(s.item)) {
            return Some((true, i as u8, to, count));
        }
    }
    None
}
