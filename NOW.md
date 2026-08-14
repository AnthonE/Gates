# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.
An item is ≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `gates-loop/findings/` note.

> **Rebuilt 2026-08-05, then pruned again the same day.** The file had
> reached 2040 lines: `merge=union` means three lanes append and nothing
> ever deletes, so it accumulated ~12 items whose own titles said "done this
> pass", a duplicate, and a large block of browser-renderer work the client
> pivot retires. **Nine more "done this pass" items had accumulated by that
> evening** — 966 lines again within hours of the rebuild, which is the
> merge strategy and not the lanes' discipline. Pruning is therefore
> recurring maintenance, not a one-off: if it is not automated, budget it
> every few passes. Everything removed is in git. Nothing open was dropped
> — §0q exists because two judge-ranked gaps were written down **only**
> inside a done item and would have gone with it.
>
> **Pruned again 2026-08-09: 3,839 → 644 lines, 55 → 30 sections.** That
> day's ~15 landed slices were struck in place and are deleted here; the §8
> un-reconciled block and every 2026-08-05 "done this pass" item went with
> them, and the stale triage sections (ranged, jump, the browser gates) were
> verified against the tree before deletion — ranged landed 2026-08-06,
> jump is pressed natively, the `loop/*` branches and browser gates are
> gone. Five live gaps were lifted out of done items before they went: §0q
> gained two (standing ON occupants, the 100-bot soak) and §5 three (the
> invisible arrow, mushrooms/corn, day/night) — the same rescue the first
> prune performed.
>
> ⚠ **Section labels collide, and it is the merge strategy again.** Eight are
> duplicated (`0a 0u 0v 0w 0x 0y 0z 4b` — `0v` three times), because
> `merge=union` lets each lane pick "the next free letter" against a file
> that does not yet have the others' picks. About 39 citations in other docs
> point at an ambiguous target that way; `NOW.md §0y` is nine of them and
> resolves to either the sea or persistence. **Not renumbered here**: the
> citations are mostly in `DECISIONS.md`, which is the dated record and is
> not rewritten to match a later tidy. Read a `§`-citation as a hint and
> match on the title, and when you next edit a colliding section, give it a
> label no other section has.

---

## 0gi · The island reads as one surface — two causes, one landed *(client+sim lane)*

From the visual judge's ranked gap 1, pass `20260814-142610-01`: *"the whole
island is hue 29–35°, and zero pixels of `ART.md` §3's grass band (63–74°)
exist on the ground"*. Measurements in
`gates-loop/findings/note-20260814-the-island-has-one-hue.md`.

Landed 2026-08-14: `GROUND_ALBEDO` re-placed against §3 — litter was the most
saturated surface on the island and warm, so it took the hue of every mix, and
it is 37.6% of the land. Brightness is gap 2's owner (`rig.rs`), so the ground
mean was held rather than moved — **0.0495% under Rec.601, 1.20% under
Rec.709**, and the "±0.01%" this line used to claim was neither. The repo uses
both estimators (`ground_identity.rs:139` 601, `terrain_mesh.rs:209` and
`water.rs:714` 709) and they disagree by 24×; the constraint is also the one of
four with no gate. Gate `crates/client/tests/ground_identity.rs`, 5 tests, 4 red
on the old constants. 58.1% of land in the grass band, p10 38.0° / p90 68.5°.

Remaining, in order:

1. ~~Granite is authored and never drawn~~ — **struck, and the reason it was
   struck is itself retracted (2026-08-14, both the same day).** It is not that
   the shipped island is a flat 1-in-40: that came from sweeping `-1024..1024`
   on a world centred at (1024, 1024) — **one quadrant**, and not the one the
   camera stands in. Over the whole square seed 20260731 reaches 106.00 m,
   slope 2.665 and granite on **10.0%** of its land (44-island median 7.2%),
   and **8.9% within 300 m of the capture spawn**, where the median island
   paints 0%. Granite is authored, reachable and near the camera. No seed
   moved. The bands still may not: they are ramps centred on
   `CLIFF_SLOPE_RATIO` and `biome()`'s Highland edge, and
   `crates/sim-core/tests/relief.rs` is red under that edit and now also under
   the quadrant window's return.
2. **So the missing green is the renderer's, and item 3 is the live one.**
   Nothing about the world explains a frame with no granite in it — the rock is
   there, in view distance of the spawn. Whatever eats it sits between
   `splat`/`vertex_color` and the pixel.
3. **Something between `vertex_color` and the pixel eats the green.** The OLD
   constants already held two hue populations (31.1° and 84.0°) while the judge
   measured 29–35° with nothing above it. Untested: the granite photograph's
   chroma through `base_color_texture`, the lighting, the tonemap, or a near
   band that is mostly clutter and props rather than ground.
4. **The judge read real geometry as paint.** `render/clutter.rs` ships 721
   elements a tile and is drawn; what it lacks is a shadow (`NotShadowCaster`,
   deliberately) and any contact darkening (no SSAO anywhere). `ART.md` rule 2.

## 0pop · The shard has inhabitants — what they cannot yet do *(server lane)*

From `findings/pass-20260813-230343-18-judge.md` §B.1, ranked first by three
consecutive judges: *"a shard has no inhabitants, so none of the last four
passes' work is reachable by a player."*

**Landed 2026-08-14.** `shard.toml` grows `population = N` and
`crates/server/src/population.rs` seats it — bots dialled over the shard's
own wire after the bind, full handshake, `run_bot`, so the server cannot tell
one from a player. Resident rather than a fleet: a post runs a bounded shift,
reports its `BotReport` into a gauge, and is re-manned until the shutdown
flag. Bounded at `MAX_PLAYERS - 1`, so a seat always stays a person's, and
refused outright beside `require_auth` (an inhabitant is a guest, so an
authenticating shard would have it re-dial its own closed door). Gate:
`crates/server/tests/population.rs`, 8 tests; the live one seats 4 on an
ephemeral shard and waits on `joins`/`input_dg_ok`/`actions_ok`, observed red
with no post manned (`joins 0, live 0, inputs 0, actions 0`).
Measured, 4 seated: joins 4, inputs 10, actions 3, 0 malformed, one 100 ms
look. `bin/bots`'s row resolver now calls the same one.

Remaining, ranked:
1. **Nobody has run one for longer than a test.** A shift is 300 s and the
   suite exercises ~0.2 s of it, so re-manning, the backoff and the shift
   report are gated only by construction. Cheapest next step: `population = 8`
   in `shard.toml`, run it, read the population line.
2. **They act, but nobody has checked what they can afford.** The suite uses
   the shipped spawn kit deliberately and asserts only that actions land —
   judge -18 §B.2 is the live half of this, and the satchel is still granted
   everywhere rather than crafted.
3. Two proposed defaults are in `DECISIONS.md` §open ("shard population v0").

---

## 0rc · The raid completes — what is left of it *(systems lane)*

From `findings/pass-20260813-230343-16-judge.md` §B.1, ranked the largest
playable gap in the game: *"You cannot get into anyone's base."*

**Landed this pass.** The cheap next step named by
`findings/note-20260814-charge-never-detonates.md`, taken: the detonation is
gated in the sim on *shipped* content — `crates/server/tests/raid.rs`, twig
foundation at hp 10 against `structure` 125, the real 300-tick fuse. It went
**green first run**, so the verb was never broken. That clears that note's
suspect list: `detonate`'s scan, the `find_index` re-resolve, the overkill
case (`dealt` is clamped to the piece's 10, not the charge's 125), the fuse
length. Also read and cleared this pass: the wire encoder's `EV_STRUCT_HIT`
arm, and any early sweep of a live charge — the only writes to
`World::charges` are `place`, `tick_fuses` and the save restore.

**The arrangement is cleared too (2026-08-14).** The instrumented run this
item asked for is `crates/server/tests/raid_shape.rs` — the wire's seating,
walk, one-action-per-tick cadence and one-frame hotbar selection, replayed
into `World::tick` for 905 ticks with no socket. **It raids: 21 plants, 12
`EV_STRUCT_HIT`, first breach tick 355.** So both things this item suspected
are wrong. *Attacker and owner never share a plot* is true — measured as an
integer for the first time, `peak_shared_plot == 1` — and does not stop the
raid, because an attacker plants on the foundation it laid four steps earlier
and a blast is area-not-address. And the plants do not cluster late: first at
tick 55, 17 of 21 due inside. (The shared-plot gap is still real as *design* —
judge -17 §B.3 — but it is not the explanation, and the two were one thing.)

**What remains, ranked.** All wire-only, since the harness is the optimistic
case and still raids.
1. **Dropped actions skip, not retry** — `core::wants_action` takes one per
   client per tick; a lost step 4 leaves step 5 throwing at nothing.
2. The jitter buffer's held-item timing. Cannot be the whole story: 27
   charges did arm. Chain:
   `findings/note-20260814-the-arrangement-raids.md`.

**Struck 2026-08-14, do not re-run it** (judge -18 ranked fix 1, checked
here). This list's old #1 said 905 ticks "was `30 s × TICK_HZ`, not a
reading". `TICK_HZ` is 30 (`limits.rs:15`) and 30 × 30 is **900**, so 905
cannot be that product; 905/30 = 30.17, which is the "30.2 Hz" the note
quotes, i.e. the Hz was derived *from* 905 and 905 is the measurement. The
window held.

---

## 0rs · The bots raid on the wire — what a naked raider cannot reach *(systems lane)*

From `findings/pass-20260813-230343-{13,14}-judge.md` gap 1, *"a player has
no opponent"* — ranked 1 in both. Item 1 landed 2026-08-14.

Landed before: `bots::raid_step` + `test_raid_storm`, driven straight into
`World::tick`. Landed now, the wire half: `botclient.rs` derives its plot
from **its own body** (`build_cell_of`, re-seated every `RAID_CYCLE`, so a
walking bot is not stuck out of reach), feeds `raid_step`, and writes the
frame through the same `encode_action_*` the native client calls — the
server cannot tell a raiding bot from a player. `bin/bots` raids by default
(`walk` restores the old behaviour) with rows read from `content/` by id.
Gate: `test_bots_raid_over_the_wire`, proven red three ways — a constant
plot cell, a suppressed write, a `raid_step` verb with no encoder arm.
Measured, 8 raiders × 4 s: ~110 actions each, 12 plot re-seats, plots
scattered, 0 unencodable, 0 malformed server-side, and the sim answered
47–59 build + 23–48 deploy + 12–23 move refusals apiece.

Item 1 (the fleet could not afford to play) landed 2026-08-14: a fixture
raid kit — satchel / box / wood / lock, every index and count read out of
shipped content, at the slots `RaidRows` addresses. The owner cycle now
builds and locks for real. Measured, 8 × 4 s: ~66 pieces placed, 3 deploys,
18 charges armed, `auths` 0..15 per owner — all four flatly 0 before. Proven
red three ways: a naked fleet, a kit whose layout drifts from `charge_slot`,
a dropped `ChargePlaced` arm. `struct_hits` did **not** move; that is §0rc,
not this item. (Cited as `§0rf` until 2026-08-14 — a label this file has
never had, in all three places, judge -18 ranked fix 2.)

Remaining, ranked:

1. **Bodies are out of the storm** — the throwable's `damage` is 0, so the
   raid never kills and `MAX_BACKPACKS` plus the death/respawn ring are the
   one client-driven family it misses.
2. `CLAUDE.md` wall 4's ⚠ still says `test_raid_storm` does not exist; a
   loop may not edit the walls list, so striking it is an operator act.

---

## 0tq · The HUD says every fact of a frame, not the last one *(client lane)*

**Gap pass, from `findings/pass-20260813-230343-10-judge.md` ranked gap 2**
("the game speaks one sentence per frame and silently drops the rest").

Landed 2026-08-14: `hud::Toast` is a bounded queue — `TOAST_LINES = 4` rows,
newest at the top where the single line always sat, one clock each,
drop-oldest and counted. The same report's ranked fix 1 was the proof it
mattered on shipped content: a tree pays a secondary, so one swing into a
full pack says two spill lines and the single slot showed the mushrooms and
ate the wood. Five hud tests, each proven red under its own revert. Also
that report's ranked fix 2 (a positive control in `spill.rs`, so half one's
three negatives cannot go green on a swing that missed) and fix 3b (the
word "measured" in `DECISIONS.md`).

Landed 2026-08-14 (second slice, judge ranked fix 3 of pass -11): eviction
reads a `Rank` instead of a position. A line is an `Alarm` when the fact
dies with it — a refusal, a spill, a charge going live — and the cap eats
every recoverable `Note` before it touches one; only an all-alarm stack
falls back to oldest-outright, and a push is never refused. Drawing order
is still recency, so a frame under the cap is unchanged. 29 sites moved to
`warn`. `dropped` has a reader too: `unseen` counts the burst, clears when
the stack empties, and rides the last live row as a suffix (`…+2 more`) —
a suffix because the bottom row is where a rescued alarm now sits. Four
tests, each observed red under its own revert. Left:

Landed 2026-08-14 (third slice, judge ranked fixes 1–3 of pass -12): the
arithmetic half of "nobody has looked at it". The layout numbers were
literals inside `setup` — the two spawns computed one rule twice — and are
now named constants and four derived functions the spawns call, gated by
four tests, each observed red under its own revert. What they found: the
pitch is a **percent** of window height and the type size is **px**, so the
stack self-overlaps below a 600 px window against the 720 the client opens
at (read off `Window::default()`, not typed) — a 120 px margin nobody had
computed. Also that `TOAST_LINES` × `TOAST_ROW_DIM` multiply: at 8 rows the
deepest draws at alpha 0, so the cap would hold a line nothing can show.
Plus the two cheap fixes — a repeat now raises a line's rank and never
lowers it (the one write path that skipped the field eviction reads), and
the three places naming the old drop-oldest policy now name the shipped one.
Left:

- **Nobody has LOOKED at it**, and this is now the whole of what remains.
  Same as §0sp2's last bullet: no frame in this repo has ever shown one
  line, let alone four with a `…+N more` suffix. What the gates above
  cannot answer is whether 0.52 alpha on the deepest row reads, and whether
  the suffix — appended into a centre-justified row — shifts the sentence
  under the eye. Needs a capture with a forced five-fact stack; the probe
  has no way to force one, and a frame this loop scores itself is
  diagnosis, never evidence.

## 0sp2 · The spill speaks now — for the whole of one, not part of one *(systems lane)*

Landed 2026-08-14: six producers, one drain (`World::drain_spill`), and the
same day the signal, which was this item's own open half. **It was a
client-side read of facts already on the wire, and the answer is written
down now rather than guessed at** — the zero was always there
(`EV_CRAFT_DONE` has declared "0 = full inventory" since it landed) and the
client discarded it, gather on an `if added > 0`, craft by printing
`crafted 0 × Stone Hatchet`, which said a craft had failed that had in fact
succeeded and was on the floor. Cost: one guard in `gather::swing` so the
zero has exactly one cause (a swing the cumulative schedule owed nothing
produced the identical event), a ring in `client-core`, and the HUD line
*"pack full — Wood dropped at your feet"*. No wire change, no version bump,
no knob. Left, and the first two need a wire field:

- **A partial spill is still invisible.** Some fits, some does not, and the
  shortfall never leaves the sim — the wire carries what reached the hands
  and never what was paid, so `+3 × Wood` cannot say the other 7 fell.
- **The four give-backs say nothing at all** — demolish refund, pick-up,
  unbolt, craft cancel emit no payout event, spilled or not. Operator:
  those two together are what a wire field buys (`DECISIONS.md` §open).
- **The merge ignores ownership** — a spill lands in whatever bag is
  nearest, including someone else's death bag. §open carries it.
- **Nobody has seen one.** Judge gap 1 stands: proven headless only. The
  new line included — no frame in this repo has ever shown it.

## 0wc · The crate opens — what world containers v0 left *(systems lane)*

Gap-pass item, from the merge-gate judge's ranked gap 1
(`findings/pass-20260813-230343-04-judge.md`): the destination gradient was
fully built and gated and *paid nobody* — `loot.rs:33` said "No verb opens
one yet." Landed 2026-08-14: `CONT_WORLD` is a fourth container kind, the
open re-derives the cell through `terrain::scatter`, the refill is lazy
inside `open` so the store costs the tick nothing, and the crate rides the
existing move/refusal/sync path (`DECISIONS.md` §open "world containers
v0"; wire v37; save format 5; `tests/worldcont.rs`, 17 checks).

**The panel was wired to the wrong store, and it shipped green
(2026-08-14).** The server's per-tick container drip dispatched the kind as
`if kind == CONT_BAG { backpacks } else { deploys }` — true for the two
ground kinds alive when it was written, and silently false the day
`CONT_WORLD` landed: the crate's panel read `deploys.box_slot` with a
`world_conts` index. It cannot panic (64 world containers index safely into
1 024 deploys), so **opening the pad's crate drew an empty panel over four
units of loot**, with 17 sim checks and 86 protocol fixtures green over it.
Fixed by making `World::cont_slot` — which had all three arms — `pub` and
the drip's only answer; `container_wire.rs` gains
`a_world_crate_is_drawn_from_the_crate_store` (proven red under the old
dispatch: `left: []`) and a `CONT_MAX` compile guard so a fifth kind breaks
that file until someone covers it. Two stale protocol claims went with it
(the kind field saturated at v37; `kind > CONT_MAX` now refuses nothing).

Owed, in rank order:

1. **Nobody has opened one in the running game — and this pass is why that
   matters, not why it is settled.** A headless test found the defect
   above, but only because someone went looking; the reason to look was
   that no one had booted it. Still unverified with the client attached:
   the prompt string, the panel title, the drag out of a 30-slot grid, what
   an emptied crate looks like. The capture probe **cannot** substitute —
   `VANTAGES` (`render/capture.rs`) is yaw/pitch from the spawn eye with no
   position, so it can only ever photograph wherever the player already is.
   **Standing the probe at the pad is NOT unbuilt work** — this line said it
   was and the judge measured it false (pass -07, ranked fix 1):
   `shard.toml dev_spawn = "x,z"` is parsed (`config.rs:283`), carried into
   the world (`net.rs:1618`) and returned ahead of the spawn ring
   (`world.rs:1213`). Derive the crate's anchor the way `a_pad_crate` does,
   put it in `shard.toml`, boot. That is the cheapest route to the
   verification this item asks for.
2. **An emptied crate says nothing at a distance.** The only way to learn
   the pad is farmed out is to walk to it, which makes a wasted trip the
   normal case once a shard is populated. Wants either a visible lid
   state on the mesh or the refill window shortened; the mesh is
   `render/props.rs`, the knob is in §open.
3. ~~The prize is still unguarded~~ — **landed 2026-08-14** (site guards
   v0, `DECISIONS.md` §open; `sim-core/tests/guard.rs`, 13 checks). A guard
   is a wolf slot whose home is inside a site and whose leash is that
   site's `SiteFootprint::scatter_m` instead of its species' `roam_cm` —
   both pure in the slot ordinal, so no wire field and no client change.
   Two per site, 6 of the 16 wolves.
   Owed off it: **the guard has no loot tier of its own** — it drops a
   wolf's meat and fat, so the reason to fight it is the crates behind it.
   The tier wants a third *species* (its own `drops`), and that is a client
   change, not a content row: every species match in the client is a `_ =>`
   fall-through, so a third kind would draw and sound as a pig until five
   arms are written (`render/mobs.rs`, `sound/voice.rs`, `ui/death.rs`).
   Forcing it through `loot.toml` instead does not work — `validate`
   requires `hits > 0` ("swings to open"), which a mob has no meaning for.
4. **Nobody has fought a guard in the running game.** Same standing as
   item 1 and now more of the pad's story: the wolf that hatches at the
   crates has never been seen, heard or fought with the client attached.
5. **`inventory::slots_in` is the same defect shape one function over**
   (judge, pass -07, fix 2): `CONT_BOX => BOX_SLOTS, _ => INV_SLOTS`, and
   the drip takes the panel's width from it. Right today only because a
   world container happens to be `INV_SLOTS` wide; a fifth ground kind of a
   different width draws the wrong slot count silently, and
   `a_world_crate_is_drawn_from_the_crate_store` reads `0..INV_SLOTS` so it
   would not catch it. Wants an explicit arm under the same `CONT_MAX`
   compile guard `container_wire.rs` just gained.

## 0pr · The wolf hunts — what predator v0 left *(systems lane)*

Predator v0 landed 2026-08-14: the wolf is a content row, and **nothing in
`mob.rs` branches on species** — a hunter is `brave_pct = 0` plus a notice
radius. `DECISIONS.md` §open "predator v0" has the numbers, the sources and
the phase-locked-bite bug the stride exposed. (Trimmed to the bound
2026-08-14; the landing story is in that row, not here.)

**Item 3 landed 2026-08-14 — pointing the other way.** The sim reads the
clock: the wolf hunts **worse** after dusk (30 m → 15 m), because no game in
the survey publishes a night sense ratio above 1×. §open "nocturnal senses".

**Item 1 landed 2026-08-14.** `sound/pig.rs` is `sound/voice.rs` and reads the
species off the roster slot, so a wolf howls (88 m) and growls (14 m) instead
of snorting. The register switch is not a knob — it is `CUES[Growl].radius_m`
read back out. §open "wolf voice v0" has the sources, the four places the
research changed the design, and the three follow-ons it names.

Owed, in rank order:

1. **Nobody has heard any of it.** Every claim is arithmetic — ZCR, sustain
   ratio, cadence bands. `bin/soundbank.rs` dumps the bank to WAV; ears are
   the gate that has not run, and the knobs to listen for are the two
   cadences (75 s, 2.5 s), the 0.5× night sense, and **16 predators** — all
   four are arithmetic nobody has playtested.
2. **A wolf pays no hide and no bone** — refused in the roster slice because
   it drags recipes and `ui::icons::STEMS` in with it.
3. **Night still costs the player nothing.** Nocturnal senses made the hour a
   *tactic*; it did not make the dark dangerous. The sourced follow-on is
   **not** more tuning of `night_spook_m` — it is a night-only roster variant
   (Minecraft and Valheim both gate *spawns* on darkness). The judge's gap 1
   wanted a warmth stat, which is the bigger version of the same hole.

## 0sp · The tick has been profiled — where it goes *(server lane)*

`crates/server/src/bin/profile.rs` (new, 2026-08-11) builds the stated worst
case — `MAX_PLAYERS` in one AOI cell, roster alive, store filled, everyone
acking and swinging — and splits sim from netcode by ablation. **Not a gate
and must not become one**: it reports elapsed time. `valgrind
--tool=callgrind` gives the per-function ranking, the half this box repeats.

**It settles half of §0q item 4.** A full tick at 100 clients is ~0.8 ms of
33.3; the AOI scan is O(clients × (players + mobs)) and ~0.24 ms of it, so
**the linear scan needs no spatial structure** — the soak still owes jitter
and real bytes. `state_hash` is 85 µs one tick in 32 and `encode_world`
24 µs one in 1,800: `reference/SAVES.md` §4's freeze is not ours.

Landed with it, −28 % instructions: `movement::step`'s duplicate terrain
fan; the AOI rank sort → two selections (the single largest item, ~23 %),
gated by `snapshot_budget.rs`'s `the_rank_band_agrees_with_a_full_sort…`;
the encoder's quadratic baseline scan; a whole-field `BitWriter::write`; and
the one that was a **spike** — `gather::swing` read `terrain::scatter` cold
instead of through `SlotCache`, so a hundred aligned swing cooldowns cost
1.9 ms in one `World::tick`, now 0.28.

**The client half landed 2026-08-13.** `resolve_swing` reads through
`ClientCore`'s own `SlotCache` — the predictor's, warm with the cells this
frame's movement step just filled — via `ClientCore::island`. Counted, not
timed: 61 frames of crosshair on one node cost **9 `scatter` calls, not
549** (`SlotCache::resolves`, a memo statistic, never hashed). Four gates in
`tests/ui.rs`, three proven red under their own mutation; the fourth is a
call-site scan refusing a direct `scatter(` on this path
(`tls_callsite.rs`'s shape). `render/props.rs` is deliberately excluded —
64 *distinct* cells once per chunk is not a memo's case.

Open: the encoder is now the largest phase (~0.43 ms of the 0.83), and
`World::scatter_clear` still resolves cells cold per spawn pick. **It is
not the same three-line fix** — it is `&self`, and unlike the crosshair its
3×3 window *moves every candidate* along the spawn ring, so the cells are
distinct and a memo only pays across repeated picks. Measure a respawn
storm before threading `&mut self` through the picker.

---

## 0n1 · The class-S join walk has no interest filter *(server lane)*

`reference/NETWORK.md` §9.2.1, measured 2026-08-10. `pump_events` drips the
**entire** piece store to every client — `PIECE_SYNC_BATCH = 32` per tick,
no distance test anywhere (`server/src/core.rs:1872`). At `MAX_PIECES` that
is 256 ticks (8.5 s) to teach one joiner about every structure on the
island, near or far. This is the reference game's own 2014 mistake, which
they fixed by sending spawn-local entities instead.

The restart makes it worse and is the reachable half. A removal while a
client's cursor is inside the store resets it to zero (`core.rs:1663`) —
correct under the store's swap-remove, unbounded in cost. A 3,000-piece
base walks in ~94 ticks and a raid removes pieces faster than that, so a
client joining mid-raid can be walked back to zero indefinitely and never
finish. `ev_resync` compounds it: a full event ring zeroes **every** walk
cursor at once (`client.rs:249`), and the resend it triggers refills the
ring that triggered it.

Landed this pass: `piece_walk_restarts` counts the restart, so the
livelock is visible before it is fixed. Not landed: the filter. The fix is
`NETCODE.md` §7's chunk subscription — one spatial truth for both classes,
which the doc already specifies and the tree has never had — and it wants
`test_stream_in` and §11's `test_raid_storm` with it (§11 there: **all
seven of its gates are unbuilt**, retitled to stop claiming otherwise —
and note the name is now shared, see §0rs: the sim-core storm that landed
is wall 4's caps gate, not §11's wire one).

---

## 0n2 · Monuments — the solver is two hand-written tiers *(world lane)*

Research landed 2026-08-10: `reference/MONUMENTS.md` (operator briefing —
**§0 says its provenance is the weakest here**, so read §9, not §1–§8, before
building). §9.1 is what we already got right and must not relitigate; §9.2 is
built this pass (`SiteFootprint` / `site_sweep` — a site publishes masks, not
a radius; clutter no longer grows across the pad).

Landed since: §9.3a (the drawn structure is derived from the sim's box table
— one list, so the mirror cannot drift again — plus `tests/greybox.rs` over
**every** archetype), §9.3b (the world file refuses an island that moved under
the same seed), and the debug/release probe diff that closes float contraction
on the one axis this box can reach. **All of it looked at** (§0p3 has the
recipe); two art rows fell out and are in `DECISIONS.md` §open — the shelter's
corner posts stand 1.2 m proud of its roof and read as stubs, and swept ground
reads as scattered shards at 2 m because of the pebble mesh. **The collision
skirt is closed** (operator, 2026-08-10): every occupant blocks what it draws
now, within a millimetre, and the gate holds it there.

**Deploy collision landed 2026-08-11** (deploy collision v0, `DECISIONS.md`
§open): six archetypes block at the client's own authored volumes, tops are
ground, and `tests/greybox.rs` §D holds the sim and drawn tables equal.
Residue, one line each: arrows still pass through every deployable
(`ranged.rs` never asks the solid nibbles — same class as its piece gap),
and whether a sleeper blocks stays unanswered (§0y item 1, untouched).

§9.3 is the gap and it is not urgent yet: `haven()` + `pick_minor` produce two
kinds of site, the separation floor is one hand-asserted constant
(`WAYSTATION_MIN_SEP_M`), and there is no reservation ledger. That is correct
at two tiers and is §1's starvation shape at five. **The trigger to fix it is
a third destination kind, not a spare pass** — the per-tier check chains stay
separate by a call the code already records.

Ranked after that, all from §9.4: class S still has no interest filter at all
(§0n1 — and a monument is the worst place to discover it), per-entity interest
ranges, then nav the day something defends a site. Vertical AOI layers are
premature (no underground) and moving monuments are refused on the record.

---

## 0p3 · You can photograph any authored site, and it is a config line *(client lane)*

Found 2026-08-10 while checking the greybox fix by eye. §0p2 item 3 asks for a
**viewer** for the screens nothing can photograph; four fifths of one already
exists and nobody had connected the pieces. The capture harness stands its
camera at the player's spawn, and `shard.toml`'s `dev_spawn = "x,z"` puts that
spawn anywhere. So:

1. `terrain::haven(seed)` / `haven_shelter` / `waystation_canopy` give the
   coordinates; stand 15–35 m off on the bearing you want in frame.
2. `Xvfb :9 -screen 0 1280x720x24 &`, then the shard, then
   `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:9
   WGPU_BACKEND=vulkan target/release/gates --server 127.0.0.1:4433
   --capture <dir>` — six vantages, ~40 s.
3. The vantages face N/E/S/W, so place the camera on the opposite side of
   what you want to see. Two of four attempts missed for this reason.

**This asserts nothing and must not become a gate** (`CLAUDE.md`: the visual
gate is a person, and `vantages.mjs` is why). What it changes is the cost of
looking, which was "boot the game on a machine with a GPU" and is now a
command on this box. Still owed from §0p2 item 3: the panels, which need the
camera pointed at a screen rather than at a place.

## 0pf · The client's CPU frame — two paid for, four measured and open *(client lane)*

Landed 2026-08-11. `water::animate` resolved `wave_field` **three times a
vertex** — once inside each `attribute_mut` borrow, throwing two answers away
— and now resolves it once into `Sea::field`: **1.01 → 0.38 ms, every frame**,
the largest steady-state cost the client had. `terrain_mesh::heightfield` —
one chunk a frame, so its cost IS a dropped frame — went **28 → 5.4 ms** (the
257² far mesh 485 → 186) on two halves: nine `terrain::height` taps a vertex
became three where adjacent vertices were already sampling each other's
points (bit-identical, three shares each checked at the origin rather than
assumed), and the tangent is now written in closed form instead of solved by
mikktspace, which was 12 ms of the remaining 17. Gated by `tests/ground.rs`
(4, four mutants run red) and `tests/water.rs` (28). CPU-only, release, this
box; no GPU has ever run this client, so `§0u` is still the other half.

Found on the way: **`ci/gates.sh` named its renderer-tier suites one at a
time and had never named two of them**, so `tests/water.rs` (skipped by
`required-features`) and `tests/greybox.rs` (built empty without the feature,
passing on zero tests) had never once run under it. The enumeration is gone —
every test target the crate has now runs.

Remaining, in the order the measurement ranks them:

1. **`clutter_fill` + `skirt_fill` is 2.8 ms a tile**, one tile a frame —
   now the largest cost on a streaming frame. It is `sim-core`, so wall 1
   binds and the fix is not a memo. Its `Soup` is also freshly allocated per
   tile; a `Local` would reuse it.
2. **`water::stream` is 2.2 ms every 8 m walked** (5,929 `terrain::height`
   taps on the `SNAP_M` crossing). The sea's axis is non-uniform, so there is
   no half-lattice left to share — off-thread or coarser, not cleverer.
3. **The far mesh is one ~180 ms frame during `Loading`**, with the session
   pump inside it. `heightfield` is pure and touches no ECS, so
   `AsyncComputeTaskPool` is the shape; that would also retire item 1's
   frame cost without touching `sim-core`.
4. **`terrain::height` is 502 ns** — ~12 `noise2`, four hashes each — and
   every number above is a count of it. Nearby vertices re-hash the same
   lattice corners at the low octaves; a memo is sim-core's to refuse.
5. **The sea's tangent `w` and mikktspace's disagree.** `water.rs` writes
   `-1` for a planar XZ UV set; mikktspace answers `+1` for the identical
   parameterisation on the ground (now asserted, `tests/ground.rs`). One of
   them flips the ripple map's green channel. Which is right is a question
   about how that map was authored — boot the game and look, do not guess.

## 0bd · The barrel is measured and the sim still blocks the guess *(client+sim lane)*

The drawn barrel and the blocked barrel are one number in two files and they
disagree with the real object. `OCCUPANT_R_M[BarrelSlot]` is **0.45** and its
comment cites `CylinderGeometry(0.45, 0.45, 0.95)` — the deleted browser
client's geometry, i.e. a guess carried forward. A 55-gallon drum is **0.585 m
across by 0.88 tall** (1.5x taller than wide); 0.9 x 0.95 is near-spherical and
~44% too fat. Two independent sources agree: the reference set’s `barrelroad`, and
Meshy's `auto_size` vision estimate landing on 0.880 x 0.585 unprompted
(2026-08-11).

**Why it is not already done.** It was drawn narrow on a branch, and the merge
with origin fired `greybox.rs`'s
`every_drawn_archetype_fits_the_volume_the_sim_blocks`: the sim blocked 0.1575 m
wider than the client drew, past `SLACK_R_M`. That assert is right — an
invisible collision skirt is a player passing through geometry — and it means
the mesh cannot move alone.

**The slice**: narrow `OCCUPANT_R_M` and the `(0.45, 0.975)` pair in
`terrain.rs`, and `archetype_mesh`'s cylinder, in one commit. It is a
**collision change**, so `test_replay` and `test_terrain_golden` move with it
and the goldens are regenerated deliberately in the same commit (wall 5/6).
Check the other occupants for the same browser-geometry citation while there.

## 0b · Balance sits on the reference's numbers now — what is still off *(content lane)*

Landed 2026-08-08 (operator: *"balance the game similar to rust so people
dont get too lost"*). `reference/BALANCE.md` is the research and §6 is the
standing instruction. Building blocks are 250/500/1000, a stone wall takes
four satchels, tool and melee damage are theirs, the pig is a 150-hp boar.
Two bands moved and the raid ratio re-priced itself. ⚠ **The three numbers
that used to sit here — 1.04/1.73/3.46 — were 2026-08-08's and were stale
by two days**; measured 2026-08-10 the tree read 0.69/1.38/2.77 before that
day's building work and **0.76/1.52/3.04** after it. Derive it (the probe
is five lines against `balance::check`), never quote it.

**The measurement landed 2026-08-09 and `reference/RIPLIST.md` is now the
queue for this item** — what is taken, what is outstanding, what blocks
each row, and the six steps for executing one. Read it before touching a
balance number; do not re-derive that list here.

⚠ **Two rules changed on 2026-08-10 and both are operator-spoken.**
(a) *"lighten our own math and lean on them for now"* — a band of ours
yields to a number of theirs by default (`BALANCE.md` §6.5); re-speak it
rather than treating it as evidence. (b) A number **absent** from
`RIPLIST.md` has not been decided either: asking that question found six
of twelve content files with zero coverage.

**Rows 1b, 1c and 1d all landed the same day** — building costs, the
craft column and deployable hp, `RIPLIST.md` §1c is the record. What is
left of that thread, in order:

1. **Row 1e**: `items.toml` stack sizes ✅ taken 2026-08-11 at tier 3
   (`RIPLIST.md` §1e: 5 cells moved — ammo 128, arrows 64, bandage 3,
   gunpowder 1000 — 9 confirmed matched, 12 left open with the reason
   named; the spawn kit's bandages went 5 → 3 as forced fallout).
   `armor` · `cooking` · `loot` · `research` still have zero coverage.
2. ⚠ **The source tier dropped to get 1c/1d**: every candidate page is
   `EGRESS_BLOCKED` here, so the table came through a second assistant —
   a summary of pages nobody in this loop read. Re-verify if egress opens.
   **Re-probed 2026-08-11: closed harder** (fetches blocked for every
   host, search summaries only — §1e says so), so the re-verify stays
   owed and a browser is still the only route.

Closed 2026-08-11 by the operator, all three: the rock **is** craftable
(15 → 10 stone, and the tier-4 source beat my prior — §1c says so), OBOL
**is** scrap so the research table takes its 20 (my refusal answered
itself), and the cupboard is **stronger** here on purpose — hp 500 →
1,000, the metal rung, so taking a base's privilege costs one more wall.
Nothing about the cupboard is outstanding now; it is a chosen difference.

Two results worth carrying at this level. `balance.rs` refuses a
`farm_per_min` above the sim's at-node ceiling and `tests/farmwalk.rs`
measures **969 wood/min, 71.6% duty**, ~19× the declared 50. But that
gap is a **debt owed by the world, not an error in the number**: their
ladder falls ~30× from at-node to real farming with no threat in it at
all, ours charges 1.40×, and applying their decomposition to our ceiling
puts the declared 50 inside the band. So the queue's ranking inverted —
**logistics friction (~10–30×) outranks mob→player damage (~2–5×)**, and
threat wants modelling as trip shape, never as a rate multiplier.

**Two gather mechanics are theirs as of 2026-08-09** (operator: the mark
must buy speed, not yield, and we need the finish bonus). A node's payout
is invariant at `hits × per-hit`; the glint spends its budget faster (a
tree falls in 7 swings instead of 10 for the same 300), and 20% of an ore
node / 50% of a tree is withheld for whoever lands the last swing. Gated
in `sim-core tests/gather.rs`; no wire byte moved.

Still wrong for a returning player, in rank order, all of it detailed in
`RIPLIST.md` §2: no per-material damage resistance (one `structure`
column, so the ladder above stone is compressed); and gather yields, smelt
and craft times are still ours — node totals are `READY` now, per-hit
yields are not, and our schema does not need them. Upkeep, decay and the
armour ladder differ on purpose (`BALANCE.md` §4.1), though the upkeep
*rate* turned out to match theirs. (Struck 2026-08-14: "the boar does not
fight back" and "one animal" were falsified by `mob attack v0` and the
wolf and had been left standing — the last judge's ranked fix 1.)

---

## 0rl · The release path ran, on all three platforms *(platform lane)*

`.github/workflows/release.yml` builds the client and shard for Linux,
Windows and macOS on a `v*` tag, re-runs the gates on the tagged commit,
refuses a tag that disagrees with `[workspace.package] version`, and drafts
the release.

1. ~~macOS has never been compiled, Windows only typechecked~~ — **retired
   2026-08-11, by the tag.** `v0.1.0`'s release run is green in all six
   jobs, including `build (macos-latest, aarch64-apple-darwin)` and `build
   (windows-latest, x86_64-pc-windows-msvc)`. So msvc linking and the Apple
   toolchain are no longer written-and-unproven: they compiled, linked,
   staged and archived on real runners. **The three artifacts have still
   never been RUN** — nothing here has a Mac or a Windows box to start one
   on — so the honest state moved from "does it build" to "does it launch",
   which is a tester's question and not CI's.
2. ~~No `LICENSE` file~~ — **done 2026-08-11** (MIT, © MoreRight DAO;
   `DECISIONS.md`). `LICENSE` + `NOTICE` ship in both the release archive and
   the scry depot, gated by `ci/depot.py --self-test`.
3. **The draft is drafted and nobody has published it.** That is the one
   operator act left on the release itself: open it, read what is attached,
   publish. Until then the tag exists and the download does not.
4. **`min_client` has never been raised on a live shard**, and the public
   shard now running prints `admits clients of any release`. The order is
   publish the release FIRST and raise the floor after; `refused_build`
   climbing days later is how you find out you did it backwards.

## 0ab · The store seam — what the SDK re-vendor and the depot job left *(platform lane)*

Landed 2026-08-09. The vendored SDK was **326 lines behind upstream** with
every gate in both repos green: no Windows transport (`std::os::unix::net`
was imported unconditionally, so a Windows build of this client could not
compile), no `prove`, no `profile`. Re-vendored and re-pinned; upstream now
publishes `sdk/SHA256SUMS` and gates its own rustfmt-cleanliness, so the drift
check is one `sha256sum`. `nightly.yml` now builds `--features render` and
runs `ci/depot.py`, so the depot is a CI artifact instead of one box's output.

What remains, in order:

1. **Nothing here publishes.** A build goes live when the origin's
   `published.json` names it and the digest is notarized — operator acts,
   both. The nightly artifact is the tree those acts consume, and it
   **exists**: `gates-depot-<run>` off nightly run 31475002978, 34 MB, live.
   ⚠ Two things checked 2026-08-11 and worth knowing before you reach for
   it. The `depot` job in that run passed while the run reads **failure** —
   the `nightly` job failed at "gates first", at 08:50Z, which is before the
   toolchain pin merged that evening, so it is the same red every `gates`
   run had that day and should be green on the next fire. And **neither
   publish act can happen from this box**: `scry.moreright.xyz` is a
   different host (Cloudflare-fronted, not 5.161.193.186) and there is no
   `scry` binary on PATH, so `scry digest` — the one implementation of the
   number that gets notarized — is not runnable here by construction.
2. **The shard list is written, generated, and not yet served.**
   `shards.toml` exists now and `./ci/shardlist.py` writes the one-row
   document; `manifest.servers.url` on scry's side is still `null`, so the
   launcher's Servers window and our own menu stay dark for the same missing
   file. scry's serving half is confirmed live rather than assumed —
   `GET /api/launcher/servers/gates` answers **404**, its documented
   "publishes none", not the 503 it reserves for "could not look".
   Everything downstream of that one publish exists: live counts via
   `status_url` (answering now), and join links
   (`scry://join/gates/host:port`, `deeplink.rs`). Registering the scheme
   with the desktop is the launcher's installer, and is not done.
3. **`prove` has no call site** — and this is now the *only* thing left in
   the identity seam, because the ticket door landed on the handshake we
   already have (2026-08-11, `entitle.rs`). The address is proved today:
   the shard picks the nonce AND the `issued_at`, the client composes the
   message through the one shared `protocol::siwe_message`, and the server
   rebuilds identical bytes and `ecrecover`s. That is sound, and it is why
   entitlement needed no wire change.
   What `prove` buys is the *consent prompt*: `sign_siwe` makes the launcher
   sign a string this process composed, so the player clicks through a dialog
   on every join; `Overlay::prove` has the launcher compose it, which fires no
   prompt by construction. **The cost is real and is why this is still open:**
   the launcher writes its own `Issued At`, so the server can no longer
   rebuild the bytes and must PARSE an EIP-4361 message instead — and the wire
   has to carry that message, which IS a layout change (wall 6: version bump +
   goldens in the same commit). Worth doing for the prompt alone; it is a
   slice, not a line.
4. **The depot is Linux only.** `ci/depot.py` says so in its first line and
   scry's platform enum has the other rows. The SDK can now reach a launcher
   on Windows; nothing packages a Windows build of this game.
5. **The public shard is up and no one has ever joined it** (2026-08-11).
   Boot, persistence, the SIGTERM flush and the status endpoint are all
   measured; the join is not, and **the tools here cannot measure it**:
   `bots` takes a `SocketAddr`, so it cannot dial `game.moreright.xyz` by
   name at all — which is the half that matters, because the certificate is
   issued for the name and the client validates against the platform root
   store on a non-loopback address (`tls_posture.rs`) — and it carries no
   wallet, so `require_auth = true` refuses it correctly. The first real
   join is a person with the published build, which is why it sits behind
   §0rl item 3.

---

## 0ad · The ticket door is armed but nobody has sold a copy *(platform lane)*

Landed 2026-08-11 (`crates/server/src/entitle.rs`). A shard with
`entitle_origin` set asks scry `GET /api/ticket/gates/of/<wallet>` at join
and `POST …/check` for the whole roster every `entitle_sweep_secs`, refusing
with `REFUSE_TICKET` and kicking on a **definite on-chain zero only** — a
failed read admits and bumps `entitle_unknown`, because an RPC outage that
booted every paying player is worse than the freeloader it catches. Unset is
the default and checks nothing, which is what every test and every community
shard runs (`DECISIONS.md` 2026-08-04: one build, two populations).

What is left, in order:

1. **Nothing has been driven against a real ticket contract**, because
   `ScryGameTicket:GATES` is not deployed — scry's `deployments.json` has no
   address, so `/of/<wallet>` answers `ticketed: false, entitled: true` for
   everyone and the door is a pass-through by design. Every branch is unit-
   tested against the response shapes scry actually serves (`tickets.py`),
   and none has met the live route. **First real check is the day the
   contract is deployed**, and the honest way to run it is one wallet that
   owns a copy and one that does not.
2. **The sweep interval is unspoken.** 120 s is a documented default, not an
   operator sentence, and it is the whole security property — how long a sold
   copy keeps playing. `DECISIONS.md` §open carries the row.
3. **No `prove`**, so a join still costs the player a consent dialog — §0ab
   item 3 has what that slice actually needs.

---

## 0ac · The catalogue — what twig and the cost grammar left *(systems lane)*

Landed 2026-08-10 (operator: *"we need to work on building more"*).
`reference/BUILDING.md` §7b is the research, `DECISIONS.md` §open "twig
v0" the slice: placement is twig-only and the hammer commits it, twig is
never upkept, and **the whole cost column is theirs** — 24 cells, their
grade base and their shape ratios (`RIPLIST.md` row 1b, which did not
exist until it was taken: our costs had never been compared to theirs, and
the node take is what exposed it). §9 items 11 and 12 are done; 13, 14 and
15 are not, in cost order:

1. **The window and the wall frame** (§9.13). Openings are already sockets
   here — a doorway takes a door with its own hp and its own lock — and
   these are the same idea with the insert unbuilt. `SHAPE_BITS` is 3 and
   6 of its 8 codes are used, so **two shapes fit with no wire widening**,
   and §7b.3 has already decided their prices (0.7 and 0.5). The window
   wants a collision answer first: it blocks a body and not a bullet,
   which no shape here does yet.
2. **Hard and soft sides** (§9.15, §7b.5). One rule that turns placement
   *orientation* into skill, and the reason a base can be weaker than its
   bill of materials. Needs a facing on every piece and an attack
   direction on every swing — its own pass, and it pairs with
   `RIPLIST.md` §2's per-material resistance rather than competing.
3. **Triangles** (§9.14). Half the reason their bases look like that, and
   the only item here that is a **grid change**: our cell holds one plane,
   one riser and two canonical edges, all square. Cost it as one; do not
   smuggle it in behind items 1 and 2.

---

## 0aa · Building rights — what the four slices left standing *(systems lane)*

Landed 2026-08-08/09. `reference/BUILDING.md` is the research; the rows in
`DECISIONS.md` §open (hearth crew v1, privilege v1 + the claim cache,
demolish v1, upkeep/decay v1) are what was built — coverage asks the base's
own cached volume now, not a circle. What remains:

1. **No `AutoTurret`, so the roster has two customers and not three.**
   `roster.rs` exists because the reference has four; ours has two.

---

## 0z · Doors and locks — settled *(systems lane)*

Landed whole, 2026-08-08/09: `reference/DOORS.md` is the research,
`sim-core/lock.rs` the answer, `DECISIONS.md` §open "lock v1" the slice.
Locks on boxes, the pickup tier (a GUEST works the leaf and cannot lift),
and the keypad panel all followed. Nothing remains here.

**Not owed, and stated so it is not re-litigated**: the key lock (its keys
need per-item instance data `ItemStack` has no room for, and it is the
system the reference abandoned in Devblog 193) and door tiers past wood and
metal (a content row, not a mechanic).

---

## 0y · The sea is a volume — what it still cannot do *(client lane)*

Landed 2026-08-08: `render/water.rs` (eye-centred mesh, four-wave swell with
analytic normals, per-channel optics, shore foam standing off the waterline),
`terrain_mesh::wetted`, and `sound/water.rs`. Research `reference/WATER.md`;
knobs `DECISIONS.md` §open "water v0" / "water audio v0" (the §open row also
holds the five defects found by LOOKING, not by a gate). Gated by
`tests/water.rs` (28) and eight assertions in `tests/sound.rs` — and the
water suite only started running in CI on 2026-08-11 (§0pf). Remaining:

1. **The last hard edge needs the depth prepass, and that is the next
   slice.** The alpha ramp is a *vertex* quantity off `terrain::height`, so
   it fades against the terrain and not against anything else — a boulder, a
   foundation or a player in the shallows gets a ring. The fix is standard:
   sample the depth prepass in the fragment, fade alpha and add foam as
   scene depth approaches the water's own. Needs an `ExtendedMaterial` and
   the **first WGSL in the tree** (`RENDER.md` §8); SSAO already puts a
   depth prepass on the camera, so the input exists.
2. **There is one sea state and no weather.** A storm is `WAVES` scaled by a
   scalar the sim would have to publish — wire, not renderer.
3. **Nothing reflects.** `reference/WATER.md` §5 says reflections are the
   expensive half and §6 says the payoff is the *sky* — which the
   atmosphere's specular already gives. Read both before starting.
4. **Underwater is audio-only.** A colour grade under the surface is a
   second owner of the frame's haze (`CLAUDE.md`'s coupled-lighting law); it
   wants the lighting owner, not this lane.
5. **The submerged duck is not a filter.** rodio gives gain, rate and
   panning; a real low-pass needs a DSP node. Stated in `SNAPSHOTS`.
6. **`Splash` is the only producer of the waterline.** No stroke, no wake,
   no interactive deformation — the reference merges an interactive sim into
   its own displacement (§3) and we have no producer for one.

## 0m · The pig is in — what the roster still owes *(systems lane)*

Landed 2026-08-08/09 (operator: *"let's get a pig in"*): 64 fixed roster
slots, homes from the seed, staggered think, dormancy at 240 m, a leash, a
flight, a corpse bag looted with E, a hashed snort, a distance-integrated
trot — and the kill→fire→meal loop closed with the oven (§0v below): four
content rows, gated by
`content.rs::the_kill_the_fire_and_the_meal_are_one_loop`. No navmesh: the
terrain is a pure function, so the animal steers and `movement::step`
decides. Research `reference/ANIMALS.md`; calls `DECISIONS.md` 2026-08-08
and §open ("pig voice v0", "pig gait v0"). Owed, in rank order — §9.5 has
the reasoning:

**Three defects were found by booting the game and looking, and every gate
was green through all of them** — which is what `CLAUDE.md`'s "the operator
boots the game and looks" is for. (1) `flee_pct = 100` made the pig run at
exactly the player's sprint, so it could never be caught or melee'd; now 70,
and `tests/content.rs` gates `flee_gait < 127`. (2) The massing wore
`props::tint1`, a **mean-1** modulation meant for a photograph, and rendered
near-white on an untextured material; `boxes_mesh_with` splits the two and
`tests/mob_mesh.rs` gates the mean. (3) `bodies.rs` drew a humanoid rig at
every pig's position as well, because its only filter was "not me".

**§0v below and this item closed each other** (operator: *"go ahead and
finish"*). The oven shipped cooking nothing because nothing on the island
was raw; the pig is the first thing that is, and raw meat is the only item
in the set you cannot eat — which is what gives the fire a job.

**Making the sim actually do it (`server/tests/hunt.rs`) found a hole, not
a tuning problem.** The kit had **no weapon in it**: `weapons.toml` armed
six things and no tool was one, so `held_melee` was `None` for every pocket
a fresh character owns and a hatchet could not hurt a pig, a player or a
door. Five content rows fixed it (`DECISIONS.md` 2026-08-08) and the hunt
now runs **10.1 s** from a 12 m start with the kit's own stone hatchet. The
test also reddens with the right message when `flee_pct` goes back to 100,
so yesterday's capture-found defect is gated rather than remembered.

Left open by that: whether `ttk_melee` should widen so a rock is
meaningfully worse than a crafted spear rather than one hit worse. A band
is a knob — `DECISIONS.md` §open, "tools as weapons".

1. **A butchering VERB** — the reference's actual interaction, a tool-gated
   harvest on the body. Its landing place exists now: the corpse bag
   (`mob::strike` → `backpack::stand_up`) is the verb's output.
2. ~~Nothing fights back~~ — **done 2026-08-11** (mob attack v0,
   `DECISIONS.md` §open: the widening landed as wire v36, the pig charges
   whole and flees hurt, `DEATH_BY_MOB` names the corpse). Residue: the
   combat-feel half is minimal — the victim sees hp drop and hears nothing
   pig-specific; an aggro snort cue and a damage-direction tick are audio/
   HUD follow-ups, and the charge costs the pig nothing to hold.
3. **The massing is boxy up close** — at 8 m the head barely separates from
   the body (captured 2026-08-08). Massing detail, not animation; the legs
   already trot.
4. **`MAX_MOBS = 64` has never met a playtest.** It is derived (the wire
   budget) rather than felt, and it is the one number a player answers.

---
## 0v · The fire cooks now — what it left open *(systems lane)*

Landed 2026-08-08/09: the oven (`sim-core/oven.rs`, `DECISIONS.md` §open
"oven v0"), the meat loop (§0m above), and the burnt state
(`item.burnt_meat`, gated by `content.rs::the_meal_left_on_the_fire_burns`).
Still open, and deliberately: the furnace's ore rows are station-gated
crafts in `recipes.toml`. Moving them into the oven is the reference's model
(`BaseOven`) and re-prices the whole powder chain against `CONTENT.md` §4's
bands — a balance pass with an operator's number on it, not a refactor.

## 0u · The ghost tells the truth — what it still cannot promise *(client lane)*

Landed 2026-08-07/09: the doorway ghost is three parts off
`structures::shape_parts` (the one table the piece and the ghost both emit
from), the deploy ghost mirrors the sim's own verdict while AIMING, and a
door aims an EDGE and is placeable at all. Gated in `tests/ghost.rs` against
the sim's own predicates. Remaining:

1. **Stairs are still a flat slab** in both the ghost and the piece — a ramp
   drawn as a plate. Shared, so at least they agree.
2. **A lock aimed at a DOOR is unreachable** — locks still target the plane
   (on a box the L verb works). Noted at `place::deploy_target`, not built.

## 0v · Players are people — what the rig still cannot say *(client lane)*

Landed 2026-08-07: remote bodies are a skinned mannequin (CC0, 46 clips,
`assets/models/MANIFEST.md`) with gait chosen from derived speed, facing the
wire's `yaw`, plus a held tool with bob/sway/swing (`render/viewmodel.rs`).
Remaining, ranked:

1. **Crouch, jump and swim are wired to nothing.** The clips are in the file
   and the WIRE does not carry the facts — no grounded bit, no crouch bit — so
   `BodyAnim` cannot see them. This is a protocol change (wall 6: version bump
   + regenerated goldens in the same commit), not a client one.
2. **No attack, gather or death animation on a remote.** `Feed` carries the
   LOCAL player's hits only; a remote's swing is not a fact the client is told
   about. `EV_*` has the events — this needs the draw path to read them per
   body, which is the same gap `RENDER.md` §8 item 4 names for pieces.
3. **Nobody holds anything.** The viewmodel is first-person only; a remote
   mannequin has empty hands. The rig has hand joints, so this is an attachment
   to a named joint rather than new art.
4. **Root motion is ignored.** `Jog_Fwd_Loop` translates in place here because
   position is the interpolator's; the `_RM` variants are deliberately unused.
   Feet will slide at speeds between the clips' authored ones — the fix is
   scaling playback rate to speed, which is a knob nobody has measured.
5. **A plain worn-steel albedo is the missing texture.** The axe head carries
   no map because the only metal in `assets/` is ribbed corrugated sheet
   (`viewmodel.rs` and `assets/textures/MANIFEST.md` both record it).

## 0w · The props carry a photograph — what is left after it *(client lane)*

Landed 2026-08-07: 34 CC0 textures shipped, `props::Soup` box-projects per
triangle (free on a soup — no shared vertices, no seam), `blob_mesh`
subdivides and displaces, bark/wood/stone/metal/rock are bound. Licence rail
widened the same day (`DECISIONS.md` 2026-08-07). Remaining, ranked by what
the captures show:

1. **The hemisphere fill, and it is now the top visual gap.** p10 71.0 against
   a reference 41.0 — props v1 moved it 13 the wrong way by removing the
   frame's accidental darks (`RENDER.md` §0). One owner, one iteration, inside
   the coupled lighting set; do not touch it from a parallel lane.
2. **Trees are small and sparse in the midground.** The wide vantages are an
   empty green plain between the near clutter and the far ridge, where the
   reference frames are dense. This is `terrain::scatter`'s density and the
   conifer's scale, not a material.
3. **Nothing sits IN the ground** (`ART.md` rule 2). The new boulder has a
   clean elliptical intersection with the turf and no crowding or dirt skirt.
4. **The far mesh speckles.** Grazing-angle aliasing on the 8 m LOD; the
   candidate is anisotropy, registered at 4 for a browser reason that does not
   survive the port (`ART.md` §7), so it is a proposal not an edit.
5. **Roughness maps are still unread** — all nine of them. Blocked on an ORM
   packing step, not on a slot: `metallic_roughness_texture` is glTF-packed and
   its B channel is metallic, so a greyscale rough jpg would make every surface
   a half-metal.

## 0p2 · What the UI still owes *(client lane)*

The palette, the vitals bars, the icons, the baked wheel, the hammer wheel
and the typeface all landed (2026-08-07/09; `DECISIONS.md` "ui palette v1",
"ui type v0", §open `CELL_LINE_CHARS`; gates `tests/ui.rs` §F–§K). Left:

1. **Rotate is still not a verb, and a piece has no facing to turn.** A
   placed piece is `{cx, cz, level, loc, row, hp, uh}` — rotate waits on an
   asymmetry worth turning, not on lane room (`ACTION_SUB_BITS` is 5 since
   v30; the lane holds 32).
2. **The centre readout names the verb, not the target or the upgrade's
   cost.** The wedges are glyphs now (2026-08-09, `ui::hammer::verb_icon`,
   gated with the shape wheel's in `tests/ui.rs` §G); what is still text-only
   is the middle, and filling it wants `verbs::Near` at draw time, which
   `panels::rebuild` does not hold.
3. **Nothing in this repo can photograph a panel.** `render/panels/` is not
   registered on a `--capture` run, so inventory, crafting and the wheel —
   ~1,400 lines and the screens a player spends the most time in — are seen
   only by a human with a shard up. Wanted: a **viewer, not a gate** — a
   mode that opens each panel against a stocked fixture and writes a PNG per
   screen. The visual-gate rule is retired and stays retired (`CLAUDE.md`);
   this asserts nothing.
4. **Twelve sizes is not a scale.** Collapsing to five is a real improvement
   and may not be done blind: the numbers were budgeted against 720p and the
   first cut clipped a column at both ends.
5. **Surveyed and refused: `bevy_hui`, `bevy_lunex`, `bevy_feathers`.**
   Taking `bevy_hui` would move ~5,400 lines of screen description into a
   plugin that spawns entities from data — the same reason
   `bevy_procedural_tree`'s plugin is deliberately unused. The iteration win
   it was wanted for is item 3's, and item 3 costs a fraction as much.
6. **Surveyed and refused: the freegameui.net asset MCP** (~2,100 CC0 UI
   SVGs, 2026-08-09). CC0 is welcome (`ART.md` §7) and the licence was never
   the objection: its gateway 403s from this box while `raw.githubusercontent
   .com` answers 200, so it cannot serve the loop; a tool that writes files
   into `assets/` bypasses `bake_icons.py` and §G, which are what make an
   icon re-bakeable; and pre-coloured button/gauge kits fight the
   tint-at-draw design the whole icon path is built on. A second source in
   the baker is the cheap version and is what item 2's glyphs used.

## 0y · Persistence takes the reference game's shape *(server lane)*

Landed 2026-08-07/09 and ARMED on the public shard: SIWE identity, sleepers,
the world file (temp-then-rename, identity table, backup rotation), graceful
shutdown, and two-phase eviction. Plan and sources: `reference/SAVES.md` §9;
knobs: `DECISIONS.md` §open "player persistence v0". What remains:

1. **A sleeper does not block movement** — players never collided, so
   sleeping changed nothing; the question is unanswered rather than decided.
   Lootable-alive is still item 1 of whatever comes after (Devblog 7 shipped
   it after standing too).
2. **The same-window rejoin.** A victim reconnecting in the very window that
   evicts them gets the store record fetched *before* the eviction save is
   filed — one window wide, the save ring's freshness class; the takeover
   hint already refuses to wake a condemned body.
3. **Blueprints** are the wipe-surviving payload the store split was shaped
   for; nothing to build until BPs exist.
4. **Still no WAL, and the world file answered what a WAL would have
   forced**: a world load is an *origin*, not a command — the WAL header
   pins the origin hash beside the seed and the content hash and replay
   starts there. `worldsave.rs`'s module header has the argument.
5. **Still ungated:** the three-thread shutdown path end to end, and
   `KeySlot`'s id match. Measured by hand 2026-08-07 (a signal test is a
   clock test — `CLAUDE.md`): SIGTERM flushes and exits, SIGKILL leaves no
   `.tmp` and the next boot resumes off the last cadence save.

## 0x · The client makes sound — what it cannot yet hear *(client lane)*

Landed 2026-08-06+: `sound/` is the pure model, `render/audio.rs` the Bevy
half, the bank is **generated at boot** (`sound/synth.rs` — a licence
posture, not a preference), one ring drain (`render/feed.rs`), remote
footsteps, the place cue. Research `reference/AUDIO.md`; every number is
`DECISIONS.md` §open "audio v0". Remaining, in order:

1. **Nothing scores it, because `ART.md` has no audio section at all** — and
   **nobody has heard it** (this box has no audio device), so it is honest
   programmer art until someone plays it. `cargo run -p client --bin
   soundbank -- <dir>` writes all 38 WAVs. Looking already paid twice (the
   flat wind bed, then its fix overshooting); neither was reachable from a
   statistic that only asked "does it have energy". **The score raises the
   stakes on this item rather than answering it**: nine of those WAVs are
   music, and music is the thing a listener judges fastest.
2. **The score is built and unheard** (2026-08-11). `reference/AUDIO.md`
   §8's whole design is `sound/music.rs`: gap timer, a theme of sectioned
   pieces, tiers picked at section boundaries off bumps we already had.
   What remains is the half that was always the blocker — `synth::score`
   generates nine placeholder pieces, so **the system is real and the music
   is programmer art**. Swapping in recorded pieces is a change to one
   function (`synth::render`'s music arm); the licence posture in `synth`'s
   header is why they are generated and not sourced. Two inputs the
   reference bumps on and we cannot: a weapon *equipped* (we bump on the
   swing instead) and a projectile near-miss (`reference/PROJECTILES.md`).
3. **The `--capture` run is still by hand**, and it is the only thing that
   proves *most* of the audio systems execute at all. It needs Xvfb, lavapipe
   and a shard, which is why it is not in `ci/gates.sh` yet. The score is the
   exception and shows the cheaper shape: `tests/music.rs` builds a bare
   `App` (`MinimalPlugins`, no window, no device, its own clock) and asserts
   the two music systems run and spawn what the director names. Every audio
   system with no world in its arguments could be gated that way.
4. **Two cues still have no producer**: `ImpactWood`/`ImpactMetal` need to
   know WHAT was hit, which the gather toast does not say, and `UiClick`
   needs a hook in the per-screen click handlers.
5. **No occlusion, and it needs a prerequisite rather than a pass.** A wall
   between you and a sound needs a geometry query, and the correct one is
   the sim's (`collide.rs`), not a raycast against render meshes.
6. **The ambience layer is one bird, and now it has a clock.** Birds are
   gated to daylight off the server's tick (day/night v0), so the
   prerequisite this item named is paid: **crickets are now a content-free
   companion pass** — a night-gated `Cue`, the bird layer's shape with the
   predicate inverted. The reference's localized-emitter *system* is still
   a later slice (§9.3: it arrives with a cull budget).

## 0z · The world waits for the server now — what the Bevy audit left *(client lane)*

Landed 2026-08-06: the client no longer builds a world at an origin the
server never named (`RENDER.md` §1.1; `DECISIONS.md` §open; `tests/ui.rs`
§E), plus `--features hot`. Remaining, in order:

1. **R-G4 is still the missing half of the Bevy-draws rule.** Placement has
   a gate; the no-gameplay-state-in-the-ECS rule still has none. Its answer
   is the renderer-attached/detached state-hash equality (`RENDER.md` §5).
2. **Nothing photographs the wait.** A capture run exercises it and
   `capture::PLACE_FRAMES` bounds it; *seeing* it is §0p2 item 3's viewer.

## 0x · The native client can play the game now — what it still owes *(client lane)*

Landed 2026-08-06: every wire verb has a key, the decoded stores draw, Dead
and Map exist, the look/strafe inversion is fixed (`look.rs`). Remaining:

1. **Trim Bevy's default features — with a verified build, not a guess.**
   Genuinely unused, by grep: `bevy_gilrs` (no `Gamepad` anywhere — the one
   real system-dep win, `libudev`) and `vorbis` (the bank is WAV we
   generate). Load-bearing despite older notes: `bevy_audio` (audio v0),
   `bevy_gltf`/`bevy_animation` (the mannequin), x11 and wayland (a windowed
   game). Attempted 2026-08-06 and backed out for reasons that were not the
   code: a feature change invalidates every Bevy artifact (32G → 44G on a
   49G disk, `rust-lld` SIGBUS), and a green compile is not evidence — Bevy
   answers a missing decoder with a white fallback and keeps going. It wants
   disk headroom and a `--capture` run someone looks at.
2. **Closed 2026-08-10.** The greybox mirror is one list now — the drawn
   structure is derived from the sim's box table (`props::authored`), so the
   drift cannot recur — and `crates/client/tests/greybox.rs` gates the rest,
   including the occupant table for everything that is not a tree. The sim's
   list won the authority call, and the props' invisible collision skirt is
   closed too (a boulder blocked 0.39 m wider than it drew; the rows carry
   measured bounds now and the gate is an equality check). `TERRAIN.md` §7.1
   has it. **Still uncovered: the clutter ring.**
3. **World-space anchors are still dropped** (the HUD line landed —
   `hud::readout` pins struct-hit fraction and the charge clock under the
   toast): the wall's own number at the wall itself, a clock on the charge
   mesh (`charge_deploy` unread until that mesh half wants it), and
   `stock_addr` never says WHICH hearth. None is blocked.

## 0s · The front door — what the shell, the splash and the hub left *(client lane)*

Landed 2026-08-09/10 over two passes. `Screen::Boot` is the splash (the
launcher handshake and connect are states now, so a dead shard lands on the
server list instead of `exit(1)`). `render/ui.rs` owns the shell the five
reference frames share. PLAY GAME is the reference's table. NEWS / ITEM
STORE / WORKSHOP are the scry-works launcher's and hand off to it
(`ui/hub.rs` + `manifest.rs`), and the backdrop is **footage** under a scrim,
not a live scene — the operator's correction, and the cheap way round.
Three seams that were computed and dropped are now read: the claimed
address, the launcher's shard-list url, and the launcher connection itself.
Remaining:

1. **The backdrop does not move.** Bevy decodes no video; a loop is a frame
   sequence, ~12 MB for three seconds at 720p/20fps. That trade is the
   operator's — `DECISIONS.md` §open, "menu backdrop v0". The shipped still
   is a `--capture --no-hud` plate of our own island, so a better one is a
   command, not an art commission.
2. **Nothing publishes `news`/`store`/`workshop` yet**, so all three read
   "the launcher's manifest names no link for this". The client side is
   done; the remaining act is the platform's — add the keys beside
   `servers.url` in `data/launcher/gates.manifest.json`.
3. **Ungated, by hand only:** the star, the search box, the filters and the
   OPEN IN LAUNCHER click were driven headless with `xdotool` and looked at,
   never against a populated list or a live launcher (§0v item 1).
4. **The splash cannot cover its own first ~3 s** — wgpu adapter enumeration
   and window creation precede the first Bevy frame. A second process would;
   not taken.

## 0w · The native menus landed — what they cannot do *(client lane)*

Landed 2026-08-06: `Tab` inventory + crafting, `B` build wheel, drag/drop —
arithmetic in `ui/` (pure), drawn by `render/panels/`, 23+ assertions in
`tests/ui.rs`. Remaining:

1. **The rail is not the reference's, and one wire field would fix it.**
   `EventMsg::Catalog` ships display names only, so a category rail by item
   class is not computable client-side. A class byte per item, a `PROTO_VER`
   bump and regenerated goldens in the same commit (wall 6) buys the frame's
   real rail. Today's buckets are honest but they are not that.
2. **The drag is gated as arithmetic, not as a gesture.** The spawn kit
   removed the empty-pockets blocker; press → ghost → release → send against
   a live shard is still verified by inspection only.

## 0v · The menu flow landed — what it still cannot show *(client lane)*

Landed 2026-08-06+: server-select first, `Loading`/`Paused`/`Settings`, a
failed connect returns with the reason, settings persist (`crate::config`,
`DECISIONS.md` §open "settings v0"), and `Screen::Disconnected` latches a
hangup through the menu's own teardown. Remaining:

1. **The document exists now; the two acts that serve it are on scry's
   box.** `shards.toml` is written and `./ci/shardlist.py` produces
   `target/servers.json` — one row, `game.moreright.xyz:61234`, carrying a
   `status_url`. What is left is exactly what it always was and no more:
   copy the document into `$SCRY_DEPOTS_DIR/gates/`, **then** set
   `servers.url`. In that order — `servers.url` pointing at a file that is
   not there is an error dialog on a game that is running fine, which is
   worse than the honest "no shards published" both readers draw now.
   ⚠ **scry's half is confirmed live, not assumed**: `GET
   /api/launcher/servers/gates` answers **404** as of 2026-08-11, which is
   its documented "publishes none" and is a different answer from the 503 it
   reserves for "could not look". The route is built and waiting for bytes.
   (The 2026-08-10 finding this replaces was that `/depot/` was not a
   `location` on that origin at all, so the url printed here could only 404
   for the wrong reason.)
2. ~~Player counts: three steps on a box~~ — **two of the three are done
   2026-08-11, and the count is live.** `status_addr = "127.0.0.1:8431"` is
   in `shard-public.toml` and the url is in `shards.toml`. The third step
   was "open that TCP port (the cloud firewall too)" and it was **not taken,
   on purpose**: the endpoint binds LOOPBACK and nginx fronts it on the 443
   this box already serves, so `https://game.moreright.xyz/gates/status.json`
   needs no console act, carries the same certificate as everything else we
   publish, and puts a buffer in front of a status thread that answers
   serially by design. It answers `{"players":0,"max_players":100,"tick":T}`
   right now. Both readers still draw `?` until item 1's copy happens —
   there is no list to draw a row in.
3. **Ungated, by hand only:** the end-to-end kill-the-shard-mid-play run
   behind `Screen::Disconnected`.

## 0t · the forest — what it owes, re-ranked off `reference/PLANTS.md`

Landed: `render/tree.rs` calls `bevy_procedural_tree` as ONE pure function.
**Felling v0** (2026-08-10): a chopped tree topples on a bearing derived from
the cell key, keeps its own mesh, and stays down — gate `tests/fell.rs`, knob
`DECISIONS.md` felling v0. Gates: `tests/tree.rs`, `tests/fell.rs`.

**The order below is `PLANTS.md` §6.2's and it inverts what this item used to
say.** LOD was rank 1; it is now rank 3, because clumping puts MORE stems in
the near ring and an LOD tuned against today's lattice is tuned against a
distribution we are about to replace. Measure between the two.

1. **Species v0 landed; the broadleaf has never been LOOKED at.** `SPECIES` is
   a two-row table (conifer 6.6 m / 2.9 m-wide broadleaf), pool 6, and
   `SPAWN_CLEAR_M` rose 4.0 → 4.5 with the arithmetic finally gated in Rust
   (`a_fresh_spawn_stands_clear_of_the_widest_tree` — `ci/pine_shape.mjs` was
   a dead citation). **Every check on it is arithmetic and arithmetic cannot
   say whether it reads as a tree.** Boot it and look; the parameters most
   likely to be wrong are `children`/`angle[1]` (crown spread) and leaf
   `count`/`size`, and `reference/PLANTS.md` §3.1 has ez-tree's 15 presets to
   pull real ash/aspen/oak numbers from instead of our derived-from-defaults
   block. More species is now a row in `SPECIES`, not a refactor.
2. ~~The scatter lattice~~ — **this item was wrong and is retired.**
   `terrain::clump` has always existed: an fBm field `scatter` multiplies the
   whole weight row by, squared for a ragged edge, gated by
   `sim-core/tests/scatter.rs` against a closed-form independent-draw null.
   Groves and clearings are built. What is actually open is the density
   **ceiling** — one occupant per 8 m cell — and `reference/PLANTS.md` §3.2
   prices the three ways to raise it. All are sim-core, none is cheap, and
   the cheapest (`CELL_SIZE` 8 → 4) quadruples the live `SlotLives` rows
   against `TERRAIN.md` §6's budget. Do not start it as a rendering change.
3. **The billboard LOD.** 328 trees × 5.9 k tris is 1.9 M against DESIGN §9's
   1.5 M. Octahedral impostors beat SeedThree's crossed cards (a card edge-on
   disappears); `PLANTS.md` §3.3 has both. Whatever LOD1 becomes, it sways.
4. **`aWind`** — `StandardMaterial` cannot read a custom attribute, so wind
   needs the custom material `RENDER.md` already lists. Gets LOD1 for free.
5. **The sub-canopy and shrub layers are empty** (`PLANTS.md` §2). ez-tree's
   three `bush_*` presets and a small tree at 40 % are new `Occupant`
   variants plus scatter rows once item 1 lands.
6. **The needle card is generated** (`tree::needle_image`); `WANTED.md` §9.5
   is the swap, and it is the highest-value texture on that page.
7. **Owed upstream as a bug report:** `BranchForce` pointing down hits the
   antipodal singularity in `Quat::from_rotation_arc(Y, dir)` and bends the
   whole tree sideways — droop is the limb ANGLE's job.

## 0u · the frame budgets are browser numbers and nobody has re-derived them

**Doc pass landed** (`DESIGN.md` §9, `RENDER.md` §6, `ART.md` §7,
`TERRAIN.md` §4/§6, `NETCODE.md` §4): every performance claim now says which
platform it was chosen for. The open question is not a doc problem:
`DESIGN.md` §9's budgets were set for a WebGL page and three no longer
describe what constrains us —

- **initial load < 15 MB** and `ART.md` §7's **12 MB texture payload** are a
  first-visit *download*. A depot install is not one, so 2K/4K re-sourcing
  is unblocked; what is real natively is VRAM and disk, and nothing has
  measured either.
- **< 300 draw calls / < 1.5 M tris** are WebGL-shaped, and two shipped
  numbers are already rationed against the 1.5 M: `CLUTTER_RICH_PER_TILE =
  96` and the conifer ring's "over budget" verdict (1.9 M).
- **60 fps on a mid laptop iGPU** survives — a hardware floor.

**Nothing was renumbered.** These are `(knob)` and therefore spoken, and a
budget raised by the loop that then justifies the loop's own triangle count
is the wrong direction of travel. The measurement is small: capture on a
real GPU at the ring's p90 tree count, read draw calls and frame time off
`RenderDiagnosticsPlugin` (its wall-clock half is not assertable —
`CLAUDE.md`), and propose into `DECISIONS.md` §open. Related: the anisotropy
ceiling `BASE_ANISOTROPY_MAX = 4` was set for a software-rasterizer reason
that does not transfer.

## 0a · The clutter ring's fade — two findings kept *(client lane)*

The browser item was retired 2026-08-06 (`DECISIONS.md`); the native ring is
`render/clutter.rs` and still ends hard at ~32–45 m. Two findings survive
the deleted item (full text in git):

- The fade's recipe: thin stochastically by instance hash so the same
  elements survive at a given range, then scale survivors to zero — and
  whether the edge reads at all at that distance is a question for a person
  with the game booted, not for a guess.
- Beach skirts are thin because `scatter` puts 0.22 prop centres a tile on
  the coast against 0.95 inland — the two ratios match to a tenth. That is
  the scatter table's business, not the skirt path's.

## 0ad2 · The admin lane is built — what it still cannot do *(server lane)*

Landed 2026-08-11 (admin v0, `DECISIONS.md` §open). Six verbs on the chat
lane with **no wire change**, the anomaly log with its counter sweep, and
`/bug`. Gated by `tests/admin_wire.rs` (7) and `protocol::admin` (6).
Remaining, in order:

1. **A ban dies with the process.** `Bans` is memory only; persisting one
   wants its own file with its own format version, because sharing the
   player store's header would wipe it on the next seed change.
2. **Nothing has typed a command against a live shard.** Every branch is
   gated headless; the socket half (`conn.close` with `REFUSE_ADMIN`, and
   the client's dialog for it) has never been driven end to end.
3. **The log has no reader.** It is JSONL on purpose so `jq` is the
   reader, but nothing summarises a session — and the alpha gate's "zero
   silent failures" wants a *verdict*, which is a script somebody runs
   after a playtest, not a counter.
4. **No `/who`, no `/tp <a> <b>`, no set-time.** The last is blocked by
   choice: day/night derives from the tick, so moving the clock means
   moving the tick — it wants the wire field §0y4 deliberately did not
   spend.

## 0q · The gaps nobody has claimed

Lifted out of "done this pass" items before pruning (2026-08-05, again
2026-08-09) — each was written down **only** inside a done item. All of it
is `crates/`/wire work no single-surface lane may take.

1. **The UDP socket buffer is a `NETCODE.md` row and nothing else** (found
   2026-08-11 standing the public shard up). §2.2's config-of-record says
   `SO_RCVBUF/SNDBUF 8 MiB, passed via with_bind_socket`; nothing in
   `crates/` calls it, and the shard is running on this box's default
   `rmem_max` of 212992 — the ~208 KiB the row's own "why" column names as
   too small, quoting quinn's README. Two halves and the order matters: the
   **code** half asks for the buffer (one `with_bind_socket` at the
   endpoint), and only then does an ops sysctl raise the ceiling it would
   otherwise hit. Doing ops first buys nothing measurable, which is why this
   is a `crates/` item and not a runbook line. The row is marked ⚠ in place.
2. **Shore barrels as a second destination class.** The road pays unevenly
   now (the bay slots landed) and the haven pad is the one place worth
   walking to. A second class on the shore would give the ring two ends
   rather than one. Nothing else in this file mentions it.
3. **The wipe.** Named by both judges, described nowhere. A shard lifecycle
   act with an economy half (`ALPHA.md` A1→A3) and an operator half
   (`CLAUDE.md`: wipes of a live shard are operator-only), so the loop's
   share is the mechanism, never the trigger. Needs scoping before it can be
   an item.
3. ~~You cannot stand ON anything~~ — **done 2026-08-11** (deploy collision
   v0: `slot_ground` beside `slot_blocks`, occupant and deploy tops are
   ground under the lid rule; the plinth, crate, boulder and box tops all
   stand; gated in `tests/solid_deploy.rs`).
4. **The 100-bot soak RAN 2026-08-12** — baseline in `DECISIONS.md` §open.
   Headline: **`dropped-ticks 0`** over ~61,500 ticks with 100 clients, and
   **0 shed** of 17.5 M AOI entities offered, so the tick budget and the
   interest band both held at a population they had never met. The anomaly
   log's whole path was proven in the same run (8 bots against a full shard
   made `refused_full` move, and the file gained exactly that line).
   **Four things it still does not have**, each its own small item: real
   **bytes** (nothing counts them — the 16.5 kB/s/client figure is a
   ceiling, not a measurement), jitter as a **distribution** rather than a
   threshold crossing, an **hour** (this was 25 minutes, so slow leaks are
   not excluded), and **contention** — bots walk, they do not raid, so wall
   4's caps are still gated one site at a time.
4. **You cannot stand ON anything.** `movement::step` asks `slot_blocks` and
   nothing asks a ground query for occupants — the shelter's plinth reads as
   a kerb you sink into, crate and boulder tops the same (`terrain.rs`'s
   plinth doc still says "nothing here makes a body stand on the plinth").
   Belongs beside `collide::piece_ground`, a `slot_ground` next to
   `slot_blocks`; the fourteen-box table is already there for it. Systems
   lane.
5. **The 100-bot soak has never been run.** `NETCODE.md` §9's budgets have
   never met 100 real connections: `cargo run -p server --bin bots -- 100`
   against a dev shard, held an hour — tick jitter, WAL append rate,
   per-client bandwidth recorded as counts and bytes, never wall-clock
   asserts (`CLAUDE.md`'s clock rule). The numbers land in a `DECISIONS.md`
   §open row as the measured baseline. **The AOI half is settled without
   it** (§0sp, 2026-08-11): 100 clients in one cell cost ~0.8 ms of a
   33.3 ms tick, so the linear scan needs no spatial structure. What a soak
   still owes is what a profiler cannot see — sockets, jitter, real bytes.

---

## 0r · The raid loop has offence now — what it still cannot do *(systems lane)*

Landed: `sim-core/charge.rs` — plant the held throwable at an address, fuse
from content, damage through the same `damage_piece`/`damage_deploy` a swing
uses (`ACT_THROW`/`EV_CHARGE_PLACED`; knobs `DECISIONS.md` §open "satchel
fuse v0"). X plants it natively and the HUD counts it down. Remaining:

1. ~~No blast radius~~ / ~~nothing is hurt by standing in one~~ — **both
   done 2026-08-11** (satchel blast v0, `DECISIONS.md` §open: linear
   falloff over a bounded one-cell ring, bodies take `damage` with the
   planter included, `DEATH_BY_CHARGE` on the v36 widening,
   `WORLD_SAVE_FORMAT` 4 carries the blast and fixed the mid-fuse-save
   refusal found on the way). Residue: no detonation sound or visual —
   the client learns of a blast only through `EV_STRUCT_HIT`/`EV_HEALTH`,
   so a near-miss is silent (audio lane); dud and defuse stay unbuilt.

## 0a · The island has a map now — and the trip has both ends *(ui lane)*

Landed 2026-08-09: the marker layer — haven and waystations as rings,
bed/hearth/backpack marks, one projection (`world_to_map`), cap 64
drop-newest (`DECISIONS.md` §open "map markers v1"; seven lib tests in
`ui/map.rs`). Still open, neither of it ours:

- **Operator:** the death marker (`ALPHA.md` §1 keeps position off the death
  screen; the map touches no death fact today), and whether the marked set
  is right — boxes and doors stay unmarked deliberately.
- **Respawn — the gap's other half — is BLOCKED, measured.** The wire
  carries `Respawn { on_bag: bool }` and nothing else; no owner bit and no
  cooldown ride `DeployRec` (`deploy.rs`, "never the wire"). So the client
  cannot tell its own sleeping bags from anyone's, nor which are ready, nor
  name one. "Beach or each live bag" (`ALPHA.md` §1) is a wire change first
  — systems lane.

## 1 · The native pivot — what is left of it

The client is native (operator, 2026-08-05; `DECISIONS.md` has the row). The
session, input, terrain, lighting, scatter, HUD, panels, depot packaging —
R0–R6 plus R8 — all landed, and the browser client is deleted. Left:

- **Publishing and notarizing the depot are operator acts and are NOT
  done.** The build ships as a scry depot (`ci/depot.py`, gated by
  `--self-test`); the depot ships `assets/`, not just the binary.
- **The visual gaps, ranked by measurement** (`RENDER.md` §8 carries the
  list): the hemisphere fill (§0w item 1 — the top gap, coupled-lighting
  single owner), cloud form (the deck reads stratus where `ART.md` asks for
  cumulus), and the four-way splat material (one map serves all four ground
  identities today).

---

## 4 · The event lane's payloads are law — all 32 codes gated by role

Landed (`sim-core/tests/event_roles.rs`, finished on `lane/event-gates`):
every `EV_*` code carries a role check against a real cause, all a/b-swap
mutants reproduced red, `NOT_COVERED` is empty, and the ledger seat stays
for the next code. **Remains, and it is not tests**: the stronger form is a
payload-role table both the emit site and the check read, a swap as a
*compile* error (`reference/FINDINGS.md` §1 end) — bigger than one pass.
(The `CLAUDE.md` trap-list correction this item asked for is done — that
entry names the landed gate and keeps the mechanism as the lesson.)

---

## 4b · The world lane: what the second tier left open

- **The recycler exists and the haven does not have one.** Landed
  (recycler v0, `DECISIONS.md` §open): `ARCH_RECYCLER` converts salvage on
  `oven::sweep` with the burn skipped, `CookRow::count` plus multi-row
  firing pays several outputs off one clock, wire v31. What is still open
  is the half this item was really about: **every deployable comes from a
  player placing one**, so the recycler is craftable and a destination
  still offers no verb you cannot perform at your own base. An authored
  worldgen deployable is the missing mechanism — a `DeployRec` standing at
  the pad that no player placed, which has to answer to persistence (a
  restart must not duplicate it) and to `pick_up` (nobody pockets the
  haven's machine). Systems lane. Bank and vendor stay blocked on an
  operator act.
- **The waystations want a silhouette, and it must be a *different* one.**
  Their containers and loot tables differ from the pad's now; the site
  itself is still two boxes on bare ground, and a second copy of
  `HAVEN_SHELTER` would make the two tiers look identical.
- **The pad carve is still unbuilt, and smaller than this file used to
  say**: `height` has 18 production call sites in 3 crates (not "~80 in
  four"), and `haven()` measures 12,463 taps mean over 16 seeds. Re-scope
  against 18 before assuming it cannot be a pass. Whether a tier should
  carve at all is **open for the operator** (`DECISIONS.md` §open,
  waystation canopy v0).
- **Nothing threatens the walk between them.** The pig flees and never
  fights — §0m item 2 is this gap seen from the other end.

## 4b · The domain gate reads the crate now — one residual

Landed 2026-08-05: `SOURCES` reads all `sim-core` modules both ways and
every enumeration width is classified. Remains:
`death_causes_are_a_closed_ledger` (`event_roles.rs`) still scrapes
`world.rs` alone — narrow, since the protocol gate catches a stray value
crate-wide, but its *contiguity* claim is file-local.

---

## 5 · Gameplay still missing, in rough order of what a player notices

- **The arrow does no structure damage, and it is as fat as a body.** An
  arrow that reaches a wall stops dead rather than chipping it, and
  `collide::blocked` bakes `CAPSULE_RADIUS_M` into its query, so an arrow
  threads a doorway but never an arrow slit. The honest fix for the second
  is a radius parameter on `collide` — a `sim-core` change with a
  replay-gate consequence, so it wants its own commit.
  **Operator, 2026-08-10: ranged tracks the reference game as closely as we
  can, and arrows come back** (`DECISIONS.md`; `reference/PROJECTILES.md` §9
  is the sized list). Landed off it: ballistics on the round (§9.3), and
  `EV_SHOT` + the tracer, so the arrow is visible at last (§9.2, wire v33).
  Next is **arrow recovery** (§9.7) — the spent-arrow store, the ~15 % break
  and the 10 s lodge, and the first verb in the protocol addressed to a
  world position rather than a build cell, which is why it is a protocol
  pass and not an afternoon. It gates §9.6: their bow damage is priced
  against arrows that come back, so no bow number may track theirs until it
  lands. Then `headshot_mult`, armed-and-unread since the content crate
  (§9.4) — §7 says take the most significant body part, never the first
  intersection.
- **The revolver still cannot fire.** Hitscan wants M2's rewound raycast, so
  `bake_combat` drops firearm rows deliberately, not by omission.
- ~~**Dropped loot should land somewhere you can find, not inside the
  floor**~~ — **landed 2026-08-14.** Six producers call `inv_add_spilling`
  (`gather`, `craft`, `build`, `deploy`, `lock`) and `World::drain_spill`
  stands a bag up at your feet; the client says so. §0sp2 has what remains.
- **Mushrooms and corn drop now** (2026-08-09, content rows only): the
  tree's secondary pays 1 mushroom a swing — the forest floor through the
  tree that shades it — and the coast-road barrel rolls a 2–4 corn ration.
  `content.rs::every_consumable_the_content_ships_is_reachable` gates the
  general form (every consumable producible by a live verb chain). Still
  owed, and both are code: a standalone forest-floor pickup archetype and a
  farming lane. The open verb landed 2026-08-14, so the third clause here is
  spent — but `validate` still counts barrel rows alone and its stated reason
  ("no verb opens a container") is now false. Widening the reachable set to
  the tables a verb opens is a `validate.rs` change nobody has made.
- ~~Day/night does not exist~~ — **landed 2026-08-11** (day/night v0,
  `DECISIONS.md` §open): 45-minute cycle, 70 % day, derived from the tick
  with **no wire field**, driven through the rig's coupled-set owner.
  What it does NOT do: no gameplay reads the clock (no nocturnal mobs, no
  crops, no torch — the survival clock `DESIGN.md` §2 pairs it with is
  still hunger and thirst alone), no moon or stars in the night sky, and
  no set-time admin verb — moving the clock means moving the tick, so
  that one wants the wire field this slice deliberately did not spend.
- **The coin loop is closed and the tech TREE is not.** OBOL is paid by
  the recycler and burned at the research table (research v0, and the
  operator's 2026-08-10 call that OBOL is scrap — what stages is the claim
  rail). What research does NOT have is depth: a row unlocks one recipe
  and depends on nothing, so there is no ladder, no tier, no "unlocks the
  next". The reference has a research table *and* a tech tree and they are
  separate systems; ours is the first. A tree is a content graph over the
  bits `Player::known` already carries — a `requires` column and a
  reachability check in `validate` — not a change to the sim. Also absent:
  a blueprint ITEM (learning is instant and personal, so there is nothing
  to trade) and the wipe schedule `DESIGN.md` §8 promises blueprints will
  outlive, which is unbuilt because no wipe is.
- ~~No verb opens a world container~~ — **landed 2026-08-14** (world
  containers v0). This bullet denied a verb the same commit shipped and
  stood for a whole pass; it is the merge-gate judge's ranked fix 1
  (`findings/pass-20260813-230343-05-judge.md`, check 9). Residue is
  §0wc's list, not this one.

---

## 5b · The wire accepts values the sim can never mean

`every_domain_fits_its_wire_field` (`protocol/src/event.rs`) gates ten value
domains; the sim/server refusal side is closed (`lane/wire-values`:
`BAG_GONE_*`/`REFUSE_C_*` refused at the pump and counted, `buttons` bits
4–7 refused at `accept_input`, never a disconnect). Still open, the wire act
this item always named: the *decode* side — the client's decoder taking
`why == 3` / `reason` 4..15, the button octet — plus deriving the two
`*_MAX`s into protocol's exempt list and the `PROTO_VER` judgement for
narrowing what decodes. One protocol pass. Systems lane (`crates/protocol`).

---

## 7 · Milestones — the arc is `DESIGN.md` §11; this is what the queue adds

**Read the arc there, not here.** M0 (landed) → M1 survival verbs → M2
combat true → M3 OBOL → M4 the counter and the door, with each one's exit
condition. `ALPHA.md` §6 folds into the same section, and this list used to
restate it under a second numbering — two lists, one arc, drifting apart.
Struck 2026-08-11; nothing was lost, because everything struck was a
paraphrase of §11.

Two gates sit **between** those milestones and belong to the queue rather
than to the arc:

- **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle,
  after M3 and before A2/A3 arming (`ALPHA.md` §2). A loop proposes it and
  never runs it.
- **Arming A2, then A3** is an operator act, not a milestone anyone here
  completes (`CLAUDE.md` §loop discipline).

And two items the arc does not carry, which stay real work:

1. **Anti-ESP occlusion culling** — the measure the genre proved
    (Facepunch, 2025, network-wide default). Server-side, costs no client
    trust, and the occlusion grid is a pure function of the seed, so it is
    bakeable at worldgen and a lookup in the tick. Sequence after M2: it
    wants real sightlines to tune against.
2. **~~The launcher, in Rust, with the wallet in it~~ — BUILT, and not in
    this repo** (`DECISIONS.md` 2026-08-04 asked for it). It shipped in
    `scry-forge` as `launcher-rs/` — one binary with no runtime to install,
    an account generated on the holder's machine and written as an
    encrypted keystore, and both our depots published and notarized on
    2026-08-10. It reached this list because the row itself said it is
    *"the platform's client for the whole cascade, not a Gates
    accessory"* — which is exactly why it was never ours to build. **What
    is still ours** is the seam: `crates/client/src/scry_overlay.rs` stays
    byte-identical to the SDK upstream (`CLAUDE.md` §vendored), and
    `ci/shardlist.py` writes the document the launcher's Servers window
    reads. Derive the launcher's real state from scry, never from this
    line.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.

## 5c · The protocol golden has never fuzzed a button above bit 1 *(systems lane)*

Found while landing jump. `goldens.rs` draws the input fixture's `buttons`
from `rng.next_bounded(4)`, so the golden exercises only `BTN_SPRINT` and
`BTN_CROUCH` — `BTN_PRIMARY` and `BTN_JUMP` are outside the draw. The field
is 8 bits wide either way, so the golden still pins the *layout* and nothing
is currently wrong on the wire; what it cannot see is a future encoder that
masks or reorders the high nibble.

Deliberately its own commit: widening the draw changes fixture bytes, and
changing golden bytes for a reason unrelated to the version's meaning
muddies the one signal wall 6 reads. It is a `PROTO_VER` judgement call —
the answer may be that a golden's fuzz range is not part of the wire
contract at all. Decide that first; it is the actual question. Same shape
one level down: whether `decode_input` itself should narrow the unmeant
bits is the protocol pass §5b still owes.

## 5d · The agent player has a spec and no code *(systems lane)*

`PLAYERS.md` landed 2026-08-05 — the verb set, the observation encoder, and
four walls with their gates. Nothing under it is built. `sim-core/bots.rs`
already drives deterministic synthetic input, so the missing piece is the
intent layer above it, not a new client.

Smallest useful slice, and it is not the API: **log the condition.** Every
trust-bearing verb (door, TC authorize, container access, give) gains an
event carrying whether the counterparty was online, landing inside
`tests/event_roles.rs` with two causes per code in the same commit (§4's
discipline). That field is the whole measurement `SUBSTRATE.md` §3 turns on,
it is ordinary game state a human client already sees, and retrofitting it
makes every shard-hour logged before it worthless. It is also independently
useful: offline-raid telemetry is a thing the game wants anyway.

Then the verb table, then an agent client that plays badly. Wall 1 (agent
verbs ⊆ human verbs) wants its gate in the same commit as the table, not
after — it is a subset assertion over two lists and cheap while both are
small.

Not this lane's call: what an agent pays to enter and what it earns
(`ALPHA.md` + scry side).

## OP · the operator lane — a loop cannot pick any of these

Moved to the bottom 2026-08-13, unchanged. They sat at the top of the
file for a week, so every pass read ~100 lines of work it is not allowed
to do before reaching an item it could take. They are still live and
still the operator's; nothing here is a queue entry for a builder.

## 0vj · The visual judge is off, and the port back is one script *(operator lane — harness)*

**A loop cannot do this**: the harness is outside the repo by design
(`CLAUDE.md` §the loop that builds this repo). Recorded here so the loop's
missing half is work rather than an absence nobody notices.

The loop restarted 2026-08-13 with `GATES_CAPTURE=0`. `art/capture.mjs`
drives Playwright against the browser client and has been dead since
2026-08-06 — it would fail every pass. So **no frames are captured and no
`-visual.md` is written**: every render pass until this lands is scored by
the merge-gate judge alone, which is the blind-pass condition the visual half
was built to end (M1 slices 15–20).

The replacement is already in the tree and needs no new design:
`crates/client/src/render/capture.rs` — `gates --capture DIR`, the same six
fixed vantages, settling on ring state rather than a clock, `--no-hud` for a
clean plate. `CLAUDE.md` carries this box's working `VK_DRIVER_FILES` + `Xvfb`
invocation. What is missing is a shell wrapper the runner can call in place of
`capture.mjs`, plus re-pointing `TRIPWIRE_FILES` and flipping the default back.

One repo-side half is genuinely ours and worth doing first either way: the
probe writes PNGs only, and the visual judge's prompt asks for a
`manifest.json` carrying the run's errors. A capture that reports what the
client logged while shooting is better evidence than six pictures alone.

## 0sl · The shard list reaches the game *(operator lane — two acts, in order)*

**A loop cannot finish this.** The tree half landed; publishing is the
operator's.

What was wrong: the public shard is up and its list is served
(`/api/launcher/servers/gates`, `servers.url` set), and the in-game browser
was still empty on every launch that did not come through the launcher's
Servers window — nothing on the argv could carry the url. scry gained a
`{servers}` placeholder; `ci/depot.py`'s `LAUNCH_ARGS` now asks for it.
`shards.toml` also said `eu-1` while the served document said `us-east-1`,
so the next regeneration would have re-published a row key nobody's
favourites matched; the served name won.

The two acts, and **this order is not a preference** — a depot using
`{servers}` needs a launcher that knows it, and nothing in the depot
document can declare a launcher floor, so an older launcher refuses the
whole launch:

1. **Ship the launcher** carrying `ARG_VARS` with `servers` in it
   (scry-forge, `launcher-rs`).
2. **Re-publish Gates' depot document**, so `launch.args` carries
   `--servers {servers}`. `python3 ci/depot.py`, then the depot ceremony in
   scry `docs/client/LAUNCHER.md` §8.

Until (2), the fix is inert and the browser stays empty — `--servers <url>`
on the command line is the workaround, and joining from the Servers window
already works.

## 0wd · A new world register is proposed *(operator lane — blocked, skip)*

**A loop cannot pick this up.** Logged here so it is visible, not queued.
`WORLD.md` (new, 2026-08-10) carries an exploratory operator direction, and
is a **roadmap rather than a v1 spec** — nothing in it competes with the
alpha. `DECISIONS.md` §open has the row; nothing is spoken.

Three findings in it are about the tree rather than the fiction:

- **`ART.md`'s bar and the visual rubric are measured off the reference set,
  and the rubric is checksummed outside this repo.** If the register
  changes, every visual pass is scored against pine-and-granite while
  building obsidian, and the builder cannot fix it. Three operator acts —
  palette, a reference set, rubric style section — and 2026-08-01's art row
  already names that exit. **Until then, no visual pass chases this.**
- **A ward would invalidate `CONTENT.md` §4 anchor 2 without reddening
  `test_content`.** The TTK bands compute against `balance.toml`'s
  `globals.player_hp = 100`; a second regenerating pool makes them measure a
  different quantity while staying green. Conditional — the ward is
  explicitly undecided and nothing else depends on it.
- **Extraction and world states are one system or they are two.** An opened
  gate at the bank terminal and a repaired monument are the same object: a
  bounded, tick-expiring, hashed, broadcast state. The terminal lands at A2
  (`ALPHA.md` §2); if it ships a bespoke gate first, that is one idea paid
  for twice.

Cheapest real slice if it is ever spoken: the biome gradient — a radial third
input to `biome(h, moist)` (`terrain.rs:263`) plus regenerated terrain
goldens. `WORLD.md` §9.2 has the full order, and §9.1 the timing: **decide
the register early, build it late.**

## 0gh · The GitHub job-agent seam — the door is built; three acts remain *(operator lane + docs)*

Assessed 2026-08-11; the write-up is scry-forge `docs/builders/GITHUB-JOBS.md`.
Built already: `AGENTS.md` §the deal, the PR template's submit line, `gates`
CI on every code PR, 100,000 SCRY standing on scry's board (`DECISIONS.md`
2026-08-09). The board's paid ledger is `[]`; no outside fork has opened a PR.

- **(operator, GitHub)** Branch protection on `main` requiring the `gates`
  check — PRs #56–58 merged over days of red CI before the toolchain pin
  (`DECISIONS.md` §open, the compiler); until GitHub enforces it the merge
  gate is policy. Caveat: the workflow path-filters, so a docs-only PR
  reports no check; the fix is a same-named instant no-op for those paths.
- **(operator, wallet)** Sign `scry.sig.json` seq 1 — and the tooling is
  already here, so **no key is ever pasted**: `./ci/scry_manifest.py
  --print` shows the exact text, sign it in whatever holds the steward
  key, then `./ci/scry_manifest.py --sign --seq 1 --signature 0x…
  --wallet 0x…`. Unsigned, scry applies nothing — the store row and
  update feed are wired and inert. **It now buys more than the row:**
  scry's manifest standard grew a `jobs` block (`GAME-REPO.md` §4b), so
  once signed, this repo posts its own board lane's picked work from
  `scry.json` — guidance rows, never a price — and the six rows scry
  currently keeps house-side move here.
- **(operator, once)** Settle `gates-pr` end to end on the next accepted
  PR: pay by public transfer, append the row scry-side — the board's
  `settled_to_a_worker` stops being zero in public.
- **(operator, GitHub)** The repo description still says "three.js
  frontend" — stale since the browser cut, and it is the first line a
  stranger reads above `AGENTS.md`. GitHub → About; no API path for it here.
- ~~Milestones live twice~~ — **done 2026-08-11.** `DESIGN.md` §11 owns
  the arc, §7 here points at it, M0's seven dead checkboxes are prose.

Not owed, stated so it is not re-litigated: no issues queue (this file is
the queue), no auto-pay or auto-merge (merge is the act that pays, a hand
act), no webhook (the store seam stays a commit and a poke).

