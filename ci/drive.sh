#!/usr/bin/env bash
# Drive the real game under Xvfb with xdotool and photograph what it shows:
# panels, the map, night, weather — the screens `--capture` cannot reach.
#
#   ./ci/drive.sh <outdir> <script> [shard.toml lines…]
#   ./ci/drive.sh shots/look ci/drive/look.txt 'dev_env = "clear,midnight"'
#
# Script lines (one verb each; `#` comments):
#   sleep N · key K… · down K · up K · click B · mdown B · mup B
#   move DX DY (relative, pointer-accelerated) · moveto X Y · type TEXT
#   shot NAME   (F12 in the game, copied to <outdir>/NAME.png)
#
# Env: KIT (dev_spawn_kit), POP (population, 3), SPAWN ("1500,600"), PORT
# (4455), RES (1280x720). Needs Xvfb, xdotool and lavapipe
# (mesa-vulkan-drivers). Not a gate: it scores nothing (`CLAUDE.md`).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT=$1; SCRIPT=$2; shift 2
mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd)
SCRIPT=$(cd "$(dirname "$SCRIPT")" && pwd)/$(basename "$SCRIPT")
cd "$ROOT"
[ -x target/release/shard ] && [ -x target/release/gates ] || {
  cargo build --release -p server --bin shard -p client --features client/render --bin gates || exit 1
}
WORK=$(mktemp -d)
PORT=${PORT:-4455}
{
  echo "bind = \"127.0.0.1:$PORT\""
  echo "seed = 20260731"
  echo "dev_spawn = \"${SPAWN:-1500,600}\""
  echo "dev_spawn_kit = \"${KIT:-item.wood:1000, item.torch:1, item.hatchet_stone:1, item.bow:1, item.arrow_wood:20, item.box_small:1, item.building_plan:1, item.hammer:1}\""
  echo "require_auth = false"
  echo "population = ${POP:-3}"
  echo "dev_heli = false"
  for l in "$@"; do echo "$l"; done
} > "$WORK/s.toml"
target/release/shard "$WORK/s.toml" > "$WORK/shard.log" 2>&1 &
SP=$!
for _ in $(seq 1 240); do grep -q "shard up on" "$WORK/shard.log" && break; sleep 0.25; done
grep -q "shard up on" "$WORK/shard.log" || { cat "$WORK/shard.log"; kill $SP; exit 1; }
DISP=77
while [ -e "/tmp/.X11-unix/X$DISP" ]; do DISP=$((DISP + 1)); done
Xvfb ":$DISP" -screen 0 "${RES:-1280x720}x24" >/dev/null 2>&1 &
XP=$!
sleep 1
export DISPLAY=":$DISP"
mkdir -p "$WORK/raw"
GATES_SHOTS_DIR="$WORK/raw" VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan \
  target/release/gates --server "127.0.0.1:$PORT" > "$WORK/client.log" 2>&1 &
CP=$!
shot() {
  local before; before=$(ls "$WORK/raw" | wc -l)
  xdotool key F12
  for _ in $(seq 1 60); do [ "$(ls "$WORK/raw" | wc -l)" -gt "$before" ] && break; sleep 0.25; done
  local f; f=$(ls -t "$WORK/raw" | head -1)
  [ -n "$f" ] && cp "$WORK/raw/$f" "$OUT/$1.png" && echo "shot $1"
}
while read -r cmd a b; do
  case "$cmd" in
    ''|'#'*) ;;
    sleep) sleep "$a" ;;
    key) xdotool key $a $b ;;
    down) xdotool keydown "$a" ;;
    up) xdotool keyup "$a" ;;
    click) xdotool click "$a" ;;
    mdown) xdotool mousedown "$a" ;;
    mup) xdotool mouseup "$a" ;;
    move) xdotool mousemove_relative -- "$a" "$b" ;;
    moveto) xdotool mousemove "$a" "$b" ;;
    type) xdotool type "$a $b" ;;
    shot) shot "$a" ;;
    *) echo "drive: unknown verb $cmd" ;;
  esac
done < "$SCRIPT"
kill $CP 2>/dev/null; sleep 1; kill $SP $XP 2>/dev/null
cp "$WORK/client.log" "$WORK/shard.log" "$OUT/"
echo "drive: done → $OUT"
