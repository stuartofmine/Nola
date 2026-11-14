---
name: test-software
description: Define expected behavior independently of implementation, reproduce failures, run meaningful checks with isolated fixtures, inspect actual UI where applicable, and report passed, failed, skipped and untested scope.
---

# Software verification

## Scope and available capabilities

Design and execute tests based on user journeys, contracts, risks and failure recovery. Cover asynchronous ownership, persistence, permissions and cross-platform behavior where relevant. Use isolated data and explicit oracles. Keep unit, fixture, real model, device and visual evidence separate; skipped or untested paths are not passed.

Use this workflow when explicitly requested or selected for the whole task. A role name or an isolated word is not a tool-routing signal. Check actual tool schemas, configured Host, resource scope and permissions before execution. Never claim that a tool, Skill, test or artifact is available or complete without evidence. This package does not grant access or authorize external messages.

## Workflow

1. Inspect the user's deliverable, relevant source material and repository or workspace instructions. Ask only for constraints required to proceed.
2. Define expected behavior independently of implementation, reproduce failures, run meaningful checks with isolated fixtures, inspect actual UI where applicable, and report passed, failed, skipped and untested scope.
3. Select the narrowest available tool from its schema. Discover deferred operations with tool_search when needed. Keep edits and operations inside the approved resources; a missing tool cannot be repaired by pretending the work succeeded.
4. Verify the actual outcome independently: compare against the request, inspect changed files or the generated image, and run appropriate project checks where applicable. Do not treat compilation as proof of UI behavior or a text prompt as an image.
5. Deliver the artifact or concrete findings with relevant evidence and state any failed, skipped or untested checks. Preserve user drafts and unrelated changes.

## Failure recovery

- On missing Host access, unavailable image generation, rejected permission or failed validation, explain the concrete missing capability and how to resume; retain completed work.
- Do not install external dependencies or change credentials without authorization.
- Re-read current snapshots before retrying writes; do not duplicate committed side effects or silently broaden resource scope.


## Record completion

Before the final answer, use `skill_update` to record the required progress and verifier results from the active Core runtime facts. Use their exact run, step and verifier IDs; cite real retained tool-call or execution-evidence sources for verification. Record only observations that are true, combine independent updates when possible, and preserve pending records if a tool or approval is unavailable. Core closes a satisfied contract. A final sentence, success marker or passing command alone does not close the Skill contract.
