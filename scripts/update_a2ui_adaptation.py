#!/usr/bin/env python3
"""Regenerate the reviewed production renderer inventory and patch; never changes sources."""
import difflib
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / 'web/frontend/src/vendor/a2ui-svelte'
REFERENCE = ROOT / 'third_party/a2ui'

def sha(data):
    return hashlib.sha256(data).hexdigest()

def verify_sources():
    lock = json.loads((REFERENCE / 'sources.lock.json').read_text())
    count = 0
    for source in lock['sources']:
        for entry in source['files']:
            path = REFERENCE / source['name'] / entry['path']
            if sha(path.read_bytes()) != entry['sha256']:
                raise ValueError(f"Locked reference source changed: {path.relative_to(ROOT)}")
            count += 1
    return count

def artifacts():
    lock = json.loads((REFERENCE / 'sources.lock.json').read_text())
    upstream = next(source for source in lock['sources'] if source['name'] == 'svelte')
    files, patch = [], []
    for target in sorted(path for path in VENDOR.rglob('*') if path.is_file()):
        relative = target.relative_to(VENDOR).as_posix()
        origin = REFERENCE / 'svelte' / (relative if relative == 'LICENSE' else 'src/lib/' + relative)
        before = origin.read_bytes() if origin.is_file() else b''
        after = target.read_bytes()
        source = origin.relative_to(ROOT).as_posix() if origin.is_file() else None
        files.append({'path': relative, 'sha256': sha(after), 'source': source,
                      'source_sha256': sha(before) if source else None})
        patch.extend(difflib.unified_diff(before.decode().splitlines(True), after.decode().splitlines(True),
                     fromfile=source or '/dev/null', tofile=target.relative_to(ROOT).as_posix()))
    data = ''.join(patch).encode()
    manifest = {'wire_version': 'v0.9.1', 'upstream_commit': upstream['commit'],
                'production_root': VENDOR.relative_to(ROOT).as_posix(),
                'patch': 'production-adaptation.patch', 'patch_sha256': sha(data), 'files': files}
    return manifest, data

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Verify locked sources and production artifacts without writing')
    args = parser.parse_args()
    source_count = verify_sources()
    manifest, patch = artifacts()
    if args.check:
        if patch != (REFERENCE / 'production-adaptation.patch').read_bytes() or manifest != json.loads((REFERENCE / 'production-adaptation.json').read_text()):
            raise SystemExit('Production adaptation artifacts differ; review changes and regenerate the inventory')
        print(f"Verified {source_count} locked sources and {len(manifest['files'])} production renderer files")
    else:
        (REFERENCE / 'production-adaptation.patch').write_bytes(patch)
        (REFERENCE / 'production-adaptation.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(f"Recorded {len(manifest['files'])} production renderer files")
