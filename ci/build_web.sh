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

echo "== building client-web for wasm32-unknown-unknown"
cargo build -p client-web --release --target wasm32-unknown-unknown

echo "== generating the JS glue into $out"
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript \
  --out-dir "$out" \
  target/wasm32-unknown-unknown/release/client_web.wasm
cp crates/client-web/web/index.html "$out/"

# The size, printed rather than asserted. `findings/web-build-20260909.md` §4.2
# is the standing warning that a web build does not beat a depot on bytes — it
# beats it on ceremony — and this number is the headless floor, with no
# renderer and no assets in it.
raw=$(stat -c%s "$out/client_web_bg.wasm")
gz=$(gzip -9 -c "$out/client_web_bg.wasm" | wc -c)
printf "   client_web_bg.wasm  %d bytes raw, %d gzipped\n" "$raw" "$gz"
echo "== done: python3 -m http.server 8080 --directory $out"
