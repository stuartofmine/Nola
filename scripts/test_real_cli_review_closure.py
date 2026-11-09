#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real selected-route review remediation in a disposable project and Store.

Checks actual failure/pass receipts, immutable regression expectations and the
final delivered program. Never substitutes model responses or touches user data.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sqlite3
import tempfile

from test_real_cli_long_tasks import (
    selected_route, observed_provider, fixture, apply_config, command, require_completed,
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/nola'))
    parser.add_argument('--config', type=Path, default=Path.home()/'.nola/config.json')
    args = parser.parse_args()
    binary = args.binary.resolve()
    upstream, model, secret = selected_route(args.config)
    root = Path(tempfile.mkdtemp(prefix='nola-real-review-closure-'))
    report = {'root': str(root), 'model': model,
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    print(json.dumps({'evidence': str(root), 'model': model}), flush=True)
    review_text = ('R1: average truncates fractional values before averaging.\n'
                   'R2: average([]) raises; the agreed empty-input result is 0.\n')
    tests = ('import sys\nfrom average import average\n'
             'if sys.argv[1] == "decimal":\n    assert average([1.5, 2.25]) == 1.875\n'
             'elif sys.argv[1] == "empty":\n    assert average([]) == 0\n'
             'else:\n    raise AssertionError("unknown case")\n')
    try:
        with observed_provider(upstream, model, secret, root/'provider.ndjson') as endpoint:
            project, env, config = fixture(root/'case', model, endpoint)
            settings = json.loads(config.read_text())
            settings['llm']['connections'][0]['api_key_env'] = 'NOLA_BENCHMARK_TOKEN'
            env['NOLA_BENCHMARK_TOKEN'] = 'relay-only'
            settings.pop('compaction', None)
            config.write_text(json.dumps(settings))
            (project/'average.py').write_text('def average(values):\n    return sum(int(x) for x in values) / len(values)\n')
            (project/'regression.py').write_text(tests)
            (project/'REVIEW.md').write_text(review_text)
            def git(*items):
                subprocess.run(['git', '-C', str(project), *items], check=True, capture_output=True)
            git('init', '--quiet')
            git('config', 'user.name', 'Isolated Fixture')
            git('config', 'user.email', 'fixture@localhost')
            git('config', 'commit.gpgsign', 'false')
            git('add', 'average.py', 'regression.py', 'REVIEW.md')
            git('commit', '-qm', 'review fixture')
            apply_config(binary, project, env, config, root/'case')
            prompt = ('修复 REVIEW.md 中已经采纳的 R1、R2，两条都要闭环。先读取相关文件，'
                      '用 task_scope_update 分别登记审查问题和证据；构造反例后补充精确检查标识即可。'
                      '项目中的 regression.py 是独立验收，不得修改它或 REVIEW.md。'
                      '修复前分别运行 python3 regression.py decimal 和 python3 regression.py empty，'
                      '确认两条各自真实非零失败。使用 foreground validate，并将 regression.py 声明为固定回归测试文件，'
                      'validation_targets 包含 average.py、regression.py。然后只修 average.py。'
                      '不要联网。若仓库实现需要托管工作区，正常使用 workspace 子任务只修 average.py，父级负责保留原始失败证据、审查交付并集成。修复后在最终项目位置重跑完全相同的命令，'
                      '关联原失败、修复动作和最终版本的通过凭据；未闭环不能报告完成。工作区交付所需的提交使用已有 Fixture 身份或仅本次调用的 Fixture / fixture@localhost，不改 Git 配置；集成后无需再提交原项目。')
            result = command(binary, project, env, root/'remediation', prompt,
                             'review-closure', iterations=0, timeout=600)
            report['remediation'] = result
            require_completed(result)
            assert (project/'regression.py').read_text() == tests, 'Regression expectations changed'
            assert (project/'REVIEW.md').read_text() == review_text, 'Original review evidence changed'
            check = subprocess.run(['python3', '-I', '-c',
                'import importlib.util, sys; s=importlib.util.spec_from_file_location("delivered",sys.argv[1]); '
                'm=importlib.util.module_from_spec(s); s.loader.exec_module(m); '
                'assert m.average([1.5,2.25])==1.875; assert m.average([])==0; '
                'assert m.average([-2.5,0.5])==-1', str(project/'average.py')],
                cwd=root, env=env, capture_output=True, text=True, timeout=20)
            assert check.returncode == 0, f'Final isolated program check failed: {check.stderr}'
            with sqlite3.connect(Path(env['NOLA_DB_PATH']).resolve().as_uri()+'?mode=ro', uri=True) as db:
                canonical = [(kind, json.loads(raw)) for kind, raw in db.execute(
                    "SELECT e.event_kind,e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id "
                    "WHERE r.conversation_id=? AND e.event_kind IN ('requirement_scope_changed','execution_evidence_observed') "
                    "ORDER BY e.sequence", ('review-closure',))]
            observations = {value['evidence']['evidence_id']: value for kind, value in canonical
                            if kind == 'execution_evidence_observed'}
            scopes = [value['scope'] for kind, value in canonical if kind == 'requirement_scope_changed']
            assert scopes, 'No canonical scope was registered'
            claims = [claim for requirement in scopes[-1]['requirements']
                      for claim in requirement['resource_scope'].get('required', []) if claim.get('review')]
            assert len(claims) == 2, 'Expected two retained adopted review findings'
            final_version = 'sha256:'+hashlib.sha256((project/'average.py').read_bytes()).hexdigest()
            paired = []
            for claim in claims:
                review = claim['review']
                assert all(source in observations for source in review['source_evidence'])
                failure = observations[review['counterexample_evidence']]
                failed = [check for check in failure['evidence']['receipt']['checks']
                          if not check['passed'] and check['resource_uri'] == claim['resource_uri']
                          and check['predicate'] == claim['predicate']]
                assert failed and failed[0]['version'] != final_version, 'Original failure was not linked'
                repairs = [value for value in observations.values()
                           if any(effect['resource_uri'] == claim['resource_uri']
                                  and effect['operation'] == claim['operation']
                                  and effect.get('after_version') == final_version
                                  for effect in (value['evidence'].get('receipt') or {}).get('effects', []))]
                passes = [value for value in observations.values()
                          if any(check['passed'] and check['resource_uri'] == claim['resource_uri']
                                 and check['predicate'] == claim['predicate'] and check['version'] == final_version
                                 for check in (value['evidence'].get('receipt') or {}).get('checks', []))]
                assert any(int(failure['core_envelope']['revision']) < int(repair['core_envelope']['revision'])
                           < int(passed['core_envelope']['revision']) for repair in repairs for passed in passes), 'Missing ordered final-resource repair/pass evidence'
                paired.append(claim['predicate'])
            assert len(set(paired)) == 2, 'The two independent regressions must have different identities'
            report.update(final_program_verified=True, unchanged_regression_expectations=True, unchanged_review_evidence=True,
                          paired_regression_identities=paired, review_bindings_observed=len(claims))
        rows = [json.loads(line) for line in (root/'provider.ndjson').read_text().splitlines() if line]
        generations = [row for row in rows if row.get('model') == model and row.get('requestInputPresent')]
        report['provider_requests'] = len(generations)
        report['blocked_catalog_probes'] = sum(not row.get('forwarded') for row in rows if row not in generations)
        report['transport_errors'] = sum(row.get('status') != 200 for row in generations)
        assert report['transport_errors'] == 0, 'Provider transport failed'
    finally:
        (root/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps({key: value for key, value in report.items() if key != 'remediation'}, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
