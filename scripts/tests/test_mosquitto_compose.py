import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
MOSQUITTO = ROOT / "docker" / "mosquitto"
COMPOSE_SETS = {
    "prod": ["-f", "compose.yaml", "-f", "compose.prod.yaml"],
    "test": ["-f", "compose.test.yaml"],
}


def settings(path):
    """Mosquitto config as (key, value) pairs; comments and blanks dropped."""
    pairs = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            key, _, value = line.partition(" ")
            pairs.append((key, value.strip()))
    return pairs


@unittest.skipUnless(shutil.which("docker"), "docker CLI not installed")
class MosquittoProfileTests(unittest.TestCase):
    def model(self, environment, *profiles):
        flags = [flag for profile in profiles for flag in ("--profile", profile)]
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for name in ("compose.yaml", "compose.prod.yaml", "compose.test.yaml"):
                shutil.copy(ROOT / name, work / name)
            (work / "docker").mkdir()
            (work / "docker" / "secrets.env").write_text("")  # gitignored; prod env_file must exist
            result = subprocess.run(
                ["docker", "compose", *flags, *COMPOSE_SETS[environment], "config", "--format", "json"],
                cwd=work, text=True, capture_output=True,
                env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "")},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_broker_is_absent_without_the_push_profile(self):
        for environment in COMPOSE_SETS:
            with self.subTest(environment=environment):
                self.assertNotIn("mosquitto", self.model(environment)["services"])

    def test_broker_joins_with_the_push_profile(self):
        for environment in COMPOSE_SETS:
            with self.subTest(environment=environment):
                self.assertIn("mosquitto", self.model(environment, "push")["services"])

    def test_test_broker_publishes_no_port(self):
        broker = self.model("test", "push")["services"]["mosquitto"]
        self.assertFalse(broker.get("ports"))

    def test_prod_publishes_only_the_tls_listener(self):
        ports = self.model("prod", "push")["services"]["mosquitto"]["ports"]
        self.assertEqual([port["target"] for port in ports], [8883])


class MosquittoConfigTests(unittest.TestCase):
    def test_anonymous_clients_and_local_user_lists_are_off(self):
        auth = settings(MOSQUITTO / "auth.d" / "auth.conf")
        self.assertIn(("allow_anonymous", "false"), auth)
        keys = {key for key, _ in auth}
        self.assertFalse({"password_file", "acl_file"} & keys)
        self.assertIn(("auth_opt_backends", "http"), auth)

    def test_every_decision_goes_to_the_server_endpoints(self):
        auth = dict(settings(MOSQUITTO / "auth.d" / "auth.conf"))
        self.assertEqual(auth["auth_opt_http_host"], "server")
        self.assertEqual(auth["auth_opt_http_getuser_uri"], "/mqtt/auth/user")
        self.assertEqual(auth["auth_opt_http_superuser_uri"], "/mqtt/auth/superuser")
        self.assertEqual(auth["auth_opt_http_aclcheck_uri"], "/mqtt/auth/acl")
        self.assertEqual(auth["auth_opt_http_params_mode"], "json")
        self.assertEqual(auth["auth_opt_http_response_mode"], "status")

    def test_test_config_has_one_plain_listener(self):
        listeners = [v for k, v in settings(MOSQUITTO / "mosquitto.test.conf") if k == "listener"]
        self.assertEqual(listeners, ["1883"])

    def test_prod_config_has_a_tls_listener_and_an_internal_one(self):
        prod = settings(MOSQUITTO / "mosquitto.prod.conf")
        self.assertEqual([v for k, v in prod if k == "listener"], ["8883", "1883"])
        keys = [k for k, _ in prod]
        self.assertIn("certfile", keys)
        self.assertIn("keyfile", keys)
        self.assertLess(keys.index("certfile"), keys.index("listener", keys.index("listener") + 1))

    def test_both_environments_include_the_shared_auth_config(self):
        for name in ("mosquitto.test.conf", "mosquitto.prod.conf"):
            with self.subTest(name=name):
                self.assertIn(("include_dir", "/mosquitto/config/auth.d"), settings(MOSQUITTO / name))


if __name__ == "__main__":
    unittest.main()
