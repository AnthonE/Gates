# Distill data

Kept because it cost money to make and cannot be re-recorded for free.

## `opus-2026-10-21/`

Claude Opus 5.5 (`claude-opus-5-5`, `crates/server/examples/claude_agent.py
--via api`) playing four bots for 100 minutes of wall clock on one
GatesBench island (seed 20260731, arc hour 48, `--think-ms 3000`):
972 calls, $19.08, 965 accepted decisions.

- `bot-0..3.jsonl.gz`: the `gates-decision-1` rows (`distill.rs` `row()`),
  one recording per bot. No chat text, no player names.
- `bench.json`: the run's `gates-bench-1` result. THE ISLAND 64.2
  (bots 21, 0, 105, 131).

## `students/`

`gates-student-1` files (`ops/distill/train.py`). The bred ones went
through `crates/server/examples/school.rs` and play as they are.

| file | from | held out, THE ISLAND |
|---|---|---|
| `scripted.json.gz` | the scripted policy, 2,999 warped decisions | 41.1 ± 12.2 |
| `scripted-bred.json.gz` | `scripted` + `gs1:wsO1nW7fY_xy0FAbuxvy6w` | 74.1 ± 22.6 |
| `opus.json.gz` | the Opus rows, `--val 0 --epochs 60` | 29.7 ± 6.7 |
| `opus-bred.json.gz` | `opus` + `gs1:W22DgiReCdTJCPwmfgXmgTjAi5rNizJf6VI3DvJU3BZecKK4h15Al2Dk` | 55.4 ± 7.9 |

Held out is 24 lockstep islands (`school` variants 30000-30023), 2 bodies,
30 minutes each, 95% intervals. The scripted policy itself scores
56.2 ± 18.1 on the same islands.

## Using it

```sh
python3 ops/distill/train.py ops/distill/data/opus-2026-10-21 -o student.json --val 0 --epochs 60
gunzip -k ops/distill/data/students/opus-bred.json.gz
cargo run -p server --release --example school -- --student ops/distill/data/students/opus-bred.json
cargo run -p server --release --bin jev-bot -- --bench 60 --warp --student ops/distill/data/students/opus-bred.json
```
