# Android

实现位于 [android](../../../apps/android)，使用 Kotlin/Compose、JNI 和 Android 能力。设备能力与连接 Host 的执行目录分别管理。

在本机对话里可以说“找到 GitHub 上 Nature Skills 里做论文精读卡的 Skill，装到这部手机”。也可以直接贴仓库或 Skill 目录链接。安装并启用后，在新的本机会话中即可调用。想给电脑使用时，选择已配对的 Host 并在该对话中提出安装请求。两处的 Skill 设置分别保存。[安装位置与资源范围](../../architecture/模块/11-Skill插件与MCP/README.md)

- [客户端配置与远端权限呈现](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [宿主机流程](../../design/shared/HOST_CONNECTION_WORKFLOW.md)、[权限合同](../../architecture/模块/07-权限与平台/README.md)。
- JVM 测试：`cd apps/android && ./gradlew test`；设备测试：`./gradlew connectedDebugAndroidTest`。需要 JDK 21、SDK/NDK、Rust Android targets 与 cargo-ndk；只使用明确选择的测试模拟器/设备及隔离数据。

远端长任务验收脚本为 `scripts/test_real_android_remote_long_task.py`，要求显式指定专用模拟器和可丢弃的次要用户。两分钟物理作业不等于 120 次模型工具调用，adb reverse 不证明真实 Wi-Fi。范围与费用边界见[验收范围](../../quality/VALIDATION_SCOPE.md)。

专家由主 Agent 统一调度。用户输入和附件始终进入当前主会话的普通 Run；`@` 仅附带 ID、版本及已有成员/上下文引用，作为 `main_expert_selection` 类型上下文，不改写用户原文，也不直接发出第二条用户消息。Core 在同一次准入事务中检查选择并按需建立专家上下文。

只有主 Agent 提供 `delegate_expert`。专家作为现有 Task worker 的叶子任务运行，使用自己的模型、推理档位、Skill 与权限；专家不再派发专家或临时 subagent。结果与缺失信息交回主 Agent，再由主 Agent 与用户沟通。专家历史挂在主会话下，只读；历史作者的旧格式仍可读取，新主 Agent 来源明确绑定父会话。远端主 Run 继续使用现有认证加密、SourceSubmission 与 AssistantJournal 路径，不能用本机 Runtime 代替 Host 授权。

输入时在光标处选择专家，已发消息在原句保留名字。下一条草稿可重新点名；当前会话中已加入的专家和子会话继续保留。交互与恢复规则见[消息内的专家提及](../../design/shared/EXPERT_MENTIONS.md)。

专家叶子不提供 `request_user_input`，缺失信息由主 Agent 统一向用户确认。旧冻结专家配置若含主控控制则明确拒绝恢复执行，保留历史和可用取消操作，不改写旧捕获。

### 本机会话中的 Goal

从会话菜单打开「进度」，输入目标和可选约束。启动时，Android 会读取当前模型、工具目录和权限，交给 Core 保存为该目标的执行配置。暂停后可以恢复原目标。取消目标后，设备会确认原运行已经停止。随后可以在同一会话内开始下一个目标，已完成的回复会保留。

### Privacy export on Android

Settings → Privacy → Export readable archive creates a ZIP with a `manifest.json` identifying `nola.android.personal-export` version 2. Each conversation has a Markdown document containing the user and assistant text, including committed messages still in the SQLite write-ahead log. The ZIP also contains `profile/memory.json`, `settings/model_parameters.json`, and `settings/preferences.json`. Markdown files can be opened independently or added to the document library.

Version 1 included raw SQLite files. Version 2 exports user-readable data through Core, and omits connection configuration, account credentials, tool arguments and tool outputs. The user's own text and custom prompts are preserved verbatim and can contain private information. Android has no whole-device ZIP restore interface. Previously saved ZIP files keep their original contents.

**Clear All Data** asks Android to erase Nola's app storage on this device and close the app. The next launch starts at onboarding.
