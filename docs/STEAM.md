# STEAM.md — Gates 2 on Steam

The plan for a Steam edition (working name **Gates 2**). It's the same game
with **no economy**: JUNK is a material and a leaderboard number, like scrap in
Rust. **Gates and Gates 2 are two games that are clients of the same
shards.** Gates 2 plays the normal islands, LAN and solo. **It never plays
our premium islands** (operator, 2026-10-24): it sees them in the server
list and can't join, and how to get in is the player's to find out.
The point is the funnel: people meet the game on Steam, and **elopros.com does
the pitching**, never the Steam build.

Planned 2026-10-23. Nothing here is built yet. A line that disagrees with the
code is wrong.

## 0 · The rules we design to

Valve's own pages, read 2026-10-23:

- Onboarding, *what you shouldn't publish*: **"Applications built on
  blockchain technology that issue or allow exchange of cryptocurrencies or
  NFTs."** Gates (wallet sign-in, `EloItem` unlocks, the ticket, JUNK payouts)
  is inside that line. Gates 2 is not.
- Microtransactions: **"you'll need to use the microtransaction API so Steam
  customers can only make purchases from the Steam Wallet."** Gates 2 sells
  nothing in game, so this never comes up.
- Store page: **"You cannot use your store page descriptions or graphical
  assets to promote or link to other websites. This includes via embedded
  images, QR codes and even implied URLs via text."** The site goes in the
  website field (Basic Info), nowhere else.
- Keys: **"don't give Steam customers a worse deal than Steam Key
  purchasers."** It binds Gates 2 keys, not the Gates NFT copy, which is a
  different product.
- Account linking is encouraged. Valve's one fraud tip: **"Don't allow users
  to change the linked accounts."**
- The app fee is $100 a title, there's a **21-day wait** between paying it
  and release, a **Coming Soon page must be public for at least two weeks**,
  and review takes 1–5 days.

When Valve pulled Age of Rust (2021), the developer reported the reason as
items with **real-world value**. That is the test this plan applies: nothing
a Steam session earns may become money or an NFT, by any road.

## 1 · The shape

Each shard has an **economy** switch: on means skins, crypto extraction,
prizes and a door that checks the wallet. The shard decides admission; a
client only lists and dials.

| shard | economy | Gates | Gates 2 |
|---|---|---|---|
| normal island (we run it, like a community server) | off: no skins, no crypto extraction | joins | joins |
| premium island | on: the ELO door (§4.8), PvP for crypto | joins | **listed, never joinable** |
| LAN / solo (the player's machine) | off | joins | joins |

**The one line, enforced by the shard: an economy shard refuses every Steam
sign-in.** Gates 2 greys premium islands out and won't dial one, but the
shard is what holds the line, so a modified client gets nowhere.

The hook is the list. A Gates 2 player sees premium islands they can't
enter, looks it up, and finds Gates.

What crosses between the games: **identity** (name, face, friends, boards,
playtime). What never crosses: **inventory**. Saves are per world file, so
one island's character is not another's, and Gates wipes weekly anyway.

## 2 · Phases

Each phase is one PR into `main` unless it says otherwise. None of them is a
WIPE.

### P1 · The plain shard *(server, SHARD)*

One switch in `shard.toml`: `economy = false`.

- **Boot refuses a contradiction.** A plain shard with any money setting set
  fails to start, naming the key: `extract_cap`, `prize_ticker` (and the
  `prize_*` places), the ticket door (`entitle`), item reads (`skins`), the
  Outfitter mint queue. `config.rs` already parses all of them; this is one
  check after the parse. `faces` stays: it's identity.
- **THE OUTFITTER on a plain shard** can't mint (`mints.rs`: today the token
  is the only proof of owning an unlock). Default: the unlock is
  recorded in the save and lasts until the next blueprint wipe, like a
  blueprint (decision §4.3). The alpha gift (`ARC_GIFT_*`) is off.
- **The mint job can't reach it.** The shard writes `economy: false` into
  `<world>.online.json`, and `ops/mint_pending.py` refuses that world.
  Otherwise a linked Steam player listed under a 0x key on a plain island
  would be sent a free NFT copy for playing on Steam.
- Standings and boards run unchanged: no prize, just the board.

### P2 · The premium door, and Steam stays out *(protocol + server, WIRE)*

- **Economy shards refuse Steam.** A Steam proof on a shard with the
  economy on is refused with a new code (`REFUSE_EDITION`, the 13th of 256
  `Refuse.code` values). Its sentence names no store, coin or link: "this
  server can't be joined from this game". No setting turns it off.
- **The ELO door** (Gates only). `entitle.rs` already asks elo what a wallet
  holds at join and refuses on a definite zero, never on a failed read. The
  premium door is the same read with "holds at least X of the reserve" in
  place of "holds a ticket".
- **The list says what a server is.** The challenge frame and the server
  list carry the economy switch, so Gates 2 greys out a premium island
  before dialing. Bump `PROTO_VER`, run `gen_goldens`.

P2 and P4 share one wire bump if they land together.

### P3 · The Gates 2 client build *(client, CLIENT)*

A cargo feature, `economy`, default on (same pattern as `native`), so every
existing build is unchanged. Gates 2 builds with it off:

- **Out:** wallet sign-in and the launcher signer path
  (`net/handshake.rs`'s SIWE proof), price and mint cards in
  `ui/offer.rs` (free gifts stay), `ui/locker.rs`'s ITEM STORE and
  INVENTORY, the launcher-backed NEWS / ITEM STORE / WORKSHOP entries in
  `ui/hub.rs`, `deeplink.rs` (`elo://`), and every call in `elo.rs`.
- **The server list** stops coming from elopros.com and reads a neutral
  list (decision §4.5 for the host). Premium islands are listed greyed
  out, under their island names, with no link and no coin named. **Only the
  house's own economy islands are listed**, under names the house picks: a
  community economy shard could call itself anything ("CRYPTO PVP $$$"),
  and its name would then be inside the Steam build. No prize pots and no
  JUNK totals in the Gates 2 list.
- **One plain website link** in the main menu. No pitch screen, no prices,
  no coin.
- **A CI guard.** Build the Gates 2 binary and grep its strings for
  `elopros`, `wallet`, `NFT`, `EloItem`, `ticket`. A hit fails the job. The
  string `JUNK` is allowed: it's an item.
- `--features render` code is only compiled by the render clippy, so add the
  `--no-default-features --features render,native,steam` clippy to
  `ci/quick.sh` and `gates.sh fast`.

### P4 · Steam sign-in *(protocol + server + client, WIRE)*

The template is v142, Solana sign-in (commit `82774f5`): a tag byte on the
auth frame, a verify on the shard, and a link lookup at elo.

- **Client** (feature `steam`, the `steamworks` crate, MIT/Apache, plus
  Valve's redistributable `steam_api` library in the depot) calls
  `GetAuthTicketForWebApi("gates")` and sends the ticket as auth tag **2**.
- **Shard** (`server/src/auth.rs`) posts it to
  `ISteamUserAuth/AuthenticateUserTicket` with the publisher Web API key
  (`steam_api_key_file` in `shard.toml`, a secret; the key itself is an
  operator act) and gets the 64-bit SteamID back. The shard needs no Steam
  SDK.
- **Which save** (normal islands only; economy shards refuse first, P2):
  the shard asks elo for the Steam ID's link (P6). Linked:
  the player's 0x key, so it's the same name, face and friends. Unlinked:
  `steam:<id64>` (26 bytes, under `PLAYER_KEY_MAX_BYTES = 48`, as
  `reference/SAVES.md` already worked out). The lookup failing is
  `REFUSE_LINK`, as for Solana.
- An unlinked player's default name is their Steam persona name.

### P5 · Solo and LAN *(client + server, CLIENT)*

The shard is already its own binary, and the native client already pins a
certificate by hash (`--cert-hash`, `client_endpoint` in `client/src/lib.rs`).

- **Solo:** the Steam depot ships `shard` beside `gates`. PLAY SOLO
  starts it as a child process on `127.0.0.1` with a built-in plain config
  and a world file in the player's data folder, then dials it. Quitting the
  game stops it.
- **LAN:** the same, bound to `0.0.0.0`. The self-signed certificate
  (`server/net.rs`, today only `localhost` names) adds the host's LAN
  addresses. The shard broadcasts `{name, port, cert hash}` on UDP, and the
  menu lists LAN games and dials with the pin. Typing an IP plus a pasted
  hash is the fallback.
- **Check first:** the shard's CPU and memory on a player's machine
  (`--bin profile`), and what an empty island feels like solo (decision
  §4.7).

### P6 · The store's half *(scry-forge, OFF CHAIN)*

- **Sign in with Steam** on elopros.com (Steam's OpenID). Read the Steam
  Web API terms of use before building it; this plan hasn't.
- **The link.** Steam OpenID proves the Steam ID and the wallet signs an
  `elo link` text naming it. Extend `meter/walletlink.py`, with two
  differences from the Solana link:
  1. **Private by default.** Solana links are public (`GET /link/{wallet}`).
     A public Steam↔wallet pair would show every Steam profile's holdings.
     The shard reads the link over an authenticated call; the public route
     answers only for a pair the holder chose to show.
  2. **Changing it** follows Valve's tip: one Steam ID to one wallet,
     undo allowed, re-link to a different wallet held for a cooldown
     (decision §4.4).
- **The profile page** shows Gates 2 boards and playtime from the plain
  shards' standings. This is the hook. Here, and only here, the site pitches
  Gates: the same island and the same friends, plus skins you own and JUNK
  you can cash out.
- Write the copy under the store's own rules (its `CLAUDE.md`: light voice,
  the held names, no traction numbers).

### P7 · Packaging and release *(operator acts marked)*

- Windows has to work first. `NOW.md` §0win: nobody has run the Windows build
  on Windows. Most Steam players are on Windows.
- A Steam depot: `gates` + `shard` + `steam_api` (Windows and Linux),
  `steam_appid.txt` for dev runs. `ci/depot.py` is the elo depot's builder
  and the starting point.
- Check the Steam overlay draws over Bevy/wgpu (Vulkan and DX12).
- Store assets at Steam's capsule specs (the elo store already copies them).
  AI-made assets get disclosed in Valve's content survey. **Agents playing
  on the normal islands are live AI content** (they talk in chat): the
  survey asks for that too, with the guardrails described, or the Gates 2
  list leaves agent islands out.
- The Steam Web API terms (read 2026-10-24) allow Steam data only for an end
  user's personal use, cap calls at 100,000 a day, forbid implying any
  relationship with Valve, and let Valve revoke access at any time. A
  public Steam playtime board on elopros.com is a stretch of "personal
  use"; showing a player their own is not.
- **Operator:** the Steamworks account and the $100 fee, tax and bank, the
  Web API key, the Coming Soon page (14 days or more, inside the 21-day
  wait), and the release button.

## 3 · If Valve says no

Gates 2 is a complete game without Steam, so a refusal costs a channel, not
the product.

- **It still ships:** a direct download (GitHub release, the elo depot) and
  itch.io.
- **Epic takes Gates itself, with conditions** (its *Blockchain Technology
  Guidelines*, as quoted by search on 2026-10-23; the page didn't load
  directly, so re-read it before signing): a signed Blockchain Rider; the
  **base product free**, with no add-ons sold through Epic; we handle
  payments, support, fraud and refunds; **no links to crypto or NFT
  marketplaces on Epic's pages**, so elopros.com's trade terminal can't be
  linked; not sold in China or South Korea; a Blockchain/NFT tag, a
  disclosure FAQ, and no gambling, where real-world value includes coins
  and NFTs (keep the elo gacha out of it). An AO rating is accepted only if
  blockchain is its sole cause. Gates and Gates 2 would be **separate
  listings** there too. Open question: a free Epic download whose
  public island needs a ticket bought off Epic.
- **The story is true, so tell it straight.** Quote Valve's line verbatim and
  show that the Steam build has no chain in it (the P3 guard is the proof).
  If they refused anyway, that is the argument `scry-forge` `PITCH.md` §3b
  already makes, that the rail decides. Don't bait a refusal, and don't
  publish Valve's correspondence before checking the Steam Distribution
  Agreement.
- **The funnel works without Steam too:** plain islands, Sign in with Steam
  on the site, and cross-play don't depend on Valve listing anything.

## 4 · Calls for the operator

1. **The name on Steam.** "Gates 2" reads like a sequel to a game most
   Steam users never saw.
2. **Price.** Free, or paid? Paid means the key parity rule above applies
   to anything elopros.com sells for it.
3. **THE OUTFITTER on plain islands:** an unlock that lasts until the
   blueprint wipe (default), one that lasts forever in the save, or off.
4. **Link changes:** locked once made (Valve's tip), or undo with a cooldown
   (default).
5. **The plain server list's host.** A list fetched from an elopros.com URL
   puts the store's domain in the Steam build; a neutral game domain avoids
   it.
6. **Official plain islands:** how many, on which box, and whether the public
   island also runs a plain twin.
7. **Solo on an empty island:** as is (Rust has no solo either), or with
   bots (`--bin bots`). No hosted agents: they'd need someone's API key.
8. **The premium coin: ELO (operator, 2026-10-24).** The door asks elo for
   *the reserve* (`meter/reserve.py`), never a hardcoded ticker, and reads
   RH ELO in the player's EVM wallet. The Solana ELO opens nothing until it
   has its own `deployments.json` record. Gates only: Gates 2 never reaches
   the door (§1).
9. **The premium amount:** fixed in coin, or dollars at the pool's mark,
   re-read at join.
10. **NFTs:** keep the ticket and skin NFTs for Gates, or retire them for
    the coin door. They're live on chain, so retiring them touches the
    store.

**What Valve sees.** Gates 2 sells nothing, pays nothing, and can't enter a
crypto server. It lists some servers it can't join, under island names.
Players will say in Steam reviews what those are; that's the attention, and
the build still has nothing in it to object to.

## 5 · What must stay true

| guard | where |
|---|---|
| the Gates 2 binary names no store, wallet or NFT | the P3 strings check |
| a plain shard can't boot with a money setting | the P1 boot check |
| an economy shard refuses every Steam sign-in | P2, `REFUSE_EDITION` |
| a refusal or listing in Gates 2 names no store, coin or link | P2, P3 |
| the mint job never touches a plain world | P1, `mint_pending.py` |
| a Steam↔wallet link is private unless its holder shows it | P6, `walletlink.py` |
| nothing earned in a Steam session becomes money or an NFT | all of the above |
| no reward to a linked wallet for Steam play: no airdrop, no free copy, no premium credit for Gates 2 hours | P6, the store's copy |
| no bought JUNK or ELO ever enters an island's world: PvP over carried JUNK stays play, not a wager (Epic's gambling line, and the law's) | premium islands, always |
| Gates 2 lists only the house's economy islands, under house names | P3 |
