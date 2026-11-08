> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Application Port Contract

## Role

Application is the product use-case boundary. It coordinates Core decisions,
Store transactions, and Host effects. It is not a generic database RPC layer.

Every public command belongs to one capability port. A client chooses a use
case; it does not choose tables, revisions, projection mutations, checkpoints,
or trace rows.

## Command Shape

The serialized boundary is namespaced:

```json
{
  "type": "skill",
  "command": {
    "type": "set_enabled",
    "id": "document.word",
    "enabled": true,
    "updated_at_ms": 1787420000000
  }
}
```

Logical ports group operations by capability. The existing wire dispatcher may
remain a tagged union during caller migration; a flat tag alone is not a
business-boundary defect. Consumers receive only their use-case port, and may
not gain unrelated repository powers by depending on the dispatcher.
The JSON above is a logical port example, not an implemented universal envelope;
transport identity and receipts follow the shared protocol contract.

Current extracted ports include:

- conversation and Run lifecycle
- permission request lifecycle
- Skill
- MCP
- data-source configuration and usage audit
- document-source indexing and query
- automation definition and automation Run lifecycle

Memory, model, runtime task, sync, settings, and diagnostics remain to be
extracted from the legacy command bus.

## Object Ownership

Every object has exactly one semantic owner. Serialization does not transfer
ownership.

| Object | Owner | Allowed consumers |
| --- | --- | --- |
| identity, entity invariant, lifecycle vocabulary | Domain | Core, Application, Store, Protocol |
| use-case command, query, and projection | Application | generated/native client adapters |
| deterministic transition and effect intent | Core | Application and Runtime Driver |
| versioned effect request and Host response | Protocol | Runtime Driver and Host adapters |
| `Stored*`, revision row, blob reference, inbox/outbox row | Store | Application implementation only |
| credential bytes and OS authorization handle | platform secure store | Host adapter by opaque reference |
| animation, selection, expansion, scroll intent | Presentation | the owning window |
| unsent draft, attachment versions, CAS/tombstone | device DraftRepository | draft controller and presentation |

The public Application facade must not expose a type whose name or semantics
describe how a row is stored. Store records are mapped to Application objects
inside the capability port. A platform binding may mirror an Application
object for language interop, but it may not add persistence fields or invent a
second lifecycle.

Identity values are opaque and case-sensitive. Clients, transports, and Host
adapters must preserve them byte-for-byte; they may validate shape but cannot
lowercase, normalize, derive, or replace them.

Lifecycle state is scoped by object. Session, Turn/Run, effect request,
permission request, automation definition, and automation Run are distinct
objects. A state value from one lifecycle cannot be reused as another object's
state merely because their display labels are similar.

## Boundary Types

Application request and response types are product contracts. They must use
domain identities and typed decisions. `Stored*`, SQL revision fields, ledger
mutation inputs, and checkpoint JSON are persistence details and must not be
accepted from platform clients.

Temporary use of a Store DTO inside an extracted port is migration debt, not a
final interface. It must be recorded in the convergence ledger and replaced by
an Application request before that port is considered complete.

## Mutation Rules

- Empty queue observation is a query/subscription, not a journalled command.
  Actual non-empty claiming remains atomic and idempotent; peeking is not a lease.
- Explicit configuration CAS is independent of stream archive throttling.
  Permission-policy updates enqueue an owner/Run command in the same commit;
  active acknowledgement is distinct from configuration persistence.

- Interactive messages enter through Turn commands and typed Host responses.
- Conversation rewind/edit/delete enter through typed user intents.
- Application derives projection mutations and commits them atomically with
  the matching Core transition where applicable.
- Import accepts only completed transcripts with explicit provenance.
- No ordinary client command may append arbitrary live messages, apply a complete projection,
  save a caller-built checkpoint, or write presentation traces as product
  state.
Authenticated remote replication is a separate source-validated import port;
it is not permission for UI code to manufacture a projection.

## Effect Rules

Application may request an effect, but only a Host executes it. Permission is
evaluated against the concrete capability and resource. A diagnostic event,
tool title, translated label, or user prompt keyword is never authority.

A suspended command retains its request identity and continuation. Resolution
returns through the typed Host response port. Clients must not replay the
original command or synthesize a successful result.

## Query Rules

Queries return typed projections. Clients do not join Store records, merge
Trace with messages, or rebuild Core Parts. A live notification carries only
identity and revision; the client reads the authoritative projection or typed
Part update.

## Automation Port

Automation is an object protocol rather than a group of table operations:

```json
{
  "type": "automation",
  "command": {
    "type": "run",
    "command": {
      "type": "claim",
      "run_id": "run_01...",
      "owner": "scheduler_01...",
      "lease_until_ms": 1787420060000,
      "now_ms": 1787420000000
    }
  }
}
```

`definition` commands manage the durable schedule and configuration. `run`
commands manage one execution and its lease. Run status is a typed Application
enum. Claim, heartbeat, retry, and finish are guarded transitions; clients do
not write status strings or lease columns directly. Unknown operation names,
unknown status values, stale leases, and removed flat automation commands fail
closed without mutation.

## Contract Gates

Each extracted port requires:

1. serialization fixtures shared by Rust and every native binding;
2. rejection tests for removed flat commands and unknown fields;
3. idempotent replay tests for mutations;
4. revision/conflict and active-Run tests where relevant;
5. cold-restart tests through the real Application Service or native FFI;
6. real-model multi-turn tests when the port participates in Agent routing;
7. screenshots opened and observed, plus recordings watched, for visible
   lifecycle changes.

A port is not complete because its unit tests compile. Completion requires the
old entry point to be absent, the real client to use the new entry point, and
the resulting product behavior to be observed.

## 当前命令入口（2026-10-28核对）

ApplicationCommand当前包含235个顶层wire tag（以本次源码映射为准），Model/Memory/Skill/Plugin等为嵌套用例。Service与embedded Application共用业务命令，不将HTTP handler直接写库作为另一条业务入口。具体声明与源码链接见[接口索引](../01-领域与协议/INTERFACE_CATALOG.md)。
