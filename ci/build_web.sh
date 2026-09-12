#!/usr/bin/env bash
# Build the browser client into a directory a static server can serve.
#
#   ./ci/build_web.sh [outdir]        # default: target/web
#
# Then serve it and open it. `http://localhost` is a secure context, which is
# what WebTransport requires of the PAGE; the shard it dials is `https://`
# either way.
#
#   python3 -m http.server 8080 --directory target/web
#
# **Not a gate and it never will be.** It needs `wasm-bindgen`, and what it
# produces has to be looked at in a browser — `CLAUDE.md` is explicit that no
# gate in this repo starts a browser and none should. What CI checks is that
# the crate COMPILES for the target (`ci/gates.sh`, the wasm gate); a person
# opening the page is the rest, which is the same division the native client
# has had since the visual gate was retired.
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:-target/web}"

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
echo "== building client-web for wasm32-unknown-unknown (profile: web)"
cargo build -p client-web --profile web --target wasm32-unknown-unknown

echo "== generating the JS glue into $out"
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript \
  --remove-producers-section \
  --out-dir "$out" \
  target/wasm32-unknown-unknown/web/client_web.wasm
cp crates/client-web/web/index.html "$out/"

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

# The size, printed rather than asserted. `findings/web-build-20260909.md` §4.2
# is the standing warning that a web build does not beat a depot on bytes — it
# beats it on ceremony — and this number is the headless floor, with no
# renderer and no assets in it.
raw=$(stat -c%s "$out/client_web_bg.wasm")
gz=$(gzip -9 -c "$out/client_web_bg.wasm" | wc -c)
printf "   client_web_bg.wasm  %d bytes raw, %d gzipped\n" "$raw" "$gz"
echo "== done: python3 -m http.server 8080 --directory $out"
