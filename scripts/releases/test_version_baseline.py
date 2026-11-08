# SPDX-License-Identifier: Apache-2.0
"""The reissued first release must establish one consistent Nola-owned baseline."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class FirstReleaseBaselineTests(unittest.TestCase):
    def test_numeric_rust_contracts_start_at_one(self):
        definitions = []
        pattern = re.compile(r'\bconst\s+([A-Z][A-Z0-9_]*(?:VERSION|EPOCH))\s*:\s*[ui]\d+\s*=\s*(\d+)\s*;')
        for directory in [ROOT / 'core', ROOT / 'cli/src']:
            for path in directory.rglob('*.rs'):
                if any(part in {'target', 'vendor'} for part in path.parts):
                    continue
                for symbol, value in pattern.findall(path.read_text()):
                    definitions.append((str(path.relative_to(ROOT)), symbol, value))
        self.assertGreater(len(definitions), 20, 'Contract discovery must not silently become empty')
        self.assertEqual([(p, s, v) for p, s, v in definitions if v != '1'], [])

    def test_platform_service_and_abi_contracts_match_core(self):
        fixtures = {
            'NolaMac/Services/MacApplicationServiceClient.swift': r'static let protocolVersion = (\d+)',
            'Nola/Services/IOSApplicationRuntime.swift': r'expectedServiceProtocol: UInt32 = (\d+)',
            'Nola/LLM/WebAgentProvider.swift': r'hostPermissionProtocolVersion = (\d+)',
            'android/app/src/main/java/com/nola/data/remote/dto/Dtos.kt': r'HOST_PERMISSION_PROTOCOL_VERSION = (\d+)',
            'android/app/src/main/java/com/nola/core/AndroidHostInvocation.kt': r'ANDROID_HOST_INVOCATION_BINDING_VERSION = (\d+)',
        }
        for name, pattern in fixtures.items():
            with self.subTest(path=name):
                match = re.search(pattern, (ROOT / name).read_text())
                self.assertIsNotNone(match)
                self.assertEqual(match[1], '1')
        kotlin = (ROOT / 'android/app/src/main/java/com/nola/core/AgentCoreRuntime.kt').read_text()
        self.assertIn('abi == 1 && protocol == 1', kotlin)

    def test_string_tagged_contracts_and_their_platform_producers_are_v1(self):
        codec = (ROOT / 'core/nola-host-protocol/src/tool_result.rs').read_text()
        self.assertIn('#[serde(rename = "nola.tool_result.v1")]', codec)
        self.assertIn('    V1,', codec)
        self.assertNotIn('    V2,', codec)
        for name in ['Nola/Services/DeviceAgentCoordinator.swift',
                     'android/app/src/main/java/com/nola/core/DeviceAgentCoordinator.kt',
                     'core/nola-mcp-host/src/mobile.rs', 'core/nola-store/src/execution_evidence.rs']:
            text = (ROOT / name).read_text()
            self.assertIn('nola.tool_result.v1', text)
            self.assertNotIn('nola.tool_result.v2', text)
        provider = (ROOT / 'Nola/LLM/OpenAIProvider.swift').read_text()
        self.assertIn('__NOLA_CORE_CONTEXT_V1__', provider)
        self.assertNotIn('__NOLA_CORE_CONTEXT_V3__', provider)

    def test_owned_evidence_and_matrix_formats_match_the_first_baseline(self):
        import json
        self.assertIn('SCHEMA_VERSION = 1', (ROOT / 'scripts/real_e2e_evidence.py').read_text())
        self.assertNotIn('LEGACY_SCHEMA_VERSIONS', (ROOT / 'scripts/real_e2e_evidence.py').read_text())
        self.assertEqual(json.loads((ROOT / 'scripts/real_e2e_matrix.json').read_text())['schemaVersion'], 1)
        self.assertIn('EVALUATOR_SCHEMA_VERSION = 1', (ROOT / 'evals/benchmarks/longmemeval/run.py').read_text())

    def test_store_initialization_and_release_example_match(self):
        import json
        self.assertIn("VALUES ('schema_epoch','1')", (ROOT / 'core/nola-store/src/schema.sql').read_text())
        manifest = json.loads((ROOT / 'release/manifest.example.json').read_text())
        self.assertEqual((manifest['store_schema'], manifest['store_epoch']), (1, 1))

    def test_readmes_describe_current_local_architecture(self):
        for name in ['README.md', 'README.zh-CN.md']:
            text = (ROOT / name).read_text()
            for retired in ['wrangler', 'nola_backend_env', 'BACKEND_PORT', 'Jinja', '| Backend', '├── backend/']:
                self.assertNotIn(retired, text)
            self.assertIn('VERSION_COMPATIBILITY.md', text)
            self.assertIn('windows/', text)


if __name__ == '__main__':
    unittest.main()
