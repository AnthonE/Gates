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
   made until the Crucible is lit. The revolver leaves the ARMS kiosk.
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

Days are the 4-week shape. Every number lives in `content/works.toml` and
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

### F1 · Works and the arc clock *(sim)*
The world-state table `WORLD.md` §5.4 specified. A bounded array of **works**
in `World` (`sim-core/src/works.rs`, `limits::MAX_WORKS`). Each work has:
- **a state:** sealed, open, lit or dark;
- **a progress counter for each input:** deposits clamp at the quota;
- **a condition:** fuel that decays every hour, scaled by population;
- **its timing:** an opening hour and a fallback hour.

The arc clock is ticks since the wipe began. Rows live in
`content/works.toml`. Lighting a work sets its **unlock flags**, and each work
has two:
- **a floor flag:** set once and kept for the wipe;
- **a ceiling flag:** held only while the work's condition is above zero.

Everything is in `state_hash` and the world save. First row: **THE CRUCIBLE**
(floor flag: gunpowder; ceiling flag: faster smelting).

### F2 · Unlock gates *(sim + content)*
A recipe or a vendor offer can name `unlock = "unlock.x"`, which is refused
until the flag is set. `craft.rs` and `vend.rs` each check it in one place,
and the client mirrors the lock in the craft panel. First rows: gunpowder
needs `unlock.gunpowder`, and the ARMS kiosk loses the revolver.

### F3 · Contributions *(sim)*
Depositing at a work's terminal is a player command, validated for proximity.
Each work keeps a bounded **ledger** of what each player gave, which feeds the
island panel's credits and later the rewards. Per-player rewards shrink with
repeats (AQ's signets), and quotas open in time-boxed windows so a clan cannot
pre-hoard and trigger at 1 a.m.

### F4 · The UI kit and the new screens *(client)*
- **The kit:** shared widgets (frame, header, tab bar, progress bar, button,
  list row) in `render/panels/kit.rs`, so a new screen is composition and
  nobody hand-rolls one.
- **New screens:**
  - **ISLAND:** the act, every work's bar, and its credits.
  - **WORK:** a terminal's inputs, with DEPOSIT.
  - **TALK:** a speaker's lines and replies.
  - **READ:** an inscription, its glyphs rendered as far as you know them.
  - **JOURNAL:** the glyphs you know, what you have read, your notes.
- **The HUD banner:** top centre, the act and the nearest bar, plus a world
  notice when a work changes state.
- Models stay in `crate::ui` and are tested headless (`tests/ui.rs`); drawing
  goes in `render/panels`.

### F5 · Speakers: NPCs that talk *(sim placement + client model)*
Static people at fixed, deterministic spots: the town first, the landmarks
later. Dialogue lives in `content/dialogue.toml`:
- lines grouped by speaker;
- each line can carry a condition on the arc's state, e.g. "Nobody's lit the
  Crucible yet";
- replies that only change the conversation.

Speakers never hand out quests or XP. A reply with a gameplay effect is a sim
command, like any other verb.

### F6 · Inscriptions and glyphs *(sim + client)*
The ancients' script is **a fixed alphabet** (`content/glyphs.toml`), so
learning it is lasting mastery. **Messages are seeded per wipe:** the hints
are what change. Each player has a mask of the glyphs they know (blueprint
style), learned at bilingual stones. In the READ screen, known glyphs show as
letters and unknown ones as glyphs. Inscriptions are places, never items in a
bag (`WORLD.md` §11.3). First row: one stone at THE GATE that teaches three
glyphs, and one message at a landmark.

### F7 · Mechanisms: puzzles *(sim)*
**Devices** (dial, lever, socket, plate) are grouped into a **mechanism**:
- its solution is derived from the world seed plus a server-only salt, so the
  client cannot compute it;
- an inscription somewhere else on the island carries the hint;
- on a solve, the mechanism fires **effects**: open a door, add progress to a
  work, set a flag, or spawn a crate.

Device state is shared, so anyone can scramble the dials. Rust's puzzles are
on YouTube. Ours are re-seeded every wipe, and their hints are written on the
island. First row: a three-dial lock whose hint is the F6 message.

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
| **M0 · Framework** | F1–F7, each with its one row; the ISLAND panel and HUD banner; the revolver out of town | the current one |
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

- Act lengths and quota sizes. Defaults ship in `content/works.toml` and get
  tuned on a real shard.
- Which act-II works M1 carries, and what each one's ceiling buys.
- The names of the speakers, and how many.
- Whether glyph knowledge survives a wipe the way blueprints do.
