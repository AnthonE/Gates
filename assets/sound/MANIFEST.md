# assets/sound — what ships

**Every file here is CC0** (Creative Commons Zero,
<http://creativecommons.org/publicdomain/zero/1.0/>), checked on each
source's own page. Credit is not required and is given anyway.

**Every cue the game plays is one of these files; nothing is synthesized any
more.** They are compiled into the client (`crates/client/src/sound_bank.rs`)
and decoded at boot. A one-shot cue's takes are trimmed, faded and
normalized, and played round-robin, never the same take twice running. A bed
is one seamless loop and a piece of music one 10.5 s file on the director's
grid, and both are played whole. `sound::synth` is only the fallback for a
file that fails to decode, and `crates/client/tests/sound.rs`
(`every_cue_is_a_recording`) fails if any cue has no file.

## `kenney/` — Kenney

### *Impact Sounds* 1.0, verbatim

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

### *Interface Sounds*, *UI Audio*, *RPG Audio* (2026-10-03)

<https://kenney.nl/assets/interface-sounds>, <https://kenney.nl/assets/ui-audio>,
<https://kenney.nl/assets/rpg-audio>, each pack CC0 (its `License.txt`).
Re-encoded mono 44.1 kHz at peak 0.85 (some originals run over full scale,
which the decoder would clip), lightly high-passed, otherwise whole.

| Files | Cue | Source |
|---|---|---|
| `ui_click_0` | `UiClick` | Kenney, *UI Audio*: `Audio/click5.ogg` |
| `hit_0` | `Hit` | Kenney, *Interface Sounds*: `Audio/tick_004.ogg` |
| `hit_head_0` | `HitHead` | Kenney, *Interface Sounds*: `Audio/glass_001.ogg` |
| `hit_limb_0` | `HitLimb` | Kenney, *Interface Sounds*: `Audio/drop_003.ogg` |
| `refused_0` | `Refused` | Kenney, *Interface Sounds*: `Audio/error_006.ogg` |
| `craft_done_0` | `CraftDone` | Kenney, *Interface Sounds*: `Audio/confirmation_001.ogg` |
| `trade_0` | `Trade` | Kenney, *RPG Audio*: `Audio/handleCoins.ogg` |
| `learn_0` | `Learn` | Kenney, *RPG Audio*: `Audio/bookFlip2.ogg` |
| `door_open_0`–`1` | `DoorOpen` | Kenney, *RPG Audio*: `Audio/doorOpen_1.ogg, doorOpen_2.ogg` |
| `door_close_0`–`3` | `DoorClose` | Kenney, *RPG Audio*: `Audio/doorClose_1..4.ogg` |
| `gather_0`–`2` | `Gather` | Kenney, *rpg-audio*: `Audio/dropLeather.ogg`; Kenney, *rpg-audio*: `Audio/cloth2.ogg`; Kenney, *rpg-audio*: `Audio/cloth3.ogg` |
| `equip_0`–`2` | `Equip` | Kenney, *RPG Audio*: `Audio/handleSmallLeather.ogg`, `handleSmallLeather2.ogg`, `cloth4.ogg` |
| `map_paper_0`–`1` | `MapPaper` | Kenney, *RPG Audio*: `Audio/bookFlip1.ogg` (its last flip, from 0.28 s), `bookFlip3.ogg` |
| `container_open_0`–`2` | `ContainerOpen` | Kenney, *RPG Audio*: `Audio/metalLatch.ogg` laid 60–70 ms ahead of `creak1.ogg`, `creak3.ogg` and `creak2.ogg` (−2 to −3 dB), one creak per take |

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

### 2026-10-03

Same cut as above, from each sound's Freesound HQ preview, with every page
confirmed CC0 at download. Where a row names several recordings, the take is
those real recordings layered or placed in sequence (a bandage's rip then
its wrap, a tree's crack then its fall). The rain bed and the fire crackles
were additionally gained up into a look-ahead limiter (+12 dB and +8 dB,
the loop limited across its own seam) so the bank's peak normalization
leaves them dense rather than a few loud drops. The town loop's crossfade
compensates for its very steady engine, which an equal-power crossfade
would have swelled at the join. The flyby is a whip crack, a small sonic
boom: no clean CC0 recording of a real bullet passing exists.

| Files | Cue | Source (author, id, what was used) |
|---|---|---|
| `gate_chime_0` | `GateChime` | ryuuzan, 192761, *Ryuuzan_shop_door_bell00.wav*: the double ding, 0-0.8 s |
| `sentry_lock_0` | `SentryLock` | Ultra-Edward, 840450, *Keypad*: three of its beeps (an alarm clock recording, pitched up by the author), cut to 0.1 s and set 0.13 s apart |
| `unlock_0` | `Unlock` | Ultra-Edward, 840450, *Keypad*: two of its beeps, 0.1 s apart |
| `shot_gun_0`–`3` | `ShotGun` | klangfabrik, 232750, *357 Magnum Ext.wav*: takes 0-3: four of the eight exterior .357 shots, onsets at 0.140, 2.519, 8.530 and 15.566 s, each cut 3 ms before the blast to +0.92 s |
| `shot_gun_far_0`–`2` | `ShotGunFar` | kingsrow, 431904, *Gunshots.wav*: takes 0-2: distant shots at 10.455, 25.485 and 28.129 s (source span 1.75 s each, incl. the late echo ~1.4 s after the shot) |
| `shot_bow_0`–`2` | `ShotBow` | Hanbaal, 178873, *bow2.wav*: take 0: release and string ring 0.0-0.42 s (20 lb bow, close); matthewHoldenSound, 542559, *OWI_Bow string thwang.wav*: take 1: string thwang 0.437-0.85 s; matthewHoldenSound, 542517, *OWI_Bow Srting 2.wav*: take 2: string thwang 0.657-1.06 s; bruno.auzet, 527435, *arrow cutting through the air.wav*: arrow whoosh layer, one per take, at -7 to -14 dB: whooshes at 1.862, 8.862 and 5.315 s |
| `bow_draw_0`–`1` | `BowDraw` | Paveroux, 490556, *Bow Drawn*: take 0: the main creaking draw, 0.0-0.8 s; EminYILDIRIM, 536067, *Bow Loading*: take 1: whole file, 0.02-0.47 s |
| `reload_0`–`1` | `Reload` | Dredile, 177863, *Clean Revolver Reload*: take 0: events at 0.298 (cylinder out), 1.056 and 1.43 (rounds in), 2.00 s (cylinder shut), re-spaced into 1.03 s; tkane0512, 722902, *Revolver Reload Break 2*: take 1: break 0.12, speed-loader 1.024 and 1.376, close 2.258 s, re-spaced into 0.94 s |
| `ricochet_0`–`3` | `Ricochet` | cedarstudios, 148840, *ricochet.mp3*: takes 0-3: the four ricochets in the file, at 0.000, 0.435, 0.850 and 1.362 s |
| `fuse_0` | `Fuse` | elonen, 45650, *fuse_burning_48khz.wav*: 0.03-7.75 s at 0 (fade 0.6 s) plus 2.00-4.85 s at 7.12 s to reach 9.97 s |
| `torch_out_0`–`1` | `TorchOut` | cut here from two files above: `fuse_0` (elonen, 45650) 1.20-2.10 s / 4.40-5.30 s, high-passed 900 Hz and faded out over 0.78 s, over the first 0.35 s of `fire_crackle_2` / `fire_crackle_5` (ahriik, 508110), high-passed 300 Hz |
| `bellow_0`–`1` | `Bellow` | cut here from two files above, a stand-in until a CC0 red deer roar can be fetched (Freesound is blocked from the box that cut it): `growl_0` / `growl_2` (coldvet, 404920) slowed to 0.62× (pitch and length), over the first 1.1 s of `howl_0` / `howl_1` (betchkal, 500646) slowed to 0.5× at −5 dB, band-limited 60–2200 Hz, 1.7 s with a 0.55 s fade out, peak 0.85 |
| `flyby_0`–`3` | `Flyby` | G.Lamont, 118068, *Whip Cracks.WAV*: takes 0-3: whip cracks at 19.793, 23.994, 17.685 and 9.569 s, cut 3 ms before the crack to +0.30 s, excluding the swish before it |
| `blast_0`–`2` | `Blast` | felix.blume, 251401, *Dynamite explosion in the mountain*: take 0: the single dynamite blast (onset 0.058 s) with its mountain roll, 0.054-3.554 s; alienistcog, 125937, *merrimack-st-demolition.aif*: take 1: the main demolition charge (onset 10.04 s), 10.036-12.9 s; julujanus, 768370, *2024-11-16 0750 field recording czestochowa zawodzie power plant chimney detonation explosion zoom h4npro*: take 2: the chimney detonation (onset 5.188 s), 5.184-7.5 s |
| `collapse_0`–`1` | `Collapse` | xkeril, 703247, *Big falling debris (crash)*: take 0: the whole crash from 0.0 s, cut 0.0-2.5 s; xkeril, 703248, *Fall debris (crash)*: take 1: the whole crash from 0.0 s, cut 0.0-2.3 s |
| `tree_fall_0`–`1` | `TreeFall` | Kinoton, 494071, *Big Tree Fall in Forest*: take 0: trunk cracks 0.29-0.95 s placed at 0.0; creak/rush 3.487-6.40 s placed at 0.50 so the ground thud (source 4.587 s) lands at 1.600 s; bruno.auzet, 670300, *tree cut down.wav*: take 1: first crack burst 2.74-3.35 s (+6 dB) placed at 0.0; cracking/rush 4.887-7.60 s placed at 0.50 so the ground thud (source 5.987 s) lands at 1.600 s (the 'bravo' voices at 11 s are not used) |
| `thunder_0`–`2` | `Thunder` | TRP, 567945, *200817 Thunder, pretty close crack 4pm.flac*: take 0: the isolated clap (onset 1.79 s), 1.78-7.78 s; TRP, 717907, *230713 Thunder, close crack crash big, mixpre6 AT875r EM272s, Stratford ON 2am*: take 1: the single clap (onset 0.625 s), 0.615-6.615 s; TRP, 717888, *230812 Thunder, med crack, rain stops dry rolling, Stratford 9am 2am*: take 2: the second strike (onset 9.445 s), 9.44-15.24 s |
| `splash_0`–`2` | `Splash` | Nox_Sound, 585744, *Foley_Natural_Water_Jump_Mono.wav*: takes 0-2: the three jumps at 0.05 s, 5.20 s and 9.65 s, 1.2 s each (splash plus the start of droplets falling back) |
| `land_0`–`3` | `Land` | Nox_Sound, 558477, *Footsteps_Mountain_Boots_Rock_Jump_Sequence_Mono.wav*: takes 0-3: landings 1, 2, 5 and 6 of the six jump-and-land events (onsets 0.470, 2.002, 6.515, 7.884 s) |
| `collapse_wood_0`–`1` | `CollapseWood` | craigsmith, 675900, *S37-15 Wooden building collapses quickly.wav*: take 0: the main crash of the collapse (onset about 1.05 s) and its debris, 1.04-3.20 s; craigsmith, 675899, *S30-08 Wall falls down from fire damage.wav*: take 1: the second wall fall (end of the creaking build-up, crash at about 10.1 s, debris), 9.95-12.45 s |
| `bed_wind_0` | `BedWind` | martypinso, 22606, *DMP013016 HEAVYSNOWSTORM.wav*: loop: 80.5-100.5 s (+2.5 s crossfade tail to 103.0 s), HP 60 Hz, LP 6 kHz |
| `bed_surf_0` | `BedSurf` | straget, 412308, *Big waves hit land.wav*: loop: 81.25-101.25 s (+2.5 s crossfade tail), HP 40 Hz |
| `bed_under_0` | `BedUnder` | DCSFX, 366159, *Underwater [Loop] AMB.wav*: loop: 60.5-72.5 s (+2 s crossfade tail), HP 30 Hz, LP 900 Hz |
| `bed_rain_0` | `BedRain` | jmbphilmes, 200272, *Rain heavy 2 (rural)*: loop: 8.2-24.2 s (+2 s crossfade tail to 26.2 s), HP 100 Hz; the thunderclap at 43 s is far outside the window |
| `bed_rotor_0` | `BedRotor` | John Sipos, 156678, *blackhawk.wav*: loop: 28.76-35.73 s with a 40 ms crossfade, HP 30 Hz |
| `bed_town_0` | `BedTown` | jameswrowles, 516759, *Small Diesel Generator*: right channel only (the Rode NTG-2 at 3 m); loop 62.5-78.52 s (+2 s crossfade tail), steady 3 kW-loaded running, HP 30 Hz |
| `bed_night_0` | `BedNight` | sengjinn, 175020, *AMBIENCE NIGHT FIELD CRICKET 01*: loop 12-28 s (+2 s crossfade tail), a steady field of crickets around midnight, HP 700 Hz |
| `fire_crackle_0`–`7` | `FireCrackle` | ahriik, 508110, *fire ambience, flames, crackles, pops, burning*: takes 0-7: 5.30-6.20 s, 10.20-11.00 s, 11.35-12.35 s, 13.75-14.55 s, 14.85-15.85 s, 16.55-17.45 s, 19.50-20.50 s, 24.65-25.45 s; HP 300 Hz |
| `swing_0`–`3` | `Swing` | qubodup, 60012, *swing 25*: take 0: the single swing, 0.012-0.255 s; qubodup, 60023, *swosh windy 36.flac*: take 1: 0.033-0.290 s; qubodup, 60029, *Swosh 42*: take 2: 0.056-0.305 s; qubodup, 60002, *Swoosh 15 Windy*: take 3: 0.024-0.285 s |
| `eat_0`–`2` | `Eat` | iamshort, 181271, *Eating a Carrot*: take 0: 0.466-1.37 s (bite + one chew); take 1: 2.452-3.10 s (two chews); take 2: 3.383-4.03 s (two chews) |
| `drink_0`–`2` | `Drink` | augustclaire2211, 624775, *Drinking water in gulps*: take 0: 0.335-1.47 s; take 1: 2.14-3.262 s; take 2: 4.093-5.165 s (each = two consecutive swallows) |
| `bandage_0`–`1` | `Bandage` | Rudmer_Rotteveel, 591526, *Cloth Rips Multiple Fast*: take 0: rip 1.865-2.215 s at 0.00; Rudmer_Rotteveel, 591195, *Cloth Rip Fast*: take 1: rip 0.05-0.40 s at 0.00 (-10 dB); ChuckleNutsDev, 718668, *Wrapping cloth*: wrap/rustle: take 0 0.80-1.43 s at 0.29 (+7 dB); take 1 1.86-2.49 s at 0.27 (0 dB); both --hp 200 |
| `bush_pick_0`–`2` | `BushPick` | vibe_crc, 59315, *looking_in_bushes.wav*: leafy rustle: take 0 1.105-1.56 s, take 1 1.705-2.20 s, take 2 2.395-2.86 s (hp 300, fade-in 15 ms); Bini_trns, 354103, *few small branches snapping, close up, high frequency,H6.wav*: soft snap (hp 300, lp 5000): take 0 1.528-1.60 s at 0.24; take 1 1.115-1.175 s at 0.22; take 2 0.676-0.735 s at 0.27 |
| `brush_0`–`3` | `Brush` (+remote) | EminYILDIRIM, 594774, *Bush Rubbing Movement*: take 0 0.50-1.12 s, take 1 9.18-9.78 s, take 2 14.55-15.05 s; vibe_crc, 59315, *looking_in_bushes.wav*: take 3 2.92-3.47 s (past the picks' spans); all hp 300, fade-in 15 ms, fade-out 80 ms (cut 2026-10-07) |
| `snort_0`–`2` | `Snort` | JarredGibb, 233181, *Pig - Multiple Snorts 4 - 96kHz.wav*: take 0: deep grunt 1.291-1.85 s; take 1: grunt + snort 2.126-2.64 s; JarredGibb, 233173, *Pig - Grunt 5 (deep) - 96kHz.wav*: take 2: deep grunt 0.764-1.52 s |
| `bird_0`–`5` | `Bird` | straget, 418102, *Great Tit.wav*: take 0: two 'tea-cher' notes 29.475-30.24 s; naturenotesuk, 418425, *Robin singing*: take 1: 27.675-28.28 s ('tsip-tsip-seee'); take 4: 13.595-14.06 s (short 'tsi' run); richwise, 811988, *Blackbird (isolated)*: take 2: 46.825-47.60 s; take 5: 27.29-28.225 s (opening flute part of two different phrases); straget, 418104, *Bluetit.wav*: take 3: 25.985-26.935 s (three high 'tsee' notes, before the trill) |

## `vsco/` — the score, played on real instruments (2026-10-03)

The nine pieces are rendered from **VSCO-2 Community Edition**
(<https://github.com/sgossner/VSCO-2-CE>, CC0 1.0, commit `4403009`;
recorded by Sam Gossner and Simon Dalzell, sample cutting by Elan
Hickler/Soundemote) by `render_music.py`, which downloads the samples it
uses into a cache and writes the nine files. The arrangement is
`synth::score`'s, voice for voice: A minor, 90 BPM, the i / ♭VI / ♭VII
sections, an 8 s body and 2.5 s of reverb ring-out. The cello section and
solo contrabass play the drone, the viola and violin sections the pad, a harp
the melody and timpani the pulse. Every sample's pitch is measured and
resampled to the exact equal-tempered target.

| Files | Cue |
|---|---|
| `music_open_calm`, `music_open_tense`, `music_open_combat` | `MusicOpenCalm`, `MusicOpenTense`, `MusicOpenCombat` |
| `music_turn_calm`, `music_turn_tense`, `music_turn_combat` | `MusicTurnCalm`, `MusicTurnTense`, `MusicTurnCombat` |
| `music_close_calm`, `music_close_tense`, `music_close_combat` | `MusicCloseCalm`, `MusicCloseTense`, `MusicCloseCombat` |

147 files from Freesound, 70 from Kenney and 9 from VSCO-2, about 4.3 MB of
Ogg in total.
