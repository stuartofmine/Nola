# 平台代码

| 目录 | 职责 | 开发说明 |
| --- | --- | --- |
| `ios/` | SwiftUI iOS / iPadOS 客户端 | [iOS](../docs/platforms/ios/README.md) |
| `macos/` | SwiftUI / AppKit macOS 客户端与平台 Host | [macOS](../docs/platforms/macos/README.md) |
| `android/` | Kotlin / Compose 客户端与 JNI 集成 | [Android](../docs/platforms/android/README.md) |
| `windows/` | Windows 桌面壳与本地 Host 集成 | [Windows](../docs/platforms/windows/README.md) |
| `web/` | Svelte 页面、静态资源与浏览器运行时桥接 | [Web](../docs/platforms/web/README.md) |
| `cli/` | Rust 终端客户端、Agent Host 与 HTTP 服务入口 | [CLI](../docs/platforms/cli/README.md) |

共享领域、执行和持久化逻辑在 [core/](../core/)，Apple 共用代码在 [shared/](../shared/)。跨平台边界、构建入口和测试位置见[源码目录约定](../docs/engineering/SOURCE_LAYOUT.md)。
