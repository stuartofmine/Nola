# 1.0.0 重发与版本基线验收（2026-11-09）

## 范围与授权

维护者要求纠正 README：当前产品无独立 Backend；Nola 自有协议、ABI、数据库格式与 epoch 全部从 1 开始。维护者随后明确取消此前首次上传的 1.0.0 格式适配，要求直接重新发布。此次重发成为后续兼容基线；不转换开发阶段的 54/83/8 等格式，不修改用户现有数据库。

## 变更

- 中英文 README 重写，说明原生客户端、Svelte Web、Rust CLI / Application Service / Store / Host、实际发行目标、构建与测试。移除 Worker/D1/Jinja 和旧 Backend 部署步骤。
- `dev.sh` 只构建前端与 Rust Host，默认绑定 loopback，不启动 Worker，也不按端口杀其他进程。贡献说明和 CI 移除 Backend 前提。
- 当前 Nola 数字及字符串协议、ABI、schema、epoch 归一为 1；同步 Rust、Swift、Kotlin、Web 生产者/消费者、头文件、冻结调用、恢复快照、摘要域和金样本。外部 MCP / A2UI / 模型供应商标准版本保持原值。
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

## 最终字符串协议复核与补测

发布前复核发现 `nola.tool_result.v2` 和 Apple Core context V3 标记未被第一轮数字常量检查覆盖。已同步所有生产者、消费者和测试为 v1/V1，旧 v2 明确拒绝；发布打包器检查三个实际可执行文件，拒绝仍包含 v2 的包。稳定的资源 ID、文件名和外部协议版本不作为格式号改名。

最终构建补测：Rust Agent 589、Store 464（4 跳过）、Host 38、执行证据 13、Python 工具审计 27、证据/矩阵/评估工具 142、发布工具与基线检查 15 均通过；Mac Test 配置 40 通过；iOS 模拟器上下文专项 9 通过；Android 687 通过、2 跳过，应用与测试 APK 构建成功。上述早期 Web、领域、Store 集成等专项结果仍保留；不把早期测试称为最终全量平台 UI 验收。

最终日志：`/tmp/nola-v1-tag-{state-tests,host-tests,evidence-tests,oracle-tests,android,mac,ios,release-build}.log`、`/tmp/nola-v1-owned-evidence-verified.log`、`/tmp/nola-v1-tag-publisher-verified.log`。Mac/iOS 结果为 `/tmp/nola-v1-tag-{mac,ios}.xcresult`。

## 本次重发制品

最终完整 CLI 包：183022194 字节，SHA-256 `01e395e0e0a2ee22d36fe1010a1ab881a01d614db89025374ea2d0f9a0949e31`。此前摘要为 `8a24…` 的中间包被替代，不作为最终发布依据。三个最终可执行文件都包含 `nola.tool_result.v1`，无旧 v2 标记。解压运行验证版本 1.0.0(1)、Web HTTP 200、实际创建数据库 schema/epoch 1；没有调用模型或修改用户数据库。

公开发行页：https://github.com/stuartofmine/Nola/releases/tag/v1.0.0 。发布工具只操作本次三个制品；签名密钥、本地数据库及私有 Git 历史不进入公开仓库。每周签名续期保留，更新托管使用 GitHub。最终线上验证结果如下。

非法 Host manifest 的旧测试曾期待继续启用工具，与现行 fail-closed 实现不符。已加强该断言：无 manifest 保留正常默认值；提供损坏或未知版本 manifest 时所有相关能力拒绝启用，产品校验没有放宽。专项日志 `/tmp/nola-reissue-cli-manifest-verified.log`。

## 最终线上验证

最终代码标签 `v1.0.0` 指向 `283df33`。公开 Release 的三个制品已替换并逐个核对 GitHub 返回的大小和 SHA-256；源码 main 正常快进推送，重发标签使用明确旧 SHA 的 force-with-lease。

- 匿名 `activate_source.py --dry-run`：实际公开 feed 验签、完整 183022194 字节下载与 SHA-256 验证通过，日志 `/tmp/nola-v1-final-live-activation.log`。
- 最终 Release CLI 在隔离 HOME、损坏的项目模型配置下执行 `update check` 返回 `up_to_date`，没有创建数据库。证据 `nola-v1-final-live-check-kakgxd5w/receipt.json`。
- 真实 Rust 更新器：当前 build 检查、未发布 Mac GUI 目标返回 `unavailable`、隔离较低 build 夹具 check/recheck、GitHub HTTPS 跳转、完整包下载、签名长度/摘要校验及原子写入全部通过。证据 `nola-v1-final-rust-download-baq1varb/probe.log`；临时测试例程已删除，未执行下载包或改变安装。
- 发布工具最终 15 项通过，统一 App 版本检查通过，变更路径凭据扫描与 diff 格式检查通过。完整 CLI 扫描的既有失败仍按前述记录保留，不宣称全量通过。
