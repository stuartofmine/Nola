# SPDX-License-Identifier: Apache-2.0
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
spec=importlib.util.spec_from_file_location('refresh',Path(__file__).with_name('refresh_feed.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)


class RefreshTests(unittest.TestCase):
    def test_renewal_authenticates_old_payload_and_only_changes_timestamps(self):
        openssl=os.environ.get('OPENSSL_BIN','openssl')
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);key=root/'key.pem'
            subprocess.run([openssl,'genpkey','-algorithm','ED25519','-out',str(key)],check=True,capture_output=True);key.chmod(0o600)
            source={'schema_version':1,'version':'1.0.0','build':1,'channel':'internal','public_key':module.public_key(openssl,key)}
            manifest={**{k:source[k] for k in ('schema_version','version','build','channel')},'issued_at':1,'expires_at':2,'targets':[],'notes':'unchanged'}
            payload=json.dumps(manifest).encode();data=root/'payload';data.write_bytes(payload)
            signature=subprocess.check_output([openssl,'pkeyutl','-sign','-rawin','-inkey',str(key),'-in',str(data)],stderr=subprocess.PIPE)
            feed={'payload_base64':base64.b64encode(payload).decode(),'signature_base64':base64.b64encode(signature).decode()}
            result=module.renew(source,feed,key,openssl,now=100)
            updated=json.loads(base64.b64decode(result['payload_base64']))
            self.assertEqual(updated,{**manifest,'issued_at':100,'expires_at':100+28*86400})
            feed['payload_base64']=base64.b64encode(b'{}').decode()
            with self.assertRaises(subprocess.CalledProcessError):module.renew(source,feed,key,openssl,now=100)
