> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Mac conversation architecture acceptance

## 2026-09-21 completion review and validation coverage

Review can now run with outstanding engineering obligations, without authorizing
successful publication. An execute report reopens normal execution; an accept
report with outstanding obligations returns to completion recovery. Recovery is
bounded and survives checkpoint restore. Candidate review stays tool-free even
with pending Skill completion. Validation coverage is directional, and a later
mutation without a new validation requirement preserves earlier obligations.
Shell verification requires nonempty validation_targets before execution.

Verified: Core 374 unit tests, 101 Host contract tests (including cold-restored
pending-work review), Kernel 111 unit tests, CLI 1184 tests passed / 2 ignored;
debug binary built successfully. Initial new Host fixture incorrectly omitted
the canonical completed message for a hidden stream; the fixture was corrected
to include the completed message and the full Host suite passed.

Real-provider focused trial `nola-commit-followup-yi444flk` created `789d06d`
with all three expected files and left the two exclusions untracked. It recovered
from missing validation_targets by supplying exact file paths, but did not finish
within 240 seconds. This is NOT an end-to-end pass. Unscoped mutations still
produce unknown (`*`) obligations that exact-file validation cannot discharge;
generic semantic scope/postcondition reconciliation remains incomplete. The trial
preceded the final pending-Skill private-review projection adjustment; no full
three-turn acceptance is claimed. Production history is unchanged.

## 2026-09-20 Git object admission and nonempty completion review

Production run `e2fa1f20-7410-4799-b025-aa4af2417ba3` created commit `775864bc`,
including the 246,584,488-byte `cli/scripts/nola` binary. The preceding review
had accepted a repository-wide claim from an inventory preview with empty
review items. During commit recovery, an invented SOP run identifier also
prevented `request_user_input` from reaching the user. This change does not
rewrite or remove that existing commit.

Reference comparison: local Claude's
`tool-description-bash-git-commit-and-pr-creation-instructions.md` prefers
explicit relevant staging and inspecting the staged diff; local Codex's
`prompts/src/review_request.rs` resolves a concrete review target and
`protocol/src/review_format.rs` keeps explanation/findings separate from
execution. Neither should be interpreted as a generic proof that a task has
been performed correctly.

Nola now separates pre-execution Git content admission, session-level questions,
and post-execution candidate review. Shell admission checks explicit paths before
staging and indexed object bytes before commit, including pre-staged artifacts
whose worktree contents have subsequently changed. Broad/compound staging and
automatic commit staging are rejected. The rule applies after permission too;
an approval does not turn an unsupported operation into a supported one.
The exact limitations of this direct-command contract are documented in the
module contracts; it is not an arbitrary-program security sandbox.

Empty or unresolved accept reports are protocol errors, not publication permits.
Questions no longer depend on model-supplied SOP lineage; normal executing tools
retain strict lineage validation. Tests include the production-style invalid
question lineage, empty reviews, source files previously misclassified as
artifacts, staged-object substitution, compiled headers and oversized blobs.

Local validation: Core 373 unit tests and 101 Host contract tests, Kernel 110
unit tests plus one integration test, CLI 1183 tests (2 ignored) passed.
The real-provider harness now additionally asserts that relevant untracked
source enters the commit while a generated executable and local environment
file stay untouched and outside the index. A clean worktree is not its success
criterion because intentional exclusions must remain uncommitted.

The first content-admission real-provider trial (`nola-commit-followup-0nupi28i`)
created `7d6d327` with the three intended files, left both excluded files untouched,
but failed to finish: post-commit status/log calls were declared Investigate and
did not discharge validation debt. Core still exposed tools, but three text-only
answers ignored its soft verification instruction. This trial is a failure, not
a successful end-to-end result. Validation-completion recovery now requests
RequiredTool with a pure-Validate projection and checks the effect at dispatch,
including after cold restore. It does not restrict normal subsequent editing
after the first mutation, and a real failing validator can reopen correction.

The permission PTY initially failed because its model fixture still looked for
the candidate in a live system message. The fixture now decodes the isolated
review evidence envelope; its extraction regression and permission-return test
both passed. This fixes test protocol fidelity, not a production permission bypass.

Final real-provider acceptance remains failed, not waived: the full three-turn
trial `nola-commit-followup-csuw6_zf` timed out in its first audit after cwd/tool
protocol errors. The focused root-directory commit trial
`nola-commit-followup-um92que4` created `778e5ee` with only the two tracked text
files, omitting the requested new source. It kept both excluded files outside
the index, but did not discharge validation debt. Its final provider request
actually carried `toolChoice: required` and the narrowed validation tools;
`openrouter/free` nevertheless returned text with no tool calls. Core failed
the run instead of publishing success. RequiredTool is a provider request, not
a guarantee of provider compliance or semantic task completeness. Both failed
fixtures are preserved outside the production repository; neither is evidence
that the complete user workflow now passes. Debug binary compilation succeeded.

## Reviewed-candidate publication and explicit task outcomes

Run `c32b3c85-a97d-48e6-b974-2e6142db049d` only inspected status/history for
"提交下". Its audit recorded unfinished work but a free final-generation step
replaced clarification with unexecuted shell commands. Exact rejected-label
matching then allowed Turn completion. This supersedes the earlier final rewrite
and substring-filter design, rather than extending that filter.

Accepted candidates are now published verbatim. Revisions remain private and
must be reviewed again, with a separate two-revision limit. Reports require an
explicit outcome: answered, action_completed, needs_input, blocked or incomplete.
Action completion requires current-turn Host-confirmed successful mutation call
IDs; skipped, failed, read-only and invented calls cannot satisfy that check.
This is invocation evidence, not a claim that generic runtime code can prove all
semantic postconditions. Clarification preserves the exact reviewed question and
exports needs_input through the snapshot/event/CLI reason. Blockers publish their
reviewed explanation but terminate as failed, not successful action completion.

The test harness `scripts/test_real_cli_commit_followup.py` runs review, repository
scope correction and commit follow-ups against a disposable Git repository. It
checks unchanged file bytes during review, a new commit with the expected parent,
committed contents and a clean tree. It cannot pass on printed commands or a
model judge alone. No test commits the production Nola worktree.

Review requests have a dedicated system protocol and one serialized evidence
message. Execution policies, conversation roles, tool outputs and the candidate
are data in that message, not live instructions to continue the original task.
Token accounting uses this actual review projection. The first real-provider
trial exposed the previous mixed-context defect: three audit attempts returned
ordinary review prose and the turn failed with an invalid-report error. This
failure is retained as evidence, not counted as a successful end-to-end run.
The next trial exposed a Host projection defect: Skill-catalog relocation
matched a marker inside serialized user evidence and deleted the whole evidence
message. Structured review now bypasses execution-reminder rewriting, and
catalog relocation never removes user/assistant messages by marker content.
The real-provider harness also checks that tool-free requests retain user
evidence and that fully specified fixtures do not exit as needs_input.

Local validation for this change: Core library 371 tests, Core Host contracts
100 tests, Kernel 110 unit tests plus its integration test, CLI 1178 tests
(2 ignored), and targeted catalog/scope regressions passed. The mobile-height
Chinese-draft/footer PTY and permission-panel PTY regressions passed. These
results do not establish Mac visual acceptance or production-scale audit quality.

Final real-provider trial (`nola-commit-followup-jupxvhii`, configured
`openrouter/free`) passed turn 1 with a two-message review request, actual diff
evidence and canonical `answered` outcome. Turn 2 never entered application
startup: its event/provider logs stayed empty and process sampling remained at
`_dyld_start` for over three minutes. A separate `nola --version` reproduced the
startup stall; code-signature verification passed while `syspolicyd` consumed
roughly three CPU cores. The two owned test processes were terminated without
changing system security settings. This is a blocked three-turn acceptance, NOT
a passed commit test; the commit postcondition remains unverified in this trial.

## 2026-09-20 completion admission before tool use

Production run `7e703fab-3137-4f23-bd58-25654c8ba0f0` returned an input-analysis
fragment and unrelated timestamp for a repository audit, made zero tool calls,
and was incorrectly completed. Provider provenance identifies `openrouter/free`
routing to `nvidia/nemotron-3.5-lightning:free`. The earlier tool-free recovery
fix did not cover this first-response path.

For enabled Host capabilities requiring evidence review, the first text candidate
is now private even before tool use and undergoes task/evidence admission. The
typed audit decision `execute` returns unfinished work to normal execution with
ordinary schemas and permission checks; it is not itself a tool invocation or
permission grant. Two returns are allowed, the count survives checkpoint restore,
and closed budgets cannot reopen execution. Greetings/context-supported answers
can pass without any tool. This introduces review requests for direct answers in
evidence-sensitive sessions; it does not claim zero additional latency.

Kernel regression covers first-candidate capture, bounded return and closed-budget
failure. Host regression covers a premature input-analysis candidate, private
audit, restored tools, subsequent evidence and valid completion. Chinese/English
greetings cover zero-tool completion. Protocol fixtures retain permission/retry
and canonical-history assertions while modeling the additional audit phases.

Validation: Kernel 110 tests, Core host contracts 98 tests, CLI 1177 tests passed
(2 ignored); debug binary rebuilt. Initial CLI fixture failures were retained in
`/tmp/nola-admission-cli.log` and corrected by adding protocol responses, not by
removing permission/retry checks. Final CLI log: `/tmp/nola-admission-cli-verified.log`.
Original prompt `、审计下当前git目录未提交变更` ran against the explicitly selected
configured `openrouter/free` with provider environment overrides removed:
`nola-real-dirty-audit-j30kkxye` completed, 27 tool parts, unchanged fixture and
passing independent judge. One invalid audit response and a provider transport
retry recovered. Manual inspection finds the resulting report primarily a change
summary and unverified-risk list; it is not proof of exhaustive defect analysis
or universal answer quality. This acceptance specifically covers execution and
completion admission, not the phone terminal's visual behavior.

## 2026-09-20 CLI tool-free phase correction

Production CLI run `f62825fc-eca1-4ba6-aa93-984777f1478b` entered a private
`structured_audit` request with zero tools, then received an unsolicited `bash`
call. Dispatch rejection created an execution recovery obligation although later
audit/final requests still had zero tools. It eventually failed with
`recovery_required=true`; workspace access had already been approved.

Completion policy now uses the actual emitted tool projection to reject such
proposals before dispatch. It preserves the current private candidate/report and
retries the response contract, with a bounded protocol failure for repetition.
It does not reopen tools or create a permission/recovery obligation. Existing
budget exhaustion and denied-permission rules remain authoritative.

Local Claude prompt/tool references informed scoped evidence gathering and schema
discovery guidance, not prompt keyword routing. Workspace status now separates
staged/unstaged full diff references from metadata; deterministic diff scanning
is explicitly not semantic audit coverage.

Validation: Kernel 109 tests, Core host contracts 97 tests, prompt contracts 5
tests, CLI 1177 tests passed (2 ignored). The host test injects native tool calls
in both audit and final presentation and still reaches the valid final answer;
Kernel tests cover bounded repeated rejection. CLI tests check staged/unstaged
content, bounded metadata, complete stored tail and authorized range retrieval.
The debug CLI was rebuilt. The first CLI test run had two incorrectly migrated
assertions; they were corrected before the passing full rerun.

Isolated real-model run `nola-real-dirty-audit-kon4vf7g` completed with unchanged
fixture fingerprints and a passing independent judge. It used the currently
configured `deepseek-v4-flash`, not the production failure's `openrouter/free`.
It recovered from invalid byte offsets and completed candidate/audit/final phases.
Manual inspection still found inaccurate change-count prose despite the judge
passing; this is not evidence of exhaustive or universally correct semantic
review. Status metadata now exposes an explicit changed-entry count before the
potentially truncated inventory, with Git's untracked-directory grouping noted.

## Scope and design changes

The reported conversation was an audit of uncommitted changes. Its cancelled run
contained a model batch of 121 tool calls. The fixes do not infer authorization or
select tools from audit-related words in the user's prompt.

| Boundary | Previous failure | Current contract |
| --- | --- | --- |
| Viewport | Layout and delayed follow tasks competed with user scrolling | One intent controller; generation invalidates obsolete work; history uses message anchors |
| Process timeline | A repeated tool moved an old group to the newest position | Only adjacent calls group; 24-item pages; history cursor is an event ID |
| Model staging | Unbounded pending calls could become a wall of tool cards | Shared 32-call response capacity; entire oversized batch rejected before execution |
| Recovery | Capacity errors could resemble ordinary transport failures | Stable error code, no transparent fallback, at most two corrective replans |
| Evidence | Interleaving calls evaded consecutive-batch repetition checks | Successful result fingerprints identify unchanged observations across batches; mutation resets the observation epoch |
| Large output | Truncation implied reading every remaining page | Full evidence retained separately; targeted search/range reads; exhaustive coverage only when the task requires it |
| Markdown | A table parsed correctly but long cells still ellipsized | Cells retain natural vertical size; long-row height regression and real-answer visual inspection |
| Original text | A bounded 32 KiB page still caused expensive SwiftUI whole-page layout | Native selectable text storage inside a viewport; byte-range navigation remains canonical |

The Xcode Core synchronized group treats `target` as an opaque folder, preventing
Rust test artifacts from becoming a multi-gigabyte source-tree synchronization task.

## Original-scale harness acceptance

The five-function fixture below is historical smoke coverage, not acceptance for
the later large dirty-worktree failure. The current boundary contracts are in
`ARCHITECTURE_V2_MODULE_CONTRACTS.md` section 8.1. In particular, queued work is a
typed deferred outcome, incremental receipts do not duplicate full snapshots,
and bounded stream delivery preserves durable control-event acknowledgements.

The CLI dirty-worktree run `nola-real-dirty-audit-jjmpl44f` completed with 50 tool
parts and unchanged fixture files. The Mac run `nola-mac-dirty-audit-r3jdqj2e`
used the same isolated workspace and passed canonical text reconciliation,
read-only fingerprinting and a zero-tool follow-up. Both independent response
judges passed. These are process/grounding gates, not proof of exhaustive code
coverage or of the correctness of every model finding.

The Mac image from that run exposed excessive row spacing; its cancellation probe
completed before admission. Neither was accepted as final evidence. The subsequent
`nola-mac-dirty-audit-krtzucpz` run uses intrinsic Grid sizing and requires admission
before cancellation. It passed with 26 completed tools, no tool errors, unchanged
workspace fingerprints, canonical text reconciliation and a zero-tool follow-up.
An admitted Run reached `run.cancelled` 2.022 seconds after cancellation was sent.
The independent response judge passed. Its `audit.png` was visually inspected:
the seven-row native table wraps its long evidence cells without stretched row
spacing or raw Markdown syntax. The wide table is horizontally scrollable; the
component snapshot shows its initial horizontal viewport, not all columns at once.
Image SHA-256: `af72ae9d901d8d0da9f098f3ed34a09b6a0049b1e16fcb6b46e3e4a7948b03cf`.
Production conversation storage is never used as a mutable test fixture.

The complete Mac suite passed 625 tests (12 configured skips, zero failures).
The screenshot is component-level evidence, not an interactive scroll recording;
native regression tests cover scroll intent, stale navigation and late layout.
The final intrinsic-height test also passed with a real hosting window and a
nonempty-height assertion. Its light-background snapshot was visually inspected:
three rows are visible, the long path wraps, and row height remains compact.
The earlier transparent/blank test attachments are not visual acceptance.
Final test log: `/tmp/nola-mac-table-final-visible.log`; inspected attachment:
`/tmp/nola-mac-table-visible-attachments/3FA770DE-DDF8-4A59-8114-4131D559F067.png`.

PTY scenarios use the CLI's embedded Application Service, so both processes run
the same executable. A sibling standalone service binary is not their authority.

## Recovery and execution acceptance

The final recovery changes separate logical Run identity from physical delivery
ownership. Host observes process death; Application compares the exact attempt,
generation, owner and token inside the recovery transaction. A live or replaced
owner cannot be taken over. Started operations that require reconciliation are
not replayed. Recovery also performs the normal composer input handoff, including
messages entered while the interrupted Run is resuming.

Validation promotion is additive: outstanding validation gates completion but
does not silently remove previously exposed mutation tools. Task output limits
measure observed bytes independently of the bounded retained output. Parent
presentation heartbeats are throttled independently of worker lease renewal,
and durable boundary commands are admitted fairly alongside effect receipts.

Final unit/contract checks after these changes:

- CLI: 1,172 passed, two configured skips, zero failures.
- Application: 122 unit tests and six SQLite delivery integration tests passed.
- Core: 370 unit tests and 97 Host contract integration tests passed.

The final full PTY run completed 77 scenarios: 76 passed and one failed while
sampling a partially redrawn terminal frame. The Shell task scenario counted
historical `/help` text before checking visibility, which is not a redraw
barrier. It now waits for the draft to be visible within the same four-second
deadline, retaining the resize, width and subsequent-input assertions. Its
focused rerun passed. This is a full run plus a corrective focused rerun, not a
claim that a fresh full run passed all 77 scenarios. Logs are
`/tmp/nola-pty-final-complete.log` and `/tmp/nola-pty-shell-resize-final.log`.

The real-provider Mac acceptance above predates the final cold-process recovery
changes; it is evidence for rendering, read-only audit and admitted cancellation,
not a real-provider test of mutable cold recovery. Scripted-provider PTY tests
exercise actual CLI processes, durable storage, permissions, cancellation,
worker death, takeover and queued-input ordering in isolated fixtures.

## Subsequent CLI mobile-terminal correction

The user's 14:06 CLI screenshot invalidated the earlier broad terminal acceptance:
Chinese draft prefixes and footer rows remained at old viewport positions. The
renderer recorded only surface row count and cleared it relative to the new
height; it also painted the composer outside the transcript's active margins.
It now records terminal geometry, rebuilds resized visible history from a bounded
committed-text tail, and resets origin/scroll margins before painting one surface.
Drafts are never cached as history. Leading model newlines are suppressed only in
the presentation projection.

The new real-process PTY scenario sends a greeting, changes terminal height and
width four times, then types `审计下当前目录` one character at a time. It checks the
current screen for exactly one draft, one footer and one completed response.
It passed, as did seven existing resize/streaming/footer/table/task-panel cases.
The full CLI unit run passed 1,175 tests with two configured skips. These are
PTY/emulator checks, not a claim of direct verification inside the user's phone
terminal app. Logs: `/tmp/nola-mobile-composer-pty.log`,
`/tmp/nola-terminal-geometry-pty-regressions.log`, `/tmp/nola-terminal-final-unit.log`.

## Subsequent inline-stream and permission-pane correction

The next user session exposed a different layout defect: mutable assistant text
was part of the bottom composer frame, then moved to history when finalized.
Codex's local reference sources (`streaming/controller.rs`, `insert_history.rs`,
`bottom_pane/mod.rs`) separate stable history, its mutable tail, and interactive
bottom-pane ownership. Nola now anchors the mutable tail at the history insertion
point, so committing text changes ownership without changing its screen position.
Permission cards use the same renderer, replace the composer, and offer PgUp/PgDn
for details that exceed the viewport. They no longer save/restore a separate
cursor or clear the entire screen on resize. Authorization policy is unchanged.

A new real-process test pauses provider output after the first sentence, records
its visible row, releases completion and requires the same row and exactly one
copy of the answer. It passed. Ten related PTY regressions passed, covering
permission return, scope isolation, resize, streaming continuation and tables.
Logs: `/tmp/nola-inline-position-pty-2.log` and `/tmp/nola-inline-pty-check.log`.
Final CLI unit regression: 1,177 passed, two configured skips, zero failures.
The final permission-return, task-scoped permission and mobile-resize reruns also
passed. `cli/scripts/build.sh --debug` rebuilt `target/debug/nola`; an already
running CLI must be restarted to load it. Logs: `/tmp/nola-inline-final-unit.log`,
`/tmp/nola-inline-final-permission.log`, `/tmp/nola-inline-final-build.log`.

## Historical small-fixture acceptance

Verified on 2026-09-20:

- Core: 582 passed, no failures.
- Mac contracts: 516 passed, one configured skip, no failures.
- Host process contracts: five passed, including oversized-batch recovery and exhaustion.
- Real Mac approval/write/read-back: passed in an isolated workspace.
- Final real audit: two completed tool calls, five findings, one native table,
  zero follow-up tool calls, unchanged fixture files; persistence audit passed.
- Native original-reader keyboard pagination: 0.136 seconds in the final full run.

Final real-audit artifacts are preserved locally under
`/tmp/nola-mac-audit-acceptance.50aVeo/`. Its `audit.png` was inspected after the run:
five rows are visible, cell contents wrap without ellipses, and no Markdown pipe
syntax is shown. SHA-256:
`b07fd7319ba98e57cf2be1cd6ea51c903a1fedbcd596d4bd45d7969db9e34601`.
The application already open before this work was not terminated or relaunched;
the acceptance processes used isolated storage and exited after verification.

- `cargo test --manifest-path core/Cargo.toml --quiet`
- `cargo test --manifest-path cli/Cargo.toml --test desktop_host_canonical_identity_e2e --quiet`
- `bash scripts/test_mac_layers.sh contracts`
- `bash scripts/test_real_mac_permission_approval.sh`
- `bash scripts/test_real_mac_audit.sh`

The audit script copies an intentionally defective five-function fixture into a
fresh temporary workspace. It requires five named findings, a native table node,
bounded tool execution, a natural Chinese follow-up without new tool calls, and
unchanged source files. It preserves the isolated database, provider trace, result
receipt and `audit.png` for inspection. No production conversation is edited.

The first real audit passed protocol checks but its screenshot exposed truncated
table cells. After the layout fix, a second real audit and screenshot inspection
confirmed complete wrapped cell text. This distinction is intentional: parse
success and screenshot generation alone are not visual acceptance.

Scope of visual evidence: the saved image is an actual assistant component rendered
from a real provider response, not a full application interaction recording. Scroll
intent, late layout growth, SwiftUI attachment, history anchors and stale navigation
are additionally covered by native AppKit/SwiftUI regression tests. These tests do
not guarantee every future model answer is factually correct.
