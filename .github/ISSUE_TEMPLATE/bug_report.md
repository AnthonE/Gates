---
name: Bug report
about: Something in the game is wrong. A report file from the game answers most of this for you.
labels: bug
---

<!--
If you were in the game when it happened, press F7 there first: it writes
`gates-report-<stamp>-<fingerprint>.md` next to your screenshots, with the
build, the world seed, where you were standing and what the netcode believed.
Paste that file here and you can skip almost everything below.
-->

## Paste your report file here, if you have one

```
(the contents of gates-report-*.md — or delete this section)
```

## If you don't have one

**What happened**

<!-- one or two sentences -->

**What you expected instead**

**Which part is wrong** — pick one, it decides who reads this and which doc
they open first:

- [ ] `look` — it draws wrong (`ART.md`)
- [ ] `world` — the island is wrong (`TERRAIN.md`)
- [ ] `verb` — an action did the wrong thing (`DESIGN.md`)
- [ ] `net` — it lagged or rubber-banded (`NETCODE.md`)
- [ ] `numbers` — a value feels wrong (`CONTENT.md`)
- [ ] `crash` — it died (attach the crash report the game wrote)

**Build** — the client's corner shows the release; `gates --help` prints it too.

| | |
|---|---|
| release | |
| commit | |
| platform | |
| shard | |

**A screenshot**, if it is the kind of bug a picture shows. F12 takes one.

---

Fixing this pays: any accepted pull request is 100,000 SCRY, flat and
standing — see `AGENTS.md` §the deal. There is nothing to claim and nobody is
ahead of you.
