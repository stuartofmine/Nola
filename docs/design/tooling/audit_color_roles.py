#!/usr/bin/env python3
"""Read production color declarations and report cross-platform text contrast.

This checks authored sRGB colors, not rendered system materials or accessibility
settings. Run again after platform edits; snapshots are not completion evidence.
"""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ASSETS = {'canvas': 'NolaCanvas', 'surface': 'NolaSurface', 'panel': 'NolaSurfaceRaised',
          'content': 'NolaTextPrimary', 'muted': 'NolaTextSecondary', 'outline': 'NolaControlOutline'}
ANDROID = {'canvas': 'background', 'surface': 'surface', 'panel': 'surfaceVariant',
           'content': 'onSurface', 'muted': 'onSurfaceVariant', 'outline': 'outline'}


def asset_colors(platform):
    result = {'light': {}, 'dark': {}}
    for role, name in ASSETS.items():
        data = json.loads((ROOT / platform / 'Assets.xcassets' / f'{name}.colorset/Contents.json').read_text())
        for entry in data['colors']:
            appearances = {a['appearance']: a['value'] for a in entry.get('appearances', [])}
            if 'contrast' in appearances:
                continue
            mode = 'dark' if appearances.get('luminosity') == 'dark' else 'light'
            channels = entry['color']['components']
            def channel(value):
                return int(value, 16) if value.startswith('0x') else round(float(value) * 255)
            result[mode][role] = '#' + ''.join(f'{channel(channels[c]):02x}' for c in ['red', 'green', 'blue'])
    return result


def web_colors():
    css = (ROOT / 'apps/web/frontend/src/theme.css').read_text()
    light = css.split('@media', 1)[0]
    dark = re.search(r'@media\s*\(prefers-color-scheme:\s*dark\)\s*\{\s*:root(?::not\([^)]*\))?\s*\{([^}]+)', css).group(1)
    return {mode: {role: re.search(r'--' + role + r':\s*(#[0-9a-fA-F]{6})', block).group(1).lower()
                   for role in ASSETS} for mode, block in [('light', light), ('dark', dark)]}


def android_colors():
    text = (ROOT / 'apps/android/app/src/main/java/com/nola/ui/theme/Theme.kt').read_text()
    result = {}
    for mode, name in [('light', 'LightColors'), ('dark', 'DarkColors')]:
        block = re.search(r'val ' + name + r'\s*=\s*\w+\((.*?)\n\)', text, re.S).group(1)
        result[mode] = {role: '#' + re.search(r'\b' + key + r'\s*=\s*Color\(0xFF([0-9A-Fa-f]{6})\)', block).group(1).lower()
                        for role, key in ANDROID.items()}
    return result


def luminance(color):
    channels = [int(color[i:i+2], 16)/255 for i in [1, 3, 5]]
    channels = [x/12.92 if x <= .04045 else ((x+.055)/1.055)**2.4 for x in channels]
    return sum(a*b for a, b in zip(channels, [.2126, .7152, .0722]))


def audit():
    colors = {'mac': asset_colors('apps/macos'), 'ios': asset_colors('apps/ios'), 'android': android_colors(), 'web': web_colors()}
    checks = []
    for platform, modes in colors.items():
        for mode, palette in modes.items():
            for foreground, minimum in [('content', 4.5), ('muted', 4.5), ('outline', 3)]:
                for background in ['canvas', 'surface', 'panel']:
                    a, b = sorted([luminance(palette[foreground]), luminance(palette[background])])
                    ratio = (b+.05)/(a+.05)
                    checks.append(dict(platform=platform, mode=mode, foreground=foreground, background=background,
                                       ratio=round(ratio, 3), minimum=minimum, passed=ratio >= minimum))
    differences = [dict(platform=p, mode=m, role=r, actual=v, reference=colors['web'][m][r])
                   for p, modes in colors.items() if p != 'web' for m, palette in modes.items()
                   for r, v in palette.items() if v != colors['web'][m][r]]
    return dict(colors=colors, checks=checks, failures=[c for c in checks if not c['passed']], differences_from_web=differences,
                limitation='Source sRGB values only; system chrome, rendered contrast, focus and dynamic type require separate UI evidence.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    report = audit()
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'checks': len(report['checks']), 'failures': report['failures'],
                      'differences_from_web': report['differences_from_web']}, ensure_ascii=False, indent=2))
    raise SystemExit(bool(report['failures']))
