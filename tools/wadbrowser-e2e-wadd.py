#!/usr/bin/env python3
"""A fake wadd for tools/wadbrowser-e2e.sh's `key` check: HTTP on a Unix
socket (the browser's WADD_SOCKET), answering what the machine's WadBrowser
asks wadd, so the browser runs as it does on the machine (no signins.json).

    wadbrowser-e2e-wadd.py SOCKET KEYS.json LOGFILE

  GET /v1/keys/web   KEYS.json: [{"id", "name", "sites", "usedBy", "dir"}]
  GET /v1/library    a library of those keys (web keys) and no web apps
  anything else      404, as wadd says it

Each request is a line in LOGFILE ("GET /v1/keys/web").
"""
import json
import os
import socketserver
import sys
from http.server import BaseHTTPRequestHandler

SOCK, KEYS = sys.argv[1], sys.argv[2]
LOG = open(sys.argv[3], "a", buffering=1)


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def address_string(self):
        return "wadd"

    def answer(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        LOG.write(f"GET {self.path}\n")
        keys = json.load(open(KEYS))
        if self.path == "/v1/keys/web":
            self.answer(200, keys)
        elif self.path == "/v1/library":
            lib = [
                {"id": k["id"], "name": k["name"], "kind": "web", "createdAt": 1, "sites": k.get("sites", [])}
                for k in keys
            ]
            self.answer(200, {"keys": lib, "webApps": []})
        else:
            self.answer(404, {"code": "not_found", "message": f"no {self.path} here"})

    def do_POST(self):
        LOG.write(f"POST {self.path}\n")
        self.answer(404, {"code": "not_found", "message": f"no {self.path} here"})


if os.path.exists(SOCK):
    os.unlink(SOCK)
Server(SOCK, H).serve_forever()
