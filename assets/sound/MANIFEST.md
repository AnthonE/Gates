# assets/sound — what ships

**Every file here is CC0** (Creative Commons Zero,
<http://creativecommons.org/publicdomain/zero/1.0/>) **or public domain**
(US-government works, the Public Domain Mark: `pd/`), checked on each
source's own page. Credit is not required and is given anyway. The score
(`assets/music/`, last section) is generated under a vendor licence instead.

**Every cue the game plays is one of these files; nothing is synthesized any
more.** They are compiled into the client (`crates/client/src/sound_bank.rs`)
and decoded at boot. A one-shot cue's takes are trimmed, faded and
normalized, and played round-robin, never the same take twice running. A bed
is one seamless loop, played whole. The score is not in the bank: see
`assets/music/` below. `sound::synth` is only the fallback for a
file that fails to decode, and `crates/client/tests/sound.rs`
(`every_cue_is_a_recording`) fails if any cue has no file.

## `kenney/` — Kenney

### *Impact Sounds* 1.0, verbatim

<https://kenney.nl/assets/impact-sounds> (2019-12-19). Sounds by Kenney
(www.kenney.nl). Five takes each, `_000`–`_004`.

| Files | Cue | Heard as |
|---|---|---|
| `impactMining` | `ImpactStone` | a blow on stone or ground |
| `impactMetal_heavy` | `ImpactMetal` | a blow on metal |
| `impactPlank_medium` | `Place` | a piece going down |
| `impactSoft_medium` | `BulletSoil` | a round into soil, sand or grass |
| `impactGeneric_light` | `BulletStone` | a round on stone |
| `impactWood_light` | `BulletWood` | a round into wood |
| `impactMetal_light` | `BulletMetal` | a round on metal |
| `impactWood_heavy` | `Knock` | a knock on a door |
| `ore_glint_0`–`1` | `OreGlint` | cut here from `impactMetal_light_000` / `_002`: whole, sped to 1.65× / 1.80× (pitch and length), faded out over the last 90 / 80 ms, mono (cut 2026-10-11) |
| `tung_0`–`2` | `Tung` | Triple T's bat. Cut here from `impactWood_heavy_000` / `_002` / `_004`: the strike's first 0.16 s (faded over its last 30 ms) driving two narrow resonators at f and 2.76 f (f = 330 / 300 / 360 Hz, Q 45 / 30) under the dry hit at 0.35, high-passed at 90 Hz, 0.5 s, peak 0.85, mono (cut 2026-10-12) |

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
| `ui_hover_0` | `UiHover` | Kenney, *UI Audio*: `Audio/rollover2.ogg` |
| `inv_open_0`–`1` | `InvOpen` | Kenney, *RPG Audio*: `Audio/clothBelt.ogg`, `clothBelt2.ogg` |
| `inv_close_0` | `InvClose` | Kenney, *RPG Audio*: `Audio/cloth1.ogg` |
| `item_move_0`–`1` | `ItemMove` | Kenney, *RPG Audio*: `Audio/bookPlace1.ogg`, `bookPlace3.ogg` |
| `item_drop_0` | `ItemDrop` | Kenney, *RPG Audio*: `Audio/beltHandle2.ogg` |
| `craft_start_0`–`1` | `CraftStart` | Kenney, *RPG Audio*: `Audio/beltHandle1.ogg`, `metalClick.ogg` |
| `map_pin_0` | `MapPin` | Kenney, *Interface Sounds*: `Audio/drop_002.ogg` |
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
would have swelled at the join.

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
| `blast_0`–`2` | `Blast` | felix.blume, 251401, *Dynamite explosion in the mountain*: take 0: the single dynamite blast (onset 0.058 s) with its mountain roll, 0.054-3.554 s; alienistcog, 125937, *merrimack-st-demolition.aif*: take 1: the main demolition charge (onset 10.04 s), 10.036-12.9 s; julujanus, 768370, *2024-11-16 0750 field recording czestochowa zawodzie power plant chimney detonation explosion zoom h4npro*: take 2: the chimney detonation (onset 5.188 s), 5.184-7.5 s |
| `collapse_0`–`1` | `Collapse` | xkeril, 703247, *Big falling debris (crash)*: take 0: the whole crash from 0.0 s, cut 0.0-2.5 s; xkeril, 703248, *Fall debris (crash)*: take 1: the whole crash from 0.0 s, cut 0.0-2.3 s |
| `ore_crumble_0`–`2` | `OreCrumble` | cut here from `collapse_0` / `collapse_1` / `collapse_0` (xkeril, 703247 / 703248): the first 0.75 / 0.70 / 0.66 s, sped to 1.12× / 1.18× / 1.28× (pitch and length), faded out over the last 0.27 / 0.23 / 0.21 s (cut 2026-10-11) |
| `tree_fall_0`–`1` | `TreeFall` | Kinoton, 494071, *Big Tree Fall in Forest*: take 0: trunk cracks 0.29-0.95 s placed at 0.0; creak/rush 3.487-6.40 s placed at 0.50 so the ground thud (source 4.587 s) lands at 1.600 s; bruno.auzet, 670300, *tree cut down.wav*: take 1: first crack burst 2.74-3.35 s (+6 dB) placed at 0.0; cracking/rush 4.887-7.60 s placed at 0.50 so the ground thud (source 5.987 s) lands at 1.600 s (the 'bravo' voices at 11 s are not used) |
| `thunder_0`–`2` | `Thunder` | TRP, 567945, *200817 Thunder, pretty close crack 4pm.flac*: take 0: the isolated clap (onset 1.79 s), 1.78-7.78 s; TRP, 717907, *230713 Thunder, close crack crash big, mixpre6 AT875r EM272s, Stratford ON 2am*: take 1: the single clap (onset 0.625 s), 0.615-6.615 s; TRP, 717888, *230812 Thunder, med crack, rain stops dry rolling, Stratford 9am 2am*: take 2: the second strike (onset 9.445 s), 9.44-15.24 s |
| `splash_0`–`2` | `Splash` | Nox_Sound, 585744, *Foley_Natural_Water_Jump_Mono.wav*: takes 0-2: the three jumps at 0.05 s, 5.20 s and 9.65 s, 1.2 s each (splash plus the start of droplets falling back) |
| `land_0`–`3` | `Land` | Nox_Sound, 558477, *Footsteps_Mountain_Boots_Rock_Jump_Sequence_Mono.wav*: takes 0-3: landings 1, 2, 5 and 6 of the six jump-and-land events (onsets 0.470, 2.002, 6.515, 7.884 s) |
| `collapse_wood_0`–`1` | `CollapseWood` | craigsmith, 675900, *S37-15 Wooden building collapses quickly.wav*: take 0: the main crash of the collapse (onset about 1.05 s) and its debris, 1.04-3.20 s; craigsmith, 675899, *S30-08 Wall falls down from fire damage.wav*: take 1: the second wall fall (end of the creaking build-up, crash at about 10.1 s, debris), 9.95-12.45 s |
| `bed_wind_0` | `BedWind` | martypinso, 22606, *DMP013016 HEAVYSNOWSTORM.wav*: loop: 80.5-100.5 s (+2.5 s crossfade tail to 103.0 s), HP 60 Hz, LP 6 kHz |
| `bed_surf_0` | `BedSurf` | straget, 412308, *Big waves hit land.wav*: loop: 81.25-101.25 s (+2.5 s crossfade tail), HP 40 Hz |
| `bed_under_0` | `BedUnder` | DCSFX, 366159, *Underwater [Loop] AMB.wav*: loop: 60.5-72.5 s (+2 s crossfade tail), HP 30 Hz, LP 900 Hz |
| `bed_rain_0` | `BedRain` | jmbphilmes, 200272, *Rain heavy 2 (rural)*: loop: 8.2-24.2 s (+2 s crossfade tail to 26.2 s), HP 100 Hz; the thunderclap at 43 s is far outside the window |
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

## `pd/` — public domain and CC0 beyond Freesound (2026-10-11)

Cut here from originals fetched off OpenGameArt, archive.org, the National
Park Service and Wikimedia Commons (Freesound still refuses this box), the
licence read on each page at download. Each take mono 44.1 kHz, filtered as
stated, faded both ends, peak 0.85, Ogg Vorbis q4. Where a row names two
recordings the take is the two layered.

| Files | Cue | Source (author, licence, what was used) |
|---|---|---|
| `shot_crossbow_0`–`2` | `ShotCrossbow` | Still North Media, *Crossbow Shoot* (<https://opengameart.org/content/medieval-sound-effects-weapon-textures>, uploaded by MedicineStorm; CC0 in the page's licence box, the authors' words "use freely… no credit needs to be given"): releases at 0.319, 1.407, 2.973 s, 0.30 s each, HP 90 Hz |
| `bellow_0`–`3` | `Bellow` | Emmanuel Faivre, *Brame du cerf à 7h*, Forêt de Mormal (<https://archive.org/details/aporee_58365_66958>, radio aporee, Public Domain Mark 1.0): four red deer roars at 81.4, 155.3, 232.6, 267.1 s, HP 200 Hz / LP 6 kHz against the forest rumble. A real stag at medium distance; replaces the growl-and-howl stand-in |
| `flyby_0`–`3` | `Flyby` | a real supersonic crack — *The Free Firearm Sound Library* (<https://opengameart.org/content/the-free-firearm-sound-library>, Jaszczak, Nelson, Heras, Nanney; CC0), `C_31P` (AK-47) cracks at 0.3380 / 4.4015 s, 16 ms each, ahead of the muzzle blast — 4 ms in front of a whizz from USC Sunset Editorial *Heavy ammunition bullets flying overhead* (1.59, 2.64 s) and *Bullets flying overhead and hitting objects* (0.13, 0.82 s) (<https://archive.org/details/SSE_Library_BULLETS>, CC0), whizz at −3 dB. Replaces the whip crack |
| `torch_out_0`–`1` | `TorchOut` | take 0: stephan (pdsounds), *Zzrrrp sizzle* (<https://commons.wikimedia.org/wiki/File:Zzrrrp_sizzle.ogg>, public domain), 0.10–2.50 s; take 1: USC Red Library *R30-16 Gas Flame Hisses and Sputters* (<https://archive.org/details/Red_Library_Air_Industry>, CC0), the flame cutting out, 33.3–35.0 s; HP 300 Hz |
| `bird_6`–`11` | `Bird` (with `freesound/bird_0`–`5`) | National Park Service (public domain, credit "National Park Service"): hermit thrush, Yosemite (<https://www.nps.gov/subjects/sound/sounds-hermit-thrush.htm>) 1.15 / 4.90 s; western meadowlark, Yellowstone, NPS & MSU Acoustic Atlas / Jennifer Jerrett (<https://www.nps.gov/yell/learn/photosmultimedia/sounds-westernmeadowlark.htm>) 2.38 / 11.91 s; warbling vireo, Yellowstone, NPS / Jennifer Jerrett (<https://www.nps.gov/yell/learn/photosmultimedia/sounds-warblingvireo.htm>) 1.76 / 9.25 s; HP 1.2 kHz |
| `chop_0`–`4` | `ImpactWood` | USC Sunset Editorial, *Two people chopping a tree with axes* (<https://archive.org/details/SSE_Library_WOOD>, CC0): chops at 0.48, 1.77, 7.63, 10.48, 13.95 s, 0.32 s each, HP 80 Hz. A real axe in a trunk, energy to ~4 kHz, where Kenney's `impactWood_medium` had almost none above 500 Hz |
| `shot_heli_0`–`3` | `ShotHeli` | a real AK-47 — *The Free Firearm Sound Library* (<https://opengameart.org/content/the-free-firearm-sound-library>, Jaszczak, Nelson, Heras, Nanney; CC0), `C_28P` (near, single shots): the four rounds at 0.611, 3.257, 6.020, 9.156 s, right channel only (the pair is 2 ms apart and combs when summed), 0.56 s each from 2 ms before the blast, pitched to 0.85× (pitch and length) for a heavier gun, HP 60 Hz, faded over the last 150 ms (cut 2026-10-13). The heli fires 10 a second, the AK's own rate, so a burst reads round by round where the revolver's ring smeared it |
| `shot_heli_far_0`–`2` | `ShotHeliFar` | a real Mosin-Nagant (7.62×54R, a machine gun's round) — *The Free Firearm Sound Library* (as above; CC0), `M_26P` (mid distance, single shots): the three shots at 1.137, 6.105, 10.743 s, each the blast and its echo off the range ~130 ms later, right channel only, 0.62 s from 12 ms before the blast, pitched to 0.85× like the near takes, HP 80 Hz / LP 4 kHz, everything after 120 ms (the echo and the tail) 8 dB down, faded over the last 250 ms (cut 2026-10-13). Distant shots kept sharp with quiet tails, as the reference tuned its own, so a burst far off still reads round by round |
| `bed_rotor_0` | `BedRotor` | USC Red Library, *R03-02 Helicopter Flying in Place* (<https://archive.org/details/Red_Library_Aircraft_Misc>, CC0): a two-blade rotor close by, 11.6 slaps a second. Loop 6.0–28.1 s, the end moved +63 ms so the seam falls on the same point of the blade beat, 0.6 s linear crossfade. The tape is high-passed near 100 Hz, so the thump under it is rebuilt from its own slaps: the 100–600 Hz band's envelope, band-passed to 45–120 Hz and added so 50–100 Hz sits 10 dB under the whole (cut 2026-10-13). Replaces a Black Hawk's four-blade buzz |
| `bed_rotor_far_0` | `BedRotorFar` | USC Gold Tape 36, *G36-07 Helicopter Flies In and Out and In* (<https://archive.org/details/GOLD_TAPE_36_Helicopters>, CC0): the two-blade helicopter holding off at a distance, steady to ±1 dB. Loop 40.5–60.5 s, 1 s equal-power crossfade, HP 30 Hz, LP 2.5 kHz (above it is tape hiss; a far rotor has nothing there) (cut 2026-10-13) |

The USC library is the same Sunset/Red/Gold editorial collections, digitized
by Craig Smith, whose Freesound uploads already ship above (`craigsmith`).

## `assets/music/` — the score (2026-10-11)

Four whole tracks, generated with **ElevenLabs** on the operator's plan
that licenses commercial use (`DECISIONS.md` 2026-08-11: paid plan only),
on 2026-08-06; supplied 2026-10-11 to replace the nine VSCO-2 pieces. The
prompts were not kept. Not under `assets/sound/` and not in the
bank: they are streamed (`crates/sound/src/music.rs`), natively by a decoder
thread and in the browser by the page's `<audio>` element, so nothing is
compiled into the client and a page fetches only the track it plays.

Re-encoded here from the supplied 192 kbps MP3s to Ogg Vorbis, stereo,
44.1 kHz, `-q:a 3`, each gained to **−20 LUFS integrated** (they arrived
between −18.8 and −31.6) with no limiting; peaks stay under −2 dBFS.

| File | Track | Length | Gain | Role |
|---|---|---|---|---|
| `safe_zone.ogg` | *Safe Zone* | 240.0 s | −1.2 dB | the intro: the PLAY press, the menus (looped), the first minutes |
| `green_veil.ogg` | *Green Veil* | 300.0 s | −0.7 dB | a world song: a low, steady drone |
| `ruin_respiration.ogg` | *Ruin Respiration* | 300.0 s | +11.6 dB | a world song, and the one a tense moment picks |
| `empty_citadel.ogg` | *The Empty Citadel* | 240.0 s | +8.5 dB | a world song: a flat sub-bass drone |

146 files from Freesound, 65 from Kenney and 33 in `pd/` in the bank, about 4.2 MB of Ogg,
plus the four score tracks, 12.0 MB.
