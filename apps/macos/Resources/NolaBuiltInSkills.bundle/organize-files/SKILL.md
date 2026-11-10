---
name: organize-files
description: Inventory, classify, rename, copy, move, deduplicate, archive, or quarantine files within an approved workspace. Use only when the intended deliverable concerns file organization or path changes; do not use for merely reading, summarizing, or reviewing file contents. Use for downloads cleanup, project organization, naming normalization, duplicate analysis, and archival; require a dry-run manifest before mutations.
---

# Organize Files

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use document review only when classification depends on document meaning. File organization owns path mutation and must receive explicit approved manifest rows rather than prose suggestions.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Inventory**: report structure, types, sizes, age, duplicate candidates, and anomalies without changes.
- **Plan**: produce a dry-run manifest and collision policy.
- **Execute**: apply only approved manifest rows in reversible batches.
- **Archive or deduplicate**: preserve recoverability and prove equivalence before removing originals.

## Readiness

- Establish exact allowed roots, exclusions, desired taxonomy, naming convention, retention policy, and destination capacity.
- Do not traverse unrelated home, system, cloud, hidden, package, repository, or backup directories without explicit scope.
- Inventory paths, types, sizes, timestamps, hashes when needed, symlinks, aliases, packages, permissions, collisions, and available disk space.
- Distinguish byte-identical duplicates from similar filenames or similar content.

## Progress contract

Use stable stages: inventorying files, building dry-run manifest, waiting for approval, executing batch, verifying destinations, and writing result manifest. Report batch counts rather than every unchanged file.

## Workflow

1. Produce a dry-run manifest with source, proposed destination or name, action, reason, collision strategy, reversibility, and status.
2. Normalize paths and reject traversal outside approved roots. Do not follow untrusted symlinks.
3. Group operations into bounded batches. Prefer copy-verify-delete for cross-volume moves and duplicate cleanup.
4. Use deterministic collision handling: skip, versioned suffix, or user-approved replacement. Never silently overwrite.
5. Preserve extensions, metadata, package integrity, relative references, and repository boundaries where relevant.
6. Write a result manifest mapping every source to final path, checksum or size, status, and error.

## User control

- Treat an explicit imperative to organize, move, rename, or archive files inside a named folder as approval for reversible operations inside that exact scope. Do not ask a redundant second confirmation merely because the deterministic manifest has now been generated.
- Always write and validate the dry-run manifest before either executing or requesting additional approval; planning must leave a reviewable artifact even when execution cannot continue.
- Request additional confirmation only when the proposed manifest expands beyond the named scope, deletes data, overwrites an existing destination, encrypts an archive, or contains an ambiguous bulk rename that the user did not already request.
- Quarantine uncertain files instead of deleting them and preserve originals when validation is incomplete.
- Never use broad destructive shell globs or hide individual failures inside a successful batch summary.

## Validation

- Reconcile planned, succeeded, skipped, failed, and rolled-back counts.
- Verify destination existence, size and hash for copied files, collision handling, permissions, broken symlinks, package integrity, and absence of unintended changes.
- For deduplication, prove hash equality and retain at least one verified readable copy before any removal.
- Write the temporary plan input to the exact hidden temporary path `.organize-input.tmp`. Use `scripts/organize_files.py --plan .organize-input.tmp --manifest <requested-final-manifest.json>` for deterministic dry runs, then `--execute --manifest <requested-final-manifest.json>` only after approval. If the user names a manifest such as `move-manifest.json`, that exact path is the authoritative manifest from the first `--plan` call onward; never create a temporary manifest and later reconstruct, rename, summarize, or replace it by hand. The script-owned manifest records hashes, resolved collision destinations, rejected symlinks, and inverse operations.
- After execution, remove only temporary input specifications, then run `scripts/organize_files.py --validate-only --manifest <requested-final-manifest.json> --expected-state executed` against the same authoritative manifest as the final mutating-workflow gate. Do not delete, rewrite, regenerate, or otherwise mutate the manifest or workspace afterward. A successful `--execute` result, manual hash check, or hand-authored `validated: true` field is not completion evidence.
- Verify reversibility with `scripts/organize_files.py --rollback --manifest <manifest.json>` and `--validate-only --expected-state rolled_back` before reporting a rollback as complete.

## Failure handling

Stop the current batch on an unexpected scope expansion, collision-policy violation, disk exhaustion, or verification mismatch. Preserve completed work, roll back only operations with a recorded inverse, and report exact paths.

## Completion

Report counts, manifest and rollback information, final locations, exceptions, quarantined items, and any cleanup that still requires confirmation.
