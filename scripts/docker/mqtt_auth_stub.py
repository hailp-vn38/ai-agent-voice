"""Stand-in for the Vela server's /mqtt/auth/* endpoints, used only by mqtt-broker-smoke.sh.

Mirrors the rules in crates/vela-server/src/services/device_push.rs (passwords, topics, ACLs) so
the smoke test exercises the broker configuration without building the whole server. Every Device
is treated as registered. Status-only answers: 200 allows, 403 denies.
"""
import hashlib
import hmac
import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

SECRET = os.environ["SMOKE_SECRET"].encode()
SERVICE = "vela-server"


def mac(message):
    return hmac.new(SECRET, message.encode(), hashlib.sha256).hexdigest()


def topic_for(device_id):
    return f"vela/v1/devices/{hashlib.sha256(device_id.encode()).hexdigest()[:32]}/signal"


def allowed(path, body):
    username = body.get("username", "")
    if path == "/mqtt/auth/user":
        password = body.get("password", "")
        expected = mac(f"v1-service:{SERVICE}") if username == SERVICE else mac(f"v1:{username}")
        return hmac.compare_digest(expected, password)
    if path == "/mqtt/auth/acl":
        topic, acc = body.get("topic", ""), body.get("acc")
        if username == SERVICE:
            return acc == 2 and topic.startswith("vela/v1/devices/") and topic.endswith("/signal")
        return acc in (1, 4) and topic == topic_for(username)
    return False  # superuser and anything else


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            body = {}
        self.send_response(200 if allowed(self.path, body) else 403)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_):
        pass


HTTPServer(("0.0.0.0", 8000), Handler).serve_forever()
