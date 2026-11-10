#!/usr/bin/env python3
"""Create and validate a reproducible, self-contained HTML analysis report."""

from __future__ import annotations

import argparse
import csv
import html
import json
import math
import re
from collections import defaultdict
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--group-column", default="cohort")
    parser.add_argument("--conversion-column", default="converted")
    parser.add_argument("--metric-column", default="revenue")
    parser.add_argument("--validate-only", action="store_true")
    return parser.parse_args()


def finite_number(value: str, label: str) -> float:
    try:
        number = float(value)
    except ValueError as error:
        raise ValueError(f"{label} must be numeric: {value!r}") from error
    if not math.isfinite(number):
        raise ValueError(f"{label} must be finite: {value!r}")
    return number


def load_rows(path: Path, required: tuple[str, ...]) -> tuple[list[dict[str, str]], list[str]]:
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        fields = reader.fieldnames or []
        missing = [name for name in required if name not in fields]
        if missing:
            raise ValueError("missing required columns: " + ", ".join(missing))
        rows = list(reader)
    if not rows:
        raise ValueError("dataset contains no data rows")
    return rows, fields


def analyze(rows: list[dict[str, str]], fields: list[str], group: str, conversion: str, metric: str) -> dict:
    groups: dict[str, dict[str, float]] = defaultdict(lambda: {"rows": 0, "converted": 0, "metric": 0.0})
    missing = {field: 0 for field in fields}
    for index, row in enumerate(rows, start=2):
        for field in fields:
            if not (row.get(field) or "").strip():
                missing[field] += 1
        key = (row.get(group) or "").strip()
        if not key:
            raise ValueError(f"{group} is blank at CSV row {index}")
        converted = finite_number((row.get(conversion) or "").strip(), f"{conversion} row {index}")
        if converted not in (0.0, 1.0):
            raise ValueError(f"{conversion} must contain only 0 or 1 at CSV row {index}")
        value = finite_number((row.get(metric) or "").strip(), f"{metric} row {index}")
        groups[key]["rows"] += 1
        groups[key]["converted"] += converted
        groups[key]["metric"] += value

    group_rows = []
    for name in sorted(groups):
        item = groups[name]
        count = int(item["rows"])
        group_rows.append({
            "group": name,
            "rows": count,
            "converted": int(item["converted"]),
            "conversion_rate": item["converted"] / count,
            "metric_total": item["metric"],
            "metric_mean": item["metric"] / count,
        })
    total_rows = len(rows)
    total_converted = sum(item["converted"] for item in group_rows)
    total_metric = sum(item["metric_total"] for item in group_rows)
    return {
        "schema": fields,
        "rows": total_rows,
        "groups": group_rows,
        "total_converted": total_converted,
        "conversion_rate": total_converted / total_rows,
        "metric_total": total_metric,
        "metric_mean": total_metric / total_rows,
        "missing": missing,
    }


def render_report(result: dict, source: Path, group: str, conversion: str, metric: str) -> str:
    maximum_rate = max(item["conversion_rate"] for item in result["groups"]) or 1
    rows = []
    bars = []
    palette = ["#1677d2", "#18a27b", "#d97832", "#6c65c8"]
    for index, item in enumerate(result["groups"]):
        color = palette[index % len(palette)]
        rate = item["conversion_rate"]
        rows.append(
            "<tr>"
            f"<td><span class='dot' style='background:{color}'></span>{html.escape(item['group'])}</td>"
            f"<td>{item['rows']}</td><td>{item['converted']}</td><td>{rate:.1%}</td>"
            f"<td>{item['metric_total']:,.2f}</td><td>{item['metric_mean']:,.2f}</td>"
            "</tr>"
        )
        width = 520 * rate / maximum_rate
        y = 34 + index * 58
        bars.append(
            f"<text x='0' y='{y + 17}'>{html.escape(item['group'])}</text>"
            f"<rect x='84' y='{y}' width='{width:.1f}' height='28' rx='7' fill='{color}'/>"
            f"<text class='value' x='{min(594, 96 + width):.1f}' y='{y + 19}'>{rate:.1%}</text>"
        )
    missing_total = sum(result["missing"].values())
    payload = {
        "source": str(source),
        "group_column": group,
        "conversion_column": conversion,
        "metric_column": metric,
        **result,
    }
    payload_json = json.dumps(payload, ensure_ascii=False, sort_keys=True).replace("</", "<\\/")
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Experiment analysis</title><style>
:root{{--ink:#172331;--muted:#637083;--line:#dce4ec;--blue:#1677d2;--paper:#fff;--wash:#f4f8fb}}
*{{box-sizing:border-box}}body{{margin:0;background:#eaf0f5;color:var(--ink);font:15px -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
main{{max-width:980px;margin:40px auto;background:var(--paper);box-shadow:0 18px 60px #26394d20;border-radius:22px;overflow:hidden}}
header{{padding:42px 48px 34px;background:linear-gradient(135deg,#f9fcff,#eef7ff)}}
.eyebrow{{color:var(--blue);font-size:12px;font-weight:700;letter-spacing:.16em;text-transform:uppercase}}h1{{font-size:36px;letter-spacing:-.04em;margin:10px 0 9px}}p{{line-height:1.62}}.lede{{color:var(--muted);font-size:17px;margin:0;max-width:700px}}
.kpis{{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;padding:0 48px;transform:translateY(-18px)}}.kpi{{background:#fff;border:1px solid var(--line);border-radius:14px;padding:17px;box-shadow:0 8px 24px #26394d0b}}.kpi b{{display:block;font-size:25px;letter-spacing:-.03em}}.kpi span{{color:var(--muted);font-size:12px}}
section{{padding:24px 48px 34px}}h2{{font-size:21px;margin:0 0 16px}}table{{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}}th,td{{padding:13px 12px;border-bottom:1px solid var(--line);text-align:right}}th:first-child,td:first-child{{text-align:left}}th{{color:var(--muted);font-size:11px;text-transform:uppercase;letter-spacing:.08em}}.dot{{display:inline-block;width:9px;height:9px;border-radius:50%;margin-right:9px}}
.chart{{background:var(--wash);border-radius:16px;padding:18px}}svg{{display:block;width:100%;height:auto}}svg text{{fill:var(--ink);font-size:14px}}svg .value{{font-weight:700}}.contract{{display:grid;grid-template-columns:1fr 1fr;gap:16px}}.note{{border:1px solid var(--line);border-radius:14px;padding:17px}}.note strong{{display:block;margin-bottom:7px}}.note p{{margin:0;color:var(--muted)}}footer{{padding:24px 48px 35px;color:var(--muted);font-size:12px;border-top:1px solid var(--line)}}
@media(max-width:720px){{main{{margin:0;border-radius:0}}header,section,footer{{padding-left:22px;padding-right:22px}}.kpis{{grid-template-columns:1fr 1fr;padding:0 22px}}.contract{{grid-template-columns:1fr}}}}
</style></head><body><main><header><div class="eyebrow">Reproducible analysis</div><h1>Experiment analysis</h1><p class="lede">A computed view of conversion and {html.escape(metric)} by {html.escape(group)}, with row-count reconciliation and explicit causal limits.</p></header>
<div class="kpis"><div class="kpi"><b>{result['rows']}</b><span>source rows</span></div><div class="kpi"><b>{result['conversion_rate']:.1%}</b><span>overall conversion</span></div><div class="kpi"><b>{result['metric_total']:,.2f}</b><span>{html.escape(metric)} total</span></div><div class="kpi"><b>{missing_total}</b><span>missing cells</span></div></div>
<section><h2>Group reconciliation</h2><table><thead><tr><th>{html.escape(group)}</th><th>Rows</th><th>Converted</th><th>Rate</th><th>Total</th><th>Mean / row</th></tr></thead><tbody>{''.join(rows)}</tbody></table></section>
<section><h2>Conversion rate</h2><div class="chart"><svg viewBox="0 0 700 {68 + 58 * len(result['groups'])}" role="img" aria-label="Conversion rate by group">{''.join(bars)}</svg></div></section>
<section><h2>Analysis contract</h2><div class="contract"><div class="note"><strong>Question and method</strong><p>Describe observed {html.escape(conversion)} and {html.escape(metric)} by {html.escape(group)}. Calculations use all parsed rows; no imputation or causal model was applied.</p></div><div class="note"><strong>Quality and limits</strong><p>Row-count reconciliation: {result['rows']} of {result['rows']}. Missing cells: {missing_total}. This observational extract does not support a causal claim.</p></div></div></section>
<footer>Source: {html.escape(str(source))} · Generated locally · Values are reproducible from the embedded analysis payload.</footer>
<script id="analysis-data" type="application/json">{payload_json}</script></main></body></html>"""


def validate(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    required = ["<!doctype html>", "Analysis contract", "Row-count reconciliation:", "does not support a causal claim", "<svg", "id=\"analysis-data\""]
    missing = [value for value in required if value not in text]
    match = re.search(r'<script id="analysis-data" type="application/json">(.*?)</script>', text, re.S)
    if missing or not match:
        raise ValueError("invalid analysis report: " + ", ".join(missing or ["embedded payload missing"]))
    payload = json.loads(match.group(1).replace("<\\/", "</"))
    if payload["rows"] != sum(item["rows"] for item in payload["groups"]):
        raise ValueError("group row counts do not reconcile")
    if abs(payload["metric_total"] - sum(item["metric_total"] for item in payload["groups"])) > 1e-9:
        raise ValueError("group metric totals do not reconcile")
    return {"bytes": path.stat().st_size, "groups": len(payload["groups"]), "rows": payload["rows"], "validated": True}


def main() -> None:
    args = parse_args()
    if not args.validate_only:
        if args.input is None:
            raise ValueError("--input is required unless --validate-only is used")
        required = (args.group_column, args.conversion_column, args.metric_column)
        rows, fields = load_rows(args.input, required)
        result = analyze(rows, fields, *required)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(render_report(result, args.input, *required), encoding="utf-8")
    print(json.dumps(validate(args.output), separators=(",", ":"), sort_keys=True))


if __name__ == "__main__":
    main()
