#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in. Send one real Electron chat turn and verify its canonical result after relaunch."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[3]
WINDOWS = ROOT / 'apps/windows'
sys.path.insert(0, str(ROOT / 'scripts'))

from canonical_run_audit import audit_runs
from provider_stream_acceptance import require_text_stream
from provider_trace_metrics import provider_trace_metrics
from real_e2e_evidence import record_step
from real_e2e_judge import evaluate_receipt
from test_real_cli_long_tasks import apply_config, fixture, observed_provider, selected_route


def durable_answer(db, owner):
    canonical = audit_runs(db, owner)
    if canonical != {'runCount': 1, 'outcomes': ['answered']}:
        raise ValueError('Expected a single answered, sealed Run')
    run, assistant, result = db.execute(
        """SELECT r.id,r.assistant_message_id,rr.result_json FROM runs r JOIN run_results rr ON rr.run_id=r.id
        WHERE r.conversation_id=? AND r.kind='conversation'""", (owner,)).fetchone()
    parts = json.loads(result)['value']['answer_part_refs']
    answer = []
    for ref in parts:
        row = db.execute(
            """SELECT p.content_json FROM message_parts p JOIN messages m ON p.message_id=m.id
            WHERE p.id=? AND p.kind='text' AND p.state='completed' AND m.id=? AND m.role='assistant'
            AND m.conversation_id=? AND m.run_id=?""", (ref, assistant, owner, run)).fetchone()
        if not row:
            raise ValueError('Sealed answer references a missing text part')
        payload = json.loads(row[0])
        if payload.get('is_complete') is not True or payload.get('message_id') != assistant:
            raise ValueError('Incomplete or mismatched final text')
        answer.append(payload['text'])
    return canonical, run, ''.join(answer)


def durable_file_reads(db, owner, run, note, marker):
    rows = db.execute(
        """SELECT p.content_json FROM message_parts p JOIN messages m ON m.id=p.message_id
        WHERE m.conversation_id=? AND m.run_id=? AND m.role='assistant' AND p.kind='tool'""", (owner, run))
    reads = []
    for (payload,) in rows:
        part = json.loads(payload)
        result = part.get('structured_output') or {}
        if (part.get('state') != 'completed' or part.get('capability_id') != 'read_file'
                or result.get('complete') is not True or result.get('content') != marker + '\n'
                or Path(result.get('path', '')).resolve() != note.resolve()):
            raise ValueError('Run contains an unexpected tool or file result')
        source_kinds = {kind for (kind,) in db.execute(
            """SELECT kind FROM canonical_tool_sources WHERE conversation_id=? AND run_id=? AND part_id=?
            AND json_extract(binding_json,'$.call_id')=? AND json_extract(binding_json,'$.name')='read_file'""",
            (owner, run, part['id'], part['call_id']))}
        if source_kinds != {'tool_call', 'tool_result'}:
            raise ValueError('File read has no matching canonical call and result')
        reads.append({'call_id': part['call_id'].rsplit('::', 1)[-1], 'name': 'read_file',
                      'input': part['input'], 'read_content': result['content']})
    return reads


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-windows', action='store_true', help='Require a native Windows run for the platform matrix')
    parser.add_argument('--config', type=Path, default=os.environ.get('NOLA_CONFIG_PATH') or Path.home() / '.nola/config.json')
    default_binary = ROOT / 'target/debug' / ('nola.exe' if os.name == 'nt' else 'nola')
    parser.add_argument('--binary', type=Path, default=os.environ.get('NOLA_WINDOWS_CLI_PATH') or default_binary)
    args = parser.parse_args()
    if args.require_windows and sys.platform != 'win32':
        parser.error('Native Windows Electron acceptance requires Windows')
    requested = os.environ.get('NOLA_DESKTOP_MODEL_EVIDENCE')
    external = Path(requested).resolve() if requested else None
    if external:
        external.parent.mkdir(parents=True, exist_ok=True)
        if external.exists():
            parser.error('NOLA_DESKTOP_MODEL_EVIDENCE must be a new file')
    binary = args.binary.resolve(strict=True)
    upstream, model, credential = selected_route(args.config)
    evidence = Path(tempfile.mkdtemp(prefix='nola-electron-real-model-'))
    print('Evidence:', evidence, flush=True)
    report = {'passed': False, 'model': model, 'client': 'windows-electron', 'hostOS': sys.platform}
    marker = 'NOLA-DESKTOP-' + uuid.uuid4().hex.upper()
    calls = {}
    read_outputs = set()
    prompt_contained_marker = None

    def observe(request):
        nonlocal prompt_contained_marker
        items = request.get('input')
        if not isinstance(items, list) or request.get('model') != model:
            return
        if prompt_contained_marker is None:
            prompt_contained_marker = marker in json.dumps(items)
        for item in items:
            if not isinstance(item, dict):
                continue
            ident = item.get('call_id')
            if not ident:
                continue
            if item.get('type') == 'function_call':
                calls[ident] = {'name': item.get('name'), 'arguments': item.get('arguments')}
            if item.get('type') == 'function_call_output' and marker in json.dumps(item.get('output')):
                read_outputs.add(ident)

    try:
        with observed_provider(upstream, model, credential, evidence / 'provider.ndjson', request_observer=observe) as endpoint:
            store = evidence / 'store'
            project, env, config = fixture(store, model, endpoint, profile='cautious')
            env.update(HOME=str(store / 'home'), USERPROFILE=str(store / 'home'),
                       NOLA_E2E_USER_DATA=str(store / 'electron'), NOLA_WINDOWS_CLI_PATH=str(binary),
                       NOLA_ELECTRON_MODEL_EVIDENCE=str(evidence))
            workspace = store / 'electron' / 'Nola Workspace'
            workspace.mkdir(parents=True)
            note = workspace / 'handoff-note.txt'
            expected_bytes = (marker + '\n').encode('ascii')
            note.write_bytes(expected_bytes)
            if os.name == 'nt':
                for name in ['SystemRoot', 'WINDIR', 'PATHEXT', 'ComSpec', 'TEMP', 'TMP']:
                    if name in os.environ:
                        env[name] = os.environ[name]
                for name in ['APPDATA', 'LOCALAPPDATA']:
                    directory = store / name
                    directory.mkdir()
                    env[name] = str(directory)
            npm = shutil.which('npm', path=env['PATH'])
            node = shutil.which('node', path=env['PATH'])
            if not npm or not node:
                raise RuntimeError('Node and npm are required')
            for cwd in [ROOT / 'apps/web/frontend', WINDOWS]:
                subprocess.run([npm, 'run', 'build'], cwd=cwd, env=env, check=True,
                               stdout=subprocess.DEVNULL, timeout=240)
            apply_config(binary, project, env, config, store)
            with (evidence / 'renderer.log').open('w') as log:
                subprocess.run([node, 'tests/realModelAcceptance.cjs'], cwd=WINDOWS, env=env,
                               stdout=log, stderr=subprocess.STDOUT, check=True, timeout=360)
            ui = json.loads((evidence / 'renderer.json').read_text())
            if not ui['passed'] or not ui['restored']:
                raise ValueError('Renderer did not confirm a restored model reply')
            database = Path(env['NOLA_DB_PATH'])
            with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as db:
                canonical, run, reply = durable_answer(db, ui['conversation'])
                durable_reads = durable_file_reads(db, ui['conversation'], run, note, marker)
            if not reply or not (reply == ui['expectedToken'] == ui['response'] == ui['restoredReply']):
                raise ValueError('Provider answer, reopened renderer, and durable conversation disagree')
            trace = evidence / 'provider.ndjson'
            records = [json.loads(line) for line in trace.read_text().splitlines()]
            responses = [r for r in records if r.get('forwarded') is True and r.get('model') == model]
            streams = require_text_stream(responses)
            metrics = provider_trace_metrics([trace])
            if metrics['malformedRows'] or metrics['completedModelStreams'] < streams:
                raise ValueError('Upstream trace is incomplete or inconsistent')
            tool_names = {name for response in responses for name in response.get('responseToolNames', [])}
            verified = [dict(call_id=ident, **calls[ident]) for ident in sorted(read_outputs)
                        if ident in calls and calls[ident]['name'] == 'read_file'
                        and Path(json.loads(calls[ident]['arguments'])['path']).name == note.name]
            if (prompt_contained_marker is not False or not verified or tool_names != {'read_file'}
                    or {r['call_id'] for r in durable_reads} != {r['call_id'] for r in verified}
                    or note.read_bytes() != expected_bytes):
                raise ValueError(f'No matching upstream file read, result and unchanged fixture (tools: {sorted(tool_names)})')
            assertions = {'model_reply_restored': True, 'single_sealed_run': True, 'workspace_file_read_verified': True}
            audit_path = evidence / 'persistence-audit.json'
            audit_path.write_text(json.dumps({'passed': True, 'conversation_id': ui['conversation'],
                'run_id': run, 'canonical': canonical, 'persisted_reply': reply, 'expected_token': marker, 'assertions': assertions,
                'verified_read_file': verified, 'durable_file_reads': durable_reads}, indent=2))
            observations = [{'id': identifier, 'kind': kind, 'path': evidence / filename}
                            for identifier, kind, filename in (
                                ('provider', 'provider_trace', 'provider.ndjson'),
                                ('run-audit', 'persistence_audit', 'persistence-audit.json'),
                                ('renderer', 'test_report', 'renderer.json'),
                                ('desktop-screenshot', 'ui_screenshot', 'restored.png'),
                                ('desktop-accessibility', 'accessibility_tree', 'accessibility.txt'),
                                ('desktop-timeline', 'ui_timeline', 'timeline.json'),
                                ('desktop-transcript', 'transcript', 'transcript.json'))]
            matrix = json.loads((ROOT / 'scripts/real_e2e_matrix.json').read_text(encoding='utf-8'))
            contract = matrix['contracts']['windows-model-relaunch']
            checks = evaluate_receipt({'steps': [{'observations': observations}]}, {'_contract': contract})
            (evidence / 'checks.json').write_text(json.dumps(checks, indent=2))
            if not checks['passed']:
                raise ValueError(checks['errors'])
            report.update(passed=True, persistence_audit=str(audit_path), verified_read_file=verified,
                          observations={item['kind']: str(item['path']) for item in observations},
                          platform=ui['os'], canonical=canonical, conversation_id=ui['conversation'],
                          run_id=run, expected_token=ui['expectedToken'], persisted_reply=reply, restored=True,
                          provider_trace=str(trace), providerTextStreams=streams, database=str(database),
                          renderer_evidence=str(evidence / 'renderer.json'),
                          provider_models=sorted({m for r in responses for m in r.get('responseModels', [])}))
            if sys.platform == 'win32' and ui['os'] == 'win32' and os.environ.get('NOLA_E2E_RECEIPT_PATH'):
                record_step('windows-model-relaunch', platforms=['windows'],
                    modules=['agent', 'core', 'provider', 'persistence', 'tools', 'ui'],
                    boundaries=['electron-renderer', 'windows-ui', 'local-host', 'provider', 'sqlite', 'filesystem'],
                    assertions=assertions,
                    assertion_evidence={'model_reply_restored': ['renderer', 'provider', 'run-audit', 'desktop-transcript',
                                                                'desktop-accessibility', 'desktop-screenshot', 'desktop-timeline'],
                                        'single_sealed_run': ['run-audit'], 'workspace_file_read_verified': ['run-audit', 'provider']},
                    observations=observations)
                print('Open desktop-screenshot and attach its findings with scripts/attach_visual_review.py.', flush=True)
            if external:
                with external.open('x', encoding='utf-8') as file:
                    json.dump(report, file, indent=2)
                    file.write('\n')
    finally:
        (evidence / 'result.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
