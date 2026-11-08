// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Builds the device tool catalog exposed to the shared agent runtime.
// - Security: Host tools are never injected by the mobile catalog.

import Foundation

#if canImport(CoreLocation)
import CoreLocation
#endif
#if canImport(EventKit)
import EventKit
#endif

public enum AgentCoreToolCatalog {
    public static func tools(
        for mode: AgentCoreExecutionMode,
        isForeground: Bool = true,
        allowedTools: Set<String> = [],
        deniedTools: Set<String> = [],
        allowedCapabilities: Set<String> = [],
        deniedCapabilities: Set<String> = []
    ) -> [AgentCoreJSONValue] {
        guard let plan = try? resolvePlan(
            for: mode,
            isForeground: isForeground,
            allowedTools: allowedTools,
            deniedTools: deniedTools,
            allowedCapabilities: allowedCapabilities,
            deniedCapabilities: deniedCapabilities
        ) else {
            return []
        }
        return plan.enabledTools.map { resolved in
            schema(for: resolved, resultEgressAllowed: false)
        }
    }

    public static func resolvePlan(
        for mode: AgentCoreExecutionMode,
        isForeground: Bool,
        allowedTools: Set<String> = [],
        deniedTools: Set<String> = [],
        allowedCapabilities: Set<String> = [],
        deniedCapabilities: Set<String> = []
    ) throws -> AgentCoreToolPolicyOutput {
        let candidates: [AgentCoreJSONValue]
        switch mode {
        case .hostRemote: candidates = []
        case .deviceLocal, .hybrid: candidates = deviceTools
        }
        let manifests = candidates.compactMap { manifest(for: $0) }
        var capabilities = Set(["none", "files_read", "files_write"])
        var authorizations: [String: String] = [
            "none": "granted",
            "files_read": "ask",
            "files_write": "ask",
            "clipboard": isForeground ? "ask" : "unavailable",
            "location": isForeground ? locationAuthorization : "unavailable",
            "calendar": isForeground ? calendarAuthorization : "unavailable",
        ]
        if isForeground {
            capabilities.formUnion(["clipboard", "location", "calendar"])
        }
        for denied in deniedCapabilities {
            authorizations[denied] = "denied"
        }
        let routes: [String]
        switch mode {
        case .deviceLocal: routes = ["device"]
        case .hostRemote: routes = ["agent_host"]
        case .hybrid: routes = ["device", "agent_host"]
        }
        let input = AgentCoreToolPolicyInput(
            schemaVersion: 1,
            platform: "ios",
            runtime: AgentCoreRuntimeCapabilityManifest(
                routes: routes,
                capabilities: capabilities.sorted(),
                authorizations: authorizations,
                tools: manifests
            ),
            conversation: AgentCoreConversationToolPolicy(
                allowedTools: allowedTools.sorted(),
                deniedTools: deniedTools.sorted()
            ),
            userPolicy: AgentCoreUserToolPolicy(
                allowedTools: allowedTools.sorted(),
                deniedTools: deniedTools.sorted(),
                allowedCapabilities: allowedCapabilities.sorted(),
                deniedCapabilities: deniedCapabilities.sorted()
            )
        )
        return try AgentCoreToolPolicyResolver.resolveToolPolicy(input)
    }

    private static func manifest(for schema: AgentCoreJSONValue) -> AgentCoreRuntimeToolManifest? {
        guard case .object(let root) = schema,
              case .object(let function)? = root["function"],
              case .string(let name)? = function["name"] else {
            return nil
        }
        return AgentCoreRuntimeToolManifest(
            toolId: name,
            handlerId: name,
            schema: schema,
            route: "device",
            available: true
        )
    }

    static func schema(
        for resolved: AgentCoreResolvedTool,
        resultEgressAllowed: Bool
    ) -> AgentCoreJSONValue {
        guard case .object(var root) = resolved.schema else { return resolved.schema }
        var metadata: [String: AgentCoreJSONValue]
        if case .object(let existing)? = root["x_nola"] {
            metadata = existing
        } else {
            metadata = [:]
        }
        metadata["tool_id"] = .string(resolved.toolId)
        metadata["target"] = .string(resolved.route)
        metadata["capability"] = .string(resolved.capability)
        metadata["permission"] = .string(resolved.permission)
        metadata["authorization"] = .string(resolved.authorization)
        metadata["risk"] = .string(resolved.risk)
        metadata["requires_result_egress"] = .bool(resolved.requiresResultEgress)
        metadata["result_egress_allowed"] = .bool(resultEgressAllowed)
        root["x_nola"] = .object(metadata)
        return .object(root)
    }

    private static var locationAuthorization: String {
#if canImport(CoreLocation)
        switch CLLocationManager().authorizationStatus {
        case .notDetermined: return "ask"
        case .restricted, .denied: return "denied"
        case .authorizedAlways, .authorizedWhenInUse: return "granted"
        @unknown default: return "unavailable"
        }
#else
        return "unavailable"
#endif
    }

    private static var calendarAuthorization: String {
#if canImport(EventKit)
        switch EKEventStore.authorizationStatus(for: .event).rawValue {
        case 0: return "ask"
        case 1, 2: return "denied"
        default: return "granted"
        }
#else
        return "unavailable"
#endif
    }

    static let deviceTools: [AgentCoreJSONValue] = [
        tool(
            id: "device.skills.invoke",
            name: "skill_invoke",
            description: "Load the complete instructions for one enabled Skill selected from an available_skill context entry. Calling this tool is required before following or claiming use of that Skill. Pass the exact advertised name; a successful invoked_skill result activates its instructions for the current turn.",
            capability: "none",
            effect: "read",
            risk: "safe",
            permissionRisk: "ordinary_read",
            properties: [
                "skill": stringProperty("Exact advertised Skill name"),
                "args": .object([
                    "type": .string("object"),
                    "description": .string("Optional structured arguments for the Skill"),
                    "additionalProperties": .bool(true)
                ]),
                "activation": .object([
                    "type": .string("string"),
                    "enum": .array([.string("turn"), .string("session")]),
                    "description": .string("Requested activation lifetime; defaults to turn")
                ])
            ],
            required: ["skill"]
        ),
        tool(
            id: "device.memory.search",
            name: "memory_search",
            description: "Search durable long-term memory when earlier user context may help the current task. The query is chosen by the model and is not derived by the client.",
            capability: "none",
            effect: "read",
            risk: "safe",
            permissionRisk: "ordinary_read",
            properties: [
                "query": stringProperty("Search query describing the memory evidence needed"),
                "limit": .object([
                    "type": .string("integer"),
                    "minimum": .number(1),
                    "maximum": .number(20),
                    "description": .string("Maximum number of memory records to return")
                ])
            ],
            required: ["query"]
        ),
        tool(
            id: "device.files.read",
            name: "device_read_file",
            description: "Read one file selected by the user on this device.",
            capability: "files_read",
            effect: "read",
            risk: "sensitive",
            permissionRisk: "ordinary_read"
        ),
        tool(
            id: "device.files.write",
            name: "device_write_file",
            description: "Write UTF-8 text to one file selected by the user on this device.",
            capability: "files_write",
            effect: "write",
            risk: "destructive",
            permissionRisk: "unknown",
            properties: [
                "content": .object([
                    "type": .string("string"),
                    "description": .string("UTF-8 text to write")
                ])
            ],
            required: ["content"]
        ),
        tool(
            id: "device.clipboard.read",
            name: "device_clipboard_read",
            description: "Read text from the device clipboard after explicit approval.",
            capability: "clipboard",
            effect: "read",
            risk: "sensitive",
            permissionRisk: "sensitive_read",
            requiresForeground: true
        ),
        tool(
            id: "device.clipboard.write",
            name: "device_clipboard_write",
            description: "Write text to the device clipboard after explicit approval.",
            capability: "clipboard",
            effect: "write",
            risk: "sensitive",
            permissionRisk: "overwrite",
            properties: [
                "content": .object([
                    "type": .string("string"),
                    "description": .string("Text to place on the clipboard")
                ])
            ],
            required: ["content"],
            requiresForeground: true
        ),
        tool(
            id: "device.location.current",
            name: "device_get_location",
            description: "Get the device's current location after foreground approval.",
            capability: "location",
            effect: "read",
            risk: "sensitive",
            permissionRisk: "sensitive_read",
            requiresForeground: true
        ),
        tool(
            id: "device.calendar.list",
            name: "device_search_calendar_events",
            description: "List device calendar events in an ISO 8601 time range. Defaults to the next 24 hours.",
            capability: "calendar",
            effect: "read",
            risk: "sensitive",
            permissionRisk: "sensitive_read",
            properties: dateRangeProperties,
            requiresForeground: true
        ),
        tool(
            id: "device.calendar.create",
            name: "device_create_calendar_event",
            description: "Create an event in the device's default calendar after explicit approval.",
            capability: "calendar",
            effect: "external_write",
            risk: "destructive",
            permissionRisk: "new_file",
            properties: [
                "title": stringProperty("Event title"),
                "start_iso": stringProperty("ISO 8601 start time"),
                "end_iso": stringProperty("ISO 8601 end time"),
                "notes": stringProperty("Optional event notes")
            ],
            required: ["title", "start_iso", "end_iso"],
            requiresForeground: true
        )
    ]

    private static let dateRangeProperties: [String: AgentCoreJSONValue] = [
        "start_iso": stringProperty("Optional ISO 8601 range start"),
        "end_iso": stringProperty("Optional ISO 8601 range end, at most 31 days after the start")
    ]

    private static func stringProperty(_ description: String) -> AgentCoreJSONValue {
        .object(["type": .string("string"), "description": .string(description)])
    }

    private static func tool(
        id: String,
        name: String,
        description: String,
        capability: String,
        effect: String,
        risk: String,
        permissionRisk: String,
        properties: [String: AgentCoreJSONValue] = [:],
        required: [String] = [],
        requiresForeground: Bool = false
    ) -> AgentCoreJSONValue {
        .object([
            "type": .string("function"),
            "function": .object([
                "name": .string(name),
                "description": .string(description),
                "parameters": .object([
                    "type": .string("object"),
                    "properties": .object(properties),
                    "required": .array(required.map(AgentCoreJSONValue.string)),
                    "additionalProperties": .bool(false)
                ])
            ]),
            "x_nola": .object([
                "tool_id": .string(id),
                "target": .string("device"),
                "capability": .string(capability),
                "effect": .string(effect),
                "risk": .string(risk),
                "permission_contract": .object(["risk": .string(permissionRisk)]),
                "result_destination": .string("device_only"),
                "requires_foreground": .bool(requiresForeground)
            ])
        ])
    }
}
