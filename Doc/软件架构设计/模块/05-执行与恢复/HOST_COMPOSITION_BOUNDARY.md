> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# A05 Host composition: measured closure and first cut

## Streaming deadline ownership (2026-10-13)

Transport owns connect/header waits and no-progress deadlines, not a second
implicit total task budget. The Chat and Anthropic readers previously abandoned
tool arguments 45 seconds after the first tool fragment, even while bytes were
arriving. That cap is removed, including its environment override. Meaningful
content/argument progress renews the existing idle deadline; heartbeat-only
traffic does not. Output memory bounds, provider output limits, cancellation
and the admitted outer execution budget remain independent bounds.

Only a provider-completed tool transaction may leave the reader. Disconnects
without a completion discard partial calls and expose
`tool_transaction_incomplete`, never execution credit or success. Retry policy
still bounds resampling; the model must not receive a fabricated tool result.
Two real socket regressions cover Chat/Anthropic tool arguments arriving for
48 seconds, plus a completion regression verifies discarded partial arguments
and the safe public failure code. Execution results are recorded separately
from these source contracts.

Validation: `cargo test -p nola_cli --lib llm::streaming::` passed 92 tests,
including both 48-second socket cases, partial-tool discard/retry, visible
partial-answer non-retry and rejected-consumer delivery. Log:
`/tmp/nola-streaming-final-tests.log`. This is transport regression evidence,
not a passing Terminal-Bench replay of the new deadline policy.

The freight replay's approximately 45-second retry intervals are consistent
with this defect, but the archived failure projection was only `provider_failed`;
there is insufficient evidence to claim its exact cause was proven. General
typed transport/protocol failure coverage remains an open architecture gate.

## File media observations (2026-10-13)

Reference audit: pi `packages/coding-agent/src/core/tools/read.ts` returns image
content; OpenCode `packages/opencode/src/tool/read.ts` returns image/PDF
attachments; Codex `core/src/tools/handlers/view_image.rs` rejects non-visual
models. Nola already had a browser-artifact media bridge; the missing link was
ordinary authorized file reads, not absence of all media infrastructure.

`read_file` owns authorized source access and signature detection. PNG/JPEG/GIF/
WebP return media; PDF defaults to text, with explicit `visual=true` selecting
the original document. A bounded read (30 MiB safety limit, not a performance
SLO) produces a content-addressed snapshot under the session ToolOutputStore.
The existing HostArtifact event carries snapshot URI, MIME, original name and
read operation. Snapshot failure fails the observation before publication;
reading produces no mutation or validation credit. Core persists the artifact
reference through its existing result/checkpoint path, not base64 prompt text.

The provider adapter materializes a read snapshot, verifies its digest, then
appends media only after the entire adjacent native tool-result group. Source
file changes cannot change a retained observation. Missing/corrupt/oversized
snapshots and non-visual models yield explicit model-visible unavailability,
not silent omission or claimed visual inspection. Media contents are data,
not a new user instruction. Responses uses input_file; Chat uses file; Anthropic
uses document. Provider-specific support still needs actual server acceptance.

Initial LAN probe: `192.168.31.18:8081/v1` returned HTTP 400 saying the server was
started without the vision encoder and needs setup with vision enabled. Thus
that initial server configuration could not validate visual understanding. Client-side media
transport tests do not prove recognition, and no model capabilities or server
configuration were silently changed. Snapshot transport/replay fixtures and
protocol PDF mappings passed; process-crash durability and real visual-provider
acceptance remain separate gates. Cross-device transfer of local snapshot URIs
requires the existing resource-transfer boundary; a file URI alone is not bytes.

After the owner restarted the server, the same endpoint accepted image input
(HTTP 200). An isolated real CLI run on 2026-10-13, Run
`266f0aa0-0bf8-47a4-8e27-ac471e2436c6`, called `read_file` on a PNG, committed a
session snapshot artifact, and sealed Answered with a closed canonical answer
stream. The answer identified an engineering drawing, visible dimensions and
the correct gNucleus title/part name. The fixture's explicit vision capability
was isolated; global config was not changed, and its credential copy was removed.
Evidence is under the local temporary directory `nola-vision-acceptance-9j_oj3oq`
(`events.ndjson`, `evidence.json`, `session.db.outputs`). This proves a real PNG
read-to-answer path, not exact dimension accuracy, PDF acceptance, CAD delivery,
or passage of either visual benchmark task. One small dimension was read as 2
instead of 12, so recognition must not substitute for geometry verification.

This is a source audit and implementation record, not a compilation or runtime
acceptance result. A05's UI-free Host compilation boundary is **not complete**.

The process-liveness observation is now owned by `process_observation.rs`, not
the terminal emitter. Core recovery and cancellation no longer import presentation
to probe another PID. Unix probes use `kill(pid, 0)` directly, without a diagnostic
subprocess. Only ESRCH proves absence; permission or other observation errors keep
the owner potentially alive. Non-Unix lacks a reliable implementation here and
conservatively retains the owner instead of parsing localized `tasklist` output.
This observation grants no execution authority and does not prove PID start identity.
Source fixtures are written; platform runtime acceptance remains pending.

The input-intent journal audit additionally requires an existing visible intent
to synchronize its file and directory before recovery delivery: a prior rename
may have become visible without durable directory publication. Unix reads and
locks reject non-private ownership/mode or shared hard links; existing journal
directories are validated rather than silently chmodded. Invalid persistence
never becomes an empty queue or permission to send. The corresponding rejection
fixtures are source-only; filesystem crash behavior remains a final acceptance gate.

## Actual closure

Both executable entry functions call `CliApplication::run` in `nola_cli`.
`lib.rs` includes terminal, Web, Host, and the main-parts compilation unit.
Before this cut, `main_parts/bootstrap.rs` had 4,502 lines and imported
crossterm and terminal input at its top level; interactive.rs had 3,697 lines.
These counts describe the inspected snapshot, not architectural budgets.

Key concrete reverse edges are:

| Runtime consumer | Product dependency | Required next seam |
| --- | --- | --- |
| core_agent_runtime and Host driver | concrete `agent::Emitter` | Runtime event sink; CLI renderer owns terminal state |
| Emitter (4,130-line inspected file) | terminal_ui, interaction, root `interactive_input_bar` | Separate event/state projection from rendering and input |
| Host/provider/tool setup | concrete `config::Config` | Validated invocation configuration separate from args/prompts |
| Config (2,071-line inspected file) | terminal_input, terminal prompting | CLI parser/setup adapter producing validated settings |
| runtime_effect_supervisor and task execution | interaction::ExecutionCancellation | Execution control separate from terminal listener lifecycle |
| Host entry dispatch | Web `Server` and product session presentation | Explicit Host service commands; separate Web composition |

Moving or renaming the library would retain these edges. `cfg`-hiding UI while
keeping product-shaped contracts would not establish the required boundary.

## Implemented input-delivery seam

`cli/src/runtime_input.rs` is a UI-independent execution-input service with:

- Explicit `RuntimeInput::UserFollowUp` / `ClientControl`; no language or slash
  matching, no keys, drafts, terminal I/O, Core or database dependency.
- A bounded process-local FIFO and explicit `InputReceipt::Local/Durable`.
- An injected `DurableInputPort`; Application's existing agent-command FIFO
  remains the only durable fact. Producer admission does not acknowledge
  consumer processing or complete a Run.
- Failure propagation: a failed durable admission is not silently converted
  into a successful local queue entry.

`ExecutionCancellation::queue_runtime_input` now accepts the typed input and
returns its receipt. It never modifies the draft or synthesizes an Enter key.
The CLI adapter alone classifies slash commands and clears the unchanged draft
after confirmed admission. Keyboard and raw-byte Enter use that same adapter,
so raw input no longer drops drafts when the queue is full.

The execution owner supplies cancellation status. Inputs observed after
cancellation remain local for the outer interaction, never drained into the
cancelled active Run; deferred composer turns similarly do not drain active
boundary input. Children continue sharing the visible turn's local mailbox.
Application and Runtime Driver retain durable claim/ACK ownership unchanged.

## Remaining boundaries and caveats

The interactive producer now uses `boundary_input_intent.rs`, not the legacy
per-call UUID helper. A client-only request journal freezes database/source-entry/
session/request/submission identity, timestamp and exact payload before send.
It has bounded real bytes, private permissions, file/parent-directory sync and
a cross-process File lock. It stores no authorization or execution facts.
`EnqueueAgentCommand` and its existing received-command receipt remain canonical;
consumer ACK may remove the queue item, never the admission receipt.

NewSubmit and ResumePending are separate typed operations. Repeated text never
implies retry. Any unresolved intent blocks new submits; `/input-recovery`
inspects it and `/input-recovery resume <original-id>` reconciles the same owner
and payload, including after restart. Malformed files and unknown receipts are
errors/pending, not absence. The interactive outer loop also refuses to turn a
pending draft into an ordinary new Run. Source receipt verification precedes
any absent-receipt submission; all replays retain their original ID.

Canonical committed admission stays Durable even if local retirement fails.
Before cleanup a process-local poison entry retains the frozen envelope; an
unlink/directory-sync failure cannot reopen NewSubmit in that process. Recovery
of a surviving old file after restart reads its original canonical receipt.
This poison is client safety state, not an alternative business ledger.

Unix uses 0700/0600 and no-follow file opens; Windows applies an owner-only
protected DACL. Windows directory flush and both-platform filesystem behavior
still require final runtime acceptance. A persistence/security failure prevents
new dispatch. The legacy helper remains used by noninteractive source tests;
this extraction does not claim that every independent producer has migrated.

ExecutionCancellation now retains only the CLI adapter's terminal-listener and
composer state, wrapping `RuntimeExecutionControl`. Shared execution code does
not name this adapter. Host still compiles the CLI product through the remaining
composition and permission-presentation boundaries discussed below.

## Implemented execution-control ownership cut

`runtime_control.rs` owns execution identity/cancellation, explicit background
transfer capability, typed local input mailbox, immutable durable session owner
and input disposition. It has no terminal, keyboard, renderer, draft or permission
window dependency. Child controls derive cancellation locally and retain their
original shared input owner. Ancestor cancellation still reaches descendants;
child cancellation never cancels siblings or the parent.

Durable input calls the existing frozen client intent and Application receipt
flow; control adds no durable ledger and does not acknowledge consumer work.
Rebinding the same handle to another session fails. Cancellation and NextTurn
disposition prevent local text being drained into the current active boundary.
The CLI adapter shares its local mailbox with the control but exclusively owns
draft/cursor and keyboard listener state. Its explicit scope installs both the
CLI adapter and shared execution identity, and RAII drops the listener on errors.

Core/Host, ToolContext, external executor, MCP and task/effect supervisors now
use `RuntimeExecutionControl`. Native/Web callers construct that handle directly;
they do not construct terminal adapters. Core never starts a terminal listener.
Permission-authored notes cross `UserInstructionHandoff`: a CLI adapter restores
its draft, while a headless owner retains a typed next-input item. Neither path
automatically runs the note or turns a disconnect into a business outcome.

Source tests cover nested-scope restoration, isolated child cancellation,
shared mailbox ownership, immutable owner rejection, background transfer and
headless instruction retention. They have not been run.

## Implemented permission/question interaction port

`runtime_interaction.rs` owns narrow request/choice data and `RuntimeInteraction`.
Tools and task execution no longer import `interaction` for prompt types or
terminal functions. The shared permission fallback uses only the current
execution's explicitly installed port. `TerminalInteraction` is a CLI adapter;
native/Web resolvers retain their existing authenticated decision paths, and a
default headless control installs `UnavailableInteraction`, not the terminal.
Unavailable is `TransportInterrupted(Unavailable)`, never allow or deny.
Core/Application still evaluates and commits permission decisions; this port
does not write grants, change rules or mark an invocation successful.

Committed user questions receive an explicit interaction port in
`user_input_wait::present_pending`. Unavailable/cancelled presentation leaves
the durable wait unanswered and defers further presentation. Only an actual
validated answer enters the existing Application response transaction. A
permission's log/transport summary is plain request data, not ANSI framing or a
terminal-width probe.

Child controls inherit an explicit interaction capability, but spawning a
thread does not inherit ambient TLS authority. The worker must enter its owned
control scope. Execution identity, runtime control and CLI adapter TLS now use
insertion-keyed binding stacks: out-of-order guard destruction removes only
that binding and never restores a captured, already-expired owner. This fixes
the concrete non-LIFO counterexample where dropping A while B was active and
then dropping B previously resurrected A (including its interaction capability).

The same permission-chain audit covers Host permission resolver, Host session
owner/blocked status, approval-cache scope, SOP invocation, Device resolver and
its runtime identity, and Device tool catalog. All enter/drop bindings now use
the same exact-removal stack and thread-bound guards. Owner and blocked status
share one live binding; Device callback and identity likewise move together.
Resolver/catalog access checks the current Host permission owner, so switching
from A to B cannot borrow A's callback while B has none. Reentering the *current*
same owner preserves its identity and blocked state for Core's nested scope;
entering a later scope with the same string does not revive an expired binding.
SOP `None` remains an explicit masking frame, not a search for an older value.
Explicitly captured callback Arcs remain usable by their existing Host turn
contract; this cleanup does not pretend to revoke those callbacks or replace
Core's authorization, received-command, Run, or source-Host checks.

Pending custom permission text and pending trace collections are not enter/drop
authority stacks and retain their existing consumption/exact-trace contracts.
No general unrelated TLS refactor is included. Source counterexamples cover
cross-session A/B resolver pairing and mixed drop order, same-name new owner,
blocked-state isolation, approval-cache and SOP masking, Device identity cleanup,
and unbound worker threads. These are not executed acceptance results.

Source regressions cover unavailable permission/question results, no answer
commit when presentation is unavailable, explicit child capability versus
unbound worker, and non-LIFO cleanup across all three scope layers. No builds
or tests ran. This follows the lifecycle-boundary direction recorded from
Cursor's public harness articles in `HARNESS_CONTROL_SCHEDULING.md`; the local
`cursor.mdx` is a documentation-setup guide, not evidence of Cursor's runtime
implementation. The actual changes and failure cases above come from Nola code.

Next extraction order:

1. Split Config parsing and terminal setup from a validated Host invocation
   value. Inventory Config fields consumed by provider/tool/catalog/session
   modules; preserve typed source identity independently from process entry.
2. Define event/progress/control ports for core_agent_runtime and Host
   orchestration. Implement native JSON and CLI renderers separately. Move
   Emitter's session-lock/process helpers out of presentation before extraction.
3. Separate process entry dispatch: service/worker/Host commands cannot share
   terminal interactive functions or Web server composition. Then move the
   execution closure into a shared service crate consumed by CLI and Host.
4. Add dependency deny-edges and a Host-only compilation target only after the
   real implementation closure excludes terminal/Web; do not loosen gate rules.

Source regressions cover explicit slash-shaped follow-ups, untouched drafts,
durable failures, typed FIFO separation, cancellation, and full-queue draft
retention. None have been run in this implementation phase.

## Implemented output ownership and independent native port

`source_journal.rs` now names the UI-independent committed source reader.
`SessionStore` publishes canonical cursors and accepts an optional
`JournalPublicationObserver`; the observer reads using that store's original
ApplicationPort. In particular, an ephemeral/no-save Run is never reopened
through a persistent default database to render its committed parts.

Only the CLI adapter installs `terminal_journal_presentation.rs`. Its bounded
source reader and activity/text display state are no longer members of
CoreEventPresenter or SessionStore. Presentation updates drain before the next
visible event; they remain display state, not execution or completion facts.
JSON delivery does not install the terminal observer.

Goal declarations no longer travel back from Emitter to control execution.
The execution outcome reads an exact sealed Run through SessionStore and
extracts only a completed `goal_complete` tool's structured committed output,
in canonical part order. `CoreRunOutcome.goal_signal` is a **model claim**;
`AcceptCompletion` remains the authority to verify evidence and accept Goal
completion. Prose, another tool's output, an unsealed Run, and malformed data
cannot manufacture a valid declaration. Source regressions cover those cases.

Core runtime now accepts `RuntimeOutputPort`, never a concrete Emitter.
`NativeRuntimeOutput` implements it independently with no terminal renderer,
stdin reader or storage writes. It retains committed trace ordering, permission
challenge reservations, recovery isolation, terminal-frame deferral, and the
existing bounded ordered native writer. Background silent sinks, native desktop
cancel/resume/fresh requests, Web native/recovery transport and task execution
all use this implementation. CLI Emitter is an adapter, not the implementation
behind the native sink. Fresh native Host input no longer crosses CLI prompt
reading/slash-command interpretation.

The output port receives only `ExecutionContext` for cancellation on transport
failure. CLI Emitter optionally binds its same-identity terminal control in its
own adapter; native code cannot read drafts or attach a terminal listener.
Output failure requests only that bound token and reports a transport
diagnostic; it is not an Application rejection, does not delete a committed
seal and never authorizes replay/new admission. A client losing the reply must
reconcile the original Run/receipt through its existing recovery protocol.
Source regressions cover invisible-trace delivery, permission handoff without
duplicate traces, state-only usage, old-Run isolation, deferred terminals and
bound-token cancellation. Goal tests also reject empty/unknown declarations
and prevent a newer failed declaration from reviving an older successful claim.

This still does **not** establish UI-free Host compilation: shared runtime entry
and execution control still reference product composition/interaction, and the
crate includes CLI/Web modules. Those edges require the separate entry/control
and configuration cuts before a genuine shared execution crate can be extracted.
No compilation or tests ran for this cut.

## Implemented configuration capability and model invocation cut

`Config::from_args` is now explicitly non-interactive. Optional first-run setup
is a `ConfigurationSetup` capability supplied only by the CLI composition root;
`AgentHost` uses `NoConfigurationSetup`, and a Host stdin request never invokes
setup even if a caller supplies a terminal adapter. `cli_config_setup.rs` alone
owns terminal prompting, secure raw input and first-run configuration writes.
It preserves the existing private temporary-file credential write path. Explicit
CLI model/key options cannot be overwritten by the optional generated setup.
Existing project/global/environment merge precedence otherwise remains unchanged.

The pure path helpers moved to `config_paths.rs`: RuntimeConfig, instructions
and tool paths no longer import Config to resolve a home directory or repository
boundary. This removes the previous reverse edge back into the terminal parser.

`ModelInvocationSettings` is an owned immutable value built after the active
product/session/request overrides are composed. Its private fields include only
model routes, ephemeral credentials, output preference, reasoning, frozen output
admission and compaction options. It has no command, prompt, terminal, Web server,
environment/file loader or serde implementation. Debug is redacted. Construction
rejects missing endpoints with a typed error and validates HTTP/native routes and
executable fallbacks before metadata I/O. `model_route.rs` owns the shared route
validation without depending back on Config or admission.

`host_output_admission` now accepts only this value. Actual Host binding,
recovery, context/output budgets, initialization and provider fallback consumers
construct it at their composition edge. Existing route/credential hashes, native
logical identity, original frozen snapshot validation and reasoning/output rules
remain in force. Changing a later mutable Config or user prompt cannot mutate an
already constructed invocation value. This value is validated configuration,
not proof of a committed Run; Application still owns admission and persistence.

Source tests cover non-interactive defaults, explicit setup capability, Host
stdin setup rejection, model/key priority, typed missing endpoints, independent
Host construction, invalid fallback rejection before I/O, snapshot immutability
and secret-safe Debug. Existing recovery/native/provider fixtures consume the
new value. No builds or tests have run for this change.

### Remaining full Config split (not completed by this cut)

| Current Config contents / consumers | Required owner and next contract |
| --- | --- |
| command, show_help, host_request_stdin, resume_session, continue_last; bootstrap command/session selection | CLI/Host entry options; resolve selected session before constructing execution input |
| web_host, web_port, web_root | Web server launch settings; never part of Host invocation |
| message, output_path, force, dry_run, context_files; run/init/wiki adapters | Explicit command payload, artifact output policy and validated context attachments, not shared parser state |
| json_output/debug and concrete Emitter | Injected event sink/diagnostics, not transport selection inferred by Host |
| runtime generation options, selected model/provider and frozen retry limits | Extend model invocation settings to physical LlmConfig construction; current output-admission slice alone does not eliminate every Config reference |
| allowed_dirs/permission_scope_id/profile/tool_allowlist/skills/MCP catalogs; ToolContext and child tasks | Validated execution authority/catalog value with explicit parent delegation and refresh revision, not a clone of the CLI product Config |
| desktop resume/cancel, automation lease/spec, task execution, submission/message IDs | Typed Application-admitted invocation provenance; cancellation is a separate operation, not an empty run config |

The remaining execution consumers still depend on Config and Emitter. There is
no Host-only compilation target yet and no claim that A05's full dependency
closure is closed. Do not use aliases, deref-to-Config wrappers or cfg-hidden
terminal modules to count these remaining edges as removed.

## Tool execution policy versus live authority

The subsequent audit corrected the dependency inventory: `ToolContext` did not
contain `Config`; `CliAgentHostDriver.external_execution_config` did. Changing
the name of either type would not remove that reverse dependency.

`tools/execution_policy.rs` now owns an immutable `ToolExecutionPolicy` value:
command prefixes, auto-approval preference, network policy and permission rules.
Its owned private fields expose only borrowed/read-only access. Tool adapters no
longer independently mutate those four fields on a cloned ToolContext. Both
serial and parallel Core-admitted tool dispatch create an explicit
`without_auto_approval()` derivative, preserving all other restrictions and the
parent's snapshot. This value has no parser, terminal, model credentials or
configuration-file loading, and is not a durable authorization receipt.

Do not freeze dynamic authorization into that value. In the CLI Host,
`allowed_dirs` supplies working-directory/project context, not a filesystem
sandbox or proof of resource sensitivity. Native Hosts retain their actual OS
resource grants. `SessionPermissionScope` and the concrete Core effect decision
retain grant/revocation ownership. Ordinary external CLI paths do not need a
second directory approval; sensitive reads and operation risks follow the
[permission boundary contract](../07-权限与平台/PERMISSION_BOUNDARY_CONTRACT.md#filesystem-scope-and-model-requested-sensitive-reads). Discovery
and tool visibility do not authorize execution. The existing concrete request
evaluation, Core-approved invocation path, resource validation and permission
ports still run; snapshot construction does not replace any of them. This cut
does not claim to eliminate the existing terminal-coupled cancellation handle.

Source regression `owned_snapshot_and_effect_narrowing_do_not_mutate_parent`
checks owned input and independent derived policy. Existing concrete permission,
resource and invocation fixtures now construct the value explicitly. No tests
or builds have run for this cut.

### HostDriver Config consumers removed in the current source cut

| Actual consumer | Necessary data / next boundary |
| --- | --- |
| Initial and committed capability catalog | reasoning tier and whether the route is native; does not need CLI arguments |
| Provider message preparation | admitted attachments; separate from mutable composer input |
| Side-effect dispatch gate | admitted memory policy plus a live authority lookup; frozen policy alone must not authorize execution |
| MCP task lease | project root only; now receives `&Path` |
| Delegated worker lease and external executor | project root, backend/session/model/provider, external execution budgets, admitted permission/resource settings and delegated request; extract a dedicated invocation rather than cloning/reinterpreting product Config |
| Promoted successor Run | actual physical Host invocation binding compared with the newly committed Run; keep this identity check when extracting settings |

`CliAgentHostDriver` now holds `HostExecutionSettings`, not `Config` or a wrapper
containing it. Its inputs are the reasoning tier, native-route flag, admitted
memory policy and execution-only settings. `ExternalExecutionSettings` contains
the backend/session/model/provider selection, workspace, context components,
attachments, resource budget, resolved permission profile and input provenance.
Neither value contains terminal flags, startup commands, model credentials,
parser methods or configuration-file loading. Composition creates owned
snapshots; executor methods borrow them. Per-delegation context/model/session
derivation remains local to that execution and does not mutate the parent.

Permission default precedence is resolved once by composition; the external
adapter still restricts allow-all for non-live origins. Host memory authority
is re-read on each non-cancellation effect. Missing admission fails closed.
Cancellation remains available after revocation. Supervisor leases consume
only their execution inputs; MCP leases consume only the workspace path.

Successor verification is a composition-supplied `FnMut` port, called after
promotion and before dispatch. It reconstructs the physical invocation binding
and rechecks already-loaded automation context bytes, device resolver/egress evidence and
memory authority at the same point as before. It is deliberately not a cached
binding or a closure retained by the effect driver. Exact-Run/cancellation
execution cannot promote successors and does not require that capability.
`HostInvocationVerificationContext` owns the non-secret binding, frozen memory
ceiling, expected device-surface digest and loaded artifact byte witnesses.
Constructing it validates artifact source metadata and loaded-context membership;
verification rehashes the owned loaded bytes, not files on disk, and obtains
current device resolver/egress and memory facts. It cannot reread CLI arguments
or silently reload model configuration. Verification failures leave the newly admitted Run durable for exact recovery;
they do not dispatch it under an old identity.

Static/source acceptance added: owned snapshot and typed permission precedence,
missing memory admission without a CLI constructor, migrated live-revocation,
native-catalog, delegated process and MCP lease fixtures. No builds/tests were
run. Residual A05 dependencies remain in the composition layer (CoreAgentHost
execution Config, Supervisor's subagent configuration registry), and in lower
shared modules (memory admission composition helpers, RuntimeExternalExecution
still declared in runtime_config, cancellation's interaction module). These are
not claimed as a separately buildable Host-only crate closure.

### Supervisor invocation and exact binding lifetime

The subsequent A05 cut replaces the Supervisor registry, serialized worker
request, recovered child derivation and worker execution input with the shared
`HostInvocationRequest`. They do not accept or recreate CLI `Config`. Child
derivation clears parent occurrence/task authority and one-shot context/message
identities, narrows the workspace and declared tool catalog, and explicitly
disables reviewed-text delivery for its machine transport. Product adapters
perform the CLI-to-request conversion before invoking Supervisor APIs.

Registry ownership is now a unique process-local binding token, not a saved
previous configuration. Dropping a guard removes only its token, so overlapping
Hosts that exit out of order cannot resurrect a dead binding. An explicit owner
that is absent or blank never falls back to another session in the same project.
Selection requires exactly one live candidate, including when multiple bindings
share an owner; no-owner fallback counts all eligible live bindings rather than
choosing the newest or the first hash-map entry. Ambiguity/missing ownership does
not invoke reconciliation or start effects. Durable Task claim/admission remains
the execution authority after selection.

Source regressions cover missing and blank owners, same-workspace ambiguity,
same-owner overlapping bindings, cross-thread non-LIFO removal, immutable loaded
artifact bytes and changed device-surface verification. These are added source
cases, not executed acceptance results; unified testing remains pending.

The process-level Runtime Effect Supervisor likewise stores only composed
`HostInvocationRequest` values for cold recovery. Installing one and notifying
the supervisor use a single configuration-registry critical section. Notification
is only a synchronous supervisor-state update (no worker execution or join);
failure restores the previous entry before unlocking. Thus readers cannot
execute a provisional failed registration, and its rollback cannot overwrite a
newer concurrent request. A channel-controlled two-thread source regression
covers rejection overlapping a later successful registration.

Cursor's public [harness report](https://cursor.com/blog/continually-improving-agent-harness)
emphasizes measuring changes and separating expected tool failure causes;
its [dynamic context account](https://cursor.com/blog/dynamic-context-discovery)
describes on-demand discovery. The Nola boundary above is a design inference
from our source, not a claim about Cursor's closed-source permission types.
Final acceptance must retain permission-denied versus invalid-input versus
execution-failure distinctions, and measure latency/tool errors/cache behavior
on the same representative runs; fewer public fields alone is not evidence of
an improvement in runtime quality or performance.

## Composed execution request (current source cut)

Background transfer capability is separate from transfer intent. Only the
composition owner enables it; requesting transfer on a default/headless handle
does not enable it, and an already cancelled handle rejects new requests.
Foreground Shell observation gives observed cancellation priority over both a
user transfer and the automatic grace deadline. It rereads canonical task state
before producing a deferred receipt, and waits for an actual terminal record
after cancellation rather than synthesizing `cancelled`. A canonical existing
cancel request or successful submission suppresses repeat submissions. Storage
errors and unconfirmed rejection are surfaced as observation/submission errors,
not completion evidence; they retain the original `task_id` in structured task
data and explicitly carry `ExecutionState::Unknown`. They remain failed
observations, not `NotStarted`, `Completed`, or a fabricated `Deferred/running`
acknowledgment. The task reference is correlation, not a new Task promotion or
authorization to replay its command. They do not overwrite task state. This is an observed
cancellation ordering guarantee, not an atomic transfer/cancel transaction.
Source regressions cover capability versus intent, cancellation/transfer choice,
and once-only acknowledged cancellation submission; execution remains pending.

The Core runtime entrypoints and retained `CoreAgentHost.execution_config` now
consume `HostInvocationRequest`, not product `Config`. The request has explicit
model, tool, session/authority, input/context, execution and runtime-option
blocks. `HostRuntimeOptions` excludes config-file sources, duplicate route and
resource defaults, CLI debug/startup/Web options and parser methods. There is no
`Deref<Config>`, reverse conversion, or closure retaining a product Config in
these runtime entries. Supervisor child work uses the same request contract,
not a second product-shaped clone.

`Config::host_invocation_request()` is a fallible composition edge: after
product/session overrides it resolves `--continue` to a concrete conversation
and freezes the surface's default permission profile. Thus the Host cannot
silently choose a different latest conversation or consult command mode to
choose permission defaults. Conversion errors propagate without dispatch.
Reviewed-text support is an explicit delivery capability; cancellation arrives
through the execution-control argument, never through a JSON-output flag.
The generic no-control entry uses noninteractive control.

Owned request values are prospective input, not admission evidence. Existing
Application Run admission, immutable Host binding, exact recovery ownership,
loaded artifact validation and live memory/device authority checks remain
mandatory. Serialization serves only the private worker launch channel; it is
not a public settings or log DTO. Process-local authority and prior admission
receipts cannot be recreated by deserializing this input. Its Debug output is
fully redacted, including attachments/context/MCP registry details. The current
shape intentionally rejects unknown old product-envelope fields.

Smaller consumers now receive smaller inputs: instruction discovery takes only
instruction patterns, AgentRegistry takes only profile definitions, and active
SOP context takes project root plus conversation identity. Their implementations
do not import the full product/runtime configuration merely to read those keys.

Source tests cover owned composition, exclusion of product controls, redacted
Debug and cold worker loss of in-process authority; actual Core/provider/recovery
fixtures were migrated to the same entry contract. This is still source-only:
no build, test or measured baseline was run during this implementation phase.
The executable/library composition and remaining dependency closure require a
final audit; a new request type alone does not establish a Host-only crate.

### Shared SOP context query, not the CLI work presenter

`sop_context` owns Host SOP context construction and shared identity/version
helpers. Core and tool lineage checks no longer import `work`, which remains a
product `Config`-based work-list presenter. Existing owner/version/protocol
source cases exercise the shared helpers; this removes a production dependency
edge, not proof of an independently compilable Host target.

Model context uses `VisibleActiveSopRuns` through StorePort, SQLite Store,
Application and the Host store adapter. It selects the exact stored workspace
identity, the requested session or unowned workspace records, and nonterminal
status **before** applying a resource bound. No owner means only unowned runs.
Callers must supply the workspace identity used when creating the run; the query
does not canonicalize aliases or widen authorization. Execution-time lineage
and permission validation are still mandatory.

The query returns one complete SQLite statement snapshot, not pages assembled
across concurrent changes. It reads at most 257 matching records: up to 256 are
returned, while overflow is an explicit error propagated by context composition,
never an empty or silently truncated context. This is a resource safety bound,
not a latency or capacity SLA. Source tests cover more than 32 foreign and
terminal records, workspace-owned visibility, missing/blank owner identities,
exact root identity, overflow, and pinned Application projections. None has
been executed during the implementation-only phase.

### Native entry execution slice

`entry::run_agent_host` routes the native `run --host-request-stdin` transport
directly to `native_host_launch`, before `CliApplication`, product `Config`
construction, or CLI command dispatch. Native launch options compose the owned
`HostInvocationRequest`; `native_host_request` validates and applies stdin
submit/resume/cancel frames; `native_host_entry` owns native output, cancellation
signals, permission scope, exact recovery and supervisor lifetime. This path
never converts back to product Config. The CLI binary's explicit native adapter
also terminates at the same typed entry after its product argv conversion.

The current Mac protocol keeps its stdin frame and explicit launch identity,
route and directory arguments. The duplicate `--yes` and `--network-policy`
native argv options are removed and rejected: stdin `permission_profile` is
the sole user permission intent, bounded by live Host admission. There is no
compatibility parser for those removed native options. Native `--api-key` is rejected,
and the Mac sends its selected credential only in the private stdin frame.
CLI-only environment defaults no longer shape the native catalog or grants: explicit native launch
directories plus loaded workspace configuration and stdin permission intent
are the inputs. Model generation overrides have one shared implementation for
configuration composition and Host options, not a second drifting parser.
The stdin read itself is bounded to 4 MB plus one overflow byte. Cold cancellation
is detected before workspace configuration/model credentials are loaded, and
still addresses the exact durable conversation, Run and assistant identity.
Resume validates that identity before discovery and again before outbox recovery.

Server, sessions, session inspection/deletion, catalog, and worker modes cannot
be deleted as dead compatibility: `MacLocalAgentRuntime.swift` invokes
`--list-execution-backends`, `sessions --json` and `session delete`, and task and
application workers have independent process entry protocols. The terminal-free
composition below preserves these consumers rather than dropping their modes.
Source cases were migrated to the typed request and extended for the native
transport parser and cold cancellation; execution verification is still pending.

### Terminal-free Host production closure (source implementation)

The native executable no longer dispatches any mode through `CliApplication`.
`native_host_commands` owns the existing Server, session JSON and process-worker
protocols. Session JSON construction is shared with the CLI through
`session_projection_output`; process/lock observation is shared through
`session_process_observation`. Neither helper imports terminal presentation.
Tool permission prompts preserve the exact command and full directory; only
the interaction adapter wraps text. The Host no longer calls a terminal preview
formatter or reads terminal dimensions when constructing authorization content.

Cargo's default `terminal-ui` feature enables optional `crossterm`,
`unicode-width` and `unicode-segmentation` dependencies and the terminal/module
composition. `--no-default-features --features native-host` omits these production
modules and `main_parts`; the `nola` binary requires `terminal-ui`. Test builds
may include terminal fixtures through dev dependencies and `cfg(test)` without
making them part of the Host production closure. The Mac runtime build script
selects `nola_cli/native-host` explicitly. Its service binary remains included.

Final acceptance must execute, not infer success from this source graph:
`cargo check -p nola_cli --no-default-features --features native-host --bin nola-agent-host`,
then the ordinary CLI configuration and shared/native protocol tests. None of
these commands has been run in the implementation phase.

Remaining composition dependency: native Server/session commands still parse
the shared product `Config`, and Server still includes the Web module and
`tiny_http`. This is now outside the Core invocation graph, but it is not a
minimal native-only service composition. Replacing that shared command/settings
parser with typed control-plane inputs is the next cut; neither a feature flag
nor the removal of terminal code proves that remaining boundary is complete.

### Mac launch provenance: submission versus exact-Run recovery

The closed, required `mac_launch.input_origin` is frozen launch provenance,
not an authorization grant. Fresh stdin submissions must match it against
their actual `input_origin`. Resume continues to reject any new input-origin
field (both envelope and configuration); absence of input is not evidence that
the original Run was interactive. Its preserved descriptor instead passes the
existing exact original HostInvocationBinding comparison before execution.
Changing scheduled to interactive (or the reverse) in that descriptor fails
recovery without changing the checkpoint. Swift cold reconstruction preserves
the descriptor, not a new input channel. Source regressions cover actual native
decode/hydration and both origins through persisted Run cold recovery; final
unified execution of this batch remains pending.

Exact invocation reads of a finished Run use only the current `sealed` contract.
Store carries the validated canonical result seal under the same pinned read as
the invocation and checkpoint. Application checks the three owner identities,
result digest, completion timestamp, committed checkpoint decision and terminal
Turn status; a status label alone cannot authorize terminal recovery. A missing
active Turn still requires that Run's committed checkpoint and result evidence.
No legacy completed/failed/cancelled Run labels are accepted as seal substitutes.
Regression sources exercise actual answered/failed/cancelled sealing, cold reads,
foreign assistant rejection and removal of the immutable seal in an isolated DB.

## File mutation payload admission (2026-10-13 audit and partial implementation)

Post-v4 implementation: string content, pretty-serialized JSON including its
newline, and append payloads share MAX_WRITE_BYTES=500000 UTF-8 bytes. The
misleading maxLength is removed; Host checks final bytes before mutation.
write_file uses atomic replacement with mode preservation. Capacity/truncation
recovery no longer forces append. Tools 446/446 and Core Host contracts 145/145
passed; this is not LAN benchmark acceptance. Transactional chunk staging and
idempotent append are not implemented. The following retains the v4 findings.

The live harness-v4 freight trial rejected an otherwise complete `write_file`
call at 16,384 UTF-8 bytes, then the model recovered with four ordered appends.
This is an observed additional protocol cost, not evidence that the file is
correct or that the benchmark passed. That batch's binary remained frozen.

Conflicting contracts observed in v4:

- `checkpoints_review.rs` limits string content to 16 KiB, but serialized
  `json_content` bypasses that limit and uses `MAX_WRITE_BYTES` instead.
- `catalog.rs` expresses the byte limit using JSON Schema `maxLength`, whose
  unit is Unicode code points, not UTF-8 bytes.
- Core's schema validator enforces `minLength` but not `maxLength`, leaving
  the model-visible schema, Core admission and Host execution inconsistent.
- Stream transaction/memory admission is independent. A complete tool call
  is not incomplete merely because a separate Host payload policy rejects it.
- `recovery_tool_for_failure` maps oversized/truncated `write_file` failures
  to `append_file`, then restricts the next tool projection. This is not a
  semantics-preserving correction when the original intent was full replacement
  of an existing file. Current append has no chunk identity or expected offset;
  `create=true` does not mean create-only. Do not describe it as an idempotent
  large-file transaction or infer that replay after interruption is safe.

Target ownership and IO:

1. Host declares its supported mutation payload byte capacity once in the
   capability contract; Core freezes that declaration for the Turn. It is a
   resource bound, not a strategy rule forcing ordinary source files into chunks.
2. Core validates actual supported schema constraints using their defined units.
   A byte capacity must not masquerade as `maxLength`; ASCII, CJK and emoji must
   have the same documented byte-bound behavior across all clients.
3. Host serializes either text or structured JSON into the final bytes, applies
   one capacity check, and rejects before creating a file or changing its state.
   Permissions, expected-content guards, atomic replacement and cancellation
   remain mandatory. JSON cannot be a larger-payload bypass.
4. Rejection is a typed non-executed argument/capacity result, distinct from
   provider truncation and unknown side effect. The model may choose a legal
   alternative; no implicit append, blind retry or authorization expansion.
5. Recovery preserves create/replace/append intent. Never force a replacement
   into append or remove the observations needed for safe correction. Append
   remains an explicit incremental operation, not the obligatory default for
   a complete ordinary file. A future staging protocol must include immutable
   chunk IDs, expected offsets/content versions, integrity validation and one
   final atomic commit before it may advertise replay safety.

Acceptance must exercise exact-limit/over-limit UTF-8 and structured JSON,
zero side effects on rejection, advertised/frozen/runtime limits agreeing,
complete long tool streams, duplicate/cold-replayed calls and Unicode boundary
cases. Benchmark task names and artifact paths must not enter this policy.
Do not claim this contract implemented until those paths are changed and tested.
