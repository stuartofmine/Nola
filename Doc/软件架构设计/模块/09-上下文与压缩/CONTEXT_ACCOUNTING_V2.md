> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Context accounting V2 implementation contract

Status: partially implemented, acceptance incomplete. This document refines ARCHITECTURE_V2 sections
3, 12 and 16; it does not declare those requirements complete.

## 2026-10-21：跨端共享维护契约

当前改造已接线，集中验收进行中；以[当前任务](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/跨端共享业务收敛与验收_20261021.md)记录结果，不继承下方历史测试结论。

- Kernel `compaction_summary` 拥有冷分段、滚动摘要与有界拆分计划。CLI Host 直接调用共享计划；移动端经 Application `step_compaction_summary` 的 `start / accept / split` 执行不透明状态，返回下一次 `request` 或最终 `complete`，客户端不解析状态或另算分段。
- Kernel `evaluate_compaction_maintenance` 统一摘要验收、续写及预算重写决策。Host 只报告实际 provider 完成原因、工具调用和可用 continuation；缺少完成原因不等于完成，不能提交被截断的摘要。协议无法续写时明确失败，不伪造 continuation。
- 移动端手动维护先捕获来源见证，再读取输入，经 `Context::PrepareMaintenance` 验证来源并生成计划；`CommitMaintenance` 重新校验摘要和预算，通过 Store 原子来源校验提交。旧 `CommitCompaction` 命令及客户端包装已删除。
- 运行中自动维护仍由 Agent 的持久 attempt、恢复及取消机制驱动；手动维护使用 Application 事务，两者共用策略。每次物理模型请求保留原 attempt/辅助 operation 计量；中间分段成功不意味着最终压缩成功。
- iOS 删除回复完成后另行触发压缩的阈值与后台入口。客户端不在 Core 维护之外再启动一套自动压缩工作流。
- 热分叉实际请求快照、缓存前缀、Responses continuation、usage 和物理取消继续由 provider/Host 适配负责。共享策略不以重新生成的逻辑历史冒充实际请求前缀。

## October 10: bounded incremental compaction

The shared CLI/macOS Host uses the following maintenance flow:

- Keep the existing eligible physical-request fork, preserving its original prefix.
  Eligibility is not proof of a cache hit. Missing snapshots, expired eligibility,
  changed settings and unknown cache usage select the cold path.
- Cold compaction processes chronological segments and updates a rolling checkpoint.
  The Core-selected recent tail stays outside the archive. Long tool text is
  excerpted only in this disposable request projection; original history is unchanged.
- Reserve input, rolling-checkpoint and output space together; check the serialized
  request before sending. Core supplies the admitted window in the Compact effect;
  the Host does not guess capacity from a missing request snapshot. Provider overflow
  can split a segment.
- A rejected summary is discarded. Split only the rejected segment, retaining the
  last accepted checkpoint; never repeat the same full archive with a shorter prompt.
  Bound the initial plan to 64 segments and recovery to four splits. Cancellation
  starts no further segment. Do not silently switch maintenance to another model.
- Record each physical attempt's usage, including rejected and intermediate outputs.
  Metrics include strategy, completed/remaining segments and splits. Intermediate
  summaries never emit CompactionCompleted; Core validates the final summary and
  retained-tail window budget before committing the replacement.
- Exhaustion returns a retryable failure with the original context intact; it does
  not fabricate a summary or continue sampling into a full window. Existing turn
  failure/retry semantics remain; this is not a new automatic background-resume state.
- Goal, constraints, execution evidence and permission records remain their existing
  independent canonical aggregates, not authority reconstructed from summary text.
  Intermediate checkpoints and physical cache witnesses are process-local; a restart
  starts cold compaction again from the preserved canonical archive.

This closes identical-input summary retries, not all compaction acceptance gaps.
Real-provider semantic recall, restart cost and client recovery UX require separate
acceptance; deterministic protocol tests must not be reported as those results.

Validation for this change (October 10): 22 CLI compaction tests, 26 Core
compaction tests, four context-budget tests, one durable compaction receipt/cold
replay test and one Goal evidence restoration test passed. The HTTP fixture
verifies that rejection reduces the next segment, a successful segment is carried
forward, every attempt is billed, and exhausted recovery never emits completion.
The receipt test rejects invalid summaries before committing and preserves the
canonical history across reopening. These tests do not modify live conversations
or establish real-model summary quality/cache hit rate.

## Original audit evidence

The September 7 audit found the following production paths:

- `cli/src/store/compaction.rs` opens SQLite and creates
  `context_runtime_ledgers` on every ledger operation.
- `CliAgentHostDriver::execute_compaction` writes success before emitting
  `CompactionCompleted`. Core can subsequently reject the summary budget or
  usage payload, leaving the Host ledger inconsistent with accepted state.
- Success, failure and tool-output writes discard persistence errors.
- Driver construction uses `open_default().ok()` and disables the ledger under
  `cfg!(test)`. Most Host unit tests therefore exercise a different path.
- Failure counting has no effect/response identity. Re-executing one delivery
  can increase the failure count again and trip the three-failure guard.
- `enforce_cumulative_task_budget` combines checkpoint model usage with an
  independently opened default database's tool estimate.

The existing cold-Host fuse test explicitly injects the legacy database and
proves only that a seeded count blocks a request. It does not prove atomicity,
delivery deduplication, or correct accounting after Application rejection.

## Implemented since the audit

- Removed the Host compaction ledger, default database fallback and callback
  writes. Task budget enforcement no longer adds tool byte estimates to
  cumulative provider usage.
- Core checkpoints now carry the planned source fingerprint and archive-specific
  retry state. The fingerprint hashes serialized canonical input directly,
  without retaining another complete archive copy.
- Added cold-restore retry tests and real-model Mac compaction/memory sessions.
  These do not prove rejected-attempt cost accounting or fault-injected atomic
  acceptance. Those gates below remain open.
- Real Service history recall was exercised with four 10-million-character
  conversations and 68 alternating queries: P95 37.4 ms, sampled peak RSS
  32.8 MiB, post-warm growth 224 KiB. This is a short process-level query test,
  not a full-workspace rendering or long-duration memory guarantee.

## Ownership and state

Application accepts concrete Host results, lets Core reduce them, and commits
the resulting checkpoint together with the Host inbox receipt, event journal
and successor outbox through the existing `commit_transition` transaction.

Accounting belongs to the Core-owned context state persisted in that checkpoint.
It must not introduce a second mutable ledger maintained by Host callbacks.
Host reports observations; it neither decides that compression was accepted nor
updates durable counters. No new SQLite connection is needed per observation.

Keep these quantities distinct:

- Current model working-set usage, with provider/estimated origin.
- Cumulative provider usage, including reported usage of compaction attempts.
- Accepted compaction count and accepted summary generation.
- Failed compaction attempts and the bounded retry guard for a source archive.
- Tool result bytes presented to the model, excluding separately stored full
  artifacts. This is an estimate input, not additional provider-billed tokens.

Do not add tool estimates to provider input usage and label the result actual
token consumption. Define the task budget's units explicitly before replacing
the current combined budget calculation.

The pending compaction plan must identify its source working-set revision or
stable fingerprint and summary budget. A fingerprint is computed from the
canonical planned prefix, not from an independently reformatted Host prompt.
Do not retain another copy of the archive just for accounting.

## Acceptance and failure semantics

1. Validate session, Run, effect, request, lease and response sequence using the
   existing protocol before accepting an observation.
2. Deduplicate accepted deliveries using the existing inbox/transition identity.
   A retry with a new sequence must not create a second terminal outcome for an
   already settled effect.
3. Validate summary structure, summary budget and usage. Accepted compaction
   replaces only the model working set; canonical transcript Parts are intact.
4. Commit accepted outcome, counters and checkpoint atomically. A database
   failure must leave the request recoverable and must not acknowledge success.
5. A model call that consumed tokens but produced a rejected summary is not an
   accepted compaction. If its cost is retained, record a validated attempt-cost
   observation with its own identity; never advance the accepted generation.
6. Malformed protocol responses do not consume the pending response sequence.
   A corrected delivery remains usable. Transport retry, rejected candidate and
   completed failed attempt must not all increment the same failure counter.
7. Cold restore reproduces the same retry guard and usage without consulting a
   CLI database. Changed archive identity resets only the applicable guard.

The existing automatic-compaction-per-turn progress limit and the repeated
archive failure guard have different purposes. Reconcile them explicitly in
Core; do not silently remove one while deleting the Host ledger.

## Required implementation changes

- Extend typed Core compaction state and Host observations only where the
  current contracts lack source identity or attempt cost.
- Update Core reducers and checkpoint round-trip coverage before removing the
  Host ledger. Application must commit terminal accounting on its durable path.
- Remove `context_runtime_store`, its default-open fallback, all Host writes,
  and `cli/src/store/compaction.rs` once their consumers use canonical state.
- Make task-budget enforcement read the same committed context projection used
  by clients. Preserve explicit distinctions between usage and estimates.
- Expose bounded accounting fields in the context projection. Mac footer and
  inspector, iOS, Android, CLI and Web consume that projection per conversation.
- Do not migrate old ledgers or add compatibility fallbacks; this is the V2
  hard-cut contract.

## Regression and performance evidence required

| Scenario | Required observation |
| --- | --- |
| Accepted summary | Generation and counters advance once; original transcript remains readable |
| Identical response replay | No additional cost, failure or generation increment |
| Terminal effect replay with new sequence | No second terminal accounting outcome |
| Oversized or malformed summary, then corrected delivery | Pending plan survives; only accepted summary changes working set |
| Provider cost with rejected summary | Cost and accepted-compaction count remain distinct |
| Store commit failure, then retry | No partial counter/checkpoint/outbox update |
| Cold restart after each failure | Same guard state and next allowed action |
| Different source archive | No stale guard inherited from the previous archive |
| Same conversation through different clients | Identical usage and compaction projection |
| Large tool artifact | Bounded model-result accounting; no full artifact materialization |
| Real-model compaction followed by recall | Random fact retained or found through canonical history with provenance |

Use temporary canonical stores and production Host configuration paths. Tests
must not disable persistence through `cfg!(test)` to obtain a passing result.
Use deterministic fault injection for commit failures and a real provider for
semantic compaction/recall; the latter alone does not establish transactional
correctness.

The September 7 Mac switching run passed two tests in
`/tmp/nola-long-switch-isolated.xcresult`: process-scoped test-storage isolation
and switching between two 10-million-character conversations twelve times.
Measured cold completion was 226 ms, switching P95 207 ms including test
settling, resident window 50 messages / 501,350 text bytes, cache growth about
3.17 MiB, initial UI growth about 81 MiB, and measured switching RSS growth zero.
These are client-process measurements in a transcript hosting harness. They do
not establish Service RSS, full workspace interaction or animation frame pacing.
The captured screenshot shows a transcript tail; it does not cover the sidebar,
composer or inspector. Those remain separate acceptance gaps.

## Native Mac history-path audit

The real-provider recall test exposed an additional boundary violation:
`MacConversationRecallBridge` loaded the complete conversation and every context
checkpoint, then ran `MacConversationRecallIndex` in Swift. This path did not use
the bounded Application search exercised by Service performance tests. Its
results mixed original messages, generated summaries and unbounded knowledge
fields. Service query measurements therefore did not establish Mac recall cost.

The tool bridge has been changed to the typed Application
`search_conversation_text` command. Results carry bounded snippets and canonical
message/Part identities, ordinals and character offsets. Summaries and knowledge
remain separate aggregates instead of being implicitly added to history results.
Mac repository/UI `searchConversationAsync` still uses the old full-load search
and remains a required migration, including explicit query-error presentation.

Tool event `text` is a brief status summary, not the result body. Test assertions
must inspect `trace.payload.result` and decode nested protocol JSON, then require
an original-message result containing the random fact and canonical identities.
A fact found only in a generated summary is insufficient evidence.

The strengthened Mac test group includes real-model compaction/recall, native
bridge isolation from knowledge/summary fields, and a ten-million-character
tail query. Its latest execution has not produced a final verified outcome.
Previous attempts failed test assertions and stalled in XCTest failure
symbolication. Do not record these attempts as passes or confuse a live test
process with evidence of product progress. UI screenshots and frame pacing,
whole-path memory measurements, and the UI search migration remain outstanding.

## 当前共享入口与复制（2026-10-28核对）

Application context.rs与engine compaction_driver.rs分别承接用例及压缩推进，Host执行物理请求；各Renderer不另建压缩状态机。usage、cache measurement与diagnostic各有明确来源，remote_run_context_usage保存源记录摘要。统计未知与零值分开，复制诊断的camelCase字段必须原样保留，不受外层snake_case编码策略改写。
