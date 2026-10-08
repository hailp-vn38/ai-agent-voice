#!/usr/bin/env python3
"""Reject local Docker builds whose storage is outside /mnt/storage."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib


def check_storage(info, containerd_root):
    paths = [Path(info["DockerRootDir"])]
    if ["driver-type", "io.containerd.snapshotter.v1"] in info.get("DriverStatus", []):
        paths.append(Path(containerd_root))
    for path in paths:
        if not path.resolve().is_relative_to(Path("/mnt/storage")):
            raise ValueError(f"Storage {path} is outside /mnt/storage; see docs/docker.md")


def main():
    endpoint = os.environ.get("DOCKER_HOST") or subprocess.check_output(
        ["docker", "context", "inspect", "--format", "{{.Endpoints.docker.Host}}"], text=True
    ).strip()
    if not endpoint.startswith("unix://"):
        raise ValueError("Storage check requires a local Docker daemon")
    info = json.loads(subprocess.check_output(["docker", "info", "--format", "{{json .}}"], text=True))
    # ponytail: reads the standard containerd config; custom --config paths need an explicit check.
    config_path = Path("/etc/containerd/config.toml")
    config = tomllib.loads(config_path.read_text()) if config_path.exists() else {}
    check_storage(info, config.get("root", "/var/lib/containerd"))
    print("Docker storage is under /mnt/storage")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
