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
elif sys.argv[1] == 'info' and '{{.SecurityOptions}}' in sys.argv:
    print(os.environ.get('FAKE_SECURITY_OPTIONS', '[name=seccomp,profile=builtin]'))
elif sys.argv[1] == 'info':
    print(json.dumps({'DockerRootDir': os.environ.get('FAKE_DOCKER_ROOT', '/var/lib/docker')}))
else:
    with open(os.environ['DOCKER_CALLS'] + '.user', 'w') as output:
        output.write(f"{os.environ.get('APP_UID', '')}:{os.environ.get('APP_GID', '')}")
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
                [str(MANAGER), *args], cwd=root, env=environment,
                text=True, capture_output=True,
            )
            calls = [json.loads(line) for line in calls_file.read_text().splitlines()] if calls_file.exists() else []
            user_file = Path(f"{calls_file}.user")
            self.compose_user = user_file.read_text() if user_file.exists() else None
            return result, calls

    def test_prod_up_runs_as_container_root_on_rootless_docker(self):
        result, _ = self.invoke(
            "prod", "up", FAKE_SECURITY_OPTIONS="[name=seccomp,profile=builtin name=rootless]",
            APP_UID="", APP_GID="",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.compose_user, "0:0")

    def test_prod_up_runs_as_invoking_user_on_rootful_docker(self):
        result, _ = self.invoke("prod", "up", APP_UID="", APP_GID="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.compose_user, f"{os.getuid()}:{os.getgid()}")

    def test_prod_up_keeps_explicit_user(self):
        result, _ = self.invoke(
            "prod", "up", FAKE_SECURITY_OPTIONS="[name=rootless]", APP_UID="1234", APP_GID="5678",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.compose_user, "1234:5678")

    def test_prod_routes_from_another_working_directory(self):
        result, calls = self.invoke("prod", "logs", "server")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [[
            "compose", "-p", "vela-prod", "-f", "compose.yaml", "-f", "compose.prod.yaml",
            "logs", "--follow", "--tail", "100", "server",
        ]])

    def test_pull_selects_service(self):
        result, calls = self.invoke("prod", "pull", "web")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][-2:], ["pull", "web"])

    def test_tools_update_runs_in_prod_server(self):
        result, calls = self.invoke("prod", "tools-update")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][7:12], ["exec", "-T", "server", "sh", "-euc"])
        self.assertIn("/app/data/bin", calls[-1][-1])

    def test_up_checks_storage_before_starting(self):
        result, calls = self.invoke("test", "up", "web")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Application storage is under /mnt/storage", result.stdout)
        self.assertEqual(calls[-1][-6:], ["up", "--detach", "--wait", "--wait-timeout", "120", "web"])

    def test_build_accepts_default_docker_storage(self):
        result, calls = self.invoke("prod", "build", FAKE_DOCKER_ROOT="/var/lib/docker")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][-1], "build")
        self.assertFalse(any(call[0] == "info" for call in calls))

    def test_down_keeps_volumes(self):
        for environment, files in [
            ("prod", ["-f", "compose.yaml", "-f", "compose.prod.yaml"]),
            ("test", ["-f", "compose.test.yaml"]),
        ]:
            for services in [(), ("all",), ("server",), ("web",), ("server", "web")]:
                with self.subTest(environment=environment, services=services):
                    result, calls = self.invoke(environment, "down", *services)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    selection = [] if services == ("all",) else list(services)
                    self.assertEqual(calls, [[
                        "compose", "-p", f"vela-{environment}", *files,
                        "down", *selection,
                    ]])

    def test_down_propagates_compose_failure(self):
        result, _ = self.invoke("prod", "down", "server", FAKE_COMPOSE_EXIT="17")
        self.assertEqual(result.returncode, 17)

    def test_build_accepts_multiple_services_and_defaults_to_prod(self):
        for prefix, project in [((), "prod"), (("prod",), "prod"), (("test",), "test")]:
            with self.subTest(prefix=prefix):
                result, calls = self.invoke(*prefix, "build", "server", "web")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls[-1][1:3], ["-p", f"vela-{project}"])
                self.assertEqual(calls[-1][-3:], ["build", "server", "web"])
                self.assertIn("Application storage is under /mnt/storage", result.stdout)

    def test_build_all_omits_service_selection(self):
        for prefix in [(), ("prod",), ("test",)]:
            for services in [(), ("all",)]:
                with self.subTest(prefix=prefix, services=services):
                    result, calls = self.invoke(*prefix, "build", *services)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(calls[-1][-1], "build")

    def test_config_does_not_print_secrets(self):
        result, calls = self.invoke("prod", "config")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1][-2:], ["config", "--quiet"])

    def test_rejects_invalid_commands_before_docker(self):
        for args in [
            ("prod", "run"), ("prod", "down", "--volumes"),
            ("prod", "logs", "unknown"), ("prod", "pull", "unknown"), ("test", "tools-update"), ("prod", "tools-update", "server"), ("staging", "up"),
            ("build", "unknown"), ("build", "all", "server"),
            ("build", "server", "--no-cache"), ("prod", "up", "server", "web"),
            ("prod", "down", "unknown"), ("prod", "down", "all", "server"),
            ("test", "down", "web", "--volumes"),
        ]:
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
