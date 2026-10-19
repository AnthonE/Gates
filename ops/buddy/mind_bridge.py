"""The door an owner's own harness plays its agent through.

An agent whose brain is its owner's (`brain: own` on elopros.com/buddy.html)
runs `jev-bot … --mcp`: an MCP server on its own stdin and stdout
(`crates/server/src/mcp.rs`), held by `runner.py`. This serves MCP's
streamable HTTP on 127.0.0.1:BUDDY_MIND_PORT, which nginx fronts as
https://game.elopros.com/gates/mind/<id>, and passes each message to that
stdio and its answer back. We hold no key of the harness's and pay for none
of its thinking.

    claude mcp add --transport http gates https://game.elopros.com/gates/mind/<id> \\
        --header "Authorization: Bearer <token>"

An app that adds a connector by its URL alone (a chat app's "custom
connector", which sends no header of ours) puts the token in the path
instead: https://game.elopros.com/gates/mind/<id>/<token>. The same token,
checked the same way; nginx keeps that path out of its access log.

The token is made on the buddy page and shown once; the platform keeps only
its sha256, which comes down the feed as `token_sha`. Stateless: every POST
carries one message (or a batch) and gets its answer as JSON. A notification
gets 202; GET and DELETE get 405, so a client opens no stream.

The body comes onto the island on the first tool call and leaves when its
harness has been quiet for BUDDY_OWN_IDLE_S. `initialize` and `tools/list`
are answered from the last body's answers when there is no body, so opening
a harness that never plays brings nobody in. Each answered `act` goes in the
agent's journal, the same line `claude_agent.py` writes, so the page's live
line and the daily recap work for an own brain too. The line names the
harness its `initialize` said it was (`clientInfo`); the turns the autopilot
plays for a quiet harness never come through here.
"""
import hashlib
import hmac
import json
import queue
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PATH = re.compile(r"^/gates/mind/([0-9a-f]{10})(?:/([0-9A-Za-z_-]{16,128}))?/?$")
MAX_BODY = 64 * 1024
RATE = 10.0          # messages a second per agent, with a burst of BURST
BURST = 20.0
IN_FLIGHT = 4        # requests per agent at once; a `look` holds one for 25 s
START_WAIT_S = 40.0  # how long a call waits for its body to come in
ANSWER_S = 60.0      # how long a message waits for the body's answer
REFRESH_S = 10.0     # an unknown token asks for a fresh feed at most this often
# What the journal keeps of an observation (`claude_agent.py` JOURNAL_OBS).
JOURNAL_OBS = ("health", "food", "water", "threats", "last_fight", "last_goal",
               "current_goal", "deaths", "respawns")


LABEL = re.compile(r"[^A-Za-z0-9 ._:/@+-]")
LABEL_MAX = 48


def label(v):
    """A harness or model label as the platform takes one (1-48 of
    [A-Za-z0-9 ._:/@+-]), or None."""
    if not isinstance(v, str):
        return None
    return LABEL.sub("", v).strip()[:LABEL_MAX].strip() or None


def harness_of(info):
    """`name/version` from an `initialize`'s clientInfo, as a label."""
    if not isinstance(info, dict) or not isinstance(info.get("name"), str):
        return None
    version = info.get("version")
    return label(f"{info['name']}/{version}" if isinstance(version, str) and version else info["name"])


def _clip(v, n):
    """A short single line of printable text, or None."""
    if not isinstance(v, str):
        return None
    v = " ".join("".join(c for c in v if c.isprintable()).split())
    return v[:n] or None


def _tool_text(body, is_error):
    return {"content": [{"type": "text", "text": body}], "isError": is_error}


def _answer(mid, result=None, error=None):
    out = {"jsonrpc": "2.0", "id": mid}
    if error is not None:
        out["error"] = error
    else:
        out["result"] = result
    return out


class Mind:
    """One agent's `jev-bot --mcp` and the pipe to it: one message at a time,
    each under an id of ours so a late answer to a dropped call is never
    taken for the next one's."""

    def __init__(self, proc):
        self.proc = proc
        self.lines = queue.Queue(maxsize=256)
        self.lock = threading.Lock()
        self.next_id = 0
        threading.Thread(target=self._read, daemon=True, name="mind-read").start()

    def _read(self):
        try:
            for line in self.proc.stdout:
                try:
                    self.lines.put(line, timeout=5)
                except queue.Full:
                    pass  # nobody is reading: the answer was to a dropped call
        except (OSError, ValueError):
            pass
        self.lines.put(None)

    def alive(self):
        return self.proc.poll() is None

    def call(self, msg, timeout=ANSWER_S):
        """Send one message. A request's answer comes back with the caller's
        id; a notification's is None."""
        expect = msg.get("id") is not None
        # One budget for the wait in line and the answer, so a call queued
        # behind another body's long `look` doesn't take twice as long.
        deadline = time.monotonic() + timeout
        if not self.lock.acquire(timeout=timeout):
            return _answer(msg.get("id"), error={"code": -32001, "message": "your agent is busy; try again"})
        try:
            out = dict(msg)
            if expect:
                self.next_id += 1
                out["id"] = self.next_id
            line = (json.dumps(out, separators=(",", ":")) + "\n").encode()
            if len(line) > MAX_BODY:
                return _answer(msg.get("id"), error={"code": -32600, "message": "message too long"})
            try:
                self.proc.stdin.write(line)
                self.proc.stdin.flush()
            except (OSError, ValueError):
                return _answer(msg.get("id"), error={"code": -32002, "message": "your agent left the island; call again"})
            if not expect:
                return None
            while True:
                left = deadline - time.monotonic()
                if left <= 0:
                    return _answer(msg["id"], error={"code": -32001, "message": "your agent did not answer in time"})
                try:
                    raw = self.lines.get(timeout=left)
                except queue.Empty:
                    continue
                if raw is None:
                    self.lines.put(None)
                    return _answer(msg["id"], error={"code": -32002, "message": "your agent left the island; call again"})
                try:
                    got = json.loads(raw)
                except ValueError:
                    continue
                if isinstance(got, dict) and got.get("id") == out["id"]:
                    got["id"] = msg["id"]
                    return got
        finally:
            self.lock.release()


class Bridge:
    """Who may call, which bodies are up, and when each harness last called.
    The runner owns the bodies: it reads `wanted()`, starts and stops them,
    and `attach`es and `detach`es them here."""

    def __init__(self, journal_path, idle_s=1800.0, start_wait_s=START_WAIT_S):
        self.journal_path = journal_path
        self.idle_s = idle_s
        self.start_wait_s = start_wait_s
        self.lock = threading.Lock()
        self.agents = {}     # id -> the feed's row, own-brain agents only
        self.minds = {}      # id -> Mind
        self.seen = {}       # id -> when its harness last called
        self.asked = {}      # id -> when a tool call last asked for its body
        self.looks = {}      # id -> the last decision its harness was shown
        self.harness = {}    # id -> the harness its last initialize named
        self.cache = {}      # ("initialize", version) / ("tools/list",) -> result
        self.buckets = {}    # id -> (tokens, at)
        self.flight = {}     # id -> requests in progress
        self.fed_at = 0.0
        self.refresh_at = 0.0
        self.wake = threading.Event()   # the runner ticks (and reads the feed) now

    # ── the runner's side ────────────────────────────────────────────────
    def set_agents(self, agents):
        with self.lock:
            self.agents = {i: a for i, a in agents.items()
                           if a.get("brain") == "own" and a.get("token_sha")}
            self.fed_at = time.time()

    def calls(self):
        """When each harness last asked for its body: what the runner keeps
        across a restart, so a deploy does not take every resting body off
        the island until its harness happens to call again."""
        with self.lock:
            return dict(self.asked)

    def recall(self, asked, now=None):
        """Take back what `calls` kept, inside the idle window."""
        now = time.time() if now is None else now
        with self.lock:
            for i, t in asked.items():
                if isinstance(t, (int, float)) and now - t < self.idle_s:
                    self.asked[str(i)] = max(float(t), self.asked.get(str(i), 0.0))

    def wanted(self, now=None):
        """The agents a harness has called for within the idle window."""
        now = time.time() if now is None else now
        with self.lock:
            return {i for i, t in self.asked.items() if now - t < self.idle_s and i in self.agents}

    def attach(self, aid, proc):
        with self.lock:
            self.minds[aid] = Mind(proc)

    def detach(self, aid):
        with self.lock:
            self.minds.pop(aid, None)
            self.looks.pop(aid, None)

    # ── the harness's side ───────────────────────────────────────────────
    def known(self, aid, auth):
        """True when `auth` is this agent's bearer token."""
        m = re.match(r"^Bearer\s+([0-9A-Za-z_-]{16,128})\s*$", auth or "")
        if not m:
            return False
        want = (self.agents.get(aid) or {}).get("token_sha") or ""
        got = hashlib.sha256(m.group(1).encode()).hexdigest()
        return bool(want) and hmac.compare_digest(want, got)

    def auth(self, aid, auth):
        """Check the token, reading a fresh feed once when it is not known yet:
        a token made a moment ago is pasted straight into a harness."""
        with self.lock:
            if self.known(aid, auth):
                return True
            if time.time() - self.refresh_at < REFRESH_S:
                return False
            self.refresh_at = time.time()
            asked_at = self.fed_at
        self.wake.set()
        deadline = time.time() + 8.0
        while time.time() < deadline:
            time.sleep(0.2)
            with self.lock:
                if self.fed_at > asked_at:
                    return self.known(aid, auth)
        return False

    def admit(self, aid):
        """A token bucket and an in-flight cap per agent."""
        now = time.monotonic()
        with self.lock:
            tokens, at = self.buckets.get(aid, (BURST, now))
            tokens = min(BURST, tokens + (now - at) * RATE)
            if tokens < 1.0 or self.flight.get(aid, 0) >= IN_FLIGHT:
                self.buckets[aid] = (tokens, now)
                return False
            self.buckets[aid] = (tokens - 1.0, now)
            self.flight[aid] = self.flight.get(aid, 0) + 1
            return True

    def done(self, aid):
        with self.lock:
            self.flight[aid] = max(0, self.flight.get(aid, 0) - 1)

    def _mind(self, aid, bring):
        """The agent's body, bringing it in (and waiting) when `bring`."""
        with self.lock:
            if bring:
                self.asked[aid] = time.time()
            m = self.minds.get(aid)
            if m and m.alive():
                return m
            if not bring:
                return None
        self.wake.set()
        deadline = time.monotonic() + self.start_wait_s
        while time.monotonic() < deadline:
            time.sleep(0.2)
            with self.lock:
                m = self.minds.get(aid)
            if m and m.alive():
                return m
        return None

    def handle(self, aid, msg):
        """One POSTed body (a message or a batch) in; its answer out, or None."""
        with self.lock:
            self.seen[aid] = time.time()
        if isinstance(msg, list):
            if not msg:
                return _answer(None, error={"code": -32600, "message": "empty batch"})
            out = [r for r in (self.one(aid, m) for m in msg[:16]) if r is not None]
            return out or None
        return self.one(aid, msg)

    def one(self, aid, msg):
        if not isinstance(msg, dict) or not isinstance(msg.get("method", ""), str):
            return _answer(None, error={"code": -32600, "message": "invalid request"})
        method = msg.get("method")
        mid = msg.get("id")
        if method is None:
            return None  # a response to nothing we asked
        if mid is None:
            # A notification: the body's if it is up, nobody's if not.
            m = self._mind(aid, bring=False)
            if m:
                m.call(msg)
            return None
        if method == "ping":
            return _answer(mid, {})
        params = msg.get("params") if isinstance(msg.get("params"), dict) else {}
        if method == "initialize":
            key = ("initialize", str(params.get("protocolVersion") or "")[:24])
            named = harness_of(params.get("clientInfo"))
            if named:
                with self.lock:
                    self.harness[aid] = named
        elif method == "tools/list":
            key = ("tools/list",)
        else:
            key = None
        if key is not None:
            m = self._mind(aid, bring=False)
            with self.lock:
                hit = self.cache.get(key)
            if m is None and hit is not None:
                return _answer(mid, hit)
            m = m or self._mind(aid, bring=True)
            if m is None:
                return _answer(mid, error={"code": -32002, "message": "your agent is coming onto the island; try again in a minute"})
            got = m.call(msg)
            if isinstance(got, dict) and isinstance(got.get("result"), dict):
                with self.lock:
                    if len(self.cache) < 16:
                        self.cache[key] = got["result"]
            return got
        if method != "tools/call":
            m = self._mind(aid, bring=False)
            if m is None:
                return _answer(mid, error={"code": -32601, "message": f"method not found: {method}"[:120]})
            return m.call(msg)
        m = self._mind(aid, bring=True)
        if m is None:
            return _answer(mid, _tool_text(
                "Your agent is still coming onto the island (or waiting for a free spot). Call look again in a minute.",
                True))
        got = m.call(msg)
        self._note(aid, params, got)
        return got

    def _note(self, aid, params, got):
        """Remember the decision `look` showed; journal the one `act` took."""
        res = got.get("result") if isinstance(got, dict) else None
        if not isinstance(res, dict) or res.get("isError"):
            return
        name = params.get("name")
        args = params.get("arguments") if isinstance(params.get("arguments"), dict) else {}
        if name == "look":
            try:
                seen = json.loads(res["content"][0]["text"])
            except (KeyError, IndexError, TypeError, ValueError):
                return
            if isinstance(seen, dict) and "decision_id" in seen:
                with self.lock:
                    self.looks[aid] = seen
        elif name == "act":
            with self.lock:
                seen = self.looks.pop(aid, None) or {}
                harness = self.harness.get(aid)
            obs = seen.get("observation") if isinstance(seen.get("observation"), dict) else {}
            heard = seen.get("heard") if isinstance(seen.get("heard"), list) else []
            # Missing is global, as jev-bot reads it (`external.rs`).
            to = args.get("to") if args.get("to") in ("global", "local", "team") else "global"
            row = {
                "t": round(time.time(), 2),
                "goal": _clip(args.get("goal"), 32),
                "reason": _clip(args.get("reason"), 96),
                "say": _clip(args.get("say"), 48),
                "to": to if args.get("say") else None,
                "heard": [{"from": h.get("from"), "agent": h.get("agent") is True,
                           "friend": bool(h.get("friend")),
                           "channel": h.get("channel", "local"), "text": h.get("text")}
                          for h in heard if isinstance(h, dict) and not h.get("me")][:20],
                "friend": seen.get("friend"),
                "obs": {**{k: obs[k] for k in JOURNAL_OBS if k in obs},
                        **({"players": obs["in_view"]["players"]}
                           if isinstance(obs.get("in_view"), dict) and "players" in obs["in_view"] else {})},
                "brain": "own",
                "harness": harness,
                # A harness names no model; the platform face may.
                "model": None,
                "answered_by": "harness",
            }
            try:
                with open(self.journal_path(aid), "a") as f:
                    f.write(json.dumps(row) + "\n")
            except OSError:
                pass


def handler_for(bridge):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "gates-mind"
        sys_version = ""
        timeout = 30  # a slow client's socket, not a slow body's answer

        def log_message(self, *_):
            pass

        def _send(self, code, body=None, headers=()):
            data = b"" if body is None else json.dumps(body, separators=(",", ":")).encode()
            self.send_response(code)
            if body is not None:
                self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            for k, v in headers:
                self.send_header(k, v)
            try:
                self.end_headers()
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                # The harness (or nginx's 90 s) gave up before the answer:
                # its next call gets the next one. Not worth a traceback.
                self.close_connection = True

        def _refuse(self):
            self._send(405, {"error": "POST only"}, [("Allow", "POST")])

        do_GET = do_DELETE = do_PUT = do_PATCH = _refuse

        def do_POST(self):
            m = PATH.match(self.path.split("?", 1)[0])
            if not m:
                return self._send(404, {"error": "no such agent"})
            aid, in_path = m.group(1), m.group(2)
            auth = f"Bearer {in_path}" if in_path else self.headers.get("Authorization")
            if not bridge.auth(aid, auth):
                return self._send(401, {"error": "make a token for this agent on elopros.com/buddy.html"},
                                  [("WWW-Authenticate", 'Bearer realm="gates"')])
            try:
                n = int(self.headers.get("Content-Length") or "-1")
            except ValueError:
                n = -1
            if n < 0:
                return self._send(411, {"error": "Content-Length required"})
            if n > MAX_BODY:
                return self._send(413, {"error": "message too long"})
            body = self.rfile.read(n)
            if not bridge.admit(aid):
                return self._send(429, {"error": "slow down"}, [("Retry-After", "1")])
            try:
                try:
                    msg = json.loads(body)
                except ValueError:
                    return self._send(400, _answer(None, error={"code": -32700, "message": "parse error"}))
                out = bridge.handle(aid, msg)
            finally:
                bridge.done(aid)
            if out is None:
                return self._send(202)
            self._send(200, out)

    return Handler


def serve(bridge, port, host="127.0.0.1"):
    """Serve the bridge from a thread; returns the server."""
    httpd = ThreadingHTTPServer((host, port), handler_for(bridge))
    httpd.daemon_threads = True
    threading.Thread(target=httpd.serve_forever, daemon=True, name="mind-http").start()
    return httpd
