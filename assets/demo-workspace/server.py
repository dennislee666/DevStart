#!/usr/bin/env python3
"""Tiny loopback-only HTTP service used by the public demo workspace."""
import argparse
import json
from http.server import BaseHTTPRequestHandler, HTTPServer


parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, required=True)
parser.add_argument("--label", required=True)
arguments = parser.parse_args()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(
            {"status": "ok", "demo": True, "service": arguments.label}
        ).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format_string, *args):
        print(format_string % args, flush=True)


server = HTTPServer(("127.0.0.1", arguments.port), Handler)
print(
    f"{arguments.label} is a local demo endpoint at 127.0.0.1:{arguments.port}",
    flush=True,
)
server.serve_forever()
