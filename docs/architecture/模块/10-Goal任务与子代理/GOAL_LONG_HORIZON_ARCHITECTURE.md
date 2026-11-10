> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Goal and Long-Horizon Execution

The dated implementation and validation sections below retain their original
scope. Their schema numbers are historical; current versions come from code.
Normative ownership and recovery follow the current software architecture.

## 当前关系与执行覆盖（2026-11-02）

统一所有权、输入归属和生命周期见 [AGENT_THREAD_ARCHITECTURE](AGENT_THREAD_ARCHITECTURE.md)。本文的长期目标能力不能由一次 worker 完成声明替代：Run seal、Task receipt、Goal 逐项验收是三个不同事实。当前 CLI 与 macOS 原生（复用 Host 进程）可自动推进，macOS 还有后台唤醒；iOS/Android 本机与 CLI/macOS/Web Host 均消费同一个 Application `step_goal`；同步 Host 也仅恢复该 step 准入的 Run，不再独立组装轮次，仅承诺 App/Host 可执行期间推进；Start 复用已建立会话的冻结配置，配置改变需先建立新普通 Run，全新会话直接 Goal 尚未接线。`continue` 与 `incomplete` 必须分开，后者没有自动 wake。本轮不新增跨端任务引擎、订阅系统或持久化迁移。

## Unified on-disk work archive (2026-10-09)

Goal, task/subagent state, retained evidence and compaction records are exported
by the shared Store after an outer transaction commits. SQLite remains the
transactional authority; generated files are recovery data, not a second mutable
state machine. Rollbacks never publish a snapshot. A filesystem failure does not
turn an already committed command into a failed/retryable side effect: publication
remains pending and retries on a subsequent commit or Store reopen.

The private directory is adjacent to the canonical database, outside the project
unless the database itself was placed there:

```text
<database>.work/<sha256(conversation_id)>/
  manifest.json
  generations/<content hash>/
    checkpoint.json   # baseline plus ordered replay inputs and durable cursor
    goal.json         # structured working agreement, including superseded entries
    goal.md           # human-readable constraints, progress, blockers, next actions
    tasks.json        # baseline projection and committed owned durable task rows
    evidence.json     # baseline successful-result index, not acceptance authority
    compactions.json  # retained compaction records and source item references
```

Each publication writes and syncs a complete immutable generation before atomically
replacing the manifest. The current and previous published generations are retained;
this is a recovery snapshot, not an unlimited backup of every superseded Goal.
Unpublished staging directories are ignored. Directories/files are owner-private
on Unix (0700/0600); symlinks and special files are rejected. Conversation identifiers
never become path components directly. A cross-process publication lock precedes
the consistent database read snapshot, preventing stale publishers from moving the
manifest backward. Verified readers share that lock and validate every member's
SHA-256, identity and generation before exposing data.

Checkpoint persistence can contain a baseline body followed by replay inputs.
`checkpoint.json` preserves both: the Store CAS revision is not assumed to be the
Core revision. Readable projections identify their baseline revision and
`replay_pending`; they must not be mistaken for a fully reduced latest projection.
Application `inspect_archived_work` uses Core restore and ordered replay validation
to reconstruct the latest Goal, tasks and transcript without the original database.
It is **read-only inspection**, not automatic database import, permission restoration
or worker restart. Missing external artifact/output blobs are not recreated: the
archive retains their references, not copies of referenced files. Checksums detect
corruption, not malicious rewriting with recomputed checksums; archived content
never grants tool authorization or proves semantic task completion.

SQLite update hooks mark checkpoint, task and replay changes dirty. Unrelated UI,
trace and read transactions do not trigger full publication. Automatic publication
compares version/cursor/task digest to skip unchanged work; explicit verified reads
and full flushes validate members. `work_archive_status` reports availability,
pending publication and replay state. This shared implementation is used by hosts
that persist through NolaStore; no separate client-owned archive writer is needed.

Coverage includes transaction rollback, interrupted staging, corruption rejection,
private path handling, retry after reopen, unchanged-work skipping, database-free
Core reconstruction, replay-gap rejection and real service cold restart. Native UI
and real model-provider acceptance are separate from this storage test gate.

Archive acceptance: 5 Store archive tests, 1 Application offline recovery test,
1 real service cold-restart test and all 8 Goal PTY scenarios pass. The broader
Store unit suite reports 357 passed, 2 failed, 1 ignored: both failures are existing
migration tests comparing `idx_model_profiles_selected` SQL verbatim despite only
whitespace differences. They are not counted as passing or silently waived.

## Durable working agreement (2026-10-09)

Goal checkpoints now retain a versioned working agreement in addition to the
objective and verified requirements. `work.entries` holds constraints,
requirement details, decisions, reported progress, blockers, and next actions.
Each entry has an immutable id, provenance, source reference, optional superseded
record, and requirement/task/evidence references. Exact user additions are
captured in submission order as `work.user_updates`, including additions queued
during an executing Goal turn;
ordinary conversation is not silently enrolled into a Goal.

`goal_update` is a Core-local model control, not a Host tool. It requires the
Goal id and current projection revision and executes alone in its batch. Core
assigns model provenance; the model cannot claim user authorship or replace a
user-authored record. Trusted GoalRuntime `UpdateWork` submissions can record
explicit user-approved changes. Updates validate atomically; stale or malformed
batches leave the entire projection unchanged. Replacements retain history.

`reported_done` progress needs evidence references but does not change verified
requirement status, accept child results, or complete a Goal. Task references
remain links, not a second execution state machine or new permission grants.
Verified progress is derived from requirements, not a model-supplied percentage.

Every Goal model request reconstructs its working view from the checkpoint,
including after compaction/restart. Original constraints, all requirements,
user additions and current work entries are retained without the old first-8 /
first-12 truncation. Superseded work history stays in the checkpoint, not in each
prompt. Active structured work is bounded to 48 KiB with explicit rejection,
never silent constraint loss. This section recorded checkpoint schema 11 at the
time; it is not the current schema declaration. Incompatible data is explicitly
rejected, and no user database is deleted automatically.

Completion now uses explicit per-requirement claims and Core acceptance, not a
mandatory additional auditor request. Plain final text cannot complete a Goal.
`AcceptCompletion` checks the projection revision, complete requirement coverage,
retained successful Goal-owned turn results (or a response deliverable), settled
contract, and terminal state of Goal-owned tasks. It commits acceptance atomically.
Core verifies evidence existence; semantic relevance remains the model's judgment,
not a claim of deterministic proof. Independent review is model-selected through
`request_completion_review`. Platform-specific Goal editors are outside this change.

`work.source_turn_ids` and `work.owned_task_ids` are assigned by Core, not by
model-authored work entries. Completion may cite an earlier retained result from
the same Goal, but cannot borrow unrelated conversation output. References that
cannot resolve are rejected. Compaction retains the canonical transcript in the
checkpoint: a Core-owned evidence index is rebuilt from that transcript for Goal
model requests, even when the model context contains only a summary. The index
and acceptance share the same resolver. Stable references are
`tool:<turn_id>:<call_id>` and `response:<item_id>`; a short tool reference is
available only if unique. Failed results, unrelated turns and summary claims
are excluded. The latest response also has the `response` alias. Preview text
is explicitly untrusted data, not an instruction or semantic proof.

## Task ancestry and writer admission

`TaskLineage.resumed_from_task_id` identifies the previous bounded Task record, not merely its Run, independently
of `parent_task_id`. Derived follow-up work creates a new Task and preserves the predecessor's actual parent and child
session. Core rejects live, missing, self-referential or mismatched predecessors;
resuming a sibling does not create an artificial cancellation descendant.
Application reconstruction and CLI task status retain this separate relationship.

Write-capable agent admission is enforced by a single conditional Store INSERT,
inside the task/event creation transaction. Two processes cannot both pass an
earlier Host-only availability check. Nonterminal writers, including permission
waits and reconciliation, retain exclusion; terminal writers release it. Readers
and writers in other projects remain independent. This is admission control, not
a new lease or permission grant; normal tool permissions still apply.
Existing filesystem project paths are canonicalized before creation, so symlink
aliases cannot acquire separate writer slots. Resume ownership checks use the
same path identity.

Final regression gate for this change: 424 Core unit tests, 120 Core/Host
contracts, 9 Task-handle contracts, 29 Store task tests, 2 Application task-result
tests, 3 CLI completion-protocol tests, 1 CLI resume-owner test, 8 Goal PTY tests,
and 1 real service cold-restart test passed (597 total). Store tests include
concurrent writer admission and symlink aliases; Core tests include ambiguous
evidence references and restoration with summary-only model context. No native
Mac UI or real model-provider run was performed.

Acceptance validation: 422 Core unit tests and 120 Core/Host contract tests
passed; 3 CLI completion-protocol tests passed. Updated PTY scenarios passed
for explicit output-only completion without an auditor and for preserving a
draft through acceptance, then committing the next user/assistant turn once.
An ordinary answer with no completion claim leaves the Goal unfinished; a new
user input can yield its continuation rather than implicitly completing it.
No native Mac build or real-provider acceptance is claimed by this test gate.

Validation: 422 Core unit tests, 119 Core/Host contract tests, 2 Goal checkpoint
tests, 5 input-origin tests, and 1 real service cold-restart test passed (549
total). The restart test persists work entries before replacing the service
process and checks them after resume. The model-control test covers trusted
provenance, checkpoint restore, no Host effect, and live queued user additions.
The CLI debug binary was rebuilt successfully. Native Mac UI/build and real
provider sessions were not exercised by this gate.

## Execution lifecycle (existing)

Nola Goal is durable execution state, not a prompt loop. Core owns the objective,
constraints, requirements, subtask contracts, execution reports, evidence, audits,
wake time, retry count, and worker lease. CLI, Web, and macOS are Hosts that start
or resume work and render the same projection.

This design adopts the useful separation described by DeepSeek Harness and the
LongHorizon work: management, bounded execution, and independent audit are separate
responsibilities. The reference is architectural input, not a wire-compatible
dependency:

- <https://github.com/deepseek-ai/deepseek-harness>
- <https://arxiv.org/abs/2608.01964>

## Manager, Executor, Acceptance

1. **Manager / Core** creates typed requirements and one bounded subtask contract.
   It persists every transition using idempotent operation IDs.
2. **Executor / Host** performs the current contract and reports artifacts,
   blockers, and claimed completion. Executor prose is never completion evidence.
3. **Acceptance / Core** resolves each explicit requirement claim to committed
   evidence. The model can request independent review when warranted; it is not
   forced for every Goal. A single blanket pass cannot complete all requirements.

There is no fixed tool-call or Goal-round limit. A cycle ends because it is
completed, needs user input, is cancelled, reaches its configurable runtime budget,
or encounters a retryable failure. Absence of progress in a small number of model
rounds is not a terminal condition.

## Durable lifecycle

The authoritative state is `GoalProjection` in the Agent Core checkpoint:

```text
start -> claim -> plan contract -> execute -> report -> audit
  |         |                                      |
  |         +-> heartbeat / release / retry        +-> complete
  +-> pause / schedule / resume / cancel
```

- Every interactive or scheduled Goal worker acquires a lease.
- The lease covers at least the configured execution budget plus a safety margin.
- A live lease prevents a second worker from executing the same Goal.
- A paused Goal becomes discoverable when its durable wake time arrives.
- A running Goal becomes discoverable when its worker lease expires.
- Legacy running checkpoints without a lease become recoverable after a two-hour
  safety window rather than remaining stuck forever.
- Recovering a partially executed contract records an interrupted execution report
  and moves it to audit. It does not blindly replay side effects.
- Retry delay is exponential from one to 64 minutes and remains bounded without an
  artificial attempt-count terminal state.

## Platform contract

- **CLI** exposes start, status, pause, resume, schedule, cancel, and manual
  compaction while preserving the Goal checkpoint across process restarts.
- **Web** exposes the same lifecycle through the conversation Goal endpoint. Goal
  mutation is rejected for nonexistent conversations, preventing orphan state.
- **macOS** runs due Goals through Agent Host and treats a stream that ends without
  an explicit successful terminal event as failure.
- External Codex, Claude Code, and OpenCode sessions remain execution backends, not
  Goal authorities. Their text, reasoning, tools, usage, and session IDs are mapped
  to the common Host protocol.

## Verification evidence

Current acceptance evidence includes:

- Core Goal state, audit evidence, wake-time, lease ownership, and expired-lease
  reclaim tests.
- CLI due-discovery tests for scheduled wake, live lease, expired lease, legacy
  unleased recovery, and `needs_user` exclusion.
- A real-model CLI process-restart scenario proving objective persistence and resume.
- Real Codex, Claude Code, and OpenCode first-turn and native-session-resume runs.
- Web start, pause, schedule, resume, cancel, and durable readback through the real
  HTTP product path.
- A macOS unsigned development build using the embedded release Agent Host.

Wall-clock waiting for several days is not used as a test strategy. Multi-day
behavior is tested by persisted timestamps, process replacement, lease expiry, and
restart, while real provider calls verify the execution boundary.
