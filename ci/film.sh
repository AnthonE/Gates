#!/usr/bin/env bash
# Trailer footage and ads, on a box with no GPU.
#
#   ci/film.sh record TAPE.rec [SECONDS] [options]   # tape a populated shard
#   ci/film.sh scan TAPE.rec [EVERY_S]               # who is where, charges, blasts
#   ci/film.sh shoot SHOTS.json [MORE.json ...]      # draw shot lists (lavapipe)
#   ci/film.sh cut CUT.json [MORE.json ...]          # edit ads (ci/film_cut.py)
#   ci/film.sh bank DIR                              # the game's music and sfx as WAVs
#
# record options: --population N (8) · --spawn x,z (1650,1590) · --walk x,z
#                 (where the recorder stands, 1672,1612) · --no-charges (the
#                 raiders are armed by default) · --seed N · --port N
#                 The defaults are the tape the lists in ci/film/ were shot on:
#                 a base on the slope above the sunset beach, raided twice.
#
# How it works: `record` joins a live shard as a guest and writes down every
# datagram and event with the time it arrived (crates/client/src/bin/record.rs).
# `gates --film` hands those bytes back to an ordinary session on a fixed
# frame clock with a scripted camera and pipes each shot into ffmpeg
# (render/film.rs), so lavapipe at ~1 frame a second still makes smooth 30 fps
# footage of bots building and raiding. `scan` prints a tape's timeline — who
# is where, when charges are planted and pieces come down — which is how a shot
# list finds its moments. Shot lists are JSON, documented in render/film.rs;
# cut lists at the top of ci/film_cut.py. The house lists are in ci/film/.
#
# The tape is wire bytes: re-record it after a PROTO_VER bump.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=ci
BIN="target/$PROFILE"
fail() { echo "film: $*" >&2; exit 1; }

cmd="${1:-}"
[ -n "$cmd" ] || { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
shift

case "$cmd" in
record)
  TAPE="${1:?record needs a tape path}"; shift
  SECONDS_=600
  case "${1:-}" in ''|*[!0-9.]*) ;; *) SECONDS_="$1"; shift ;; esac
  POPULATION=8; SPAWN="1650,1590"; WALK="1672,1612"; CHARGES=1; SEED=20260731; PORT=4433
  while [ $# -gt 0 ]; do
    case "$1" in
      --population) POPULATION="$2"; shift 2 ;;
      --spawn) SPAWN="$2"; shift 2 ;;
      --walk) WALK="$2"; shift 2 ;;
      --no-charges) CHARGES=0; shift ;;
      --seed) SEED="$2"; shift 2 ;;
      --port) PORT="$2"; shift 2 ;;
      *) fail "unknown record option $1" ;;
    esac
  done
  cargo build --profile "$PROFILE" -p server --bin shard -p client --bin record >/dev/null 2>&1 \
    || fail "the shard or the recorder did not build"
  WORK="$(mktemp -d)"
  trap '[ -n "${SHARD_PID:-}" ] && kill "$SHARD_PID" 2>/dev/null; rm -rf "$WORK"' EXIT
  KIT="item.wood:1000, item.box_small:1, item.lock_code:1"
  [ "$CHARGES" = 0 ] || KIT="item.satchel_charge:1, $KIT"
  cat > "$WORK/film.toml" <<TOML
bind = "127.0.0.1:$PORT"
seed = $SEED
dev_spawn = "$SPAWN"
dev_spawn_kit = "$KIT"
require_auth = false
population = $POPULATION
TOML
  "$BIN/shard" "$WORK/film.toml" > "$WORK/shard.log" 2>&1 &
  SHARD_PID=$!
  for _ in $(seq 1 240); do
    grep -q "population $POPULATION seated" "$WORK/shard.log" && break
    kill -0 "$SHARD_PID" 2>/dev/null || { cat "$WORK/shard.log" >&2; fail "the shard exited"; }
    sleep 0.25
  done
  grep -q "population $POPULATION seated" "$WORK/shard.log" || fail "the population never seated"
  mkdir -p "$(dirname "$TAPE")"
  echo "film: taping $SECONDS_ s of seed $SEED, $POPULATION inhabitants at $SPAWN"
  ARGS=("127.0.0.1:$PORT" "$TAPE" "$SECONDS_")
  [ -z "$WALK" ] || ARGS+=(--walk "$WALK")
  "$BIN/record" "${ARGS[@]}"
  ;;

scan)
  cargo build --profile "$PROFILE" -p client --bin record >/dev/null 2>&1 \
    || fail "the recorder did not build"
  "$BIN/record" --scan "${1:?scan needs a tape}" "${2:-5}"
  ;;

shoot)
  [ $# -gt 0 ] || fail "shoot needs a shot list"
  cargo build --profile "$PROFILE" -p client --features render --bin gates >/dev/null 2>&1 \
    || fail "the client did not build (CLAUDE.md: fresh box needs libwayland-dev libasound2-dev libudev-dev)"
  command -v ffmpeg >/dev/null || fail "no ffmpeg (apt-get install ffmpeg)"
  ICD=/usr/share/vulkan/icd.d/lvp_icd.json
  [ -f "$ICD" ] || fail "no lavapipe ICD at $ICD — apt-get install mesa-vulkan-drivers"
  DISP=99
  while [ -e "/tmp/.X11-unix/X$DISP" ]; do DISP=$((DISP + 1)); done
  Xvfb ":$DISP" -screen 0 2048x2048x24 >/dev/null 2>&1 &
  XVFB_PID=$!
  trap 'kill "$XVFB_PID" 2>/dev/null' EXIT
  for _ in $(seq 1 40); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.25; done
  for SHOTS in "$@"; do
    echo "film: shooting $SHOTS"
    VK_DRIVER_FILES="$ICD" DISPLAY=":$DISP" WGPU_BACKEND=vulkan \
      "$BIN/gates" --film "$SHOTS" 2>&1 | grep --line-buffered -E "film:|gates:|ERROR|panicked" \
      | sed -u 's/\x1b\[[0-9;]*m//g'
    [ "${PIPESTATUS[0]}" = 0 ] || fail "$SHOTS did not finish"
  done
  ;;

cut)
  python3 -c "import PIL" 2>/dev/null || fail "no Pillow (pip install Pillow)"
  python3 ci/film_cut.py "$@"
  ;;

bank)
  cargo run --profile "$PROFILE" -q -p client --bin soundbank -- "${1:?bank needs a directory}"
  ;;

*) fail "unknown command $cmd (record, scan, shoot, cut, bank)" ;;
esac
