# 新原型跨平台 UI 实施

目标：按照当前原型和 UI 设计更新 Web、iOS、macOS、Android 及 CLI 适用的交互。开始：2026-10-28；进行中，不以配色或局部组件完成代替整体完成。

基线：[交互规范](../../docs/design/nola-ui-prototype/docs/UI-INTERACTION.md)、[视觉规范](../../docs/design/nola-ui-prototype/docs/DESIGN-SPEC.md)、[完整能力矩阵](../../docs/design/nola-ui-prototype/FEATURE-COVERAGE.csv)。覆盖矩阵中的管理能力同样属于本任务，不只修改对话页。原型模拟数据/权限/回复不进入生产；操作接既有Core端口，业务缺口须明确记录，不能用假成功替代。

## 实施及验收范围（2026-10-29 当前结论）

下表只汇总已取得的当前证据。后文保留逐批失败及修复过程；旧“尚未构建/待测”描述属于当时记录，不能覆盖这里的最新结果。完整差距见[原型验收审计](UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md)。

| 工作线 | 当前实现与证据 | 尚未关闭 |
| --- | --- | --- |
| 视觉基础 | 四端中性色、专家八色及当前生产组件已接；Mac新批40/40、Android当前生产16/16；Web类型0错误/0警告、593项通过 | 所有尺寸/主题/字体/RTL组合尚非全覆盖；不能拿死Room页截图验收 |
| 手机导航 | 真实Today快照、独立会话列表/返回、专家二级管理有原生操作证据；iOS CurrentWork六路现工具导航与原草稿已实际走过 | 所有管理表单取消/冲突、Today深业务待补 |
| 桌面工作区 | Mac/Web唯一顶栏、内容标签与工具分工、独立owner草稿及阅读状态；已有实际窗口/Chrome局部交互 | Chrome已恢复实际新建/指令/面板与普通及双专家任务点击；全按钮、主题/尺寸和长任务矩阵仍待补齐 |
| 导航所有权 | 保留owner页面及请求/DOM身份；Mac真实切换草稿/锚点；PDF新owner与异步token竞态已修并原生回归 | QuickLook通用阅读位置、全部二级页逐操作仍待证 |
| 检查器 | 真实上下文/执行/参与者、进度及Goal投影已接；Web有界只读Git diff | 移动端工具组织与深审阅/产物业务见下行 |
| 消息与专家 | Mac生产父锚点27项回归；iOS两真实expert均sealed，实际滚动全文4212/3928字、姓名/独立草稿/再次kill恢复通过；Android真实思考571个可见增量样本 | 原序 Part/Surface 共享修复与新原生桥回归通过；Android 原20行长任务仍失败，全端混合失败/远程组合尚不全 |
| 消息动作 | Core普通及完整专家重发；Mac4个真实sealed Runs；Web普通与双专家的当前Chrome真实点击/编辑重发已完成（新capture来源与旧缺见证拒绝样例分别记录）；Android普通r7与专家r8真实编辑重发已过，核全文/新owner/原source不变及Activity重建 | sealed失败专家“新尝试”未实现，不能把原投递恢复称失败重试；iOS编辑重发仍需真实操作；本轮新授权代码未被旧结果覆盖 |
| 工作工具深业务 | 真Git diff、附件/工具产物引用、现Goal事实保留 | finding/gate、产物版本编辑比较、深Goal验收关联及Today自动整理未实现，不由客户端模拟 |
| 二级管理 | 专家管理入口四端可达；Mac40条原生回归；Android实际创建/模型模板/归档编辑恢复/取消已过；iOS管理Cancel/Discard及Resume精确刷新focused回归已过；Web共享可读目录与管理保存确认已接 | 模板捕获授权已完成本批平台编测及各自范围真实验证，详见验收审计；其余管理表单/冲突矩阵尚未全部实测 |
| CLI | 实际PTY发送/帮助/退出/原会话恢复；新产物英文恢复标题及原答复已实际核对；路径4项与delegated4项回归通过 | 未声称CLI全部用例通过；旧Goal恢复等剩余失败另核，不用共享套件绿覆盖 |
| 共享与总验收 | Core目录2/2及Web GET/HEAD；共享精确OS死亡证明12项SQLite+3项CLI通过，iOS实际在旧租约到期前124秒恢复原Runs；Apple Responses公开思考协议回归通过；Android当前660项JVM（2跳过）与16项production通过 | 共享有效参数查询42个不同用例通过，原生 caller/本机及源Host baseline 尚未接完；深业务及全平台验收缺口仍在，全goal active |

最新窄批状态：共享原序/真实Rebase投影13个单元累计通过及14项真实Store回归通过；当前状态枚举23/23。Web完整593/593、零跳过、类型零错误/零警告、生产及交互构建通过；Chrome普通及双专家编辑重发真实完成，阅读modal实际发现重代理/CAS故障后修复，A驻留/B重开/换owner关闭均实际通过，隐藏重激活/关闭后焦点恢复也已真实点验。iOS新reader真实XCUITest1/1与两hosted2/2通过；Android新JNI focused13/13、20个不同Compose及原A12只读1/1通过。Mac真实Store sealed/交错状态已通过，两个reader经实际AX标识隔离与准确原生控件定位后r40均通过（不是77项整批重跑）。普通终态canonical见证已补代码及回归，真实Chrome验收因窗口消失仍待；专家卡片自己上下文的可信提交、完整历史读取、原长任务失败及管理深业务仍未关闭。全goal保持active，详细失败与证据见验收审计。

验证必须使用真实生产组件与隔离数据。先比对要求和代码，再更新实现；不能以不存在的自动日报/任务/权限状态填满原型布局。用户此前明确不做减少动态效果专项，本轮不为该专项扩展工作。

## 首批修改

- Apple两端已有语义颜色资源改为统一中性底/正文/辅助文字；iOS现有真黑canvas保持。
- Web主题去除旧基础青绿偏色、普通正文改16px、桌面控件基础触区36px，手机44px保留。
- Android主题改中性色层级，bodyLarge采用17sp/28sp；平台Material交互保留。
- 尚未修改导航与消息结构，不能宣称原型落地；当前仅Web主题AA检查1/1通过。

## 首批验证记录

- Web Svelte检查0错误/0警告：`/tmp/nola-ui-prototype-check.log`。首次生产构建通过，仍有既有chunk大小警告；最后控件outline调整后的生产构建也通过：`/tmp/nola-ui-prototype-web-build-final.log`。
- 首次全量535/536，一项测试硬编码旧配色；现改为直接读取实际CSS颜色并计算文本/控件对比度，发现并修正dark outline不足。最终536/536、0跳过：`/tmp/nola-ui-prototype-web-tests-final.log`。
- Apple两端语义色资源的明暗正文/辅助文字/主动作对比计算通过；不是原生窗口或asset编译通过证明。
- Chrome通过CUA启动后进程可见，但两次getApp均cgWindowNotFound、browser inventory为空；未取得真实页面，不记视觉或按钮通过。此阻点不阻止继续修改导航结构。

## 并行实施与统一验证门槛（用户追加）

用户要求三个subagent并行，所有修改和主代理审计结束后才统一编译/测试。当前iOS、Mac、Android分工已启动；root负责Web、CLI及共享协议。自本要求起没有新启动编译或测试，先前已启动的Web进程均确认exit0；其542项结果只适用于该批较早代码，不覆盖后续管理标签/位置端口/专家颜色改动。

Web当前源码改动：单48px对话顶栏、对话常驻首标签和PDF/管理页内容标签、左三类别仅切左列表、管理页独立URL位置端口、保留挂载的对话/编辑页面、局部预览显式转标签、移除PDF的确认与标签同步、320–480检查器参数。专家完整外部身份与八色预览/保存/取消已接线，配色只落气泡。以上均等待整批静态审计及最后统一验证，不宣称当前生产已通过。

已确认共享业务缺口：CanonicalBranchIntent只复制原历史前缀，不能直接实现新规范的改写消息/仅从改写消息起的新分支/原模型专家快照重发。各端编辑提交不得用当前选中配置或重新复制聊天伪造成功；root继续补真实共享准入。单专家已sealed Failed与原投递恢复是不同语义，现有幂等submit不能被当作新尝试。

### 并行批次静态审计进展

- iOS、Android：Today真实事实、独立会话列表及二级管理入口已修改；Today查询排除远程镜像、核对精确Run/会话owner；加载、错误、取消、后台及迟到回写已静态审阅。Android补齐生产只读查询分类，避免测试stub绕过真实门禁。
- macOS：单48px工作栏、每会话内容标签、五工作工具及更多已接既有数据；普通/专家阅读锚点与文字文件详情位置已修改。PDF/QuickLook位置还未覆盖。root审计发现远程附件本地路径读取风险，已要求补源Host门禁。
- Apple专家颜色已抽至`Shared/Presentation/AppleExpertAppearance.swift`，共享八色、默认blue及单一JSON Data存储协议；删除两端重复定义/旧key/FNV，不做迁移。
- Web：外部消息操作、收进更多菜单的选择动作、700px手机边界、四个已接真实数据的工作工具（概览/产物/进度/目标）、顶层popover更多菜单；管理标签稳定ID、独立阅读位置、异步回写owner及独立关闭确认已修改。管理编辑器切换会话仍会卸载，当前必须确认未保存内容；尚未达到“编辑器草稿完整保留”的最终要求。
- Web审阅仍缺可信Git reader；上下文/执行/参与者的最终工具分类、手机Today与独立会话页仍需实施。不得把四主工具称为五工具全部完成。
- 精确编辑重发：新会话只从编辑后的用户消息开始，不复制任何历史前缀。现有CanonicalBranchIntent不应用于此。需Core原子读取原发送模型/专家配置见证，准备全新owner/身份/权限后才复用现有普通或专家准入。各端只有独立编辑器和诚实不可用状态，尚未接通真实重发。
- Core同snapshot Today聚合、封存回答/专家新尝试重试、版本化产物与审阅finding/gate接口仍缺；不得以客户端当前配置、旧Run/checkpoint、幂等投递恢复或虚构结果顶替。

此段记录全部为实现/静态审计状态。用户要求后的编译、测试、模拟器、真实模型验证仍未启动；早期542项通过不覆盖这些修改。CLI及能力矩阵整体验收也未完成。goal保持active。


### 后续共享能力与手机 Web

三代理继续分工：Android代理负责Core精确编辑重发 producer/prepare/consume 的可证闭环；iOS代理负责Core同一读事务Today facts及窄只读Automation port；Mac代理完成远程文件来源修正后负责CLI适用交互。中央Application命令及平台接线由root统一，避免各端再建Runtime。

Web已新增独立`/conversations`生产页面，复用现有会话管理器的真实搜索/新建/进入逻辑，Chat手机返回入口指向该列表；Host静态路由一起注册。Today首屏与新的共享facts端口仍在实施，不把这个列表当完整手机导航已验收。


### 当前共享收口（未验证）

Core Today已实现同read transaction聚合普通/子任务权限和问题、queued occurrences、独立全局recent_conversations、显式分页。iOS/Android已删原N+1编排；Web适配器只读同一Application snapshot。不能把needs_response分页后的会话自行排序伪称全局最近。

普通编辑重发已实现Core typed producer→prepare→consume首版：原发送模型见证、源消息revision、事务幂等/回滚、空新owner、私有凭据reference隔离。可信Host fresh policy/model-only credential重建及Apple/CLI/Web真实派发仍在接线。专家缺完整选定集合的原子准入快照，仍明确拒绝，不当普通消息丢弃专家后重发。准备结果不等于发送/完成。

本轮已经发现并修复的来源风险包括Mac远程附件映射到本机路径、Today旧父会话漏子任务等待、分页后误称全局最近、generic product_settings暴露私有重发材料。最终静态审计及统一编译/单元/协议/真实UI/真实模型验证均未结束；不以未运行的回归源码当测试证据。


### 最后一批实现与主审门槛（当前状态，覆盖前述阶段性缺口）

- 三代理持续并行：Apple/Shared与Web专家入口、Core专家上下文占用与Mac/Android调用、Core重发证明与Web只读专家增量投影。root审计中央命令、工作区导航/保留页面、来源隔离和任务记录。
- Today四端统一读Core同事务快照，包含普通等待、owned task和专家上下文；全局最近独立查询，不能由等待分页推断；无法关联的真实专家工作明确提示需关注。
- Web管理标签按完整Host/会话/标签owner保留挂载，独立URL、请求生命周期、DOM/ARIA身份、草稿及阅读位置；隐藏owner的原生弹窗暂停，重新选择时按实际展示顺序恢复。关闭才销毁，跨会话切换不再用丢弃确认代替保留。
- Web审阅已有当前可信本地Host工作区的有界只读Git reader，单独展示staged/unstaged真实diff；远程会话不能读本地同名路径，超限/超时不是clean。没有新增提交/推送权限，也没有模拟finding/gate或产物版本接口。
- 普通重发由Core持久原模型见证和新目标，可信Host只取原模型材料、重新建立工具/权限与当前系统指导。实际一次构造的submission与config一起证明，并由Store核组件摘要和既有input admission digest；原正文、旧历史、资源凭据和授权不能被偷偷带入或在证明后替换。
- 专家发送采用Core一次完整集合准入；编辑重发采用原集合/模型/指令定义见证生成完整新计划，逐child真实准入后才导航外层会话。正在修正selected到Run创建间的上下文占用，以及factory失败但Run未创建时的精确重试规则。完整组完成读取也必须同一读事务，不能把不同时间的child状态拼成成功。
- Web专家正文原先仅全历史轮询，当前在改用既有AssistantJournalReader/RunObserver的只读增量投影，保留同一顺序、思考和终态验证协议；不是另建A2UI Runtime。

仍需如实验收：统一编译/单元/协议测试、真实GUI点击和模型任务、窄屏/大字体/长名/思考与长输出、停止/权限/未知投递恢复、跨会话草稿及弹窗隔离。远程child缺可信源模型/指令见证时，完整专家重发明确拒绝，不把投递receipt伪造为发送快照。版本化产物、审阅finding/gate等原型模拟能力未因此成为生产功能。以上未运行的回归源码不计为通过证据。


### 统一验证已启动

三路实现与root静态审计结束后，统一开始编译/测试。首轮Web暴露导入路径、声明顺序和页面module泛型问题，已修；也移除只覆盖死侧栏的测试，改当前生产组件及完整owner DOM身份。

| 范围 | 本轮实测结果 | 证据与边界 |
| --- | --- | --- |
| Web类型/Svelte检查 | 0错误、0警告 | `/tmp/nola-ui-20261028-web-check.log` |
| Web自动测试 | 561通过、0失败、0跳过 | `/tmp/nola-ui-20261028-web-tests.log`；含真实生产组件SSR和协议/状态回归，不等于浏览器点击 |
| Web生产构建 | exit0 | `/tmp/nola-ui-20261028-web-build.log`；仍有单chunk超500KB构建提醒 |
| Chrome真实操作 | 组件交互首批通过；完整产品/模型待测 | 用户打开窗口后CUA已恢复。真实Chrome点击模型选择、保留工作区owner校验，页面报告unique ids/label/aria、独立草稿、native suspension/restoration、inactive late show、显式关闭及owner-only abort通过。移动断点弹层关闭/重开保留输入，Escape恢复Open panel焦点。地址为隔离组件harness `127.0.0.1:5183/review-tests/tests/interaction.html`，不作为完整产品或真实模型证据 |
| Core/CLI、iOS、macOS、Android | 进行中 | 各自隔离测试数据与构建目录；原生及真实模型结果待实际完成后记录 |

没有把原型演示状态、未运行源码、SSR或编译成功记作真实模型/点击验收。goal保持active。

Chrome后续实际点击：`dialog.html`运行隐藏父容器确认框→取消，页面返回“PASS：可见、取消返回 false、退出模态、卸载清理”；`a2ui-streaming.html`发布卡片→后续思考→Reasoning delta，实际文本更新“后续思考：新增片段”；Text delta后出现“最终正文”，Complete后展开Execution process可同时回看首轮/后续思考及Publish UI Completed。以上为生产组件与显式fixture输入的浏览器验证，不冒充真实模型输出。

真实验收准备发现四端共同遗漏：精简导航后专家picker仍可选专家，但创建/管理目录入口已不可达。已补各端picker二级管理入口，沿用Core `configure_agent`与directory/agent精确revision；不恢复独立Room创建/开聊。Web移除无人调用的旧`AgentRooms.svelte`整套聊天页，改`ExpertManager`仅管理定义；保留编辑defaults和未确认写入身份，归档使用当前协议`archived`，不伪造删除命令。Web补丁类型检查0错误/0警告，实际完整产品点击待新Host可执行产物。

iOS首轮编译修复缺Combine导入与Mac旧draft参数后，iOS专项41条实际通过（`/tmp/nola-ui-ios-20261028.Oa9wmz/contracts-retry1.xcresult`）；UI/真实模型尚未完成。CLI完整测试首轮栈溢出中止，独立NOLA_HOME/逐用例/单worker复核中；不计为全套通过。

2026-10-29后续真实验收：

- 完整Web隔离Host `127.0.0.1:5187`已由本批新`target/debug/nola`启动，仅使用临时NOLA_HOME/DB。Chrome真实点击创建新会话、更多→参与者→管理专家，创建/编辑/归档/恢复均回到实际目录；关闭管理后picker显示更新名称。颜色选择取消后仍蓝色，保存紫色后重开选中紫色。`/`候选→`/experts`插入→Enter直接打开参与者；`@`候选选择后清空查询并显示单个所选专家chip。均非DOM脚本代点。
- 默认qwen连接独立真实请求30秒超时，Host真实回合sealed failed，未计通过。另一条已配置Responses连接直接请求HTTP200且返回专用marker；仅临时配置切换默认后，生产Web client→真实Host/Store/模型→verified terminal普通回合通过。编辑重发真实被Core拒绝`edited_resend_model_differs_from_prepared_intent`，正定位冻结字段差异，未放松守卫。证据`/tmp/nola-validation-20261028/web-real/protocol-secondary-report.json`；API验收不冒充Chrome推理点击。
- Core关键真实Store事务回归29/29通过：agent_room_contexts14、edited_resend_transaction_e2e6、expert_message_transactions9（`core-final-focused.log`）。此前fixture2,048输入预算小于真实完整请求，已按其原64k模型配置48k预算，未削生产guard。
- Android持久AVD实际instrumentation14项中13通过，含专家创建并Core读回、IME/草稿/新会话/模型菜单/设置；余1旧fixture发送已删除finish wire，正在改为现协议lease不能制造成功的断言。截图`/tmp/nola-validation-20261028/android-screenshots`，不是模型推理证据。
- Mac真实模型/UI两用例实际2通过/0跳过，含专家703结果与长流式60条/会话切换/草稿/锚点；后置runner把2cases错当最后artifact需2Runs，仍需修验收汇总，不将后置失败涂绿。实际日志`/tmp/nola-mac-real-memory.mPmYUo`。单独29项contract仍在修AX测试helper和旧导航断言。
- CLI单worker/独立NOLA_HOME基线2241通过、48失败、8忽略；query主模型误受公开delegation端点限制已窄修并开始转绿。默认测试线程栈溢出与其余热cursor/恢复失败尚待收口。所有失败保留，不称全平台通过。

真实重发进一步定位（待修复后复测）：安全诊断仅输出schema字段名，差异为`output_admission.configured_context_window`。CLI新submission入口错误丢弃已由可信工厂从原始见证恢复的模型output admission，随后按当前默认重算。现仅prepared operation和typed frozen capability均存在时保留该model admission；Host memory policy仍无条件新建，原模型声明capacity与原输出额度一致，fresh上下文预算重算，不带旧历史/用量/checkpoint/阈值。回归`fresh_resend_preserves_model_output_limits_but_rebuilds_host_memory_policy`，等待集中编译及完整真实重发。

视觉证据审计更正：Android所报8种Room明暗/字体/宽度图实际直接挂载已无生产caller的`AgentRoomsScreen`旧行组件，撤回这些图的当前产品覆盖表述；真实MainActivity管理/IME/草稿/模型菜单路径仍有效。Mac `MacRoomAndConnectionVisualTests`同样直接capture旧独立Room页。两端正在删除死页面并迁移视觉用例到当前生产专家回复/管理组件；Android当前专家回复全文名在38dp侧列，大字体会竖排，已定位并修为正文上方宽列单次身份。保留旧结果作为审计证据，不用它们充当新UI通过。


### 2026-10-29 最新证据（覆盖上方同项阶段性状态）

- Web真实生产client API→隔离Rust Host/Store→实际配置模型的完整验收四阶段均通过：普通回合verified terminal、普通编辑新owner且原文保留、两个专家完整组实际成功、两个专家编辑新children且原manifest不变。报告`/tmp/nola-validation-20261028/web-real/protocol-current-report.json`，失败数组为空。此为真实API/模型证据，不冒充Chrome点击推理。
- 对应修复未放松任何Core守卫：trusted resend factory恢复原模型output admission，重建当前Host memory policy；prepared Web user保留canonical ID；专家PromptTemplate只进入一次context管线；原完整manifest冻结正文经现Host私有capability传入，未扩展普通Renderer定义注入能力。
- Mac真实验收最终为4个answered sealed Runs的普通/专家编辑重发，以及独立长流式332状态/26个AX文本长度；报告`/tmp/nola-mac-ui-validation-20261028/FINAL.md`。旧2cases汇总错误已更正，不沿用早期失败计数。
- iOS普通实际encrypted Web Host路线通过真实read_file、5次递增输出、唯一最终气泡与冷恢复；`/tmp/nola-real-mobile-host.WhIP5f/ios-real.xcresult`，同DB实际Core终态/正文/tool核验通过，错误marker负测被拒绝。此结果不代替native cloud双专家，后者另行验证中。
- Android最新当前生产instrumentation16/16通过，日志`/tmp/nola-validation-20261028/android-production-16-current.log`。实际专家107个live/nonempty样本，最终UI全文1458字符与Core exact child一致；停止→新发送→Activity恢复也已通过。另一次ordinary20行已完整显示但模型base64解码答错，严格marker验收失败，保留该模型质量失败，不记UI丢尾也不把该普通项涂绿。
- Core最终共享套件：Application862通过/2忽略、Domain273通过、mobile bindings25通过、Store unit458通过/4忽略及集成通过。CLI完整早期失败尚不能由这些计数覆盖；本轮root四项路径/故障隔离回归实际4/4，另delegated恢复4/4，真实PTY发送/帮助/退出/指定会话恢复有单独证据，未声称全部CLI测试通过。
- Web管理进一步补现有custom模板目录选择；不另建store，不伪造Apple builtin。静审同时发现未确认编辑仅按ID读取会把旧定义误当成功，现要求更高revision和完整目标定义一致，保留不确定草稿。此批类型检查0错误/0警告、Web563/563（无skip）通过；新Rust目录边界用例与新Host实际只读接口仍待编译验证。
- 用户再次说明Chrome已打开/解锁后，CUA仍返回cgWindowNotFound，浏览器inventory为空，createBrowserTab(chrome)返回Browser is not available。没有补写新的浏览器成功记录，也没有重复要求解锁；此前实际Chrome管理/组件证据保留其范围。

完整原型差距见`UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md`：失败专家新尝试、review finding/gate、产物版本、深Goal关系、Today整理等不能靠客户端模拟补齐；现有能力的管理入口/阅读状态/移动工具组织正在分平台收尾。当前goal仍active，未宣称原型全量完成。

Web额外真实回归：原两个专家真实完成后，把当前目录的两者改名、改描述、清空选择defaults并归档，再编辑原消息；完整原模型/指令见证均存在，新计划`original`逐字段等于原manifest、两个child context/Run均为新身份，两实际Run仍verified成功。`protocol-archived-expert-report.json`两阶段通过，无失败。没有重开当前目录专家，也没有重跑成功子集冒充完整组。

模板目录新增Rust边界用例实际1/1（首条错误exact filter运行0条未计通过，纠正完整module路径后确实执行1条）；新CLI可执行产物编译exit0。新Host实际GET/HEAD读空目录通过，报告`prompt-catalog-http-report.json`。最初测试误期望loopback无cookie应401，实测200；核现router确认是既有localTrust策略，改按真实边界验证且未改变认证实现。该测试没有验证remote-peer认证。Web最终默认模板文案采用真实继承语义，不加None字段，原null/缺省仅metadata编辑保持原值。

目录复用收口：最终把Web目录解析/ReadContent判定移至Core `list_readable_expert_templates`，Web固定Host principal转发；Android复用安装现`assistant.resource_principal_id`。Core专项2/2实际通过，含真正SQLite Store/当前命令wire/read-only分类、不同principal不可见、metadata-only/错误资源revision不可见、重复UUID拒绝并保留原坏目录。此前CLI helper测试为中间证据，最终逻辑不在CLI重复维护。日志`core-expert-catalog-final.log`，最终CLI重建进行中。

iOS新增native cloud双专家真实验收尚未通过：已证明两真实Swift v8 HTTP调用、终态前public正文56→168、actual terminate/relaunch后同原Run继续增长；第一原child恢复后sealed且完整3992bytes，第二child在240秒窗口内仍running。观察到约134秒冷恢复执行等待叠加模型78.7秒/122秒生成，不通过提高timeout掩盖，也不因此放宽精确lease/owner守卫。该失败不抹掉此前ordinary encrypted WebHost成功，亦不允许后者冒充native双专家全部通过。

最终共享目录产物：`cli-bin-shared-catalog-final.log`编译exit0，新隔离Host实际命中Core只读query，GET/HEAD当前空目录与no-body均通过，报告`prompt-catalog-shared-http-report.json`。后续没有因目录功能重复发送模型请求。CLI新产物实际PTY恢复原`real-ui-cli-20261029`：英文Resume session标题、原用户与实际CLI_REAL_UI_CHECK_OK回答可见，输入/exit即时退出exit0，输出原session+project恢复命令；`cli-resume-current-report.json`。不声称忙时slash命令或所有终端状态均已覆盖。

Mac新批经root两轮主审后集中Test回归40/40通过、0skip（contracts-19）：模型/模板配置CAS及原context不变、共享selector字段helper、管理未保存取消/丢弃、原生PDF页/scale/destination/多次快速切owner，以及正文小字号。首轮contracts-18失败保留；实际发现的新owner PDF沿用旧页位置已修复。此批未新增真实模型证据，不拿原4Run代替新增菜单所有选项实际点击。

root视觉复核当前生产附件：导出contracts-19的原生管理/专家组件图（`root-mac-c19-attachments`），另目视Android当前MainActivity IME布局及320px/2.0字号专家组件。Mac管理明暗无表单裁切；图中透明header在黑底查看器不可见，RGBA确认alpha透明，不误报为真实黑字黑底。仍发现Mac专家48px姓名列把长名截成ellipsis，iOS38px姓名列也存在竖排风险；已重开此项，正抽共享Apple头像/全文姓名视觉组件，并迁移到气泡上方宽列只显示一次。原40/40不充当这个待改布局通过。

冷恢复新增共享只读process plan/OS证明校验并沿既有observed orphan事务；新增expiry逐值CAS，活/Unknown不接管，终态并发只由精确同Run/digest/binding的sealed读取确认。真实Store delivery套件12/12、mobile cargo check通过；CLI及原生实际kill/relaunch仍待最终验收。详见`COLD_RUN_PROCESS_OBSERVATION_20261029.md`。

Mac姓名收尾最终：`expert-name-25.xcresult/.log`同一受影响单方法1/1、0fail0skip，实际NSWindow明暗截图及AX全文姓名恰一次、heading宽>100；root已分别目视最终`expert-name-25-artifacts/current-expert-reply-{dark,light}.png`，确认全文无ellipsis、无重复、时间文字清楚。时间/任务详情label/来源位置及memory metadata采用现NolaTextSecondary，指纹正文从9pt升12；当前资产与canvas对比light6.29:1/dark9.36:1。装饰icon不变，未新增测试或重复40项/模型。首次Shared wrapper声明与新AX测试helper编译/激活失败均保留（20–23），最终25才计通过。

2026-10-29继续验证与主审：

- Web同样存在38/48px姓名竖排栏，已将ExpertReplyFrame全文姓名移到bubble上宽列，头像仍独立颜色入口；源只显示一次姓名，允许完整换行。类型0错误/0警告、生产构建exit0（`web-name-check/build.log`）；未新增浏览器点击通过声明。
- iOS新桥/Swift本批首轮42契约41通过、1旧stub未识别process查询失败；最小修fixture后该case实际重跑通过，不为测试放松生产恢复。当前工作面板已实际点6目的地并保留原草稿，但管理按钮命中仅19pt使后半测试失败；已把44pt frame/contentShape移到Button label内，实际管理与native双专家新桥任务继续验证。
- Android新JNI/app/testAPK与JVM643项（2skip）首批完成；新的source gate/draft绑定也经真实操作后暴露两类问题：列表长项未compose需真实滚动定位（fixture），以及RunRegistry canonical conversation owner被UI draft storage alias混淆导致真实模型流虽开始而界面不更新（生产问题）。现明确分开execution owner与draft存储身份；新回归body断言通过但JVM Main dispatcher清理失败仍保留，修环境后复跑。旧坏APK真实runner不计通过。
- Archive语义主审纠正：不能把directory archived推断为现有Room participant不可发送。Core正常expert admit保留已有context snapshot；撤回客户端自动清理已选收件人变更。只阻目录新增选择archived，不改已有选择或原pending manifest。
- root发现Mac旧row.project只供单测，生产另用global createdAt排序；晚child可能落在后续父消息后。正在改真实生产parent-anchor projector并移除dead测试路径，以Core canonicalMessageID而非partID/UI UUID绑定原父；代码已落待主审后受影响Workspace验证，不以旧纯helper测试记通过。
