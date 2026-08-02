#!/usr/bin/env bash
# ci/gates.sh — exactly what CI runs. Run it before every PR.
#
# Two stages: the docs gate (always) and the code gates (the workspace).
# The one rule this script must never break (CLAUDE.md §traps): a suite
# that cannot run its work must say so LOUDLY and exit nonzero — a pass
# it didn't earn is the worst bug class. The nightly BUILDER may report
# "nothing to build"; this TEST gate may not.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0

# ── docs gate: tickers are bare (wall 8) ────────────────────────────────
# SCRY, OBOL, MYRRH — never a `$` prefix. A `$` is a shell variable.
if git grep -nE '\$(SCRY|OBOL|MYRRH)\b' -- '*.md' '*.toml' '*.rs' 2>/dev/null; then
  echo "GATE RED: \$-prefixed ticker found — tickers are bare (CLAUDE.md wall 8)" >&2
  fail=1
else
  echo "gate green: tickers bare"
fi

# ── code gates ──────────────────────────────────────────────────────────
if [[ -f Cargo.toml ]]; then
  echo "── cargo fmt ──"
  cargo fmt --all --check
  echo "── clippy (walls ride in clippy.toml / lint config) ──"
  cargo clippy --workspace --all-targets -- -D warnings
  echo "── the gates ──"
  cargo test --workspace
else
  cat >&2 <<'EOF'
GATE RED: THE WORKSPACE DOES NOT EXIST YET.
M0 (NOW.md #1) is the current work: workspace + five crates, sim-core
tick/worldgen/state_hash, protocol codec + goldens, server, client, and
the gates themselves (test_alloc_zero, test_replay, test_parity_wasm,
test_protocol_golden). This script fails on purpose until that lands —
a suite must never report a pass for work it did not do.
EOF
  fail=1
fi

exit "$fail"
