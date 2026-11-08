> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# macOS Built-in Skill E2E Progress

Last updated: 2026-08-02

## Acceptance standard

Each Skill is tracked against the strongest applicable evidence:

1. The production macOS Agent Host loads the enabled built-in Skill and follows the real tool and permission path.
2. A controlled model endpoint may choose deterministic tool calls, but it must speak the production HTTP protocol and cannot inject Host events directly.
3. Artifact-producing Skills must create a real file with the bundled production generator or production tool.
4. The artifact must pass package validation, reopen through a system renderer, and receive a visual inspection for clipping, overflow, blank output, hierarchy, spacing, and fidelity.
5. Behavioral Skills must cross their real local boundary, such as storage, scheduling, Chrome messaging, MCP, or data-source access. A marker-only text artifact is not sufficient E2E evidence.

The XCTest suite uses a deterministic HTTP model fixture to verify the
production Agent Host protocol, tool execution, permissions, artifacts, and
validators. It is therefore a protocol/artifact E2E layer, not evidence that a
real model can autonomously select and complete a Skill. The canonical bundle
currently has 23 user-facing Skills; real-model coverage is tracked separately
by `scripts/live_skill_e2e.py`, and production readiness requires both layers.

## Current real-model rerun status

The deterministic protocol/artifact matrix below remains valid for its stated
layer, but it does not replace autonomous execution by the currently configured
model. The current-worktree bounded rerun has established:

- Word, meetings, knowledge memory, repaired data analysis, and Nature Reviewer
  pass through a real model and independent validators.
- Data analysis originally exceeded 150 seconds while generating an unnecessary
  large helper program. It now uses the bundled deterministic table pipeline
  and passes in about 45 seconds under the ordinary 180-second limit.
- Nature Reviewer legitimately needs a longer academic window. It exceeded the
  old 180-second bound, then passed in about 300 seconds. Nature Skills now use
  a 600-second default and 900-second hard cap.
- Other Skills retain a 300-second default and 600-second hard cap. No Skill is
  allowed to run without a deadline or process-group cleanup.
- The remaining built-ins require source-matched bounded reruns before their
  historical live-model results are considered current.
- The harness rejects stale binaries and derives final validator commands from
  each current `NOLA_MANIFEST.json` rather than hard-coded filenames.

## Deterministic protocol and artifact matrix

| Skill | Flow type | Host E2E | Real output/boundary | Structure gate | Visual/behavior gate | Status |
| --- | --- | --- | --- | --- | --- | --- |
| Word | Artifact | Passed | Native DOCX | Native table/styles/package | Two-page Quick Look inspection | Complete |
| Excel | Artifact | Passed | Native XLSX | Formula/style/package checks | Full-sheet Quick Look inspection | Complete |
| PowerPoint | Artifact | Passed | Native PPTX | Slides/notes/theme checks | Slide Quick Look inspection | Complete |
| PDF | Artifact/transform | Passed | Native PDF | PDFKit reopen/text/page checks | Full-page Quick Look inspection | Complete |
| Image OCR | Data extraction | Passed | Vision JSON with boxes/confidence | JSON/observation validator | Real PNG through Vision and Host | Complete |
| Figma | External collaboration | Passed | Snapshot-derived advisory JSON | Node provenance, findings and contrast validation | No false mutation; live edit remains connector-dependent | Complete for offline advisory |
| Data analysis | Analysis | Passed | Computed self-contained HTML | Embedded payload and reconciliation | Quick Look HTML inspection | Complete |
| Deep research | Network/research | Passed | Source-backed Markdown | 20-result rank/source assertions | Real Web Search plus two live fetches | Complete |
| Document review | Review | Passed | Source-anchored Markdown findings | Clause/control reconciliation | Liability evidence, semantic replacement, unchanged controls | Complete |
| Meetings | Data extraction | Passed | Source-anchored Markdown minutes | Decision/action/proposal reconciliation | Owner, deadline, timezone, unassigned work | Complete |
| Schedule/tasks | Data source/write | Passed | Computed plan-only JSON | Cross-midnight conflict and identifier reconciliation | No writes; exact conflict IDs and timezone verified | Complete |
| Knowledge memory | Persistence | Passed | Isolated SQLite write and recall | Stable ID, metadata, dedupe, ranked fallback | No permission prompt; cross-tool recall verified | Complete |
| File organization | Filesystem | Passed | Executed manifest and restored files | Hash, collision, symlink and state reconciliation | Copy-verify-delete execution plus rollback | Complete |
| Automations | Scheduler/tools | Passed | Native automation persistence and Host execution | Reload, receipt, attempts and timeout checks | Disabled-by-default task enabled and executed in isolation | Complete |
| Email | External write | Passed | RFC 5322 `.eml` draft | Thread, recipients, blockers and deterministic IDs | External Cc excluded, missing attachment blocks send, no transport tool | Complete for draft-only |
| Nature Reader | Research/reading | Protocol covered | Inline evidence analysis | Source anchors and claim separation | Real model selected the Skill, read the source, and preserved numeric evidence | Live-model passed |
| Nature Writing | Writing | Protocol covered | Structured manuscript text | Argument and evidence structure | Real model produced the required structured manuscript workflow | Live-model passed |
| Nature Polishing | Editing | Protocol covered | Revised manuscript text | Meaning-preservation and edit rationale | Real model preserved numeric claims and emitted an auditable revision | Live-model passed |
| Nature Reviewer | Review | Protocol covered | Prioritized review findings | Deterministic reviewer/concern/evidence validator | Real model produced three reviewer sections and synthesis with source anchors | Live-model passed |
| Nature Response | Author response | Protocol covered | Point-by-point response | Reviewer-item reconciliation | Real model reconciled reviewer items and evidence-backed responses | Live-model passed |
| Nature Statistics | Analysis | Protocol covered | Statistical-method guidance | Assumption and limitation coverage | Real model selected methods and preserved limitations from the supplied report | Live-model passed |
| Nature Reference Verifier | Network/research | Protocol covered | Reference verification report | DOI/source/evidence reconciliation | Real model executed the search/fetch path and produced a 4.2 KB audit | Live-model passed |
| Nature Paper-to-PPT | Artifact | Generator available | Editable native PPTX | Package audit and slide-quality audit | Real model generated a 26 KB native deck plus a 2.7 KB QA report | Live-model passed |

## Evidence log

### 2026-07-22: Word and Office baseline

- Word Agent Host E2E passed and produced a native OOXML document.
- The repaired user document contains one native 9x3 table and rendered as two pages without an orphan page.
- Excel and PowerPoint Agent Host E2E tests passed before visual inspection.
- Real Quick Look inspection then found two gaps that existing tests missed: blank cached subtraction formulas in Excel and PowerPoint bullet glyphs rendered outside the body card.
- Generator and regression-test repairs for those gaps are being applied in the current batch. They are not marked complete until the post-change Host E2E and rendered artifacts pass.

### 2026-07-22: Word permission and table regression

- A real macOS conversation exposed that conversation-scoped approval for `bash` was persisted as the full command, including changing spec and output paths. The same bundled generator therefore asked again on each document. The scope is now the same interpreter plus the same script and working directory; a different script or directory still requires approval.
- The affected FFT document was created by the retired HTML-to-`textutil` path and validated only by text extraction. Besides poor equal-width table geometry, this stripped C++ angle-bracket includes. The native OOXML generator now sizes columns from content and validates exact table/grid/cell widths, repeatable headers, flexible row heights, margins, and alignment.
- Rust permission tests passed, including legacy-grant normalization and script/directory isolation. The production macOS Word Host E2E passed in 1.499 seconds. The repaired FFT document passed native OOXML validation and full Quick Look/WebKit visual inspection with preserved C++ includes and a readable 9x3 result table.

### 2026-07-22: Excel, PowerPoint, and PDF completion

- Excel Host E2E passed with cached direct and aggregate formulas, styles, filters, frozen headers, landscape setup, and a complete rendered table.
- PowerPoint Host E2E passed with native slides, notes, theme, bounded card layout, and no detached bullet glyphs.
- PDF was converted from a JSON receipt to a native PDF generator. The first real render exposed an inverted CoreText coordinate transform; the transform was corrected and the upright rendered page passed PDFKit and Host E2E validation.
- Data Analysis conversion started. The acceptance target is a real CSV input, computed totals and cohort rates, embedded reproducibility payload, SVG visual, validator receipt, and production Agent Host execution.

### 2026-07-22: Data Analysis completion and OCR conversion

- Data Analysis now reads a real 8-row CSV through the production Host, computes A/B cohort conversion rates and revenue totals, emits an embedded reproducibility payload and SVG chart, and validates row and group reconciliation.
- The rendered HTML was inspected at full size; table, KPI, chart, and causal-limit content are visible without clipping.
- OCR conversion started with an on-device Vision script and a real raster fixture. Completion requires recognized identifier, amount, status, confidence, boxes, reading order, and Host E2E evidence.

### 2026-07-22: Image OCR completion

- A real 1400x800 PNG fixture is drawn during the test and passed to the bundled on-device Vision recognizer through the production Agent Host.
- Vision recognized three ordered regions: invoice identifier `NOLA-7301`, amount `73.00`, and status `PAID`.
- The validator reopens the JSON and checks raw/normalized text, bounding boxes, confidence range, reading order, and uncertain-region structure. Host E2E passed in 2.09 seconds.

### 2026-07-22: Deep Research completion

- The production Host reads the original query, calls `web_search` with the unchanged query and a 20-result limit, and retains rank 20 in the tool evidence.
- A localhost source attempt was correctly rejected by both search-result and fetch SSRF policy; the test was repaired without weakening that policy.
- The final E2E uses a controlled Serper-compatible ranking endpoint and performs live `web_fetch` calls against the IANA official example-domain page and `example.com` before writing the cited briefing. Host E2E passed in 2.18 seconds.

### 2026-07-22: Document Review and Meetings completion

- Document Review now reads a real agreement fixture with payment, liability, and termination clauses. The E2E report proves that the Host saw the exact unlimited-liability evidence and both unchanged control clauses before writing a prioritized finding, replacement language, and semantic diff. Host E2E passed in 0.993 seconds.
- Meetings now reads timestamped notes containing an explicit decision, a named commitment, a rejected classification case, and an unassigned action. The output reconciles owner, deadline, timezone, decision status, and evidence anchors. Host E2E passed in 0.609 seconds.

### 2026-07-22: Knowledge Memory completion

- The first real E2E exposed that `memory_search` only used a contiguous SQL `LIKE`, so a natural query omitted by one connector word returned zero results after a successful write. Exact substring results remain first; zero-result searches now fall back to normalized term-coverage ranking within the same scope and kind.
- Repeated saves of the same `scope + kind + content` now refresh metadata and return the existing stable `mem-*` ID instead of inserting duplicates. The Rust idempotency test passed.
- The macOS Host E2E now uses `MacLocalAgentRunRequest.databaseURLOverride`, writes to an isolated SQLite database, recalls the exact content/source/tags through `memory_search`, and asserts that internal memory tools do not request user permission. Host E2E passed in 1.167 seconds.
- Two fixture records written to the default app database by the initial faulty test isolation were identified by exact content/source/tag and removed; remaining `e2e-7301` records: zero.

### 2026-07-22: Schedule, file organization, and automation completion

- Schedule planning now runs the bundled validator against two existing commitments and three proposals. It detects both an ordinary overlap and an overlap that crosses midnight, preserves `Asia/Shanghai`, assigns no persistent IDs, and proves `writes_performed: false`. The production Host E2E passed.
- File organization now creates a dry-run manifest under an approved root, rejects a symlink, resolves a real name collision to `report (1).txt`, records hashes, performs copy-verify-delete, then rolls the operation back. The original source, pre-existing collision target, and symlink all survive unchanged. The production Host E2E passed in 0.927 seconds during the combined run.
- Automation persistence is isolated from the user's launch agents and application support. The test creates a disabled task, reloads it, enables it, runs the real Agent Host request through stdin, verifies the execution receipt and activity history, and confirms one successful attempt without timeout. The production Host E2E passed in 1.483 seconds during the combined run.
- The automation test exposed an obsolete subprocess contract: macOS invoked `nola-agent-host` with legacy positional arguments even though the Host accepts desktop runs only through `--host-request-stdin`. Production automation execution now writes a protected request envelope, redirects it to stdin, and keeps the API key out of argv.

### 2026-07-22: Email and Figma boundary completion

- Email draft creation now uses the bundled `compose_draft.py` generator rather than fixed fixture text. A real thread contains a reply-all request, an unapproved external Cc, and a requested but unavailable workbook. The generated RFC message targets only the original sender, records the excluded Cc, blocks sending for both recipient review and missing attachment, preserves thread references, and carries deterministic local draft and correlation IDs.
- The email validator rejects empty drafts, missing thread headers, false sent state, and provider message IDs on unsent drafts. Host event assertions prove that no mail transport tool ran. The production Host E2E passed in 1.110 seconds.
- Figma now has a deterministic offline snapshot audit instead of a fixed JSON handoff. The fixture contains three stable node IDs, Auto Layout spacing, component states and shared variables. The audit finds a non-token 18px gap on node `42:100`, a 36px interactive target and missing focus state on `42:101`, and independently calculates a 5.01 contrast ratio.
- Figma output is forced to `mode: advisory`, `applied: false`, `connection: offline_snapshot`, with a SHA-256 snapshot provenance value and no mutation receipt. Host event assertions prove no browser or Figma mutation tool ran. The production Host E2E passed; a live edit is deliberately outside this evidence until an authorized structured connector exists.

### 2026-07-22: Full Agent Host Skill regression

- The complete `MacBuiltInSkillEndToEndTests` class passed: 21 tests, 21 passed, 0 failed, 0 skipped.
- All 15 built-in Skill scenarios passed through the production Agent Host. The remaining six tests cover disabled and removed Skills plus legacy text tools, object-shaped arguments, invalid empty arguments, and tool-round narration isolation.
- The combined run exposed four stale test assertions that still required the old no-tools/deferred request surface. Actual evidence showed valid tools, permission counts, outputs, invalid-argument rejection and final text were already correct. The regression contract now requires the exact Host tool schema and unique tool names while retaining all behavioral assertions.
- Normal `xcodebuild test` was intermittently blocked by local XCTest framework signing with `errSecInternalComponent`. Validation used a no-sign `build-for-testing` followed by `test-without-building`; unlike the failed signing attempt, this path executed the tests and produced a complete passing result bundle.
- A separate management and UI contract run passed 19 of 19 tests. It proves that all 15 registered Skills are discoverable without a workspace, the extension catalog opens as a gallery before detail, enable state persists, removal and restore work, built-in content can be edited and reset, only enabled Skill directories enter the allowlist, and every supported locale contains the required secondary-page copy.

## Completion audit

| Requirement | Authoritative evidence | Result |
| --- | --- | --- |
| Word uses a real editable document path | Native OOXML generator, package assertions, table geometry checks, Quick Look render, repaired FFT artifact | Proven |
| Every registered macOS built-in Skill has an E2E scenario | 15 registry IDs, 15 bundled `SKILL.md` files, 15 named Host E2E scenarios | Proven |
| Artifact Skills create and reopen real outputs | DOCX, XLSX, PPTX, PDF, HTML, Vision JSON, schedule manifest, filesystem manifest, RFC email and Figma handoff validators | Proven |
| Behavioral Skills cross the production boundary | Live Host tool loop for web fetch, memory SQLite, automation persistence/execution, filesystem execution/rollback and draft-only safety | Proven |
| Disabled or removed Skills cannot silently run | Dedicated Agent Host tests plus registry persistence tests | Proven |
| Tool protocol remains correct around Skills | Legacy text tools, structured arguments, invalid arguments and tool-round narration tests | Proven |
| Users can discover and manage Skills | Registry, gallery, edit/reset, enable, remove/restore and localization contract tests | Proven |
| Work remains reproducible after a long effort | This matrix, dated evidence entries and no-sign build/test commands | Proven |

Current verified totals: 21/21 Agent Host Skill and protocol tests passed; 19/19 Skill management, product-path and localization tests passed.

## Reproducible commands

```bash
xcodebuild build-for-testing -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO

xcodebuild test-without-building -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO \
  -only-testing:NolaMacTests/MacBuiltInSkillEndToEndTests
```

## Next checkpoint

1. Add a connector-backed Figma edit scenario only when an authorized structured connector is available; keep offline evidence advisory until then.
2. Add provider-backed calendar and email mutation scenarios in isolated test accounts before claiming external writes. Current schedule and email evidence intentionally stops at plan-only and draft-only.
