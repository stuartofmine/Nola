# 1.0.0 重发与版本基线验收（2026-11-09）

## 范围与授权

维护者要求纠正 README：当前产品无独立 Backend；Nola 自有协议、ABI、数据库格式与 epoch 全部从 1 开始。维护者随后明确取消此前首次上传的 1.0.0 格式适配，要求直接重新发布。此次重发成为后续兼容基线；不转换开发阶段的 54/83/8 等格式，不修改用户现有数据库。

## 变更

- 中英文 README 重写，说明原生客户端、Svelte Web、Rust CLI / Application Service / Store / Host、实际发行目标、构建与测试。移除 Worker/D1/Jinja 和旧 Backend 部署步骤。
- `dev.sh` 只构建前端与 Rust Host，默认绑定 loopback，不启动 Worker，也不按端口杀其他进程。贡献说明和 CI 移除 Backend 前提。
- 当前 Nola 数字协议、ABI、schema、epoch 归一为 1；同步 Rust、Swift、Kotlin、Web 生产者/消费者、头文件、冻结调用、恢复快照、摘要域和金样本。外部 MCP / A2UI / 模型供应商标准版本保持原值。
- 文件恢复记录统一为版本 1，使用显式 `resource_scoped` 表达授权模式；保留逐资源身份、路径和预算检查，不用版本号代替模式字段。
- 原先“仅有一个未发布版本”的文档约束替换为发布兼容规则。后续变更须维护本次已发布基线；数据库格式变化必须先提供明确、可回滚的升级路径和旧版回归夹具。当前只有格式 1，不虚构未来迁移器。
- 数据库格式不匹配仍严格拒绝，错误信息不再建议“删除重建”。新增对开发格式 83 拒绝且文件字节不变的测试。

## 已通过

| 范围 | 结果 | 本机证据 |
| --- | --- | --- |
| Rust Agent | 589 通过 | `/tmp/nola-reissue-core-final.log` |
| Rust 领域 | 243 通过 | `/tmp/nola-reissue-domain-final.log` |
| Rust Host / 服务协议 | 各 38 通过 | `/tmp/nola-reissue-core-final.log` |
| Store 单元 | 464 通过，4 项既有规模测量跳过 | `/tmp/nola-reissue-core-final.log` |
| Store 集成 | 18 通过，含新格式创建/重开、未知格式不改写 | `/tmp/nola-reissue-store-integration.log` |
| 原生 ABI / 任务 / 生命周期 | 37 通过 | `/tmp/nola-reissue-native-contracts-final.log` |
| CLI 权限 / 恢复 / 更新 | 14 通过；偏好设置追加 5 项、非法 manifest 拒绝用例 1 项通过 | `/tmp/nola-reissue-cli-{permissions,preimages-final,resource-checkpoints,update}.log` |
| 更新器 | 14 通过 | `/tmp/nola-reissue-core-final.log` |
| Web | 610 通过，Svelte 0 错误 0 警告，生产构建通过 | `/tmp/nola-reissue-web-final-{tests,check,build}.log` |
| 发布工具 / 新基线检查 | 13 通过（OpenSSL 3） | `/tmp/nola-reissue-publisher-final.log` |
| Windows | TypeScript 构建通过；未执行 Windows OS 测试 | `/tmp/nola-reissue-windows.log` |
| Mac 原生 | 40 通过，0 跳过，Test 配置 | `/tmp/nola-reissue-mac-verified.xcresult` |
| iOS Simulator | 50 通过、1 项缺录制轨迹跳过；追加冻结模型 5 项通过 | `/tmp/nola-reissue-ios-final.xcresult`、`/tmp/nola-reissue-ios-prepared-model.xcresult` |
| Android | 689 项中 687 通过、2 跳过；应用与测试 APK 构建通过 | `/tmp/nola-reissue-android-verified.log` |
| 真实 CLI 包 | ZIP 完整性、解压执行、Web HTTP、实际 schema/epoch 1 验证通过 | `nola-v1-bundle-verification-_3_21eob/receipt.json` |

没有调用真实模型，不将构建或协议测试写成模型、安装或全仓库 CI 验收。

## 修正的失败与限制

首轮测试存在旧编号断言、把 1 列为非法版本的负例、旧摘要域对应的固定向量，均同步到实际新基线；摘要向量用独立 Python hashlib/struct 计算，不复制被测实现的输出。生命周期金样本缺失现行必填 `execution_source`，已显式补充 `kind=model`，解码器保持严格。

Mac 首轮在金样本改名期间发生旧资源路径复制失败；下一轮发现测试正例仍传入 52，改为 1 后 40 项通过。iOS 首次尝试被 Xcode 测试互斥保护拒绝，后续串行测试通过；保留失败日志，不绕过保护或轮询物理手机。Android 首轮发现未同步的字符串协议检查、数据源连接器下限及测试正负例，修正后 689 项测试无失败。

既有架构依赖边界和 Mac 本地化缺项仍见此前审计记录，本次不宣称全仓库检查全部通过。当前可分发范围仍为 macOS Apple Silicon CLI 开发预览，不发布未经安装验收的其他平台制品。

## 扩展 CLI 扫描的失败边界

- 默认线程栈的全量 CLI 单测因 `task_notification_is_imported_by_original_store_before_boundary_ack` 栈溢出中止；不能宣称默认条件全量通过。
- 使用 32 MiB 测试线程栈继续扫描的一轮结果为 2246 通过、107 失败、8 跳过（`/tmp/nola-reissue-cli-full-large-stack.log`）。该轮包含本机模型配置干扰和若干旧金样本/负例；其中本轮编号归一涉及的负例已修正并另做专项复验。它不是当前全量最终通过结果。
- 隔离配置目录后单独运行配置解析用例为 71 通过、1 失败；剩余用例手工拼出的 Host receipt 缺现行必填策略输入，解码器继续拒绝，不增加兼容回退来放行测试。
- 另有 SOP 等待完成证明、外部执行器初始化/会话身份、缺字段的控制令牌金样本等场景失败。当前协议基线/平台专项验证通过不代表这些问题已解决；未放宽安全校验、移除失败测试或把全仓库 CI 标为全绿。

## 本次重发制品

完整 CLI 包：183022136 字节，SHA-256 `8a24f5b7b93d1f0dc4900484d2fc13815b554129bd5adba3917ec746fafaacf1`。本机签名 feed 经真实 Rust 验证，声明 Store schema/epoch 1。实际上传和匿名下载结果将在远端验证后追加。

非法 Host manifest 的旧测试曾期待继续启用工具，与现行 fail-closed 实现不符。已加强该断言：无 manifest 保留正常默认值；提供损坏或未知版本 manifest 时所有相关能力拒绝启用，产品校验没有放宽。专项日志 `/tmp/nola-reissue-cli-manifest-verified.log`。
