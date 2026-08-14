# reference/dumps/20260814-systems-survey.md — a relayed survey, raw

**A dump, not a doc** (`SOURCES.md` §"the pipe back in"): the operator
pasted this on 2026-08-14; it was written by a second assistant with live
internet access. Nobody in this loop read the pages behind it, so every
claim below is **second-assistant summary tier** — `RIPLIST.md` §1c's
posture — except where a same-day pass upgraded or corrected it:

- **The OEZP violence study: read whole at primary tier the same day**
  from this box. `RIPLIST.md` §5.6 is the record and supersedes the
  numbers below where they differ. One wobble found: "offensive actions
  fall by 19%" conflates the per-player mean (2.84 → 2.31, −19%, which
  the paper states) with the absolute count (416 → 322, −23%). And the
  summary missed the sharpest structural fact: the 20 purely-offensive
  players offend *below* the population mean (2.45 vs 2.84 — removing
  them raises the remainder to 2.91); the concentration is 7 hyperactives
  at 13.42 offensive acts each.
- **The July 2, 2026 "Common Ground" update** (clan tables, rentable
  shops, Softcore raid windows 18:00–21:00 server-local, the 1-hour
  cupboard aging) and **the Oct 2025 "Meta Shift"** (blueprint fragments:
  5 basic → WB2, 5 advanced → WB3, non-craftable, monument-sourced, with
  the stated intent of pushing players back into contested monuments)
  are **corroborated by an independent search pass the same day** — two
  summary routes agreeing, still nothing primary. The Nov 2025 BP-wipe
  reversal also corroborates (PC Gamer covered it).
- Everything else — Feb 2026 modular boats, the Deep Sea, May 2026
  workbench rares, the industrial throughput constants, "clans disabled
  in Hardcore", cargo's historical 2–4-day cadence, the marketplace's
  exact fees and timers — is a **single unverified summary**. The
  research pass that consumes a claim re-sources it first.

What consumed this so far: `SOURCES.md` §0 item 14 (closed), §3b (the
systems queue), `RIPLIST.md` §5.5–§5.6. The text below is verbatim.

---

Yes. I found a few things that are more consequential than another pass over gather yields.

First, one concrete win: the Austrian paper is readable from here now through the journal’s alternate hostname, including the full PDF. So the highest-priority “browser blocked” row in reference/SOURCES.md can be closed.

The violence paper gives a useful result — but not quite the one SOURCES.md expects

The study watched 146 vanilla-Rust players for 30 minutes each: 73 player-hours total, drawn from 404 Twitch users / 640 collected videos. It excluded modified servers and stream-sniping cases.

Its results:

Per-player interaction share: 35% non-violent + 17% defensive = 52% non-offensive, versus 48% offensive.
111/146 players, 76%, committed at least one offensive act.
In absolute counts there were 359 non-violent, 140 defensive, and 416 offensive interactions — 915 total.
The 7 most aggressive players (5% of the sample) produced 94 offensive interactions. Remove them and offensive actions fall by 19%, while offensive acts/player fall from 2.84 to 2.31.
The authors explicitly warn that streamers are probably biased toward more violent/extravagant play because action attracts viewers.

That is excellent evidence for Gates, but I would not use it to derive a 2–5× farm-rate threat divisor. The paper does not measure farming trips, carried value, trip interruption, deaths, or loss-of-load. So the previous framing that this paper would supply “the missing magnitude” for the 19× farming gap is too strong.

What it does settle is the shape of threat.

Gates already says threat should affect trip shape rather than act as a flat multiplier. This paper supports that unusually well: aggression is widespread—76% offend at least once—but also heavy-tailed, with a tiny fraction of players generating disproportionate violence, while most interactions overall are still non-offensive. That argues for threat being spatially and socially concentrated: dangerous actors, hotspots, risky routes, and occasional catastrophic loss, not a uniform “everyone farms 3× slower” tax.

I'd record the study as EXACT study statistics / INDIRECT applicability to farm throughput.

The biggest untouched subject is actually mobility + logistics

This is the one I would research next.

Gates' measured walker gets 969 wood/min at 71.6% duty, around 19× the declared 50/min; the repo has correctly concluded that the missing term is overwhelmingly world logistics rather than node payout.

But there's a subtle problem with the current reference model: Rust does not have one logistics divisor throughout a wipe.

Players acquire horses, cars, helicopters, boats and storage-capable vehicles. The 2020 modular-vehicle update was significant enough that Facepunch widened and smoothed roads and changed how helicopters entered the economy; cars themselves have chassis, modules, locks and maintenance. Then in February 2026 they added player-built modular boats whose sails/engines change handling and speed, with fuel-burning engines explicitly priced as expensive mobility for escaping/chasing. They also made the Deep Sea an intermittently opening destination.

That means I'd create reference/LOGISTICS.md, with one central measurement:

How does effective collection throughput change from naked → established solo → established group?

Not “what speed is the horse?” The useful table would be:

term	naked	established solo	group
travel speed
carried resources/trip
acquisition/fuel overhead
deposit-trip interval
exposure time per unit gathered
transport loss probability
effective gather/min

That would test whether RIPLIST's 10–30× logistics term is really a constant or mostly an early-wipe phenomenon. I suspect the latter, but that needs measurement rather than assumption.

And it directly affects Gates' weekly pacing.

Closely behind that: industrial automation

There is basically another whole side of Rust logistics inside the base.

The 2023 Industrial Update added storage adapters, conveyors, splitters/combiners, automatic crafting and electric furnaces. Conveyors can filter 12 item/categories and maintain minimum/maximum stock targets. More usefully for Gates, Facepunch published actual throughput limits: 16 visible input/output containers per conveyor, 32 items of a stack moved per tick, default move/craft attempts every 5 seconds, and default 12 whole stacks moved per industrial tick.

This matters because Rust progressively removes domestic labour from established players.

So the complete logistics decomposition is more like:

world acquisition → physical transport → storage sorting → processing → crafting → redeployment

Gates' current farm analysis mostly measures the first two.

reference/INDUSTRY.md could answer a much more interesting question than “should Gates have conveyor belts?”:

Which chores does a mature survival game deliberately automate away as the wipe progresses, so late-game players spend their time contesting rather than sorting boxes?

That is directly relevant to keeping week 3 alive.

Social organization is suddenly a particularly good research target

This one became much more useful last month.

Rust's July 2, 2026 update added a first-party clan layer: clans are larger/more structured than teams, have roles and permissions, their own chat and announcement/MOTD, are created at physical Clan Tables, expose clan identity on nameplates, and show nearby clan members on the map. Interestingly, Facepunch disables the whole clan system in Hardcore mode.

That's a fascinating set of choices for Gates because it separates several things people normally lump together as “teams”:

membership → authority → communication → identification → positional information

And each can have a different scope.

I would research reference/SOCIAL.md before Gates grows much beyond ad-hoc grouping. Questions worth extracting:

What exactly can each clan role authorize?
What is team size versus clan size?
Which permissions are per-person versus inherited?
How far does “nearby” map visibility extend?
What information disappears in Hardcore?
How do cupboards, locks, turrets and respawns interact with team/clan membership?
What happens when someone is kicked while assets remain authorized?

For a 100-player shard, that probably matters more than another weapon table.

Safe zones, markets and raid windows form one surprisingly coherent research topic

Gates already has a haven and a future bank/extraction seam, so this material maps unusually cleanly.

Rust's Outpost combines a protected safe zone, public crafting/processing stations, vending, and a drone market. The 2021 marketplace deliberately removed danger from the actual item exchange: a purchase costs 20 scrap delivery, can take up to five minutes, and the terminal is reserved to the buyer for ten minutes; delivery drones cannot be attacked. Facepunch explicitly said the purpose was to make trading safer and encourage more trading.

Then July 2026 added a much stranger experiment: rentable shops with real-time hourly rent, a 12-hour minimum opening stake, escalating ×2/×3/... takeover costs, and 24-hour recovery for belongings after eviction.

And in the same patch, Softcore received configurable raid windows, defaulting to 18:00–21:00 server-local time. Outside them, TC-covered building blocks and doors are protected; newly placed cupboards must age for one hour before granting protection, specifically to prevent abuse.

That gives Gates public examples for three different problems:

safe exchange, persistent economic occupancy, and offline-raid protection.

I would probably call the doc reference/SAFEZONES.md or TRADE.md, but I'd keep those three mechanisms separate. In particular, the raid-window implementation is worth studying before Gates invents its own answer to “I can't be online 24/7 for a weeks-long shard.”

Events are the closest shipped analogue to WORLD.md's extraction windows

This may be the strongest connection to the new Gates fiction.

WORLD.md proposes server-wide extraction windows because a known opening time causes players to converge on a route. Rust has spent years doing exactly that with server events.

The original Cargo Ship is a particularly clean specimen: historically it triggered every 2–4 in-game days, announced itself spatially by circling the island, was visible on the map/audible at long distance, required a boat to access, contained a timed hack, and forced the winner to defend the moving objective from everyone else.

The 2026 Deep Sea uses the same idea at a larger scale: it opens and closes intermittently “like a world event,” contains high-value destinations, AI-protected objectives and player conflict, and requires players to invest in boats before participating.

So reference/EVENTS.md should probably measure:

cadence · warning time · access investment · visible telegraph · objective dwell time · reward · exit exposure

Those are exactly the knobs an extraction window needs.

And this is one place where I'd research several generations of the same mechanism rather than just current values: airdrop → helicopter → cargo → oil rig/hackable crates → wipe events → Deep Sea. Facepunch has effectively run a decade-long series of experiments in “scheduled reasons to leave your base.”

Progression deserves its own reference doc too

Not because Gates should copy Rust's tech tree, but because the public history contains repeated examples of why progression systems fail.

The 2020 tech tree replaced random experimentation with deterministic visible paths so unlucky players still accumulated guaranteed progress.

Then in October 2025 Facepunch explicitly said progression had become too fast and scrap too abundant, hurting world PvP, and inserted Blueprint Fragments into key monuments/events specifically to push players out of their bases, contest territory, and slow clan snowballing.

One month later they swung the other direction: because most servers weren't BP-wiping, fresh map wipes skipped early/midgame entirely. They wiped BPs and sharply reduced unlock costs to restore early-game structure without making the reset a grind.

And as recently as May 2026, workbenches gained rare non-craftable upgrades.

That sequence is much more useful than an item-cost table. The lesson is that progression exists partly to manufacture reasons to move through dangerous space, not merely to delay access to guns.

I'd call it reference/PROGRESSION.md, and the primary metric I'd want is what fraction of progression can happen while staying inside your base at each stage.

Moderation is still worth doing — and the public data is better than I expected

The proposed MODERATION.md is viable at better than search-summary confidence.

The official wiki documents the F7 reporting interface and even the server-side report payload: server owners can print summaries to console or configure an HTTP endpoint; reports include reporter/target identity, report type, server/location context, play time and optionally a screenshot.

combatlog <Steam64ID> gives admins recent incoming/outgoing damage history for another player.

And the demo system has a conceptually important split: ordinary client demos versus full-server demos intended to record every action of all players, with configurable disk allocation and upload URL. The wiki currently says the latter's playback client is still under development.

That suggests a good Gates moderation stack is not “more logging.” It is:

report → immutable identifiers/context → compact adjudication log → escalate to expensive replay evidence only when necessary → action with reason/duration

That would pair very naturally with Gates' deterministic/replayable core.

My priority order

If this were my research queue, I'd now do:

Close the OEZP row immediately. The data is available; importantly, change its conclusion from “threat magnitude” to heavy-tailed threat shape.
LOGISTICS.md — transport/carry/fuel/deposit loops by wipe stage. This is the one most likely to change the interpretation of the measured 19× gap.
EVENTS.md — because it gives WORLD.md's extraction/window design a decade of shipped comparables.
PROGRESSION.md — especially the 2025–2026 sequence, because Gates is explicitly a weeks-between-resets game.
SOCIAL.md — the July 2026 clan implementation makes this newly fertile.
INDUSTRY.md — what mature players stop doing manually.
MODERATION.md — less game-design leverage, but probably the most important missing shard-operations document.
Safe zones/trade/offline-raiding, either one doc or split when the material becomes large enough.

The deeper finding is that Gates has already researched Rust's micromechanics unusually well. The next gains aren't likely to come from another “how much sulfur?” lookup. They're in Rust's systems over time: how a naked's logistics become a veteran's logistics, how a solo becomes a clan, how a quiet map is periodically forced to converge, how chores disappear, and how a month-long world remains worth logging into.
