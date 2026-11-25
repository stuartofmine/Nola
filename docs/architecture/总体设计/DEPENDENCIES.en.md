# Rust workspace dependencies

[简体中文](DEPENDENCIES.md) · English

The table includes direct workspace dependencies declared for production or build use, including optional and target-specific edges. The root Cargo workspace and the standalone Windows Tauri workspace have separate manifests. Reproduce the table from the root with `cargo metadata --offline --locked --no-deps --format-version 1`. Keep packages in `workspace_members`, exclude dev dependencies, and use package names when a dependency has a local alias.

| Package | Direct workspace dependencies |
| --- | --- |
| `nola-code-memory` | None |
| `nola_agent_engine` | `nola_domain`, `nola_host_protocol`, `nola_kernel` |
| `nola_application` | `nola_agent_engine`, `nola_domain`, `nola_host_protocol`, `nola_kernel`, `nola_measurement`, `nola_protocol`, `nola_store_port` |
| `nola_application_bootstrap` | `nola_application`, `nola_store` |
| `nola_cli` | `nola-code-memory`, `nola_application`, `nola_application_bootstrap`, `nola_code_mode_host`, `nola_core`, `nola_credential_host`, `nola_domain`, `nola_host_protocol`, `nola_mcp_host`, `nola_measurement`, `nola_plugin_host`, `nola_protocol`, `nola_runtime_driver`, `nola_runtime_supervisor`, `nola_service`, `nola_service_client`, `nola_skill_host`, `nola_store_port`, `nola_update_host` |
| `nola_code_mode_host` | `nola_domain` |
| `nola_core` | `nola_agent_engine`, `nola_domain`, `nola_host_protocol`, `nola_kernel` |
| `nola_credential_host` | `nola_windows_permissions` |
| `nola_domain` | None |
| `nola_host_protocol` | `nola_domain` |
| `nola_kernel` | `nola_domain` |
| `nola_mcp_host` | `nola_core` |
| `nola_measurement` | None |
| `nola_mobile_bindings` | `nola_application`, `nola_application_bootstrap`, `nola_core`, `nola_domain`, `nola_host_protocol`, `nola_kernel`, `nola_mcp_host`, `nola_protocol`, `nola_runtime_driver`, `nola_skill_host`, `nola_update_host` |
| `nola_model_runtime_host` | None |
| `nola_plugin_host` | `nola_domain` |
| `nola_protocol` | `nola_domain` |
| `nola_runtime_driver` | `nola_domain`, `nola_host_protocol` |
| `nola_runtime_supervisor` | `nola_domain`, `nola_protocol`, `nola_service_client` |
| `nola_service` | `nola_application`, `nola_application_bootstrap`, `nola_domain`, `nola_protocol`, `nola_windows_permissions` |
| `nola_service_client` | `nola_protocol`, `nola_windows_permissions` |
| `nola_skill_host` | None |
| `nola_store` | `nola_domain`, `nola_kernel`, `nola_measurement`, `nola_store_port` |
| `nola_store_port` | `nola_domain` |
| `nola_update_host` | `nola_domain` |
| `nola_windows_permissions` | None |

Domain owns identities and invariants. Kernel depends on Domain for decisions. Protocols and Store Port describe boundaries. Application coordinates mutations through Store Port. Bootstrap supplies the SQLite implementation. These arrows describe what can be linked. Whether a path executes on a particular device depends on its Host, configuration and admission.

The driver operates the transport loop and task worker. Application owns Goal coordination. The engine is a development dependency of the driver. The CLI composes service, model, Skill, plugin and execution adapters. Mobile Bindings produces the native deliverables and links the adapters available to embedded hosts. The Windows permissions crate supplies the OS-specific ACL primitives.

Use the dependency boundary integration suite to check layering. It compares declared edges, including renamed dependencies. A passing structural check supplies no evidence about authentication, physical tool execution or GUI behavior. [Workspace manifest](../../../Cargo.toml) · [Boundary tests](../../../crates/nola-engine/src/tests.rs) · [Windows manifest](../../../apps/windows/src-tauri/Cargo.toml)
