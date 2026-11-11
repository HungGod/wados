#!/usr/bin/env python3
"""try-stream.sh's streaming-socket client (Python's standard library, no
Node.js): opens a WebSocket (subprotocol "selkies") to URL, counts the
messages it's sent for SECONDS, and prints

    messages=<how many> closed=<the close code, or none>

The handshake refused (no 101) prints closed=1006, as a browser reports it.

    ws-socket.py ws://127.0.0.1:PORT/ws-GUID/api/websockets?token=KEY [SECONDS]
"""

import base64
import os
import socket
import struct
import sys
import time
from urllib.parse import urlsplit


def read_exact(s: socket.socket, n: int) -> bytes:
    out = b""
    while len(out) < n:
        chunk = s.recv(n - len(out))
        if not chunk:
            raise ConnectionError("closed")
        out += chunk
    return out


def main(url: str, seconds: float) -> str:
    u = urlsplit(url)
    if u.scheme != "ws":
        return "messages=0 closed=1006"
    path = (u.path or "/") + (f"?{u.query}" if u.query else "")
    messages = 0
    deadline = time.monotonic() + seconds
    try:
        s = socket.create_connection((u.hostname, u.port or 80), timeout=max(1.0, seconds))
    except OSError:
        return "messages=0 closed=1006"
    with s:
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall(
            (
                f"GET {path} HTTP/1.1\r\nHost: {u.netloc}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: selkies\r\n\r\n"
            ).encode()
        )
        head = b""
        try:
            while b"\r\n\r\n" not in head:
                chunk = s.recv(4096)
                if not chunk:
                    return "messages=0 closed=1006"
                head += chunk
        except OSError:
            return "messages=0 closed=1006"
        status, _, rest = head.partition(b"\r\n\r\n")
        if b" 101 " not in status.split(b"\r\n", 1)[0] + b" ":
            return "messages=0 closed=1006"
        buf = rest
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return f"messages={messages} closed=none"
            s.settimeout(left)
            try:
                while len(buf) < 2:
                    chunk = s.recv(65536)
                    if not chunk:
                        return f"messages={messages} closed=1006"
                    buf += chunk
                b0, b1 = buf[0], buf[1]
                n, at = b1 & 0x7F, 2
                if n == 126:
                    while len(buf) < 4:
                        buf += read_exact(s, 4 - len(buf))
                    n, at = struct.unpack(">H", buf[2:4])[0], 4
                elif n == 127:
                    while len(buf) < 10:
                        buf += read_exact(s, 10 - len(buf))
                    n, at = struct.unpack(">Q", buf[2:10])[0], 10
                if b1 & 0x80:  # a server never masks; skip the key if one does
                    at += 4
                while len(buf) < at + n:
                    buf += read_exact(s, at + n - len(buf))
                payload, buf = buf[at : at + n], buf[at + n :]
            except socket.timeout:
                return f"messages={messages} closed=none"
            except (ConnectionError, OSError):
                return f"messages={messages} closed=1006"
            op = b0 & 0x0F
            if op == 0x8:  # close: its code, if it sent one
                code = struct.unpack(">H", payload[:2])[0] if len(payload) >= 2 else 1005
                return f"messages={messages} closed={code}"
            if op in (0x0, 0x1, 0x2) and b0 & 0x80:  # a whole message (its last frame)
                messages += 1


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    print(main(sys.argv[1], float(sys.argv[2]) if len(sys.argv) > 2 else 4.0))
