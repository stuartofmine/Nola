// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Own stable built-in tool metadata and platform execution profiles.
// - Security: Runtime manifests report handlers; they cannot redefine policy metadata.

use crate::agent::policy::{capability_for_tool_name, HostCapability};
use crate::tool_policy::{ToolPermission, ToolPlatform, ToolRisk, ToolRoute};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinToolMetadata {
    pub tool_id: String,
    pub capability: String,
    pub risk: ToolRisk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformToolProfile {
    pub platform: ToolPlatform,
    pub local_route: ToolRoute,
    pub supports_device_bridge: bool,
    pub supports_agent_host: bool,
    pub supports_web_client: bool,
}

pub const CANONICAL_DEVICE_TOOL_IDS: &[&str] = &[
    "device_read_file",
    "device_write_file",
    "device_clipboard_read",
    "device_clipboard_write",
    "device_get_location",
    "device_create_calendar_event",
    "device_search_calendar_events",
];

pub fn canonical_device_tool_capability(tool_id: &str) -> Option<&'static str> {
    match normalize_tool_id(tool_id).as_str() {
        "device_read_file" => Some("files_read"),
        "device_write_file" => Some("files_write"),
        "device_clipboard_read" | "device_clipboard_write" => Some("clipboard"),
        "device_get_location" => Some("location"),
        "device_create_calendar_event" | "device_search_calendar_events" => Some("calendar"),
        _ => None,
    }
}

pub fn normalize_tool_id(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', '.'], "_")
}

pub fn platform_profile(platform: ToolPlatform) -> PlatformToolProfile {
    match platform {
        ToolPlatform::Ios | ToolPlatform::Android => PlatformToolProfile {
            platform,
            local_route: ToolRoute::Device,
            supports_device_bridge: true,
            supports_agent_host: true,
            supports_web_client: false,
        },
        ToolPlatform::Macos | ToolPlatform::Cli | ToolPlatform::AgentHost => PlatformToolProfile {
            platform,
            local_route: ToolRoute::AgentHost,
            supports_device_bridge: true,
            supports_agent_host: true,
            supports_web_client: false,
        },
        ToolPlatform::Web => PlatformToolProfile {
            platform,
            local_route: ToolRoute::WebClient,
            supports_device_bridge: true,
            supports_agent_host: true,
            supports_web_client: true,
        },
    }
}

pub fn builtin_tool_metadata(tool_id: &str) -> Option<BuiltinToolMetadata> {
    let tool_id = normalize_tool_id(tool_id);
    if tool_id.is_empty() {
        return None;
    }
    if tool_id.starts_with("device_") {
        let capability = canonical_device_tool_capability(&tool_id)?;
        return Some(BuiltinToolMetadata {
            risk: risk_for(&tool_id, capability),
            tool_id,
            capability: capability.to_string(),
        });
    }
    let canonical = tool_id.strip_prefix("host_").unwrap_or(&tool_id);
    // Core-owned orchestration tools must not inherit file/process capabilities
    // from tool-name heuristics. Their data access is governed by the feature
    // setting and Host implementation, not by a synthetic filesystem grant.
    let capability = explicit_capability(canonical)
        .or_else(|| internal_control_tool(canonical).then_some("none"))
        .or_else(|| capability_from_runtime(canonical))?;
    Some(BuiltinToolMetadata {
        risk: risk_for(&tool_id, capability),
        tool_id,
        capability: capability.to_string(),
    })
}

fn code_intelligence_capability(name: &str) -> Option<&'static str> {
    // Exact Core-owned operations only. Hosts retain source-scope and index
    // mutation admission; arbitrary provider-prefixed tools stay unknown.
    match name {
        "codebase_index_status" | "codebase_projects" | "codebase_architecture"
        | "codebase_search" | "codebase_text_search" | "codebase_trace"
        | "codebase_read_symbol" | "codebase_query" | "codebase_schema"
        | "codebase_check_coverage" | "codebase_changed_impact" => Some("files_read"),
        "codebase_index" | "codebase_delete_index" | "codebase_adr"
        | "codebase_ingest_traces" => Some("code_index"),
        _ => None,
    }
}

fn explicit_capability(name: &str) -> Option<&'static str> {
    if let Some(capability) = code_intelligence_capability(name) { return Some(capability); }
    match name {
        "workspace_access" => Some("files_read"),
        "clipboard_read" | "clipboard_write" => Some("clipboard"),
        "computer_status" | "conversation_recall" | "render_ui" => Some("none"),
        "get_location" => Some("location"),
        "create_calendar_event" | "search_calendar_events" => Some("calendar"),
        "search_reminders" | "create_reminder" | "update_reminder" => Some("reminders"),
        "search_contacts" => Some("contacts"),
        "query_health" | "health_summary" => Some("health"),
        "pick_photo" | "search_photos" => Some("photos"),
        "system_time" => Some("none"),
        _ => None,
    }
}

fn capability_from_runtime(name: &str) -> Option<&'static str> {
    match capability_for_tool_name(name) {
        HostCapability::None => None,
        HostCapability::FilesRead => Some("files_read"),
        HostCapability::FilesWrite => Some("files_write"),
        HostCapability::Process => Some("process"),
        HostCapability::Network => Some("network"),
        HostCapability::Background => Some("background"),
        HostCapability::Camera => Some("camera"),
        HostCapability::Microphone => Some("microphone"),
        HostCapability::Photos => Some("photos"),
        HostCapability::Location => Some("location"),
        HostCapability::Calendar => Some("calendar"),
        HostCapability::Reminders => Some("reminders"),
        HostCapability::Contacts => Some("contacts"),
        HostCapability::Health => Some("health"),
        HostCapability::Notifications => Some("notifications"),
        HostCapability::Clipboard => Some("clipboard"),
        HostCapability::Computer => Some("computer"),
    }
}

fn internal_control_tool(name: &str) -> bool {
    matches!(
        name,
        "agent_config_context"
            | "agent_context"
            | "agent_plan_file"
            | "agent_task_status"
            | "delegate_agent"
            | "goal_complete"
            | "memory_search"
            | "memory_save"
            | "memory_update"
            | "propose_agent_plan"
            | "request_user_input"
            | "send_agent_message"
            | "set_thinking_mode"
            | "skill"
            | "skill_context"
            | "skill_invoke"
            | "skill_list"
            | "skill_skip"
            | "skill_update"
            | "sop"
            | "sop_invoke"
            | "sop_run_update"
            | "tool_search"
            | "update_agent_todos"
            | "workspace_checkpoint"
    ) || name.starts_with("tool_output_")
}

fn risk_for(tool_id: &str, capability: &str) -> ToolRisk {
    if tool_id == "workspace_access" {
        return ToolRisk::Medium;
    }
    if matches!(
        tool_id,
        "device_clipboard_read"
            | "device_get_location"
            | "device_search_calendar_events"
            | "device_search_reminders"
            | "device_search_contacts"
            | "device_query_health"
            | "device_health_summary"
    ) {
        return ToolRisk::ReadOnly;
    }
    match capability {
        "none" | "files_read" => ToolRisk::ReadOnly,
        "network" | "health" => ToolRisk::Low,
        "code_index" | "files_write" | "background" | "camera" | "microphone" | "photos" | "location"
        | "calendar" | "reminders" | "contacts" | "notifications" | "clipboard" => ToolRisk::Medium,
        "process" | "computer" => ToolRisk::High,
        _ => ToolRisk::Critical,
    }
}

pub fn default_route(platform: ToolPlatform, tool_id: &str) -> ToolRoute {
    let id = normalize_tool_id(tool_id);
    if requires_agent_host_route(&id) {
        return ToolRoute::AgentHost;
    }
    if id.starts_with("device_") {
        return ToolRoute::Device;
    }
    if id.starts_with("host_") {
        return ToolRoute::AgentHost;
    }
    if matches!(id.as_str(), "web_search" | "web_fetch" | "fetch_url") {
        return ToolRoute::Cloud;
    }
    platform_profile(platform).local_route
}

fn requires_agent_host_route(id: &str) -> bool {
    matches!(id, "terminal_input" | "language_service_list" | "language_service_query" | "git_worktree_list" | "git_worktree_create" | "git_worktree_integrate")
        || code_intelligence_capability(id).is_some()
}

pub fn route_supported(platform: ToolPlatform, tool_id: &str, route: ToolRoute) -> bool {
    let id = normalize_tool_id(tool_id);
    let profile = platform_profile(platform);
    // These handlers own Host-side processes/configuration. A mobile client
    // advertising the broad background/files capability cannot implement them
    // through its native device bridge or move their execution to the browser.
    if requires_agent_host_route(&id) {
        return route == ToolRoute::AgentHost && profile.supports_agent_host;
    }
    if id.starts_with("device_") {
        return route == ToolRoute::Device && profile.supports_device_bridge;
    }
    if id.starts_with("host_") {
        return route == ToolRoute::AgentHost && profile.supports_agent_host;
    }
    if matches!(id.as_str(), "web_search" | "web_fetch" | "fetch_url") {
        return match route {
            ToolRoute::Cloud => true,
            ToolRoute::AgentHost => profile.supports_agent_host,
            ToolRoute::WebClient => profile.supports_web_client,
            ToolRoute::Device => false,
        };
    }
    match platform {
        ToolPlatform::Ios | ToolPlatform::Android => {
            route == ToolRoute::Device || route == ToolRoute::AgentHost
        }
        ToolPlatform::Macos | ToolPlatform::Cli | ToolPlatform::AgentHost => {
            route == ToolRoute::AgentHost
        }
        ToolPlatform::Web => route == ToolRoute::WebClient || route == ToolRoute::AgentHost,
    }
}

pub fn capability_supported(platform: ToolPlatform, route: ToolRoute, capability: &str) -> bool {
    if route == ToolRoute::AgentHost || route == ToolRoute::Cloud {
        return true;
    }
    if route == ToolRoute::Device
        && matches!(
            platform,
            ToolPlatform::Macos | ToolPlatform::Cli | ToolPlatform::Web | ToolPlatform::AgentHost
        )
    {
        return true;
    }
    match platform {
        ToolPlatform::Ios | ToolPlatform::Android => !matches!(capability, "process" | "computer"),
        ToolPlatform::Macos | ToolPlatform::Cli | ToolPlatform::AgentHost => true,
        ToolPlatform::Web => route == ToolRoute::WebClient,
    }
}

pub fn permission_for(
    platform: ToolPlatform,
    route: ToolRoute,
    capability: &str,
) -> ToolPermission {
    if capability == "none" {
        return ToolPermission::None;
    }
    match route {
        ToolRoute::AgentHost => ToolPermission::AgentApproval,
        ToolRoute::Cloud | ToolRoute::WebClient => ToolPermission::PlatformCapability,
        ToolRoute::Device => match capability {
            "files_read" | "files_write" => ToolPermission::ScopedResource,
            "network" => ToolPermission::PlatformCapability,
            "background" if matches!(platform, ToolPlatform::Ios | ToolPlatform::Android) => {
                ToolPermission::PlatformLifecycle
            }
            "background" => ToolPermission::PlatformCapability,
            "clipboard" => ToolPermission::AgentApproval,
            _ => ToolPermission::SystemPermission,
        },
    }
}

pub fn requires_result_egress(route: ToolRoute, capability: &str) -> bool {
    route == ToolRoute::Device && capability != "none"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_process_tools_cannot_be_advertised_as_device_or_web_handlers() {
        for platform in [ToolPlatform::Ios, ToolPlatform::Android, ToolPlatform::Macos,
            ToolPlatform::Cli, ToolPlatform::Web, ToolPlatform::AgentHost] {
            for name in ["terminal_input", "language_service_list", "language_service_query", "git_worktree_list", "git_worktree_create", "git_worktree_integrate"] {
                assert_eq!(default_route(platform, name), ToolRoute::AgentHost);
                assert!(route_supported(platform, name, ToolRoute::AgentHost));
                for route in [ToolRoute::Device, ToolRoute::Cloud, ToolRoute::WebClient] {
                    assert!(!route_supported(platform, name, route), "{platform:?}/{name}/{route:?}");
                }
            }
        }
    }

    #[test]
    fn apple_stable_ids_have_core_owned_metadata() {
        let cases = [
            ("device_read_file", "files_read"),
            ("device_write_file", "files_write"),
            ("device_clipboard_read", "clipboard"),
            ("device_clipboard_write", "clipboard"),
            ("device_get_location", "location"),
            ("device_create_calendar_event", "calendar"),
            ("device_search_calendar_events", "calendar"),
        ];
        for (tool_id, capability) in cases {
            assert_eq!(
                builtin_tool_metadata(tool_id).unwrap().capability,
                capability
            );
        }
    }

    #[test]
    fn unknown_tools_do_not_inherit_none_capability() {
        assert!(builtin_tool_metadata("totally_unknown_handler").is_none());
        assert!(builtin_tool_metadata("device_read_clipboard").is_none());
        assert!(builtin_tool_metadata("device_write_clipboard").is_none());
        assert_eq!(
            builtin_tool_metadata("request_user_input")
                .unwrap()
                .capability,
            "none"
        );
    }

    #[test]
    fn internal_skill_and_memory_tools_do_not_inherit_file_capabilities() {
        for tool_id in [
            "skill_invoke",
            "skill_context",
            "skill_list",
            "sop_run_update",
            "memory_search",
            "memory_save",
            "memory_update",
        ] {
            let metadata = builtin_tool_metadata(tool_id).unwrap();
            assert_eq!(metadata.capability, "none", "{tool_id}");
            assert_eq!(metadata.risk, ToolRisk::ReadOnly, "{tool_id}");
            assert_eq!(
                permission_for(ToolPlatform::Ios, ToolRoute::Device, &metadata.capability),
                ToolPermission::None,
                "{tool_id}"
            );
            assert!(!requires_result_egress(
                ToolRoute::Device,
                &metadata.capability
            ));
        }
    }

    #[test]
    fn native_orchestration_is_a_background_capability() {
        let metadata = builtin_tool_metadata("nola_orchestrate").unwrap();
        assert_eq!(metadata.capability, "background");
        assert_eq!(metadata.risk, ToolRisk::Medium);
        assert_eq!(
            permission_for(
                ToolPlatform::Macos,
                ToolRoute::AgentHost,
                &metadata.capability,
            ),
            ToolPermission::AgentApproval
        );
    }

    #[test]
    fn interactive_host_tools_have_precise_capabilities() {
        assert_eq!(
            builtin_tool_metadata("computer_status").unwrap().capability,
            "none"
        );
        assert_eq!(
            builtin_tool_metadata("computer_get_clipboard")
                .unwrap()
                .capability,
            "clipboard"
        );
        assert_eq!(
            builtin_tool_metadata("chrome_native_open_page")
                .unwrap()
                .capability,
            "network"
        );
    }
}

#[cfg(test)]
#[test]
fn native_code_memory_metadata_is_host_only_and_unknown_ids_stay_closed() {
    for (id, capability, risk) in [
        ("codebase_search", "files_read", ToolRisk::ReadOnly),
        ("codebase_index", "code_index", ToolRisk::Medium),
    ] {
        let metadata = builtin_tool_metadata(id).unwrap();
        assert_eq!(metadata.capability, capability);
        assert_eq!(metadata.risk, risk);
        for platform in [ToolPlatform::Cli, ToolPlatform::Web, ToolPlatform::Ios] {
            assert_eq!(default_route(platform, id), ToolRoute::AgentHost);
            assert!(route_supported(platform, id, ToolRoute::AgentHost));
            for route in [ToolRoute::Device, ToolRoute::Cloud, ToolRoute::WebClient] {
                assert!(!route_supported(platform, id, route));
            }
        }
    }
    assert!(builtin_tool_metadata("codebase_invented_tool").is_none());
}
