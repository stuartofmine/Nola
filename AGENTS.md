> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Repository Guidelines

## Project Structure & Module Organization

Nola is a local-first personal AI assistant with several clients and shared services. `Nola/` contains the SwiftUI iOS app, with UI in `Nola/UI/`, services in `Nola/Services/`, LLM code in `Nola/LLM/`, assets in `Nola/Assets.xcassets/`, and localization in `*.lproj/`. `NolaMac/` contains the native SwiftUI/AppKit macOS app; its tests live in `NolaMacTests/`. iOS unit tests live in `NolaTests/`; UI tests live in `NolaUITests/`. `backend/worker/` is the primary Cloudflare Worker + D1 backend; `backend/app/` contains the Python backend implementation. `cli/` provides the independent Rust CLI and Rust Web backend. `web/` contains templates/static assets plus native host helpers, and `android/` is the Kotlin/Compose client. Shared TypeScript/Bazel modules are under `modules/`; the Rust FFI core is `core/`. Docs are in `Doc/`.

## Build, Test, and Development Commands

- `./dev.sh`: start the local backend and web development stack with reload.
- `cd backend && npm run typecheck`: run the Worker TypeScript check used by CI.
- `cd backend && python -m pytest -q`: run backend Python tests.
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

Place iOS Swift tests in `NolaTests/` or `NolaUITests/` and macOS tests in `NolaMacTests/`, with names ending in `Tests.swift`. Backend tests live in `backend/tests/` and `backend/tests_integration/` using `test_*.py`. Android JVM tests live in `android/app/src/test/`; device/Compose tests live in `android/app/src/androidTest/`. Rust CLI/Web tests can stay inline under `#[cfg(test)]` in `cli/`. Add tests for behavior changes, API contracts, scheduling, persistence, streaming message order, and FFI boundaries.

## Commit & Pull Request Guidelines

Git history uses Conventional Commit-style prefixes, especially `feat:`. Keep commits focused and descriptive, for example `feat: add chat thinking parser` or `fix: handle OAuth relay errors`. PRs should include a summary, validation commands, linked issues when relevant, screenshots for UI changes, docs updates for user-facing behavior, and removed-interface notes. Update all current callers together; never add old-version shims or data converters.

## Security & Configuration Tips

Do not commit production secrets, local databases, generated logs, `.env` files, or provisioning material. Backend configuration is loaded from `nola_backend_env` or `NOLA_BACKEND_ENV` JSON; local web examples start from `web/.env.example`. Treat LAN mode and native-browser tokens as local-trust features.

## Agent Routing Guardrails

Do not infer tool authorization or select Web, memory, file, Skill, MCP, or platform tools by matching keywords in the user's text. The model selects a tool from its schema, and Core evaluates the concrete tool call, resource, permission, and result. Do not repair routing mistakes by adding language-specific keyword or exclusion lists; fix the tool contract, catalog, model guidance, or recoverable protocol handling instead. Regression coverage must include natural follow-ups, multilingual prompts, ambiguous words such as “search” and “current,” unknown-tool recovery, and real-model end-to-end sessions.

Architecture, audit, and comparison prompts that mention capability names such as tools, skills, memory, or permissions are one semantic task; capability names are not routing or authorization signals. Clients must not rewrite prompts or force tools from language-specific phrase lists.
