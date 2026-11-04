#!/usr/bin/env python3
"""A mock of GitHub for tools/vm/check-tailnet.sh, never the real one: the
OAuth device flow and the REST calls wadd makes (crates/wad-github), on the
laptop's 127.0.0.1 (10.0.2.2 from the VMs, whose wadd.toml points [github]
web and api here).

    github-mock.py PORT LOG [GITDIR]

    POST /login/device/code         a device code (form: client_id, scope)
    GET  /login/device              the device page (what the sign-in window shows)
    POST /login/oauth/access_token  slow_down once, then authorization_pending,
                                    until the code is approved; then the token
    POST /mock/approve              the person typed the code (form: user_code):
                                    200 if it's the one handed out, else 404
    GET  /mock/state                what was handed out (JSON, for the check)
    GET  /api/user                  whose the token is, with X-OAuth-Scopes
    GET  /api/user/repos            two repositories
    GET  /sam-wados/notes.git/…     with GITDIR: a repository to clone, over git's
                                    "dumb" HTTP (static files), behind Basic auth
                                    as GitHub's (x-access-token, the token): 401
                                    first, so git asks wadd's credential helper

Every request is a line of JSON in LOG (the form's fields, never a token).
Standard library, and git to make the repository.
"""

import base64
import json
import os
import secrets
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

SCOPES = "repo, workflow, read:org"
USER = {"login": "sam-wados", "name": "Sam Wados", "id": 1000}
LOCK = threading.Lock()
STATE = {"device_code": None, "user_code": None, "approved": False, "token": None, "slowed": False}


def repos(base):
    return [
        {
            "full_name": f"sam-wados/{n}", "name": n, "private": p, "description": d,
            "clone_url": f"{base}/sam-wados/{n}.git", "default_branch": "main",
            "pushed_at": "2026-11-01T00:00:00Z",
        }
        for n, p, d in (("notes", True, "Sam's notes"), ("sprites", False, None))
    ]


class Handler(BaseHTTPRequestHandler):
    server_version = "github-mock"

    def log_message(self, *_):
        pass

    def record(self, form):
        entry = {
            "at": round(time.time(), 3), "method": self.command, "path": urlsplit(self.path).path,
            "form": {k: v for k, v in form.items() if k not in ("device_code",)},
            "auth": bool(self.headers.get("authorization")),
            "accept": self.headers.get("accept", ""), "agent": self.headers.get("user-agent", ""),
        }
        with LOCK, open(self.server.log, "a") as f:
            f.write(json.dumps(entry) + "\n")

    def form(self):
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n).decode() if n else ""
        return {k: v[0] for k, v in parse_qs(body).items()}

    def send(self, code, body, headers=()):
        data = (body if isinstance(body, str) else json.dumps(body)).encode()
        self.send_response(code)
        self.send_header("content-type", "text/html" if isinstance(body, str) else "application/json")
        self.send_header("content-length", str(len(data)))
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def token_ok(self):
        auth = self.headers.get("authorization", "")
        return STATE["token"] is not None and auth in (f"Bearer {STATE['token']}", f"token {STATE['token']}")

    def do_POST(self):
        form = self.form()
        self.record(form)
        path = urlsplit(self.path).path
        if path == "/login/device/code":
            with LOCK:
                STATE.update(
                    device_code=secrets.token_hex(20),
                    user_code=f"{secrets.token_hex(2).upper()}-{secrets.token_hex(2).upper()}",
                    approved=False, token=None, slowed=False,
                )
            self.send(200, {
                "device_code": STATE["device_code"], "user_code": STATE["user_code"],
                "verification_uri": f"{self.server.base}/login/device", "expires_in": 900, "interval": 1,
            })
        elif path == "/login/oauth/access_token":
            with LOCK:
                if form.get("device_code") != STATE["device_code"]:
                    answer = {"error": "incorrect_device_code"}
                elif not STATE["slowed"]:
                    STATE["slowed"] = True
                    answer = {"error": "slow_down", "interval": 2}
                elif not STATE["approved"]:
                    answer = {"error": "authorization_pending"}
                else:
                    STATE["token"] = STATE["token"] or f"gho_wadosmock{secrets.token_hex(16)}"
                    answer = {"access_token": STATE["token"], "token_type": "bearer", "scope": SCOPES.replace(" ", "")}
            self.send(200, answer)
        elif path == "/mock/approve":
            with LOCK:
                ok = STATE["user_code"] is not None and form.get("user_code") == STATE["user_code"]
                if ok:
                    STATE["approved"] = True
            self.send(200 if ok else 404, {"approved": ok})
        else:
            self.send(404, {"message": "Not Found"})

    def git_ok(self):
        auth = self.headers.get("authorization", "")
        if not auth.startswith("Basic ") or STATE["token"] is None:
            return False
        try:
            user, _, password = base64.b64decode(auth[6:]).decode().partition(":")
        except ValueError:
            return False
        return user == "x-access-token" and password == STATE["token"]

    def git(self, path):
        root = os.path.realpath(self.server.gitdir)
        file = os.path.realpath(os.path.join(root, path.lstrip("/")))
        if not file.startswith(root + os.sep):
            return self.send(404, {"message": "Not Found"})
        if not self.git_ok():
            return self.send(401, {"message": "Requires authentication"}, [("www-authenticate", 'Basic realm="GitHub"')])
        if not os.path.isfile(file):
            return self.send(404, {"message": "Not Found"})
        with open(file, "rb") as f:
            data = f.read()
        # Plain files: git falls back to its dumb protocol.
        self.send_response(200)
        self.send_header("content-type", "application/octet-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.record({})
        path = urlsplit(self.path).path
        if self.server.gitdir and path.startswith("/sam-wados/") and ".git/" in path:
            self.git(path)
        elif path == "/login/device":
            self.send(200, "<!doctype html><title>Device activation (mock)</title>"
                           "<body style='font: 48px sans-serif'><h1>GitHub (a mock)</h1>"
                           "<form><input name=user_code placeholder='XXXX-XXXX'></form></body>")
        elif path == "/mock/state":
            self.send(200, {k: STATE[k] for k in ("user_code", "approved", "token")})
        elif path == "/api/user":
            if self.token_ok():
                self.send(200, USER, [("x-oauth-scopes", SCOPES)])
            else:
                self.send(401, {"message": "Bad credentials"})
        elif path == "/api/user/repos":
            if self.token_ok():
                self.send(200, repos(self.server.base))
            else:
                self.send(401, {"message": "Bad credentials"})
        else:
            self.send(404, {"message": "Not Found"})


def make_repo(gitdir):
    """sam-wados/notes.git: one commit, its files served as they are."""
    bare = os.path.join(gitdir, "sam-wados", "notes.git")
    work = os.path.join(gitdir, "work")
    env = dict(os.environ, GIT_AUTHOR_NAME="Sam", GIT_AUTHOR_EMAIL="sam@wados.test",
               GIT_COMMITTER_NAME="Sam", GIT_COMMITTER_EMAIL="sam@wados.test", GIT_CONFIG_GLOBAL="/dev/null",
               GIT_TERMINAL_PROMPT="0", GIT_ASKPASS="/bin/false", SSH_ASKPASS="/bin/false")
    run = lambda *a, cwd=None: subprocess.run(["git", *a], cwd=cwd, env=env, check=True, capture_output=True)
    run("init", "-q", "--bare", "-b", "main", bare)
    run("init", "-q", "-b", "main", work)
    with open(os.path.join(work, "README.md"), "w") as f:
        f.write("# Notes\n\nThe mock GitHub's repository (tools/vm/github-mock.py).\n")
    run("add", "README.md", cwd=work)
    run("commit", "-q", "-m", "First notes", cwd=work)
    run("push", "-q", bare, "main", cwd=work)
    run("update-server-info", cwd=bare)


def main(argv):
    if len(argv) not in (3, 4):
        raise SystemExit(__doc__)
    port, log = int(argv[1]), argv[2]
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.log = log
    server.gitdir = argv[3] if len(argv) == 4 else None
    if server.gitdir:
        make_repo(server.gitdir)
    # What the VMs call it.
    server.base = f"http://10.0.2.2:{port}"
    open(log, "w").close()
    server.serve_forever()


if __name__ == "__main__":
    main(sys.argv)
