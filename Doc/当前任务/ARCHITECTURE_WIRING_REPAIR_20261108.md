# 未贯通链路修复记录

日期：2026-11-08。范围：记忆所有权、历史 provider 恢复清理，以及测试暴露的 Mac 计划 revision 接线。本文不关闭全平台 UI、真实模型或真机验收。

## 已修改的合同与调用链

- [Domain owner](../../core/nola-domain/src/memory_owner.rs) 替代独立 Core helper；语义记录必填 owner/namespace，禁止从 provenance 或缺失字段推断归属。
- [Application Recall](../../core/nola-application/src/memory.rs) 必填执行 binding；[Store](../../core/nola-store/src/memory_store.rs) 在排序/LIMIT 前按 owner 过滤，去重、更新、替换、后台合并和同步安装保持归属。跨 owner supersedes 与未来补到的跨归属目标明确拒绝。
- CLI/Mac Host 使用显式 ToolContext 和已准入请求身份，workspace ID 与冻结 Run 身份共用计算；Web 会话搜索绑定已核对的 conversation。自定义集合使用 `custom:<name>`，未知 scope 不再变成全局集合。
- 移动端 profile 搜索改用 Application Recall；原生记录保留 owner。iOS 删除“Core 选择为空后重新加入全部长期记忆”的客户端回退；记忆引擎不可用时明确失败。Android 同样拒绝把引擎不可用当作空搜索结果。
- 记忆选择 ABI 为 4，Application Service 协议为 54，Store version 为 83（epoch 8）。不匹配明确拒绝，不自动迁移、删除或重建用户库。
- [bootstrap](../../core/nola-application-bootstrap/src/lib.rs)、[Service 参数](../../core/nola-service/src/main.rs) 与 [Mac supervisor](../../NolaMac/Services/MacApplicationServiceClient.swift) 删除旧 provider 解码/转换、一次性恢复标记、恢复参数及普通启动归档扫描。用户显式归档仍保留完整原文件，新库不补回旧模型配置。
- [Mac 计划保存](../../NolaMac/Services/MacApplicationPort.swift) 传递调用者读到的 expected_revision；过期版本冲突保留原计划，不自动重读后覆盖。
- Rust 测试配置通过显式 fixture 注入运行配置，避免单测依赖开发者个人模型连接配置；权限恢复 fixture 补齐当前外部执行所需的已准入父快照，没有放宽生产权限。

## 验证范围

确定性测试覆盖跨会话/轮次/工作区、相同内容不同 owner、外归属记录超过查询页、并发 Service 读写、冷启动、owner 不可变、同步拒绝回滚、合并分组/摘要、缺 owner/binding 与 scope 拼错的拒绝。Apple 检查使用独立 Test 构建目录和隔离存储；没有用 unsigned Debug 替换开发中的签名应用。

本轮检查结果：

| 检查 | 结果 |
| --- | --- |
| Rust 工作区 | `cargo check --workspace --tests` 通过；主要 crate 库与二进制测试通过，原有跳过项不计为通过 |
| Core 记忆合同 | 38 项通过 |
| Application/Store 记忆、checkpoint、同步集成 | 45 项通过 |
| CLI memory filter | 63 项通过，包含并发 Service 和 scope 错误拒绝 |
| Mac Test | 35 项通过，包含当前 ABI 与计划 revision 冲突 |
| iOS Simulator Test | 8 项通过，包含真实请求准备中的敏感记忆排除及普通记忆保留 |
| Android JVM | 688 项报告，2 项跳过，其余通过；构建、测试 APK 编译和 lint 通过 |
| Android 原生制品 | arm64-v8a 与 x86_64 的 memory-selection 导出均核对为 ABI 4 |
| 当前 Rust 产品可执行文件 | nola、nola-agent-host、nola-service 编译通过；隔离数据库下 help、model connections 冒烟退出 0 |
| 文档/格式 | 当前本地链接与 git diff 空白检查通过；本地化文件基线未被构建改写 |

Apple 使用 `-configuration Test`、独立派生目录与 `SWIFT_EMIT_LOC_STRINGS=NO`；Android 运行 `testDebugUnitTest assembleDebug assembleDebugAndroidTest lintDebug`。未运行 connectedDebugAndroidTest，也未把模拟器或测试 APK 编译写成 Android 真机验收。

后续已完成 CLI 真实 provider 的交替私有标记、冷启动、实际压缩、权限等待与后台任务验证，见[真实模型长任务验收](CLI_REAL_LONG_TASK_ACCEPTANCE_20261108.md)。这不覆盖模型请求并发。

仍需独立验证：真实 provider 的并发私有标记测试、Android 设备执行、原生 workspace/turn 选择界面，以及其余架构差距。当前管理 List/Get/Replace 是管理合同，不可当作模型局部 Recall 的授权边界；Store-relative `local` 是当前单个人记忆 profile，不是模型 profile 或新增账号目录。

后续跨平台真实测试另修复三处客户端接线：Mac 不再发送 null reasoning_tier；Android 的 ABI 1 接受协议更新为 54；Android 将已封口 Run 的物理收尾状态投影到 composer，收尾完成前禁用发送并保留可编辑草稿。详见[跨平台真实长任务验收](CROSS_PLATFORM_REAL_LONG_TASK_ACCEPTANCE_20261108.md)，不据此关闭所有 UI/真机验收。

后续修复 iOS 紧凑导航的重复宿主：自定义底部导航不再叠加隐藏的系统 TabView，避免隐藏 UITabBar 的无限匹配动画阻塞模型表单键盘事件。新增按标签保留 hosting 根、仅挂载当前页的容器；模型表单实际切页、草稿保留、保存和真实回复通过，端点输入从约 121 秒恢复为约 1 秒。常规宽度继续系统 TabView；详见上述真实测试报告与客户端交互配置合同。

长任务随后复现 iOS 提交层先取消消费者、native Stop 后提交的竞态：消费者 detach 退役 permit，遗留 running Run。已把提交任务退役回调放到原生 Core interrupt 持久化之后；真实模型复测出现 cancelled 封口，后续消息正常回答。未把观察流 EOF 或 UI 错误当作取消成功。
