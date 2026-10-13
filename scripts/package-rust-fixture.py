#!/usr/bin/env python3
"""Create a reproducible ZIP for Admin Plugin Store upload."""
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[1]
folder = ROOT / "plugins" / "fixture-rust-1.0.0"
destination = ROOT / "plugins" / "fixture-rust-1.0.0.zip"

entries = sorted(p for p in folder.rglob("*") if p.is_file())
with ZipFile(destination, "w") as archive:
    for file in entries:
        name = file.relative_to(folder).as_posix()
        info = ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
        info.compress_type = ZIP_DEFLATED
        info.external_attr = 0o100644 << 16
        archive.writestr(info, file.read_bytes())
print(destination)
