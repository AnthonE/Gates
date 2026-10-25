//! Electricity: Rust's IO entities, the core kit (`deployables.toml`
//! `[power]`). Parts are deployables with ports ([`ports`]); a **wire** joins
//! one part's output to another's input ([`WireRec`], laid with the wire
//! tool, `World`'s `Command::Wire`). Power is an integer rate, Rust's rW,
//! solved for the whole island every [`POWER_PERIOD_TICKS`].
//!
//! ## The parts
//!
//! | part | ports | does |
//! |---|---|---|
//! | small generator | 0 → 1 | `out_rw` while it runs, burning low grade fuel (`oven.rs`: it is a burner like a furnace, `CookContent::gen_fuel_*`) |
//! | solar panel | 0 → 1 | up to `out_rw` at noon, falling off toward dawn and dusk, none at night |
//! | battery | 1 → 1 | stores what reaches it (`charge_pct` of it) up to its capacity, gives up to `out_rw` of it, and only what its output asks for |
//! | switch | 1 → 1 | passes what reaches it, less its draw, while it is on (`DeployRec::open`) |
//! | splitter | 1 → 3 | splits what reaches it, less its draw, evenly over the outputs in use |
//! | root combiner | 2 → 1 | sums its two inputs, less its draw |
//! | ceiling light | 1 → 1 | lit with `draw_rw` reaching it; passes the rest on |
//! | auto turret | 1 → 0 | fires only with `draw_rw` reaching it (`turret.rs`) |
//!
//! ## One solve a period, in two passes
//!
//! [`Grid`] is derived, never saved: one walk of the deploy store finds the
//! parts, the saved wires join them, and a topological order runs every
//! part after whatever feeds it. A part a wire loop feeds is left out of
//! the order and gets nothing; the connect verb refuses a loop
//! ([`would_loop`]), so that is a defence and not a rule anybody meets.
//!
//! 1. **Demand**, children first: what each part asks of its input — its
//!    own draw plus what its outputs ask (a splitter's split is even, so it
//!    asks its busiest output's share of every output), a battery the rate
//!    that would fill it.
//! 2. **Supply**, parents first: what each source gives, and what each part
//!    passes on. A battery gives `min(out_rw, what it holds, what its output
//!    asks)` — Rust's batteries drain only what the circuit uses.
//!
//! Everything is integer rW, and a battery's charge is rW·ticks
//! ([`CellRec`]), so a period moves `rW × POWER_PERIOD_TICKS` of it.
//!
//! ## What is saved
//!
//! The wires and the batteries' charges ([`PowerStore`], in `Deploys`,
//! world format 36). A switch's position is its deploy record's `open`
//! bit, and a generator's run and its fuel unit are its box's oven state,
//! both saved already. A light coming on or going out is told the way a
//! fire's flame is (`EV_OVEN`), so it reaches the client with no new event.

use crate::build::{Pieces, BUILD_REACH_M};
use crate::deploy::{
    box_addr, box_key, DeployContent, DeployRec, Deploys, ARCH_BATTERY, ARCH_COMBINER,
    ARCH_GENERATOR, ARCH_LIGHT, ARCH_SOLAR, ARCH_SPLITTER, ARCH_SWITCH, ARCH_TURRET, REFUSE_D_FULL,
    REFUSE_D_KIND, REFUSE_D_OWNER, REFUSE_D_REACH, REFUSE_D_WIRE_FAR, REFUSE_D_WIRE_LOOP,
    REFUSE_D_WIRE_PORT,
};
use crate::limits::{
    DAY_PHASE_TICKS, DAY_PORTION, DAY_TICKS, MAX_BATTERIES, MAX_DEPLOY_DEFS, MAX_POWER_NODES,
    MAX_WIRES,
};
use crate::world::{EventQueue, Player, EV_DEPLOY_REFUSED, EV_WIRE};

/// Ticks between solves: half a second.
pub const POWER_PERIOD_TICKS: u64 = 15;

/// Ports a part's outputs number at most (the splitter's three).
pub const MAX_OUTS: usize = 3;
/// Ports its inputs number at most (the root combiner's two).
pub const MAX_INS: usize = 2;

/// `(inputs, outputs)` of an archetype; `(0, 0)` for anything that is not
/// a power part. Structural: what a part *is* is its ports.
pub const fn ports(arch: u8) -> (u8, u8) {
    match arch {
        ARCH_GENERATOR | ARCH_SOLAR => (0, 1),
        ARCH_BATTERY | ARCH_SWITCH | ARCH_LIGHT => (1, 1),
        ARCH_SPLITTER => (1, 3),
        ARCH_COMBINER => (2, 1),
        ARCH_TURRET => (1, 0),
        _ => (0, 0),
    }
}

/// Whether an archetype is a power part at all.
pub const fn is_part(arch: u8) -> bool {
    let (i, o) = ports(arch);
    i > 0 || o > 0
}

/// One part's numbers, baked from `[power]`'s rows (`deployables.toml`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerDef {
    /// What a source gives, or a battery gives at most, rW.
    pub out_rw: u16,
    /// What a part takes for itself, rW.
    pub draw_rw: u16,
    /// A battery's capacity, rW·ticks.
    pub capacity: u32,
    /// The share of what reaches a battery it stores, per cent.
    pub charge_pct: u16,
}

/// The baked `[power]` table: every deploy row's numbers, the wire tool and
/// how far a wire reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerContent {
    pub parts: [PowerDef; MAX_DEPLOY_DEFS],
    /// The item that lays wires; `gather::NO_ITEM`-like 0 when content has none.
    pub wire_tool: u16,
    /// A wire's longest span, millimetres.
    pub wire_max_mm: u32,
}

impl PowerContent {
    pub const EMPTY: Self = Self {
        parts: [PowerDef {
            out_rw: 0,
            draw_rw: 0,
            capacity: 0,
            charge_pct: 0,
        }; MAX_DEPLOY_DEFS],
        wire_tool: 0,
        wire_max_mm: 0,
    };

    pub fn def(&self, row: u8) -> PowerDef {
        self.parts.get(row as usize).copied().unwrap_or_default()
    }
}

impl Default for PowerContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// One wire: from output `out` of the part at box address `from` to input
/// `inp` of the part at `to`. Saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireRec {
    pub from: u32,
    pub out: u8,
    pub to: u32,
    pub inp: u8,
}

/// Bytes of one [`WireRec`] in the save and `state_hash`.
pub const WIRE_BYTES: usize = 10;

impl WireRec {
    pub fn bytes(&self) -> [u8; WIRE_BYTES] {
        let mut b = [0u8; WIRE_BYTES];
        b[0..4].copy_from_slice(&self.from.to_le_bytes());
        b[4] = self.out;
        b[5..9].copy_from_slice(&self.to.to_le_bytes());
        b[9] = self.inp;
        b
    }

    pub fn from_bytes(b: &[u8; WIRE_BYTES]) -> Self {
        Self {
            from: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            out: b[4],
            to: u32::from_le_bytes([b[5], b[6], b[7], b[8]]),
            inp: b[9],
        }
    }

    /// Whether this wire touches the part at `key`.
    pub fn touches(&self, key: u32) -> bool {
        self.from == key || self.to == key
    }
}

/// One battery's charge, rW·ticks, keyed by its box address. A battery that
/// has never held anything keeps none. Saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRec {
    pub key: u32,
    pub charge: u32,
}

/// Bytes of one [`CellRec`].
pub const CELL_BYTES: usize = 8;

/// The saved half: every wire and every battery's charge. Dense, swap-
/// removed, boxed like the shops; a part's wires and its charge go with it
/// in `Deploys::remove_at`.
pub struct PowerStore {
    wires: Box<[WireRec; MAX_WIRES]>,
    n_wires: usize,
    cells: Box<[CellRec; MAX_BATTERIES]>,
    n_cells: usize,
}

impl Default for PowerStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerStore {
    pub fn new() -> Self {
        Self {
            wires: crate::boxed_array(WireRec::default()),
            n_wires: 0,
            cells: crate::boxed_array(CellRec::default()),
            n_cells: 0,
        }
    }

    pub fn wires(&self) -> &[WireRec] {
        &self.wires[..self.n_wires]
    }

    pub fn cells(&self) -> &[CellRec] {
        &self.cells[..self.n_cells]
    }

    /// The charge of the battery at `key`, rW·ticks.
    pub fn charge(&self, key: u32) -> u32 {
        self.cells()
            .iter()
            .find(|c| c.key == key)
            .map_or(0, |c| c.charge)
    }

    /// Whether output `out` of `from` is wired.
    pub fn out_taken(&self, from: u32, out: u8) -> bool {
        self.wires().iter().any(|w| w.from == from && w.out == out)
    }

    /// Whether input `inp` of `to` is wired.
    pub fn in_taken(&self, to: u32, inp: u8) -> bool {
        self.wires().iter().any(|w| w.to == to && w.inp == inp)
    }

    /// Lay a wire; false when the store is full.
    pub(crate) fn add(&mut self, w: WireRec) -> bool {
        if self.n_wires == MAX_WIRES {
            return false;
        }
        self.wires[self.n_wires] = w;
        self.n_wires += 1;
        true
    }

    /// Take up every wire on the part at `key`; how many went.
    pub(crate) fn unwire(&mut self, key: u32) -> usize {
        let mut gone = 0;
        let mut i = 0;
        while i < self.n_wires {
            if self.wires[i].touches(key) {
                self.n_wires -= 1;
                self.wires[i] = self.wires[self.n_wires];
                self.wires[self.n_wires] = WireRec::default();
                gone += 1;
            } else {
                i += 1;
            }
        }
        gone
    }

    /// The part at `key` is gone: its wires and its charge with it.
    pub(crate) fn forget(&mut self, key: u32) {
        self.unwire(key);
        if let Some(i) = self.cells().iter().position(|c| c.key == key) {
            self.n_cells -= 1;
            self.cells[i] = self.cells[self.n_cells];
            self.cells[self.n_cells] = CellRec::default();
        }
    }

    /// Write cell `i`'s charge, or a new cell's when `i` is `None`. Returns
    /// the cell's index, `None` when a new one would not fit.
    fn set_charge(&mut self, i: Option<usize>, key: u32, charge: u32) -> Option<usize> {
        match i {
            Some(i) => {
                self.cells[i].charge = charge;
                Some(i)
            }
            None if charge == 0 || self.n_cells == MAX_BATTERIES => None,
            None => {
                self.cells[self.n_cells] = CellRec { key, charge };
                self.n_cells += 1;
                Some(self.n_cells - 1)
            }
        }
    }

    /// Replace everything from a decoded world save. Boot-only.
    pub(crate) fn restore(&mut self, wires: &[WireRec], cells: &[CellRec]) {
        self.n_wires = wires.len().min(MAX_WIRES);
        self.wires[..self.n_wires].copy_from_slice(&wires[..self.n_wires]);
        for w in self.wires[self.n_wires..].iter_mut() {
            *w = WireRec::default();
        }
        self.n_cells = cells.len().min(MAX_BATTERIES);
        self.cells[..self.n_cells].copy_from_slice(&cells[..self.n_cells]);
        for c in self.cells[self.n_cells..].iter_mut() {
            *c = CellRec::default();
        }
    }
}

/// Whether a wire from `from` to `to` would close a loop: whether `from`
/// is already fed, through any run of wires, by `to`. A walk over the
/// saved wires from `to`, bounded by the store: each wire is followed at
/// most once.
pub fn would_loop(store: &PowerStore, from: u32, to: u32) -> bool {
    if from == to {
        return true;
    }
    let wires = store.wires();
    let mut seen = [false; MAX_WIRES];
    let mut stack = [0u32; MAX_WIRES];
    let mut top = 1;
    stack[0] = to;
    while top > 0 {
        top -= 1;
        let at = stack[top];
        for (k, w) in wires.iter().enumerate() {
            if seen[k] || w.from != at {
                continue;
            }
            if w.to == from {
                return true;
            }
            seen[k] = true;
            stack[top] = w.to;
            top += 1;
        }
    }
    false
}

const NONE: u16 = u16::MAX;
const INDEX_LEN: usize = 2 * MAX_POWER_NODES;
const _: () = assert!(INDEX_LEN.is_power_of_two() && MAX_POWER_NODES < NONE as usize);

/// The solve: every part this period, how it is wired, and what reached it.
/// Derived each period from the deploy store and [`PowerStore`]; boxed.
pub struct Grid {
    n: usize,
    key: Box<[u32; MAX_POWER_NODES]>,
    arch: Box<[u8; MAX_POWER_NODES]>,
    row: Box<[u8; MAX_POWER_NODES]>,
    /// A switch's position, a generator's run; true for the rest.
    on: Box<[bool; MAX_POWER_NODES]>,
    /// rW reaching the part this period, every input summed.
    inp: Box<[u16; MAX_POWER_NODES]>,
    /// What the part asks of its input, rW.
    want: Box<[u32; MAX_POWER_NODES]>,
    powered: Box<[bool; MAX_POWER_NODES]>,
    /// A battery's cell in [`PowerStore`], `NONE` without one.
    cell: Box<[u16; MAX_POWER_NODES]>,
    /// The part each output feeds, `NONE` unwired.
    outs: Box<[[u16; MAX_OUTS]; MAX_POWER_NODES]>,
    /// Inputs not yet run, for the order.
    deg: Box<[u8; MAX_POWER_NODES]>,
    order: Box<[u16; MAX_POWER_NODES]>,
    n_order: usize,
    index: Box<[u16; INDEX_LEN]>,
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn slot_of(key: u32) -> usize {
    (key.wrapping_mul(0x9E37_79B1) >> (32 - INDEX_LEN.trailing_zeros())) as usize
}

impl Grid {
    pub fn new() -> Self {
        Self {
            n: 0,
            key: crate::boxed_array(0),
            arch: crate::boxed_array(0),
            row: crate::boxed_array(0),
            on: crate::boxed_array(false),
            inp: crate::boxed_array(0),
            want: crate::boxed_array(0),
            powered: crate::boxed_array(false),
            cell: crate::boxed_array(NONE),
            outs: crate::boxed_array([NONE; MAX_OUTS]),
            deg: crate::boxed_array(0),
            order: crate::boxed_array(0),
            n_order: 0,
            index: crate::boxed_array(NONE),
        }
    }

    /// Parts this period.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn find(&self, key: u32) -> Option<usize> {
        if key == 0 {
            return None;
        }
        let mask = INDEX_LEN - 1;
        let mut s = slot_of(key);
        loop {
            let k = self.index[s];
            if k == NONE {
                return None;
            }
            if self.key[k as usize] == key {
                return Some(k as usize);
            }
            s = (s + 1) & mask;
        }
    }

    /// Whether the part at `key` had its draw met this period; `None` for no
    /// part there (or one past `MAX_POWER_NODES`, which stands dark).
    pub fn powered(&self, key: u32) -> Option<bool> {
        self.find(key).map(|k| self.powered[k])
    }

    /// rW reaching the part at `key` this period.
    pub fn input(&self, key: u32) -> u16 {
        self.find(key).map_or(0, |k| self.inp[k])
    }

    /// The parts that show a lit state to the client (lights, turrets): each
    /// one's key and whether it is lit, for the deploy walk.
    pub fn lit(&self, key: u32) -> Option<bool> {
        self.find(key)
            .filter(|&k| shows_lit(self.arch[k]))
            .map(|k| self.powered[k])
    }

    /// Every part's key, input and state, little-endian, for `state_hash`.
    pub fn hash_into(&self, h: &mut impl FnMut(&[u8])) {
        for k in 0..self.n {
            let mut b = [0u8; 7];
            b[0..4].copy_from_slice(&self.key[k].to_le_bytes());
            b[4..6].copy_from_slice(&self.inp[k].to_le_bytes());
            b[6] = self.powered[k] as u8;
            h(&b);
        }
    }

    /// Rebuild from the store: the parts, their wiring and their order.
    fn build(&mut self, dc: &DeployContent, deploys: &Deploys) {
        self.n = 0;
        self.index.fill(NONE);
        let mask = INDEX_LEN - 1;
        for d in deploys.entries() {
            let Some(def) = dc.defs.get(d.row as usize) else {
                continue;
            };
            if !is_part(def.arch) {
                continue;
            }
            if self.n == MAX_POWER_NODES {
                break;
            }
            let key = box_key(d.cx, d.cz, d.level, d.loc);
            let k = self.n;
            let mut s = slot_of(key);
            while self.index[s] != NONE {
                s = (s + 1) & mask;
            }
            self.index[s] = k as u16;
            self.key[k] = key;
            self.arch[k] = def.arch;
            self.row[k] = d.row;
            self.on[k] = match def.arch {
                ARCH_SWITCH => d.open,
                ARCH_GENERATOR => deploys
                    .box_index(key)
                    .is_some_and(|i| deploys.oven_states()[i].lit),
                _ => true,
            };
            self.inp[k] = 0;
            self.want[k] = 0;
            self.powered[k] = false;
            self.cell[k] = NONE;
            self.outs[k] = [NONE; MAX_OUTS];
            self.deg[k] = 0;
            self.n += 1;
        }
        let store = deploys.power();
        for (ci, c) in store.cells().iter().enumerate() {
            if let Some(k) = self.find(c.key) {
                self.cell[k] = ci as u16;
            }
        }
        for w in store.wires() {
            let (Some(a), Some(b)) = (self.find(w.from), self.find(w.to)) else {
                continue;
            };
            let out = w.out as usize;
            if out < MAX_OUTS && self.outs[a][out] == NONE {
                self.outs[a][out] = b as u16;
                self.deg[b] = self.deg[b].saturating_add(1);
            }
        }
        // Kahn's order, seeded in store order so it is the same on every
        // machine. A part a loop feeds never reaches zero and is left out.
        self.n_order = 0;
        for k in 0..self.n {
            if self.deg[k] == 0 {
                self.order[self.n_order] = k as u16;
                self.n_order += 1;
            }
        }
        let mut head = 0;
        while head < self.n_order {
            let a = self.order[head] as usize;
            head += 1;
            for c in self.outs[a] {
                if c == NONE {
                    continue;
                }
                let c = c as usize;
                self.deg[c] -= 1;
                if self.deg[c] == 0 {
                    self.order[self.n_order] = c as u16;
                    self.n_order += 1;
                }
            }
        }
    }

    fn child_want(&self, k: usize, port: usize) -> u32 {
        match self.outs[k][port] {
            NONE => 0,
            c => self.want[c as usize],
        }
    }

    fn send(&mut self, k: usize, port: usize, rw: u32) {
        let c = self.outs[k][port];
        if c != NONE {
            let c = c as usize;
            self.inp[c] = (self.inp[c] as u32 + rw).min(u16::MAX as u32) as u16;
        }
    }
}

/// Parts whose lit state the client draws: a light's glow, a turret's lamp.
pub const fn shows_lit(arch: u8) -> bool {
    matches!(arch, ARCH_LIGHT | ARCH_TURRET)
}

/// Ticks of daylight in a day: `day_frac` below `DAY_PORTION`.
const DAYLIGHT_TICKS: u64 = (DAY_TICKS as f64 * DAY_PORTION as f64) as u64;

/// What a solar panel rated `out_rw` gives at `tick`: `out_rw × 4t(1−t)` over
/// the daylight's share `t`, so its peak at noon and nothing at night. Integer
/// throughout.
pub fn solar_rw(out_rw: u16, tick: u64) -> u32 {
    let ph = tick.wrapping_add(DAY_PHASE_TICKS) % DAY_TICKS;
    if ph >= DAYLIGHT_TICKS {
        return 0;
    }
    let d = DAYLIGHT_TICKS;
    (out_rw as u64 * 4 * ph * (d - ph) / (d * d)) as u32
}

/// The two grids: this period's solve and the last one's, swapped each
/// period so a light that changed is told without a third store.
pub struct Power {
    pub grid: Box<Grid>,
    prev: Box<Grid>,
}

impl Default for Power {
    fn default() -> Self {
        Self::new()
    }
}

impl Power {
    pub fn new() -> Self {
        Self {
            grid: Box::new(Grid::new()),
            prev: Box::new(Grid::new()),
        }
    }

    /// Whether the turret at `key`, drawing `draw_rw`, may fire: a turret
    /// whose row asks nothing needs no wire.
    pub fn feeds(&self, key: u32, draw_rw: u16) -> bool {
        draw_rw == 0 || self.grid.powered(key) == Some(true)
    }
}

/// One solve, on its period. Battery charges are written back to the
/// store; a light or turret whose lit state moved is told (`EV_OVEN`).
pub fn sweep(
    tick: u64,
    dc: &DeployContent,
    pc: &PowerContent,
    deploys: &mut Deploys,
    power: &mut Power,
    events: &mut EventQueue,
) {
    if !tick.is_multiple_of(POWER_PERIOD_TICKS) {
        return;
    }
    core::mem::swap(&mut power.grid, &mut power.prev);
    let g = &mut *power.grid;
    g.build(dc, deploys);
    if g.n == 0 && power.prev.n == 0 {
        return;
    }
    let period = POWER_PERIOD_TICKS as u32;

    // 1. Demand, children first.
    for idx in (0..g.n_order).rev() {
        let k = g.order[idx] as usize;
        let d = pc.def(g.row[k]);
        let draw = d.draw_rw as u32;
        g.want[k] = match g.arch[k] {
            ARCH_SPLITTER => {
                let used = g.outs[k].iter().filter(|&&c| c != NONE).count() as u32;
                let most = (0..MAX_OUTS).map(|p| g.child_want(k, p)).max().unwrap_or(0);
                draw + used * most
            }
            ARCH_SWITCH if !g.on[k] => draw,
            ARCH_SWITCH | ARCH_COMBINER | ARCH_LIGHT => draw + g.child_want(k, 0),
            ARCH_BATTERY => {
                let held = g.cell[k] as usize;
                let held = if g.cell[k] == NONE {
                    0
                } else {
                    deploys.power().cells()[held].charge
                };
                d.capacity.saturating_sub(held).div_ceil(period)
            }
            _ => draw,
        };
    }

    // 2. Supply, parents first.
    for idx in 0..g.n_order {
        let k = g.order[idx] as usize;
        let d = pc.def(g.row[k]);
        let draw = d.draw_rw as u32;
        let inp = g.inp[k] as u32;
        match g.arch[k] {
            ARCH_GENERATOR => {
                g.powered[k] = g.on[k];
                let out = if g.on[k] { d.out_rw as u32 } else { 0 };
                g.send(k, 0, out);
            }
            ARCH_SOLAR => {
                let out = solar_rw(d.out_rw, tick);
                g.powered[k] = out > 0;
                g.send(k, 0, out);
            }
            ARCH_BATTERY => {
                let cell = (g.cell[k] != NONE).then_some(g.cell[k] as usize);
                let held = cell.map_or(0, |i| deploys.power().cells()[i].charge);
                let stored = (inp * period).saturating_mul(d.charge_pct as u32) / 100;
                let held = held.saturating_add(stored).min(d.capacity);
                let out = (d.out_rw as u32).min(g.child_want(k, 0)).min(held / period);
                let held = held - out * period;
                if let Some(i) = deploys.power_mut().set_charge(cell, g.key[k], held) {
                    g.cell[k] = i as u16;
                }
                g.powered[k] = out > 0;
                g.send(k, 0, out);
            }
            ARCH_SPLITTER => {
                g.powered[k] = inp >= draw && inp > 0;
                let used = g.outs[k].iter().filter(|&&c| c != NONE).count() as u32;
                if g.powered[k] && used > 0 {
                    let each = (inp - draw) / used;
                    for p in 0..MAX_OUTS {
                        g.send(k, p, each);
                    }
                }
            }
            ARCH_SWITCH => {
                g.powered[k] = g.on[k] && inp >= draw && inp > 0;
                if g.powered[k] {
                    g.send(k, 0, inp - draw);
                }
            }
            _ => {
                // The combiner, the light, the turret: lit with its draw met,
                // passing on what it does not take (a turret has no output).
                g.powered[k] = inp >= draw && inp > 0;
                if (ports(g.arch[k]).1) > 0 {
                    g.send(k, 0, inp.saturating_sub(draw));
                }
            }
        }
    }

    // 3. Tell the client what lit or went dark.
    for k in 0..g.n {
        if !shows_lit(g.arch[k]) {
            continue;
        }
        let was = power.prev.lit(g.key[k]).unwrap_or(false);
        if g.powered[k] != was {
            let (cx, cz, level, loc) = crate::deploy::box_addr(g.key[k]);
            crate::oven::announce(cx, cz, level, loc, g.powered[k], 0, events);
        }
    }
}

/// The part at box address `key`: its store index, record and archetype.
fn part_at(dc: &DeployContent, deploys: &Deploys, key: u32) -> Option<(usize, DeployRec, u8)> {
    let (cx, cz, level, loc) = box_addr(key);
    let i = deploys.find_index(cx, cz, level, loc)?;
    let d = deploys.entries()[i];
    let arch = dc.defs.get(d.row as usize)?.arch;
    is_part(arch).then_some((i, d, arch))
}

/// Whether `p` stands within a hand's reach of `d` and has its building
/// privilege (`turret::spares`: its owner, or its owner's crew where it
/// stands) — Rust asks building privilege of every wire and switch.
fn may_work(pieces: &Pieces, deploys: &Deploys, d: &DeployRec, p: &Player) -> Result<(), u32> {
    let (x, z) = d.xz();
    let (px, pz) = crate::deploy::player_xz(p);
    let (dx, dz) = (x - px, z - pz);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        return Err(REFUSE_D_REACH);
    }
    if !crate::turret::spares(pieces, deploys, d.owner, x, z, p.id) {
        return Err(REFUSE_D_OWNER);
    }
    Ok(())
}

/// Flip the switch at the address (`Command::Use`). False when no switch
/// stands there, so the use verb asks the next thing; true otherwise, with
/// a refusal or the flip and its `EV_DOOR` (the record's `open` bit is the
/// switch's position, and that event is what tells it).
#[allow(clippy::too_many_arguments)]
pub fn toggle(
    dc: &DeployContent,
    pieces: &Pieces,
    deploys: &mut Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) -> bool {
    let Some((i, d, arch)) = part_at(dc, deploys, box_key(cx, cz, level, loc)) else {
        return false;
    };
    if arch != ARCH_SWITCH {
        return false;
    }
    if let Err(why) = may_work(pieces, deploys, &d, p) {
        events.push(EV_DEPLOY_REFUSED, p.id, why, 0);
        return true;
    }
    deploys.set_open_at(i, !d.open);
    crate::deploy::announce_door(deploys, i, p.id, events);
    true
}

/// Lay a wire from output `out` of the part at `from` to input `inp` of the
/// part at `to` (`Command::Wire`): the wire tool in hand, the second part in
/// reach, building privilege at both, both ports real and free, a span the
/// content allows, and no loop. Refusals are `EV_DEPLOY_REFUSED`; a wire
/// laid is `EV_WIRE`.
#[allow(clippy::too_many_arguments)]
pub fn connect(
    dc: &DeployContent,
    pc: &PowerContent,
    pieces: &Pieces,
    deploys: &mut Deploys,
    p: &Player,
    w: WireRec,
    events: &mut EventQueue,
) {
    let refuse = |events: &mut EventQueue, why| events.push(EV_DEPLOY_REFUSED, p.id, why, 0);
    let held = p.inv.get(p.frame.sel as usize).copied().unwrap_or_default();
    if pc.wire_tool == 0 || held.count == 0 || held.item != pc.wire_tool {
        return refuse(events, REFUSE_D_KIND);
    }
    let (Some((_, a, a_arch)), Some((_, b, b_arch))) =
        (part_at(dc, deploys, w.from), part_at(dc, deploys, w.to))
    else {
        return refuse(events, REFUSE_D_KIND);
    };
    if w.out >= ports(a_arch).1 || w.inp >= ports(b_arch).0 {
        return refuse(events, REFUSE_D_WIRE_PORT);
    }
    if let Err(why) = may_work(pieces, deploys, &b, p) {
        return refuse(events, why);
    }
    let (ax, az) = a.xz();
    if !crate::turret::spares(pieces, deploys, a.owner, ax, az, p.id) {
        return refuse(events, REFUSE_D_OWNER);
    }
    let (bx, bz) = b.xz();
    let dy = (a.level as f32 - b.level as f32) * crate::build::LEVEL_H_M;
    let span_mm = ((ax - bx) * (ax - bx) + (az - bz) * (az - bz) + dy * dy).sqrt() * 1000.0;
    if span_mm > pc.wire_max_mm as f32 {
        return refuse(events, REFUSE_D_WIRE_FAR);
    }
    let store = deploys.power();
    if store.out_taken(w.from, w.out) || store.in_taken(w.to, w.inp) {
        return refuse(events, REFUSE_D_WIRE_PORT);
    }
    if would_loop(store, w.from, w.to) {
        return refuse(events, REFUSE_D_WIRE_LOOP);
    }
    if !deploys.power_mut().add(w) {
        return refuse(events, REFUSE_D_FULL);
    }
    events.push(
        EV_WIRE,
        w.from,
        w.to,
        1 << 16 | (w.out as u32) << 8 | w.inp as u32,
    );
}

/// Take up every wire on the part at `key` (`Command::Unwire`): in reach,
/// under its building privilege. One `EV_WIRE` for all of them.
pub fn unwire(
    dc: &DeployContent,
    pieces: &Pieces,
    deploys: &mut Deploys,
    p: &Player,
    key: u32,
    events: &mut EventQueue,
) {
    let Some((_, d, _)) = part_at(dc, deploys, key) else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    };
    if let Err(why) = may_work(pieces, deploys, &d, p) {
        events.push(EV_DEPLOY_REFUSED, p.id, why, 0);
        return;
    }
    if deploys.power_mut().unwire(key) > 0 {
        events.push(EV_WIRE, key, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::{DeployDef, ARCH_TURRET, PLACE_ANY};

    // One row of each part, in this order.
    const BATTERY: u8 = 0;
    const SPLITTER: u8 = 1;
    const LIGHT: u8 = 2;
    const TURRET: u8 = 3;
    const SWITCH: u8 = 4;
    const SOLAR: u8 = 5;
    const BIG: u8 = 6;

    fn content() -> (DeployContent, PowerContent) {
        let mut dc = DeployContent::EMPTY;
        let arches = [
            ARCH_BATTERY,
            ARCH_SPLITTER,
            ARCH_LIGHT,
            ARCH_TURRET,
            ARCH_SWITCH,
            ARCH_SOLAR,
            ARCH_BATTERY,
        ];
        for (k, &arch) in arches.iter().enumerate() {
            dc.defs[k] = DeployDef {
                arch,
                placement: PLACE_ANY,
                hp: 100,
                item: 40 + k as u16,
                ..DeployDef::INERT
            };
        }
        dc.def_count = arches.len() as u16;
        let mut pc = PowerContent::EMPTY;
        let tick = 60 * crate::limits::TICK_HZ;
        pc.parts[BATTERY as usize] = PowerDef {
            out_rw: 15,
            capacity: 400 * tick,
            charge_pct: 100,
            ..Default::default()
        };
        pc.parts[BIG as usize] = PowerDef {
            out_rw: 100,
            capacity: 24_000 * tick,
            charge_pct: 100,
            ..Default::default()
        };
        pc.parts[SPLITTER as usize].draw_rw = 1;
        pc.parts[SWITCH as usize].draw_rw = 1;
        pc.parts[LIGHT as usize].draw_rw = 2;
        pc.parts[TURRET as usize].draw_rw = 10;
        pc.parts[SOLAR as usize].out_rw = 20;
        (dc, pc)
    }

    /// A part of `row` at cell `cx`, its key.
    fn place(d: &mut Deploys, row: u8, cx: u16) -> u32 {
        let rec = DeployRec {
            cx,
            cz: 500,
            row,
            owner: 1,
            hp: 100,
            ..Default::default()
        };
        assert!(d.insert(rec, 0));
        box_key(cx, 500, 0, 0)
    }

    fn wire(d: &mut Deploys, from: u32, out: u8, to: u32) {
        assert!(d.power_mut().add(WireRec {
            from,
            out,
            to,
            inp: 0
        }));
    }

    fn charge(d: &mut Deploys, key: u32, rwm: u32) {
        let c = rwm * 60 * crate::limits::TICK_HZ;
        assert!(d.power_mut().set_charge(None, key, c).is_some());
    }

    fn solve(dc: &DeployContent, pc: &PowerContent, d: &mut Deploys, power: &mut Power) {
        let mut ev = EventQueue::default();
        sweep(0, dc, pc, d, power, &mut ev);
    }

    /// A splitter splits evenly, so it asks its busiest output's share of
    /// each: a turret (10) beside a light (2) is fed when the source covers
    /// 1 + 2 × 10, and starves behind the small battery's 15.
    #[test]
    fn a_splitter_asks_its_busiest_share_of_every_output() {
        let (dc, pc) = content();
        for (source, fed) in [(BIG, true), (BATTERY, false)] {
            let mut d = Deploys::new();
            let mut power = Power::new();
            let b = place(&mut d, source, 500);
            let s = place(&mut d, SPLITTER, 501);
            let l = place(&mut d, LIGHT, 502);
            let t = place(&mut d, TURRET, 503);
            charge(&mut d, b, 100);
            wire(&mut d, b, 0, s);
            wire(&mut d, s, 0, l);
            wire(&mut d, s, 1, t);
            solve(&dc, &pc, &mut d, &mut power);
            assert_eq!(power.grid.powered(t), Some(fed), "source row {source}");
            assert_eq!(power.grid.powered(l), Some(true));
            let want = if fed { 10 } else { 7 };
            assert_eq!(power.grid.input(t), want);
        }
    }

    /// A battery gives only what its output asks, and runs flat on time:
    /// 400 rWm at a light's 2 rW is 200 minutes.
    #[test]
    fn a_battery_drains_what_is_drawn_and_no_more() {
        let (dc, pc) = content();
        let mut d = Deploys::new();
        let mut power = Power::new();
        let b = place(&mut d, BATTERY, 500);
        let l = place(&mut d, LIGHT, 501);
        charge(&mut d, b, 400);
        wire(&mut d, b, 0, l);
        let full = d.power().charge(b);
        solve(&dc, &pc, &mut d, &mut power);
        assert_eq!(power.grid.input(l), 2, "it gave more than the light asks");
        assert_eq!(full - d.power().charge(b), 2 * POWER_PERIOD_TICKS as u32);
        let periods = 200 * 60 * crate::limits::TICK_HZ as u64 / POWER_PERIOD_TICKS - 1;
        for _ in 0..periods {
            solve(&dc, &pc, &mut d, &mut power);
        }
        assert_eq!(power.grid.powered(l), Some(true));
        solve(&dc, &pc, &mut d, &mut power);
        assert_eq!(
            power.grid.powered(l),
            Some(false),
            "it outlasted its charge"
        );
    }

    /// A switch passes power on only while it is on (its `open` bit).
    #[test]
    fn a_switch_gates_what_it_feeds() {
        let (dc, pc) = content();
        let mut d = Deploys::new();
        let mut power = Power::new();
        let b = place(&mut d, BIG, 500);
        let sw = place(&mut d, SWITCH, 501);
        let l = place(&mut d, LIGHT, 502);
        charge(&mut d, b, 100);
        wire(&mut d, b, 0, sw);
        wire(&mut d, sw, 0, l);
        solve(&dc, &pc, &mut d, &mut power);
        assert_eq!(power.grid.powered(l), Some(false), "an off switch fed it");
        let i = d.find_index(501, 500, 0, 0).unwrap();
        d.set_open_at(i, true);
        solve(&dc, &pc, &mut d, &mut power);
        assert_eq!(power.grid.powered(l), Some(true));
        assert_eq!(power.grid.input(l), 2);
    }

    /// No wire may close a loop, and a part taken up takes its wires and
    /// its charge with it.
    #[test]
    fn loops_are_refused_and_a_part_takes_its_wires_with_it() {
        let (_dc, _pc) = content();
        let mut d = Deploys::new();
        let a = place(&mut d, SPLITTER, 500);
        let b = place(&mut d, SPLITTER, 501);
        let c = place(&mut d, BATTERY, 502);
        wire(&mut d, a, 0, b);
        wire(&mut d, b, 0, c);
        assert!(would_loop(d.power(), c, a), "c feeds a through b");
        assert!(would_loop(d.power(), a, a));
        assert!(!would_loop(d.power(), a, c));
        charge(&mut d, c, 10);
        d.power_mut().forget(c);
        assert_eq!(d.power().wires().len(), 1);
        assert_eq!(d.power().charge(c), 0);
    }

    /// The sun: nothing at night, its whole rating at noon.
    #[test]
    fn a_solar_panel_follows_the_day() {
        use crate::limits::{DAY_PHASE_TICKS, DAY_TICKS};
        let noon = DAYLIGHT_TICKS / 2 + DAY_TICKS - DAY_PHASE_TICKS;
        assert_eq!(solar_rw(20, noon), 20);
        let night = DAYLIGHT_TICKS + 100 + DAY_TICKS - DAY_PHASE_TICKS;
        assert_eq!(solar_rw(20, night), 0);
        assert!(solar_rw(20, noon - DAYLIGHT_TICKS / 4) < 20);
    }
}
