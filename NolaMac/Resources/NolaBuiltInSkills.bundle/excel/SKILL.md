---
name: excel-workbook-authoring
description: Create or edit reliable, editable Excel workbooks (.xlsx) with structured tables, formulas, validation, charts, and documented assumptions. Use when the requested deliverable is an Excel workbook; do not use merely because source data happens to be tabular.
---

# Create Excel Workbook

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove disposable helper files before the final validation. The workbook JSON specification is validation provenance, not a disposable helper: keep it through completion. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use `analyze-data` for nontrivial statistical or data-quality reasoning, then consume its verified tables. Do not duplicate analytical calculations while laying out the workbook.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Activation boundary

Use this workflow for a requested `.xlsx` deliverable or an explicit workbook edit. Preserve original workbooks and unrelated sheets, formulas, names, styles, and metadata unless the user requests changes. Never overwrite the source file without explicit approval.

## Operating modes

- **Model**: build a calculation, forecast, budget, tracker, or scenario workbook.
- **Report**: present cleaned data, pivots, summaries, and charts for a defined audience.
- **Template**: create reusable input sheets with instructions and validation.
- **Edit**: make scoped changes while preserving formulas, references, and workbook behavior.

## Readiness

- Define the grain, units, currency, timezone, reporting period, business rules, expected inputs, and authoritative totals.
- Inspect source files for schemas, row counts, missing values, duplicates, formulas, merged cells, named ranges, hidden sheets, and external links.
- For a new workbook that fits the supported tabular contract, use the bundled dependency-free `scripts/create_workbook.py` generator. Write a UTF-8 JSON specification in the workspace, then invoke `python3 <skill-root>/scripts/create_workbook.py --spec <spec-path> --output <final-path>`.
- The JSON specification contains `title` and a non-empty `sheets` array. Each sheet contains `name`, `columns`, and `rows`; a string beginning with `=` becomes an editable formula.
- A sheet may contain `charts`. Each chart uses `type` (`column`, `bar`, or `line`), `title`, a category range such as `A2:A5`, one or more series objects such as `{"name":"Budget","values":"B2:B5"}`, and an optional anchor such as `G2`. Prefer this native contract over writing temporary `openpyxl` helper scripts.
- After generation, run `python3 <skill-root>/scripts/create_workbook.py --validate-only --output <final-path>`. Treat a missing cached formula result, malformed Open XML part, or absent worksheet as a failed deliverable.
- For workbooks created by the bundled generator, its `--validate-only` mode is the authoritative package, native-formula, cached-result, worksheet, and chart validator. Encode the requested source values, formulas, totals, and chart ranges explicitly in the retained JSON specification so that the artifact remains auditable.
- Once the final validation command succeeds, stop all file-mutating and inspection tools. Then use `skill_update` only to mark `inspect`, `execute`, and `verify` complete, register artifact `workbook` with the final canonical `file://` URI, and record verifier `completion-audit` with the retained validation receipt as evidence. Do not delete the specification, list the output, inspect Open XML with `python -c` or a heredoc, or run a second ad hoc validator afterward; those actions either invalidate the validation state, trigger shell-policy false positives, or add no stronger evidence.
- Use `openpyxl` only when a previous tool result has established that it is available and the request requires features outside the bundled generator. Do not install packages merely to create a workbook, and do not assume an authoring library recalculates formulas.
- Ask before resolving ambiguous denominators, fiscal periods, conversion rates, or conflicting source totals.

## Default design system

- Use `#18212B` for primary text, `#1769E0` for headings and key series, and `#EEF4FA` for restrained fills.
- Separate Instructions, Inputs, Calculations, and Outputs when complexity warrants it. Use descriptive sheet names and workbook tables.
- Distinguish editable inputs, formulas, and outputs consistently without relying on color alone.
- Apply real number formats for dates, percentages, currency, decimals, and negative values. Freeze useful headers and set practical column widths.

## Progress contract

Publish stage changes only: profiling inputs, designing the workbook, writing formulas, building visuals, recalculating, and validating. Let the tool trace carry file-level details without flooding the conversation.

## Workflow

1. Write a workbook specification listing sheets, table grain, columns, formulas, validations, named ranges, charts, and reconciliation totals.
2. Normalize source data into clean tabular regions with one header row, stable types, no decorative merged cells, and explicit missing-value handling.
3. Build formulas from cells or named ranges instead of embedding unexplained constants. Document assumptions beside inputs.
4. Add validation lists, bounds, conditional formatting, filters, freeze panes, and protected formula regions where appropriate.
5. Create charts only when they answer a question. Use clear titles, units, source ranges, accessible colors, and restrained series counts.
6. Save to a new filename, request recalculation from a compatible spreadsheet engine if available, reopen the Open XML package, and validate.

## Validation gate

- Validate the `.xlsx` package, sheet order, expected tables, used ranges, formulas, named ranges, validations, hyperlinks, filters, and charts.
- Reconcile source and output row counts, totals, dates, units, and a sample of transformed records.
- Scan formulas for `#REF!`, `#DIV/0!`, `#VALUE!`, unintended blanks, stale external links, circular references, and inconsistent copied ranges.
- Because authoring libraries may not calculate formulas, distinguish formula integrity from recalculation. If no calculation engine is available, report that cached results were not independently refreshed.
- Inspect representative rendered sheets for clipped headers, unreadable widths, hidden data, chart overlap, and print-area problems.
- Verify direct arithmetic and aggregate formulas both display cached values before Excel performs its own recalculation.

## Completion contract

Report workbook structure, input and formula conventions, reconciliations, recalculation status, known limitations, and the final absolute file path. Never claim completion unless the workbook exists and reopens successfully.

## Safety and quality rules

- Never overwrite an existing workbook without explicit approval.
- Never hide unexplained constants, silently discard rows, or present estimates as observed values.
- Preserve raw source data or a traceable import path. Do not embed credentials or sensitive source locations.
- Keep formulas editable and the main outputs understandable without reverse-engineering the workbook.
