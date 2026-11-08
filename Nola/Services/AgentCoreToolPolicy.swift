// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed Swift binding for the shared Core tool-policy ABI v2.
// - Security: Core owns tool metadata and returns an identity-bound, expiring plan receipt.

import Foundation

#if canImport(NolaCore)
import NolaCore
#endif

nonisolated public struct AgentCoreExpectedIdentity: Codable, Equatable, Sendable {
    public let hostId: String?
    public let clientId: String?
    public let sessionId: String?

    public init(hostId: String? = nil, clientId: String? = nil, sessionId: String? = nil) {
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
    }

    enum CodingKeys: String, CodingKey {
        case hostId = "host_id"
        case clientId = "client_id"
        case sessionId = "session_id"
    }
}

nonisolated public struct AgentCoreRuntimeToolManifest: Codable, Equatable, Sendable {
    public let toolId: String
    public let handlerId: String?
    public let schema: AgentCoreJSONValue
    public let route: String
    public let available: Bool

    public init(
        toolId: String,
        handlerId: String? = nil,
        schema: AgentCoreJSONValue,
        route: String,
        available: Bool = true
    ) {
        self.toolId = toolId
        self.handlerId = handlerId
        self.schema = schema
        self.route = route
        self.available = available
    }

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case handlerId = "handler_id"
        case schema, route, available
    }
}

nonisolated public struct AgentCoreRuntimeResultEgressReceipt: Codable, Equatable, Sendable {
    public let toolId: String
    public let route: String
    public let manifestId: String
    public let hostId: String
    public let clientId: String
    public let sessionId: String
    public let issuedAtMs: Int64
    public let expiresAtMs: Int64
    public let allowResultEgress: Bool

    public init(
        toolId: String,
        route: String,
        manifestId: String,
        hostId: String,
        clientId: String,
        sessionId: String,
        issuedAtMs: Int64,
        expiresAtMs: Int64,
        allowResultEgress: Bool
    ) {
        self.toolId = toolId
        self.route = route
        self.manifestId = manifestId
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
        self.issuedAtMs = issuedAtMs
        self.expiresAtMs = expiresAtMs
        self.allowResultEgress = allowResultEgress
    }

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

nonisolated public struct AgentCoreRuntimeCapabilityManifest: Codable, Equatable, Sendable {
    public let manifestId: String
    public let manifestVersion: Int64
    public let issuedAtMs: Int64
    public let expiresAtMs: Int64
    public let hostId: String?
    public let clientId: String?
    public let sessionId: String?
    public let routes: [String]
    public let capabilities: [String]
    public let authorizations: [String: String]
    public let resultEgressReceipts: [AgentCoreRuntimeResultEgressReceipt]
    public let tools: [AgentCoreRuntimeToolManifest]

    public init(
        manifestId: String = "apple-runtime",
        manifestVersion: Int64 = 1,
        issuedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        expiresAtMs: Int64? = nil,
        hostId: String? = nil,
        clientId: String? = nil,
        sessionId: String? = nil,
        routes: [String],
        capabilities: [String],
        authorizations: [String: String],
        resultEgressReceipts: [AgentCoreRuntimeResultEgressReceipt] = [],
        tools: [AgentCoreRuntimeToolManifest]
    ) {
        self.manifestId = manifestId
        self.manifestVersion = manifestVersion
        self.issuedAtMs = issuedAtMs
        self.expiresAtMs = expiresAtMs ?? issuedAtMs + 60_000
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
        self.routes = routes
        self.capabilities = capabilities
        self.authorizations = authorizations
        self.resultEgressReceipts = resultEgressReceipts
        self.tools = tools
    }

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

nonisolated public struct AgentCoreConversationToolPolicy: Codable, Equatable, Sendable {
    public let mode: String
    public let allowedTools: [String]
    public let deniedTools: [String]
    public let allowedCapabilities: [String]
    public let deniedCapabilities: [String]

    public init(
        mode: String = "agent",
        allowedTools: [String] = [],
        deniedTools: [String] = [],
        allowedCapabilities: [String] = [],
        deniedCapabilities: [String] = []
    ) {
        self.mode = mode
        self.allowedTools = allowedTools
        self.deniedTools = deniedTools
        self.allowedCapabilities = allowedCapabilities
        self.deniedCapabilities = deniedCapabilities
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case allowedTools = "allow_tools"
        case deniedTools = "deny_tools"
        case allowedCapabilities = "allow_capabilities"
        case deniedCapabilities = "deny_capabilities"
    }
}

nonisolated public struct AgentCoreUserToolPolicy: Codable, Equatable, Sendable {
    public let allowedTools: [String]
    public let deniedTools: [String]
    public let allowedCapabilities: [String]
    public let deniedCapabilities: [String]
    public let maxRisk: String
    public let allowResultEgressRoutes: [String]

    public init(
        allowedTools: [String] = [],
        deniedTools: [String] = [],
        allowedCapabilities: [String] = [],
        deniedCapabilities: [String] = [],
        maxRisk: String = "high",
        allowResultEgressRoutes: [String] = []
    ) {
        self.allowedTools = allowedTools
        self.deniedTools = deniedTools
        self.allowedCapabilities = allowedCapabilities
        self.deniedCapabilities = deniedCapabilities
        self.maxRisk = maxRisk
        self.allowResultEgressRoutes = allowResultEgressRoutes
    }

    enum CodingKeys: String, CodingKey {
        case allowedTools = "allow_tools"
        case deniedTools = "deny_tools"
        case allowedCapabilities = "allow_capabilities"
        case deniedCapabilities = "deny_capabilities"
        case maxRisk = "max_risk"
        case allowResultEgressRoutes = "allow_result_egress_routes"
    }
}

nonisolated public struct AgentCoreScopedGrant: Codable, Equatable, Sendable {
    public let capability: String
    public let route: String?
    public let scope: String?
    public let expiresAtMs: Int64

    public init(capability: String, route: String? = nil, scope: String? = nil, expiresAtMs: Int64) {
        self.capability = capability
        self.route = route
        self.scope = scope
        self.expiresAtMs = expiresAtMs
    }

    enum CodingKeys: String, CodingKey {
        case capability, route, scope
        case expiresAtMs = "expires_at_ms"
    }
}

nonisolated public struct AgentCoreResultEgressGrant: Codable, Equatable, Sendable {
    public let toolId: String
    public let route: String
    public let manifestId: String
    public let hostId: String
    public let clientId: String
    public let sessionId: String
    public let expiresAtMs: Int64

    public init(
        toolId: String,
        route: String,
        manifestId: String,
        hostId: String,
        clientId: String,
        sessionId: String,
        expiresAtMs: Int64
    ) {
        self.toolId = toolId
        self.route = route
        self.manifestId = manifestId
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
        self.expiresAtMs = expiresAtMs
    }

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

nonisolated public struct AgentCoreToolPolicyInput: Codable, Equatable, Sendable {
    public let schemaVersion: Int
    public let platform: String
    public let nowMs: Int64
    public let expectedIdentity: AgentCoreExpectedIdentity?
    public let runtime: AgentCoreRuntimeCapabilityManifest
    public let conversation: AgentCoreConversationToolPolicy
    public let userPolicy: AgentCoreUserToolPolicy
    public let scopedGrants: [AgentCoreScopedGrant]
    public let resultEgressGrants: [AgentCoreResultEgressGrant]

    public init(
        schemaVersion: Int = 2,
        platform: String,
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        expectedIdentity: AgentCoreExpectedIdentity? = nil,
        runtime: AgentCoreRuntimeCapabilityManifest,
        conversation: AgentCoreConversationToolPolicy = .init(),
        userPolicy: AgentCoreUserToolPolicy,
        scopedGrants: [AgentCoreScopedGrant] = [],
        resultEgressGrants: [AgentCoreResultEgressGrant] = []
    ) {
        self.schemaVersion = schemaVersion
        self.platform = platform
        self.nowMs = nowMs
        self.expectedIdentity = expectedIdentity
        self.runtime = runtime
        self.conversation = conversation
        self.userPolicy = userPolicy
        self.scopedGrants = scopedGrants
        self.resultEgressGrants = resultEgressGrants
    }

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

nonisolated public struct AgentCoreResolvedTool: Codable, Equatable, Sendable {
    public let toolId: String
    public let name: String
    public let enabled: Bool
    public let route: String
    public let capability: String
    public let permission: String
    public let authorization: String
    public let risk: String
    public let requiresResultEgress: Bool
    public let reason: String
    public let schema: AgentCoreJSONValue

    enum CodingKeys: String, CodingKey {
        case toolId = "tool_id"
        case name, enabled, route, capability, permission, authorization, risk, reason, schema
        case requiresResultEgress = "requires_result_egress"
    }
}

nonisolated public struct AgentCoreToolPolicyOutput: Codable, Equatable, Sendable {
    public let schemaVersion: Int
    public let platform: String
    public let policyDigest: String
    public let manifestDigest: String
    public let manifestValidUntilMs: Int64
    public let enabledTools: [AgentCoreResolvedTool]
    public let disabledTools: [AgentCoreResolvedTool]

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

nonisolated public struct AgentCorePreparedToolPlan: Equatable, Sendable {
    public let output: AgentCoreToolPolicyOutput
    public let expectedIdentity: AgentCoreExpectedIdentity
    public let authorizationSnapshot: [String: String]
    public let isForeground: Bool
    public let scopedGrants: [AgentCoreScopedGrant]
    public let resultEgressGrants: [AgentCoreResultEgressGrant]
    /// Original admission input, never reconstructed from enabledTools.
    public let originalIntent: AgentCoreToolPolicyInput?

    public init(output: AgentCoreToolPolicyOutput, expectedIdentity: AgentCoreExpectedIdentity,
                authorizationSnapshot: [String: String], isForeground: Bool,
                scopedGrants: [AgentCoreScopedGrant], resultEgressGrants: [AgentCoreResultEgressGrant],
                originalIntent: AgentCoreToolPolicyInput? = nil) {
        self.output = output
        self.expectedIdentity = expectedIdentity
        self.authorizationSnapshot = authorizationSnapshot
        self.isForeground = isForeground
        self.scopedGrants = scopedGrants
        self.resultEgressGrants = resultEgressGrants
        self.originalIntent = originalIntent
    }

    public var schemas: [AgentCoreJSONValue] { output.enabledTools.map(\.schema) }

    public func resolvedTool(
        id: String,
        route: String,
        hostId: String?,
        clientId: String?,
        sessionId: String?,
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) -> AgentCoreResolvedTool? {
        guard nowMs < output.manifestValidUntilMs,
              expectedIdentity.hostId == hostId,
              expectedIdentity.clientId == clientId,
              expectedIdentity.sessionId == sessionId,
              let tool = output.enabledTools.first(where: { $0.toolId == id && $0.enabled }),
              tool.route == route,
              tool.authorization != "denied",
              tool.authorization != "unavailable" else {
            return nil
        }
        if tool.requiresResultEgress {
            guard let hostId,
                  let clientId,
                  let sessionId,
                  resultEgressGrants.contains(where: {
                      $0.toolId == tool.toolId
                          && $0.route == tool.route
                          && $0.hostId == hostId
                          && $0.clientId == clientId
                          && $0.sessionId == sessionId
                          && $0.expiresAtMs > nowMs
                  }) else {
                return nil
            }
        }
        return tool
    }
}

nonisolated public enum AgentCoreToolPolicyError: LocalizedError, Equatable {
    case unavailable
    case incompatibleABI(UInt32)
    case invalidResponse
    case invalidPlan(String)

    public var errorDescription: String? {
        switch self {
        case .unavailable: return "The shared Core tool-policy library is unavailable."
        case .incompatibleABI(let version): return "Unsupported Core tool-policy ABI version: \(version)."
        case .invalidResponse: return "Core returned an invalid tool-policy response."
        case .invalidPlan(let reason): return reason
        }
    }
}

/// Stateless policy ABI. This type cannot own or mutate an Agent execution.
nonisolated public enum AgentCoreToolPolicyResolver {
    public static var toolPolicyABIVersion: UInt32 {
#if canImport(NolaCore)
        nola_core_tool_policy_abi_version()
#else
        0
#endif
    }

    public static func resolveToolPolicy(_ input: AgentCoreToolPolicyInput) throws -> AgentCoreToolPolicyOutput {
        try resolveToolPolicyJSON(JSONEncoder().encode(input))
    }

    /// Resolves the frozen Core contract without rebuilding the payload in Swift.
    /// Tests use this entry point to feed the repository's shared golden fixture
    /// directly through the same C ABI used in production.
    public static func resolveToolPolicyJSON(_ data: Data) throws -> AgentCoreToolPolicyOutput {
#if canImport(NolaCore)
        let version = nola_core_tool_policy_abi_version()
        guard version == 2 else { throw AgentCoreToolPolicyError.incompatibleABI(version) }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_tool_policy_resolve(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { throw AgentCoreToolPolicyError.invalidResponse }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else {
            throw AgentCoreToolPolicyError.invalidResponse
        }
        return try JSONDecoder().decode(AgentCoreToolPolicyOutput.self, from: Data(raw.utf8))
#else
        throw AgentCoreToolPolicyError.unavailable
#endif
    }
}
