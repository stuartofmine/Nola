# Android

实现位于 [android](../../../apps/android)，使用 Kotlin/Compose、JNI 和 Android 能力。设备能力与连接 Host 的执行目录分别管理。

- [客户端配置与远端权限呈现](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [宿主机流程](../../design/shared/HOST_CONNECTION_WORKFLOW.md)、[权限合同](../../architecture/模块/07-权限与平台/README.md)。
- JVM 测试：`cd apps/android && ./gradlew test`；设备测试：`./gradlew connectedDebugAndroidTest`。需要 JDK 21、SDK/NDK、Rust Android targets 与 cargo-ndk；只使用明确选择的测试模拟器/设备及隔离数据。

远端长任务验收脚本为 `scripts/test_real_android_remote_long_task.py`，要求显式指定专用模拟器和可丢弃的次要用户。两分钟物理作业不等于 120 次模型工具调用，adb reverse 不证明真实 Wi-Fi。范围与费用边界见[验收范围](../../quality/VALIDATION_SCOPE.md)。

专家由主 Agent 统一调度。用户输入和附件始终进入当前主会话的普通 Run；`@` 仅附带 ID、版本及已有成员/上下文引用，作为 `main_expert_selection` 类型上下文，不改写用户原文，也不直接发出第二条用户消息。Core 在同一次准入事务中检查选择并按需建立专家上下文。

只有主 Agent 提供 `delegate_expert`。专家作为现有 Task worker 的叶子任务运行，使用自己的模型、推理档位、Skill 与权限；专家不再派发专家或临时 subagent。结果与缺失信息交回主 Agent，再由主 Agent 与用户沟通。专家历史挂在主会话下，只读；历史作者的旧格式仍可读取，新主 Agent 来源明确绑定父会话。远端主 Run 继续使用现有认证加密、SourceSubmission 与 AssistantJournal 路径，不能用本机 Runtime 代替 Host 授权。

专家 chip 表示可取消的会话选择，普通准入消费该次正文与附件草稿后仍保留选择；取消 chip 不移除已加入成员或删除其上下文。Main 自行决定是否需要调用所选专家。

专家叶子不提供 `request_user_input`，缺失信息由主 Agent 统一向用户确认。旧冻结专家配置若含主控控制则明确拒绝恢复执行，保留历史和可用取消操作，不改写旧捕获。
