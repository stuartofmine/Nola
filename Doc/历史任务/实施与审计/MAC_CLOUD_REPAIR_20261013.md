> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Mac and cloud repair checklist

## Scope

Repair the latest Mac conversation and the failed cloud checks together. Do not
change user databases, credentials, permissions, or the signed Debug app. Finish
implementation and review before consolidated testing. No compatibility layer or
task-specific routing heuristics.

## Confirmed defects and acceptance

- [x] Xcode Cloud: prepare Rust before archive; smoke-test bootstrap without network installation.
- [x] Tool discovery: keep Host metadata outside model arguments; exercise actual dispatch validation.
- [x] Result persistence: verify the new result contract against frozen request references, not obsolete model operations.
- [x] Result references: report the offending field and expected reference namespace.
- [x] External executors: admit Codex without requiring an HTTP model endpoint; preserve frozen execution identity.
- [x] Failed Host transition: remove the rejected result transaction; preserve unknown execution state on transport failure rather than inventing a terminal result.
- [x] Recovery and skills: do not demand unavailable control tools or repeat Host configuration defects.
- [x] Ordinary replies: remove conflicting instructions that encourage unnecessary result-control calls.
- [x] Subscription timing: do not repeatedly subscribe to a conversation not yet admitted.
- [x] E2E CI: current Run fixtures, provider metrics function, immutable evidence hashes.
- [x] Localization CI: recognize .strings tables; repair missing translations without relaxing coverage.
- [x] Consolidated validation, including negative paths; explicitly distinguish local tests from cloud/device verification.

## Evidence

Latest Mac conversation: `03446AFD-D6E9-48A8-99A4-E3B70BC395BC`.
Failed run: `0DE4374D-FB60-478B-862A-34D1BD9287FF`.
Archive Build 30 stops before Rust compilation: `Rust toolchain not found in PATH`.
Local OSS checks before repair: 37 failed / 52 passed in the E2E job; localization
diagnostics: iOS 427, Android 349, Web 370 (diagnostics, not distinct defects).

## Implementation boundaries

- Cloud `ci_post_clone.sh` installs rustup only if absent, resolves the repository
  toolchain and Apple targets, and exposes Cargo to the archive phase. Cargo's
  dependency/cache writes are allowed by the Core target build configuration.
- Discovery takes model arguments and trusted Host catalog as separate inputs.
  Model-authored internal metadata remains a schema error even after approval.
- Model result references are frozen with each outbound request. Store verifies
  the request digest, owner, input frontier, scope, evidence and original control
  call before committing assessments. Maintenance retains those source requests.
- External executors bind backend/model/provider, not an invented HTTP endpoint.
  Mac cold recovery uses that frozen identity; permissions still come from the
  original launch descriptor and the executor session from the Core checkpoint.
- Skill completion guidance follows the actual exposed tools. Ordinary text
  answers do not require a task-result tool. Missing evidence and blocker
  references identify their separate namespaces in recoverable errors.
- New conversation drafts wait for Store admission before subscribing; genuine
  subscription failures use cancellable bounded backoff and deduplicated logs.
- Evidence receipts hash immutable copies of observations; validation rejects
  post-recording changes. Locale coverage remains strict across all three clients.

The signed Debug app, user conversation database and model credentials are not
modified by this repair. Existing failed history is not rewritten as success.
Cloud archive success must be confirmed by a new Cloud run, not inferred from a
local toolchain or bootstrap mock.

## Additional defects found by real execution

- Codex uses an owned `app-server --listen stdio://` process, with a bounded
  initialization deadline and bounded shutdown. It does not restart or depend
  on the developer's shared daemon. Only a completed external turn is success;
  interrupted, failed and malformed terminal events remain failures.
- An unspecified external model remains absent, rather than becoming an empty
  model name. The original external session identity is retained for follow-ups.
- Store accepts a delegated completion as a result source only after validating
  the original request digest and matching run, request, session, turn, attempt
  and delegated invocation identities. Progress events cannot seal a result.
- Replaying an already committed Host response returns its original transition
  receipt. It does not require a still-active Run after that Run was sealed.
- Ordinary streamed answers bind their result to canonical TextPart IDs, in
  projection order, rather than provider aggregate message IDs. Core projects
  the pending items before deciding the result; the existing terminal boundary
  still closes the parts and publishes the transition atomically.
- Maintenance instructions do not create user requirements. Maintenance no-op
  admission reads the actual `pending_host_effects` checkpoint field. No user
  result validation is bypassed to accommodate internal maintenance.

## Consolidated validation (2026-10-14)

- Python evidence/audit/localization/bootstrap suites: 99 passed. Bootstrap shell
  syntax and all 3 bootstrap mock tests also passed independently.
- Strict iOS, Android, Web localization audits passed; Android
  `:app:processDebugResources` succeeded.
- Rust Core: 518 library and 6 prompt contract tests passed. Host protocol: 29
  passed. Store: 409 passed, 1 ignored. Application nonterminal replay: 9 passed,
  including compact-result persistence, reopen/pruning and forged-source rejection.
- After the additional real-execution fixes, Core 518 and Store 409 were rerun
  successfully (Store still has 1 ignored test). Final focused coverage: turn
  transactions 20, delegated delivery 6, compact results 2, port/historical
  projections 7, prompt contracts 6, skill/goal lifecycle 1; all passed.
- External executor protocol tests: 36 passed, including malformed and failed
  terminal handling. These are separate from real Codex end-to-end acceptance.
- Final real Mac Codex acceptance: 1 test passed (29.610 seconds), executing
  two turns, verifying real tool events and file content, canonical successful
  seals and continuation of the same persisted external session. The test reads
  canonical message and delegated invocation identities instead of obsolete
  placeholder UUIDs or a retired side-channel status event. Evidence:
  `/tmp/nola-codex-mac-e2e.9K0pqm/result.xcresult`.
- CLI focused suites: tool discovery 29, model invocation 5, Host invocation 3,
  concise tool descriptions 1 passed. Fresh CLI binary built successfully.
- Mac Test configuration: 192 passed, 1 skipped, 0 failed in workspace behavior
  and resume authorization suites. Tests now obtain output-admission fixtures from
  Application instead of encoding an obsolete version by hand.
- LAN model `192.168.31.18:8081`: real CLI two-round recovery passed (2 tool calls
  and results initially; recovered round needed no duplicate file reads).
- Real Mac rendered-chat test passed, including strict comparison of the
  production result with the persisted terminal seal (`action_completed`). Both
  real UI exporters now share the same canonical transcript evidence contract.
- Final rerun with the new process UI and all Core fixes also passed against
  `192.168.31.18:8081`: 1 passed, 0 failed/skipped, one canonical successful
  `action_completed` seal. Evidence: `/tmp/nola-mac-real-memory.3mLYCt`.
- Local unsigned generic-iOS Archive succeeded, including Rust and Swift
  compilation. This does not validate Xcode Cloud installation or signing;
  a new Cloud run is still required after these changes are pushed.

Local diagnostic artifacts (not repository fixtures):
`/tmp/nola-mac-real-memory.n9Z02J`,
`/tmp/nola-cloud-archive.ZFs0Wh/archive.log`.

## Prompt and tool-description reduction

A separate agent reviewed the Claude Code prompts under the reference materials.
The implementation removes repetition while preserving authorization, evidence,
recovery, tool-schema and cache-prefix boundaries. It deliberately favors clear
concise instructions over forced classical Chinese.

Core policy text: 23,032 to 14,949 UTF-8 bytes (35.1% reduction).
Tool descriptions: 37,098 to 33,253 UTF-8 bytes (10.4% reduction).
These are text-size measurements, not measured token or latency improvements.

## Follow-up: assistant process presentation

User-observed defects: tools accumulate below prose, reasoning appears as
disconnected cards, and a finished turn has no unified process disclosure.
The current journal display promotes all reasoning Parts ahead of other Parts,
destroying the reasoning/tool chronology while preserving it in storage.

The Mac reply contains one process disclosure followed by public content.
Inside the process, typed reasoning, steps, tools and diagnostics retain their
canonical order and stable Part identities. Adjacent text grouping is determined
from the original sequence, so moving the process presentation cannot join text
across a real tool boundary. Public progress text is retained, not silently
reclassified as private reasoning or discarded.

Running turns show live progress. Canonical terminal state, not the end of an
individual reasoning Part, controls automatic process collapse. Explicit user
disclosure choices survive updates. Failures and pending permissions remain
discoverable. Detail bodies retain bounded journal reads and original metadata;
grouping introduces neither another model call nor a rewritten execution log.

Design direction: a quiet, continuous process rail belonging to Nola's local
execution record, not separate ornamental thinking cards. Use semantic macOS
content surfaces and primary/secondary labels, 13pt primary process rows,
28pt minimum row hit regions, and one clear disclosure hierarchy. Regular and
compact widths retain the same vertical structure; headers may wrap. Do not
animate every streamed update or move the user's scroll position on collapse.
Respect reduced motion and expose status independently of color.

References read: apple-design HIG `layout.md` (Visual hierarchy),
`disclosure-controls.md` (Disclosure triangles), `accessibility.md`, `color.md`,
`typography.md`, `designing-for-macos.md`, `progress-indicators.md`,
`generative-ai.md`, and `motion.md`.
Codex's [official App Server contract](https://learn.chatgpt.com/docs/app-server)
separates stable items and item completion from turn completion. The local
`参考资料/codex-main/codex-rs/tui/src/chatwidget/streaming.rs` also collects
reasoning summary sections before committing their transcript block. These
support lifecycle and grouping decisions, not a claim that Codex's closed desktop
UI uses this exact layout.

Process UI acceptance: the 46 other journal tests passed; the final interaction
test passed separately after correcting its locale expectation. It uses a real
Application journal with reasoning/tool/reasoning/tool/text, not hand-authored
display metadata. Actual accessibility-button presses expand, collapse and
reopen the process; stable Part IDs and terminal collapse are checked. Six
rendered screenshots cover light/dark, 380pt width, reopened details, and a
failed tool followed by recovery with a German header. The recovered turn keeps
its completed status and a separate historical-failure warning. No screenshots
or test databases are committed to the repository.

## Follow-up: disconnected recovery must not occupy the workspace

The October 14 01:20 production log identifies a missing `recent_activity`
field in the unfinished conversation checkpoint. This bounded prompt advisory,
along with its mutation streak counters, now has an empty/default decode value.
Required execution control fields remain required; no user database is rewritten
and no unfinished Run is declared terminal.

Mac previously treated even a disconnected recovery owner as global execution
capacity. Local disconnected owners are now parked by conversation when another
conversation is selected. Parking retains the exact Run/message identity,
cancellation intent, workspace selection and error; it releases only presentation
ownership and advances the callback generation. Returning restores the same owner
under a fresh generation and requires fresh recovery inspection. Live execution,
resume startup and cancellation delivery are not detached this way. This change
does not claim concurrent execution of multiple live Mac transports.

Interaction follows apple-design `feedback.md` (Best practices: feedback near
the item it describes) and `loading.md` (Best practices: let people do other
things while waiting). Recovery warnings belong to their conversation, not an
unrelated draft. The existing controls and semantic colors are retained.

Recovery regression also exposed an admission/observation mismatch: accepted
local submissions could remain marked as draft conversations when no optional
session-status event arrived. Durable submission acceptance now marks the local
conversation queryable, allowing the existing observer to see late terminal
commits. Transport EOF and cancellation delivery still cannot manufacture a seal.

Validation: 43 Mac tests passed, 0 failed/skipped (local terminal recovery,
submission preparation and coordinator suites), plus the Rust missing-advisory
decode regression. The new cross-conversation tests cover actual Application
admission of a second Run and retention of the first Run's cancellation intent.
Final LAN-model Mac rendering acceptance passed with one `action_completed`
canonical seal; artifacts: `/tmp/nola-mac-real-memory.20Cduu`. Tests used the
isolated Test product and databases, not the signed Debug app or user history.

## Search boundaries and recorded cold recovery (2026-10-14)

The 01:38 search used only `web_search`/`web_fetch`, but the search adapter
silently escalated insufficient HTTP results to a Chrome-extension search.
Remove that adapter and the fallback API entirely: public HTTP search returns
partial evidence or an error; browser interaction is a separate model-selected
tool with its own permission contract, never a query-keyword routing rule.

The same startup's cancellation receipt was rejected with `committed Host effect
changed immutable payload or response policy`. Replaying a checkpoint tail
reconstructed model prompts using the current renderer, then compared those
new bytes with an older committed request. Fix the data source, not the integrity
check: Store includes pending model requests from its digest-verified outbox in
the same recovery snapshot. After replay, Core materializes those original
messages/tools/settings, retaining exact model/output-part identities, response
policy, effect ownership, cursor and cancellation state. Regular rebind remains
hash-strict; corrupted outbox records still fail. This path neither dispatches
effects nor grants permissions. Deferred hot-rollback replay must not reinsert
requests it has already settled.

Regression coverage includes changed-renderer cold cancellation through terminal
confirmation, foreign model/Run/output-part rejection, and empty/partial HTTP
search results. `recorded_recovery_e2e` is opt-in: it takes a read-only SQLite
backup of a supplied private failure database, then exercises recovery and
cancellation on a temporary copy only. No production history is rewritten and
no original task tools or model calls are executed by that regression.

Validation: Core 520 passed; Application replay/cancellation/port suites 26
passed; Store 409 passed/1 ignored; CLI HTTP search 24 passed. The recorded
Application test reclaimed the original model request with unchanged payload,
then committed cancellation and reopened it idempotently. The recorded CLI
cancel-only test reached a real `Cancelled` seal on another private copy;
all 10 cancel-only tests passed with `RUST_MIN_STACK=16777216`, matching the
local pipeline baseline. The initial default-stack cancellation test process
overflowed; the rerun's larger test stack is not a production fix. The large
CLI test binary also emits an ld compact-unwind-size warning. Neither issue
is reported as fixed by this change. Original user data and signed Mac Debug
products were not replaced; rebuild/restart the Mac app to load the new Rust
service and agent-host implementation.

## Follow-up retrieval and media admission (2026-10-14 evening)

The 22:07 retrieval follow-up ended with HTTP 400 after downloading three HTML
documents under PNG filenames. The persisted last model request labels all three
artifacts `image/png`. This is a confirmed input-contract defect; the generic
persisted provider error alone does not prove the provider's exact rejection
reason. The run also attempted two tools absent from its successor-turn schema.

Model media admission now checks the byte signature against the declared type,
for both explicit user attachments and digest-bound tool snapshots. Mismatches
become text-only unavailable-media evidence, retaining the native tool result
and call ordering so the model can recover instead of sending an invalid image.
This is format-signature admission, not full image/PDF decoding. No task-specific
URL rewrite or competition-specific handling is introduced. Known structured
provider media error codes receive an actionable failure category without
copying provider response bodies or echoed private data into user diagnostics.

Successor requests re-project up to eight recently successful tool identities
from typed history using the current capability catalog. Old schemas and grants
are never restored; unavailable tools stay unavailable, and final synthesis and
restricted recovery do not gain these tools. Prompt wording is not inspected.

Follow-up admission: Host checks shell syntax before requesting approval, using
the execution boundary's existing parser without accessing the requested files.
`tool_preflight_failed` settles the pending tool as failed/not-started, removes
the pending permission, and resumes the model for argument correction. It never
creates an authorization/denial receipt or an execution effect. Unrestricted
resource mode retains its existing behavior; full resource checks still happen
at execution. This is syntax admission, not a guarantee that a command succeeds.

The Mac permission card now explains that remembering an approval covers only
the requested operation/resource in this conversation. Following apple-design's
permission clarity guidance, this scope is visible before confirmation. Failed
or cancelled replies with completed tool output point to retained activity
details; they do not manufacture a synthesized answer or treat reasoning as a
tool result. Live Mac acceptance is still required. Production conversation data
and the signed Debug app remain untouched.

Validation: `RUST_MIN_STACK=16777216 cargo test -p nola_core --lib --quiet`
passed 531 tests; the corresponding `nola_cli --lib` suite passed 1,850 tests
with five ignored. The existing large-binary Darwin compact-unwind linker
warning remains. No live Mac/provider acceptance is claimed for these changes.

Follow-up validation: Core unit tests 531 passed; Host contract tests 149 passed,
including preflight failure with no execution start, no permission receipt, no
grant change, model continuation, and checkpoint restoration. Mac
`MacChatWorkspaceBehaviorTests` in the isolated `Test` configuration passed 190
tests with one skipped. CLI full-suite run: 1,851 passed, five ignored, one
5,000-conversation listing performance assertion failed at 1.244 seconds while
the Mac build was also running. That assertion passed alone at 486 ms; the
initial full run is not described as green. No performance threshold was relaxed.
All three new Chat strings have eight translations. The existing broad Mac
localization audit still fails (5,768 findings; the smaller critical-copy audit
reports 27 missing translations in untouched Localizable entries).
Both recorded-recovery regressions passed against separate temporary backups of
conversation `03446AFD-D6E9-48A8-99A4-E3B70BC395BC`: Application restored/reclaimed
the exact pending model payload and durably recorded cancellation; CLI cancel-only
recovery reached a terminal Cancelled result without running the model or task
tools. These tests do not cancel the original conversation or constitute a live
Mac-window/provider acceptance test.

## October 14 follow-up: capability truth and result presentation

The recorded memory-save request had long-term memory explicitly disabled. Its
terminal result was `incomplete`, even though message delivery and four tool
activities had finished. These are distinct states, not permission failures.

- Core publishes the effective memory flag and disablement reason as runtime
  facts. Skill admission rejects explicitly disabled builtin dependencies before
  loading their workflow. Neither mechanism enables memory or grants access.
- Deferred discovery indexes capability identity, title, and top-level purpose,
  not nested parameter names such as `max_memory_bytes`. Full callable schemas
  remain unchanged.
- Mac process summaries read the durable run outcome. Completed delivery alone
  is labelled ended while the result is loading; incomplete/blocked results
  remain distinct from successful answers/actions.
- Autolinking excludes code spans and existing Markdown links. Artifact media
  claims are checked against file signatures; HTML named `.png` is not an image.
- General evidence guidance requires exact source URLs and separate attribution
  of user statements, retrieved facts, and inference. Fetch failures alone do
  not establish anti-bot protection or justify claiming information is private.

These changes do not repair historical model prose, enable disabled memory, or
replay failed writes automatically. Memory remains explicitly configurable in
the existing conversation inspector. Live-provider acceptance remains separate
from deterministic regression tests.

Follow-up validation: Core 532 unit tests and 149 Host contract tests passed;
CLI 1,853 passed, five ignored, zero failed. The Mac `Test` configuration passed
277 tests with one skipped across `MacMarkdownParserTests`,
`MacAssistantJournalTransportTests`, and `MacChatWorkspaceBehaviorTests`.
`git diff --check` passed. No signed Debug app replacement or production
conversation mutation was performed. Live-model and interactive window
acceptance have not yet been performed for this batch.

## October 15: stable Skill chrome and honest research validation

- Skill title/status/statistics and its disclosure are now separate equatable
  views. They consume visible metadata, not the journal cursor or streamed body.
  Real status/count changes still update; unrelated text does not animate the
  static chrome. Body readers retain their current cursor and authorization
  provenance, rather than freezing stale content references to prevent redraws.
- Research validation reports `structure_validated`, `distinct_hosts`, and
  `evidence_verified: false`. It does not certify claims or independence from
  domain counts. Single-repository original papers remain legitimate inputs.
  The Skill requires a query/claim/source ledger and separate scope, freshness,
  independence and claim-support audits. Unrestricted research starts without
  a site filter; subsequent restrictions must address an explicit evidence gap.
- The research SKILL.md was previously ignored by the blanket Markdown rule;
  it now has an explicit repository exception.
- Unified logging identified the reported NSXPCDecoder warnings in Nola PID
  29510 as `IMKXPCServerProxy` connections to PID 1833, Apple's
  `/System/Library/Input Methods/SCIM.app/Contents/PlugIns/SCIM_Extension.appex/Contents/MacOS/SCIM_Extension`.
  This identifies the input-method channel, not a Nola model/storage failure.
  Composer updates already avoid replacing marked text and only request focus
  when not first responder. No private-framework patch, input-method shutdown,
  global log suppression, or relaxed decode allowlist was introduced. The
  system warning itself remains unresolved.

Validation: the structure validator's three regression tests passed; Mac
`MacAssistantJournalTransportTests` passed all 49 tests in `Test`, including
unchanged Skill chrome versus actual status/count changes. `git diff --check`
passed. No live-provider research rerun or frame-by-frame interactive acceptance
of the changed build is claimed. The signed Debug application was not replaced.

### Follow-up acceptance

- Added a mounted `NSHostingView` regression that opens the actual Skill
  disclosure through accessibility, updates sibling streamed text eight times,
  and checks that its expanded content remains exposed. The full journal suite
  now passes 50 tests, zero skipped, in the isolated `Test` configuration.
  This verifies disclosure continuity, not frame-by-frame absence of every
  visual flash in the full conversation window.
- The live Skill harness now retains successful transcripts as well as failed
  ones when workspaces are kept. Its 19 contract tests pass; the research
  structure validator's three tests also pass.
- Rebuilt the CLI through its workspace build script and used the configured
  LAN model (`192.168.31.18:8081`, `qwen3.8-flash-next-iq3_s`) with isolated test
  state. A 180-second research run timed out: 12 searches were started, the
  first unrestricted, with subsequent targeted official-site queries. The
  first 11 completed searches parsed 99 pages. This demonstrates excessive
  research expansion despite the Skill's suggested eight-query budget; it
  is not a successful research acceptance. Raw evidence is retained under
  temporary workspace `nola-live-skills-bni19iaj`.
- A second run with the normal 600-second ceiling finished after about 470
  seconds with canonical outcome `incomplete`, not success. It wrote a
  27,968-byte report and ran the bundled structure validator. Its 16 actual
  searches were misleadingly grouped as nine in the report, and the same
  Skill was activated twice. The report correctly distinguished structural
  validation from evidence verification, but that is insufficient acceptance.
  Evidence remains in temporary workspace `nola-live-skills-zxbr1oiw`.
- The run exposed two harness defects, now covered by regressions:
  `requirements_update` records outcome evidence rather than modifying the
  validated artifact; and `run.completed` must have a successful task outcome
  (`answered` or `action_completed`). `incomplete`, `blocked`, missing outcomes
  and cancellations must not pass merely because execution stopped. Replaying
  the second trace now reports its actual incomplete outcome.

Remaining: research-loop convergence and trace-derived reporting counts have
not been fixed by these UI/validator changes. Full-window frame-level visual
acceptance and the system input-method warning also remain outstanding.
