#!/usr/bin/env python3
"""Fail-closed final-artifact acceptance for Office outputs on macOS."""

import argparse
import html
import json
import plistlib
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path
from xml.etree import ElementTree


PACKAGE_PARTS = {
    "docx": {"[Content_Types].xml", "word/document.xml", "word/styles.xml"},
    "xlsx": {"[Content_Types].xml", "xl/workbook.xml", "xl/worksheets/sheet1.xml"},
    "pptx": {"[Content_Types].xml", "ppt/presentation.xml", "ppt/slides/slide1.xml"},
}
WORD_NAMESPACE = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def package_text(archive):
    values = []
    for name in archive.namelist():
        if not name.endswith(".xml"):
            continue
        try:
            root = ElementTree.fromstring(archive.read(name))
        except ElementTree.ParseError:
            continue
        values.extend(text for text in root.itertext() if text)
    return "\n".join(values)


def validate_package(path, kind, required_text, expected_tables, expected_rows, expected_columns):
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        missing = sorted(PACKAGE_PARTS[kind] - names)
        if missing:
            raise ValueError(f"missing package parts: {', '.join(missing)}")
        text = package_text(archive)
        missing_text = [value for value in required_text if value not in text]
        if missing_text:
            raise ValueError(f"missing required content: {missing_text}")

        result = {
            "kind": kind,
            "package_part_count": len(names),
            "required_text_matched": required_text,
        }
        if kind != "docx":
            return result

        document = ElementTree.fromstring(archive.read("word/document.xml"))
        namespace = {"w": WORD_NAMESPACE}
        tables = document.findall(".//w:tbl", namespace)
        table_rows = [table.findall("./w:tr", namespace) for table in tables]
        table_columns = []
        for rows in table_rows:
            table_columns.append(
                max((len(row.findall("./w:tc", namespace)) for row in rows), default=0)
            )
        result.update({
            "native_table_count": len(tables),
            "table_row_counts": [len(rows) for rows in table_rows],
            "table_column_counts": table_columns,
        })
        if expected_tables is not None and len(tables) != expected_tables:
            raise ValueError(f"expected {expected_tables} native tables, found {len(tables)}")
        if expected_rows is not None and (not table_rows or len(table_rows[0]) != expected_rows):
            actual = len(table_rows[0]) if table_rows else 0
            raise ValueError(f"expected {expected_rows} rows in the first table, found {actual}")
        if expected_columns is not None and (not table_columns or table_columns[0] != expected_columns):
            actual = table_columns[0] if table_columns else 0
            raise ValueError(f"expected {expected_columns} columns in the first table, found {actual}")
        return result


def quick_look_receipt(path, required_text):
    with tempfile.TemporaryDirectory(prefix="nola-final-artifact-") as directory:
        for arguments in (["-o", directory, "-p", str(path)], ["-o", directory, "-t", "-s", "1200", str(path)]):
            result = subprocess.run(
                ["/usr/bin/qlmanage", *arguments],
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
            if result.returncode != 0:
                raise RuntimeError(f"Quick Look failed: {result.stderr.strip()}")
        files = list(Path(directory).rglob("*"))
        preview = next((item for item in files if item.name == "Preview.html"), None)
        thumbnail = next((item for item in files if item.suffix.lower() == ".png"), None)
        if preview is None or thumbnail is None:
            raise RuntimeError("Quick Look did not produce both Preview.html and a PNG thumbnail")
        preview_text = preview.read_text(encoding="utf-8", errors="replace")
        decoded_preview = html.unescape(preview_text)
        missing = [value for value in required_text if value not in decoded_preview]
        if missing:
            raise ValueError(f"Quick Look preview is missing required content: {missing}")
        thumbnail_bytes = thumbnail.stat().st_size
        if thumbnail_bytes < 10_000:
            raise ValueError(f"Quick Look thumbnail is unexpectedly small: {thumbnail_bytes} bytes")
        return {
            "renderer": "macOS Quick Look",
            "preview_html_bytes": len(preview_text.encode("utf-8")),
            "thumbnail_bytes": thumbnail_bytes,
            "required_text_matched": required_text,
        }


def application_version(path):
    info_path = path / "Contents" / "Info.plist"
    try:
        with info_path.open("rb") as handle:
            info = plistlib.load(handle)
        return str(info.get("CFBundleShortVersionString") or info.get("CFBundleVersion") or "")
    except (OSError, plistlib.InvalidFileException):
        return ""


def microsoft_word_receipt(path, expected_tables, expected_rows, expected_columns):
    application = Path("/Applications/Microsoft Word.app")
    if not application.exists():
        raise RuntimeError("Microsoft Word is not installed")
    # Word can keep an already-open document's pre-overwrite contents in memory.
    # Open a uniquely named copy so this gate always checks the current bytes.
    with tempfile.TemporaryDirectory(prefix="nola-word-acceptance-") as directory:
        validation_path = Path(directory) / "artifact-under-test.docx"
        shutil.copy2(path, validation_path)
        escaped_path = str(validation_path).replace("\\", "\\\\").replace('"', '\\"')
        script = f'''
set targetAlias to POSIX file "{escaped_path}" as alias
set targetHFS to targetAlias as text
tell application "Microsoft Word"
    open targetAlias read only true add to recent files false
    set tableCount to -1
    set rowCount to -1
    set columnCount to -1
    set documentCount to count documents
    repeat with documentIndex from 1 to documentCount
        if (full name of document documentIndex) is targetHFS then
            set tableCount to count of tables of document documentIndex
            if tableCount > 0 then
                set rowCount to count of rows of table 1 of document documentIndex
                set columnCount to count of columns of table 1 of document documentIndex
            end if
        end if
    end repeat
    return (tableCount as text) & "," & (rowCount as text) & "," & (columnCount as text)
end tell
'''
        result = subprocess.run(
            ["/usr/bin/osascript", "-e", script],
            capture_output=True,
            text=True,
            timeout=45,
            check=False,
        )
    if result.returncode != 0:
        raise RuntimeError(f"Microsoft Word rejected the document: {result.stderr.strip()}")
    try:
        table_count, row_count, column_count = [int(value) for value in result.stdout.strip().split(",")]
    except ValueError as error:
        raise RuntimeError(f"unexpected Microsoft Word receipt: {result.stdout.strip()!r}") from error
    if expected_tables is not None and table_count != expected_tables:
        raise ValueError(f"Microsoft Word found {table_count} tables; expected {expected_tables}")
    if expected_rows is not None and row_count != expected_rows:
        raise ValueError(f"Microsoft Word found {row_count} rows; expected {expected_rows}")
    if expected_columns is not None and column_count != expected_columns:
        raise ValueError(f"Microsoft Word found {column_count} columns; expected {expected_columns}")
    return {
        "application": "Microsoft Word",
        "version": application_version(application),
        "opened_read_only": True,
        "native_table_count": table_count,
        "first_table_row_count": row_count,
        "first_table_column_count": column_count,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--kind", choices=["docx", "xlsx", "pptx"])
    parser.add_argument("--contains", action="append", default=[])
    parser.add_argument("--contains-json", default="[]")
    parser.add_argument("--expected-tables", type=int)
    parser.add_argument("--expected-rows", type=int)
    parser.add_argument("--expected-columns", type=int)
    parser.add_argument("--quick-look", action="store_true")
    parser.add_argument("--microsoft-word", action="store_true")
    parser.add_argument("--report", type=Path)
    arguments = parser.parse_args()

    try:
        json_required_text = json.loads(arguments.contains_json)
    except json.JSONDecodeError as error:
        parser.error(f"--contains-json must be a JSON array: {error}")
    if not isinstance(json_required_text, list) or not all(
        isinstance(value, str) for value in json_required_text
    ):
        parser.error("--contains-json must be a JSON array of strings")
    required_text = arguments.contains + json_required_text

    path = arguments.artifact.resolve()
    kind = arguments.kind or path.suffix.lower().lstrip(".")
    report = {
        "success": False,
        "artifact": str(path),
        "exists": path.is_file(),
        "bytes": path.stat().st_size if path.is_file() else 0,
        "package": {},
        "quick_look": {},
        "external_system": {},
        "error": "",
    }
    try:
        if kind not in PACKAGE_PARTS:
            raise ValueError(f"unsupported artifact kind: {kind}")
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError("artifact does not exist or is empty")
        report["package"] = validate_package(
            path,
            kind,
            required_text,
            arguments.expected_tables,
            arguments.expected_rows,
            arguments.expected_columns,
        )
        if arguments.quick_look:
            report["quick_look"] = quick_look_receipt(path, required_text)
        if arguments.microsoft_word:
            if kind != "docx":
                raise ValueError("--microsoft-word requires a DOCX artifact")
            report["external_system"] = microsoft_word_receipt(
                path,
                arguments.expected_tables,
                arguments.expected_rows,
                arguments.expected_columns,
            )
        report["success"] = True
    except (OSError, RuntimeError, ValueError, zipfile.BadZipFile) as error:
        report["error"] = str(error)

    payload = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True)
    print(payload)
    if arguments.report:
        arguments.report.parent.mkdir(parents=True, exist_ok=True)
        arguments.report.write_text(payload + "\n", encoding="utf-8")
    raise SystemExit(0 if report["success"] else 1)


if __name__ == "__main__":
    main()
