> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# UI 和交互设计

更新：2026-11-08。现行行为按下列契约维护；静态原型仍供视觉方向、布局实验和演示参考，不能覆盖已接受的产品变更或证明生产能力已实现。

## 按用途与平台阅读

[共享交互](shared/CONVERSATION_UI_DESIGN.md)、[视觉系统](shared/DESIGN_SYSTEM.md)、[宿主机流程](shared/HOST_CONNECTION_WORKFLOW.md)只维护一份。平台专项进入[平台目录](../platforms/README.md)；可操作设计进入[原型目录](prototype/README.md)；验收边界进入[质量目录](../quality/README.md)。本地任务/历史不随仓库分发。

## 当前阅读顺序

1. [对话与工具交互](shared/CONVERSATION_UI_DESIGN.md)：输入区、模型原生档位、提示词位置、主题、流式显示与移动手势。
2. [会话检查器](shared/CONVERSATION_INSPECTOR.md)、[Apple 适配](../platforms/apple/CONVERSATION_INSPECTOR.md)：右侧原地展开、多区段、草稿与观察生命周期。
3. [视觉系统](shared/DESIGN_SYSTEM.md)：语义主题、悬停/按压、复制动作、间距、减少动态效果及平台实现入口。
4. [统一对话与可保存 Agent](shared/MULTI_AGENT_NAVIGATION.md)：领域对象目标与执行归属；目标不等于当前 Room 代码已经删除。
5. [客户端交互与配置架构](../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)：状态所有者、精确参数调用链、各端差异与源码映射。
6. [视觉验收](shared/VISUAL_ACCEPTANCE_CONTRACT.md)、[跨端对齐门槛](shared/CROSS_PLATFORM_UI_PARITY.md)：原型、组件、隔离存储、整窗和真实模型证据分别记录。

手机当前以会话列表进入聊天，主导航为对话、活动、资料、设置；Today 不再是现行默认入口。桌面保留单顶栏、内容标签、独立左栏和可关闭右栏。检查器原地展开，预设提示词归入模型设置，聊天推理选择按模型实际档位显示，不并列固定三档。

[原型目录](prototype/README.md)、[演示交互](prototype/docs/UI-INTERACTION.md)、[视觉参数](prototype/docs/DESIGN-SPEC.md)与[能力矩阵](prototype/FEATURE-COVERAGE.csv)保留各自同步日期。未同步的脚本和历史截图应明确标为演示/待核对，不再称为生产交互的唯一基线。

## 全业务原型（2026-11-10）

新增[场景入口](prototype/scenarios.html)与[全业务清单](prototype/docs/BUSINESS-SCENARIOS.md)：68 个业务场景、220 个状态，按 macOS、iOS/iPadOS、Android、Web、Windows 和 CLI 视角审阅。当前 Web 14 路由全部映射，异常、拒绝、保存未确认、长任务恢复与未知格式不再只靠说明文字带过。原有完整交互原型保留，设置的原型预览提供入口。

这是浏览器中的设计状态演示，适用平台不表示实现或验收完成；原型不执行模型、Host、OAuth、系统授权或数据操作。逐状态回放与原生、真实模型验收分开记录。

## 跨平台契约

- [视觉系统与组件规范](shared/DESIGN_SYSTEM.md)：当前基线索引与平台映射；详细视觉值以原型视觉规范为准。
- [统一对话与可保存 Agent](shared/MULTI_AGENT_NAVIGATION.md)：对话身份、参与者、配置快照及逐目标执行的架构/行为契约；原型所示导航是演示，不能证明领域实现已完成。
- [会话右侧检查面板](shared/CONVERSATION_INSPECTOR.md)：检查器数据所有权和状态语义；桌面入口和展示分区以当前原型交互规范为准。

- [权限配置与恢复交互](shared/权限配置与恢复交互_20261016.md)：仅谨慎/全部允许，配置提交和当前生效分开，恢复入口按真实确定性显示。

- [会话交互契约](shared/CONVERSATION_UI_CONTRACT_20261015.md)
- [会话界面设计](shared/CONVERSATION_UI_DESIGN.md)
- [流式输出交互](shared/PROMPT_UI_STREAMING_SOP.md)
- [工具活动展示](shared/tool-activity-presentation.md)
- [跨平台一致性](shared/CROSS_PLATFORM_UI_PARITY.md)
- [执行展示与构建边界](shared/CROSS_PLATFORM_EXECUTION_PRESENTATION_AND_BUILD.md)
- [视觉验收](shared/VISUAL_ACCEPTANCE_CONTRACT.md)

## macOS 专项

以下按日期标记的内容是各次实施/审计记录，可能描述过时的组件值或界面结构。它们保留作证据和背景；发生呈现冲突时，以本文顶部链接的 2026-10-28 原型规范为准，不将旧记录当作当前设计要求。

### 实时思考与过程折叠（2026-10-19）

- 工具历史默认折叠不等于隐藏正在生成的思考：reasoning 在正文上方即时展示，工具边界不收起此前思考；最新正文 Part 开始产生公开内容后才自动收起。正文片段随 journal 增量更新，不等待模型或任务结束。
- 展开执行过程时只在完整时间线展示同一思考，不重复渲染；思考行自己的展开选择由对应消息的展示状态维护。终态不把遗留 running 元数据重新当作实时内容。
- 验收挂载实际 journal 组件，在任务未结束时连续投递两段 reasoning，断言默认折叠下正文已读取并增长，同时工具历史仍保持折叠。
- 来源与同源引用只展示一份，默认最多三条；额外条目用 `…` 展开，保留完整已提交记录和原始跳转信息。执行过程标题移除装饰图标，保留折叠箭头。
- 左侧处理中圆点可见直径放大四倍（5.76pt → 23.04pt），与处理中文案共同使用 28pt 高度对齐；只更新圆点透明度，减少动态效果设置下静止。
- 模型陷入长段循环思考时，公共模型流适配层应用既有重复块检测并明确报错，不将截断结果伪装成完成、不自动反复重采样；该防护不能替代对模型服务本身退化原因的排查。

### 模型连接工作区（2026-10-19）

- 默认展示可搜索的紧凑连接列表：图标、名称、地址、启用状态、模型数量。点击连接打开独立详情；添加 API、导入、ChatGPT 与主机管理统一收在添加菜单，分别打开编辑面板，不在主列表展开表单。
- 连接保留名称、地址、启用状态和账户归属；模型测试及高级能力覆盖按需展开。停用连接不删除凭据。
- 使用现有水墨主题的 `MacDesignColor.canvas`、系统前景色及分组表单，不新增色板。正文使用系统字体，辅助说明使用 callout/caption；不加入页面切换动画。
- 窄窗口保留相同搜索与添加入口，名称和地址单行截断并保留完整提示。授权、导入、测试各自保留明确的取消和费用提示。
- 参考 `参考资料/magpie/internal/gui/assets/app.js` 的 `renderProviders`：摘要行与独立 editor/import modal 分离；不照搬其网关功能或品牌色。Nola 使用现有青山品牌色、40pt 图标容器、14pt 行内间距、24pt 页边距、正文/辅助字体层级；仅悬停背景反馈，无列表入场动画。
- 常规与紧凑布局均为「搜索 / 刷新 / 添加 → 连接摘要行 → 点击打开详情」；不再使用上一版五分区菜单加大块表单作为主页。
- 详情中以模型为操作边界：启用开关与模型选项同组，选项内提供测试及能力覆盖；列表支持名称和模型 ID 搜索。没有目录时显示获取列表的下一步说明。保存模型选择为主要操作。
- 新增连接输入 HTTP(S) 地址，禁止内嵌凭据与片段；保存后直接定位新连接详情。此处仅做输入格式校验，不代表服务连通性已验证。
- 模型详情使用独立滚动列表，不再嵌套大块分组表单。未保存的模型选择是编辑器草稿；目录刷新和模型测试不得覆盖它，未编辑的选择则跟随服务更新。关闭时明确询问是否放弃，保存成功后更新基线。
- 整页标题区统一为「模型 / 默认模型菜单 / 更多」，窄窗口自动分两行；移除独立默认模型说明区及装饰性标题图标，错误仍常驻展示。来源切换位于标题下方，列表使用 30pt 图标和 10pt 行上下边距。
- 详情模型行将启用开关与选项按钮横向对齐，点选后才展开设置。保存区固定于详情底部，停用连接收进次级菜单。验收必须渲染当前 `MacModelCenterView`，不能用旧 `MacRemoteModelManagerView` 的截图代替；使用隔离存储注入，覆盖 520/1000pt 及明暗模式。
- 连接工作区与 ChatGPT 授权页面的静态文案统一进入 `ModelCenter` 资源表；本轮补齐简体中文和英文，其他语言仍需补齐，不宣称全语言完成。
- 依据 apple-design 的 macOS、布局、设置、侧边栏、无障碍、字体、颜色及文案规范；具体工作区划分是本项目的信息架构选择。
- 验收使用独立数据库与 Test 配置，覆盖 460pt 窗口下空态、已连接、加载及错误状态的明暗截图，以及中文资源打包断言。不调用真实模型、不触碰用户凭据。

- [信息架构](../platforms/macos/UI_INFORMATION_ARCHITECTURE.md)
- [应用设计](../platforms/macos/APP_DESIGN.md)

其他平台首先遵循跨平台契约，平台专项设计随任务补齐；当前不宣称已经拥有每个窗口的独立设计与真实验收。

## 数据边界

组件仅拥有展示状态。配置、消息、执行、权限、上下文与缓存统计的权威来自会话聚合及已提交投影。
切换会话加载对应数据和草稿；新窗口不能继承另一会话的执行中、错误或待授权状态。
实现差距和后续验收统一见当前任务（本地工作记录，不随仓库分发）。

## 2026-11-04 系统整改顺序（本轮）

当前审查基线 `5a77ef12`。先核源实现，再改原型和代码；不以构建或 fixture 冒充真实账户/动画验收。

| 优先级 | 用户问题 / 已确认事实 | 决策与验收 |
| --- | --- | --- |
| P1 | 模型空态只露 API 表单；Codex API 配置与 ChatGPT 账户路径混淆，账户保存后发现/启用链不直达 | 清晰区分 API、官方 ChatGPT 授权、配置文件导入。保留来源所有权；不把订阅 token 当 API key。核对目录→启用→冻结→调用，真实登录由用户完成 |
| P1 | 真实 Mac 原图及 AX 均缺少关闭/最小化/缩放；自绘顶栏覆盖 full-size 标题区 | 恢复系统标题栏及系统三按钮，不自绘假交通灯；实际最小化/恢复与缩放验证 |
| P2 | 右侧八个同权重图标拥挤 | 归为「任务」（概览/进度/目标）、「产物」（审查/产物）、「上下文」（上下文/执行/参与者）三个具名入口；保留当前面板、无“更多” |
| 已核/真实账户待验 | 简单/中等/复杂是否真影响 Responses | 追到冻结配置及 wire；展示实际 effort/自动能力，只显示公开推理摘要，不承诺私有思维链 |
| 已核/最终窗口待验 | 网络完成与 UI 呈现是否脱节，水面渐显太弱 | 分离接收/权威正文/短暂增量动效；记录首 delta、completed、最后可见时间；旧文不重播、完成快速收尾，减少动画保留 |
| P1 | 子代理目前继承父执行者，不能选择已安装 harness | 复用现有 Codex app-server、Claude stream-json、OpenCode adapter，Core 冻结 profile 执行者；Host 实际进程/取消/恢复沿原契约，不新建任务引擎 |

参考实现：Magpie `yetone/magpie` 是协议/账号网关，T3 Code `pingdotgg/t3code` 是 harness 控制面，二者不是可互换的账户协议。具体源码与官方许可依据记录在模型/委派模块文档。未完成项及真实验收限制必须随结果保留。

### 本轮已确认的实现缺口与边界

- ChatGPT 目录读取缺少推理 dispatch 已有的近过期刷新；目录刷新现在沿同账户锁/注册/subject，Core 回执采用刷新后 session revision，不换账户、不扩大授权。
- 账户目录中的明确 reasoning levels 曾被丢弃；只有来源实际声明时才初始化共享推理能力，未知不按模型名猜。Responses 层同时移除对 SIWC 请求公开 summary 的硬编码排除；effort 与公开摘要支持必须分开判断。
- 三档原本已贯通 Core tier→effort，不能写成“完全未实现”；界面现在显示解析后的实际 effort 或服务商默认值。模型是否真实产出摘要仍需实际账户/能力验证。
- 共享 Apple renderer 接收文本即更新权威正文，16ms 合帧，没有 completed 后长打字队列。水面效果属于绘制层短暂增量渐显，不能靠减慢网络或延迟正文制造。连续帧用于验动作，静态截图/编译不证明效果。
- 外部子代理已有 Host driver，缺 profile 执行者的独立冻结；本轮复用同一 Task/Run/权限/lease/取消链。Room 专家与 subagent 是不同实体，不把 Room 模型配置冒充 harness 选择。
- Nola 当前私有仓库和本地许可证不能自行证明官方开源本地账户共享计划资格。真实 ChatGPT 登录、订阅资格及新授权必须由正常官方流程确认；本轮未替用户登录或生成持久访问。

### 端到端补核（本轮第二阶段）

- 不把“已冻结执行者”当作已执行：以生产 child Host 和现有三个协议的隔离进程替身检查启动、事件、审批、取消物理停止与严格恢复；真实程序安装与登录/执行分开报告。
- 不把授权表单当作账户可用：拒绝/过期/网络错误必须沿真实 callback/service 状态进入可操作文案；拒绝不交换 token，不创建账户，不泄露上游错误描述。真实订阅资格仍待用户正常授权流程确认。
- 不把请求体和 renderer 各自通过当作同一链通过：Mac 请求组装、冻结 Run、实际 Responses wire、公开摘要 journal 和展示投影需使用同一个隔离公开 SSE 回放串起。
- 新确认的启动问题：内置 Skill 使用正文 digest 作 version，却将绝对 instructionsRef 一并纳入不可变版本；仅移动 App 包就可能触发 immutable conflict。修复应生成覆盖正文及来源引用的新版本，不松动 Core 不可变校验、不直接迁移/重写旧库。


### 独立链路复审与修复

- 模型目录能力清除覆盖和初次配置统一使用 Core `effective_capabilities`；重新应用托管目录时同步更新其推理档位声明，用户选中的档位仍由 parameters 所有。不能将目录支持与用户实际请求混为一谈。
- 普通 Mac API 新提交携带读取配置时的 profile revision，Host 拒绝提交前已变更的配置；恢复使用原 descriptor 的身份，并继续执行不可变 Host binding 精确校验，不拿当前目录版本重标旧 Run。凭据 gate 仍核路由/账户身份，不把目录更新误当旧 Run 凭据失效。
- Web 辅助请求与主执行共用 thinking 判定；effort `none` 不请求摘要，automatic 依据已有能力，摘要支持仍独立处理。
- 移动端 frozen generation 保留公开摘要能力的 true/false/null，HTTP 请求尊重明确 false，重发和恢复继续使用冻结值。iOS 显式 effort 已接同一 Core 解析器并传入 Responses/Chat wire；三档的物理映射仍只在 Core。Anthropic budget 等未实现协议保持拒绝。
- 外部执行者权限回调返回错误时，先检查已有 typed cancel，再沿物理停止确认路径处理；没有 ACK 时保留未知。Codex leader 退出后 stderr EOF 等待受同一退出期限约束，不因残留管道无限等或提前提交成功。
- 实际 SIGKILL 进程组曾被 OS 拒绝，后续未重试；此前协议进程 cleanup 断言失败仍保留，不能用编译代替最终取消通过。GUI 工具自动解锁错误也未重试，不推断真实屏幕锁定状态。


### iOS 三档推理接线

- `ResolveRemoteModelProfile` 对显式 request `reasoning_tier` 或 profile parameters `reasoningTier` 使用现有 Core `resolve_model_reasoning_tier`，而非只记标签；没有声明可调整档位的模型仍返回空 selection，客户端不提供可调控件、请求不编造 effort；不兼容的显式物理参数仍拒绝。未选择档位时保留服务商/模型原默认。
- iOS 模型参数页仅对声明 effort 能力的远程模型提供服务商默认/简单/中等/复杂；Core 解析结果随 profile revision 进入 prepared client 和冻结 generation，恢复/编辑重发保持原 selection/tier，而不重新按当前偏好推算。
- Responses 发 `reasoning.effort`，Chat Completions 发 `reasoning_effort`。summary 是独立可选能力，关闭展示不悄悄改掉已选 effort，effort none 不请求 summary。没有承诺上游必定返回公开摘要，更不展示私有思维链。
- 本次只运行共享 Core 定向回归与编译本地 wire/状态断言，不再尝试此前被拒绝的原生窗口或进程操作；真实手机/窗口交互与真实上游输出仍须单列验收。


### 真实 Responses 补验

- 现有 OpenAI 配置实际完成两次虚构 UI 样例：medium/high + summary:auto 均 HTTP 200、Core answered；上游 delta 和 terminal 均未含公开摘要。请求 effort 的传递链与上游是否提供公开摘要须分开表达，不以缺少摘要伪造思考过程。
- 同一真实高档请求的原生离屏 journal renderer 中，末尾标记由部分可见到完整；网络 completed 到最终稳定栅格约 525ms（50ms 采样与截图开销的上界）。这不是桌面窗口、滚动或系统主题的验收结果。
- Codex 订阅文件导入与 Nola 独立 ChatGPT 授权是不同入口。文件导入不能把订阅 token 当 API key；现有 Nola ChatGPT 授权有正常刷新路径，导入页不应笼统写“令牌刷新不支持”。


### Codex 来源导入的认证与路由准入

- 外部登录、Nola 自己的账户授权和 API 密钥是三种不同的来源所有权。Codex ChatGPT 登录用于其本地执行器；独立 Nola ChatGPT Responses 使用官方 SIWC client/host/用户授权，不复制 Codex token、不借用其刷新权限。
- 修复配置导入默认“不导入凭据、启用模型”组合：Codex 内置 OpenAI 来源声明需要认证；没有已批准 API 凭据时，预览保留配置但不允许确认成匿名可用连接。真正声明匿名的自定义服务仍可导入。已有同地址、同协议且 ready 的 API 账户可用于配置更新，不重新读取或复制源密钥；换路由不能复用该判断。
- 认证分类尊重来源 `auth_mode`。订阅模式不因残留 API-key 字段被转成 API；API-key 模式也不因残留或 null token 字段被误标订阅。没有启用凭据读取时，不擅自打开来源 auth 文件。
- 显式 `model_providers.openai` 优先于内置默认，保留其地址、协议及认证配置。导入、目录发现、启用配置与一次真实推理验证分别报告；连接 `enabled` 只展示“已启用”，不冒充网络“已连接”或推理已验证。
- 本机只读核查：Codex 自身识别既有 ChatGPT 登录；已检查的日常 Nola 模型库没有 `nola_chatgpt` 账户。完成独立授权需要用户经官方登录同意新的 Nola 授权；本轮不发起注册、登录或保存凭据。旧的错误导入不自动迁移、删除或借用其他账户，需用户明确重新配置。

依据：Magpie `internal/provider/chatgpt_api.go` 将独立 SIWC 和 Codex backend 分开；OpenAI 官方 SIWC 文档要求 client、用户/workspace 与 host 绑定。参考源码并不使跨 client 复制 token 合法，也不替代实际账号资格与授权确认。

应用更新入口、状态、下载与平台安装交接见[软件更新交互](shared/SOFTWARE_UPDATE_EXPERIENCE.md)。

2026-11-10 的目标输入、ChatGPT 账户登录、焦点与错误恢复变更见跨平台交互检查（本地工作记录，不随仓库分发）。


### 水面浮现与检查器可读性（2026-11-10）

- 新文字从最终基线下方 3pt/dp/px 轻轻上浮，0.65 的柔焦逐渐收清，透明度从 0.08 收敛到 1；波头沿新增内容前进。正常窗口 520ms，完成后最多 160ms 收尾。参数及边界见 [流式 SOP](shared/PROMPT_UI_STREAMING_SOP.md#9-渐进显现动画)。
- 已显示文字不重播，权威正文和布局立即更新。关闭流式状态不能直接卸载尚在收尾的绘制组件。Reduced Motion 立即清晰显示，切回正常模式不重播历史。
- Mac 检查器正文/详情/注释统一为 13/12/11pt 角色，避免大面积 9–10pt 文本；详情开合由父布局调整高度，内容轻淡入，不缩放字形。
- 验证必须覆盖实际 renderer，包含连续增量、大段突发、完成、替换、选区、窄屏与组合字符；回放不等于真实模型网络时序。

专家上下文在主会话下的树形归属、旧配置历史、只读阅读与返回父会话继续的边界见[专家子会话与历史阅读](shared/EXPERT_CONVERSATION_HISTORY.md)。
