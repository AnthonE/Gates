#!/usr/bin/env bash
# The local check in MVP mode (CLAUDE.md): format, the one text gate CI would
# otherwise fail an hour in, and a warnings-clean compile of the workspace AND
# the game (`--features render` code is invisible to every other command).
# Minutes cold, seconds warm. CI runs the full suite (`ci/gates.sh`).
#
#   ci/quick.sh                   # fmt + knob check + clippy (workspace, game)
#   ci/quick.sh sim-core server   # ...then those crates' tests
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all

if command -v node >/dev/null 2>&1; then
  node ci/knob_registry.mjs >/dev/null ||
    { echo "quick: a number you changed is pinned in DECISIONS.md §Open — update its value there" >&2; exit 1; }
else
  echo "quick: node not found — SKIPPING the knob check (CI still runs it)" >&2
fi

cargo clippy --workspace --all-targets -- -D warnings

pkg-config --exists wayland-client alsa libudev 2>/dev/null ||
  { echo "quick: the game needs: sudo apt-get update && sudo apt-get install -y libwayland-dev libasound2-dev libudev-dev" >&2; exit 1; }
cargo clippy -p client --features render --all-targets -- -D warnings

for crate in "$@"; do
  cargo test -p "$crate"
done
echo "quick: OK"
