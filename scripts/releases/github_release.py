#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Publisher-only GitHub API. Client downloads remain anonymous."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import urllib.error
import urllib.parse
import urllib.request

REPOSITORY = 'stuartofmine/Nola'


class NoAPIRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args):
        raise ValueError('Authenticated GitHub API requests must not redirect')


class GitHub:
    def __init__(self):
        token=os.environ.get('GITHUB_TOKEN')
        if not token:
            process=subprocess.run(['git','credential','fill'],input='protocol=https\nhost=github.com\n\n',text=True,capture_output=True,
                env={**os.environ,'GIT_TERMINAL_PROMPT':'0'},check=True,timeout=15)
            fields=dict(line.split('=',1) for line in process.stdout.splitlines() if '=' in line)
            token=fields.get('password')
        if not token:raise ValueError('GitHub publisher authentication is unavailable')
        self._token=token
        repo=self.api('GET',f'/repos/{REPOSITORY}')
        if repo['full_name']!=REPOSITORY or repo['private']:
            raise ValueError('The approved public Nola repository was not found')

    def api(self,method,path,value=None,file=None):
        host='uploads.github.com' if file else 'api.github.com'
        if not path.startswith(f'/repos/{REPOSITORY}/') and path != f'/repos/{REPOSITORY}':
            raise ValueError('Publisher request is outside the approved repository')
        headers={'Authorization':'Bearer '+self._token,'User-Agent':'Nola-Release-Publisher',
            'Accept':'application/vnd.github+json','X-GitHub-Api-Version':'2022-11-28'}
        payload=None
        if file:
            headers.update({'Content-Type':'application/octet-stream','Content-Length':str(file.stat().st_size)})
            payload=file.open('rb')
        elif value is not None:
            headers['Content-Type']='application/json';payload=json.dumps(value).encode()
        request=urllib.request.Request('https://'+host+path,data=payload,method=method,headers=headers)
        try:
            with urllib.request.build_opener(NoAPIRedirect).open(request,timeout=600) as response:
                body=response.read();return json.loads(body) if body else None
        except urllib.error.HTTPError as error:
            raise RuntimeError(f'GitHub {method} failed with HTTP {error.code} at {path.split("?")[0]}') from None
        finally:
            if file and payload:payload.close()

    def upload(self,release_id,path):
        return self.api('POST',f'/repos/{REPOSITORY}/releases/{release_id}/assets?name='+urllib.parse.quote(path.name),file=path)

    def signing_secret(self,path):
        # PyNaCl is a publisher dependency only, never bundled in the app.
        from nacl.public import PublicKey,SealedBox
        from nacl.encoding import Base64Encoder
        import base64
        key=self.api('GET',f'/repos/{REPOSITORY}/actions/secrets/public-key')
        encrypted=SealedBox(PublicKey(key['key'].encode(),Base64Encoder)).encrypt(path.read_bytes())
        self.api('PUT',f'/repos/{REPOSITORY}/actions/secrets/NOLA_UPDATE_SIGNING_KEY',
            {'key_id':key['key_id'],'encrypted_value':base64.b64encode(encrypted).decode()})


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='command',required=True)
    secret=sub.add_parser('store-signing-key');secret.add_argument('--private-key',required=True,type=Path)
    args=parser.parse_args()
    try:
        client=GitHub()
        if args.command=='store-signing-key':client.signing_secret(args.private_key)
        print('GitHub publisher configuration completed; no private key was printed.')
    except (ValueError,OSError,RuntimeError,subprocess.CalledProcessError) as error:
        parser.exit(1,str(error)+'\n')


if __name__=='__main__':main()
