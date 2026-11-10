# 开发与构建

| 用途 | 文档 |
| --- | --- |
| 源码目录与构建边界 | [源码目录约定](SOURCE_LAYOUT.md) |
| 基本环境与构建 | [仓库 README](../../README.md) |
| 各平台入口 | [平台目录](../platforms/README.md) |
| Rust 缓存与制品隔离 | [构建缓存](../architecture/工程规范/RUST_BUILD_CACHE.md) |
| Rust / Swift / Kotlin 规范 | [编码规范](../architecture/工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md) |
| 模块与依赖 | [架构入口](../architecture/README.md) |
| 测试方法与已测范围 | [质量入口](../quality/README.md) |

普通编译与测试使用隔离夹具。Mac XCTest 使用 Test 配置；正常 Debug 签名制品不能被未签名测试输出覆盖。真实模型验收会产生费用，不能把生产数据库或真实设备随意当夹具。

`python3 scripts/check_docs.py` 检查公开文档链接、场景清单及本地记录边界。`scripts/releases/export_public.py` 即使面对误跟踪的任务记录也拒绝导出；`.gitignore` 的例外不得重新放行本地任务/历史。
