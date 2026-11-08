> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Reference Agent Prompt and Runtime Audit

This document compares Nola with the local reference implementations under
`参考资料/`. It is an implementation audit, not a feature-marketing table.
Every adoption item must preserve Nola's local-first, data-source-driven,
cross-platform product boundary.

## Executive conclusion

The leading projects are strong for different reasons:

| Project | Strongest system-prompt property | Strongest runtime property |
| --- | --- | --- |
| Codex | Small stable policy prefix plus explicit repository instruction hierarchy | Core-owned turns, approvals, active-turn steering, durable task and tool events |
| Claude Code | Highly modular contextual reminders, durable correction memory, careful delegated-agent authority rules | Mature subagent, hook, background-session and permission workflows |
| OpenCode | Provider-aware prompt assembly and instruction loading close to the accessed resource | Persistent sessions, asynchronous prompt/abort APIs, child-session continuation and broad provider/MCP support |
| Nola | Typed Core policy, model profiles, structured capability catalogs, context planning and provenance | Local-first multi-platform Host architecture, data sources, automation, native tools and shared Rust Core |

Nola's historical weakness was not a lack of individual tools. It was that
prompt policy, permissions, task state, memory, and UI state could diverge
between clients. The correct adoption strategy is therefore to move semantics
into Core contracts and make each Host a typed executor and projector.

## System prompt comparison

### Stable policy versus turn-specific context

Codex keeps stable behavioral policy separate from project instructions and
turn context. This improves prompt-cache reuse and makes authority boundaries
legible. Nola now follows the same structure in
`core/src/agent/prompt_composer.rs`: stable safety, truthfulness, tool-order,
memory, Skill, task-completion and delegation fragments are composed separately
from the typed instruction manifest and context components.

Nola must not regress to one client concatenating a large free-form prompt.
Client-generated prose has lower authority, weak provenance, unstable cache
identity, and inconsistent behavior across iOS, Android, Web, CLI and macOS.

### Authoritative per-turn environment state

Codex rebuilds an environment `WorldState` for each turn/step. It keeps the
current working directory, workspace roots, shell, date, timezone, network
boundary and filesystem boundary separate from stable policy, and emits a
diff when those facts change. Permission instructions are another independent
state section rather than an implication derived from the user's wording.

Nola now applies the same boundary at its shared Agent Host entry point. The
Host composes one non-cacheable `RuntimeState` fragment containing the current
date, timezone, shell, active working directory, Git root, Host-exposed
filesystem roots, network policy and effective permission-profile mode. Core
accepts that fragment only as a `HostFact`; its wrapper explicitly prevents
runtime facts from overriding policy or granting access and side effects.
Internal Skill roots remain undisclosed. Concrete operations still pass
through Core permission evaluation, so this snapshot improves model planning
without becoming an authorization channel or a client-side routing shortcut.

### Repository instruction hierarchy

Codex discovers `AGENTS.md` from workspace root to the active directory and
supports `AGENTS.override.md` as the same-scope override. Claude Code uses
`CLAUDE.md` and `CLAUDE.local.md`. OpenCode loads global/project instructions
and can discover more specific instructions when a file is accessed.

Nola's implementation is split deliberately:

- `cli/src/instructions.rs` discovers and reads repository-owned files.
- `core/src/agent/instruction_hierarchy.rs` selects applicable sources,
  preserves root-to-leaf precedence, and produces a stable manifest.
- `cli/src/agent_host_driver.rs` resolves instructions for concrete mutation
  targets so a nested file operation cannot rely only on turn-start context.

Adopted in this audit:

- `NOLA.local.md` shadows `NOLA.md` in the same directory.
- `AGENTS.override.md` shadows `AGENTS.md` in the same directory.
- `CLAUDE.local.md` shadows `CLAUDE.md` in the same directory.
- Ancestor and descendant scopes remain separate; an override does not erase
  applicable parent rules.
- Instruction inspection tools and project initialization recognize the same
  filenames and `.nola/rules` directory as turn-start discovery.

Nola intentionally selects only one foreign compatibility family for a
workspace. It does not blindly stack exported `AGENTS.md`, `CLAUDE.md`, and
`CONTEXT.md` families, because those files often repeat or contradict the same
policy. Native Nola instructions and explicitly configured sources remain
additive.

### Tool selection and routing

Codex, Claude Code, and OpenCode expose typed tool schemas and let the model
select concrete calls. Nola's architectural rule is stricter because it must
remain consistent across five clients:

- No language-specific keyword routing.
- No client prompt rewrite that forces Web, memory, Skill, MCP, file, or
  platform tools.
- The model selects from the active schema.
- Core evaluates the concrete call, resource, permission, authority and result.
- Unknown-tool recovery refreshes the catalog or returns a recoverable protocol
  error; it does not guess a similarly named tool.

This is enforced by Core tool policy and the Host catalog rather than by prose
alone. Architecture questions that mention capability names remain ordinary
semantic questions and must not trigger those capabilities.

### Memory

Claude Code's most useful memory rule is that memories must be applicable,
durable and legible, while mutable claims must be verified against current
sources. Its conservative memory selector also matches what a query is about,
not isolated words.

Nola has adopted this in two layers:

- `core/src/agent/prompt_composer.rs` defines when memory may be recalled or
  written and prohibits secrets, task state, workspace-derivable facts and
  unsupported assistant claims.
- `core/src/agent/memory_worker.rs` performs typed extraction/consolidation with
  provenance, stable identities and explicit supersession.

Nola's advantage is that memory is not a hidden Markdown side effect. It is a
Core-owned runtime projection shared by Hosts. Remaining work is primarily
product observability: users need clear inspection, scope, correction, and
deletion controls on every client.

### Delegated agents and live guidance

Codex supports steering an active turn. Claude Code distinguishes teammate
guidance from user authority and prevents permission laundering. OpenCode
persists child sessions and accepts asynchronous prompts.

Nola now models delegated work as a durable Core Task with lineage, ownership,
lease, cancellation and a bounded guidance queue. A guidance message has a
stable ID and a Core-assigned FIFO sequence. The child Host acknowledges the
stable message ID only after the guidance crosses its safe execution boundary.

Implemented paths:

- Core guidance queue and acknowledgement protocol:
  `core/src/agent/task.rs`, `core/src/agent/protocol.rs`.
- Safe-boundary Host submission source:
  `core/src/agent/host.rs`, `cli/src/core_host_loop.rs`.
- CLI child-agent delivery and trusted `send_agent_message` tool:
  `cli/src/interaction.rs`, `cli/src/task_runtime.rs`.
- Web task guidance endpoint and visible task-card input:
  `cli/src/web.rs`, `web/app/templates/chat.html`.
- Native task-projection models:
  `Nola/Services/AgentCoreRuntime.swift`,
  `android/app/src/main/java/com/nola/core/AgentCoreTaskProjection.kt`, and
  `NolaMac/Models/MacCoreTaskProjection.swift`.
- Encrypted mobile Host controls and cursor-resumable observation:
  `Nola/LLM/WebAgentProvider.swift`,
  `android/app/src/main/java/com/nola/data/remote/api/WebHostApiServiceImpl.kt`,
  and `android/app/src/main/java/com/nola/ui/screen/chat/ChatViewModel.kt`.
- macOS Host stdin multiplexing:
  `cli/src/desktop_host_control.rs` and
  `NolaMac/Services/MacLocalAgentRuntime.swift`.

Delegated guidance carries teammate authority only. It can refine work but
cannot approve a permission, alter configuration, or impersonate the user.

### Compaction and continuation

All three reference projects preserve a continuation summary rather than
blindly truncating recent messages. Nola's Core owns context usage and
compaction decisions; the LLM produces a structured semantic summary while
code preserves recent messages, tool-result anchors, task state, permission
state and full transcript replay.

The stable invariant is that compaction may reduce provider input but must not
rewrite history, lose unresolved constraints, or turn an unverified outcome
into completion. CLI, Web and native context meters must project the same Core
snapshot rather than estimating from message count.

Provider output completion is now treated as a separate protocol concern.
Nola retains streamed text and reasoning deltas as the authoritative content,
uses the terminal provider event for finish reason and usage, and only projects
a continuation segment after overlap removal. Protocol-correction retries and
length-continuation requests have independent budgets, so an invalid tool call
cannot consume the allowance for a truncated answer. Exhausting that allowance
is an explicit recoverable incomplete-output failure rather than a false
successful completion. Tool-capable requests buffer only a bounded semantic
prefix; they no longer hold the entire final answer until the terminal event.
The provider adapters also require an explicit terminal protocol signal.
Disconnecting before the first event remains safely retryable; disconnecting
after text marks the segment incomplete and enters bounded continuation; a
disconnect during tool-call arguments discards that partial call and fails
instead of executing uncertain JSON. EOF and idle timeout can therefore no
longer be silently rewritten to a successful `stop` finish reason.

### Verification evidence

Claude Code explicitly requires verification before a completion claim, while
Codex distinguishes interactive development from the slower final validation
phase. Nola now expresses the stronger cross-platform version as the stable
Core fragment `nola.core.verification-evidence`: evidence must match the scope
of the claim. Compilation, static inspection, unit tests, integration tests,
end-to-end sessions and real-model runs are not interchangeable. The same rule
also prevents routine tests from touching production credentials, user data,
Keychain, Accessibility or screen-recording permission unless that boundary is
the explicit subject of the test.

### Provider-native prompt caching

Codex benefits from a stable prefix and provider-specific request adaptation.
Nola has typed model capability profiles and includes instruction identity in
the invocation plan. The adopted boundary is explicit:

- Core decides whether prompt caching is enabled for the concrete model turn.
- Core supplies the complete model-prefix digest, including the stable policy,
  applicable instruction manifest and exact Step tool projection.
- A Host/provider adapter may namespace that digest for provider and model wire
  requirements, but a client UI or conversation store must never invent a key.
- Provider-reported cache-read/cache-write usage and Core invalidation evidence
  are projected independently; a configured key is not evidence of a hit.

CLI, iOS and Android now gate provider cache identity on Core's
`model_invocation_plan.prompt_cache`. The iOS non-event-stream compatibility
path was corrected during this audit so it cannot bypass that decision. Native
context inspectors consume the same `prompt_cache_diagnostic` and reported
usage fields as CLI/Web rather than estimating hits from prompt similarity.

The remaining gap is operational evidence: reconnect, model switching,
instruction changes and Step tool-projection changes still need cross-platform
sessions proving the expected invalidation reason and provider token accounting.

## Recoverable background-control lessons

The reference projects separate command acceptance from event delivery instead
of keeping one chat response alive as the source of truth:

- Codex remote control addresses a client stream with `client_id` and
  `stream_id`, orders server envelopes with `seq_id`, and acknowledges chunks
  independently. A reconnect therefore retains and replays only unacknowledged
  wire segments.
- Claude Code keeps a background Agent's pending inputs, displayed transcript,
  and completion notification as distinct state. Its `notified` guard prevents
  a background completion from being injected twice.
- OpenCode accepts asynchronous prompts without waiting for the answer and
  publishes session status through an independent server-sent event stream.

Nola already has stable Task IDs, ownership leases, persisted task events,
idempotent guidance message IDs, and Core acknowledgement state. The missing
transport must preserve those identities rather than inventing UI-local ones:

1. Every persisted Task event receives a monotonic cursor within its owning
   session.
2. A subscriber resumes strictly after its last acknowledged cursor.
3. Command acceptance, child acknowledgement, progress, and terminal state are
   separate events; an HTTP `202` proves only acceptance.
4. Replayed events are idempotent by Task ID, projection revision, guidance
   message ID, and event cursor.
5. Slow or disconnected clients never block the Task worker and never require
   the original chat stream to remain open.

The encrypted Host bridges now support Task guidance/cancellation, immediately
return the authoritative Core projection, and expose a separate cursor-resumable
Task SSE. macOS, iOS and Android reconnect that stream without replaying the
command and merge only non-stale projection revisions. This is implemented but
still needs disconnect, backlog, duplicate-event and terminal-state validation
before it counts as reliable evidence.

### Checkpoint replay is not request re-execution

Codex keeps process identity and cancellation state attached to a managed
process, while OpenCode records interrupted tool parts instead of treating an
abort as permission to rerun them. Nola now makes the same distinction in
Core: only effects declared `Idempotent` are automatically delivered again
after checkpoint restoration. A `Recoverable` effect remains visible through
`pending_recovery_effect_count` and requires inspection or an explicit resume
decision; `RequiresRecoveryDecision` remains stricter still. A Host response
cursor may replay already-produced events, but it never proves that repeating
an arbitrary tool invocation is safe. This prevents restart recovery from
duplicating file writes, document creation or external side effects.

### One cancellation primitive for child processes

Codex routes shell execution through managed processes whose output draining,
cancellation token, process identity and confirmed termination share one
lifecycle. OpenCode likewise binds an `AbortSignal` to the tool process and
persists an interrupted terminal part. Nola's main Bash tool already terminated
its process group, but internal formatter and `git apply` helpers bypassed that
path with blocking `wait_with_output()` calls. They now use a shared bounded
waiter that continuously drains both output pipes, observes the turn's
`ExecutionCancellation`, enforces a timeout and terminates the whole process
group before returning cancellation. This removes a class of turns where Esc
stopped the UI/model but left an internal formatter or patch subprocess alive.
The execution scope now carries the actual turn token in addition to a numeric
diagnostic ID, so deeply shared Git helpers can consume the same cancellation
without threading a duplicate parameter through every analysis function.

## Runtime feature comparison

| Capability | Codex | Claude Code | OpenCode | Nola status |
| --- | --- | --- | --- | --- |
| Active-turn steering | Native turn steer | Foreground/background agent messages | Async prompt API | Core, CLI and Web expose typed controls; macOS, iOS and Android route guidance/cancellation to the active local coordinator or paired Host and consume cursor-resumable authoritative projections |
| Durable delegated work | Collaboration agents | Subagents and background sessions | Child sessions | Core Task lineage, lease, retry, cancel and guidance implemented |
| Permission continuity | Structured approvals and sandbox | Rich allow/deny/session workflows | Tool permissions | Core projection and scoped grants implemented; Host UI parity still being completed |
| Project instructions | Hierarchical AGENTS + override | CLAUDE + local/rules | Global/project plus read-time discovery | Typed hierarchy and mutation-target discovery implemented; override semantics adopted |
| Per-turn environment | Structured WorldState with cwd, roots, shell, date/timezone, network and filesystem boundaries | Contextual runtime reminders | Session/environment state | Shared Agent Host emits one typed, non-cacheable RuntimeState HostFact; focused cross-platform validation remains open |
| Memory | Limited product memory emphasis | Mature correction/project memory | Session-oriented persistence | Typed cross-session memory runtime; inspection UX remains uneven |
| Hooks | Tool/session lifecycle hooks | Extensive hooks | Plugins/events | Core lifecycle hook registry exists; client management UX remains incomplete |
| MCP/Skills | Skills/plugins/MCP | Skills/plugins/connectors | MCP/plugins | Core catalogs and isolated Skill runtime implemented; gallery/editor parity remains ongoing |
| Context compaction | Model-assisted continuation | Model-assisted summaries | Session compaction | Core-owned model+code compaction implemented |
| Data sources | Workspace-centric | Connectors/browser/project | Provider/plugin ecosystem | Nola advantage: native personal data sources with provenance and freshness contracts |
| Automation | Background tasks | Background sessions/scheduled tasks | Session APIs | Nola scheduler plus data-source/Skill orchestration; reliability evidence still incomplete |
| Cross-platform native product | CLI/TUI/app server | CLI/desktop surfaces | TUI/Web | Nola advantage: iOS, Android, macOS, CLI and Web sharing Core contracts |

## Nola-specific advantages to preserve

1. **Data-source-driven context.** Health, calendar, reminders, files and other
   sources have authorization, freshness, provenance and disclosure metadata.
2. **Local-first execution.** A backend outage must not disable local models,
   local memory, local data inspection or local automation.
3. **Core-owned semantics.** Permission, memory, context, tasks, Skills and
   tool policy are portable contracts rather than duplicated client logic.
4. **Native presentation.** Each UI may use platform-native interaction and
   animation, but it must project the same ordered events and durable state.
5. **Explicit Host boundary.** CLI, Web and Mac are different product surfaces;
   using the same engine does not make a native app a disguised terminal.

### Resource access is not execution authority

Open WebUI's flat read/write grants make shared Skills easy to browse, but its
server-owned model does not establish a local Host, content revision, or
execution boundary. Nola now evaluates managed-resource access in Core using a
resource kind/ID, principal, optional Host, revision, content digest and expiry.
Metadata read, content read, content edit and sharing management are distinct.
Public grants cannot mutate resources, and every decision explicitly leaves
tool execution unauthorized. Skill activation and MCP tool calls therefore
continue through the independent capability and permission contracts.

## Remaining implementation gaps

1. Validate the reconnectable Task event stream under disconnect, replay,
   duplicate delivery, a full 256-event backlog, task cancellation and terminal
   completion. The protocol and macOS/iOS/Android consumers now exist;
   cross-platform validation evidence remains open.
2. Add real-provider operational evidence for prompt-cache reuse and
   invalidation across the native clients; their cache identity and diagnostics
   now follow the same Core invocation contract as CLI/Web.
3. Make memory, Skill, MCP, data-source and automation inspectors equally
   editable and localized across all clients.
4. Add real-model cross-platform sessions proving that semantic tool selection
   does not depend on keywords, including multilingual and ambiguous prompts.
5. Add crash/reconnect evidence for queued guidance, pending permissions,
   background Tasks, compaction and attachment replay.

## Verification status

This audit records implementation evidence and current gaps. The changes made
during this pass have not yet been compiled or exercised end to end. The goal
must remain active until focused contract tests and real-model cross-platform
sessions prove the listed behavior.
## Cancellation completion is an observed state, not an acknowledgement

- Web cancellation now separates `cancelRequested` from `terminationConfirmed`. The latter only becomes true after the registered execution has actually left the active-run registry.
- Computer and accessibility Python helpers now share the same cancellation token, process-group termination, and bounded timeout semantics as Bash, Git, and formatter subprocesses.
- Native computer waits, clipboard polling, application polling, and window polling now observe the same turn cancellation token at short intervals instead of sleeping for up to 300 seconds uninterruptibly.
- A Host may acknowledge receipt immediately, but clients must keep the turn in a cancelling state until a terminal Core trace or an observed execution termination confirms completion.
- Core's terminal Host response is now named `cancellation_confirmed`, not `cancellation_acknowledged`. The protocol contract requires the target execution to have exited before this terminal response is emitted.
- iOS and Android device Hosts emit the same `cancellation_confirmed` wire value; Android's terminal-response classifier was updated with the protocol rather than retaining a platform-only legacy state.
## Provider streams require an explicit terminal event

- iOS and Android OpenAI-compatible providers now require `[DONE]` or a non-empty `finish_reason` before emitting model completion.
- Transport EOF without an explicit terminal event is a retryable incomplete-stream failure, not an implicit `stop`.
- Incomplete streamed tool-call arguments are never flushed or executed after a disconnected stream.
- The provider-neutral iOS and Android `TextAgentModelClient` layer independently requires an explicit typed `Completed` event, so a custom provider cannot bypass the transport terminal invariant by silently closing its stream.
## Continuation stitching is Core-owned

- Reconnected model segments are stitched by `nola_core::agent_model::stitch_agent_model_continuation`.
- CLI no longer owns a private overlap implementation. Swift and Kotlin access the same policy through the Core C/JNI ABI.
- The shared policy removes sufficiently long repeated prefixes and avoids reopening unfinished Markdown code fences.
- Mobile model adapters keep the first attempt live, buffer one recovery attempt, and expose only the Core-verified unique suffix. They never replay a partial stream after tool-call arguments have begun.
- A recovery response without a provable overlap is rejected as divergent instead of duplicating or silently replacing visible text.
- A recovery response must add text, reasoning, or a complete tool-call stream. An explicit terminal event with no progress cannot promote an earlier partial answer to completed state.
