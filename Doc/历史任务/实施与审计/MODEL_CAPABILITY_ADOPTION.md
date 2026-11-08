> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Model Capability and Stream Contract Adoption

This document turns provider-specific behavior into a Core-owned contract. It
adopts the strongest patterns from Codex, OpenCode, and Claude Code without
putting provider-name checks or language-specific routing into Core.

## Problem statement

Nola previously treated a model invocation as messages, tools, and opaque
settings. Each Host then inferred native tools, reasoning behavior, context
window, prompt cache, and continuation rules independently. The same configured
model could therefore behave differently on CLI, Web, macOS, iOS, and Android.

The dangerous cases are not cosmetic:

- signed or encrypted reasoning state can be lost between requests;
- a streamed tool call can disappear if only the final response is parsed;
- a retry after partial tool output can execute an action twice;
- cache identity can describe different tools than the provider actually sees;
- context compression can trigger at different points on different clients;
- a tool marked parallel-safe can still execute serially or cross a permission
  barrier incorrectly.

## Reference strengths

### Codex

- Central model metadata describes context limits and supported behavior.
- Request continuation such as `previous_response_id` is explicit.
- Tool and response events use stable typed identities.

### OpenCode

- Provider metadata is preserved with reasoning/content parts.
- Provider transforms normalize wire differences into one session model.
- Session processing keeps tool, reasoning, text, and terminal state separate.

### Claude Code

- Safe tools use bounded concurrency; unsafe tools and permission prompts form
  barriers.
- Partial tool-call output is not silently retried through a different transport.
- Prompt-cache break detection covers policy, tools, model, effort, and request
  features rather than one arbitrary string.

## Core-owned capability profile

The Host discovers facts; Core freezes and reasons over a typed profile.

```text
wire_protocol
native_tool_calls
parallel_tool_calls
reasoning_mode
reasoning_continuity
prompt_cache
request_continuation
context_window
maximum_context_window
effective_context_percent
reserved_output_tokens
input_modalities
stream_capabilities
```

Unknown values must conservatively disable optional behavior. Core must not infer
capabilities from the provider name, model name substring, or user request.

## Core-owned invocation plan

Each model Step should freeze:

```text
model_id
capability_profile_digest
tool_projection
tool_choice
parallel_tool_calls
reasoning_policy
cache_policy
model_prefix_digest
context_budget
continuation_state
stream_attempt_id
```

The Provider adapter maps this plan to actual API fields. It must not widen the
tool set, change reasoning policy, or invent a new cache identity.

## Reasoning continuation state

Reasoning needs display text and non-display provider state.

```text
provider_namespace
model_id
response_id
block_id
integrity_kind
opaque_state
```

Requirements:

- state survives checkpoint and restore;
- the UI never renders opaque state;
- provider adapters receive it unchanged when the capability profile requires
  continuation;
- compaction explicitly preserves, replaces, or clears it;
- unknown opaque fields are retained, not interpreted by Core;
- state from one provider/model namespace is never sent to another.

## Normalized stream lifecycle

```text
ModelStreamStarted(attempt_id, response_id?)
TextDelta(part_id, sequence, text)
ReasoningDelta(part_id, sequence, text)
ReasoningMetadataPatch(part_id, provider_state)
ToolCallStarted(call_id, name)
ToolArgumentsDelta(call_id, sequence, bytes)
ToolCallCompleted(call_id)
UsageUpdated(usage)
ModelCompleted(attempt_id, finish_reason)
ModelStreamFailed(attempt_id, retryability, partial_state)
```

Core owns stable IDs, sequence validation, replay deduplication, assembly, and
terminal validation. UI clients project normalized Parts only.

## Retry invariants

- A retry has a new `stream_attempt_id`.
- Replayed deltas for an existing attempt are ignored by sequence.
- Once a tool call has emitted executable arguments, fallback to a transport
  that may repeat the call is forbidden unless Core explicitly abandons the
  attempt before execution.
- A completed/failed/cancelled attempt cannot accept new deltas.
- Final tool calls must match the current Step tool projection.

## Parallel tool invariants

- Only `ConcurrentSafe` calls may overlap.
- Permission requests, destructive calls, workspace mutation, and declared
  barriers remain serial.
- Each call has independent cancellation and timeout state.
- Host results may arrive out of order.
- Core commits conversation state deterministically by call identity and planned
  ordering, not completion timing.

## Cache signature

The semantic cache signature covers at least:

```text
stable Core prompt digest
exact Step tool schema digest
model capability profile digest
reasoning policy and effort
cache policy version
provider-visible stable request features
```

The Host may namespace this signature for a provider/model, but must not rebuild
the semantic inputs.

## Context-window consistency

Core freezes both the advertised maximum and effective working window, together
with source and confidence. Every Host displays and compacts against the same
snapshot. A silent `128K` fallback is allowed only as an explicitly identified
unknown-capability fallback, not as model truth.

## Acceptance matrix

1. Fragmented OpenAI tool arguments assemble identically on every Host.
2. Interleaved parallel calls retain stable call IDs and independent arguments.
3. DeepSeek reasoning content is returned when the profile requires it.
4. Signed/encrypted reasoning survives checkpoint without entering visible text.
5. Stream interruption cannot duplicate an already emitted tool action.
6. Concurrent-safe tools overlap in wall time; unsafe tools remain barriers.
7. Context window and compression threshold match across all clients.
8. Cache signature is stable for ordering noise and changes for semantic inputs.
9. Cached input, reasoning, and output usage remain non-zero when supplied.
10. Unknown model capability safely disables optional features.
11. macOS MLX façade produces the same normalized lifecycle as API providers.
12. Provider/model switches reject incompatible continuation state.

## Prohibited shortcuts

- Provider/model substring checks in Core.
- Tool routing from words in the user request.
- Reconstructing opaque reasoning state from visible text.
- Waiting for final text before exposing a native tool call.
- Treating a batch label as proof of concurrent execution.
- Retrying after partial tool output without an explicit abandonment decision.
- Different context-window truth on different clients.
