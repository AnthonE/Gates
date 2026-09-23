#!/usr/bin/env bash
# The local check in MVP mode (CLAUDE.md): format, then a warnings-clean
# compile of the workspace AND the game (`--features render` code is invisible
# to every other command).
# Minutes cold, seconds warm. CI runs the full suite (`ci/gates.sh`).
#
#   ci/quick.sh                   # fmt + clippy (workspace, then the game)
#   ci/quick.sh sim-core server   # ...then those crates' tests
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

pkg-config --exists wayland-client alsa libudev 2>/dev/null ||
  { echo "quick: the game needs: sudo apt-get update && sudo apt-get install -y libwayland-dev libasound2-dev libudev-dev" >&2; exit 1; }
cargo clippy -p client --features render --all-targets -- -D warnings

for crate in "$@"; do
  cargo test -p "$crate"
done
echo "quick: OK"
