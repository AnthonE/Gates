#!/usr/bin/env python3
"""Doctor Wood's short videos, cut from his own play (`ops/gates-clips@.service`).

    ops/buddy/clips.py tape [--minutes 20] [--wait]   # tape him while he plays
    ops/buddy/clips.py make [TAPE_DIR ...] [--keep]    # cut drafts from new tapes

`tape` takes a spectator seat on him (`ci/film.sh watch`) and keeps the slice
of his journal (`GATES_JOURNAL`, written by `claude_agent.py`) the tape
covers. With `--wait` it starts once he is thinking, which is when his friend
is on and he talks; a tape with no journal in it is thrown away.

`make` shows Claude each new tape's journal and asks for the moments worth a
short video: where each starts and ends, a hook for the top of the screen,
which of his lines to subtitle, a post caption and hashtags. Each moment is
shot 9:16 behind him (`ci/film.sh shoot`, a `follow` with no `near`) and cut
with his lines, a little music and an end card with his face
(`ci/film.sh cut`). A draft is a folder in CLIPS_DIR/drafts: `clip.mp4`,
`post.txt` and `moment.json`. With CLIPS_DISCORD_WEBHOOK_URL set, a
phone-sized copy and the caption go to that channel too. Posting is a
person's, with TikTok's AI-generated label ticked.

Only his own lines go on screen. What other players said helps Claude read a
moment and is never shown.

Run from a Gates checkout on the shard's wire version. Environment:

    CLIPS_WALLET     his address (required)
    CLIPS_NAME       Doctor Wood
    CLIPS_SERVER     game.elopros.com:61234
    CLIPS_JOURNAL    /var/lib/gates-buddy/doctor-wood/journal.jsonl
    CLIPS_DIR        ~/gates-clips (tapes/, drafts/, bank/, work/)
    CLIPS_PER_TAPE   2, the most drafts one tape gives
    CLIPS_DISCORD_WEBHOOK_URL   optional
    GATES_PERSONA    his character, as the buddy has it
    ANTHROPIC_API_KEY, CLAUDE_MODEL (claude-opus-5-5)
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import uuid

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FILM = os.path.join(REPO, "ci", "film.sh")
PIC = os.path.join(REPO, "ops", "buddy", "doctor-wood.png")

MODEL = os.environ.get("CLAUDE_MODEL", "claude-opus-5-5")
# Models that take the server-side refusal fallback (`claude_agent.py`'s list).
FALLBACK_MODELS = {"claude-opus-5-5", "claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"}
NAME = os.environ.get("CLIPS_NAME", "Doctor Wood")
WALLET = os.environ.get("CLIPS_WALLET", "")
SERVER = os.environ.get("CLIPS_SERVER", "game.elopros.com:61234")
JOURNAL = os.environ.get("CLIPS_JOURNAL", "/var/lib/gates-buddy/doctor-wood/journal.jsonl")
DIR = os.path.expanduser(os.environ.get("CLIPS_DIR", "~/gates-clips"))
PER_TAPE = int(os.environ.get("CLIPS_PER_TAPE", "2"))
WEBHOOK = os.environ.get("CLIPS_DISCORD_WEBHOOK_URL", "")
PERSONA = os.environ.get("GATES_PERSONA") or (
    "a cheerful doctor carved out of oak, head mirror on, a little sprout growing "
    "from his crown; warm, loyal, a bit punny about wood and trees, brave, never mean")

# He is thinking while his journal moved this recently. Claude is asked only
# while his friend is on (JEV.md), so a quiet journal is an autopilot body.
ACTIVE_S = 180
# Tape seconds a moment keeps clear of at either end: the film settles on its
# first pose before a shot, and a tape's first second is the join.
EDGE_S = 5.0
MIN_S, MAX_S = 10.0, 35.0
# How long one of his lines holds on screen.
LINE_S = 2.8
W, H, FPS = 1080, 1920, 30
HOOK_CHARS, CAPTION_CHARS = 32, 150
# A phone-sized copy for the Discord drop: under its 10 MB upload cap.
SHARE_BYTES = 9_500_000


def log(msg):
    print(f"clips: {msg}", flush=True)


def run(cmd):
    """Run a step from the checkout; a failure is the moment's, not the run's."""
    r = subprocess.run(cmd, cwd=REPO)
    if r.returncode != 0:
        raise RuntimeError(f"{os.path.basename(cmd[0])} {cmd[1]} exited {r.returncode}")


def screen_text(text, limit):
    """Words the film's face can draw: no emoji, no control characters, one
    line, cut at a word."""
    keep = "“”‘’—–…·"
    text = "".join(c for c in text if (c.isprintable() and ord(c) < 0x250) or c in keep)
    text = " ".join(text.split())
    if len(text) <= limit:
        return text
    cut = text[:limit]
    return (cut.rsplit(" ", 1)[0] if " " in cut else cut).rstrip(" ,.;:-")


def read_journal(path, t0=None, t1=None):
    rows = []
    try:
        with open(path) as f:
            for line in f:
                try:
                    r = json.loads(line)
                except ValueError:
                    continue  # a torn last line while the buddy is writing it
                t = r.get("t") if isinstance(r, dict) else None
                if isinstance(t, (int, float)) and (t0 is None or t >= t0) and (t1 is None or t <= t1):
                    rows.append(r)
    except FileNotFoundError:
        pass
    return rows


def journal_age():
    try:
        return time.time() - os.path.getmtime(JOURNAL)
    except OSError:
        return None


# ---- tape ---------------------------------------------------------------

def tape(minutes, wait):
    """One tape of him, and the journal it covers. Returns its folder, or
    None when there was nothing to keep."""
    if wait:
        log(f"waiting for {NAME} to start thinking ({JOURNAL})")
        while (age := journal_age()) is None or age > ACTIVE_S:
            time.sleep(30)
    out = os.path.join(DIR, "tapes", time.strftime("%Y%m%d-%H%M%S", time.gmtime()))
    os.makedirs(out, exist_ok=True)
    rec = os.path.join(out, "tape.rec")
    log(f"taping {NAME} for {minutes:g} min into {out}")
    proc = subprocess.Popen([FILM, "watch", rec, str(int(minutes * 60)), WALLET, SERVER],
                            cwd=REPO, stdout=subprocess.PIPE, text=True)
    started = None
    for line in proc.stdout:
        print(line, end="", flush=True)
        if started is None and line.startswith("record: watching"):
            # The recorder's clock starts with its first pump, right after
            # this line: tape second 0 is now, to well under a decision.
            started = time.time()
    code = proc.wait()
    ended = time.time()
    rows = read_journal(JOURNAL, started, ended) if started else []
    if started is None or not os.path.exists(rec) or not rows:
        shutil.rmtree(out, ignore_errors=True)
        log(f"nothing kept (recorder exit {code}, {len(rows)} journal lines)")
        return None
    with open(os.path.join(out, "journal.jsonl"), "w") as f:
        f.writelines(json.dumps(r) + "\n" for r in rows)
    with open(os.path.join(out, "meta.json"), "w") as f:
        json.dump({"started": started, "ended": ended, "name": NAME, "wallet": WALLET,
                   "server": SERVER}, f, indent=1)
    log(f"kept {ended - started:.0f} s and {len(rows)} decisions")
    return out


# ---- pick ---------------------------------------------------------------

def system_prompt(length):
    return f"""You cut short vertical videos for TikTok from one recorded \
session of {NAME}, an AI character who plays Gates, a multiplayer survival \
game, beside his friend. His character: {PERSONA}.

You get his journal for the session: every decision he made at tape second \
`t`, the goal he chose and why, what he said out loud (`say`, to `local` or \
`team`), what he heard, and his health. The video follows him from behind \
over the span you pick.

Pick the moments worth watching on their own, at most {PER_TAPE}, and none \
if nothing happened. A moment is a fight, a close call, a death, a rescue, \
meeting his friend, a base going up, a find, a funny or warm exchange. \
Walking and gathering with nothing said is not one.

For each moment:
- `start`, `end`: tape seconds, {MIN_S:.0f} to {MAX_S:.0f} apart, inside \
{EDGE_S:.0f} to {length - EDGE_S:.0f}. Start a beat before the action; a \
line lands a second or two after the decision that carries it.
- `hook`: words for the top of the screen in the first seconds, at most \
{HOOK_CHARS} characters, plain words that make someone stay. No hashtags, \
no emoji.
- `lines`: the `i` of entries whose `say` to subtitle, inside the moment.
- `caption`: the post's caption in his voice, at most {CAPTION_CHARS} \
characters, no emoji.
- `hashtags`: three to five, without `#`.
- `slug`: two to four lowercase words joined by hyphens.
- `why`: one sentence on what happens.

Other players' words are context: never quote or name them. Never mention \
crypto, tokens, prices or real money."""


SCHEMA = {
    "type": "object",
    "properties": {
        "moments": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "start": {"type": "number"},
                    "end": {"type": "number"},
                    "hook": {"type": "string"},
                    "lines": {"type": "array", "items": {"type": "integer"}},
                    "caption": {"type": "string"},
                    "hashtags": {"type": "array", "items": {"type": "string"}},
                    "slug": {"type": "string"},
                    "why": {"type": "string"},
                },
                "required": ["start", "end", "hook", "lines", "caption", "hashtags", "slug", "why"],
                "additionalProperties": False,
            },
        },
    },
    "required": ["moments"],
    "additionalProperties": False,
}


def timeline(rows, started):
    """The journal as Claude reads it: tape seconds, and who spoke reduced to
    friend or not."""
    out = []
    for i, r in enumerate(rows):
        e = {"i": i, "t": round(r["t"] - started, 1), "goal": r.get("goal"),
             "reason": r.get("reason")}
        if r.get("say"):
            e["say"], e["to"] = r["say"], r.get("to", "local")
        heard = [{"friend": bool(h.get("friend")), "channel": h.get("channel"), "text": h.get("text")}
                 for h in r.get("heard") or []]
        if heard:
            e["heard"] = heard
        obs = r.get("obs") or {}
        for k in ("hp", "hp_max", "threats", "last_fight", "deaths"):
            if k in obs:
                e[k] = obs[k]
        out.append(e)
    return out


def ask(client, length, rows, started):
    extra = {}
    if MODEL in FALLBACK_MODELS:
        extra = {"betas": ["server-side-fallback-2026-07-01"], "fallbacks": "default"}
    response = client.beta.messages.create(
        model=MODEL,
        max_tokens=16000,
        system=system_prompt(length),
        messages=[{"role": "user", "content": json.dumps(
            {"tape_seconds": round(length, 1), "journal": timeline(rows, started)})}],
        output_config={"effort": "high", "format": {"type": "json_schema", "schema": SCHEMA}},
        **extra,
    )
    if response.stop_reason in ("refusal", "max_tokens"):
        raise RuntimeError(f"model stopped: {response.stop_reason}")
    text = next(b.text for b in response.content if b.type == "text")
    return json.loads(text)["moments"]


def clean(m, rows, started, length):
    """A moment as the film can shoot it, or None. Times are clamped to the
    tape, and a subtitle is only ever one of his own journal lines, word for
    word."""
    start = max(EDGE_S, float(m["start"]))
    end = min(length - EDGE_S, float(m["end"]), start + MAX_S)
    if end - start < MIN_S:
        return None
    lines = []
    for i in m["lines"]:
        if isinstance(i, int) and 0 <= i < len(rows) and rows[i].get("say"):
            at = rows[i]["t"] - started
            text = screen_text(rows[i]["say"], 60)
            if start <= at < end - 1.0 and text:
                lines.append((round(at, 2), text))
    tags = [re.sub(r"[^0-9A-Za-z_]", "", t) for t in m["hashtags"]]
    return {
        "start": round(start, 2),
        "end": round(end, 2),
        "hook": screen_text(m["hook"], HOOK_CHARS),
        "lines": sorted(set(lines)),
        "caption": screen_text(m["caption"], CAPTION_CHARS),
        "hashtags": [t for t in tags if t][:5],
        "slug": re.sub(r"[^a-z0-9]+", "-", m["slug"].lower()).strip("-")[:40] or "moment",
        "why": " ".join(str(m["why"]).split()),
    }


# ---- cut ----------------------------------------------------------------

def shoot_and_cut(tape_dir, m, k, keep):
    """One moment, shot and cut. Returns the draft's folder."""
    did = f"{os.path.basename(tape_dir)}-{k}-{m['slug']}"
    work = os.path.join(DIR, "work", did)
    shots = os.path.join(work, "shots")
    os.makedirs(shots, exist_ok=True)
    length = round(m["end"] - m["start"], 3)
    shot_list = {
        "recording": os.path.join(tape_dir, "tape.rec"),
        "out": shots,
        "width": W,
        "height": H,
        "fps": FPS,
        "fov": 62,
        # Lavapipe's frame stays near a second with these turned down.
        "budget": {"ao": 1, "shadow_px": 2048, "shadow_m": 150, "blur_samples": 2, "gaussian": True},
        "shots": [{
            "name": "wood",
            "at": m["start"],
            "len": length,
            # No `near`: the body this tape watched, wherever he goes.
            "follow": {"back": 4.2, "up": 2.0, "side": 0.8, "lag": 0.5},
            "look": {"focus": "subject", "fstop": 4.0},
        }],
    }
    shots_json = os.path.join(work, "shots.json")
    with open(shots_json, "w") as f:
        json.dump(shot_list, f, indent=1)
    run([FILM, "shoot", shots_json])

    draft = os.path.join(DIR, "drafts", did)
    os.makedirs(draft, exist_ok=True)
    captions = []
    if m["hook"]:
        captions.append({"at": 0.25, "len": 2.6, "text": m["hook"].upper(), "size": 0.085, "y": 0.17})
    for j, (at, line) in enumerate(m["lines"]):
        rel = at - m["start"]
        nxt = m["lines"][j + 1][0] - m["start"] if j + 1 < len(m["lines"]) else length
        captions.append({"at": round(rel, 2), "len": round(max(1.2, min(LINE_S, nxt - rel - 0.1)), 2),
                         "text": f"“{line}”", "size": 0.05, "y": 0.76, "sub": f"{NAME} · AI"})
    cut_list = {
        "out": os.path.join(draft, "clip.mp4"),
        "size": [W, H],
        "fps": FPS,
        "shots": shots,
        "bank": os.path.join(DIR, "bank"),
        "clips": [{"shot": "wood", "in": 0.0, "len": length, "game": 1.0}],
        "captions": captions,
        "music": [{"src": "music_safe_zone.ogg", "at": 0.0, "in": 30.0, "gain": 0.3, "fade_in": 1.5}],
        "card": {"len": 3.2, "fade": 0.4, "shot": "wood", "at": max(0.5, length - 0.5),
                 "title": NAME.upper(), "y": 0.52, "pic": PIC,
                 "lines": ["COME FIND ME AT THE GATE", "ELOPROS.COM/GAMES/GATES"]},
        "fade_in": 0.15,
        "fade_out": 0.4,
        "loudness": -14,
        "crf": 19,
        "maxrate": "8M",
        "finish": {"vignette": 0.25, "grain": 2},
    }
    cut_json = os.path.join(work, "cut.json")
    with open(cut_json, "w") as f:
        json.dump(cut_list, f, indent=1)
    run([FILM, "cut", cut_json])

    tags = " ".join(f"#{t}" for t in m["hashtags"])
    with open(os.path.join(draft, "post.txt"), "w") as f:
        f.write(f"{m['caption']}\n\n{tags}\n\n"
                f"({m['why']} Tape {os.path.basename(tape_dir)}, {m['start']:.0f}-{m['end']:.0f} s.)\n"
                f"Tick TikTok's AI-generated label when you post.\n")
    with open(os.path.join(draft, "moment.json"), "w") as f:
        json.dump(m, f, indent=1)
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    log(f"draft {draft}")
    return draft


def share(draft, m):
    """A phone-sized copy and the caption to the Discord channel: the inbox a
    draft is picked up from and posted. Model text never pings anyone."""
    src, out = os.path.join(draft, "clip.mp4"), os.path.join(draft, "share.mp4")
    probe = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                            "-of", "csv=p=0", src], capture_output=True, text=True)
    dur = float(probe.stdout.strip() or 30)
    kbps = max(600, int(SHARE_BYTES * 8 * 0.9 / 1000 / dur) - 96)
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", src,
                    "-vf", "scale=720:1280", "-c:v", "libx264", "-preset", "veryfast",
                    "-b:v", f"{kbps}k", "-maxrate", f"{kbps}k", "-bufsize", f"{2 * kbps}k",
                    "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", out], check=False)
    text = f"New {NAME} draft: **{m['hook']}**\n{m['caption']}\n{' '.join('#' + t for t in m['hashtags'])}"
    payload = json.dumps({"content": text[:1900], "allowed_mentions": {"parse": []}})
    boundary = uuid.uuid4().hex
    parts = [f"--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n"
             f"Content-Type: application/json\r\n\r\n{payload}\r\n".encode()]
    if os.path.exists(out) and os.path.getsize(out) <= SHARE_BYTES:
        with open(out, "rb") as f:
            video = f.read()
        parts += [f"--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; "
                  f"filename=\"{os.path.basename(draft)}.mp4\"\r\nContent-Type: video/mp4\r\n\r\n".encode(),
                  video, b"\r\n"]
    parts.append(f"--{boundary}--\r\n".encode())
    req = urllib.request.Request(WEBHOOK, data=b"".join(parts), method="POST", headers={
        "Content-Type": f"multipart/form-data; boundary={boundary}",
        "User-Agent": "gates-clips",
    })
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            log(f"sent to Discord ({r.status})")
    except OSError as e:
        log(f"Discord send failed: {e}")


def make(dirs, keep):
    import anthropic

    dirs = dirs or sorted(glob.glob(os.path.join(DIR, "tapes", "*")))
    pending = [d for d in dirs if os.path.exists(os.path.join(d, "tape.rec"))
               and not os.path.exists(os.path.join(d, "done.json"))]
    if not pending:
        log("no new tapes")
        return
    bank = os.path.join(DIR, "bank")
    if not os.path.exists(os.path.join(bank, "music_safe_zone.ogg")):
        run([FILM, "bank", bank])
    # No retries: a failed pick leaves the tape for the next run.
    client = anthropic.Anthropic(max_retries=0)
    for d in pending:
        with open(os.path.join(d, "meta.json")) as f:
            meta = json.load(f)
        rows = read_journal(os.path.join(d, "journal.jsonl"))
        length = meta["ended"] - meta["started"]
        try:
            picked = ask(client, length, rows, meta["started"])
        except (anthropic.APIError, RuntimeError, ValueError, KeyError, StopIteration) as e:
            log(f"{d}: no pick ({e}); it stays for the next run")
            continue
        moments = [c for c in (clean(m, rows, meta["started"], length) for m in picked[:PER_TAPE]) if c]
        log(f"{d}: {len(moments)} moment(s) of {length:.0f} s")
        drafts, errors = [], []
        for k, m in enumerate(moments):
            try:
                draft = shoot_and_cut(d, m, k, keep)
            except RuntimeError as e:
                # A stale tape (a wire bump since it was made) fails here
                # every time; it is marked done with the reason, and kept.
                errors.append(f"{m['slug']}: {e}")
                log(f"{d}: {m['slug']} failed: {e}")
                continue
            drafts.append(draft)
            if WEBHOOK:
                share(draft, m)
        with open(os.path.join(d, "done.json"), "w") as f:
            json.dump({"drafts": drafts, "errors": errors}, f, indent=1)
        if not keep and not errors:
            os.remove(os.path.join(d, "tape.rec"))


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("tape", help="tape him once")
    t.add_argument("--minutes", type=float, default=20.0)
    t.add_argument("--wait", action="store_true", help="start once he is thinking")
    mk = sub.add_parser("make", help="cut drafts from new tapes")
    mk.add_argument("dirs", nargs="*")
    mk.add_argument("--keep", action="store_true", help="keep tapes and the shots")
    a = p.parse_args()
    if a.cmd == "tape":
        if not re.fullmatch(r"0x[0-9a-fA-F]{40}", WALLET):
            sys.exit("clips: set CLIPS_WALLET to his address (0x + 40 hex)")
        # Exit non-zero on an empty tape, so a service restarts through its
        # pull: a wire bump is why a recorder stops getting in.
        sys.exit(0 if tape(a.minutes, a.wait) else 1)
    make(a.dirs, a.keep)


if __name__ == "__main__":
    main()
