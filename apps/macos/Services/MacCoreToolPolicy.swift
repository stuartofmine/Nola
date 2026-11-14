// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Creates identity-bound Core v2 tool plans for this local macOS Host.
// - Remote catalogs are issued by the execution Host through MacRemoteHostService.
// - Security: Every permission approval revalidates plan freshness and manifest identity.

import ApplicationServices
import CoreGraphics
import Foundation

#if canImport(NolaCore)
import NolaCore
#endif

nonisolated indirect enum MacCoreJSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: MacCoreJSONValue])
    case array([MacCoreJSONValue])
    case null

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let item = try? value.decode(Bool.self) { self = .bool(item) }
        else if let item = try? value.decode(Double.self) { self = .number(item) }
        else if let item = try? value.decode(String.self) { self = .string(item) }
        else if let item = try? value.decode([String: MacCoreJSONValue].self) { self = .object(item) }
        else { self = .array(try value.decode([MacCoreJSONValue].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .string(let item): try value.encode(item)
        case .number(let item): try value.encode(item)
        case .bool(let item): try value.encode(item)
        case .object(let item): try value.encode(item)
        case .array(let item): try value.encode(item)
        case .null: try value.encodeNil()
        }
    }
}

nonisolated struct MacCoreToolIdentity: Codable, Equatable, Sendable {
    let hostId: String
    let clientId: String
    let sessionId: String

    enum CodingKeys: String, CodingKey {
        case hostId = "host_id"
        case clientId = "client_id"
        case sessionId = "session_id"
    }
}

nonisolated struct MacCoreToolManifestEntry: Codable, Equatable, Sendable {
    let toolId: String
    let handlerId: String
    let schema: MacCoreJSONValue
    let route = "agent_host"
    let available: Bool

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case handlerId = "handler_id"
        case schema, route, available
    }
}

nonisolated struct MacCoreToolRuntimeManifest: Codable, Equatable, Sendable {
    let manifestId: String
    let manifestVersion: Int64
    let issuedAtMs: Int64
    let expiresAtMs: Int64
    let hostId: String
    let clientId: String
    let sessionId: String
    let routes: [String]
    let capabilities: [String]
    let authorizations: [String: String]
    let resultEgressReceipts: [MacCoreRuntimeResultEgressReceipt] = []
    let tools: [MacCoreToolManifestEntry]

    enum CodingKeys: String, CodingKey {
        case manifestId = "manifest_id"
        case manifestVersion = "manifest_version"
        case issuedAtMs = "issued_at_ms"
        case expiresAtMs = "expires_at_ms"
        case hostId = "host_id"
        case clientId = "client_id"
        case sessionId = "session_id"
        case routes, capabilities, authorizations, tools
        case resultEgressReceipts = "result_egress_receipts"
    }
}

nonisolated struct MacCoreRuntimeResultEgressReceipt: Codable, Equatable, Sendable {
    let toolId: String
    let route: String
    let manifestId: String
    let hostId: String
    let clientId: String
    let sessionId: String
    let issuedAtMs: Int64
    let expiresAtMs: Int64
    let allowResultEgress: Bool

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case route
        case manifestId = "manifest_id"
        case hostId = "host_id"
        case clientId = "client_id"
        case sessionId = "session_id"
        case issuedAtMs = "issued_at_ms"
        case expiresAtMs = "expires_at_ms"
        case allowResultEgress = "allow_result_egress"
    }
}

nonisolated struct MacCoreToolPolicySelection: Codable, Equatable, Sendable {
    let mode: String
    let allowedTools: [String]
    let deniedTools: [String]

    init(
        mode: String = "agent",
        allowedTools: [String] = [],
        deniedTools: [String] = []
    ) {
        self.mode = mode
        self.allowedTools = allowedTools
        self.deniedTools = deniedTools
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case allowedTools = "allow_tools"
        case deniedTools = "deny_tools"
    }
}

nonisolated struct MacCoreToolUserPolicy: Codable, Equatable, Sendable {
    let allowedTools: [String] = []
    let deniedTools: [String] = []
    let allowedCapabilities: [String] = []
    let deniedCapabilities: [String] = []
    let maxRisk = "high"
    let allowResultEgressRoutes: [String] = []

    enum CodingKeys: String, CodingKey {
        case allowedTools = "allow_tools"
        case deniedTools = "deny_tools"
        case allowedCapabilities = "allow_capabilities"
        case deniedCapabilities = "deny_capabilities"
        case maxRisk = "max_risk"
        case allowResultEgressRoutes = "allow_result_egress_routes"
    }
}

nonisolated struct MacCoreScopedGrant: Codable, Equatable, Sendable {
    let capability: String
    let route = "agent_host"
    let scope: String
    let expiresAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case capability, route, scope
        case expiresAtMs = "expires_at_ms"
    }
}

nonisolated struct MacCoreToolPolicyInput: Codable, Equatable, Sendable {
    let schemaVersion = 1
    let platform = "macos"
    let nowMs: Int64
    let expectedIdentity: MacCoreToolIdentity
    let runtime: MacCoreToolRuntimeManifest
    let conversation: MacCoreToolPolicySelection
    let userPolicy: MacCoreToolUserPolicy
    let scopedGrants: [MacCoreScopedGrant]
    let resultEgressGrants: [MacCoreResultEgressGrant] = []

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case platform
        case nowMs = "now_ms"
        case expectedIdentity = "expected_identity"
        case runtime, conversation
        case userPolicy = "user_policy"
        case scopedGrants = "scoped_grants"
        case resultEgressGrants = "result_egress_grants"
    }
}

nonisolated struct MacCoreResultEgressGrant: Codable, Equatable, Sendable {
    let toolId: String
    let route: String
    let manifestId: String
    let hostId: String
    let clientId: String
    let sessionId: String
    let expiresAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case route
        case manifestId = "manifest_id"
        case hostId = "host_id"
        case clientId = "client_id"
        case sessionId = "session_id"
        case expiresAtMs = "expires_at_ms"
    }
}

nonisolated struct MacCoreResolvedTool: Codable, Equatable, Sendable {
    let toolId: String
    let name: String
    let enabled: Bool
    let route: String
    let capability: String
    let permission: String
    let authorization: String
    let risk: String
    let requiresResultEgress: Bool
    let reason: String
    let schema: MacCoreJSONValue

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case name, enabled, route, capability, permission, authorization, risk, reason, schema
        case requiresResultEgress = "requires_result_egress"
    }
}

nonisolated struct MacCoreToolPolicyOutput: Codable, Equatable, Sendable {
    let schemaVersion: Int
    let platform: String
    let policyDigest: String
    let manifestDigest: String
    let manifestValidUntilMs: Int64
    let enabledTools: [MacCoreResolvedTool]
    let disabledTools: [MacCoreResolvedTool]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case platform
        case policyDigest = "policy_digest"
        case manifestDigest = "manifest_digest"
        case manifestValidUntilMs = "manifest_valid_until_ms"
        case enabledTools = "enabled_tools"
        case disabledTools = "disabled_tools"
    }
}

nonisolated struct MacCoreToolPlanReceipt: Equatable, Sendable {
    let output: MacCoreToolPolicyOutput
    let identity: MacCoreToolIdentity
    let sourceManifestId: String
    let policyInput: MacCoreToolPolicyInput

    func allowsMCP(_ descriptor: Data?, toolID: String) -> Bool {
        guard let descriptor,
              let policy = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(policyInput)),
              let value = try? JSONSerialization.jsonObject(with: descriptor) as? [String: Any],
              value["id"] as? String == toolID,
              let data = try? JSONSerialization.data(withJSONObject: ["policy": policy, "descriptor": value]) else { return false }
        #if canImport(NolaCore)
        return data.withUnsafeBytes { nola_core_tool_policy_mcp_allowed($0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
        #else
        return false
        #endif
    }

    func resolvedPermissionTool(
        toolId: String,
        hostId: String,
        clientId: String,
        sessionId: String,
        policyDigest: String? = nil,
        manifestDigest: String? = nil,
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) -> MacCoreResolvedTool? {
        // A static permission domain of "none" does not waive runtime resource
        // approval (for example, reading other conversations). Authenticate the
        // pending Core challenge; Core still owns its resource/operation grant.
        guard nowMs < output.manifestValidUntilMs,
              identity.hostId == hostId,
              identity.clientId == clientId,
              identity.sessionId == sessionId,
              policyDigest == nil || policyDigest == output.policyDigest,
              manifestDigest == nil || manifestDigest == output.manifestDigest,
              let tool = output.enabledTools.first(where: { $0.toolId == toolId && $0.enabled }),
              tool.route == "agent_host",
              tool.authorization != "denied",
              tool.authorization != "unavailable",
              tool.permission != "unsupported" else { return nil }
        return tool
    }

    func matchesPermissionChallenge(
        toolId: String,
        hostId: String,
        clientId: String,
        sessionId: String,
        policyDigest: String?,
        manifestDigest: String?,
        mcpDescriptorData: Data? = nil
    ) -> Bool {
        identity.hostId == hostId
            && identity.clientId == clientId
            && identity.sessionId == sessionId
            && policyDigest == output.policyDigest
            && manifestDigest == output.manifestDigest
            && (output.enabledTools.contains {
                $0.toolId == toolId
                    && $0.enabled
                    && $0.route == "agent_host"
                    && $0.authorization != "denied"
                    && $0.authorization != "unavailable"
                    && $0.permission != "unsupported"
            } || allowsMCP(mcpDescriptorData, toolID: toolId))
    }

    var receiptObject: [String: Any]? {
        guard let data = try? JSONEncoder().encode(output),
              var outputObject = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let identityData = try? JSONEncoder().encode(identity),
              let identityObject = try? JSONSerialization.jsonObject(with: identityData) else { return nil }
        outputObject["expected_identity"] = identityObject
        outputObject["source_manifest_id"] = sourceManifestId
        outputObject["policy_input"] = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(policyInput))
        return outputObject
    }

    var receiptBase64: String? {
        guard let object = receiptObject,
              let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return nil }
        return data.base64EncodedString()
    }
}

enum MacCoreToolPolicyError: LocalizedError {
    case unavailable
    case incompatibleABI(UInt32)
    case invalidManifest(String)
    case invalidResponse
    case stalePlan

    var errorDescription: String? {
        switch self {
        case .unavailable:
            return String(localized: "The shared Core tool-policy runtime is unavailable.")
        case .incompatibleABI(let value):
            return String(
                format: String(localized: "Unsupported Core tool-policy ABI version: %u."),
                value
            )
        case .invalidManifest(let reason): return reason
        case .invalidResponse:
            return String(localized: "Core returned an invalid tool plan.")
        case .stalePlan:
            return String(localized: "The host tool plan expired or its manifest changed.")
        }
    }
}

nonisolated enum MacCoreToolPolicy {
    // Executed by the bound Core publisher, outside the generic file/tool registry.
    private static let coreOwnedTools = ["render_ui"]
    // Permission-receipt partitions, not a second execution registry. The Rust
    // platform_catalog_contract_tests compares their union to tools::specs(),
    // rejecting missing, duplicate and unregistered handlers.
    private static let localFileAndProcessTools: [String] = [
        "agent_config_context", "agent_context", "agent_plan_file", "agent_task_status",
        "append_file", "apply_patch", "bash", "bash_background_status",
        "bash_background_stop", "bash_background_wait", "bash_command_review", "call_graph",
        "change_review", "changed_symbols", "ci_context", "code_diagnostics", "code_navigation", "conversation_recall",
        "code_search", "coding_conventions", "command_context", "context_pack",
        "delete_file", "diff_review", "document_search", "edit_file", "edit_file_lines",
        "environment_probe", "failure_context", "failure_triage", "file_dependencies", "file_outline", "file_tree",
        "find_files", "find_up", "focused_context", "format_files", "format_plan", "git_blame",
        "git_commit", "git_diff", "git_history", "git_show", "git_worktree_list", "git_worktree_create", "git_worktree_integrate", "glob", "goal_complete", "grep", "handoff_context",
        "impact_analysis", "implementation_search", "instruction_context",
        "language_service_list", "language_service_query", "terminal_input",
        "list_dir", "list_tasks", "memory_save", "memory_search", "move_file", "multi_edit_file", "nola_orchestrate", "notebook_edit",
        "notebook_read", "project_overview", "propose_agent_plan", "read_file", "read_many_files",
        "read_symbol", "read_task", "related_files", "rename_symbol", "replace_in_files", "replace_symbol_body",
        "repo_map", "request_user_input", "send_agent_message", "set_thinking_mode", "Skill", "skill_skip", "skill_update",
        "SOP", "sop_run_update", "spawn_agent", "delegate_expert", "stop_task", "symbol_search", "system_time",
        "test_context", "tool_output_find", "tool_output_list", "tool_output_read", "tool_output_search",
        "tool_search", "update_agent_todos", "validation_plan", "verification_review", "wait_task", "workspace_checkpoint",
        "workspace_access", "workspace_status", "workspace_symbols", "write_file",
    ]
    private static let webTools = ["web_search", "web_fetch"]
    private static let macProductivityTools = [
        "search_calendar_events", "create_calendar_event",
        "search_reminders", "create_reminder", "update_reminder",
    ]
    private static let browserActions = [
        "accessibility_snapshot", "assert", "click", "console_logs", "dom_snapshot", "drag", "hover",
        "inspect", "list_pages", "network_events", "open_page", "press_key", "screenshot", "scroll",
        "select_option", "set_checked", "status", "storage_state", "switch_page", "type_text", "wait_for",
    ]
    private static let unifiedBrowserTools = [
        "browser_status", "browser_start", "browser_stop", "browser_snapshot", "browser_navigate",
        "browser_click", "browser_type_text", "browser_screenshot", "browser_tabs", "browser_switch_page",
    ]
    private static let computerActions = [
        "accessibility_snapshot", "activate_app", "assert_app", "assert_clipboard", "assert_element", "assert_text",
        "assert_window", "click", "click_element", "click_text", "close_window", "copy_selection", "drag",
        "drag_element", "element_at", "find_text", "focus_element", "focus_window", "get_clipboard", "get_position",
        "hotkey", "key_down", "key_up", "list_apps", "list_displays", "list_menus", "list_windows", "mouse_down",
        "mouse_up", "move_mouse", "move_to_element", "move_to_text", "observe", "open_item",
        "perform_element_action", "perform_window_action", "press_at", "press_element", "press_key", "quit_app",
        "read_text", "screenshot", "screenshot_element", "screenshot_window", "scroll", "scroll_at", "scroll_element",
        "select_menu", "select_text", "set_clipboard", "set_element_value", "set_window_bounds", "status", "type_at",
        "type_element", "type_text", "wait", "wait_for_app", "wait_for_clipboard", "wait_for_element", "wait_for_text",
        "wait_for_window",
    ]

    static func localPlan(
        executableURL: URL,
        projectURL: URL,
        sessionId: String,
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) throws -> MacCoreToolPlanReceipt {
        guard FileManager.default.isExecutableFile(atPath: executableURL.path) else {
            throw MacCoreToolPolicyError.invalidManifest(
                String(localized: "The local agent host is not executable.")
            )
        }
        let identity = MacCoreToolIdentity(hostId: "local", clientId: "mac", sessionId: sessionId)
        let fileGrantExpiry = nowMs + 300_000
        let tools = Set(
            localFileAndProcessTools
                + coreOwnedTools
                + webTools
                + macProductivityTools
                + unifiedBrowserTools
                + browserActions.filter { $0 != "upload_file" }.map { "chrome_native_\($0)" }
                + computerActions.map { "computer_\($0)" }
        ).sorted()
        // macOS TCC state is deliberately not frozen into this signed receipt.
        // Accessibility and Screen Recording can change while an Agent Host
        // turn is suspended in System Settings. The native Mac bridge checks
        // the current OS capability immediately before each operation instead.
        let localAuthorizations = localHostAuthorizations()
        return try resolve(
            identity: identity,
            manifestId: "mac-local:agent-host-v2",
            manifestVersion: 2,
            nowMs: nowMs,
            capabilities: [
                "files_read", "files_write", "process", "network", "background", "computer", "clipboard",
                "calendar", "reminders",
            ],
            authorizations: localAuthorizations,
            toolIds: tools,
            scopedGrants: [
                .init(capability: "files_read", scope: projectURL.standardizedFileURL.path, expiresAtMs: fileGrantExpiry),
            ]
        )
    }

    static func localHostAuthorizations() -> [String: String] {
        [
            // This map reports whether the native Host can provide a
            // capability. Agent Core separately decides whether a concrete
            // invocation needs confirmation.
            "files_read": "granted",
            "files_write": "granted",
            "process": "granted",
            "network": "granted",
            "background": "granted",
            // This authorizes routing to the in-app native bridge. It does not
            // bypass macOS TCC; concrete computer tools still fail closed when
            // the running Nola app lacks the relevant system permission.
            "computer": "granted",
            "clipboard": "granted",
            "calendar": "granted",
            "reminders": "granted",
        ]
    }

    static func decodeOutput(_ data: Data) -> MacCoreToolPolicyOutput? {
        try? JSONDecoder().decode(MacCoreToolPolicyOutput.self, from: data)
    }

    /// Sends an already-encoded frozen v2 payload through the production ABI.
    /// This keeps the Mac golden test independent from a duplicate Swift fixture.
    static func resolveOutputJSON(_ data: Data) throws -> MacCoreToolPolicyOutput {
#if canImport(NolaCore)
        let version = nola_core_tool_policy_abi_version()
        guard version == 1 else { throw MacCoreToolPolicyError.incompatibleABI(version) }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_tool_policy_resolve(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { throw MacCoreToolPolicyError.invalidResponse }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer),
              let output = decodeOutput(Data(raw.utf8)),
              output.schemaVersion == 1 else { throw MacCoreToolPolicyError.invalidResponse }
        return output
#else
        throw MacCoreToolPolicyError.unavailable
#endif
    }

    private static func resolve(
        identity: MacCoreToolIdentity,
        manifestId: String,
        manifestVersion: Int64,
        nowMs: Int64,
        capabilities: [String],
        authorizations: [String: String],
        toolIds: [String],
        scopedGrants: [MacCoreScopedGrant]
    ) throws -> MacCoreToolPlanReceipt {
        let input = MacCoreToolPolicyInput(
            nowMs: nowMs,
            expectedIdentity: identity,
            runtime: .init(
                manifestId: manifestId,
                manifestVersion: manifestVersion,
                issuedAtMs: nowMs,
                expiresAtMs: nowMs + 300_000,
                hostId: identity.hostId,
                clientId: identity.clientId,
                sessionId: identity.sessionId,
                routes: ["agent_host"],
                capabilities: capabilities,
                authorizations: authorizations,
                tools: toolIds.map {
                    .init(toolId: $0, handlerId: $0, schema: toolSchema(id: $0), available: true)
                }
            ),
            conversation: .init(),
            userPolicy: .init(),
            scopedGrants: scopedGrants
        )
        let output = try resolveOutputJSON(JSONEncoder().encode(input))
        return .init(output: output, identity: identity, sourceManifestId: manifestId, policyInput: input)
    }

    private static func toolSchema(id: String) -> MacCoreJSONValue {
        .object([
            "type": .string("function"),
            "function": .object([
                "name": .string(id),
                "description": .string("Host-provided \(id) handler"),
                "parameters": .object([
                    "type": .string("object"),
                    "properties": .object([:]),
                    "additionalProperties": .bool(true),
                ]),
            ]),
        ])
    }

}
