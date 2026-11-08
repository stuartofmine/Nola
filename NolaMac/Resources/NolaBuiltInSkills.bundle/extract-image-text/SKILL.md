---
name: extract-image-text
description: Extract, transcribe, translate, and structure printed or handwritten text, tables, forms, receipts, screenshots, and scanned content from images. Use when pixels are the primary source and OCR accuracy matters; use work-with-pdf for page-preserving PDF OCR.
---

# Extract Text from Images

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Route page-preserving PDF OCR to the PDF Skill. Hand structured tables to data analysis only after reading order, row boundaries, and uncertain cells are recorded.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Faithful transcription**: preserve wording, line breaks, labels, and reading order as closely as possible.
- **Structured extraction**: return requested fields, rows, columns, or key-value pairs with source regions.
- **Normalization**: produce cleaned text while retaining a separate faithful extraction.
- **Translation**: translate only after extraction and keep source and translated text distinct.

## Readiness

- Inspect image count, file type, resolution, orientation, perspective, crop, contrast, language, handwriting, columns, and likely structure.
- Prefer on-device OCR. Do not upload private images or use external recognition without approval.
- Clarify required output schema, acceptable uncertainty, and whether exact typography or spatial layout must be retained.

## Progress contract

Use stable stages: inspecting images, preprocessing selected regions, recognizing text, reconstructing structure, verifying critical fields, and saving output. Batch repeated pages or images.

## Workflow

1. Preserve originals. Create derived rotations, crops, deskewed views, or contrast adjustments only as separate temporary assets.
2. Detect orientation and regions before OCR. Use appropriate language models and avoid aggressive enhancement that changes characters.
3. Reconstruct reading order, headings, columns, tables, checkboxes, and form relationships from spatial evidence.
4. Preserve raw OCR separately from normalized output when corrections are made.
5. Mark uncertain characters as uncertain; attach image and region references to material names, dates, identifiers, totals, and signatures.
6. For tables, verify row and column boundaries before exporting structured data.

## Bundled on-device path

- On macOS, run `swift "<skill-root>/scripts/extract_text.swift" --input "<image>" --output "<result.json>"` for local printed-text recognition before considering any external service. When the requested deliverable is plain text, add `--text-output "<result.txt>"`; the same program writes text in observation order.
- Validate the exact result with `swift "<skill-root>/scripts/extract_text.swift" --validate-only --output "<result.json>"`, adding the same `--text-output "<result.txt>"` when text was delivered.
- Preserve the observation order, bounding boxes, Vision confidence, raw text, normalized text, normalization decisions, and uncertain regions. Do not replace them with a prose-only transcription.

## macOS extraction contract

1. Read `NOLA_MANIFEST.json`, but do not read the bundled Swift source or probe image libraries, packages, file signatures, or metadata unless the OCR command reports a concrete input error.
2. With `cwd` set to the active workspace, run the bundled script once using relative input and output paths. For a requested `.txt`, provide both a descriptive JSON evidence path and `--text-output <requested.txt>`.
3. The JSON is durable OCR evidence, not a temporary helper. Keep it alongside the text because it contains reading order, bounding boxes, confidence, normalization decisions, and uncertain regions.
4. Do not manually rewrite recognized text with `write_file`, create an alternate OCR script, or delete the JSON evidence.
5. Make the final tool call the same bundled script with `--validate-only`, the JSON `--output`, and the same `--text-output` when present. Require exit code zero, `"validated": true`, and `"text_output_validated": true` for text delivery.
6. After the validation receipt, stop calling tools and report both paths plus uncertainties. Do not reread the text or run `ls` afterward.

## User control

- Confirm before storing or sharing extracted identity, health, financial, credential, or third-party personal information.
- Never infer cropped, obscured, blurred, or illegible content and never perform identity verification from appearance.
- Do not silently autocorrect identifiers, amounts, or proper names.

## Validation

- Compare output against representative regions at original resolution and sample every distinct layout type.
- Independently verify totals, dates, decimal separators, currencies, account or document identifiers, checkbox states, and table alignment.
- Report OCR confidence by page or field and enumerate unresolved values rather than replacing them with plausible text.
- Confirm generated files preserve UTF-8 text and requested row/field counts.

## Failure handling

Return partial output with exact failed images or regions, likely cause, and recommended recapture settings. Keep low-confidence raw text visibly separated from verified data.

## Completion

Return the requested transcription or structure, source-region references, normalization decisions, confidence and unresolved fields, validation performed, and absolute paths for generated artifacts.
