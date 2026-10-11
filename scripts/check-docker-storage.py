#!/usr/bin/env python3
"""Check application storage without restricting Docker's data root."""
from pathlib import Path
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def check_storage(target_dir, storage=Path("/mnt/storage")):
    if not storage.is_dir():
        raise ValueError(f"Application storage directory {storage} is missing")
    if not Path(target_dir).resolve().is_relative_to(storage.resolve()):
        raise ValueError(f"Cargo build output {target_dir} is outside {storage}")


def main():
    config = tomllib.loads((ROOT / ".cargo/config.toml").read_text())
    check_storage(config["build"]["target-dir"])
    print("Application storage is under /mnt/storage; Docker may use its default data root")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
