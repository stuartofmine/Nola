# 平台文档

平台入口负责接入、平台交互和构建约束，不复制共享业务状态机或授权合同。

| 平台 | 入口 | 实现 |
| --- | --- | --- |
| macOS | [macOS](macos/README.md) | SwiftUI/AppKit、本地 Application Service 与 Host 集成 |
| iOS / iPadOS | [iOS](ios/README.md) | SwiftUI、embedded Application 与设备能力 |
| Android | [Android](android/README.md) | Compose、JNI、设备能力与远端 Host 客户端 |
| Web | [Web](web/README.md) | Rust Host 提供的 Svelte 界面 |
| Windows | [Windows](windows/README.md) | 原生桌面壳、共享 Web 页面及拥有的本地 Host |
| CLI | [CLI](cli/README.md) | Rust 终端、Agent Host 与本地 HTTP 服务 |
| Apple 共用 | [Apple](apple/README.md) | 两端共享呈现与检查器约定 |

[共享设计](../design/README.md)、[客户端契约](../architecture/模块/13-客户端投影/README.md)、[质量与范围](../quality/README.md)分别维护。原型的平台标签只是设计视角，不代表原生平台全部通过。
