> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Real End-to-End Testing

Nola uses one test matrix for evidence that crosses real process, provider, Core,
storage, and UI boundaries. A test name containing `EndToEnd` is not evidence by
itself. Real acceptance scenarios must use the configured model, production
defaults, isolated persisted state, and observable product output.

## Evidence layers

| Layer | Proves | Does not prove |
|---|---|---|
| Contract | Core state transitions and protocol invariants | Host wiring or UI |
| System | Provider, Agent Host, Core, tools, persistence, recovery | Rendered product UI |
| Product UI | The real client process and rendered user-visible result | Other clients |

Mocks and deterministic SSE fixtures remain useful contract tests. They must not
be described as real-provider or full product E2E evidence.

## Unified runner

List the smoke matrix without executing it:

```bash
python3 scripts/run_real_system_tests.py --list
```

Run the real-model smoke matrix for CLI, Web, Mac, iOS, and Android:

```bash
python3 scripts/run_real_system_tests.py --suite smoke
```

Run one platform or one scenario:

```bash
python3 scripts/run_real_system_tests.py --suite full --platform mac
python3 scripts/run_real_system_tests.py --suite smoke --scenario cli-multitool-recovery
python3 scripts/run_real_system_tests.py --scenario android-provider-streaming
python3 scripts/run_real_system_tests.py --scenario ios-provider-streaming
```

Passing one or more `--scenario` values without `--suite` runs those scenarios
directly; it does not silently intersect them with the default smoke suite.

The runner reads `~/.nola/config.json` by default. Use `--config` to select a
different configuration. Secrets are injected into child processes but are not
written to the report. Logs and `report.json` are stored under
`.artifacts/real-e2e/<timestamp>/`.

The iOS product scenario isolates its Host runtime, database, credentials, and
conversation on every run while reusing only Xcode build products under
`.artifacts/real-e2e-cache/ios-derived-data`. Set
`NOLA_IOS_REAL_CLEAN_BUILD=1` for a fully cold build, or set
`NOLA_IOS_REAL_DERIVED_DATA=/absolute/path` to select a CI cache location.

Passing requires all selected scenarios to execute. A zero exit status that only
contains skipped tests is rejected. Each scenario has an explicit timeout and
records its duration, platform coverage, evidence layer, and log path. While a
scenario is running, the runner prints a heartbeat every 30 seconds with elapsed
time and its live log path. A timeout terminates the scenario process group, not
only its wrapper shell, so spawned servers and device helpers cannot remain
orphaned.

The heartbeat also reports the most recent bracketed phase emitted by the
scenario, for example `building NolaMac`, `running isolated real-provider tool
turn`, or `judging answer against real tool evidence`. Scenario scripts must
emit a phase before every potentially long build, launch, provider, artifact,
and Judge stage. This keeps cold native compilation distinguishable from an
Agent or provider stall. Build logs, runtime evidence, and semantic audit remain
separate artifacts even when the scenario has one outer safety timeout.

A successful process exit is not sufficient evidence. Real tool scenarios must
assert named runtime invariants, durable state, tool receipts, streaming/render
projections, and a separately generated semantic verdict. When a runtime
assertion fails, the log must list its stable assertion name rather than collapse
the result into a single boolean.

Independent scenario runs can be combined without weakening the evidence gate:

```bash
python3 scripts/aggregate_real_e2e_reports.py \
  --suite full \
  --report .artifacts/real-e2e/cli/report.json \
  --report .artifacts/real-e2e/web/report.json \
  --output .artifacts/real-e2e/aggregate.json
```

The aggregator does not trust the source report's `passed` flag alone. It reloads
the current matrix, verifies the receipt execution ID, observation hashes,
required chains, real Provider traffic, persistence artifacts, UI evidence, and
semantic Judge audit, then calculates coverage from receipt modules rather than
matrix declarations. Duplicate scenario evidence is rejected instead of silently
choosing a convenient result.

A failed attempt is retained in `rejectedResults` but contributes no lanes or
modules. A later independently validated receipt may cover the same scenario
after a fix; the historical failure remains visible without making the suite
permanently impossible to complete. An unreadable receipt, an invalid passing
receipt, or two competing passing receipts remains an aggregation error.

Audit the declared matrix without executing product scenarios:

```bash
python3 scripts/real_e2e_coverage.py --suite smoke
python3 scripts/real_e2e_coverage.py --suite full
```

The coverage command exits non-zero when a required platform lane is absent or
when fidelity metadata is inconsistent. Filtering the runner by `--scenario` or
`--platform` evaluates that requested slice without falsely claiming that the
whole smoke or full suite passed.

## CI tiers

Public pull-request CI runs the matrix parser, receipt schema, fallback failure
evidence, and central Judge tests without credentials. Real product execution is
defined in `.github/workflows/real-e2e.yml` and requires a controlled self-hosted
macOS runner labeled `nola-real-e2e`; that runner owns the required simulators,
Chrome integration, local signing state, and real-model configuration. Configure
the `NOLA_REAL_E2E_CONFIG_JSON` secret and set the
`NOLA_ENABLE_NIGHTLY_REAL_E2E` repository variable to `1` for scheduled full
runs. Manual dispatch can select `smoke` or `full`. Receipts and redacted logs
are uploaded even when a scenario fails.

The Web product scenario uses `web/.venv/bin/python` and requires the Playwright
browser installation owned by that environment. It never falls back to Xcode's
system Python, because doing so makes the same repository pass or fail depending
on shell path ordering.

The native macOS managed-browser scenario does not use that Python environment. Xcode bundles a
pinned standalone Node runtime plus `playwright-core` into `Nola.app`; tests execute the copy inside
the built product and then cross the Rust `browser_*` adapter. Chrome extension tests remain
separate because they validate the attached, signed-in browser path.

Mobile lifecycle scripts accept `NOLA_REAL_SKIP_ANDROID=1` or
`NOLA_REAL_SKIP_IOS=1` for focused failure reproduction. Xcode wrappers create
isolated DerivedData directories by default. Set a wrapper's documented
DerivedData environment variable only when intentionally reusing one build
within a single test run.

Native provider acceptance is split into independent Android and iOS scenarios.
`NOLA_REAL_MOBILE_PLATFORM=android|ios|all` controls the shared wrapper. Android
real-provider tests are force-executed instead of accepting Gradle's
`UP-TO-DATE` result. Both platforms route through the transparent provider probe
and must retain a sanitized `provider_trace`; a passing XCTest/JUnit report with
zero observed upstream requests fails the central judge.

Android device setup is a shared lifecycle contract. The adapter must wait for
`sys.boot_completed=1`, report whether it starts or reuses a device, and validate
the device ABI before Gradle or instrumentation begins. When an adapter owns an
emulator, teardown waits until its serial disappears from ADB before another
scenario can start the same AVD. Sending `emu kill` and returning immediately is
not valid cleanup because it creates a shutdown/start race in sequential suites.

Failure evidence is part of the contract. UI tests must capture the last known
DOM/native accessibility state, visible transcript, timeline, event stream,
provider traffic, durable-store metadata, screenshot when available, and process
log before teardown. A failing test that leaves only a stack trace is itself an
invalid E2E adapter.

Streaming UI timelines must also be monotonic within one assistant delivery.
Once text is visible, a lagging cumulative Core snapshot that is merely a prefix
of the accepted text must not clear and replay the bubble. Genuine divergent
content may replace a projection, but ordinary Part/message-identity transitions
must preserve the stable prefix. The central `ui_progressive` validator rejects
text shrinkage even when the final transcript and screenshot look correct.

As a final safety net, the matrix runner creates a failed `scenario-process`
step when the platform adapter produces no receipt. It retains the process log
and exit metadata without converting infrastructure failure into product success.

## Regression requirements

Every production incident must add a scenario with the same relevant defaults
and persisted-state shape. In particular, multi-tool regressions must not force
`parallel_tool_calls=true`; checkpoint recovery must start from a real persisted
checkpoint; and the next turn must prove the failed task did not leak forward.

Native-client acceptance runs must override their application-support and cache
roots with per-run temporary directories. A test that creates sessions in the
user's production database is invalid even when its assertions pass.

`parallel_tool_calls` is a request preference, not a response-validation rule.
When a provider returns a valid batch of tool calls, the host must forward the
complete batch to Core. The corresponding real-model regression must make a
second, unrelated turn in the same conversation and assert an exact answer with
zero tool calls; this detects both rejected batches and leaked prior intent.

Mocks must cover provider behavior outside requested preferences. A mock that
always obeys `parallel_tool_calls=false` cannot validate recovery from a provider
that still emits multiple calls in one assistant turn.

The smoke matrix must always contain at least one real-model chain for each client
platform: CLI, Web, macOS, iOS, and Android. The full matrix additionally covers
memory, data sources, Skills, compaction, cancellation, permissions, and rendered
structured output.

The macOS full matrix includes an explicit `safe`-mode denial scenario. It
requires exactly one visible permission request, proves the denied tool never
entered execution, and verifies that Core resumes to one persisted assistant
answer. The structured-render scenario requires at least three assistant body
mutations so a final-only Markdown render cannot pass as streaming.

The current product has no independent Backend acceptance lane. Remote execution
acceptance starts an owned Rust Host with an isolated Store and the selected real
model route. Browser/mobile submission, exact approval, committed Run/result,
physical side effects, disconnect/reconnect, cancellation, and cold restoration
must be checked separately. Loopback or adb reverse does not prove real LAN,
Bonjour, firewall, or physical-device background behavior. Standalone prototype
replay is design evidence and never counts as a real-model platform lane.

Real-model assertions verify protocol outcomes, persisted state, tool events,
privacy boundaries, and unique markers. They must not depend on incidental model
wording. Every Agent run must reach exactly one terminal outcome; a run that
remains `running` until the scenario deadline is a product failure, not a test
retry condition.

The normal provider budget is deliberately bounded. Explicit long-running or
academic workflows may raise `NOLA_LLM_HTTP_TIMEOUT_SEC` and the stream timeout
variables at launch; ordinary interactive runs must not inherit an unbounded or
15-minute request stall.

## Integrated CLI and Web acceptance

The full matrix contains two complementary production-runtime scenarios:

```bash
python3 scripts/run_real_system_tests.py \
  --scenario cli-skill-permission-runtime

python3 scripts/run_real_system_tests.py \
  --scenario cli-web-integrated-runtime
```

`cli-skill-permission-runtime` proves the standalone CLI path with a real
provider, Skill activation, permission suspension and resumption, cancellation,
failed-turn recovery, persisted session permission, and debug evidence.

`cli-web-integrated-runtime` starts the release CLI/Web product and verifies the
token session, Core v2 event projection, ordered message persistence, validated
conversation data sources, recommended write policy, destructive denial,
cross-conversation memory, built-in data-analysis Skill artifacts, manual and
automatic compaction, background-task lifecycle, and provider traffic. Set
`NOLA_REAL_WEB_ONLY=1` only for focused Web failure reproduction; it is not a
substitute for the matrix scenario.

Core v2 assistant output is a stable Part projection, not an append-only delta
stream. E2E adapters must fold `core_part_updated` by stable `part_id`, preserve
first-appearance order, and replace each Part with its latest text. Depending on
the transport, the serialized Part discriminator can be `kind` or `type`.
Concatenating every update duplicates text; reading only legacy
`assistant_delta` produces an empty answer and can create duplicate persisted
assistant messages.

Assertions distinguish product contracts from model phrasing. Unique dynamic
markers, tool terminal outcomes, durable state, artifact contents, and permission
effects are hard requirements. Markdown wrappers, explanatory prose, or a final
newline are not failures unless the product contract explicitly requires exact
bytes. Trace sequence is checked as strictly ordered and unique within each run;
filtered internal events may leave valid gaps.

External execution units have a separate real-process contract test:

```bash
cd cli
NOLA_RUN_REAL_EXTERNAL_EXECUTOR_E2E=1 \
NOLA_EXTERNAL_EXECUTOR_BACKENDS=codex,claude_code,opencode \
NOLA_CODEX_MODEL=gpt-5.6-sol \
./tests/real_external_executor_e2e.sh
```

It performs a first turn and a native-session resume for every installed executor.
The persisted Goal restart scenario is `test_goal_resume` in
`scripts/test_real_cli_agent.py`; it must use a real provider and a new CLI process,
not an in-memory runtime reset.
