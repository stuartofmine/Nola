#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-model ordinary external reads in cautious mode; isolated home/database."""
import argparse
import json
import os
from pathlib import Path
import secrets
import tempfile
import subprocess
from run_tool_reference_compaction_e2e import canonical_results, run_prompt

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/nola')
    parser.add_argument('--config', type=Path, default=Path.home() / '.nola/config.json')
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix='nola-external-resources-'))
    root.chmod(0o700)
    workspace, external, home = (root / name for name in ('workspace', 'external', 'home'))
    for path in (workspace, external, home / '.nola'):
        path.mkdir(parents=True)
    config = home / '.nola/config.json'
    settings = {'llm': json.loads(args.config.read_text())['llm'], 'allowed_dirs': [str(workspace)]}
    with config.open('x') as handle:
        config.chmod(0o600)
        json.dump(settings, handle)
    markers = [secrets.token_hex(16) for _ in range(2)]
    for name, marker in zip(('first.txt', 'second.txt'), markers):
        (external / name).write_text(marker + '\n')
    database = root / 'state.db'
    session = 'external-' + secrets.token_hex(8)
    environment = {**os.environ, 'NOLA_HOME': str(home), 'NOLA_DB_PATH': str(database), 'NOLA_SKIP_ONBOARDING': '1'}
    # No --yes / allow_all / injected tool selection: ordinary reads must need no approval.
    command = [str(args.binary.resolve()), '--project', str(workspace), '--session', session,
               '--json', '--max-iterations', '8']
    prompts = [
        f'读取 {external / "first.txt"}，告诉我其中的校验码。',
        f'继续，看看同一目录的 second.txt，告诉我里面的校验码。',
        f'Search the current directory {external} for first.txt and report its verification code.'
    ]
    report = {'passed': False, 'evidence': str(root), 'session': session}
    print(f'Evidence: {root}', flush=True)
    try:
        with (root / 'import.log').open('wb') as log:
            imported = subprocess.run([str(args.binary.resolve()), 'model', 'apply-config', str(config)],
                cwd=workspace, env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=30)
        assert imported.returncode == 0, 'Isolated model import failed; inspect import.log'
        for index, prompt in enumerate(prompts):
            run_prompt(command, prompt, workspace, environment, root / f'turn-{index}.ndjson', 180)
        results = canonical_results(database, session)
        successful = [result for result in results if not result.get('is_error')]
        for marker in markers:
            assert any(marker in json.dumps(result['output']) for result in successful), 'Missing actual external file evidence'
        assert not any(result.get('name') == 'workspace_access' for result in results), 'Unnecessary directory grant'
        assert not any('workspace_access_required' in json.dumps(result) for result in results)
        import sqlite3
        with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as db:
            approvals = db.execute("SELECT count(*) FROM message_parts WHERE kind='tool' AND json_type(content_json,'$.permission')='object'").fetchone()[0]
        assert approvals == 0, 'Ordinary external reads requested approval'
        report.update(passed=True, completed_turns=len(prompts), tool_results=len(results), approvals=approvals)
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        config.unlink(missing_ok=True)
        (root / 'summary.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
