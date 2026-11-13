#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Preview/remove regenerable integration-test executables in this checkout only.

Dependencies, ordinary binaries, native archives, Release, databases and evidence
are never candidates. Uses Cargo's own profile lock, not a process-name scan.
"""
import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import stat
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MAGIC = (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe", b"MZ")


@dataclass(frozen=True)
class Runner:
    path: Path
    package: str
    size: int
    modified_ns: int
    device: int
    inode: int


@contextmanager
def cargo_lock(profile):
    try:
        import fcntl
    except ImportError as error:
        raise RuntimeError("This cleanup requires Unix Cargo file locking; no files removed") from error
    marker = profile / ".cargo-lock"
    descriptor = os.open(marker, os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("Cargo is using this build profile; retry when the build finishes") from error
        yield
    finally:
        os.close(descriptor)


def candidates(root, profile, metadata, keep):
    """Recognize Cargo integration artifacts by target AND fingerprint identity."""
    deps = profile / "deps"
    if not deps.is_dir():
        return []
    packages = {item["id"]: item for item in metadata["packages"]}
    all_files = list(deps.iterdir())
    removable = []
    for identity in metadata["workspace_members"]:
        package = packages[identity]
        manifest = Path(package["manifest_path"]).resolve()
        if not manifest.is_relative_to(root.resolve()):
            continue
        runners = []
        names = {target["name"].replace("-", "_"): target["name"] for target in package["targets"] if target["kind"] == ["test"]}
        for path in all_files:
            match = re.fullmatch(r"(.+)-([a-f0-9]{16})(?:\.exe)?", path.name)
            if not match or match[1] not in names or path.is_symlink():
                continue
            fingerprint = profile / ".fingerprint" / (package["name"] + "-" + match[2])
            marker = fingerprint / ("test-integration-test-" + names[match[1]])
            if not marker.is_file() or marker.is_symlink():
                continue
            info = path.stat()
            if not stat.S_ISREG(info.st_mode) or not info.st_mode & 0o111:
                continue
            with path.open("rb") as stream:
                if not stream.read(4).startswith(MAGIC):
                    continue
            runners.append(Runner(path, package["name"], info.st_size, info.st_mtime_ns, info.st_dev, info.st_ino))
        runners.sort(key=lambda item: (item.modified_ns, item.path.name), reverse=True)
        removable.extend(runners[keep:])
    return removable


def remove_runner(runner):
    # Refuse a replacement between planning and applying, including symlinks.
    current = runner.path.lstat()
    if not stat.S_ISREG(current.st_mode) or (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns) != (
        runner.device, runner.inode, runner.size, runner.modified_ns
    ):
        raise RuntimeError("Build output changed during cleanup; retry after the build")
    runner.path.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="remove planned regenerable runners; default previews only")
    parser.add_argument("--keep-runners", type=int, default=8, help="retain the newest N integration runners per workspace package (default 8)")
    args = parser.parse_args()
    if args.keep_runners < 1:
        parser.error("--keep-runners must be positive")
    profile = ROOT / "target" / "debug"
    if not profile.is_dir():
        print("No workspace Debug build cache")
        return
    env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target")}
    result = subprocess.run([os.environ.get("CARGO", "cargo"), "metadata", "--locked", "--offline", "--no-deps", "--format-version=1", "--manifest-path", str(ROOT / "Cargo.toml")],
        env=env, check=True, text=True, capture_output=True)
    with cargo_lock(profile):
        plan = candidates(ROOT, profile, json.loads(result.stdout), args.keep_runners)
        for runner in plan:
            print(f'{runner.size / 1048576:.1f} MiB  {runner.path.relative_to(ROOT)}')
        total = sum(item.size for item in plan)
        print(f'{"Removing" if args.apply else "Preview"}: {len(plan)} integration runners, {total / 1073741824:.2f} GiB apparent size; keeping {args.keep_runners} newest per package')
        if args.apply:
            for runner in plan:
                remove_runner(runner)
            print("Removed runners only; dependencies and application data preserved")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError) as error:
        raise SystemExit(str(error))
