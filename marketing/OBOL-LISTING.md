---
status: note
lane: [economy, brand]
updated: 2026-08-07
about: "OBOL's listing copy — the short and long token-info blurbs for an explorer, a DEX listing or a wallet, plus the facts table behind them"
---
# OBOL — listing copy

> ⚠ **The product name is JUNK; the on-chain symbol is still `OBOL`, and this
> file follows the chain.** The 2026-08-21 rename (scry → Elo, OBOL → JUNK,
> MYRRH → ORBS) moved the game's words, not the deployment: `/api/onchain`,
> read that day, names SCRY, OBOL and MYRRH and does not know ELO, JUNK or
> ORBS. An ERC-20's `symbol()` is fixed at deploy, so **renaming a ticker is a
> redeploy** — new address, new pool, new listing — which is an on-chain
> operator act performed in `scry-forge` and pasted here. Until it happens
> every ticker and address below is correct as written and **must not be
> swept**: paste-ready copy that disagrees with the contract is worse than
> copy carrying an old name.


> **Moved here from `scry-forge/watchtower/marketing/` on 2026-08-11.** OBOL is
> **Gates' coin, not the platform's** (operator, 2026-08-07: *"scry only has
> SCRY! myrrh and obol are actually gates now"*), so its listing copy belongs in
> this repo — elo's rule is that nothing about a game lives there except its
> listing row. Nothing moved on chain; the contract, the pool and the address
> are untouched.
>
> ⚠ **The derivation commands below run in `scry-forge`, not here.**
> `pool_seeds.py`, `contracts/preflight.py` and `contracts/src/SpoilsToken.sol`
> are all paths in that repo, because the contracts and the pool seeds are the
> platform's to hold even for a coin the game owns. Re-derive there, paste here.

Paste-ready token-info text. Every number below was derived
(`python3 pool_seeds.py`, `python3 contracts/preflight.py`,
`contracts/src/SpoilsToken.sol` — **in `scry-forge`**) — **re-derive before you
paste**, and never retype a figure from here into another page.

---

## Short — ~50 words, for a token-info field

> **OBOL** is the working coin of Gates, the survival game on elo — a
> curated, open-source game platform on RH-Chain. Play mints it under a
> distributor-only grant; the game's own sinks burn it; it pairs against
> SCRY, the platform's reserve. There is no public faucet.

## One-liner — for a name field or a card subtitle

> Gates' working coin. Minted by play, burned by spending.

---

## Long — ~150 words, for a project-description box

> **OBOL is the working coin of Gates.**
>
> elo (`elopros.com`) is a curated, open-source game platform on
> RH-Chain — games built by agents in public, played by humans and agents,
> settled on chain. Gates, an open-source survival game, is the first title,
> and it runs two coins: OBOL the elastic one you earn by playing, MYRRH the
> capped one you cannot play for. You do not buy your way into OBOL — play
> mints it on posted rules, and the game's own sinks burn it back.
>
> Supply is elastic on purpose: OBOL mints with play and retires with
> spending, so the sinks are what hold it, not a ceiling. Mint is
> **distributor-only** — earned, never faucetted — and burn is open to anyone
> holding it.
>
> One rule the coin never touches: **no fee in any amount moves a
> measurement.** OBOL buys goods, entries and play. It cannot buy a score.

---

## Facts table

| field | value |
|---|---|
| name | Obol |
| symbol | `OBOL` |
| decimals | 18 |
| chain | RH-Chain (`eip155:4663`) |
| contract | `0xa003af4a6c38629a986545afc8f9312c7eb76220` — **live on 4663, source-verified** |
| standard | ERC-20 (`contracts/src/SpoilsToken.sol`) |
| supply model | **elastic, uncapped** (`cap()` returns 0) — mint and burn track the game economy |
| mint | **distributor-only.** One `minter` address; no public mint, no faucet, no claim-by-holding |
| burn | open — any holder may `burn` their own, or `burnFrom` an allowance |
| admin surface | **none beyond the minter role.** No owner, no pause, no blacklist, no transfer fee, no upgrade proxy. The minter can rotate itself away and cannot be set to the zero address |
| supply accounting | `totalMinted` and `totalBurned` are monotone and public; `totalSupply` = minted − burned. A burn never re-opens mint room |
| sibling coin | **MYRRH** — Gates' capped coin, 21,000,000, farm-emitted only |
| base pair | `SCRY` — the platform's fixed-supply reserve, `0xDa2a4b23459e9ca88183e990802be644AcA7C4B0` |

## Opening liquidity, at launch

Derive with `python3 pool_seeds.py` — it parses the shipped defaults out of
`contracts/deploy_town.sh`, which is the script that broadcasts.

| pool | OBOL side | other side | opening cross |
|---|---:|---:|---:|
| OBOL / `SCRY` | 7,650,000 | 76,500,000 `SCRY` | 10 `SCRY` per OBOL |
| MYRRH / OBOL | 10,000,000 | 2,000,000 MYRRH | 5 OBOL per MYRRH |

Both pools are protocol-owned and open at genesis, so OBOL is one swap from
`SCRY` and one swap from MYRRH — there is no routing cliff between the coins.
The MYRRH/OBOL pool costs no `SCRY`: both sides are house-minted.

Because the house mints both sides, **the MYRRH/OBOL price is never an oracle
for any elo system.**

## What OBOL is not

- **Not a governance token.** No token on the platform carries governance
  weight.
- **Not the platform's coin.** elo has exactly one coin — SCRY, the reserve.
  OBOL and MYRRH are Gates' (`SENTENCES.md` 2026-08-07); every listed title
  brings its own coins, and they all pair against SCRY.
- **Not a yield instrument.** Nothing here promises a return, and the
  measurement side of the product cannot be bought at any price.
- **Not a free faucet.** The mint gate is the safety property. A coin anyone
  can mint drains any pool it faces; this one is earned.

## Status — say this plainly, it is checkable

**OBOL is deployed and source-verified on chain 4663.** Never retype a supply
figure from here: read `totalMinted()` / `totalBurned()` / `totalSupply()` off
the contract, or `/api/onchain`, which is the only thing that upgrades a
contract from *written* to *deployed*.

**Two facts a rug screen will check, stated before it asks:**

- **`minter()` is the OBOL granary, not an EOA** — mint authority moved there
  when the farm went live.
- **Nothing has been retired yet.** The deploy wallet holds the granary's
  **steward** seat, `stewardMint` is uncapped, and the slot can be handed
  anywhere. That is deliberate and undoable while the town is still being
  built; retiring it is its own operator act and is not done.

## Links

| | |
|---|---|
| site | `https://elopros.com` |
| the store | `https://elopros.com/` |
| pools + fees, live | `https://elopros.com/api/pools` |
| on-chain card | `https://elopros.com/api/onchain` |
| icon | `watchtower/marketing/obol-icon.svg` · flat: `obol-flat.svg` |

---

### Notes for whoever pastes this

- **Tickers are bare** — `OBOL`, `SCRY`, `MYRRH`. Never `$SCRY`; a `$` in this
  repo is a shell variable.
- **Drop one is stealth and is deliberately not advertised here.** If a listing
  form asks about distribution, the honest answer is the mint gate and the pool
  seeds, not a drop the recipients have not been told about.
- **The town's own game rooms retired from the product on 2026-08-02** (the
  Barrow delve, duels, the Table — `SENTENCES.md`, same date). Do not list
  them as OBOL's earn paths; play-minting is Gates' side of the model now.
