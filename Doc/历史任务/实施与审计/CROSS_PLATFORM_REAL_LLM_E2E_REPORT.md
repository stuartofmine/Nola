> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Cross-platform real LLM E2E report

Last updated: 2026-08-02

## Purpose

This report tracks tests that exercise a configured, real LLM through Nola's
product paths. A mock HTTP server, a scripted event stream, a static prompt
snapshot, or a validator-only test does not count as real-model coverage.

Secrets must be supplied from the user's existing local configuration or
temporary environment variables. They must not be copied into the repository,
test result attachments, or console summaries.

## Authoritative status at 2026-08-01

This section supersedes status statements in the dated historical notes below.
Those notes are retained as an audit trail of failures and reruns, not as the
current coverage verdict.

| Product path | Latest real-model result | Authoritative evidence |
| --- | --- | --- |
| Core runtime | Pass | Full Rust suites passed: 114, 24, 1, and 4 tests; automatic compaction and transactional memory behavior are included |
| CLI agent | Pass | `scripts/test_real_cli_agent.py`; read/write, recommended and destructive permission handling, failure recovery, cancellation, tool history, Web search, large files, manual/automatic compaction, background tasks, and Goal resume |
| CLI data source | Pass | `NOLA_REAL_LLM_E2E=1 ./scripts/test_real_cli_data_source.sh`; randomized grounded value, disabled-source negative case, usage audit, zero permission events, and no checkpoint payload leak |
| CLI-hosted Web | Pass | `scripts/test_real_cli_web.sh`; token login, SSE, persistence, reversible write, destructive denial/resume, memory, and compaction |
| Web data source | Pass | Production authenticated Web session, persisted source contract, real-model grounding, and usage disclosure |
| macOS Agent Host | Pass | Real stream/tool/render, recommended permissions, cross-session memory, data-source grounding, structured progressive response, Skill lifecycle, automatic real-model compaction, and running-tool cancellation/recovery |
| iOS Agent Host | Pass | `scripts/test_real_mobile_agent_host.sh`; ordinary tool lifecycle, destructive denial/resume, cross-conversation memory, and encrypted Host cancellation/recovery; 4/4 real tests passed |
| iOS provider/data source | Pass | `scripts/test_real_mobile_providers.sh`; configured real stream plus validated source grounding and disclosure |
| iOS Skill lifecycle | Pass | `scripts/test_real_mobile_skill_lifecycle.sh`; one real test executed, one passed, zero skips |
| Android Agent Host | Pass | `scripts/test_real_mobile_agent_host.sh`; tool lifecycle, permission resume, cross-conversation memory, and encrypted Host cancellation/recovery; 4/4 real tests passed |
| Android provider/data source | Pass | `scripts/test_real_mobile_providers.sh` and the previously completed product vertical in `scripts/test_real_android_product_data_source.sh` |
| Android Skill lifecycle | Pass | `scripts/test_real_mobile_skill_lifecycle.sh`; device instrumentation reported `OK (1 test)` |
| Bundled Skills | Partial | Current-worktree bounded rerun: Word, meetings, knowledge memory, repaired data analysis, and Nature Reviewer passed. The remaining built-ins still require source-matched reruns before the earlier all-23 claim is current evidence |

## 2026-08-02 evidence correction

The earlier all-23 live-Skill result is retained as historical evidence, but it
is no longer treated as sufficient proof for the current worktree. The harness
could reuse a release binary older than the CLI, Core, or bundled Skill sources,
and current provider behavior exposed failures that the old run did not predict.

Current bounded real-model evidence:

| Case | Result | Duration | Evidence |
| --- | --- | ---: | --- |
| `word-document-authoring` | Pass | about 36 s | Native DOCX package and table validation |
| `assist-meetings` | Pass | under 31 s | Declared file workflow and final artifact checks |
| `curate-knowledge-memory` | Pass | under 31 s | Real persistence and recall boundary |
| `analyze-data` | Pass after repair | about 45 s | Bundled deterministic pipeline, reconciled rows/totals, distinct successful `--validate-only` call |
| `nature-reviewer` | Pass with academic timeout | about 300 s | Claim-anchored review plus bundled final validator |

Data analysis was repaired rather than granted an unbounded timeout. The Skill
now uses a bundled CSV/TSV pipeline instead of generating a large helper program.
The harness derives validator contracts from each manifest, rejects textual
lookalikes, refuses stale binaries, terminates process groups at the bound, and
uses category-specific limits: Nature academic Skills default to 600 seconds
with a 900-second hard cap; data analysis remains 180 seconds; other Skills
remain 300 seconds with a 600-second hard cap.

Mobile provider, data-source, uploaded-Skill, Host tool, cancellation, memory,
and destructive-permission real tests pass. They do not prove the macOS/CLI-Web
built-in artifact Skills. Android real Compose UI, device/system permissions,
Web search/Chrome/MCP, local models, and error-path coverage remain separate
known gaps.

The standalone Android product-data-source script was requested again at the
end of this pass, but no emulator or device remained online. This rerun is
environment-blocked and is not counted as new evidence. The earlier complete
device vertical remains the latest valid product-path result. Reconnecting a
device and rerunning it is still desirable, but the report does not convert a
missing device into either a product failure or a pass.

### What counts as a real E2E here

1. The configured external model is actually called; a mock, prompt snapshot,
   deterministic parser, or schema-only test is insufficient.
2. The request enters through the production client Provider or Agent Host,
   not a test-only direct call around Core.
3. Tool, permission, persistence, stream-order, or artifact effects are checked
   independently of the model's prose claim.
4. A skipped test is rejected rather than reported as a pass.
5. Isolated workspaces and randomized facts prevent stale files or memorized
   fixture text from satisfying assertions.

## Current evidence

| Surface | Product path exercised | Real model | Tools or data | Result | Evidence |
| --- | --- | --- | --- | --- | --- |
| macOS | Swift test target -> `MacAgentHostRuntime` -> Agent Host -> Core -> provider -> versioned Swift event stream | Yes | Provider stream, permission-free completion | Pass | `MacAgentCapabilityEndToEndTests.testRealConfiguredModelStreamsThroughMacAgentHostWhenExplicitlyEnabled`; 1 test, 0 failures, 5.738 s |
| macOS recommended permissions | Swift app -> Agent Host -> Core -> real model -> native file tools | Yes | Reversible write without a prompt; destructive delete suspended, denied, and resumed | Pass | `scripts/test_real_mac_agent_host.sh`; write emitted 0 permission requests, delete emitted exactly 1, denied file remained unchanged |
| macOS Word Skill | Swift app -> Agent Host -> Core -> real model -> bundled Word Skill -> artifact validator | Yes | `write_file`, `create_document.py`, native table, Quick Look rendering | Pass | `permission-word.docx`; 5 completed tool calls, 0 permission requests, package and Quick Look validation passed |
| macOS cross-session memory | Swift XCTest -> `MacAgentHostRuntime` -> Agent Host -> Core -> real model -> shared SQLite -> second conversation | Yes | `memory_save`, durable persistence, `memory_search`, exact randomized recall, zero permission prompts | Pass | `scripts/test_real_mac_memory.sh`; 1 test executed, 1 passed, 0 skipped, 0 failed |
| macOS validated Web source | Local HTTP source -> first-read validation and saved contract -> Context Core -> Agent Host -> Core -> real model | Yes | Randomized JSON evidence, conversation disclosure record, exact grounded answer | Pass | `scripts/test_real_mac_data_source.sh`; 1 test executed, 1 passed, 0 skipped, 0 failed |
| iOS | iOS provider -> configured OpenAI-compatible endpoint -> async stream | Yes | Provider stream | Pass | `EndToEndFlowTests.testRealOpenAIProviderStreamsConfiguredModelWhenExplicitlyEnabled`; iPhone 17 Pro simulator |
| iOS uploaded Skill | `ChatMemoryProfile` -> `DefaultContextManager` -> Context Core -> `OpenAIProvider` -> real model | Yes | Enable, edit A -> B, disable propagation | Pass | `EndToEndFlowTests.testRealModelObservesUploadedSkillEnableEditAndDisableLifecycle`; 1 executed, 1 passed, 0 skipped |
| iOS Agent Host | Production `WebAgentProvider` -> encrypted mobile API -> Agent Host -> Core -> real model | Yes | Ordinary tool, destructive permission denial, cross-conversation `memory_save` / `memory_search` | Pass | `scripts/test_real_mobile_agent_host.sh`; 3 tests, 0 failures, protected file preserved, exact randomized memory recalled |
| Android | Kotlin provider -> configured OpenAI-compatible endpoint -> stream | Yes | Provider stream and validated Android data-source context | Pass | `OpenAiProviderRealEndToEndTest`; 3 tests, 0 failures |
| Android uploaded Skill | encrypted profile store -> repository -> Rust Core FFI -> `OpenAiProvider` -> real model | Yes | Enable, edit A -> B, disable propagation | Pass | `AndroidProductDataSourceRealEndToEndTest.uploadedSkillLifecycleFlowsThroughStoreRepositoryCoreAndRealProvider`; `OK (1 test)` |
| Android Agent Host | Production `WebAgentProvider` -> encrypted mobile API -> Agent Host -> Core -> real model | Yes | Ordinary tool, destructive permission denial, cross-conversation `memory_save` / `memory_search` | Pass | `scripts/test_real_mobile_agent_host.sh`; 3 tests passed, exactly one permission trace, protected file preserved, exact randomized memory recalled |
| CLI | CLI batch entry -> Core agent loop -> provider -> NDJSON | Yes | Session persistence and final stream | Pass | Isolated session `cli-live-real-1`; exact response and replay verified |
| CLI tools | CLI batch entry -> Core agent loop -> provider -> file tools -> NDJSON | Yes | `skill_skip`, `write_file`, `read_file`, artifact verification | Pass | `/tmp/nola-cli-real-tool-e2e-20260730-181611`; 20 events; artifact content `NOLA_REAL_TOOL_OK` |
| CLI destructive permission | CLI batch entry -> real model -> destructive shell proposal -> permission gate | Yes | `bash rm` denied and target independently checked | Partial | `/tmp/nola-delete-permission-real-e2e-20260730-184725`; file preserved, but batch NDJSON lacks a structured permission event |
| Cross-session memory | macOS/iOS/Android/CLI/Web conversation A -> `memory_save` -> isolated persistent store -> conversation B -> `memory_search` | Yes | Internal memory tools, persistence, recall, permission semantics, model classification drift | Pass | `scripts/test_real_mac_memory.sh`, `scripts/live_memory_e2e.sh`, `scripts/test_real_mobile_agent_host.sh`, and `scripts/test_real_cli_web.sh`; exact randomized recall and zero permission events |
| Web | Host-token login -> cookie and CSRF session -> conversation persistence -> Agent v2 SSE -> durable traces | Yes | Browser session, real-model `read_file`, semantic stream deltas, context accounting, destructive shell approval, memory, compaction | Pass | `web/tests/test_real_llm_web_agent_e2e.py`, `web/tests/test_agent_host_full_e2e.py`, and `scripts/test_real_cli_web.sh` |
| Built-in Skills | Skill prompt -> real model -> declared tools/scripts -> artifact validators | Yes | Current-worktree bounded sample; full 23-case rerun pending | Partial | Word, meetings, memory, repaired data analysis, and Nature Reviewer pass |

The real endpoints and API keys are intentionally omitted from this report.

## Defects found and fixed during this pass

1. The existing iOS real-provider test silently skipped when environment
   variables were supplied only to the `xcodebuild` shell. The real variables
   now reach the test process through a temporary `.xctestrun`; the test was
   rerun and passed instead of being counted from a skip.
2. macOS had no test proving the complete Swift-to-Agent-Host real-model path.
   An opt-in test now asserts the exact streamed result, terminal event,
   absence of an unnecessary permission request, and versioned traces.
3. `MacCapabilityService.screenRecordingSnapshot()` constructed a permission
   snapshot without returning it. This prevented the macOS test target from
   compiling and has been corrected.
4. Earlier Web testing called the trace endpoint without first creating the
   production conversation. The corrected test follows the browser's actual
   login, conversation, message, stream, and trace sequence.
5. A provider-only response is insufficient evidence for agent behavior. A
   real-model CLI case now proves autonomous tool choice, write/read execution,
   artifact contents, and the final response in one isolated run.
6. A tool-capable model transaction could publish a conversational preamble
   before the provider later revealed a function call. Core eventually
   discarded the provisional text, but clients had already displayed it and
   visibly flashed or duplicated content. Tool-capable transactions now remain
   provisional until their terminal choice is known. A real memory write proves
   that no assistant text is visible before the last tool call; the accepted
   final answer is released as semantic presentation chunks.
7. Agent Host permission requests were split across a legacy event and Core
   traces, and a persistent Host could lose the turn-scoped resolver before
   executing an effect. The Host now captures the resolver at turn start and
   emits one permission request with the authoritative Core trace.
8. A denied destructive operation could prompt repeatedly in one turn. The
   first explicit denial now suppresses later permission prompts until the next
   turn, while completed tools are not replayed.
9. The latest macOS Word conversation generated and internally validated its
   document but ended with a generic status-1 error because an older bundled
   Core still required another validation round. The current Core classifies
   bundled document creators as mutating validators; the same product path was
   rebuilt and passed with Quick Look verification.
10. Non-CLI recommended permissions previously treated broad write categories
    as approval-worthy. Core now allows ordinary reads, network use, reversible
    writes, and edits while suspending concrete irreversible invocations such as
    delete, overwrite move, permanent clear, force reset, and non-preview restore.
11. The Agent Host forced a Skill decision before exposing normal task tools.
    This caused simple reads and deletes to load unrelated Skills. Skills are now
    optional specialized workflows in the same model catalog; real macOS reads
    use direct file tools and destructive deletes still reach the permission gate.
12. After a user denied an operation, an older engineering-validation obligation
    could discard the model's truthful denial answer and request another model
    turn. Permission denial now supersedes stale validation while still rejecting
    any attempted tool bypass.
13. A real iOS memory run saved a fact as `kind=other`, then a later model searched
    only `reference` and `user`, hiding the exact saved fact and returning an
    Android near-match. Text search now treats model-selected kind as a soft hint,
    broadens exact matching before fuzzy ranking, and supports the same kind enum
    as memory save. The failed real run was retained, fixed, and rerun successfully.
14. Mobile real E2E failures previously deleted their only isolated debug runtime,
    and Android assertions omitted terminal reason/detail fields. Failures now
    preserve a credentials-scrubbed runtime automatically and emit actionable
    trace diagnostics.
15. The Web real E2E still counted obsolete flat `tool_started/tool_result`
    events, so successful canonical traces appeared to have no memory tools.
    Its protocol assertions now read versioned trace envelopes while retaining
    legacy compatibility; real write, memory, permission, and compaction cases
    pass together.
16. Shell environment variables supplied to `xcodebuild test` did not reach the
    macOS XCTest process, so the new real-memory case initially reported a skip.
    `scripts/test_real_mac_memory.sh` now injects credentials into a temporary
    `.xctestrun`, checks the authoritative `xcresult`, and rejects skipped tests;
    the complete two-conversation product path was then executed and passed.
17. The existing macOS Web-source E2E stopped after assembling a prompt, so it
    proved fetching and policy gates but not that a real model received the
    source through the product Host. A new case validates a random local JSON
    source, records its disclosure, passes it through Context Core and Agent
    Host, and requires the real model to return the unseen random headline.
18. The browser E2E still expected ordinary `write_file` calls to open a
    permission panel. That contradicted the recommended non-CLI policy and made
    correct reversible execution look broken. The permission case now uses a
    destructive `bash rm`, proves the file exists before approval, and proves it
    is removed only after the same Core call resumes.
19. The deterministic destructive fixture initially called `delete_file`
    without reading its target. The resulting read-before-edit rejection was
    incorrectly interpreted as a permission-resume failure. The fixture now
    keeps file-tool safety coverage separate from permission recovery and uses
    destructive shell execution for the latter.
20. Web had no opt-in browser E2E that reached a configured real model. The new
    gate logs in through the production page, establishes the cookie/CSRF
    session, sends a natural file question, requires autonomous `read_file`
    selection, multiple semantic response deltas, durable message and trace
    evidence, no spurious permission card, and non-zero context accounting.
21. The Web E2E hard-coded the English `Send` accessibility label while running
    a Chinese page and read context usage from an obsolete nested field. It now
    asserts locale-independent enabled state and the authoritative
    `totalTokens` context-report field.
22. iOS appended a speculative “reasoning model used its output budget” notice
    whenever long reasoning ended in a short answer. This modified valid model
    output and broke exact Skill responses. The unsupported heuristic was
    removed; persisted and displayed text now preserve the actual final answer.
23. Android ignored provider `reasoning_content` and never sent DashScope's
    `enable_thinking=false` control. A thinking model could consume the whole
    completion budget and expose an empty response. Android now matches iOS:
    it streams explicit thinking blocks and sends the provider-specific control
    only to compatible DashScope Qwen/QwQ models.
24. Android instrumentation returns shell status zero even for JUnit failures.
    The reusable mobile Skill runner now parses `OK (1 test)` and rejects JUnit
    failure status instead of trusting the `adb` process code.
25. A reasoning-only response could terminate the Agent Host without a final
    answer or tool call. CLI Host recovery now performs one bounded request in
    the same request chain, disables thinking for the correction, and removes
    incompatible reasoning history before retrying.
26. When no tools were available, unavailable-tool recovery still instructed
    the model to call exactly one tool. This produced repeated impossible
    `list_dir` calls. The correction now explicitly tells the model to answer
    directly when the schema catalog is empty.
27. Cross-conversation memory was flaky because `memory_save` opened another
    validation obligation and required the model to search immediately after a
    successful durable write. Core now treats the internal save receipt as the
    completion of its mutation transaction; recall remains independently
    verified in a new conversation.
28. Android's real Agent Host tests had drifted to only a permission scenario.
    The suite now also proves a permission-free ordinary tool lifecycle and
    durable cross-conversation memory, using the same production provider and
    state reducer as the client.
29. A macOS Skill lifecycle fixture overrode `SKILL.md` but unintentionally
    inherited production artifact validators, so it tested contradictory
    contracts. The fixture now declares its own empty validation manifest and
    keeps lifecycle propagation separate from artifact-generation coverage.
30. iOS and Android stopped only their local stream task. The remote Agent Host
    could continue running a model or long-lived tool and persist results after
    the UI appeared stopped. Both Providers now send an authenticated,
    AES-GCM-encrypted cancellation request for the concrete conversation, and
    their chat coordinators invoke it when stopping or abandoning a Web Host
    generation.
31. The CLI-hosted Web backend implemented browser cancellation but did not
    register the equivalent mobile route, returning HTTP 501 to correctly
    encrypted clients. `/api/mobile/chat/agent/v2/cancel` now verifies the
    mobile token, decrypts the request, and reaches the same authoritative
    cancellation registry as the browser route.
32. A macOS test named `testRealAgentHostRepeatedCompaction...` used a mock
    OpenAI-compatible server. It remains useful Host/Core contract coverage but
    is not real-model evidence. A separate opt-in test now uses the configured
    provider, triggers real LLM summarization, verifies `context.compacted`,
    proves the original transcript remains intact, and recalls a randomized
    early fact after compaction.
33. The first real macOS compaction run also exposed an invalid test assumption:
    Agent Host owns Core checkpoints but the macOS client owns transcript
    persistence. The corrected test writes the user message through the client
    store before starting Host work and reloads the store before asserting;
    it does not require the Host to duplicate UI persistence responsibilities.

## Permission boundary

Non-CLI clients use the recommended Nola permission profile. Reads, searches,
ordinary process execution, and reversible file creation or editing proceed
without a Nola confirmation. Irreversible operations such as deleting user
data remain suspended until the client records an explicit decision. A denial
resumes the same turn without replaying completed tools.

This policy does not bypass operating-system security. Accessibility, Screen
Recording, Keychain, browser-extension trust, and other platform permissions
remain controlled by macOS, iOS, or Android and are requested only when the
corresponding capability is actually exercised.

## Coverage gaps that remain open

| Capability | macOS | iOS | Android | CLI | Web |
| --- | --- | --- | --- | --- | --- |
| Real provider streaming | Pass | Pass | Pass | Pass | Pass |
| Real tool selection and execution | Pass | Pass | Pass | Pass | Pass |
| Recommended permission decisions | Pass | Pass | Pass | Pass | Pass |
| Destructive-operation suspension and resume | Pass | Pass | Pass | Pass | Pass |
| Long-term memory write and cross-session recall | Pass | Pass | Pass | Pass | Pass |
| Skill enable/disable and edit propagation | Pass | Pass | Pass | Pass | Pass |
| Data source -> context -> grounded answer | Pass | Pass | Pass | Pass | Pass |
| Tool progress and expandable details in rendered UI | Partial | Partial | Partial | Pass | Partial |
| Context compaction with real model | Pass | Missing | Missing | Pass | Pass |
| Cancellation during thinking/tool execution | Pass | Pass | Pass | Pass | Pass |

`Partial` is deliberately not treated as a pass. Protocol/state-reducer tests
do not by themselves prove rendered animation, hit targets, accessibility, or
visual stability. The remaining rows require
real-model execution plus authoritative state or artifact assertions, not only
snapshot or contract tests.

The standalone CLI destructive case is intentionally partial: safety held, the model
reported the failed deletion truthfully, and a later read proved the file still
existed. However, the model selected shell deletion instead of the available
native `delete_file` capability, and non-interactive NDJSON represented the
denial only as a failed tool result. Agent Host clients now have separate
real-model proof that the permission request is structured, suspended, and
resumable without replaying completed tools.

## Required acceptance criteria

For each supported surface, completion requires:

1. A real configured model is reached through the product's production entry.
2. Stream order, stable message identity, terminal state, and traces are
   asserted.
3. At least one read-only tool and one reversible write tool are selected by
   the model and their effects are verified independently.
4. Recommended mode performs reversible work without prompting and suspends
   before an irreversible operation.
5. A pending permission can resume without replaying completed tools.
6. Long-term memory is written, persisted, and recalled in a new conversation.
7. At least one platform-native data source reaches the final grounded answer.
8. Context usage and compaction are checked against token accounting rather
   than message counts.
9. User-visible tool, thinking, attachment, and error states are validated in
   the rendered client, not only in backend events.
10. Tests use isolated workspaces and databases and do not request Accessibility,
    Screen Recording, or Keychain access unless that permission is the behavior
    under test.
## 2026-07-30 Web Agent Host regression closure

The authenticated Web product path was exercised with the configured real LLM provider, persistent Agent Host, Core planner, real filesystem tools, permission suspension/resume, SSE transport, database persistence, and context compaction.

Observed results:

- Authenticated login and token-backed session creation succeeded.
- A normal assistant turn completed through SSE without an assistant reset.
- The model selected and executed `write_file`; the expected file content was independently verified and produced zero permission requests under the recommended non-CLI profile.
- `delete_file` was not pre-exposed. After `skill_skip`, the model used dynamic tool discovery, selected the deferred destructive tool, and emitted exactly one permission request.
- A denied delete resumed the same turn, produced no tool execution error, and left the protected file unchanged.
- Manual compaction replaced six context items, retained three current items, preserved the checkpoint, and the same real model continued the conversation without an assistant reset.

Defects found and closed:

1. The persistent Host permission resolver retained an SSE sender after a turn completed, so HTTP streams could remain open after the final event. A turn-lifetime guard now releases resolver captures on every exit path.
2. The model could treat a missing matching Skill as evidence that no ordinary tool existed. The unified planner now explains deferred schemas and semantic `tool_search`; `skill_skip` explicitly returns control to the normal tool catalog.
3. Web destructive permission coverage previously accepted a model answer that merely claimed a denial. The real E2E now requires an actual permission event and independently verifies filesystem state.

Authoritative command:

```bash
NOLA_REAL_WEB_ONLY=1 NOLA_SKIP_CLI_BUILD=1 ./scripts/test_real_cli_web.sh
```

Latest result: **Pass**.

## 2026-08-01 real cancellation and macOS compaction closure

### Cancellation

- Web browser: the production page starts a real-model foreground `bash sleep`
  tool, calls `stopActiveGeneration()`, persists exactly one `run.cancelled`
  terminal, and completes a randomized recovery turn. Result: **1 passed in
  9.51 seconds**.
- macOS: `MacAgentHostRuntime` cancels only after the real `bash` tool emits
  `tool.started`; the run has one cancelled terminal, completes in under ten
  seconds, and the same conversation accepts a real recovery turn. Result:
  **1 passed, 0 skipped, 0 failed**.
- Android: the production `WebAgentProvider` sends the encrypted mobile cancel
  request after `tool.started`, observes `run.cancelled`, and recovers in the
  same Host conversation. The complete real Host class passed **4/4** tests.
- iOS: the production `WebAgentProvider` follows the same encrypted mobile path
  and generation-scoped conversation identity. The complete selected suite
  passed **4/4** tests in **25.477 seconds**; its cancellation case passed in
  **4.173 seconds**.
- CLI: the existing PTY real gate still proves Esc cancellation and a successful
  subsequent request.

The mobile route contract additionally passed the focused Rust test
`remote_web_host_surface_only_exposes_authenticated_mobile_api`.

### macOS real-model compaction

`scripts/test_real_mac_compaction.sh` is an explicit opt-in test. It uses the
configured external model and a 16K product context window, sends bounded
multi-turn history, requires a real `context.compacted` event, checks that the
client-owned original transcript remains present, and recalls an unpredictable
first-turn fact after the LLM-generated summary. Latest result: **1 passed, 0
skipped, 0 failed**.

iOS and Android automatic compaction remain unproven through their production
clients. Shared Host compaction evidence is not counted as mobile-client proof;
those matrix cells intentionally remain `Missing`.

## 2026-07-30 CLI data-source real-provider acceptance

The CLI data-source path was exercised from the actual product command through validation, persisted contract discovery, per-conversation selection, Core context preparation, the configured real LLM, usage persistence, and checkpoint sanitization.

Observed results:

- `/sources add` performed a bounded real read and persisted a verified connector contract with reader description, discovered fields, record count, freshness policy, observation time, and a values-redacted sample.
- A conversation-opt-in source was unavailable to an unrelated conversation.
- After explicit per-conversation enablement, the real model returned the exact randomized source value; Markdown-only wrapping was normalized without relaxing content equality.
- An instruction-like string inside the source remained reference data and did not control the model response.
- Source use produced zero permission events and persisted a `cli_chat` / `conversation_context` usage record.
- The one-turn source payload was absent from the durable Agent checkpoint.

Defect found and closed:

- Non-interactive and piped CLI invocations sent `/sources ...` to the LLM instead of the product command layer. The shared one-shot dispatcher now handles `/sources` locally before any context or provider request. The existing spaced-path regression now enters through `run_prompt` rather than calling the handler directly.

Authoritative command:

```bash
NOLA_REAL_LLM_E2E=1 ./scripts/test_real_cli_data_source.sh
```

Latest result: **Pass**.
## 2026-07-31 context, data-source, and permission follow-up

- macOS/non-CLI recommended permission behavior: verified. Reversible reads and workspace writes are preauthorized; destructive operations such as deletion still require an explicit decision. Conversation grants are reused rather than prompting for every write.
- Web data-source grounding: **Pass** through the real authenticated Web session, persisted data-source contract, explicit source test, SSE chat, real model grounding, and usage disclosure record.
- iOS data-source grounding: **Pass** through production `ChatManager`, `DefaultContextManager`, `URLFetcherService`, conversation source selection, real `OpenAIProvider`, persisted message order, and automatic usage disclosure.
- Android data-source grounding: **Partial**. The current real test validates document inspection, Provider grounding, and audit-record construction, but it manually supplies model context. It does not yet prove the device `ContextRepositoryImpl -> AndroidContextCoordinator -> ChatViewModel -> Provider` path.
- Shared Agent Core compaction: **Pass** with a real model. The test forces automatic compaction, observes `context.compacted`, recalls an early exact fact after multiple large turns, and verifies failed model requests produce an error terminal without a false completion.
- Fixed a Core compaction boundary defect: an oversized current user round could be summarized away, causing the model to repeat an earlier instruction. The newest round is now always retained intact; only completed history is eligible for summarization.
- Fixed a Core host-interaction defect: when `request_user_input` reports that interactive input is unavailable, the turn now ends with the unresolved question instead of repeatedly invoking the model and tool until the batch guard fails.

## 2026-07-31 Android product data-source vertical

- Added a device-level product E2E that exercises `MemoryProfileDataStore -> AndroidDataSourceRepository -> ContextRepositoryImpl -> AndroidContextCoordinator/Rust Core FFI -> OpenAiProvider -> configured real LLM -> usage audit persistence`.
- The data source is fetched through a real localhost HTTP socket, validated into a reusable contract, selected by the product access policy, and passed through the same context builder used by `ChatViewModel`.
- The first real run failed and proved a product defect: Android emitted `authorization = configured` and an epoch-millisecond `observedAt`; Core only accepts readable authorization snapshots (`authorized`, `granted`, `available`) and RFC 3339 freshness timestamps, so it replaced the source body with an omission notice.
- Fixed Android custom-source metadata to emit `authorized` plus ISO-8601/RFC 3339 time. The rerun passed with the real model returning a per-run random fact and the source usage record persisted.
- Command: `./scripts/test_real_android_product_data_source.sh`
- Result: `OK (1 test)`, no skips.

## 2026-07-31 permission and mobile Agent Host verification

### Verified

- Non-CLI Agent Host surfaces now default to the recommended permission profile. Ordinary reversible file, process, browser, desktop, memory, and skill operations proceed without Nola approval; concrete destructive operations still suspend for a decision. macOS TCC and explicit user denials remain authoritative.
- Core recommended-permission classifier: 12 focused cases passed, including safe native automation/document generation and irreversible shell, Git, AppleScript, and file deletion cases.
- macOS remote capability plan: 2 focused XCTest cases passed. Connected file, process, browser, computer, and clipboard capabilities are available to Agent Core without converting every invocation into a permission prompt.
- Android OpenAI provider: 2 real-model requests passed (incremental streaming and validated data-source disclosure/audit).
- Android production WebAgentProvider against an isolated real CLI Web Agent Host: 3 real-model flows passed (safe tool lifecycle without approval, cross-conversation memory save/recall, destructive delete permission denial and resume).
- The real Android tool flow now also replays authoritative Core Trace events through the production chat state reducer. It verifies one stable assistant placeholder ID, incremental assistant text, and the `system_time` tool transition to `done` without a duplicate assistant message.

### Not yet proven

- iOS real-provider tests built successfully, but the simulator run did not reach Nola: the first simulator lost its CoreSimulatorService connection and a clean replacement remained stuck in LaunchServices data migration. This is recorded as infrastructure-blocked, not passed and not a product assertion failure.
- A healthy iOS simulator/device run is still required for the three production WebAgentProvider flows and the provider/data-source baseline before cross-platform mobile verification can be considered complete.
- The macOS native permission-status XCTest no longer assumes the production app bundle ID while running in the XCTest host. It now compares the bridge response with the live permission subject, and the focused test passes.

## 2026-07-31 CLI and CLI-hosted Web real-product regression

### Verified

- CLI JSON streaming: one real answer, no assistant reset, one terminal completion event, and ordered thinking/assistant deltas.
- Interactive terminal: all 40 requested scrollback lines remained visible; the answer path did not enable an ANSI scroll region; Esc stopped generation in 743 ms with only 13 additional buffered lines; the next real request succeeded.
- CLI Agent: safe file read, explicit write denial and `--yes` approval, conversation-scoped permission reuse, failed-tool recovery, ordered tool results, cross-process tool history, manual and automatic LLM compaction, background lifecycle, and persisted Goal resume all passed.
- Real web search preserved the original Chinese query `2026年7月伊朗局势`, returned 20 ranked results, parsed 13 pages, and exposed source URLs.
- CLI-hosted Web token session: authentication, conversation persistence, SSE ordering, recommended safe write without approval, destructive delete suspension and denial, protected-file preservation, memory save/search across conversations, and compacted continuation all passed.
- Web permission evidence matched the intended profile: safe write requests = 0; destructive delete requests = 1; memory permission requests = 0.

### Not yet proven

- A real in-app browser DOM/visual run could not start because the browser runtime reported no available browser instances. The HTTP/SSE product path is verified, but Web tool cards, thinking presentation, progressive Markdown, and visual stability remain unproven in an actual rendered page.

## 2026-07-31 全平台真实模型与 Skill 扩展审计

### 覆盖结论

本轮不以静态 schema、mock LLM 或单纯编译成功代替真实功能验证。23 个 macOS 内置 Skill 均通过已配置真实 LLM、Agent Core、Agent Host、真实工具执行及产物检查的端到端链路：

- Nature：`nature-reader`、`nature-writing`、`nature-polishing`、`nature-reviewer`、`nature-response`、`nature-statistics`、`nature-ref-verifier`、`nature-paper2ppt`
- 文档：`word-document-authoring`、`excel-workbook-authoring`、`powerpoint-presentation-authoring`、`work-with-pdf`
- 研究与设计：`collaborate-in-figma`、`analyze-data`、`conduct-deep-research`、`review-documents`、`extract-image-text`
- 日常工作：`assist-meetings`、`plan-schedule-tasks`、`curate-knowledge-memory`、`organize-files`、`design-automations`、`assist-email`

真实调用暴露并修复了以下仅靠静态测试无法发现的问题：

1. 模型只返回 reasoning、未返回正文或工具调用时，Core 会错误结束。现在仅进行一次有界恢复请求，并保留完整工具能力；再次出现才明确失败。
2. `tool_search` 找到的延迟工具只在下一轮短暂可见，后续模型轮会丢失。现在在当前用户轮次内持续保留发现的 schema，下一用户轮自动清空。
3. Skill 的真实资源目录来自运行时返回值，Host 过去重新推导目录，临时安装目录下会阻止脚本执行。现在使用 Skill 成功结果中的 `base_directory`，仅在缺失时回退托管目录。
4. shell 策略错误拒绝安全的 `/dev/null` 重定向和文件描述符复制。现在精确允许 `/dev/null` 与描述符复制，并继续拒绝任意文件重定向。
5. Figma、深度研究和文件整理暴露了伪 E2E：测试把产物存在或只读 Python/shell 校验误判成完整成功。真实测试现会验证 Skill 选择、工具链、产物、持久化与只读校验边界。
6. Host 曾把“未放进精简模型 schema”误当成“Core 不可执行”，导致 `workspace_status`、`tool_output_read`、`propose_agent_plan` 被错误协议纠正。现在执行可用性以 Core 本轮完整能力目录为准，模型仍只接收精简 schema；真正未知工具和跨用户轮遗留动态工具仍被拒绝。
7. 普通 `move_file` 被无条件升级成整次调用权限范围。现在可逆移动按目标资源授权；仅不可逆或明确破坏性参数采用完整调用权限。

### 平台证据

| 平台 | 真实模型/协议证据 | 结论 |
|---|---|---|
| macOS | 原生客户端经唯一 Agent Host 路径完成真实模型、Skill、工具、记忆及破坏性权限链路 | 已验证核心 Agent 链路；23 个内置 Skill 全通过 |
| CLI | 真实 provider 回归覆盖安全工具、动态工具、长任务、权限暂停/恢复、上下文与 Skill；PTY 覆盖真实 Host 循环 | 已验证；全量 Rust 测试通过 |
| Web | 与 CLI 共享 Agent Host/Core；真实会话覆盖工具、权限 broker、上下文和 Skill 协议 | 已验证后端协议；浏览器内视觉交互仍需可用 DOM 环境 |
| Android | 真实 provider + Agent Host 覆盖安全工具生命周期、跨会话记忆、破坏性权限拒绝/恢复及 UI reducer | 已验证协议与客户端状态归约 |
| iOS | 真实 provider 目标可构建，Core/Host 合同测试通过 | 模拟器连接/迁移环境未提供稳定运行时证据，不能表述为完整 E2E 通过 |

### 回归结果

- `core`: 全量测试通过，包括 111 个库测试、24 个 Host 合同测试及附加测试套件。
- `cli`: `cargo test --quiet` 通过，主套件 `619 passed, 1 ignored`；PTY 交互 `23 passed`；其余附加套件通过。
- PTY 专项覆盖：连续多工具、本地化展示、内部完整输出续读、参数别名、超过 100 文件的显式广范围计划、未知工具恢复、权限自定义输入、流式 Markdown、长输出滚屏、Esc 中断及上下文 footer。
- Skill E2E：23/23 真实 LLM 调用通过，并检查真实工具结果与产物。

### 客观剩余风险

- iOS 当前只有构建与合同证据，缺少稳定模拟器/真机下的完整真实模型、权限和 UI E2E。
- 当前运行环境没有可控制的内置浏览器页面，Web 的 DOM、视觉回归和真实点击流未完成；这不影响已通过的 Web Host/API 协议测试。
- `conduct-deep-research` 能正确完成，但真实模型在多来源收敛上耗时明显高于其他 Skill；后续应以质量不下降为约束优化工具轮次和停止条件，而不是添加关键词路由。
- Rust 编译仍有既存 dead-code 警告；不影响本轮测试结果，但应单独清理，避免真正的新警告被噪声掩盖。

## 2026-08-01 Mobile automatic compaction verification

This section supersedes any earlier entry that lists iOS or Android automatic context compaction as missing.

- Android: **Pass**. The production `WebAgentProvider` completed the existing four real Host scenarios plus automatic compaction and post-compaction recall against an isolated 16K-token Host.
- iOS: **Pass**. Five production `WebAgentProvider` scenarios completed with zero failures and zero skips. Automatic compaction emitted `context.compacted`, continued the same conversation, and recalled a randomized first-turn fact after compaction in 38.562 seconds.
- The test uses an isolated Host configuration with a 16K context window and 12K compaction threshold. It does not mock the model, provider, mobile transport, trace stream, or persistence layer.
- Defect found and fixed: the iOS closed `AgentTraceKind` contract omitted `context.compacted`, making the trace reducer non-exhaustive once the event was added and leaving production clients unable to represent a real Host event.
- Defect found and fixed: the compaction flag was inherited by Gradle but not by the iOS Simulator test process. The runner now injects it into the generated `.xctestrun` target environment, and a skipped test can no longer be mistaken for coverage.

Commands:

```bash
NOLA_SKIP_CLI_BUILD=1 bash scripts/test_real_mobile_compaction.sh
NOLA_SKIP_CLI_BUILD=1 NOLA_SKIP_ANDROID=1 NOLA_SKIP_IOS_BUILD=1 bash scripts/test_real_mobile_compaction.sh
```

## 2026-08-01 Rendered chat vertical verification

- macOS: **Pass**. A real configured model ran through production `MacWorkspaceModel.send()`, selected and completed `read_file`, retained the real tool trace and Markdown in the authoritative assistant message, and rendered that message through production `MacMessageView` into a non-empty bitmap.
- The test fails if the model skips the required tool, the Host loses the trace, the message projection drops Markdown/tool state, or the production SwiftUI bubble cannot render.
- Android coverage correction: `ChatHostVerticalEndToEndTest` uses an in-process deterministic loopback SSE fixture. It is valid vertical UI contract coverage, but it is not a real Host or real-model test; the method was renamed to make this explicit.
- iOS coverage correction: `testCanonicalHostTranscriptRendersThinkingToolsAndIncrementalMarkdownExactlyOnce` is a launch-fixture UI test, not a real-model test.
- Remaining gap: iOS and Android still need a single-run real Host-to-production-rendered-chat UI test. Their real Provider tests and fixture UI tests pass independently, but that is not equivalent to a vertical UI E2E.

Command:

```bash
bash scripts/test_real_mac_rendered_chat.sh
```
## 2026-08-01 Mobile trace continuity and rendered-chat verification

### Defect found by real-model testing

The production mobile Agent Host stream emitted trace sequences `1, 2, 3, 4, 6, 7, 8, 9` for a real tool turn. Sequence 5 was a canonical trace that the terminal presentation intentionally kept quiet via `end_core_event_without_output()`. Because the same emitter also owns the native Host JSON protocol, the event was incorrectly removed from transport. Strict mobile reducers then rejected every later event, leaving the tool running and the assistant body empty.

The fix separates presentation silence from protocol delivery: quiet terminal events remain invisible in the human CLI, but every numbered canonical trace is emitted to JSON/native Hosts. Interactive permission requests now transport their canonical `tool.permission.requested` trace only inside the nonce/digest-bearing `permission_request` envelope, preventing duplicate permission UI while preserving sequence continuity.

### Real verification

- iOS production `WebAgentProvider`: 4/4 real-model scenarios passed (tool lifecycle with contiguous trace assertion, cancellation and recovery, destructive permission denial/resume, and cross-conversation memory).
- iOS production rendered chat: 1/1 XCUI scenario passed. The real model called `read_file`; the production assistant bubble rendered tool activity, Markdown answer content, and the randomized file marker.
- Android production `WebAgentProvider`: 4/4 real-model scenarios passed. The tool scenario now also asserts the complete trace sequence is contiguous.
- macOS production rendered chat: 1/1 real-model scenario passed, including native tool trace and Markdown bitmap rendering.

Android currently has real production Provider coverage but not an equivalent device-level, real-model Compose rendering assertion. Existing Android UI-host tests using loopback fixtures are not counted as real-model rendered UI evidence.

## 2026-08-01 Web Skill recovery audit

A new production Web vertical was added to `scripts/test_real_cli_web.sh`. It signs in through the token session, creates an isolated conversation, asks the real configured model to invoke the bundled `analyze-data` Skill against a generated CSV, and accepts the run only when the requested Markdown artifact exists, contains revenue and limitations analysis, emits one Skill start, requests no permission, completes the run, and preserves contiguous trace sequence numbers.

The real runs exposed and drove fixes for four independent reliability defects:

1. A truncated `write_file` tool payload was marked execution-skipped, so Core could accept later planning prose as `run.completed` despite zero successful mutations. Core now retains a protocol-recovery obligation; a compatible successful tool call must clear it, otherwise completion is rejected and the run eventually fails explicitly.
2. The model-facing failure summary previously truncated the full schema before `next_action`. Failure feedback now prioritizes compact `error`, `message`, and `next_action` fields.
3. Recovery recommended `append_file` while the deferred tool catalog did not expose its schema. Core now records the concrete recovery tool and injects its Host-catalog schema into the next model request. This is driven by the observed tool protocol error, not user-text routing.
4. The Bash path-policy parser rejected the read-only `$?` exit-status display and mislabeled it as a workspace-boundary violation. Exact `$?` display is now accepted while command, environment, and path expansions remain blocked.

Real evidence after these fixes: the Web data-analysis path emitted `dataSkillStarted=1`, `dataSkillPermissionRequests=0`, `dataSkillRunCompleted=1`, `dataSkillTraceContiguous=true`, `dataSkillArtifactExists=true`, `dataSkillArtifactHasRevenue=true`, and `dataSkillArtifactHasLimitations=true`.

The same full-suite run also exposed a test-contract error: a cross-session memory fixture called its value a “verification code,” correctly triggering the model's credential-safety refusal. The fixture now uses an explicitly non-sensitive durable project label. A subsequent gateway response returned a completely empty model completion; Core now permits one bounded empty-completion continuation and fails only if the provider repeats the empty response. This final recovery is Core-compiled and awaits the next rebuilt production-suite repetition.

## 2026-08-09 macOS retrieval, continuation, and rendered-stream audit

- The production macOS rendered-chat E2E passed through `MacWorkspaceModel`, Agent Host, shared Core, a configured real model, production persistence, and `MacMessageView`. It observed progressive Markdown stages with a stable committed prefix and no rendered-text regression.
- Core Text Parts that share one provider `message_id` are now concatenated into one authoritative assistant body instead of replacing earlier streamed segments. This closes the regression where a later provider segment could make an older answer disappear or visibly shrink.
- `web_search` returns a compact parsed-page evidence report while preserving the ranked source index. Normal `web_fetch` results use the same bounded inline-evidence policy, avoiding unnecessary persistence/continuation for ordinary pages.
- Internal `tool_output_*` continuation tools are no longer part of the initial model catalog. They are activated only by a typed Host result carrying a real `full_output_ref`; URLs and guessed paths cannot activate or address them.
- The real-model semantic Judge now samples every completed tool within a bounded evidence budget and reads the configured endpoint through SSE. A direct isolated real-provider receipt passed on the first attempt with all four quality scores at 4/4.
- The macOS scenario writes its evidence step before returning a failing acceptance code. Provider traffic, canonical events, persistence, rendering metrics, and the exact failed assertions therefore remain available to the central validator; a semantic or quality failure can no longer be misreported as an unexecuted product path.
- The natural retrieval regression repeatedly proved clean tool pairing, zero tool execution errors, durable session persistence, stable incremental rendering, and a zero-tool follow-up. It also exposed two remaining model-policy failures that are intentionally not hidden by the harness: some runs exceeded the scenario's eight-tool quality budget, and some answers appended excluded results or unsolicited references to the previous task. Core response and evidence policy were tightened without user-text keyword routing; the complete scenario must be rerun before those two gates are reported as passed.

Relevant commands:

```bash
python3 scripts/test_judge_real_e2e_response.py
cd core && cargo test --quiet prompt_composer
python3 scripts/run_real_system_tests.py \
  --scenario mac-parallel-natural-follow-up \
  --artifacts .artifacts/real-e2e/mac-parallel-natural-follow-up
```

## Mac parallel retrieval final gate (2026-08-10)

The `mac-parallel-natural-follow-up` real-provider scenario passed from the native Mac product runtime in 118.5 seconds. Evidence is stored under `.artifacts/real-e2e/mac-parallel-complete-final/`.

Validated boundaries:

- six tool calls completed within the eight-call product ceiling;
- every tool start had a paired terminal result and no protocol error occurred;
- reaching the shared retrieval budget transitioned through Core's tool-free evidence synthesis instead of retrying hidden tools;
- the assistant body was progressively projected and rendered as Markdown with a stable committed prefix;
- the durable session audit passed and the isolated natural follow-up used no tools or stale task content;
- the independent real-model Judge passed through the Thinking continuation protocol: its first `reasoning_content` was returned to the provider, then the verdict was submitted through the structured `submit_verdict` call.

The run also exercises bounded transport retries for read-only `web_fetch` requests. HTTP responses remain authoritative; only request-transport failures receive the finite retry sequence.

## 2026-08-18 checkpoint and background-permission recovery audit

- A real PTY product path now covers a parent conversation spawning a durable child Agent, the child suspending on `write_file`, the user granting that capability for the conversation, two writes reusing the same grant, and the completed result returning through `/tasks`.
- The original run appeared hung for nearly an hour because each persisted Core checkpoint retained the pending `journal.checkpoint` request, whose payload contained the preceding checkpoint. Payloads doubled on successive progress events and reached tens of megabytes while the process spent its time serializing recursive JSON.
- Core checkpoint projection now removes internal persistence effects. External pending effects remain recoverable through the committed outbox. Application uses a transition-provided checkpoint when present and Core's persistence-safe projection for stream responses that do not emit a checkpoint effect.
- Background tool/status events update the durable task read model and Trace without synchronously writing a parent-Core task mutation for every animation frame. Lifecycle boundaries such as permission wait/resume and completion remain canonical Core facts.
- The E2E no longer assumes ordering between concurrent parent and child model connections or invents an extra model synthesis after an already-final tool continuation. It validates both artifacts, exactly one permission panel, completed task state, checkpoint size below 2 MB, and absence of `journal.checkpoint` inside every persisted checkpoint.

Commands:

```bash
cd cli && cargo test --quiet --test pty_interaction background_agent_reuses_permission_panel_and_conversation_scoped_approval -- --exact
cd core && cargo test --quiet
```

## 2026-08-24 macOS canonical resource and Word Skill verification

The macOS real-product test executed the complete `Swift -> Agent Host -> Core -> real LLM -> Skill -> write_file -> bash -> canonical Store -> SwiftUI` path.

Validation commands:

```bash
cd core
cargo test -p nola_host_protocol artifact_contract_accepts_encoded_absolute_uris_and_rejects_raw_paths --quiet
cargo test -p nola_core --test agent_core_host_contract invalid_host_artifact_is_rejected_before_consuming_the_pending_effect --quiet
cd ..
./scripts/test_real_mac_product_ui_multiturn.sh
```

Authoritative evidence for the final run:

- Diagnostics: `/tmp/nola-mac-real-memory.n0rTnv`
- Final screenshot: `/tmp/nola-mac-real-memory.n0rTnv/runtime/mac-product-ui-screenshots/09-word-completed.png`
- Host/Core event log: `/tmp/nola-mac-real-memory.n0rTnv/runtime/mac-agent-events.json`
- Canonical metrics: 2 completed runs, 0 failed runs, 0 open Parts, 0 empty completed assistants, 0 contradictory Trace failures.
- Artifact event URI: `file:///private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-product-ui-9252CF71-AD5C-4C61-BEF4-D6BB585CE3B3/AI%E8%B5%9B%E4%BA%8B_NOLA-MAC-REAL-CHAIN-E2E.docx`.
- Generated document receipt: `validated: true`, one native Word table (4 columns x 9 rows), and 8 hyperlinks.

The real run also verified that the user sees the Skill invocation, specification write, generator command, validation step, final answer, and clickable document link. The UI did not expose the previous unexplained model-turn/tool-call counter detail.

The broader Host contract is now green. Core no longer keeps a platform-specific whitelist of direct tool names: the Host catalog owns model visibility through the typed exposure contract, while specialized capabilities remain deferred until activated. Runtime session configuration is canonicalized through `AgentSessionConfiguration::from_runtime`, avoiding checkpoint drift between raw and restored configurations. The complete `agent_core_host_contract` suite passes `46/46`, and the Core library suite passes `221/221`.

A subsequent real run exposed a separate Provider-to-Core boundary defect: after the Word Skill activated, the OpenAI-compatible stream returned `function.arguments` as JSON text, but a Host path submitted that text to Core as a JSON string. Core correctly rejected `write_file` with `invalid_tool_arguments` before execution. The Host boundary now assembles streamed argument fragments and converts the completed payload to a structured JSON value exactly once. Regression coverage includes double-encoded gateway payloads and whitespace-only stream fragments; all targeted tests pass.

The final real macOS rerun passed with diagnostics at `/tmp/nola-mac-real-memory.dRTc2E`: two canonical runs completed, zero failed runs, zero open Parts, zero empty completed assistant messages, and zero contradictory Trace failures. `write_file` completed successfully, the bundled Word Skill produced a validated `.docx` with one native table and one hyperlink, and the artifact was projected as a canonical percent-encoded `file:` URI. Manual inspection of `runtime/mac-product-ui-screenshots/08-word-completed.png` confirmed that the visible UI contains the reasoning/Skill/file/command process and the clickable final document result.
