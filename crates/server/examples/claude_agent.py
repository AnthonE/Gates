#!/usr/bin/env python3
"""Claude plays Gates: our mind for gates-agent and jev-watch.

The body starts this program and owns its pipes: one JSON request per line
on stdin, one reply per line on stdout; logs go to stderr. A tactician
answers every turn and a strategist plans on a thread of its own, drawing on
the house pool when GATES_POOL is set. Its rules are the brief
(`crates/server/brief/`, beside this script, one level up, or at
GATES_BRIEF_DIR). Everything else, every environment variable included, is
`crates/server/AGENT-MANUAL.md` ("claude_agent.py", "Environment").

    pip install anthropic
    export ANTHROPIC_API_KEY=...              # or `ant auth login`
    cargo run -p server --release --bin gates-agent -- --local --seconds 600 \\
        --think-ms 3000 --timeout-ms 20000 \\
        --external python3 crates/server/examples/claude_agent.py [--via api|claude-code] [--persona TEXT]
"""
import base64
import calendar
import collections
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
MODEL = os.environ.get("CLAUDE_MODEL", "claude-opus-5-5")
# Kept under the body's --timeout-ms: a slow answer fails here, at once,
# rather than arriving after the body has moved on to the next request.
TIMEOUT_S = float(os.environ.get("CLAUDE_TIMEOUT_S", "15"))
STRATEGIST_MODEL = os.environ.get("STRATEGIST_MODEL", "claude-sonnet-5-5")
STRATEGIST_EFFORT = os.environ.get("STRATEGIST_EFFORT", "medium")
STRATEGIST_EVERY_S = float(os.environ.get("STRATEGIST_EVERY_S", "600"))
STRATEGIST_TIMEOUT_S = float(os.environ.get("STRATEGIST_TIMEOUT_S", "120"))
# Events bring the strategist sooner, but never closer together than this.
STRATEGIST_GAP_S = float(os.environ.get("STRATEGIST_GAP_S", "90"))
# The share of its class's allowance past which the strategist waits.
STRATEGIST_HEADROOM = float(os.environ.get("STRATEGIST_HEADROOM", "0.8"))
PLANNER_MODEL = os.environ.get("PLANNER_MODEL", "")
PLANNER_PER_DAY = int(os.environ.get("PLANNER_PER_DAY", "3"))
POOL = os.environ.get("GATES_POOL", "")
POOL_USD = float(os.environ.get("GATES_POOL_USD", "400"))
POOL_CLASS = os.environ.get("GATES_POOL_CLASS", "")
POOL_SMOOTH = os.environ.get("GATES_POOL_SMOOTH", "1") not in ("0", "", "off", "no")
# The chat rule's cap (protocol CHAT_MAX_BYTES), and how many such lines
# one `say` may fill (mind::Lines::MAX). The body refuses more rather than
# cutting it, so it is cut here, at a word.
SAY_BYTES = 120
SAY_LINES = 3
# What the body speaks at most (agent_voice.rs MAX_SPEECH_CHARS).
SPEAK_CHARS = 240
REASON_CHARS = 96
THOUGHT_CHARS = 280
CAPTION_CHARS = 300
# What it keeps between turns: its own notes, and the chat it heard.
NOTES_CHARS = 300
# The strategy's parts (the contract's caps).
AMBITION_CHARS = 160
ORDER_CHARS = 120
ORDERS_MAX = 4
SUMMARY_CHARS = 600
PLAN_STEPS = 8
STANCES = ("friend", "neutral", "wary", "enemy", "rival")
STANCE_MAX = 8
# A bug report's title (client::report::TITLE_MAX) and what happened.
BUG_TITLE_CHARS = 120
BUG_BODY_CHARS = 1500
BUG_KINDS = ["", "verb", "world", "numbers", "look", "net", "other"]
CHAT_LOG_LINES = 20
DIGEST_TURNS = 30
FRAME_URL = os.environ.get("GATES_FRAME_URL", "")
WEBHOOK_URL = os.environ.get("DISCORD_WEBHOOK_URL", "")
SHARE_EVERY_S = float(os.environ.get("SHARE_EVERY_S", "300"))
JOURNAL = os.environ.get("GATES_JOURNAL", "")
HARNESS = os.environ.get("GATES_HARNESS", "")
# The observation fields a journal line keeps: enough to tell a fight, a
# death or a haul from a walk, and small enough to write every turn.
JOURNAL_OBS = ("health", "food", "water", "threats", "last_fight", "last_goal",
               "current_goal", "deaths", "respawns")
# Models that take the server-side refusal fallback (API route only).
FALLBACK_MODELS = {"claude-opus-5-5", "claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"}
# Anthropic's posted list prices, dollars per million tokens: input, output,
# cache read, 5-minute cache write (Haiku's rate is for prompts up to 100K,
# which a turn always is). A model not listed is billed at the dearest row,
# so a rented buddy is never billed under what its calls cost.
PRICES = {
    "claude-fable-5-1": (10.00, 50.00, 0.25, 12.50),
    "claude-opus-5-5": (4.00, 20.00, 0.20, 5.00),
    "claude-sonnet-5-5": (2.00, 10.00, 0.20, 2.50),
    "claude-haiku-5-5": (0.10, 0.50, 0.01, 0.125),
}
SPEND = os.environ.get("GATES_SPEND", "")
CREDIT_FILE = os.environ.get("GATES_CREDIT_FILE", "")
STATUS_FILE = os.environ.get("GATES_STATUS", "")
NOTES_FILE = os.environ.get("GATES_NOTES", "")

# The brief's pieces (`crates/server/brief/`), by name.
BRIEF = ("core", "culture", "lines", "friend", "voice", "crew", "share", "strategist", "council")


def find_brief_dir():
    for d in (os.environ.get("GATES_BRIEF_DIR"), os.path.join(HERE, "brief"),
              os.path.join(HERE, "..", "brief")):
        if d and os.path.isfile(os.path.join(d, "core.md")):
            return os.path.normpath(d)
    return None


def load_brief(d=None):
    """{piece: text} from the brief directory; exits when there is none,
    since a mind without its rules must not play."""
    d = d or find_brief_dir()
    if not d:
        sys.exit("claude_agent.py: no brief (crates/server/brief/); set GATES_BRIEF_DIR")
    out = {}
    for name in BRIEF:
        with open(os.path.join(d, name + ".md")) as f:
            out[name] = f.read().strip()
    return out


def one_line(v, n):
    """Printable, single-spaced, at most `n` characters."""
    text = "".join(c if c.isprintable() else " " for c in str(v or ""))
    return " ".join(text.split())[:n]


def character_text(character, persona):
    """Who the mind is, as its system prompt says it: the request's
    `character`, else the --persona text."""
    if isinstance(character, dict) and character.get("name"):
        c = character
        parts = [f"# Who you are\n\nYou are {one_line(c.get('name'), 32)}"
                 + (f", a {one_line(c.get('kind'), 16)} on the island." if c.get("kind") else ".")]
        if c.get("persona"):
            parts.append(one_line(c["persona"], 600))
        traits = c.get("traits")
        if isinstance(traits, dict) and traits:
            parts.append("Your traits, 0-100: " + ", ".join(
                f"{one_line(k, 16)} {int(v)}" for k, v in sorted(traits.items())
                if isinstance(v, (int, float))) + ".")
        quirks = [one_line(q, 80) for q in c.get("quirks") or [] if isinstance(q, str)]
        if quirks:
            parts.append("Your quirks: " + "; ".join(quirks[:8]) + ".")
        if c.get("goal"):
            parts.append("Your life goal: " + one_line(c["goal"], 200))
        parts.append("Play this character; the rules above still hold.")
        return "\n\n".join(parts)
    if persona:
        return "# Who you are\n\n" + persona + "\n\nPlay this character; the rules above still hold."
    return ""


def tactician_system(brief, request, persona, voice, can_share):
    parts = [brief["core"], brief["culture"], brief["lines"]]
    if request.get("friend"):
        parts.append(brief["friend"])
    if request.get("crew"):
        parts.append(brief["crew"])
    if voice:
        parts.append(brief["voice"])
    who = character_text(request.get("character"), persona)
    if who:
        parts.append(who)
    if can_share:
        parts.append(brief["share"])
    return "\n\n".join(parts)


def strategist_system(brief, character, persona, council=False):
    parts = [brief["strategist"]]
    if council:
        parts.append(brief["council"])
    parts += [brief["culture"], brief["lines"]]
    who = character_text(character, persona)
    if who:
        parts.append(who)
    return "\n\n".join(parts)


def schema(share, voice=False):
    # The same schema every turn: it is part of the cached prefix, so an enum
    # of this turn's options would miss the cache nearly every time. The
    # options are in the turn, and `main` refuses a goal not among them.
    props = {
        "goal": {"type": "string"},
        "reason": {"type": "string"},
        "thought": {"type": "string"},
        "say": {"type": "string"},
        "to": {"type": "string", "enum": ["global", "team", "local"]},
        "code": {"type": "string"},
        "notes": {"type": "string"},
        "bug": {"type": "string"},
        "bug_kind": {"type": "string", "enum": BUG_KINDS},
    }
    if voice:
        props["speak"] = {"type": "string"}
    if share:
        props["share"] = {"type": "string"}
    return {
        "type": "object",
        "properties": props,
        "required": list(props),
        "additionalProperties": False,
    }


def _obj(props):
    return {"type": "object", "properties": props, "required": list(props),
            "additionalProperties": False}


def strategist_schema():
    """The strategist's one answer. A step's `item` "" and `count` 0 mean no
    `until`; the contract's shape is made from it (`strategy_reply`)."""
    s = {"type": "string"}
    return _obj({
        "thought": s,
        "strategy": _obj({"ambition": s, "project": s,
                          "standing_orders": {"type": "array", "items": s},
                          "summary": s}),
        "plan": {"type": "array", "items": _obj({"goal": s, "item": s, "count": {"type": "integer"}})},
        "stance": {"type": "array", "items": _obj({"name": s, "stance": {"type": "string", "enum": list(STANCES)}})},
        "council": s,
    })


def plan_goals(options):
    """The goals a plan step may name: offered ones as written, never a
    template (`walk:<bearing>:<metres>`)."""
    return {o for o in options if isinstance(o, str) and "<" not in o}


def strategy_reply(answer, goals, people):
    """The strategist's answer as the reply fields the contract names:
    {"thought", "strategy", "plan", "stance"}, each checked and cut. A plan
    step whose goal is not one the body offers is dropped (the body refuses
    a plan with one whole); a stance on a name memory does not hold is
    dropped."""
    out = {}
    thought = one_line(answer.get("thought"), THOUGHT_CHARS)
    if thought:
        out["thought"] = thought
    st = answer.get("strategy") or {}
    strategy = {
        "ambition": one_line(st.get("ambition"), AMBITION_CHARS),
        "project": one_line(st.get("project"), AMBITION_CHARS),
        "standing_orders": [o for o in (one_line(x, ORDER_CHARS) for x in (st.get("standing_orders") or [])
                                        if isinstance(x, str)) if o][:ORDERS_MAX],
        "summary": one_line(st.get("summary"), SUMMARY_CHARS),
    }
    if strategy["ambition"] or strategy["project"]:
        out["strategy"] = strategy
    plan = []
    for step in answer.get("plan") or []:
        if not isinstance(step, dict) or step.get("goal") not in goals:
            continue
        row = {"goal": step["goal"]}
        item, count = one_line(step.get("item"), 32), step.get("count")
        if item and isinstance(count, int) and 1 <= count <= 1_000_000:
            row["until"] = {"item": item, "count": count}
        plan.append(row)
    if plan:
        out["plan"] = plan[:PLAN_STEPS]
    stance = []
    for s in answer.get("stance") or []:
        if (isinstance(s, dict) and s.get("stance") in STANCES and s.get("name") in people
                and s["name"] not in [x["name"] for x in stance]):
            stance.append({"name": s["name"], "stance": s["stance"]})
    if stance:
        out["stance"] = stance[:STANCE_MAX]
    return out


def clip_say(text):
    """Printable text that fits SAY_LINES chat lines cut at spaces, as the
    body cuts it: no slash command, and what does not fit is dropped."""
    text = re.sub(r"\s+", " ", "".join(c if c.isprintable() else " " for c in text)).strip()
    text = text.lstrip("/").strip()
    lines, cur = [], ""
    for word in text.split(" "):
        word = word.encode()[:SAY_BYTES].decode(errors="ignore")
        if cur and len(cur.encode()) + 1 + len(word.encode()) > SAY_BYTES:
            lines.append(cur)
            cur = ""
        cur = f"{cur} {word}" if cur else word
    lines.append(cur)
    # A line after the first that opens with "/" would be a command.
    return " ".join(l for i, l in enumerate(lines[:SAY_LINES]) if i == 0 or not l.startswith("/"))


def fetch_frame():
    """The body's current view as (jpeg bytes, age in ms), or None."""
    if not FRAME_URL:
        return None
    try:
        with urllib.request.urlopen(FRAME_URL, timeout=2) as r:
            jpeg = r.read()
            state = json.loads(r.headers.get("X-Bot-State") or "{}")
        return jpeg, int(state.get("age_ms", 0))
    except (OSError, ValueError) as e:
        print(f"no frame: {e}", file=sys.stderr)
        return None


def read_status():
    """GATES_STATUS as an object, or None."""
    if not STATUS_FILE:
        return None
    try:
        with open(STATUS_FILE) as f:
            doc = json.load(f)
        return {"left_usd": round(float(doc["left_usd"]), 2)}
    except (OSError, ValueError, KeyError, TypeError):
        return None


def load_notes():
    try:
        with open(NOTES_FILE) as f:
            return " ".join(f.read().split())[:NOTES_CHARS]
    except OSError:
        return ""


WALK_ANY = "walk:<bearing>:<metres>"
BEARINGS = ("ahead", "ahead-right", "right", "behind-right",
            "behind", "behind-left", "left", "ahead-left")


def walk_ok(goal, options):
    """A filled-in walk (`walk:ahead-left:300`) answers the walk template
    when it is on offer; the body checks it again (`mind::Walk::parse`)."""
    if WALK_ANY not in options or not goal.startswith("walk:"):
        return False
    parts = goal.split(":")
    return (len(parts) == 3 and parts[1] in BEARINGS and parts[2].isdigit()
            and 10 <= int(parts[2]) <= 600)


def save_notes(notes):
    try:
        tmp = NOTES_FILE + ".tmp"
        with open(tmp, "w") as f:
            f.write(notes)
        os.replace(tmp, NOTES_FILE)
    except OSError as e:
        print(f"notes: {e}", file=sys.stderr)


def turn_content(request, recent, frame, notes, chat_log, strategy=None):
    turn = {
        "observation": request["observation"],
        "options": request["options"],
        "heard": request.get("heard", []),
        "team": request.get("team", {}),
        "friend": request.get("friend"),
        "crew": request.get("crew"),
        "places": request.get("places", {}),
        "recent": list(recent),
        "notes": notes,
        "chat_log": list(chat_log),
    }
    if request.get("memory"):
        turn["memory"] = request["memory"]
    strategy = request.get("strategy") or strategy
    if strategy:
        turn["strategy"] = strategy
    if request.get("voice"):
        turn["speaking"] = bool(request.get("speaking"))
    status = read_status()
    if status:
        turn["credit"] = status
    blocks = []
    if frame:
        jpeg, age_ms = frame
        turn["view"] = f"the picture is your own first-person view, {age_ms} ms old"
        blocks.append({
            "type": "image",
            "source": {"type": "base64", "media_type": "image/jpeg",
                       "data": base64.b64encode(jpeg).decode()},
        })
    blocks.append({"type": "text", "text": json.dumps(turn)})
    return blocks


def call_usd(model, usage):
    """What one call cost, in dollars, from the usage it reports."""
    p_in, p_out, p_read, p_write = PRICES.get(model) or max(PRICES.values())
    n = lambda k: getattr(usage, k, 0) or 0
    return (n("input_tokens") * p_in + n("output_tokens") * p_out
            + n("cache_read_input_tokens") * p_read
            + n("cache_creation_input_tokens") * p_write) / 1e6


# The house pool's classes and their shares of a day's allowance.
SHARES = {"protagonist": 0.40, "villages": 0.30, "teammates": 0.15, "other": 0.15}
KIND_CLASS = {"protagonist": "protagonist", "villager": "villages", "village": "villages",
              "teammate": "teammates"}
# How far ahead of the clock a day's allowance accrues: a mind may spend
# two hours' worth at once, never the day's.
SMOOTH_LEAD_S = 7200


def pool_class(kind):
    """A pool class from GATES_POOL_CLASS or a character's kind."""
    if POOL_CLASS in SHARES:
        return POOL_CLASS
    return KIND_CLASS.get(str(kind or ""), "other")


def month_key(now):
    return time.strftime("%Y-%m", time.gmtime(now))


def day_key(now):
    return time.strftime("%Y-%m-%d", time.gmtime(now))


def days_left(now):
    """Days left in this UTC month, today included."""
    t = time.gmtime(now)
    return calendar.monthrange(t.tm_year, t.tm_mon)[1] - t.tm_mday + 1


def allowance_today(budget, spent_before_today, now):
    """What the whole pool may spend today: what is left of the month's
    budget at midnight, over the days left."""
    return max(0.0, budget - spent_before_today) / days_left(now)


def class_cap(budget, spent_before_today, cls, now, smooth=True):
    """What class `cls` may have spent by `now` today: its share of today's
    allowance, accrued through the day (two hours ahead of the clock) when
    `smooth`."""
    cap = allowance_today(budget, spent_before_today, now) * SHARES.get(cls, SHARES["other"])
    if smooth:
        into_day = now % 86400
        cap *= min(1.0, (into_day + SMOOTH_LEAD_S) / 86400)
    return cap


def rolled(doc, now):
    """The pool document moved on to `now`: a new month starts at zero, a
    new day notes what was spent before it."""
    doc = dict(doc) if isinstance(doc, dict) else {}
    if doc.get("month") != month_key(now):
        doc = {"month": month_key(now), "spent_usd": 0.0, "by_class": {}}
    if doc.get("day") != day_key(now):
        doc["day"] = day_key(now)
        doc["day_start_usd"] = float(doc.get("spent_usd", 0.0))
        doc["today"] = {}
        doc["councils"] = 0
    return doc


class Pool:
    """The house's shared monthly budget (GATES_POOL): one JSON file every
    house mind on the box reads and charges, under an flock on a lock file
    beside it. Unset, or unreadable, it says yes and keeps nothing: the
    per-agent GATES_CREDIT_FILE still binds."""

    def __init__(self, path, budget, cls, smooth=True, clock=time.time):
        self.path, self.budget, self.cls, self.smooth, self.clock = path, budget, cls, smooth, clock

    def _edit(self, fn, write=True):
        import fcntl  # noqa: PLC0415 — POSIX only, and only with a pool
        with open(self.path + ".lock", "a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX if write else fcntl.LOCK_SH)
            try:
                with open(self.path) as f:
                    doc = json.load(f)
            except (OSError, ValueError):
                doc = {}
            doc = rolled(doc, self.clock())
            doc["budget_usd"] = self.budget
            result = fn(doc)
            if not write:
                return result
            tmp = f"{self.path}.{os.getpid()}.tmp"
            with open(tmp, "w") as f:
                json.dump(doc, f, separators=(",", ":"))
            os.replace(tmp, self.path)
            return result

    def may(self, stage):
        """May this class make a `stage` call now: "tactician", "strategist"
        (only under STRATEGIST_HEADROOM of its share) or "council" (a
        strategist call, and fewer than PLANNER_PER_DAY today)?"""
        if not self.path:
            return True

        def check(doc):
            now = self.clock()
            cap = class_cap(self.budget, doc["day_start_usd"], self.cls, now, self.smooth)
            used = float(doc["today"].get(self.cls, 0.0))
            if stage == "tactician":
                return used < cap
            if stage == "council" and int(doc.get("councils", 0)) >= PLANNER_PER_DAY:
                return False
            return used < cap * STRATEGIST_HEADROOM
        try:
            return self._edit(check, write=False)
        except OSError as e:
            print(f"pool: {e}", file=sys.stderr)
            return True

    def charge(self, usd, council=False):
        if not self.path:
            return

        def add(doc):
            doc["spent_usd"] = float(doc.get("spent_usd", 0.0)) + usd
            doc["today"][self.cls] = float(doc["today"].get(self.cls, 0.0)) + usd
            doc["by_class"][self.cls] = float(doc["by_class"].get(self.cls, 0.0)) + usd
            if council:
                doc["councils"] = int(doc.get("councils", 0)) + 1
        try:
            self._edit(add)
        except OSError as e:
            print(f"pool: {e}", file=sys.stderr)


class Spend:
    """The running cost of this mind's calls (GATES_SPEND), the credit it
    may not pass (GATES_CREDIT_FILE) and the house pool it draws on
    (GATES_POOL). None set: it counts and never stops."""

    def __init__(self, pool=None):
        self.usd = 0.0
        self.model = MODEL
        self.pool = pool or Pool("", 0, "other")
        self.lock = threading.Lock()
        try:
            with open(SPEND) as f:
                for line in f:
                    try:
                        self.usd += float(json.loads(line).get("usd", 0))
                    except (ValueError, AttributeError, TypeError):
                        pass
        except OSError:
            pass

    def out(self):
        """True once the spend has reached the credit. An unreadable credit
        file is no credit: it spends nothing rather than guess."""
        if not CREDIT_FILE:
            return False
        try:
            with open(CREDIT_FILE) as f:
                return self.usd >= float(f.read().strip())
        except (OSError, ValueError):
            return True

    def bill(self, model, usage, role="tactician"):
        cost = call_usd(model, usage)
        with self.lock:
            self.usd += cost
            if role == "tactician":
                # The model that answered, which a server-side fallback may change.
                self.model = model
        self.pool.charge(cost, council=role == "council")
        if not SPEND:
            return
        n = lambda k: getattr(usage, k, 0) or 0
        row = {"t": round(time.time(), 2), "model": model, "usd": round(cost, 8),
               "in": n("input_tokens"), "out": n("output_tokens"),
               "cache_read": n("cache_read_input_tokens"),
               "cache_write": n("cache_creation_input_tokens")}
        if role != "tactician":
            row["for"] = role
        with self.lock:
            try:
                with open(SPEND, "a") as f:
                    f.write(json.dumps(row) + "\n")
            except OSError as e:
                print(f"spend: {e}", file=sys.stderr)


def ask_api(client, system, blocks, out_schema, spend, model=None, effort="low", role="tactician"):
    model = model or MODEL
    extra = {}
    if model in FALLBACK_MODELS:
        extra = {"betas": ["server-side-fallback-2026-07-01"], "fallbacks": "default"}
    response = client.beta.messages.create(
        model=model,
        max_tokens=4096 if role == "tactician" else 8192,
        # Cached: a mind's rules and character are the same every turn.
        system=[{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        messages=[{"role": "user", "content": blocks}],
        output_config={
            "effort": effort,
            "format": {"type": "json_schema", "schema": out_schema},
        },
        **extra,
    )
    # Billed before it is judged: a refused or cut-off answer still cost.
    spend.bill(response.model or model, response.usage, role)
    if response.stop_reason in ("refusal", "max_tokens"):
        raise RuntimeError(f"model stopped: {response.stop_reason}")
    text = next(b.text for b in response.content if b.type == "text")
    return json.loads(text)


# The unmodified Claude Code CLI, signed in the way its owner signed it in.
# This program never reads, stores or forwards a credential: it only removes
# API keys from the child's environment, so the plan login is what is used.
WORKDIR = tempfile.mkdtemp(prefix="gates-claude-")
CLI_ENV = {k: v for k, v in os.environ.items()
           if k not in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "DISCORD_WEBHOOK_URL")}


def ask_claude_code(system, blocks, out_schema, model=None, effort="low", timeout=None):
    cmd = [
        "claude", "-p", "--json-schema", json.dumps(out_schema),
        "--system-prompt", system, "--model", model or MODEL, "--effort", effort,
        "--tools", "", "--disallowedTools", "mcp__*", "--strict-mcp-config",
        "--safe-mode", "--disable-slash-commands", "--no-session-persistence",
        "--permission-prompts", "none",
    ]
    if any(b["type"] == "image" for b in blocks):
        # A picture rides Claude Code's streaming input: one user message.
        cmd += ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose"]
        stdin = json.dumps({"type": "user", "message": {"role": "user", "content": blocks}}) + "\n"
    else:
        cmd += ["--output-format", "json"]
        stdin = blocks[-1]["text"]
    run = subprocess.run(cmd, input=stdin, capture_output=True, text=True,
                         timeout=timeout or TIMEOUT_S, env=CLI_ENV, cwd=WORKDIR)
    result = None
    for line in run.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "result":
            result = event
    if not result or result.get("subtype") != "success" or "structured_output" not in result:
        detail = (result or {}).get("result") or run.stderr.strip()[-200:] or f"exit {run.returncode}"
        raise RuntimeError(f"claude -p gave no answer: {detail}")
    return result["structured_output"]


def post_share(caption, jpeg):
    """The frame and its caption to the owner's Discord channel. Model text
    never pings anyone: `allowed_mentions` parses nothing."""
    boundary = uuid.uuid4().hex
    payload = json.dumps({"content": caption, "allowed_mentions": {"parse": []}})
    body = b"".join([
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n"
        f"Content-Type: application/json\r\n\r\n{payload}\r\n".encode(),
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; "
        f"filename=\"view.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".encode(),
        jpeg,
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(WEBHOOK_URL, data=body, method="POST", headers={
        "Content-Type": f"multipart/form-data; boundary={boundary}",
        "User-Agent": "gates-claude-agent",
    })
    with urllib.request.urlopen(req, timeout=5) as r:
        return r.status


def label(v):
    """A harness or model label as the platform takes one (1-48 of
    [A-Za-z0-9 ._:/@+-]), or None."""
    return re.sub(r"[^A-Za-z0-9 ._:/@+-]", "", str(v or "")).strip()[:48].strip() or None


def journal(request, goal, reason, say, to, speak="", harness=None, model=None,
            thought="", strategy=None):
    """One line in GATES_JOURNAL. A journal that cannot be written costs a log
    line, never a decision."""
    obs = request.get("observation") or {}
    character = request.get("character") if isinstance(request.get("character"), dict) else {}
    row = {
        "t": round(time.time(), 2),
        "goal": goal,
        "reason": reason,
        "thought": thought,
        "say": say,
        "to": to,
        "speak": speak,
        "heard": [{"from": h["from"], "agent": h.get("agent") is True,
                   "friend": h.get("friend", False),
                   "channel": h.get("channel", "local"), "text": h["text"],
                   "voice": h.get("voice", False)}
                  for h in request.get("heard", []) if not h.get("me")],
        "friend": request.get("friend"),
        "obs": {**{k: obs[k] for k in JOURNAL_OBS if k in obs},
                **({"players": obs["in_view"]["players"]}
                   if isinstance(obs.get("in_view"), dict) and "players" in obs["in_view"] else {})},
        "harness": label(harness),
        "model": label(model),
        "answered_by": "harness",
    }
    if strategy:
        row["strategy"] = strategy
    if character.get("name"):
        row["name"] = character["name"]
        row["kind"] = character.get("kind")
    try:
        with open(JOURNAL, "a") as f:
            f.write(json.dumps(row) + "\n")
    except OSError as e:
        print(f"journal: {e}", file=sys.stderr)


def journal_digest(path, n=DIGEST_TURNS):
    """The last `n` turns of a journal as the digest the strategist reads,
    so a restarted mind remembers what it was doing."""
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - (1 << 16)))
            lines = f.read().splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if isinstance(r, dict) and r.get("goal"):
            out.append(digest_row(r.get("t") or 0, r.get("goal"), r.get("reason"), r.get("say"),
                                  r.get("to"), [h for h in r.get("heard") or [] if isinstance(h, dict)],
                                  r.get("obs") or {}))
    return out


def digest_row(t, goal, reason, say, to, heard, obs):
    """One turn as the strategist reads it: short, and team chat kept (it
    is the mind's own)."""
    row = {"t": round(float(t)), "goal": goal, "why": one_line(reason, REASON_CHARS)}
    if say:
        row["said"] = f"[{to or 'global'}] {one_line(say, 200)}"
    if heard:
        row["heard"] = [f"{one_line(h.get('from'), 24)} [{h.get('channel', 'local')}]: "
                        f"{one_line(h.get('text'), 160)}" for h in heard[-4:]]
    for k in ("health", "deaths", "last_fight", "threats"):
        if k in obs:
            row[k] = obs[k]
    return row


def pack_counts(observation):
    out = {}
    for i in ((observation or {}).get("pack") or {}).get("items") or []:
        if isinstance(i, dict) and isinstance(i.get("name"), str):
            out[i["name"]] = out.get(i["name"], 0) + int(i.get("count") or 1)
    return out


class Triggers:
    """What makes the strategist think before its timer: the moments a
    person would stop and rethink."""

    def __init__(self):
        self.first = True
        self.deaths = None
        self.attacked = False
        self.people = set()
        self.enemies_seen = {}
        self.friend_on = False
        self.plan = None

    def check(self, request, now):
        """The reasons this request is one to rethink on, maybe none."""
        why = []
        obs = request.get("observation") or {}
        deaths = obs.get("deaths")
        if isinstance(deaths, int):
            if self.deaths is not None and deaths > self.deaths:
                why.append("you died")
            self.deaths = deaths
        home = obs.get("home") if isinstance(obs.get("home"), dict) else {}
        attacked = bool((request.get("team") or {}).get("home_attacked") or home.get("under_attack"))
        if attacked and not self.attacked:
            why.append("your home is under attack")
        self.attacked = attacked
        people = [p for p in (request.get("memory") or {}).get("people") or [] if isinstance(p, dict)]
        names = {p.get("name") for p in people if p.get("name")}
        new = names - self.people
        if new and not self.first:
            why.append("someone new: " + ", ".join(sorted(new))[:120])
        self.people |= names
        for p in people:
            if p.get("stance") in ("enemy", "rival") and (p.get("seen_ago_s") or 1e9) <= 15:
                if now - self.enemies_seen.get(p["name"], -1e9) > 900:
                    why.append(f"{p['stance']} in sight: {p['name']}")
                self.enemies_seen[p["name"]] = now
        on = bool((request.get("friend") or {}).get("online"))
        if on and not self.friend_on and not self.first:
            why.append("your friend arrived")
        self.friend_on = on
        if self.plan:
            counts = pack_counts(obs)
            if all(s.get("until") and counts.get(s["until"]["item"], 0) >= s["until"]["count"]
                   for s in self.plan):
                why.append("your plan is done")
                self.plan = None
        self.first = False
        return why


class Strategist:
    """The slow half of the mind, on a thread of its own. `consider` starts
    a think when one is due and none is running; `take` hands over what the
    last one decided, once."""

    def __init__(self, ask, brief, persona, pool, every_s=STRATEGIST_EVERY_S, clock=time.monotonic):
        self.ask, self.brief, self.persona, self.pool = ask, brief, persona, pool
        self.every_s, self.clock = every_s, clock
        self.lock = threading.Lock()
        self.thread = None
        self.result = None
        self.last = None          # when the last think started
        self.strategy = None      # what it last decided, echoed while the body does not
        self.set_at = None

    def due(self, why):
        if self.last is None:
            return True
        gap = self.clock() - self.last
        return gap >= self.every_s or (bool(why) and gap >= STRATEGIST_GAP_S)

    def busy(self):
        return self.thread is not None and self.thread.is_alive()

    def consider(self, request, digest, why):
        if self.every_s <= 0 or self.busy() or not self.due(why):
            return False
        if not self.pool.may("strategist"):
            return False
        self.last = self.clock()
        snap = {"request": request, "digest": list(digest), "why": why or ["it is time to take stock"]}
        self.thread = threading.Thread(target=self._run, args=(snap,), daemon=True, name="strategist")
        self.thread.start()
        return True

    def turn(self, snap):
        r = snap["request"]
        strategy = r.get("strategy") or self.echo()
        turn = {
            "why_now": snap["why"],
            "character": r.get("character"),
            "memory": r.get("memory") or {},
            "strategy": strategy,
            "now": {k: r.get(k) for k in ("observation", "team", "friend", "crew", "places")},
            "goals": sorted(plan_goals(r.get("options") or [])),
            "recent": snap["digest"],
        }
        return turn

    def echo(self):
        if not self.strategy:
            return None
        return {**{k: v for k, v in self.strategy.items() if k != "summary"},
                "set_ago_s": int(self.clock() - self.set_at)}

    def _run(self, snap):
        r = snap["request"]
        people = {p.get("name") for p in (r.get("memory") or {}).get("people") or [] if isinstance(p, dict)}
        goals = plan_goals(r.get("options") or [])
        turn = self.turn(snap)
        try:
            system = strategist_system(self.brief, r.get("character"), self.persona)
            answer = self.ask(system, [{"type": "text", "text": json.dumps(turn)}], strategist_schema(),
                              STRATEGIST_MODEL, STRATEGIST_EFFORT, "strategist")
            council = one_line(answer.get("council"), 200)
            if council and PLANNER_MODEL and self.pool.may("council"):
                print(f"war council: {council}", file=sys.stderr)
                turn["council"] = council
                system = strategist_system(self.brief, r.get("character"), self.persona, council=True)
                try:
                    answer = self.ask(system, [{"type": "text", "text": json.dumps(turn)}],
                                      strategist_schema(), PLANNER_MODEL, "high", "council")
                except Exception as e:  # noqa: BLE001 — the strategist's answer stands
                    print(f"war council failed: {e}", file=sys.stderr)
            out = strategy_reply(answer, goals, people)
        except Exception as e:  # noqa: BLE001 — a failed think costs that think
            print(f"strategist: {e}", file=sys.stderr)
            return
        with self.lock:
            self.result = out
            if out.get("strategy"):
                self.strategy = out["strategy"]
                self.set_at = self.clock()
        print("strategist: " + (out.get("thought") or "(no thought)")
              + (f" / project: {out['strategy']['project']}" if out.get("strategy") else "")
              + (f" / {len(out['plan'])}-step plan" if out.get("plan") else ""), file=sys.stderr)

    def take(self):
        with self.lock:
            out, self.result = self.result, None
        return out

    def give_back(self, out):
        """What `take` handed over and no reply carried: the next one does,
        unless a newer think has landed."""
        with self.lock:
            self.result = self.result or out


def main():
    via = "api"
    persona = os.environ.get("GATES_PERSONA", "")
    args = sys.argv[1:]
    usage = "usage: claude_agent.py [--via api|claude-code] [--persona TEXT]"
    while args:
        flag = args.pop(0)
        if flag == "--via" and args and args[0] in ("api", "claude-code"):
            via = args.pop(0)
        elif flag == "--persona" and args:
            persona = args.pop(0)
        else:
            sys.exit(usage)
    persona = " ".join(persona.split())[:600]
    brief = load_brief()
    harness = HARNESS or f"claude_agent.py/{via}"
    pool = Pool(POOL, POOL_USD, pool_class(None), POOL_SMOOTH)
    spend = Spend(pool)
    if via == "api":
        import anthropic
        # No retries: a retry would outlive the body's deadline for this request.
        client = anthropic.Anthropic(timeout=TIMEOUT_S, max_retries=0)
        slow = anthropic.Anthropic(timeout=STRATEGIST_TIMEOUT_S, max_retries=1)
        errors = (anthropic.APIError,)
        ask = lambda system, blocks, out: ask_api(client, system, blocks, out, spend)
        think = lambda system, blocks, out, model, effort, role: ask_api(
            slow, system, blocks, out, spend, model, effort, role)
    else:
        errors = (subprocess.TimeoutExpired, OSError)
        ask = ask_claude_code
        think = lambda system, blocks, out, model, effort, role: ask_claude_code(
            system, blocks, out, model, effort, STRATEGIST_TIMEOUT_S)
    strategist = Strategist(think, brief, persona, pool)
    triggers = Triggers()
    recent = collections.deque(maxlen=10)
    chat_log = collections.deque(maxlen=CHAT_LOG_LINES)
    digest = collections.deque(journal_digest(JOURNAL) if JOURNAL else [], maxlen=DIGEST_TURNS)
    notes = load_notes() if NOTES_FILE else ""
    # Numbers a teammate typed on team chat: the only codes passed on.
    team_numbers = set()
    code_sent = None
    bugs_sent = set()
    shared_at = None
    for line in sys.stdin:
        request = json.loads(line)
        character = request.get("character") if isinstance(request.get("character"), dict) else {}
        if not POOL_CLASS and character.get("kind"):
            pool.cls = pool_class(character["kind"])
        for h in request.get("heard", []):
            entry = {"from": h["from"], "me": h.get("me", False),
                     "channel": h.get("channel", "local"), "text": h["text"]}
            if h.get("voice"):
                entry["voice"] = True
            chat_log.append(entry)
            if not h.get("me"):
                how = "heard (voice)" if h.get("voice") else "heard"
                print(f"{how} {h['from']}: {h['text']}", file=sys.stderr)
                if h.get("channel") == "team":
                    team_numbers.update(re.findall(r"(?<!\d)\d{1,4}(?!\d)", h["text"]))
        if spend.out():
            print(json.dumps({"id": request["id"], "error": "out of credit"}), flush=True)
            continue
        why = triggers.check(request, time.time())
        if strategist.consider(request, digest, why):
            print("strategist thinks: " + "; ".join(why or ["timer"]), file=sys.stderr)
        thought_out = strategist.take()
        # Past its share the house's autopilot plays, unless the strategist
        # has a plan to hand over: one call carries it to the body.
        if not pool.may("tactician") and not thought_out:
            print(json.dumps({"id": request["id"], "error": "house pool: today's share spent"}), flush=True)
            continue
        frame = fetch_frame()
        can_share = bool(frame and WEBHOOK_URL) and (
            shared_at is None or time.monotonic() - shared_at >= SHARE_EVERY_S)
        voice = bool(request.get("voice"))
        system = tactician_system(brief, request, persona, voice, can_share)
        try:
            answer = ask(system, turn_content(request, recent, frame, notes, chat_log, strategist.echo()),
                         schema(can_share, voice))
            goal = answer["goal"]
            if goal not in request["options"] and not walk_ok(goal, request["options"]):
                raise RuntimeError(f"model chose {goal!r}, not on offer")
            reason = one_line(answer["reason"], REASON_CHARS)
            thought = one_line(answer.get("thought"), THOUGHT_CHARS)
            say = clip_say(answer.get("say", ""))
            speak = " ".join(str(answer.get("speak") or "").split())[:SPEAK_CHARS] if voice else ""
            to = answer.get("to") if answer.get("to") in ("team", "local") else "global"
            code = str(answer.get("code") or "").strip()
            bug = str(answer.get("bug") or "").strip()
            bug_kind = answer.get("bug_kind") or "other"
            kept = notes
            notes = " ".join(str(answer.get("notes") or "").split())[:NOTES_CHARS]
            if NOTES_FILE and notes != kept:
                save_notes(notes)
        except errors + (RuntimeError, ValueError, KeyError, TypeError, StopIteration) as e:
            # No goal in the reply: the body counts a failure and backs off.
            # Never a substituted decision (AGENT-MANUAL.md). The strategist's answer
            # waits for the next reply.
            if thought_out:
                strategist.give_back(thought_out)
            print(f"no decision: {e}", file=sys.stderr)
            print(json.dumps({"id": request["id"], "error": str(e)[:200]}), flush=True)
            continue
        recent.append({"goal": goal, "said": say, "to": to, "spoke": speak})
        digest.append(digest_row(time.time(), goal, reason, say, to,
                                 [h for h in request.get("heard", []) if not h.get("me")],
                                 request.get("observation") or {}))
        reply = {"id": request["id"], "goal": goal, "reason": reason}
        if thought_out:
            reply.update({k: v for k, v in thought_out.items() if k != "thought"})
            thought = thought_out.get("thought") or thought
            if thought_out.get("plan"):
                triggers.plan = thought_out["plan"]
        if thought:
            reply["thought"] = thought
        if JOURNAL:
            journal(request, goal, reason, say, to, speak, harness, spend.model, thought,
                    (thought_out or {}).get("strategy"))
        print(f"{goal}: {reason}" + (f" / thinks: {thought}" if thought else "")
              + (f' / says to {to} "{say}"' if say else "")
              + (f' / speaks "{speak}"' if speak else ""), file=sys.stderr)
        if say:
            reply["say"] = say
            reply["to"] = to
        if speak:
            reply["speak"] = speak
        # A code goes on once, and only one a teammate typed on team chat.
        if code and code != code_sent and code in team_numbers:
            reply["code"] = code
            code_sent = code
            print("took the team's lock code", file=sys.stderr)
        # A bug goes on once: the same title again is the same report.
        if bug:
            title, _, body = bug.partition("\n")
            title = " ".join(title.split())[:BUG_TITLE_CHARS]
            if title and title.lower() not in bugs_sent:
                bugs_sent.add(title.lower())
                reply["report"] = {"kind": bug_kind if bug_kind in BUG_KINDS[1:] else "other",
                                   "title": title, "body": body.strip()[:BUG_BODY_CHARS]}
                print(f'reports a bug: "{title}"', file=sys.stderr)
        print(json.dumps(reply), flush=True)
        caption = " ".join(str(answer.get("share") or "").split())[:CAPTION_CHARS]
        if can_share and caption:
            shared_at = time.monotonic()
            try:
                post_share(caption, frame[0])
                print(f'shared: "{caption}"', file=sys.stderr)
            except OSError as e:
                print(f"share failed: {e}", file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, BrokenPipeError):
        pass  # the body is shutting down; its pipes close with it
