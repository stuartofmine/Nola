# Nola 主流产品对标整改与 v2 精简架构落地总结（Mainstream Parity Remediation）

> **当前基线**：`v2`（Nola 2.0.0 精简 8-crate 架构，`crates/nola-types` → `nola-store` → `nola-llm` → `nola-tools` → `nola-mcp` → `nola-engine` → `nola-modules` → `nola-app`，统一 `nola` 单一可执行文件与 SQLite `user_version = 2`）。

## 1. 背景、目标与 v2 落地状态

基于对 Nola 全栈源码的深度代码审查与主流产品（Claude Code、Codex CLI、OpenCode、Cursor）对标，Wave 1（Workstreams A–F）与 Wave 2（Workstreams G–L）全部 12 项对标整改能力已完成并原生融合进 Nola v2 8-crate 架构中，彻底消除了历史 `core/` 与 `apps/cli/` 多二进制、多实现冗余：

| v2 Crate | 承载的对标整改能力 |
| --- | --- |
| `crates/nola-types` | 统一策略档位（含 `accept_edits`）、工具契约、上下文预算与模型路由结构 |
| `crates/nola-store` | SQLite WAL + FTS5（`user_version = 2`、`PRAGMA journal_size_limit`）、显式拒绝旧版 v1 / 外国库、`VACUUM INTO` 原子备份、Encrypt-then-MAC 静态加密 Vault 与 UDS 服务锁 |
| `crates/nola-llm` | Anthropic / OpenAI Prompt Caching 断点、Extended Thinking `signature` 连续性、共享连接池、流式状态去重与精准云端隐私脱敏（`privacy.rs`） |
| `crates/nola-tools` | 统一文件变更管线（`file_pipeline.rs`）、能力探测 OS 沙箱（`bwrap` / Seatbelt）与 fail-closed `shell_pipeline.rs` / `pty.rs`、`ignore::WalkBuilder` 代码检索（`search.rs`）、常驻 LSP 进程池（`lsp.rs`）、内容寻址大输出溢写（`output_store.rs`）、DNS 固定 SSRF 防护（`web.rs`）与 QuickJS `code_mode.rs` |
| `crates/nola-mcp` | OAuth 2.1 / PKCE 与 RFC 9728 发现（`oauth.rs`）、Legacy SSE 传输兼容（`transport.rs`）与无锁并发 HTTP 工具调用 |
| `crates/nola-engine` | 可拦截/改写参数的 Lifecycle Hooks（`hooks.rs`）、项目/用户分层权限合并（`policy.rs`）、两级微压缩 + 摘要压缩（`compaction.rs`）、工具前缀顺序稳定（`catalog.rs`）、结构化输出与二次取消保护（`loop_runner.rs` / `loop_guard.rs`） |
| `crates/nola-modules` | 记忆、代码索引、GitHub Skill 安装、Goal/SOP、资料源、多专家 Room/Peer、自动化与调度器、A2UI、本地模型/插件以及 Ed25519 签名更新校验（`updater.rs`） |
| `crates/nola-app` | 统一 `nola` 单一二进制（CLI、TUI `tui.rs`、本地 HTTP/Web Host `web_server.rs` 防 DNS Rebinding / CSRF、守护进程 `daemon.rs` 对端 UID 校验）与统一 C ABI / JNI FFI 库（`ffi.rs`） |

---

## 2. Wave 1 整改规格（Workstreams A–F，已整合至 v2）

### Workstream A：沙箱、PTY、权限与自动格式化安全加固
**v2 归属文件**：
- `crates/nola-engine/src/policy.rs`
- `crates/nola-tools/src/sandbox.rs`
- `crates/nola-tools/src/pty.rs`
- `crates/nola-tools/src/shell_pipeline.rs`
- `crates/nola-tools/src/file_pipeline.rs`
- `crates/nola-tools/src/code_tools.rs`

**核心改动（已完成）**：
1. **`pty_exec` 接入沙箱、环境变量脱敏与权限规则**：
   - 启动 PTY 子进程前脱敏敏感环境变量，并在开启原生沙箱策略时 fail-closed 包装执行；`pty_exec` 纳入 shell 命令权限匹配器，且按会话/工作区隔离 PTY 归属。
2. **原生沙箱补漏与 Fail-Closed**：
   - Linux `bwrap` 屏蔽敏感文件并启用 `--unshare-pid` / `--unshare-ipc`；macOS Seatbelt 收紧敏感路径 `file-read*` 并按需拒绝 `network-bind`；显式要求沙箱失败时 fail-closed 返回错误。
3. **权限策略防越权与中间档补齐**：
   - 破坏性命令（`rm`、`rmdir`、`unlink`、`shred`、`truncate`、`kill` 等）不自动放宽；通配符匹配禁止隐式穿透复合 shell 分隔符；支持 `cautious`、`accept_edits`、`allow_all` 三档权限配置。
4. **安全编辑与文件变更连续性**：
   - `FileMutationPipeline` 保证每次文件写入/编辑产出版本跃迁与精确 diff，无 `sh -lc` 命令注入风险。

---

### Workstream B：Agent Harness、Prompt Caching、Thinking 签名与流式/隐私修复
**v2 归属文件**：
- `crates/nola-llm/src/{anthropic.rs,openai_chat.rs,openai_responses.rs,gemini.rs,client.rs,sse.rs,privacy.rs}`
- `crates/nola-engine/src/{loop_runner.rs,loop_guard.rs,catalog.rs,middleware.rs}`

**核心改动（已完成）**：
1. **Anthropic / OpenAI Prompt Caching 全链路**：固定静态 `system` 与工具前缀顺序，稳定消息边界挂载 `cache_control: {"type": "ephemeral"}`，易变上下文仅追加在尾部。
2. **Anthropic Extended Thinking 连续性**：解析并回传 `signature` / `redacted_thinking` 块。
3. **消除流式 $O(N^2)$ 瓶颈与复用 HTTP 连接池**：共享 `reqwest::Client` 与有界增量流处理。
4. **精准云端隐私脱敏**：`crates/nola-llm/src/privacy.rs` 保留 JSON 数值类型与编程常规字段（如 `page_token`），重点脱敏高置信度凭据字段与密钥模式。
5. **消除误杀与并发/取消响应**：忽略代码块内的协议示例标签、避免误杀 Markdown 表格、支持立即取消信号与高并发工具批调度。

---

### Workstream C：代码检索、常驻 LSP 与大文件读取修复
**v2 归属文件**：
- `crates/nola-tools/src/{search.rs,file_pipeline.rs,lsp.rs,code_tools.rs}`

**核心改动（已完成）**：
1. **`grep` / `glob` 遍历与大文件处理**：采用 `ignore::WalkBuilder` 遵守 `.gitignore`，自动跳过二进制与超大文件并计数，不中断整体检索。
2. **统一常驻 LSP 进程池**：`LspPool` 按 `(workspace_root, server_command)` 复用持久 LSP 进程并在异常退出时自动重建。
3. **大文件分页读取保护**：支持按行/字节窗口读取，避免对超大文件全量阻塞读取。

---

### Workstream D：守护进程安全边界、本地 Web Host 防护、SSRF 与存储自洁
**v2 归属文件**：
- `crates/nola-app/src/{daemon.rs,web_server.rs,web_api.rs}`
- `crates/nola-store/src/{db.rs,service_lock.rs,vault.rs}`
- `crates/nola-tools/src/web.rs`

**核心改动（已完成）**：
1. **UDS 私有目录与对端 UID 校验**：Unix Socket 落于用户私有 `0700` 目录，连接双侧校验对端 UID 与当前 `geteuid()` 一致。
2. **本地 Web Host 防 DNS Rebinding 与 CSRF**：对本地回环 `/api/*` 校验 `Host` 头与写请求 `Origin` / `Sec-Fetch-Site` 同源策略，限制 Nonce 缓存容量上限。
3. **DNS 固定 SSRF 防护与响应体上限**：强制仅允许 `http`/`https`，每次连接与重定向前校验解析地址并拒绝回环、内网、链路本地与元数据地址。
4. **SQLite WAL 边界治理**：`crates/nola-store/src/db.rs` 按 scope 限制事件保留水位并提供 `VACUUM INTO` 原子备份。

---

### Workstream E：CLI / TUI 开发者体验对标升级
**v2 归属文件**：
- `crates/nola-app/src/{cli.rs,tui.rs,config.rs}`

**核心改动（已完成）**：
1. **过程透明化**：TUI 流式展示柔和暗色推理摘要（`ReasoningDelta`）、工具调用与 `/verbose` 展开切换，以及会话级权限审批。
2. **统一命令行与自动化能力**：单一 `nola` 二进制覆盖 `run`、`tui`、`web`、`server`、`config`、`model`、`mcp`、`skills`、`memory`、`plugin`、`sessions`、`goal`、`room`、`scheduler`、`update`（含 `verify-feed`）与 `version`。

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

**核心改动（已完成）**：
1. **Web 代码与 Diff 语法高亮**：零依赖多语言语法高亮器与 Unified / Side-by-Side Diff 视图切换。
2. **Web 性能修复**：有界并发加载会话历史、预计算消息 ID 集合消除 $O(N^2)$ 扫描、按需刷新单文件 diff。
3. **移动端性能/泄漏修复**：清理 `AppleUIEditingRegistry` 视图条目并缓存 Android `Markwon` 实例。

---

## 3. Wave 2 深度演进规格（Workstreams G–L，已整合至 v2）

- **Workstream G（MCP OAuth 2.1 / PKCE、SSE 与并发调用）**：落地于 `crates/nola-mcp/src/{lib.rs,oauth.rs,transport.rs}`。
- **Workstream H（可拦截 Lifecycle Hooks 与项目级权限规则）**：落地于 `crates/nola-engine/src/{hooks.rs,policy.rs}`。
- **Workstream I（SQLite 严格版本保护与 Vault 静态加密）**：落地于 `crates/nola-store/src/{db.rs,vault.rs}`。
- **Workstream J（两级上下文微压缩与工具前缀稳定）**：落地于 `crates/nola-engine/src/{compaction.rs,catalog.rs}`。
- **Workstream K（CLI 自动化参数闭环与 TUI 展开）**：落地于 `crates/nola-app/src/{cli.rs,tui.rs,config.rs}`。
- **Workstream L（Web 首启引导、长会话分窗、LaTeX 渲染、双栏 Diff 与 Tailcat 限流）**：落地于 `apps/web/frontend/src/` 与 `transport/tailcat/transport.go`。

---

## 4. 验证策略（v2）

- **文档与目录检查**：
  - `python3 scripts/check_docs.py`
  - `python3 scripts/check_source_layout.py`
  - `python3 scripts/localization_coverage.py`
- **Rust 单元测试（遵守 `AGENTS.md` 构建范围约定）**：
  - `cargo test --locked --workspace --lib`
  - 或按 crate 定向：`cargo test --locked -p nola-types -p nola-store -p nola-llm -p nola-tools -p nola-mcp -p nola-engine -p nola-modules -p nola-app --lib`
- **脚本与发布回归**：
  - `python3 -m unittest discover -s scripts/tests`
  - `python3 -m unittest discover -s scripts/releases`
- **Web 前端检查与单测**：
  - `npm --prefix apps/web/frontend run check`
  - `npm --prefix apps/web/frontend test`
  - `npm --prefix apps/web/frontend run build`

