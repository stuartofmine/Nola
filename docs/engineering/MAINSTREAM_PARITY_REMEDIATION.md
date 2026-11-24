# Nola 主流产品对标整改设计方案（Mainstream Parity Remediation）

> **基线提交**：`main` @ `01f24de1` · **兼容约束**：严格遵守 Nola 1.0.0 兼容基线（协议/ABI/SQLite schema_epoch=1 不破坏，不自动删除或静默改写用户数据）。

## 1. 背景与目标

基于对 Nola 全栈源码（~127 万行）的深度代码审查与主流产品（Claude Code、Codex CLI、OpenCode、Cursor）对标，本轮整改聚焦高杠杆、可验证的 6 条并行工作流，按“先止血、再提效、后提体验”的原则分模块落地，确保每条链路具备回归单元测试与契约验证。

### 非目标（留待后续专题）
- 不破坏现有 1.0.0 数据库 schema epoch 与跨端二进制 ABI 契约。
- 不在本轮内执行 `nola-service` 全面异步化（`tokio`/`axum` 重写）或桌面端技术栈合并（macOS SwiftUI vs Tauri），这些作为 Phase 4 独立架构提案推进。

---

## 2. 六路并行改造规格与文件边界

为保证并发修改零冲突、职责清晰，将改造拆分为 6 个文件完全互斥的工作流：

### Workstream A：沙箱、PTY、权限与自动格式化安全加固
**归属文件**：
- `core/nola-kernel/src/permission_policy.rs`
- `apps/cli/src/permissions.rs`
- `apps/cli/src/native_sandbox.rs`
- `apps/cli/src/tools/pty.rs`
- `apps/cli/src/tools/parts/background.rs`
- `apps/cli/src/task_runtime_parts/execution.rs`
- `apps/cli/src/tools/parts/editing.rs`
- `apps/cli/src/tools/parts/checkpoints_review.rs`

**核心改动**：
1. **`pty_exec` 接入沙箱、环境变量脱敏与权限规则**：
   - 启动 PTY 子进程前调用 `native_sandbox::scrub_sensitive_env`，并在存在可用原生沙箱策略时包装命令（沙箱显式要求但包装失败时 fail-closed）。
   - 在 `permissions.rs` 与 `permission_policy.rs` 中将 `pty_exec` 纳入 shell 命令权限匹配器，使其受所有 `bash` 拒绝/允许规则约束。
   - 在 `PtySessionManager` 中记录会话/工作区归属标识，阻止跨会话任意读写或关闭他人的 `pty-N`。
2. **原生沙箱补漏与 Fail-Closed**：
   - Linux `bwrap`：对 `deny_read` 中的普通文件使用 `--ro-bind /dev/null <file>` 屏蔽（覆盖 `.git-credentials`、`.netrc`、`.pgpass`、`.bash_history`、`.zsh_history`），并补充 `--unshare-pid` 与 `--unshare-ipc` 隔离。
   - macOS Seatbelt：将敏感路径规则由 `file-read-data` 收紧为 `file-read*`，并在 `allow_network == false` 时同时拒绝 `network-bind`。
   - 前台 `bash`（`background.rs`）与后台任务（`execution.rs`）：当策略明确开启原生沙箱且 `wrap_command_in_native_sandbox` 返回 `Err` 时返回显式错误而非静默裸奔。
3. **权限策略防越权与中间档补齐**：
   - `permission_policy.rs`：移除对破坏性命令（`rm`、`rmdir`、`unlink`、`shred`、`truncate`、`kill`、`killall`、`pkill`）的 1-token 会话自动放宽；对 `bash_command_glob_matches` 与 `permissions.rs` 的通配匹配增加 shell 复合分隔符检查（禁止未显式包含 `;`、`&&`、`||`、`|`、`$()`、`` ` ``、`\n` 的规则匹配复合命令）。
   - `permissions.rs`：新增中间档权限配置 `accept_edits`（自动允许工作区内文件读写编辑，对 shell/网络/破坏性操作仍需审批），兼容保留 `cautious` 与 `allow_all`。
4. **自动格式化安全与编辑连续性修复**：
   - `editing.rs`：将 `format_plan_items` 改为参数数组（argv）直接执行，彻底消除 `sh -lc` 字符串拼接导致的命令注入；单文件写操作仅格式化目标文件，不再触发全仓库 `cargo fmt`/`swiftformat .`/`npm run format`；当 `process_sandbox.is_some()` 时安全跳过自动格式化并附带提示，不再令已写成功的 `write_file` 以 `partial_execution: true` 失败。
   - `checkpoints_review.rs`：修复 `reindent_replacement` 对刻意退格行误加 `target_base` 破坏缩进的缺陷；确保编辑/写入后的读凭证连续性不被无故清空。

---

### Workstream B：Agent Harness、Prompt Caching、Thinking 签名与流式/隐私修复
**归属文件**：
- `apps/cli/src/llm_parts/provider_protocols.rs`
- `apps/cli/src/llm_parts/stream_state.rs`
- `apps/cli/src/llm_parts/transport.rs`
- `apps/cli/src/llm_parts/responses.rs`
- `apps/cli/src/llm_parts/streaming.rs`
- `core/nola-model-privacy/src/lib.rs`
- `core/nola-agent-engine/src/agent/runtime/model_planner.rs`
- `core/nola-agent-engine/src/agent/runtime/submission.rs`
- `core/nola-agent-engine/src/agent/loop_guard.rs`
- `core/nola-agent-engine/src/agent/runtime/turn_lifecycle.rs`
- `core/nola-agent-engine/src/agent/runtime/tool_scheduler.rs`

**核心改动**：
1. **修复 Anthropic / OpenAI Prompt Caching 全链路**：
   - `provider_protocols.rs`：在剥离 `_nola_cache_tail` 之前先定位稳定历史边界；为 Anthropic 的 `tools` 末尾、静态 `system` 块、以及 `_nola_cache_tail` 之前的最后一条稳定消息挂载 `cache_control: {"type": "ephemeral"}`，绝不把断点打在每步变化的易变尾块之后。
   - `model_planner.rs`：将 `invoked_skill_context_items()` 追加到 `model_context` 之后而非插在历史之前；保持工具列表前缀顺序稳定，避免新增工具引起 `BTreeMap` 重排导致整表缓存失效。
2. **修复 Anthropic Extended Thinking 连续性**：
   - `provider_protocols.rs`：解析并保留 SSE 中的 `signature_delta` / `signature` 与 `redacted_thinking` 块，在 `convert_messages_to_anthropic` 构造包含 `tool_use` 的历史 assistant 回合时正确回传已签名的 `thinking` / `redacted_thinking` 块。
3. **消除流式 $O(N^2)$ 性能炸弹与每请求新建运行时**：
   - `stream_state.rs`：`append_choice_text` 仅在受限尾部窗口与节流步长（如每 256 字节）下做重复检测，彻底消除每 token 全文 `clone`。
   - `transport.rs` / `responses.rs`：复用 `OnceLock<tokio::runtime::Runtime>` 与共享 `reqwest::Client` 连接池，启用 HTTP/2 长连接复用。
   - `provider_protocols.rs`：修正 `detect_chat_provider`，不再仅凭未知端点模型名含 `"qwen"` 就误判为 `LlamaCpp` 并注入非标准字段。
4. **修复云端隐私脱敏误伤代码**：
   - `core/nola-model-privacy/src/lib.rs`：不再对 `Value::Number` 执行正则脱敏（防止破坏 JSON 整数类型）；收窄 `secret_field()`，放行分页与编程常见字段（`token`、`next_page_token`、`page_token`、`continuation_token`、`cancellation_token`、`csrf_token` 等），仅精确命中真实凭据字段（`api_key`、`access_token`、`refresh_token`、`id_token`、`client_secret`、`private_key`、`password` 等）；避免把带引号的普通代码字面量（如 `let token = "EOF"`）或工具输出中的普通邮箱改写为占位符，重点保留高置信度密钥格式（`sk-`、`ghp_`、`AKIA`、PEM 私钥、Bearer 头等）。
5. **消除误杀与提升并发/取消响应**：
   - `submission.rs`：`contains_embedded_tool_call_protocol` 忽略 Markdown 围栏代码块与行内代码中的 `<tool_call>` 示例。
   - `loop_guard.rs`：放宽 `output_is_structurally_repetitive`，避免误杀 Markdown 表格与结构化代码。
   - `turn_lifecycle.rs`：支持二次 `cancel()` 强制中止，防止不可取消工具永久锁死用户 `Ctrl+C`。
   - `tool_scheduler.rs`：将 `MAX_CONCURRENT_TOOL_BATCH_SIZE` 从 `4` 提升至 `12`。

---

### Workstream C：代码检索、常驻 LSP 与大文件读取修复
**归属文件**：
- `apps/cli/src/tools/parts/search_planning.rs`
- `apps/cli/src/tools/parts/permissions_output.rs`
- `apps/cli/src/tools/parts/file_read_evidence.rs`
- `apps/cli/src/tools/lsp_diagnostics.rs`
- `apps/cli/src/tools/language_service_tools.rs`

**核心改动**：
1. **重构 `grep` / `glob` 遍历与大文件处理**：
   - 使用 workspace 已有依赖 `ignore::WalkBuilder` 遵守 `.gitignore`（保留常见构建目录过滤）。
   - 在检查文件大小前先跳过二进制文件；遇到 `>4MB` 的大文件时**跳过并记录跳过计数**，绝不因单个大图/锁文件/SQLite 导致整次 `grep` 报错中止。
2. **统一常驻 LSP 进程池**：
   - 让 `language_service_tools.rs` 复用持久化 LSP 会话池（按 `(workspace_root, server_command)` 缓存复用，异常退出自动重建），彻底消灭每次符号查询都冷启/杀死 `rust-analyzer`/`tsserver` 的高延迟问题；在 `lsp_diagnostics.rs` 中补充 `.js`/`.jsx` 文件类型映射。
3. **大文件分页读取保护**：
   - `file_read_evidence.rs`：对超大文件（>8 MiB）的分页 `read_file` 避免每次同步读取并 SHA-256 哈希整份文件至 EOF，只读取请求窗口所需范围并清晰标注大文件读状态。

---

### Workstream D：守护进程安全边界、本地 Web Host 防护、SSRF 与存储自洁
**归属文件**：
- `core/nola-service/src/lib.rs`
- `core/nola-service-client/src/lib.rs`
- `core/nola-store/src/connection.rs`
- `apps/cli/src/web_parts/routes.rs`
- `apps/cli/src/web_parts/streaming_state.rs`
- `apps/cli/src/web_parts/native_bridge.rs`

**核心改动**：
1. **UDS 私有目录与 `SO_PEERCRED` 双向校验**：
   - `nola-service-client` 将默认 Unix Socket 迁移至用户私有 `0700` 目录（优先 `$XDG_RUNTIME_DIR/nola`，回退至数据库同级 `.nola-run/` 而非全局 `/tmp`）。
   - Unix 平台在 `nola-service` `accept()` 与 `nola-service-client` `connect()` 两侧校验对端 UID 等于当前 `geteuid()`（Linux `SO_PEERCRED` / macOS `getpeereid`）。
2. **本地 Web Host 防 DNS Rebinding 与 CSRF**：
   - `routes.rs`：对所有 `local_trust` 下的 `/api/*` 请求校验 `Host` 头必须属于本地回环（`127.0.0.1` / `localhost` / `[::1]`），对写请求（`POST`/`PUT`/`PATCH`/`DELETE`）校验若携带 `Origin` 或 `Sec-Fetch-Site` 则必须同源，阻断恶意网页跨域调用本地 Agent。
3. **修复定时快照 SSRF 与响应体上限**：
   - `streaming_state.rs` (`build_html_snapshot`)：强制仅允许 `http`/`https` 协议，在首次请求与每次重定向前解析目标地址并拒绝 loopback、RFC1918 内网、链路本地（含 `169.254.169.254`）及 IPv6 ULA/link-local；限制响应体最大读取 `4 MiB`。
4. **移动端 Nonce 缓存上限**：
   - `native_bridge.rs`：为 `MOBILE_AUTH_NONCES` 增加容量上限（如 10,000 条）与过期/超限驱逐，防止内存耗尽。
5. **SQLite 无限增长治理（兼容 schema epoch=1）**：
   - `core/nola-store/src/connection.rs`：配置 `PRAGMA journal_size_limit = 67108864`（64 MiB）防止 `-wal` 无限膨胀；在打开数据库时按保守时间窗口/条数上限清理过旧的 `command_receipts` 与 `projection_events`（保留近期活跃记录，不改表结构）。

---

### Workstream E：CLI / TUI 开发者体验对标升级
**归属文件**：
- `apps/cli/src/terminal_ui.rs`
- `apps/cli/src/agent_parts/emitter.rs`
- `apps/cli/src/main_parts/interactive.rs`
- `apps/cli/src/main_parts/bootstrap.rs`
- `apps/cli/src/main_parts/commands.rs`
- `apps/cli/src/main_parts/composer_completion.rs`
- `apps/cli/src/interaction.rs`

**核心改动**：
1. **过程透明化（Thinking + 子代理 + 内联 Diff）**：
   - `emitter.rs`：实现 `Emitter::reasoning()`（流式输出柔和暗色思考摘要，而非空实现 NO-OP），并在交互模式不再静默吞掉 `subagent_event`。
   - `interaction.rs`：修复权限弹窗中 unified diff 渲染（同时识别 `+`/`-` 行，排除 `+++`/`---`）；修复 `execution_cancel_key` 支持大写 `C`（Caps Lock 兼容）。
   - `emitter.rs`：在编辑类工具完成摘要中输出精简彩色内联 diff 预览。
2. **高频工作流命令（`!cmd`、`/fork`、`/copy`、帮助一致性）**：
   - `interactive.rs`：支持以 `!` 开头的快捷本地命令执行（如 `!git status`），将输出展示并可选带入会话；暴露 `/fork` 命令对接已有 `CreateConversationBranch`；新增 `/copy` 命令（支持本地剪贴板 + SSH/WSL 下 OSC 52 回退）；补齐 `/thinking` 与 `/input-recovery` 在帮助列表和补全表中的一致性。
3. **`@file` 补全与终端兼容性**：
   - `composer_completion.rs`：改用 `ignore::WalkBuilder` 遵守 `.gitignore`，提高扫描上限至 4,000 文件并支持模糊子序列排序。
   - `bootstrap.rs` / `terminal_ui.rs`：按 `COLORTERM` 检测决定是否输出 24-bit 暗色输入条（在 256 色/Terminal.app 下安全降级）；将 `MAX_VIEWPORT_REPLAY_BYTES` 从 256 KiB 提升至 2 MiB；对 `ViewState` 落盘做防抖（不再每次按键都写盘）。

---

### Workstream F：Web 前端与跨端高杠杆体验/性能修复
**归属文件**：
- `apps/web/frontend/src/lib/markdown.ts`
- `apps/web/frontend/src/components/Markdown.svelte`
- `apps/web/frontend/src/components/WorkspaceDiffPanel.svelte`
- `apps/web/frontend/src/pages/Chat.svelte`
- `apps/web/frontend/src/lib/conversationSubmissionController.ts`
- `shared/apple/GenerativeUI/AppleUISurfaceView.swift`
- `apps/android/app/src/main/java/com/nola/ui/components/MarkdownRenderer.kt`

**核心改动**：
1. **Web 代码与 Diff 语法高亮**：
   - 在 `markdown.ts` 中实现轻量零依赖语法高亮器（支持 Rust/TS/JS/Python/Swift/Kotlin/Go/JSON/YAML/Bash/SQL/HTML/CSS/Diff），并在 `Markdown.svelte` 与 `WorkspaceDiffPanel.svelte` 中接入渲染与主题样式。
2. **Web 性能修复（消灭 N+1 瀑布与 $O(N^2)$ 扫描）**：
   - `conversationSubmissionController.ts`：将加载历史会话时串行的 `for ... await` 拉取 `/api/chat/agent/v2/journal` 改为有界并发（并发度 6）批量拉取。
   - `Chat.svelte`：预计算消息 ID 集合，消除 `{#each messages}` 内部的 `messages.some(...)` $O(N^2)$ 扫描。
   - `WorkspaceDiffPanel.svelte`：点击文件仅刷新当前文件 diff，不再重复拉取 checkpoints 和 worktrees。
3. **移动端严重性能/泄漏定点修复**：
   - `AppleUISurfaceView.swift`：在视图销毁时清理 `AppleUIEditingRegistry.shared.entries` 并加容量上限；停止无条件 80ms 全窗口 `UIView` DFS 轮询。
   - `MarkdownRenderer.kt`：将 `Markwon` 实例按 `Context` 缓存复用，避免 `LazyColumn` 中每个 Markdown 块重复构造 `Markwon.builder(context)`。

---

## 3. 验证策略

- **文档检查**：`python3 scripts/check_docs.py`
- **Rust 单元测试（按模块精确筛选，遵守 `AGENTS.md` 构建范围约定）**：
  - `cargo test --locked -p nola_kernel --lib`
  - `cargo test --locked -p nola_model_privacy --lib`
  - `cargo test --locked -p nola_agent_engine --lib`
  - `cargo test --locked -p nola_store --lib`
  - `cargo test --locked -p nola_service --lib`
  - `cargo test --locked -p nola_service_client --lib`
  - `cargo test --locked -p nola_cli --lib`
- **Web 前端检查与单测**：
  - `npm --prefix apps/web/frontend run check`
  - `npm --prefix apps/web/frontend test`
  - `npm --prefix apps/web/frontend run build`

---

## 4. Wave 2 深度演进规格（Workstreams G–L）

在 Wave 1（Workstreams A–F）完成止血、Prompt Cache 修复与基础体验补齐后，Wave 2 继续并行落地 6 条高杠杆对标能力（严格保持 1.0.0 兼容与 `--offline --locked` 零依赖变更）：

### Workstream G：MCP OAuth 2.1 / PKCE、Legacy SSE 与并发无锁调用
**归属文件**：`core/nola-mcp-host/src/{lib.rs,mcp_host.rs,oauth.rs}`
1. **OAuth 2.1 / PKCE 与 RFC 9728 发现**：新增 `oauth.rs`，实现 PKCE S256 verifier/challenge 生成、RFC 9728 `/.well-known/oauth-protected-resource` 与 Authorization Server 元数据解析、Token 刷新与到期判断，支持远程 MCP 服务器通过 Bearer OAuth token 鉴权（不再仅限于静态 Header）。
2. **Legacy SSE 传输兼容**：在 `mcp_host.rs` 中打通 `McpTransportKind::ServerSentEvents`（解析 `event: endpoint` + POST JSON-RPC + SSE 响应匹配），解除原先的 `mcp_transport_unsupported` 阻断。
3. **解除全局互斥锁串行瓶颈**：重构 HTTP MCP 调用路径，仅在读取/更新连接元数据快照时短持锁，HTTP 网络往返期间完全释放 `Mutex`，允许同一远程 MCP 服务器并发处理多个工具调用。

### Workstream H：可拦截 Lifecycle Hooks 与项目级权限规则
**归属文件**：`core/nola-agent-engine/src/lifecycle_hooks.rs`、`apps/cli/src/agent_host_driver_parts/lifecycle_execution.rs`、`apps/cli/src/permission_grants.rs`
1. **可阻断/可改写参数的 Hooks**：在 `lifecycle_hooks.rs` 与 `lifecycle_execution.rs` 中支持 `HookDecision::{Allow, Block { reason }, ModifyInput { input }}`（保留既有观察型 Hook 100% 兼容），新增项目 `.nola/hooks.json` 与用户 `~/.nola/hooks.json` 加载器；超时处理改为独立进程/通道清理，避免持锁线程泄漏死锁。
2. **项目级权限配置**：在 `permission_grants.rs` 中支持读取并合并工作区 `.nola/settings.json` 与 `.nola/settings.local.json` 中的 `permissions.allow` / `permissions.deny` / `permissions.profile` 规则（优先级：显式 Deny > 项目 local > 项目 shared > 用户全局）。

### Workstream I：SQLite 前向迁移框架、归档写锁降噪与凭据库加密落盘
**归属文件**：`core/nola-store/src/schema_store.rs`、`core/nola-store/src/work_archive_worker.rs`、`core/nola-credential-host/src/lib.rs`
1. **SQLite 版本化前向迁移引擎**：在保持当前 `user_version = 1` / `schema_epoch = 1` 100% 兼容的前提下，在 `schema_store.rs` 中实现 `MigrationStep` 注册表、`plan_forward_migrations` 与事务化可回滚 `apply_forward_migrations`（迁移前文件快照保护，遇到未知更高版本显式拒绝不毁数据）。
2. **降低 `work_archive_worker` 写锁竞争**：闲时采用自适应退避（1s → 8s），仅当 publication 序号确实推进时才开启写事务，减少与主库写者的 WAL 锁冲突。
3. **凭据库静态加密（Encrypt-then-MAC）**：`nola-credential-host` 使用 `0600` 随机主密钥 + HKDF-SHA256 派生加密/认证双密钥 + 随机 128-bit Nonce 的 Encrypt-then-MAC（`nola-enc-v1:`）信封加密落盘，向后兼容读取历史明文行并在写入时自动升级为密文，释放时显式清零敏感内存。

### Workstream J：两级上下文微压缩与工具前缀端到端稳定
**归属文件**：`core/nola-agent-engine/src/agent/{capability_projection.rs,compaction_planner.rs}`、`core/nola-agent-engine/src/agent/runtime/compaction_driver.rs`
1. **工具投影端到端顺序稳定**：`capability_projection.rs` 保留调用方传入的稳定前缀顺序（不再在 `compile` 阶段用 `BTreeMap` 把动态新增工具插回前缀中间），同时保持确定性去重与合法性校验。
2. **Tier-1 廉价微压缩（Micro-Compaction）**：在触发昂贵的 Tier-2 LLM 总结压缩或硬阻断 `context.input_budget_exceeded` 前，优先对历史中已过期的超长旧工具输出做无损元信息保留的截断裁剪，大幅降低触发全量总结的频率。

### Workstream K：CLI 自动化/CI 参数闭环与 TUI `Ctrl+O` 展开
**归属文件**：`apps/cli/src/{config.rs,agent_parts/emitter.rs,main_parts/{bootstrap.rs,commands.rs,interactive.rs}}`
1. **CLI 参数与自动化能力**：打通 `--permission-profile accept_edits`、新增 `--fork-session`（结合 `--session`/`--resume` 自动创建分支）、`--output-format json`（单条结构化最终 JSON 摘要输出）、`--json-schema <json|@file>`（结构化输出约束透传）。
2. **TUI `Ctrl+O` 详情展开切换**：在交互模式下按 `Ctrl+O` 切换工具输出与推理摘要的紧凑/展开视图，并同步在底部状态提示中反馈当前模式。

### Workstream L：Web 端首启引导、长会话分窗、LaTeX 渲染、双栏 Diff 与 Tailcat 限流
**归属文件**：`apps/web/frontend/src/{pages/Chat.svelte,lib/markdown.ts,lib/markdown.test.ts,components/Markdown.svelte,components/WorkspaceDiffPanel.svelte}`、`transport/tailcat/transport.go`
1. **Web 端首启引导与长会话分窗**：`Chat.svelte` 在未配置可用模型时展示内联两步快速引导卡片（一键跳转模型配置或输入连接），并对超长对话默认渲染最近窗口（如 80 条）+ 一键展开更早消息，防止数百条消息拖垮 DOM。
2. **Web 端零依赖 LaTeX 数学排版**：`markdown.ts` + `Markdown.svelte` 支持行内 `$...$` 与块级 `$$...$$` 数学公式安全格式化渲染（涵盖上下标、分式、希腊字母与常用算符，不侵入代码块）。
3. **Web 端双栏 / 统一 Diff 切换**：`WorkspaceDiffPanel.svelte` 支持 Unified / Side-by-Side 视图切换。
4. **Tailcat 隧道安全加固**：`transport/tailcat/transport.go` 增加并发 TCP 代理信号量上限（默认 128）、连接读写空闲超时，以及 `validRelay` 拒绝回环/内网地址。

