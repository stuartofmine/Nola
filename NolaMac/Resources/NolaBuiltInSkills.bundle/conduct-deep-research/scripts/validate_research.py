#!/usr/bin/env python3
"""Validate a saved Markdown research report without third-party packages."""

import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlparse


def validate(path):
    text = path.read_text(encoding="utf-8")
    headings = re.findall(r"(?m)^#{1,6}\s+\S.+$", text)
    urls = re.findall(r"https?://[^\s)>\]}]+", text)
    hosts = sorted(
        {
            urlparse(url.rstrip(".,;:")).hostname
            for url in urls
            if urlparse(url.rstrip(".,;:")).hostname
        }
    )
    access_markers = re.findall(
        r"(?i)(access(?:ed| date)?|retrieved|查阅|访问日期|检索日期)\s*[:：]?\s*\d{4}[-/.年]\d{1,2}",
        text,
    )
    failures = []
    if len(text.strip()) < 800:
        failures.append("report is too short for a deep research artifact")
    if len(headings) < 3:
        failures.append("report needs at least three Markdown headings")
    if len(urls) < 3:
        failures.append("report needs at least three inline source URLs")
    if not access_markers:
        failures.append("report needs at least one explicit access date")
    if failures:
        raise ValueError("; ".join(failures))
    return {
        "structure_validated": True,
        "evidence_verified": False,
        "remaining_checks": ["claim_to_source_support", "source_independence", "scope_coverage", "freshness"],
        "bytes": path.stat().st_size,
        "headings": len(headings),
        "urls": len(urls),
        "distinct_hosts": len(hosts),
        "hosts": hosts,
        "access_date_markers": len(access_markers),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    args = parser.parse_args()
    path = Path(args.input)
    if not path.is_file():
        parser.error(f"report does not exist: {path}")
    print(json.dumps(validate(path), ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
