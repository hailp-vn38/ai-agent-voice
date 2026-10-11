import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "docker_storage", Path(__file__).parents[1] / "check-docker-storage.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class StorageTests(unittest.TestCase):
    def test_accepts_build_output_on_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            storage = Path(directory)
            module.check_storage(storage / "target", storage)

    def test_rejects_build_output_outside_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "outside"):
                module.check_storage("/var/tmp/target", Path(directory))

    def test_rejects_missing_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            storage = Path(directory) / "missing"
            with self.assertRaisesRegex(ValueError, "missing"):
                module.check_storage(storage / "target", storage)

    def test_rejects_symlink_outside_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            storage = Path(directory) / "storage"
            storage.mkdir()
            (storage / "target").symlink_to(Path(directory) / "target")
            with self.assertRaisesRegex(ValueError, "outside"):
                module.check_storage(storage / "target", storage)


if __name__ == "__main__":
    unittest.main()
