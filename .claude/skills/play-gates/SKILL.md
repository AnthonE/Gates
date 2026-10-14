---
name: play-gates
description: Put Claude in the game. Builds jev-bot, makes the agent's wallet key once, and starts a Claude-driven agent on the public Gates island (or a private one with `local`), paid with the user's ANTHROPIC_API_KEY. Use when the user asks to play Gates with Claude, run their own AI agent, or get an AI teammate going from this repo.
---

# Play Gates with Claude

`jev-bot` is the agent's body. `crates/server/examples/claude_agent.py` is its
mind: it calls the Anthropic API with the user's `ANTHROPIC_API_KEY`, once per
decision. This session only sets things up and starts it. The full guide is
`crates/server/JEV.md`, "Bring your own agent".

Argument: `local` plays on a private island on this machine (no key, nobody
else there). Anything else, or nothing, is the public island,
`game.elopros.com:61234`. A word other than `local` is the agent's name
(default `claude`, 1 to 16 printable characters).

## 1. Check

Run these and stop at the first gap, saying how to fix it:

- `cargo --version`. Missing: install Rust from https://rustup.rs.
- `python3 -c "import anthropic"`. Missing: `pip install anthropic`.
- `test -n "$ANTHROPIC_API_KEY" && echo set`. Missing: the user exports it
  in their own shell and restarts this session. Never ask them to paste the
  key into the chat, and never print it.
- The public island needs a wallet key, and a key file needs Linux, macOS or
  WSL (it must be mode 0600). On plain Windows, offer `local` instead.

## 2. Build once

```sh
cargo build -p server --release --bin jev-bot
```

The first build takes several minutes. Afterwards call `target/release/jev-bot`.

## 3. Wallet key, once (public island only)

```sh
mkdir -p -m 700 ~/.gates
test -f ~/.gates/agent.key || target/release/jev-bot --new-key ~/.gates/agent.key
```

`--new-key` never overwrites a file and prints the address. Show the user the
address. Never read, cat or print the key file itself. Gates is free, so any
wallet gets in; nothing has to be bought or funded.

## 4. Play

Public island (run in the background):

```sh
target/release/jev-bot --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name claude \
    --seconds 3600 --think-ms 3000 --timeout-ms 20000 --max-requests-hour 300 \
    --external python3 crates/server/examples/claude_agent.py
```

Private island: the same, with `--local` in place of `--server …` and
`--agent-key …`. It prints the island's address to join from the game.

`--external` must stay the last jev-bot flag. Then tell the user, in a few
lines:

- the `watch it live:` link it printed (its own first-person view);
- to team up: in the game, look at it, press **N**, then `/t follow me`
  (`/t help` lists the rest: wood, stone, ore, cloth, home, a door code);
- to stop: kill the process. It also ends after `--seconds`.

## Cost

Every decision is one API request, so `--max-requests-hour` is the spend
bound. `CLAUDE_MODEL` picks the model (the script defaults to
`claude-opus-5-5`; `CLAUDE_MODEL=claude-sonnet-5-5` costs less). To bill the
user's Claude plan instead of the key, add `--via claude-code` after the
script, with `--think-ms 10000 --timeout-ms 30000`.
