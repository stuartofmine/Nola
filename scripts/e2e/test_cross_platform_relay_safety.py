# SPDX-License-Identifier: Apache-2.0
"""Local transport safety only; these are not real-model acceptance results."""
import json
from pathlib import Path
import tempfile
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from test_real_cross_platform_long_tasks import relay


class CrossPlatformRelaySafetyTests(unittest.TestCase):
    def test_unselected_requests_never_reach_provider_or_become_forwarded_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            secret = 'test-only-secret-must-stay-in-memory'
            with relay('https://provider.invalid/v1', 'selected-model', secret, root) as endpoint:
                for path, payload in [
                    ('/chat/completions', {'model': 'selected-model', 'input': 'hello'}),
                    ('/responses', {'model': 'other-model', 'input': 'hello'}),
                    ('/responses', {'model': 'selected-model', 'input': []}),
                ]:
                    request = Request(endpoint + path, data=json.dumps(payload).encode(),
                        headers={'Content-Type': 'application/json'})
                    with self.assertRaises(HTTPError) as error:
                        urlopen(request, timeout=5)
                    self.assertEqual(error.exception.code, 403)
                origin = endpoint.removesuffix('/v1')
                with urlopen(origin + '/__nola_test/forwarded', timeout=5) as response:
                    self.assertEqual(json.load(response), [])
            records = [json.loads(line) for line in (root/'provider.ndjson').read_text().splitlines()]
            self.assertEqual(len(records), 3)
            self.assertTrue(all(record['forwarded'] is False for record in records))
            self.assertNotIn(secret, (root/'provider.ndjson').read_text())
