#!/usr/bin/env python3
"""A mock of the Cloudflare API calls the installer makes (tools/vm/check-install.sh).
Nothing reaches Cloudflare. GET /__seen says what it was asked.

    cloudflare-mock.py PORT
"""

import json
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

seen = []


class Mock(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, result, code=200):
        body = json.dumps({"success": code == 200, "errors": [], "result": result}).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self):
        url = urlparse(self.path)
        size = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(size)) if size else None
        if url.path == "/__seen":
            body = json.dumps(seen).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        seen.append({"method": self.command, "path": url.path, "body": body,
                     "auth": self.headers.get("Authorization", "")})
        q = parse_qs(url.query)
        if self.command == "GET" and url.path == "/zones":
            return self.reply([{"id": "z1", "account": {"id": "a1"}}] if q.get("name") == ["wadspaces.com"] else [])
        if self.command == "GET" and url.path == "/accounts/a1/cfd_tunnel":
            return self.reply([])
        if self.command == "POST" and url.path == "/accounts/a1/cfd_tunnel":
            return self.reply({"id": "t1", "token": "fake-machine-token"})
        if self.command == "PUT" and re.fullmatch(r"/accounts/a1/cfd_tunnel/t1/configurations", url.path):
            return self.reply({})
        if self.command == "GET" and url.path == "/zones/z1/dns_records":
            return self.reply([])
        if self.command == "POST" and url.path == "/zones/z1/dns_records":
            return self.reply({"id": "r1"})
        return self.reply(None, 404)

    do_GET = do_POST = do_PUT = do_DELETE = handle_any


HTTPServer(("127.0.0.1", int(sys.argv[1])), Mock).serve_forever()
