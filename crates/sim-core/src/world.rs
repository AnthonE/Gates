//! World state, the command buffer, the tick, and `state_hash` (DESIGN.md
//! §4/§7). Fixed-capacity everything: no allocation anywhere in this module,
//! at construction or in the tick. All mutation flows through `Command`s
//! applied in submission order, then players step in slot order — the fixed
//! order determinism requires.

use crate::craft::{self, CraftContent, CraftJob};
use crate::gather::{self, GatherContent, ItemStack, SlotLives, NO_CELL};
use crate::input::InputFrame;
use crate::limits::{
    CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_COMMANDS_PER_TICK, MAX_EVENTS_PER_TICK, MAX_PLAYERS,
    STATE_HASH_INTERVAL,
};
use crate::movement::{self, Body};
use crate::rng::cell_hash;
use crate::terrain::{self, ScatterTable};
use xxhash_rust::xxh3::Xxh3;

/// Noise channel reserved for spawn-point selection.
const CH_SPAWN: u32 = 96;

/// Integer event codes (CLAUDE.md wall 3) — the sim's outbound facts, one
/// ring per tick, drained by the server after `tick` returns.
/// EV_GATHER: a = player id, b = item index << 16 | units actually added.
pub const EV_GATHER: u8 = 1;
/// EV_SLOT_HARVESTED: a = cell key (cx << 16 | cz), b = gatherable index.
pub const EV_SLOT_HARVESTED: u8 = 2;
/// EV_SLOT_RESPAWNED: a = cell key, b = 0.
pub const EV_SLOT_RESPAWNED: u8 = 3;
/// EV_WEAK_MARK: a = player id, b = cell key, c = weak-hit bit << 8 |
/// next mark heading (u8 over the 256-entry yaw LUT). Swinger-only fact:
/// the mark is per-player (gather.rs).
pub const EV_WEAK_MARK: u8 = 4;
/// EV_CRAFT_DONE: a = player id, b = item index << 16 | units actually
/// added (0 = full inventory; the loss is announced, never silent).
pub const EV_CRAFT_DONE: u8 = 5;
/// EV_CRAFT_REFUSED: a = player id, b = `craft::REFUSE_*` reason code.
pub const EV_CRAFT_REFUSED: u8 = 6;

#[derive(Clone, Copy, Debug, Default)]
pub struct SimEvent {
    pub code: u8,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

/// Per-tick event ring (limits.rs: MAX_EVENTS_PER_TICK, drop newest,
/// counted). Derived output, not sim state — it stays out of state_hash
/// the way `last_hash` does.
pub struct EventQueue {
    entries: [SimEvent; MAX_EVENTS_PER_TICK],
    len: usize,
    /// Events refused by a full ring since the last clear (diagnostic).
    pub dropped: u32,
}

impl EventQueue {
    pub fn push(&mut self, code: u8, a: u32, b: u32, c: u32) {
        if self.len == MAX_EVENTS_PER_TICK {
            self.dropped += 1;
            return;
        }
        self.entries[self.len] = SimEvent { code, a, b, c };
        self.len += 1;
    }

    pub fn entries(&self) -> &[SimEvent] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn clear(&mut self) {
        self.len = 0;
        self.dropped = 0;
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self {
            entries: [SimEvent::default(); MAX_EVENTS_PER_TICK],
            len: 0,
            dropped: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub id: u32,
    pub active: bool,
    pub body: Body,
    /// Last applied input — sim state, so input-reuse replays for free.
    pub frame: InputFrame,
    /// 6 hotbar + 24 backpack (ALPHA.md §1). A join starts empty — the
    /// naked spawn punches its first resources (gatherables' hand rows).
    pub inv: [ItemStack; INV_SLOTS],
    /// Tick the next swing is allowed at (gather.rs cadence).
    pub next_swing: u64,
    /// Weak-spot chase: the cell this player last landed a hit on
    /// (`NO_CELL` = none) and how many hits they've landed there. The mark
    /// heading derives from these (gather.rs), so they are sim state.
    pub ws_cell: u32,
    pub ws_hits: u16,
    /// Craft queue, dense with the head at 0 (craft.rs). Sim state.
    pub jobs: [CraftJob; CRAFT_QUEUE],
    /// Tick the head job's current unit completes at; 0 = idle.
    pub craft_done_at: u64,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            id: 0,
            active: false,
            body: Body::default(),
            frame: InputFrame::default(),
            inv: [ItemStack::default(); INV_SLOTS],
            next_swing: 0,
            ws_cell: NO_CELL,
            ws_hits: 0,
            jobs: [CraftJob::default(); CRAFT_QUEUE],
            craft_done_at: 0,
        }
    }
}

/// Every mutation the sim accepts. The WAL is exactly this stream plus the
/// tick numbers (DESIGN.md §7).
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Join {
        id: u32,
    },
    Leave {
        id: u32,
    },
    Input {
        id: u32,
        frame: InputFrame,
    },
    /// Enqueue `count` crafts of recipe row `recipe` (craft.rs validates
    /// and refuses by event, never by panic).
    Craft {
        id: u32,
        recipe: u16,
        count: u16,
    },
    /// Cancel the queue job at `index`, refunding its remaining inputs.
    CraftCancel {
        id: u32,
        index: u16,
    },
}

pub struct World {
    pub seed: u64,
    pub tick: u64,
    pub players: [Player; MAX_PLAYERS],
    pub scatter: ScatterTable,
    /// Baked gather rules (gather.rs). Construction input like `seed`:
    /// inert until the boot path installs the table baked from
    /// `content/*.toml`, before the first tick. The WAL pins the content
    /// hash it was baked from when the WAL file format lands.
    pub gather: GatherContent,
    /// Baked recipe rules (craft.rs). Construction input like `gather`.
    pub craft: CraftContent,
    /// Sparse harvested/damaged slot records (TERRAIN.md §2).
    pub slot_lives: SlotLives,
    /// This tick's outbound events; cleared at tick start.
    pub events: EventQueue,
    /// Hash stamped every `STATE_HASH_INTERVAL` ticks (0 until the first).
    pub last_hash: u64,
    /// Dev-only fixed spawn override in meters (DECISIONS.md §open). None
    /// (the default) is the shipping behavior: scattered `spawn_pos`. Set
    /// only from `shard.toml dev_spawn` — it exists so a test can put two
    /// clients inside AOI range on demand. Config, not state: it is world
    /// construction input like `seed`, so it stays out of `state_hash`;
    /// when the WAL file format lands, it pins into the header beside the
    /// seed so replays reproduce the spawns they were played under.
    pub dev_spawn: Option<(f32, f32)>,
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            tick: 0,
            players: [Player::default(); MAX_PLAYERS],
            scatter: ScatterTable::alpha_default(),
            gather: GatherContent::EMPTY,
            craft: CraftContent::EMPTY,
            slot_lives: SlotLives::new(),
            events: EventQueue::default(),
            last_hash: 0,
            dev_spawn: None,
        }
    }

    /// Deterministic spawn: hashed candidates over the island interior,
    /// first walkable one wins; island center as the total-miss fallback.
    /// The beach spawn ring proper is a later worldgen slice.
    pub fn spawn_pos(&self, id: u32) -> (f32, f32) {
        if let Some(p) = self.dev_spawn {
            return p;
        }
        let mut attempt = 0i32;
        while attempt < 96 {
            let h = cell_hash(self.seed, id as i32, attempt, CH_SPAWN);
            let x = 224.0 + (h % 1600) as f32;
            let z = 224.0 + ((h >> 32) % 1600) as f32;
            let hy = terrain::height(self.seed, x, z);
            if (1.5..45.0).contains(&hy) && terrain::slope(self.seed, x, z) < 1.0 {
                return (x, z);
            }
            attempt += 1;
        }
        (terrain::ISLAND_SIZE * 0.5, terrain::ISLAND_SIZE * 0.5)
    }

    fn slot_of(&self, id: u32) -> Option<usize> {
        self.players.iter().position(|p| p.active && p.id == id)
    }

    fn apply(&mut self, cmd: &Command) {
        match *cmd {
            Command::Join { id } => {
                if self.slot_of(id).is_some() {
                    return;
                }
                if let Some(slot) = self.players.iter().position(|p| !p.active) {
                    let (x, z) = self.spawn_pos(id);
                    self.players[slot] = Player {
                        id,
                        active: true,
                        body: Body::at(self.seed, x, z),
                        ..Player::default()
                    };
                }
                // No free slot: refuse silently here; the accept path
                // already hard-caps at the shard limit (limits.rs).
            }
            Command::Leave { id } => {
                if let Some(slot) = self.slot_of(id) {
                    self.players[slot].active = false;
                }
            }
            Command::Input { id, frame } => {
                if let Some(slot) = self.slot_of(id) {
                    let mut frame = frame;
                    if frame.sel as usize >= HOTBAR_SLOTS {
                        // The wire refuses 6–7 at decode; a non-wire
                        // command (bot, test, WAL) falls back to slot 0.
                        frame.sel = 0;
                    }
                    self.players[slot].frame = frame;
                }
            }
            Command::Craft { id, recipe, count } => {
                if let Some(slot) = self.slot_of(id) {
                    craft::enqueue(
                        &self.craft,
                        self.tick,
                        &mut self.players[slot],
                        recipe,
                        count,
                        &mut self.events,
                    );
                }
            }
            Command::CraftCancel { id, index } => {
                if let Some(slot) = self.slot_of(id) {
                    craft::cancel(
                        &self.craft,
                        &self.gather,
                        self.tick,
                        &mut self.players[slot],
                        index,
                    );
                }
            }
        }
    }

    /// One fixed tick: apply at most `MAX_COMMANDS_PER_TICK` commands in
    /// order (overflow policy: defer — the caller keeps the tail), step
    /// every active player in slot order (move, then swing), release due
    /// respawns, stamp the hash on cadence.
    pub fn tick(&mut self, commands: &[Command]) {
        self.events.clear();
        for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK) {
            self.apply(cmd);
        }
        let seed = self.seed;
        let tick = self.tick;
        for p in self.players.iter_mut() {
            if p.active {
                movement::step(seed, &mut p.body, &p.frame);
                gather::swing(
                    seed,
                    tick,
                    &self.gather,
                    &self.scatter,
                    &mut self.slot_lives,
                    &mut self.events,
                    p,
                );
                craft::step(&self.craft, &self.gather, tick, p, &mut self.events);
            }
        }
        self.slot_lives.respawn_due(tick, &mut self.events);
        self.tick += 1;
        if self.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            self.last_hash = self.state_hash();
        }
    }

    /// xxh3 over canonical sim state, allocation-free. Slot order is the
    /// canonical order. `dev_spawn` and the baked `gather` table are
    /// construction input, not state — they influence the sim the way
    /// `seed` does, and pin alongside it (seed + content hash in the WAL
    /// header). The event ring is derived output and stays out.
    pub fn state_hash(&self) -> u64 {
        let mut h = Xxh3::new();
        h.update(&self.seed.to_le_bytes());
        h.update(&self.tick.to_le_bytes());
        for p in self.players.iter() {
            if !p.active {
                continue;
            }
            let mut buf = [0u8; 48];
            buf[0..4].copy_from_slice(&p.id.to_le_bytes());
            buf[4..8].copy_from_slice(&p.body.qx.to_le_bytes());
            buf[8..12].copy_from_slice(&p.body.qy.to_le_bytes());
            buf[12..16].copy_from_slice(&p.body.qz.to_le_bytes());
            buf[16..20].copy_from_slice(&p.body.qvy.to_le_bytes());
            buf[20] = p.body.grounded as u8;
            buf[21..23].copy_from_slice(&p.frame.seq.to_le_bytes());
            buf[23] = p.frame.buttons;
            buf[24..26].copy_from_slice(&p.frame.yaw.to_le_bytes());
            buf[26] = p.frame.pitch;
            buf[27] = p.frame.move_x as u8;
            buf[28] = p.frame.move_z as u8;
            buf[29..37].copy_from_slice(&p.next_swing.to_le_bytes());
            buf[37] = p.frame.sel;
            buf[38..42].copy_from_slice(&p.ws_cell.to_le_bytes());
            buf[42..44].copy_from_slice(&p.ws_hits.to_le_bytes());
            h.update(&buf);
            for s in p.inv.iter() {
                let mut sb = [0u8; 4];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                h.update(&sb);
            }
            let mut cb = [0u8; 8 + CRAFT_QUEUE * 4];
            cb[0..8].copy_from_slice(&p.craft_done_at.to_le_bytes());
            for (j, job) in p.jobs.iter().enumerate() {
                cb[8 + j * 4..8 + j * 4 + 2].copy_from_slice(&job.recipe.to_le_bytes());
                cb[8 + j * 4 + 2..8 + j * 4 + 4].copy_from_slice(&job.remaining.to_le_bytes());
            }
            h.update(&cb);
        }
        h.update(&(self.slot_lives.len() as u64).to_le_bytes());
        for e in self.slot_lives.entries() {
            let mut buf = [0u8; 16];
            buf[0..2].copy_from_slice(&e.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&e.cz.to_le_bytes());
            buf[4..6].copy_from_slice(&e.hits.to_le_bytes());
            buf[8..16].copy_from_slice(&e.respawn_at.to_le_bytes());
            h.update(&buf);
        }
        h.digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain;

    /// The point and seed `ci/browser_smoke.mjs` puts both tabs on. Guarded
    /// here natively so a worldgen change that sinks or steepens it fails
    /// this test, not the browser gate.
    const SMOKE_SEED: u64 = 20260731;
    const SMOKE_SPAWN: (f32, f32) = (1024.0, 1024.0);

    #[test]
    fn dev_spawn_overrides_every_join() {
        let mut w = World::new(SMOKE_SEED);
        w.dev_spawn = Some(SMOKE_SPAWN);
        w.tick(&[Command::Join { id: 7 }, Command::Join { id: 8 }]);
        for id in [7u32, 8] {
            let p = w.players.iter().find(|p| p.active && p.id == id).unwrap();
            // Body::at quantizes at 3 cm x/z: exact in quantized space.
            assert_eq!(p.body.qx, movement::quant_xz(SMOKE_SPAWN.0));
            assert_eq!(p.body.qz, movement::quant_xz(SMOKE_SPAWN.1));
        }
        // And None still scatters: two ids land apart.
        let mut w2 = World::new(SMOKE_SEED);
        w2.tick(&[Command::Join { id: 7 }, Command::Join { id: 8 }]);
        let a = w2.players[0].body;
        let b = w2.players[1].body;
        assert!(a.qx != b.qx || a.qz != b.qz);
    }

    #[test]
    fn smoke_spawn_point_is_walkable() {
        let (x, z) = SMOKE_SPAWN;
        let h = terrain::height(SMOKE_SEED, x, z);
        let s = terrain::slope(SMOKE_SEED, x, z);
        assert!(
            (1.5..45.0).contains(&h) && s < 1.0,
            "browser-smoke spawn ({x},{z}) unwalkable at seed {SMOKE_SEED}: height {h} slope {s}"
        );
    }
}
