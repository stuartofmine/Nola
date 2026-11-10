---
name: analyze-data
description: Profile, clean, join, transform, statistically analyze, visualize, and explain CSV, TSV, Excel, JSON, logs, or authorized database data. Use for quantitative questions, metrics, anomalies, experiments, forecasts, and reproducible reports; use excel-workbook-authoring when workbook construction itself is the primary deliverable.
---

# Analyze Data

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use `excel-workbook-authoring` only when an editable workbook is the requested deliverable. Hand verified tables and chart data to presentation or PDF Skills instead of recomputing them there.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Profile**: establish schema, grain, coverage, distributions, missingness, duplicates, and quality risks.
- **Explain**: answer a defined question with descriptive statistics and traceable calculations.
- **Compare**: evaluate cohorts, periods, scenarios, or experiments with appropriate uncertainty.
- **Model**: forecast or classify only with explicit target, validation design, and limitations.
- **Deliver**: create a reproducible script, clean dataset, chart set, or report when requested.

## Readiness

- Preserve source data and use read-only database connections. Record source path or query, extraction time, grain, units, currency, timezone, and reporting period.
- Locate metric definitions and data dictionaries. Ask before choosing ambiguous joins, denominators, attribution windows, fiscal calendars, or missing-value policy.
- Profile schema, row counts, types, nulls, duplicates, ranges, category cardinality, date coverage, encodings, and possible selection bias.
- Minimize sensitive fields and never upload data without approval.

## Progress contract

Publish only stage changes: profiling data, resolving quality issues, running analysis, checking robustness, building visuals, and validating results. Do not stream every query or dataframe operation as prose.

## Workflow

1. Write an analysis contract mapping each user question to data, filters, metrics, method, checks, and output.
2. Keep raw, cleaned, and derived layers separate. Record every filter, join, imputation, exclusion, and transformation in reproducible code.
   - For custom analysis, write a named script inside the active workspace and execute that file with the workspace as `cwd`.
   - Never place a multiline program in `python -c`, `node -e`, a shell heredoc, or another opaque shell argument. Those forms cannot be audited reliably by the workspace path policy and are not a reproducible deliverable.
   - Use the structured file tools to create or patch the script. Keep source and output paths explicit arguments rather than embedding absolute paths throughout the program.
3. Reconcile joins with before/after row counts, unmatched keys, duplicate expansion, and authoritative totals.
4. Choose statistics appropriate to scale, sample, and design. Distinguish observed description, correlation, prediction, and causal inference.
5. Use charts with explicit title, population, period, units, baseline, and source. Avoid dual axes and decorative encodings that distort comparisons.
6. Test material findings against alternative reasonable filters, outliers, missingness choices, and subgroup definitions.

## Bundled report path

- For CSV or TSV descriptive analysis, use the bundled deterministic Markdown pipeline by default instead of writing a new analysis program:
  `python3 <skill-root>/scripts/analyze_table.py --input <source.csv> --output <report.md> [--group-column <column>] [--numeric-columns <comma-separated-columns>]`.
- If `--group-column` is omitted, the pipeline selects the first non-numeric column. If `--numeric-columns` is omitted, it analyzes every consistently numeric column. Supply explicit columns when the user's question or schema makes them clear.
- Validate the exact source and delivered Markdown report as the final tool call:
  `python3 <skill-root>/scripts/analyze_table.py --validate-only --input <source.csv> --output <report.md> [the same column options]`.
- The deterministic pipeline profiles schema and missingness, reconciles row counts and every numeric total, emits grouped statistics and limitations, and embeds a machine-checkable payload. Do not recreate this pipeline, generate a replacement helper, inspect its implementation, or probe Python dependencies when it supports the input and requested output.
- For CSV cohort analysis with binary conversion and a numeric metric, use `python3 <skill-root>/scripts/analyze_dataset.py --input <source.csv> --output <report.html>` rather than composing calculated findings by hand.
- Column defaults are `cohort`, `converted`, and `revenue`; override them with `--group-column`, `--conversion-column`, and `--metric-column` when the user's schema is unambiguous.
- Validate the exact delivered report with `python3 <skill-root>/scripts/analyze_dataset.py --validate-only --output <report.html>`.
- The report embeds the computed payload, row reconciliation, causal boundary, and an SVG chart. Do not replace it with a prewritten narrative receipt.

## Custom Markdown analysis contract

Use this path only when neither bundled pipeline can express required joins, transformations, statistical tests, modeling, or output semantics. A Markdown request by itself is not a reason to write a custom program:

1. Inspect only the named source data and determine its exact columns, row count, units, and requested reconciliations before writing code.
2. Create one clearly named reproducible analysis program in the active workspace, such as `analysis_pipeline.py`. It is a supporting deliverable, not a temporary helper, so do not delete it.
3. The program must accept explicit `--input` and `--output` paths plus `--validate-only`. Generation must compute findings from the source and write the requested report. Validation must reopen both the source and delivered report, recompute authoritative row counts and totals, and fail if required findings, source coverage, totals, or the limitations section do not reconcile.
4. Run the program once to generate the report. Correct any program or report defect before final validation; do not use inline Python, shell heredocs, or a second validator.
5. After all edits are complete, run the same program with `--validate-only --input <source> --output <report>` as the final tool call. Require exit code zero and a structured receipt containing `"validated": true` plus reconciled row and total values.
6. After that receipt, stop calling tools and report the analysis, reproducible program, and final report paths. Do not delete the program, reread the report, or run `ls` after validation.

## User control

- Confirm before modifying a database, replacing source files, publishing dashboards, or sharing sensitive outputs.
- Do not infer protected traits, identity, intent, or causal effects unsupported by design and evidence.
- Label imputation, estimates, forecasts, model-generated values, and uncertainty prominently.

## Validation

- Re-run from the recorded inputs and confirm reproducible results.
- Validate only after temporary helpers have been removed. A custom reproducible analysis program is not a temporary helper and must remain available. The final successful command must validate the exact report or dataset delivered to the user, and no cleanup, read, or edit may follow it.
- Reconcile row counts, totals, units, dates, group sums, and a sample of source-to-output records.
- Check leakage, train/test separation, baseline performance, residual or error behavior, and calibration for predictive work.
- Verify every chart value against its underlying table and every narrative number against computed output.

## Failure handling

Stop when the grain, join, metric, or source authority remains unresolved and would change the conclusion. Preserve diagnostic outputs and state exactly what additional definition or data is required.

## Completion

Report the analysis contract, source coverage, methods, key findings, effect sizes or uncertainty, quality limitations, robustness checks, and absolute paths for reproducible artifacts.
