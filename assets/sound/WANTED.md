# assets/sound — WANTED

What the sound still wants. What ships, and where every file came from, is
`MANIFEST.md`; the cue list is `crates/sound/src/lib.rs::Cue`.

Every cue is a recording (`crates/client/src/sound_bank.rs`); the synth is
only the fallback for a file that fails to decode. The score is four
ElevenLabs tracks streamed from `assets/music/` (`crates/sound/src/music.rs`).

## Fetching

- **Freesound answers 403 to the cloud box** (2026-10-11). Kenney,
  OpenGameArt, archive.org (the USC sound-effects library, radio aporee),
  nps.gov and Wikimedia Commons answer; Commons rate-limits (429) for hours.
- **Licence rail**: CC0 or public domain; CC-BY with a `NOTICE` credit;
  ElevenLabs on the paid plan (record the plan, the date and the prompt).
  NC and SA are refused, because the game is sold. Read the licence on each
  file's own page. Never anything ripped from a game, xeno-canto or the BBC
  archive.

## Cutting a take

Mono, **44.1 kHz** (the bank refuses any other rate), Ogg Vorbis, peak about
0.85; the bank trims, fades and normalizes each take again at boot. A cue's
takes are `name_0` … `name_n` and play round-robin, never the same one twice
running; a bed is one seamless loop. Every take is compiled into the client,
so the browser downloads it: keep takes short. Write the manifest row with
the source, author, licence, and what was cut from where.

## Still wanted

| Cue | Today | Wanted |
|---|---|---|
| `BedRotor` | a 22 s two-blade loop from a high-passed tape, its thump rebuilt | a full-band close two-blade hover with its own low end |
| `BedUnder` | a 12 s loop | a ≥ 30 s underwater loop. Leads: Nox_Sound 482116 (Freesound, CC0); gmason's OpenGameArt rumble (CC0, darker than today's) |
| `Bellow` | a real red deer at medium distance | a close roar. Leads: jorickhoofd 169540 / 169539 (Freesound, CC-BY) |
| `Bird` | twelve phrases, European and North American | more European single phrases |
| `ImpactStone`, `ImpactMetal`, `Place`, `Bullet*`, `Knock` | Kenney's *Impact Sounds* | real recordings. USC (archive.org, CC0): *R28-57 Dig Tunnel with Pick and Shovel*, *Metal barrel being hit with wooden hammer* |
| `Hurt`, `Death` | one male voice | a second voice |
