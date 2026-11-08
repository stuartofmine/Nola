> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 历史详情的按需读取交互

2026-09-08。保留当前工作区；本阶段把已有 `read_historical_part` 查询接到用户操作，
不改变规范消息、工具权限或投影事件序列。

## 实际入口

| 客户端 | 用户入口 | 查询与生命周期 |
| --- | --- | --- |
| macOS | 消息行“查看历史详情”，sheet 内选择 Part | MacApplicationPort 单 Part 查询；请求绑定owner、canonical revision和窗口序列，离开/更新窗口使选择失效 |
| iOS | ChatBubble 的“Historical details”按钮 | 独立 sheet，经现有 FFI typed query；canonical Parts 变化或关闭使 generation 失效，回包重验身份 |
| Android | MessageBubble 的“Historical details”按钮 | 独立 dialog；UI历史元数据标为 `@Transient`，取消后不发布迟到native结果 |
| Web | 每条历史消息下的“历史详情”按钮 | 独立dialog，以固定owner/revision GET查询；加载/失败重试/关闭，与实时bubble reducer分开 |
| CLI | `nola session part <conversation> <message> <part> <revision>` | 单次有界查询，输出typed JSON；revision必须为正整数，不加载整个SavedSession |

CLI 示例：`nola session part c m p63 1`。身份与 revision 应来自当前规范投影；
过期时重新获取会话窗口再查询，不自动将请求改成最新 revision。
Web GET为 `/api/chat/conversations/{id}/historical-part?message_id=…&part_id=…&revision=…`，
仍处于既有Web认证边界内；不是独立匿名原文服务。

所有详情最多请求32768编码字节，完整字段保持原值（包含合法null），未加载字段仍为Deferred，
整Part未加载仍为NotLoaded，格式不支持仍为Unsupported并可显示规范文本预览。
显示原URI/权限字段只是历史数据，不创建链接执行、工具事件或权限恢复动作。
本阶段**没有增加超大原字段的range下载交互**，不得把详情打开等同于原字段全部下载。

## Web与CLI实际修复

`cli/src/historical_part.rs`是CLI命令和Web route共用查询adapter，使用Protocol DTO并重验owner/revision及响应预算。
窗口WebMessage的展示meta保留historicalParts，使初次打开或分页得到的历史节点都能产生详情入口。
这些仅是展示metadata，未改规范Part，也未将状态复制进另一个数据库。

Web `historical-part-detail.js`以generation+AbortController控制一个当前详情请求，
返回后必须仍是相同owner/revision/fingerprint；dialog用textContent显示数据，绝不把历史字段作为HTML执行。
应用新分页/规范窗口前关闭旧详情；已开始分页→打开旧行详情→分页返回的时序也会丢弃迟到详情结果。
实时流更新只刷新描述性历史入口，不用查询结果伪造projection事件。

真实测试发现CLI原先对每个命令执行cleanup及orphan Run recovery，会使历史查询额外写入收据。
现在显式Sessions/SessionShow/SessionPart读取跳过这两个通用预检；SessionPart的成功/过期查询均不产生写入收据。
本结论针对这条查询及预检，未宣称所有既有SessionShow内部包装都已完成客户端职责清理。

## 本阶段验证

- Mac原生：MacHistoricalPartDetailTests4项 + MacCanonicalSessionProjectionTests10项，14/14通过。
  包含真实64Part Service query、原window/messages/eventSequence不变及迟到响应拒绝。
  `/tmp/nola-history-detail-mac-native.log`。
- iOS模拟器：IOSHistoricalPartDetailTests2项 + 投影契约9项，11/11通过。
  初次缺Combine的编译错误已修复，最终日志`/tmp/nola-history-detail-ios-final.log`。
- Android JVM：历史详情3项 + 投影契约5项，8/8通过。
  `/tmp/nola-history-detail-android.log`。
- CLI参数解析1/1：显式revision、缺模型配置仍可解析、错误revision拒绝；
  `/tmp/nola-history-detail-cli-parse.log`。
- 真实Web/CLI/Service跨进程1/1：64Part初窗保留NotLoaded，HTTP与CLI单Part结果一致，
  owner/revision错误拒绝，窗口/原JSON/receipt不变，Service冷重启后结果相同；
  并从真实HTTP页面取得脚本做Node语法检查。`/tmp/nola-history-detail-web-cli-final.log`。
- Node交互状态与真实分页函数回归：关闭、换会话、重试、错身份、超预算、unknown状态、
  null/Deferred保持，以及先分页后打开详情的竞态；`node web/tests/historical_part_detail_test.js`通过。

**尚未验证**：完整浏览器点击与截图、三端原生视觉截图、移动真实UI点击到native库的全链路。
本轮按Browser技能初始化后确认可用浏览器列表为空，未绕过该工具冒充浏览器验收。
原生测试、HTTP和Node状态测试各有明确范围，不能替代ARCHITECTURE_V2 §20的截图、
真实模型、多窗口性能或五端全量一致性门槛。
