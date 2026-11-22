#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check public documentation paths, local-record isolation and prototype inventory."""
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def check_architecture_languages(errors):
    architecture = ROOT / 'docs/architecture'
    indexes = {suffix: (architecture / f'README{suffix}.md').read_text()
               for suffix in ('', '.en')}
    for directory in sorted((architecture / '模块').glob('[0-9][0-9]-*')):
        for suffix, index in indexes.items():
            page = directory / f'README{suffix}.md'
            if not page.is_file() or f']({page.relative_to(architecture)})' not in index:
                errors.append(f'Missing module guide or index entry: {page.relative_to(ROOT)}')
    for english in sorted(architecture.rglob('*.en.md')):
        switch = re.search(r'\[简体中文\]\(([^)]+)\)', english.read_text())
        chinese = english.parent / switch.group(1) if switch else None
        if not chinese or not chinese.is_file() or f'[English]({english.name})' not in chinese.read_text():
            errors.append(f'Missing reciprocal language link: {english.relative_to(ROOT)}')
    requirements = architecture / '总体设计'
    identifiers = [set(re.findall(r'REQ-(?:ARCH|RUN)-\d+', (requirements / filename).read_text()))
                   for filename in ('REQUIREMENTS.md', 'REQUIREMENTS.en.md')]
    if identifiers[0] != identifiers[1]:
        errors.append(f'Architecture requirement IDs differ by language: {sorted(identifiers[0] ^ identifiers[1])}')


def main():
    errors = []
    check_architecture_languages(errors)
    links = 0
    documents = sorted((ROOT / 'docs').rglob('*.md'))
    documents += [ROOT / path for path in ['README.md', 'README.zh-CN.md', 'AGENTS.md', 'apps/README.md', 'shared/README.md', 'tests/apple/README.md', 'apps/cli/README.md', 'release/README.md']]
    for document in documents:
        text = re.sub(r'```.*?```', '', document.read_text(), flags=re.S)
        for destination in re.findall(r'\]\(([^)\n]+)\)', text):
            url = urlsplit(destination.strip('<>'))
            if url.scheme or not url.path:
                continue
            target = (document.parent / unquote(url.path)).resolve()
            links += 1
            if not target.is_relative_to(ROOT):
                errors.append(f'{document.relative_to(ROOT)}: link escapes repository: {destination}')
            elif target.is_relative_to(ROOT / 'Doc'):
                errors.append(f'{document.relative_to(ROOT)}: links to local-only records: {destination}')
            elif not target.exists():
                errors.append(f'{document.relative_to(ROOT)}: missing link: {destination}')
    for document in (ROOT / 'docs/design/prototype').glob('*.html'):
        for destination in re.findall(r'(?:src|href)="([^"?#]+)', document.read_text()):
            url = urlsplit(destination)
            if url.scheme or not url.path or url.path.startswith('/'):
                continue
            if not (document.parent / unquote(url.path)).exists():
                errors.append(f'{document.relative_to(ROOT)}: missing asset: {destination}')
    catalog = json.loads((ROOT / 'docs/design/prototype/BUSINESS-SCENARIOS.json').read_text())
    for scene in catalog['scenarios']:
        for source in scene['sources']:
            if source.startswith('Doc/') or not (ROOT / source).is_file():
                errors.append(f'{scene["id"]}: invalid source path: {source}')
    subprocess.run(['python3', str(ROOT / 'docs/design/prototype/sync_business_scenarios.py'), '--check'], cwd=ROOT, check=True)
    git = subprocess.run(['git', 'rev-parse', '--is-inside-work-tree'], cwd=ROOT, capture_output=True)
    if git.returncode == 0:
        tracked = subprocess.check_output(['git', 'ls-files', '-z', '--', 'Doc'], cwd=ROOT)
        if tracked:
            errors.append('Local Doc/ records are still tracked by Git')
        if subprocess.check_output(['git', 'ls-files', '-z', '--', 'legacy', 'backend'], cwd=ROOT):
            errors.append('Historical implementations are still tracked by Git')
        for path in ['Doc/当前任务/record.md', 'Doc/历史任务/record.json', 'legacy/backend/source.py', 'backend/source.py']:
            ignored = subprocess.run(['git', 'check-ignore', '--no-index', '-q', path], cwd=ROOT)
            if ignored.returncode:
                errors.append(f'Local record is not ignored: {path}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'Passed: {len(documents)} documents, {links} repository links, prototype assets/sources, and local-record policy')


if __name__ == '__main__':
    main()
