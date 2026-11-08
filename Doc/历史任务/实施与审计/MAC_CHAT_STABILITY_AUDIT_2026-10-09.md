> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Mac 对话稳定性审计与设计候选（2026-10-09）

状态：阶段 1 的审计、设计反思、实现与专项验收完成；当前证据及验证边界见 §7。§1–6 保留实施前发现与实施过程，不代表当前仍待批准或未测试。整批失败和不改源码的重跑分别记录，不将分批证据写成首次全绿；Rust/Swift 警告阶段由总执行记录单独跟踪。

## 1. 审计范围与证据

读取当前工作树，不以之前的“测试通过”回复作为当前 UI 正确性的证据。检查 Journal 消息、流式 Markdown、Transcript 行生命周期、原生滚动控制器及其测试。未修改生产代码，未运行编译，未读取或写入用户凭证。

### A. 同一消息在两个结构容器间迁移（确定的结构缺陷，高优先级）

`NolaMac/Features/Chat/MacTranscriptView.swift` 将 `messages.dropLast()` 放在 `LazyVStack/ForEach`，末条消息则放在独立条件分支。新增用户消息或助手消息后，原末条消息从独立分支转入历史容器。`.id(message.id)` 只能稳定所在结构中的身份，不能把两个不同父容器中的 SwiftUI State 拼接成同一实例。

下游 `MacJournalMessageTextView` 的 `parts/loadedReference`、`MacJournalTextPreview` 的 Markdown 投影、活动展开选择、短语 reveal 状态全是视图 State。重建时 Journal 首帧没有 parts，随后异步水合恢复高度，因此可能出现空白、跳变、重复 reveal 与用户展开状态丢失。这是渲染生命周期缺陷，不是调动画时长可解决的问题。

### B. Journal 已有内容但可见性仍由旧投影决定（确定的策略缺口）

`macChatMessageIsVisible` 仅认活动消息、角色、`message.text` 和旧 `presentation.hasVisibleContent`。与之相反，`MacMessageView.hasAssistantResponse` 已将 `journalReference != nil` 视为存在回复。运行结束后，空 legacy text 的 Journal 消息可能被窗口策略过滤，即使 Journal 中已经有思考或正文。显示入口与内容所有权的判定不一致。需要直接以 canonical Journal 所有权支持已进入窗口的终态消息，且明确空取消消息如何呈现。

### C. 高频游标通过 task 取消/重启驱动水合（可由代码确认的竞态风险）

`MacJournalMessageTextView.task(id: ReadKey(reference: ...))` 在每次 cursor 更新时取消读任务；字段 body 则又以 root/generation/isFinal 为键开启独立读任务。取消检查能防止旧值提交，但没有保证正在到来的下一帧不会反复取消第一帧读取。当存储或远端读取慢于 token 更新，可能到终态才能看到完整结果；当前不能仅凭源码断言用户这次确实触发了饥饿。

metadata 成功时立即调用 `onActivitiesLoaded`，旧 trace 被过滤，而对应 body 仍在下一层异步读取。因此控制层“已覆盖”与显示层“已准备好”也不是同一个时刻。需要明确就绪契约，而不是认为拿到 metadata 就代表内容已绘制。

### D. 滚动的意图模型正确，但没有完整生命周期验收

`MacTranscriptScrollController` 已区分 latest/reading/locating，用户手势有代数栅栏，异步布局合并为一个主队列任务。这应保留。风险在 A 导致锚点注销与重新注册、以及 SwiftUI layout 和 AppKit frame 通知各自推进时序；不能再引入固定延迟 scrollTo 或让几何变化修改用户意图。

`MacTranscriptScrollObserver` 有 detach 代数栅栏与观察注销；审计未发现需随意重写 observer 的确定证据。应以真实 NSHostingView/NSScrollView 生命周期测试覆盖 mount/unmount、下一轮发送及终态收尾，而非只测单独 controller 数学。

### E. 正文动画本身已避免双层 Text，仍应禁止历史重播

`MacStreamingPhraseGroup` 已让同一个 Text 承担布局和绘制，使用 glyph reveal，不应退回 overlay 双层布局。`MacStreamingBlockRevealModifier` 只在 onAppear reveal；A 中重新 mount 会把已出现的流式块再次归入新块。另一个风险是未走 Journal 的 `MacMessageView` 在完成时从 semantic nodes 切成整份 `MacMarkdownView`。需要列出是否仍有生产路径使用此分支；如有，统一生命周期；如已不可达，删除不可达兼容展示分支而不是继续维护双渲染架构。

## 2. 设计候选

1. **稳定的、受限的行呈现所有者（推荐）。** 将 Journal 已发布 parts/reference、已读取有界 body、Markdown 投影和用户展开选择放进会话窗口持有的 row presentation owner，以 canonical message identity 为键；SwiftUI 行只是 observer。当前 hybrid 尾部/历史布局可以保留来控制内存，但移入历史时复用同一个 owner，不重新清空与水合。缓存只覆盖当前有界消息窗口，退出窗口/会话时取消工作并释放；每字段仍采用现有 64 KiB 读取上限，禁止缓存全量正文。reveal 的已见身份/时间线也必须属于稳定 owner 或由 owner 给出明确的历史抑制标记，不仅迁出文本状态。
   - 不采用简单“全部 eager VStack”。主任务提供的已有性能证据：此前 eager 方案切换 RSS 约 216 MB，超过 64 MB 门槛；hybrid 约 41 MB。该门槛不能为通过 UI 测试而放宽。上述数字来自主任务交叉审计，本子任务未重跑。
   - 替代方案：单一 ForEach 配合已测量高度的离屏 placeholder 实现显式虚拟化。它能保持逻辑身份，但需要处理宽度变化、阅读锚点及未测量行估算，风险比稳定 owner 更大；本轮不优先重建一个通用虚拟列表。
2. **统一内容所有者。** 可见性基于 canonical message/Journal；终态只是 seal 同一行，不移除正在展示的合法 Journal。reasoning 始终在正文上方；展开状态归该消息/Part，不归临时网络请求。
3. **读取与通知分离。** 同一 owner 一次最多一个 hydrate。游标通知仅更新 desired cursor；当前读取成功先发布仍有效的单调快照，再读取最新 desired。只在 owner 改变、退出页面、用户重试时取消。禁止通过每个 token 取消读任务来实现 latest wins。字段读取同样保持上一帧直到新内容原子就绪。显示层消费已解析视图状态，不将正文水合完成与 metadata 就绪混为一谈。
4. **无布局动画的内容更新。** 行集合/水合/终态变更不继承父层结构动画；短语 opacity/blur reveal 保留。用户主动展开思考属于独立交互，不让 token 更新重置用户选择。
5. **单一滚动权威。** 保留原生 controller，只在 layout 完成后协调 latest 或 reading anchor。终态不改变 scroll intent；下一轮发送是显式的用户意图事件，规则单独规定；文本到达不是用户手势。

## 3. 反思时必须回答的问题

- 稳定 owner 的 residency 是否有明确上限？现在“加载更早”可不断增大 visibleMessageLimit；不能把所有历史 owner 永久留在全局字典。要区分窗口内轻量身份/用户选择与实际驻留正文，至少验证 20/40/长消息窗口成本，以及切换会话释放与 RSS ≤ 64 MB 原门槛。
- 高频读取队列如何防止 stale owner、取消后的误发布、错误重试死循环？需要固定 owner generation 与游标单调断言，且明确失败后保留上一帧。
- 仅 metadata 已就绪时显示思考标题并非错误，但不得让已显示的思考正文消失再回来。
- 保留真实事件记录与“思考固定顶部”展示策略的分离，不改写数据时间顺序。
- 已完成历史消息打开时不 reveal；当前消息同一新增 node 只 reveal 一次；用户关闭 Reduce Motion 时不重播历史。
- 用户滚动中插入上方内容如何保持同一文本位置？保留可视锚点，不用整个文档相对百分比。
- 完成时 metadata HStack 从无到有会增加约 31 px（主任务交叉审计）。应为时间/复制操作保留稳定 footprint，未完成时隐藏且不可交互、不可被辅助功能聚焦；不能只将完成时滚动修正当成解决布局跳变。

## 4. 实施前确定的验证门槛（当前结果见 §7）

- Hosting 测试：已展开含思考 Journal 回复 → 追加用户行 → 追加下一助手行；旧正文不出现空帧，展开状态不变，行 state 生命周期不重启。
- 慢读取测试：读取耗时大于通知间隔，持续增量期间仍能提交中间帧，终态全部呈现；切换会话不混入旧正文。
- 终态矩阵：普通完成、失败、取消、无正文、只有思考、延迟思考、重连与历史恢复；合法 Journal 行不被旧 text 判定隐藏。
- 原生滚动：跟随增长、终态高度收缩、手动上翻、思考展开、下一轮发送、切换会话及返回最新；无空白视口，用户阅读锚点不漂移。
- 动画：同一 node 首次可见渐清晰；历史/重载不重播；Reduce Motion 无 reveal；表格代码不改变先前测量布局。
- 性能：受限窗口长正文/千万字符存储、连续十轮新增、滚动历史；确认不会自动全量水合或无界累积可视树。
- 在整体实现完成后统一执行相关 Swift/Rust 验证，记录命令、准确测试范围、日志路径及未覆盖项，不能用纯策略单测宣称视觉验收通过。

## 5. 本阶段结论

第一优先是让呈现状态不再依赖双父容器的临时视图身份，并统一 Journal 可见性；第二是把异步读取生命周期从 token 通知中拆出来。应保留 hybrid 已证明的内存优势，不以无界 eager/global cache 换取稳定。滚动补偿和 reveal 参数应在这些结构问题修复后验收，不能继续作为替代根因修复的补丁。

## 6. 经主任务设计反思批准后的实现记录

主任务批准记录在 `MAC_UI_REDESIGN_EXECUTION_2026-10-09.md`。以下是统一构建前的实现检查点；当时尚未验证。后续集中回归、实际帧/截图检查与残余边界已记录于 §7，以该节作为当前验收结论。

- `MacJournalMessageTextView.swift`：引入会话 transcript 持有的 `MacJournalPresentationResidency`、`MacJournalPresentationOwner` 与 `MacJournalBodyState`。metadata 和字段读取各自单 worker 串行推进 desired snapshot，owner 改变时取消并用 generation 栅栏拒绝旧结果。coverage 与 parts 同次发布；字段始终最多读取两个 32 KiB range。
- 强复用缓存上限 16 个 row owner，每个 owner 8 个 body prefix。另有不持有对象的 weak registry：已挂载行/字段持有自己的 owner/state，因此超缓存上限不能取消可见读取或制造空 owner。退出消息窗口及页面时显式取消；无进程全局正文缓存。上限约束的是离屏复用缓存，不假称可见内容树不存在内存成本。
- 活动展开选择、正文投影、已经显示过的 node 身份与 coverage 迁至 owner。Preview 用 `@State attachedState` 一次绑定，避免每次读取属性推进 LRU。Markdown renderer 的初始化接受已出现 node 集；同一节点跨父容器重建不 replay，新节点继续 reveal。
- `MacTranscriptView.swift` 保留 hybrid layout，注入 owner、清理窗口成员，统一 Journal-only 终态可见性。
- `MacMessageView.swift` 保留 metadata footprint，完成前控制不可点击、辅助功能不可聚焦；不在完成时新插入一行。

新增验证：缓存上限以外 24 个已持有 owner/field 保持引用不变；50 次通知 burst 不取消第一个有效读取且只补读最新游标；跨 owner 重用拒绝旧在途结果；Journal-only 终态窗口可见；真实 Application fixture 写入 reasoning/text，NSHostingView 从末行父容器移入 LazyVStack，正文 hydration、用户展开选择、reveal 身份及可见文字高度保持。原 metadata 完成测试改为验证固定行高度，不再接受新增约 31 px 的旧行为。

## 7. 01:36 用户反馈：追加显现、终态空白与空间利用

### 审计

- 结构化 Markdown 只有块首次挂载时的 reveal，块内持续追加未参与动画；短语的任务键只有节点 ID，同一 mutable tail ID 内增长也不能重新安排新增文字显现。这是动画身份与内容版本混淆，不应通过重建整个文本节点解决。
- Journal 预览把 reveal 条件限制为 `hasStreamed && !isFinal`，终态标记先于最后一次异步正文水合时，末批新文字直接跳出。应保留“此 owner 曾经历实时流”的事实，历史加载仍不动画。
- 原生滚动使用 document bounds；该尺寸可能包含 lazy 历史的估算高度。用户主动返回最新后，终态重排是否使 viewport 留在内容之外，必须用真实 transcript 和实际末行位置验收，不能只测 documentHeight 相等。
- 宽度上限 1,040 pt 与 composer 960 pt 不一致；比例断点还会导致窗口增宽时列宽反而减少。正文 15 pt 在大窗口偏小。

### 设计与反思

1. 单个原生 Text 同时负责测量和绘制；只给新追加的可见字符安排渐清晰、轻上浮，先前字符不重新动画。保持完整 Markdown 解析上下文，不拆开链接/粗体语法来制造片段。Reduce Motion 直接显示，历史重挂载不重播。
2. 完成只封存流状态，不结束尚未显现的末批文字；动画不能延迟数据提交、改变测量高度或让正文换树。
3. 滚动沿用 latest/reading 意图与用户手势栅栏，优先使用实际尾部边界；不以全量 eager 布局、固定延迟滚动、持续强拉底部规避问题。保留长会话性能门槛。
4. transcript/composer 最大宽度统一 1,200 pt，使用连续的可用宽度约束，紧凑窗口保持边距；正文 16 pt、用户文字 15 pt。不是无上限铺满窗口，仍保证可读行长。

### 实施与验证

- `MacMarkdownView.swift` 统一用完整 inline Markdown 解析结果构造 attributed pieces / 数学附件，按新增可见字符范围分配 reveal 时间，稳定 ID 下的段落、标题、列表、表格和代码追加均不再依赖 `onAppear`。一个原生 Text 同时负责布局与绘制；0.32 秒、5 pt 上浮、1.8 pt 初始模糊，历史立即显示，Reduce Motion 禁用显现。
- 明确分离 `contentIsFinal` / `isStreaming`（内容解析终态）与 `animateNewNodes` / `animatesAppends`（绘制资格）。参数必填，不用 nil 或旧调用推断兼容。实时完成后的正文与重新打开历史采用同一最终解析规则，末批文字仍允许收尾动画。
- `MacTranscriptLatestAnchor` 提供 eager footer 的实际边界。跟随定位与用户滚动距离共用该边界；已建立锚点暂时重挂载时保持当前位置，不退回 lazy document 的估算末尾。没有扩大历史渲染窗口或改成全量 eager。
- transcript/composer 上限统一 1,200 pt；列宽连续增长，用户气泡也不得超出极窄可用宽度；正文 16 pt、用户文字 15 pt。

统一回归证据：

1. `/tmp/nola-chat-reveal-scroll-finalbatch-20261009.log`：382 项，0 失败，1 项明确 opt-in 系统文件选择器跳过。
2. 加入解析终态分离和不透明截图后，`/tmp/nola-chat-sealed-reveal-finalbatch-20261009.log`：409 项，1 跳过，1 失败。唯一失败是 `testJournalPublishesTextBeforeTerminalAndDoesNotReplayAtCompletion` 在创建源服务时 `startup timed out`，发生在写入正文之前；不能把该整批记为全绿。
3. 未改代码、未放宽超时，原样重复该测试 3 次：`/tmp/nola-chat-startup-unchanged-retest-20261009.log`，3 次全部通过。启动延迟根因尚未确定，不归因于已证实的 UI 缺陷，也不宣称服务启动问题已修复。
4. 新增 6 项 append reveal 测试全部通过：真实窗口 active/settled 像素对比、旧前缀像素稳定、同 ID/公式旁追加、终态末批显现、Reduce Motion、未闭合标记完成后与历史重开一致。主任务导出并查看了终态的模糊/清晰两帧，确认不是通过空白画面获得像素差异。
5. 新增 2 项滚动测试全部通过：原生 stale extent / footer remount / 用户阅读，以及真实 Journal append→finish、点击实际返回最新按钮、composer clearance 变化和 resize。640/1,440 pt、明暗主题的最终文字像素均断言可见；截图人工检查确认最后一段位于视口内。测试宿主须设置一致的不透明背景，避免 alpha PNG 被查看器错误呈现为白色空框。
6. 原有 Markdown 26 项、Journal 34 项、长会话性能两项、模型编辑 8 项和静态布局/resize 回归均通过；本批没有再次出现模型页面的 `Publishing changes from within view updates`。

边界：尚未录制用户同一模型会话的现场重放；动画进行中若整行迁入另一父容器，仍以已见 node 身份避免重播，未显现的少量尾部可能直接完成。已完成后重挂载、正文保留和滚动定位有测试；本轮不把这一残余动画细节描述成已获得逐帧连续性保证。
