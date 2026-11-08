> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# iOS 未提交草稿聚合与并发写入

## 所有权和边界

device-local草稿以数据库路径scope + draft UUID / canonical conversation UUID定位。未提交草稿保存正文、安全附件引用、模型目录ID + local/configured/host来源鉴别及完整ModelParameters快照；绝不保存ModelInfo里的连接凭据或API key。已创建会话的模型配置只从Core读取，草稿中的配置字段仅允许unsent owner。

新草稿可显式用全局默认初始化一次。恢复时按保存的modelID精确查当前可用目录；目录不存在、配置缺失或磁盘读取失败均显示错误并禁止直接发送，不能改用全局选中模型。首次发送将已冻结参数传入会话创建，再退休unsent身份；已创建会话继续由canonical配置仓储管理。

## 文件事务

沿用Mac文件仓储的flock + atomic write方案。每个manifest包含revision、deleted及草稿内容；仓储实例在load时记录observedRevision，save只比较此基线，成功后推进本实例基线，不在save前重读后盲覆盖。所有编码/磁盘save位于串行后台队列，锁覆盖read/CAS/write。跨实例基线冲突明确报错，内存正文、附件和配置不清空，不自动reload覆盖本地编辑。

删除保留revision墓碑，迟到写即使原来没有文件也不能复活。canonical会话发送仅清空草稿，不写删除墓碑；会话真正删除和unsent提交才退休owner。写失败通知包含仓储实例身份与owner，避免另一个窗口误接收或误清错误。

附件blob使用UUID+内容SHA256不可变名称，manifest保存UUID及hash；加载验证hash。CAS在写blob前检查，blob写完但manifest提交失败只留下未引用blob，旧manifest仍引用旧版本。错误状态按owner保存，切换B不显示A冲突。

## 验收

1. 新草稿模型/参数重建后保持原值，全局默认改变不影响旧草稿。
2. 模型目录已删除时明确错误，无全局fallback；manifest不含ModelInfo/token。
3. 两实例读取相同revision，A成功后B失败，B内存内容保留。
4. 删除后旧实例写入失败，重建仍观察墓碑，不恢复已删除草稿。
5. 同实例连续后台写保序；发送既有会话后可以继续写下一条草稿。

先实现与编写测试，统一阶段才编译/运行。跨设备同步、冲突自动合并和附件GC不属于本次；冲突保留在原窗口，用户可复制后重新打开目标草稿，不隐式覆盖别的窗口。

## 实施记录

- `FileConversationDraftRepository`加入revision manifest、flock、实例读取基线、删除墓碑；CAS在附件写入之前检查。pending-owner索引也在相同队列/锁中读写。失败通知携带仓储UUID，ChatView仅消费自己实例错误。
- 未提交配置仅存modelID和参数；恢复经`IOSDraftModelResolver`精确查目录，无全局fallback。参数快照用于首次创建；响应风格在未提交草稿中更新，不提前创建会话。
- `ConfiguredConversationCreationStore`将模型、canonical modelSelectionID（非虚拟Host）和完整参数一次create提交；移除ChatManager原三事务创建。保留已有会话selectedModelInfo展示字段，尚未完成全部跨端身份投影替换，不能宣称身份完全统一。
- 显式切换已存在会话模型使用同一revision CAS事务更新modelSelectionID、selectedModelInfo和必要初始参数，已有参数验证后保留；虚拟Host清canonical configured身份。读取时如果canonical身份与展示快照不一致明确拒绝，不无声执行旧模型。全部跨端展示从canonical profile派生仍未完成。
- 提交前冲刷并验证草稿基线；unsent退休使用CAS；现有会话提交只清草稿，不写墓碑。新建会话先不发布当前选择，退休成功才发布，失败仍展示原草稿。草稿文件与Core数据库不是同一事务，验证之后若另一窗口恰好改草稿，退休失败会阻止发送并保留草稿，但可能留下一个已创建且未发送的新会话；这是明确剩余跨存储边界。已有选择读取异常或丢失明确拒绝，不隐式创建替代会话。
- 新增配置恢复/缺模型、两实例冲突、删除后迟到、既有会话连续输入、原子配置创建拒绝无效输入的测试。尚未编译或运行本批测试。

## 统一验证记录

首轮iOS模拟器整组54项，52通过、2失败；草稿CAS/blob/新建原子性新增测试均通过。失败根因：并行会话测试清理临时目录EPERM（行为断言已通过）；严格权限测试对规范化JSON比较原字符串键序。分别保持最终清理失败可见的有限排空重试、结构化JSON全值相等断言修复。新增“创建先不发布选择/已有会话读取失败不隐式新建”测试后进行同整组复跑。

最终第三轮 **55/55通过，0跳过、0失败**（第二轮另一项流式fixture出现相同清理EPERM，统一复用严格有限排空helper后复跑）。环境：iPhone 18 Pro Max / iOS 27.0 Simulator，单worker，`scripts/run_client_tests.sh`；六组包含ConversationComposerDraftStoreTests、ConversationDraftRepositoryTests、IOSDraftAggregateTests、CanonicalConversationStoreEndToEndTests、ModelServiceConfigurationInstallationTests、ChatManagerTests。证据：`/tmp/nola-ios-draft-aggregate.imdrZb/Results-3.xcresult`，日志`test-3.log`。唯一编译警告为既有NolaCore Copy Headers受unsandboxed script延迟，无本批新增Swift告警；xcresult runtimeWarnings为空。没有测试实体设备或改动Mac Debug产品。
