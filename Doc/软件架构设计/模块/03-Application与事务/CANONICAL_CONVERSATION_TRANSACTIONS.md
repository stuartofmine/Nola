> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Canonical Conversation Transactions

## 工作、执行与上下文所有权（2026-11-02）

实体关系统一见[会话、Run、Goal 与子任务边界](../10-Goal任务与子代理/AGENT_THREAD_ARCHITECTURE.md)。公开写命令保留准确 Conversation/Run/Task/owner/revision；补充现有 Task 走既有 guidance FIFO 与水位回执，不借当前选中会话改写归属。Task 续接新建工作记录，封存 Run 不重开。Goal 的 incomplete 控制落为现有 ConcludeIncomplete，保留未完成要求，无自动 wake。

活动详情是只读投影：context_scope 从 conversation execution ownership 派生，execution_host_id 仅来自已记录的 remote Run 来源；缺少记录显示未知，不猜当前客户端即执行 Host。不返回凭据或整个 invocation 配置；来源导航仍需原会话核验合法操作。32 条/32KB 冷工作集不改变 canonical 原文和按授权召回的读取范围。

## Source of truth

### Context compaction boundaries and acceptance

Compaction operates on a model-context projection, never on canonical transcript
bodies. Keep persistent instructions and the latest user request verbatim. Select
a bounded recent tail at completed tool-batch boundaries, including inside a single
long user turn. Overlapping calls/results remain together; an unresolved batch is
retained rather than summarized as completed work. Cold restore uses the same plan.

The summary body budget is the smaller of available context space and half the
archive's estimated tokens after reserving four framing tokens. An empty or too-small
archive cannot receive an expansion-sized budget. Automatic threshold compaction
skips archives below 64 estimated tokens; hard provider-overflow recovery remains a
separate path. Validate both the output budget and strictly positive net savings
before consuming the pending plan or accepting the Host response. This also applies
to restored plans with previously oversized budgets.

Independent completion review is model-selected, not automatic or user-configured.
Core offers a typed `request_completion_review` control tool on eligible native
tool-call steps. The model may answer directly, or call it with the complete
candidate answer as its sole tool call. Core verifies that the control tool was
offered, records its result, and only then starts the separate review request.
Tool availability and prior tool execution alone never schedule a reviewer.
Direct completion still obeys Core's permission, tool-receipt, iteration-budget,
and unfinished-work rules. It does not prove the truth of arbitrary prose claims;
an optional reviewer cannot provide that guarantee either. A typed completion-
claim contract could bind action claims to effect receipts without user-language
keyword routing, but is not a prerequisite for ordinary direct answers.

### Completion review subject identity

Independent review receives a Core-built `nola.completion_review.v1` packet, not
a live user instruction to “review this evidence.” Its original user messages,
current-turn user IDs, candidate answer, Host-confirmed mutation IDs, and other
evidence are separate fields. `context_item_order` preserves their chronology;
user bodies and the candidate are not duplicated in the evidence collection.
The review JSON format applies to the reviewer only, never to the user's answer.
Embedded text cannot create new user authority. Accepted candidates still publish
verbatim, and existing receipt/unfinished-work checks still govern completion.

### Cross-platform ownership

Host envelopes carry two independent attempt identities. The optional top-level
`attempt_id` identifies the logical model attempt when applicable. The required
claimed-effect lease at `delivery.attempt_id` identifies a physical delivery
attempt, including permission and cancellation effects without any model attempt.
Never require these IDs to be equal or invent a model attempt to admit a tool or
permission effect. Model-specific execution retains its own attempt validation.

Claim, recovery, heartbeat, and response admission validate the physical lease's
attempt, worker generation, owner, token, and deadline. Renewal matches the prior
physical lease, not the model attempt. Responses preserve both identities in
their respective fields. Cross-platform regression fixtures must include absent
model attempts, different model/delivery IDs, and foreign or expired leases;
fixtures assigning the same ID to both do not prove this contract.

Permission history is an execution-owned receipt sequence, persisted in the
checkpoint independently from pending permission UI. Each receipt retains the
request, turn, call, capability, resource, and a discriminated outcome:
authorization, interruption, or guidance. Returning to the composer is an
interruption, not a denial; neither it nor cancellation revokes existing rules.
Automatic resolution of sibling requests and delegated permissions use the same
receipt contract. A tool cancellation also preserves its reason in model context.

Each model request receives fresh Core permission rules and the latest 32
permission receipts in chronological order. The complete receipt history remains
durable; this bounded projection does not replace authorization evaluation.
Independent review receives the same rules and receipts as explicit packet
fields and checks the current request, not merely the previous task. Grants are
not execution evidence; only actual successful tool receipts establish execution.
The review's `constraints` index is advisory and may be omitted (normalized to
an empty array); malformed values still fail. Decision/outcome consistency,
nonempty acceptance evidence, explicit violations, and effect receipt checks
remain mandatory. Missing advisory metadata must not trigger a whole model call.

Core emits `model_generation_started` and `model_review_started` at model-request
boundaries. CLI review activity survives provider-attempt notifications until
generation resumes or the run terminates. Private review JSON must never be
rendered as an answer. Hosts may opt in through the checkpointed
`supports_reviewed_text_parts` capability. Such hosts append candidate text to
the transcript with a durable `pending` review state. Approval changes the same
Part to `approved`, without regenerating, moving, or duplicating its body.
Rejection retains the original body as `rejected` and appends any revision as a
new Part. Cancellation/failure settles pending drafts as `interrupted`.
Draft visibility is not completion authority: permission, effect receipt, and
unfinished-work checks remain mandatory, and review JSON remains private.

Only ordinary or approved text enters restored model history and the durable
assistant body shown by CLI. Pending, rejected, and interrupted candidates stay
in the canonical Part ledger for diagnostics, but neither live terminal output
nor session replay presents them as an answer. The CLI displays review progress
without appending an unapproved body it cannot retract. JSON clients receive
canonical Part updates; native clients must distinguish provisional review
presentation from the approved answer rather than treating drafts as final.

Compaction selection, summary acceptance, and the typed completion-review packet
are shared Core behavior for CLI, macOS, iOS, Android, and Web-backed sessions.
Core also constrains independent review's invocation plan: hidden reasoning and
executor continuation are disabled, with a recomputed semantic/cache identity.
Hosts consume that plan instead of maintaining a CLI-only audit exception.
The reviewer is not the conversation's generation model. Core binds every Host
response to its originating `ModelResponseMode` and discards reviewer provider
continuation, whether delivered as a standalone event or embedded in a Reasoning
item. Approval retains the candidate generation's continuation; review metadata
must never replace it, even when both requests use the same configured model ID.

Trace consumers must keep `compacting` active through nested model attempts,
return to `processing` on `context.compacted`, and clear transient phases on
cancellation/failure/completion. Model progress without tool identity is never a
tool card. Plan/validation bookkeeping cannot replace the active phase. Native
clients retain their existing input and scrolling surfaces; CLI keyboard hints
are not conversation content to replicate on touch/desktop clients.

### Mac folder consent and long-task presentation

The non-App-Sandbox Mac product separates explicit application workspace consent
from OS security-scoped bookmarks. An incoming `workspace_access` request only
opens the built-in permission card; the exact directory and reason are visible
without expanding raw arguments. Accepting records that directory for this
conversation and resolves the same Core request. It never opens `NSOpenPanel`,
changes cwd, approves writes/shell commands, or bypasses macOS privacy controls.
The working-directory chooser remains an explicit user action. Confirmed roots
survive restart and are removed by conversation workspace revocation.

Mac uses Core's default model-round budget (`0`, no arbitrary round ceiling).
Explicit caller budgets retain their exact value. Cancellation, permission
decisions, effect receipts and completion review still govern execution; opening
the app is not authority to install packages or modify global configuration.

Native Markdown handles HTML line breaks without executing HTML and renders
math with the existing SwiftMath dependency, offline. Code spans remain literal,
unsupported formulas retain their original source, and copy/full-text selection
continues to use canonical Markdown. Formula images use template foregrounds so
light/dark appearances do not inherit a cached color from another view.

### Execution outcome is independent of transport lifetime

Local child exit, pipe EOF, a cancellation exception, and a remote HTTP
cancellation acknowledgement are transport facts, not terminal Run evidence.
Every client execution location must reconcile the exact conversation, Run and
assistant identity against the committed terminal record before completing the
message or notifying completion. A late transport failure cannot override a
committed completed/failed/cancelled outcome. One finalization owner drains the
presentation and records completion once.
Activity and notification inputs retain the typed terminal outcome; a Boolean
success flag must not turn a committed cancellation into a failed activity.

Without that evidence, preserve the unresolved Run identity and cancellation
intent. Stop transport activity indicators, retain the draft and message body,
and prevent a new submission from overwriting the unresolved execution. A status
check is read-only; it must not replay the prompt or issue a remote resume for a
local Run. Absence from one read does not prove pre-dispatch rejection. Runtime
recovery must use the original frozen Run and verified delivery ownership through
Application, not manufacture terminal state in the Presenter.

`inspect_run_recovery` is a read-only, same-snapshot observation bound to exact
conversation/Run/assistant identities. Its missing, identity-mismatch,
inconsistent, terminal and active variants must remain distinct at the client.
Active reports aggregate persisted evidence only: checkpoint/lease existence
does not establish checkpoint validity or process liveness. No lease token,
credential, owner identity or effect payload belongs in the presentation result.
The client retains the canonical assistant ID captured at submission, rather
than attempting to reverse a display UUID.

An observed-orphan recovery command must carry the expected Run ID and recheck
it inside the mutation transaction alongside observed delivery identities. An
earlier active-Run lookup is insufficient: a stale request cannot first recover
another Run's deliveries and only afterward discover its identity mismatch.

### CLI transcript lifecycle

The transcript owns assistant text and tool receipts. Live activity stays at its
transcript-tail anchor; it is never staged in the composer and moved into history.
Its clock measures elapsed wall time in the current stage, not estimated task
completion. Repeated stage notifications and tool detail updates preserve that clock.

The terminal has separate body-segment and turn-completion boundaries. A reviewed
candidate remains a review-status update until approval; only then is its body
appended as a durable segment. Approval does not finish the turn or release queued
user input. Review labels are not abbreviated as short tool-status titles: their
full meaning must remain visible.

Compaction lifecycle events remain visible even in quiet mode. A model attempt
inside compaction must not replace the compaction stage with generic thinking;
completion or failure releases that stage. Plan/validation snapshots must not be
mistaken for live execution. JSON continues to carry canonical traces only.

The composer surface owns input instructions and a separate queued-input count,
never appended to the editable draft. Short viewports prioritize draft and input
guidance over session metadata. Permission challenges retain their exclusive surface.
This borrows elapsed-time/action guidance from the local Codex status indicator and
state-driven hints from OpenCode's prompt, without adopting their bottom activity layout.

The canonical Store is the only durable source of complete conversation history. A
conversation is represented by ordered `Message` records and their ordered `Part`
records. Clients and transport adapters must use the Store message identifier when
addressing a conversation boundary.

An Agent Core checkpoint is recovery state, not a transcript database. A terminal
checkpoint may intentionally contain no transcript, model context, or item ownership.
Code must therefore never recover, branch, revert, export, or render a conversation by
treating a checkpoint as the complete message history.

## Application boundary

Conversation mutations cross one typed Application boundary. The Application layer:

- loads the durable conversation and message parts from Store;
- reconstructs temporary typed Core items and ownership from canonical message IDs;
- asks Core to evaluate the branch or revert transition;
- acknowledges internal checkpoint effects generated while rebuilding the temporary
  Core projection;
- commits the resulting conversation, messages, parts, and checkpoint through one
  Store transaction.

Web, CLI, macOS, iOS, and Android adapters must not independently mutate checkpoints
or compensate for partially committed conversation writes. They translate platform
input into an Application command and project the returned result only.

## Agent turn ownership

For an Agent conversation, the Host is the sole transcript writer. A client submits a
canonical submission identity, User message identity, and Assistant message identity;
it must not pre-insert either message. The Application `BeginTurn` transaction creates
the User message, pending Assistant message, Run, Parts, and checkpoint together. A
terminal event completes that same Assistant message and never inserts a replacement.

Native CLI execution is itself a Host. It uses the canonical User `ItemId` already
present in its Core input and the Core Turn identifier for the Run. Network clients
must provide all Host identities explicitly; missing or partial identity is a protocol
error rather than a request for a legacy fallback.

Agent Trace is an ordered diagnostic journal, not a transcript writer or UI content projection.
For a canonical Turn:

- a diagnostic Trace identifier is not authority to infer a canonical Run;
  recovery supplies the expected canonical Run explicitly and validates the
  returned submission identity (native execution may use equal identifiers);
- Trace `messageId` equals the pending durable Assistant message identifier;
- transport emitters forward Trace events but do not persist a duplicate journal;
- recovery restores the existing Assistant bubble from canonical Message Parts and
  may attach live progress only after validating the Run identity;
- message display order comes only from the Store ordinal, not timestamps or IDs.

Browser recovery and terminal fallbacks are read-only projections when `hostOwned` is
true. They may repair the visible state after a disconnect, but they cannot append User
or Assistant records.

### Remote committed content and presentation ownership

A receiving client mirrors a remote Turn through typed Application transactions,
not by persisting provider or Trace deltas. `open_remote_conversation_turn` creates
the receiving Run and its exact message owners and binds the selected authenticated
`source_host_id` to that Run before transport dispatch. This mirror admission does
not create a second executing Turn. Both running and terminal commits must match
the bound Host, conversation, submission/Run, and assistant identity. A new Run may
select a different Host; a payload cannot change an existing Run's binding.

`remote_assistant_projection_committed_v1` carries one already-committed assistant
read model. Its source conversation revision and committed Run cursor come from one
source read snapshot. The target atomically replaces only the owned assistant Parts,
advances its own canonical revision and replay watermark, and publishes projection
and synchronization changes. Exact source replay is a no-op, conflicting reuse is
an error, and stale content cannot overwrite current or terminal content. Local
delivery time is not source content identity. Running content never authorizes a
tool, resolves a permission challenge, or completes a Run.

Target full/window/page queries project the current local revision and Run state,
including before the first terminal snapshot and during a later Run. The original
Host revision stays in immutable source fields; it is not the target read-model
revision. Recovery requests carry `expected_run_id`, not a guessed Trace Run ID.

Presentation owns accepted text, visible text, parser state, and undisplayed segments
together. Detachment pauses display, not execution; a replacement invalidates stale
reveal ownership. An unchanged accepted snapshot can still have pending display
work. Optional projection reads cannot turn an already-committed execution response
into a failed transaction, and consumers without a live projection subscription do
not request these snapshots.

The current bounded cumulative-snapshot transport is not a claim of linear streaming
cost. Committed typed deltas, bounded large-body recovery, and terminal delivery
independent of whole-body loading remain tracked work in
`../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md`; raw storage deltas must not bypass review visibility.

Iteration limits are execution policy, not transcript compatibility. Product requests
default to no global round limit. Tests, bounded automations, and callers with a known
budget may provide a positive `max_iterations`; the Host must pass that value unchanged
to Core.

## Transaction guarantees

Creating a conversation branch uses one `BEGIN IMMEDIATE` SQLite transaction. The
transaction inserts the target conversation and settings, copies the selected messages
and parts, writes the Core checkpoint, and advances the conversation revision. Any SQL
or validation failure rolls back all of these writes.

Retries are idempotent for the same target identity and committed state. A target that
already exists with different state is a conflict; it is never overwritten as a retry.
The source conversation remains immutable. Revert visibility is a projection over the
source history rather than destructive deletion.

## Required verification

Changes to this boundary require all of the following:

- Store tests against a real SQLite database, including mid-transaction failure and
  reopen verification;
- Application tests that start from durable messages and an empty terminal checkpoint;
- restart/idempotency verification;
- a real token-authenticated Web-to-Host-to-Core-to-LLM session that branches a
  multi-turn conversation and reads the result back through the product API;
- the complete Rust workspace test suite.

The product E2E must assert message identity, order, roles, and distinct branch
identity. It must also assert one Trace per Run, Trace-to-Assistant identity, progressive
Markdown projection, permission suspend/resume, browser reload recovery, and durable
SQLite state. A successful HTTP response alone is insufficient.
# Composer reasoning contract

The macOS composer no longer projects context accounting or response verbosity
as a thinking control. Context accounting remains available to the runtime.
Its input surface is fully opaque. Permission mode and thinking are always
discoverable in a separate compact footer below the input, including when
thinking capability is unconfigured (explicit status plus a Model Center
configuration action, never fabricated supported values). The inspector overview
prioritizes current model and workspace actions; context metrics, data sources,
response preferences and working context remain expanded, without disclosure groups.
Session sandboxes show a friendly name, not the full internal filesystem path.
Local PDF references use PDFKit, other document types use native Quick Look,
and source references retain the line-aware text inspector. Preview lifetime owns
security-scoped access; binary documents never enter the source-code decoder.

Wire protocol is an endpoint property independent of thinking capability.
Sparse conversation/model overrides take precedence over model defaults, including
explicit zero values. The effective parameters and protocol are frozen at Send
and transported to the local runtime; selecting a model in a conversation must
not mutate the global default. Unsupported protocol parameters fail explicitly.
The empty-input hint is drawn by the native editor with its own TextKit
container geometry; it is not a second SwiftUI text layout over the caret.

Thinking capability is explicitly configured per endpoint/model in Model Center.
Unknown capability is not evidence that a provider supports Responses or a
particular effort. Responses routes display only their declared effort values.
Chat Completions maps the three product presets to declared low/medium/high
effort; Anthropic maps them to three explicitly configured, increasing budgets.
Invalid or unsupported configurations are rejected, not silently clamped.
Output-token limits use the shared model-parameter bounds; the Anthropic
minimum thinking budget must not constrain Responses effort configuration.
Conversation numeric editors retain raw input until it parses and satisfies
the field bounds. Invalid or incomplete input disables Save rather than
silently committing the previous numeric value. Disabling an override or
restoring model defaults clears that field's raw draft and validation state.
An unset thinking preference inherits the model's declared default; explicit
Automatic omits the wire-level reasoning selection, even when a model default
exists. A persisted selection that is no longer supported remains visible and
blocks submission until corrected; it must not silently become Automatic.
Validation uses the effective conversation selection, so a valid explicit
override is not rejected because a shadowed model default became invalid.
The conversation editor exposes inheritance separately from explicit Automatic
and resets its draft identity when switching model routes. Thinking selection never rewrites the prompt
or changes the persisted response-style preference.

The preference and capability snapshot are captured when Send is pressed.
Selection during a run applies to the next message. Thinking preferences belong
to the same conversation/model override record as generation parameters.
Switching models restores each route's own selection without carrying another
route's selection or erasing the previous route. A declared default must belong
to the effective protocol's supported options (or Automatic). Explicit thinking routes
do not silently fall back to models whose reasoning contract is unknown.
Host request validation and the frozen invocation identity include the typed
reasoning selection; adapters validate protocol compatibility and output limits.
Effective parameters appear in model-response provenance. Review and compaction
requests clear user thinking overrides independently.

The explicit Responses adapter uses streaming and `store:false` for stateless
continuation. Tool calls are not executable until the terminal response agrees
with the streamed transaction. Complete provider output items remain in the
continuation checkpoint; the next request projects a single supported reasoning
representation (plaintext when the item contains reasoning text, encrypted state
otherwise), without mixing mutually incompatible fields. Continuation remains
bound to endpoint/model/credential identity; reasoning replay state is cleared
on compaction. Neither
reasoning state nor provider-state objects are rendered as user-facing text.
For OpenRouter router aliases, the first generation response's concrete model ID
becomes a route lock in the checkpointed conversation continuation. When no opaque
reasoning is attached, that route lock survives compaction and later generation
requests use that concrete model; a provider response that reports a different
model fails explicitly. Independent review may run on a different concrete model,
but cannot replace the generation route lock or provider continuation. The lock
is validated against the frozen endpoint, alias, and credential identity and is
not inferred from prompt words or a mutable global selection.
Provider failures cross Host and Application as a bounded code, safe message,
retryability, and optional HTTP status; the canonical Model Attempt retains
those fields. Raw response bodies remain local diagnostics, never persisted as
conversation failure text. A status-less stream/protocol error therefore has a
different code from an HTTP 400 rejection.

### Shared conversation prompt tone

`settings_json.promptResponseStyle` is a shared optional string with values
`professional`, `casual`, and `playful`. Missing/null means inherit the selected
model's default `ModelParameters.responseStyle`; a concrete value overrides that
default for this conversation, including after changing models or reopening.
The effective value is frozen when the user submits a message.

This field is not Mac's `responseStyleRawValue` (the Mac composer’s three-level
response-depth choice), nor a reasoning effort/budget. Prompt *tone* must never
be translated into provider thinking parameters. On Mac, response depth is a
separate user intent: its three levels can map internally to declared Responses
efforts or ordered thinking budgets. Unsupported values are never sent; when no
reasoning capability is declared, depth affects presentation only. The same
turn-frozen choice also supplies a user-instruction-scoped answer-depth hint;
it does not rewrite the user's prompt or the shared prompt-tone setting. The composer
must not expose transport parameters or create a second thinking picker. A
conversation picker must not write the
global model-selection or model-parameter store. The settings UI distinguishes
inheritance from an explicit choice and waits for a persistence receipt before
presenting success.

Clients editing their own metadata preserve unknown settings fields; clearing an
owned optional value must still remove/reset that value. Invalid non-object
settings are errors, not permission to replace the document with an empty object.
The current whole-object settings update is not compare-and-swap: preservation
of fields in a read/merge/write operation does not prove concurrent cross-client
write isolation. A revisioned field-patch command remains required for that case.

### Atomic endpoint configuration

An existing remote endpoint edit is a typed Application command with the exact
active member-ID/revision map observed by the editor. Store checks both membership
and every revision within the write savepoint; a concurrent added model, changed
model or deleted model invalidates the entire draft. Endpoints are not updated by
replacing the entire provider's catalog or looping over independent Web writes.

Name, base URL, wire protocol and enabled state update all endpoint members in one
transaction, including sync publication. Model parameters, selection, priorities,
capabilities, other endpoints and credentials remain unchanged. Incompatible
reasoning configuration on any member rejects the whole command. The Web editor
uses the complete returned profile snapshot because a group write advances all
affected revisions. Creating/deleting endpoints and credential management are
separate operations, not implied by this edit command.

### Host-local endpoint credentials

Endpoint credentials are separate from synchronizable model profiles, Application
commands and receipts. The Web credential editor reads public status plus the
complete endpoint-member revisions and a private-vault revision. Replacement and
clearing are explicit operations; an empty password is not an implicit clear.
The host binds credentials to endpoint ID, normalized URL and wire protocol.
Changing either route identity component must not carry an old key to that route.

Clearing retains an explicit tombstone so old singleton/default credentials cannot
silently reappear through compatibility fallback. Public `hasCredential` describes
only the endpoint-specific vault, not those legacy candidates. The editor clears
its password input after submission and never renders arbitrary server error
bodies or automatically repeats an uncertain write. Model-directory concurrency
and vault concurrency are separate: a private-vault CAS must not be described as
a single transaction spanning the canonical model database.

The new vault uses a separate SQLite database protected by Unix directory/file
permissions (0700/0600); it is not encrypted storage. Non-Unix vault management
currently fails closed. These guarantees apply to new endpoint-specific secrets:
legacy `web.agent.settings` credentials have not been migrated or erased by this
change. Existing unmanaged endpoints can still use their exact-URL legacy
candidate until explicitly managed/cleared. Static file serving must reject the
private directory and resolve symlinks before enforcing web-root containment.

### Android turn-owned model configuration

An interactive send captures the selected conversation model, effective parameters
and an independent client before context preparation. Cloud clients freeze the
endpoint/credential identity too; subsequent model/tool requests in that turn do
not re-read a mutable global ModelService. Local execution checks that its captured
loaded engine and parameters remain available, failing explicitly if replaced
rather than silently running another model. Conversation selection is persisted
with new-conversation creation; restoring a conversation is not a global-default
change. This does not yet provide the complete Responses/thinking configuration
or multi-endpoint persistence on Android.

### Automation reconciliation across clients

Automation definitions share one Application store. A client's editable list is
not necessarily the whole store: the existing native agent schedule and Web
source/script schedule have different versioned metadata contracts. Reconcile
must therefore carry an explicit, nonempty `schedule_formats` scope. Definitions
outside it are rejected; identities already belonging to another format cannot
be taken over, even through an otherwise valid scoped input. Missing entries are
tombstoned only within the declared scope, in the same atomic transaction as
upserts and sync-outbox changes. Results include only that scope.

Native and Web readers select metadata decoders by the persisted format before
decoding, rather than attempting to decode every definition as their local type.
The current native and Web execution/editing contracts are still different: this
is a data-integrity prerequisite, not completion of cross-platform automation
parity. Unifying their semantic definitions, scheduler execution and editor UI,
and adding conflict protection for simultaneous same-format snapshot edits,
remain required work; do not describe filtered client lists as full parity.

### Cancellation after a lost local transport (target contract, implementation pending)

Stopping a transport attempt is not evidence that its durable Run was cancelled.
In particular, cancellation during asynchronous workspace revalidation can prevent
the replacement process from starting while the original Run remains active.
This state needs a cancellation-delivery path, not a new submission or ordinary
resume. Clearing the UI cancellation flag cannot resolve it.

Application must accept an idempotent cancellation intent bound to the exact
conversation, Run and canonical assistant. It reads the original configuration
and reduces Core `Interrupt` inside the receipt unit of work. The client supplies
neither a fabricated terminal status nor a replacement model configuration.
Already-terminal evidence wins a late cancellation. A committed cancellation
intent may still be pending; the receipt must not imply confirmed termination.

Cancellation recovery dispatches only cancellation effects. It must never drain
ordinary pending model/tool effects first, perform MCP discovery, or require a
new provider credential or a fresh workspace grant merely to request stopping.
Revoked permission still prevents ordinary execution. A missing target in a new
process's in-memory registry is not proof that its old owner stopped: use exact
owner liveness evidence and transactional lease fencing, contact a live owner,
or retain an unresolved result. Unknown and remote owners cannot be declared dead.

Clients preserve the original identity and cancellation intent across retries.
They expose retrying cancellation delivery separately from resuming execution,
and close the UI only from exact canonical terminal evidence. Acceptance requires
the true pre-launch revalidation race, zero ordinary dispatch during cancellation,
revoked-access cancellation, repeated receipt identity, stale-owner rejection,
unknown-owner preservation and completion-versus-cancellation races.

Cancellation delivery admission must require a repository-confirmed write unit
of work before reading ownership facts or releasing a lease. A caller using the
direct Application helper must not accidentally bypass the received-command
transaction and partially commit a five-field lease deletion. Unsupported direct
calls fail before all writes; merely documenting that callers should use receipts
is insufficient. The exact lease release, cancellation-attempt reclamation,
restricted claims and command receipt roll back together.

Bound cancellation inspection by the currently pending cancellation targets, not
the total historical attempts in a long Run. Read the latest attempt for each
exact target through indexed storage queries. A history-length ceiling must not
turn otherwise provable cancellation into a permanently blocked operation.

## Session presentation and cache accounting (2026-10-09)

Mac journal metadata and its snapshot cursor are one immutable presentation
value. A body range is authorized against that cursor, never the newest cursor
from another publication. Read failures belong to an exact request; a newer
snapshot may retry automatically, while a superseded failure cannot poison the
pending read. Rust snapshot reachability validation remains mandatory.

Composer execution state and Stop target belong to the selected conversation.
Workspace execution capacity is separate: the current single execution slot may
block sending in another conversation, but must not present it as running or
discard its draft. Switching conversations does not clear another conversation's
pending authorization.

`ContextUsageView.conversation_cache_usage` is a rebuildable projection over
persisted request facts, independent of `cache_usage` (latest physical request)
and context occupancy. Group committed model-attempt events by Run and attempt
identity and use the latest version once; model-backed compaction already has a
ModelAttempt and must not be counted again from context notifications.
Compute sum(cache reads) / sum(input tokens), not
an average of percentages. Track reported/unreported request counts separately;
the UI must label partial coverage rather than treating missing requests as
zero-input cache misses. Invalid counters are unreported; overflowing totals
cannot produce a valid hit rate. Request facts survive process restart,
message-window pagination and model-context compaction; clients neither increment
their own counters nor use the currently loaded message window as the ledger.

Verification: 73 selected Mac XCTest cases passed in the isolated `Test`
configuration (journal transport/body reads, composer recovery, submission
preparation, remote-permission consumption, context snapshots). Rust durable
cache-ledger regression and two historical projection-port tests passed. No
user database or signed Debug application was overwritten by validation.

## Responses incomplete-stream recovery (2026-10-09)

Responses consumes complete SSE frames, including multiline data fields and
comments. Only a validated `response.completed` envelope commits a completed
response. EOF or `[DONE]` is not proof of completion. Partial text/reasoning on
transport interruption enters Core's bounded semantic continuation with
`stream_incomplete`; incomplete opaque response items and response IDs are
discarded. A terminal `response.incomplete` with `max_output_tokens` preserves
reported usage and enters the existing output-limit continuation path.

Pending function calls never become executable through either recovery path.
Content filtering, unknown incomplete reasons, malformed events and explicit
provider failure remain failures with bounded, distinct diagnostics rather than
copied provider payloads. Core remains responsible for continuation limits,
no-progress detection and canonical-message assembly.

The blocking HTTP client's timeout bounds header wait and individual body reads,
not the lifetime of an actively streaming response. Connection establishment now
has an explicit 30-second bound. The historical roughly 180-second failure is
not enough evidence to identify its upstream cause; its terminal SSE was not
recorded. Local real-HTTP fixtures cover completed/incomplete envelopes, partial
tools, EOF, multiline frames and active/stalled connections. Authenticated
replay against the original LAN model remains outstanding (unauthenticated
model discovery returns HTTP 401).

Verification: 40 selected Rust unit/real-HTTP contract tests and four PTY
integration regressions passed. The PTY cases exercise shared continuation,
single canonical answer, reasoning-only interruption and early Markdown
visibility; they do not constitute an authenticated LAN Responses model test.

Mac failure presentation resolves known committed failure codes to localized
explanations. It never infers failure categories from provider prose. Original
journal bodies and codes remain available in diagnostic details; unknown codes
keep their original body visible. This presentation mapping does not authorize
retry, change run settlement, or resubmit tools. Both model-attempt and terminal
failure cards use the same mapping.

Verification: 38 Mac journal transport/presentation tests passed in `Test`,
including compiled Simplified Chinese resources for 12 failure codes and
unknown-code/provider-prose fallback. This is not a manual visual acceptance
of the running Debug application.

## Streaming viewport ownership (2026-10-10)

Journal Markdown renders its existing semantic projection directly; hydration
must not feed a second whole-document parser on every publication. Stable node
identity and previously revealed nodes belong to the journal owner, not to a
temporary row mount. Selection changes fence asynchronous loading and rendered
projections by conversation identity; the running conversation's observer is
independent of which conversation is selected.

The native transcript controller coalesces geometry changes once per display
frame rather than debouncing indefinitely. Its own clip-origin changes are not
new content changes. User scrolling invalidates queued follow work; viewport
size changes and the measured latest anchor remain inputs to reconciliation.
These policies require hosted-window streaming/scroll tests and real-provider
conversation-switch acceptance in addition to reducer tests. Real tests use an
isolated application support directory and the Xcode `Test` configuration.

The same change set closes every still-open reasoning Part at the authoritative
`ModelCompleted` boundary, including provisional process preambles. It does not
infer completion by matching display text or wait for the whole turn to finish.
Tool start/end timestamps use the admitted observation clock and survive replay.

Compaction reports each physical request's usage durably before validating its
summary. Failed and superseded attempts remain billable ledger facts. A rejected
summary gets at most one shorter rewrite against the same archive and output cap;
the invalid summary is not appended to context and returned tool calls are never
executed. Existing user ledgers are not retroactively rewritten. This executor
change covers Rust-hosted Mac and CLI, not the separate native iOS executor.

Validation on 2026-10-10: 59 selected Mac tests passed, including real hosted
windows at 640/1440 widths in light/dark appearance, terminal reflow, selection
fences and observation reuse. 52 selected Rust/CLI tests passed, including actual
Bash pipeline receipts and local HTTP compaction fixtures. These fixtures are
not a substitute for a real remote-provider conversation-switch acceptance run.

Real-provider acceptance subsequently passed (1 test, 0 skipped, 20.6 seconds)
using the configured OpenRouter route and an isolated database. The production
Mac window switched to a new draft during the request, returned to the active
conversation, received incremental states, exposed the completed answer to AX,
and kept its measured footer in the viewport. Evidence is retained at
`/tmp/nola-mac-real-memory.2fFzqh/` (result bundle, screenshots, timeline and
redacted provider trace). Final and new-draft screenshots were visually inspected.
This does not establish a frame-rate benchmark or cover every provider's failure
mode, and it does not replace/restart the user's signed Debug application.

The run also corrected two acceptance infrastructure defects: the runner's
database name now matches `nola-v4.db`, and ephemeral test credentials share a
thread-safe vault scoped to normalized database identity (not store instance).
Production Keychain behavior is unchanged. SwiftUI AX nodes are traversed by
their Objective-C selectors rather than discarded for not declaring a protocol.
All 43 follow-up Mac tests passed (overlapping the first batch), including
cross-instance credentials, cross-database isolation, streaming AX and localized
model-attempt presentation. Failed-attempt cards retain failure facts; retry IDs
alone never imply successful recovery, and internal route IDs stay in details.

Follow-up from the 01:54 macOS recording: the transcript window now uses one
measured, keyed message container instead of moving its tail between eager and
lazy parents. Follow-intent changes do not animate the entire document. Plain
tool diagnostics show at most 24 lines / 4096 characters inline; their complete
content remains available through the journal reader, without a nested vertical
scroll region. These are layout/identity corrections, not a measured FPS claim.

The CLI composer clears authored input only after a durable or local queue
receipt. A full queue leaves the exact draft available for retry and displays a
localized notice. Idle/live paste normalization preserves newlines and tabs and
does not silently truncate content. Shift/Alt-Enter inserts a newline; repeated
Enter events do not submit twice. Shared editing operates on grapheme clusters
and supports logical-line navigation in multiline drafts.

Follow-up validation: all 4 `MacTranscriptCompletionScrollTests` passed,
including hosted production views and painted-glyph checks at 640/1440 widths
in light/dark appearance. The real-provider conversation-switch/stream/final
viewport test passed again (1 passed, 0 skipped) with isolated state; evidence:
`/tmp/nola-mac-real-memory.Klq23v/`. This does not measure frame time for the
user's original tool-heavy conversation, nor install a new signed Debug app.

## 提交、热状态与发布的精确顺序（2026-10-28源码核对）

依据 `core/nola-application/src/runtime.rs` 与 `assistant_publication_transaction.rs`。事务所有者必须是一个明确调用层；“所有调用都包一层transaction”会错误提交调用方尚未完成的工作。

| 入口状态 | 处理顺序 | 谁提交/回滚 |
| --- | --- | --- |
| publication调用已在UoW内 | 直接运行apply，不新建/结束外层事务 | 原调用方；内层返回Err不代替外层rollback |
| publication独立调用 | begin → apply → commit | 本层；apply失败rollback，commit失败尝试清理并返回需reopen/reconcile |
| Runtime hot-only命令 | execute → 成功commit_hot_command；失败恢复Runtime工作单元 | supervisor；不能据热delta宣称数据库已提交 |
| Runtime需durable flush | begin Store UoW → begin_durable_flush → execute → Store commit → supervisor adoption | Runtime；数据库提交成功后才接纳对应热状态 |

关键线性化点是Store提交成功，不是reduce完成、HTTP写出或ViewModel更新。若Store COMMIT返回失败，可能是ACK丢失：rollback只是清理当前连接，不能证明磁盘从未提交。Runtime poison并要求reopen；不得发布该overlay、继续复用热状态或自动重跑有副作用的请求。

若Store已提交但supervisor adoption失败，磁盘是恢复权威，仍poison并reopen；不能把数据库“回滚到内存”或伪造另一次成功。若执行本身失败且Store回滚成功，必须同步恢复热Runtime工作单元，避免后继命令继承未提交的审批、outbox或消息修订。

`with_publication_transaction`把canonical业务变化与公开投影提交在同一边界。已有外层UoW时，它既不commit也不rollback外层；调用方必须传播失败并决定整笔事务处理。bare Runtime明确拒绝接纳外部拥有的事务，因此两种入口不能随意混搭。

验收反例：内层失败外层仍可决定回滚；外层COMMIT失败后连接不可继续加入新命令；数据库无未提交公开事件；重开后以持久回执决定是否已执行。源码测试 `failed_outer_commit_does_not_leak_a_joinable_transaction`、`joined_publication_never_settles_its_callers_transaction` 是对应检查入口，本次没有重新运行。

本批窄入口 `prepare_mobile_task_guidance(input: {request, now_ms})` 只接受已 started 的准确 `send_agent_message` effect；从实际 parent Run/Turn、call 和 Task authority 校验归属。`Run:call_id` 是稳定 message_id，重放复用原 submitted_at 与回执；accepted 表示已排队（Deferred），不声称已被 worker 消费。child pump 仍经现有 Run/lease 核验消费，停止后新输入明确拒绝。该接口不新增业务状态机、持久表或外部订阅。
