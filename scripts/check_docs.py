#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check public documentation paths, local-record isolation and prototype inventory."""
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def main():
    errors = []
    links = 0
    documents = sorted((ROOT / 'docs').rglob('*.md'))
    documents += [ROOT / path for path in ['README.md', 'README.zh-CN.md', 'AGENTS.md', 'cli/README.md', 'release/README.md']]
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
        for path in ['Doc/当前任务/record.md', 'Doc/历史任务/record.json']:
            ignored = subprocess.run(['git', 'check-ignore', '--no-index', '-q', path], cwd=ROOT)
            if ignored.returncode:
                errors.append(f'Local record is not ignored: {path}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'Passed: {len(documents)} documents, {links} repository links, prototype assets/sources, and local-record policy')


if __name__ == '__main__':
    main()
