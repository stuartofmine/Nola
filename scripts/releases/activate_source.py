#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Activate a publisher source only after live signature and package verification."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import stat
import tempfile
import urllib.request
from urllib.parse import urlsplit

MAX_FEED = 1024 * 1024
MAX_PACKAGE = 2 * 1024 * 1024 * 1024


def redirect_allowed(previous, following):
    old, new = urlsplit(previous), urlsplit(following)
    if any(url.scheme != 'https' or not url.hostname or url.username or url.password or url.fragment for url in (old, new)):
        return False
    return (old.hostname, old.port or 443) == (new.hostname, new.port or 443) or (
        old.hostname in ('github.com', 'api.github.com') and new.hostname == 'release-assets.githubusercontent.com'
        and (old.port or 443) == 443 and (new.port or 443) == 443)


class UpdateRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        hops = getattr(req, '_nola_update_redirects', 0) + 1
        if hops > 5 or not redirect_allowed(req.full_url, newurl):
            raise ValueError('Untrusted or excessive update redirect')
        following = super().redirect_request(req, fp, code, msg, headers, newurl)
        if following is not None:
            following._nola_update_redirects = hops
            for name in ('Authorization', 'Cookie', 'Proxy-Authorization'):
                following.remove_header(name)
        return following


def open_https(url):
    parsed = urlsplit(url)
    if parsed.scheme != 'https' or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise ValueError('An HTTPS URL without credentials or fragments is required')
    request = urllib.request.Request(url, headers={'User-Agent': 'Nola-Software-Update/1'})
    response = urllib.request.build_opener(UpdateRedirect).open(request, timeout=60)
    if response.status != 200:
        response.close(); raise ValueError('Publication is not available over HTTPS')
    return response


def verify_rust(root, candidate_path, feed_path, target):
    subprocess.run(['cargo', 'run', '--locked', '-p', 'nola_update_host', '--example', 'verify_feed', '--',
                    str(candidate_path), str(feed_path), target], cwd=root, check=True)


def activate(root, candidate, target, dry_run=False, opener=open_https, verifier=verify_rust):
    destination = root / 'release/update-source.json'
    original = destination.read_bytes()
    current = json.loads(original)
    for field in ('version', 'build'):
        if candidate[field] != current[field]:
            raise ValueError('The candidate source cannot change the installed release identity')
    if not candidate.get('feed_url') or not candidate.get('public_key'):
        raise ValueError('A real feed and public signing key are required')
    with opener(candidate['feed_url']) as response:
        feed = response.read(MAX_FEED + 1)
    if not feed or len(feed) > MAX_FEED:
        raise ValueError('Published feed size is invalid')
    with tempfile.TemporaryDirectory(prefix='nola-source-activation-') as name:
        directory = Path(name); source = directory/'source.json'; source.write_text(json.dumps(candidate))
        envelope = directory/'feed.json'; envelope.write_bytes(feed)
        # Exact current Rust protocol, key, time, identity, Store and target checks.
        verifier(root, source, envelope, target)
    manifest = json.loads(base64.b64decode(json.loads(feed)['payload_base64'], validate=True))
    selected = next(item for item in manifest['targets'] if item['target'] == target)['artifact']
    if selected['kind'] != 'store':
        expected = selected['size']
        if expected <= 0 or expected > MAX_PACKAGE:
            raise ValueError('Published package size is invalid')
        digest = hashlib.sha256(); size = 0
        with opener(selected['url']) as response:
            while chunk := response.read(65536):
                size += len(chunk)
                if size > expected:
                    raise ValueError('Published package exceeds signed size')
                digest.update(chunk)
        if size != expected or digest.hexdigest() != selected['sha256']:
            raise ValueError('Live package failed size or digest verification')
    if destination.read_bytes() != original:
        raise ValueError('Local release source changed during verification; retry')
    if not dry_run:
        # Same-directory replacement prevents a half-written client trust source.
        with tempfile.NamedTemporaryFile('w', dir=destination.parent, delete=False, prefix='.source-') as handle:
            staged = Path(handle.name)
            json.dump(candidate, handle, indent=4); handle.write('\n')
        try:
            staged.chmod(stat.S_IMODE(destination.stat().st_mode))
            staged.replace(destination)
        finally:
            staged.unlink(missing_ok=True)
    return {'verified': True, 'activated': not dry_run, 'target': target, 'feed_url': candidate['feed_url']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--target', required=True)
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    try:
        value = activate(Path(__file__).resolve().parents[2], json.loads(args.source.read_text()), args.target, args.dry_run)
        print(json.dumps(value, indent=2))
    except (ValueError, KeyError, StopIteration, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Update source was not activated: {error}\n')


if __name__ == '__main__':
    main()
