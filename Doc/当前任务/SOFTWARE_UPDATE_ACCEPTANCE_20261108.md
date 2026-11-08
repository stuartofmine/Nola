# 软件更新验收记录（2026-11-08）

## 当前状态：改用公开 GitHub 分发

维护者已同意使用 `stuartofmine/Nola` 公开仓库。以下早期 Cloudflare 准备记录仅保留为过程证据，已撤销本轮新增的 Worker 更新路由和 R2 绑定；现行方案不依赖 Cloudflare 更新托管。客户端源已配置公开 GitHub raw feed 和 Ed25519 公钥；实际 Release、匿名下载、真实 Rust 下载链路及 GitHub Actions 自动续签均已验证，详见下节。首包范围为 macOS Apple Silicon 完整 CLI，其他平台未提供包时显示 unavailable。

源码公开采用独立初始历史的审查快照，不推送原私有仓库历史，不复制证书、签名密钥、数据库、日志或构建目录。现有 GitHub 凭据写入 Actions Secret 返回 403；维护者已自行配置正确名称的 NOLA_UPDATE_SIGNING_KEY。真实 Actions 工作流已完成续签并推送，自动每周续签现已启用。

## 首次公开发布

- 公开源码根提交 `4e56147849ae1f706d1a328cf102d6b67cf803d7`，无父提交，独立于私有开发历史。审查快照 7205 文件，排除 cer 目录中的证书/私钥及 Android local.properties。
- 真实 Release：https://github.com/stuartofmine/Nola/releases/tag/v1.0.0 （明确标为开发预览）。仅包含 macOS Apple Silicon 完整 CLI，无其他平台安装器，不冒充 Developer ID 公证包。
- Release 优化构建成功；完整包 183020819 字节，SHA-256 `dca9be4e40446c1e4ff0efc999e97a443f2542ca08bb431792acd151fb42148e`，GitHub 上传返回摘要和大小一致。
- ZIP 完整性、解压后 CLI 1.0.0/build 1、托管 Node v24.20.0、隔离数据库/配置的 Web HTTP 启动通过；没有调用模型或访问用户会话。
- 新增 GitHub 有限 HTTPS 跳转与缺失平台状态用例后 Host 14 项、CLI 3 项契约通过；OpenSSL 3 发布工具 9 项通过。macOS 自带旧版 openssl 不支持所需 Ed25519，工具/文档要求 OpenSSL 3。
- 初次打包因依赖文件时间早于 1980 失败；修正 ZIP 时间戳处理，并采用暂存后不覆盖发布，重建包和完整解压验收通过。失败产物没有上传。
- 匿名线上验收通过：`activate_source.py --dry-run` 使用公开 raw feed，经 Rust 验签并完整下载实际 183 MB Release 包核对 SHA-256；真实 Release CLI 在损坏的项目配置下返回 up_to_date，未创建会话数据库。
- 临时隔离发布夹具模拟较低 build，使用真实 Rust Host 完成线上 check/recheck、GitHub 有限重定向、流式下载、长度/哈希和原子写文件；macOS GUI 缺包返回 unavailable。夹具不修改已安装版本，也不执行下载的包；临时例程已删除。证据 `nola-rust-live-download-8oz15ql8/probe.log`。
- 公开仓库 CI 尚非全绿：Linux 更新合同通过；首次在线本地化检查将 Swift 动态拼接 key 前缀误认为完整 key，已修正提取器并加回归测试。10 项检查器测试及 iOS/Android/Web 质量检查通过，不添加虚假翻译 key 或关闭检查。Mac 质量审计另发现大量原有缺失翻译（9700 项），未作为本次发布修复范围，不能宣称全平台本地化完整。macOS 更新 job 被取消，Windows job 仍运行。
- 真实自动续签通过：https://github.com/stuartofmine/Nola/actions/runs/42496279148 ，结果 success。GitHub bot 提交 `b7f653b` 仅改 feed；新签名由本地当前 Rust 验证器接受，逐字段比较确认只有 issued_at/expires_at 变化，版本/目标/包摘要不变。后续每周自动续签已启用。手动 dispatch API 的 403 已记录，工作流通过自身配置变更触发完成首次实跑。

## 早期实现与验证记录

# 软件更新机制实现与验收

日期：2026-11-08；本地当前工作树。单版本约束适用，不引入旧格式转换、兼容回退或自动重建。对应[架构](../软件架构设计/总体设计/SOFTWARE_UPDATE_ARCHITECTURE.md)、[交互](../UI和交互设计/SOFTWARE_UPDATE_EXPERIENCE.md)、[发布工具](../../release/README.md)。

## 实现范围

- 新 `nola_update_host` Host 组件：Ed25519 精确字节验签、有效期、渠道与单调 build、平台目标、当前 Store 格式检查、纯文本说明、HTTPS、大小边界、流式 SHA-256 与不覆盖的原子文件发布。
- 当前源 `release/update-source.json` 随程序编译；feed/public_key 为空时 disabled 且不访问网络，不能误报最新。
- 下载前重新验签并绑定 publication digest；变更、损坏、短包、超长、断线都不能变成可安装包。没有后台执行或杀进程逻辑。
- iOS/Mac 共用 SwiftUI 控件与受限独立 FFI；Android JNI 与设置入口，区分直装/商店构建；Web/Windows 共用设置和 Host API，文件经过 Host 校验后由下载管理器保存；CLI 更新入口独立于模型配置。
- Android 直装使用 FileProvider 与系统安装来源授权；商店 overlay 不声明安装权限，合并后构建检查禁止该权限。Mac 下载打上 quarantine 标记。Windows 只接受 owned Host 精确下载 URL 和预期包类型，完成后显示文件夹，不自动执行安装器。
- 发布版本统一到 1.0.0/build 1（内部 crate 版本仍独立）。iOS Info.plist 使用构建变量，发布脚本同步 Apple/Android/Windows/源，并提供只读一致性检查。
- 离线签名工具计算实际文件大小和 digest，私钥不进入客户端；Rust 发布校验工具与只读 CI 合同测试已接入。没有自动上传、生产发布、真实应用安装或修改用户数据。

## 已核对证据

| 范围 | 结果与证据 |
| --- | --- |
| 更新 Host | 13 项契约测试通过，`/tmp/nola-update-rust.log`：签名篡改/错误公钥、过期/未来/渠道/格式、目标与存储合同、大小/路径/摘要、原子下载、禁止覆盖、变更 offer、错误状态 |
| CLI / HTTP | 3 项更新入口和真实本地 HTTP 测试通过，`/tmp/nola-update-cli-tests-final.log`；验证 disabled 不误报、跨源拒绝、源 URL 注入拒绝、方法限制、摘要格式；产品二进制构建通过 |
| Windows | TypeScript 构建与 7 项单元测试通过，`/tmp/nola-update-windows-build-final.log`、`/tmp/nola-update-windows-tests.log`；其中新下载边界测试覆盖当前 Host、重复/附加参数、跨源、凭据和非法摘要。未执行 Windows OS 下载/安装实测 |
| Web | Svelte 零错误零警告、原有 610 项测试与生产构建通过；`software_update_regression.cjs` 三组桌面/320px 深浅色浏览器夹具通过，测试失败重试、不误报最新、纯文本说明、真实浏览器下载事件、下载请求语义、数据格式阻止与无横向溢出；已复核深色 320px 截图。证据 `/tmp/nola-update-ui/receipt.json` 与截图 |
| 发布工具 | 两项 Python 工具测试通过，包含 dry-run、同步版本、单调性、签名、包大小/摘要、禁止覆盖、公钥不匹配及私钥不输出；`/tmp/nola-update-publisher-tests-final.log` |
| 签名互验 | OpenSSL 3 工具产生的隔离假包 feed 由真实 Rust verify_feed 接受，`/tmp/nola-update-cross-language-verify-final.log`；fixture 目录 `nola-release-cross-language-3y7p8kdn`；测试私钥已移除，只保留公开 feed 和校验材料。不是实际应用包或生产密钥 |
| Mac | 2 项原生 XCTest 通过，真实 FFI 拒绝 install 操作、精确发布摘要与纯文本说明投影；Test 构建通过，`/tmp/nola-update-mac.xcresult`、`/tmp/nola-update-mac.log` |
| Android | Debug 应用/测试 APK、JVM 测试、lint 构建通过；只读 AVD 独立用户中的更新控件/JNI 用例 `OK (1 test)`，`nola-android-update-sox0xb9r/receipt.json`，不调用模型或安装应用。商店权限审计结果在下方记录 |
| iOS Simulator | Test 构建和两项原生 XCTest 通过；真实 FFI 拒绝 install、精确摘要/纯文本投影；`/tmp/nola-update-ios.xcresult`、`/tmp/nola-update-ios.log`。未使用或轮询物理手机 |

版本只读检查 `python3 scripts/releases/prepare_version.py --check` 为 1.0.0/build 1；`--dry-run` 未实际升为 1.0.1。签名/下载测试使用隔离 mock 包和临时目录，不执行任何测试包。

## 失败修正与边界

第一次 Android 构建发现未开启 BuildConfig；开启后发现 manifest tools:node 不接受占位符。已改为明确的直装/商店 overlay 和合并后权限审计。第一次 Windows 构建暴露新增文案缺项及继承覆盖顺序，已补齐并重新通过。原始失败日志保留，不写成首次全通过。

当前是手动检查与显式安装机制；没有后台自动检查、静默覆盖、差分包、断点续传、精确下载百分比或跨启动最高见过版本缓存。原生取消只取消展示/后续交接，阻塞请求本身受连接/总时长限制。Windows 在 Mac 上只做编译/单元与共享浏览器验证，不冒充 Windows 系统实测。

生产尚缺真实 HTTPS feed、公钥、商店 ID、平台签名/公证与实际跨版本安装验收。默认源保持 disabled，未伪造域名或密钥，没有把当前开发 Debug 应用替换成测试或 Release。提供这些发布配置后才能宣称线上分发生效；不把本地测试通过扩大为实际生产升级、自动重启或用户数据兼容证明。

Android 商店 merged manifest 审计最终通过：直装包含 REQUEST_INSTALL_PACKAGES、商店不包含，证据 `/tmp/nola-update-android-permission-audit.json`、`/tmp/nola-update-android-store-final.log`。构建审计也会拒绝依赖引入此权限。早期 overlay 的无匹配移除警告已通过空的商店 overlay 与构建后断言消除，未隐藏警告。

CLI 实际二进制额外验收通过：在损坏的项目配置下 `--version` 与 `update check` 仍正常工作；显式源文件中伪造的 99.0.0/build 999 不改变已安装身份；disabled 下载明确失败且未创建包目录或会话数据库。证据 `nola-update-cli-smoke-_k_6om54/receipt.json`。

最终 Android 补充复测：`nola-android-update-final-5o0ms0f8/receipt.json` 两项 instrumentation 通过，涵盖检查入口/JNI 拒绝安装，以及使用真实已安装 APK 解析的应用 ID/版本校验。当前应用包可识别；错误 build 与其他应用包被拒绝，没有实际安装。最新 Android 应用/测试 APK、JVM、lint 构建再次通过，`/tmp/nola-update-android-final3.log`；JVM 共 689 项，687 通过、2 跳过。

补跑架构依赖边界检查为 1 通过、2 失败：现有 `nola_application -> nola_runtime_driver` 依赖违反守卫，连带其变异测试失败，见 `/tmp/nola-update-boundary.log`。已核对 HEAD 中的 manifest 和守卫同样包含此矛盾，本轮未修改两者，也未放宽白名单。现行依赖文档已记录这条既有耦合；不能将本轮更新功能通过写成全仓库检查全绿。

最后检查：版本一致性、Python/Node 语法与 `git diff --check` 通过；发布校验工具同时验证 publisher identity 与当前 Store 格式。没有提交、推送或发布生产制品。

## 请求启用后的部署准备

已检查实际渠道：当前 GitHub 仓库为私有，Releases 为空；现有 Worker 域名为 nola.top。Wrangler 两次检查均为刷新失败（400 / Not logged in），因此本轮没有执行远程部署、创建 bucket、上传包或写入发布密钥。已通过文本输入提示维护者恢复 Cloudflare 登录。

域名最初用 Python 默认 User-Agent 返回 Cloudflare 1010/403；用产品实际 `Nola-Software-Update/1` 复测则可到达 Worker，返回当前路径未部署的 404。未据此关闭全站安全设置，也未将初始 403 当作更新器永久不可达。

本轮准备：只读 R2 发布路由及 `NOLA_UPDATES` 绑定；实际生效的只有本地代码。Worker 类型检查、6 项路由测试及 Wrangler 部署 dry-run 通过，输出 `/tmp/nola-update-hosting-{typecheck,tests,dry-run}.log`。没有 D1 写入或账户/模型权限改动。

新增 `activate_source.py` 启用守卫，只有真实 HTTPS feed 经 Rust 验签和所选平台包完整性核对后才原子写源；并发编辑时拒绝覆盖。发布工具共 5 项 Python 测试通过，`/tmp/nola-update-activation-tests-final2.log`。更新 Host 13 项、CLI 3 项测试重新通过，并将 disabled 测试改为独立夹具，避免正式启用后 CI 访问生产源或因公钥变化失败。

**当前仍未启用线上更新**：Cloudflare 认证尚未恢复，真实安装包、签名 feed 和发布密钥尚未配置。`release/update-source.json` 保持空 URL/公钥，不将部署预演写成已部署，不把占位地址或测试密钥写入客户端。
