import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


MANAGER = Path(__file__).resolve().parents[1] / "compose.sh"
DOCKER = """#!/usr/bin/env python3
import json, os, sys
with open(os.environ['DOCKER_CALLS'], 'a') as output:
    output.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'context':
    print('unix:///var/run/docker.sock')
elif sys.argv[1] == 'info':
    print(json.dumps({'DockerRootDir': os.environ.get('FAKE_DOCKER_ROOT', '/mnt/storage/docker')}))
else:
    sys.exit(int(os.environ.get('FAKE_COMPOSE_EXIT', '0')))
"""


class ComposeManagerTests(unittest.TestCase):
    def invoke(self, *args, **overrides):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            docker = root / "docker"
            docker.write_text(DOCKER)
            docker.chmod(0o755)
            calls_file = root / "calls.jsonl"
            environment = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}", DOCKER_CALLS=str(calls_file))
            environment.pop("DOCKER_HOST", None)
            environment.update(overrides)
            result = subprocess.run(
                ["bash", str(MANAGER), *args], cwd=root, env=environment,
                text=True, capture_output=True,
            )
            calls = [json.loads(line) for line in calls_file.read_text().splitlines()] if calls_file.exists() else []
            return result, calls

    def test_prod_routes_from_another_working_directory(self):
        result, calls = self.invoke("prod", "logs", "server")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [[
            "compose", "-p", "voice-agent-prod", "-f", "compose.yaml", "-f", "compose.prod.yaml",
            "logs", "--follow", "--tail", "100", "server",
        ]])

    def test_up_checks_storage_before_starting(self):
        result, calls = self.invoke("test", "up", "web")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[1][0], "info")
        self.assertEqual(calls[-1][-6:], ["up", "--detach", "--wait", "--wait-timeout", "120", "web"])

    def test_build_does_not_run_on_system_disk(self):
        result, calls = self.invoke("prod", "build", FAKE_DOCKER_ROOT="/var/lib/docker")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("outside /mnt/storage", result.stderr)
        self.assertFalse(any(call[0] == "compose" for call in calls))

    def test_down_keeps_volumes(self):
        result, calls = self.invoke("prod", "down")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][-1], "down")
        self.assertNotIn("--volumes", calls[-1])

    def test_config_does_not_print_secrets(self):
        result, calls = self.invoke("prod", "config")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][-2:], ["config", "--quiet"])

    def test_rejects_invalid_commands_before_docker(self):
        for args in [("prod", "run"), ("prod", "down", "--volumes"), ("prod", "logs", "unknown"), ("staging", "up")]:
            with self.subTest(args=args):
                result, calls = self.invoke(*args)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(calls, [])

    def test_propagates_compose_failure(self):
        result, _ = self.invoke("prod", "status", FAKE_COMPOSE_EXIT="17")
        self.assertEqual(result.returncode, 17)

    def test_run_delegates_to_existing_test_gate(self):
        result, calls = self.invoke("test", "run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(call[-3:] == ["node", "/smoke.mjs", "after-restart"] for call in calls))
        self.assertEqual(calls[-1][-3:], ["down", "--volumes", "--remove-orphans"])


if __name__ == "__main__":
    unittest.main()
