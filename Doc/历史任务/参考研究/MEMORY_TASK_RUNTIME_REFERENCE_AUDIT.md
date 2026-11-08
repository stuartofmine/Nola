> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Memory and Task Runtime Reference Audit

## Scope

This audit compares Nola's memory and scheduled-task runtime with the checked-in Codex, OpenCode, and Claude Code reference projects. It focuses on lifecycle contracts rather than UI resemblance or keyword-based routing.

## Executive findings

Nola already has useful pieces, but they currently stop at platform boundaries:

1. Core can now decide whether a structured memory is eligible by namespace, confidence, retention, age, expiry, and supersession.
2. CLI memory search now sends its structured candidates through that Core decision before returning results.
3. iOS, macOS, and Android now migrate their legacy long-term-memory formats into structured records and pass candidates through Core eligibility selection.
4. Core now emits a redacted `memory.used` trace from the context components that survived token budgeting. The event carries record/component identity, namespace, provenance source, observation time, verification requirement, and truncation state, but never memory content or hidden reasoning.
5. macOS scheduled turns now declare `scheduled_action` as their input origin, and Core prevents them from inheriting conversation- or turn-scoped grants.
6. Android scheduled work now fetches through the data-source reader, attaches the snapshot as typed reference context, and executes the configured prompt as a `scheduled_action` through the same Agent/Core lifecycle. Permission requests persist as `needs_attention` without being auto-denied; resume UI remains pending.

The next phase must migrate storage and background execution, not add more prompt prose.

## Reference-project lessons

### Codex: memory is a durable two-stage job system

Codex does not treat memory as one mutable summary string.

- Stage 1 stores per-thread extraction output together with the source rollout timestamp, rollout path, summary, working directory, branch, and generation timestamp.
- Stage 1 claims use ownership tokens, leases, retry backoff, retry exhaustion, source freshness, scan limits, and worker limits.
- Phase 2 is a singleton consolidation job with an input watermark and a last-success watermark.
- Completion advances the watermark transactionally and records exactly which Stage 1 snapshots were selected.
- Usage is measured for cited Stage 1 outputs rather than inferred from prompt injection alone.
- A stale worker cannot finalize another worker's job because success and heartbeat require the matching ownership token.

Nola should absorb the job semantics, not copy the database names.

### Claude Code: consolidation is gated, isolated, observable, and abortable

Claude Code's background consolidation uses cheap gates before expensive model work:

- feature and mode eligibility;
- elapsed-time threshold;
- number of new sessions;
- scan throttling;
- a consolidation lock;
- an abort controller;
- a forked agent with restricted read-only tools;
- progress events that show reasoning summaries and tool-use counts;
- rollback behavior when the task is killed.

Its scheduled prompts enter a command queue with lower priority and are drained between turns. A scheduled prompt is not modeled as an interactive user's live authorization.

Nola should use the same separation: scheduling decides when work may be enqueued; Core still evaluates the concrete turn origin, tools, resources, and permissions.

### OpenCode: context capacity and permission requests are runtime state

OpenCode calculates usable context from the model's declared input/context limits and reserves output capacity. Overflow is based on actual input, output, and cache token accounting.

Its permission dock replies to a pending permission event with `once`, `always`, or `reject`. The UI does not independently continue the tool or manufacture authorization state.

Nola should keep the permission request suspended in Core/Host and make every client a renderer and responder for the same event contract.

## Current Nola platform matrix

| Capability | Core | CLI | macOS | iOS | Android | Web |
|---|---|---|---|---|---|---|
| Structured memory candidate | Yes | Yes | Yes, with one-time legacy import | Yes, with immediate idempotent legacy import | Yes, with legacy-list migration | Reuses CLI paths inconsistently |
| Central eligibility gate | Yes | Production search path | Connected to stored long-term memory | Connected to profile storage | Connected to stored long-term memory | Inherits only when using CLI search path |
| Provenance retained | Yes | Yes | Yes for structured records | Yes; manual, imported and assistant-tool sources remain distinct | Yes for structured records | Partial |
| Expiry and maximum age | Yes | Mapped when present | Mapped into Core selection | Mapped into Core selection | Mapped into Core selection | Partial |
| Supersedes relation | Yes | Mapped when present | Mapped into Core selection | Mapped into Core selection | Mapped into Core selection | Partial |
| Current-state verification flag | Yes | Returned with selected result and trace | Preserved in injected context and message trace | Preserved in trace projection | Preserved in trace projection | Rendered from canonical trace |
| Background input origin | Yes | Parsed by Agent Host | Emits `scheduled_action` | Scheduled URL reports execute through Agent/Core | Scheduled Worker executes through Agent/Core with typed reference context | Pending |
| Background grant isolation | Yes | Yes through Core | Yes through Agent Host | Core scheduled origin prevents borrowing live-turn grants | Permission requests suspend as `needs_attention`; no interactive grant reuse or automatic denial | Pending |
| Durable job lease/watermark | Contract implemented | SQLite claim, heartbeat, retry and ownership-safe finish | Launchd runner has atomic lease, owner token, heartbeat and scheduled-success watermark | Pending | Stable occurrence IDs and bounded retry; durable cross-process lease remains pending | Uses CLI runtime but restore parity remains incomplete |
| Memory citation usage tracking | Canonical redacted `memory.used` event | Persisted in Trace | Decoded and shown in message evidence | Preserved in Trace projection | Preserved in Trace projection | Persisted, restored, and rendered |

## Required shared memory schema

Every platform-owned store must persist enough information to produce a Core `MemoryRecord` without inventing values at read time.

Required fields:

- stable record ID;
- namespace and scope;
- memory kind;
- statement or content;
- confidence;
- provenance kind and source reference;
- observation timestamp;
- creation and update timestamps;
- retention policy;
- optional expiry;
- optional superseded record IDs;
- tags;
- optional content hash and previous-version reference.

Legacy strings may be migrated as `imported` provenance with an explicit migration timestamp and conservative confidence. They must not be mislabeled as direct user statements or fresh observations.

## Required background-job schema

Memory extraction, consolidation, scheduled Agent turns, and long-running data-source refreshes should share a durable job envelope:

- job ID and kind;
- input origin;
- source watermark;
- status: queued, running, suspended, succeeded, failed, cancelled;
- worker ID and ownership token;
- lease expiry and heartbeat timestamp;
- retry count, retry time, and last error;
- progress summary safe for UI display;
- structured artifacts and citations;
- cancellation request timestamp;
- completion watermark.

Permission suspension is not failure. A job waiting for a decision remains `suspended` and resumes from its checkpoint after Core accepts the response.

## Migration sequence

### 1. Finish the shared boundary

- Expose Core memory selection through the C and JNI boundaries.
- Add Swift and Kotlin DTOs that encode the exact Core schema.
- Keep the selection result structured, including reject reasons and `requires_current_state_verification`.

### 2. Migrate stores without losing legacy data

- CLI: retain the current structured SQLite store and add content-hash/version fields.
- macOS: replace `UserDefaults` long-term-memory strings with a structured local store; import the old string once.
- iOS: move long-term memory out of the profile string and preserve conversation compaction as a separate namespace.
- Android: replace `List<String>` with structured serialized records or a Room table; keep compacted conversation summaries separate.
- Web: use the same CLI/Core record and selection contract rather than a browser-only memory format.

### 3. Separate retrieval from applicability

Retrieval may rank candidates lexically, semantically, or by a provider index. Core then applies authorization, namespace, retention, freshness, supersession, and limit rules. Neither stage may infer tool authorization from words in the user's prompt.

### 4. Add extraction and consolidation jobs

- Stage 1 extracts candidate facts from completed conversations.
- Stage 2 consolidates candidates, resolves contradictions, writes supersession links, and retains provenance.
- Both stages use leases, bounded concurrency, retries, watermarks, and restricted tools.
- Model-produced output is validated before commit.

### 5. Make memory legible

- Show which memories were used in a response from the canonical `memory.used` event, never from client-side inference.
- Allow opening full provenance and version history; the current Trace projection exposes safe source and observation metadata but not yet the management destination.
- Mark mutable snapshots that require current-state verification; Web renders the marker and native projections retain it for their final UI treatment.
- Support correcting, disabling, deleting, and viewing version history.

## Acceptance tests

### Core contract tests

- low-confidence memory is rejected;
- expired and over-age memory is rejected;
- a superseded record is rejected even when ranked first;
- session-only memory is excluded outside its session;
- mutable tool/imported snapshots require verification;
- selection order remains the retriever's order;
- scheduled and replay turns cannot consume interactive turn grants.

### Store migration tests

- legacy strings are imported once and remain readable;
- provenance is never upgraded during migration;
- interrupted migration is idempotent;
- deleting or correcting a record writes deterministic supersession/version metadata;
- all clients decode records written by the shared schema.

### Real-model end-to-end tests

- a natural multilingual follow-up recalls an applicable user preference without keyword routing;
- an outdated project fact is either excluded or visibly marked for verification;
- a scheduled task cannot reuse a conversation-only file or shell grant;
- a permission request suspends and resumes the same tool call exactly once;
- consolidation does not duplicate records when retried after lease expiry;
- the UI exposes cited memories and intermediate task progress without leaking hidden reasoning.

## Non-goals

- Do not select tools or permissions with language-specific keyword lists.
- Do not inject every stored memory into every prompt.
- Do not collapse conversation compaction, user preferences, project facts, and imported data into one string.
- Do not let a client UI manufacture grants, completion events, or memory freshness.
- Do not run consolidation inside the interactive turn without job isolation and cancellation.
