> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Remote session Port cutover

本轮迁移 remote turn、remote terminal projection、mirror 写入和完整 session projection 查询的实际 Application helper。没有修改历史语义、远程 owner/cursor 准入规则或 Runtime 装配。

## 实际依赖与事务

`nola-store-port/remote_session.rs` 拥有五个原 Store DTO：`BeginRemoteTurnInput`、`CommitRemoteTransitionResult`、`CommitRemoteConversationProjectionInput`、`RemoteSessionMirrorInput`、`StoredRemoteSessionMirror`。Store contracts 仅 reexport；其余未迁远程/上下文 DTO 不假装归属已完成。

`RemoteSessionRepository` 提供远程 turn 创建、Run→assistant owner 查询、远程 terminal projection 原子提交、mirror 原子保存。Application 的 `open_remote_conversation_turn`、`commit_remote_conversation_projection`、`save_remote_session_mirror` 已泛型化；terminal helper 另依赖 `PermissionRepository`，保留同一 turn 中 Application 新增 grants，不让远程 snapshot 替换时抹除它们。原 assistant ID、projection contract/version、source Host、terminal outcome、重复 Part/assistant 检查全部保留。

`SessionProjectionRepository::load_authoritative_session` 一次返回 `AuthoritativeSessionRecord::Local(StoredCheckpoint)` 或 `Remote(StoredRemoteSessionMirror)`，或 None。SQLite adapter 用 `read_snapshot::pin` 在同一快照内判断 authority 并读取对应记录，避免先后独立读决定来源时看到不一致状态；外层 UoW 已存在则直接复用。`load_session_projection` 只接收此 Port，并沿用原本 local/remote decoder；resume 聚合可以直接使用这个泛型完整投影 helper。

所有写适配器均调用当前同一 `NolaStore` 实例的原方法：canonical Parts、remote projection/mirror、permission grants、Run/transition/outbox 与 receipt 仍由现有事务体系提交。不另开 writer，也不提前结束外层 UoW。

## 实证与旧断言校正

新增 `remote_session_port_e2e`：

- 非 SQLite 仓库调用真实 open/terminal/mirror/query helper。错误 assistant Part owner、错误 Run 绑定、空 source Host 被拒；注入仓库写失败不留下 terminal fact；保存 mirror 后新仓库实例可通过完整 session helper恢复相同投影。该 fixture 不声称替代真实 SQLite cursor/CAS 验证。
- 真实 Runtime+隔离数据库在 terminal 最终 receipt UPDATE 失败时，逐表逐值证明所有数据库事实回到基线（含 canonical Parts、mirror/projection、grants、Run、events/outbox、receipt）。同请求重试后关闭/重开，exact receipt 返回相同 outcome，事实不重复，完整远程 session 和 Application grant 都保留。

`remote_projection_identity_e2e` 的三项旧断言已经过时：这些测试调用普通 conversation projection/window/page，并不经过本轮修改的完整 `load_session_projection`。生产读取现将存储的远程 JSON Part 表达成 Historical；原断言仍要求 live Tool/Reasoning/Text，或要求第二次 append 后 Historical 的 canonical_revision 停留在 2。本轮只修测试：逐项校验全部原字段、schema/status、Part/Message/conversation owner、Part revision、当前 canonical_revision，以及原 JSON 的精确 SHA/UTF-8 length。第二代 append 仅允许 canonical_revision 从 2 更新到 4，其余 Historical source/字段完全相等。没有恢复 live Tool 或减少原内容断言。

## 剩余范围

Application Runtime/统一 dispatcher 的具体 Store 依赖仍由后续聚合 Port 切面处理。未被上述 helper 使用的 legacy remote transition 写入和其他 remote context/projection 读取接口不在本次声称的迁移范围内；并未修改 source Host 认证协议或让 remote claims 获得本地来源授权。

最终验证：新 remote session integration 2/2；remote identity/window/10M/历史保持 4/4；Application session_projection unit 5/5；remote snapshot/transcript 冷重启定向 1/1。`cargo check --workspace --all-targets --quiet` 通过（session 47046）；仅剩其他切面正在收尾的 permission helper dead_code 警告。无运行中测试或环境阻塞。
