---
name: nature-response
description: "Draft, audit, or revise Nature-style revision correspondence packages: point-by-point reviewer response letters, rebuttal letters, revision cover letters, LaTeX cover/response templates, and red-marked revised-manuscript excerpts. Use for reviewer comments, editor decision letters, pasted editorial emails, response drafts, cover letters, response to reviewers, rebuttal, 修回信, 返修邮件, 编辑邮件, 返修 cover letter, 审稿意见回复, 逐点回复, 大修回复, 小修回复, 回复审稿人, 修改稿回复, 写rebuttal, 回应审稿意见, 标红修改, or LaTeX 模板."
---

# Nature Reviewer Response — Router

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before starting. Confirm required tools, runtime dependencies, permissions, and validation steps.
- All workflow and reference files are already bundled locally; never download or self-install a Skill.
- Load only the references selected by this task. Ask before external writes or private-source access, and stop at readiness if a required capability is unavailable.
- Never bypass authentication, paywalls, robots restrictions, CAPTCHAs, or browser security controls.


This skill is split into two layers:

- A **static layer** under `static/` that holds versioned, reusable content fragments (the default stance and red lines, and the response workflow with output format).
- A **dynamic layer** (this file plus `manifest.yaml`) that loads the core every time and reaches for the deeper response references or templates only when a step needs them.

Do not try to apply the response logic from memory or from this router. Always load fragments from disk as described below.

## Routing protocol

Follow these four steps every time the skill is invoked.

### 1. Load the manifest and the core layer

Read [manifest.yaml](manifest.yaml). Then read every file listed under `always_load`:

- `static/core/stance.md` — the editor-facing purpose, the default stance, the red lines, and the source hierarchy that apply to every response job.
- `static/core/workflow.md` — accepted inputs, the revision correspondence workflow, and the output package format.

### 2. No content axis — identify mode and language inline

Unlike nature-writing or nature-figure, nature-response has no fragment axis. Its variation is identified at runtime, not by loading different content bodies:

- **task mode** — `draft` / `audit` / `revise` / `triage-only` / `cover-letter` / `revision-package` / `latex-template` / `appeal-like`.
- **decision type** — minor revision, major revision, revise-and-resubmit, transfer after review, or unclear.
- **user language** — if the user writes Chinese, also produce the 中文核对 block.

Use `references/intake-and-routing.md` to fix the task mode, minimum inputs, and readiness state before drafting. Route appeal-like cases separately; do not draft an appeal as the default path.

### 3. Run the workflow

Follow the workflow in `core/workflow.md`: if the user pasted a journal email, first parse manuscript metadata, decision type, editor instructions, reviewer reports, required files, and deadlines from the email; identify mode and decision type; extract editor instructions (IDs `E.1`) then reviewer comments (`R1.1`, `R2.1`) when present; classify each item by response action and independently verified work status; build a strategy summary; draft point-by-point responses and/or a revision cover letter; map every claimed change to a manuscript location or explicit placeholder; mark changed manuscript text in red on a backed-up copy when editing; format quoted revised manuscript text in the response letter in italics; start each new reviewer response on a new page in LaTeX/print-oriented outputs; flag missing author input; run QA; and derive package readiness from the per-item statuses and blocking state.

Never invent experiments, citations, line numbers, figure panels, supplementary items, editor instructions, or manuscript changes. Mark anything the author must supply as `AUTHOR_INPUT_NEEDED`.

### 4. Reach for references only when needed

The files under `references/` and `templates/` are deep resources, not defaults. Open them on demand per the `references.on_demand` table in the manifest — for example `references/comment-taxonomy.md` to classify comments, `references/action-mapping.md` for tracker fields, `references/tone-and-stance.md` for disagreement wording, `references/difficult-cases.md` for impossible experiments / conflicting reviewers / appeal-like cases, `references/chinese-author-alignment.md` for Chinese author notes, `references/latex-templates.md` for `.tex` cover/response/redline outputs, and `references/qa-checklist.md` before finalizing.

## Why this split

- The static layer is versioned and reviewable; the core stays small for a normal response.
- The dynamic layer keeps each invocation cheap: the difficult-case, taxonomy, and QA depth load only when a step needs them.
- The router itself is short on purpose. Update fragments and references, not this file, when adding scope.
- This structure mirrors `nature-writing`, `nature-polishing`, `nature-reader`, `nature-paper2ppt`, `nature-figure`, and `nature-citation`.

## Readiness

- Confirm that the requested source material, output goal, audience, and constraints are available before substantive work begins.
- State any missing evidence, inaccessible source, or assumption that could materially change the result.

## Workflow

1. Preserve the user's original request and identify the smallest set of relevant source material.
2. Apply this Skill's domain workflow and load only the shared references explicitly needed for the current step.
3. Keep claims, transformations, and intermediate decisions traceable to their source evidence.
4. Produce the requested deliverable in the user's language and requested format.

## User control

- Keep analysis and drafts non-destructive by default.
- Confirm with the user before an external publication, submission, overwrite, or other consequential side effect.
- Never silently broaden the task, invent missing evidence, or replace an unresolved fact with a confident assertion.

## Validation

- Check completeness, internal consistency, evidence traceability, terminology, and output-format validity.
- Mark unresolved items and uncertainty explicitly instead of treating them as verified.
- Re-open or re-render generated artifacts when the format supports deterministic validation.

## Completion

- Return the finished result, the validation status, material limitations, and any unresolved items.
- When an artifact is created, include its exact path and a concise description of what was produced.
