> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Skill Runtime Architecture

## Product contract

macOS and CLI Web expose the same built-in Skill catalog. A Skill is an executable capability package, not only prompt text. Each package contains a `SKILL.md` workflow and may include scripts or agent metadata.

## Single source of truth

The canonical packages live in `NolaMac/Resources/NolaBuiltInSkills.bundle`. macOS loads that bundle directly. The CLI build embeds the same files into the Rust binary and materializes enabled packages under `~/.nola/cli/skills/built-in` at runtime. An installed CLI therefore does not require a Nola source checkout.

All built-in packages are shipped with the application. Nola never downloads a Skill or its reference files at task time. "On demand" below refers only to context loading: the files are already local, but the agent reads only the selected workflow and references.

## Built-in research capabilities

Eight research-oriented Skills and one internal shared-reference package are bundled alongside the other functional categories:

- full-paper reading and bilingual source-grounded translation
- scientific manuscript planning and drafting
- academic language polishing
- traceable pre-submission review
- reviewer and editor response drafting
- statistical reporting audit
- academic reference verification
- paper-to-presentation production

Each user-facing package includes `NOLA_MANIFEST.json`. The contract declares its package identity, supported platforms, required tools, optional runtimes, permissions, secrets, and validation entry points. The agent must inspect this contract before execution and fail readiness checks before starting expensive work. `nature-shared` is materialized with the suite but is not shown as a standalone Skill.

The imported workflows are based on the Apache-2.0 `nature-skills` project at commit `8d674eb50454d4b49d3e0be625a121ad4863840b`. README, evaluation fixtures, and repository-only tests are excluded from the product bundle. CAPTCHA bypass, browser impersonation, paywall bypass, and other anti-bot automation are not included.

The Web UI persists only user-owned state:

- enabled or disabled
- removed or restored
- edited `SKILL.md` content
- uploaded custom Skills

Bundled scripts are immutable product resources. Disabling or deleting a built-in Skill removes its managed runtime directory so historical filesystem grants cannot make it discoverable again.

Every user-facing built-in Skill ships a `NOLA_MANIFEST.json` contract. It declares delivery mode, functional category, platform support, required and optional tools, runtime dependencies, permission classes, secrets, validation gates, and recommended Skill handoffs. The manifest is product-owned and read-only; users edit workflow instructions in `SKILL.md`, not the security contract.

The gallery groups Skills only by user-facing function: Documents, Data & Analysis, Design & Presentations, Communication, Planning & Automation, Knowledge & Research, and Workspace. Source repository and publisher attribution remain licensing metadata and never create separate product suites.

## Agent loading

The normal context contains a compact catalog with the Skill ID, routing description, and managed base directory. The agent loads the selected workflow through `skill_context` before execution. Relative script paths are resolved from the returned `base_directory`.

This prevents all full Skill documents from consuming every conversation's context window while keeping execution deterministic and inspectable.

## UI model

Both clients use a gallery-first interaction:

1. Browse the available capabilities.
2. Enable, disable, delete, or restore a package.
3. Open a package to inspect its workflow and bundled files.
4. Edit the workflow or invoke the Skill by name.

Custom Skills remain editable and removable. Built-in packages retain their stable identity and can always be restored.
## Activation lifetime

Loading a Skill applies it to the active Turn by default. Core stores that
invocation on `AgentTurnState`; the next user Turn starts with an empty Skill
activation set. This prevents a document, research, or design workflow from
silently changing unrelated later requests.

A Skill may be pinned to the conversation only through an explicit
`activation: session` invocation. Pinned Skills are separately represented in
conversation state and remain visible until removed. Hosts must not infer
session pinning from repeated use or from the Skill category.

## 当前安装与执行接口（2026-10-28核对）

Application SkillCommand与PluginCommand分别维护技能来源/版本和插件安装意图。Plugin Install记录manifest/package_digest及expected_revision，SetEnabled/Remove使用CAS；enabled不授予调用权限。Host负责包字节校验、凭据和活连接，Core按具体工具调用授权；冻结Skill与工具manifest恢复不能扩大原执行能力。
