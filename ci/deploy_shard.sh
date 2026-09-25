#!/usr/bin/env bash
# Deploy the public Gates shard. Build, stage, restart, verify.
#
#   ./ci/deploy_shard.sh          # show what would happen, change nothing
#   ./ci/deploy_shard.sh --go     # actually deploy
#   ./ci/deploy_shard.sh --go --strand-clients   # deploy a wire published clients cannot speak
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

GO=0; STRAND=0
for a in "$@"; do
  case "$a" in --go) GO=1 ;; --strand-clients) STRAND=1 ;; esac
done
run() { if [ "$GO" = 1 ]; then echo "+ $*"; "$@" || exit 1; else echo "  would run: $*"; fi; }

echo "== 0. who this wire strands =="
# ⚠ **A deploy that moves PROTO_VER strands every client already published on
# the old number, and this script's own verify cannot see it** — the shard
# comes up, binds, answers status.json and proves its cert, and every player
# who installed last week is REFUSE_VERSION. That is 2026-09-12: the shard
# went to 62 for the browser client and both live desktop depots were 61. So
# read what players actually run — the depot pointers and the page's
# `current` on the origin — ask each build's commit for its PROTO_VER, and
# refuse to strand anybody unless told to.
want=$(grep -oP 'pub const PROTO_VER: u16 = \K[0-9]+' crates/protocol/src/lib.rs)
report=$(python3 - "$want" <<'PY'
import json, re, subprocess, sys
want = sys.argv[1]
try:
    out = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "morr",
         "cat /data/apps/scry-data/depots/gates/published.json; echo; "
         "readlink /data/apps/scry-data/gates-web/current"],
        capture_output=True, text=True, timeout=30)
except subprocess.TimeoutExpired:
    print("UNKNOWN morr did not answer in 30 s"); sys.exit(0)
if out.returncode != 0:
    print("UNKNOWN could not read the published clients from morr: " + out.stderr.strip()[:200]); sys.exit(0)
lines = out.stdout.splitlines()
try:
    clients = [("depot " + p, b) for p, b in json.loads(lines[0]).items()]
except (IndexError, json.JSONDecodeError):
    print("UNKNOWN published.json did not parse"); sys.exit(0)
if len(lines) > 1 and lines[1].strip():
    clients.append(("web page", lines[1].strip()))
for name, build in clients:
    m = re.search(r"-g([0-9a-f]{7,40})", build)
    src = subprocess.run(["git", "show", m.group(1) + ":crates/protocol/src/lib.rs"],
                         capture_output=True, text=True) if m else None
    p = re.search(r"pub const PROTO_VER: u16 = (\d+)", src.stdout) if src and src.returncode == 0 else None
    got = p.group(1) if p else None
    if got is None:
        print(f"UNKNOWN {name} {build}: its commit is not in this clone (git fetch)")
    elif got != want:
        print(f"STRANDED {name} {build} speaks proto {got}")
    else:
        print(f"ok {name} {build} speaks proto {got}")
PY
)
printf '%s\n' "$report" | sed 's/^/  /'
if printf '%s\n' "$report" | grep -qE '^(STRANDED|UNKNOWN)'; then
  if [ "$STRAND" = 1 ]; then
    echo "  !! deploying proto $want over those anyway (--strand-clients) — republish them next"
  else
    echo "  !! proto $want would refuse the clients above at the handshake. Deploy anyway"
    echo "     only on purpose, and republish them after: re-run with --strand-clients."
    exit 1
  fi
fi

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
    echo "  public: $(curl -fsS -m 5 https://game.elopros.com/gates/status.json 2>&1 || echo 'NOT REACHABLE — is the nginx location installed?')"
    # The name the shard can PROVE, which is a different question from the
    # name nginx answers on 443 above — they are two certificates and only
    # this one is in the QUIC handshake. A shard serving a chain for a name
    # nobody dials is dark with every check on this page green (2026-08-20
    # to 08-23; ops/certbot-deploy-hook.sh carries the story).
    NAME=$(grep -oP '^domain\s*=\s*"\K[^"]+' "$CFG")
    # The verdict is on stdout and the exit code is 0 either way — see
    # ops/certbot-deploy-hook.sh, where writing this the obvious way made
    # the check accept anything.
    if openssl x509 -noout -checkhost "$NAME" \
         -in /home/master/gates-certs/fullchain.pem 2>/dev/null \
         | grep -q "does match certificate"; then
      echo "  cert:   covers $NAME"
    else
      # Found live 2026-09-25: the shard had served game.moreright.xyz's
      # chain for two days. The hook picks the lineage that covers $NAME and
      # restarts the shard, so run it rather than print it.
      echo "  cert:   !! /home/master/gates-certs/fullchain.pem does NOT cover $NAME — running the deploy hook"
      sudo "$HOOK" || exit 1
      if openssl x509 -noout -checkhost "$NAME" \
           -in /home/master/gates-certs/fullchain.pem 2>/dev/null \
           | grep -q "does match certificate"; then
        echo "  cert:   covers $NAME now"
      else
        echo "  cert:   !! still does not cover $NAME; every joiner will fail the handshake"
        exit 1
      fi
    fi
  else
    echo "  !! did not come up. journalctl -u $UNIT -n 50 --no-pager"
    sudo -n journalctl -u "$UNIT" -n 30 --no-pager
    exit 1
  fi
else
  echo "  would poll: systemctl is-active, 61234/udp bound, /status.json answering"
fi

echo
echo "== 6. the agents =="
# The Jev services speak the wire too, so a deploy that moves PROTO_VER
# strands them exactly as it strands a published client: gates-jev plays the
# public shard, gates-watch runs its own private one but ships this tree's
# content and renderer. Rebuild both from this commit whenever installed.
AGENT_BIN=/home/master/gates-jev
WATCH=/mnt/hive-data/gates-watch
if systemctl list-unit-files gates-jev.service >/dev/null 2>&1 \
   || systemctl list-unit-files gates-watch.service >/dev/null 2>&1; then
  if [ "$GO" = 1 ]; then
    cargo build --release -p server --features watch --bin jev-bot --bin jev-watch || exit 1
  else
    echo "  would run: cargo build --release -p server --features watch --bin jev-bot --bin jev-watch"
  fi
fi
if systemctl list-unit-files gates-jev.service >/dev/null 2>&1; then
  run cp -f target/release/jev-bot "$AGENT_BIN/jev-bot.new"
  run mv -f "$AGENT_BIN/jev-bot.new" "$AGENT_BIN/jev-bot"
  run sudo systemctl restart gates-jev.service
else
  echo "  gates-jev.service not installed"
fi
if systemctl list-unit-files gates-watch.service >/dev/null 2>&1; then
  REL="$WATCH/releases/$(git rev-parse --short=12 HEAD)"
  run mkdir -p "$REL"
  run cp -f target/release/jev-watch "$REL/jev-watch"
  run rsync -a --delete content/ "$REL/content/"
  # Tracked files only: an untracked asset is not ours to ship.
  if [ "$GO" = 1 ]; then
    git ls-files -z assets | sed -z 's#^assets/##' \
      | rsync -a --from0 --files-from=- assets/ "$WATCH/assets/" || exit 1
  else
    echo "  would sync tracked assets/ into $WATCH/assets/"
  fi
  if [ "$(readlink "$WATCH/current")" != "$REL" ]; then
    run ln -sfn "$(readlink "$WATCH/current")" "$WATCH/previous"
    run ln -sfn "$REL" "$WATCH/current"
  fi
  run sudo systemctl restart gates-watch.service
else
  echo "  gates-watch.service not installed"
fi

echo
if [ "$GO" = 1 ]; then
  echo "== deployed. =="
  echo "  journalctl -u $UNIT -f      # the 10-second stats line"
else
  echo "== dry run. re-run with --go to deploy. =="
fi
