# System design

[简体中文](SOFTWARE_DESIGN.md) · English

Nola runs on the user’s devices. A client submits work to an Application instance bound to a Store. A model endpoint generates output. An execution Host owns the physical operation and its credentials. Those roles can run on different machines. Choosing a remote model still permits local tool execution when the run’s policy and Host allow it.

## From user input to a visible reply

1. The client submits a conversation-scoped command. Application checks identity, source, expected revision and receipt history. A read returns a snapshot or cursor.
2. The agent engine reduces the input against committed state and frozen configuration. Application commits the transition, checkpoint and next work through Store Port. Bootstrap supplies the concrete Store.
3. A driver or worker claims committed work. The model or tool Host performs I/O. Its observations carry the original effect, attempt and execution generation.
4. Application accepts the matching observation, commits the resulting state and publishes a journal increment. The client updates the existing message parts. A late reply to another conversation cannot unlock this composer.

[Reducer](../../../crates/nola-engine/src/lib.rs) · [Application commands](../../../crates/nola-app/src/daemon.rs) · [Driver](../../../crates/nola-engine/src/loop_runner.rs) · [Journal port](../../../crates/nola-store/src/db.rs)

## Identities and ownership

| Identity | What survives a view or process restart |
| --- | --- |
| Store / authority | The namespace that owns the record and its revisions. Replicas retain source identity and epoch. |
| Agent / participant | An agent is reusable configuration. A participant binds a specific configuration revision to one conversation. |
| Conversation / context | The conversation owns its public history. Each participant has a scoped working context. The coordinator has its own context. Room membership and dispatch still have explicit Application records. |
| Run / turn | A run identifies an admitted execution. A turn identifies an interaction boundary. Read their persisted relationship. A sealed run keeps its terminal result. |
| Effect / delivery / attempt | An effect requests work. A delivery identifies a dispatch under a lease. An attempt records a physical model or tool operation and its usage. |
| Goal / Task | A Goal carries an objective and acceptance conditions. A Task owns a unit of delegated work and its result. Inspect both before declaring the objective complete. |
| Window / draft | Selection, scrolling and disclosure belong to the client. An unsent draft is device editing state. The conversation ledger owns sent messages. |

## Current runtime paths

| Scenario | What commits and how it recovers |
| --- | --- |
| CLI or Web startup | Read user and project layers. Publish the catalog, enabled models and default in one transaction. A rejected catalog leaves the committed selection available. Resuming a conversation uses its binding. [File configuration](../模块/08-模型配置与调用/MODEL_FILE_CONFIGURATION.en.md) |
| Send, edit and resend | Bind submission to the original conversation and request ID. An edit rewinds the affected suffix and working context in that conversation, then admits the replacement execution. A stale edit must reload the source state. [Edited resend](../../../crates/nola-app/src/web_server.rs) |
| Main agent and experts | The main agent receives user input and dispatches Tasks. An expert reuses its child conversation under that parent and returns its result. Temporary subagents share the Task execution path. [Goals and experts](../模块/10-Goal任务与子代理/README.en.md) |
| Remote long task | Pair and authenticate with the source Host. Reattach to the admitted run and journal cursor. Reconcile the receipt after a disconnect and obtain stop evidence from the executor for cancellation. [Remote Hosts](../模块/14-远程Host与同步/README.en.md) |
| Desktop local model | Inspect hardware and engine readiness on the inference machine. Recommend weights and a fitting context. Confirm the plan before downloading, then verify the served model ID after launch. [Models](../模块/08-模型配置与调用/README.en.md) |
| Extensions and UI | Install Skills at the chosen location. Discover MCP tools on their Host. Desktop applications discover plugin pages from installed declarations. Each call still passes run admission. Core owns A2UI surface state while the client owns pending edits. [Extensions](../模块/11-Skill插件与MCP/README.en.md) · [Clients](../模块/13-客户端投影/README.en.md) |

## Persistence and recovery

Application coordinates writes within one authority. Desktop processes reach it through the local Service. Mobile builds can embed the same use cases. Store implements transactions and integrity checks. A stream can append canonical text chunks. Explicit edit and deletion commands change the visible history through revisions and tombstones. Compaction retains original text and its scope.

After a lost commit acknowledgement, look up the original request. After a lost external result, inspect the original effect and attempt on the executing Host. These are separate recovery operations. A database transaction can deduplicate admission, but a remote process may already have performed a physical action. A UI cancel request, delivery of that request and evidence of a stopped action are separately observable.

Across Stores, the source Host owns the run. The observing device stores validated projections and its cursor. Reconnect from that cursor and check the source identity. An observation can be retried without launching a second copy of the work.

## Model, context and authorization

A profile binds an endpoint, protocol, capabilities and account reference. Application resolves session overrides and the Host freezes the actual invocation. Each provider attempt has a budget, cancellation scope and independent usage. Only the executing Host resolves private credentials. A model’s advertised capabilities do not convey user or OS permission.

Core selects history for compaction. The maintenance request receives a budget for the whole resulting input, including instructions, schemas and retained tail. Commit a validated summary with provenance. A cache hit requires provider usage from the actual attempt. Unknown counters remain unknown. [Context and compaction](../模块/09-上下文与压缩/README.en.md)

## Client projection and acceptance

Clients apply committed increments to stable message and part IDs. Transient progress is bound to its run and attempt. A terminal seal closes the stream, and the client preserves the reader’s position. Late usage revises its owning attempt. Parent and child totals must avoid double counting.

Review manifest edges, transaction regressions, actual model calls and native interaction separately. An observable button, a compiled binding and a successful remote run answer different questions. [Implementation map](CURRENT_IMPLEMENTATION_MAP.en.md) · [Dependencies](DEPENDENCIES.en.md) · [Module contracts](../MODULE_CONTRACTS.en.md) · [Version policy](VERSION_COMPATIBILITY.en.md)
