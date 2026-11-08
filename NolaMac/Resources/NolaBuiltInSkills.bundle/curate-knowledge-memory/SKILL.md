---
name: curate-knowledge-memory
description: Propose, store, organize, merge, correct, expire, retrieve, export, or delete durable personal knowledge and cross-conversation memory. Use when information should remain useful beyond the current conversation or the user asks what is remembered; never persist sensitive information without explicit consent.
---

# Curate Knowledge and Memory

## Nola runtime contract

- Use the manifest and execution contract returned by the `Skill` invocation. Read `NOLA_MANIFEST.json` only when that returned contract is absent or reports a digest/version mismatch; do not search the user's workspace for the bundled manifest.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Accept durable facts from other Skills only with provenance and scope. Normal internal memory writes do not request filesystem or shell permission; sensitive memory still requires explicit consent.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Propose**: identify useful durable candidates without writing them.
- **Remember**: store approved stable facts, preferences, commitments, relationships, or reusable knowledge.
- **Reconcile**: merge duplicates, supersede stale facts, and resolve user corrections.
- **Recall**: retrieve relevant records with provenance and uncertainty.
- **Forget or export**: remove or return user-selected durable records and verify scope.

## Readiness

- Check conversation-level cross-session memory setting, tool authorization, and user identity scope.
- Separate transient context, current task state, inferred beliefs, and durable user-provided knowledge.
- For every candidate, capture subject, fact, provenance, observed-at time, confidence, sensitivity, and intended retrieval scope. Set an expiry or review date only when the user or source explicitly supplied one; age alone is never evidence that a durable historical fact expires.

## Progress contract

Use stable stages internally: identifying candidates, checking existing memory, resolving conflicts, requesting consent, writing changes, and verifying memory. The applicable stages depend on the operating mode, so do not create synthetic `skill_update` records for them. The actual memory tool calls and their canonical receipts are the execution record. Do not expose private memory content in generic progress text.

## Workflow

1. Extract minimal atomic candidates in the user's language without adding inference. Preserve the exact event predicate and decisive qualifiers: for example, keep “started,” “finished,” “increased,” or “decreased” instead of reducing the statement to a broad topic. Resolve explicit relative dates against a dated source only when the result is unambiguous, and retain the source wording in the record.
2. Search existing memory once with a focused query and at most five results for semantic duplicates, contradictions, stale versions, and more authoritative user corrections. Refine the query only when the first result identifies one concrete unresolved conflict; do not expand a truncated broad result or repeat an empty write-after-read query.
3. Score durability: likely useful later, stable enough, attributable, not already stored, and appropriate for persistence.
4. Show material or sensitive candidates and intended scope before writing. Prefer merge or supersede over near-duplicate creation.
5. Store concise records with provenance, observed-at time, and sensitivity. Omit expiry unless the source explicitly declares it. Link corrections to superseded records when supported. When several independent records are ready, issue their `memory_save` calls together in one parallel tool response instead of consuming one model round per record.
6. On recall, distinguish remembered data from current source data and never let stale memory override a fresh authoritative source.

## User control

- Long-term memory is an internal product capability, not an external side effect. Saving a normal, explicitly user-provided durable preference does not require a separate system permission prompt; still report what was stored and support immediate correction or deletion.
- Require explicit consent before storing health, identity, financial, intimate, location-pattern, or third-party personal information.
- Never store passwords, authentication secrets, private keys, payment credentials, or highly sensitive inferred traits.
- Honor correction, export, forgetting, and disable requests immediately. Do not recreate deleted memory from summaries or backups exposed to the agent.

## Validation

- Treat the canonical normalized record and artifact URI returned by a successful `memory_save` as synchronous read-back evidence for that write. Do not pass a `nola://memory/...` URI to a filesystem reader.
- When a declared verifier applies, use its declared method and cite the actual completed `memory_save` tool call as a structured evidence source. An artifact URI alone is not execution proof. A plain remember operation with no declared verifier needs no invented verification update.
- Confirm corrections supersede stale records and deletions remove every targeted durable record exposed by the tool.
- For recall, reconcile, deletion, or scope-isolation work, test one representative query after the final change. A plain remember operation does not require a second search when its canonical save receipt already proves the record and scope.

## Failure handling

If memory state, identity scope, or consent is ambiguous, keep candidates in the current conversation only. On partial deletion, enumerate remaining record identifiers and do not claim forgetting is complete.

## Completion

State exactly what was proposed, stored, merged, superseded, expired, exported, or deleted, including record identifiers when available. If nothing was persisted, say so explicitly.
