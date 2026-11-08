#!/usr/bin/env python3
"""Create a small, editable XLSX workbook using only the Python standard library."""

import argparse
import ast
import json
import operator
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape
from xml.etree import ElementTree


BINARY_OPERATORS = {
    ast.Add: operator.add,
    ast.Sub: operator.sub,
    ast.Mult: operator.mul,
    ast.Div: operator.truediv,
}
UNARY_OPERATORS = {ast.UAdd: operator.pos, ast.USub: operator.neg}


def column_name(index):
    value = ""
    while index:
        index, remainder = divmod(index - 1, 26)
        value = chr(65 + remainder) + value
    return value


def column_index(name):
    value = 0
    for character in name.upper():
        value = value * 26 + ord(character) - 64
    return value


def safe_sheet_name(value):
    cleaned = re.sub(r"[\[\]:*?/\\]", " ", str(value)).strip() or "Sheet1"
    return cleaned[:31]


def cell_xml(reference, value, style, cached_formula_value=None):
    if isinstance(value, bool):
        return f'<c r="{reference}" s="{style}" t="b"><v>{1 if value else 0}</v></c>'
    if isinstance(value, (int, float)):
        return f'<c r="{reference}" s="{style}"><v>{value}</v></c>'
    text = str(value)
    if text.startswith("="):
        formula = escape(text[1:])
        if isinstance(cached_formula_value, float) and cached_formula_value.is_integer():
            cached_formula_value = int(cached_formula_value)
        cached = "" if cached_formula_value is None else f"<v>{cached_formula_value}</v>"
        return f'<c r="{reference}" s="{style}"><f>{formula}</f>{cached}</c>'
    return (
        f'<c r="{reference}" s="{style}" t="inlineStr">'
        f'<is><t xml:space="preserve">{escape(text)}</t></is></c>'
    )


def evaluate_numeric_expression(node):
    if isinstance(node, ast.Expression):
        return evaluate_numeric_expression(node.body)
    if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
        return node.value
    if isinstance(node, ast.BinOp) and type(node.op) in BINARY_OPERATORS:
        return BINARY_OPERATORS[type(node.op)](
            evaluate_numeric_expression(node.left),
            evaluate_numeric_expression(node.right),
        )
    if isinstance(node, ast.UnaryOp) and type(node.op) in UNARY_OPERATORS:
        return UNARY_OPERATORS[type(node.op)](evaluate_numeric_expression(node.operand))
    raise ValueError("unsupported formula expression")


def cached_cell_value(reference, all_rows, cache, visiting):
    reference = reference.upper()
    if reference in cache:
        return cache[reference]
    if reference in visiting:
        raise ValueError("circular formula reference")
    match = re.fullmatch(r"([A-Z]+)(\d+)", reference)
    if not match:
        raise ValueError("invalid cell reference")
    row_index = int(match.group(2)) - 1
    column = column_index(match.group(1)) - 1
    try:
        value = all_rows[row_index][column]
    except IndexError as error:
        raise ValueError("cell reference outside used range") from error
    visiting.add(reference)
    try:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            if not str(value).startswith("="):
                raise ValueError("formula references a non-numeric cell")
            resolved = formula_cache(value, all_rows, cache, visiting)
            if resolved is None:
                raise ValueError("formula could not be cached")
        else:
            resolved = value
        cache[reference] = resolved
        return resolved
    finally:
        visiting.remove(reference)


def formula_cache(value, all_rows, cache, visiting=None):
    text = str(value)
    if not text.startswith("="):
        return None
    visiting = visiting or set()
    expression = text[1:]

    def replace_sum(match):
        start_column, start_row, end_column, end_row = match.groups()
        values = [
            cached_cell_value(f"{column_name(column)}{row}", all_rows, cache, visiting)
            for row in range(int(start_row), int(end_row) + 1)
            for column in range(
                column_index(start_column),
                column_index(end_column) + 1,
            )
        ]
        return str(sum(values))

    try:
        expression = re.sub(
            r"SUM\(([A-Z]+)(\d+):([A-Z]+)(\d+)\)",
            replace_sum,
            expression,
            flags=re.IGNORECASE,
        )
        expression = re.sub(
            r"\b([A-Z]+\d+)\b",
            lambda match: str(cached_cell_value(match.group(1), all_rows, cache, visiting)),
            expression,
            flags=re.IGNORECASE,
        )
        return evaluate_numeric_expression(ast.parse(expression, mode="eval"))
    except (SyntaxError, ValueError, ZeroDivisionError):
        return None


def worksheet_xml(columns, rows, has_drawing=False):
    all_rows = [columns] + rows
    formula_values = {}
    row_parts = []
    for row_index, row in enumerate(all_rows, start=1):
        cells = []
        is_total = row_index > 1 and str(row[0]).strip().lower() in {
            "total", "subtotal", "合计", "总计",
        }
        for column_index, value in enumerate(row, start=1):
            reference = f"{column_name(column_index)}{row_index}"
            is_numeric = isinstance(value, (int, float)) or str(value).startswith("=")
            if row_index == 1:
                style = 1
            elif is_total:
                style = 6 if is_numeric else 5
            elif row_index % 2 == 1:
                style = 4 if is_numeric else 3
            else:
                style = 2 if is_numeric else 0
            cached = formula_cache(value, all_rows, formula_values)
            if cached is not None:
                formula_values[reference] = cached
            cells.append(cell_xml(reference, value, style, cached))
        height = 27 if row_index == 1 else 23
        row_parts.append(f'<row r="{row_index}" ht="{height}" customHeight="1">{"".join(cells)}</row>')
    last_column = column_name(max(1, len(columns)))
    last_row = max(1, len(all_rows))
    widths = []
    for index in range(1, len(columns) + 1):
        visible_lengths = [len(str(row[index - 1])) for row in all_rows if index <= len(row)]
        width = min(34, max(13, max(visible_lengths, default=10) + 3))
        widths.append(f'<col min="{index}" max="{index}" width="{width}" customWidth="1"/>')
    widths = "".join(widths)
    drawing = '<drawing r:id="rId1"/>' if has_drawing else ""
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <dimension ref="A1:{last_column}{last_row}"/>
  <sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>
  <sheetFormatPr defaultRowHeight="23"/>
  <cols>{widths}</cols>
  <sheetData>{"".join(row_parts)}</sheetData>
  <autoFilter ref="A1:{last_column}{last_row}"/>
  <printOptions gridLines="0"/>
  <pageMargins left="0.5" right="0.5" top="0.65" bottom="0.65" header="0.3" footer="0.3"/>
  <pageSetup orientation="landscape" fitToWidth="1" fitToHeight="0"/>
  {drawing}
</worksheet>"""


def chart_range_formula(sheet_name, reference):
    match = re.fullmatch(r"([A-Za-z]+)(\d+):([A-Za-z]+)(\d+)", str(reference).strip())
    if not match:
        raise ValueError(f"invalid chart range: {reference}")
    start_column, start_row, end_column, end_row = match.groups()
    if int(start_row) > int(end_row) or column_index(start_column) > column_index(end_column):
        raise ValueError(f"reversed chart range: {reference}")
    quoted_sheet = sheet_name.replace("'", "''")
    return (
        f"'{quoted_sheet}'!${start_column.upper()}${start_row}:"
        f"${end_column.upper()}${end_row}"
    )


def chart_xml(sheet_name, chart, chart_index):
    chart_type = str(chart.get("type") or "column").lower()
    if chart_type not in {"column", "bar", "line"}:
        raise ValueError(f"unsupported chart type: {chart_type}")
    title = escape(str(chart.get("title") or "Summary"))
    categories = chart_range_formula(sheet_name, chart.get("categories") or "A2:A2")
    series = chart.get("series") or []
    if not isinstance(series, list) or not series:
        raise ValueError("each chart requires at least one series")
    colors = ["1769E0", "20A464", "E59A18", "D45151", "6E62C9"]
    series_xml = []
    for index, item in enumerate(series):
        if not isinstance(item, dict):
            raise ValueError("chart series must contain objects")
        name = escape(str(item.get("name") or f"Series {index + 1}"))
        values = chart_range_formula(sheet_name, item.get("values") or "B2:B2")
        color = colors[index % len(colors)]
        marker = '<c:marker><c:symbol val="circle"/><c:size val="5"/></c:marker>' if chart_type == "line" else ""
        series_xml.append(
            f'<c:ser><c:idx val="{index}"/><c:order val="{index}"/>'
            f'<c:tx><c:v>{name}</c:v></c:tx>'
            f'<c:spPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill>'
            f'<a:ln><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></a:ln></c:spPr>'
            f'{marker}<c:cat><c:strRef><c:f>{escape(categories)}</c:f></c:strRef></c:cat>'
            f'<c:val><c:numRef><c:f>{escape(values)}</c:f></c:numRef></c:val></c:ser>'
        )
    category_axis = 41000000 + chart_index * 2
    value_axis = category_axis + 1
    if chart_type == "line":
        plot = (
            '<c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>'
            + "".join(series_xml)
            + f'<c:smooth val="0"/><c:axId val="{category_axis}"/><c:axId val="{value_axis}"/></c:lineChart>'
        )
    else:
        direction = "bar" if chart_type == "bar" else "col"
        plot = (
            f'<c:barChart><c:barDir val="{direction}"/><c:grouping val="clustered"/><c:varyColors val="0"/>'
            + "".join(series_xml)
            + f'<c:gapWidth val="70"/><c:axId val="{category_axis}"/><c:axId val="{value_axis}"/></c:barChart>'
        )
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>{title}</a:t></a:r></a:p></c:rich></c:tx><c:layout/><c:overlay val="0"/></c:title>
    <c:autoTitleDeleted val="0"/><c:plotArea><c:layout/>{plot}
      <c:catAx><c:axId val="{category_axis}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="{value_axis}"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx>
      <c:valAx><c:axId val="{value_axis}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:numFmt formatCode="General" sourceLinked="1"/><c:majorGridlines/><c:tickLblPos val="nextTo"/><c:crossAx val="{category_axis}"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>
    </c:plotArea><c:legend><c:legendPos val="b"/><c:layout/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart>
</c:chartSpace>'''


def drawing_parts(charts, first_chart_index):
    anchors = []
    relationships = []
    for offset, chart in enumerate(charts):
        anchor = str(chart.get("anchor") or f"G{2 + offset * 16}").upper()
        match = re.fullmatch(r"([A-Z]+)(\d+)", anchor)
        if not match:
            raise ValueError(f"invalid chart anchor: {anchor}")
        column = column_index(match.group(1)) - 1
        row = int(match.group(2)) - 1
        width = max(4, min(16, int(chart.get("width_columns") or 8)))
        height = max(8, min(30, int(chart.get("height_rows") or 15)))
        relationship_id = f"rId{offset + 1}"
        chart_index = first_chart_index + offset
        anchors.append(f'''<xdr:twoCellAnchor><xdr:from><xdr:col>{column}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>{column + width}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row + height}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{offset + 2}" name="Chart {chart_index}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="{relationship_id}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>''')
        relationships.append(f'<Relationship Id="{relationship_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart{chart_index}.xml"/>')
    drawing = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">' + "".join(anchors) + '</xdr:wsDr>'
    rels = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(relationships) + '</Relationships>'
    return drawing, rels


def styles_xml():
    return """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="3">
    <font><sz val="11"/><color rgb="FF18212B"/><name val="Aptos"/></font>
    <font><b/><sz val="11"/><color rgb="FFFFFFFF"/><name val="Aptos Display"/></font>
    <font><b/><sz val="11"/><color rgb="FF18212B"/><name val="Aptos"/></font>
  </fonts>
  <fills count="5">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FF1769E0"/><bgColor indexed="64"/></patternFill></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFF3F7FB"/><bgColor indexed="64"/></patternFill></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFDCEAFB"/><bgColor indexed="64"/></patternFill></fill>
  </fills>
  <borders count="2">
    <border><left/><right/><top/><bottom/><diagonal/></border>
    <border><left style="thin"><color rgb="FFD9E2EC"/></left><right style="thin"><color rgb="FFD9E2EC"/></right><top style="thin"><color rgb="FFD9E2EC"/></top><bottom style="thin"><color rgb="FFD9E2EC"/></bottom><diagonal/></border>
  </borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="7">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="1" xfId="0" applyAlignment="1"><alignment vertical="center" wrapText="1"/></xf>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyAlignment="1"><alignment horizontal="center" vertical="center" wrapText="1"/></xf>
    <xf numFmtId="4" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1"><alignment horizontal="right" vertical="center"/></xf>
    <xf numFmtId="0" fontId="0" fillId="3" borderId="1" xfId="0" applyAlignment="1"><alignment vertical="center" wrapText="1"/></xf>
    <xf numFmtId="4" fontId="0" fillId="3" borderId="1" xfId="0" applyNumberFormat="1"><alignment horizontal="right" vertical="center"/></xf>
    <xf numFmtId="0" fontId="2" fillId="4" borderId="1" xfId="0" applyAlignment="1"><alignment vertical="center" wrapText="1"/></xf>
    <xf numFmtId="4" fontId="2" fillId="4" borderId="1" xfId="0" applyNumberFormat="1"><alignment horizontal="right" vertical="center"/></xf>
  </cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"""


def build_workbook(spec, output):
    title = str(spec.get("title") or "Nola Workbook")
    sheets = spec.get("sheets") or []
    if not sheets:
        raise ValueError("spec must contain at least one sheet")

    output.parent.mkdir(parents=True, exist_ok=True)
    sheet_names = [safe_sheet_name(sheet.get("name", f"Sheet{index}")) for index, sheet in enumerate(sheets, 1)]
    sheet_overrides = "\n".join(
        f'  <Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        for index in range(1, len(sheets) + 1)
    )
    total_charts = sum(len(sheet.get("charts") or []) for sheet in sheets)
    chart_overrides = "\n".join(
        f'  <Override PartName="/xl/charts/chart{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>'
        for index in range(1, total_charts + 1)
    )
    drawing_overrides = "\n".join(
        f'  <Override PartName="/xl/drawings/drawing{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>'
        for index, sheet in enumerate(sheets, 1) if sheet.get("charts")
    )
    workbook_sheets = "".join(
        f'<sheet name="{escape(name)}" sheetId="{index}" r:id="rId{index}"/>'
        for index, name in enumerate(sheet_names, 1)
    )
    workbook_relationships = "".join(
        f'<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>'
        for index in range(1, len(sheets) + 1)
    )
    workbook_relationships += (
        f'<Relationship Id="rId{len(sheets) + 1}" '
        'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" '
        'Target="styles.xml"/>'
    )

    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as package:
        package.writestr(
            "[Content_Types].xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
{sheet_overrides}
{chart_overrides}
{drawing_overrides}
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>""",
        )
        package.writestr(
            "_rels/.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>""",
        )
        package.writestr(
            "docProps/core.xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{escape(title)}</dc:title><dc:creator>Nola</dc:creator></cp:coreProperties>""",
        )
        package.writestr(
            "docProps/app.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Nola</Application></Properties>""",
        )
        package.writestr(
            "xl/workbook.xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{workbook_sheets}</sheets><calcPr calcId="191029" fullCalcOnLoad="1" forceFullCalc="1"/></workbook>""",
        )
        package.writestr(
            "xl/_rels/workbook.xml.rels",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{workbook_relationships}</Relationships>""",
        )
        package.writestr("xl/styles.xml", styles_xml())
        next_chart_index = 1
        for index, sheet in enumerate(sheets, 1):
            columns = [str(value) for value in (sheet.get("columns") or [])]
            rows = sheet.get("rows") or []
            if not columns:
                raise ValueError(f"sheet {index} has no columns")
            if any(len(row) != len(columns) for row in rows):
                raise ValueError(f"sheet {index} row width does not match columns")
            charts = sheet.get("charts") or []
            if not isinstance(charts, list):
                raise ValueError(f"sheet {index} charts must be a list")
            package.writestr(
                f"xl/worksheets/sheet{index}.xml",
                worksheet_xml(columns, rows, has_drawing=bool(charts)),
            )
            if charts:
                package.writestr(
                    f"xl/worksheets/_rels/sheet{index}.xml.rels",
                    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing%d.xml"/></Relationships>' % index,
                )
                drawing, relationships = drawing_parts(charts, next_chart_index)
                package.writestr(f"xl/drawings/drawing{index}.xml", drawing)
                package.writestr(f"xl/drawings/_rels/drawing{index}.xml.rels", relationships)
                for offset, chart in enumerate(charts):
                    chart_index = next_chart_index + offset
                    package.writestr(
                        f"xl/charts/chart{chart_index}.xml",
                        chart_xml(sheet_names[index - 1], chart, chart_index),
                    )
                next_chart_index += len(charts)


def validate_workbook(output):
    required = {"[Content_Types].xml", "xl/workbook.xml", "xl/styles.xml"}
    with zipfile.ZipFile(output) as package:
        names = set(package.namelist())
        missing = sorted(required - names)
        if missing:
            raise ValueError(f"workbook is missing required parts: {', '.join(missing)}")
        for name in names:
            if name.endswith((".xml", ".rels")):
                ElementTree.fromstring(package.read(name))
        sheets = sorted(name for name in names if re.fullmatch(r"xl/worksheets/sheet\d+\.xml", name))
        if not sheets:
            raise ValueError("workbook has no worksheets")
        formulas = 0
        uncached_formulas = 0
        for name in sheets:
            xml = package.read(name).decode("utf-8")
            formulas += xml.count("<f>")
            uncached_formulas += len(re.findall(r"<f>.*?</f></c>", xml))
        if uncached_formulas:
            raise ValueError(f"workbook has {uncached_formulas} formulas without cached values")
        charts = sorted(name for name in names if re.fullmatch(r"xl/charts/chart\d+\.xml", name))
    return {
        "bytes": output.stat().st_size,
        "formulas": formulas,
        "charts": len(charts),
        "sheets": len(sheets),
        "validated": True,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec")
    parser.add_argument("--output", required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    output = Path(args.output)
    if not args.validate_only:
        if not args.spec:
            parser.error("--spec is required unless --validate-only is used")
        spec = json.loads(Path(args.spec).read_text(encoding="utf-8"))
        build_workbook(spec, output)
    print(json.dumps(validate_workbook(output), ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
