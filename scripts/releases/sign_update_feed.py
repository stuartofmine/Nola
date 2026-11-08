#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Offline Ed25519 release signing. Requires OpenSSL 3; never copies a private key."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile


def public_key(openssl, key):
    if os.name != 'nt' and stat.S_IMODE(key.stat().st_mode) & 0o077:
        raise ValueError('Private signing key must be readable only by its owner')
    der = subprocess.check_output([openssl, 'pkey', '-in', str(key), '-pubout', '-outform', 'DER'], stderr=subprocess.PIPE)
    prefix = bytes.fromhex('302a300506032b6570032100')
    if len(der) != 44 or not der.startswith(prefix):
        raise ValueError('Expected an Ed25519 private key')
    return base64.b64encode(der[len(prefix):]).decode('ascii')


def sign(args):
    key = Path(args.private_key).resolve(strict=True)
    public = public_key(args.openssl, key)
    if args.command == 'public-key':
        print(public)
        return
    source = json.loads(Path(args.source).read_text())
    manifest = json.loads(Path(args.manifest).read_text())
    if source['schema_version'] != 1 or manifest['schema_version'] != 1 or not source.get('feed_url'):
        raise ValueError('A current, configured public release source is required')
    if public != source.get('public_key'):
        raise ValueError('Signing key does not match the public key pinned in the shipped source')
    for field in ('version', 'build', 'channel'):
        if source[field] != manifest[field]:
            raise ValueError(f'Release source and manifest disagree on {field}')
    root = Path(args.artifact_dir).resolve(strict=True)
    for target in manifest['targets']:
        artifact = target['artifact']
        if artifact['kind'] == 'store':
            continue
        filename = artifact['filename']
        path = (root / filename).resolve(strict=True)
        if path.parent != root or path.name != filename or not path.is_file() or path.is_symlink():
            raise ValueError('Every package must be a regular file directly in artifact-dir')
        digest = hashlib.sha256()
        size = 0
        with path.open('rb') as handle:
            for chunk in iter(lambda: handle.read(65536), b''):
                digest.update(chunk); size += len(chunk)
        if size <= 0 or size > 2 * 1024 * 1024 * 1024:
            raise ValueError('Package size out of bounds')
        artifact['size'] = size
        artifact['sha256'] = digest.hexdigest()
    payload = json.dumps(manifest, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    with tempfile.TemporaryDirectory(prefix='nola-release-sign-') as directory:
        data = Path(directory) / 'payload.json'; data.write_bytes(payload)
        signature = subprocess.check_output([args.openssl, 'pkeyutl', '-sign', '-rawin', '-inkey', str(key), '-in', str(data)], stderr=subprocess.PIPE)
    if len(signature) != 64:
        raise ValueError('Invalid Ed25519 signature length')
    output = Path(args.output)
    if output.exists():
        raise ValueError('Output already exists; choose a new output path')
    envelope = {'payload_base64': base64.b64encode(payload).decode('ascii'), 'signature_base64': base64.b64encode(signature).decode('ascii')}
    with output.open('x') as handle:
        json.dump(envelope, handle, indent=2); handle.write('\n')
    print(f'Signed feed: {output}. Verify it with the Rust verify_feed tool before publishing.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for command in ('public-key', 'sign'):
        item = sub.add_parser(command)
        item.add_argument('--openssl', default=os.environ.get('OPENSSL_BIN', 'openssl'))
        item.add_argument('--private-key', required=True)
        if command == 'sign':
            for name in ('source', 'manifest', 'artifact-dir', 'output'):
                item.add_argument('--' + name, required=True)
    try:
        sign(parser.parse_args())
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Release signing failed: {error}\n')


if __name__ == '__main__':
    main()
