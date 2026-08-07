# reference/SAVES.md — how the reference game remembers a player

**Research, not law.** This owns nothing. It is what the reference game
actually does about persistence and identity, in enough detail to size ours
against it, plus §9 — the only section that says anything about *our* code.

**Source posture, and it is the clean one** (`AUDIO.md`'s, not `SPAWN.md`'s):
public devblogs, the convar list the game prints to any player who types
`find server`, server-host documentation, Steamworks' own public docs, and
`rust-systems.txt` — this repo's existing MIT rip of Oxide's patch project.
Nothing decompiled, nothing extracted, nothing ships. Full provenance in §10.

Written 2026-08-07, the day after `crates/server/src/store.rs` landed, because
the question "how does the reference game do this" had never been asked and the
answer turned out to contradict the shape we had just built.

---

## 1 · The finding that reframes everything: there is no player save file

**Your body is furniture.** When a player disconnects, the reference game does
not remove them from the world and write them to a side file. The player object
*stays where it is*, stoops down, and sleeps.

Devblog 7 — which is the same post that introduced saving at all, and that is
not a coincidence:

> "When a player leaves the server their body now remains."

The post is explicit that this replaced an earlier design that did what we do:

> "This is a bit nicer than the old version of sleepers, where the player
> object gets removed and is replaced with a fake version of the player."

And the consequence they call out first is that the body is still *live state*,
not a serialized snapshot of one:

> "This means that things like metabolism continue to run."

So in the reference game **player persistence and world persistence are the
same file and the same mechanism.** You are saved because you are an entity,
exactly as a wall is. There is no `PlayerSave`, no key→record index, and no
restore-on-join, because there is nothing to restore: your body was never gone.

Waking is a deliberate act, not automatic — the same devblog:

> "When your player is asleep and you join the server, the player won't wake up
> immediately but will be asleep until you press fire."

### Why this is a design pillar and not an implementation detail

Offline raiding — the thing the genre is *about* — rides on this. A sleeping
body is in the world, so it can be found, killed, and (later) looted. A
disconnect is not a safe-word. Our model makes it one, and that is the
divergence §9 is about.

Note the order they shipped it in, because it is a smaller first slice than it
looks: bodies stayed **before** they were lootable. Devblog 7 lists looting and
dragging as future wants, not as part of the slice.

## 2 · The unit of persistence is the networked entity, and the save format IS
the network serialization

From `rust-systems.txt`, `BaseNetworkable` — six hooks, and two of them are the
entire persistence story sitting next to the snapshot path:

```
BaseNetworkable  [6]
  IOnEntitySaved     ToStream(Stream, BaseNetworkable/SaveInfo)
  OnEntityLoaded     Load(BaseNetworkable/LoadInfo)
  OnEntitySnapshot   SendAsSnapshot(Connection, NetWrite, Boolean)
  OnEntitySnapshot   SendAsSnapshot(Connection, Boolean)
  OnEntityKill       Kill(BaseNetworkable/DestroyMode, Boolean)
  OnEntitySpawned    Spawn()
```

`ToStream` writes an entity to the save; `SendAsSnapshot` writes it to a
connection. **Both live on the same base class**, so in that codebase every
persistable thing is a networkable thing and one class owns both paths.

Stated carefully, because the hook table is the whole evidence here: it proves
the *class* is shared and the two methods sit side by side. Whether they share
one underlying writer is an inference from that, not something these signatures
say, and this document does not read their source to check. The load-bearing
half needs no inference — **the persistence boundary is the networked entity**,
which is what makes a sleeping player and a wall the same problem.

## 3 · The whole save system is one class with three hooks

```
SaveRestore  [3]
  OnNewSave      Load(String, Boolean)
  OnSaveLoad     Load(String, Boolean)
  OnServerSave   DoAutomatedSave(Boolean)
```

`DoAutomatedSave` on a timer, `Load` at boot, and a third hook to distinguish
"this is a fresh world" from "this is a loaded one". A full-world entity walk,
and no more machinery than that.

Sleepers are three hooks on `BasePlayer` and nothing to do with the save path,
because the save path already covers them:

```
  OnPlayerSleep      StartSleeping()
  OnPlayerSleepEnd   EndSleeping()
  OnPlayerSleepEnded EndSleeping()
```

## 4 · The save is a stop-the-world stall, and a decade did not fix it

The convar is `server.saveinterval`, in seconds, **default 600**. Every
server-host guide that discusses performance says the same two things: saving
is heavy, and the fix is to do it less often. In the hosts' own words, on a
high-population server the save "causes a massive lag spike that freezes
everyone in place", and the advice is to keep the interval in the 300–600 s
band as "a balance between safety and smoothness".

Read that trade honestly: **the only knob is how much progress a crash costs
against how often everyone freezes.** There is no third option offered
anywhere, by anyone, for a game that has shipped this system since 2013.

What Facepunch *did* optimise is the other end — load, not save. Devblog 96
put building stability into the save so a restart could skip recomputing it
("much faster server restarts"), and a later pass freed the world
serialization cache after spawning. Both are boot-time wins. The save-time
freeze is what the community still routes around with a convar.

## 5 · Two files, split by what must outlive a wipe

Inside `server/<identity>/`:

| file | holds | survives a wipe? |
|---|---|---|
| `proceduralmap.<size>.<seed>.<n>.sav` | the world: every entity, **including sleeping bodies** | no — deleting it *is* the wipe |
| `UserPersistence.db` | per-player data, blueprints being the part that matters | **yes, deliberately** — a wipe keeps it |
| `Storage.db` | sign images | operator's choice |

So they *also* separate "the things that must outlive a wipe" into a
player-keyed database, distinct from the world blob. A blueprint wipe and a map
wipe are different operator acts on different files, which is exactly how the
hosts' documentation describes them.

## 6 · Corruption is routine, and the answer is numbered rotations

Backups sit next to the live file as `....sav.1`, `....sav.2`, **higher number
= older**. The count is a convar, `server.saveBackupCount`, documented with
**2 as both the default and the minimum**.

Recovery is manual and the guides spell out the loop: copy `.sav.1` over the
exact original `.sav` name and boot. And — this is the part worth copying the
*reasoning* from — they warn that `.sav.1` may itself be corrupt, because the
corruption may have started several save cycles ago, so you walk back to
`.sav.2`. Boot-time symptoms are named too: `Error loading save` followed by a
NullReferenceException, or a `Couldn't load ... - file doesn't exist`.

The lesson is not "keep a backup". It is that **a whole-file save makes
corruption an all-or-nothing event for every player at once**, which is why a
rotation depth of one is not enough to be worth having.

## 7 · Identity: a session ticket the server resolves with the issuer

This is the shape `crates/server/src/auth.rs` already claims to copy, and here
it is concretely, from Steamworks' public documentation:

1. The client asks Steam for a **session ticket** and waits for the callback.
2. It sends the ticket to the server.
3. The server makes an **HTTPS request to `api.steampowered.com`**, calling
   `ISteamUserAuth/AuthenticateUserTicket` with the ticket hex-encoded.
4. On success the call **returns the user's 64-bit SteamID**.

The server will not let a client finish connecting without that round trip
completing. A ticket can be cancelled or reused, and the server hears about it
through a `ValidateAuthTicketResponse` callback — which is where the
`AuthTicketCanceled` kick that fills the mod forums comes from.

**The key is 8 bytes.** And ownership on a persisted entity is that same
`UInt64`, visible in the hook signatures without reading a line of their code:

```
SleepingBag  [4]
  OnSleepingBagValidCheck   ValidForPlayer(UInt64, Boolean)
  OnSleepingBagDestroy      DestroyBag(UInt64, NetworkableId)
```

## 8 · What the reference game does NOT do

Worth stating, because absences are evidence too:

- **No WAL, no command log, no replay.** Persistence is periodic full-world
  snapshots. A crash loses up to `saveinterval`, and that is the whole
  guarantee. Nothing reconstructs the interval between saves.
- **No per-record integrity.** The file is trusted or it is not; there is no
  mechanism to lose one player and keep the rest.
- **No incremental or background save.** See §4 — the stall is the design.
- **No determinism requirement on a load.** They are not replaying anything, so
  a loaded world only has to be *legal*, not *identical*.

## 9 · What it means for us

**Operator, 2026-08-07: adopt this model** (*"i want what rust has pretty much
so we cant go wrong, ill deviate after we get established"*). So this section
is a plan, not a menu. `DECISIONS.md` carries the call; `NOW.md` §0y carries
the sequence.

### 9.1 The body must stay in the world (the divergence to close)

Today `Command::Leave` deactivates the player and `store.rs` writes a record.
That is precisely the "old version of sleepers" §1 quotes them replacing. It
costs us the genre's central consequence: **a disconnect currently makes you
safe.** Closing it is a sim change plus one wire bit, and Devblog 7's own
shipping order says lootable can come after standing.

### 9.2 Our store does not become wrong — it changes job

Once bodies stay, the record is no longer how a player comes back within a
shard's life; it is how they come back when the world does **not** have them:
a fresh shard, a wiped world, a world save that refused. Keep it. It also
already is the §5 split — keyed by an opaque identity, separate from the world
— which is the file they keep across a wipe.

### 9.3 Where we are already ahead, and it is measured

- **The save stall.** §4 is a decade-old unfixed freeze whose only mitigation
  is a convar. Our sweep is one player per tick, bounded, skip-if-unchanged,
  on a thread that is not the sim. Do not trade that away for a full-world
  snapshot when §9.4 lands — the world stores want the same treatment.
- **Per-record integrity.** §8: they cannot lose one player and keep the rest.
  We can, and do (xxh3 per record, refused at load, counted).
- **Determinism.** §8 again: they have no replay to satisfy. We do (wall 5),
  which is why a restore rides `Command::JoinAs` instead of being read out of
  a file by the sim. A world load has to answer the same question, and §8 says
  the reference game is no help here — this part is ours to invent.

### 9.4 Where to copy them exactly

- **Numbered backup rotation, minimum depth 2** (§6), for the reason §6 gives
  rather than the obvious one: corruption may predate the newest backup.
- **The wipe split** (§5): world blob and player-keyed store are different
  files, wiped by different operator acts.
- **The validator** (§7): one HTTPS call to the issuer, returning one key.
  `PLAYER_KEY_MAX_BYTES = 48` is 6× a SteamID64 and needs no change.

### 9.5 What NOT to copy

- **`saveinterval` as the only safety knob.** It is the symptom of §4, not a
  feature. Ours is a sweep, and the cadence is derived from `MAX_PLAYERS`.
- **A trusted file.** Their loader takes the save's word for it. Ours must not:
  a save file is the one non-command path into `World`, so it validates like a
  wire decode (`PlayerSave::read_le`), and that stays true for world state.
- **Full-world serialization on the sim thread.** §4.

## 10 · Provenance

Facts only. Nothing here is copied into the game, the client, or the build.

- **Facepunch devblogs** — `rust.facepunch.com/news/friday-devblog-7`
  (sleepers and the first save system; every §1 quote), `devblog-96` and
  `devblog-164` (stability serialization, load-time wins, the serialization
  cache). Public marketing/technical posts, quoted as commentary.
- **Steamworks documentation** — `partner.steamgames.com/doc/features/auth`
  and `/doc/api/isteamuser`. Public developer documentation; §7's flow.
- **The convar list** — `server.saveinterval`, `server.saveBackupCount`,
  `server.save`. Printed by the game to any player who asks, and restated in
  every server host's knowledge base.
- **Server-host documentation** — Shockbyte, Host Havoc, Survival Servers,
  GameServerKings, Tempest, and the uMod/Oxide community threads: §4's stall
  reports, §5's file table, §6's rotation and recovery loop. Operator-facing
  documentation about running a server, not about its source.
- **`reference/rust-systems.txt`** — this repo's existing rip of
  `OxideMod/Oxide.Rust`'s `resources/Rust.opj`, **MIT**, © 2013–2020 Oxide
  Team and Contributors. Source of every hook block quoted in §2, §3 and §7:
  hook names, patched class names, method signatures. See `reference/README.md`
  for the regeneration command and the licence posture.
- **`CarbonCommunity/Carbon.Hooks.*`** — GPL-3.0, deliberately **not**
  consulted for this document, the same posture `README.md` records.
