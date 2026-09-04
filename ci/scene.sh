#!/usr/bin/env bash
# Photograph the game with somebody else in it.
#
#   ./ci/scene.sh                      # 6 inhabitants, shipped seed, shots/<stamp>/
#   ./ci/scene.sh --population 12
#   ./ci/scene.sh --out /tmp/look --no-hud
#   ./ci/scene.sh --play               # walk into it yourself; F12 shoots
#   ./ci/scene.sh --charges            # arm the raiders (they may kill the camera)
#   ./ci/scene.sh --settle 240         # give the base longer to go up
#   ./ci/scene.sh --keep               # leave the shard's log and config behind
#
# ## What this is for
#
# `--capture` on its own photographs an EMPTY ISLAND. That is not a defect of
# the probe — it is one process, and one process is the whole population, so
# every frame it has ever taken is a picture of terrain. The two things
# anybody actually wants to look at are the two it could not reach: **another
# player**, and **a building**.
#
# Both already exist in this tree and had never been pointed at a camera:
#
#   · `population = N` in shard.toml seats bots over the shard's own wire —
#     the full QUIC handshake, `run_bot`, indistinguishable from a player.
#     Half of them build a twig base, deploy a box and arm a code lock; half
#     plant a charge and raid it (`crates/server/src/population.rs`).
#   · `dev_spawn = "x,z"` puts EVERY body on one point instead of hashing a
#     bearing per player id, which is what makes the builders and the camera
#     neighbours rather than kilometres of coastline apart.
#   · `dev_spawn_kit` hands them the wood, so the base goes up in the minute
#     the probe is standing there rather than after a shift of chopping.
#
# This script writes that config, boots that shard, and runs the probe at it.
# The aiming is `render/capture.rs`'s scene pass: it finds the nearest body
# and the nearest base and points the camera at them.
#
# ## What comes out
#
#   0-design.png … 5-sky.png   the six fixed vantages (unchanged, still citable)
#   6-swing.png                the verb pass: a swing and the mark it left
#   7-player.png               another player, at eye height
#   8-build.png                a base, from `BUILD_STANDOFF_M` back
#
# The last three are CONDITIONAL and say so when they are skipped — an island
# with no neighbour and no base is a real answer, and a frame not taken is
# better than a frame of empty grass presented as a base.
#
# ## `--play`: the other half, and often the better one
#
# The probe aims by arithmetic and a person aims by looking, so `--play` boots
# the same populated world and hands it to the interactive client instead of
# the probe. **F12 already takes a screenshot on any screen** (`render/shot.rs`
# — the game reads its own frame, so it works where a desktop screenshot key
# does not). Walk up to somebody, walk inside their base, press F12. That is
# the shot the scene pass keeps trying to compute, taken by someone who can
# see what is in the way.
#
# ## Not a gate, and must not become one
#
# It scores nothing. `CLAUDE.md` is explicit that the pixel gate was the
# mistake and that a person looking is the visual gate; this exists to remove
# the reason a person could not look, which was that there was nothing in the
# world to look at.
set -euo pipefail
cd "$(dirname "$0")/.."

POPULATION=6
SEED=20260731
# `RENDER.md`'s measured pin, and it is a SAFETY number before it is a framing
# one. The probe is an ordinary client: its body stands unarmed in the world
# for the whole run, and mob homes concentrate on the flat walkable interior —
# three of four runs died on one pass and were diagnosed as everything else
# first. 1500,600 is the ground the material measurements were taken on and
# has been quiet across runs. Do NOT pin the island centre (1024,1024), which
# is the middle of exactly where the roster homes.
#
# This rig raises that risk rather than lowering it: the scene pass stands
# still waiting for a base to go up, and a raider's charge does not check who
# is beside it. A probe that dies now says so and exits nonzero.
SPAWN="1500,600"
OUT=""
PORT=4433
HUD=1
KEEP=0
CHARGES=0
PLAY=0
# Extra items prepended to every inhabitant's kit, `dev_spawn_kit` syntax.
#
# **What it is for is the one question the default kit cannot answer.** The
# base kit is wood, a box and a lock — the things a bot needs to BUILD — and
# not one of them has a row in `ui::hold::HELD_MODELS`, so every body in
# `7-player.png` is correctly empty-handed and the frame says nothing about
# whether a remote's hand works. Measured on 2026-08-31, which is how this
# flag came to exist: the first two-player frame this repo has ever taken
# showed two silhouettes holding nothing, and there was no way to tell that
# from the failure it looks exactly like.
#
#   ./ci/scene.sh --kit "item.hatchet_stone:1"
#
# The modelled rows are keyed on `HELD_MODELS`, not on the item id — the
# hatchet is `item.hatchet_stone`. They are rock, stone hatchet, stone pickaxe, hammer,
# building_plan, wooden_spear, hunting_bow, torch and revolver.
KIT_EXTRA=""
# Seconds the world runs before the camera arrives, and it is doing two jobs.
#
# **It is a safety number first.** `dev_spawn` puts every body on one point,
# so a probe that connects the moment the population is seated spawns INSIDE
# six bots and is beaten or blown up before the first vantage — measured, and
# it is what the probe's death guard reports. Bots walk continuously, so by
# the time this has elapsed they are tens of metres out and spread.
#
# **It is a content number second**: a twig base is not up at second zero.
# Measured on this box at 110 s — six inhabitants had a base of foundations
# and floors standing, and the probe survived all nine frames.
#
# It is a clock, and `CLAUDE.md` is right that a gate must never wait on one.
# This is not a gate: it scores nothing and asserts nothing. The half that IS
# observable state — "is there a base yet" — is the probe's own
# `SUBJECT_FRAMES`, which reports what it found rather than assuming it. What
# this buys is the world having had a chance to become worth photographing.
SETTLE=120

while [ $# -gt 0 ]; do
  case "$1" in
    --population) POPULATION="$2"; shift 2 ;;
    --seed)       SEED="$2";       shift 2 ;;
    --spawn)      SPAWN="$2";      shift 2 ;;
    --out)        OUT="$2";        shift 2 ;;
    --port)       PORT="$2";       shift 2 ;;
    --play)       PLAY=1;          shift ;;
    --settle)     SETTLE="$2";     shift 2 ;;
    --charges)    CHARGES=1;       shift ;;
    --kit)        KIT_EXTRA="$2";  shift 2 ;;
    --no-hud)     HUD=0;           shift ;;
    --keep)       KEEP=1;          shift ;;
    -h|--help)    sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "scene: unknown argument \`$1\`" >&2; exit 1 ;;
  esac
done
[ -n "$OUT" ] || OUT="shots/$(date -u +%Y%m%d-%H%M%S)"

fail() { echo "SCENE FAIL: $*" >&2; exit 1; }

# ---------------------------------------------------------------- build ----
# Release, because the probe's frame rate is what its budgets are spent in:
# every wait in `render/capture.rs` is a FRAME COUNT, so a debug build does
# not merely run slower, it gives the bots less wall-clock to build in before
# `SUBJECT_FRAMES` runs out.
echo "scene: building (release)…"
cargo build --release -p server --bin shard >/dev/null 2>&1 \
  || fail "the shard did not build — run \`cargo build --release -p server --bin shard\` to see why"
cargo build --release -p client --features render --bin gates 2>&1 | tail -20 \
  || true
[ -x target/release/gates ] || fail "the client did not build. A fresh box needs \
libwayland-dev, libasound2-dev and libudev-dev to compile it, and \
libxkbcommon-x11-0 plus mesa-vulkan-drivers to run it (CLAUDE.md)."

# --------------------------------------------------------------- config ----
WORK="$(mktemp -d)"
CONF="$WORK/scene.toml"
LOG="$WORK/shard.log"
cleanup() {
  [ -n "${SHARD_PID:-}" ] && kill "$SHARD_PID" 2>/dev/null || true
  [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null || true
  if [ "$KEEP" = 1 ]; then
    echo "scene: kept $WORK (config + shard log)"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

# **No charge unless asked, and that is a photographer's call rather than a
# balance one.** Armed, the attacker half plants satchels on the point every
# body — including the camera — spawned on. The first run of this rig died to
# one, and the frame before it carried the HUD's own "CHARGE · 2s · N 11M".
# The base still goes up without them: wood is what a foundation costs.
KIT="item.wood:1000, item.box_small:1, item.lock_code:1"
[ "$CHARGES" = 0 ] || KIT="item.satchel_charge:1, $KIT"
# Prepended, so it lands on hotbar slot 0 — which is the slot a bot selects,
# and therefore the one that reaches the wire as `EntityState::held`.
[ -z "$KIT_EXTRA" ] || KIT="$KIT_EXTRA, $KIT"

# No save_file and no world_file on purpose: a scene is a fresh island every
# time. Persisting one would pin this config's seed and content hash into a
# file that the next run with a different seed refuses to boot against.
cat > "$CONF" <<TOML
bind = "127.0.0.1:$PORT"
seed = $SEED
# Everybody on one point: the builders and the camera have to be neighbours.
dev_spawn = "$SPAWN"
# The wood is the whole difference between a base and an empty field. A twig
# foundation is 50 wood (content/building.toml) and the content spawn kit is
# a rock and a torch, so a stock inhabitant has to chop for a shift first.
# Slot order is hotbar order (inventory::SpawnKit) and the raid script reads
# its charge from slot 0, which is why it goes first when there is one.
dev_spawn_kit = "$KIT"
# Guests, so require_auth must stay false — an authenticating shard refuses
# every inhabitant and the population re-dials its own closed door forever.
require_auth = false
population = $POPULATION
TOML

mkdir -p "$OUT"
echo "scene: seed $SEED · spawn $SPAWN · population $POPULATION · out $OUT"

# ---------------------------------------------------------------- shard ----
./target/release/shard "$CONF" > "$LOG" 2>&1 &
SHARD_PID=$!

# Poll for what the shard SAYS, never for a number of seconds. A fixed sleep
# is the failure `CLAUDE.md` names — it measures this box's load and not the
# shard — so this waits on two printed facts and gives up on a bounded number
# of looks rather than a deadline.
wait_for() { # <pattern> <tries> <what>
  local i=0
  while [ "$i" -lt "$2" ]; do
    grep -q "$1" "$LOG" && return 0
    kill -0 "$SHARD_PID" 2>/dev/null || { cat "$LOG" >&2; fail "the shard exited during boot"; }
    sleep 0.25; i=$((i + 1))
  done
  cat "$LOG" >&2
  fail "$3"
}
wait_for "shard up on" 240 "the shard never bound $PORT in 60 s of looking"
ADDR="$(sed -n 's/^shard up on \([^ ]*\).*/\1/p' "$LOG" | head -1)"
echo "scene: shard up on $ADDR"

if [ "$POPULATION" -gt 0 ]; then
  wait_for "population $POPULATION seated" 240 \
    "the shard bound but never seated its population — see the log above"
  echo "scene: $POPULATION inhabitants seated, raiding"
  if [ "$SETTLE" -gt 0 ]; then
    # See $SETTLE's definition: the pile has to disperse and a base has to go
    # up, and neither is a thing the shard prints. Counted down out loud so a
    # long one does not read as a hang.
    echo "scene: letting the world run $SETTLE s — the spawn pile disperses and a base goes up"
    for i in $(seq "$SETTLE" -10 1); do printf '\r  %3ds…' "$i"; sleep 10; done
    printf '\r        \r'
  fi
fi

# ----------------------------------------------------------------- play ----
# Hand the world to a person. No Xvfb and no lavapipe: this wants the real
# display and the real GPU, and the shard stays up until the client is closed
# (the trap tears it down).
if [ "$PLAY" = 1 ]; then
  echo "scene: launching the game — walk over, and press F12 to shoot."
  echo "scene: F12 shots land in $OUT"
  # `GATES_SHOTS_DIR` is taken verbatim and outranks the Pictures rule
  # (`client/src/shot.rs`) — it exists for exactly this, a script driving the
  # client that already knows where it wants the files.
  GATES_SHOTS_DIR="$OUT" ./target/release/gates --server "$ADDR"
  exit $?
fi

# ------------------------------------------------------------------ x/gpu --
# lavapipe: a CPU rasterizer, so every budget the probe spends is a frame
# count and none of them is a millisecond. The ICD filename is `lvp_icd.json`
# and NOT `lvp_icd.x86_64.json` — point this at a path that does not exist and
# the loader says only "Unable to find a GPU" (CLAUDE.md).
ICD=/usr/share/vulkan/icd.d/lvp_icd.json
[ -f "$ICD" ] || fail "no lavapipe ICD at $ICD — apt-get install mesa-vulkan-drivers"

DISP=99
while [ -e "/tmp/.X11-unix/X$DISP" ]; do DISP=$((DISP + 1)); done
Xvfb ":$DISP" -screen 0 1600x900x24 >/dev/null 2>&1 &
XVFB_PID=$!
for i in $(seq 1 40); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.25; done
[ -e "/tmp/.X11-unix/X$DISP" ] || fail "Xvfb never came up on :$DISP"

# ---------------------------------------------------------------- probe ----
ARGS=(--server "$ADDR" --capture "$OUT")
[ "$HUD" = 1 ] || ARGS+=(--no-hud)
echo "scene: shooting…"
set +e
VK_DRIVER_FILES="$ICD" DISPLAY=":$DISP" WGPU_BACKEND=vulkan \
  ./target/release/gates "${ARGS[@]}" 2>&1 | sed 's/^/  /'
RC=${PIPESTATUS[0]}   # cargo|tail reports tail's code, not cargo's (CLAUDE.md)
set -e

SHOTS=$(find "$OUT" -name '*.png' -size +0 | wc -l)
echo
if [ "$RC" != 0 ]; then
  echo "scene: the probe exited $RC — $SHOTS frame(s) landed in $OUT" >&2
  exit "$RC"
fi
echo "scene: $SHOTS frame(s) in $OUT"
ls -1 "$OUT"/*.png 2>/dev/null | sed 's/^/  /'
