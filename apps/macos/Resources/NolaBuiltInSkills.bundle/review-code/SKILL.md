---
name: review-code
description: Read the diff and relevant implementation; test plausible failure paths; report severity, location, trigger and impact; avoid speculative findings and style-only noise.
---

# Code review

## Readiness

Review changes for concrete correctness, security, compatibility and maintainability defects. Read surrounding code and trace triggering conditions. Prioritize actionable findings with file locations, impact and reproducible evidence. Do not edit code unless asked. Distinguish verified findings from questions; say when no actionable defects were found.

Use this workflow when explicitly requested or selected for the whole task. A role name or an isolated word is not a tool-routing signal. Check actual tool schemas, configured Host, resource scope and permissions before execution. Never claim that a tool, Skill, test or artifact is available or complete without evidence. This package does not grant access or authorize external messages.

## Workflow

1. Inspect the user's deliverable, relevant source material and repository or workspace instructions. Ask only for constraints required to proceed.
2. Read the diff and relevant implementation; test plausible failure paths; report severity, location, trigger and impact; avoid speculative findings and style-only noise.
3. Select the narrowest available tool from its schema. Discover deferred operations with tool_search when needed. Keep operations inside the approved resources; a missing tool cannot be repaired by pretending the work succeeded.
4. Deliver concrete findings with relevant evidence and state any failed, skipped or untested checks. Preserve user drafts and unrelated changes.

## User control

- Code review is read-only by default; confirm with the user before applying fixes, mutating files, installing dependencies, or changing credentials.
- On missing Host access, rejected permission or failed validation, explain the concrete missing capability and how to resume; retain completed findings.

## Validation

- Verify each reported finding independently against surrounding call sites, contracts, and reproducible trigger conditions; separate verified defects from open questions.
- Run non-destructive checks where applicable and do not treat speculative concerns as confirmed bugs.

## Completion

Before the final answer, use `skill_update` to record the required progress and verifier results from the active Core runtime facts. Use their exact run, step and verifier IDs; cite real retained tool-call or execution-evidence sources for verification. Record only observations that are true, combine independent updates when possible, and preserve pending records if a tool or approval is unavailable. Core closes a satisfied contract. A final sentence, success marker or passing command alone does not close the Skill contract.
