---
name: powerpoint-presentation-authoring
description: Create or edit polished, editable PowerPoint presentations (.pptx) with a coherent narrative, reusable layouts, charts, visuals, and speaker notes. Use when the requested deliverable is a presentation file, not for a plain-text outline alone.
---

# Create PowerPoint Presentation

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use `nature-paper2ppt` for a scientific paper-to-deck workflow and use data analysis for nontrivial calculations. This Skill remains responsible for the final editable package and slide-level validation.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Activation boundary

Use this workflow for a requested `.pptx` deliverable or an explicit presentation edit. Preserve the original deck, master, theme, notes, animations, and unrelated slides unless the user requests changes. Never overwrite the source deck without explicit approval.

## Operating modes

- **Create**: develop narrative, slide architecture, visuals, and speaker support from source material.
- **Edit**: change selected slides while preserving theme and neighboring narrative.
- **Template fill**: use existing layouts and placeholders without rebuilding the visual system.
- **Executive distill**: reduce a larger source into a decision-oriented deck with traceable evidence.

## Readiness

- Determine audience, presentation setting, duration, slide count, decision sought, language, aspect ratio, brand assets, and whether speaker notes or handouts are required.
- Inspect every source and, for an existing deck, inventory masters, layouts, theme fonts, colors, placeholders, notes, charts, images, and slide sizes.
- For a new narrative deck that fits the supported title/bullets/notes contract, use the bundled dependency-free `scripts/create_presentation.py` generator. Write a UTF-8 JSON specification in the workspace, then invoke `python3 <skill-root>/scripts/create_presentation.py --spec <spec-path> --output <final-path>`.
- The JSON specification contains `title` and a non-empty `slides` array. Each slide contains `title`, `bullets`, and `notes`; the output uses editable text boxes, a 16:9 canvas, a restrained theme, and one notes page per slide. A bullet may be a string or `{ "text": "Source label", "url": "https://…" }`; use the object form for requested hyperlinks. Never patch generated Open XML with a temporary script.
- The bundled layout supports at most five concise points per slide, 90 title characters, and 180 characters per point. Split overloaded content into another slide rather than shrinking it until unreadable.
- After generation, run `python3 <skill-root>/scripts/create_presentation.py --validate-only --output <final-path>` and treat malformed relationships, missing notes, or off-canvas shapes as failures.
- Use this exact completion order for a bundled-generator deck: write one compact JSON specification; generate the deck; use an already-available supported renderer for an optional bounded preview; remove preview output; then run `create_presentation.py --validate-only` as the final file-system tool call. Do not create helper scripts. Do not delete, list, render, or otherwise mutate or inspect workspace files after that successful final validation. Use `skill_update` only to mark `inspect`, `execute`, and `verify` complete, register artifact `presentation` with the final canonical `file://` URI, and record verifier `completion-audit` with the retained validation receipt as evidence; then immediately return the final artifact and concise validation summary.
- Do not write slide-splitting scripts, render-audit programs, OCR programs, or other helper code merely to manufacture a visual-validation pipeline. Quick Look's single deck thumbnail is not evidence that every slide rendered. When no existing faithful per-slide renderer is available, rely on the bundled validator's all-slide content, relationship, notes, geometry, and bounds checks and state that full visual rendering remains a limitation.
- Use `python-pptx` only when a previous tool result has established that it is available and the request requires richer editable objects. Do not install packages merely to create a deck. Do not claim support for animations, advanced charts, or master edits that the available library cannot preserve.

## Default design system

- Use `#18212B` for primary text, `#1769E0` for emphasis, and `#EEF4FA` for quiet surfaces unless a supplied brand system overrides them.
- Use one idea per slide, short assertion-led titles, a consistent grid, generous margins, and a limited set of slide archetypes.
- Keep body text generally at 18 pt or larger and titles at 28 pt or larger. Prefer diagrams, charts, and evidence over dense paragraphs.
- Preserve editable text and shapes. Use high-resolution visuals with clear provenance and do not stretch images.

## Progress contract

Publish stage changes only: extracting evidence, shaping the story, composing slides, checking overflow, rendering, and validating. Do not narrate every shape insertion.

## Workflow

1. Create a story spine: context, tension, evidence, options, recommendation, action, and appendix as appropriate.
2. Produce a slide plan with purpose, title assertion, evidence, visual form, and speaker note for each slide.
3. Select existing layouts or define a small repeatable layout set. Align all objects to a grid and use consistent spacing tokens.
4. Build charts from verified values and label units, periods, sources, and assumptions. Simplify visuals without changing meaning.
5. Add speaker notes for nuance, transitions, sources, and talking points rather than duplicating slide text.
6. Save to a new filename, reopen the package, render all slides, and run the validation gate.

## Validation gate

- Validate the `.pptx` package, slide order, dimensions, layouts, relationships, notes, images, charts, and required content.
- When an existing faithful per-slide renderer is available, render every slide and inspect for overflow, clipping, off-canvas objects, missing fonts, low contrast, inconsistent alignment, accidental overlap, and unreadable charts. Do not build a renderer or OCR pipeline during ordinary deck creation.
- Compare all names, dates, figures, claims, citations, and chart values with the source material.
- Check narrative continuity, repeated slide titles, appendix references, and that notes match the correct slides.
- If faithful rendering is unavailable, report that visual validation is incomplete.

## Completion contract

Report audience, narrative structure, slide count, editable elements, validation performed, rendering limitations, and the final absolute file path. Never claim completion unless the deck exists, reopens, and passes content checks.

## Safety and quality rules

- Never overwrite an existing deck without explicit approval.
- Never fabricate metrics, quotations, logos, customer claims, or source attribution.
- Do not convert the entire presentation to slide images to hide layout defects.
- Keep confidential source material and brand assets inside the approved workspace.
