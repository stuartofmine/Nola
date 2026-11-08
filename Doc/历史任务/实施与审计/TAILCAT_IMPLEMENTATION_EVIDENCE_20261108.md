# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

## Tailcat 传输接入核对（2026-11-06）

参考为 Tailscale 官方 `tailscale/tailcat`（BSD-3-Clause），不是 Tailscale 账户登录。它提供用户态 WireGuard、NAT 穿透和 DERP 中继，不负责 Nola 的配对、Host 身份、权限、Run 所有权或事件恢复。官方明确 wrapper 实验性，无 API/wire 稳定保证。核对版本：最近发布 v0.7.0；另查当前源码 `e00cdde6ff787f801361ff85ed085775c775d4c5`，包含精确端口映射等后续变化及 Go >=1.27.2 要求，不能混用 CLI 契约或把 main 当固定版本。

### 当前实现与调用链

Nola 已有 Host HTTP/SSE 与 `/api/mobile` 认证加密信封；Mac `MacRemoteHostService`、iOS WebAgentProvider 和 Android `WebHostHttpClientFactory` 消费同一 Host API，浏览器走 Web session/CSRF。传输应只替换连接承载，不新增 Task 引擎或降低 HTTPS/证书校验。

新 `nola server --host 127.0.0.1 --port 5001 --tailcat-ingress`（或 web）只开启**远程入口认证语义**，不会启动 Tailcat/创建密钥。该模式强制 loopback 请求走原 Web 登录及 CSRF，`/api/web/session` 返回 `localTrust:false`、`transportIngress:tailcat`；`/api/mobile` 保持原 token、Host identity 和加密协议。本机限定的 source/Codex、OpenCode、Kimi 账户登录/绑定入口在该 ingress 明确拒绝，不能因转发显示为 loopback 而扩大账户授权。

这是必须先落的边界：直接把 Tailcat 转发到默认本机信任监听器，会令远端请求被误认成本机浏览器。本机 HTTP 原有直接模式不变；未知 peer 不授予本机信任。该标记是 Host 启动配置，不信任客户端伪造的 Forwarded/X-Forwarded-* 标头。

### 最小兼容方案与尚未落地部分

1. 可选可信传输组件只服务 Nola 的**一个精确 loopback 端口**，不得开启 `all`、exit-node、SSH、文件或任意 exec 服务。必须先确认该端口的 Host identity 及 `localTrust:false`，不能只看端口存在。独立认证仍由 Nola 完成。
2. Server 的 client allowlist 必须明确配置；固定源码使用 `AllowClient` 回调，nil 会允许所有；适配器拒绝空公钥集合，不使用该默认值。连接地址可能包含 PSK，按凭据处理：不放普通 URL、DNS、日志、截图或模型上下文。CLI 会把地址打印 stderr，直接继承 stdout/stderr 不适合产品接线。
3. 桌面/浏览器最小开发路径可由已审计的独立传输把本地端口接到隧道，并使用当前 URL API。但已发布 v0.7.0 的 localhost 服务和后续精确目标映射有差异；不可用未确认版本的转发去接任意本机服务。产品应采用固定版本的库级字节流或受控桥，秘密走安全 IPC，不以带 PSK 的命令行参数冒充安全客户端集成。
4. iOS/Android 需要内嵌 Go 库/平台绑定、前后台关闭和重连、路由变化及 secure credential store 的实际验证。不能在 iOS 启动桌面 CLI，也不能把已有 HTTP URL 输入框标成原生 Tailcat 支持。本产品 Web 由 CLI 启动，Tailcat 连接与转发由 CLI 持有，浏览器保持普通 HTTP/SSE，不引入浏览器 WASM；秘密地址不能塞进浏览器 URL。
5. 断开传输只改变连接状态；执行 Host 仍拥有 Task/Run。恢复继续精确 owner/Run/cursor，不重发副作用或制造停止 ACK。公共 DERP 无 SLA；产品不能承诺总是直连、永久可用或后台常驻。

已获用户明确授权后，官方 Go1.27.3（校验官方 SHA256）及固定 Tailcat 提交已在独立开发目录安装/构建，未改系统 PATH。`transport/tailcat` 现在实现可选库级 Host/Client 与私有 stdio 适配器；`nola tailcat-bridge /absolute/program SHA256` 只选择已校验的程序，不传入账户环境，不启动 Nola Run。双方握手先校验精确 Nola Host identity 与 ingress，再对客户端公布本地 origin；未完成握手不会伪报已连接。控制协议只接受 version1，秘密仅走父进程私有管道，普通终端/日志输出会拒绝；关闭连接不改变任务权威。

实际通过：离线准入/状态检查与内存流测试；真实 WireGuard/DERP 合成 HTTP/SSE 用例（401 保留、完成前首帧、精确字节、关闭监听）；Rust入口到 Go进程的隔离连接。上述 HTTP origin 是协议样例，**不是用户实际 Nola Host、Mac UI 或四端成品连接验收**。没有创建持久设备密钥、操作用户账户、改变网络安全设置或重试原受拒 Unix IPC/进程观察。

后续获得用户明确批准后，按原方式重试一次隔离 Application Service Unix socket 启动已成功；未修改安全设置或替换 IPC。随后正常隔离 Nola Server/Application Service 经 Tailcat 的真实连接检查通过：身份核对、未登录会话查询401、现有 Web 登录200、认证后 Core 会话查询200、缺少 CSRF 写请求403。该测试在临时应用目录使用临时测试身份，Host 和两个桥进程已退出，零模型请求；不等同四端 App UI、跨机器网络或持久设备重连验收。

Host 现可用 `--tailcat-helper /absolute/program --tailcat-helper-sha256 SHA256` 配合 `--tailcat-ingress` 显式启用临时授权代理；缺少精确程序/摘要或非私有监听配置明确拒绝。`POST /api/mobile/transport/tailcat` 沿用现有 HMAC/令牌认证和加密信封，connect 校验 expectedHostId 与临时公钥，close 校验原凭据 owner 与 leaseId。授权仅保存在内存，20分钟到期；私有管道退出即停止桥。仅物理退出后返回 closed，证据不足返回 stopUnconfirmed；不变更 Task/Run，不生成业务取消 ACK，不新增持久表、设备密钥或权限。

Mac/iOS 已共用 `Shared/Protocol/NolaTailcatTransport.swift` 的可选内嵌 SDK 适配器，Android 的 Ktor 普通/控制客户端共用相同 Go Session。客户端在已配对、已有 expected Host identity、无证书 pin 的私有根路径 HTTP Host 上自动交换授权并建立承载；逻辑 URL、模型身份、凭据索引和 HMAC 路径不变。未配置 Tailcat 的 Host 继续原连接；Host 宣告支持后连接失败明确失败，不静默换回旧承载或重发业务请求。HTTPS/pinned/custom-prefix 继续现有受信任传输，不放宽 TLS。SDK 是显式构建的可选本地资产，未链接时不声称支持。

App 进入后台只停用新连接派发、清理未完成启动，已建立临时身份保留在内存以供前台返回复用；不承诺后台继续执行。配置删除/替换、完整关闭会释放连接和尽力释放 Host lease。20分钟过期、App退出后的新启动仍需重新经认证 bootstrap；不声称已有永久跨网冷重连。原生共享适配器已在真实隔离 Nola Host 验证自动授权、认证读取、park/resume、完整关闭重连及逻辑 URL 保留；这不是 Mac/iOS 实窗或实际 OS 挂起验收。Android 编译通过，但 adb smartsocket5037 EPERM 阻止动态验证，未改端口或重试。桌面 Mac 的现有 Web 管理交接已映射到 App 持有的桥地址，真实登录与 Core 查询通过；移动端外部浏览器不映射本地桥，以免宿主 App 挂起后失效。浏览器保留同源登录/CSRF，仅显示入口模式不伪称隧道已建立；独立浏览器 WASM 不属于当前产品链路。详见[可选传输实现与私有IPC](../../../transport/tailcat/README.md)。

来源：[官方 README](https://github.com/tailscale/tailcat)、[v0.7.0 发布](https://github.com/tailscale/tailcat/releases/tag/v0.7.0)、[当前固定源码](https://github.com/tailscale/tailcat/tree/e00cdde6ff787f801361ff85ed085775c775d4c5)、[安全边界](https://github.com/tailscale/tailcat/blob/e00cdde6ff787f801361ff85ed085775c775d4c5/SECURITY.md)、[forward 实现](https://github.com/tailscale/tailcat/blob/v0.7.0/cmd/tailcat/forward.go)。未复制第三方实现。

### CLI 持有远程 Web 连接

`nola web --connect-host ... --expected-host-id ... --tailcat-helper ... --tailcat-helper-sha256 ... --token-stdin` 在本机 Core/Agent 启动前进入传输适配。既有 Nola Host token 仅由私有 stdin 输入、内存持有；共享加密移动接口交换临时 grant，已校验的 Go helper 提供 loopback HTTP/SSE。浏览器仍需正常 Web 登录及 CSRF，传输不替代业务认证。无 connect-host 的原 web 行为不变。连接当前最多20分钟，退出只释放传输，不取消 Host Task；不新增持久身份或后台续租。CLI 已编译。原沙箱内尝试及获准的一次重试均在 loopback bind 遇 EPERM，保留失败。随后正常的单命令沙箱外执行申请获批，真实 CLI→Tailcat→Host HTTP 闭环12.682秒通过：身份校验、未登录401、正常登录200、Core查询200、缺CSRF403、CLI退出0及测试Host退出；无客户端Core数据库、零模型调用、无持久凭据。浏览器原图、实际SSE UI与跨机器网络仍未验。
