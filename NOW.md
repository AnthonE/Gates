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

## 0wd · A new world register is proposed *(operator lane — blocked, skip)*

**A loop cannot pick this up.** Logged here so it is visible, not queued.
`WORLD.md` (new, 2026-08-10) carries an exploratory operator direction, and
is a **roadmap rather than a v1 spec** — nothing in it competes with the
alpha. `DECISIONS.md` §open has the row; nothing is spoken.

Three findings in it are about the tree rather than the fiction:

- **`ART.md`'s bar and the visual rubric are measured off `Rust Images/`,
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
`test_stream_in` and `test_raid_storm` with it (§11 there: **all seven of
its gates are unbuilt**, retitled this pass to stop claiming otherwise).

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

**Next in this class, and it is a feature rather than a reconciliation: no
deployable blocks movement.** `movement.rs` never consults `Deploys` and
`collide::blocked` takes only the piece column index, so a player walks
through a furnace, a box, a hearth and a recycler; only a closed door stops
anyone, and it does that as a piece-edge bit. The client draws all ten
archetypes at authored sizes (`structures::deploy_size`), so this is drawn
geometry with no blocked volume at all — the greybox gate cannot catch it
because there is nothing on the sim side to compare against. Systems lane.

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
`RIPLIST.md` §2: the boar does not fight back; no per-material damage
resistance (one `structure` column, so the ladder above stone is
compressed); one animal; and gather yields, smelt and craft times are
still ours — node totals are `READY` now, per-hit yields are not, and our
schema does not need them. Upkeep, decay and the armour ladder differ on
purpose (`BALANCE.md` §4.1), though the upkeep *rate* turned out to match
theirs.

---

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
   both. The nightly artifact is the tree those acts consume.
2. **The shard list is written and never served.** `ci/shardlist.py` produces
   the document and both consumers parse it, but `manifest.servers.url` on
   scry's side is `null`, so the launcher's Servers window and our own menu
   are dark for the same missing file. Everything downstream of that one
   publish now exists: live counts via `status_url`, and join links
   (`scry://join/gates/host:port`, `deeplink.rs`). Registering the scheme
   with the desktop is the launcher's installer, and is not done.
3. **`prove` has no call site.** `sign_siwe` hands the launcher a string this
   process composed; `Overlay::prove(server, nonce)` binds it to a name and a
   nonce the SHARD chose, which is the difference between a signature a shard
   can verify and one it can replay. Wiring it is a wire change (wall 6), so
   it waits for the identity-in-handshake slice, not for this one.
4. **The depot is Linux only.** `ci/depot.py` says so in its first line and
   scry's platform enum has the other rows. The SDK can now reach a launcher
   on Windows; nothing packages a Windows build of this game.

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
`tests/water.rs` (22) and eight assertions in `tests/sound.rs`. Remaining:

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
2. **Nothing fights back.** Needs a mob→player damage path, a new death
   cause on the wire (**the 2-bit cause field is saturated since wire v24**,
   so this is the widening), and a combat-feel answer to being hit by
   something you cannot reliably hit back.
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
5. **The viewmodel is two untextured boxes**, and it is in a third of the frame.
6. **Roughness maps are still unread** — all nine of them. Blocked on an ORM
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
   soundbank -- <dir>` writes all 19 WAVs. Looking already paid twice (the
   flat wind bed, then its fix overshooting); neither was reachable from a
   statistic that only asked "does it have energy".
2. **Music is the highest-value unbuilt thing and the design is written
   down.** `reference/AUDIO.md` §8: gap timer, sectioned themes, an
   intensity scalar off event codes we already have. Every input exists;
   what does not exist is music — a **content** blocker, not engineering.
3. **The `--capture` run is still by hand**, and it is the only thing that
   proves the audio systems execute at all. It needs Xvfb, lavapipe and a
   shard, which is why it is not in `ci/gates.sh` yet.
4. **Two cues still have no producer**: `ImpactWood`/`ImpactMetal` need to
   know WHAT was hit, which the gather toast does not say, and `UiClick`
   needs a hook in the per-screen click handlers.
5. **No occlusion, and it needs a prerequisite rather than a pass.** A wall
   between you and a sound needs a geometry query, and the correct one is
   the sim's (`collide.rs`), not a raycast against render meshes.

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

1. **Nothing is published, so every list is the Direct row.** The url is
   `DECISIONS.md` §open and an operator act: serve `servers.json`, set
   `servers.url` in scry's `data/launcher/gates.manifest.json`. Until then
   both the menu and the launcher's Servers window are correctly dark.
   ⚠ **Measured 2026-08-10: "serve it" had no route to serve it from** —
   `/depot/` is not a `location` on that origin, so the url this script
   printed could only 404. **Closed 2026-08-11 on scry's side**: `GET
   /api/launcher/servers/{slug}` serves `$SCRY_DEPOTS_DIR/<slug>/servers.json`
   byte-for-byte, keeping 404 (publishes none) apart from 503 (could not
   look), and `PUBLISH_URL` here points at it. **One act left and it is on
   the box**: copy the document, then set `servers.url`. In that order —
   `servers.url` stays null until the file lands, because an error dialog on
   a game that is running fine is worse than an honest "no shards published".
2. **Player counts: the code is done end to end; what is left is operator
   acts.** A row may carry `status_url` and both readers poll it every
   `STATUS_POLL_SECS` (`DECISIONS.md` §open "shard status poll v0"); the
   count is never baked into the document, because a generated number is
   stale before it is served. The three remaining steps are all on a box:
   set `status_addr` in `shard.toml`, open that TCP port (the cloud firewall
   too), and put the url in `shards.toml`. Until then every row draws `?`,
   which is correct.
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

## 0q · The gaps nobody has claimed

Lifted out of "done this pass" items before pruning (2026-08-05, again
2026-08-09) — each was written down **only** inside a done item. All of it
is `crates/`/wire work no single-surface lane may take.

1. **Shore barrels as a second destination class.** The road pays unevenly
   now (the bay slots landed) and the haven pad is the one place worth
   walking to. A second class on the shore would give the ring two ends
   rather than one. Nothing else in this file mentions it.
2. **The wipe.** Named by both judges, described nowhere. A shard lifecycle
   act with an economy half (`ALPHA.md` A1→A3) and an operator half
   (`CLAUDE.md`: wipes of a live shard are operator-only), so the loop's
   share is the mechanism, never the trigger. Needs scoping before it can be
   an item.
3. **You cannot stand ON anything.** `movement::step` asks `slot_blocks` and
   nothing asks a ground query for occupants — the shelter's plinth reads as
   a kerb you sink into, crate and boulder tops the same (`terrain.rs`'s
   plinth doc still says "nothing here makes a body stand on the plinth").
   Belongs beside `collide::piece_ground`, a `slot_ground` next to
   `slot_blocks`; the fourteen-box table is already there for it. Systems
   lane.
4. **The 100-bot soak has never been run.** `NETCODE.md` §9's budgets have
   never met 100 real connections: `cargo run -p server --bin bots -- 100`
   against a dev shard, held an hour — tick jitter, WAL append rate,
   per-client bandwidth recorded as counts and bytes, never wall-clock
   asserts (`CLAUDE.md`'s clock rule). The numbers land in a `DECISIONS.md`
   §open row as the measured baseline.

---

## 0r · The raid loop has offence now — what it still cannot do *(systems lane)*

Landed: `sim-core/charge.rs` — plant the held throwable at an address, fuse
from content, damage through the same `damage_piece`/`damage_deploy` a swing
uses (`ACT_THROW`/`EV_CHARGE_PLACED`; knobs `DECISIONS.md` §open "satchel
fuse v0"). X plants it natively and the HUD counts it down. Remaining:

1. **No blast radius** — the content half landed, the arithmetic did not.
   `blast_m` is schema'd, baked to `ThrowDef::blast_cm`, walked into
   `canon::hash`, and **nothing reads it**: a charge damages only the
   address it was planted on. What remains is the falloff and a bounded
   multi-target scan; `combat::raid`'s 3x3 column-index ring is the shape to
   copy. The content hash has already moved, so the cost is paid whether or
   not the slice is taken. Knob: `DECISIONS.md` §open "satchel blast v0"
   (PROPOSED 3 m).
2. **Nothing is hurt by standing in one.** `ThrowDef::damage` is carried and
   `EV_CHARGE_PLACED` has no player-damage half, so the defender's seconds
   are free to spend standing on the charge. A new `DEATH_BY_*` if taken —
   and the 2-bit cause field is saturated since wire v24, so that is a
   widening. Lands with item 1 or not at all — they share the falloff.

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
- **Dropped loot should land somewhere you can find, not inside the floor**
  — and `gather::inv_add` still loses overflow, a documented policy
  (`DECISIONS.md` §open) pointing at exactly this slice.
- **Mushrooms and corn drop now** (2026-08-09, content rows only): the
  tree's secondary pays 1 mushroom a swing — the forest floor through the
  tree that shades it — and the coast-road barrel rolls a 2–4 corn ration.
  `content.rs::every_consumable_the_content_ships_is_reachable` gates the
  general form (every consumable producible by a live verb chain). Still
  owed, and both are code: a standalone forest-floor pickup archetype and a
  farming lane; plus the cache/crate open verb before loot-only food could
  sit at a destination — the gate deliberately counts barrel rows alone.
- **Day/night does not exist.** `DESIGN.md` §2 pairs it with the survival
  clock; nothing in `crates/` reads a time of day.
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
- **No verb opens a world container.** `loot.crate` and `loot.cache` are
  parsed, content-hashed, placed and openable by nothing — barrels are
  smashed, and `Occupant::{CrateSlot, CacheSlot}` appear only in terrain
  placement and collision radii. With the recycler landed this is the
  other half of the same walk: the thing you carry salvage home *from*.

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
    the operator holds no keys and can restore nothing. It is the
    platform's client for the whole cascade, not a Gates accessory.

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
