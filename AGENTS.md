> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Repository Guidelines

## Project Structure & Module Organization

Nola is a local-first personal AI assistant with several clients and shared services. `Nola/` contains the SwiftUI iOS app, with UI in `Nola/UI/`, services in `Nola/Services/`, LLM code in `Nola/LLM/`, assets in `Nola/Assets.xcassets/`, and localization in `*.lproj/`. `NolaMac/` contains the native SwiftUI/AppKit macOS app; its tests live in `NolaMacTests/`. iOS unit tests live in `NolaTests/`; UI tests live in `NolaUITests/`. `cli/` provides the independent Rust CLI and Rust Web Host. `web/` contains templates/static assets plus native host helpers, and `android/` is the Kotlin/Compose client. Shared TypeScript/Bazel modules are under `modules/`; the Rust FFI core is `core/`. Docs are in `Doc/`.

## Build, Test, and Development Commands

- Build the Svelte assets and start `nola web` for the local Web client.
- `cd core && cargo test --quiet`: run Rust core tests.
- `cd core && cargo build --release`: build Rust library artifacts.
- `cd android && ./gradlew test`: run Android unit tests when Android code changes.
- `cd android && ./gradlew connectedDebugAndroidTest`: run Android instrumentation tests with a running emulator or device; install `cargo-ndk` and Android Rust targets first.
- `xcrun simctl list devices available`: list valid simulator destinations before running iOS tests; do not hard-code a simulator that is absent locally.
- `./scripts/run_client_tests.sh`: run iOS tests with bounded parallel workers and automatic cleanup of XCTest simulator clones created by the run. Set `PARALLEL_TEST_WORKERS=<count>` to tune concurrency.
- `./scripts/test_mac_layers.sh full`: run macOS tests against the architecture-qualified local Mac destination without polling connected iOS devices.

Mac XCTest must use the `Test` configuration (the scheme and runner select it).
Never run unsigned tests into `Products/Debug`: that replaces the signed app the
developer is using and breaks Keychain identity checks. Keep normal Debug builds
signed with the project's configured identity; Rust target reuse is independent
of the separate Xcode `Products/Test` output directory.

## Coding Style & Naming Conventions

Follow the style already used in the touched area. Current Swift, Kotlin, TypeScript, Rust, and Python source uses four-space indentation. Use `UpperCamelCase` for types (`ServiceContainer`, `MainActivity`), `lowerCamelCase` for Swift/Kotlin/TypeScript functions and values, and `snake_case` for Python and Rust functions. Keep SPDX/file-overview headers on files that already use them. Avoid broad refactors in feature or bug-fix PRs.

## Testing Guidelines

The repository owner grants standing authorization for compilation and testing.
Do not ask for approval again for ordinary builds or tests. Use isolated test
databases and fixtures; this authorization does not permit deleting user data,
changing production credentials, or polling locked physical iOS devices.

Place iOS Swift tests in `NolaTests/` or `NolaUITests/` and macOS tests in `NolaMacTests/`, with names ending in `Tests.swift`. Android JVM tests live in `android/app/src/test/`; device/Compose tests live in `android/app/src/androidTest/`. Rust CLI/Web tests can stay inline under `#[cfg(test)]` in `cli/`. Add tests for behavior changes, API contracts, scheduling, persistence, streaming message order, and FFI boundaries.

## Commit & Pull Request Guidelines

Git history uses Conventional Commit-style prefixes, especially `feat:`. Keep commits focused and descriptive, for example `feat: add chat thinking parser` or `fix: handle OAuth relay errors`. PRs should include a summary, validation commands, linked issues when relevant, screenshots for UI changes, docs updates for user-facing behavior, and removed-interface notes. Update current callers together; preserve the reissued 1.0.0 compatibility baseline and test any subsequent versioned adapters or data migrations.

## Security & Configuration Tips

Do not commit production secrets, local databases, generated logs, `.env` files, or provisioning material. Treat LAN mode and native-browser tokens as local-trust features.

## Agent Routing Guardrails

Do not infer tool authorization or select Web, memory, file, Skill, MCP, or platform tools by matching keywords in the user's text. The model selects a tool from its schema, and Core evaluates the concrete tool call, resource, permission, and result. Do not repair routing mistakes by adding language-specific keyword or exclusion lists; fix the tool contract, catalog, model guidance, or recoverable protocol handling instead. Regression coverage must include natural follow-ups, multilingual prompts, ambiguous words such as “search” and “current,” unknown-tool recovery, and real-model end-to-end sessions.

Architecture, audit, and comparison prompts that mention capability names such as tools, skills, memory, or permissions are one semantic task; capability names are not routing or authorization signals. Clients must not rewrite prompts or force tools from language-specific phrase lists.
