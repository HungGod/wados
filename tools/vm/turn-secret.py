#!/usr/bin/env python3
"""Cloudflare TURN's key for the stream checks (tools/vm/check-stream.sh),
read in place from the person's secrets, and nothing else of them.

    turn-secret.py check    exit 0 when the file has [cloudflare.turn] with a
                            token_id and an api_token (prints nothing of them)
    turn-secret.py toml     the pair as /etc/wados/turn.toml has it, on stdout,
                            which must be a pipe or a file (the check sends it
                            over SSH's stdin): never a terminal
    turn-secret.py mask     stdin to stdout, line by line, with the pair's values
                            replaced: what a check prints goes through it

The file: WADOS_SECRETS, a folder with cloudflare.toml in it (as host/build.sh
takes the folder for its mok/), or the file itself; by default this
checkout's host/secrets (as host/build.sh finds it). Only the [cloudflare.turn]
table's lines are parsed: no other table (the tunnel's tokens) is ever read
into a value, used or printed. Standard library only.
"""

import os
import re
import stat
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT = os.path.normpath(os.path.join(HERE, "..", "..", "host", "secrets"))
HEADER = re.compile(r"^\s*\[\s*([^\]\s]+)\s*\]\s*(#.*)?$")


def path():
    p = os.environ.get("WADOS_SECRETS") or DEFAULT
    return os.path.join(p, "cloudflare.toml") if os.path.isdir(p) else p


def turn():
    """(token_id, api_token), or None."""
    try:
        with open(path(), encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError:
        return None
    mine, inside = [], False
    for line in lines:
        m = HEADER.match(line)
        if m:
            inside = m.group(1) == "cloudflare.turn"
            continue
        if inside:
            mine.append(line)
    try:
        t = tomllib.loads("\n".join(mine))
    except tomllib.TOMLDecodeError:
        return None
    a, b = t.get("token_id"), t.get("api_token")
    if isinstance(a, str) and isinstance(b, str) and a.strip() and b.strip():
        return a.strip(), b.strip()
    return None


def quote(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "check":
        sys.exit(0 if turn() else 1)
    if cmd == "toml":
        mode = os.fstat(sys.stdout.fileno()).st_mode
        if os.isatty(sys.stdout.fileno()) or not (stat.S_ISFIFO(mode) or stat.S_ISREG(mode) or stat.S_ISSOCK(mode)):
            sys.exit("turn-secret.py: toml goes to a pipe or a file, never a terminal")
        t = turn()
        if not t:
            sys.exit("turn-secret.py: no [cloudflare.turn] token_id and api_token in " + path())
        sys.stdout.write(
            "# Cloudflare TURN's key, put here by tools/vm/check-stream.sh for this run only.\n"
            f"[cloudflare.turn]\ntoken_id = {quote(t[0])}\napi_token = {quote(t[1])}\n"
        )
        return
    if cmd == "mask":
        t = turn()
        pairs = [(t[1], "‹TURN api_token›"), (t[0], "‹TURN token_id›")] if t else []
        while True:
            line = sys.stdin.readline()
            if not line:
                break
            for value, name in pairs:
                line = line.replace(value, name)
            sys.stdout.write(line)
            sys.stdout.flush()
        return
    sys.exit(__doc__.strip())


if __name__ == "__main__":
    main()
