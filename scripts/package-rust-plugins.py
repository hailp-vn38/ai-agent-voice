#!/usr/bin/env python3
"""Build trusted repo plugins and create reproducible native ELF Store ZIPs."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[1]
PLUGINS = ("web-extractor", "document-analysis", "fixture-rust-1.0.0")


def package(folder, binary, destination):
    manifest = tomllib.loads((folder / "plugin.toml").read_text())
    if manifest["runtime"]["kind"] != "rust":
        raise ValueError("not a Rust plugin")
    entrypoint = manifest["runtime"]["entrypoint"]
    text = (folder / "plugin.toml").read_text().replace(
        f'entrypoint = "{entrypoint}"', 'entrypoint = "bin/plugin"'
    )
    files = {"plugin.toml": text.encode(), "bin/plugin": binary.read_bytes()}
    required = {manifest["agent"]["instructions"]}
    for operation in manifest["operations"].values():
        required.update((operation["input_schema"], operation["output_schema"]))
    for name in required:
        path = (folder / name).resolve()
        if not path.is_relative_to(folder.resolve()):
            raise ValueError("package path escapes plugin directory")
        files[name] = path.read_bytes()
    if not files["bin/plugin"].startswith(b"\x7fELF"):
        raise ValueError("build did not produce a Linux ELF binary")
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Stage beside the destination so a failed build never replaces a valid ZIP.
    with tempfile.NamedTemporaryFile(dir=destination.parent, suffix=".zip") as staged:
        with ZipFile(staged.name, "w") as archive:
            for name, content in sorted(files.items()):
                info = ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
                info.compress_type = ZIP_DEFLATED
                info.external_attr = (0o100555 if name == "bin/plugin" else 0o100644) << 16
                archive.writestr(info, content)
        if Path(staged.name).stat().st_size > 20 * 1024 * 1024:
            raise ValueError("Store ZIP exceeds 20 MiB")
        shutil.copyfile(staged.name, destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plugins", nargs="*", choices=PLUGINS)
    parser.add_argument("--output", type=Path, default=ROOT / "data/curated-plugins")
    args = parser.parse_args()
    for name in args.plugins or PLUGINS:
        folder = ROOT / "plugins" / name
        manifest = tomllib.loads((folder / "plugin.toml").read_text())
        identity = manifest["plugin"]
        with tempfile.TemporaryDirectory(prefix="rust-plugin-build-") as temporary:
            project = folder
            if name == "fixture-rust-1.0.0":
                # The source fixture intentionally has no [workspace] for Store validation.
                project = Path(temporary)
                shutil.copytree(folder / "src", project / "src")
                for file in ("Cargo.toml", "Cargo.lock"):
                    shutil.copyfile(folder / file, project / file)
            cargo = str(project / "Cargo.toml")
            subprocess.run([str(ROOT / "scripts/agent-test.sh"), "run", "--", "cargo", "test",
                            "--locked", "--quiet", "--manifest-path", cargo], cwd=ROOT, check=True)
            subprocess.run([str(ROOT / "scripts/agent-test.sh"), "run", "--", "cargo", "build",
                            "--release", "--locked", "--manifest-path", cargo], cwd=ROOT, check=True)
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
                 "--manifest-path", cargo], cwd=ROOT))
            binary = Path(metadata["target_directory"]) / "release" / identity["id"]
            destination = args.output.resolve() / f'{identity["id"]}-{identity["version"]}.zip'
            destination.parent.mkdir(parents=True, exist_ok=True)
            staged = Path(temporary) / "package.zip"
            package(folder, binary, staged)
            subprocess.run([str(ROOT / "scripts/check-rust-plugin.sh"), "--static-only", str(staged)],
                           cwd=ROOT, check=True)
            with tempfile.TemporaryDirectory(dir=destination.parent) as publication:
                pending = Path(publication) / destination.name
                shutil.copyfile(staged, pending)
                pending.replace(destination)
                # Curated install resolves <plugin-id>.zip, while downloads retain the version.
                curated = Path(publication) / f'{identity["id"]}.zip'
                shutil.copyfile(staged, curated)
                curated.replace(destination.parent / curated.name)
        print(destination)


if __name__ == "__main__":
    main()
