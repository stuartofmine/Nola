# Nola

A local-first personal AI assistant for macOS, iOS, Android, Windows, the Web and the terminal.

[中文](README.zh-CN.md) · [Source](https://github.com/stuartofmine/Nola) · [Downloads](https://github.com/stuartofmine/Nola/releases) · [Documentation](docs/README.md)

## What it does

Nola combines conversations, model connections, local workspaces, tool execution, memory and long-running tasks. Clients use a shared Rust runtime; each platform owns its interface, permissions and lifecycle. Model inference uses the connections configured by the user. Supported local model runtimes are managed separately by each client.

The CLI can expose its local Host through an authenticated HTTP service for the Web client and remote clients.

Native and Web expert settings provide seven starting roles: code writing, code review, testing, personal assistance, visual art, presentations and academic writing. Each includes instructions, a suggested tool pack and associated Skills. Experts have their own model-declared reasoning effort; use `@` in graphical conversation composers to select an expert. Existing conversations retain their joined configuration until an explicit update. A single main agent interacts with the user and delegates to experts through the shared subagent Task runtime. Experts keep their own model, tools and persistent child history and do not recursively delegate. See the [expert interaction design](docs/design/shared/MULTI_AGENT_NAVIGATION.md).

Experts can also run through an installed Antigravity, Codex, Claude Code or OpenCode on the connected desktop Host. Bind the implementation and review roles to their preferred executors, then mention both in a conversation. The main agent can send the implementation commit to a second agent for review and verification. Model choices come from the Host, including the native Antigravity reasoning variants. [Expert configuration and mentions](docs/design/shared/EXPERT_MENTIONS.md)

## Downloads

Choose an installer or archive from the [1.0.0 release](https://github.com/stuartofmine/Nola/releases/tag/v1.0.0-build.3). The release page lists the available platforms, signing status and checksums.

For the Linux CLI and Web client, extract the complete archive and run the following commands from its directory.

```sh
./nola --version
./nola --help
./nola web --host 127.0.0.1 --port 5001
```

Configure a model connection and select a declared model before starting a conversation. Credentials belong in local configuration or the platform credential store, never in source control.

## Architecture

[Architecture guide](docs/architecture/README.en.md) · [中文架构指南](docs/architecture/README.md)

| Component | Implementation | Responsibility |
| --- | --- | --- |
| macOS · `apps/macos/` | SwiftUI / AppKit | Native desktop client and platform Host integration |
| iOS · `apps/ios/` | SwiftUI | Mobile client and iOS capabilities |
| Android · `apps/android/` | Kotlin / Compose | Mobile client, JNI integration and Android capabilities |
| Windows · `apps/windows/` | Tauri 2 / WebView2 / Svelte | Desktop client and owned local Host integration |
| Web · `apps/web/frontend/` | Svelte / TypeScript | Browser interface served by the Rust Host |
| Unified Rust Workspace · `crates/` | 8 Rust crates (`nola-types`, `nola-store`, `nola-llm`, `nola-tools`, `nola-mcp`, `nola-engine`, `nola-modules`, `nola-app`) | Single `nola` binary (CLI, TUI, local HTTP/Web Host, background daemon) and C ABI / JNI FFI core library |
| Shared Apple code · `shared/apple/` | Swift | Shared presentation and protocol code |

Native clients have their own UI; the macOS client is not a CLI wrapper. The unified `nola` Host binary and embedded mobile FFI library compile from the same `crates/` workspace and share the same SQLite WAL store, agent engine, and tool/module contracts. Remote Host access requires authentication and explicit authorization.

See [source layout](docs/engineering/SOURCE_LAYOUT.md) for platform roots, shared modules and test locations.

## Build and run from source

Use stable Rust and Node.js 24 or newer. Apple clients require Xcode. Android builds additionally require JDK 21, Android SDK/NDK, Rust Android targets and `cargo-ndk`. Windows builds require the native Windows toolchain.

From the repository root:

```sh
npm --prefix apps/web/frontend ci
npm --prefix apps/web/frontend run build
cargo build --locked -p nola-app --bin nola
./target/debug/nola --help
./target/debug/nola web --host 127.0.0.1 --port 5001 --web-root "$PWD/apps/web"
```

The same local build/start sequence is available as `./dev.sh`; `WEB_HOST` and `WEB_PORT` configure the listener.

For model configuration, use the Web/native settings or run `nola config` to inspect file sources and the saved Host default. Save your changes and start Nola. Startup validates the catalog and applies its connections, models and default together. If validation fails, Nola reports the file and keeps the saved catalog. `llm.default_model` must name a connection and model declared in the same configuration.

In the interactive CLI, `Ctrl+V` attaches a clipboard image; an absolute image path can also be pasted. Image analysis requires declared image-input support. After a permission denial, `/allow` opens conversation permissions. Explicit configuration denials and operating-system permissions must be changed at their source; request a retry afterward.

## Tests

Run checks for the area being changed:

```sh
cargo test --locked --workspace --lib
npm --prefix apps/web/frontend run check
npm --prefix apps/web/frontend test
./scripts/test_mac_layers.sh contracts
./scripts/run_client_tests.sh
(cd apps/android && ./gradlew test)
```

Apple test runners use isolated Test outputs. macOS Debug builds must retain their configured signing identity. Android instrumentation tests require a running emulator or explicitly selected device. Contract tests do not constitute real-model or cross-platform installation acceptance; consult the validation record（本地工作记录，不随仓库分发） for results and known gaps.

## Design and validation

[Interactive prototype](docs/design/prototype/README.md) · [68 business scenarios and 220 design states](docs/design/prototype/docs/BUSINESS-SCENARIOS.md) · [Remote Host and prototype validation](docs/quality/VALIDATION_SCOPE.md)

Prototype replays, native UI checks, and real-model execution have separate scopes. A platform shown in the prototype does not assert that every native workflow has been implemented or tested.

## Versions and compatibility

**Nola 2.0.0** establishes the lean 8-crate Rust architecture, unified `nola` binary, and SQLite WAL schema (`user_version = 2`) as the new supported baseline. Legacy v1 databases are explicitly rejected without silently deleting or rewriting user data. Application version/build numbers are separate from contract versions; external standards retain their own version identifiers.

Subsequent releases must preserve published interfaces and stored data, with explicit negotiation for protocol changes and tested, recoverable migrations for data changes. Unsupported formats are rejected without automatically deleting or rebuilding user data. See the [compatibility policy](docs/architecture/总体设计/VERSION_COMPATIBILITY.en.md).

## Updates and releases

Source and signed update metadata are hosted on GitHub; complete packages are distributed through GitHub Releases. Clients verify the Ed25519 signature, platform, data format, package length and SHA-256. Update checks and installation handoff are explicit user actions. Signed metadata is renewed weekly by GitHub Actions.

See [release instructions](release/README.md). Do not commit credentials, signing material, databases, logs or generated build artifacts.

## Contributing

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md)

Licensed under [Apache-2.0](LICENSE).
