# SPDX-License-Identifier: Apache-2.0
import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('portable_paths', Path(__file__).resolve().parents[1] / 'check_portable_paths.py')
GUARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GUARD)


class PortablePathTests(unittest.TestCase):
    def run_guard(self, files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            subprocess.run(['git', 'add', '.'], cwd=root, check=True)
            with patch.object(GUARD, 'ROOT', root), contextlib.redirect_stdout(io.StringIO()):
                GUARD.main()

    def test_tracked_machine_home_paths_block_public_sources(self):
        slash = chr(92)
        for value in ['/' + 'Users/developer-a/project', '/' + 'home/developer-b/project',
                      'C:' + slash + 'Users' + slash + 'developer-c' + slash + 'project']:
            with self.subTest(value=value), self.assertRaisesRegex(SystemExit, 'source.py:2: developer home path'):
                self.run_guard({'source.py': '# source\n' + value})

    def test_machine_scratch_paths_block_public_records(self):
        value = '/' + 'var/folders/ab/random-machine-root/T/evidence'
        with self.assertRaisesRegex(SystemExit, 'record.md:1: machine-specific temporary path'):
            self.run_guard({'record.md': value})

    def test_relative_references_and_anonymous_security_fixtures_remain_valid(self):
        self.run_guard({'source.py': "path = './apps/cli'", 'tests/fixture.txt': '/Users/example/project'})

    def test_local_only_history_and_pinned_external_sources_are_outside_public_scan(self):
        value = '/' + 'Users/developer-a/private'
        self.run_guard({'legacy/backend/local.py': value, 'Doc/当前任务/record.md': value,
                        'third_party/upstream/fixture.txt': value, 'source.py': "path = './core'"})

    def test_first_party_benchmark_harnesses_are_checked(self):
        value = '/' + 'Users/developer-a/local-harness'
        with self.assertRaisesRegex(SystemExit, 'run.py:1: developer home path'):
            self.run_guard({'evals/benchmarks/terminal-bench/run.py': value})
