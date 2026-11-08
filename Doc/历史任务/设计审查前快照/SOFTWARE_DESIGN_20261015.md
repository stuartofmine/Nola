> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola 软件设计文档（重写版）

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

## 请求密度与缓存指标（2026-10-09）

简单/中等/复杂是同一个显式 `reasoning_tier`（`concise/balanced/detailed`），
不能由 provider 的 effort、模型名字或用户文本反推。它同时决定推理参数、Core 固定工作流
说明密度和首轮直接暴露的工具集；请求与 Host generation binding 保存该档位，恢复必须精确匹配。
会话选择覆盖默认值，子任务继承冻结的运行配置。尚无档位选择器的移动端明确使用 balanced。

| 档位 | Core 工作流说明 | 首轮内置工具基准 |
| --- | --- | --- |
| 简单 | 人工编写的紧凑等价说明 | 6：发现、追问、Skill、读文件、工作区访问与状态 |
| 中等 | 压缩最长的检索、连续对话、任务完成与产物工作流说明 | 15：现有常用基础集 |
| 复杂 | 完整工作流说明 | 24：基础集加常用文件、编辑和网页工具 |

数量是可用性和既有授权/子任务工具范围过滤前的基准，不保证每个 Host 暴露相同数量。
档位只改变可见性，不增加权限，也不删除完整可发现目录；延后工具由模型通过结构化发现激活。
这是固定产品元数据，不是根据语言或任务关键词路由。用户输入、历史、用户/仓库约束、安全、
权限协议不按字符截断。档位控制的是可控固定开销，不承诺任意长输入的总 prompt 都低于固定值。
同一档位的系统前缀保持确定性；切换档位改变前缀/工具摘要，可能使下一请求失去旧前缀缓存。

工具说明在 schema 源人工精简，保留参数、边界、副作用和结果契约。模型投影只移除
`function.description` 与参数根说明完全相同的重复文本；字段说明、不同的根说明和发现目录原文保留。

侧边栏缓存事实与上下文占用是独立指标：缓存字段变化必须发布，即使占用率仍在同一合并区间。
未上报不等于 0%，服务端真实 read/write 优先于本地缓存控制开关和前缀诊断；
关闭显式 provider 缓存参数不能显示为服务端缓存不可用。已有日志中第二轮 15,450/15,485
命中不能被上一轮的 0% 覆盖。

验证记录：Core 427 项、Host 协议 27 项、Mac 集中回归 16 项、iOS generation snapshot
4 项和 Android generation snapshot 3 项通过；CLI 请求密度、档位传输、子任务隔离、动态工具发现专项以及
runtime 33 项通过。固定策略测试的三档 UTF-8 长度为 12,713 / 18,066 / 22,789 字节
（含同一约束 fixture，并非完整请求 token 数）；初始 15 个工具顶层说明由 4,536
减至 2,823 字符，字段 schema 不变。未据此宣称真实首 token 延迟已改善：真实端点
复测尚受钥匙串读取超时阻塞。CLI 全量尚未通过，另有生命周期事件、MCP 任务记录、
journal 通知回放断言失败；权限重复恢复测试在默认测试线程栈溢出，16 MiB 栈下通过，
不能以增大栈的诊断结果宣称默认配置已修好。

22:20 实机会话发现的跨构建续聊回归：已完成的旧 checkpoint 不含新必填 `reasoning_tier`，
新提交仍完整反序列化旧 Host binding，导致准入前失败。整改采用两条明确加载路径：

- 活动 Run 保持严格完整反序列化及精确身份验证，不能补默认参数、替换权限或跳过待执行效果。
- 新 Turn 的历史加载将旧 Host binding 作为不参与执行的历史元数据排除，但保留正文、工具调用与结果、
  游标、Goal/Task 等状态；解析后必须验证身份、终态/空闲、无排队输入、无未收尾效果。
  CLI 和 Application 的新 Turn 准入使用同一 Core 规则，原始存储与历史 invocation 不被改写。

Mac 提交契约：stdin 写入只表示 dispatching；事务成功后的 `submission_admitted` 或经过权威验证的
Run 事件才表示 accepted。确定的准入前拒绝发送 `submission_rejected`，会话/Run/assistant 三元组
必须匹配；准备阶段拒绝还须查证没有同身份的规范 Run，事务失去回执的情形不得伪装为拒绝。
拒绝清掉临时消息并保留原草稿，不覆盖后来编辑；EOF、回执未知或接受后的异常继续保持待核实，
不可自动重复提交。拒绝面板与执行恢复面板独立，前者不提供无效的“检查执行状态”。

Mac transcript 与底部输入/错误区域使用实际纵向布局和不透明语义背景，不以透明 overlay 加末尾
补白来预留位置。工具行及思考分组都通过统一显示标题策略本地化，协议 capability ID、日志和权限
匹配不随翻译改变。`write_file` 的操作固定为写入/覆盖（`dry_run` 只预览），不接受 `operation_kind`；
必须明确此参数契约，不得丢弃 `create` 后悄悄执行覆盖。

本次续聊与提交整改验证：Core 库测试 432 项、Application 隔离数据库恢复测试 6 项、
CLI runtime 33 项及写文件参数/提交回执专项 2 项通过。恢复测试覆盖旧终态配置缺字段、
不可识别的旧 Host binding、历史及 Goal 完整保留，以及活动任务拒绝替换配置且数据库不变。
Mac 提交票据、终态恢复、进程会话、工具标题和交互布局 5 组共 44 项通过；窄窗口
AX 几何断言验证 transcript 与底部区域不重叠，权限区域限制高度且可滚动。
以上不替代真实 provider 首 token 复测，也不代表前述 CLI 全量失败已修复。

22:48 实机续测补充：桌面在 BeginAssistantTurn 之前还会调用 idle configuration submission。
该入口此前仍严格解析旧 Host binding，且 idle command 的错误被过早转成字符串，导致明确拒绝
保留了“需恢复”的 fence。现将 idle controls / revert / maintenance 新执行统一接入已收尾历史
解码；活动 Run 和 replay 尾保持严格恢复。idle command 保留类型化的事务结果，经同一收据
处理更新 fence。测试必须覆盖配置准备→准入，而不能只调用 BeginAssistantTurn。
错误面板使用简短状态和默认折叠的“诊断详情”，不把完整进程 stderr 当主文案。
22:58 凭据拒绝的独立根因：无签名 XCTest 写入 Products/Debug，覆盖日常签名应用，
securityd 报 cdhash mismatch / rc=-67034 并拒绝原 Keychain ACL。Mac scheme TestAction
与测试入口现固定使用已有 Test configuration（Products/Test、tests-host bundle ID）；
正常 Debug 保留开发证书。不得通过重置密钥、放宽钥匙串 ACL 或关闭系统签名校验规避。
签名恢复后，Test 配置的凭据门禁、提交票据、恢复检查及输入框恢复 19 项通过；
测试前后正常 Debug 应用 CDHash 完全一致，codesign 严格校验通过。运行中的旧应用
仍须退出重开；上述结果不代替重启后的真实 Keychain 读取验收。
本次追加验证：Application 恢复及 idle checkpoint 8 项、CLI runtime/receipt/Host 恢复
49 个不同用例、Mac 上述 5 组 44 项、构建缓存脚本 41 项通过。既有默认栈溢出的
重复权限恢复用例未纳入本次 15 项 Host 恢复子集；不宣称 CLI 全量验收通过。

## 跨端界面交互契约（2026-10-09 窗口审查整改）

界面沿用各平台的语义色、系统字体和导航组件，不增加新的平行主题。
整改重点是让本地工作台中的保存、退出、错误与授权可预测：

- 编辑器持有独立草稿，保存成功后才能关闭；失败在当前编辑器显示并保留输入。
  保存中阻止重复提交；可取消的网络验证/推理保留取消入口，取消后的迟到结果不得覆盖新状态。
- 明确区分即时设置与需提交的编辑文档；取消不能既暗中提交又丢弃最后一段输入。
  多个底层存储 API 的顺序提交不等于原子事务，失败必须如实显示，不宣称整体回滚。
- 页面内容与呈现容器分工：Tab 根、push 子页、sheet 各自管理返回或关闭。
  Android 系统返回与可见返回按钮必须指向同一层级。
  Mac 扩展编辑器集中裁定退出，路由只转发目的操作；取消不得触发之后的迟到跳转。
  目的操作包含切页及关联模型变更（新建/选择会话、选择模型、输入焦点），不能只拦截
  页签赋值而提前修改业务状态。确认执行前重新检查目标仍存在；命令面板先关闭再请求退出。
- Mac 删除会话需用户确认，只删除会话记录，保留会话工作空间中的生成文件。
  外部选定目录也不随会话删除。重复删除被抑制，持久化失败显示错误而非假装成功。
  删除待回执期间保留草稿并阻止同会话新提交，失败后恢复发送。
- Mac 文件选择和日志导出改为异步系统面板，优先附着发起窗口；工具申请目录权限仍使用
  既有内置确认，不把工作区选择面板重新引入权限申请流程。
- 工具页面不依据执行位置猜测“已授权”。没有权威能力投影时显示“使用时检查”；
  浏览器连接只说明连接状态，不代表读写或控制权限。实际调用仍由 Core 检查。
- 重要操作反馈保留到用户关闭或下一操作，不用短定时器清除；独立控件有语义名称，
  正文支持字体变化，窄窗口和长内容有滚动或纵向后备布局。
  iOS 纠正反馈仅在 Core 明确成功后关闭；Android 待授权入口仅定位真实请求，
  不因进入请求位置而授权或恢复自动滚动，点击执行前检查请求身份仍有效。
- iOS 历史详情区分可读正文、结构化字段与技术诊断；原文通过公开的
  `read_conversation_export_range` 按记录身份、版本和字节偏移分页读取，不从显示文本推导文件权限。
  原文只读，不解释为工具指令；校验响应身份、版本、范围和结束标志，失败保留当前页，
  取消或切换记录后的迟到响应不得覆盖页面。页内选择/复制不冒充完整记录导出。
- Android 定时表单使用原生日期/时间选择器和可搜索时区，选择器确认只更新草稿。
  更换时区保持显示的当地日期与钟表时间，并明确告知不是保持实际时刻；一次性任务遇到
  夏令时不存在时间拒绝提交，重复时间须明确选择偏移。每日/每周计划保留独立的当地时间语义，
  不套用今天的夏令时判断，也不扩展调度核心未支持的频率。

逐项依据、实施状态和未验收项见 [三端审查总表](../实施与审计/UI_AUDIT_2026-10-09.md)。
代码修复不等于所有窗口已通过视觉、VoiceOver/TalkBack 或设备验收。

### Mac 嵌入运行时的安装边界

Rust 产物到 `NolaRuntime`、再到 App 的嵌入可执行文件共用
`scripts/lib/install_mac_executable.sh`：先复制到同目录的专用 `.nola-staging` 文件，
比对内容、遵循原有构建签名设置并验证签名，最后通过 rename 替换目标。
不得以 `cp` 原地覆写旧可执行文件；否则内核缓存的代码签名可能与新内容失配，
即使磁盘上的签名检查成功，启动仍可能被终止。

复制或校验失败保留旧目标；退出清理本次 staging 文件。Xcode 单写者构建阶段串行
拥有对应临时文件，逐文件声明输出权限，不扩大到整个 App 目录。临时输出清除后，
后续构建会重新检查安装阶段，Cargo 仍复用增量产物。此为单个文件的原子安装，不宣称
多个 helper 的替换构成跨文件原子事务。故障注入回归见
`scripts/test_install_mac_executable.sh`。

## 聊天控件与授权层级（三端，2026-10-09）

本轮按 apple-design 的可访问性、布局和隐私审查落实局部改进，不改变 Core
权限判定或工具调用参数。Mac 思考展开和附件删除使用至少 28pt 点击区域；
iOS 思考标题、授权按钮使用至少 44pt，附件保留独立可访问的删除操作；
Android 思考标题、附件删除及授权按钮使用至少 48dp，并保持 Material 外观。

普通工具调用优先展示单次授权，会话授权保持独立次要操作；Mac 文件夹请求
保留实际支持的会话授权，不伪造单次语义。Mac/iOS 原始参数默认折叠，原因、
目标/匹配范围等已有安全信息仍可读；展开后不改变调用。Android 授权按钮纵向
排列、允许标签换行，以支持窄屏和大字体，期限文案区分单次与会话选择。
Mac 权限内容使用不透明语义背景，减少动态效果时发送按钮不缩放。

Mac 工作区和输入框宽度规则保持不变，Markdown 段落另设 820pt 阅读上限；
代码和表格不受段落上限约束。这是产品阅读排版选择，不是 HIG 强制尺寸。
本轮未宣称所有客户端、所有历史/原文阅读器已采用同一排版实现。

验证：修改的 Swift 文件通过 frontend parse，工程 plist 校验通过；新增 Mac
正文限宽契约测试及 Android 窄屏/大字体授权回调测试。完整测试尚未通过验收：
本次 Xcode 在工程读取的 getattrlist 阻塞，Android KSP 在 clonefile 阻塞，
已停止本轮构建；无连接的 Android 设备，因此不宣称截图或仪器测试通过。

## 思考与正文的发布顺序（2026-10-09）

Run 准入可以创建规范消息身份，但不能把尚无正文的文本槽发布成占据 transcript
顺序的可见 Part。准入通知使用该持久化 owner 的有效 genesis 游标；真正的正文、
思考和工具活动在首次实际发布时取得顺序。底层保留实际事件交错；Mac 显示层则
将 typed reasoning 固定放在正文上方，各组内部维持原顺序，晚到和历史恢复同样
遵循此规则。未曾发布的空文本槽关闭不制造可见正文；既有不可变 journal
不作原地改写。

Mac 根据 typed reasoning 元数据使用本地化标题：运行中为“思考中”，完成后为
“思考过程”，不直接显示协议的 reasoning 标题。正文预览区分“正在读取”和
“已读取但被截断”：仅后者显示完整阅读入口，避免初次读取前短暂出现入口再消失。

思考区使用独立的语义背景、边框和内容缩进，与正文区分。整个标题行是可通过
鼠标和键盘操作的展开按钮，而不是仅箭头可点。运行中默认展开并读取增量，完成
后保留当前展开状态以免高度骤缩；历史恢复默认收起。用户明确选择的展开状态
优先，不被后续 token 或终态覆盖。
空内容显示本地化的等待/未返回详情状态。首 token 验收分开统计提交准备、provider
首事件等待和已提交内容到界面的延迟，不能把隐藏思考造成的等待归因于模型，
也不能把显示层改进宣称为 provider 首 token 加速。
Host 对 typed reasoning 的首个有效片段立即提交，后续仍按语义片段和有界等待
合并；正文/工具前缀的校验与缓冲不变，不能用首 token 优化绕过该保护。

Journal 正文的有界读取结果通过增量语义投影接入已有浮现动画，流式、完成态保持
同一组节点和渲染树；历史初次加载不重播动画。动效只影响绘制，不增加持久化或
人为延迟，不重排已显示文字，并遵循系统减少动态效果设置。聊天记录保留已有
条数/字节窗口，历史行保持惰性布局，最后一条消息与底部留白独立准确测量，且
不随流式/完成态切换布局身份，兼顾长会话内存与尾部定位。内容几何、终态和
底部组件尺寸变化通知同一个滚动控制器校准，仅跟随模式回到底部，不抢走用户阅读位置。
桌面聊天列宽度上限为 1200pt，输入框与聊天列同宽；Markdown 段落独立限宽，
窄窗口继续按可用宽度收缩。

## 模型中心与按需授权（2026-10-09）

模型中心以搜索列表和单项详情组织本地/API 模型；窄窗口采用列表进入详情。
连接设置、模型基本信息、生成参数、能力声明和访问用途分开呈现。服务目录发现
只证明目录访问成功，不证明推理、工具调用或思考参数可用；Responses 诊断不得
自动更改用户选择的协议。目录和错误均绑定发起请求时的连接地址、密钥与协议。

API 编辑以连接为草稿边界。切换到其他连接须处理未保存修改；保存时核对该连接
原模型成员和版本，再合并最新的其他连接，通过规范目录 CAS 提交。参数编辑附着
于所选模型，修改连接未保存时不能打开已持久化参数编辑器。

保存按钮和持久化准入均只校验当前连接。未编辑连接可能有待处理的配置问题，
必须按原始规范记录保留，不能重新校验、重编码或补齐其能力声明而阻塞当前保存。
新模型默认开放对话、检索和问题修复用途，并启用工具调用请求；填写模型 ID 即可
添加，高级参数为可选设置，不强制逐项确认。启用工具请求不等于验证服务能力，
更不等于授予工具执行权限；不支持工具的模型可以显式关闭，敏感操作仍走 Core 授权。
全量目录保存也须保留语义未变的连接 sourceJSON 原文，避免 JSON 键顺序变化制造
虚假版本冲突。此次补充回归覆盖其他连接缺少能力声明时新增/修改 WSL 连接、
原记录不变、真实冲突拒绝、新模型默认用途与工具请求，以及添加面板的直接提交。

模型能力声明与执行授权不是同一集合。“敏感数据”布尔标记不能代表按资源授权，
不再作为模型编辑器中的授权开关。推荐权限模式对私人数据、记忆、设备控制、
浏览器自动化和命令执行默认 Ask，使用现有持久化权限请求及 AllowOnce /
AllowConversation / Deny 流程；用户明确规则和已授权的具体范围按 Core 策略执行。
不能依靠 shell 命令关键词把进程执行判定成安全。记忆写入授权须绑定具体内容，
不能仅凭相同 scope 放行不同写入。

边界：本轮不新增“自动识别全部敏感正文并按接收方申请外发授权”的能力。现有
planner 没有完整的类型化敏感数据流向契约；目录访问、文件读取授权和模型能力
声明均不得被描述为该外发授权的证明。后续实现必须在模型发送边界连接明确的
数据来源、接收方和批准范围，不能用提示词、模型名称或 UI 开关代替。

本轮验收：Core 全量库测试 420 项通过；CLI 权限过滤 103 项、推荐模式 8 项通过；
Mac 模型管理、连接并发保存、网络发现、思考配置与截图 66 项通过，补充布局、
导航和权限交互 12 项通过。已人工检查发现列表、模型详情、本地模型列表的宽窄
窗口截图。Mac 日志为 `/tmp/nola-model-center-redesign-final-20261009.log` 与
`/tmp/nola-model-center-layout-20261009.log`。此记录不代表全部平台 UI 已重构，
也不代表未实现的敏感正文外发分类链路已验收。

## Host 状态写入与文件资源验证（2026-10-08，已集中复验）

Host 自有状态不产生工作区文件 checkpoint，但不能因此免除已采用 Skill 的验证
义务。记忆和自动化适配器在提交后精确读回规范记录，返回真实资源 URI、提交版本
及同版本检查；能力声明使用现有 `MutateAndValidate`。Core 只消除匹配资源与版本
的验证义务，不用“已保存”等正文、工具名称或成功布尔值代替凭据，也不顺带清除
其他文件的验证义务。提交后读回失败必须保留已提交 effect 并报告验证失败。

文件操作的资源身份是目录项：解析并授权父目录，保留末端链接本身。符号链接版本
摘要链接内容而非目标内容；验证不跟随链接进入未授权路径。普通文件读取须拒绝
末端链接替换，避免检查与读取之间的竞态。链接、普通文件和不存在状态使用不同
类型的版本标记，不能相互冒充。

## 移动端冷恢复与流式显示边界（2026-10-09，已集中验收）

恢复已有 Run 不重新选择模型：先读取冻结的调用配置，再通过当前仍有效的授权
重建该 provider 和原始生成参数；工具计划使用已提交的 Core session，不把会话
路由 ID 当执行身份。等待权限投递后必须再次核验授权、刷新工具计划并核对 Run
及 checkpoint 见证。取消或撤销的执行者不能继续 claim。

调度结果显示租约已过期，不代表旧投递已经变成可领取状态。移动端先调用精确的
`recover_expired_run_deliveries`：在同一写事务中校验原 Run、冻结调用绑定及当前
租约到期时间，只中断真正过期的投递，再重新查询调度并领取。期间续租仍有效的
投递不得被接管；缺失租约字段必须拒绝。此操作不宣称原进程死亡，不终止整个 Run，
也不清除其他投递、委托权限或 Trace。Mac 已证明工作进程死亡时使用独立的既有
observed-owner CAS 路径，不把两种恢复证据混为一谈。

控制、权限和 Task 视图分别恢复。Task 使用独立只读冷查询返回身份与 checkpoint
revision/cursor，不塞进每个流式回复，也不把会话正文交给 UI 作为恢复控制信息。
流式显示消费已提交的 journal 游标和增量 Part；仅遇到未知消息时补充有界准入
元数据。更新单条消息不得重排或丢弃用户已加载的历史。界面同时校验会话路由、
当前 coordinator 和 Core session，三者不互相替代。

本地与远程完成状态只来自精确的已提交 terminal seal。Trace、Flow/流 EOF 和
消息已经可见都不能授权完成。界面只更新当前 coordinator 所属的原 Run；失败和
取消凭据不能解释为成功。正文增量只推进 journal 游标，结构窗口仅在新消息准入、
owner 变化和终态时刷新；切换会话或 source epoch 后不得复用旧窗口准入缓存。

远程流结束后的观察阻塞只能由同一 Run、消息、generation 和观察 lease 的真实
完成路径解除，并须成功读到规范存储已无活动 Run。迟到调用返回 stale、其他调用
仍在收尾或已经完成时，都不能借用当前新 Run 的身份执行退出清理；读取失败也
不能当成“无活动 Run”。会话级结构观察与单 Run 的执行观察分别保持生命周期。

2026-10-08 r44b Rust 工作区与真实 HTTP 模型验收已通过；Android JVM 为
358 通过、2 跳过，Android JNI 九项全部通过。Mac r45 全量 932 项中 12 跳过、
零失败；随后修正迟到完成调用边界，r46 四组受影响测试共 43 项全部通过。
iOS r46 完整模拟器验收为 566 个逻辑用例：557 通过、9 跳过、零失败，包含
恢复操作时钟和同步释放回归。Web r46 为 264 项通过，类型检查零错误、零警告，
独立 journal 向量校验通过。本轮身份、权限恢复与增量发布切换已完成验收。
这些结果不代表真实 MLX 冷恢复已验收，也不证明 provider HTTP 请求物理上仅发生
一次。最终证据和限制见[实施状态文档](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)
顶部验收摘要；该文档后续章节保留的是中间验证历史。

## 后台任务所有权收口（2026-10-08，实施中）

执行租约不是用户权限。用户授权决定某次具体调用能访问什么资源；执行所有权决定
哪个后台工作进程可以推进任务。租约只用于需要跨进程接管的执行，不作为所有前台
交互、消息显示或权限确认的通用模型，也不保证外部副作用只发生一次。

后台 Task 的动态执行权归持久化 Task/Attempt 唯一维护；Core 的父任务投影只消费
状态，不另建一套 claim、heartbeat 和 recovery 权威。每次接管分配新的执行身份，
旧工作进程不得追加任务事件、结算结果或消费新执行者的权限确认。

Task 从 Prepared 到 Admitted 必须消费实际父提交导出的有界、不可变准入事实，包含
原始会话、Turn、Run、effect、调用及子任务身份和提交凭证。裸 TaskHandle、布尔
`core_promoted`、trace 或最新快照均不能替代准入事实。执行与恢复消费同一事实，
不复制整个父 checkpoint、对话正文或配置作为任务授权。

父会话与后台任务可能属于两个存储所有者，不能伪装成单数据库原子事务。准入必须
可幂等重放；在完成准入前丢失原始临时父会话时应拒绝执行，不补造父身份。`--no-save`
仍不保留父会话正文，显式持久后台任务只保存其必要任务输入、结果及控制事实。

来源验证发生在原 Store：核验实际提交、事件游标及身份后，通过受信任的 Application/
Host 边界导出最小事实。目标 Store 不得要求内存父会话已经存在于目标数据库；也不得
仅凭调用者可自行计算的摘要认可来源。摘要用于完整性和幂等冲突检测，不等于认证。
跨进程传递复用已有本地服务信任边界，不向模型或普通工具暴露直接准入接口。

以上为本批实现目标，完整接线和集中验收尚未完成；实际证据见
[Application journal 实施状态](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)。

## 统一恢复输入顺序（2026-10-08）

同一 Run 的 Host 响应与运行中的用户/控制提交共用一条持久化输入顺序，以及统一的
4095 条、4 MiB 重放窗口。提交、事件、outbox、checkpoint 游标与发布凭证处于同一事务。
用户提交记录精确输入、决策时间、会话身份，以及冷路径在提交前实际应用的最小请求绑定；
不能伪装成 Host 响应，也不能在重放时用今天的租约或最新请求替代历史绑定。

冷恢复先从基线按序重算两类输入，再绑定当前投递状态；重放不执行工具、不重复发布。
非终态活动提交在窗口内只追加恢复事实并更新控制见证，终态和容量边界才重写完整基线。
物理基线不等于当前状态，控制判断必须读取恢复后的当前状态，不能直接解码旧基线正文。

内部任务结果导入、任务指导导入和取消提交直接消费轻量 revision／控制结果，不构造
完整恢复或展示快照。热状态取消仅在 Run、会话和当前事务 revision 全部匹配时复用
Supervisor 控制状态；未命中仍执行冷恢复，禁止从过期缓存判断权限或运行状态。

权限面板沿用现有查询命令，但 StorePort 只接收选择性的权限状态，不再以完整
StoredCheckpoint 作为权限读取参数。Runtime 仅在 Run、会话、Store revision 精确匹配且
没有未提交事件时导出热权限；Store 在固定读取快照内再次核验 revision 和游标。
热、冷路径共用原始 Turn 准入身份和控制见证校验。热读取不读取 checkpoint 正文；
冷恢复仍负责从统一输入序列还原当前状态，不能把物理基线当作当前权限。

活动提交返回值统一为 commit/events/traces/control，不再返回 checkpoint/snapshot；
任务指导和客户端活动输入消费相同契约。共享 Rust 接口切换已通过 r44b 全工作区验收。
CLI 常驻执行对象改为选择性执行状态，不保存 checkpoint 或 AgentCoreSnapshot；
收尾和活动控制从 Application 读取当前执行配置、权限、待执行标识与累计预算事实。
Core 直接借用活跃状态生成该结果，不经过可能复制终态全文的 checkpoint projection。
Supervisor 仅在提交后保留最近一份轻量终态执行状态，且仍按 Store revision 校验；
冷读取才恢复完整状态。后台记忆维护单独读取必要维护状态，不进入每次前台执行返回值。
旧 Runtime Driver 的全量 checkpoint 读取接口和 Snapshot 输出变体已删除。空闲提交
及终态任务指导恢复直接返回选择性执行状态，不再携带完整 checkpoint/snapshot。
初始准入、维护准入和回退同样返回选择性执行状态；持久化基线仍保存完整 checkpoint，
但执行确认不再将其传回客户端。CLI 直接接收该状态，已删除全量恢复结果接收方法和
snapshot presenter。iOS/Android 在领取 Host 工作前校验准入控制及权限版本，拒绝
旧全量返回字段和缺失/不一致的权限版本。Rust 和 Android JVM 已通过本轮验收；
客户端冷恢复与跨端运行验收仍待完成，不能以历史测试结果替代。

有状态的普通/子任务准入将刚接受的 Core 交给现有 Supervisor 的事务覆盖层：
先借用状态写入持久化基线，Store 提交成功后才保留 Core，避免首个 Host 回应重复
读取全文恢复。无状态和有状态命令共用准入权限门禁及任务绑定事务，不建立第二份
执行权。Android 与 Swift 一样拒绝缺失事件数组，不能将协议错误解释为没有增量。
首响应禁读全文、准入回执回滚及 Rust 畸形返回用例已在 r44b 全工作区中通过；
原生客户端仍需独立验收。

CLI 准入结果保留结构化收据状态，不能先转错误字符串再判断是否可重试。
明确拒绝、版本冲突及确定未发送的失败不要求恢复不存在的 checkpoint；成功返回
仍须接收执行状态，提交未知或待处理则保持执行禁用并核对原请求收据。子任务仅在
权限及租约检查通过、实际发送前设置该禁用状态。已提交结果的解码失败不能归类为
“未发送”：服务端返回和收据核对使用同一解码规则，保留原请求 ID 并要求恢复。
以上共享路径的回归用例已通过 r44b 工作区验收。

移动端执行消费者仅接收 control 加独立权限投影，不再回退到 snapshot，也不从执行
返回值恢复整份 Task 投影；实时 Task 状态仍由已提交事件更新。权限版本必须与控制
见证一致，并在发布日志提示、领取后续 Host 工作前校验。客户端供 UI 使用的轻量
Snapshot 类型不代表仍保留全量 Core 返回协议。冷 Task 恢复链路仍须一并验收。

冷权限恢复必须使用原始调用记录中的 Core session ID 校验 Host 请求，不得替换为
会话路由 ID；等待执行租约可用后，还须重新核验原调用及 checkpoint 见证再领取。
iOS 与 Android 已接入原调用的冻结依赖重建及实时权限校验，不允许用空回调或当前
全局配置代替原绑定。Android 初始 assistant 身份来自 active Run，而不是最新的
可见 assistant。跨平台设备验收仍须验证这条真实调用链，不能只验证绑定 DTO。

Agent 模型适配器在创建时捕获 ModelService 当前的实际 provider，并从同一实例读取
能力声明；不持有可在运行中切换的选择器。provider 安装与捕获同步，设置变更只影响
后续新适配器。此约束只保证进程内执行稳定，尚不能替代持久化 Host 调用绑定和冷
恢复重建；HTTP 配置含密钥及 URL query，不得直接序列化进调用记录。

iOS 普通准入现从 IOSPreparedRunModel 的实际执行来源及 Core 输出准入结果构造
HostInvocationBinding，并经 ChatManager → DeviceAgentCoordinator 写入 turn_identity。
执行策略摘要覆盖本次 Host catalog、工具 schema、权限规则和上下文预算。生成参数
保留整数 seed、stop 原文及实际输出上限。凭据仍使用既有非敏感摘要和临时来源标记，
这些摘要不等于凭据本身，也不授予后续调用权；恢复必须重新取得当前仍有效的凭据。

后续接线：iOS 生产冷恢复现从原绑定恢复生成参数及输出预算，从当前仍获授权的本机
模型记录独立取得凭据，再严格比较 route、密钥/query 摘要及 generation。使用的是
协议允许的临时凭据独立验证，不伪造托管引用。原生模型按已准入 catalog ID 重建。
实际恢复回调已接入协调器；等待结束后重新核验凭据与工具授权，并比较包含模型能力、
已安装 Skill 内容身份的策略摘要。配置或授权不匹配时拒绝，不回退到当前选中模型。
Android 使用同样的冻结配置、精确 Core session 和冷 Task 投影恢复边界。
最终客户端验收状态见本文首节及实施状态文档，不以历史单元测试替代设备运行。

终态消息归属必须来自本次 TurnStarted 的明确绑定，或同一会话、同一 Run 的持久化
Core Turn 归属。不得通过 `turn_id + ":assistant"` 猜测后续回合的消息 ID；缺失或冲突
直接拒绝本次提交。初始消息仍由原始 Run 准入事实指定。
按本轮要求，整体接通后再集中编译测试；历史回归不代表当前改造已通过。证据与剩余调用点见
[Application journal 实施状态](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)。

## Rust 构建与缓存存储策略（2026-10-08）

此前集中验收在链接阶段耗尽磁盘，并非运行时断言失败。原因不止 Cargo target：
macOS unpacked 调试对象、多个原生输出目录及 mbx 自动学习的私有增量状态共同放大
存储。仅设置 Cargo `incremental=false` 不会关闭 mbx learned incremental。

- 工具链使用 `rust-toolchain.toml` 的共享 stable 安装，已从 1.91.1 升级到 1.99.0；
  不另保留同版本的数字命名工具链副本。升级保留已安装跨平台 target。
- 默认 dev/test 使用 line-tables-only、split-debuginfo=off、incremental=false；
  保留文件行号回溯，不保留全量变量类型信息。完整 LLDB 调试显式使用
  `cargo build --profile dev-debug`（full/packed），不污染日常构建配置。
- 原生 CLI、Mac 内嵌服务和真实 Mac XCTest 共用 workspace `target`；跨平台 ABI
  仍由 Cargo target triple 隔离。明确的 target-dir 覆盖保留。删除 Mac 自行按二进制
  mtime 跳过 Cargo 的捷径，由 Cargo 核验依赖、profile 与工具链指纹。
- 默认 Cargo 并行度为 2，可通过 `-j`/`CARGO_BUILD_JOBS` 覆盖。公共 launcher 默认
  关闭 mbx learned/eager incremental，仍保留 Cargo freshness 和 mbx 共享 action cache。
- `sh scripts/configure_rust_cache.sh --apply` 是显式的**本机共享**配置操作，不由构建
  隐式执行：总回收预算 40 GiB，action 12 GiB、managed target 24 GiB、增量 4 GiB，
  单 crate 增量上限 1 GiB，磁盘保留 12 GiB，15 分钟回收间隔、7 天非活跃期限。
  会影响本机其他项目的缓存策略，但不改远端凭据。构建脚本不自动删除用户文件。
- 本机已用 `mbx adopt /Users/gaojc/Desktop/Nola` 纳管原 target，保留原路径符号链接；
  未纳管目录不受上述 target 预算约束。其他 checkout 需显式 adopt，不递归扫描迁移。
  回收先用 `mbx gc --dry-run` 检查，再执行 `mbx gc`；清理单项目用
  `mbx clean /明确的项目路径`。只针对可再生编译状态，不涉及会话数据库。

预算是软回收目标，不是磁盘配额：活跃构建、受保护状态和临时链接文件可能超出预算。
10-09 发现最近使用的单个 target 也会被 mbx 整体保留，不能用该预算约束内部旧变体。
改用显式 `scripts/clean_rust_workspace.sh`（默认预览、`--apply` 才清理）委托 Cargo
只清本项目 native Debug workspace 包；保留第三方依赖和 Release。构建超 24 GiB 提醒，
不做隐式清理。实测清理前后及逻辑/物理空间差异见 `../工程规范/RUST_BUILD_CACHE.md`。
遇到空间不足应结束该轮验收并释放明确归属的旧产物，不循环重试，不运行广域 rm。
依据：[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)、
[mbx 配置与预算](https://mr-boxington.jdx.dev/configuration)。

本批构建脚本 37 项单测通过（`python3 scripts/tests/test_rust_build_cache.py`）；
实际新工具链的全量 Rust 运行时验收另计，脚本测试不替代应用功能验收。

## 本轮整体架构整改进度（2026-10-04）

2026-09-28开启的goal已落地共享生成/能力准入、会话与远端profile意图、CLI Provider
依赖拆分、精确执行/凭据来源、权限与取消恢复、Mac运行/订阅/呈现/提交所有者、
类型化记忆策略，以及canonical正文writer/reader/replica等改造。不是只有设计文档或
少量UI补丁。累计14类交付、分平台证据、当前回归失败和剩余门槛统一记录在
[整体模块化方案第8节](../实施与审计/MODULAR_REDESIGN_2026-09-28.md)，逐批历史在其第9节。

必须区分三条内容合同：私有canonical Text分块writer已接生产并做规模实测；公开
assistant journal已实现Domain/Store/planner，但未接完整生产提交点或客户端传输；
Core私有控制checkpoint/恢复正文分离仍未实施。10-01富字段公共语义已接实际跨端
消费者，Web回归通过，Rust六crate lib、Application完整tests、Android完整JVM及
Mac/iOS各七类原生build/test已通过（含iOS统一整数codec）；CLI完整binary重跑
1,403通过/2忽略，短stream本地deadline、主区tail及同锚点指定PTY已重验。
fifty-read汇总/终态也已有实际验收。准确范围与尚未完成的Markdown/公开journal
生产迁移见上述总览。不沿用旧产物的绿灯，也不把定向过滤组当作全部平台验收。

下方带日期的联调记录保留其当时证据，不当作最新源码或本轮全部变更的清单；具体
模块边界以[模块合同](../实施与审计/ARCHITECTURE_V2_MODULE_CONTRACTS.md)和上述专项合同为准。

10-02继续改的是发布边界，不是新增UI补丁：完整有序多assistant/多frame原子集合、
schema33集合凭证及成员表、实际编码预算分帧，以及不复制累计正文的canonical owner
映射已通过六crate lib（1,119项）和完整Application/Store/Domain tests（1,037项，
含重叠lib，不能相加）。CLI完整重跑1,408通过/0失败/2忽略；首跑两项目录
混版本失败及两层capture整改见总览8.6。私有Text的2/8/32MiB规模
专项已在schema33重新通过。10-01跨端结果不自动继承到本批Store/Application源码。
整组不可变/replay并不证明第一次producer没有遗漏Core可见变化；权威changed-owner
coordinator、Task/Skill/Attempt、idle/recovery来源及生产接线仍须完成，不提前切消费者。
另确认旧inline历史range每页重复全JSON披露扫描的性能缺口；响应内存有界不等于
计算量有界。整改与本次实测分别记录在[规模验收](../实施与审计/STREAMING_SCALE_ACCEPTANCE.md)。
CLI单目录capture之后继续迁移Skill会话所有权：不可变`SkillRuntimeContext`
区分managed、明确禁用和显式目录，贯穿目录/signature、driver、后台恢复和实际
加载。目录元数据不扩大文件权限，展示路径不充当授权来源，执行仍核验当前Host
manifest。该批完整CLI及指定Web联调已通过，准确结果见总览8.8；源选择不是
文件版本封存，也不保证消除实时撤权或文件替换竞争。

10-03新增共享Rust编译缓存接入和持久任务执行权整改：
[缓存合同](../../软件架构设计/工程规范/RUST_BUILD_CACHE.md)保留原产物目录，不做全局shim或target迁移；
跨不同新工程路径已有实际cache hit，但不是整项目速度/空间结论。
[durable合同](../../软件架构设计/模块/05-执行与恢复/DURABLE_RUNTIME_DESIGN.md)参考Pi实际提交与恢复代码，统一
Retry/Finish与续租/进度的typed执行权，并要求worker持有原attempt，不从registry
后继身份借权。当前实现/回归状态统一见整体方案8.9/8.10，未称全平台完成。

10-04根据真实Mac最新会话收口配置、执行和显示边界：未声明能力不再静默变成全关；
Core冻结的工具投影与Host实际请求必须相符，文本步骤的纠错不再要求不存在的工具。
工作区授权只消费精确typed结果，cwd不限制其他已授权产物目录；显式产物使用精确
preimage恢复。Core拥有Run终态，Mac使用持久Attempt事实而非重复传输失败卡片。
最新Mac原生合同、CLI完整回归、Web联调与CLI/Mac真实模型关键链路已通过；
持久化审计读回原文必须使用共享Application接口，不能以显示或trace缓存替代。
实际问题、验证状态和保留门槛见[总览8.11](../实施与审计/MODULAR_REDESIGN_2026-09-28.md#811-mac实际会话暴露的执行和显示边界)。

10-04续批把实际Turn/Part归属写入同一canonical事务，internal续跑核验真实Message、
前序Run和checkpoint CAS。权威changed-input已接提交前校验，窄owner/order/successor
端口和公共单项语义已有实现；它们不等于完整journal发布已启用。初始memory Source
持久化、独立Core session的读取与internal handoff历史卡片正在同批整改，验收及当前
限制见[总览8.12](../实施与审计/MODULAR_REDESIGN_2026-09-28.md#812-初始内容独立会话身份与历史归属2026-10-04)。

## 工作区访问与恢复（2026-09-19）

2026-09-20 再次排查确认：此前双向汇合修复仅保证“两个事件均已送达”的情况。
Emitter 暂存权限投影，Host 在另一 stdout 出口发送 challenge 后仅置 delivered 标记，
需要下一次 Emitter emit 才刷新队列；此时 Host 已等用户决定，Mac 则等投影，形成死锁。
现移除 delivered 标记集合与 Emitter 私有权限缓冲，改由 `native_host_output` 单一
writer 管理预留位置和同步排空；challenge 发送本身即完成交付屏障，不依赖下一事件。
进程测试先接收 challenge + 匹配 pending projection，再分别批准与取消并验证落库；
Mac 实机验收 harness 改为经过生产 WorkspaceModel 的权限消费入口，不再直接答复
原始 challenge。模型服务 fixture 测试与真实模型/真实 Mac 验收分别报告。

批准验收另外暴露了资源合同遗漏：`bash.output_artifacts` 为含 `path`、`operation` 的
对象数组，而 Host 曾声明空 resource_keys，导致成功变更只留下未知资源 `*`，之后即使
read_file(validate) 成功也不能结清验证义务。修正 Host 的冻结资源描述声明 `path`，
复用 Core 现有的 cwd + 相对路径投影；不通过匹配自然语言、忽略验证或放行终态绕过。
验收不固定模型的授权次数（验证可用 read_file 或 bash），但必须至少经过一次权限卡片，
每个 challenge 均到达实际 UI 消费器，并验证 canonical completion 与产物。

本次已通过：有序输出单测、无后续事件的 Host 子进程批准/取消与落库测试、Mac 权限
消费者 11 项回归，以及真实 Mac + 真实模型的拒绝、批准后读回验证场景。
其中 shell 专项已验证 bash 相对 output_artifacts → read_file 绝对路径 validate →
canonical completed，保留严格权限卡片与持久化检查。
历史的单纯写入后未验证即答复失败样例保留为独立证据，不能用带显式验证步骤的场景替代其验收。

2026-09-20 授权事件与停止合同：Core 的 pending permission projection 与 Host 的
nonce challenge 是独立事件流，不保证先后顺序。Mac 在任一事件到达时汇合最新状态；
只有会话、generation、request ID、call ID、capability 都匹配且请求仍在最新投影中，
才可显示授权入口。重复事件幂等，旧 revision、已撤销请求或停止中的 generation 不得
重新打开卡片。不能依赖延迟重试、重复投影或 Host trace 到达顺序来保证正确性。

Stop 先表示“正在停止”，不提前宣称终态。原生 Host 收到 SIGINT/SIGTERM 时同时设置
execution cancellation 并关闭交互等待，唤醒权限、设备工具和用户输入 waiter；
关闭是 transport interruption，不是用户 deny/allow。执行返回后由 Core/Application
提交 Interrupt 并排空终态 effects，持久化 run、assistant 与 Parts 的 cancelled 状态。
Host 超时兜底仅终止仍登记的同一个子进程，不能对已退出后复用的 PID 发信号。
回归测试包含真实 Host 子进程在 permission_request 后收到 SIGINT、stdin 保持打开，
并检查独立数据库中的取消终态；模型服务使用确定性 fixture，不冒充真实模型验收。
模型指导明确覆盖用户可见进度的语言一致性，以及已给定目标目录时不先扫描父目录。
这属于通用语义指导，不增加中英文关键词路由或自动授权；不能据此声称模型行为已获
确定性保证，真实模型自然语言回归仍须独立验收。

2026-09-20：Mac 书签存储改为每会话多目录 record（`workspace.access.v2`），primary ID
与附加目录独立；Host 启动时显式注入全部重新激活成功的 roots。新增 `MacWorkspaceBookmarkIO`
平台端口，隔离 NSOpenPanel、OS 句柄、书签编解码，测试用隔离 UserDefaults 与确定性适配器。
附加授权不再改变工作区选择；运行中禁用 sandbox 撤销，防止 UI 声称撤销而 Host 仍持有旧权限。
书签解析在后台线程进行，回到主线程后验证持久记录快照；解析期间发生撤销或重新选择时，
旧结果不得重新激活目录。平台端口测试覆盖多目录恢复、默认目录独立、失败不授权、撤销竞争。
原生 NSOpenPanel 的打开/取消有 opt-in 真窗口测试；当前 macOS 的 `ok:` 自动确认接口
抛出未实现异常，因此不能将确认路径描述为已完成 GUI 自动化验收。

新增 `workspace_access(root)`：独立 Core permission domain、初始工具可见、串行 HostState
执行边界。成功结果返回规范化 root；Host 将它加入本会话根集合且保留原 cwd。文件和 shell
越界统一返回 `workspace_access_required`，普通操作批准不再绕过目录校验。Mac 仅对这一
能力请求 OS 目录授权（包括用户直接点 Allow），不再以 root/path/cwd 等任意参数猜测授权。
按会话保留 security-scope 句柄，选择无关目录不落盘，恢复中的旧书签不覆盖新选择。

参考 `参考资料/opencode-dev/packages/opencode/src/tool/external-directory.ts` 与 `bash.ts`：
外部目录访问和命令执行分别判定。本实现通过显式能力复用 Core 的 challenge/decision 流程，
不引入根据自然语言自动放行的客户端分支。完整不变量见模块合同“外部工作区访问合同”。

> 文档定位：本文件记录当前实现、接口和联调细节。总体数据所有权、分层与不变量以
> [ARCHITECTURE.md](../实施与审计/ARCHITECTURE.md) 为最高权威。Core checkpoint、平台缓存和 Trace
> 都不是完整产品数据事实源；仓库不保留与该合同冲突的兼容实现。

Host 仅输出 canonical `core_part_updated` 与带 Core Trace 的 `agent_trace`。客户端和 E2E
不解析 `assistant_delta`、`tool_use`、`tool_result`、`done` 等旧帧；开发阶段不迁移旧
schema、checkpoint 或会话数据。

Host-effect 租约的续期按单调时钟截止时间驱动，不能仅依赖响应队列空闲超时；持续到达的
模型 delta 也必须在下一个响应入库前触发到期心跳。只有 Application 确认的新租约可写入
后续响应的 fencing identity，续期失败须中止外部调用，不能以过期租约提交终态。

模型配置按稳定 model ID 归一到唯一 `ModelProfile`。显示名不参与持久化身份，模型参数
不使用独立兼容 Store、前缀键或影子 Profile。

远程模型的 `capabilities` 只表示产品用途（chat、retrieval 等），不能据此推断模型调用能力。
`executionCapabilities` 单独声明该端点上的具体模型是否支持原生工具调用、图像输入、推理
和采样参数。缺失或 JSON `null` 表示尚未声明，不是“不支持”的证据；模型仍可在目录中
显示和编辑，但调用前必须拒绝并要求确认。完整声明全 false 则是可用的纯文本模型。
地址、协议或具体模型身份改变后旧声明失效；显示名称和启用状态不改变执行能力。
共享端点更新命令与客户端模型中心遵守同一规则，不能跨新路由继承旧能力。
`samplingRequiresNoReasoning` 进一步表示采样参数仅在显式
`reasoning.effort=none` 时可用；该合同要求 Responses 协议、已声明推理能力和 `none` 可选强度。
Chat Completions 的输出上限字段由同一模型执行合同中的 `chatOutputTokenField` 决定：
`auto` 对官方 `api.openai.com` 端点采用 `max_completion_tokens`，对其他兼容端点采用
`max_tokens`；模型档案可显式选用任一字段以支持代理或特殊兼容实现。此选择只依据精确端点
身份与档案配置，不依据模型名、用户措辞或失败后的猜测重试；Responses 固定使用
`max_output_tokens`，Anthropic Messages 固定使用 `max_tokens`。
Automatic 不等于 None，因为模型默认强度可能不同。端点的 `wireProtocol` 只选择 Responses、Chat Completions
或 Anthropic Messages 适配器，不自动赋予该模型推理、工具或视觉能力。模型中心允许编辑并
保存这两类不同能力，但不能保存要求推理控制而未声明推理支持的配置。
推理能力声明是观测和适配合同，不是用户选择的开关。`Automatic` 不得因能力缺省或
`reasoning=false` 而向提供方发送禁用 thinking 的参数，也不得强制覆盖提供方默认 effort；
只有用户显式选择 `none` 才发送禁用语义。Responses 的 stateless 续接保存并重放完整
output items（含提供方返回的推理项），不依赖旧版 `include` 请求参数；终态 envelope
必须明确是已完成的 `response` 对象，须与已流式接收的文本和工具调用相符，并补齐缺失的
末尾 delta，不能把完整输出吞掉或重复显示。
模型中心的 Responses 验证必须对当前端点与具体模型发出一次真实 Responses 请求，并验证
`response` envelope；验证通过只把协议切换写入未保存草稿，不自动修改模型推理能力或
effort 列表。HTTP 成功可能来自忽略未知参数的兼容网关，故不能把路由可用等同于 effort
生效。能力声明应来源于模型文档或单独的、可说明证据强度的验证；未声明在 UI 中表达为
「尚未配置」，而非提供方「不支持」。验证期间若端点、凭据或模型草稿变化，旧结果不能
回写新草稿。模型档案、端点协议与会话选择分别冻结到 Turn，三者不得互相推断。
每个客户端必须将协议身份随模型档案持久化并冻结到 Turn；若本机尚无对应适配器，
必须在发起网络请求前明确拒绝，不能把 Responses 或 Anthropic 的档案交给 Chat Completions
适配器。Responses 的工具续跑还必须保留提供方的 `function_call`、`call_id` 与必要的推理
输入项，不能仅凭聊天文本或工具结果重建协议状态。同一用户 Turn 中每次模型调用的
Responses output items 都随对应 canonical reasoning item 持久化（即使没有可显示的推理
文本），按原顺序附着到相应 assistant/tool-call 消息。Responses 无状态请求跨 Turn 回放
这些原始 output items 及其对应工具结果；只有其他协议的旧 Turn 工具调用才改写为有界
历史证据。会话压缩可替换已完成的旧历史，但不得截断当前用户请求之后的 Responses
推理、调用和结果链。路由别名（如 `openrouter/free`）的续接身份仍是 Turn 冻结的配置模型；
提供方返回的具体模型另存为来源信息，并可在同一路由续接时锁定实际模型，不能把它写成
canonical continuation 的 `model_id`。仅有 opaque continuation、没有可见摘要或文本的 reasoning
item 属于 canonical 协议状态，不生成空白的用户界面思考卡片。

一次 Turn 的调用配置按 `ModelProfile + endpoint protocol + model defaults + sparse conversation
overrides` 解析和验证后冻结：会话显式值（含零值和清空）高于模型默认值；不支持的推理、
采样或协议专属生成参数必须在网络请求前失败，不静默忽略、不让 UI 通过用途标签猜测。
模型声明无条件采样支持时发送其保存的默认值（即使数值恰好等于 Nola 的回退值）；
条件采样合同在 Automatic 下省略采样参数，仅显式覆盖并选择 `none` 时下发。不支持或
会话显式关闭采样覆盖时省略 temperature/top-p，由 provider 采用自身默认值。Host 只能依据冻结
配置生成协议请求；macOS Host 请求中声明的视觉和执行能力不得被 CLI 默认值重新覆盖。
CLI 当前的 fallback 传输只替换端点、密钥和模型 ID，因此 macOS 只能把协议、执行能力、推理
合同和完整默认参数均相同的模型纳入同一 fallback 链；不同合同必须作为独立路由处理，不能
借用主模型的能力或采样参数。

macOS 聊天输入框下方只提供当前会话的快捷选择，模型中心负责端点和模型默认合同；
快捷菜单须明确区分「继承模型默认值」（移除 sparse override）与「Provider default」
（显式 `automatic` override）。从快捷菜单或模型选择器进入模型中心时，应打开当前模型
对应的 API／本地页，并定位 API 模型档案。模型中心保存后发出的配置变更事件必须刷新
聊天端可观察的模型目录修订，使当前模型名称、支持的 effort、默认值及非法旧会话选择
同步重算；已冻结的运行中 Turn 不随之改变。

iOS 的模型目录、最近选择与 URL 抓取历史直接使用 canonical Store。Core 写命令允许以
JSON `null` 表示成功 acknowledgement，客户端适配器不得把缺少业务结果误判为写入失败。

版本：2026-08-24（更新：canonical Part、Projection 与 Trace 所有权收口）
状态：Active  
适用范围：iOS + macOS + Android + Web + Rust CLI + 共享 Rust Core + Cloudflare Worker 后端

## 1. 文档目标

本文档用于定义 Nola 当前可落地、可联调、可演进的系统设计，重点覆盖以下范围：

1. iOS、macOS、Android 与 Web/CLI 客户端
2. 共享 Rust Agent Core 与平台 Host 边界
3. Cloudflare Worker 后端
4. Chat、数据源、长期记忆、鉴权、订阅与定时动作
5. 本地开发联调模式与跨端测试边界
6. Agent 执行追踪、持久化回放与跨端一致的流式呈现

本文档替代历史混合内容，避免"需求、变更记录、实现片段"混排导致维护困难。

## 1.0 跨端执行单元与消息渠道

Nola Agent Core 负责会话、上下文、能力目录、权限、事件和持久化。每个会话可以在产品标题区域选择执行单元：`Nola`、已安装的 `Codex`、`OpenCode` 或 `Claude Code`。客户端只展示 Host 返回的发现目录和可用状态，不根据用户文本猜测执行器。外部执行器由 Host 以独立进程适配，保存其原生会话标识，以便后续轮次恢复；未安装、版本探测失败和启动失败必须作为明确状态返回。

该选择是会话级配置，Mac、iOS、Android、CLI 和 Web 使用同一 `execution_backend` 语义。外部执行器自身的沙箱、登录和审批状态不等价于 Nola 权限授予，Host 不得伪造权限对等关系。

模型来源与执行单元是两个正交维度：API、本地 MLX/GGUF 和远端 Host 决定模型推理来源；Nola、Codex、OpenCode、Claude Code 决定 Agent 执行 harness。实现参考 Unsloth Desktop 等本地模型产品的分层方式，但 Nola 不把外部执行器嵌入模型运行时。Host 发现已安装可执行文件、探测版本/登录/可达状态并返回目录；客户端只呈现该目录。标题区域的选择不得启动本地模型，也不得通过关键词改写用户请求或强制路由。

Telegram Bot、WhatsApp Cloud API、微信公众号和企业微信通过统一渠道链路接入：提供方 webhook 经签名验证和去重后写入 Worker inbox；CLI Web Host 以租约领取消息，交给 Agent Host 和该会话选定的执行单元；完成后由 Worker 调用提供方发送接口回复。租约支持续期、完成、可重试失败和终止失败，防止多 Host 重复消费。密钥只保存为 Worker 环境变量引用，客户端和 Host API 不返回密钥值。个人微信没有受支持的通用机器人接口，因此 `wechat_official` 明确表示微信公众号，`we_com` 表示企业微信。

Web/CLI 提供完整渠道画廊、连接编辑、回调地址和安全密钥引用管理。Mac、iOS 与 Android 显示 Host 汇总状态，并通过一次性安全管理跳转进入同一控制面，避免把渠道密钥复制进每个客户端。渠道验收必须同时覆盖提供方 wire contract、Worker 持久化/租约、Host 消费、真实执行单元/LLM 和回复发送；仅渲染一张渠道卡片不构成功能验收。

Mac 后台调度只允许稳定安装位置中的 App 注册。LaunchAgent 直接运行 bundle 内的 `nola-agent-host --background-scheduler-tick`，并使用与前台一致的 `MacAppEnvironment.databaseURL`（当前开发环境为 `State/nola-v4.db`，不读取或迁移旧数据库）。原生 UI、Agent Host 与后台调度器不得各自推导数据库文件名。DerivedData/测试构建必须注销同 label 的陈旧任务；启动时 Core 清理死亡 PID 持有的 session run lock，并按会话隔离恢复遗留 Run，防止旧锁永久阻塞会话。

## 1.1 核心架构原则（2026-04-24）

长程目标的 Manager、Executor、Auditor 分层、证据门禁、租约接管、定时唤醒和
跨进程恢复规范见 [GOAL_LONG_HORIZON_ARCHITECTURE.md](../../软件架构设计/模块/10-Goal任务与子代理/GOAL_LONG_HORIZON_ARCHITECTURE.md)。

Nola 是开源的本地优先（local-first）个人助理，遵循以下原则：

1. **客户端自持**：客户端必须在没有后端的情况下也能完成本地核心功能
   （注册、登录、本地设置、查看已有数据等）。
2. **后端仅用于必要联网能力**，是作者自运营的付费私有服务，仅在下列场景使用：
   - LLM 调用中转
   - 推送通知调度与下发
   - 多端同步（订阅付费后开启）
   - 订阅 / 支付（Stripe）
3. **最小上传原则**：非必要不上传用户信息；本地注册/登录不上传；官方后端账号仅接收 Apple、Google 或 GitHub 已验证身份返回的最小账号字段。
4. **显式上传披露**：任何将用户数据上传到后端的动作都必须在执行前通过明确的
   说明页或弹窗告知用户具体上传的字段、用途、是否可撤销，并需用户主动勾选同意。
5. **本地业务逻辑优先**：业务逻辑尽量落在客户端；后端只处理它必须处理的部分。
6. **Chat-first**：Chat 是主交互界面；模型、宿主机、工具、数据源和记忆作为对话能力呈现，避免形成互相重复的设置孤岛。
7. **数据源驱动**：文件、网页、系统数据、健康数据和工作区都通过明确的数据源/附件边界进入上下文。
8. **自优化与长期记忆**：上下文压缩、检索与模型路由由系统根据真实 token 和能力状态调整；稳定记忆与用户偏好自动总结、去重并可管理。
   Mac 的跨会话长期记忆默认关闭，只有用户明确启用后才参与新会话和定时任务；普通聊天界面不额外暴露记忆开关。
9. **Part 与 Projection 单一内容源**：Core 的 typed Part lifecycle 描述正文、思考摘要、
   工具、来源与 Artifact；Application 原子提交后形成 `ConversationProjection`。Trace 只描述
   进度、耗时和协议审计，不拥有产品内容，也不能用于恢复或覆盖消息。平台 Host 只执行
   effect 并返回相关响应；任何客户端不得自行生成第二套消息、Part 或执行状态机。
10. **产品事实单一写入口**：完整消息、Message Part、运行、记忆、权限、Skill、MCP、
    数据源、任务与自动化只能通过 Application Command 写入 Nola Store。客户端缓存只用于
    草稿和投影，不得在流结束时用 Provider 文本覆盖已提交消息，也不得把 `UserDefaults`、
    DataStore 或 JSON 文件升级为第二事实源。
11. **消息查询不降级**：会话查询中的每条 `messages[]` 直接携带其有序 `parts[]`。顶层
    `conversation_projection` 是按稳定 `part_id` 增量渲染的快照，不是第二份消息数据库。
    OpenAI/Anthropic 等供应商要求的 `tool_calls`、`tool` message 和 reasoning 回传格式只属于
    模型上下文投影，不得成为产品会话契约，也不得覆盖 canonical Message/Part。
12. **投影来源必须可验证**：本机 Agent Host 使用稳定 `source_host_id=local`；远端传输层
    必须在认证后替换为连接 Host 的真实标识。客户端不得因字段缺失而猜测来源，也不得为了
    兼容缺失字段而放宽远端投影校验。

Web 客户端的具体实现：
- 本机 SQLite 文件位于 `$NOLA_DATA_DIR` 或 `~/.nola/cli/`。会话、权限、Skill、durable
  Task、长期记忆、自动化定义和自动化执行结果由当前 `nola-store` schema 持久化，并且只能
  通过 `nola-application` 写入；`cli/src/store/` 只提供产品用例适配，不保存 JSON 产品状态，
  不包含旧表或旧 schema 读取路径。Core checkpoint 直接按 `session_id` 读写 SQLite，不能用
  `*.checkpoint.json`、`*.context.json` 或伪文件路径传递会话身份。
  数据源定义、脱敏验证契约、会话覆盖、使用/云披露审计和 MCP 目录属于同一当前 schema；文件与 HTTP
  原文只由 Host 按需读取，不写入 Store。桌面与移动端通过同一 ApplicationCommand 合同访问
  数据源、长期记忆和自动化事实。
- 注册/登录全部本地完成，密码以 Argon2/Bcrypt 散列存储（由 Rust CLI 处理）。
- 当前 Web 通过 `cargo run -- web` 启动（`cli/src/web.rs`），由 Rust CLI 内嵌的 HTTP 服务器
  （基于 `tiny_http`）服务 Jinja2 模板 + 静态资源 + 本地优先 API。无 Python 运行时依赖。
- `/link-backend` 页面是显式后端链接入口；跳转 Apple、Google 或 GitHub OAuth 后自动注册/复用官方账号。
- Web API 入口以 RPC 风格分布在 `cli/src/web.rs` 的路由处理中，不再使用 Python 路由装饰器。

iOS 客户端的具体实现：
- `BackendUploadConsent`（`Nola/Services/BackendUploadConsent.swift`）是全局唯一
  的上传同意开关，持久化在 `UserDefaults`（`com.nola.privacy.backend_upload_consent_v1`）。
- `PrivacyDisclosureView`（`Nola/UI/PrivacyDisclosureView.swift`）通过"设置 →
  隐私与数据上传"进入，逐项列出将会上传的字段（Apple 身份令牌、客户端 UUID、
  APNs 令牌、时区、定时任务配置、AI 对话内容、订阅凭证），用户打开开关即记录
  同意。
- `PaidPushClient.authenticateWithApple` 与 `ServiceContainer.signInForPaidPushAndSyncIfPossible`
  在未同意前会抛出 `PaidPushClientError.consentRequired`，从而阻止 Apple 登录
  弹窗出现；错误的 `localizedDescription` 会引导用户去设置页完成同意。
- 撤销同意只停止后续上传，不会删除服务器上已有数据。

### 1.1.1 官方后端 OAuth 登录与自动注册（2026-05-09）

官方后端账号不再使用 Nola 自建邮箱密码鉴权。账号建立/登录统一由第三方身份提供方完成：

| Provider | 后端入口 | 校验方式 | 自动注册/复用规则 |
|----------|----------|----------|------------------|
| Apple | `POST /v1/auth/apple` | 校验 Apple identity token JWKS、issuer、`APPLE_SIGNIN_AUDS` audience | 按 Apple `sub` 或已验证邮箱复用；否则创建/映射账号 |
| Google | `POST /v1/auth/google` | 校验 Google ID token 签名、issuer、可选 audience | 按 `google_user_id` 或已验证邮箱复用；未验证邮箱只生成 provider synthetic email |
| GitHub | `POST /v1/auth/github` | 用 GitHub access token 调 `/user` 与 `/user/emails`，取主 verified email | 按 `github_user_id` 或已验证邮箱复用；否则创建 `auth_provider='github'` 账号 |

`/v1/auth/register`、`/v1/auth/login`、`/api/mobile/login`
均返回 410；后续 API 仍使用后端签发的 Nola JWT (`Authorization: Bearer`)。
Web 的 `/login` 和 `/link-backend` 展示 Apple / Google / GitHub 入口。本地 Web
推荐配置 `OAUTH_RELAY_BASE_URL`：点击第三方登录后先跳转官方后端
`/oauth/login` 登录页，用户在官方域名页面选择 Apple / Google / GitHub 后进入
`/oauth/start/{provider}`，官方后端用合法域名回调
`/oauth/callback/{provider}` 完成 provider 认证，生成一次性 relay code，再回跳本地
`/auth/relay/callback`；本地 Web 用 `/oauth/relay/exchange` 换取 Nola access token
并缓存在本机 `LocalUser` 中。这样本地 `127.0.0.1` 不会作为 Apple/Google/GitHub
的 redirect URI，避免 `redirect_uri_mismatch`。Apple / Google / GitHub 的 OAuth
Client ID / Secret 只允许配置在官方后端的 `nola_backend_env` JSON；Web 端只保存
`OAUTH_RELAY_BASE_URL`，不保存 provider secret。

OAuth 接口字段约定：

| 接口 | 请求体 | 成功响应 |
|------|--------|----------|
| `POST /v1/auth/apple` | `{ "identityToken": "...", "clientUuid": "<uuid>" }` | `accessToken`, `expiresIn`, `userId`, `email?`, `displayName?`, `authProvider="apple"`, `providerUserId`, `appleUserId` |
| `POST /v1/auth/google` | `{ "idToken": "...", "clientUuid": "<uuid>" }` | `accessToken`, `expiresIn`, `userId`, `email?`, `displayName?`, `authProvider="google"`, `providerUserId` |
| `POST /v1/auth/github` | `{ "accessToken": "...", "clientUuid": "<uuid>" }` | `accessToken`, `expiresIn`, `userId`, `email?`, `displayName?`, `authProvider="github"`, `providerUserId` |
| `POST /oauth/relay/exchange` | `{ "code": "<one-time relay code>" }` | 与 provider 登录成功响应一致 |

Provider 登录请求体里的 `clientUuid` 必须非空且为合法 UUID（hyphenated 或 hex 均可），后端统一规范化为 hyphenated UUID；OAuth Relay 的 `client_uuid` query 参数如省略则由后端生成。Web 会在本机会话内生成并传递合法 UUID；iOS 使用 `PurchaseService` / Keychain 中的本地 UUID。Apple 应配置 `APPLE_SIGNIN_AUDS` 同时包含 Web Services ID（如 `com.nola.auth.apple`）与 iOS Bundle ID（如 `com.gaojc.Nola`）；后端签发的 Nola JWT `sub` 统一使用 `provider:provider_user_id`（例如 `apple:<sub>`、`google:<sub>`）。

### 1.1.2 当前账号身份规则

Apple、Google 和 GitHub 登录都以已验证的 provider subject 绑定唯一
`UserAccountRecord.user_id`。客户端收到 `userId` 后直接将其作为同步、订阅、推送与
聊天的账号标识。当前版本不读取旧身份表、不迁移设备 UUID，也不提供账号合并接口；
冲突绑定直接拒绝，由用户重新选择正确账号。


## 2. 系统总览

Nola 采用多端本地客户端 + 可选网关后端的架构：

1. iOS 客户端：移动端 Chat、HealthKit/系统数据、通知、记忆与本地模型
2. macOS 客户端：原生桌面 Chat、工作区、本机/远端宿主和桌面工具；普通聊天不启动 CLI 进程
3. Android 客户端：Compose Chat、Health Connect、平台权限和 JNI Agent Runtime
4. Web/CLI：独立的本地控制台、终端工作流与自动化宿主
5. 共享 Rust Core：统一 Agent 状态机、权限排序、工具事件和上下文规划
6. Cloudflare Worker 后端：OAuth、模型代理、订阅、通知与可选同步
7. 共享 canonical Part/Projection：统一正文、思考摘要、工具、来源、Artifact 和上下文用量；
   Domain Event/Store 负责恢复。Diagnostic Trace 仅提供进度、时序和审计，平台 presenter
   只把 Application projection 与瞬时动画 overlay 映射到原生 UI

### 本地模型运行边界

API 模型与本地模型是模型中心中的平行来源，不能把“本地模型”实现成 API 配置的
附属开关。选择、下载、启动是三个独立状态：选择模型不分配推理内存，下载完成后仍需
用户明确启动；停止会释放容器并使本地 OpenAI 兼容端点退出可用状态。聊天只能选择已经
启动的本地模型，不能因选中或进入模型页面而隐式启动模型。

模型状态按所有权拆成四层，任何客户端不得用一个布尔值同时表达这些状态：

1. 会话选择是 Nola Store 中的稳定引用：执行单元、Host、模型来源、provider 与 model id。
   它随会话恢复，但不证明对应运行实例当前可用。
2. Host 模型配置保存端点、模型目录、默认采样参数和安全凭据引用。密钥正文只存在于
   Keychain/Keystore/进程凭据注入，不能进入 Store、会话设置、日志或 Host Catalog。
3. 安装记录保存 artifact path、格式、量化、校验摘要、原生窗口和资源估算。下载中、已下载、
   校验失败与可启动是不同状态；文件存在不能替代完整性校验。
4. 运行实例是 Host 的短生命周期能力，由 Catalog revision、availability epoch、实例 id、
   启动阶段、实际窗口和 heartbeat 描述。进程退出或 lease 失效即不可用，不能由持久配置推断。

开始 Turn 时，Core 只接受 Host 当前发布且可用的模型能力，并把上述稳定引用、实际窗口和
Catalog 版本冻结进 Turn Context。选择变化只影响下一 Turn；运行中的模型断开通过规范 Host
事件结束或恢复当前 Turn。客户端可以缓存启动进度用于动画，但不得用缓存覆盖 Host 事实。

Apple 平台的 Qwen3.5 0.8B/4B 文本聊天使用 `mlx-swift-lm` 的
`LLMModelFactory`。模型仓库虽然包含视觉配置，但无图文本请求不得进入
`VLMModelFactory` 的多模态 MRoPE 路径。当前目录向用户展示至少 64K 的可配置上下文，
同时保留模型声明的 262K 原生窗口信息；0.8B 建议至少 8GB 统一内存，4B 建议至少
16GB。参数配置与运行生命周期分离，包含 temperature、top-p、top-k、重复惩罚、最大
输出和上下文预算。

真实 macOS 验收必须覆盖完整权重、显式启动、带 Bearer 鉴权的 `/v1/models`、
`/v1/chat/completions` 实际生成以及停止。静态目录测试或跳过的模型测试不能替代该验收。
iOS 复用相同 LLM 工厂契约。Android 的自定义 OpenAI-compatible API 属于 `Cloud` 来源，
端侧 GGUF 才属于 `Local` 来源；两者没有别名、迁移读取或隐式回退。Android 使用固定上游
revision 的 `llama.cpp` Android binding，CPU backend 静态链接进应用。目录保存 artifact
路径、发布仓库、SHA-256、字节数、上下文窗口和建议内存；下载写入 `.partial`，与发布方 LFS
SHA-256 一致后才原子安装。选择、下载、启动、停止和删除是独立动作，后台 Agent 在没有明确
激活模型时直接失败，不能自行读取 API 配置启动模型。

Android 目录当前提供 Qwen3.5 4B Q4_K_M（约 2.74GB，建议 8GB RAM）和 Qwen3 0.6B Q4_0
（约 429MB，建议 2GB RAM），二者声明 64K 上下文上限。启动时将 temperature、top-p、
top-k、最大输出和上下文预算下发到 native context/sampler；native 停止位置必须严格遵守
最大输出，context shift 使用实际 context 而非编译期常量。

### 桌面响应式页面

macOS 二级页面使用统一桌面断点。可用宽度达到 760pt 时使用可调整双栏；更窄时转换为
“上方选择列表、下方详情”，而不是继续压缩 sidebar 和 detail。日志、代码和缓存内容在
窄宽下必须在页面内换行，不能依赖超出窗口的固定最小宽度。视觉端到端测试必须同时渲染
常规和紧凑窗口，并人工检查附件；只断言 PNG 非空不代表布局通过。

响应式收缩必须保持任务主线，而不是等比例挤压全部控件。紧凑聊天标题栏只保留侧栏入口、
当前模型和会话标题，执行后端、Host、工作区、搜索与检查器收进一个仍可访问的 overflow
菜单；标题可截断，模型入口不得压缩到不可见。需要两个各自最小宽度的工作区（例如模型测试
的设置与输出）在 820pt 以下切换为单栏分段导航，开始执行后自动展示输出。设置等垂直侧栏在
高度不足时必须可滚动，并在选择变化后把当前项滚入可视区域；空间充足时仍维持系统项靠底。

CLI Web 在 macOS/Linux 上由同一个 `nola` 二进制提供，并使用令牌会话、HttpOnly
Cookie、SameSite=Strict 与 CSRF 契约。Windows 目标也应使用同一 Rust Web host；在
非 Windows 主机交叉检查时，缺少 MinGW 或 MSVC Windows SDK/C 标准库只能记为构建
环境阻塞，不能声称 Windows 原生构建通过，也不能把第三方 `ring` C 编译失败归因于
Nola 源码。CLI/Web 的用户状态根目录由共享平台路径契约解析：`NOLA_HOME` 优先，其次
Unix/macOS 的 `HOME`，Windows 使用 `USERPROFILE`，最后回退到 `HOMEDRIVE + HOMEPATH`。
会话数据库、Skill、数据源、配置和 Web 登录令牌不得各自直接假设 `HOME` 存在。

### 工具轮后的最终正文

工具执行完毕且验证、恢复、权限等 Core 义务均已清除时，模型返回的首个非空、无工具调用
的新正文就是该 Turn 的最终正文。Core 必须完成 Turn，不得为了“再润色一次”默认发起第二次
tool-free synthesis。只有当前正文与该工具轮已经记录的临时正文完全相同，或仍包含嵌入式
工具协议时，才视为 provider replay 并请求重写；验证或恢复义务未清除时仍必须继续保留工具
能力。该边界由 Core host contract、canonical Part lifecycle fixture 和 CLI host-loop E2E
共同约束，防止重复回答、正文延迟、气泡重置以及 Host/客户端双重收尾。

这里的完成仅指 Turn 协议结束，不自动证明自然语言任务目标已经达成。已登记的验证、
恢复与 Skill 义务不能被输出超限后的重复续写绕过；无进展采用有界恢复，耗尽后明确失败。
不得以扫描用户动词、强制工具或重复请求聊天授权来代替具体调用与结果证据。

### 2.1 Core / Host 规范边界

Core / Host 的规范协议、职责矩阵、Message Part、能力目录、权限、压缩与恢复设计见
[`CORE_HOST_ARCHITECTURE.md`](../../软件架构设计/总体设计/CORE_HOST_ARCHITECTURE.md)。该文档是 Agent Runtime
分层的权威来源。

当前仓库使用 `AgentCore` 作为对话唯一公开 Kernel，私有 `AgentRuntime` 仅负责确定性归约。CLI 与 `nola-agent-host` 复用同一
Core Host loop；iOS 和 Android 的 DeviceAgentCoordinator 使用 v2
Submission/Event/HostRequest 协议。旧 `core/src/agent_runtime.rs`、v1 C ABI、
JNI 与 Swift runtime 已删除，不存在兼容回退。平台仍可保留网络和系统 API adapter，
但不得在 adapter 中维护第二份 Turn、工具、权限、消息排序或上下文压缩状态。
Host effect 以 effect/request identity、Host lease、Session/Turn owner、Catalog
revision、availability epoch 和单调 sequence 完成归属校验；取消同样走规范
HostRequest/HostResponse，不存在 driver 私有旁路。

Core v2 的产品级变更入口是 `AgentSubmission`，不是 reducer 内部的
`AgentCommand`。CLI、Web、macOS、iOS 和 Android 只提交用户输入、权限答复、
Host 回包、Catalog 更新、断连、恢复、取消、压缩或重置；模型完成、工具完成等
生命周期变化由 Core 根据带关联 ID 的 Host response 归约产生。

Web 普通 Turn 提交类型化历史、当前输入和原始上下文组件；不得在进入 Core 前预摘要。
显式 `/compact` 也提交 Core `Compact` submission，与自动压缩共享同一状态机。
无活动 Turn 时压缩只更新 Session 上下文，不触发一次新的模型生成。桌面 Host 的
上下文在请求入口转换为 `ContextComponent`，运行时不存在 `host_context_messages`
注入旁路。

Core checkpoint 通过内部 `journal.checkpoint` effect 交给 Host，再由 Application 写入
Nola Store。checkpoint 只保存活动 Run 的 reducer 恢复状态、revision、event cursor、
压缩工作集和待恢复的外部 effect；它不是完整会话正文的第二份持久事实源。内部
`journal.checkpoint` effect 必须从 checkpoint projection 中剔除，否则下一次快照会把
上一次快照作为自身请求载荷递归嵌入并指数膨胀。Transition 已产生安全 checkpoint 时
Application 直接提交该版本；不产生 checkpoint 的流式增量由 Application 调用 Core 的
安全 checkpoint projection 兜底，不允许在 Application 或客户端解析 JSON 删除字段。
Run 终止后，新的
Runtime 从 Nola Store 的 Message/Part transcript 重建，不能把上一个终态 checkpoint
恢复成下一轮输入。CLI、Web 与原生客户端共享同一合同，不再维护各自的 checkpoint 文件。

编辑、回退和分支不得改写或删除消息账本。回退由 Core 产生新的 checkpoint，由
Application 将 checkpoint 与 Message 可见性投影放入同一个 SQLite `BEGIN IMMEDIATE`
事务提交；任一 revision、消息归属或 JSON 校验失败时，两者都不得前进。投影使用稳定
`operation_id` 幂等重放，重启后界面只读取当前可见投影，审计、恢复和分支仍可读取完整
不可变 Message/Part 历史。客户端不得先提交 checkpoint、再单独隐藏消息。

后台任务的状态、工具进度和呼吸状态属于可重建投影。Host 可以高频更新 Task read model
和 Trace，但不得为每个 UI progress event 向父会话 Core 反写一次领域命令；只有领取、
等待权限、权限恢复、完成、失败和取消等生命周期事实进入 Core。该边界避免展示刷新触发
checkpoint 写放大，也保证客户端关闭后仍可由 Store、Run Event 和 Trace 重建界面。

iOS 设备工具在 Core tool plan 绑定后直接进入类型化 Host executor，不再把已验证请求
往返转换为 `host_call_requested` UI 事件。Android v2 coordinator 同样直接构造
Host call request；旧事件 parser 不属于主运行路径。平台 UI 只能消费 Core Trace，
不得以这些旧事件重建工具、权限或消息状态。

macOS 远端 Agent 请求将用户消息放入 `messages`，把会话级来源事实放入
`context_components`。Web Host 不再把 macOS 上下文当作 system chat history；
组件经 Core Context Engine 清洗和预算后只冻结到当前 Turn。

Core v2 的恢复 ABI 支持从活动 Run checkpoint 重建 Session，并返回按 replay safety
筛选后的 pending Host requests。运行中的 Session 同时持有从 Store 投影的 `transcript`
和可压缩的 `model_context`：前者的持久权威是 Store，后者只保存下一次采样所需视图。每个 Turn 在
创建时冻结模型、工具 schema、上下文窗口和 Host 目录版本为 `TurnContext`，运行中的
配置变化不能污染当前 Turn。Host Catalog 可以在运行时升级，并通过 lease/heartbeat
维护可用性；Host 超时由 Host 转换为 `host_disconnected` 提交给 Core。工具输出以
模型预览、完整输出引用和 artifact 三层保存，大输出不会直接占满模型上下文。

Prompt 同样属于 Core 所有权。平台只提交类型化片段：稳定平台事实进入 `HostFact`，
用户选择的回复风格、模板和自定义规则进入 `UserInstruction`，记忆、文档、附件、Web、
MCP 与平台数据进入不可信 `Reference`。Core 统一渲染显式信任边界和稳定前缀摘要；
iOS、Android、Web、CLI 与 macOS Host 不得重新拼接一份平台私有 system prompt，也不得
把检索内容、工具输出或用户模板放入原始 `System` 通道。

每个 Turn 冻结类型化 MCP 目录及 revision。Core 将具体 MCP tools 投影为精确工具能力，
同时最多投影一个 `mcp_resources_read` 和一个 `mcp_prompts_get` 聚合能力；请求必须以
`server_id + resource_id` 或 `server_id + prompt_name` 精确解析，不能根据用户文本、名称
前缀或传输地址猜测。Host 保存连接与凭据，并按目录声明的 Streamable HTTP/stdio 传输执行。
`list_changed` 只刷新下一 Turn 的目录，不得改变正在运行的 Turn。

代码工作区额外提供 Core 所有的 `CodeIntelligenceOperation` 契约。Core 使用稳定的
`codebase_*` capability ID 表达索引、架构、符号/文本检索、调用链、源码片段与
覆盖检查；它不拥有数据库、文件监听或 MCP 进程。CLI、Rust Web Host 与
macOS `nola-agent-host` 共享同一个 Host adapter，直接调用编译进 Host 的 Rust Code
Memory 引擎，将工具契约映射到稳定 capability。当前只注册引擎已实现的 11 个操作；
图查询、变更影响、ADR 和 trace ingestion 在原生语义完成前不进入模型目录。第三方
工具仍使用普通 server-qualified MCP ID，禁止依据用户文字、语言或关键词改写路由。
引擎缓存位于 Nola 应用数据目录，工作区通过 canonical path 隔离 retained connection；
普通启动不运行上游 installer，也不修改其他客户端配置。若发行包未携带该引擎，Host
保留现有 repo map、symbol search、call graph 与文件工具作为降级路径，不得阻塞聊天。
代码索引按内容摘要增量更新，并在同一个 SQLite transaction 中发布新 generation；取消或
失败保留上一代完整索引。查询先核对文件集合的 metadata，对实际返回的文件再校验内容摘要，
跨进程搜索缓存同时使用 generation 与 SQLite `data_version` 失效。解析覆盖不完整会随
coverage 返回，不能被当作完整仓库事实。能力选择只依赖冻结目录和 typed provider 错误，
不得分析用户文本或用语言关键词决定路由。
Rust 引擎随 `nola` 和 `nola-agent-host` 一起静态编译，不下载、查找或启动 Code Memory
companion，也不依赖用户 `PATH`。CLI、Rust Web、macOS 与 Windows 使用同一 Host provider；
移动端不注册这组能力。Harbor 只上传 Nola runtime，并走相同的原生工具路径，不提供
benchmark 私有的 Code Memory 注入参数。Bazel 目标只构建共享 Core/移动端模块，不承担
CLI、Rust Web 或桌面应用的发行打包。
引擎静态链接 66 个语言和工程格式 grammar；其中 18 个语言具备显式声明和作用域符号提取 profile，
其余语言先提供文件发现、解析覆盖和原文检索。`get_graph_schema` 返回两个实际注册表，
Host 和模型不得把 grammar 可用性改写为未实现的完整调用图语义。
编程语言集合按 GitHub Innovation Graph 2026 Q1 聚合活动设置 0.1% 使用率下限；JSON、YAML、
TOML、Markdown、Dockerfile、CMake、SQL 等工程格式独立保留，不套用编程语言分母。
索引扫描只为当前项目实际出现的语言按 worker 延迟初始化并复用 parser；文件读取、摘要和
解析使用有界并发，最终 SQLite generation 仍由单线程 transaction 原子发布。

当前技术栈：

1. 唯一后端：Cloudflare Worker + D1；本地开发使用 Wrangler + 本地 D1
2. Web：Rust CLI Web 后端 + Jinja 模板/静态资源 + 原生 JavaScript
3. iOS：SwiftUI + ServiceContainer 依赖注入体系，通过 `NolaCore.xcodeproj` 产出的静态 `NolaCore.framework` 复用 Agent 状态机、权限分类和上下文规划
4. Android：Compose + Hilt，通过薄 JNI `.so` 链接同一份 Rust `libnola_core.a`
5. macOS：SwiftUI + AppKit 原生 UI；本地与远端执行均通过 Agent Host 协议接入，不以 CLI UI 作为运行时
6. 共享 Agent Core：Rust functional core，所有 Host 使用同一
   Submission/Event/HostRequest、TurnContext、工具事务、checkpoint 和上下文语义

## 3. 仓库结构（关键目录）

1. backend/
   - app/main.py：后端入口与路由挂载
   - app/purchases.py：订阅、验单、tier、usage
   - app/push.py：设备注册、sources 同步、run_now、结果查询、调试接口
   - app/db.py：SQLModel 模型与数据库读写函数
   - app/region_routing.py：跨区域转发策略

2. web/
   - app/templates/*：登录、仪表板、定时任务、AI 对话、执行记录等页面
   - app/static/*：CSS、JavaScript、图标与本地 vendor 资源
   - native_browser/、native_app/：OS/browser native messaging host 辅助脚本

3. cli/
   - src/web.rs：Web 启动、模板/静态资源服务与本地优先 API
   - src/config.rs、src/main.rs：`nola web` 命令入口

4. Nola/
   - iOS 应用代码（UI、服务、模型、通知、LLM 路由等）
   - 通过 `NolaCore.xcodeproj` 的静态 framework 产品接入共享 Rust Core；生成物不提交仓库，App target 不直接调用 Cargo

5. NolaMac/
   - 原生 macOS Chat、会话归档、附件解析、工作区、模型与宿主机服务
   - macOS 正常 Chat 不依赖或启动 CLI 子进程

6. android/
   - Compose/Hilt 客户端、Health Connect、平台权限与设备级 UI 测试
   - 通过 JNI `.so` 链接各 ABI 的 `libnola_core.a`

7. core/
   - 共享 Agent 状态机、权限与 ContextPlan
   - `scripts/build_xcframework.sh` 与 `scripts/build_android.sh` 生成平台产物

8. Doc/
   - REQUIREMENTS.md：产品需求
   - SOFTWARE_DESIGN.md：本设计文档
   - AGENT_TRACE_ARCHITECTURE.md：Agent Trace 唯一协议、Journal、reducer 与呈现不变量
   - CORE_HOST_ARCHITECTURE.md：Core/Host 权威所有权、effect、权限、压缩与恢复契约
   - CORE_HOST_LAYERING.md：四个参考项目提炼出的分层规则与不变量
   - AGENT_CORE_REWRITE.md：单一 AgentCore Kernel、私有 reducer 重写决策与旧路径删除约束
   - CORE_HOST_REFERENCE_SYNTHESIS.md：参考实现取舍、Core/Host 目标分层与迁移门禁
   - PROMPT_UI_STREAMING_SOP.md：通用流式对话与增量 Markdown 工程规范
   - TESTING.md：测试证据等级、真实边界与发布门禁
   - tool-activity-presentation.md：工具与长任务过程的气泡内呈现契约
   - CHANGELOG.md：高层变更记录

## 3.1 数据与文档边界

1. 附件在客户端本地解析后作为引用上下文注入，附件文本本身不能触发工具或被当作用户命令。
2. PDF 与 Office 文档解析失败必须显示明确错误，不能回退成无关的提醒或自动化意图。
3. DOCX 提取覆盖正文、表格、页眉、页脚、脚注和尾注；扫描型 PDF 是否可读取取决于可用文本层/OCR 能力。
4. iOS HealthKit 读取授权对应用并不完全透明，因此 UI 以直接读取结果为事实来源，而不是仅依赖 share authorization 状态。
5. iOS Data Tab 与健康详情共享同一份 dashboard snapshot，并响应 HealthKit observer 变化；macOS 只消费已同步的 iPhone 健康摘要。

## 4. 核心领域模型

后端数据库核心实体：

1. PurchaseRecord
   - 用途：订阅身份与等级
   - 关键字段：uuid、product_id、tier、is_active、expires_date
   - 有效 tier 必须同时满足 `is_active=true` 且 `expires_date` 未过期；过期或停用记录在客户端与网关中都按 `free` 处理。

2. UsageRecord
   - 用途：按月统计 token/request 消耗
   - 关键字段：client_uuid、period_start、tokens_used、requests_count、prompt_tokens、completion_tokens、total_tokens、monthly_limit
   - `client_uuid` 统一以 UUID hex（无连字符、小写）存储，避免同一用户因 hyphen/hex 混用产生两份用量记录。
   - `(client_uuid, period_start)` 是唯一键；启动时会合并旧库中同周期重复记录，用量递增通过数据库 upsert 原子累加。

3. UserIdentityRecord
   - 用途：Apple 用户标识与 client_uuid 绑定
   - 关键字段：client_uuid、apple_user_id、home_region

4. ScheduledSourceRecord
   - 用途：URL 支持的定时自动化任务配置
   - 关键字段：source_id、url、schedule_frequency、scheduled_time_iso、timezone、workflow_prompt、llm_model、skills_json

本地提醒不写入 ScheduledSourceRecord / URLDataSource；提醒由客户端 `UserTaskService`
保存为本地任务并交给本地通知系统调度，避免和 URL 数据源、周期抓取任务混用。
后端 `/v1/sources/sync` 仅接受可由服务端执行的网页/搜索等自动化源；Web
`/api/scheduled-actions` 还接受 `format=file` / `format=folder` 的本地数据源任务，
但这类任务只存入 Web 本地 `local_scheduled_actions`，读取路径必须位于
`LocalUserAgentSettings.allowed_dirs`，不会同步到后端。两端都必须拒绝
提醒与自动化使用不同的当前数据模型，提醒只由 `UserTaskService` 管理。

脚本自动化是第三类定时动作，但只属于 Web 宿主机本地能力：

1. 存储在 Web 本地 `local_scheduled_actions(format=script)`，脚本元数据以 JSON 存入 `user_prompt`
2. 不写入 `ScheduledSourceRecord`，后端必须拒绝 `format=script`
3. LLM 生成或 Agent 创建的脚本默认 `enabled=false` 且没有 `approvedAt`
4. 用户在 Web 界面审查并确认后，`approvedAt` 写入元数据且任务才可启用
5. 运行时读取 `LocalUserAgentSettings`：必须开启本地开发者工具，cwd 必须位于 allowed_dirs，脚本每条命令必须通过 bash prefix 白名单
6. 执行结果写入 `local_execution_results`，events 中保存 `kind=script_run`、cwd、exitCode、stdout、stderr、approvedAt 与 lineCount

5. ScheduledJobRecord / ScheduledResultRecord
   - 用途：执行任务与结果存储

## 5. 鉴权与登录设计

### 5.1 后端鉴权

后端存在两类令牌：

1. Access Token（Bearer）
    - 用于 /v1/sources/sync 等用户写操作
    - 由 SERVER_JWT_SECRET 签名

2. Debug Token
   - 用于 /v1/debug/* 运维/调试接口

### 5.2 Web 登录流程

Web 登录流程已实现为本地会话 + 官方 OAuth 链接模式：

1. 用户访问 /login
2. 用户可选择 Apple、Google、GitHub 登录官方后端；Rust CLI Web 后端以本机状态文件保存本地会话/配置
3. 用户也可提交本地邮箱 + 密码（仅本机校验，不上传后端），用于离线/本地缓存账户
4. 若该本地用户已完成 OAuth 后端链接，登录后直接从本地 `LocalUser.backend_access_token` / `backend_user_id` 恢复联网会话，不阻塞调用后端刷新 token
5. 受保护页面（/dashboard、/scheduled-actions、/results、/settings）要求登录
6. 若要使用跨端同步/联网能力，用户需在 `/link-backend` 选择 Apple、Google 或 GitHub 完成显式绑定；access token 仅在该流程及后续联网 API 调用中使用

注意：

1. 登录页已移除 token 直接登录入口
2. 官方后端已禁用 Nola 自建邮箱密码鉴权；`/v1/auth/register`、`/v1/auth/login` 仅返回不再支持
3. 已链接用户在本地登录成功后会自动恢复 access token（使用 OAuth 链接时保存的本地凭据，不使用当次登录密码联网换取）
4. access token 仅用于后端已链接会话的联网调用
5. 定时任务、执行记录与 AI 对话均采用本地优先：本地读写始终可用；若已链接后端则额外尝试云端增强
6. 订阅校验仅用于云端 LLM 调用链路（本地回退模式不依赖订阅）

## 6. 定时动作（Scheduled Actions）设计

### 6.1 读链路

1. Web 页面请求 /api/scheduled-actions
2. Web 优先读取本地 `local_scheduled_actions`，并关联本地 `local_execution_results` 形成最新状态
3. 若已链接后端，相关写操作会额外尝试同步到后端以提升跨端体验

### 6.2 写链路

1. 用户在 AI 对话中输入自然语言
2. 客户端解析为三类对象：本地提醒写入 `UserTaskService`，URL/搜索周期任务写入 URL-backed scheduled action，宿主机脚本写入 Web 本地 script action
3. URL 周期任务在已链接后端时可组装 sources/sync payload 并同步到 /v1/sources/sync
4. 后端写入 ScheduledSourceRecord，仅代表 URL 支持的周期自动化任务；脚本与提醒都不得同步到后端
5. URL 周期任务后续由调度器执行或 run_now 执行；本地提醒由本地通知系统执行；脚本自动化由 Web 宿主机在本地沙箱/白名单约束下执行

### 6.3 已修复的关键问题

1. db.py 中 upsert_sources 参数名 timezone 遮蔽 datetime.timezone，导致 500
2. 已通过重写时间获取实现修复（使用 dt_timezone 别名）
3. 本地提醒只进入 `UserTaskService`；`URLDataSource` 不接受提醒类型或提醒 URI
4. Web/后端曾允许 reminder 形态进入 scheduled sources；现同步与创建入口会拒绝该形态，避免重新污染自动化任务表
5. 脚本自动化不能当作后端 scheduled source：服务端不具备用户宿主机权限，也不应运行用户/LLM 生成脚本；只能在用户本机 Web 进程中执行，并保留审批、白名单和日志

## 7. 后端接口分层

### 7.1 健康与运维

1. GET /healthz
2. GET /admin/outbox/dlq/count
3. POST /admin/outbox/dlq/replay

### 7.2 购买与用量

1. GET /purchases/pricing/catalog
2. POST /purchases/verify_purchase
3. GET /purchases/tier/{uuid}
   - 返回有效 `tier`、原始/上报档位 `reported_tier`、官方模型权限、官方月度 token 限额、订阅是否有效与到期时间。
   - 停用或过期订阅必须降级为 `free`，不能只凭 `tier=subscription` 放行。
4. GET /purchases/usage/status/{client_uuid}
   - 只读接口，不因查询状态创建 `UsageRecord`。
   - 返回 tokens_used、monthly_limit、remaining_tokens、usage_percentage、requests_count、prompt_tokens、completion_tokens、total_tokens、period_start、last_updated。
5. POST /purchases/usage/consume
   - 写入接口；`token_cost` 与 `requests` 必须为正数。
   - 返回 `allowed=false` + `reason=quota_exceeded` 时不写入消耗。
6. GET /purchases/stripe/products
   - 返回可在 Web 订阅页展示的按月 Stripe 产品列表（product_id、display_price、currency、period）。
7. POST /purchases/stripe/checkout
   - 需 Bearer token；创建 Stripe Checkout Session（mode=subscription），并在 metadata 中写入 client_uuid/product_id。
8. POST /purchases/stripe/checkout/verify
   - 需 Bearer token + `session_id`；后端向 Stripe 拉取 checkout/subscription 做服务端校验，通过后 upsert `PurchaseRecord`。
9. POST /purchases/stripe/webhook
   - Stripe 回调入口；验证签名后根据订阅事件更新 `PurchaseRecord`（active/trialing/deleted）。

### 7.3 Push 与定时动作

1. POST /v1/devices/register
2. POST /v1/sources/sync
3. DELETE /v1/sources/{source_id}
4. POST /v1/sources/{source_id}/run_now
5. GET /v1/results/{result_id}
6. POST /v1/results/{result_id}/ack
7. GET /v1/notifications
   - 返回当前用户官方通知列表、read/unread 状态与 unreadCount；通知由后端调度/推送路径写入 `NotificationRecord`
8. GET /v1/notifications/unread_count
9. POST /v1/notifications/{notification_id_or_dedupe_key}/read
10. POST /v1/notifications/read_all
11. GET /v1/debug/push/client/{client_uuid}

`POST /v1/sources/sync` 的 source 项使用 `llmModel`、`workflowPrompt`、`skills` 当前字段；后端会在写入时校验技能目录并持久化到 `scheduledsourcerecord`。执行链路使用 source 自定义模型与工作流提示，`skills` 先执行并把输出注入 LLM 上下文。

### 7.4 LLM 与模型

1. POST /v1/chat/completions（支持 stream=true，SSE 流式输出）
2. GET /v1/models
3. 访问要求：Bearer token + X-Client-UUID header + subscription 订阅身份
4. 模型：当前后端默认转发至 Dashscope（Qwen 系列），响应中含 reasoning_content 字段（思考链）
5. 官方后端 LLM 网关透明转发模型请求，不重写用户 query、不自动检索、不注入隐藏摘要。联网检索只能来自模型提交的 `web_search` 类型化工具调用，由 Core 校验当前 Turn 的 Catalog、参数、预算和权限后交给 Host 执行。
6. 官方后端只提供 `POST /v1/chat/completions` 模型中转（`stream=true` 时透传上游模型 SSE）。它不是 Agent Host，不生成 Agent 事件，也不提供 Agent 工具。
7. 官方后端 Agent 设置持久化在 `AgentSettingRecord`，权限规则持久化在 `AgentPermissionRuleRecord`：
   - `GET/POST /v1/agent/settings`
   - `GET/POST/DELETE /v1/agent/permission_rules`
   - `GET /v1/agent/tools`
8. 官方后端提供订阅别名接口 `/v1/subscribe/products`、`/v1/subscribe/checkout`、`/v1/subscribe/verify`，内部复用 `/purchases/stripe/*` 的服务端校验与 `PurchaseRecord` upsert 逻辑。

### 7.5 Mobile Host API

官方 Worker 的移动账户/附件接口使用 `/api/mobile/*`；Agent 执行只连接用户配对的 `nola web` Agent Host：

1. `POST /api/mobile/login`
   - 官方后端不再接受邮箱/密码登录，返回 410
   - iOS/移动端应使用 Apple、Google 或 GitHub 登录拿到的 Nola JWT 作为后续 bridge token
2. `GET /api/mobile/me`
   - 校验 bearer token 并返回 local/user identity
3. `GET /api/mobile/agent/settings`
   - 返回 host-owned Agent settings 的只读子集
   - 响应使用 bearer token 派生 AES-256-GCM envelope
4. `POST /api/mobile/attachments/parse`
   - 请求必须带 `X-Nola-Mobile-Crypto: aes-gcm-v1`
   - 请求体使用 bearer token 派生 AES-256-GCM 解密；当前后端提供受大小限制的纯文本解析器
5. Worker 不提供 `/api/mobile/chat/agent/v2/stream`。该路径只存在于配对的 Agent Host，请求体和 SSE 事件使用 AES-256-GCM envelope，事件必须是 canonical Core Part/Trace；旧 `text` / `reasoning` / `done` 帧属于协议错误。

移动桥接加密逻辑位于 Worker，共享 Rust Core 端实现位于 `core/src/mobile_crypto.rs`，两端保持协议一致：

- AAD: `nola-mobile-bridge-v1`
- Algorithm: AES-256-GCM
- Key: `SHA256(access_token)`
- Envelope: `v` / `alg` / `nonce` / `ciphertext` / `tag`

## 8. 配置与环境策略

### 8.1 后端关键环境变量

后端只从 `nola_backend_env` 读取配置；该变量必须是 JSON object。启动时
`app.env_config.load_backend_env()` 会把 JSON key/value 注入进进程环境，后续模块统一
通过 `os.getenv` 读取。`backend/.env` 不再作为配置源。

关键 JSON key：

1. `DATABASE_URL`
2. `TARGET_OPENAI_API_BASE` / `TARGET_OPENAI_API_KEY`
3. `SERVER_JWT_SECRET` / `SERVER_JWT_ISSUER`
4. `PUSH_FEATURE_ENABLED` / `PUSH_SCHEDULER_ENABLED`
5. `REGION_ID` / `REGION_BASE_CN` / `REGION_BASE_GLOBAL`
6. `OAUTH_RELAY_PUBLIC_BASE_URL` / `APPLE_SIGNIN_CLIENT_ID` / `GOOGLE_OAUTH_CLIENT_ID` / `GITHUB_OAUTH_CLIENT_ID`
7. `SERPER_API_KEY` / `SERPER_API_URL`（显式 `web_search` Host 工具使用；LLM 网关不读取）

### 8.2 本地联调推荐配置

为保证本地可用，建议：

1. `nola_backend_env.DATABASE_URL` 使用 SQLite 文件
2. 关闭 outbox 同步与跨区复杂行为
3. 按需开启 PUSH_FEATURE_ENABLED，调度器可先关闭

示例原则：

1. 后端可启动优先
2. 数据可落库优先
3. 写链路通优先

### 8.3 Cloudflare Worker + D1 后端

后端仅运行 Cloudflare Worker，并使用 D1
`DATABASE_URL`。Worker 版本位于 `backend/worker/`，通过 `DB` D1 binding 持久化：

1. `backend/worker/src/index.ts`：Worker HTTP 入口，覆盖健康检查、OAuth Relay、
   provider auth、购买/用量、模型/聊天代理、聊天历史、设备、定时数据源、结果查询，
   并补齐了 `/api/mobile/*`、`/v1/agent/*`、`/v1/debug/*`、`/admin/outbox/dlq/*`、
   `/search/serper` 与 Stripe 购买路由。
2. `backend/worker/migrations/0001_current_schema.sql`：唯一 D1 schema，表名与当前 SQLModel
   表一致（如 `purchaserecord`、`usagerecord`、`scheduledsourcerecord`、
   `agentpermissionrulerecord`）。Worker 响应需保持 iOS/Web 代理契约：支持
   `x-nola-access-token`，结果详情包含 source/failure/quota 字段，Agent settings
   GET 只返回 `has_agent_llm_api_key`，不回传明文 key；mobile bridge 的加密响应
   必须带 `type: "encrypted"` 以便 iOS 解密。
3. `backend/worker/wrangler.toml`：D1 binding 名称固定为 `DB`。
4. Worker 仍读取 `nola_backend_env` JSON（可配置为 Cloudflare JSON variable/secret），
   但 `DATABASE_URL` 对 Worker 无效；D1 binding 才是持久化来源。

Cloudflare Worker 是请求驱动运行时，不能使用 Python 后端的后台 scheduler loop。
定时执行后续应迁移到 Worker Cron Triggers；当前 Worker 已支持 `run_now` 通过
`ctx.waitUntil()` 异步抓取、摘要并写入 D1 结果；成功和失败都会持久化为
`ScheduledResultRecord`，并在配置 APNs 时向已注册设备发送 resultId 通知。

## 9. Web 设计原则

CLI、Web 与原生客户端的用户 Turn 统一由 Agent Core 驱动。Web 历史以 typed
`AgentItem` 导入，新输入作为当前 Turn 增量提交；Host 不再把整段历史扁平化为用户
prompt，也不再生成客户端业务状态。Core Part 与 Application `ConversationProjection` 是
正文、思考摘要、工具、来源、Artifact 和上下文用量的唯一产品语义来源。Diagnostic Trace
只记录进度与时序；Host 仅附加 correlation、lease、challenge 等传输元数据。

Web completion 与流式 Agent 请求直接提交原始历史、当前输入和
`ContextComponent`。普通用户 Turn 不再预调用 Web 私有的 LLM 上下文摘要器；
组件准备、真实 token 计量和阈值压缩由 Core 完成，避免双摘要、重复持久化和额外
首 token 延迟。

Nola 尚未发布，不维护旧版内部数据或传输协议。Core 内部事件使用嵌套事件结构，
Agent Host v2 只输出扁平化、带 `protocol_version: 2` 与明确 `type` 的 Host DTO；客户端
不得交叉解码。Web 本地存储只接受 camelCase 会话字段，资源结构只接受当前正式字段：
Skill 使用 `id/name/description/category_id/content`，MCP 使用
`server_id/name/endpoint`，Document 使用 `id/name/content` 及可选 `uri/mime/size`。
旧字段、旧路径和旧 schema 直接拒绝，不提供迁移、双写、读取回退或静默默认值。

1. Web 是后端能力的可视化代理层，不是独立业务后端
2. Web 读写必须走真实后端，不允许长期 mock 依赖
3. 会话态必须与页面权限一致（页面可见性、API 行为一致）
4. 登录态和错误提示要可解释（例如 token 缺失、后端 5xx）

### 9.1 Web Agent v2（Claude Code 风格的工具调用循环，2026-04-27）

Web 端统一使用 `/api/chat/agent/v2/stream`。这是一个借鉴 Claude Code（`参考资料/claude-code` 中的
`Tool` / `QueryEngine` / `query.ts queryLoop` / `WebFetchTool` 等模块）思路、
由 Rust CLI Web Host 承载的多轮工具调用 Agent。Agent 模式开关只控制当前 Turn
是否开放工具与 Agent 策略，不切换执行协议或消息写入方。浏览器不直接创建 User /
Assistant 消息；Agent Host 使用客户端提供的权威消息 ID 原子提交 Turn。

- 实现位置：`cli/src/web.rs`、`cli/src/agent.rs`、`cli/src/tools/mod.rs`
- LLM 路径：Web 进程**直接**调用 OpenAI 兼容端点（不经后端代理），以拿到
  原生 `tool_calls` 协议；凭据**复用聊天页右上角「连接设置」**配置并保存在
  `LocalUserAgentSettings` 的本地 LLM
- 工具分类：
  - `web_safe`（默认可用）：`now`、`calc`、`web_fetch`（带 SSRF 防护）、
    `browser_status` / `browser_start` / `browser_snapshot` / `browser_navigate` / `browser_click` / `browser_type_text` / `browser_screenshot` / `browser_tabs` / `browser_stop`（统一语义工具；`target=managed` 走内置 Playwright 隔离 Chrome，`target=attached` 走用户 Chrome extension；快照返回带 revision 的稳定 element ref，操作后旧 ref 失效）、
    `computer_status` / `computer_screenshot` / `computer_get_position` / `computer_list_windows` / `computer_move_mouse` / `computer_click` / `computer_drag` / `computer_type_text` / `computer_press_key` / `computer_scroll` / `computer_activate_app`（通过 native app host 对宿主机桌面做确认后的截图、窗口读取、鼠标、键盘、滚动和应用激活）、
    `web_search`（DuckDuckGo HTML）、`list_scheduled_tasks` /
    `create_scheduled_task` / `create_search_task` / `create_local_data_source_task` /
    `create_reminder_task` /
    `delete_scheduled_task`、`get_task_history`、`update_agent_todos`、`get_agent_settings`、
    `propose_agent_plan`、`set_local_dev_tools_enabled`、`update_allowed_dirs`、`update_bash_prefixes`、
    `update_long_term_memory`、`update_user_preferences`
  - `local_dev`（默认禁用）：`read_file`、`write_file`、`edit_file`、`multi_edit_file`、`list_dir`、`glob`、
    `grep`、`disk_usage`、`bash`，必须在「设置 → 智能助理 (Agent)」中开启并配置
    允许目录；`bash` 还必须配置命令前缀白名单
- 聊天页新增 Agent 权限面板：支持直接点选本地开发者工具开关、编辑允许目录 /
  bash 前缀；自然语言表达授权变更时，LLM 通过上述 `web_safe` 设置工具更新
   `LocalUserAgentSettings`，前端在该轮结束后重新拉取并刷新面板状态
- 参考 Claude Code `TodoWriteTool` 增加 `update_agent_todos`：复杂多步骤任务可在单轮
  Agent 请求内维护 session-local checklist，要求最多一个 `in_progress`，并鼓励显式
  verification/test/self-check 步骤；该清单不写入长期记忆或后端同步
- 参考 Claude Code PlanMode / ExitPlanMode 的安全边界，但不引入完整终端状态机：
  `propose_agent_plan` 用现有 inline permission card 呈现计划审批。复杂、高影响或
  权限敏感任务在 Explore 后、Execute 前先发计划；未批准时 Agent 以
  `done(reason=permission_required)` 停止，且顺序执行分支会短路，避免同轮继续执行
  计划之后的工具。批准通过一次性 `permission_decisions` 生效，不写入永久规则。
- 参考 Claude Code `commandSemantics` 调整本地 bash 结果解释：`grep`/`rg` exit=1
  视为“无匹配”而非工具失败，`diff` exit=1 视为“文件有差异”，`find` exit=1
  视为部分目录不可访问；同时继续拒绝 `${}` / `$[]` 等 shell substitution 形态
- Agent 上下文拼装对 MCP/Skill/Document 采用去重与预算控制：按 endpoint 去重 MCP server，
  按 name+content 去重上传 Skill/Document，并限制注入数量与文本长度，避免 MCP 与 Skill
  互相挤占导致上下文失衡
- 所有聊天 Turn 都使用 Core Prompt Composer。共享 Prompt 文件只提供可版本化的
  内容片段；Web、iOS 或 Android 不得拼装第二份完整 system
  prompt。稳定政策、Host 事实、用户指令和 Reference 的顺序及信任边界由 Core 决定。
  已授权且新鲜的平台数据优先于 Skill 目录进入 Turn 预算；已启用数据源读取失败会明确
  终止上下文准备，禁止静默降级为无数据回答。
- Explore -> Plan -> Execute -> Verify -> Reflect 是可供模型采用的工作方法，不是客户端按
  关键词选择的模式。具体工具是否可用来自当前 Turn 冻结的能力 schema，平台边界也由 Host
  目录表达，而不是写入语言相关的排除词表。
- 三种回复风格以分段形式存放在 `Nola/SharedPrompts/nola_response_styles.txt`
  （`[professional]` / `[casual]` / `[playful]`）。Web 端由 Rust CLI Web 后端直接读取 Prompt 文件并在构建系统消息时注入对应
  风格段；普通聊天、自定义 LLM 和 Agent v2 都接收 `responseStyle` /
  `response_style` 并把当前风格写入系统提示，但不会把三种风格同时交给模型选择。
  默认风格为 `casual`（日常聊天），非法值会回退到默认值。Web 聊天页工具栏和
  `/models` 模型参数页共用 `LocalUserAgentSettings.response_style`。
- Web 聊天页模型选择参考 iOS `ModelsView` 的分区表达：`modelSelect` 包含
  “云端模型”与“本地模型”分组。云端模型来自 `/api/models`；本地模型来自统一
  `ModelProfile` 投影。两类模型都走
  Agent Host v2；Host 根据会话模型配置解析端点和凭据，API Key 不回填到前端。
- Web 会话级模型选择持久化在 canonical conversation settings 的
  `selected_model_source` 与 `selected_model_id`：`auto` 表示自动选择，`cloud` 存储
  具体云端模型名，`local` 表示已安装并显式启动的本地模型。`/api/chat/conversations`
  创建和 PATCH 均支持 `selectedModelSource` / `selectedModelId`；前端
  `modelSelect` change 后立即 PATCH 当前会话，加载会话或刷新页面时按该会话设置
  恢复选择，后续 Turn 均由 Agent Host v2 读取同一选择。
- Web 模型中心与聊天页共用账号级 `LocalUserAgentSettings` 保存本地
  OpenAI-compatible endpoint / API key / model 与默认 `response_style`；`GET/POST
  /api/agent/settings` 是权威读写路径。浏览器 `localStorage` 不作为模型配置或回复风格
  的配置来源；API Key 仅保存在本地 SQLite，GET 响应只返回 `has_agent_llm_api_key`。
- Web 聊天页从 pending 到 completed 始终复用同一个 assistant bubble、同一个
  Markdown renderer 和同一组稳定节点。正文通过“已提交语义块 + 可变尾块”增量解析，
  完整标题、段落、列表、表格和代码块在传输结束前即可结构化显示；完成态不得把 raw
  text 整体替换为另一套富文本。thinking、阶段、工具参数、文件或 URL、截断结果都由
  `AgentRunProjection` 更新气泡内的稳定 activity row。重复事件或只推进内部 cursor
  而不改变可见语义的事件不得触发 DOM mutation、布局动画或滚动。
- macOS、iOS、Android 与 Web 使用同义的交互动效合同：普通 disclosure 为 220ms，
  流式稳定块 reveal 为 180ms，可排队的视觉延迟不得超过 320ms。自动跟随只在用户仍位于
  底部且没有主动拖动时生效；上滚立即停止，回到底部后才恢复。Reduced Motion 下取消
  非必要位移/尺寸动画并缩短跟随校准。复制和时间等消息操作只在终态创建，但终态后必须
  对鼠标、键盘和触屏持续可达；hover/focus 只能改变强调度，不能控制命中或存在性。
- macOS 语义短语的渐显与静态显示必须共用同一个 `Text` / `Text.Layout`，只改变绘制透明度
  和短语偏移，不得隐藏正文后用第二个 overlay 文本重新排版。回归测试需检查动画中的
  实际文字像素边界与已显示内容，不能只断言节点身份或气泡的 fitting size。
  每条流式消息持有自己的解析器；完成时先接管订阅的显示任务，再提交未完成尾块。
  已提交节点的身份与内容不得因完成或相同正文的元数据刷新被重建。只有权威正文真正
  修订时才允许走显式替换路径，完成事件本身不是正文替换信号。终态可在同一气泡内把
  相同 canonical text 提升为 document-mode Markdown，以恢复跨流式短语边界的标题、列表、
  表格等结构；提升不得改写正文、重新触发 reveal，且必须通过完成前后布局回归测试。
  Mac 的表格语法由 `MacMarkdownTableSyntax` 统一拥有，流式组装器和 document parser
  不得各自维护分隔行判断。支持可选外侧竖线、转义竖线、单列表格及对齐标记；表头与
  分隔行列数必须一致。分隔行尚未结束时只缓冲，不把未确认的表格控制语法展示为正文。
  滚动跟随只由可见内容或视口变化触发，不由运行状态翻转触发；动画中的滚动不得被
  下一帧的无动画校准打断。同批新短语按顺序错开浮现，完成信号不截断尾段动画。
  长回答在气泡顶部提供复制入口，并提供只读全文选择视图以支持跨段落、代码块复制；
  此视图由用户显式打开，不替换聊天正文的渲染器。
- macOS 操作反馈遵循“就地状态优先、模态决定兜底”：保存、发现、同步等可恢复操作使用
  页面内 notice，至少区分 information / success / warning / failure；success 和 information
  可在四秒后平滑退场，warning 与 failure 保留到用户处理或关闭。系统 Alert / confirmation
  只用于必须立即决定、会丢失未保存内容或产生破坏性副作用的操作。notice 需同时提供图标、
  标题和可执行说明，颜色不能成为唯一状态信号。
- macOS 顶层页面使用稳定的系统 window canvas，不以大面积渐变承担层级。页面标题栏、指标条、
  panel、notice 和空状态通过间距、边框与语义色建立层级；会话侧栏重命名在原行内完成，并支持
  Return 保存、Escape 取消和失焦提交，避免为轻量编辑切换到系统 Alert。
- macOS 表单在主按钮不可用时显示首个可行动的校验原因，并在新建流程聚焦首个必填字段。
  隐藏或未启用的字段不参与校验；模型参数必须在保存前按运行时真实边界校验，不得保存时静默
  clamp。Store 的失败状态由页面 presenter 消费，成功重试应清除旧错误。
- API 模型的创建是一个完整配置事务，不能先插入无意义的空行再要求用户寻找另一个窗口。
  添加面板一次收集提供商目录/手动模型 ID、显示名、允许用途、独立的敏感数据授权、参数预设
  与高级参数；保存端点时同时提交新模型参数。预设只写入明确的 temperature / top-p 数值，
  用户改写数值后显示为 custom，不在运行时按模型名称猜测参数或能力。
- 自动上下文压缩属于用户可见的运行阶段。`phase=compacting` 在当前 assistant bubble 的过程区
  显示进行中状态；失败保留具体原因且说明未替换原上下文；完成后进行中状态退场并由持久化
  checkpoint 卡片接替，展示被归纳项数、摘要、fingerprint 和查看原始上下文入口。完整历史继续
  保留，客户端不得像不可见的内部优化一样静默处理压缩。
- 浏览器控制是一个工具契约、两个 Host adapter。模型调用统一的 `browser_*` 并显式提交
  `target=managed|attached`，不得由客户端按用户文字关键词重写。`managed` 由 macOS App 打包的
  Node + `playwright-core` 启动隔离的系统 Chrome profile，按 session 保持页面，并优先返回
  ARIA snapshot、可见文本和稳定 element ref；`attached` 用于用户当前/已登录 Chrome。当用户安装
  `web/native_browser/extension` Chrome 插件并把
  `chrome://extensions` 显示的真实 extension id 传给 `web/native_browser/install_host.sh`
  后，脚本会在 macOS 的 `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`
  写入 `com.nola.native_browser.json`，其 `path` 指向可执行 launcher
  `~/.nola/nola_native_browser_host_launcher.sh`。Launcher 用安装时解析出的绝对
  `python3` 路径启动 `web/native_browser/host/nola_native_browser_host.py`，避免 Chrome
  GUI 环境缺少 PATH 导致 host 立即退出。Native host 通过 token 保护的
  `/api/native-browser/host/poll` 与 `/api/native-browser/host/result` 长轮询领取命令并回传结果；
  attached adapter 操作用户真实 Chrome 标签页。旧 `chrome_native_*` 仅是 adapter 内部动作，
  不进入模型工具目录。
- macOS 对话启动只导出浏览器能力与 app-owned RPC 地址，不启动或等待 Node/Playwright。
  Host 在 Core 批准并实际执行 managed 浏览器工具时，才调用本地 broker 的
  `/api/native-browser/managed/rpc`；该端点验证 token 后由 App 按需启动并持有运行时。
  后续工具和跨轮对话复用同一运行时及 session，预览 UI 也使用同一个服务；不得回退为
  每轮 Host 各自启动浏览器进程。运行时故障在浏览器工具调用处返回可恢复错误，不阻塞普通对话。
- 通用本地应用和 computer-use 只允许通过版本化 Agent Host 协议回调签名客户端执行。
  Web 不维护 Python/AppleScript 常驻进程、第二命令队列或 `/api/native-app/*` 旁路。
- 浏览器工具安全确认：统一 `browser_*` 在执行账号、登录、密码、验证码、授权、
  支付或其他敏感信息相关操作前会返回 Core `HostRequest::Permission`。Host 在原 Turn 上展示
  内联确认卡片，并通过独立 HostResponse 通道提交决定；禁止重发用户消息或创建恢复 Turn。
- 聊天页采用 Claude Code 风格上下文管理内核（默认自动运行，不再要求用户手动操作）：
  - `LocalChatConversation.session_memory`
  - `LocalChatConversation.last_compacted_at`
  - `LocalChatMessage.include_in_context`
  - `LocalCompactedMemoryEntry`（compact 时保存 summary + raw_excerpt + archived message ids）
  - `LocalUserAgentSettings.long_term_memory`
  - `LocalUserAgentSettings.user_preferences`
  - `LocalUserAgentSettings.mcp_servers_json`
  - `LocalUserAgentSettings.uploaded_skills_json`
  - `LocalUserAgentSettings.uploaded_documents_json`
  - `/api/chat/conversations/{id}/context` 返回当前会话 context usage 估算，并附带
    `suggestionDetails`（按 high/medium/low/info 分级排序，包含 title/detail 与预计可回收 token）
  - 自动 compact：消息写入后若上下文使用率达到阈值（默认 80%）且活跃消息数量达到阈值，
    会自动将较早消息折叠到 session memory，并持久化 `LocalCompactedMemoryEntry` 原始快照；
    若当前用户配置了本地 OpenAI-compatible endpoint/key/model，则用该模型生成 compact 摘要，
    否则使用本地确定性摘要
  - 自动记忆检索：普通 Web 聊天、自定义 LLM 聊天与 Agent 运行时都会基于最新用户输入，在 session memory / compact 原始快照 /
    已折叠消息中自动检索相关片段，并与全局长期记忆、用户偏好一起注入当前轮上下文
  - Web 普通聊天与自定义 LLM 聊天不会由客户端按关键词主动沉淀记忆；系统提示只暴露隐藏
    Nola memory tool 协议。主 LLM 判断需要保存时，在回复末尾发起
    `<nola_memory_tool>{"long_term_memory":[],"user_preferences":[]}</nola_memory_tool>`；
    前端会把隐藏工具请求从流式 UI 与会话历史中剥离，再通过 `/api/agent/settings`
    合并写入长期记忆 / 用户偏好。没有隐藏工具请求就不落盘
  - `/api/chat/conversations/{id}/compact` 与 `/api/chat/conversations/{id}/memory/search` 仍保留为
    系统/调试接口，不要求普通用户手动触发
  - `/api/chat/conversations/{id}/session-memory` 支持直接查看 / 编辑 session memory
  - `/api/agent/settings` 同时持久化全局长期记忆、用户偏好、MCP 接口、用户上传 Skill 与文档，并在普通 Web 聊天、自定义 LLM 与 Agent 请求时一起注入上下文
  - MCP 能力来自类型化冻结目录：具体工具按精确 server/tool route 投影；资源和提示分别由
    固定的 `mcp_resources_read`、`mcp_prompts_get` schema 访问。Host 支持 Streamable HTTP
    与 stdio JSON-RPC，资源/提示结果继续作为不可信工具输出进入上下文。资源模板目前只编目，
    尚不执行化
  - `/api/chat/agent/v2/stream` 与 `/api/mobile/chat/agent/v2/stream` 在会话模式下都会自动回查原始记忆，并与 MCP/Skill 配置一起注入同一轮 ToolUseContext
- Web 对话标题是规范化会话元数据，不是消息保存、流式渲染或客户端本地猜测的副作用。
  `titleSource` 是必填的类型化状态：`automatic`、`first_message` 或 `user`。Host 在调用
  Core 之前，以独立的可见用户文本提交首问标题；模型使用的附件展开内容和补充上下文
  不得进入标题。标题折叠连续空白并限制为最多 64 个字符。显式重命名和分支标题归用户
  所有，后续消息不得覆盖。客户端只在消息与 Trace 完成耐久化后刷新规范化会话元数据，
  不比较本地化默认字符串，也不自行持久化推断标题。
- 工具结果在 Host→Core 边界必须拆分为不同所有权层级：`ToolPart.summary` 是 Host 提供的
  人类可读结果摘要，`output_preview` 是有界技术内容，`structured_output` 是机器可读结果，
  `full_output_ref` 指向超限完整输出，`artifacts` 描述可打开的文件或 URL。客户端不得把
  整个 JSON 信封序列化到摘要，也不得从 `output_preview` 反向推断结构化字段。工具过程在
  消息中保持可见，参数和技术输出由用户展开查看，artifact 保持直接可访问。
  `git_diff` 删除 `max_diff_chars` 参数与归一化兼容分支，完整 patch 统一交给 Host 输出层；
  工具内部不得先截断再声称保存了完整输出。范围收窄使用 `paths`，二进制使用 Git 原生
  diff 表示。完整输出文件的读取使用 UTF-8 安全字节 cursor；读完文件不等同于完成审计。
- 数据模型：`LocalUserAgentSettings`（用户级开关 / 白名单）、
  `AgentPermissionRule`（per-tool `allow` / `deny` / `ask` 规则，可带
  tool-specific match，例如 `broad:/Users`）—— 均为**本地** SQLite 表，
  启动时 `db.create_all()` 自动建表。权限评估参考 Claude Code 的分层模式：
  工具先返回内容级风险判断，规则层再按 match specificity 匹配；同一 specificity 下
  `deny > ask > allow`，更具体的内容规则优先于工具级 `*`。
- 第三方执行器的原生事件只能进入对应 Host adapter；adapter 必须把事件归一为相关联的
  `HostResponseEnvelope`，产品 UI 只消费 Core Trace，不接受旧版 Nola 事件协议
- `tool_result.meta` 可携带本地 artifact 元数据；浏览器截图保存到
   `web/app/static/agent_artifacts/<local_user_id>/`，前端工具卡片内联展示截图预览
- 浏览器页面状态存放在单次 `ToolUseContext.extras` 中，并在 SSE 请求结束时由
  `run_agent_sse` 清理；不会跨用户或跨请求复用浏览器页面
- 受管浏览器默认读取语义快照，只有布局、canvas 或视觉歧义需要时才截图。截图先保存为标准
  `image/*` HostArtifact；Core 保留 artifact identity，但只有当前 Turn 冻结的
  `ModelCapabilityProfile.vision_input=true` 时，Agent Host 才读取受限大小的本地图片并映射到
  provider 的结构化图片块。禁止按模型名猜测多模态能力。
- 浏览器导航和请求必须执行 URL/network policy；测试 fixture 可使用显式本地测试策略，生产权限
  不得因 Playwright 或 extension adapter 不同而改变。
- 本机磁盘问题应先在 Explore 阶段判断范围：`/`、`/Users`、`/System`、`/Library`、
  `/private`、`/Volumes` 等大范围扫描会返回 `ask` 权限决策，要求 Agent 先向用户确认；
  wildcard allow 不会覆盖工具层判断出的高风险 `ask`，只有明确确认或内容级 allow 规则
  才可继续。用户确认后优先使用 `disk_usage` 在允许目录内读取磁盘容量与一级目录用量。
  该工具对权限错误和超时做摘要并返回 `skipped_or_partial`，避免模型直接用宽泛 `du`
  shell 命令长时间扫全盘。`bash` 工具同样会拦截未确认的 `du/find/mdfind` 大范围扫描。
- 交互式权限：Core 将工具 Part 置为 `waiting_for_permission` 并发出带稳定 effect/request
  标识的 HostRequest。Host 在当前 assistant 气泡中展示确认卡片，通过独立权限响应端点把决定提交
  回同一 Turn；确认过程不产生 `done`、不重放 prompt，也不另建 assistant draft。
- 并发策略：当前轮所有 `tool_calls` 全部 `is_read_only && is_concurrency_safe`
  时并发执行，否则顺序执行
- 兼容性：`local_dev` 工具与本仓库既有「客户端独立自持、最小上传、不暴露
  本地数据给后端」原则一致——它们是对**本机进程**的本地能力扩展，不会
  把本地文件 / shell 输出发送给后端

### 9.2 iOS Chat Agent / Context Controls（2026-04-27，2026-05-06 更新）

iOS 默认把任务解析能力与 Claude Code 风格的上下文管理落在本地聊天层；当用户
在模型管理中选择「Nola Web 主机」模型时，iOS 会通过 Web mobile bridge 把输入
交给宿主机 Web Agent，多轮工具循环仍在 Web 主机执行：

- Agent 模式：
   - `ChatView` 使用 `@AppStorage("chat.agent_mode.enabled")` 保存显式开关，输入栏以
     “Agent / 复杂问题” pill 展示状态
   - `ChatManager.processMessage(..., agentMode:)` 在本机模型/普通云模型下只注入
     `PromptTemplates.complexAgentMode` 与 iOS no-code 边界，不增加任何本地代码执行、
     shell、文件写入或浏览器自动化能力
   - 选择 Web 主机模型时，iOS 通过 `WebAgentProvider.makeHostAgentPayload(...)`
     发送 `agent_mode=true`，由宿主机 `/api/mobile/chat/agent/v2/stream` 注入复杂
     Agent prompt；能否使用浏览器、本地文件或 shell 工具仍完全由宿主机
      `LocalUserAgentSettings` 与权限白名单决定
- 自动长期记忆 / 用户偏好沉淀：
   - 普通本机 iOS 对话（非 Web 主机模型）不会由客户端按关键词主动触发提取；`ChatManager`
     只在本轮 prompt 中暴露隐藏的 Nola memory tool 协议
   - 主对话 LLM 认为确实需要沉淀时，才在回复末尾发起隐藏工具请求：
     `<nola_memory_tool>{"long_term_memory":[],"user_preferences":[]}</nola_memory_tool>`
   - `ChatManager` 会把隐藏工具请求从 UI 与会话历史中剥离，再在后台落盘；没有隐藏工具请求就不写入长期记忆 / 用户偏好
   - 工具请求只保存跨会话稳定可复用的信息和明确/自然表达的长期偏好；prompt 明确禁止保存一次性任务、
     临时兴趣、密钥凭据或敏感个人属性
   - `ChatManager.mergedMemoryProfile(...)` 按行去重后通过 `CanonicalChatMemoryProfileStore`
     写入结构化 Memory、Skill、MCP Server 与版本化 ProductSettings；iOS 不再保留
     UserDefaults 记忆档案或附件影子副本。后续 `DefaultContextManager.buildContext(...)`
     会从 canonical Store 投影这些条目并注入普通对话上下文
   - Web 主机模型不走 iOS 本地沉淀，仍由宿主机 Agent 的长期记忆 / 用户偏好工具作为持久化入口
- 更多设置：
   - `ChatView` 输入栏保留回复风格快捷切换；其余当前对话参数统一由“更多设置”
    pill 打开 `ChatMoreSettingsSheet`，不再提供步数或最大迭代配置
   - 每模型生成参数由 canonical `ModelProfile.parameters_json` 持有；iOS/macOS/Android
     参数 UI 与推理入口读取同一投影，不写 UserDefaults 或 Android DataStore
   - iOS/Android 直连云模型的执行能力由同一 canonical `ModelProfile.source_json.executionCapabilities`
     显式声明，缺失字段按不支持处理；能力随模型档案版本更新，Turn 开始时从当前档案冻结
     endpoint、模型、参数和能力，不能从过期会话展示值或另一个模型的全局 endpoint 推断。
     工具和图片只在已声明支持时发送；未声明工具能力时，设备 Agent 不向模型公布工具目录。
     非默认 temperature/top_p 只在已声明 sampling 支持时发送；未声明时默认值也不隐式写入请求。
     iOS 与 Android 均按档案的 `wireProtocol` 选择 Chat Completions 或 Responses；Responses 使用
     `store=false` 与加密 reasoning item 的不透明续跑状态，Core 负责保存并在后续模型调用时交还，
     Host 必须核对 provider、模型和路由后才可回放。iOS/Android 端点编辑器显式选择协议，不能在保存时
     隐式改写已有档案协议；iOS 云模型密钥按模型 ID 保存在 Keychain，档案及会话快照不写明文密钥。
     移动端尚无会话级 effort 选择，因此仅能执行不需要 effort 的 Responses 档案。
     从其他客户端导入的档案若声明了显式默认 effort／budget，移动端在模型档案解析处拒绝该次调用，
     不能忽略声明后把请求当作自动 effort 发送。
     两端均不允许
     声明「仅在 reasoning=none 时采样」这类需要显式 effort 控制的条件能力。拒绝无效配置而非
     猜测模型家族或静默降级。Web 主机模型仍由主机自己的能力与权限策略控制。
   - `ChatMoreSettingsSheet` 集中管理模型参数（回复风格、最大上下文、最大输出、
     temperature/top_p、thinking/stream）、Agent 执行（Agent 开关、检索增强）、
     自动 compact 阈值、session memory、长期记忆、用户偏好和本机 Skill 启用状态
   - Agent 执行不设置人为步数上限；任务在得到最终结果、用户主动停止、权限被拒绝或遇到
     不可恢复错误时结束，防循环与资源保护由运行时安全策略负责，而不是交给用户配置“50 步”
   - 自动 compact 阈值通过 `UserDefaults["chat.auto_compact.trigger_percent"]` 持久化，
     `ChatManager.autoCompactConversationIfNeeded(...)` 按当前模型上下文上限的百分比触发，
     默认 80%，允许范围 50% 到 95%
- 回复风格：
   - `ModelParameters.responseStyle` 保存每个模型的默认风格（`professional` /
     `casual` / `playful`），`ModelParametersView` 提供分段选择
   - `ChatView` 在新对话/当前输入栏提供风格 pill，默认读取当前模型参数，发送时通过
     `ChatManager.processMessage(..., responseStyle:)` 写入 `ModelInput.userPrefs["response_style"]`
   - `OpenAIProvider` 与 `LocalProvider` 使用 `PromptTemplates.chatSystem(responseStyle:)`
     拼接基础聊天 prompt 和当前所选语言风格段；关闭 thinking 不改变该 prompt
   - `Conversation.selectedModelInfo` 保存 iOS 当前会话模型选择；`ChatView` 选择模型后
     立即写入当前会话，`ChatManager` 恢复最近会话、切换会话、撤销删除恢复会话时优先
     使用该会话保存的模型，再回退到全局最近模型。
- 默认任务解析：
   - `ChatManager.processMessage(_:)` 对任务型输入直接进入 `TaskParsingService`
   - 选择 Web 主机模型时跳过 iOS 原生任务解析，把输入发往宿主机 Agent，避免拦截
     “打开浏览器/点击/搜索”等桌面操作指令
- Web 主机桥接：
   - iOS 调用 `POST /api/mobile/chat/agent/v2/stream`
   - Rust CLI Web 后端在 mobile bridge 内读取宿主机 agent settings，
     使用宿主机配置的 LLM endpoint/API key/model、MCP 与工具权限；请求体中的
     prompt/timezone/工具启用意图只代表移动端输入，不允许 iOS 覆盖宿主机 LLM 配置
    - `WebAgentProvider.makeHostAgentPayload(...)` 仅发送 `messages`、
      `allow_local_dev_tools`、`max_iterations`、`timezone`、可选 `agent_mode`、
      当前回复风格 `response_style`，
      不会发送 `model`、`customEndpoint` 或 `customApiKey`
   - 开发启动默认开启 LAN 模式：`dev.sh` 默认 `NOLA_LAN=1`，将 Web/后端绑定到
     `0.0.0.0`，Web 默认使用 HTTP 方便 iOS 连接；需要本机-only 时可用
     `NOLA_LAN=0 ./dev.sh`
   - iOS 允许局域网 HTTP（Info.plist `NSAllowsLocalNetworking=true`）。如果用户手动启用
     HTTPS，「Nola Web 主机」模型仍可保存 `tlsCertificateSHA256`，`WebAgentProvider`
     会对 `/api/mobile/login`、`/api/mobile/me` 与 Agent stream 使用同一指纹做
     URLSession server certificate pinning
   - mobile bridge 业务通信使用 access token 派生的 AES-256-GCM envelope：
     iOS 对请求 JSON 加密并带上 `X-Nola-Mobile-Crypto: aes-gcm-v1`；Web 端使用同一个
     bearer token 做 SHA-256 派生密钥解密，未加密的 Agent 请求返回 400；Web Agent
     SSE 中每个 JSON 事件也加密为 `type=encrypted` envelope 后再发送
   - `WebAgentProvider.generateAgentEventStream(...)` 解密 canonical Web Agent SSE 后，将
     `text` / `reasoning` / `tool_use` / `tool_result` / `permission_request` / `done`
     归一为带 effect/request/session/turn 关联的 Host 响应；消息、工具卡和状态栏均由
     Core Trace 投影，不直接渲染 wire event
   - iOS 收到权限 HostRequest 后由 `ChatManager.pendingHostPermissionRequest`
     挂起当前工具 effect，`ChatBubble` 在对应工具事件卡片内显示允许/拒绝按钮；决定通过
     `/api/mobile/chat/agent/v2/permission` 返回，原 SSE 与原 Turn 保持打开
   - `tool_result.meta.screenshot_url` / `final_url` / `title` 会解析为
     `HostAgentToolEvent`，相对 artifact URL 以 Web 主机 baseURL 补全
   - `ChatManager.hostToolEvents` 作为当前回复的结构化工具事件流，`ChatView` 将其写入
     当前 assistant draft；`ChatBubble` 用工具卡片展示运行状态、工具名、参数/结果摘要、
     最终 URL 和浏览器截图链接/预览
- Claude 风格上下文管理（自动）：
  - `Conversation.sessionMemory`
  - `Conversation.lastCompactedAt`
  - `Conversation.compactedMemoryEntries`（compact 原始快照，含 summary/rawExcerpt/archived ids）
  - 长期记忆、用户偏好、Skill 和 MCP 目录通过 iOS Store FFI 写入 canonical records。
    `UserDefaults` 只保存明确的 Host/UI 本地偏好，不导入或回填产品事实
  - `DefaultContextManager.estimateContextUsage(...)` 估算 active messages / session memory /
    long-term memory / user preferences / MCP / uploaded skills / uploaded documents /
    supplemental context 的 token 占用，并生成分级上下文建议（含 estimatedSavingsTokens）
  - `DefaultContextManager.compactConversation(...)` 把较早消息摘要为带覆盖范围的 context
    snapshot；原 Message/Part 保持不可变，Presenter 根据 snapshot 的 archived ids 从后续
    prompt 投影中排除，不再更新第二份消息行
  - `DefaultContextManager.searchRawMemory(...)` 在 session memory / compact 快照 /
    archived 消息中检索原始信息，供自动记忆回查与上下文增强使用
  - `ChatManager` 在消息写入后自动评估上下文占用并触发 compact；在构造 prompt 时自动检索
    相关原始记忆片段并注入当前轮上下文
- 会话级附加上下文：
  - `Conversation.selectedPromptTemplateID`
  - `Conversation.dataSourceIDs`
  - `DataSourceKind` / `DataSourceCapability` 描述通用可读取数据源的类别与能力；
    `URLDataSource` 只保存当前网站、搜索结果、API、订阅源、日历订阅、文件和文件夹类型
  - `ReadableDataSource` / `ReadableDataSourceItem` 是新的上下文抽象，允许事件型来源与文档、
    文件夹、搜索结果等任意可读来源共同进入对话上下文
  - 本地文件/文件夹来源通过 SwiftUI `fileImporter` 创建，`URLDataSource.securityBookmarkData`
    保存文件选择器返回的 bookmark；读取时优先解析 bookmark，并在本机提取 UTF-8 文本预览，不上传文件内容
  - `ChatView` 同一面板仍允许用户为当前对话选择 prompt template，并挂接本地
    `URLFetcherService.sources` 中的可读取数据源；UI 不再把该能力限定为 URL
  - `DefaultContextManager.buildContext(...)` 在构造 prompt 时读取这些字段，把 user
    preferences、long-term memory、session memory、实时 MCP tools/resources/资源内容、上传 Skill /
    可读取数据源、模板与可读取数据源的上下文项共同注入当前对话上下文
  - 数据源上下文项注入策略：
    - 优先读取 source 历史中的 `ReadableDataSourceItem`（含历史摘要与 rawResponse）
    - 若 source 是定时自动化（有 schedule），额外从 `LocalJSONStorage.scheduled_action_runs.json`
      + 关联 journal 读取最近执行输出，作为同一数据源的上下文项
    - 若 source 历史为空且允许实时抓取，按 source 格式执行一次 `fetchEventsStateless`，把 URL
      抓取结果转换为可读上下文项再注入
- Web 可读取数据源：
  - Web 聊天页使用 `LocalUserAgentSettings.uploaded_documents_json` 保存浏览器选择的文件与文件夹文本内容；条目带 `kind=file|folder` 元数据，默认只作为本地上下文资产注入对话
  - Web 定时文件/文件夹数据源复用 `LocalScheduledAction.format=file|folder` 与 `url` 字段保存宿主机绝对路径；创建与执行都会通过 Agent 允许目录校验
  - `run-now` 读取本地文件或枚举文件夹中的可读文本文件，生成 `local_file_snapshot` events 并写入 `local_execution_results`；这类数据源不会触发后端同步或云端 run_now
- 同步保护：
  - `ChatManager.syncConversationsFromBackend()` 在导入远端会话标题 / 时间戳时，会保留
    本地会话上的 `language`、`sessionMemory`、`lastCompactedAt`、
    `selectedPromptTemplateID`、`dataSourceIDs`、`persistenceEnabled`，避免本地上下文状态
    被远端空字段覆盖

## 10. iOS 与 Web 的关系

1. iOS 是主客户端
2. Web 是辅助入口（管理、调试、桌面访问）
3. iOS 连接到自托管 Web 主机时，Web 主机是 LLM、MCP、浏览器与本地工具的执行环境；
   iOS 只是输入与展示端，不上传或覆盖宿主机的 endpoint/key/model 配置
4. 两端共享同一后端模型与接口语义
5. 所有关键业务逻辑最终以后端约束为准

## 11. 非功能性要求

1. 可观测性
   - 后端关键链路要有结构化日志

2. 一致性
   - 同一 client_uuid 在 iOS/Web 的 tier 与 usage 表现一致

3. 安全性
   - token 不落日志
   - Debug 接口受保护

4. 可维护性
   - 文档分层：需求、设计、变更记录分离
   - 设计文档禁止混入大段历史 patch 内容

5. 数据库性能（2026-04-24）
   - **后端**：所有"热路径"查询均有对应的复合或部分索引支撑，统一在
      `db.ensure_indexes()` 中声明，`init_db()` 启动时幂等创建
      （`CREATE INDEX IF NOT EXISTS`）。
   - `usagerecord(client_uuid, period_start)` 同时是用量正确性唯一约束；创建失败会阻止启动，避免并发写入产生重复月度记录。
   - 复合索引覆盖：`usagerecord(client_uuid, period_start)`、
     `chatconversationrecord(client_uuid, updated_at)`、
     `chatmessagerecord(conversation_id, id)`、
     `pushdevicerecord(client_uuid, device_token)` / `(client_uuid, push_enabled)`、
     `scheduledsourcerecord(client_uuid, source_id)`、
     `scheduledjobrecord(status, scheduled_at)`、
     `scheduledresultrecord(client_uuid, created_at)`、
     `scheduledresultackrecord(result_id, device_id)`。
   - 部分索引覆盖调度器"待处理集合"扫描：
     `scheduledresultrecord(scheduled_at) WHERE silent_push_sent=FALSE`、
     `scheduledresultrecord(fallback_due_at) WHERE visible_fallback_sent=FALSE`、
     `outboxevent(id) WHERE sent_at IS NULL`。索引体积只随未处理行数增长。
   - 并发写优化：`add_result_ack` 使用原子
     `UPDATE … SET ack_count = ack_count + 1` 代替 SELECT+update，消除读改写
     竞态并减少一次往返。
   - **Web（Rust Host）**：Web 会话、设置、自动化任务与结果只读写 Nola Store；
     `~/.nola/web/` 仅保存 Web Host 身份和访问令牌，不保存业务状态。
   - **iOS（FileBasedUnifiedStorage）**：加入进程内缓存 + dedupeKey/id
     字典索引，消除每次读写的 JSON 全量解码与 O(N) 线性搜索。写入时同步
     更新缓存，保持 `ioQueue` 串行化，线程安全不变。

## 12. 当前已验证状态

截至 2026-05-16，以下链路已验证：

1. Web 登录（本地邮箱/密码本地缓存 + Apple/Google/GitHub 官方 OAuth 链接）可用
2. Web 登录后读取 /api/tasks 可获取真实后端 tier/usage
3. Web 定时动作创建链路已打通（依赖有效 access_token）
4. 后端 sources/sync 的关键 500 问题已修复
5. Web AI 对话（SSE 流式）链路已验证可用
6. Web 执行记录页接口已对接（依赖 debug token）
7. 定时动作删除和立即执行链路已实现
8. Web Stripe 订阅完整链路已实跑：`/purchases/stripe/checkout` → `/purchases/stripe/checkout/verify` → `/purchases/stripe/webhook`，最终 `/purchases/tier/{uuid}` 返回 `subscription`
9. 官方后端邮箱密码注册/登录已禁用；Google/GitHub provider auth 自动注册/复用账号的后端测试通过，Apple 合并测试保持通过
9. 真实 no-mock 集成入口 `./scripts/run_real_integration_tests.sh` 已验证：backend Uvicorn + SQLite、Web 调真实 backend、iOS 订阅/token 提示测试均可跑通
10. AI 对话标题链路已验证：Backend/Web/iOS 均使用首个非空 user query 命名默认会话，显式标题和后续消息不覆盖
11. Web/iOS Agent 模式显式开关链路已验证：Web 开启后走 Agent v2 并注入复杂 Agent prompt；iOS 本机仅注入 no-code Agent prompt，连接 Web 主机时只转发 `agent_mode`，不携带 iOS 侧 LLM 配置
12. Web Agent 本机磁盘查询链路已验证：未确认的全盘/大目录扫描会被工具权限层拦截并通过 `permission_request` 触发 Web/iOS 交互式确认；窄目录或已确认扫描可通过 `/api/chat/agent/v2/stream` 调用 `disk_usage`，返回已用/可用空间和一级目录统计；权限或超时目录会显式标记为 partial
13. 定时任务支持按 source 配置 `llmModel` / `workflowPrompt` / `skills`，`run_now` 与调度执行都会读取并在诊断中回填，非法 skills 会被 `sources/sync` 拒绝
14. Web Agent 不提供独立 native app host；本地应用能力统一由 Agent Host 的客户端回调执行
15. iOS 会话级数据源上下文支持 URL 实时抓取兜底（历史为空时）与定时任务最近执行输出注入（run 记录 + journal 文本）

### 12.0.1 Stripe 订阅实跑结果（2026-05-03）

基于本地测试环境对同一 `client_uuid=11111111-1111-1111-1111-111111111111` 的完整链路执行结果：

1. Checkout 创建成功：`CHECKOUT_STATUS 200`
2. 支付后校验成功：`VERIFY_STATUS 200`，返回 `tier=subscription`、`subscription_active=true`
3. Webhook 更新成功：`WEBHOOK_STATUS 200`
4. 最终订阅状态：`FINAL_TIER subscription`
5. 到期时间被更新：`FINAL_EXPIRES 2026-06-30T00:00:00`

说明：

1. 校验路径用于支付成功回跳后的“立即生效”（减少仅依赖 webhook 的延迟）。
2. webhook 路径用于续费、取消、状态变更的持续同步。

### 12.1 本地联调用测试 ID（仅开发环境）

以下信息仅用于本地开发和联调验证，不得用于生产环境：

1. 测试 client_uuid
   - `11111111-1111-1111-1111-111111111111`
   - ⚠️ 注意：后端内部将 UUID 转换为 hex 无连字符格式存库，purchaserecord 表中实际存储为：`11111111111111111111111111111111`

2. 测试 apple_user_id
   - `test.apple.user.111111111111`

3. 测试 access_token（有效期至 2027-04-19，可重新生成）
   ```
   eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJub2xhIiwic3ViIjoidGVzdC5hcHBsZS51c2VyLjExMTExMTExMTExMSIsImlhdCI6MTc3NjU0ODAxMywiY2xpZW50X3V1aWQiOiIxMTExMTExMS0xMTExLTExMTEtMTExMS0xMTExMTExMTExMTEiLCJleHAiOjE4MDgwODQwMTN9.JFrPvJJpHyagntqlilEulNQQZg_y3GReueqtWV9l_s4
   ```
   生成方式（在 backend/ 目录执行）：
   ```python
   import jwt, time
   SECRET = '115c3d73fd428b81e3dfd202a8b8ec857c3c43ea2967ebf30ca2f45bb6e57702'
   now = int(time.time())
   payload = {
       'iss': 'nola', 'sub': 'test.apple.user.111111111111',
       'iat': now, 'client_uuid': '11111111-1111-1111-1111-111111111111',
       'exp': now + 86400 * 365,
   }
   print(jwt.encode(payload, SECRET, algorithm='HS256'))
   ```

4. 测试 purchase 记录（PostgreSQL nola_cn 库）
   - 字段：product_id=com.nola.sub, tier=subscription, is_active=true
   - UUID 必须以 hex 无连字符格式插入：`11111111111111111111111111111111`
   - 插入 SQL：
     ```sql
     INSERT INTO purchaserecord (uuid, original_transaction_id, product_id, purchase_date, expires_date, is_active, tier, created_at, last_verified_at)
     VALUES ('11111111111111111111111111111111', 'test-txn-111111111111', 'com.nola.sub',
             NOW(), NOW() + INTERVAL '1 year', true, 'subscription', NOW(), NOW())
     ON CONFLICT (uuid) DO UPDATE SET is_active=true, tier='subscription', expires_date=NOW()+'1 year';
     ```

5. 本地数据库
   - 类型：Wrangler 本地 D1
   - schema：`backend/worker/migrations/0001_current_schema.sql`

### 12.2 本地启动命令

```bash
cd backend && npm run dev
```

Web 在 `cli/` 中通过 `cargo run -- web --host 127.0.0.1 --port 5001 --web-root ../web` 启动。

## 13. 已知坑点与修复记录

1. Worker 与本地开发必须使用同一份 D1 schema 和 API contract。
2. 模型思考内容与正文必须按 provider wire protocol 解析后投影为 Core Part，不得由 UI 猜测。

## 14. 后续演进建议

1. 登录能力增强
   - 增加自动签发 access_token 流程，减少手工粘贴 token

2. 可观测性增强
   - 为 Web 增加连接诊断面板（backend health、token 可写性）

3. 测试体系增强
   - 增加 Web 端到端测试（登录、创建、回查）

4. 文档治理
   - CHANGELOG 只放摘要
   - SOFTWARE_DESIGN 只放稳定架构与现状

---

维护约定：

1. 新增业务能力时，优先更新本文件中的接口分层、数据模型、链路设计三节
2. 历史叙述和一次性修复过程，统一记录在 CHANGELOG 或独立技术记录文档

Runtime Driver 只对外提供 committed Application port。旧的事务 committer、直接注入 Host
response、直接执行调用方传入的 committed request 等接口已经删除，不作为未上线产品的兼容面。
当前仍需收敛的进程内 reducer 仅用于尚未 Application 化的 submission，不允许新增调用者。
新对话合同接收原始 `AgentSubmission` 和冻结的 `AgentCoreConfig`；Application 自行恢复或初始化
Core，并在一次事务中提交用户消息、空的助手消息、Run、checkpoint 与 external outbox。客户端
不得预先归约后上传 checkpoint 或 Host request。
活动 Run 内的取消、配置调整等内部 submission 通过带幂等 transition ID 的 Application
事务提交。存在活动 effect 的取消先原子提交非终态 `cancelling` 与 cancel outbox；关联 Host
确认再原子清除 target/cancel effect、发布 `TurnCancelled` 并关闭 Run。终态事务不为已经
关闭的 Run 留下无法接收响应的延迟取消 effect。
文件工具的输入资源属于可展开的工具过程，不等于面向用户的最终附件。Host 不得因为 `write_file`、`edit_file` 等调用包含路径，就把该路径提升到消息附件；这会泄露 Skill 的 JSON specification、临时脚本和校验辅助文件。最终附件只来自工具结果明确声明的 `artifact_path`、`output_path`、`artifacts` 等结构化字段，或命令中可静态确认且成功存在的显式 `--output` 参数。客户端继续展示文件操作及参数，但附件区域仅展示明确交付物。
## 真实 Agent Host 验收记录（2026-08-21）

内部协议兼容层移除后，使用真实远端模型和真实 `nola-agent-host` 进程执行：

```bash
cargo build --bin nola-agent-host
python3 scripts/test_real_agent_host.py target/debug/nola-agent-host
```

验收覆盖并通过以下链路：

- 客户端上下文通过 Host v2 协议注入，模型能够准确返回上下文中的唯一事实。
- 模型自主选择 `write_file`，事件严格按 `tool.started -> tool.completed -> core_part_updated -> run.completed` 排序。
- 后续自然追问继承同一会话的产物意图和工作区授权，不依赖关键词路由。
- 多轮真实请求触发 LLM 与 Core 协同压缩，并能召回压缩前的早期事实。
- 无效密钥只产生一个失败终态，进程非零退出，不产生虚假的完成事件。

该验收不使用 mock provider。测试在隔离的 HOME、数据库和临时工作区中运行，复用本机 Nola 配置但不输出密钥。

Worker 定时数据源现已由每分钟 Cron Trigger 驱动，按数据源 IANA 时区计算 `once`、`daily`、`weekly` 到期槽位，并通过 D1 原子认领保证同一数据源槽位只执行一次。真实验收命令为：

```bash
npm --prefix backend/worker run typecheck
npm --prefix backend/worker run test:scheduler:e2e
```

## 15. Canonical Application Service 与上下文工作集实现合同（2026-08-24）

1. `nola-service` 按 canonical database path/socket 复用并承担设备级单写职责。CLI、Web、
   `nola-agent-host` 与 macOS 客户端不再传递 parent PID，也不因一个短进程退出而终止服务。
2. macOS 后台 Goal、会话恢复和 Inspector 只调用 `MacApplicationPort` 查询
   `ConversationProjection`/session projection；不得读取 `*.agent-v2.checkpoint.json`。
3. canonical SQLite 中的 `conversations`、`messages`、`message_parts` 和 `core_checkpoints`
   分别承担完整记录、typed Part 与活动 Core 工作集职责。checkpoint 不得替代 transcript。
4. `seal_completed_turn_model_context` 清除临时 reasoning、工具交换和 draft，但保留该 Turn
   生成的 Compaction item。下一轮使用该压缩工作集，同时 UI 和历史查询仍读取完整 Message/Part。
5. 数据源定义、验证状态、ReadRecipe、内容格式、会话覆盖和 usage audit 写入 canonical Store。
   Host 的一次性原始读取不进入 checkpoint；显式 Snapshot/Blob 和已经提交的 Assistant Text Part
   按各自产品对象持久化，二者不得混淆。
6. `scripts/e2e/cargo_paths.sh` 与 `scripts/e2e/cargo_paths.py` 是测试侧统一 Cargo 产物解析入口，
   通过 `cargo metadata` 获取 target directory。新增测试不得硬编码 `cli/target`。

当前真实验证覆盖：

- 同一 session 的两个独立 CLI 进程复用 Application Service，第二轮无需工具即可恢复首轮事实。
- Agent Host 在短进程连续调用后仍可完成工具顺序、失败终态、真实压缩与早期事实召回。
- 数据源启用、停用、usage audit 与 checkpoint 原始载荷隔离均通过真实 LLM 验证。
- Apple 客户端通过 `NolaCore.xcodeproj` target dependency 构建 Core；macOS 同一次依赖图构建并打包当前 `nola-agent-host` 和 `nola-service`，不得复用协议版本不匹配的常驻 Service。

该 E2E 启动真实 Wrangler 本地运行时、应用真实 D1 migration、触发 `__scheduled`、执行真实外网抓取，并验证结果落库和重复触发幂等。

仍未完成的能力不得由上述结果代替，包括通用 Native App Host；这些项目继续保留为显式实现与验收项。Android 端侧模型已用真实 Qwen3 0.6B GGUF 在 Android 35 arm64 模拟器完成 native engine 与生产 `LocalLlamaProvider` 两层生成 E2E，不能以静态目录测试替代后续真机性能基准。

## 2026-10-09：Mac 默认模型与稳定呈现契约

- 全局默认对话模型由 canonical `chat.default_model` 严格版本化设置表达自动选择或精确
  profile ID。会话显式选择高于默认；缺失设置代表自动，损坏设置不得偷偷降级为首个模型。
  模型目录只读取事实，不读取所有连接凭据；提交只解析所选路由的凭据与参数。
- 默认选择与列表顺序、优先级、本地运行目标相互独立。模型中心区分连接归属与模型属性，
  保留编辑草稿，按连接修订保存，冲突不覆盖其他连接。设置摘要与输入框使用相同默认投影。
- 每轮提交冻结精确模型身份、参数及默认修订；远程 Host 在派发前检查继承默认的修订。
  生成中改变默认不改变当前请求，下一次继承默认的提交读取新值。
- Journal 呈现 owner 持有稳定内容身份、展开状态与已见节点；正文解析终态和新增文字动画
  资格是两个独立状态。完成不清空正文、不截断末批显现；历史加载不重复播放流式动画。
- 新增文字在不改变布局测量的前提下轻微上浮并渐清晰，Reduce Motion 直接显示。
  旧回复因新消息迁入历史时允许立即稳定显示，不承诺跨父容器逐帧保持剩余动画。
- 跟随与阅读是明确滚动意图；返回最新使用实际尾部锚点，而非 lazy document 的估算高度。
  完成和窗口重排不能把视口留在正文之外，也不能覆盖用户主动阅读位置。
- 聊天正文与输入框最大宽度统一为 1,200 pt，随可用宽度连续变化；正文 16 pt。

审计、设计反思、实现及分批验收证据见 `../../历史任务/实施与审计/MAC_UI_REDESIGN_EXECUTION_2026-10-09.md`
与对应聊天、模型中心、UI 模块审计。编译警告整改独立验收，不能由上述 UI 回归替代。
