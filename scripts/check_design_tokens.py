#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify shared design tokens, WCAG AA contrast, and cross-platform surface alignment."""

from __future__ import annotations

import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
TOKENS_PATH = ROOT / "shared/design-tokens/tokens.json"

REQUIRED_COLOR_ROLES = (
    "canvas",
    "surface",
    "surfaceSubtle",
    "surfaceElevated",
    "ink",
    "inkSecondary",
    "inkMuted",
    "border",
    "outline",
    "accent",
    "onAccent",
    "accentSubtle",
    "success",
    "warning",
    "danger",
    "info",
)

FORBIDDEN_COLD_OR_OLED_SURFACES = {
    "#000000",
    "#080808",
    "#0f1115",
    "#111111",
    "#141517",
    "#161920",
}

HEX_RE = re.compile(r"^#[0-9a-fA-F]{6}$")


def normalize_hex(value: str) -> str:
    raw = value.strip().lower()
    if raw.startswith("0xff") and len(raw) == 10:
        return f"#{raw[4:]}"
    if raw.startswith("0x") and len(raw) == 8:
        return f"#{raw[2:]}"
    if raw.startswith("#") and len(raw) == 4:
        return "#" + "".join(ch * 2 for ch in raw[1:])
    return raw


def relative_luminance(hex_color: str) -> float:
    normalized = normalize_hex(hex_color)
    channels = [int(normalized[index:index + 2], 16) / 255.0 for index in (1, 3, 5)]
    linear = [
        channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4
        for channel in channels
    ]
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]


def contrast_ratio(first: str, second: str) -> float:
    lum_a = relative_luminance(first)
    lum_b = relative_luminance(second)
    return (max(lum_a, lum_b) + 0.05) / (min(lum_a, lum_b) + 0.05)


def extract_css_vars(block: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for name, value in re.findall(r"--([\w-]+)\s*:\s*(#[0-9a-fA-F]{3,6})\b", block):
        result[name] = normalize_hex(value)
    return result


def check_tokens_schema_and_contrast(tokens: dict, errors: list[str]) -> int:
    checks = 0
    if tokens.get("schemaVersion") != 1:
        errors.append(f"tokens.json: expected schemaVersion 1, got {tokens.get('schemaVersion')!r}")
    for section in ("color", "typography", "radius", "spacing", "motion"):
        if section not in tokens or not isinstance(tokens[section], dict):
            errors.append(f"tokens.json: missing top-level object {section!r}")

    colors = tokens.get("color", {})
    for appearance in ("light", "dark"):
        palette = colors.get(appearance)
        if not isinstance(palette, dict):
            errors.append(f"tokens.json: missing color.{appearance} palette")
            continue
        for role in REQUIRED_COLOR_ROLES:
            raw = palette.get(role)
            if not isinstance(raw, str) or not HEX_RE.match(raw):
                errors.append(f"tokens.json: color.{appearance}.{role} must be #RRGGBB, got {raw!r}")
                continue
            normalized = normalize_hex(raw)
            if appearance == "dark" and normalized in FORBIDDEN_COLD_OR_OLED_SURFACES:
                errors.append(
                    f"tokens.json: color.dark.{role} uses forbidden cold/OLED value {raw}"
                )

        if any(role not in palette or not HEX_RE.match(str(palette[role])) for role in REQUIRED_COLOR_ROLES):
            continue

        for text_role in ("ink", "inkSecondary", "inkMuted", "accent", "success", "warning", "danger", "info"):
            for bg_role in ("canvas", "surface", "surfaceSubtle", "surfaceElevated"):
                ratio = contrast_ratio(palette[text_role], palette[bg_role])
                checks += 1
                if ratio < 4.5:
                    errors.append(
                        f"tokens.json ({appearance}): {text_role} ({palette[text_role]}) on "
                        f"{bg_role} ({palette[bg_role]}) contrast {ratio:.2f} < 4.5:1"
                    )

        button_ratio = contrast_ratio(palette["onAccent"], palette["accent"])
        checks += 1
        if button_ratio < 4.5:
            errors.append(
                f"tokens.json ({appearance}): onAccent/accent contrast {button_ratio:.2f} < 4.5:1"
            )

        for fg_role, bg_role in (
            ("outline", "canvas"),
            ("outline", "surface"),
            ("accent", "accentSubtle"),
        ):
            ratio = contrast_ratio(palette[fg_role], palette[bg_role])
            checks += 1
            if ratio < 3.0:
                errors.append(
                    f"tokens.json ({appearance}): {fg_role}/{bg_role} contrast {ratio:.2f} < 3.0:1"
                )
    return checks


def assert_mapping(
    source_label: str,
    actual_map: dict[str, str],
    expected_map: dict[str, str],
    errors: list[str],
) -> None:
    for key, expected in expected_map.items():
        actual = actual_map.get(key)
        if actual != normalize_hex(expected):
            errors.append(
                f"{source_label}: expected {key}={normalize_hex(expected)}, got {actual!r}"
            )


def check_prototype_css(light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    style_path = ROOT / "docs/design/prototype/style.css"
    style_lines = style_path.read_text(encoding="utf-8").splitlines()
    root_line = next((line for line in style_lines if line.startswith(":root{")), "")
    dark_line = next((line for line in style_lines if line.startswith(":root[data-theme=dark]{")), "")
    assert_mapping(
        "docs/design/prototype/style.css (:root)",
        extract_css_vars(root_line),
        {
            "bg": light["canvas"],
            "surface": light["surface"],
            "soft": light["surfaceSubtle"],
            "fg": light["ink"],
            "muted": light["inkSecondary"],
            "line": light["border"],
            "edge": light["outline"],
            "button": light["accent"],
            "inverse": light["onAccent"],
            "danger": light["danger"],
            "success": light["success"],
            "warning": light["warning"],
            "info": light["info"],
        },
        errors,
    )
    assert_mapping(
        "docs/design/prototype/style.css (dark)",
        extract_css_vars(dark_line),
        {
            "bg": dark["canvas"],
            "surface": dark["surface"],
            "soft": dark["surfaceSubtle"],
            "fg": dark["ink"],
            "muted": dark["inkSecondary"],
            "line": dark["border"],
            "edge": dark["outline"],
            "button": dark["accent"],
            "inverse": dark["onAccent"],
            "danger": dark["danger"],
            "success": dark["success"],
            "warning": dark["warning"],
            "info": dark["info"],
        },
        errors,
    )

    material_text = (ROOT / "docs/design/prototype/surface-material.css").read_text(encoding="utf-8")
    if "#111" in material_text or "#0f1115" in material_text.lower() or "#141517" in material_text.lower():
        errors.append("docs/design/prototype/surface-material.css: contains cold or pure-black dark surface")
    if normalize_hex(dark["surface"]) not in material_text.lower():
        errors.append(
            f"docs/design/prototype/surface-material.css: missing canonical dark surface {dark['surface']}"
        )

    mobile_lines = (ROOT / "docs/design/prototype/mobile-im.css").read_text(encoding="utf-8").splitlines()
    mobile_light = next((line for line in mobile_lines if 'data-mobile-proposal="im24"] {' in line), "")
    mobile_dark = next((line for line in mobile_lines if 'data-theme="dark"] {' in line), "")
    assert_mapping(
        "docs/design/prototype/mobile-im.css (light)",
        extract_css_vars(mobile_light),
        {
            "im-bg": light["canvas"],
            "im-surface": light["surface"],
            "im-active": light["surfaceSubtle"],
            "im-text": light["ink"],
            "im-muted": light["inkSecondary"],
            "im-line": light["border"],
            "im-error": light["danger"],
        },
        errors,
    )
    assert_mapping(
        "docs/design/prototype/mobile-im.css (dark)",
        extract_css_vars(mobile_dark),
        {
            "im-bg": dark["canvas"],
            "im-surface": dark["surface"],
            "im-active": dark["surfaceSubtle"],
            "im-text": dark["ink"],
            "im-muted": dark["inkSecondary"],
            "im-line": dark["border"],
            "im-error": dark["danger"],
        },
        errors,
    )


def check_web_theme_css(light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    css = (ROOT / "apps/web/frontend/src/theme.css").read_text(encoding="utf-8")
    light_block = css.split("@media", 1)[0]
    media_dark_block = css.split("@media (prefers-color-scheme: dark)", 1)[1].split("@media", 1)[0]
    explicit_dark_block = css.split(':root[data-appearance="dark"]', 1)[1].split("}", 1)[0]

    assert_mapping(
        "apps/web/frontend/src/theme.css (:root)",
        extract_css_vars(light_block),
        {
            "canvas": light["canvas"],
            "surface": light["surface"],
            "rail": light["surfaceSubtle"],
            "panel": light["surfaceSubtle"],
            "content": light["ink"],
            "muted": light["inkSecondary"],
            "border": light["border"],
            "outline": light["outline"],
            "primary": light["accent"],
            "on-primary": light["onAccent"],
            "accent": light["accent"],
            "on-accent": light["onAccent"],
            "accent-subtle": light["accentSubtle"],
            "danger": light["danger"],
            "success": light["success"],
            "warning": light["warning"],
            "info": light["info"],
        },
        errors,
    )
    expected_dark = {
        "canvas": dark["canvas"],
        "surface": dark["surface"],
        "rail": dark["surface"],
        "panel": dark["surfaceSubtle"],
        "content": dark["ink"],
        "muted": dark["inkSecondary"],
        "border": dark["border"],
        "outline": dark["outline"],
        "primary": dark["accent"],
        "on-primary": dark["onAccent"],
        "accent": dark["accent"],
        "on-accent": dark["onAccent"],
        "accent-subtle": dark["accentSubtle"],
        "danger": dark["danger"],
        "success": dark["success"],
        "warning": dark["warning"],
        "info": dark["info"],
    }
    assert_mapping(
        "apps/web/frontend/src/theme.css (prefers-color-scheme: dark)",
        extract_css_vars(media_dark_block),
        expected_dark,
        errors,
    )
    assert_mapping(
        "apps/web/frontend/src/theme.css (data-appearance=dark)",
        extract_css_vars(explicit_dark_block),
        expected_dark,
        errors,
    )


def check_android_theme(light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    kt = (ROOT / "apps/android/app/src/main/java/com/nola/ui/theme/Theme.kt").read_text(encoding="utf-8")
    light_match = re.search(r"internal val LightColors = lightColorScheme\((.*?)\n\)", kt, flags=re.S)
    dark_match = re.search(r"internal val DarkColors = darkColorScheme\((.*?)\n\)", kt, flags=re.S)
    if not light_match or not dark_match:
        errors.append("Theme.kt: missing LightColors or DarkColors definition")
        return

    def parse_kotlin_scheme(body: str) -> dict[str, str]:
        return {
            name: normalize_hex(code)
            for name, code in re.findall(r"(\w+)\s*=\s*Color\((0x[0-9A-Fa-f]{8})\)", body)
        }

    light_scheme = parse_kotlin_scheme(light_match.group(1))
    dark_scheme = parse_kotlin_scheme(dark_match.group(1))

    assert_mapping(
        "Theme.kt (LightColors)",
        light_scheme,
        {
            "background": light["canvas"],
            "surface": light["surface"],
            "surfaceVariant": light["surfaceSubtle"],
            "onBackground": light["ink"],
            "onSurface": light["ink"],
            "onSurfaceVariant": light["inkSecondary"],
            "outline": light["outline"],
            "outlineVariant": light["border"],
            "primary": light["accent"],
            "onPrimary": light["onAccent"],
            "error": light["danger"],
            "tertiary": light["warning"],
        },
        errors,
    )
    assert_mapping(
        "Theme.kt (DarkColors)",
        dark_scheme,
        {
            "background": dark["canvas"],
            "surface": dark["surface"],
            "surfaceVariant": dark["surfaceSubtle"],
            "onBackground": dark["ink"],
            "onSurface": dark["ink"],
            "onSurfaceVariant": dark["inkSecondary"],
            "outline": dark["outline"],
            "outlineVariant": dark["border"],
            "primary": dark["accent"],
            "onPrimary": dark["onAccent"],
            "error": dark["danger"],
            "tertiary": dark["warning"],
        },
        errors,
    )

    for label, scheme in (("LightColors", light_scheme), ("DarkColors", dark_scheme)):
        for fg_key, bg_key in (
            ("onBackground", "background"),
            ("onSurface", "surface"),
            ("onSurfaceVariant", "background"),
            ("onSurfaceVariant", "surface"),
            ("onSurfaceVariant", "surfaceVariant"),
            ("onPrimaryContainer", "primaryContainer"),
            ("onPrimary", "primary"),
            ("error", "surface"),
            ("onTertiary", "tertiary"),
        ):
            if fg_key in scheme and bg_key in scheme:
                ratio = contrast_ratio(scheme[fg_key], scheme[bg_key])
                if ratio < 4.5:
                    errors.append(
                        f"Theme.kt ({label}): {fg_key}/{bg_key} contrast {ratio:.2f} < 4.5:1"
                    )


def check_windows_shell(light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    windows_src = ROOT / "apps/windows/src"
    tokens_ts = (windows_src / "themeTokens.ts").read_text(encoding="utf-8")
    main_ts = (windows_src / "main.ts").read_text(encoding="utf-8")
    startup_ts = (windows_src / "startupPage.ts").read_text(encoding="utf-8")

    light_match = re.search(r"LIGHT_THEME_TOKENS\s*:\s*DesktopThemeTokens\s*=\s*\{(.*?)\}", tokens_ts, flags=re.S)
    dark_match = re.search(r"DARK_THEME_TOKENS\s*:\s*DesktopThemeTokens\s*=\s*\{(.*?)\}", tokens_ts, flags=re.S)
    if not light_match or not dark_match:
        errors.append("apps/windows/src/themeTokens.ts: missing LIGHT_THEME_TOKENS or DARK_THEME_TOKENS")
        return

    def parse_ts_object(body: str) -> dict[str, str]:
        return {
            key: normalize_hex(val)
            for key, val in re.findall(r"(\w+)\s*:\s*[\"'](#[0-9a-fA-F]{6})[\"']", body)
        }

    expected_keys = (
        "canvas",
        "surface",
        "surfaceSubtle",
        "ink",
        "inkSecondary",
        "border",
        "outline",
        "accent",
        "onAccent",
    )
    assert_mapping(
        "apps/windows/src/themeTokens.ts (LIGHT_THEME_TOKENS)",
        parse_ts_object(light_match.group(1)),
        {key: light[key] for key in expected_keys},
        errors,
    )
    assert_mapping(
        "apps/windows/src/themeTokens.ts (DARK_THEME_TOKENS)",
        parse_ts_object(dark_match.group(1)),
        {key: dark[key] for key in expected_keys},
        errors,
    )

    if "DARK_THEME_TOKENS.canvas" not in main_ts or "LIGHT_THEME_TOKENS.canvas" not in main_ts:
        errors.append(
            "apps/windows/src/main.ts: BrowserWindow backgroundColor must use DARK_THEME_TOKENS.canvas / LIGHT_THEME_TOKENS.canvas"
        )
    if "DARK_THEME_TOKENS.canvas" not in startup_ts or "LIGHT_THEME_TOKENS.canvas" not in startup_ts:
        errors.append(
            "apps/windows/src/startupPage.ts: startup HTML must use LIGHT_THEME_TOKENS / DARK_THEME_TOKENS"
        )

    for ts_file in sorted(windows_src.glob("*.ts")):
        content = ts_file.read_text(encoding="utf-8")
        for match in re.findall(r"#[0-9a-fA-F]{3,6}\b", content):
            if normalize_hex(match) in FORBIDDEN_COLD_OR_OLED_SURFACES:
                errors.append(
                    f"{ts_file.relative_to(ROOT)}: contains forbidden cold/OLED surface literal {match}"
                )


def check_apple_fallbacks(light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    ios_swift = (ROOT / "apps/ios/UI/DesignSystem.swift").read_text(encoding="utf-8")
    ios_pairs = {
        name: (normalize_hex(l_hex), normalize_hex(d_hex))
        for name, l_hex, d_hex in re.findall(
            r"static let (\w+)\s*=\s*mobileColor\((0x[0-9A-Fa-f]{6}),\s*(0x[0-9A-Fa-f]{6})\)",
            ios_swift,
        )
    }
    expected_ios = {
        "backgroundColor": (light["canvas"], dark["canvas"]),
        "chatBackgroundColor": (light["canvas"], dark["canvas"]),
        "surfaceColor": (light["surface"], dark["surface"]),
        "elevatedColor": (light["surfaceSubtle"], dark["surfaceSubtle"]),
        "rowFillColor": (light["surfaceSubtle"], dark["surfaceSubtle"]),
        "primaryColor": (light["ink"], dark["ink"]),
        "secondaryContentColor": (light["inkSecondary"], dark["inkSecondary"]),
        "onActionColor": (light["onAccent"], dark["onAccent"]),
        "accentSubtleColor": (light["accentSubtle"], dark["accentSubtle"]),
        "borderColor": (light["border"], dark["border"]),
        "separatorColor": (light["border"], dark["border"]),
    }
    for prop, (exp_l, exp_d) in expected_ios.items():
        actual = ios_pairs.get(prop)
        expected_pair = (normalize_hex(exp_l), normalize_hex(exp_d))
        if actual != expected_pair:
            errors.append(
                f"apps/ios/UI/DesignSystem.swift: expected {prop}={expected_pair}, got {actual!r}"
            )

    mac_swift = (ROOT / "apps/macos/UI/MacPageChrome.swift").read_text(encoding="utf-8")
    for enum_name, expected_palette in (("Light", light), ("Dark", dark)):
        block_match = re.search(rf"enum {enum_name}\s*\{{(.*?)\}}", mac_swift, flags=re.S)
        if not block_match:
            errors.append(f"apps/macos/UI/MacPageChrome.swift: missing MacDesignTokens.Palette.{enum_name}")
            continue
        actual_mac = {
            key: normalize_hex(val)
            for key, val in re.findall(r"static let (\w+):\s*UInt32\s*=\s*(0x[0-9A-Fa-f]{6})", block_match.group(1))
        }
        assert_mapping(
            f"apps/macos/UI/MacPageChrome.swift (Palette.{enum_name})",
            actual_mac,
            {role: expected_palette[role] for role in REQUIRED_COLOR_ROLES},
            errors,
        )


def main() -> int:
    if not TOKENS_PATH.is_file():
        print(f"Missing canonical design tokens file: {TOKENS_PATH.relative_to(ROOT)}", file=sys.stderr)
        return 1

    tokens = json.loads(TOKENS_PATH.read_text(encoding="utf-8"))
    errors: list[str] = []
    contrast_checks = check_tokens_schema_and_contrast(tokens, errors)
    if not errors:
        light = tokens["color"]["light"]
        dark = tokens["color"]["dark"]
        check_prototype_css(light, dark, errors)
        check_web_theme_css(light, dark, errors)
        check_android_theme(light, dark, errors)
        check_windows_shell(light, dark, errors)
        check_apple_fallbacks(light, dark, errors)

    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print(
        f"Passed: {TOKENS_PATH.relative_to(ROOT)} schema v{tokens['schemaVersion']}, "
        f"{contrast_checks} WCAG contrast checks, and 6 platform surface alignments"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
