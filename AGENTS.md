> **发布兼容约束：Nola 2.0.0（v2 精简 8-crate 架构）是新的受支持基线，SQLite 数据库格式 `user_version = 2`，统一单一二进制 `nola` 与统一 FFI 静态库 `nola-app`；此前 v1 开发格式不保留兼容或静默转换代码。从 2.0.0 基线起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式（含旧版 v1 数据库）明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Repository Guidelines

## Project Structure & Module Organization

Nola is a local-first personal AI assistant with native clients and a unified 8-crate Rust workspace under `crates/`:
- `crates/nola-types`: shared domain models, messages, tool specs, budgets, policy, and module types.
- `crates/nola-store`: SQLite WAL + FTS5 unified store (`user_version = 2`), encrypted vault, and service lock.
- `crates/nola-llm`: multi-provider LLM client (OpenAI Chat/Responses, Anthropic, Gemini, Sign in with ChatGPT, SSE, privacy redactor).
- `crates/nola-tools`: unified tool runtime (file mutation pipeline, OS sandboxed shell/PTY, ripgrep/glob search, LSP pool, code tools, binary doc readers, SSRF-safe web fetch/search, CDP browser, desktop automation, QuickJS code mode).
- `crates/nola-mcp`: MCP client manager with stdio/HTTP/SSE transports and OAuth 2.1 / PKCE.
- `crates/nola-engine`: agent execution loop, compaction, hooks, loop guard, middleware, planning, policy, and subagents.
- `crates/nola-modules`: persistent memory, code memory, skills, goals/SOPs, data sources, rooms/peers, automations/messaging, scheduler, A2UI, local models/plugins, and signed software updater.
- `crates/nola-app`: single unified `nola` binary (CLI, TUI, local Web/HTTP Host, background daemon, update verifier) and C ABI / JNI FFI library (`rlib`, `staticlib`, `cdylib`).

Platform clients live under `apps/`: `apps/ios/` (SwiftUI iOS app, tests in `tests/apple/ios/` and `tests/apple/ios-ui/`), `apps/macos/` (SwiftUI/AppKit macOS app, tests in `tests/apple/macos/`), `apps/android/` (Kotlin/Compose client), `apps/windows/` (Tauri 2 desktop shell), and `apps/web/` (Svelte/TypeScript Web client + browser runtime bridge). Shared Apple code and design tokens live in `shared/`. Public docs are in `docs/`, organized by purpose and platform. `Doc/当前任务/` and `Doc/历史任务/` are local-only records; do not track or export them. Historical implementations in `legacy/` are also local-only and must not be tracked or exported.

## Build, Test, and Development Commands

- Build the Svelte assets (`npm --prefix apps/web/frontend run build`) and start `nola web --web-root "$PWD/apps/web"` for the local Web client.
- `cargo test --locked --workspace --lib`: run all Rust workspace unit tests.
- `cargo test --locked -p <crate> --lib [filter]`: run focused unit tests for a single crate (e.g. `nola-engine`, `nola-store`, `nola-tools`, `nola-modules`, `nola-app`).
- `cargo build --locked --release -p nola-app --bin nola`: build the unified `nola` release binary.
- `./scripts/build_apple_core.sh`, `./scripts/build_mac_runtime.sh`, `./scripts/build_android_core.sh`, `./scripts/build_cli_runtime.sh`: build native static libraries and bundled runtimes through the shared cache launcher.
- `cd apps/android && ./gradlew test`: run Android unit tests when Android code changes.
- `cd apps/android && ./gradlew connectedDebugAndroidTest`: run Android instrumentation tests with a running emulator or device; install `cargo-ndk` and Android Rust targets first.
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

Place iOS Swift tests in `tests/apple/ios/` or `tests/apple/ios-ui/` and macOS tests in `tests/apple/macos/`, with names ending in `Tests.swift`. Android JVM tests live in `apps/android/app/src/test/`; device/Compose tests live in `apps/android/app/src/androidTest/`. Rust unit tests live under `#[cfg(test)]` inside each crate in `crates/` (e.g. `crates/nola-engine/src/tests.rs`, `crates/nola-app/src/cli.rs`). Add tests for behavior changes, API contracts, scheduling, persistence, streaming message order, and FFI boundaries.

## Commit & Pull Request Guidelines

When the owner asks to push or publish Nola, update the public repository
`https://github.com/stuartofmine/Nola.git` on `main` by default. Use
`scripts/releases/export_public.py` to produce a reviewed source snapshot from
the intended commit, then commit that snapshot on top of the current public
`main` in a clean, isolated checkout. Preserve the public history and independent
release files. Verify the exported diff and the remote commit after pushing.
Never push the development branch or its history directly to the public remote.
Use the development remote only when the owner asks for it explicitly.

Git history uses Conventional Commit-style prefixes, especially `feat:`. Keep commits focused and descriptive, for example `feat: add chat thinking parser` or `fix: handle OAuth relay errors`. PRs should include a summary, validation commands, linked issues when relevant, screenshots for UI changes, docs updates for user-facing behavior, and removed-interface notes. Update current callers together; preserve the 2.0.0 compatibility baseline and test any subsequent versioned adapters or data migrations.

## Security & Configuration Tips

Do not commit production secrets, local databases, generated logs, `.env` files, or provisioning material. Treat LAN mode and native-browser tokens as local-trust features.

## Agent Routing Guardrails

Do not infer tool authorization or select Web, memory, file, Skill, MCP, or platform tools by matching keywords in the user's text. The model selects a tool from its schema, and Core evaluates the concrete tool call, resource, permission, and result. Do not repair routing mistakes by adding language-specific keyword or exclusion lists; fix the tool contract, catalog, model guidance, or recoverable protocol handling instead. Regression coverage must include natural follow-ups, multilingual prompts, ambiguous words such as “search” and “current,” unknown-tool recovery, and real-model end-to-end sessions.

Architecture, audit, and comparison prompts that mention capability names such as tools, skills, memory, or permissions are one semantic task; capability names are not routing or authorization signals. Clients must not rewrite prompts or force tools from language-specific phrase lists.

## Rust Build Scope

Filtered Rust unit tests must select `--lib`; integration tests must select
`--test <target>`. A bare filter still compiles every integration target and can
link many large test executables without running them. For example use
`cargo test -p nola-engine --lib compaction`, or
`cargo test -p nola-app --lib resolve_route`.
Use `scripts/test_rust.sh <package> [filter]` for the bounded unit-test default.
Preview disposable integration runner cleanup with
`python3 scripts/maintain_build_storage.py`; `--apply` preserves dependencies,
normal executables, release outputs, databases and recorded task evidence.
