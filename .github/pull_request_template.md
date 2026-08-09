## What this is

<!-- one paragraph: which NOW.md item / board quest, and what changed -->

## The checklist (AGENTS.md is the law here)

- [ ] `./ci/gates.sh` green locally
- [ ] no invented numbers — every tunable is spoken in `DECISIONS.md` or ships its documented default
- [ ] no wall weakened — if a gate or golden changed, the same commit says why
- [ ] one crate; `protocol` / `limits.rs` changes are alone in this PR
- [ ] content changes are `content/*.toml` only, never code
- [ ] delivered on the board if you want paying — `POST /api/munus/gates-pr/submit`
      with this PR's link. Standing bounty: **no claim needed**, 100,000 SCRY
      per accepted PR, see `AGENTS.md` §the deal
