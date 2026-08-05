# Gates · PARITY.md — the gap to the reference game, and the route through it

> **Owns nothing.** A survey to cut items from, never a queue — the same
> posture `CLAUDE.md` gives `MENUS.md`. `NOW.md` is still the only list that
> answers "what next"; this file answers "what is the shape of the whole
> thing, and what order does it want." Written 2026-08-05 against the tree,
> not against the docs: every "built" below was read in the code, and every
> "unbuilt" was checked for by grep before it was written down.

## 0 · The finding, in one paragraph

The gap is much less architectural than it looks from inside a bug report.
Most of what is missing is **already designed** in `NETCODE.md`/`TERRAIN.md`,
a large slice is **already paid for** — content rows that are validated,
balance-banded and folded into the content hash, which `bake` then throws
away — and the single most-missed feature, ranged combat, exists as **one
salvageable commit** that flies an arrow in pure integers and was failed for
a wire gap, not for its sim. Three primitives unblock most of the rest.
Nothing found in this sweep requires moving a wall.

---

## 1 · Built (read in the tree, not inferred)

- **World** — seeded heightfield island, biomes, cliffs, beaches, the coast
  road, bays, the haven pad and the waystation (greyboxed).
- **Movement** — kinematic capsule vs terrain *and* placed pieces, step-up,
  jump, wade, sprint/crouch. Quantized both sides; prediction shares the code.
- **Gather** — trees, stone/metal/sulfur nodes, bushes, barrels; weak-spot
  sectors, hit counts, respawn windows.
- **Inventory** — 30 slots, 6 hotbar, containers, server-validated moves,
  splits and stacks.
- **Craft** — the T0–T1 ladder, workbench and furnace stations, a queue.
- **Build** — grid, piece defs, upgrade tiers, repair, structural collapse
  from a broken foundation, decay, upkeep.
- **Deploy** — sleeping bag with respawn cooldowns, boxes, doors, hearth,
  furnace, workbench; privilege and decay interact correctly.
- **Combat** — melee only: reach, a 30° aim cone, point-blank bypass.
- **Death** — one backpack holding the whole inventory, despawn by best-item
  rarity, bag respawn anchors.
- **Survival, chat, loot tables, the satchel's place-and-fuse.**
- **Netcode** — QUIC/WebTransport, delta snapshots against an acked baseline,
  AOI, reliable chunk event streams, prediction, interpolation.
- **The walls** — WAL replay, state hash, native/wasm parity, zero-alloc
  tick, protocol goldens, content hash.

## 2 · Designed and unbuilt — the doc has it, the code does not

Each of these is specified somewhere load-bearing. None needs new design.

| what | where it is specified | state |
|---|---|---|
| ranged weapons | `content/weapons.toml` + `salvage/ranged-v0` | one commit, judged FAIL on wall 6 only |
| lag-comp rewind ring | `NETCODE.md` §8 (formula, 250 ms clamp, ~48 kB) | nothing |
| dropped items that arc and settle | `NETCODE.md` §6.4 (Gaffer-sourced) | nothing |
| arrows/spears that stick | `NETCODE.md` §6.4, same arc | nothing |
| satchel blast falloff | `content/weapons.toml` `blast_m`, baked | column read by nothing |
| anti-ESP occlusion culling | `NOW.md` §7 item 18 | nothing, correctly after M2 |
| monuments | `TERRAIN.md` §7/§8, `reference/SPAWN.md` | pad hook only; zero built |

## 3 · Armed content the sim throws away

This is the cheapest work in the repo and it keeps recurring, so it gets its
own section. In each case the data is written, validated, balance-banded and
hashed — and the sim never reads it. `ranged-v0`'s commit message names the
pattern exactly: *"The data was armed and the sim could not read it."*

- **Bow, crossbow, revolver + their `[weapon.ballistic]` blocks** —
  `bake_combat` drops every row at `kind != Melee` (`combat.rs` §what-v0-does-not-do).
- **Armor** — four rows with `reduction_pct` and `move_penalty_pct`, and a
  balance band asserting any one piece adds ≤ 2 hits-to-kill. No sim reads it.
- **`headshot_mult = 2`** — banded in `balance.toml` so a data edit cannot
  quietly make a headshot a one-tap. No head exists to hit.
- **`blast_m`** — baked, hashed, and read by nothing; a charge still damages
  only the address it was planted on.
- **Per-weapon `rate_per_min`** — every swing currently rides gather's one
  interval.

## 4 · Absent from both code and design

Honest list, so nobody plans against a doc that does not exist: animals and
hunting, day/night (an `ALPHA.md` knob, unbuilt), weapon durability,
recoil/spread patterns, swimming (wade only), corpses/ragdolls (deliberately
skipped for the backpack — Facepunch's own consolidation). Vehicles, farming,
electricity and teams UI are out of scope for v1 by `DESIGN.md` §2.

---

## 5 · The three primitives

Nearly everything in §2 and §3 lands behind one of these, and they are
mutually independent — three lanes can take them in the same window.

**P1 · A radius parameter on the segment query.** `collide::blocked` inflates
every query by `CAPSULE_RADIUS_M` because that constant is baked into it, so
an arrow can only go where a body could go. `ranged.rs` states this in its own
header and calls the radius parameter the honest fix. Unblocks: truthful
projectile paths, line-of-sight on melee (there is **none** today — `strike`
does not even take the collision index), blast line-of-sight, and settling an
item against the world.

**P2 · `EV_SHOT` on the wire.** The one thing that failed `ranged-v0`: a shot
arrives as `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else, so no client can
tell an arrow from a swing and nothing can draw a projectile. Needs the event
code and its subtype, a `PROTO_VER` bump, and the goldens regenerated in the
same commit (wall 6). It needs **no** new `ACT_*` — the bow fires on the
existing `BTN_PRIMARY`. Unblocks: every projectile any client will ever draw,
including the magic-theme staff bolts.

**P3 · The rewind ring.** `NETCODE.md` §8, awake players only, ~48 kB
preallocated, favoring clamped to 250 ms. Unblocks: fair hitscan, headshots at
range, and ranged combat that survives a 150 ms/5% profile.

## 6 · The route

Ordered by what a player notices, with dependencies respected. Each numbered
item is one iteration: branch → build → gates green → merge.

**Phase 1 — the bow fires (P1, P2)**
1. Rebase `salvage/ranged-v0` onto `main`. One commit over a 338-commit-old
   base; conflicts land in `bake.rs`, `combat.rs`, `limits.rs`, `world.rs`.
   Start from the branch — it is a slice, not a rewrite.
2. `EV_SHOT` + `PROTO_VER` bump + regenerated goldens, same commit. This is
   the whole scope of the judged FAIL.
3. Radius parameter on `collide::blocked`; arrows stop threading gaps only a
   player could walk through.
4. Both clients draw the tracer.

**Phase 2 — combat true (M2, needs P3)**
5. The rewind ring.
6. Headshots — pitch already reaches the sim via `pitch_lut`; this needs a
   head volume on the capsule and the banded `headshot_mult` wired through.
7. Armor reduction — read the four rows, honour the ≤2-hits band.
8. Per-weapon cadence off `rate_per_min`.
9. Satchel blast falloff — read `blast_m`, both damage columns linear to zero.
10. Line-of-sight on melee (falls out of P1).

**Phase 3 — things behave when you drop them**
11. Ground snap in `backpack::stand_up`. One call. Fixes a live defect today:
    `drop_for` is called from inside `die()` at the death address, so a bag
    dropped mid-air or on a piece that is later raided hangs where it was.
12. The class D arc + forced settle, `NETCODE.md` §6.4 as written — 16-tick
    rest ring, 2 s deadline, `settled` event carrying the resting transform.
    Supersedes 11 cleanly.
13. Arrows and spears that stick, same arc, `stuck` event.
14. Cosmetic tumble and spin in Bevy — unhashed, uncommitted, free.

**Phase 4 — the world reads as a place**
15. Monuments. Greenfield; the theme call (`§7`) wants answering first.
16. Day/night cycle.
17. Animals.

**Phase 5 — M3/M4** — the economy behind its switches, ops, the launcher,
the door. Unchanged by anything above.

**The dividing rule, for every item in phases 3–4:** if it can change what
you can loot, it is sim, integer-quantized and hashed. If it only changes
what you see, it is the renderer's and costs the walls nothing.

## 7 · What only the operator can answer

- **The theme.** Whether the item set, monuments and weapons go fantasy
  (staffs, pylons, ruins) or stay scavenger-industrial. It is nearly free in
  code — the sim is theme-agnostic and only three `"item.` strings exist
  outside content — and it is **not** free in the visual gate, which is
  measured off `Rust Images/`. Changing the nouns is cheap; changing the
  lighting register unanchors `ART.md` §3 and the visual rubric together.
- **Whether monuments wait on that call.** Phase 4 item 15 says they should.
- **The knobs each phase proposes**, per `CLAUDE.md`: written into
  `DECISIONS.md` §open as proposed, never into code as invented.
