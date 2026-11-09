#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Read-only audit of local real-provider evidence; never starts inference.

The one-attempt/one-successful-request alignment is deliberately strict. Evidence
with partial, retried or multi-request summary attempts needs a separate audit.
Source hashes prove provenance, not semantic accuracy of a generated summary.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3


def audit(root, require_disabled_tools=False):
    root = Path(root)
    with closing(sqlite3.connect((root / 'case/nola.db').resolve().as_uri() + '?mode=ro', uri=True)) as db:
        events = [((run, seq), json.loads(raw)) for run, seq, raw in db.execute(
            'SELECT run_id,sequence,payload_json FROM run_events ORDER BY created_at_ms,sequence')]
        checkpoints = [json.loads(raw) for (raw,) in db.execute(
            'SELECT checkpoint_json FROM core_checkpoint_bodies')]
    items = {}
    for checkpoint in checkpoints:
        for item in checkpoint['state']['conversation']['transcript']:
            if item['id'] in items:
                assert items[item['id']] == item, 'canonical item changed between checkpoints'
            items[item['id']] = item
    requested = [e for _, e in events if e.get('type') == 'compaction_requested']
    completed = [e for _, e in events if e.get('type') == 'compaction_completed' and e.get('compaction_item_id')]
    attempts = [(seq, e) for seq, e in events if e.get('state') == 'succeeded'
        and e.get('type', 'model_attempt_changed') == 'model_attempt_changed']
    attempts.sort(key=lambda entry: entry[1]['completed_at_ms'])
    rows = [json.loads(line) for line in (root / 'provider.ndjson').read_text().splitlines()]
    rows = [r for r in rows if r.get('forwarded') is not False and r.get('status') == 200 and not r.get('error')]
    assert len(rows) == len(attempts), 'requires one successful request per succeeded attempt'
    summaries = []
    business = []
    for index, ((seq, attempt), wire) in enumerate(zip(attempts, rows)):
        usage = attempt['usage']
        assert usage['input_tokens'] == wire['responseUsage']['input_tokens'], 'provider input alignment mismatch'
        assert usage['output_tokens'] == wire['responseUsage']['output_tokens'], 'provider output alignment mismatch'
        assert wire['requestSha256'] == wire['upstreamRequestSha256'], 'relay changed input'
        if not (usage.get('cache_usage') or {}).get('is_compaction'):
            business.append((seq, usage['input_tokens']))
            continue
        if require_disabled_tools:
            assert wire['toolChoice'] == 'none', 'summary permits tool calls'
            assert index > 0 and wire['toolSchemaSha256'] == rows[index - 1]['toolSchemaSha256'], 'tool catalog changed'
        summaries.append({'sequence': seq, 'input_tokens': usage['input_tokens'],
            'cached_tokens': usage.get('cached_input_tokens'), 'output_tokens': usage['output_tokens'],
            'tool_choice': wire.get('toolChoice'),
            'duration_ms': attempt['completed_at_ms'] - attempt['started_at_ms']})
    assert completed, 'no actual committed compaction'
    assert len(summaries) == len(completed) == len(requested), 'summary lifecycle count mismatch'
    for row, request, result in zip(summaries, requested, completed):
        item = items[result['compaction_item_id']]
        sources = [items[key] for key in item['source_item_ids']]
        fingerprint = hashlib.sha256(json.dumps(sources, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
        assert fingerprint == item['source_fingerprint'] == result['source_fingerprint'] == request['source_fingerprint'], 'archive fingerprint mismatch'
        assert len(sources) == result['replaced_item_count'] == request['source_item_count'], 'archive count mismatch'
        assert item['summary'] == result['summary'], 'committed summary differs from event'
        before = next((tokens for seq, tokens in reversed(business) if seq[0] == row['sequence'][0] and seq[1] < row['sequence'][1]), None)
        after = next((tokens for seq, tokens in business if seq[0] == row['sequence'][0] and seq[1] > row['sequence'][1]), None)
        row.update(archive_estimated_tokens=result['original_token_count'], source_count=len(sources),
            requested_full_estimated_tokens=request['estimated_tokens'],
            preceding_business_input=before, following_business_input=after,
            business_input_reduction=None if before is None or after is None else before - after,
            source_fingerprint_verified=True)
    return {'compactions': summaries, 'all_sources_verified': True,
        'input_tokens': sum(r['input_tokens'] for r in summaries),
        'cached_tokens': sum(r['cached_tokens'] or 0 for r in summaries),
        'duration_ms': sum(r['duration_ms'] for r in summaries),
        'semantic_accuracy': 'requires comparison with the independent fixture oracle; hashes alone do not prove it'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('--require-disabled-tools', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    report = audit(args.evidence, args.require_disabled_tools)
    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.write_text(text + '\n')
    print(text)


if __name__ == '__main__':
    main()
