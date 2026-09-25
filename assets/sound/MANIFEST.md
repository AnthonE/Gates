# assets/sound — what ships

**Every file here is CC0.** Source: Kenney, *Impact Sounds* 1.0
(<https://kenney.nl/assets/impact-sounds>, created 2019-12-19). Licence:
Creative Commons Zero, <http://creativecommons.org/publicdomain/zero/1.0/>.
Credit is not required and is given anyway: sounds by Kenney (www.kenney.nl).

The files are compiled into the client (`crates/client/src/sound_bank.rs`),
decoded at boot, trimmed and normalized, and played as five takes of one cue.
Cues without a row here are synthesized (`crates/sound/src/synth.rs`).

| Files (`kenney/`, takes `_000`–`_004`) | Cue | Heard as |
|---|---|---|
| `footstep_grass` | `StepGrass`, `RemoteStepGrass` | a step on grass |
| `footstep_concrete` | `StepRock`, `RemoteStepRock` | a step on rock |
| `impactMining` | `ImpactStone` | a blow on stone or ground |
| `impactWood_medium` | `ImpactWood` | a blow on wood |
| `impactMetal_heavy` | `ImpactMetal` | a blow on metal |
| `impactPlank_medium` | `Place` | a piece going down |
| `impactSoft_medium` | `BulletSoil` | a round into soil, sand or grass |
| `impactGeneric_light` | `BulletStone` | a round on stone |
| `impactWood_light` | `BulletWood` | a round into wood |
| `impactMetal_light` | `BulletMetal` | a round on metal |
| `impactPunch_medium` | `FleshHit` | a blow or a round into a body |
| `impactWood_heavy` | `Knock` | a knock on a door |

60 files, 564 KB, 44.1 kHz stereo Ogg Vorbis as published (downmixed to mono
at decode).
