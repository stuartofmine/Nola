---
name: conduct-deep-research
description: Conduct substantial multi-source investigations and produce a research brief, report, comparison, or evidence synthesis. Use when the requested outcome needs a research workflow and explicit verification. For a narrow current fact, release, or date lookup, use ordinary web search and fetch directly; freshness or a citation alone does not require this Skill.
---

# Conduct Deep Research

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Keep discovery and evidence synthesis here. Use the scientific reader for full-paper reconstruction, the reference verifier for field-level citation checks, and authoring Skills only after the evidence ledger is stable.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Rapid brief**: answer a narrow current question with a small, high-quality evidence set.
- **Deep report**: map a broad question, inspect ranked results, cross-check claims, and synthesize decisions and uncertainty.
- **Domain search**: restrict discovery to a user-specified site or corpus while preserving the original query.
- **Academic review**: prioritize original papers, official repositories, methods, datasets, and publication status.

## Readiness

- Preserve the original question. Define scope, geography, entities, time horizon, event date versus publication date, freshness, and decision context.
- Use web search before local retrieval for current external facts. Query rewriting belongs to the caller: never silently replace the user's intent inside the search tool.
- Prefer primary sources, filings, official records, direct statements, standards, original papers, and authoritative datasets.

## Progress contract

Use stable stages: mapping the question, searching the web, opening primary sources, cross-checking claims, resolving conflicts, and drafting the synthesis. Update only when the stage or evidence gap changes.

## Workflow

1. Build a question map with claims to establish, evidence needed, likely primary sources, and freshness thresholds.
2. Search the original query. Use explicit follow-up queries only for named evidence gaps, and use `site:` restrictions when the user requests a specific domain.
   For unrestricted discovery, begin without a host restriction. A later domain-specific query must target a recorded evidence gap; it must not silently redefine the overall corpus. Academic mode adjusts network waits, not the corpus. An archive is one discovery channel, not the whole academic literature. Review relevant publication venues, original repositories and author/institution pages as the question requires, without imposing a fixed site quota.
3. For a normal deep report, use at most 8 search queries and open 8–12 strong sources. The search result page may contain up to 20 ranked results; assessing that result set does not mean fetching all 20 pages. Exceed this budget only when the user explicitly asks for an exhaustive or systematic review, and state why.
4. Open sources and maintain a source ledger: title, author or institution, URL, publication date, event date, source type, claims supported, and limitations.
5. Treat search snippets only as discovery hints. Never cite a snippet as evidence when the underlying page can be opened.
6. Cross-check consequential claims with two independent sources when possible. Explain credible disagreement instead of averaging it away.
7. Separate verified fact, source allegation, expert interpretation, and assistant inference.

## User control

- Do not bypass paywalls, authentication, robots restrictions, geographic controls, or private-account boundaries.
- Ask before accessing private sources or saving sensitive research material outside the workspace.
- Do not fabricate citations, dates, quotations, or source consensus.

## Validation

- Reopen every cited source and verify that it directly supports the adjacent claim.
- Check identities, dates, numerical units, revisions, retractions, and whether a current claim relies on stale reporting.
- Remove duplicate syndications from independence counts and trace summaries back to original reporting where possible.
- Maintain a compact evidence ledger in the report: actual queries and their scope reasons; claims linked to source URLs and supporting excerpts; publication/version dates; shared original sources; unresolved coverage gaps. Derive search counts from tool records, not recollection. Different domains do not prove independence, and separate original papers on one archive need not be dependent. An unrelated news URL cannot validate an academic survey.
- Verify short quotations exactly and keep them necessary and limited.

## Large report transport

- Do not attempt to send a long report in one `write_file` call. Tool-call JSON may be truncated even when the model can hold the full draft.
- For a report likely to exceed 5,000 UTF-8 bytes, call `append_file` in ordered sections of at most 5,000 bytes. Set `create=true` only for the first section and use `create=false` for every later section.
- Keep each section independently valid Markdown and append in final reading order. Append each numbered section exactly once. Before appending, check the existing heading index so a retry cannot duplicate a completed section.
- After the last append, reopen the completed file once and audit headings, citations, access dates, and the source ledger; do not rewrite the full report merely to correct one section.
- `apply_patch(dry_run=true)` is only a one-time preview. If the preview is correct, immediately submit the same patch with `dry_run=false`; if it is wrong, revise it once. Never repeat an identical dry run and never leave an accepted correction unapplied.
- For a saved Markdown report, run `python3 <skill-root>/scripts/validate_research.py --input <report-path>`. This is a structure-only check of headings, URLs and access-date markers. Its output explicitly does not verify evidence. Never describe its exit code as proof of factual accuracy, source independence, freshness or coverage.
- Complete the separate claim-support, independence, scope and freshness audits against the evidence ledger. A missing audit stays unverified; return a bounded partial result if the necessary evidence cannot be obtained. Once these checks are satisfied, stop using tools and return the result; avoid further cosmetic audits.

## Failure handling

If live search, source access, or evidence quality is insufficient, return a bounded partial result with attempted queries, unavailable sources, and specific gaps. Do not fill gaps from memory while presenting the answer as current research.

## Completion

Lead with the answer or decision-relevant synthesis, place citations next to supported claims, and close with scope, confidence, disagreements, source limitations, and unresolved questions.
