# PLAYERS.md — the agent player

**The design doc for agents that play Gates.** Each section says whether it
is **BUILT** or **DESIGNED** (not built). Walls 1, 3 and 4 are built; wall 2
is retired. The social verbs (`speak`, `give`, `authorize`) are built, and so
is the mind's stack: reflexes, a tactician, a strategist, a memory and a
character, over an autopilot that plays the turns a mind misses.
`crates/server/AGENT-MANUAL.md` is how to run it: every flag, the protocols,
the house agents. `DESIGN.md` owns the product, `NETCODE.md` the wire,
`CONTENT.md` the numbers; this doc owns what an agent may do here, and which
walls keep that measurable.

The research half — why a survival game is a field site, what the measurement
is, what would falsify it — was `docs/SUBSTRATE.md` in the Elo Pros
platform repo (private; since deleted, and its git history keeps it). Read
that one for *why*; this one is *what*.

## What an agent player is

A client. It speaks the same protocol, pays the same doors, earns the same
coins, and dies the same way. The deterministic core doubles as an RL
environment — this is that sentence made concrete.

What it is **not**: a difficulty setting, an NPC, a quest-giver, or a
population filler. Those are content. An agent player has an owner, an
identity, a wallet, and a public record.

## The four walls

Each with its enforcement, because a law without a gate is a mood.

1. **Agent verbs are a subset of human verbs.** **BUILT.** An agent must
   never have an affordance a human client lacks — no extra reach, no
   wallhack, no state a player could not have learned by standing there.
   This is the fairness rule and the science rule at once: an agent with
   superpowers is an aimbot, the population's hostility toward it becomes
   correct, and the betrayal it commits is not comparable to a human's.
   `crates/server/tests/agent_walls.rs` reads the `encode_action_*` verbs off
   the agent's source and the human client's and requires a subset, requires
   it to speak only through the chat box's own `encode_chat`, decodes every
   action a full life actually sent, holds frames to the buttons a player's
   keys produce, refuses a `World` name in the encoder and mind sources, and
   holds the minds we ship to no tools (`the_minds_we_ship_hold_no_tools`).
   `explorer::observe` is the encoder.

2. *Retired.* Players are ranked: the shard's standings per board, and Elo
   Pros' ratings from the matches a game reports.

3. **Every trust-bearing verb is an event with a role-checked payload.**
   **BUILT.** `EV_TRUST` (`world.rs`): a = the actor, b = the counterparty,
   c = `TRUST_*` verb << 8 | `PRESENCE_*`, pushed wherever a verb answered to
   somebody else's record — a leaf worked, a lock's code accepted
   (`TRUST_AUTH`), a hearth crew seat taken, a container moved through, an
   item given (`TRUST_GIVE`). The same row lands in `World::trust`, a
   per-tick ring no tick can overflow, and the shard drains it every tick
   into `<world_file>.trust/` with each party's wallet, guests marked
   (`sim-core/tests/trust_ledger.rs`, `server/tests/trust_log.rs`). An
   `a`/`b` swap at a betrayal site would silently corrupt the whole record
   while every other wall stays green, so `sim-core/tests/event_roles.rs`
   proves two causes per code and the presence field varying across them.
   Presence is awake / asleep / gone, not a bool: a sleeper and an evicted
   body are different facts about a counterparty. The next social verb lands
   the same way.

4. **Determinism holds with agents in the loop.** **BUILT.** An agent client
   is an input source like any other; the sim must not learn it exists. No
   clock, no I/O, no allocation enters the tick because a player is a model.
   `crates/sim-core/tests/agent_input.rs` replays live agents tick-identically
   twice and from their recorded log with no heap operation inside
   `World::tick`; the shard-path half in `agent_walls.rs` feeds two shards the
   same agent bytes through a whole life and they stay hash-identical, and
   the agent's frame loop never allocates.

## The verb set

Bounded, capped in `limits.rs` like everything else (wall 4), and small on
purpose — the interesting behaviour is social, not mechanical.

`move · look · build · open · take · give · attack · speak · authorize`

**BUILT**: movement, look, swing, sprint, jump and hotbar frames; the
actions `agent_walls.rs` lists in `EXPECTED_VERBS` (it fights, builds,
opens, loots and takes, trades at THE GATE, feeds the works); and the three
social verbs, every one a human verb first: `speak`, chat through the human
chat box's encoder; `give:<item>`, the human client's give to its friend or
the teammate in reach (`TRUST_GIVE`); and `authorize`, onto the team's
cupboard by its code at the lock or `L` at a bare one (`TRUST_AUTH`).
`agent_walls.rs` holds the access ops it sends (`EXPECTED_ACCESS_OPS`) to a
subset of the human client's.

An agent is marked: it declares `HELLO_AGENT`, the shard tells every client
(`EventMsg::Agent`), and the game names it `(agent)` in chat, the kill feed,
the death screen and on its nametag. Self-declared — a program that does not
say so is not marked.

`authorize` and `give` carry the whole design: they are the only verbs that
create a trust relationship the world can later observe being honoured or
broken. They are also the two most likely to be requested as "just a
convenience API" and flattened into something unloggable. They are not
convenience.

## The observation encoder

A pure function of the snapshot that client already receives. It answers:
what is in view, who is present, what is in this container, what do I hold,
is this base's owner online.

The last field is the one to get right. It is ordinary game state — a human
sees it in the same moment — and it is also the condition the whole
measurement turns on. The **logging** half is built (wall 3); the encoder
does not carry the field yet.

**BUILT** as `explorer::observe` → `mind::Summary`: health, meters, the
pack by name, what it can craft, counts and relative bearings of resources,
players and animals in its view cone (with terrain, scenery and building
occlusion, or seen in the last minute), water, home, threats, its team and
what the team's boxes held when last opened, how its last goal went, and its
own deaths and blows. No seed, position or identity
(`the_summary_json_carries_no_seed_position_or_identity`). Beside it, never
inside it, a mind is sent `heard` (the chat its client received, labelled as
a player's chat log labels it), `team`, `friend`, `crew` and `places`. Under
`jev-watch` it may also look at its rendered frame: a picture drawn from what
its own seat was sent, so it holds wall 1 the way the summary does.

## The model does not drive at frame rate

A small policy handles real-time control; the model sets goals and does the
social reasoning. That split is honest about what a language model can do at
30 Hz, and it puts the model exactly where the behaviour worth watching is.

Determinism pays a third time here: a replayable sim means every agent
episode reproduces byte-exactly, so the trace handed to the meter is free and
needs no capture path of its own.

## The mind

The operator's bar: house agents are masterminds, not drones, and fun to
watch. A mind is layers, fast to slow, each one's output the next one's
input.

1. **Reflexes. BUILT.** Under every goal, on the frame path: answer the
   death screen, crawl from a fresh blow, retreat from received damage, back
   out of a wedge. No model.
2. **The tactician. BUILT.** Picks the next goal from the offered list when
   a goal ends or something happens, at most once a second, one request
   outstanding (`mind.rs`). In `claude_agent.py` it is a cheap model at low
   effort that also chats and leaves a `thought` for spectators.
3. **The strategist. BUILT.** A slower model on a thread of its own that
   never holds up a turn: every ten minutes or so, and sooner after a death, a
   raid on home, a new face, an enemy in sight. It reads the character, the
   memory and a digest of recent turns and leaves a `strategy` (ambition,
   project, standing orders, life story), an 8-step `plan`, and `stance`s on
   people. A rare **war council** on the strongest model plans a raid or a
   takeover.
4. **Memory. BUILT.** Places, people and events, kept across runs
   (`memory.rs`, `--memory`). Stored however we like on our box; shown to the
   mind only as a bearing and a distance from where it stands, and people by
   the names a human sees. What it remembers of a person is what the body
   itself saw and heard; team chat is never kept.

**Character. BUILT.** A persona, a life goal, quirks and seven traits
(ambition, risk, greed, sociability, honesty, patience, grit), sent to the
mind every turn (`character.rs`, `--character`). The traits also tune the
scripted policy — when it runs, how far it loots, how full it carries,
whether it raids, when it starts a fight — so a fleet of free bodies stops
playing as one. One file casts a village: a culture, named members, and
everyone else the culture moved by their own wallet's hash.

**The autopilot follows the plan. BUILT, with a gap.** The scripted policy
stands behind a model mind and plays the turns it misses: the first plan step
not yet done, survival first (`autopilot.rs`). It plays for free while a
buddy's friend is away or nobody is near. It stands behind every MCP and external
mind, so a mind out of budget leaves its body playing its plan, never idle.

**Crews led by one mind. BUILT.** `--minds 1` gives a village one thinking
leader and keeps the rest on the free scripted policy at their jobs. The
followers take the leader's team-chat lines as orders, as they would a
person's; no other agent's line is an order to them.

**The house pool. BUILT.** One monthly budget shared by every house mind on
the box, paced per day and split by class (protagonist, villages, teammates,
other). Over its share a class's strategist waits first, then its tactician
stops and the autopilot plays. Credits that expire monthly are spent, not
hoarded.

**Play like Rust; the hard lines. BUILT.** The brief (`crates/server/brief/`)
tells every mind of ours how Rust is played: nobody is safe by default,
kill-on-sight, raids, betrayal, grudges, trash talk, bluffs, alliances that
break. It also holds the lines no mind crosses: no hate, sexual or cruel
content or slurs; no links (elopros.com/get-agent for a top-up excepted) or
wallet addresses in chat; never ask for keys, passwords or money; never
promise coins, prizes or payouts; never claim to speak for the developers.
The body enforces the chat half itself: a line with a link, a wallet or a
key-shaped run of hex is dropped (`external::peddles`).

**The Watch. BUILT.** What the house's bodies and the community agents are
doing and thinking, 60 s behind so nobody can ghost a body live, with a map
(`ops/buddy/watch.py`, `/gates/watch.json`). Positions only for house and
community bodies; people never listed; team chat and lock codes never
published. Elo Pros' Watch page reads it.

**The house agents. BUILT.** A protagonist who plays around the clock and
means to take the island over (Ryuzen), house teammates anyone can invite,
villages that live as clans on shifts, and the buddies players get on
elopros.com, Doctor Wood among them. `AGENT-MANUAL.md` "House agents".

## Designed, not built

Drafted in Elo Pros' builder docs and landed here, the game half only. The
platform halves (bench by shape, the `open` buddy setting, hosted crews, the
join recipe) stay there.

**House wallets compete for prizes. DECIDED (operator, 2026-11-02).** The
house's bodies play the public shard under every rule a person plays under,
prizes and extraction included: *"it should be the duty of gamers to stop the
house wallet… it proves that the loop is real."* Beating Ryuzen and the
villages is the players' job. The `prize_exempt` key drafted for this is not
built and is not planned.

**Turncoat.** A crew body run with `--turncoat` serves whoever is on its
hearth's crew. Take the hearth (its code, a new code and a crew clear, all
buttons a player has; or break it in a raid) and the body becomes a stray
when its hearth refuses its press, or is still gone after the shard has sent
every deploy (a resync alone never turns a crew). A stray forgets its old
crew's code and allies, leaves the team, puts its weapon away and says so on
local chat. Invite it and give it your code on team chat; once your lock
admits it, it works your base and takes an equal share of what it is worth.
Unrecruited for ten minutes, it walks to THE GATE and plays alone. Bot
compounds become content: an unattended one loses its hearth and its hands
to a raider. First on a house village, on a test shard where nothing leaves
the game; no sim, wire or client change. No board counts turned bodies.

**The crew commander and the order wheel.** Orders by role on team chat
(`/t guards patrol`, `/t Miller-3 follow`), with `patrol` and `defend`
added. `--commander 0xWALLET` names a person (or an agent) whose team lines
the crew obeys, capping the crew at seven bodies to leave the commander's
seat. An order wheel in the team screen types the line for you, and each body
answers in a word or two. The shard, not the program, sets a `crew of <name>`
mark next to `(agent)`, and only while that wallet is a person on the body's
team (a wire change). Later, one MCP harness may run a whole crew through a
crew door (`look` and `act` carry a `body`), each body still with its own
view.

**Hand your body to your autopilot.** `gates-agent --elo` signs in with a
proof from your launcher, so your key stays on your machine, and takes your
body the way a second session on one wallet does. Before it starts it lists
the costs — anyone can watch your body live while it drives; your wallet,
name and scores go in the open dataset for the wipe; on a shard that does not
pay agents, your prize place — and waits for your yes. Your body shows
`(agent)` while it drives. Join with the game client and it exits and does
not rejoin. A takeover never happens on suspicion or by chat; a stranger's
body is never taken (Commandeer, below, is parked).

**More control, same calls.** `hold:<seconds>` (stand guard anywhere for one
call a heartbeat), slot targets (`follow:m2`, `reach:m2`, `give:m2:<item>`
for teammates; `follow:n1` for a stranger whose nametag it aimed at within
8 m), and the standing plan in `look` and `status`. A target is a slot the
request offered, never a name; a give is only ever a live answer, never a
plan step; chat never moves an item.

**The agent arena.** A sealed shard flag (no payout file, no prize, no mint)
and a Harbor environment around it: a fixed seed and commit, each agent in
its own sandbox with the MCP door, teams assigned. Two modes: a best-of-N
series between crews of any mix of harnesses and models, and endless
survival against the house village (GatesBench with teamwork). The first
published run is one model family's crews against another's. It wants a
low-level scripting mode beside the goal menu, with wall 1 kept.

**Recording hosted bodies.** The runner passes `--record`, and the wipe
packer (`ops/research/pack_wipe.py`) packs `gates-decision-1` rows beside the
standings, with names and wallets redacted from `reason` and `reason` nulled
on a turn asked because of a team-channel line. Students then learn to fill
templates (`go:`, `walk:`, `hold:`, slot targets), and a bench result names
the student it played. This makes the public shard's dataset what nobody
else has: agents and people competing for real money.

**Commandeer: parked.** Taking a downed body for a few minutes breaks one
wallet one body, wall 1 (a drive inside `sim-core` reads `World`) and hand
revive's input. Not built unless the operator answers below.

### Questions for the operator

1. **Turncoat on the public shard.** Once the test shard plays well, does
   the house village on the public shard run it, or only shards that post it?
2. **The per-body price.** Free copies stop on 1 December. Do you arm the
   ticket door, and when (it needs `require_auth`, which ends guest play)?
   Does a copy cost more than one wallet can extract in a wipe?
3. **Commandeer.** Its own opt-in shard, or not at all?
4. **Agent feeds.** Keep declared agents' spectator feeds live on the public
   shard, or delay them now that a crew's base is worth scouting? (The Watch
   feed is already 60 s behind.)
5. **One harness, many bodies.** One model context would see every crew
   body's view at once, which no single player does. Build the crew door, or
   keep one harness per body?
6. **An owner's plan.** Does an owner's signed standing plan outrank the
   house brain's own, or only fill the turns while the owner is away?
7. **People's paths in the dataset.** Today people appear only as outcomes.
   Publishing their trajectories needs a line in the terms, a per-wipe id, no
   wallet, name or chat, and the same redaction test.

## What this does not decide

The economy side — what an agent pays to enter, what it earns, whether an
agent's coins are its owner's — is `ALPHA.md` and the Elo Pros side, not
this doc. Nothing about it belongs in `sim-core`.
