> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Web / Mac 会话草稿仓储

遵循 CONVERSATION_AGGREGATE_CONTRACT_20261015.md。消息、Run、配置仍由 canonical
仓储持有；此模块只存本设备尚未提交的草稿，不提供跨设备同步。

- Mac：实际 canonical 数据库路径旁的 `<database filename>.drafts` 本地目录，
  不写 productSettings、conversation settings_json 或同步 outbox。以数据库路径隔离作用域，
  conversation ID 的 SHA-256 为文件名；目录 0700、文件 0600，并排除系统备份。
- Web：IndexedDB 的 Host ID + conversation ID 复合键；不持久化运行 Task、
  授权 challenge 或凭据。每次写入冻结草稿，按提交顺序串行落盘。
- 切换先读取目标草稿，旧加载结果不能发布到新会话。编辑后待提交文本属于原 owner，
  切换不触发保存共享字段；删除清除正文并保留墓碑，迟到加载不复活。
- 附件只保存受限的已提取文本或可恢复引用，不声称恢复文件系统访问授权。
- 存储失败保留内存草稿并呈现失败，不以空草稿覆盖。测试覆盖重建、Host 同 ID 隔离、
  删除/加载交错和写入排序。全部改动后统一回归。

## 已实施

- Web：`conversationDraftRepository.ts` 为 IndexedDB 仓储，`workspaceDrafts.ts`
  管理按 Host 的内存投影、加载门禁、字段级 dirty 合并与有序写入。每次编辑写原 owner，
  不在切换或卸载时把当前输入搬到另一个会话。清理页面投影不删除持久草稿。
- Mac：`MacConversationDraftRepository` 使用 canonical repository 的实际数据库作用域，
  不误用默认 ApplicationPort。串行后台写入，预加载期间的文本/附件修改独立合并。
  本地 JSON envelope 持有 revision 与删除墓碑；flock 将跨实例/进程的读取、CAS 与原子替换
  包成事务，不创建第二份 Core 数据库。旧实现误用可同步设置表，本轮已移除所有草稿设置写入；
  不擅自迁移或删除已有数据库/同步队列内容。
  尚未执行的同 owner 保存合并为最新快照；读取与删除隔断合并批次，不能越过删除屏障。
  显式新会话先创建 canonical 身份再进入侧栏；启动时未提交的临时身份在首次编辑时保存，
  身份本身不改变，不需要复制为另一个 draft ID。附件不恢复临时文件访问授权。
- 两端按 revision 拒绝旧窗口覆盖，并保留无正文的删除墓碑，防止旧窗口复活已删草稿。
  保存失败保持内存与错误提示；不会自动读取最新 revision 后强制覆盖另一窗口内容。
  当前冲突恢复应保留/复制本窗口文本再重新打开，不把普通 Retry 当强制覆盖操作。
- 模型选择、参数和权限仍使用 canonical 配置读写及 revision 门禁；现有用户显式选择的
  “继承默认模型”语义保留，不将其误判为上一会话状态串用。

## 验收边界

已添加 Web 仓储投影回归和 Mac 临时数据库仓储测试（重建、作用域隔离、早编辑、
迟到加载、删除与 CAS 冲突）。2026-10-15 整批实现后统一验证：

- `web/frontend`: `npm run check && npm test && npm run build` 成功，类型检查零错误/警告，388 项测试通过。
- Mac 本地文件仓储整改后重跑 `Test` 配置：`MacConversationDraftRepositoryTests` 与 `MacChatWorkspaceBehaviorTests`，195 项，194 通过、1 跳过、0 失败。新增真实 ApplicationPort 回归确认保存、删除草稿前后 `pendingSyncChanges` 完全不变，旧 productSetting 键未创建，重新实例化后墓碑仍阻止复活。跳过的是需要显式打开真实系统面板的 `testNativeWorkspacePickerCancellation`。
  xcresult 为本机 `/tmp/nola-mac-local-drafts.A5ZUHl/Results.xcresult`，runtimeWarnings 为空；未覆盖正在使用的签名 Debug 应用。

真实窗口重启交互尚未验收。Web IndexedDB 真实多标签窗口行为仍需浏览器验收，
内存仓储替身测试不等同于 IndexedDB 引擎验收。
