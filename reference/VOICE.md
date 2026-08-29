# reference/VOICE.md — how the reference game does proximity voice chat

Research, not law. It owns nothing in `crates/`. It exists because two live
claims in this tree point at a question nobody had opened: `ALPHA.md` §1 cuts
voice as *"a rabbit hole with its own transport"*, and `reference/AUDIO.md`
§9.9 lists voice chat under *"what the reference does that we should NOT copy
yet"*. **The first of those is now wrong on its own terms** — the transport
is the rabbit hole the reference dug and then climbed out of, and we are
already standing where they ended up. §9 is the argument; §2 is the fact it
rests on.

`AUDIO.md` is the sibling doc and owns the mixer this would feed. Nothing
here is decompiled and nothing here ships.

## 0 · Provenance — read this first

Four sources, unequal, ranked. Read the rank before the claim.

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so what it proves is **shape**: which class handles what.
   Read directly out of a file in this repo, no summary in the way. §1 is
   this source and nothing else, which is why it leads.
2. **The developer's own devblog**, by number. Devblog 189 is the load-
   bearing one and §2 is built on it.
3. **Steam's own API documentation** and Facepunch's wrapper wiki, for the
   codec facts in §3. Primary *documents* — but see the caveat below; they
   arrived as summaries.
4. **Community threads and guides** for the radius in §4. Weakest tier by a
   wide margin, player-maintained and undated, and here they **disagree with
   each other**. Treated as "there is a radius and it is tens of metres,"
   never as a number to take.

**The honesty note, and it earned its keep this time.** Every page fetch
attempted from this container was refused by the egress proxy —
`rust.facepunch.com`, `wiki.facepunch.com`, `partner.steamgames.com`,
`support.facepunchstudios.com`. Tiers 2–4 therefore come through **search-
result summaries of those pages, not the pages themselves**, which is
`DOORS.md` §0's caveat in full. Two consequences worth stating:

- **A summary reported a fact that is a decade stale, and tier 1 caught it.**
  One search summary said Rust voice "is P2P." That was true until 2017 and
  is false now; `ServerMgr.OnPlayerVoice(Message)` in tier 1 settles it in
  the other direction (§1). A summary can drop a qualifier — it can also
  drop a *date*, which is worse, because the result reads as present tense.
- **Reachability is a property of the container, exactly as `SOURCES.md` §0
  says.** `reference/DURABILITY.md` fetched `wiki.facepunch.com` pages
  *whole* on 2026-08-15; the same host is blocked here on the same day. Do
  not read either measurement as a standing fact about the host. Probe.

## 1 · Voice is a server message — the shape, off the hook table

`reference/rust-systems.txt` places exactly one voice hook, and **where** it
places it is the whole finding:

```
  ServerMgr  [8]
    IOnPlayerBanned        OnValidateAuthTicketResponse(UInt64,UInt64,AuthResponse)
    OnClientAuth           OnGiveUserInformation(Message)
    OnClientDisconnect     ReadDisconnectReason(Message)
    OnPlayerSetInfo        ClientReady(Message)
    OnPlayerSpawn          SpawnNewPlayer(Connection)
    OnPlayerVoice          OnPlayerVoice(Message)
```

`OnPlayerVoice` hangs off **`ServerMgr`**, not off `BasePlayer`, and its
argument is a `Message` — the same class and the same argument shape as
auth, ready, spawn and disconnect. Three things follow, and none of them
needs a devblog:

1. **Voice arrives at the server as a top-level network message.** It is not
   a peer stream the server is unaware of, and it is not an entity RPC. It
   is handled beside the connection lifecycle itself.
2. **The server is therefore the router.** A server that receives every voice
   packet is the thing that decides who gets a copy — there is no other
   candidate in the path.
3. **A mod can see, rewrite or refuse a voice packet before it is relayed.**
   That is what a hook *is*, and it is why the community's "voice as radio",
   "admin broadcast" and "voice range" plugins are all possible. It also
   means the reference treats voice routing as **policy**, sitting at the
   same layer as bans.

What tier 1 does **not** prove: the radius, the codec, the packet rate, or
whether attenuation is client-side. Those are §§3–4 and they are weaker.

## 2 · The history — P2P, an IP leak, and the move onto the server

This is the finding that matters most, and it is the reference publishing a
postmortem on itself.

**Voice originally went over Steam's P2P network** — client to client
directly, bypassing the game server entirely. It is the obvious design: the
server carries none of the bandwidth, latency is one hop instead of two, and
the voice API and the transport ship in the same SDK.

**It became an attack surface.** A P2P voice session is a direct socket
between two machines, so a player could read the other player's **IP address
off it** — and players did, then DDoSed each other off the server. The
attack needs no cheat and no exploit; it is a property of the topology. Any
player you could hear was a player whose home connection you could find.

**Facepunch moved voice onto the server** (Devblog 189) and stated the second
consequence themselves: routing through the server *"opens up the way for
features like loudspeaker systems, phones, and tape recorders."* Four years
later that is exactly what shipped (§5).

Read the arc as one sentence: **they paid for the cheap transport, removed
it under duress, and the removal turned out to be the feature platform.**

The cost they took on is real and is not hidden — the server now carries
every talker's uplink and fans it out to every listener in range. §6 is what
that cost looks like when it goes wrong.

## 3 · The codec, and what Steam does and does not give you

Tier 3, and the distinction in the last line is the useful part.

- Steam's voice API is capture + codec, not transport: `GetVoice` pulls
  compressed audio from the microphone, `DecompressVoice` turns it back into
  **raw single-channel 16-bit PCM**. The decoder accepts any sample rate from
  **11 025 to 48 000 Hz**.
- The compressed payload is **Opus** — described by people who have reversed
  it as a thin wrapper around Opus packets, decoded as mono with the rate
  carried in the packet itself. (Steam's older voice codec was not Opus; a
  community feature request asking for Opus predates the change. Treat "Opus"
  as true of the modern API and undated as to when it became so.)
- **Steam explicitly does not send the data.** Its own documentation says the
  voice API provides no means of transmitting voice — you carry the bytes on
  whatever networking you already have.

That last bullet is the one to keep. The SDK a game reaches for to get voice
"for free" hands back **a codec and a microphone, and no transport at all**.
The transport was always the game's problem; Rust's mistake in §2 was
answering it with the *other* thing Steam ships (P2P sockets) rather than
with the server it already ran.

## 4 · Proximity — the radius, and why the mechanic is the cost

**The radius is tens of metres and the sources disagree on the number.**
Community threads land around 60 units; guides say only "a few dozen metres"
and "voices fade with distance until inaudible." Nothing at tier 1 or 2
carries a figure. **Do not take a number from this section** — §9.4 says what
to do instead.

What the sources agree on and what is worth more than the number:

- **Falloff, not a cliff.** Nearby is loud and clear, further is quieter,
  past the range it is nothing. Same shape as any positional audio.
- **The range is small relative to engagement distance.** You can be shot
  from well outside the distance at which you can be *talked to*. That is
  the mechanic: closing to talking range is closing to killing range, and
  both parties know it.
- **Talking is positional disclosure.** Your voice tells anyone in range
  that you exist, roughly where you are, and how many of you there are. In a
  game about ambush, speech is a tactical cost paid up front, and the whole
  "hello? friendly!" folk-ritual of the genre is players negotiating that
  cost in real time.
- **There is no whisper or yell tier.** A standing feature request asks for
  toggleable whisper/yell volume, which is evidence it does not exist: one
  radius, no player control over it.

The design reading: proximity voice is not a chat feature that happens to be
spatial. It is a **disclosure mechanic**, and the radius is the price. That
is why it belongs to the same family as nametags-only-within-8 m
(`ALPHA.md` §1) and why it is worth more in this genre than in most.

## 5 · Voice as a prop — what server routing unlocked

Devblog 189's promise, delivered as the **Voice Props DLC** (2021-07-01).
Every item on the list is *voice re-routed by an object*, which is only
expressible when the server already holds the packets:

| prop | what it does to the routing |
|---|---|
| **microphone stand** | rebroadcasts your voice from a fixed point, short range; two pitch modes (squeaky, deep) |
| **megaphone** | projects your voice far past the normal radius |
| **boombox** | plays back recorded audio — and internet radio streams |
| **cassette recorder + cassettes** | records a voice to an *item*, which can be carried, dropped, looted and played elsewhere |
| **mobile phone / telephone** | routes voice between two arbitrary points on the map by number, plus voicemail |

Four distinct routing rules — reposition, widen, store-and-replay, and
address-by-identifier — and none of them is a codec change. They are all
**edits to "who gets a copy"**, which is a decision the server was already
making. A P2P design can express none of them without inventing a server.

Note also what it says about monetization: the reference sold voice *props*,
never voice *reach*. Nobody bought a bigger radius. (`BUSINESS.md`'s rail
would refuse that anyway — it is an advantage over another player.)

## 6 · The failure modes they have published

Tier 3–4, but they are the developer's own support pages and community
consensus, and all three are about **cost**, not about audio quality.

1. **Voice causes client hitching.** Facepunch maintains a support article
   titled *"Freezing or Hitching when using in-game voice"* — a shipped,
   documented, acknowledged stall on the *speaking* client. The mechanism is
   not in reach from here, but the existence of the article is the fact: the
   capture/encode path is on a budget it can blow.
2. **Packet flooding with concurrent talkers.** Community reports put the
   knee around **8–10 simultaneous transmitters**, at which point the
   server's fan-out becomes the problem. Fan-out is quadratic in the worst
   case — every talker × every listener in range — and a crowd is exactly
   when everyone talks.
3. **Muting and censorship is a permanent support surface.** They maintain a
   documented mute/censorship policy and per-player muting. Open voice in a
   sold product is a moderation obligation, not a feature you ship and walk
   away from.

Together: voice's cost is **client CPU on the talker, server fan-out on the
crowd, and human moderation forever**. None of those three is the codec.

## 7 · The verb inventory, complete

What a player can actually do, as verbs, so §9 can be checked against it:

- **talk** (push-to-talk, held key) · **hear** (automatic, positional,
  range-limited) · **mute a player** (persistent, client-side list) ·
  **test my own mic** (a loopback convar) · **see who is transmitting** (a
  convar listing current talkers) · **adjust voice volume** (a mixer bus of
  its own).

And, via §5's props: **rebroadcast**, **amplify**, **record**, **play back**,
**call**, **leave a message**.

## 8 · Sources

Tier 1 is in this tree. Tiers 2–4 are search summaries; see §0.

- `reference/rust-systems.txt` (from OxideMod/Oxide.Rust, MIT) — the
  `ServerMgr` hook block quoted in §1.
- Devblog 189, `rust.facepunch.com/news/devblog-189` — the P2P → server move
  and the stated reason. **Fetch blocked; summary only.**
- *Wounding Update & Voice Props DLC*, `rust.facepunch.com/news/wounding-and-voice-props`,
  and the store page — §5's item list. **Blocked; summary only.**
- Steam Voice documentation, `partner.steamgames.com/doc/features/voice`, and
  `wiki.facepunch.com/steamworks/SteamUser.DecompressVoice` — §3. **Blocked;
  summary only.**
- *"Freezing or Hitching when using in-game voice"* and *"Muting and
  Censorship"*, `support.facepunchstudios.com` — §6. **Blocked; titles and
  summaries only.**
- Community threads (umod.org, oxidemod.org, codefling, steamcommunity,
  rust.nolt.io) — §4's radius, §6's flooding knee, §7's whisper/yell request.
  Tier 4.

## 9 · What it means for us

The five that matter. §9.1 is the one that would be unfixable later.

### 9.1 · Client-side attenuation is a wallhack — the cull is server-side or it is nothing

If the server broadcasts voice widely and the **client** decides what is
audible, then a modified client hears the whole island. It costs an attacker
nothing: they do not defeat a check, they simply skip one. Proximity would be
a *rendering* convention rather than a rule, and in a game whose entire
premise is that you do not know who is nearby, that is the single most
valuable cheat available.

So: **the server sends a voice frame only to players inside the radius**, and
the client's falloff is cosmetic on top of a set that is already correct. This
is not a new mechanism for us — it is exactly what `CHAT_LOCAL_CM`'s doc
comment already commits local chat to, and exactly what AOI does. Getting it
wrong is a wire-shaped mistake, which makes it a wall-6 problem, which makes
it expensive after the fact. **Decide it now even if we build it late.**

### 9.2 · Proximity voice is AOI with a smaller radius — we already own the mechanism

The arithmetic, in our own constants:

| | |
|---|---|
| `AOI_ENTER_CM` | 17 600 (176 m) |
| a voice radius in the reference's band (§4) | ~6 000 cm (60 m) |
| `CHAT_LOCAL_CM` | 2 000 (20 m) |

A voice radius anywhere in §4's band sits **well inside `AOI_ENTER_CM`**, so
every player who could hear you is already an entity in your interest set —
already positioned, already scanned, already in hand on the server this tick.
The routing decision is therefore **one distance compare against a set the
AOI scan already produced**, not a new spatial query and not a new subsystem.
`CHAT_LOCAL_CM`'s comment states this property for chat in as many words;
voice inherits it unchanged.

This is what retires `ALPHA.md`'s *"rabbit hole with its own transport."* The
rabbit hole is real and §2 is a decade of the reference falling into it — but
it is only reachable from a P2P start. **We have no P2P anything.** We have
one authenticated QUIC session per player carrying unreliable datagrams
(`NETCODE.md` §2), which is precisely the lane voice wants and precisely
where Devblog 189 forced the reference to end up.

### 9.3 · Voice touches none of the determinism walls, and that is unusual

Worth stating plainly because it changes the risk profile. Audio bytes are
**relayed, never simulated**:

- **Wall 1 (sim-core is pure)** — untouched. Voice never enters `sim-core`.
  The sim already publishes positions; the *server* crate reads them and
  routes. Nothing about a codec, a buffer or a thread crosses that boundary.
- **Walls 2 and 3 (zero-alloc, no locks/strings in the tick)** — untouched,
  for the same reason. The relay is not the sim thread.
- **Wall 5 (determinism)** — untouched. Voice is not in `state_hash`, not in
  the WAL, and a replay is unaffected by who said what. Two shards replaying
  one WAL agree bit-for-bit whether or not anybody spoke.
- **Wall 7 (content never touches code)** — untouched; there is no content
  row here. The radius is a **knob**, not a content number, and belongs in
  `limits.rs` beside `CHAT_LOCAL_CM` with `DECISIONS.md` §open carrying it.
- **Wall 6 (the wire)** — **this one applies.** A voice datagram is a new
  packet type: `PROTO_VER` bump and goldens regenerated in the same commit.
- **Wall 4 (bounded everything)** — **this one applies, and it is where the
  design actually lives.** §6.2's flooding knee is wall 4 stated by someone
  who did not have wall 4. Every one of these needs a cap and a stated
  overflow policy: frames per talker per second, concurrent talkers relayed
  per listener, bytes per frame, and the per-tick fan-out work item count.

So the honest summary is: **voice is a wire slice and a bounding slice, and
nothing else in this repo's law has an opinion about it.** That is a much
smaller blast radius than the alpha cut assumed.

### 9.4 · The radius is ours to pick, and §4's number may not be taken

`BALANCE.md` §6 makes "take theirs" the default and requires a mechanism case
only to differ. **It does not license taking a number this weak.** §4's ~60 m
is tier 4, undated, and self-contradictory across sources; §6.3's ladder for a
split source cannot break a tie between "60 units" and "a few dozen metres,"
because those are not two measurements, they are one vague claim twice.

What we can take is the *relation*, which every source agrees on and which is
the design content anyway: **voice range ≫ local-chat range, and voice range
≪ engagement range.** Ours would then be an opening value in `CONTENT.md`'s
sense — spoken into `DECISIONS.md` §open, moved by ears and playtest, exactly
as every number in audio v0 already is. It is a `RIPLIST.md` row that is
**blocked on research nobody has done**, and it should be filed as one rather
than guessed into `limits.rs`.

### 9.5 · Build order, and the recommendation

**Not now.** `NOW.md` leads with five items from the operator's 2026-08-15
playtest, and `ALPHA.md` §5 cuts voice from alpha. Neither of those is
overturned by this document, and a research doc is not a licence to jump the
queue. What this document changes is the *reason* for the cut: it stands on
scope now, not on "a rabbit hole with its own transport," and that sentence
in `ALPHA.md` should be corrected because it is a claim about our
architecture that is no longer true.

**When it is built, the order is forced by which half carries the risk, and
it is not the half people expect.**

1. **Routing first, and it needs no audio at all.** The wire type, the
   server-side radius cull off the AOI set, the caps and their overflow
   policies, and a gate that proves a listener outside the radius receives
   **zero bytes** — not attenuated bytes. This slice is testable end to end
   with a synthetic payload and a bot swarm (`bin/bots`), on a box with no
   microphone and no sound card, which is the box CI runs on. It is also
   where §9.1 lives, so it is the half that must not be got wrong.
2. **Codec second, and it is the only genuinely risky part.** Opus is a C
   library, and this tree has already paid for a C++ dependency's threading
   model on the Windows mingw cross-build (`CLAUDE.md` traps, 2026-08-13:
   `basis_universal_sys` and `__mingwthr_key_dtor`). Adding `libopus` reopens
   exactly that seam on exactly that target. Pure-Rust Opus implementations
   now exist but are young; **evaluate them against the cross-build, not
   against a decode benchmark**, because the cross-build is where the cost
   showed up last time.
3. **Capture third.** `cpal` is already in the tree transitively (rodio, via
   `bevy_audio`), and input is `build_input_stream`. Two constraints from
   this repo's own record: it must be off the frame path (§6.1 is the
   reference shipping a documented hitch on the *talking* client), and **the
   client must run with no microphone and no sound card**, the same rule the
   capture probe already proves for output.
4. **Mute list with the first playable voice, not after.** §6.3 is a
   permanent obligation and it is cheap while the surface is small.

**Deliberately not ours, ever: P2P.** §2 is a complete argument and the
reference paid for it in players' home connections.

**Deliberately deferred: the props.** §5 is the reward for having done §9.1
correctly, and it is a routing edit rather than a new system — which is the
point. It costs nothing to leave undone and it stays cheap, *provided* the
relay is a routing decision the server owns rather than a broadcast the
client filters. That is §9.1 again, which is the one thing worth deciding
today.
