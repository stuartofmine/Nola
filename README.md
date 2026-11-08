> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola

Source: [stuartofmine/Nola](https://github.com/stuartofmine/Nola) · [Releases](https://github.com/stuartofmine/Nola/releases)

This is an active development preview. Download only the platform assets explicitly listed in a release; the first binary preview is the macOS Apple Silicon CLI bundle. Other native clients can be built from source. Update manifests are signed; installation never migrates or resets user data. See [release instructions](release/README.md).

Nola is a **local-first** personal AI assistant system.

> Chinese version: [`README.zh-CN.md`](README.zh-CN.md)

## Product Direction

Nola is built around three product principles:

- **Data-source driven:** conversations can use documents, websites, system data, health summaries, and local workspaces as explicit context.
- **Self-optimizing:** context planning, model routing, retrieval, and execution policy adapt without exposing legacy step-count controls to users.
- **Durable personal intelligence:** stable preferences and long-term memory are summarized, deduplicated, inspectable, and managed locally first.

The chat experience is the primary surface. Agent tools, data sources, memory, and automation support the conversation instead of becoming separate product silos.

## Architecture

| Component | Stack | Description |
|---|---|---|
| iOS (`Nola/`) | SwiftUI | Mobile chat, HealthKit/system data, notifications, memory, and local model support |
| macOS (`NolaMac/`) | SwiftUI/AppKit | Native desktop chat, local/remote host selection, workspaces, tools, and downloaded models |
| Android (`android/`) | Kotlin/Compose + Hilt | Mobile chat, Health Connect, platform permissions, and shared Agent runtime integration |
| Shared core (`core/`) | Rust + C ABI/JNI | Deterministic Agent state, permissions, tool contracts, and context planning |
| Web (`web/`) + CLI (`cli/`) | Rust CLI backend + Jinja templates/static assets | Independent local console, automation host, and developer CLI |
| Backend (`backend/`) | Cloudflare Worker + D1 | OAuth relay, model proxy, notifications |

The macOS app is not a CLI wrapper. Normal native chat does not launch a CLI process; CLI/Web remain independent surfaces for terminal workflows and host automation.

Local and API models are parallel model sources on Apple platforms. Selecting or downloading a
local model never starts it implicitly; users explicitly start and stop the runtime. Qwen3.5 0.8B
and 4B text chat use MLX's LLM factory with a configurable context budget of at least 64K tokens.
Android keeps custom OpenAI-compatible APIs separate from native on-device models. Its pinned
`llama.cpp` runtime downloads GGUF artifacts into app-private storage, verifies the publisher LFS
SHA-256 before atomic installation, and requires an explicit Start action before Chat or Agent can
select the model. Qwen3.5 4B Q4_K_M is the recommended profile; Qwen3 0.6B Q4_0 is the lightweight
profile. Both expose a configurable context budget up to the catalogued 64K window.
The Rust CLI also serves the authenticated Web UI on macOS, Linux, and Windows; native Windows
release validation requires a Windows SDK/toolchain rather than a macOS-only cross-check.

## Repository Layout

```text
.
├── backend/                # Backend services
├── android/                # Android Compose client
├── cli/                    # Rust CLI and Rust Web backend
├── core/      # Shared Rust Agent core and native build scripts
├── Nola/                   # iOS app
├── NolaMac/                # Native macOS app
├── NolaTests/              # iOS unit tests
├── NolaUITests/            # iOS UI tests
├── NolaMacTests/           # macOS unit, contract, and visual tests
├── web/                    # Web templates, static assets, and native host helpers
├── Doc/                    # Docs
├── dev.sh                  # Dev startup (hot reload by default)
└── deploy.sh               # Release deploy script
```

## Local Development

In the interactive CLI, copy an image and press `Ctrl+V` (including macOS terminals), or paste an image's absolute path, to attach it. Delete its placeholder before sending to remove it. Image analysis requires a model with declared image input support. After a permission denial, use `/allow` to review conversation permissions; explicit `deny` rules must be changed in their configuration, and OS permissions must be granted in the OS. Then explicitly request a retry. The cache hit rate describes the latest model request; `—` means complete usage was not reported.

### Prerequisites

- Node.js + npm
- Rust toolchain (`rustup` + `cargo`)
- macOS with Xcode for iOS/macOS development
- JDK 21, Android SDK, Android NDK, and `cargo-ndk` for Android native builds
- macOS/Linux shell environment for backend, Web, and CLI development

### Start the dev stack

```bash
./dev.sh
```

Default behavior:
- Backend runs Worker `dev` mode with automatic reload
- Web is served by the Rust CLI backend

Common switches:

```bash
NOLA_LAN=0 ./dev.sh
BACKEND_PORT=8000 WEB_PORT=5001 ./dev.sh
```

### Native builds and tests

The iOS test runner selects an available simulator, bounds parallel workers, and removes only XCTest clones created by that invocation.

```bash
./scripts/run_client_tests.sh
# Optional: PARALLEL_TEST_WORKERS=8 ./scripts/run_client_tests.sh
xcodebuild test -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS'
```

Android uses Rust static archives for all supported ABIs. The build script discovers `cargo` and the NDK instead of relying on a machine-specific absolute path.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi \
  i686-linux-android x86_64-linux-android
cargo install cargo-ndk --locked
cd android
./gradlew test
./gradlew assembleDebugAndroidTest
./gradlew connectedDebugAndroidTest   # requires a running emulator or device
```

Shared core and backend checks:

```bash
cd core && cargo test --quiet
cd backend && npm run typecheck
cd backend && python -m pytest -q
```

## Current Platform Behavior

- **Attachments:** iOS and macOS extract supported document text locally. DOCX extraction includes body, tables, headers, footers, footnotes, and endnotes. Attachment contents are reference context and are never treated as user commands by themselves.
- **Health data:** iOS reads HealthKit directly and shares one dashboard snapshot between the Data tab and detail view. HealthKit observer changes refresh the visible dashboard. macOS only shows an iPhone health summary after a sync payload exists.
- **Agent runtime:** iOS and Android use the shared Rust command/event contract. Platform clients remain responsible for OS authorization, sandboxed files, UI, and lifecycle.
- **Generated native artifacts:** XCFrameworks, static archives, JNI libraries, logs, and local databases are build outputs and must not be committed.

## Release Deployment

`deploy.sh` validates environment settings and deploys with `wrangler deploy` by default.

```bash
export nola_backend_env='{"SERVER_JWT_SECRET":"..."}'  # full JSON required
export CLOUDFLARE_API_TOKEN=xxx
./deploy.sh
```

Optional flags:

```bash
DEPLOY_RUN_TESTS=1 ./deploy.sh
DEPLOY_RUN_TYPECHECK=0 ./deploy.sh
./deploy.sh --env production
```

## Documentation

- Documentation hub: [Architecture, UI, current tasks, and history](Doc/README.md)
- API docs (EN): [`Doc/软件架构设计/模块/01-领域与协议/API.md`](Doc/软件架构设计/模块/01-领域与协议/API.md)
- API docs (中文): [`Doc/软件架构设计/模块/01-领域与协议/API.zh-CN.md`](Doc/软件架构设计/模块/01-领域与协议/API.zh-CN.md)
- Requirements: [`Doc/软件架构设计/总体设计/REQUIREMENTS.md`](Doc/软件架构设计/总体设计/REQUIREMENTS.md)
- Software design: [`Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md`](Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md)
- Shared mobile Agent core: [`Doc/软件架构设计/模块/13-客户端投影/MOBILE_SHARED_CORE.md`](Doc/软件架构设计/模块/13-客户端投影/MOBILE_SHARED_CORE.md)
- Changelog: [`Doc/历史任务/实施与审计/CHANGELOG.md`](Doc/历史任务/实施与审计/CHANGELOG.md)

## Community

- Contributing guide: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Code of Conduct: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- Security policy: [`SECURITY.md`](SECURITY.md)

## License

Licensed under the **Apache License 2.0**.  
See [`LICENSE`](LICENSE) for full terms.

## Configuration

- Backend config is loaded from `nola_backend_env` (or `NOLA_BACKEND_ENV`) JSON only
- Web local config example: `web/.env.example`
- Never commit production secrets
