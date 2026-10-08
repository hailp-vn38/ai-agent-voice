import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "docker_storage", Path(__file__).parents[1] / "check-docker-storage.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class StorageTests(unittest.TestCase):
    def test_rejects_default_docker_root(self):
        with self.assertRaisesRegex(ValueError, "/var/lib/docker"):
            module.check_storage({"DockerRootDir": "/var/lib/docker"}, "/mnt/storage/containerd")

    def test_rejects_separate_containerd_store(self):
        with self.assertRaisesRegex(ValueError, "/var/lib/containerd"):
            module.check_storage(
                {"DockerRootDir": "/mnt/storage/docker", "DriverStatus": [["driver-type", "io.containerd.snapshotter.v1"]]},
                "/var/lib/containerd",
            )

    def test_accepts_both_stores_on_storage(self):
        module.check_storage(
            {"DockerRootDir": "/mnt/storage/docker", "DriverStatus": [["driver-type", "io.containerd.snapshotter.v1"]]},
            "/mnt/storage/containerd",
        )

    def test_legacy_store_needs_only_docker_root(self):
        module.check_storage({"DockerRootDir": "/mnt/storage/docker"}, "/var/lib/containerd")


if __name__ == "__main__":
    unittest.main()
