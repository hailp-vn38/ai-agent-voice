import importlib.util
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

spec = importlib.util.spec_from_file_location(
    "rust_plugins", Path(__file__).parents[1] / "package-rust-plugins.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RustPluginPackagingTests(unittest.TestCase):
    def test_packages_only_runtime_files_and_is_reproducible(self):
        folder = module.ROOT / "plugins/fixture-rust-1.0.0"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "worker"
            binary.write_bytes(b"\x7fELF" + b"fixture")
            first, second = root / "first.zip", root / "second.zip"
            module.package(folder, binary, first)
            module.package(folder, binary, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with ZipFile(first) as archive:
                self.assertIn("bin/plugin", archive.namelist())
                self.assertNotIn("src/main.rs", archive.namelist())
                self.assertNotIn("check-cases.json", archive.namelist())
                self.assertIn(b'entrypoint = "bin/plugin"', archive.read("plugin.toml"))

    def test_invalid_binary_does_not_replace_existing_package(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary, destination = root / "worker", root / "package.zip"
            binary.write_bytes(b"not an ELF")
            destination.write_bytes(b"existing package")
            with self.assertRaisesRegex(ValueError, "ELF"):
                module.package(module.ROOT / "plugins/document-analysis", binary, destination)
            self.assertEqual(destination.read_bytes(), b"existing package")
