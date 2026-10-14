#!/usr/bin/env python3
"""Build the deterministic Rust fixture as a native v2 Plugin Store package."""
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
subprocess.run(
    [sys.executable, str(root / "scripts/package-rust-plugins.py"),
     "fixture-rust-1.0.0", "--output", str(root / "plugins")],
    cwd=root, check=True,
)
