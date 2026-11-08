> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 外部执行单元与消息渠道参考审计

更新日期：2026-08-14

## 结论

Nola 不把 Codex、Claude Code 或 OpenCode 当成普通模型供应商，也不解析它们面向终端用户的彩色文本。Host 发现本机安装与认证状态，使用各产品公开的非交互结构化输出协议启动独立进程，将原生会话 ID 按 Nola 会话保存，并把结构化文本、思考、工具、错误和终态归一化为 Agent Core 事件。客户端只选择 Host 返回为可用的执行单元。

消息渠道采用官方机器人或公众号接口，不模拟个人微信客户端。提供方 webhook 先完成签名验证和 provider event 去重，再持久化到 inbox；Host 以有期限租约领取，调用所选执行单元，最后通过持久化 outbox 调用提供方发送接口。凭据只以服务端 secret 引用保存。

## 官方依据与采用方式

| 系统 | 官方能力 | Nola 采用方式 |
|---|---|---|
| Codex CLI | `codex exec` 支持脚本化执行、JSONL 事件以及按 thread/session 恢复；审批和 sandbox 是显式运行参数 | 使用 JSONL 事件，不抓取 TUI；保存 thread ID；Nola 权限边界不被外部 CLI 的显示文本替代 |
| Claude Code | `claude -p` 提供 `stream-json`，`--resume` 恢复指定 session，非交互模式有独立权限参数 | 使用 `stream-json` 和原生 session ID；工具块按结构化事件显示 |
| OpenCode | `opencode run --format json`，`--session` 恢复，亦可连接常驻 `opencode serve` | 当前使用进程模式；保留以后切换常驻服务以降低 MCP 冷启动的协议空间 |
| Unsloth Studio | 本地 Web UI、预编译 llama.cpp、MLX/GGUF 生命周期、工具输出面板、运行计时 | 借鉴本地服务状态机、明确启动阶段和工具输出可观测性；它不是 Codex/Claude/OpenCode 编排协议来源 |
| Telegram | Bot API 支持 HTTPS webhook、`secret_token` 请求头、递增 `update_id` 和 `sendMessage` | 校验 secret header，以 update ID 去重，持久化后异步回复 |
| WhatsApp | WhatsApp Cloud API 使用 Meta webhook 验证、签名回调与 Graph API 消息发送 | 校验 verification challenge 与请求签名，以 provider message ID 去重 |
| 微信公众号 | 官方服务器配置通过 token/signature 验证，消息使用 XML 回调，客服消息通过官方接口发送 | 仅支持公众号，不宣称个人微信自动化；签名验证后规范化为统一 envelope |
| 企业微信 | 回调 URL 验证与加密消息签名，应用消息使用企业微信官方接口 | 独立 `we_com` 类型，不与微信公众号或个人微信混用 |

## 不采用的方案

- 不用关键词决定执行单元、工具或渠道。
- 不用 shell 彩色输出、光标控制序列或自然语言提示推断工具状态。
- 不把外部 CLI 的 `--dangerously-*` 模式作为默认权限策略。
- 不在 iOS、Android、Mac 分别保存渠道密钥；移动和桌面客户端通过安全 handoff 打开 Host 管理面。
- 不把没有官方通用机器人 API 的个人微信注入、Hook 或桌面自动化包装成正式能力。

## 验证标准

1. 每个已安装且已认证的执行器必须完成两轮真实请求，第二轮使用同一原生 session，并精确返回随机令牌。
2. 未安装或未认证执行器必须显示明确状态，不能静默回退到 Nola。
3. 渠道测试必须覆盖合法签名、非法签名、重复事件、租约领取、续租、完成、失败恢复和实际回复发送。
4. 至少两个真实外部执行器必须完成渠道到 Host 再到 LLM 的双轮续接。
5. 统一 E2E 报告必须包含结构化 provider trace、事件流和持久化审计；单独脚本退出码为零不构成完成证据。

## 2026-08-13 权威转录与渐进渲染审计

- macOS 会话归档是展示缓存。恢复与审计以 Agent Host SQLite 的
  `session`、`message`、`agent_run`、`trace_event` 和 `tool_event` 为准。
- 会话重放兼容 Core 毫秒时间戳，也兼容缺少逐条时间戳的旧 Host 记录。
  旧记录获得确定性的展示顺序；当前 Host 的持久化时间戳保持不变。
- Core 不公开尚未确认、之后可能转为工具调用的候选正文。正文确认完成后，
  再按 Markdown 安全的句末、段落或长度边界，无损生成连续的
  `assistant.delta` 事件。
- Core Part projection 是持久恢复快照；Agent Trace delta 是实时正文的唯一
  传输通道。禁止二者同时修改实时正文，避免终局快照抢先覆盖流式展示。
- 修复后的 macOS 真实验收记录：914 字正文、5 个 assistant delta、5 次正文
  mutation、3 次 mutable-tail 更新、1 个 Markdown 表格、1 个代码块、3 个可见
  文件工具，committed prefix 身份稳定，工具生命周期配对，持久化 Trace 序号连续。
  产物：`/tmp/nola-mac-real-e2e.581guu`。
- Android、iOS Simulator 与 Web 外部执行的真实 provider 路径通过。
  产物：`/tmp/nola-cross-platform-stream-1786616623`。
- Codex 与 Claude 的真实外部执行和消息渠道验证通过，覆盖官方 webhook 签名、
  去重、持久化和回复投递。该轮 OpenCode 尚未安装，目录明确显示不可用且未静默回退；
  后续安装后的真实验证见下方当前验收记录。
  产物：`/tmp/nola-real-e2e-1786615058`。

## 2026-08-13 后续记录根因与闭环

- 实际 Agent Host 会话 `180F5E49-5DF3-4F59-BE38-C71F2B6FB860` 在 SQLite 中
  保留了完整的两轮用户消息、两条最终回答和五次工具调用；旧 Mac 展示归档只剩
  三条消息，并把第二轮回答排到了第二轮用户消息之前。Mac 现在把归档视为缓存，
  进入会话时按 Host 的稳定消息归属、序号和时间戳修复，而不再以本地到达时间重建。
- 工具回合中的模型文本始终是私有候选。Core 必须再发起一次 `tools=[]` 的独立最终
  综合请求，但审计的是“确实跨过无工具请求边界”，不能因为综合后的文本碰巧与候选
  相同就判定重放。只拒绝最终综合中仍携带工具调用协议的非法结果。
- 流式文本使用 Core 稳定 Part ID，而兼容模型在 completion 中可能返回 provider
  message ID。投影层现在将 completion 合并回同一 Turn 的开放文本 Part，避免创建
  第二个非空正文。被撤回的候选允许保留空协议墓碑，但每个 Turn 只能存在一个非空
  权威正文。
- Mac 启动崩溃来自 `MacAutomationStore.shared` 与 `MacWebSourceStore.shared` 的递归
  `dispatch_once` 初始化。Web 数据源依赖改为延迟解析；当前 Mac 测试启动未再触发该
  崩溃。
- Core Host 合约 43/43、Agent Trace 13/13、Mac 会话和产品路径 22/22 通过。
  Android 真实 Host 的五个场景及 iOS 真实 Host 的四个场景全部通过，覆盖普通工具、
  破坏性权限、取消恢复、长期记忆写入及跨会话召回。
- 外部执行与渠道阶段验收产物：`/tmp/nola-final-executor-messaging-1786620524`。
  Codex 和 Claude Code 均完成双轮真实续接及 session 持久化；Telegram、WhatsApp、
  微信公众号、企业微信完成签名、非法签名拒绝、去重、持久投递。该阶段 OpenCode
  未安装，目录正确返回 `not_installed`；安装后的真实验收记录如下。

语义评审必须独立于传输评审。一次并行检索运行虽然流式、工具和持久化全部
正确，但最终答案仍列出了用户明确要求排除的候选，因此被真实模型 Judge 判定
失败。传输成功不能被当作回答质量成功。

## 2026-08-14 跨平台真实验收补充

- macOS 使用当前生产代码完成 Agent Host 设置握手、稳定 Host 身份、消息渠道状态、
  安全管理链接、无效令牌本地化错误与远程取消协议验证，5/5 通过。结果：
  `/Users/gaojc/Library/Developer/Xcode/DerivedData/Nola-etfchqtmfngzxnfmkehixzbnbyhd/Logs/Test/Test-NolaMac-2026.08.14_9-03-30-+0800.xcresult`。
- iOS Simulator 通过真实 Web Host 与真实配置的 LLM 完成生产聊天 UI 验收：发送后立即
  出现回复容器、渐进 Markdown、工具生命周期、单一最终气泡、重启后消息恢复，以及
  Host 消息渠道状态卡和安全管理入口。结果：
  `/Users/gaojc/Library/Developer/Xcode/DerivedData/Nola-apsmpvzsqnqaexaofahbqumoivdy/Logs/Test/Test-Nola-2026.08.14_8-57-50-+0800.xcresult`。
- Android API 35 模拟器分别通过执行单元标题栏选择器和消息渠道模型中心测试。执行单元
  目录会禁用不可用项；Telegram、WhatsApp、微信公众号、企业微信均有可见状态和管理入口。
- Web 新增生产 Host + Chromium 浏览器级验收。测试通过登录页进入设置，关闭真实首启
  引导，确认四类渠道与 Nola、Codex、Claude Code、OpenCode 四个执行单元，通过 UI
  保存 Codex 后台执行单元和 Telegram 连接，并验证刷新持久化与 secret 不回传。用例：
  `web/tests/test_agent_host_full_e2e.py::test_real_web_messaging_gallery_persists_connection_and_exposes_execution_units`。
- Web Host 使用当前 Release 二进制完成 Codex 0.147.0、Claude Code 2.1.143 与
  OpenCode 1.18.18 两轮真实调用和原生 session 精确恢复。三者当前证据保存在
  `/private/tmp/nola-external-web-oh9ib9ql`，其中持久化审计明确记录三种执行单元的
  `sessionPersisted=true` 与 `resumeExact=true`。
- OpenCode 还通过官方 `opencode run --format json` 直接调用真实 OpenAI-compatible
  模型端点并返回精确 token；Mac 原生 Host 随后通过
  `MacExternalExecutionRealEndToEndTests` 完成三种执行单元的真实双轮恢复，结果包为
  `/tmp/NolaMacExternalExecutors-1786714241.xcresult`。官方安装器在本机未采用测试传入的
  临时安装目录，而是安装到 `~/.opencode/bin`，审计因此记录实际发现路径，不声称隔离安装。
- 渠道到外部执行单元的组合验收已分别使用 Codex、Claude Code 和 OpenCode 运行两轮
  真实模型调用：第一轮只返回 `ACK`，第二轮从相同渠道会话恢复并精确返回上一轮保存的
  marker。验收同时发现并修正了旧测试中“prompt 要求 `ACK: marker`、断言却要求 `ACK`”
  的自相矛盾，避免把严格遵循指令的执行单元误判为失败。
- Telegram、WhatsApp Cloud API、微信公众号和企业微信的签名、拒绝非法签名、去重、
  租约、持久 outbox 与回复投递由本地官方 wire-contract 服务完成。当前机器没有配置这些
  提供方的生产凭据，因此本文不宣称向真实 Telegram/Meta/微信账号完成了线上投递；真实
  LLM 与真实外部执行单元调用已经完成，生产账号验收需在提供方凭据可用后另行执行。
- 消息渠道全栈重新验证 Telegram、WhatsApp Cloud API、微信公众号和企业微信的官方
  wire contract，覆盖四次 ingress、四次 durable delivery 和 Host 管理面。Telegram
  另以真实 Codex 完成两条入站消息、两条 provider 回复、四个 provider draft 和同一
  对话续接；Claude Code 也通过通用渠道协议完成两轮真实回复和同一会话恢复。没有生产
  渠道密钥的提供方使用官方线协议本地模拟器，不冒充真实公网账号。

CLI 没有桌面标题栏，对应交互是 `/executor` 和 footer 当前执行单元；Web、iOS、Android
与 macOS 则在对话标题工具栏附近选择。所有客户端只消费 Host/Core 的结构化目录，不按
用户文本关键词推断执行单元，也不在不可用时自动降级到 Nola。

## 官方资料

- OpenAI Codex CLI：<https://developers.openai.com/codex/noninteractive>
- OpenAI Codex 命令参考：<https://developers.openai.com/codex/cli/reference>
- Anthropic Claude Code CLI：<https://docs.anthropic.com/en/docs/claude-code/cli-usage>
- OpenCode CLI：<https://opencode.ai/docs/cli/>
- Unsloth Studio：<https://github.com/unslothai/unsloth>
- Telegram Bot API：<https://core.telegram.org/bots/api>
- Meta WhatsApp Cloud API：<https://developers.facebook.com/docs/whatsapp/cloud-api/>
- 微信公众平台开发文档：<https://developers.weixin.qq.com/doc/offiaccount/Getting_Started/Overview.html>
- 企业微信开发文档：<https://developer.work.weixin.qq.com/document/>
## 2026-08-15 macOS real execution verification

- The first `xcodebuild` invocation compiled successfully but skipped the real test because the scheme did not forward `NOLA_RUN_REAL_MAC_EXTERNAL_E2E` into the XCTest host. This run is not counted as execution verification.
- The test was rerun through its explicit `/tmp/nola-run-real-mac-external-e2e` gate with the production `nola-agent-host` binary.
- `MacExternalExecutionRealEndToEndTests.testInstalledExternalExecutionUnitsRunAndResumeThroughMacHost` exercised Codex, Claude Code, and OpenCode through the macOS host. Every execution unit completed an initial request and a resumed request, with stable external session identity and no duplicated assistant token.
- Result: 1 test executed, 0 skipped, 0 failures, 38.931 seconds.
- XCTest evidence: `/tmp/NolaMacExternalExecutors-current/Logs/Test/Test-NolaMac-2026.08.15_2-25-44-+0800.xcresult`.

Production conversation records remain separate from this isolated E2E environment. At audit time, `~/Library/Application Support/Nola/Conversations/mac-conversations.json` and `~/Library/Application Support/Nola/Logs/nola.log` had not advanced beyond 2026-08-13, so they cannot be cited as evidence for the 2026-08-15 execution result.
