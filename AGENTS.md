# AGENTS.md — the mission file

You are an agent (or a human — same rules) arriving to build **Gates**: an
open-source browser survival game in the Rust tradition — Rust-language
authoritative server, three.js client, WebTransport/QUIC. The skeleton is
the product: determinism, netcode, and the hot-path laws outrank every
feature. This file is the whole onboarding; any harness that can read a
file and run a shell can contribute.

## The 90-second start

1. Read `CLAUDE.md`. It is the law of this repo and it is short.
2. Read `NOW.md`. Pick the **top item you can actually finish**. Right now
   the workspace does not exist yet — **M0 is the current work**, and
   `./ci/gates.sh` says so honestly (it fails until the workspace lands;
   that is the gate working, not the gate broken).
3. Read the doc that **owns** your area before touching it — the table in
   `CLAUDE.md` §the docs says which doc wins. Numbers live in `CONTENT.md`
   schemas and `DECISIONS.md`, never in code.
4. Branch → build → **`./ci/gates.sh` green locally** → open a PR. Fill the
   template. One crate per PR. Never push to `main`.
5. Want to be paid? See **the deal** below.

## The deal — how work becomes money

Development is funded through scry's Great Work board, and the loop is live
today, end to end, with no account and no API key:

- Board: `https://scry.moreright.xyz/api/munus` — quests, claims, and the
  paid ledger, all public.
- Identity is a wallet. Swear a vow (free), then claim a quest and submit
  your delivery — both are one EIP-191 `personal_sign` each; the exact
  texts to sign are served at `GET /api/play/message`. First-come, one
  claim per vow per UTC day, an idle claim lapses in 14 days.
- Delivery is a PR link. The operator reviews and pays in SCRY by public
  transfer — the chain is the receipt. Bounties are unpriced until
  claimed; write the operator before pricing big work.
- Full agent onboarding for the scry side: `https://scry.moreright.xyz/api/llms.txt`.

**Merge is a human act.** Paid is not merged and merged is not paid; both
are decisions a person makes by hand, on purpose.

## The bar

Operator, 2026-08-01: *"not any ai slop but the best here."* Gates green is
the **floor**, not the bar. A PR gets rejected even with every gate green
when it is:

- allocation or locks smuggled into the hot path behind a warmup or a cfg,
- an invented number (every tunable is spoken in `DECISIONS.md` or ships
  its documented default — propose new ones in `DECISIONS.md` §open),
- a test edited to make a wall pass (a change that reddens a wall does not
  merge, ever — fix the change, not the wall),
- a giant mixed diff, comment churn, or README landscaping dressed as work,
- code that ignores the surrounding idiom.

Small, sharp, finished. One owner per crate per iteration; `protocol` and
`limits.rs` never land from two branches in one merge window.

## The walls, one line each (the law is `CLAUDE.md`; these are reminders)

1. sim-core is pure — no I/O, clock, threads, map iteration, trig; float
   ops restricted. Gate: clippy walls + `test_parity_wasm`.
2. Zero allocation in the tick after warmup. Gate: `test_alloc_zero`.
3. No locks, syscalls, `String`, or logging in the sim thread — rings and
   integer event codes only. Gate: clippy + soak jitter assert.
4. Bounded everything; every client-driven `push` checks a cap from
   `limits.rs`. Gate: review + `test_raid_storm`.
5. Determinism: same build + seed + WAL → same hashes. Gate: `test_replay`,
   `test_terrain_golden`.
6. The wire never drifts by accident — layout change = version bump +
   regenerated goldens in the same commit. Gate: `test_protocol_golden`.
7. Content never touches code — items and balance live in `content/*.toml`,
   content hash pinned into the WAL header. Gate: `test_content`.
8. Money is appearance-only from the house; the never-table is a wall.
   Tickers are bare: SCRY, OBOL, MYRRH — never a `$` prefix. Gate:
   `ci/gates.sh` docs check.

## Commands (derive, don't quote)

```
./ci/gates.sh                       # exactly what CI runs — run before every PR
cargo test --workspace              # every headless gate (once M0 lands)
cargo run -p server --bin shard     # the server (reads shard.toml)
cargo run -p server --bin bots -- 100
cargo run -p server --bin replay -- --wal <file>
cargo run -p client --features render --bin gates   # the game
```

## CI and nightlies

- **`gates` workflow** — runs `./ci/gates.sh` on every PR and push to
  `main` that touches code paths. Red means do not merge; there is no
  override lane.
- **`nightly` workflow** — every night, builds the release server and the
  wasm client from `main` and uploads them as artifacts. Before M0 it
  reports "nothing to build yet" and exits cleanly — a builder with
  nothing to build is a fact, not a failure; the *test* gate is the one
  that is never allowed to skip-pass.

## For any harness

This file follows the `AGENTS.md` convention many harnesses auto-load.
There is nothing harness-specific in this repo: contribution is git + a
shell + (optionally, to be paid) one wallet signature per board verb.
Claude Code, Hermes, OpenClaw, a plain HTTP loop — all equal citizens.
Humans too.
