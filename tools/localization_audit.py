#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Implements localization audit logic in the project module.
# - Interfaces:
#   - unescape_min
#   - parse_strings
#   - placeholders
#   - Finding
#   - looks_like_real_text
#   - main
# - APIs:
#   - none
#
import argparse
import datetime
import pathlib
import re
from dataclasses import dataclass


PAIR_RE = re.compile(r'"((?:\\.|[^"\\])*)"\s*=\s*"((?:\\.|[^"\\])*)"\s*;')


def unescape_min(s: str) -> str:
    return (
        s.replace('\\"', '"')
        .replace('\\n', '\n')
        .replace('\\t', '\t')
        .replace('\\r', '\r')
    )


def parse_strings(path: pathlib.Path) -> tuple[dict[str, str], list[tuple[str, str, str]]]:
    text = path.read_text(encoding="utf-8", errors="replace")
    items: dict[str, str] = {}
    dupes: list[tuple[str, str, str]] = []

    seen: dict[str, str] = {}
    for m in PAIR_RE.finditer(text):
        k = unescape_min(m.group(1))
        v = unescape_min(m.group(2))
        if k in seen and seen[k] != v:
            dupes.append((k, seen[k], v))
        seen.setdefault(k, v)
        items.setdefault(k, v)

    return items, dupes


# Heuristic placeholder matcher for String(format:)
# NOTE: We must treat '%%' as a literal percent, not a placeholder.
PLACEHOLDER_RE = re.compile(
    r'%(?:\d+\$)?[-+# 0]*(?:\d+)?(?:\.\d+)?(?:hh|h|ll|l)?[diuoxXfFeEgGaAcCsSp@]'
)


def placeholders(s: str) -> list[str]:
    out: list[str] = []
    i = 0
    while i < len(s):
        if s[i] != '%':
            i += 1
            continue
        # Skip literal percent
        if i + 1 < len(s) and s[i + 1] == '%':
            i += 2
            continue
        m = PLACEHOLDER_RE.match(s, i)
        if m:
            out.append(m.group(0))
            i = m.end()
        else:
            i += 1
    return out


@dataclass(frozen=True)
class Finding:
    path: str
    value: str


def looks_like_real_text(s: str) -> bool:
    s2 = s.strip()
    if not s2:
        return False
    if s2 in {"•", "·", "-", "BG"}:
        return False
    if re.fullmatch(r"[0-9:%\s\.]+", s2):
        return False
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description="Audit client localization")
    parser.add_argument(
        "--root",
        default=str(pathlib.Path(__file__).resolve().parents[1]),
        help="Repo root (default: inferred)",
    )
    parser.add_argument(
        "--app-dir",
        default="Nola",
        help="App directory relative to repo root (default: Nola)",
    )
    parser.add_argument(
        "--baseline",
        default="en",
        help="Baseline language code for comparison (default: en)",
    )
    parser.add_argument(
        "--out",
        default="Doc/当前任务/LOCALIZATION_AUDIT.md",
        help="Output markdown path relative to repo root",
    )
    args = parser.parse_args()

    root = pathlib.Path(args.root).resolve()
    app_root = (root / args.app_dir).resolve()

    strings_files = sorted(app_root.glob("**/*.lproj/*.strings"))
    swift_files = sorted(app_root.glob("**/*.swift"))

    by_lang: dict[str, dict[str, str]] = {}
    duplicates: list[tuple[str, str, str, str]] = []

    for sf in strings_files:
        lang = sf.parent.name.replace(".lproj", "")
        mapping, dupes = parse_strings(sf)
        by_lang.setdefault(lang, {}).update(mapping)
        for k, a, b in dupes:
            duplicates.append((str(sf.relative_to(root)), k, a, b))

    baseline_lang = args.baseline
    baseline = by_lang.get(baseline_lang, {})

    # Extract keys defined by our centralized localization layer
    localizable_swift = app_root / 'Services' / 'Localizable.swift'
    defined_keys: set[str] = set()
    if localizable_swift.exists():
        txt = localizable_swift.read_text(encoding='utf-8', errors='replace')
        # Matches: string("...") inside L10n
        defined_keys.update(re.findall(r'\bstring\(\s*\"([^\"]+)\"', txt))

    # Extract used localization keys from Swift (direct calls / string extension)
    key_pats = [
        re.compile(r"\bL10n\.(?:string|format)\(\s*\"([^\"]+)\""),
        re.compile(r"\bNSLocalizedString\(\s*\"([^\"]+)\""),
        re.compile(r"\"([^\"]+)\"\.localized\b"),
        re.compile(r"\"([^\"]+)\"\.localized\("),
    ]

    used_keys: set[str] = set()
    for sf in swift_files:
        txt = sf.read_text(encoding="utf-8", errors="replace")
        for pat in key_pats:
            used_keys.update(key for key in pat.findall(txt) if "\\(" not in key)

    missing_in_baseline = sorted(used_keys - set(baseline.keys())) if baseline else sorted(used_keys)

    missing_defined_in_baseline: list[str] = []
    if baseline and defined_keys:
        missing_defined_in_baseline = sorted(defined_keys - set(baseline.keys()))

    unused_in_defined: list[str] = []
    if baseline and defined_keys:
        unused_in_defined = sorted(set(baseline.keys()) - defined_keys)

    # Hardcoded UI strings (heuristic)
    hardcoded_pats = [
        re.compile(r"Text\(\s*\"([^\"]+)\"\s*\)"),
        re.compile(r"Button\(\s*\"([^\"]+)\"\s*,"),
        re.compile(r"Label\(\s*\"([^\"]+)\"\s*,"),
        re.compile(r"\.navigationTitle\(\s*\"([^\"]+)\"\s*\)"),
        re.compile(r"\.alert\(\s*\"([^\"]+)\"\s*,"),
    ]

    hardcoded: list[Finding] = []
    for sf in swift_files:
        txt = sf.read_text(encoding="utf-8", errors="replace")
        for pat in hardcoded_pats:
            for val in pat.findall(txt):
                if looks_like_real_text(val):
                    hardcoded.append(Finding(str(sf.relative_to(root)), val))

    hardcoded_count_by_file: dict[str, int] = {}
    for f in hardcoded:
        hardcoded_count_by_file[f.path] = hardcoded_count_by_file.get(f.path, 0) + 1

    # Missing/extra vs baseline
    missing_by_lang: dict[str, list[str]] = {}
    extra_by_lang: dict[str, list[str]] = {}
    if baseline:
        for lang, mapping in by_lang.items():
            missing_by_lang[lang] = sorted(set(baseline.keys()) - set(mapping.keys()))
            extra_by_lang[lang] = sorted(set(mapping.keys()) - set(baseline.keys()))

    # Placeholder mismatch
    all_keys: set[str] = set()
    for mapping in by_lang.values():
        all_keys.update(mapping.keys())

    placeholder_mismatches: list[tuple[str, str, list[str], str, list[str]]] = []
    for key in sorted(all_keys):
        ph_sets: dict[str, list[str]] = {}
        for lang, mapping in by_lang.items():
            if key in mapping:
                ph_sets[lang] = placeholders(mapping[key])
        if len(ph_sets) < 2:
            continue
        ref_lang = baseline_lang if baseline_lang in ph_sets else next(iter(ph_sets.keys()))
        ref = sorted(ph_sets[ref_lang])
        for lang, ph in ph_sets.items():
            normalized = sorted(ph)
            if normalized != ref:
                placeholder_mismatches.append((key, ref_lang, ref, lang, normalized))

    # Suspicious TODO/TBD/FIXME markers in values
    # (keep conservative to avoid false positives like "ToDo")
    suspicious: list[tuple[str, str, str]] = []
    for lang, mapping in by_lang.items():
        for k, v in mapping.items():
            if re.search(r"(?:^|\W)(?:TODO:|TBD:|FIXME:)(?:\W|$)", v):
                suspicious.append((lang, k, v))

    # Potentially untranslated: value identical to baseline (en) in non-en languages
    identical_to_baseline: dict[str, list[str]] = {}
    if baseline:
        for lang, mapping in by_lang.items():
            if lang == baseline_lang:
                continue
            same = [k for k, v in mapping.items() if k in baseline and v == baseline[k]]
            if same:
                identical_to_baseline[lang] = sorted(same)

    # Known typo family: urlsourses.* (should be urlsources.*)
    typo_urlsourses: dict[str, list[str]] = {}
    for lang, mapping in by_lang.items():
        bad = sorted([k for k in mapping.keys() if k.startswith('urlsourses.')])
        if bad:
            typo_urlsourses[lang] = bad

    out_path = root / args.out
    out_path.parent.mkdir(parents=True, exist_ok=True)

    lines: list[str] = []
    lines.append("# Localization Audit (Client)")
    lines.append("")
    lines.append(f"生成时间: {datetime.date.today().isoformat()}")
    lines.append(f"扫描范围: {args.app_dir} Swift 源码 + {args.app_dir}/**/*.lproj/*.strings")
    lines.append("")
    lines.append("## 总览")
    lines.append(f"- strings 文件: {len(strings_files)}")
    lines.append(f"- Swift 文件: {len(swift_files)}")
    lines.append(f"- Swift 引用 key 数: {len(used_keys)}")
    lines.append(f"- 基准语言: {baseline_lang} (keys={len(baseline)})")
    lines.append("")

    lines.append("## 1) Swift 引用但基准语言缺失的 key（高优先级）")
    lines.append(f"- 缺失数量: {len(missing_in_baseline)}")
    for k in missing_in_baseline[:200]:
        lines.append(f"  - {k}")
    if len(missing_in_baseline) > 200:
        lines.append(f"  - ...(仅展示前200项，剩余 {len(missing_in_baseline)-200} 项)")
    lines.append("")

    lines.append("## 1.1) Localizable.swift 定义但基准语言缺失的 key（高优先级）")
    lines.append(f"- 缺失数量: {len(missing_defined_in_baseline)}")
    for k in missing_defined_in_baseline[:200]:
        lines.append(f"  - {k}")
    if len(missing_defined_in_baseline) > 200:
        lines.append(f"  - ...(仅展示前200项，剩余 {len(missing_defined_in_baseline)-200} 项)")
    lines.append("")

    if unused_in_defined:
        lines.append("## 1.2) 基准语言存在但未在 Localizable.swift 定义（可能是遗留/未使用）")
        lines.append(f"- 数量: {len(unused_in_defined)}")
        for k in unused_in_defined[:120]:
            lines.append(f"  - {k}")
        if len(unused_in_defined) > 120:
            lines.append(f"  - ...(仅展示前120项，剩余 {len(unused_in_defined)-120} 项)")
        lines.append("")

    if baseline:
        lines.append("## 2) 各语言相对基准语言的缺失 key")
        for lang in sorted(missing_by_lang.keys()):
            if lang == baseline_lang:
                continue
            missing = missing_by_lang[lang]
            lines.append(f"### {lang}")
            lines.append(f"- 缺失数量: {len(missing)}")
            for k in missing[:120]:
                lines.append(f"  - {k}")
            if len(missing) > 120:
                lines.append(f"  - ...(仅展示前120项，剩余 {len(missing)-120} 项)")
            lines.append("")

        lines.append("## 3) 非基准语言存在但基准语言缺失的 key（需要补齐英文或清理）")
        for lang in sorted(extra_by_lang.keys()):
            if lang == baseline_lang:
                continue
            extra = extra_by_lang[lang]
            if not extra:
                continue
            lines.append(f"### {lang}")
            lines.append(f"- 多出数量: {len(extra)}")
            for k in extra[:120]:
                lines.append(f"  - {k}")
            if len(extra) > 120:
                lines.append(f"  - ...(仅展示前120项，剩余 {len(extra)-120} 项)")
            lines.append("")

    lines.append("## 4) 占位符/格式不一致（String(format:) 风险）")
    lines.append(f"- 不一致条目: {len(placeholder_mismatches)}")
    for key, ref_lang, ref, lang, ph in placeholder_mismatches[:200]:
        lines.append(f"- {key}: {ref_lang}={ref} vs {lang}={ph}")
    if len(placeholder_mismatches) > 200:
        lines.append(f"- ...(仅展示前200项，剩余 {len(placeholder_mismatches)-200} 项)")
    lines.append("")

    lines.append("## 4.1) 已知拼写错误 key（urlsourses.*）")
    total_typos = sum(len(v) for v in typo_urlsourses.values())
    lines.append(f"- 条目: {total_typos}")
    for lang in sorted(typo_urlsourses.keys()):
        keys = typo_urlsourses[lang]
        lines.append(f"### {lang}")
        lines.append(f"- 数量: {len(keys)}")
        for k in keys[:80]:
            lines.append(f"  - {k}")
        if len(keys) > 80:
            lines.append(f"  - ...(仅展示前80项，剩余 {len(keys)-80} 项)")
        lines.append("")

    lines.append("## 5) .strings 中重复 key（值不一致）")
    lines.append(f"- 条目: {len(duplicates)}")
    for path, k, a, b in duplicates[:120]:
        lines.append(f"- {path}: {k}")
        lines.append(f"  - A: {a}")
        lines.append(f"  - B: {b}")
    if len(duplicates) > 120:
        lines.append(f"- ...(仅展示前120项，剩余 {len(duplicates)-120} 项)")
    lines.append("")

    lines.append("## 6) Swift UI 中疑似硬编码文案（未走本地化）")
    lines.append(f"- 命中数量(去除符号/纯数字等): {len(hardcoded)}")
    if hardcoded_count_by_file:
        lines.append("- Top 文件(按命中数):")
        for path, cnt in sorted(hardcoded_count_by_file.items(), key=lambda kv: (-kv[1], kv[0]))[:12]:
            lines.append(f"  - {path}: {cnt}")
    for f in hardcoded[:220]:
        lines.append(f"- {f.path}: \"{f.value}\"")
    if len(hardcoded) > 220:
        lines.append(f"- ...(仅展示前220项，剩余 {len(hardcoded)-220} 项)")
    lines.append("")

    lines.append("## 7) 可疑未完成翻译（TODO/TBD/FIXME）")
    lines.append(f"- 条目: {len(suspicious)}")
    for lang, k, v in suspicious[:120]:
        lines.append(f"- {lang} {k} = {v}")

    lines.append("")
    lines.append("## 8) 非基准语言中与基准语言值完全相同（疑似未翻译/回填英文）")
    if not identical_to_baseline:
        lines.append("- 未发现")
    else:
        for lang in sorted(identical_to_baseline.keys()):
            keys = identical_to_baseline[lang]
            lines.append(f"### {lang}")
            lines.append(f"- 数量: {len(keys)}")
            for k in keys[:120]:
                lines.append(f"  - {k}")
            if len(keys) > 120:
                lines.append(f"  - ...(仅展示前120项，剩余 {len(keys)-120} 项)")
            lines.append("")

    out_path.write_text("\n".join(lines), encoding="utf-8")

    print("Wrote", out_path)
    print(
        {
            "strings_files": len(strings_files),
            "swift_files": len(swift_files),
            "used_keys": len(used_keys),
            "baseline_keys": len(baseline),
            "missing_in_baseline": len(missing_in_baseline),
            "missing_defined_in_baseline": len(missing_defined_in_baseline),
            "placeholder_mismatches": len(placeholder_mismatches),
            "hardcoded_ui_strings": len(hardcoded),
            "duplicates": len(duplicates),
        }
    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
