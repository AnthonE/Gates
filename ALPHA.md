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
  ⚠ **The screen has a map on it since 2026-08-16 and "no map position"
  still holds** (operator, `DECISIONS.md`; bag choice v0). The two are not
  in conflict: the rule is about where *you fell* — a screen that said so
  would hand the raider standing over your body a pin to the base they just
  cleared — and what the map marks is **your own bags**, which are yours
  already. No corpse marker, no player marker, and the bag row is not drawn
  at all for a player who has none. `ui::map::no_wake_map_marks_the_corpse`
  is the structural half, because the way this breaks is somebody helpfully
  adding a "you died here" dot.
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
  on the bidi lane. No voice at alpha — **scope, and no longer "a rabbit
  hole with its own transport"**, which is what this line said until
  `reference/VOICE.md` was written: that rabbit hole is only reachable from
  a P2P start, the reference fell into it and was forced out of it under
  attack (Devblog 189), and we have no P2P anything. A voice radius sits
  inside `AOI_ENTER_CM`, so the routing is one compare against a set the AOI
  scan already produced (§9.2). Revisit post-alpha **(knob)**; §9.1 is the
  one decision that is expensive later.
- **Nametags**: crew-less alpha — names render only within 8 m and only
  when aimed at **(knob)**. Identity ambiguity is content in this genre.
- **Day/night**: 45-min cycle **(knob)**, server clock in the G channel;
  night is genuinely dark (torch tradeoff: light = visibility = target).

## 2 · Staged economy arming (do NOT launch coins into an unstable game)

**What stages is the CLAIM RAIL, not the currency** (operator,
2026-08-10). Two things share the name OBOL and only one of them is a
token risk:

- **Carried OBOL is an ordinary item** — paid by the recycler, dropped on
  death, raidable, spent at the research table. That is scrap's job, it is
  **live from A1**, and there is nothing to stage: an item stack in a save
  file cannot be cashed out.
- **The claim rail is the banked balance** — a ledger row keyed to a
  wallet, its merkle export, redemption on-chain. That is what a netcode
  bug could turn into a token incident, so that is what arms in stages,
  each its own switch. The break-things-freely privilege (which expires
  when the operator posts) is spent on A1, not A3:

- **A1 — the survival economy, whole** (first playtests): OBOL is earned
  and spent in world, no bank terminal, no ledger, skins vendor dark.
  Everything a player touches is here; the game must be fun before a
  balance is worth anything.
- **A2 — the ledger, off-chain**: the bank terminal opens and the
  carried/banked split goes live, balances accruing — **no merkle export
  yet**; they are real and unclaimable while wipe cadence and dupe-testing
  settle. Posted plainly on the site: "banked OBOL becomes claimable at
  A3."
- **A3 — the claim + the counter**: merkle export on the scry claim rail,
  skin catalog opens (SCRY/MYRRH), munus-first-sale delivery includes a
  recorded round. This is the moment that needs the operator's announce,
  and everything before it is rehearsal.

Dupe posture from A2 onward: any inventory/econ transaction that fails
verification twice quarantines the item stack and flags the WAL span for
replay — the deterministic replay is the dupe investigation tool.

## 3 · Ops (the server is a service the moment one stranger joins)

- **Admin lane** — **built 2026-08-11** (admin v0, `DECISIONS.md` §open):
  wallet-allowlisted, on the chat lane rather than a new message (no wire
  change; `protocol/admin.rs` has the argument). `/kick` `/ban` `/say`
  `/tp` `/give` `/save` — and `/tp`/`/give` are commands, so an admin act
  IS in the stream a replay reads, which is what this line asked for.
  Still owed: a ban that survives a restart (memory only today), IP
  banning (only the wallet is banned — an IP is not proved by anything),
  and `wipe-now`, which is §0q item 2's unscoped mechanism.
- **Config**: one `shard.toml` — every knob in these four docs reads from
  it; a knob not in the file doesn't exist.
- **Supervision**: systemd unit, restart-on-exit (DESIGN L7 contract),
  ulimits + the UDP sysctls from NETCODE §2.2 in the unit file.
- **Backups**: snapshot + WAL shipped to object storage every 30 min and
  at wipe; a wipe archives (never deletes) the final state + hash chain.
- **Observability**: a status JSON on localhost — **built, and narrower
  than this line** (`status.rs` serves players/max/tick; the other four
  are counters nothing publishes yet) — plus the **anomaly log**, built
  2026-08-11, which is what §6's "zero silent failures" is measured
  against. Still owed: the tiny public shard page, and a reader that
  turns a session's log into a verdict.
- **Client error capture**: the browser shape of this is retired with the
  browser client (`window.onerror` describes nothing that exists). A
  native panic hook posting the build hash is the replacement and is
  **not built**; the server half it would pair with now exists.
- **Hosting**: one 4-core/8 GB VPS with UDP-tolerant DDoS filtering
  **(knob: provider)**, game subdomain + ACME cert, no CDN in front of
  the UDP port (there is nothing to CDN). Reference-hardware perf gates
  run on this exact box class.

## 4 · Product & launch

- **The name** — `Gates`, spoken 2026-07-31 (`DECISIONS.md`). Settled; no
  longer blocks anything public.
- **Landing page**: one static page on the game domain — what it is, the
  what the house does not sell (`BUSINESS.md`), wipe
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

Vehicles · farming · electricity · ~~NPC/animal AI~~ **(animals un-cut
2026-08-08, see below)** · voice chat · teams UI
· ~~server browser (one shard)~~ **un-cut 2026-08-06, see below** ·
anti-cheat beyond authoritative sim + anomaly log · localization · mobile
controls (**and now mobile itself**: this read "Android runs, but it's a
desktop game", which was true of a web page opened on a phone. The browser
client is cut, the shipping artifact is a desktop depot, and nothing runs on
Android) · monuments beyond the
haven (TERRAIN's pad carver is the hook) · skin trading/editions (A3
sells; trading is its own later gate).

**Animals are back in** (operator, 2026-08-08 — `DECISIONS.md`: *"let's get
a pig in"*). The cut was written as "NPC/animal AI", and what landed is a
fixed 64-slot roster of pigs that wander, flee, pay fat and cloth when
killed — and, since 2026-08-11, **fight back** (mob attack v0: a whole pig
charges and bites, a hurt one flees, `DEATH_BY_MOB` on the wire). There is
still no AI *system* — nothing hunts across the map or packs — so the
expensive part of the original cut stands. It was cheap because the walls had already paid for it: the terrain
is a pure function, so there is no navmesh to bake, and an animal drives the
same `movement::step` a player does. `reference/ANIMALS.md` is the research
and §9.5 lists what v0 does not have.

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
