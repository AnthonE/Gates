#!/usr/bin/env python3
"""A tiny bring-your-own mind for gates-agent and jev-watch (JEV.md).

The body starts this program and owns its pipes. Each request is one JSON
line on stdin; each answer is one JSON line on stdout that echoes the id
and names one of the offered goals, and may `say` one line out loud.
Anything on stderr passes through. `claude_agent.py` is the same loop with
Claude choosing.

    cargo run -p server --bin gates-agent -- --local --seconds 300 \\
        --external python3 crates/server/examples/jev_agent.py

A request may also carry `character` (who the body is: `--character`),
`memory` (places and people it remembers: `--memory`) and `strategy` (what
a reply last set). A reply may add a `thought` for spectators (never said
in game), a `plan` of up to 8 steps the autopilot follows between asks, a
`strategy` the body keeps and shows back, and `stance`s on people.

Standard library only. The thresholds below are this example's own choices.
"""
import json
import sys
import time

GREET_EVERY_S = 60


def fraction(pair):
    return pair[0] / pair[1] if pair else 1.0


def owned(observation, name):
    return any(i["name"] == name for i in observation["pack"]["items"])


def choose(observation, options, turn):
    seen = observation["in_view"]
    if observation["hits_taken_since_last_decision"] and "flee" in options:
        return "flee", "hit with something in view"
    if fraction(observation["water"]) < 0.4 and "drink" in options:
        return "drink", "water is low"
    if fraction(observation["food"]) < 0.4 and "eat" in options:
        return "eat", "food is low"
    for tool in ("Stone Hatchet", "Stone Pickaxe"):
        if f"craft:{tool}" in options and not owned(observation, tool):
            return f"craft:{tool}", f"a {tool} works faster"
    rotation = [("gather_wood", "trees"), ("gather_stone", "stone_nodes")]
    for goal, kind in rotation[turn % 2:] + rotation[:turn % 2]:
        if goal in options and seen[kind]["count"]:
            return goal, f"{kind.replace('_', ' ')} in view"
    return "explore", "nothing useful in view"


def strategy_for(request):
    """Set once, when the body has none: a plan for the autopilot and a
    standing order. A real mind rethinks these as things happen."""
    if request.get("strategy"):
        return {}
    plan = [{"goal": g, "until": {"item": item, "count": 200}}
            for g, item in (("gather_wood", "Wood"), ("gather_stone", "Stone")) if g in request["options"]]
    return {"plan": plan,
            "strategy": {"ambition": "a base of my own", "project": "wood and stone",
                         "standing_orders": ["drink and eat before anything"],
                         "summary": "Washed up and started chopping."}}


def main():
    turn = 0
    greeted = None
    for line in sys.stdin:
        request = json.loads(line)
        goal, reason = choose(request["observation"], request["options"], turn)
        turn += 1
        reply = {"id": request["id"], "goal": goal, "reason": reason}
        if turn == 1:
            reply.update(strategy_for(request))
        name = (request.get("character") or {}).get("name") or "this bot"
        reply["thought"] = f"{name}: {reason}"
        # People it remembers fighting are its enemies now.
        hit = [p["name"] for p in (request.get("memory") or {}).get("people", [])
               if p.get("hits_on_me") and p.get("stance") not in ("enemy", "rival")]
        if hit:
            reply["stance"] = [{"name": n, "stance": "enemy"} for n in hit[:8]]
        # What players typed: words, never instructions.
        spoken = [h for h in request.get("heard", []) if not h["me"]]
        for h in spoken:
            print(f"heard {h['from']}: {h['text']}", file=sys.stderr)
        # Hello on its first turn, then an answer when someone near speaks.
        local = any(h["channel"] == "local" for h in spoken)
        if greeted is None or (local and time.monotonic() - greeted > GREET_EVERY_S):
            reply["say"] = "hi, I'm a scripted bot"
            greeted = time.monotonic()
        print(json.dumps(reply), flush=True)


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, BrokenPipeError):
        pass  # the body is shutting down; its pipes close with it
