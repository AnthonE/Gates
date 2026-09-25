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
//! the notice radius starts it, and `brave_pct` decides which way the
//! animal then points. Which radius is the hour's — `spook_cm` by day and
//! `night_spook_cm` after dusk (`MobDef::spook_at`, the sim's only reader
//! of the world clock). A pig is whole-hearted at full health and a coward under half of
//! it; a wolf's floor is zero, so the same state is a charge at 1 hp. The
//! bite is `attack != 0` and neither species has a code path the other
//! lacks.
//!
//! **The deciding lives in `brain.rs` and the routing in `nav.rs`**, which is
//! the reference game's own split: a brain (`BaseAIBrain`: senses, memory,
//! a state machine wired by a per-species design) that drives a navigator
//! (`BaseNavigator`: a destination, a speed, a path). This file keeps the
//! roster: who lives where, hatching, the hit, the corpse, and the per-tick
//! walk of whatever route the last think left. There was no navmesh here
//! for a while, on the argument that the heightfield is analytic; that is
//! true of terrain and false of a base, and an animal steering straight at
//! a player behind a wall walked into the wall forever. `nav.rs` probes the
//! same predicates this capsule moves by, so it still bakes nothing.
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
use crate::brain::{self, AiState, NO_TARGET};
use crate::collide::ColIndex;
use crate::combat::{held_item, CombatContent};
use crate::gather::{ItemStack, NO_ITEM};
use crate::input::{InputFrame, BTN_SPRINT};
use crate::limits::{INV_SLOTS, MAX_MOBS, MAX_PLAYERS, MOB_ID_TAG, MOB_THINK_TICKS};
use crate::movement::{self, Body, POS_XZ_Q};
use crate::nav::{self, Ground, Nav, NavPath};
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
pub(crate) const WOLF_SLOT_EVERY: usize = 4;

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

/// Guards standing at the haven pad, and at each live waystation
/// (`DECISIONS.md` §open, "site guards v0" and "the guard chain").
///
/// **Two numbers rather than one, because a flat roster is the defect.**
/// Guards v0 gave every authored site the same two, so the pad and a
/// waystation cost a player exactly the same to walk up to — while
/// `ci/haven_prize.mjs` gates a strictly rising *prize* chain across three
/// tiers (road shoulder → waystation → pad, each out-paying the one below on
/// yield, rolls and reach). A rising reward against a flat risk is a gradient
/// of **travel time**, which is what `reference/RIPLIST.md` §0's threat frame
/// names: their yields were priced for contested farming and ours were not,
/// so taking the yield without the interruption that balanced it imports half
/// a number.
///
/// The magnitudes are a proposed default and the operator's word is owed on
/// how far the ladder should lean; what is not a preference is the **shape**,
/// and that is what the const block below and `tests/guard.rs` §A hold: the
/// richest site keeps strictly the most, and the tier below it keeps some.
/// The road shoulder keeps none and is the chain's zero.
pub const HAVEN_GUARDS: usize = 4;
pub const WAYSTATION_GUARDS: usize = 2;

/// Guards an inland site keeps. **Zero, and it follows from the prize.**
///
/// The risk chain is the prize chain's shadow (`ci/haven_prize.mjs`, and the
/// const block below) — that is what the two asserts under [`SITE_GUARDS`]
/// say in the other direction, that the pad may not be cheaper to rob than a
/// waystation and that a waystation may not be free. `terrain::INLAND_CRATES`
/// is 0, so the inland site pays nothing and there is nothing to keep; the
/// assert below is what stops the two drifting apart.
pub const INLAND_GUARDS: usize = 0;

/// Roster slots spent guarding, across every site. A stated number and not
/// a hashed draw, for `WOLF_SLOT_EVERY`'s reason: a gate counts it instead
/// of sampling it. The pad's four plus [`terrain::WAYSTATIONS`] at
/// [`WAYSTATION_GUARDS`] each is **8 of the 16 wolves**, leaving 8 hunting
/// the island — two fewer than guards v0 left, which is the price of the
/// gradient and is paid out of the free roster rather than out of the cap.
///
/// **Summed per tier, not per site**, which is the difference
/// `terrain::INLAND_SITES` turns on: the inland tier is in this sum at
/// [`INLAND_GUARDS`] = 0, so a third authored place landed without moving
/// the eight that hunt.
pub const SITE_GUARDS: usize =
    HAVEN_GUARDS + terrain::WAYSTATIONS * WAYSTATION_GUARDS + terrain::INLAND_SITES * INLAND_GUARDS;

const _: () = assert!(
    SITE_GUARDS * WOLF_SLOT_EVERY <= MAX_MOBS,
    "the guard slots run past the roster — every guard is a wolf slot, so \
     SITE_GUARDS * WOLF_SLOT_EVERY is the last slot one can claim"
);
// The chain's shape, at the definition, in the form `terrain.rs` uses for
// the footprints: get it wrong and the crate does not build, so there is no
// version of the tree where the pad is the cheapest site to rob and a suite
// is merely red.
const _: () = assert!(
    HAVEN_GUARDS > WAYSTATION_GUARDS,
    "the pad keeps no more guards than a waystation — the risk chain does \
     not rise where ci/haven_prize.mjs proves the prize chain does"
);
const _: () = assert!(
    WAYSTATION_GUARDS > 0,
    "a waystation keeps no guards, so the middle tier of the prize chain is \
     free to rob and only the pad costs anything"
);
// The same sentence for the tier that pays nothing, in both directions: a
// site with nothing to steal keeps nobody, and a site that starts paying
// starts keeping someone. Arming `terrain::INLAND_CRATES` without arming
// this is the shape that would ship a free crate on the quietest site on the
// island.
const _: () = assert!(
    (terrain::INLAND_CRATES == 0) == (INLAND_GUARDS == 0),
    "the inland tier's guards and its containers disagree — the risk chain \
     is the prize chain's shadow (ci/haven_prize.mjs)"
);
// `guard_site_of` below maps every non-pad guard ordinal onto the WAYSTATION
// block of `Haven::minor`, so the mapping is complete only while that block
// is the whole of the non-pad roster. Arming `INLAND_GUARDS` fires this
// rather than silently homing an inland guard at `minor[0]`.
const _: () = assert!(
    SITE_GUARDS - HAVEN_GUARDS == terrain::WAYSTATIONS * WAYSTATION_GUARDS,
    "the guard roster reaches past the tier guard_site_of can address"
);

/// Which authored site a roster slot keeps, or `None` for the free roster.
/// **Site 0 is the haven pad; 1..=`WAYSTATIONS` index [`Haven::minor`].**
///
/// That range is the WAYSTATION block of `minor`, not all of it — the inland
/// tier occupies the slots after it and keeps nobody ([`INLAND_GUARDS`]).
/// The const block above is what holds the two in step.
///
/// Guards are drawn from the *predator* slots and are not a species: a
/// guard is a wolf that lives at a destination, so `kind_of` still answers
/// what is chasing you and the wire still carries no species field. That is
/// deliberate and it is the reason this landed without a client change —
/// every species match in the client is a `_ =>` fall-through to the pig
/// (`render/mobs.rs`, `sound/voice.rs`, `ui/death.rs`), so a third *kind*
/// would draw and sound as a pig until five separate arms were written.
///
/// Pure in the slot ordinal for `kind_of`'s reason: worldgen the two sides
/// recompute rather than transmit. What the seed still varies is *where* on
/// its site the guard stands.
#[inline]
pub const fn guard_site_of(slot: usize) -> Option<usize> {
    if !slot.is_multiple_of(WOLF_SLOT_EVERY) {
        return None;
    }
    let g = slot / WOLF_SLOT_EVERY;
    if g >= SITE_GUARDS {
        return None;
    }
    // The pad takes the first block of guard ordinals and the waystations
    // divide the rest evenly. Ordering matters only in that it is *stated*:
    // a gate counts per site rather than sampling, so the pad being first is
    // a fact the test reads rather than a property anything depends on.
    if g < HAVEN_GUARDS {
        return Some(0);
    }
    Some(1 + (g - HAVEN_GUARDS) / WAYSTATION_GUARDS)
}

/// The footprint a site presents — the same [`terrain::SiteFootprint`] the
/// clutter sweep reads, picked by site index rather than by position, so a
/// guard's leash costs no terrain probe.
#[inline]
fn footprint_of(site: usize) -> terrain::SiteFootprint {
    if site == 0 {
        terrain::HAVEN_FOOTPRINT
    } else {
        // Every lesser tier presents the same footprint — the inland site is
        // a waystation's massing standing somewhere else — so this stays one
        // branch. It is `WAYSTATION_FOOTPRINT` that `site_sweep` and the
        // carve read for every entry of `minor` too.
        terrain::WAYSTATION_FOOTPRINT
    }
}

/// A guard's leash in planar centimetres — **the site's scatter radius, not
/// the species' `roam_cm`**. This is the whole of what makes a guard guard.
///
/// A wolf roams 90 m, which is five and a half pad radii: left on the
/// species number a guard would spend most of its life somewhere else and
/// the crates would be unattended exactly when someone walked up to them.
/// Bound to `SiteFootprint::scatter_m` it holds the ground it was placed
/// on, and the ground is the same disc the sweep already clears.
///
/// It does not shorten a chase. The leash is only consulted when the animal
/// is *not* roused (`brain::far_from_home` is only ruled in calm states), so a guard still charges a player it
/// notices at the wolf's full spook radius and still commits for
/// `flee_ticks` past the last moment they were close — it simply comes home
/// afterwards instead of wandering off with the pad behind it.
#[inline]
pub fn guard_leash_cm(slot: usize) -> Option<i64> {
    guard_site_of(slot).map(|s| (footprint_of(s).scatter_m * 100.0) as i64)
}

/// Item rows one species may drop. Structural cap, not a knob: the bake
/// refuses a longer table rather than truncating one.
pub const MOB_LOOT_ROWS: usize = 4;

/// Noise channels. Disjoint from every channel `terrain.rs` and `world.rs`
/// spend, which is the only property that matters about them.
const CH_MOB_HOME: u32 = 112;

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
    ///
    /// Daylight only, since nocturnal senses: [`MobDef::spook_at`] picks
    /// between this and `night_spook_cm` off the world clock.
    pub spook_cm: i64,
    /// The same radius after dusk, planar centimetres — the first number in
    /// this file that the *hour* decides.
    ///
    /// **It is a second radius rather than a multiplier, and it is not
    /// required to be the smaller one**, because the direction is content's
    /// call and not the sim's (wall 7). The shipped roster points it down:
    /// the reference game's 2024 predator rework *reduced* wolf sight at
    /// night, on the grounds that an animal that hunts you in pitch black
    /// "feels a bit like a landmine" — night danger there is a problem of
    /// what the *player* can see, and sharpening the animal on top of it
    /// was the defect they were fixing. No game in the survey publishes a
    /// day→night sense ratio above 1× for one creature; the one published
    /// 2× is a night-only *variant*, which is a different row in this table
    /// and not this field (`DECISIONS.md` §open, "nocturnal senses").
    ///
    /// A rousing already running is untouched by dusk falling across it —
    /// `brain::sense` refreshes `roused_until` while you are inside the radius and
    /// otherwise lets it run out, so a chase that starts in daylight ends
    /// on `flee_ticks` and not on a boundary. That is the desirable shape
    /// and it costs no code: it falls out of refresh-not-recheck.
    pub night_spook_cm: i64,
    /// Ticks between a death and the same slot hatching again at the same
    /// home. **The same home**, which is where we and the reference part
    /// company on purpose: their refill re-samples the distribution and the
    /// population migrates over a wipe (`reference/SPAWN.md` §3.5). Ours is
    /// the slot model `TERRAIN.md` §2 already applies to trees.
    pub respawn_ticks: u32,
    /// The animal's hit volume, a cylinder standing on its feet: radius and
    /// height in **centimetres** (`mobs.toml` `body_r_cm` / `body_h_cm`).
    /// What `melee::mob_cast` tests the swing's ray against — so a pig is
    /// 0.8 m tall to a spear because content says so, and a level swing
    /// from a 1.6 m eye passes over it until the hunter looks down. Zero
    /// height is a species nothing can hit, the inert row's shape.
    pub body_r_cm: u16,
    pub body_h_cm: u16,
    /// The sight cone, as the cosine of its half-angle in permille
    /// (`mobs.toml` `sight_deg`, baked). Only a crouched player needs to be
    /// in it: everyone else is heard across the whole notice radius, and a
    /// sneaker outside it is not noticed at all (`brain::sense`, the
    /// reference's `IgnoreNonVisionSneakers`).
    pub sight_dot_pm: i16,
    /// How far this animal answers a pack-mate already on a target,
    /// centimetres. **Zero is solitary** and non-zero is a pack animal: it
    /// runs `brain::WOLF` and takes turns closing in.
    pub pack_cm: i64,
    /// A target holding a lit torch inside this radius is circled, not
    /// bitten, centimetres. Zero fears nothing.
    pub fire_fear_cm: i64,
    /// What the corpse holds: the killing blow stands these rows up as a
    /// ground bag at the death cell (`strike_slot`), and the killer loots it
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
        night_spook_cm: 0,
        respawn_ticks: 0,
        body_r_cm: 0,
        body_h_cm: 0,
        sight_dot_pm: 0,
        pack_cm: 0,
        fire_fear_cm: 0,
        loot: [ItemStack {
            item: NO_ITEM,
            count: 0,
            cond: 0,
            skin: 0,
        }; MOB_LOOT_ROWS],
    };

    /// The notice radius in force on `tick`, planar centimetres.
    ///
    /// The whole of the clock's reach into the sim. Everything downstream —
    /// which way the animal points, whether it bites, how long it commits —
    /// is `brave_pct`'s and `flee_ticks`' business exactly as before, so the
    /// hour changes *when an encounter starts* and nothing about how it goes.
    /// That is deliberately the smallest surface that makes the day
    /// different from the night, and it keeps one comparison
    /// (`world::is_night`) as the only new determinism input.
    #[inline]
    pub fn spook_at(&self, tick: u64) -> i64 {
        if crate::world::is_night(tick) {
            self.night_spook_cm
        } else {
            self.spook_cm
        }
    }
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
            // Prey's flinch is not clock-keyed: the reference changed its
            // predator's senses and said nothing about its boar's, and
            // `reference/BALANCE.md` §6.2 refuses a difference with no
            // mechanism behind it. Equal here is a statement, not a stub —
            // `the_clock_moves_the_hunter_and_not_the_prey` reads it.
            night_spook_cm: 1_200,
            respawn_ticks: 9_000,
            // The client draws a pig 0.78 m high and 1.5 m long
            // (`render/mobs.rs` PIG_H_M / PIG_LEN_M); a cylinder of this
            // radius covers the body's width and most of its length, and
            // the height is the drawn one. The shipped rows in
            // `content/mobs.toml` say the same, which
            // `client/tests/mob_volume.rs` holds — it caught them 2 cm
            // apart the first time it ran.
            body_r_cm: 55,
            body_h_cm: 78,
            // 240° of sight, the shipped row's: a 120° blind spot behind.
            sight_dot_pm: -500,
            pack_cm: 0,
            fire_fear_cm: 0,
            loot: [ItemStack {
                item: NO_ITEM,
                count: 0,
                cond: 0,
                skin: 0,
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
            night_spook_cm: 1_500,
            respawn_ticks: 9_000,
            body_r_cm: 60,
            body_h_cm: 85,
            sight_dot_pm: -500,
            pack_cm: 4_000,
            fire_fear_cm: 800,
            loot: [ItemStack {
                item: NO_ITEM,
                count: 0,
                cond: 0,
                skin: 0,
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

/// One roster slot. Every field a tick can move is hashed
/// (`world.rs::state_hash`) — **`home_q*` and `homed` are not**, and this
/// doc claimed the opposite until 2026-08-14. They are a pure function of
/// the seed, recomputed identically by every build, so they are worldgen and
/// `haven` one field over is excluded for the same reason. The consequence
/// worth knowing: a disagreement about where an animal *lives* is not caught
/// on the first tick by the hash — it is caught on the first tick the animal
/// moves, because every position downstream of it is hashed.
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
    /// The brain's memory of its target runs until this tick (0 = never
    /// roused): refreshed while the target is noticed, left to run out when
    /// it is not, so a chase or a flight outlasts the last sighting by
    /// `flee_ticks` (`brain::sense`).
    pub roused_until: u64,
    /// What the brain is doing, and how that state's last think went
    /// (`brain::RUNNING` / `FINISHED` / `FAILED`).
    pub state: AiState,
    pub status: u8,
    /// The state's timer: `brain::Cond::Timer` holds from this tick on.
    pub state_until: u64,
    /// The remembered target, a player slot, or `brain::NO_TARGET`.
    pub target: u8,
    /// The senses notice nobody but an attacker until this tick — how an
    /// animal that gave up on a target it could not reach stays given up.
    pub calm_until: u64,
    /// Failed routes to the current target, in a row.
    pub tries: u8,
    /// Stuck detection: where the body stood last think, and how many
    /// thinks running it has walked without getting anywhere.
    pub last_qx: i32,
    pub last_qz: i32,
    pub stuck: u8,
    /// The heading the brain wants while no route is being walked (facing
    /// a bite, circling, bolting with nowhere planned).
    pub want_yaw: u16,
    /// Which point of its rounds a guard walks to next.
    pub leg: u8,
    /// The brain's position memory (the reference's position slots): the
    /// last noise it heard, remembered until `poi_until`. Prey runs from
    /// it, a hunter goes to see.
    pub poi_qx: i32,
    pub poi_qz: i32,
    pub poi_until: u64,
    /// The tick it was last struck — the out-of-combat clock its healing
    /// runs off (`brain::heal`).
    pub hurt_at: u64,
    /// The tick it last howled for its pack (`brain::HOWL_COOLDOWN_TICKS`).
    pub howled_at: u64,
    /// Struck before it had noticed anyone. A pack animal answers that by
    /// backing off, calling its pack and coming back with it (the
    /// reference's reworked wolf); cleared once it closes in again.
    pub ambushed: bool,
    /// The route being walked (`nav.rs`).
    pub path: NavPath,
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
        // Slot order, so a pack's leader has its home before any member
        // draws a den around it.
        for slot in 0..MAX_MOBS {
            let leader = pack_leader_of(slot).filter(|&l| l != slot && mobs.m[l].homed);
            let home = match leader {
                Some(l) => {
                    den_of(seed, haven, slot, &mobs.m[l]).or_else(|| home_of(seed, haven, slot))
                }
                None => home_of(seed, haven, slot),
            };
            let mob = &mut mobs.m[slot];
            mob.kind = kind_of(slot);
            mob.target = NO_TARGET;
            let Some((x, z)) = home else {
                continue;
            };
            mob.homed = true;
            mob.home_qx = movement::quant_xz(x);
            mob.home_qz = movement::quant_xz(z);
            // Facing is drawn with the home so a fresh world is not a
            // parade of pigs all pointing at +Z.
            mob.yaw = ((cell_hash(seed, slot as i32, -1, CH_MOB_HOME) & 0xFF) as u16) << 8;
            mob.want_yaw = mob.yaw;
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
    pub(crate) fn push(&mut self, b: Bite) {
        if self.len < self.entries.len() {
            self.entries[self.len] = b;
            self.len += 1;
        }
    }
}

/// Turn rate while walking a route: eight LUT entries (11.25°) a tick,
/// ~340°/s. The reference's newest animals turn through an arc rather than
/// on the spot (its `LimitedTurnNavAgent`); this is the same idea at the
/// capsule's scale, and it is what makes a route read as a walk.
pub const MOB_TURN_STEP: u16 = 8 << 8;

/// A heading this far off the one wanted is turned on the spot rather than
/// walked round, so a sharp corner is a pivot and not a loop around it.
const TURN_IN_PLACE: u16 = 64 << 8;

/// The tick's pack calls, bounded (wall 4): the roster slots that howled.
/// `world::tick` turns each into an `EV_HOWL` after the roster has stepped,
/// `Bites`' split for `Bites`' reason. **Overflow drops the howl's sound**,
/// never the call — the pack answers off the roster.
pub struct Howls {
    slots: [u8; crate::limits::MAX_HOWLS_PER_TICK],
    len: usize,
}

impl Default for Howls {
    fn default() -> Self {
        Self::new()
    }
}

impl Howls {
    pub const fn new() -> Self {
        Self {
            slots: [0; crate::limits::MAX_HOWLS_PER_TICK],
            len: 0,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn entries(&self) -> &[u8] {
        &self.slots[..self.len]
    }

    #[inline]
    pub(crate) fn push(&mut self, slot: u8) {
        if self.len < self.slots.len() {
            self.slots[self.len] = slot;
            self.len += 1;
        }
    }
}

/// One tick of the whole roster.
///
/// Order is slot order, which is the fixed order determinism wants, and the
/// pass is over `MAX_MOBS` regardless of how many are alive — a scan whose
/// length depends on liveness is a scan whose cost is a player's business.
/// Every slot thinks against one `brain::peers` snapshot, taken before the
/// first body moves and refreshed slot by slot as each one decides.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &crate::terrain::Haven,
    tick: u64,
    day_tick: u64,
    sense_pm: u32,
    mc: &MobContent,
    cols: &ColIndex,
    occ: &mut Occupants,
    mobs: &mut Mobs,
    players: &[Player; MAX_PLAYERS],
    lit: &[bool; MAX_PLAYERS],
    noises: &crate::noise::Noises,
    nav: &mut Nav,
    bites: &mut Bites,
    howls: &mut Howls,
) {
    bites.clear();
    howls.clear();
    nav.begin_tick();
    let mut peers = brain::peers(&mobs.m, tick);
    let mut ground = Ground {
        seed,
        haven,
        cols,
        occ,
    };
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
                hatch(seed, haven, mob, &def);
            }
            continue;
        }

        // The think tick, phase-offset by slot: `MAX_MOBS / MOB_THINK_TICKS`
        // animals decide on any given tick and the rest only walk.
        if tick % MOB_THINK_TICKS == (slot as u64) % MOB_THINK_TICKS {
            let mut ctx = brain::Ctx {
                tick,
                day_tick,
                sense_pm,
                players,
                lit,
                noises,
                howls: &mut *howls,
                peers: &peers,
                nav,
                ground: &mut ground,
            };
            brain::think(&mut ctx, slot, &def, mob, bites);
            peers[slot] = brain::Peer::of(mob, tick);
        }
        if !mob.awake {
            continue;
        }
        let frame = drive(mob, &def);
        movement::step(seed, haven, cols, ground.occ, &mut mob.body, &frame);
    }
}

/// The body's input for this tick: the route's next corner (or the brain's
/// heading when there is no route), turned toward at `MOB_TURN_STEP`, at
/// the think's gait. Arriving stops the body where it arrived; what it does
/// next is the next think's business. A running state sprints unless the
/// animal is limping (`brain::limping`).
fn drive(mob: &mut Mob, def: &MobDef) -> InputFrame {
    let want = if mob.path.active() {
        match nav::waypoint(&mut mob.path, mob.body.qx, mob.body.qz) {
            Some((tx, tz)) => nav::yaw_toward(
                (tx - mob.body.qx) as f32,
                (tz - mob.body.qz) as f32,
                mob.yaw,
            ),
            None => {
                mob.gait = 0;
                mob.want_yaw
            }
        }
    } else {
        mob.want_yaw
    };
    mob.yaw = nav::turn_toward(mob.yaw, want, MOB_TURN_STEP);
    let move_z = if nav::yaw_gap(mob.yaw, want) > TURN_IN_PLACE {
        0
    } else {
        mob.gait
    };
    InputFrame {
        yaw: mob.yaw,
        move_z,
        buttons: if matches!(mob.state, AiState::Chase | AiState::Flee) && !brain::limping(def, mob)
        {
            BTN_SPRINT
        } else {
            0
        },
        ..InputFrame::default()
    }
}

/// Stand the animal back up at its own home, with an empty head.
fn hatch(seed: u64, haven: &crate::terrain::Haven, mob: &mut Mob, def: &MobDef) {
    // `Body::at` puts the capsule on the heightfield — the same one the
    // home was chosen against at construction, so this lands standing.
    // Re-quantized from the stored quanta rather than kept as floats:
    // the home IS its quantized value, and dequantizing it here is what
    // makes a hatch bit-identical to the construction that placed it.
    let x = mob.home_qx as f32 * POS_XZ_Q;
    let z = mob.home_qz as f32 * POS_XZ_Q;
    mob.body = Body::at(seed, haven, x, z);
    mob.hp = def.hp;
    mob.alive = true;
    mob.gait = 0;
    mob.roused_until = 0;
    mob.awake = false;
    mob.state = AiState::Idle;
    mob.status = brain::RUNNING;
    mob.state_until = 0;
    mob.target = NO_TARGET;
    mob.calm_until = 0;
    mob.tries = 0;
    mob.last_qx = mob.body.qx;
    mob.last_qz = mob.body.qz;
    mob.stuck = 0;
    mob.want_yaw = mob.yaw;
    mob.poi_until = 0;
    mob.hurt_at = 0;
    mob.howled_at = 0;
    mob.ambushed = false;
    mob.path.clear();
}

/// Land one already-taken swing on the animal in `slot` — the one
/// `melee::cast` found the ray entering first.
///
/// The pick is no longer here (it was the planar cone every other target
/// got, and `melee.rs`'s header says why it went); what is left is the
/// consequence, which is the same as it was.
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
/// Returns true when the animal took the hit.
#[allow(clippy::too_many_arguments)]
pub fn strike_slot(
    cc: &CombatContent,
    bc: &BackpackContent,
    mc: &MobContent,
    tick: u64,
    attacker: usize,
    players: &[Player; MAX_PLAYERS],
    mobs: &mut Mobs,
    bags: &mut Backpacks,
    events: &mut EventQueue,
    slot: usize,
) -> bool {
    let a = &players[attacker];
    if !a.active || a.hp == 0 || slot >= mobs.m.len() {
        return false;
    }
    let Some(def) = cc.held_melee(held_item(a)) else {
        return false;
    };
    let attacker_id = a.id;
    if !mobs.m[slot].alive || mobs.m[slot].hp == 0 {
        return false;
    }

    let species = mc.def(mobs.m[slot].kind);
    let mob = &mut mobs.m[slot];
    let died = def.damage >= mob.hp;
    mob.hp -= def.damage.min(mob.hp);
    // Hurt is the other way into a flight, and the only one that does not
    // need the attacker to be close: shot from range, the animal still runs.
    // The attacker becomes the target whoever the animal was minding — the
    // reference's `Attacked` event — and a hit ends any sulk it was in.
    mob.ambushed = mob.roused_until <= tick;
    mob.roused_until = tick + species.flee_ticks as u64;
    mob.awake = true;
    if mob.target != attacker as u8 {
        mob.tries = 0;
    }
    mob.target = attacker as u8;
    mob.calm_until = 0;
    mob.hurt_at = tick;
    // EV_HIT is the attacker's own fact and the server routes it by `a`,
    // so a tagged mob id in `b` reaches the hand that swung and nothing
    // else — the hitmarker, exactly as a player hit draws it.
    events.push(
        EV_HIT,
        attacker_id,
        mob_id(slot),
        crate::world::hit_c(crate::collide::Part::Chest, def.damage),
    );
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

/// Free wolves den together, this many to a pack: the reference's wolves
/// come in packs, and a pack is only a pack if its members live within
/// call of each other (`MobDef::pack_cm`). 8 free wolves are packs of
/// 3, 3 and 2.
pub const PACK_SIZE: usize = 3;

/// A pack-mate's den is drawn this far around its leader's home.
const DEN_MIN_M: f32 = 4.0;
const DEN_SPAN_M: f32 = 10.0;
const CH_MOB_DEN: u32 = 116;

/// The first slot of the pack a free predator slot belongs to, or `None`
/// for prey and for guards (whose pack is their post). Pure in the slot,
/// like `kind_of`.
pub const fn pack_leader_of(slot: usize) -> Option<usize> {
    if !slot.is_multiple_of(WOLF_SLOT_EVERY) {
        return None;
    }
    let g = slot / WOLF_SLOT_EVERY;
    if g < SITE_GUARDS {
        return None;
    }
    let free = g - SITE_GUARDS;
    Some((SITE_GUARDS + free - free % PACK_SIZE) * WOLF_SLOT_EVERY)
}

/// Which pack a slot answers, or `None` for the solitary: a guard's pack is
/// its post and a free predator's is its den's leader. Pure in the slot.
/// Only a pack-mate's call is answered — the reference's howl carries to
/// the wolf's own pack and nobody else's.
pub const fn pack_of(slot: usize) -> Option<u8> {
    if let Some(site) = guard_site_of(slot) {
        return Some(200 + site as u8);
    }
    match pack_leader_of(slot) {
        Some(l) => Some(l as u8),
        None => None,
    }
}

/// A den for a pack-mate: an annulus around the leader's home, rejected
/// against exactly what `home_of` rejects. `None` sends the slot back to
/// an ordinary home — a lone wolf is still a wolf.
fn den_of(seed: u64, haven: &Haven, slot: usize, leader: &Mob) -> Option<(f32, f32)> {
    let (lx, lz) = (
        leader.home_qx as f32 * POS_XZ_Q,
        leader.home_qz as f32 * POS_XZ_Q,
    );
    for attempt in 0..HOME_TRIES {
        let h = cell_hash(seed, slot as i32, attempt, CH_MOB_DEN);
        let r = DEN_MIN_M + ((h & 0xFFFF) as f32 / 65536.0) * DEN_SPAN_M;
        let (fx, fz) = yaw_dir((((h >> 16) & 0xFF) as u16) << 8);
        let (x, z) = (lx + fx * r, lz + fz * r);
        if terrain::height(seed, x, z) <= terrain::BEACH_MAX_H
            || terrain::slope(seed, x, z) >= HOME_MAX_SLOPE
            || terrain::in_haven(haven, x, z)
            || terrain::in_waystation(haven, x, z)
        {
            continue;
        }
        return Some((x, z));
    }
    None
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
    // A guard's home is the one place this function otherwise refuses.
    if let Some(site) = guard_site_of(slot) {
        return guard_home_of(seed, haven, slot, site);
    }
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

/// Where a site guard stands: the **swept apron**, the annulus between its
/// site's `swept_m` and `scatter_m`.
///
/// Both radii come from the site's own [`terrain::SiteFootprint`] and each
/// is doing a job. The outer one is the leash, so a guard placed outside it
/// would walk home on its first think. The inner one is the floor the
/// clutter sweep already clears for the site's structures — the crate ring
/// and the shelter — so drawing outside it is what keeps a guard from
/// hatching inside a container it would then be shoved out of every tick.
/// The apron is also simply where a guard belongs: between the road and the
/// prize, with the crates at its back.
///
/// Uniform by *area*, which is why the draw is on r² and not on r; the
/// straightforward version crowds every guard onto the inner edge. The
/// bearing is the yaw LUT's, so no trigonometry enters wall 1's float list.
///
/// A waystation the solver never filled has no apron and no guard: the slot
/// stays unhomed, which is the same honest empty a slot that found no land
/// gets, and `step` skips it for the same reason.
fn guard_home_of(seed: u64, haven: &Haven, slot: usize, site: usize) -> Option<(f32, f32)> {
    let (sx, sz) = if site == 0 {
        (haven.x, haven.z)
    } else {
        let w = &haven.minor[site - 1];
        if !w.live {
            return None;
        }
        (w.x, w.z)
    };
    let fp = footprint_of(site);
    let lo2 = fp.swept_m * fp.swept_m;
    let hi2 = fp.scatter_m * fp.scatter_m;
    for attempt in 0..HOME_TRIES {
        let h = cell_hash(seed, slot as i32, attempt, CH_MOB_HOME);
        let t = (h & 0xFFFF) as f32 / 65536.0;
        let r = (lo2 + t * (hi2 - lo2)).sqrt();
        let (fx, fz) = yaw_dir((((h >> 16) & 0xFF) as u16) << 8);
        let x = sx + fx * r;
        let z = sz + fz * r;
        // **The land line, not the beach band** — the one place a guard's
        // walkability differs from a wanderer's, and it is not a loosened
        // check, it is a different question.
        //
        // `BEACH_MAX_H` (sea + 2 m) is a *margin*: `home_of` keeps the free
        // roster well clear of the water so a leash centre is never
        // somewhere the go-home rule fires on, and so nothing has to learn
        // to swim. But the haven is scored for low relief near the coast
        // road, and on real seeds the pad lands inside that margin — seed
        // 1's pad centre is at **1.28 m**, under the 2.0 band, with the
        // crates and the shelter standing on it regardless. Rejecting the
        // apron for that refuses to guard exactly the destinations the
        // solver likes best; measured, seed 1 had **no** admissible bearing
        // at any radius and seed 42 had roughly half.
        //
        // `LAND_MIN_H` (sea + 0.6 m) is the question actually being asked —
        // terrain.rs calls it "the land line: ground below this is water's
        // edge, not somewhere a thing stands or a road runs", and scatter
        // and the coast road already share it. A guard stands where the
        // road runs.
        if terrain::height(seed, x, z) <= terrain::LAND_MIN_H {
            continue;
        }
        if terrain::slope(seed, x, z) >= HOME_MAX_SLOPE {
            continue;
        }
        return Some((x, z));
    }
    None
}
