#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-model visual file-tool smoke; requires PyMuPDF and Poppler."""
import argparse
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import time


def final_journal_answers(database):
    answers = []
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
        for owner, part_id, raw_head in db.execute(
                "SELECT owner, part_id, head_json FROM assistant_journal_parts"):
            head = json.loads(raw_head)
            metadata = head.get("metadata", {})
            if metadata.get("kind") != "text" or not metadata.get("data", {}).get("is_final"):
                continue
            for body in head.get("bodies", []):
                if body["field"] != "markdown":
                    continue
                chunks = db.execute(
                    "SELECT m.chunk_index, c.bytes FROM assistant_journal_manifest_chunks m "
                    "JOIN assistant_journal_chunks c ON c.digest=m.digest "
                    "WHERE m.owner=? AND m.part_id=? AND m.field=? AND m.generation=? "
                    "AND m.chunk_index<? ORDER BY m.chunk_index",
                    (owner, part_id, json.dumps(body["field"]), body["content_generation"],
                     int(body["chunk_count"]))).fetchall()
                if [row[0] for row in chunks] != list(range(int(body["chunk_count"]))):
                    raise ValueError("Incomplete final answer journal")
                raw = b"".join(row[1] for row in chunks)
                if len(raw) != int(body["utf8_bytes"]):
                    raise ValueError("Final answer journal byte count mismatch")
                if raw:
                    answers.append(raw.decode("utf-8"))
    return answers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    import pymupdf
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / "project"
    project.mkdir()
    (project / ".git").mkdir()
    code = str(secrets.randbelow(900000) + 100000)
    doc = pymupdf.open()
    doc.new_page(width=400, height=200).insert_text((30, 75), f"Code: {code}", fontsize=30)
    doc.save(project / "card.pdf")
    source = json.loads(args.config.read_text())
    config = project / "nola.json"
    with open(config, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as handle:
        json.dump({"llm":source["llm"], "allowed_dirs":[str(project)],
            "model_capabilities":{"vision_input":True,"native_pdf_input":False},
            "max_iterations":0}, handle)
    started = time.monotonic()
    try:
        with (root / "events.ndjson").open("x") as log:
            result = subprocess.run([str(args.binary.resolve()), "--project", str(project),
                "--json", "--yes", "--max-iterations", "0"], cwd=project,
                env={**os.environ,"NOLA_DB_PATH":str(root / "state.db"),"NOLA_SKIP_ONBOARDING":"1"},
                input="使用 read_file 的 pdf_page=1 查看 card.pdf，只回复图片里的六位验证码。不要提取 PDF 文本，不要修改文件。",
                text=True, stdout=log, stderr=subprocess.STDOUT, timeout=120)
    finally:
        config.unlink(missing_ok=True)
    tools = []
    answers = []
    for line in (root / "events.ndjson").read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        trace = event.get("trace", {})
        payload = trace.get("payload", {})
        if trace.get("kind") == "tool.completed":
            tools.append({"name":payload.get("tool_name"),"status":trace.get("status"),
                "arguments":payload.get("arguments")})
        if trace.get("kind") == "tool.started" and payload.get("tool_name") == "requirements_update":
            answers.append(payload.get("arguments", {}).get("answer", ""))
    # Final read-only replies live in the canonical journal, not tool arguments.
    answers.extend(final_journal_answers(root / "state.db"))
    report = {"official_benchmark":False,"elapsed_seconds":time.monotonic()-started,
        "exit_code":result.returncode,"expected":code,"answers":answers,"tools":tools,
        "answer_matches":any(answer.strip() == code for answer in answers)}
    (root / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps(report, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
