#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Reject developer home paths and machine-specific macOS scratch roots in public files."""
import importlib.util
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('public_export', ROOT / 'scripts/releases/export_public.py')
PUBLIC_EXPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLIC_EXPORT)
# These anonymous identities exercise home lookup, redaction and permission boundaries.
FIXTURE_IDENTITIES = {'example', 'person', 'me', 'real-user', 'fallback', 'user', 'fixture', 'u'}
HOME = re.compile(r'/(?:Users|home)/([A-Za-z0-9_][A-Za-z0-9_.-]*)/')
WINDOWS_HOME = re.compile(r'[A-Za-z]:[\\/]+Users[\\/]+([A-Za-z0-9_][A-Za-z0-9_.-]*)[\\/]+')
MAC_SCRATCH = re.compile(r'/(?:private/)?var/folders/[A-Za-z0-9]{2}/[^/\s]+/(?:T|C)/')


def violations(name, text):
    for pattern in (HOME, WINDOWS_HOME):
        for match in pattern.finditer(text):
            if match.group(1) not in FIXTURE_IDENTITIES:
                yield name, text.count('\n', 0, match.start()) + 1, 'developer home path'
    for match in MAC_SCRATCH.finditer(text):
        yield name, text.count('\n', 0, match.start()) + 1, 'machine-specific temporary path'


def main():
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    errors = []
    checked = 0
    for name in filter(None, names):
        if PUBLIC_EXPORT.excluded(name) or name.startswith(('evals/benchmarks/terminal-bench/tasks/', 'evals/benchmarks/longmemeval/cases/', 'third_party/', 'apps/android/app/src/main/cpp/third_party/', 'apps/cli/vendor/')):
            continue
        path = ROOT / name
        if not path.is_file() or path.is_symlink():
            continue
        try:
            text = path.read_text()
        except UnicodeError:
            continue
        errors.extend(violations(name, text))
        checked += 1
    if errors:
        raise SystemExit('\n'.join(f'{name}:{line}: {reason}' for name, line, reason in errors))
    print(f'Passed: {checked} public text files contain no developer home or machine scratch paths')


if __name__ == '__main__':
    main()
