# The counter that could not see the command

Measurement note, 2026-08-30, from lag-compensation slice 5.

## What happened

Slice 5 minted the favour at `server/src/core.rs`, replacing the `favour: 0`
literal that three judge reports in a row ranked as the repo's largest gap.
The slice landed with sixteen gates: the formula swept across its clamp, the
two inputs that mint zero, the ack-corroboration in both directions, the
band that absorbs reordering, and an end-to-end test asserting that a shard
executing a real datagram moved `favour_granted` and `favour_sum`.

Then the mutant was run, per `CLAUDE.md` — restore `favour: 0` at the
command construction, the exact literal that had shipped.

**All sixteen passed.**

## Why

The mint binds once and uses twice, which is the right shape:

```rust
let favour = stats::favour_for(now, view);
stats.record_favour(favour);
self.cmd_buf[n] = Command::Input { id: c.id, frame, favour };
```

The comment above it claimed the counter "cannot disagree with the command".
That is true of the code as written and **false as a property a gate can
hold**: the mutant separates the two lines, and every assertion is on the
counter. `cmd_buf` is private, so no test in the crate can read the field
that actually reaches the sim.

This is `CLAUDE.md`'s naive-rebuild trap one level over. There the defect was
a test whose "independent" implementation called the function under test;
here it is a counter written *beside* a value rather than *downstream* of it.
Both produce a number that agrees with itself and says nothing about the
thing under test. The general shape:

> **A counter adjacent to a value witnesses the value, never its
> destination.** If the defect you fear is "the value did not arrive", the
> observable has to be on the far side of the arrival.

## What fixed it

One gate on a consequence: two players on a shard, the victim inside the
fixture's 2 m reach for four ticks and outside it for four, one swing driven
through `push_input`. A stale ack mints 7, the sim rewinds, and the victim
loses hp. A fresh ack mints 3, reaches only ticks the victim had already
left, and the swing misses. The assertion is on `hp` — the only thing a
player experiences — and it needs no access to `cmd_buf` at all.

It is the only gate in the file that goes red under the shipped literal.

## The eight mutants

| # | mutant | caught by |
|---|---|---|
| 1 | `favour: 0` at the command (the shipped defect) | **only** the swing gate |
| 2 | believe the claim, no corroboration | the two ack-regression gates |
| 3 | `evidence > claim` instead of `>=` | nothing — **equivalent**, see below |
| 4 | pay out a staleness past the ceiling | the mints-nothing gate |
| 5 | no clamp at the mint | 3 gates |
| 6 | drop the interp-delay term | 3 gates |
| 7 | count zeros as granted favours | the unacked-client gate |
| 8 | never drain the disagreement relay | 2 gates |

Seven of eight caught. **M3 is semantically equivalent and no gate should
catch it**: the branch compares two ages against the same `now`, so
`evidence == claim` implies the two ticks are equal and both arms return the
same value. Recorded because "one mutant survived" reads as a hole until
somebody checks which.

## What this does not close

Nothing here has been fired over a real link. The shard's own measurement is
~4.1 ticks of favour on loopback, which is a floor and says nothing about a
200 ms connection, and `REWIND_MAX_TICKS = 7` remains a doc's number.
`favour_clamped` on `/status.json` is the counter that will answer it —
the fraction of fights where the 233 ms ceiling, rather than the link, is
the binding constraint. Read it before moving that constant.
