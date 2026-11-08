#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Renew timestamps on an authenticated existing release; never change packages."""
import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from sign_update_feed import public_key


def renew(source, feed, key, openssl='openssl', now=None):
    if public_key(openssl, key) != source['public_key']:
        raise ValueError('Signing key does not match the published public key')
    payload = base64.b64decode(feed['payload_base64'], validate=True)
    signature = base64.b64decode(feed['signature_base64'], validate=True)
    if len(payload) > 1024 * 1024 or len(signature) != 64:
        raise ValueError('Invalid existing feed size')
    with tempfile.TemporaryDirectory(prefix='nola-feed-renew-') as name:
        root=Path(name);data=root/'payload';signed=root/'signature';public=root/'public.pem'
        data.write_bytes(payload);signed.write_bytes(signature)
        with public.open('wb') as handle:
            subprocess.run([openssl,'pkey','-in',str(key),'-pubout'],stdout=handle,stderr=subprocess.PIPE,check=True)
        # Renewal is a publisher action: verify the old signature even if expired.
        # The client still rejects expired feeds without any exception.
        subprocess.run([openssl,'pkeyutl','-verify','-rawin','-pubin','-inkey',str(public),'-in',str(data),'-sigfile',str(signed)],check=True,capture_output=True)
        manifest=json.loads(payload)
        for field in ('version','build','channel','schema_version'):
            if manifest[field] != source[field]:
                raise ValueError('Release identity changed; publish a new release instead')
        stamp=int(time.time()) if now is None else now
        manifest['issued_at']=stamp;manifest['expires_at']=stamp+28*86400
        updated=json.dumps(manifest,ensure_ascii=False,separators=(',',':')).encode()
        data.write_bytes(updated)
        signature=subprocess.check_output([openssl,'pkeyutl','-sign','-rawin','-inkey',str(key),'-in',str(data)],stderr=subprocess.PIPE)
    return {'payload_base64':base64.b64encode(updated).decode(),'signature_base64':base64.b64encode(signature).decode()}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--private-key',required=True,type=Path)
    parser.add_argument('--openssl',default=os.environ.get('OPENSSL_BIN','openssl'))
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[2]
    path=root/'release/feed.json'
    try:
        source=json.loads((root/'release/update-source.json').read_text())
        value=renew(source,json.loads(path.read_text()),args.private_key,args.openssl)
        # Validate all current target and Store contracts before replacing the file.
        with tempfile.TemporaryDirectory(prefix='nola-renew-validation-') as name:
            staged=Path(name)/'feed.json';staged.write_text(json.dumps(value,indent=2)+'\n')
            manifest=json.loads(base64.b64decode(value['payload_base64']))
            for item in manifest['targets']:
                subprocess.run(['cargo','run','--locked','-p','nola_update_host','--example','verify_feed','--',str(root/'release/update-source.json'),str(staged),item['target']],cwd=root,check=True)
            path.write_text(staged.read_text())
        print('Renewed signed release timestamps; package identities are unchanged.')
    except (ValueError,KeyError,OSError,subprocess.CalledProcessError) as error:
        parser.exit(1,f'Feed renewal failed: {error}\n')


if __name__=='__main__':main()
