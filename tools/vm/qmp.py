#!/usr/bin/env python3
"""QEMU's monitor (QMP) for tools/vm.sh: screenshots, keys, power.

    qmp.py SOCKET shot FILE.png      the screen, as QEMU's display has it
    qmp.py SOCKET keys TEXT          typed, a key at a time (US layout)
    qmp.py SOCKET key COMBO...       e.g. ret, esc, ctrl-alt-f2
    qmp.py SOCKET powerdown          the ACPI power button
    qmp.py SOCKET wakeup             out of suspend (S3)
    qmp.py SOCKET quit               QEMU exits now
    qmp.py SOCKET status             running, paused, shutdown...

Standard library only.
"""

import json
import os
import socket
import sys
import time

# Characters to QEMU's key codes (qcodes), US layout: (qcode, shifted).
PLAIN = {
    " ": "spc", "\n": "ret", "\t": "tab", "-": "minus", "=": "equal",
    "[": "bracket_left", "]": "bracket_right", "\\": "backslash",
    ";": "semicolon", "'": "apostrophe", "`": "grave_accent",
    ",": "comma", ".": "dot", "/": "slash",
}
SHIFTED = {
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
    "*": "8", "(": "9", ")": "0", "_": "minus", "+": "equal",
    "{": "bracket_left", "}": "bracket_right", "|": "backslash",
    ":": "semicolon", '"': "apostrophe", "~": "grave_accent",
    "<": "comma", ">": "dot", "?": "slash",
}
HOLD_MS = 40


def qcodes(ch):
    """The keys pressed together for one character."""
    if "a" <= ch <= "z" or "0" <= ch <= "9":
        return [ch]
    if "A" <= ch <= "Z":
        return ["shift", ch.lower()]
    if ch in PLAIN:
        return [PLAIN[ch]]
    if ch in SHIFTED:
        return ["shift", SHIFTED[ch]]
    raise SystemExit(f"qmp.py: no key for {ch!r}")


class Qmp:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(30)
        self.sock.connect(path)
        self.buf = b""
        self.read()  # the greeting
        self.call("qmp_capabilities")

    def read(self):
        """The next message that isn't an event."""
        while True:
            while b"\n" not in self.buf:
                chunk = self.sock.recv(65536)
                if not chunk:
                    raise SystemExit("qmp.py: QEMU closed the monitor")
                self.buf += chunk
            line, self.buf = self.buf.split(b"\n", 1)
            if not line.strip():
                continue
            msg = json.loads(line)
            if "event" not in msg:
                return msg

    def call(self, cmd, **args):
        msg = {"execute": cmd}
        if args:
            msg["arguments"] = args
        self.sock.sendall(json.dumps(msg).encode() + b"\n")
        r = self.read()
        if "error" in r:
            raise SystemExit(f"qmp.py: {cmd}: {r['error'].get('desc', r['error'])}")
        return r.get("return")

    def keys(self, codes):
        self.call(
            "send-key",
            keys=[{"type": "qcode", "data": c} for c in codes],
            **{"hold-time": HOLD_MS},
        )
        # send-key returns before it lets go of the keys.
        time.sleep((HOLD_MS + 30) / 1000)


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    q = Qmp(argv[1])
    cmd, rest = argv[2], argv[3:]
    if cmd == "shot" and len(rest) == 1:
        out = os.path.abspath(rest[0])
        tmp = out + ".tmp"
        q.call("screendump", filename=tmp, format="png")
        os.replace(tmp, out)
    elif cmd == "keys" and len(rest) == 1:
        for ch in rest[0]:
            q.keys(qcodes(ch))
    elif cmd == "key" and rest:
        for combo in rest:
            q.keys(combo.split("-"))
    elif cmd == "powerdown" and not rest:
        q.call("system_powerdown")
    elif cmd == "wakeup" and not rest:
        q.call("system_wakeup")
    elif cmd == "quit" and not rest:
        try:
            q.call("quit")
        except SystemExit:
            pass  # it went before answering
    elif cmd == "status" and not rest:
        print(q.call("query-status")["status"])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
