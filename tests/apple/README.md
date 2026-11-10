# Apple 测试

| 目录 | Xcode 测试目标 |
| --- | --- |
| `ios/` | `NolaTests` |
| `ios-ui/` | `NolaUITests` |
| `macos/` | `NolaMacTests` |
| `mlx/` | MLX 运行时测试源码；根工程尚未配置独立测试 target |

运行入口保持根目录 `scripts/run_client_tests.sh` 和 `scripts/test_mac_layers.sh`。XCTest 使用 Test 配置与隔离夹具，不能覆盖开发者正在使用的签名 Debug 应用。测试选择器沿用原目标名称，例如 `NolaTests/RustInteropTests`，不随磁盘目录改名。

Android 测试位于 `apps/android/app/src/test/` 与 `src/androidTest/`；Rust 和 Web 测试保留在所属 crate / 前端工程中。
