# SPDX-License-Identifier: Apache-2.0
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('activate_source',Path(__file__).with_name('activate_source.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)


class ActivationTests(unittest.TestCase):
    def test_failed_signature_or_live_package_preserves_disabled_source(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);(root/'release').mkdir()
            current={'schema_version':1,'version':'1.0.0','build':1,'channel':'internal','feed_url':None,'public_key':None}
            path=root/'release/update-source.json';path.write_text(json.dumps(current));before=path.read_bytes()
            candidate={**current,'feed_url':'https://github.com/stuartofmine/Nola/releases/latest/download/feed.json','public_key':'fixture-public-key'}
            payload={'targets':[{'target':'cli-macos-aarch64','artifact':{'kind':'archive','url':'https://github.com/stuartofmine/Nola/releases/download/v1.0.0/Nola.zip','size':7,'sha256':hashlib.sha256(b'package').hexdigest()}}]}
            feed=json.dumps({'payload_base64':base64.b64encode(json.dumps(payload).encode()).decode()}).encode()
            def opener(url):return io.BytesIO(feed if url==candidate['feed_url'] else b'corrupt')
            def reject(*args):raise ValueError('Signature failed')
            with self.assertRaises(ValueError):module.activate(root,candidate,'cli-macos-aarch64',opener=opener,verifier=reject)
            self.assertEqual(path.read_bytes(),before)
            with self.assertRaises(ValueError):module.activate(root,candidate,'cli-macos-aarch64',opener=opener,verifier=lambda *a:None)
            self.assertEqual(path.read_bytes(),before)
            self.assertEqual(list((root/'release').iterdir()),[path])

    def test_verified_live_package_can_be_checked_then_activated(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);(root/'release').mkdir()
            current={'schema_version':1,'version':'1.0.0','build':1,'channel':'internal','feed_url':None,'public_key':None}
            path=root/'release/update-source.json';path.write_text(json.dumps(current))
            candidate={**current,'feed_url':'https://github.com/stuartofmine/Nola/releases/latest/download/feed.json','public_key':'fixture-public-key'}
            payload={'targets':[{'target':'cli-macos-aarch64','artifact':{'kind':'archive','url':'https://github.com/stuartofmine/Nola/releases/download/v1.0.0/Nola.zip','size':7,'sha256':hashlib.sha256(b'package').hexdigest()}}]}
            feed=json.dumps({'payload_base64':base64.b64encode(json.dumps(payload).encode()).decode()}).encode()
            def opener(url):return io.BytesIO(feed if url==candidate['feed_url'] else b'package')
            value=module.activate(root,candidate,'cli-macos-aarch64',dry_run=True,opener=opener,verifier=lambda *a:None)
            self.assertFalse(value['activated']);self.assertEqual(json.loads(path.read_text()),current)
            value=module.activate(root,candidate,'cli-macos-aarch64',opener=opener,verifier=lambda *a:None)
            self.assertTrue(value['activated']);self.assertEqual(json.loads(path.read_text()),candidate)

    def test_concurrent_source_edit_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);(root/'release').mkdir()
            current={'schema_version':1,'version':'1.0.0','build':1,'channel':'internal','feed_url':None,'public_key':None}
            path=root/'release/update-source.json';path.write_text(json.dumps(current))
            candidate={**current,'feed_url':'https://github.com/stuartofmine/Nola/releases/latest/download/feed.json','public_key':'fixture-public-key'}
            payload={'targets':[{'target':'cli-macos-aarch64','artifact':{'kind':'archive','url':'https://github.com/stuartofmine/Nola/releases/download/v1.0.0/Nola.zip','size':7,'sha256':hashlib.sha256(b'package').hexdigest()}}]}
            feed=json.dumps({'payload_base64':base64.b64encode(json.dumps(payload).encode()).decode()}).encode()
            edited={**current,'channel':'preview'}
            def change(*args):path.write_text(json.dumps(edited))
            def opener(url):return io.BytesIO(feed if url==candidate['feed_url'] else b'package')
            with self.assertRaisesRegex(ValueError,'changed during verification'):
                module.activate(root,candidate,'cli-macos-aarch64',opener=opener,verifier=change)
            self.assertEqual(json.loads(path.read_text()),edited)

    def test_redirect_policy_matches_client_github_boundary(self):
        self.assertTrue(module.redirect_allowed('https://github.com/a','https://release-assets.githubusercontent.com/asset'))
        for url in ['http://github.com/a','https://evil.example/a','https://user:pass@release-assets.githubusercontent.com/a','https://release-assets.githubusercontent.com:444/a']:
            self.assertFalse(module.redirect_allowed('https://github.com/a',url))
