---
name: test-software
description: Define expected behavior independently of implementation, reproduce failures, run meaningful checks with isolated fixtures, inspect actual UI where applicable, and report passed, failed, skipped and untested scope.
---

# Software verification

## Readiness

Design and execute tests based on user journeys, contracts, risks and failure recovery. Cover asynchronous ownership, persistence, permissions and cross-platform behavior where relevant. Use isolated data and explicit oracles. Keep unit, fixture, real model, device and visual evidence separate; skipped or untested paths are not passed.

Use this workflow when explicitly requested or selected for the whole task. A role name or an isolated word is not a tool-routing signal. Check actual tool schemas, configured Host, resource scope and permissions before execution. Never claim that a tool, Skill, test or artifact is available or complete without evidence. This package does not grant access or authorize external messages.

## Workflow

1. Inspect the user's deliverable, relevant source material and repository or workspace instructions. Ask only for constraints required to proceed.
2. Define expected behavior independently of implementation, reproduce failures, run meaningful checks with isolated fixtures, inspect actual UI where applicable, and report passed, failed, skipped and untested scope.
3. Select the narrowest available tool from its schema. Discover deferred operations with tool_search when needed. Keep edits and operations inside the approved resources; a missing tool cannot be repaired by pretending the work succeeded.
4. Deliver the test artifacts and concrete findings with relevant evidence and state any failed, skipped or untested checks. Preserve user drafts and unrelated changes.

## User control

- Always use isolated test fixtures and temporary directories; confirm with the user before touching live databases, installing dependencies, changing credentials, or running against external services.
- On missing Host access, rejected permission or failed execution, explain the concrete missing capability and how to resume; retain completed test work.

## Validation

- Verify the actual outcome independently: ensure newly added tests fail on the unpatched defect or validate explicit behavioral oracles, and do not treat compilation alone as proof of UI or runtime behavior.
- Report exact passed, failed, skipped and untested scope separately.

## Completion

Before the final answer, report the exact scope that passed, failed, or was skipped, grounded in real command output.
