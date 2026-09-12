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
  before=$(stat -c%s "$out/client_web_bg.wasm")
  echo "== wasm-opt -Os ($opt)"
  "$opt" -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-mutable-globals --enable-sign-ext --enable-reference-types \
    -o "$out/client_web_bg.wasm.opt" "$out/client_web_bg.wasm"
  mv "$out/client_web_bg.wasm.opt" "$out/client_web_bg.wasm"
  printf "   wasm-opt: %d -> %d bytes\n" "$before" "$(stat -c%s "$out/client_web_bg.wasm")"
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
gz=$(stat -c%s "$out/client_web_bg.wasm.gz")
printf "   client_web_bg.wasm  %d bytes raw, %d gzipped (.wasm.gz beside it)\n" "$raw" "$gz"
echo "== done: python3 -m http.server 8080 --directory $out"
