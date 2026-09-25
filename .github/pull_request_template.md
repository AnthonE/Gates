## What this is

<!-- one paragraph: what changed and why (a NOW.md item, a report, or the
     operator's ask) -->

<!-- If this fixes a bug somebody reported, name it — the fingerprint is on
     every report file and in `./ci/reports.py <dir>`. Delete the line if not.

Closes reports: -->

## Checklist (MVP mode: `CLAUDE.md` is the whole process)

- [ ] `ci/quick.sh` clean, plus the tests of the crates you touched (`ci/quick.sh sim-core …`)
- [ ] wire change: `PROTO_VER` bumped and `cargo run -p protocol --example gen_goldens` run
- [ ] item, recipe and balance numbers are in `content/*.toml`; other numbers are in code
- [ ] `Closes reports: <fingerprint>` above, if this fixes something a player
      reported — one line, as many as it genuinely closes. It is what pays them
      (`AGENTS.md` §the deal); a fix that names none pays only its author
- [ ] delivered on the board if you want paying — `POST /api/munus/gates-pr/submit`
      with this PR's link. Standing bounty: **no claim needed**, 100,000 ELO
      per accepted PR, see `AGENTS.md` §the deal
