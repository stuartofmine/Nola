> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Skill Runtime Reference Audit

## Boundary

Skill selection is a model decision over a bounded metadata catalog. It must
not be forced from user-text keywords. Core evaluates the concrete invocation;
the Host owns discovery, access checks, and filesystem or remote I/O.

## Reference findings

| System | Useful contract | Gap to avoid |
|---|---|---|
| Codex | Immutable discovery snapshot, typed dependencies, explicit versus implicit policy, exact filesystem provenance, body injection only after selection | Loader policy is intentionally coupled to its own Host filesystem abstraction |
| OpenCode | Metadata-only model catalog, exact-name load, permission check before returning instructions, bounded resource sample | Duplicate names can overwrite and bodies are read during discovery |
| Claude Code | Exact listed name, scoped-name precedence, duplicate-load prevention, inline/fork distinction, explicit success criteria for authored Skills | Product-specific suggestion/search prompts must not become routing heuristics |
| Open WebUI | Separate list response from full content, active-state and per-user/group read access enforced at `view_skill` | Mutable database content has no immutable version/content identity |

## Nola contract

1. The model sees stable Skill id, display metadata, and capability summary.
2. The Host resolves the exact enabled id and reads `SKILL.md` from its trusted
   Skill root.
3. The Host hashes the exact loaded bytes and returns `version`,
   `instructions_ref`, and `instructions_digest` with the invocation payload.
4. Core persists that identity in the turn or conversation Skill state.
5. A Core `SkillRuntimeCommand::Start` is the Host's materialization
   attestation. Core rejects it unless the reference ends in the same canonical
   lowercase SHA-256.
6. Required tools are resolved independently through the typed capability
   catalog. Missing tools remain a recoverable catalog-refresh state.
7. Explicit native Host Skill roots replace the standalone catalog. The same
   frozen roots generate model metadata and service exact-id loading; clients
   do not inject a second routing prompt into project instructions.
8. Duplicate ids across configured roots are rejected as ambiguous rather than
   resolved by directory iteration order.

This separates selection, materialization, authorization, and execution while
avoiding a redundant Host round trip after the Host has already loaded the
instructions.

## Remaining convergence work

- Make every native Host produce the same materialization attestation before
  submitting a Skill run.
- Add scope/origin and implicit-invocation policy to the shared descriptor.
- Persist a bounded Skill resource manifest so recovery can prove which scripts
  and templates were available without injecting their contents into prompts.
