---
name: word-document-authoring
description: Create or edit polished, editable Word documents (.docx) including reports, proposals, letters, resumes, policies, and structured business documents. Use when the requested deliverable is a Word file; do not use for simple reading or summarization when no Word output is required.
---

# Create Word Document

## Nola runtime contract

- The Skill invocation result contains a compact `_nola_execution_context` object before the long instructions body. Read its absolute `workspace_root` and `base_directory` first. The same values may also appear as top-level compatibility fields, but execution must use `_nola_execution_context`; do not call `read_file` for a bare `NOLA_MANIFEST.json`, search for the manifest, rediscover the Skill directory, or infer paths from the process current directory.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove disposable helper files before the final validation. The JSON specification used by the bundled generator is validation provenance, not a disposable helper: keep it through completion unless the generator itself removes it before emitting its receipt. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use document review for substantive critique and deep research for missing current evidence. Consume their anchored results; do not repeat the review or research during layout.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Activation boundary

Use this workflow only when a `.docx` deliverable or an explicit Word edit is requested. For an existing document, preserve content, styles, comments, headers, footers, numbering, and section structure unless the user asks to change them. Never overwrite the source file without explicit approval.

## Operating modes

- **Create**: turn source material into a new document with a clear hierarchy and deliberate page design.
- **Edit**: make scoped changes while preserving unrelated content and formatting.
- **Template fill**: populate existing fields and styles without redesigning the template.
- **Review output**: add comments or tracked changes only when the available library can preserve them reliably; otherwise produce a separate review document and disclose the limitation.

## Readiness

- Treat source material already present in the current conversation, including an immediately preceding verified research result, as inspected input. Do not read the workspace merely to rediscover that material.
- Treat `workspace_root` and `base_directory` in `_nola_execution_context` as authoritative, existing, Host-validated paths. They require no `ls`, `file_tree`, `project_overview`, `tool_search`, manifest read, or other discovery call.
- Inspect files only when the requested document actually depends on user-supplied files whose contents have not yet been returned by a tool in this run. Resolve the destination relative to the authoritative `workspace_root`; do not inspect its parent directory before creating a new file.
- Determine audience, purpose, page size, language, target length, template or brand constraints, and whether a table of contents, citations, comments, or tracked changes are required.
- For a new document on macOS with no supplied template, resolve the bundled standard-library generator as `<base_directory>/scripts/create_document.py`, where `base_directory` is the exact value returned by the successful Skill invocation. Do not search for the script or infer its location. It creates native OOXML paragraphs, styles, code blocks, and Word tables without depending on `python-docx` or lossy HTML conversion. Use `python-docx` only when a previous tool result in this run has established that it is available and the requested edit requires package-level control.
- Never create a DOCX with `textutil`, HTML-to-DOCX conversion, RTF conversion, or renamed HTML. Those paths flatten native Word tables and can delete literal code tokens such as C/C++ angle-bracket includes.
- Do not probe for Python packages, run `pip`, or install dependencies merely to create a new document. Never claim that a dependency or permission changed unless a tool result confirms it.
- For an existing `.docx`, inspect paragraphs, styles, tables, sections, relationships, headers, footers, and embedded assets before editing.

## macOS creation contract

When the user has already supplied enough content for a new `.docx`:

1. Enter deterministic create mode immediately. The only successful tool sequence is one `write_file` call for the JSON specification followed by one `bash` call to the bundled generator. Do not inspect the workspace, Skill bundle, manifest, script, destination directory, or output file before these calls. Do not call `tool_search`, `file_tree`, `project_overview`, `read_file`, or an exploratory `bash`; do not create a Python generator.
2. Call `write_file` once with the destination spec path and pass the complete specification as the `json_content` object. The tool-call root must be exactly `{"path":"<workspace-relative-spec>.json","json_content":{"title":"...","blocks":[...]}}`; `json_content` itself must be an object, never the blocks array. Never serialize the specification into the string `content` field: nested JSON string escaping is fragile for long documents. Use `title` plus an ordered `blocks` array. Supported block types are `heading` (`level`, `text`), `paragraph` (`text`), `code` (`text`), `code_file` (`path`), `table` (`columns`, `rows`), `bullets` (`items`), and `page_break`. Preserve literal code characters such as `<`, `>`, and `&` as ordinary JSON string values. When the document must include an existing long source file, use `{"type":"code_file","path":"workspace-relative-source.cpp"}` instead of copying its contents into the tool arguments; the generator securely resolves it relative to the specification directory and validates the imported text.
   - A table's `columns` is always an array of header strings, never an integer column count. Its `rows` is always an array of row arrays, and every row must have exactly `columns.count` string values. The minimal valid shape is `{"type":"table","columns":["Name","Status"],"rows":[["Example","Open"]]}`. Do not add a numeric `columns`, object-shaped rows, or an unsupported replacement field.
   - If the runtime returns `invalid_tool_arguments`, read its `expected_schema` and `next_action`, then make at most one corrected call whose root is a JSON object. Never retry the same rejected shape, never wrap the arguments in an array or string, and never fall back to serialized JSON in `content`.
3. Call `bash` with `operation_kind` set to `mutate_and_validate`, `cwd` set to the exact `workspace_root` returned by the Skill invocation, and relative workspace filenames in the command: `/usr/bin/python3 "<base_directory>/scripts/create_document.py" --spec "<relative-spec-file>" --output "<relative-output-file>"`. Substitute the exact returned `base_directory` directly. Its existence is guaranteed by the successful Skill invocation, so never pass the literal placeholder, search for the generator, test whether it exists, or infer a workspace from the process current directory. The operation kind is Core's typed completion receipt; do not omit it or rely on command-text inference. Quote every path. Do not change `cwd` to the Skill bundle or join generation and validation with `&&`, pipes, substitutions, or redirects.
4. If the Mac permission panel appears, wait for that exact tool call to resume. Do not ask the user to change a global permission mode and do not interpret a chat message such as "I approve" as runtime approval.
5. Treat the generator's JSON receipt as the structural gate: it reopens the package, checks required OOXML parts, compares every heading/paragraph/code/list value, and compares every native table cell and dimension with the source specification. A nonzero exit, missing `"validated": true`, or mismatched table is a failed document. Once the receipt contains `"validated": true`, stop all file-mutating and inspection tools. Then use `skill_update` only to mark `inspect`, `execute`, and `verify` complete, register artifact `document` with the final canonical `file://` URI, and record verifier `completion-audit` with the retained validation receipt as evidence. Do not replace or repeat this gate with ad hoc `python -c`, inline scripts, raw XML substring checks, or directory listings. If the creation call was interrupted before a receipt was returned, run the same bundled script with `--validate-only --spec <spec-path> --output <final-path>`.
6. For ordinary document creation, do not call `computer_status`, launch Word or Quick Look, request Accessibility or Screen Recording permission, or perform any other tool call after the validated receipt. Visual application-level inspection is a separate enhanced-validation mode used only when the user explicitly asks for it; inability to perform that optional inspection must not turn a structurally validated document into a failed creation.

The JSON specification is an implementation and validation source, not the requested deliverable. Keep it in the workspace after the validated receipt so cleanup cannot invalidate the completed run, but report only the final `.docx` unless the user asks for intermediate files.

## Default design system

- Use `#18212B` for primary text, `#1769E0` for restrained accents, and `#EEF4FA` for subtle fills.
- Use one professional body family already available on the Mac, a clear heading scale, 1.15-1.3 line spacing, and consistent paragraph spacing.
- Build hierarchy with Word styles, not manual bolding and blank lines. Use proper lists, captions, table headers, page breaks, and section breaks.
- Render every `http://` or `https://` value as a real external Word hyperlink relationship, including URLs inside native table cells; visible URL text alone is not a clickable source link.
- Keep normal body text readable at 10.5-12 pt. Avoid dense full-width tables, decorative borders, and orphaned headings.

## Progress contract

Publish a progress update only when the stage changes: inspecting sources, building structure, applying layout, validating content, rendering pages, and saving output. Do not repeat unchanged status or expose raw implementation chatter.

## Workflow

1. Create a content map with title, audience, sections, evidence, tables, figures, and appendices. Resolve contradictions before layout.
2. Select or preserve styles. Define margins, header/footer behavior, page numbering, heading levels, table style, and accessibility metadata.
3. Draft content in logical order. Use tables only for real comparisons or structured facts; keep prose as prose.
4. For edits, identify exact target paragraphs or XML-backed objects, modify the smallest possible scope, and retain a source-to-output change list.
5. Add navigation and accessibility: semantic headings, descriptive links, alternative text when supported, repeating table headers, and logical reading order.
6. Save to a descriptive new filename, reopen it, and run the validation gate.

## Validation gate

- Reopen the `.docx` package and validate that it is readable, non-empty, and contains the expected sections, tables, images, headers, footers, and page settings.
- Extract document text and compare required names, dates, figures, headings, links, and table values against the source material.
- When the user explicitly requests visual inspection, render through an available Word-compatible renderer or Quick Look path and inspect every page for clipping, overflow, blank pages, broken numbering, orphaned headings, split rows, missing fonts, and inconsistent spacing. This optional step is not part of the default macOS creation contract.
- For edits, compare unchanged sections with the source and verify that comments or tracked changes survived when promised.
- If faithful rendering is unavailable, report that limitation rather than claiming visual validation.

## Completion contract

Report the document purpose, major design decisions, content or edit scope, validation performed, known limitations, and the final absolute file path. Never claim completion unless the file exists, reopens successfully, and satisfies the requested content.

## Safety and quality rules

- Never overwrite an existing file unless the user explicitly approves that exact path.
- Never fabricate citations, signatures, approvals, legal effect, or facts missing from the source.
- Do not flatten an editable document to an image-only file as a substitute for correct authoring.
- Keep temporary files inside the workspace and remove only temporary files created during this run.
