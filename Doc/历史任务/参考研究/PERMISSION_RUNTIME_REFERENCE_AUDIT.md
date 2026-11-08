> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Permission Runtime Reference Audit

## Scope

This audit compares the checked-in Codex, Claude Code, OpenCode, and Nola permission systems. It
focuses on runtime behavior rather than labels in a settings screen.

## Reference strengths

### Codex

- Approval is a typed lifecycle with a stable approval ID, owning Turn, concrete command/files,
  working directory, reason, parsed command, and the exact decisions a client may present.
- Policy review can be resolved by configuration hooks, an automated guardian, or the user, and
  telemetry records the actual decision source.
- Session approval, executable-policy amendments, network-policy amendments, extra filesystem
  grants, denial, abort, and timeout are distinct outcomes instead of overloaded booleans.
- Sandbox denial and approval are separate concepts: the runtime first attempts the constrained
  operation, then asks only for the missing capability when policy permits escalation.

### Claude Code

- A permission decision retains its rule source and human-readable reason, which makes the UI and
  debugging explain why an operation was allowed, denied, or prompted.
- Shell authorization uses parsed commands and different matching strictness for allow versus
  deny/ask. Wrappers, redirections, compound commands, substitutions, and xargs-style execution are
  handled as security semantics rather than string decoration.
- Rule updates explicitly name their destination and persist only at the intended scope.
- Permission modes and dangerous bypass behavior are represented separately from ordinary rules.

### OpenCode

- One central service owns pending requests and suspends the caller on a deferred result.
- `always` appends approved patterns, immediately re-evaluates every pending request in the same
  session, and resolves every request now covered by the new rule.
- Rejecting a request clears the remaining pending requests for that session, preventing a chain of
  stale prompts after the user has stopped the operation.
- Pending requests can be listed and replayed by UI clients without reconstructing them from tool
  status text.

## Nola strengths

- Core derives authorization from the concrete tool input, never from keywords in user text.
- Product, user-global, Host, conversation, and Turn rule sources are typed.
- Last matching rule wins, and only live interactive input may reuse conversation/Turn authority.
- The recommended profile allows reversible work and asks before concrete irreversible actions.
- Shell session grants reject compound commands, substitutions, newlines, and unsafe quoting.
- Conversation rules are checkpointed while Turn rules remain ephemeral.
- Permission resolution is part of the Core effect journal, so a late duplicate Host response is
  rejected.

## Root cause fixed

Before this change, one model response could create multiple permission effects before the user
answered any of them. Selecting "allow for this conversation" appended the correct Core rule but
only resumed the selected tool. Already-pending calls were not re-evaluated, so clients continued
to display prompts that had become obsolete.

Core now performs the same convergence step as OpenCode:

1. append the conversation rule;
2. re-derive the concrete resource for every pending tool in the active Turn;
3. evaluate it against the updated Core rule set;
4. close each matching `permission.ask` effect;
5. mark those tools pending again and run the normal scheduler;
6. emit resolution updates so every client dismisses the stale request.

The behavior belongs in Core. Mac, CLI, Web, iOS, and Android must not implement independent
popup-deduplication rules.

## Remaining gaps

Nola now carries a typed `PermissionEvaluation` from Core policy into the Host permission request.
It includes the effective decision, a stable reason code, rule index, matched rule, and rule source.
This distinguishes explicit policy, recommended-profile confirmation, interactive grants excluded
from automated origins, and absence of any matching rule without parsing user-facing strings.

1. Store pending permission requests as a first-class projection so reconnecting clients can list
   them without scanning tool parts.
2. Distinguish denial, user correction/custom response, timeout, cancellation, and Host failure in
   the public protocol.
3. Replace remaining CLI-era wildcard rules with Core-issued structured amendments.
4. Move shell parsing from a conservative token helper to a reusable parsed-command contract while
   keeping deny/ask matching stricter than allow matching.
5. Add cross-platform contract fixtures proving identical decisions for multilingual prompts,
   ambiguous tool names, scheduled input, replay, delegated work, compound shell commands, and
   matching requests already pending in parallel.
## Implemented convergence: durable pending permission projection

Core now checkpoints the authoritative pending-permission projection alongside
tasks, memories, and the conversation projection. Each request retains its
stable request/Turn/call identity, concrete Core-derived resource, original
arguments, rationale, and typed policy evaluation. Hosts can therefore restore
the same permission UI after reconnecting without scanning message bubbles,
re-running policy, or opening duplicate sheets.

The projection is updated at the same lifecycle boundaries as the Host effect
journal: request creation, direct resolution, conversation-rule fan-out, Turn
cancellation, and removal of an ephemeral sensitive Turn from a checkpoint.
## Implemented convergence: Core decides, Host presents

The Core Host bridge no longer reclassifies Bash commands, recomputes the
resource scope, or applies the CLI's legacy Turn-wide denial cache after Core
has emitted a permission request. Core owns the concrete authorization
decision and auditable request lifecycle. The Host presentation layer now only
waits for a user/background-task response and returns that response against the
same stable request ID and exact Core resource.
