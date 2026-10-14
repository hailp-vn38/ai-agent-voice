#!/usr/bin/env python3
"""Rebuild the qualification ZIP deterministically from its checked-in source."""
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

root = Path(__file__).resolve().parents[1]
source = root / "plugins" / "fixture-research-1.0.0"
output = source.parent / f"{source.name}.zip"
with ZipFile(output, "w", compression=ZIP_DEFLATED) as archive:
    for path in sorted(source.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts and not path.name.startswith("."):
            entry = ZipInfo(path.relative_to(source).as_posix(), (1980, 1, 1, 0, 0, 0))
            entry.compress_type = ZIP_DEFLATED
            entry.external_attr = (0o100644 << 16)
            archive.writestr(entry, path.read_bytes())
print(f"Rebuilt {output.relative_to(root)}")
