# Gates · BUSINESS.md — what we sell

**Read this when you are building the store, the entry price, or the bank.
Not otherwise.** It was wall 8 of `CLAUDE.md` and it did not belong there: it
had no gate, in a list whose header says a law without a gate is a mood, and it
cost context on every pass that never touched money. Engineering rules are in
`CLAUDE.md`. This is a product decision, owned by the operator, changed by the
operator saying so.

## We sell IAP. That is the business.

| sellable | |
|---|---|
| **the game itself** | a uniform entry price, one door at one price |
| **skins, cosmetics, appearance** | the main line |
| **anything player-to-player** | players sell each other everything; the pools exchange the coins |

Price and currency are `DECISIONS.md` §open. No number is invented here.

## The one line: don't sell an advantage over another player

Not a moral position — a product one, and the same line Rust holds while
selling skins at scale. A survival game is a fight between players; sell one of
them a bigger gun and the other stops playing.

Never for sale **from the house**:

- damage, armor, speed, capacity, gather rate
- upkeep, decay pauses, protection windows
- blueprints, tech, crafting speed
- loot odds, spawn quality, map intel
- queue priority over another player *(knob, default never)*

**A skin is not an advantage** and this table has never said it was. Sell every
skin you want, including ones that look expensive. What is out is a stat.

**The entry price is not on the table either** (operator, 2026-08-05): a game
that costs money sells access uniformly to everyone playing, so it grants
nobody an edge. What stays out is a *better door than the next player's*.

## Changing this

It is the operator's call and it does not need an argument from the code.
Say it, it goes in `DECISIONS.md` the same day, this file changes. Nothing in
`crates/` reads this document.

## Housekeeping

- Economy stages (A1/A2/A3) arm only by operator act — `ALPHA.md`.
- Tickers are bare: SCRY, OBOL, MYRRH. Never a `$` prefix.
