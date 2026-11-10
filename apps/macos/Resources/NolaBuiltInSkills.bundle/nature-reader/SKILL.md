---
name: nature-reader
description: "Build full-paper Chinese-English side-by-side, figure/table-aware, source-grounded Markdown readers for journal or conference papers from PDF, DOI, arXiv, publisher HTML, or pasted text. Use whenever the user asks to translate or read a paper, make 中英文对照/原文对照/全文翻译解读, extract figures or tables into the right positions, preserve figure/table placement near relevant prose, or keep exact source anchors for every block. This skill must not degrade into a summary-only output unless the user explicitly asks for a summary. Also trigger on general paper-reading and translation requests even without the word \"Nature\", such as reading/translating an academic paper, literature reading, understanding a paper, and Chinese phrasings like 读论文、精读论文、论文翻译、文献翻译、文献阅读、学术阅读、帮我读这篇文章、翻译这篇paper."
---

# Full-Paper Markdown Reader — Router

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before starting. Confirm required tools, runtime dependencies, permissions, and validation steps.
- All workflow and reference files are already bundled locally; never download or self-install a Skill.
- Load only the references selected by this task. Ask before external writes or private-source access, and stop at readiness if a required capability is unavailable.
- Never bypass authentication, paywalls, robots restrictions, CAPTCHAs, or browser security controls.


This skill is split into two layers:

- A **static layer** under `static/` that holds versioned, reusable content fragments (core principles, the reading workflow, the output contract, and per-source-format extraction guidance).
- A **dynamic layer** (this file plus `manifest.yaml`) that detects the request's source format and loads only the fragments needed for the current job.

Do not try to apply the reading logic from memory or from this router. Always load fragments from disk as described below.

## Routing protocol

Follow these five steps every time the skill is invoked.

### 1. Load the manifest and the core layer

Read [manifest.yaml](manifest.yaml). It declares the `source_format` axis, the allowed values, and the file paths each value maps to.

Also read every file listed under `always_load`. These hold the core principles, the reading workflow, and the output contract that apply to every reading job, plus the shared Terminology Ledger used to build the recurring-term table.

### 2. Detect the source format

Decide the `source_format` value using the manifest's `detect:` hint and the user's input:

- `pdf-text` — selectable-text PDF. Default.
- `scanned-pdf` — image-only or OCR-required PDF.
- `html` — publisher or preprint HTML page.
- `doi-arxiv` — a bare DOI or arXiv link that must be resolved first.
- `pasted-text` — pasted prose or notes with no retrievable original layout.

State the detected value in one short line to the user before processing, so they can correct you cheaply. A source may map to more than one value (for example a DOI that resolves to a PDF); load the resolution fragment first, then the fragment for the resolved artifact.

### 3. Load the matching fragment(s)

Read the file mapped for the detected `source_format`. Do **not** read every fragment in `static/`. Load only what step 2 selected.

### 4. Build the reader using the loaded material

Apply the loaded fragments in this priority order:

1. Core principles (`core/principles.md`) — bilingual reader by default, translate for meaning, never degrade to a summary, copyright caution.
2. Source-format fragment — how to extract text, figures, and tables for this input.
3. Reading workflow (`core/workflow.md`) — the six-step source-map-first process.
4. Output contract (`core/output-contract.md`) — required files and the pre-response verification checklist.

Build the Terminology Ledger as you translate (`../nature-shared/core/terminology-ledger.md`); it becomes the `paper.md` recurring-term table and the `source_map.json` glossary.

If constraints prevent full processing, still create a draft reader and label missing pages, figures, or low-confidence crops in `translation_notes.md`. Do not switch to summary mode.

### 5. Reach for references only when needed

The files under `references/` are deep references, not defaults. Open them on demand per the `references.on_demand` table in the manifest:

- detailed figure/table cropping and placement → `references/figure-extraction.md`.
- exact field schema for `paper.md` / `source_map.json` → `references/output-spec.md`.
- answering follow-up questions with source citations → `references/grounding-rules.md`.

## Why this split

- The static layer is versioned and reviewable. Adding a new source format is one new fragment plus one manifest line.
- The dynamic layer keeps each invocation cheap: only the fragment relevant to this input enters context.
- The router itself is short on purpose. Update fragments, not this file, when adding scope.
- This structure mirrors `nature-writing` and `nature-polishing` so shared content lives in `nature-shared/`.

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
