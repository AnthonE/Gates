#!/usr/bin/env bash
# The Full Auto update trailer, drawn in a browser from the game's own models,
# its own sounds and footage shot with ci/film.sh:
#
#   ci/trailer/make.sh                     # → ci/trailer/out/full-auto.mp4 (1080p) and -720p.mp4
#   ci/trailer/make.sh stills 16,40.5      # just those frames, as JPEGs in build/stills
#   ci/trailer/make.sh preview             # every 4th frame at 720p, quick to check a cut
#
# trailer.html draws every frame as a pure function of t: render.mjs screenshots
# it in headless Chromium and pipes the frames to ffmpeg; mix.py lays the score
# and the hits on the cue list the page exports. The same shape as scry-forge's
# art/junk-trailer, and the two share render.mjs and mix.py.
#
# Footage: each NN-name.mp4 in SHOTS (default film/trailer-full-auto, written by
# `ci/film.sh shoot ci/film/full-auto.json`) becomes clips/<name>/. A scene whose
# clip is missing falls back to a store still from scry-forge (FORGE=…).
set -euo pipefail
cd "$(dirname "$0")"
HERE="$PWD"
GATES="$(cd ../.. && pwd)"
FORGE="${FORGE:-$GATES/../scry-forge}"
SHOTS="${SHOTS:-$GATES/film/trailer-full-auto}"
fail() { echo "trailer: $*" >&2; exit 1; }
command -v ffmpeg >/dev/null || fail "no ffmpeg"
command -v node >/dev/null || fail "no node"

MODE=video; STILLS=""
while [ $# -gt 0 ]; do
  case "$1" in
    stills) MODE=stills; STILLS="$2"; shift 2 ;;
    preview) MODE=preview; shift ;;
    *) fail "unknown option $1" ;;
  esac
done

B="$HERE/build"
mkdir -p "$B"/{lib,fonts,img,icons,glb,clips,music,snd,node_modules} "$HERE/out"

# three.js r160 and the loaders the game's GLBs need (KTX2 textures)
T3=https://cdn.jsdelivr.net/npm/three@0.160.0
for f in build/three.module.js examples/jsm/loaders/GLTFLoader.js examples/jsm/loaders/KTX2Loader.js \
         examples/jsm/utils/BufferGeometryUtils.js examples/jsm/utils/WorkerPool.js \
         examples/jsm/libs/basis/basis_transcoder.js examples/jsm/libs/basis/basis_transcoder.wasm \
         examples/jsm/libs/ktx-parse.module.js examples/jsm/libs/zstddec.module.js \
         examples/jsm/environments/RoomEnvironment.js; do
  [ -s "$B/lib/$f" ] || { mkdir -p "$(dirname "$B/lib/$f")"; curl -fsSL -o "$B/lib/$f" "$T3/$f"; }
done
[ -e "$B/node_modules/playwright" ] || ln -sfn "$(npm root -g)/playwright" "$B/node_modules/playwright"

cp trailer.html render.mjs "$B/"
cp "$GATES"/crates/client/fonts/RobotoCondensed-{Bold,Regular}.ttf "$B/fonts/"
for f in spacegrotesk jbmono; do
  if [ -s "$FORGE/watchtower/fonts/$f.woff2" ]; then cp "$FORGE/watchtower/fonts/$f.woff2" "$B/fonts/"; fi
done

# the game's models: the guns, the raid kit, the farm, the Realm drop
M="$GATES/assets/models"
for m in held/assault_rifle held/semi_automatic_rifle held/bolt_action_rifle held/thompson held/custom_smg \
         held/python_revolver held/semi_automatic_pistol held/pump_shotgun held/rocket_launcher \
         held/timed_explosive_charge held/rocket held/bandage deploy/planter_box deploy/vending \
         crops/corn_plant crops/potato_plant crops/pumpkin_vine site/slot_machine \
         held/skins/verdigris_rifle held/skins/verdigris_python held/skins/verdigris_longsword held/skins/verdigris_hatchet \
         held/skins/lapis_thompson held/skins/lapis_bolt_rifle held/skins/lapis_pickaxe held/skins/lapis_mace \
         held/skins/tidewrack_shotgun held/skins/tidewrack_smg held/skins/tidewrack_sar held/skins/tidewrack_cleaver; do
  out="$B/glb/$(basename "$m").glb"
  [ "$out" -nt "$M/$m.glb" ] || python3 patch_glb.py "$M/$m.glb" "$out"
done
[ -e "$GATES/assets/models/held/revolver.glb" ] && python3 patch_glb.py "$M/held/revolver.glb" "$B/glb/revolver.glb" || true
cp "$GATES"/assets/icons/*.png "$B/icons/"
# the sigil, cut from the wordmark's tile
python3 - "$FORGE/art/gates/gates-wordmark.png" "$B/img/sigil.png" <<'PY' || true
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
im.crop((416, 562, 642, 788)).save(sys.argv[2])
PY
# store stills: the fallback for any footage not shot yet, and some backdrops
for s in raiders raid-satchel heli island-desert island-morning fel-wolf gate-aerial gate-above gate-day gate-inside \
         wolves town-street-day market-street storm verdigris boar; do
  [ -s "$FORGE/art/gates/play-$s.jpg" ] && cp "$FORGE/art/gates/play-$s.jpg" "$B/img/" || true
done

# footage: every NN-name.mp4 a shot list wrote, as JPEG frames at 1920x804
python3 - "$SHOTS" "$B/clips" <<'PY'
import glob, json, os, re, subprocess, sys
src, dst = sys.argv[1], sys.argv[2]
index = {}
for mp4 in sorted(glob.glob(os.path.join(src, "*.mp4"))):
    name = re.sub(r"^\d+-", "", os.path.basename(mp4)[:-4])
    out = os.path.join(dst, name)
    stamp = os.path.join(out, ".src")
    if not (os.path.exists(stamp) and open(stamp).read() == str(os.path.getmtime(mp4))):
        os.makedirs(out, exist_ok=True)
        for f in glob.glob(os.path.join(out, "*.jpg")):
            os.remove(f)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", mp4, "-vf",
                        "scale=1920:-2:flags=lanczos,crop=1920:804", "-q:v", "2",
                        os.path.join(out, "%03d.jpg")], check=True)
        open(stamp, "w").write(str(os.path.getmtime(mp4)))
    index[name] = len(glob.glob(os.path.join(out, "*.jpg")))
json.dump(index, open(os.path.join(dst, "index.json"), "w"))
print("trailer: footage", index or "(none yet: store stills stand in)")
PY

# what the mix plays: the score, the game's own recordings, the trailer hits
cp "$GATES/assets/music/green_veil.ogg" "$B/music/"
for s in pd/shot_rifle_0 pd/shot_rifle_2 pd/shot_bolt_0 pd/shot_smg_0 pd/shot_smg_2 pd/shot_pistol_0 pd/shot_pistol_2 \
         pd/shot_pistol_3 pd/shot_shotgun_0 pd/shot_rocket_0 pd/reload_rifle_0 pd/impact_metal_2 pd/place_stone_1 \
         pd/place_metal_0 pd/headshot_0 freesound/sentry_lock_0 freesound/blast_0 freesound/blast_1 freesound/splash_0 \
         freesound/splash_2 freesound/bandage_0 freesound/flesh_1 freesound/bush_pick_0 freesound/gate_chime_0 \
         freesound/collapse_0 kenney/trade_0 kenney/ui_click_0 kenney/equip_1 kenney/impactPlank_medium_002; do
  cp "$GATES/assets/sound/$s.ogg" "$B/snd/$(basename "$s").ogg"
done
[ -s "$B/sfx/trailer_boom.wav" ] || python3 "$GATES/ci/film_sfx.py" "$B/sfx" >/dev/null

PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$PORT" --bind 127.0.0.1 -d "$B" >/dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null' EXIT
export TRAILER_URL="http://127.0.0.1:$PORT/trailer.html" TRAILER_Q=""
sleep 0.5

if [ "$MODE" = stills ]; then
  node "$B/render.mjs" stills "$B/stills" "$STILLS"
  echo "trailer: frames in $B/stills"; exit 0
fi

node "$B/render.mjs" cues "$B/cues.json"
python3 mix.py "$B" "$B/cues.json" "$B/mix.wav"
LN=$(ffmpeg -hide_banner -nostats -i "$B/mix.wav" -af loudnorm=I=-14:TP=-1.5:LRA=11:print_format=json -f null - 2>&1 \
  | python3 -c "import sys, json; t = sys.stdin.read(); j = json.loads(t[t.rindex('{'):]); print(':'.join(f'{k}={j[v]}' for k, v in (('measured_I', 'input_i'), ('measured_TP', 'input_tp'), ('measured_LRA', 'input_lra'), ('measured_thresh', 'input_thresh'), ('offset', 'target_offset'))))")
AF="loudnorm=I=-14:TP=-1.5:LRA=11:$LN:linear=true"

if [ "$MODE" = preview ]; then
  TRAILER_STEP=4 node "$B/render.mjs" video "$B/preview.mkv"
  ffmpeg -v error -y -i "$B/preview.mkv" -i "$B/mix.wav" -map 0:v -map 1:a -af "$AF" -ar 48000 \
    -vf scale=1280:720 -r 30 -c:v libx264 -preset veryfast -crf 26 -pix_fmt yuv420p -c:a aac -b:a 160k -shortest \
    "$HERE/out/full-auto-preview.mp4"
  ls -la "$HERE"/out/; exit 0
fi

node "$B/render.mjs" video "$B/silent.mkv"
ffmpeg -v error -y -i "$B/silent.mkv" -i "$B/mix.wav" -map 0:v -map 1:a -af "$AF" -ar 48000 \
  -c:v libx264 -preset slow -crf 19 -maxrate 8M -bufsize 16M -pix_fmt yuv420p -c:a aac -b:a 192k -movflags +faststart -shortest \
  "$HERE/out/full-auto.mp4"
ffmpeg -v error -y -i "$HERE/out/full-auto.mp4" -vf scale=1280:720:flags=lanczos -c:v libx264 -preset slow -crf 23 -maxrate 3M -bufsize 6M \
  -pix_fmt yuv420p -c:a copy -movflags +faststart "$HERE/out/full-auto-720p.mp4"
ls -la "$HERE"/out/
