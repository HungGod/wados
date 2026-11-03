#!/usr/bin/env python3
"""VS Code's terminals in tmux (wadspaces-term-attach), so their jobs carry
on while the wadspace moves between the screen and a stream: a "tmux"
terminal profile, the default, and VS Code bringing its terminals back when
it opens again. Only what the user hasn't set themselves, and only in a
settings file that's plain JSON (VS Code takes comments too: then it's left
alone). Run at each desktop start (features/vscode.sh's autostart hook)."""
import json
import os
import sys

PATH = os.path.expanduser("~/.config/Code/User/settings.json")
PROFILE = {"path": "/usr/local/bin/wadspaces-term-attach", "args": ["vscode"], "icon": "terminal-tmux"}
DEFAULTS = {
    "terminal.integrated.defaultProfile.linux": "tmux",
    "terminal.integrated.persistentSessionReviveProcess": "onExitAndWindowClose",
}


def merged(settings):
    """The settings with ours added where they're missing; None: nothing to add."""
    changed = False
    profiles = settings.get("terminal.integrated.profiles.linux")
    if profiles is None:
        settings["terminal.integrated.profiles.linux"] = profiles = {}
    if isinstance(profiles, dict) and "tmux" not in profiles:
        profiles["tmux"] = PROFILE
        changed = True
    for k, v in DEFAULTS.items():
        if k not in settings:
            settings[k] = v
            changed = True
    return settings if changed else None


def main():
    try:
        with open(PATH, encoding="utf-8") as f:
            text = f.read()
    except FileNotFoundError:
        text = ""
    try:
        settings = json.loads(text) if text.strip() else {}
    except ValueError:
        return 0
    if not isinstance(settings, dict):
        return 0
    out = merged(settings)
    if out is None:
        return 0
    os.makedirs(os.path.dirname(PATH), exist_ok=True)
    tmp = PATH + ".wadspaces"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(out, f, indent=4)
        f.write("\n")
    os.replace(tmp, PATH)
    return 0


if __name__ == "__main__":
    sys.exit(main())
