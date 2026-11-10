# SPDX-License-Identifier: Apache-2.0
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('prepare_version', HERE/'prepare_version.py')
prepare = importlib.util.module_from_spec(spec); spec.loader.exec_module(prepare)


class VersionPreparationTests(unittest.TestCase):
    def test_dry_run_and_one_identity_update_only_distribution_files(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            for folder in ('release', 'apps/windows', 'apps/android/app', 'Nola.xcodeproj'):
                (root/folder).mkdir(parents=True, exist_ok=True)
            (root/'release/update-source.json').write_text(json.dumps({'version':'1.0.0','build':1}))
            (root/'apps/windows/package.json').write_text('{"version":"1.0.0"}')
            (root/'apps/windows/package-lock.json').write_text('{"version":"1.0.0","packages":{"":{"version":"1.0.0"}}}')
            (root/'apps/android/app/build.gradle.kts').write_text('versionCode = 1\nversionName = "1.0.0"')
            (root/'Nola.xcodeproj/project.pbxproj').write_text('CURRENT_PROJECT_VERSION = 1;\nMARKETING_VERSION = 1.0;')
            (root/'untouched.db').write_bytes(b'user-data')
            before = (root/'release/update-source.json').read_bytes()
            prepare.prepare(root, '1.1.0', 2, True)
            self.assertEqual((root/'release/update-source.json').read_bytes(), before)
            prepare.prepare(root, '1.1.0', 2)
            self.assertEqual(prepare.validate_identity(root), {'version':'1.1.0','build':2})
            self.assertEqual(json.loads((root/'apps/windows/package-lock.json').read_text())['packages']['']['version'],'1.1.0')
            self.assertIn('versionCode = 2',(root/'apps/android/app/build.gradle.kts').read_text())
            self.assertIn('MARKETING_VERSION = 1.1.0;',(root/'Nola.xcodeproj/project.pbxproj').read_text())
            self.assertEqual((root/'untouched.db').read_bytes(),b'user-data')
            with self.assertRaises(ValueError): prepare.prepare(root,'1.2.0',2)
            with self.assertRaises(ValueError): prepare.prepare(root,'01.2.0',3)


class SigningTests(unittest.TestCase):
    def test_offline_signing_pins_key_and_hashes_actual_package(self):
        openssl = os.environ.get('OPENSSL_BIN','openssl')
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);key=root/'key.pem'
            subprocess.run([openssl,'genpkey','-algorithm','ED25519','-out',str(key)],check=True,capture_output=True)
            key.chmod(0o600)
            base=['python3',str(HERE/'sign_update_feed.py')]
            public=subprocess.check_output(base+['public-key','--openssl',openssl,'--private-key',str(key)],text=True).strip()
            self.assertEqual(len(base64.b64decode(public)),32)
            source={'schema_version':1,'version':'1.0.1','build':2,'channel':'internal','feed_url':'https://updates.example.com/feed.json','public_key':public}
            (root/'source.json').write_text(json.dumps(source));(root/'Nola.zip').write_bytes(b'package')
            manifest={**{key:source[key] for key in ['schema_version','version','build','channel']},'issued_at':1,'expires_at':2,'store_schema':1,'store_epoch':1,'notes':'fixture','targets':[{'target':'cli-macos-aarch64','artifact':{'kind':'archive','url':'https://updates.example.com/Nola.zip','filename':'Nola.zip'}}]}
            (root/'manifest.json').write_text(json.dumps(manifest))
            command=base+['sign','--openssl',openssl,'--private-key',str(key),'--source',str(root/'source.json'),'--manifest',str(root/'manifest.json'),'--artifact-dir',str(root),'--output',str(root/'feed.json')]
            subprocess.run(command,check=True,capture_output=True)
            envelope=json.loads((root/'feed.json').read_text());payload=base64.b64decode(envelope['payload_base64'])
            self.assertNotIn(key.read_text(),(root/'feed.json').read_text())
            self.assertEqual(json.loads(payload)['targets'][0]['artifact']['size'],7)
            (root/'payload').write_bytes(payload);(root/'signature').write_bytes(base64.b64decode(envelope['signature_base64']))
            with (root/'public.pem').open('wb') as handle:
                subprocess.run([openssl,'pkey','-in',str(key),'-pubout'],stdout=handle,check=True,stderr=subprocess.PIPE)
            subprocess.run([openssl,'pkeyutl','-verify','-rawin','-pubin','-inkey',str(root/'public.pem'),'-in',str(root/'payload'),'-sigfile',str(root/'signature')],check=True,capture_output=True)
            # A release cannot accidentally replace a published envelope in place.
            self.assertNotEqual(subprocess.run(command,capture_output=True).returncode,0)
            source['public_key']='A'*44;(root/'source.json').write_text(json.dumps(source))
            self.assertNotEqual(subprocess.run(command,capture_output=True).returncode,0)


if __name__ == '__main__': unittest.main()
