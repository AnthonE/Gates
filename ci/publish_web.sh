#!/usr/bin/env bash
# Publish the browser client to the origin: elopros.com/games/gates/.
#
#   ./ci/publish_web.sh                  # build, stage, upload, flip `current`
#   ./ci/publish_web.sh --no-build       # upload what target/webdist already holds
#   ./ci/publish_web.sh --dry-run        # everything but the upload and the flip
#   ./ci/publish_web.sh --host morr      # the origin's ssh alias (default: morr)
#
# **An OPERATOR ACT** (CLAUDE.md §loop discipline: publishing the page or the
# link is never the loop's). The loop can build the page; a person publishes
# it, the same way `ci/publish_depot.py` is a person's act for the depot.
#
# ## The shape, and why it is the depot's
#
# Every publish lands whole in its own directory, `<root>/<build>/`, and the
# page the world sees is `<root>/current`, a symlink flipped LAST — so a
# half-uploaded build is never served, a publish is one atomic rename, and a
# rollback is one `ln -sfn` to the previous directory. `--link-dest` against
# `current` hard-links every file that did not change (92 MB of assets change
# rarely; the 34 MB module changes every time), which is `LAUNCHER.md` §8d's
# rule for the depot applied here. The build id is `ci/depot.py`'s own —
# `<version>-g<sha>` plus its `-dirty` marker — so a directory on the origin
# names the commit it came from, and a dirty tree says so in its name.
#
# What serves it: `scry-forge/deploy/nginx/elopros.com.conf`, `location ^~
# /games/gates/`, whose `alias` is `<root>/current/` and whose CSP is the
# reason the page works there at all ('wasm-unsafe-eval', and the shard in
# `connect-src`, and `gzip_static` for the `client_web_bg.wasm.gz` the build
# writes beside the module — 34 MB raw is 8.6 MB on the wire that way). After
# the first publish the store's play button is ONE
# field away — `play_url` on the Gates listing, set from the dev desk or in
# `watchtower/listings/listings.json` — and it is deliberately not set before
# the files exist, because that field is the promise that the game runs.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="morr"
ROOT="/data/apps/scry-data/gates-web"
BUILD=1
DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --dry-run) DRY=1 ;;
    --host) HOST="$2"; shift ;;
    --root) ROOT="$2"; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
  shift
done

OUT="${OUT:-target/webdist}"

if [ "$BUILD" = 1 ]; then
  ./ci/build_web.sh "$OUT"
fi
for f in index.html app.js client_web.js client_web_bg.wasm; do
  [ -f "$OUT/$f" ] || { echo "$OUT/$f is missing — run ./ci/build_web.sh" >&2; exit 1; }
done

# **Refuse a staged tree with cargo in it.** `build_web.sh` stages from
# `git ls-files` and is careful, but what it leaves can be polluted after it
# exits — `target/web` was both the page and cargo's `[profile.web]` host
# directory until 2026-09-12, so one `ci/gates.sh` run put `deps/`, `build/`
# and `incremental/` inside a tree that was about to be uploaded, with every
# gate green because an artifact nobody tracks is invisible to all of them.
# The directory moved, and this refuses the shape rather than that one path:
# a publish is the last place to find out, so it is the place that checks.
junk="$(find "$OUT" \( -name '*.rlib' -o -name '*.rmeta' -o -name '*.d' \
  -o -name '*.so' -o -name '.fingerprint' -o -name 'incremental' \) -print -quit)"
[ -z "$junk" ] || {
  echo "$OUT holds build artifacts ($junk) — that is cargo's output, not the page." >&2
  echo "Rebuild it clean: ./ci/build_web.sh $OUT" >&2
  exit 1
}

# The same id the depot carries, from the same function, so the two never
# name one commit two ways.
# The module is the "binary" here: on a dirty tree the id carries its content
# hash, so two different builds cannot share one directory name.
ID="$(OUT="$OUT" python3 -c 'import os, sys, pathlib; sys.path.insert(0, "ci"); import depot; print(depot.build_id(pathlib.Path(os.environ["OUT"]) / "client_web_bg.wasm"))')"
DEST="$ROOT/$ID"
echo "== build $ID"
echo "   $(du -sh "$OUT" | cut -f1) staged, $(find "$OUT" -type f | wc -l) files"
echo "== to $HOST:$DEST  (current -> $ID after the upload)"

if [ "$DRY" = 1 ]; then
  echo "dry run: nothing uploaded, nothing flipped"
  exit 0
fi

# rsync creates the last path component and no more; the root is ours to make.
ssh -o BatchMode=yes "$HOST" "mkdir -p '$ROOT'"
# `--link-dest` is relative to the DESTINATION directory: `../current` is the
# build being served now, and every unchanged file becomes a hard link to it.
rsync -a --info=stats1 --link-dest="../current" "$OUT/" "$HOST:$DEST/"
# Flip last, atomically: a new symlink beside the old one, then one rename.
ssh -o BatchMode=yes "$HOST" "ln -sfn '$ID' '$ROOT/current.new' && mv -T '$ROOT/current.new' '$ROOT/current' && ls -l '$ROOT/current'"

echo "== live: https://elopros.com/games/gates/"
echo "   the store's play button is play_url on the Gates listing — set it now if this is the first publish"
