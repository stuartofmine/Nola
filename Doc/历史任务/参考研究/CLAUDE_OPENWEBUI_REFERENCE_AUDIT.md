> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Claude Code and Open WebUI Reference Audit

## Scope

This audit compares checked-in reference behavior with Nola. It focuses on system-instruction architecture and runtime capability contracts, not visual similarity or copying prompt prose.

## Executive findings

1. Claude Code splits safety, task scope, memory, background execution, and cache guidance into independent prompt fragments. Nola has the right Core concepts, but some Hosts still duplicate policy in prose or bypass the shared lifecycle.
2. Open WebUI exposes Skill metadata first and loads full instructions only through an authorized `view_skill(id)` operation. Nola follows this for built-in Skills, but Web custom Skills currently bypass the boundary.
3. The useful design pattern is not a larger system prompt. It is a stable prompt prefix plus typed late-bound state, exact capability identifiers, scoped authorization, durable suspension, and truthful terminal evidence.

## System instruction comparison

### Action safety and truthful outcomes

Claude Code states four separate invariants:

- irreversible or outward-facing actions require confirmation unless the authorization is durable;
- approval in one context does not automatically extend to another;
- destructive targets are inspected before mutation;
- skipped or failed validation must be reported as such.

Nola Core already models input origin, scoped permission grants, tool policy receipts, and canonical terminal events. The remaining gap is Host parity: every client must suspend the same concrete tool call when authorization is missing and must derive success from the Core terminal state rather than an HTTP response, process exit attempt, or optimistic UI update.

Adoption rule: keep these invariants in Core reducers and permission/tool contracts. Prompts may explain them to the model, but must not be their enforcement boundary.

### Task scope

Claude Code explicitly rejects speculative refactors and half-finished abstractions. Nola's broad agent guidance favors autonomy, which is useful, but can lead to unrequested file creation or unnecessary workflow expansion.

Adoption rule: Core and Host should distinguish the requested outcome from optional improvements. A tool plan must be justified by the concrete outcome; a Skill must materially change the workflow or artifact contract. Shared nouns and language-specific keywords are not routing signals.

### Persistent memory

Claude Code evaluates memory along three axes:

- applicable to future decisions;
- durable across sessions;
- legible without the original conversation.

It also treats memories as snapshots that must be checked against current sources. Nola now has structured candidates, provenance, retention, expiry, supersession, and a Core eligibility gate across the native clients. Remaining gaps are:

- automatic consolidation of durable corrections without storing transient task state;
- a visible explanation of which memories affected a response;
- current-state verification before mutable imported or tool-derived facts are presented as current;
- one shared mutation contract across CLI, Web, iOS, Android, and macOS.

Adoption rule: do not inject a monolithic memory string. Select structured records in Core, preserve provenance in the context component, and record usage as a projection that the UI can cite.

### Prompt caching

The Claude reference reduces prompt caching to a byte-prefix invariant and classifies inputs as global, session, turn, or request scoped. It warns that timestamps, UUIDs, conditional tool lists, and nondeterministic serialization near the front invalidate the entire cache.

Nola's Core prompt composer already separates a stable prefix from a dynamic suffix and publishes `stable_prefix_digest`. CLI/Web now derive the main-turn OpenAI-compatible cache identity only from that Core digest plus a provider/model namespace. Explicit configured keys still override it; classifier and compaction requests do not reuse the main-turn identity.

Remaining gaps:

- native iOS, Android, and macOS providers decode the digest but do not consistently bind it to provider cache identity;
- Anthropic breakpoint placement is provider-driven rather than explicitly derived from the Core stability boundary;
- dynamically changing tool schemas can still invalidate a provider prefix even when the textual system prefix is stable;
- cache usage metrics are not projected consistently to debug UI.

Adoption rule: never hash the final rendered prompt to recover a cache key. That prompt contains dynamic context by design. Use only the Core snapshot digest and deterministic provider/model namespacing.

### Background execution

Claude Code gives each background session an isolated job directory so parallel work cannot collide through shared temporary paths. Nola's scheduled Agent contract now separates `scheduled_action` from interactive input and prevents borrowing live-turn grants.

Remaining gaps:

- each scheduled occurrence needs a stable idempotency key and job workspace;
- Android once-only scheduling, local-time recurrence, retry reconciliation, and permission-resume UI remain incomplete;
- job results need canonical trace/result references rather than ad hoc event blobs;
- long-running jobs need durable lease, heartbeat, stale-owner recovery, and ownership-safe finish on every Host.

## Skill comparison

Open WebUI's Skill flow is metadata-first:

1. list or search Skill records;
2. expose name, description, active state, and access metadata;
3. call `view_skill(id)` only when detailed instructions are needed;
4. re-check active state and resource authorization at load time;
5. return the full content only for the selected authorized Skill.

Nola's built-in CLI/macOS flow is close: the model receives a compact catalog and calls one `Skill(skill=<exact id>)` entry point. However, Web custom Skills currently violate the contract:

- the full uploaded object can enter turn context;
- the loader accepts only compile-time built-in IDs;
- a context-index failure can silently fall back to the full uploaded records;
- disabled or removed custom Skills have no unified materialized-directory revocation path.

Required target contract:

- normalize every Skill to a stable exact ID;
- keep catalog entries metadata-only;
- atomically materialize enabled custom Skills under a Host-controlled root;
- resolve full instructions only after an exact `Skill` call;
- re-check enabled/removed state at invocation time;
- return base directory, immutable instruction reference, required tools, validation steps, and activation scope;
- let Core own Skill run state, budget, missing-tool suspension, completion, and replay.

## What Nola should not copy

- Do not convert extracted reference prompt text into one giant Nola system prompt.
- Do not rely on English trigger phrases to select tools, Skills, memory, or Web retrieval.
- Do not make client UI state an authorization source.
- Do not treat a process launch, HTTP response, or generated path as proof that the requested outcome exists.
- Do not expose hidden chain-of-thought. Show concise reasoning summaries, concrete tool calls, parameters, results, and pending decisions instead.

## Evidence

- Claude action safety: `参考资料/claude-code-system-prompts-main/system-prompts/system-prompt-action-safety-and-truthful-reporting.md`
- Claude task scope: `参考资料/claude-code-system-prompts-main/system-prompts/system-prompt-doing-tasks-no-unnecessary-additions.md`
- Claude persistent memory: `参考资料/claude-code-system-prompts-main/system-prompts/system-prompt-persistent-memory-usage-and-writing-guidance.md`
- Claude prompt caching: `参考资料/claude-code-system-prompts-main/system-prompts/data-prompt-caching-design-optimization.md`
- Claude background jobs: `参考资料/claude-code-system-prompts-main/system-prompts/system-prompt-background-session-instructions.md`
- Open WebUI Skill loader: `参考资料/open-webui-0.10.2/backend/open_webui/tools/builtin.py`
- Open WebUI Skill authorization and lifecycle: `参考资料/open-webui-0.10.2/backend/open_webui/routers/skills.py`
- Nola prompt composer: `core/src/agent/prompt_composer.rs`
- Nola capability and Skill runtime: `core/src/agent/capability.rs`
- Nola Core tool/Skill reducer: `core/src/agent/runtime/tool_driver.rs`
- Nola CLI/Web Skill catalog: `cli/src/builtin_skills.rs`
- Nola CLI/Web Skill loader: `cli/src/tools/mod.rs`
