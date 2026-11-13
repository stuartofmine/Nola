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

## 专家会话

在当前会话的输入框键入 `@` 或 `＠` 可查找已启用的专家；中文紧接着输入也可触发，例如 `请@研究`。选择候选后才会添加专家，普通文本和邮箱地址不会自动分派。

主会话保留专家署名和回答，待审批操作仍可在主会话处理。完整的专家会话与工具执行详情在桌面端的右侧检查器，或移动端当前会话的「更多 → 专家会话」中查看，不会改变当前主会话。

## 执行过程

每条助手回复最多显示一个执行过程入口。思考摘要和工具步骤按原有顺序排列；正文、交互卡片和待审批操作仍在入口之外。CLI 保留按时间顺序显示的终端输出和独立审批交互。
