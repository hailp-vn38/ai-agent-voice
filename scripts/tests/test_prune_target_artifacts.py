import importlib.util
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location('pruner', SCRIPTS / 'prune-target-artifacts.py')
pruner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pruner)

OLD = 'a' * 16
NEW = 'b' * 16
OTHER = 'c' * 16


class PruneTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / 'repo'
        self.target = Path(directory.name) / 'target'
        self.deps = self.target / 'debug' / 'deps'
        self.deps.mkdir(parents=True)
        tests = self.root / 'crates' / 'vela-server' / 'tests'
        tests.mkdir(parents=True)
        (tests / 'admin_api.rs').write_text('')

    def touch(self, name, age=0, size=1):
        path = self.deps / name
        path.write_bytes(b'x' * size)
        stamp = time.time() - age
        os.utime(path, (stamp, stamp))
        return path

    def prune(self, mode='aggressive', build_running=False):
        return pruner.prune(self.root, self.target, mode, build_running=build_running)

    def test_removing_a_binary_also_removes_its_objects_and_debug_info(self):
        self.touch(f'admin_api-{OLD}', age=100)
        self.touch(f'admin_api-{OLD}.d', age=100)
        self.touch(f'admin_api-{OLD}.admin_api.abc-cgu.0.rcgu.o', age=100)
        (self.deps / f'admin_api-{OLD}.dSYM').mkdir()
        self.touch(f'admin_api-{OTHER}', age=50)
        self.touch(f'admin_api-{NEW}', age=1)
        self.touch(f'admin_api-{NEW}.admin_api.abc-cgu.0.rcgu.o', age=1)

        self.prune('auto', build_running=True)

        left = {path.name for path in self.deps.iterdir()}
        self.assertNotIn(f'admin_api-{OLD}', left)
        self.assertNotIn(f'admin_api-{OLD}.d', left)
        self.assertNotIn(f'admin_api-{OLD}.dSYM', left)
        self.assertNotIn(f'admin_api-{OLD}.admin_api.abc-cgu.0.rcgu.o', left)
        self.assertIn(f'admin_api-{NEW}.admin_api.abc-cgu.0.rcgu.o', left)

    def test_objects_do_not_count_as_generations_of_a_binary(self):
        self.touch(f'admin_api-{NEW}', age=1)
        self.touch(f'admin_api-{NEW}.admin_api.one.rcgu.o', age=1)
        self.touch(f'admin_api-{NEW}.admin_api.two.rcgu.o', age=1)

        self.prune('auto')

        self.assertTrue((self.deps / f'admin_api-{NEW}').exists())
        self.assertTrue((self.deps / f'admin_api-{NEW}.admin_api.one.rcgu.o').exists())

    def test_aggressive_sweeps_orphans_of_any_crate_and_keeps_owned_objects(self):
        self.touch(f'serde-{OLD}.serde.abc-cgu.0.rcgu.o', age=1)
        self.touch(f'serde-{OLD}.d', age=1)
        (self.deps / f'serde-{OLD}.dSYM').mkdir()
        self.touch(f'tokio-{NEW}.tokio.abc-cgu.0.rcgu.o', age=1)
        self.touch(f'libtokio-{NEW}.rlib', age=1)
        self.touch(f'tokio-{NEW}.d', age=1)
        self.touch(f'bytes-{OTHER}.bytes.abc-cgu.0.rcgu.o', age=1)
        self.touch(f'libbytes-{OTHER}.rmeta', age=1)
        self.touch(f'mac-{OTHER}.mac.abc-cgu.0.rcgu.o', age=1)
        self.touch(f'libmac-{OTHER}.dylib', age=1)

        self.prune('aggressive')

        left = {path.name for path in self.deps.iterdir()}
        self.assertEqual(
            left,
            {
                f'tokio-{NEW}.tokio.abc-cgu.0.rcgu.o',
                f'libtokio-{NEW}.rlib',
                f'tokio-{NEW}.d',
                f'bytes-{OTHER}.bytes.abc-cgu.0.rcgu.o',
                f'libbytes-{OTHER}.rmeta',
                f'mac-{OTHER}.mac.abc-cgu.0.rcgu.o',
                f'libmac-{OTHER}.dylib',
            },
        )

    def test_recent_orphans_survive_while_a_build_may_still_need_them(self):
        recent = self.touch(f'serde-{OLD}.serde.abc-cgu.0.rcgu.o', age=60)
        stale = self.touch(f'serde-{OTHER}.serde.abc-cgu.0.rcgu.o', age=pruner.ORPHAN_MIN_AGE_SECONDS + 60)

        self.prune('aggressive', build_running=True)

        self.assertTrue(recent.exists())
        self.assertFalse(stale.exists())

    def test_auto_mode_never_sweeps_recent_orphans(self):
        recent = self.touch(f'serde-{OLD}.serde.abc-cgu.0.rcgu.o', age=60)

        self.prune('auto', build_running=False)

        self.assertTrue(recent.exists())

    def test_reports_what_it_freed(self):
        self.touch(f'serde-{OLD}.serde.abc-cgu.0.rcgu.o', size=2048)

        report = self.prune('aggressive')

        self.assertEqual(report.files, 1)
        self.assertEqual(report.bytes, 2048)

    def test_missing_target_is_not_an_error(self):
        report = pruner.prune(self.root, self.target / 'absent', 'aggressive', build_running=False)

        self.assertEqual(report.files, 0)

    def test_agent_test_prune_runs_end_to_end_and_reports(self):
        orphan = self.touch(f'serde-{OLD}.serde.abc-cgu.0.rcgu.o', size=4096)
        environment = {**os.environ, 'CARGO_TARGET_DIR': str(self.target)}

        result = subprocess.run(
            ['bash', str(SCRIPTS / 'agent-test.sh'), 'prune'],
            env=environment,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('RESULT: PASS', result.stdout)
        self.assertIn('files', result.stdout)
        if not pruner.build_running():
            self.assertFalse(orphan.exists())

    def test_agent_test_prune_does_not_need_a_temp_file(self):
        source = (SCRIPTS / 'agent-test.sh').read_text()
        body = source.split('prune_stale_target_artifacts() {', 1)[1].split('\n}\n', 1)[0]

        self.assertNotIn('<<', body)


if __name__ == '__main__':
    unittest.main()
