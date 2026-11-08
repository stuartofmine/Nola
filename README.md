# This project is still in early-stage

# Nola


A local-first personal AI assistant for macOS, iOS, Android, Windows, the Web and the terminal.


[中文](README.zh-CN.md) · [Source](https://github.com/stuartofmine/Nola) · [Downloads](https://github.com/stuartofmine/Nola/releases) · [Documentation](Doc/README.md)

## What it does

Nola combines conversations, model connections, local workspaces, tool execution, memory and long-running tasks. Clients use a shared Rust runtime; each platform owns its interface, permissions and lifecycle. Model inference uses the connections configured by the user. Supported local model runtimes are managed separately by each client.

The CLI can expose its local Host through an authenticated HTTP service for the Web client and remote clients.

## Downloads

The 1.0.0 development preview provides a complete **macOS Apple Silicon CLI** bundle. Keep its CLI, Agent Host, service, Web assets and browser runtime together. It is not Apple Developer ID notarized. Other platform installers have not been published; their source is included.

Download only assets listed on the [release page](https://github.com/stuartofmine/Nola/releases). In the extracted folder:

```sh
./nola --version
./nola --help
./nola web --host 127.0.0.1 --port 5001
```

Configure a model connection and select a declared model before starting a conversation. Credentials belong in local configuration or the platform credential store, never in source control.

## Architecture

| Component | Implementation | Responsibility |
| --- | --- | --- |
| macOS · `NolaMac/` | SwiftUI / AppKit | Native desktop client and platform Host integration |
| iOS · `Nola/` | SwiftUI | Mobile client and iOS capabilities |
| Android · `android/` | Kotlin / Compose | Mobile client, JNI integration and Android capabilities |
| Windows · `windows/` | Native desktop shell + Web UI | Desktop client and owned local Host integration |
| Web · `web/frontend/` | Svelte / TypeScript | Browser interface served by the Rust Host |
| CLI · `cli/` | Rust | Terminal client, Agent Host and local HTTP service |
| Core · `core/` | Rust, C ABI / JNI | Domain contracts, Agent execution, Application Service, persistence and Host interfaces |
| Shared Apple code · `Shared/` | Swift | Shared presentation and protocol code |

Native clients have their own UI; the macOS client is not a CLI wrapper. Conversations and durable runtime state are managed by the local Application Service and Store. Remote Host access requires authentication and explicit authorization.

## Build and run from source

Use stable Rust and Node.js 24 or newer. Apple clients require Xcode. Android builds additionally require JDK 21, Android SDK/NDK, Rust Android targets and `cargo-ndk`. Windows builds require the native Windows toolchain.

From the repository root:

```sh
npm --prefix web/frontend ci
npm --prefix web/frontend run build
cargo build --locked -p nola_cli -p nola_service --bins
./target/debug/nola --help
./target/debug/nola web --host 127.0.0.1 --port 5001 --web-root "$PWD/web"
```

The same local build/start sequence is available as `./dev.sh`; `WEB_HOST` and `WEB_PORT` configure the listener.

For model configuration, use the Web/native settings or `nola model apply-config /absolute/path/to/config.json`. `llm.default_model` must name a connection and model declared in the same configuration.

In the interactive CLI, `Ctrl+V` attaches a clipboard image; an absolute image path can also be pasted. Image analysis requires declared image-input support. After a permission denial, `/allow` opens conversation permissions. Explicit configuration denials and operating-system permissions must be changed at their source; request a retry afterward.

## Tests

Run checks for the area being changed:

```sh
cargo test --locked -p nola_domain -p nola_protocol -p nola_store -p nola_update_host
npm --prefix web/frontend run check
npm --prefix web/frontend test
./scripts/test_mac_layers.sh contracts
./scripts/run_client_tests.sh
(cd android && ./gradlew test)
```

Apple test runners use isolated Test outputs. macOS Debug builds must retain their configured signing identity. Android instrumentation tests require a running emulator or explicitly selected device. Contract tests do not constitute real-model or cross-platform installation acceptance; consult the [validation record](Doc/当前任务/V1_REISSUE_ACCEPTANCE_20261109.md) for results and known gaps.

## Versions and compatibility

The reissued **1.0.0 is the first supported baseline**. All Nola-owned protocol, ABI and database format/epoch versions start at **1**. Earlier development formats are unsupported. Application version/build numbers are separate from contract versions; external standards retain their own version identifiers.

Subsequent releases must preserve published interfaces and stored data, with explicit negotiation for protocol changes and tested, recoverable migrations for data changes. Unsupported formats are rejected without automatically deleting or rebuilding user data. See the [compatibility policy](Doc/软件架构设计/总体设计/VERSION_COMPATIBILITY.md).

## Updates and releases

Source and signed update metadata are hosted on GitHub; complete packages are distributed through GitHub Releases. Clients verify the Ed25519 signature, platform, data format, package length and SHA-256. Update checks and installation handoff are explicit user actions. Signed metadata is renewed weekly by GitHub Actions.

See [release instructions](release/README.md). Do not commit credentials, signing material, databases, logs or generated build artifacts.

## Contributing

[Contributing](CONTRIBUTING.md) · [Code of conduct](CODE_OF_CONDUCT.md) · [Security](SECURITY.md)

Licensed under [Apache-2.0](LICENSE).
