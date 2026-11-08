// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Maps structured assistant state and tool activity into localized UI presentation.

import Foundation

enum MacAgentActivityPresentationPolicy {
    struct Projection: Equatable {
        let title: String
        let detail: String
    }

    static func project(title: String, detail: String) -> Projection {
        // Visibility and lifecycle semantics come from the structured Core
        // event. Presentation must never infer protocol meaning from localized
        // or misspelled status text.
        Projection(title: title, detail: detail)
    }
}

enum MacToolPresentationPolicy {
    /// Localize presentation only: canonical IDs and declared extension titles
    /// remain untouched in the journal and permission contract.
    static func displayTitle(for trace: MacAssistantTrace) -> String {
        if trace.tool != nil {
            let title = trace.title.trimmingCharacters(in: .whitespacesAndNewlines)
            if !title.isEmpty, title != "Tool", title != trace.capabilityID {
                return displayTitle(for: title)
            }
            return displayTitle(for: trace.capabilityID, arguments: trace.arguments)
        }
        return MacToolTraceDisplayPolicy.operationTitle(for: trace)
            ?? displayTitle(for: trace.title, arguments: trace.arguments)
    }

    static func displayTitle(
        for rawTitle: String,
        arguments: String? = nil
    ) -> String {
        let canonicalTitle = rawTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        if let skillID = MacBuiltInSkillID(rawValue: canonicalTitle),
           let descriptor = MacBuiltInSkillRegistry.descriptors.first(where: { $0.id == skillID }) {
            return descriptor.title
        }
        let normalized = rawTitle
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
            .lowercased()
        let target = primaryTarget(in: arguments, toolName: normalized)
        let presentation: (key: String, fallback: String, includesTarget: Bool)?
        switch normalized {
        case "workspace_access":
            presentation = ("chat.host_tool.name.workspace_access", "Authorize folder access", false)
        case "bash", "shell", "run_command", "execute_command", "bash_command_review",
             "bash_background_status", "bash_background_wait", "bash_background_stop", "terminal_input":
            presentation = ("chat.host_tool.name.command", "Run command", true)
        case "read_file", "read_text_file", "read_many_files", "notebook_read", "file_outline", "read_symbol":
            presentation = ("chat.host_tool.name.read_file", "Read file", true)
        case "write_file", "create_file", "append_file":
            presentation = ("chat.host_tool.name.write_file", "Write file", true)
        case "edit_file", "edit_file_lines", "multi_edit_file", "notebook_edit", "apply_patch", "replace_text",
             "replace_symbol_body", "replace_in_files", "rename_symbol", "format_files", "move_file":
            presentation = ("chat.host_tool.name.edit_file", "Edit file", true)
        case "delete_file", "remove_file":
            presentation = ("chat.host_tool.name.delete_file", "Delete file", true)
        case "list_dir", "list_directory", "file_tree", "find_files", "find_up":
            presentation = ("chat.host_tool.name.list_folder", "List folder", true)
        case "project_overview", "repo_map":
            presentation = ("chat.host_tool.name.project_overview", "Inspect project overview", true)
        case "codebase_search", "codebase_text_search", "codebase_trace", "codebase_read_symbol":
            presentation = ("chat.host_tool.name.search_files", "Search files", true)
        case "codebase_index", "codebase_index_status", "codebase_projects", "codebase_architecture",
             "codebase_query", "codebase_schema", "codebase_check_coverage", "codebase_changed_impact",
             "codebase_delete_index", "codebase_adr", "codebase_ingest_traces":
            presentation = ("chat.host_tool.name.workspace", "Inspect workspace", true)
        case "grep", "glob", "search_files", "ripgrep", "code_search", "workspace_symbols", "symbol_search",
             "implementation_search", "code_navigation", "language_service_query", "language_service_list", "call_graph", "failure_context", "failure_triage",
             "file_dependencies", "related_files", "document_search":
            presentation = ("chat.host_tool.name.search_files", "Search files", true)
        case "workspace_status", "code_diagnostics", "coding_conventions", "instruction_context", "skill_context",
             "agent_context", "command_context", "agent_config_context", "ci_context", "context_pack", "focused_context",
             "impact_analysis", "validation_plan", "test_context", "format_plan",
             "verification_review", "handoff_context", "workspace_checkpoint":
            presentation = ("chat.host_tool.name.workspace", "Inspect workspace", false)
        case "git_diff", "change_review", "diff_review", "changed_symbols", "git_history", "git_blame", "git_show":
            presentation = ("chat.host_tool.name.changes", "Inspect changes", false)
        case "web_search":
            presentation = ("chat.host_tool.name.web_search", "Search web", true)
        case "web_fetch", "fetch_url":
            presentation = ("chat.host_tool.name.web_read", "Read webpage", true)
        case "tool_search":
            presentation = ("chat.host_tool.name.search_tools", "Search tools", false)
        case "tool_output_list", "tool_output_read", "tool_output_search", "tool_output_find":
            presentation = ("chat.host_tool.name.read_output", "Read tool output", false)
        case "skill":
            presentation = ("chat.host_tool.name.skill", "Use skill", false)
        case "sop", "sop_invoke", "sop_run_update":
            presentation = ("chat.host_tool.name.sop", "Run procedure", false)
        case "memory_save", "memory_search":
            presentation = ("chat.host_tool.name.memory", "Manage memory", false)
        case "search_calendar_events":
            presentation = ("chat.host_tool.name.search_calendar", "Search calendar", false)
        case "create_calendar_event":
            presentation = ("chat.host_tool.name.create_calendar_event", "Create calendar event", false)
        case "search_reminders":
            presentation = ("chat.host_tool.name.search_reminders", "Search reminders", false)
        case "create_reminder":
            presentation = ("chat.host_tool.name.create_reminder", "Create reminder", false)
        case "update_reminder":
            presentation = ("chat.host_tool.name.update_reminder", "Update reminder", false)
        case "spawn_agent", "nola_orchestrate":
            presentation = ("chat.host_tool.name.subtask", "Run subtask", false)
        case "list_tasks", "read_task", "wait_task", "stop_task":
            presentation = ("chat.host_tool.name.background", "Background task", false)
        case "propose_agent_plan", "update_agent_todos", "agent_plan_file":
            presentation = ("chat.host_tool.name.plan", "Update plan", false)
        case "system_time":
            presentation = ("chat.host_tool.name.time", "Read system time", false)
        case "chrome_native_status":
            presentation = ("chat.host_tool.name.browser_status", "Check browser connection", false)
        case "chrome_native_list_pages":
            presentation = ("chat.host_tool.name.browser_tabs", "List browser tabs", false)
        case "chrome_native_switch_page":
            presentation = ("chat.host_tool.name.browser_switch", "Switch browser tab", true)
        case "chrome_native_open_page":
            presentation = ("chat.host_tool.name.browser_open", "Open webpage", true)
        case "chrome_native_inspect", "chrome_native_dom_snapshot", "chrome_native_accessibility_snapshot":
            presentation = ("chat.host_tool.name.browser_inspect", "Inspect webpage", true)
        case "chrome_native_screenshot":
            presentation = ("chat.host_tool.name.browser_capture", "Capture webpage", true)
        case "chrome_native_click":
            presentation = ("chat.host_tool.name.browser_click", "Click webpage", true)
        case "chrome_native_hover":
            presentation = ("chat.host_tool.name.browser_hover", "Hover on webpage", true)
        case "chrome_native_drag":
            presentation = ("chat.host_tool.name.browser_drag", "Drag on webpage", true)
        case "chrome_native_type_text":
            presentation = ("chat.host_tool.name.browser_type", "Type on webpage", true)
        case "chrome_native_select_option", "chrome_native_set_checked":
            presentation = ("chat.host_tool.name.browser_form", "Update webpage form", true)
        case "chrome_native_press_key", "chrome_native_scroll":
            presentation = ("chat.host_tool.name.browser_navigate", "Navigate webpage", true)
        case "chrome_native_assert", "chrome_native_wait_for":
            presentation = ("chat.host_tool.name.browser_verify", "Verify webpage", true)
        case "chrome_native_console_logs", "chrome_native_network_events", "chrome_native_storage_state":
            presentation = ("chat.host_tool.name.browser_diagnostics", "Read browser diagnostics", true)
        default:
            if normalized.hasPrefix("chrome_") || normalized.hasPrefix("browser_") {
                presentation = ("chat.host_tool.name.browser", "Use browser", true)
            } else if normalized.hasPrefix("computer_") {
                presentation = ("chat.host_tool.name.computer", "Control computer", false)
            } else if normalized.hasPrefix("skill_") {
                presentation = ("chat.host_tool.name.skill", "Use skill", false)
            } else if normalized.hasPrefix("sop_") {
                presentation = ("chat.host_tool.name.sop", "Run procedure", false)
            } else if normalized.hasPrefix("mcp_") {
                presentation = ("chat.host_tool.name.mcp", "Call MCP", false)
            } else if normalized.hasPrefix("background_") {
                presentation = ("chat.host_tool.name.background", "Background task", false)
            } else {
                presentation = nil
            }
        }
        guard let presentation else {
            let trimmed = rawTitle.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else {
                return NSLocalizedString("chat.host_tool.name.generic", value: "Tool", comment: "Generic tool display name")
            }
            // Host traces may already contain a localized presentation title.
            // Preserve it instead of producing “Unknown tool · Read file”.
            return trimmed
        }
        let title = NSLocalizedString(
            presentation.key,
            value: presentation.fallback,
            comment: "Localized Agent Host tool title"
        )
        return presentation.includesTarget ? actionTitle(title, target: target) : title
    }

    private static func actionTitle(_ action: String, target: String?) -> String {
        guard let target, !target.isEmpty else { return action }
        let value = target.count > 72
            ? String(target.prefix(71)) + "…"
            : target
        return "\(action) \(value)"
    }

    private static func primaryTarget(in arguments: String?, toolName: String) -> String? {
        guard let arguments,
              let data = arguments.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data)
                as? [String: Any] else {
            return nil
        }
        if ["bash", "shell", "run_command", "execute_command", "bash_command_review"]
            .contains(toolName),
           let command = object["command"] as? String {
            return conciseCommandTarget(command)
        }
        for key in [
            "query", "pattern", "url", "path",
            "target_path", "root", "cwd", "directory",
        ] {
            guard let value = object[key] as? String else { continue }
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }
            if key.contains("path") || ["root", "cwd", "directory"].contains(key) {
                return URL(fileURLWithPath: trimmed).lastPathComponent
            }
            return trimmed.split(whereSeparator: \.isNewline).first.map(String.init)
        }
        return nil
    }

    private static func conciseCommandTarget(_ command: String) -> String? {
        let tokens = command
            .split(whereSeparator: \.isWhitespace)
            .map { token in
                String(token).trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            }
            .filter { !$0.isEmpty }
        guard !tokens.isEmpty else { return nil }
        if let outputIndex = tokens.firstIndex(of: "--output"),
           tokens.indices.contains(outputIndex + 1) {
            return URL(fileURLWithPath: tokens[outputIndex + 1]).lastPathComponent
        }
        if let script = tokens.first(where: { $0.hasSuffix(".py") || $0.hasSuffix(".sh") }) {
            return URL(fileURLWithPath: script).lastPathComponent
        }
        return URL(fileURLWithPath: tokens[0]).lastPathComponent
    }
}

enum MacThinkingPresentationPolicy {
    static func summary(
        at index: Int,
        in traces: [MacAssistantTrace]
    ) -> String {
        let hasPreviousTool = traces[..<index].contains { $0.kind == .tool }
        let hasNextTool = traces[(index + 1)...].contains { $0.kind == .tool }
        if hasPreviousTool, hasNextTool {
            return String(localized: "Reviewing the tool result")
        }
        if hasPreviousTool {
            return String(localized: "Preparing the answer")
        }
        return String(localized: "Planning the next step")
    }
}

enum MacAssistantStatusVisibilityPolicy {
    static func shouldShow(
        hasTraces: Bool,
        isStreaming: Bool,
        hasCommittedText: Bool,
        hasLiveActivities: Bool
    ) -> Bool {
        _ = hasCommittedText
        return hasTraces || (isStreaming && hasLiveActivities)
    }
}
