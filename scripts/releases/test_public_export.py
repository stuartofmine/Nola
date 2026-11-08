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

    def test_unknown_token_stops_export_before_creating_destination(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)/'private';root.mkdir();destination=Path(name)/'public'
            subprocess.run(['git','init','-q',str(root)],check=True)
            # Construct the synthetic token so this test source contains none.
            (root/'source.txt').write_text('ghp_'+'A'*36)
            with self.assertRaisesRegex(ValueError,'credential findings'):module.export(root,destination,[])
            self.assertFalse(destination.exists())
