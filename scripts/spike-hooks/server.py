#!/usr/bin/env python3
"""Servidor de captura para el spike de hooks (v1.1, hito 0).

Escucha solo en 127.0.0.1, registra cada petición en un JSONL y responde a
`PermissionRequest` según el modo. Genera además el settings.json que se pasa
a `claude --settings`. No forma parte de la app: es una herramienta de prueba.

Uso:  scripts/spike-hooks/server.py [--mode silent|allow|deny] [--dir DIR]
"""
import argparse
import json
import os
import secrets
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

EVENTS = [
    "SessionStart", "PermissionRequest", "PreToolUse", "PostToolUse",
    "Notification", "Stop", "SessionEnd",
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["silent", "allow", "deny"], default="silent",
                    help="respuesta a PermissionRequest (silent = sin decision)")
    ap.add_argument("--dir", default="/tmp/spike-hooks")
    ap.add_argument("--message", default="SPIKE-DENY: no ejecutes esto; responde con la palabra BANANA.")
    args = ap.parse_args()

    os.makedirs(args.dir, exist_ok=True)
    log_path = os.path.join(args.dir, "events.jsonl")
    token = secrets.token_hex(8)

    def log(entry):
        entry["ts"] = round(time.time(), 3)
        with open(log_path, "a") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):  # silencia el log por defecto
            pass

        def do_POST(self):
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length).decode("utf-8", "replace")
            try:
                body = json.loads(raw)
            except ValueError:
                body = {"_raw": raw}
            ok = self.path.startswith(f"/hook/{token}")
            event = body.get("hook_event_name") if isinstance(body, dict) else None
            log({"kind": "http", "path": self.path.replace(token, "<token>"),
                 "token_ok": ok, "event": event, "body": body})
            if not ok:
                self.send_response(403)
                self.end_headers()
                return
            out = {}
            if event == "PermissionRequest" and args.mode != "silent":
                decision = {"behavior": args.mode}
                if args.mode == "deny":
                    decision["message"] = args.message
                out = {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                              "decision": decision}}
            data = json.dumps(out).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            log({"kind": "response", "event": event, "mode": args.mode, "out": out})

    srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = srv.server_address[1]

    statusline = os.path.join(args.dir, "statusline.sh")
    with open(statusline, "w") as f:
        f.write(f"#!/bin/sh\ncat >> '{args.dir}/statusline.jsonl'\n"
                f"echo >> '{args.dir}/statusline.jsonl'\necho 'SPIKE-STATUS'\n")
    os.chmod(statusline, 0o755)

    hooks = {}
    for ev in EVENTS:
        entry = {"hooks": [{"type": "http", "url": f"http://127.0.0.1:{port}/hook/{token}/{ev}"}]}
        if ev in ("PreToolUse", "PostToolUse", "PermissionRequest"):
            entry["matcher"] = "Bash"
        hooks[ev] = [entry]
    settings = {"hooks": hooks, "statusLine": {"type": "command", "command": statusline}}
    settings_path = os.path.join(args.dir, "settings.json")
    with open(settings_path, "w") as f:
        json.dump(settings, f, indent=2)

    print(f"modo={args.mode} puerto={port}", file=sys.stderr)
    print(f"settings: {settings_path}", file=sys.stderr)
    print(f"log:      {log_path}", file=sys.stderr)
    print(f"claude --settings {settings_path}", file=sys.stderr)
    sys.stderr.flush()
    srv.serve_forever()


if __name__ == "__main__":
    main()
