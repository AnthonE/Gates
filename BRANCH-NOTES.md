# Branch notes — `claude/upbeat-gauss-om06az`

One slice, one commit: **wounded v0** — a lethal swing, bite or body shot
lays the body down instead of killing it, and a minute later a hashed roll
stands it up or makes the corpse. This replaces the
`claude/game-visual-improvements-fj1qyz` note.

**Read `reference/WOUNDED.md` first** (§0 for where every number came from,
§9 for what was built against the tree), then `DECISIONS.md` §open's
"wounded v0" row and `NOW.md` §0wnd.

## What landed

- `crates/sim-core/src/wound.rs` — the window (40–50 s), the odds (20 % +
  up to 25 % for full meters), the crawl (a third of a walk), the minute
  between downs; integer arithmetic over `rng::cell_hash`, so a replay
  rolls the same die.
- `World::down_or_die` between the funnel and the corpse at every kill
  site; `World::wound_tick` in the live and sleeping branches; `EV_DEATH`
  and the death count moved into `World::die`.
- Wire v63: `EntityState::wounded`, `SUB_WOUNDED`, `SUB_RECOVERED`. Save
  formats 6 (store) and 13 (world) carry the bit and its two clocks.
- Client: the predictor crawls the same frames through the same function;
  camera drop and roll, a vignette, the reference's two-number line, two
  toasts, `Cue::Death` on the fall; remote bodies hold `Death01`.
- Gates: `sim-core/tests/wounded.rs`, two role checks, every ledger the
  change touched (§9.7 lists them), the replay golden regenerated with its
  reason beside it.

## What is measured, and what is not

Measured: the odds land at the rate they name over ten thousand draws;
the crawl's ratio to the walk is `1/CRAWL_DIV` over the same ground; the
loot storm's worst tick still overflows the ring with a fifth of the
population on the ground (its kit grew a fourth stack to keep that claim
honest — the file says why).

Not measured: **the look.** No page describes the reference's wounded
camera and this box has no GPU. `CRAWL_EYE_M`, `WOUND_ROLL_RAD`,
`WOUND_DROP_S` and the vignette are a first cut; `WOUNDED.md` §9.5 is the
checklist for the person who boots it.

## What remains

`NOW.md` §0wnd, in order: the hands-on revive (hold E 6 s), the syringe on
a downed body, the medkit-in-belt rule, refusal toasts while down, a drag
clip and a voice for the fall, and whether the odds should be resent as
the meters drain.
