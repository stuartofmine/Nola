# Deployment and implementation map

[简体中文](CURRENT_IMPLEMENTATION_MAP.md) · English

Choose the execution authority before tracing a feature. The machine hosting the model endpoint can differ from the machine running the agent and tools. Store identity is explicit. An embedded database and a desktop Service can share a schema while owning different data.

## Platform deployment

| Client | Deployment | Implementation |
| --- | --- | --- |
| macOS | SwiftUI/AppKit client, local Application Service and a separate nola-agent-host for platform work. | [apps/macos](../../../apps/macos) · [apps/cli/src/native_host_entry.rs](../../../crates/nola-app/src/main.rs) |
| iOS / iPadOS | SwiftUI uses the embedded Application over the C ABI. Paired mode observes and controls work admitted on the source Host. | [apps/ios](../../../apps/ios) · [core/nola-mobile-bindings/src/lib.rs](../../../crates/nola-app/src/ffi.rs) |
| Android | Compose reaches the embedded Application through JNI. Local models, files and OS permissions belong to the device. Paired execution belongs to the source Host. | [apps/android](../../../apps/android) · [core/nola-mobile-bindings](../../../crates/nola-app) |
| CLI | The Rust terminal uses the local Application. Interactive startup and native agent-host stdin requests have separate entry paths. | [apps/cli/src/main_parts/bootstrap.rs](../../../crates/nola-app/src/cli.rs) · [apps/cli/src/application_port.rs](../../../crates/nola-app/src/daemon.rs) |
| Web | The Svelte client connects to a Rust HTTP Host and updates pages from receipts and journal increments. | [apps/web/frontend](../../../apps/web/frontend) · [apps/cli/src/web_parts/routes.rs](../../../crates/nola-app/src/web_server.rs) |
| Windows | Tauri 2 hosts the shared Web UI in WebView2. The desktop shell owns the local Rust Host process and its lifetime. | [apps/windows/src-tauri](../../../apps/windows/src-tauri) · [apps/windows/src](../../../apps/windows/src) |

## Module entry points

| Module contract | Sources |
| --- | --- |
| [01 · Domain and protocols](../模块/01-领域与协议/README.en.md) | [nola-domain/src/lib.rs](../../../crates/nola-types/src/lib.rs) · [nola-protocol/src/lib.rs](../../../crates/nola-types/src/lib.rs) · [nola-host-protocol/src/lib.rs](../../../crates/nola-types/src/lib.rs) |
| [02 · Core and the agent state machine](../模块/02-Core与状态机/README.en.md) | [nola-agent-engine/src/reducer_port.rs](../../../crates/nola-engine/src/lib.rs) · [agent/runtime/submission.rs](../../../crates/nola-engine/src/loop_runner.rs) · [core/src/lib.rs](../../../crates/nola-app/src/lib.rs) |
| [03 · Application commands and transactions](../模块/03-Application与事务/README.en.md) | [nola-application/src/application_command.rs](../../../crates/nola-app/src/daemon.rs) · [nola-application/src/lib.rs](../../../crates/nola-modules/src/lib.rs) · [nola-application-bootstrap/src/runtime_owner.rs](../../../crates/nola-app/src/daemon.rs) |
| [04 · Storage, journals and indexes](../模块/04-存储与索引/README.en.md) | [nola-store/src/schema_store.rs](../../../crates/nola-store/src/db.rs) · [nola-store/src/canonical_text_store.rs](../../../crates/nola-store/src/db.rs) · [nola-store-port/src/assistant_journal.rs](../../../crates/nola-store/src/db.rs) |
| [05 · Execution, cancellation and recovery](../模块/05-执行与恢复/README.en.md) | [nola-runtime-driver/src/lib.rs](../../../crates/nola-engine/src/loop_runner.rs) · [nola-runtime-driver/src/task_execution_authority.rs](../../../crates/nola-engine/src/subagents.rs) · [cli/src/native_host_entry.rs](../../../crates/nola-app/src/main.rs) |
| [06 · Tools and execution evidence](../模块/06-工具与证据/README.en.md) | [nola-application/src/tool_source.rs](../../../crates/nola-tools/src/lib.rs) · [nola-store/src/tool_source.rs](../../../crates/nola-store/src/db.rs) · [nola-domain/src/execution_evidence.rs](../../../crates/nola-types/src/tool.rs) |
| [07 · Permissions and platform capabilities](../模块/07-权限与平台/README.en.md) | [agent/runtime/permission_policy.rs](../../../crates/nola-engine/src/policy.rs) · [nola-store/src/permission_source.rs](../../../crates/nola-engine/src/policy.rs) · [src/tools/platform_catalog_contract_tests.rs](../../../crates/nola-tools/src/tests.rs) |
| [08 · Model configuration and inference](../模块/08-模型配置与调用/README.en.md) | [nola-application/src/model.rs](../../../crates/nola-llm/src/lib.rs) · [nola-application/src/model_profile_resolution.rs](../../../crates/nola-app/src/config.rs) · [nola-credential-host/src/lib.rs](../../../crates/nola-store/src/vault.rs) |
| [09 · Context, compaction and recall](../模块/09-上下文与压缩/README.en.md) | [nola-application/src/context.rs](../../../crates/nola-engine/src/compaction.rs) · [agent/runtime/compaction_driver.rs](../../../crates/nola-engine/src/compaction.rs) · [src/llm_parts/request_snapshot.rs](../../../crates/nola-llm/src/client.rs) |
| [10 · Goals, tasks and experts](../模块/10-Goal任务与子代理/README.en.md) | [nola-application/src/goal_execution.rs](../../../crates/nola-modules/src/goals_sops.rs) · [src/agent_room/submission.rs](../../../crates/nola-modules/src/rooms_peers.rs) · [nola-mobile-bindings/src/child_pump.rs](../../../crates/nola-engine/src/subagents.rs) |
| [11 · Skills, plugins and MCP](../模块/11-Skill插件与MCP/README.en.md) | [nola-application/src/skill.rs](../../../crates/nola-modules/src/skills.rs) · [nola-application/src/mcp.rs](../../../crates/nola-mcp/src/lib.rs) · [nola-mcp-host/src/lib.rs](../../../crates/nola-mcp/src/lib.rs) |
| [12 · Memory, data sources and automation](../模块/12-记忆数据源与自动化/README.en.md) | [nola-application/src/memory.rs](../../../crates/nola-modules/src/memory.rs) · [nola-application/src/data_source.rs](../../../crates/nola-modules/src/data_sources.rs) · [nola-application/src/automation_execution.rs](../../../crates/nola-modules/src/automations_messaging.rs) |
| [13 · Clients, conversation projections and A2UI](../模块/13-客户端投影/README.en.md) | [nola-mobile-bindings/src/lib.rs](../../../crates/nola-app/src/ffi.rs) · [nola-mobile-bindings/src/projection.rs](../../../crates/nola-app/src/ffi.rs) · [apple/Protocol/RemoteSubmissionIdentity.swift](../../../shared/apple/Protocol/RemoteSubmissionIdentity.swift) |
| [14 · Remote Hosts and synchronization](../模块/14-远程Host与同步/README.en.md) | [src/web_parts/routes.rs](../../../crates/nola-app/src/web_server.rs) · [transport/tailcat/README.md](../../../transport/tailcat/README.md) |
| [15 · Measurement, tests and release acceptance](../模块/15-观测与验收/README.en.md) | [nola-measurement/src/lib.rs](../../../crates/nola-types/src/budget.rs) · [tests/suites/dependency_boundary.rs](../../../crates/nola-engine/src/tests.rs) · [../scripts/test_mac_layers.sh](../../../scripts/test_mac_layers.sh) |

## Artifacts and evidence

The agent engine implements the state machine. The Core facade produces an rlib. Mobile Bindings packages the final native libraries and retains the helper symbols used across the native boundary. Application Bootstrap assembles the concrete database implementation. The source Host owns remote results and the receiving client resumes a projection.

Check the actual manifest edges separately from a call path. A component test verifies only the behavior it exercised. Native installation, UI input, live inference and reconnecting an interrupted long task each require their own evidence. [Cargo dependencies](DEPENDENCIES.en.md) · [Version policy](VERSION_COMPATIBILITY.en.md) · [Measurement and acceptance](../模块/15-观测与验收/README.en.md)
