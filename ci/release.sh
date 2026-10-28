#!/usr/bin/env bash
# Ship HEAD everywhere a player meets it: the public shard, the browser page,
# both desktop depots, and the store's copy of the repo manifest.
#
#   ./ci/release.sh               # the gates and the plan; changes nothing
#   ./ci/release.sh --go          # ship it
#   ./ci/release.sh --go --wipe   # ship it and end the world (you type to confirm)
#   ./ci/release.sh --go --warn   # ship it after the island's /restart countdown
#
# **An OPERATOR ACT** (CLAUDE.md: deploying the public shard, publishing the
# depot or the web page). A dry run by default, the same shape as
# ci/deploy_shard.sh. The dry run touches the network only for HEAD's gates
# run, which it dispatches and waits on when main's HEAD has none yet. Run it on the shard box: the deploy and --wipe act on this
# machine, the page and the depots go to the origin over ssh.
#
# The order is the point. publish_web.sh and publish_depot.py each refuse a
# client the live shard would refuse, so the shard goes first, and everything
# is built before anything moves, so a failed build strands nobody. A step that
# fails stops the release and prints the commands that finish it by hand.
#
# --wipe stops the shard (which flushes), closes the wipe into the hall the way
# the shard's own wipe does (`shard --close-wipe N`: podiums, last wipe's
# placings, the payout files), then moves the world and the player store into
# `<world_file>.wipes/<unix>/` (the shard's own archive layout,
# crates/server/src/wipe.rs) and counts the wipe in `<world_file>.wipe`. Players
# keep nothing, blueprints included: a wipe the shard runs keeps them by
# rewriting each record, and a release that changes the record cannot. A HEAD
# that cannot read the live save headers refuses to deploy without --wipe,
# because its shard would refuse to boot and restart until systemd gives up.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

UNIT=gates-shard.service
CFG=shard-public.toml
# Where deploy_shard.sh stages the running shard: its config is the one the
# live world was posted under, so the wipe closes it with those prizes.
DEPLOY=/home/master/gates-deploy
STATUS_URL=https://game.elopros.com/gates/status.json
PAGE_URL=https://elopros.com/games/gates/play/build.json
MANIFEST_URL=https://elopros.com/api/launcher/manifest/gates
PULL_URL=https://elopros.com/api/store/repo/pull

GO=0; WIPE=0; WARN=""
for a in "$@"; do
  case "$a" in
    --go) GO=1 ;;
    --wipe) WIPE=1 ;;
    --warn) WARN=" --warn" ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "release: unknown flag $a (it takes --go, --wipe and --warn)" >&2; exit 2 ;;
  esac
done
run() { if [ "$GO" = 1 ]; then echo "+ $*"; "$@"; else echo "  would run: $*"; fi; }

REFUSED=()
refuse() { REFUSED+=("$1"); echo "  !! $1"; }

cfg() { grep -oP "^$1\\s*=\\s*\"?\\K[^\"#\\s]+" "$CFG" | head -1; }
src() { grep -oP "pub const $1: \\w+ = \\K[0-9_]+" "$2" | tr -d _; }

SHA=$(git rev-parse HEAD)
SHORT=$(git rev-parse --short=9 HEAD)
VER=$(grep -m1 -oP '^version = "\K[^"]+' Cargo.toml)
PROTO=$(src PROTO_VER crates/protocol/src/lib.rs)
# ci/depot.py's own id, so this script never names a build a second way.
BUILD=$(python3 -c 'import sys; sys.path.insert(0, "ci"); import depot; print(depot.build_id())')
SAVE=$(cfg save_file)
WORLD=$(cfg world_file)

echo "== 0. HEAD =="
echo "  $SHORT · version $VER · proto $PROTO · build $BUILD"
if [ -n "$(git status --porcelain)" ]; then
  refuse "the tree has uncommitted changes; a release ships a commit (publish_depot.py refuses this too)"
fi
# deploy_shard.sh and ci/depot.py read target/ by path; build_web.sh follows
# CARGO_TARGET_DIR. Built anywhere else, the shard and the depots would ship
# whatever stale binaries target/ holds.
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ "$(realpath -m "$CARGO_TARGET_DIR")" != "$(realpath -m target)" ]; then
  refuse "CARGO_TARGET_DIR is $CARGO_TARGET_DIR; deploy_shard.sh and depot.py read ./target. Unset it"
fi

echo
echo "== 1. HEAD's gates run on main =="
if ! command -v gh >/dev/null 2>&1; then
  refuse "gh is not installed, so nothing can say HEAD is green"
else
  gh_err=$(mktemp); trap 'rm -f "$gh_err"' EXIT
  # Sets $verdict: GREEN/RED/PENDING <url>, NONE, or ERROR <why>.
  gates_verdict() {
    runs=$(gh run list --commit "$SHA" --workflow gates --limit 50 \
             --json databaseId,status,conclusion,event,headBranch,url 2>"$gh_err")
    rc=$?
    if [ "$rc" != 0 ]; then
      verdict="ERROR could not ask GitHub: $(head -c 300 "$gh_err")"
    else
      # A dispatched run on main counts, and so does a push run from before
      # the gates moved to ship time. A red one, or one still running,
      # outranks a green one.
      report=$(python3 - "$runs" <<'PY'
import json, sys
try:
    runs = json.loads(sys.argv[1])
except json.JSONDecodeError as e:
    print(f"ERROR gh answered something that is not JSON: {e}")
    sys.exit()
runs = [r for r in runs
        if r.get("event") in ("push", "workflow_dispatch") and r.get("headBranch") == "main"]
RED = {"failure", "timed_out", "startup_failure", "action_required"}
done = [r for r in runs if r.get("status") == "completed"]
red = [r for r in done if r.get("conclusion") in RED]
live = [r for r in runs if r.get("status") != "completed"]
green = [r for r in done if r.get("conclusion") == "success"]
for r in runs:
    print(f"  {r.get('event'):<17} {r.get('status')}/{r.get('conclusion') or '-'}  {r.get('url')}")
print("RED " + red[0]["url"] if red else "PENDING " + live[0]["url"] if live
      else "GREEN " + green[0]["url"] if green else "NONE")
PY
  )
      printf '%s\n' "$report" | sed '$d'
      verdict=$(printf '%s\n' "$report" | tail -1)
      # HEAD's tree may be one a branch's run already passed (a run dispatched
      # on a branch that was up to date with main): gates.yml's last job names
      # the tree it tested, so that run counts and main's own run of the same
      # files need not be waited for. A red run of HEAD still outranks it.
      case "$verdict" in
        PENDING* | NONE)
          tree=$(git rev-parse 'HEAD^{tree}')
          same=$(gh api "repos/{owner}/{repo}/actions/artifacts?name=gates-tree-$tree" \
                   --jq '.artifacts[].workflow_run.id' 2>/dev/null | while read -r id; do
                   [ "$(gh api "repos/{owner}/{repo}/actions/runs/$id" --jq .conclusion 2>/dev/null)" = success ] \
                     && gh api "repos/{owner}/{repo}/actions/runs/$id" --jq .html_url && break
                 done)
          if [ -n "$same" ]; then
            echo "  (HEAD's own run: ${verdict})"
            verdict="GREEN $same (the same tree, ${tree:0:12})"
          fi ;;
      esac
    fi
  }
  gates_verdict
  # The gates run when we ship, not on every push (gates.yml's header). No
  # run yet: start one on main and wait for it, then wait on whatever is
  # running. The dry run does this too, so a dry run first warms the release.
  if [ "$verdict" = NONE ]; then
    if [ "$(git ls-remote origin refs/heads/main | cut -f1)" != "$SHA" ]; then
      verdict="ERROR HEAD $SHORT is not origin/main, and the gates run on main. Merge it first"
    elif gh workflow run gates --ref main 2>"$gh_err"; then
      echo "  no run for $SHORT yet: dispatched one on main"
      for _ in $(seq 24); do
        sleep 5
        url=$(gh run list --commit "$SHA" --workflow gates --event workflow_dispatch \
                --limit 1 --json url --jq '.[0].url // empty' 2>/dev/null)
        [ -n "$url" ] && { verdict="PENDING $url"; break; }
      done
      [ "$verdict" = NONE ] && verdict="ERROR dispatched the gates, but no run for $SHORT showed up in 2 minutes"
    else
      verdict="ERROR could not dispatch the gates: $(head -c 300 "$gh_err")"
    fi
  fi
  if [ "${verdict%% *}" = PENDING ]; then
    url=${verdict#PENDING }
    echo "  waiting on $url (about 35 minutes)"
    gh run watch "${url##*/}" --interval 60 >/dev/null 2>&1
    gates_verdict
  fi
  case "$verdict" in
    GREEN*) echo "  green: ${verdict#GREEN }" ;;
    RED*) refuse "HEAD's gates run is red (${verdict#RED }). Release a commit whose run is green" ;;
    PENDING*) refuse "HEAD's gates run has not finished (${verdict#PENDING }). Wait for it to go green" ;;
    NONE) refuse "no finished gates run on main for $SHORT (a cancelled one counts as none). Dispatch one: gh workflow run gates --ref main" ;;
    *) refuse "${verdict#ERROR }" ;;
  esac
fi

echo
echo "== 2. can HEAD's shard read the live saves =="
# The headers' cheap refusals (store.rs and worldfile.rs load paths): magic,
# format, slot count, seed. The content layout and the island digest are
# checked at boot, where deploy_shard.sh's verify sees a refusal.
saves=$(python3 - "$SAVE" "$WORLD" \
  "$(src SAVE_FORMAT crates/server/src/store.rs)" \
  "$(src MAX_SAVED_PLAYERS crates/server/src/store.rs)" \
  "$(src WORLD_FILE_FORMAT crates/server/src/worldfile.rs)" \
  "$(src WORLD_SAVE_FORMAT crates/sim-core/src/worldsave.rs)" \
  "$(cfg seed)" <<'PY'
import struct, sys
save, world = sys.argv[1], sys.argv[2]
fmt, cap, wfile, wblob, seed = (int(x) for x in sys.argv[3:8])
kinds = (
    (save, b"GATESAV\0", 24, (("format", "<H", 8, fmt), ("slots", "<I", 12, cap),
                              ("seed", "<Q", 16, seed))),
    (world, b"GATESWLD", 20, (("file format", "<H", 8, wfile), ("blob format", "<H", 10, wblob),
                              ("seed", "<Q", 12, seed))),
)
for path, magic, n, fields in kinds:
    try:
        with open(path, "rb") as f:
            h = f.read(n)
    except FileNotFoundError:
        print(f"absent {path}")
        continue
    except OSError as e:
        print(f"UNKNOWN {path}: {e}")
        continue
    if len(h) < n or h[:8] != magic:
        print(f"WIPE {path}: not a Gates file of that kind (magic or header)")
        continue
    bad = [f"{name} {struct.unpack_from(code, h, off)[0]}, HEAD {want}"
           for name, code, off, want in fields if struct.unpack_from(code, h, off)[0] != want]
    print(f"WIPE {path}: " + "; ".join(bad) if bad else f"ok {path}")
PY
)
saves_rc=$?
printf '%s\n' "$saves" | sed 's/^/  /'
if [ "$saves_rc" != 0 ]; then
  refuse "the save check itself failed (above): a constant it reads moved, or $CFG lost a key"
elif printf '%s\n' "$saves" | grep -q '^UNKNOWN'; then
  refuse "could not read the live saves; run this as the shard's user"
fi
if printf '%s\n' "$saves" | grep -q '^WIPE'; then
  if [ "$WIPE" = 1 ]; then
    echo "  HEAD cannot load them: this release is a wipe, and --wipe is set"
  else
    refuse "HEAD cannot load the live saves above, so its shard would refuse to boot. Re-run with --wipe"
  fi
elif ! printf '%s\n' "$saves" | grep -q '^ok'; then
  echo "  (no live saves here: not the shard box, or a fresh shard)"
fi

# The two the headers above cannot see, asked of HEAD's own shard: the
# content layout and the island's digest (`shard --check-saves`, read-only,
# and the boot's own checks). Built here, in the dry run too, because a
# release whose shard refuses to boot is a release that takes the island down,
# and this step exists to say so first. Step 3's build is then a no-op.
if [ -n "$SAVE$WORLD" ]; then
  echo "  HEAD's shard on them (cargo build --release -p server --bin shard):"
  if cargo build --release -q -p server --bin shard; then
    check=$(target/release/shard --check-saves "$CFG" 2>&1)
    check_rc=$?
    printf '%s\n' "$check" | sed 's/^/    /'
    case "$check_rc" in
      0) ;;
      3)
        if [ "$WIPE" = 1 ]; then
          echo "  HEAD's shard cannot load them: this release is a wipe, and --wipe is set"
        else
          refuse "HEAD's shard cannot load the live saves (above): this release is a wipe. Hold it for the scheduled wipe, or re-run with --wipe"
        fi ;;
      *) refuse "the save check could not run (above)" ;;
    esac
  else
    refuse "HEAD's shard did not build, so nothing can say whether it loads the live saves"
  fi
fi

# The wipe, as a function so the dry run lists exactly what --go would do.
# The close runs HEAD's shard (built in step 3) on the config the world ran
# under; HEAD's own file can post prizes that world never had.
CLOSE_CFG=$CFG
[ -f "$DEPLOY/$CFG" ] && CLOSE_CFG=$DEPLOY/$CFG
wipe_files() {
  python3 - "$WORLD" "$SAVE" "$1" target/release/shard "$CLOSE_CFG" <<'PY'
import os, pathlib, re, subprocess, sys, time
world, save, go = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
shard, close_cfg = sys.argv[4], sys.argv[5]
if not world or not save:
    sys.exit("shard-public.toml names no world_file or save_file to wipe")
now = int(time.time())
arch = pathlib.Path(f"{world}.wipes/{now}")
# wipe.rs `apply`: the world, its backups, a half-written temp, the trust log,
# the teams and the standings. The store moves whole: HEAD cannot rewrite its
# records.
names = [world + s for s in ("", ".1", ".2", ".3", ".tmp", ".trust",
                             ".teams", ".teams.tmp",
                             ".standings", ".standings.json")] + [save]
moving = [p for p in names if os.path.exists(p)]
state = pathlib.Path(world + ".wipe")
st = {"wipes": 0, "last": 0}
if state.exists():
    for raw in state.read_text().splitlines():
        line = raw.split("#")[0].strip()
        if not line:
            continue
        k, sep, v = (x.strip() for x in line.partition("="))
        if not sep or k not in st or not v.isdigit():
            sys.exit(f"wipe state {state}: bad line {raw!r}, which the shard refuses too")
        st[k] = int(v)
n = st["wipes"] + 1
# The close, as end_world in bin/shard.rs runs it before wipe::apply. The
# shard decides; this only says what it will find.
stand, last = pathlib.Path(world + ".standings"), pathlib.Path(world + ".lastwipe")
hall = pathlib.Path(world + ".hall")
def lines(p):
    return [l.split("\t") for l in p.read_text().splitlines()] if p.exists() else []
closed = any(f[:2] == ["wipe", str(n)] for f in lines(last)) \
    or any(f[:2] == ["podium", str(n)] for f in lines(hall))
rows = [f for f in lines(stand) if f[0] == "row"]
# A world this young is most likely the one the shard's own schedule just
# started (wipe_days): closing it posts a podium and a payout for minutes of
# play and counts the wipe twice. Stop the shard before its wipe hour and
# release after it instead.
age = now - st["last"]
if st["last"] and age < 2 * 3600:
    print(f"  !! this world began {age // 60} min ago: the shard has likely just run its own wipe.")
    print(f"     Closing it posts wipe {n}'s podium and payout for that world, and counts a wipe twice.")
if closed:
    print(f"  wipe {n} is closed already (the hall beside {world} has it); not twice")
elif stand.exists():
    cfg_text = pathlib.Path(close_cfg).read_text() if os.path.exists(close_cfg) else ""
    prizes = re.search(r'^prize_ticker\s*=\s*"([A-Z]+)"', cfg_text, re.M) and re.search(
        r'^prize_(island|works|hoard|fight)\s*=\s*"[^"]*[1-9]', cfg_text, re.M)
    extracted = any(len(f) > 10 and f[10].isdigit() and int(f[10]) > 0 for f in rows)
    owes = f", what it owes into {world}.payout-{n}.csv" if prizes or extracted else ""
    print(f"  close wipe {n} ({len(rows)} standings rows, prizes as {close_cfg} posts them):")
    print(f"    its podiums into {hall}, its island placings into {last}{owes}")
else:
    print(f"  no {stand}: the shard that ran this world kept no standings,")
    print(f"    so the hall gets no entry for wipe {n} and nothing is owed for it")
for p in moving:
    print(f"  {p} ({os.path.getsize(p):,} bytes) -> {arch}/")
if not moving:
    print(f"  nothing to move beside {world}")
print(f"  {state}: wipes {st['wipes']} -> {n}, last -> {now}. The hall stays.")
if not go:
    sys.exit()
print(f"+ {shard} --close-wipe {n} {close_cfg}", flush=True)
if subprocess.run([shard, "--close-wipe", str(n), close_cfg]).returncode != 0:
    sys.exit(f"  wipe {n} was not closed (above), so nothing moved")
arch.mkdir(parents=True)
for p in moving:
    os.rename(p, arch / os.path.basename(p))
# wipe.rs `write_state`'s format; `read_state` takes nothing else.
tmp = pathlib.Path(f"{state}.tmp")
tmp.write_text("# Gates wipe state, written by the shard at every wipe.\n"
               "# last = the unix second the current world began.\n"
               f"wipes = {n}\nlast = {now}\n")
os.replace(tmp, state)
print(f"  wipe {n}: archived in {arch}")
PY
}
if [ "$WIPE" = 1 ]; then
  echo
  echo "== 2b. what --wipe closes and moves =="
  wipe_files 0 || refuse "the wipe could not be planned (above)"
fi

if [ "$GO" = 1 ] && [ "${#REFUSED[@]}" -gt 0 ]; then
  echo
  echo "== refused: nothing was built or changed =="
  exit 1
fi

# Asked before the builds, so the rest runs unattended.
if [ "$GO" = 1 ] && [ "$WIPE" = 1 ]; then
  echo
  echo "== --wipe: this ends the world on $(cfg domain): every base, every body and"
  echo "   every player's save, blueprints included. The files are archived, not deleted."
  ans=""
  if ! read -r -p "   type wipe to go on: " ans </dev/tty; then
    echo; echo "  !! --wipe needs a terminal to confirm on; nothing done"; exit 1
  fi
  [ "$ans" = wipe ] || { echo "  not confirmed; nothing done"; exit 1; }
fi

echo
echo "== 3. build all of it, before anything moves =="
run cargo build --release -p server --bin shard || exit 1
# deploy_shard.sh rebuilds the agents after the restart; built here, that is a no-op.
if systemctl is-enabled --quiet gates-jev.service 2>/dev/null \
   || systemctl is-enabled --quiet gates-watch.service 2>/dev/null \
   || [ -n "$(systemctl list-units --all --plain --no-legend 'gates-teammate@*.service' 'gates-buddy@*.service' 'gates-buddies.service' 'gates-village@*.service' 2>/dev/null)" ]; then
  run cargo build --release -p server --features watch --bin jev-bot --bin jev-watch || exit 1
fi
run ./ci/build_web.sh target/webdist || exit 1
run ./ci/depot.py --platform linux-x86_64 || exit 1
run ./ci/depot.py --platform win-x86_64 || exit 1
if [ "$GO" = 1 ]; then
  python3 - "$PROTO" "$SHORT" "$BUILD" <<'PY' || exit 1
import json, pathlib, sys
proto, short, build = int(sys.argv[1]), sys.argv[2], sys.argv[3]
page = json.loads(pathlib.Path("target/webdist/build.json").read_text())
if page.get("proto") != proto or page.get("commit") != short:
    sys.exit(f"  !! target/webdist/build.json says proto {page.get('proto')} commit "
             f"{page.get('commit')}, HEAD is {proto} {short}. Is GATES_GIT_SHA set?")
for plat, doc in (("linux-x86_64", f"target/depot/{build}/depot.json"),
                  ("win-x86_64", f"target/depot/win-x86_64/{build}/depot.json")):
    d = json.loads(pathlib.Path(doc).read_text())
    if d.get("build") != build or d.get("platform") != plat:
        sys.exit(f"  !! {doc} says {d.get('build')} {d.get('platform')}, expected {build} {plat}")
print(f"  the page and both depots name {short}, proto {proto}")
PY
else
  echo "  would check: target/webdist/build.json and both depot.json name $SHORT, proto $PROTO"
fi

if [ "$WIPE" = 1 ]; then
  echo
  echo "== 3b. the wipe =="
  # Stopped first: a restart's SIGTERM flushes the old world over the move.
  run sudo systemctl stop "$UNIT" || exit 1
  if [ "$GO" = 1 ]; then
    if systemctl is-active --quiet "$UNIT"; then
      echo "  !! $UNIT is still active after stop; nothing moved"; exit 1
    fi
    if ! wipe_files 1; then
      echo "  !! the wipe failed part way; $UNIT is STOPPED. Look in ${WORLD%/*} before anything else."
      echo "     If nothing moved, sudo systemctl start $UNIT brings the old world back."
      exit 1
    fi
  else
    echo "  would close the wipe and move what 2b lists, then count it"
  fi
fi

echo
echo "== 4. publish: the shard first, because the page and the depots refuse a wire it does not speak =="
STEPS=(
  "./ci/deploy_shard.sh --go --strand-clients$WARN"
  "./ci/publish_web.sh --no-build"
  "./ci/publish_depot.py --platform linux-x86_64 --no-build --notarize"
  "./ci/publish_depot.py --platform win-x86_64 --no-build --notarize"
)
for i in "${!STEPS[@]}"; do
  read -ra cmd <<< "${STEPS[$i]}"
  if ! run "${cmd[@]}"; then
    echo
    echo "  !! stopped at: ${STEPS[$i]}"
    [ "$i" = 0 ] && [ "$WIPE" = 1 ] && echo "     $UNIT was stopped for the wipe and stays down until the deploy succeeds."
    echo "     finish by hand, in this order:"
    for ((j = i; j < ${#STEPS[@]}; j++)); do echo "       ${STEPS[$j]}"; done
    echo "       then ./ci/release.sh's steps 5-7 (the check, the store pull, the tag)"
    exit 1
  fi
done

echo
echo "== 5. every surface speaks proto $PROTO and names $SHORT =="
if [ "$GO" = 1 ]; then
  st=$(curl -fsS -m 15 "$STATUS_URL" 2>&1)
  pg=$(curl -fsS -m 15 "$PAGE_URL?release=$SHORT" 2>&1)
  mf=$(curl -fsS -m 30 "$MANIFEST_URL" 2>&1)
  if ! python3 - "$PROTO" "$SHORT" "$BUILD" "$st" "$pg" "$mf" <<'PY'
import json, sys
proto, short, build = int(sys.argv[1]), sys.argv[2], sys.argv[3]
def load(text):
    try:
        d = json.loads(text)
    except json.JSONDecodeError:
        d = None
    return d if isinstance(d, dict) else {"_raw": text[:120]}
st, pg, mf = (load(t) for t in sys.argv[4:7])
bad = 0
def check(ok, msg):
    global bad
    bad += not ok
    print(("  ok  " if ok else "  !!  ") + msg)
check(st.get("proto") == proto, f"shard status.json: proto {st.get('proto', st.get('_raw'))}")
check(pg.get("proto") == proto and pg.get("commit") == short,
      f"page build.json: proto {pg.get('proto', pg.get('_raw'))}, commit {pg.get('commit')}")
rows = {b.get("platform"): b for b in mf.get("builds", [])}
for plat in ("linux-x86_64", "win-x86_64"):
    b = rows.get(plat, {})
    check(b.get("version") == build and b.get("depot_state") == "published",
          f"depot {plat}: {b.get('version')} ({b.get('depot_state')})")
sys.exit(1 if bad else 0)
PY
  then
    echo "  !! a surface above does not match HEAD; the store was not poked and nothing is tagged"
    exit 1
  fi
else
  echo "  would check: $STATUS_URL proto, $PAGE_URL proto and commit,"
  echo "               $MANIFEST_URL serving $BUILD on both platforms"
fi

echo
echo "== 6. the store re-reads the repo manifest =="
WARN=0
if [ "$GO" = 1 ]; then
  out=$(curl -sS -m 60 -w '\n%{http_code}' -X POST -H 'content-type: application/json' \
          -d '{"game":"gates"}' "$PULL_URL" 2>&1)
  code=${out##*$'\n'}
  echo "  $code $(printf '%s' "${out%$'\n'*}" | head -c 400)"
  case "$code" in
    2??) ;;
    *) WARN=1
       echo "  !! the store did not take it; everything above is live. Re-poke once fixed:"
       echo "     curl -X POST -H 'content-type: application/json' -d '{\"game\":\"gates\"}' $PULL_URL" ;;
  esac
else
  echo "  would run: curl -X POST -H 'content-type: application/json' -d '{\"game\":\"gates\"}' $PULL_URL"
fi

echo
echo "== 7. the tag (the release workflow builds the downloads from it) =="
# Local tags in a dry run, which stays off the network; origin's too under --go.
if [ -n "$(git tag -l "v$VER")" ] \
   || { [ "$GO" = 1 ] && git ls-remote --exit-code --tags origin "refs/tags/v$VER" >/dev/null 2>&1; }; then
  echo "  !! v$VER is tagged already. Bump [workspace.package] version for a new release;"
  echo "     this one ships as $BUILD with no tag of its own."
else
  echo "  git tag -a v$VER -m \"Gates $VER\" $SHA && git push --no-verify origin v$VER"
fi
floor=$(cfg min_client)
if [ -n "$floor" ] && [ "$floor" != "$VER" ]; then
  echo "  then, once both depots serve $BUILD: min_client in $CFG is $floor; raise it to $VER"
  echo "  in a commit and a shard deploy if this release should be the floor. Never before."
fi

echo
if [ "$GO" = 1 ]; then
  echo "== released $BUILD. =="
  exit "$WARN"
fi
if [ "${#REFUSED[@]}" -gt 0 ]; then
  echo "== dry run: --go would refuse =="
  for r in "${REFUSED[@]}"; do echo "  - $r"; done
  exit 1
fi
echo "== dry run. re-run with --go to release. =="
