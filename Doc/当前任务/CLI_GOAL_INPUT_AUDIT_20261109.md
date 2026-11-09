# CLI 最新会话与 Goal 输入排查（2026-11-09）

只读检查本机最新 CLI 会话：10 个用户回合、91 个工具部分。没有重放或修改原会话、数据库、账户凭据。原始文本、工具输出和数据库路径不放入版本库。

## 会话证据

- 1 个用户中断，不能统计为模型失败。工具中另有 8 个 failed：code index 不可用、rg 不在运行环境 PATH、2 次网页抓取地址校验拒绝、搜索结果缺乏有效支持、管道截断 SIGPIPE、后台状态 max_bytes 越界等，原因各不相同。
- Goal 自动附加“仅限当前 scripts 目录”等约束；模型因此停止访问同项目源码并请求扩展范围。这些约束不是用户输入。
- Goal 原文与回合 contract objective 不同：后者保存了整段 autonomous prompt。Goal 最后的保存状态仍 running，而模型已经报告 needs_user；本次不能仅凭最后回复断言状态机已正确收敛。
- 普通修改请求多次以建议、解释和后续计划结束，未交付改动。一次隔离终端探测证明发出了备用屏幕序列，不能证明原终端布局已验证或已修复。

## 本次修复

1. 无目标时进入现有 composer 的独立 Goal 输入状态；展示用途、示例、提交与取消提示，直接输入目标原文。Esc 返回普通聊天；空输入不启动执行。
2. 保留多行目标和连续空格，避免 split_whitespace 改写粘贴内容。
3. 约束保留换行；拒绝只有约束、没有目标，以及缺少约束值的参数。
4. 参数识别忽略引号内示例，支持 `--` 字面量；自然语言中的撇号不吞掉后续参数。
5. `start` 允许保留词作为目标，pause/cancel 不再误作目标。
6. 移除 CLI 编造的目录限制、禁止临时文件、所有网络/测试均需确认等默认约束。
7. 使用共享控制 intent 和当前 Goal ID/control token；区分排队与应用，拒绝后不伪报成功。
8. 当前会话 resume 不跳到同项目的其他最近会话。
9. 有未结束目标时不再悄悄忽略新目标，保留新草稿并显示恢复/取消指引。
10. 持久终态优先于本地残留目标变量；终态后允许显式启动新目标。
11. 回合 contract objective 保存 Goal 原文，运行指导单独传入模型。
12. slash 控制错误返回输入区并保留草稿，不结束 CLI。
13. 补齐后台 status/wait max_bytes schema 的 1000–100000 边界，与实际校验一致。
14. Goal 提示遵循当前权限和授权；模糊目标先澄清，不强行继承此前无关任务；调查和回答不强制变成代码修改。

## 参考实现

阅读本机 Codex 参考版本：`tui/src/chatwidget/goal_menu.rs` 的编辑输入与显式状态操作；`tui/src/chatwidget/tests/goal_validation.rs` 的多行原文、输入验证与不隐式提交普通消息。采用现有 Rust composer 和共享 Core 控制入口，没有引入第二套 Goal 状态或复制源码。另读 OpenCode 的 `packages/opencode/src/cli/cmd/tui/ui/dialog-prompt.tsx`：明确区分确认、取消和 busy，忙碌时禁用提交并恢复焦点。Nola 沿用已有 composer 和控制回执，避免另建文本输入状态。参考目录是本机版本，不代表远端最新版。

交互取舍：终端复用现有颜色、字号和输入区，目标原文占主要位置，控制状态用文字说明；不额外增加遮挡对话的模态层。apple-design 的 entering-data（必填验证）、focus-and-selection（避免无交互改变焦点）用于原则检查；终端具体实现是本项目工程判断。

## 验证

- `cargo test -p nola_application --lib goal_coordinator -j 2 --quiet`：3 passed。
- `RUST_MIN_STACK=33554432 cargo test -p nola_cli --lib tests::goal_ -j 2 --quiet`：18 passed（含当前会话恢复与共享 Web 权限中断测试）。
- 当前 CLI 测试二进制 `terminal_ui::`：75 passed。
- 当前 CLI 测试二进制 `tests::background_output_schema_matches_the_actual_validation_limits`：1 passed。
- `cargo build -p nola_cli --bin nola -j 2 --quiet`：成功；链接器有现有 unwind table 大小警告。
- 独立 PTY（隔离 NOLA_HOME、数据库和空项目；跳过首次引导）：进入备用屏幕、`/goal` 草稿、无效目标不启动 Run、slash 错误后仍可输入、60×16 缩放、`/exit` 退出恢复原屏幕均通过。原用户库不参与。
- 较宽的首轮 `goal_`：37 passed / 1 failed；暂停后第二次 HTTP 请求用例触发 20 秒超时，单独复查仍超时。采样显示主要在 checkpoint 加载/JSON 序列化，未证明锁死。诊断性延长第二次请求等待至 60 秒后，复查又在首次请求的 20 秒等待处超时，因此仍不能确认恢复端到端成功。该诊断性测试改动已撤销，保留原验收时限与失败证据；没有用放宽门槛宣称修复。

本轮 HTTP fixture 测试不等于真实远端模型测试。

## 保留问题

网页地址校验不应为此次排查而放开；需单独核查 DNS/代理和目标站点。rg 缺失需定位工作进程实际 PATH；不能通过安装依赖或假报可用来掩盖。code index 的 native provider 不可用仍需要恢复能力或明确降级。SIGPIPE 是实际管道退出，不应一概改成成功。原会话 Goal needs_user 与 running 差异需要在当前构建独立重现，不能直接改写用户存储来“修正”。未声称已完成所有终端和真实模型交互验收。

### 未通过项的具体证据

暂停/恢复 HTTP fixture 失败不属于目标文本解析。隔离数据库显示 Run 已建立，模型 effect 已进入 leased/execution_started，但在测试期限内未观察到 provider 请求；需要进一步定位 Host 恢复和请求准备的耗时。约 1.55 MB 的 checkpoint 与反复 JSON 读取/哈希是采样发现，尚未建立完整因果关系。不得因此绕过冻结配置校验、租约或原始数据保护。此项仍未解决，本轮不是全绿验收。


### 2026-11-10 后续复测

上述暂停/恢复超时已在后续修复后通过原有验收：`goal_` 共 39 项通过（204.46 秒），包含恢复后的第二次实际 HTTP fixture 请求，未延长每次请求的 20 秒限制。终端测试 75 项通过。Core 四个共享包共 1460 项通过，精确 Run 恢复数据库集成 9 项通过。

完整采样进一步定位到开发构建的 SHA-256 软件内核，以及恢复阶段重复读取、摘要和解码大型检查点。已启用 ARM SHA 指令、优化开发/测试摘要内核，并在同一快照和事务内传递已校验配置及检查点。CLI 元数据缓存改为结构化键，终端重复内容统计直接比较原文；独立恢复读取仍校验持久化摘要，修改原始配置字节的回归仍拒绝。修复不改变既有格式和权限边界。

该结果关闭本页记录的 HTTP fixture 超时项；不等于完成真实远端模型或所有终端环境验收。
