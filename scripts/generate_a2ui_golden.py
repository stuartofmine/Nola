#!/usr/bin/env python3
"""Regenerate the synthetic Core A2UI golden without exposing partial files."""
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    destination = root / "tests/fixtures/a2ui/form_interaction_v0_9_1.json"
    generated = subprocess.run(
        ["cargo", "run", "-p", "nola_kernel", "--example", "a2ui_golden", "--quiet", "-j2"],
        cwd=root, check=True, stdout=subprocess.PIPE,
    ).stdout
    json.loads(generated)  # Never replace a valid fixture with compiler/error output.
    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".a2ui-", delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(generated)
            output.flush()
            os.fsync(output.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
