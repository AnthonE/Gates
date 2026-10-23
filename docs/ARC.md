# Gates · ARC.md — the server levels up

**Spoken 2026-10-07.** Operator: *"Yes lets do all of this. I think this lets
us be Rust but more."* This page owns the wipe arc, the frameworks that carry
it and the order they are built in. `WORLD.md` still owns the fiction (and §11
its secret). `DESIGN.md` owns everything the arc does not touch.

Read §1 and §4 before building any of it. Build the frameworks (§4) before the
meat (§5's later milestones), so content lands in slots that already exist.

---

## 0 · The one idea

Rust's player hits a ceiling in hours and the wipe runs for weeks, so day 20
is day 3 with more guns. Late-wipe surplus can only buy violence, scrap is
spent once, and the world remembers nothing.

**Flip it.** Personal progression stays shallow and crude. The deep, slow
ladder belongs to the **world**: monuments the whole server restores by
pouring materials into them, and each one changes the shard for everybody.
The wipe becomes a story the server writes once: who lit the Crucible, who
opened the Gate, whether the launch went up.

---

## 1 · Rulings (2026-10-07)

What this supersedes is named, so nobody fights it later.

1. **The world starts broken** and players switch it back on (`WORLD.md`
   §5.1, now spoken). The coast stays livable on wipe day (§5.2).
2. **Every unlock is global. Floor guaranteed, ceiling contested.** Each work
   also opens on a posted fallback hour if nobody pays for it. Contributing
   only makes it come sooner. Whoever holds a work gets a bonus on top, never
   a lockout. (`WORLD.md` §4.3, generalised from extraction to every work.)
3. **Guns are gated by the world, not the tech tree.** Gunpowder cannot be
   made until the Crucible is lit, and the ARMS kiosk keeps the revolver and
   its rounds under the counter until then.
4. **Powers are loot.** Strange gear arrives late: it is droppable, runs on a
   charge that only comes from below, and is never on the character sheet.
   This **supersedes `WORLD.md` §7.1** ("player gear stays crude forever").
   There are still no classes, levels or XP (`CONTENT.md`).
5. **The wipe's length follows the ladder:** about one act per week. With two
   acts built, wipes run two weeks. The Descent earns a month.
6. **NPCs talk. Walls carry writing in the ancients' glyphs, and puzzles are
   seeded per wipe** with their hints written on the island. `WORLD.md` §11.3
   still holds: nothing names the SI, the machine or what is under the
   island.
7. **Rust's numbers stay the default** (`reference/BALANCE.md` §6) for
   everything the arc does not touch. The arc's own pacing is ours: gun
   timing, quotas, act lengths and decay.
8. **Expectations are set at first contact.** The first public build already
   shows the arc: the island panel, the sealed steel doors, the Gate's bar.
   That holds even while the later acts stay sealed.

---

## 2 · The wipe, act by act

| act | days (4-week wipe) | the world | what players do | ends when |
|---|---|---|---|---|
| **I · Rats** | 1–3 | broken and dark; only the coast is kind | bows, spears, starter bases; no gunpowder exists | the first act-II work opens for deposits |
| **II · Power** | 3–10 | works come back one by one | feed them (fuel, ore, parts); each lit work unlocks something for everyone and decays if it is not fed | enough act-II works are lit, or the Gate's fallback hour arrives |
| **III · The Gate** | 10–20 | the war effort | one server-wide quota at the Severed Gate, carried there by hand. Countdown, then it opens: sealed doors unseal, things leak out and roam, strange gear starts dropping | the Gate opens |
| **IV · Descent** | 20–28 | the island at its worst | raid the labs under the island; the finale at the Ziggurat pad is a launch, and the launch is the wipe | the wipe |

Days are the 4-week shape. Every number lives in `content/arc.toml` and
`shard.toml`, not here.

---

## 3 · Where every surplus goes

| surplus | destination | who it serves |
|---|---|---|
| **JUNK** | the bank (A2, `DESIGN.md` §3.1) | you; redeemable |
| **bulk** (wood, stone, ore, fuel, parts) | the works | everyone; the public good |
| **sulfur** | **contested**: blow open a base, *or* blow a seal | the choice Rust never offers |

Work quotas are paid in materials, never JUNK. JUNK stays money, and the works
eat what is otherwise worthless late.

---

## 4 · The frameworks

Each framework ships with **one** content row that proves it. The meat is more
rows. Code paths are where each one lives.

### F1 · Works and the arc clock *(sim)* — built
The world-state table `WORLD.md` §5.4 specified. A bounded array of **works**
in `World` (`sim-core/src/works.rs`, `limits::MAX_WORKS`). Each work has:
- **a state:** sealed, open, lit or dark;
- **a progress counter for each input:** deposits clamp at the quota;
- **a condition:** fuel that decays every hour, scaled by population;
- **its timing:** an opening hour and a fallback hour.

The arc clock is the world's tick (a wipe is a fresh world at tick 0). Rows
live in `content/arc.toml`. A work stands at a **spot** (`sim-core/src/spot.rs`):
a site (town, ziggurat or a landmark kind, nearest the middle first) plus an
offset, resolved from the seed on both sides. A seed without that site has no
terminal, and the work lights only at its fallback hour
(`content` test `every_shipped_work_stands_on_the_public_island`). Lighting a
work sets its **unlock flags**, and each work has two:
- **a floor flag:** set once and kept for the wipe;
- **a ceiling flag:** held only while the work's condition is above zero.

Everything is in `state_hash` and the world save (format 21). Ceiling effects
are `[[effect]]` rows turning a knob code (`works::KNOB_*`); the first knob is
furnace speed. First row: **THE CRUCIBLE** at the anvil rock (floor:
`unlock.gunpowder`; ceiling: `unlock.smelting`, furnaces ×2).

### F2 · Unlock gates *(sim + content)* — built
A recipe or a vendor offer can name `unlock = "unlock.x"`, which is refused
until the flag is set (`craft::REFUSE_WORLD`, `vend::REFUSE_V_LOCKED`). Both
ride the wire (six bits on a recipe row and an offer), and the craft panel and
the kiosk name the work that must burn. First rows: gunpowder, and ARMS's
revolver and pistol rounds. Loot tables are not gated yet: a revolver can
still drop from a crate before the Crucible burns.

### F3 · Contributions *(sim)* — built: deposits and the ledger
Depositing at a work's terminal is `Command::Arc` (one wire action, `ACT_ARC`,
with an op, so every later arc verb rides it too), validated for proximity.
Each work keeps a bounded **ledger** (`MAX_WORK_CREDITS`) of each giver's share
in basis points of the quota; each player sees their own share. Still owed:
per-player rewards that shrink with repeats (AQ's signets), and quota windows
so a clan cannot pre-hoard and trigger at 1 a.m.

### F4 · The UI kit and the new screens *(client)* — WORK, ISLAND, banner built
- **The kit:** shared widgets (card over scrim, header, section, progress bar,
  button, row, hint) in `render/panels/kit.rs`, so a new screen is
  composition. The old panels move onto it when next touched. Still owed: one
  palette (the menu, panel and HUD colours disagree), a focus/Escape stack,
  and a panel trait instead of the eight match sites a new `Panel` touches.
- **New screens:**
  - **ISLAND** (`O`): the act, every work's bar, what it gives, your share.
  - **WORK** (`E` at a terminal): its inputs with DEPOSIT, its tank with FUEL.
  - **TALK:** a speaker's lines and replies.
  - **READ:** an inscription, its glyphs rendered as far as you know them.
  - **JOURNAL:** the glyphs you know, what you have read, your notes.
- **The HUD banner** (`render/arc_hud.rs`): top centre under the compass, the
  act and the work that matters most now; a toast to everyone when a work
  opens, lights or goes to embers. Terminals are drawn in the world
  (`render/works.rs`): a black plinth, a fire bowl, a gold ring.
- Models stay in `crate::ui` and are tested headless (`tests/ui.rs`); drawing
  goes in `render/panels`.

### F5 · Speakers: NPCs that talk *(sim + server + client)* — built
People at fixed spots (`sim-core/src/lore.rs`), the town first. Each speaker
in `content/arc.toml` `[[speaker]]` has topics; each topic is a list of lines,
and the first whose `when` holds is said. A `when` is a condition on the
works: `sealed:`, `open:`, `lit:`, `burning:` or `embers:` plus a work, or
`holds:` or `lacks:` plus an unlock.

`E` at a speaker opens TALK and asks the first topic.
- A topic press is `Command::Arc` with `OP_TALK`. The sim checks reach and
  announces it (`EV_ARC_DID`).
- The **server** composes the line against the world now and sends it to
  that player (`SUB_ARC_TEXT`). No string reaches the sim.

Speakers never hand out quests or XP. First rows: THE GATEKEEPER and THE
TRADER at THE GATE. Still owed: replies with a gameplay effect, and speakers
out at the landmarks.

### F6 · Inscriptions and glyphs *(sim + server + client)* — built
The ancients' script is **a fixed alphabet** (`[glyphs]`, A–Z and 0–9), so
learning it is lasting mastery. Each glyph is drawn as a fixed 3×5 shape
(`client/src/ui/glyphs.rs`).
- **The glyph mask** (`Player::glyphs`): bit per glyph, kept across death and
  in the player file (store format 8).
- **Teaching:** reading a stone (`OP_READ`) teaches the glyphs it names
  (`teaches`).
- **The text:** the server composes it, filling each `{mech.x}` with this
  wipe's answer, and sends it only to the player at the stone. READ draws a
  glyph you know as its letter and one you do not as its shape.
- **The journal:** ISLAND's JOURNAL tab shows the whole alphabet as far as you
  read it, and every stone you have read.

Inscriptions are places, never items in a bag (`WORLD.md` §11.3). First rows:
- the gate stone, which teaches ten common letters;
- the count stone at the spires, which teaches the digits;
- the ring word at the arch, which holds the ring lock's answer.

### F7 · Mechanisms: puzzles *(sim + server + client)* — built: dials
A mechanism (`sim-core/src/mech.rs`) is a row of **dials**. Each press turns
one notch (`OP_TURN`), and anyone may turn any dial.
- **The answer** comes from the world seed and a **salt the server draws** for
  each new world, which it saves and never sends. A client that holds the
  seed still cannot compute it.
- **The solve:** the turn that shows the answer pays the turner, rests the
  lock (`rest_minutes`), scrambles the dials, and tells the whole island
  (`EV_MECH_SOLVED`).

Rust's puzzles are on YouTube. Ours are re-seeded every wipe, and their hints
are written on the island. First row: THE RING LOCK at the stone ring, three
dials of eight, paying a green keycard, which is the Ziggurat's first door.

Still owed:
- other devices (lever, socket, plate);
- effects beyond a reward (open a door, add to a work, set a flag).

### F8 · The wipe itself *(server)*
- `wipe_days` in `shard.toml`.
- An admin `wipe-now` (running it on a live shard stays operator-only).
- The arc clock resets at the wipe.
- The player file must survive a balance-only content edit, because today any
  `content/*.toml` change discards it (`server/src/store.rs`).

A month-long wipe with mid-wipe tuning needs all of it.

---

## 5 · Build order

| milestone | what lands | wipe it supports |
|---|---|---|
| **M0 · Framework** | F1–F7, each with its rows; the ISLAND panel and HUD banner; the revolver under the counter. **Built 2026-10-07**, except F8 | the current one |
| **M1 · Rust but more** | act II in full: three to four works from `WORLD.md` §5.3 (Crucible, Observatory, Wells, Spine); a speaker cast at THE GATE; the first glyph trail; F8 | **two weeks**: the first public expectation |
| **M2 · The Gate** | the Severed Gate site and the war effort quota; convoys; the countdown; the opening event; the first roaming things, drawn to noisy bases; sealed steel doors planted everywhere | three weeks |
| **M3 · Descent** | prefab interiors under the terrain behind those doors; the interest grid's vertical layer; strange gear and its charge; extraction at the Gate (A2) | a month |
| **M4 · Launch** | the Ziggurat finale as the wipe; credits across wipes ("opened the Gate, wipe 14") | a month |

---

## 6 · Guardrails

- **Classic AQ and Dune's Landsraad died of pre-hoarding.** Quotas open in
  windows, and rewards shrink with repeats.
- **Power Trip's criticism applies:** groups that can hold three monuments at
  once win. Global floors and fallback hours are the answer; never ship a
  work without both.
- **Crypto paid for pure grinding attracts bots.** Redeemable money stays at
  the end of a risky walk (extraction). Grinding pays in glory: titles,
  credits, cosmetics.
- **Determinism:** works, masks and mechanisms are sim state. They are
  hashed, capped in `limits.rs`, alloc-free in the tick, and use no trig. A
  per-wipe seed must come from the world seed plus a server salt, never from
  a clock.
- **§11 holds.** Dialogue and inscriptions never name the SI, the machine,
  the gate system or what is under the island. Clues are places and
  behaviours.
- **Wire changes** bump `PROTO_VER` and regenerate goldens in the same
  commit.

---

## 7 · Open — the operator's

- Act lengths and quota sizes. Defaults ship in `content/arc.toml` and get
  tuned on a real shard.
- Which act-II works M1 carries, and what each one's ceiling buys.
- The names of the speakers, and how many.
- Whether glyph knowledge survives a wipe the way blueprints do.
