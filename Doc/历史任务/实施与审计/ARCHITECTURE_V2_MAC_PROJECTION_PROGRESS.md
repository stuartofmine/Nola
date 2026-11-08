> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Mac projection implementation status

Date: 2026-09-05

This is an implementation checkpoint, not an acceptance report. The governing
requirements remain in `ARCHITECTURE_V2.md`. No architecture requirement is
considered complete merely because a change below has been applied.

## Observed production-session evidence

The local conversation `0A75A3DC-8AD8-4927-9E31-800A89E3646C` contains five
user/assistant pairs. The last run completed around 02:35 on September 5.
Its log repeatedly reported an append-only projection violation, with
`acceptedBytes=424` while authoritative content grew to 3668 bytes.
The canonical database contains the final answer. This establishes a
client/projection disagreement, not a missing provider answer.

The log message was not located in the current Mac sources during inspection.
Do not assume that the running application matches the current worktree.
Build identity and a fresh reproduction are necessary before claiming this
incident is fixed.

Screen capture failed with `could not create image from display`. No current
visual acceptance evidence was obtained. Logs do not establish whether spacing,
animation, scrolling, folding, or typography meet the UI requirements.

## Applied changes

The Mac application subsequently built successfully for the local arm64 Mac
destination with signing disabled. The contracts test target also compiled
after its outdated constructors, backup fixtures, and removed Host-history
interfaces were updated. The first executing suite has passed several real
Host boundary cases, including permission isolation/reuse, denied deletion,
compacted-history recall, and image attachments. The full run is still in
progress; this is not a suite-level pass or visual acceptance.

The active run log is `/tmp/nola-mac-projection-contracts-seventeenth.log`.
Xcode continues to emit locked-device discovery diagnostics despite the
architecture-qualified Mac destination. Avoiding iOS test selection alone has
not resolved that test-runner isolation issue.

### Semantic reveal buffer

`MacAssistantPresentation.swift`: the paced buffer traverses pending text once
and tracks phrase length instead of repeatedly removing its first character
and recounting the phrase. This preserves the existing boundary policy. It is
not proof that long-conversation memory or frame-time budgets are satisfied.

### Reasoning identity and lifecycle

`MacCanonicalConversationRepository.swift`: each reasoning trace now uses its
canonical Part ID. Previously every reasoning Part in a message used one
message-derived ID, so later steps replaced earlier steps. The trace state now
comes from the Part rather than the parent message, including cancelled and
failed states.

### Projection acknowledgement

`MacApplicationPort.swift` now reads a suffix without advancing the committed
client cursor. It checks conversation identity, contiguous sequence, and the
returned cursor. An empty suffix cannot advertise further pages.

The repository reads subsequent pages using the provisional cursor, reduces
the events, then acknowledges the cursor using a compare-and-swap operation
before caching the projection. A failed reduction does not acknowledge the
events. The page-draining loop checks cancellation.

## Remaining ownership and resource risks

1. Snapshot, cursor, and cached projection do not yet form one subscriber-owned
   state object. Cursor acknowledgement alone does not prove correctness when
   snapshot refreshes and subscribers overlap.
2. The suffix-draining loop accumulates all pages before publishing. A sustained
   producer can delay visible updates and grow memory. Replace this with a
   bounded, transaction-consistent publishing boundary; do not expose half a
   transaction merely to cap batch size.
3. Snapshot loading still assembles all message pages. Pagination of transport
   alone does not satisfy bounded resident history or viewport loading.
4. The generic thinking stream accumulator still guesses snapshot/delta
   semantics from text prefixes and suffixes. Callers must move to explicit
   event identity and operation semantics, not additional string heuristics.
5. Projection construction still assembles text across Parts. Stable Part-level
   render ownership and order across reasoning, tools, and text need acceptance
   coverage for both cold snapshots and live updates.
6. Failure placeholders are recognized by localized text in the transcript
   reducer. Errors should be typed Parts, not text that requires recognition
   and replacement.

## Required verification before acceptance

- Compile the Mac target and relevant test target using a qualified local Mac
  destination. Do not enumerate, unlock, or poll connected iOS hardware.
- Project multiple reasoning Parts with distinct IDs and identical text;
  preserve all identities and their independent terminal states.
- Inject a reduction failure after receiving a suffix. Retry from the same
  acknowledged cursor and recover the exact content without duplicate text.
- Exercise overlapping snapshot refresh, subscription, and cancellation;
  never acknowledge a suffix against a mismatched base projection.
- Exercise repeated, missing, wrong-conversation, and out-of-order events,
  including multiple mutations in the same transaction revision.
- Sustain a producer while switching between multiple ten-million-character
  conversations. Measure memory, first visible content latency, and frame time.
- Observe screenshots and a live recording of Markdown streaming, open code
  fences, reasoning expansion, permission interruption, and terminal updates.
- Reproduce the latest session on the identified newly built application;
  compare canonical Parts with visible content, rather than only checking that
  the request completed successfully.

The application and test-target compilation checks have now been executed.
The running contracts suite does not yet provide results for every check above;
in particular, current screenshots, sustained-producer measurements, and
snapshot/subscriber conflict injection remain unverified.

## 2026-09-05 follow-up: failures reproduced and bounded-window groundwork

The interrupted contracts run exposed real failures. It is not a passing result.
Its cancellation test was sampled while stuck in Foundation `waitUntilExit()`;
the owned test run was interrupted and exited with status 75.

Implemented and individually verified:

- Full projection assembly now combines every page's Core-owned surface, rather
  than combining all messages with only the first page's surface. Duplicate
  identities, inconsistent revisions, and non-progressing pages are rejected.
  Both ten-million-character Mac performance tests passed without relaxed limits.
- Remote Host test teardown uses asynchronous termination notification with a
  five-second failure deadline instead of blocking `waitUntilExit()`. All five
  remote connection tests passed, including cancellation.
- Identifying a provisional tool Part as protocol plumbing removes only that
  Part's stale UI row. Real tools retain title/state updates. Three targeted
  presentation tests passed.
- A cancelled or superseded recall request cannot clear newer query results on
  its error path. Recall tests now await observable search completion and cover
  switching conversations during a pending search. All three recall tests passed.
- Empty authoritative text remains a meaningful reset, not a missing update.
  The metadata test follows version 2 while retaining checks against duplicated
  canonical facts. The workspace and repository suites passed 138 tests together.
- Mac has a typed bridge to Core's bounded history window command. History
  ordinals are separate from event journal cursors. Nine repository tests passed,
  including complete backwards traversal with no repeated or missing messages.
- Store chooses retained message headers using UTF-8 text and JSON byte lengths
  before hydrating Part payloads. This avoids allocating the rejected page bodies.
  One oversized message is still returned intact to guarantee progress; this is
  not a strict per-message memory cap. New tests cover a ten-million-character
  message and multi-byte text with structured content. `cargo test -p nola_store`
  passed; no query-phase RSS measurement has been performed.

Remaining architecture gaps, not acceptance exceptions:

1. Mac UI still loads complete transcripts; the new window bridge alone does not
   meet the bounded resident-history requirement. Visible history, live tail,
   subscription state, and eviction need an integrated implementation.
2. Remote window materialization still loads the full remote projection before
   retaining selected IDs. Local session projection loading can also decode a
   whole checkpoint. Both can defeat the transport window's apparent bounds.
3. Snapshot/cache ownership and event acknowledgement need coherent isolation.
   The current subscriber drains all available event pages into one collection.
4. No new screenshot-based visual acceptance or real-provider multi-turn session
   was performed by these targeted tests. Passing mocked Host contracts is not
   equivalent evidence.
5. Xcode still emitted locked-device discovery diagnostics during Mac-only runs.

A fresh complete Mac contracts run was started after these fixes. Its result is
pending, not implied by the targeted passes above. Its diagnostic log is
`/tmp/nola-mac-contracts-after-window-fixes.log`.

### Verification correction and context-query follow-up (2026-09-05)

This entry supersedes the earlier pending full-contract result and the description of accumulating all journal events before reduction.

- The full macOS contract run completed with 425 XCTest cases, 10 skips, and 4 failing assertions; a further 11 Swift Testing cases passed. After correcting the terminal context contract expectations and the missing permission localization, the targeted remote-context/localization run passed 14 tests. The full suite has not been rerun and is not claimed green.
- Journal pages are now reduced incrementally rather than accumulated into one event array. The subscriber still drains until caught up and retains a full projection; this is not yet a bounded live-tail architecture.
- The Application command `load_conversation_context_usage_v1` reads canonical usage inputs without decoding the full Rust conversation projection. Local inputs include strict identity, cursor, integer, and usage-origin checks. Remote inputs read the canonical remote usage projection. SQLite may still parse the stored JSON internally.
- macOS context statistics use that command. CLI/Web conversation metadata queries now use dedicated context-usage and settings queries instead of loading full conversations and projections. `cargo check -p nola_cli` passed; existing unused-code/import warnings remain.
- Six Application usage tests and a real-service cold-restart test passed in the preceding work. The real-service test uses a synthetic ten-million-character checkpoint, not an LLM. CLI unit tests use an in-process Application runtime and do not validate production service transport.
- Outstanding: wire macOS history windows into the actual chat UI; resolve snapshot/cache/cursor ownership; bound subscriber work; remove full remote-history decoding. No new screenshot-based visual acceptance, real-model conversation acceptance, or process-RSS measurement has been completed for these changes.

### Window/subscription integration boundary (2026-09-05)

The following supersedes earlier descriptions of full checkpoint decoding and port-owned subscription cursors:

- Mac repositories now hold the canonical projection and its event sequence as one cache entry. The transport no longer owns or implicitly advances a shared conversation cursor. Independent-repository and bounded-backlog tests passed.
- Local window/page reads select presentation inputs rather than decoding the complete Agent checkpoint. Part ownership is resolved from canonical message identity, persisted Part binding, then the first assistant message of the owning Run. Task and Skill records are selected for visible messages. A reopened SQLite test with two assistant messages in one Run and a ten-million-character older Part passed against the full-projection oracle.
- Remote window/page reads filter messages inside Store instead of deserializing the complete remote projection in Application. The remote commit/reopen integration test preserves reasoning, tools, text and context usage in the selected message.
- The window command now reads its projection and `event_sequence` inside `with_consistent_read`. `before_ordinal` remains an independent history position. The Mac window decode/subscription regression passed. These tests do not establish RSS, frame-time or screenshot acceptance.

#### Remaining UI integration requirement

`MacWorkspaceModel.loadSelectedConversationIfNeeded` still calls the full-history archive loader. `MacTranscriptView` currently limits an already-resident message array; it is not storage-backed pagination.

The existing generic subscription cannot be attached unchanged to a partial window: `replaceSurface` carries whole-conversation membership, while the reducer assumes all referenced messages are resident. Historical Part updates may also target nonresident messages. Silently ignoring every missing message would conceal actual protocol gaps.

The integration must preserve these distinct responsibilities:

1. Application defines which message identities belong to a subscribed window and supplies a coherent window projection/event position. Clients must not infer membership from timestamps or prompt text.
2. History pages retain their own ordinal boundaries and do not advance the live subscription cursor. Newly loaded older pages must not replace a newer live tail.
3. The page model exposes storage-backed previous-page availability and loading state; existing local-array slicing is not sufficient.
4. Window changes and live updates preserve stable Message/Part identities, scroll anchors, and current context usage. Concurrent deletion, mutation outside the window, and switching during a load need explicit coverage.
5. Acceptance still requires actual Mac UI interaction and screenshots, plus RSS/frame-time measurements across multiple long conversations. No claim of UI completion is made by the query tests above.

## Window integration and validation update (2026-09-05)

This entry supersedes the earlier statement that the Mac chat view has not yet
adopted windowed history reads.

- Chat selection now uses an Application-backed window model. Earlier pages
  replace the resident page rather than accumulating the full transcript.
- Concurrent loads of the same conversation and page share one pending task.
  Superseded loads report cancellation instead of publishing an empty window.
- Window updates use the Application event sequence, separately from the history
  ordinal. The client currently checks for changes every 250 ms; this is not yet
  a service-side wait/wake subscription.
- Permission restoration has a dedicated small read projection, avoiding full
  session hydration solely to restore permission state. This does not change
  authorization policy.

Validation completed:

- Three Rust permission-projection fixture tests passed; CLI cargo check passed.
- MacConversationLazyLoadingTests and MacChatWorkspaceBehaviorTests: 134 tests,
  zero failures. Log: /tmp/nola-mac-permission-small-read-tests.log.
- The central-header workspace snapshot test passed, and the generated PNG was
  opened and visually inspected. Log: /tmp/nola-mac-window-visual-tests.log.
  Image: /tmp/nola-workspace-ui.png. These temporary files are not repository
  artifacts.

The screenshot is a deterministic fixture, not the user's latest conversation.
Visual concerns remain: the model label truncates, the header has substantial
unused vertical space, and placeholder/timestamp contrast is weak. This capture
has no visible inspector or history navigation and cannot establish streaming
smoothness, page-switch continuity, or permission interaction correctness.

Remaining acceptance gaps include real multi-turn service/UI interaction,
streaming observation, long-conversation page switching, process memory/frame
measurements, canonical context accounting when only a history window is loaded,
and replacing idle polling with an appropriate service notification mechanism.
Passing the tests above does not close those gaps.

## Cursor-only waits and Mac window subscription (2026-09-05)

- Projection read queries no longer create durable command receipts in the
  service. Mutation admission and replay remain durable. A real-process test
  performs 128 unchanged window reads and checks both receipt count and SQLite
  data_version remain unchanged.
- Event suffix waits now release the Application mutex using a condition
  variable, rather than querying every 25 ms. Receipted command completion wakes
  readers, which recheck their own scopes. Unrelated commands may also wake
  readers; notifications are not yet scope-specific.
- Added projection_cursor_wait: a validated cursor and bounded wait duration
  produce only a current cursor. This is a wake hint, not acknowledgement or
  event delivery. Future cursors and waits above 30 seconds are rejected.
- Mac window refresh waits for this cursor hint before requesting its bounded
  window. The window retains its old acknowledged sequence until the complete
  new window is accepted. Normal idle observation no longer queries the window
  every 250 ms. A separate short yield remains while an explicit page load is
  publishing its result.
- Mac currently limits waits to 1500 ms. The synchronous socket wait is not
  immediately cancellable; obsolete responses remain guarded by task
  cancellation, window generation and selection checks. Transport cancellation
  and rapid-switch resource measurements remain open work.

Nine real-service process tests passed in
/tmp/nola-cursor-hint-service-tests.log, including concurrent suffix wakeup,
cursor-only response validation, query freshness, write replay and the existing
long-stream RSS check. This does not prove UI frame smoothness or memory bounds
for every conversation shape. The Mac XCTest run for this integration was
started separately; its result must be recorded after completion.

Mac integration validation completed: xcodebuild succeeded and 135 selected
XCTest cases passed with zero failures. The new window-wait/rename test passed
in 0.282 seconds. Log: /tmp/nola-mac-cursor-wait-tests.log. This validates the
production Mac port and window model, not screenshot-based streaming or
rapid-switch frame performance.

## Client query cancellation (2026-09-05)

Mac window refresh now propagates task cancellation to its cursor-wait socket.
A lock-protected cancellation handle shuts down the registered descriptor; the
request remains its sole closer and unregisters before closing, preventing
cancellation from shutting down a recycled descriptor. Projection queries do
not attempt durable receipt recovery on transport failure or accept an
"accepted" write receipt. Mutation receipt recovery is unchanged.

Mac build and 136 selected XCTest cases passed. The new test cancels a
five-second cursor wait, asserts return within one second, then renames and
refreshes through the same port. The whole test took 0.225 seconds (not a
standalone cancellation latency measurement). Log:
/tmp/nola-mac-query-cancellation-tests.log.

The service-side waiter can still remain until another command completes or
its bounded wait expires. Socket cancellation during connect/supervisor startup
and server-side disconnect-driven waiter cleanup are not yet proven. Rapid
multi-window resource and frame measurements remain acceptance gaps.

## Disconnect-aware service waits (2026-09-05)

Production projection waits now poll the client socket and a nonblocking
notification socket together. Registration is protected by the Application
writer mutex, so a commit between registration and poll queues a wake instead
of being lost. Peer shutdown ends the wait without a deadline or another
commit. No extra monitor thread or periodic SQL query is required. The waiter
registry allows at most 128 active waits; dead weak entries are pruned.
Health responses expose projection_waiters for operational visibility.

Ten real-process tests passed, including eight simultaneous 30-second waits
whose connections are closed after server registration. The test observes the
server waiter count return to zero within two seconds and checks no receipts
were added. Log: /tmp/nola-service-disconnect-wait-tests.log. Two unit tests also
passed, but the idle unit test still uses a no-socket Condvar branch; remove
that testing-only divergence rather than treating it as production-path proof.

Mac rebuilt and all six window-loading tests passed against the integration.
Log: /tmp/nola-mac-disconnect-wait-tests.log. This still does not measure UI
frames under rapid multi-window switching, nor bound all non-subscription
service connections. The IPC contract uses one request per connection; closing
the request side during a wait is treated as cancellation.

## Actual long-window performance fixture audit (2026-09-05)

The 10M-character tests now assert bounded selected windows instead of requiring
all 10M characters in client UI state. Screenshots capture the actual transcript
window; the previous short replacement message is no longer used as visual
proof. Full archive character preservation remains separately checked.

The run FAILED: two tests produced 15 assertions that the final tool/Skill
presentation was absent. Investigation found that the fixture's
restoreBackupSnapshot -> messageInputs path emits only text Parts, with the
Mac message/presentation serialized inside content_json. It does not seed
canonical tool Parts. The fixture therefore did not previously exercise the
claimed tool/Skill round trip. Fix the fixture to seed canonical Parts; do not
make the reader decode a platform UI blob as canonical activity.

Measured (not visual acceptance): two 10M conversations, 12 switches, mean
69.1 ms and P95 142.9 ms including the test's fixed 50-ms run-loop pump.
Final selected window: 24 messages / 240648 text bytes. Client cache warmup
growth: 34177024 bytes; switching growth: 12075008 bytes. These are client
process RSS differences, not total service memory or frame-time measurements.

The opened /tmp/nola-large-conversation-switch.png showed a large blank
assistant area. The contrast assertion still passed, demonstrating that it is
not sufficient visual verification. Offscreen capture/lifecycle and actual
rendering must be distinguished before assigning the cause. Metrics and log:
/tmp/nola-large-conversation-switch-metrics.json,
/tmp/nola-large-conversation-metrics.json,
/tmp/nola-mac-real-window-performance-tests.log.

## Canonical activity fixture and mounted capture (2026-09-05)

Long-conversation performance fixtures now use the production create/import
commands to write independent file.read and skill.execute tool Parts, followed
by the text Part. The test checks the imported capability IDs and restored
presentation. It no longer depends on decoding serialized Mac presentation
objects inside a text Part. Fixture tools are recorded results, not live tool
or Skill execution.

Captures are mounted in real AppKit windows and preserve the actual long text.
Both generated PNGs were opened; text is now visible. This change also switches
the final message to the semantic-presentation renderer, so mounting alone has
not been isolated as the cause of the previous blank image. Screenshots still
show joined sentences such as "validation.Attention" although fixture lines
end with newlines; investigate whitespace preservation independently.

Run result: single-conversation test passed; multi-conversation test failed its
64 MiB RSS-growth assertion (250118144 bytes observed). Its baseline is before
UI mounting, so this number includes initial UI allocation, not just repeated
switching. Separate cold render from warmed switching before diagnosing a leak.
Do not increase the gate to conceal the result. Twelve switches averaged
223 ms, P95 365 ms, including each fixed 50-ms run-loop pump. Window membership
and tool restoration assertions passed. Log:
/tmp/nola-mac-canonical-long-window-tests.log.

## 2026-09-05: completed transcript rendering lifetime

- Fixed the phrase-layout identifier conversion (`UUID.uuidString`). Explicit newline fragments now tell the flow layout to start a new line, including newline-only fragments. A mounted-window screenshot confirms the repeated fixture sentences no longer concatenate at their boundaries.
- Completed phrase groups now use the ordinary native Markdown document renderer. Only streaming groups retain per-phrase reveal state/tasks; completed structured blocks also avoid inactive reveal wrappers. Canonical message text and group identities are unchanged.
- Compiled and ran `MacMarkdownParserTests` and `MacLargeConversationPerformanceTests`: 20 tests passed. Evidence: `/tmp/nola-mac-completed-render-tests.log` and `/tmp/nola-large-conversation-switch.png`.
- Same two 10,000,000-character conversations, 12 measured switches, unchanged 64 MiB repeated-switch growth gate: P95 0.0877 s versus 0.2592 s in the preceding run; initial UI allocation 43,302,912 bytes versus 163,676,160 bytes; measured positive post-warmup RSS growth 0 versus 107,315,200 bytes. Switch duration includes the test's fixed 50 ms run-loop interval and is not a frame-time measurement. Zero positive growth is a clamped endpoint delta, not proof that no objects leak.
- The fixture uses the real Application Service and canonical message/Tool/Skill parts, not real LLM calls or actual Skill execution. This covers bounded-history restoration and native transcript rendering, not full product acceptance.
- Still unverified: visual continuity at streaming-to-completed renderer handoff, scrolling through tool cards outside the captured viewport, prolonged repeated-switch memory plateaus, and Application Service process RSS during live multi-turn runs. The broader Architecture V2 work remains incomplete.

## 2026-09-05: unified streaming/completed paragraph layout

- Replaced the phrase flow layout and per-phrase SwiftUI reveal views with a single concatenated native Text per group. Phrase identity is carried by TextAttribute; TextRenderer applies a 700 ms opacity/vertical reveal only while drawing new runs. The TimelineView pauses after the group's reveal finishes. Completion no longer switches to a different paragraph renderer.
- Removed the obsolete flow-layout helper/test and whole-block reveal wrapper. Existing canonical source-preservation tests remain; the mounted-window handoff test now compares widths 360 and 720 before/after completion.
- `xcodebuild test` for MacMarkdownParserTests, MacLargeConversationPerformanceTests, and MacVisualSnapshotTests/testStreamingCompletionPreservesNativeMarkdownHeight passed: 20 tests. Log: `/tmp/nola-mac-unified-text-tests.log`. Both handoff height assertions passed within 1 point; the preceding implementation failed by 83 and 62.5 points.
- Screenshots reveal an unresolved defect despite those green tests: code blocks show colored syntax tokens but omit ordinary foreground text, including in the completed 720-point snapshot. Removing whole-block animation did not resolve this. The cause must be investigated rather than described as an animation fix. Height equality does not prove visual completeness.
- Long-conversation switching P95 is now 0.2284 s, initial UI RSS growth 85,999,616 bytes, measured positive post-warmup RSS growth zero. The 64 MiB repeated-switch gate still passes, but latency regressed against the distinct completed-renderer experiment (0.0877 s). Immutable parsed/attributed content caching remains necessary; no claim of full performance completion.
- New-phrase animation across actual timed deltas, pixel-level code visibility, dark/light appearance consistency, and all-platform V2 acceptance remain open.

## 2026-09-05: native code foreground appearance resolution

- The missing ordinary code text was reproduced in the mounted SwiftUI screenshots while colored syntax tokens remained visible. The highlighter supplied semantic NSColor values without an explicit SwiftUI appearance when bridging to AttributedString.
- MacCodeBlockView now passes its colorScheme as an NSAppearance. The shared highlighter resolves foreground/background colors into device RGB under that appearance before returning the attributed string. Existing AppKit preview callers can retain native dynamic colors by omitting the appearance.
- Added a regression for light code under dark ambient AppKit appearance and dark code under light ambient appearance, checking plain identifiers, a function name, and line-number colors. No model routing, message persistence, or canonical data was changed.
- Compiled and ran MacCodeReferenceTests and the mounted Markdown handoff test: 7 tests passed. Log: `/tmp/nola-mac-code-appearance-tests.log`. Inspected the 360-point streaming and 720-point completed PNGs; both now show line numbers and the complete `let answer = 42` / `print(answer)` code. This corrects the unresolved visual defect recorded above.
- This is not a full dark-mode UI acceptance run or live LLM test. Long-conversation caching, timed incremental reveal coverage, and the broader cross-platform Architecture V2 requirements remain open.

## 2026-09-05: context authority audit correction

- Corrected an incomplete audit conclusion: the apparent local-first branch in `load_conversation_context_usage_v1` does not establish local-first authority. Store's local and remote usage SQL already compares checkpoint/mirror timestamps, using the same newer-remote/local-on-tie rule as session projection selection. No duplicate production selector was added.
- Extended the real Store remote large-history test to compare the dedicated usage query with the remote projection, insert an older local checkpoint, then update local to the same timestamp and reopen the database. The query retained remote usage while local was older and selected local usage on the tie, consistently after reopening.
- `cargo test -p nola_application --test context_usage_read_e2e --test remote_projection_identity_e2e`: 9 tests passed. Log: `/tmp/nola-context-authority-retest.log`. These are canonical persistence/projection fixtures, not real-model execution.
- The prior Mac change forbids estimating persisted conversation context from a paged UI transcript and removes value-based rejection of zero authoritative usage. Its regression run still has one failure: an imported, never-run conversation exposes zero window/prompt budget. Explicit unmeasured-state presentation and client revision-aware replacement remain open; do not restore partial-history estimates simply to satisfy the old positive-budget assertion.

## 2026-09-05: unmeasured context presentation

- Mac now treats absent context snapshots or non-positive window/prompt budgets as unavailable for usage display. Composer and inspector share `contextUsageText`, which returns the localized `Context not measured` rather than presenting a fabricated zero percentage. A valid budget with zero used tokens remains available.
- The inspector no longer equates `estimated` with an in-progress calculation. Available estimates use the same determinate fraction as measured usage; missing measurements do not produce a permanent spinner. Added the new label in all nine existing Mac catalog languages.
- Updated conversation-switch and authoritative-zero regression assertions. `MacChatWorkspaceBehaviorTests` (130) and `MacLargeConversationPerformanceTests` (2) passed after compilation. Log: `/tmp/nola-mac-unmeasured-context-tests.log`.
- This resolves the previously recorded imported-conversation test failure without restoring UI-window token estimation. Availability is currently derived from existing budget fields; an explicit cross-platform measurement-state contract, query failures distinct from never-measured state, and revision-aware snapshot replacement remain unimplemented. No new inspector screenshot or live LLM session was captured in this run.

## 2026-09-05: context measurement revision is not event order

- Removed the Mac conversation reducer's secondary `measuredAtRevision` comparison for SetContextUsage. The validated Application event sequence orders committed mutations; a measurement revision can restart across execution sources and must not also advance the Application projection revision.
- Added a reducer regression where the next Application event replaces measurement revision 900 with 2 and zero usage under a different valid window. Duplicate event sequence remains rejected. This test covers reduction, not workspace snapshot publication or a live Host switch.
- Post-run context refresh now uses `conversationContextUsage` instead of hydrating the complete canonical session projection to retrieve a few statistics. No full transcript is required by that refresh.
- Mac compilation and tests passed: 130 MacChatWorkspaceBehaviorTests plus 6 MacCanonicalSessionProjectionTests. Log: `/tmp/nola-mac-context-event-order-tests.log`.
- Cross-platform search found the same measurement-revision comparison in `core/nola-kernel/src/projection_reducer.rs`; it remains to be changed with Kernel regression coverage. Workspace-level estimated-versus-provider precedence and stale query response rejection also remain open. Do not treat the Mac-only reducer change as complete cross-platform context convergence.

## 2026-09-05: Kernel projection ordering parity

- Kernel SetContextUsage now follows validated contiguous Application event order rather than comparing execution-source measurement revisions. Added coverage for revision 900 -> 2, zero usage, duplicate delivery, lower sequence, and a sequence gap; rejected events leave projection/cursor unchanged.
- Kernel UpsertMessage and ReplaceSurface now accept equal-revision sibling events with distinct sequential event identities, matching Mac transaction semantics. Strictly older revisions still remain stale.
- `cargo test -p nola_kernel`: 6 tests passed. Evidence: `/tmp/nola-kernel-event-order-tests.log`.
- This establishes reducer contract parity, not all-platform runtime adoption. A bounded search of cli/src, core/nola-application/src, Android app sources, and Nola Swift sources found no direct ConversationProjectionReducer reference. Indirect FFI/protocol integration must be traced before claiming this shared implementation governs those clients. Mac still has its own native projection reducer.
