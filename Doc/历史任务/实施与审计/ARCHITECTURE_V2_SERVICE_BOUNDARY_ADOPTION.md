> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Architecture V2: Application Service boundary adoption

Status: partial implementation, not Architecture V2 completion.
Evidence date: 2026-09-06. Executed environment: macOS arm64.

## Required boundary

Application owns canonical mutations and durable command receipts. A client
transport failure is not evidence that a command failed to commit. Retrying a
mutation must preserve its request identity and payload. Clients must not repair
an uncertain result by issuing a new mutation with a new identity.

RPC waiting and durable task execution are separate lifetimes. A transport
deadline bounds how long a caller waits for one receipt; it does not cancel an
Application task or establish that its effects were rolled back.

## Implemented in the shared Rust client

Implementation: `core/nola-service-client/src/lib.rs`.

- Health RPCs use a 250 ms deadline shared by connect, write and response read.
- Ordinary RPCs default to 30 seconds. `with_request_timeout` accepts an explicit
  positive, representable budget. Health retains its separate short budget.
- Unix socket connection establishment is nonblocking when a deadline exists.
  The Linux full-queue EAGAIN path retries connection establishment within the
  original budget; it does not resend an Application command.
- Read/write timeouts use the remaining budget rather than resetting after each
  byte. A peer cannot extend a response indefinitely by trickling bytes.
- Submission transport errors include the request ID and an uncertain-outcome
  warning. Submission does not automatically resend after a transport failure.
- Service stderr is drained continuously into a 16 KiB byte tail. Reader shutdown
  is cancellable and does not require pipe EOF. UTF-8 loss replacement means the
  displayed string can be larger than the retained byte buffer.
- Owned startup processes are killed and reaped when startup exits with an error.
  Successfully started processes and stderr captures are retained in the registry.

The budget covers one RPC, not the preceding service startup/readiness phase.
Swift client adoption and its separately executed gates are recorded below.

## Model mutation adoption

- CLI/Web local-model parameter saves now dispatch `ConfigureParameters`, rather
  than upserting a client snapshot of the entire profile.
- Android parameter editors now capture canonical snapshots and dispatch
  revision-checked `ConfigureParameters` through the versioned JNI submission
  boundary. Editing does not register a missing model or restore a tombstone.
  Batch reset remains a separate, unconditional operation.
- Application/Store preserve canonical identity, selection and source fields
  during parameter updates. Batch failure rolls back preceding writes.
- Provider/runtime identity cannot be reassigned through a live or tombstoned
  profile ID. This is enforced below individual client adapters.

See `ARCHITECTURE_V2_MODEL_CONFIGURATION_ADOPTION.md` for the broader model
configuration adoption work and its remaining client paths.

## Executed evidence

| Scope | Executable coverage | Result |
| --- | --- | --- |
| Rust client | `cargo test -p nola_service_client` | 9 unit tests and 1 dependency-boundary test passed |
| Service lifecycle | `cargo test -p nola_service --test client_process_e2e --test service_process_e2e --test model_runtime_cold_restart_e2e` | 8 tests passed |
| Uncertain commit | `cargo test -p nola_service --test command_timeout_recovery_e2e` | 1 real process/proxy/socket test passed |
| Model transactions | `cargo test -p nola_application --test model_mcp_port_e2e --test model_transaction_visibility_e2e --test model_sync_atomicity_e2e` | 13 tests passed |
| CLI model adapter | `cargo test -p nola_cli --bin nola local_models::tests` | 6 tests passed; Application runs in-process under cfg(test) |
| Android model parameters | `ModelParameterTransactionEndToEndTest` on the Android 15 arm64 emulator | 2 instrumentation tests passed through production adapter/JNI/SQLite |

The uncertain-commit test starts the actual service and forwards real protocol
frames through a Unix socket proxy. The proxy delays the first mutation receipt
after receiving it from the service. Assertions cover:

1. The client reports a response-read failure with the original request ID.
2. Exactly one mutation was sent before the explicit retry.
3. The original ID retrieves a committed durable receipt.
4. Explicit retry sends one additional request without changing the model record.
5. Reusing the ID with different parameters is rejected without changing data.
6. After service restart, the entire original receipt and model record are intact.

This is not a real-LLM or UI test. It proves the transport/receipt boundary in the
specified scenario, not every crash point, caller recovery path or platform.

Local execution logs:

- `/tmp/nola-service-rpc-budget-20260906.log`
- `/tmp/nola-service-rpc-budget-ipc-20260906.log`
- `/tmp/nola-command-timeout-recovery-strict-20260906.log`
- `/tmp/nola-model-transaction-regression-20260906.log`
- `/tmp/nola-cli-model-parameters-20260906.log`
- `/tmp/nola-android-parameter-edit-20260906.log`

Logs under `/tmp` are transient evidence, not committed test artifacts. Existing
unused-code/import warnings remain; these results are not a warning-free claim.

## Remaining work and acceptance evidence

| Gap | Required evidence before closure |
| --- | --- |
| Client recovery after UI/process restart | Persist submission identity and payload before dispatch; restart the client after an uncertain commit and recover without a new mutation |
| Typed uncertainty | Distinguish validation, transport uncertainty, rejection and revision conflict without parsing human-readable error strings |
| Optimistic concurrency | Carry caller revision expectations through commands; simultaneous stale edits must conflict rather than silently overwrite |
| Service registry lifecycle | Repeated exits/restarts must reap and remove dead process/capture entries; concurrent startup must not accumulate services |
| Startup stderr diagnostics | Exercise the complete startup failure path with large output and inherited stderr, not only the capture helper |
| Linux behavior | Execute full-queue, timeout, service lifecycle and receipt recovery tests on Linux, not just compile conditional branches |
| Mac parity | Validate native transport startup, request deadlines and receipt recovery with actual Mac UI interactions |
| Client mutation adoption | Audit remaining model selection/registration, credential writes, and legacy preference fallbacks; parameter-only tests are insufficient |
| Slow legitimate commands | Measure large projections and long task submission separately; move long execution behind durable task status instead of increasing every RPC timeout |
| Full V2 boundary | Complete ownership and revisioned projection adoption for conversations, tools, permissions, memory, skills, MCP, data sources and task recovery |

The next implementation should close caller identity/recovery and lifecycle gaps,
not add automatic transport retry with a newly generated request ID. No schema
migration, user database reset, or production credential modification was part of
this work.

## Follow-up: service ownership and task creation

Subsequent implementation on 2026-09-06 added:

- The Rust client removes exited owned services and associated stderr captures
  when checking service availability. Inspection errors retain ownership rather
  than being interpreted as proof of process exit.
- Concurrent startup within one client process is serialized per socket, with
  health rechecked after locking. Weak lock entries are pruned. A real-process
  fixture with eight concurrent callers recorded exactly one service launch.
- The service acquires a nonblocking exclusive database-side file lock before
  touching its socket. Existing database symlinks resolve to the canonical path.
  A competing service using another socket is rejected; the original service
  remains healthy, and a replacement can acquire ownership after its exit.
- Lock files retain their inode after release. The lock is advisory and only
  enforced by cooperating Service processes; it does not yet prevent direct
  Application/Store users from opening the database. Hard-link aliases and
  clients racing across processes remain outside the executed ownership test.
- CLI external-executor, MCP and agent entry paths submit `CreateRuntimeTask`
  directly. The shared task port no longer accepts a runtime-state object.
- Application creates the task and its initial `created` event in one Store
  savepoint. Duplicate identity is checked within that boundary. CLI no longer
  follows creation with a separate initial-event RPC. Existing MCP task takeover
  remains a separate lifecycle path, not recreation.

Additional evidence:

- `cargo test -p nola_application -p nola_service --no-fail-fast`: 155 tests
  passed after the atomic task-creation change.
- `cargo test -p nola_cli --bin nola task_runtime::`: 11 tests passed.
- `cargo test -p nola_cli --bin nola store::task::`: 2 tests passed after updating
  assertions for exactly one canonical initial event, while retaining terminal
  event, source-content and notification-acknowledgement checks.
- `cargo check -p nola_cli --bins` passed after typed creation adoption.
- A real service test installs a temporary SQLite trigger that rejects event
  insertion. Failed creation leaves neither task nor initial event; successful
  creation produces one event and a duplicate creation does not add another.

The expanded Service suite initially exposed two obsolete recovery fixtures that
omitted required scheduling metadata. They now construct the creation contract
explicitly, with resource budgets and automatic process cleanup. Production
validation was not relaxed to accommodate those fixtures.

Latest local logs:

- `/tmp/nola-task-creation-full-regression-20260906.log`
- `/tmp/nola-cli-task-events-fixed-20260906.log`
- `/tmp/nola-cli-task-create-atomic-20260906.log`
- `/tmp/nola-database-service-ownership-20260906.log`
- `/tmp/nola-startup-singleflight-ipc-20260906.log`

Mac packaged-service validation is a separate gate; Rust results above do not
prove native UI integration or complete cross-platform Architecture V2 adoption.

## Native Mac verification and outstanding rendering warning

After the ownership/task changes, NolaMac compiled and all 14
`MacCanonicalStoreEndToEndTests` passed. All 8 `MacAssistantResponseVisualTests`
also passed. The 720- and 920-point startup-error screenshots were inspected:
the localized title, explanation, disclosure label and retry button were visible
without clipping. The existing retry test sends actual window mouse events and
successfully mounts content after repairing only its isolated fixture database.

The combined visual run still emitted AppKit's "Invalid attempt to open a new
transaction during CA commit" warning. Isolation produced narrower evidence:

- Running only `testStartupRetryButtonLoadsContentAfterExplicitFixtureRepair`
  passed without that warning in its log.
- Running only `testStartupFailureRendersWithoutMountingConversationContent`
  passed, then emitted the warning after the test-completion line.

This points investigation toward screenshot-window cleanup or deferred rendering
work. It does not prove the warning is harmless, exclude a production rendering
defect, or justify suppressing it. A stack trace at
`NSCGSTransactionCreatedDuringCommitError` is still needed before assigning root
cause. No production UI change was made based only on this timing correlation.

Result bundles:

- `/tmp/nola-mac-service-ownership-20260906.xcresult`
- `/tmp/nola-mac-service-visual-20260906.xcresult`
- `/tmp/nola-mac-retry-isolated-20260906.xcresult`
- `/tmp/nola-mac-failure-isolated-20260906.xcresult`

## Connection resource admission and Linux gate

The Service now bounds connection workers, including projection waiters, to at
most 128. Admission is further reduced using `RLIMIT_NOFILE`, reserving 64 file
descriptors and two descriptors of headroom per worker. Request reading borrows
the accepted socket instead of cloning its descriptor. Request reads and receipt
writes each have their own 30-second frame deadline. These limits are not task
execution deadlines.

Pressure testing exposed a real service exit at `accept`: EMFILE (too many open
files). That path now backs off rather than terminating the canonical writer.
This does not guarantee availability under system-wide descriptor exhaustion or
reserve health-check capacity when all worker slots are occupied.

The full Service suite passed 39 tests on macOS after the descriptor fix. The
ownership, 256-connection pressure and uncertain-commit tests passed three extra
consecutive runs. The timeout proxy fixture explicitly restores blocking mode
after accept, avoiding macOS inherited nonblocking sockets causing spurious
WouldBlock failures.

The local Docker daemon was unavailable when Linux verification was attempted.
`.github/workflows/rust-build.yml` now includes a `service-boundary` matrix for
Ubuntu 24.04 and macOS 14, running both `nola_service_client` and `nola_service`
with the lockfile, a bounded job timeout and uploaded test logs. Adding this job
is not evidence that either remote runner has executed or passed. Linux runtime
validation remains open until an actual run is inspected.

## Model revision preconditions and client adoption (2026-09-06)

`ConfigureParameters` has an explicit `model_profile:<id>` revision scope.
Application compares the envelope precondition inside its write unit of work,
including tombstone revisions. Conflicts are durable structured receipts;
replaying an old successful request does not reapply its mutation. Other command
scopes still reject unsupported envelope preconditions rather than ignore them.

CLI Web configuration reads canonical parameters and `configurationRevision`
together. HTTP saves require `expectedRevision`, return 409 for conflicts, and
retain the open editor's input. Mac parameter editors now retain a snapshot
revision and obtain their next revision from the committed receipt, not a later
read. Native Mac parameter and store tests passed (18 tests in the adoption run).
iOS parameter editors now capture the canonical snapshot and submit through the
versioned FFI boundary; all 5 ModelSelectionCanonicalEndToEndTests passed.
Android model editors use the same revision contract; all 3 isolated Android 15
arm64 ModelParameterTransactionEndToEndTest instrumentation tests passed.
Android chat settings also retain the opening snapshot, await the commit before
reporting success, and preserve parameters not exposed in the editor. Its Kotlin
production/test compilation and JVM unit suite passed, but chat-editor visual
and conflict-interaction instrumentation coverage remains open.
API profile editors, remaining unconditional parameter-reset callers, and other
resource types are not covered by these model-parameter adoption results.

Repeatable browser gate (no LLM or model download):

```sh
cargo build -p nola_cli --bin nola -p nola_service --bin nola-service
python3 scripts/test_web_model_revision_e2e.py
python3 scripts/test_web_model_revision_e2e.py --locale zh-CN --width 390 --height 844
python3 scripts/test_web_model_revision_e2e.py --locale en-US --width 390 --height 844
```

The real CLI Web/Service browser runs passed in English desktop and Chinese and
English 390x844 viewports. Screenshots were inspected: conflict text visible,
input retained, successful value reopened, no horizontal overflow or page script
errors. Fixtures use isolated homes/databases and clean up owned process groups.

## Receipt parity and native communication deadlines (2026-09-06)

Service submission and receipt lookup now both use Application's protocol receipt
projection. A recovered admission stays `accepted`; transports must not fabricate
a terminal rejection. The real Service process regression verifies matching
submission/lookup receipts across two starts without reexecuting the command.
Service, Service Client and mobile FFI suites passed after this change.

Mac preserves the accepted receipt after its recovery window and surfaces a
distinct pending-outcome error with the original request ID, localized in all
nine Mac catalog languages. A transport failure enters at most one receipt
recovery window; it does not implicitly resubmit the mutation. Rust's synchronous
client also preserves the request/client identities in its pending error rather
than describing a valid receipt as a protocol failure.

Mac ordinary RPCs now have a 30-second monotonic deadline shared by connect,
write and read. Recovery lookups use the remaining recovery-window budget.
This is not a task-execution timeout and does not bound preceding service startup.
Health probes keep their shorter 250-millisecond deadline.

Executed native evidence:

- `/tmp/nola-pending-real-service-mac.xcresult`: 16 tests passed, including an
  isolated interrupted admission across two actual Mac-owned service starts.
- `/tmp/nola-mac-request-deadline.xcresult`: 18 tests passed, including silent
  and trickling socket peers with a shortened explicit transport deadline.
- `/tmp/nola-pending-client-tests.log`: 12 Rust unit tests and one boundary test
  passed. This is not evidence of native UI rendering or Linux execution.

Further native evidence:

- `/tmp/nola-mac-write-backpressure.xcresult`: 19 tests passed, including a
  2 MiB request sent to a connected peer that never reads.
- `/tmp/nola-mac-cancel-backpressure.xcresult`: 21 tests passed, including
  cancellation of blocked writes and silent responses. These use the ordinary
  30-second transport default and assert cancellation returns within 0.8 seconds.
  Fixture cleanup deliberately waits longer than the measured request itself.
- The first full-backlog test failed because it assumed a pending connection.
  This Mac returned ECONNREFUSED after successful connections filled the queue.
  The corrected test retains queue-saturation and bounded-return assertions,
  distinguishing explicit refusal from a pending connection deadline. Neither
  outcome proves Linux backlog behavior or a successful recovery after pressure.

Remaining gates include mid-frame disconnect recovery, mobile pending-outcome
recovery, native pending-state visual interaction, full-duration deadline
coverage, and availability after backlog pressure subsides.
The mobile FFI submission entry currently supports only model parameter commands;
this work does not prove full Architecture V2 adoption or remove all direct writers.

## Mobile memory submission and editor snapshots (2026-09-06)

This section updates the earlier mobile submission scope. The versioned mobile
submission boundary now admits product-setting mutations, memory Commit/Delete/
Replace, user-skill mutations and MCP mutations, in addition to model parameter
configuration. Queries remain on the query/execute path. This is not complete
adoption of all Application commands or client-side receipt recovery.

Memory Replace runs only inside durable command admission. Application compares
all expected canonical records with current records, validates candidates before
writing, and replaces the set in the same write unit of work. A stale snapshot
is rejected with `memory_snapshot_conflict`; it is not yet a structured Conflict
receipt. Replacement is bounded to 1,000 records. Returned canonical IDs, rather
than client-proposed IDs, determine retained records after deduplication.

The iOS memory profile carries a nonserialized canonical snapshot. Saving uses
that baseline instead of a client-side upsert/delete diff. A failed initial query
leaves no usable save baseline. The broader profile save still writes skills,
MCP and document settings separately and is not an atomic multi-resource command.

Android also retains the raw canonical memory snapshot separately from its
editable projection. Unchanged records reuse the raw semantic record to preserve
fields absent from the UI model. The chat memory editor now calls a memory-only
save operation: it does not resubmit Skill, MCP or document lists. Its next baseline
comes directly from the Replace receipt, not from a later query that might include
another writer's changes. The displayed profile's load snapshot is retained for
editing; explicit editor reopening/reload and conflict-resolution UX still need
end-to-end interaction coverage.

Executed evidence:

- iOS `IOSApplicationRuntimeTests`: 8 tests passed, including stale-profile save
  rejection and memory persistence/deletion across reopening. Local result bundle:
  `/tmp/nola-ios-memory-snapshot-save.xcresult`.
- Android `AndroidMemoryCommandEndToEndTest` and
  `AndroidApplicationRuntimeRealEndToEndTest`: 9 instrumentation tests passed on
  the Android 15 arm64 emulator after compiling the production app and test APK.
  Local log: `/tmp/nola-memory-editor-regression.log`.
- Android coverage includes real JNI semantic commit/read/delete, stale profile
  rejection without losing another editor's addition, explicit removal across
  database reopening, consecutive saves using committed snapshots, and rejection
  after an intervening writer. Snapshot comparison checks complete records by ID
  and rejects duplicate IDs; query ordering is not a replacement contract.

These are isolated persistence/JNI integration tests, not screenshots, UI-driver
sessions or real-LLM tests. The emulator was shut down after execution. No user
application database, credentials or physical device permissions were modified.

Remaining boundary work includes client restart recovery with durable request
identity, structured memory conflicts with localized UI, editor cancellation and
concurrent-save interaction tests, and removal of the remaining broad profile
save paths that independently rewrite Skill/MCP/document resources. General
profile loading still assembles multiple resource queries instead of consuming
one revisioned Application projection. These gaps prevent a V2 completion claim.

## MCP editor command adoption (2026-09-06)

Application now provides `McpCommand::Configure` and `DeleteChecked`. Both require
an admitted durable write transaction and a positive expected canonical revision.
They reject stale revisions and missing/deleted records. Configure starts from the
stored record, retaining canonical identity and creation metadata. Editing cannot
register or resurrect a server. Legacy Upsert/Delete still exist and are not
protected by these new preconditions; their remaining callers must be replaced.
Conflicts currently use rejected receipts with a machine-readable message prefix,
not a structured Conflict outcome. This is an explicit remaining protocol gap.

The mobile submission boundary admits both commands. Android MCP projections now
retain canonical revision as transient editor metadata and fail on decode errors
instead of silently omitting malformed servers. Android capability-center edits,
enable changes and confirmed deletions use individual commands rather than the
broad memory-profile save. Edit and deletion dialogs retain the original server
identity instead of resolving an old list index after a refresh. New servers use
independent generated IDs, allowing identical display names. Creation now uses
the admitted `McpCommand::Create`: identity checks include tombstones, and a
duplicate identity cannot replace or resurrect a server. Replaying an old create
receipt after deletion returns that receipt without recreating its effect.

The MCP editor keeps its draft on submission failure. Only the committed callback
closes it; pending saves disable text editing, save and cancel, and expose progress.
The ViewModel rejects repeated MCP submissions while saving. Core still owns the
revision check; the UI does not retry a conflict using a freshly fetched revision.
Existing client resource-access checks remain, but this does not prove permission
enforcement is complete at the Application boundary.

Executed evidence:

- Rust `mcp_editor_receipt_e2e` and `model_mcp_port_e2e`: 7 tests passed. Coverage
  includes stale edit/delete rejection, unchanged canonical creation metadata,
  durable rejection replay after reopening, and no edit resurrection of a tombstone.
- Android `AndroidMcpEditorEndToEndTest`, `AndroidMemoryCommandEndToEndTest`,
  `AndroidApplicationRuntimeRealEndToEndTest` and `McpEditorInteractionTest`:
  12 tests passed on the Android 15 arm64 emulator. Latest local log:
  `/tmp/nola-mcp-editor-ui-regression.log`.
- The Compose test renders the production MCP editor and submits through the real
  Android runtime/JNI/Core/database boundary. It checks disabled controls during
  busy state, visible conflict text, retained draft and unchanged newer data.
  Its callback wiring is a test harness, not the production ViewModel/navigation;
  successful dialog closure, cancellation, accessibility and restart recovery
  remain unverified at full-screen level. No visual screenshot review was performed.

Android chat-side MCP list editing and generic profile save still need removal of
broad writes. iOS/Mac/CLI/Web editor adoption, typed and
localized conflict UX, and corresponding Skill commands remain open. These tests
do not establish complete cross-platform resource ownership or V2 completion.

Create-only follow-up evidence: 8 Rust Application integration tests and 13 Android
instrumentation tests passed, including duplicate live identity rejection,
tombstone rejection through JNI after reopening, and durable create receipt
replay without resurrection. Logs: `/tmp/nola-mcp-create-core.log` and
`/tmp/nola-mcp-create-android.log`. Stable client draft/submission identity across
process restarts is not implemented by these tests or by UUID generation alone.

## Web MCP isolation and production-build gate (2026-09-07)

Web capability and chat MCP editors now submit a separate `mcp_baseline` containing
stable server IDs and the revisions observed by the editor. The catalog adapter
builds explicit Create/Configure/DeleteChecked operations and submits them as one
Application ApplyChanges command. It does not query the latest catalog to invent
an editor baseline or delete servers added since that baseline. Application rejects
unchecked or nested commands in the batch and rolls back earlier operations when
any later operation fails.

The MCP branch of `/api/agent/settings` now returns before document, Skill and
ordinary setting writes. Mixed MCP/other-resource submissions are rejected before
mutation. A missing MCP baseline is also rejected. Resource-access reconciliation
and capability discovery remain before the MCP commit; broader relocation of
these responsibilities into Application is still required. Errors from MCP commit
are currently grouped into HTTP 409, which does not yet distinguish transport
uncertainty, invalid payloads and actual revision conflicts.

The unused `cli/src/store/mcp.rs` implementation and its module declaration have
been removed. This eliminates its duplicate unversioned upsert/delete-diff path.
Stable-ID validation and cold-reopen coverage now live with the catalog adapter.
During removal, an orphaned `#[cfg(test)]` temporarily gated the following memory
module. Production-bin compilation exposed this regression despite passing MCP
unit tests. The orphaned attribute has been removed; production and test build
configurations must remain separate acceptance gates.

Executed on 2026-09-07:

- `cargo build -p nola_cli --bins -p nola_service --bin nola-service`: passed.
- `cargo test -p nola_cli --bin nola mcp_catalog_port::tests`: 2 tests passed.
- `python3 scripts/test_web_mcp_baseline_e2e.py`: passed against the rebuilt CLI
  Web and actual Service processes, with isolated HOME/database and real token
  login. Concurrent additions survived deletion from an older baseline; stale
  mutation returned 409; mixed mutation and missing baseline returned 400;
  other-resource projections remained unchanged.
- Combined log: `/tmp/nola-cli-production-memory-fix.log`. Existing unused-code
  and unused-import warnings remain.

The HTTP test is not a browser interaction/screenshot test. Browser draft behavior,
protocol error localization, post-commit projection races and durable browser
submission identity/recovery remain unverified or incomplete. General non-MCP
settings saves still combine other resource writes. The shared Core's historical
Upsert/Delete variants and remaining native caller paths have not all been removed.

## Skill editor adoption and client evidence (2026-09-07)

### Canonical editing contract

- `SkillCommand::GetEditor` returns the complete canonical record together with its Store revision. An editor must retain that snapshot rather than reconstructing its baseline at save time.
- `CreateUser` rejects both live and tombstoned identities. `ConfigureUser`, `SetEnabledChecked`, and `DeleteUserChecked` require an existing live identity and an exact revision match, inside durable command admission.
- User editing cannot modify or delete built-in Skills. Built-in activation may be toggled with a revision precondition. Existing version-content immutability and content-digest validation remain enforced by Store.
- Mobile receipt admission now accepts these mutations and MCP `ApplyChanges`. Queries remain outside the mobile mutation submission endpoint.
- CLI in-process execution now admits the guarded Skill/MCP mutations and memory replacement through receipts, including atomic MCP batches. This closes the earlier difference where service-backed execution admitted them but `--no-save` and in-process tests executed them without the required transaction. This is not yet a universal command/query classification for every Application command.

### Android implementation

- `AndroidApplicationRuntime.SkillEditorSnapshot` retains the full canonical JSON record and its revision. Editing changes the requested name/content while preserving metadata and complete tool references, including references that the simplified uploaded-content list cannot represent.
- The intelligence Skill page creates individual Skills and opens a canonical snapshot before editing. Save success closes the editor; rejection preserves its draft. Delete confirmation retains identity and revision, rather than resolving an index from a later list.
- Skill activation/deletion/editing no longer save the entire `ChatMemoryProfile`. Memory and MCP state are not rewritten by these page actions.
- Reading an editor snapshot does not increment the save-success counter. Errors are displayed once inside an open editor instead of stacking a second error dialog above it. Cancel clears that editor error.
- Skill save-error text now uses resources in the seven existing Android language directories. The message does not assert that an uncertain operation definitely failed to commit. Underlying diagnostic errors remain in ViewModel state.

### Executed evidence

- `cargo test -p nola_application --test skill_editor_receipt_e2e --test mcp_editor_receipt_e2e`: five tests passed. Covers stale edits/deletes, built-in restrictions, identity resurrection prevention, receipt replay after reopening, and batch rollback. Log: `/tmp/nola-skill-editor-core.log`.
- `cargo test --manifest-path core/nola-mobile-bindings/Cargo.toml --lib`: 21 tests passed, including actual C ABI admission and reopened database receipt replay. Log: `/tmp/nola-mobile-resource-admission.log`.
- CLI production build of all binaries and `nola-service` passed. Two Application-port tests and two MCP-catalog tests passed. Logs: `/tmp/nola-cli-editor-admission.log`, `/tmp/nola-cli-editor-production.log`.
- Android `AndroidSkillEditorEndToEndTest` exercises real JNI, version rejection, metadata preservation, deletion, and reopening. MCP regression cases were also executed; the four-test run passed. Log: `/tmp/nola-android-skill-editor-e2e.log`.
- `SkillViewModelEndToEndTest` uses a real runtime and datastore. It covers same-name independent identities, reading without a save notification, stale save/delete rejection, success callback behavior, and unchanged memory/MCP state. The three-test ViewModel/editor/JNI run passed. Log: `/tmp/nola-android-skill-vm-e2e.log`.
- `SkillScreenEndToEndTest` renders the production `IntelligenceScreen` with a real ViewModel. It navigates from the section switcher and gallery into the editor, creates a concurrent canonical edit, submits a stale draft, cancels, and reopens the latest record. It exposed the duplicate error dialog missed by component-only tests; that defect was fixed.
- The latest three-test screen/editor/ViewModel run passed after UI changes. Log: `/tmp/nola-android-skill-polish-e2e.log`. Actual screenshots were exported and visually inspected at `/tmp/nola-skill-screen-evidence/gallery.png` and `/tmp/nola-skill-screen-evidence/conflict.png`. These are local temporary evidence, not portable repository artifacts.
- Screenshot export initially failed because UI automation does not parse shell operators in a raw command string. The test now sends the script through a shell stdin pipe, checks export success, and preserves images outside the package before Gradle cleanup.

### Remaining requirements and limits

- This does not complete Architecture V2 or all client Skill adoption. The iOS library and chat-side gallery now use checked single-resource commands, as detailed below. macOS, CLI/Web Skill editing and remaining Android chat-side controls still require an explicit audit and migration; old unchecked/broad save APIs remain.
- The Android profile is still composed from multiple queries, not one consistent Application projection snapshot. Its Skill decoder now rejects malformed user records instead of silently dropping them. Its simplified display model still cannot represent all canonical metadata; full editor snapshots preserve that metadata during edits.
- Client-side resource ownership/grant checks are not proof of canonical Core authorization. The identity/access metadata projection and authoritative authorization path still need consolidation.
- Revision rejections still expose string diagnostics rather than a shared typed conflict outcome. UI guidance currently asks users to retain their edits and reopen; there is no integrated compare/rebase flow or durable draft recovery yet.
- Android Skill/MCP datastore operations and profile flow now use IO dispatching. A dedicated threading/latency audit remains: passing emulator correctness tests does not establish responsiveness for large catalogs or documents. iOS capability operations are still synchronous.
- The seven translations compiled, but only the tested default-language emulator screenshots were visually inspected. Small screens, large fonts, keyboard-visible editing, other locales, and full application navigation outside `IntelligenceScreen` remain unverified.
- Tests use isolated databases and a dedicated owned emulator. They do not call an LLM and do not establish model tool-selection, Skill execution, or long-task correctness.

## iOS capability adoption and real UI evidence (2026-09-07)

### Implemented boundaries

- The personal capability library and the chat Skill sheet share `PersonalCapabilitiesLibraryView`. Skill and MCP edits, toggles, and deletes use single-resource commands with canonical editor revisions. The chat gallery no longer submits an entire memory profile when closing.
- MCP edits retain unknown configuration fields and the canonical creation time. Editing name/endpoint/description does not implicitly toggle activation. Deletion uses the snapshot retained when requesting confirmation.
- Skill instructions preserve leading/trailing whitespace and newlines. Only emptiness validation uses a trimmed copy. Imported UTF-8 text is bounded before loading into the editor; it opens as an unsaved draft instead of being silently truncated or evicting older Skills.
- `saveMemoryText` has authority only over memory records. It requires the editor's canonical memory baseline and does not rewrite capabilities or uploaded documents. The chat More Settings page uses this method; its Skill switches submit independent checked commands.
- `loadCapabilityLibrary` reads only Skill/MCP resources and rejects an undecodable user capability rather than presenting a partial list as complete. The UI retains the previous list and blocks mutations after a read failure until retry succeeds. This is a scoped multi-query read, not yet an atomic cross-resource snapshot.

### Executed evidence and discovered failures

- `/tmp/nola-ios-capability-read-isolation.xcresult`: 15 real Core/SQLite integration tests passed. Coverage includes cold reopen, stale revisions, identity resurrection rejection, preservation of additional MCP configuration, capability isolation during memory writes, and recovery after a malformed capability is removed. Corrupt document settings do not block capability reads.
- `/tmp/nola-ios-skill-lifecycle-accessibility.xcresult`: the production iOS UI lifecycle test passed through creation, editing, app restart, verbatim content checks, disabling, another restart, deletion confirmation, and a final restart proving absence. It uses a fixed per-test run ID and an isolated database, not user persistence.
- The lifecycle test initially exposed whitespace trimming at the editor boundary. Production code was fixed without weakening the content-equality assertion.
- Expanding the lifecycle test exposed the card identifier being inherited by its children, hiding the switch/menu identifiers. Explicit accessibility containment fixed the hierarchy; the switch also has a meaningful name.
- Six screenshot attachments were captured in the lifecycle result. The deletion confirmation and post-restart empty state were visually inspected. Earlier edit/reopen and gallery screenshots were also inspected. These are local temporary artifacts, not committed binaries.
- Navigation-only UI coverage passed separately, but is not evidence for write correctness. UI execution requires `INCLUDE_UI_TESTS=1`; omitting it produces no available test bundles when selecting only UI tests.

### Still required

- Remove the remaining broad `saveProfile`/`clearProfile` APIs and the old context-sheet bulk editor helpers after migrating document and context controls. The remaining context-sheet memory callback still uses the old path.
- Move iOS synchronous storage/file work off the UI thread; bound catalog/content materialization and provide a consistent projection contract.
- Cover the chat-sheet entry point, file picker/import errors, concurrent editor conflict recovery, read-failure retry UI, all locales, and small-screen/large-font layouts with actual UI tests.
- New read/import recovery strings cover the seven existing iOS localizations. This does not establish complete error localization across all services or visual review of every locale.
- Preserve built-in and opaque canonical Skill identities in a richer client catalog model; the current user-uploaded view model still assumes UUID identities and excludes built-ins.
- Replace string error classification with typed conflict/uncertain-result handling, durable draft/submission recovery, and authoritative Core authorization. These UI tests do not prove LLM Skill selection/execution or long-task correctness.

## Skill library transaction adoption (2026-09-07)

- Application `Skill.ApplyChanges` accepts bounded, checked editor mutations in one durable command receipt. Nested batches, unchecked operations and queries are rejected. Conflict rolls back the complete batch; receipt replay after reopening does not execute it again.
- `Skill.RestoreBuiltIns` checks the complete built-in identity set, each editor revision, and the `skill.builtin.hidden` setting revision. Enabling and clearing visibility are one transaction.
- `Skill.RemoveBuiltIn` checks both revisions, rejects user Skills and malformed visibility schemas, preserves other hidden identities, and atomically disables and hides the selected Skill.
- Mac registry removal and restoration now call those Application commands rather than saving a visibility setting separately. Mac unchecked MCP upsert/delete wrappers are removed; user Skill deletion requires the caller's editor snapshot rather than an ID-only deletion or a freshly loaded revision at save time.
- Idempotent enable operations preserve the complete editor snapshot, including revision and update time. A real state change increments the revision once; a stale precondition is rejected even for an otherwise no-op operation.

Evidence before the final unchecked-wrapper cleanup: nine `skill_editor_receipt_e2e` database tests and eight selected Mac Skill/MCP tests passed; mobile bindings and CLI passed compilation checks. The cleanup is being validated with the same Mac tests plus the canonical Skill reopen test. These are isolated database/Application integration tests, not visual UI or real-model sessions.

Remaining: bundled Skill registration still uses upsert during catalog access; other clients and Core retain older mutation variants. Generic product-setting writes have not yet been restricted from capability-owned keys. Client catalogs still need consistent revisioned projections rather than separate list/editor/visibility queries. This section does not declare the full V2 architecture complete.
