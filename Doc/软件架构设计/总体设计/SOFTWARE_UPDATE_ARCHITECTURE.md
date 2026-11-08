> 兼容基线与后续升级规则见 [版本兼容策略](VERSION_COMPATIBILITY.md)。

# 软件更新与发布架构

核对日期：2026-11-08。此设计描述当前落地的手动检查、可信下载、GitHub 分发和平台安装交接。实际发布目标及验证结果见本轮验收记录；尚无自动后台检查或静默安装。

## 边界与状态所有者

更新属于 **Host 的软件分发能力**。`core/nola-update-host` 统一处理发布源、签名、版本判断、目标选择和流式校验下载；它只读取 Store 的格式常量，不打开会话数据库。UI、Agent、模型配置和工具路由不拥有安装权限；更新 API 不接受浏览器提供的下载 URL、公钥、文件路径或执行命令。

| 对象 | 所有者 | 当前契约 |
| --- | --- | --- |
| 当前发布身份、渠道、HTTPS feed、公钥 | 随程序构建的 `release/update-source.json` | 发布构建固定；公钥不是凭据，私钥不进入仓库或客户端 |
| 发布版本、有效期、平台包、说明、Store 格式 | 签名的 feed | 验证签名后解析，不执行说明中的 Markdown/HTML/命令 |
| 已安装 Store 格式 | 当前 `nola_store::STORE_SCHEMA_VERSION` / `STORE_SCHEMA_EPOCH` 常量 | 不以远程声明覆盖，不探测或改写用户数据 |
| 检查中、错误、选中发布、下载反馈 | 对应客户端的页面/操作代次 | 导航取消不触发安装，旧异步反馈不覆盖新操作 |
| 下载暂存与完整文件 | Host 私有缓存或 CLI 显式指定的目录 | 校验完成前不出现正式文件；不覆盖已有文件 |
| 安装、系统权限、程序替换 | 操作系统/商店与用户 | 用户明确发起，不自动停止任务或重启 Host |

第一版不注册 Agent 更新工具、不读取模型 API Key、不发送机器 ID/会话内容，也不将更新错误转换为模型调用或“已是最新”。

## 唯一的发布合同

源格式 `schema_version=1`，包含 `version`（三段数字）、全平台单调增长的 `build`、`channel`、`feed_url` 与 32 字节 Ed25519 `public_key`（Base64）。URL 与公钥同时为空表示 **disabled**，只配一项是配置错误。

feed 为 JSON envelope：

```json
{"payload_base64":"<exact manifest bytes>","signature_base64":"<Ed25519 signature>"}
```

对解码后的原始 payload 字节验签，不对重编码 JSON 验签。payload 必须包含 `schema_version`、`channel`、`version`、`build`、`issued_at`、`expires_at`、`store_schema`、`store_epoch`、纯文本 `notes` 与唯一 `targets`。未知字段和未知版本拒绝，不添加别名或回退。

比较使用整数 build，避免 `1.10` 和 `1.9` 的字典排序错误。相同 build 却有不同版本或数据合同是身份冲突；发布 build 较小显示 **ahead**，不降级。新版本数据合同不同显示 **incompatible_data**，不提供更新下载，不转换旧数据。整包更新可以改变内部程序协议，但同一发行包内的 App、Host、Core、service 和 Web 必须共同构建；连接的其他客户端仍遵循已有版本不匹配拒绝规则。

feed 上限 1 MiB、说明上限 32 KiB、最多 32 个唯一目标、包上限 2 GiB。有效期不得超过 31 天；过期、未来超过五分钟、设备时钟错误均明确报错。发布方须在过期前续签当前发布，不靠客户端忽略过期。每周续签原先已认证的同一发布，包身份不变。未配置 Actions Secret 时不能宣称已启用自动续签。当前没有持久化最高见过的 feed 序号；签名重放受有效期限制，这是当前明确的限制。

所有网络地址必须是 HTTPS、不能包含用户名密码或 fragment；最多五跳，只允许同源和 GitHub 到官方 release-assets.githubusercontent.com CDN，不退回 HTTP。使用系统/库正常 TLS 验证和固定的无身份 User-Agent，不携带模型凭据、Cookie 或浏览器身份访问发布源。

## 从检查到安装

1. 用户明确检查。加载编译时源，未启用时不发请求；下载并验签 feed，再校验格式、渠道、时间、目标与数据合同。
2. 显示当前版本、新版本、纯文本说明；分别呈现 disabled / up_to_date / ahead / unavailable / incompatible_data / available，网络/验签失败是错误状态。
3. 用户请求下载时重新获取 feed 并校验 `release_id=SHA256(payload)`。发布已变化时要求重新检查，不能把旧确认用于新包。
4. 以 64 KiB 块写入同目录临时文件，检查累计大小及 SHA-256；短包、超长、错误 digest、断线都销毁临时文件。flush 后以不覆盖语义发布正式路径。没有解压、执行脚本或替换程序的步骤。
5. 用户选择系统安装器/商店。安装前结束相关任务；本机制不为用户杀进程。Mac 标记互联网下载的 quarantine 信息；Android 先核对 APK 应用 ID、versionName、versionCode 与已签名 offer，再使用 FileProvider 和系统安装授权；Windows 保存后显示文件位置，用户自行打开安装器。OS 仍负责平台签名和安装判断。
6. 新程序正常启动时仍由现有 Store 打开合同拒绝不匹配格式，绝不自动删除、重建或转换用户数据库。

HTTP 请求有连接和总时长上限。当前原生 FFI 是 IO worker 上的阻塞传输；页面取消会取消展示及后续安装交接，但不能立即中断已经进入的 Rust 阻塞请求。缓存由系统管理，尚未实现跨启动断点续传、百分比进度、静默替换或自动重启。这些限制不影响验签和原子发布。

## 各端落地

| 平台 | 检查与下载 | 安装交接 |
| --- | --- | --- |
| macOS 直装 | Shared SwiftUI → 独立 FFI → update Host | 校验后的 DMG/PKG；显示文件、用户打开系统安装器 |
| iOS | 同一 FFI，只接受 `ios` 的 Store artifact | App Store/TestFlight HTTPS 链接，不下载可执行包 |
| Android 直装 | JNI → 同一 Host，精确架构目标 | APK、FileProvider；未授权时打开该应用的安装来源设置，返回后再点安装 |
| Android 商店 | `-PnolaDistribution=store`，目标 `android-store` | 只接收 Play Store 链接；构建 overlay 不声明 REQUEST_INSTALL_PACKAGES，合并后构建检查禁止该权限 |
| Windows | 共享设置 UI → 当前本地 Rust Host | 已验包以原生下载管理器保存；完成后显示文件夹，不自动运行 EXE/MSI |
| Web | 检查的是连接的 Host，UI 随该 Host 整包发布 | 原生下载导航流式保存，避免在 JS 中建立 2 GiB Blob；浏览器自身不安装 Host |
| CLI | `nola update check` / `download <absolute-directory>` | 下载完整 archive 后由用户停止受影响进程并安装；不直接覆盖当前运行的 nola |

Windows 目标按运行包的 x64/arm64、CPU/Vulkan/CUDA 精确选择，不把 GPU 安装包静默换成 CPU 版。暂不支持的目标明确拒绝。Mac App Store 分发、增量更新和包管理器代管不在当前实现范围，不能套用直装安装器。

CLI 更新入口在模型配置解析前；`--source` 仅用于用户明确指定的本地公开源，不能修改已安装版本身份。`nola --version` 输出同一发布版本和 build。源配置没有环境变量别名或搜索多份旧源的机制。

## Web 与原生边界

- `GET /api/software-update`：只读本地版本和是否启用，不访问发布服务器。
- `POST /api/software-update/check`：接受空对象；要求同源，延续已有远程登录与 CSRF 校验。
- `GET /api/software-update/package?release_id=<64hex>`：只接受精确发布摘要和同源导航/Origin，延续登录；不接受来源 URL、公钥或磁盘目录。只产生本次下载的私有临时文件，不安装。验完包才发送 attachment，传输结束清理临时目录。
- Windows 只拦截当前 owned Host 的这一精确下载地址；错误 JSON 不能当成 EXE 保存。没有 renderer shell 执行入口。
- `nola_core_software_update_json` 为独立、受限的 JSON FFI，operation 只有 check/download；复用现有字符串释放函数，不扩展 Application 写命令或改变当前会话协议。

## 发布工程与尚需配置

`prepare_version.py` 在发行前同步 Apple、Android、Windows 与更新源身份；iOS Info.plist 使用构建变量，避免固定版本遮盖版本更新。Cargo 内部 crate 版本不代表 App 发布版本。Xcode 输入依赖显式包含发布源文件，Cargo 追踪 include_str，修改公钥/源/版本后必须重新构建所有分发包。

`sign_update_feed.py` 使用离线 OpenSSL Ed25519 私钥，对实际包计算大小和 digest，不复制/输出私钥；输出不覆盖现有 feed。Rust `verify_feed` 工具在上传前校验签名和完整合同。先上传平台签名后的完整包到 draft Release，再验证每个目标，发布 Release 后最后提交签名 feed；保留分发包供人工恢复程序，数据格式不匹配时仍不转换旧数据。密钥轮换需要现有信任链能验证的新应用发行包，当前不支持靠网络 feed 添加任意新信任根。

公开发布目标为 `stuartofmine/Nola`；源码和签名 feed 位于公开 main，二进制包位于 Releases。源文件已配置公开公钥和 raw.githubusercontent.com feed URL；实际上线结果以本轮验收记录为准。发布私钥不进入仓库，自动续签需要 GitHub Actions Secret。首次只发布真实完成验证的目标，没有包的其他平台显示 unavailable，而非误报最新。Cloudflare 更新分发的准备代码已经移除；现行客户端不依赖独立 Backend。

发布步骤见 [release/README](../../../release/README.md)，逐端结果见[本轮验收](../../当前任务/SOFTWARE_UPDATE_ACCEPTANCE_20261108.md)。

## 平台依据

商店应用的可执行代码更新遵守 [Apple 审核规则 2.5.2](https://developer.apple.com/app-store/review/guidelines/#software-requirements)。Android 直装授权遵循 [PackageManager.canRequestPackageInstalls](https://developer.android.com/reference/android/content/pm/PackageManager#canRequestPackageInstalls())，商店构建不携带该安装权限。Windows 下载交接使用 [Electron downloadURL](https://www.electronjs.org/docs/latest/api/web-contents#contentsdownloadurlurl-options) 与 [DownloadItem](https://www.electronjs.org/docs/latest/api/download-item)，没有把浏览器里的链接直接当成本地可执行命令。
