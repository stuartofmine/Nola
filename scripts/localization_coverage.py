#!/usr/bin/env python3
"""Audit localization-key, locale coverage, and obvious untranslated UI copy.

This is intentionally a contract check, not a translation-quality judge. It
prevents a localized UI helper from silently falling back to English because a
key never entered a catalog or because one supported locale was omitted.
"""

from __future__ import annotations

import argparse
import ast
import collections
import html
import json
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MAC_LOCALES = (
    "de",
    "es",
    "fr",
    "ja",
    "ko",
    "ru",
    "zh-Hans",
    "zh-Hant",
)
IOS_LOCALES = ("de", "es", "fr", "ja", "ru", "zh-Hans")
WEB_LOCALES = ("zh-Hans", "en", "de", "es", "fr", "ja", "ru")

SWIFT_KEY_PATTERNS = (
    re.compile(r'String\(localized:\s*"([^"\\]+)"'),
    re.compile(r'NSLocalizedString\(\s*"([^"\\]+)"'),
    re.compile(r'LocalizedStringKey\(\s*"([^"\\]+)"'),
    re.compile(
        r'\b(?:Text|Label|Button|Toggle|Picker|Section|GroupBox|Menu|Link|TextField|SecureField|ContentUnavailableView|SettingsLink|LabeledContent|ShareLink|ProgressView)\(\s*"([^"\\]+)"'
    ),
    re.compile(r'\.(?:navigationTitle|help|accessibilityLabel|accessibilityHint)\(\s*"([^"\\]+)"'),
    re.compile(r'\b[A-Za-z][A-Za-z0-9]*Text\(\s*"([^"\\]+)"\s*,'),
    re.compile(r'\b[A-Za-z][A-Za-z0-9_]*Text\(\s*"([^"\\]+)"\s*,'),
)

NONLOCALIZED_SWIFT_LITERALS = {
    "-",
    "·",
    "•",
    "Nola",
    "0",
    "0.7",
    "4096",
    "512",
    "https://example.com/mcp",
    "0.00 ... 2.00",
    "0.10 ... 1.50",
    "16 ... 1024 tokens",
    "512 ... 32768 tokens",
    "4096 ... 32768 tokens",
    "nola web --host 0.0.0.0 --port 5001",
}

RAW_RUNTIME_LOCALIZATION_PATTERNS = (
    re.compile(r'return\s+\.failure\(\s*"'),
    re.compile(r'\bpanel\.(?:title|message|prompt)\s*=\s*"'),
    re.compile(
        r'\b(?:Text|Label|Button)\s*\([^,\n]*\?\s*"(?!Nola"|CODE"|\")(?=[A-Za-z])'
    ),
    re.compile(r'\.help\s*\([^\n]*(?:\?|\?\?)\s*"(?!\")(?=[A-Za-z])'),
)

SIMPLIFIED_CHINESE_MARKERS = (
    "错误",
    "复制",
    "路径",
    "没有",
    "暂无",
    "项目",
    "内容",
    "打开",
    "应用",
    "测试",
    "这里",
    "通过",
    "生产",
    "链路",
    "耗时",
    "服务",
    "修复",
    "请",
    "解压",
    "然后",
    "页面",
    "所选",
    "显示",
    "编辑",
    "参数",
    "主机",
    "三个",
    "什么",
    "简洁",
    "要点",
    "说明",
)


def audit_raw_runtime_localization(name: str, root: Path) -> list[str]:
    """Reject strings that bypass localization in runtime and AppKit sinks."""
    failures: list[str] = []
    for path in root.rglob("*.swift"):
        source = path.read_text(encoding="utf-8")
        for line_number, line in enumerate(source.splitlines(), start=1):
            if any(pattern.search(line) for pattern in RAW_RUNTIME_LOCALIZATION_PATTERNS):
                failures.append(
                    f"{name}: {path.relative_to(root)}:{line_number}: raw runtime string bypasses localization"
                )
    return failures


def swift_keys(root: Path) -> set[str]:
    keys: set[str] = set()
    for path in root.rglob("*.swift"):
        source = path.read_text(encoding="utf-8")
        for pattern in SWIFT_KEY_PATTERNS:
            keys.update(pattern.findall(source))
    return {
        key
        for key in keys
        if "\\(" not in key
        and "%@" not in key
        and key not in NONLOCALIZED_SWIFT_LITERALS
    }


def apple_catalogs(root: Path) -> dict[str, dict]:
    entries: dict[str, dict] = {}
    for path in root.glob("*.xcstrings"):
        payload = json.loads(path.read_text(encoding="utf-8"))
        entries.update(payload.get("strings", {}))
    strings_entry = re.compile(
        r'^\s*"((?:\\.|[^"\\])*)"\s*=\s*"((?:\\.|[^"\\])*)"\s*;\s*$',
        re.MULTILINE,
    )
    for path in root.glob("*.lproj/*.strings"):
        locale = path.parent.name.removesuffix(".lproj")
        source = path.read_text(encoding="utf-8")
        for raw_key, raw_value in strings_entry.findall(source):
            key = bytes(raw_key, "utf-8").decode("unicode_escape") if "\\" in raw_key else raw_key
            value = (
                bytes(raw_value, "utf-8").decode("unicode_escape")
                if "\\" in raw_value and raw_value.isascii()
                else raw_value.replace(r'\"', '"').replace(r"\\", "\\")
            )
            entry = entries.setdefault(key, {})
            entry.setdefault("localizations", {})[locale] = {
                "stringUnit": {"state": "translated", "value": value}
            }
    return entries


def audit_apple_catalog_integrity(
    name: str,
    catalog_root: Path,
    locales: tuple[str, ...],
) -> list[str]:
    """Reject incomplete catalogs and ambiguous duplicate translations.

    Merging catalogs into one dictionary is useful for source-key coverage, but
    it used to hide missing values: a complete entry in Chat.xcstrings could
    overwrite an incomplete entry in Localizable.xcstrings. Xcode can compile
    both tables, so each table must be independently complete and duplicate
    keys must resolve to the same user-facing copy.
    """
    failures: list[str] = []
    occurrences: dict[str, list[tuple[Path, dict]]] = collections.defaultdict(list)
    for path in sorted(catalog_root.glob("*.xcstrings")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        for key, entry in payload.get("strings", {}).items():
            if not key.strip():
                failures.append(f"{name}: {path.name}: empty localization key")
                continue
            occurrences[key].append((path, entry))
            for locale in locales:
                if translated_value(entry, locale) is None:
                    failures.append(f"{name}: {path.name}: {key}: missing {locale}")

    strings_entry = re.compile(
        r'^\s*"((?:\\.|[^"\\])*)"\s*=\s*"((?:\\.|[^"\\])*)"\s*;\s*$',
        re.MULTILINE,
    )
    for path in sorted(catalog_root.glob("*.lproj/*.strings")):
        values_by_key: dict[str, set[str]] = collections.defaultdict(set)
        for key, value in strings_entry.findall(path.read_text(encoding="utf-8")):
            values_by_key[key].add(value)
        for key, values in sorted(values_by_key.items()):
            if len(values) > 1:
                failures.append(
                    f"{name}: {path.relative_to(catalog_root)}: conflicting duplicate key: {key}"
                )

    for key, entries in sorted(occurrences.items()):
        if len(entries) < 2:
            continue
        for locale in locales:
            values = {
                value
                for _, entry in entries
                if (value := translated_value(entry, locale)) is not None
            }
            if len(values) > 1:
                tables = ", ".join(path.name for path, _ in entries)
                failures.append(
                    f"{name}: conflicting {locale} translation for {key}: {tables}"
                )
    return failures


def audit_apple_catalog_routing(
    name: str,
    source_root: Path,
    catalog_root: Path,
) -> list[str]:
    """Ensure explicit String Catalog calls resolve against the right table.

    A merged-key coverage check can pass even when a key exists only in
    Chat.xcstrings but the call omits `table: "Chat"`. At runtime that lookup
    uses Localizable and exposes the raw key. Validate each literal
    `String(localized:)` call against its actual table instead.
    """
    failures: list[str] = []
    tables: dict[str, set[str]] = {}
    for path in catalog_root.glob("*.xcstrings"):
        payload = json.loads(path.read_text(encoding="utf-8"))
        tables[path.stem] = set(payload.get("strings", {}))
    # .strings and .xcstrings compile to the same runtime table. Prefer the
    # source-language table; translations alone must not mask a missing key.
    for path in catalog_root.glob("en.lproj/*.strings"):
        keys = re.findall(r'^\s*"((?:\\.|[^"\\])*)"\s*=', path.read_text(encoding="utf-8"), re.MULTILINE)
        tables.setdefault(path.stem, set()).update(keys)
    default_keys = tables.get("Localizable", set())
    call_pattern = re.compile(
        r'String\(localized:\s*"([^"\\]+)"(?P<arguments>[^)]*)\)',
        re.MULTILINE,
    )
    table_pattern = re.compile(r'table:\s*"([^"\\]+)"')
    for path in source_root.rglob("*.swift"):
        source = path.read_text(encoding="utf-8")
        for match in call_pattern.finditer(source):
            key = match.group(1)
            table_match = table_pattern.search(match.group("arguments"))
            table = table_match.group(1) if table_match else "Localizable"
            available = tables.get(table)
            line = source.count("\n", 0, match.start()) + 1
            if available is None:
                failures.append(
                    f"{name}: {path.relative_to(source_root)}:{line}: missing catalog table {table}"
                )
            elif key not in available:
                failures.append(
                    f"{name}: {path.relative_to(source_root)}:{line}: {key}: missing from {table}.xcstrings"
                )
            elif table == "Localizable" and key not in default_keys:
                failures.append(
                    f"{name}: {path.relative_to(source_root)}:{line}: {key}: default lookup cannot resolve"
                )
    return failures


def translated_value(entry: dict, locale: str) -> str | None:
    unit = entry.get("localizations", {}).get(locale, {}).get("stringUnit", {})
    if unit.get("state") != "translated":
        return None
    value = unit.get("value")
    return value if isinstance(value, str) and value.strip() else None


APPLE_FORMAT_PATTERN = re.compile(
    r"%(?:\d+\$)?[-+#0 ']*\d*(?:\.\d+)?(?:hh|h|ll|l|j|z|t|L)?[@a-zA-Z%]"
)
STRUCTURED_LOCALIZATION_KEY = re.compile(
    r"^[a-z][a-z0-9_]*(?:\.[a-z0-9_]+)+$"
)


def apple_placeholders(value: str) -> list[str]:
    """Return semantic printf conversion types, ignoring legal reordering."""
    tokens: list[str] = []
    for token in APPLE_FORMAT_PATTERN.findall(value):
        if token == "%%":
            continue
        match = re.search(r"(?:hh|h|ll|l|j|z|t|L)?[@a-zA-Z]$", token)
        if match is not None:
            tokens.append(match.group(0))
    return sorted(tokens)


STABLE_UI_TERMS = {
    "64K",
    "Chrome",
    "Hugging Face",
    "JSON",
    "MLX",
    "MCP",
    "ModelScope",
    "Nola",
    "Nola CLI",
    "Nola for Mac",
    "OK",
    "PROMPT",
    "RSS / Atom",
    "Top K",
    "Top P",
    "URL",
    "Wiki",
    "WeCom",
    "Web",
    "Health Connect",
    "HIIT",
    "gpt-4o",
    "API",
    "HTML",
    "HTTP",
    "ID",
    "RSS",
    "UUID",
    "bpm",
    "esc",
    "kcal",
    "km",
    "min",
    "nola",
}


def obviously_untranslated(source: str, localized: str, locale: str) -> bool:
    if source.casefold() != localized.casefold():
        return False
    if re.search(r"https?://|^[%0-9$@./:_ -]+$", source):
        return False
    without_formats = APPLE_FORMAT_PATTERN.sub("", source)
    if not re.search(r"[A-Za-z]", without_formats):
        return False
    if without_formats.strip(" ·/()") in STABLE_UI_TERMS:
        return False
    if re.fullmatch(
        r"(?:API|HTTP|HTTPS|JSON|HTML|RSS|UUID|ID)(?:\s*/\s*(?:API|HTTP|HTTPS|JSON|HTML|RSS|UUID|ID))*",
        source,
    ):
        return False
    if re.fullmatch(r"(?:%[^ ]+\s+)?(?:tokens?|bpm|kcal|km|min)", source):
        return False
    if re.fullmatch(r"[a-z]{2}(?:-[A-Za-z]+)?(?:,\s*[a-z]{2}(?:-[A-Za-z]+)?)*…?", source):
        return False
    if re.fullmatch(r"[A-Za-z_]+/[A-Za-z_]+", source):
        return False
    if source in {"model-name"}:
        return False
    # Product names, technical controls, and measurement units intentionally
    # remain stable across locales.
    if source in STABLE_UI_TERMS or source == "Quick Look":
        return False
    # Short English fallbacks are especially visible in CJK secondary pages.
    # The previous five-word threshold let entire labels such as "Not
    # downloaded" and "OpenAI-compatible provider" ship untranslated.
    if locale in {"ja", "ko", "zh-Hans", "zh-Hant"}:
        return bool(re.search(r"[A-Za-z]{2,}", source))
    return len(re.findall(r"[A-Za-z]+", source)) >= 5


def audit_apple(
    name: str,
    source_root: Path,
    catalog_root: Path,
    locales: tuple[str, ...],
    quality: bool,
) -> list[str]:
    failures = audit_apple_catalog_integrity(name, catalog_root, locales)
    failures.extend(audit_apple_catalog_routing(name, source_root, catalog_root))
    failures.extend(audit_raw_runtime_localization(name, source_root))
    keys = swift_keys(source_root)
    entries = apple_catalogs(catalog_root)
    if name == "ios":
        for key in sorted(key for key in entries if key.startswith("mac.")):
            failures.append(f"{name}: Mac-only key leaked into iOS resources: {key}")
    # String Catalogs also contain keys reached through dynamic navigation,
    # AppKit bridges, formatted SwiftUI labels, and generated subpage models.
    # Limiting the contract to a few source-code regexes made those screens
    # silently fall back to English while the audit reported success.
    keys.update(key for key in entries if key.strip())
    for key in sorted(keys - entries.keys()):
        failures.append(f"{name}: missing catalog key: {key}")
    for key in sorted(keys & entries.keys()):
        english_value = translated_value(entries[key], "en")
        if english_value is None:
            if STRUCTURED_LOCALIZATION_KEY.fullmatch(key):
                failures.append(f"{name}: {key}: missing English source")
                continue
            source_value = key
        else:
            source_value = english_value
        if source_value == key and STRUCTURED_LOCALIZATION_KEY.fullmatch(key):
            failures.append(f"{name}: {key}: internal key leaked into English UI")
        for locale in locales:
            value = translated_value(entries[key], locale)
            if value is None:
                failures.append(f"{name}: {key}: missing {locale}")
            elif apple_placeholders(source_value) != apple_placeholders(value):
                failures.append(f"{name}: {key}: placeholder mismatch in {locale}")
            elif locale == "zh-Hant" and any(
                marker in value for marker in SIMPLIFIED_CHINESE_MARKERS
            ):
                failures.append(f"{name}: {key}: simplified Chinese leaked into zh-Hant")
            elif quality and (
                (key.startswith("autogen.") and value == key)
                or obviously_untranslated(source_value, value, locale)
            ):
                failures.append(f"{name}: {key}: untranslated {locale}")
    return failures


def android_strings(path: Path) -> set[str]:
    tree = ET.parse(path)
    return {
        node.attrib["name"]
        for node in tree.getroot()
        if node.tag in {"string", "plurals", "string-array"} and "name" in node.attrib
    }


ANDROID_FORMAT_PATTERN = re.compile(r"%(?:\d+\$)?[A-Za-z%]")


def android_string_values(path: Path) -> dict[str, str]:
    tree = ET.parse(path)
    return {
        node.attrib["name"]: "".join(node.itertext())
        for node in tree.getroot()
        if node.tag == "string" and "name" in node.attrib
    }


def android_placeholders(value: str) -> list[str]:
    placeholders: list[str] = []
    for token in ANDROID_FORMAT_PATTERN.findall(value):
        if token == "%%":
            continue
        placeholders.append(token[-1])
    return sorted(placeholders)


def audit_android(quality: bool = False) -> list[str]:
    resources = ROOT / "android/app/src/main/res"
    default_path = resources / "values/strings.xml"
    if not default_path.exists():
        return ["android: missing values/strings.xml"]
    default = android_strings(default_path)
    default_values = android_string_values(default_path)
    failures: list[str] = []
    locale_dirs = {
        "de": "values-de",
        "es": "values-es",
        "fr": "values-fr",
        "ja": "values-ja",
        "ru": "values-ru",
        "zh-Hans": "values-zh",
    }
    for locale, directory in locale_dirs.items():
        path = resources / directory / "strings.xml"
        if not path.exists():
            failures.append(f"android: missing locale resource: {locale} ({directory})")
            continue
        localized = android_strings(path)
        localized_values = android_string_values(path)
        for key in sorted(default - localized):
            failures.append(f"android: {key}: missing {locale}")
        for key in sorted(default & localized):
            if android_placeholders(default_values.get(key, "")) != android_placeholders(
                localized_values.get(key, "")
            ):
                failures.append(f"android: {key}: placeholder mismatch in {locale}")
            elif quality and obviously_untranslated(
                default_values.get(key, ""), localized_values.get(key, ""), locale
            ):
                failures.append(f"android: {key}: untranslated {locale}")
    return failures


def rust_match_strings(source: str, function_name: str) -> dict[str, str]:
    match = re.search(
        rf"fn {re.escape(function_name)}\(message: Message\).*?"
        r"\{\s*match message \{(.*?)\n\s*\}\n\}",
        source,
        re.DOTALL,
    )
    if match is None:
        return {}
    body = match.group(1)
    positions = [
        (item.start(), item.group(1))
        for item in re.finditer(r"Message::(\w+)\s*=>", body)
    ]
    values: dict[str, str] = {}
    for index, (start, key) in enumerate(positions):
        end = positions[index + 1][0] if index + 1 < len(positions) else len(body)
        literal = re.search(r'"(?:\\.|[^"\\])*"', body[start:end], re.DOTALL)
        if literal is None:
            continue
        try:
            values[key] = ast.literal_eval(literal.group(0))
        except (SyntaxError, ValueError):
            continue
    return values


def audit_cli(quality: bool) -> list[str]:
    source_path = ROOT / "cli/src/i18n.rs"
    if not source_path.exists():
        return ["cli: missing cli/src/i18n.rs"]
    source = source_path.read_text(encoding="utf-8")
    enum_match = re.search(r"pub enum Message\s*\{(.*?)\n\}", source, re.DOTALL)
    if enum_match is None:
        return ["cli: Message enum not found"]
    variants = set(
        re.findall(r"^\s*([A-Z][A-Za-z0-9_]*)\s*,", enum_match.group(1), re.MULTILINE)
    )
    failures: list[str] = []
    english = rust_match_strings(source, "en")
    for missing in sorted(variants - english.keys()):
        failures.append(f"cli: en: missing Message::{missing}")
    for locale in ("zh_hans", "de", "es", "fr", "ja", "ru"):
        values = rust_match_strings(source, locale)
        for missing in sorted(variants - values.keys()):
            failures.append(f"cli: {locale}: missing Message::{missing}")
        if re.search(
            rf"fn {locale}\(message: Message\).*?_\s*=>\s*en\(message\)",
            source,
            re.DOTALL,
        ):
            failures.append(f"cli: {locale}: English fallback is not allowed")
        if quality:
            for key in sorted(variants & values.keys() & english.keys()):
                quality_locale = "zh-Hans" if locale == "zh_hans" else locale
                if obviously_untranslated(english[key], values[key], quality_locale):
                    failures.append(f"cli: {locale}: Message::{key}: untranslated")
    return failures


PLACEHOLDER_PATTERN = re.compile(
    r"https?://[^\s|)]+|%\([^)]+\)[A-Za-z]|%[A-Za-z]|\{[^{}]+\}"
)


def web_placeholders(value: str) -> list[str]:
    """Compare semantic placeholders rather than HTML serialization details."""
    decoded = html.unescape(value)
    return sorted(
        token.rstrip(".,;!?。；！？")
        for token in PLACEHOLDER_PATTERN.findall(decoded)
    )


def web_runtime_snapshot() -> dict:
    result = subprocess.run(
        ["node", "--experimental-strip-types", str(Path(__file__).with_name("web_localization_snapshot.mjs")), str(ROOT)],
        capture_output=True, text=True, check=True, timeout=60,
    )
    return json.loads(result.stdout)


def audit_web_snapshot(snapshot: dict, quality: bool) -> list[str]:
    """Audit the actual runtime registry and current Svelte/TypeScript source."""
    failures: list[str] = []
    if snapshot.get("locales") != list(WEB_LOCALES):
        failures.append("web: runtime supportedLocales must be the current seven locales")
    for duplicate in snapshot.get("duplicateKeys", []):
        failures.append(f"web: {duplicate['file']}: duplicate catalog source key {duplicate['key']}")
    catalogs = snapshot.get("catalogs")
    if not isinstance(catalogs, list) or not catalogs:
        return failures + ["web: runtime messageCatalogs is empty or invalid"]
    known: set[str] = set()
    for index, catalog in enumerate(catalogs):
        if not isinstance(catalog, dict) or not isinstance(catalog.get("en"), dict) or not catalog["en"]:
            failures.append(f"web: catalog {index}: missing nonempty English source")
            continue
        english = catalog["en"]
        if set(catalog) != set(WEB_LOCALES):
            failures.append(f"web: catalog {index}: locale set differs from supportedLocales")
        for key in english:
            if key in known:
                failures.append(f"web: duplicate registered key: {key}")
            if not key.strip():
                failures.append(f"web: catalog {index}: empty localization key")
        known.update(english)
        for locale in WEB_LOCALES:
            values = catalog.get(locale)
            if not isinstance(values, dict):
                failures.append(f"web: catalog {index}: missing {locale}")
                continue
            for key in sorted(english.keys() - values.keys()):
                failures.append(f"web: {key}: missing {locale}")
            for key in sorted(values.keys() - english.keys()):
                failures.append(f"web: {key}: unexpected key in {locale}")
            for key in sorted(english.keys() & values.keys()):
                source, localized = english[key], values[key]
                if not isinstance(source, str) or not source.strip() or not isinstance(localized, str) or not localized.strip():
                    failures.append(f"web: {key}: empty or invalid {locale}")
                    continue
                if web_placeholders(source) != web_placeholders(localized):
                    failures.append(f"web: {key}: placeholder mismatch in {locale}")
                if quality and locale != "en" and not key.endswith(".placeholder") and obviously_untranslated(source, localized, locale):
                    failures.append(f"web: {key}: untranslated {locale}")
    source_keys = snapshot.get("sourceKeys")
    if not isinstance(source_keys, list) or not source_keys or not snapshot.get("sourceFiles"):
        failures.append("web: current Svelte/TypeScript source key audit is empty")
    else:
        for occurrence in source_keys:
            if occurrence["key"] not in known:
                failures.append(f"web: {occurrence['file']}: unregistered source key {occurrence['key']}")
    return failures


def audit_web(quality: bool) -> list[str]:
    try:
        return audit_web_snapshot(web_runtime_snapshot(), quality)
    except (OSError, subprocess.SubprocessError, ValueError, TypeError, KeyError) as error:
        return [f"web: current runtime catalog/source audit unavailable: {type(error).__name__}: {error}"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--platform",
        choices=("all", "mac", "ios", "android", "cli", "web"),
        default="all",
    )
    parser.add_argument("--max-errors", type=int, default=200)
    parser.add_argument(
        "--quality",
        action="store_true",
        help="also reject sentence-like localized values copied from English",
    )
    args = parser.parse_args()

    failures: list[str] = []
    if args.platform in {"all", "mac"}:
        failures.extend(
            audit_apple("mac", ROOT / "NolaMac", ROOT / "NolaMac", MAC_LOCALES, args.quality)
        )
    if args.platform in {"all", "ios"}:
        failures.extend(audit_apple("ios", ROOT / "Nola", ROOT / "Nola", IOS_LOCALES, args.quality))
    if args.platform in {"all", "android"}:
        failures.extend(audit_android(args.quality))
    if args.platform in {"all", "cli"}:
        failures.extend(audit_cli(args.quality))
    if args.platform in {"all", "web"}:
        failures.extend(audit_web(args.quality))

    if failures:
        print(f"localization coverage failed: {len(failures)} issue(s)", file=sys.stderr)
        for failure in failures[: max(0, args.max_errors)]:
            print(f"- {failure}", file=sys.stderr)
        if len(failures) > args.max_errors:
            print(
                f"- ... {len(failures) - args.max_errors} more (raise --max-errors)",
                file=sys.stderr,
            )
        return 1

    audited_scope = "all configured platforms" if args.platform == "all" else args.platform
    print(f"localization coverage passed for {audited_scope}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
