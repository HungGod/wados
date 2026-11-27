#!/usr/bin/env python3
"""The local pages tools/wadbrowser-e2e.sh opens in WadBrowser (no real sites).

    wadbrowser-e2e-server.py PORT LOGFILE [MEDIA-DIR]

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
  /video?FILE           plays MEDIA-DIR/FILE (muted); title "video FILE playing" once it has, else
                        "video FILE error <code> <message>"
  /mse                  what Media Source (YouTube's player) says it plays: title "mse avc1=… vp9=… av1=… aac=… opus=…"
  /media/FILE           MEDIA-DIR's FILE (Range requests too, as a video's are)
"""
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
LOG = open(sys.argv[2], "a", buffering=1)
MEDIA = sys.argv[3] if len(sys.argv) > 3 else None
TYPES = {".mp4": "video/mp4", ".webm": "video/webm", ".mkv": "video/x-matroska"}

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
        elif p == "/video":
            f = u.query
            js = (
                f"var v = document.querySelector('video'), f = {f!r};"
                "v.addEventListener('timeupdate', function () {"
                " if (v.currentTime > 0.3) document.title = 'video ' + f + ' playing'; });"
                "v.addEventListener('error', function () { var e = v.error || {};"
                " document.title = 'video ' + f + ' error ' + e.code + ' ' + (e.message || ''); });"
                "v.play().catch(function (e) { document.title = 'video ' + f + ' error play ' + e; });"
            )
            self.send(page(f"video {f}", f'<video muted playsinline src="/media/{f}"></video>', js))
        elif p == "/mse":
            js = (
                "var t = {avc1: 'video/mp4; codecs=\"avc1.64001f\"', vp9: 'video/webm; codecs=\"vp09.00.10.08\"',"
                " av1: 'video/mp4; codecs=\"av01.0.05M.08\"', aac: 'audio/mp4; codecs=\"mp4a.40.2\"',"
                " opus: 'audio/webm; codecs=\"opus\"'};"
                "var out = ['mse']; for (var k in t) out.push(k + '=' + (window.MediaSource ? MediaSource.isTypeSupported(t[k]) : 'none'));"
                "document.title = out.join(' ');"
            )
            self.send(page("mse", script=js))
        elif p.startswith("/media/") and MEDIA:
            self.media(p[len("/media/"):])
        elif p == "/download":
            self.send(
                b"downloaded by WadBrowser\n",
                "application/octet-stream",
                [("Content-Disposition", 'attachment; filename="wb test #1.txt"')],
            )
        else:
            self.send_response(404)
            self.end_headers()


    def media(self, name):
        import os

        path = os.path.join(MEDIA, os.path.basename(name))
        if not os.path.isfile(path):
            self.send_response(404)
            self.end_headers()
            return
        data = open(path, "rb").read()
        ctype = TYPES.get(os.path.splitext(path)[1], "application/octet-stream")
        rng = self.headers.get("Range", "")
        if rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            start = int(a or 0)
            end = min(int(b) if b else len(data) - 1, len(data) - 1)
            chunk = data[start : end + 1]
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        else:
            chunk = data
            self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(len(chunk)))
        self.end_headers()
        try:
            self.wfile.write(chunk)
        except (BrokenPipeError, ConnectionResetError):
            pass


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
