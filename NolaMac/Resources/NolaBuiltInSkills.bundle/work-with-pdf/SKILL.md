---
name: work-with-pdf
description: Read, extract, OCR, create, merge, split, rotate, redact, annotate, fill, or validate PDF files. Use whenever a PDF is the central input or deliverable, including scanned documents and layout-sensitive transformations; use document-review instead when the main goal is substantive critique.
---

# Work with PDF

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use document review for substantive findings and image OCR for standalone raster sources. This Skill owns page preservation, irreversible redaction gates, and final PDF validation.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Inspect**: extract text, metadata, tables, forms, attachments, bookmarks, and page geometry without modifying the file.
- **Transform**: merge, split, rotate, reorder, crop, compress, add pages, or convert while preserving fidelity.
- **OCR**: produce searchable text from scanned pages while retaining page images and marking uncertainty.
- **Redact or fill**: perform irreversible redaction or form completion with page-level visual verification.
- **Create**: generate a PDF only when PDF is the requested final format; preserve an editable source when practical.

## Readiness

- Resolve every input and output path inside the workspace. Treat originals as immutable and fingerprint them when an irreversible operation is requested.
- Inspect signature, encryption, permissions, page count, dimensions, rotation, text coverage, forms, annotations, attachments, bookmarks, and size.
- Select the least lossy available tool: native metadata/text extraction first, structured PDF libraries next, rendering/OCR only for pages that need it.
- For a new structured report PDF, use the bundled native macOS generator: `swift <skill-root>/scripts/create_pdf_report.swift --spec <spec-path> --output <final-path>`. The JSON specification contains `title`, optional `summary`, non-empty `sections` with `heading` and `paragraphs`, and an optional `footer`.
- After generation, run the same script with `--validate-only --output <final-path>`. A PDF signature alone is insufficient; validation must reopen the file, count pages, and extract non-empty text.
- Do not install software, bypass protection, upload private files, or remove signatures without approval.

## macOS creation contract

When the user requests a new structured PDF and has supplied enough content:

1. Read `NOLA_MANIFEST.json`, but do not read or copy the bundled generator source. Its command-line interface and validation receipt are fully specified here.
2. Write one JSON specification inside the active workspace. Use only `title`, optional `summary`, non-empty `sections` containing `heading` and `paragraphs`, and optional `footer`.
3. With `cwd` set to the active workspace, run `swift "<skill-root>/scripts/create_pdf_report.swift" --spec "<relative-spec>" --output "<relative-output>"`. Do not use inline Swift, Python wrappers, pipes, redirects, substitutions, or package probes.
4. Remove the temporary specification before final validation. Do not create any other helper or verification script.
5. Run `swift "<skill-root>/scripts/create_pdf_report.swift" --validate-only --output "<relative-output>"` as the final tool call. Require exit code zero and a JSON receipt containing `"validated": true`, a positive page count, and non-empty extracted text.
6. After that receipt, stop calling tools and report the final absolute path. Do not run `ls`, `find`, raw signature checks, PDFKit snippets, or a second advisory validator.

The bundled receipt is the default structural validation gate. Rendering and page-by-page visual inspection are enhanced validation used only when the user explicitly requests visual QA or when a lossy transformation, redaction, form fill, or source-preservation claim requires it. Lack of optional visual QA must not invalidate a structurally validated new report.

## Progress contract

Publish only stage transitions: inspecting PDF, extracting structure, processing pages, applying changes, rendering changed pages, and validating output. For long files, report bounded page ranges rather than repetitive per-page messages.

## Workflow

1. Build a page manifest with page number, size, rotation, text presence, and requested action.
2. For extraction, retain page and region anchors. For tables, preserve rows and columns and flag merged or ambiguous cells.
3. For transforms, write a new output and preserve bookmarks, metadata, links, annotations, and forms when supported. Disclose losses before proceeding.
4. For OCR, preprocess orientation and contrast conservatively, retain the original image layer, and separate recognized text from corrections.
5. For redaction, remove underlying content and associated metadata; a colored rectangle is not redaction.
6. Reopen the output and reconcile it with the page manifest. Render changed pages when enhanced visual validation is required by the operation or explicitly requested by the user.

## User control

- Confirm before overwriting, decrypting, flattening editable content, removing signatures, irreversible redaction, or sending a PDF outside the Mac.
- Treat hidden text, attachments, metadata, identity details, health records, and financial data as sensitive.
- If the requested fidelity cannot be achieved with available tools, offer a non-destructive alternative and do not claim success.

## Validation

- Verify PDF signature, encryption state, page count, order, dimensions, rotations, expected bookmarks, and output size.
- Extract text from representative and changed pages; reconcile required names, dates, identifiers, totals, and page ranges.
- For enhanced visual validation, render changed pages and inspect for blank output, clipping, missing fonts, broken images, shifted form fields, annotation loss, and visible but recoverable redacted text.
- For merge/split operations, reconcile source and output page counts. For OCR, sample low-confidence regions against original pixels.

## Failure handling

Keep the original and partial output separate. Report the exact failed page or operation, tool limitation, and whether the partial file is safe to inspect. Never silently skip a failed page.

## Completion

Report operation mode, source files, affected page ranges, preservation losses, validation performed, and the final absolute file path. Never claim completion unless the output exists and passes the relevant checks.
