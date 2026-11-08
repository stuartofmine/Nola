# Apple ModelAttempt 展示语义收口

当前状态：共享展示逻辑、三处生产 caller、本地化已落，Mac本批编译与窄测完成。iOS对应两项回归由iOS统一增量报告确认，不把Mac结果当iOS运行证据。

- `fallback_attempt_id` 是已提交 successor 关系。只有 fallback model/provider 非空不能推断重试已经发生。
- successor 存在且已知实际 provider/model 有字段发生变化：中性“模型调用路线已变更”；没有可证明的变更：中性“请求已重新开始”。后者只承诺请求有后继，不声称路线相同，也不声称业务成功。
- 没有 successor 时按当前 Part state 展示 pending/running/waiting/completed/cancelled/failed；只有真实 failed 才采用失败红色。cancelled 为停止、中性展示。
- 主卡模型名称仅取 model 字段，不能把 opaque provider namespace、executionMode 或未知值拼成模型名。现 journal technical disclosure 可保留原始 provider 事实。
- `Shared/Presentation/AppleModelAttemptPresentation.swift` 统一决策；iOS ChatBubble、Mac repository V1 projection、Mac journal view 接同一 helper。Mac 现 trace 的 `.incomplete` 用作中性已 settled 诊断，不修改 Core Run outcome。
- helper 文案统一查默认 Localizable 表：Mac 已补 English/简繁中文，iOS 已补 English/简体中文；不让 Mac journal 查错原 Chat table。

回归源码覆盖 successor 与建议 fallback 的区分、同路线恢复、未知路线、真正路线变更、全部生命周期 state、模型名不包含 executionMode、iOS 实际 Part→生产 disclosure、Mac V1 projection 的最终展示与中文标题。运行结果以本轮主验收报告实际日志为准，不重复真实模型。

## 明确保留的源协议缺口

`Nola/Services/DeviceAgentCoordinator.swift` 当前在 model attempt producer（约1058、1395、1410）把 `activeConfig.executionMode.rawValue` 写作 provider。这不是实际模型 provider namespace。此次只修展示，不在其它原生构建进行时改执行 producer 或冻结模型绑定；也不增加按字符串关键字过滤/映射 provider 的兼容层。后续应从当前可信实际模型 route/factory 的稳定 provider identity 产生该字段，并保持实际 Core invocation/attempt 对应关系。主 UI 不展示该执行模式为供应商，但不得把这个 UI 修正说成源 provider 协议已修。

## Mac 本批实际结果

- `/tmp/nola-mac-ui-validation-20261028/model-attempt-27.log/.xcresult`：Shared helper3、Journal生产模型名/中文标题2均通过；V1 projection17中13通过、4个旧fixture失败。两个直接受影响的succession/route-change projection测试均通过。
- 四个fixture仅修当前schema：工具execution_source由非法数组写法改为object；3个native kernel初始projection显式current contract_version=2与execution_ownership=user。无生产兼容或断言放宽。
- `/tmp/nola-mac-ui-validation-20261028/projection-fixtures-28.xcresult`：selector少括号导致0 tests，明确不计通过，保留记录。
- `/tmp/nola-mac-ui-validation-20261028/projection-fixtures-29.log/.xcresult`：复用已编译Test产物、精确SwiftTesting方法()，四项实际 **4/4 passed、0 fail、0 skip**；xcresult summary核totalTestCount=4。

因此指定的Shared3/Journal2/Projection17各case均已有对应通过证据，但27首轮整次run本身仍保留失败结果，不改写为单次全绿。没有重跑27项父消息投影、姓名25或真实模型。
