> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Architecture V2: projection reducer adoption

## Status (2026-09-05)

This is an implementation-gap audit, not a completion report. The Architecture
V2 requirement that embedded, daemon, and remote clients share projection
semantics is not established by the current Kernel unit tests.

## Observed production boundaries

| Boundary | Observed implementation | Remaining gap |
| --- | --- | --- |
| Pure Kernel | `core/nola-kernel/src/projection_reducer.rs` | Workspace member with no production dependency found in inspected manifests or callers |
| Application | `core/nola-application` depends on `nola_core`, domain, protocol, and Store | Does not depend on `nola_kernel`; projection materialization and event publication are separate from the new reducer |
| Mobile FFI | `core/nola-mobile-bindings/src/lib.rs` exports Application commands and force-links existing `nola_core` symbols | No Kernel projection reduction export |
| iOS command boundary | `Nola/Services/IOSCanonicalAgentApplication.swift` issues typed canonical turn commands through IOSApplicationRuntime | This proves Application command adoption, not shared client projection reduction |
| Mac event presentation | `NolaMac/Services/MacConversationProjectionClient.swift` reduces typed Application conversation mutations natively | Independent Swift reducer; repository owns subscription snapshot/cursor pairing |
| Mac history windows | `MacConversationWindowModel` consumes bounded Application window queries | Full window replacement should not be converted into full-history replay just to use a reducer |
| Android | AgentTrace and AgentCorePart reducers are called by UI/workers | Trace/Part contracts are not automatically equivalent to canonical ConversationProjection events |
| CLI/Web | Rust AgentParts emitter exposes context and activity changes | Shared canonical Kernel consumption was not found by the bounded search; complete transport/presenter tracing remains required |

The Store already chooses local/remote context authority by timestamp, with
local winning ties. Tests now cover this. Do not add a second source selector
in a client based on branch order or measurement revision.

## Required integration shape

1. Keep canonical persistence and business mutations in Application. Projection
   rendering must not become another database writer or Host-owned transcript.
2. Define a pure, bounded projection-reduction adapter. It receives a validated
   canonical snapshot plus its Application cursor and a scoped event suffix.
   It returns the next projection and cursor together or rejects the batch
   without publishing a partial result.
3. Rust clients call the same Kernel through that adapter. Native clients use
   an explicit ABI that does not require a second service process. Do not send
   whole long transcripts over IPC for each token merely to centralize code.
4. Preserve state needed for attempt ownership and terminal-Part protection
   across adapter calls. Recreating a reducer from only visible text is not
   sufficient; `part_attempts` and event identity behavior need explicit
   restore semantics.
5. Keep window subscriptions bounded. A page replacement or a source change
   must install a snapshot/cursor pair; it must not splice the new page into a
   full-history shadow copy maintained by the UI.
6. Remove platform mutation logic only after production callers use the shared
   adapter and real transport tests demonstrate equivalent results. A Cargo
   dependency, exported symbol, or fixture test alone is not adoption.

## Contract issues to close before enabling the adapter

- Mac currently rejects duplicate sequences in a supplied batch, whereas
  Kernel suppresses repeated event IDs. Specify duplicate delivery at the
  subscription boundary and validate identity before accepting any no-op.
- Kernel's recent seen-event identities are now capped at 256 entries and
  validated after conversation scope. A 10,000-event regression passes. This
  bounds entry count, not arbitrary identity-string bytes; Part-attempt and
  completed-Part lifetime/restore bounds still require implementation.
- Measurement revision is provenance, not the Application cursor. Source
  changes may lower it. Snapshot responses still need scoped freshness so a
  delayed query cannot overwrite a newer event.
- Equal-revision sibling updates are legitimate within one transaction; event
  sequence orders them. Mac and Kernel now have targeted regressions, but they
  do not yet run against one shared transport fixture.
- The Mac workspace still prioritizes provider usage over estimated usage
  without a canonical query cursor. An authoritative newer estimate must not
  be discarded simply because its precision category differs.
- Unknown context, never-measured context, and query failure require explicit
  semantics. Current Mac availability uses positive window/prompt budgets;
  that UI safeguard is not a full cross-platform protocol design.

## Acceptance gates

1. Trace a production command and its committed event suffix through service,
   embedded FFI, and remote transport; record the actual reducer entry invoked.
2. Replay identical canonical fixtures through Rust and native ABI entry points:
   same-transaction siblings, duplicates, gaps, wrong scope, reused identities,
   source changes, zero usage, cancelled/failed Parts, and attempt mismatch.
3. Disconnect/reconnect during a run. Resume from the acknowledged cursor
   without resubmitting the user command or duplicating final text.
4. Verify snapshot replacement and event application atomically preserve their
   cursor. Failed batches leave the previously displayed projection unchanged.
5. Run bounded-window tests with multiple ten-million-character conversations,
   long event suffixes, and repeated switches. Measure both client and service
   RSS; include restore state and deduplication collections in the budget.
6. Capture native/Web screenshots during real incremental updates and after
   completion. Compare content, order, layout, tool details, and context state;
   a height assertion or parser test is not visual acceptance.
7. Run actual multi-turn model/tool sessions after fixture gates. Fixture Tool
   and Skill Parts do not prove those tools or Skills executed successfully.

## Existing evidence, with limits

- `/tmp/nola-kernel-event-order-tests.log`: six Kernel tests pass; no client
  adoption or transport is exercised.
- `/tmp/nola-mac-context-event-order-tests.log`: native Mac reducer and workspace
  tests pass; they do not call the Kernel.
- `/tmp/nola-context-authority-retest.log`: nine canonical Store/Application
  fixtures pass, including source competition and database reopen.
- `/tmp/nola-mac-unmeasured-context-tests.log`: 132 Mac tests pass; measurement
  availability is still a client derivation from existing fields.

These gaps remain part of the original Architecture V2 goal. None of these
narrow test results establishes completion of the architecture refactor.

## Follow-up: bounded duplicate identities

`/tmp/nola-kernel-bounded-dedupe-retest.log` records eight passing Kernel tests
with no compiler warnings. Recent duplicate IDs must retain the same scoped
identity, sequence, Run and attempt. Foreign scope and conflicting identity
return errors without advancing the cursor. Expired deliveries fail the cursor
check instead of being applied again. This does not validate changed mutation
payloads under an otherwise identical event identity; durable event immutability
remains an Application contract and needs adapter-level coverage.

## Follow-up: rejected text events and removed Parts

`/tmp/nola-kernel-part-lifecycle-tests.log` records eleven passing Kernel tests.
Text-target validation now precedes attempt binding, so an event targeting a
missing message cannot reserve a Part for the wrong attempt before failing.
Message removal rejects older revisions and releases that message's attempt and
completed-text bookkeeping. A 1,000-cycle create/append/complete/remove fixture
returns both Part collections to baseline after every cycle.

This does not yet define restoration of live attempt ownership, cleanup for
Parts replaced by an UpsertMessage, or atomic application of a multi-event
batch. Those are still adapter adoption requirements, not covered by these
single-event and removal-path tests. No production client was switched to Kernel
in this change.

## Follow-up: message replacement bookkeeping

`/tmp/nola-kernel-upsert-lifetime-tests.log` records twelve passing Kernel tests.
UpsertMessage now releases attempt/final-text entries for Parts removed by the
replacement and registers final Text Parts introduced by the new snapshot. A
1,000-replacement fixture verifies bounded tracking and rejects late deltas
against the newly installed final text. Retained Part IDs preserve existing
attempt guards.

Live attempt restoration, whole-batch atomicity, identity-byte budgets, and
production adapter adoption remain open. The passing lifecycle fixtures do not
establish those properties.

### Atomic batch reduction validation

`ConversationProjectionReducer::apply_batch` stages a batch and commits only after
all events succeed. Errors leave the projection, sequence cursor, duplicate-event
tracking, and part-attempt tracking unchanged. Empty batches are no-ops; batches
larger than 256 events are rejected before staging.

Validation: `cargo test -p nola_kernel` passed 15 tests, including batch rollback,
repeated delivery, and oversized-batch rejection. Evidence for this run is in
`/tmp/nola-kernel-atomic-batch-tests.log` (local, not a repository artifact).

This is a Kernel contract test, not a Mac UI or production integration test.
Staging clones the supplied projection, so callers must provide a bounded window
and enforce decoded-payload byte limits; the event-count limit alone does not
bound memory. Production adoption still requires explicit restoration of attempt
identity, snapshot/cursor consistency, transport adapters, and native UI coverage.

### Production adoption: Application projection diff

Application now depends on `nola_kernel`. Its production
`journal_conversation_projection_change` calls Kernel's
`conversation_projection_diff`; the previous Application-local diff and text
mutation implementation have been removed. Kernel owns pure mutation selection;
Application still owns event IDs, ordered sequence allocation, serialization, and
Store writes. This supersedes the earlier finding of no production Kernel caller
for the event-generation path only. Native event consumption is not yet unified.

The shared diff requires snapshots from the same conversation and history window;
it is not a general diff between unrelated paginated windows. Existing algorithms
and wire mutations are unchanged by this extraction.

Validation: Application projection-journal tests passed 7/7, including 10 million
character text growth emitting only the new suffix, canonical completion replacing
preview, structural changes using message upsert, and reopened Store summaries.
Kernel tests passed 16/16. These are not native UI or real-model E2E results.

Kernel also provides an opaque, move-based in-process checkpoint preserving event
cursor, recent duplicate identities, text completion state, and `(run, attempt)`
ownership. It avoids copying text when parked and resumed, but does not provide a
serialized ABI checkpoint or replace Application persistence.

### Adoption regression evidence and remaining gates

Application's entire library suite passed 52 tests after adopting Kernel diff;
Service's library suite passed 4 tests, including parent identity, disconnected
wait cancellation, queued wakeup delivery, and no periodic Store reads while idle.
Logs: `/tmp/nola-application-kernel-adoption-full-lib.log` and
`/tmp/nola-service-kernel-adoption-lib.log` (local evidence only).

The generator/reducer round-trip test found and fixed a message-envelope revision
loss: text deltas now advance the message revision, and the generator falls back
to authoritative upsert when independent message revisions cannot be represented
by a Part delta. Kernel's 18 tests and Application's 7 targeted journal tests passed.

Real temporary SQLite integration now covers three successive receipted commands,
reopening the database between commands, one-event pages, duplicate delivery, and
full projection equality. Fault injection covers a foreign-conversation payload,
a missing middle event, a missing tail, a cursor ahead of the journal, and a zero
page limit. The suffix reader rejects these instead of silently delivering another
conversation or returning an indefinitely empty `has_more` response. Normal caught-up
empty pages remain valid. Three journal integration tests plus the existing window
update integration test passed (`/tmp/nola-projection-no-progress-e2e.log`).

Still unproven: native Kernel consumer ABI and its actual Mac caller, equivalent
client adoption on iOS/Android/Web/CLI, whole-UI screenshots during timed streaming,
real-model multi-turn end-to-end behavior, serialized ownership restoration, and
bounded memory across the service and all presenter windows. These are required
follow-up work, not waived by the library regression results.

### Live snapshot ownership audit: required producer work

The native bindings now expose a bounded, stateful Kernel presenter handle and
require explicit `stream_owners` for non-final Text Parts when opening a snapshot.
FFI tests passed 2/2 and Kernel tests passed 18/18, but there is not yet a canonical
Application producer for this metadata. Do not enable the Mac consumer by guessing
owners or disabling `restore_owned` validation.

Observed source boundaries:

- `command_receipt::projection_stream_identity` constructs `RunAttemptId` from
  `ReduceAndAcceptHostResponse.input.response.effect_id`; other commands return no
  stream identity. This mapping needs an explicit execution-attempt contract before
  it is used as durable Part ownership. An effect is not proven equivalent to a
  run attempt merely because both are opaque identifiers.
- Store `messages` holds `run_id`; `message_parts` has no dedicated attempt owner.
- Core `PartOwnership` contains only `message_id` and `turn_id`. `TextPart` flattens
  this ownership into its persisted structured payload.
- Conversation materialization deliberately favors stored Part-to-message binding
  over the latest run ID for steered/multi-turn messages. Applying the latest effect
  or latest run to every restored Part would discard that distinction.

Required implementation order:

1. Define whether an attempt identifies one model invocation, one retry, or an
   entire run execution. Keep effect identity distinct unless the contract proves
   a one-to-one relationship for that specific operation.
2. Application must bind each mutable Part to its concrete run/attempt at creation
   and persist that binding with the canonical Part write. Later effects must not
   silently reassign an existing Part. No client-generated identities.
3. Read ownership for the selected projection window in the same snapshot/cursor
   boundary. Use indexed canonical ownership, not a scan of historical JSON events.
4. Include ownership in the native snapshot envelope, then adopt the Kernel handle
   in Mac. The in-process checkpoint preserves ownership across window parking;
   it does not solve cold restore from storage.
5. Validate real multi-effect turns, retries, steering, completion, restart during
   streaming, and delayed events from the previous attempt. Compare stored ownership,
   emitted identity, and accepted native events end to end.

This producer gap is incomplete architecture work, not an external blocker. Current
native handle tests use explicitly supplied fixture ownership and must not be
reported as proof of production live-snapshot restoration.

### Mac production-path audit correction

`MacChatWorkspaceView.observeConversationProjection` uses
`MacConversationWindowModel.refreshIfChanged`, which waits for a cursor change and
loads a revision-consistent bounded Application window. It does not call the native
Swift event reducer. The repository's `waitForConversationProjectionChange` was
referenced only by tests; that obsolete subscription entry has now been removed.
Tests now exercise independent live window cursors and coalescing 300 title changes
into the current authoritative window rather than replaying obsolete titles.

Do not add a second retained Kernel projection to this snapshot-based UI merely to
claim native adoption. Application already calls Kernel for mutation generation;
Mac should render the canonical window without independently reconstructing domain
state. The C ABI remains a separately tested option for clients that actually
consume event suffixes. Its presence does not justify duplicating Mac window state.

The earlier proposed blanket Mac handle takeover is superseded by this production
call-site evidence. Remaining work includes removing other unused Swift reduction
code, auditing full-projection caches used outside the window path, and exercising
timed streaming, switching, permissions, and real model tasks through the actual UI.

## 2026-09-05: Mac bounded windows and native text rendering

### Ownership and production path

- Mac's production transcript consumes bounded Application projection windows;
  it does not maintain a second full Kernel reducer snapshot per window.
- The repository retains only a small LRU of observed conversation sequences,
  not complete conversation projections. Window loading and message preparation
  run outside MainActor; publication checks the selection generation.
- Application windows carry optional live-text stream ownership. `nil` means
  unavailable ownership, not an empty ownership set. The native projection ABI
  consumes the Application window envelope directly and requires known owners.
- Store schema 11 persists compact request-to-text/reasoning-Part ownership
  independently of acknowledged outbox rows, which are pruned. This is not a
  migration: older databases remain incompatible and must not be silently deleted.

### Rendering evidence and implementation

The two-conversation benchmark uses 10 million characters per conversation,
12 switches, the real Application persistence path, and native hosting windows.
The resident Application window was 50 messages / 501,350 text bytes in the
reported fixture. These are synthetic conversations, not real-model sessions.

Initial switch P95 was approximately 229 ms. A diagnostic process sample found
approximately 84% of main-thread samples in CoreGraphics transparency-layer
compositing. Inline parsing caches, coalesced scroll callbacks, and drawing
whole static lines did not materially reduce that bottleneck.

The effective change is to remove the custom TextRenderer from settled text.
Native Text remains the layout owner even during animation. A non-interactive,
accessibility-hidden overlay draws the active reveal and is removed afterward.
This avoids retaining custom drawing surfaces for historical text and avoids
replacing the layout node at animation completion. Whole-message opacity and
offset entrance animation were removed; semantic phrase reveal remains.

- Latest unsampled switch P95: approximately 72 ms, including the benchmark's
  explicit 50 ms run-loop settling interval. This is not a first-paint metric.
- Measured switch RSS growth: 0 bytes in this run, not a general no-leak proof.
- The first static/custom branch implementation changed height by 13 pt at both
  360 and 720 widths. Active-animation sampling exposed this regression; the
  native-layout/overlay implementation passes the same assertions.
- Immutable inline parsing is cached with NSCache count/cost limits and a
  per-entry input-size cutoff. These are eviction policies, not hard RSS limits.
- Scroll geometry notifications are coalesced asynchronously, with generation
  checks and cancellation on detach, rather than publishing SwiftUI state from
  AppKit's synchronous layout notification stack.

### Test coverage and diagnostic operation

`MacVisualSnapshotTests.testStreamingCompletionPreservesNativeMarkdownHeight`
now samples layout at approximately 50, 250, 500 and 800 ms, rather than only
waiting until the 700 ms animation is over. Active and settled screenshots are
saved for inspection.

`testIncrementalRevealMatchesStaticLayoutWhileResizing` appends four Markdown
chunks 80 ms apart, including an initially unclosed code fence, at widths 720,
360, 520 and 360. It compares live height with a static reference and repeats
with reveal suppressed. This checks component policy, not actual system
Accessibility preference changes, pixel-perfect glyph positions, or user input.

For diagnostic stack sampling, pass
`TEST_RUNNER_NOLA_PROFILE_TRANSCRIPT_SWITCH=1` to the targeted xcodebuild test.
The test samples only its own process for three seconds, writing
`/tmp/nola-transcript-switch-stacks.txt`. Diagnostic sampling is off by default
and recorded in the metrics; sampled timings must not be used as benchmark
comparisons.

### Remaining work

- AppKit CA-commit transaction warnings still occur in native UI tests; the
  scroll callback change did not establish their root cause or eliminate them.
- Actual accessibility preference changes, selection/link interaction during
  reveal, dark appearance, and long-running continuous reveal need further
  integration coverage.
- Cold conversation selection still measured approximately 232 ms in the last
  performance run and is distinct from warm transcript switching.
- These results do not establish completion of the Architecture V2 migration,
  parity across clients, or correctness of every live event/permission workflow.

## 2026-09-05: Android data-source integration gap

A source audit after fixing the mobile FFI lifecycle test found an actual
production caller still using the removed aggregate contract:

- `MemoryProfileDataStore.saveProfile` calls `canonical::upsertDataSource` for
  every source through a Kotlin function reference. A search limited to
  `upsertDataSource(` misses this caller.
- `AndroidApplicationRuntime.upsertDataSource` still sends `upsert`, embeds
  validation/configuration in legacy JSON strings, and does not request a
  validation token. Current Application rejects this command.
- `listDataSources` expects `metadata_json` and silently drops records without
  it; current typed lifecycle results do not use that client-owned envelope.
- `deleteDataSource` omits `expected_revision` required by the current command.
- Usage recording still sends `metadata_json` instead of the typed `metadata`
  object. Nullable usage fields are also decoded permissively and failures are
  silently dropped.

The old `saveProfile` reconciliation also rewrites sources, usage, Skills, MCP,
and memories and deletes records absent from the submitted profile. A stale
profile is not an authoritative deletion command for independent aggregates.
This must be replaced with per-aggregate intentions and revision-aware writes,
not a compatibility `upsert` handler or a synthetic new metadata wrapper.

Required integration work:

1. Introduce typed Android projections/submissions for the canonical source
   configuration, authorization, validation, read state and revision.
2. Route create/update/validation/enable/delete through public Application
   lifecycle commands and preserve Host ownership of actual reads and secrets.
3. Stop mutating data sources as a side effect of unrelated profile saves.
   Emit explicit source mutations from the responsible repository/UI actions.
4. Exercise Android caller serialization against the real Application ABI,
   including create, failed validation, edit, stale revision, list, reopen and
   delete. Rust FFI tests alone do not cover Kotlin payload construction.

This section records an identified unresolved production gap. Android has not
been fixed or compiled by this audit. The iOS `saveDataSource` path inspected
already uses lifecycle commands; that observation is not full iOS validation.
