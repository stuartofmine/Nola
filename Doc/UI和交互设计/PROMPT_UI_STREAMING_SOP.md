> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 流式 AI 对话界面工程 SOP

## 1. 目的与适用范围

本 SOP 适用于包含以下能力的 AI 对话产品：

- 流式文本回复
- Markdown 或富文本渲染
- 推理过程与进度状态
- 工具调用、搜索、文件操作或代码执行
- 桌面端、移动端和 Web 多端实现

目标不是尽快把 token 放到屏幕上，而是让用户持续感受到同一个助手正在完成同一项任务：响应立即建立、状态容易理解、内容逐步显现、结构保持稳定、过程可以追溯、完成前后没有跳变。

本文同时定义实现契约、测试方法和发布门禁。只验证最终字符串、只测试 Markdown parser、只检查完成态截图，都不能证明流式对话界面正确。

## 2. 设计原则

### 2.1 稳定优先于速度

用户更容易接受稍慢但连续、可预测的反馈，无法接受闪烁、重排、消息消失或正文突然倾倒。传输速度和呈现速度应解耦。

### 2.2 语义优先于 token

网络 token 是传输单位，不是视觉单位。界面应按完整短语、句子或 Markdown block 提交内容，而不是逐 token 重绘。

### 2.3 状态是属性，不是新消息

等待、推理、工具执行、正文生成和完成是同一次回复的不同状态。状态变化应原位更新，不应创建新的消息或替换整棵视图树。

### 2.4 内部协议不得泄漏到用户界面

内部 phase、函数名、调试标签、原始 JSON 和错误堆栈只进入日志或可展开详情。用户界面只消费结构化、可本地化的展示状态。

### 2.5 视觉验收必须覆盖过程

流式体验的问题发生在状态转换之间。验收必须观察等待、推理、工具、增量正文、排空和完成，而不是只看最终结果。

## 3. 术语与状态模型

### 3.1 核心术语

| 术语 | 定义 |
| --- | --- |
| canonical content | 已从模型或 Agent 收到的完整内容，可能包含未闭合 Markdown |
| presented content | 已通过稳定性门禁并提交给界面的单调递增内容 |
| transport completed | 上游不再发送增量 |
| presentation completed | 呈现队列已经排空，最后一个视觉批次已经建立 |
| stable node | 后续增量到达时可以复用身份、位置和状态的界面节点 |
| semantic batch | 可独立阅读且不会因下一个字符改变基本语义的最小呈现批次 |
| Trace Journal | 按序持久化规范化运行事件、用于重放和缺口恢复的追加式记录 |
| semantic no-op | 事件有效且推进序号或游标，但不会改变任何用户可见投影 |

### 3.2 推荐状态机

```text
idle
  -> pending
  -> reasoning
  -> tool_running
  -> streaming
  -> draining
  -> completed

任意运行态 -> failed | cancelled
```

状态含义：

- `pending`：用户已提交请求，但尚无可展示的上游事件。
- `reasoning`：助手正在分析，可能有规范化后的进度说明。
- `tool_running`：一个或多个工具正在执行；它可以与 reasoning 或 streaming 并存。
- `streaming`：canonical content 正在增长。
- `draining`：传输已结束，但 presented content 仍在按节奏追赶 canonical content。
- `completed`：传输与呈现均完成。
- `failed` / `cancelled`：使用独立终止策略，不复用正常完成路径。

## 4. 常见根因与禁止修法

出现闪烁、重复、顺序错误、Markdown 延迟渲染或正文突然出现时，优先检查以下根因：

1. Host 将内部事件字符串直接交给 View，View 再用字符串猜测阶段。
2. pending、reasoning、tool 和正文分别创建消息，首个增量或完成时又更换消息。
3. 标题、详情、最新事件序号或完整文本 hash 被当作节点身份。
4. streaming 使用 raw text，completed 切换到另一套 Markdown renderer。
5. 每个 transport token 都触发 Markdown 解析、布局和滚动。
6. transport completed 后直接显示 canonical full content，绕过尚未排空的呈现队列。
7. 对整个气泡使用 transition、blur、位移或尺寸动画，导致旧内容反复参与动画。
8. 工具事件触发正文自动滚动、推理卡重建或整个气泡重新测量。
9. 主题、字体或可访问性设置未进入渲染缓存 key，导致跨主题复用错误颜色或布局。
10. 测试只检查编码前位图、最终字符串或静态 View，产生运行态假阳性。

禁止用以下方式掩盖根因：

- 在最外层增加 debounce
- 完成后整体去重正文
- 用固定延时等待 UI “稳定”
- 完成态重新创建一套富文本视图
- 用更长动画隐藏布局抖动
- 只增加最终态快照而不重放真实事件序列

修复应落在事件契约、状态机、presentation reducer、稳定节点 diff 或缓存身份上。

## 5. Host 与界面的结构化事件契约

推荐链路：

```text
Host event
  -> normalized run event
  -> conversation reducer
  -> semantic stream assembler
  -> stable presentation model
  -> platform renderer
```

### 5.1 推荐字段

| 字段 | 语义 | 展示规则 |
| --- | --- | --- |
| `runID` | 本次回复的稳定标识 | 关联消息、活动和工具，不直接展示 |
| `phase` | 当前运行阶段 | 通过本地化 key 映射，不显示原始值 |
| `activityKind` | reasoning、searching、reading、writing、executing 等有限枚举 | 决定图标、标题和允许的详情格式 |
| `activityTitleKey` | 本地化标题 key | 运行期间保持稳定 |
| `activityDetail` | 可选的人类可读进度 | 规范化且实际变化时才发布 |
| `toolCallID` | 工具调用稳定标识 | 原位更新 queued、running、completed、failed |
| `assistantDelta` | canonical content 增量 | 进入 assembler，不直接驱动 View |
| `transportCompleted` | 上游关闭信号 | 进入 draining，不能立即替换成完整正文 |
| `sequence` | 同一 Trace 内单调递增的事件序号 | 驱动幂等、缺口检测和恢复，不直接展示 |
| `eventID` | 事件的稳定唯一标识 | 重放去重，不作为界面节点身份 |
| `conversationID` / `messageID` | 会话与回复归属 | 隔离并绑定同一个 assistant 气泡 |

### 5.2 归一化规则

1. 内部函数名、协议 phase、调试标签和原始工具输出不得成为用户可见标题。
2. pending 使用通用本地化 key，例如 `assistant.processing`；品牌名如需出现，应由本地化参数注入，而不是写死在状态机中。
3. reasoning 期间主标题固定，例如“思考中”；工具开始或结束不得改变主标题。
4. reasoning 完成后可显示“思考过程”等完成态标题，但不把最后一个内部步骤当作副标题。
5. activity detail 更新前执行 trim、空白归一化和等值判断。内容不变时不得发布新的 presentation state。
6. tool result 可以进入工具详情，但不得再次作为 assistant 正文追加。
7. 长任务的 phase、推理、工具、参数、资源和结果必须投影在同一个 assistant 气泡中；不得分散为全局状态栏或独立消息。
8. 工具运行中展示清洗后的参数和资源；完成时保留真实、清洗且截断的结果或错误，不得用通用完成文案覆盖。

## 6. 稳定身份与 Reducer 不变量

### 6.1 身份来源

| 层级 | 稳定身份来源 | 禁止做法 |
| --- | --- | --- |
| assistant message | 用户发送时立即生成的 message ID | 首个增量、工具事件或完成时重新生成 |
| activity/reasoning | `runID + activity lane` | 使用标题、详情或最新 event index |
| tool call | `toolCallID` | running 与 completed 分别创建工具卡 |
| Markdown block | 持久的语义 block sequence ID | 使用完整文本 hash 或每次 parse 的临时下标 |
| attachment | 持久 attachment ID | 发送后用路径或显示名重新匹配 |

### 6.2 运行时不变量

1. 用户发送后立即创建 assistant message，不等待首个网络增量。
2. 一次回复从 pending 到 completed 始终使用同一个 message ID。
3. activity title、detail、状态和展开状态是属性，不是身份。
4. tool status 只原位更新同一工具卡。
5. 已封口 Markdown block 的顺序、文本和 ID 不得因后续事件改变。
6. 重复事件必须幂等。
7. tool start 不得重置 reasoning、正文、展开状态或滚动状态。
8. assistant delta 只更新 canonical buffer 和 assembler，不覆盖历史正文。
9. transport completed 只改变传输状态并触发 drain，不复制完整答案。
10. streaming 与 completed 使用同一个正文组件和同一组节点身份。
11. 会话恢复后保留原消息和工具身份，历史内容不重新播放入场动画。
12. semantic no-op 必须推进 reducer 序号和持久化游标，但不得发布新的 presentation state。
13. 检测到 sequence gap 时必须缓存后续事件并从 Trace Journal 补齐缺口，不得跳过缺失状态继续展示。
14. Journal 重放必须与实时消费得到相同投影，并隔离不同 conversation、run、message 和 tool call。

## 7. 流式 Markdown 稳定前缀

### 7.1 双缓冲模型

每条生成中的回复至少维护：

- `canonicalContent`：上游已收到的完整正文，允许包含未完成 Markdown。
- `presentedContent`：已经通过稳定前缀门禁并按节奏提交的内容。

界面只渲染 presented content。presented content 必须单调增长，不能回退或替换。

### 7.2 可安全提交的内容

- 已配对的行内强调、链接和行内代码
- 已结束的标题、列表项、引用、分隔线
- 已闭合的 fenced code block
- 已具备表头、分隔行和完整数据行的 GFM table
- 不会因下一字符改变 block 类型的普通段落前缀
- 已形成最小可读语义的短语或句子

### 7.3 必须等待的尾部

- 未闭合的反引号、强调、删除线、链接、图片或括号
- 只有表头、尚无合法分隔行的潜在表格
- 未闭合的 fenced code block
- 仍可能变成标题、列表、引用或代码围栏的行前缀
- 过短、尚无独立语义的 token 尾部

### 7.4 解析和 diff 规则

1. parser 可以对 canonical content 重算，但 presentation diff 必须复用语义相同的旧节点。
2. 已封口 block 原位保留，只允许尾部 block 增长或追加新 block。
3. streaming、draining 和 completed 共用同一个 Markdown renderer。
4. 不得先显示 raw Markdown，再在完成后整体切成富文本。
5. 整段输入、逐字符输入、固定小块输入、随机网络分块和 burst 输入必须得到一致的最终 block 类型、顺序与文本。
6. 渲染缓存 key 必须包含会影响输出的主题、字号、动态类型、语言方向和可访问性参数。

## 8. 传输完成与绘制收尾

传输接收、权威正文和短暂绘制动效是三个时间点，不能用视觉队列拖慢正文：

1. 接收内容后立即更新 accepted/visible 正文及布局；Apple 的共享 scheduler
   仅以 16ms 合并当前最新快照，不建立逐字待输出队列。
2. `transportCompleted` 后不会再有上游增量；终态正文仍经同一个增量 renderer
   收尾，只解析可变尾部，不替换气泡、message ID 或已封口 block。
3. `presentationCompleted` 表示最后一批 glyph 的绘制已清晰；Apple 单批最长
   360ms，从各批实际收到时开始，不能累计成几秒的排空延迟。
4. 取消、失败和 incomplete 保留已接收内容，状态依据 Core 的真实终态；
   不用动画计时伪造任务完成，也不清空可阅读的公开推理摘要。
5. 验收分别记录首 delta、上游 completed、最后正文布局及最后动效清晰时间；
   组件回放不是实际网络时序，必须明确区分。

## 9. 渐进显现动画

Apple 两端共同使用 `AppleStreamingInlineText`，只对新增渲染字符的 range
播放水面上浮波形，不逐字延迟提交，也不对整个气泡反复淡入：

| 参数 | 当前值 | 目的 |
| --- | --- | --- |
| 快照合帧 | 16ms | 同一帧采用最新内容，避免队列积压 |
| 单批 glyph 波形 | 360ms | 形成连续、可感知的浮现 |
| 位移 | 3pt 向上归位 | 仅绘制，不改变布局 |
| blur | 0.65pt 向 0 收敛 | 新字由柔和到清晰 |
| opacity | 0 向 1 收敛 | 波头沿本批新增字移动 |

后续批次到达可以叠加，旧 range 的起始时间不变。修订、结构替换和历史打开
立即可读，不重播旧文。tool start/finish、reasoning 折叠和 completed 不重播
正文。Reduce Motion 立即清晰显示，移除位移与 blur。代码、表格、公开推理
摘要复用同一渲染规则；平台只保留测量、滚动与键盘差异。

### 一手参考与取舍

[Streamdown 2.2](https://vercel.com/changelog/streamdown-2-2) 将流式动画作用于新词，
并处理未完成标记，支持这里“增量局部显现、旧文不重播”的选择；Nola 保留
原生共享 glyph 绘制，不引入 React 包。[assistant-ui Text](https://www.assistant-ui.com/docs/api-reference/primitives/message-part)
的 smooth / Reduce Motion 设计作为无障碍参考；不照搬会使网络完成后仍长期排空
的文字积压机制。这里的动效参数属于绘制层，不改变模型接收速度和 Core 权威正文。

## 10. 推理状态与工具调用

1. assistant 气泡在发送后立即存在，pending 状态显示稳定状态灯和本地化处理文案。
2. pending 转 reasoning 时更新同一 activity 容器，不插入第二个状态条。
3. reasoning 主标题在运行期间保持不变。
4. detail 应完整显示或提供明确的展开能力，不能被固定高度无提示截断。
5. detail 未变化时不得发布新的 View state，即使底层开始了新工具调用。
6. 工具卡默认显示友好名称、状态和当前动作；参数、结果、文件路径和具体操作在展开后显示。
7. queued、running、completed、failed 使用同一张工具卡原位更新。
8. 工具和推理区域的更新不能改变正文 block 的身份、位置或动画进度。
9. 完成态应自动收起非关键过程，但必须允许用户重新展开查看历史。
10. 运行中的展开详情展示清洗后的参数和资源；完成详情展示真实、清洗且有明确截断标记的结果或错误。
11. 通用的“完成”状态只能作为视觉状态，不能替换实际结果、文件路径、URL、查询或产物链接。

## 11. 自动跟随与输入区域层级

1. 自动跟随目标只由最后一个已提交正文 block 决定，不包含 status、reasoning 或 tool ID。
2. 用户主动向上滚动后暂停跟随；回到底部或主动点击恢复后才重新开启。
3. 不为每个 token 启动滚动动画。新语义 block 提交后复用一个可中断的平滑事务。
4. 最新正文应停在输入区域上方的安全距离内，而不是每次跳到窗口最底边。
5. 输入区域背景必须不透明，位于消息层上方；消息滚动到其下方时应被遮挡而不是透出。
6. 输入区域高度变化时只更新必要的 bottom inset，避免与正文动画反复测量。
7. 会话切换时先恢复该会话的 scroll/follow state，再接受新事件。

## 12. 本地化与可访问性

1. UI 使用本地化 key，不直接展示 Host 字符串。
2. 品牌名、工具名和动态对象使用参数插值，避免拼接导致语序错误。
3. 运行态标题应保持稳定，语言切换不能改变 activity identity。
4. 支持从右到左语言时，图标、进度、代码和表格方向需分别处理。
5. 大字体下不得截断关键状态和工具动作；必要时允许卡片自然增高。
6. 状态不能只依赖颜色表达，必须同时有文本、形状或辅助功能标签。
7. Reduce Motion、Increase Contrast、屏幕阅读器和键盘导航必须纳入验收矩阵。

## 13. 分层测试策略

### 13.1 Parser 测试

覆盖标题、段落、强调、链接、图片、列表、嵌套列表、引用、代码围栏、表格和混合语言标点。每个样本至少使用：

- 整段输入
- 逐字符输入
- 固定小块输入
- 带固定随机种子的随机分块
- 最后一个 delta 与 transport completed 紧邻的 burst 输入

断言最终 block 类型、顺序和文本一致，并断言未闭合结构不会提前泄漏 raw syntax。

### 13.2 Reducer 测试

至少驱动完整序列：

```text
send
  -> pending
  -> reasoning
  -> tool running
  -> assistant deltas
  -> tool completed
  -> transport completed
  -> draining
  -> presentation completed
```

每一步断言 message ID、activity ID、toolCall ID 和旧 Markdown block ID 不变。额外覆盖重复事件、tool result 重放、取消、失败、主题切换和会话恢复。

Reducer 测试还必须断言：

- semantic no-op 接受事件并推进 sequence/cursor，但 presentation 发布次数保持不变。
- sequence gap 出现后，后续事件不进入可见投影；从 Journal 补齐缺口后按序继续。
- 实时消费、完整 Journal 重放和从任意 cursor 恢复得到相同投影。
- 相同 toolCall ID 在不同 conversation 或 run 中互不污染。
- 工具完成保留真实截断结果，不被通用状态字符串覆盖。

### 13.3 真实 UI 更新测试

测试必须在同一挂载实例内连续更新。原生端使用真实 hosting/runtime，Web 使用真实 DOM 与浏览器事件；禁止为每个阶段重新 mount。

至少采集：

1. 用户发送后、Host 尚无响应
2. reasoning 首次出现
3. detail 不变但新工具开始
4. 第一段结构化 Markdown 已出现、传输尚未结束
5. transport completed 且仍在 draining
6. presentation completed

必须断言：

- 同一个 assistant 气泡始终存在
- reasoning/tool 更新前后稳定节点身份不变
- detail 不变时像素和布局不闪烁
- Markdown 在传输完成前已经结构化
- draining 不是空白，也不是完整正文瞬间出现
- completed 后标题、段落、表格和代码区域仍完整可见
- 主题切换后前景色、表格和工具卡仍可读

### 13.4 截图与像素测试可靠性

1. 断言必须检查最终将被保存或附加的同一份编码图像，而不是另一份编码前位图。
2. 对可变图形缓冲执行深拷贝，避免后续帧覆盖已经记录的证据。
3. 保存后重新解码图像，并检查关键区域不是全黑、全白、透明或空白。
4. 关键区域至少包括状态区、正文标题、表格、代码块和输入区域。
5. 自动断言通过后仍需人工打开最终导出的图像检查。
6. 截图等待只能等待明确的状态条件，不能用任意 sleep 掩盖竞态。

### 13.5 明确禁止的伪验收

- 只断言最终字符串相等
- 只在 completed 后截图
- parser 测试通过后宣称流式 UI 已通过
- 为每个状态新建 View 再比较静态结果
- 只检查元素存在，不检查身份、顺序、布局和中间状态
- 只检查编码前位图，不检查最终导出的附件
- 只在 preview 或 mock 组件中验收，不运行真实 reducer 与 renderer

## 14. 跨平台验收矩阵

每次修改流式渲染、气泡、推理状态、工具卡、滚动或输入区域后，至少完成下列矩阵：

| 客户端类型 | 运行环境 | 外观与尺寸 | 必拍阶段 |
| --- | --- | --- | --- |
| 桌面原生端 | 实际 window/hosting runtime | 浅色、深色；窄窗口、宽窗口 | pending、reasoning、tool、streaming、draining、completed |
| 移动原生端 | Simulator/emulator；发布前补真机 | 浅色、深色；窄屏、大字体 | 同上，并检查键盘与 Reduce Motion |
| Web | 正式应用 host 与真实浏览器 | 桌面、移动 viewport；浅色、深色 | 同上，并检查 DOM 节点未被替换 |

固定测试样本应同时包含：

- 二级标题
- 强调和链接
- 至少三项列表
- 引用
- 至少三列的表格
- fenced code block
- 长段落和混合语言标点

除 Markdown 样本外，还必须维护一个版本化 canonical Trace fixture，至少包含 pending、reasoning、长任务 phase、带参数和资源的工具开始、增量正文、带真实截断结果的工具完成、usage、validation 和 terminal 事件。所有客户端必须用同一 fixture 驱动生产 adapter、reducer 与 renderer；禁止各端自行构造语义相近但字段和顺序不同的替代样本。

人工检查字体层级、行距、表格边界、工具卡尺寸、气泡背景、滚动跟随和输入区域遮挡关系。

## 15. 发布门禁

以下条件必须同时满足：

- [ ] pending 使用本地化产品文案，无内部 phase 泄漏
- [ ] message、activity、tool、attachment 和 Markdown block 使用稳定身份
- [ ] detail 不变时，新工具事件不会造成推理区域闪烁
- [ ] streaming、draining 与 completed 共用同一个 Markdown renderer
- [ ] Markdown 在传输完成前已经按稳定前缀结构化显示
- [ ] transport completed 后队列继续 drain，没有正文瞬间倾倒
- [ ] 新节点动画不影响旧节点、气泡尺寸和工具卡位置
- [ ] completed 后没有残留 mask、blur、transform 或不可见正文
- [ ] 工具详情可展开，文件操作包含路径、动作和截断结果
- [ ] 长任务的 phase、参数、资源和结果均在同一个 assistant 气泡内投影
- [ ] 运行中可查看清洗参数/资源，完成后保留真实且有截断标记的结果
- [ ] semantic no-op 推进序号和游标，但不产生 UI publication 或节点 mutation
- [ ] sequence gap 会阻止后续状态展示，并能通过 Journal 按序恢复
- [ ] 实时消费、刷新/重连恢复和完整 Journal 重放得到一致投影
- [ ] 所有客户端通过同一版本化 canonical Trace fixture
- [ ] 自动跟随只响应正文，输入区域不透明且层级正确
- [ ] 深色、浅色、窄屏、宽屏、Reduce Motion 和大字体均有运行态证据
- [ ] 主题与可访问性参数参与渲染缓存身份
- [ ] 测试包含完成前中间状态，没有使用 final-string-only 作为通过依据
- [ ] 最终导出的截图经过重新解码、自动断言和人工检查

命令通过只是必要条件。只有 parser、reducer、真实 UI 更新、运行环境截图和人工视觉检查五类证据齐全，才可以发布。

## 16. 缺陷复盘模板

每次出现“测试通过但 UI 仍错误”时，记录：

1. 用户可观察到的现象
2. 能稳定复现问题的真实事件序列
3. 被破坏的状态机或身份不变量
4. 根因位于协议、reducer、presentation、renderer、动画、布局还是测试证据
5. 原测试为何产生漏测或假阳性
6. 新增的最小失败测试
7. 修复前后同阶段截图
8. 已验证的客户端、主题、尺寸和辅助功能配置
9. 尚未覆盖的环境与残余风险

最终汇报应分别列出 parser、状态机、真实 UI、设备或浏览器运行、截图和人工视觉检查证据。任何一层都不能替代其他层。
