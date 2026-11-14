# 共享平台代码

| 目录 | 职责 |
| --- | --- |
| `apple/` | iOS / macOS 共用的呈现、协议、权限与平台辅助代码 |
| `apple-core/` | Apple FFI 框架工程、头文件与桥接入口 |
| `design-tokens/` | 跨平台机器可读设计 Token 真源（`tokens.json`） |
| `mlx-runtime/` | Apple 本地 MLX 推理运行时 |

Rust 业务核心仍在 [core/](../core/)。共享目录不承载客户端私有页面。Apple 工程统一从 [apple/Nola.xcodeproj](../apple/Nola.xcodeproj) 打开，测试位于 [tests/apple/](../tests/apple/)。
