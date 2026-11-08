> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Canonical 流式正文合同

2026-09-28。整体改造 P5 的实施合同。Text生产写入已接线并通过定向验收，
全端与规模性能验收仍未完成。

## 当前问题与所有者

原路径Application `part_mutations_for_update`每次Text delta哈希完整旧前缀，Store
又读全文校验并SQL拼接。现在新建/空占位Text已使用独立planner与Chunked正文，
Inline/Sealed仍可读；已有非空非stream字段保留原路径，不自动覆盖其原文。
因此并非所有历史写入、Core checkpoint与投影clone成本都已消除。已有journal
publication仍不能当作此canonical私有正文主链的替代。

公开 assistant journal 的展示字段不是 canonical 原始字段的授权引用，尤其不能
把 ToolInput/Output 引用强转成 content_json。原始字段所有者仍由持久化的
conversation → message/Run → part → field 确定，客户端提供的 digest 不是读取权限。

## 目标数据模型

新增 canonical-private streaming text head：version、完整 owner（conversation、
run、message、part）、field=content_text、generation、revision、root、UTF-8字节数、
chunk数量。Replace增加generation；实际Append推进revision。root用独立domain
separator绑定owner/field/generation；不得每次Append重新计算完整正文SHA。

Store分别保存字段head、物理chunks、不可变manifest链，以及按owner/generation/
累计字节索引的chunk布局。物理chunk可去重，但任何读取必须从已提交head的完整
owner出发。不能凭全局chunk digest访问其他字段。Sealed既有完成态约束不放宽。

### 内部续接的两种归属（生命周期回归发现，已接线）

内部续接保持同一可见assistant消息，不制造额外用户消息。`messages.run_id`表示
当前执行Run，不是该消息所有历史Part的不可变创建者；stream head中的run身份则
不可重写。不得用修改head/root、取消校验或新建可见消息规避这一冲突。

整改要求：可信的source/replica续接事务记录canonical message与历次Run的精确关联。
读取历史stream时必须验证当前执行者和原创建者均属于该消息的合法续接关系；
写入仍要求当前Run，历史读取资格不授予旧Run继续写入的能力。关联不得根据请求中的
head自报身份生成，也不得依赖replica没有的runtime runs。

Run的初始占位Part也必须由准入事务明确记录，不能从复用消息的最早Part推测。
没有创建占位Part的内部Run应返回None，终态只关闭本Run实际拥有的Part。
Sealed历史字段仍要求已完成Part及合法发布证据；若消息因可信续接重新queued，
只能凭明确续接事实读取原已完成字段，不能全局撤销完成态栅栏。

schema32添加`canonical_message_run_owners`记录可信续接归属，
`canonical_run_initial_parts`记录本机准入时的准确占位Part。旧库增表升级，不根据
最早Part猜测历史Run的占位关联；旧版仍在执行的Run恢复需另行验收。
Store续接前后读取、冷重开、无runtime-runs的replica、旧Run写入拒绝与事务回滚
定向回归已通过；完整客户端验收不由这些存储测试替代。

## 必须一起迁移的生产链

1. Application规划Declare/Append/Close；Kernel准入所有者、状态与预期head。
2. Store在commit_transition事务内检查generation/revision/root/长度，写本次新增
   chunks和head CAS；冲突、缺依赖或配额错误回滚整个事务。
3. 同事务写checkpoint、revision及replica增量；replica带新增bytes和head CAS，
   不重新发布完整sealed对象。重放不推进revision。
4. Close可以做一次全文封存/最终digest；普通Upsert明确metadata-only或Replace，
   不能恢复累计content_text/content_json双写。
5. hydrate/context、range/search/export、branch、semantic/nested source、derivation
   与archive读取新backing；不能把空inline槽当空正文。

## 读取前置整改

CanonicalFieldReader是函数内固定快照的原始字段接口。物理实现可以不同：Inline
仍使用一次SQLite借用匹配/有界字符截取，避免对大inline反复substr造成回退；
Sealed用有界range。调用者不依据存储形态选择正文真源。

已迁移conversation projection正文与transcript search、summary和context工具结果
分类。JSON hydration、review投影和双字段branch已经接通Inline/Sealed统一reader，
定向结果见[JSON读取迁移](../../../历史任务/实施与审计/CANONICAL_JSON_HYDRATION_PLAN.md)和
[双字段分支](../../../历史任务/实施与审计/CANONICAL_BRANCH_FIELD_PLAN.md)。这些仍不具备Chunked读取能力；
不得在reader清单与replica未完成时启用新writer。

## 增量事件依据与实现分层

OpenAI Responses 使用带类型的语义事件，正文增量、完成和错误是不同事件，
不是每次都交付一份最终消息。这支持在适配层保留增量与生命周期的区分，
但不规定我们内部数据库的chunk或CAS算法。
来源：[官方流式响应文档](https://developers.openai.com/api/docs/guides/streaming-responses)，
2026-09-28重新获取并核对。

本地参考 `参考资料/opencode-dev/packages/opencode/src/session/processor.ts`
的text-start/delta/end分支：start创建Part，delta传明确session/message/part/field及
新增文本，end更新完成态。这里只借鉴事件职责拆分，不能据此推断其物理存储
已经具备线性复杂度，也不能把它当作Nola端到端性能证据。

当前实现阶段：

- Domain `canonical_stream.rs`定义完整owner、head和Declare/Append/Replace/Close。
  每块最多32768 UTF-8字节；Replace显式增加generation，Close后禁止写入。
  head/root不是读取授权，不证明底层chunk存在。单次CAS重放拒绝；外层事务receipt
  才负责识别已经提交的相同请求并返回原结果。
- Application `canonical_part_projection.rs`中的transition-local planner接入两处
  实际事件循环；Store `canonical_part_mutation.rs`共用于source与replica。新Text
  delta不再调用旧prefix SHA/SQL全文拼接，已有非stream正文暂保留其原策略。
- Store `canonical_stream_store.rs`已是生产模块，schema31增表保留旧会话。提交
  新chunk、manifest、head与checkpoint/同步仍在原事务中；reader已接Chunked。
- 已有每字段256MiB写前上限及Close完整链审计。仍需全Store物理配额/回收、
  各端实际运行验收和2/8/32MiB性能量测；当前结果不能替代这些门槛。

## Chunked读取接线批次（2026-09-28，Store定向验收通过）

读取批次将`canonical_stream_read`接入`published_part_body`和`CanonicalFieldReader`；
当时未启用writer，后续生产接线才加入schema31。未安装stream schema时只读resolver返回
无stream；安装后如存在head，不能退回空inline槽掩盖损坏或所有者冲突。

读取证明分两层：

- 范围和末尾预览：固定快照、持久化完整owner/visibility、当前head、被访问块及
  邻接几何、本地manifest哈希。范围使用offset索引，tail使用反向chunk索引；
  查询额外尾块分别使用index和offset范围索引，禁止用OR条件扫描整代旧块。
- 完整性发布：Close及branch调用`require_complete`审计manifest祖先链、整代
  chunk及尾部布局。不能把局部range的成功称为完整历史证明，也不能声称它能
  检测任意恶意数据库重写后形成的自洽分叉；持久布局由已准入的事务维护。

小stream被branch复制成inline也必须先做完整审计；不能只有大对象复制才检查。
搜索使用已解析owner下的通用有界range，不伪造immutable reference来绕过类型。
原始JSON仍不接受text stream backing，不能把公开journal或正文引用强转成JSON。

消费路径验证包含原始hydrate、Unicode tail/prefix/range/search、可见性/错误owner、
真实export manifest/range、真实branch独立复制及删源后重开、损坏源发布回滚。
Store lib248项、六组integration31项通过，日志和边界记录在总计划。Application
验收单独记录；此结果仍不代表生产writer、replica增量协议和端到端性能已完成。

## 生产写入接线边界

现有`RunEffectRepository`已增加按完整owner查询原始text状态的必需接口，
返回Missing、Existing（Inline/Sealed原始字段）或Stream(head)。不能用Option<Head>
把尚未创建Part与已存在但非stream的正文混为一谈。查询仅从同一connection/snapshot
读取有界身份；owner不符报错，Missing不提供创建授权，commit仍验证声明与消息归属。

`accept_host_event`与`reduce_and_commit_active_submission_with_core`各持有一个
transition-local正文planner，按完整owner缓存一次查询，并按事件顺序推进head。
同批第二个delta必须引用第一个delta的结果，不能重新读尚未提交的旧head。超32KiB
delta按UTF-8边界拆块。不要将持久head塞入Core checkpoint、展示DTO或公开journal。

Part创建/初始placeholder、`complete_assistant_turn`及非delta的`part_upsert`必须
一起纳入，区分metadata-only与明确Replace。新建stream与已有原始字段各有明确
策略，不自动以空head覆盖非stream正文。最终全部操作仍交原`commit_transition`
事务，checkpoint revision与各head CAS共同拒绝读后竞争。source和replica使用相同
操作顺序；事务receipt去重在CAS之前，不能先写head再提交普通transition。

## 生产接线的所有者与生命周期约束（2026-09-28，定向验收通过）

正文的持久所有者由canonical消息的conversation/run/message与Part/field关系证明。
不能要求同步接收方存在来源机器的`runs`执行记录：replica复制canonical消息声明，
而不复制本地执行账本。本机`commit_transition`仍先加载真实Run、验证消息归属；
replica先验证publication及消息的run归属，再调用同一字段mutation入口。禁止为了
让stream可读而在副本伪造Run，也不能把来源提供的hash当授权。

新增`PartMutation::Stream`和不含正文/摘要的`Metadata`。已有stream上旧Upsert与
prefix Append必须拒绝，避免重新制造第二份真源；Metadata不能绕过stream Close
将开放正文标为完成。最后的Part Close只有在字段已关闭后才能记录终态和最终摘要，
摘要从原始字段reader核验，不能对空inline槽做SHA。当前字段格式保持严格 owner/CAS。这里不规定旧版本数据升级、转换或重建流程，不能据此授权自动改写已有会话或删除本地数据库。

Core并非只用`PartUpdate::Completed`表达完成：Created、TextReplaced及review更新
也可带`is_complete`。planner在这些实际完成路径统一Stream Close与Part Close，
否则丢弃旧正文后会留下空但开放的stream。已经关闭的正文只更新review metadata，
不再次替换正文或推进head。真实Core测试同时覆盖正常完成与loop-guard丢弃后继续运行。

## 验收门槛

- Inline/Sealed/Chunked相同Unicode、tail、搜索字面量与字符偏移；同owner/visibility。
- Application delta→Close→replica→冷重启后全部正文消费者一致。
- 错owner、旧root、缺chunk、事务中失败、重放和显式Replace不产生半提交。
- 2/8/32MiB追加记录实际每步读写量；Append不得扫描旧前缀。
- 单独记录Core checkpoint全量序列化成本，不把字段层改进称为端到端线性复杂度。

阶段结果与命令记录于 MODULAR_REDESIGN_2026-09-28.md；本合同存在不等于跨端读取、性能及真实运行验收完成。

## 摘要和上下文分类的读取边界

会话摘要从首个可见user Part的原始content_text读取512字符前缀，不能直接截取
可能为空的inline槽。读取保持同一快照，引用冲突报错而不是回退成空预览。

上下文用户轮数对provider_message的原始content_json做结构化分类，识别协议中的
content对象或数组项type=tool_result；这是数据协议标签，不是自然语言路由。
数组必须完整遍历，不能用“前64项”的展示预算做语义判断。允许有界内存多次扫描，
不允许全文hydrate巨型JSON。重复目标键、损坏JSON或引用错误明确失败，不能误算
普通用户轮。窗口JSON读取的后续实现与验收单列于JSON迁移文档，不以分类测试代替。

## 展示段落投影（2026-11-08）

Apple 的 `Shared/Presentation/AppleStreamingMarkdown.swift` 与 Android 的 `StreamingMarkdownProjection.kt` 按稳定段落和一个活跃尾段呈现。完成时尾段原地稳定，不重新分词并重播全部正文。代码围栏按字符、长度与闭合行匹配；内层较短围栏不能关闭外层代码块。

这是客户端渲染分区，不是新的持久正文或另一个 Part 生命周期。已提交字段、hash/CAS、续接身份与读取权限仍遵循上文；修正/替换文本必须有原始规范事件依据。历史消息、工具事件及检查器开合不得重新播放已稳定的段落。
