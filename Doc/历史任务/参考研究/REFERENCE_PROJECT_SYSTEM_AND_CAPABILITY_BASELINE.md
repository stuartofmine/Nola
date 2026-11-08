> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Reference Projects: System Prompt and Capability Baseline

Last updated: 2026-08-05

## Purpose

This document compares Nola with the checked-in reference implementations under
`参考资料/`. It separates model guidance from runtime enforcement. A prompt that
describes a rule is not evidence that the product enforces it; a similarly named
module is not evidence that every Host consumes it.

## Executive assessment

Nola's differentiator is not another coding-agent shell. It is a local-first,
cross-platform personal agent whose data sources, long-term memory, scheduled
work, Skills, MCP capabilities, and device actions share one durable execution
model. The reference projects are stronger in narrower areas:

| Project | Strongest design trait | What Nola should absorb |
| --- | --- | --- |
| Codex | Core-enforced tool, permission, lifecycle, and event protocols | Canonical lifecycle contracts, trusted hooks, deterministic UI event projection |
| Claude Code | Modular prompt fragments, explicit completion rules, rich hooks, prompt-cache discipline | Small policy fragments, hook matching, digest-only stable-prefix diagnostics, non-black-box task progress |
| OpenCode | Provider-aware prompt selection, Deferred permission interaction, event bus, clean Skill discovery | Recoverable permission protocol, provider overlays, capability discovery without keyword routing |
| Open WebUI | Knowledge collections, access grants, ingestion visibility, multi-user administration | Data-source observability and provenance; not its server-first ownership model |

## 1. System prompt comparison

### Codex

Codex keeps its main prompt relatively compact. Important behavior is enforced
outside the prompt by its tool registry, sandbox, approvals, item lifecycle,
compaction, and session protocol. The prompt primarily describes working style,
scope, completion behavior, and communication.

Nola already follows this direction through Core capability projection,
permission evaluation, canonical Parts, context preparation, and model-family
profiles. The remaining risk is Host drift: a rule implemented only in one
client is not a Nola rule.

### Claude Code

Claude Code composes many focused fragments instead of maintaining one monolithic
persona prompt. Representative strengths are:

- explicit action safety and truthful result reporting;
- investigate-before-asking behavior;
- parallel independent tool calls;
- persistent memory guidance;
- strict stable-prefix prompt-cache ordering;
- lifecycle hook instructions and tool-result feedback.

Nola now has matching Core fragments for semantic tool selection, truthful
outcomes, permission feedback, external retrieval, memory evidence, conversation
continuity, investigation before clarification, and explicit task completion.
Host-specific identity and workflow are separate trust classes: concrete
surface identity is a `host_fact`, while supplemental workspace or surface
workflow is a bounded `host_instruction` rendered below Core policy with an
explicit non-authorization boundary. Ordinary Host workflow text is no longer
labelled product `policy` merely because it participates in the stable prefix.
This is enforced by the native ABI and CLI Host composer: caller-supplied
`policy` fragments fail with a structured error instead of being promoted,
the default public Rust composer is Host-safe, and native Swift/Kotlin Host
trust enums do not expose a `policy` case.
The next improvement is not more prose. It is measuring whether each model/Host
obeys the same contract in real sessions.

### OpenCode

OpenCode selects a provider-specific base prompt and keeps permission requests as
recoverable suspended operations. This is preferable to forcing every model
through identical wording or turning permission denial into terminal failure.

Nola has model-family overlays and resumable Core permissions. It must retain the
rule that prompt text never routes a capability by language-specific keywords.
The main model chooses from a frozen schema; Core evaluates the concrete call.

### Open WebUI

Open WebUI is not primarily an agent prompt reference. Its strength is the
server-side knowledge and administration plane. Nola should borrow transparent
source status, ingestion history, access grants, and provenance, but not move
personal source ownership or protected-data reads to a central server.

## 2. Prompt architecture status

| Mechanism | Nola status | Evidence or remaining gate |
| --- | --- | --- |
| Stable system prefix + volatile turn suffix | Implemented in Core | `core/src/agent/prompt_composer.rs` |
| Core-only product policy | Implemented in Host composition APIs | Native ABI and CLI reject Host `policy`; bounded compaction uses `host_instruction` |
| Model-family overlays | Implemented | `core/src/agent/model_capability.rs` |
| Explicit completion contract | Implemented | `nola.core.task-completion` fragment |
| Hidden keyword router | Removed from main Agent Host flow | Real-model request-count regression still required |
| OpenAI prompt-cache identity | Implemented | Core prefix digest consumed by CLI/iOS/Android |
| Provider cache telemetry | Core event/Trace contract implemented | CLI Host reports provider evidence; Mac/Web preserve the same usage snapshot and bounded diagnostic, while product-facing presentation remains to be finished |
| Prompt conformance evaluation | Incomplete | Needs multilingual real-model scenario suite across Hosts |

## 3. Functional architecture comparison

### 3.1 Tool execution and Parts

Codex has the clearest separation between model items, tool requests, execution,
approval, and UI events. Nola's Agent Core v2 now follows the same essential
shape: the model emits a call, Core owns lifecycle and permissions, the Host
performs I/O, and canonical Parts drive presentation.

Remaining gaps:

- every Host must render the same canonical Part lifecycle without inventing a
  second local status state machine;
- tool outputs need bounded previews plus durable artifact references;
- unknown or stale tools must return recoverable protocol feedback rather than
  trigger prompt rewriting or a second classifier;
- runtime tests must prove that tool start, permission suspension, resume,
  completion, interruption, and replay preserve order.

### 3.2 Permissions

Claude Code and OpenCode treat permission as a suspended interaction, not a
failed tool. Codex adds strong scope and trust metadata. Nola Core already has
ordered rules, source precedence, conversation grants, pending requests,
origin-aware restrictions, and automatic resolution of matching pending calls.

Remaining work is cross-Host conformance, not another permission engine:

- `allow once`, `allow conversation`, custom response, denial, and restart must
  behave identically on CLI, Web, macOS, iOS, and Android;
- scheduled, replayed, recovered, and delegated input must never inherit an
  interactive conversation grant incorrectly;
- Hooks, Skills, MCP servers, and device bridges may narrow permission but may
  never widen or bypass Core policy.

### 3.3 Skills

Claude Code and OpenCode provide strong discovery and progressive loading.
Nola adds built-in document, data, design, research, and automation Skills across
Mac and CLI/Web. The important contract is:

1. discovery exposes bounded metadata;
2. the main model chooses a Skill semantically;
3. loading the Skill adds trusted instructions and declared resources;
4. tools remain separately permissioned;
5. completion produces a validated artifact or an explicit blocker.

Keyword-based Skill forcing is prohibited. End-to-end tests must use real model
selection and verify the produced artifact, not merely assert that a Skill name
appeared in a prompt.

Codex's plugin trust model also exposed a local Nola weakness: macOS activation
was previously bound only to a Skill or MCP configuration path. Local Skill and
per-server MCP identities now include a bounded content SHA-256 digest, so a
modified file cannot silently inherit the prior activation. A full plugin
marketplace manifest is intentionally deferred until Nola has a real third-party
installation workflow; adding one earlier would expand the attack surface
without improving current built-in discovery.

### 3.4 MCP

Nola supports MCP tools, static resources, prompts, and now parameterized
resource templates. Resource and prompt catalog operations are classified as
read-only investigation and have localized CLI presentation.

Remaining gates:

- exercise a real template-capable MCP server through CLI/Web and macOS Host;
- render the server, operation, bounded arguments, result preview, and error
  without exposing credentials;
- handle catalog changes and stale model tool calls as recoverable events.

### 3.5 Memory

Nola is architecturally ahead of the coding-agent references in personal memory:
it has namespaces, scope, provenance, confidence, retention, selection, usage
ledger, extraction jobs, and consolidation. Claude Code's persistent memory and
Open WebUI's knowledge stores reinforce two principles Nola must preserve:

- memory is evidence with provenance, not an invisible prompt append;
- compaction and consolidation must preserve the transcript and expose what was
  summarized, retained, superseded, or rejected.

The remaining proof is a cross-session, cross-process, real-model recall suite,
including contradictory updates and sensitive-memory exclusion.

### 3.6 Context management and prompt caching

Codex provides the strongest canonical item/session model. Claude Code provides
the clearest byte-stable cache-prefix discipline. Nola already centralizes
context preparation and compaction planning in Core, and separates ephemeral
turn context from durable checkpoint state.

Remaining work:

- expose provider-reported cache read/write tokens and cache-break reasons;
- show one consistent total-window usage projection before, during, and after a
  response on every client;
- prove compaction preserves complete tool transactions and reasoning
  continuation fields for providers that require them.

### 3.7 Data sources

Open WebUI is stronger at collection administration; Nola is stronger at
local-first protected-data access. Core already owns access policy, validated
reader contracts, freshness, consent, refresh decisions, and usage provenance.
CLI/Web already consume that resolver.

A concrete publication defect was found: the Rust data-source ABI existed but
was absent from the Swift header and Android JNI. The bridge is now exposed with
typed wrappers and contract tests. Core ABI v3 also seals validation contracts,
so clients cannot accidentally use a different digest material. macOS delegates
contract sealing, policy, freshness, and usability to Core; iOS delegates system
source conversation enablement; Android delegates contract sealing, refresh,
and final injection decisions. Hosts continue to own protected-data permission,
credentials, I/O, cloud-disclosure constraints, persistence, and presentation.
Custom URL-source policy surfaces and native first-connect flows still need
cross-platform conformance evidence.

### 3.8 Scheduling and long-running work

Nola's scheduled tasks and background work are core product capabilities absent
from most coding-agent references. They should use the same plan, capability,
permission, source, memory, and artifact contracts as interactive chat.

Required invariants:

- background execution is selected only for genuinely long-running work;
- a durable task handle exposes progress and can be inspected or cancelled;
- non-interactive origin never silently inherits interactive permission;
- intermediate results are visible in the conversation and recoverable after a
  process restart.

### 3.9 Lifecycle hooks

This is a material Nola gap. Codex and Claude Code support trusted lifecycle
hooks around session, user prompt, tool use, notifications, and completion.
Nola now has a Core-owned, observation-only planning and execution-projection
contract with exact canonical matching, source and trust checks, deterministic
ordering, bounded timeouts, stable invocation IDs, and explicit Host capability
requirements. The canonical Trace pipeline represents Hook start and terminal
states, and the CLI/Web Host driver emits prompt, tool, error, and turn-end
events through that pipeline. Core now also builds a fail-closed registry
snapshot: every accepted rule requires an exact trusted Host handler binding
with matching kind and source, and the executable registry has a stable digest.
Lifecycle Hook ABI v3 exposes that same registry builder to Swift and Android
Hosts instead of allowing clients to duplicate the validation contract.
The persistent CLI/Web driver emits `session_start` before the first user-prompt
Hook and deduplicates it independently from Turn lifecycle state.
No production configuration loader supplies that registry yet, and native
Hosts do not execute the contract, so this remains runtime groundwork rather
than a complete user-facing Hook feature.

The checked-in implementations expose materially different trust models:

- Codex discovers `hooks.json` and makes bypassing Hook trust an explicit,
  dangerously named operator choice. The useful property is configuration
  provenance and trust visibility before execution, not arbitrary commands.
- Claude Code validates typed synchronous and asynchronous Hook output, streams
  progress, propagates cancellation, and bounds execution with timeouts. Its
  Hooks can also block continuation, rewrite tool input/output, inject context,
  and change permission decisions.
- OpenCode plugins receive mutable output objects at model, prompt, permission,
  shell, and tool boundaries. This is flexible, but permits a plugin to modify
  system text or arguments outside the normal Agent decision path.

Nola adopts provenance, typed progress, cancellation, timeout, and terminal
result semantics. It deliberately does not adopt permission mutation, system
prompt mutation, tool-input replacement, or tool-output replacement in Hook
schema v1. Those operations would invalidate the frozen Turn snapshot and
create a second authorization path. Any future mutating protocol requires a
new Core schema, an explicit capability, and ordinary permission evaluation.

Adopt a Core-owned hook protocol with Host execution:

- events: `session_start`, `user_prompt_submit`, `pre_tool_use`,
  `post_tool_use`, `tool_error`, `turn_end`, `session_end`;
- scopes: managed, product, user, workspace;
- handlers: command, HTTP, prompt, and internal action, restricted by platform;
- outcomes: observation status, bounded redacted preview, duration, truncation,
  and a stable error code only;
- safety: explicit trust, timeout, output cap, redaction, no recursive hook
  execution, no tool-input or tool-result mutation, no blocking decision, and
  no permission widening;
- presentation: canonical started/completed Parts, deduplicated live status,
  durable audit entries, expandable details.

This should not be implemented as arbitrary scripts embedded in prompts. A
future Host registry must bind trusted handler IDs to platform-owned execution
details; executable commands, URLs, credentials, and prompts must not be
smuggled through Core matcher metadata.

The remaining production acceptance gates are concrete:

- load rules from managed, product, user, workspace, and plugin registries with
  source provenance and a stable digest;
- require an exact Host-owned handler binding for every accepted handler ID;
- surface rejected, modified, unavailable, timed-out, and cancelled Hooks in
  canonical Trace without leaking secrets or raw command output;
- persist `session_start` delivery across crash/replay and emit `session_end`
  exactly once for an observed close; process-local CLI/Web delivery is already
  deduplicated independently from Turn state;
- prove the same event order on CLI-hosted Web and native Agent-Host clients;
- keep iOS and Android free of process Hooks unless a platform-specific trusted
  executor and capability are explicitly provided.

## 4. What Nola should not copy

- Do not adopt server-first ownership of personal data from Open WebUI.
- Do not add language-specific keyword routing or hidden query classifiers.
- Do not expose raw chain-of-thought; show concise, truthful progress and tool
  evidence instead.
- Do not let hooks, Skills, MCP, or a remote Host bypass Core permissions.
- Do not make macOS depend on spawning the CLI process for normal chat.
- Do not duplicate Core policy in every UI ViewModel or Swift service.

## 5. Adoption order

1. Finish remaining custom-source and first-connect resolver consumption, then
   prove native data-source conformance with persisted conversation overrides.
2. Validate MCP resource templates against a real server and expose canonical
   Part presentation on every Host.
3. Validate prompt-cache diagnostics across real provider sessions. macOS,
   iOS, Android, and CLI Web now persist/render the shared Core Trace diagnosis
   and provider-reported read/write tokens; clients do not infer cache hits
   independently. The CLI terminal exposes the same checkpointed diagnosis in
   its wide debug footer only; normal and narrow footers keep context and
   permission information ahead of diagnostics.
4. Connect the observation-only Core Hook contract to canonical Trace Parts on
   each Host; consider gated pre-tool blocking only after permission conformance
   tests.
5. Build a real-model cross-Host conformance suite covering multilingual natural
   follow-ups, ambiguous capability words, unknown-tool recovery, permissions,
   compaction, Skill artifacts, MCP, memory, and scheduled work.

## 6. Completion evidence

The comparison goal is not complete until all of the following are true:

- the matrix above is backed by current source evidence for every reference;
- each adopted mechanism has a Core contract or an explicit Host-only reason;
- every client consumes the relevant Core contract;
- unit/contract tests cover serialization and deterministic policy;
- real end-to-end sessions cover model choice, Host I/O, permissions, UI event
  order, persistence, and produced artifacts;
- product documentation describes the resulting user model rather than internal
  implementation names.
