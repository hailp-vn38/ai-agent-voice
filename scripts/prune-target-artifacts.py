#!/usr/bin/env python3
"""Remove build artifacts that cargo no longer needs from the debug target directory.

Usage: prune-target-artifacts.py <repo-root> <auto|aggressive>

Old generations of the project's own artifacts are removed together with everything that
belongs to them. On macOS debug info stays in the object files next to a binary, so removing
a binary alone leaves most of its size behind. Objects whose binary or library is already
gone are then swept for every crate.

Lives in its own file because a shell heredoc needs a temp file, which is unavailable exactly
when the disk is full and pruning matters most.
"""
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

# A running build has objects whose binary does not exist yet, so an orphan this recent may
# still be linked. Linking follows codegen within minutes.
ORPHAN_MIN_AGE_SECONDS = 3600

_PROJECT_ARTIFACTS = {
    "voice_agent_server",
    "libvoice_agent_server",
    "provider_bench",
    "provider_bench_av",
    "provider_runtime_bench",
    "zerotts_core_check",
}
_ARTIFACT = re.compile(r"^([a-zA-Z0-9_]+)-([0-9a-f]{16})(\..+)?$")
_OWNED_LIBRARY_EXTENSIONS = (".rlib", ".rmeta", ".dylib")


def _is_companion(extension):
    """True for files that only exist on behalf of a binary or library."""
    return extension.endswith((".o", ".d", ".dSYM", ".txt"))


@dataclass
class Report:
    files: int = 0
    bytes: int = 0

    def __str__(self):
        return f"pruned {self.files} files, {self.bytes / 2**30:.1f} GiB"


def build_running():
    """Whether cargo or rustc is running; assumes yes when that cannot be determined."""
    try:
        for name in ("cargo", "rustc"):
            found = subprocess.run(["pgrep", "-x", name], capture_output=True)
            if found.returncode != 1:
                return True
    except OSError:
        return True
    return False


def _remove(path, report):
    """Remove a file or directory tree, counting only what was actually removed."""
    try:
        if path.is_dir() and not path.is_symlink():
            sizes = [item.lstat().st_size for item in path.rglob("*") if item.is_file()]
            shutil.rmtree(path)
            report.files += len(sizes)
            report.bytes += sum(sizes)
        else:
            size = path.lstat().st_size
            path.unlink()
            report.files += 1
            report.bytes += size
    except OSError:
        pass


def _prune_incremental(debug, keep, report):
    incremental = debug / "incremental"
    if not incremental.is_dir():
        return
    groups = {}
    for entry in incremental.iterdir():
        if entry.is_dir() and "-" in entry.name:
            try:
                modified = entry.stat().st_mtime
            except OSError:
                continue
            groups.setdefault(entry.name.rsplit("-", 1)[0], []).append((modified, entry))
    for entries in groups.values():
        entries.sort(key=lambda item: item[0], reverse=True)
        for _, old in entries[keep:]:
            _remove(old, report)


def _prune_old_generations(deps, test_stems, mode, keep, report):
    tracked = test_stems | _PROJECT_ARTIFACTS
    companions = {}
    groups = {}
    for entry in deps.iterdir():
        match = _ARTIFACT.match(entry.name)
        if not match:
            continue
        name, digest, extension = match.group(1), match.group(2), match.group(3) or ""
        if _is_companion(extension):
            companions.setdefault(f"{name}-{digest}", []).append(entry)
        elif name in tracked:
            try:
                modified = entry.stat().st_mtime
            except OSError:
                continue
            groups.setdefault((name, extension), []).append((modified, name, digest, entry))
    for (name, extension), entries in groups.items():
        entries.sort(key=lambda item: item[0], reverse=True)
        limit = 0 if mode == "aggressive" and name in test_stems and extension == "" else keep
        for _, entry_name, digest, old in entries[limit:]:
            _remove(old, report)
            owner = entry_name[3:] if entry_name.startswith("lib") else entry_name
            for companion in companions.pop(f"{owner}-{digest}", []):
                _remove(companion, report)


def _sweep_orphans(deps, min_age, report):
    present = {entry.name for entry in deps.iterdir()}
    newest_allowed = time.time() - min_age
    for name in present:
        match = _ARTIFACT.match(name)
        if not match or not _is_companion(match.group(3) or ""):
            continue
        owner = f"{match.group(1)}-{match.group(2)}"
        if owner in present or any(f"lib{owner}{ext}" in present for ext in _OWNED_LIBRARY_EXTENSIONS):
            continue
        path = deps / name
        try:
            if path.lstat().st_mtime > newest_allowed:
                continue
        except OSError:
            continue
        _remove(path, report)


def prune(root, target, mode, build_running=build_running):
    """Prune `target`/debug. `build_running` is a bool, or a callable that returns one."""
    report = Report()
    debug = Path(target) / "debug"
    if not debug.is_dir():
        return report
    keep = 1 if mode == "aggressive" else 2
    _prune_incremental(debug, keep, report)
    deps = debug / "deps"
    if deps.is_dir():
        test_dir = Path(root) / "crates" / "voice-agent-server" / "tests"
        test_stems = (
            {path.stem.replace("-", "_") for path in test_dir.glob("*.rs")} if test_dir.is_dir() else set()
        )
        _prune_old_generations(deps, test_stems, mode, keep, report)
        running = build_running() if callable(build_running) else build_running
        thorough = mode == "aggressive" and not running
        _sweep_orphans(deps, 0 if thorough else ORPHAN_MIN_AGE_SECONDS, report)
    return report


def main(arguments):
    if len(arguments) != 2 or arguments[1] not in ("auto", "aggressive"):
        print("usage: prune-target-artifacts.py <repo-root> <auto|aggressive>", file=sys.stderr)
        return 2
    root = Path(arguments[0])
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    print(prune(root, target, arguments[1]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
