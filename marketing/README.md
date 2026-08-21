# marketing/ — Gates' own coins, described for the outside

> ⚠ **The product name is JUNK/ORBS/ELO; the on-chain symbol is still `OBOL/MYRRH/SCRY`, and this
> file follows the chain.** The 2026-08-21 rename (scry → Elo, OBOL → JUNK,
> MYRRH → ORBS) moved the game's words, not the deployment: `/api/onchain`,
> read that day, names SCRY, OBOL and MYRRH and does not know ELO, JUNK or
> ORBS. An ERC-20's `symbol()` is fixed at deploy, so **renaming a ticker is a
> redeploy** — new address, new pool, new listing — which is an on-chain
> operator act performed in `scry-forge` and pasted here. Until it happens
> every ticker and address below is correct as written and **must not be
> swept**: paste-ready copy that disagrees with the contract is worse than
> copy carrying an old name.


What a stranger reads about **OBOL** and **MYRRH** when they meet them
somewhere that is not this repo: an explorer's token-info field, a DEX
listing, a wallet's coin row.

| file | what it is |
|---|---|
| `OBOL-LISTING.md` | OBOL's listing copy — short and long blurbs, plus the facts table behind them |
| `MYRRH-LISTING.md` | the same for MYRRH |
| `obol-icon.svg` · `myrrh-icon.svg` | the full mark, for a listing that renders detail |
| `obol-flat.svg` · `myrrh-flat.svg` | the flat single-colour mark, for a 16–32px coin row |

## Why these live here now

They arrived from `scry-forge/watchtower/marketing/` on 2026-08-11. They were
written there because they were built there — OBOL and MYRRH were the first
coins on the platform and read as the platform's for months. They are not:
**elo has exactly one coin and it is SCRY** (operator, 2026-08-07: *"scry only
has SCRY! myrrh and obol are actually gates now"*), and elo's own rule is that
nothing about a game lives in that repo except its listing row. So this is the
separation catching up with the sentence.

**Nothing moved on chain.** Both tokens are deployed and source-verified on
RH-Chain (4663), both pools are open, and every address is what it was. What
changed is which repo owns the words.

## The one thing to know before editing

⚠ **Every number in these files is derived in `scry-forge`, not here.** The
contracts, the pool seeds and the preflight are the platform's to hold even for
a coin this game owns — `pool_seeds.py`, `contracts/preflight.py`,
`contracts/src/SpoilsToken.sol`. Re-derive there, paste here, and never retype a
figure out of one of these pages into another page. The live reads are
`elopros.com/api/onchain` and `/api/pools`.
