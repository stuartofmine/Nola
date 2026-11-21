# Cross-module workflows and recovery

[简体中文](跨模块流程与恢复.md) · English

| Workflow | Normal path | Interruption and recovery |
| --- | --- | --- |
| Create or send | The client retains its draft, submits under the conversation identity and observes the admission receipt. The committed run drives model and tool work. | If admission loses its acknowledgement, look up the original request before resubmitting. Switching windows changes only the observer. |
| Edit and resend | The user chooses an earlier message. Application checks the current source and revisions, rewinds the visible suffix and its context, then accepts the replacement in the same conversation. Linked expert history is scoped to what the edit affected. | A stale source or a conflicting active operation must preserve the draft and expose the conflict. Recover a remotely accepted edit from its source receipt. |
| Model call and tool action | A frozen profile and request purpose select a route. A tool call is validated against the run catalog and concrete resource. The Host returns execution evidence. | Reconcile an uncertain external effect. A failed tool attempt and a later successful attempt remain distinct. The reply’s reasoning display does not acquire a failure status from the tool. |
| Compact or continue | Plan against the full input budget, retain required context and validate summary lineage on commit. An eligible physical request snapshot supports prefix reuse. | Resume the stored maintenance intent. Retain bounded partial output for the supported continuation path. Cancelled or empty output cannot replace a valid summary. |
| Main agent, Goal and expert | Admit the user’s objective in the main context. Bind a delegated Task to its parent and frozen member configuration. Persist acceptance before waking the worker. | Restore the original Task. A missing reply or wait timeout leaves its outcome open. An expert returns unresolved questions to the main agent. |
| Usage and cache | Each physical attempt owns its counters. A later report revises its own record. Calculate cache ratios from counters with the appropriate denominator. | Preserve unavailable counts as unknown and deduplicate repeated reports. Compaction usage is maintenance usage. |
| Remote observation and cancellation | Authenticate the source Host and resume the journal cursor. To stop work, send a cancellation for that run. | A disconnected observer can return later. Verify the Host’s stop evidence before publishing a cancelled outcome. |

See [contracts and receipt outcomes](../MODULE_CONTRACTS.en.md), [context](../模块/09-上下文与压缩/README.en.md), [experts](../模块/10-Goal任务与子代理/README.en.md) and [remote execution](../模块/14-远程Host与同步/README.en.md).
