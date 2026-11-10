> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Data Source Centered Architecture

Status: product/ownership target with source alignment on 2026-11-08. The conceptual fields below are not a claim that every external tool result already travels through one unified DataSource wire object. The actual configuration/validation/override use case is [Application data_source.rs](../../../../core/nola-application/src/data_source.rs); the Host supplies physical reads, freshness and resource facts. Authentication, source enablement, read authorization and result disclosure are distinct.

Memory owner isolation and typed automation definitions have separate contracts: [memory gap](MEMORY_SCOPE_V2.md), [current automation](AUTOMATION_CROSS_PLATFORM_CONTRACT.md). A data-source-centered product direction does not close either runtime boundary by itself.

Nola treats every external or local input as a governed data source before it
becomes chat context, an agent tool, a scheduled-task input, an insight, a
memory, or a notification. Data Center is the user's perception and data
control surface, not a permission dashboard.

## Product Responsibilities

Data Center must answer four questions:

1. What can Nola currently perceive?
2. Is that information authorized, fresh, complete, and readable?
3. Which conversation, Skill, task, answer, or memory used it?
4. How can the user change, retest, disable, or delete it?

The primary flow is:

`Data source -> Skill -> scheduled or interactive run -> artifact/insight -> memory or notification`

Chat orchestrates that flow. Data Center configures and explains it.

## Core Model

A data source describes where information comes from, how it is read, and what
Nola is allowed to do with it.

- `id`: stable source identity.
- `kind`: file, folder, database, URL, web search, API, health, calendar,
  reminder, MCP, uploaded document, web host, or custom.
- `location`: path, URL, host name, database name, or another source locator.
- `capabilities`: readable, searchable, watchable, schedulable, writable,
  executable, contextual.
- `privacy`: local storage, summary-only, cloud context allowed, agent tools
  allowed, write-back allowed, confirmation required.
- `accessPolicy`: default enabled, conversation opt-in, ask when relevant, or
  blocked.
- `connectionState`: validating, ready, disabled, needs permission,
  unavailable, schema changed, or error.
- `freshnessPolicy`: maximum acceptable age and refresh behavior.
- `lastValidatedAt` and `lastSyncedAt`: latest successful contract test and
  refresh times.

## Global Defaults and Conversation Overrides

Every source has a global access policy:

- `defaultEnabled`: available to every new conversation.
- `conversationOptIn`: off by default and exposed in the conversation inspector.
- `askWhenRelevant`: the agent may propose enabling it for a concrete task.
- `blocked`: unavailable to conversations and autonomous runs.

A conversation stores its own source override and the override survives app
restart and conversation switching. The conversation inspector shows only the
sources relevant to that conversation: currently enabled, available to enable,
suggested by Nola, and actually used. It does not duplicate the full Data Center.

The inspector must distinguish temporary activation, activation for this
conversation, and changing the global default. Each used source displays its
last read time, freshness, and reason for use.

## First Connection Validation

Adding a source is not successful merely because authentication or an OS
permission request succeeded. The first connection must execute a bounded,
read-only validation:

1. Authenticate or acquire platform permission.
2. Perform the smallest real read that proves usable data is available.
3. Capture a bounded sample and infer its schema.
4. Identify stable IDs, timestamps, pagination, incremental cursors, and update
   behavior.
5. Detect sensitive fields and apply redaction rules.
6. Validate the normalized output consumed by context assembly.
7. Persist the resulting source contract and mark the source ready.

A source that returns zero records is not automatically valid. The validator
must distinguish an empty time range from missing read permission, an incorrect
query, an unsupported device, or a broken connector.

## Data Source Contract

Successful validation creates a reusable contract. The contract stores how to
read and interpret the source, not an unbounded copy of the user's initial data.

```text
DataSourceContract
  identity and connector version
  access policy and granted scopes
  reader definition and bounded query templates
  normalized schema and field semantics
  stable ID, timestamp, pagination, and incremental cursor rules
  redacted format sample and sample summary
  freshness, caching, and retry policies
  latest validation result and compatibility version
```

Raw validation samples remain local, bounded, encrypted with the application's
local storage policy, and replaceable. A redacted format sample may be retained
for diagnostics. Sensitive values, credentials, health records, and document
contents must not be embedded in the contract.

## Runtime Flow

1. `DataSource`: catalog entry, policy, and contract.
2. `Connector`: platform-specific reader or tool adapter.
3. `Validation`: real read plus schema and permission verification.
4. `Snapshot`: bounded read result with count, cursor, timestamp, provenance,
   and token estimate.
5. `Index`: searchable representation for retrieval and memory search.
6. `ContextPack`: selected snapshots assembled for a chat turn or task run.
7. `Consumer`: chat, Skill, scheduled operation, insight, notification,
   write-back, or long-term memory.
8. `UsageRecord`: source, fields, purpose, consumer, time, and disclosure scope.

Before every read, the runtime checks the global policy, conversation override,
authorization, freshness, and contract version. A schema mismatch stops normal
injection, marks the source as `schemaChanged`, and requires revalidation rather
than silently passing malformed content to the model.

## Data Center Information Architecture

- **Overview**: source health, stale data, failed syncs, running tasks, and
  actionable anomalies.
- **Sources**: source gallery and detail pages for connection, defaults,
  validation, schema, freshness, and deletion.
- **Flows**: visible source-to-Skill-to-task-to-output-to-memory relationships.
- **Insights**: derived trends and conflicts with traceable provenance.
- **Usage and privacy**: which consumer read which source, cloud disclosure,
  memory writes, revocation, and deletion.

Avoid vanity metrics and raw database dashboards. Every warning or metric must
lead to an understandable action.

## Platform Contract

- Shared Core owns source identity, policy semantics, validation results,
  normalized schema, freshness decisions, provenance, and context selection.
- Platform hosts own OS permission APIs and concrete reads such as HealthKit,
  Health Connect, files, calendars, Chrome messaging, and platform storage.
- Platform-specific capabilities may differ, but serialized policies and
  validation states must retain the same meaning across clients.
- A host cannot report a source as ready until it has passed the shared
  validation contract.

## Privacy Rules

Raw samples must not leave the device unless the source explicitly permits
cloud context. Health data defaults to summary-only local handling. Disabling a
source immediately prevents new reads; deleting it removes its credentials,
contract, cached snapshots, index entries, and future scheduled use while
retaining only minimal auditable tombstones when required.

## macOS Acceptance Gates

The macOS implementation is not considered ready from static screenshots or mock-only tests. Its release gate combines the following independent checks:

- `node scripts/validate_mac_localizations.js` validates the critical Data Center and conversation-source copy in English, Simplified Chinese, Traditional Chinese, Japanese, Korean, French, German, and Spanish. It also rejects format-placeholder drift.
- `MacVisualSnapshotHarness` renders the real SwiftUI page in an AppKit window without Screen Recording permission. Set `NOLA_CAPTURE_PAGE=data-center`, `NOLA_CAPTURE_WIDTH`, `NOLA_CAPTURE_HEIGHT`, `NOLA_CAPTURE_APPEARANCE`, `NOLA_CAPTURE_UI`, and `NOLA_CAPTURE_EXIT=1` to exercise regular and compact widths in light and dark appearances. The harness injects isolated data-source stores so it never reads or mutates the user's production registry.
- `MacWebSourceE2ETests` proves that the first successful real read persists the source contract, detected format, observation time, sample count, and policy; failed reads must not create a usable contract.
- `MacBuiltInSkillEndToEndTests.testAutomationSkillEndToEnd` proves the source-to-skill-to-scheduled-automation path and provenance recording.
- `scripts/test_real_mac_agent_host.sh` proves that source context crosses the native macOS runtime and the Agent Host boundary into a real configured LLM request.

A platform promotion must preserve the same product contract: global defaults, conversation overrides, first-read validation, explicit provenance, localization coverage, responsive visual evidence, and a real-provider context-injection test.

### Real source-to-automation acceptance

`scripts/test_real_mac_agent_host.sh` can exercise the complete production
path by setting `NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL` and
`NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN=1`. The acceptance run must:

- perform a real first read before persisting the source;
- save a usable connector/schema contract with a redacted sample;
- inject the verified source into a conversation and record provenance;
- execute an expired one-shot refresh and clear its schedule;
- let the real model select the automation orchestration tool;
- pass the active provider credentials through the protected Host request;
- load the selected Skill, consume the refreshed source, write and validate
  the artifact, and persist a successful activity receipt;
- restore all user state and remove acceptance artifacts after completion.

The harness uses an isolated source registry and store. It never treats a
compiled XCTest target, a mock-only provider, or a successful source refresh
alone as proof of the end-to-end contract.

## Cross-platform acceptance evidence (2026-07-25)

The data-source contract is considered complete only when a source passes a real
read before persistence, records its connector and schema, follows default or
conversation-level policy, reaches the model context, and leaves a disclosure
record. Static presence of UI or model types is not sufficient evidence.

### macOS

- `MacDataSourceRegistryTests`: policy, validation contract, conversation override,
  failed revalidation, usage history, and restart persistence.
- `MacWebSourceE2ETests`: real localhost HTTP read, first-read persistence gate,
  context injection, refresh, and failed-source orchestration rejection.
- `MacBuiltInSkillEndToEndTests.testAutomationSkillEndToEnd`: data source, Skill,
  and scheduled automation execute as one chain.
- `MacSecondaryPageVisualEndToEndTests.testDataCenterRendersInsideARealWindowAtRegularAndCompactWidths`:
  the data center renders in real AppKit windows at regular and compact widths.
- `scripts/test_real_mac_agent_host.sh`: the built app used the real configured
  provider through Agent Host, executed visible file tools, refreshed a real HTTP
  source, preserved the stable stream prefix, hid internal control tools, and
  terminated the Host after the turn.
- `node scripts/validate_mac_localizations.js`: 93 critical keys across 8 locales.

### iOS

- `tests/apple/ios-ui/CoreUserFlowTests.testDataCenterShowsValidatedSourceContractPolicyAndUsage`:
  the real app exposes source status, saved contract, policy editor, and local/cloud
  usage disclosure.
- `scripts/test_real_mobile_providers.sh`: the validated-source fact reached the
  real configured model, the response contained the unique fact, and disclosure
  was recorded. The generic streaming and endpoint-normalization cases also passed.

### Android

- `AndroidDataSourceRepositoryTest`: real Ktor HTTP exchange, JSON/HTML inspection,
  sensitive-field redaction, policy/cloud filtering, expiry refresh, failure
  exclusion, and Core-used-ID audit mapping.
- `DataWorkspaceUiTest`: four Compose device tests cover access, validated source
  state, the editor contract, and responsive navigation.
- `scripts/test_real_mobile_providers.sh`: two real-provider tests passed, including
  validated-source context and disclosure.
- `node scripts/validate_android_localizations.js`: 74 data-center keys across 7
  locale catalogs with key, empty-value, and placeholder parity checks.

### Core, CLI, and Web

- `cargo test --quiet data_source` in `core/`: six data-source contract tests passed.
- `cargo test --quiet data_source` in `apps/cli/`: nine persistence, policy, Web route,
  and localization tests passed.
- `scripts/test_real_cli_web.sh`: real CLI streaming, terminal scrollback, Esc
  cancellation, session permissions, tool recovery, Web search, large-file
  planning, LLM compaction, background tasks, Goal resume, Web login/token session,
  persisted messages, traces, and post-compaction continuation passed.
- `web_search` always collects the top 20 ranked candidates. If a page blocks
  server-side extraction, the ranked title, URL, and search snippet remain clearly
  labeled degraded evidence instead of being discarded or presented as parsed body
  text. The original query is never rewritten inside the tool.

### Repeatable commands

```bash
node scripts/validate_mac_localizations.js
node scripts/validate_android_localizations.js
cd core && cargo test --quiet data_source
cd apps/cli && cargo test --quiet data_source
./scripts/test_real_mac_agent_host.sh
./scripts/test_real_mobile_providers.sh
NOLA_SKIP_CLI_BUILD=1 ./scripts/test_real_cli_web.sh
```
