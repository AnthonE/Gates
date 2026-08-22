# findings/ — design notes too long for `NOW.md`

`CLAUDE.md` §loop discipline says detail worth keeping goes to
`DECISIONS.md` §open (a knob) or a **`gates-loop/findings/` note**, with the
`NOW.md` item pointing at it in one line.

**This is not that directory, deliberately.** The loop's harness lives at
`/mnt/hive-data/gates-loop`, *outside* this repo, and `CLAUDE.md` says why in
as many words: if it lived in here, an agent would have write access to the
criteria it is scored against. Creating `gates-loop/` in-tree would establish
exactly that path. The harness is also simply absent on the box these notes
were written on, so its `findings/` was not reachable either.

So: same job, different name, no collision with the harness. A note here is
**a plan or a measurement, never a law** — the four docs that bind are the
walls, the gates in CI, the operator's spoken word, and measurements
(`CLAUDE.md`). A note that disagrees with a passing gate is wrong.

One rule, inherited: **every claim carries the command or the `file:line`
that produced it.** A note whose numbers nobody can re-run is prose.
