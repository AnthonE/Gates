# assets/sound — what ships

**Every file here is CC0** (Creative Commons Zero,
<http://creativecommons.org/publicdomain/zero/1.0/>), checked on each
source's own page. Credit is not required and is given anyway.

The files are compiled into the client (`crates/client/src/sound_bank.rs`),
decoded at boot, trimmed and normalized, and played as several takes of one
cue. Cues without a row here are synthesized (`crates/sound/src/synth.rs`).

## `kenney/` — Kenney, *Impact Sounds* 1.0

<https://kenney.nl/assets/impact-sounds> (2019-12-19). Sounds by Kenney
(www.kenney.nl). Five takes each, `_000`–`_004`.

| Files | Cue | Heard as |
|---|---|---|
| `impactMining` | `ImpactStone` | a blow on stone or ground |
| `impactWood_medium` | `ImpactWood` | a blow on wood |
| `impactMetal_heavy` | `ImpactMetal` | a blow on metal |
| `impactPlank_medium` | `Place` | a piece going down |
| `impactSoft_medium` | `BulletSoil` | a round into soil, sand or grass |
| `impactGeneric_light` | `BulletStone` | a round on stone |
| `impactWood_light` | `BulletWood` | a round into wood |
| `impactMetal_light` | `BulletMetal` | a round on metal |
| `impactWood_heavy` | `Knock` | a knock on a door |

## `freesound/` — Freesound, cut here (2026-10-01)

Cut from each sound's high-quality Freesound preview (44.1 kHz Ogg). Each take
is one event, downmixed to mono, high-passed, faded at both ends, peak-
normalized and re-encoded as Ogg Vorbis. Sources are `https://freesound.org/s/<id>/`.

| Files | Cue | Source (author, id, what was used) |
|---|---|---|
| `step_sand_0`–`7` | `StepSand` (+remote) | Nox_Sound, 564893, *Footsteps_Mountain_Boots_Wet_Sand_Sequence_Mono*: walk takes |
| `step_grass_0`–`7` | `StepGrass` (+remote) | Nox_Sound, 556042, *Footsteps_Mountain_Boots_Grass_Mono*: walk takes |
| `step_rock_0`–`7` | `StepRock` (+remote) | Nox_Sound, 558472, *Footsteps_Mountain_Boots_Rock_Walk_Sequence_Mono*: walk takes |
| `step_litter_0`–`4` | `StepLitter` (+remote) | Nox_Sound, 496420, *Footsteps_Leaves_Stereo*: the five walk takes |
| `step_litter_5`–`7` | `StepLitter` (+remote) | Nox_Sound, 530383, *Footsteps_Boots_Gritty_Ground_Woods_Barks_Mono*: walk takes |
| `step_water_0`–`4` | `StepWater` (+remote) | Nox_Sound, 490951, *Footsteps_Walk*: the five average-depth water takes |
| `howl_0`–`3` | `Howl` | betchkal, 500646, *Cooper Creek 20160313_014852 solitary wolf howl very clear* (a wild wolf, Wrangell–St. Elias National Park): its four howls, declicked |
| `growl_0`–`5` | `Growl` | coldvet, 404920, *Dog Growl - Beast / Creature*: six growl phrases, 1.15 s each |
| `hurt_0`–`5` | `Hurt` | unfa, 610991, *Short Male Pain Grunts* |
| `death_0`–`3` | `Death` | unfa, 610998, *Medium Male Pain Grunts*, each over the body hitting the ground from leonelmail, 504626, *BODY FALL - V HVY - DIRT* |
| `flesh_0`–`5` | `FleshHit` | tortillathrilla, 652679, *punches_juicy_processed* |

63 files from Freesound and 45 from Kenney, about 1.4 MB of Ogg in total.
