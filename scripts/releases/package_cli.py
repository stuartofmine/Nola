#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Package the matching CLI, Agent Host, service, Web UI and browser runtime."""
import argparse
import platform
import os
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--browser-runtime',required=True,type=Path)
    parser.add_argument('--node-license',required=True,type=Path)
    args=parser.parse_args()
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':raise SystemExit('This packager requires macOS Apple Silicon')
    root=Path(__file__).resolve().parents[2]
    source=json.loads((root/'release/update-source.json').read_text());version=source['version']
    if args.output.exists():raise SystemExit('Output already exists')
    runtime=args.browser_runtime.resolve()
    for name in ['bin/node','bridge.cjs','owner_queue.cjs','node_modules/playwright-core/LICENSE']:
        if not (runtime/name).is_file():raise SystemExit('Managed browser runtime is incomplete: '+name)
    subprocess.run(['python3',str(root/'scripts/releases/prepare_version.py'),'--check'],check=True)
    name=f'Nola-cli-macos-aarch64-{version}'
    with tempfile.TemporaryDirectory(prefix='nola-cli-package-') as temporary:
        bundle=Path(temporary)/name;bundle.mkdir();(bundle/'bin').mkdir()
        for binary in ['nola','nola-agent-host','nola-service']:
            path=root/'target/release'/binary
            if not path.is_file():raise SystemExit('Release executable missing: '+binary)
            shutil.copy2(path,bundle/'bin'/binary)
        for folder in ['app/static','app/templates']:
            path=root/'web'/folder
            if path.is_dir():shutil.copytree(path,bundle/'web'/folder,ignore=shutil.ignore_patterns('__pycache__','agent_artifacts','*.db','*.log'))
        shutil.copytree(runtime,bundle/'web/playwright_runtime')
        shutil.copy2(root/'LICENSE',bundle/'LICENSE')
        (bundle/'licenses').mkdir();shutil.copy2(args.node_license,bundle/'licenses/Node-LICENSE.txt')
        (bundle/'nola').write_text('''#!/bin/sh
set -eu
NOLA_BUNDLE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export NOLA_WEB_ROOT="${NOLA_WEB_ROOT:-$NOLA_BUNDLE_ROOT/web}"
export NOLA_PLAYWRIGHT_NODE="${NOLA_PLAYWRIGHT_NODE:-$NOLA_BUNDLE_ROOT/web/playwright_runtime/bin/node}"
export NOLA_PLAYWRIGHT_BRIDGE="${NOLA_PLAYWRIGHT_BRIDGE:-$NOLA_BUNDLE_ROOT/web/playwright_runtime/bridge.cjs}"
exec "$NOLA_BUNDLE_ROOT/bin/nola" "$@"
''');(bundle/'nola').chmod(0o755)
        (bundle/'README.txt').write_text('Nola '+version+' ('+str(source['build'])+') — macOS Apple Silicon CLI preview\n\n'
            'Run ./nola --version, ./nola --help, or ./nola web. Keep the complete folder together.\n'
            'This preview is not Apple Developer ID notarized. Native installers for other platforms are not included.\n'
            'Finish active tasks before manually replacing a previous installation. User data is not migrated or reset.\n'
            'Source and updates: https://github.com/stuartofmine/Nola\n')
        metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=root))
        notices=[]
        for package in metadata['packages']:
            if not package.get('source'):continue
            label=package['name']+'-'+package['version'];notices.append(label+': '+str(package.get('license') or 'see included license'))
            directory=Path(package['manifest_path']).parent
            for item in directory.iterdir():
                if item.is_file() and item.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
                    target=bundle/'licenses/cargo'/label/item.name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(item,target)
        (bundle/'THIRD_PARTY_NOTICES.txt').write_text('\n'.join(sorted(notices))+'\n\nNode and Playwright license texts are included.\n')
        with tempfile.NamedTemporaryFile(dir=args.output.parent, prefix='.nola-package-', delete=False) as staged:
            staged_path=Path(staged.name)
        try:
            with zipfile.ZipFile(staged_path,'w',zipfile.ZIP_DEFLATED,compresslevel=6,strict_timestamps=False) as archive:
                for path in sorted(bundle.rglob('*')):
                    if path.is_file():archive.write(path,path.relative_to(bundle.parent))
            os.link(staged_path,args.output)
        finally:
            staged_path.unlink(missing_ok=True)
    print('Created complete CLI release archive:',args.output)


if __name__=='__main__':main()
