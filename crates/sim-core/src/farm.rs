//! Farming v2: one seed is one plant (`content/cooking.toml` `[farm]` and
//! `[[crop]]`). Every living plant on the island sits in one fixed store,
//! [`Farm`], beside the loose stacks: a bed plant names its planter by box
//! key and a spot 0..6, a ground plant (later) names [`HOST_GROUND`].
//!
//! The planter keeps its box record only as a one-slot water tank
//! (`oven::PLANTER_LAYOUT`, `OvenState::water`). A plant grows by daylight
//! under open sky; its pace is the worst of its water, its soil and the air
//! (Rust's "health is the minimum of the conditions"). It ripens, stays ripe
//! for a window, dies, and withers away. Picking it pays the crop and kills
//! the plant.
//!
//! [`step`] runs right after the oven sweep, on the ovens' 15-tick stride:
//! the planters bank rain, then every plant whose index matches the tick
//! is visited. Removals are collected and applied after the pass, so a
//! swap-remove never re-times a plant inside it. No HashMap, no allocation.

use crate::climate::ClimateContent;
use crate::deploy::{box_key, BoxRec, Deploys, ARCH_PLANTER, DEPLOY_VOL};
use crate::deploy::{
    REFUSE_D_KIND, REFUSE_D_PLANTS_FULL, REFUSE_D_PLANT_ITEM, REFUSE_D_PLANT_SOIL,
    REFUSE_D_PLANT_SPOT, REFUSE_D_PLANT_STAGE, REFUSE_D_PLANT_WATER, REFUSE_D_SPOT,
};
use crate::gather::{GatherContent, ItemStack};
use crate::limits::{INV_SLOTS, MAX_CROPS, MAX_PLANTS, TICK_HZ};
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::weather::Wx;
use crate::world::{EventQueue, Player, EV_GATHER, EV_PLANT, EV_PLANT_GONE};

// ---------------------------------------------------------------- stages

pub const STAGE_SEED: u8 = 0;
pub const STAGE_SEEDLING: u8 = 1;
pub const STAGE_SAPLING: u8 = 2;
pub const STAGE_CROSSBREED: u8 = 3;
pub const STAGE_MATURE: u8 = 4;
pub const STAGE_FRUITING: u8 = 5;
pub const STAGE_RIPE: u8 = 6;
pub const STAGE_DYING: u8 = 7;
pub const STAGE_NAMES: [&str; 8] = [
    "SEED",
    "SEEDLING",
    "SAPLING",
    "CROSSBREED",
    "MATURE",
    "FRUITING",
    "RIPE",
    "DYING",
];

/// Still growing: SEED..=FRUITING.
pub const fn is_growing(stage: u8) -> bool {
    stage <= STAGE_FRUITING
}

/// A clone may be taken: SAPLING..=MATURE.
pub const fn is_clonable(stage: u8) -> bool {
    stage >= STAGE_SAPLING && stage <= STAGE_MATURE
}

/// Gives genes to a crossbreeding neighbour: SEED..=RIPE (never DYING).
pub const fn is_donor(stage: u8) -> bool {
    stage <= STAGE_RIPE
}

// ---------------------------------------------------------------- genes

pub const GENE_SLOTS: usize = 6;
pub const GENE_X: u8 = 0;
pub const GENE_W: u8 = 1;
pub const GENE_G: u8 = 2;
pub const GENE_Y: u8 = 3;
pub const GENE_H: u8 = 4;
pub const GENE_CHARS: &[u8; 5] = b"XWGYH";
/// 5^6 genomes, base-5 digits slot 0 lowest; 0 is XXXXXX, neutral.
pub const GENOME_VALUES: u32 = 15_625;
/// Crossbreed weights: red genes outweigh green ones (Rust's 60/100).
pub const CROSS_RED_W: u32 = 10;
pub const CROSS_GREEN_W: u32 = 6;

const POW5: [u16; GENE_SLOTS] = [1, 5, 25, 125, 625, 3125];

/// Slot `k`'s letter (`GENE_*`).
pub fn gene(g: u16, k: usize) -> u8 {
    if k >= GENE_SLOTS {
        return GENE_X;
    }
    ((g / POW5[k]) % 5) as u8
}

/// `g` with slot `k` set to `letter`.
pub fn with_gene(g: u16, k: usize, letter: u8) -> u16 {
    if k >= GENE_SLOTS || letter > GENE_H {
        return g;
    }
    let old = gene(g, k) as u16;
    g - old * POW5[k] + letter as u16 * POW5[k]
}

/// How many slots carry `letter`.
pub fn count(g: u16, letter: u8) -> u32 {
    (0..GENE_SLOTS).filter(|&k| gene(g, k) == letter).count() as u32
}

/// The genome as six ASCII letters, slot 0 first.
pub fn letters(g: u16) -> [u8; GENE_SLOTS] {
    let mut out = [b'X'; GENE_SLOTS];
    for (k, c) in out.iter_mut().enumerate() {
        *c = GENE_CHARS[gene(g, k) as usize];
    }
    out
}

// ---------------------------------------------------------------- beds

pub const PLANTER_COLS: u8 = 3;
pub const PLANTER_ROWS: u8 = 2;
pub const PLANTER_SPOTS: u8 = PLANTER_COLS * PLANTER_ROWS;
pub const SPOT_PITCH_X_M: f32 = 0.5;
pub const SPOT_PITCH_Z_M: f32 = 0.36;
/// How far under the planter's top its soil lies.
pub const BED_SOIL_DROP_M: f32 = 0.06;
/// A ground plant's spot, and the host it names.
pub const SPOT_GROUND: u8 = 15;
/// `deploy::box_key` never sets bit 31, so no planter can be this.
pub const HOST_GROUND: u32 = u32::MAX;

/// A bed spot's position in the planter's own frame: local X along its
/// 1.6 m side. `spot = row * 3 + col`.
pub fn spot_local(spot: u8) -> (f32, f32) {
    let col = (spot % PLANTER_COLS) as f32;
    let row = (spot / PLANTER_COLS) as f32;
    ((col - 1.0) * SPOT_PITCH_X_M, (row - 0.5) * SPOT_PITCH_Z_M)
}

/// A bed spot's world XZ, for a planter at `(cx, cz)` standing at `pose`.
pub fn spot_xz(cx: u16, cz: u16, pose: crate::footprint::Pose, spot: u8) -> (f32, f32) {
    let (x, z) = crate::footprint::centre(cx, cz, pose);
    let (lx, lz) = spot_local(spot);
    crate::footprint::Rect::new(x, z, 0.0, 0.0, pose.yaw).world(lx, lz)
}

/// Two bed spots touch (the crossbreed neighbourhood): a different spot at
/// most one column and one row away.
pub fn adjacent(a: u8, b: u8) -> bool {
    let (ac, ar) = ((a % PLANTER_COLS) as i32, (a / PLANTER_COLS) as i32);
    let (bc, br) = ((b % PLANTER_COLS) as i32, (b / PLANTER_COLS) as i32);
    a != b && (ac - bc).abs() <= 1 && (ar - br).abs() <= 1
}

// ---------------------------------------------------------------- looks

pub const LOOK_CORN: u8 = 0;
pub const LOOK_HEMP: u8 = 1;
pub const LOOK_POTATO: u8 = 2;
pub const LOOK_PUMPKIN: u8 = 3;
pub const LOOK_BERRY: u8 = 4;
pub const LOOK_COUNT: u8 = 5;
pub const LOOK_NAMES: [&str; LOOK_COUNT as usize] =
    ["CORN", "HEMP", "POTATO", "PUMPKIN", "BERRIES"];
/// A look's full-grown height, metres.
pub const LOOK_H_M: [f32; LOOK_COUNT as usize] = [1.8, 1.45, 0.6, 0.45, 1.0];
/// How big a plant draws at each stage, per mille of full grown.
pub const STAGE_SCALE_PM: [u16; 8] = [60, 150, 350, 500, 750, 900, 1000, 950];
/// The pick cylinder's radius, metres.
pub const PLANT_PICK_R_M: f32 = 0.3;
/// The closest two ground plants may stand.
pub const GROUND_SPACING_M: f32 = 0.6;

/// The cylinder a hand reaches for: `(radius, height)` in metres. The sim's
/// reach and the client's ray both use it.
pub fn pick_volume(look: u8, stage: u8) -> (f32, f32) {
    let h = LOOK_H_M.get(look as usize).copied().unwrap_or(1.0);
    let s = STAGE_SCALE_PM.get(stage as usize).copied().unwrap_or(1000) as f32 / 1000.0;
    (PLANT_PICK_R_M, (h * s).max(0.15))
}

/// Whether an eye at `eye` is within `gather::PICK_REACH_M` of the nearest
/// point of a plant's pick cylinder standing at `(x, y, z)` — the same
/// arithmetic as `gather::pick`.
pub fn in_pick_reach(look: u8, stage: u8, at: (f32, f32, f32), eye: (f32, f32, f32)) -> bool {
    let (r, h) = pick_volume(look, stage);
    let (dx, dz) = (at.0 - eye.0, at.2 - eye.2);
    let planar = ((dx * dx + dz * dz).sqrt() - r).max(0.0);
    let vertical = (at.1 - eye.1).max(eye.1 - (at.1 + h)).max(0.0);
    let reach = crate::gather::PICK_REACH_M;
    planar * planar + vertical * vertical <= reach * reach
}

// ---------------------------------------------------------------- verbs

pub const TEND_PICK: u8 = 0;
pub const TEND_CLONE: u8 = 1;
pub const TEND_CLEAR: u8 = 2;
pub const TEND_WATER: u8 = 3;

/// Why a plant left the store (`EV_PLANT_GONE.b`).
pub const GONE_PICKED: u8 = 0;
pub const GONE_CLONED: u8 = 1;
pub const GONE_CLEARED: u8 = 2;
pub const GONE_WITHERED: u8 = 3;
pub const GONE_UPROOTED: u8 = 4;

/// `PlantRec::flags`.
pub const FL_DRY: u8 = 1;
pub const FL_CROSSED: u8 = 2;

/// RNG channels (`rng::cell_hash`): the gene roll and the leaves.
pub const CH_GENES: u32 = 109;
pub const CH_LEAVES: u32 = 110;

/// Ticks between one plant's visits: the ovens' stride.
pub const FARM_PERIOD_TICKS: u64 = crate::oven::OVEN_PERIOD_TICKS;
/// Removals one tick's pass applies; past it, a plant waits for its next
/// visit.
pub const MAX_PLANT_REMOVALS_PER_TICK: usize = 64;
/// Planters found gone in one tick's pass, each uprooted whole (a scan of
/// the store); past it, a host waits for its plants' next visit.
pub const MAX_LOST_HOSTS_PER_TICK: usize = 8;
/// One saved plant (`worldsave.rs`).
pub const PLANT_BYTES: usize = 58;

// ---------------------------------------------------------------- records

/// One living plant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlantRec {
    /// Monotonic from 1, never reused: also the planting order. 0 is none.
    pub id: u32,
    /// Who planted it.
    pub owner: u32,
    /// The planter's box key, or [`HOST_GROUND`].
    pub host: u32,
    /// 0..6 in a bed, [`SPOT_GROUND`] on the ground.
    pub spot: u8,
    /// Index into `FarmContent::crops`.
    pub crop: u8,
    pub stage: u8,
    pub flags: u8,
    /// Base-5 digits, slot 0 lowest ([`gene`]).
    pub genes: u16,
    /// 1000 in a bed; the biome's soil on the ground.
    pub soil_pm: u16,
    /// `terrain::aridity` × 1000 at the soil point, cached at planting.
    pub arid_pm: u16,
    /// The soil's height over the sea, metres, cached.
    pub alt_m: i16,
    /// Growing visits with health above zero, and their summed health. Both
    /// stop together when `visits` saturates.
    pub visits: u16,
    pub health_sum: u32,
    /// The soil point, `movement::quant_xz` / `quant_y`.
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Milli-ticks of full-health growth; ripe at `grow_ticks * 1000`.
    pub grown: u32,
    /// A ground plant's own water, wu. Bed plants drink from the planter.
    pub water: u32,
    /// RIPE ends at it, DYING withers at it; 0 otherwise.
    pub deadline: u64,
}

impl PlantRec {
    /// The soil point, metres.
    pub fn pos(&self) -> (f32, f32, f32) {
        (
            self.qx as f32 * POS_XZ_Q,
            self.qy as f32 * POS_Y_Q,
            self.qz as f32 * POS_XZ_Q,
        )
    }

    /// Average health over its growing visits, per mille (1000 before any).
    pub fn avg_pm(&self) -> u32 {
        if self.visits == 0 {
            1000
        } else {
            self.health_sum / self.visits as u32
        }
    }

    /// The health band the wire shows, 0..=10.
    pub fn band(&self) -> u8 {
        (self.avg_pm() / 100).min(10) as u8
    }

    /// `EV_PLANT.b`.
    pub fn ev_b(&self) -> u32 {
        ((self.stage as u32) << 8) | self.flags as u32
    }
}

/// A plant's [`PLANT_BYTES`], little-endian, field order: what the save
/// writes and what `state_hash` folds.
pub fn pack(p: &PlantRec, out: &mut [u8; PLANT_BYTES]) {
    let mut at = 0;
    let mut put = |b: &[u8]| {
        out[at..at + b.len()].copy_from_slice(b);
        at += b.len();
    };
    put(&p.id.to_le_bytes());
    put(&p.owner.to_le_bytes());
    put(&p.host.to_le_bytes());
    put(&[p.spot, p.crop, p.stage, p.flags]);
    put(&p.genes.to_le_bytes());
    put(&p.soil_pm.to_le_bytes());
    put(&p.arid_pm.to_le_bytes());
    put(&p.alt_m.to_le_bytes());
    put(&p.visits.to_le_bytes());
    put(&p.health_sum.to_le_bytes());
    put(&p.qx.to_le_bytes());
    put(&p.qy.to_le_bytes());
    put(&p.qz.to_le_bytes());
    put(&p.grown.to_le_bytes());
    put(&p.water.to_le_bytes());
    put(&p.deadline.to_le_bytes());
}

/// [`pack`]'s inverse. Checks nothing: the save's decoder validates.
pub fn unpack(b: &[u8; PLANT_BYTES]) -> PlantRec {
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut d = [0u8; 8];
    d.copy_from_slice(&b[50..58]);
    PlantRec {
        id: u32_at(0),
        owner: u32_at(4),
        host: u32_at(8),
        spot: b[12],
        crop: b[13],
        stage: b[14],
        flags: b[15],
        genes: u16_at(16),
        soil_pm: u16_at(18),
        arid_pm: u16_at(20),
        alt_m: u16_at(22) as i16,
        visits: u16_at(24),
        health_sum: u32_at(26),
        qx: u32_at(30) as i32,
        qy: u32_at(34) as i32,
        qz: u32_at(38) as i32,
        grown: u32_at(42),
        water: u32_at(46),
        deadline: u64::from_le_bytes(d),
    }
}

/// Every living plant: dense, swap-removed, boxed.
pub struct Farm {
    entries: Box<[PlantRec; MAX_PLANTS]>,
    /// The host's last box index, a lookup accelerator only: verified
    /// against the box key on use. Never hashed, never saved.
    hint: Box<[u16; MAX_PLANTS]>,
    len: usize,
    next_id: u32,
}

impl Default for Farm {
    fn default() -> Self {
        Self::new()
    }
}

impl Farm {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(PlantRec::default()),
            hint: crate::boxed_array(u16::MAX),
            len: 0,
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[PlantRec] {
        &self.entries[..self.len]
    }

    /// The id the next plant gets.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.entries().iter().position(|p| p.id == id)
    }

    /// Store a new plant, giving it the next id. `None` when full.
    pub fn insert(&mut self, mut rec: PlantRec) -> Option<usize> {
        if self.len >= MAX_PLANTS {
            return None;
        }
        rec.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let i = self.len;
        self.entries[i] = rec;
        self.hint[i] = u16::MAX;
        self.len += 1;
        Some(i)
    }

    /// Remove entry `i`, moving the last into its place.
    pub fn swap_remove(&mut self, i: usize) -> PlantRec {
        if i >= self.len {
            return PlantRec::default();
        }
        let gone = self.entries[i];
        let last = self.len - 1;
        self.entries[i] = self.entries[last];
        self.hint[i] = self.hint[last];
        self.entries[last] = PlantRec::default();
        self.hint[last] = u16::MAX;
        self.len = last;
        gone
    }

    /// Remove entry `i` and announce it.
    pub fn remove(&mut self, i: usize, why: u8, by: u32, events: &mut EventQueue) {
        if i >= self.len {
            return;
        }
        let gone = self.swap_remove(i);
        events.push(EV_PLANT_GONE, gone.id, why as u32, by);
    }

    pub fn spot_taken(&self, host: u32, spot: u8) -> bool {
        self.entries()
            .iter()
            .any(|p| p.host == host && p.spot == spot)
    }

    pub fn has_host(&self, host: u32) -> bool {
        self.entries().iter().any(|p| p.host == host)
    }

    /// Remove every plant in `host`'s beds ([`GONE_UPROOTED`]).
    pub fn uproot_host(&mut self, host: u32, events: &mut EventQueue) {
        let mut i = self.len;
        while i > 0 {
            i -= 1;
            if self.entries[i].host == host {
                self.remove(i, GONE_UPROOTED, 0, events);
            }
        }
    }

    pub fn ground_count(&self) -> usize {
        self.entries()
            .iter()
            .filter(|p| p.host == HOST_GROUND)
            .count()
    }

    pub fn ground_count_of(&self, owner: u32) -> usize {
        self.entries()
            .iter()
            .filter(|p| p.host == HOST_GROUND && p.owner == owner)
            .count()
    }

    /// A ground plant stands within `r_q` (position quanta) of `(qx, qz)`.
    pub fn ground_near(&self, qx: i32, qz: i32, r_q: i32) -> bool {
        let rr = r_q as i64 * r_q as i64;
        self.entries().iter().any(|p| {
            let (dx, dz) = ((p.qx - qx) as i64, (p.qz - qz) as i64);
            p.host == HOST_GROUND && dx * dx + dz * dz < rr
        })
    }

    /// A load (`worldsave.rs`, which validated every record).
    pub fn restore(&mut self, recs: &[PlantRec], next_id: u32) {
        self.len = recs.len().min(MAX_PLANTS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        for p in self.entries[self.len..].iter_mut() {
            *p = PlantRec::default();
        }
        for h in self.hint.iter_mut() {
            *h = u16::MAX;
        }
        self.next_id = next_id.max(1);
    }
}

// ---------------------------------------------------------------- content

/// One crop (`cooking.toml` `[[crop]]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CropDef {
    pub look: u8,
    /// The item that plants it.
    pub seed: u16,
    /// What a pick pays, and how many at full health.
    pub yield_item: u16,
    pub count: u16,
    /// Ticks from seed to ripe at full health; 0 is inert.
    pub grow_ticks: u32,
    /// Clones a take pays (S3).
    pub clones: u8,
}

/// The farm's ruleset, baked from `cooking.toml`. Inline in `CookContent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FarmContent {
    pub crops: [CropDef; MAX_CROPS],
    pub crop_count: u8,
    /// A full waterskin, and what it is left as once poured.
    pub water_full: u16,
    pub water_empty: u16,
    /// Water units (1 wu = one plant drinking for one tick): one skin, a
    /// planter's tank, and what full rain banks per second.
    pub skin_wu: u32,
    pub planter_cap_wu: u32,
    pub rain_wu_s: u32,
    /// Where each growing stage ends, per cent of the grow, cumulative.
    pub stage_end_pct: [u8; 6],
    /// How long a ripe plant stays ripe, and how long a dying one stands.
    pub ripe_ticks: u32,
    pub dying_ticks: u32,
    /// Gene effects (S3): +g_pct% speed per G, +y_pct% yield per Y, each H
    /// closes h_pct% of the soil and temperature gaps, +w_pct% water per W.
    pub g_pct: u8,
    pub y_pct: u8,
    pub h_pct: u8,
    pub w_pct: u8,
}

impl FarmContent {
    /// No crops: nothing plants.
    pub const EMPTY: Self = Self {
        crops: [CropDef {
            look: 0,
            seed: 0,
            yield_item: 0,
            count: 0,
            grow_ticks: 0,
            clones: 0,
        }; MAX_CROPS],
        crop_count: 0,
        water_full: 0,
        water_empty: 0,
        skin_wu: 0,
        planter_cap_wu: 0,
        rain_wu_s: 0,
        stage_end_pct: [100; 6],
        ripe_ticks: 0,
        dying_ticks: 0,
        g_pct: 0,
        y_pct: 0,
        h_pct: 0,
        w_pct: 0,
    };

    /// A corn crop on the gather fixture's items (seed 1, yield 0, a full
    /// skin 2 that empties to 3), at the shipped numbers. For tests and the
    /// alloc gate; `CookContent::probe_fixture` leaves the farm EMPTY, so
    /// the parity probe has no farm.
    pub fn probe_fixture() -> Self {
        let mut f = Self::EMPTY;
        let hz = TICK_HZ;
        f.crops[0] = CropDef {
            look: LOOK_CORN,
            seed: 1,
            yield_item: 0,
            count: 4,
            grow_ticks: 900 * hz,
            clones: 2,
        };
        f.crop_count = 1;
        f.water_full = 2;
        f.water_empty = 3;
        f.skin_wu = 3600 * hz;
        f.planter_cap_wu = 7200 * hz;
        f.rain_wu_s = 6 * hz;
        f.stage_end_pct = [5, 20, 40, 45, 70, 100];
        f.ripe_ticks = 10_800 * hz;
        f.dying_ticks = 1800 * hz;
        f.g_pct = 10;
        f.y_pct = 10;
        f.h_pct = 10;
        f.w_pct = 25;
        f
    }

    /// The crop whose seed is `item`.
    pub fn crop_of_item(&self, item: u16) -> Option<u8> {
        self.crops[..(self.crop_count as usize).min(MAX_CROPS)]
            .iter()
            .position(|c| c.grow_ticks > 0 && c.seed == item)
            .map(|k| k as u8)
    }

    pub fn crop(&self, k: u8) -> Option<&CropDef> {
        self.crops[..(self.crop_count as usize).min(MAX_CROPS)].get(k as usize)
    }

    /// Water one plant drinks per farm period, wu.
    pub fn need(&self, genes: u16) -> u32 {
        let w = count(genes, GENE_W);
        (FARM_PERIOD_TICKS as u32 * (100 + w * self.w_pct as u32) / 100).max(1)
    }

    /// Growth speed, per mille.
    pub fn speed_pm(&self, genes: u16) -> u32 {
        1000 + count(genes, GENE_G) * self.g_pct as u32 * 10
    }

    /// A condition `c` (per mille) lifted by the plant's H genes.
    pub fn hardy(&self, genes: u16, c: u32) -> u32 {
        let c = c.min(1000);
        let k = (count(genes, GENE_H) * self.h_pct as u32).min(100);
        c + (1000 - c) * k / 100
    }

    /// What a pick pays.
    pub fn pay(&self, rec: &PlantRec) -> u16 {
        let Some(c) = self.crop(rec.crop) else {
            return 0;
        };
        let y = count(rec.genes, GENE_Y);
        let pay =
            c.count as u64 * rec.avg_pm() as u64 * (100 + y as u64 * self.y_pct as u64) / 100_000;
        pay.clamp(1, u16::MAX as u64) as u16
    }
}

// ---------------------------------------------------------------- tick

/// One farm period's slice (`World::tick`, right after `oven::sweep`):
/// (a) every planter whose turn it is banks rain under open sky; (b) every
/// plant whose turn it is is visited — its host checked, its ripe or dying
/// clock run, or (by day under open sky) it drinks and grows by the worst
/// of its water, soil and air. `feet_of` is a planter box's floor height,
/// `open_sky_at` whether a point is unroofed.
#[allow(clippy::too_many_arguments)]
pub fn step(
    fc: &FarmContent,
    air: &ClimateContent,
    farm: &mut Farm,
    deploys: &mut Deploys,
    tick: u64,
    night: bool,
    wx: &Wx,
    day_tick: u64,
    feet_of: &dyn Fn(&BoxRec) -> f32,
    open_sky_at: &dyn Fn(f32, f32, f32) -> bool,
    events: &mut EventQueue,
) {
    let period = FARM_PERIOD_TICKS as usize;
    let phase = (tick % FARM_PERIOD_TICKS) as usize;
    let (boxes, ovens) = deploys.oven_parts_mut();
    let tank_top = DEPLOY_VOL[ARCH_PLANTER as usize][1];

    // (a) Rain into every open planter's tank, idle ones too.
    if wx.rain > 0 && fc.rain_wu_s > 0 && fc.planter_cap_wu > 0 {
        let gain = (fc.rain_wu_s as u64 * FARM_PERIOD_TICKS * wx.rain.min(1000) as u64
            / (TICK_HZ as u64 * 1000))
            .min(u32::MAX as u64) as u32;
        for i in (phase..ovens.len()).step_by(period) {
            if ovens[i].arch != ARCH_PLANTER || ovens[i].water >= fc.planter_cap_wu {
                continue;
            }
            let (x, z) = boxes[i].xz();
            if open_sky_at(x, z, feet_of(&boxes[i]) + tank_top) {
                ovens[i].water = ovens[i].water.saturating_add(gain).min(fc.planter_cap_wu);
            }
        }
    }

    // (b) The plants. A bed whose planter is gone is uprooted whole after
    // the pass (every plant in it, not only the one visited), so no plant
    // outlives its planter by more than a period however the store moves.
    let mut removals = [(0usize, 0u8); MAX_PLANT_REMOVALS_PER_TICK];
    let mut n_rm = 0usize;
    let mut lost = [0u32; MAX_LOST_HOSTS_PER_TICK];
    let mut n_lost = 0usize;
    let mut defer = |i: usize, why: u8, n_rm: &mut usize| {
        if *n_rm < MAX_PLANT_REMOVALS_PER_TICK {
            removals[*n_rm] = (i, why);
            *n_rm += 1;
        }
    };
    for i in (phase..farm.len).step_by(period) {
        let mut rec = farm.entries[i];
        // 1. The bed's planter still stands (the decay and raid self-heal).
        let mut bed = None;
        if rec.host != HOST_GROUND {
            let is_host = |j: usize| {
                let b = &boxes[j];
                box_key(b.cx, b.cz, b.level, b.loc) == rec.host && ovens[j].arch == ARCH_PLANTER
            };
            let h = farm.hint[i] as usize;
            let found = if h < boxes.len() && is_host(h) {
                Some(h)
            } else {
                (0..boxes.len()).find(|&j| is_host(j))
            };
            match found {
                Some(j) => {
                    farm.hint[i] = j as u16;
                    bed = Some(j);
                }
                None => {
                    if !lost[..n_lost].contains(&rec.host) && n_lost < MAX_LOST_HOSTS_PER_TICK {
                        lost[n_lost] = rec.host;
                        n_lost += 1;
                    }
                    continue;
                }
            }
        }
        // 2. Ripe and dying run on their clock alone.
        if !is_growing(rec.stage) {
            if tick >= rec.deadline {
                if rec.stage == STAGE_RIPE {
                    rec.stage = STAGE_DYING;
                    rec.deadline = tick + fc.dying_ticks as u64;
                    farm.entries[i] = rec;
                    events.push(EV_PLANT, rec.id, rec.ev_b(), i as u32);
                } else {
                    defer(i, GONE_WITHERED, &mut n_rm);
                }
            }
            continue;
        }
        // 3. Growing: by daylight under open sky, or not at all.
        let Some(crop) = fc.crop(rec.crop).copied().filter(|c| c.grow_ticks > 0) else {
            continue;
        };
        if night {
            continue;
        }
        let (x, y, z) = rec.pos();
        if !open_sky_at(x, z, y) {
            continue;
        }
        let need = fc.need(rec.genes);
        let water_pm = {
            let bank = match bed {
                Some(h) => {
                    // A planter drinks a skin only while a plant thirsts.
                    if ovens[h].water < need && fc.skin_wu > 0 {
                        if let Some(s) = boxes[h].items[crate::oven::fuel_slots(ARCH_PLANTER)]
                            .iter_mut()
                            .find(|s| s.count > 0 && s.item == fc.water_full)
                        {
                            s.item = fc.water_empty;
                            ovens[h].water = ovens[h]
                                .water
                                .saturating_add(fc.skin_wu)
                                .min(fc.planter_cap_wu.max(fc.skin_wu));
                        }
                    }
                    &mut ovens[h].water
                }
                None => &mut rec.water,
            };
            let pm = if *bank >= need {
                1000
            } else {
                (*bank as u64 * 1000 / need as u64) as u32
            };
            *bank -= (*bank).min(need);
            pm
        };
        let temp_pm = crate::climate::crop_pm(
            air,
            crate::climate::air_dc_at(air, rec.arid_pm, rec.alt_m as i32, day_tick, wx),
        ) as u32;
        let health = water_pm
            .min(fc.hardy(rec.genes, rec.soil_pm as u32))
            .min(fc.hardy(rec.genes, temp_pm));
        let mut changed = false;
        let dry = water_pm == 0;
        if dry != (rec.flags & FL_DRY != 0) {
            rec.flags ^= FL_DRY;
            changed = true;
        }
        let band0 = rec.band();
        if health > 0 {
            let add = FARM_PERIOD_TICKS * health as u64 * fc.speed_pm(rec.genes) as u64 / 1000;
            rec.grown = (rec.grown as u64 + add).min(u32::MAX as u64) as u32;
            if rec.visits < u16::MAX {
                rec.visits += 1;
                rec.health_sum = rec.health_sum.saturating_add(health);
            }
        }
        let full = crop.grow_ticks as u64 * 1000;
        while is_growing(rec.stage)
            && rec.grown as u64 * 100 >= fc.stage_end_pct[rec.stage as usize] as u64 * full
        {
            rec.stage += 1;
            changed = true;
            if rec.stage == STAGE_RIPE {
                rec.deadline = tick + fc.ripe_ticks as u64;
            }
        }
        if rec.band() != band0 {
            changed = true;
        }
        farm.entries[i] = rec;
        if changed {
            events.push(EV_PLANT, rec.id, rec.ev_b(), i as u32);
        }
    }
    // Descending, so every pending index is still where it was found.
    for k in (0..n_rm).rev() {
        let (i, why) = removals[k];
        farm.remove(i, why, 0, events);
    }
    for &host in &lost[..n_lost] {
        farm.uproot_host(host, events);
    }
}

// ---------------------------------------------------------------- verbs

/// Plant one unit of `p.inv[slot]` in spot `spot` of the planter `b`
/// (`Command::Plant`). The caller has checked the planter is one and is in
/// reach; `feet_y` is its floor (`col_base_y + level_y`). Returns the new
/// plant's store index, or a `REFUSE_D_*`.
#[allow(clippy::too_many_arguments)]
pub fn plant_bed(
    fc: &FarmContent,
    seed: u64,
    farm: &mut Farm,
    b: &BoxRec,
    feet_y: f32,
    p: &mut Player,
    slot: usize,
    spot: u8,
    events: &mut EventQueue,
) -> Result<usize, u32> {
    let stack = p.inv.get(slot).copied().unwrap_or_default();
    let Some(crop) = fc.crop_of_item(stack.item).filter(|_| stack.count > 0) else {
        return Err(REFUSE_D_PLANT_ITEM);
    };
    let host = box_key(b.cx, b.cz, b.level, b.loc);
    if spot >= PLANTER_SPOTS || farm.spot_taken(host, spot) {
        return Err(REFUSE_D_PLANT_SPOT);
    }
    if farm.len() >= MAX_PLANTS {
        return Err(REFUSE_D_PLANTS_FULL);
    }
    let (x, z) = spot_xz(b.cx, b.cz, b.pose, spot);
    let y = feet_y + DEPLOY_VOL[ARCH_PLANTER as usize][1] - BED_SOIL_DROP_M;
    let rec = PlantRec {
        owner: p.id,
        host,
        spot,
        crop,
        stage: STAGE_SEED,
        soil_pm: 1000,
        arid_pm: (crate::terrain::aridity(seed, x, z) * 1000.0).clamp(0.0, 1000.0) as u16,
        alt_m: (y - crate::terrain::SEA_LEVEL).clamp(-1000.0, 10_000.0) as i16,
        qx: crate::movement::quant_xz(x),
        qy: crate::movement::quant_y(y),
        qz: crate::movement::quant_xz(z),
        ..PlantRec::default()
    };
    let Some(i) = farm.insert(rec) else {
        return Err(REFUSE_D_PLANTS_FULL);
    };
    let s = &mut p.inv[slot];
    s.count -= 1;
    if s.count == 0 {
        *s = ItemStack::default();
    }
    let rec = farm.entries()[i];
    events.push(EV_PLANT, rec.id, rec.ev_b(), i as u32);
    Ok(i)
}

/// Tend plant `i` (`Command::Tend`): pick it, take a clone, clear it, or
/// water it. The caller has checked reach. What will not fit the pack lands
/// in `spill` for the caller to drain.
#[allow(clippy::too_many_arguments)]
pub fn tend(
    fc: &FarmContent,
    gc: &GatherContent,
    farm: &mut Farm,
    deploys: &mut Deploys,
    p: &mut Player,
    i: usize,
    act: u8,
    spill: &mut [ItemStack; INV_SLOTS],
    events: &mut EventQueue,
) -> Result<(), u32> {
    let Some(rec) = farm.entries().get(i).copied() else {
        return Err(REFUSE_D_SPOT);
    };
    match act {
        TEND_PICK => {
            if rec.stage != STAGE_RIPE {
                return Err(REFUSE_D_PLANT_STAGE);
            }
            let Some(crop) = fc.crop(rec.crop).copied() else {
                return Err(REFUSE_D_PLANT_STAGE);
            };
            let pay = fc.pay(&rec);
            let added = crate::gather::inv_add_spilling(
                &mut p.inv,
                spill,
                crop.yield_item,
                pay,
                gc.stack_max_of(crop.yield_item),
                gc.cond_max_of(crop.yield_item),
            );
            events.push(
                EV_GATHER,
                p.id,
                ((crop.yield_item as u32) << 16) | added as u32,
                (pay - added) as u32,
            );
            farm.remove(i, GONE_PICKED, p.id, events);
            Ok(())
        }
        // Clones land with genes (S3).
        TEND_CLONE => Err(REFUSE_D_PLANT_STAGE),
        TEND_CLEAR => {
            if rec.stage != STAGE_DYING {
                return Err(REFUSE_D_PLANT_STAGE);
            }
            farm.remove(i, GONE_CLEARED, p.id, events);
            Ok(())
        }
        TEND_WATER => {
            // Ground plants drink from their own bank (S4).
            if rec.host == HOST_GROUND {
                return Err(REFUSE_D_PLANT_SOIL);
            }
            let sel = p.frame.sel as usize;
            let hand = p.inv.get(sel).copied().unwrap_or_default();
            if fc.skin_wu == 0 || hand.count == 0 || hand.item != fc.water_full {
                return Err(REFUSE_D_PLANT_ITEM);
            }
            let Some(h) = deploys.box_index(rec.host) else {
                return Err(REFUSE_D_SPOT);
            };
            let (_, ovens) = deploys.oven_parts_mut();
            let bank = &mut ovens[h].water;
            let room = fc.planter_cap_wu.saturating_sub(*bank);
            if room < fc.skin_wu / 4 {
                return Err(REFUSE_D_PLANT_WATER);
            }
            *bank += fc.skin_wu.min(room);
            p.inv[sel].item = fc.water_empty;
            Ok(())
        }
        _ => Err(REFUSE_D_KIND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genes_round_trip_and_count() {
        let mut g = 0u16;
        for (k, l) in [GENE_G, GENE_G, GENE_Y, GENE_H, GENE_W, GENE_X]
            .iter()
            .enumerate()
        {
            g = with_gene(g, k, *l);
        }
        assert_eq!(&letters(g), b"GGYHWX");
        assert_eq!(count(g, GENE_G), 2);
        assert_eq!(count(g, GENE_X), 1);
        assert!((g as u32) < GENOME_VALUES);
        assert_eq!(with_gene(g, 0, GENE_X), g - GENE_G as u16);
    }

    #[test]
    fn spots_lie_on_the_bed_and_touch_their_neighbours() {
        for s in 0..PLANTER_SPOTS {
            let (lx, lz) = spot_local(s);
            assert!(
                crate::fmath::fabs(lx) < 0.8 && crate::fmath::fabs(lz) < 0.4,
                "spot {s} off the bed"
            );
        }
        assert!(adjacent(0, 4) && adjacent(1, 5) && !adjacent(0, 2) && !adjacent(3, 3));
    }

    #[test]
    fn a_pick_pays_the_count_at_full_health_and_never_nothing() {
        let fc = FarmContent::probe_fixture();
        let mut r = PlantRec::default();
        assert_eq!(fc.pay(&r), 4);
        r.visits = 10;
        r.health_sum = 10 * 550;
        assert_eq!(fc.pay(&r), 2);
        r.health_sum = 10;
        assert_eq!(fc.pay(&r), 1);
    }

    #[test]
    fn a_plant_packs_into_its_bytes_and_back() {
        let p = PlantRec {
            id: 7,
            owner: 9,
            host: 0x1234_5678,
            spot: 4,
            crop: 1,
            stage: STAGE_RIPE,
            flags: FL_DRY,
            genes: 15_000,
            soil_pm: 550,
            arid_pm: 12,
            alt_m: -3,
            visits: 600,
            health_sum: 590_000,
            qx: -5,
            qy: 77,
            qz: 123_456,
            grown: 27_000_000,
            water: 99,
            deadline: u64::MAX - 1,
        };
        let mut b = [0u8; PLANT_BYTES];
        pack(&p, &mut b);
        assert_eq!(unpack(&b), p);
    }
}
