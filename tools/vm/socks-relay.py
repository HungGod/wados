#!/usr/bin/env python3
"""A local port onto the tailnet, through a SOCKS5 proxy (the test tailnet's
device, tools/tailnet.sh socks), for tools/vm/check-tailnet.sh's viewer:

    socks-relay.py SOCKS_HOST:PORT DEST_IP:PORT

Listens on 127.0.0.1 (a port of its own, printed on the first line of
stdout), and carries each connection to DEST through the proxy. A browser
on the laptop then opens http://localhost:<port>/: a secure context (Selkies'
WebCodecs, the Secure cookie) without a flag, while every byte still crosses
the tailnet as the device. Runs until it's killed. Standard library only.
"""

import socket
import struct
import sys
import threading


def pipe(a, b):
    try:
        while True:
            data = a.recv(65536)
            if not data:
                break
            b.sendall(data)
    except OSError:
        pass
    finally:
        for s in (a, b):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def connect(proxy, dest):
    s = socket.create_connection(proxy, timeout=20)
    s.sendall(b"\x05\x01\x00")
    if s.recv(2) != b"\x05\x00":
        raise OSError("the SOCKS5 proxy wants authentication")
    s.sendall(b"\x05\x01\x00\x01" + socket.inet_aton(dest[0]) + struct.pack(">H", dest[1]))
    reply = s.recv(10)
    if len(reply) < 2 or reply[1] != 0:
        raise OSError(f"SOCKS5 connect refused ({reply[1] if len(reply) > 1 else '?'})")
    s.settimeout(None)
    return s


def serve(client, proxy, dest):
    try:
        upstream = connect(proxy, dest)
    except OSError as e:
        print(f"relay: {e}", file=sys.stderr, flush=True)
        client.close()
        return
    threading.Thread(target=pipe, args=(client, upstream), daemon=True).start()
    pipe(upstream, client)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__.strip())
    ph, pp = sys.argv[1].rsplit(":", 1)
    dh, dp = sys.argv[2].rsplit(":", 1)
    proxy, dest = (ph, int(pp)), (dh, int(dp))
    ls = socket.socket()
    ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    ls.bind(("127.0.0.1", 0))
    ls.listen(64)
    print(ls.getsockname()[1], flush=True)
    while True:
        c, _ = ls.accept()
        threading.Thread(target=serve, args=(c, proxy, dest), daemon=True).start()


if __name__ == "__main__":
    main()
