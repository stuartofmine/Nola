> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Core 控制状态与恢复正文分离

2026-10-01 设计与实施追踪记录。下文“当前”仅指各日期记录中的版本，后续进度沿各节记录阅读；
不能据此使用旧 schema8 恢复现有数据。当前版本数值只从代码声明读取，设计边界见[总纲](../../总体设计/SOFTWARE_DESIGN.md)。
本文不能作为端到端性能验收证明。现有私有 Text 分块、公开 assistant journal、
Core 恢复正文是三个不同的合同，引用不可相互强转。

## 当前生产路径与成本

2026-10-01 审查时的 schema8 checkpoint 内包含完整 `AgentThreadState`。当时核查发现：

| 边界 | 当前行为 | 需要消除的成本 |
| --- | --- | --- |
| `runtime/submission.rs::submit` | 为错误回滚克隆整个 session | 每个 delta 复制累计正文 |
| `runtime/projection.rs::apply_part` | 克隆更新后的完整 Part | 增量事件携带累计正文 |
| `runtime/host_response.rs::envelope_events` | 已改为消费owned事件（下文）；事件本身仍含累计Part | 上游累计Part仍需拆分 |
| `runtime.rs::checkpoint` | 已直接取小metadata，不再构造并丢弃完整snapshot | checkpoint状态本身仍需一次克隆 |
| `runtime/bootstrap.rs::snapshot` | 克隆整个 conversation projection | 控制转换返回完整呈现内容 |
| `runtime/host_response.rs::dispatch_accepted_host_response` | clone PendingEffect与完整request Value并重复解码 | 每delta重复复制冻结模型输入 |
| Application `runtime.rs` | 热批次缓存完整事件；字节预算只计 response | 实际缓存量超出计量对象 |
| Application `lib.rs` | checkpoint 转 Value、克隆、解码、再投影、编码 | 同一持久边界重复物化 |
| `conversation_transition_store.rs` | CAS 更新整个 checkpoint JSON | 正文累计编码和数据库写入 |
| `new_memory_usage_entries` | 为取得旧 usage ID 解码完整 checkpoint | 小元数据查询扫描全部状态 |

64KiB 热批次只减少持久化次数，received-command receipt 每次要求真实持久化。
不能通过跳过 receipt 的持久化、删掉错误回滚或降低 checkpoint 频率解决这些成本。
现有 2/8/32MiB Store 实测使用固定 `{}` checkpoint，证明的是字段追加路径，
没有测到本表的 Core 路径。

本地参考 `参考资料/codex-main/codex-rs/thread-store/README.md` 将 canonical history
append 与 metadata patch 分开，活动线程有自己的持久化所有者。采用这一职责边界；
Nola 的原子 checkpoint/effect/receipt 和具体内容引用由自己的合同验证。

2026-10-01再次按OpenAI Docs检索并打开
[Responses流事件参考](https://developers.openai.com/api/reference/resources/responses/streaming-events)：
文本delta带item身份、output/content位置及sequence，完成另有事件。本文据此推导
Nola自己的内容身份/顺序/生命周期必须分离；这不是官方替Nola规定Store版本、回滚或
权限模型。provider序号不充当本机已提交cursor，provider完成payload也不意味着每次
delta都要再次交付累计Part。接口适配仍保留其各自完成与异常校验。

## 数据所有者

Core 保留纯决策能力；外部正文对象的读写由 Application/Store 承担。目标持久形态
为版本化的控制 checkpoint 加正文绑定目录，而不是把现有全文 JSON 隐藏到另一个字段。

恢复引用至少包含协议版本、conversation、创建 Run、逻辑对象身份、typed field、
generation、root、UTF-8字节数和编码类型。模型 transcript、Text、Reasoning、工具
输入/输出及等待中的模型请求分别有字段类型；不把任何工具 JSON 当 Markdown。
结构化内容保持类型和数值，不经过会损失大整数的二次解析/编码。

控制 checkpoint 保存精确 execution/permission/call/effect/attempt 状态、cursor、
幂等身份、冻结配置、上下文版本和绑定目录。正文目录只引用该提交已经准入的版本。
完整会话投影改为显式查询；每次 transition 返回小控制快照和有身份的增量操作。

公开 journal 只包含准许展示的内容；恢复正文必须包括未批准草稿、隐藏 reasoning、
实际调用与返回、provider continuation 和待执行请求所依赖的输入。这些私有对象
不能因为持有 digest、客户端 owner 或公开 snapshot 就可读取。

## 提交与回滚

一个 Application write unit of work 同时提交新增正文版本、控制 checkpoint CAS、
canonical 变化、effect outbox 和 command receipt。只有外层提交成功后才通知消费者。
相同 receipt 重放返回原结果；未经提交的增量不获得恢复或公开读取资格。

Core 的热数据需要可追加、可按转换回滚的正文结构。仅把 `String` 放入 `Arc` 后
继续 `push_str` 仍会在共享时复制旧前缀，不满足目标。回滚记录新增块、长度、head、
受影响的小控制记录及新增身份，恢复错误前的状态；不能删除现有回滚来换取速度。
Replace 分配新 generation；旧版本仍可由原已提交 checkpoint 读取。等长替换不是同版。

绑定目录和当前 head 的更新属于不同语义。冷恢复须使用提交时的不可变版本，不能
用最新 head 覆盖历史 checkpoint 的正文。原文阅读器的跨页版本问题也按独立版本
句柄解决，不能用“总长度相等”替代 generation/root 校验。

下一垂直切片是热ModelDelta/Reasoning/ModelToolCallDelta的转换级mutation journal，
入口包住accept_response至finish_transition全部fallible阶段。Append记旧长度/head，
失败truncate新增字节；Replace/Remove将旧owned buffer移入undo，Created记新增身份
和order长度。cursor、accepted submission、pending response sequence/state、trace
pending等受影响控制记录也须准确撤销，不克隆含冻结上下文的整个Turn作“小记录”。
冻结pending request改为一次验证的不可变typed payload加小facts，每delta读取facts，
不重复解码实际模型输入。事件/effect在成功前不交付外部；失败可按原序号修正重试。

事务还须覆盖config/catalog。当前错误回滚仅恢复session，而配置命令能改config；
新事务不得继续依赖这个不完整边界。其他命令在完整迁移前仍保留原回滚，但不据此
声称整体路径已增量化，也不以跳过received receipt或持久化来换取成本下降。

## 恢复加载入口

新增一个 Application 恢复加载所有者：校验 checkpoint、配置、Run 与 Host 绑定；
从其已提交目录读取准确正文版本；构造 Core 的恢复候选；所有验证成功后替换 live
runtime。失败不能污染 live 配置、目录、权限或 checkpoint。

以下当前消费者必须统一接入后才能切换持久 schema：

| 消费者 | 所需语义 |
| --- | --- |
| `assistant_turn`、active/idle submission、Host response、显式完成 | 同一提交版本、冻结上下文、增量续写 |
| `maintenance_turn`、`run_recovery`、`run_cancellation`、`cancellation_delivery` | 精确执行身份、取消意图与实际投递事实 |
| `session_projection`、`LoadCheckpoint` | 呈现查询与恢复查询分开，隐藏内容不成为普通回复 |
| CLI `SessionStore`、`CommittedCoreRuntime`、cancel-only runtime | 验证候选后原子恢复，保留原凭据来源和 execution policy |
| Runtime Port `load_core_checkpoint`、iOS FFI restore | 消费同一版本解析结果，不自行猜正文或目录 |

现有 `canonical_stream_read::current` 要求读取 head 等于当前 head，并且 resolver 只
允许可见 assistant Text。它不能直接作为上述历史版本和私有执行正文的恢复接口。
新 reader 应从已提交 checkpoint 的完整绑定出发，再验证准确 manifest/chunk；
不会根据公开展示规则拒绝 Core 合法的隐藏恢复内容，也不会授予普通 UI 私有读取权。

## 实施切片

1. 明确借用 checkpoint 的复制边界与已经 owned checkpoint 的消费边界，先消除
   接收、恢复时的额外副本。保留 active/terminal-with-pending 的完整恢复状态及
   现有终态目录裁剪条件；这一步不代表热路径已经增量化。
2. Domain 定义 typed control checkpoint、私有内容引用、增量操作和版本准入。
   Application 建立唯一恢复加载入口，Store 实现不可变版本及同事务绑定。
3. Core 采用可回滚追加正文；事件只交付新字节及生命周期；转换快照与完整呈现查询
   分离。热批次统计实际保留的操作与字节，不只统计 response。
4. 接通表中全部生产消费者和所有正文 family，完成冷恢复和失败验收后切换持久
   schema，删除旧全文持久路径。尚未上线，不增加长期双写或猜测恢复的兼容路径。
5. 按真实保留根实现全 Store 配额/回收：checkpoint、effect、immutable snapshot 和
   活动绑定必须保留所依赖的版本，不能只按当前 head 清理。

活事件与完整呈现查询须一同迁移：Core所有Part生产者及Trace，Application两个
reducer/热缓存，Runtime Driver acceptance，CLI presenter/emitter/goal observation/
messaging与native/Web bridge，FFI submit，以及iOS/Android/Web的Part替换reducer。
Mac主正文已canonical，但旧Core decoder/诊断仍需迁移。`AgentTransition`改小控制
快照和typed变化；完整projection为显式attach/query，其他Task/Goal/Memory/Permission/
Skill投影按revision及受影响记录读取。不能只删event.part却在下游再次hydrate全文，
也不能只删snapshot.projection而保留其他无限投影的自动复制。

## 验收证据

统计实际复制、编码、解码、哈希、读写和缓存字节，以及触碰的正文/控制记录数。
使用真实 Core 和 Application 的 2/8/32MiB、1,000 等长 delta；分别测试热批次和
每条 received-command 两条入口，记录新增字节成本与已有正文规模的关系。

必测：冷重启继续、receipt 重放、同长 Replace、旧 generation、隐藏 review、
Reasoning 与工具结构化数据、跨 runtime 取消、等待批准、provider continuation、
事务后段失败、Core 转换错误回滚和恢复候选失败。历史版本可读不授予旧 Run 写权。
指针保留或单个字段 benchmark 只能证明对应复制边界，不能替代上述全链路验收。

## 已实施的消费边界（2026-10-01）

`AgentThreadState::into_checkpoint_projection(self)`消费已有状态；借用入口仍克隆
一次后转发，不裁剪活跃runtime。Application的durable projection和Core restore
接通消费入口，删除Host journal原来仅返回self.clone的中间方法。终态裁剪仍要求
turn已终止且没有pending effect，冻结请求与实际调用保留。

`owned_checkpoint_projection_preserves_bodies_and_borrowed_semantics`覆盖活动、压缩、
完成、取消、失败、完成但仍有pending的六种状态：借用/消费结果逐字节一致，消费前后
transcript、model context、Text、冻结context与真实ModelStream请求的正文地址相同。
Core checkpoint过滤回归lib15、host contract10、turn context1，共26项通过；命令及
日志见总改造文档。上述第2–5项和每delta回滚复制/累计事件仍未实施。

事件封装的唯一生产调用者现在交出`Vec<AgentEvent>`，`envelope_events`使用
`into_iter`转移原事件。新增大Text/Created/Delta和非Part事件合同，验证四个正文地址
保持、完整wire字节、事件顺序、独立turn身份、cursor与revision不变；空批次不推进
cursor。此处不再复制正文，但上游Created仍含两个Part、delta事件仍含累计Part，
不能将局部所有权转移当作事件合同已经增量化。完整Core lib393、Host contract115、
turn context2共510项通过，日志`/tmp/nola-owned-envelope-core-20261001.log`。

同日继续删除`checkpoint()`仅为取得session/revision/cursor/catalog而生成完整snapshot
的额外副本，直接读取四个权威字段。checkpoint schema、created_at=0、reason、原
借用state投影及终态裁剪保持不变；六态×17种reason与旧snapshot构造逐字节相同，
另检查idle及cursor/revision/catalog边界。Core完整lib395、Host115、turn context2共
512项通过，日志`/tmp/nola-checkpoint-metadata-core-20261001.log`。Application lib182
和六组integration33均通过，`/tmp/nola-checkpoint-metadata-application-20261001.log`。
这不是checkpoint状态已经无复制，也不是新的持久schema或增量恢复实现。
