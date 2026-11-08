> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 原文读取与远端快照边界

2026-10-01，实施与定向验收中。补充现有canonical流式正文合同，不替代未完成的
公开journal或版本化范围读取设计。

## 读取资格由原始Part事实决定

- window的deferred标记只表示预览未包含全部正文，不授予读取资格。
- `DeferredTextPart.is_final`是Store依据该Part状态输出的必需事实。不能根据整条
  消息是否运行、预览长短或展示Part恰好是Text类型推测；Historical JSON展示也必须
  保留原正文的精确生命周期。
- 无版本参数的range接口仅服务已完成Text Part。消息已完成，或存在可信的内部续接
  归属关系，才允许读取；还须检查完整conversation/message/part、visibility与backing。
- 续接时同一消息里的旧已完成正文可读，新开放正文不可借旧正文资格读取。
- `DeferredTextPart.can_read_original`是Store输出的必需读取资格事实，与range共用
  同一个准入函数。消息失败/取消时Part可能已经final，两种事实不能互相推测。
  `can_read_original=true`必须同时满足`is_final=true`，但它不是跨页版本或授权凭据。
- Mac使用这两项精确deferred事实决定全文入口，Store每次读取仍是授权与状态检查的所有者。
  UI不通过隐藏历史正文能力来绕过共享合同缺字段。

现有分页接口没有generation/root参数，不能据此提供运行中可替换正文的跨页一致性。
后续应由打开原文返回不可变版本句柄，后续range精确绑定版本，另补替换/重开测试；
不能把total byte length相同视为正文版本相同。

## Mac阅读器的查询所有权

`MacOriginalTextReadModel`现在是实际sheet的view-local查询所有者。每次翻页、换Part、
重试、关闭和`onDisappear`同步取消旧`MacApplicationQueryCancellation`，range端口
将该token传到ServiceClient；取消可以关闭正在读取的socket，不只是阻止UI回写。
view-local generation与精确selection共同拒绝迟到结果，预取消Task不能重置当前页
或启动新查询。旧window helper同样传递token，不再维持只有Task取消的旁路。

generation在这里是请求生命周期，不是正文版本。现有total length校验仍不能保证
跨页正文一致性，也不会因查询取消接通而获得版本语义。取消读取socket不声称已中断
服务端所有SQLite计算。四项真实Mac回归覆盖导航/换Part、关闭、Task取消、预取消；
预取消token经isolated实际端口被ServiceClient拒绝，迟到回写由可控Reader验证。

版本化合同切换必须一起覆盖Protocol、Application command/dispatcher、StorePort、
Mac端口及实际sheet、CLI `conversation_recall`工具schema/结果。CLI目前也会按owner+
offset直接读；只改Mac并不能闭合模型端跨页读取。每Part首次open获得严格版本，
所有next/previous/start/retry复用它；失效不得静默重新open。关闭释放准确handle，
每页仍验证owner、可见性与审核披露，版本引用不授予私有读取权。未发现其他移动/Web
UI独立消费当前range DTO，但它们共享Host工具的入口也在上述CLI范围内。

## 普通正文与审核候选

canonical保存原始候选正文用于执行恢复和有明确状态的审计，不代表它能成为普通
回复。原JSON的review只有不存在、null或approved允许正文展示；pending、rejected、
interrupted以及重复、未知类型、未知值或无法取得的控制字段均不成为普通正文。
普通全文range、窗口资格、历史text/markdown字段、canonical preview和搜索snippet
共用Store disclosure规则，不能从“Part已完成”或“原字节还在”推测批准。

历史投影没有Core checkpoint时也执行该规则；受限正文的review等状态metadata
保留，正文与fallback preview均移除。原始存储不因展示限制被覆盖或删除。
显式私有审计/导出与普通正文是不同工作流，不通过普通原文接口授权。

当前控制字段提取使用有界内存的两遍JSON visitor：Inline在单条SQLite row内借用
原值分块，避免每块substr重复复制完整inline；Sealed使用有界原范围读取。它仍扫描
完整JSON，尚不是按独立控制metadata常数时间查询。后续checkpoint/metadata拆分须
一并消除该成本，不能据此宣称所有正文读取或执行链已经线性。

## 公开日志历史读取与源端写权限

公开journal的identity包含原创建Run。可信内部续接后，旧journal的cursor与固定
snapshot保留读取及释放资格：原Run/conversation/local source/epoch必须有效，当前
与历史Run的message归属都须由可信续接证明。源端publish/prepare仍只接受当前Run，
历史成员关系不会授予旧Run新写权限。缺失任一可信记录、未知当前Run、伪epoch或
foreign owner均拒绝；不能用强改旧journal identity使其追随当前Run。

这只是Store历史读取合同；Application公开journal生产发布及各端传输仍未接线。

## 远端快照三个预算阶段

1. Store在hydrate前统计原字段UTF-8总长，包括Inline/Sealed/Chunked，避免空inline
   槽绕过8MiB预加载保护。这个长度不是传输编码长度。
2. Application使用Domain有界serializer检查远端envelope的实际JSON编码大小。
3. Application发送/接收共用展示Part→canonical input转换，并用同一个Domain函数
   检查接收侧canonical parts编码。Store接收也调用该函数，其返回字节直接用于摘要。

三阶段各有不同保护对象，但共用8MiB上限。不能从第1或第2阶段通过推断第3阶段
一定通过：JSON转义、包装字段与当前展示快照冗余都会增加编码大小。
共享编码忽略本地delivery timestamp，保持同一次远端内容重投递的摘要稳定。
有界serializer在超过上限时立即失败，不先克隆所有Part并生成超限Vec后才判断。

此合同修正现有snapshot的准入一致性，不称其已实现增量传输，也不保留不安全的
“发送成功但接收必超限”路径作为兼容行为。全文传输和checkpoint去累计化仍按总计划实施。
