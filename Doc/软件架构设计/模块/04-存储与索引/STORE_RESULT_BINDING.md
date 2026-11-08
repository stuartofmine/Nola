> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Store result binding — current implementation and remaining audit

This document records the schema 69 Store boundary for the current, unreleased
protocol. It is not a compatibility plan. Implementation has not yet been
compiled or tested: the complete goal is being implemented and audited before
the final combined verification run.

## Production data flow

Application commits original Core envelopes through `commit_transition` or
initial admission. `run_requirements::index_event` consumes the four events
`requirement_scope_changed`, `requirement_assessed`, `run_result_decided`, and
`runtime_decision_fact_observed` in that transaction. Caller-authored canonical
results are not accepted by a separate port.

`run_requirement_sources` stores immutable source pointers and digests, not a
second writable semantic ledger. It binds the exact Run, conversation, Turn,
Core revision/cursor, original transition, and optional original Host inbox
record plus its digest. Model decisions must use the same transition's original
ModelCompleted response and exact effect/request/attempt/call; a historical
successful response cannot authorize a new decision. Original admission events
have no transition pointer and are never retroactively assigned to a later
transition.

At every scope, assessment, and result, `run_input_frontier::audit_for_decision`
reconstructs the full original applied-input set through the fixed decision
frontier. Original events, Turn application records, dense Run application
records, every prefix digest, and accepted item identities must agree in both
directions. Turn-local sequence numbers are not a Run watermark. A scope covers
all applied original item IDs, including those from earlier Turns in that Run.

Assessments bind the exact scope version and original execution evidence. An
action result is checked against all assessments through its frontier, not only
the successful assessments named by the result. Cancelled requires an original
Interrupt in the fact's own transition, or an exact Host permission
ReturnToComposer response; arbitrary cancellation strings are not sufficient.

## Atomic publication

The caller opens one write unit of work: original events, checkpoint, journal,
result, and V2 seal commit together. `seal_result` re-reads original sources and
rejects missing indices, late applied input, later control facts, open user
waits, and unaccounted effects. `commit_write_unit_of_work` rejects a Run marked
sealed without its immutable seal. Terminal recovery cannot synthesize a seal
from checkpoint or message status.

Run lifecycle terminal status is `sealed`; the outcome lives in `RunResult`.
V2 seal contains the complete result, result digest, independent Core and Store
checkpoint revisions, and the frozen assistant manifest. Only actual bound
agent child Runs contribute a ChildReceipt/result digest; shell receipts and
tasks failing before child admission are not counterfeit child results.

Unknown external effects are retained, not acknowledged. A pending effect may
remain only when the result explicitly retains it (Cancelled/Failed unknown
refs, or Blocked original blocker refs), and its latest durable delivery attempt
is OutcomeUnknown, was actually started, has ended, and has no settlement.
Prepared, leased, and execution-started attempts still prevent a seal. Store
rejects new dispatch/lease renewal for a sealed Run. Recovery inspection reads
the exact complete V2 seal in the same pinned snapshot; sealed without a seal
is an invariant error.

## Remaining work before final verification

- Scope changes now re-read requirements_update original arguments, validate
  the exact expected version/frontier token, and bind new local keys, dependencies,
  and every retained/replaced/withdrawn requirement to the resulting scope.
  Continue auditing assessment/result operation-to-event correspondence.
- Add exact Host/source-transition substitution and unknown-effect seal
  integration fixtures; test no old ModelCompleted response can authorize a
  later decision and no live attempt can be made terminal by changing a label.
- Migrate old status-only Application completion fixtures to real Core admission
  and Host reducer delivery. Do not restore the removed completion shortcut.
- The five identified status-only Store transition fixtures have been migrated:
  append-sync, deletion, journal resumption, and maintenance (separate maintenance
  contract). Conversation fixtures now submit original input/scope/fact/result
  events and publish the V2 seal in one unit of work. Continue auditing direct-SQL
  fixtures and remote manifests; these source edits have not been executed.
- Run the final combined Domain/Core/Kernel/Store/Application and client suites
  only after the goal's implementation and audit are complete.
