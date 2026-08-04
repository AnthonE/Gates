# reference/FINDINGS.md — what the mod loaders' commit history teaches

Ripped facts again, not design. `reference/rust-systems.txt` answers *what
systems exist*; this file answers *which ones bled, and where*. A decade
of mod-loader commits is a log of every place the reference game's own
surfaces turned out to be harder than they looked.

Dated 2026-08-04. Corpus: `OxideMod/Oxide.Rust` **1,455 commits** and
`CarbonCommunity/Carbon` **6,134 commits** (7,589 total), full history,
commit subjects mined and the interesting diffs read.

## 0 · What is NOT in here, so it is not mined again

**Roughly 6,100 of the 7,589 commits are loader infrastructure, not game
knowledge.** Carbon's history is dominated by its own plugin pipeline:
threading, Roslyn compilation, hot-reload, Harmony patch lifecycle,
assembly publicizing. The raw bug histogram over both repos — 52 NRE/null,
50 perf/alloc/GC, 30 race/thread, 25 index/range, 19 memory — is a
C#-runtime-inside-Unity story, and our sim is single-threaded,
allocation-free after warmup, and has no plugin loader at all.

The yield is in Oxide.Rust, which tracks the *game*. That is where the
rest of this file comes from.

## 1 · The dominant bug class is payload shape, not logic

**49 commits in Oxide.Rust touch a hook's arguments. About 27 of those are
corrections to a payload that had already shipped wrong**, and at least
four hooks had their payload corrected more than once —
`OnEntityBuilt`, `OnCollectiblePickup`/`Pickedup`, `OnEntityReskin`,
`OnItemStacked`. A sample, verbatim:

```
Fix wrong second argument for OnPlayerSpawn hook
Fix wrong arguments being passed to OnWireConnect and OnWireClear
Fix wrong argument being used in OnHammerHit
Fix incorrect arguments in OnEntityBuilt hook
Fix incorrect CanBeTargeted hook argument
Fix OnLootPlayer hook providing wrong 2nd arg
Fixed 2nd argument in OnRotateVendingMachine hook
Swap args for OnBonusItemDrop, add OnBonusItemDropped
Make CanLootEntity [resource container] arg order match others
Update argument order in various "Can" player hooks
Revert OnQuarryGather 2nd argument to Item
```

Not one of these is a logic error. Every one is *the right value in the
wrong position*, shipped, used by plugins, and found in the field.

**Why their gate did not catch it.** Every hook record in `Rust.opj`
carries an `MSILHash` — a hash of the patched method's body, so a game
update that changes the method flags the patch. That is a genuinely good
gate, and it is the *exact analogue of our byte-golden*. It caught none of
the 27, because **a hash over the shape of a payload is blind to the
meaning of the fields inside it.**

### 1.1 · We have the same hole, and it is wider

Our event lane is 25 event types (`world.rs`), each documenting its
payload in a doc comment over a positional `u32` triple:

```rust
/// EV_DEATH: a = the player who died, b = the player who killed them
/// EV_HIT: a = attacker player id, b = victim player id, c = damage dealt.
/// EV_GATHER: a = player id, b = item index << 16 | units actually added.
```

Swap `a` and `b` at an emit site — `events.push(EV_DEATH, attacker, victim, 0)` —
and every gate stays green:

- **`test_protocol_golden`** pins the *encoder's* bytes. The emit site is
  not the encoder; the fixture never moves. Green.
- **`test_replay`** pins state hashes. `World::state_hash` covers seed,
  tick, bodies, frames, hp, the survival clock and inventory. **The event
  queue is not state and is not in it.** Green.
- **clippy / the type system.** Every field is `u32`. A swap type-checks.
  Green.

The consequence is silent and permanent: every kill feed on every client
names the wrong killer, forever, and nothing in CI has an opinion. Two of
our events are worse than `EV_DEATH` because both fields are the same kind
of thing — `EV_HIT` (`a` attacker, `b` victim) has the identical shape.

This is wall 6's own principle — "the wire never drifts by accident" —
with a hole in it exactly where the reference ecosystem's history says the
drift actually happens. `CLAUDE.md`'s "a law without a gate is a mood"
applies to the doc comments above: they are law, and they have no gate.

**The gate, now partly built** — `crates/sim-core/tests/event_roles.rs`,
`NOW.md` item 1. Not a byte fixture; that is the gate that already exists
and already misses this. It drives one known cause through the real
`World` and asserts the fields against their roles: `EV_HIT`, `EV_HEALTH`,
`EV_DEATH`, `EV_BAG_DROPPED`, `EV_GATHER` — the five where two fields are
the same *kind* of thing and so nothing but the values tells them apart.
Two disciplines make it able to fail: a check whose three fields are not
mutually distinguishable is refused outright (a permutation would satisfy
it), and each code must appear exactly once on its tick, which makes it a
double-emit gate as well. Coverage is pinned at 5 of 25 rather than
implied, so the remaining twenty are a stated debt.

**Stronger still, and not yet built:** a payload-role table that both the
emit site and the check read, making a swap a *compile* error rather than
a test failure. Larger than the twenty remaining checks, and it should not
block them.

## 2 · The item-move path is where the reference actually bled

Three commits on 7 Feb 2019, twenty-eight minutes apart, all one-line
changes to the same file:

```
20:54  Fixed kick when moving items          InjectionIndex 65→66, RemoveCount 3→2
20:58  Fixed item stacking/combining patches
21:22  Fix the fixed player looting kick issue   InjectionIndex 45→43
```

The third is titled as a fix of the fix. The mechanism is in the diffs:
the hook was spliced at the wrong point in the method — either ahead of a
validation the server needed, or eating one real instruction too many —
and **the server kicked the client**. Not a wrong number, not a lost item:
a disconnect, because the server's container state and the client's
diverged and the anti-cheat path treats that as a forged request.

Two things follow, and both point at the keystone verb from `MENUS.md` §6:

- **The item-move verb is the most bug-prone thing in the reference
  game**, and it earns that on the *validation ordering*, not the
  arithmetic. Where the check sits relative to the mutation is the whole
  game.
- **Its failure mode is desync-kick.** For us that is worse than it was
  for them, because prediction means the client has already drawn the
  move. `CLAUDE.md`'s existing quantize-both-sides trap is the same law:
  the server sims on the values it transmits, and a container move has to
  refuse on the same values the client predicted with.

Related, from the same corpus: `Fix players being kicked when gathering
hemp (IL error in OnCropGather)`, `Fixed OnRecycleItem hook stopping
recycling prematurely`, `Fixed NRE in OnItemCraft`. The item lane is where
their fixes cluster, by a wide margin.

## 3 · One trap we already dodged, for an unrelated reason

```
Fixed stack overflow in OnPlayerDeath & integrated base code for Die()
```

Their death hook could re-enter: `Die()` fires the hook, a handler calls
something that kills again, and the stack unwinds into the floor.

Ours cannot, and the reason is worth recording so nobody "simplifies" it
away. `world.rs` separates the announcement from the respawn — its own
comment says it plainly:

> the callee counts it and announces it, the caller walks the spawn ring —
> because `respawn` needs the whole world and the verb needs one player

That is exactly the fix Oxide had to retrofit, and we did not reason our
way to it: `respawn` needs `&mut self` while the verb holds one player, so
the borrow checker refused the re-entrant shape before anyone could write
it. **A trap bought for free, but only as long as the split survives.**

## 4 · Event reuse — ours, flagged before it bites

Their `Removed duplicate OnBonusItemDrop hook` and two rounds of
`Fixed double deprecated hook call with OnActiveItemChange/d` are the
same family: one cause emitting one event twice, or two causes emitting
one event that consumers cannot tell apart.

We have the second half already, and — correcting the first cut of this
file — it is documented intent rather than an oversight. **`EV_GATHER` is
emitted from three modules**: `gather.rs` (a harvest), `world.rs`, and
`backpack.rs` (a bag loot). Its doc comment argues the case out loud:

> Read it as "these units entered your inventory", not as "a node paid":
> looting a backpack announces its take the same way, and deliberately —
> the client's `+N Item` toast is the right feedback for both.

So this is a decision, not a defect, and the reference's own duplicate-hook
fixes are not evidence against it. The one thing worth carrying forward is
where the decision stops holding: the moment anything *counts* gathers — a
stat, a quest, an economy sink — looting your own bag scores as
harvesting, and the cause has to become a field rather than an inference.
`test_event_roles` checks the packed halves of `EV_GATHER` today; it
cannot check a cause the payload does not carry.

## 5 · The parallel worth stealing

`Rust.opj` pins an `MSILHash` per patched method so that a game update
flags every patch whose target moved. We already do the isomorphic thing
one level up — the content hash in the WAL header, so a replay replays the
content it was played under (wall 7).

The lesson from §1 is not that hashing is wrong. It is that **they hashed
the thing they were attached to and not the thing they were promising**,
and the 27 payload fixes are what lives in that gap. Ours has the same
shape: we hash content and pin wire bytes, and the promise nobody hashes
is what each field in an event *means*.
