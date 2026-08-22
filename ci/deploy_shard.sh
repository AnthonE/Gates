#!/usr/bin/env bash
# Deploy the public Gates shard. Build, stage, restart, verify.
#
#   ./ci/deploy_shard.sh          # show what would happen, change nothing
#   ./ci/deploy_shard.sh --go     # actually deploy
#
# **This is an OPERATOR ACT** (`CLAUDE.md`: "deploying to the public shard").
# It is a dry run by default for the same reason `ci/publish.sh` is — the
# loop may propose a deploy and may not perform one — and the two scripts
# are deliberately the same shape so neither surprises you.
#
# ## What a deploy actually moves
#
# Three trees, and they are separate on purpose:
#
#   this repo            what is BUILT — rebuilt constantly, by three lanes
#   /home/master/gates-deploy   what is RUNNING — binary, content/, config
#   /home/master/gates-data     what players OWN — players.save, island.world
#
# `gates-shard.service` has the long version. The short one: the running
# shard must not depend on which branch a worktree is on, and `content_dir`
# is resolved against the CWD while the content hash is pinned into the save
# files — so a live shard pointed at the live tree refuses to boot the first
# time somebody edits `content/*.toml`.
#
# ## The two failures this checks for, because both are silent
#
# **The content hash.** Staging new `content/` under an existing
# `island.world` makes the next boot refuse (wall 7, and the refusal is
# correct). This script stages content and restarts in one step so you find
# out in the ten seconds you are watching, not at 3 a.m. on a crash restart.
#
# **The certs.** The shard reads the PEMs once, at boot. A certbot renewal
# is invisible until something restarts the shard, so a deploy is also how a
# renewal takes effect — and an expired chain refuses every joiner with a TLS
# error that looks like a client bug.
set -uo pipefail
cd "$(dirname "$0")/.."

DEPLOY=/home/master/gates-deploy
DATA=/home/master/gates-data
UNIT=gates-shard.service
CFG=shard-public.toml
HOOK=/etc/letsencrypt/renewal-hooks/deploy/copy-gates-cert.sh

GO=0; [ "${1:-}" = "--go" ] && GO=1
run() { if [ "$GO" = 1 ]; then echo "+ $*"; "$@" || exit 1; else echo "  would run: $*"; fi; }

echo "== 1. build =="
if [ "$GO" = 1 ]; then
  cargo build --release -p server --bin shard || exit 1
else
  echo "  would run: cargo build --release -p server --bin shard"
fi
BIN=target/release/shard
[ "$GO" = 1 ] && { [ -x "$BIN" ] || { echo "  !! no $BIN after the build"; exit 1; }; }

echo
echo "== 2. stage =="
run mkdir -p "$DEPLOY" "$DATA"
run cp -f "$BIN" "$DEPLOY/shard.new"
run mv -f "$DEPLOY/shard.new" "$DEPLOY/shard"   # atomic; a running shard keeps its inode
run rsync -a --delete content/ "$DEPLOY/content/"
run cp -f "$CFG" "$DEPLOY/$CFG"

echo
echo "== 3. the unit =="
if ! sudo -n cmp -s "ops/$UNIT" "/etc/systemd/system/$UNIT" 2>/dev/null; then
  echo "  unit differs from the installed copy (or is not installed)"
  run sudo cp -f "ops/$UNIT" "/etc/systemd/system/$UNIT"
  run sudo systemctl daemon-reload
else
  echo "  unchanged"
fi

echo
echo "== 3b. the certbot deploy hook =="
# The shard reads its PEMs once, at boot. Without this hook a renewal is
# invisible to it and the chain expires under a running shard — see
# ops/certbot-deploy-hook.sh for why that failure is disguised as a client bug.
if ! sudo -n cmp -s ops/certbot-deploy-hook.sh "$HOOK" 2>/dev/null; then
  echo "  hook differs from the installed copy (or is not installed)"
  run sudo cp -f ops/certbot-deploy-hook.sh "$HOOK"
  run sudo chmod +x "$HOOK"
else
  echo "  unchanged"
fi

echo
echo "== 4. restart =="
# `restart` and not `stop; start`: systemd sends SIGTERM and WAITS for the
# flush (TimeoutStopSec=120) before it starts the new one, which is the whole
# reason the unit sets KillSignal.
run sudo systemctl enable --now "$UNIT"
run sudo systemctl restart "$UNIT"

echo
echo "== 5. verify =="
if [ "$GO" = 1 ]; then
  ok=0
  for _ in $(seq 1 30); do
    # Observable state, never elapsed milliseconds (CLAUDE.md: a gate that
    # waits on a clock is not a gate on this box).
    if systemctl is-active --quiet "$UNIT" \
       && ss -ulnp 2>/dev/null | grep -q ':61234' \
       && curl -fsS -m 3 http://127.0.0.1:8431/status.json >/dev/null 2>&1; then
      ok=1; break
    fi
    sleep 1
  done
  if [ "$ok" = 1 ]; then
    echo "  active, bound on 61234/udp, status answering:"
    curl -fsS -m 3 http://127.0.0.1:8431/status.json; echo
    echo "  public: $(curl -fsS -m 5 https://game.moreright.xyz/gates/status.json 2>&1 || echo 'NOT REACHABLE — is the nginx location installed?')"
  else
    echo "  !! did not come up. journalctl -u $UNIT -n 50 --no-pager"
    sudo -n journalctl -u "$UNIT" -n 30 --no-pager
    exit 1
  fi
else
  echo "  would poll: systemctl is-active, 61234/udp bound, /status.json answering"
fi

echo
if [ "$GO" = 1 ]; then
  echo "== deployed. =="
  echo "  journalctl -u $UNIT -f      # the 10-second stats line"
else
  echo "== dry run. re-run with --go to deploy. =="
fi
