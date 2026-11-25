#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify shared design tokens, WCAG AA contrast, and cross-platform surface alignment."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]

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
    "#14161b",
    "#16181d",
    "#161920",
    "#1c1f26",
    "#e7e9ee",
    "#f2f2f7",
}

EXPECTED_A2UI_MAPPINGS = {
    "a2ui-color-text": "var(--content)",
    "a2ui-color-text-muted": "var(--muted)",
    "a2ui-color-primary": "var(--primary)",
    "a2ui-color-on-primary": "var(--on-primary)",
    "a2ui-color-surface": "var(--surface)",
    "a2ui-color-surface-raised": "var(--panel)",
    "a2ui-color-border": "var(--border)",
    "a2ui-color-danger": "var(--danger)",
}

EXPERT_BUBBLE_COLORS = ("red", "orange", "yellow", "green", "teal", "blue", "purple", "pink")

HEX_RE = re.compile(r"^#[0-9a-fA-F]{6}$")
COLOR_LITERAL_RE = re.compile(r"(?:#[0-9a-fA-F]{3,8}\b|0x[0-9a-fA-F]{6,8}\b)")
MAC_FORBIDDEN_COLOR_API_RE = re.compile(
    r"\.(?:textBackgroundColor|controlBackgroundColor|windowBackgroundColor)\b"
)
IOS_FORBIDDEN_COLOR_API_RE = re.compile(
    r"(?:\bsystemGroupedBackground\b"
    r"|\bsecondarySystemGroupedBackground\b"
    r"|\bUIColor\.systemBackground\b"
    r"|Color\(\s*\.systemBackground\s*\)"
    r"|Color\(\s*\.secondarySystemBackground\s*\)"
    r"|Color\(\s*\.tertiarySystemBackground\s*\))"
)


def normalize_hex(value: str) -> str:
    raw = value.strip().lower()
    if raw.startswith("0xff") and len(raw) == 10:
        return f"#{raw[4:]}"
    if raw.startswith("0x") and len(raw) == 8:
        return f"#{raw[2:]}"
    if raw.startswith("#ff") and len(raw) == 9:
        return f"#{raw[3:]}"
    if raw.startswith("#") and len(raw) == 4:
        return "#" + "".join(ch * 2 for ch in raw[1:])
    return raw


def normalize_opaque_hex(token: str) -> str | None:
    """Normalize solid opaque hex/0x literals; ignore translucent alpha hexes like #0003 or 0x20000000."""
    raw = token.strip().lower()
    if raw.startswith("0xff") and len(raw) == 10:
        return f"#{raw[4:]}"
    if raw.startswith("0x") and len(raw) == 8:
        return f"#{raw[2:]}"
    if raw.startswith("#ff") and len(raw) == 9:
        return f"#{raw[3:]}"
    if raw.startswith("#") and len(raw) == 7:
        return raw
    if raw.startswith("#") and len(raw) == 4:
        return "#" + "".join(ch * 2 for ch in raw[1:])
    return None


def scan_forbidden_hex_literals(
    root: Path,
    files: list[Path],
    errors: list[str],
    forbidden: set[str] = FORBIDDEN_COLD_OR_OLED_SURFACES,
) -> None:
    for path in files:
        content = path.read_text(encoding="utf-8", errors="ignore")
        for line_number, line in enumerate(content.splitlines(), start=1):
            for match in COLOR_LITERAL_RE.finditer(line):
                normalized = normalize_opaque_hex(match.group(0))
                if normalized in forbidden:
                    errors.append(
                        f"{path.relative_to(root)}:{line_number}: contains forbidden cold/OLED literal {match.group(0)}"
                    )


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


def extract_css_var_refs(block: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for name, value in re.findall(r"--([\w-]+)\s*:\s*(var\(--[\w-]+\))", block):
        result[name] = value
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


def assert_var_ref_mapping(
    source_label: str,
    actual_map: dict[str, str],
    expected_map: dict[str, str],
    errors: list[str],
) -> None:
    for key, expected in expected_map.items():
        actual = actual_map.get(key)
        if actual != expected:
            errors.append(
                f"{source_label}: expected --{key}={expected}, got {actual!r}"
            )


def check_prototype_css(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    style_path = root / "docs/design/prototype/style.css"
    if not style_path.is_file():
        errors.append("docs/design/prototype/style.css: missing file")
        return
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

    material_path = root / "docs/design/prototype/surface-material.css"
    if not material_path.is_file():
        errors.append("docs/design/prototype/surface-material.css: missing file")
    else:
        material_text = material_path.read_text(encoding="utf-8")
        if "#111" in material_text or "#0f1115" in material_text.lower() or "#141517" in material_text.lower():
            errors.append("docs/design/prototype/surface-material.css: contains cold or pure-black dark surface")
        if normalize_hex(dark["surface"]) not in material_text.lower():
            errors.append(
                f"docs/design/prototype/surface-material.css: missing canonical dark surface {dark['surface']}"
            )

    mobile_path = root / "docs/design/prototype/mobile-im.css"
    if not mobile_path.is_file():
        errors.append("docs/design/prototype/mobile-im.css: missing file")
    else:
        mobile_lines = mobile_path.read_text(encoding="utf-8").splitlines()
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


def check_expert_bubble_colors(
    root: Path,
    light: dict[str, str],
    dark: dict[str, str],
    errors: list[str],
) -> None:
    colors_path = root / "apps/web/frontend/src/lib/expertBubbleColors.ts"
    if not colors_path.is_file():
        return
    source = colors_path.read_text(encoding="utf-8")
    parsed = {
        name: (normalize_hex(light_hex), normalize_hex(dark_hex))
        for name, light_hex, dark_hex in re.findall(
            r"(\w+)\s*:\s*\[\s*['\"](#[0-9a-fA-F]{6})['\"]\s*,\s*['\"](#[0-9a-fA-F]{6})['\"]\s*\]",
            source,
        )
    }
    for color_name in EXPERT_BUBBLE_COLORS:
        if color_name not in parsed:
            errors.append(f"apps/web/frontend/src/lib/expertBubbleColors.ts: missing color {color_name!r}")
            continue
        light_bg, dark_bg = parsed[color_name]
        if light_bg in FORBIDDEN_COLD_OR_OLED_SURFACES or dark_bg in FORBIDDEN_COLD_OR_OLED_SURFACES:
            errors.append(
                f"apps/web/frontend/src/lib/expertBubbleColors.ts ({color_name}): uses forbidden cold/OLED value"
            )
        for role_name, fg_hex, bg_hex, mode in (
            ("ink", light["ink"], light_bg, "light"),
            ("inkSecondary", light["inkSecondary"], light_bg, "light"),
            ("ink", dark["ink"], dark_bg, "dark"),
            ("inkSecondary", dark["inkSecondary"], dark_bg, "dark"),
        ):
            ratio = contrast_ratio(fg_hex, bg_hex)
            if ratio < 4.5:
                errors.append(
                    f"apps/web/frontend/src/lib/expertBubbleColors.ts ({color_name}, {mode}): "
                    f"{role_name} ({fg_hex}) on {bg_hex} contrast {ratio:.2f} < 4.5:1"
                )


def check_web_theme_css(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    theme_path = root / "apps/web/frontend/src/theme.css"
    if not theme_path.is_file():
        errors.append("apps/web/frontend/src/theme.css: missing file")
        return
    css = theme_path.read_text(encoding="utf-8")
    if "@media (prefers-color-scheme: dark)" not in css or ':root[data-appearance="dark"]' not in css:
        errors.append("apps/web/frontend/src/theme.css: missing dark theme blocks")
        return

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

    assert_var_ref_mapping(
        "apps/web/frontend/src/theme.css (:root a2ui)",
        extract_css_var_refs(light_block),
        EXPECTED_A2UI_MAPPINGS,
        errors,
    )
    assert_var_ref_mapping(
        "apps/web/frontend/src/theme.css (prefers-color-scheme: dark a2ui)",
        extract_css_var_refs(media_dark_block),
        EXPECTED_A2UI_MAPPINGS,
        errors,
    )
    assert_var_ref_mapping(
        "apps/web/frontend/src/theme.css (data-appearance=dark a2ui)",
        extract_css_var_refs(explicit_dark_block),
        EXPECTED_A2UI_MAPPINGS,
        errors,
    )

    diagram_path = root / "apps/web/frontend/src/lib/diagramDocument.ts"
    if diagram_path.is_file():
        diagram_source = diagram_path.read_text(encoding="utf-8").lower()
        for role_hex in (light["canvas"], light["ink"], dark["canvas"], dark["ink"]):
            if normalize_hex(role_hex) not in diagram_source:
                errors.append(
                    f"apps/web/frontend/src/lib/diagramDocument.ts: missing Warm Paper/Ink fallback {normalize_hex(role_hex)}"
                )

    web_src = root / "apps/web/frontend/src"
    if web_src.is_dir():
        web_files = [
            path
            for path in sorted(web_src.rglob("*"))
            if path.is_file()
            and path.suffix in {".ts", ".svelte", ".css"}
            and "vendor" not in path.relative_to(web_src).parts
            and ".test." not in path.name
        ]
        scan_forbidden_hex_literals(root, web_files, errors)

    check_expert_bubble_colors(root, light, dark, errors)


def check_android_theme(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    theme_path = root / "apps/android/app/src/main/java/com/nola/ui/theme/Theme.kt"
    if not theme_path.is_file():
        errors.append("Theme.kt: missing file")
        return
    kt = theme_path.read_text(encoding="utf-8")
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

    design_system_path = root / "apps/android/app/src/main/java/com/nola/ui/components/DesignSystem.kt"
    if not design_system_path.is_file():
        errors.append("DesignSystem.kt: missing file")
    else:
        ds_source = design_system_path.read_text(encoding="utf-8")
        ds_colors = {
            name: normalize_hex(code)
            for name, code in re.findall(r"val\s+(bgLight|bgDark)\s*=\s*Color\((0x[0-9A-Fa-f]{8})\)", ds_source)
        }
        assert_mapping(
            "apps/android/app/src/main/java/com/nola/ui/components/DesignSystem.kt",
            ds_colors,
            {"bgLight": light["canvas"], "bgDark": dark["canvas"]},
            errors,
        )

    android_ui_root = root / "apps/android/app/src/main/java/com/nola/ui"
    if android_ui_root.is_dir():
        scan_forbidden_hex_literals(root, sorted(android_ui_root.rglob("*.kt")), errors)

    android_res_root = root / "apps/android/app/src/main/res"
    if android_res_root.is_dir():
        res_files = [
            path
            for values_dir in sorted(android_res_root.glob("values*"))
            if values_dir.is_dir()
            for path in sorted(values_dir.glob("*.xml"))
            if path.name in {"colors.xml", "themes.xml"}
        ]
        scan_forbidden_hex_literals(root, res_files, errors)


def check_windows_shell(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    windows_src = root / "apps/windows/src"
    tokens_path = windows_src / "themeTokens.ts"
    desktop_export = root / "apps/windows/scripts/export-shell-strings.mjs"
    startup_path = windows_src / "startupPage.ts"
    if not tokens_path.is_file():
        errors.append("apps/windows/src/themeTokens.ts: missing file")
        return

    tokens_ts = tokens_path.read_text(encoding="utf-8")
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

    assert_mapping(
        "apps/windows/src/themeTokens.ts (LIGHT_THEME_TOKENS)",
        parse_ts_object(light_match.group(1)),
        {key: light[key] for key in REQUIRED_COLOR_ROLES},
        errors,
    )
    assert_mapping(
        "apps/windows/src/themeTokens.ts (DARK_THEME_TOKENS)",
        parse_ts_object(dark_match.group(1)),
        {key: dark[key] for key in REQUIRED_COLOR_ROLES},
        errors,
    )

    if not desktop_export.is_file() or "startupPageHtml(" not in desktop_export.read_text(encoding="utf-8"):
        errors.append("Tauri startup resources must render the themed Windows startup page")

    if not startup_path.is_file():
        errors.append("apps/windows/src/startupPage.ts: missing file")
    else:
        startup_ts = startup_path.read_text(encoding="utf-8")
        if "DARK_THEME_TOKENS.canvas" not in startup_ts or "LIGHT_THEME_TOKENS.canvas" not in startup_ts:
            errors.append(
                "apps/windows/src/startupPage.ts: startup HTML must use LIGHT_THEME_TOKENS / DARK_THEME_TOKENS"
            )

    scan_forbidden_hex_literals(root, sorted(windows_src.rglob("*.ts")), errors)
    tauri_src = root / "apps/windows/src-tauri/src"
    if tauri_src.is_dir():
        scan_forbidden_hex_literals(root, sorted(tauri_src.rglob("*.rs")), errors)


def check_apple_fallbacks(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    ios_ds_path = root / "apps/ios/UI/DesignSystem.swift"
    if not ios_ds_path.is_file():
        errors.append("apps/ios/UI/DesignSystem.swift: missing file")
    else:
        ios_swift = ios_ds_path.read_text(encoding="utf-8")
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
            "elevatedColor": (light["surfaceElevated"], dark["surfaceElevated"]),
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

    mac_chrome_path = root / "apps/macos/UI/MacPageChrome.swift"
    if not mac_chrome_path.is_file():
        errors.append("apps/macos/UI/MacPageChrome.swift: missing file")
    else:
        mac_swift = mac_chrome_path.read_text(encoding="utf-8")
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

    mac_root = root / "apps/macos"
    if mac_root.is_dir():
        mac_swift_files = sorted(mac_root.rglob("*.swift"))
        for path in mac_swift_files:
            content = path.read_text(encoding="utf-8", errors="ignore")
            for line_number, line in enumerate(content.splitlines(), start=1):
                match = MAC_FORBIDDEN_COLOR_API_RE.search(line)
                if match:
                    errors.append(
                        f"{path.relative_to(root)}:{line_number}: uses forbidden AppKit background color {match.group(0)}"
                    )
        scan_forbidden_hex_literals(root, mac_swift_files, errors)

    ios_root = root / "apps/ios"
    if ios_root.is_dir():
        ios_swift_files = sorted(ios_root.rglob("*.swift"))
        for path in ios_swift_files:
            content = path.read_text(encoding="utf-8", errors="ignore")
            for line_number, line in enumerate(content.splitlines(), start=1):
                match = IOS_FORBIDDEN_COLOR_API_RE.search(line)
                if match:
                    errors.append(
                        f"{path.relative_to(root)}:{line_number}: uses forbidden UIKit system background {match.group(0)}"
                    )
        scan_forbidden_hex_literals(root, ios_swift_files, errors)

    shared_apple_root = root / "shared/apple"
    if shared_apple_root.is_dir():
        scan_forbidden_hex_literals(root, sorted(shared_apple_root.rglob("*.swift")), errors)


def check_apple_colorsets(root: Path, light: dict[str, str], dark: dict[str, str], errors: list[str]) -> None:
    role_map = {
        "NolaCanvas": (light["canvas"], dark["canvas"]),
        "NolaSurface": (light["surface"], dark["surface"]),
        "NolaSurfaceRaised": (light["surfaceElevated"], dark["surfaceElevated"]),
        "NolaAccentSubtle": (light["accentSubtle"], dark["accentSubtle"]),
        "NolaDivider": (light["border"], dark["border"]),
        "NolaControlOutline": (light["outline"], dark["outline"]),
        "NolaTextPrimary": (light["ink"], dark["ink"]),
        "AccentColor": (light["accent"], dark["accent"]),
        "NolaTextSecondary": (light["inkSecondary"], dark["inkSecondary"]),
        "NolaOnAccent": (light["onAccent"], dark["onAccent"]),
        "NolaSuccess": (light["success"], dark["success"]),
        "NolaWarning": (light["warning"], dark["warning"]),
        "NolaDanger": (light["danger"], dark["danger"]),
    }

    def entry_hex(entry: dict) -> str | None:
        comp = entry.get("color", {}).get("components", {})
        r, g, b = comp.get("red"), comp.get("green"), comp.get("blue")
        if not (isinstance(r, str) and isinstance(g, str) and isinstance(b, str)):
            return None
        if not all(val.lower().startswith("0x") and len(val) == 4 for val in (r, g, b)):
            return None
        return f"#{r[2:].lower()}{g[2:].lower()}{b[2:].lower()}"

    for platform_dir in ("apps/macos/Assets.xcassets", "apps/ios/Assets.xcassets"):
        base = root / platform_dir
        for colorset_name, (exp_light, exp_dark) in role_map.items():
            contents_path = base / f"{colorset_name}.colorset" / "Contents.json"
            if not contents_path.is_file():
                errors.append(f"{contents_path.relative_to(root)}: missing colorset Contents.json")
                continue
            payload = json.loads(contents_path.read_text(encoding="utf-8"))
            actual_light = None
            actual_dark = None
            for entry in payload.get("colors", []):
                appearances = entry.get("appearances") or []
                app_pairs = {(a.get("appearance"), a.get("value")) for a in appearances}
                hx = entry_hex(entry)
                if hx is None:
                    errors.append(f"{contents_path.relative_to(root)}: non-hex srgb components in entry")
                    continue
                if hx in FORBIDDEN_COLD_OR_OLED_SURFACES:
                    errors.append(
                        f"{contents_path.relative_to(root)}: contains forbidden cold/OLED surface {hx}"
                    )
                if not appearances:
                    actual_light = hx
                elif app_pairs == {("luminosity", "dark")}:
                    actual_dark = hx
            if actual_light != normalize_hex(exp_light):
                errors.append(
                    f"{contents_path.relative_to(root)} (light): expected {normalize_hex(exp_light)}, got {actual_light!r}"
                )
            if actual_dark != normalize_hex(exp_dark):
                errors.append(
                    f"{contents_path.relative_to(root)} (dark): expected {normalize_hex(exp_dark)}, got {actual_dark!r}"
                )


def run_checks(root: Path = ROOT) -> tuple[list[str], int, int]:
    tokens_path = root / "shared/design-tokens/tokens.json"
    if not tokens_path.is_file():
        return [f"Missing canonical design tokens file: {tokens_path.relative_to(root)}"], 0, 0

    tokens = json.loads(tokens_path.read_text(encoding="utf-8"))
    errors: list[str] = []
    contrast_checks = check_tokens_schema_and_contrast(tokens, errors)
    if not errors:
        light = tokens["color"]["light"]
        dark = tokens["color"]["dark"]
        check_prototype_css(root, light, dark, errors)
        check_web_theme_css(root, light, dark, errors)
        check_android_theme(root, light, dark, errors)
        check_windows_shell(root, light, dark, errors)
        check_apple_fallbacks(root, light, dark, errors)
        check_apple_colorsets(root, light, dark, errors)

    schema_version = int(tokens.get("schemaVersion", 0)) if isinstance(tokens.get("schemaVersion"), int) else 0
    return errors, schema_version, contrast_checks


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    root = args.root.resolve()

    errors, schema_version, contrast_checks = run_checks(root)
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print(
        f"Passed: shared/design-tokens/tokens.json schema v{schema_version}, "
        f"{contrast_checks} WCAG contrast checks, and full-source 6-platform surface alignment"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
