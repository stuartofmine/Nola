#!/usr/bin/env python3
"""Create an editable DOCX with native Word tables using only the standard library."""

import argparse
import json
import re
import zipfile
from html.parser import HTMLParser
from pathlib import Path
from xml.sax.saxutils import escape
from xml.etree import ElementTree


WORD_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE_REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
URL_PATTERN = re.compile(r"https?://[^\s<>\"']+")


def strings_in(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for nested in value.values():
            yield from strings_in(nested)
    elif isinstance(value, list):
        for nested in value:
            yield from strings_in(nested)


class DocumentHTMLParser(HTMLParser):
    """Convert the small semantic HTML subset used by the former Word workflow."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.blocks = []
        self.active = None
        self.buffer = []
        self.table_columns = []
        self.table_rows = []
        self.table_row = None
        self.table_cell = None
        self.table_cell_is_header = False
        self.suppressed_depth = 0

    def handle_starttag(self, tag, attrs):
        tag = tag.lower()
        if tag in ("head", "style", "script"):
            self.suppressed_depth += 1
            return
        if self.suppressed_depth:
            return
        if tag in ("h1", "h2", "h3", "p", "pre", "li"):
            self.active = tag
            self.buffer = []
        elif tag == "br" and self.active:
            self.buffer.append("\n")
        elif tag == "tr":
            self.table_row = []
        elif tag in ("th", "td"):
            self.table_cell = []
            self.table_cell_is_header = tag == "th"

    def handle_endtag(self, tag):
        tag = tag.lower()
        if tag in ("head", "style", "script"):
            self.suppressed_depth = max(0, self.suppressed_depth - 1)
            return
        if self.suppressed_depth:
            return
        if tag in ("h1", "h2", "h3", "p", "pre", "li") and self.active == tag:
            text = "".join(self.buffer)
            if tag.startswith("h"):
                self.blocks.append({"type": "heading", "level": int(tag[1]), "text": text.strip()})
            elif tag == "pre":
                self.blocks.append({"type": "code", "text": text.strip("\n")})
            elif tag == "li":
                self.blocks.append({"type": "bullets", "items": [text.strip()]})
            elif text.strip():
                self.blocks.append({"type": "paragraph", "text": text.strip()})
            self.active = None
            self.buffer = []
        elif tag in ("th", "td") and self.table_cell is not None:
            value = "".join(self.table_cell).strip()
            if self.table_row is not None:
                self.table_row.append((value, self.table_cell_is_header))
            self.table_cell = None
        elif tag == "tr" and self.table_row is not None:
            values = [value for value, _ in self.table_row]
            if self.table_row and all(is_header for _, is_header in self.table_row) and not self.table_columns:
                self.table_columns = values
            elif values:
                self.table_rows.append(values)
            self.table_row = None
        elif tag == "table":
            if self.table_columns or self.table_rows:
                columns = self.table_columns
                rows = self.table_rows
                if not columns and rows:
                    columns, rows = rows[0], rows[1:]
                self.blocks.append({"type": "table", "columns": columns, "rows": rows})
            self.table_columns = []
            self.table_rows = []

    def handle_data(self, data):
        if self.suppressed_depth:
            return
        if self.table_cell is not None:
            self.table_cell.append(data)
        elif self.active:
            self.buffer.append(data)


def spec_from_html(path):
    parser = DocumentHTMLParser()
    parser.feed(path.read_text(encoding="utf-8"))
    parser.close()
    return {"title": path.stem, "blocks": parser.blocks}


def text_run(value, *, bold=False, color=None, font=None, size=None):
    text = str(value)
    properties = []
    if bold:
        properties.append("<w:b/>")
    if color:
        properties.append(f'<w:color w:val="{color}"/>')
    if font:
        properties.append(
            f'<w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:eastAsia="{font}"/>'
        )
    if size:
        properties.append(f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>')
    run_properties = f'<w:rPr>{"".join(properties)}</w:rPr>' if properties else ""
    preserve = ' xml:space="preserve"' if text[:1].isspace() or text[-1:].isspace() else ""
    return f"<w:r>{run_properties}<w:t{preserve}>{escape(text)}</w:t></w:r>"


def linked_text_runs(value, relationships, *, bold=False, color=None):
    text = str(value)
    runs = []
    cursor = 0
    for match in URL_PATTERN.finditer(text):
        url = match.group(0).rstrip(".,;:!?)]}，。；：！？）】")
        end = match.start() + len(url)
        if match.start() > cursor:
            runs.append(text_run(text[cursor:match.start()], bold=bold, color=color))
        relationship_id = f"rIdLink{len(relationships) + 1}"
        relationships.append((relationship_id, url))
        runs.append(
            f'<w:hyperlink r:id="{relationship_id}">'
            f'<w:r><w:rPr><w:color w:val="1769E0"/><w:u w:val="single"/>'
            f'{"<w:b/>" if bold else ""}</w:rPr><w:t>{escape(url)}</w:t></w:r>'
            '</w:hyperlink>'
        )
        cursor = end
    if cursor < len(text):
        runs.append(text_run(text[cursor:], bold=bold, color=color))
    return "".join(runs) if runs else text_run(text, bold=bold, color=color)


def paragraph(value, style=None, *, align=None, keep_next=False, code=False, relationships=None):
    properties = []
    if style:
        properties.append(f'<w:pStyle w:val="{style}"/>')
    if align:
        properties.append(f'<w:jc w:val="{align}"/>')
    if keep_next:
        properties.append("<w:keepNext/>")
    if code:
        properties.extend([
            '<w:shd w:val="clear" w:color="auto" w:fill="EEF4FA"/>',
            '<w:spacing w:before="40" w:after="40" w:line="250" w:lineRule="auto"/>',
        ])
    paragraph_properties = f'<w:pPr>{"".join(properties)}</w:pPr>' if properties else ""
    if code:
        lines = str(value).splitlines() or [""]
        runs = []
        for index, line in enumerate(lines):
            if index:
                runs.append("<w:r><w:br/></w:r>")
            runs.append(text_run(line, font="Menlo", size="17"))
        content = "".join(runs)
    else:
        content = linked_text_runs(value, relationships if relationships is not None else [])
    return f"<w:p>{paragraph_properties}{content}</w:p>"


def display_width(value):
    return max(1, len(str(value).encode("utf-8")))


def table_column_widths(columns, rows, total_width=9000):
    column_count = len(columns)
    base_width = min(1200, total_width // column_count)
    weights = []
    for index, column in enumerate(columns):
        values = [column] + [row[index] for row in rows]
        weights.append(max(4, min(32, max(display_width(value) for value in values))))
    remaining = total_width - (base_width * column_count)
    weight_total = sum(weights)
    widths = [base_width + (remaining * weight // weight_total) for weight in weights]
    widths[-1] += total_width - sum(widths)
    return widths


def table_cell(value, relationships, *, header=False, alternate=False, width=3000):
    fill_color = "1769E0" if header else "EEF4FA" if alternate else None
    fill = (
        f'<w:shd w:val="clear" w:color="auto" w:fill="{fill_color}"/>'
        if fill_color
        else ""
    )
    color = "FFFFFF" if header else "18212B"
    return (
        "<w:tc>"
        f'<w:tcPr><w:tcW w:w="{width}" w:type="dxa"/><w:vAlign w:val="center"/>{fill}'
        '<w:tcMar><w:top w:w="90" w:type="dxa"/><w:left w:w="110" w:type="dxa"/>'
        '<w:bottom w:w="90" w:type="dxa"/><w:right w:w="110" w:type="dxa"/></w:tcMar>'
        "</w:tcPr>"
        f'<w:p><w:pPr><w:keepLines/><w:spacing w:before="0" w:after="0" '
        f'w:line="240" w:lineRule="auto"/><w:jc w:val="center"/></w:pPr>'
        f'{linked_text_runs(value, relationships, bold=header, color=color)}</w:p>'
        "</w:tc>"
    )


def table_block(columns, rows, relationships):
    columns = [str(value) for value in columns]
    if not columns:
        raise ValueError("table must contain at least one column")
    if any(len(row) != len(columns) for row in rows):
        raise ValueError("table row width does not match columns")
    widths = table_column_widths(columns, rows)
    grid = "".join(f'<w:gridCol w:w="{width}"/>' for width in widths)
    borders = "".join(
        f'<w:{edge} w:val="single" w:sz="6" w:space="0" w:color="C8D3DF"/>'
        for edge in ["top", "left", "bottom", "right", "insideH", "insideV"]
    )
    header = (
        '<w:tr><w:trPr><w:tblHeader/><w:cantSplit/></w:trPr>'
        + "".join(
            table_cell(value, relationships, header=True, width=width)
            for value, width in zip(columns, widths)
        )
        + "</w:tr>"
    )
    body = []
    for row_index, row in enumerate(rows):
        body.append(
            '<w:tr><w:trPr><w:cantSplit/></w:trPr>'
            + "".join(
                table_cell(value, relationships, alternate=row_index % 2 == 1, width=width)
                for value, width in zip(row, widths)
            )
            + "</w:tr>"
        )
    return (
        "<w:tbl>"
        '<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="9000" w:type="dxa"/>'
        '<w:tblInd w:w="120" w:type="dxa"/>'
        '<w:tblLayout w:type="fixed"/><w:tblLook w:val="04A0" w:firstRow="1" '
        'w:lastRow="0" w:firstColumn="0" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>'
        f"<w:tblBorders>{borders}</w:tblBorders></w:tblPr>"
        f"<w:tblGrid>{grid}</w:tblGrid>{header}{''.join(body)}</w:tbl>"
    )


def document_body(spec):
    blocks = spec.get("blocks") or []
    if not blocks:
        raise ValueError("spec must contain at least one block")
    output = []
    relationships = []
    for index, block in enumerate(blocks, start=1):
        kind = block.get("type")
        if kind == "heading":
            level = int(block.get("level", 1))
            if level not in (1, 2, 3):
                raise ValueError(f"block {index} has unsupported heading level")
            output.append(paragraph(block.get("text", ""), f"Heading{level}", keep_next=True, relationships=relationships))
        elif kind == "paragraph":
            output.append(paragraph(block.get("text", ""), "Normal", relationships=relationships))
        elif kind == "code":
            output.append(paragraph(block.get("text", ""), "Code", code=True))
        elif kind == "table":
            output.append(table_block(block.get("columns") or [], block.get("rows") or [], relationships))
        elif kind == "bullets":
            for item in block.get("items") or []:
                output.append(paragraph(f"•  {item}", "ListParagraph", relationships=relationships))
        elif kind == "page_break":
            output.append('<w:p><w:r><w:br w:type="page"/></w:r></w:p>')
        else:
            raise ValueError(f"block {index} has unsupported type: {kind!r}")
    output.append(
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" '
        'w:header="567" w:footer="567" w:gutter="0"/></w:sectPr>'
    )
    return "".join(output), relationships


def materialize_external_blocks(spec, spec_path):
    """Resolve bounded workspace-local source references before generation."""
    base = spec_path.resolve().parent
    for index, block in enumerate(spec.get("blocks") or [], start=1):
        if block.get("type") != "code_file":
            continue
        relative = Path(str(block.get("path") or ""))
        if not str(relative) or relative.is_absolute():
            raise ValueError(f"block {index} code_file path must be workspace-relative")
        source = (base / relative).resolve()
        try:
            source.relative_to(base)
        except ValueError as error:
            raise ValueError(f"block {index} code_file escapes the specification workspace") from error
        if not source.is_file():
            raise ValueError(f"block {index} code_file does not exist: {relative}")
        block.clear()
        block.update({"type": "code", "text": source.read_text(encoding="utf-8")})
    return spec


def styles_xml():
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="{WORD_NS}">
  <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos" w:eastAsia="PingFang SC"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:color w:val="18212B"/></w:rPr></w:rPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:spacing w:after="100" w:line="285" w:lineRule="auto"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="260" w:after="140"/></w:pPr><w:rPr><w:b/><w:color w:val="1769E0"/><w:sz w:val="36"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="220" w:after="100"/></w:pPr><w:rPr><w:b/><w:color w:val="18212B"/><w:sz w:val="30"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:color w:val="18212B"/><w:sz w:val="25"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/><w:rPr><w:rFonts w:ascii="Menlo" w:hAnsi="Menlo"/><w:sz w:val="18"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="40" w:line="260" w:lineRule="auto"/><w:ind w:left="360" w:hanging="180"/></w:pPr></w:style>
  <w:style w:type="table" w:default="1" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="C8D3DF"/><w:left w:val="single" w:sz="4" w:color="C8D3DF"/><w:bottom w:val="single" w:sz="4" w:color="C8D3DF"/><w:right w:val="single" w:sz="4" w:color="C8D3DF"/><w:insideH w:val="single" w:sz="4" w:color="C8D3DF"/><w:insideV w:val="single" w:sz="4" w:color="C8D3DF"/></w:tblBorders></w:tblPr></w:style>
</w:styles>'''


def validate_document(spec, output):
    required_parts = {
        "[Content_Types].xml",
        "_rels/.rels",
        "word/document.xml",
        "word/styles.xml",
        "word/_rels/document.xml.rels",
    }
    with zipfile.ZipFile(output) as package:
        missing_parts = sorted(required_parts - set(package.namelist()))
        if missing_parts:
            raise ValueError(f"generated DOCX is missing package parts: {missing_parts}")
        root = ElementTree.fromstring(package.read("word/document.xml"))
        relationship_root = ElementTree.fromstring(
            package.read("word/_rels/document.xml.rels")
        )

    namespace = {"w": WORD_NS}
    relationship_namespace = {"r": PACKAGE_REL_NS}
    # OOXML may split one visible paragraph across runs (for example around a
    # hyperlink). Validation compares the continuous user-visible text rather
    # than requiring each source string to live in one XML text node.
    document_text = "".join(value for value in root.itertext() if value)
    expected_tables = [block for block in spec["blocks"] if block.get("type") == "table"]
    actual_tables = root.findall(".//w:tbl", namespace)
    if len(actual_tables) != len(expected_tables):
        raise ValueError("generated DOCX did not preserve every table as a native Word table")

    for index, block in enumerate(spec["blocks"], start=1):
        kind = block.get("type")
        expected_values = []
        if kind in ("heading", "paragraph"):
            expected_values.append(str(block.get("text", "")))
        elif kind == "code":
            expected_values.extend(str(block.get("text", "")).splitlines())
        elif kind == "bullets":
            expected_values.extend(str(value) for value in block.get("items") or [])
        for value in expected_values:
            if value not in document_text:
                raise ValueError(f"generated DOCX is missing content from block {index}: {value!r}")

    table_receipts = []
    for index, (expected, actual) in enumerate(zip(expected_tables, actual_tables), start=1):
        expected_rows = [expected.get("columns") or []] + (expected.get("rows") or [])
        actual_rows = []
        for row in actual.findall("./w:tr", namespace):
            actual_rows.append([
                "".join(cell.itertext())
                for cell in row.findall("./w:tc", namespace)
            ])
        expected_rows = [[str(value) for value in row] for row in expected_rows]
        if actual_rows != expected_rows:
            raise ValueError(f"generated DOCX table {index} cells do not match the specification")
        table_width = actual.find("./w:tblPr/w:tblW", namespace)
        table_indent = actual.find("./w:tblPr/w:tblInd", namespace)
        width_attribute = f"{{{WORD_NS}}}w"
        if table_width is None or int(table_width.attrib.get(width_attribute, "0")) != 9000:
            raise ValueError(f"generated DOCX table {index} has an invalid table width")
        if table_indent is None or int(table_indent.attrib.get(width_attribute, "-1")) != 120:
            raise ValueError(f"generated DOCX table {index} has an invalid table indent")
        grid_widths = [
            int(column.attrib.get(width_attribute, "0"))
            for column in actual.findall("./w:tblGrid/w:gridCol", namespace)
        ]
        expected_widths = table_column_widths(expected_rows[0], expected_rows[1:])
        if grid_widths != expected_widths or sum(grid_widths) != 9000:
            raise ValueError(f"generated DOCX table {index} has inconsistent column widths")
        header = actual.find("./w:tr/w:trPr/w:tblHeader", namespace)
        if header is None:
            raise ValueError(f"generated DOCX table {index} is missing a repeating header")
        if actual.find(".//w:trHeight", namespace) is not None:
            raise ValueError(f"generated DOCX table {index} uses a fixed row height")
        for row in actual.findall("./w:tr", namespace):
            cells = row.findall("./w:tc", namespace)
            for column_index, cell in enumerate(cells):
                cell_width = cell.find("./w:tcPr/w:tcW", namespace)
                if cell_width is None or int(cell_width.attrib.get(width_attribute, "0")) != grid_widths[column_index]:
                    raise ValueError(f"generated DOCX table {index} has inconsistent cell widths")
                if cell.find("./w:tcPr/w:vAlign", namespace) is None:
                    raise ValueError(f"generated DOCX table {index} is missing vertical alignment")
                if cell.find("./w:tcPr/w:tcMar", namespace) is None:
                    raise ValueError(f"generated DOCX table {index} is missing cell margins")
        table_receipts.append({
            "rows": len(actual_rows),
            "columns": len(actual_rows[0]) if actual_rows else 0,
            "column_widths": grid_widths,
        })
    hyperlink_relationships = [
        relationship
        for relationship in relationship_root.findall("./r:Relationship", relationship_namespace)
        if relationship.attrib.get("Type", "").endswith("/hyperlink")
    ]
    relationship_id_attribute = f"{{{REL_NS}}}id"
    linked_relationship_ids = {
        hyperlink.attrib.get(relationship_id_attribute)
        for hyperlink in root.findall(".//w:hyperlink", namespace)
    }
    expected_urls = [
        url.rstrip(".,;:!?)]}，。；：！？）】")
        for value in strings_in(spec)
        for url in URL_PATTERN.findall(value)
    ]
    if expected_urls:
        actual_targets = {relationship.attrib.get("Target") for relationship in hyperlink_relationships}
        if not set(expected_urls).issubset(actual_targets):
            raise ValueError("generated DOCX did not preserve every URL as an external hyperlink")
        if not linked_relationship_ids.issubset({
            relationship.attrib.get("Id") for relationship in hyperlink_relationships
        }):
            raise ValueError("generated DOCX contains an unresolved hyperlink relationship")
    return {
        "validated": True,
        "output": str(output.resolve()),
        "bytes": output.stat().st_size,
        "native_tables": len(actual_tables),
        "tables": table_receipts,
        "hyperlinks": len(hyperlink_relationships),
    }


def build_document(spec, output):
    title = str(spec.get("title") or "Nola Document")
    body, hyperlink_relationships = document_body(spec)
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as package:
        package.writestr(
            "[Content_Types].xml",
            '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>''',
        )
        package.writestr(
            "_rels/.rels",
            '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>''',
        )
        package.writestr(
            "docProps/core.xml",
            f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{escape(title)}</dc:title><dc:creator>Nola</dc:creator></cp:coreProperties>''',
        )
        package.writestr(
            "docProps/app.xml",
            '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Nola</Application></Properties>''',
        )
        package.writestr(
            "word/document.xml",
            f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{WORD_NS}" xmlns:r="{REL_NS}"><w:body>{body}</w:body></w:document>''',
        )
        package.writestr(
            "word/_rels/document.xml.rels",
            f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  {''.join(f'<Relationship Id="{relationship_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{escape(url, {chr(34): "&quot;"})}" TargetMode="External"/>' for relationship_id, url in hyperlink_relationships)}
</Relationships>''',
        )
        package.writestr("word/styles.xml", styles_xml())

    return validate_document(spec, output)


def main():
    parser = argparse.ArgumentParser()
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--spec")
    source.add_argument("--html")
    parser.add_argument("--output", required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    if args.spec:
        spec_path = Path(args.spec)
        spec = materialize_external_blocks(
            json.loads(spec_path.read_text(encoding="utf-8")), spec_path
        )
    else:
        spec = spec_from_html(Path(args.html))
    output = Path(args.output)
    receipt = validate_document(spec, output) if args.validate_only else build_document(spec, output)
    print(json.dumps(receipt, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
