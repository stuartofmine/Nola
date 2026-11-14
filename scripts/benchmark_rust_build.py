#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare isolated cold, warm and leaf-rebuild Rust library builds without clearing user targets.

Run cargo fetch --locked first. Registry downloads and cache restores are excluded.
Only temporary target directories created by this process are removed.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', required=True)
    parser.add_argument('--variants', default='16:2,256:2,256:4', help='workspace codegen-units:jobs pairs; registry codegen stays 16')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    variants = []
    for value in args.variants.split(','):
        try:
            units, jobs = map(int, value.split(':'))
        except ValueError:
            parser.error('variants must be comma-separated codegen-units:jobs pairs')
        if not 1 <= units <= 1024 or not 1 <= jobs <= 32:
            parser.error('variants require 1..1024 codegen units and 1..32 jobs')
        variants.append((units, jobs))
    cargo = os.environ.get('CARGO', 'cargo')
    metadata = json.loads(subprocess.check_output([cargo, 'metadata', '--locked', '--offline', '--no-deps', '--format-version=1', '--manifest-path', str(ROOT / 'Cargo.toml')]))
    members = set(metadata['workspace_members'])
    if not any(p['id'] in members and p['name'] == args.package and any('lib' in t['kind'] for t in p['targets']) for p in metadata['packages']):
        parser.error('select a workspace package with a library target')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # Do not inherit a compiler cache or caller-selected incremental behavior.
    env = {**os.environ, 'RUSTC_WRAPPER': '', 'RUSTC_WORKSPACE_WRAPPER': '', 'CARGO_INCREMENTAL': '0'}
    results = []
    for index, (units, jobs) in enumerate(variants):
        label = f'{index + 1}-cgu{units}-jobs{jobs}'
        with tempfile.TemporaryDirectory(prefix=label + '-', dir=output) as directory:
            target = Path(directory) / 'target'
            command = [cargo, 'build', '--locked', '--offline', '--manifest-path', str(ROOT / 'Cargo.toml'), '-p', args.package, '--lib', '--profile', 'dev', '--jobs', str(jobs), '--target-dir', str(target), '--timings', '--message-format=json-render-diagnostics', '--config', f'profile.dev.codegen-units={units}', '--config', 'profile.dev.package."*".codegen-units=16']
            result = {'variant': label, 'codegen_units': units, 'jobs': jobs}
            for phase in ['cold', 'warm', 'rebuild']:
                if phase == 'rebuild':
                    # Only this run's owned target; retain dependency artifacts.
                    clean = [cargo, 'clean', '--locked', '--offline', '--manifest-path', str(ROOT / 'Cargo.toml'), '-p', args.package, '--profile', 'dev', '--target-dir', str(target)]
                    subprocess.run(clean, env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                print(f'Running {label} {phase}', flush=True)
                started = time.monotonic()
                with (output / f'{label}-{phase}.jsonl').open('w') as stdout, (output / f'{label}-{phase}.log').open('w') as stderr:
                    process = subprocess.run(command, env=env, stdout=stdout, stderr=stderr)
                seconds = round(time.monotonic() - started, 3)
                if process.returncode:
                    raise SystemExit(f'{label} {phase} failed; see {label}-{phase}.log (owned temporary target removed)')
                artifacts = [json.loads(line) for line in (output / f'{label}-{phase}.jsonl').read_text().splitlines() if line.startswith('{')]
                result[phase] = {'seconds': seconds, 'compiled_artifacts': sum(event.get('reason') == 'compiler-artifact' and event.get('fresh') is False for event in artifacts)}
                timing = target / 'cargo-timings/cargo-timing.html'
                if timing.exists():
                    shutil.copy2(timing, output / f'{label}-{phase}.html')
                print(f'{label} {phase}: {seconds}s, {result[phase]["compiled_artifacts"]} compiled artifacts', flush=True)
            result['target_allocated_bytes'] = int(subprocess.check_output(['du', '-sk', str(target)]).split()[0]) * 1024
            results.append(result)
            (output / 'results.json').write_text(json.dumps({'package': args.package, 'profile': 'dev', 'incremental': False, 'compiler_cache': False, 'registry_codegen_units': 16, 'results': results}, indent=2) + '\n')
    print('All benchmark targets removed; reports retained', flush=True)


if __name__ == '__main__':
    main()
