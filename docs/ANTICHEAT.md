# ANTICHEAT.md — Gates' half of the anti-cheat plan

The plan of record is oneshot's `docs/ANTICHEAT.md`, with its sources in
`docs/ANTICHEAT-SOURCES.md` there (operator, 2026-11-07: *"we would require
a profile on a player before we ever payout from games. then we can catch if
they deviate?"*). oneshot goes first because it has 128 Hz aim, replays that
re-run to the server's hash, and a review queue. This page is what changes
for Gates, and what Gates already gives the plan. The payout side is Elo
Pros' (`docs/store/PLAYER-PROFILES.md` in scry-forge).

## 1 · The plan in five lines

1. **A profile per wallet before any payout.** One match can't decide (at a
   4% false-positive rate and 1% cheaters, about 80% of single-match flags
   are innocent); many matches multiply into a decision (Wald's sequential
   test).
2. **Read it three ways:** against the player's own past (a cheat switched on
   is a step, improvement is a ramp); against the rest of their game (an
   aimbot raises aim and nothing else); against everyone (cheaters as a tail
   or a second clump, and an estimate of how many).
3. **A ban costs the profile.** Wallets are free; a profile that took weeks to
   build isn't.
4. **Our own aimbots, bred** (`sim-core/src/genome.rs`, `examples/zoo.rs`,
   `examples/school.rs`), laid over real play as the detector's training
   data, scored by the largest advantage a cheat gets unflagged.
5. **Evidence, never a verdict.** Models rank; the operator bans (`CLAUDE.md`).

## 2 · What Gates already has

- **`crates/server/src/cheat.rs`:** snap hits, headshot rate and wall
  tracking per player, as `kind:"suspect"` lines in the anomaly log
  (`anomaly.rs`). Thresholds still first guesses (`NOW.md` §0chk).
- **The breeding stack the plan copies:** `genome.rs` (the seed-chain genome,
  Such et al. 2017), `zoo.rs` (a (1+λ) strategy against the agent in the
  arena's lockstep), `school.rs` (a distilled student bred past its teacher on
  `lockstep.rs` islands). oneshot copies `genome.rs` under its `RIP.md` rule.
- **The payout point.** `standings.rs` writes each wipe's payout file
  (`payout_path`), keyed by wallet; Elo Pros pays it (`meter/gamepay.py`).
  The profile gate is a filter there: a row pays only when its wallet's
  profile is in good standing, and a held row is listed as held, not dropped.
- **The armed set is the perimeter** (`DESIGN.md` §10). Only armed, official
  shards pay, so only they need profiles. Declared agents stay exempt
  (`standings_agent`); they don't play armed shards.

## 3 · What is different here

- **30 Hz, not 128.** A tick holds a third of a 100 ms flick, so a fine aim
  fingerprint is out of reach. Gates' profile leans on what a survival game
  has more of: movement, routes, building, fights chosen and fights fled,
  time to first shot after a sighting. View 2 (aim against the rest of the
  game) is the strongest view here.
- **No input log yet.** `DESIGN.md` §10 names "offline aim analysis over the
  WAL", and `lib.rs` says the WAL is still a later slice. Until it lands, no
  match can be re-run with a cheat laid over it, so Gates' bred cheats play
  in the arena's lockstep against the agent, as the wolves do (`zoo.rs`), not
  over human recordings.
- **Long sessions, not matches.** The unit is a fight (a sighting to a death
  or a break-off), and the profile is per wipe as well as per wallet.
- **Bows and arrows.** Snap is measured at release and the hit lands later
  (`SNAP_HIT_TICKS`); a model must read the release, not the hit.

## 4 · Gates' stages

**Stage 0 — no players needed.**
- A profile row per player per session from `cheat.rs`'s counts plus the
  rest of the game, written off the sim thread like the anomaly log.
- The payout file gains a hold: wallets without a profile in good standing
  are listed as held, with why.
- Cheats as genomes in the arena (`tests/arena/rig.rs`), bred against
  `cheat.rs` the way `zoo.rs` breeds wolves.

**Stage 1 — the first armed wipe.**
- Tune `cheat.rs` on real `suspect` lines (`NOW.md` §0chk item 1).
- An admin verb that lists a wipe's flags by name and spectates the suspect
  (§0chk item 3), so review is an in-game act.
- The payout hold goes live.

**Stage 2 — after the WAL.**
- Overlays on recorded play; the detector, the change-point watch and the
  player embedding from oneshot's plan, ported.
- Occlusion culling stays the prevention for ESP (`DESIGN.md` §10, §0chk
  item 4); wall tracking only detects it.

## 5 · Rules (the same as oneshot's)

- Keyed on the wallet; play numbers only; on the box; deleted after a set
  time unless a case is open.
- The terms say play is profiled before a payout.
- Trained weights, the bred zoo and profile data stay off the repo. The code
  and this plan are public.
