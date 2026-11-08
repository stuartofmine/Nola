> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Canonical JSON hydration 迁移计划

2026-09-28。状态：**读取实现已接入，本批 Store/Application/CLI 验收通过**。
隶属 [Canonical 流式正文合同](../../软件架构设计/模块/13-客户端投影/CANONICAL_STREAMING_TEXT_CONTRACT.md) 的读取前置整改；
不是 Chunked writer 已上线或所有 JSON 消费者已统一的声明。

## 迁移前问题（保留为验收依据）

- `core/nola-store/src/conversation_read_repository.rs` 的
  `load_conversation_projection_with_window` 已通过 `CanonicalFieldReader`
  统一正文 text hydration，但 JSON 仍直接来自 `message_parts.content_json` 的 SQL。
- 当前 `part_json_limit` 按 SQLite `length(TEXT)` 的字符数判断，0 表示不限制。
  普通超限 JSON 返回 NULL；text Part 若有非 null 的 review，则用 `json_set`
  替换 `$.text`。后者不会限制 review 或其他字段的大小，不能视为硬输出预算。
- 同路径的 `open_text_projection` 明确拒绝 deferred JSON。这是当前消费者的
  安全限制，不能在 JSON 真源尚未接入时全局移除。
- `bounded_message::message_bytes` 已计算两字段原始 UTF-8 字节数；
  `load_parts` 验证 JSON 引用后不 hydrate JSON，返回的 `content_json=None`
  表示未加载，不足以证明原始字段缺失。
- `CanonicalFieldReader::read_range` 可读取原始 JSON；`read_text` 是文本投影
  接口，不能用它把 JSON 重新解释成普通正文。Historical/semantic reader 已有
  owner-bound 有界字段提取能力，应复用而非另造 JSON 真源。

## 所有权与原始 JSON 保真

原始字段以完整 conversation/message/part/field 身份，在同一固定读快照中解析。
Inline、Sealed，以及将来的 Chunked backing 不得由调用者自行猜测。
错 owner、冲突引用和未发布引用必须报错，不能降级为 absent 或空正文。
完整读取遇缺 chunk 必须报错；预算导致 Deferred 时仅验证绑定，不扫描未读取的
全部 chunks，因此 Deferred 不是完整性验证通过的证明。

完整原始 JSON 读取必须逐字节保留空白、字段顺序、转义、重复键及数字词法。
不能通过 `serde_json::Value` 或浮点数中转后重编码来实现 hydration；这会改变
原始字节，并可能改变大整数、长小数或指数形式。语义消费者可以验证并拒绝
歧义字段，但不得将其重新编码结果冒充原始字段。

公开 assistant journal 的展示引用不是原始 JSON 的读取授权。此迁移不转换
ToolInput/ToolOutput 展示引用，不改变原始字段所有者和可见性约束。

## Reader 私有结果语义

建议在 Store 内部引入结果类型，名称可随现有代码风格调整：

| 结果 | 意义 | 禁止的解释 |
| --- | --- | --- |
| `Absent` | 权威字段解析证明原始 JSON 不存在 | 不能由空 inline 槽推断 |
| `Complete(String)` | 原始字段完整且在明确读取额度内，字节未被改写 | 不能装入裁剪后的 JSON |
| `Deferred` | 原始字段存在，但本次读取预算不足或此路径不加载 | 不能当作缺失，也不能当空对象 |

`Deferred` 应保留 reader 已有的精确 owner 和原始长度事实供当前调用使用，
不新增可跨请求携带的授权 handle。JSON 文本 `null` 是 `Complete("null")`，
不是 `Absent`。预算不足不是错误；身份或存储损坏仍是错误。

私有读取/投影交接保留该三态。仅有 `StoredPart.content_json: Option<String>`
不能表达全部状态；Store/Application Part读DTO增加 `content_json_deferred`，
将预算省略证据传给显示消费者。false+None表示缺失，true+None表示未加载；
投影JSON有值但不携带原始digest。此标记不写入数据库，也不是调用工具的授权。
历史模型读取仍使用原始字段接口，不能从显示DTO反推原始 JSON 是否存在。

## 预算单位与显式语义收紧

建议将**有界 JSON hydration** 的限制统一为 UTF-8 字节数，与窗口内容预算一致。
这是从当前字符数限制迁移的显式行为变更，不是等价重构：相同数值预算下，
多字节文本可能更早 Deferred。参数名、内部注释和测试必须表达字节单位，
不能仅替换实现而继续宣称旧字符语义。

- 明确区分原始字段字节额度、窗口累计内容字节额度和最终传输帧编码额度。
  JSON 再嵌入外层响应时的转义及 metadata 开销不能假定为零。
- 保留显式无界导出路径；不能因私有 API 使用 0 而让一般窗口误入无界读取。
- 有界读取不得为了决定是否超限先分配全文。长度事实和有界 range 用于准入，
  跨 UTF-8 边界截取只能用于预览，不能产生伪完整原始 JSON。
- 底层 range 最小读取额度为 4 bytes；短 token 应读取合法范围后在已验证 token
  终点截断，不得把 `{}` 等合法短值视为错误。

若本阶段必须完全保持旧字符限制，应先保留独立字符预算适配层并明确标注；
不得将其与新的硬字节预算混称。建议一次批准并测试上述字节预算收紧，避免双义参数。

## Review 投影

Review 是展示/上下文投影，不是原始 JSON hydration 的职责。

1. 先从权威原始 JSON 中获取受验证的目标 token span，复用现有有界语义提取。
2. 如需改写 `text`，仅替换该 token；其他原始字节保持不变。新增目标字段时
   同样需要明确对象结构规则，不用全文 Value round-trip。
3. 重复目标键等歧义必须明确失败或返回不支持，不能任意选择后宣称 canonical。
4. 对**最终投影**计算实际预算。巨大 review 或其他字段无法容纳时 Deferred，
   不能沿用旧行为把仅缩短 text 的巨型对象完整返回。
5. 投影结果必须与 `Complete(original)` 分开，不能继承原始内容 digest，
   不能反向写入原始 JSON。

第 4 项是对旧 review 特例的语义收紧，应独立记录并测试。不能同时声称
“完全保持旧 review 输出”与“现在满足硬字节上限”。

## 最小实施顺序

1. Reader 增加原始 JSON 的私有有界结果及 exact range 集成；不改 writer。
2. Repository 分别打开 text、JSON 字段，去掉 JSON inline SQL 真源，保留同一
   读快照；把预算和 review 投影处理放在显式调用层。
3. 仅在该消费者能正确处理 JSON backing 后移除其 text-only 门禁。其他尚未
   迁移的消费者继续保留拒绝，不能全局放宽 `open_text_projection`。
4. `bounded_message` 复用 reader 的身份、长度和范围接口，并通过明确 deferred
   状态接入现有 historical reader，不全文 hydrate 大 JSON。
5. 完成同源 fixture 的跨路径验证后，再更新 reader inventory。此步骤不授权
   局部启用 Chunked writer；branch、replica 等仍须按总合同闭合。

## 测试矩阵

| 维度 | 必测行为 |
| --- | --- |
| 真源一致 | Inline/Sealed 同 owner、同原始 JSON，完整结果逐字节一致 |
| 数值与词法 | `2^53+1`、Int64 两端、长小数、指数形式不被重编码；转义和空白保留 |
| 三态 | 原始缺失、JSON null、空对象、预算 Deferred 各自可辨 |
| 字节预算 | ASCII/中文/emoji，恰好额度及超 1 byte；窗口累计额度不溢出 |
| Review | 无 review、null review、存在 review；text 替换；巨大其他字段最终 Deferred |
| 小范围读取 | `{}`、`[]`、短标量、跨 chunk Unicode 和 token 末尾不足 4 bytes |
| 窗口内引用 | deferred JSON 本身位于选中窗口内，不能只用窗口外分类测试代替 |
| 不完整结果 | Deferred 保留消息/Part 身份和分页 cursor，不能伪造成空原始 JSON |
| 错误 | 错 owner、缺 chunk、冲突 inline/reference、未发布引用均报错、不 fallback |
| 歧义 | Review 目标重复键明确拒绝或不支持；原始读取仍不擅自重写 |
| 资源界限 | 巨 JSON 不全文分配；记录读量/扫描次数，不把有界内存误称单次线性扫描 |

## 当前实施边界

已新增reader `read_json(Option<usize>)`，Inline单次SQL、Sealed有界range，完整原文
不重编码；repository不再直接SQL取JSON，而是独立解析两个字段。review通过已验证
token span只替换text，预算按最终UTF-8字节数核算，重复目标键报错。bounded_message
仍不hydrate JSON，现通过统一reader计量/预览，并明确携带JSON deferred标记。

CLI的tail是显示/输入历史恢复路径，模型checkpoint与完整历史恢复不使用它；但显示
仍需拒绝把deferred的review metadata当作无需审查。对应DTO与CLI消费已一起验证，
不会把未知状态草稿当最终答案，也不会把空的延迟provider预览当工具事件。延迟预览
当前保守省略，后续可补显式未加载提示及按需加载交互；不能把省略称作完整展示。
其余text-only消费者、publication/replica、branch门禁继续保留，Chunked writer未开放。
实际命令和通过数量记录在整体改造文档，本文不将未完成的跨层验证算作完成。
