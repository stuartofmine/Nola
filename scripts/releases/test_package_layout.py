# SPDX-License-Identifier: Apache-2.0
"""The source tree can move while the installed CLI resource contract stays stable."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('package_cli', HERE / 'package_cli.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackageLayoutTests(unittest.TestCase):
    def test_moved_source_builds_the_same_self_contained_bundle_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {
                'release/update-source.json': json.dumps({'version': '1.0.0', 'build': 1}),
                'apps/windows/package.json': '{"version":"1.0.0"}',
                'apps/windows/package-lock.json': '{"version":"1.0.0","packages":{"":{"version":"1.0.0"}}}',
                'apps/android/app/build.gradle.kts': 'versionCode = 1\nversionName = "1.0.0"',
                'Nola.xcodeproj/project.pbxproj': 'CURRENT_PROJECT_VERSION = 1;\nMARKETING_VERSION = 1.0.0;',
                'apps/web/app/templates/chat.html': '<main>Fixture chat</main>',
                'apps/web/app/static/ui/index.js': '// Fixture UI',
                'LICENSE': 'Fixture project license',
                'node-license.txt': 'Fixture Node license',
                'runtime/bin/node': 'Fixture runtime',
                'runtime/bridge.cjs': '// Fixture bridge',
                'runtime/owner_queue.cjs': '// Fixture queue',
                'runtime/node_modules/playwright-core/LICENSE': 'Fixture Playwright license',
            }
            for name, value in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(value)
            for name in ['nola', 'nola-agent-host', 'nola-service']:
                path = root / 'target/release' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'fixture executable nola.tool_result.v1')
            scripts = root / 'scripts/releases'
            scripts.mkdir(parents=True)
            shutil.copy2(HERE / 'prepare_version.py', scripts / 'prepare_version.py')
            output = root / 'release.zip'
            args = ['package_cli', '--output', str(output), '--browser-runtime', str(root / 'runtime'), '--node-license', str(root / 'node-license.txt')]
            with patch.object(package, '__file__', str(scripts / 'package_cli.py')), patch.object(package.platform, 'system', return_value='Darwin'), patch.object(package.platform, 'machine', return_value='arm64'), patch.object(package.subprocess, 'check_output', return_value=b'{"packages":[]}'), patch.object(sys, 'argv', args):
                package.main()
            with zipfile.ZipFile(output) as archive:
                prefix = 'Nola-cli-macos-aarch64-1.0.0/'
                expected = ['nola', 'bin/nola', 'bin/nola-agent-host', 'bin/nola-service', 'web/app/templates/chat.html', 'web/app/static/ui/index.js', 'web/playwright_runtime/bin/node', 'web/playwright_runtime/bridge.cjs']
                self.assertTrue(all(prefix + name in archive.namelist() for name in expected))
                self.assertFalse(any('/apps/' in name for name in archive.namelist()))
                launcher = archive.read(prefix + 'nola').decode()
                self.assertIn('$NOLA_BUNDLE_ROOT/web/playwright_runtime/bin/node', launcher)
                self.assertIn('$NOLA_BUNDLE_ROOT/web}', launcher)
