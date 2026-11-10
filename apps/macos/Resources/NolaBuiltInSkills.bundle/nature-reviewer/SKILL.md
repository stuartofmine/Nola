---
name: nature-reviewer
description: "Simulate a Nature-style reviewer assessment from the referee perspective rather than an author rebuttal. Use when the user wants a pre-submission review, reviewer report, peer-review style critique, novelty/significance/technical soundness assessment, reviewer-style manuscript evaluation, 审稿人视角评估, 预审稿意见, or Nature reviewer report. Return 3 reviewer reports plus a cross-review synthesis, grounded only in the local Nature reviewer source basis. Also trigger on general pre-submission review requests during academic writing even without the word \"Nature\", such as getting a mock peer review for any journal, critiquing a draft as a reviewer would, assessing novelty/rigor before submission, and Chinese phrasings like 审稿人视角、模拟审稿、预审、帮我审一下论文、投稿前自审、审稿意见模拟、找论文问题."
---

# Nature Reviewer Assessment Skill

## Nola runtime contract

- The host has already loaded this Skill and its manifest. Do not reopen `NOLA_MANIFEST.json`, `SKILL.md`, bundled scripts, or reference files during an ordinary manuscript review.
- All workflow and reference files are already bundled locally; never download or self-install a Skill.
- Load only the references selected by this task. Ask before external writes or private-source access, and stop at readiness if a required capability is unavailable.
- Never bypass authentication, paywalls, robots restrictions, CAPTCHAs, or browser security controls.
- When the user requests a Markdown artifact, create that exact path rather than returning only chat text.
- Keep the complete review below 6,000 UTF-8 bytes and write it immediately with one `write_file` call. `append_file` never replaces an existing draft and is not the default for this report.
- Keep the report concise enough to audit: one setup section, exactly three reviewer sections with exactly two source-anchored concerns each, one synthesis, and one unsupported-claims section.
- After writing, run `python3 scripts/validate_review.py --source <manuscript-path> --review <review-path>` as the final artifact gate. The validator checks the three-reviewer structure, cross-review synthesis, stable concern IDs, claim/evidence pointers, resolution tests, and source-data anchoring. Do not mutate the artifact after validation; re-reading it is allowed but does not replace the validator.


Use this skill to simulate a `Nature`-style reviewer assessment package from the referee
side.

This skill is for reviewer-style manuscript evaluation, not for drafting the authors'
response. If the user wants rebuttal writing, route to `nature-response`.

## Default stance

- Ground the review only in the local source basis plus manuscript facts supplied by the user.
- Evaluate the manuscript against source-grounded axes: `originality`, `scientific importance`, `interdisciplinary readership`, `technical soundness`, and `readability for nonspecialists`.
- Use the 12-axis technical concern taxonomy only as an internal coverage checklist; it supplements but never replaces the five source-grounded axes.
- Return exactly `3 reviewer reports + 1 cross-review synthesis` unless the user explicitly asks for another structure.
- The three reviewers may differ only in `emphasis`; do not invent reviewer identities, specialties, institutions, or biographies.
- Identify who would be interested in the results and why.
- Identify technical failings that must be addressed before the authors' case is established.
- Give every substantive concern a stable ID, a faithful `claim_pointer`, and a verifiable `evidence_pointer`; mark missing locations instead of inventing them.
- Distinguish clearly between what is supported, what is weak, and what is not assessable from the provided material.
- When the manuscript has a clear technical domain, use claim-dependent domain gates as supporting checks, but keep the output inside the same 3-reviewer `nature-reviewer` structure.
- Do not claim the editor's final decision or certainty about fit to `Nature`.

## Accepted inputs

The skill may receive:

- full manuscript draft
- abstract, summary paragraph, or cover-summary style text
- introduction, results, discussion, or methods excerpts
- figure legends, selected figures, or result notes
- author notes in Chinese or English describing the claimed contribution
- pre-submission positioning notes

If the provided material is partial, perform a bounded review and mark the assessment boundary explicitly.

## Workflow

1. Run the bundled bounded extractor as the first and only discovery command:
   `python3 <skill-root>/scripts/prepare_review.py --source <manuscript-path>`.
   It emits a compact JSON packet containing every non-empty source line, stable `L<n>` anchors, measurable values, and the output contract. Do not replace it with `cat`, `find`, `ls`, repeated `read_file`, or ad-hoc parsing.
2. Treat that packet as the complete evidence boundary. Mark absent methods, uncertainty, controls, comparisons, citations, or scope as not assessable; never invent them.
3. Draft exactly six concise concerns, two per reviewer. Every concern must contain a stable ID, `Claim pointer`, `Evidence pointer`, and `Resolution test`. Both pointers must cite a valid packet anchor such as `L3`; quote the source when useful.
4. Write the complete report immediately with one `write_file` call to the requested path. Do not compose the report in thinking and postpone the write, and do not create a helper program.
5. The synthesis must name at least two concern IDs when describing consensus or weighting differences.
6. Run `python3 <skill-root>/scripts/validate_review.py --source <manuscript-path> --review <review-path>` as the final command. Stop after a successful structured receipt.

## Output format

Unless the user asks for another format, return:

```text
Review setup
- Input scope:
- Assessment boundary:
- Shared manuscript claim summary:
- Visible evidence base:
- Missing materials affecting confidence:

Reviewer 1
- Overall assessment:
- Who would be interested in the results, and why:
- Major strengths:
- Major concerns:
- Technical failings that need to be addressed before the case is established:
- Assessment against Nature-style criteria:
- Recommendation posture:

For each substantive concern:
- Concern ID: R1-M1
- Axis:
- Claim pointer:
- Evidence pointer:
- Concern and resolution test:

Reviewer 2
[Same structure]

Reviewer 3
[Same structure]

Cross-review synthesis
- Consensus strengths:
- Consensus technical risks:
- Where emphasis differs across reviewers:
- Broad-interest / significance readout:
- Most important issues to resolve before a strong Nature-style case is established:

Risk / unsupported claims
- [specific unsupported or not-assessable items]
```

## Red lines

- Do not invent reviewer identities, specialty roles, or selection history.
- Do not invent experiments, validations, controls, citations, figure details, line numbers, or prior-work distinctions not present in the input.
- Do not silently turn reviewer assessment into author rebuttal drafting.
- Do not present the review as an editorial decision letter.
- Do not state that the manuscript belongs in `Nature` as a settled fact.
- Do not omit technical failings when the provided evidence does not establish the authors' case.

## Related files

| File | Open when |
|---|---|
| [references/source-basis.md](references/source-basis.md) | You need source provenance, local rule summaries, or source-vs-implementation boundaries |
| [references/reviewer-workflow.md](references/reviewer-workflow.md) | You need the invocation order, fact-base extraction flow, or synthesis rules |
| [references/review-axes.md](references/review-axes.md) | You need the evaluation axes or reviewer weighting logic |
| [references/technical-concern-taxonomy.md](references/technical-concern-taxonomy.md) | You need the internal 12-axis coverage check, concern ledger, or claim/evidence-pointer rules |
| [references/domain-specific-review-gates.md](references/domain-specific-review-gates.md) | The manuscript has clear chemistry, engineering, materials, atmospheric, climate-ecology, hydrology, or remote-sensing evidence chains |
| [references/report-structure.md](references/report-structure.md) | You need the default output contract or section anatomy |
| [references/role-boundaries.md](references/role-boundaries.md) | You need constraints on reviewer differences and editor-versus-reviewer boundaries |
| [references/qa-checklist.md](references/qa-checklist.md) | You are finalizing an output and need groundedness / non-invention checks |
| [references/editorial criteria and processes.md](references/editorial criteria and processes.md) | You need the primary local Nature source text |

These references are progressive disclosure for unusual policy or domain-specific questions. Do not open any of them for the ordinary source-bounded pre-submission review path; the bounded extractor already supplies the stable review axes and output contract.

## Source hierarchy

Use sources in this order:

1. `references/editorial criteria and processes.md`
2. manuscript facts supplied by the user
3. conservative local implementation rules documented in `references/source-basis.md`
4. domain-specific supporting gates in `references/domain-specific-review-gates.md`

If a user asks for policy-level certainty beyond this local source, state the limit instead of improvising broader journal policy.

## Readiness

- Confirm that the requested source material, output goal, audience, and constraints are available before substantive work begins.
- State any missing evidence, inaccessible source, or assumption that could materially change the result.

## Workflow

1. Preserve the user's request and use the bundled extractor on the named manuscript.
2. Keep every concern traceable to the extractor's source-line anchors.
3. Write the bounded review before elaborating further, then run the bundled validator once.

## User control

- Keep analysis and drafts non-destructive by default.
- Confirm with the user before an external publication, submission, overwrite, or other consequential side effect.
- Never silently broaden the task, invent missing evidence, or replace an unresolved fact with a confident assertion.

## Validation

- Check completeness, internal consistency, evidence traceability, terminology, and output-format validity.
- Mark unresolved items and uncertainty explicitly instead of treating them as verified.
- Run `python3 scripts/validate_review.py --source <manuscript-path> --review <review-path>` after the final write. A successful read-back alone is not validation evidence.

## Completion

- Return the finished result, the validation status, material limitations, and any unresolved items.
- When an artifact is created, include its exact path and a concise description of what was produced.
