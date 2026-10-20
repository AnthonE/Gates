"""A day with an agent in a few lines, for the card on elopros.com/buddy.html.

`runner.py` asks `due()` once an hour, and runs `write()` for each agent it
names from a thread of its own. A recap covers the journal since the last
one (`journal/<id>.jsonl`, written by `claude_agent.py` or, for an own brain,
`mind_bridge.py`): runs of the same goal fold into one line each, and one
model call (CLAUDE_MODEL) turns them into a few sentences in the agent's own
voice and up to three moments. The call is billed to the agent's pool like
its thinking (a line in `spend/<id>.jsonl`), and an agent whose pool is empty
gets none. At most one a day each; the last three ride in rent.json.

Only the agent's own lines are ever quoted. What others said helps the model
read a moment and is never repeated.
"""
import importlib.util
import json
import os
import time

QUIET_S = 20 * 60        # a session ends when its journal has been still this long
LONG_S = 6 * 3600        # or this much play has piled up while it keeps going
MIN_ROWS = 10            # fewer decisions than this is not a day
READ_MAX = 8 << 20       # the most of a journal one recap reads
MAX_SEGMENTS = 120
TEXT_CHARS = 400
LINE_CHARS = 120
MIN_LEFT_USD = 0.01

SCHEMA = {
    "type": "object",
    "properties": {
        "text": {"type": "string"},
        "moments": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {"t": {"type": "number"}, "line": {"type": "string"}},
                "required": ["t", "line"],
                "additionalProperties": False,
            },
        },
    },
    "required": ["text", "moments"],
    "additionalProperties": False,
}


def _prices(agent_py):
    """The price list `claude_agent.py` bills thinking at, so a recap is
    billed the same way."""
    spec = importlib.util.spec_from_file_location("claude_agent_prices", agent_py)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.PRICES


def call_usd(prices, model, usage):
    p_in, p_out, p_read, p_write = prices.get(model) or max(prices.values())
    n = lambda k: getattr(usage, k, 0) or 0
    return (n("input_tokens") * p_in + n("output_tokens") * p_out
            + n("cache_read_input_tokens") * p_read
            + n("cache_creation_input_tokens") * p_write) / 1e6


def last(recaps_path):
    """The newest recap line, or None."""
    try:
        with open(recaps_path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - 16384))
            lines = f.read().splitlines()
    except OSError:
        return None
    for line in reversed(lines):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if isinstance(r, dict):
            return r
    return None


def recent(recaps_path, n=3):
    """The last `n` recaps as rent.json carries them."""
    try:
        with open(recaps_path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - 16384))
            lines = f.read().splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue
        out.append({"day": r.get("day"), "text": r.get("text"), "moments": r.get("moments") or []})
    return out


def rows_since(journal_path, off):
    """The journal's whole lines from byte `off` (or its last READ_MAX), and
    the offset after them."""
    try:
        with open(journal_path, "rb") as f:
            f.seek(0, os.SEEK_END)
            end = f.tell()
            if end < off:
                off = 0  # a new journal
            off = max(off, end - READ_MAX)
            f.seek(off)
            data = f.read(end - off)
    except OSError:
        return [], off
    rows = []
    cut = data.rfind(b"\n") + 1
    for line in data[:cut].splitlines():
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if isinstance(r, dict) and isinstance(r.get("t"), (int, float)):
            rows.append(r)
    return rows, off + cut


def due(journal_path, recaps_path, now):
    """True when the agent has a day worth telling: enough new decisions, a
    session that has ended (or a long one), and no recap yet today."""
    prev = last(recaps_path) or {}
    today = time.strftime("%Y-%m-%d", time.gmtime(now))
    if prev.get("day") == today:
        return False
    try:
        if now - os.path.getmtime(journal_path) < QUIET_S:
            rows, _ = rows_since(journal_path, int(prev.get("to_off") or 0))
            return len(rows) >= MIN_ROWS and now - rows[0]["t"] >= LONG_S
    except OSError:
        return False
    rows, _ = rows_since(journal_path, int(prev.get("to_off") or 0))
    return len(rows) >= MIN_ROWS


def segments(rows):
    """Runs of the same goal folded into one entry each: when, how many
    decisions, a reason, what the agent said, and what happened to it."""
    out = []
    for r in rows:
        obs = r.get("obs") if isinstance(r.get("obs"), dict) else {}
        goal = str(r.get("goal") or "?")[:32]
        if not out or out[-1]["goal"] != goal:
            out.append({"t": round(r["t"]), "goal": goal, "decisions": 0})
        s = out[-1]
        s["decisions"] += 1
        if r.get("reason") and "why" not in s:
            s["why"] = str(r["reason"])[:96]
        if r.get("say") and r.get("to") != "team":  # team chat is private; recaps are not
            said = s.setdefault("said", [])
            if len(said) < 4:
                said.append({"t": round(r["t"]), "line": str(r["say"])[:144]})
        hp = obs.get("health")
        hp = hp[0] if isinstance(hp, list) and hp else hp  # [now, max]
        if isinstance(hp, (int, float)):
            s["lowest_health"] = min(s.get("lowest_health", 1e9), hp)
        for k in ("threats", "last_fight", "deaths"):
            if obs.get(k) not in (None, [], {}, 0):
                s[k] = obs[k]
        heard = [h for h in r.get("heard") or [] if isinstance(h, dict)]
        if heard:
            s["heard"] = s.get("heard", 0) + len(heard)
            if any(h.get("friend") for h in heard):
                s["friend_spoke"] = True
    return out[-MAX_SEGMENTS:]


def system_prompt(name, persona, kind):
    reader = ("anyone who visits your page" if kind == "community"
              else "the player you belong to, who was not watching all of it")
    return f"""You are {name}, an AI character who plays Gates, a multiplayer \
survival game. {persona}

Write a short recap of your latest session on the island for {reader}. You \
get your journal folded into segments: when each began (unix seconds), the \
goal you chose and how many decisions in a row, a reason, the lines you said \
out loud, your lowest health, threats, fights and deaths, and how many lines \
you heard.

- `text`: at most {TEXT_CHARS} characters, first person, in your voice, \
plain words. What you did, what went well or badly, one thing you would like \
to do next time. No lists, no emoji.
- `moments`: up to three things worth hearing about, each one sentence of at \
most {LINE_CHARS} characters with `t`, the unix second it happened, taken \
from the segment it is about. None if nothing stood out.

Only quote your own lines. Never quote, name or describe what other players \
said. Never mention crypto, tokens, prices or real money. Never hateful, \
sexual or cruel."""


def _plain(text, n):
    text = " ".join("".join(c for c in str(text) if c.isprintable()).split())
    if len(text) <= n:
        return text
    cut = text[:n]
    return (cut.rsplit(" ", 1)[0] if " " in cut else cut).rstrip(" ,;:-") + "…"


def clean(got, t0, t1):
    """The model's answer as the card shows it: short, printable, and each
    moment inside the span it covered."""
    moments = []
    for m in (got.get("moments") or [])[:3]:
        if not isinstance(m, dict) or not isinstance(m.get("t"), (int, float)):
            continue
        line = _plain(m.get("line") or "", LINE_CHARS)
        if line and t0 - 60 <= m["t"] <= t1 + 60:
            moments.append({"t": int(m["t"]), "line": line})
    return {"text": _plain(got.get("text") or "", TEXT_CHARS), "moments": moments}


def write(client, model, prices, aid, a, paths, now=None):
    """Make one agent's recap and bill it. Returns the recap, or None."""
    now = time.time() if now is None else now
    prev = last(paths["recaps"]) or {}
    rows, end = rows_since(paths["journal"], int(prev.get("to_off") or 0))
    if len(rows) < MIN_ROWS:
        return None
    segs = segments(rows)
    response = client.beta.messages.create(
        model=model,
        max_tokens=2048,
        system=system_prompt(a["name"], str(a.get("persona") or "")[:600], a.get("kind")),
        messages=[{"role": "user", "content": json.dumps({"segments": segs}, separators=(",", ":"))}],
        output_config={"effort": "low", "format": {"type": "json_schema", "schema": SCHEMA}},
    )
    cost = call_usd(prices, response.model or model, response.usage)
    n = lambda k: getattr(response.usage, k, 0) or 0
    with open(paths["spend"], "a") as f:
        f.write(json.dumps({"t": round(now, 2), "model": response.model or model, "usd": round(cost, 8),
                            "in": n("input_tokens"), "out": n("output_tokens"),
                            "cache_read": n("cache_read_input_tokens"),
                            "cache_write": n("cache_creation_input_tokens"), "for": "recap"}) + "\n")
    if response.stop_reason in ("refusal", "max_tokens"):
        return None
    text = next(b.text for b in response.content if b.type == "text")
    rec = clean(json.loads(text), rows[0]["t"], rows[-1]["t"])
    if not rec["text"]:
        return None
    rec.update({"day": time.strftime("%Y-%m-%d", time.gmtime(now)), "at": int(now),
                "from_t": int(rows[0]["t"]), "to_t": int(rows[-1]["t"]), "to_off": end})
    with open(paths["recaps"], "a") as f:
        f.write(json.dumps(rec) + "\n")
    return rec


def run(client_factory, model, agent_py, jobs, log):
    """Write each due recap in turn; one failure costs that agent's recap
    until the next hour, never the others'."""
    try:
        prices = _prices(agent_py)
        client = client_factory()
    except Exception as e:  # noqa: BLE001 — no client, no recaps this hour
        log(f"recaps skipped: {e}")
        return
    for aid, a, paths in jobs:
        try:
            rec = write(client, model, prices, aid, a, paths)
            if rec:
                log(f"{aid}: recap for {rec['day']}")
        except Exception as e:  # noqa: BLE001
            log(f"{aid}: no recap this hour: {e}")
