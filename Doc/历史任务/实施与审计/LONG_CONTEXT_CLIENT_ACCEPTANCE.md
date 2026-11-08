> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Long-context client acceptance

Active objective (2026-10-10): improve performance, execution flow and UI in
both macOS and CLI against the real long macOS conversation. Keep the CLI
session footer below the input throughout idle, execution, permission and
completion; remove decorative tree branches. Passing small fixtures alone is
not completion of this objective.

## Current acceptance map (UI/harness priority)

This map supersedes stale "remaining" statements in the chronological evidence
below; the original failures and measurements are intentionally retained.

| Requirement | Evidence now | Remaining boundary |
| --- | --- | --- |
| CLI footer stays below input; no decorative tree prefix | Current entire PTY suite passed 86 tests after request normalization/continuation changes (`nola-ui-harness-consolidated-cli.log`); recorded replay and footer units also passed | Terminal checks cover specified sizes/lifecycles, not every terminal implementation |
| Same recorded long history in both clients | Both read all retained messages; Mac verifies 138 parts / 259 fields / 550,419 bytes; CLI verifies exact repeated message values and bounded replay | Do not equate the CLI's 16-KB visible preview with full-history contents |
| Mac manual history scrolling and follow through completion/resize | Consolidated current-source scroll suite passed all seven tests, including recorded full-workspace paging/switching (`nola-ui-harness-consolidated-mac.log`) | Synthetic/recorded controls and generated real-provider recovery are complementary, not proof of every arbitrary conversation |
| Conversation-local composer/draft state | Native button real-provider switch test passed; native Return/delete race regressions passed | Cross-conversation admission is intentionally distinguished from displaying another conversation's Stop state |
| Interrupted output survives recovery without duplicated list/restarted paragraph | Forced once-only real-provider test passed in `nola-mac-real-memory.bRDtOe`; exact 1...60, completed delivery and completed persisted Run | One generated scenario is not universal recovery proof; ordinary 240-s latency gate remains separate from 480-s two-generation functional gate |
| Long response reader remains usable and complete | 46 transport/native-window tests passed in `nola-reader-empty-pages-tests.log`, including empty fields and actual paging/close/reopen | No claim that every settings/application window is covered by this chat-focused objective |
| Long-context responsiveness and memory | Warm Mac readiness around 233 ms; cold observed readiness 1.870 s after exact-memo reuse; recorded App RSS approximately 332 MB across cycles; both client process-tree baselines measured | Not a Release memory guarantee, exact FPS, or continuous peak; current focus is UI/harness rather than further cold-projection optimization |

All recorded-history operations are read-only replay into isolated copies, with
execution adapters that cannot resume captured pending work. Consolidated
current-source functional acceptance has completed; this is not a claim that
every UI or responsiveness issue is resolved. The reproduced history-page
wake-lateness tail (143 ms maximum, 2.15 ms p95) remains unattributed.

Follow-up priority is UI and the agent execution harness: conversation-local
controls, streaming presentation, scrolling ownership, and consistent execution,
permission, interruption/recovery and persisted progress state. Treat the
remaining history tail as a targeted investigation, not grounds for another
broad cold-start/cache redesign. Preserve the distinction between an individual
failed attempt and a failed final response. New fixes need observable behavioral
evidence; additional profiling without phase alignment is not acceptance.

## Recorded baseline

- Source conversation: `52CB0A1E-EDF2-4C43-B0AD-927A69BA48F3`.
- Consistent SQLite backup: `/tmp/nola-long-context.WwDcC9/source.db`.
- The backup is private local test data, not a repository fixture or a file to
  upload. Do not modify or replay the live database.
- At capture: 7 user + 7 assistant messages, 138 journal parts, 259 referenced
  body fields totalling 550,419 bytes (largest 47,226), 3,851 journal records
  totalling 1,665,108 batch bytes. Four runs completed, two failed, one running.
- A small number of messages can contain many independently updating parts.
  Message-count-only residency is not a sufficient performance contract.
- Never resume outstanding effects from this snapshot. Rendering/inspection
  tests must not execute commands, tools or model requests from captured runs.

### Storage versus current rendering cost

Read-only inspection of this backup found approximately 499.8 MiB in
`command_receipts`, 105.2 MiB in `sync_outbox`, and 78.2 MiB in
`projection_events`. These are whole-store disk allocations, **not** resident
memory or the selected conversation's body size.

The 6,049 `reduce_and_accept_host_response` receipts contain 218.01 MiB of
results in total, but recent hourly averages (2026-10-09 21:00 through
2026-10-10 02:00) are only 485–820 bytes. Historical growth must not be
mistaken for proof that today's streaming ACK still accumulates the body.
Existing service regression coverage asserts bounded streaming ACKs.

There are also 10,777 snapshot-open and 10,777 snapshot-release receipts.
The current Mac range reader opens/releases one snapshot per body range.
This is a concrete repeated-work candidate, not yet a measured attribution
of scrolling stalls. Snapshot capabilities must remain bound to their exact
cursor; optimization must not substitute a newer body during an older read.

The opt-in
`MacTranscriptCompletionScrollTests/testRecordedLongConversationProjectionAndBodyReads`
copies a private backup and reads the selected conversation through production
projection and journal APIs. Supply `NOLA_LONG_CONTEXT_FIXTURE_DB` and
`NOLA_LONG_CONTEXT_CONVERSATION` to the test runner. It never creates a workspace
execution driver, prints only counts/timings, and retains its isolated database
for receipt diagnostics. This measures data loading, **not** painted UI frames.

Measured 2026-10-10 with the unsigned Test product (original signed app and
source backup untouched): 14 messages, 138 parts, 259 fields, 550,419 bytes.
The phase-separated run passed and reported:

| Phase | Seconds |
| --- | ---: |
| Port/service construction | 2.411 |
| First conversation index query | 0.011 |
| First repository conversation load | 158.645 |
| Repeated repository conversation load | 7.196 |
| All journal body ranges | 0.774 |

Log: `/tmp/nola-recorded-phases-test.log`. Isolated database:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-recorded-read-661A186F-EE96-40D9-8352-EFEB0D41744E/fixture.sqlite`.
An earlier unsplit run also measured 160.328 seconds before body reads and
0.592 seconds for body reads. This is not a one-off body-rendering inference:
the next profiling target is the repository load, especially shared Core
window projection, pending checkpoint replay and presentation-input queries.
The first/repeated-load difference does not by itself prove which is responsible.

### Shared reducer profiling and first optimization

Repeated unchanged runs also measured approximately 5.94–6.00 seconds per
projection load; another first load recorded 158.68 seconds. These long wall
samples were later matched to system sleep; see the clock correction below.

`/tmp/nola-recorded-query-sample.txt` sampled the isolated service during the
query: `checkpoint_read_override` reconstructs the pending frontier, replaying
2,058 committed records. The dominant sampled reducer path copied the frozen
Host capability catalog in `annotate_concurrent_tool_batches` even when a
streaming delta produced no new requests. The implementation now returns
before constructing that catalog when fewer than two requests exist. This
preserves batching/permission semantics and also benefits CLI/live streaming.

- Host-contract batch regressions: 10 passed (`/tmp/nola-replay-batch-tests.log`).
- Same recorded Mac fixture after rebuilding: first projection 3.095 seconds,
  repeated projection 2.921 seconds, all body ranges 0.593 seconds; all 138
  parts and 550,419 body bytes remained readable.
- Log: `/tmp/nola-recorded-batch-fastpath.log`. This is an observed improvement
  versus the approximately 6-second runs, not a claim that all stalls are gone.
- Startup sampling separately showed synchronous work-archive publication;
  `/tmp/nola-recorded-projection-sample.txt` is startup evidence, not query
  evidence. Preserve the required file-based durable work records.

Remaining architectural issue: a historical presentation read still depends
on reconstructing the execution checkpoint frontier. Do not bypass its
freshness/ownership validation; move the required presentation facts to a
transactionally maintained read model or reuse an exact revision-bound result.
Real UI/CLI rendering and memory acceptance remain open.

### Cross-client recorded read and formatting baseline

Further Mac phase separation (`/tmp/nola-recorded-mapping.log`) measured a
single production window query at 1.500 s, Swift message mapping at 0.0045 s,
and settings read at 0.0004 s. The repository read pages the conversation and
measured 3.488 s initially and 2.788 s on repeat. These measurements locate
steady-state cost in shared reads, not Swift message conversion. The apparent
intermittent long first-load stall is qualified by the clock correction below.

CLI opt-in test `tests::recorded_long_context_cli_replay_baseline` now consumes
the same private snapshot through `load_interactive_session_snapshot` and
production replay/Markdown formatting functions. Run with the same two fixture
environment variables and `cargo test -p nola_cli --bin nola
recorded_long_context_cli_replay_baseline -- --ignored --nocapture`.
The test copies to an isolated temporary database, restores `NOLA_DB_PATH`,
and never constructs a turn driver or resumes captured effects. It asserts
both user and assistant content survive, without logging the content.

Latest passing result (`/tmp/nola-recorded-cli-baseline.log`): 14 total and
14 retained messages, 14 display commits (7 user and 4 assistant text commits),
2.529 s load, 0.897 ms replay projection, 34.730 ms Markdown/tool layout,
392 rendered lines / 16,314 bytes. This is the bounded CLI preview, not a claim
that its 16-KB rendered output contains every byte of the 550-KB full history.
Full-history access, real PTY scrolling and process-tree RSS still require
separate acceptance; formatter tests are not a substitute for those checks.

## Requirements and evidence gates

### Recovery read memo (implemented, bounded)

Shared Application checkpoint inspection now asks the repository for a pure
recovery-read memo before replaying the same committed frontier. SQLite keeps
one recovered checkpoint, with an 8-MiB JSON ceiling. It is usable only in a
pinned READ transaction, keyed by conversation, SQLite `data_version` and
`total_changes()`. A local write, another connection's commit, or a WRITE/no
transaction invalidates it. A memo is not a committed checkpoint and is never
consulted by effect execution or delivery. Other adapters may decline memoization.

The regression proves same-snapshot reuse, conversation isolation, external
commit invalidation, an old pinned reader's consistency during another writer's
commit, local write invalidation, rollback exclusion, and the size ceiling.
`/tmp/nola-read-cache-isolation.log` passed; `cargo check -p nola_application`
passed. The original schema and durable work files are unchanged.

Recorded Mac test passed with 138 parts and 550,419 body bytes unchanged:
repository loads measured 0.571/0.568 s after the first window. Two subsequent
unchanged runs measured first windows at 1.228 and 1.342 s, then repository
loads at 0.598/0.646 and 0.677/0.758 s. Logs:
`/tmp/nola-recorded-read-cache.log`, `/tmp/nola-recorded-stall.log`,
`/tmp/nola-recorded-stall2.log`.

CLI recorded replay also passed after rebuilding: 2.550 s first load and
71.782 ms repeated load, with exact equality of all retained message values.
Formatting produced the same 392 lines / 16,314 bytes. Log:
`/tmp/nola-recorded-cli-cache-warm.log`.

**Corrected interpretation:** the first run's isolated window recorded 153.587 s
of continuous-clock time. The power log shows a matching system sleep interval;
this is not evidence of a 153-second awake service stall. Cache reuse improves
awake repeated reads but must not be credited with eliminating system sleep.

### Clock correction: all three long wall samples overlap system sleep

Read-only `pmset -g log` evidence matches each anomalous run:

| Test log | Test interval | Sleep / wake interval | Reported sleep |
| --- | --- | --- | --- |
| `/tmp/nola-recorded-phases-test.log` | 03:35:20–03:38:09 | 03:35:21–03:37:55 | 154 s |
| `/tmp/nola-recorded-profile3.log` | 04:26:51–04:29:39 | 04:26:56–04:29:30 | 154 s |
| `/tmp/nola-recorded-read-cache.log` | 07:22:32–07:25:10 | 07:22:33–07:25:08 | 155 s |

All times are October 10, 2026, +0800. Sleep transition timestamps have only
second-level resolution, so do not subtract them to manufacture an exact awake
latency. They invalidate the previous inference of an unexplained ~150-second
query stall. The test now records both ContinuousClock and SuspendingClock for
the first window and whole run, flags sleep overlap over one second, and marks
sleep-contaminated performance samples invalid without skipping behavior checks.
No user power settings or production timeout policies were changed.

## Remaining acceptance gates

### Recorded production-view baseline (October 10)

The opt-in recorded test now hosts `MacTranscriptView` in an actual NSWindow,
without an agent driver. It waits for stable document geometry, manually scrolls
to history, verifies the position survives asynchronous layout, presses the real
accessibility action for return-to-latest, and checks the measured footer at
640 and 1440 point widths. This test passed. Screenshots were visually inspected:
`/tmp/nola-recorded-transcript-640.png` and `-1440.png`.
Log: `/tmp/nola-recorded-hosted.log`.

Initial hosted-view settling was 3.267 seconds (includes a deliberate stability
observation interval, **not** first-frame latency). Follow settling was 0.281 /
0.268 seconds (includes the test's waiting interval, **not** frame time).
Document heights were 25,474 / 22,979 points for only 14 messages. Sampled RSS
reached 333,328 KiB for the test app, with service samples of 189,072 and 18,320
KiB (`/tmp/nola-recorded-hosted-rss.log`). These sparse samples are a baseline,
not a guaranteed process-tree peak or a demonstrated memory improvement.

Visual findings remain open: failed-tool raw JSON dominates the transcript;
`Waiting for permission` and `compaction_resampling` leak untranslated/internal
status strings. The screenshots show content above the measured composer
clearance rather than a wholly blank completion viewport. This host intentionally
does not include the composer or resume the captured running work, so it does not
prove full-window switching or active-stream performance.

### Outstanding gates

Follow-control and initialization follow-up:

- Scroll profiling now separates native scrolling, layout and display, recording
  the worst sample index rather than attributing their total to drawing. In
  `/tmp/nola-recorded-scroll-phases.log`, worst samples were at index 0 and
  almost entirely layout. The previous 80 ms history-page spike did not recur
  in that isolated run (history layout maximum 12.46 ms).
- Extracted the follow button into its own SwiftUI observation boundary. Reading
  `followsLatest` no longer establishes that dependency in the body projecting
  every message row; appearance and action remain unchanged.
- The expanded suite exposed an actual initialization inconsistency: eagerly
  restoring the selected transcript bypassed creation of the bounded history
  window whenever no subsequent reload emptied it. Workspace initialization now
  restores sidebar metadata only; the existing async selected-conversation path
  owns both message loading and paging state. This also removes full transcript
  materialization from synchronous workspace construction.
- `/tmp/nola-window-initialization-regression.log`: 190 tests, 189 passed, one
  opt-in native folder-picker test skipped, zero failures. Includes the real
  recorded full-workspace paging/switch test and completion/reflow/manual-scroll
  tests. No sleep overlap. In this larger shared-process suite the history page
  still had a 78.33 ms layout sample at index 8, so the observer split is not
  evidence that all tail latency is fixed. Shared-suite RSS is not comparable to
  fresh-process baselines; body-loading/layout overlap remains to be isolated.

Recorded full-workspace opt-in (`NOLA_LONG_CONTEXT_FULL_WORKSPACE=1`):
`/tmp/nola-recorded-full-workspace.log` passed in 48.553 seconds, no sleep
overlap. The same private fixture now also enters `MacChatWorkspaceView` with
its inspector and composer. Persistence-only runtime/remote adapters prevent
execution of captured pending work; model/store/bookmark dependencies use the
private copy and unique defaults. Three history-to-draft-to-history cycles keep
the draft and selected composer state correct. Screenshot inspected:
`/tmp/nola-recorded-full-workspace.png`; captured permission/failure/recovery
states remain visible, and the sidebar shows the persisted cumulative cache
ratio rather than resetting it on navigation.

Full-workspace host resident bytes after each switch/scroll cycle:
324,403,200 / 324,796,416 / 324,976,640. Native layout/display sweep p95:
1.51 / 1.64 / 1.55 ms, maxima 20.05 / 21.35 / 22.69 ms. These are the production
**bounded latest window**, not all 14 messages mounted together; they must not
be compared as an improvement over the complete-transcript 9.94 ms p95 in the
same test. Older-page navigation is covered by the subsequent result below.

Recorded full-workspace paging passed in `/tmp/nola-recorded-history-pagination.log`
(49.515 s, no sleep overlap). The test presses the actual localized earlier-page
button, waits for a changed canonical message page, and verifies the union of
visited message IDs equals the full recorded archive. This fixture's older page
contains all 14 messages. It then presses the identified return-to-latest button
and verifies both the history state and original latest-window IDs are restored,
followed by the three draft-switch cycles. No captured execution is resumed.
The full-history page's 120 layout/display samples measured p50 7.12 ms,
p95 11.34 ms, maximum 79.73 ms, with 334,905,344 resident bytes. This exposes a
tail stall despite acceptable typical layout time; it is not proof of uniformly
smooth scrolling. The latest-window sweeps remain faster and are logged separately.
External one-second process-tree sampling observed a 964,128 KiB peak with
five processes, versus roughly 525–529 thousand KiB with three processes later
(`/tmp/nola-full-workspace-process-rss.log`). This first sampler did not record
child identities, so the transient peak's source is not established yet.
Repeat passed in `/tmp/nola-recorded-full-workspace-repeat.log`. The detailed
sampler `/tmp/nola-full-workspace-process-detail.log` observed a 652,848 KiB
peak and identified extra `nola-agent-host`, Codex and OpenCode processes.
This does not identify every contributor to the first run's larger peak.
Source inspection confirms `MacChatWorkspaceView` starts
`MacLocalExecutionBackendCatalog.fetch()` on appearance and configuration
notifications; `--list-execution-backends` probes external CLI versions,
authentication and models. That is a separate startup/navigation cost from
transcript rendering. The Rust single-backend discovery documentation already
reserves full discovery for explicit pickers. Mac now honors that boundary:
appearance, configuration notifications and selecting the local host no longer
launch full discovery. The execution-unit menu exposes a localized explicit
discovery/refresh action with in-flight deduplication, loading text and errors.
The built-in Nola entry stays available; a failed external scan does not replace
it with an unreachable placeholder. Stored external backend/model selections
remain unchanged, and an absent/changed catalog displays the saved model ID
instead of silently labeling it as a different default.

Post-change recorded full-workspace regression passed (one test, zero skips):
`/tmp/nola-recorded-no-auto-discovery.log`, result bundle
`Test-NolaMac-2026.10.10_10-45-57-+0800.xcresult`. Its 28 process-tree samples
(`/tmp/nola-no-auto-discovery-processes.log`) observed only the test application
and storage services, with sampled peak 566,512 KiB. Sampling began after launch,
so this is not an absolute startup peak; no FPS/steady-memory reduction is
claimed from this comparison. Saved external model identity regression passed
in `/tmp/nola-backend-display-regression.log`. Explicit discovery menu interaction
and loading/error presentation still need rendered interaction coverage.

Real-provider full-window regression rerun (October 10):
`MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible`
passed with zero skips in `/tmp/nola-real-switch-current.log`; isolated artifacts
are under `/tmp/nola-mac-real-memory.zRsbLK/runtime`. Inspected both PNGs:
the new conversation shows its unsent draft and no stop button while the other
turn runs, and the completed conversation shows item 60 and `NOLA-STREAM-END`
above the composer. The test asserts multiple incremental accessibility states,
switching during a live request, selected-composer idle state, and the terminal
viewport aligned with the latest anchor. This is a fresh 60-item answer in the
full workspace, not a real model run appended to the recorded long archive;
it does not measure gesture frame pacing or prove arbitrary long-history use.

Acceptance harness maintenance: the CLI readiness check previously searched
JSON transport notifications for answer text, although bodies now reside in the
canonical journal. It now reads persisted typed/chunked text and requires a
completed run plus the exact assistant marker. The reader has an offline
regression for UTF-8 chunks, inline typed text, role/order and session isolation
(`python3 scripts/test_real_cli_resilience_contract.py`, passed). The queued
sleep-only scenario explicitly selects its isolated allow-all profile: its
previous recommended-profile plus `--yes` combination correctly prompted for
shell permission and never reached queue delivery. No production permission
defaults or authorization rules were changed. The older Mac bubble-only test
also needed explicit test capability configuration and actual new-conversation
creation, not merely selection of an unknown conversation ID.

CLI real-provider queued-follow-up rerun passed after those harness corrections:
`/tmp/nola-real-cli-queued-contract.log`, isolated runtime
`/tmp/nola-real-cli-skill-permission.NU3ofJ`. Provider readiness, the real `bash`
sleep call, first answer, and queued second answer completed. Assertions compare
the exact durable four-message text order and check both responses in PTY output.
This complements the seven scripted-provider interaction cases below, not a
claim that every permission/recovery/live-provider case has passed.

The separate bubble/tool/Markdown real-provider scenario now also passes:
`/tmp/nola-real-rendered-canonical.log`, isolated artifacts
`/tmp/nola-mac-real-memory.7D1TVY/runtime`. One XCTest passed with zero skips;
post-run checks found one completed run, zero failed runs, zero open parts,
zero empty completed assistants and zero contradictory failure traces.
The pre-request failure was a missing explicit workspace grant in the fixture
(confirmed by activity diagnostics in `BJTMZX`), not provider latency. The test
now injects a unique bookmark-defaults suite and confirms only its canonicalized
temporary workspace; the suite is removed afterward. Production authorization
remains unchanged. The post-run validator also reads typed/chunked Text bodies
instead of incorrectly requiring the legacy inline content_text column. Offline
reader coverage includes genuinely empty bodies and exclusion of reasoning.
Inspected the successful tool/heading/marker screenshot from the preceding
`aym6Vz` run; that run passed XCTest but exposed the stale post-run validator.
This bubble-only screenshot does not establish full-transcript layout quality.

Latest CLI flow regression: seven real PTY tests passed in
`/tmp/nola-cli-flow-regression.log` (four footer/height cases, dynamic resize with
stream cancellation and next command, permission return without automatic agent
resume, and queued follow-up delivery after completion). These use scripted
providers and isolated projects; they are execution/interaction evidence but do
not replace the separate real-provider acceptance requirement.

Awake scroll baseline: the hosted recorded transcript now performs 120 native
scroll updates, explicitly laying out/displaying between updates and yielding
16 ms for normal event processing. `/tmp/nola-recorded-scroll-sweep.log` passed:
median layout/display work 6.67 ms, p95 9.84 ms, maximum 34.33 ms. Continuous and
suspending clocks both measured 27.209 s, so this sample has no sleep overlap.
These are CPU-side synchronous layout/display timings, not GPU frame pacing or
trackpad gesture latency. A one-second external sampler observed a combined
479,184 KiB across the isolated XCTest host and its two services (25 samples;
`/tmp/nola-scroll-process-tree-rss.log`); transient peaks between samples remain
unknown. Production Debug and original user data were untouched.

Completed-tool progressive disclosure: contiguous runs of at least three
successful tool parts now share a count row; any text, failure, cancellation,
unknown, running, pending or permission-waiting part breaks the run. Expanding
mounts the original tool rows, in original order, with all details intact.
Reference: local OpenCode `ContextToolGroup`; Apple lists-and-tables guidance
on succinct rows/detail disclosure informed the control. No capability-name
routing or execution changes were made. Pure grouping/order regression passed.
The recorded hosted test presses the actual localized group button, asserts
document growth, closes it, and verifies the original height returns; then runs
the prior resize/scroll checks. Passed: `/tmp/nola-recorded-tool-action.log`.
Its AX traversal ceiling is now 20,000 instead of 4,000 for long content, and the
button is located by its localized accessible label (identifier-only lookup
was insufficient in this hosted tree). The control exposes expand/collapse
state and a full-row 28-point minimum hit area. This fixture has few eligible
contiguous runs, so the height reduction is modest, not a broad speedup claim.

The remaining synchronous metadata merge/write was measured on the isolated
recorded database using the real repository save path: 4.12 ms
(`/tmp/nola-recorded-metadata-save.log`, passed). The async window read in the
same run was 313 ms with 27 UI-actor heartbeats. Do not attribute the earlier
150-second wall sample to metadata saving. A future
async writer must preserve ordering and cross-client settings fields rather
than fire-and-forget captured snapshots; this fast measured write is not the
current priority over mounted transcript layout and awake read measurements.

Completion/UI-thread fix: the final projection and bounded conversation reload
now use detached reads instead of synchronous service calls on MainActor.
`loadAsync` also reads sidebar/settings off-main and retries a stale snapshot
asynchronously rather than falling back to synchronous full-history loading.
Each new suspension in finalization validates the run generation and presentation
attachment before publishing; detached completion retains its existing path.
Nine selected tests passed in `/tmp/nola-offmain-completion-fixed.log`, covering
repository restart, completion side effects, detach/reattach and the recorded
window. Its async window read took 0.311 s while 27 main-actor heartbeat ticks
executed. Bounded window IDs match the canonical window, not the full archive.
This demonstrates responsive suspension for that read, not that every operation
in completion or startup is now asynchronous; persistence/rename and other
remaining synchronous call sites still need scrutiny.

Memory baseline (recorded fixture, October 10):

- CLI repeats exact snapshot equality and production formatting 20 times before
  its PTY replay. Darwin `getrusage(RUSAGE_SELF)` high-water RSS was 64,536,576
  then 68,272,128 bytes; the 20 iterations took 1.612 seconds. This is the test
  process only, not the service process tree (`/tmp/nola-recorded-memory.log`).
- Mac now reads actual resident bytes with `task_info` before/after three hosted
  window lifecycles. Only the first captures PNGs, avoiding repeated bitmap
  capture as a confounder. After closing cycles 0/1/2, resident bytes were
  267,632,640 / 269,598,720 / 270,450,688; later visible-window samples were about
  275–276 MB. All scroll and resize assertions passed
  (`/tmp/nola-recorded-mac-cycles.log`). The large first-window increase did not
  recur at the same magnitude, but three cycles do not establish absence of a
  leak. These are XCTest host numbers, not a release-app memory budget.
- Owner/body reuse caches were inspected: strong offscreen limits are 16 owners
  and 8 body prefixes per owner; mounted views legitimately retain their states.
  Thus further work should measure mounted content/layout and service allocation,
  rather than assuming the existing reuse dictionaries have no bounds.

Recorded CLI PTY replay now passed (`/tmp/nola-recorded-pty.log`). The ignored
fixture test starts a second copy of its test executable through macOS `script`,
loads a separate private database copy, and calls the production transcript and
composer renderer without entering the task-reconciling interactive loop.
Real PTY sizes 100x30, 40x15 and 120x40 are applied using TIOCSWINSZ; VT100 parsing
asserts one visible footer below the retained draft at each size, with no footer
tree prefix. Recorded transcript text may legitimately contain tree characters.
The harness explicitly paints invalidated frames, matching the production event
loop's separation of state changes and painting. First load was 2.524 s and
repeat load 73.376 ms. This validates recorded rendering/resize, not keyboard
editing, scrollback interaction, model streaming or process-tree peak memory.

Follow-up fixes verified by `/tmp/nola-recorded-status.log` (two tests passed):
failed tool calls no longer force their raw fields open on initial load or on a
failure transition. Their failure status and manual disclosure remain visible.
Added Chinese waiting-for-permission text and exact committed-code presentation
for `model_resampled` / `invalid_compaction_summary`; original attempt bodies
remain in diagnostics. `compaction_resampling` is a reason body, not the committed
failure code, so no body-keyword matching was introduced.
The same recorded fixture now measures 14,173 / 14,023 points at 640 / 1440 width,
down from 25,474 / 22,979. All 259 bodies / 550,419 bytes remain readable.
This reduces default layout volume, not proven frame latency or RSS. Subsequent
completed-tool grouping and CLI recorded-PTY results are documented above;
neither substitutes for full-window real-provider interaction acceptance.

1. Persistent CLI footer below input: reducer plus real PTY assertions during
   request latency, streaming, permission, cancellation, completion and resize;
   exactly one visible footer, correct cursor, no tree-branch prefixes.
2. Shared long-context fixture on macOS and CLI: load exact recorded content
   through production projection/rendering paths without executing captured work.
3. Performance: record first usable frame, main-thread/update stalls, steady and
   peak process-tree memory, repeated navigation/resize behaviour. Establish a
   baseline before selecting budgets; do not report an FPS improvement without
   measurement.
4. Display: long tool output, code, tables, reasoning, truncation/full-reader
   affordances, completion tail, manual scroll and return-to-latest. Inspect
   rendered evidence, not only model state.
5. Flow: captured failure/active states, permission ownership, cancellation,
   conversation switching, queued input preservation, recovery and exactly-once
   publication. Do not hide failures or broaden permissions as a workaround.
6. Real-provider regression complements (does not replace) recorded long-history
   acceptance. Tests use isolated state and never replace the signed running app.

## Current work

### Main-thread model-label storage read (2026-10-10)

The isolated recorded full-workspace run in
`/tmp/nola-recorded-profile-layout.log` passed (57.722 s, awake/wall clocks agree).
Its 40-second, 5-ms process sample is `/tmp/nola-layout-sample.txt`.
One main-thread SwiftUI header path has 190 samples in
`selectedModelCapabilityStatus -> configuredModels -> listModelProfiles ->
MacApplicationServiceClient.waitForSocket -> poll`. This is direct evidence of
synchronous storage access during view evaluation, not merely expensive text
layout. Sampling across the whole test does not attribute the 75.40-ms history
page layout spike to this specific call.

The capability label now uses the existing revision-bound model catalog. Its
status is populated from the same configured-model snapshot already read when
constructing that catalog; header reevaluation adds no service request. Model
configuration notifications still invalidate the catalog; backend changes still
hide the Nola-specific label. A regression changes the stored declaration, checks
100 label reads remain on the current snapshot, then explicitly refreshes and
checks the new declaration becomes visible. First catalog load and other model
configuration getters remain separate performance work; this change does not
claim all view-driven I/O has been eliminated.

Validation: `/tmp/nola-catalog-label-regression.log`, Test configuration, two
tests passed, zero skipped/failed. The recorded test passed in 50.634 s with
matching awake/wall clocks. History pagination still showed a 157.09-ms layout
spike (index 8; p95 combined step 10.84 ms), so this is **not** evidence that
scrolling stalls are solved. AppKit also logged a layout-reentrancy warning;
its source has not yet been attributed to production versus the hosted test's
explicit layout calls. Next investigation must distinguish that warning and
body hydration/layout work rather than infer success from the passing assertions.

Natural-layout control: `NOLA_LONG_CONTEXT_NATURAL_LAYOUT=1` disables explicit
layout/display calls within the 120-step scroll sweep (the setup/settle helper
still forces layout). Every sweep now also measures lateness resuming on the
main actor after a 16-ms sleep, using the awake clock. This is scheduling delay,
not GPU frame time. `/tmp/nola-natural-layout.log` passed the recorded test in
48.594 s, with matching awake/wall clocks. History-page native scroll work peaked
at 0.79 ms, but wake lateness reached 144.79 ms (p95 2.15 ms). Thus the long
history stall is not solely an artifact of explicitly forcing each scroll-step
layout. The layout-reentrancy warning also persisted, though setup/settle still
prevents attributing that warning to production alone. This evidence prioritizes
hydration-triggered UI work over optimizing native scroll-position updates.

Follow-up samples (`/tmp/nola-natural-sample.txt`, 1-ms sampling, and
`/tmp/nola-history-sample.txt`, 5-ms sampling) show synchronous SwiftUI graph
flush/layout under `MacJournalBodyState.markdownProjection.setter`, including
the initial empty projection assignment. The body reader now constructs and
updates a local value before assigning that observable projection once, avoiding
the intermediate empty semantic tree. Existing node identity/stream-finalization
logic is retained. This does not yet consolidate all body-state fields into one
publication.

The first sample also exposed `MacThinkingActivityFormatter.currentStep` doing
whole-history sentence normalization during view evaluation. It now searches
backwards for the last nonempty segment rather than allocating normalized strings
for every earlier sentence. Tag removal and the punctuation-only fallback are
unchanged. Equivalence coverage compares the previous algorithm across multilingual
punctuation, whitespace, combining characters/emoji, four length limits, and a
10,000-sentence history. Profiling overhead affected the sampled run; its timings
must not be used as an uninstrumented performance baseline.

Validation: `/tmp/nola-hydration-regression.log` passed all four selected tests
(summary equivalence, hydrated identity preservation, no terminal replay, recorded
full workspace). The natural history sweep measured 2.17-ms maximum wake
lateness in this run, versus 144.79 ms previously. This is encouraging but needs
a fresh-process repeat: hydration timing can move work outside the sampled
sweep, and the three preceding tests make this run's ~866 MB workspace RSS
incomparable with the fresh-process ~332 MB baseline. The layout-reentrancy
warning remains unresolved. Do not claim memory improvement or final latency
acceptance from this single regression run.

Fresh-process repeat `/tmp/nola-hydration-fresh.log` passed but reproduced a
156.37-ms history wake-lateness spike; full workspace RSS was ~333 MB. This
contradicts treating the preceding 2.17-ms peak as a stable latency improvement.
Inspection of the preview shows a second intermediate-render issue: assigning
`text` before `markdownProjection` allows the fallback `MacMarkdownView` to see
new text without its semantic nodes. `MacJournalBodyState` now publishes text,
semantic projection, artifacts and load completeness as one observed snapshot,
including one reset on owner change. Generation/cancellation checks and stable
semantic-node assembly remain unchanged. Validation is recorded below once the
current Test run completes.

Snapshot validation: `/tmp/nola-body-snapshot-regression.log` passed 44 tests,
zero failures (41 transport/body tests, two identity/completion tests, recorded
full workspace). A residency test that directly assigned the now-read-only text
was updated to retain user-owned revealed-node state instead; real hydration
and stale-read tests still cover text. History wake lateness was 2.23 ms in this
shared-process run. As the preceding fresh-process repeat disproved the same
kind of warm-run improvement, this is correctness evidence, **not** cold-history
latency acceptance. A fresh-process repeat and the layout-reentrancy investigation
remain required.

Visual follow-up: inspected `/tmp/nola-recorded-full-workspace.png`. The recovery
surface appears to retain excess vertical space above the composer. Its measured
height/preference propagation needs a rendered short/expanded-content regression;
this is an open UI finding, not yet a proven sizing root cause.

Recovery-height reproduction: `testInterruptionSurfaceFitsShortContentAndBoundsLongContent`
hosts a real window and the same vertically fixed-size composer container.
A simple fixed child passed, but adding conditional diagnostic disclosure
reproduced all six assertions failing: viewport height stayed at the preference
default 160 pt for 64/400/48-pt content and collapsed/expanded/collapsed details
(`/tmp/nola-interruption-disclosure.log`). The surface now directly observes
content geometry instead of propagating a background height preference through
the scroll/disclosure hierarchy. The stale 160-pt default and its preference key
are removed; the existing maximum-height cap is preserved. This addresses both
wasted space for short notices and incorrect capacity for long notices.

Fix validation: `/tmp/nola-interruption-fixed.log` passed all six transcript
completion tests, including all short/long/disclosure height assertions and the
recorded full workspace (no failures/skips). Inspected the regenerated
`/tmp/nola-recorded-full-workspace.png`: the unused band between recovery actions
and the composer is gone, with more transcript visible; recovery text and the
status-check action remain accessible. History wake-lateness maximum was 6.95 ms
in this shared-process run, which is not a replacement for cold-process latency
acceptance. The unrelated layout-reentrancy warning is still present.

Layout-reentrancy investigation: attaching LLDB to the isolated Test process was
denied by macOS (`/tmp/nola-layout-backtrace.log`); no security settings were
changed. Natural-layout mode now also avoids forced layout/display in `settle`,
not only in the sweep. `/tmp/nola-all-natural-layout.log` passed but still logged
the warning and a 153.64-ms cold history scheduling spike. Thus the test's direct
`layoutSubtreeIfNeeded` calls are not required to reproduce it (other system/AX
layout activity is still present).

Code inspection found `MacComposerTextEditor.sizeThatFits` modifying the live
NSTextContainer size and forcing its layout during SwiftUI size negotiation.
It now measures an independent attributed-text/TextKit stack, without touching
the live view/container. A regression verifies width-dependent wrapping, the
132-pt cap, the empty 30-pt height, and unchanged live frame/container/text.
This removes a size-measurement side effect; whether it accounts for the warning
requires the full-window result, not inference from that unit test.

`/tmp/nola-composer-measurement.log`: seven tests passed, zero failures/skips;
the regenerated full-window screenshot retains the composer/hint and compact
recovery area. The warning **still occurs**, and the recorded history sweep
peaked at 161.78-ms wake lateness. The measurement change is therefore not the
warning's proven fix and does not close performance acceptance. Further work
must locate another source, rather than repeating this hypothesis.

Body-stage instrumentation (`NOLA_DIAGNOSE_BODY_HYDRATION=1`, Debug only) records
preparation versus snapshot-publication durations above 5 ms, with byte counts
only. `/tmp/nola-body-timing.log` reproduced a 166.01-ms cold-history scheduling
spike but logged no body stage above that threshold. This excludes a single
measured parse/publication call as the direct source in this run; deferred UI
work or multiple smaller operations remain candidates. Startup-wide sampling
can miss that short interval, so the opt-in `NOLA_PROFILE_HISTORY=1` test flag
prints/flushed a phase marker and waits two seconds before pressing the history
button. It is a sampler rendezvous, not production behaviour or a timing result.

Targeted history sample `/tmp/nola-targeted-history-sample.txt` finally captures
the interval: the main thread has 227 samples in
`MacInspectorWorkingContextView.body -> MacWorkspaceModel.memoryEnabled ->
memoryEnabledDefault -> productSetting -> waitForSocket -> poll`. This is a
separate synchronous display getter, not body parsing. The profiled run passed
but sampling stretched its peak to ~1.18 s, so only the stack attribution is
used. The inspector now loads the global memory default asynchronously off the
main actor and holds a display-only value. Configuration revision changes
refresh it; setting the global memory default emits the existing configuration
notification. A cancelled view task cannot publish its result. Runtime
`memoryEnabled` resolution and conversation override semantics are unchanged.
Regression coverage changes the global value both ways while preserving an
explicit conversation-level disable.

`/tmp/nola-memory-display-regression.log` passed both tests. However cold history
still reached 166.38-ms wake lateness, with ~333 MB full-workspace RSS. Removing
this sampled blocking getter does not eliminate the stall. Other synchronous
inspector reads (including per-source override resolution) remain in the sampled
UI paths; the next useful diagnostic is duration/call-stack attribution at the
shared main-thread service boundary rather than another isolated getter guess.

Shared-boundary diagnostic `NOLA_DIAGNOSE_MAIN_SERVICE=1` (Debug only) logs
main-thread service calls over 5 ms with call stacks, without request payloads.
`/tmp/nola-main-service-timing.log` identifies a 1.04-s
`adoptColdLocalRun -> loadExactRunInvocation` call on the UI thread. Startup
health checks and the test's own deliberate synchronous baseline reads are
logged too and must not be confused with rendering getters. The cold-recovery
invocation read now runs detached; its result/error is accepted only for the
same active conversation, generation and recovery run. Navigating away does not
transfer recovery to the newly selected conversation, nor cancel legitimate
background ownership. A finalized run cannot receive the delayed result.

`/tmp/nola-cold-recovery-async.log` passed the scripted cold-permission recovery
end-to-end test and recorded long-window test. The exact-invocation read is no
longer the main-thread cold-adoption stall. The same diagnostics now identify
the subsequent `checkLocalRunRecovery -> inspectRunRecovery` as a 1.20-s UI-thread
call; that remaining recovery inspection must also be asynchronous with the
same generation guards (including its error path). History sweep wake lateness
still peaked at 169.26 ms, so neither startup recovery nor scrolling is declared
fully solved.

Recovery inspection follow-up: cold-run ownership and `inspectRunRecovery`
now have background repository entry points. Cold adoption checks the original
generation after suspension before establishing a recovery owner. The explicit
status-check flow verifies the exact recovery identity after inspection and
after runtime-resumability checks; delayed errors are similarly guarded. This
closes the remaining measured synchronous inspection path without making reads
resume commands or changing canonical terminal/permission authority.

Recovery-chain verification (`/tmp/nola-recovery-chain-async.log`): all 18
selected tests passed, including cold permission recovery, 16 terminal/cancel
contracts, and the recorded full workspace. No measured main-thread service
call remained in `adoptColdLocalRun` / `checkLocalRunRecovery`. This is not a
blanket responsiveness pass: natural-layout history-page scheduling still
peaked at 146.40 ms; it is not GPU frame timing.

The same diagnostic found permission hydration in
`loadSelectedConversationIfNeeded` synchronously blocking the main thread for
1.03 s (active owner) and 0.94 s (projection). Permission hydration now runs
off-main, preserving the before/after run-identity check and propagating task
cancellation. Publication checks selection revision, selected conversation,
presentation attachment, cancellation, and active-turn ownership after the
await, including refusing to replace permission state updated by a live event
during the read. It does not grant permissions or execute captured tools.
`/tmp/nola-permission-read-async.log` passed all 21 permission tests, selection
isolation, and the recorded full workspace. The measured cold recovery/load
paths no longer logged main-thread service calls over 5 ms. History scheduling
peaked at 4.76 ms in this particular run; fresh-run consistency is still unproven.
The overall run failed one other test (three assertions): canonical admitted
rows did not appear within the existing 120 ms bound. A repeat without the
diagnostic (`/tmp/nola-permission-read-final.log`) reproduced that failure.

Admission wake-up finding: `applyJournalCursor` requested observation on the
first committed owner, but `startProjectionObservation` reused the existing
observer even while it was in its missing-conversation retry sleep. The first
committed cursor for a missing row now cancels that observer before replacement;
the coordinator still drains the old task before publishing from its successor.
Ordinary selection and subsequent cursor deltas continue reusing observation.
The unchanged 120 ms regression now passes. Final source verification in
`/tmp/nola-admission-wake-regression.log`: 30 tests passed, including 21
permission tests, seven observation-coordinator tests, superseded cold-load
selection isolation, and the formerly failing admission test. This confirms
the specific wake-up fix, not full-app long-context performance completion.

Presentation-read cancellation follow-up: several repository async APIs awaited
detached tasks without forwarding cancellation. Leaving a page could therefore
abandon its result but still perform remaining history-page reads, token
estimation, and metadata reads. `MacConversationReadTask` now forwards parent
cancellation, checks before starting work and before publishing, and drains an
already-running synchronous service call. Cold owner/recovery reads, display
projection, canonical metadata, full-history loading, search, and permission
hydration use it. Full-history paging and permission read stages check
cancellation between calls. This affects presentation reads only, not execution
or durable authorization. It does not promise interruption of an in-flight
synchronous service request. Deterministic gated regressions exercise skipped
work, off-main execution, cancellation during a call, and independent replacement
reads. The initial wider regression (`/tmp/nola-read-cancellation.log`) also
exposed two tests that assumed model construction eagerly loaded messages.
They now await the production async load and compare against the canonical
repository/actual journal body rather than treating the initial empty UI as
the expected stored transcript. No content assertions were removed.

Final verification: `/tmp/nola-read-cancellation-final.log`, 42 tests passed,
including the recorded full workspace (private copy, no tool execution), all
product-path integration tests, permission tests and observation lifecycle.
History-page wake lateness still peaked at 84.51 ms, so the intermittent stall
is not resolved. RSS was 357.3 MB with full history visible and approximately
356 MB after switching; this mixed-suite observation is not a controlled
before/after memory improvement claim. Cancellation tests prove avoiding
remaining work, not reclaiming memory from a service call already executing.

Code-block hotspot follow-up: the targeted history sample contained 31 samples
in `MacCodeSyntaxHighlighter`. Inspection showed all four regexes compiled
inside each line render. Rules now compile once per document (four compilations
instead of 2,000 for 500 lines), with no retained source-content cache. Token
priority, semantic colors, line numbers and selected-line shading are unchanged.
New coverage checks 500-line Swift/Python/TypeScript/C++ blocks, Unicode,
keyword/literal/string/comment colors, and selected-line shading. The existing
opposite-appearance test remains. Verification passed in
`/tmp/nola-code-highlighting.log`, including the recorded full workspace. Each
500-line highlighting case took 8.4–9.2 ms; the history-page wake-lateness peak
was 4.78 ms in this run. RSS with all recorded history visible was 331.5 MB.
These are computation/scheduling measurements, not GPU frame times; previous
fresh-run variability means they do not yet prove all layout stalls eliminated.

Independent-process repeat (`/tmp/nola-long-cold-repeat.log`, same built Test
binary, no concurrent build) passed. History-page wake lateness peaked at
2.16 ms; after three new-draft/return/scroll cycles it peaked at 18.61–21.83 ms.
RSS was 330.9 MB with all recorded history visible and 331.9 / 332.2 / 331.8 MB
after the three cycles (no monotonic growth in this short run). Awake and wall
times both 46.734 s exclude sleep contamination. Crucially, AppKit still logged
`_NSDetectedLayoutRecursion` at 12:20:45.981, despite the low history-page
scheduling peak. Treat that warning as a separate unresolved layout defect,
not evidence that the 150 ms spike persists or that all layout is now correct.

Layout-reentry attribution: `/tmp/nola-focus-diagnostic.log` reproduced the
warning without any composer-focus call, ruling out that suspected path. The
temporary composer markers were removed. An opt-in XCTest-only AppKit method
probe then captured same-view reentry in `/tmp/nola-layout-reentry-probe.log`:
`NSThemeFrame -> setStyleMask -> MacChatWindowChromeConfigurator.WindowAttachmentView.applyWindowConfiguration -> viewDidMoveToWindow`.
The view attachment happened inside SwiftUI's existing window layout. This is
an application call-site bug, not merely an unexplained system warning.

Window chrome configuration now coalesces onto the next main queue turn. It
resolves the current window and latest title at delivery, skips detached views,
and still applies style/toolbar configuration only once per attached window.
Lifecycle tests cover deferred attachment, title-only updates without resetting
toolbar, rapid reparenting, and detachment before delivery. The same recorded
window with the reentry probe passed in `/tmp/nola-window-reentry-fixed.log`:
three tests passed; the probe observed 875 layout calls and zero same-view
reentries, and the AppKit warning was absent. Before the fix the same probe
captured `NSThemeFrame` reentry and the warning. The recorded history scheduling
peak was 2.14 ms (awake/wall clocks agree). Inspected the resulting full-window
screenshot: title, panes, recovery controls and composer remain visible. This
resolves the attributed chrome-attachment warning; it is not a claim that all
performance/display/flow acceptance gates for both clients are complete.

CLI current-source revalidation: `/tmp/nola-cli-footer-current.log` passed four
real-PTY footer cases (English/Chinese completion, populated second turn,
mobile-height changes with incremental Chinese typing). Extended the existing
permission/resize test to assert exactly one visible `Context` footer below
the permission controls at both 46x16 and 118x32, with no tree-branch prefix.
`/tmp/nola-cli-flow-current.log` then passed that strengthened test, dynamic
resize with streaming cancellation/next command, and queued follow-up delivery.
The digit-only permission choice returns to the composer and does not resume
the rejected operation. Seven focused PTY cases passed in total; providers are
scripted and projects isolated. This is current terminal flow evidence, not
a blanket claim that every CLI command or real-provider path was retested.

Inspector data-source display: each row previously synchronously read its
conversation override both while rendering and from the Toggle getter.
The inspector now loads a bounded source-ID snapshot off-main, scoped to the
selected conversation and local override revision. It refreshes on edits,
source-list changes, reattachment and application activation. Controls cannot
use another conversation's snapshot while loading; failures leave them disabled
with the read diagnostic available as help. Global policy/validation still use
the current observable registry state. Runtime `resolution()` retains its fresh
canonical override read and never consumes this display snapshot.

New regression verifies separate conversations, explicit false versus inherit,
revision invalidation, and fresh runtime denial while an old display snapshot
still contains true. All five selected tests passed, including the recorded
full workspace, in `/tmp/nola-data-source-display.log`. No over-5-ms main-thread
service diagnostic referenced the data-source inspector/override getter.
History wake lateness peaked at 2.17 ms; no layout-reentry warning appeared.
This mixed diagnostic run's RSS remained about 371 MB over three cycles; it
does not establish a memory reduction relative to a different test process.

UI audit finding from the recorded screenshot: the overview header used
only `isSelectedConversationVisiblyRunning` to choose Running/Ready. The
captured unresolved local run consequently shows Ready beside the recovery
surface saying its execution result is unknown. This needs a selected-session
status projection, not a global activity spinner or changed run authority.

Implemented `selectedConversationDisplayStatus`: selected-session loading,
running, permission/input wait, recovery checking, unconfirmed result,
cancellation and final presentation reconciliation are distinct. The overview
uses localized text plus a semantic status color. Recovery belonging to another
conversation does not become the new draft's status. This projection performs
no storage read and never changes execution/cancellation/permission authority.
Regression assertions extend actual local EOF recovery, remote permission
switching, submission preparation, and the captured full workspace's rendered
accessibility text. Initial verification (`/tmp/nola-session-status.log`)
exposed a real transient: selecting any empty conversation marked it loading,
including unsaved drafts with no canonical history. Selection now marks loading
only for persisted conversations, and a nil selection never means loading.
The rendered-status test also reads AXValue as well as AXLabel, since native
static text exposes its content through AXValue. Final verification passed in
`/tmp/nola-session-status-final.log`: 41 tests, zero failures, including local
transport-loss/terminal contracts, remote permission ownership, new-draft
submission state and the recorded full workspace's actual status text. History
scheduling peak was 6.50 ms, and three switch cycles retained about 340–342 MB
in this process. These results validate the status fix, not all-client completion.

- Implemented footer ownership by the session surface, independent of emitter
  teardown; moved active footer below composer and retained it during permission.
- Removed footer branch decoration; updated assertions that previously required
  footer disappearance (that old expectation conflicts with the objective).
- Terminal renderer unit suite: 48 passed. Context-footer real PTY tests: 3
  passed; mobile-height resize/draft/footer PTY regression: 1 passed.
- Numbered permission-mode choices select and confirm immediately (no Enter
  required); arrows/Tab only move selection, Enter confirms, Esc cancels.
  Repeat/release events and Ctrl/Alt-modified digits do not confirm. Unit
  regression passed; digit-only return-to-composer real PTY regression passed
  (subsequent message accepted, cancelled tool not automatically resumed).
- Read local Codex bottom-pane ownership and OpenCode session scrollbox/prompt
  separation. Apply the ownership distinction; do not copy their styling blindly.
- Recorded data-read and formatter harnesses, hosted long-history Mac UI,
  recorded CLI PTY resize, and repeated-load/window RSS measurements now pass
  on both clients as detailed above. Full application-window interaction and
  current real-provider regression remain open; hosted views and short
  streaming/switch tests do not alone prove full-app long-history performance.

### Full CLI PTY sweep and startup-lease correction (2026-10-10)

`/tmp/nola-cli-pty-full.log`: 86 real-PTY scenarios completed, 78 passed and
8 failed. This is not a green full-suite result. The failures were inspected
individually, preserving their private fixture databases and diagnostics:

- Broad-scope guidance used the compact prompt contract rather than the old
  expanded wording. The test now checks the compact exhaustive-scope ledger
  requirement; plan publication, truncated discovery and narrower continuation
  assertions remain unchanged.
- The ordinary-turn test selected the last repaint of the submitted prompt,
  incorrectly treating it as the start of the turn. It now checks from the first
  occurrence and retains actual visible single-answer/editable-composer checks.
- Queued steering is acknowledged by queue state, not an echoed copy of the
  input. The crash test waits for that state and still independently verifies
  canonical persistence before terminating the process and exactly-once use
  after restart.
- Memory save/search and general-shell validation require scoped permission.
  Fixtures now explicitly approve each protected call using a digit alone.
  Reversible structured writing must finish before shell approval, and the
  subsequent destructive command remains denied with the fixture file intact.
  The second panel is checked on the visible screen, not against an earlier
  permission panel retained in raw terminal history. No production permission
  default or authorization check was relaxed.
- System sleep contaminated background scheduling timing. `pmset` recorded
  repeated roughly 150-second sleep intervals. A process-lifetime `caffeinate
  -is` assertion was used for rechecks, not a persistent power-setting change.
  Twelve-task contention and two-idle-client fairness passed unchanged on that
  recheck. This does not prove scheduling across machine sleep is complete.
- The worker-kill recovery failure reproduced while awake and was a real
  startup-handoff defect: lease expiry was 1791611231996 ms, while the command's
  started event was recorded at 1791611231920 ms, leaving only 76 ms. The first
  heartbeat was incorrectly scheduled relative to completion of startup, so
  the worker lost ownership before the test could inject its kill. The worker
  now renews the same transported attempt with fresh CAS before starting
  effects and measures heartbeat elapsed time from that renewal. Expired or
  replaced identities are not revived.

`/tmp/nola-cli-pty-failure-recheck.log` passed five of the eight original
failures. After the startup-lease correction and remaining permission fixture
updates, `/tmp/nola-cli-worker-permission-recheck.log` passed all three remaining
cases, including real worker-kill reconciliation without side-effect replay.
Thus every original failure has a passing focused recheck; the entire 86-case
suite has not yet been rerun on the final source. Startup-lease unit coverage
also extends the fresh-identity case to run beyond its transported expiry while
retaining rejection of expired/replaced identities. That regression passed in
`/tmp/nola-shell-startup-lease-unit.log` (one test, 3.43 seconds).

### Native Return dispatch and process-tree memory baseline (2026-10-10)

The broader Mac interaction run (`/tmp/nola-mac-window-flow-final.log`) passed
28 of 29 tests and exposed a real native-input race. The NSTextView delegate
commits the draft synchronously, but Return checked `state.canSend` from the
previous SwiftUI display snapshot. Typing into an empty editor and immediately
pressing Return could therefore do nothing. The keyboard action now goes to
the existing action sink, which checks the current selected conversation and
current composer state. A rejected action does not stop dictation; an accepted
one does. The button's disabled presentation remains snapshot-based. No model
submission/attachment/recovery authority was weakened.

`/tmp/nola-mac-native-submit-final.log` passed all 35 selected tests: native
editing and immediate submission, compact permission/question layout,
dictation ownership, full-reader keyboard paging and byte preservation,
independent history windows, inspector tab state, window chrome, submission
preparation and recovery blocking. The apple-design review's relevant principles
were standard keyboard behavior and responsive interaction (`keyboards.md ›
Best practices`, `loading.md › Best practices`); no cosmetic redesign was added.

The recorded full-workspace test then passed separately with the same private
fixture: `/tmp/nola-recorded-process-tree-run.log`. It read all 14 messages,
138 parts, 259 bodies and 550,419 bytes, and completed three full-workspace
draft/back/scroll cycles. Their App RSS was 332,464,128 / 332,808,192 /
332,414,976 bytes. No captured execution was replayed.

An external observer reused `process_rows` from
`scripts/measure_mac_release_memory.py`, attaching only to the exact
`Products/Test/Nola.app/Contents/MacOS/Nola` process and enumerating its
descendants. It did not launch another app, include Xcode, modify the developer's
app, or terminate the observed process. `/tmp/nola-recorded-process-tree-memory.jsonl`
contains 43 approximately one-second samples through process exit, with both
RSS and `/usr/bin/footprint` physical footprint for the App and its two service
children. Sampled peak sums were 547,618,816 bytes RSS and 432,291,840 bytes
physical footprint (not necessarily at the same sample). At the final sample,
the sum was 303,382,528 bytes physical footprint: 187,695,104 App,
110,100,480 fixture service, 5,586,944 other test-host service.

These are sampled process-tree sums, not a continuously measured absolute peak,
and RSS can count shared pages more than once. This is a baseline for the Test
configuration including XCTest, not a standalone Release memory claim. The
external footprint observer may perturb scheduling; the 151-ms history wake
outlier in this run is retained but is not used as a latency improvement or
regression verdict. Awake and wall clocks agreed (48.119 seconds), ruling out
machine sleep for this sample, not ruling out observer overhead. CLI tree-memory
comparison, a clean final full-PTY rerun, and remaining whole-app acceptance
still remain open.

### CLI recorded replay process-tree sampling (2026-10-10)

The current read-only replay harness passed again in
`/tmp/nola-recorded-cli-rss-run.log`: 14 retained messages, 14 ordered commits,
392 rendered lines / 16,314 bytes, and real PTY verification at 100x30, 40x15
and 120x40. Each size retained one undecorated footer below the draft. Twenty
load/render repetitions preserved exact saved messages; process peak RSS went
from 64,471,040 to 68,157,440 bytes. Cold load was 2.609 seconds and warm load
76.666 ms, with the external RSS observer and the independent PTY regression
suite running concurrently, so these timings are not a clean latency comparison.

The initial footprint observer stopped when a short-lived child exited during
inspection (`/tmp/nola-recorded-cli-tree-memory.jsonl`). That partial trace is
not accepted as a whole-run peak. The existing memory script now supports an
explicit RSS-only snapshot mode for transient process trees; default detailed
footprint/budget behavior remains strict. Two unit tests verify transitive
descendant selection without unrelated processes and that inspection failures
cannot silently become zero-valued successful memory-budget samples.

`/tmp/nola-recorded-cli-tree-rss.jsonl` then captured 59 lightweight samples
through root process exit. Peak tree RSS sum was 138,805,248 bytes, consisting
of the test parent (68,255,744), actual PTY replay child (67,813,376), `script`
(1,163,264), and the process-lifetime sleep inhibitor (1,572,864). The parent
and child intentionally hold separate private fixture copies, so the sum is
test-harness cost, not normal single-CLI residency. No agent/service execution
was resumed. This supplements the per-process high-water mark; it is not a
standalone interactive-run or continuous absolute-peak measurement.

Final full CLI PTY revalidation is running in
`/tmp/nola-cli-pty-final-awake.log` with process-lifetime sleep inhibition and
one test worker. No completed full-suite verdict is claimed yet.

### Real-provider stream-switch acceptance: newly exposed recovery defect

The isolated real-provider run in `/tmp/nola-mac-real-memory.YGvJsh/runtime`
passed its original stream/switch/terminal-scroll assertions. That is **not**
a complete content acceptance: manual accessibility inspection found 75 numbered
items for the requested 60, with numbering restarting after the first 15.
The journal contains two public final text parts (1,308 and 7,444 UTF-8 bytes),
so this is not merely a duplicated accessibility subtree. Provider metadata
records two requests: the first has no terminal finish reason; the second stops
normally. The new-conversation screenshot separately confirms that its unsent
draft does not inherit the other conversation's running composer state.

The traced recovery path maps transport interruption to `output_limited`, commits
the partial text, and requests a continuation. Core's continuation guard compares
normalized text and a 160-character prefix; it cannot establish that paraphrased
restarted content is a valid continuation. Do not patch this with numbered-list
or Chinese-keyword heuristics, or silently delete retained canonical content.
Recovery semantics and content verification remain an open acceptance gate.

`testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible` now checks the
actual rendered numbered sequence against this test prompt's requested 1...60,
in addition to the existing switch, incremental publication, end-marker, and
viewport assertions. Applying that sequence check to the saved real artifact
fails (restart indices 0 and 15), as intended. Swift frontend parsing passed;
the strengthened XCTest has not yet been rebuilt/rerun. This explicitly supersedes
any claim that the prior passing test proved a coherent, nonduplicated answer.

### Request-order correction and completed CLI sweep

The final pre-correction CLI PTY sweep finished: **86 passed, 0 failed** in
724.02 seconds (`/tmp/nola-cli-pty-final-awake.log`). This includes the
ten-million-character restore, interrupted-stream continuation, durable worker
recovery, permission interactions, footer/resize, and background fairness tests.
It does not certify subsequent request-adapter changes.

Further inspection found that Chat Completions normalization hoisted **every**
system-role message, including the late Core continuation instruction, into one
initial system string. This changed contextual ordering and the cached prefix;
structured system content was also serialized into text. The adapter now keeps
Core's instruction roles, ordering, content types, and boundaries, while retaining
tool-sequence repair, adjacent assistant-fragment merging, and media limits.
Regression tests cover late instructions, structured system content, stable
previous-request prefixes, and idempotent normalization. Their build/test run is
recorded in `/tmp/nola-openai-order-regression.log`: **14 passed, 0 failed**.
This fixes an observed adapter defect, not proof that all semantic continuation
restarts are resolved; the strengthened real-provider UI acceptance remains due.
The fresh Test-configuration rebuild and real-provider rerun are in progress in
`/tmp/nola-real-stream-order.log`; no post-change real-provider verdict is claimed.

The CLI interrupted-visible-stream PTY fixture now also inspects the actual JSON
request received by its server: the continuation instruction must occur **after**
the retained assistant prefix. Its earlier presence-only assertion could pass
even when the adapter moved the instruction into the initial system message.
Post-change PTY verification passed: **1 passed, 0 failed**, 10.37 seconds,
in `/tmp/nola-wire-continuation-pty.log`.
Both this process and the Test rebuild were confirmed live, compiling Rust/Swift;
neither was restarted on an observation timeout.

Remaining performance evidence gap: full-workspace navigation currently reports
`switch_and_sweep`, including the subsequent 120-step scroll exercise. That is
not an isolated time-to-usable measurement. Separate readiness timing must be
captured before the sweep, with an explicit rendered-content/input readiness
condition. The earlier history-page maximum scheduler lateness of 151 ms also
remains an outlier to investigate in an uncontended run, not a proved frame rate.

The recorded full-workspace harness now samples initial and per-switch readiness
separately, before fixed settle delays and scroll sweeps. Readiness requires
nonempty selected messages, rendered assistant text in the accessibility tree,
and a visible editable native composer; it inserts and restores a private draft
marker and checks that the selected model receives both edits. It never submits
that marker. Output is labeled `recorded_workspace_ready`, with a 16-ms sample
interval, as observed UI readiness rather than exact first-paint time or FPS.
Frontend syntax parsing and diff whitespace validation passed; a fresh recorded
fixture run is still required for these new measurements.

### Real continuation rerun and streaming-probe buffering defect

The request-order rerun (`/tmp/nola-mac-real-memory.fuvwSU/result.xcresult`)
**failed** the strengthened sequence assertion: items 1...31 were followed by
31...60. Thus instruction-order repair did not prove correct partial-item
continuation. Keep this as a product recovery failure, not a passing scroll test.

Both real runs also record a first-request `BrokenPipeError` at the proxy,
not an upstream terminal failure. Inspection found its `HTTPResponse.read(8192)`
could buffer available SSE bytes until the requested size was filled. Changed
it to `read1(8192)` so the observer forwards available data promptly. The new
`test_provider_probe_streaming.py` uses a real local upstream that cannot finish
until the downstream receives its tiny first event: **1 passed**, 1.01 seconds.
This tests transport timing deterministically without contacting a model.

A fresh real run with the corrected observer is running in
`/tmp/nola-real-stream-unbuffered.log`, reusing the already rebuilt Test binaries.
It is not evidence that partial-item recovery is repaired; a passing normal
stream alone cannot discharge the interrupted-stream recovery gate.

The continuation instruction itself also contradicted append-only publication:
it asked to resume from the "last completed semantic point", which can rewind an
unfinished item. Core now explicitly says its retained text is not replaced and
the next output must start after the exact final character, completing partial
words/items/rows without restarting their number or prefix. The CLI wire fixture
asserts this suffix contract as well as instruction order. Its new verification
passed (**1 passed, 0 failed**, 5.23 seconds) in `/tmp/nola-exact-suffix-pty.log`; this prompt change is not yet in
the already-running Mac Test binary and must receive separate real validation.

The unbuffered-probe real run completed successfully:
`/tmp/nola-mac-real-memory.QzmAVm/result.xcresult`, **1 passed, 0 failed**.
Rendered numbering is exactly 1...60. There was one upstream request, lasting
199,429.57 ms, with no proxy transport error. Its 2,667,540-byte response exceeds
the probe's 524,288-byte capture bound, so the empty captured `finishReasons`
does **not** establish a missing terminal event. This is normal-stream/switch/
tail acceptance, not forced-interruption proof, and predates the exact-suffix
prompt update.

A two-second sample of this Test application (`/tmp/nola-unbuffered-mac-sample.txt`)
found 133 of 178 main-thread samples inside the test's full accessibility-tree
collection. The test polled that expensive operation every 100 ms and forced
layout on each pass. It now retains lightweight 100-ms state observation but
paces full AX snapshots at one second, records `axSampled`, and leaves normal
layout scheduling to the app. Final complete-tree and screenshot assertions
remain. These observer-heavy timings must not be reported as normal app cost.
Fresh Test rebuild for the readiness and observation changes is running in
`/tmp/nola-readiness-build.log` before an uncontended recorded-fixture run.

### Readiness probe exposes lost native deletion

The new harness initially required an explicit `Any?` annotation to resolve
AppKit's overloaded accessibility selector; the corrected Test build passed in
`/tmp/nola-readiness-build-recheck.log`. Its uncontended fixture run then failed:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-5oqyow57/result.xcresult`.
After inserting then clearing the readiness marker, the native editor was empty
but model draft remained `READINESS-PROBE`.

The native delegate suppressed edits when `parent.text == textView.string`, but
the binding getter reads the previous SwiftUI display snapshot. An insertion
followed by deletion before that snapshot refreshes therefore drops the deletion.
Removed that snapshot comparison: native edit notifications always commit through
the existing conversation-scoped action sink. No permissions or submission guards
changed. The immediate-edit/Return window test now performs insert, delete, and
insert again before submission, asserting both native text and authoritative draft.
The original recorded readiness assertion remains unchanged.

Fresh build and both tests are running in `/tmp/nola-native-delete-build.log` and
`/tmp/nola-native-delete-recorded.log`. The apple-design component check was scoped
to normal text editing/data entry; no visual redesign or custom keyboard behavior
was added. Root-cause evidence is the failed native interaction test and binding
implementation, not an inferred visual symptom.

### Native deletion fix and uncontended recorded readiness results

Both tests passed (**2 passed, 0 failed, 0 skipped**) in
`/tmp/nola-native-delete-recorded.log`; result bundle:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-pfkci84w/result.xcresult`.
Exported standard output is under `/tmp/nola-readiness-diagnostics`.
The recorded fixture still reads all 14 messages, 138 parts, 259 bodies and
550,419 bytes. Runtime warnings are empty in the XCTest summary. This run had
no concurrent builds or process-memory sampler; awake and wall clocks agree
(48.54028 seconds), excluding sleep contamination.

Observed initial full-workspace readiness was 3.66999 seconds; subsequent
conversation switches were 209.26, 227.41 and 213.03 ms, including successful
native insert/clear verification. These are Test-host observed readiness samples,
not standalone cold-launch or exact first-paint measurements. App RSS after each
switch/sweep was 331,235,328 / 331,563,008 / 331,186,176 bytes (roughly 316 MiB),
stable across these three cycles, not a proof of universal memory reduction.
History-page scheduler lateness remained an outlier: p95 2.159 ms, max 125.954 ms.
Subsequent transcript sweeps had p95 around 2.13–2.15 ms and maxima 23.58–26.81 ms.
Initial readiness and history hydration still warrant profiling; this evidence
does not discharge the full performance goal.

### Profile attribution: remove a synchronous test-only archive reload

The profiled repeat passed (one test) in
`/tmp/nola-recorded-readiness-profile.log`; its 40-second, 5-ms sampling report
is `/tmp/nola-readiness-profile-sample.txt`. The principal main-thread service
wait in this capture is **test code**, not the workspace constructor: 244 samples
under `exerciseRecordedWorkspace`'s `XCTUnwrap(repository.load(...))`, including
197 + 41 socket-poll samples while loading two archive pages. The call generated
expected message IDs after opening the window, despite the same complete fixture
already being available as `stored.messages`.

The harness now passes those preloaded IDs into the workspace exercise, removing
the redundant synchronous read without weakening the assertion that every saved
message is reachable through real history paging. Syntax and whitespace checks
passed. Fresh build and uncontended verification are in
`/tmp/nola-readiness-isolated-build.log` and `/tmp/nola-readiness-isolated-run.log`.
Do not attribute this test-harness correction to a product performance gain.

The uncontended rerun passed (one test, no failures/skips/runtime warnings):
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-4l3h1dtn/result.xcresult`.
Exported output is `/tmp/nola-isolated-readiness-diagnostics`. All 259 bodies /
550,419 bytes remain intact. History sweep wake lateness max was 2.161 ms,
versus the previous 125.954-ms observation, but initial readiness was **4.847 s**,
not an improvement over 3.670 s. This may reflect changed scheduling of pending
UI hydration once the test no longer blocks; it is not established causation.
Warm switch readiness remained 219.20 / 222.50 / 225.00 ms, with RSS
333,398,016 / 333,758,464 / 333,447,168 bytes. Wall/awake duration was 48.2131 s.

Added separate model-construction, window-attachment, and selected-load timing
markers before readiness sampling to locate that remaining initial delay. These
markers still need a fresh build/run; they do not change product behavior.

### Duplicate selection-load ownership

Phase instrumentation passed in `/tmp/nola-workspace-phase-run.log` (one test),
with exported diagnostics under `/tmp/nola-workspace-phase-diagnostics`:
model construction 33.0 ms, window attachment 222.9 ms, awaited selected load
3.279 s, observed readiness 4.847 s. Code inspection established overlapping
callers: `MacWorkspaceModel.selectedConversationID.didSet` loads each new
selection, while `MacChatWorkspaceView.task(id: selectedConversationID)` also
loads it. The test additionally awaited a direct load before sampling readiness.

The view task now runs on initial mount only; the model remains responsible for
subsequent selection loads, projection observation, and bookmark restoration.
The recorded test no longer injects another initial/switch load or waits for
all ancillary loading work before observing actual rendered/input readiness.
No cached content or permission authority substitutes for fresh runtime checks.
Fresh Test build and lazy-loading, native-interaction, and recorded-fixture
regressions are running in `/tmp/nola-selection-owner-build.log` and
`/tmp/nola-selection-owner-tests.log`; performance improvement is not yet proved.

That regression finished with **23 passed, 0 failed, 0 skipped**, no runtime
warnings. Diagnostics are `/tmp/nola-selection-owner-diagnostics`. Initial
observed readiness was 2.695 s; warm switches 221.21 / 227.24 / 226.72 ms;
history-page scheduling max 4.433 ms. All recorded bodies remain readable.
Because product duplicate-load ownership and test observation changed together,
do not attribute the entire difference from the earlier 4.847 s to product speed.
RSS (346.47 / 346.10 / 345.87 MB) is stable across these cycles but comes from a
process that also ran the other 22 tests, not a fresh isolated-memory baseline.

Added an Application-journal fixture continuation boundary and a native Mac
regression that splits `PREFIX-ALPHA-OMEGA` across two model requests. It checks
both exported canonical text and a single rendered paragraph. This separates
semantic model repetition from Markdown-block fragmentation across persisted
parts. Build/run are `/tmp/nola-suffix-ui-build.log` and
`/tmp/nola-suffix-ui-run.log`; no passing verdict is claimed yet.

### Adjacent continuation text is one Markdown stream

The deterministic native-window test failed before the fix: canonical export
contained `PREFIX-ALPHA-OMEGA`, but separate part views rendered separate
paragraphs. Public adjacent text parts now share the first part's bounded
Markdown preview and semantic node identity. Nontext events remain boundaries;
this does not deduplicate repeated model output. Request equality includes all
continuation content references so suffix changes invalidate hydration.
Preview reads still have a total two-range budget, not two reads per part.
The full reader advances between segments and marks completion only after the
last segment, rather than stopping at the first segment's end.

Build `/tmp/nola-adjacent-text-build.log` passed. Transport and native-window
regressions `/tmp/nola-adjacent-text-tests.log` passed **42 tests, zero failures
or skips**, no runtime warnings; result bundle is under
`nola-mac-history-acceptance-tlsrlckn`. The recorded full-workspace test also
passed separately in `/tmp/nola-adjacent-text-recorded.log`, with diagnostics
`/tmp/nola-adjacent-recorded-diagnostics`: all 14 messages / 138 parts / 259
bodies / 550,419 bytes remain readable. Initial observed readiness was 2.733 s;
warm switches 219.69 / 233.24 / 240.05 ms. Full-workspace RSS was
333,365,248 / 333,168,640 / 332,906,496 bytes across cycles. Main-actor scheduler
wake lateness p95 was about 2.14–2.16 ms, max 25.63 ms; these are not frame times.
No captured pending work was resumed. These results do not establish that real
provider interruption recovery or every long-reader interaction is complete.

A further bounded-continuation regression covers the shared two-range preview
budget, availability of unread text, canonical export, and preservation of tool
boundaries. Its first fixture attempted two consecutive output-limit recoveries
and failed to claim the second continuation (`/tmp/nola-adjacent-boundary-tests.log`);
no UI assertion ran. The corrected fixture uses one recovery with a 100,000-byte
suffix, testing the read budget without assuming another recovery is admitted.
Build/run are `/tmp/nola-adjacent-bounded-build.log` and
`/tmp/nola-adjacent-bounded-tests.log`.

That oversized repeated-output fixture also did not produce a terminal seal, so
the bounded-read assertion now intentionally uses the live continuation cursor
instead of assuming successful completion. Final build
`/tmp/nola-adjacent-live-build.log` and regression
`/tmp/nola-adjacent-live-tests.log` **passed (one test, zero failures/skips)**.
The normal finished continuation remains covered by the native-window test in
the 42-test batch. Full-reader multi-segment button interaction and forced
real-provider interruption still require dedicated end-to-end verification.

### Full-reader native interaction acceptance

`testLongContinuationFullReaderLoadsEverySegmentThroughNativeButtons` now opens
the actual journal preview's SwiftUI sheet through accessibility press, reads a
39,010-byte continuation after a separate prefix, and presses the production
next-section button twice. It checks exact concatenated visible text, absence of
the next button only after the final bytes, explicit close returning control to
the transcript, and fresh first-page state on reopening. This uses an isolated
Application journal with a live cursor, not a real-provider request or captured
pending work.

The component review using apple-design (`buttons.md` Content; `sheets.md`
Anatomy) exposed a misleading next-page label and no explicit sheet dismissal.
The reader now uses the existing localized next-section text visibly instead
of an unlabeled chevron with the open-reader accessibility label. A fixed Close
button dismisses it, with the standard cancel keyboard shortcut. Native button
close/reopen is exercised; the Esc key itself was not synthesized by this test.

Build `/tmp/nola-reader-dismiss-build.log` passed. Combined journal transport,
joined-paragraph, and full-reader interaction verification passed **44 tests,
zero failures/skips/runtime warnings** in `/tmp/nola-reader-dismiss-tests.log`.
Result bundle:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-w1zsxl4w/result.xcresult`.
This closes the explicit full-reader multi-segment button interaction gap, not
the separate forced real-provider interruption or initial-readiness work.

### Cold-selection recovery timing attribution

Added opt-in DEBUG phase timing to selected-conversation loading and cold local
run adoption, enabled only by `NOLA_LONG_CONTEXT_FIXTURE_DB`. No conversation
IDs, prompts, credentials or invocation data are logged. No release behavior,
recovery order, or permission check changes.

Two recorded full-workspace runs passed, with complete recorded body checks:
`/tmp/nola-cold-load-phases-tests.log` and
`/tmp/nola-cold-recovery-phases-tests.log`. Their builds passed. Exported
diagnostics are `/tmp/nola-cold-load-phases-diagnostics` and
`/tmp/nola-cold-recovery-phases-diagnostics`.

First pass: recovery returned at 2.431 s, observed UI readiness 2.758 s.
Second pass cumulative recovery phases: active owner lookup 1.223 s; exact
recovery inspection 1.228 s; original invocation read 2.326 s; selection capture
2.327 s; adoption returned 2.336 s. Observed readiness was 2.744 s and warm
switches 230.62 / 235.58 / 229.86 ms. The transcript observer also loads the
window concurrently, so this timing does not prove that removing the adoption
await alone would improve readiness. The slow intervals are owner lookup and
invocation retrieval, not the 5-ms inspection or bookmark capture. Further
profiling must separate service queueing/reconstruction from actual lookup
work; do not skip exact run identity/integrity validation to obtain a fast pass.

### Service queue attribution diagnostics

Added opt-in `NOLA_SERVICE_QUERY_TIMING=1` output separating the Application
mutex wait from read-only command execution. Only command kinds and microsecond
durations are emitted, no arguments/identifiers/results. Mac DEBUG explicitly
forwards this flag to its service and the fixture prints filtered lines from
the existing bounded stderr buffer after initial readiness. Normal launches do
not enable this diagnostic.

The first run `/tmp/nola-service-query-phases-tests.log` passed the history
checks but emitted no target query timings: instrumentation was mistakenly
placed in the adjacent reconciliation branch. It is **not queue attribution
evidence**. The branch placement was corrected and an enabled-but-empty
diagnostic now fails the test. Fresh build is
`/tmp/nola-query-timing-read-build.log`; the corrected run is pending below.

Source inspection also confirms `load_exact_run_invocation` materializes the
recovery frontier to validate exact checkpoint/owner/config integrity. It is
not merely a configuration-row read. A future optimization must retain that
proof, and distinguish replay execution from waiting for another command.

The corrected diagnostic run passed (one recorded full-workspace test, zero
failures/skips/runtime warnings) in `/tmp/nola-query-timing-read-tests.log`.
Diagnostics: `/tmp/nola-query-timing-read-diagnostics`; result bundle under
`nola-mac-history-acceptance-077_2jda`. Initial readiness was 2.760 s; warm
switches 232.23 / 236.84 / 233.54 ms. The captured startup commands establish:

| Command | Mutex wait | Execution |
| --- | ---: | ---: |
| Selected conversation window | 0.001 ms | 1,230.450 ms |
| Active run identity | 1,230.445 ms | 0.106 ms |
| Exact recovery inspection | 0 ms | 0.557 ms |
| Exact run invocation | 0 ms | 1,048.099 ms |

Thus the apparent 1.23-second identity lookup is head-of-line blocking behind
the window projection, not slow identity SQL. Invocation retrieval separately
spends about one second executing recovery validation. Earlier window reads in
this same test include a 1,257-ms cold read and 253–296-ms subsequent reads.
The next implementation target is shared projection/recovery work and query
scheduling; changing the active-run SQL or skipping ownership checks would not
address the measured cause. Instrumentation itself is not a performance fix.

### Reuse validated read materialization for exact invocation inspection

The internal `StoredExactRunCoreCheckpoint` proof can now carry the existing
bounded read memo, fetched while the same SQLite read snapshot pins the Run,
configuration, replay inputs and admission witnesses. Application uses it only
when conversation, Store revision, event cursor and update time match the
baseline header. Otherwise it materializes recovery as before. Config digest,
replay Run/config binding, decoded Core identity, terminal status, admission,
and exact frozen Host binding checks remain unchanged. The memo is still not
a durable checkpoint, effect-execution shortcut, or cross-snapshot authority.

New Store coverage verifies that an external commit cannot change a pinned
reader's proof and that both external and local writes invalidate reuse on the
next read. It passed in `/tmp/nola-exact-memo-store-tests.log` (one test).
The nonterminal replay integration test also warms the actual conversation
window query and compares the entire exact-invocation result before and after
warmup, for hot/cold runtime reads over multiple tool cycles. Invocation
integrity and nonterminal regressions are being run in
`/tmp/nola-exact-memo-application-tests.log`; no latency improvement is claimed
until the recorded Mac rerun measures this new path.

Verification completed: Store regression **1 passed**, Application invocation
and nonterminal replay suites **13 passed**, and Mac recorded full-workspace
regression **1 passed**, all with zero failures/skips. Mac build and run:
`/tmp/nola-exact-memo-mac-build.log`, `/tmp/nola-exact-memo-mac-tests.log`;
exported diagnostics `/tmp/nola-exact-memo-mac-diagnostics`, result bundle under
`nola-mac-history-acceptance-ods4ft_a`.

Compared with the preceding instrumented run, exact invocation execution fell
from 1,048.099 ms to **135.418 ms**; initial observed readiness fell from 2.760 s
to **1.870 s**. This is one comparable measured rerun, not a latency guarantee.
Active owner lookup still waited 1,224.877 ms for the projection lock (its own
execution was 0.124 ms), so that separate projection cost remains. Warm switches
were 233.47 / 231.28 / 233.40 ms; full-workspace RSS was
332,431,360 / 332,726,272 / 332,365,824 bytes across cycles. All 14 messages /
138 parts / 259 bodies / 550,419 bytes remain intact. No captured effects ran.

### CLI regression after shared recovery memo change

Rebuilt the current CLI test executable and replayed the same private long
conversation in an isolated database. `/tmp/nola-shared-memo-cli-recorded.log`
passed: 14 retained messages, exact repeated message equality, 392 rendered
lines / 16,314 bytes. Initial load was 2.487 s; warm load 74.03 ms; formatting
6.67 ms. Twenty repeated reads/renderings took 1.670 s. Test-process RSS high
water rose from 65,372,160 to 69,074,944 bytes; this is neither whole-process-tree
RSS nor proof of continuously flat live memory.

The real PTY replay passed at 100x30, 40x15 and 120x40, preserving one footer
below the unsent draft without tree prefixes; captured work was not resumed.
`/tmp/nola-shared-memo-cli-footer.log` passed nine footer/state/width tests and
`/tmp/nola-shared-memo-cli-digits.log` passed direct numeric confirmation.
The full interactive footer PTY subset is running separately in
`/tmp/nola-shared-memo-cli-footer-pty.log`.

The interactive subset finished with **4 passed, zero failures/skips** (6.90 s):
Chinese completed-response footer, return to next prompt without transcript
pollution, mobile-height resize, and populated footer during the second turn.
Together this turn verified 15 CLI tests, including the recorded PTY replay.
These are current footer/replay regressions, not a substitute for the remaining
Mac initial projection and real-provider interruption acceptance.

### UI and harness priority: native submission and recovered status

The current priority is UI interaction and agent-harness correctness, not
further optimization of the measured cold projection. The real streaming
workspace test now presses the actual composer accessibility button instead
of calling `model.send()`. It checks the new conversation has no Stop button,
that admission blocking is explained, the running conversation does not inherit
the other draft, terminal completion restores Send, and the unsent draft is
preserved when selected again. Stable Send/Stop accessibility identifiers are
attached to the production action, not a substitute test control.

The first run (`/tmp/nola-mac-real-memory.As561L`) failed an incorrect test
assumption that the new conversation could send during another active Run.
Inspection of its real screenshot confirmed the current single-active-work
constraint is explicitly explained. The revised assertion checks that actual
button availability matches admission state and requires its explanation.
Rerun `/tmp/nola-native-send-switch-verified.log` passed one real-provider test,
zero failures/skips; artifacts are `/tmp/nola-mac-real-memory.MuUhZo`.
The existing exact 1...60 visible list and terminal scroll geometry assertions
remain in place. This does not prove forced mid-answer interruption recovery.

Screenshot inspection also exposed an independent UI defect: a completed answer
was tinted red because an earlier request attempt had failed and recovered.
`MacMessageView` now derives the outer failure treatment from the message's
delivery state, retaining attempt failure details without marking a successful
answer as failed. The regression extends failure-presentation coverage across
completed, running, cancelled, unknown and failed states; its verification is
tracked separately from the preceding real-provider run.

The provider evidence proxy no longer analyzes only the first 512 KiB of a
response. It incrementally inspects all lines, retaining at most one bounded
line, explicitly counting oversized lines, and recording `[DONE]` markers.
Three Python tests passed: immediate small-event forwarding, terminal evidence
after 20,000 split events, and bounded recovery after an oversized line. The
successful real-provider rerun recorded one Done marker and no oversized lines.
These statistics are observer evidence, not production stream acceptance rules.

Recovered-response styling and composer boundary verification completed:
`/tmp/nola-recovered-response-ui-build.log` succeeded;
`/tmp/nola-recovered-response-ui-tests.log` passed all 13 selected tests with
zero failures/skips. This includes the failure-state policy, native editing,
compact permission layout and composer recovery states. A forced successful
retry screenshot of the new color treatment remains to be captured; the
earlier real-provider pass predates that styling change.

### Forced partial-answer recovery acceptance

`provider_probe.py` now offers an explicitly opt-in, once-only interruption
after a configured amount of public text. It forwards already received bytes,
closes valid HTTP chunk framing without synthesizing a provider terminal, and
leaves subsequent requests untouched. It never injects after a finish reason
or Done marker. The real Mac runner accepts
`NOLA_MAC_REAL_INTERRUPT_AFTER_TEXT_BYTES` and rejects a nominally green run
unless exactly one sufficiently long partial answer was actually interrupted.
All five proxy regressions passed, including a local HTTP server test proving
that interruption does not wait for upstream EOF or manufacture a transport
hang. Normal invocation leaves fault injection disabled.

The isolated real-provider run in `/tmp/nola-forced-interruption-ui.log`
(`/tmp/nola-mac-real-memory.hSHndC`) is the first forced recovery acceptance.
Its result is not yet claimed. An additional native assertion now requires
the final assistant delivery state to be completed, not merely visible text
and an idle composer; that assertion must be rebuilt before its next run.

The first forced run finished **failed**, specifically at the ordinary 240 s
total deadline (`xcresult` reports that assertion). Injection actually occurred
after 818 text bytes / 209.446 s, and a continuation request followed with nine
messages ending in assistant/system. It produced 4,984 text bytes before test
cleanup closed the connection; this is not a successful recovery receipt.
No duplicate-number/end-marker assertion failed, but the missing terminal
completion still invalidates acceptance.

Ordinary acceptance retains its 240 s limit. Explicit forced-interruption runs
now use a separately identified, bounded 480 s functional budget, because they
require two real generations; passing this gate cannot establish latency
acceptance. `/tmp/nola-forced-interruption-ui-functional.log` rebuilds the test
including the completed-delivery assertion and runs this separate gate.
Provider evidence also counts the `reasoning` delta alias without retaining its
text, so long reasoning streams are not mislabeled as zero-reasoning traffic.

The rebuilt functional gate **passed**, one real-provider XCTest with no
failures/skips/runtime warnings, in `/tmp/nola-mac-real-memory.bRDtOe`.
It injected exactly once after 820 public-text bytes. The continuation finished
in 19.180 s with `finish_reason=stop` and one Done marker. The native test
verified exact visible numbering 1...60, the end marker, completed delivery,
terminal bottom geometry, and independent draft/control state across switching.
Read-only inspection of the isolated database confirmed one completed Run.
The six proxy regressions also passed, including reasoning-alias accounting.

The completed screenshot was inspected: the response now has a neutral surface
despite the retained failed-attempt diagnostics, closing the outstanding color
verification. It also shows a preview/Read Original footer despite this short
answer reaching its end marker; that completeness presentation needs separate
inspection. The forced test proves this generated scenario, not all recovery
shapes or long-context performance targets. No captured user effects ran.

### Empty terminal field falsely advertises unread preview content

Read-only inspection of the successful forced-recovery journal confirmed two
text fields of 820 and 5,912 bytes followed by a legitimate zero-byte field.
The two-read preview displayed both nonempty fields, then incorrectly treated
the remaining empty field as unread content. Preview hydration now skips exact
zero-byte fields before applying its unchanged two-range-read budget; an
all-empty group completes without a range read. Invalid/unknown length strings
are not treated as empty. This does not remove canonical parts or join across
tool/activity boundaries.

The first regression run passed 44 tests and failed the new fixture assumption:
the ordinary deterministic continuation does not create the empty field seen
on provider retry. The corrected test explicitly supplies empty metadata
between/after two real Application-backed text fields, and separately tests an
all-empty request. Native UI coverage also checks a fully displayed short
continuation does not offer the truncated-content reader. Verification logs:
`/tmp/nola-empty-preview-build-verified.log` and
`/tmp/nola-empty-preview-tests-verified.log`.

Verification completed: **45 passed, zero failures/skips** across the journal
transport suite and native short-continuation/full-reader button tests. The
100,000-byte continuation still retains the same two-read preview limit and
offers its full reader. This closes the short-preview false truncation; the
full reader's own paging should apply the same empty-segment rule when an
otherwise long grouped answer contains empty fields.

### Full reader shares empty-field semantics with preview

`MacJournalBodyState.Segment.hasReadableContent` now owns the exact zero-byte
test used by both preview hydration and full-reader pagination. Empty leading,
intermediate or trailing fields do not consume a page action. An all-empty
reader reaches completion without a body query. Nonempty pages retain their
original cursor-bound range reads and offsets.

New native coverage mounts the production `MacJournalTextReader` in an actual
NSWindow, clicks its accessibility buttons, and checks two clicks produce the
exact joined 39-KiB continuation despite empty fields at all three positions.
It separately requires zero clicks and no reader error for an all-empty group.
The reader is module-internal (not public) so tests can mount the real view,
not a replacement implementation. Existing sheet open/close/reopen tests and
bounded preview tests run alongside it. Build succeeded in
`/tmp/nola-reader-empty-pages-build.log`; verification uses
`/tmp/nola-reader-empty-pages-tests.log`.

Verification completed: **46 passed, zero failures/skips**, including the new
native empty-field reader cases, exact joined text, two-click pagination,
all-empty completion, existing sheet dismissal/reopen, and bounded transport
coverage. Result bundle: `nola-mac-history-acceptance-5odne8en/result.xcresult`.
The preview/full-reader empty-field handling gap is now closed.

### Consolidated UI/harness audit

The current full `MacTranscriptCompletionScrollTests` suite passed **7 tests,
zero failures/skips**, with the original private fixture and full-workspace /
natural-layout flags enabled. Evidence: `/tmp/nola-ui-harness-consolidated-mac.log`,
exported diagnostics `/tmp/nola-ui-harness-consolidated-mac-diagnostics`, bundle
`nola-mac-history-acceptance-x_z81d6q/result.xcresult`.
All 14 messages / 138 parts / 259 bodies / 550,419 bytes remained readable.
Initial observed native readiness was 1.825 s; switches were 227.53, 229.27 and
236.28 ms. First window read was 1.253 s with no sleep overlap. Three workspace
cycles recorded App RSS 508,379,136 / 508,641,280 / 490,782,720 bytes. This suite
shares a test process across seven cases, unlike the earlier one-case ~332-MB
measurement; no comparable memory improvement or regression is inferred.
History-page scheduling also retained a **132.02-ms maximum wake-lateness**
sample (p95 2.14 ms). The synchronous scroll-update interval was below 0.69 ms
on that page, but natural-layout mode does not force layout inside that
interval. It therefore does **not** measure actual layout cost. This is not an
FPS measurement and must not be erased by the green behavior verdict: a
history-loading interaction tail remains to be attributed.

The audit found the full CLI PTY pass predates subsequent request-order and
continuation changes, even though focused post-change tests passed. Therefore
the current entire `pty_interaction` target is running with one worker and
process-lifetime sleep inhibition in `/tmp/nola-ui-harness-consolidated-cli.log`.
It started after the Mac measurements ended; no final CLI verdict is claimed
until this exact process exits and the test totals are checked.

Follow-up inspection corrected the scroll observer's misleading `layout_*`
metric names: they measured the complete synchronous update interval and, in
natural mode, omitted actual AppKit/SwiftUI layout during suspension. Output
now uses `update_*` with `includes_forced_layout`, and scheduling output retains
the worst wake sample's index before sorting. No product timing or behavior
was changed. Swift syntax parsing and whitespace checks passed; the diagnostic
change still needs the next built recorded run after the current CLI suite.

### Current-source full CLI verdict

The exact consolidated CLI process exited successfully: **86 passed, 0 failed,
0 ignored, 0 filtered out**, 722.53 s, in
`/tmp/nola-ui-harness-consolidated-cli.log`. This is after the request-order,
continuation and shared recovery memo changes, closing the stale full-suite
evidence gap. It covers persistent footer and mobile-height resize, live input,
streamed tables, in-place output, partial-stream recovery, repeated compaction
with goal/permission ownership, real worker/subagent cancellation and restart,
fairness/resource budgets, and ten-million-character history restoration under
a controlled memory limit. No failed case was restarted or skipped.

The only pending current-source diagnostic edit is the Mac scroll metric naming
and worst-wake index. Rebuild and one uncontended recorded run with body timing
can now proceed without CLI CPU/memory contention. Its purpose is to interpret
the observed history-page latency tail and compare a fresh single-case memory
sample, not to reopen the now-completed CLI sweep.

### Uncontended Mac history-tail measurement

The rebuilt single-case test passed in
`/tmp/nola-history-ui-isolated-timing.log`; diagnostics are
`/tmp/nola-history-ui-isolated-diagnostics`. It verified the same complete
14-message / 138-part / 550,419-byte fixture. Initial observed readiness was
1.852 s; switches were 233.99 / 235.42 / 239.10 ms. Workspace App RSS was
332,136,448 / 332,398,592 / 332,087,296 bytes, consistent with the preceding
single-case baseline rather than the shared-suite ~508-MB sample.

The history-page wake-lateness tail reproduced at **143.09 ms, index 53**,
with p95 2.15 ms. Latest-window sweeps had maxima 18.86–20.83 ms at index 0.
Body preparation instrumentation showed no corresponding slow parse; empty-body
publication samples of 43–52 ms occurred separately. These timings do not prove
what blocked rendering at index 53. A read-only sample of the exact Test app
process is running alongside the isolated replay in
`/tmp/nola-history-ui-sampled-run.log`, output
`/tmp/nola-history-ui-app-sample.txt`. It targets no developer app or captured
effects, and its overhead disqualifies it as a clean latency comparison.

That sampled replay completed successfully (one test, zero failures/skips).
The first 40-second sample was dominated by earlier whole-transcript screenshot
and accessibility traversal, not the target history interaction. A second
sample attempt correctly reported the already-exited test PID; it was not
restarted. With sampling plus the existing two-second pre-history rendezvous,
the history wake maximum was only 2.17 ms; this timing perturbation is **not**
evidence that the reproduced 143-ms tail is fixed. Future attribution must
synchronize capture to the specific history phase rather than process launch.
No speculative product optimization was made from the unaligned sample.

### Phase-aligned history profiling

The opt-in `NOLA_HISTORY_PROFILE_DIRECTORY` test marker now emits the Test
process PID immediately before history paging, without the previous two-second
settling delay. An external observer verified the executable was under
`Products/Test` before sampling five seconds. Build and the recorded test passed
(one pass, no failures/skips). Evidence: result bundle
`nola-mac-history-acceptance-hkqb4vv_`, sample `nola-history-phase-c0d_1w65/history.txt`,
and exported diagnostics `/tmp/nola-history-phase-diagnostics`.

Unlike the earlier launch-aligned sample, this capture found 826 main-thread
samples in `MacWorkspaceModel.persistArchiveNow` through `saveMetadata`, with
825 in synchronous service socket waiting. This is a concrete UI-thread I/O
path to remove, while preserving archive ordering and revision semantics.
The sampled history wake maximum was 1167 ms (p95 11.02 ms); instrumentation
perturbs timing, so this is attribution evidence, not a clean performance
baseline or proof that all of the earlier 143-ms tail has the same cause.

### Background metadata persistence

Workspace metadata saves now capture their values on MainActor and enqueue
service reads/writes on a serial utility queue. Synchronous metadata imports
and deletion share that ordering. Completion can clear pending knowledge only
when the conversation still exists and the scheduler revision is current.
Core's already-durable terminal Run is not held behind this Mac-only metadata
commit. New-conversation creation and explicit synchronous repository callers
are not claimed to be asynchronous by this change.

Build passed (`/tmp/nola-async-metadata-build-2.log`); repository, local terminal
contract and recorded-history acceptance passed **30 tests, zero failures or
skips** (`/tmp/nola-async-metadata-tests.log`). A suspended storage queue test
proves submissions return without blocking MainActor, the newest queued snapshot
wins, and deletion follows pending writes. No failed test was skipped.

Unprofiled recorded replay: initial readiness 1.843 s, warm switches
227–236 ms, history wake p95 2.057 ms / maximum 66.483 ms, three workspace RSS
samples 335.09–335.36 MB. Diagnostics: `/tmp/nola-async-metadata-diagnostics`.
The maximum improved relative to the preceding 143-ms clean observation but is
not zero or a universal frame-time guarantee; further work must retain this
remaining tail rather than declare every scrolling problem fixed.

### Archive completion ownership and submission boundary

Follow-up inspection found an ABA acknowledgement risk: forgetting a
conversation removed its revision counter, so restoring the same ID could
reuse an outstanding save's revision. The archive scheduler now allocates
monotonic revisions across its lifetime while still releasing per-conversation
tracking entries. Tests cover forgotten/restored IDs, independent conversations,
and a newer edit invalidating an in-flight acknowledgement before debounce.

Submission inspection confirms configuration and conversation values are captured
before asynchronous context preparation, rather than waiting for Mac metadata
durability. The existing native submission tests exercise changes during that
wait, cancellation and conversation-local composer state. Build and the combined
submission/repository run passed (`/tmp/nola-archive-generation-build.log`,
`/tmp/nola-archive-generation-tests.log`); these are deterministic harness tests,
not another real-provider performance measurement.

### Post-save phase attribution

The current built recorded test passed again with a five-second phase-aligned
sample (one pass, zero failures/skips). Sample:
`nola-history-postsave-lv_js0oq/history.txt`; result:
`nola-mac-history-acceptance-wh6yik3a`; diagnostics:
`/tmp/nola-history-postsave-diagnostics`.

Storage waiting now appears under the background metadata operation, not the
main-thread archive save. The main-thread sample instead includes 71 samples
under `MacConversationWindowModel.window.setter` → Observation invalidation →
SwiftUI graph transaction flush/layout, with code-block rendering also present.
This narrows the next investigation to presentation observation/layout work;
it does not justify dropping canonical updates or adding a blind render delay.
Sampled history wake maximum was 356 ms (p95 3.14 ms), so the earlier clean
66-ms result is not replaced with a claim of universally smooth scrolling.

### Page-navigation observation boundary

`hasEarlierMessages` and the oldest-message ordinal now have independent
observable values. The transcript page identity no longer subscribes to the
whole canonical window just to read its ordinal. Metadata/title/cursor updates
still publish canonically, but unchanged page navigation does not invalidate.
The real-store paging test uses Observation tracking to assert no navigation
notification on rename and a notification on actual paging.

Build passed and lazy-loading plus the entire scroll suite passed **20 tests,
zero failures/skips** (`/tmp/nola-page-observation-tests.log`). Diagnostics:
`/tmp/nola-page-observation-diagnostics`. The shared-suite recorded run measured
1.847-s initial readiness and history wake p95 2.124 ms, maximum 141.393 ms.
Thus this is a verified dependency correction, **not** a verified removal of
the remaining latency tail; the previous 66-ms observation was not a stable
upper bound. Shared-suite RSS was stable across cycles at 498.4–498.8 MB and
must not be compared directly with isolated-case RSS.

### Inline code rendering reuse

Sampling identified syntax highlighting inside `MacCodeBlockView.body` during
layout. A view-local single-entry cache now reuses the rendered attributed value
for identical source/language/appearance. Changes replace that value; no global
cache of streamed prefixes is introduced, and copy still uses original source.
Tests verify twenty identical renders perform one highlight, light/dark changes
resolve different colors, language/source changes invalidate, and an evicted
prefix is recomputed rather than retained.

Build passed; code-reference tests plus the entire scroll suite passed **15
tests, zero failures/skips** (`/tmp/nola-inline-highlight-tests.log`). Recorded
replay initial readiness was 1.799 s; history wake p95 2.136 ms, maximum 66.933 ms.
Diagnostics: `/tmp/nola-inline-highlight-diagnostics`. This removes demonstrated
repeated rendering work; a single timing observation does not establish a new
worst-case latency bound. Shared-suite RSS samples were 508.87 / 508.90 / 491.98 MB.

### Three independent current-source replays

Three sequential isolated Test processes all passed the full recorded-history
case, with no profiling, restart or skipped case. Bundles:
`nola-mac-history-acceptance-n6pkniw3`, `...-5bnmr26_`, `...-ajnrv35b`;
diagnostics `/tmp/nola-repeated-history-{1,2,3}`.

| Replay | Initial readiness | History wake p95 | Maximum / index | Workspace RSS across switches |
| --- | --- | --- | --- | --- |
| 1 | 1.836 s | 2.137 ms | 126.76 ms / 58 | 332.46–332.81 MB |
| 2 | 1.834 s | 2.141 ms | 133.98 ms / 59 | 331.97–332.74 MB |
| 3 | 1.841 s | 2.138 ms | 131.18 ms / 59 | 332.32–333.04 MB |

Warm readiness across all nine switches was 228.5–235.6 ms. Memory does not
monotonically grow across those cycles. Crucially, this repeatability contradicts
using the earlier ~67-ms single observation as the current tail: a ~130-ms
main-actor scheduling interruption still repeats near the middle of the scroll
sweep. The next attribution must examine that interval on the latest build,
not keep presenting a green functional test as proof of smooth scrolling.

### Background Markdown preparation — incomplete acceptance

The latest phase sample (`nola-history-current-9oyctpxm/history.txt`, result
`nola-mac-history-acceptance-r4dph_99`) still showed semantic Markdown assembly
on MainActor as well as native layout. Preparation now transfers a value-copy
projection through the cancellation-aware background read helper, then checks
generation/cancellation before atomically publishing the snapshot. The parser
and projection explicitly conform to Sendable; the build passed.

Acceptance is **not green**: `/tmp/nola-background-markdown-tests.log` reports
51 passed / 1 failed / 0 skipped. The 100,000-character plain-text hydration
case still had zero published bytes after its 200 five-ms polling iterations
(expected bounded preview 65,536 bytes). Previously synchronous parsing also
blocked the polling actor, so this reveals an actual latency concern rather
than justification for silently increasing the wait. Inspection found repeated
whole-tail/line scans while the semantic assembler consumes small phrases.
Investigate that parser cost before accepting the background-preparation change;
retain the failed result and all ownership/cancellation coverage.

### Latest CLI permission incident (2026-10-10)

Read-only inspection of the live CLI store identified conversation
`20261010160018094253000-15839-0`, latest Run
`5564fe0e-018a-465b-9d19-3c4df37d311b`. No recorded command was replayed.
The journal records conversation approval, but shell Git writes subsequently
fail creating `.git/index.lock` and test files. Source inspection confirms
`git_process_preservation` intentionally makes existing staged Git metadata
read-only for opaque processes, independently of capability approval.

Earlier `git_commit` calls supplied `paths:["Nola/"]`; change-membership checking
ran before directory validation and returned misleading generic guidance.
Validation now reports the concrete directory/file contract before membership.
Process preservation now also annotates structured tool data (not only text),
including recovery capabilities and the fact permission approval cannot lift
the restriction. It does not claim the guard caused every command failure.
Both changes have regression tests; the tools test process is currently live,
log `/tmp/nola-cli-permission-contract-tests.log`, and no pass is claimed yet.
Repeated `tool_not_exposed_in_model_step` results in the same incident remain
an additional capability-lifecycle investigation. Do not disable preservation
or claim the entire permission incident fixed from the diagnostic changes.

Follow-up: Core rejected `host_resource_preservation` as an activation source
and also discarded activation on failed results. It now accepts that typed
Host source on both successful and failed results; failed discovery/Skill
results and arbitrary sources remain rejected. This only exposes recovery
schemas, never grants execution permission. A Core integration test verifies
the activation event and recovery schema in the next actual model request.

Validation: the initial CLI tools suite finished 415 passed / 2 failed. The new
directory fixture collided with its existing `nola` file on case-insensitive
macOS; renaming the fixture directory to `Sources` resolved that test. The
other failure, file-inspection discovery for an MP3 query, remains unresolved
and must not be reported green. Subsequent focused runs passed 8 Git commit
tests, 3 process-preservation tests, 3 Core activation unit tests, and 1 Core
Host integration test. Logs: `/tmp/nola-preservation-git-tests.log`,
`/tmp/nola-preservation-shell-tests.log`, `/tmp/nola-preservation-core-tests.log`,
and `/tmp/nola-preservation-host-contract.log`. These are fixture/contract
tests, not a new real-provider CLI acceptance run; no user Git effects were
replayed. Other tool-exposure failures in the recorded session still require
separate verification.

### Markdown preparation regression repaired (2026-10-10)

The semantic assembler now rejects impossible fence/structured-block prefixes
before searching the remaining line, and records whether incoming pending
source contains a pipe before attempting table parsing. The table hint is
conservative across consumption and resets when pending source is exhausted;
it does not change the table grammar or rewrite content. This eliminates
repeated full-tail scans for the plain long-line hydration fixture while
preserving phrase identities and the background atomic publication change.
Reference review: Codex's `markdown_stream.rs` separates stable source boundary
collection from rendering; OpenCode's `markdown-stream.ts` separates live open
fences from stable content. Neither is grounds to replace our existing native
streaming contract without equivalence tests.

Test-config build `/tmp/nola-parser-prefix-build.log` succeeded. Combined
Markdown parser, journal transport and transcript scroll acceptance passed
87 / 87, zero skipped, `/tmp/nola-parser-prefix-tests.log`; result bundle
`nola-mac-history-acceptance-9k7swv5n`. The previously failing bounded 100K
body/owner-switch regression passed in 0.569 seconds with its original waits.
Diagnostics exported to `/tmp/nola-parser-prefix-diagnostics`.

The recorded full-workspace history-page sweep still reports maximum wake
lateness 135.279 ms (index 61; p95 2.134 ms), so this is NOT evidence that the
remaining historical scroll stall is fixed. The transcript sweep max was
10.878 ms. This shared test-process run is not an isolated memory comparison.
No production Debug binary or live user conversation data was changed.

### Inspector render-path service read removed (2026-10-10)

A fresh phase-triggered sample of the current Test binary is retained at
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-parser-profile-htqglqob/history.txt`.
The test passed (1/1; `/tmp/nola-parser-profile-run.log`). Sampling perturbed
history wake lateness to 454.404 ms and is not a clean performance comparison.
The stack still contains native layout/SwiftUI transaction work and initial
code highlighting; it also proves that inspector body evaluation called
`retrievalEnabled` through preferences into synchronous Application service IO.

The inspector now consumes an asynchronously fetched retrieval-default snapshot
from its own canonical repository, with a pure conversation-override projection.
Configuration changes refresh the snapshot; the inherited toggle is disabled
until its first default arrives. Retrieval preference changes now emit the same
configuration notification as memory preference changes. Execution authorization
and persistence contracts were not relaxed. No visual restyling was performed.

Test build `/tmp/nola-retrieval-display-build.log` succeeded; inspector and full
scroll suites passed 13/13, zero skipped (`/tmp/nola-retrieval-display-tests.log`,
bundle `nola-mac-history-acceptance-f4gwjbqo`). New coverage verifies the isolated
repository default, refreshed default and conversation-override precedence.
Clean recorded history still shows 138.791 ms maximum wake lateness at index 57
(p95 2.138 ms). This corrects an IO-in-body defect but does NOT resolve the
remaining scroll stall; next work must inspect the layout/update transactions,
not claim improvement from the sampled-vs-unsampled timing difference.

### Canonical page publication is idempotent (2026-10-10)

Navigation and projection observation now use `publishCanonicalMessages`, which
does not mutate the observed dictionary when the canonical message array is
identical. Removing a nonexistent presentation overlay is also a no-op. This
retains full message equality, including journal references, rather than
suppressing real updates based only on message IDs. A new observation test
verifies zero invalidations for the same page and one for changed text with
the same identity.

Test build `/tmp/nola-idempotent-page-build.log` and 21 pagination/scroll tests
passed (zero skipped), `/tmp/nola-idempotent-page-tests.log`, bundle
`nola-mac-history-acceptance-jwcjv9yw`. History max wake lateness remained
134.943 ms at index 58, p95 2.136 ms. Thus duplicate publication was a genuine
unnecessary update path, but is NOT established as the primary stall cause.
Next investigation should isolate delayed body hydration/publication and its
layout transaction, rather than interpreting this small timing difference as
a solved performance issue.

### Hydration/layout timeline rejects premature publication throttling (2026-10-10)

Opt-in diagnostics now record monotonic uptime on all journal body publications
(byte counts and durations only, never text). The isolated XCTest layout probe
reports native layout calls over 40 ms; scroll wake outliers have matching
uptime. Production probes remain disabled by default. Build and recorded
acceptance passed, `/tmp/nola-hydration-timeline-build.log` and
`/tmp/nola-hydration-timeline-tests.log`, bundle
`nola-mac-history-acceptance-s7wqdyd6`; diagnostics exported to
`/tmp/nola-hydration-timeline-diagnostics`.

At uptime 58357.925578 the history sweep recorded 69.517 ms lateness (index59).
The next body publications occurred AFTER it, at 58357.926949 onward, with
publication itself around 25–44 microseconds. This does not support the earlier
hypothesis that those body publication assignments caused the stall. Native
layout over40ms was recorded earlier (58356.609945, 41.419ms), not overlapping
that outlier. Because the probe changes timing, 69ms vs135ms is NOT accepted as
an optimization. Do not add publication delays on this evidence; investigate
pre-publication view construction/SwiftUI transaction work next.

### CLI discovery regression and full tool suite closed (2026-10-10)

The unresolved MP3 discovery failure came from unweighted term coverage:
common words and substrings in names could outrank a rare format explicitly
declared by the appropriate tool. Lexical tool discovery now weights term
scores by their frequency in the filtered available catalog. It introduces no
language stop-list, audio-specific route, user-prompt rewriting or permission
grant. Explicit select/exact/prefix modes and Core-supplied semantic rankings
retain their existing paths. A synthetic ZX42 format fixture ensures the fix
is not specific to the originally failing format.

All 28 tool-search tests passed in 0.34s, including multilingual follow-ups,
filters, injected dynamic schemas and the previously failing audio metadata
query (`/tmp/nola-discovery-selectivity-tests.log`). Then the full tools group
passed 418/418, zero failures/ignored, 26.45s
(`/tmp/nola-tools-selectivity-full-tests.log`), including the new preservation
diagnostics/directory selection tests and execution transaction tests. This
supersedes the earlier 415/417 failed run, which remains recorded above.
It is not a fresh real-provider CLI or PTY acceptance run and does not close
the remaining Mac scroll latency or all recorded capability-exposure failures.

### Fresh real-provider CLI composition acceptance (2026-10-10)

Rebuilt the actual CLI (`/tmp/nola-cli-real-acceptance-build.log`, 21.33s),
then ran the existing `--split-commits` generated-repository fixture against
configured `openrouter/free`. Evidence:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-commit-followup-fj2pc1tg`;
driver log `/tmp/nola-real-preserved-index-acceptance.log`. The model recovered
from an invalid shell `root` argument and created three disjoint commits.
No operation targeted the actual user repository or live conversation database.

The driver initially exited on an obsolete assertion requiring audit outcome
`action_completed` rather than ordinary terminal `completed`. The harness now
accepts ordinary completion or the appropriate optional audit outcome, requires
exactly one completion and rejects accompanying failure/cancellation. It still
requires actual Git/file/database postconditions; no completion label alone
passes execution. Two offline harness contract tests pass.

Without replaying any writes, independent read-only verification of the captured
run passed: exact three requested paths committed in three parent-linked,
disjoint commits; HEAD bytes and worktree bytes equal the fixture; unrelated
index entry retains its expected blob/mode and its later unstaged bytes;
excluded environment/binary files unchanged and untracked; persisted run is
completed. The original driver failure remains in evidence (not retroactively
called a green driver run). This is a verified real execution/composition
scenario, not interactive permission-panel or comprehensive PTY acceptance.

### Permission acceptance evidence correction (2026-10-10)

The first real-provider numeric permission run (`nola-real-permission-digits-h65e0z9i`
under the generated temporary evidence directory) exited successfully but was
not valid acceptance: the driver matched a composer placeholder emitted before
the second permission decision. Its raw terminal capture still showed the
permission panel. The new opt-in vt100 replay assertion correctly rejected that
capture; the persisted pending run is not evidence of a product cancellation
failure because the old driver terminated the child before processing the key.

The driver now observes only output after the raw numeric choice (no Enter),
and retains terminal bytes on failure. Offline contracts verify the observation
offset, exact key bytes, failed capture retention and child cleanup (3/3 tests,
including the existing canonical-body reader). Choice 3 means return to composer,
not deny; neither is permission for a second write.

The corrected real-provider run (`nola-real-permission-digits-tglxbpje`) approved
the first write using raw key 1 and created the expected 16-byte fixture. Subsequent
provider responses included HTTP 502; the canonical assistant did not complete
within 150 seconds and the driver exited 1. This is a failed/incomplete real
acceptance run, not a green permission or footer check. No user data or live
conversation was mutated by these generated-fixture tests.

Fresh focused CLI PTY regression passed 6/6 in 87.09 seconds
(`/tmp/nola-permission-focused-recheck.log`): task-scoped background permission,
pending permission on goal resume, restart work chain, deny/draft handoff,
return-to-composer cancellation and repeated compaction with exactly-once resume.
The separate real-capture replay test was explicitly ignored because no valid
new completed real-provider capture exists. The real-provider driver additionally
requires durable `[completed, cancelled]` run states after returning to composer;
the refreshed offline harness contracts pass 3/3. These deterministic PTY tests
do not replace the still incomplete real-provider acceptance.

### Mac history publication investigation and replay purity (2026-10-10)

A fresh native recorded-workspace run passed with sampling evidence in
`nola-history-current-profile-7pwp0id7/history.txt` under the temporary evidence
root (result bundle `nola-mac-history-acceptance-ld1gsgbx`). Main-thread samples
include 99 samples in `publishCanonicalMessages` -> Observation invalidation ->
SwiftUI transaction/layout flush, and substantial accessibility focus traversal.
Sampling perturbed timing (history max 307.744ms at index 0); this is a path
diagnostic, not a comparable clean performance result.

Added opt-in DEBUG `NOLA_DIAGNOSE_PAGE_PUBLICATION` structural counters and elapsed
publication timing, without contents, identifiers or paths. A clean diagnostic
run (`nola-mac-history-acceptance-spkbcwvr`, exported to
`/tmp/nola-page-publication-diagnostics`) reports a 13-row same-owner page update
with zero text/journal-reference changes but two presentation changes. Its history
max wake lateness remains 134.272ms (index 57). Setter duration itself was only
0.107ms; the sampled synchronous flush is not guaranteed on every publication.

Code inspection also found a replay-purity defect: `finish()` stamped wall-clock
time onto unfinished Parts while projecting terminal stored messages. It now
accepts an optional finish time; the canonical reader supplies nil for unknown
times, while actual live completion keeps the existing default timestamp. Existing
terminal Part timestamps are untouched. Added completed/failed/cancelled repeated
projection equality and unknown-time coverage.

Build succeeded (`/tmp/nola-replay-timestamp-build.log`; existing headermap warnings
remain). Projection, visibility, overlay and scroll tests pass 40/40 with no skips
(`/tmp/nola-replay-timestamp-tests.log`, bundle
`nola-mac-history-acceptance-0mfakhgn`). The original long conversation still has
133.976ms max history wake lateness (index 58), and the same two presentation-only
changes. Thus replay timestamp purity is fixed, but it does NOT explain or resolve
the observed history stall. Next investigation must identify the remaining exact
presentation-field changes and resulting layout work. RSS 576MB was measured in
this multi-class process and must not be compared with the earlier isolated
332MB process as a product memory regression without an isolated repeat.

### Stable canonical tool display eliminates spurious history updates (2026-10-10)

Further opt-in field diagnostics isolate both changed rows to traces only. Run
`nola-mac-history-acceptance-vj0018gf` shows 8/8 and 33/33 matching trace identities,
unchanged order, detail, lifecycle, timestamps and structured tool metadata; history
wake lateness still reaches 124.727ms. The canonical JSON display encoder used for
tool arguments/results did not sort dictionary keys, so equal structured values
could produce unequal presentation strings on reread. The older trace JSON display
path already sorted keys; this canonical path now does too. Nested object order,
array order, round-trip value equality and literal string passthrough have explicit
regression coverage. No model input, authorization rule or canonical stored value
is rewritten by this display change.

First isolated rebuilt native replay (`nola-mac-history-acceptance-6lo8nlrr`,
`/tmp/nola-stable-json-isolated-test.log`, diagnostic export
`/tmp/nola-stable-json-isolated-diagnostics`) passes. The 13 -> 13 presentation-only
publication disappears. History max wake lateness is 27.052ms (index 29), p95
2.138ms; resident memory is 321,650,688 bytes. The historical page remains at 13
visible messages, compared with the previous spurious-update run's 14; the test
still reaches every expected message across history and latest pages. These are
one-run measurements, not a claim that all scrolling/UI or memory requirements
are satisfied. Follow-up independent measurements and broader regression remain
required.

### Historical-page/live-overlay ownership boundary (2026-10-10)

The broader stable-JSON regression passed 84/84, but its recorded workspace still
hit 124.695ms and grew the historical page from 13 to 14 visible messages despite
zero spurious canonical publications (`nola-mac-history-acceptance-x7btwe2e`). This
contradicts treating the first isolated 27ms result as a complete stall fix.

The overlay merge unconditionally appended missing live owners to any canonical
page. Added an explicit `canonicalPage` display scope for selected history: matching
rows may receive updates, but off-page overrides/pending submissions are retained
without being appended. Internal execution views retain their latest scope. Tests
cover page membership, on-page correction, retained off-page state, pending rows,
and returning to a latest view. The recorded workspace now asserts exact page IDs
remain unchanged throughout scrolling (all expected messages must still be reached
across page navigation).

That boundary passes 86/86 tests (`/tmp/nola-history-overlay-scope-tests.log`,
bundle `nola-mac-history-acceptance-g1hf7fdx`); recorded history stays at 13 rows,
with 62.402ms maximum wake lateness. Subscription was additionally materializing
the active answer's reveal tree even when only its Run metadata—not its message—
belonged to the history window. That cosmetic projection is now admitted only if
the active assistant belongs to `window.messages`. Core recording, run lifecycle,
permission handling and stored content are unchanged; returning to latest reads
current canonical state. Combined regression and independent isolated repeats are
run after this final admission guard, not inferred from the prior 86-test result.

Final combined verification passes 86/86 with zero failures/skips
(`/tmp/nola-owner-admission-regression.log`, bundle
`nola-mac-history-acceptance-sjm3_zwd`). Page identity remains stable, and spurious
trace publications are absent. The multi-class process nevertheless reports a
127.473ms history peak, so the full performance objective remains open.

Two subsequent independent recorded-workspace processes both pass the full native
navigation/scroll checks with the new exact page-membership assertion:

| Run | History wake p95 | History wake max | Peak sample index | History RSS |
| --- | --- | --- | --- | --- |
| isolated-a (`nola-mac-history-acceptance-ryslhk7v`) | 2.060ms | 62.514ms | 58 | 319,651,840 bytes |
| isolated-b (`nola-mac-history-acceptance-luz1l1ur`) | 2.133ms | 65.640ms | 59 | 319,881,216 bytes |

Logs: `/tmp/nola-owner-admission-isolated-a.log` and
`/tmp/nola-owner-admission-isolated-b.log`; exported diagnostics use the same stems
plus `-diagnostics`. Both history pages retain 13 messages, all expected messages
remain reachable across pages, and there is no identical-owner trace rewrite.
These independent peaks are lower than the previously repeated isolated
126–135ms results, but still exceed a frame budget. The earlier one-off 27ms result
is not treated as representative, and the multi-class 127ms result is not omitted.
Next profiling must target the remaining delayed work near sweep indices 58–59
with no off-page answer rendering or unstable JSON string comparisons left in
that path. All processes for this verification batch exited normally.

### Code-block palette preparation and remaining stall (2026-10-10)

Fresh post-scope native sampling passes in `nola-scoped-history-profile-0qh5sljw`
(result `nola-mac-history-acceptance-s2rx6r2u`). The main-thread sample includes 42
code-block body samples, 33 in first-pass highlighting, while canonical terminal
service lookup accounts for one sample. This does not establish the entire
remaining slow-frame cause, but confirms code preparation remains on the UI path.

`MacCodeSyntaxHighlighter` now prepares its finite font/color palette once per
document. Explicit appearance resolves each palette color once, rather than
copying the full highlighted attributed string and resolving colors separately
for every token run. Source, matching rules, typography and highlighting semantics
remain unchanged; no content cache was added. A 250-line regression compares every
attributed-string value against the previous per-run resolution algorithm in
light, dark and both high-contrast appearances. Existing Unicode, token, line
number, language, theme and mounted-block cache tests remain included.

Build succeeds; code-reference plus scroll group passes 16/16 with zero failures
or skips (`/tmp/nola-highlight-palette-tests.log`, result
`nola-mac-history-acceptance-6mflzdpt`). Prepared explicit-appearance rendering takes
2.94–3.27ms for 250 lines in that run. Whole-history wake lateness still reaches
112.933ms. A separate recorded-workspace process also passes its navigation and
exact membership checks, but records 114.368ms at sample 57, p95 2.143ms, RSS
319,946,752 bytes (`/tmp/nola-highlight-palette-isolated.log`, result
`nola-mac-history-acceptance-jxz8a6o4`). Thus this removes repeated per-token work
and preserves output, but is NOT evidence that the remaining overall scroll stall
is solved or that the previous isolated 63–66ms peaks are reliably maintained.
No process from this batch remains running.

### Static text rendering and measured main-loop stall (2026-10-10)

An attempted delayed one-second sample (`nola-late-scroll-profile-1hypqqa2`, native
result `nola-mac-history-acceptance-9402m9ow`) captured only 51 main-thread samples,
48 waiting for events. It did not establish the expensive stack; do not interpret
that short sampling miss as proof that the UI was idle during the observed stall.

Cold/static `MacStreamingInlineText` now renders its attributed content directly
instead of creating a paused TimelineView, reveal task and custom line renderer.
Live callers retain the existing animated subtree and reveal behavior. A new
hosted Markdown check compares static and reveal-suppressed live layout at 420pt
and 960pt widths, including multilingual prose, emphasis and code. Markdown,
streaming-state and native scrolling regressions pass 52/52 with no skips
(`/tmp/nola-static-text-render-tests.log`, result
`nola-mac-history-acceptance-hhu0o18z`). The observed history wake peak remains
108.498ms, so this path separation is not claimed to solve the main stall.

Added opt-in XCTest-only `NOLA_DIAGNOSE_RUNLOOP` instrumentation, observing
after-waiting/before-waiting spans separately from asynchronous wake lateness.
Before-waiting observation runs after normal AppKit/Core Animation commit hooks;
output contains only timing/phase information. The independent instrumented native
run passes (`/tmp/nola-runloop-probe-test.log`, result
`nola-mac-history-acceptance-tz6jtuhi`, diagnostics
`/tmp/nola-runloop-probe-diagnostics`). History captures 152 active spans; at uptime
61426.037709–61426.190461 an active span lasts 152.752ms, overlapping the 107.678ms
late wake at sweep index 59. Thus this instance is actual prolonged main-loop work,
not merely a late asynchronous task with an otherwise idle UI. Other transcript
sweeps have 11–40ms maximum active spans. Next profiling should rendezvous at the
scroll-sweep start itself and capture this delayed active interval rather than
assuming page-load marker time is the same phase. All test/build processes have
finished; the overall performance acceptance remains open.

### Sweep-aligned sampling, snapshot rendering and footer recheck (2026-10-10)

The opt-in XCTest marker `sweep-<phase>.pid` now rendezvous with the actual
scroll sweep, not page navigation. The aligned external sample completed in
`nola-sweep-aligned-profile-jig9_8yw`; its native result
`nola-mac-history-acceptance-gf7ktzbj` FAILED the bottom assertion: visible maxY
5825 versus document maxY 10451. The run-loop active maximum was 1338.582ms.
External sampling affects timing; this is not a representative uninstrumented
latency, nor may the failed bottom assertion be discarded as a passing run.
Diagnostics: `/tmp/nola-aligned-profile-diagnostics`.

Main-thread sampling includes journal projection publication and repeated
`needsHistoricalReader` scans inside individual Part rendering. The view now
captures one matching projection and derives that page-wide flag once per body
evaluation, rather than rereading the owner and scanning every Part per row.
No journal contents, permissions or history membership are changed.

Test-configuration build passed (`/tmp/nola-journal-snapshot-build.log`). An
independent native recorded-workspace run passed 1/1 with no skips (result
`nola-mac-history-acceptance-jocux6eq`, log
`/tmp/nola-journal-snapshot-acceptance.log`, diagnostics
`/tmp/nola-journal-snapshot-diagnostics`). History retains exactly 13 rows; RSS
391151616 bytes; wake lateness maximum 120.477ms and main-loop active maximum
145.091ms. Thus the remaining stall is NOT solved; a single passing run also
does not establish that the sampled bottom-position failure is resolved.

Current CLI footer PTY recheck: 4 passed, 0 failed, 1 ignored
(`/tmp/nola-footer-current-verification.log`, `cargo test -p nola_cli --test
pty_interaction footer -- --test-threads=1`). Covers Chinese completed output,
next-prompt uniqueness, mobile height changes and the active second-turn footer.
The ignored case still requires a valid isolated real-provider permission capture;
it is not accepted coverage. Raw-digit/new-frame harness contracts pass 3/3 via
`python3 scripts/test_real_cli_resilience_contract.py`; initial module-style
unittest invocation failed import and did not run tests. No real-provider
permission acceptance was completed in this batch. All launched processes exited.

### Real permission entry and discovery contract failures (2026-10-10)

Two fresh isolated real-provider runs used the configured provider without
changing credentials, the user repository or the live conversation database.
`nola-real-permission-current-7la_hc3m` failed before any approval: the model
completed with prose saying write_file was unavailable and incorrectly suggested
the fixture directory was outside its workspace. This is not an authorization
denial; no tool was attempted. The existing global policy already advises
discovery, but the tool-local tool_search description only advertised schema
lookup. It now explicitly states deferred-schema activation for later calls,
discovery before declaring unavailability/substituting a broader tool, and that
discovery grants no permission. No routing keyword rule or permission bypass
was introduced. The catalog contract test passes 1/1
(`/tmp/nola-discovery-contract-current.log`); binary rebuild passed
(`/tmp/nola-discovery-current-build.log`).

The new-binary run `nola-real-permission-discovery-u0qse60t` also FAILED overall.
Its first operation used bash, requested permission and completed after raw digit
1 without Enter; the expected file contents were verified. The second Run ended
without a write or approval request. The model again claimed write_file was
unavailable, and additionally interpreted the supplied literal
SECOND_MUST_NOT_BE_WRITTEN as an instruction to refuse. Thus the description
clarification has NOT established reliable discovery, and neither real run
proves allow-once repetition, return-to-composer or post-permission footer
acceptance. Failed raw terminal captures and isolated databases are retained.

The second requested payload is now the neutral SECOND_REVISION to avoid
confounding the permission test with instruction-like fixture data. The actual
invariant remains unchanged: first content must remain intact, second operation
must ask, raw digit 3 must cancel, and durable statuses must be completed/cancelled.
Harness contracts still pass 3/3. The neutral-payload real-provider case has not
yet run. All processes from this batch exited; no background acceptance is live.

### Foundational tool surface and fresh approval acceptance (2026-10-10)

Balanced mode exposed general bash while deferring basic file writes/edits. Its
initial metadata now includes write_file, edit_file and apply_patch alongside
read_file and bash. Concise still defers mutations, Detailed remains larger, and
all tiers retain discovery. No user-text routing, grants, permission domains or
parameter schemas are changed. Three request-density contracts pass
(`/tmp/nola-foundational-tools-tests.log`) and binary build passes.

Fresh neutral-payload run `nola-real-permission-foundational-pq7qk3ux` used the
correct write_file tool, asked for permission and completed the first write after
raw digit 1. It FAILED on the second request: the model treated the consumed
allow_once receipt as a prohibition on submitting a new call. Its response
explicitly said additional permission was required but never let Core ask for it.

Core permission feedback now distinguishes a consumed/expired grant from denial:
for a new user-requested operation submit the concrete call so Core can obtain
fresh approval before execution; never reuse the old grant. Existing denial and
no-circumvention rules remain. This shared policy is preserved across all tiers.
Two prompt-budget contracts pass in
`/tmp/nola-permission-feedback-verified-tests.log`; initial `prompt_budget` test
filter matched zero tests and is not counted (the module name is
prompt_composer::budget). The initial hyphenated package name was also corrected
to nola_core before verification. New binary build succeeds.

Real-provider acceptance now PASSES in isolated fixture
`nola-real-permission-feedback-kx81xhho`: first raw digit 1 writes exact first
content, a second operation requests fresh permission, raw digit 3 returns to
composer, durable Run states are completed/cancelled and the first file content
is unchanged. No Enter is sent after either digit. The captured terminal bytes
also pass the previously ignored actual-screen replay test with no skips
(`/tmp/nola-real-permission-feedback-replay.log`): one composer, one Context
footer below it, no leading tree-line character and no permission-panel residue.
This is one real-provider sequence, not proof of every provider/permission case
or a resolution of the separate Mac scroll stalls and bottom-position failure.

The current binary's permission PTY group also passes 7/7 non-ignored cases
(`/tmp/nola-permission-feedback-pty-regression.log`), covering background task
scope/isolation, pending approval before resumed parent requests, restart,
denial/draft preservation, return-to-composer cancellation and repeated
compaction/exactly-once resume. The capture test is ignored in that ordinary
group but passed separately above using actual real-provider bytes. All build,
real-provider and regression processes in this batch finished.

### Preserve explicit scroll intent through gesture-end reflow (2026-10-10)

MacTranscriptScrollController previously reclassified follow intent from geometry
at didEndLiveScroll, even after a real didLiveScroll had already reported the
user reaching the bottom. Async hydration between those events could grow the
document and misclassify that user as reading history. It now retains the last
observed user-scroll intent; endpoint sampling remains the fallback for discrete
mouse wheels without didLiveScroll. A later actual scroll away still cancels
following. This does not force following merely because document geometry changes.

Added a real NSScrollView regression with content growth between reaching bottom
and ending the gesture, followed by intentional scrolling away and further growth.
Test-configuration build passed (`/tmp/nola-scroll-intent-build.log`). The full
MacTranscriptCompletionScrollTests group passes 8/8, no skips, including the new
regression and isolated recorded-conversation workspace interaction
(`/tmp/nola-scroll-intent-tests.log`, result `nola-mac-history-acceptance-gnr2jlr5`,
diagnostics `/tmp/nola-scroll-intent-diagnostics`). Historical membership remains
13 messages; history wake lateness still reaches 106.793ms. RSS is 570982400 bytes
in this multi-test process, not an isolated memory baseline. The separate sampled
mid-gesture bottom assertion failure is not claimed resolved solely by this test;
remaining rendering latency and memory acceptance are open. All processes exited.

### Lower-rate native layout profile (2026-10-10)

Read-only inspection of the recorded fixture's journal heads rules out a stale
running-text explanation for the history: all text Parts are final and all
reasoning Parts are completed (including those attached to the unresolved Run).
The unresolved Run does retain one waiting-for-permission tool. No state or
history was rewritten to obtain these observations.

Sampled the current native Test binary at 10ms instead of 1ms, rendezvous at
the history sweep marker, for three seconds. Evidence directory:
`nola-lowrate-history-profile-hs6br1rh`; result
`nola-mac-history-acceptance-8s78i3rz`; exported diagnostics
`/tmp/nola-lowrate-history-diagnostics`. Native test passes 1/1, no skips, with
13 history rows and RSS 391331840 bytes. Wake lateness peaks at 173.267ms, index
57; run-loop active maximum 210.757ms. Sampling still perturbs execution; these
values are not an uninstrumented performance comparison or an improvement claim.

The main-thread sample includes 23 AppKit layout-tree samples, 21 NSHostingView
layout/render samples, 15 renderDisplayList samples and nested display-list view
updates. App-local parser/highlighter/body frames appear sparsely (one sample
each in this capture). This shifts the next investigation toward native view
hierarchy and invalidation breadth, not an assumed runaway historical streaming
animation or another speculative parser micro-optimization. The capture also
includes later XCTest accessibility traversal, so aggregate stack counts must
not be equated with the exact isolated stall duration. All processes exited.

### Remove redundant journal handover publications (2026-10-10)

Inspection confirmed two publishers for the same handover state: the shared
journal owner adopted activity/reference coverage after reading Parts, then
MacMessageView callbacks adopted those identical sets into the same owner again.
The callbacks now update only the view-local fallback when no shared owner is
provided. Shared-owner publication compares next coverage values before assigning
them; unchanged cursor updates no longer mutate observed coverage in place.
Coverage retains its existing per-identity, monotonic union behavior, so removed
Parts cannot reappear through older trace snapshots. No execution state changes.

An observation regression verifies that the journal cursor advances while
identical activity/reference coverage emits no change notification. Build passes
(`/tmp/nola-coverage-invalidation-build.log`). Journal transport and native scroll
groups pass 52/52 with zero failures/skips (`/tmp/nola-coverage-invalidation-tests.log`,
result `nola-mac-history-acceptance-354qul8k`, diagnostics
`/tmp/nola-coverage-invalidation-diagnostics`). Recorded history keeps 13 messages,
but wake lateness still peaks at 121.796ms. RSS 573947904 bytes is from the combined
52-test process, not an isolated baseline. This verifies removal of redundant
state publication, not resolution of the remaining layout stall or memory target.
All processes finished.

### Separate scroll-position and elapsed-time stall hypotheses (2026-10-10)

The recorded native sweep now accepts diagnostic NOLA_SCROLL_SWEEP_STEPS
(bounded 60–480, default unchanged at 120). Percentile indices and endpoint
fractions derive from the chosen count. Assertions, content, 16ms cadence,
history membership and actual native scrolling remain unchanged.

Build passes (`/tmp/nola-scroll-speed-build.log`). Two independent unsampled
native runs each pass 1/1 with no skips:

- 60 steps: log `/tmp/nola-scroll-speed-60.log`, result
  `nola-mac-history-acceptance-r6n8dxje`, diagnostics
  `/tmp/nola-scroll-speed-60-diagnostics`. History wake peak 122.417ms at index 58
  (near the bottom).
- 240 steps: log `/tmp/nola-scroll-speed-240.log`, result
  `nola-mac-history-acceptance-1ncz81e3`, diagnostics
  `/tmp/nola-scroll-speed-240-diagnostics`. History wake peak 107.384ms at index 57
  (near the first quarter).

Thus the peak follows roughly one second of elapsed sweep time rather than a
fixed viewport location, despite a fourfold speed difference. Next investigation
should isolate delayed publication/layout after hydration instead of assuming
a particular mid-page code block or table is expensive on first visibility.
This is diagnostic evidence, not a product performance fix. The unconditional
one-second TimelineView in private MacAgentTimelineRow has no call sites and
must not be blamed merely because its interval matches. All processes exited.

### Align body publication with the delayed layout (2026-10-10)

Combined opt-in body, canonical publication, run-loop and layout instrumentation
passes 1/1 (`/tmp/nola-timed-publication-diagnostic.log`, result
`nola-mac-history-acceptance-0z9o_umz`, diagnostics
`/tmp/nola-timed-publication-diagnostics`). History page publishes at uptime
63571.638. Small/empty body prefixes publish at 63571.806; remaining 56/2976/11405
byte bodies publish at 63572.926–63572.957, immediately before a 63.818ms NSThemeFrame
layout and 107.819ms wake lateness. The overlapping run-loop span is 155.539ms.
There is no canonical page republish at that instant. These observations link
the late content arrival to layout, but do not yet identify where its wait occurs.

A controlled trial moved only synchronous projection long-poll waiting out of
Swift's cooperative executor onto DispatchQueue. It built and passed the same
isolated native test (`/tmp/nola-poll-io-acceptance.log`, result
`nola-mac-history-acceptance-z7hamwye`, diagnostics `/tmp/nola-poll-io-diagnostics`),
but history still peaked at 130.814ms at index 59, active span 156.357ms. Thus this
change did not remove the timed stall. The experimental bridge was reverted in
source instead of retaining speculative complexity; the Test binary still
contains that experiment until the next build. Next diagnosis should timestamp
body request admission, worker start and range response, not presume long-poll
starvation. No process remains running and no production data was changed.

### Locate delayed bodies in service window projection (2026-10-10)

Added opt-in DEBUG range timing with admission, worker start, port connection,
snapshot-open completion and range return timestamps; only durations and byte
counts are emitted. Native build/test pass (`/tmp/nola-range-phases-tests.log`,
result `nola-mac-history-acceptance-vb_wk4as`, diagnostics
`/tmp/nola-range-phases-diagnostics`). At history hydration, the three nonempty
body tasks start almost immediately around uptime 63835.480, but snapshot open
does not finish until 63836.603–63836.604. Range return follows within milliseconds.
The delayed phase is snapshot service admission/execution, not Swift task startup
or Markdown preparation. The prior experimental long-poll bridge is absent from
both current source and the rebuilt Test binary.

Extended existing service-query diagnostics to print after the history sweep,
not only initial window attachment. The initial timing run before that extension
passed but could not report the relevant phase. Updated build/test pass
(`/tmp/nola-history-service-timing-tests.log`, result
`nola-mac-history-acceptance-jl2fbngm`, diagnostics
`/tmp/nola-history-service-timing-diagnostics`). The service reports a historical
load_conversation_projection_window_v1 at 1170069us and subsequent
load_conversation_projection_window_update_v1 at 1175338us, with negligible lock
wait for those expensive queries. Other simple requests wait over 1.14 seconds
for the shared runtime lock. Snapshot operations do not have this query timing
classification, but their client-side delay coincides with these expensive window
queries. The next fix belongs in window projection cost/redundant reconstruction
under the runtime lock, not speculative UI animation changes. Native history
wake lateness still reaches 129.171ms. All processes finished.

### Startup recovery is not a new transport failure (2026-10-10)

The user's 19:28 startup restored Run DC1ACABA-9C9C-4CB1-8249-1980B05D4D2B,
created at 01:34 and last updated at 02:06, still nonterminal. Startup recovery
now retains its read-only inspection before loading the frozen launch descriptor,
reports an absent invocation explicitly, and labels a restored task separately
from a newly interrupted transport. A cancellation action is available for the
exact inspected local Run even when its resume configuration cannot be restored.
It uses the existing durable cancellation delivery and terminal reconciliation;
no synthetic completion, automatic replay, new submission or permission grant.

Test configuration build passes (`/tmp/nola-startup-recovery-build.log`). Recovery
inspection, local terminal/cancellation contracts and recorded full-workspace
acceptance pass 21/21, zero failures/skips (`/tmp/nola-startup-recovery-tests.log`,
bundle `nola-mac-history-acceptance-vuwjt23r`). The freshly rendered
`/tmp/nola-recorded-full-workspace.png` was visually inspected: the Chinese
unfinished-task notice and cancellation action are both visible. The captured
conversation uses persistence-only execution adapters; its historical commands
were not replayed. The user's live database and signed Debug app were untouched.
This does not establish that every historical launch can resume, nor does it
claim the broader paused performance goal is complete.

### Explicit CLI AllowAll is one execution policy (2026-10-10)

The 19:29 CLI conversation approved bash for Nola's repository, then failed at
the independent workspace boundary before execution. `/allow` also updated only
environment/booleans, leaving the typed Core profile stale. The picker now updates
that typed profile. Host construction freezes it into the execution's permission
scope; tools do not infer unrestricted authority from auto_approve or read the
process-global profile at execution time. Explicit AllowAll permits filesystem
resources outside configured roots, bypasses the internal shell path checker and
staged-Git preservation sandbox, and approves local tool policy. Working-directory
selection, argument validation, execution receipts, cancellation and OS access
checks remain intact. Recommended/Safe retain their restrictions. Resource scope
is not persisted as a directory grant and does not leak to another execution.

Three resource/Git/shell regressions pass; the picker transition test passes.
Full tools regression: 420 passed, one failure in the pre-existing tool_search
description-length budget (not changed here), zero ignored
(`/tmp/nola-allow-all-tools-regression.log`). Picker evidence is in
`/tmp/nola-allow-all-picker-test.log`. Normal CLI build succeeds and updates
target/debug/nola (`/tmp/nola-allow-all-build.log`). No real-provider rerun or
user-repository command replay was performed for this change.

### CLI receipt and capability-boundary correction (October 10, 19:48 session)

Read-only inspection of the latest CLI run found a rejected `bash.root` argument,
a successful explicit-selection commit of 146 files, and a subsequent nonempty
Git status. The final model answer nevertheless claimed a clean worktree and
answered the synthetic capability-activation notice as if it were a new request.
This was not an AllowAll permission denial. A provider request also failed and
was retried; the run itself reached completed.

Bounded model tool projections now preserve budgeted scalar receipt facts instead
of retaining only a clipped JSON string. Git receipts include the observed
remaining changed-file count and worktree-clean flag, while retaining the explicit
false task-completion attestation. Capability activation remains in typed schemas
and tool-result metadata, not a new trailing conversational instruction. The bash
schema explicitly identifies `cwd` and its invocation-directory default; invalid
arguments remain rejected rather than being silently reinterpreted.

Three bounded-output tests, one capability-boundary test and eight isolated Git
transaction tests pass. User history and repository commits were not modified by
the verification. No real-provider replay was performed, so this does not prove
that the configured model can no longer make unsupported completion claims.

### Recorded Mac compaction cancellation (October 10, 20:11 session)

Read-only inspection found incremental compaction with 14 initial segments. The
first summary exhausted its 8,000-token output allowance and was split; three
subsequent segments succeeded before the Host reported `cancelled`. Recorded
attempts total 370,999 ms and 29,136 reported output tokens. Cancellation origin
is not established. This is not a terminal truncation failure.

Core now treats that Host response as cancellation, clears the pending compaction
projection, and retains original context without consuming a compaction failure
allowance. A metadata-only fixture in
`core/tests/fixtures/mac_compaction_20261010.json` replays the split/checkpoint
trajectory with synthetic content. A separate Core contract test verifies the
terminal state, preserved transcript/context, and absence of failure events.

All 11 compaction contract tests and the recorded Host planner replay pass
(`/tmp/nola-recorded-mac-compaction-core.log` and
`/tmp/nola-recorded-mac-compaction-host.log`). These are isolated deterministic
regressions, not real-provider performance or summary-quality acceptance. The
six-minute partial-compaction latency remains an open issue; this correction does
not make incremental checkpoints durable across process restarts. Live history
and the signed Mac app were not modified.

### Cold-compaction generation amplification correction

The recorded run spent 76–98 seconds per completed physical request, producing
29,136 output tokens before cancellation. The cold planner imposed both a half
input-window allowance and a fixed 64,000-byte segment ceiling, and rewrote the
whole rolling checkpoint per segment. It now packs against the reserved window
instead; exact serialized request checks and bounded splitting remain in place.
The synthetic 14-entry captured-size workload requires at most seven segments
instead of fourteen. Cold requests cap output at 4,096 tokens and prompt for at
most 1,024; cached request forks keep their existing physical request contract.
Incomplete summaries remain rejected, not silently accepted or text-truncated.

These are request-count/output-budget improvements, not measured real-provider
latency guarantees. Missing request snapshots, checkpoint quality under the new
budget, and restart durability still require real-provider acceptance.

Compaction regressions pass 24/24, including serialized HTTP output-budget and
truncation-recovery coverage (`/tmp/nola-compaction-latency-tests-final.log`).
Rust CLI/Host binaries build successfully; the signed Mac app was not replaced.
