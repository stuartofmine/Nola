> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Tool activity presentation

Visual specifications and defaults are defined by [DESIGN_SYSTEM.md](DESIGN_SYSTEM.md). This is the target contract; existing clients are not assumed to have completed it.

Tool calls are first-class projections inside the assistant message that initiated the run on every Nola client. They are not transient loading indicators, independent transcript messages, or global status bars. Their authoritative source is the shared versioned `AgentTraceEvent` stream and append-only Trace Journal, so they remain available after a conversation is reopened or an interrupted run is recovered.

## Presentation contract

- The assistant bubble exists before the first model or tool event. Pending state, reasoning, every long-task phase, tool activity, Markdown output, and the final result update that same stable bubble.
- The outer process disclosure is collapsed by default, showing only a localized “Processing details” entry. Individual tool rows appear only after the user expands it; those rows show the friendly tool name, execution state, and safe short summary.
- While running, expanding a row shows sanitized arguments and affected resources, including file operation and path, URL, query, workspace, or generated artifact as applicable.
- On completion, the same row retains the real sanitized, truncated result or error. A generic `completed`, `tool_output_read`, or internal phase label must not erase parameters, resources, or a useful result.
- Expanding a completed row shows, in order: affected resources, sanitized arguments, sanitized bounded result or error, and artifact links.
- File tools expose the operation and paths without requiring the user to inspect raw JSON.
- Running, failed, and successful calls never expand automatically. Blocking failures and live permission decisions remain visible outside the process disclosure; their raw diagnostics stay collapsed. User expansion state survives event updates.
- Secrets, OAuth material, clipboard contents, and device-bridge payloads follow the existing redaction boundary before persistence or display.
- Tool rows use stable `toolCallID` identity. Queued, running, progress, completed, and failed events update in place and must not rebuild the assistant bubble or Markdown nodes.
- Reasoning and phase details publish only when their normalized visible value changes. A Trace event that is a semantic no-op advances the reducer and Journal cursor but produces no view publication or DOM/native node mutation.

## Ordering and recovery

- Clients consume the shared `AgentTraceEvent` envelope and must not maintain a second platform-specific tool-event contract.
- Duplicate events are idempotent, and events from another conversation, run, message, or tool call cannot mutate the current projection.
- Out-of-order events may be buffered. A sequence gap must trigger Journal recovery; later events cannot be displayed as if the missing transition had occurred.
- Journal replay after reopen, restart, reconnect, or stream interruption reconstructs the same tool rows and assistant content without replaying entrance animation.
- The canonical long-task Trace fixture is the cross-client compatibility gate. Every renderer must project the same tool order, sanitized parameters, resources, bounded results, terminal state, and message identity from that fixture.

## Client behavior

- iOS, Android, and macOS use clickable disclosure rows.
- Web uses disclosure rows scoped by assistant bubble, run, and `toolCallID`; reconnect and refresh resume from the persisted Trace cursor.
- CLI presents response-scoped processing status by default, with explicit keyboard/command access to process details. Debug and structured output modes retain sanitized inspectable events; they do not justify flooding the normal transcript with tool rows. This target requires client implementation and verification.
