"""Measure HTTP upgrade and application hello against an isolated qualification process.

Build first: cargo build --locked -p voice-agent-server --bin voice-agent-server --features qualification-providers
Run: python3 .scratch/ws-connection-latency/probe.py --binary /path/to/voice-agent-server
Optional --budget-ms asserts connection-to-ServerHello latency; no production credentials needed.
"""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import socket
import sqlite3
import statistics
import struct
import subprocess
import tempfile
import threading
import time
import tomllib
import uuid


def measure(address):
    started = time.perf_counter()
    with socket.create_connection(address, timeout=8) as sock:
        tcp = time.perf_counter()
        key = base64.b64encode(os.urandom(16)).decode()
        request = (f"GET /voice/v1/ HTTP/1.1\r\nHost: {address[0]}:{address[1]}\r\n"
                   f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                   "Sec-WebSocket-Version: 13\r\nProtocol-Version: 1\r\n"
                   "Device-Id: latency-probe\r\nClient-Id: latency-probe\r\n\r\n")
        sock.sendall(request.encode())
        wire = sock.makefile("rb")
        status = wire.readline()
        assert b"101" in status, status.decode().strip()
        headers = {}
        while (line := wire.readline()) != b"\r\n":
            assert line, "upgrade closed"
            name, value = line.decode().split(":", 1)
            headers[name.lower()] = value.strip()
        expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        assert headers.get("sec-websocket-accept") == expected
        upgraded = time.perf_counter()
        hello = json.dumps({"type": "hello", "version": 1, "transport": "websocket",
                            "features": {"mcp": False}, "audio_params": {"format": "opus",
                            "sample_rate": 16000, "channels": 1, "frame_duration": 60}}).encode()
        mask = os.urandom(4)
        sock.sendall(b"\x81\xfe" + struct.pack("!H", len(hello)) + mask
                     + bytes(value ^ mask[i % 4] for i, value in enumerate(hello)))
        first = wire.read(2)
        # shortcut: this probe expects the canonical unfragmented ServerHello, use a WS library for other wire scenarios.
        assert len(first) == 2 and first[0] == 0x81 and not first[1] & 0x80, "expected ServerHello text"
        length = first[1] & 0x7f
        if length == 126:
            length = struct.unpack("!H", wire.read(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", wire.read(8))[0]
        assert length <= 65536
        response = json.loads(wire.read(length))
        assert response["type"] == "hello" and response["session_id"]
        finished = time.perf_counter()
        sock.sendall(b"\x88\x80" + os.urandom(4))
        wire.close()
    return {"tcp_ms": (tcp - started) * 1000, "upgrade_after_tcp_ms": (upgraded - tcp) * 1000,
            "hello_after_upgrade_ms": (finished - upgraded) * 1000, "total_ms": (finished - started) * 1000}


class Mcp(http.server.BaseHTTPRequestHandler):
    delay = 0
    seen = []

    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method = request["method"]
        self.seen.append(method)
        if method == "initialize":
            time.sleep(self.delay)
            result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "latency-probe", "version": "1"}}
        elif method == "tools/list":
            result = {"tools": [{"name": "probe", "description": "Latency probe",
                                  "inputSchema": {"type": "object", "properties": {}}}]}
        else:
            self.send_response(202)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        data = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass  # The short-budget case intentionally cancels discovery.


def run(binary, samples):
    production = tomllib.loads((Path(__file__).resolve().parents[2] / "docker/config.prod.example.toml").read_text())["mcp"]["external"]
    mcp = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mcp)
    threading.Thread(target=mcp.serve_forever, daemon=True).start()
    rows = []
    try:
        with tempfile.TemporaryDirectory(prefix="ws-latency-") as directory:
            root = Path(directory)
            database = root / "probe.db"
            for name, delay, budget, overall in [("no_mcp", 0, 3000, 5000), ("fast_mcp", 0, 3000, 5000),
                                                ("slow_mcp", 0.6, 3000, 5000), ("slow_mcp_100ms_budget", 0.6, 100, 5000),
                                                ("slow_mcp_production_budget", 0.6, production["per_server_resolution_timeout_ms"], production["overall_resolution_budget_ms"])]:
                config = (f'[server]\nbind = "127.0.0.1:0"\npublic_ws_url = "ws://127.0.0.1:8000/voice/v1/"\n'
                          f'[database]\nurl = "sqlite://{database}"\n'
                          '[database.devices.enrollment]\nenabled = false\n'
                          '[provider_defaults]\nvad = "q"\nasr = "q"\nllm = "q"\ntts = "q"\n'
                          f'[mcp.external]\nper_server_resolution_timeout_ms = {budget}\n'
                          f'overall_resolution_budget_ms = {overall}\n'
                          '[mcp.external.network]\nallowed_hosts = ["127.0.0.1"]\n')
                for kind in ("vad", "asr", "llm", "tts"):
                    config += f'[providers.{kind}.instances.q]\nadapter = "qualification_{kind}"\n'
                (root / "config.toml").write_text(config)
                Mcp.delay = delay
                Mcp.seen = []
                for migrating in ([True, False] if name == "no_mcp" else [False]):
                    bound_file = root / "bound.json"
                    bound_file.unlink(missing_ok=True)
                    env = dict(os.environ, VOICE_AGENT_CONFIG=str(root / "config.toml"),
                               VOICE_AGENT_BOUND_ADDRESS_FILE=str(bound_file), VOICE_AGENT_STARTUP_NONCE=str(uuid.uuid4()),
                               RUST_LOG="warn")
                    with (root / "server.log").open("w") as log:
                        process = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
                        try:
                            deadline = time.monotonic() + 15
                            while not bound_file.exists():
                                assert process.poll() is None, (root / "server.log").read_text()
                                assert time.monotonic() < deadline, "startup timed out"
                                time.sleep(0.02)
                            if not migrating:
                                host, port = json.loads(bound_file.read_text())["address"].rsplit(":", 1)
                                timings = [measure((host, int(port))) for _ in range(samples)]
                                rows.append({"case": name, "samples": timings, "median_ms": {
                                    k: round(statistics.median(t[k] for t in timings), 3) for k in timings[0]},
                                    "mcp_methods": list(Mcp.seen)})
                        except Exception:
                            print((root / "server.log").read_text())
                            raise
                        finally:
                            process.terminate()
                            process.wait(timeout=20)
                    # All fixture writes happen while this database's sole process owner is stopped.
                    with sqlite3.connect(database) as db:
                        if migrating:
                            db.execute("INSERT INTO agents (key,name,created_at,updated_at) VALUES ('probe','Probe',1,1)")
                            db.execute("INSERT INTO devices (device_id,agent_id,created_at,updated_at) VALUES ('latency-probe',1,1,1)")
                        elif name == "no_mcp":
                            db.execute("INSERT INTO mcp_servers (key,name,url,connect_timeout_ms,request_timeout_ms,created_at,updated_at) VALUES ('probe','Probe',?,2000,2000,1,1)",
                                       (f"http://127.0.0.1:{mcp.server_port}/mcp",))
                            db.execute("INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,created_at) VALUES (1,1,1)")
            baseline, fast, slow, short, production_result = rows
            assert "tools/list" in fast["mcp_methods"], "MCP fixture must complete discovery"
            assert slow["median_ms"]["total_ms"] - baseline["median_ms"]["total_ms"] >= 450
            assert slow["median_ms"]["total_ms"] - short["median_ms"]["total_ms"] >= 350
            assert all(r["median_ms"]["hello_after_upgrade_ms"] < 100 for r in rows)
            assert production_result["median_ms"]["total_ms"] < 550, "production discovery budget must bound the slow-MCP case"
    finally:
        mcp.shutdown()
        mcp.server_close()
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--budget-ms", type=float)
    args = parser.parse_args()
    assert args.samples > 0
    results = run(str(Path(args.binary).resolve()), args.samples)
    output = Path(__file__).with_name("measurements.json")
    output.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps([{k: v for k, v in row.items() if k != "samples"} for row in results], indent=2))
    if args.budget_ms is not None:
        assert all(r["median_ms"]["total_ms"] <= args.budget_ms for r in results), "connection budget exceeded"
