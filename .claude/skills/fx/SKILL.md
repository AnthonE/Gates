---
name: fx
description: Make or tune a visual effect in Gates (sparks, flashes, dust, smoke, blood, splashes, fire, a blast) on the fx stage: stage it alone in front of a dark and a light card, read its filmstrip and numbers, retune it with knobs without a rebuild, compare against the last run, and bake the result into the recipe. Use when asked to add, fix, tune, compare or review an effect or a particle look.
---

# Effects in Gates

An effect is judged as it moves, never from a single still. The loop is:
1. stage the effect alone;
2. read its **strip** (every frame, its envelope and its numbers);
3. change one thing;
4. read the strip again and its **diff** against the last run.

The shared `fx` crate (`crates/fx`, Bevy-free) owns the particle pool, the sprite sheet, the
recipes and the stage's contract. Gates' half is `crates/client/src/render/fx/`. oneshot and
Before Colony vendor `crates/fx` and `ci/fxstrip.mjs` from here, so the same loop runs in all
three games.

```bash
ci/fx.sh list                                    # every effect the stage can fire
ci/fx.sh bullet-metal                            # → shots/fx/bullet-metal/{strip.png,numbers.json,diff.png}
ci/fx.sh bullet-metal --knobs 'Sparks.speed=8,16;Flash.life=0.03,0.05'   # retune, no rebuild
ci/fx.sh bullet-metal --seed 7                   # another roll of the dice
ci/fx.sh blast --frames 15 --span 2 --bloom      # more frames, a shorter span, the desktop's bloom
node ci/board.mjs look --views fx-bullet-metal   # the strip on the board, beside its refs and marks
```

**Read `strip.png` first.** Its parts:
- **The hero frame:** the brightest frame, drawn large.
- **The envelope:** how much each card changed over time; the last run is dashed.
- **Every frame:** captioned with its step, time, live particles (glow/soft) and per-card numbers.

Then read `numbers.json` for anything the picture leaves open, and `diff.png` after a change.

Setup:
- The first run builds the stage; `--no-build` reuses it.
- It runs headless on lavapipe under Xvfb (`apt-get install mesa-vulkan-drivers xvfb`), about 20 s per effect.
- A rerun with nothing changed shows 0% moved: the frames repeat exactly.

## The stage

- **One effect, fired twice at once:**
  - in front of a **dark card** (the left half of the frame; light reads here);
  - in front of a **light card** (the right half; smoke, dust and blood read here).
- **The throw:** out of the card, toward the camera and a little up. Sparks bounce off the card they hit.
- **The camera:** fixed (`fx::stage::Framing`), sized to the effect's radius.
- **The look:** the game's sun, exposure (`DAY_EV100`) and TonyMcMapface.
- **What's off:** no shake and no TAA. Bloom is off unless `--bloom`; the web has no bloom, so off is the bar.
- **The clock** steps exactly 1/60 s:
  - step 0 is the **plate**, before the effect fires;
  - the effect fires on step 1;
  - the clock holds still for every frame it saves.
- **What it fires:** the game's own code (`render/fx/stage.rs` → `Fx::impact` with a real `Contact`). The strip is what a blow in the game throws.

## The numbers

Every number is measured per card against the plate, on sRGB 0–255 Rec.601 luma.

| number | what it says | read it as |
|---|---|---|
| `add` | mean light added over the card | glow: sparks, flash, fire. Peaks in the first frames of an impact |
| `sub` | mean light taken away | smoke, dust, blood. Mostly on the light card |
| `cover` | % of the card that changed at all | how much of the frame the effect owns |
| `peak` | 99.9th-percentile luma of what changed | 255 = something hit white |
| `clip` | % of the changed pixels at 250+ | a flash may clip for a frame; a cloud never should |
| `bbox`, `c` | where it changed, 0–1 of the card; its centre | spread and drift; `travel` is how far `c` moved |
| `envelope` | `peak`/`peak_t`, `fall_t` (half), `life_t` (last seen), `area_max`, `travel` | the effect's shape in time |
| `vs_prev` | % of each frame that moved since the last run | a change should move only the frames it means to |
| `plate.ok` | the cards read ~25 and ~185 | if not, the stage's light changed and nothing else is comparable |

The numbers keep a floor under the eye; they never replace it. Look at the strip.

## Make or tune one

1. **Write the brief before touching anything** (3–5 lines). It covers:
   - the event's **envelope**: an instant (an impact, a muzzle) or sustained (a fire), and how long, in seconds;
   - its **size** in metres;
   - its **colour and temperature** over life (white-hot → ember, grey → clear);
   - what must read on **each card**;
   - the **reference**, if any (a board ref, or the operator's words).
2. **Find the recipe.**
   - A blow's layers are `render/fx/table.rs`:
     - `effect(weapon, matter)` picks up to five layers and their counts;
     - `built(layer)` is each layer's `Recipe` (`fx::recipe::Recipe`: orient, cell, speed, spray, up, gravity, drag, life, size, grow, c0→c1, tinted, spin, stretch, len_max, bounce);
     - the sprites are `render/fx/atlas.rs` (`CELLS`, a layout of `fx::atlas::Shape`s).
   - Other effects hand-build particles (`gun.rs`, `world.rs`, `gate_fx.rs`, `town_fx.rs`).
3. **Stage it.**
   - New: add it to `render/fx/stage.rs`'s `list()` (a name, a radius, a span) so `ci/fx.sh` can fire it.
   - Something not driven by `Fx::impact`: give `stage::fire` a branch that calls its code.
4. **Shoot, read, judge** against the brief, or send the `fx-judge` agent (`.claude/agents/fx-judge.md`) for a read that is not anchored on yours.
5. **Tune with knobs**, one idea at a time: `--knobs 'Layer.key=value;…'`.
   - Values: a number, a range `a,b`, a colour `r,g,b,a` (linear, HDR allowed), `true`/`false`, or `billboard`/`stretch`/`flat`.
   - Layer names are `table::Layer::name()`.
   - `stage.json` prints every layer's effective recipe in knob syntax.
6. **Bake it in.** Copy the numbers into `built(layer)`, rebuild, and shoot once more without knobs. `diff.png` must show only the frames you meant to change, and `vs_prev` must be ~0% against the last knob run.
7. **Check what you can't see on one roll:** two more `--seed`s for anything random, and `--bloom` once for the desktop.
8. **Board.** `node ci/board.mjs look --views fx-<name>` puts the strip beside its references and the operator's marks.
   - You may `node ci/board.mjs mark fx-<name> note "…" --by <you>`.
   - `right` and `wrong` are the operator's word, never yours.
   - A new effect gets an `fx-<name>` view in `board/board.json`; copy an existing one and add the name to the shared `shoot` list.
9. **Then:** `ci/quick.sh fx` and commit.

## Judging (after the threejs-procedural-vfx and threejs-visual-validation skills here)

- **Every layer has a role:** silhouette, motion, light or residue. A layer you can't name a role for goes.
- **One event drives every layer.** Their onsets line up: an impact peaks in the first two or three frames (`peak_t` ≤ 0.05 s on the dark card), its residue comes after, and nothing starts late by accident.
- **It reads with bloom off.** A hot core on the dark card; streaks never thinner than `fx::pool::MIN_PX` (a spark never vanishes at range). Bloom is a response to brightness, not the shape.
- **Alpha layers read on the light card.** Dust the colour of the card is invisible there; the numbers say so (`sub` ≈ 0).
- **Clip is for an instant.** `clip` over ~5% past the first frames is a blown-out blob; lower `c0` or `size`.
- **Life matches the brief.** `life_t` and `gone`: nothing hangs past the brief or pops out (a sudden drop in the envelope with `cover` still high).
- **Motion from the strip, never one frame.** A good still can hide a pop, a stall or a layer that never moves.
- **Three seeds before calling anything random good.**
- **Budget.** `live` (glow/soft) against the pools (`GLOW_POOL` 512, `SOFT_POOL` 256): one blow should use a small share; a firefight is dozens of them.

## Rules

- `crates/fx` and `ci/fxstrip.mjs` are canonical here, Bevy-free and dependency-free. After changing them, run `tools/fx-sync.sh` in oneshot and `scripts/fx-sync.sh` in Before Colony; their copies refuse local edits (`crates/fx/tests/vendored.rs`).
- The sprite sheet is pinned (`the_default_sheet_is_pinned`, `the_sprite_sheet_is_unchanged`). A sprite change is a change to every effect that draws it: shoot the strips that use it, then repin.
- Numbers live in code (`table.rs`), never in docs.
