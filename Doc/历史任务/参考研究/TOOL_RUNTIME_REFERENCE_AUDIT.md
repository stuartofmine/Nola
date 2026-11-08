> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Tool Runtime Reference Audit

## Scope

This note compares the checked-in Codex and OpenCode references with Nola's
Core/Host tool execution boundary. It records runtime contracts, not product
specific routing rules.

## Reference strengths

- Codex separates bounded live execution deltas from the final command result.
  The final result retains raw output metadata and truncation information;
  consumers never need to reconstruct the authoritative result from UI deltas.
- Codex models cancellation as an explicit terminal outcome. A cancellation may
  win before a matching start notification, so consumers cannot assume every
  finish has a visible start.
- OpenCode treats a tool invocation as one stable message Part whose state is
  updated over time instead of appending unrelated status messages.

## Nola contract

Nola now uses three distinct channels:

1. `ToolCallProgress` is correlated by stable `call_id` and updates the existing
   Tool Part in place. Status text replaces the previous status; stdout/stderr
   deltas append only to a bounded 16,384-character UI preview.
2. `ToolCompleted` is the only terminal tool result. It owns the normalized
   output, `full_output_ref`, artifacts, error state, and model-visible result.
3. `HostProgress` remains effect-level progress for work that is not a concrete
   tool invocation, such as model retry or Host acceptance.

Live deltas are presentation data. They are not appended to model context,
conversation items, or the durable tool result. The Host effect lease and
sequence checks reject duplicate or late responses after completion or
cancellation.

## Presenter requirements

- Render progress inside the matching tool card, not in a second global status
  strip.
- Preserve Part identity and layout while its status changes.
- Ignore no-op status replacements to prevent flashing.
- Preserve structured result, preview and human summary as independent fields;
  presentation priority must not erase missing/null/empty distinctions.
- Public `full_output` describes only the message/Part/call owner of a retained
  result. Do not expose or resolve the Host's private `full_output_ref`, or infer
  a reader/permission action from display metadata. A future complete-output
  reader needs its own owner-bound source and policy verification; that public
  handle-to-reader path is not implemented by the current reference DTO.
- Keep artifacts independently addressable and openable after the Turn ends.

## Remaining host work

- Long-running tool implementations should emit stdout/stderr deltas through
  `ToolCallProgress`; synchronous tools currently emit only the structured
  running status and terminal result.
- Platform presenters should localize capability titles without changing the
  stable capability or call identifiers.
