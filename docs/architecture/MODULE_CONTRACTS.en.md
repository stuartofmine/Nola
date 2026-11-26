# Module contracts, receipts and failure handling

[简体中文](模块契约.md) · English

Application commands carry `protocol_version`, `request_id`, `client_id`, optional revision preconditions and the command payload. The authenticated principal, authority and request identity scope deduplication. Reusing a request identity binds the caller to its original payload. A model catalog change is scoped to a Host. A conversation mutation additionally requires the conversation owner. Host delivery has a separate namespace for the run, effect, attempt and worker generation.

## Outcomes and the next operation

| Observation | Meaning | Recovery |
| --- | --- | --- |
| Accepted | The request has been durably received. | Query or observe the same request. |
| Committed | The use case committed its business facts and follow-up intent. Physical work may remain. | Use the returned revision or cursor. |
| Rejected | The change was definitely refused. | Correct the failing input or authorization before a new intent. |
| Conflict | A revision, owner or existence precondition failed. | Preserve the draft, reread the authoritative record and resolve the conflict. |
| Transport lost or commit acknowledgement missing | The client cannot determine whether the mutation committed. | Reconcile the original request ID. |
| External effect uncertain | A Host may already have performed an action. | Inspect the effect and external evidence before any physical retry. |

The first four outcomes are stored receipt tags. Read-only queries can return a snapshot without creating a business receipt. Service failures expose `code`, `message`, `recoverable` and optional `details`. Some internal boundaries return strings. A recoverable flag alone cannot establish that repeating an action is safe. [Service types](../../crates/nola-types/src/lib.rs) · [Client command outcomes](../../crates/nola-app/src/daemon.rs)

## Applying the failure to its owner

| Failure | Owner and action |
| --- | --- |
| Invalid configuration or unsupported capability | Refuse admission and identify the field that can be corrected. |
| Permission or OS refusal | Explain the resource and authority that can change the decision. Preserve its scope. |
| Model limit, rate limit or incomplete output | Inspect the original attempt. Apply the corresponding context, backoff or continuation policy. |
| Missing evidence or inconsistent result | Keep the scope and uncertainty visible. Require an observation of the actual resource. |
| Persistence or invariant failure | Reconcile the commit. Reject cross-conversation results and writes after a terminal seal. |

## Module directory

- [MOD-01 · Domain and protocols](模块/01-领域与协议/README.en.md)
- [MOD-02 · Core and the agent state machine](模块/02-Core与状态机/README.en.md)
- [MOD-03 · Application commands and transactions](模块/03-Application与事务/README.en.md)
- [MOD-04 · Storage, journals and indexes](模块/04-存储与索引/README.en.md)
- [MOD-05 · Execution, cancellation and recovery](模块/05-执行与恢复/README.en.md)
- [MOD-06 · Tools and execution evidence](模块/06-工具与证据/README.en.md)
- [MOD-07 · Permissions and platform capabilities](模块/07-权限与平台/README.en.md)
- [MOD-08 · Model configuration and inference](模块/08-模型配置与调用/README.en.md)
- [MOD-09 · Context, compaction and recall](模块/09-上下文与压缩/README.en.md)
- [MOD-10 · Goals, tasks and experts](模块/10-Goal任务与子代理/README.en.md)
- [MOD-11 · Skills, plugins and MCP](模块/11-Skill插件与MCP/README.en.md)
- [MOD-12 · Memory, data sources and automation](模块/12-记忆数据源与自动化/README.en.md)
- [MOD-13 · Clients, conversation projections and A2UI](模块/13-客户端投影/README.en.md)
- [MOD-14 · Remote Hosts and synchronization](模块/14-远程Host与同步/README.en.md)
- [MOD-15 · Measurement, tests and release acceptance](模块/15-观测与验收/README.en.md)
