#!/usr/bin/env bash
# Has upstream moved under the vendored elo SDK? (NOW.md §7 item 2)
#
# `crates/client/src/elo_overlay.rs` is vendored from `AnthonE/scry-forge`
# (`sdk/rust/elo_overlay.rs`). `VENDORED_SHA256` catches a local edit and
# cannot catch upstream moving; drift once broke every login for eight days.
# This compares our bytes against the hash scry-forge publishes beside the
# SDK — scry-forge, never the `scryward` mirror, which lags and would read a
# false green (`crates/client/src/elo.rs` says why).
#
#   ci/elo_drift.sh                 # fetch sdk/SHA256SUMS (needs SCRY_FORGE_TOKEN)
#   ci/elo_drift.sh <SHA256SUMS>    # check against a file already in hand
set -euo pipefail
cd "$(dirname "$0")/.."

have=$(sha256sum crates/client/src/elo_overlay.rs | cut -d' ' -f1)
pin=$(grep -A1 'pub const VENDORED_SHA256' crates/client/src/elo.rs | grep -o '[0-9a-f]\{64\}')
if [ "$have" != "$pin" ]; then
  echo "elo_drift: elo_overlay.rs ($have) is not VENDORED_SHA256 ($pin): edited here. Fix it upstream and re-copy." >&2
  exit 1
fi

sums=${1:-}
if [ -z "$sums" ]; then
  : "${SCRY_FORGE_TOKEN:?elo_drift: set SCRY_FORGE_TOKEN, a token that can read AnthonE/scry-forge}"
  sums=$(mktemp)
  trap 'rm -f "$sums"' EXIT
  curl -fsSL \
    -H "Authorization: Bearer $SCRY_FORGE_TOKEN" \
    -H "Accept: application/vnd.github.raw" \
    https://api.github.com/repos/AnthonE/scry-forge/contents/sdk/SHA256SUMS -o "$sums"
fi

line=$(grep -E '(^|[ */])elo_overlay\.rs$' "$sums" || true)
if [ -z "$line" ]; then
  echo "elo_drift: scry-forge's sdk/SHA256SUMS names no elo_overlay.rs; the layout moved, so read it and fix this script." >&2
  exit 1
fi
upstream=$(echo "$line" | head -1 | grep -o '^[0-9a-f]\{64\}')
if [ "$upstream" != "$have" ]; then
  echo "elo_drift: upstream moved. scry-forge publishes $upstream, Gates vendors $have." >&2
  echo "  Re-vendor sdk/rust/elo_overlay.rs, re-pin VENDORED_SHA256, run the client's tests." >&2
  exit 1
fi
echo "elo_drift: OK ($have)"
