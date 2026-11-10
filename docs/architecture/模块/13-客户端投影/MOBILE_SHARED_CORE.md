> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 移动端共享 Application/Core 接入

状态：现行接入边界，2026-11-08 核对 Application 制品与 embedded 所有权；当前任务总账（本地工作记录，不随仓库分发）单独跟踪。

## 公开边界

iOS/Android 通过共享 Application 用例和投影访问业务，不自行创建可变 Core 指针或维护第二套工具循环。
Swift/Kotlin wrapper 和 C/JNI 只负责安全编解码、内存/句柄生命周期和错误映射。
已经删除的 nola_core_agent_v2_* ABI 不应按旧文档重建。
ABI/协议版本以代码定义为唯一数值来源，调用前校验，不兼容明确拒绝。

## 当前制品与调用

`nola_core` 是 rlib facade；`nola_mobile_bindings` 产出 staticlib/cdylib/rlib，包含 Application C ABI 与 embedded runtime，并通过 force-link 保留 Core 中已存在的纯辅助符号。构建脚本从 `libnola_mobile_bindings.a` 打包为平台归档，某些输出仍名为 `libnola_core.a`；这是制品命名，不是第二份 Core 或旧接口兼容。

Swift `IOSApplicationRuntime`、Kotlin `AndroidApplicationRuntime` 使用 Application 句柄和当前协议提交用例；JNI/C 负责边界编解码，不接管领域状态机。业务 Store 由 Service/embedded owner 独占，按 canonical 数据库身份共享 runtime。open 失败返回具体错误，不能另开空数据库掩盖问题。

共享 MCP 的移动 HTTP Host adapter 也在最终 bindings 中，但其具体执行仍须经过 Core/Application 准入与 Host/config/session 围栏。没有移动 stdio 进程能力就不能借桌面配置假装可用。

源码：[bindings](../../../../core/nola-mobile-bindings/src/lib.rs)、[原生制品](../../../../core/nola-mobile-bindings/Cargo.toml)、[Apple 打包](../../../../core/scripts/build_xcframework.sh)、[Android 打包](../../../../core/scripts/build_android.sh)。

## 对象分工

- WorkspacePresentation：选中会话、窗口状态和投影订阅。
- ConversationRunController：固定 owner 的执行观察与控制，生命周期独立于选中页面。
- Application 用例：创建/配置/发送/等待决定/取消/恢复；持久回执决定结果。
- DraftRepository：设备草稿和安全附件，异步读取、CAS、墓碑，失败保留编辑。
- Platform Host：资源解析、OS 权限、实际工具与模型传输，不决定应用政策或任务完成。

原生可嵌入 Application，也可连接认证后的远端 authority。UI 不直接打开另一份 canonical writer。
身份至少包含 store/authority scope 与 conversation；Run/Attempt 再绑定具体执行，远端路径不能当作本机授权。

## 发送、切换与恢复

发送读取原会话配置，冻结调用，提交成功才退休对应草稿。
本地文件草稿与 canonical Store 无跨库原子性；采用持久发送身份、回执查询与幂等退休记录恢复。
切换页面只更新选择/订阅，不借用另一会话的 isGenerating、权限等待或恢复状态。
前台页面离开不等于用户取消；后台是否继续取决于 OS 能力，进程回收后从持久边界恢复。

## 权限、投影与计量

应用授权与 OS 资源授权分离；已有有效范围直接使用，必要时才打开系统 picker。
移动端沙盒不能因为 allow_all 执行本机不支持的 shell/浏览器控制。
正式 Part 由已提交投影更新，临时 overlay 不写 canonical 正文；终态按原 Run 回执核对。
usage 从共享 Attempt 账本或已认证远端累计快照读取，切换不清零，不重复记入父/子费用。

编译产物与桥接方式沿用现有构建所有者；Rust 框架、JNI 和 native Host 产物不提交仓库。
本契约不声称每端已实现全部能力；验收同时覆盖本地/API/远端后端、快速切换、进程回收及草稿冲突。
旧指针 ABI 与旧事件列表已归档为历史接入快照（本地工作记录，不随仓库分发）。

## Apple 客户端共享源码（2026-10-20）

`shared/apple/Chat/` 由 Xcode 同时编入 iOS 与 macOS target，不复制平台版本，也不保留旧名称别名。

- `ConversationTurnInput` 固定本轮提交原文，明确历史由 Core 管理或由调用方提供已提交消息。不得把历史末尾 user 当成本轮输入，也不得按文本相等去重。iOS 在实际 Core 输入边界使用 prepared 历史；Mac 在实际 run 命令边界使用 coreManaged，历史只由 Core 读取一次。
- `AuthoritativeTextReconciliation` 共用完整文本快照的 unchanged、append、replace 判断；缩短、分歧及清空均接受权威替换。iOS Markdown buffer 与 Mac presentation session 使用同一规则。

平台保留原生排版、滚动、键盘与动画调度。Mac 的 reveal lease 与 iOS 的帧合并生命周期不同，不应通过复制状态机实现表面一致。模型配置按运行/容器的明确 owner 解析；iOS 无参数刷新复用显式选择，不能被异步全局默认值覆盖。Mac 继续由冻结的 run 配置提供 owner。

## 当前桥接与本地生命周期（2026-10-28核对）

移动Application C ABI版本为1；Service协议号从Rust导出读取，不由Swift/Kotlin分别猜测。open/free/execute与持久submit有不同职责，同canonical数据库共享embedded runtime及准入锁。Shared包含AppleSubmissionFrontier验证；Mac与iOS复用原源身份检查。页面/流/协程退出只detach，显式业务Stop另走持久取消。
