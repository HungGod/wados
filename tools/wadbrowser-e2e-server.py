#!/usr/bin/env python3
"""The local pages tools/wadbrowser-e2e.sh opens in WadBrowser (no real sites).

    wadbrowser-e2e-server.py PORT LOGFILE

Each request is a line in LOGFILE: "<path> cookie=<Cookie header> dnt=<DNT> gpc=<Sec-GPC>".
Pages report back through their title (WADBROWSER_TEST_LOG has each title).

  /set?name=N&value=V   sets a cookie kept a day; title "set N"
  /show                 title "cookies: <document.cookie>"
  /opener               opens /popup as a popup; title "opener got: <message>" when it posts back
  /popup                posts "hello-from-popup" to its opener, then closes itself
  /filter               an image the test list blocks (/wb-ad-banner.png), one it doesn't
                        (/wb-ok.png) and a box its hiding rule hides; title "filter blocked=B allowed=A hidden=H"
  /headers              title "headers dnt=<DNT> gpc=<Sec-GPC>" (the page's own request's)
  /go?to=URL            goes to URL (a page's own navigation, after it loads)
  /download             a file to download ("wb test #1.txt")
  /big?WORD             white, with "WORD" in huge black letters (for pixel checks); title "BIG WORD"
"""
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
LOG = open(sys.argv[2], "a", buffering=1)

PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000d49444154789c6360000002000154a24f5d0000000049454e44ae426082"
)


def page(title, body="", script=""):
    return (
        "<!doctype html><html><head><meta charset=utf-8>"
        f"<title>{title}</title></head><body>{body}<script>{script}</script></body></html>"
    ).encode()


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, body, ctype="text/html; charset=utf-8", headers=()):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urllib.parse.urlsplit(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        h = self.headers
        LOG.write(f"{u.path} cookie={h.get('Cookie', '')} dnt={h.get('DNT', '')} gpc={h.get('Sec-GPC', '')}\n")
        p = u.path
        if p == "/set":
            n, v = q.get("name", "c"), q.get("value", "1")
            self.send(page(f"set {n}"), headers=[("Set-Cookie", f"{n}={v}; Max-Age=86400; Path=/")])
        elif p == "/show":
            self.send(page("show", script="document.title = 'cookies: ' + document.cookie;"))
        elif p == "/opener":
            js = (
                "addEventListener('message', function (e) { document.title = 'opener got: ' + e.data; });"
                "setTimeout(function () { window.open('/popup', 'oauth', 'width=480,height=600'); }, 300);"
            )
            self.send(page("opener", script=js))
        elif p == "/popup":
            js = (
                "document.title = 'popup has opener: ' + !!window.opener;"
                "window.opener.postMessage('hello-from-popup', '*');"
                "setTimeout(function () { window.close(); }, 500);"
            )
            self.send(page("popup", script=js))
        elif p == "/filter":
            body = (
                '<div class="wb-ad-box" id="box">an ad</div>'
                '<img id="ad" src="/wb-ad-banner.png"><img id="ok" src="/wb-ok.png">'
            )
            js = (
                "var r = {blocked: 0, allowed: 0};"
                "function done() { var hidden = getComputedStyle(document.getElementById('box')).display == 'none' ? 1 : 0;"
                " document.title = 'filter blocked=' + r.blocked + ' allowed=' + r.allowed + ' hidden=' + hidden; }"
                "var n = 0; function one() { if (++n == 2) setTimeout(done, 300); }"
                "var ad = document.getElementById('ad'), ok = document.getElementById('ok');"
                "ad.onerror = function () { r.blocked++; one(); }; ad.onload = one;"
                "ok.onload = function () { r.allowed++; one(); }; ok.onerror = one;"
                "if (ad.complete) one(); if (ok.complete) { if (ok.naturalWidth) r.allowed++; one(); }"
            )
            self.send(page("filter", body, js))
        elif p in ("/wb-ok.png", "/wb-ad-banner.png"):
            self.send(PNG, "image/png")
        elif p == "/headers":
            self.send(page(f"headers dnt={h.get('DNT', '-')} gpc={h.get('Sec-GPC', '-')}"))
        elif p == "/go":
            to = q.get("to", "/show")
            self.send(page("go", script=f"setTimeout(function () {{ location.href = {to!r}; }}, 300);"))
        elif p == "/big":
            word = urllib.parse.quote(u.query or "big")
            body = f'<div style="font:bold 220px sans-serif;color:#000;line-height:1">{word.upper()}</div>'
            self.send(
                page(f"BIG {word}", body).replace(b"<body>", b'<body style="margin:0;background:#fff">'),
            )
        elif p == "/download":
            self.send(
                b"downloaded by WadBrowser\n",
                "application/octet-stream",
                [("Content-Disposition", 'attachment; filename="wb test #1.txt"')],
            )
        else:
            self.send_response(404)
            self.end_headers()


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
