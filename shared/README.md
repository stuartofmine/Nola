# 共享平台代码

| 目录 | 职责 |
| --- | --- |
| `apple/` | iOS / macOS 共用的呈现、协议、权限与平台辅助代码 |
| `apple-core/` | Apple FFI 框架工程、头文件与桥接入口 |
| `mlx-runtime/` | Apple 本地 MLX 推理运行时 |
| `models/` | 共用 Swift 任务数据类型（`UserTask` 等） |

Rust 业务核心仍在 [core/](../core/)，TypeScript / Bazel 共享模块在 [modules/](../modules/)。共享目录不承载客户端私有页面。Apple 工程统一从根目录的 `Nola.xcodeproj` 打开，测试位于 [tests/apple/](../tests/apple/)。
