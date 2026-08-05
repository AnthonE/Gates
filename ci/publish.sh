#!/usr/bin/env bash
# Publish EVERYTHING in this repo to GitHub. Nothing left on the box.
#
# Written 2026-08-05 because the desktop client (`crates/client`) existed only
# on this machine while a remote branch NAMED `claude/rust-desktop-build-*`
# held two docs commits and no code — the exact "random code in a random place
# never to be seen" failure this script exists to end.
#
# This is an OPERATOR ACT (CLAUDE.md): publishing is never done by the loop.
# The `pre-push` hook the runner installs blocks pushes, so every push here
# passes --no-verify deliberately.
#
#   ./ci/publish.sh          # show what would be pushed, push nothing
#   ./ci/publish.sh --go     # actually push
#
# CREDENTIALS. This box has an SSH key at ~/.ssh/id_ed25519 that GitHub does
# not accept, no gh CLI, no credential helper and no token. Fix ONE of:
#   * add ~/.ssh/id_ed25519.pub to github.com/AnthonE/Gates -> Settings ->
#     Deploy keys, WITH write access, then: git remote set-url origin
#     git@github.com:AnthonE/Gates.git
#   * or a PAT: git config credential.helper store, then push once by hand
set -uo pipefail
cd "$(dirname "$0")/.."

GO=0; [ "${1:-}" = "--go" ] && GO=1
run() { if [ "$GO" = 1 ]; then echo "+ $*"; "$@"; else echo "  would run: $*"; fi; }

echo "== remote =="; git remote get-url origin

# 1. The trunk. Everything merged lives inside this history, so this one push
#    is the bulk of the job -- including crates/client, which the remote has
#    never had a single file of.
echo
echo "== 1. main =="
ahead=$(git rev-list --count origin/main..main 2>/dev/null || echo "?")
echo "  $ahead commit(s) ahead of origin/main"
if git merge-base --is-ancestor origin/main main 2>/dev/null; then
  echo "  fast-forward: yes (no force needed, nothing on the remote is discarded)"
else
  echo "  !! NOT a fast-forward. Stop. Merge origin/main first; never force here."
  exit 1
fi
run git push --no-verify origin main

# 2. Branches whose work is NOT inside main. These are the ones that would be
#    genuinely lost if this box died. Everything else under loop/* and lane/*
#    is already contained in main's history and is not pushed -- that is not
#    leaving it behind, it is declining to publish a duplicate.
echo
echo "== 2. branches holding work that is NOT in main =="
UNIQUE=""
for r in $(git for-each-ref --format='%(refname:short)' refs/heads); do
  [ "$r" = "main" ] && continue
  git merge-base --is-ancestor "$r" main 2>/dev/null && continue
  # unique COMMITS is not enough; require unique CONTENT
  [ -z "$(git diff --stat main..."$r" 2>/dev/null)" ] && continue
  UNIQUE="$UNIQUE $r"
  echo "  $r -- $(git diff --shortstat main..."$r" 2>/dev/null | sed 's/^ *//')"
done
[ -n "$UNIQUE" ] && run git push --no-verify origin $UNIQUE || echo "  (none)"

# 3. Tags. salvage/ranged-v0 is the one that matters: judged FAIL, never
#    rebuilt, and the only copy of ~1969 lines of ranged-weapon sim work.
echo
echo "== 3. tags =="
run git push --no-verify --tags origin

echo
if [ "$GO" = 1 ]; then
  echo "== done. verify: =="
  echo "  git ls-remote --heads origin | wc -l"
  echo "  git ls-tree -r --name-only origin/main -- crates/client   # must be non-empty"
else
  echo "== dry run. re-run with --go to publish. =="
fi
