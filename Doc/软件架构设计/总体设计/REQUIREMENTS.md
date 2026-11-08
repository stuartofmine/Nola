> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola 需求说明书（重写精简版）

版本：2026-08-24（更新：跨端消息所有权与投影验收）
状态：Active  
对应设计文档：Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md

## 1. 文档目的

本文档定义 Nola 当前阶段的产品需求基线，用于：

1. 对齐 iOS、Web、后端三端范围
2. 明确功能边界与验收标准
3. 为研发排期与测试提供统一输入

本版本聚焦“可上线、可联调、可验证”，不保留历史开发日志。

### 跨端执行与消息接入

1. 每个客户端必须允许用户在会话标题区域选择 Nola 或已安装的 Codex、OpenCode、Claude Code 作为执行单元；选择、外部会话 ID 和可用状态必须按会话持久化，切换会话后恢复。
2. 消息渠道必须支持 Telegram、WhatsApp Cloud API、微信公众号和企业微信的连接状态、配置入口、签名验证、消息去重、租约消费、失败恢复和回复发送；不得把个人微信非官方自动化描述为正式能力。
3. 执行器与消息渠道验收必须包含真实已安装执行器、真实 LLM 请求和至少两轮会话恢复；协议 fixture 只能证明协议，不能替代真实边界验证。
4. 对话标题必须是 Host 所有的规范化会话元数据，并持久化标题来源。默认会话的首个非空可见用户输入生成一次自动标题；附件展开内容、隐藏上下文和模型输出不得参与。用户显式重命名后，任何自动流程不得覆盖。
5. 客户端不得通过本地化文案判断标题状态，也不得在流式过程中猜测并写入标题；必须在规范化提交完成后从会话 API 刷新投影。验收应覆盖重启恢复、用户重命名保护、附件首问以及多语言首问。
6. 工具结果必须提供独立的人类摘要、有界技术预览、结构化结果、完整输出引用和 artifact；UI 主层不得显示序列化结果信封。工具调用及其中间状态必须可见，参数与技术详情可展开，文件和 URL artifact 可直接打开。
7. 工具结果验收必须使用真实 LLM 发起真实工具调用，并同时检查实时状态、历史重放、DOM 结构、截图和 Host 日志；断言应证明主摘要不是 JSON、技术详情可展开且完整结果未丢失。

## 2. 产品定位

Nola 是一个覆盖 iOS、macOS、Android、Web 与 CLI 的本地优先个人智能助理系统。
各客户端可以提供不同的平台工具，但共享 Agent 执行语义、权限边界、上下文规划和
Trace 呈现契约。

核心能力：

1. 用户身份与订阅分级
2. 对话与任务解析
3. 定时动作（Scheduled Actions）
4. 结果通知与历史可见

## 3. 产品范围（In Scope）

### 3.1 客户端范围

1. iOS 客户端
   - 主交互端
   - 本地数据与通知体验
   - 与后端能力联动

2. Web 客户端
   - 登录与会话态访问
   - 仪表板数据查看
   - 定时动作创建、查看、删除、立即执行
   - AI 对话（流式 SSE，支持多轮；可显式开启 Agent 模式处理复杂问题）
   - 执行记录查看（结果历史与详情）

### 3.2 后端范围

1. 统一 API 网关
2. 订阅与用量接口
3. 定时动作与推送接口
4. 调试与健康检查接口
5. 当前 Web/iOS 所需的官方模型、Agent 设置、移动桥接与订阅别名接口

## 4. 非范围（Out of Scope）

1. 缺少平台等价系统 API、硬件或用户授权时无法提供的专属能力
2. 完整企业级多租户后台
3. 无授权的第三方数据抓取
4. 未经确认的自动云同步

## 5. 用户角色

1. 普通用户
   - 使用 iOS 或 Web 查看状态、创建定时动作

2. 订阅用户
   - 使用需 LLM 的能力（AI 对话、定时任务中的 LLM 生成）

3. 开发/运维人员
   - 使用调试接口、日志与健康检查做联调排障

## 6. 核心业务需求

### 6.1 账户与登录

1. Web 必须提供登录页
2. 官方后端账号必须使用 Apple、Google 或 GitHub 登录并自动注册，不得使用自建邮箱密码鉴权
3. 登录后必须建立会话（session）
4. 受保护页面必须要求登录访问
5. 设置页必须提供本地密码重置能力（当前密码 + 新密码 + 二次确认）
6. 已链接后端的用户登录后应从本地保存的后端会话凭据恢复 access token（无需手工粘贴，也不得阻塞等待网络刷新）
7. Web 本机邮箱 + 密码仅作为本地缓存/离线账户登录方式，不上传到官方后端；后端链接页必须引导用户选择 Apple、Google 或 GitHub

验收标准：

1. 未登录访问受保护页面，重定向到登录页
2. 登录后可访问仪表板与定时任务页面
3. 退出后会话失效
4. `/v1/auth/register`、`/v1/auth/login` 和官方后端移动邮箱密码登录返回不再支持，不能创建或校验官方后端密码账号
5. `/v1/auth/apple`、`/v1/auth/google`、`/v1/auth/github` 校验第三方身份后自动创建/复用 `UserAccountRecord` 并签发 Nola access token
6. 本地 Web 使用 `127.0.0.1` / `localhost` 时，必须可通过官方后端 OAuth Relay 完成 Apple/Google/GitHub 登录，第三方 provider 的 redirect URI 不得指向本地地址

### 6.2 订阅分级与用量

1. 系统必须支持 tier 查询
2. 系统必须支持 usage 状态查询
3. Web 仪表板必须展示后端返回的 tier 与 usage 信息
4. Web 必须支持通过 Stripe Checkout 发起按月订阅支付
5. 支付成功后，Web 必须把 checkout `session_id` 交给官方后端做服务端校验
6. 官方后端校验成功后必须写入/更新 PurchaseRecord，并让 tier 变为 `subscription`
7. 订阅用户可使用 Google 检索增强（Serper），且每月检索次数上限为 100 次（超额应返回可解释错误）

验收标准：

1. 调用 tier 接口可返回 free/pro/subscription
2. usage 接口返回月度用量与配额
3. Web 页面显示与接口结果一致
4. Stripe 支付成功回跳后，在设置页可看到“已验证并同步”的结果提示
5. 后端 webhook 与 `/purchases/stripe/checkout/verify` 任一路径成功后，`/purchases/tier/{uuid}` 返回 `subscription`
6. 订阅过期时 `/purchases/tier/{uuid}` 的有效 `tier` 必须降级为 `free`，同时保留 `reported_tier` 供 Web 显示原订阅档位
7. 订阅用户触发第 101 次当月 `web_search` 时，Host 返回可解释的额度错误；LLM 网关不得隐式发起检索

### 6.3 定时动作（Scheduled Actions）

1. 用户可在 AI 对话中用自然语言创建 URL、搜索或宿主机脚本类型的周期自动化任务
2. URL 自动化任务需包含 name、url、frequency、time；脚本自动化需包含 name、script、frequency、time
3. 创建成功后动作可在列表中查询
4. 定时任务读写默认走本地存储（本地优先）
5. 用户可删除已创建的定时动作
6. 用户可对已创建动作触发立即执行
7. 定时任务管理接口对所有已登录用户开放，不再要求订阅身份
8. 后端链接后可作为可选增强（跨端同步、云端推送触发等），不可阻断本地功能
9. 仅 LLM 调用链路要求订阅（例如 AI 对话与定时任务执行中的 LLM 生成）
10. 本地提醒属于本地任务与通知，不应作为 URL 数据源或 URL 自动化任务展示/调度
11. Web/API/后端的定时任务只接受当前自动化数据模型，本地提醒只进入 `UserTaskService`
12. 脚本自动化仅在用户自己的 Web 宿主机本地保存与运行，不上传后端；LLM 生成脚本默认停用，需用户审查批准后才可执行
13. 脚本执行必须受本地开发者工具开关、允许目录、bash 前缀白名单、超时、资源限制和执行日志约束
14. 后端 URL/搜索类定时任务必须支持每任务可选模型（`llmModel`），未指定时回退后端默认模型
15. 后端 URL/搜索类定时任务必须支持每任务工作流提示（`workflowPrompt`），用于覆盖默认执行流程提示词
16. 后端 URL/搜索类定时任务必须支持每任务技能清单（`skills`），仅允许白名单技能并在执行中产出可观测诊断

验收标准：

1. 未链接后端时，本地仍可创建/查看/删除/立即执行任务
2. 已链接后端时，任务可额外同步到后端（失败不影响本地成功）
3. 创建后列表可读到该动作
4. 删除后列表不再包含该动作
5. 立即执行触发后结果出现在执行记录中
6. 脚本任务审批前不可启用；运行结果必须记录 stdout/stderr、退出码、运行目录与失败原因
7. 含 `llmModel`/`workflowPrompt`/`skills` 的任务写入后，可在列表读取回显并在执行诊断中看到生效值

### 6.4 结果与通知

1. 后端应保存执行结果与关联信息
2. 客户端应能按 resultId 获取结果
3. 系统应支持可见通知与静默通知模式
4. Web 客户端应提供执行记录列表页
5. 执行记录应支持展开查看 AI 摘要、失败原因、提取事件
6. 官方后端下发的通知必须持久化为可同步消息，iOS 与 Web 均可查看未读和已读历史
7. iOS 数据页通知入口有未读消息时必须显示红点；点击未读消息后应标为已读并清除红点

验收标准：

1. 结果查询接口可返回结构化结果
2. ack 接口可记录结果确认
3. 通知模式由配置控制且可观测
4. Web 执行记录页可正确展示状态、时间、来源信息与详情
5. GET /v1/notifications 可返回当前用户通知列表和 unreadCount
6. Web 消息页可展示未读/已读通知，并支持单条或全部标为已读

### 6.5 AI 对话

1. iOS、macOS、Android 与 Web 客户端都应提供 AI 对话页；CLI 提供语义一致的终端呈现
2. 对话必须支持真实流式输出。Provider 或外部执行器事件必须由 Host 归一化为 typed Core
   Part response 与独立 Diagnostic Trace，再经 Application transaction 形成
   `ConversationProjection`；UI 不得直接解释 Provider、stdout 或互不兼容的原始事件
3. 支持多轮对话历史（本地存储）
4. 支持模型选择（已链接后端时可从后端 /v1/models 获取，未链接时可使用本地回退）
5. AI 对话采用本地优先：无 access token 时也可对话；有 token 时可启用云端增强
6. 新建/默认标题的对话必须以用户输入的首个非空 query 作为标题；显式标题不得被首条消息或后续消息覆盖
7. Web 与 iOS 必须支持三种系统回复风格：严谨专业、日常聊天、俏皮活泼；用户可在新对话/当前聊天入口临时选择，也可在模型参数页面设置默认风格
8. iOS 对话输入栏必须提供“更多设置”入口，集中管理 AI 对话参数：模型采样参数、回复风格、Agent 最大迭代次数、自动 compact 触发阈值、长期记忆、用户偏好、session memory 与 Skill Library；不应把“最大迭代次数”作为单独主入口暴露在输入栏
9. **聊天页默认以 Agent 多轮工具调用模式工作**：
   - 在用户配置了"本地 LLM 端点 + Key"后，Agent 会以 OpenAI 兼容 `tool_calls` 协议
     连续多轮调用工具完成任务（搜索、抓取网页、打开渲染网页并截图、点击页面区域、填写输入框、增删定时任务、查看历史等）
   - Web 安全工具默认可用；本地开发者工具（读写文件 / grep / bash）需在
     「设置 → 智能助理 (Agent)」或聊天页中的 Agent 权限面板显式启用并配置允许目录 / 命令白名单
   - 用户也可以直接在对话中用自然语言说明是否允许本地开发者工具、允许哪些目录、
      以及允许哪些 bash 前缀；Agent 应通过本地权限设置工具落盘并同步刷新界面状态
    - Agent 复用同一份本地 LLM 配置（不单独配置 endpoint/key）
   - Web Agent 可使用主机端浏览器工具打开公开 http(s) 页面、读取渲染后的可见文本和可操作元素、保存截图，并在同一轮会话中继续按坐标/选择器点击、输入文字、按键和滚动页面；私网、localhost 与 file URL 必须被拒绝，页面重定向与子资源请求也必须复用同类安全拦截
10. iOS 聊天页默认启用任务型自然语言解析能力：
   - 任务型输入直接进入任务解析 / 确认流；复杂问题 Agent 模式可在更多设置中开启
   - 普通聊天输入仍按常规对话流处理
   - 当用户选择「Nola Web 主机」模型时，iOS 只作为输入与展示端：LLM endpoint/key/model、MCP、浏览器和本地开发者工具权限均使用 Web 宿主机配置，iOS 不覆盖这些宿主机设置
   - Web 主机执行浏览器/工具调用时，iOS 聊天页应展示结构化工具卡片，包含运行状态、工具名称、参数/结果摘要、最终 URL 与截图 artifact 链接/预览
11. Web 与 iOS 聊天页必须支持 Claude Code 风格的上下文管理：
   - 用户可查看当前会话的 context usage 估算（消息、session memory、补充上下文占用）
   - 用户可手动执行 compact，把较早消息折叠到 session memory，同时继续保留本地历史
    - 用户可直接查看 / 编辑 session memory，并随会话本地持久化
    - 用户可维护跨会话持久化的长期记忆与用户偏好，并自动进入后续普通聊天、自定义 LLM 和 Agent 对话上下文
    - Web 普通/自定义对话与 iOS 本机普通对话都应由主 LLM 判断是否需要沉淀长期信息/偏好：只有模型发起隐藏记忆工具请求时才持久化；Web Agent 通过显式长期记忆 / 用户偏好工具持久化。不得保存一次性任务、临时兴趣、密钥或敏感个人信息
    - 用户可维护跨会话持久化的 MCP 接口元数据、上传的 Skill 与上传的可读取数据源（文件/文件夹内容），并自动进入后续对话上下文
    - MCP 接口设置不仅进入上下文，还应在本地客户端实际按 MCP JSON-RPC 访问：Web Agent 可列出/调用 tools 与读取 resources；iOS 构建上下文时可拉取 live tools/resources 与部分资源内容
12. iOS 聊天页仍应支持会话级附加上下文：
    - 用户可为当前对话选择 prompt template
    - 用户可把本地可读取数据源挂接到当前对话，作为这轮会话的补充上下文；数据源可以是网站、搜索结果、API/订阅源、文件/文件夹、系统数据或其他 Nola 可读取来源
    - 对于 URL 类数据源，若本地历史为空，构建上下文时应支持直接从 URL 抓取可读预览作为上下文输入
    - 对于配置了 schedule 的数据源，构建上下文时应优先注入最近定时执行输出（含 run 记录与可用的结果文本）
    - iOS 本地文件/文件夹数据源必须通过系统文件选择器授权；读取、预览与上下文注入默认只发生在本机
    - Web 本地文件/文件夹定时数据源必须限制在 Agent 允许目录内；浏览器上传的文件/文件夹内容仅作为本地对话上下文资产保存，不代表可持续访问宿主机路径
    - 数据源可在对话中创建/配置，也可用于定时操作并通知结果；本地执行与本地记录仍为默认路径，后端只作为可选同步/推送增强
    - 上述附加上下文选择应随会话本地持久化，不因远端会话同步被覆盖

验收标准：

1. 凭据要求由当前选定连接/账户决定；没有可用模型或必需凭据时显示可操作配置错误，不伪造本地回答或隐式切换模型。
2. 选定模型需要账户或订阅时，由所属 Host 核对凭据、资格与 profile revision 后发起流式请求；不可用时保留草稿并报告真实原因。
3. Qwen3-thinking 类模型的 reasoning_content 内容正确处理（不显示为"无内容返回"）
4. 刷新页面后历史对话可恢复
5. 默认标题的新对话在保存首条非空 user 消息后，标题等于首个 query；空白消息不阻止后续首个非空 query 命名，第二条及后续非空消息不覆盖该标题
6. Web 与原生模型选择显示来源/连接及模型身份，选项来自当前 Host 启用的 Core profile/本地模型目录；切换须整体绑定端点、协议、能力与凭据引用，不只替换模型名，也不被另一连接的全局配置劫持。
7. Agent 对话中的等待、阶段、思考、工具、权限、验证与结果都必须投影到当前
   assistant 气泡内：用户提交后立即创建唯一 `message_id` 的 pending 气泡；工具以稳定
   `part_id` 形成稳定行，运行中即可查看脱敏参数、目标文件或 URL，完成后
   可展开查看脱敏且截断的真实结果或错误；不得用独立全局状态条替代，也不得因新事件
   重建气泡。正文使用稳定已提交 Markdown 块与一个可变尾块；完成态只允许在同一气泡内
   将相同 canonical text 提升为 document-mode Markdown，不得整体替换正文或重放渐显。
   Part 必须按 `part_id + cursor + revision` 幂等提交，gap 暂停投影并请求
   重放；刷新、断线或重开会话只从 canonical `ConversationProjection` 恢复。Diagnostic
   Trace 可以采样或删除，删除后不得改变任何可见内容。
7.1 自动压缩触发后，当前 assistant 气泡必须先显示进行中状态；成功后显示可持久恢复的
    checkpoint 卡片，包含归纳项数、摘要和查看原始上下文入口；失败时明确显示原因并说明
    原始上下文是否被替换。客户端不得将压缩作为完全不可见的内部优化。
7.2 保存、发现、刷新等可恢复操作不得默认使用阻断式 Alert。页面内反馈必须区分信息、成功、
    警告和失败并提供文字/图标；只有破坏性操作、丢弃未保存内容和必须立即作答的决定使用模态确认。
7.3 API 模型添加流程必须在提交前同时配置模型 ID、显示名、允许用途、敏感数据授权和推理参数；
    不得先插入空模型行，再要求保存后进入另一个窗口补充必要参数。
7.4 表单主操作不可只以 disabled 状态表达错误；必须就地说明缺少或无效的字段。校验只作用于
    当前启用且可见的选项，参数超限不得静默截断后保存。后台 Store 已记录的可恢复错误必须投影
    到当前页面，并在适用时提供重试入口。
8. 聊天页可直接切换本地开发者工具开关，并编辑允许目录 / bash 前缀白名单
9. 用户在对话中自然语言修改本地工具授权后，新的权限状态会持久化并反映到聊天页面板
10. Web、iOS、macOS 与 Android 的 Agent 执行都必须遵循“平台适配器 → Host protocol →
    Core/Application transaction → ConversationProjection → 平台 presenter”的单向链路。
    Diagnostic Trace 是旁路可观测性数据，不参与正文、工具卡或历史恢复。平台可根据能力选择
    普通对话或复杂工作流，但 UI 不得维护第二套运行状态，也不得把内部协议字段直接展示给用户
11. Web 与 iOS 聊天页可查看 context usage、手动 compact，并查看 / 编辑 session memory、长期记忆与用户偏好
12. iOS 聊天页可为当前会话保存 prompt template 与可读取数据源上下文，切换会话后可恢复
12.1 iOS 聊天页选中 URL 数据源后，若该 source 暂无历史记录，构建上下文时可直接抓取 URL 预览内容
12.2 iOS 聊天页选中定时数据源后，构建上下文时会注入最近定时执行输出（含 run/journal 文本）
13. Web 与 iOS 聊天页可本地配置 MCP 接口，并真实访问 MCP server；上传 Skill / 可读取数据源作为全局上下文资产，默认不上传后端
14. 浏览器操作统一使用 `browser_*` 语义工具，并由模型在结构化参数中选择 `target=managed|attached`：`managed` 使用 macOS App 内置 Playwright 与隔离 Chrome profile，适合可靠、可复现的自动化；`attached` 使用 Nola Chrome extension 操作用户当前标签页或已登录会话。两个目标共享 session、语义快照、稳定 element ref、截图 artifact、权限和工具卡契约，不暴露 Playwright/extension 私有工具族。默认先读取无障碍树和可见文本；只有模型配置显式声明视觉输入能力时，截图才作为结构化图片提供给模型。涉及账号、登录、密码、验证码、授权、支付或其他敏感信息流程必须弹出确认卡片，用户允许后才能继续
15. iOS 连接 Nola Web 主机后，Web Agent 请求不携带 iOS 侧 model/customEndpoint/customApiKey；浏览器操作等宿主机工具事件会在 iOS 聊天流中以工具卡片展示
16. iOS 本机 Agent 模式不得执行代码、shell、文件写入或浏览器自动化；只有连接 Nola Web 主机且宿主机工具权限允许时，才可通过宿主机执行相关工具
17. Web 聊天页可把本地文件和浏览器选择的文件夹内容保存为本地可读取上下文；这些内容默认不上传官方服务器
18. Web 定时任务可创建并立即执行 `format=file` / `format=folder` 的本地数据源任务，读取路径必须位于 Agent 允许目录，执行结果写入本地记录
19. Web Agent 对本机大范围扫描或高成本探索必须返回 `ask` 权限决策并要求用户确认；工具级 wildcard allow 不得绕过这类内容级风险判断
20. 任一客户端连接宿主机时都必须把 `permission_request` 渲染为当前 AI 回复气泡内的附加确认卡片：允许只对本次请求生效，拒绝时 Agent 不继续执行需要授权的工具
20.1 Host 发出的 `request_user_input` 必须绑定到当前 run/session，并在当前 assistant 气泡内逐一展示全部未决问题与选项；用户作答后恢复同一轮执行。若交互通道不可用，回复必须保留全部未决问题、明确标记 `needs_user`，不得只丢出第一个问题或自行替用户选择。
20.2 长期记忆召回属于内部上下文，不得伪装成外部“来源”。聊天投影应使用稳定的内部记忆标识、本地化标题与可操作入口；无法打开的项目不得显示跳转图标。
20.3 聊天输入状态以 conversation ID 为作用域。未发送文本、附件及其异步解析结果、Host 用户询问卡和权限卡不得在切换会话后泄漏到另一会话；旧输入控件延迟送达的回调必须携带原 conversation ID，不能写入当前新会话。发送后只清空实际提交会话的输入状态。
21. 各客户端普通聊天系统提示必须加载 Nola 基础人格 prompt：强调理解意图、必要时澄清、真实精准、高效有洞察，并包含图片理解安全边界（不得识别真实人物、不得猜测敏感属性、仅在用户请求时处理图片生成）。选择回复风格后，普通聊天、自定义 LLM、Agent 模式与宿主机桥接都必须在系统提示中只注入当前所选风格的语言风格 prompt，不得把三种风格同时交给模型自行选择；默认风格为“日常聊天”。所有流式正文、thinking 与工具 Part 必须通过同一 canonical projection 增量提交；语义无变化时不得发布新的可观察 UI 状态
22. iOS 会话设置明确区分当前会话的参数覆盖与全局模型默认编辑器；预设提示词位于模型区。保存经原 owner/revision 的配置端口提交，失败保留编辑草稿；切模型、会话或关闭面板不整包回写其他对象。
23. 官方后端承担其现行认证、账户、代理与同步接口，执行 Host 承担 Core 准入和 Agent 投影；实际路由以接口目录与 backend/worker/src/index.ts、cli/src/web_parts/routes.rs 为准，不为已删除路径保留兼容实现。
24. 执行 Host 的 Agent 观察以 canonical Core Part/Trace 和已提交投影为准，不由客户端自建状态机；外部模型代理继续适配其声明的 provider 协议。官方后端不因提供模型代理而获得用户宿主机 shell、文件或浏览器工具授权。
25. Web Agent 处理复杂多步骤任务时应能维护本轮 session-local todo 清单；该清单只服务当前任务推进，不写入长期记忆，不上传官方后端
26. Web 当前会话保存 Core 模型选择与覆盖；刷新、切会话或继续发送均恢复该会话选择，未覆盖字段按明确继承语义解析，不能被全局最近模型替换。
27. 连接、profile、默认模型与模型参数由当前 Host 的 Core 模型仓库拥有，会话覆盖由会话配置拥有；不再以 LocalUserAgentSettings 单例充当模型目录。浏览器 localStorage 仅保存允许的展示偏好或明确的设备草稿，不是模型配置、权限或业务事实的权威、迁移来源或回退来源。
28. iOS 当前会话选择的模型必须随 `Conversation` 本地持久化；切换会话、恢复最近会话或撤销删除恢复会话时，应使用该会话保存的模型，而不是全局最近模型覆盖
26. Web Agent 本地 bash 工具应按常见命令语义解释 exit code，避免把 `rg/grep` 无匹配、`diff` 有差异等信息性结果误判为工具失败，并继续拒绝 shell substitution 形态
27. Web Agent 对复杂、高影响或权限敏感任务应在执行前通过 `propose_agent_plan` 生成计划审批卡；未批准前不得继续执行后续工具调用，批准只对本次请求生效
29. macOS 紧凑窗口不得持续压缩到主要动作消失：单一顶栏保留导航/标签，模型与原生档位在输入区可发现，右侧工具原地展开或使用紧凑覆盖层；多工作区页面按可用宽度适配，短窗口侧栏可滚动。响应式验收必须检查真实渲染、焦点和连续操作，不能只检查视图可构造或 PNG 非空。
30. 本地 Agent Host 必须使用有限且可配置的外层模型迭代预算。Core 不得因为不同的有效读取达到固定次数，就撤销调查能力；只有规范化后的相同 typed tool batch 连续重复达到阈值，或出现可证明的失败/无进展状态时，才暂停相关调查。暂停必须通过当前 assistant 气泡说明重复的能力与次数，并让用户选择继续一次、改变方案或停止；用户作答后恢复同一轮状态。不得依赖语言关键词、工具名黑名单或仅具提示性的 checkpoint 防止循环。用户对上一条询问作出自然语言选择时，应复用本轮保留证据继续执行，除非已观察状态发生变化，不得从头重复同一调查。达到外层预算时必须进入无工具最终归纳，明确列出未完成或未验证工作，不得假装完成。
31. Shell 合同应接受安全的双引号转义正则，并按管道语义解释状态码：由后续有界消费者（例如 `head`）正常提前关闭造成的 SIGPIPE 可视为信息性结束，其他 SIGPIPE 仍是失败；不得为规避误报而放宽 command substitution 等既有安全边界。

### 6.5 健康检查与调试

1. 后端必须提供健康检查接口
2. 调试接口必须受 token 保护
3. 关键状态（feature gate）需可观测

验收标准：

1. healthz 返回服务状态与关键开关
2. 未授权调用调试接口被拒绝
3. 授权后可获取调试信息

## 7. 非功能需求

### 7.1 安全

1. Token 不得写入业务日志明文
2. 调试能力与生产能力隔离
3. 会话数据应受服务端签名保护
4. iOS 与自托管 Web 主机的 mobile bridge 在拿到 bridge access token 后，业务请求体与 SSE 事件必须使用该 token 派生 AES-256-GCM 密钥加密；未加密的 Agent 请求应被拒绝
5. 开发启动默认开启局域网 HTTP 监听以便 iOS 连接宿主机；Agent 业务 body/SSE 必须继续使用 AES-GCM 加密，Authorization header 仅适合可信局域网；需要本机-only 时可显式关闭 LAN 模式

### 7.2 可靠性

1. 后端关键链路必须有错误日志
2. 对外错误响应必须可解释
3. 本地联调应有可运行的最小配置

### 7.3 一致性

1. 相同 client_uuid 在 iOS/Web 展示一致
2. tier 与 usage 以服务端结果为准
3. 文档、接口、实现语义保持一致

### 7.4 可维护性

1. 需求文档只描述需求，不混入历史 patch
2. 设计文档只描述架构与实现边界
3. 变更记录统一进入 CHANGELOG

### 7.5 架构所有权与协议（REQ-ARCH）

1. `REQ-ARCH-001`：每类产品对象必须且只能有一个语义所有者。Domain 负责身份与不变量，Application 负责用例命令、查询和投影，Core 负责纯状态转换，Store 负责持久化记录，Host 负责执行外部效果，Presentation 仅负责临时交互状态。
2. `REQ-ARCH-002`：Application 是设备上的唯一产品写入入口。macOS、iOS、Android、CLI 和 Web 不得直接写 Store、调用 Core reducer 或自行折叠 Host 流为完整消息。
3. `REQ-ARCH-003`：任何公开 Application API、FFI 或客户端协议不得暴露 `Stored*`、SQL revision、checkpoint JSON、inbox/outbox 行或其他持久化结构。Application 必须在端口内部完成 Store 记录与产品对象的映射。
4. `REQ-ARCH-004`：跨进程通信必须使用有版本、可拒绝未知字段的类型化协议。每个请求必须包含稳定请求 ID；流事件必须包含稳定事件 ID与单调序列；重放必须幂等。
5. `REQ-ARCH-005`：身份值大小写敏感且不透明。客户端、Transport 与 Host 不得 lowercase、trim 后替换、从显示名推导或重新生成显式提交的 conversation/message/part/run/request ID。
6. `REQ-ARCH-006`：Session、Turn/Run、Effect Request、Permission Request、Automation Definition、Automation Run 与 Background Task 必须使用各自的类型化生命周期，禁止共用自由字符串状态。
7. `REQ-ARCH-007`：自动化协议必须以 `AutomationDefinition` 和 `AutomationRun` 为对象边界。定义负责调度配置；Run 负责 enqueue、claim、heartbeat、retry、finish。租约与状态转换由 Application 校验，客户端不得直接写状态或租约字段。
8. `REQ-ARCH-008`：非宿主客户端只能通过 Agent Host 协议执行外部效果。Host 不得创建或覆盖产品 Message、Part、Run、权限授予或任务记录。
9. `REQ-ARCH-009`：Application Service 必须按 canonical 数据库实例复用并保持单写。客户端、窗口、Host Turn 和短生命周期进程不得通过 parent PID 或连接关闭隐式终止服务。
10. `REQ-ARCH-010`：完整 transcript 必须由 Store 的 Message/Part ledger 保存；Compaction 只替换模型工作集。完成 Turn 后必须保留该 Turn 生成的有效 Compaction item，直到被新的工作集显式替换。
11. `REQ-ARCH-011`：客户端不得读取或写入 `*.checkpoint.json`、`*.context.json` 等文件恢复产品会话。活动 checkpoint 只能通过 Application 写入 canonical Store，并且不能作为完整消息事实源。
12. `REQ-ARCH-012`：数据源首次添加必须执行真实读取验证，保存类型化 ReadRecipe、内容格式与验证状态。一次性原始响应不得默认进入 checkpoint、消息或日志；只有显式 Snapshot/Blob 和最终提交的 Message Part 可以按其产品语义持久化。
13. `REQ-ARCH-013`：Shell、Python、Xcode 与 E2E 必须通过 `cargo metadata` 发现 Rust target directory，不得硬编码 workspace 下的 `target/debug` 或 `target/release` 路径。
14. `REQ-ARCH-014`：以重新发布的 1.0.0 为受支持基线；后续协议/数据变更须维护已发布版本兼容，提供协商、可恢复迁移与旧版回归夹具。此前开发格式不支持，未知命令必须在写入前失败。
15. `REQ-ARCH-015`：每个提取后的 Application port 必须具备 Rust 与原生绑定序列化 fixture、未知字段/旧命令拒绝测试、幂等重放、冲突/租约、冷启动恢复测试；参与 Agent 路由的端口还必须通过真实 LLM 多轮测试。
16. `REQ-ARCH-016`：可见交互验收必须同时提供真实运行日志和人工视觉证据。每张截图必须实际打开检查，每段录屏必须从头到尾观看；OCR、文件存在、退出码或单元测试不能替代视觉验收。
17. `REQ-ARCH-017`：branch、rebase 与 revert 必须从不可变 Message/AgentItem ledger 和显式 ownership 重建 canonical Part projection。禁止提交“transcript 非空但 projection 为空”的 checkpoint；冷启动后的 Part 身份、顺序、状态和归属必须与操作前一致。
18. `REQ-ARCH-018`：所有客户端的上下文占用必须来自同一 Core/Application 快照，展示百分比、已用 token 和模型总窗口。消息条数、字符数或客户端自行估算不得冒充 token 窗口占用。
19. `REQ-ARCH-019`：Product UI E2E 的 transcript 必须从 canonical Store 冷读生成，不得由测试把 prompt 与预期答案拼成证据。设备、模拟器、ADB、浏览器、Host 和网络探针的每次调用都必须有单次超时与总期限。
20. `REQ-ARCH-020`：一次用户意图形成的会话资料变更必须是一个类型化 Application 事务。标题、会话设置和同步 outbox 不得由客户端拆成多个提交；成功时只推进一次 revision，失败时不得留下部分更新。Web DTO 或任意 JSON 文档不得作为领域写命令。

对应验收标准：

1. 两个先后退出的短生命周期 Host/CLI 进程可以访问同一数据库，第二个进程不会遇到 Application Service 被第一个进程带走的问题。
2. 真实 LLM 会话触发压缩后，下一轮能召回压缩前事实；数据库完整消息数与 Part 顺序不因压缩改变。
3. 删除所有客户端 checkpoint 文件路径后，macOS、CLI、Web 与 Agent Host 仍能从 Application projection 恢复。
4. 数据源启用时可回答验证值，停用后不可访问；usage audit 有记录，checkpoint 不包含未选择的原始注入载荷。
5. 使用自定义 `CARGO_TARGET_DIR` 执行构建或 E2E 时，测试仍定位并运行本次构建产物。
6. branch/rebase/revert 后关闭进程并重新打开 Store，历史 user、assistant、reasoning、tool 与 compaction Part 仍按原 ownership 和顺序存在。
7. iOS、Android、macOS、Web 与 CLI 对同一会话展示相同的已用 token、总窗口和百分比；切换会话后立即读取该会话快照。
8. Product UI 测试的截图、accessibility tree、timeline、真实 provider trace 与 canonical 冷读 transcript 指向同一个 Turn，并且终态正文一致。
9. 失效模拟器或无响应 ADB 不会让测试无限挂起；测试在期限内恢复设备或以明确的基础设施错误失败。
10. 同时修改会话标题和设置后，冷重开 Store 仍得到同一组值，revision 只增加一次且同步队列不存在中间状态。

## 8. 接口需求（对齐当前实现）

### 8.1 后端核心接口

1. GET /healthz
2. GET /purchases/tier/{uuid}
3. GET /purchases/usage/status/{client_uuid}
4. POST /purchases/usage/consume
5. POST /v1/sources/sync
6. GET /v1/debug/push/client/{client_uuid}
7. POST /v1/results/{result_id}/ack
8. GET /v1/models
9. GET/POST /v1/agent/settings
10. GET/POST/DELETE /v1/agent/permission_rules
11. GET /v1/agent/tools
12. POST /v1/chat/stream
13. POST /v1/chat/custom/stream
14. POST /v1/chat/agent/stream
15. POST /v1/chat/agent/v2/stream
16. GET /v1/subscribe/products
17. POST /v1/subscribe/checkout
18. POST /v1/subscribe/verify
19. POST /api/mobile/login
20. GET /api/mobile/me
21. GET /api/mobile/agent/settings
22. POST /api/mobile/attachments/parse
23. POST /api/mobile/chat/agent/v2/stream

### 8.2 Web 代理接口

1. GET /api/health
2. GET /api/tasks
3. GET /api/scheduled-actions
4. POST /api/scheduled-actions（支持网页/搜索，以及 Web 宿主机允许目录内的 `format=file` / `format=folder` 本地数据源）
5. DELETE /api/scheduled-actions/\<source_id\>
6. POST /api/scheduled-actions/\<source_id\>/run-now
7. GET /api/results
8. GET /api/results/\<result_id\>
9. POST /api/chat/agent/v2/stream（唯一持久化聊天入口，SSE 流式）
10. GET /api/models
11. GET/POST /api/agent/settings
12. GET/POST/DELETE /api/agent/permission_rules
13. GET /api/agent/tools
14. POST /api/mobile/login
15. GET /api/mobile/me
16. GET /api/mobile/agent/settings
17. POST /api/mobile/attachments/parse
18. POST /api/mobile/chat/agent/v2/stream

## 9. 本地联调需求

1. 必须支持 SQLite 本地运行
2. Web 与 backend 可独立启动
3. 必须可通过命令行完成端到端验证：
   - 登录
   - 读取任务信息
   - 创建定时动作
   - 回查动作列表

## 10. 验收清单（当前版本）

### 10.1 功能验收

1. Web 登录、退出、受保护页面行为正确
2. 仪表板显示真实后端 tier/usage
3. 定时动作写入链路在有效 token 下成功
4. 动作创建后查询可见
5. 定时动作可删除、可立即执行
6. AI 对话页可流式输出，支持多轮会话
7. 执行记录页可查看历史结果与详情

### 10.2 技术验收

1. 后端健康检查稳定返回 200
2. 关键接口 4xx/5xx 错误可定位
3. 文档结构保持"需求/设计/变更"分离
4. 测试用户在本地库有 subscription 购买记录，AI 对话链路可通

## 11. 版本边界与后续

### 11.1 本版本完成定义

1. 三端联调主流程可用
2. 登录与写链路可验证
3. 文档体系完成一轮治理

### 11.2 下一版本建议

1. 完善密码安全策略（复杂度、失败次数限制、可选二次验证）
2. 增加前端可视化诊断面板
3. 增加 Web 端到端自动化测试
4. 增加订阅生命周期回归测试

---

维护约定：

1. 需求变更先改本文件，再改设计文档
2. 任何一次性修复记录写入 CHANGELOG
3. 文档提交必须标注版本日期

## 12. Run 配置与流式性能要求（REQ-RUN）

1. `REQ-RUN-001`：平台必须在每个新 Assistant Run 或 Maintenance Run 开始时显式提交不可变 `InvocationConfigSnapshot`。Core 不得从上一轮、conversation settings、checkpoint、环境变量或平台配置文件推断当前模型与会话策略。
2. `REQ-RUN-002`：本地拥有的 Run 配置快照必须归属于 Run，并保存脱敏内容与稳定 digest；远端镜像 Run 只保存宿主机权威投影，不得伪造本地 provider 配置。API Key、OAuth token 和原始凭据不得进入 Store、checkpoint、事件或日志，只能使用凭据引用由 Host 解析。
3. `REQ-RUN-003`：同一 Run 的 Host 事件必须携带相同 `config_digest`。缺失或不匹配时必须在推进 sequence、消费 effect、写 Part 或 checkpoint 前原子拒绝。
4. `REQ-RUN-004`：新 Run 必须重新接收平台配置；修改平台设置不得修改活动 Run。恢复活动 Run 时只能使用该 Run 已提交的配置快照，不能使用当前平台默认值。
5. `REQ-RUN-005`：健康运行路径必须复用常驻 `ActiveRunActor`。不得为每个 provider delta 重开数据库、反序列化完整 checkpoint 或创建新的 Core runtime。
6. `REQ-RUN-006`：流式文本必须按语义边界、工具/状态变化、字节阈值、50-100ms 延迟上限或终止事件组成 `PartDeltaBatch`；不得按 token 形成独立数据库事务。
7. `REQ-RUN-007`：每个批次必须在一个事务中提交 Part、event cursor、checkpoint、effect acknowledgement 和 successor outbox，失败不得产生可见部分状态。
8. `REQ-RUN-008`：大数据必须通过带 digest、大小、类型、来源、时间和授权范围的 `DataReference` 传递，并以有界 chunk/search Host effect 读取；原始大载荷不得写入 checkpoint、配置快照或普通 trace。
9. `REQ-RUN-009`：平台可持久化自己的用户配置，但传给 Core 的只能是当前调用的类型化快照。Core 负责使用快照处理数据和 LLM 协议，不负责决定或更新平台默认 provider、模型和参数。

## Canonical Host resource requirements (2026-08-24)

1. Every artifact returned by any Host path MUST contain a valid canonical absolute URI and a non-empty display name.
2. Local files MUST use normalized, percent-encoded `file://` URIs. Raw paths and unencoded local file URIs MUST be rejected before any Core state mutation.
3. Rejection MUST be atomic: the response sequence and pending effect remain available so the Host can submit one corrected response using the same effect identity.
4. Unicode filenames MUST round-trip from native path to Host event, canonical Part/Trace storage, restored projection, clickable UI, and open action without lossy reconstruction.
5. Direct, delegated, task, replay, and restore paths MUST apply the same resource contract. Clients MUST NOT add path compatibility branches.
6. Aggregated UI rows MUST use user-facing capability titles and factual state. Internal counters such as model-turn/tool-call tuples MUST NOT appear as unexplained detail text.
7. Acceptance evidence requires all of: protocol contract tests, Core atomicity tests, a real-model Host session, persisted canonical metrics, an event containing the canonical URI, and a screenshot showing the resulting resource in the product UI.
8. A narrow passing test MUST NOT be reported as full Core health when the enclosing contract suite fails; unresolved failures remain explicit release blockers.
