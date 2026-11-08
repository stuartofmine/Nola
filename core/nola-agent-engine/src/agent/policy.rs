// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns platform-neutral capability and context-window policy.
// - Boundary: Tool implementations and UI permission prompts belong to Hosts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    None,
    FilesRead,
    FilesWrite,
    Process,
    Network,
    Background,
    Camera,
    Microphone,
    Photos,
    Location,
    Calendar,
    Reminders,
    Contacts,
    Health,
    Notifications,
    Clipboard,
    Computer,
}

pub const DEFAULT_ACTIVE_CONTEXT_TOKEN_CEILING: usize = 96_000;
pub const DEFAULT_MODEL_OUTPUT_TOKENS: usize = 16_384;
pub const SAFE_MODEL_OUTPUT_TOKEN_CEILING: usize = 32_768;
pub const ABSOLUTE_MODEL_OUTPUT_TOKEN_CEILING: usize = 200_000;

pub fn capability_for_tool_name(tool_name: &str) -> HostCapability {
    let name = normalize_tool_name(tool_name);

    if matches!(
        name.as_str(),
        "computer_get_clipboard"
            | "computer_set_clipboard"
            | "computer_copy_selection"
            | "computer_assert_clipboard"
            | "computer_wait_for_clipboard"
    ) {
        return HostCapability::Clipboard;
    }
    if is_read_tool(&name) {
        return HostCapability::FilesRead;
    }
    if is_write_tool(&name) {
        return HostCapability::FilesWrite;
    }
    if is_background_tool(&name) {
        return HostCapability::Background;
    }
    if is_process_tool(&name) {
        return HostCapability::Process;
    }
    if is_network_tool(&name) {
        return HostCapability::Network;
    }
    if matches!(name.as_str(), "camera" | "take_photo") {
        return HostCapability::Camera;
    }
    if matches!(name.as_str(), "microphone" | "record_audio") {
        return HostCapability::Microphone;
    }
    if matches!(
        name.as_str(),
        "photos" | "photo_library" | "pick_photo" | "search_photos"
    ) {
        return HostCapability::Photos;
    }
    if matches!(
        name.as_str(),
        "location" | "current_location" | "get_location"
    ) {
        return HostCapability::Location;
    }
    if matches!(
        name.as_str(),
        "calendar" | "calendar_events" | "create_calendar_event" | "search_calendar_events"
    ) {
        return HostCapability::Calendar;
    }
    if matches!(
        name.as_str(),
        "reminders" | "search_reminders" | "create_reminder" | "update_reminder"
    ) {
        return HostCapability::Reminders;
    }
    if matches!(name.as_str(), "contacts" | "search_contacts") {
        return HostCapability::Contacts;
    }
    if matches!(
        name.as_str(),
        "health" | "health_data" | "query_health" | "health_summary"
    ) {
        return HostCapability::Health;
    }
    if matches!(name.as_str(), "notifications" | "schedule_notification") {
        return HostCapability::Notifications;
    }
    if is_computer_tool(&name) {
        return HostCapability::Computer;
    }

    HostCapability::None
}

/// Core-owned built-in prompt policy. Resource containment remains enforced
/// by the executing Host even for operations without a user prompt.
pub fn tool_permission_domain(tool_name: &str) -> Option<String> {
    let domain = permission_name_for_tool(tool_name);
    let internal = matches!(normalize_tool_name(tool_name).as_str(),
        "system_time" | "skill" | "skill_update" | "skill_skip" | "sop"
        | "sop_invoke" | "sop_run_update"
        | "bash_background_status" | "bash_background_wait"
        | "spawn_agent" | "list_tasks" | "read_task" | "wait_task");
    if internal || domain == "none" || domain == "read" { None } else { Some(domain) }
}

#[cfg(test)]
#[test]
fn same_conversation_recall_agrees_with_native_catalog_without_granting_memory() {
    assert_eq!(crate::tool_catalog::builtin_tool_metadata("conversation_recall").unwrap().capability, "none");
    assert_eq!(permission_name_for_tool("conversation_recall"), "none");
    assert!(tool_permission_domain("conversation_recall").is_none());
    assert_eq!(tool_permission_domain("memory_search").as_deref(), Some("memory_search"));
}

#[cfg(test)]
#[test]
fn permission_contract_preserves_sensitive_and_unknown_domains() {
    for name in ["delete_file", "workspace_checkpoint", "notebook_edit", "git_commit", "unknown_tool", "bash", "memory_save", "memory_search"] {
        assert!(tool_permission_domain(name).is_some(), "{name}");
    }
    for name in ["read_file", "Skill", "SOP", "bash_background_status", "conversation_recall"] {
        assert!(tool_permission_domain(name).is_none(), "{name}");
    }
}

#[cfg(test)]
#[test]
fn private_builtin_catalog_domains_reach_the_default_permission_challenge() {
    use super::permissions::{PermissionPolicyDecision, PermissionRule, PermissionRuleSet,
        PermissionRuleSource, PermissionSubject};
    let rules = PermissionRuleSet { rules: vec![PermissionRule {
        capability_pattern: "*".into(), resource_pattern: "*".into(),
        decision: PermissionPolicyDecision::Allow, source: PermissionRuleSource::ProductDefault,
    }] };
    for tool in ["memory_save", "memory_search", "search_contacts",
        "search_calendar_events", "computer_get_clipboard", "get_location", "bash"] {
        let domain = tool_permission_domain(tool).expect("sensitive tools must not bypass the prompt");
        assert_eq!(rules.evaluate(&PermissionSubject { capability: domain, resource: "specific-resource".into() }),
            PermissionPolicyDecision::Ask, "{tool}");
    }
}

pub fn permission_name_for_tool(tool_name: &str) -> String {
    let name = normalize_tool_name(tool_name);
    // This tool can only read public text in the executing conversation. Its
    // schema cannot select another session; the store revalidates part ownership.
    // Match the native catalog's "none" capability, not private memory access.
    if name == "conversation_recall" {
        return "none".into();
    }
    if matches!(name.as_str(), "workspace_access" | "memory_search" | "memory_save"
        | "memory_update" | "memory_delete") {
        // Cross-conversation/private memory is not ordinary workspace read.
        // Preserve its identity before broad file-read capability mapping.
        return name;
    }
    // These built-ins have argument-sensitive irreversible modes. Collapsing
    // them into the generic `edit` domain discards the operation identity
    // before Core can apply the recommended confirmation policy.
    if matches!(
        name.as_str(),
        "delete_file" | "workspace_checkpoint" | "notebook_edit"
    ) {
        return name;
    }
    // Status-only host introspection is part of capability discovery. It does
    // not control the computer and must not inherit the `computer_*` approval
    // domain merely because of its name prefix.
    if name == "computer_status" {
        return "none".to_string();
    }
    if matches!(
        name.as_str(),
        "bash"
            | "bash_background_start"
            | "bash_background_status"
            | "bash_background_stop"
            | "terminal_input"
            | "language_service_query"
            | "bash_background_wait"
            | "bash_command_review"
    ) {
        return "bash".to_string();
    }
    if is_browser_tool(&name) {
        return "browser".to_string();
    }
    match capability_for_tool_name(&name) {
        HostCapability::FilesRead => "read",
        HostCapability::FilesWrite => "edit",
        HostCapability::Process => "bash",
        HostCapability::Network => "web",
        HostCapability::Background => "background",
        HostCapability::Camera => "camera",
        HostCapability::Microphone => "microphone",
        HostCapability::Photos => "photos",
        HostCapability::Location => "location",
        HostCapability::Calendar => "calendar",
        HostCapability::Reminders => "reminders",
        HostCapability::Contacts => "contacts",
        HostCapability::Health => "health",
        HostCapability::Notifications => "notifications",
        HostCapability::Clipboard => "clipboard",
        HostCapability::Computer => "computer",
        HostCapability::None => name.as_str(),
    }
    .to_string()
}

pub fn default_compaction_threshold_for_window(tokens: usize) -> usize {
    tokens
        .saturating_mul(60)
        .checked_div(100)
        .unwrap_or(0)
        .min(DEFAULT_ACTIVE_CONTEXT_TOKEN_CEILING)
}

pub fn prompt_token_limit_for_window(
    context_window_tokens: usize,
    reserved_output_tokens: usize,
) -> usize {
    let safety_margin = (context_window_tokens / 20).clamp(512, 8_192);
    context_window_tokens
        .saturating_sub(reserved_output_tokens)
        .saturating_sub(safety_margin)
        .max(512)
}

pub fn reserved_output_tokens_for_window(
    context_window_tokens: Option<usize>,
    configured_output_tokens: usize,
) -> usize {
    let configured_output_tokens = configured_output_tokens.max(256);
    let Some(window) = context_window_tokens else {
        return configured_output_tokens;
    };
    let absolute_room = window.saturating_sub(512).max(256);
    // Reserve the same admitted allowance used on the wire and by compaction.
    // A fractional reservation lets ordinary requests consume the space needed
    // for an unchanged-prefix summary, forcing a cold restart at compaction.
    configured_output_tokens.min(absolute_room)
}

/// Heuristic used only for context reservation. This must never rewrite an
/// admitted provider output limit; generation admission is owned by Domain.
pub fn context_output_reservation_tokens(
    declared_output_tokens: Option<usize>,
    context_window_tokens: Option<usize>,
) -> usize {
    let requested = declared_output_tokens
        .filter(|tokens| *tokens > 0)
        .unwrap_or(DEFAULT_MODEL_OUTPUT_TOKENS);
    reserved_output_tokens_for_window(context_window_tokens, requested)
}

fn normalize_tool_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn is_read_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "git_worktree_list"
            | "language_service_list"
            | "read_many_files"
            | "notebook_read"
            | "file_tree"
            | "file_outline"
            | "list_dir"
            | "find_files"
            | "find_up"
            | "grep"
            | "glob"
            | "repo_map"
            | "code_search"
            | "read_symbol"
            | "symbol_search"
            | "implementation_search"
            | "workspace_symbols"
            | "workspace_status"
            | "read_notebook"
            | "agent_config_context"
            | "agent_context"
            | "call_graph"
            | "change_review"
            | "changed_symbols"
            | "ci_context"
            | "code_diagnostics"
            | "code_navigation"
            | "coding_conventions"
            | "command_context"
            | "failure_context"
            | "failure_triage"
            | "file_dependencies"
            | "project_overview"
            | "environment_probe"
            | "instruction_context"
            | "skill_context"
            | "skill_invoke"
            | "Skill"
            | "skill_skip"
            | "context_pack"
            | "focused_context"
            | "impact_analysis"
            | "validation_plan"
            | "test_context"
            | "format_plan"
            | "related_files"
            | "tool_output_read"
            | "tool_output_find"
            | "tool_output_list"
            | "tool_output_search"
            | "diff_review"
            | "verification_review"
            | "handoff_context"
            | "git_diff"
            | "git_history"
            | "git_blame"
            | "git_show"
            | "memory_search"
            | "document_search"
    )
}

fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "git_commit" | "write_file"
            | "append_file"
            | "edit_file"
            | "edit_file_lines"
            | "apply_patch"
            | "delete_file"
            | "move_file"
            | "create_directory"
            | "edit_notebook"
            | "notebook_edit"
            | "format_files"
            | "replace_symbol_body"
            | "multi_edit_file"
            | "replace_in_files"
            | "rename_symbol"
    )
}

fn is_process_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "shell" | "run_command" | "exec" | "bash_command_review" | "language_service_query" | "git_worktree_create" | "git_worktree_integrate"
    )
}

fn is_background_tool(name: &str) -> bool {
    matches!(
        name,
        "nola_orchestrate"
            | "background"
            | "background_start"
            | "background_status"
            | "background_stop"
            | "background_wait"
            | "bash_background_start"
            | "bash_background_status"
            | "bash_background_stop"
            | "terminal_input"
            | "bash_background_wait"
            | "spawn_agent"
            | "list_tasks"
            | "read_task"
            | "wait_task"
            | "stop_task"
    )
}

fn is_network_tool(name: &str) -> bool {
    matches!(
        name,
        "web" | "web_search" | "web_fetch" | "fetch_url" | "arxiv_search"
    ) || is_browser_tool(name)
}

fn is_browser_tool(name: &str) -> bool {
    if name.starts_with("browser_") {
        return true;
    }
    matches!(
        name,
        "chrome_native_accessibility_snapshot"
            | "chrome_native_assert"
            | "chrome_native_click"
            | "chrome_native_console_logs"
            | "chrome_native_dom_snapshot"
            | "chrome_native_drag"
            | "chrome_native_hover"
            | "chrome_native_inspect"
            | "chrome_native_list_pages"
            | "chrome_native_network_events"
            | "chrome_native_open_page"
            | "chrome_native_press_key"
            | "chrome_native_screenshot"
            | "chrome_native_scroll"
            | "chrome_native_select_option"
            | "chrome_native_set_checked"
            | "chrome_native_status"
            | "chrome_native_storage_state"
            | "chrome_native_switch_page"
            | "chrome_native_type_text"
            | "chrome_native_wait_for"
    )
}

fn is_computer_tool(name: &str) -> bool {
    matches!(
        name,
        "computer_accessibility_snapshot"
            | "computer_activate_app"
            | "computer_assert_app"
            | "computer_assert_element"
            | "computer_assert_text"
            | "computer_assert_window"
            | "computer_click"
            | "computer_click_element"
            | "computer_click_text"
            | "computer_close_window"
            | "computer_drag"
            | "computer_drag_element"
            | "computer_element_at"
            | "computer_find_text"
            | "computer_focus_element"
            | "computer_focus_window"
            | "computer_get_position"
            | "computer_hotkey"
            | "computer_key_down"
            | "computer_key_up"
            | "computer_list_apps"
            | "computer_list_displays"
            | "computer_list_menus"
            | "computer_list_windows"
            | "computer_mouse_down"
            | "computer_mouse_up"
            | "computer_move_mouse"
            | "computer_move_to_element"
            | "computer_move_to_text"
            | "computer_observe"
            | "computer_open_item"
            | "computer_perform_element_action"
            | "computer_perform_window_action"
            | "computer_press_at"
            | "computer_press_element"
            | "computer_press_key"
            | "computer_quit_app"
            | "computer_read_text"
            | "computer_screenshot"
            | "computer_screenshot_element"
            | "computer_screenshot_window"
            | "computer_scroll"
            | "computer_scroll_at"
            | "computer_scroll_element"
            | "computer_select_menu"
            | "computer_select_text"
            | "computer_set_element_value"
            | "computer_set_window_bounds"
            | "computer_type_at"
            | "computer_type_element"
            | "computer_type_text"
            | "computer_wait"
            | "computer_wait_for_app"
            | "computer_wait_for_element"
            | "computer_wait_for_text"
            | "computer_wait_for_window"
    )
}
