> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola CLI

Rust command-line agent for local coding workflows. It follows the same high-level loop used by the Web host: stream a configured LLM provider, execute approved local tools, append tool results, and continue until the model stops.

To save the connections and default model declared in your configuration, run
`nola model apply-config`. It reads `~/.nola/config.json`, or `config.jsonc` when
the JSON file is absent (`NOLA_HOME` overrides the application home). To select
a different file, pass its absolute path. This explicit command updates the
Host model center without making a model request; ordinary startup does not
import or rewrite model configuration. When the binary is not on `PATH`, use
`/absolute/path/to/nola model apply-config`.

## Core / Host architecture

The CLI and `nola-agent-host` are Host surfaces, not independent Agent decision
engines. The target architecture is:

- shared Rust Core owns Session/Turn state, input queues, permissions, context
  policy, compaction decisions, tool lifecycle, cancellation, and recovery;
- CLI Host owns provider I/O, shell/files/git execution, terminal permission UI,
  and Journal/checkpoint persistence;
- Web Host adds authenticated HTTP/SSE/WebSocket transport without duplicating
  the Agent loop;
- every asynchronous Host result returns the originating effect ID and is
  reduced by Core before it reaches a presenter.
- product surfaces mutate Core only through `AgentSubmission`; reducer-level
  `AgentCommand` values are not a Host or UI protocol.

The detailed normative contract is documented in
[`../docs/architecture/总体设计/CORE_HOST_ARCHITECTURE.md`](../../docs/architecture/总体设计/CORE_HOST_ARCHITECTURE.md).

### Shared harness and isolated evaluation

The normal CLI, Rust Web and native `nola-agent-host` use the same tool catalog,
Core recovery policy and verification evidence contracts. Environment discovery
(`environment_probe`), structured missing-dependency recovery, PDF page rendering
(`read_file.pdf_page`), shell failure diagnostics and repeated-work recovery do
not require an evaluation mode or a separate harness installation. Build both
product executables together with `scripts/build_cli_runtime.sh` from the repo root.

Isolation belongs to the evaluator: its disposable-container installation grant,
deadline supervisor, external public-check retry wrapper and official scoring
are not product defaults. In particular, dependency recovery on a user's machine
does not grant system-wide installation permission. Native PDF transport still
requires an explicit provider capability declaration, independent of image input.

To check both product entrypoints in fresh containers, run
`scripts/run_environment_recovery_smoke.py` with the same `--runtime` directory,
`--config` and separate new `--output` directories, once with `--entrypoint cli`
and once with `--entrypoint agent-host`. Neither invocation enables benchmark
tool overrides. These are scoped integration checks, not benchmark scores or
native-client UI tests. The native test uses a session-bound client-policy
fixture; it does not remove the Host's required identity/receipt checks.

## Usage

```bash
cd apps/cli
NOLA_LLM_ENDPOINT=http://127.0.0.1:11434/v1 \
NOLA_LLM_MODEL=qwen2.5-coder \
cargo run -- "Inspect this repository and summarize the Android Web Host bridge"
```

也可直接用编译脚本在同一命令里完成编译并进交互：

```bash
cd apps/cli
./scripts/build.sh --run
```

`--run` 会在构建完成后直接启动 CLI。它默认进入纯命令行 交互模式，不会启动 web 服务。`--` 之后的参数会原样透传给 `nola`（例如直接传会话参数）：

```bash
cd apps/cli
./scripts/build.sh --run -- --help
./scripts/build.sh --run -- --session android-audit
```

如果只想先构建不执行：

```bash
cd apps/cli
./scripts/build.sh
```

直接启动交互模式（默认进入命令行对话）：

```bash
cd apps/cli
../target/release/nola
``` 

Continue work across invocations:

```bash
cargo run -- --session android-audit "Inspect android Web Host behavior"
cargo run -- --continue "Continue from the last session"
cargo run -- sessions
cargo run -- session show android-audit
```

Initialize repository memory files:

```bash
cargo run -- init
cargo run -- wiki --output .nola/wiki.md
```

Start the persistent Nola Agent Server for native clients:

```bash
cargo run -- server --host 127.0.0.1 --port 5001
```

`nola server` is the canonical long-running Host entry point. It exposes the
authenticated Agent, session, permission, task, model, Skill, MCP, and data-source
APIs used by Nola clients. `nola serve` is an alias. Use `--host 0.0.0.0` only on
a trusted private network or behind an HTTPS reverse proxy.

Start the same Server with the Nola Web interface:

```bash
cargo run -- web --host 127.0.0.1 --port 5001 --web-root ../web
```

Opening `/` redirects to the Chat workspace. The automation dashboard remains
available at `/dashboard`; models, automations, settings, and local capabilities
are linked from the Chat sidebar.

Use the CLI as a Nola Server for clients on the same private network:

```bash
cargo run -- server --host 0.0.0.0 --port 5001
```

On first start, the CLI creates a stable Host ID and a random access token under
`~/.nola/cli/web/`. Both files are private to the current OS user (`0600` on
Unix). The startup output prints the access token so it can be entered into the
iOS, Android, or macOS Web Host configuration through a separate trusted
channel. The token and Host ID remain stable across restarts.

Loopback browser access remains local-trust. A remote browser is redirected to
`/login` and must enter the Host access token. The token is verified once and is
not stored in browser storage; the Host issues a bounded, expiring HttpOnly
session cookie and requires a session-bound CSRF token on state-changing
requests. Sign-out revokes the server-side session immediately.

Unauthenticated remote access is limited to health discovery, login assets, and
the independently authenticated `/api/mobile/*` surface. Mobile requests still
authenticate with HMAC-SHA256 and encrypt sensitive JSON and SSE events with
AES-GCM. Browser cookies do not provide transport encryption: use this mode only
on a trusted private network, or place the CLI listener behind an HTTPS reverse
proxy and set `NOLA_TRUST_HTTPS_PROXY=1` on the loopback-bound CLI process.

This serves the compiled Svelte single-entry Web UI (`app/static/ui/`) and
shared static assets (`app/static/`) from the Rust CLI process, alongside Rust
`/api/*` handlers for chat, agent streaming, conversations, automations,
results, settings, native host bridges, mobile clients, and the dashboard
summary. Unknown `/api/*` paths return a normal `404 not_found` response.

## Build

```bash
cd apps/cli
./scripts/build.sh         # 默认构建 release，并输出 target/release 下的完整原生 runtime
./scripts/build.sh --debug # Debug 构建，用于本地排查
./scripts/build.sh --clean --release
./scripts/build.sh --target x86_64-apple-darwin
```

原生 host 构建会同时放置 `nola` 和 `nola-agent-host`。Code Memory 作为 Rust
library 编译进这两个可执行文件，不需要额外的进程或 companion。
索引成功后，Host 会将当前配置允许的结构查询、符号搜索、源码读取、调用追踪及索引文本搜索工具暴露给下一轮模型，并返回项目名和查询指引。索引失败不激活这些工具；工具可见性不代替资源授权。

脚本默认使用 `apps/cli/Cargo.toml`，等价于：

```bash
cd apps/cli
cargo build --release
```

如需安装到本地 `cargo bin` 路径，可使用 `cargo install`：

```bash
cd apps/cli
cargo install --path . --bin nola
```

### Anthropic KV Prompt Cache

Nola CLI 在 Anthropic 路径会注入一条 `cache_control` 标记（默认开启），默认放在最新用户/助手消息上，保证与 web/claude-code 的单点标记策略一致；可配合 `NOLA_CLI_PROMPT_CACHE_SKIP_WRITE=true` 切到倒数第二条消息。

- 开关：`NOLA_CLI_PROMPT_CACHE`（默认 `true`）
- 关闭：`NOLA_CLI_PROMPT_CACHE=false` 或 `NOLA_CLI_PROMPT_CACHE=off`
- 可选 TTL：`NOLA_CLI_PROMPT_CACHE_TTL=1h`（其余值按不带 TTL 处理）
- 可选：`NOLA_CLI_PROMPT_CACHE_SKIP_WRITE=true`（等价于 web/claude-code 的 `skipCacheWrite`）
- 说明：开关仅影响 Anthropic 请求体构建，不影响 OpenAI 兼容通道。
- 额外：在命中缓存前缀的消息中，CLI 会给 `tool_result` 块补充 `cache_reference`，避免因工具输出参与上下文抖动导致缓存提前失效。

### OpenAI Prompt Cache

Nola CLI 在检测到官方 OpenAI Provider 时会在请求体里注入 `prompt_cache_key`，并可选 `prompt_cache_retention`，用于提升 KV 缓存命中。

通用配置优先级为 `CLI flags > config > defaults`。LLM endpoint、model、provider 和 API Key 均不从环境变量读取。OpenAI 缓存现在可由配置文件驱动（推荐）：

- `llm.openai_cache`：可选对象，包含下面字段
  - `enabled`: 开关（布尔）
  - `key`: 完整 `prompt_cache_key`（高优先于 `key_prefix`）
  - `key_prefix`: key 前缀，默认 `nola-openai`
  - `retention`: `prompt_cache_retention`，例如 `24h`、`in_memory`
- 快速开关：也可使用环境变量
  - `NOLA_OPENAI_PROMPT_CACHE`（默认 `true`）
  - `NOLA_OPENAI_PROMPT_CACHE_KEY`
  - `NOLA_OPENAI_PROMPT_CACHE_KEY_PREFIX`（默认 `nola-openai`）
  - `NOLA_OPENAI_PROMPT_CACHE_RETENTION`
- 说明：仅在 `detect_chat_provider` 命中 `OpenAiFlavor::OpenAi` 时生效；`openai` 兼容通道（如 DeepSeek/通用兼容层）不受影响。
- 说明：`llm.openai_cache` 与环境变量都支持 `true/false/on/off/yes/no/1/0` 布尔值。

> 提示：首次或新机器编译前请先确认已安装 Rust 工具链（`rustup` + `cargo`），并通过 `rustup` 安装目标平台，如 `rustup target add x86_64-apple-darwin`。

`init` scans the repository, calls the configured LLM, and writes a `NOLA.md` file that includes concise agent instructions plus an inline project wiki. Existing output files are not overwritten unless `--force` is passed. Use `--dry-run` to preview the generated markdown. `wiki` remains available when you want a standalone wiki file.

Runtime configuration is loaded from:

- `~/.nola/config.json` or `~/.nola/config.jsonc`
- `.nola/config.json` or `.nola/config.jsonc` in the project root
- `nola.json` or `nola.jsonc` in the project root

CLI flags win over config files. LLM endpoint, model, provider, API Key, permissions, and behavior settings are not read from environment variables. Supported config keys include `llm.endpoint`, `llm.api_key`, `llm.model`, `llm.provider`, `llm.openai_cache`, `allowed_dirs`, `bash_prefixes`, `max_iterations`, `force_tool_set`, `network_policy`, `instructions`, `permissions`, `agents`, `compaction`, and `debug`.

CLI 工作区目录用于默认路径和项目上下文，不是文件系统沙箱。普通文件读取和已获批的命令不会仅因目标位于工作区外而被阻断；操作系统权限、显式权限规则以及覆盖/删除等操作审批仍然有效。敏感文件（例如 `.ssh`、`.env`、密钥文件）按具体读取资源审批。模型判断普通命名的文件包含私人或敏感数据时，可在文件或 Shell 工具调用中填写 `sensitive_read_reason` 主动请求审批；该字段不能降低系统判定的风险，也不代表用户已经批准。`workspace_access` 仅用于需要目录授权的原生宿主，不是 CLI 跨目录读取的前置步骤。

例如，模型读取私人文档时可调用 `read_file` 并传入：

```json
{
  "path": "/Users/example/Documents/medical-notes.txt",
  "sensitive_read_reason": "该文档含有私人医疗信息，需要确认读取。"
}
```

谨慎模式按具体操作及已有有效授权决定是否确认；全部允许模式仍不产生应用确认。批准命令不代表将后续所有命令视为只读，覆盖和删除仍按操作风险处理。已运行的 CLI 需退出后重启以加载重建的二进制，无需为跨目录访问切换到全部允许。

权限面板 `2. 本会话允许` 支持有效的进程调用，授权范围是当前会话与 Host 下的具体命令、工作目录及参数，不是整个 Shell 工具，也不会自动扩成命令族。后续仍解析资源、检查观察风险及显式限制；敏感证据变化会重新确认。授权执行脚本不等于冻结脚本依赖或数据内容。未提供会话授权选项的挑战保留编号 2 并标注不可选，快捷键提示同步省略 2。

普通 Shell 源码检索由 Host 检查实际 argv 与资源：支持字面量 `grep`、`head`/`tail`、`cat`/`wc`、目录列表、固定数字行范围的 `sed -n`，以及受支持命令之间的管道、`;`、`&&`、`||`。系统程序路径固定，执行环境隔离；递归检索有完整资源检查与数量上限，敏感目标（含符号链接）仍审批。`operation_kind=inspect` 不是授权，任意脚本、展开、重定向写入或未知参数仍保留确认；可用结构化检索工具表达这些读取需求。`grep` 无匹配的退出码 1 仍按真实命令结果报告，不等于权限被拒绝。

规范与测试范围见[权限边界契约](../../docs/architecture/模块/07-权限与平台/PERMISSION_BOUNDARY_CONTRACT.md#filesystem-scope-and-model-requested-sensitive-reads)。真实模型回归脚本为 `python3 scripts/test_real_cli_external_resources.py`（在仓库根目录执行）；它创建隔离配置、数据库和测试文件，不修改现有用户会话。



默认情况下，CLI 会用精简交互日志：只显示“正在思考”和真实工具执行状态，不输出 `prepare` / `plan` / 内部控制工具、工具参数或响应详情，适合直接与用户交互。
要记录完整明细（LLM 请求参数、工具参数、重试/降级/流式读取异常等）用于排查，可在 config 开启 `debug`（`~/.nola/config.json`、项目 `.nola/config.json` 或 `nola.json`）：

```jsonc
{
  "debug": false
}
```

`true` 时会开启更完整的终端交互（包含完整状态/工具参数）并写本地调试日志。效果等同于运行时传 `--debug`。

本地日志默认路径优先级：

- `NOLA_CLI_DEBUG_LOG_PATH`
- `~/.cache/nola/cli-debug.log`
- `~/.nola/cli-debug.log`
- `~/.nola/logs/cli-debug.log`
- `<project-root>/.nola/cli-debug.log`

精简示例（默认）：

- `状态  decide 正在思考`
- `工具  running read_file`
- `完成  read_file ok`

交互式输入界面（默认启动）会在输入栏下方显示运行上下文，包含：

- 当前模型 / endpoint（可通过 `--model`、`--force-tool-set`、`--network-policy` 影响）
- 当前会话参数（`max_iterations`、工具策略等）
- 当前工作目录与工作区路径（`project_root`）

交互视口支持 `PgUp` / `PgDn` 翻页（保留一行重叠），`Ctrl+Home` 到历史开头，`Ctrl+End` 返回最新内容。浏览历史时底部显示返回提示，流式输出不会抢走阅读位置；普通 `Home` / `End` 仍用于编辑输入，审批面板打开时由面板接管按键。

活动状态仅重绘变化的终端行，保留输入光标与已显示内容。可用 `NOLA_REDUCE_MOTION=1 nola` 关闭循环转圈动画，改用静态标记；状态文字、耗时、完成与错误反馈仍保留。此选项在进程启动时读取，不会更改系统动画设置。

此外，模型返回的 Markdown 代码块（```）会在终端里做轻量语法高亮，关键类型/关键字/字符串会有颜色区分，便于快速阅读。

Example:

```jsonc
{
  "llm": {
    "endpoint": "http://192.168.31.18:8080/v1",
    "model": "qwen3.6-35b-a3b",
    "provider": "llamacpp",
    "openai_cache": {
      "enabled": false,
      "key": "project-openai-cache-key",
      "key_prefix": "project",
      "retention": "24h"
    }
  },
  "allowed_dirs": ["."],
  "bash_prefixes": ["git", "rg", "cargo test"],
  "network_policy": "allow",
  "permissions": {
    "bash": {
      "git status*": "allow",
      "rm *": "deny",
      "*": "ask"
    },
    "edit": "ask",
    "web": "allow"
  },
  "agents": {
    "review": {
      "description": "Focused code-review sub-agent",
      "prompt": "Review the diff for correctness risks. Findings first.",
      "tools": ["git_diff", "read_file", "grep", "diff_review"],
      "max_turns": 6
    }
  },
  "compaction": {
    "token_threshold": 100000,
    "keep_recent_messages": 10
  },
  "debug": false
}
```

Instruction context is loaded automatically from `NOLA.md`, `NOLA.local.md`, `AGENTS.md`, `AGENTS.override.md`, and `.nola/rules/*.md`, plus any explicit `instructions` entries in config. Markdown frontmatter and HTML comments are stripped, `@relative/path.md` includes are expanded with a depth cap, and oversized files are truncated before entering the model context.

Useful options:

- `--endpoint URL`: provider endpoint. Otherwise read from `llm.endpoint` in Nola config.
- `--api-key KEY`: API key. Otherwise read from `llm.api_key` in Nola config.
- `--model MODEL`: model name. Otherwise read from `llm.model` in Nola config. DeepSeek endpoints default to `deepseek-chat`.
- `llm.provider`: optional provider hint in Nola config when auto-detection is ambiguous. Supported values: `openai`, `deepseek`, `llamacpp`, `vllm`, `dashscope`, `anthropic`.
- `--allow-dir PATH`: directory tools may access. Repeatable; defaults to the current directory.
- `--bash-prefix PREFIX`: allowed bash command prefix, for example `git`, `rg`, `cargo test`.
- `--yes`: approve model-network confirmation for `init` / `wiki` only. It never grants Agent tool permission; select the conversation's `cautious` or `allow_all` permission profile explicitly.
- `--json`: print newline-delimited JSON events.
- `--debug`: enable detailed terminal interaction (full status/tool traces) and write local debug logs for LLM/tool interactions (same as `"debug": true` in config).
- `--max-iterations N`: cap tool-capable model requests, then allow one tool-free final synthesis; unlimited by default, and `0` also means unlimited.
- `--force-tool-set MODE`: choose exposed tools. `auto` sends a focused coding set plus `tool_search`, `all` sends every tool schema, and `write` sends the SWE write-focused set.
- `--agent-mode MODE`: set agent mode explicitly (`swe` or `standard`).
- `--swe-mode`: shortcut for `--agent-mode=swe`.
- `--session ID`: load and save a named session.
- `--continue`: continue the latest saved session.
- `--no-save`: run without writing a session file.
- `--root PATH`: project root for `init` / `wiki`; defaults to the current directory.
- `--output PATH`: output file for `init` (`NOLA.md`) or `wiki`.
- `NOLA_OPENAI_PROMPT_CACHE`: optional `true/false/on/off/1/0`, controls OpenAI 官方 KV 缓存注入（默认 `true`）。
- `NOLA_OPENAI_PROMPT_CACHE_KEY`: 完整 `prompt_cache_key`，覆盖 `key_prefix` 自动组合。
- `NOLA_OPENAI_PROMPT_CACHE_KEY_PREFIX`: key 前缀（默认 `nola-openai`）。
- `NOLA_OPENAI_PROMPT_CACHE_RETENTION`: 可选 `prompt_cache_retention`，例如 `24h`、`in_memory`。
- `--force`: overwrite existing `init` / `wiki` output files.
- `--dry-run`: preview `init` / `wiki` output without writing files.

Implemented tool groups:

- Local coding: `read_file`, `write_file`, `edit_file`, `multi_edit_file`, `list_dir`, `glob`, `grep`, `bash`, `propose_agent_plan`, `update_agent_todos`.
- Browser (Chrome extension): `chrome_native_status`, `chrome_native_inspect`, `chrome_native_open_page`, `chrome_native_screenshot`, `chrome_native_dom_snapshot`, `chrome_native_accessibility_snapshot`, `chrome_native_storage_state`, `chrome_native_console_logs`, `chrome_native_network_events`, `chrome_native_list_pages`, `chrome_native_switch_page`, `chrome_native_hover`, `chrome_native_click`, `chrome_native_drag`, `chrome_native_type_text`, `chrome_native_select_option`, `chrome_native_set_checked`, `chrome_native_press_key`, `chrome_native_scroll`, `chrome_native_assert`, `chrome_native_wait_for`.
- Desktop computer-use: `computer_status`, `computer_observe`, `computer_list_displays`, `computer_screenshot`, `computer_screenshot_window`, `computer_screenshot_element`, `computer_read_text`, `computer_find_text`, `computer_move_to_text`, `computer_click_text`, `computer_assert_text`, `computer_wait_for_text`, `computer_wait`, `computer_accessibility_snapshot`, `computer_element_at`, `computer_press_at`, `computer_type_at`, `computer_scroll_at`, `computer_move_to_element`, `computer_click_element`, `computer_press_element`, `computer_focus_element`, `computer_perform_element_action`, `computer_type_element`, `computer_set_element_value`, `computer_assert_element`, `computer_wait_for_element`, `computer_get_position`, `computer_move_mouse`, `computer_mouse_down`, `computer_mouse_up`, `computer_click`, `computer_drag`, `computer_drag_element`, `computer_type_text`, `computer_get_clipboard`, `computer_set_clipboard`, `computer_copy_selection`, `computer_assert_clipboard`, `computer_wait_for_clipboard`, `computer_press_key`, `computer_hotkey`, `computer_key_down`, `computer_key_up`, `computer_scroll`, `computer_scroll_element`, `computer_list_apps`, `computer_assert_app`, `computer_wait_for_app`, `computer_list_windows`, `computer_focus_window`, `computer_set_window_bounds`, `computer_perform_window_action`, `computer_close_window`, `computer_assert_window`, `computer_wait_for_window`, `computer_list_menus`, `computer_select_menu`, `computer_activate_app`, `computer_quit_app`, `computer_open_item`.

`grep` supports `context_lines` for nearby source context. `read_file` returns line-numbered output by default and supports `start_line` plus `line_count` for focused reads of large files. Use `multi_edit_file` when several exact replacements are needed in one file; it writes only after every replacement succeeds. `propose_agent_plan` publishes a structured execution plan, while permission prompts independently gate high-impact actions; `update_agent_todos` keeps the current work checklist visible to the model.

In the Core permission panel, press `I` to add an instruction, or enter
`i <instruction>` in line-input mode. Saving the instruction keeps the original
tool call paused and shows another permission prompt. The tool arguments remain
unchanged; review them before explicitly approving or denying the call. Guidance
is limited to 4 KiB per entry and 16 entries within a 16 KiB total budget.
The separate Return to composer action cancels the current turn.

Browser tools communicate exclusively with the user-installed Chrome extension through the native messaging bridge. Nola does not install or launch Playwright, headless Chromium, or a system-browser fallback. Use `chrome_native_status` before a browser workflow, inspect the active tab after each navigation or mutation, and use `chrome_native_assert` / `chrome_native_wait_for` to verify results. Public static content and internet search remain available through `web_fetch` and `web_search`.

Computer-use tools operate the host desktop. On macOS they use `open`, `screencapture`, Quartz input events, System Events, Accessibility metadata, and `pbpaste`/`pbcopy`, so the terminal may need Screen Recording, Accessibility, and Automation permissions. Start with `computer_observe` for combined display, window, accessibility, cursor, visible-text, and screenshot context. Use `computer_list_displays` for exact display bounds, `computer_screenshot` for a display or crop, `computer_screenshot_window` for a known window, `computer_read_text` / `computer_find_text` for visible text bounds, and `computer_screenshot_element` for an accessibility-element crop. Prefer `computer_press_element`, `computer_focus_element`, `computer_perform_element_action`, and `computer_set_element_value` when Accessibility exposes reliable actions or values; use `computer_move_to_element`, `computer_click_element`, `computer_drag_element`, `computer_scroll_element`, `computer_type_element`, `computer_element_at`, `computer_press_at`, `computer_type_at`, or `computer_scroll_at` when you need point or element-specific targeting. `computer_click`, `computer_click_text`, and `computer_click_element` support true native double/triple click via `clicks=2` or `clicks=3`. Use `computer_hotkey`, `computer_key_down`, and `computer_key_up` for shortcut and key-hold workflows, and verify with `computer_assert_*`, `computer_wait_for_*`, `computer_get_clipboard`, or `computer_wait` after UI changes.

## Interactive controls

Interactive commands are recognized only when the submitted line starts with `/`; ordinary words such as `init`, `resume`, and `continue` remain normal model input.

- `/resume` opens the session restore page; `/continue` resumes the latest project session.
- `/history` replays the complete user/assistant session view.
- `/new` starts a clean session in the current workspace.
- `/goal TARGET` runs an autonomous goal until completion or a genuine user blocker. Use `/goal status`, `/goal resume`, and `/goal off` to inspect, resume, or cancel the current session goal.
- `/plan` shows the current session execution checklist without calling the model.
- `/compact` compacts model context while preserving the complete replay transcript.
- `/cd PATH` changes the persistent CLI workspace and starts a project session there.
- `/jobs`, `/jobs ID`, and `/jobs stop ID` list, inspect, and stop persisted background tasks.
- `/init` creates or updates project guidance and long-term project memory in `NOLA.md`.

While an Agent run is active, type a supplemental instruction and press Enter to queue it. Press Esc to interrupt the current execution. Long-running servers, watchers, builds, and similar jobs are available in ordinary tasks as well as `/goal`; the model can select background tools explicitly and the runtime can promote clearly persistent shell commands automatically.

Transient model failures are retried only before any text or reasoning delta reaches the user. Configure the policy with `NOLA_LLM_MAX_RETRIES` (default `2`), `NOLA_LLM_RETRY_INITIAL_DELAY_MS` (default `2000`), and `NOLA_LLM_RETRY_MAX_DELAY_MS` (default `120000`). Numeric `Retry-After` headers take precedence. Authentication, invalid-request, context-limit, cancellation, and partial-stream failures are never replayed.

## Localization

The CLI follows `NOLA_CLI_LANG`, then `NOLA_LANG`, then the system locale variables. It supports the same common UI languages as the mobile clients: `zh-Hans`, `en`, `de`, `es`, `fr`, `ja`, and `ru`. Unknown locales fall back to English. JSON event field names, tool names, and model-facing tool schemas stay in English for script and LLM compatibility.

```bash
NOLA_CLI_LANG=zh-Hans cargo run -- --help
NOLA_CLI_LANG=ja cargo run -- sessions
```

Sessions are stored in the current schema-epoch database (currently `~/.nola/nola-v4.db`) by default together with tool events and resumable task state. Advancing the Store epoch selects a new default file instead of reopening or migrating an incompatible database. Set `NOLA_DB_PATH` to override the SQLite path. Persisted conversation/tool payloads are sanitized before storage; API keys remain in environment or config sources rather than session messages.

### 运行期间的终端命令

复制图片后按 `Ctrl+V` 添加附件（macOS 终端同样使用此快捷键），或单独粘贴图片绝对路径。发送前删除图片占位符即可移除附件；分析图片需要所选模型声明图片输入能力。空闲输入和运行期间的后续消息都支持添加图片。

权限拒绝后，用 `/allow` 查看本会话权限；配置中的显式 `deny` 规则需修改对应配置，操作系统权限需在系统设置中授予，调整后明确要求重试。底部缓存命中率指最近一次模型请求，`—` 表示服务端未上报完整数据；0% 只表示该次请求未命中。

输入以 `/` 开头的命令会走客户端控制路径，不进入后续消息队列，也不会发给模型。
运行期间 `/allow`（或 `/a`）立即打开权限面板，`/help`、`/status`、`/history` 立即显示。
其他命令会立即提示先按 Esc 返回输入后执行，不会悄悄排队。
运行期间选择的权限模式在下一轮生效；当前待审批操作仍需单独确认。
权限面板的方向键按实际选项数量循环，无效数字不会选中不存在的项。
