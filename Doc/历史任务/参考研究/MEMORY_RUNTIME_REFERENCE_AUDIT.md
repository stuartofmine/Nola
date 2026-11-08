> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Memory Runtime Reference Audit

## Scope

This audit compares Nola's checked-in memory implementation with the checked-in Codex and
OpenCode references. It separates four concerns that were previously conflated:

1. selecting existing memories for one Turn;
2. extracting durable signal from completed conversations;
3. consolidating overlapping or stale records;
4. showing which memories were written or used, with evidence.

## Reference findings

### Codex

Codex treats memory production as a durable two-stage pipeline rather than a prompt flag.

- Stage one claims eligible completed rollouts, applies source/age/idle filters, limits
  concurrency, requests structured extraction output, redacts secrets, and checkpoints the
  result with retry metadata.
- Stage two obtains one global consolidation lease, selects a bounded input set using recency and
  actual usage, materializes stable raw artifacts, runs an isolated consolidation agent, and only
  commits output that attests the frozen input watermark.
- Consolidated memory remains evidence-addressable. Citations carry concrete file paths and line
  ranges plus rollout/thread identifiers.
- The read path records use separately from storage. Retention can therefore use actual
  `last_usage` instead of assuming that every stored memory remains valuable.
- Clearing a memory root rejects symlinks and avoids recursively deleting an attacker-controlled
  target.

### OpenCode

OpenCode does not provide an equivalent cross-conversation memory subsystem, but its session
runtime contributes useful adjacent constraints:

- session summaries and diffs are typed persisted state rather than transient UI labels;
- child work keeps stable IDs and parent relationships;
- cancellation propagates from the owning session;
- retry state exposes attempt, message, and next retry time;
- context overflow is classified separately from retryable provider failure.

## Nola before this change

Nola already had useful policy primitives:

- typed scope, namespace, kind, retention, sensitivity, confidence, importance, supersession, and
  provenance;
- deterministic selection and rejection reasons;
- a `MemoryUsage` record for memories that survive context budgeting;
- Core-owned CRUD that does not request user permission for an internal memory write;
- a guardrail that ordinary user text never becomes a memory mutation through keyword matching.

The missing layer was the runtime between a completed conversation and those records:

- `automatic_extraction_enabled` was a policy boolean without a shared extraction job;
- there was no global consolidation lease or frozen input/completion watermark;
- CRUD had no operation-level idempotency envelope;
- there was no durable usage ledger or citation contract;
- `MemoryUpdated` was dropped from Agent Trace, preventing an auditable client history;
- platform wrappers selected records but independently owned policy defaults and persistence.

## Core contract added

`core/src/agent/memory_runtime.rs` now defines a platform-neutral, checkpointable contract.

- Extraction and consolidation are distinct job kinds.
- Every command is wrapped in an immutable `operation_id`; replay is idempotent.
- Claims use a worker ID, ownership token, heartbeat, and finite lease.
- Retry preserves the input watermark, accepts an explicit Retry-After delay, and otherwise uses
  bounded exponential backoff.
- Lease sweeping reclaims abandoned work instead of waiting for a child process to exit forever.
- Successful completion must use the same watermark as the frozen input and the output kind must
  match the claimed job.
- Only one non-terminal consolidation job may exist at a time.
- Extraction output carries structured records, citations, a rollout summary, output digest, and
  secret-redaction count.
- Consolidation output carries resulting records, citations, superseded IDs, summary, and digest.
- A usage ledger records the session, Turn, context digest, time, and citations for records that
  were actually injected.

Core does not call an LLM or write a database. Hosts execute effects and persist the serialized
projection. This preserves the local-first platform boundary while making behavior consistent.

## Trace visibility

`MemoryUpdated` now becomes a `memory.updated` Agent Trace event. The reducer retains sanitized
updates in `AgentRunProjection.memory_updates`. Clients can use this to show a small, localized
history such as "saved preference", "updated project fact", or "forgot memory" without exposing
raw transport labels.

## Session integration

`MemoryRuntimeProjection` is now serialized inside the authoritative Agent
Session checkpoint and exposed in `AgentCoreSnapshot`. Hosts submit idempotent
`MemoryRuntimeCommandEnvelope` values through `AgentSubmission::MemoryRuntime`.
Extraction/consolidation ownership, retry deadlines, completion watermarks and
usage accounting therefore survive process replacement together with the
conversation rather than living in a platform-private queue.

This integration deliberately does not make Core perform I/O. A Host claims a
job, executes the restricted model workflow, and submits the resulting state
transition; Core validates ownership and frozen-input attestation before the
checkpoint can advance.

CLI `session show --json`, the Web conversation endpoint, and the macOS session
decoder now carry the same serialized runtime projection. Reconnect therefore
preserves queued, running and retry-waiting work instead of inferring it from
tool traces or treating it as completed when the Host process exits.

The CLI Agent Host protocol also defines
`core_memory_runtime_projection_updated`, parallel to the existing Core
permission projection event. It carries the authoritative revision and full
checkpointed projection; native clients must reduce by revision rather than
turning individual transport lines into ad-hoc job state.

On restore, the checkpoint's frozen memory policy remains authoritative. Host
defaults and environment-derived settings apply only when creating a session;
changing an existing conversation must be an explicit idle Core configuration
transition. This avoids making a valid checkpoint unreadable merely because a
UI preference changed between process launches.

When a completed root Turn has both long-term memory and automatic extraction
enabled, Core now snapshots the authoritative transcript identity and enqueues
one deterministic extraction job. The transcript digest is the frozen source
watermark, so replaying completion cannot duplicate work and a worker cannot
silently process a different transcript. Core deliberately records no wall
clock at enqueue time; the authenticated Host provides real time when claiming
the job rather than making a reducer depend on platform time.

## Legacy migration boundary

iOS and Web still contain legacy parsers that recognize hidden memory payloads
inside assistant text. Those parsers are not an acceptable automatic-extraction
transport: model prose must never become an internal command merely because it
contains a reserved tag. They must be retired after the restricted Host worker
lands, not copied into macOS, Android, CLI, or Core. During migration:

- explicit `memory_save` remains the only interactive write path;
- automatic extraction consumes a claimed `MemoryRuntimeProjection` job and
  returns a typed `MemoryExtractionOutput`;
- user-visible assistant text is never parsed for memory mutations;
- the worker cannot call memory tools recursively;
- job completion and record persistence share the frozen input watermark and
  idempotent operation IDs.

## Remaining Host work

The contract is intentionally ahead of platform adapters. Remaining work should follow this order:

1. persist the Agent checkpoint containing `MemoryRuntimeProjection` beside each platform's memory database;
2. execute extraction and consolidation through the authenticated Agent Host with restricted
   capabilities and no recursive memory access;
3. persist citations and usage ledger entries;
4. derive retention ordering from usage stats and record timestamps;
5. project `memory.updated` and citations into native Mac, iOS, Android, CLI, and Web UI;
6. remove platform-hardcoded memory policy defaults after session configuration is authoritative.

Host completion must be driven by the durable job state, not process exit or pipe EOF. Academic or
other long-running work may have a larger execution budget, but a completed job must never remain
blocked because a reusable Host process is still alive.
