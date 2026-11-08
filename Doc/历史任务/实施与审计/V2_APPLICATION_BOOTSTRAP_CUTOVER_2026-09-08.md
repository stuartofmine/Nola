> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Application 主库装配切换（2026-09-08）

本次切换落实 `ARCHITECTURE_V2.md` §3.1、§4.3 的装配职责分离，保留现有工作区和数据库协议；不等于 Application 已删除具体 Store 或旧 Core 的全部生产依赖。

## 审计结论

切换前 `core/nola-application/src/runtime.rs` 同时承担了：创建主库父目录、`NolaStore::open`、判断不兼容 schema、删除 database/WAL/SHM 后重建、选择 `LazyContentUploadStore` 的路径与实现、构造 Supervisor。Service 虽注入上传仓库，主库仍由 Application 打开；移动 FFI 与 CLI 临时会话又调用了默认打开路径。

这些属于服务所有者的装配职责，能够作为完整切面迁出，无需先修改历史语义、归档或全部 Run 用例。只新增 factory 而保留原生产调用不算完成。

## 本次实际生产链路

```text
Service 单写者锁 ──┐
嵌入式 FFI 单写者锁 ├─ nola_application_bootstrap ── NolaStore / LazyContentUploadStore
CLI :memory: ──────┘                  │
                           ApplicationRuntime::from_store
                                      │
                            Application 命令与事务
```

- 新增独立 `core/nola-application-bootstrap`，仅依赖 Application 和 Store；不依赖 IPC Service、客户端 SDK、平台 UI 或 Host。
- Service `src/lib.rs` 的真实启动路径改为 bootstrap；原上传仓库与主库的分散装配删除。Service 的直接 `nola_store` 正常依赖已删除，保留 dev-dependency 仅供隔离测试检查真实数据库。
- `core/nola-mobile-bindings/src/lib.rs::open_owned_runtime` 在已有 `.service.lock` 独占锁之后调用 bootstrap；未引入整个 `nola_service` 依赖。移动端 FFI 仍是嵌入式服务边界，不是允许任意客户端直接写库的 SDK。
- `cli/src/application_port.rs::ephemeral` 使用同一 bootstrap 打开 `:memory:`；持久化 CLI 的生产路径仍只使用 Service Client。其 `cfg(test)` 直连隔离库分支也改为 bootstrap。
- Application 删除生产 `open/open_with_policy/open_with_upload_repository` 和文件删除代码，仅接受已经打开的 `NolaStore` 与上传仓库。其内部单元测试保留 `cfg(test)` 开库辅助；该辅助不进入生产库。
- Application integration tests 通过 `tests/support/mod.rs` 显式构造隔离 fixture，不使用 bootstrap 的反向 dev-dependency，避免形成 Cargo 循环。非 SQLite 上传仓库注入测试直接使用新构造入口。

`ApplicationPersistencePolicy` 移入 bootstrap。默认 `open` 始终使用 `RequireCurrent`，损坏或不兼容库返回错误；bootstrap 不会显式删除主库、WAL 或 SHM。原显式 `--recreate-incompatible-store` 才能选择 `RecreateIncompatible`，并且只有 Store 的 `IncompatibleSchema` 错误允许执行；普通损坏错误也不能触发它。没有新增自动 fallback、旧库迁移或客户端重试重建。

实测 SQLite 自身可能在尝试打开明显非数据库文件时删除损坏 WAL/SHM。因此 bootstrap 在读写打开前增加固定 16-byte 格式头的只读检查，拒绝非空且明显不符合 SQLite 签名的文件；新路径、空文件和 `:memory:` 仍合法。这不复制 Store 的 schema 判断，不扫描整个文件，也不声称能预判页损坏；签名正确时的内部恢复与 journal 行为仍由 SQLite 管理，不能把“默认没有显式删除分支”扩展成任意损坏状态下 SQLite 都绝不触碰 journal。

Bootstrap 本身不获取第二把锁：Service 和嵌入式 FFI 已在真实入口先获取服务单写者锁；再次获取会让同一拥有者锁自己。公开 bootstrap API 文档明确仅供服务所有者装配，调用者必须先完成锁的获取。

## 事务 UoW 的实际边界与剩余具体依赖

`command_receipt.rs::execute_received_command` 已通过 `CommandTransactionRepository` 执行外层 write UoW，业务 mutation 在内层 savepoint 中执行；Projection journal、outbox 与 terminal receipt 共用这个事务。执行失败时回滚 mutation，保存拒绝收据；外层提交失败会回滚持久化和 Supervisor 暂态。事务所有权没有迁给 bootstrap，也没有拆成多个仓库连接。

不能据此宣称 Application 已依赖倒置完成：

- `ApplicationRuntime.store` 仍是具体 `NolaStore`；`from_store` 明确暴露该过渡，不用别名或 `Any` 隐藏它。
- Application `lib.rs` 的 Host response 接受、Run/Checkpoint 恢复、skill 注册、conversation branch 等函数仍接受 `&NolaStore` / `&mut NolaStore`，并使用具体 Store DTO。
- `runtime.rs` 热 Run 恢复调用具体 checkpoint revision 等接口，Supervisor 保存旧 `nola_core::AgentCore`。Application 的 `nola_store`、`nola_core` 正常 Cargo 依赖仍存在。
- 一个 port 名称或事务 trait 已落地，不意味着其所有命令、读取、错误和 DTO 都已经脱离 Store。

下一完整切面建议先处理 Run/Checkpoint/Effect 命令上下文：把这些实际加载、原子提交和恢复方法及其 DTO 移入 Store Port，令使用它们的 Application helper 真实改为 trait 参数；统一外层 UoW 使用同一仓库对象。最终 Runtime 持有由各已迁移 Repository 组成的事务端口，而不是每个功能独立连接。只有最后一个具体 DTO/方法调用和 `from_store(NolaStore, ...)` 被移除后，才能删除 Application 的正常 `nola_store` 依赖。旧 Core 的纯决策迁移是另一项门槛，不应通过把整旧 Core 重新命名成 Kernel 隐藏。

可执行的下一批文件/入口：

| 当前生产入口 | 需迁移的实际依赖 | 删除条件 |
| --- | --- | --- |
| `nola-application/src/runtime.rs` 两条 hot Run 恢复分支 | `checkpoint_revision` 与 Run 当前状态读取 | Runtime 只调用同一事务仓库的读取端口 |
| `nola-application/src/lib.rs::begin_assistant_turn` | Store `begin_turn` 及其输入/结果 DTO | 输入、结果与错误由 Port/Domain 定义，真实调用不再接受 `&mut NolaStore` |
| 同文件 `reduce_and_accept_host_response_with_core`、`reduce_and_commit_active_submission_with_core` | checkpoint 加载、`commit_transition`、effect outbox/host result 的原子提交 | 冷恢复、重复结果、失败回滚共同通过新 Repository，不旁路旧实现 |
| `command_receipt.rs`、`projection_journal.rs` | 目前 trait 调用仍固定在具体 `self.store` 上 | 新聚合端口的业务方法与 journal/receipt 明确共享一个 UoW，不能各自打开连接 |

这批应先把 Store 的 `BeginTurnInput/Result`、`StoredCheckpoint`、`StoredActiveRun` 和 transition 契约移动到 Port，再逐个替换上述函数的真实签名和调用。保留 Store adapter 的 SQL/savepoint 实现，复用已有 Turn、Host response、receipt fault 和冷重启测试；增加一个非 SQLite事务fixture检验 Application 不调用旧具体入口。只增加 trait 而不修改这些生产函数不算这一批完成。


## 验证

新增 bootstrap 回归覆盖默认保留旧 schema 字节、损坏主库/WAL/SHM 字节、显式重建仅接受不兼容 schema、真实命令 receipt 共提交、关闭/重开后重放和上传库延迟创建。终态如下：

- `cargo check --workspace --all-targets --quiet` 通过，所有被迁移的生产入口和测试入口均编译。
- bootstrap 4/4；Application archive derivation 四故障回滚 1/1、setting receipt 3/3、非 SQLite 上传注入 1/1。
- Service 真实 IPC duplicate-request/Projection suffix 1/1；移动 Rust FFI 真实 open/execute 1/1。这两项不等于本次已经跑过原生 Swift/Kotlin/JNI。
- Service 显式 recreate 子进程测试在本代理环境被 Unix socket bind `Operation not permitted` 阻塞，已交 root 运行，不能记作通过。
- CLI 两个真实服务入口旧 schema 保护的原测试通过，但审阅发现它缺少具体拒绝原因断言，环境级 socket 失败也可能通过。因此新增 stderr 必须包含 canonical schema 拒绝的断言，再交 root 复跑；旧一次绿色结果不作为强化后的验证证据。

`cargo tree -p nola_service -e normal --depth 1` 确認 Service 无直接 Store；同命令检查移动 bindings，确认没有引入整个 Service。Store 仍经 Application/bootstrap 间接存在，未声称消除所有生产 SQLite 依赖。


root最终补验：真实Service process6/6（含explicit_development_reset_recreates_only_an_incompatible_schema）
及历史预算process1/1通过，`/tmp/nola-history-bootstrap-service-final.log`。
加强精确stderr断言的CLI schema-preservation1/1通过，`/tmp/nola-bootstrap-cli-schema.log`。
workspace最终all-targets检查通过，`/tmp/nola-history-bootstrap-workspace-final.log`。
这些root终态覆盖前述子任务socket EPERM造成的未验证项，不把环境失败计作通过。


后续增量已完成RunEffect读取/提交生产切面，详见
[V2_RUN_EFFECT_PORT_CUTOVER_2026-09-08.md](V2_RUN_EFFECT_PORT_CUTOVER_2026-09-08.md)。
上面的后续清单是装配切换当时的规划；当前剩余重点为BeginTurn/内部Run创建及Runtime聚合端口，
不是仍需从零新增RunEffect seam。
