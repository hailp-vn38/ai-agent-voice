#!/usr/bin/env python3
"""One-shot jsonl-v1 Python plugin for the Rust Agent Job Platform.

stdin:  one JSON request line
stdout: exactly one JSON response line on normal/application-error paths
stderr: diagnostics only

The host/container, not this plugin, enforces filesystem/network isolation.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import time

from research import collect_topic, make_report

MAX_REQUEST_BYTES = 16 * 1024
MAX_RESPONSE_BYTES = 64 * 1024


class PluginInputError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def respond(response: dict) -> None:
    payload = json.dumps(response, ensure_ascii=False, separators=(",", ":"))
    if len(payload.encode("utf-8")) > MAX_RESPONSE_BYTES:
        raise PluginInputError("response_too_large", "Result exceeds the 64 KiB stdout limit")
    sys.stdout.write(payload + "\n")
    sys.stdout.flush()


def fail(request_id: str, code: str, message: str) -> dict:
    return {
        "schema_version": 1,
        "request_id": request_id,
        "ok": False,
        "error": {"code": code, "message": message[:240]},
        "artifacts": [],
    }


def require_args(arguments: object, *, allowed: set[str]) -> dict:
    if not isinstance(arguments, dict) or set(arguments) - allowed:
        raise PluginInputError("invalid_arguments", "Arguments must be an object with allowed fields only")
    return arguments


def artifact_metadata(directory: Path, name: str, media_type: str, content: bytes) -> dict:
    # The Rust host must independently verify paths, bytes, MIME and digest.
    path = directory / name
    path.write_bytes(content)
    return {
        "path": name,
        "media_type": media_type,
        "size_bytes": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }


def handle(request: dict) -> dict:
    request_id = request.get("request_id", "unknown")
    if not isinstance(request_id, str) or not 1 <= len(request_id) <= 128:
        raise PluginInputError("invalid_request", "request_id must be 1-128 characters")
    if request.get("schema_version") != 1:
        raise PluginInputError("invalid_request", "Unsupported schema_version")
    if set(request) != {"schema_version", "request_id", "operation", "arguments", "artifact_dir"}:
        raise PluginInputError("invalid_request", "Unexpected or missing request fields")
    operation = request["operation"]
    arguments = request["arguments"]
    directory = request["artifact_dir"]
    if not isinstance(directory, str) or not Path(directory).is_absolute():
        raise PluginInputError("invalid_request", "artifact_dir must be an absolute host-provided path")

    if operation == "collect_topic":
        args = require_args(arguments, allowed={"topic", "limit"})
        topic = args.get("topic")
        limit = args.get("limit", 10)
        if not isinstance(topic, str) or not 1 <= len(topic.strip()) <= 100:
            raise PluginInputError("invalid_arguments", "topic must contain 1-100 characters")
        if type(limit) is not int or not 1 <= limit <= 20:
            raise PluginInputError("invalid_arguments", "limit must be an integer from 1 to 20")
        output_dir = Path(directory)
        if not output_dir.is_dir() or output_dir.is_symlink():
            raise PluginInputError("artifact_dir_unavailable", "Host-provided artifact directory does not exist")
        result = collect_topic(topic.strip(), limit)
        records = json.dumps(result["records"], ensure_ascii=False, indent=2).encode("utf-8")
        report = make_report(result).encode("utf-8")
        artifacts = [
            artifact_metadata(output_dir, "records.json", "application/json", records),
            artifact_metadata(output_dir, "report.md", "text/markdown", report),
        ]
        return {"schema_version": 1, "request_id": request_id, "ok": True, "result": result, "artifacts": artifacts}

    if operation == "simulate_failure":
        args = require_args(arguments, allowed={"mode", "seconds"})
        mode = args.get("mode")
        if mode == "application_error":
            return fail(request_id, "fixture_application_error", "Synthetic recoverable application failure")
        if mode == "crash":
            print("Synthetic worker crash (exit=23)", file=sys.stderr, flush=True)
            os._exit(23)
        if mode == "malformed_output":
            sys.stdout.write("not-json\n")
            sys.stdout.flush()
            raise SystemExit(0)
        if mode == "sleep":
            seconds = args.get("seconds", 3)
            if type(seconds) is not int or not 1 <= seconds <= 30:
                raise PluginInputError("invalid_arguments", "seconds must be 1-30")
            time.sleep(seconds)
            return {"schema_version": 1, "request_id": request_id, "ok": True, "result": {"slept_seconds": seconds}, "artifacts": []}
        raise PluginInputError("invalid_arguments", "mode must be crash, malformed_output, sleep, or application_error")

    raise PluginInputError("unsupported_operation", "Operation is not declared by this plugin")


def main() -> None:
    request_id = "unknown"
    try:
        raw = sys.stdin.buffer.readline(MAX_REQUEST_BYTES + 1)
        if len(raw) > MAX_REQUEST_BYTES:
            raise PluginInputError("invalid_request", "Request exceeds 16 KiB")
        if not raw.endswith(b"\n"):
            raise PluginInputError("invalid_request", "Expected exactly one newline-terminated JSON request")
        request = json.loads(raw.decode("utf-8"))
        if not isinstance(request, dict):
            raise PluginInputError("invalid_request", "Request must be an object")
        if isinstance(request.get("request_id"), str):
            request_id = request["request_id"][:128]
        response = handle(request)
        respond(response)
    except PluginInputError as exc:
        respond(fail(request_id, exc.code, str(exc)))
    except (UnicodeDecodeError, json.JSONDecodeError):
        respond(fail(request_id, "invalid_request", "Invalid UTF-8 or JSON"))
    except (OSError, ValueError) as exc:
        # Local I/O failure is a plugin runtime error; the Rust host decides health.
        print(f"worker I/O failure: {type(exc).__name__}", file=sys.stderr, flush=True)
        respond(fail(request_id, "artifact_write_failed", "Unable to publish plugin output"))
    except Exception as exc:
        # Unexpected exceptions are fatal; do not forge an application-level success.
        print(f"unexpected worker error: {type(exc).__name__}", file=sys.stderr, flush=True)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
