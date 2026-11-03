#!/usr/bin/env python3
"""A wadd state folder holding the spike's populated sample (the library
crates/wad-model/tests/data/populated.json: ten wadspaces, three tags,
keys), for the shell's screenshots and trials:

    tools/seed-library.py <state dir> [library.json]

It writes <state dir>/library.json (all but the wadspaces) and
<state dir>/wadspaces/<id>.json, as wadd's store keeps them. Run wadd on
it with [machine] id = "m-surface" (the sample's machine).
"""
import json
import os
import sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
state = sys.argv[1]
src = sys.argv[2] if len(sys.argv) > 2 else os.path.join(root, "crates/wad-model/tests/data/populated.json")
lib = json.load(open(src))
os.makedirs(os.path.join(state, "wadspaces"), mode=0o700, exist_ok=True)
for w in lib.get("wadspaces", []):
    with open(os.path.join(state, "wadspaces", w["id"] + ".json"), "w") as f:
        json.dump(w, f, indent=2)
rest = dict(lib)
rest["wadspaces"] = []
with open(os.path.join(state, "library.json"), "w") as f:
    json.dump(rest, f, indent=2)
print(f"{len(lib.get('wadspaces', []))} wadspaces into {state}")
