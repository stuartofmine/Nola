> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Memory Recall Architecture

## Status

This document separates the implemented memory pipeline from the target recall contract. It is
not evidence that the target contract is already available.

## Reference comparison

### Codex

The checked-in Codex memory implementation uses two write stages and progressive disclosure:

- Stage 1 extracts bounded rollout-level evidence.
- Stage 2 consolidates it into a compact always-loaded summary, a searchable handbook, rollout
  summaries, and optional reusable skills.
- The summary is an index, not a copy of memory content.
- Detailed memory is retrieved explicitly and returned with file and rollout citations.
- Input selection, ownership, watermarks, retention, metrics, and failure outcomes are persisted.

The main idea Nola should absorb is not the file format. It is the separation between a very
small routing layer and evidence that is loaded only when the current task needs it.

### Claude Code

The checked-in Claude prompts use a compact index plus topic files. Consolidation is explicitly
incremental: inspect the index, gather recent signal, merge rather than duplicate, convert relative
dates to absolute dates, remove contradicted facts, and prune stale index entries. Memory writes are
restricted to the memory area and use a limited read-before-write workflow.

The useful ideas for Nola are absolute temporal semantics, explicit correction and forgetting,
compact routing metadata, and bounded consolidation. Nola's Core ownership token, lease heartbeat,
frozen input digest, and completion watermark are already stronger than Claude's file-lock scheme.

### OpenCode

No equivalent staged durable-memory subsystem was found in the checked-in OpenCode tree. Its more
relevant strength is extensibility and session/tool composition. Nola should retain typed Core
policy and provenance rather than replacing them with unrestricted plugin-owned memory writes.

## Implemented Nola behavior

The following behavior exists in the current tree:

- Core owns typed memory records, namespaces, scopes, provenance, retention, confidence,
  supersession, quality assessment, deterministic policy selection, usage accounting, and
  staged extraction/consolidation jobs.
- Memory jobs have frozen source snapshots, input digests and watermarks, ownership tokens,
  leases, heartbeats, retry state, and Core-validated structured output.
- Automatic extraction must cite supplied user items. Assistant-only claims cannot become
  automatic long-term memory.
- The shared Memory Worker prompt excludes secrets, sensitive inference, transient task state,
  activity logs, workspace-derived facts, unconfirmed assistant claims, and duplicated
  authoritative instructions.
- iOS and Android claim Core jobs, invoke a model, submit structured results, and persist Core
  checkpoint effects.
- macOS uses `nola-agent-host`; CLI and Web share the same Core Agent runtime and background
  Memory Worker path.
- Core selection rejects disabled, invalid, disallowed, low-confidence, session-only, expired,
  too-old, superseded, and over-limit records and marks mutable state that requires verification.
- macOS/CLI/Web expose model-callable memory search. iOS and Android primarily assemble long-term
  memory during context preparation.

## Current gaps

### Always-loaded and task-relevant memory are conflated

iOS and Android can inject up to 20 policy-selected long-term records during context preparation.
Core currently receives policy and candidate records, but no current-task recall request or token
budget. Confidence, importance, age, retention, namespace, and supersession are necessary safety
filters; they do not prove relevance to the current turn.

### Cross-platform record semantics drift

- iOS maps its existing profile model into personal/global Core records and collapses much of the
  source kind information into preference or reference.
- iOS and Android currently assign a fixed importance value while preparing Core candidates.
- Android preserves more namespace, scope, and kind information than iOS.
- macOS/CLI/Web can perform explicit search, while mobile clients rely more heavily on eager
  injection.

These are product-behavior differences, not presentation differences. They must not be repaired
with language-specific query rewriting or client keyword gates.

### Retrieval budgets are not first-class

Selection limits count records rather than model tokens. A few long records can consume more
context than many compact profile facts. The same recall result can therefore have materially
different cost across clients and models.

### Recall evidence is not uniformly visible

Core stores provenance and citations, but every client must present recalled memory as inspectable
context with its source, observation time, verification requirement, and reason for inclusion.

## Target Core contract

Core should produce two independent projections.

### 1. Resident profile projection

This projection is small and safe to include on every turn when long-term memory is enabled.

Eligible content:

- explicit user identity needed for interaction;
- explicit, durable, cross-session preferences;
- durable feedback about how the assistant should work;
- active user decisions that are genuinely global.

Ineligible content:

- project or workspace state;
- mutable external facts;
- reference material;
- task progress, recent activity, or previous assistant prose;
- records whose only justification is lexical overlap with the current message.

The input must include a token budget and Host-computed token counts for the active model. Core
must return the consumed token count, rejected candidates, and deterministic selection reasons.

### 2. On-demand recall projection

Detailed identity, project, decision, reference, and procedural memory is retrieved only through a
model-selected recall call. The user's prompt remains unchanged. The model chooses the recall tool
from its schema; clients must not force it through phrase lists.

The Host may use SQLite FTS, embeddings, a remote index, or another storage implementation to find
candidates. It returns standardized candidates with stable IDs, namespace, scope, kind, source,
observation time, updated time, retrieval score, and token count. Core then applies:

1. conversation memory policy;
2. allowed namespace and workspace boundary;
3. retention, age, supersession, sensitivity, and confidence rules;
4. current-state verification requirements;
5. explicit token budget;
6. deterministic tie-breaking;
7. citation and usage-ledger projection.

The Host retrieves bytes; Core decides what may enter model context.

## Proposed typed request

The exact ABI shape may evolve, but the semantic fields are required:

```text
MemoryRecallRequest
  request_id
  session_id
  workspace_id?
  mode: resident_profile | on_demand
  policy
  allowed_namespaces
  maximum_input_tokens
  candidates[]
    record
    token_count
    retrieval_score?
    matched_fields[]
    retrieval_source
```

```text
MemoryRecallPlan
  selected[]
    record
    token_count
    selection_reason
    requires_current_state_verification
    citations[]
  rejected[]
    memory_id
    reason
  used_input_tokens
  remaining_input_tokens
  plan_digest
```

`selection_reason` is diagnostic output, not permission. It must be stable enough for UI,
debugging, and regression fixtures without exposing hidden chain-of-thought.

## Host responsibilities

- Persist records and Core checkpoints transactionally.
- Provide model-accurate token counts.
- Execute candidate retrieval without changing the user's query.
- Keep workspace and conversation identity stable.
- Render selected memory and citations as inspectable context.
- Return usage events to Core after a selected memory is actually included or cited.
- Never infer authorization from a search term, memory kind, or model explanation.

## Core responsibilities

- Own selection policy and all rejection reasons.
- Enforce namespace, scope, retention, sensitivity, supersession, and token limits.
- Distinguish resident profile from on-demand recall.
- Require current-state verification for mutable records.
- Produce stable plan and citation digests.
- Keep automatic extraction, consolidation, recall, update, and forgetting auditable.

## Cross-platform acceptance requirements

The same fixture must produce the same Core plan on iOS, Android, macOS, CLI, and Web.

Required coverage:

- a natural follow-up that requires a prior preference without repeating its wording;
- Chinese, English, and mixed-language recall;
- ambiguous terms such as `search` and `current` that must not force a tool;
- project memories from two workspaces that must not leak across boundaries;
- a newer record superseding an older record;
- relative dates stored as absolute dates;
- an expired record and a low-confidence inference being rejected;
- a mutable reference requiring verification;
- a token budget selecting fewer long records than short high-value records;
- unknown-tool recovery without losing the turn;
- explicit save, update, forget, and disabled-memory behavior;
- citations and usage events surviving persistence and replay;
- real-model end-to-end sessions proving that recall is model-selected rather than triggered by
  client keyword logic.

## Migration order

1. Add the typed resident/on-demand recall request and plan to Core without changing storage.
2. Make CLI/Web and `nola-agent-host` use the plan around their existing memory search.
3. Replace eager mobile injection with the resident profile projection.
4. Add model-callable on-demand recall to iOS and Android using their native stores.
5. Normalize iOS and Android kind, namespace, importance, and provenance persistence.
6. Make every client render recall citations and selection metadata consistently.
7. Remove legacy client-side memory ranking only after contract and real-model tests pass.

## Implementation gates discovered after ABI v2

The resident/on-demand Core contract now exists, but mobile context assembly must not switch to
resident-only injection until both gates below are implemented.

### Mobile on-demand execution

- iOS has a generic native Host executor for installed capabilities, but its current installed
  handlers do not connect `memory_search` to the profile memory store.
- Android's device executor currently handles Skill invocation, scoped file access, clipboard,
  location, and calendar access. It has no model-callable memory handler.
- macOS/CLI/Web already have model-callable memory search through `nola-agent-host` and the shared
  CLI tool runtime.

Switching mobile clients to resident-profile-only injection before adding native on-demand recall
would silently make project/reference memory unreachable. The correct order is therefore:

1. add native `memory_search` execution backed by the existing iOS and Android stores;
2. return standardized candidates to Core for on-demand policy selection;
3. only then replace eager mobile injection with the resident profile projection.

### Token accounting

- iOS's shared telemetry estimator currently uses a rough four-characters-per-token heuristic;
  the prompt-template UI uses a different CJK-aware heuristic.
- Android exposes aggregate context token fields but no per-memory active-model tokenizer at the
  recall boundary.
- Core has one deterministic JSON token estimator for context budgeting, but it is not currently
  exported as a memory/token-count ABI.

Hosts should use the active model tokenizer when available. A shared Core estimate may be used as
a conservative fallback only if the recall request identifies it as estimated and Core applies a
safety margin. Estimated counts must never be presented as provider-reported exact usage.

## 当前记忆用例（2026-10-28核对）

Application memory.rs定义MemoryCommand的Commit/Replace/Recall/Get/GetMany/UsageStats/Delete/List。Commit携带MemoryCandidate和MemoryCommitIntent；Recall使用MemoryRecallQuery及命名域，不能将聊天文本当作来源可信证明。代码索引由独立nola-code-memory模块承接，不把检索索引当作canonical原文或工具授权。

## 候选获取、语义筛选与提交意图（2026-10-28源码核对）

Application Recall先验证RFC3339 now_iso，再由Store按namespace、scope、可选query/kind和时间做有界候选获取（最多50）；Store只负责获取与字面匹配，不能把这一结果直接当作允许进入prompt的最终记忆。

Core select_memories随后执行confidence、supersession、freshness、namespace及token政策。当前该Recall边界允许normal敏感度、OnDemand模式，limit钳制到1…50；没有current-session身份，因此include_session_retention=false。session级保留不能凭同Agent名称跨会话暴露。其他kind的superseding记录参与冲突抑制，但typed recall不能把不匹配kind的结果当匹配项返回。

自动长期记忆提交的ValidatedAutomatic intent必须携带quality assessment并通过automatic候选校验；ExplicitUser/Imported/Synced也必须通过record.validate，不能理解成无需来源验证。Replace先比对当前记录再更换候选，不是UI整表覆盖。

闲时consolidation使用冻结input digest与memory id/revision前置条件：Application重验语义输入，Store在checkpoint提交事务内再次核对已观察行版本，关闭读后写前的竞态。用户在后台任务运行期间修改或删除记忆，不能被旧整理结果静默覆盖。

Context中引用Memory与写入长期记忆是两个用例。context provenance/disclosed_to_model只描述来源与实际披露，不授予写入intent；code-memory索引同样不能取代canonical原文与scope授权。
