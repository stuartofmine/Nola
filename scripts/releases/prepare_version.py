#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Prepare one release identity across app bundles. Does not touch installed data."""
import argparse
import json
from pathlib import Path
import re


def prepare(root, version, build, dry_run=False):
    if not re.fullmatch(r'(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})', version):
        raise ValueError('Version must contain three numeric components')
    source_path = root / 'release/update-source.json'
    source = json.loads(source_path.read_text())
    if build > 2_100_000_000 or build <= source['build']:
        raise ValueError('A release build must increase monotonically')
    source.update(version=version, build=build)
    edits = {source_path: json.dumps(source, indent=4) + '\n'}
    for name in ('windows/package.json', 'windows/package-lock.json'):
        path = root / name; value = json.loads(path.read_text()); value['version'] = version
        if 'packages' in value:
            value['packages']['']['version'] = version
        edits[path] = json.dumps(value, indent=2) + '\n'
    path = root / 'android/app/build.gradle.kts'; text = path.read_text()
    text, count = re.subn(r'versionCode = \d+', f'versionCode = {build}', text)
    text, names = re.subn(r'versionName = "[^"]+"', f'versionName = "{version}"', text)
    if count != 1 or names != 1:
        raise ValueError('Android version declarations are ambiguous')
    edits[path] = text
    path = root / 'Nola.xcodeproj/project.pbxproj'; text = path.read_text()
    text, builds = re.subn(r'CURRENT_PROJECT_VERSION = \d+;', f'CURRENT_PROJECT_VERSION = {build};', text)
    text, names = re.subn(r'MARKETING_VERSION = [^;]+;', f'MARKETING_VERSION = {version};', text)
    if not builds or not names:
        raise ValueError('Apple version declarations missing')
    edits[path] = text
    if not dry_run:
        for path, value in edits.items():
            path.write_text(value)
    return [str(path.relative_to(root)) for path in edits]


def validate_identity(root):
    source = json.loads((root / 'release/update-source.json').read_text())
    version, build = source['version'], source['build']
    for name in ('windows/package.json', 'windows/package-lock.json'):
        value = json.loads((root / name).read_text())
        if value['version'] != version or ('packages' in value and value['packages']['']['version'] != version):
            raise ValueError(f'Windows release identity differs: {name}')
    text = (root / 'android/app/build.gradle.kts').read_text()
    if re.findall(r'versionCode = (\d+)', text) != [str(build)] or re.findall(r'versionName = "([^\"]+)"', text) != [version]:
        raise ValueError('Android release identity differs')
    text = (root / 'Nola.xcodeproj/project.pbxproj').read_text()
    versions = re.findall(r'MARKETING_VERSION = ([^;]+);', text)
    builds = re.findall(r'CURRENT_PROJECT_VERSION = (\d+);', text)
    if not versions or set(versions) != {version} or not builds or set(builds) != {str(build)}:
        raise ValueError('Apple release identity differs')
    return {'version': version, 'build': build}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version'); parser.add_argument('--build', type=int)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    try:
        root = Path(__file__).resolve().parents[2]
        if args.check:
            if args.version is not None or args.build is not None or args.dry_run:
                raise ValueError('--check cannot change a release identity')
            print(json.dumps(validate_identity(root), indent=2)); return
        if args.version is None or args.build is None:
            raise ValueError('--version and --build are required to prepare a release')
        paths = prepare(root, args.version, args.build, args.dry_run)
        print(json.dumps({'dry_run': args.dry_run, 'files': paths}, indent=2))
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f'Release preparation failed: {error}\n')


if __name__ == '__main__':
    main()
