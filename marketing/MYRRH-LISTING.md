---
status: note
lane: [economy, brand]
updated: 2026-08-07
about: "MYRRH's listing copy — the short and long token-info blurbs for an explorer, a DEX listing or a wallet, plus the facts table behind them"
---
# MYRRH — listing copy

> **Moved here from `scry-forge/watchtower/marketing/` on 2026-08-11.** MYRRH is
> **Gates' coin, not the platform's** (operator, 2026-08-07: *"scry only has
> SCRY! myrrh and obol are actually gates now"*), so its listing copy belongs in
> this repo — scry's rule is that nothing about a game lives there except its
> listing row. Nothing moved on chain; the contract, the pool and the address
> are untouched.
>
> ⚠ **The derivation commands below run in `scry-forge`, not here.**
> `pool_seeds.py`, `contracts/…` and the deploy scripts are all paths in that
> repo, because the contracts and the pool seeds are the platform's to hold even
> for a coin the game owns. Re-derive there, paste here.

Paste-ready token-info text. Every number below was derived
(`python3 pool_seeds.py`, `python3 contracts/preflight.py`,
`contracts/script/DeployGardener.s.sol`, `contracts/src/SpoilsToken.sol` —
**in `scry-forge`**) — **re-derive before you paste**, and never retype a figure
from here into another page. Sibling copy: `OBOL-LISTING.md`.

---

## Short — ~50 words, for a token-info field

> **MYRRH** is Gates' scarce coin — the capped half of the survival game's
> pair on scry, a curated open-source game platform on RH-Chain. Capped at
> 21,000,000 and farmed on a halving schedule, it has exactly one source —
> staking liquidity in the Garden — and one job: it burns. Playing mints
> none of it.

## One-liner — for a name field or a card subtitle

> Gates' scarce coin. Farmed, never played for. Burned, never pooled into a
> prize.

---

## Long — ~150 words, for a project-description box

> **MYRRH is Gates' scarce coin.**
>
> scry (`scry.moreright.xyz`) is a curated, open-source game platform on
> RH-Chain — games built by agents in public, played by humans and agents,
> settled on chain. Gates, the first title, runs two coins: OBOL is what you
> earn by playing. MYRRH is the coin you cannot play for at all.
>
> It has **one source**: stake the Garden's liquidity shares in the Gardener
> and farm it, on a halving schedule that starts at 600 MYRRH/day and stops
> dead after forty years. Supply is capped at **21,000,000**, and the cap is a
> lifetime mint budget — burning retires supply and never re-opens room to
> mint.
>
> Spending it destroys it. Every posted MYRRH sink is a burn — an offering at
> the on-chain shrine, and the game's own sinks under the same rule — never a
> route into a pot.
>
> No fee in any amount moves a measurement. MYRRH cannot buy a score.

---

## Facts table

| field | value |
|---|---|
| name | Myrrh |
| symbol | `MYRRH` |
| decimals | 18 |
| chain | RH-Chain (`eip155:4663`) |
| contract | `0xde967108cc27db651e8cfec7dd18db814508b893` — **live on 4663, source-verified** |
| standard | ERC-20 (`contracts/src/SpoilsToken.sol`) |
| max supply | **21,000,000**, set at deploy and **immutable** |
| what the cap means | a **lifetime mint budget**, not a supply ceiling. `totalMinted` is monotone; a burn moves `totalBurned` and `totalSupply` and never refunds mint room |
| emission | **the Gardener farm is the only source.** Era 0 pays **600 MYRRH/day**, halving every four years, stopping dead at forty |
| what is welded | the **shape** — the halving period and the number of halvings are contract constants. The rate itself is an owner dial (`setRewardPerSecond`) |
| how you get it | stake `SEED` — the MYRRH/OBOL Garden's liquidity shares — in the Gardener. **Play mints none** |
| farm discipline | 67% of each harvest locks · a **single cliff at deploy + 90 days**, not a rolling lock · a withdrawal-fee slash ladder measured from your last deposit, which *is* rolling |
| burn | open — any holder may `burn` their own, or `burnFrom` an allowance |
| admin surface | **none beyond the minter role.** No owner, no pause, no blacklist, no transfer fee, no upgrade proxy. The minter cannot exceed the cap and cannot be set to the zero address |
| sibling coin | **OBOL** — Gates' elastic coin, uncapped, minted by play |
| base pair | `SCRY` — the platform's fixed-supply reserve, `0xDa2a4b23459e9ca88183e990802be644AcA7C4B0` |

**The cap is never reached, by construction.** The farm's entire forty-year run
emits a small fraction of the 21,000,000, and the rest is deliberate headroom
for MYRRH sources added later — every future source shares this one ceiling,
because `cap` is immutable and `totalMinted` never decrements. **Do not retype
the float or the headroom anywhere**; `python3 contracts/preflight.py` computes
and prints both on every run, and typed copies have gone stale three times.

## Where MYRRH goes — every sink is a burn

The standing rule: **a MYRRH sink retires the coin, never routes it into a
pot.** Burn is open on the contract (any holder may `burn`), the on-chain
shrine takes a plain offering — no payout, no buff, no odds — and Gates' own
in-game sinks are the game's to design under the same rule. (The town's old
MYRRH rooms — the Reliquary, the Agora's charm, the Roads fairs — retired
from the product on 2026-08-02 and are not sinks to advertise.)

## Opening liquidity, at launch

Derive with `python3 pool_seeds.py` — it parses the shipped defaults out of
`contracts/deploy_town.sh`, which is the script that broadcasts.

| pool | MYRRH side | other side | opening cross |
|---|---:|---:|---:|
| MYRRH / `SCRY` | 470,000 | 23,500,000 `SCRY` | 50 `SCRY` per MYRRH |
| MYRRH / OBOL | 2,000,000 | 10,000,000 OBOL | 5 OBOL per MYRRH |

Both pools are protocol-owned and open at genesis, so MYRRH is one swap from
`SCRY` and one swap from OBOL. A player who earns OBOL is always one swap from
MYRRH — there is no routing cliff between the coins. The MYRRH/OBOL pool
costs no `SCRY`: both sides are house-minted.

Because the house mints both sides, **the MYRRH/OBOL price is never an oracle
for any scry system.**

## What MYRRH is not

- **Not a governance token.** No token on the platform carries governance
  weight.
- **Not the platform's coin.** scry has exactly one coin — SCRY, the reserve.
  OBOL and MYRRH are Gates' (`SENTENCES.md` 2026-08-07); every listed title
  brings its own coins, and they all pair against SCRY.
- **Not a yield instrument.** Nothing here promises a return, and the
  measurement side of the product cannot be bought at any price.
- **Not farmable with `SCRY`.** The canonical `SCRY` pools are never farmed.
  The money token is never emitted by the farm, so it does not inflate to pay
  for its own liquidity.
- **Not earnable by playing.** This is the deliberate difference from OBOL, and
  it is what makes the cap safe to weld.

## Status — say this plainly, it is checkable

**MYRRH is deployed and source-verified on chain 4663, and the farm is live** —
the Gardener has been minting since 2026-07-30, which is the first time MYRRH
was earnable at all. Every MYRRH that existed before that was hand-minted.
Never retype a supply figure from here: read `totalMinted()` / `cap()` off the
contract, or `/api/onchain`.

**Two facts a rug screen will check, stated before it asks:**

- **`minter()` is the MYRRH granary, not an EOA** — and the **21,000,000 cap is
  immutable**, so no key can exceed it.
- **Nothing has been retired yet.** The deploy wallet holds the granary's
  **steward** seat and the slot can be handed anywhere. That is deliberate and
  undoable while the town is still being built; retiring it is its own operator
  act and is not done.

(An earlier copy of this page carried a dated obligation about the granary
grant being 1,250 MYRRH/day. It was resolved 2026-07-30 — the grant is sized
by the cliff, no dated obligation remains, and the live number is readable:
`cast call $GRANARY_MYRRH 'availableToday(address)(uint256)' $GARDENER`.)

## Links

| | |
|---|---|
| site | `https://scry.moreright.xyz` |
| the farm | `https://scry.moreright.xyz/gardens` |
| pools + fees, live | `https://scry.moreright.xyz/api/pools` |
| on-chain card | `https://scry.moreright.xyz/api/onchain` |
| icon | `watchtower/marketing/myrrh-icon.svg` · flat: `myrrh-flat.svg` |

---

### Notes for whoever pastes this

- **Tickers are bare** — `MYRRH`, `OBOL`, `SCRY`. Never `$SCRY`; a `$` in this
  repo is a shell variable.
- **600/day and 12,500/day are different numbers with different jobs, and they
  get confused.** 600 MYRRH/day is the **era-0 emission rate**
  (`REWARD_PER_SECOND`). 12,500 MYRRH/day is `FARM_DAILY_CAP` — the granary's
  daily mint **throttle**, sized to drain the day-90 cliff lump in about three
  days rather than about a month. Only the first belongs in listing copy.
- **Drop one's MYRRH pot is stealth and is deliberately not advertised here.**
  If a listing form asks about distribution, the honest answer is the cap, the
  single source and the schedule — not a drop the recipients have not been told
  about.
- **Do not say "scarce by emission, room 3 of the barrow."** The barrow's MYRRH
  bands went empty on 2026-07-26; play mints no MYRRH at all. Two live error
  strings still say otherwise — see the note below.
- **`TOKENOMICS.md` is stale on the charm price** (it says base 30; `agora.py`
  says base 10). Do not quote a charm price from a doc.
