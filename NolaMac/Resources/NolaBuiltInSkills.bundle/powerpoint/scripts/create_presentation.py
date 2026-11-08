#!/usr/bin/env python3
"""Create an editable PPTX presentation using only the Python standard library."""

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape
from xml.etree import ElementTree


def text_run(text, size=2200, bold=False, color="18212B", hyperlink_id=None):
    bold_value = ' b="1"' if bold else ""
    hyperlink = (
        f'<a:u val="sng"/><a:hlinkClick r:id="{hyperlink_id}"/>'
        if hyperlink_id
        else ""
    )
    return (
        f'<a:r><a:rPr lang="zh-CN" sz="{size}"{bold_value}>'
        f'<a:solidFill><a:srgbClr val="{color}"/></a:solidFill>{hyperlink}</a:rPr>'
        f'<a:t>{escape(str(text))}</a:t></a:r>'
    )


def shape(shape_id, name, x, y, width, height, paragraphs, fill=None):
    fill_xml = (
        f'<a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>'
        if fill
        else "<a:noFill/>"
    )
    paragraph_xml = "".join(f"<a:p>{paragraph}<a:endParaRPr lang=\"zh-CN\"/></a:p>" for paragraph in paragraphs)
    return f"""
<p:sp>
  <p:nvSpPr><p:cNvPr id="{shape_id}" name="{escape(name)}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
  <p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>{fill_xml}<a:ln><a:noFill/></a:ln></p:spPr>
  <p:txBody><a:bodyPr wrap="square" anchor="ctr" lIns="228600" rIns="228600" tIns="114300" bIns="114300"/><a:lstStyle/>{paragraph_xml}</p:txBody>
</p:sp>"""


def normalized_bullets(slide, slide_index):
    bullets = []
    for value in slide.get("bullets") or []:
        if isinstance(value, dict):
            text = str(value.get("text") or "").strip()
            url = str(value.get("url") or "").strip()
        else:
            text = str(value).strip()
            url = ""
        if not text:
            continue
        if url and not re.fullmatch(r"https?://[^\s]+", url):
            raise ValueError(f"slide {slide_index} contains an invalid hyperlink")
        bullets.append((text, url))
    return bullets


def slide_xml(slide, slide_index, slide_count):
    title = slide.get("title") or "Untitled"
    bullets = normalized_bullets(slide, slide_index)
    if len(title) > 90:
        raise ValueError(f"slide {slide_index} title exceeds 90 characters")
    if len(bullets) > 5:
        raise ValueError(f"slide {slide_index} has more than 5 supporting points")
    if any(len(value[0]) > 180 for value in bullets):
        raise ValueError(f"slide {slide_index} contains a supporting point longer than 180 characters")
    bullets = bullets or [("No supporting points supplied.", "")]
    kicker_shape = shape(
        2,
        "Section label",
        685800,
        228600,
        1828800,
        365760,
        [text_run("NOLA BRIEF", size=1000, bold=True, color="FFFFFF")],
        fill="1769E0",
    )
    title_shape = shape(
        3,
        "Title",
        685800,
        685800,
        10820400,
        731520,
        [text_run(title, size=2900, bold=True, color="1769E0")],
    )
    card_top = 1645920
    card_gap = 137160
    available_height = 4114800
    card_height = (available_height - card_gap * (len(bullets) - 1)) // len(bullets)
    cards = []
    for index, (value, url) in enumerate(bullets, 1):
        y = card_top + (index - 1) * (card_height + card_gap)
        paragraph = (
            text_run(f"{index:02d}", size=1300, bold=True, color="1769E0")
            + text_run(
                f"   {value}",
                size=1950,
                color="1769E0" if url else "18212B",
                hyperlink_id=f"rIdLink{index}" if url else None,
            )
        )
        cards.append(
            shape(
                3 + index,
                f"Point {index}",
                914400,
                y,
                10210800,
                card_height,
                [paragraph],
                fill="EEF4FA" if index % 2 else "F7FAFC",
            )
        )
    footer_shape = shape(
        10,
        "Slide progress",
        10058400,
        6286500,
        1143000,
        274320,
        [text_run(f"{slide_index:02d} / {slide_count:02d}", size=1000, color="6B7280")],
    )
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
    {kicker_shape}{title_shape}{''.join(cards)}{footer_shape}
  </p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>"""


def notes_xml(notes):
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
    {shape(2, "Notes", 685800, 457200, 10820400, 5486400, [text_run(notes or "No speaker notes.", size=1800)])}
  </p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:notes>"""


def build_presentation(spec, output):
    slides = spec.get("slides") or []
    if not slides:
        raise ValueError("spec must contain at least one slide")
    output.parent.mkdir(parents=True, exist_ok=True)
    slide_overrides = "\n".join(
        f'  <Override PartName="/ppt/slides/slide{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>\n'
        f'  <Override PartName="/ppt/notesSlides/notesSlide{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>'
        for index in range(1, len(slides) + 1)
    )
    slide_ids = "".join(
        f'<p:sldId id="{255 + index}" r:id="rId{index + 1}"/>'
        for index in range(1, len(slides) + 1)
    )
    slide_relationships = "".join(
        f'<Relationship Id="rId{index + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{index}.xml"/>'
        for index in range(1, len(slides) + 1)
    )
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as package:
        package.writestr(
            "[Content_Types].xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
  <Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/>
{slide_overrides}
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>""",
        )
        package.writestr(
            "_rels/.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>""",
        )
        title = escape(str(spec.get("title") or "Nola Presentation"))
        package.writestr(
            "docProps/core.xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:creator>Nola</dc:creator></cp:coreProperties>""",
        )
        package.writestr(
            "docProps/app.xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Nola</Application><Slides>{len(slides)}</Slides><Notes>{len(slides)}</Notes></Properties>""",
        )
        package.writestr(
            "ppt/presentation.xml",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst>{slide_ids}</p:sldIdLst>
  <p:notesMasterIdLst><p:notesMasterId r:id="rId{len(slides) + 2}"/></p:notesMasterIdLst>
  <p:sldSz cx="12192000" cy="6858000" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>""",
        )
        package.writestr(
            "ppt/_rels/presentation.xml.rels",
            f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  {slide_relationships}
  <Relationship Id="rId{len(slides) + 2}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="notesMasters/notesMaster1.xml"/>
</Relationships>""",
        )
        package.writestr(
            "ppt/slideMasters/slideMaster1.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:sldLayoutIdLst><p:sldLayoutId id="1" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>""",
        )
        package.writestr(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>""",
        )
        package.writestr(
            "ppt/slideLayouts/slideLayout1.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>""",
        )
        package.writestr(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>""",
        )
        package.writestr(
            "ppt/theme/theme1.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Nola"><a:themeElements><a:clrScheme name="Nola"><a:dk1><a:srgbClr val="18212B"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="354352"/></a:dk2><a:lt2><a:srgbClr val="EEF4FA"/></a:lt2><a:accent1><a:srgbClr val="1769E0"/></a:accent1><a:accent2><a:srgbClr val="19A974"/></a:accent2><a:accent3><a:srgbClr val="F59E0B"/></a:accent3><a:accent4><a:srgbClr val="DB4B4B"/></a:accent4><a:accent5><a:srgbClr val="6B7280"/></a:accent5><a:accent6><a:srgbClr val="94A3B8"/></a:accent6><a:hlink><a:srgbClr val="1769E0"/></a:hlink><a:folHlink><a:srgbClr val="6B7280"/></a:folHlink></a:clrScheme><a:fontScheme name="Nola"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme><a:fmtScheme name="Nola"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>""",
        )
        package.writestr(
            "ppt/notesMasters/notesMaster1.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notesMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:hf hdr="0" ftr="0" dt="1" sldNum="1"/><p:notesStyle/></p:notesMaster>""",
        )
        package.writestr(
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>""",
        )
        for index, slide in enumerate(slides, 1):
            package.writestr(f"ppt/slides/slide{index}.xml", slide_xml(slide, index, len(slides)))
            hyperlink_relationships = "".join(
                f'<Relationship Id="rIdLink{bullet_index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{escape(url)}" TargetMode="External"/>'
                for bullet_index, (_, url) in enumerate(normalized_bullets(slide, index), 1)
                if url
            )
            package.writestr(
                f"ppt/slides/_rels/slide{index}.xml.rels",
                f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide{index}.xml"/>{hyperlink_relationships}</Relationships>""",
            )
            package.writestr(f"ppt/notesSlides/notesSlide{index}.xml", notes_xml(slide.get("notes", "")))
            package.writestr(
                f"ppt/notesSlides/_rels/notesSlide{index}.xml.rels",
                f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="../notesMasters/notesMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="../slides/slide{index}.xml"/></Relationships>""",
            )


def validate_presentation(output):
    required = {
        "[Content_Types].xml",
        "ppt/presentation.xml",
        "ppt/slideMasters/slideMaster1.xml",
        "ppt/theme/theme1.xml",
    }
    namespaces = {
        "a": "http://schemas.openxmlformats.org/drawingml/2006/main",
        "r": "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "pr": "http://schemas.openxmlformats.org/package/2006/relationships",
    }
    with zipfile.ZipFile(output) as package:
        names = set(package.namelist())
        missing = sorted(required - names)
        if missing:
            raise ValueError(f"presentation is missing required parts: {', '.join(missing)}")
        for name in names:
            if name.endswith((".xml", ".rels")):
                ElementTree.fromstring(package.read(name))
        slides = sorted(name for name in names if re.fullmatch(r"ppt/slides/slide\d+\.xml", name))
        notes = sorted(name for name in names if re.fullmatch(r"ppt/notesSlides/notesSlide\d+\.xml", name))
        if not slides or len(slides) != len(notes):
            raise ValueError("every slide must have exactly one notes page")
        shape_count = 0
        hyperlink_count = 0
        for name in slides:
            root = ElementTree.fromstring(package.read(name))
            relationship_name = name.replace("ppt/slides/", "ppt/slides/_rels/") + ".rels"
            if relationship_name not in names:
                raise ValueError(f"{name} is missing its relationship part")
            relationship_root = ElementTree.fromstring(package.read(relationship_name))
            relationships = {
                relationship.attrib.get("Id"): relationship
                for relationship in relationship_root.findall("pr:Relationship", namespaces)
            }
            for hyperlink in root.findall(".//a:hlinkClick", namespaces):
                relationship_id = hyperlink.attrib.get(
                    "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
                )
                relationship = relationships.get(relationship_id)
                if relationship is None:
                    raise ValueError(f"{name} contains an unresolved hyperlink relationship")
                if not relationship.attrib.get("Type", "").endswith("/hyperlink"):
                    raise ValueError(f"{name} hyperlink points to a non-hyperlink relationship")
                target = relationship.attrib.get("Target", "")
                if relationship.attrib.get("TargetMode") != "External" or not re.fullmatch(
                    r"https?://[^\s]+", target
                ):
                    raise ValueError(f"{name} contains an invalid external hyperlink target")
                hyperlink_count += 1
            transforms = root.findall(".//a:xfrm", namespaces)
            shape_count += len(transforms)
            for transform in transforms:
                offset = transform.find("a:off", namespaces)
                extent = transform.find("a:ext", namespaces)
                if offset is None or extent is None:
                    continue
                x = int(offset.attrib["x"])
                y = int(offset.attrib["y"])
                width = int(extent.attrib["cx"])
                height = int(extent.attrib["cy"])
                if x < 0 or y < 0 or x + width > 12192000 or y + height > 6858000:
                    raise ValueError(f"{name} contains an off-canvas shape")
    return {
        "bytes": output.stat().st_size,
        "hyperlinks": hyperlink_count,
        "notes": len(notes),
        "shapes": shape_count,
        "slides": len(slides),
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
        build_presentation(spec, output)
    print(json.dumps(validate_presentation(output), ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
