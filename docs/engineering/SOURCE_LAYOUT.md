# 源码目录约定

仓库按平台与用途组织。目录调整不改变应用标识、可执行文件名、安装包资源布局或用户数据路径。

```text
apps/                 六个平台入口
  ios/                SwiftUI iOS / iPadOS
  macos/              SwiftUI / AppKit macOS
  android/            Kotlin / Compose
  windows/            Windows 桌面壳
  web/                Svelte、Web 资源与浏览器运行时桥接
  cli/                Rust CLI、Agent Host、HTTP 服务
core/                 Rust 领域、Application Service、Store、FFI
shared/               跨平台设计 Token（design-tokens）、Apple 共用代码、FFI 框架、MLX
tests/apple/          Apple 单元与 UI 测试
transport/            跨模块传输定义
assets/               跨平台设计资源
scripts/              构建、测试、生成与发布工具
tools/                开发辅助工具
evals/                评测工具与夹具
third_party/          第三方源码、适配与许可
release/              发布版本、签名公钥与制品约定
docs/                 按用途、平台分类的公开文档
Doc/                  仅本地当前任务与历史记录，不提交或导出
```

`legacy/` 中的历史实现仅保留本地，整目录忽略且禁止公开导出，不属于 GitHub 源码树。

根目录保留 `Cargo.toml` / `Cargo.lock` 与 `ci_scripts/`；Apple 工程入口位于 `apple/Nola.xcodeproj`，通过相对路径将源码根定位到仓库根目录。Xcode target、scheme、bundle ID 和测试选择器沿用原名称。构建、测试和发布脚本都显式使用这个工程入口，不依赖根目录自动发现工程。

Rust 工作区从根目录运行；Android 在 `apps/android/` 执行 Gradle；Web 与 Windows 分别使用 `npm --prefix apps/web/frontend` 和 `npm --prefix apps/windows`。平台依赖和完整命令见[仓库 README](../../README.md)与[平台入口](../platforms/README.md)。

CLI 安装包继续使用 `bin/`、`web/` 资源布局；用户项目的默认 Web 根仍为 `<project>/web`。从源码启动时显式传 `--web-root "$PWD/apps/web"`，避免将源码组织泄漏到用户项目路径。应用支持目录、凭据、数据库、协议与发布版本均不因源码搬迁而改变。

新增文件放到所属平台或共用模块，平台内测试贴近实现；Apple 测试集中在 `tests/apple/`。生成制品、缓存、数据库和任务记录不能随源码迁移加入版本控制。

目录检查：

```sh
python3 scripts/check_source_layout.py
python3 scripts/check_design_tokens.py
python3 scripts/check_portable_paths.py
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts/releases -p 'test_*.py'
```

第一项检查源码入口、Cargo 依赖、Svelte 相对导入与 Apple 工程文件引用；发布脚本回归另行验证安装包仍使用 `web/` 布局。这些静态检查不能替代各平台编译和运行测试。Mac XCTest 必须使用 Test 输出，保留正常 Debug 签名身份。

仓库内引用以仓库根目录或当前脚本位置为基准，不能写开发机的用户名、桌面目录或 macOS 随机临时目录。浏览器验收脚本的默认证据写到被忽略的 `.artifacts/`；随机临时目录由系统 API 创建。Chrome 通过 Playwright 的平台渠道发现，也可显式设置 `CHROME_PATH`；相对配置按仓库根目录解析。工具调用需要绝对路径时在运行时解析，不把解析结果写回源码。系统程序搜索目录、容器约定和验证权限/脱敏行为的匿名绝对路径夹具保持各自语义。
