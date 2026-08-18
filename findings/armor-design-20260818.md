# armor-design-20260818.md — equipment v0, planned against the tree

Written 2026-08-18 by a read-only design agent. Every claim below carries a
`file:line` read that session; no cargo command was run (another lane was
compiling).

⚠ **Independently re-checked before filing** — the five load-bearing claims,
by the commands rather than by trust: `CONT_SELF/BAG/BOX/WORLD` = 0/1/2/3 with
`CONT_MAX = CONT_WORLD` (`inventory.rs:57/62/78/97/103`); `CONT_KIND_BITS = 2`
in **both** `protocol/src/event.rs:320` and `protocol/src/lib.rs:1137`;
`grep -rniE 'damage_type|damagetype|dmgtype' crates/ content/` → **0**;
`grep -rn armor crates/sim-core/src/` → **2**; `grep -ni armor
crates/content/src/bake.rs` → **0**. Anchor 2's `for a in &c.armors` loop is
at `balance.rs:117` and does credit a *head* piece against *body* hits, as
§5(a) says. The rest is the agent's, at its stated provenance.

## 0 · Two corrections to `reference/ARMOR.md` before anything else

Both are the dead-citation class `CLAUDE.md` warns about; both claims survive,
both pointers rotted.

1. §9.1 and `NOW.md` say `grep -rn armor crates/sim-core` returns **one**
   comment. It returns **two lines** — `crates/sim-core/src/combat.rs:37` and
   `:38` — one sentence, expanded on 2026-08-18 to cite `ARMOR.md` §9.1 back.
   Substance unchanged. (That expansion was this session's own edit, which is
   how a doc's count goes stale inside a day.)
2. §9.2 cites `protocol/src/event.rs:279` for `CONT_KIND_BITS`. That line is
   now `const SUB_IMPACT: u32 = 51;`. The constant is at
   **`crates/protocol/src/event.rs:320`**. The value (`2`) and the claim are
   correct.

Everything else §9.1 cites is live and exact: `schema.rs:317`, `schema.rs:26`,
`validate.rs:507`, `canon.rs:170`, `balance.rs:117`.

## 1 · What `content/armor.toml` says, and what survives the bake

Four columns, three rows (`content/armor.toml:52-68`), shape at
`crates/content/src/schema.rs:324-329`:

| column | type | consumer today |
|---|---|---|
| `id` | item id | `validate.rs:509` (item-backed), `canon.rs:172` |
| `slot` | `ArmorSlot {Head, Body}` (`schema.rs:317-321`) | `validate.rs:511-517`, `canon.rs:173` |
| `reduction_pct` | u32, ≤ 90 | `validate.rs:518-520`, `canon.rs:174`, `balance.rs:118` |
| `move_penalty_pct` | u32 | `canon.rs:175` — **and nothing else, anywhere** |

Rows: `armor_burlap_head` head/10/0, `armor_burlap_body` body/15/0,
`armor_roadsign_body` body/25/5.

**Baked into a table the sim could read today: none of it.**
`grep -n -i armor crates/content/src/bake.rs` returns zero lines.
`Content.armors` (`crates/content/src/lib.rs:148`, filled at `:222`/`:239`) is
parsed, validated, folded into the content hash and balance-checked — and then
dropped on the floor at bake. Wall 7 says the sim reads a baked table, so the
honest sentence is: **there is no baked armor table and no struct to hold one.**

**Validated but discarded at bake, second instance:** `Item::slot` —
`EquipSlot {Hand, Head, Body, None}` (`schema.rs:26-31`), on every item row.
`validate.rs:511-517` reads it only to assert the armor row's slot agrees with
the item's; `bake.rs` never reads it. So the sim does not know which items are
wearable either, and `CanWearItem`'s data source is presently unbaked.

**The struct wall 7 names is `sim_core::combat::CombatContent`**
(`crates/sim-core/src/combat.rs:234-253`) — built by
`content::Content::bake_combat` (`crates/content/src/bake.rs:537`), installed
at `crates/server/src/net.rs:298`. It already carries `player_hp` and three
item-indexed tables (`melee`, `throw`, `ranged`, `ammo`), all on the bake's
sorted-rank item index, and every player-damage site already holds a
`&CombatContent`. Armor goes there:

```
pub armor: [ArmorDef; MAX_ITEM_DEFS],   // MAX_ITEM_DEFS = 64, limits.rs:203
pub wear_slot: [u8; MAX_ITEM_DEFS],     // EquipSlot, finally baked
pub dtype: [u8; MAX_ITEM_DEFS],         // §3
```

No new content-plumbing shape is invented: this is exactly how `bake_ammo`
(`bake.rs:734`) installs a row keyed by the round's item index.

## 2 · Every route a player loses hp, and whether armor reduces it

Seven, not five. The four hit routes each hand-copy the same damage liturgy
(hp debit, deaths counter, `EV_HEALTH`, `EV_DEATH`, `die`); `world.rs:3357`
calls it "`combat::strike`'s exact damage liturgy" in its own comment, which is
the tell that it is copied rather than shared.

| # | route | site | armor? | why |
|---|---|---|---|---|
| 1 | melee swing | `combat.rs:536-537` | **YES** | slash/blunt/stab by weapon |
| 2 | arrow | `ranged.rs:457-458` | **YES** | bullet. This site already computes the hit height and throws it away (`ranged.rs:442`) |
| 3 | blast | `charge.rs:539-540` | **YES** | explosion. The one site where coverage must be *weighted across all areas* — an explosion genuinely reaches every area at once |
| 4 | mob bite | `world.rs:3368-3369` (buffered at `mob.rs:800`) | **YES** | bite. Also the strongest reason boots/legs would ever be a decision |
| 5 | starve / dehydrate | `survival.rs:308-315` | **NO** | metabolic, not a hit. A chest plate does not feed you, and `ARMOR.md` §6 says only player damage wears worn items — the clock is not a player. Farming in armor must not cost a repair bill |
| 6 | salt water | `survival.rs:572-573` | **NO** | the cost *is* the price of the drink; reducing it would make armor a desalinator |
| 7 | keypad shock | `deploy.rs:1980` → `lock.rs:516` | **NO** | it floors at 1 hp and never kills (`lock.rs:57`). Reducing it makes an armored raider immune to a mechanic whose entire job is to cost tries |

Route 7 is in neither `ARMOR.md`'s list nor the brief's. It is the proof that
"add an arm at each site" loses.

### Where the single reduction function must sit

**`crates/sim-core/src/combat.rs`**, beside `CombatContent`, because
`player_hp` already lives there and all four hit routes already import it:

```rust
pub struct Hurt { pub dealt: u16, pub absorbed: u16, pub died: bool }
pub fn hurt(cc: &CombatContent, v: &mut Player, raw: u16, dt: DamageType,
            events: &mut EventQueue) -> Hurt;          // reduced
pub fn hurt_unreduced(cc: &CombatContent, v: &mut Player, raw: u16,
                      events: &mut EventQueue) -> Hurt; // routes 5-7
```

Both share one body. The *choice* is then a function name at the call site,
visible in a diff, rather than an omission that is invisible by construction.
Routes 5–7 each get a one-line comment naming why — an unexplained omission
reads as an oversight the next time somebody greps.

**How it cannot be forgotten by the next damage source.** Rust cannot make
`Player::hp` private within its own crate, so the enforceable version is this
repo's own idiom — a source-reading gate, the shape of `tests/persist.rs:826`
(`every_player_field_is_classified_across_a_death`) and `tests/event_roles.rs`.
New `crates/sim-core/tests/damage_routes.rs`: scan `src/*.rs` for writes to a
`Player`'s `hp` (`hp -=`, `hp =`, `hp.saturating_sub`), require every site to
be inside `combat.rs` or on a named allowlist with its reason string, and fail
**loudly** on a site it cannot classify. `CLAUDE.md`'s `pop_*` entry is
explicit that such a list must be *derived from the surface*, never hand-kept —
the nine-against-fourteen drift is the precedent.

Arithmetic, wall 1: `dealt = raw - (raw as u32 * pct / 100) as u16`. Integer
division, no float, no libm.

## 3 · Damage types: they do not exist anywhere

```
grep -rniE 'damage_type|damagetype|dmgtype|dtype' crates/ content/   →  0 lines
```

Not in `sim-core`, not in `content/`. `weapons.toml`'s `kind` is
`WeaponKind {Melee, Bow, Firearm, Throwable}` — a *weapon* kind that steers the
bake (`bake.rs:672` takes bows, and `combat.rs:54` records that firearms fall
through), not a damage type. `Weapon` carries `kind`, `damage`, `structure`,
`headshot_mult` and nothing else.

**So a damage type on a hit is the first thing to build.** `ARMOR.md` §9.3's
ordering holds and is not repealed by the operator's maximal-scope pick —
`DECISIONS.md` 2026-08-17 says so itself ("what the doc's reasoning still buys
is the *ordering*, which does not change").

Take `DECISIONS.md` §open's spoken set of **six**: slash, blunt, stab, bullet,
explosion, bite. Radiation, cold, heat, electric and falling are omitted
because nothing in this world deals them — shipping columns nothing can key is
the *paid nobody* failure recreated in the schema. That omission also keeps
`ARMOR.md` §3's radiation footnote (subtractive **on a rate**, where damage is
proportional) out of the schema entirely, which is right: a percentage field is
the wrong shape for it twice over.

The type must ride *with* the damage from the baked table, never be re-derived
at the site: `Bite` (`mob.rs`) and `Arrow` (`ranged.rs`) already carry `damage`
as a copied field, and the type is one more `u8` beside it.

## 4 · `CONT_WEAR`: §9.2 verified, and priced

**Counted rather than trusted.** `crates/sim-core/src/inventory.rs`:

| value | const | line |
|---|---|---|
| 0 | `CONT_SELF` | `:57` |
| 1 | `CONT_BAG` | `:62` |
| 2 | `CONT_BOX` | `:78` |
| 3 | `CONT_WORLD` | `:97` |
| — | `CONT_MAX = CONT_WORLD` | `:103` |

`inventory.rs:98-102` says it in as many words: *"There is no forgeable kind
left: every value of the field is now a real container."* `CONT_KIND_BITS = 2`
at **both** `protocol/src/lib.rs:1137` and `protocol/src/event.rs:320`.
**§9.2 is correct.** All four values are spent; `CONT_WEAR = 4` needs three bits.

### The price

- `CONT_KIND_BITS` 2 → 3, **in both files in the same commit**.
  `event.rs:317-319` states why: the two lanes carry the address identically
  "or neither is sound". Widening one is the failure that comment names.
- Layout moves on three messages: `ACT_CONTAINER` (`lib.rs:1443`), `ACT_MOVE`
  (`lib.rs:1474` **and** `:1476` — the field appears twice), `SUB_CONT_SYNC`
  (`event.rs:2111`, decode `:3131`).
- **`PROTO_VER` 48 → 49** (`lib.rs:645`). All **96** goldens re-key:
  `tests/golden/v48_*.bin` → `v49_*`, and `tests/protocol_golden.rs:60`
  (`const GOLDEN: [&[u8]; 96]`) moves with them. Same commit, wall 6.
- **New fixtures: three, not one.** v37's own record (`lib.rs:485-489`, and
  `goldens::action_move_box`'s doc) is that when the third kind landed only its
  *open* was pinned and "the bytes meaning *take it out* went a whole version
  unchecked". Wear needs: a move **into** a wear slot, a move **out of** one,
  and its opening sync.
- **Free, and do not delete:** the `kind > CONT_MAX` guards at `lib.rs:1462`,
  `lib.rs:2053`-area, `event.rs:2100` and `event.rs:3138` start refusing again
  (values 5–7 become forgeable). `lib.rs:1115-1132` predicted exactly this —
  *"they are the thing that starts working again the moment this width is
  widened ahead of the kind set — which is the next container kind's first
  move."*
- **Free:** `REFUSE_M_WEAR = 9`. `REFUSE_M_BITS = 4` (`event.rs:325`) holds
  1..=15 and `REFUSE_M_MAX` is 8 (`inventory.rs:173`). (`event.rs:322`'s
  comment still says the reasons "run 1..=7" — stale since oven v0; correct it
  while you are there.)
- **Cheap, and the reason §9.2 is right:** `slots_in` (`inventory.rs:110`)
  grows one arm, `CONT_WEAR => WEAR_SLOTS`, and `client/src/ui/slots.rs:56`
  **re-exports it rather than mirroring it**, so both ends move together by
  construction. The check that stops a drop on box slot 20 stops a helmet in
  the boots slot, for free.
- `WEAR_SLOTS` is a cap and belongs in `limits.rs` beside `BOX_SLOTS` (`:429`)
  with a stated overflow policy — wall 4.

### What §9.2 does **not** mention and costs real work

`Player` grows `worn: [ItemStack; WEAR_SLOTS]`, and that is walls 5 and 4, not 6:

- **`state_hash`** (`world.rs:3495`): a sibling loop after `for s in p.inv`
  (`world.rs:3564-3570`). It must be its own loop, appended —
  `world.rs:3620-3633` records that a store hashed unconditionally moved
  `GOLDEN_FINAL_HASH` and cost eight zero bytes. `worn` is per-player and
  always present, so **it will move `test_replay`'s pinned hash.** Regenerate
  deliberately and say so in the commit; do not discover it.
- **`PlayerSave`** (`persist.rs:148-206`): `PLAYER_SAVE_BYTES`
  (`persist.rs:64`) is pinned at **256** by name in `tests/persist.rs:506`.
  Two wear slots → 268; eight → 304. Bump `store.rs`'s `SAVE_FORMAT` in the
  same commit.
- **`tests/persist.rs:826`** goes red the instant `worn` is declared, until
  `die`/`wake` classify it. **That is the gate you want** — it forces the "does
  a corpse drop what it was wearing" decision into a test rather than a
  reviewer's attention. Recommendation: worn goes into the death bag with the
  inventory (`backpacks.drop_for`, `world.rs:1897`), because armor as loot is
  what makes killing an armored player worth doing.
- `Player` derives `Copy` and is copied by value in `World::die`
  (`world.rs:1896`), so `crate::boxed_array` is unavailable. `DECISIONS.md`
  2026-08-17 already measured the wasm shadow-stack exposure: +48 B/player,
  `World` ~302 → ~307 KiB against 1 MiB — 1.6 %, not the thing that tips it.
  `test_parity_wasm` is the gate that would say otherwise.

## 5 · What breaks in balance

### The honest answer: **nothing goes red, and that is the defect**

`crates/content/src/balance.rs:95-130` computes anchor 2 purely from
`content/*.toml`. Armor being *applied* changes no content number, so
`test_content` stays green while its meaning rots. **This is `WORLD.md` §8.2's
ward collision, one system over and no longer conditional** — the doc says it
in its own words: *"The gate does not go red — it goes quietly wrong, which
this repo already knows is the worse failure."* The ward is hypothetical; armor
is spoken and scheduled.

Anchor 2 is quietly wrong in three separate ways, and only the third produces a
red:

**(a) It is slot-blind.** `balance.rs:117-128` loops **every** armor row
against a **body** hits-to-kill. It credits `item.armor_burlap_head` — a head
piece — with reducing body hits. Under any coverage model that is false. Under
the *current* sim, which has no hit areas at all (`combat.rs:36`: *"aim is
planar… there is no head to hit"*), it is false in the other direction: a head
piece reduces nothing at all, ever. **So `item.armor_burlap_head` is still dead
content on the day armor v0 ships**, unless the model sums both slots. That is
the audit repeating itself one level in, and it is the thing to decide before
writing code.

**(b) It is a ceiling with no floor.** A worn set that adds *zero* hits
satisfies `armor_extra_hits_max` perfectly. Armor could be entirely decorative
with every gate green.

**(c) It cannot see a set.** `armor_extra_hits_max = 2`
(`content/balance.toml:63`) is per *piece*, by its own comment. Combined head
10 % + roadsign 25 % = 35 %:

| weapon | dmg | base | +armor set |
|---|---|---|---|
| rock | 20 | 5 | **+3** |
| spear_wood | 20 | 5 | **+3** |
| revolver | 20 | 5 | **+3** |
| hatchet/pickaxe stone | 25 | 4 | **+3** |
| metal tools, spear_metal, bow | 30 | 4 | +2 |
| crossbow | 35 | 3 | +2 |

(Computed off `content/weapons.toml` and `globals.player_hp = 100` with
`balance.rs:52`'s own `hits_to_kill`. Under the `RIPLIST.md` §1h take — head 15
/ shirt 10 / roadsign 20 — the same three break at +3; under multiplicative
stacking, rock/spear_wood/revolver still break.)

**So: the anchor that goes red is anchor 2's `armor_extra_hits_max` clause —
the moment it is rewritten to be honest.** Not before. `+3 > 2` on four weapons.

### The honest fix

Not one band. Three, and they are `DECISIONS.md` §open "equipment v0"'s own
list, verified here:

1. **A head TTK band.** `content/balance.toml:59-61`'s `headshot_mult` band
   exists, in its own words, *"so a data edit can't quietly make headshots
   one-tap past the TTK band"* — and it pins the multiplier while pinning no
   head TTK, because nothing ever applied it (`grep -rn headshot
   crates/sim-core/src` → three comments saying there are none). The day the
   multiplier is baked, crossbow 35 × 2 = 70 kills in **2** head hits against
   100 hp while `ttk_bow = [3, 4]` stays green.
2. **An armor minimum.** Closes (b).
3. **A full-set ceiling.** Closes (c) — and this is the one that goes red at +3
   and needs an operator re-speak of `armor_extra_hits_max` (2 → 3) *or* a
   re-priced ladder. `DECISIONS.md` §open "balance bands", not a code edit.

Plus a fourth thing that is not a band: **`hits_to_kill` and the sim's reducer
must be the same arithmetic.** `balance.rs:52` computes
`ceil(hp*100 / (damage*(100-pct)))` — an exact division. The sim will compute a
per-hit integer floor and subtract. Those are not the same function, and if
they disagree the band describes a fight nobody has. Assert them equal for
every (weapon, armor, type) pair in `test_content`.

### Already-red, unrelated to application

`crates/content/tests/content.rs:518-521` (`band_breaks_refused`) anchors on
the literal string `reduction_pct = 25` to mutate it into a band break. It
reddens the moment the armor numbers move — the fixture rot already filed in
`content/armor.toml:34-42` and `DECISIONS.md`'s §1h row. `RIPLIST.md` §5 step 4
puts it in the same commit as the numbers.

## 6 · The item-move trap, specifically

`CLAUDE.md`'s entry: three Oxide fixes in 28 minutes, the third a fix of the
fix, all landing as **the server disconnecting the client**, because container
state diverged; the bug is *validation ordering against the mutation*, never
arithmetic; and prediction makes it worse because the client has already drawn
the move.

Seven things this plan does about it, each pointing at a line:

1. **No `ACT_WEAR`, ever.** `CONT_WEAR` is a *kind*; the verb stays `ACT_MOVE`,
   so the whole six-step refusal ladder at `world.rs:1683-1832` applies
   unchanged. A second path into container mutation is the entire failure.
2. **`plan_move` and `resolve` (`inventory.rs:203`, `:256`) are not touched** —
   and *cannot* be. `plan_move` is handed `ItemStack` by value and returns a
   plan; it holds nothing it could corrupt (`inventory.rs:12-26`). The wear
   rule is a content predicate and `plan_move` is given no content by design
   (`world.rs:1801-1807` states this in full).
3. **The one new rule goes exactly where `REFUSE_M_OVEN` goes** —
   `world.rs:1808-1818`: after the address resolves, before `plan_move`, asked
   of the **source item by value**, against the baked `wear_slot` table.
   `REFUSE_M_WEAR = 9`, free.
4. **The mutation stays two writes.** `set_cont_slot` (`world.rs:1630-1647`)
   grows one arm, `CONT_WEAR => self.players[slot].worn[s]`, mirroring
   `cont_slot` (`:1620`). Both writes still land below the last `return`
   (`world.rs:1832-1834`).
5. **Prediction: there is none to make worse, and that must stay true.**
   `client/src/render/panels/inv.rs:51-58` forbids an optimistic container copy
   *by name* — "There is no optimistic copy of a container anywhere in this
   file" — and the drag's source cell only lights up rather than emptying.
   `client-core` holds only the server's shadow (`core.rs:836`). The wear grid
   inherits this verbatim: it draws `core.worn`, which only `SUB_CONT_SYNC`
   writes.
6. **The real new divergence risk is the subscription, not the verb.**
   `open_cont_kind` (`server/src/client.rs:199`) and `cont_kind`
   (`client-core/src/core.rs:832`) are **single-slot on both ends**, and
   `CONT_SELF` on `SUB_CONT_SYNC` is reserved as the CLOSE sentinel
   (`core.rs:1706-1711`, `event.rs:2103`). Wear therefore needs its **own
   always-on shadow on both ends** — `last_wear` beside `last_cont`
   (`client.rs:204`), `worn` beside `cont` (`core.rs:836`) — never the
   open-container slot. Otherwise opening the wear grid evicts the box you are
   dragging out of, and that *is* container-state divergence, arriving through
   the panel instead of the verb. No new event subtype is owed:
   `encode_event_cont_sync` already takes a kind and already bounds itself with
   `slots_in(kind)`.
7. **One predicate, both ends.** `ui/slots.rs:245-250` rule 5 requires a
   **non-zero handle** for any kind ≠ `CONT_SELF`; `world.rs:1710-1714`
   resolves `ground` on the same assumption. `CONT_WEAR` has no handle. Add
   `inventory::is_own(kind) -> bool` (`CONT_SELF | CONT_WEAR`) and use it at
   **both** sites, or the server refuses a move the client was allowed to draw
   — precisely the "computed on different values" defect. And
   `CONT_WEAR ↔ CONT_BOX` must refuse as `REFUSE_M_NO_CONTAINER`, for the same
   one-handle-field reason bag↔box does (`inventory.rs:144-152`), *refused
   locally* so no round trip is spent.

## 7 · Staging

Each slice lands green alone. Cheapest first.

**S1 · The funnel. No content change, no wire change, no behaviour change —
the one worth landing by itself.**
Introduce `combat::hurt` / `hurt_unreduced` and convert all seven sites (§2).
Armor is not read; nothing reduces yet. Ship `tests/damage_routes.rs`, the
derived source gate.
*Gate it earns:* `test_replay`'s pinned hash is **unchanged** — that is the
proof this refactor changed nothing, and a stronger claim than any assertion
you could write. Plus the new route gate, proven red by deleting one call site.
*Why alone:* it is the only piece that gets structurally harder with every
damage source added, and it costs nothing to review because the hash is the
review. After it, armor is a one-line insertion inside one function and the
next damage source is born reduced.

**S2 · Damage types. Content + bake, consumed by the funnel.**
`damage_type` on every `weapons.toml` row (six values), `schema` + `canon` +
`validate` + `bake_combat::dtype`. `hurt` takes it and still does nothing with
it. `content/armor.toml`'s `reduction_pct` → per-type vector in **this** slice,
taking `RIPLIST.md` §1h's five columns whole, with `content.rs:518-521`'s
fixture re-anchored in the same commit (`RIPLIST.md` §5 step 4). One content
rewrite, not two — `ARMOR.md` §9.3's warning paid.
*Gate:* `test_content` green with the vector; a per-type band break refused by
fixture.

**S3 · Armor reduces. Still no wire bump.**
`bake_combat::armor` + `wear_slot`; `Player::worn` (walls 4/5: `limits.rs`,
`state_hash`, `PlayerSave`, `store.rs` format, the field ledger at
`persist.rs:826`); the reduction inside `hurt`. Nothing can *equip* yet — the
only filler is content: a `[[spawn_wear]]` table in `balance.toml`, wall-7
clean and replay-safe for exactly the reason `SpawnKit` is
(`inventory.rs:308-319` — the content hash is already in the WAL header).
**A player spawns in a burlap shirt and the rock takes 6 hits instead of 5.**
Armor does something, felt, with the wire untouched.
*Gate:* a sim test writing `worn` directly and proving each of the five reduced
routes reduces and the two unreduced routes do not. `test_replay`'s hash moves
here, deliberately.

**S4 · `CONT_WEAR` — the wire bump.** §4 in full. `PROTO_VER` 48 → 49, 96
goldens re-keyed, 3 added, `slots_in` arm, `REFUSE_M_WEAR`, `is_own` on both
ends, the wear shadow on both ends, the grid in the panel. This is what makes
armor *equippable* rather than *effective*.
*Gate:* `test_protocol_golden`; `tests/inventory_move.rs` extended with
wear-in, wear-out, wrong-slot, and wear↔box.

**S5 · The three balance bands.** §5. Head TTK band, armor floor, full-set
ceiling; `hits_to_kill` slot-aware and pinned against the sim's own reducer.
**Needs an operator word first** — `armor_extra_hits_max` breaks at +3.

**S6 · Hit areas / coverage.** The geometric band on the capsule
(`DECISIONS.md` §open has the four heights as proposed defaults). Until this
lands, `item.armor_burlap_head` pays nobody however S3 is written — say that in
the commit rather than letting it be discovered.

**S7 · Condition on armor** (`ARMOR.md` §9.4 — `ItemStack.cond` already exists,
`gather.rs`), **then the 25 % broken floor**, **then `move_penalty_pct`** (no
consumer exists anywhere in `movement.rs`; its non-stacking rule is a combining
rule a content file cannot state and a knob must — `ARMOR.md` §9.5 item 4).
