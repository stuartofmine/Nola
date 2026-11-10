#!/usr/bin/env python3
"""Fill audited macOS localization gaps through the configured Nola model.

The script treats localization coverage as the source of truth, validates every
format placeholder, persists a reusable translation memory, and updates the
catalogs only after a complete batch has passed validation.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
AUDIT = ROOT / "scripts" / "localization_coverage.py"
DEFAULT_BINARY = ROOT / "apps/cli" / "target" / "debug" / "nola"
MEMORY_PATH = ROOT / "scripts" / "mac_translation_memory.json"
CATALOGS = tuple((ROOT / "apps/macos").glob("*.xcstrings"))
MISSING_RE = re.compile(r"^- mac: (.*): missing ([A-Za-z-]+)$")
PLACEHOLDER_RE = re.compile(r"%(?:\d+\$)?(?:lld|ld|zu|@|d|i|u|f|s|c|%)")
LANGUAGE_NAMES = {
    "de": "German",
    "es": "Spanish",
    "fr": "French",
    "ja": "Japanese",
    "ko": "Korean",
    "ru": "Russian",
    "zh-Hans": "Simplified Chinese",
    "zh-Hant": "Traditional Chinese",
}


def audited_gaps() -> dict[str, list[str]]:
    result = subprocess.run(
        [sys.executable, str(AUDIT), "--platform", "mac", "--max-errors", "100000"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    gaps: dict[str, list[str]] = {}
    for line in result.stdout.splitlines():
        match = MISSING_RE.match(line.strip())
        if match:
            key, locale = match.groups()
            gaps.setdefault(locale, []).append(key)
    return gaps


def load_catalogs() -> tuple[dict[Path, dict], dict[str, list[tuple[Path, dict]]]]:
    documents: dict[Path, dict] = {}
    entries: dict[str, list[tuple[Path, dict]]] = {}
    for path in CATALOGS:
        document = json.loads(path.read_text(encoding="utf-8"))
        documents[path] = document
        for key, entry in document.get("strings", {}).items():
            entries.setdefault(key, []).append((path, entry))
    return documents, entries


def source_value(key: str, entry: dict) -> str:
    unit = entry.get("localizations", {}).get("en", {}).get("stringUnit", {})
    return unit.get("value") or key


def placeholders(value: str) -> list[str]:
    return sorted(PLACEHOLDER_RE.findall(value))


def parse_model_json(raw: str) -> dict[str, str]:
    part_order: list[str] = []
    parts: dict[str, str] = {}
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") != "core_part_updated":
            continue
        part = event.get("part") if isinstance(event.get("part"), dict) else {}
        if part.get("kind") != "text":
            continue
        part_id = str(event.get("part_id") or part.get("id") or len(part_order))
        if part_id not in parts:
            part_order.append(part_id)
        parts[part_id] = str(part.get("text", ""))
    content = "".join(parts[part_id] for part_id in part_order).strip()
    if content.startswith("```"):
        content = re.sub(r"^```(?:json)?\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    start, end = content.find("{"), content.rfind("}")
    if start < 0 or end < start:
        raise ValueError("model did not return a JSON object")
    value = json.loads(content[start : end + 1])
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(item, str) for key, item in value.items()
    ):
        raise ValueError("model response must be a string-to-string JSON object")
    return value


def translate_batch(binary: Path, locale: str, values: dict[str, str]) -> dict[str, str]:
    language = LANGUAGE_NAMES.get(locale, locale)
    token_to_key = {f"k{index:04d}": key for index, key in enumerate(values)}
    model_values = {token: values[key] for token, key in token_to_key.items()}
    prompt = f"""Translate the following macOS interface strings from English to {language}.
Return exactly one JSON object with the same keys and translated string values.
Preserve product names, paths, Markdown, ellipses, and every printf placeholder exactly.
Use concise, natural native UI language. Do not add commentary or code fences.

{json.dumps(model_values, ensure_ascii=False, indent=2)}"""
    failure = ""
    translated: dict[str, str] | None = None
    for attempt in range(1, 5):
        result = subprocess.run(
            [str(binary), "--no-tools", "--json", "--no-save", prompt],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=180,
        )
        try:
            if result.returncode != 0:
                failure = result.stderr.strip() or result.stdout[-1200:].strip()
                raise RuntimeError(failure or f"Nola exited with {result.returncode}")
            translated = parse_model_json(result.stdout)
            break
        except (RuntimeError, ValueError, json.JSONDecodeError) as error:
            failure = str(error)
            if attempt < 4:
                time.sleep(attempt * 2)
    if translated is None:
        raise RuntimeError(f"translation batch failed after 4 attempts: {failure}")
    if set(translated) != set(token_to_key):
        missing = sorted(set(token_to_key) - set(translated))
        extra = sorted(set(translated) - set(token_to_key))
        raise ValueError(f"key mismatch; missing={missing}, extra={extra}")
    result: dict[str, str] = {}
    for token, key in token_to_key.items():
        source = values[key]
        target = translated[token].strip()
        if not target:
            raise ValueError(f"empty translation for {key}")
        if placeholders(source) != placeholders(target):
            raise ValueError(
                f"placeholder mismatch for {key}: {placeholders(source)} != {placeholders(target)}"
            )
        result[key] = target
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--batch-size", type=int, default=30)
    parser.add_argument("--locale", action="append", dest="locales")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error(f"Nola binary not found: {args.binary}")

    gaps = audited_gaps()
    documents, entries = load_catalogs()
    memory = json.loads(MEMORY_PATH.read_text(encoding="utf-8")) if MEMORY_PATH.exists() else {}
    requested = set(args.locales or gaps)
    completed = 0

    for locale in sorted(requested):
        keys = sorted(set(gaps.get(locale, [])))
        if not keys:
            continue
        locale_memory = memory.setdefault(locale, {})
        for offset in range(0, len(keys), max(1, args.batch_size)):
            batch_keys = keys[offset : offset + max(1, args.batch_size)]
            sources: dict[str, str] = {}
            for key in batch_keys:
                candidates = entries.get(key)
                if not candidates:
                    raise KeyError(f"catalog entry not found: {key}")
                sources[key] = source_value(key, candidates[0][1])
            pending = {key: value for key, value in sources.items() if key not in locale_memory}
            if pending:
                translated = translate_batch(args.binary, locale, pending)
                locale_memory.update(translated)
                MEMORY_PATH.write_text(
                    json.dumps(memory, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8",
                )
            for key in batch_keys:
                target = locale_memory[key]
                for _, entry in entries[key]:
                    entry.setdefault("localizations", {})[locale] = {
                        "stringUnit": {"state": "translated", "value": target}
                    }
                completed += 1
            if not args.dry_run:
                for path, document in documents.items():
                    path.write_text(
                        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                        encoding="utf-8",
                    )
            print(f"{locale}: {min(offset + len(batch_keys), len(keys))}/{len(keys)}", flush=True)

    print(f"Applied {completed} audited localization entries.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
