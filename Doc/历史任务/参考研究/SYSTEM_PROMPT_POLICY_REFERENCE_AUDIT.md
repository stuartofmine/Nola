> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# System Prompt Policy Reference Audit

## Problem

Nola previously had several correct behavioral rules inside large CLI and
macOS-specific prompt strings. That made behavior depend on which Host assembled
the request. iOS, Android, Web, local models and future Hosts could omit or
silently diverge from product-wide invariants.

The reference projects demonstrate that the durable advantage is not one giant
prompt. It is a set of small policy modules selected for a request snapshot:

- Claude Code separates action safety, truthful reporting, clarification,
  correction and execution guidance into independently maintained fragments.
- Codex separates base instructions from typed turn/step context and treats
  actual tool results as the evidence for user-visible claims.
- OpenCode assembles provider, model, agent, tool and session material at the
  request boundary instead of using UI text as runtime policy.
- Open WebUI gates exposed capabilities before request construction rather than
  relying on prompt prose to hide unavailable tools.

## Core-owned policy

`compose_agent_prompt` now injects four immutable, cacheable fragments before
composing Host-supplied fragments:

| Fragment | Contract |
|---|---|
| `nola.core.action-scope` | Chat generation is not file or external-action authorization; approval remains concrete and scoped. |
| `nola.core.truthful-outcomes` | Claims about tools, files, memory, retrieval and validation require a successful observed result. |
| `nola.core.semantic-tool-selection` | Tool choice uses semantic intent, schemas and availability; capability words are not routing or authorization rules. |
| `nola.core.investigate-before-question` | Resolve answerable ambiguity with read-only evidence before interrupting, without turning investigation into side-effect authority. |

These IDs are reserved by construction. A Host that supplies the same ID gets a
duplicate-fragment error instead of shadowing product policy. The fragments are
part of the stable prefix and therefore part of its SHA-256 cache identity.

## Boundaries

Core policy does not decide whether a concrete action is allowed. The model may
request a tool, but Core still evaluates the concrete call, resource, origin and
grant. Host capability snapshots still decide what can execute. UI labels and
localized strings never become permission rules.

Platform prompts remain responsible for:

- product identity and platform-specific interaction language;
- actual Host capability facts for the current Turn;
- selected response style and user-configured instructions;
- typed Skill, memory, data-source and attachment context.

They must not duplicate or weaken the Core fragments above.

## Regression cases

The Core composer covers the following failures directly:

1. A user asks for a code example: generating the example is allowed, silently
   writing a file is not.
2. A write or test fails: the response reports the failure rather than claiming
   completion.
3. A prompt mentions “search”, “memory”, “Skill” or “permission” while asking an
   architecture question: the words alone do not force a tool.
4. A question can be resolved from available read-only evidence: investigate
   before asking a broad clarification.
5. A Host attempts to replace a reserved Core fragment: composition fails.

## Remaining convergence

- Remove semantically duplicate rules from platform identity strings after
  snapshot tests prove equivalent behavior.
- Make provider prompt-cache telemetry report the Core fragment IDs and stable
  digest without logging prompt content.
- Add real-model cross-platform scenarios for action-versus-content intent,
  failed-tool truthfulness, multilingual capability mentions and ambiguous
  follow-ups.
