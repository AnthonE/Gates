# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.
An item is ≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `gates-loop/findings/` note.

> **Rebuilt 2026-08-05.** The file had reached 2040 lines: `merge=union`
> means three lanes append and nothing ever deletes, so it accumulated ~12
> items whose own titles said "done this pass", a duplicate, and a large
> block of browser-renderer work the client pivot retires. Everything
> removed is in git. Nothing open was dropped; where an item is retired
> rather than finished, it says so.

---

## 1 · The client is becoming a native Rust desktop app

*(Operator, 2026-08-05. `DECISIONS.md` has the row. This outranks the
milestones below and retires a block of browser work outright.)*

Two slices have landed and both are on `main`:

- `crates/client` — the session. Connects to an **unmodified** shard over
  the same `wtransport`/QUIC the browser used, same `PROTO_VER`. Measured
  against a live shard: `snap sent 135`, 120 applied client-side,
  `in ok/bad/drop 270/0/0`, `leaves 0`.
- `crates/client/src/bin/gates.rs` — Bevy 0.18 behind the optional
  `render` feature (default **off**; the code tier stays ~106 s). Chase
  camera, reference plane, a cuboid per body. **Runs, and draws** — 30 s
  under Xvfb + lavapipe against a live shard, frame captured, session
  healthy throughout (`in ok/bad/drop 729/0/0`, `snap sent 434`). Item 2
  has the recipe.

**The rule that holds the pivot together: Bevy draws, it does not decide.**
`sim-core` keeps the walls, `ClientCore` keeps prediction; the ECS reads
those and writes transforms. Gameplay state in a Bevy component would
retire the determinism walls with nothing in CI to notice.

Next slices, roughly in order:

1. **Input** — keyboard/mouse into `ClientCore::set_input`. Every verb
   exists server-side; nothing native can press one yet.
2. **Terrain** — mesh `sim_core::terrain`. It is a pure function of the
   seed and both sides already agree on it, so this is meshing, not
   design. `web/src/terrain.js` is the reference for *what* to draw.
3. **A native visual gate** — item 2 below. The pivot's real debt.
4. **HUD, inventory, container panel** against the wire that already
   carries them (v19 `ACT_CONTAINER` / `SUB_CONT_SYNC`).

Retired by this pivot rather than finished: `MIGRATION.md` (three.js →
`WebGPURenderer` + TSL) is **moot** — you do not port three.js *and*
replace it. The lighting red (`TONAL_MAX_P10`) goes with it: it is the
coupled tonemap/sky/exposure/fog set, and a port re-derives that set. Also
retired — the shader-arithmetic, texture-photograph, shadow-distance and
capture-drift items. All are in git before this commit. `web/` still
builds and is still gated; nothing is deleted until the native client can
replace it.

---

## 2 · The native visual gate — the recipe exists, the gate does not

Every visual gate is browser-shaped: `browser_smoke`'s 12 probes, 43
`readPixels` sites, `vantages`, the capture harness. A native client
inherits none of them, and `MIGRATION.md` already stated the rule this
inherits — **a render path that lands without its probes ships a client
with no visual gates at all**, which is forbidden outright.

**The box CAN see — proven 2026-08-05, and this is the recipe.** The
earlier claim here (no display, therefore no native visual gate) was
wrong. The client ran for 30 s and a frame was captured off it:

```
Xvfb :99 -screen 0 1280x720x24 &
DISPLAY=:99 WGPU_BACKEND=vulkan \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  ./target/debug/gates 127.0.0.1:<port>
DISPLAY=:99 xwd -root -silent | xwdtopnm | pnmtopng > frame.png
```

`AdapterInfo { name: "llvmpipe (LLVM 20.1.2)", backend: Vulkan }` — Mesa's
lavapipe software rasterizer, no GPU needed. The session stayed healthy
throughout: `in ok/bad/drop 729/0/0`, `snap sent 434`, `leaves 0`.

So a native visual gate is **buildable here now**, and it is the next
gate to write. Two notes for whoever writes it: lavapipe is a CPU
rasterizer, so budget on frame COUNT and pixel assertions, never on frame
time (`CLAUDE.md`: a gate that waits on a clock is not a gate on this
box); and one live renderer at a time, since two was the browser tier's
whole problem.

**What the first frame showed, unfixed:** the body draws and is lit, and
there is no ground under it. The reference plane is at `y = 0` while the
player spawns at terrain height, so it sits far below the camera and out
of frame. Fixed by slice 2 of item 1 (mesh the real heightfield); until
then the plane is decoration in the wrong place.

---

## 3 · `browser_smoke` is red on a clean trunk, twice over

Both measured, both confirmed pre-existing on unmodified trunks, neither
caused by a diff. Kept because they explain why the renderer tier has not
run since 2026-08-04 — not because they are queued work:

- **tab B never reaches the world** — two live renderers on a box with no
  GPU. `__gatesDebug` never publishes, ~68–70 s of liveness cap, while tab
  A reaches the world in under a second in the same run. The
  two-live-renderers class `CLAUDE.md` names. Not a timeout to widen.
- **`TONAL_MAX_P10`** — p10 luma 112 against a ceiling of 60 (reference bar
  40.5). Retired by item 1.

---

## 4 · The event lane's payloads are law with no gate

Nine `EV_*` codes carry positional `u32` fields whose meaning lives only in
a `/// EV_*: a = … b = …` comment in `world.rs`. Swap `a` and `b` at an
`events.push` site and every wall stays green: the encoder is untouched
(`test_protocol_golden` green), the event queue is not in `state_hash`
(`test_replay` green), and every field is `u32` (clippy green).

This is the hole `reference/FINDINGS.md` §1 measured in the reference
ecosystem — 49 Oxide commits touching hook arguments, ~27 correcting a
payload that had already shipped wrong, four hooks corrected more than
once, and their `MSILHash` (the exact analogue of our golden) caught none
of them. `event_roles.rs` covers part of this now; finish it.

---

## 5 · Gameplay still missing, in rough order of what a player notices

- **Jump.** Gravity is there, jump is not — and jump is what makes a lintel
  matter. Wire change, so systems lane only.
- **Ranged.** There is a revolver in `loot.barrel` and nothing to fire it.
  `salvage/ranged-v0` is a judged-**FAIL** attempt (wall 6, the wire
  drifted, reproduced executably). Read the report before rebuilding.
- **Dropped loot** should land somewhere you can find, not inside the floor.
- **Base repair, decay and upkeep** — a base can be broken into and cannot
  be repaired; this is the loop that makes a day matter.
- **Death and your own base** — a death evicted you from what you built and
  nothing you built said otherwise.

---

## 5b · The wire accepts values the sim can never mean

`every_domain_fits_its_wire_field` (`protocol/src/event.rs`) now gates ten
value domains against the fields that carry them — a sim domain outgrowing
its wire field is the shape of the 2026-08-05 FAIL, and it is caught now.
Writing it measured two live holes running the *other* way, left unfixed on
purpose: narrowing what decodes is a wire act, and that pass was a gate.

- **`BAG_GONE_*`** — `encode_event_bag_removed` bounds against the *width*
  (`why >= 1 << BAG_GONE_BITS`), not the domain (largest live is 2), and
  the decoder does not bound it at all. `why == 3` round-trips as a
  removal reason that means nothing.
- **`REFUSE_C_*`** — 4 bits for a domain topping out at 3, and neither end
  bounds the upper edge; only `reason == 0` is refused. Values 4..15 cross
  intact.

Both are forgery slack, not drift: the sim cannot emit either today, so
nothing is broken for a player. The fix is the closed-set posture
`DEATH_BY_*` now has — a derived `*_MAX` on the sim side, checked at both
ends — and it wants its own pass because it changes what decodes, which
means deciding whether a narrowing owes `PROTO_VER` a bump.

Systems lane (`crates/protocol`, `crates/sim-core`).

---

## 6 · Unmerged work, kept deliberately

Nothing judged PASS is stranded. These failed or were stopped, and the
harness kept them rather than merging. **Do not merge one to clear the
list** — failed work in the trunk is the one thing the judge exists to
prevent. If a lane rebuilds any of these, start from the branch.

| tag | what | why it is here |
|---|---|---|
| `salvage/ranged-v0` | ranged weapons | judged FAIL, wall 6 |
| `salvage/bark-photo` | bark texture | judged FAIL; textures retired by the pivot |
| `salvage/m1-surface-grain` | surface grain | stopped unmerged; same |
| `salvage/container-contents-wire` | container wire v19 | duplicate of what landed |
| `salvage/container-contents-2` | container wire v19 | duplicate of what landed |
| `salvage/cont-max-mirror` | `CONT_MAX` fix | absorbed; content-identical to `main` |

---

## 7 · Milestones

13. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
14. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
    projectiles · the anomaly log.
15. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch · the
    claim rail · shard ops.
16. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle.
17. **M4 — arm A2, then A3** (operator acts): claim rail export · skin rail
    · the desktop launcher.
18. **Anti-ESP occlusion culling** — the measure the genre proved
    (Facepunch, 2025, network-wide default). Server-side, costs no client
    trust, and the occlusion grid is a pure function of the seed, so it is
    bakeable at worldgen and a lookup in the tick. Sequence after M2: it
    wants real sightlines to tune against.
19. **The launcher, in Rust, with the wallet in it** (`DECISIONS.md`
    2026-08-04). One static binary, `egui`, no webview: patcher, shard
    list, balances, and a self-custody wallet on `alloy` signing the
    EIP-191 join the server already accepts — so no protocol moves and
    nothing enters the sim's blast radius. Key backup is the feature, not a
    footnote: phrase shown once and confirmed back, encrypted keystore
    only, never logged and never in the WAL, and the plain sentence that
    the operator holds no keys and can restore nothing. **Unchanged by the
    client pivot** — it was always Rust, and it is the platform's client
    for the whole cascade, not a Gates accessory.
20. **`cargo test --workspace` overflows a debug thread's stack**; only
    `--release` (what CI runs) is green. Pre-existing. The cause is size,
    not logic — `World` is ~416 kB of fixed capacity and `ShardCore::new`
    builds it on the stack, so an unoptimized frame holds two or three
    copies against a 2 MB limit. It bites anyone who types the obvious
    command. Fix: box the big fixed-capacity members (`Pieces`, `Deploys`,
    `SlotLives`) at construction, the way `ShardCore` already boxes its
    client array — one allocation at boot, none in the tick.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.
