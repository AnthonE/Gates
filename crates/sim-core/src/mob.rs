//! Animals — the roster, the wander, and the kill (`reference/ANIMALS.md`).
//!
//! **Two species: prey and a predator.** This header said "one species at
//! v0 and the whole file is written so a second costs a content row and
//! nothing else" for three days, and adding the second is what measured the
//! claim. It was two thirds right. The *behaviour* was genuinely free — a
//! hunter is `brave_pct = 0` (courage that never runs out, so the rousing
//! is always a charge) and a notice radius wide enough to start one across
//! open ground, both numbers in `content/mobs.toml` and not one branch in
//! this file. What was NOT free is the roster itself: `MOB_KINDS` was 1,
//! `Mobs::new` wrote `MOB_PIG` into all 64 slots, and the bake matched one
//! string. So the honest form of the old claim, which is now true, is: a
//! third species costs a content row, a bake arm, and an ordinal — and no
//! behaviour, unless it wants a mechanism these two do not have (packs,
//! and a blind spot, are the two `ANIMALS.md` §9.5 still names).
//!
//! **What separates a predator from brave prey is the reaction, not the
//! bite.** One rousing timer serves both and always did: a player inside
//! `spook_cm` starts it, and `brave_pct` decides which way the animal then
//! points. A pig is whole-hearted at full health and a coward under half of
//! it; a wolf's floor is zero, so the same state is a charge at 1 hp. The
//! bite is `attack != 0` and neither species has a code path the other
//! lacks.
//!
//! **There is no navmesh here and there is not going to be one**, which is
//! the single biggest thing the research changed. The reference game bakes
//! one on server start, at 100% CPU for minutes, and spent devblogs cutting
//! its resolution to claw the boot time back (`reference/ANIMALS.md` §2).
//! It has to: its world is a *file*, so the walkable surface is not knowable
//! without walking it. Ours is a pure function — `terrain::height` and
//! `terrain::slope` answer at any point for the cost of a hash — so an
//! animal can steer and let `movement::step` say yes or no. That is not a
//! cheaper navmesh, it is the absence of the problem a navmesh solves.
//!
//! **The animal drives the same capsule a player does.** A think tick sets
//! a heading and a gait; the step builds an `InputFrame` out of them and
//! calls `movement::step`. Every wall, cliff ratio, step-up, wade slowdown
//! and tree the player collides with is therefore already true of the pig,
//! for free, in the code the client predicts with — and the position it
//! reaches is quantized by the same function, so the animal obeys
//! quantize-both-sides without a second implementation to keep in step. The
//! gait is what makes this honest rather than a shortcut: `InputFrame`'s
//! move axis is `−127..=127` and `movement::step` scales speed by
//! `move_z / 127`, so a grazing pig at gait 63 walks at 1.49 m/s out of the
//! same `WALK_SPEED` a player sprints away from.
//!
//! **Bounded, per wall 4, and in two directions at once.** The roster is
//! `MAX_MOBS` fixed slots — no spawn path can ask for a sixty-fifth pig.
//! Decisions are phase-offset across `MOB_THINK_TICKS` (≈ 4 animals think
//! per tick, not 64), and an animal with no player inside `MOB_WAKE_CM`
//! does not step at all. Both of those are the reference game's own
//! measures — a fixed think rate and dormancy at distance — and both are
//! replay-safe here because their predicates are sim state and nothing
//! else.
//!
//! Every number in `MobDef` is content (`content/mobs.toml`, wall 7); the
//! constants in this file are structure — cadences, caps and the shape of
//! the walk — and are registered in `DECISIONS.md` §open ("animals v0").

use crate::backpack::{BackpackContent, Backpacks, BAG_Y_OFFSET_Q};
use crate::collide::ColIndex;
use crate::combat::{held_item, CombatContent};
use crate::fmath::fabs;
use crate::gather::{ItemStack, CONE_COS, DY_MAX_M, NO_ITEM, POINT_BLANK_M2};
use crate::input::{InputFrame, BTN_SPRINT};
use crate::limits::{INV_SLOTS, MAX_MOBS, MAX_PLAYERS, MOB_ID_TAG, MOB_THINK_TICKS, MOB_WAKE_CM};
use crate::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use crate::occupy::Occupants;
use crate::rng::cell_hash;
use crate::terrain::{self, Haven};
use crate::world::{EventQueue, Player, EV_HIT};
use crate::yaw_lut::yaw_dir;

/// Species ordinals. The wire does not carry one — a client reads the
/// species off the roster slot the id names, because the roster's kinds are
/// worldgen and worldgen is shared.
pub const MOB_PIG: u8 = 0;
pub const MOB_WOLF: u8 = 1;
pub const MOB_KINDS: usize = 2;

/// One roster slot in this many is a predator (`DECISIONS.md` §open,
/// "predator v0"). 64 slots at 1-in-4 is **16 wolves and 48 pigs**, exactly,
/// on every seed.
///
/// A stride and not a hashed draw, which is the bounded form wall 4 wants:
/// the predator count is a stated number a gate can count rather than a
/// distribution a gate has to sample. What the seed still varies is *where*
/// they live — `home_of` draws per slot — so two shards agree on how many
/// wolves exist and on nothing else about them.
const WOLF_SLOT_EVERY: usize = 4;

/// Which species a roster slot holds.
///
/// **This function is why the wire carries no species field.** `protocol`
/// v29 rejected a `kind` on `EntityState` on cost — bits on every record of
/// every snapshot, for a value that never changes for the life of an
/// entity — and the rejection is only affordable because the answer is
/// derivable on both sides. A client masks a mob id down to its slot
/// (`slot_of_id`) and asks here; the sim asks here at construction. There
/// is no handshake field, no snapshot bit, and no way for the two sides to
/// disagree about what is chasing you.
///
/// It takes no seed on purpose. It could — `home_of` does — but a species
/// that varied per seed would be a fact the client cannot check against
/// anything, and the roster's *positions* already carry all the per-seed
/// variety a world needs.
#[inline]
pub const fn kind_of(slot: usize) -> u8 {
    if slot.is_multiple_of(WOLF_SLOT_EVERY) {
        MOB_WOLF
    } else {
        MOB_PIG
    }
}

/// Item rows one species may drop. Structural cap, not a knob: the bake
/// refuses a longer table rather than truncating one.
pub const MOB_LOOT_ROWS: usize = 4;

/// Noise channels. Disjoint from every channel `terrain.rs` and `world.rs`
/// spend, which is the only property that matters about them.
const CH_MOB_HOME: u32 = 112;
const CH_MOB_THINK: u32 = 113;

/// Candidate homes drawn before a roster slot gives up and stays empty.
/// Bounded like the spawn ring's `SPAWN_CANDIDATES` and for the same
/// reason: this runs at world construction, once, and a loop that retried
/// until it succeeded would be a hang on a seed with no land.
const HOME_TRIES: i32 = 24;

/// Ground an animal will stand on. Same walkability shape the spawn ring
/// and the foundation use, so a pig's home is somewhere a player could
/// have built.
const HOME_MAX_SLOPE: f32 = 1.0;

/// One species, as content sees it. Everything here is a number from
/// `content/mobs.toml`; nothing in this struct is chosen in code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MobDef {
    /// Hit points at hatch. **Zero means inert** — the same door
    /// `CombatContent::player_hp` uses: a content set that names no
    /// animals leaves the roster empty and nothing hatches, so a shard can
    /// run without wildlife and no code path has to ask whether it should.
    pub hp: u16,
    /// `InputFrame::move_z` while roaming, `0..=127` — the fraction of
    /// `WALK_SPEED` this animal ambles at.
    pub gait: u8,
    /// `move_z` while roused — fleeing *or* charging, one fast gait. The
    /// sprint button rides with it, so the speed ceiling is `SPRINT_SPEED`
    /// rather than `WALK_SPEED`.
    pub flee_gait: u8,
    /// How long one rousing lasts, in ticks — the fright's span and the
    /// charge's commitment, one number.
    pub flee_ticks: u16,
    /// Damage per bite. **Zero means this species never fights** — the
    /// same inert door `hp == 0` is for the whole row, so a pacifist
    /// animal is a content row and not a code path.
    pub attack: u16,
    /// Bite reach, planar centimetres.
    pub attack_range_cm: i64,
    /// Ticks between bites. The cooldown is phase-locked — a bite lands
    /// on ticks where `tick % attack_ticks == slot % attack_ticks` — so
    /// it needs no per-mob timer, no new hashed state, and replays for
    /// free (the same trick `MOB_THINK_TICKS` plays with thinking).
    pub attack_ticks: u16,
    /// Percent of max hp at which courage runs out: at or above it a
    /// roused animal charges, below it the same rousing is a flight.
    /// Their boar's own rule — aggressive when whole, flees hurt.
    ///
    /// **Zero is the predator**, and it is the whole of what makes one: a
    /// floor of zero is a floor no wound can go under, so every rousing is
    /// a charge and the animal never breaks off. Prey and hunter differ by
    /// this number and by how far away they notice you, and by nothing in
    /// this file.
    pub brave_pct: u8,
    /// Leash: planar centimetres from the home the seed chose. Past it the
    /// animal heads home instead of wandering — the reference game's own
    /// fix for animals that "ended up at the coast of the island"
    /// (`reference/ANIMALS.md` §2), and the reason nothing here needs to
    /// know how to swim.
    pub roam_cm: i64,
    /// A player closer than this, planar centimetres, is **noticed** — and
    /// what the animal does about it is `brave_pct`'s business, not this
    /// field's. On prey it reads as a fright radius, which is what it was
    /// named for; on a predator the same number is the range it hunts from,
    /// and the two are one field because they are one comparison. The
    /// content validator gates `attack_range_m <= spook_m` on exactly this
    /// reading: nothing bites what it has not noticed.
    pub spook_cm: i64,
    /// Ticks between a death and the same slot hatching again at the same
    /// home. **The same home**, which is where we and the reference part
    /// company on purpose: their refill re-samples the distribution and the
    /// population migrates over a wipe (`reference/SPAWN.md` §3.5). Ours is
    /// the slot model `TERRAIN.md` §2 already applies to trees.
    pub respawn_ticks: u32,
    /// What the corpse holds: the killing blow stands these rows up as a
    /// ground bag at the death cell (`strike`), and the killer loots it
    /// like any other bag. `NO_ITEM` ends the table.
    pub loot: [ItemStack; MOB_LOOT_ROWS],
}

impl MobDef {
    pub const INERT: Self = Self {
        hp: 0,
        gait: 0,
        flee_gait: 0,
        flee_ticks: 0,
        attack: 0,
        attack_range_cm: 0,
        attack_ticks: 0,
        brave_pct: 0,
        roam_cm: 0,
        spook_cm: 0,
        respawn_ticks: 0,
        loot: [ItemStack {
            item: NO_ITEM,
            count: 0,
        }; MOB_LOOT_ROWS],
    };
}

/// The baked species table (`content::Content::bake_mobs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobContent {
    pub defs: [MobDef; MOB_KINDS],
}

impl MobContent {
    /// Inert: no species has hit points, so the roster never hatches.
    pub const EMPTY: Self = Self {
        defs: [MobDef::INERT; MOB_KINDS],
    };

    /// The fixture the probe and the sim tests run on — the alpha roster's
    /// shape without a content directory. Loot is deliberately `NO_ITEM`
    /// here: item *indices* are a property of the loaded set, and a
    /// fixture that invented them would be asserting against a table it
    /// made up.
    ///
    /// **Both species, because `Mobs::new` makes both.** A fixture that
    /// armed only the pig would leave every wolf slot inert (`hp == 0`
    /// never hatches), so a third of the roster would silently not exist
    /// and every test here would be measuring a world the shipped content
    /// does not produce — the shape of an assertion that passes for the
    /// wrong reason.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.defs[MOB_PIG as usize] = MobDef {
            hp: 80,
            gait: 63,
            flee_gait: 127,
            flee_ticks: 90,
            attack: 15,
            attack_range_cm: 200,
            attack_ticks: 60,
            brave_pct: 50,
            roam_cm: 6_000,
            spook_cm: 1_200,
            respawn_ticks: 9_000,
            loot: [ItemStack {
                item: NO_ITEM,
                count: 0,
            }; MOB_LOOT_ROWS],
        };
        c.defs[MOB_WOLF as usize] = MobDef {
            hp: 100,
            gait: 69,
            flee_gait: 107,
            flee_ticks: 180,
            attack: 20,
            attack_range_cm: 200,
            attack_ticks: 60,
            brave_pct: 0,
            roam_cm: 9_000,
            spook_cm: 3_000,
            respawn_ticks: 9_000,
            loot: [ItemStack {
                item: NO_ITEM,
                count: 0,
            }; MOB_LOOT_ROWS],
        };
        c
    }

    pub fn def(&self, kind: u8) -> MobDef {
        self.defs
            .get(kind as usize)
            .copied()
            .unwrap_or(MobDef::INERT)
    }
}

/// One roster slot. Every field is sim state and every one of them is
/// hashed — including `home_q*`, which never changes, because a shard that
/// somehow disagreed about where an animal lives would disagree about every
/// step it takes afterwards and the hash should say so on the first tick
/// rather than the tenth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mob {
    pub kind: u8,
    /// Standing in the world. False either because it is dead and waiting
    /// on `respawn_at`, or because this slot never found a home.
    pub alive: bool,
    /// A slot with no home is permanently empty — the seed had no land at
    /// any of its `HOME_TRIES` draws. Kept as a flag rather than a
    /// sentinel position so nothing has to know which coordinate means
    /// "nowhere".
    pub homed: bool,
    pub hp: u16,
    pub body: Body,
    /// Heading, on the wire's yaw scale. The animal's whole steering state:
    /// `movement::step` turns it into a direction through the same LUT a
    /// player's view yaw goes through, and the snapshot ships it as the
    /// facing a client draws.
    pub yaw: u16,
    /// `InputFrame::move_z` this animal is currently walking at. Zero is
    /// grazing — standing still is a *state*, not the absence of one.
    pub gait: i8,
    /// Fleeing until this tick (0 = calm).
    pub roused_until: u64,
    /// Awake, as of the last think tick. Recomputed there and not per
    /// tick, so a dormant animal costs one comparison a tick.
    pub awake: bool,
    /// Home, in position quanta — the leash's centre and the hatch point.
    pub home_qx: i32,
    pub home_qz: i32,
    /// Tick this slot may hatch on. Zero on a fresh world, so every homed
    /// slot hatches on tick 1 under armed content.
    pub respawn_at: u64,
}

/// The roster: fixed slots, homes chosen once at world construction.
///
/// Built in `World::new` beside `terrain::haven` and for the same reason —
/// it is a pure function of the seed, it costs a bounded number of terrain
/// probes, and having it before the first tick means nothing in the tick
/// has to ask whether the world is ready yet.
#[derive(Clone)]
pub struct Mobs {
    pub m: [Mob; MAX_MOBS],
}

impl Mobs {
    /// Every slot's home, drawn from the seed and rejected against the
    /// terrain. Allocation-free and bounded: `MAX_MOBS × HOME_TRIES`
    /// terrain probes, at construction, never in a tick.
    pub fn new(seed: u64, haven: &Haven) -> Self {
        let mut mobs = Self {
            m: [Mob::default(); MAX_MOBS],
        };
        for (slot, mob) in mobs.m.iter_mut().enumerate() {
            mob.kind = kind_of(slot);
            let Some((x, z)) = home_of(seed, haven, slot) else {
                continue;
            };
            mob.homed = true;
            mob.home_qx = movement::quant_xz(x);
            mob.home_qz = movement::quant_xz(z);
            // Facing is drawn with the home so a fresh world is not a
            // parade of pigs all pointing at +Z.
            mob.yaw = ((cell_hash(seed, slot as i32, -1, CH_MOB_HOME) & 0xFF) as u16) << 8;
        }
        mobs
    }

    /// How many slots found land. The gate reads this; nothing in the sim
    /// does.
    pub fn homed(&self) -> usize {
        self.m.iter().filter(|m| m.homed).count()
    }

    pub fn alive(&self) -> usize {
        self.m.iter().filter(|m| m.alive).count()
    }
}

/// The entity id of a roster slot. The high bit is the species-vs-player
/// question answered for the client (`limits::MOB_ID_TAG`); the low bits
/// are the slot, which is also how a client looks the animal back up.
#[inline]
pub fn mob_id(slot: usize) -> u32 {
    MOB_ID_TAG | slot as u32
}

/// The roster slot an entity id names, or `None` for a player.
#[inline]
pub fn slot_of_id(id: u32) -> Option<usize> {
    if id & MOB_ID_TAG == 0 {
        return None;
    }
    let slot = (id & !MOB_ID_TAG) as usize;
    (slot < MAX_MOBS).then_some(slot)
}

/// The LUT entry nearest a direction — the inverse `yaw_dir` does not
/// have, and deliberately not an `atan2`.
///
/// Wall 1 forbids trig, but that is not the reason this is a search. The
/// sim's direction space **is** the 256-entry table: `movement::step` will
/// re-quantize any angle to it before moving anything, so picking the entry
/// with the largest dot product is not an approximation of the right answer,
/// it is the right answer. An `atan2` here would be a float approximation
/// whose result gets rounded to this same table anyway.
///
/// Bounded and allocation-free: 256 iterations, on a think tick only, for
/// the ≈ 4 animals thinking on it. A zero-length direction keeps the
/// current heading, which is what a pig standing exactly on its target
/// should do.
fn yaw_toward(dx: f32, dz: f32, current: u16) -> u16 {
    if dx * dx + dz * dz <= 0.0 {
        return current;
    }
    let mut best = current;
    let mut best_dot = f32::NEG_INFINITY;
    for i in 0..256u16 {
        let (ex, ez) = yaw_dir(i << 8);
        let dot = ex * dx + ez * dz;
        if dot > best_dot {
            best_dot = dot;
            best = i << 8;
        }
    }
    best
}

/// The nearest player who could see this animal: planar distance² in
/// centimetre², the delta to them, and their slot. Sleepers do not count —
/// a body nobody is driving has no client watching, so it has no business
/// keeping wildlife awake — but a body on the death screen does, because
/// that player is still looking at the world. **`hp == 0` is deliberately
/// NOT filtered here**: under inert combat content every body reads zero
/// hp, and an animal that went blind on an unarmed shard would freeze the
/// wake/flee behaviour the roster tests pin. The *bite* checks hp at its
/// own site — waking at a corpse is fine, worrying one is not.
fn nearest_player(players: &[Player; MAX_PLAYERS], body: &Body) -> Option<(i64, i32, i32, usize)> {
    let mut best: Option<(i64, i32, i32, usize)> = None;
    for (slot, p) in players.iter().enumerate() {
        if !p.active || p.sleeping {
            continue;
        }
        let dx = p.body.qx - body.qx;
        let dz = p.body.qz - body.qz;
        // 3 cm quanta into centimetres, in i64 — the AOI convention.
        let d2 = (dx as i64 * 3) * (dx as i64 * 3) + (dz as i64 * 3) * (dz as i64 * 3);
        if best.is_none_or(|(bd2, _, _, _)| d2 < bd2) {
            best = Some((d2, dx, dz, slot));
        }
    }
    best
}

/// One landed bite, parked for `world::tick` to apply. The roster loop
/// cannot write a player — it holds the array immutably so the whole
/// roster reads one consistent tick — so a bite is a record here and a
/// mutation there, the same split `BoxStore::spill` uses for the same
/// borrow.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bite {
    pub mob_slot: u8,
    pub victim: u8,
    pub damage: u16,
    pub range_cm: u16,
}

/// The tick's bites, bounded (wall 4). The cap is generous by construction:
/// only a thinking animal can bite, so at most `MAX_MOBS / MOB_THINK_TICKS`
/// (+1 rounding) land per tick. **Overflow drops the bite** — a full
/// buffer is a merciful tick, never a panic and never a queue.
pub struct Bites {
    entries: [Bite; crate::limits::MAX_MOB_BITES_PER_TICK],
    len: usize,
}

impl Default for Bites {
    fn default() -> Self {
        Self::new()
    }
}

impl Bites {
    pub const fn new() -> Self {
        Self {
            entries: [Bite {
                mob_slot: 0,
                victim: 0,
                damage: 0,
                range_cm: 0,
            }; crate::limits::MAX_MOB_BITES_PER_TICK],
            len: 0,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn entries(&self) -> &[Bite] {
        &self.entries[..self.len]
    }

    #[inline]
    fn push(&mut self, b: Bite) {
        if self.len < self.entries.len() {
            self.entries[self.len] = b;
            self.len += 1;
        }
    }
}

/// One tick of the whole roster.
///
/// Order is slot order, which is the fixed order determinism wants, and the
/// pass is over `MAX_MOBS` regardless of how many are alive — a scan whose
/// length depends on liveness is a scan whose cost is a player's business.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    tick: u64,
    mc: &MobContent,
    cols: &ColIndex,
    occ: &mut Occupants,
    mobs: &mut Mobs,
    players: &[Player; MAX_PLAYERS],
    bites: &mut Bites,
) {
    bites.clear();
    for slot in 0..MAX_MOBS {
        let mob = &mut mobs.m[slot];
        if !mob.homed {
            continue;
        }
        let def = mc.def(mob.kind);
        if !mob.alive {
            // Hatching is the one thing a dormant slot still does. It is
            // cheap, it is off a tick comparison, and an animal that only
            // came back when somebody was standing there to watch would be
            // a shard whose population depends on who logged in.
            if def.hp > 0 && tick >= mob.respawn_at {
                hatch(seed, mob, &def);
            }
            continue;
        }

        // The think tick, phase-offset by slot: `MAX_MOBS / MOB_THINK_TICKS`
        // animals decide on any given tick and the rest only integrate.
        if tick % MOB_THINK_TICKS == (slot as u64) % MOB_THINK_TICKS {
            think(seed, tick, slot, &def, mob, players, bites);
        }
        if !mob.awake {
            continue;
        }

        let frame = InputFrame {
            yaw: mob.yaw,
            move_z: mob.gait,
            buttons: if mob.roused_until > tick {
                BTN_SPRINT
            } else {
                0
            },
            ..InputFrame::default()
        };
        movement::step(seed, cols, occ, &mut mob.body, &frame);
    }
}

/// Stand the animal back up at its own home.
fn hatch(seed: u64, mob: &mut Mob, def: &MobDef) {
    // `Body::at` puts the capsule on the heightfield — the same one the
    // home was chosen against at construction, so this lands standing.
    // Re-quantized from the stored quanta rather than kept as floats:
    // the home IS its quantized value, and dequantizing it here is what
    // makes a hatch bit-identical to the construction that placed it.
    let x = mob.home_qx as f32 * POS_XZ_Q;
    let z = mob.home_qz as f32 * POS_XZ_Q;
    mob.body = Body::at(seed, x, z);
    mob.hp = def.hp;
    mob.alive = true;
    mob.gait = 0;
    mob.roused_until = 0;
    mob.awake = false;
}

/// One decision. Everything an animal chooses is chosen here, on the think
/// tick, and the ticks in between only integrate what this left behind.
fn think(
    seed: u64,
    tick: u64,
    slot: usize,
    def: &MobDef,
    mob: &mut Mob,
    players: &[Player; MAX_PLAYERS],
    bites: &mut Bites,
) {
    let near = nearest_player(players, &mob.body);
    mob.awake = near.is_some_and(|(d2, _, _, _)| d2 <= MOB_WAKE_CM * MOB_WAKE_CM);
    if !mob.awake {
        // A dormant animal keeps its heading and stops walking, so it does
        // not wake up mid-stride into a wall it never saw.
        mob.gait = 0;
        return;
    }

    // A player inside the spook radius rouses the animal, and refreshes a
    // rousing already running — which is what makes both halves work: a
    // chase (either direction) lasts `flee_ticks` past the last moment you
    // were close.
    if let Some((d2, _, _, _)) = near {
        if d2 <= def.spook_cm * def.spook_cm {
            mob.roused_until = tick + def.flee_ticks as u64;
        }
    }

    if mob.roused_until > tick {
        // Courage decides which way the same rousing points (their boar's
        // own rule, `reference/ANIMALS.md` §7): whole, it charges the
        // player; hurt below `brave_pct` of max, the identical state is a
        // flight. A species with `attack == 0` never charges.
        let brave =
            def.attack > 0 && (mob.hp as u32) * 100 >= (def.hp as u32) * (def.brave_pct as u32);
        if let Some((d2, dx, dz, victim)) = near {
            if brave {
                // Straight at them, on the same LUT the walk uses.
                mob.yaw = yaw_toward(dx as f32 * POS_XZ_Q, dz as f32 * POS_XZ_Q, mob.yaw);
                let in_reach = d2 <= def.attack_range_cm * def.attack_range_cm;
                // **Closed: stand and bite.** A charge that keeps sprinting
                // at a target it has already reached does not stop there —
                // it runs past, turns on its next think, and runs back, and
                // the result is an animal orbiting the player on a
                // two-think (30-tick) cycle.
                //
                // That is worse than ugly, because the bite is phase-locked
                // to `attack_ticks` (60) and 60 is a multiple of 30: the
                // bite therefore samples the *same point* of the orbit
                // forever. If that point is outside reach, that slot can
                // never bite that player — not rarely, never. Slot 0's
                // phase happened to land inside reach, which is why the pig
                // bit anything at all and why `a_whole_pig_charges_and_bites`
                // was green for three days; the first pig to hatch anywhere
                // else (slot 1, once `kind_of` gave slot 0 to the wolf) had
                // its bite locked out permanently. Two correct periods
                // beating against each other, and the gate could not see it
                // because the gate only ever hunted slot 0.
                mob.gait = if in_reach {
                    0
                } else {
                    def.flee_gait.min(127) as i8
                };
                // The bite: in reach, on this slot's phase (the doc on
                // `attack_ticks` — a phase lock is a cooldown with no
                // state). Recorded, not applied: the borrow is the reason
                // `Bites` exists.
                let period = def.attack_ticks.max(1) as u64;
                if in_reach && players[victim].hp > 0 && tick % period == (slot as u64) % period {
                    bites.push(Bite {
                        mob_slot: slot as u8,
                        victim: victim as u8,
                        damage: def.attack,
                        // isqrt on i64 is not on wall 1's float list, but
                        // f32 sqrt is; the cast is the AOI convention run
                        // backwards and the range is a sentence's worth of
                        // precision, not a sim quantity.
                        range_cm: ((d2 as f32).sqrt()) as u16,
                    });
                }
                return;
            }
            // Directly away, on the same LUT the walk uses.
            mob.yaw = yaw_toward(-(dx as f32) * POS_XZ_Q, -(dz as f32) * POS_XZ_Q, mob.yaw);
        }
        mob.gait = def.flee_gait.min(127) as i8;
        return;
    }

    // The leash. Two ways an animal is out of place and both answer "go
    // home": past the roam radius, or standing on the beach — the second
    // is what keeps a wanderer out of the sea without teaching anything
    // here to swim.
    let hdx = mob.home_qx - mob.body.qx;
    let hdz = mob.home_qz - mob.body.qz;
    let home_d2 = (hdx as i64 * 3) * (hdx as i64 * 3) + (hdz as i64 * 3) * (hdz as i64 * 3);
    let x = mob.body.qx as f32 * POS_XZ_Q;
    let z = mob.body.qz as f32 * POS_XZ_Q;
    let beached = terrain::height(seed, x, z) <= terrain::BEACH_MAX_H;
    if home_d2 > def.roam_cm * def.roam_cm || beached {
        mob.yaw = yaw_toward(hdx as f32 * POS_XZ_Q, hdz as f32 * POS_XZ_Q, mob.yaw);
        mob.gait = def.gait.min(127) as i8;
        return;
    }

    // Otherwise: graze or amble. The draw is per (slot, think index), so it
    // is a pure function of the world's own clock and reproduces under
    // replay without anything being stored between ticks.
    let draw = cell_hash(
        seed,
        slot as i32,
        (tick / MOB_THINK_TICKS) as i32,
        CH_MOB_THINK,
    );
    if draw & 3 == 0 {
        mob.gait = 0;
        return;
    }
    // A turn, not a new bearing. ±32 LUT entries is ±45°, and the
    // difference between this and drawing a fresh heading every half second
    // is the difference between an animal and a glitch.
    let turn = ((draw >> 8) & 0x3F) as i32 - 32;
    let idx = ((mob.yaw >> 8) as i32 + turn) & 0xFF;
    mob.yaw = (idx as u16) << 8;
    mob.gait = def.gait.min(127) as i8;
}

/// Resolve one already-taken swing (cadence paid, no node and no player
/// hit) against the animals in front of the attacker.
///
/// The same pick every other target gets — nearest inside the weapon's
/// reach and gather's aim cone — and it sits between the player scan and
/// the raid scan in `world::tick` for the reason the order implies: a
/// player is always the intended target over an animal, and an animal is
/// always the intended target over the wall behind it.
///
/// **The kill leaves a body, not a payment.** The loot rows stand up as a
/// ground bag at the death position (`Backpacks::stand_up` — the same
/// container a player's death, a barrel and a broken box already use), so
/// the killer walks over and opens it like any bag, and nothing new
/// crosses the wire. The reference's real interaction is a butchering
/// *verb* on the corpse — a tool-gated harvest — and that verb now has a
/// landing place: this bag is where its output would go (`NOW.md` §0m).
///
/// Two exits pay nothing, both the bag store's own policy, restated here
/// because this is a call site wall 4 reads: an **inert ladder**
/// (`base_ticks == 0`, content that never armed backpacks) stands up no
/// bag, so a kill under such a set drops nothing rather than inventing a
/// lifetime; and a **full store** evicts the bag nearest its own despawn
/// (`BAG_GONE_EVICTED`), exactly as a player death does.
///
/// Bounded: one pass over `MAX_MOBS`, on a swing tick only, allocation-free.
#[allow(clippy::too_many_arguments)]
pub fn strike(
    cc: &CombatContent,
    bc: &BackpackContent,
    mc: &MobContent,
    tick: u64,
    attacker: usize,
    players: &[Player; MAX_PLAYERS],
    mobs: &mut Mobs,
    bags: &mut Backpacks,
    events: &mut EventQueue,
) -> bool {
    let a = &players[attacker];
    if !a.active || a.hp == 0 {
        return false;
    }
    let Some(def) = cc.held_melee(held_item(a)) else {
        return false;
    };
    let reach = def.reach_cm as f32 * 0.01;
    let ax = a.body.qx as f32 * POS_XZ_Q;
    let ay = a.body.qy as f32 * POS_Y_Q;
    let az = a.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(a.frame.yaw);
    let attacker_id = a.id;

    let mut best: Option<(f32, usize)> = None;
    for (slot, m) in mobs.m.iter().enumerate() {
        if !m.alive || m.hp == 0 {
            continue;
        }
        let dx = m.body.qx as f32 * POS_XZ_Q - ax;
        let dy = m.body.qy as f32 * POS_Y_Q - ay;
        let dz = m.body.qz as f32 * POS_XZ_Q - az;
        let d2 = dx * dx + dz * dz;
        if d2 > reach * reach || fabs(dy) > DY_MAX_M {
            continue;
        }
        let aimed = d2 <= POINT_BLANK_M2 || dx * fx + dz * fz > CONE_COS * d2.sqrt();
        if aimed && best.is_none_or(|(bd2, _)| d2 < bd2) {
            best = Some((d2, slot));
        }
    }
    let Some((_, slot)) = best else {
        return false;
    };

    let species = mc.def(mobs.m[slot].kind);
    let mob = &mut mobs.m[slot];
    let died = def.damage >= mob.hp;
    mob.hp -= def.damage.min(mob.hp);
    // Hurt is the other way into a flight, and the only one that does not
    // need the attacker to be close: shot from range, the animal still runs.
    mob.roused_until = tick + species.flee_ticks as u64;
    mob.awake = true;
    // EV_HIT is the attacker's own fact and the server routes it by `a`,
    // so a tagged mob id in `b` reaches the hand that swung and nothing
    // else — the hitmarker, exactly as a player hit draws it.
    events.push(EV_HIT, attacker_id, mob_id(slot), def.damage as u32);
    if !died {
        return true;
    }
    mob.alive = false;
    mob.hp = 0;
    mob.respawn_at = tick + species.respawn_ticks as u64;

    // The corpse: the loot rows, packed into a bag standing where the
    // animal died. The killer's inventory is untouched until they press
    // the loot verb on it — which is what makes the take a choice, and
    // what a butchering verb will later gate with a tool. `owner` is the
    // dead animal's own tagged id, the field's meaning ("who died")
    // applied to a body that was never a player; the wire never carries
    // it (world.rs, `EV_BAG_DROPPED`).
    let mut items = [ItemStack::default(); INV_SLOTS];
    let mut n = 0usize;
    for row in species.loot.iter() {
        if row.item == NO_ITEM || row.count == 0 {
            continue;
        }
        items[n] = *row;
        n += 1;
    }
    let (qx, qy, qz) = (mob.body.qx, mob.body.qy + BAG_Y_OFFSET_Q, mob.body.qz);
    bags.stand_up(bc, qx, qy, qz, mob_id(slot), &items, tick, events);
    true
}

/// A home for one roster slot: uniform over the island square, rejected
/// against the same walkability the spawn ring uses, plus the two authored
/// sites.
///
/// Uniform-and-reject rather than the reference game's quadtree importance
/// sampler (`reference/SPAWN.md` §3.1–3.2). Theirs exists because their
/// spawn filter is a baked byte field over a map file that has to be
/// sampled proportionally without scanning; ours can simply *ask the
/// terrain* at any point, and 24 draws against ~50% land is a home for all
/// but a handful of slots. What we do steal is the structure they got
/// right: **sample cheaply and approximately, reject exactly.**
fn home_of(seed: u64, haven: &Haven, slot: usize) -> Option<(f32, f32)> {
    for attempt in 0..HOME_TRIES {
        let h = cell_hash(seed, slot as i32, attempt, CH_MOB_HOME);
        let x = ((h & 0xFFFF) as f32 / 65536.0) * terrain::ISLAND_SIZE;
        let z = (((h >> 16) & 0xFFFF) as f32 / 65536.0) * terrain::ISLAND_SIZE;
        // Inland: above the beach band, so the leash's centre is never
        // somewhere the "go home" rule would immediately fire on.
        if terrain::height(seed, x, z) <= terrain::BEACH_MAX_H {
            continue;
        }
        if terrain::slope(seed, x, z) >= HOME_MAX_SLOPE {
            continue;
        }
        // The two authored sites are the map's meeting places (TERRAIN.md
        // §8). A pig standing in the haven's loot pad on every wipe is a
        // free kill on a schedule, which is the opposite of what a
        // destination is for.
        if terrain::in_haven(haven, x, z) || terrain::in_waystation(haven, x, z) {
            continue;
        }
        return Some((x, z));
    }
    None
}
