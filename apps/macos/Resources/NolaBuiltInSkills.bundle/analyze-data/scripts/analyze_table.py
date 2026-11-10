#!/usr/bin/env python3
"""Generate and validate a deterministic Markdown profile for CSV/TSV data."""

from __future__ import annotations

import argparse
import csv
from decimal import Decimal, InvalidOperation
import json
import math
from pathlib import Path
import re


PAYLOAD_PATTERN = re.compile(
    r"<!-- nola-analysis-payload\n(\{.*?\})\n-->", re.DOTALL
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--group-column")
    parser.add_argument("--numeric-columns", help="Comma-separated numeric columns")
    parser.add_argument("--validate-only", action="store_true")
    return parser.parse_args()


def delimiter_for(path: Path) -> str:
    return "\t" if path.suffix.lower() == ".tsv" else ","


def load_rows(path: Path) -> tuple[list[str], list[dict[str, str]]]:
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle, delimiter=delimiter_for(path))
        fields = [field.strip() for field in (reader.fieldnames or [])]
        rows = [
            {field: (row.get(field) or "").strip() for field in fields}
            for row in reader
        ]
    if not fields:
        raise ValueError("dataset has no header")
    if not rows:
        raise ValueError("dataset has no data rows")
    return fields, rows


def decimal_value(value: str) -> Decimal:
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        raise ValueError(f"not numeric: {value!r}") from error
    if not math.isfinite(float(number)):
        raise ValueError(f"not finite: {value!r}")
    return number


def infer_numeric_columns(fields: list[str], rows: list[dict[str, str]]) -> list[str]:
    result = []
    for field in fields:
        values = [row[field] for row in rows if row[field] != ""]
        if not values:
            continue
        try:
            for value in values:
                decimal_value(value)
        except ValueError:
            continue
        result.append(field)
    return result


def select_columns(
    fields: list[str], rows: list[dict[str, str]], group: str | None, numeric: str | None
) -> tuple[str, list[str]]:
    inferred = infer_numeric_columns(fields, rows)
    numeric_columns = (
        [item.strip() for item in numeric.split(",") if item.strip()]
        if numeric
        else inferred
    )
    unknown = [field for field in numeric_columns if field not in fields]
    if unknown:
        raise ValueError("unknown numeric columns: " + ", ".join(unknown))
    if not numeric_columns:
        raise ValueError("dataset has no consistently numeric columns")
    group_column = group or next(
        (field for field in fields if field not in inferred), fields[0]
    )
    if group_column not in fields:
        raise ValueError(f"unknown group column: {group_column}")
    numeric_columns = [field for field in numeric_columns if field != group_column]
    if not numeric_columns:
        raise ValueError("group column cannot be the only numeric column")
    return group_column, numeric_columns


def format_decimal(value: Decimal) -> str:
    normalized = value.quantize(Decimal("0.01"))
    return format(normalized, "f")


def analyze(
    fields: list[str],
    rows: list[dict[str, str]],
    group_column: str,
    numeric_columns: list[str],
) -> dict:
    missing = {field: sum(not row[field] for row in rows) for field in fields}
    groups: dict[str, dict] = {}
    totals = {field: Decimal("0") for field in numeric_columns}
    minima: dict[str, Decimal | None] = {field: None for field in numeric_columns}
    maxima: dict[str, Decimal | None] = {field: None for field in numeric_columns}
    for row_number, row in enumerate(rows, start=2):
        group = row[group_column] or "(missing)"
        bucket = groups.setdefault(
            group,
            {"rows": 0, "totals": {field: Decimal("0") for field in numeric_columns}},
        )
        bucket["rows"] += 1
        for field in numeric_columns:
            if row[field] == "":
                continue
            try:
                value = decimal_value(row[field])
            except ValueError as error:
                raise ValueError(f"CSV row {row_number}, column {field}: {error}") from error
            totals[field] += value
            bucket["totals"][field] += value
            minima[field] = value if minima[field] is None else min(minima[field], value)
            maxima[field] = value if maxima[field] is None else max(maxima[field], value)
    return {
        "schema": fields,
        "rows": len(rows),
        "group_column": group_column,
        "numeric_columns": numeric_columns,
        "missing": missing,
        "totals": {field: format_decimal(value) for field, value in totals.items()},
        "minimums": {field: format_decimal(value or Decimal("0")) for field, value in minima.items()},
        "maximums": {field: format_decimal(value or Decimal("0")) for field, value in maxima.items()},
        "groups": [
            {
                "group": group,
                "rows": bucket["rows"],
                "totals": {
                    field: format_decimal(value)
                    for field, value in bucket["totals"].items()
                },
            }
            for group, bucket in sorted(groups.items())
        ],
    }


def render(result: dict, source: Path) -> str:
    numeric = result["numeric_columns"]
    lines = [
        "# Data analysis",
        "",
        f"Source: `{source}`",
        "",
        "## Source profile",
        "",
        f"- Rows: **{result['rows']}**",
        f"- Columns: {', '.join(result['schema'])}",
        f"- Grouping: `{result['group_column']}`",
        f"- Numeric measures: {', '.join(numeric)}",
        f"- Missing cells: **{sum(result['missing'].values())}**",
        "",
        "## Reconciled totals",
        "",
        "| Measure | Total | Minimum | Maximum |",
        "|---|---:|---:|---:|",
    ]
    for field in numeric:
        lines.append(
            f"| {field} | {result['totals'][field]} | "
            f"{result['minimums'][field]} | {result['maximums'][field]} |"
        )
    lines.extend(
        [
            "",
            f"## By {result['group_column']}",
            "",
            "| Group | Rows | " + " | ".join(numeric) + " |",
            "|---|---:|" + "---:|" * len(numeric),
        ]
    )
    for group in result["groups"]:
        lines.append(
            f"| {group['group']} | {group['rows']} | "
            + " | ".join(group["totals"][field] for field in numeric)
            + " |"
        )
    lines.extend(
        [
            "",
            "## Findings",
            "",
            "All displayed totals were computed from the source rows and reconcile to the grouped totals above.",
            "",
            "## Limitations",
            "",
            "- This is a descriptive profile; it does not establish causality or statistical significance.",
            "- Units, currency, collection method, and source quality are not inferred when absent from the input.",
            "- Missing values are counted and excluded from numeric sums; they are not imputed.",
            "",
            "<!-- nola-analysis-payload",
            json.dumps(result, ensure_ascii=False, sort_keys=True),
            "-->",
            "",
        ]
    )
    return "\n".join(lines)


def validate_report(output: Path, expected: dict) -> dict:
    text = output.read_text(encoding="utf-8")
    match = PAYLOAD_PATTERN.search(text)
    if not match:
        raise ValueError("report is missing its analysis payload")
    actual = json.loads(match.group(1))
    if actual != expected:
        raise ValueError("report payload does not reconcile to the current source")
    for heading in ("## Reconciled totals", "## Findings", "## Limitations"):
        if heading not in text:
            raise ValueError(f"report is missing {heading}")
    group_rows = sum(group["rows"] for group in actual["groups"])
    if group_rows != actual["rows"]:
        raise ValueError("group row counts do not reconcile")
    for field in actual["numeric_columns"]:
        grouped = sum(Decimal(group["totals"][field]) for group in actual["groups"])
        if grouped != Decimal(actual["totals"][field]):
            raise ValueError(f"group totals do not reconcile for {field}")
    return {
        "validated": True,
        "rows": actual["rows"],
        "groups": len(actual["groups"]),
        "totals": actual["totals"],
        "bytes": output.stat().st_size,
    }


def main() -> None:
    args = parse_args()
    fields, rows = load_rows(args.input)
    group, numeric = select_columns(
        fields, rows, args.group_column, args.numeric_columns
    )
    result = analyze(fields, rows, group, numeric)
    if not args.validate_only:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(render(result, args.input), encoding="utf-8")
    print(json.dumps(validate_report(args.output, result), sort_keys=True))


if __name__ == "__main__":
    main()
