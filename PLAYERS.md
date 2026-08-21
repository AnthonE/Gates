# PLAYERS.md — the agent player

**DESIGN, 2026-08-05. One wall of it is built.** `crates/sim-core/src/bots.rs`
drives deterministic synthetic input and the `bots` bin runs it at scale, so a
non-human client is already first-class, and **wall 3's event exists**
(`EV_TRUST`, 2026-08-18) — which is deliberate ordering rather than a
convenient place to start: the online field cannot be retrofitted onto a
record already written. What does not exist is the intent API, the verb table,
the other three gates, or anything that reads a trust row. This doc owns that
surface and nothing else. `DESIGN.md` still owns the product, `NETCODE.md` the wire,
`CONTENT.md` the numbers.

The research half — why a survival game is a field site, what the measurement
is, what would falsify it — is `scry-forge/docs/SUBSTRATE.md`. This is the
game-side half: what an agent may do here, and which walls keep that
measurable. Read that one for *why*; this one is *what*.

## What an agent player is

A client. It speaks the same protocol, pays the same doors, earns the same
coins, and dies the same way. `CLAUDE.md` already says agents will play this
and that the deterministic core doubles as an RL environment — this is that
sentence made concrete.

What it is **not**: a difficulty setting, an NPC, a quest-giver, or a
population filler. Those are content. An agent player has an owner, an
identity, a wallet, and a public record, and is scored by nothing the server
computes.

## The four walls

Each with its enforcement, because a law without a gate is a mood. **Wall 3's
is built** (2026-08-18, `NOW.md` §5d); the other three are unbuilt, each is a
small test, and none of them should land after the verb API rather than with
it.

1. **Agent verbs are a subset of human verbs.** An agent must never have an
   affordance a human client lacks — no extra reach, no wallhack, no state a
   player could not have learned by standing there. This is the fairness rule
   and the science rule at once: an agent with superpowers is an aimbot, the
   population's hostility toward it becomes correct, and the betrayal it
   commits is not comparable to a human's. → a test asserting the agent verb
   table is a strict subset of the player input table, and that the observation
   encoder is a pure function of what that client's snapshot already carried.

2. **No global leaderboard.** No endpoint returns a total ordering across
   ladders, and no ladder is convertible into another. Wealth, structures
   standing, alliances held, nights survived — plural and deliberately
   non-commensurable. One ranked number rebuilds a benchmark and manufactures
   metagamers on purpose. The reference game has no global ladder either, so
   this costs nothing the genre wanted. → a test that no response carries a
   cross-ladder total, and no ladder exposes a weight.

3. **Every trust-bearing verb is an event with a role-checked payload.**
   **BUILT** — `EV_TRUST` (`world.rs`, 2026-08-18): a = the actor, b = the
   counterparty, c = `TRUST_*` verb << 8 | `PRESENCE_*`, pushed wherever a
   verb answered to somebody else's record — a leaf worked, a lock's code
   accepted, a hearth crew seat taken, a container moved through. This is the
   surface that most needs the gate: an `a`/`b` swap at a betrayal site
   silently corrupts the whole record the measurement reads while every other
   wall stays green, and nothing encodes this event, so not even a byte-golden
   is watching. It landed **inside** `crates/sim-core/tests/event_roles.rs`
   with four causes, five proven silences and a closed-ledger parse of both
   value domains, every assertion reproduced red under its own mutant
   (`NOW.md` §4's discipline).
   Two things this doc got wrong and the build corrected. **The online field
   is not a bool**: a sleeper and an evicted body are different facts about a
   counterparty, so presence is awake / asleep / gone. **`item given` has no
   code**, because there is no player-to-player give verb in the sim —
   `TRUST_GIVE` lands with the verb, not before it. → the next social verb
   lands the same way; two causes per code, and the presence field proven to
   vary across them.

4. **Determinism holds with agents in the loop.** An agent client is an input
   source like any other; the sim must not learn it exists. No clock, no I/O,
   no allocation enters the tick because a player is a model. → `test_replay`
   and `test_alloc_zero` extended over a scripted agent-input fixture, in the
   same commit as the API.

## The verb set

Bounded, capped in `limits.rs` like everything else (wall 4), and small on
purpose — the interesting behaviour is social, not mechanical.

`move · look · build · open · take · give · attack · speak · authorize`

`authorize` (granting another player TC or door access) and `give` are the two
that carry the whole design: they are the only verbs that create a trust
relationship the world can later observe being honoured or broken. They are
also the two most likely to be requested as "just a convenience API" and
flattened into something unloggable. They are not convenience.

## The observation encoder

A pure function of the snapshot that client already receives. It answers: what
is in view, who is present, what is in this container, what do I hold, is this
base's owner online.

The last field is deliberate and it is the one to get right. It is ordinary
game state — a human sees it in the same moment — and it is also the condition
the whole measurement turns on (`SUBSTRATE.md` §3). It must be logged at every
trust-bearing verb from the first shard that runs; retrofitting it makes the
early record worthless. The **logging** half is built (wall 3 above); the
encoder is not, and neither is a sink for the rows — the sim mints them and
`ShardCore`'s event drain currently ignores the code.

## The model does not drive at frame rate

A small policy handles real-time control; the model sets goals and does the
social reasoning. That split is honest about what a language model can do at
30 Hz, and it puts the model exactly where the behaviour worth watching is.

Determinism pays a third time here: a replayable sim means every agent episode
reproduces byte-exactly, so the trace handed to the meter is free and needs no
capture path of its own.

## What this does not decide

The economy side — what an agent pays to enter, what it earns, whether an
agent's coins are its owner's — is `ALPHA.md` and the elo side, not this doc.
The vow (what an agent declares it plays for) lives in elo's `VOWS.md` and
binds through `playauth`; nothing about it belongs in `sim-core`, which must
not learn that vows exist.
