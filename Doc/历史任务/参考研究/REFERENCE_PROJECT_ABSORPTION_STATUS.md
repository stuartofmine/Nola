> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Reference-project absorption status

This document turns the Codex CLI, Claude Code, and OpenCode comparison into
implementation checkpoints. It complements
`REFERENCE_PROJECT_PROMPT_AND_RUNTIME_AUDIT.md` and
`REFERENCE_PROJECT_CAPABILITY_MATRIX.md`; it is not a claim that the listed
work has been compiled or verified.

## System-prompt design

| Reference strength | Nola direction | Current evidence | Remaining gap |
| --- | --- | --- | --- |
| Codex composes stable policy, project instructions, runtime context, and the user turn as separate layers. | Keep policy and capability contracts stable; inject project and data context as typed components rather than rewriting user text. | `core/src/agent/context.rs`, `cli/src/project_instructions.rs`, `SharedPrompts/` | Prove identical precedence and prompt snapshots across CLI, Web, iOS, Android, and macOS. |
| Codex rebuilds authoritative environment and permission WorldState independently of stable policy. | Agent Host emits current runtime facts once per turn as non-cacheable `HostFact`; user text cannot alter or authorize them. | `core/src/agent/prompt_composer.rs`, `cli/src/core_agent_runtime.rs` | Compile and prove cwd/root/shell/date/timezone/network/permission changes produce the expected dynamic prompt diff on every Host surface. |
| Claude Code describes tools by schema and lets the model select them. | Tool routing must be schema-driven; Core evaluates the concrete call after selection. | `core/src/agent/tool_policy.rs`, repository `AGENTS.md` guardrails | Remove any remaining client-side prompt rewrites or language-specific routing shortcuts. |
| OpenCode keeps provider/model quirks outside the semantic user request. | Normalize provider protocol at the Host boundary without mutating user intent. | `cli/src/provider.rs`, `cli/src/agent_host_driver.rs` | Consolidate provider compatibility and reasoning replay behavior in one tested adapter. |
| Leading agents compact into a structured continuation artifact, not a plain transcript truncation. | Preserve task state, decisions, files, permissions, failures, and unresolved work in Core-managed compaction. | `core/src/agent/context.rs`, CLI compaction flow | Add cross-platform golden fixtures proving equivalent continuation after compaction. |

## Runtime and feature design

| Reference strength | Absorbed Nola behavior | Current evidence | Remaining gap |
| --- | --- | --- | --- |
| Structured turn events separate reasoning, tool calls, results, permissions, and final output. | Agent Host emits typed lifecycle events and clients render one ordered turn timeline. | `core/src/agent/events.rs`, `cli/src/agent_host_driver.rs` | Finish event parity and reconnection replay for every client. |
| Permissions are scoped grants evaluated against a concrete operation. | Resource visibility and tool execution are separate contracts; sharing never authorizes execution. | `core/src/agent/resource_access.rs`, `cli/src/resource_access.rs` | Persist account/device/group principals and expose grant editing consistently. |
| Tool results are bounded while full artifacts remain addressable. | Context receives truncated structured results and artifact references instead of unbounded output. | Core context/result contracts and CLI Host adapters | Audit every tool adapter and add large-output end-to-end cases. |
| Stream deltas are durable content while terminal events carry completion metadata. | Host reconciles final text/reasoning from streamed deltas, deduplicates continuation segments, separates recovery budgets, and refuses to execute tool calls from unterminated streams. | `cli/src/llm.rs`, `cli/src/agent_host_driver.rs`, `cli/src/model_output.rs` | Compile and run truncated Markdown, reasoning-only, terminal-snapshot mismatch, disconnect and native-client rendering E2E cases. |
| Long operations have explicit task identity, progress, cancellation, and resumable state. | Background work is a task only when duration warrants it; foreground turns remain directly cancellable. | Core task contracts and platform task coordinators | Finish acknowledgement, reconnect, and cancellation conformance across local and remote Hosts. |
| Crash recovery distinguishes event replay from side-effect replay. | Only idempotent effects automatically cross checkpoint restoration; recoverable effects remain pending for inspection or explicit resume. | `core/src/agent/runtime/effect_dispatch.rs`, `core/src/agent/runtime/bootstrap.rs` | Compile and exercise crash points before acceptance, during streaming, after side effects and before terminal acknowledgement. |
| Child-process cancellation uses one lifecycle primitive. | Formatter, patch and shared Git subprocesses drain output, honor the scoped turn token, time out, and terminate their process group instead of blocking in `wait_with_output`. | `cli/src/tools/mod.rs`, `cli/src/interaction.rs` | Migrate remaining platform helper probes, then run cancellation E2E on macOS/Linux. |
| Skills and MCP are managed capabilities rather than prompt snippets. | Stable IDs, owner/grants, revision, digest, discovery metadata, and content access are evaluated independently. | `core/src/agent/resource_access.rs`, `cli/src/builtin_skills.rs`, `cli/src/mcp_host.rs` | Complete the same managed-resource contract for all data-source and prompt-template stores. |
| Context inclusion is an explicit plan. | A resource may be visible in UI without being readable by the model; only `ReadContent` data sources enter ContextPlan. | `cli/src/resource_access.rs`, `cli/src/web.rs` | Apply the same projection to every platform context assembler and verify token accounting. |

## Invariants Nola must preserve

1. User text is never rewritten to force a tool, Skill, MCP server, memory, or data source.
2. The model selects a typed capability; Core authorizes the concrete operation and resource.
3. Resource grants control metadata/content/edit/sharing only and never bypass tool execution policy.
4. UI projection, model-context projection, and runtime connection configuration are distinct views.
5. Permission suspension is a protocol state, not an error or a second model turn.
6. Tool history records normalized call, bounded result, artifacts, and status; it does not retain the full per-round tool catalog.
7. Compaction preserves semantic continuation state and the complete transcript remains replayable.
8. Every platform consumes the same Core decisions while retaining platform-native tools and UI.

## Verification still required

- Rust compilation and unit tests for managed Skill, MCP, and data-source reconciliation.
- Real-model multilingual cases proving there is no keyword routing.
- Permission allow-once, conversation, persistent, deny, custom-response, and cancellation sessions.
- MCP credential redaction and credential-preserving edit cases.
- Context token/count parity between footer, inspector, Host request, and persisted conversation.
- Cross-platform event-order and streaming-rendering fixtures.
### Cancellation and helper process lifecycle

- [x] Web cancellation distinguishes request acceptance from confirmed execution termination.
- [x] Python computer/accessibility helpers participate in the active turn cancellation token and process-group cleanup.
- [x] Native computer wait and polling tools are cooperatively cancellable.
- [x] Core Host protocol uses a terminal `cancellation_confirmed` response whose contract requires observed target execution exit, rather than treating receipt acknowledgement as completion.
- [x] iOS and Android device Host adapters use the same confirmed-cancellation terminal contract.
### Mobile provider stream integrity

- [x] iOS and Android require explicit upstream stream completion.
- [x] Mobile device Hosts do not finalize partial streamed tool arguments after EOF.
- [x] Provider-neutral mobile Agent adapters reject typed streams that close without `Completed`.
- [x] Mobile partial-text continuation retries once, buffers the recovery stream, and releases only the Core-verified suffix.
- [x] Empty recovery completions cannot finalize an earlier partial response.
- [x] Continuation overlap and Markdown-fence stitching policy is implemented once in Core and exposed to CLI, iOS, and Android.
