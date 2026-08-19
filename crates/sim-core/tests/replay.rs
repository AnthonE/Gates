//! `test_replay` (DESIGN.md §7/§12): same build + same seed + same command
//! log → the same state hashes, every stamp. Until the server's WAL exists
//! this drives a deterministic in-memory command script — the contract is
//! identical, the fixture just isn't a file yet. The final hash is also
//! pinned: any accidental drift in sim behavior reddens this gate.

use sim_core::backpack::BackpackContent;
use sim_core::bots::bot_frame;
use sim_core::build::BuildContent;
use sim_core::combat::{CombatContent, ThrowDef};
use sim_core::craft::CraftContent;
use sim_core::gather::GatherContent;
use sim_core::limits::{HOTBAR_SLOTS, STATE_HASH_INTERVAL, TICK_HZ};
use sim_core::loot::LootContent;
use sim_core::rng::Pcg32;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x0047_4154_4553; // "GATES"
const TICKS: u64 = 900;

/// Pinned end-state hash for (SEED, the script below). Regenerates only
/// with an intentional sim change, in the same commit (CLAUDE.md wall 5).
///
/// Regenerated this commit, and **structurally rather than
/// behaviourally**: the deployed box became a container, so the box
/// store's length entered the digest. This script places no box, so the
/// number below moved by exactly eight zero bytes — one `u64` count and
/// not one record. That distinction is the whole value of the note: this
/// regeneration is *not* evidence that a box move runs here, and
/// `crates/sim-core/tests/box_container.rs` is what owns that behaviour.
/// The move verb itself is unchanged on this surface — no bot sends one.
///
/// The regeneration before it was **behavioural: the barrel loop runs on
/// this surface.** Bot 31 is held on four scanned barrel cells in turn and
/// swings them apart, so the number below is a function of the weighted
/// pick, the count draw, the stack rule that files the roll into a
/// container, and the container's own address and timer. Change a weight,
/// the Lemire multiply-shift, the roll-count band or `hits`, and this
/// reddens.
///
/// It first went green *without* covering any of that, which is the more
/// useful half of this note: arming `LootContent` moved the hash on barrel
/// **hits** alone — `slot_lives` is hashed, so counting a hit is visible
/// even when nothing ever breaks — and the run made zero containers. A
/// bot walks out of a barrel's 2 m reach long before the second of two
/// swings 38 ticks apart, so a teleport-once fixture reproduces hits
/// forever and a smash never. The `made > 0` assert below exists so that
/// cannot recur silently: a moved hash is not evidence, and the count is.
///
/// The regeneration before this one was **structural rather than
/// behavioural** — the distinction matters, because only one of the two is
/// evidence that a new rule runs here. The death screen put five fields on every
/// `Player` (`dead` and the four facts the wire encodes off it) and all
/// five are in the state digest, so the number below moved by ten bytes
/// per active body. It moved for no other reason: **nothing on this
/// surface can die**, so every one of those ten bytes is the value a live
/// body carries — `false`, `0`, `0`, `NO_ITEM`, `0`. The script installs
/// no `CombatContent`, so every body here has `hp_max == 0` (the drink
/// comment below says the same thing about the salt death), and a body
/// that cannot die never reaches `World::die`.
/// `test_alloc_zero` and `crates/sim-core/tests/bag_respawn.rs` own the
/// behaviour; what this gate owns is that the screen is *in the hash*,
/// which is what a WAL resumed over a corpse depends on.
///
/// The regeneration before it was structural for the same reason:
/// respawn-on-bag put the deploy store's bag cooldowns into the digest,
/// and this script stands a great many deployables, so it moved by eight
/// zero bytes per record.
///
/// The regeneration before that: **the drink verb runs on this surface.**
/// Bot 21 is stood on a scanned shoreline and presses drink every seven
/// ticks from the moment it joins, so the number below is a function of
/// the meter write, the full refusal, the dry refusal, and the five
/// `terrain::height` compares that decide which of the three a press is.
/// Move `DRINK_REACH_M`, change the tap pattern, or let a full meter pay
/// hp, and this reddens.
///
/// The regeneration before it: **the survival clock started running on
/// this surface** — 64 bodies draining every tick and two of them eating,
/// which is what made the hash a function of the drain arithmetic rather
/// than of its absence. Change a numerator, a denominator or the
/// drain→hurt→heal order and this reddens too.
///
/// The two before those were structural rather than behavioural: the
/// survival module's fields entering `state_hash` while the script left
/// them all zero, and before that the death backpack's two zero-length
/// store fields.
/// Regenerated for a MERGE, which is a case the notes above never cover and
/// the one that most deserves suspicion. Two lanes each moved this hash on
/// their own trunk — `main` to `0x66B6_0CC0_1555_D451` (the container wire and
/// the box store), `lane/looks` to `0x2869_95C7_F9D2_BFA1` (scatter clumping,
/// the road and the haven pad) — and the merged sim is neither. Taking either
/// side would have been the dangerous resolution: green would then mean one
/// lane's behaviour had been dropped, and this gate exists to notice exactly
/// that. The number below is read off a run of the merged tree and confirmed
/// stable across repeated runs, not carried over from a side.
/// Regenerated again for structural collapse (build.rs `collapse_from`).
/// Two causes, and both were checked rather than assumed: `sweep_support`
/// joins `state_hash` beside the other two sweep cursors, and — with that
/// field held out of the digest and the sweep disabled — the final hash
/// still moved, so the cascade genuinely changes the world this script
/// replays. Pieces that used to hang in the air after the thing under them
/// broke now come down, which is the point. Determinism itself is
/// unaffected and asserted above: both runs agree tick for tick.
/// Regenerated again for occupant solidity (occupy.rs): trees, boulders,
/// nodes, barrels, crates and the haven shelter now stop a body, so the same
/// input script walks a different path than it did through a hollow island.
/// The evidence that this is the intended cause and not a broken script is
/// that **every behavioural floor in `run()` still passes** — the bots place,
/// deploy, decay, door, eat, drink and craft exactly as before, and the two
/// runs still agree tick for tick. Only where they stood when they did it
/// moved. The memo behind the query (`World::slot_cache`) is deliberately not
/// in `state_hash`; it is a pure function's cache, so it cannot move this
/// number, and holding it out is what keeps that true.
///
/// Regenerated again from `0xB09C_FF07_A63B_4581` for the composition blend
/// (`terrain::scatter_row`, TERRAIN.md §1 stage 9): the scatter pass stopped
/// picking one biome weight row and started blending four by the ground's own
/// splat weights, so in the ~10% of land cells that sit in a transition band
/// the script meets a different occupant than it did — a bush where a tree
/// stood, and the solid-occupant path above then walks the bots somewhere
/// else. The same evidence as the entry above says this is the intended cause
/// and not a broken script: **every behavioural floor in `run()` still
/// passes** — place, deploy, decay, door, eat, drink, craft — and the two runs
/// still agree tick for tick, which is asserted before this line is reached.
/// Heights did not move, so nothing about the terrain the bots stand on
/// changed; only what is scattered on it, and only at biome edges.
/// Regenerated again for the jump verb (`movement.rs` `JUMP_SPEED`,
/// `input.rs` `BTN_JUMP`): `bot_frame` now sets the jump bit once every
/// `JUMP_PERIOD` frames, so the same input script walks an arc where it used
/// to walk flat, and `state_hash` folds `qvy` and `grounded`.
///
/// The cause was checked rather than assumed, on the two axes that separate
/// an intended move from a broken script:
///
/// - **Determinism is untouched and is asserted above** — both runs still
///   agree tick for tick, on every stamped hash and on the final one. The
///   jump is a pure function of the frame, so it could not have been
///   otherwise, but "could not" is not evidence and the assertion is.
/// - **Every behavioural floor in `run()` still passes.** The bots place,
///   deploy, decay, door, eat, drink, craft and smash barrels exactly as
///   before — same floors, same script. That is what says the bots gained a
///   verb rather than lost their footing: a period that left them airborne
///   most of the time would have starved the build and gather floors first,
///   and those floors are the reason `JUMP_PERIOD` is 128 and not 8.
///
/// The RNG stream is deliberately *not* part of this move. The jump bit is
/// keyed off `seq`, not off a fourth `rng` draw, precisely so that this
/// number moved for one reason and not for two — a stream shift would have
/// re-rolled every bot's entire life and made the two causes indistinguishable
/// from inside this file.
/// Regenerated again for the raid verb (wire v23, `charge.rs`), and this
/// time the number moved for **three** stated reasons rather than one —
/// which is worth writing down, because the paragraph above earned its
/// single-cause claim by keeping the rng stream out of the move, and this
/// commit cannot make the same claim:
///
/// 1. `Command::Throw` joined the scripted arm at ticks 165/166, so two
///    charges are planted and two blasts land four ticks later.
/// 2. `World::state_hash` gained the charge store. A burning fuse is state
///    — two shards disagreeing about one disagree about whether a base is
///    standing ten seconds later — so it is hashed, and it is hashed even
///    on the ticks the store is empty (a length prefix of zero).
/// 3. A `CombatContent` is installed on this surface at all for the first
///    time. Only `throw[3]` is armed and `player_hp` stays 0, so no body
///    gained hp and nothing became able to hurt anything — but the table
///    is construction input and the world is no longer built from
///    `EMPTY`.
///
/// **Determinism itself did not move, and that is the assertion that
/// matters here.** `hashes_a == hashes_b` and `final_a == final_b` both
/// still hold — the two runs agree tick for tick — so what changed is what
/// the sim *does*, not whether it does it reproducibly. A regenerated
/// golden beside a red equality assert would be the bug this constant
/// exists to catch; a regenerated golden beside a green one is the verb
/// landing.
///
/// Regenerated once more by the MERGE of the two entries above, which is
/// the case neither of them covers. `main` moved this hash for the biome
/// composition blend and `lane/systems` moved it for the jump verb; the
/// merged sim carries both, so it is neither number. Taking a side would
/// have gone green while silently dropping one lane's behaviour from the
/// replayed surface — the exact drift this constant exists to catch. The
/// value below is read off a run of the merged tree.
/// Regenerated once more by **sleepers**, and this is the shape of turn the
/// first entry above describes rather than the second: `state_hash` gained
/// state, and nothing this script does exercises it. The script never
/// leaves, so every body in it is awake for every tick — `sleeping` folds a
/// zero byte, `slept_at` eight, and `World::evictions` eight more, on every
/// hash. The number moves because the *definition* of the state moved, not
/// because any body behaved differently.
///
/// That distinction is checkable and was checked: `hashes_a == hashes_b`
/// and `final_a == final_b` are asserted before this constant is, and both
/// were green on the run this value was read off. A regenerated golden
/// beside a red equality assert would be the bug the constant exists to
/// catch; beside a green one it is the state definition widening, which is
/// exactly what a slice that adds a field to `Player` is supposed to do.
/// Regenerated once more by **lock v1**, and this is the *first* shape —
/// the verb landing, not the definition widening. The lock store is
/// hashed on the arrow idiom (no length prefix), so an empty one folds
/// nothing and this number would not have moved on its own; it moved
/// because the script now bolts a code lock onto the door at tick 155,
/// misses its code at 156 (a **shock**, so a door verb writes a body's
/// hp) and arms it at 158. Three real state changes on the replayed
/// surface, one of them to a player record.
///
/// `hashes_a == hashes_b` and `final_a == final_b` were green on the run
/// this value was read off, which is the check that separates the two
/// shapes: a regenerated golden beside a red equality assert is the drift
/// this constant exists to catch, beside a green one it is the verb.
/// Regenerated once more by the **building-rights** slice, and it is
/// both shapes at once — which is why they are separated here rather
/// than asserted together. **The definition widened**: `state_hash` gained
/// the crew list on every hearth and a placement tick on every piece and
/// deployable, and the script's pieces all carry non-zero ticks, so the
/// number would have moved with no verb changed. **And the verbs
/// landed**: the script now runs the crew ops through `Command::Access`
/// and both halves of `Command::Demolish`, one of which is refused by the
/// grace window on purpose.
///
/// Regenerated once more, in the same commit, by **upkeep/decay v1** —
/// and this one is purely the first shape, a verb changing. No state was
/// added: upkeep now charges **per material** rather than demanding one
/// hearth cover a whole piece, and an unpaid piece rots at its own
/// material's rate rather than a flat 5 %. The script leaps thirty
/// upkeep periods at its midpoint, so both changes bite hard on it.
///
/// Regenerated once more by the **merge** of the building-rights branch
/// with the oven/mob work that landed on `main` in parallel. Purely the
/// first shape again and none of it a behaviour change of mine: the
/// probe fixture grew a sixth deployable row, the oven's burn state is
/// hashed, animals are on the world, and the script's kit grew the
/// fixture's two new items. Two branches that each moved this number
/// cannot both be right about it, so it is read fresh off the merged
/// tree.
///
/// Regenerated again at **research v0**, and this time for two causes
/// that both belong in the number. The craft probe fixture's row 2 is
/// blueprint-gated now, so a script that used to enqueue it is refused —
/// a real behaviour change, deliberately chosen so the gate covers the
/// refusal. And `Player::known` joined `state_hash`, which it had to:
/// a `Command::Research` mutates it and what it changes is which craft
/// requests the sim honours from then on, so a mask outside the hash
/// would let two replays of one WAL diverge on the first gated craft
/// with every other field still matching — `[backpack]`'s defect one
/// layer over.
///
/// `hashes_a == hashes_b` and `final_a == final_b` were green on the run
/// this value was read off, which is the check that keeps a regenerated
/// golden evidence rather than a shrug.
///
/// **Regenerated 2026-08-10 for `SPAWN_CLEAR_M` 4.0 → 4.5.** The tree pool
/// gained a second, wider species, so the sim's spawn clearance rose to keep
/// a fresh spawn standing clear of the widest canopy — and a larger clearance
/// picks different beach cells, which moves every spawn position and therefore
/// the whole run. **The drift is in the inputs, not the rules**: the diff that
/// caused it is one constant in `world.rs` and the rest of that file's change
/// is comments, `test_terrain_golden` did NOT move (worldgen is untouched, only
/// which cleared cell a player is placed on), and both determinism assertions
/// above were green on the run this value came off. That is the evidence;
/// without those three the honest move would have been to find the bug instead.
/// Operator, 2026-08-10: *"we have no worlds to wipe so thats fine"* — the
/// regeneration is cheap because there is nothing live to invalidate.
///
/// **Regenerated 2026-08-11 for deploy collision v0**, and it is purely
/// the first shape — a verb changing, no state widened. `movement::step`
/// now consults the solid-deploy nibbles and stands bodies on occupant
/// and deploy tops (`slot_ground` / `piece_ground`'s solid arm), so every
/// walk in this script that passes near the placed box, the door's cell
/// or a scattered rock can resolve to a different quantized position from
/// that tick on. Nothing joined `state_hash` — the collision index is
/// derived state and stays out of it — and `test_terrain_golden` did NOT
/// move (worldgen untouched). `hashes_a == hashes_b` and
/// `final_a == final_b` were green on the run this value was read off,
/// which is what separates a landed verb from a drift.
///
/// **Regenerated 2026-08-14 for world containers v0**, and this one is the
/// *other* shape — state widened, no verb changed. `World::world_conts`
/// joined `state_hash`, and this script never opens a container, so what
/// entered the digest is a length of **zero** and sixty-four records that
/// do not exist. The run is byte-identical in every other respect: no
/// command in the script is new, no existing rule moved, and
/// `test_terrain_golden` did NOT move (worldgen untouched). That is the
/// cheapest possible cause for a moved golden and also the easiest to
/// wave through, so the evidence is the same as every entry above —
/// `hashes_a == hashes_b` and `final_a == final_b` were green on the run
/// this value was read off, which is why the equality asserts sit *before*
/// the pin rather than after it.
///
/// **Regenerated 2026-08-15 for item durability v0**, and it is both
/// shapes at once, stated so nobody has to reconstruct it: state widened
/// (`ItemStack` grew `cond`, so all four container loops in `state_hash`
/// fold two more bytes per stack) AND the fixture's inputs moved (the
/// gather probe fixture arms `cond_max` on items 0/1 and a wear row per
/// node, and every minted stack now arrives at its item's ceiling, so the
/// wear arithmetic runs inside this script's own farming). No command is
/// new and `test_terrain_golden` did NOT move (worldgen untouched).
/// `hashes_a == hashes_b` and `final_a == final_b` were green on the run
/// this value was read off — the panic that produced it was at the pin
/// line alone, with both determinism asserts above it already passed.
///
/// **Regenerated 2026-08-16 for the build base lattice** (`DECISIONS.md`
/// §open "build base lattice v0"): `build::column_floor_y` snaps every
/// column's base to the 0.5 m lattice, so every piece, solid-deploy
/// volume and box-drop height this script produces moved by up to a
/// quarter-quantum — positions the bodies then walk on, which is the
/// whole surface. The same change made a solid top take the step rule
/// (`collide::deploy_blocked`), so the bots' paths over the barrel beach
/// shifted too. No verb changed; `test_terrain_golden` did NOT move
/// (worldgen itself is untouched — the lattice reads terrain, it does
/// not write it). Evidence as above: both determinism equalities were
/// green on the run this value was read off, run twice.
///
/// **Regenerated 2026-08-16 a third time, and this one is neither shape —
/// it is the MERGE of the two above.** Durability v0 and the build base
/// lattice landed on separate branches, each regenerating this pin for
/// its own cause, and each was right about its own tree. Neither hash
/// describes the tree that has both, so the merge could not pick a side:
/// `0x3151…` (durability) and `0x8A52…` (lattice) are both stale here.
/// The value below was read off the merged tree, and the evidence is the
/// same as every entry above and load-bearing precisely because a merge
/// is where a golden is easiest to wave through — both determinism
/// equalities green, `test_terrain_golden` unmoved, and the failure that
/// produced it at the pin line alone.
/// **Regenerated 2026-08-17 for `OCCUPANT_R_M`'s `Tree` row, 0.26 → 0.2398**,
/// and it is purely the first shape: a rule changed, no state was added.
/// The row was the bottom radius of the *browser* client's hand-authored
/// `CylinderGeometry(0.13, 0.26)` — a mesh deleted with that client — while
/// the native client draws a generated trunk measuring 0.2398 m at the base.
/// Nothing re-measured it when the mesh was replaced, so the sim blocked a
/// cylinder up to 0.11 m proud of the visible bark at chest height. It is the
/// measurement now, held there by
/// `client/tests/tree.rs::the_blocked_cylinder_is_the_trunk_the_client_draws`.
///
/// The script's bots walk a beach thick with scatter, so a 2 cm narrower
/// trunk changes where bodies stop against trees and the whole run follows.
/// Evidence, the same as every entry above: both determinism equalities were
/// green on the run this value was read off — the failure was at the pin line
/// alone — and `test_terrain_golden` did NOT move, which is the check that
/// matters here specifically, because a radius is read by collision and never
/// by worldgen. A tree stands in the same place; a body stops 2 cm nearer it.
///
/// **Regenerated 2026-08-18 for the barrel's measured proportions**
/// (`DECISIONS.md` §open "barrel proportions v1"): `OCCUPANT_R_M` and
/// `OCCUPANT_TOP_M` for `BarrelSlot` went from the deleted browser
/// client's guess (0.45 / 0.975) to the measured 55-gallon drum
/// (0.2925 / 0.88). That is a collision change and this script's bots
/// walk a beach the road's barrels stand on, so every body that
/// squeezed past one takes a different path from that tick onward —
/// the first shape, a verb changing, with no state added and no field
/// widened. `test_terrain_golden` did NOT move: worldgen places slots,
/// it does not read their radii. Evidence as every entry above:
/// `hashes_a == hashes_b` and `final_a == final_b` were both green on
/// the run this value was read off, and the failure that produced it
/// was at the pin line alone.
///
/// ⚠ **Regenerated a third time, at the merge of the two above, and the value
/// is NEITHER of theirs.** Both entries are collision changes and both landed
/// — the tree's radius on one side, the barrel's proportions on the other —
/// so the script's bots walk past both and the run diverges from either
/// branch's pin. This is the wire-version collision `protocol/src/lib.rs`
/// records (v38–v40) in its determinism form: two branches each correctly
/// claimed the next number, and the merge has to take one neither claimed.
/// Read off a run of the merged tree; both determinism equalities were green
/// on it and `test_terrain_golden` did not move, which is the check that
/// matters, because a radius is read by collision and never by worldgen.
///
/// **Regenerated 2026-08-19 for `Player::worn` (armor v0), and this one is
/// a shape none of the entries above are: no verb changed and no body took
/// a different path — a FIELD entered the digest.** `state_hash` grew a
/// sibling loop over the two worn stacks after the inventory's, so the
/// hash moved the instant the field existed, before anything could put
/// anything in it. Every body in this script is naked and stays naked;
/// what changed is that the digest now says the sim carries worn
/// equipment, where before it said nothing at all.
///
/// That makes the usual evidence *necessary and not sufficient*, so both
/// halves were checked. The usual half: `hashes_a == hashes_b` and
/// `final_a == final_b` were green on the run this value was read off, the
/// failure was at the pin line alone, and `test_terrain_golden` did not
/// move (worn equipment is not worldgen). The half this shape needs: the
/// twelve bytes are the only cause, which is checkable by arithmetic
/// rather than by trust — `worn` is per-player and always present, so
/// unlike every store loop in `state_hash` there is no length prefix to
/// fold zeroes into, and `world.rs:3620` already records what that
/// distinction cost once (a store hashed unconditionally moved this pin
/// and eight zero bytes were the whole of it). **Measured rather than
/// argued**: deleting the `worn` loop from `state_hash` and running this
/// suite returns the digest to `0xDFFD_AE59_3232_47C6` bit for bit, so the
/// twelve bytes are not merely *a* cause, they are the only one.
///
/// `0xDFFD_AE59_3232_47C6` is the value before, and it is left written
/// here on purpose: the next reader's question is *which change moved it*,
/// and a pin that only ever shows its current value cannot answer that.
const GOLDEN_FINAL_HASH: u64 = 0xE6C1_8463_97AE_FB21;

/// A standable point with sea inside `DRINK_REACH_M`, scanned off the
/// heightfield rather than typed in — the same reason `walk_up_the_beach`
/// walks instead of teleporting to a remembered coordinate: a number that
/// held at one seed and one set of generator constants is a fixture that
/// silently stops meaning what it says.
fn shoreline(seed: u64) -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < sim_core::terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < sim_core::terrain::ISLAND_SIZE {
            let h = sim_core::terrain::height(seed, x, z);
            if (sim_core::terrain::SEA_LEVEL..sim_core::terrain::BEACH_MAX_H).contains(&h)
                && (sim_core::terrain::height(seed, x + r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x - r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z + r) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z - r) < sim_core::terrain::SEA_LEVEL)
            {
                return (x, z);
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island has no coast — the generator changed under this gate");
}

/// The first `n` barrel cells on the island, scanned off `terrain::scatter`
/// rather than typed in — same reason `shoreline` scans: a cell that held
/// a barrel at one seed and one weight table is a fixture that silently
/// stops meaning what it says.
///
/// Returns the barrel's own world position, because the smasher is stood
/// exactly on it: `POINT_BLANK_M2` bypasses the aim cone, so the swing
/// lands without the script also having to reproduce a yaw.
fn barrel_cells(seed: u64, scatter: &sim_core::terrain::ScatterTable, n: usize) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    // The same haven the sim resolves at init: it vetoes scatter, so a script
    // built without it could aim a swing at a barrel the world never placed.
    let haven = sim_core::terrain::haven(seed);
    let span = (sim_core::terrain::ISLAND_SIZE / sim_core::terrain::CELL_SIZE) as i32;
    let mut cz = 0;
    while cz < span && out.len() < n {
        let mut cx = 0;
        while cx < span && out.len() < n {
            let s = sim_core::terrain::scatter(seed, scatter, &haven, cx, cz);
            if s.occupant == sim_core::terrain::Occupant::BarrelSlot {
                out.push((s.x, s.z));
            }
            cx += 1;
        }
        cz += 1;
    }
    assert!(
        !out.is_empty(),
        "this island has no barrels — the scatter table changed under this gate"
    );
    out
}

/// Stand a kitted bot on ground that will hold a foundation.
///
/// The beach spawn ring puts a fresh player at ~1.2 m and a foundation
/// wants 1.5 m (`build::FOUNDATION_MIN_H_M`), so before this the whole
/// scripted build/deploy/door/lock/upgrade arc below depended on where a
/// five-second random walk happened to stop — which held at this seed and
/// collapsed to one placement at `SEED ^ 1`. Stepping inland along the ray
/// to island center until the cell center is buildable is what a player
/// does at a fresh spawn, done deterministically: same seed, same result,
/// and the same `foundation_terrain_ok` the sim refuses on, so the fixture
/// cannot drift away from the rule.
///
/// A fixture arrangement, like the inventory grants beside it — applied
/// identically on both runs, so the replay contract is untouched.
fn walk_up_the_beach(world: &mut World, seed: u64, slot: usize) {
    let b = world.players[slot].body;
    let mut x = b.qx as f32 * sim_core::movement::POS_XZ_Q;
    let mut z = b.qz as f32 * sim_core::movement::POS_XZ_Q;
    let c = sim_core::terrain::ISLAND_SIZE * 0.5;
    let (mut dx, mut dz) = (c - x, c - z);
    let d = (dx * dx + dz * dz).sqrt();
    if d <= 0.0 {
        return;
    }
    dx /= d;
    dz /= d;
    // Bounded: 200 steps of 3 m is 600 m, more than the beach-to-center
    // distance, so the loop is a search and never a walk to nowhere.
    for _ in 0..200 {
        let cx = sim_core::build::build_cell_of(x);
        let cz = sim_core::build::build_cell_of(z);
        let ax = (cx as f32 + 0.5) * sim_core::build::BUILD_CELL_M;
        let az = (cz as f32 + 0.5) * sim_core::build::BUILD_CELL_M;
        if sim_core::build::foundation_terrain_ok(seed, hv(seed), ax, az) {
            break;
        }
        x += dx * sim_core::build::BUILD_CELL_M;
        z += dz * sim_core::build::BUILD_CELL_M;
    }
    world.players[slot].body = sim_core::movement::Body::at(seed, hv(seed), x, z);
}

fn run(seed: u64) -> (Vec<u64>, u64) {
    let mut world = World::new(seed);
    world.gather = GatherContent::probe_fixture();
    world.craft = CraftContent::probe_fixture();
    world.build = BuildContent::probe_fixture();
    world.deploy = sim_core::deploy::DeployContent::probe_fixture();
    // Barrels, on the replayed surface. Both halves are needed or the
    // gate watches nothing: the loot table decides a barrel is breakable
    // at all, and the despawn ladder is what lets the container it breaks
    // into actually stand up. Armed together, a smashed barrel writes to
    // both hashed stores — `slot_lives` (the slot's bit and its jittered
    // respawn) and `backpacks` (the container, its address, its timer and
    // its rolled contents) — so the weighted walk, the count draw and the
    // stacking rule are all functions of the number below.
    // The raid tool, and *only* the raid tool. This surface has never
    // installed a `CombatContent` at all — `player_hp` stays 0, so no body
    // gains hp, nothing can be hurt and nobody dies, which is what keeps
    // this script's bots standing where it left them. Arming the melee
    // rows to reach the charge verb would have put a brawl and a
    // hatchet-rate raid on the replayed surface as a side effect of adding
    // a throwable, and moved this file's golden for three reasons instead
    // of one.
    //
    // `structure` is 10 against the fixture's 100 hp pieces on purpose: two
    // charges land, two blasts announce, and nothing is removed — so the
    // removal floors below still count decay and only decay. A charge that
    // takes a piece down is covered where it belongs, in
    // `tests/event_roles.rs`, against a wall the test owns.
    let mut combat = CombatContent::EMPTY;
    combat.throw[3] = ThrowDef {
        damage: 0,
        structure: 10,
        fuse_ticks: 4,
        reach_cm: (sim_core::build::BUILD_REACH_M * 100.0) as u16,
        blast_cm: 1,
    };
    world.combat = combat;
    world.loot = LootContent::probe_fixture();
    world.backpack = BackpackContent::probe_fixture();
    let barrels = barrel_cells(seed, &world.scatter, 4);
    // The clock, on the replayed surface: meters granted at join, drained
    // every tick by the rational accumulator, and eaten back up by the
    // scripted consumes below — so a change to `survival::step`'s
    // arithmetic moves the pinned hash instead of passing unnoticed.
    //
    // The spans are widened off the fixture's seconds, and the reason is
    // this script rather than the ring: at 10 s / 8 s every body on the
    // island empties inside the first 240 ticks and starts dying of it
    // around tick 440, and a body that dies wakes up somewhere else —
    // which would quietly move the builder, the hearth's feeder and the
    // door's owner off the arc this script exists to pin. Wider than the
    // 900 ticks it runs, nobody empties, nobody starves, and every body
    // the script addresses is still standing where the script left it.
    // The meter floor at the end is what holds that claim.
    let mut survival = SurvivalContent::probe_fixture();
    survival.food_span_ticks = 90 * TICK_HZ;
    survival.water_span_ticks = 72 * TICK_HZ;
    world.survival = survival;
    let mut rng = Pcg32::new(seed, 11);
    let mut yaws = [0u16; 64];
    let mut joined: u32 = 0;
    let mut hashes = Vec::new();
    let (mut placed, mut deployed, mut decayed, mut doors) = (0u32, 0u32, 0u32, 0u32);
    let (mut locked_seen, mut unlocked_seen, mut upgraded_seen) = (false, false, false);
    let mut charges_planted = 0u32;
    let mut hotbar_saved = [sim_core::gather::ItemStack::default(); HOTBAR_SLOTS];
    let (mut eaten, mut eat_refused) = (0u32, 0u32);
    let (mut drank, mut drink_refused) = (0u32, 0u32);
    let mut hearth_cell = (0u16, 0u16);

    for t in 0..TICKS {
        let mut cmds: Vec<Command> = Vec::new();
        if t % 9 == 0 && joined < 64 {
            joined += 1;
            cmds.push(Command::Join { id: joined });
        }
        if t == 450 {
            cmds.push(Command::Leave { id: 3 });
        }
        if t == 500 {
            cmds.push(Command::Join { id: 3 }); // slot reuse is part of the contract
        }
        for id in 1..=joined {
            if (450..500).contains(&t) && id == 3 {
                continue;
            }
            let f = bot_frame(&mut rng, yaws[id as usize - 1], t as u16);
            yaws[id as usize - 1] = f.yaw;
            cmds.push(Command::Input { id, frame: f });
            // The craft verb rides the same log: periodic enqueues (row 3
            // is out of range — the refusal path) and rarer cancels.
            if (t + id as u64).is_multiple_of(37) {
                cmds.push(Command::Craft {
                    id,
                    recipe: ((t / 37 + id as u64) % 4) as u16,
                    count: 1 + (id as u64 % 3) as u16,
                });
            }
            if (t + id as u64).is_multiple_of(149) {
                cmds.push(Command::CraftCancel {
                    id,
                    index: (t % 4) as u16,
                });
            }
            // The build verb rides the log too: places at the player's own
            // cell (successes once wood accrues) plus out-of-range rows and
            // mismatched locs (the refusal paths).
            if (t + id as u64).is_multiple_of(53) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                // Row 5 is out of range (the fixture is 5 rows since the
                // stone rung joined it) — the refusal path stays in
                // surface.
                cmds.push(Command::Place {
                    id,
                    row: ((t / 53 + id as u64) % 6) as u16,
                    cx: cell(b.qx),
                    cz: cell(b.qz),
                    level: ((t / 106) % 2) as u8,
                    loc: ((t / 53 + id as u64) % 4) as u8,
                });
            }
            // The deploy verb too: a bag and a workbench at the player's
            // feet (the success shapes, once the granted or gathered
            // items are there), a rotating junk request for the refusal
            // reasons, and a feed (mostly the no-hearth refusal).
            if (t + id as u64).is_multiple_of(71) {
                let b = &world.players[(id as usize - 1) % 64].body;
                let cell = |q: i32| {
                    sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                        .clamp(0, 1023) as u16
                };
                let (cx, cz) = (cell(b.qx), cell(b.qz));
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: 1,
                    cx: (cx + 1).min(1023),
                    cz,
                    level: 0,
                    loc: 0,
                });
                cmds.push(Command::PlaceDeploy {
                    id,
                    row: ((t / 71 + id as u64) % 5) as u16,
                    cx,
                    cz,
                    level: ((t / 142) % 2) as u8,
                    loc: ((t / 71 + id as u64) % 4) as u8,
                });
                cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                });
            }
            // The eat verb rides the log too, on two of the eight bots the
            // kit at t=149 reaches and neither of the two the hearth arc
            // addresses: id 2 eats slot 20, which that kit fills with
            // fixture item 0 — the one item the survival fixture makes food
            // — and id 4 reaches for slot 21, which it fills with item 1,
            // which is not. The landed consume and the announced refusal,
            // both on the replayed surface; before t=149 both slots are
            // empty, so the early ticks ride the refusal too.
            if id == 2 && (t + id as u64).is_multiple_of(41) {
                cmds.push(Command::Consume { id, slot: 20 });
            }
            if id == 4 && (t + id as u64).is_multiple_of(43) {
                cmds.push(Command::Consume { id, slot: 21 });
            }
            // The drink verb rides it too, on bot 21 — which joins at
            // t=180, carries no kit, founds nothing and is addressed by no
            // other scripted arm. It is stood on a scanned shoreline at
            // t=200 (below); before that the presses land on wherever the
            // ring put it, which is the refusal path. So the landed drink,
            // the dry refusal and the full refusal are all on the replayed
            // surface, and a change to the verb's arithmetic — the meter
            // write, or the five `terrain::height` compares that decide
            // whether there is water — moves the pinned hash.
            //
            // **The salt death is deliberately NOT on this surface**, and
            // that is a fact about the fixture rather than about the verb:
            // this script installs no `CombatContent`, so every body here
            // has `hp_max == 0`, and a cost clamped to a body's own hp is
            // zero. `test_alloc_zero` and `tests/survival.rs` own the kill
            // site; arming combat here to reach it would put 64 bots in a
            // brawl through the middle of the build arc this script exists
            // to pin. Every 7 ticks and
            // not every 29 on purpose: the widened water span drops one
            // unit every ~22 ticks, so a 7-tick cadence finds the meter
            // still full on most presses — which is how the *full* refusal
            // gets onto the replayed surface at all, rather than only in
            // the unit tests.
            if id == 21 && (t + id as u64).is_multiple_of(7) {
                cmds.push(Command::Drink { id });
            }
        }
        // A scripted hearth: grant a kit to the first eight bots (a
        // fixture arrangement, like the wire tests' server-side grants —
        // identical on both runs, so the replay contract holds), then
        // bot 1 founds, hearths, and feeds one remembered cell. This
        // pins the pay path — everything unpaid decays by the leaps.
        // The raid verb's hand, and it has to be the whole hotbar: `bot_frame`
        // draws `sel` from the rng every tick, so a charge in one slot would
        // be held on about a sixth of the ticks and this gate's floor would
        // be a coin flip rather than a floor. A fixture arrangement like the
        // kit grant below — identical on both runs, so the replay contract
        // holds — and restored two ticks later so bot 0 goes back to holding
        // the tools the gather floors need it holding.
        if t == 165 {
            hotbar_saved.copy_from_slice(&world.players[0].inv[..HOTBAR_SLOTS]);
            for slot in 0..HOTBAR_SLOTS {
                world.players[0].inv[slot] = sim_core::gather::ItemStack {
                    item: 3,
                    count: 8,
                    cond: 0,
                };
            }
        }
        if t == 167 {
            world.players[0].inv[..HOTBAR_SLOTS].copy_from_slice(&hotbar_saved);
        }
        if t == 149 {
            for w in 0..8usize {
                if world.players[w].active {
                    for (k, &(item, count)) in [
                        (0u16, 200u16),
                        (1, 200),
                        (2, 50),
                        (3, 50),
                        (4, 50),
                        // Item 6 is the probe fixture's fire (oven v0) and
                        // item 7 its code lock (lock v1) — the hands the
                        // oven and lock arcs below need.
                        (6, 50),
                        (7, 50),
                    ]
                    .iter()
                    .enumerate()
                    {
                        world.players[w].inv[20 + k] = sim_core::gather::ItemStack {
                            item,
                            count,
                            cond: 0,
                        };
                    }
                    walk_up_the_beach(&mut world, seed, w);
                }
            }
        }
        // The drinker, stood on a shoreline scanned off the heightfield —
        // a fixture arrangement like the kit grant above, identical on both
        // runs, so the replay contract holds. Scanned rather than typed in
        // because a hard-coded coast goes stale the first time the
        // generator's constants move, and a drinker staged inland would
        // turn the landed-drink floor below into a coin flip.
        if t == 200 && world.players[20].active {
            let (x, z) = shoreline(seed);
            world.players[20].body = sim_core::movement::Body::at(seed, hv(seed), x, z);
        }
        // The smasher. Held on a scanned barrel rather than teleported once,
        // because a barrel wants two landed swings 38 ticks apart and a bot
        // walks out of its 2 m reach long before the second: teleport-once
        // reproduced barrel *hits* and never a single smash, which is how
        // this surface first went green while covering nothing. Rotating it
        // through four barrels every 200 ticks means four different cells
        // roll four different tables, so the weighted walk is in the hash
        // and not just the slot bit.
        if world.players[30].active {
            let (x, z) = barrels[(t as usize / 200) % barrels.len()];
            world.players[30].body = sim_core::movement::Body::at(seed, hv(seed), x, z);
        }
        if t == 150 {
            let b = &world.players[0].body;
            let cell = |q: i32| {
                sim_core::build::build_cell_of(q as f32 * sim_core::movement::POS_XZ_Q)
                    .clamp(0, 1023) as u16
            };
            hearth_cell = (cell(b.qx), cell(b.qz));
        }
        if (150..=166).contains(&t) {
            let (cx, cz) = hearth_cell;
            let id = world.players[0].id;
            match t {
                150 => cmds.push(Command::Place {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                151 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 0,
                    cx,
                    cz,
                    level: 0,
                    loc: 0,
                }),
                // A doorway on the same cell's low-x edge, a door in it,
                // then the door verbs' whole arc — placement seals the
                // edge locked, its owner's toggles open and reseal it,
                // and the lock verb rides both ways (a stranger's lock
                // attempt in between, refused) — so the bodies that walk
                // that edge afterwards feel each state. All of it before
                // the feeds, which hand the same wood to the hearth.
                152 => cmds.push(Command::Place {
                    id,
                    row: 3,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                153 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 2,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                154 | 157 => cmds.push(Command::Use {
                    id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                // Then the upgrade verb's own arc on a second wall: a
                // wood wall at the low-z edge, climbed to the fixture's
                // stone rung, then asked back down (the tier refusal),
                // then asked for by a second bot — which bounces on
                // **reach**, not on the hearth's claim: the two have
                // wandered ~90 m apart by now and the reach test comes
                // first. The claim refusal is `build::tests::
                // upgrade_answers_to_a_foreign_claim`'s to assert; what
                // rides here is the re-row and two of the bounces. All
                // of it before the feeds, which hand the builder's whole
                // stock to the hearth.
                159 => cmds.push(Command::Place {
                    id,
                    row: 1,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_ZLO,
                }),
                160 | 161 => cmds.push(Command::Upgrade {
                    id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_ZLO,
                    material: if t == 160 {
                        sim_core::build::MAT_STONE
                    } else {
                        sim_core::build::MAT_WOOD
                    },
                }),
                162 => cmds.push(Command::Upgrade {
                    id: world.players[1].id,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_ZLO,
                    material: sim_core::build::MAT_METAL,
                }),
                // Lock v1's whole arc on the replayed surface: bolt the
                // code lock on, arm it with a code, miss the code once
                // (the shock — a *player hp* write from a door verb, and
                // therefore state a replay must reproduce exactly), then
                // unlock it again. 156 is a hand the lock does not
                // remember, so the refusal path is replayed too.
                155 => cmds.push(Command::PlaceDeploy {
                    id,
                    row: 5,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                156 | 158 => cmds.push(Command::Access {
                    id: if t == 156 { world.players[1].id } else { id },
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                    op: if t == 156 {
                        sim_core::deploy::ACCESS_OP_ENTER
                    } else {
                        sim_core::deploy::ACCESS_OP_SET_CODE
                    },
                    code: if t == 156 { 4321 } else { 1234 },
                }),
                // Then the repair verb, on the one address that names two
                // things: the doorway placed at 152 and the door hung in
                // it at 153 share `LOC_EDGE_XLO` exactly, so 163 and 164 are
                // the same four coordinates differing only in the bit that
                // picks the store. The upkeep leaps above have been
                // draining both by then, so these land as real payments
                // rather than as `REFUSE_B_INTACT` — and a repair mutates
                // a structure store *and* `Player::inv`, which is what
                // puts the verb inside this gate instead of beside it.
                // Demolish v1, both stores and both sides of the window:
                // 167 lands (the doorway at 152 is still inside its ten
                // minutes at this tick) and 168 is refused, because the
                // foundation this arc stands on went up hundreds of ticks
                // earlier. A verb whose refusal never replays is a verb
                // half-covered.
                167 | 168 => cmds.push(Command::Demolish {
                    id,
                    deploy: false,
                    cx,
                    cz,
                    level: 0,
                    loc: if t == 167 {
                        sim_core::build::LOC_EDGE_ZLO
                    } else {
                        sim_core::build::LOC_PLANE
                    },
                }),
                163 | 164 => cmds.push(Command::Repair {
                    id,
                    deploy: t == 164,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                // The raid verb, on the addresses the repair arm above just
                // mended. It belongs in *this* gate specifically because a
                // charge is the one thing in the sim whose effect outlives
                // the tick that asked for it: the plant lands here, the
                // damage lands `fuse_ticks` later, and a replay that
                // reproduced the command stream but not the fuse deadline
                // would diverge silently at a tick no command names.
                165 | 166 => cmds.push(Command::Throw {
                    id,
                    deploy: t == 166,
                    cx,
                    cz,
                    level: 0,
                    loc: sim_core::build::LOC_EDGE_XLO,
                }),
                _ => cmds.push(Command::Feed {
                    id,
                    cx,
                    cz,
                    level: 0,
                }),
            }
        }
        // Leap the clock three upkeep periods on a cadence: over the run
        // that is ~40 periods, enough to decay unpaid fixture pieces
        // (100 hp − 5/period) all the way to removal — charge, decay,
        // and the removal cascade all inside the replayed surface.
        if t % 64 == 63 {
            world.tick += 3 * sim_core::deploy::UPKEEP_PERIOD_TICKS;
        }
        world.tick(&cmds);
        // The scripted upgrade's own verdict, read from the store the
        // tick after it ran: an event says a piece was announced, only
        // the record says which rung it stands on now.
        if t == 160 {
            upgraded_seen = world
                .pieces
                .find(
                    hearth_cell.0,
                    hearth_cell.1,
                    0,
                    sim_core::build::LOC_EDGE_ZLO,
                )
                .is_some_and(|p| p.row == 4);
        }
        for e in world.events.entries() {
            match e.code {
                sim_core::world::EV_CHARGE_PLACED => charges_planted += 1,
                sim_core::world::EV_PIECE_PLACED => placed += 1,
                sim_core::world::EV_DEPLOY_PLACED => deployed += 1,
                sim_core::world::EV_PIECE_REMOVED | sim_core::world::EV_DEPLOY_REMOVED => {
                    decayed += 1
                }
                sim_core::world::EV_CONSUMED => eaten += 1,
                // One refusal code, two verbs — so the counters partition
                // by the body that pressed. Ids 2 and 4 are the only ones
                // the eat script addresses, 21 the only drinker. Counting
                // the union would sink the eat floor below: bot 21 presses
                // every 7 ticks from t=180 and most of those find a full
                // meter, so its refusals alone clear `eat_refused >= 8`
                // and the assert stops being able to fail on the verb its
                // own message names.
                sim_core::world::EV_CONSUME_REFUSED => match e.a {
                    2 | 4 => eat_refused += 1,
                    21 => drink_refused += 1,
                    _ => {}
                },
                sim_core::world::EV_DRANK => drank += 1,
                sim_core::world::EV_DOOR => {
                    doors += 1;
                    if e.b & 2 == 0 {
                        unlocked_seen = true;
                    } else {
                        locked_seen = true;
                    }
                }
                _ => {}
            }
        }
        if world.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            hashes.push(world.last_hash);
        }
    }
    // Counted from events, not the final stores: decay legitimately
    // removes early placements before the run ends.
    // Floors, not `> 0`: a script that lands one placement exercises the
    // build path about as well as one that lands none — and that is
    // exactly what the beach ring did to `SEED ^ 1` before
    // `walk_up_the_beach` staged the kitted bots, while `> 0` stayed
    // green. Measured this commit at placed 8 / deployed 50 / decayed 28
    // at `SEED` and 5 / 34 / 18 at `SEED ^ 1`; the floors sit under both,
    // where thinning stops being incidental and becomes a lost gate.
    assert!(
        placed >= 4,
        "the script placed {placed} pieces — the build success path is falling out \
         of the replay surface"
    );
    assert!(
        deployed >= 16,
        "the script deployed {deployed} — the deploy success path is falling out \
         of the replay surface"
    );
    assert!(
        decayed >= 8,
        "only {decayed} decayed away — the removal path is falling out of the \
         replay surface"
    );
    assert!(
        doors >= 4,
        "the scripted door never toggled — the use verb fell out of the replay surface"
    );
    assert!(
        upgraded_seen,
        "the scripted wall never reached its stone rung — the upgrade verb fell \
         out of the replay surface"
    );
    assert!(
        locked_seen && unlocked_seen,
        "the scripted door never changed hands both ways — the lock verb fell out \
         of the replay surface"
    );
    // The raid verb's own floor, and it is the one arm on this surface that
    // needs one most: a `Command::Throw` whose every instance refused would
    // still be *in* the command stream, still reach `charge::place`, and
    // still hash identically on both runs — a verb covered by the letter of
    // the gate and by none of its meaning. This counts announcements, which
    // only a landed plant makes.
    assert!(
        charges_planted > 0,
        "not one charge was planted in the whole window — the raid verb is in \
         the command stream and refusing every time, which hashes the same as \
         a verb that works"
    );
    // The clock's own three claims on this surface, each read off the thing
    // it moves. Content installed and never exercised would hash the same
    // as content that ran, which is the whole failure this block forecloses.
    let witness = &world.players[9]; // joins at t=81, eats nothing, is addressed by no scripted arm
                                     // Strictly inside the ceiling at both ends, and both ends matter: at the
                                     // ceiling means nothing drained, at zero means the meter was never
                                     // granted — which is exactly the state inert content leaves, and the
                                     // state the previous commit's script hashed while claiming coverage.
    assert!(
        witness.food > 0
            && witness.food < survival.max_food
            && witness.water > 0
            && witness.water < survival.max_water,
        "the drain witness reads food {} water {} against a ceiling of {} / {} \
         — a meter at its ceiling never drained and a meter at zero was never \
         granted; either way survival::step is not on the replay surface",
        witness.food,
        witness.water,
        survival.max_food,
        survival.max_water
    );
    assert!(
        world
            .players
            .iter()
            .all(|p| !p.active || (p.food > 0 && p.water > 0)),
        "a body ran its meters dry inside the script — the widened spans above \
         no longer hold, and a starvation respawn is about to move a body the \
         build arc depends on"
    );
    assert!(
        eaten >= 8 && eat_refused >= 8,
        "the script landed {eaten} consumes and {eat_refused} refusals — the eat \
         verb is falling out of the replay surface"
    );
    // Floors, not `> 0`, for the same reason the build ones are floors: a
    // script that lands one drink exercises the verb about as well as one
    // that lands none. Both halves are named because both are arithmetic
    // the pinned hash is now a function of — the meter write and the hp
    // debit on one side, five `terrain::height` compares on the other.
    assert!(
        drank >= 4 && drink_refused >= 4,
        "the script landed {drank} drinks and {drink_refused} refusals — the \
         drink verb is falling out of the replay surface"
    );
    assert_eq!(
        world.players[20].hp_max, 0,
        "the drinker has hp on this surface now — the drink's hp debit is live \
         here and the comment above it, which says it is not, has gone stale"
    );
    // The barrel loop ran on this surface, and the golden below is only
    // evidence of the roll while this holds. Bots swing on a beach where a
    // quarter of the cells hold a barrel, so an empty container store means
    // the smash never fired and the pinned hash has quietly stopped
    // watching it — the exact failure the loot slice found here, where the
    // gate went green because the fixture was unarmed rather than because
    // the code was right.
    // `next_id` is monotonic from 1, so it counts containers *created*
    // rather than containers still standing — the fixture's despawn ladder
    // is short enough that an end-state count would read zero on a surface
    // that smashed plenty.
    let made = world.backpacks.next_id() - 1;
    assert!(
        made >= 2,
        "only {made} containers stood up in {TICKS} ticks — the barrel smash \
         is not running on this surface and the golden no longer covers it"
    );
    (hashes, world.state_hash())
}

#[test]
fn test_replay() {
    let (hashes_a, final_a) = run(SEED);
    let (hashes_b, final_b) = run(SEED);

    assert_eq!(hashes_a.len() as u64, TICKS / STATE_HASH_INTERVAL);
    assert_eq!(
        hashes_a, hashes_b,
        "replay diverged: same seed + same commands must reproduce every stamped hash"
    );
    assert_eq!(final_a, final_b);
    assert_eq!(
        final_a, GOLDEN_FINAL_HASH,
        "sim behavior drifted from the pinned replay golden; if intentional, \
         regenerate the golden in this same commit"
    );

    // A different seed must actually change the world (guards against a
    // degenerate hash or a sim that ignores its inputs).
    let (_, final_other) = run(SEED ^ 1);
    assert_ne!(final_a, final_other);
}
