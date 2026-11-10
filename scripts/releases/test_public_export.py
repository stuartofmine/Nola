# SPDX-License-Identifier: Apache-2.0
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('public_export',Path(__file__).with_name('export_public.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)


class PublicExportTests(unittest.TestCase):
    def test_export_excludes_even_tracked_keys_and_all_private_git_history(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)/'private';root.mkdir();destination=Path(name)/'public'
            subprocess.run(['git','init','-q',str(root)],check=True)
            (root/'cer').mkdir();(root/'cer/key.p8').write_text('not-public')
            (root/'source.rs').write_text('// public source\n')
            subprocess.run(['git','add','.'],cwd=root,check=True)
            report=module.export(root,destination,[])
            self.assertFalse((destination/'cer').exists());self.assertFalse((destination/'.git').exists())
            self.assertEqual((destination/'source.rs').read_text(),'// public source\n');self.assertFalse(report['history_copied'])

    def test_local_editor_agent_and_private_workflow_settings_never_publish(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)/'private';root.mkdir();destination=Path(name)/'public'
            subprocess.run(['git','init','-q',str(root)],check=True)
            for directory in ['.github','.vscode','.claude','.cargo','.codex','.idea']:
                (root/directory).mkdir();(root/directory/'local-settings.txt').write_text('private workspace preferences')
            (root/'README.md').write_text('Public project')
            (root/'default.profraw').write_bytes(b'local coverage')
            (root/'Tests.xcresult').mkdir();(root/'Tests.xcresult'/'Info.plist').write_text('private test results')
            subprocess.run(['git','add','.'],cwd=root,check=True)
            module.export(root,destination,[])
            self.assertEqual((destination/'README.md').read_text(),'Public project')
            for directory in ['.github','.vscode','.claude','.cargo','.codex','.idea']:
                self.assertFalse((destination/directory).exists(),directory)
            self.assertFalse((destination/'default.profraw').exists())
            self.assertFalse((destination/'Tests.xcresult').exists())

    def test_local_tasks_and_history_never_export_even_when_tracked(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)/'private';root.mkdir();destination=Path(name)/'public'
            subprocess.run(['git','init','-q',str(root)],check=True)
            for folder in ['Doc/当前任务','Doc/历史任务']:
                (root/folder).mkdir(parents=True)
                (root/folder/'record.md').write_text('Local working record')
                (root/folder/'trace.json').write_text('{"private": true}')
            (root/'docs/platforms/android').mkdir(parents=True)
            (root/'docs/platforms/android/README.md').write_text('Public Android guide')
            subprocess.run(['git','add','.'],cwd=root,check=True)
            module.export(root,destination,[])
            self.assertFalse((destination/'Doc').exists())
            self.assertEqual((destination/'docs/platforms/android/README.md').read_text(),'Public Android guide')
            self.assertTrue((root/'Doc/当前任务/record.md').is_file())
            self.assertTrue((root/'Doc/历史任务/trace.json').is_file())

    def test_unknown_token_stops_export_before_creating_destination(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)/'private';root.mkdir();destination=Path(name)/'public'
            subprocess.run(['git','init','-q',str(root)],check=True)
            # Construct the synthetic token so this test source contains none.
            (root/'source.txt').write_text('ghp_'+'A'*36)
            with self.assertRaisesRegex(ValueError,'credential findings'):module.export(root,destination,[])
            self.assertFalse(destination.exists())
