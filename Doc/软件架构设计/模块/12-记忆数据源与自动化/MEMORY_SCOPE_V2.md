> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Memory ownership and recall isolation

Source checked: 2026-11-08. Concrete ownership is now part of the current memory record, Application recall and canonical Store format. Validation results below distinguish deterministic tests from real-provider/platform acceptance.

## Current contract

- Domain `memory_owner.rs` owns `MemoryOwner` and `MemoryExecutionBinding`; the unconnected Core facade helper has been removed. `MemoryRecord.owner` is required on decoding, with no provenance-based inference or old-format fallback.
- Namespace describes a collection; owner describes a concrete visibility boundary; provenance describes evidence. They are independently validated. Workspace, conversation and turn owners carry their concrete IDs. Turn ownership includes its conversation.
- The current product has one personal memory profile per canonical Store. Its explicit Store-relative ID is `local`, unrelated to model configuration profiles or login accounts. Separate canonical Stores remain separate ownership boundaries; this does not introduce a multi-account memory directory.
- `MemoryRecallQuery.binding` is mandatory. Application resolves its exact owner from namespace and scope before touching Store. An absent workspace/conversation/turn binding is an error, never a global-search fallback.
- Store `owner_json` is required and canonically encoded. The recall index, literal/lexical queries, successor-chain queries and content deduplication include owner before ranking/LIMIT. Ordinary commits, replacement and replica installation cannot move an existing identity between owners. Supersession cannot cross owners, including a target arriving after its reference.
- Store version is 83 (epoch remains 8), Service protocol is 54; the stateless memory-selection ABI is 4. Existing incompatible stores are rejected; no owner is inferred and no database is converted, deleted or recreated on normal startup.

## Execution and platform wiring

CLI/Mac Rust Host passes an explicit binding in ToolContext. The conversation and Run/turn IDs come from the admitted Host request; the workspace ID uses the same frozen workspace identity calculation as AgentTurnIdentity. Tool schemas expose scope and content/query, not owner IDs. Named collections use the explicit `custom:<name>` form; unknown scope strings are rejected instead of silently becoming global collections. Concurrent calls do not share owner state through environment variables or mutable session globals.

CLI resident personal recall uses Application Recall rather than an unscoped management List. Rust Web conversation search binds the requested canonical conversation after checking its existence; a missing workspace or turn binding is rejected rather than guessed from an allowed directory.

Mobile profile search now uses Application Recall for the personal and current profile-editor collections. It does not rank an unscoped editor List to implement model search. iOS and Android preserve owner in their canonical/selection projections. iOS no longer reinserts all durable records when Core rejects every candidate; a missing/incompatible memory engine is an explicit failure on native memory-search paths. The profile editor retains records outside its projected owner/collection instead of rewriting their ownership. Mobile memory search currently offers personal-profile recall; this change does not claim that native workspace/turn selection UI exists.

Automatic extraction explicitly writes the current personal profile. Consolidation validates a single frozen owner and copies that owner to its output; input digests and the existing revision/CAS transaction remain authoritative. A worker's citations do not select its owner.

Management List/Get/GetMany/Delete and whole-profile Replace remain explicit management contracts. They can enumerate canonical records for editing; they are not the model recall authorization boundary. Do not pass their unscoped results to a local-scope tool as a substitute for Recall.

## Acceptance cases

- Conversations A and B store identical content under local scope; both records
  exist, and each recall returns only its own ID.
- Two turns in A cannot recall each other's turn-local memory. Conversation A
  memory remains available to both according to the explicit recall plan.
- Workspaces A and B remain isolated even when paths have the same final name.
- Global personal memory can be recalled across conversations by policy.
- A global record citing conversation A remains global; changing provenance
  does not change ownership.
- Unbound or mismatched local scope is rejected before the Store query.
- Concurrent real-model runs cannot exchange owner context through Host state.
- More than one result page of high-ranked foreign-owner records cannot crowd
  out the valid record; filtering happens in the Store query before LIMIT.
- Cold restart, sync, deletion and content deduplication preserve isolation.
- Mac, iOS, Android, CLI and Web submit the same typed ownership contract.

`memory_port_e2e` now includes concrete owner isolation, identical-content deduplication, foreign rows exceeding the bounded page, turn/workspace binding, cross-owner overwrite/supersession rejection, cold reopening and missing-owner/binding wire rejection. Store and worker regression coverage remains required alongside it. Concurrent real-provider sessions with private markers and each native device interaction are separate acceptance work; deterministic tests do not prove those environments.

## Maintenance input identity

The consolidation scheduler previously hashed only selected IDs and refreshed
only missing Core records. Editing an existing memory could therefore reuse a
completed job identity while leaving its projected input stale. The scheduler
now refreshes changed records and derives job identity from sorted, validated
semantic records, hashed without a second serialized input buffer.

Application now checks that digest against canonical semantic records both on
job creation and on completion. Both `MemoryWorkerCompleted` and the direct
`MemoryRuntime::Complete` entry pass this check. It then passes the observed
row revisions to Store, which rechecks them inside the checkpoint transaction.
An intervening edit or deletion rejects the whole commit rather than advancing
the job without its memory projection. Ordinary candidate identity guards also
apply to background output; tombstones and scope changes cannot be bypassed.

Deterministic evidence: all 158 Application tests passed, including both
completion paths after cold restart with edited/deleted inputs and unchanged
inputs. Store tests exercise another connection updating a record between
observation and commit, plus rollback of checkpoint, rows and sync outbox.

This is semantic input comparison plus commit-time revision fencing, not a
persisted vector of the revisions at job creation. An edit followed by an exact
semantic restoration before completion is indistinguishable from unchanged
input. Strict source-version admission, extraction-source revision checks,
real-model concurrent maintenance, and owner isolation remain open gates.

The Host scheduler now borrows its committed snapshot and only copies changed
selected records. Core ranks references before copying the selected 40 records.
A deterministic 10-million-character / 1,000-record selection test measured
439,312 allocated bytes. This excludes loading the source records, checkpoint
serialization and UI rendering; it is not a whole-process memory bound.
