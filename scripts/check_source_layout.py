#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate repository source roots and build references without invoking services."""
import json
from pathlib import Path
import platform
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
OLD_ROOTS = {'Nola', 'NolaMac', 'NolaCore', 'NolaMLXRuntime', 'NolaTests', 'NolaMacTests', 'NolaUITests', 'NolaMLXRuntimeTests', 'Shared', 'Models', 'android', 'web', 'windows', 'cli', 'backend'}


def main():
    errors = []
    actual = {path.name for path in ROOT.iterdir()}
    for old in sorted(actual & OLD_ROOTS):
        errors.append(f'Obsolete source root remains: {old}')
    for folder in ['apps/ios', 'apps/macos', 'apps/android', 'apps/web', 'apps/windows', 'apps/cli', 'shared/apple', 'shared/apple-core', 'shared/design-tokens', 'shared/mlx-runtime', 'tests/apple/ios', 'tests/apple/macos', 'tests/apple/ios-ui', 'tests/apple/mlx']:
        if not (ROOT / folder).is_dir():
            errors.append(f'Missing source root: {folder}')
    for fixture in (ROOT / 'tests/apple/macos/SourceFixtures').iterdir():
        if fixture.is_symlink():
            if not fixture.is_file() or not fixture.resolve().is_relative_to(ROOT):
                errors.append(f'Invalid source fixture symlink: {fixture.relative_to(ROOT)}')
    manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
    cargo_paths = 0
    for member in manifest['workspace']['members']:
        path = ROOT / member / 'Cargo.toml'
        if not path.is_file():
            errors.append(f'Missing Cargo member: {member}')
            continue
        def dependencies(value):
            nonlocal cargo_paths
            if isinstance(value, dict):
                if isinstance(value.get('path'), str):
                    target = (path.parent / value['path']).resolve()
                    cargo_paths += 1
                    if not (target / 'Cargo.toml').is_file():
                        errors.append(f'{path.relative_to(ROOT)}: missing Cargo dependency {value["path"]}')
                for child in value.values():
                    dependencies(child)
        # Dependency tables only: [lib]/[[bin]] path values are source files.
        data = tomllib.loads(path.read_text())
        for key in ['dependencies', 'dev-dependencies', 'build-dependencies', 'target']:
            dependencies(data.get(key, {}))
    assets = 0
    for source in (ROOT / 'apps/web/frontend/src').rglob('*.svelte'):
        for value in re.findall(r'\bfrom\s+[\'"](\.[^\'"\n]+)[\'"]', source.read_text()):
            target = (source.parent / value.split('?', 1)[0]).resolve()
            candidates = [target, *(Path(str(target) + suffix) for suffix in ['.ts', '.js', '.svelte'])]
            if target.suffix == '.js':
                candidates.append(target.with_suffix('.ts'))
            if not any(candidate.is_file() for candidate in candidates):
                errors.append(f'{source.relative_to(ROOT)}: missing relative import {value}')
            assets += 1
    projects = 0
    if platform.system() == 'Darwin':
        for project, base in [('apple/Nola.xcodeproj/project.pbxproj', ROOT), ('shared/apple-core/NolaCore.xcodeproj/project.pbxproj', ROOT / 'shared/apple-core')]:
            data = json.loads(subprocess.check_output(['plutil', '-convert', 'json', '-o', '-', str(ROOT / project)]))
            for item in data['objects'].values():
                if item.get('isa') == 'PBXFileSystemSynchronizedRootGroup' or (item.get('isa') == 'PBXFileReference' and item.get('sourceTree') == 'SOURCE_ROOT'):
                    value = item.get('path', '')
                    if value and '$' not in value and not (base / value).exists():
                        errors.append(f'{project}: missing source reference {value}')
                for key in ['INFOPLIST_FILE', 'CODE_SIGN_ENTITLEMENTS']:
                    value = item.get('buildSettings', {}).get(key)
                    if value and '$' not in value and not (base / value).is_file():
                        errors.append(f'{project}: missing {key}: {value}')
            projects += 1
    if errors:
        raise SystemExit('\n'.join(errors))
    subprocess.run([sys.executable, str(ROOT / 'scripts/check_portable_paths.py')], check=True)
    print(f'Passed: source roots, {len(manifest["workspace"]["members"])} Cargo members, {cargo_paths} dependency paths, {assets} Svelte imports, {projects} Xcode projects')


if __name__ == '__main__':
    main()
