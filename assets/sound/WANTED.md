# assets/sound — WANTED

Every sound this client plays, as a sourcing worklist. **Owns nothing** —
`assets/models/WANTED.md`'s shape for audio. The enum is the authority
(`crates/client/src/sound/mod.rs::Cue`, 40 cues); this file is read against
it, and **the command is the claim, not the count**:

```
cargo run -p client --bin soundbank -- /tmp/bank   # dump the current bank, listen, A/B
```

Today every cue is generated at boot (`sound/synth.rs`). Two operator calls
frame the replacement work:

- **`DECISIONS.md` 2026-08-11** — ElevenLabs is approved for audio, **paid
  plan only** (their free tier is non-commercial and this game is sold).
  Record vendor, plan, date and the **full prompt** per file. A prompt
  describes the *thing*, never the reference game — the Facepunch rail's new
  surface. "ElevenLabs retires `synth.rs`'s reason, not the synth": generated
  audio is for what a synth cannot reach, and the four interface symbols
  (§1.3) are where the synth is arguably already the right register.
- **`DECISIONS.md` 2026-08-07** — the licence rail: **CC0 default, CC-BY
  ships with a `NOTICE`/CREDITS entry, NC and SA refused** (non-commercial
  does not survive a sold product; share-alike does not survive a closed
  depot). Applies per FILE, read off the page it ships from, never off a
  pack's reputation.

⚠ **The audio hosts are blocked from this box in both fetch pipes — but
search is not.** Probed 2026-08-15: kenney.nl, sonniss.com, freesound.org,
opengameart.org, pixabay.com all 403 at the egress proxy, for direct
downloads and page fetches alike (the posture 2026-08-07's row measured for
mesh hosts), while web *search* reaches out. So §3 is verified at **search
tier** — `DOORS.md`'s exact caveat, named rather than hidden — and the
licence box on the actual page still decides at download time.
**Downloading binaries is an operator act from a machine that can reach the
hosts**, or from a session whose environment network policy allows them (a
per-environment setting on the remote box).

---

## 0 · Pipeline — what a file has to be to ship here

| thing | value | why |
|---|---|---|
| format | **WAV, 16-bit PCM, mono, 44.1 kHz** | the bank's own format (`synth::to_wav16`, `SAMPLE_RATE`). rodio resamples anything, but native rate skips the resample and matching the bank keeps A/B honest. |
| channels | **mono, load-bearing** | positional cues are panned per-ear by the spatial sink; the bank is mono by construction. Downmix stereo sources in the edit pass. |
| peak | **normalize to ~0.9 full scale** | `synth::PEAK`. One peak for the whole bank so `CueDef::gain` is the ONLY thing deciding relative loudness — a file at its own natural level splits the mix between the table and the file, which is the split that makes a mix impossible to reason about. **Never bake loudness in.** |
| edges | start and end **near silence**, no clicks | `tests/sound.rs` asserts this of the generated bank (0.5 ms head, 4 ms tail — `synth::edges`); the same gates port to loaded files. Trim tight: cooldowns and lengths in §1 assume no dead air. |
| reverb | **dry for every one-shot** | the world is outdoors and the submerged snapshot ducks a mix it cannot un-reverberate. Two exceptions: the beds may carry natural space, and the music's tail IS the mechanism (§4). |
| count | **one file per cue** | the bank is `[Vec<u8>; CUE_COUNT]`. Variation is `Cue::pitch_var` (±7–16 % playback rate), not round-robin files — a variation bank per action is what the reference does (`reference/AUDIO.md` §5) and is a *system* change, not a download. Buy takes to AUDITION, ship one. |
| loops | the three beds loop **seamlessly**, ≥ 10.5 s | the current loop is 10.5 s cut from 12 s by an equal-power crossfade (`synth::loop_seam` — a linear fade dips ~3 dB at the join, audible as a pulse forever). Generate with a loop option or hand-seam the same way. |
| naming | `NN_CueName.wav`, the soundbank's own stems | `00_StepSand.wav` … `39_Growl.wav`. Discriminants are append-only (the enum's own rule), so the numbers are stable. |
| provenance | a manifest row per file, **before** it ships | `assets/models/MANIFEST.md`'s columns: vendor/source, licence or plan, URL or prompt + task id, date. Create `assets/sound/MANIFEST.md` with the first landed file. CC-BY additionally writes the author into the NOTICE. |
| candidates | **out of the tree** | auditioning "a lot of sounds" means a gitignored `assets/sound/candidates/` — `assets/textures/candidates/` is the precedent (1.3 GB, gitignored), and the 2026-08-14 depot trap is why it matters: the packager stages from `git ls-files` now, but the queue stays out of the tree anyway. |

**Refused regardless of where found**: any rip of the reference game's audio
(the IP rail — a YouTube "Rust sounds" video is not a licence);
xeno-canto recordings (overwhelmingly NC/SA — the exact trap `synth.rs`'s
header records for SeedThree's and Eanpa-Sky's bird files, which are also
still refused); BBC Sound Effects archive (non-commercial terms).

**The swap seam is one function.** Nothing loads audio files today —
`render/audio.rs::build_bank` calls `synth::bank()` and `AudioSource` does
not care where bytes came from, so a loader is a change at that seam
(per-cue override with synth fallback is the obvious first shape). The
structural gates in `tests/sound.rs` port as-is — energy, no clipping, quiet
edges, seam continuity — and determinism becomes trivial (fixed bytes).
Budget: all 35 files as WAV ≈ **12 MB** (≈ 0.8 MB one-shots + 2.8 MB beds +
8.3 MB score) against a 124 MB depot — fine as WAV. If someone reaches for
OGG instead, note the interlock: Bevy's `vorbis` feature is currently on the
trim list as unused (`NOW.md` §0x trim item) and would become load-bearing.

---

## 1 · The inventory — 40 cues, 35 files to source

Lengths are the current bank's, read off `synth.rs` — advisory except where
marked hard. "own" = non-positional (happens to *you*; no distance, no pan).
★ = highest gain over the synth: source these first. ✋ = keep-synth
candidate: a learned symbol at fixed pitch, where generated-simple is
arguably correct — source only if a take clearly beats it.

### 1.1 · Footsteps — 5 files (+5 remote cues that reuse them, **do not source separately**)

Surface is picked from the terrain under the boot (`sound/steps.rs`); the
remote five (`23–27_RemoteStep*`) are **byte-identical by delegation** — the
ground decides what a step sounds like, not whose boot it is
(`tests/sound.rs` pins the equality). ±10 % pitch variation comes free, so
one GOOD take per surface is enough. Retriggers every ~0.85 m of stride.

| file | len | character to match |
|---|---|---|
| ★ `00_StepSand` | 0.18 s | soft dull thud, top end absorbed (LP ~1.1 kHz), no scuff |
| ★ `01_StepGrass` | 0.17 s | light crisp rustle over a soft thud, brighter than sand |
| ★ `02_StepLitter` | 0.22 s | dry leaf-and-twig crackle, sparse discrete snaps, forest floor |
| ★ `03_StepRock` | 0.14 s | hard short knock, sharp attack, brightest of the five, tiny grit |
| ★ `04_StepWater` | 0.34 s | ankle-deep slosh — a swell, not a slap (22 ms attack), band-limited |

### 1.2 · Tools and impacts

| file | len | fires | character |
|---|---|---|---|
| `05_Swing` | 0.26 s | every melee/tool swing (own) | air whoosh, band sweeping UP 400→2600 Hz, envelope peaks ⅓ in — a moving band, not a hiss; **no impact in it** |
| ★ `06_ImpactWood` | 0.30 s | hitting a tree (positional, 40 m) — *bank ready, producer owed* (`NOW.md` §0x item 4) | deep solid thock ~185 Hz, fast decay, dry, a little debris |
| ★ `07_ImpactStone` | 0.26 s | hitting rock/ore (40 m); **also reused** as the puff when a felled trunk despawns (`render/audio.rs`) | gritty crunch, mostly noise, little ring, hard attack |
| `08_ImpactMetal` | 0.55 s | hitting metal (48 m) — *producer owed* | clank with **inharmonic** ring (620 + 1370 Hz — deliberately not a musical interval), bright transient |

### 1.3 · Interface signals — fixed pitch, learned as symbols

`pitch_var` is **zero** here: a symbol that changes pitch is a symbol that
takes longer to learn. Pick clean, distinct, consistent — and these four are
the strongest keep-synth candidates (2026-08-11: "the synth is still right
for anything synthesizable").

| file | len | fires | character |
|---|---|---|---|
| `09_Gather` | 0.20 s | resources entered your bag (own) | short pocketing thunk-rustle — diegetic-ish, the one non-symbol here (±7 % var) |
| ✋ `10_CraftDone` | 0.36 s | craft finished (own) | two soft notes UP (C5→G5) — the only cue allowed to be musical |
| ✋ `11_Refused` | 0.14 s | any refusal (two rings feed it) | short flat LOW buzz ~155 Hz with a ~9 Hz beat — noticed then ignored; must stay out of the 2–5 kHz carve |
| ✋ `12_Hit` | 0.035 s | your hit landed (own; one marker per volley) | very short bright tick 2.1 + 3.15 kHz — nothing else in the bank lives up there; that is what makes it readable through a fight |
| ✋ `18_UiClick` | 0.018 s | menu/panel click — *producer owed* | tiny neutral tick ~1.45 kHz |

### 1.4 · Body and world

| file | len | fires | character |
|---|---|---|---|
| `13_Hurt` | 0.42 s | you took damage (own, priority 7) | thud with a **falling** pitch 240→130 Hz — every organism reads a falling pitch as damage; a short human grunt fits |
| `14_Death` | 1.10 s | you died (own, priority 8, outranks everything) | longer, deeper collapse 190→55 Hz, final |
| `15_Place` | 0.24 s | a build piece seats (positional, **at the socket** — the reference shipped this at the world origin for a while; ours carries a position or does not fire) | solid low wooden thunk ~140 Hz |
| ★ `16_Splash` | 0.80 s | breaking the water surface, either direction (own) | low displacement body falling 135→75 Hz + broadband burst + **droplet tail** — the tail is what separates it from a big footstep |
| ★ `17_TreeFall` | 1.9 s | a tree comes down (positional, **96 m — the loudest, furthest thing in the game**, sets `MAX_AUDIBLE_M`) | a sequence: rising canopy swish (~1.1 s) → one hard crack → heavy ground thud with a low tail that outlives everything |

### 1.5 · The beds — 3 seamless loops (`render/audio.rs` holds one voice each, gains only)

| file | loop | character |
|---|---|---|
| ★ `19_BedWind` | 10.5 s | low airy body + brighter gust layer; gusts at ~10.5 s and ~5.25 s periods, swing bounded ~8 dB — the first cut read as the ambience *cutting out* every ten seconds, and `the_bed_gusts` now gates the swing from both sides |
| ★ `20_BedSurf` | 10.5 s | waves: a break (low boom) roughly every 5.25 s, the broadband wash arriving *after* it and outlasting it (the lag is what makes it a sea and not a tremolo on noise), a slower swell so no two breaks are identical |
| ★ `21_BedUnder` | 10.5 s | dark by construction — almost nothing above 400 Hz but sparse rising bubble blips (~every 1.4 s); gated darker than the wind bed |

### 1.6 · Animals and the forest layer

| file | len | fires | character |
|---|---|---|---|
| ★ `22_Snort` | 0.55 s | the pig announces itself (positional, 40 m — heard before seen, `reference/ANIMALS.md`) | **double** nasal exhale with ~26 Hz flutter + a low falling grunt under it; dark (230–950 Hz); the pair is what reads as an animal and not a pneumatic valve |
| ★ `28_Bird` | 0.62 s | daylight forest layer, fired from a drawn perch (positional, 44 m) | three whistled chirps ~2.1–3.7 kHz with vibrato and a jittered internal rhythm; **heard for minutes on end** — widest pitch var in the bank (±16 %) and still the most exposed repetition risk; audition several takes hard |
| ★ `38_Howl` | 3.0 s | wolf far voice (positional, **88 m** — second only to the tree; "the island tells you it has wolves, and roughly where") | fast rise onto a held wavering note ~400–470 Hz, slow waver (~1.6 Hz), a few abrupt pitch *breaks*, long terminal fall ending BELOW where it started; a single lone wolf, no pack answer |
| ★ `39_Growl` | 1.15 s | wolf near voice (positional, 14 m — inside its notice radius; register picked by distance, `sound/voice.rs`) | low rough continuous rumble, F0 70–110 Hz — slow enough to hear individual pulses — through a real throat (formants at ~340/1020/1700/2380 Hz), period-doubling mid-call, near-square envelope; **must read darker than the howl with your back turned** |

### 1.7 · The score — 9 files, hard constraints, see §4

`29_MusicOpenCalm` … `37_MusicCloseCombat`: three sections of one theme ×
three intensity tiers. Not a download — a composer brief (§4).

---

## 2 · The ElevenLabs sheet

Technique first (all of it follows from §0):

- **Paid plan only**, and record plan + date + full prompt per take you keep
  (2026-08-11 is the authority; rights attach at generation time).
- The SFX endpoint takes a **duration of 0.5–30 s** (unset = it guesses
  from the prompt) and a **prompt influence**; **`loop` exists and is the
  beds' tool** — on the `eleven_text_to_sound_v2` model (verified against
  their docs 2026-08-15). Generate **4+ takes per prompt**, audition
  against the soundbank dump, keep one.
- Ticks shorter than the floor (`12_Hit`, `18_UiClick`) come out of a 0.5 s
  take trimmed to the transient — or honestly, keep the synth (✋).
- One edit pass on every keeper: downmix to mono, trim edges to silence,
  peak-normalize ~0.9, export WAV 16-bit 44.1 kHz, stem-name it.
- Words that earn their place: *"one shot"*, *"single"*, *"dry, no reverb"*,
  *"close-mic"*, *"no voices"*, *"seamless loop"* (beds only). Never name
  the reference game or any game.

| file | ask for | prompt |
|---|---|---|
| `00_StepSand` | 0.5 s → trim 0.18 | One single footstep on dry loose sand, soft dull thud, weight settling, no scuff, no gravel, close-mic, dry, no reverb, one shot |
| `01_StepGrass` | 0.5 s → 0.17 | One single footstep on short dry grass, light crisp rustle over a soft thud, close-mic, dry, no reverb, one shot |
| `02_StepLitter` | 0.5 s → 0.22 | One single footstep on a forest floor of dry leaves and small twigs, crackling leaf-litter crunch with sparse sharp snaps, dry, no reverb, one shot |
| `03_StepRock` | 0.5 s → 0.14 | One single boot step on bare solid rock, hard short knock, sharp bright attack, tiny grit scatter, dry, no reverb, one shot |
| `04_StepWater` | 0.5 s → 0.34 | One single footstep in ankle-deep water, a short soft slosh that swells rather than slaps, small swirl, no big splash, dry, one shot |
| `05_Swing` | 0.5 s → 0.26 | A single fast axe swing through air, short rising whoosh, accelerating, clean air cut, no impact, no grunt, dry, one shot |
| `06_ImpactWood` | 0.5 s → 0.30 | A single axe blow into a living tree trunk, deep solid knock, dry wood thock with a few bark chips, no creaking, no falling, exterior, one shot |
| `07_ImpactStone` | 0.5 s → 0.26 | A single pickaxe strike on solid rock, gritty stone crunch with a dull clink and crumbling debris, hard attack, short, dry, one shot |
| `08_ImpactMetal` | 0.7 s → 0.55 | A single hard strike on a thick metal plate, clanging metallic ring that is not a musical note, industrial, decay under half a second, dry, one shot |
| `09_Gather` | 0.5 s → 0.20 | Scooping up and pocketing a handful of resources, one short satisfying cloth-and-wood rummage thunk, quick, dry, one shot |
| `10_CraftDone` ✋ | 0.5 s → 0.36 | A soft warm two-note completion chime rising upward, gentle and round, minimal interface sound, no sparkle tail |
| `11_Refused` ✋ | 0.5 s → 0.14 | A short quiet low muted error buzz around 150 hertz, flat dull denial tone, unmusical, dark, dry, one shot |
| `12_Hit` ✋ | 0.5 s → 0.035 | A tiny sharp bright click marker, one crisp high tick, extremely short, clean, dry |
| `13_Hurt` | 0.6 s → 0.42 | A short human pain grunt with a dull body thud, breathy, pitch falling, no words, dry, one shot |
| `14_Death` | 1.5 s → 1.10 | A human collapse, a heavy groaning exhale falling in pitch into a body slumping onto the ground, final, dry, one shot |
| `15_Place` | 0.5 s → 0.24 | A heavy wooden building piece dropping into its socket, one solid low carpentry thunk with a brief settle, construction, dry, one shot |
| `16_Splash` | 1.0 s → 0.80 | A body plunging through a lake surface, one full splash, deep hollow bloom then droplets pattering back onto the water, exterior, one shot |
| `17_TreeFall` | 2.5 s → 1.9 | A large pine tree falling in a forest, rushing foliage building up, one loud sharp trunk crack, then a heavy ground impact thud with settling debris, exterior |
| `18_UiClick` ✋ | 0.5 s → 0.018 | A minimal soft neutral interface click, one tiny tick, very short, clean, no tone |
| `19_BedWind` | ≥ 12 s, **loop** | Steady outdoor wind over open coastal grassland, low airy body with slow gentle gusts every several seconds, no birds, no leaves, no voices, smooth, seamless loop |
| `20_BedSurf` | ≥ 12 s, **loop** | Ocean waves on a beach from a short distance, a wave breaking about every five seconds, low boom then a trailing hiss of wash, no seagulls, no voices, seamless loop |
| `21_BedUnder` | ≥ 12 s, **loop** | Underwater ambience, dark muffled low rumble, occasional small rising bubbles, dense, calm, no music, seamless loop |
| `22_Snort` | 0.7 s → 0.55 | A wild boar double snort, two short forceful nasal exhales in quick succession with a low grunt underneath, close, dark, dry, one shot |
| `28_Bird` | 0.8 s → 0.62 | One small songbird calling, three quick clear whistled chirps with a slight warble, bright, close, no other birds, no ambience, dry, one shot |
| `38_Howl` | 3.5 s → 3.0 | A single lone wolf howl, rising quickly onto a long held slowly wavering note, then falling away low at the end, open air, no pack answering, no echo |
| `39_Growl` | 1.5 s → 1.15 | A wolf growling close and continuous, very low rough rattling throat rumble, menacing and sustained, no bark, no snap, dry, one shot |

---

## 3 · Open source that fits — and what each costs at the rail

Verify the licence **on the page, per file**, and write the manifest row
before it ships. CC-BY additionally writes the author into the NOTICE.

| source | licence class | good for | notes |
|---|---|---|---|
| **kenney.nl** — *Impact Sounds* (×130), *Interface Sounds* (×100), *UI Audio*, *RPG Audio* (×50: footsteps, creaks, coins, knives) | CC0, the whole site's standing policy (counts verified 2026-08-15, search tier) | `06/07/08_Impact*`, `15_Place`, the four ✋ interface symbols, footstep spares | first stop — zero notice cost, game-ready one-shots, already the icon rail's licence tier |
| **freesound.org** with the licence filter set to **CC0** (CC-BY acceptable with the notice) | per-file — the site's set is CC0 / CC-BY / **CC-BY-NC, and NC is the one the filter is FOR** | the organic set: per-surface footsteps, `16_Splash`, `22_Snort` (search boar/pig grunt — close-perspective grunt sets exist), `38_Howl` / `39_Growl` (§3.1 pins one), all three beds (wind loop / shore waves loop / underwater loop) | record URL + author + licence per file; prefer recordists' own uploads over compilation accounts (provenance) |
| **opengameart.org**, licence filter CC0/CC-BY | per-asset | **Fantozzi's Footsteps (Grass/Sand & Stone)** — CC0, 12 single steps, 16-bit 44.1 kHz FLAC/OGG, three of our five surfaces; other footstep and UI packs | same drill: the page's licence box decides |
| **Sonniss GDC bundles** | **not CC** — Sonniss's own licence: worldwide, non-exclusive, royalty-free, unlimited projects, **no attribution**, no reselling sounds individually, media production only (search tier, 2026-08-15) | pro-grade everything, tens of GB | **operator call before use**: the rail names CC0/CC-BY only. The 2026-08-11 row already added a fourth basis (vendor contract) for generated assets, so the shape of the call exists — but someone must read this contract whole first |
| **pixabay.com** sound effects | **not CC** — Pixabay Content License | broad coverage | same class as Sonniss: permissive but off-rail, operator call |
| refused | — | — | reference-game rips in any wrapper; xeno-canto (NC/SA); BBC SFX archive (NC); SeedThree / Eanpa-Sky bird files (the documented trap) |

### 3.1 · Pinned candidates (search tier, 2026-08-15 — confirm on the page before the manifest row)

| for | candidate | licence as reported |
|---|---|---|
| `00–03_Step*` | *Fantozzi's Footsteps (Grass/Sand & Stone)*, opengameart.org — 12 single steps | CC0 (itself cut from freesound CC0 sources) |
| `06/07/08_Impact*`, `15_Place` | Kenney *Impact Sounds* — kenney.nl/assets/impact-sounds | CC0 |
| the four ✋ symbols | Kenney *Interface Sounds* + *UI Audio* | CC0 |
| `38_Howl` | freesound.org/s/398430/ — "Wolf howl" by NaturesTemper, ~8 s | CC0 |
| `39_Growl` | freesound.org/people/newagesoup/sounds/338674/ — "wolf-growl.wav" by newagesoup | **unconfirmed** — the summary notes a sub-bass enhancement; read the page |
| avoid | freesound 267179, "Scary Ghost Wolf Howling" | CC0, but a *pitch-shifted human scream* — wrong register for a contact call, and an animal voice with human provenance is a question nobody needs |

The boar, the tree fall and the three beds pinned nothing at search tier —
plentiful behind freesound's CC0 filter, but no specific file could be
licence-read from here. ElevenLabs stays the likelier winner for the tree
fall and the growl either way (§2).

---

## 4 · The score — a brief, not a download

Nine pieces, and the constraints are the *mechanism*, not taste:
`music::Director` cuts from any piece to any other at a section boundary,
and everything below is what makes that cut inaudible
(`sound/music.rs`, `reference/AUDIO.md` §8).

| constraint | value | breaks if violated |
|---|---|---|
| tempo | **90 BPM, all nine** | cross-piece cuts land off-grid |
| key | **A minor** (root A2 = 110 Hz), pentatonic melody register | a cut becomes a modulation |
| sections | 3, chords **i / ♭VI / ♭VII** (Am / F / G) — a plagal turn that never resolves | a cadence the director keeps stepping on |
| piece length | **10.5 s**: 8.0 s body + 2.5 s tail | — |
| tail | **no note onset after 8.0 s**; the last 2.5 s is ring-out only (reverb/delay) | the next piece starts at 8.0 s *over* the tail — a note there collides |
| tiers | Calm / Tense / Combat differ in **density and register, never tempo or key** | see tempo/key |
| levels | deliver all nine at **equal peak** | the cue table's gains (0.55 / 0.78 / 1.0) do the tiering; a louder combat mix double-applies it |
| cadence | a piece plays every 4–8 min of world time (first at ~30 s) | not a file property — the director's, listed so nobody masters for wallpaper |

Character, per the current placeholders: a low drone (root+fifth, detuned)
under a slow chord pad, a sparse plucked pentatonic line, and — combat tier
only — a low pulse every third beat. Dark wilderness, no percussion below
combat. ElevenLabs Music can attempt this brief verbatim; whether any
generator hits the grid exactly is the question, and a DAW pass to conform
tempo/key/tail is expected either way. Swap seam: `synth::render`'s music
arm (`NOW.md` §0x item 2).

---

## 5 · Priority, if the session is one evening

1. **The wolf pair + the pig** (`38/39/22`) — the most exposed synthesis in
   the bank: sustained tonal animal voices are what arithmetic fakes worst.
2. **Footsteps ×5** — the most-heard sounds in the game.
3. **`17_TreeFall`, `16_Splash`, the impacts ×3, `09_Gather`, `15_Place`** —
   the gather loop's whole soundscape.
4. **The beds ×3** — long exposure, but the current ones are competent
   noise-craft; gains are smaller.
5. **`28_Bird`** — high exposure, needs the pickiest audition (repetition).
6. **`13_Hurt`, `14_Death`, `05_Swing`.**
7. The four ✋ symbols — last, or never.

What does NOT need buying: the five remote steps (aliases), and nothing
lands without its manifest row (§0).
