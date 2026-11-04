#!/usr/bin/env bash
# The fx stage, headless: one effect at a time in front of a dark card and a
# light card, a frame per step on a clock that steps 1/60 s, each composed
# into a strip with its numbers and a diff against the last run
# (`examples/fx_stage.rs`, `ci/fxstrip.mjs`, the `fx` skill).
#
#   ci/fx.sh list                                  # every effect it can stage
#   ci/fx.sh bullet-metal blast                    # → shots/fx/<name>/strip.png, numbers.json, diff.png
#   ci/fx.sh bullet-metal --knobs 'Sparks.speed=4,12;Flash.life=0.04,0.07'   # retune, no rebuild
#   ci/fx.sh bullet-metal --seed 7 --frames 15 --span 0.8 --bloom
#   ci/fx.sh --no-build …                          # reuse the last build
#
# Needs Xvfb, lavapipe (mesa-vulkan-drivers) and Playwright's Chromium, like
# ci/scene.sh. The dev profile, so it reuses what `cargo build` already made.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=${FX_OUT:-shots/fx}
BUILD=1
ARGS=()
NAMES=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --knobs|--seed|--frames|--span|--size) ARGS+=("$1" "$2"); shift ;;
    --bloom) ARGS+=("$1") ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) NAMES+=("$1") ;;
  esac
  shift
done
[ ${#NAMES[@]} -gt 0 ] || { sed -n '2,16p' "$0"; exit 2; }

fail() { echo "fx: $*" >&2; exit 1; }
BIN=target/debug/examples/fx_stage
if [ "$BUILD" = 1 ]; then
  cargo build -q -p client --features render,native --example fx_stage
fi
[ -x "$BIN" ] || fail "no $BIN — run without --no-build"

if [ "${NAMES[0]}" = list ]; then
  "$BIN" "$OUT" list
  exit 0
fi

ICD=/usr/share/vulkan/icd.d/lvp_icd.json
[ -f "$ICD" ] || fail "no lavapipe ICD at $ICD — apt-get install mesa-vulkan-drivers"
DISP=99
while [ -e "/tmp/.X11-unix/X$DISP" ]; do DISP=$((DISP + 1)); done
Xvfb ":$DISP" -screen 0 1600x900x24 >/dev/null 2>&1 &
XVFB_PID=$!
trap 'kill $XVFB_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 40); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.25; done
[ -e "/tmp/.X11-unix/X$DISP" ] || fail "Xvfb never came up on :$DISP"

# The last run becomes the one the next is measured against.
for n in "${NAMES[@]}"; do node ci/fxstrip.mjs keep "$OUT/$n"; done
set +e
VK_DRIVER_FILES="$ICD" DISPLAY=":$DISP" WGPU_BACKEND=vulkan \
  "$BIN" "$OUT" "${NAMES[@]}" "${ARGS[@]}" 2>&1 | grep -v -E '^\s*$|WARN.*(wgpu_hal|naga)|^ALSA lib|Screenshot saved' | sed 's/^/  /'
RC=${PIPESTATUS[0]}   # a pipe reports its last command's code (CLAUDE.md)
set -e
[ "$RC" = 0 ] || fail "the stage exited $RC"

for n in "${NAMES[@]}"; do node ci/fxstrip.mjs frames "$OUT/$n"; done
