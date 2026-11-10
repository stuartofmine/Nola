# Android

实现位于 [android](../../../apps/android)，使用 Kotlin/Compose、JNI 和 Android 能力。设备能力与连接 Host 的执行目录分别管理。

- [客户端配置与远端权限呈现](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [宿主机流程](../../design/shared/HOST_CONNECTION_WORKFLOW.md)、[权限合同](../../architecture/模块/07-权限与平台/README.md)。
- JVM 测试：`cd apps/android && ./gradlew test`；设备测试：`./gradlew connectedDebugAndroidTest`。需要 JDK 21、SDK/NDK、Rust Android targets 与 cargo-ndk；只使用明确选择的测试模拟器/设备及隔离数据。

远端长任务验收脚本为 `scripts/test_real_android_remote_long_task.py`，要求显式指定专用模拟器和可丢弃的次要用户。两分钟物理作业不等于 120 次模型工具调用，adb reverse 不证明真实 Wi-Fi。范围与费用边界见[验收范围](../../quality/VALIDATION_SCOPE.md)。
