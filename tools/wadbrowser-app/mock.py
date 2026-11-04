#!/usr/bin/env python3
"""Local pages for WadBrowser's headless checks (no real sites).

    mock.py PORT LOG

/a.html /b.html /c.html   plain pages (their titles: Page A, B, C)
/app/                     a web app's start page; /app/?wander=1 goes to
                          http://localhost:PORT/elsewhere.html after a moment
                          (another site: 127.0.0.1 and localhost differ)
/login/device?form=single|split
                          GitHub's device page, as its two forms: one
                          `user-code` field, or `user-code-0`..`-7`. Without
                          `signed=1` it first sends you to a sign-in page
                          (/login), which comes back by itself, as GitHub's
                          sign-in does. The form is drawn late (after 700 ms).
                          Each `input`/`change` event is counted; once every
                          field has a value the page reports them:
                          /report?form=..&value=..&inputs=..&changes=..
                          A submit would hit /submitted (the filler must never).
Every request is logged to LOG, one line each: METHOD PATH.
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, urlparse

PORT = int(sys.argv[1])
LOG = open(sys.argv[2], "a", buffering=1)

PAGE = "<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head><body style='font:20px sans-serif;background:#223;color:#eee'><h1>{title}</h1>{body}</body></html>"

DEVICE = """<!doctype html><html><head><meta charset=utf-8><title>Device activation</title></head>
<body style='font:20px sans-serif;background:#0d1117;color:#e6edf3'>
<h1>Device activation</h1><p id=status>Enter the code displayed on your device</p>
<div id=slot></div>
<script>
const FORM = %(form)r;
let inputs = 0, changes = 0, reported = false;
function report() {
  const fields = [...document.querySelectorAll('input[type=text]')];
  if (reported || !fields.length || fields.some((f) => !f.value)) return;
  reported = true;
  const value = fields.map((f) => f.value).join(FORM === 'split' ? '' : '');
  document.getElementById('status').textContent = 'Filled: ' + value;
  fetch('/report?form=' + FORM + '&value=' + encodeURIComponent(value) + '&inputs=' + inputs + '&changes=' + changes);
}
setTimeout(() => {
  const form = document.createElement('form');
  form.action = '/submitted';
  form.method = 'post';
  form.addEventListener('submit', (e) => { e.preventDefault(); fetch('/submitted'); });
  if (FORM === 'single') {
    form.innerHTML = '<input type=text name="user-code" id="user-code" maxlength=9 autocomplete=off style="font:28px monospace;width:12em">';
  } else {
    let h = '';
    for (let i = 0; i < 8; i++) {
      h += '<input type=text name="user-code-' + i + '" id="user-code-' + i + '" maxlength=1 style="font:28px monospace;width:1.4em;margin:2px">';
      if (i === 3) h += '<span style="font:28px monospace"> - </span>';
    }
    form.innerHTML = h;
  }
  form.innerHTML += '<button type=submit style="font:20px sans-serif;margin-left:1em">Continue</button>';
  form.addEventListener('input', () => { inputs++; });
  form.addEventListener('change', () => { changes++; setTimeout(report, 50); });
  document.getElementById('slot').appendChild(form);
}, 700);
</script></body></html>"""

LOGIN = """<!doctype html><html><head><meta charset=utf-8><title>Sign in to GitHub</title></head>
<body style='font:20px sans-serif;background:#0d1117;color:#e6edf3'><h1>Sign in to GitHub</h1>
<input name=login id=login_field placeholder="Username"> <input type=password name=password>
<script>setTimeout(() => { location.href = %(back)r; }, 900);</script></body></html>"""


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, code, body, ctype="text/html; charset=utf-8", extra=()):
        data = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        LOG.write(f"POST {self.path}\n")
        self.send(200, "no")

    def do_GET(self):
        LOG.write(f"GET {self.path}\n")
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        p = u.path
        if p in ("/a.html", "/b.html", "/c.html", "/elsewhere.html"):
            name = {"/a.html": "Page A", "/b.html": "Page B", "/c.html": "Page C", "/elsewhere.html": "Elsewhere"}[p]
            return self.send(200, PAGE.format(title=name, body=f"<p>{p}</p>"))
        if p == "/app/":
            body = "<p>The app's own page.</p>"
            if q.get("wander"):
                body += f"<script>setTimeout(() => location.href = 'http://localhost:{PORT}/elsewhere.html', 1500)</script>"
            return self.send(200, PAGE.format(title="Mock App", body=body))
        if p in ("/login/device", "/login/device/"):
            form = q.get("form", "single")
            if not q.get("signed"):
                back = f"/login/device?form={form}&signed=1"
                return self.send(302, "", extra=[("Location", f"/login?return_to={quote(back, safe='/')}")])
            return self.send(200, DEVICE % {"form": form})
        if p == "/login":
            return self.send(200, LOGIN % {"back": q.get("return_to", "/")})
        if p in ("/report", "/submitted", "/favicon.ico"):
            return self.send(200 if p != "/favicon.ico" else 404, "ok")
        return self.send(404, PAGE.format(title="Not here", body=""))


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
