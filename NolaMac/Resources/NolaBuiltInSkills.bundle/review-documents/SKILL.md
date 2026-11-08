---
name: review-documents
description: Review, compare, summarize, critique, comment on, or redline contracts, policies, specifications, reports, papers, and proposals. Use when the user needs traceable risks, inconsistencies, changes, evidence, or revisions; use format-specific authoring skills when visual production is the main task.
---

# Review Documents

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Keep substantive review and the finding ledger here. Hand format-preserving edits to the matching Word or PDF Skill with exact anchors and approved replacement text.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Executive review**: identify purpose, key terms, decisions, material risks, and open questions.
- **Criteria review**: assess against a supplied rubric, policy, specification, or checklist.
- **Version comparison**: explain semantic changes, not just text differences, with old/new anchors.
- **Redline or comments**: propose or apply revisions while preserving traceability and formatting.

## Readiness

- Identify authoritative versions, review purpose, audience, governing criteria, deadline, and requested output.
- Preserve originals and extract structure with page, section, heading, table, paragraph, or clause anchors.
- Determine whether tracked changes and comments can be preserved by the available toolchain before promising them.
- For legal, medical, financial, or compliance material, state that the work is analytical support rather than professional advice.

## Progress contract

Use stage messages only: indexing documents, comparing versions, evaluating criteria, drafting findings, applying revisions, and validating output. Do not report each paragraph scan.

## Workflow

1. Map document structure, defined terms, references, obligations, evidence, decisions, and appendices.
2. Define finding categories: factual inconsistency, ambiguity, omission, conflict, unsupported claim, risk, usability, and style.
3. Record each finding as severity, confidence, exact anchor, evidence, consequence, and recommended action or replacement text.
4. For comparisons, distinguish additions, removals, changed meaning, changed responsibility, changed dates or amounts, and formatting-only differences.
5. For redlines, change the smallest scope, retain a change ledger, and never silently accept all revisions.
6. Separate document facts from external assumptions and reviewer interpretation.

## User control

- Confirm before overwriting, accepting changes, deleting clauses, removing comments, flattening tracked changes, or sharing confidential material.
- Do not invent missing terms, assert legal effect, or conceal uncertainty caused by incomplete extraction.
- Keep quotations limited to what is needed to locate and explain the finding.

## Validation

- Reconcile headings, definitions, cross-references, dates, names, figures, tables, citations, obligations, and signature blocks.
- Verify each high-severity finding against the document and each replacement against the stated review goal.
- Reopen edited output and compare unchanged sections, formatting, comments, tracked changes, pagination, and attachments with the source.
- Check that every finding has a valid page or section anchor.

## Failure handling

Stop if extraction is incomplete, versions are ambiguous, or the editing tool cannot preserve promised review metadata. Provide a separate review report rather than damaging the source.

## Completion

Deliver an executive summary, prioritized finding register, version-change summary when applicable, open questions, recommended actions, and validated absolute paths for any outputs.
