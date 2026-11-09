#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real selected-model regression for managed workspaces and final delivery.

Uses disposable projects/Stores and an in-memory credential relay. Checks the
actual deployed program in an isolated interpreter, not the assistant's claim.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from test_real_cli_long_tasks import (
    selected_route, observed_provider, fixture, apply_config, command, require_completed,
)


def git(project, *args):
    return subprocess.run(['git', '-C', str(project), *args], check=True,
                          capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/nola'))
    parser.add_argument('--config', type=Path, default=Path.home()/'.nola/config.json')
    args = parser.parse_args()
    binary = args.binary.resolve()
    upstream, model, secret = selected_route(args.config)
    original = json.loads(args.config.read_text())['llm']
    selected = original['default_model']
    connection = next(c for c in original['connections'] if c['id'] == selected['connection'])
    declaration = next(m for m in connection['models'] if m['id'] == model)
    root = Path(tempfile.mkdtemp(prefix='nola-real-workspace-delivery-'))
    report = {'root': str(root), 'model': model,
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    print(json.dumps({'evidence': str(root), 'model': model}), flush=True)
    try:
        with observed_provider(upstream, model, secret, root/'provider.ndjson') as endpoint:
            project, env, config = fixture(root/'case', model, endpoint)
            settings = json.loads(config.read_text())
            settings['llm']['connections'][0]['models'] = [declaration]
            settings['llm']['connections'][0]['api_key_env'] = 'NOLA_BENCHMARK_TOKEN'
            env['NOLA_BENCHMARK_TOKEN'] = 'relay-only'
            settings.pop('compaction', None)
            config.write_text(json.dumps(settings))
            git(project, 'init', '--quiet')
            (project/'input.csv').write_text('region,amount\n北区,2.5\n南区,4\n北区,-0.5\n\n')
            input_digest = hashlib.sha256((project/'input.csv').read_bytes()).hexdigest()
            git(project, 'add', 'input.csv')
            git(project, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@localhost',
                'commit', '-qm', 'input fixture')
            # Deliberately no usable persistent identity. This must be resolved
            # before mutation, using an explicit per-invocation agent identity.
            git(project, 'config', 'user.name', '')
            git(project, 'config', 'user.email', '')
            git(project, 'config', 'commit.gpgsign', 'false')
            apply_config(binary, project, env, config, root/'case')
            prompt = ('在项目根目录交付 report.py，仅用 Python 标准库。运行 python3 report.py 输入CSV路径，'
                      '按 region 汇总 amount，忽略空行，stdout 只输出一个 JSON 对象，数值为和，保留 Unicode。'
                      '先检查现有 input.csv。使用正常托管工作区实现并集成到本项目，保护输入文件。'
                      '如果工作区交付需要 Git 提交，明确使用仅本次调用的 Nola Agent / nola@localhost 身份，'
                      '不要更改 Git 配置。请在最终项目位置验证正常输入、负数和空行；不要只报告子工作区或临时环境通过。')
            result = command(binary, project, env, root/'implementation', prompt,
                              'workspace-delivery', iterations=0, timeout=600)
            report['implementation'] = result
            require_completed(result)
            for line in (root/'implementation/events.ndjson').read_text().splitlines():
                trace = json.loads(line).get('trace') or {}
                payload = trace.get('payload') or {}
                if trace.get('kind') == 'tool.completed' and payload.get('tool_name') == 'bash':
                    if (payload.get('arguments') or {}).get('operation_kind') == 'validate':
                        assert 'dedicated_tool_required' not in payload.get('detail', ''), 'Final validation blocked by cwd coordinator guard'

            assert (project/'report.py').is_file(), 'No program at final requested destination'
            check = subprocess.run(['python3', '-I', str(project/'report.py'), str(project/'input.csv')],
                                   cwd=root, env=env, capture_output=True, text=True, timeout=20)
            assert check.returncode == 0, 'Final isolated interpreter could not execute delivery'
            assert json.loads(check.stdout) == {'北区': 2.0, '南区': 4.0}, 'Incorrect delivered behavior'
            unseen = root/'unseen.csv'
            unseen.write_text('region,amount\n"West, coast",1.75\nNorth,0\n"West, coast",-2.25\n\n')
            extra = subprocess.run(['python3', '-I', str(project/'report.py'), str(unseen)],
                                   cwd=root, env=env, capture_output=True, text=True, timeout=20)
            assert extra.returncode == 0 and json.loads(extra.stdout) == {'West, coast': -0.5, 'North': 0}, 'Unseen CSV behavior failed'
            assert hashlib.sha256((project/'input.csv').read_bytes()).hexdigest() == input_digest, 'Input file was changed'

            before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in [project/'report.py', project/'input.csv']}
            status_before = git(project, 'status', '--porcelain')
            followup = command(binary, project, env, root/'followup',
                'Search the current workspace and explain where the delivered program handles blank rows '
                'and negative amounts. If a workspace_audit helper is unavailable, use the available '
                'read-only tools. Do not edit anything or access the network.', 'workspace-delivery', iterations=0, timeout=300)
            report['followup'] = followup
            require_completed(followup)
            assert git(project, 'status', '--porcelain') == status_before, 'Read-only follow-up changed project state'
            assert all(hashlib.sha256((project/name).read_bytes()).hexdigest() == digest
                       for name, digest in before.items()), 'Read-only follow-up changed delivered bytes'
            assert git(project, 'config', 'user.name').strip() == ''
            assert git(project, 'config', 'user.email').strip() == ''
            report['final_destination_verified'] = True
            report['persistent_git_identity_unchanged'] = True
        rows = [json.loads(line) for line in (root/'provider.ndjson').read_text().splitlines() if line]
        forwarded = [row for row in rows if row.get('forwarded') is not False]
        report['provider_requests'] = len(forwarded)
        report['provider_http_200'] = sum(row.get('status') == 200 for row in forwarded)
        report['unforwarded_catalog_requests'] = sum(row.get('upstreamPath', '').endswith('/models') for row in rows if row.get('forwarded') is False)
        assert all(row.get('status') == 200 and not row.get('error') for row in forwarded), 'A real generation failed'
        report['passed'] = True
    except Exception as error:
        report['passed'] = False
        report['failure'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        (root/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
        print(json.dumps({k:v for k,v in report.items()
                          if k not in {'implementation', 'followup'}}, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
