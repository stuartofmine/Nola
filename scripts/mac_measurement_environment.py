# SPDX-License-Identifier: Apache-2.0
"""Secret-free, opt-in measurement environment for an isolated Mac XCTest."""
import hashlib
import json
import os
from pathlib import Path


def configure(target, source_path, artifacts, enabled):
    environment = target.setdefault("EnvironmentVariables", {})
    for key in ("NOLA_MEASUREMENT_FILE", "NOLA_MEASUREMENT_BUILD_DIGEST"):
        environment.pop(key, None)
    if not enabled:
        return
    host = target.get("TestHostPath", "").replace(
        "__TESTROOT__", str(Path(source_path).resolve().parent))
    if not host or "__" in host:
        raise ValueError("measurement requires a resolved XCTest host")
    resolved_host = Path(host).resolve(strict=True)
    binaries = (resolved_host / "Contents/MacOS" if resolved_host.is_dir()
                and resolved_host.suffix == ".app" else resolved_host.parent)
    if binaries.name != "MacOS" or binaries.parent.name != "Contents":
        raise ValueError("measurement requires an app-bundle XCTest host")
    if not (binaries / "nola-agent-host").is_file():
        raise ValueError("measurement requires the actual embedded Agent Host")
    members = []
    for path in sorted(binaries.iterdir()):
        if not path.is_file():
            continue
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        members.append({"path": "Contents/MacOS/" + path.name,
                        "sha256": digest.hexdigest(), "bytes": path.stat().st_size})
    identity = {"schema_version": 1, "scope": "mac_app_macos_artifact_set", "members": members}
    build_digest = hashlib.sha256(json.dumps(identity, sort_keys=True,
        separators=(",", ":")).encode()).hexdigest()
    directory = Path(artifacts).resolve() / "measurement"
    directory.mkdir(mode=0o700)
    # Exclusive creation prevents appending a prior experiment or following a
    # caller-supplied symlink. No configuration/provider data enters this file.
    for name, content in (("events.jsonl", ""), ("build.json", json.dumps(
            {**identity, "build_digest": build_digest}, indent=2) + "\n")):
        descriptor = os.open(directory / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as handle:
            handle.write(content)
    environment.update(NOLA_MEASUREMENT_FILE=str(directory / "events.jsonl"),
                       NOLA_MEASUREMENT_BUILD_DIGEST=build_digest)
