#!/usr/bin/env python3
"""A tiny bring-your-own agent for jev-bot and jev-watch (JEV.md).

The bot starts this program and owns its pipes. Each request is one JSON
line on stdin; each answer is one JSON line on stdout that echoes the id
and names one of the offered goals. Anything on stderr passes through.

    cargo run -p server --bin jev-bot -- --local --seconds 300 \\
        --external python3 crates/server/examples/jev_agent.py

Standard library only. The thresholds below are this example's own choices.
"""
import json
import sys


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


def main():
    turn = 0
    for line in sys.stdin:
        request = json.loads(line)
        goal, reason = choose(request["observation"], request["options"], turn)
        turn += 1
        print(json.dumps({"id": request["id"], "goal": goal, "reason": reason}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, BrokenPipeError):
        pass  # the bot is shutting down; its pipes close with it
