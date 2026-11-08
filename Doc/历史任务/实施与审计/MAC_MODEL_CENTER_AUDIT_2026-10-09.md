> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Mac 模型中心：审计、设计与反思（2026-10-09）

状态：阶段 2 的审计、设计、反思、实现及模型专项验收完成；中英文独立进程截图已复核，宿主键盘/AX 交互通过，未发现未关闭的模型中心阻断问题。未执行操作系统 VoiceOver 语音人工走查，不将 AX 契约验证冒充语音测试；该人工增强检查不作为本轮新增发布门禁。下列第 1 节保留实施前证据，第 5 节记录当前实现与验证边界。

## 1. 实施前代码审计

本次以工作区源码为依据，未修改用户配置、未读取或记录凭据。

| 现象 | 代码证据 | 根因与分类 |
| --- | --- | --- |
| 默认对话模型没有明确设置动作 | `MacRemoteModelManagerView.modelDetail` 仅显示 `Default for chat`；`modelRowActions` 只有优先级、排序、删除 | 功能缺失，不是按钮配色问题 |
| 界面的“默认”不一定等于运行选择 | 标记来自 `MacRemoteModelOrdering.primaryChatModelID`（优先级）；服务 `loadThrowing` 优先 `profile.selected`；工作区 `selectedModelOption` 无覆盖时直接取 `configuredModelOptions.first` | 三条默认解析路径，领域模型分裂 |
| 保存连接可能重设默认 | `MacAppleChatConfigurationStore.persistEndpoints` 每次将 `selectedID` 重算为 `eligible.first`，而非保留显式默认 | 编辑连接、排序与默认选择被错误耦合 |
| 本地“Select”不是统一对话默认 | `MacLocalModelConfigurationStore.selectedModelID` 从本地 profile 的 selected 读取；工作区始终优先远程 configuredModels，再考虑本地下载列表第一项 | 本地运行目标与默认对话对象混用；用户选本地，聊天仍可能远程 |
| 底层 selected 语义也不自洽 | `core/nola-store/src/model_store.rs`：replace 要求每 provider/runtime 有一个 selected；upsert 将所有其他 provider 的 selected 清空 | provider 级选择不应冒充跨来源产品默认 |
| 编辑层级碎片化 | 模型详情 basic / parameters / capabilities / access 菜单；parameters 页本身只有打开另一 sheet 的入口；有未保存连接时参数入口禁用 | 操作对象与保存边界不一致；模型参数是模型对象，不应先保存才能编辑 |
| 使用范围、能力、权限容易混淆 | access 页包含 chat/retrieval/issueRepair 和权限解释；capabilities 页包含工具协议能力及 reasoning 控制 | 三个领域需要明确区分，不应让用户把勾选工具能力当授权 |
| 页面层级过多 | Models → Local/API → 连接树 → 模型 → 配置菜单 → 参数 sheet | 高操作成本；普通设置藏在层层切换后 |
| 切换连接丢失目标模型 | sidebar 模型选择发现另一连接 dirty 后只保留 pendingEndpointSelection；确认后 selectedModelID=nil | 导航意图不是结构化目标，确认完成没有恢复用户点的模型 |
| 状态提示导致布局跳动 | 远程 manager 顶部 notice 出现/4 秒自动消失，并绑定整个容器动画 | 状态反馈占据可变布局高度，改变列表/详情可用区域 |
| 会话覆盖已有正确基础 | `MacConversationModelSettingsView` 文案说明仅本会话/模型；`MacWorkspaceModel.resolvedModelParameters` 由覆盖应用于默认值 | 应保留，而非新做第二套参数存储 |

## 2. 设计：先确定所有权

### 2.1 四类对象，不能互相借用字段

| 对象 | 所拥有的数据 | UI 位置 |
| --- | --- | --- |
| 全局对话偏好 | 默认模型的规范 profile identity；不包含凭据、推理参数或工具授权 | 模型中心固定顶部“默认对话模型”；适用于所有仍继承默认的会话 |
| 连接 | 名称、URL、凭据、默认协议、连接启用、诊断 | 模型库旁的“管理连接”，或模型详情所属连接链接 |
| 模型配置 | profile ID、提供商 model ID、显示名、模型启用、模型参数、协议能力、推理等级映射、可参与用途 | 模型详情；显式显示所属连接/本机 |
| 会话配置 | 已选择的 profile identity、逐模型参数覆盖、三档响应复杂度、权限模式 | 对话输入框及本会话设置，不在模型中心编辑 |

工具调用能力是提供商协议能力；用途是模型是否可参与某任务；权限是具体工具/资源的访问决策。这三者的存储与执行检查都维持分离。模型中心不提供假装能授权文件/设备的工具开关。

### 2.2 统一默认对话契约

引入一个明确的跨 API/本地“默认对话模型”产品设置，由现有 canonical product setting 持久化；值使用规范 profile identity（必要时含来源），不得用显示名、provider model name、priority 或当前运行实例名代替。

- 设置默认不改变连接排序、优先级、采样参数、本地运行进程、已有会话的显式选择或已经捕获的运行配置；仍继承默认的已有会话在下一次提交时读取新值。
- API 与已安装可用本地模型出现在同一默认选择器中，按来源分组；相同 model name 通过 profile identity 区分。
- 连接保存/删除/模型参数保存不得默默覆写该设置。
- 默认不存在时，可为首次使用解析明确的自动候选；显示“自动选择”，不能伪称用户已设置默认。
- 显式默认已删除、停用或本地未安装时，显示“默认模型不可用，请选择”；不静默换到另一提供商。旧会话显式模型也遵循同样原则。
- 短暂健康故障不覆盖偏好。是否自动故障转移是执行路由策略，必须可追踪，不得通过改全局默认实现。
- 远程 Host 会话由该 Host 的目录/default 描述决定，不能拿本机 profile ID 解析为同名远端模型。
- 会话模型意图明确区分“继承全局默认”和“显式选择 profile”。新会话默认为继承；界面必须标明继承。更改全局默认只影响仍继承的会话，不改变任何显式选择；已捕获的运行配置绝不随全局更新改变。不要把第一次生成时的解析结果误写为用户显式选择。
- 解析结果必须区分可用、自动但无候选、显式全局默认缺失、显式会话模型缺失。后两者不可折叠为 nil 后再统一 fallback；提供“选择模型”或用户明确的“恢复继承”动作。
- 每次提交顺序：本会话显式模型 → 当前 Host 的全局默认/明确自动候选；参数顺序：协议有效默认 → 模型参数 → 会话逐模型覆盖 → 本次三档映射。提交前冻结本次有效配置。

不要用 provider/runtime 的 `selected` 修补统一默认。现有 selected 是历史目录/本地运行选择，需在实现中显式隔离，避免连接重存清空另一个域的默认。新增设置必须经 canonical port，不使用 UserDefaults 或只保存在视图中的状态。

### 2.3 信息架构与交互

1. 页面固定头：标题、默认对话选择器（支持 API / 本地）、“管理连接”、诊断次级菜单。
2. 模型库：搜索；来源筛选；模型行仅显示名称、所属连接/本机、可用状态、默认标记。默认设置与列表排序解耦。
3. 模型详情头：模型名、所属连接链接、明确“设为默认对话模型”动作、启用状态；删除放次级菜单。
4. 详情主体只保留清楚的两级：常用设置与高级设置。常用为名称、生成参数、上下文与输出限制；高级为协议能力、三档映射、任务用途/自动路由。连接 URL / key 不在模型表单内重复。
5. 参数在同一模型编辑上下文内编辑，或复用具备草稿绑定的参数 editor；不能是 parameters tab → button → 第二个独立持久化 sheet。新增模型也能先设置参数再一次保存。
6. 连接编辑是单一事务对象，保存只验证/写入该连接，保留已经实现的跨连接隔离/CAS。模型编辑若依托该事务，所有字段必须汇入同一 draft；禁止一个字段立即保存、另一个要点击保存却没有范围提示。
7. 保留真实导航目标 `connection(id)` / `model(connectionID, profileID)`；跨 dirty 边界提供保存并继续、放弃并继续、继续编辑。保存失败保留原草稿和目标。
8. 状态提示使用不改变主区域高度的固定反馈位/覆盖层；避免定时 banner 移除引发布局动画。大区域布局切换不做 opacity 叠放式双界面交叉淡入，保留所需 draft state 而非常驻两整套隐藏视图。
9. 窄窗口退化为模型列表与详情的单层往返；不要再添第二个外层侧栏。键盘焦点、滚动位置随对象 identity 保持；保存不重新选择列表第一项。
10. 本地模型的下载/启动/停止属于运行生命周期，和“设为默认”分成不同动作。正在运行另一个模型不应阻止设置后续继承提交的默认模型。

## 3. 设计反思与修正

### 不采纳的捷径

- “设为默认”通过把 priority 改成 100：会改变故障转移/其他用途路由；最高优先级并不能表达用户偏好。
- 直接使用 `profile.selected`：当前 API 保存和本地 select 会相互影响，而且不同代码使用顺序/selected 不一致。
- 只加一个全局 picker：如果工作区提交、已有会话和连接保存不消费同一解析规则，UI 只是另一份假状态。
- 将所有模型属性展开为一张大表单：看似减少层级，但把 URL/key、运行状态、权限和参数并列，更难理解。需要“对象归属”而非仅减少折叠。
- 每个字段自动保存：网络/凭据/协议存在联合校验；写一半会让模型短暂不可用。按一致的对象事务保存。
- 为避免不可用默认而静默选其他云提供商：可能改变数据接收方和费用。不可用必须显式反馈。

### 留下的风险与实现约束

- product setting 的读写失败不能伪装成功；保存后重新读取/发布变更通知，中心与聊天读取同一值。
- 默认存储如使用独立设置，测试必须证明目录重存/删除不覆盖该设置，也不会让无配置的新会话隐式选同名模型。
- 同时改全局默认时应采用现有版本控制可提供的冲突语义；不得整包覆盖其他无关设置。
- 连接编辑和参数编辑需共享草稿数据；若采用独立模型事务，明确显示保存范围并保证参数修订不会被旧连接快照覆盖。
- 现有敏感访问默认询问继续有效，重构不能为了减少开关而放开 Core 授权。
- 避免泛化成通用 settings framework；只抽取统一默认解析、编辑草稿、模型参数 form 等明确复用边界。

## 4. 验收要求（结果见第 5 节）

- API A/B、本地 L 同时存在，任意设置默认后新会话与 UI 一致；重启仍一致。
- 同名 API 模型分别属于不同连接，选择、参数、默认均按 profile identity 生效。
- 改默认不更改显式选择的既有会话/正在生成请求；新会话和明确继承的会话采用新值，继承状态有可见标识。
- 本会话参数覆盖优先，恢复继承后读取模型最新默认；三档映射只用于该次提交。
- 改连接 URL/名称、排序、采样参数不会改变显式默认；错误的另一连接不阻塞保存。
- 显式默认停用/删除/未安装：清晰缺失状态，不静默跨提供商；用户可一键重新选择。
- 连接共享字段对所有所属模型一致；模型独有字段不泄漏给同连接另一模型。
- dirty 状态点击另一连接下具体模型，保存/放弃后到达正确模型；取消保留当前位置与草稿。
- 新增模型可以配置参数后一次保存；保存错误保留编辑内容。
- 本地模型运行时仍能更改未来默认，不强制启动/停止模型。
- 中文/英文、紧凑/宽窗口、键盘导航、VoiceOver 标签；无瞬时错误提示、无保存成功 banner 抽走布局。
- 回归现有 revision conflict、连接保存隔离、canonical config roundtrip、会话参数和权限边界测试。

## 5. 实现与验证记录

### 5.1 当前实现

- `MacModelCenterView` 增加固定默认选择器，复用 `MacChatModelCatalog` 与 `MacDefaultChatModelStore`；显式标记、自动候选、失效引用和读写错误分别显示。默认写入使用持有的 revision；损坏配置修复通过 adapter 的 raw snapshot revision，仍走 checked set。
- API/本地页面改为条件渲染；中心持有 API 草稿 owner 与本地 downloader，不再把两整套页面叠放淡入。API 连接、修订、参数值、无效输入文字和精确导航目标在草稿 owner 中保留。
- API 详情改为常用/高级两层。参数直接绑定该连接的 pending parameters，新增模型也可编辑后一次保存；范围、种子与非法数字输入阻止保存，已有连接的 CAS 隔离提交保持不变。
- 默认徽标读取真实显式 profile identity；优先级仅出现在路由语境。模型详情提供默认动作及所属连接链接。本地运行选择改称“运行目标/设为启动目标”，与独立默认动作分开，设置默认不启动或停止运行实例。
- dirty 跨连接导航携带 `connection`/`model(connectionID, profileID)`/`newConnection`，提供保存并继续、放弃、继续编辑。失败保留草稿与目的地；通知反馈为固定 24pt 状态行，完整说明与重新加载动作在弹出层，取消定时移除与整页通知动画。
- `MacModelCenterEditingTests` 覆盖精确目的地、保留无效输入、参数纠正、真实 canonical 默认与 priority/停用分离。新增字符串覆盖八种既有语言，包括聊天菜单共享文案与 Settings 的默认入口。

### 5.2 验证中发现并修正的问题

参数首次读取发现修订冲突时，原先的局部“重新加载”会继续使用旧连接 revision，无法恢复；现已改为加载整个连接基线，有草稿时先确认。跨连接放弃冲突草稿清除当前连接的阻塞状态，不把 A 的冲突带到 B。紧凑窗口“管理连接”明确打开详情；默认选择器补充独立 AX label/value；只有参数变化也显示模型行未保存标记。管理器支持注入 canonical application；真实 NSWindow/AX 操作测试覆盖键盘输入参数后保存并到达 B、实际 CAS 冲突保留草稿后放弃到达 B，以及未编辑参数冲突后的整连接重新加载，均已通过。

来源重挂载：本地检查对象、搜索与紧凑详情状态迁至中心持有的小型 owner；下载器仍独立保持。两种来源的模型库以对象 ID 绑定 scroll position，API 模型/连接详情以规范对象 ID 隔离视图身份。真实窗口测试输入本地搜索、点击模型、移除并重建来源页面、返回列表后检查搜索及检查对象仍保留，已通过。同一对象保存不改变详情 ID、不重选第一项，也没有显式重置 scroll/focus 的代码。主动切换来源会卸载详情，不自动夺回键盘焦点；精确像素位置与选区不跨来源另建缓存。此项区别于同对象编辑期间的稳定性，尚无宿主 caret/像素滚动断言，不声称已验证。

真实窗口暴露 View.init 修改已被中心观察的草稿 owner，会触发 SwiftUI “Publishing changes from within view updates”；目录水合已移至挂载后的 task，并保持来源重挂载不重读草稿。视觉复核确认固定 96pt 留白过大，已改为上述 24pt 状态行。“Parameter Profile” 原目录为空，已补八语言翻译。AX 保存动作实际完成数据库写入与精确跳转后，因节点同步销毁返回 false；测试以实际状态/数据库结果验证动作。放弃按钮查找原先误中下层同名工具栏按钮，改为优先精确 ID，名称回退仅限附属窗口原生按钮。这些修正已通过下列集中重测。测试辅助仅在当前测试进程启用并恢复 AppKit accessibility，不改变系统辅助功能权限或窗口安全层级。

### 5.3 自动化验收证据（2026-10-09）

集中日志 `/tmp/nola-chat-sealed-reveal-finalbatch-20261009.log`：409 个测试、1 个跳过、1 个失败。唯一失败为 Journal 测试的启动 fixture 超时，发生在正文验证前；**不能把该整批称为全通过**。下列模型相关套件独立结果均为通过：

| 需求 | 当前证据 | 边界 |
| --- | --- | --- |
| canonical 默认、CAS、损坏配置修复、Rust 解析 | `MacDefaultChatModelStoreTests` 3 项 | 不在 UI 另写解析器 |
| 精确导航、同连接草稿、非法输入、保存/放弃、冲突重新加载、来源重挂载 | `MacModelCenterEditingTests` 8 项，其中 4 项真实窗口交互 | 继续编辑的目标保留有状态测试；未单列真实窗口取消动作 |
| 同名模型不误配、显式会话不受默认改动影响、不可用发送拦截、目录不读无关凭据 | `MacWorkspaceDefaultModelTests` 3 项 | 非真实外部提供商网络验收 |
| Settings 默认摘要与精确深链、本地摘要不加载 API 配置 | `MacSettingsDefaultModelTests` 2 项 | 与中心共享 presentation/catalog |
| 模型中心 API/本地紧凑和宽窗口可渲染 | `MacSecondaryPageVisualEndToEndTests` 1 项，680×720 与 1040×760 | 渲染成功不等同完整人工视觉验收 |
| 八语言目录与中文标题 | `MacSecondaryPageCatalogCoverageTests` 6 项 | 不能代替 EN/ZH 独立进程截图或 VoiceOver 听读 |

已有连接事务、canonical roundtrip、会话参数、路由与权限相关回归包含在集中批次中；各项结果以原日志为准，不用旧测试总数替代新增行为证明。

### 5.4 中英文视觉与提交边界补验（阶段 2 收口）

英文使用独立测试进程 `-testLanguage en -testRegion US`，日志 `/tmp/nola-ui-completion-gates-en-20261009.log`；模型编辑 8 项、目录覆盖 6 项、语言 4 项、页面导航 3 项、页头导航 1 项、中心宿主渲染 1 项、详情截图 1 项均通过。此批总计 40 项，有一个非模型 UI 的 terminal-recovery 测试产生两个断言失败，不能宣称整批通过；该测试随后在中文批次重跑通过。

中文使用 `-testLanguage zh-Hans -testRegion CN`，日志 `/tmp/nola-ui-completion-gates-zh-20261009.log`；中心渲染、详情截图和上述 terminal-recovery 重跑共 3 项全部通过，退出码 0。最后的删除次级菜单修改已包含在这两轮编译及截图中。

实际查看的图像包括 `/tmp/nola-model-detail-en-dark-1000.png`、`/tmp/nola-model-detail-en-light-400.png` 及中文 `/tmp/nola-model-detail-dark-1000.png`、`/tmp/nola-model-detail-light-400.png`。宽窗深色详情与窄窗浅色详情中，常用/高级设置、生成参数、Parameter Profile/参数预设均正确本地化，无重叠或截断；窄窗显示单层返回入口，删除位于省略号菜单，固定状态位未恢复为原先的大块空白。fixture 模型显示名中的中文与连接名 Home server 是用户数据，不应翻译。

从两轮 xcresult 导出的中心原图和 manifest 位于 `/tmp/nola-center-evidence-en/`、`/tmp/nola-center-evidence-zh/`。已查看两种语言的 API 紧凑页与本地宽页：固定默认选择器、自动无候选提示、继承说明、管理连接、来源切换均可见；本地运行目标与默认偏好明确区分。空 API 目录与尚未安装本地模型的 fixture 会禁用默认按钮，这是可用性规则的预期表现，不据此宣称已验证有凭据的真实云服务。

新增提交边界证据：英文批次 `MacDefaultModelSubmissionBoundaryTests` 2 项通过，覆盖坏全局设置阻止继承、显式会话仍捕获精确路由、Host 最新默认 revision 在 HTTP 前拒绝过期提交。`MacLocalRunTerminalContractTests.testChangingDefaultKeepsActiveInvocationBytesAndNextInheritedRunUsesNewProfile` 与 `testUnavailableSessionAndDefaultModelsBlockBothLocalAndAPISubmissions` 通过，补足运行中配置冻结、下次继承提交与 local/API 不可用组合拦截。

### 5.5 非阻断的验证边界与可选增强

- 默认入口提供独立 AX label/value，真实窗口测试执行原生输入与 AX 保存/放弃操作；这是无障碍契约及动作验证，不等同操作系统 VoiceOver 的人工听读。未要求开启系统语音、修改辅助功能权限或解除安全屏蔽。
- 同一详情保存保持规范对象 identity，未发现显式 caret/scroll 重置；可增补保存前后精确选区/像素断言。主动来源导航不强制恢复旧焦点，以免抢走新操作位置；不为此新增泛用焦点缓存。
- 本地正在运行时更改默认的 UI/存储路径不调用 start/stop，运行配置冻结已有测试；真实下载模型长时间运行与外部提供商网络连通性不在本轮 UI fixture 验收范围。
- 中英文截图、八语言目录、事务冲突、精确导航和默认提交边界已关闭本轮必需门禁。上述人工增强与精确像素断言不应被表述为已执行，也不构成已知功能缺陷。

## 6. 二次反思：精确持久化、解析与并行实现契约

本节保留实施前确定的协议与约束；下文“建议/实现时”是当时的设计用语。当前已落实到 `core/nola-application/src/model_selection.rs`、`MacApplicationPort.putProductSettingChecked`、`MacDefaultChatModelStore` 和共享 `MacChatModelCatalog`；验证结果见第 5 节。它不是另一份尚未实施的新计划。

### 6.1 已验证的 CAS 能力

Rust 已有 `ApplicationCommand::PutProductSettingChecked`，字段为 key / value_json / expected_revision / updated_at_ms；`product_setting::put_checked` 在 durable receipt Unit of Work 内比较旧 revision。创建时 expected_revision 为 null，更新为正整数，不能传 0。冲突为 `product_setting_revision_conflict`。

`product_setting_editor_receipt_e2e.rs` 已覆盖冲突、冷重启和幂等 receipt。审计时 Mac 的 `MacApplicationPort.putProductSetting` 只调用无条件命令；现已补 checked adapter，不重造 CAS，不用“先读后无条件写”假装并发安全。Application Service 负责 durable receipts，adapter 继续走已有 execute，不绕开服务直写 SQLite。

### 6.2 精确数据结构

建议跨平台产品键：`chat.default_model`。此键语义是**该执行 Host 的默认对话偏好**，不是 Mac UI 私有偏好。当前本机中心只编辑本机 canonical store；连接远程 Host 时读取该 Host 自己的偏好，绝不能把本机设置直接套到远端目录。

值采用严格 tagged union，示例：

```json
{"version":1,"selection":{"kind":"profile","profile_id":"canonical-profile-id"}}
```

恢复自动选择：

```json
{"version":1,"selection":{"kind":"automatic"}}
```

- 未设置等同 automatic；JSON 损坏、未知 version/kind 是 configuration error，不能等同未设置。
- profile_id 是 `model_profiles.id`，API 和 local MLX 已处于同一 ID 域。Store writer 已禁止相同 ID 更换 provider/runtime，故不需要把 Swift `configured:`/`local:` view ID 放进产品协议。
- 不保存 API key、URL、模型名称、displayName、priority、Host endpoint 字符串。显示时通过 canonical profile 解析所属来源。
- 若此设置通过同步到另一主机且 profile/本地安装不存在，解析为不可用，不选择同名替代。不能将 schema 的 Host 所有权解释为“所有设备一定能运行同一模型”。
- 全局默认的 enabled/安装校验在选择时完成；提交时再检查。目录变化不自动重写偏好，保留可修复的原引用。

### 6.3 共享纯解析器与错误路径

以 Rust Application 为语义所有者，新增独立小模块（建议 `model_selection.rs`），而不是 Swift manager 私有 helper。纯函数输入：会话意图 inherit / explicit(profile_id)、上述默认配置、当前执行 Host 的模型候选事实（profile identity、可用于 chat、availability）。输出 tagged result：

- resolved(profile_id, origin: conversation / default / automatic)
- unavailable(profile_id, origin, reason: missing / disabled / not_installed / unsupported_runtime / not_allowed_for_chat)
- no_automatic_candidate
- invalid_configuration

自动候选可维持现有明确排序，但该排序不能影响 explicit/default 分支。传入候选顺序是自动排序事实，不允许客户端通过列名称/语言关键词偷偷重排。无副作用、无 profile.selected 写入、无模型启动、无权限授予。

Swift 只映射 UI identity ↔ canonical profile identity、提供本机安装事实并展示结果；同一个解析输出服务于中心默认说明、工作区当前模型、发送前检查和路由 primary。短期可在 Swift 定义同等纯模型以桥接，但发布实现不得留下两个互相独立的算法；实现验收必须验证 Rust 契约与 Swift bridge roundtrip。

`selectedModelOption: Optional` 可以保留为 View 的便利投影，但必须增加非 Optional `modelSelectionResolution`；发送入口先 switch resolution，unavailable/error 直接可见反馈并不发模型请求。禁止继续使用 `selectedModelOption?.… ?? configured.first`。`MacAppleChatConfigurationStore.routePlanThrowing` 对传入显式 profile 找不到/不健康时返回明确错误，不得退到 healthy.first。

对于已明确允许的自动故障转移，可在冻结 primary 后按现有协议/参数兼容规则建立 fallback；它不得倒过来改变默认，也不能用 nil primary 触发。此次 UI 重构不扩张故障转移授权。

### 6.4 selected 的处置边界

当前 `model_store.rs` 的 selected 选择位混合 provider catalog 活跃项和全局偏好；仅添加产品键后不应还让任何聊天入口依赖它。保留的用途必须缩小为 provider/runtime 内部的运行/目录活动项，或在后续统一删除；不能称它为 chat default。

若本轮保留该字段，必须把 upsert 的 displacement 限于同 provider/runtime，和 replace 的范围不变量一致，并回归 API 保存不改变本地 profile revision/运行目标、本地 select 不改变 API selected。否则新设置虽正确，目录保存仍会产生无关写和潜在 revision conflict。该变更由领域实现负责人统一处理，不允许 UI 层补偿回写 selected。

### 6.5 可并行的文件所有权

| 工作包 | 独占主要文件 | 交付契约 |
| --- | --- | --- |
| 领域默认与 Rust 契约 | 新 model_selection Rust 模块与 command bridge；model_store selected 范围；MacApplicationPort；新 Mac default selection adapter | typed snapshot(read revision)、checked set、resolution，无 SwiftUI |
| 工作区接入 | MacWorkspaceModel、MacAppleChatService 路由/目录保存、MacConversationModels 必要的 identity bridge；相关测试 | 不静默 fallback；会话继承与显式；每次 run 冻结 |
| 模型页面 | MacModelCenterView、MacRemoteModelManagerView、MacLocalModelManagerView、MacModelParametersView、ModelCenter.xcstrings | 只使用领域 adapter；统一默认入口、草稿事务、对象归属；不直写产品键 |
| 验证/设计记录 | 此文档、独立新增测试文件 | 结果逐项记录，禁止把旧测试总数当新功能验收 |

并行前先锁定 adapter 方法/DTO，领域代码负责人负责 ports 文件；UI 作者不要自行在同一文件增加另一套读写。聊天闪动修复完成后再启动模型实施，遵循用户优先级。

### 6.6 必须补充的领域验收

- stale revision 默认保存拒绝且不改变读值；两次相同 receipt 不产生两次变更。
- profile ID 含冒号、同名/不同连接、同步到缺失模型 Host，均不错误匹配。
- 默认保存不改变 profile.selected/排序/参数/运行状态。
- profile.selected 的 provider/runtime 操作不改变其他作用域记录修订。
- 显式不可用四种来源（会话/默认 × local/API）发送前失败，不发生 provider HTTP 请求。
- 缺失设置允许自动；坏设置报错；两者不能混同。
- 运行中的请求捕获配置后更改默认，当前请求字节/模型身份保持不变，下个继承会话提交再解析。
