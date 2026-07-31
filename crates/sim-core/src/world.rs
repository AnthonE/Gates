//! World state, the command buffer, the tick, and `state_hash` (DESIGN.md
//! §4/§7). Fixed-capacity everything: no allocation anywhere in this module,
//! at construction or in the tick. All mutation flows through `Command`s
//! applied in submission order, then players step in slot order — the fixed
//! order determinism requires.

use crate::input::InputFrame;
use crate::limits::{MAX_COMMANDS_PER_TICK, MAX_PLAYERS, STATE_HASH_INTERVAL};
use crate::movement::{self, Body};
use crate::rng::cell_hash;
use crate::terrain::{self, ScatterTable};
use xxhash_rust::xxh3::Xxh3;

/// Noise channel reserved for spawn-point selection.
const CH_SPAWN: u32 = 96;

#[derive(Clone, Copy, Debug, Default)]
pub struct Player {
    pub id: u32,
    pub active: bool,
    pub body: Body,
    /// Last applied input — sim state, so input-reuse replays for free.
    pub frame: InputFrame,
}

/// Every mutation the sim accepts. The WAL is exactly this stream plus the
/// tick numbers (DESIGN.md §7).
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Join { id: u32 },
    Leave { id: u32 },
    Input { id: u32, frame: InputFrame },
}

pub struct World {
    pub seed: u64,
    pub tick: u64,
    pub players: [Player; MAX_PLAYERS],
    pub scatter: ScatterTable,
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
                        frame: InputFrame::default(),
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
                    self.players[slot].frame = frame;
                }
            }
        }
    }

    /// One fixed tick: apply at most `MAX_COMMANDS_PER_TICK` commands in
    /// order (overflow policy: defer — the caller keeps the tail), step
    /// every active player in slot order, stamp the hash on cadence.
    pub fn tick(&mut self, commands: &[Command]) {
        for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK) {
            self.apply(cmd);
        }
        let seed = self.seed;
        for p in self.players.iter_mut() {
            if p.active {
                movement::step(seed, &mut p.body, &p.frame);
            }
        }
        self.tick += 1;
        if self.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            self.last_hash = self.state_hash();
        }
    }

    /// xxh3 over canonical sim state, allocation-free. Slot order is the
    /// canonical order. `dev_spawn` is construction input, not state — it
    /// influences spawns the way `seed` does, and pins alongside it.
    pub fn state_hash(&self) -> u64 {
        let mut h = Xxh3::new();
        h.update(&self.seed.to_le_bytes());
        h.update(&self.tick.to_le_bytes());
        for p in self.players.iter() {
            if !p.active {
                continue;
            }
            let mut buf = [0u8; 32];
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
