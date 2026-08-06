# Gates · ALPHA.md — everything else the alpha needs (v0.1)

> The gap sweep: what a playable, announceable alpha requires beyond
> `DESIGN.md` + `NETCODE.md` + `TERRAIN.md`. Ordered by "the game is a
> lie without it" → "ops" → "product." Knobs marked **(knob)**. One real
> design hole found during this sweep is fixed here: **sleeping bags.**

## 1 · Gameplay pieces the docs missed or under-specified

- **Sleeping bags (the respawn anchor)** — not in DESIGN.md, and the loop
  doesn't work without them: death without an anchor = random beach spawn,
  which makes raids unpunishable and basing pointless. A bag is a cheap
  deployable (cloth), class S, wallet-bound, respawn-on-it with a
  per-anchor cooldown (~5 min **(knob)**); a bed later halves it. Cap
  bags per player (~8 **(knob)**). Destroying someone's bags is legitimate
  raid strategy — that's the point of them.
- **Respawn flow**: death screen (who/what killed you — range and weapon,
  no map position), choose beach or a bag, spawn with nothing. No
  spawn protection anywhere, haven excepted.
- **First-person feel checklist** (client acceptance, not features):
  fixed-timestep-decoupled camera, ~10 Hz-to-60 fps interpolation never
  visible, sprint/crouch states in the input bitfield (already sized),
  fall damage from impact velocity, headbob off by default, FOV slider
  75–110.
- **Hotbar + inventory UX**: 6 hotbar slots, 24 inventory, drag/drop DOM
  overlay (per DESIGN §9 — UI never in the render loop), radial menu for
  build-piece selection on hold, hold-E interactions with a progress ring
  (loot, revive later). Server validates every move (it already does);
  the UI is allowed to be plain — clarity over chrome at alpha.
- **Chat**: global text + 20 m local **(knob: local on/off)**, profanity
  untouched (survival chat is survival chat), rate-limited server-side,
  on the bidi lane. No voice at alpha — a rabbit hole with its own
  transport; revisit post-alpha **(knob)**.
- **Nametags**: crew-less alpha — names render only within 8 m and only
  when aimed at **(knob)**. Identity ambiguity is content in this genre.
- **Day/night**: 45-min cycle **(knob)**, server clock in the G channel;
  night is genuinely dark (torch tradeoff: light = visibility = target).

## 2 · Staged economy arming (do NOT launch coins into an unstable game)

The coins are designed (DESIGN §3) but they arm in stages, each its own
switch, so a netcode bug never becomes a token incident — and the
break-things-freely privilege (which expires when the operator posts)
is spent on A1, not A3:

- **A1 — pure survival** (first playtests): OBOL faucets OFF, salvage
  drops components only, no bank terminal, skins vendor dark. The game
  must be fun broke first.
- **A2 — OBOL on, off-chain**: faucets/sinks live, carried/banked split
  live, ledger accruing — **no merkle export yet**; balances are real but
  unclaimable while wipe cadence and dupe-testing settle. Posted plainly
  on the site: "banked OBOL becomes claimable at A3."
- **A3 — the claim + the counter**: merkle export on the scry claim rail,
  skin catalog opens (SCRY/MYRRH), munus-first-sale delivery includes a
  recorded round. This is the moment that needs the operator's announce,
  and everything before it is rehearsal.

Dupe posture from A2 onward: any inventory/econ transaction that fails
verification twice quarantines the item stack and flags the WAL span for
replay — the deterministic replay is the dupe investigation tool.

## 3 · Ops (the server is a service the moment one stranger joins)

- **Admin lane**: wallet-allowlisted admin commands on the bidi stream —
  kick, ban (wallet + IP), teleport, give, broadcast, save-now, wipe-now.
  Every admin act is a WAL event (visible in replay; abuse of admin is
  visible too, which is the scry-brand posture).
- **Config**: one `shard.toml` — every knob in these four docs reads from
  it; a knob not in the file doesn't exist.
- **Supervision**: systemd unit, restart-on-exit (DESIGN L7 contract),
  ulimits + the UDP sysctls from NETCODE §2.2 in the unit file.
- **Backups**: snapshot + WAL shipped to object storage every 30 min and
  at wipe; a wipe archives (never deletes) the final state + hash chain.
- **Observability**: a status JSON on localhost (tick p99, players,
  entities by class, WAL lag, datagram loss estimate) + a tiny public
  shard page (players online, wipe clock, uptime) — publish the real
  numbers, zeros included; that's house style.
- **Client error capture**: window.onerror + unhandledrejection POSTed
  with build hash (no third-party SDK at alpha); server pairs it with the
  anomaly log.
- **Hosting**: one 4-core/8 GB VPS with UDP-tolerant DDoS filtering
  **(knob: provider)**, game subdomain + ACME cert, no CDN in front of
  the UDP port (there is nothing to CDN). Reference-hardware perf gates
  run on this exact box class.

## 4 · Product & launch

- **The name** — `Gates`, spoken 2026-07-31 (`DECISIONS.md`). Settled; no
  longer blocks anything public.
- **Landing page**: one static page on the game domain — what it is, the
  never-table (what money can't buy, verbatim from DESIGN §3.3), wipe
  clock, wipe/uptime numbers, and — as of `DECISIONS.md` 2026-08-05 — **two**
  buttons rather than one: *buy* (the native client, the official armed
  shards) and *try it in the browser* (the demo, unarmed shards). **The
  second button is dead.** The browser client is cut (`DECISIONS.md`
  2026-08-06) and there is nothing behind that link; the page ships one
  button until a replacement is spoken (§open, "the board's playable link").
  Shipping a *try it* button over a retired client is worse than shipping
  none — it is the dark-panel defect with a purchase attached. Price and
  what the purchase gates are §open, so the page cannot be finished until
  they are spoken. A card on scry's build page links here at A3, not
  before **(operator)**.
- **Alpha predates the store**: the A1 cohort below plays free — there is
  nothing armed to gate and no counter open yet (`DECISIONS.md` §open).
- **Playtest cohort**: the SCRY holder community is a real, waiting
  audience — that's the documented state of the town — so A1's 10–20
  testers come from there via the operator's channels when he chooses.
  Feedback lands in one channel (Discord/Telegram **(knob)**) + a `/bug`
  chat command that stamps tick + position into the anomaly log.
- **Session metrics** (file ledger, honest, published): joins, median
  session length, deaths, structures placed, wipe-over-wipe retention.
  No third-party analytics.
- **Trailer material**: the deterministic replay is the capture tool —
  record a raid's WAL, replay with a free camera for footage. Cheap and
  exactly on-brand.

## 5 · Explicitly cut from alpha (so nobody re-litigates silently)

Vehicles · farming · electricity · NPC/animal AI · voice chat · teams UI
· ~~server browser (one shard)~~ **un-cut 2026-08-06, see below** ·
anti-cheat beyond authoritative sim + anomaly log · localization · mobile
controls (Android runs, but it's a desktop game) · monuments beyond the
haven (TERRAIN's pad carver is the hook) · skin trading/editions (A3
sells; trading is its own later gate).

**The server browser is back in** (operator, 2026-08-06 — `DECISIONS.md`).
It was cut on the reasoning that alpha runs one shard, so a browser has
nothing to browse; the operator asked for the intro screen and the list
directly, which outranks the cut. Two things make it cheap enough that the
original reasoning no longer holds either way: scry's launcher has carried
a dark `ServersWindow` and the `scry-shardlist-v1` shape all along waiting
on a title to publish one, and a one-shard list is still a list — it is
what turns "type an address" into "pick the shard", and it is the only
screen a player sees before the world. It does not un-cut anything else on
this line, and in particular it arms no economy stage: the browser lists
shards, and which of them are armed stays §2's question and an operator
act.

The anti-cheat cut is now load-bearing rather than merely deferred
(`DECISIONS.md` 2026-08-04): **the armed set is the perimeter.** Alpha is
A1 — unarmed, no redeemable coin — so there is nothing on an alpha shard
worth cheating for, and the protection an official armed shard eventually
carries is a §2 arming question, not an alpha one. What alpha still owes
is the evidence: the anomaly log and the replays are what A1 is supposed
to produce, and both later measures (occlusion culling, offline aim
analysis over the WAL — `NOW.md` 18) read from them. A kernel anti-cheat
is not on this ladder at all; it has no native client to attach to and it
would ban the agent players the training goal depends on.

## 6 · Order of work (folds into DESIGN §11)

M0 shell → M1 verbs (+ **bags**, hotbar, chat) → M2 combat (+ death/respawn
flow, day/night) → M3 OBOL machinery dark + ops hardening (admin, backups,
status, error capture) → **A1 playtest** → tune → M4 arm A2, then A3 with
the announce and the board delivery.

The gate for calling it "alpha" is not a feature list — it's the CI walls
green plus one number: **20 strangers, one wipe cycle, p99 tick under
budget, zero silent failures in the anomaly log.** Everything else is
taste.
