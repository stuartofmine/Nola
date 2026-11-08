# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Application journal producer implementation status

## Acceptance — identity, recovery and incremental publication cutover (2026-10-08)

The requested supported-platform cutover is implemented and verified. The table
below is the current acceptance record; subsequent sections preserve historical
failures and intermediate states rather than redefining the final result.

| Requirement | Current evidence |
| --- | --- |
| Freeze the original invocation/Run; revalidate current credentials and permissions before recovery | Rust workspace r44b exit 0; exact invocation, renewal/CAS and real SQLite cold-reclaim regressions pass. |
| Recover actual platform execution rather than only decode a binding | Mac r45 whole-app cold reconstruction test passes after killing the owned Host at permission waiting; Android real JNI permission-wait/reopen/seal test passes; iOS r46 Store/HTTP-parser cold recovery test passes. |
| One committed incremental publication contract | Rust local/remote journal receipt tests pass; Service long-stream test reads actual cursor-bound text and checks bounded RSS/ACKs; ordinary content does not invalidate row windows, terminal transitions do. |
| Retire obsolete foreground full-checkpoint paths | First Host response and hot cancellation pass guarded tests that reject full checkpoint-body reads/writes; hot/cold selective execution state matches. Swift/Android no longer accept the fake checkpoint-persistence Host branch. |
| Exact terminal state and observer ownership | No success from trace/EOF; Mac r46's 43 recovery/observer tests pass after the final stale/duplicate completion fix. Android JVM/JNI and iOS r46 cover exact owner-bound terminal seals. |
| Client integration | Mac full r45: 932 cases, 12 skipped, zero failures; final affected Mac suites r46: 43 passed. iOS full r46: 566 logical cases, 557 passed, nine skipped, zero failures (563 passed expanded device runs). Android JVM: 358 passed, two skipped; real JNI: nine passed. |
| CLI/Web and real-provider behavior | Rust workspace r44b includes CLI 1533 passed/three ignored and PTY 84 passed. Web r46: 264 passed; Svelte check zero errors/warnings; independent JavaScript journal vectors pass. Real-provider r44 acceptance passes in 20.836 seconds. |

Authoritative final logs are `/tmp/nola-cutover-r44b-workspace-20261008.log`,
`/tmp/nola-r45-mac-full-20261008.log`, `/tmp/nola-r46-mac-recovery-20261008.log`,
`/tmp/nola-r46-ios-full-20261008.log`, `/tmp/nola-r44-android-kotlin-20261008.log`,
`/tmp/nola-r44-android-native-20261008.log`, `/tmp/nola-r46-web-tests-20261008.log`,
`/tmp/nola-r46-web-check-20261008.log` and `/tmp/nola-r44-real-submission-20261008.log`.
The final iOS xcresult reports `Passed`, no failures and no runtime warnings.

Boundaries are explicit: full durable baselines still exist for cold recovery,
bounded replay compaction and terminal persistence; they are not per-delta UI or
execution responses. Optional debug diagnostics may read a cold checkpoint when
entering the next prompt, not during streaming/redraw. Native MLX identity and
channel-security tests pass, but real MLX inference remains unverified because
weights are absent. Native isolated subagents without the authenticated Mac
channel are not advertised as executable; detached Native memory inference is
not claimed. The real-provider check observes durable attempt identity, not
provider-side exactly-once HTTP execution. These are not implied by green tests.

## Verification history (intermediate results below)

### R44 Rust workspace passed; client acceptance remains open

Latest client result: `/tmp/nola-r45-mac-full-20261008.log` exited 0 with
932 cases, 12 skipped and zero failures in 343 seconds. This includes the actual
permission-wait Host kill, whole-app cold reconstruction of the original Run,
resumed permission and exact seal, plus EOF-to-next-submission recovery. A final
review nevertheless found that the finish function registered observer cleanup
before validating the calling message; a stale call could retire a finalized
replacement Run's newer observer. That narrow boundary and its regression are
being corrected, so the preceding full run does not certify the final source yet.
That final Mac boundary was subsequently corrected and the four affected
recovery/observer/coordinator suites passed all 43 cases, exit 0
(`/tmp/nola-r46-mac-recovery-20261008.log`). Stale and duplicate completion calls
cannot retire a replacement observer even when its Run is already finalized.

iOS r45 verified the lifecycle fix, including all five former crash cases and
the new synchronous TaskLocal release case. Its sole remaining failure was
permission recovery's next model response: successor claims re-sampled the wall
clock instead of using the explicit operation time. Claims now use each begin,
accepted response or active submission's timestamp consistently; a contract
regression checks that sequence. Full simulator verification is running in
`/tmp/nola-r46-ios-full-20261008.log`. No lease/identity predicate was weakened.

The frozen Rust/CLI batch is under workspace verification with two compiler
jobs and no concurrent native build. The first compile found an incorrect `.0`
access on a `u64` in the new regression; it was corrected before resuming in
`/tmp/nola-cutover-r44b-workspace-20261008.log`. Compilation has completed and
the command subsequently exited 0 for the full workspace, including doc tests.
The earlier failed compile is not test evidence.
The CLI binary suite has now passed 1533 cases (zero failures, three ignored),
including Native channel identity, resume decoding, pre-send authorization,
redirect rejection and the unchanged 5,000-conversation latency gate. All six
canonical-stream integration cases and the real SQLite expired-lease cold-reopen
case passed. This is now a completed workspace result, not only selected targets.
The real PTY suite subsequently passed all 84 cases in 308.23 seconds. The
previously failing service connection-pressure and long-stream publication/RSS
cases also passed, as did the runtime heartbeat cases, without loosening lease
or performance limits. Mac full acceptance used isolated DerivedData but exited
65 before tests: Swift could not type-check the long expression in
`MacLocalAgentRuntime.swift` at line 2131 (`/tmp/nola-r44-mac-full-20261008.log`).
The expression is being split without changing its validation semantics.
After that correction, `/tmp/nola-r44b-mac-full-20261008.log` compiled and ran
932 cases: 12 skipped, four failed cases with 12 assertion failures (one
unexpected), exit 65. The cold-recovery fixture omitted deferred-tool discovery
and never reached permission waiting; journal parity still expected a window
invalidation for imported content. Two remote-permission/EOF recovery cases need
separate investigation, including whether the production observer misses journal
updates after transport EOF. Corrections are in progress; Mac acceptance is open.
Android JVM and AndroidTest Kotlin compilation passed in 37 seconds: all 93 XML
reports are from this run, totaling 358 passed, two skipped and zero failures.
Real Android JNI acceptance completed on the isolated emulator: nine cases
passed, exit 0 in 3m19s (`/tmp/nola-r44-android-native-20261008.log`), including
permission-wait cold reopen, exact original-Run reclaim and terminal seal checks.
iOS full simulator acceptance uses its existing isolated DerivedData. The first
launch exited 70 because the previous simulator was no longer available; the
retry uses the currently available iPhone 17 Pro
(`/tmp/nola-r44b-ios-full-20261008.log`). That run exited 65: 565 logical cases,
550 passed, six failed and nine skipped. Permission recovery reached the successor
claim, but its fixture then sent an earlier timestamp and correctly hit
`attempt time moved backwards`. Five other cases aborted during synchronous
TaskLocal/isolated deinitialization on the iOS 26 runtime (four data-source cases
and one UserTask case), not their persistence assertions. The fixture clock and
explicit nonisolated ARC-only teardown are being corrected together, with
lifecycle regression coverage; no test is skipped to conceal the failures.
Mac corrections also include a production
EOF-recovery gate: after an exact committed terminal seal and presentation drain,
the old observer must stop blocking the next submission without clearing a
replacement owner's gate. That correction is not yet reverified.

The freshly built binary also passed the real-provider submission/journal
acceptance in 20.836 seconds (`/tmp/nola-r44-real-submission-20261008.log`):
admission-before-journal, typed seal/manifest digests, exact sealed UTF-8 answer,
duplicate durable-attempt frontier, Host restart, multilingual follow-up and
cancel-before-late-post. Evidence is isolated under `nola-real-submission-cuk58khq`
in the system temporary directory. Physical provider HTTP calls were not observed;
this does not prove provider-side exactly-once execution or Native MLX inference.
It includes exact expired-delivery recovery, request-bound Native model
authorization, redirect rejection, imported-journal window suppression and
terminal invalidation coverage. The earlier r43 binary results do not certify
these edits.

Android now derives local completion from the exact source Store terminal seal,
not a diagnostic trace or a normally ended Flow. The completed event carries the
validated Run/initial-assistant/Core identity; UI finalization is also fenced by
the current coordinator. Failed/cancelled seals cannot become successful answers.
The obsolete `persist_checkpoint` Host branch that merely returned `persisted`
without writing anything has been removed. JVM and real JNI assertions were
added but have not been compiled or rerun yet. iOS remote journal presentation
now reuses owner-scoped window admission and consumes committed cursors directly
for ordinary deltas. New admission/owner/terminal updates still refresh structural
metadata. Its retry, view-switch, epoch and repeated-delta regressions have only
passed static parsing, not simulator acceptance.

Native model identity is distinct from its replaceable loopback channel. Each
physical request is checked against the Mac authority; native transport does
not follow redirects. Isolated native subagents lack the authenticated Mac
control channel, so their capability is explicitly unavailable rather than
silently downgraded to HTTP. Detached native memory inference is likewise not
claimed as supported. Local MLX weight files are absent; identity/transport
fixtures do not prove real native cold-Run inference.

### R43 platform closure in progress

Android's frozen-owner recovery corrections passed `:app:testDebugUnitTest` and
`:app:compileDebugAndroidTestKotlin` together (exit 0, 23 seconds,
`/tmp/nola-r43-android-kotlin-20261008.log`). This compiles the new real-JNI
permission-wait/reopen/journal/seal instrumentation case but does not execute it.
The isolated `nola_avd` emulator is booted as `emulator-5554`. The native library
built successfully, but instrumentation acceptance is not achieved. After two
fixture corrections (nullable initial cursor and the selected test class name),
the second run executed nine cases: seven passed and two failed
(`/tmp/nola-r43b-android-native-20261008.log`). Permission-wait cold recovery
claimed no request; another fixture attempted to persist an override before
admitting its conversation. Both require correction and rerunning; JVM success
does not supersede these failures.

iOS source now uses exact terminal seals and one scoped cold/hot journal
consumer. Its consolidated simulator run exited 65: 563 logical cases, 552
passed, two failed and nine skipped (`/tmp/nola-r43-ios-full-20261008.log`).
The real Store/HTTP parser cold-recovery case confirmed the same expired-delivery
empty-claim defect as Android. The digest contract fixture returned its old
response ordering, so a permission query received a claim response. Both are
being corrected together; no simulator acceptance is claimed.
Mac's first r43 attempt exited before compilation because a separate Xcode
process held the default DerivedData database. The test script now accepts
`NOLA_MAC_DERIVED_DATA` for an isolated build without interrupting that process.

Mac HTTP full-app cold recovery reconstructs the original Run from its frozen
nonsecret launch descriptor plus current credentials and authority. Native MLX
recovery additionally separates frozen model/profile/revision identity from
the replaceable loopback endpoint and token. The Mac batch is implemented but
not compiled; CLI validation is being connected at each physical request
boundary, including transport fallbacks, rather than only the outer model
attempt event. None of this is yet full-app native recovery acceptance.

The r43 Rust workspace run exited 101 with five failing targets (CLI binary,
PTY, committed runtime, database-service ownership and service process). CLI unit tests reported 1523
passed, one failed and three ignored: the 5,000-conversation latency assertion
took 1.77 seconds against its one-second limit during concurrent native builds.
An idle rerun of the same compiled CLI case passed at 438ms without changing the
threshold. The committed-runtime target had four lease-expiry failures under the
full run; its unchanged binary subsequently passed all 12 cases both serially
and with default test concurrency while native builds were idle. These support
load sensitivity, not a claim that every timing race is eliminated. The two
Service failures are being inspected separately. These binaries predate the
latest native-channel and precise expired-recovery edits.

The PTY suite subsequently completed with 83 passed and one failed. Retained
database evidence proves A1/B1/A2 process execution and lifecycle events were
ordered without overlap; the failed assertion used a claim timestamp sampled
before acquiring the write lock, not the process-start timestamp. The corrected
test requires actual process start/finish ordering, exact lifecycle event
ordering and the existing external output order. It is not yet rerun. Recovery
fixes distinguish expiry (checked against current state inside the write
transaction) from proven owner-process death; mobile clients must not fabricate
the latter to reclaim an expired permission delivery.

### R42 verification and native bridge closure

Workspace library tests and all 10 `run_cancellation_e2e` tests completed with
exit 0 (`/tmp/nola-cutover-r42-rust-20261008.log`). This includes the guarded
first Host response and exact hot/cold selective-state equality; r41's remaining
Rust assertion failure is now superseded. The library command does not include
CLI binary unit tests, which run separately in
`/tmp/nola-cutover-r42-cli-20261008.log`. macOS full acceptance logs to
`/tmp/nola-cutover-r42-mac-full-20261008.log`; it exited 65 after 930 XCTest cases,
12 skipped and 127 assertion failures (89 unexpected). Many failures originate
from the shared frozen-output fixture error below; this is not a passing run.

The CLI binary run subsequently exited 0: 1524 passed, 0 failed, 3 ignored.
It includes native bridge receipt preservation/rejection, real memory readback,
automation version checks, and symbolic-link identity evidence. Mac r42 revealed
three fixture errors corrected after its binary was built: the frozen output
preference falsely labelled 4096 tokens a product default (actual default 16384),
automation execution restarted report numbering at a hard-coded 3 after creation
had consumed more rounds, and Python/Rust spelled parentheses differently in
file URIs. These fixture corrections are not yet rerun.

Completion audit still found real platform gaps, so the goal remains open:
Android recovered the latest displayed assistant instead of the Run's initial
owner; iOS cold publication repeatedly loaded windows and used trace/EOF rather
than exact terminal seals; Mac full-app cold recovery depended on in-memory
launches. Android's owner fix is implemented but untested, iOS consolidation and
native recovery tests are being added, and Mac must reconstruct from the original
Run's nonsecret frozen launch facts plus currently valid credentials/permissions.
Same-process helper restart is not proof of full-app cold recovery.

The post-r42 batch also suppresses row-window invalidation for journal-only Host
content/progress updates and imported journal records when row count/title/root
are unchanged. Admission, lifecycle and control invalidations remain. A real
command-receipt stream regression checks that journal/head bytes advance while
the conversation-window cursor does not. This closes the native observer's
per-chunk bounded-page reload without reintroducing checkpoint reads. The new
policy and mobile/native recovery additions still await consolidated testing.

The native automation path is not the generic CLI adapter: the Rust device
bridge delegates to `MacAgentOrchestrationService`. Its production adapter now
returns the actual canonical upsert revision and independent readback check;
the bridge preserves and validates the Host-owned execution envelope instead
of dropping it. Tests distinguish typed evidence from business fields such as
`read_back_verified`, and preserve committed effects on failed verification.
The Mac E2E still executes the scheduled automation, in addition to checking
the canonical record and receipt version.

This receipt attests the automation definition, not cross-resource atomicity.
The pre-existing WebSource API can swallow persistence failures and does not
return a typed commit observation; that separate API is not repaired or claimed
as verified here. Generic CLI/Web `agent` schedule support also must not be
inferred from the native macOS automation test.

### R41 final results and parallel correction batch

Both consolidated processes have now exited. Rust workspace r41 exited 101 with
only `run_cancellation_e2e` failing on the obsolete control-presence assertion
described below; its corrected cold-equivalence assertion still needs rerunning.
macOS compiled and executed 930 XCTest cases, with 12 skipped and 42 assertion
failures (3 unexpected), exiting 65. Assertion count is not failed-test count.
No full macOS acceptance is claimed.

Three workers now divide canonical memory commit/readback receipts, automation
commit/readback receipts, and macOS fixture/product regressions. The integration
thread owns typed capability declarations and exact-version validation coverage.
Symbolic-link evidence now preserves the directory entry and hashes the link
itself; it does not follow the target or widen authorization. Host-owned writes
must return real committed-record versions and matching readback checks, using
the existing `MutateAndValidate` contract rather than bypassing Skill validation.
New regression coverage keeps unrelated and stale-version obligations open.
These corrections are not yet compiled or tested; acceptance follows the batch.

The macOS originating-intent observation assertion was stale: Store deliberately
routes exact observation through bound receipt acceptance, including cold
terminal discovery. The updated client test rejects changed owner/digest while
accepting the exact receipt idempotently; no production permission was relaxed.

### Consolidated acceptance started after r40 implementation

The r40d full workspace run finished with exit 101 and exactly two failed test
targets: `branch_transaction_e2e` (old reply contract assertion) and
`run_cancellation_e2e` (missing initial replay binding). CLI unit tests passed
1518 with 2 ignored, and all 84 PTY tests passed. Those results describe the
pre-correction binaries, not the newly added Service regressions or Store fix.
The corrected full workspace verification now logs to
`/tmp/nola-cutover-r41-workspace-20261008.log`; its final result is pending.

During r41, the branch regression passed. The first-response regression passed
the checkpoint-body read/write guard and reached an obsolete assertion requiring
control in an incremental reply. Its source now requires the compact reply and
queries HostExecutionState separately while the body remains guarded, then
compares the complete selective state after cold reopen. This assertion correction
is not yet rerun; r41 remains a failed run until superseded.

R41 has passed the new real-Service
`service_task_admission_rejected_receipt_recovers_after_process_restart` test.
Android `:app:compileDebugKotlin` completed with exit 0 in 18 seconds using one
worker and a 1536 MiB Gradle heap (`/tmp/nola-cutover-r41-android-compile-20261008.log`).
This does not validate native Android execution. JVM tests are now running via
`:app:testDebugUnitTest` (`/tmp/nola-cutover-r41-android-tests-20261008.log`); their
dependency graph contains no Rust/NDK build. That run completed with exit 0 in
18 seconds: XML reports 357 tests, 0 failures/errors and 2 skipped (355 executed
successfully), including the new invocation, journal merge and Core-session UI
tests. It is not a substitute for Android native instrumentation or Apple tests.

macOS full-layer acceptance was started with `CARGO_BUILD_JOBS=2
./scripts/test_mac_layers.sh full`, logging to
`/tmp/nola-cutover-r41-mac-full-20261008.log`. Its FFI dependency built, but no
client/test success is claimed until the process exits and results are inspected.
Rust workspace r41 remains live independently; neither run has been restarted
merely because a progress interval elapsed.

Real-model acceptance exited 0 in 33.521 seconds using the current built CLI:
`/tmp/nola-cutover-r41-real-model-20261008.log`, isolated evidence directory
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-real-submission-2dnjc5w4`.
It passed admission-before-journal, owner-bound terminal seal/manifest digests,
exact UTF-8 answer, duplicate unchanged durable attempt frontier, Host restart,
Chinese natural follow-up and cancel-before-late-post. The fixture removed its
private credential copy on exit. Physical provider HTTP calls were not observed:
this is not an external exactly-once proof. R41's 84 CLI PTY tests also all passed.
Apple/native and the corrected first-response cold-equivalence assertion remain
pending, so the overall cutover is not complete.

R41 also passed `committed_admission_timeout_recovers_the_original_run_across_service_restart`
through the real Service and delayed transport proxy. macOS full tests compiled
and started, but automation, file-organization and knowledge-memory Skill tests
have failed (final-answer observation and declared mutation-version evidence).
These failures are under source/fixture investigation; no Apple acceptance claim
is made and tests are not being weakened to match missing output.

The first workspace build stopped before tests: `admit_with_core` required the
whole Agent command repository family, while stateful runtime admission did not
declare its new direct creation/history dependencies. The helper now requires
only publication, Run creation, canonical history and Task ownership; the runtime
repository contract explicitly includes creation/history. No compatibility shim
or checkpoint reconstruction was restored.

Workspace attempts `/tmp/nola-cutover-r40-workspace-20261008.log` and
`/tmp/nola-cutover-r40b-workspace-20261008.log` exited 101 during compilation.
The next build (`/tmp/nola-cutover-r40c-workspace-20261008.log`) reached CLI and
found a remaining Task ACK consumer reading the deleted cumulative `goal` field.
It now uses the selective `has_running_goal` execution fact, preserving the
distinction between active and running Goals without restoring full Goal state.
That attempt also exited 101 before tests. The next full workspace run uses
`/tmp/nola-cutover-r40d-workspace-20261008.log`; its result must be read before any
acceptance claim. Native/mobile and real-provider acceptance remain pending.
The “not tested” statements below describe the implementation handoff, not a
successful validation of the current tree.

The r40d workspace attempt has completed compilation and entered tests; do not
infer its final outcome from that milestone. Web `npm test` exited 0 with 264
passing tests (`/tmp/nola-cutover-r40-web-tests-20261008.log`), and `npm run check`
exited 0 with zero errors/warnings (`/tmp/nola-cutover-r40-web-check-20261008.log`).
The independent JavaScript journal vectors and trace journal recovery script also
exited 0. These are protocol/frontend checks, not native or real-model acceptance.

Coverage audit still requires real Service admission unknown-result recovery and
Task import definite-rejection/reconnect retry. Existing SQLite recovery tests use
InProcess, while the new receipt-chain tests cover only request-ID selection;
neither alone proves those Service vertical paths.

During r40d, the 10 MB branch test failed because it still expected bounded prompt
text in the now-thin admission reply. Its source assertions now require no
checkpoint/snapshot or initial items in that reply, while retaining the existing
committed Host request assertions for the actual excerpt, original-size metadata
and payload bound. This correction has not been rerun; r40d continues using the
already-built test binary and remains a failing acceptance run until superseded.

The first-Host-response body-guard regression also failed with a real hot-path
write: initial admission wrote the checkpoint but did not bind a replay base to
the new Run, so ExecutionStarted fell back to a second full baseline. Store now
binds the initial baseline, exact header/witness and zero replay frontier inside
ordinary/internal/maintenance admission transactions. The regression still forbids
both reads and writes through its checkpoint-body view, and additionally checks
the admission frontier and cold reconstruction after the first response. This
implementation awaits the next consolidated run; the guard was not weakened.

Added (not yet run) real-Service coverage uses a delayed receipt proxy for actual
BeginAssistantTurn, reconciles its typed unknown result without resubmission, then
checks exact original receipt replay after service restart. A separate CLI test
launches production Service in a child process, injects a SQLite Task import
failure, and proves rejected-receipt successor admission across process restart.

### r40 mobile recovery closure and incremental Android display — not tested

Follow-up source inspection confirms Prepared Task recovery is already wired at
session reopen and exact dispatch retry; the older missing-path audit is stale.
The remaining service-specific defect was fixed separately: an explicitly rejected
admission receipt now permits a deterministic bounded successor request, while
pending/unknown/committed outcomes retain their original request identity. New
sessions do not scan for nonexistent prior source commits. Android controls now
precede Task/Skill events, and a changed Core identity resets only its old local
Task/Skill projections. These additions also await consolidated acceptance.

Parallel implementation connected Android ordinary admission to the actual model's
HostInvocationBinding v7 and Core output admission. Permission recovery reconstructs
the frozen provider from currently authorized profiles/installed models before any
claim, renews the concrete tool plan for the committed Core session, and rechecks
the exact Run witness. The UI no longer captures today's selected model as a
prerequisite to recovery. Local llama clients fence model and generation parameters
under the same execution mutex; a matching model ID alone cannot reuse mismatched
parameters. Native `llama_cpp` is an explicit adapter, not an MLX alias.

iOS and Android cold recovery now restore control and Tasks as well as permission
requests. `load_conversation_task_projection_v1` is a separate read-only cold query:
it returns only Core session, checkpoint revision/cursor and Task projection. Each
client checks that witness against its frozen Run and rereads the exact proof
before claim. Task state is not added to hot HostExecutionState replies. iOS also
checks cancelled/revoked execution ownership during preparation.

Android local journal notifications now use the existing cursor-based Part reader;
the bounded conversation window is read only when message admission is missing.
Projected admission metadata is cached rather than reconstructed per delta. Local
and remote merges preserve paged history and unrelated newer rows, inserting only
missing admitted messages in canonical order. UI delivery fences the conversation
and coordinator separately from the Core session identity. Recovery no longer
fabricates an assistant message ID from the Run ID.

Added regression definitions cover cold Task query shape/witnesses, cancellation,
foreign owners, native adapter identity, frozen model parameters, and incremental
message merge ordering/history. Static diff and call-site reviews only: **no r29–r40
compilation or tests have run**. Full Rust/mobile/provider acceptance, validation of
cross-store Prepared Task imports, and remaining platform integration are not
claimed complete. Earlier test totals do not validate this working tree.

### r39 wire iOS production cold model reconstruction — not tested

ChatManager's local permission recovery now reads the exact frozen invocation,
reconstructs the provider from admitted generation/output values, and uses the
committed Core session independently of the conversation route. HTTP secrets are
obtained from currently enabled trusted local profiles, then the full non-secret
route (including credential/query digests) and generation must match. Native
models resolve the exact admitted catalog ID. No selected-model fallback, current
generation defaults or plaintext configuration persistence is used. This uses the
protocol's allowed independent verification of ephemeral sources; it does not
pretend they became managed handles.

The real coordinator now receives a recovery callback, rather than invoking the
required gate without one. It reconstructs again for live credential authorization,
renews the tool plan, checks execution ownership and compares the complete binding.
The Application repeats authorization after waiting and rereads the exact proof
before claiming. Recovery stream termination requests cancellation. Policy hashes
now include admitted model capability facts and installed Skill snapshot identity;
changes fail closed rather than silently replacing Run dependencies.

Generated scalar representations are normalized through the wire codec before
comparison, preserving large integer seeds. Added real isolated-profile regression
definitions change stored generation defaults, verify frozen reconstruction, then
delete authorization and require rejection. None has run. All r29–r39 code remains
uncompiled/untested per the requested batch workflow. Android admission/recovery
parity, cold Task projection/control restoration, remaining recovery ownership and
the consolidated native/provider acceptance are still incomplete.

### r38 connect iOS prepared model binding to ordinary admission — not tested

IOSPreparedRunModel now exports non-secret HostInvocationBinding model facts from
its actual HTTP/native execution source and the existing Core output admission.
ChatManager passes this prepared value into the real DeviceAgentCoordinator;
makeRuntimeConfig writes host_invocation into turn_identity, hashing the concrete
Host catalog, enabled schemas, permission rules and context budget used at that
admission. No second persistence store or model-setting lookup is introduced.

Captured generation includes exact integer seed, stop strings, sampling,
thinking/cache controls and output tokens. Credential/query identities remain
the existing output-route hashes/provenance, never plaintext configuration.
Regression definitions check large integer preservation, no API key in encoded
bindings, stable policy hashing and changed-policy identity. Not executed.

This connects ordinary iOS prepared Run admission, but does not complete cold
reconstruction: route secret sources are still ephemeral, managed credential
reference resolution/current revocation and production recovery preparation remain
open, as does Android parity. Other coordinator construction paths are not claimed
durably bound. The iOS recovery gate remains enforced rather than bypassed.
No r29–r38 compilation/tests have run; static diff/call-site checks only.

### r37 bind live native Agent clients to one provider — not tested

TextAgentModelClient now captures ModelService's actual provider at construction
and derives capability declarations from that same instance. Previously it could
retain the mutable selector while reading its capability surface, then dispatch
later steps through a replacement provider. ModelService provider installation
and capture are now synchronized. A regression definition changes the selector
after Agent construction and checks that only a new Agent uses the replacement.

This is a connected hot-execution correction, NOT durable cold recovery. Source
inspection confirms IOSPreparedRunModel already has exact in-memory HTTP/native
provenance and Core-authored output admission, but explicitly is not a persisted
Host binding. Native DeviceAgentCoordinator does not yet put host_invocation into
turn_identity. Next work must serialize non-secret admitted route/generation and
policy identity from that actual source, retain authorized secret references, and
wire reconstruction into production recovery. Current settings or plaintext
provider configuration must not be serialized as a substitute.

No r29–r37 compile/tests have run. Only static source/diff checks. The whole goal,
especially mobile durable admission/reconstruction and broad acceptance, remains
incomplete; there are no live build/test processes.

### r36 cold permission identity fix and production wiring audit — not tested

iOS permission recovery now validates claimed Host requests against the exact
Run's committed Core session ID, not the conversation route ID. Missing committed
Core identity fails before preparation/claim. Recovery rereads the exact frozen
invocation/checkpoint witness after the effect-work wait and before claiming;
it refuses a changed proof. Regression definitions use intentionally different
conversation/Core identities and a proof changed on the final pre-claim read.

The production cold-recovery audit exposed an actual unconnected vertical path,
not merely missing tests: DeviceAgentCoordinator calls
IOSCanonicalAgentApplication.recoverPendingPermissionWait without its required
prepareRecovery callback. ChatManager constructs that coordinator from current
LLM settings, current skills/tool plan and current model/context parameters.
The Application gate therefore rejects the production attempt; only fixtures
supply the callback. Android ChatViewModel/DeviceAgentCoordinator likewise build
from current configuration, and AndroidCanonicalAgentRuntime has no exact frozen
invocation reconstruction gate. Neither path is accepted as complete.

Next implementation must freeze native Host dependencies at admission and rebuild
the actual provider/catalog/authority from that admitted binding at cold recovery,
then restore Task display/control ownership through canonical sources. Do not
silence the iOS guard with a no-op callback or equate route and Core identities.
No r29–r36 builds/tests have run. Source/diff checks only; no process is live.

### r35 remove mobile cumulative transition consumers — not tested

iOS and Android DeviceAgentCoordinator no longer fall back from control to a
legacy snapshot. They reject cumulative transition fields, require an independent
permission projection matching the control revision, and validate/decode it
before emitting journal hints or processing Host deliveries. The UI Snapshot
event remains a scalar client display type, not a Core snapshot transport.
iOS no longer restores a whole Task projection from execution replies; its
existing committed Task event reducer remains the live Task update path.

Swift Host-response/active-submission processing now validates the transition and
exact permission owner before claiming successor work, matching Android. The
previous implementation evaluated the claim argument before entering validation.
Updated regression definitions check rejection of snapshot/checkpoint fields,
missing/mismatched permissions, and no successor claim after a foreign permission
read. An Android cursor fixture no longer supplies the retired snapshot field.

No r29–r35 builds or tests have run. Only static call-site/diff inspection has
run. Journal authenticity/replay remains owned by existing committed receivers;
this batch changes their execution-consumer boundary, not their wire identities.
Cross-client cold Task recovery/publication and consolidated native/provider
acceptance remain to be closed. No running build/test handle exists.

### r34 typed admission failures and retry fences — not tested

CLI ordinary/Task admission now retains `ApplicationServiceCommandError` through
the Application port and SessionStore instead of collapsing it to text before
Host adoption. Ephemeral admission also uses the existing receipt transaction.
The Host only fences immediately before dispatch: inactive Task authority does
not fence an operation that was never sent. A confirmed rejection/conflict or
pre-send failure releases that operation's fence; success remains fenced until
execution-state adoption, and pending/unknown outcomes remain fenced. This fixes
the first-admission rejection path that otherwise tried to recover a checkpoint
that never existed. No absence query or error-message heuristic proves rollback.

Service-client inspection also found committed-result decoding errors incorrectly
tagged as pre-send transport failures. Initial decoding and receipt reconciliation
now share a decoder that preserves the original request identity and requires
reconciliation/reopen on malformed committed results. The in-process receipt
path follows the same rule and preserves structured rejection details. Admission
diagnostic logging uses the typed outcome directly.

Added regression definitions cover real ephemeral first-admission rejection,
retry, adoption fencing and uncertain-result refusal, plus malformed committed
result classification. None has been run. No r29–r34 compilation or tests have
run; static call-site/diff checks only. Initial lost acknowledgements still
require exact original-receipt reconciliation; this change does not authorize
reissuing them with a new request ID. Cross-client incremental-publication and
recovery closure, followed by the consolidated acceptance cycle, remains open.

### r33 retain admitted Core and reject malformed client replies — not tested

Stateful ordinary and Task admission now transfer the reduced Core into the
existing RuntimeSupervisor transaction overlay. The baseline is serialized from
a borrowed checkpoint view; the Core is neither consumed nor restored from the
reply. The supervisor adopts it only after the Store commit. Both command paths
share the same Task authority gate and binding, inside the publication transaction;
stateless command dispatch uses that gate and drops the resulting Core. Runtime
checks the admitted Run/digest against Store facts and uses its committed event
sequence/revision, rather than inventing a second execution owner or cache.

Added, but NOT run, regressions prohibit checkpoint-body reads for the first Host
response after stateful admission and reject retained state after admission
receipt rollback. The previous r32 first-response cold-recovery limitation is
therefore addressed in implementation, not yet proven by acceptance. Explicit
cold recovery and maintenance reads remain separate paths.

Android now rejects missing/malformed event and trace arrays instead of silently
substituting empty arrays. It parses journal publications before claiming work.
Swift malformed-admission tests assert that legacy fields, missing event arrays
and missing/null/mismatched permission versions never claim Host deliveries;
Android envelope regressions cover retired fields and invalid arrays. These are
pending regression definitions, not passing evidence.

No r29–r33 build or tests have run. Only source/call-site and diff-whitespace
inspection has run. Failed initial-admission recovery in the CLI, cross-client
publication/identity closure and consolidated native/provider acceptance remain
open; the overall goal is not achieved. No test/build process is live.

### r32 admission, maintenance and revert replies — implementation, tests deferred

Initial admission, maintenance admission and conversation revert now return
`execution: HostExecutionState` instead of checkpoint/snapshot. Admission uses
Core.reduce_at rather than constructing a cumulative transition snapshot; full
checkpoint persistence remains at the durable baseline boundary. CLI adopts
the selected execution state directly. Its full-checkpoint adoption helper and
snapshot presenter method are deleted; explicit cold restoration remains.

iOS and Android admission consume execution.control and execution.permissions;
subsequent controls use the same control/permission contract. Both adapters
reject cumulative reply fields and missing/mismatched permission revisions.
Admission validation occurs before claiming Host deliveries. Rust and Swift
fixtures are migrated to the new contract; tests needing full durable content
read the Store explicitly rather than relying on execution acknowledgements.

No r29–r32 compilation or tests have run. Static call-site inspection is not
acceptance. Remaining closure includes malformed/recovery boundary regression
coverage, failed-admission retry ownership, cross-client identity/publication
inspection and then one consolidated compilation/test/native/provider cycle.
Background maintenance cold reads and first-Host-response cold recovery remain
explicit performance limitations; this batch does not claim to remove them.

### r31 idle reply and terminal ownership cutover — implementation, tests deferred

`AcceptedIdleSubmission` now returns journal cursors/events/traces plus the
selective execution view, not checkpoint/snapshot. The idle reducer no longer
calls Core.snapshot before persistence. CLI idle controls, session lifecycle,
and Task guidance terminal recovery adopt this execution result directly.
Stateless/Runtime import acknowledgements use its control revision. Goal service
fixtures read the actual persisted Goal projection instead of expecting it in a
control acknowledgement. The unused Runtime Driver `CoreHostOutput::Snapshot`
variant and consumer arm are deleted; initial admission/revert snapshot paths
remain explicit and still need closure.

The two terminal-status paths and terminal Message helper no longer synthesize
successor ownership as `{turn_id}:assistant`. They resolve this transaction's
explicit TurnStarted binding or the persisted Core-Turn owner, reject a foreign
Run/conversation or conflicting birth, and fail if no owner exists. Initial Run
ownership still comes from its admitted assistant Message. Regression assertions
cover opaque birth IDs and refusal to manufacture an unknown Turn owner.

No r29/r30/r31 compilation or tests have run. Only call-site inspection and
diff-whitespace checks have been performed; no process handle is live. Continue
the overall admission/revert/cross-client and identity/publication audit before
the requested consolidated acceptance. None of the historical green test runs
below validates the current worktree.

### r30 CLI execution state cutover — implementation in progress, not tested

The current worktree removes `CommittedCoreRuntime`'s retained checkpoint and
AgentCoreSnapshot fields. CLI execution finalization, active control adoption,
and Task boundary imports now consume `LoadHostExecutionState`. The selected
Core view contains control, committed configuration/permission state, pending
effect identities, pending guidance IDs, admission readiness and cumulative
budget facts. It excludes transcript, model context, assistant projection,
memory records, and full Goal/Task/Skill projections. It is extracted directly
from borrowed live Thread state: invoking checkpoint_projection_view here would
silently clone the entire terminal Thread, so this route does not use it.

RuntimeSupervisor serves exact-revision committed active state and retains only
one selective terminal view after commit, not the terminal Core/transcript.
Terminal permission reads share that retained exact Run owner. Cold misses use
the canonical recovery sequence; Store revision is checked again before reply.
SessionStore checks canonical Run/digest ownership before and after adoption.
CLI adoption validates session/catalog/permission/effect consistency, prepares
the native catalog before replacement, and keeps dispatch fenced on failure.

The obsolete `CommittedRuntimeApplicationPort::load_core_checkpoint` method
and all its implementations are removed. Budget enforcement, effect resumption,
invocation binding, Goal controls and successor admission use the selected
state. Background memory maintenance uses explicit Store/Application reads;
it no longer forces memory or full snapshots into every foreground execution.
Its explicit maintenance checkpoint reads remain a separate optimization item.

Regression fixtures are being migrated to execution/control assertions; SQLite
body-read guards now cover active and terminal selective adoption, cold/hot
parity and terminal permission reads. These new assertions have NOT been run.
Per the user's instruction, no compilation or tests have run for r29/r30.
No handle is live. Remaining work includes static call-site/ownership audit,
admission/idle full-result paths and cross-client contract closure, then one
consolidated build/test/native/provider acceptance sequence. Do not report the
old r28 green run as validation of this current worktree.

### r29 active submission contract cutover — implementation in progress, tests deferred

User explicitly requested finishing the overall implementation before another
compile/test batch. No compilation or tests have run for r29; the r28 results
below are historical evidence only and do not validate the current worktree.

`AcceptedActiveSubmission` now contains commit/events/traces/control only.
Its checkpoint/snapshot fields and the internal full-result materializer are
deleted, not retained behind a compatibility switch. Runtime and stateless
dispatch use this same contract, including active Task guidance. CLI boundary
submission processing no longer receives/discards whole recovery/display state;
SessionStore updates its exact control/permission notifications at that boundary.
iOS and Android active submissions use their existing control-plus-permission
read path, matching Host response processing. Affected contract fixtures are
being migrated without running them yet.

At the r29 boundary the CLI's legacy full execution projection was NOT yet removed. Its explicit
`commit_core_submission` consumer currently performs an explicit recovery read
after the thin reply, and execution finalization still loads a full checkpoint.
This is a visible remaining dependency, not claimed as a hot-path completion.
Those consumers and their state ownership must be replaced before consolidated
acceptance. Dispatch is fenced before control/idle submissions so a committed
operation followed by a read/adoption failure cannot execute from stale state.
No build/test handle is live.

### r28 permission read cutover — focused acceptance terminal

The existing `LoadConversationPermissionProjectionV1` command now consumes
selective committed permission state from the retained Runtime when Run,
conversation and Store revision match and no deferred events remain. StorePort
permission reads no longer take a full `StoredCheckpoint`. The Store pins its
read snapshot, checks the supplied revision/cursor against the current header,
and lazily reads the body only for cold baseline reads. Both paths share the
same original Turn admission and permission control-witness validator.

The real SQLite regression makes checkpoint body reads fail, then exercises
both CurrentRun and Control queries through ApplicationRuntime. It compares
against an independent cold recovery and rejects a stale permission revision.
Store coverage rejects foreign conversation, stale revision and wrong cursor;
existing malformed baseline, ownership and remote-source regressions remain.
This hot fixture has empty pending permissions; it is not a new real-provider
permission-dialog acceptance claim.

r28a (84732) exited 101 because the extracted validator initially required the
whole conversation repository family. Its bound now names only the actual
ConversationQueryRepository dependency. r28b (44613) exited 0 with 295 tests.
r28c (72977) exited 0 after stronger cold/hot parity and stale-control checks:
280 library, 8 permission, 7 cancellation and 20 turn transaction tests passed
(315 total). Log: `/tmp/nola-cutover-r28c-permission-read-20261008.log`.
No test remains running. This does not close the remaining full CLI acceptance,
execution-finalization restore, or native/provider acceptance requirements.

### r27 internal control commit cutover — focused acceptance terminal

The shared active-submission reducer now commits from a borrowed checkpoint
view and returns `CommittedActiveSubmission` (commit/events/traces/control),
not an eagerly cloned checkpoint and cumulative display snapshot. The obsolete
`transition_checkpoint` cloning helper is deleted. Actual Task guidance import,
Task terminal import and Run cancellation consume revision/status directly.
The existing full public acceptance is materialized only at its remaining
explicit consumers; this is an internal result separation, not a second reducer
or a new unconnected public query.

Cancellation preparation uses Supervisor control only when Run, conversation
and transaction-observed checkpoint revision all match. A miss retains exact
cold recovery; no stale hot state or caller-provided status authorizes control.
The hot cancellation regression replaces the body table with a read-failing
view, verifies that the guard rejects body reads, then requires a committed
cancellation without a body update and checks the recovered current state.

r27a (77227) terminated at compile with a missed direct cancellation call site.
That stateless entry now supplies no hot control and consumes the same lightweight
commit outcome, rather than materializing a full acceptance only to discard it.
r27b is terminal (69459, exit 0), log
`/tmp/nola-cutover-r27b-control-commit-20261008.log`: library units 280,
cancellation 7, task Run binding 14, Task terminal receipts 9, turn transactions
20; all 330 passed. The new real SQLite body-read/body-write guard passed,
alongside exact current-state recovery, stale peer fencing and terminal import
receipt regressions. No build/test remains running. This is focused acceptance,
not a fresh full workspace/native/provider pass. CLI full acceptance/restore, idle results,
memory queries and configuration payload hashing still require further work;
this batch does not claim all hot checkpoint dependencies are removed.

### r26 correction acceptance terminal; next hot-path cutover open

r26e is terminal (87284, exit 0): cancellation 6/6, task Run binding 14/14,
desktop Host 8/8. r26f direct desktop-binary repetition is terminal (4339,
exit 0), log `/tmp/nola-cutover-r26f-host-repeat-20261008.log`: five consecutive
8/8 runs, 40 passes. All failed targets from r25c have now passed focused
correction acceptance. This does not claim a new full-workspace invocation,
native Swift build or real-provider pass. No build/test remains live.

Next required implementation work remains the actual full-checkpoint consumers:
`AcceptedActiveSubmission` / idle receipts, CLI `commit_core_submission`,
`execute_committed_application_port` and their memory/Goal/configuration readers.
Do not reconstruct a supposedly bounded view by loading a full checkpoint behind
another query. The unified recovery log removes the forced active-submission
baseline write, not these remaining snapshot/restore dependencies.

r25c full workspace acceptance is terminal (handle 4129, exit 101). Exactly
three targets failed: cancellation 5/1, task Run binding 13/1, desktop Host 5/2.
CLI units passed 1516/0/2 ignored; PTY passed 84/84 (347.87s), including the
mixed-project pressure correction. Other workspace targets finished without a
reported failure. This is not a full green acceptance.

The cancellation fixture now reads the production materialized current state.
The guidance eviction fixture materializes the current checkpoint before
clearing dedup IDs and establishes the exact consumed input prefix; it no longer
combines an old body with a newer header. Production recovery checks are intact.

Desktop permission failure retained its isolated DB at
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/.tmpBvIht8/cancel.db`.
Its actual terminal evidence is `provider_failed` before a permission challenge,
not an observed permission UI ordering failure. The activation fixture observed
2 model requests where 3 were required. Transport root cause is not yet proven.
Both fixtures now enable isolated provider diagnostics and retain those logs on
failure; no timeout, expected request count or permission assertion is relaxed.

r26a is terminal (87234, exit 101): cancellation 6/6 and task binding 14/14
passed; desktop Host 6/7 still failed. Its isolated provider log captured a
connection closed before the request/response completed. r26b instrumentation
passed 7/7, but direct repeat r26c reproduced failure on its second iteration:
the fixture accepted a socket and immediately read WouldBlock with zero bytes,
then closed it. The repeat shell used `break` and returned zero; its failed
test result, not that shell exit code, is authoritative.

r26d confirmed accepted sockets have O_NONBLOCK on this Mac. The fixture now
explicitly switches accepted streams to blocking mode before its existing read
timeout; no timeout or production provider retry policy was widened. Original
desktop cases passed 7/7. A new fragmented-request regression initially failed
because its own nonblocking listener's accept needed readiness handling; that
fixture setup now waits boundedly for accept, as the original server does.

Consolidated correction acceptance ran as handle 87284, log
`/tmp/nola-cutover-r26e-recovery-host-20261008.log`, selecting cancellation,
task Run binding and desktop Host; its terminal result is recorded above.

### r25 unified reducer recovery — historical acceptance evidence

Active submissions now record their exact typed input, pre-reduction session
identity and decision clock in the same ordinal/byte frontier as Host responses.
The existing replay-order table admits either a Host inbox reference or a
submission owned by the committing transition, never a fabricated Host event.
Schema 60 preserves existing v59 Host ordering through a transactional upgrade.
Warm boundary code already flushes deferred Host inputs before the submission;
cold recovery now consumes both kinds in order, checks session/clock and forbids
Host responses disguised as direct submissions. It does not dispatch or publish.

Nonterminal active submissions use replay writes within the existing combined
4095-record/4-MiB limits; terminal and budget boundaries retain full baselines.
The accepted-submission return DTO and CLI post-execution restore still carry
full checkpoint/snapshot state and remain unfinished work.

First validation r25a (handle 3256) terminated with exit 101: Application units
280, history 2 and Run/Effect 5 passed; turn transactions passed 18 and failed 2.
Exact cold-state assertions exposed pre-submission cold rebindings missing from
the recorded facts (pending request resume cursor and dispatched/streaming state).
Submission facts now retain the actual minimal binding overlays applied before
reduction; recovery applies them before the recorded submission, without loading
copied request bodies. The comparison also uses identical JSON serialization for
f32 fields instead of comparing string serialization with `to_value` promotion.

Consolidated Store/Application validation r25b is terminal (handle 97275, exit 0),
log `/tmp/nola-cutover-r25b-unified-replay-20261008.log`: Application units 280,
history 2, Run/Effect 5, turn transactions 20, Store units 359 all passed (666
total, one explicit disk performance benchmark ignored). Exact cold submission
state, corruption rejection, mixed interrupt history, migration and atomic
rollback assertions passed. This is not full workspace/native/provider acceptance.
Regressions include queued NextTurn baseline-byte stability and exact cold state,
mixed Host/interrupt replay, corrupted submission identity/clock, Host bypass,
duplicate transaction identity, outer rollback and v59 order preservation.
Sources are frozen during this acceptance run.

Full workspace acceptance r25c is now live as handle 4129, log
`/tmp/nola-cutover-r25c-workspace-20261008.log`, command
`cargo test --workspace --locked --no-fail-fast` through the shared build-cache
wrapper. r25b is terminal; do not poll it or start a competing artifact build.

r25c partial evidence: compilation completed in 4m23s. The run remains live.
Two Application fixture failures have been localized, not corrected during the
source freeze: `run_cancellation_e2e` inspects the physical baseline and expects
the newer Cancelling state after an incremental interruption; its preceding
late-response fencing assertions pass. `task_run_binding_e2e` edits the older
baseline to evict accepted operation IDs, invalidating its replay prefix while
leaving the newer event cursor in the header. That eviction fixture must first
materialize the current frontier and deliberately establish the replacement
baseline; the production cold reader correctly rejects the inconsistent cursor.
Do not weaken current-state/cursor validation or silently ignore replay history.
CLI and remaining workspace acceptance are not yet terminal.

### r23 acceptance terminal; next cutover batch open

r24 removes the active-submission warm path's eager materialization of frozen
Host request bodies for settlement inventory. `RunEffectRepository` now requires
`pending_outbox_effect_ids`; the SQLite adapter selects only pending effect IDs
on the caller's connection, while cold restore retains full committed-request
validation and rebinding. The in-memory repository adapters implement the same
contract without a full-body fallback. Store regression assertions cover foreign
Run isolation, empty-ID rejection, visibility within the outer transaction,
settlement removal and rollback. These changes do not remove active-submission
checkpoint serialization or the CLI's post-execution full restore; both remain
open.

r24a validation is terminal (handle 1858, exit 0), log
`/tmp/nola-cutover-r24a-store-application-20261008.log`. The consolidated
Store/Application run passed 665 tests: Application units 280, historical
projection adapter 2, Run/Effect adapter 5, turn transactions 20, Store units
358; one explicit disk performance benchmark remains ignored. This verifies the
new repository contract and the selected transactional regressions, not removal
of all hot checkpoints, native parity, real-provider acceptance or the pending
PTY pressure fixture correction. No build/test is left running by this batch.

The next checkpoint cutover must address the actual persistence constraint:
`commit_transition` currently requires Host inbox reducer evidence for replay,
and replay ordering is joined to Host inbox rows. Active submissions cannot
be made incremental by merely selecting `CheckpointWrite::Replay`; their exact
typed reducer inputs need the same ordered, transactional recovery ledger first.
Do not fabricate Host events for user submissions or remove their baseline
before cold recovery can replay them.

r23a handle 66000 is terminal (exit 101). Production ownership 8/8, turn
transactions 20/20, work-step transactions 5/5, Core lifecycle fixture 2/2,
Runtime Driver integration 12/12, CLI unit 1516/0/2 ignored all passed. PTY
finished 83/84; its only failure was mixed-project pressure's timestamp-based
3-slot assertion. No artifact build/test is currently live.

The retained pressure DB shows a shell claim timestamp 1791449422612 preceding
the previous shell's end timestamp 1791449422613, but its committed started event
is later (1791449422628). Caller-sampled times do not establish transaction order.
The same database's ordered lifecycle event IDs yield maximum occupancy 3 and
minimum 0. The fixture now derives intervals from committed started/terminal
event IDs and requires exactly one of each per task; task counts, identities,
terminal receipts, project/global bounds and no-replay artifact assertions stay.
That test correction is not yet rerun. This evidence concerns lifecycle order,
not a proof that lease expiry physically stops an external process.

### r23 ordering correction — historical batch observations

r22a process 89000 is terminal with exit 101. Six targets failed: Application
production ownership (7/1), turn transactions (18/2), CLI unit (1513/3/2 ignored),
PTY (82/2), Core lifecycle fixture (1/1), Runtime Driver integration (11/1).
Other workspace targets passed, including new concurrent observed-work recovery.
The failures trace to premature successor message creation, the old no-owner
assertion and the shared fixture's missing explicit TurnStarted message field.

r23 stages the accepted Turn birth until its InputApplied users are ensured;
before a Part, next Turn, or transaction end it flushes an otherwise empty birth.
Both active-submission and Host-response paths use the same flush helper. The
binding and message still commit atomically before dispatching successor work.
The production ownership test now requires a birth owner and its identity after
the first body, while retaining no-Part-before-body and unaccepted-Turn rejection.
Both shared fixture TurnStarted records carry their explicit Core message IDs.
These corrections are not yet validated; full checkpoint cutover remains open.
Focused consolidated acceptance is now running under handle 66000, log
`/tmp/nola-cutover-r23a-focused-tests-20261008.log`: workspace selection of
production ownership, turn transactions, work-step transactions, Core lifecycle
fixture, committed Runtime Driver, full PTY and CLI unit targets. Production and
test sources are frozen during this batch; do not start another artifact build.

r23a partial verified results: compile completed in 1m17s. Production ownership
passed 8/8, turn transactions 20/20, and work-step transactions 5/5. The cold
successor failure-before-first-body test now reaches and passes its publication,
owner and failed-Attempt assertions; concurrent recovery also passed. CLI unit
and subsequent selected targets are still running under 66000. This is not a
full workspace, native or real-provider pass, nor completion of checkpoint work.

Further r23a evidence: CLI units passed 1516/0/2 ignored. Both ordinary queued
follow-up and durable steering after process loss passed, as did Goal composer.
One mixed-project pressure/worker-kill test failed; its final assertion output
is still pending. Read-only inspection of
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-pty-cross-project-mixed-pressure-kill-e2e-6455-1791449378507426000/shared-nola.db`
shows 6 Agent tasks completed, 5 Shell tasks completed and the killed Shell
failed with reconciliation_required and safe_to_retry=false. Do not infer the
failure cause from the test name or rewrite these terminal outcomes. Process
66000 remains live; retain the batch freeze until its terminal result.

### r22 Turn birth and recovery correction — historical acceptance observations

The batch is frozen for `cargo test --workspace --locked --no-fail-fast` through
the shared cache helper, process handle 89000, log
`/tmp/nola-cutover-r22a-rust-tests-20261008.log`.
No r22 pass is claimed. The Goal mock now accepts either legal ordering of
durable NextTurn follow-up and Goal audit; the original failed request's final
messages were original user, original assistant answer, then the exact follow-up
user input. The mock still requires evidence read, Goal completion, one follow-up
answer, and exactly four total model requests. It no longer requires audit to
precede a follow-up that reached Core before the Goal loop scheduled its audit.

Partial r22a evidence: compilation finished successfully in 5m01s. The production
Turn-binding target has one failing old-contract assertion at
`assistant_turn_binding_production_e2e.rs:689`: it expects no successor Turn owner
before any Part. The new accepted TurnStarted fact deliberately establishes that
owner at birth. After this frozen run ends, replace that assertion with exact
birth-owner identity and unchanged ownership after first body/cold reopen;
retain the separate assertion that an unmaterialized Part has no Part owner.
Do not remove the rejection assertion for an unaccepted Turn at line 831.
Other test targets are still running; new regression outcomes are not yet known.

Further r22a evidence: the new two-connection `competing_recovery` test passed.
Both queued-successor transaction tests fail earlier at message ordering:
message index 3 is user, not assistant. This is a production regression from
ensuring the assistant immediately on TurnStarted, which precedes InputApplied.
Preserve user-before-assistant ordering while recording the birth binding in
the same transaction: stage birth ownership until that Turn's InputApplied is
processed, or flush before the next non-input event/end when no input exists.
Do not weaken the ordered conversation assertions. Failure-before-body
publication is still unverified because its fixture stops at this earlier check.
Several CLI presenter tests also failed; final diagnostics are still pending.

CLI unit target finished 1513 passed / 3 failed / 2 ignored. All three failures
decode the shared `core/tests/fixtures/canonical_part_lifecycle_v1.json` and
report missing `assistant_message_id` on TurnStarted. That sample contains two
TurnStarted events and is also consumed by Swift/Web tests; update its explicit
birth identities consistently with the Core fixture generator, not a serde
default or client fallback. The PTY target is now running under the same 89000
process. No additional build was started and production/test sources remain
frozen during this acceptance run.

r22a PTY target finished 82 passed / 2 failed (346.18 seconds). Goal composer,
fair queue, mixed Host takeover and Goal/Skill restart passed. Failures are
ordinary queued follow-up and durable steering after process loss. The retained
queued-follow-up DB at
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-pty-queued-follow-up-73155-1791448382605445000/nola-e2e.db`
confirms canonical ordinals 1=user, 2=assistant, 3=successor assistant,
4=follow-up user. This is the same newly introduced message-creation ordering
defect, not justification to change UI sorting or weaken ordered-message tests.
The workspace process continues beyond PTY; its final status is not yet known.

r21a process 51160 is terminal with exit 101. Desktop identity passed 7/7;
PTY passed 82/84. Mixed peer takeover passed. Remaining failures are completed
Goal composer and fair queued subagents. The Goal mock first rejects an input
that is not its expected audit request; the missing successor ownership then
prevents publishing the resulting model failure. Fixing that ownership is
necessary but does not by itself prove the original composer-order failure fixed.
The fair queue mock reports a different first task; ordering evidence still
needs inspection. No build/test process remains live from r21a.

r22 now adds explicit `assistant_message_id` to Core `TurnStarted` events (no
old-wire default). Both Host-response and active-submission Application paths
ensure the successor message and record its typed TurnStarted binding in the
same canonical transaction. Store requires the matching committed Run event
before accepting that binding; initial Application admission remains separate.
The journal producer's ownership checks stay strict. Extended the real queued
successor integration test to require an exact owner before any Part exists and
after reopening, plus binding idempotence/conflict unit coverage. Not compiled
or executed yet. Added a cold-reopen variant of the real queued-successor test
that submits a non-retryable Host failure before any body and requires committed
journal publication, unchanged exact ownership, and a failed Attempt.

The r21 fair-queue DB proves FIFO admission was correct (First fair task created
and started before Second). The first worker instead exited on startup with
`reconcile observed Work step transactions failed: application_rejected: work
step transaction changed before transition was committed`. The CLI recovery
loop scanned global observed work, then sent strict Advance commands; another
Host finishing the same revision turned a benign lost CAS into worker failure.
r22 adds shared `ReconcileObserved` with explicit optional acceptance, deriving
the final outcome only from persisted observed evidence. CLI uses this command
instead of duplicating outcome selection. A changed revision/phase or lost CAS
is a no-op; storage errors still propagate, strict Advance is unchanged, and
checkpointed uncertain effects are never finalized by recovery. Added a real
two-connection concurrent recovery test requiring exactly one commit and an
unchanged uncertain transaction. These changes are not tested yet. Goal mock
ordering remains unresolved; do not weaken FIFO or Goal business assertions.

### r20 correction batch — terminal; r21 historical observations

Workspace acceptance process 83901 is terminal with exit 101, log
`/tmp/nola-cutover-r20a-rust-tests-20261008.log`, using the shared cache helper
and `cargo test --workspace --locked --no-fail-fast`. Exactly two targets failed:
desktop identity (6 passed / 1 failed) and PTY (83 passed / 1 failed). CLI units
passed 1516 / 0 / 2 ignored; all other workspace targets passed. This is not
native Xcode or real-provider acceptance. The r20 implementation freeze ended.

r21 changes, not yet tested: the takeover mock now permits the parent's
post-tool sampling while its first child remains held, validating that this is
not another child request. It retains the original strict Task and side-effect
assertions. The native permission fixture now preserves its isolated database,
exit status, challenge/projection and bounded recent output on failure; the
production cause of that failure is not yet established. No timeout was raised.

r21a focused acceptance is running under process 51160, log
`/tmp/nola-cutover-r21a-host-tests-20261008.log`, command through the shared cache
helper: `cargo test -p nola_cli --locked --test desktop_host_canonical_identity_e2e
--test pty_interaction --no-fail-fast`. This runs both complete failed targets,
not the full workspace. Poll the actual process for current status; no r21 pass
is claimed. Do not start another artifact build while it is live.

r21a partial results: desktop identity 7/7 passed (the prior intermittent native
permission failure remains unexplained); mixed peer takeover passed. A different
PTY case, `completed_goal_audit_returns_immediately_to_the_composer`, failed.
Its retained isolated DB is
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-pty-completed-goal-returns-to-composer-40022-1791446701831768000/nola-e2e.db`.
Canonical Goal evidence records `application_rejected: visible Attempt has no
exact persisted assistant owner`. The successor `:turn:2:assistant` Message
exists, but `assistant_turn_message_bindings` contains only initial admission.
The successor ModelAttempt is running and its exact outbox Turn cannot resolve
an assistant owner before a Part binding exists. Current active-submission code
ensures the successor Message from active Turn state without recording a Turn
admission binding; `AssistantTurnBindings` only collects Part bindings. Fix the
Turn birth/owner contract transactionally, including failure-before-first-body
and steering-created successors, rather than guessing an owner in the journal
producer or suppressing failure cards. Full PTY output is still pending.

Partial r20a evidence: the actual workspace log now reports
`saved_and_ephemeral_results_commit_before_ack_and_retry_the_same_committed_response`
as passed. This validates that fixture's original-response retry and altered
business-payload rejection; it is not a workspace, PTY, native, or real-provider
pass. Process 83901 was re-polled and remained live at this observation.
The CLI unit target subsequently finished: 1516 passed, 0 failed, 2 ignored
(82.61 seconds). The 84-test PTY target has started; its continuous task-scoped
permission-panel regression passed. Remaining PTY and workspace results are
still pending, not inferred from the successful unit target.

Further r20a PTY evidence: Goal/Skill/background restart and repeated automatic
compaction now pass. The mixed cross-project peer takeover test failed; do not
claim a full PTY pass. Its retained isolated database is
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-pty-cross-project-mixed-host-takeover-e2e-24013-1791445955379653000/shared-nola.db`.
Read-only inspection shows one shell correctly requires reconciliation and the
other completed; both Agent tasks imported failed canonical child Run results,
whose Host inbox contains `failed.error.code=provider_failed`. Thus this is not
only a missing UI marker. Before release, the fixture server holds one child
request and asserts that every further request is `/models`; inspect the final
panic to determine whether another legitimate child request violates that
fixture assumption. This is a hypothesis, not yet a proven root cause. Do not
change lease duration or rewrite terminal receipts to hide the failure.

The final PTY output confirms that hypothesis: the mock server panicked at
`background_flows.rs:2962` on `POST /v1/chat/completions` before the origin was
killed. PTY finished 83 passed / 1 failed (360.67 seconds). Correct the server's
single-in-flight assumption without relaxing original Task identity, immutable
terminal-receipt, or uncertain-shell side-effect assertions.
The desktop identity target also failed one of seven tests:
`stopping_native_permission_wait_persists_cancelled_turn_before_host_exit` did
not observe both the challenge and matching permission projection. Its target
finished in 7.83 seconds, below the fixture's 30-second wait: do not call this a
slow-test timeout. The fixture currently drops its temporary DB and omits its
already-collected recent stdout on failure; retain those diagnostics before
attempting to infer a production cause. The workspace process remains live.

The next hot-path cutover must change the real CLI consumer, not merely add a
projection type: `CommittedCoreRuntime::execute_committed_application_port`
still loads a full checkpoint after `effects.execute`, and
`restore_committed_checkpoint` clones it into a restored Core to rebuild a full
snapshot. Submission consumers in `core_agent_runtime_host_controls.rs` likewise
consume accepted checkpoints. Configuration/catalog adoption, pending permission
display, Goal state and memory-worker planning currently depend on that state;
each needs an authoritative scoped contract before removing the restore. Keep
full restoration only at explicit cold recovery/admission boundaries. A bounded
control snapshot alone cannot substitute for configuration or memory state, and
moving a full checkpoint read into a new query would not satisfy the cutover.

r19a process 86794 is terminal with exit 101. Its only failed targets are CLI
unit (1515 passed / 1 failed / 2 ignored) and PTY (81 / 3). All other workspace
targets passed. This is not native or real-provider acceptance.

The shared Application now reconciles exact previously committed terminal Host
responses before taking the hot Core or draining deferred publications. Store
checks original envelope equality and its actual committed-transition join;
Application validates frozen invocation identity and requires the same active
Run. Success returns applied=false with current durable revision/cursor, no
events or control snapshot, and no reducer execution or restored lease. An
altered envelope is a conflict. This preserves newer hot pending output and
allows target result-ACK retry without executing the source tool again. Added
tampered-business-text rejection to the saved/ephemeral result regression.
Cross-successor-Run replay is deliberately not authorized by this path.

PTY fixture corrections retain business assertions: the second Goal pause
asserts the exact canonical needs_user report and blocker instead of requiring
its summary as synthesized assistant text; automatic compaction waits for its
exact committed final control result (the mock mixes content and tool calls,
whose speculative text is suppressed); idle-command testing waits for a durable
idle Run before enqueuing configuration. Active-boundary rejection handling is
not proven by that idle-only fixture. No r20 runtime pass is claimed.

### r18 correction batch — terminal, two failed targets; r19 corrections started

r19a batch is frozen for consolidated workspace acceptance. Command:
`cargo test --workspace --locked --no-fail-fast` through the shared cache helper,
log `/tmp/nola-cutover-r19a-rust-tests-20261008.log`, process handle 86794. Its
live process, not this historical note, determines completion; no pass claimed.

Partial r19a runtime evidence: typed local cancellation and real HTTP cancellation
tests passed. Source/target birth failure recovery, lost-source rejection, and
the saved SessionStore open hook passed. CLI unit target finished 1515 passed /
1 failed / 2 ignored; the workspace process remains live. The remaining
single-result delivery test now passes its real model-request business payload
assertions but fails at the subsequent original-response replay: Application
rejects the settled effect as `not bound to the active Core`. The warm reducer
looks up current pending effect control before proving an already committed
response. Do not rebind/re-execute retired work to satisfy this test. Exact
committed-envelope/transition evidence must authorize only idempotent acceptance
and target ACK reconciliation, with altered payload/identity rejected and no
old control projection replacing current state. No code changes during r19a.

Further partial r19a evidence: PTY continuous task-scoped permission, fair task
queue, and explicit streaming-child cancellation now pass. Goal restart fails
later waiting for `CHAIN-RESUMED`, which its mock supplies only as goal_complete
summary (not model text); its persisted two Runs are completed and background
artifact exists. Preserve the no-synthetic-assistant contract and assert this
second control report from canonical state. Idle-composer queue failure is not
yet a proven idle failure: the fixture waits for a repeated prompt string and
enqueues configuration while the durable Run and model attempt are still running.
It needs an actual terminal/idle barrier before testing idle drain; active-boundary
rejection handling requires separate examination. Automatic-compaction PTY also
timed out and remains undiagnosed. Workspace process 86794 is still live.

Two further r18 PTY failures were traced to production defects, not weakened
assertions. Fair-queue evidence shows the first Task completed and both Tasks
were Admitted, but the second stayed queued while the parent awaited a model
response. The worker reaper now reconciles using the bound parent supervisor
policy after releasing the worker/attempt handles. Goal evidence shows an
Application rejection (`canonical Text planning failed: Closed`) while accepting
goal_complete; the Goal had scheduled retry after execution failed, not actually
committed the expected needs_user report. Core no longer synthesizes assistant
text from the terminal Goal control summary. It retains ToolCompleted and ends
the turn without re-opening the preceding text Part. The strict PTY report and
restart assertions remain unchanged and are required to pass.

r18c is now terminal (exit 101, process 14786). Only CLI unit and PTY
interaction targets failed: respectively 1510 passed / 1 failed / 2 ignored and
80 passed / 4 failed. The PTY failures are background task-scoped permission,
Goal/Skill/background restart, fair task queue, and explicit child cancellation.
All other workspace targets passed, including Service guidance restart and
desktop identity. This does not cover native Xcode or real-provider acceptance.

The r19 result-delivery assertion now decodes actual `AgentItem` values and
requires exactly one successful `wait_task` result for the dispatched call ID,
then checks business Task identity, result, status, and budget. Not rerun yet.
Explicit-cancellation failure has durable evidence beyond a missing toast:
the isolated fixture's child Run committed Host `Failed { provider_failed }`
and a failed terminal seal, while the global Task has `cancel_requested=1`.
The UI accurately displays that failed receipt. Investigate cancellation/error
ordering before changing terminal classification; do not overwrite a sealed
failure merely because a cancellation flag exists.

The cancellation transport defect is now corrected, not yet validated: both
locally observed cancellation branches in `llm_parts/streaming.rs` previously
fed translated `llm_cancelled_message()` through generic provider classification,
which has no cancellation classification and returns `provider_failed`. They
now construct stable `cancelled` control with retry/fallback disabled. Existing
Core handling already maps this code to cancellation. Added tests cover the
typed flags, non-authoritative provider cancellation strings, and an actual HTTP
stream held open until the consumer requests local cancellation. No Task receipt
or already-sealed Run status is rewritten.

The r19 permission notification fix is also implemented, not yet validated.
The failed PTY fixture proves first approval activated its successor attempt and
both writes executed; a second Bash permission was durably pending but hidden.
`ui_notified=true` from the first panel survived the second suspension. The
shared suspension transaction now resets UI delivery, and notification ACKs
carry the exact pending permission request ID. Store rejects an old occurrence
ACK, a resolved permission ACK, or a terminal ACK against a live permission.
The CLI uses the existing pending-suspension query before this exact ACK.
The same-clock two-suspension regression now verifies fresh delivery, duplicate
ACK idempotence and rejection of the first panel's late ACK. Model context
consumption no longer acknowledges independent UI presentation.

r17c is terminal with exit 101 and four failed targets: CLI unit tests
(1508 passed / 2 failed / 2 ignored), desktop Host identity (6 / 1), PTY
interaction (77 / 7), and Service guidance receipt restart (0 / 1). The log is
`/tmp/nola-cutover-r17c-rust-tests-20261008.log`; do not restart its finished
process based on the historical running notes below.

Two production causes are now evidenced. TaskPromoted was terminal to the
Application/outbox but still streaming in Core's pending-effect journal: actual
PTY databases contained completed parent Runs with acknowledged spawn effects
still pending in Core. The handoff now completes that exact foreground effect,
retaining late/duplicate-response rejection; new tests cover cold idle terminal
import and a real successor Run. Separately, single-task wait result delivery
incorrectly used the whole-mailbox resume acknowledgement, which fails when a
peer also has an unread result. The fix must acknowledge only the exact consumed
Task receipt and preserve the peer and independent UI delivery evidence.

The old wait tool also consumed unread state before its ToolCompleted response
was accepted. A single-result ACK primitive alone does not fix that ordering.
The r18 connection work moves consumption behind the original Session Store's
actual accepted response, matching the dispatched effect/request/call and Task
identity against a bounded typed result witness. Cross-Store source acceptance
and Task consumption are not one SQLite transaction: an ACK failure must leave
the result unread and the committed tool result recoverable, without re-running
the tool. A cold restart may redeliver that unread result through the ordinary
resume mailbox; no exactly-once external/model-consumption claim is made.

Fixture fixes do not relax production contracts: the CLI source freezes its real
Host binding before Run admission; generic shell display fixtures use the real
shell producer; external queue tests use typed Application ingress. Guidance
reconciliation asserts private Core changes and exact unchanged public cursor
when no public Task metadata changes. Goal pause tests verify the actual
assistant text followed by paused UI and then cold-read the exact committed Goal
summary/blockers, rather than demanding control-tool arguments as assistant text.
Native permission approval failure still needs actual exit/stdout diagnostics;
the shared cancellation assertion wording did not prove death by signal.
The r18 implementation batch is frozen. r18a exited 101 during compilation:
the new result-delivery regression used `RuntimeTaskAttempt.attempt_id` instead
of its real `id` field. r18b also exited 101 at compile time: the new Core
handoff test compared checkpoints whose state does not implement PartialEq.
It now compares the complete serialized checkpoints, retaining the no-mutation
assertion. Both compilation issues are corrected. r18c is running
`test --workspace --locked --no-fail-fast`; its live process determines terminal
status. Logs: `/tmp/nola-cutover-r18{a,b,c}-rust-tests-20261008.log`. Static Rust
parsing and diff checks passed. Partial r18c evidence: Application single-result
delivery tests passed (2/2), Task Run binding passed (14/14), and desktop Host
identity passed (7/7), including native permission handoff. CLI unit tests ended
1510 passed / 1 failed / 2 ignored; the workspace process is still running PTY
tests, so this is not a terminal workspace result.

Read-only diagnosis of the failed CLI result-delivery assertion found a contract
mismatch in the test: `effect_dispatch.rs` serializes `request.items` as typed
`AgentItem` values, whereas the assertion filters for provider-shaped
`role: tool` messages. `AgentItem::ToolResult` carries `type: tool_result` and
`output`, not `role` and `content`. Correct the assertion against the actual
typed request after the frozen run finishes, retaining business result, status,
budget, and exact call identity checks. This diagnosis does not yet prove that
the corrected end-to-end assertion passes or that provider serialization passes.

The r18 birth-recovery gap is now connected in r19, not yet validated. Previously
admission transfer existed only inside `accepted.commit.applied`: source commit
followed by failed target import stranded Prepared tasks after restart. Saved
SessionStore open now pages target Prepared identities (64 per page), locates
original queued Core admission events in that session, and invokes the existing
bounded export verifier and trusted import. No TaskHandle or checkpoint grants
authority. Exact Host response replay uses the same path even for applied=false;
source publication/cursor bookkeeping happens before the fallible target import.
Missing original evidence leaves Prepared work unadmitted rather than guessing
from the default database. Recovery does not execute spawn or create an attempt.
New tests cover injected target import failure, source reopen, exact proof/ACK
retry, foreign owners, lost ephemeral evidence, unchanged source checkpoint,
and the real saved SessionStore open hook. Runtime acceptance is still pending;
do not claim fully verified Task birth recovery or cross-platform parity yet.

### Historical r17 correction batch — superseded by terminal result above

r16e completed with exit 101 and 12 failing targets. Its process is terminal;
do not restart it based on the historical running notes below. PTY finished with
74 passed / 10 failed. Web 30 and cache-script 38 passing results remain valid
only for their own scopes. Native and real-provider r16 acceptance did not run.

The next production fixes are bounded: strict typed public queue admission and
one queue envelope/dispatch contract for composer, Host boundary and task inbox;
source-owned idle journal publication; serialized worker-progress decision time;
and immutable parent tool-call identity in the Store. Real tests exposed the
FIFO gap: `task_terminal_available` was decoded as a plain `AgentSubmission` by
composer and boundary consumers, causing errors or leased queue-head stalls.

Fixture corrections preserve actual behavior: genuine Agent scenarios use the
real source export/import chain; generic lease/receipt tests use shell tasks;
protocol examples use version 48; immutable-field corruption is rejected at the
Store boundary; warm permission tests bind activation to the real successor
grant rather than obsolete parent approval. The compaction animation fixture
now supplies distinct observations instead of a repeated paragraph that correctly
triggered the production repetition guard. Saved-tool assertions verify scoped
canonical call identities, the real discovery call and the three business calls.
The batch is now frozen for consolidated acceptance. Parent guidance producers
directly emit the shared typed notification; the obsolete `ResolveTaskGuidance`
submission and its conversion bridge are deleted. Negative ingress coverage
includes that removed wire variant. Idle observing now attaches the source-owned
journal queue, but its current production callers are memory operations whose
events do not produce assistant journal entries; this is not evidence of new
idle assistant text publication. Real task notification tests exercise trusted
imports, publication, FIFO ordering, replay and malformed-input release.
r17a exited 101 during compilation: the boundary terminal-import call omitted
the pending guidance IDs required by the trusted export. It now reads those IDs
from the original Session Store before import, retaining settlement coverage
rather than substituting an empty list. This pre-import recovery read and the
post-import control recovery remain explicit checkpoint-removal work.
r17b exited 101 during test compilation: the cold cancellation assertion used
the serialized name `pending_host_effects` instead of the Rust field
`pending_effects`; the assertion now uses the real field without weakening its
Cancelling/target/cancel-effect checks. r17c is running
`test --workspace --locked --no-fail-fast`; its live process is the completion
authority. Logs are `/tmp/nola-cutover-r17{a,b,c}-rust-tests-20261008.log`.
r17c has passed compilation and reached runtime tests. Application unit tests
pass 279/279; atomic ownership 5/5, task Run binding 13/13, terminal receipts 9/9,
and tool-batch permission/recovery 10/10 pass. These include exact successor
ownership, lost claim ACK, immutable identity rejection, real saved/ephemeral
terminal sources and rejection of removed parent command wires. Overall
workspace, CLI/PTy, native and real-provider acceptance remain unproven.

The remaining checkpoint cutover is a vertical change, not just response-field
deletion: active/idle acceptances still return checkpoint plus snapshot; CLI
restores and clones both after committed control operations; active submissions
still write a baseline on every reduction. The existing typed replay currently
records Host responses, not ordinary submissions. Next work must carry actual
submission decision/binding facts through that recovery chain, return real
bounded control and journal evidence, and migrate CLI configuration/catalog,
Goal and Memory consumers to exact-owner queries. Preserve full checkpoints for
cold recovery and explicit queries, not routine successful hot submissions.
Acceptance must compare real response/receipt sizes and WAL growth under small
and large histories, and retain lost-ACK, pending-guidance, permission and
cross-Run identity checks; payload slimming alone is insufficient evidence.

### Historical r16 acceptance notes — superseded by the terminal result above

The implementation batch is frozen. Workspace acceptance r16a exited 101 during
compilation (missing idle repository bound and an optional guidance queue used
without matching `Some`); r16b exited 101 on a missing CLI `ApplicationCommand`
import. r16c exited 101 on one replay fixture initializer missing the new
`claimed_effects` field. r16d exited 101 because an admission unit test compared
`AgentEvent` directly without `PartialEq`; it now compares the complete typed
serialization instead. Those compilation issues are corrected. r16e is running with
`test --workspace --locked --no-fail-fast`; its live process, not log presence,
determines completion. Logs are `/tmp/nola-cutover-r16{a,b,c,d,e}-rust-tests-20261008.log`.
r16e has entered runtime tests. The real cross-Store Task ownership suite passes
all five cases; source-owned saved/ephemeral terminal imports and the true idle
parent late-terminal regression pass. This does not make the batch green:
Application unit tests report 277 passed / 2 failed, CLI unit tests 1498 passed /
8 failed / 2 ignored, and three Application integration targets have one failure
each. PTY and subsequent targets are still running; no overall result is claimed.

Read-only diagnosis separates obsolete fixtures from production defects. Two
generic task receipt fixtures try to claim an unadmitted Agent draft; one
identity fault-injection fixture is rejected by the Store's immutable-identity
trigger before reaching the expected admission check. The warm permission
fixture still expects an unactivated lease after the global successor grant,
although that grant now directly authorizes activation. These tests must retain
their safety/atomicity assertions without restoring parent approval. A genuine
queue ingress defect remains: generic enqueue silently permits payloads that
fail `AgentSubmission` deserialization. Public producers already submit typed
commands, so the fix is strict schema admission, not a blacklist of retired
command names. All corrections await the frozen batch's terminal result.

The Web submission/observation/Markdown and trace set now passes 30 tests. The
legacy trace fixture was corrected to treat rejected assistant body deltas as
noncanonical input that cannot consume canonical trace sequence/event IDs;
production rejection behavior was not relaxed. Cache script tests pass 38.
Native and real-provider r16 acceptance have not run yet. The implementation
notes below describe the batch and retain their explicit unverified limits.

All r15 acceptance processes are terminal. Mac full exited 65 because two raw
multiline JSON fixture lines in `MacChatWorkspaceBehaviorTests.swift` had invalid
indentation; those lines are corrected, but native acceptance has not rerun.

The r16 implementation batch fixes internal compaction output-budget admission,
Task terminal preview byte limits without rewriting the original child seal,
and tests that still expected body text in SSE or confused an ephemeral session
with a persisted journal. The no-save UI test remains, with separate saved-session
journal coverage. Nonterminal replay now checks the pruned active outbox and
the exact identities of twelve settled deliveries separately. Publication tests
explicitly publish the real `Committed` notification; the Web model override
fixture now uses model-ID-keyed overrides. These changes have not passed runtime
acceptance yet.

The remaining architecture change is a single durable Task execution authority:
an immutable admission fact from the actual parent commit, followed by durable
Task/Attempt ownership. The parent Core projection must not maintain a second
lease ledger. Ephemeral parents must not be silently persisted to make recovery
work; an unadmitted Task cannot execute if its original source is lost. This
vertical chain is still being implemented, not claimed complete.

The local service transport now has a dedicated `host_import_agent_task_admission`
entry that constructs the non-deserializable internal Application import command.
It uses the existing local IPC trust boundary and durable receipt transaction;
it does not introduce a new signature or lease authority. Ordinary Application
JSON cannot construct the internal command. Added transport tests cover invalid
admission rejection before sending, forbidden revision/extra-field inputs, and
lost-ACK lookup under the original request/client identity without resubmission.
These tests are written but not executed. Host cache signatures now include
retention mode and the retained database route; a real acquisition test covers
same-route reuse and cross-route replacement. Source export/import and lifecycle
integration must finish before the consolidated acceptance run.

The same boundary now covers terminal and guidance projections. Global Task
storage decides execution, terminal settlement and guidance acceptance/delivery;
the original parent Application validates the exported admission against its own
source epoch and Task identity before applying a presentation update. A global
FIFO notification is not itself authority. The CLI reads the real source record,
imports through the retained parent port, refreshes its committed view and only
then acknowledges the notification. Terminal export selects only the parent's
bounded pending guidance IDs, not the entire accumulated guidance history.
The child-Run entry no longer decodes a second parent checkpoint/lease ledger.
Ordinary JSON import/terminal/guidance paths must reject forged facts, including
when the ephemeral parent has no global Task row. Service rejection tests now
cover all three trusted import types; actual cross-Store and lifecycle acceptance
remain pending. None of this implementation status substitutes for a passing run.

Terminal/guidance import now also reseeds committed assistant-journal heads from
the admission's exact original parent Run, followed by a `Committed` presenter
notification. An import ACK alone is not UI delivery. Added regressions use real
parent completion/interruption and child settlement, including a late import
into an idle ephemeral parent without modifying its existing Run seal. Guidance
tests distinguish global acceptance from parent observation and reject worker
summary completion of a bound Run before checking canonical terminal settlement.
These regressions are written, not yet executed.

The live terminal importer now selects pending guidance IDs from the Host's
already recovered committed view, eliminating its extra full recovery read.
Checkpoint removal is not complete: cold terminal delivery still loads parent
state, and both live notification paths restore the committed parent projection
afterward. These are explicit remaining control-path reads, not evidence that
all live updates are incremental. The
streaming body publication path must remain journal-based while those remaining
control reads are reduced to bounded state queries/deltas after this batch's
identity and recovery invariants are verified.

The legacy parent-authority surface being removed is explicit: `ClaimTask`,
`HeartbeatTask`, `ResumeTaskAfterPermission`, `RecoverTaskAfterPermission`,
`ScheduleTaskRetry`, `SweepExpiredTasks`, `QueueTaskGuidance`, and
`AcknowledgeTaskGuidance`. Their old wire forms must fail deserialization, not
remain compatibility handlers. Typed global task operations and trusted
projection imports replace them. The receipt's `allow_unclaimed_parent` flag
and CLI parent-claim mirror guards are removed as well. The fabricated idle
claim fixture's publication/checkpoint assertions now belong to the real idle
terminal import regression. Static inspection is not runtime acceptance.

Frozen-run audit found one additional publication gap to close in the next
correction batch: `CoreAgentHost::commit_idle_submission_observing` creates its
presenter with execution identity only, although `SessionStore` has already
captured that idle commit's journal cursors. Unlike `commit_core_submission`,
this route does not attach the notification queue. It needs the same retained
Store publication route and a real idle-commit delivery regression; no renderer
fallback to cumulative snapshot text should be added.

### r15 consolidated acceptance results

Latest authoritative results: r15c Rust exited 101 after all targets were
attempted. Seven targets failed: Application nonterminal replay, in-memory
effect lifecycle and atomic Task ownership; CLI unit, canonical Web Host and
PTY; Core Host contract. All rustdoc targets passed, unlike r14d. Real-provider
acceptance exited zero in 28.584 seconds, including multilingual follow-up,
Host restart, exact sealed UTF-8 answer, duplicate unchanged durable Attempt
frontier and cancellation before a late POST. It does not observe physical
provider HTTP call count. Mac full acceptance failed during test compilation,
log `/tmp/nola-cutover-r15-mac-full-20261008.log`; no native passing result is claimed.

The observations below were recorded while r15c was running and are superseded
by these terminal results and the r16 section where applicable. In particular,
the expired-worker failure was an unauthorized ledger event changing the event
count, not an obsolete exit-code expectation as initially recorded below.

The implementation batch is frozen. Web trace tests passed (49), and build-cache
script tests passed (38). The initial script-module invocation failed to import;
the supported direct script invocation then completed successfully.
The first r15 Rust command exited 101 during compilation: the new active parent
ownership mutation omitted `transition_id`. It now derives a stable transition
identity from the actual Run and claim/recovery operation. The complete twelve-
package r15b command also exited 101 during test compilation: the new complete
checkpoint equality assertion required an unimplemented `AgentThreadState`
`PartialEq`. It now compares complete serialized states, not selected fields.
The unused old full-request binding wrappers were removed. The complete command
is running as r15c. Compilation completed in 4m 17s; Application unit tests
passed (278), and integration tests are executing. This is not a complete Rust
passing result. Android debug
and release JVM tests completed successfully: each has 348 tests, 346 passed and
two skipped, with zero failures/errors. Native Mac and real-provider gates remain
pending. Logs are `/tmp/nola-cutover-r15c-rust-tests-20261008.log`,
`/tmp/nola-cutover-r15-web-trace-tests-20261008.log`,
`/tmp/nola-cutover-r15-build-cache-tests-20261008.log`, and
`/tmp/nola-cutover-r15-android-jvm-20261008.log`.

Live r15c observations (not final results): the measured budget/child-seal
Application matrix passed. Nonterminal replay is 5/6: its final effect count
reads the pending outbox, whose acknowledged entries are purged on reopen,
instead of immutable execution history. The in-memory delegated lifecycle test
rejects the late response but expects an obsolete error string. All four new
atomic Task ownership tests fail before their target behavior because their
fixture has a parent Task Turn without persisted assistant ownership. CLI also
reports failures in a publication test that omits the explicit `Committed`
notification, a Web fixture with incorrectly shaped model overrides, and an
expired-worker test expecting an old exit code. The credential HTTP integration
still expects body text in SSE rather than reading the committed journal. These
are triage findings, not fixes or evidence to waive the assertions. PTY tests
are still running; no source edits are being made during this acceptance run.

Read-only PTY triage found a production ownership discontinuity, not just stale
assertions: `--no-save` parents use `SessionStore::open_ephemeral`, but background
agent Tasks are persisted in the shared database. The new joint claim cannot
find that parent's checkpoint. The cross-project provider fallback fixture's
shared database contains two queued agent Tasks and repeated
`agent task parent checkpoint is missing` recovery events. The resolution must
connect the real parent/Task ownership and retention contracts; enabling saved
sessions only in the tests or weakening the parent gate would hide the defect.
The maintenance compaction fixture also records a failed provider response;
its complete PTY failure output confirms a timeout waiting for compaction
completion and still requires provider-path diagnosis.

PTY has now completed with 71 passed/12 failed in 474.90 seconds; the whole Rust
command is still running. CLI unit tests completed with 1,508 passed/three failed/
two ignored. Core Host-contract tests have one delegated-lifecycle failure
(117 passed). Real-provider acceptance is running separately against the newly
built CLI, using isolated evidence directory
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-real-submission-aca1im8f`
and log `/tmp/nola-cutover-r15-real-submission-20261008.log`. No outcome is yet
claimed for that run.

The twelve-package r14d command exited 101. Seven targets failed: application
nonterminal replay (three failures), CLI unit (1,506 passed/one failed/two
ignored), PTY (61 passed/22 failed, 642.70 seconds), and four rustdoc targets
reporting missing crates (Application, mobile bindings, runtime driver,
service). Rustdoc artifact availability still requires diagnosis; no tests are
disabled. The complete log is `/tmp/nola-cutover-r14d-rust-tests-20261008.log`.

r15 was implemented as one batch before consolidated testing. Current
changes include complete typed Web conversation assembly shared by list/get,
explicit read-only defaults for cross-client sessions, strict corrupt-profile
rejection and canonical model-selection precedence; exact current leased
Attempt reads (including expired leases, never latest-history substitution);
shared Domain resource scheduling types; and Task terminal classification using
validated measured budget exhaustion while preserving the actual child seal.
Task atomic recovery/claim is implemented with joint Application transactions;
new regressions cover cold ownership, parent-write rollback, renewal races and
lost acknowledgement replay. Final static review precedes acceptance.
Hot request-control separation
is implemented and statically frozen: ordinary stream reduction/replay no longer
materializes the complete request; explicit dispatch, cold restore and sampling
authority changes remain materialization boundaries. New large-prompt decode
count and recovery tests have not yet run.
Rust/CLI body trace generation/buffering is removed; Web rejects the removed
body trace contract and uses the nine-control shared fixture. Native consumers
have been migrated, preserving direct non-Agent model presentation. Mac behavior
tests now exercise actual journal append/read/seal rather than obsolete body
trace injection. Shared fixture duplicates retain the same event sequence.
Web owned-profile writers now validate the complete DTO before persistence;
branch creation uses the same profile contract and canonical model selection.
Core now emits only an rlib; native artifacts belong to mobile bindings, removing
duplicate native Core outputs. Whether this resolves rustdoc failures is unproven.
Lease claims also reject any token already present in that Task's durable Attempt
history, inside the same write transaction. This prevents a fresh Store Attempt
from colliding with Core claim-operation deduplication. Cold-reopen regressions
cover both fair and direct claims; both permission successor allocation paths
now use the same guard. These regressions have not yet run.
Mac compilation and CLI maintenance/fixture corrections are also included.
These are implementation claims only, not passing acceptance evidence.

### r14 correction batch and live observations (historical)

The r13 eleven-package test command terminated with exit 101 after all targets
were attempted: sixteen targets failed. PTY completed with 57 passed and 26
failed in 763.77 seconds; CLI unit results were 1,492 passed, 15 failed, two
ignored. Core unit results were 413 passed/one failed, and Store unit results
348 passed/four failed/one ignored. This supersedes the live-run observations
below. The original full log remains `/tmp/nola-cutover-r13-rust199-tests-20261008.log`.

The correction batch is now implemented and frozen for consolidated acceptance:
shared pure effect contracts move to Domain (Store/Port no HostProtocol
dependency); internal admissions publish their message identity in the same
transaction; cold readers reconstruct the same reason and exact digest; initial
Run recovery proves admission ownership explicitly. Cursor merging preserves
the returned request's independently established delivery authority and never
copies a hot lease. Tests retain foreign-identity rejection, exact rollback,
real cold state and immutable baseline semantics. Disclosure fixtures now
publish generation/backing facts before reading, and oversized-corruption
tests distinguish schema rejection from intentionally injected corrupt storage.
Bounded conversation listing and old task/retry fixture corrections are also
included. The first r14 compilation found two remaining Store test imports of
the removed HostProtocol dependency; both now use the canonical Domain types,
without restoring that dependency. Subsequent compilation also caught omitted
fields in two test fixtures (`BeginTurnInput.assistant_turn_bindings` in mobile
bindings and `Fixture.reopen_after_boundary` in the isolation regression), now
supplied explicitly. The current twelve-package rerun (including mobile
bindings) is recorded in `/tmp/nola-cutover-r14d-rust-tests-20261008.log`.
Android Debug/Release JVM verification each completed with 347 tests, zero
failures/errors and two skipped. Mac full verification is running separately.
No overall r14 acceptance result is claimed. The r14d application lib completed
278 tests successfully. Runtime integration still reports three failures in
`nonterminal_replay_e2e`, including a 264,610-byte durable receipt at model
completion; the original bounded-receipt assertion remains intact. The new
cold-reader reason/digest agreement regression passed. CLI tests are still
running and already report a rejected-restart transaction regression; final
counts must wait for process completion.

Read-only follow-up identifies an unbounded body path in control receipts:
`AcceptedHostResponse.traces` retains Core assistant/thinking delta/reset trace
bodies, including the accumulated assistant buffer flushed at completion.
Filtering `PartUpdated` from public events does not remove this second channel.
Next-batch work must remove body-trace production/buffering and connect all
consumers exclusively to canonical journal bodies, preserving control trace
sequence semantics; no receipt-limit increase or truncation is intended.

Two multi-cycle replay fixtures repeat identical tool arguments and reach the
existing repeated-batch guard; future fixture correction must use distinct
operations without disabling that guard. The CLI unit target completed with
1,506 passed, one failed and two ignored. Its rollback comparison baseline is
before a legitimate final old-Attempt reasoning flush (192 remaining bytes):
the extra accepted sequence advances checkpoint 7 to 8, with no new sampling
admission or changed Attempt/delivery ledger. The strict physical comparison
must be anchored after that accepted old-output boundary, not after the earlier
retry status notification. These corrections are not implemented yet.

PTY is still live, but its isolated pre-spawn recovery database already proves
two production gaps. A successor claim reaches the parent Core as Running,
then its one-second lease expires during acknowledgement/startup before the
first scheduled heartbeat; exact child admission correctly refuses it. Store
orphan recovery then moves durable Task/Attempt to retry-waiting without
retiring the parent's Core ownership, so the following claim is rejected as
not queued/retry-waiting. Recovery must reconcile the verified expired attempt
and parent Core ownership through the shared Application boundary; neither
relaxed claim checks nor extending an already expired lease is acceptable.
Evidence: `/private/tmp/nola-cli-pty-subagent-pre-spawn-recovery-e2e-61818-1791426668167880000/nola-e2e.db`.
No correction or passing recovery acceptance is claimed yet.

Mac full verification exited 65 during test compilation: `boundedSkillEvents`
was in the artifact struct instead of its MainActor test class, causing missing
symbol and actor-isolation errors. The helper has been moved into the test
class and stream/consumer setup made actor-correct, retaining permission,
cancellation and timeout checks. This Mac-only correction is not yet retested.

The actual provider submission script ran against the rebuilt CLI with an
isolated database. Its first answer, duplicate durable-attempt frontier check,
and Host restart/cold seal/body checks passed; the second same-conversation
multilingual follow-up returned HTTP 500 before completion. This is a failed
end-to-end gate, not a successful real-provider acceptance. Evidence is retained
in `/tmp/nola-cutover-r14-real-submission-20261008.log` and the isolated directory
named there; private fixture credentials were removed by the script on exit.
Read-only triage identifies a pre-admission adapter defect: canonical-created
conversations have no Web-owned profile; `get_web_conversation` synthesizes only
summary fields, then the next submission's title adoption decodes that object
as a complete `WebConversation` (missing required `titleSource` and other
fields). The second submission is rejected before a new Run is admitted. The
next batch must assemble a complete typed product projection from canonical
summary plus explicit profile defaults/overrides, not relax recovery identity
checks or retry the model call. This correction is not implemented yet.

PTY durable task evidence additionally exposed a missing prepared-Attempt bind
on a newly declared effect under a warm owner: ExecutionStarted was normalized
to Accepted with an Attempt ID, but replay retained `binding: null` before the
later sampling admission. The producer now compares bounded binding metadata
before/after ledger validation and records every real change, ignoring only the
journal-owned resume cursor, rather than using response kinds as a whitelist.
An actual three-cycle warm model/tool test checks persisted pre-sampling
bindings and cold equality. A separate regression checks checkpoint reason and
exact digest agreement across all cold readers after real model/tool cycles.
These regressions are included in the running batch, not yet claimed passing.

### r13 toolchain/storage recovery and consolidated rerun (historical)

After user authorization to redesign build storage, Nola's learned mbx state
was removed with workspace-scoped `mbx clean` (58.1 GiB logical reported).
The workspace target was explicitly adopted into managed storage. Dev/test
now use line tables without unpacked debug objects or Cargo incremental state;
native Mac/CLI build roots are unified. Automatic mbx learned/eager state is
disabled by default, with explicit machine storage budgets documented in
`../../总体设计/SOFTWARE_DESIGN.md`. Rust stable/Cargo upgraded from 1.91.1 to 1.99.0 and rustup
to 1.29.1, retaining all twelve installed target triples. The temporary duplicate
version-named toolchain was removed; the shared stable installation is used.
All 37 launcher/profile tests passed. Available disk at the start of the new
build was approximately 170 GiB; this includes external space recovery and is
not a measured saving attributable solely to the new profiles.

The eleven-package `mbx test --locked --no-fail-fast` has restarted under the
new toolchain at `/tmp/nola-cutover-r13-rust199-tests-20261008.log`. Compilation
completed in 5m56s; runtime tests are still executing. Early results already
include failures in Application startup recovery and initial-source replica
replay, so this is not an overall pass. Runtime, native and real-provider acceptance remains
required; the r12 compilation failure below is retained as historical evidence.

Read-only triage while this run remains live found a production omission:
`begin_internal_turn` enqueues resume publication but lacks the ordinary turn's
`run_message_admission::publish` before commit; current context reads correctly
reject that Run's missing admission proof. Separately, startup recovery's mock
tail omits the prepared Attempt binding, and compaction/model-attempt tests read
raw baseline bodies as current state. Corrections must preserve strict identity
checks and physical rollback assertions while reading semantic current state
through the actual cold replay API. `nonterminal_replay_e2e` also reports a hot
cursor/delivery mismatch requiring producer/supervisor investigation. These are
triage findings, not fixes or passes; source remains frozen until the complete
test run exits.

Further read-only triage: `overlay_request_cursor` compares the entire request
after normalizing only Attempt facts/cursor. Durable outbox declarations may
have no `delivery` lease while a validated cold bind retains one in hot Core;
claiming attaches that lease independently. Cursor merging must distinguish
this delivery authority from immutable request identity, without accepting
foreign Run/session/effect/request/payload or allowing a cursor read to claim
work. Exact-run initial recovery also still assumes that any `turn_started`
requires a later checkpoint commit witness, although initial admission now
persists a real start event; it needs an explicit initial-admission witness
case, not removal of identity checks. These changes are not yet applied.

The two cold checkpoint adapters also disagree on `CheckpointReason`:
`session_projection::materialize_local_recovery` forces `Manual` while
`load_current_checkpoint` preserves the final replayed reason. This explains
an exact-read digest versus LoadCheckpoint mismatch without an intervening
write. They must share reconstruction/serialization semantics, preserving
digest CAS. CLI unit execution finished with 1,492 passed, 15 failed and two
ignored; the encompassing run is still live in the 83-test PTY suite. Some
CLI fixtures still fake terminal status without an actual seal; these need
real seal production rather than weakening terminal validation. The 5,000-row
conversation test also exceeded its one-second threshold; investigate the
full-list-before-limit query rather than merely raising the threshold.

### r12 control/outbox correction batch frozen

Application/Core, Store/StorePort and CLI/native consumers are frozen. The
eleven-package `check --keep-going --tests` ended with exit 101 at
`/tmp/nola-cutover-r12-rust-check-20261008.log`; Runtime Driver and Runtime
Supervisor are included as test targets, not only dependencies. No r12 pass is
claimed for Rust: four integration targets reported ten compile errors (old
Driver/Host snapshot fields and return type, an error assertion's Debug bound,
and a HostEffectId accessor). Production targets reported no compile errors.
The four fixture corrections were applied together, retaining terminal,
lease and failure-reason assertions. The eleven-package `test --no-fail-fast`
attempted the frozen batch directly, without another redundant check-only pass
(`/tmp/nola-cutover-r12-rust-tests-20261008.log`). It exited 101 during compilation:
the linker for `run_payload_wal_e2e` and LLVM for `conversation_recall_e2e`
reported `No space left on device`. No runtime tests executed, so this is neither
a runtime pass nor an assertion failure. At termination the volume had only
119 MiB available; `target/debug/deps` occupied 93 GiB and the shared mbx
incremental cache occupied 38 GiB. No manual cache or user-data deletion was
performed. Capacity must be restored before rerunning the full gate.
Android Debug and Release JVM test gates exited 0:
each reports 347 tests, zero failures/errors and two skips (345 passed). Log:
`/tmp/nola-cutover-r12-android-unit-20261008.log`. These do not validate Android
native FFI or device execution. `AgentControlSnapshot` is constructed from scalar state without first
cloning the full snapshot. Existing outbox effects use typed binding updates;
only first declarations serialize their immutable request bodies. Permission
queries use exact Run scopes plus independently committed admission identity.
Runtime, migration/WAL, native and real-provider gates remain required.

### r11 consolidated verification started

The nine-package Rust `check --tests` reached two compile failures in sequence:
the boxed active-turn method reference (r11) and the CLI snapshot-release test's
unspecified generic result (r11b). Both are corrected; neither run passed.
Logs: `/tmp/nola-cutover-r11-rust-check-20261008.log` and
`/tmp/nola-cutover-r11b-rust-check-20261008.log`. The check includes Core,
Domain, HostProtocol, StorePort, Store, Application, Service, CLI and Credential
Host so the new protocol/domain unit tests are not omitted as mere dependencies.
The r11c nine-package Rust `check --keep-going --tests` exited 0 in 5m19s.
Runtime and real-provider gates remain unproven. The subsequent consolidated
`test --no-fail-fast` run covered the same nine packages but was deliberately
terminated during compilation (exit 143), before any runtime test result.
Its log is `/tmp/nola-cutover-r11-rust-tests-20261008.log`. Before starting it, the public-part
fixture gained an explicit successful-projection assertion instead of silently
ignoring its `Result`.

The stop followed a concrete hot-path gap: non-delta Host response receipts
still embedded `AgentCoreSnapshot`, including the cumulative Part projection.
Checkpoint Replay alone therefore did not remove cumulative-body write
amplification. The next correction replaces this receipt field with a small
typed control result, with permission contents read through the existing exact
projection query only when needed. The outbox now separates its immutable
request body from mutable delivery bindings in schema 57; Application wiring
is being completed. No compile-only result establishes these fixes.
The real received-command boundary regression now supplies over one MiB of
actual text/tool results and checks persisted receipt size, absence of the old
snapshot, cold equality and duplicate receipt replay. It has not run yet.
The r11 Web frontend rerun (`npm run check && npm test`) exited 0:
Svelte diagnostics reported zero errors/warnings and all 264 tests passed.
This does not establish the native/runtime or real-provider gates.

Read-only audit during the frozen test run found a remaining recovery-discovery
gap: `resume_snapshot_port` filters missing body/malformed checkpoint JSON out
of candidates, and `inspect_exact_run` collapses a missing body into
`checkpoint_present=false`. Actual checkpoint loading already fails on a
missing body, so this is not evidence of a cross-Run permission bypass. Explicit
corruption reporting and regressions are now implemented, not yet compiled or
run. Normal
GoalRuntime control changes write baselines; Host replay does not independently
mutate GoalProjection, and the selected candidate is checked again after replay.
No extra Goal state table is justified by this finding.

The permission read contract no longer accepts an unscoped conversation-only
request. Both explicit cold `current_run` and hot `control` scopes require an
exact Run. Control refresh additionally validates session, active Turn and
permission revision, allows only forward event-cursor progress and returns the
actual witness. Responses contain pending permissions, not cumulative receipt
history. Tests now cover empty pending ownership, successor Runs with colliding
permission revisions, missing/null witnesses, and unrelated checkpoint content.
Client migrations and these regressions remain uncompiled/unrun in this batch.
Cold reads additionally bind the checkpoint session to the exact Run's stored
`turn_started` admission envelope. Equal-but-foreign root/state session fields
or missing admission evidence fail explicitly; optional frozen configuration
session IDs are not used to guess this authority.

The follow-up batch removes the unused text-only field-reader path, singleton
journal publication wrapper, and duplicate text-close planner API. Their tests
now exercise the production field-aware reader, publication set, and public-part
close operations, retaining owner/field isolation and atomicity assertions.
The frozen batch passed `--keep-going --tests` across the same nine packages
(`/tmp/nola-cutover-r11c-rust-check-20261008.log`).

The nonterminal replay producer and consumer now share typed normalized input,
actual optional binding, exact decision clock and reconstructed checkpoint
reason. The response-kind whitelist has been removed; terminal/budget boundaries
retain explicit baselines. Store maintains cumulative byte frontiers and checks
actual cold-tail bytes against them. New tests cover multiple real model/tool
boundaries, cancellation, legacy evidence isolation, migration rollback, and
record/byte thresholds.

Task terminal receipts now reference their seal rather than copying all result
heads. Manifest verification uses two immutable passes with the unchanged digest
format; public preview is capped at 4 KiB with bounded candidate references.
Disclosure checks no longer parse or measure inline JSON per page. Regression
instrumentation counts actual scanner consumption; SQLite EXPLAIN additionally
checks that the disclosure query does not materialize the JSON column. These
are implementations and pending test assertions, not measured r11 successes.

### r10 acceptance ended; next correction batch is unverified

The complete six-package Rust command ended with exit 101 after all remaining
Store integration tests and doc tests (24 Store doc tests pass). Twelve targets
failed: four Application integrations, CLI unit binary plus four integrations,
Service process integration, Store unit suite, and Store context-usage read model.
No failed, aborted, or unexecuted case is counted as passed. The next batch is
being implemented as a whole before rebuilding or rerunning acceptance.

Schema 56 now physically separates `core_checkpoints` (revision/cursor/time)
from `core_checkpoint_bodies` (baseline JSON). All admission, projection restore,
idle and transition baseline writers use the same transactional header/body
writer. Replay updates remain header/witness-only. Body replacement invalidates
its replay base and derives the new permission witness; deleting a body removes
derived witnesses. Missing bodies fail recovery rather than looking like a new
session. A one-time transactional move removes the old column, with no dual-read
fallback. Cold recovery, archive cleanup, presentation reads and SQL fixtures
have been switched to the new physical layout. Added migration/rollback/missing-
body/cascade tests and extended real-Core WAL coverage are **not yet run**.

Initial admission control-event persistence, Core heap-owned state boundaries,
and authoritative task terminal result/parent-delivery separation are also in
the current parallel implementation batch. Their completion remains unproven.

Initial event admission now checks the exact baseline's Core session, Core
revision and event cursor in the same transaction. Store CAS revisions and Core
reduction revisions are explicitly separate clocks. A foreign but internally
consistent group of event envelopes must roll back the entire admission.

Removing the remaining nonterminal model/tool boundary baselines requires a
typed replay fact: normalized response, exact reducer decision time, and the
lightweight request binding applied before reduction (only when a bind really
occurred). Saving an entire Host request/prompt is not an acceptable replacement
for a full checkpoint. Replay must use the captured identity, never the latest
lease or model attempt. Both record-count and byte bounds are needed for the
replay tail. This path is under implementation, not verified.

Earlier response-only replay rows do not always prove whether a hot/cold
reduction rebound its request. Migration may mark rows already covered by a
baseline as covered; ambiguous live tails must retain their original evidence
and require explicit recovery for that Run. They must not prevent unrelated
sessions from opening, fabricate old authority, or automatically repeat model
sampling/tool side effects. There is no read-time legacy-format guessing path.

The following paragraphs record the r10 results before this new batch:

Web build/check/tests pass (264 tests, zero check warnings). Android Debug and
Release each pass 346 tests with two skips. The six-package Rust test check
passes after restoring the missing repository-trait import in test support;
runtime tests subsequently ended as recorded above. iOS acceptance completed with script exit 0:
the result bundle reports 548 tests, 539 passed, zero failures, nine skipped
(545 device-level passed invocations including dynamic parameters). Its cold
journal paging and real-source Web summary regressions now pass. This does not
prove Mac completion or resolve the Rust failures below.

Mac full acceptance was deliberately interrupted after a Skill fixture waited
indefinitely for a real `request_user_input(loop_checkpoint)`. It is **aborted**,
not passed: the fixture has neither a user-input handler nor an overall timeout.
Two earlier Skill cases failed as well. Investigation identified request-count
stage advancement, premature candidate-final disposal, missing workspace
configuration, and execution/rollback mutations without corresponding declared
effects. Replace the fixture's request counter with exact call/receipt-driven
state; unexpected user decisions must fail and cancel, not silently grant or
fabricate completion. Production resource-version validation must remain strict.

The latest system diagnostic (06:04:17, parent XCTest process 37111) still records
2,147.49 MB of file-backed dirty writes over 112 seconds. It belongs to automated
acceptance, not the latest manual launch. The immutable Run payload split alone
therefore does not prove removal of write amplification; remaining commit/WAL
paths need measured attribution.

Read-only stack attribution finds 42 of 59 samples in commit/WAL work and 17
in mutations. The sampled interval overlaps the repeated-compaction capability
test, but the diagnostic's cumulative interval also includes preceding tests;
do not assign all 2.15 GB to that single case. Remaining candidate payloads are
baseline checkpoints, full admission/active/idle command results, and canonical
Part row replacement/SQL text concatenation. Admission's full checkpoint is
currently consumed by the CLI to restore frozen execution state. Removing that
payload requires an exact recovery handoff, not simply omitting the field.
Separate once-per-Run baseline cost from repeated hot writes before changing
the persistence contract.

The actual `run_payload_wal_e2e` runtime test now passes: 64 real streamed
commits compare 4 KiB and 2 MiB frozen configurations with a pinned WAL reader,
and verify exact cold-reopened payload identity. Its absolute/relative write
bounds prove the targeted Run payload separation, not all checkpoint or
model-boundary write costs. Replay still updates the checkpoint clock/header
in the same physical row as the baseline body. Splitting that storage must
preserve the single transactional baseline/replay frontier and corruption
checks; no old-column fallback is acceptable.

Rust runtime acceptance has exposed two context-read failures so far. One
historical fixture uses zero-based Part ordinals despite the one-based contract.
The other is a production admission gap: Core emits initial `ContextUpdated`
and checkpoints its fingerprint, but initial admission does not persist that
event to the authoritative Run event source. Subsequent unchanged estimates
are suppressed, leaving no committed usage fact. Fix initial event publication
inside the admission unit of work; do not introduce a checkpoint fallback or
manufacture a usage event in the test. These corrections are not yet applied.

A further window-read regression still expects checkpoint-only usage `123`;
its fixture commits no authoritative usage event. The new result `0`/unknown
is consistent with the no-checkpoint-fallback contract. Correct that obsolete
expectation while preserving its Part ownership and bounded-allocation checks.

Further r10 failures are not a completed acceptance result: five permission
tests compare immutable canonical requests with claimed requests carrying a
delivery lease. Correct them by separately verifying the durable attempt's
ownership/token/generation/state and the canonical request; do not discard
lease assertions. One batching test still requires uncommitted Part events;
replace that expectation with unchanged journal frontier before commit and
exact publication after the boundary, retaining rollback/cold-replay coverage.
The CLI test binary aborts with stack overflow in idle checkpoint decoding
(`reduce_and_commit_idle_submission_in_transaction`, confirmed by the system
crash report), so its unfinished tests are not passes. Three process-level
journal tests also fail in shared test support that attempts snapshot work
outside a write unit of work.

PTY acceptance has now finished: **9 passed, 72 failed**, 1,396.73 seconds.
The enclosing Rust command subsequently completed. Actual panic
output confirms a shared production display failure: `TerminalJournalReader`
requests a 65,536-byte body range while Store permits 4 through 32,768 bytes.
The reader is also used by SourceRunJournal, so this is not merely a terminal
screen assertion. Completed source Runs/seals can coexist with missing displayed
answers. Correct the range protocol constants and consumers, preserve Store's
bound, and test multi-page Unicode plus failure-safe cursor advancement. Do not
attribute every PTY failure to this one cause; provider-fixture assertions and
task recovery failures also need their own evidence.

### r9 terminal evidence and r10 corrections

All r9 handles are terminal. Web builds and passes 264 tests; type checking has
zero errors and two unused CSS warnings. Android Debug and Release each run
343 tests with one failure and two skips: the real loopback SSE test omitted
the outer encrypted-envelope type, so production correctly rejects it as an
untyped plaintext event. Mac and the combined Rust command fail compilation
because new-Run admission calls nonexistent container `is_empty` methods.
The separately attempted Core/Store/Application/Service test build also stops
before execution: shared remote-submission test support omitted the
`RunMessageAdmissionRepository` trait import. These are not Rust test passes.
iOS summary reports 544 tests: 533 passed, two failed, nine skipped. Both
failures are real-source Web summary cold journal reads.

r10 distinguishes cold directory display order `(section, position, part_id)`
from incremental Part-ID order. Cold continuation is the last scanned key,
which may be retired or postdate the snapshot; empty pages are valid if the
opaque cursor advances without cycling. The Application page does include
`at` (although its internal Store page does not), and exact pinned-cursor
validation remains mandatory. iOS additionally removes a lexical sort in its
presentation fixture that previously concealed the production contract.
Android and iOS corrections/regressions are written, not tested. Mac's existing
opaque-directory traversal does not contain the same lexical-order bug.

The same correction batch is moving immutable Run configuration JSON out of
the frequently updated Run row, retaining its bounded identity digest and all
transaction/CAS checks. There will be one payload source, no old-column fallback
or dual write. Core will also own the new-Run admissibility predicate, including
queued inputs and buffered terminal responses, rather than exposing containers
for CLI to infer quiescence. Implementation and consolidated validation remain
in progress; neither hot-path performance nor overall acceptance is achieved.

### r8 evidence and the next root-cause corrections

Mac r8 compiled and executed 930 XCTest cases: 12 skipped and 166 failure
records across 74 distinct failing methods; the separate 27 Swift Testing cases
passed. Large-history performance, Markdown, task observation and integer
precision tests did not show independent failures. Many remote/UI failures
originate before presentation, in first receipt admission. Android Release ran
338 tests with two failures and two skips: the new control-request fixtures
inject a stream MockEngine but production correctly uses an independent control
engine, so the requests reach unserved localhost port 80. Rust test compilation
stopped on two calls to a nonexistent presenter method in the new regression;
it is not a Rust test pass. iOS r8 finished with 539 tests: 517 passed,
13 failed, nine skipped. Ten failures came from the reader requesting 64 KiB
against the Store's 32 KiB range limit; the other three were first receipt
publication, an obsolete fixture string, and missing snake_case citation line
coding. This result does not prove the new fixes below.

Confirmed production causes to correct together after the batch:

- New-Run admission adopts terminal checkpoint-redacted `tools=[]` after an
  idle configuration commit. Recovering an active Run must keep its frozen
  configuration; starting a new Run must use its newly captured capabilities.
- Preparing a remote intent creates conversation revision 1 without a content
  publication. Receipt admission increments that revision and incorrectly uses
  the append path merely because revision is not 1. Content publication state,
  not conversation revision, determines whether a base exists.
- History-window journal discovery invents local provenance for an archived
  assistant's `run_id`, even when no real Run/journal exists. Discover actual
  journal heads first; do not manufacture admission for imported history or
  weaken ownership validation on real journal reads.
- Remote context usage has no durable receive/read model. Summary checkpoint
  token accounting is not current model-context occupancy and cannot be used
  as a UI fallback. This remains a real cold-recovery gap, separately from the
  tests that currently confuse those two facts.

Other fixture corrections must retain production contracts: explicitly declare
vision capability in the native-image scenario; keep Skill lifecycle/validation
obligations, use the same isolated database for Skill disablement and execution,
and create a real frozen invocation for cancel-only recovery. The real-provider
script still needs exact completed seal/manifest verification; receipt-only
duplicate responses alone do not prove no extra provider request occurred.

The next correction batch now fixes initial turn publication from actual
content-marker absence and first ordinal, not metadata revision. It refuses to
repair missing provenance of existing history by republishing it. Added tests
cover publication failure rollback, exact receipt replay after reopen and a
local first turn following an abandoned remote intent. Historical head discovery,
iOS bounded ranges and citation encoding are corrected with regressions. The
cancel-only fixture now admits its frozen Host binding through Application, and
the Web-source opt-in fixture creates the actual conversation before granting
its override. These changes are not yet tested.

Android loopback testing exposed an unconditional production rewrite of every
configured loopback Host to the emulator alias. This legacy rewrite is being
removed: configured origins must be preserved, and `10.0.2.2` remains an explicit
configuration choice. This does not merge control and streaming connection pools.

For context usage, reuse the existing authoritative `AgentEvent::ContextUpdated`
committed to `run_events` in the same Application transaction. Its payload is
already bounded and Core emits it only when changed. An exact-Run indexed query
and owner-validated replica reception must replace memory-only remote usage;
another source checkpoint decoder or duplicate source statistics table is not
needed. The correction batch now connects the exact-root record to Host
observation/journal reads, receiver admission validation and durable storage,
and Mac, iOS, Android and Web presentation. iOS terminal recovery reads usage
even when its live notification was lost; the old in-memory usage ledger is
removed, and missing committed totals are not replaced by history estimates.
Full conversation projections must use this same committed query, not their
former checkpoint-derived usage. Source/receiver/cold-reopen regressions are
added; the batch has not yet been built or tested.

An additional performance acceptance issue was found in the macOS system
diagnostic `nola-service_2026-10-08-043112_MacBook-Air-6.diag`: 2,147.48 MB
of file-backed writes over 82 seconds. Its parent PID 95261 matches the r8
Mac XCTest host, not the last manual Xcode launch. Sampled stacks include
command transactions, model-attempt persistence and SQLite WAL/checkpoint
writes. This proves substantial test-run I/O, but does not yet establish a
per-token write amplification factor or identify the responsible SQL payload.
The actual 04:30:47–04:31:11 sampling interval falls inside
`testRealAgentHostRepeatedCompactionPreservesOriginalConversationRecall`:
the fixture makes 24 turns with roughly 6 KiB filler per turn and short ACKs,
not gigabytes of generated text. This narrows investigation to production
persistence under repeated turns/compaction; absent a retained fixture database,
the report alone cannot assign write volume to individual tables.
Do not claim the hot-path performance gate passed until that attribution and
the bounded-write regressions have been verified.

### r7 consolidated validation finished; remaining compile gates are not passes

All r6 and r7 processes have reached terminal status. r7 Web production build,
type checking (zero errors/warnings), and all 260 tests pass. Android production
compilation passes but unit-test compilation fails on the new receipt test's
JUnit argument order; no Android tests ran. iOS production compilation passes
but the migrated rich-part test contains an orphaned old method fragment, so
its tests did not run. Rust and Mac stop on the new CLI journal consumer's
borrow/type errors; Rust also reports an invalid inner doc comment in an
included PTY test file. Neither constitutes a runtime or terminal-seal pass.

r6's missing Store-port serde_json production dependency, Web strip-only
constructor/type/reactivity errors, Android receipt adapter branch, and iOS
decode isolation/task-validation visibility errors are corrected. The next
correction batch preserves journal ownership and full rich-part assertions;
it must not restore cumulative events or remove failing semantic coverage.
Real-provider, cold recovery, and bounded checkpoint/receipt acceptance remain
unproven. Logs are `/tmp/nola-cutover-r{6,7}-*-20261008.log`.

### r5 consolidated validation finished; r6 corrections in progress

The current iOS cutover now reads assistant content through pinned journal
snapshots, typed part changes and UTF-8 ranges for both device execution and
remote reception. Window metadata publishes owners first; cold history then
hydrates those exact owners through the same reader. UI journal cursors remain
separate from Store event/message revisions: a metadata refresh cannot clear
committed content, and a late draft cannot resurrect retired content. Device
transitions forward `assistant_publications`; the old device PartUpdated events
and Web PartUpdated parser/consumer are removed. Traces no longer author either
local or remote assistant text/reasoning. Reader and UI cursor regressions have
been added, but this correction batch has **not** been compiled or tested.

iOS terminal return now awaits the exact root journal read after seal reception;
it does not return whichever prefix happens to have reached the UI queue. Live
permission/device handling binds the admitted submission's Host and Client and
uses its captured transport; permission requests additionally match conversation
and canonical Run. Await boundaries recheck execution generation before running
device work, submitting results, or updating presentation. Task cancel/guidance
and observation pin the conversation's provider, reject owner changes, and clear
old task presentation on selection changes without cancelling the original Run.

Final consumer audit also found Messaging Channel and goal-audit readers still
expecting the removed cumulative Part event. They are being moved to exact-owner
Application journal/seal reads. The auditor must retain its existing ephemeral
Store rather than persisting an internal audit conversation in user history.
These consumers are part of this batch's freeze gate, not optional follow-up.

The frozen r5 batch has finished. Web production build
passes, type checking reports zero errors and one existing non-void-tag warning,
and 251 tests pass. Android Debug and Release each report 331 tests, zero failures
or errors and two skips. Mac compilation stopped before tests on an optional
UUID passed to a nonoptional owner identity in `MacWorkspaceModel`; this is not
a Mac test pass. iOS compiled and reports 524 tests: 513 passed, two failed and
nine skipped. Rust compilation stopped on the new clock regression attempting
to serialize non-serializable `AgentReduction`, before executing its tests.
The r6 fixes compare all reduction fields without changing the production wire
type and reject an invalid optional Mac owner mapping rather than inventing an
identity. iOS selection-versus-execution generation, memory-kind assertions and
test teardown are being corrected together with CLI partial-stream presentation;
these corrections have not been revalidated.

The Task Claim confirmation reader now compares recovered **Store transaction
revision** with its metadata waterline, not Core's independent reduction revision.
The added regression deliberately uses Core revision 17 and Store revision 3.
This distinction matters after a multi-reduction durable batch.

Known unresolved scope includes real-provider acceptance, remote local admission
placeholders when a request never reached its source, and non-Claim PTY failures
(partial-stream duplicate/presentation ordering, first-delta delay, idle queue
drain and obsolete scripted tool/audit scenarios). They are not covered by the
successful Web/Android unit results or declared fixed by the Claim correction.

### Remote admission replacement in progress — not yet batch-validated

The removed `OpenRemoteConversationTurn` path created canonical messages and a
queued Run before the remote HTTP request. Its source-host binding was required
by message-admission and journal reception. Therefore deleting that Run locally,
or treating discover-null/404/409 as rejection, is not a valid recovery fix.
An original request may still arrive or may already have completed at source.

The replacement must connect these steps as one vertical change, not add unused
receipt helpers:

1. Persist a receiver submission intent, separate from canonical Run admission,
   with source Host, Client, conversation, submission, user/root identities and
   a digest of the immutable semantic request. Pending input presentation reads
   this intent; it does not pretend the source has admitted a Run.
2. Source registration durably binds the same identity/digest before scheduling
   execution. Exact retries consult the same receipt; changed payload or owner
   conflicts. Source defaults and resolved invocation must remain frozen after
   admission. A transport request ID or process-local worker flag is not this
   receipt.
3. Source Run admission and the admitted receipt share the actual Application
   transaction. Only that authenticated fact authorizes creating the receiver's
   replica Run/root/source and following admission pages → journal → seal.
   Existing local Run/root cold recovery always observes that exact owner before
   considering discovery; a source with no *active* Run may still have its seal.
4. Explicit rejection/cancellation of a not-yet-admitted request is a source
   fact. Cancellation must atomically leave a tombstone that a late admission
   checks; deleting receiver state cannot cancel possible execution.

The Core replacement and Host integration are now being implemented together
with four-client migration. These edits have not yet been compiled or tested as
a batch. No compatibility fallback may resurrect optimistic admission.

The Host `/api/{mobile/}chat/agent/v2/submission` endpoint has three operations:
`prepare` only validates the exact wire owner and computes canonical request
digest (it does not create a source receipt or claim execution); `observe` reads
the exact identity/digest receipt; `cancel` atomically records the cancellation
tombstone or returns an already-admitted receipt. Canonical stream identity uses
`conversationId`, `expected_host_id`, `client_id`, `submission_id`,
`user_message_id`, and `assistant_message_id`; the request `session_id` alias is
removed. Stream digest is checked against the original JSON before mutable Host
defaults are resolved. Only a newly created source registration may dispatch;
duplicate requests emit the existing receipt and close without running again.

`submission_receipt` SSE carries a nested `receipt`. A shared delivery fence
publishes admission before the first journal/control event or synchronous
permission challenge. Worker exit rejects only an unadmitted registration;
admitted Runs retain their real durable lifecycle. Normal worker exit persists
and sends the final receipt before EOF; Drop rejection is only an unwinding
fallback. HTTP EOF is not completion. Active-Run `/cancel` now requires the exact
submission identity and request digest, validates an admitted source receipt,
and signals only the matching client/root/Run worker under its generation lock.
The conversation-only cancellation and implicit replacement interfaces are
removed. Missing source receipts do not authorize remote resume or advertise it
as available. A failed durable Host identity load now prevents server startup;
it never silently generates an ephemeral replacement for a paired Host.

Prepared local intents with no bound request may be abandoned locally. Bound
intents require a source cancellation/rejection fact. Receiver admission creates
identity shells, not a copy of draft text/attachment metadata; authenticated
source admission records populate canonical user content. Cold observation can
adopt a genuine admitted `submission_receipt` when no originating intent exists,
using the same atomic shell/source-pin writer. It cannot bypass an existing
intent's digest check or change the origin Client. When discovery omits an
already-terminal originating intent, observation reuses exact bound receipt
acceptance instead of rejecting the legitimate cold read or creating another
replica. That path preserves terminal status and still rejects changed Client,
request digest, or source epoch. This is not a new cross-client
authorization grant. Current Run observation returns this source receipt beside
the admission cursor, rather than synthesizing one from that cursor.

Acceptance still requires consolidated builds/tests and real-provider recovery
coverage. The goal is not complete.

The retired Mac Part transport reducer tests now use actual Application-authored
journal cursors, pinned snapshots and terminal seals; rich presentation-only
cases use canonical typed DTOs rather than forged transport Parts. The old
standalone Part reducer harness is removed and the cross-client alignment
script targets current journal/receipt consumers. These are test migrations,
not evidence of a successful validation run; the full batch remains unexecuted.

The new `scripts/test_real_submission_journal.py <current-nola-binary>` scenario
is prepared for that batch, not yet executed. It starts an isolated real Host
with `NOLA_HOME`/`NOLA_DB_PATH`, keeps the OS home untouched, and reads public
assistant text only through bounded journal snapshots/ranges. Its gates include
admission-before-content, source terminal seal, duplicate-no-dispatch, a full Host
restart followed by exact receipt/body recovery, a multilingual natural follow-up,
and cancellation before a delayed POST. Passing it would prove that specific
real-provider HTTP path, not all native UI or tool-permission scenarios.

Remote context usage also lacks a durable independent publication: the live
`context_usage` event updates client memory, while imported cold projections use
default usage. Compaction checkpoint statistics are not a replacement for runtime
usage. Preserve source/Run/frontier identity when closing this remaining control
projection path; do not restore cumulative assistant snapshots to carry it.

### r4 consolidated validation finished; r5 corrections unverified (2026-10-08)

The r4 batch completed consolidated validation. Its Web production
build and type checking pass (one non-void HTML self-closing-tag warning), and
all 248 Web tests pass. Android Debug and Release unit suites each report
319 tests, zero failures/errors and two skipped tests. These are client-level
results, not proof of native bridge, device UI or real-model acceptance.

The six-package Rust/Core/Store/Application/Service/CLI/credential-host run
compiled successfully and finished with exit 101 and ten failing test targets.
Its first reported failure was
`run_effect_port_e2e::cold_effect_replay_uses_explicit_time_and_saturates_deadline`:
the observation timestamp is outside Core's supported range. The r5 correction
rejects unsupported observations before mutation and uses supported boundary
times in recovery fixtures; the clock/lease contract is not weakened.
macOS full validation compiled and finished with exit 65: 919 tests executed,
12 skipped and 85 failure records (these are not a count of distinct failing
test cases). iOS validation finished with exit 65: its result summary reports
517 tests, 488 passed, 20 failed and nine skipped. Real-model acceptance remains
pending. The Rust PTY subset finished with 45 passed and 36 failed. These failures require diagnosis, not blanket
timeout increases or restoration of retired storage/protocol paths.
The latest Mac launch also exposed explicit OS local-network policy denial;
the source now declares the usage description and distinguishes that denial
from ordinary offline errors. The rebuilt Mac application has not yet been
fully validated, and no system authorization settings were changed. The four
`MacNetworkFailureTests` subsequently passed, including built-app usage metadata.

The running Mac suite exposed an actual producer admission-order defect:
preflight `UpdateSessionConfiguration` commits an idle session and publishes a
permission projection before `begin_core_turn` admits the requested Run. Mac's
exact-Run check correctly rejects this event. Fix publication lifecycle binding;
do not remove the check, retry until admission happens, or treat an older active
Turn as proof of the new Run. The Rust batch also found an exact replay mismatch
in the pending model request's `attempt_started` flag. r5 now isolates pre-admission
transport publication and admits the typed model-attempt mutation into the same
Core/checkpoint transaction. Neither correction has been revalidated yet.

r5 also repairs parent-task Claim confirmation: enqueue is not application.
Only the exact persisted operation and matching owner/input/token/lease permit
execution. Revision-unchanged confirmation reads only metadata, not a cumulative
checkpoint. Rejected queue operations still have no durable rejection receipt;
they fail closed at the existing confirmation deadline rather than authorizing
execution. That limitation remains explicit.

Fixture corrections migrate three real CLI/desktop tests from cumulative SQL
assistant bodies to paginated public journal snapshots/ranges; two fixed-ID
Application fixtures now declare their public journal parts in the same source
transition UoW. The Web HTTP fixture no longer parses a HEAD response's advertised
Transfer-Encoding as a body. None of these changes is a passing-test claim.

Mac validation also exposed a journal-only presentation wiring defect:
`MacCanonicalConversationRepository.projectMessages(window:)` filters completed
assistant rows without inline Parts before attaching their journal ownership.
Its load, async load, archive, paging and observer paths do not all preserve
journal references. The fix must share the ownership-aware window projection
across those paths, not restore cumulative inline text or weaken missing-owner
validation. Separately, tests that directly inspect a journal-only message's
inline `text` must use its bounded journal reader to assert actual content.

After the Mac suite finished, its platform-only correction batch now implements
that ownership-aware path for load, async load, archives, paging and observers;
Workspace retains the window's journal-backed rows. Tool-progress fixtures now
include required metadata, and body/tool assertions use actual journal reads.
The remote execution stream also closes immediately after verifying its exact
durable terminal, so trailing frames cannot reopen its presentation or permission
lifecycle. These corrections and their regression fixtures have not been rerun.
Rust sources remain frozen for their still-running first batch. Following its
completed validation, iOS fixtures and identity preparation are now being
corrected together; they have not been retested.

### Unreleased protocol cutover: no compatibility path

The owner explicitly requires removal of obsolete interfaces before release.
The current batch removes the cumulative running/terminal assistant protocols,
full Core snapshot transport, remote snapshot mirror authority, their commit/query
commands, and the replica assistant replacement bypass. Old packets are not
accepted as hints or fallbacks. Existing user databases are not deleted.
Application service protocol is 48 and Store schema is 52; incompatible earlier
experimental seal shapes are rejected explicitly, not interpreted as completion.

The replacement has three separate durable contracts:

- Message admission freezes exact Run/root ownership and source message order.
  User/assistant declarations use a hash chain, 16 KiB UTF-8 chunks, fixed
  frontiers and bounded pages. Receivers stage independent chunks and admit a
  declaration only after its complete digest validates. Source and recipient
  ordinals/timestamps are not conflated.
- Assistant journals carry committed content. Known-owner deltas do not refetch
  admission metadata, and received journals cannot grant tool/permission authority.
- Terminal seals require both an admission frontier and every assistant journal
  frontier. Each message retains its own terminal state/time; the Run has a
  separate terminal state. EOF, traces and notifications cannot complete a Run.

Mac, iOS, Android and Web consumers and fixtures are being migrated together.
Steering must preserve the interleaved user/assistant rows without creating a new
submission. Historical recovery reads visible-window owners and current heads,
which may advance beyond a frozen terminal seal, without reopening the Run.
The batch has not yet passed unified compilation, regression or real-model
acceptance. Android cold process recovery does not automatically resubscribe
to a still-running remote execution stream; read-only committed-state recovery
is separate from that capability.

Read-only audit during validation found that this recovery gap is broader than
missing stream reattachment. Android restores a live processing state and then
uses the input-blocking gate to reject its own recovery callback; its visible
window recovery also skips Runs without a terminal seal and depends on existing
journal heads. Web cold selection clears its active turn and reads history once,
without restoring active-Run observation. Existing `/agent/v2/resume` cannot
attach to a still-running worker: it returns `409 run_in_progress` instead.
These are required follow-up fixes, not optional UI polish. Recovery must use
exact durable Run/root identity, a generation-fenced observation lifecycle, and
re-observable permission/control state without creating or replacing execution.

#### Required observation closure (implementation landed; acceptance pending)

Use one authenticated, mobile-encrypted read-only Run observation contract for
all clients. Discovery returns only the durable active Run/root; subsequent
reads require exact Host, Client, conversation, Run and root identities.
Observation returns bounded admission/seal/control watermarks and currently
valid permission challenges, never checkpoint bodies or cumulative messages.
Existing journal readers remain the content path.

Before exposing worker or challenge state, bind the worker registration to the
Application-admitted Run/root/client and execution generation. A conversation-only
registration cannot prove that the requested Run is executing; before admission
it is only starting/unknown. The ordinary permission resolver, not just resume,
must retain this binding. Re-observation of a challenge requires the same owner,
an unexpired nonce, a live waiter and a still-pending Core permission request.
Never recover authorization from trace records or issue a fresh decision on read.

Visible-client observation and new-input eligibility are separate state-machine
guards. One generation-fenced observer catches up admission, current heads and
the terminal manifest; cancellation on switch/background and bounded retry
backoff prevent stale callbacks and duplicate observers. Resume remains an
explicit execution recovery operation; an existing worker or a racing 409 means
observe, not replace. Worker absence alone is not effect replay authorization.

Acceptance must cover cold open before the first journal record, active streaming,
permission wait, worker replacement, disconnect/reattach, verified terminal and
late callbacks after selection changes on each client. iOS's current one-shot
remote recovery followed by local-only observation is being replaced in this
batch. All four client implementations and server observation are now frozen
for the r5 validation recorded above. Old trace-driven Mac execution catchup is removed, not retained
as a fallback; diagnostics must not reintroduce it as permission authority.

The implementation now writes a typed baseline or replay update. Eligible
streaming commits persist normalized inputs and a current delivery witness
without rewriting checkpoint bodies; cold readers reconstruct state from the
Run-bound baseline and ordered input tail. CLI live publication uses journal
notifications instead of cumulative assistant projections. These are implemented
paths, **not a declaration of acceptance**. The dated notes below record the
development sequence and may describe earlier intermediate states.

The first unit/binary validation found real defects as well as stale fixtures:
SQLite witness triggers conflicted with outer checkpoint UPSERTs, and witness
projection used a Rust member name rather than Core's serialized
`pending_host_effects` field. Both have been corrected. Store schema 50 refreshes
the view/triggers and repairs baseline witnesses while preserving witnesses
newer than a replay baseline. A contract regression compares actual Core JSON,
SQLite projection, and incremental witness serialization; migration coverage
checks that a current replay cursor is not overwritten by the old baseline.

The port-only integration fixture now supports rollback-capable journal
publication rather than bypassing it. A new consolidated Core/Store/Application/
Service/CLI run includes integration targets and uses loopback proxy exclusions.
Compilation succeeded. Application unit tests passed 281/281, including the
Core/SQL/incremental witness contract; the port-only effect/publication suite
passed 5/5. CLI main unit tests passed 1485 with 1 failure and 2 ignored. The
full integration run finished with 18 failing targets and is **not passing**: failures
include old cumulative-protocol assertions, incomplete ownership/admission
fixtures, source-vs-local revision expectations, and a replay equality check.
The latter exposed a genuine deterministic-state issue: reducer trace generation
read the current wall clock instead of the admitted observation time.
That correction is now implemented but not yet revalidated; the replay fixture also incorrectly rebound a
newer delivery cursor before replaying the baseline. PTY tests clear the process
environment, so loopback proxy exclusions must be set inside their harness too;
this is not assumed to explain every PTY failure. The service-process suite passed
11/11 but took 1,010 seconds; a live sample identified the hot-path issue below.
Store unit tests passed 323 with one historical disclosure assertion failure and
one ignored test. Corrections and the remaining cutover are being collected before
the next unified test run. macOS compilation/UI checks, real-model
acceptance are not claimed complete. The terminal aggregate path has since been
deleted in the current unvalidated batch.

### Current correction batch (not yet compiled or accepted)

Sampling the live service long-stream integration test identified a second hot
path: the command-receipt wrapper loaded complete conversation projections before
and after each Host response. With replay-backed checkpoints those presentation
reads replayed the entire committed input prefix even though the reducer was warm.
The legacy projection diff therefore reintroduced quadratic recovery work outside
the otherwise incremental commit path. This is a producer boundary bug, not a
reason to relax recovery validation or increase timeouts.

Core response/control commands now use revisioned conversation invalidations;
their assistant content remains owned by the transactional assistant journal.
They no longer load full projections merely to compute that second live diff.
The real-service 2,048-delta regression additionally checks that its conversation
feed contains only bounded invalidations, including completion. Cold presentation
reads still reconstruct the exact current state; they are not allowed to read a
stale baseline for speed. The completed run used the previous executable,
so it does not validate this correction.

Web consumption was also found incomplete: only the paired-mobile journal read
endpoint existed, while the Svelte client still consumed legacy Part snapshots.
The current batch connects authenticated browser journal reads and the actual
chat/model-test consumers. The earlier staged-retirement policy is superseded by
the owner's unreleased-project decision: old producers and interfaces are deleted
in this batch, and all consumers must pass the replacement contract together.

## Implementation chronology

Unvalidated recovery update (2026-10-07): the SQLite recovery read now pins the
checkpoint, its explicit Run-bound replay prefix, and normalized Host input tail
under one snapshot. Both cold Application reduction entrances rebuild Core from
this bundle before binding current outbox delivery facts. Replay consumes the
original decision timestamps, checks contiguous Run-wide ordering and frozen
request identity, and discards returned effects/publications rather than sending
them again. Missing reducer inputs, gaps, and regressed replay heads fail closed.
Store regression assertions have been added but not run. This connects the
recovery reader; it does **not** implement incremental checkpoint writes: current
transition writers still serialize full checkpoints and normally leave no tail.
The baseline/tail write contract and consolidated validation remain outstanding.

Cold task admission, task guidance/terminal checks, cancellation preparation,
and the LoadCheckpoint command now use the same recovered-state adapter. The
reader checks the reconstructed event cursor against the committed frontier.
Store CAS revisions count transactions, whereas Core revisions count reductions;
these must not be compared for equality after batching. Cancellation's old
equality check was removed in favor of Core envelope/state validation. A pending
real-SQLite fixture substitutes an earlier baseline under the current frontier
and calls the production LoadCheckpoint command to compare reconstructed state
without appending replay inputs. Session UI reads now capture a typed local
recovery bundle, frozen invocation configuration, and required delegated-session
evidence in the same SQLite snapshot used to select local versus remote
authority. Local projections invoke the same replay reducer as cold execution;
remote mirrors retain their own authority and do not use local replay. The
pending real-SQLite regression also compares the production session projection
against the fully committed Core state. Remaining direct baseline consumers
still need cutover before incremental writes can be enabled.

Startup reconciliation now reads the same typed checkpoint/replay bundle inside
its existing write transaction. It restores and replays before comparing the
current Turn, pending outbox, model attempts, and delegated permission records;
an explicit replay owner from another Run is rejected. The pending repository
regression covers a nonempty tail with a distinct Store CAS revision and verifies
that a foreign Run owner produces no recovery plan. This is still unvalidated.

Cancellation delivery now captures the same local recovery bundle with its
outbox and delivery attempts in one snapshot, and uses the shared cold
materializer before admitting cancellation. It validates the cancellation's
frozen Core identity/payload, the outbox row identity, and the cancellation
attempt's Run/request ownership. Target envelopes use Core's common validated
request accessor instead of separately decoding an unchecked snapshot field.
The isolated SQLite corruption fixture now covers session, request, effect,
Host, lease, Turn, and target substitutions; these assertions are pending the
consolidated test run.

Assistant/maintenance Turn admission, idle submissions and revert, memory-worker
readiness and planning now use recovered checkpoint reads as well. The two Turn
admission paths use the transactional memory-usage ledger instead of cloning and
parsing the previous checkpoint to diff usage. Their old helper references were
removed. The pending end-to-end fixture now starts a new Turn after reading the
completed baseline-plus-tail recovery, covering execution continuation as well
as display. Exact-Run invocation proof reads now capture the typed local recovery
bundle under the same read snapshot as their Run/Turn ownership witnesses. They
materialize through the shared cold reader and hash the reconstructed checkpoint
returned by LoadCheckpoint; a replay Run or frozen-config mismatch is rejected.
The separate raw checkpoint payload read was removed. Removing full checkpoint
writes still requires changing the write contract and derived delivery witnesses;
it is not yet safe or complete. All of these edits remain unvalidated.

The transition write contract now explicitly distinguishes Baseline from Replay.
The Store Replay branch advances only checkpoint metadata and the supplied
delivery witness while retaining the original baseline/prefix, in the same
transaction as reducer inputs and publication changes. It requires an exact Run
baseline, contiguous normalized input history, and unchanged Core session
ownership. Schema 49 prevents metadata-only updates from parsing the full
checkpoint to regenerate delivery witnesses. Application now selects Replay for
recorded model/tool streaming progress when an exact baseline and current
delivery witness exist. Readiness uses durable metadata, not checkpoint bodies;
after 4096 recorded inputs it refreshes the baseline to bound the replay window.
Control/terminal boundaries and checkpoint-only external commits still write
Baseline. The narrow witness serializer includes pending-effect identities,
response cursors, and permission continuation identities, excluding transcript,
model request bodies, and permission arguments. The real-SQLite cold-delta test
now installs a trigger that rejects full checkpoint writes during the stream
commit, checks unchanged baseline bytes and one recorded replay input, then
reopens storage and completes the response. This production cutover and its
regression assertions are not compiled or tested yet; they are not acceptance
evidence. Remaining direct storage/client readers must still be audited.

Presentation subset, context-usage, permission-panel and remote assistant
snapshot reads now accept an Application-reconstructed checkpoint when durable
tail metadata indicates replay is required. Without a tail they retain their
bounded SQL subset path. Store pins a read snapshot and checks the supplied
conversation/revision/cursor before filtering it; a caller that supplies no
current checkpoint while a tail exists is rejected rather than displaying the
baseline. The cold-delta regression now exercises local/remote presentation,
context usage and permission reads before completion. This does not eliminate
the legacy aggregate publication protocol or prove its runtime performance;
these new assertions remain unrun.

The CLI Host live publisher no longer queries or queues RemoteAssistantProjection
at stream, control, guidance, admission or terminal boundaries. Its queue contains
only committed journal targets and reconnect seeding state; the cumulative
projection emitter and its deduplication field were removed. The renamed journal
notification publisher drains targets before their corresponding Core events.
Explicit snapshot queries and the separate terminal conversation envelope remain
and must not be mistaken for eliminated protocols. The oversized-stream test now
asserts journal frames and absence of the retired live cumulative event through
the terminal boundary, rather than expecting a legacy snapshot read to fail.
These changes are pending consolidated validation.

Status (2026-10-06): production journal writes, paired-host HTTP reads and live
connected-Mac notification ingestion are connected; the visible UI read-model
cutover is not. This supplements
`COMMITTED_ASSISTANT_JOURNAL_DESIGN.md`.

Current unvalidated batch (2026-10-06): stream parsing now requires the expected
conversation and the immutable configuration captured when connecting. Journal
notifications are checked against that conversation before any Store operation
or HTTP request; ordinary decoded events are checked against it as well. Journal
catch-up no longer looks up a possibly changed global/run configuration. The
unused per-record entry point that re-resolved that configuration was removed.
The ingestion fixture now includes an otherwise valid notification arriving on
another conversation's stream and a settings edit pointing at a different URL
under the same Host name. These changes and new assertions are intentionally
not compiled or run yet: validation is deferred until the requested complete
implementation batch is ready. Earlier acceptance counts below describe the
earlier tested source, not this batch.

The pending Mac reader also displays committed Task progress and bounded body
fields. Legacy task cards retire only after adoption of an exact Task ID under
the same journal owner; Task IDs and Part IDs are separate namespaces. Guidance
and cancellation still use the existing conversation-bound admission callbacks,
not authority synthesized from display metadata. Exact-ID handover regression
assertions are written but not run. Task control eligibility and durable
Run/Host binding still require audit before this cutover is considered complete.

Task-control HTTP acknowledgements now require the requested conversation and
Task IDs before decoding their optional projection. A missing or JSON-null
projection is accepted as absence of display data, matching the Host's 202
response contract; malformed projection data still fails. Present task handles
must match their dictionary IDs and acknowledged conversation. This does not
prove command application or task completion, and does not yet bind this legacy
control response to an immutable Run/Host receipt. Regression cases are pending
the consolidated validation run.

The same pending audit separates durable task-command acceptance from the
process-local wake: after successful FIFO enqueue, registry/Host drain errors
no longer report submission rejection. The committed queue remains the recovery
source; this does not assert that the command has been applied. Cold remote
resume also installs its captured connection selection into the per-conversation
target map, as submission already does, so later task requests do not consult
edited global settings. This closes settings drift after selection, not durable
Host/Run provenance validation before selecting a cold-recovery target. Both
changes remain uncompiled and need integration coverage in the final batch.

Cold remote resume now first resolves the active Run's assistant identity and
uses the existing exact recovery inspection to require its persisted source
Host to equal the selected connection's expected Host. This check runs before
fetching remote settings or dispatching resume. Missing/mismatched/local/terminal
observations are not remote-resume authority. Store-backed and decoder regression
assertions cover matching, wrong, empty and absent Host provenance and terminal
observations, but have not been executed. The Host must still admit the exact
Run at dispatch; this read check is not a lease or execution authorization.
Resume preparation also retains its tool plan locally across the settings HTTP
await, checks cancellation and unchanged stream owner/active Run afterwards,
then adopts connection, plan and stream owner without suspension. A late settings
response can no longer overwrite a newer submission's plan. This concurrency
change still needs a delayed-response integration fixture and consolidated tests.

CLI submission presentation now borrows the committed snapshot when its caller
must retain/return it. Idle, observed-idle and active submission paths no longer
clone the full snapshot for presentation; the ordinary owned runtime output uses
the same borrowed renderer. Journal publications still precede each emitted
event/trace/snapshot, and the source regression now exercises the borrowed path.
This does not remove legacy full projection serialization or its recovery seed:
a fresh transport may still need that seed even when the Run is unchanged.
These changes remain untested with the rest of the batch.

Committed runtime restoration now returns only success/failure and moves the
validated snapshot into runtime ownership. Callers that only adopt a checkpoint
no longer pay for an unused full snapshot return; the execution API that actually
returns a snapshot retains its explicit copy. Presenter identity setup, terminal
status, context-window and active-goal checks borrow the existing snapshot. The
conversation-revert path also moves its accepted snapshot instead of cloning it.
Existing atomic-restore regression assertions inspect runtime state directly
and remain applicable, but are not yet rerun. This removes redundant copies,
not checkpoint reconstruction or persistence serialization, and does not claim
post-commit adoption failure fencing is complete.

The pending CLI runtime now explicitly fences effect dispatch and memory-worker
planning after an attempted committed restore fails. It also sets the fence
before effect draining, since a later delivery/checkpoint-read error can follow
a successful durable commit. Only successful checkpoint adoption clears this
process-local fence; the prior snapshot remains readable but is not execution
authority. Atomic-restore regression assertions now attempt dispatch and memory
planning after rejected adoption and require both to fail, then verify that a
valid committed adoption clears the fence. These assertions are not run yet;
this does not cover all Host driver APIs or Store settlement uncertainty.
Submission configuration access now obeys that same fence rather than returning
stale config to admission/reduction paths. Reacquiring a cached Host recovers
through its original SessionStore before reuse, verifies the Core session ID,
and adopts the committed checkpoint; failed reads/adoption retain the fence.
This applies to retained ephemeral stores too, without substituting an empty
runtime. The stale-config assertion is added; cached-Host recovery integration
coverage and the consolidated test run remain pending.
The pending regression now commits a configuration change through the retained
ephemeral SessionStore, injects failed in-memory adoption, verifies a missing
original Store leaves the fence intact, and reacquires the identical cached Host
to compare its recovered checkpoint with the actual commit. It has not run.
Warm exact-Run resume, scheduled drains and task-inbox drains also perform this
recovery before using runtime state; inbox recovery precedes claiming commands.
Fenced-runtime recovery now uses a SessionStore recovery read that verifies its
retained Run/config digest against the active canonical Run and reuses the exact
invocation/checkpoint witness validation already used by cold resume. An idle
recovery rejects nonterminal Core state or pending effects without a canonical
Run, rechecks that no Run appeared during the read, then retires the stale
process-local active-turn pointer. These are fail-closed observations, not a new
execution lease or atomic multi-read snapshot. This path remains untested.

The pending Mac journal reader now renders source/citation Parts directly,
including inline destinations, page/line/section facts and bounded field-backed
titles, URIs, snippets and evidence. Evidence unwraps its versioned envelope only
after complete reading; incomplete URI fields never become clickable links.
Inline destinations reuse the existing provenance-gated artifact opener, so a
remote file URI is not interpreted as a receiver-local file. Old source rows
retire only after exact source/citation ID adoption under the same full journal
owner; the two ID namespaces remain separate. Handover regression assertions
are added but not executed. Citation byte-range navigation, field-backed link
opening and remote artifact retrieval remain incomplete.
Field-backed source/citation URI navigation is now connected for complete fields
within a fixed 32 KiB budget. Range validation must prove EOF before the value
becomes a destination; the resolved value is tied to its original read reference
and cleared on version change. Larger fields stay in the explicit body reader,
never a truncated link. Budget/invalid-length tests verify rejection before
Store opening and remain unrun. Remote artifact retrieval and citation range
navigation are still pending.
Unchanged source URI reads now key reuse by database, complete journal owner,
Part and complete content reference, rather than the message cursor. Unrelated
message commits no longer reread that URI or hide its link. Owner/database/root/
generation changes invalidate reuse; corresponding assertions are written but
unrun. The root reader still enumerates the snapshot Part directory on each
notification: `ReadAssistantJournalParts` exposes full directory pagination, not
changed-Part pagination. Removing that cost requires a Store-backed delta read;
this field-level fix is not claimed to complete incremental projection cutover.

StorePort now exposes snapshot Part changes between a verified retained base
cursor and a pinned target snapshot. It returns the latest target head for each
changed Part, including explicit `None` tombstones, with bounded page payloads
and stable Part-ID pagination. Foreign/reversed/expired cursor windows fail.
This is the first implementation step of directory cutover: Application/wire/Mac
integration, a sequence-range index and regression coverage remain pending.
The UI still uses the full directory read; no performance or cutover completion
is claimed for this untested Store addition.
The change page is now exposed through the read-only Application command and
the Mac Application port. Its wire envelope carries exact base and target
cursors. Transport budgeting retains explicit retirements and does not advance
past trimmed entries. Service protocol is 45. Visible-reader integration,
index work and regression coverage remain pending; no builds/tests have run.

The same unvalidated batch adds bounded local Run journal discovery to the
StorePort/Application and paired-host `heads` operation. It selects only the
current source epoch and exact locally authored Run, rejects imported Runs and
wrong conversations, validates each returned message's read provenance, and
pages at most 64 cursor values without decoding message bodies or checkpoints.
The pending schema is now 44: it adds the owner-tuple expression index and a
nullable exact Run binding on remote display mirrors. Service protocol is 46.
Schema 44 additionally indexes Part versions by owner/sequence/Part. The Mac
visible reader now uses change pages for same-owner advances, validates base/
target and pagination, applies explicit retirements, and atomically adopts the
fully read result in canonical section/position order. Initial attachment,
owner/database changes and explicit retry still use a full directory snapshot;
successful retry resumes incremental reads. Failures retain the previous view.
Migration/query-plan assertions are written; behavioral delta/retirement tests
and consolidated validation remain pending. No current-batch test has run.
The Store behavioral fixture is now written: actual committed declarations and
retirements are read one entry per page, then compared against a pinned target
despite a later removal/new Part. It checks explicit tombstones, unchanged-window
emptiness, wrong-owner/reversed windows, insufficient byte budgets and an expired
base. It has not been executed. Mac/Application end-to-end delta coverage and
the consolidated validation run remain pending.
Application change pagination now has a cold-reopen, actual encoded-byte-budget
fixture alongside full-directory pagination. The encrypted Mac ingestion fixture
retains the initial publication cursor, reads that snapshot and advances it via
the real Mac change reader, comparing ordered IDs, metadata/content revisions and
body references against a full final snapshot. Cancelled unchanged-cursor updates
are also rejected. These are written integration assertions, not executed results;
no current-batch compilation or tests have run.
Change-page decoders now require explicit nullable `head` and `next_part_id`
keys. Missing head data is not a retirement, and a missing continuation field
is not end-of-page. Mac additionally rejects empty/mismatched change IDs. Rust
and Swift malformed-wire regression assertions are added and remain unrun.
Host-response cancellation routing now queries Core turn status and the exact
pending effect's cancellation state directly. It no longer constructs up to
three full presentation snapshots plus a manual checkpoint merely to evaluate
those control predicates. Snapshot production reuses the same status query.
A basic read-only state/revision equivalence assertion is written but not run;
existing cancellation integration coverage must also run in consolidated
validation. Persistence checkpoint generation remains, so this is not the full
hot-path checkpoint removal.
Memory-usage accounting no longer deserializes the entire previous typed Core
checkpoint merely to collect usage IDs. A narrow serde projection reads only
`state.memory_runtime.usage_ledger[*].usage_id`, retaining the existing defaults
for absent memory accounting and rejecting malformed JSON/missing usage IDs.
Full recovery/admission validation remains separate and unchanged. A real typed
checkpoint fixture checks deduplication/new entries and malformed input; unrun.
The checkpoint JSON is still loaded and scanned, so this removes full-state
allocation, not the underlying persistence read or serialization.
The obsolete `successor_requests` channel has been deleted from Runtime's
committed acceptance type and all CLI/test adapters. Runtime already discarded
it and claims executable successors through the committed outbox instead; the
presentation acknowledgement no longer transports a second request list. The
Application acceptance still contains this field and still performs its extra
read, which must be removed next with its API assertions updated to inspect the
outbox. This deletion is uncompiled/untested with the pending batch.

That Application follow-up is now implemented: Host-response and active-submission
acceptances no longer return `successor_requests`. Host-response acceptance no
longer rereads committed delivery/checkpoint solely to populate that unused
return field. Active submission moves its internal request list instead of
cloning it; requests used to commit outbox mutations remain intact. Promotion,
cancellation and effect-resume assertions now inspect durable outbox state and
leased delivery rather than treating an acceptance list as execution authority.
Service protocol is 46. All these changes remain uncompiled/untested.
Continuation construction now runs only at a persistence boundary and moves
the already validated request into the outbox input; transient deltas no longer
clone full model requests for a discarded return list. Core pending-request
decoding also borrows its stored JSON value instead of first cloning that whole
tree. A frozen-payload/cursor/nonmutation regression is added but not run. This
does not eliminate typed request decoding or persistent outbox request bodies.
Committed outbox delivery now decodes a narrow checkpoint witness (outer/inner
Core session IDs and pending effect request IDs/response cursors), not the full
AgentThreadState. The same atomic repository read and all existing Run/effect/
request identity, model-attempt and delegated permission binding checks remain.
Full recovery still validates Core state separately. Projection shape, exact
u64 cursor and required-request-ID assertions are written but unrun; existing
delivery integration tests must run in the consolidated batch. The full JSON
read/scan remains, and Core still eagerly builds transition snapshots before
Application suppresses delta-only snapshots; neither cost is claimed removed.
Mac trace recovery invokes
discovery and journal catch-up before returning its replay batch, using the
configuration captured for that request. Existing cumulative projection replay
is still retained for the old visible reader; it is not claimed removed.
Discovery is a current observation, not an atomic multi-owner terminal seal:
concurrent owner creation still needs the live stream/final completion protocol.
New migration/query-plan, local-source/foreign/imported-source, encrypted HTTP
discovery and Mac wire/recovery assertions are written but deferred with the
rest of this batch. None of the earlier test counts validates these changes.

The pending source-adapter changes also forward publication sets from idle
submission and recovered task guidance, where an active Run is not a valid way
to infer the changed assistant owner. Guidance validates the current Host Run
before enqueuing its accepted cursor set. A JSON presenter seeds an empty
process-local queue from the exact active Run's committed head pages once per
Run; rebuilding the presenter does not repeatedly enumerate unchanged heads.
This seeds notifications only: no reducer output, checkpoint or transcript is
converted into journal records. Read failures remain presentation/recovery
errors, not a rejection of an already committed model/tool response. The added
CLI regression clears the notification queue over a real accepted Store Run
and checks exact cursor restoration and no duplicate seed on presenter rebuild;
it is not a killed-process or real-model test and has not been run yet.

Mac's existing Part-page DTO now decodes metadata through an exhaustive typed
12-kind enum instead of opaque JSON. Text/resource values retain their
inline-versus-field distinction; tool provenance, permission display, exact
full-output owner, source/citation/attachment facts, task/skill/attempt fields,
review visibility and historical schema/revision remain explicit. Counter
strings and task progress binary64 bits are not routed through JSON Double.
This changes the actual native reader contract, not canonical storage and not
execution authorization. All-kind wire fixtures were added but are unrun with
this batch. The UI still needs to consume these heads and body references;
typed decoding alone is not a visible-reader cutover.

Pending Mac text-view integration now passes committed cursors through local
decoding, remote ingestion and recovered replay to Workspace's per-message
presentation bindings. The binding is outside canonical message overlays so
ordinary metadata refreshes do not erase it; it is cleared on conversation
deletion/cache eviction, and changed ownership must match the observed Run.
Its database URL comes only from the repository and is excluded from Codable
messages/archives. The assistant response view uses fixed-cursor Part pages and
per-Part Markdown body references when this binding exists. A body preview is
limited to two 32-KiB ranges; explicit reading loads further ranges on demand
against a frozen selection. No content is written back into canonical messages.
This is a partial **visible text** cutover, not the final all-kind UI cutover:
historical text uses its existing schema adapter; tool/activity/source/task
controls and copy/export still use existing canonical presentation, cold message
loading still needs journal discovery, and the cumulative transport is still
present. The new view reader is included in the ingestion fixture, but neither
it nor this source batch has been compiled/run. Earlier counts do not apply.

The pending reader also adopts Part heads and their exact cursor together, so
a newer notification cannot pair old body references with a newer snapshot.
Head reads reject foreign identities and duplicate Part IDs; range reads check
UTF-8 byte geometry and exact end-of-content. Cancellation reaches detached
reads, and dismissing the explicit reader cancels its manually requested page.
Already-cancelled callers are rejected before creating a worker. The ingestion
fixture includes cancellation assertions; these remain unrun with this batch.

Cold-window integration is now pending in the same batch: Store includes
`assistant_journal_heads` for the bounded selected messages under the window's
read snapshot. Current canonical Run ownership and admitted remote Host/epoch
provenance determine these references; absent journals do not manufacture a
binding. Application carries them through both local and remote windows. Mac's
actual history-window loader binds them to its own database URL, including after
eviction/reopening, rather than waiting for a new live notification. Local and
remote Store-window assertions and Mac cold-port restoration assertions are
written, not run. This closes the planned cold-window text binding gap; it does
not remove cumulative transport, canonical body previews or checkpoint reads.

The pending Runtime pump change removes its `latest_snapshot` accumulator and
the optional full-snapshot return value. Production callers ignored that return
and either read committed state separately or inspected durable cancellation
state. Acceptance now moves each snapshot to the existing publisher once;
boundary submissions share that same publication helper. There is no additional
full snapshot cloned/retained by the pump after every accepted delta. Runtime
terminal/boundary tests inspect the published committed output, and the silent
Host fixture checks no snapshot publication. These changes remain uncompiled
and unrun. Snapshot production, legacy cumulative publication and checkpoint
serialization still exist; this removes the redundant pump copy, not those
remaining paths.

Application mirror persistence now consumes the command-owned snapshot and
redacts the capability catalog in place, instead of cloning the entire snapshot
before serialization. The terminal completion decoder similarly consumes its
owned checkpoint JSON value. No authority or cursor semantics change here;
tests that intentionally reuse a fixture explicitly clone only their fixture.
Existing catalog-redaction, SkillRun-evidence preservation, permission-ledger
and reopening assertions remain in the deferred test batch. This still does
not eliminate full mirror serialization or full checkpoint persistence.

Remote mirror ordering now persists `run_id` with both running and terminal
writes. Event cursors are compared only within that exact Run. A new admitted
Run may begin at a lower cursor after its bound predecessor is terminal; late
writes from the completed predecessor still fail admission before comparison.
Host changes still require explicit handoff. All supported schema migrations
add the nullable binding without guessing ownership or rewriting legacy mirror
bytes; such unbound display rows do not impose an ordering floor on the first
exact admitted write. Schema 42 migration and reopen/cross-Run cursor-reset,
same-Run stale/conflict, late predecessor and terminal-binding assertions are
added but unrun. This repairs mirror ordering, not cross-Host handoff or the
remaining full-snapshot wire format.

Read-side Run isolation is also pending: active successor admission hides
predecessor remote runtime metadata and context usage even before the first
successor mirror arrives. Switching the mirror's Run atomically removes the
predecessor's derived terminal projection cache (canonical transcript and
permission grants are untouched). Missing terminal metadata yields no remote
usage input instead of a JSON object containing null identity fields. Permission
display selection excludes a prior Run's mirror and excludes local checkpoint
permissions while an active remote Run owns the conversation. Added assertions
exercise the pre-first-mirror gap and missing-cache path; still unrun.

Mac permission presentation now retires its cached projection when a successful
canonical read returns none, on conversation deletion/eviction, and when a new
submission establishes a new execution generation. It does not reset on resume
of the same Run. This prevents a predecessor's high permission revision from
rejecting the next Run's initial projection, and prevents missing canonical
state from leaving an old pending request visible. A production-send-path test
seeds revision 100 and asserts the new submission clears that cache; unrun.

Within that permission scope, equal revisions now require equal typed content.
Exact duplicate delivery remains admissible so a delayed challenge can attach
to its already accepted projection; conflicting equal-revision content cannot
clear or resurrect a pending request. Both challenge-first and projection-first
fixtures include conflict assertions in the deferred test batch. Tool/process
UI still uses the legacy trace detail model; it has not been removed or claimed
covered by the journal text-reader cutover.

Reasoning presentation is now pending on the actual journal view path. Ordered
reasoning Parts render a collapsed activity row; expanding it reads title fields
and reasoning summaries through the same fixed-cursor bounded range reader as
text, with explicit paging for longer content. Journal-bound messages no longer
paint duplicate `.thinking` trace rows or the legacy aggregate pending-process
fallback. Tool/plan/status traces remain until their full details are migrated.
Reader request identity now includes the database and content field as well as
owner, Part and immutable content identity. Metadata fixture assertions were
extended; UI rendering and this batch remain uncompiled/unrun.

Step Parts now share the committed activity reader. Rows with no detail do not
offer an empty disclosure. Activity handover no longer filters a broad trace
kind: only exact Part IDs successfully loaded for the current full journal
identity are retired from legacy trace display. Coverage retains retired IDs
within that owner so old trace snapshots cannot resurrect removed Parts; it
resets across owner changes. Unmigrated tool/status/history traces remain. The
coverage reducer is used by the real message view and has new identity/removal
fixtures, not yet run.

Live remote terminal-frame decoding now performs exact-Run journal discovery
and catch-up against the captured transport target before returning the terminal
event. Previously only notification ingestion and disconnected trace replay did
so. Catch-up failure withholds terminal delivery for recovery; it does not mark
an empty answer complete. The loopback fixture now observes a discovery request
on the production terminal parser path, with no redundant chunk transfer when
already covered. This is not yet an immutable terminal watermark/owner-set seal:
discovery remains a current Store observation and the fixture source is not a
real-model terminal run. Tests remain deferred with the batch.

The terminal catch-up boundary now also requires the stream's captured Run
identity (initial submission or resume owner) before any discovery request.
Committed projection/journal/terminal envelopes require an explicit matching
source Host before transport enrichment; a foreign or missing declaration is
not overwritten into a trusted identity. Fixture assertions cover missing and
wrong Run, missing and wrong terminal Host, and zero discovery requests on all
these rejections. Journal owner identities themselves remain independently
validated by Core, rather than being rewritten to the outer active Run.

Source journal notifications now follow actual JSON-presenter demand. A plain
terminal session with no consumer does not retain notifications for every past
assistant owner. An unobserved committed publication clears the process-local
seed marker, so the next subscriber discovers durable heads even when rejoining
the same Run. This preserves durable recovery while removing an unused queue;
it does not remove the old cumulative projection still consumed by other UI
paths. Plain-CLI and same-Run detach/commit/reattach fixtures were added, unrun.

Mac's existing dynamic canonical JSON path now preserves signed/unsigned
64-bit integers outside binary64's safe integer range instead of converting
them to Double. This applies to direct Codable field reads and the Foundation
bridge already used by tool results and remote snapshot envelopes. Small
numbers keep their prior representation; fractional numbers remain binary64
as in Core's public structured-value domain. New fixtures cover 2^53+1, i64::MIN,
u64::MAX, booleans, fractions, both decoding routes and semantic encode/decode
round trips. This is not lexical JSON preservation or digest verification:
immutable content bytes remain the source for journal hashes. Tests unrun.

Tool Parts now have a connected journal detail view and participate in exact
Part handover from legacy traces. Expanding a tool displays committed state,
capability/call identity, provenance, permission display facts, resources and
referenced fields. Structured input/output/resource/artifact envelopes are
version-checked and unwrapped only after a complete bounded read; partial JSON
is not rendered as prose. Larger fields require explicit paging in the frozen
reader. Plain tool output stays literal, and large integers use the exact
canonical numeric path above. No tool URI or permission display becomes an
execution grant. Structured rendering fixtures were added but are unrun.
This is not final interaction parity: artifacts currently have structured-data
details, and full-output ownership is displayed without a dedicated fetch/
preview action. Those actions, task/skill/source views, cold-history legacy
fallbacks and old cumulative transport removal remain outstanding.

Complete tool artifact fields now decode into artifact rows in both the bounded
preview and explicit paged reader. HTTP(S) targets use the existing URL opener;
local file targets use the existing Quick Look controller only after Core's
local-journal read gate proves provenance for the exact database/owner reference.
That proof is reference-bound, not a reusable boolean across owner changes.
Relative paths, remote file authorities and arbitrary URI schemes do not get
local open actions. Imported remote journals cannot open source-host file paths
on the receiver. Remote file download/full-output retrieval remain unfinished.
Destination-policy fixtures and real source/imported-journal provenance checks
were added to the deferred batch; no UI/runtime test has run yet.

Committed Error Parts now render their failure code and bounded literal message
directly in the journal view, without requiring expansion or interpreting tool
errors as assistant Markdown. Their exact legacy trace counterparts retire only
after head adoption. Failed tool rows start expanded and open on a transition
to failure so detail is not hidden by the journal cutover. `recoverable` remains
display metadata, not an automatic retry or execution authorization. Rendering
and failure-transition behavior still need the final batch UI validation.

Skill and model-attempt Parts now render on the actual committed reader path
and retire only their adopted trace counterparts. Skill counters remain decimal
strings through display, with bounded title/result/failure fields. Model attempts
retain their exact attempt/fallback identities, provider/model, recorded state,
HTTP status and bounded failure text; a retryable marker does not issue a retry.
They share the tool activity state labels instead of inventing separate terminal
inference. Metadata fixtures now include a recorded fallback identity and status.
This remains unvalidated UI source; task controls, source/citation/attachment
Part handover and full old-transport retirement are still pending.

Task control callbacks now carry the conversation captured by their transcript
view. Workspace rejects obsolete-conversation callbacks and captures execution
location before scheduling asynchronous guidance/cancellation, so a subsequent
UI location switch cannot redirect the request. A probe fixture exercises both
controls, switches location before their MainActor Tasks run, and verifies stale
cards dispatch nothing. This is not yet an exact task/Run/Host control receipt:
that durable ownership remains the responsibility of the existing task-control
ports, and task Part UI migration is not complete. Tests remain unrun.

Live notification slice: SessionStore now captures the exact committed cursor
sets returned by admission, active submission, task guidance and Host response
acceptance. Its transport queue coalesces positions only for the same complete
source/Run/assistant identity, never substitutes the outer active Run for a
changed owner, and does not advertise hot uncommitted deltas. CoreEventPresenter
drains these before existing projection/terminal event handling. Emitter sends
`assistant_journal_committed_v1` with `session_id`, authenticated-adapter
`source_host_id` and `through`; it carries no transcript or permission decision.
The native bridge recognizes it as a canonical, untraced content notification
and sets its source Host from the actual transport owner.

MacRemoteHostService's real stream parser validates the notification's Host and
conversation and awaits the existing bounded catch-up path against the same
Application port as its configuration store. A failed transfer propagates;
exact notification retry resumes durable progress. It does not create a status
row. Local Mac execution already shares the Store and ignores this notification
at trace decoding until its visible reader switches. The old cumulative
projection still supplies UI content: this slice deliberately does not hydrate
journal content back into message_parts or claim removal of the old path.
Disconnected terminal recovery, persisted notification replay, small terminal
watermarks and the semantic UI reader still require completion before cutover.
CLI's real SessionStore/Application/presenter tests pass 4/4, including exact
cursor forwarding, small constant-shape notifications and no notification for
uncommitted model deltas (`/tmp/nola-journal-live-notification-cli-final-20261006.log`).
Mac app/test-bundle build and 3 selected tests passed at Service protocol 43 /
Store schema 41 (`/tmp/nola-journal-live-notification-mac-20261006.log`). The
ingestion fixture now enters through the production stream-frame parser, rejects
a foreign Host before network reads, exercises encrypted HTTP chunk failure and
recreated-client retry, and verifies exact replay downloads nothing and never
creates an activity event. It reconstructs the 6,000-line source answer through
the pinned native journal reader. This uses isolated real Service/SQLite plus a
loopback source fixture, not an external model, the full Rust streaming router,
or rendered UI acceptance. Notification backpressure/coalescing under a slow
client remains to be measured before the old presentation stream is removed.

Part reads now use a persistent presentation-order index (Service protocol 40,
Store schema 41), not lexicographic Part ID traversal. Declare records register
immutable section/position keys in the same source/receiver transaction; removed
entries retain their cursor position. Ties use Part ID deterministically. A
continuation must name a directory entry owned by the snapshot. Existing schema
upgrades rebuild the index one committed frame at a time, retaining same-frame
Declare/Remove positions and refusing historical replacements that moved a Part.
The actual Parts endpoint uses this index plus keyed fixed-snapshot version reads;
it does not scan every version or hydrate bodies to sort. Store library acceptance:
314 passed / 1 ignored (`/tmp/nola-journal-order-store-all-20261006.log`), followed
by the expanded same-frame tombstone/migration regression (1 passed in
`/tmp/nola-journal-order-migration-final-20261006.log`). The production UI event and
read-model cutover remains outstanding.
Final protocol-40/schema-41 Application validation passed 272 library tests and
6 real stream/two-store integration tests in
`/tmp/nola-journal-order-application-20261006.log`. No Mac UI build or rendered
pagination acceptance is claimed by those Rust tests.
Application wire-budget continuation is additionally exercised with deliberately
descending Part IDs (`z`, `m`, `a`), a response budget fitting only one whole head,
and a fresh Runtime on every page. The actual read command preserves presentation
order and pinned cursor without skipping trimmed heads or treating ID comparison
as progress; every serialized response fits the configured budget. Passed
`/tmp/nola-ordered-parts-wire-20261006.log` (1 regression). This establishes the
reader boundary, not a Mac rendering or notification-consumer cutover.
The paired-host HTTP contract suite was rebuilt on protocol 40/schema 41 and
passed 2 tests (`/tmp/nola-ordered-parts-http-selected-20261006.log`), covering the
real encrypted handler, exact committed reads, rejected forged identity and the
closed read-only operation enum. The initial `assistant_journal` name filter
selected zero tests and is not acceptance evidence. HTTP fixtures still transfer
admission records; the descending-ID wire-budget case above is Application-level.
MacApplicationPort now exposes typed fixed-snapshot open, ordered Part pages,
bounded content ranges and release. Part values preserve exact string counters,
all public metadata payloads and immutable body references without hydrating
structured tool data or treating display metadata as permission authority.
The encrypted reception integration test now reads the received 6,000-line body
through these native Port methods instead of test-only JSON command dictionaries,
checks exact byte offsets and releases its snapshot. Mac build and all 3 selected
journal ingestion/transport tests passed on protocol 40/schema 41 in
`/tmp/nola-mac-journal-reader-20261006.log`. These are native read/transport adapters;
MacWorkspaceModel and the visible conversation renderer still require cutover.
Admission, Host transitions, active submissions, idle checkpoints and explicit
completion invoke the changed-owner producer in the canonical write transaction.
Normal admission declares its actual empty text placeholder; internal continuation
does not invent one. Historical Task/Skill owners resolve through durable Turn
bindings, not the current Run or latest assistant. Schema 38 permits exact idle
source stamps and proven same-conversation historical owners.

Direct Application entry points join an existing write unit or own the complete
canonical/publication transaction. The real Host-delta failure test injects a
journal INSERT failure and checks cold-reopened checkpoint, canonical text and
every journal table against their original values before retry. Publication
cursors now survive in admission, transition and idle results (Service protocol
33). Uncommitted buffered deltas carry no cursors. Receipt replay preserves the
original cursors, not newly queried heads. Client notification and bounded journal
consumers remain unwired; returning from a transaction-local producer is not
permission to notify before outer commit.

Acceptance for the result contract: the real Service restart test now seeds a
durable parent Turn and its matching Core projection. It checks admission cursors
against Store heads and idle Task publication against the original parent Run;
three Service lifetimes still preserve old receipts and require a fresh adoption
observation. Passed: `/tmp/nola-journal-cursors-service-20261006.log` (1 test).
Application admission/stream/idle/task suites passed 22 tests in
`/tmp/nola-journal-cursors-20261006.log`. These are not client-consumer acceptance.

Fixed-window record reads are now exposed through `ReadAssistantJournal` on the
same Application/Service command path. It is classified read-only, takes exact
`after`/`through` cursors, and verifies Store ownership and the digest chain.
`max_encoded_bytes` caps the JSON result (not the surrounding Service envelope);
pages may shorten only at complete record boundaries. A record that cannot fit
returns an explicit budget error, never a truncated operation or a false EOF.
SnapshotRequired is preserved instead of falling back to a cumulative message.
The real Service test reads one-record pages, rejects an advanced/forged cursor,
and re-reads the identical window after later commits and Service restart:
`/tmp/nola-journal-read-service-20261006.log` (1 passed). Production UI consumers
are still outstanding. This endpoint
returns records and content references, not hydrated bodies or tool authority.

Exact-version content reads: `OpenAssistantJournalSnapshot` pins the supplied
committed cursor, not the current head. Store validates the owner, digest and
retention floor before persisting the handle. `ReadAssistantJournalRange` reads
only references belonging to that handle/Part/version, with bounded UTF-8 output
and decimal-string byte offsets. `ReleaseAssistantJournalSnapshot` is an exact,
idempotent release; later reads fail even after reopen. Open/release join the
durable command transaction; range reads are classified read-only. The real
Application test appends newer text before opening the older cursor, reads Chinese
and emoji in four-byte pages after cold reopen, and rejects a newer content ref,
forged proof and released handle. Five stream tests passed in
`/tmp/nola-journal-range-app-20261006.log`. Client consumption and removal of the
old cumulative projection remain open.
Service test also opens an admission-placeholder snapshot, reads its range across
process restart, releases it twice and verifies subsequent read rejection:
`/tmp/nola-journal-range-service-20261006.log` (1 passed). This exercises the wire
and lifetime boundary; nonempty UTF-8/version-growth coverage is the Application
test above, not a claim that the Service fixture ran a network model.

`ReadAssistantJournalParts` now exposes the exact snapshot's bounded directory
page through Application/Service. It preserves directory continuation across
empty pages (retired or postdated entries); actual JSON-budget trimming resumes
after the last retained Part, never after omitted heads. The Service fixture now
gets its Part/content references through this command instead of bypassing the
boundary with direct Store reads. Passed `/tmp/nola-journal-parts-service-20261006.log`
(1 test) and `/tmp/nola-journal-parts-app-20261006.log` (5 tests), including an
empty first page with nonempty continuation and eventual EOF at a genesis snapshot.
These read primitives are prerequisites, not a production UI cutover. The next
integration must replace the CLI/remote bridge's cumulative projection delivery
and the receiving client's ingestion together; a cursor-only event without a
consumer would not finish the display chain.

Manifest/chunk integrity reads are also exposed as `ReadAssistantJournalManifest`
and `ReadAssistantJournalChunk`. Both use the same exact snapshot/Part/content
authorization as range reads. Manifest pages carry bounded refs and decimal-string
indices; JSON IPC returns one base64 chunk, its original UTF-8 length and canonical
digest. It never rechunks an immutable block merely to fit JSON escaping. The
production encoder's 64 KiB control-character case stays below 90 KiB on the wire
while direct JSON text exceeds 256 KiB. The real Application stream test consumes
these commands, verifies each decoded chunk against its manifest reference and
rebuilds the original owner-bound content root after reopen. Acceptance:
`/tmp/nola-journal-chunk-app-20261006.log` (59 filtered lib tests; its integration
target was filtered out), and `/tmp/nola-journal-chunk-stream-20261006.log` (5 actual
integration tests). Target-Store ingestion/authenticated source binding is still
required before replacing Mac remote projections; the read chain alone does not
authorize remote content writes or prove a UI cutover.

Receiver admission is now connected through `ReceiveAssistantJournalRecord`
(Service protocol 34, Store schema 39). It compares the adapter-supplied Host with
the already admitted remote Run/assistant binding, then pins that Run's source
epoch. It verifies the source header/batch, consecutive sequence and previous
digest, decoded chunk bytes and owner-bound references. Remote and local paths
share the atomic frame mutation kernel but have separate authorization gates;
the receiver preserves source records and never creates local publication receipts.
Record replay returns its original cursor and performs no new publication.
Frame failure rolls back chunks, manifests, heads and newly staged epoch binding.
Accepted frames advance the target conversation revision without enqueueing a
whole-assistant replacement. Authenticated network adapters are not yet connected;
a Host string supplied to Application is not itself cryptographic authentication.

Acceptance: `/tmp/nola-journal-receiver-store-20261006.log` (50 passed) covers
wrong Host, gap, changed epoch, injected record-write failure, cold replay and
source-body reads at the target. `/tmp/nola-journal-receiver-app-20261006.log`
(6 passed) includes a two-database real Application producer→receiver→cold read
chain and rejected corrupt base64 content. This is bounded record ingestion,
not full snapshot activation: large prepared replacement manifests and multi-frame
chunk staging still require receiver-side staging before enabling UI consumers.
The old Mac/CLI cumulative projection path remains active until that cutover.

Record-scoped large-content staging is now implemented (protocol 35/schema 40).
`StageAssistantJournalContent` stores bounded verified pages outside public
manifests. Its exact next-record/Host/epoch/Part/content proof cannot advance the
public cursor or expose a partially prepared body. Lost-page ACKs replay without
rewinding progress; an explicit restart discards only the exact preparation, so a
bad partial manifest cannot poison the public content generation. Final record
ingestion walks staged chunks with bounded memory, validates their digests and
content root, and publishes all Parts/cursor in the same transaction. Failure
rolls back activation while retaining previously committed preparations for retry;
success removes the consumed staging rows. Dependent content roots within one
record activate in operation order, and appends stage only new chunks rather than
copying the committed prefix.

`/tmp/nola-journal-stage-store-20261006.log`: 51 passed, including a 180,000-byte
declaration staged across reopen, incomplete rejection, corrupt preparation reset,
exact old-page replay, injected final-activation failure and two dependent appends
without retransmitting that prefix. Application's two-store fixture now calls the
actual staging command before record ingestion. Whole-assistant snapshot staging,
authenticated network adapters, UI consumer cutover and storage quota/GC remain
outstanding; record preparation is not a claim that those boundaries are complete.

The paired-host `/api/mobile/chat/agent/v2/journal` adapter now uses the existing
pairing authentication and encrypted request/response protocol. Its closed
request enum exposes bounded reads and exact snapshot open/release, never arbitrary
Application commands or remote ingestion. A source-only head query rejects imported
journals: local readability does not authorize a Host to re-export another Host's
records under its own authenticated identity. The shared request decoder also caps
the bytes read before allocating the whole encrypted body, not only afterward.

`/tmp/nola-journal-http-20261006.log`: 2 tests passed. The HTTP test uses a real
loopback server, encryption, isolated source/target databases, actual Application
admission and per-request source reopen. It reads the exact committed records,
ingests them into the target through Application, reopens the target and verifies
idempotent replay. It rejects unauthenticated requests, forged owner/source fields
and source re-export of imported records; snapshot reads survive request boundaries,
release is idempotent and released reads fail. This fixture transfers admission
records, not model-streamed nonempty body chunks; it directly invokes the production
HTTP handler, not the entire route table. Mac consumption, authenticated nonempty
body transfer and complete snapshot recovery remain unverified and uncut-over.

Client integration exposed a record/snapshot distinction: multiple operations in
one record can overwrite a content root before the final visible Part head is
stored. Replaying that record still needs the intermediate root. Ordinary snapshot
chunk reads correctly reject it, so they cannot alone implement record transfer.
`ReadAssistantJournalRecordChunk` (Service protocol 36, unchanged schema 40) is now
routed through Application and paired-host HTTP `record_chunk`. It requires an
active exact snapshot handle, loads/verifies the committed record at its precise
cursor, proves that record introduced the requested Part/content reference, then
reads one original verified chunk. It does not broaden ordinary snapshot visibility
or accept a global digest as authority. Source-only Host admission still applies.

The Application two-store test now fetches its nonempty Unicode body through this
command after source reopen, then stages/receives it and reopens the receiver.
`/tmp/nola-journal-record-content-app-20261006.log`: all 6 integration tests passed.
This fixes a transport prerequisite discovered during client integration, not a
claim that Mac has switched from cumulative projections to journal consumption.
Store journal regression: `/tmp/nola-journal-record-content-store-20261006.log`
(52 passed), including a two-Append record, intermediate-root download and ordered
receiver application, cold reopen, wrong Part/record/index and released-handle
rejection. Existing encrypted HTTP admission/replay regression also passes after
the protocol update (`/tmp/nola-journal-record-content-http-20261006.log`, 2 tests).
The new intermediate-root path is verified at Store and nonempty command dispatch
at Application; the HTTP fixture itself still transfers admission-only records.

Receiver download decisions now use `NextAssistantJournalContent` (protocol 37,
schema unchanged). Core verifies exact Host/Run/epoch, record digest and sequence,
then returns at most one incomplete Part/content reference and its persisted
preparation prefix in operation order. The query is read-only: no epoch binding,
publication or completion is inferred from it. A null result permits attempting
record admission, which still performs its full checks; it is not a commit receipt.
Already committed exact records require no redownload. Large-content Store tests
cover recovery after reopen, readiness before publication and dependent roots;
the Application receiver test now follows this query rather than interpreting
Append operations, reopening between content preparation and the next query.
Passed: `/tmp/nola-journal-content-plan-store-20261006.log` (52 tests) and
`/tmp/nola-journal-content-plan-app-20261006.log` (6 tests).

Mac now has transport DTOs and a per-record ingestion method on
`MacRemoteHostService`, using the actual `MacApplicationPort` query/stage/receive
commands. It freezes the configured Host for the transfer, downloads bounded
encrypted record chunks and asks Core for progress after every persisted page.
The message-event entry point has NOT yet been switched to this method; record
ingestion alone does not replace cumulative message projections or provide
whole-snapshot gap recovery. Mac build/transport validation is tracked separately.
Mac target and test bundle compiled successfully, and the 2 selected
`MacAssistantJournalTransportTests` passed in
`/tmp/nola-journal-mac-transport-20261006.log`. These assert exact operation payloads,
Core-owned chunk indices and preservation of decimal values above JavaScript's
safe integer range; they do not execute the complete network ingestion method or
verify rendered UI. No message-event cutover is claimed by these tests.

The per-record Mac ingestion method now has a real Service/SQLite + encrypted
loopback HTTP integration test, `MacAssistantJournalIngestionTests`. A source
Application admits a model attempt and a 6,000-line Unicode delta; the loopback
adapter verifies request HMAC/body digest and AES-GCM, then delegates snapshot and
record-chunk reads to the real source Application Service. No journal digest, body
manifest or record is mocked. The second chunk request returns an injected 503;
new Mac Port/Host objects resume the committed preparation without downloading
chunk zero again, then commit the record. Exact replay downloads no chunks. The
target's bounded snapshot range reads reconstruct the exact original Unicode body.
Passed `/tmp/nola-journal-mac-ingestion-20261006.log`: 1 ingestion test and 2 transport
contract tests, with a successful Mac build. The new integration suite is in the
`host-e2e` test layer.

Evidence boundary: this recreates Mac client objects, not a killed/restarted Service
process; Store cold-reopen coverage remains the Rust tests above. The loopback HTTP
adapter is a test fixture, not the production Rust web router, and the model delta
is deterministic fixture input, not an external model call. Message-event delivery,
UI rendering, snapshot-gap recovery and removal of the cumulative projection path
are still outstanding.

Watermark-driven Mac catch-up now queries `AssistantJournalReceptionState`
(protocol 38, schema 40) for its exact committed receiver cursor, including Core's
genesis cursor when no record has been received. The query validates the remote
Host/Run/epoch binding; already-covered notifications still require the stored
historical digest, not just a sequence comparison. It never changes a source
binding, staging state or public head. Store journal tests passed 52/52 in
`/tmp/nola-journal-catchup-store-20261006.log`, including fresh, covered, older,
wrong-Host and forged-old-watermark cases.

`MacRemoteHostService.synchronizeAssistantJournal` freezes the authenticated target
and announced read window, fetches bounded record pages, validates page ownership
and bounds, and uses the per-record receiver. After each page it queries Core's
actual committed state instead of saving a client-only cursor. Covered notifications
do not request network pages; `SnapshotRequired` is an explicit recovery error,
not a jump to the latest head. Snapshot activation and message-event entry-point
cutover are still required. The Mac integration fixture no longer computes a
genesis digest or manually iterates records outside the production catch-up method.
Verified `/tmp/nola-journal-mac-catchup-20261006.log`: Mac build and 3 selected
tests passed. The loopback source deliberately returns one record per page;
recovery requests the genesis page only once, preserves already prepared chunks,
and a repeated covered notification issues neither record nor chunk requests.
As above, the HTTP adapter is a fixture over the real source Service, and the test
recreates Mac clients rather than killing the Service process. This is catch-up
method acceptance, not a rendered UI or event-producer cutover claim.

Production planner transport admission is now bounded by both canonical binary
bytes and actual JSON bytes (128 KiB each; Service protocol 39, unchanged schema).
The counting JSON writer stops at the limit without allocating the encoded batch.
Frames split only between complete operations; a single oversized operation fails
before publication. This leaves room in a 256 KiB preparation request for the record
header, content proofs and a maximum original chunk encoded as base64. Regression
`escaped_metadata_frames_leave_room_for_verified_chunk_transport` proves a valid
binary batch can exceed the old JSON transport limit, then checks new frames retain
every operation in order and fit the chunk-bearing request envelope. Passed
`/tmp/nola-journal-wire-budget-all-20261006.log` (60 journal lib tests) and
`/tmp/nola-journal-wire-budget-stream-20261006.log` (6 stream integration tests).

The remaining display cutover is a read-model change, not just an event rename:
`MacWorkspaceModel` still commits running remote projections, while Application's
`load_remote_projection_subset` reads `StoredMessage` Parts through
`project_stored_parts`. Journal reception publishes journal heads/versions and the
conversation revision, not replacement `message_parts` bodies. Switching only the
notice producer would therefore leave the existing visible reader without the new
text. The replacement reader must consume journal Part metadata and bounded content
references for all public Part kinds; copying accumulated journal text back into
legacy rows would restore the full-replacement cost this design is removing.

The following counts describe historical acceptance, not a current full-suite run.
The 2026-10-02 multi-owner publication set and borrowed canonical-owner changes
passed the six affected Rust lib suites (1,119 passed, one scale test default
ignored). Full Application/Store/Domain tests passed (1,037, including overlapping
libs); the full CLI binary rerun passed 1,408 tests with two Worker tests ignored.
Its directory capture fix does not complete per-runtime Skill ownership. The explicitly invoked
private Text 2/8/32 MiB Store benchmark passed separately. None enables a producer
or client rollout.

Implemented intermediate boundary (not a consumer rollout): the planner and
transaction-local publisher now cover Text, Reasoning, Tool, Source, Citation,
Artifact (the latter projects as Attachment), and Step. Domain-validated planner
tests and isolated Store publication tests passed at their recorded implementation
stages. The 2026-10-01 rich public-semantic baseline passed all six affected Rust
lib suites, all Application tests, the full Android debug JVM suite, and seven
native test classes on each of Mac and iOS. The full CLI binary suite passed
1,403 tests with two Worker-specific tests ignored. The fifty-read receipt and
short-stream deadline/main-timeline regressions have targeted PTY acceptance.
This does not cover every existing Markdown tail fallback. See the current status
in `../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md` section 8.
Compaction is conversation-context scope, not an assistant bubble. Borrowed
Task/Skill/ModelAttempt planner adapters are now implemented, including keyed
retirement versus absence, explicit failure removal and dynamic Skill titles.
They are now consumed by the production changed-owner coordinator.
There is no `AgentPart::Error` variant to map. Application command call sites and
cross-client journal transport are not thereby complete.

The implemented field model adds `ReplaceContent`. Part generation changes only
on whole-Part replacement. A newly introduced/replaced content field takes the
new head revision as its content generation; append retains that field version.
Deleting a field retains the monotonic head clock, so re-adding it cannot revive
an old root. Whole-Part replacement may retain only exact existing references;
changed references receive the new head revision and still require owner-bound
manifest verification.

Long source/attachment metadata uses typed `JournalTextValue::Field` references,
and long resource lists use a dedicated `ToolResources` field. Citation start
and end line bounds are independent. ToolInput/ToolOutput content uses the
versioned `AssistantStructuredContentV1` JSON envelope; ToolResources uses the
typed `AssistantResourceContentV1` envelope. Object maps are ordered; scalar,
array, object, integer and floating-point values retain their types. Digests
always verify original immutable UTF-8 bytes, never parsed/re-encoded objects.
Web adapters must preserve numeric tokens losslessly (in particular u64 values
above 2^53). `ToolOutputText` is independent UTF-8 preview content, not mutually
exclusive with structured ToolOutput. Human summary uses its own ToolSummary
field; display priority must not erase any of these facts. Raw tool-input fragments
are not visible structured input. Truncated preview tails use field replacement,
preserving unrelated input manifests.

Content equality now walks the actual published manifest pages and any
transaction-local preparation/append overlay, rather than assuming one fixed
chunking of the same bytes. The narrow manifest query authorizes the exact
current published owner/reference and bounds; it does not read chunk bodies or
grant a snapshot. Equal-byte repeated Tool state remains unchanged across
publication and cold reopen; same-length changed preview bytes replace only
their field. Opaque citation URI values and independently optional source
location coordinates follow the same shared public value domain; byte ranges
must be paired and ordered. These correctness changes do not eliminate the
existing ToolState serialization/comparison of cumulative structured results.

### Exact synthetic ownership: production integration

The ordinary local projection and existing live remote-assistant export now use
the same `AssistantOwnerResolver`. Store reads `AssistantOwnershipFacts` with
the selected presentation inputs in one SQLite statement. Run admission and
canonical Part membership remain independently checked. Schema 34 adds explicit
accepted Turn-to-Message and Turn-to-Part bindings; Part body JSON is no longer
an ownership source. Application captures the actual Core Turn and canonical
Message when accepting admission and `PartUpdated`, and Store writes their typed
bindings in the same transaction as those canonical facts.
Queries are scoped to Runs represented in the requested message window; a
window need not contain the admitted message to obtain its ownership proof.
The existing JSON checkpoint scan remains; this is not a claim of constant-time
checkpoint parsing or a completed journal producer.

Task and Skill cards resolve their declared owner session/turn once, rather than
matching every message with the same Run ID. Attempt cards resolve actual
text/reasoning Part ownership; unmaterialized physical retry Parts follow a
validated same-Run/effect/invocation predecessor. Appending a later assistant no
longer changes a Run-wide map's last winner. No evidence means no synthesized
card, not broadcast. Conflicting Part owners, foreign sessions/Runs and invalid
lineage fail explicitly. Conflicting non-admission turn bindings raise an
ownership error only when that synthetic turn is requested; ordinary owned
text remains readable. A proven admission binds the actual accepted Core Turn,
not a Turn guessed from the Run ID. It overrides ambiguous reused original-turn
labels, while explicit Part ownership remains unchanged.

Internal admission now resolves the actual reused assistant slot before planning
and compares that Message, its previous Run, and checkpoint revision again in
the writer. A stale target cannot be silently substituted. Schema 33-to-34
migration creates empty relationship tables; it does not infer or backfill old
relationships from a checkpoint or an ID suffix. Historical read provenance must
remain separate from current write/order authorization.

Validation including the additional Skill/remote assertions: Application
unit suite 161/161, local projection window 2/2, model-attempt identity 5/5 and
remote-running projection reads 2/2 passed. The window byte-budget fixture now
binds its deliberately escaped Part ID to an actual canonical Part instead of
relying on a checkpoint-only message-id string as ownership. Assertions and
read-budget limits were retained. Follow-up fixture assertions cover Task and
Skill separation with admission excluded from the local viewport, plus the
actual live remote export's admitted-assistant subset. That current remote
protocol does **not** export arbitrary successor assistants; no test should
pretend it does. General multi-assistant remote journal transport remains a
separate unfinished boundary.

### Remaining synthetic producer inputs

Implemented planner methods take borrowed changed `TaskHandle`, final
`SkillRun`, and final `ModelAttemptRecord` plus its keyed successor, together with
the resolver's exact owner and durable order key. `TaskUpdated` already carries
the complete changed Task. Skill events identify one Run (Started carries the
Run itself); obtain only that Run from the transition checkpoint map. Merge all
response/prepare/settle attempt mutations before planning and include any changed
predecessor fallback card. A failure cleared by `finish_model_attempt(..., None)`
removes the previously visible attempt card; absence from an event batch is not
removal evidence.

`assistant_journal_changes.rs` now collects these borrowed inputs at real
Begin/internal Begin, accepted Host, active submission, and explicit completion
boundaries. It exhaustively classifies Core and Skill events, resolves only the
changed Skill ID in the final typed map, and preserves the original event order.
The final attempt overlay does not reorder canonical revision-CAS mutations;
failure clearing and predecessor refresh are explicit. Synthetic Close is included
when no accepted final Part metadata already covers it. Core session identity is
supplied separately from the canonical conversation key.

The collected changes currently serve as commit preflight; they are not consumed
by the production planner/publisher. Both ordinary display and the implemented
journal adapters consume the shared borrowed single-item mappers in
`assistant_synthetic_semantics.rs`. Those mappers
do not choose an owner, scan history, or grant execution authority.

The following schema boundaries apply before those methods are enabled:
Task progress accepts all finite binary64 values in [0,1], including subnormals
not representable by the former u64 rational. `JournalProgress` now preserves
IEEE-754 bits losslessly in a strict decimal-string/u64BE contract rather than
rounding; construction, JSON and binary decoding reject nonfinite or out-of-range
values. Signed zero is retained. Step's display title uses dedicated
`JournalStepMetadata` plus inline/typed `Title` content, not a localization key or
background Task identity. It still renders as a Transcript-section step/task card;
completion changes control metadata only and retains the exact immutable title
reference. Skill's display name now uses the same dynamic-text treatment:
short names remain inline, long names use an owner-bound typed `Title` reference.
Switching back to inline releases that reference before deleting its field.
Every model-attempt head requires a `FailureMessage` body, even when empty;
declaring failure metadata without its public message is invalid. Explicit
removal reads a typed retired state and cannot recreate the retired Part ID.
synthetic Tasks use Tasks-section IDs `task:{id}`. Compaction is explicitly
filtered out before ordinary assistant materialization and should yield a typed
non-assistant-scope outcome, not a manufactured chat Part. The idle checkpoint
submission path also needs an explicit commit-stamp policy if it changes an
already-owned Task/Skill after foreground Run completion; a transition stamp
must not be fabricated for that save-only entry point.

## Commit boundary and ownership

The producer belongs inside the canonical command write unit of work, not in
CLI emission or a subsequent `load_conversation_projection_v1` call. Collect
changed inputs while planning the canonical transition, persist the canonical
transition, then append its journal batches before the outer transaction commits.
Failure rolls back both. Only notify consumers after the outer commit succeeds.
Hot Core events with `persist=false`, rejected responses, receipt replay, and
`TransitionCommit.applied=false` must not create new records.

Use a typed Store stamp:

```rust
enum AssistantSourceCommit {
    Admission { run_id: String },
    Transition { transition_id: String },
    // Planned: exact same-UoW evidence from the corresponding Store writer,
    // not client-declared revision or a fabricated Transition ID.
    IdleCheckpoint { /* server-issued checkpoint commit identity */ },
    Recovery { /* server-issued recovery commit identity */ },
}
```

Store must prove the stamp was applied in this write unit of work and resolve
the actual checkpoint/source revision itself. Admission must be stamped by both
`begin_turn` and `begin_internal_turn`; it is not an invented transition ID.
Current `AssistantSourceCommitStamp` implements Admission and Transition only.
Idle checkpoint and recovery writes remain missing proof cases. A historical
stamp must not authorize a first journal publication.

Identity is Store epoch + conversation + canonical Run + actual assistant
message. `CanonicalPartProjection` and `ensure_assistant_message_for_part` in
Application already map the first Core assistant to the admitted message
and preserve later successor assistant IDs. A Run can therefore own several
assistant journals. Journal Store already validates exact persisted assistant
membership/role/Run ownership rather than requiring the admitted assistant only.
Current write ownership remains distinct from trusted historical read/release
ownership; same-UoW ensured-message proof and exact source author still apply.
Never accept arbitrary client ownership or
derive it solely from an ID suffix. Tasks and skills need the same explicit
turn-to-message mapping; their owner turn is not necessarily the outer Run ID.

## Actual producers and missing-change risks

| Entry point | Existing typed inputs | Journal obligation |
| --- | --- | --- |
| `assistant_turn::begin_assistant_turn` | Core transition event envelopes, admitted assistant/placeholder, initial `model_attempt::prepare_effects` mutations | Process initial parts and synthetic placeholder once, establish ownership/order, admission stamp; do not wait for the first response. Internal turns need the same handling. |
| `lib::accept_host_event` | Validated response, `PartUpdated`, canonicalized part, ensured messages, synthetic `PartMutation::Close`, response and successor attempt mutations | Collect only accepted deltas; also collect Task/Skill events currently ignored by its Part-specific match. Flush after the successful canonical commit inside the UoW. |
| `lib::reduce_and_commit_active_submission_with_core` | Submission transition envelopes, newly prepared effects, settled effects/attempts, ensured messages and synthetic closes | Covers interrupt, permission, task completion and successor promotion without a Host response. Must use the same planner, not a second presentation implementation. |
| `model_attempt::response_mutations` | `Vec<ModelAttemptMutation>` carrying final records and expected revisions | Failure/retry can change visible output without a `PartUpdated`. Failed attempt and fallback metadata are visible; a successful attempt without failure currently is not. |
| `model_attempt::prepare_effects` / `settle_effects` | Same mutation type, effect binding and predecessor IDs | Merge all mutations for this commit before projection. A successor changes the predecessor's fallback fields; load that predecessor by ID, not every Run attempt. Prepared/invisible attempts may yield no journal change. |
| `lib::complete_assistant_turn` | Explicit assistant ID/part, close mutations, settled attempts | Existing third direct `commit_transition` call; cannot bypass the producer. Close metadata must match ordinary completion. |
| Maintenance / idle paths | Maintenance checkpoint or idle submission | No assistant journal for compaction alone. If an idle operation can visibly mutate an existing task/skill, require exact owner and a suitable typed source stamp; do not silently pretend it is an active Run transition. |
| `Store run_recovery::with_plan` | Authoritative recovery plan that directly terminates Parts | Publish terminal control states in the same recovery UoW. Enabling a consumer while this path bypasses publication would leave permanent Running cards. |

### Complete publication set implementation and leaf acceptance

The former one-record helper rejected otherwise legitimate transitions exceeding
128 operations, 128 KiB canonical metadata, or 256 KiB supplied append content.
The 2026-10-02 source implementation replaces that limitation with
`CommitAssistantPublicationSet { stamp, groups }`. Each distinct exact owner has
its captured starting cursor and ordered `AssistantJournalFrame` values. The
singleton helper is a wrapper for a complete singleton set; calling it separately
for each owner cannot publish one multi-owner source commit.

Store resolves the actual source revision and derives the source key and frame
operation IDs. Schema 33 records the complete set digest, owner/frame/operation/
supplied-chunk counts, supplied UTF-8 bytes and final cursors, plus exact ordered
frame membership. The digest binds owner membership and order, starting cursors,
canonical batch bytes and supplied chunk order, count, digest and length. Replay
must verify the full persisted record/header chain and membership, not merely
find a receipt or accept one matching prefix. Missing/reordered frames, changed
bytes or different owners must fail. First publication still requires the actual
source applied in the current write UoW; a historical read proof grants no first
publication authority.

Planner `finish` now returns ordered bounded frames for one owner. UTF-8 chunking
preserves bytes, and Append operations introduce at most four 64-KiB chunks so
each operation can fit in one frame. Packing checks actual canonical batch bytes,
operation count, supplied chunk count and supplied raw UTF-8 bytes. It retains the
touched-head overlay and operation order across frames; splitting does not reset
Part/content generations or recompute old chunks. These are separate budgets:
128-KiB metadata and 256-KiB supplied content do not imply a 256-KiB combined JSON
transport envelope. Transport cannot rechunk an already committed manifest.

The Store set savepoint owns all of its introduced chunks, records, versions,
heads, membership rows and publication receipt. Final-frame failure rolls back
earlier frames. Caller preparations made before that savepoint remain invisible
but belong to the outer command UoW, together with checkpoint, effects and command
receipt. Preparation errors, mismatched resulting references and set failures
propagate; the coordinator must abort the outer UoW before notifying consumers.
The publisher prechecks typed batch encoding and group ownership, but Store also
checks supplied content and source authority. Do not interpret an error as an
optional display failure or assume all earlier preparations were locally undone.

A complete replay manifest proves equality with the already persisted set. It
does not prove that a first submitted set includes every visible change in the
source transition. That separate production gate requires an authoritative typed
changed-input visitor, exact changed-owner set and coverage of synthetic closes
and dependencies. Self-declared counts, a successful leaf set test or an old
conversation projection diff are not substitutes for that source coverage.

New fixtures cover isolated Store two-owner/three-frame groups, cold whole-set
replay and tampering, current/old/rolled source stamps, owner/CAS rejection and
final-frame failure with explicit outer rollback/retry. Planner fixtures cover
operation-count, metadata-byte and large Unicode-delta splits; an Application
fixture passes the borrowed canonical owner through actual planning/publication
and cold reads. These fixtures passed the 2026-10-02 six-crate lib run, not the
production Begin/Host/submission/recovery integration gate. Full current-source
tests also passed; exact scope and logs are tracked in overall redesign section 8.5.

Source author and presentation owner are separate facts for idle Task/Skill and
multi-assistant updates. A historical membership proof alone grants no source write
authority. The production coordinator must resolve the current authorized writer
and exact target independently, including internal resumption, before this set
contract can be enabled. Do not create an assistant or guess a Run to bridge a
missing owner/proof case.

`conversation_projection.rs::project_tasks`, `project_skills`, and
`project_model_attempts` are existing display policies, not incremental sources.
Their per-item mapping is now extracted into pure borrowed helpers. The journal
adapters still need to consume those helpers; do not call the functions that scan
all projection order arrays or all attempts.
`projection_journal.rs` is the existing conversation invalidation/diff journal,
not the new body journal and not a cheap changed-item source.

## Planner interface and bounded reads

`assistant_journal_planner.rs` now consumes a borrowed `CanonicalPartProjection`
through `part_update_view`. `PublicPart::from_canonical` changes only the borrowed
message identity: the initial Core assistant maps to the admitted message and a
successor retains its own canonical message. The planner rejects a different
canonical Run context, message or Part before loading a head. The Core turn ID
remains distinct from the source Run, including child turns. This avoids cloning
the cumulative Part just for journal ownership remapping; it does not remove
cloning in every existing persistence or projection path.

```rust
enum VisibleJournalPlan {
    Unchanged,
    Changed(Vec<PreparedAssistantJournal>), // ordered bounded frames for one owner
}
// Each frame carries a typed batch, bounded supplied chunks and preparations.
// The complete producer groups (captured starting cursor, plan) by exact owner.
// No caller-provided source revision or arbitrary public JSON operation.
```

The production changed-input visitor is connected as validation before canonical
writes. Its collected result still needs the exact-owner coordinator and actual
publication call. The visitor consumes, in original event order:

- `PartUpdated { update: &PartUpdate, part: &AgentPart, ... }`, after owner
  canonicalization; use the update's new bytes, not cumulative part text.
- `TaskUpdated { task: &TaskHandle, ... }`, already a complete changed task.
- `SkillRuntimeUpdated { update: &SkillRuntimeEvent, ... }`: extract the changed
  skill Run ID and obtain that one final record from the transition checkpoint
  map. Enumerate variants explicitly; session/catalog events are not skill cards.
- Final merged `ModelAttemptMutation` records plus keyed predecessors and
  synthetic closes. Do not derive these from trace strings.

Keyed journal-head reads already exist. Schema 34 and
`AssistantJournalPlanningRepository` add exact persisted Turn/Part owner reads,
a keyed same-Run/effect/invocation attempt-successor read, and durable per-owner
section reservations backed by counters. These narrow planning reads do not load
a checkpoint, body, or full presentation. Reservations require the current
same-UoW applied source stamp; historical ownership alone does not authorize
them. Existing snapshot pagination is not a substitute for a head lookup.
Maintain a touched-head overlay so multiple operations on
one part see preceding operations within this commit. Read each touched head
once. Feed the resulting typed operations through Domain validation in Store.

No-change is an Application planning result: do not construct an empty Domain
batch. Compare typed metadata and content refs before making `Update`; unchanged
metadata must not increment head or metadata revision. Changed source ledger
revision alone is not changed visible metadata. Replay is separate from no-change.

## Visible-operation mapping

- Creation: `Declare` with bounded metadata and content refs. Deduplicate the
  synthetic admission placeholder against a `Created` event for that same part.
- Public text delta: `Append` using only the delta bytes and persisted current
  content ref. Never compute SHA over the old full text. UTF-8 chunks are at most
  64 KiB; one incoming large replacement is split before bounded preparation.
- Text replacement: `Replace` with new generation and manifest; same content and
  metadata is unchanged. Completion usually updates final metadata only, not a
  full-body rewrite. Synthetic closes participate even without a Completed event.
- Review: pending/rejected/withheld text publishes no private body chunks.
  Approval reveals the eligible text exactly once via a new public generation;
  revocation replaces with an empty withheld generation. A review-only event is
  not lost because its text delta is empty. Existing policy allows public text
  only when review is absent or approved.
- Reasoning: append the public summary/detail delta; title/state changes are
  bounded metadata or a typed body-field replacement. Match the existing display
  policy; do not expose private reasoning fields omitted by that policy.
- Tool input/output, source snippets, errors, task current step/result and skill
  result/failure: use the declared content fields. Repeated progress events with
  identical visible state produce no record. For structured content, define a
  versioned canonical content encoding and MIME/schema contract; raw arbitrary
  JSON bytes must not stand in for operation or metadata validation.
- Task/Skill/Attempt cards: use their dedicated section and stable synthetic part
  IDs. Preserve safe failure redaction; do not copy provider diagnostic bodies.
  Removal requires explicit authoritative removal, never absence from a delta.

The pre-migration Domain metadata did not represent every rich tool/artifact/source
property of `MessagePartView`. Shared public values and the typed metadata/body
extensions are now implemented in source; full current-source acceptance remains
required before switching journal clients. A catch-all opaque Historical payload is not a
valid substitute for current known types. Unknown future variants must fail an
explicit integration capability gate, not be silently omitted.

### Public semantic parity gate (2026-10-01 implementation and acceptance status)

There are currently three display contracts: `MessagePartView`, rich `AgentPart`
client presentation, and journal metadata/fields. Before this slice, the older DTO
omitted user-visible information. Equality against that DTO alone is not a parity
proof. A public semantic mapping is now shared by ordinary projection and planner,
with presenters migrating the same Domain contract. Destructure known source types
without `..` so every field is explicitly
classified as public, public via another contract, or private.

| Family | Required public semantics; source implementation is not rollout proof |
| --- | --- |
| Text | Preserve pending/rejected/interrupted review statuses independently of withholding the body; a status-only change must not disappear as Unchanged. |
| Reasoning | Preserve title and title-only changes with typed text refs. Append/replace summary by field; using `bodies[0]` or overwriting the whole body vector would erase unrelated title refs. |
| Tool | Keep title, human summary, structured result and output preview separately. Do not reintroduce the old mutual exclusion between structured result and preview. Preserve read-only origin/permission-display, artifact name/URI/MIME/operation, timing and an owner-bound full-output descriptor. |
| Source | Preserve typed redacted evidence. `MemoryUsage` deliberately excludes memory content/summary but includes verification, truncation, usage and supersession facts used by existing source UI. |
| Citation | Preserve citation identity, optional MIME and section alongside source/tool/page/line/range. Section and citation ID are used by current Mac/CLI consumers. |
| Attachment | Keep the operation where the file-action display requires it; do not reconstruct artifact data from a generic affected-resource type. |
| Step | Dedicated title/state is sufficient; old task-id/current-step values are derived from its own id/title, not additional background task facts. |
| Task/Skill/Attempt | Implement actual adapters, exact optional current-step/result/failure bindings, long Skill display-name refs, and required Attempt failure text. An enum field existing does not prove mapping. |
| Historical/Compaction | Keep their existing history/context scope. The current small Historical metadata is not a lossless substitute for history field states, status, binding and canonical-text metadata. |

`ToolProgress.detail` is not an additional canonical Tool field: Core assigns it
to output preview. Human `summary` is genuinely independent. Render priority is a
presenter decision, not authority to discard one of the stored public fields.
Permission-display/link metadata is not a grant, lease, token or executable request.
Never expose provider diagnostics/credentials, raw incomplete input as resolved
arguments, private full-output paths, memory body/summary or withheld drafts through
this mapping. Large display fields belong in typed content, not unbounded metadata.

Three clocks are distinct: canonical conversation/projection revision, journal
sequence/Part-head CAS revision, and content generation. Current journal header
source revision is a checkpoint revision. Retain the actual source revision or
define an explicit new presentation revision contract; do not fill the old DTO's
revision with local head CAS and call it equivalent.

Required tests vary each public field independently, including title-only,
summary-only and pending→rejected; preserve simultaneous structured result,
preview and summary; distinguish None, empty text, structured Null and empty
collections; preserve long Unicode refs across unrelated field deltas. Compare
actual reducer→Store→cold reopen→public reconstruction against an independently
field-complete baseline and actual Mac/CLI presenters, checking each clock rather
than ignoring revisions in equality. The shared Domain values, borrowed
Application mapping, ordinary projection, typed journal metadata/fields and
Mac/iOS/Web/Android actual consumer migration are now implemented in source.
The recorded 2026-10-01 baseline has Web runtime tests passing 48/48, full frontend
tests passing 229/229, and svelte-check reporting zero errors/warnings. The six
affected Rust lib suites passed 1,103 tests
(one scale test remained ignored); all Application tests passed 216 lib and 252
integration tests. Earlier diagnostic and historical-fixture failures were fixed
and rerun without relaxing the actual encoded-byte bound of less than 12,000.
Mac was rebuilt and passed 70 tests across seven classes; Android's full debug
JVM suite passed 309 tests with two skipped, including eight new regressions.
The iOS native rerun passed 64 tests after fixing top-level JSON scalar encoding
and sharing the exact integer codec with Core, including actual live commit/query/
cold consumption. The original null/large-integer assertions remain. The full CLI
binary suite passed 1,403 tests with two Worker-specific tests ignored, including
eleven new consumer/deadline/Markdown regressions. The short-stream main-timeline,
queued-input, resize/cancel and permission-return PTYs passed; fifty-read also had
targeted acceptance. This is not a journal rollout, a full PTY/UI pass, or proof of
complete parity. Earlier failures and narrower source scopes remain recorded.
ToolProgress must be tested against the actual Core update: detail changes
preview, not human summary.

Canonical Part persistence must preserve the original structured result even
when it is large and has no separately retained output file. Diagnostic previews
may be bounded, but must never replace the only canonical result with a fabricated
`{output_preview, truncated, full_output_ref}` object. Ordinary full projections
remain cumulative owned snapshots; the borrowed helper and bounded diagnostics
do not make that old transport incremental.

`ToolOutputReference` carries only message/Part/call identity. The current clients
must not invent a reader or permission action from it. Some structured JSON Null
is a result, not absence: serializers omit absent output and decode a present null
as Some(Null). Citation location absence and an explicitly empty location likewise
remain different source facts. All three revision clocks stay distinct.

The recorded 2026-10-01 lib baseline covered two earlier semantic gates: equality
uses actual manifest pages and transaction-local overlays, preserving NoChange
across different legal chunk boundaries; citation paired byte ranges and opaque
URI domains agree with ordinary public values. This is correctness coverage,
not production rollout or elimination of cumulative ToolState serialization.

## Implementation slices and acceptance

1. StorePort/Store: typed Admission/Transition stamps, exact successor membership,
   keyed journal heads, explicit turn/part ownership, durable section order, and
   attempt-successor reads are implemented. Ordinary projection/checkpoint scans
   are not substitutes. Current-source acceptance for the 2026-10-04 additions is
   tracked in overall redesign section 8.12. The 2026-10-02 complete
   publication set, bounded framing and borrowed canonical-owner changes passed
   current-source lib acceptance. Preserve first-publication UoW proof, whole-set
   replay equality, tampering and rollback tests. Idle/recovery stamps are absent.
2. Application: new planner and per-item display helpers; connect Begin, the two
   reducers, and explicit completion. Initially exercise all text/review/final
   cases behind a producer gate without replacing the existing transport. Gate
   activation requires all reachable visible kinds, not silent partial journals.
3. Add Task/Skill/Attempt adapters and dependencies;
   remove the gate only after parity tests over real commits pass. Then transport
   can consume independent assistant cursors and chunks; terminal receipt carries
   the final journal watermark rather than the full assistant.
4. Separately eliminate remaining `part_mutations_for_update` fallback full-prefix
   hashing, legacy SQLite concatenation and whole checkpoint/Part cloning. Eligible
   private canonical Text already has its own chunked production path; public
   journal framing does not make the rest of the execution path linear. This
   slice does not supply a public-journal/Core/transport end-to-end scale measurement.
   The private Text Store-only 2/8/32 MiB benchmark was separately rerun and passed.

For the next production slice, collect borrowed typed inputs from the accepted
Core event sequence, retain ordered persistence mutations, and commit canonical
facts first. Only an applied commit in the same outer write UoW, with the appropriate
receipt-acknowledgement or durable-flush purpose, permits exact
persisted-owner resolution, captured cursors, touched-head planning and one
complete set publication. A newly ensured successor is not readable before the
canonical commit; do not relax the owner gate to publish it earlier.

Task updates carry their complete changed record; Skill changes require exhaustive
event classification and one keyed final run lookup (catalog changes are not
cards). Attempt presentation may use a final-record overlay, but Store must keep
predecessor-then-successor mutation order for CAS. Clearing a failure is an explicit
card removal, not absence from the batch. Synthetic closes are typed control
inputs, not fabricated Part/body events. Group owners by first touch and preserve
event order within each owner. `persist=false`, rejected responses, unapplied
commits and receipt replay must not mint new records.

Unifying the outer UoW is part of this slice. Received commands establish a
receipt-acknowledgement scope. Bare Runtime Begin, active/idle submission,
cancellation, recovery and durable Host boundaries now establish a durable-flush
scope; pure hot deltas below the existing 64-KiB budget do not open that scope.
Store commits before retained overlays are adopted. An unknown commit outcome,
failed rollback or failed adoption poisons the runtime until a fresh owner is
opened. This transaction boundary does not by itself connect journal publication.
Idle checkpoint
and recovery also require their own authoritative source/change-set contracts;
an old Run read proof or checkpoint revision must never be promoted to a new write
stamp. Keep consumers disabled until these real paths are covered.

The coordinator must consume the implemented keyed reads, not the ordinary
projection's checkpoint-wide ownership scan or an assistant-ID suffix. Preserve
removed order positions and validate Run/effect/invocation before applying a
final attempt presentation overlay. A read proof remains insufficient for first
publication; the source must have applied in the same outer write UoW.

Do not wrap every bare `ApplicationRuntime::execute` in the existing command UoW
unchanged: `has_command_unit_of_work()` currently forces persistence and would
eliminate the 64-KiB hot batching contract, including no-save short deltas.
The implemented supervisor distinguishes receipt acknowledgement from a durable
flush: both share the same
Store and overlay rollback owner, but only the former requires every accepted
event to persist immediately. A purely hot change publishes no journal.
Idle and recovery writers must return real commit identities and exact changed
targets. Multi-Run recovery also needs explicit source-author versus target-owner
authority; it cannot silently relax the current publication set's same-Run gate.

Tests must use real Begin/Host response/submission commits, not just handcrafted
operations: initial parts; short delta; review approval/revocation; metadata-only
completion; repeated identical progress; Task/Skill changes without PartUpdated;
retry predecessor fallback; cancellation settlement; successor assistant owner;
receipt replay; rejected/stale response; planner/journal failure rolls back
canonical/outbox/receipt together; terminal cleanup does not prematurely seal.
Instrument bytes read/hashed/written and head reads for 1,000 equal deltas and
10,000 unrelated parts. Expected journal cost is proportional to new bytes plus
changed bounded heads. Measure legacy cumulative persistence separately.

Chunk transport is not implemented by this producer. Keep metadata frames as
refs and use immutable raw-UTF8 chunk delivery (or the separately specified
bounded binary/base64 framing). JSON escaping a 64-KiB control-character chunk
can exceed the 256-KiB frame budget; do not rechunk immutable content on send or
leave an otherwise valid committed chunk permanently unsendable.

### Snapshot-free Core reduction (pending consolidated validation)

Core reducers now return `AgentReduction`: events, traces, Host requests and
durability intent, without a cumulative presentation snapshot. `submit_at`
materializes a snapshot after reduction for boundary consumers; Application's
Host-response path calls `reduce_at` directly. The runtime requests a snapshot
only for persisted, non-delta responses and reads terminal status directly from
Core before retaining the hot Run. Buffered batch responses and effect replay
also reduce without constructing intermediate snapshots. This replaces the old
generate-then-discard path; it does not change response admission or commit
authority.

Application now consumes its exclusively owned candidate through
`reduce_owned_at`. A failure returns no Core, and a successful candidate remains
subject to the existing commit/adoption boundary. This removes the submission
rollback clone from the Host-response path. Borrowed Core entries retain rollback
and now cover errors from replay preparation as well as dispatch. Durable
checkpoint serialization remains; this does not complete the journal-only
cutover. A reduction-versus-projected-entry regression
fixture covers execution facts, checkpoint state and repeated close submission;
it covers borrowed and owned entry points. A rejection fixture checks borrowed
state preservation and recovery from canonical facts. These fixtures have been
added but not run, along with the rest of this implementation batch.

Host-response persistence also skips loading the prior checkpoint JSON when
the resulting memory usage ledger is empty: only the checkpoint CAS revision
is selected in the enclosing write transaction. Nonempty accounting still
loads its prior usage identities; checkpoint serialization itself remains.
Checkpoint revision increment and event-cursor conversion now fail on overflow
instead of wrapping or silently retaining a stale cursor. An unrun transaction
fixture verifies that an overflowing cursor leaves the checkpoint unchanged
and permits retry of the same Host response sequence with valid state.

Response admission now matches session, Host, effect, request, Host lease,
catalog revision and availability epoch to the committed request before
validating the delivery lease. Turn identity must also match, with only Core's
existing non-replaceable in-process Host exception for an omitted Turn echo.
A supplied foreign Turn is never normalized. Direct Host
transition acceptance additionally requires checkpoint outer/inner session,
event-envelope sessions and successor-request sessions to match that request.
Conversation identity is not assumed equal to Core session identity. The same
unrun rejection fixture includes ten foreign-identity cases and a subsequent
valid retry, so rejection must not consume the delivery sequence.

The shared Host protocol now checks `PermissionResolved.request_id` against
`RequestPermission.permission_request_id`, including authorization, instruction
and return-to-composer interventions. Previously the payload-kind check alone
allowed the latter to reach cancellation before permission identity validation.
Application admission also calls this shared payload contract before accepting
the delivery lease, covering direct transition acceptance as well as reduction.
An unrun protocol fixture covers current, stale and empty IDs for all three
resolution shapes. No grant is inferred from an accepted response envelope.

The Core integration fixture additionally reaches a real tool permission wait
through model completion, rejects each foreign resolution, compares the entire
checkpoint before/after rejection, and retries the same sequence with a valid
authorization to produce tool execution. It also exercises rejection through
the consuming reduction entry after checkpoint restore and committed-request
rebind. This fixture is pending the consolidated test run.

#### Remaining accounting dependency in Host checkpoint removal

The nonempty memory ledger path still reads the prior checkpoint to select new
usage IDs. `apply_memory_usage_entries` already enforces a unique usage ID and
rejects conflicting content, but it checks active memory existence before
handling duplicates. Sending the entire historical ledger on every commit is
therefore not a safe replacement: deletion of a previously used memory would
make an unrelated later stream commit fail. The next removal must read recorded
usage identities from the transactional ledger and submit only new entries,
with coverage for deleted historical memories, new references to deleted
memories, and identity conflicts. Keep that read on the existing writer/UoW;
do not add a second connection or derive accounting authority from a UI cache.

This Host-response dependency is now replaced in the pending implementation:
`RunEffectRepository::unrecorded_memory_usage_entries` queries the ledger by
usage ID on the existing connection, compares exact stored content, and returns
only absent records. Host acceptance loads only the checkpoint CAS revision,
including when memory usage exists. The old prior-checkpoint read is removed
from this path. A SQLite fixture covers uncommitted visibility/rollback,
deleted-memory exact replay, conflicting content, and a new usage of a deleted
memory (still rejected by commit validation). It has not been run. This still
iterates the in-memory usage ledger and does not eliminate checkpoint writes;
idle/active-submission accounting paths were still pending at that step.

The same transactional ledger filter now serves active submissions, idle
submissions and turn completion. The previous-checkpoint JSON usage-ID decoder
and its production branch have been removed; idle submissions no longer clone
checkpoint JSON for accounting. Hot active submissions no longer perform their
additional full checkpoint read for this purpose (the committed-delivery read
still carries a checkpoint witness), while cold restore retains its required
state load. Completion reads only the CAS revision before storing its supplied
final checkpoint. Active/completion cursor conversion and revision increment
also reject overflow. These changes remain uncompiled and untested in this batch.

An additional Application/SQLite Host-commit fixture now writes the initial
usage, soft-deletes its memory, and installs a trigger rejecting any repeated
ledger insertion. Subsequent exact replay must still commit. Conflicting usage
content and a new usage of the deleted memory must leave checkpoint bytes and
revision unchanged; a valid retry at the same Host sequence must then commit
with one total ledger row. This is pending validation, not a passing result.

Active submissions, idle submissions and conversation revert now also consume
their exclusively owned Core candidate via `reduce_owned_at`, eliminating the
borrowed submission rollback copy on these Application paths. Their reducers
return execution facts, control flow reads `core.turn_status()`, and snapshots
are materialized only while assembling the accepted result after persistence
and journal staging succeed. Publication still waits for the enclosing
transaction commit/adoption boundary. This does not remove the durable
checkpoint clone or serialization and remains pending consolidated validation.

Idle submission and conversation revert now consume the candidate into its
checkpoint as well, moving transcript and frozen requests instead of cloning
them. Those two paths must capture their presentation snapshot before consuming
Core; it stays private until the transaction succeeds. Hot active/Host paths
still retain Core and therefore still clone durable checkpoint state. Borrowed
and consuming checkpoint equivalence fixtures include terminal state and a real
permission wait with a rebound frozen request; neither has been run in this batch.

Host-response persistence now uses `Core::checkpoint_view`, borrowing live
Thread state for synchronous encoding, accounting and journal planning. The
internal acceptance function accepts that typed borrowed checkpoint; the public
owned acceptance entry still applies durable history projection before entering
it. Nonterminal state and terminal state with pending cleanup effects are
borrowed unchanged. Terminal history with no pending effects receives the same
owned discovery-redacted projection as before. Thus a streaming checkpoint no
longer starts by cloning the full Thread. Serialization and the full checkpoint
database write remain and must not be reported as incremental persistence.
Pending tests compare exact serialization, pointer identity of borrowed state,
terminal projection behavior, and a real restored permission request.

Store's syntax-only JSON validation now uses the existing strict no-collection
visitor for resident strings, including transition checkpoint validation and
protocol-journal payload validation. It no longer constructs/discards a complete
`serde_json::Value` tree after Application has just serialized the checkpoint.
The `Value` parser remains an independent test oracle for grammar, finite
numbers, Unicode/surrogates and nesting limits; `IgnoredAny` is deliberately not
used because its acceptance rules differ. Escaped large strings can still
allocate parser scratch space. This removes a second object-tree allocation,
not the full serialization/write, and is pending consolidated tests.

Core response-family admission and task-promotion decoding now deserialize the
frozen request by reference instead of cloning its whole JSON tree first.
Accepted-response metadata also decodes once for both review mode and elapsed
time, rather than twice. Typed request decoding and the pending-effect clone
still retain cumulative request cost; this is not a claim that per-response
request materialization is removed. Existing response/permission regression
fixtures cover these paths but remain unrun in the current batch.

Accepted-response dispatch now borrows the pending effect, copies only its
small dispatch identities/control facts, and decodes a narrow timing/review-mode
projection. It no longer clones the pending effect's complete request or
materializes model messages/tools solely to determine review mode and elapsed
time. Task promotion reads the complete original request only in its own branch.
Full response-family admission still decodes the request and is not removed by
this projection. A pending fixture checks actual ModelStream wire modes, exact
u64 timestamps, large Unicode context and non-model metadata.

Shared `HostRequest::accepts_response` now also requires exact nonempty tool
call identity for call-scoped progress and completion. Application's common
admission check therefore enforces this even for direct Host transition
acceptance, rather than depending only on Core reducer validation. Protocol
fixtures cover valid, foreign and empty progress IDs and foreign completion;
the existing Core call-mismatch fixtures remain relevant. All are pending the
consolidated run. This change does not infer permission from a matching call ID.

Mac's journal Part loop now renders Attachment metadata, reusing bounded
reference-field reads and the existing provenance-aware artifact opener. Name,
MIME type and operation are shown without decoding cumulative conversation
snapshots. A field-backed URI must be read completely within the navigation
budget before becoming a link; remote journal metadata cannot open a local file.
The metadata/navigation fixture is added but unrun, and UI rendering is not yet
verified. Legacy message attachment arrays remain because they lack the exact
journal attachment identity needed for safe per-item handover; no blanket
suppression of uploaded/historical attachments is introduced.

Task interaction review found a remaining protocol gap: journal Task metadata
does not carry supported control capabilities, while the old view used the
typed Task kind to limit guidance to agents. Do not infer those capabilities
from display keys, names, guidance text or a nonterminal flag. A versioned
control-capability projection is still required before that handover is complete.
Meanwhile the journal view now disables interaction for unknown activity states
and cancellation-requested tasks rather than treating every nonterminal string
as actionable. The state/cancellation fixture is unrun; it does not prove the
missing guidance-capability contract is implemented.

That capability chain is now wired in the pending batch. `TaskControl` is a new
canonical metadata tag (12), preserving existing `Task` bytes as read-only
historical facts. The producer derives guidance support from typed `TaskKind`
and emits explicit guidance/cancellation capabilities. A live update to a legacy
Task performs an exact-owner generation replacement, preserving its field refs
before applying changed fields. Mac decodes both variants but only enables
controls explicitly declared by the new variant, still gated by known active
state and cancellation status. Service protocol is now 47. Domain roundtrip,
planner and Mac capability fixtures are updated/added but unrun; control
capabilities are presentation facts, not authority to bypass task admission.

Pending task-control regressions now cover the complete planner operation
sequence: legacy Task → generation replacement → capability-only metadata
update → no operation for a source-only revision, followed by canonical batch
validation. A separate typed-kind matrix covers agent/tool/skill/data-refresh
tasks across queued, running and completed states. State and cancellation still
gate interaction independently of capability support. These are written tests,
not execution results.

Mac artifact provenance reads now reuse proof by database + full journal
identity, not by advancing cursor. Same-owner progress no longer resets local
links or repeats the locality query. A changed store/epoch/conversation/Run/
assistant invalidates proof, and a failed read offers explicit retry. Task view
state uses the same owner key inside its Part identity so an unsent draft cannot
carry across owners. The owner-key isolation/progress fixture is unrun; actual
SwiftUI interaction and retry behavior still require consolidated acceptance.

Canonical attachment Part IDs are now preserved alongside the filtered name/URL
arrays in Mac's process-local message projection. Journal attachment adoption
records exact attachment IDs under full journal identity, and the old card list
filters only successfully adopted entries. Same-named/URI attachments without
that identity remain visible. Retired journal attachments do not resurrect old
cards. The mapping is excluded from message Codable imports. Identity namespace,
retirement, owner-switch and non-importability fixtures are added but unrun.
Legacy attachment context-menu actions have not yet been migrated to this
provenance-aware path and remain a required follow-up.

Legacy card clicks, accessibility Quick Look, and message Quick Look/Finder
menus now use the shared destination resolver. Journal-backed local paths need
an explicit local-owner proof at the action boundary; failed proof reports an
error instead of opening the path. Menus filter to uncovered attachment IDs, so
retired/handed-over arrays are no longer a second opening path. Journal artifact
cards expose Quick Look/Finder only for proven local destinations and Open Link
for web destinations. Non-journal legacy uploads retain their existing local
behavior. Interaction/error presentation still awaits Mac UI acceptance.

Explicit message Copy/Copy as Markdown now reads public Markdown from a single
pinned journal snapshot rather than `message.text`. Metadata pages and 32-KiB
UTF-8 ranges are checked under that cursor; all content is assembled only for
the user's export action. A failed/cancelled read never publishes partial text
to the clipboard, repeated copy cancels its predecessor, and view disappearance
cancels reading. Historical schema-specific text retains the historical adapter.
The existing encrypted ingestion fixture now asserts the full 6,000-line Unicode
answer is exported instead of a deliberately stale fallback. It is unrun. The
Select Text sheet still reads the old text and is a remaining migration item.

Select Text now captures an immutable selection (journal reference + historical
fallback) when opened and uses the same pinned export reader. The sheet exposes
loading/error/retry states, cancels its structured read on dismissal, and copies
the exact displayed string rather than issuing a second read at a newer cursor.
The old direct `message.text` sheet is removed. A cancellation-before-open
fixture asserts export does not create/open its database after cancellation;
it is added but unrun. Clipboard/selection UI acceptance remains outstanding.

## Remote Run terminal seal cutover — STORE CHAIN IMPLEMENTED, ACCEPTANCE PENDING

This section records the minimum complete replacement for the remaining live
`conversation_projection_committed_v1` terminal aggregate. It is a design and
implementation checklist, **not an accepted end-to-end capability**. Do not
remove that producer merely because Part journal delivery is working.

The source vertical is now connected (pending consolidated validation): schema
51 persists the immutable manifest and seal; Application freezes them after the
last journal publication inside the existing terminal transition outer UoW.
`LocalAssistantRunTerminalSeal` and `ReadLocalAssistantRunTerminalManifest` are
strict local-source read commands. Domain owns `validate`, `validate_manifest`,
and the versioned canonical manifest digest, so receivers must not invent hash
rules. Regression additions cover cold reads, identity rejection, immutable rows,
and rollback of completion when seal insertion fails. Receiver commands now
persist the staged header, resume ordered manifest pages, verify the complete
manifest digest before admitting additional assistant owners, and atomically
finalize only after every historical sealed cursor is durable. The receiver
stores child receipt attestations as manifest evidence, never as locally
executable task receipts. Its source binding key is `(run_id, assistant_message_id)`.
Source message ordinals remain immutable manifest ordering evidence, not
recipient row identities: existing recipient rows retain their local positions;
newly admitted owners append in verified source order with transaction-local
ordinal allocation. A different local history cannot invalidate an exact
Run/source/message binding, and a matching position cannot authorize one.
Receiver regressions cover partial-page cold restart, multi-owner gating, hash
failure rollback, commit failure rollback, historical-head catchup and legal
post-terminal background records. These tests are not yet run; client recovery,
UI cutover and consolidated end-to-end acceptance remain required.
The sealed frontier is permanent, but later authorized background presentation
updates remain legal; a receiver may be beyond a sealed head and must verify its
historical digest, never replace the seal with the current presentation head.

### Current evidence and dependency

- Journal operations currently cover Part declaration, append, replacement,
  metadata/content updates and retirement. `seal()` in the Domain journal code
  seals a record digest; it is **not** a Run terminal seal. A final Text, closed
  Tool, completed Task card, EOF, trace completion, or caught-up notification is
  not terminal execution evidence.
- `assistant_journal/receive.rs::receive` persists public records/content and
  advances conversation revision. It does not commit a remote Run terminal or
  prove that the complete required owner/child frontier has arrived.
- `MacRemoteHostService.decodeStreamEvent` currently catches up every exact-Run
  journal before forwarding the legacy terminal aggregate. Live handling in
  `MacWorkspaceModel` and its transport/trace recovery paths then call
  `commitRemoteConversationProjection` before confirming completion. Tool-plan
  retirement also tests that aggregate's exact source/Run terminal status.
- `canonical_run_terminal(conversation_id, run_id)` reads durable terminal rows
  from the **receiver's own** `runs` table. It is useful after receipt admission,
  but cannot discover or authenticate an unreceived source terminal. It carries
  no journal frontier, source epoch, seal digest or child barrier.
  `load_exact_run_invocation` proves local execution/config/checkpoint identity;
  it is not a remote terminal receipt and should not export private invocation
  configuration as a workaround.

### Source transaction and immutable evidence

Add a typed terminal-seal contract bound to source Store epoch, authenticated
source Host, conversation, exact Run and canonical assistant ownership. Persist
the terminal status, actual terminal commit identity/revision and completion
time together with the **complete required assistant-head set**. Each head uses
the existing full journal identity, sequence and digest. It must include owners
with no change in the final transition, not only the final publication groups.

The capture belongs in the same outer UoW as the accepted terminal canonical
transition and its complete journal publication set, after their resulting
heads are known. A publication/seal failure rolls everything back. Once sealed,
the owner set/frontier and terminal result are immutable; exact retries replay,
changed/missing/extra owners conflict. No later write may silently move a sealed
frontier. Resume/internal follow-up must have a separately admitted Run identity.

For unbounded owner sets, persist an immutable ordered manifest with a count and
digest and expose bounded pages tied to the seal. Do not reconstruct it from
the mutable latest-head query on every page, and do not substitute a full
conversation projection. A digest authenticates consistency only; transport
authentication and exact persisted source ownership remain mandatory.

Capture the child evidence actually required by Core's completion policy:
exact task/child-Run bindings, durable terminal-receipt identity, relevant input
and completion watermarks, and the committed parent consumption/reconciliation
fact. Do not infer a barrier from Task display state, current lease absence,
trace text, or “no active Run.” This does not introduce a blanket requirement
that every independent background task must finish before a foreground answer;
the required dependency set must come from typed execution facts. Failed and
cancelled terminals retain their distinct cancellation/settlement evidence.

### Recipient admission and recovery

Expose a narrow authenticated exact-Run seal query and bounded manifest reads
alongside the existing journal transport. Add only typed read requests, never
an arbitrary Application command endpoint. Local source gates must prevent a
Host from relabelling imported journals. Notifications announce the seal's
availability; they are not the terminal write authority.

The receiver first proves the existing remote Run/source/assistant binding in
its own database. It durably receives every required record/content through
the sealed heads and verifies the complete manifest and required child evidence.
Only then may a new Application receive-terminal command atomically persist
the immutable terminal receipt, update the exact remote Run/message statuses
and the necessary bounded remote session state. It must not rewrite transcript
bodies already owned by the journal or install remote execution authority.
Missing heads/child evidence leave the Run pending catch-up, never completed.

Repeat delivery is idempotent by exact seal identity/content. Source changes,
unknown owner sets, conflicting terminal status, wrong recipient Run bindings,
gaps, and digest conflicts fail closed. A crash between content catch-up and
terminal admission must resume from durable reception state and the seal, not
from an in-memory “done” bit. Replica/reconnect discovery must enumerate pending
exact Run seals even when no live notification was received.

### Consumer cutover and old-producer removal gate

1. Wire Domain/Store-port seal types, source persistence/read ports, receiver
   admission and Application command/recovery paths end to end. Cover atomic
   rollback, altered replay, omitted owner, child barrier, imported-source
   rejection, crash-before-terminal and cold reconnect cases.
2. Add narrow authenticated Host HTTP transport and a bounded notification.
   Update `MacAssistantJournalTransport`, `MacRemoteHostService` live decode,
   trace/resume catch-up and tool-plan retirement to fetch/receive the seal.
3. Add the typed receive/query adapters to `MacApplicationPort` and
   `MacCanonicalConversationRepository`. `MacWorkspaceModel` may finalize only
   from the locally committed exact terminal fact after seal admission, with
   its existing conversation/Run/generation checks. A dropped socket, journal
   cursor or final-looking Part still cannot finalize the UI.
4. Move context-usage/other bounded session observations formerly bundled in the
   aggregate to explicit typed reads or bounded committed state. Also migrate
   the iOS/Web consumers before deleting a shared Host producer.
5. Only after live, replica, cold-recovery and client terminal acceptance pass,
   remove the live `core_conversation_projection_updated` calls in
   `cli/src/core_agent_runtime.rs` and `core_agent_runtime_host_controls.rs`,
   its emitter method, and the terminal aggregate dispatch/consumer write path.
   Explicit user/history snapshot queries remain separate read operations.

### Existing implementation to reuse, not replace with parallel machinery

- `NolaStore::commit_transition` plus
  `assistant_publication_transaction::with_publication_transaction` provide the
  shared canonical/publication UoW. `assistant_journal/publication.rs::commit`,
  `source`, `validate_target` and the whole-set publication receipt already
  verify real source commits and exact owner groups.
- `local_assistant_journal_head` and `local_run_assistant_journal_heads` provide
  exact local authorship gates. Their latest-head enumeration is useful for
  discovery, **not** as a substitute for the immutable terminal manifest.
- `assistant_journal/receive.rs::authorize`, `reception_state` and `receive`,
  content staging, exact snapshot reads and journal cursor digest verification
  provide the existing bounded receive/catch-up mechanisms.
- `runtime_task_terminal_receipt`, `record_terminal_receipt` and
  `reconcile_runtime_task_bound_run_terminal` hold typed task/child terminal
  evidence. Their existence alone does not prove the parent consumed it.
- Exact bound-intent receipt acceptance (or authorized observed-receipt
  adoption), `assistant_message_id_for_run`, and remote-source tables provide
  recipient ownership. The old `begin_remote_turn` and aggregate import paths
  are removed; terminal admission must prove journal reception, not manufacture
  completion from a client-authored aggregate.
- After that atomic receive, the existing `canonical_run_terminal` query and
  Mac `reconcileCanonicalRunTerminal`/`confirmCommittedTerminal` are the right
  bounded UI completion boundary. They must not be invoked as if a notification
  had already supplied the missing durable terminal fact.

## 远程复制的接收窗口与并发（2026-10-28源码核对）

`assistant_journal/receive.rs`核对source Host → remote_run_sources → conversation/Run/assistant绑定。后继assistant可通过已绑定journal source读取，但不能换source_store_epoch；同Run在新源incarnation上出现不能自动沿旧游标接收。

接收状态是after/complete，不是“页面已打开”。after已超过through也必须验证through确实为历史已提交cursor，避免将随意较小序号当已完成。sequence=0必须是合法genesis；序号不是单独的可信凭证，必须连同record_digest和完整identity使用。

| record相对当前head | 校验 | 结果 |
| --- | --- | --- |
| sequence≤head | 查同owner/sequence的header并要求全等 | 返回Replay原cursor，不重新发表本地事件 |
| sequence=head+1 | previous_record_digest必须等于head摘要，batch/header及supplied chunks须合法 | 在事务内activate内容、apply frame、核对所得digest与源一致后提交 |
| sequence>head+1或前摘要不符 | 无法证明连续链 | gap/conflict，先补原缺口，不能跳head |

单次receive要求已有UoW；内容激活、记录公开、清理准备stage与conversation revision推进是同一提交结果。未完成的大内容stage不能成为公开Part或可读正文。模型/工具结果中的任意字节不得因客户端拼接而改变源record摘要。

并发例：观察者A取得chunk需求后下载，B先完成并提交同record，A随后返回。next_content和stage各自重新读取已提交状态：只在源授权、完整header以及前缀/结果/目标manifest匹配时幂等确认，不重建stage。若是另一record、其他Host或错误bytes，仍拒绝。客户端不应靠捕获错误字符串直接忽略冲突。

终态接收必须先具备精确admission及terminal manifest所需journal frontier。HTTP/SSE已经结束、当前head看似很新、最新summary已出现，都不能替代该完整性证明。冷开使用原Run/terminal身份；业务终态完成后outbox合法退休不等于身份消失。
