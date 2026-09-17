#!/usr/bin/env bash
# Build the browser client into a directory a static server can serve.
#
#   ./ci/build_web.sh [outdir]        # default: target/webdist
#
# Then serve it and open it. `http://localhost` is a secure context, which is
# what WebTransport requires of the PAGE; the shard it dials is `https://`
# either way.
#
#   python3 -m http.server 8080 --directory target/webdist
#
# **Not a gate and it never will be.** It needs `wasm-bindgen`, and what it
# produces has to be looked at in a browser — `CLAUDE.md` is explicit that no
# gate in this repo starts a browser and none should. What CI checks is that
# the crate COMPILES for the target (`ci/gates.sh`, the wasm gate); a person
# opening the page is the rest, which is the same division the native client
# has had since the visual gate was retired.
set -euo pipefail
cd "$(dirname "$0")/.."

# **Not `target/web`, and this is not cosmetic.** `[profile.web]` in the root
# `Cargo.toml` makes `target/web/` cargo's own host-artifact directory for that
# profile, so `ci/gates.sh`'s `cargo build -p client-web --profile web` drops
# `deps/`, `build/`, `examples/` and `incremental/` straight into it — AFTER
# this script has staged and left. Measured 2026-09-12: a page staged at 85 MB
# was 222 MB and 743 files once the gates had run, and the publish that nearly
# went out carried .rlib, .rmeta, .so and the absolute build paths inside the
# .d files onto a public web root. The staging directory and a profile
# directory must not be the same path.
out="${1:-target/webdist}"

# A named outdir can re-make the same mistake, so ask rather than type: the
# profile list comes out of Cargo.toml, and `debug`/`release` are cargo's own
# two whether or not they are written there.
for p in $(sed -n 's/^\[profile\.\([A-Za-z0-9_-]*\)\]$/\1/p' Cargo.toml) debug release; do
  [ "${out%/}" = "target/$p" ] && {
    echo "refusing to stage into target/$p — cargo owns that path for profile" >&2
    echo "'$p', and its artifacts would land in the page after this script exits." >&2
    exit 1
  }
done

# **The CLI and the crate must be the same version.** wasm-bindgen generates
# glue against an ABI it also emits into the module, and a mismatch produces a
# module that loads and then fails on a call — so it is checked here rather
# than discovered there. One `Cargo.lock` for the workspace is what makes the
# crate side of this a single answer (see `crates/client-web/Cargo.toml`).
command -v wasm-bindgen >/dev/null || {
  echo "wasm-bindgen is not installed. cargo install wasm-bindgen-cli --version <the one in Cargo.lock>" >&2
  exit 1
}
want="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)"
have="$(wasm-bindgen --version | awk '{print $2}')"
[ "$want" = "$have" ] || {
  echo "wasm-bindgen CLI is $have and Cargo.lock resolves $want." >&2
  echo "They generate and consume the same ABI, so they have to agree:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want" >&2
  exit 1
}

# **The `web` profile, not `--release`** (root `Cargo.toml`): a page is
# downloaded and then compiled by the browser, so bytes are a cost the native
# release never pays and its profile is tuned for the wrong thing. The size
# line at the end is the measurement; `findings/web-build-20260909.md` §16
# records what each profile knob bought.
# `WEB_PROFILE=release` keeps the function names for the day a browser stack
# is the thing being read (findings §16); the default is what ships.
profile="${WEB_PROFILE:-web}"
echo "== building client-web for wasm32-unknown-unknown (profile: $profile)"
cargo build -p client-web --profile "$profile" --target wasm32-unknown-unknown

echo "== generating the JS glue into $out"
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript \
  --remove-producers-section \
  --out-dir "$out" \
  "target/wasm32-unknown-unknown/$profile/client_web.wasm"
cp crates/client-web/web/index.html crates/client-web/web/app.js "$out/"

# ── the page's own face ─────────────────────────────────────────────────────
# Roboto Condensed is what `crates/client/src/render/ui.rs` sets the whole game
# in, and the page named it in a CSS font stack for weeks while shipping no
# font file — so every visitor read the launcher in system-ui while the game
# behind it was condensed. These are that same Apache-2.0 face out of
# `crates/client/fonts/`, subset to Latin-1 plus the marks the page sets and
# converted to woff2 (~12 KB each against the 113 KB TTF). Regenerate them the
# same way if the page ever needs a glyph it has not got:
#   pip install fonttools brotli
#   pyftsubset crates/client/fonts/RobotoCondensed-Bold.ttf --flavor=woff2 \
#     --unicodes='U+0020-007E,U+00A0-00FF,…' --layout-features=kern,liga,calt,tnum \
#     --output-file=crates/client-web/web/fonts/robotocondensed-700.woff2
# The licence travels with them, which is the whole of what Apache-2.0 asks.
mkdir -p "$out/fonts"
cp crates/client-web/web/fonts/robotocondensed-400.woff2 \
   crates/client-web/web/fonts/robotocondensed-700.woff2 "$out/fonts/"
cp crates/client/fonts/LICENSE-ROBOTO.txt "$out/fonts/"

# ── the audio thread's module ───────────────────────────────────────────────
# The game's sound is rendered in Rust inside an `AudioWorkletProcessor`, which
# runs in a scope with no DOM, no `fetch` and no shared memory with the page —
# so it cannot be a corner of `client_web.wasm` (that module is Bevy and the
# transport, and none of it belongs in a callback with 2.7 ms to fill 128
# frames). `crates/sound-worklet` is a second, small cdylib over `sound`.
#
# **`--target no-modules`, and that is not a preference.** The glue it emits
# defines one global and contains no `import`, so the processor is a single
# script — an `AudioWorkletGlobalScope` has no module graph worth relying on,
# and static `import` inside a worklet has been uneven across browsers for
# years. The concatenation below is what `addModule` loads; the `.wasm` beside
# it is fetched and compiled by the PAGE and posted down the port, because a
# worklet cannot fetch anything itself.
echo "== building sound-worklet for the AudioWorklet (profile: $profile)"
cargo build -p sound-worklet --profile "$profile" --target wasm32-unknown-unknown
wk="$(mktemp -d)"
trap 'rm -rf "$wk"' EXIT
wasm-bindgen --target no-modules --no-typescript \
  --remove-producers-section \
  --out-dir "$wk" \
  "target/wasm32-unknown-unknown/$profile/sound_worklet.wasm"
cp "$wk/sound_worklet_bg.wasm" "$out/"
cat "$wk/sound_worklet.js" crates/client-web/web/audio-processor.js > "$out/audio.js"
# The glue has to have defined the global the processor calls, or the
# concatenation is two halves that load and then do nothing — silence with a
# 200 on every request, which is this page's recurring failure shape.
grep -q "registerProcessor" "$out/audio.js" || {
  echo "audio.js lost the processor registration" >&2; exit 1; }
grep -q "wasm_bindgen" "$out/audio.js" || {
  echo "audio.js has no wasm_bindgen global - wrong --target?" >&2; exit 1; }
printf "   audio.js %d bytes, sound_worklet_bg.wasm %d bytes\n" \
  "$(stat -c%s "$out/audio.js")" "$(stat -c%s "$out/sound_worklet_bg.wasm")"

# **Play the module before shipping it, where there is a node to do it with.**
# Not a browser and not a gate (`ci/gates.sh` has never needed node), but this
# is the one check that crosses the wasm ABI for real: it replays a
# Rust-authored fixture through the staged `audio.js` and compares every
# sample against what a native `Renderer` produced. Everything it catches —
# the glue not loading outside a DOM, `initSync` taking another shape,
# `out_ptr` at the wrong array, a detached view after the bank grows memory —
# is SILENCE with every Rust gate green, which is how the browser lost its
# sound for two days in the first place. A box with no node says so and ships;
# a box with one and a failing check does not ship.
if command -v node >/dev/null; then
  echo "== checking the audio module (fixture replay, no browser)"
  fxdir="$(mktemp -d)"; trap 'rm -rf "$wk" "$fxdir"' EXIT
  cargo run -q -p sound --example worklet_fixture -- "$fxdir/fixture.bin" \
    || { echo "could not build the worklet fixture" >&2; exit 1; }
  node ci/check_worklet.mjs "$out/audio.js" "$out/sound_worklet_bg.wasm" "$fxdir/fixture.bin" \
    || { echo "the staged audio module does not render what this tree renders" >&2; exit 1; }
else
  echo "== audio module check: no node, shipping unplayed (node ci/check_worklet.mjs …)"
fi

# **The assets, staged from `git ls-files` and never from a walk.**
#
# Bevy's wasm asset reader turns `AssetPlugin::file_path` ("assets") into a
# page-relative URL, so every `asset_server.load("textures/…")` is a fetch for
# `/assets/textures/…`. Until this existed the output directory held the page
# and the module and nothing else, so that was 100% of asset requests 404ing —
# not a hosting question for later, a missing step here.
#
# ⚠ **`cp -r assets "$out/"` is the bug this repo has already paid for once.**
# `ci/depot.py` staged with `shutil.copytree` and produced a **1.6 GB** depot
# against a 124 MB one, having swept in `assets/textures/candidates/` — 1.3 GB
# of raw sourcing downloads that `.gitignore` deliberately keeps out of the
# tree, and unvetted by construction, so it was a route around the licence
# rail as well as a 13× download. Every gate was green, because an untracked
# file is invisible to all of them. `git ls-files` makes `.gitignore` the one
# author of what ships and refuses to fall back to a walk.
echo "== staging assets/ (tracked files only)"
command -v git >/dev/null || { echo "git missing - refusing to stage by walking" >&2; exit 1; }
count=0
while IFS= read -r -d '' f; do
  mkdir -p "$out/$(dirname "$f")"
  cp "$f" "$out/$f"
  count=$((count + 1))
done < <(git ls-files -z assets/)
[ "$count" -gt 0 ] || { echo "git ls-files staged nothing from assets/ - refusing" >&2; exit 1; }
printf "   %d tracked asset files, %s\n" "$count" "$(du -sh "$out/assets" | cut -f1)"

# **The web asset variant: the staged models' KTX2 maps become PNG.** The
# UASTC transcoder is C++ and is not in the browser build, so every model's
# glTF load failed there as `format requires transcoding` — 47 of them, and
# the island had no rocks, nodes, sites or held items. The conversion runs
# where the transcoder exists (natively, here) over the STAGED copy, in
# place, so the tree's files and every gate over them are untouched and the
# client needs no path seam. `crates/client/src/webassets.rs` is the whole
# argument, including why level 1 (512²) and not level 0.
echo "== converting the staged models for the browser (KTX2 -> PNG)"
cargo run -q -p client --features webassets --bin web_assets -- "$out/assets/models"

# **`wasm-opt`, when the box has one.** binaryen is what actually shrinks a
# Bevy module (findings §16 — the profile knobs bought four percent, the
# names section was the rest). Not required: a box without it ships the
# bindgen output, and says so, rather than failing a build over a tool the
# page does not need to run. `WASM_OPT=/path/to/wasm-opt` names one that is
# not on `$PATH` — the `binaryen` npm package ships one as
# `node_modules/binaryen/bin/wasm-opt`.
opt="${WASM_OPT:-$(command -v wasm-opt || true)}"
if [ -n "$opt" ] && [ -x "$opt" ]; then
  echo "== wasm-opt -Os ($opt)"
  for m in client_web_bg sound_worklet_bg; do
    before=$(stat -c%s "$out/$m.wasm")
    "$opt" -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
      --enable-mutable-globals --enable-sign-ext --enable-reference-types \
      -o "$out/$m.wasm.opt" "$out/$m.wasm"
    mv "$out/$m.wasm.opt" "$out/$m.wasm"
    printf "   wasm-opt: %s %d -> %d bytes\n" "$m" "$before" "$(stat -c%s "$out/$m.wasm")"
  done
else
  echo "== wasm-opt: not installed, shipping the bindgen output (npm i binaryen, or WASM_OPT=…)"
fi

# The size, printed rather than asserted. `findings/web-build-20260909.md` §4.2
# is the standing warning that a web build does not beat a depot on bytes — it
# beats it on ceremony — and this number is the headless floor, with no
# renderer and no assets in it.
raw=$(stat -c%s "$out/client_web_bg.wasm")
# Written beside the module rather than measured and thrown away: the
# origin's `gzip_static` (`scry-forge/deploy/nginx/elopros.com.conf`, the
# `/games/gates/` block) serves this file to a browser that accepts gzip —
# with the Content-Length a streaming compressor cannot send, and no CPU
# spent per request on a 34 MB file. `-n` leaves the timestamp and name out
# of the header, so an unchanged module gzips to identical bytes and
# `publish_web.sh`'s `--link-dest` hard-links it across publishes like every
# other unchanged file. Without it the origin falls back to compressing on
# the fly; without either, measured 2026-09-12, the module went over the
# wire raw.
gzip -9 -n -c "$out/client_web_bg.wasm" > "$out/client_web_bg.wasm.gz"
gzip -9 -n -c "$out/sound_worklet_bg.wasm" > "$out/sound_worklet_bg.wasm.gz"
gz=$(stat -c%s "$out/client_web_bg.wasm.gz")
printf "   client_web_bg.wasm  %d bytes raw, %d gzipped (.wasm.gz beside it)\n" "$raw" "$gz"
# ── what the page needs to draw an honest progress bar ──────────────────────
# **`content-length` is not the denominator**, and a page that believes it is
# draws a bar that reaches 400%. The origin serves the `.gz` written just above
# (`gzip_static`), so the header a browser sees is the COMPRESSED length while
# the body it reads is DECODED — and nothing at runtime can recover the decoded
# size from the response. It is known HERE, after wasm-opt has had its say, so
# the build writes it down. `crates/client-web/web/app.js` reads this file
# first and falls back to counting megabytes with no bar when it is absent,
# which is what an older publish looks like.
#
# The stamp beside it is the only way a person looking at a page can say which
# publish they have — `publish_web.sh` lands builds as `<root>/<build>/` and
# flips `current`, so the directory name is on the server and not in the page.
cat > "$out/build.json" <<JSON
{
  "wasm_bytes": $raw,
  "wasm_gz_bytes": $gz,
  "profile": "$profile",
  "proto": $(sed -n 's/^pub const PROTO_VER: u16 = \([0-9]*\);.*/\1/p' crates/protocol/src/lib.rs | head -1),
  "built": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
printf "   build.json: %s decoded, %s gzipped\n" "$raw" "$gz"

echo "== done: python3 -m http.server 8080 --directory $out"
