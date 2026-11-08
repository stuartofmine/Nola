> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Android canonical 模型身份

会话只持久化 `modelSelectionID`；domain `selectedModelInfo` 是读取目录后的展示投影，
不是设置JSON字段。configured严格使用profileId；local使用精确设备模型ID与安装目录，
缺失/禁用profile或未安装local返回不可执行投影，但仍展示历史。旧selectedModelInfo不回退。

创建先解析有效目录身份与参数，单次create完整settings；切换捕获会话revision后执行单次
patch_configuration（modelSelectionID、modelOverrides、clientModelPresentation），成功才
发布UI/provider。参数编辑捕获模型身份/会话revision，不以重新读取revision掩盖冲突。

WebHost连接入口不是模型：`executionHostBinding={baseURL,tlsCertificateSHA256?}` 保存无凭据连接
引用，`clientHostParameters` 保存客户端参数；Host自行选择具体模型。绑定和清除modelSelectionID
同一事务，切回设备模型时清除绑定；保留Host执行，不声称具体connectedHost模型选择器已实现。

实施后添加纯契约/仓储调用回归，先完成全批代码再统一测试；不清除旧用户数据、不做兼容回退。

## 已实施与边界

- `ConversationModelIdentity` 严格configured profileId/local ID映射；配置目录缺失、禁用、删除、
  ID不符返回不可用。profile标准字段足以产生展示投影，不要求sourceJSON含Android ModelInfo。
- `ConversationRepository` 从canonical设置ID/Host binding派生模型，domain模型快照不参与序列化；
  create一次写完整初值，selectModel/selectHost一次配置CAS；参数编辑同样带编辑起点revision。
- Host参数映射为canonical generation字段，展示项为`clientModelPresentation[execution-host]`；
  该key仅展示用途，不是模型ID。Host连接/生成配置/清除模型选择同一事务，反向选择清Host绑定。
- ViewModel按成功回执发布模型和revision，目录异步owner门禁保留；Host恢复/发送检查绑定URL、
  TLS指纹与安全配置相符后冻结凭据，凭据缺失或绑定不匹配保留历史、禁止借用其他Host执行。
- 测试覆盖不同profile同wire名、缺失/禁用/删除profile、local精确匹配、原子稀疏patch、
  无第二持久身份、Host映射及含凭据URL拒绝。统一验证结果见下节。

限制：Android当前安全设置只保存一个Host配对；切回另一个绑定会话需重新连接其Host，不自动
  回退当前Host。具体connectedHost模型选择器仍未实现，由已绑定Host自行选模型。当前选择CAS
  若页面revision陈旧会明确冲突，需要重载，不盲取新revision覆盖。无真实设备交互验收声明。

## 统一验证（2026-10-15）

Core字段整批就绪后运行 `cd android && ./gradlew testDebugUnitTest`，14秒成功。
113 suites，433 tests：431通过、2跳过、0失败、0错误；新增模型身份5项全部通过。
跳过为显式启用才运行的两项 `OpenAiProviderRealEndToEndTest`，不是本批真实模型验收。
编译日志无warning。证据：`/tmp/nola-android-model-validation.qvLyVw/tests.log` 及 `junit/` XML副本。
JVM结果不等于Android设备UI、真实Host连接、跨设备同步或NDK运行时集成验收。
