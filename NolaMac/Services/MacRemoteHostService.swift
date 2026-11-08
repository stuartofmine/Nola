// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Connects the macOS client to the existing encrypted Nola mobile host stream.

import CryptoKit
import Foundation
import Security

/// Authenticated execution-Host facts. The client never derives these from settings
/// or recomputes the remote Host's policy using its own platform catalog.
private nonisolated struct MacHostToolPlanKey: CodingKey {
    let stringValue: String
    let intValue: Int? = nil
    init(_ value: String) { stringValue = value }
    init?(stringValue: String) { self.init(stringValue) }
    init?(intValue: Int) { return nil }
}

private nonisolated func macHostToolPlanFields(_ decoder: Decoder, _ fields: Set<String>) throws -> KeyedDecodingContainer<MacHostToolPlanKey> {
    let container = try decoder.container(keyedBy: MacHostToolPlanKey.self)
    guard Set(container.allKeys.map(\.stringValue)) == fields else {
        throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unexpected Host tool plan fields"))
    }
    return container
}

nonisolated struct MacRemoteHostToolPlan: Codable, Equatable, Sendable {
    struct Reference: Codable, Equatable, Sendable {
        let manifest_id: String
        let host_id: String
        let client_id: String
        let session_id: String
        let catalog_revision: String

        init(from decoder: Decoder) throws {
            let fields = try macHostToolPlanFields(decoder, ["manifest_id", "host_id", "client_id", "session_id", "catalog_revision"])
            manifest_id = try fields.decode(String.self, forKey: .init("manifest_id"))
            host_id = try fields.decode(String.self, forKey: .init("host_id"))
            client_id = try fields.decode(String.self, forKey: .init("client_id"))
            session_id = try fields.decode(String.self, forKey: .init("session_id"))
            catalog_revision = try fields.decode(String.self, forKey: .init("catalog_revision"))
        }
    }
    struct Tool: Codable, Equatable, Sendable {
        let tool_id: String
        let route: String
        let schema: MacCanonicalJSONValue

        init(from decoder: Decoder) throws {
            let fields = try macHostToolPlanFields(decoder, ["tool_id", "route", "schema"])
            tool_id = try fields.decode(String.self, forKey: .init("tool_id"))
            route = try fields.decode(String.self, forKey: .init("route"))
            schema = try fields.decode(MacCanonicalJSONValue.self, forKey: .init("schema"))
        }
    }
    let schema_version: Int
    let reference: Reference
    let issued_at_ms: Int64
    let expires_at_ms: Int64
    let tools: [Tool]

    init(from decoder: Decoder) throws {
        let fields = try macHostToolPlanFields(decoder, ["schema_version", "reference", "issued_at_ms", "expires_at_ms", "tools"])
        schema_version = try fields.decode(Int.self, forKey: .init("schema_version"))
        reference = try fields.decode(Reference.self, forKey: .init("reference"))
        issued_at_ms = try fields.decode(Int64.self, forKey: .init("issued_at_ms"))
        expires_at_ms = try fields.decode(Int64.self, forKey: .init("expires_at_ms"))
        tools = try fields.decode([Tool].self, forKey: .init("tools"))
    }

    // Host admission owns freshness; client clocks only validate structural bounds.
    func validate(hostID: String, clientID: String, sessionID: String) throws {
        let validIdentity: (String) -> Bool = {
            !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0.utf8.count <= 512
                && !$0.unicodeScalars.contains(where: { $0.value <= 0x1f || (0x7f...0x9f).contains($0.value) })
        }
        guard [hostID, clientID, sessionID, reference.manifest_id].allSatisfy(validIdentity),
              schema_version == 1, reference.host_id == hostID, reference.client_id == clientID,
              reference.session_id == sessionID,
              reference.catalog_revision.utf8.count == 64,
              reference.catalog_revision.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              issued_at_ms > 0, expires_at_ms > issued_at_ms, expires_at_ms <= 9_007_199_254_740_991,
              expires_at_ms - issued_at_ms <= 300_000,
              Set(tools.map(\.tool_id)).count == tools.count else { throw MacRemoteHostError.invalidResponse }
        for tool in tools {
            let data = try JSONEncoder().encode(tool.schema)
            guard validIdentity(tool.tool_id), tool.route == "agent_host",
                  let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  envelope["type"] as? String == "function",
                  let function = envelope["function"] as? [String: Any],
                  function["name"] as? String == tool.tool_id,
                  function["parameters"] is [String: Any] else { throw MacRemoteHostError.invalidResponse }
        }
    }

    var referenceObject: [String: Any] {
        ["manifest_id": reference.manifest_id, "host_id": reference.host_id,
         "client_id": reference.client_id, "session_id": reference.session_id,
         "catalog_revision": reference.catalog_revision]
    }
}

nonisolated enum MacRemoteContextPayload {
    static func encode(
        _ components: [MacCoreContextComponent]
    ) throws -> [[String: Any]] {
        try components.map { component in
            let data = try JSONEncoder().encode(component)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw MacRemoteHostError.invalidResponse
            }
            return object
        }
    }
}

/// Identity belongs to the connected Host's catalog, never the local model name.
nonisolated struct MacRemoteModelProfile: Hashable, Sendable {
    let hostID: String
    let profileID: String
    let revision: UInt64
    let displayName: String
    let modelName: String
}

nonisolated struct MacRemoteModelCatalogSnapshot: Equatable, Sendable {
    let hostID: String
    let profiles: [MacRemoteModelProfile]
    let defaultResolution: MacChatModelResolution
    let defaultRevision: Int64?
}

nonisolated enum MacRemoteModelCatalog {
    static func decodeSnapshot(settings: [String: Any], expectedHostID: String?) throws -> MacRemoteModelCatalogSnapshot {
        let profiles = try decode(settings: settings, expectedHostID: expectedHostID)
        guard let hostID = (settings["host_identity"] as? [String: Any])?["id"] as? String,
              let resolution = settings["default_chat_model_resolution"] as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let decoded = try JSONDecoder().decode(MacChatModelResolution.self,
            from: JSONSerialization.data(withJSONObject: resolution))
        if case let .resolved(profileID, origin) = decoded {
            guard origin != .conversation, profiles.contains(where: { $0.profileID == profileID }) else {
                throw MacRemoteHostError.invalidResponse
            }
        }
        let revision: Int64?
        if let value = settings["default_chat_model_revision"], !(value is NSNull) {
            guard let number = value as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(), number.int64Value > 0,
                  number.doubleValue == Double(number.int64Value) else { throw MacRemoteHostError.invalidResponse }
            revision = number.int64Value
        } else { revision = nil }
        return .init(hostID: hostID, profiles: profiles, defaultResolution: decoded, defaultRevision: revision)
    }

    static func decode(settings: [String: Any], expectedHostID: String?) throws -> [MacRemoteModelProfile] {
        do {
            return try AppleRemoteHostModelCatalog.profiles(settings: settings, expectedHostID: expectedHostID).map {
                MacRemoteModelProfile(hostID: $0.hostID, profileID: $0.profileID, revision: $0.revision,
                                      displayName: $0.displayName, modelName: $0.modelName)
            }
        } catch { throw MacRemoteHostError.invalidResponse }
    }
}

/// Frozen user intent. No endpoint, credentials, wire protocol or capability
/// declaration crosses this boundary; the Host resolves those from its profile.
private nonisolated struct MacRemoteGenerationOverrides: Encodable, Sendable {
    struct Options: Encodable, Sendable {
        let topK: Int?
        let presencePenalty: Double?
        let frequencyPenalty: Double?
        let repetitionPenalty: Double?
        let seed: MacSeedOverride?
        let stop: [String]?
        var isEmpty: Bool {
            topK == nil && presencePenalty == nil && frequencyPenalty == nil
                && repetitionPenalty == nil && seed == nil && stop == nil
        }
        enum CodingKeys: String, CodingKey {
            case seed, stop
            case topK = "top_k"
            case presencePenalty = "presence_penalty"
            case frequencyPenalty = "frequency_penalty"
            case repetitionPenalty = "repetition_penalty"
        }
        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encodeIfPresent(topK, forKey: .topK)
            try container.encodeIfPresent(presencePenalty, forKey: .presencePenalty)
            try container.encodeIfPresent(frequencyPenalty, forKey: .frequencyPenalty)
            try container.encodeIfPresent(repetitionPenalty, forKey: .repetitionPenalty)
            try container.encodeIfPresent(stop, forKey: .stop)
            switch seed {
            case .none: break
            case .clear: try container.encodeNil(forKey: .seed)
            case .value(let value): try container.encode(value, forKey: .seed)
            }
        }
    }
    let temperature: Double?
    let topP: Double?
    let maxOutputTokens: Int?
    let maxContextTokens: Int?
    let samplingOverride: Bool?
    let generationOptions: Options?

    init(_ value: MacConversationModelOverride?) {
        temperature = value?.temperature
        topP = value?.topP
        maxOutputTokens = value?.maxOutputTokens
        maxContextTokens = value?.maxContextLength
        samplingOverride = temperature != nil || topP != nil ? true : nil
        let options = Options(topK: value?.topK, presencePenalty: value?.presencePenalty,
            frequencyPenalty: value?.frequencyPenalty, repetitionPenalty: value?.repetitionPenalty,
            seed: value?.seed,
            stop: value?.stopSequences.map { $0.components(separatedBy: .newlines).filter { !$0.isEmpty } })
        generationOptions = options.isEmpty ? nil : options
    }
    enum CodingKeys: String, CodingKey {
        case temperature
        case topP = "top_p"
        case maxOutputTokens = "max_output_tokens"
        case maxContextTokens = "max_context_tokens"
        case samplingOverride = "sampling_override"
        case generationOptions = "generation_options"
    }
}

nonisolated struct MacRemoteDefaultModelExpectation: Equatable, Sendable {
    let revision: Int64?
    let profileID: String
}

nonisolated struct MacRemoteModelIntent: Sendable {
    let hostID: String
    let profileID: String
    let expectedRevision: UInt64
    let tier: String
    let expectedDefault: MacRemoteDefaultModelExpectation?
    private let overrides: MacRemoteGenerationOverrides

    init(profile: MacRemoteModelProfile, tier: String, overrides: MacConversationModelOverride? = nil,
         expectedDefault: MacRemoteDefaultModelExpectation? = nil) throws {
        guard overrides?.wireProtocol == nil else {
            throw MacRemoteHostError.http(400, "The connected Host owns its model protocol. Remove the session protocol override before sending.")
        }
        guard ["concise", "balanced", "detailed"].contains(tier) else {
            throw MacRemoteHostError.invalidResponse
        }
        guard expectedDefault == nil || expectedDefault?.profileID == profile.profileID else {
            throw MacRemoteHostError.invalidResponse
        }
        hostID = profile.hostID
        profileID = profile.profileID
        expectedRevision = profile.revision
        self.tier = tier
        self.expectedDefault = expectedDefault
        self.overrides = MacRemoteGenerationOverrides(overrides)
    }

    func payload() throws -> [String: Any] {
        ["profile_id": profileID, "expected_revision": expectedRevision, "tier": tier,
         "overrides": try JSONSerialization.jsonObject(with: JSONEncoder().encode(overrides))]
    }
}

nonisolated enum MacRemoteModelPayload {
    static func apply(
        to payload: inout [String: Any], intent: MacRemoteModelIntent?,
        settings: [String: Any], expectedHostID: String?,
        executionBackend: String, modelID: String, modelProviderID: String?
    ) throws {
        if executionBackend == "nola" {
            guard let intent else {
                throw MacRemoteHostError.http(400, "Select a model from the connected Host's model catalog before sending.")
            }
            let catalog = try MacRemoteModelCatalog.decode(settings: settings, expectedHostID: expectedHostID)
            guard let current = catalog.first(where: { $0.hostID == intent.hostID && $0.profileID == intent.profileID }),
                  current.revision == intent.expectedRevision else {
                throw MacRemoteHostError.http(409, "The connected Host's model profile changed or was removed. Refresh the model catalog and select it again.")
            }
            if let expectedDefault = intent.expectedDefault {
                let snapshot = try MacRemoteModelCatalog.decodeSnapshot(settings: settings, expectedHostID: intent.hostID)
                guard snapshot.defaultRevision == expectedDefault.revision,
                      snapshot.defaultResolution.profileID == expectedDefault.profileID else {
                    throw MacRemoteHostError.http(409, "The connected Host's default chat model changed or is unavailable. Refresh the model catalog before sending.")
                }
            }
            payload["expected_host_id"] = intent.hostID
            payload["model_intent"] = try intent.payload()
        } else {
            guard intent == nil else {
                throw MacRemoteHostError.http(400, "Nola model intent cannot be sent to an external execution backend.")
            }
            if !modelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                payload["execution_model"] = modelID
                if let modelProviderID, !modelProviderID.isEmpty {
                    payload["execution_model_provider"] = modelProviderID
                }
            }
        }
    }
}

nonisolated struct MacRemoteResumeRequest {
    let conversationID: String
    let expectedRunID: String
    let expectedHostID: String
    let clientID: String

    func payload() throws -> [String: Any] {
        guard [conversationID, expectedRunID, expectedHostID, clientID].allSatisfy({
            !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }) else { throw MacRemoteHostError.invalidResponse }
        return ["conversation_id": conversationID, "expected_run_id": expectedRunID,
                "expected_host_id": expectedHostID, "client_id": clientID]
    }
}

nonisolated struct MacRemoteHostConfiguration: Sendable {
    let baseURL: URL
    let token: String
    let expectedHostID: String?

    init(baseURL: URL, token: String, expectedHostID: String? = nil) {
        self.baseURL = baseURL
        self.token = token
        let normalizedHostID = expectedHostID?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        self.expectedHostID = normalizedHostID.isEmpty ? nil : normalizedHostID
        NolaTailcatTransport.register(baseURL: baseURL, token: token, hostID: self.expectedHostID)
    }
}

nonisolated struct MacRemoteHostConnectionReport: Equatable, Sendable {
    let hostID: String
    let displayName: String
    let origin: String
    let checkedAt: Date
    let messagingWorkerEnabled: Bool
    let messagingChannelCount: Int
    let messagingConnectedCount: Int
    let messagingManagementURL: URL?
}

nonisolated func macRemoteHostControlPlaneURL(baseURL: URL, path: String) -> URL? {
    guard var origin = URLComponents(url: baseURL, resolvingAgainstBaseURL: false),
          origin.scheme != nil,
          origin.host != nil else { return nil }
    if let absolute = URL(string: path),
       let scheme = absolute.scheme?.lowercased(),
       scheme == "http" || scheme == "https" {
        return absolute
    }
    let requested = URLComponents(string: path)
    origin.path = "/" + (requested?.path ?? path)
        .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    origin.query = requested?.query
    origin.fragment = requested?.fragment
    return origin.url
}

nonisolated enum MacRemoteHostConnectionState: Equatable, Sendable {
    case notConfigured
    case unverified
    case checking
    case connected(MacRemoteHostConnectionReport)
    case failed(String)

    static var initial: Self {
        MacRemoteHostConfigurationStore.shared.load() == nil ? .notConfigured : .unverified
    }

    var isConfigured: Bool {
        if case .notConfigured = self { return false }
        return true
    }
}

nonisolated struct MacPermissionInstruction: Codable, Hashable, Sendable {
    let permissionRequestID: String
    let instruction: String

    enum CodingKeys: String, CodingKey {
        case permissionRequestID = "permission_request_id"
        case instruction
    }

    static func decode(_ value: Any?) -> [Self]? {
        guard let value else { return [] }
        guard let records = value as? [[String: Any]],
              records.allSatisfy({ Set($0.keys) == ["permission_request_id", "instruction"] }),
              let data = try? JSONSerialization.data(withJSONObject: records),
              let decoded = try? JSONDecoder().decode([Self].self, from: data),
              decoded.allSatisfy({ !$0.permissionRequestID.isEmpty && !$0.instruction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else { return nil }
        return decoded
    }
}

/// Current Core descriptor captured with the exact permission challenge. A Run's
/// initial catalog is not the complete set after an approved MCP activation.
nonisolated struct MacRemotePermissionToolEvidence: Hashable, Sendable {
    let catalogRevision: UInt64
    let toolID: String
    let schemaData: Data

    static func decode(_ value: Any?, toolName: String) -> Self? {
        struct Wire: Decodable {
            let catalog_revision: UInt64
            let tool: MacRemoteHostToolPlan.Tool
            init(from decoder: Decoder) throws {
                let fields = try macHostToolPlanFields(decoder, ["catalog_revision", "tool"])
                catalog_revision = try fields.decode(UInt64.self, forKey: .init("catalog_revision"))
                tool = try fields.decode(MacRemoteHostToolPlan.Tool.self, forKey: .init("tool"))
            }
        }
        guard let value, JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value),
              let wire = try? JSONDecoder().decode(Wire.self, from: data),
              wire.tool.tool_id == toolName, !toolName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              toolName.utf8.count <= 512,
              !toolName.unicodeScalars.contains(where: { $0.value <= 0x1f || (0x7f...0x9f).contains($0.value) }),
              wire.tool.route == "agent_host", let schema = try? JSONEncoder().encode(wire.tool.schema),
              let object = try? JSONSerialization.jsonObject(with: schema) as? [String: Any],
              object["type"] as? String == "function", let function = object["function"] as? [String: Any],
              function["name"] as? String == toolName, function["parameters"] is [String: Any],
              let canonical = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return nil }
        return Self(catalogRevision: wire.catalog_revision, toolID: toolName, schemaData: canonical)
    }
}

nonisolated struct MacRemotePermissionRequest: Identifiable, Hashable, Sendable {
    struct PresentationIdentity: Hashable, Sendable {
        let hostID: String
        let clientID: String
        let sessionID: String
        let requestID: String
        let nonce: String
        let callID: String?
    }

    /// Preserve interaction state for metadata updates, but never across
    /// owners or renewed challenges that happen to reuse a request ID.
    var presentationIdentity: PresentationIdentity {
        PresentationIdentity(hostID: hostID, clientID: clientID,
            sessionID: sessionID, requestID: id, nonce: nonce, callID: callID)
    }

    static let supportedProtocolVersion = 4
    let id: String
    let allowConversation: Bool
    let callID: String?
    let protocolVersion: Int?
    let nonce: String
    let hostID: String
    let clientID: String
    let sessionID: String
    let toolName: String
    let arguments: String
    let permissionInstructions: [MacPermissionInstruction]
    let reason: String
    let message: String
    let expiresAt: Int64?
    let policyDigest: String?
    let manifestDigest: String?
    let manifestValidUntilMs: Int64?
    let toolCatalogEvidence: MacRemotePermissionToolEvidence?
    let mcpDescriptorData: Data?

    init(
        id: String,
        allowConversation: Bool,
        protocolVersion: Int?,
        nonce: String,
        hostID: String,
        clientID: String,
        sessionID: String,
        toolName: String,
        arguments: String,
        permissionInstructions: [MacPermissionInstruction] = [],
        callID: String? = nil,
        reason: String,
        message: String,
        expiresAt: Int64?,
        policyDigest: String? = nil,
        manifestDigest: String? = nil,
        manifestValidUntilMs: Int64? = nil,
        toolCatalogEvidence: MacRemotePermissionToolEvidence? = nil,
        mcpDescriptorData: Data? = nil
    ) {
        self.id = id
        self.allowConversation = allowConversation
        self.protocolVersion = protocolVersion
        self.nonce = nonce
        self.hostID = hostID
        self.clientID = clientID
        self.sessionID = sessionID
        self.toolName = toolName
        self.arguments = arguments
        self.permissionInstructions = permissionInstructions
        self.callID = callID
        self.reason = reason
        self.message = message
        self.expiresAt = expiresAt
        self.policyDigest = policyDigest
        self.manifestDigest = manifestDigest
        self.manifestValidUntilMs = manifestValidUntilMs
        self.toolCatalogEvidence = toolCatalogEvidence
        self.mcpDescriptorData = mcpDescriptorData
    }

    func decision(
        allow: Bool,
        scope: MacPermissionGrantScope = .once
    ) -> MacRemotePermissionDecision {
        MacRemotePermissionDecision(
            protocolVersion: protocolVersion,
            nonce: nonce,
            requestID: id,
            hostID: hostID,
            clientID: clientID,
            sessionID: sessionID,
            toolName: toolName,
            policyDigest: policyDigest,
            manifestDigest: manifestDigest,
            resolution: .authorization(allow: allow, scope: scope)
        )
    }

    func instructionDecision(_ instruction: String) throws -> MacRemotePermissionDecision {
        let text = instruction.trimmingCharacters(in: .whitespacesAndNewlines)
        let total = permissionInstructions.reduce(id.utf8.count + text.utf8.count) {
            $0 + $1.permissionRequestID.utf8.count + $1.instruction.utf8.count
        }
        guard !text.isEmpty, text.utf8.count <= 4096,
              permissionInstructions.count < 16, total <= 16384 else {
            throw MacPermissionInstructionError.invalidInstruction
        }
        return MacRemotePermissionDecision(
            protocolVersion: protocolVersion, nonce: nonce, requestID: id,
            hostID: hostID, clientID: clientID, sessionID: sessionID, toolName: toolName,
            policyDigest: policyDigest, manifestDigest: manifestDigest,
            resolution: .provideInstruction(text)
        )
    }
}

nonisolated enum MacPermissionResolution: Sendable {
    case authorization(allow: Bool, scope: MacPermissionGrantScope)
    case provideInstruction(String)

    var isAllowed: Bool {
        if case .authorization(allow: true, scope: _) = self { return true }
        return false
    }

    var payload: [String: Any] {
        switch self {
        case .authorization(let allow, let scope):
            return ["type": "authorization", "decision": allow
                ? (scope == .conversation ? "allow_conversation" : "allow_once") : "deny"]
        case .provideInstruction(let instruction):
            return ["type": "intervention", "intervention": [
                "type": "provide_instruction", "instruction": instruction
            ]]
        }
    }
}

nonisolated enum MacPermissionInstructionError: LocalizedError {
    case invalidInstruction
    case noPendingRequest
    case unsupported

    var errorDescription: String? {
        switch self {
        case .invalidInstruction:
            return String(localized: "Enter guidance within the remaining permission guidance limit.", table: "Chat")
        case .noPendingRequest:
            return String(localized: "The permission request is no longer pending.")
        case .unsupported:
            return String(localized: "This host does not support permission guidance.", table: "Chat")
        }
    }
}

nonisolated struct MacRemoteTaskProjectionEvent: Sendable {
    let conversationID: String
    let cursor: Int64
    let projection: MacCoreTaskProjection?
}


nonisolated enum MacPermissionGrantScope: String, Sendable {
    case once
    case conversation
}

nonisolated enum MacPermissionMode: String, CaseIterable, Identifiable, Codable, Sendable {
    case cautious
    case allowAll = "allow_all"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .cautious: return String(localized: "Cautious", table: "Permissions")
        case .allowAll: return String(localized: "Allow All")
        }
    }

    var detail: String {
        switch self {
        case .cautious:
            return String(localized: "Confirm deletion, overwrites, sensitive data access, outgoing content, and new connections. Ordinary reads and new files proceed.", table: "Permissions")
        case .allowAll:
            return String(localized: "No Nola approval prompts in this conversation. System permissions still apply.", table: "Permissions")
        }
    }

    var permissionProfile: String {
        rawValue
    }

    var networkPolicy: String { self == .allowAll ? "allow" : "ask" }
    var autoApprovesReversibleWork: Bool { true }

    static func configured(_ value: String?) throws -> Self {
        guard let value else { return .cautious }
        guard let mode = Self(rawValue: value) else {
            throw DecodingError.dataCorrupted(.init(codingPath: [],
                debugDescription: "Permission mode must be cautious or allow_all."))
        }
        return mode
    }
}

nonisolated struct MacRemotePermissionDecision: Sendable {
    let protocolVersion: Int?
    let nonce: String
    let requestID: String
    let hostID: String
    let clientID: String
    let sessionID: String
    let toolName: String
    let policyDigest: String?
    let manifestDigest: String?
    let resolution: MacPermissionResolution

    var payload: [String: Any] {
        var value: [String: Any] = [
            "nonce": nonce,
            "request_id": requestID,
            "host_id": hostID,
            "client_id": clientID,
            "session_id": sessionID,
            "tool_name": toolName,
            "resolution": resolution.payload,
        ]
        if let protocolVersion { value["protocol_version"] = protocolVersion }
        return value
    }
}

enum MacRemoteHostError: LocalizedError {
    case notConfigured
    case invalidResponse
    case http(Int, String)
    case invalidEnvelope
    case transportInterrupted

    var errorDescription: String? {
        switch self {
        case .notConfigured:
            return String(localized: "Configure a connected host in Settings before using this execution location.")
        case .invalidResponse:
            return String(localized: "The connected host returned an invalid response.")
        case .http(let status, let message):
            return message.isEmpty
                ? String(
                    format: String(localized: "Connected host returned HTTP %d."),
                    status
                )
                : message
        case .invalidEnvelope:
            return String(localized: "The connected host returned an invalid encrypted event.")
        case .transportInterrupted:
            return String(localized: "The Nola Server stream ended before the run reached a terminal state.")
        }
    }
}

nonisolated struct MacRemoteExecutionBackend: Identifiable, Hashable, Sendable, Decodable {
    struct Capabilities: Hashable, Sendable, Decodable {
        let supportsPermissionBridge: Bool
        private enum CodingKeys: String, CodingKey { case supportsPermissionBridge = "supports_permission_bridge" }
    }
    struct Model: Identifiable, Hashable, Sendable, Decodable {
        let id: String
        let displayName: String
        let providerID: String?

        private enum CodingKeys: String, CodingKey {
            case id
            case displayName = "display_name"
            case providerID = "provider_id"
        }
    }

    let id: String
    let displayName: String
    let availability: String
    let version: String?
    let diagnostic: String?
    let models: [Model]
    let defaultModelID: String?
    let capabilities: Capabilities?

    var isAvailable: Bool { availability == "available" }

    private enum CodingKeys: String, CodingKey {
        case id
        case displayName = "display_name"
        case availability
        case version
        case diagnostic
        case models
        case capabilities
        case defaultModelID = "default_model_id"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let id = try container.decode(String.self, forKey: .id)
        self.init(
            id: id,
            displayName: try container.decodeIfPresent(String.self, forKey: .displayName) ?? id,
            availability: try container.decodeIfPresent(String.self, forKey: .availability)
                ?? "unavailable",
            version: try container.decodeIfPresent(String.self, forKey: .version),
            diagnostic: try container.decodeIfPresent(String.self, forKey: .diagnostic),
            models: try container.decodeIfPresent([Model].self, forKey: .models) ?? [],
            defaultModelID: try container.decodeIfPresent(String.self, forKey: .defaultModelID),
            capabilities: try container.decode(Capabilities.self, forKey: .capabilities)
        )
    }

    init(
        id: String,
        displayName: String,
        availability: String,
        version: String?,
        diagnostic: String?,
        models: [Model] = [],
        defaultModelID: String? = nil,
        capabilities: Capabilities? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.availability = availability
        self.version = version
        self.diagnostic = diagnostic
        self.models = models
        self.defaultModelID = defaultModelID
        self.capabilities = capabilities
    }
}

nonisolated func macRemoteHostUserFacingError(_ error: Error) -> String {
    if let urlError = error as? URLError {
        switch urlError.code {
        case .timedOut:
            return String(localized: "mac.host.error.timeout")
        case .cannotFindHost, .cannotConnectToHost, .networkConnectionLost,
             .notConnectedToInternet, .dnsLookupFailed, .secureConnectionFailed:
            return String(localized: "mac.host.error.unreachable")
        default:
            break
        }
    }
    if case MacRemoteHostError.http(let status, _) = error {
        if status == 401 || status == 403 {
            return String(localized: "mac.host.error.unauthorized")
        }
        return String(
            format: String(localized: "mac.host.error.http"),
            locale: .current,
            status
        )
    }
    return error.localizedDescription
}

nonisolated final class MacRemoteHostConfigurationStore: @unchecked Sendable {
    static let shared = MacRemoteHostConfigurationStore()

    private let keychainService = "com.gaojc.Nola.mac.connected-host"
    private let keychainAccount = "access-token"
    fileprivate let applicationPort: MacApplicationPort
    private let testCredentialLock = NSLock()
    private var testToken: String?

    init(applicationPort: MacApplicationPort = MacApplicationPort.shared) {
        self.applicationPort = applicationPort
    }

    var isConfigured: Bool {
        guard let rawURL = storedConfiguration()?.url,
              let url = URL(string: rawURL),
              let scheme = url.scheme?.lowercased() else { return false }
        return scheme == "http" || scheme == "https"
    }

    func load() -> MacRemoteHostConfiguration? {
        guard let stored = storedConfiguration(),
              let url = URL(string: stored.url),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let token = loadToken(), !token.isEmpty else {
            return nil
        }
        return MacRemoteHostConfiguration(
            baseURL: url,
            token: token,
            expectedHostID: stored.hostID
        )
    }

    func displayValues() -> (url: String, token: String, hostID: String) {
        (
            storedConfiguration()?.url ?? "",
            loadToken() ?? "",
            storedConfiguration()?.hostID ?? ""
        )
    }

    func save(url: String, token: String, hostID: String? = nil) throws {
        let normalized = url.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedHostID = hostID?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if let previous = storedConfiguration()?.url, let url = URL(string: previous) {
            NolaTailcatTransport.unregister(baseURL: url)
        }
        if normalized.isEmpty {
            _ = try applicationPort.deleteProductSetting(key: MacProductSettingKeys.connectedHostConfiguration)
        } else {
            let existingClientID = storedConfiguration()?.clientID
            try put(Configuration(
                schemaVersion: 1,
                url: normalized,
                hostID: normalizedHostID.isEmpty ? nil : normalizedHostID,
                clientID: existingClientID
            ), in: applicationPort)
        }
        saveToken(token.trimmingCharacters(in: .whitespacesAndNewlines))
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    func clientID() throws -> String {
        if let existing = storedConfiguration()?.clientID, !existing.isEmpty { return existing }
        let generated = UUID().uuidString.lowercased()
        let existing = storedConfiguration()
        try put(Configuration(
            schemaVersion: 1,
            url: existing?.url ?? "",
            hostID: existing?.hostID,
            clientID: generated
        ), in: applicationPort)
        return generated
    }

    private func storedConfiguration() -> Configuration? {
        guard let setting = try? applicationPort.productSetting(key: MacProductSettingKeys.connectedHostConfiguration),
              let data = setting.valueJSON.data(using: .utf8),
              let value = try? JSONDecoder().decode(Configuration.self, from: data),
              value.schemaVersion == 1 else { return nil }
        return value
    }

    private func put(_ configuration: Configuration, in store: MacApplicationPort) throws {
        let data = try JSONEncoder().encode(configuration)
        guard let valueJSON = String(data: data, encoding: .utf8) else {
            throw ConfigurationError.invalidEncodedState
        }
        _ = try store.putProductSetting(
            key: MacProductSettingKeys.connectedHostConfiguration,
            valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }

    private struct Configuration: Codable {
        let schemaVersion: Int
        let url: String
        let hostID: String?
        let clientID: String?
    }

    private enum ConfigurationError: LocalizedError {
        case invalidEncodedState

        var errorDescription: String? {
            switch self {
            case .invalidEncodedState:
                return "The connected-host configuration could not be encoded."
            }
        }
    }

    private func loadToken() -> String? {
        if MacProcessRuntimePolicy.usesEphemeralCredentials {
            testCredentialLock.lock()
            defer { testCredentialLock.unlock() }
            return testToken
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private func saveToken(_ token: String) {
        if MacProcessRuntimePolicy.usesEphemeralCredentials {
            testCredentialLock.lock()
            testToken = token.isEmpty ? nil : token
            testCredentialLock.unlock()
            return
        }
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
        ]
        SecItemDelete(base as CFDictionary)
        guard !token.isEmpty else { return }
        var item = base
        item[kSecValueData as String] = Data(token.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        SecItemAdd(item as CFDictionary, nil)
    }
}

protocol MacRemoteHostManaging: Sendable {
    func isConfigured() async -> Bool
    func testSavedConnection() async throws -> MacRemoteHostConnectionReport
    func requestWebManagementURL(target: String) async throws -> URL
    func fetchExtensions() async throws -> [MacExtensionItem]
    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend]
    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot
}

extension MacRemoteHostManaging {
    func requestWebManagementURL() async throws -> URL {
        try await requestWebManagementURL(target: "/settings#channels")
    }
}

nonisolated struct MacUIQueueHead: Decodable, Sendable {
    let command_id: String
    let kind: String
    let user_message_id: String?
    let lease_until_ms: Int64?
}

nonisolated struct MacUIQueueState: Decodable, Sendable {
    let head: MacUIQueueHead?
    let dispatch: MacSourceSubmissionReceipt?
    private enum CodingKeys: String, CodingKey { case head, dispatch }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        guard fields.contains(.head), fields.contains(.dispatch) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Queue state requires head and dispatch"))
        }
        head = try fields.decodeIfPresent(MacUIQueueHead.self, forKey: .head)
        dispatch = try fields.decodeIfPresent(MacSourceSubmissionReceipt.self, forKey: .dispatch)
    }
    func validate(hostID: String, clientID: String, conversationID: String) throws {
        guard let receipt = dispatch else { return }
        guard let head, receipt.protocol_version == 1,
              receipt.identity.source_host_id == hostID, receipt.identity.client_id == clientID,
              receipt.identity.conversation_id == conversationID, receipt.identity.submission_id == head.command_id,
              receipt.identity.user_message_id == head.user_message_id,
              !receipt.request_digest.isEmpty,
              [.prepared, .admitted, .rejected, .cancelled].contains(receipt.phase) else { throw MacRemoteHostError.invalidResponse }
    }
}

nonisolated struct MacRemoteConversationProgress: Decodable, Sendable {
    let source_host_id: String
    let conversation_id: String
    let goal_projection: MacCanonicalJSONValue
    let plan: MacCanonicalConversationPlan?
}

protocol MacRemoteAgentRunning: MacRemoteHostManaging {
    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws

    func conversationProgress(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacRemoteConversationProgress
    func goalControl(_ intent: AppleGoalControlIntent, target: MacRemoteSubmissionTarget) async throws -> AppleGoalControlResponse

    func previewUISurface(_ input: AppleUIPreviewInput<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async throws -> AppleUIPreviewResult<MacCanonicalJSONValue>
    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?, target: MacRemoteSubmissionTarget) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue>
    func queuedUIInput(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacUIQueueState
    func cancelQueuedUIInput(conversationID: String, commandID: String, requestDigest: String, target: MacRemoteSubmissionTarget) async throws
    func listUISurfaces(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> [AppleUISnapshot<MacCanonicalJSONValue>]
    func readUISurfaceRevision(_ reference: AppleUISurfaceReference, target: MacRemoteSubmissionTarget) async throws -> AppleUISnapshot<MacCanonicalJSONValue>?
    func submitUIInteraction(_ input: AppleUISubmission<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue>

    func prepareRoomRequest(snapshot: MacRoomRemoteRequestPreparation, modelIntent: MacRemoteModelIntent,
        modelID: String, modelProviderID: String?, memoryEnabled: Bool) async throws -> MacRemoteSubmissionIntent
    func runBoundRoomRequest(identity: MacRemoteSubmissionIdentity, digest: String,
        target: MacRemoteSubmissionTarget) async -> AsyncThrowingStream<MacLocalAgentEvent, Error>
    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget
    func permissionConfiguration(identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
                                 expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?) async throws -> MacRemotePermissionConfiguration
    func refreshSubmission(conversationID: String, target: MacRemoteSubmissionTarget, cancel: Bool, expectedIdentity: MacRemoteSubmissionIdentity?) async throws -> MacRemoteSubmissionIntent?
    func observeRun(conversationID: String, expectedRunID: String?, assistantMessageID: String?,
                    target: MacRemoteSubmissionTarget) async throws -> MacRemoteRunObservationBatch
    func resume(conversationID: String, expectedRunID: String) async -> AsyncThrowingStream<MacLocalAgentEvent, Error>
    func run(
        conversationID: String,
        submissionID: String,
        userMessageID: String,
        assistantMessageID: String,
        prompt: String,
        contextComponents: [MacCoreContextComponent],
        modelID: String,
        modelProviderID: String?,
        modelIntent: MacRemoteModelIntent?,
        memoryEnabled: Bool,
        executionBackend: String,
        externalSessionID: String?,
        submission: MacRemoteSubmissionControl
    ) async -> AsyncThrowingStream<MacLocalAgentEvent, Error>
    func resolvePermission(
        request: MacRemotePermissionRequest,
        allow: Bool,
        scope: MacPermissionGrantScope
    ) async throws
    func providePermissionInstruction(request: MacRemotePermissionRequest, instruction: String) async throws
    func sendTaskGuidance(
        conversationID: String,
        parentRunID: String,
        taskID: String,
        message: String,
        target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskGuidanceReceipt
    func requestTaskCancellation(
        conversationID: String,
        parentRunID: String,
        taskID: String,
        target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskCancellationReceipt
    func cancelRun(identity: MacRemoteSubmissionIdentity, requestDigest: String) async throws
    func taskEvents(
        conversationID: String,
        afterCursor: Int64
    ) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error>
}

extension MacRemoteAgentRunning {
    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws { throw MacRemoteHostError.invalidResponse }

    func conversationProgress(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacRemoteConversationProgress { throw MacRemoteHostError.invalidResponse }
    func goalControl(_ intent: AppleGoalControlIntent, target: MacRemoteSubmissionTarget) async throws -> AppleGoalControlResponse { throw MacRemoteHostError.invalidResponse }

    func previewUISurface(_ input: AppleUIPreviewInput<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async throws -> AppleUIPreviewResult<MacCanonicalJSONValue> { throw MacRemoteHostError.invalidResponse }
    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?, target: MacRemoteSubmissionTarget) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue> { throw MacRemoteHostError.invalidResponse }
    func queuedUIInput(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacUIQueueState { throw MacRemoteHostError.invalidResponse }
    func cancelQueuedUIInput(conversationID: String, commandID: String, requestDigest: String, target: MacRemoteSubmissionTarget) async throws { throw MacRemoteHostError.invalidResponse }
    func listUISurfaces(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> [AppleUISnapshot<MacCanonicalJSONValue>] { throw MacRemoteHostError.invalidResponse }
    func readUISurfaceRevision(_ reference: AppleUISurfaceReference, target: MacRemoteSubmissionTarget) async throws -> AppleUISnapshot<MacCanonicalJSONValue>? { throw MacRemoteHostError.invalidResponse }
    func submitUIInteraction(_ input: AppleUISubmission<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue> { .rejected("此 Host 不支持表单交互。") }

    func prepareRoomRequest(snapshot: MacRoomRemoteRequestPreparation, modelIntent: MacRemoteModelIntent,
        modelID: String, modelProviderID: String?, memoryEnabled: Bool) async throws -> MacRemoteSubmissionIntent {
        throw MacRemoteHostError.invalidResponse
    }
    func runBoundRoomRequest(identity: MacRemoteSubmissionIdentity, digest: String,
        target: MacRemoteSubmissionTarget) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: MacRemoteHostError.invalidResponse) }
    }
}

actor MacRemoteHostService: MacRemoteAgentRunning {
    static let shared = MacRemoteHostService()

    private let configurationStore: MacRemoteHostConfigurationStore
    private let cryptoAAD = Data("nola-mobile-bridge-v1".utf8)
    private var activeToolPlans: [String: MacRemoteHostToolPlan] = [:]
    private struct ToolPlanOwner: Equatable {
        let runID: String
        let streamID = UUID()
    }
    private var activeToolPlanOwners: [String: ToolPlanOwner] = [:]
    private var runTargets: [String: MacRemoteSubmissionTarget] = [:]
    private var observedPermissions: [String: (request: MacRemotePermissionRequest, target: MacRemoteSubmissionTarget, runID: String, assistantID: String)] = [:]
    private var observedRunTargets: [String: (runID: String, target: MacRemoteSubmissionTarget)] = [:]

    private func configuration(for conversationID: String) -> MacRemoteHostConfiguration? {
        runTargets[conversationID]?.configuration ?? configurationStore.load()
    }

    private func retireToolPlan(conversationID: String, owner: ToolPlanOwner?) {
        guard let owner, activeToolPlanOwners[conversationID] == owner else { return }
        activeToolPlans.removeValue(forKey: conversationID)
        activeToolPlanOwners.removeValue(forKey: conversationID)
    }

    private func terminalRetiresToolPlan(_ event: MacLocalAgentEvent, conversationID: String,
                                        owner: ToolPlanOwner?, hostID: String?) -> Bool {
        if event.assistantRunTerminalSealDigest != nil {
            guard let owner, event.canonicalRunID == owner.runID,
                  activeToolPlanOwners[conversationID] == owner,
                  hostID == configuration(for: conversationID)?.expectedHostID,
                  let terminal = try? configurationStore.applicationPort.canonicalRunTerminal(
                    conversationID: conversationID, runID: owner.runID),
                  terminal.conversationID == conversationID, terminal.runID == owner.runID else { return false }
            retireToolPlan(conversationID: conversationID, owner: owner)
            return true
        }
        return false
    }

    init(configurationStore: MacRemoteHostConfigurationStore = .shared) {
        self.configurationStore = configurationStore
    }

    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget {
        guard let configuration = configurationStore.load() else { throw MacRemoteHostError.notConfigured }
        return MacRemoteSubmissionTarget(configuration: configuration, clientID: try configurationStore.clientID())
    }

    func isConfigured() async -> Bool {
        configurationStore.isConfigured
    }

    func testSavedConnection() async throws -> MacRemoteHostConnectionReport {
        guard let configuration = configurationStore.load() else {
            throw MacRemoteHostError.notConfigured
        }
        return try await testConnection(configuration: configuration)
    }

    func requestWebManagementURL(target: String = "/settings#channels") async throws -> URL {
        guard let configuration = configurationStore.load() else {
            throw MacRemoteHostError.notConfigured
        }
        return try await requestWebManagementURL(target: target, configuration: configuration)
    }

    /// Model management is a separate Host owner, never the native Mac account store.
    /// Capture pairing once so a settings change during identity verification cannot retarget it.
    func requestModelManagementURL(configuration: MacRemoteHostConfiguration) async throws -> URL {
        guard configuration.expectedHostID != nil else { throw MacRemoteHostError.notConfigured }
        _ = try await testConnection(configuration: configuration)
        try Task.checkCancellation()
        let url = try await requestWebManagementURL(target: "/models?tab=api", configuration: configuration)
        try Task.checkCancellation()
        guard let current = configurationStore.load(),
              current.expectedHostID == configuration.expectedHostID,
              current.baseURL == configuration.baseURL,
              current.token == configuration.token else { throw MacRemoteHostError.notConfigured }
        return url
    }

    private func requestWebManagementURL(target: String, configuration: MacRemoteHostConfiguration) async throws -> URL {
        let envelope = try encrypt(["target": target], token: configuration.token)
        let body = try JSONSerialization.data(withJSONObject: envelope)
        var request = URLRequest(url: mobileURL(baseURL: configuration.baseURL, path: "web-handoff"))
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, String(decoding: data.prefix(1_000), as: UTF8.self))
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let object = root["type"] as? String == "encrypted"
            ? try decrypt(root, token: configuration.token)
            : root
        guard let path = object["path"] as? String,
              let url = macRemoteHostControlPlaneURL(baseURL: configuration.baseURL, path: path) else {
            throw MacRemoteHostError.invalidResponse
        }
        return try await NolaTailcatTransport.browserURL(for: url)
    }

    func testConnection(
        configuration: MacRemoteHostConfiguration
    ) async throws -> MacRemoteHostConnectionReport {
        let settings = try await fetchSettingsObject(
            configuration: configuration,
            timeoutInterval: 8
        )
        guard let identity = settings["host_identity"] as? [String: Any],
              let hostID = identity["id"] as? String,
              !hostID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw MacCoreToolPolicyError.invalidManifest(
                String(localized: "The connected host did not provide a stable host identity.")
            )
        }
        if let expectedHostID = configuration.expectedHostID,
           expectedHostID != hostID {
            throw MacCoreToolPolicyError.invalidManifest(
                String(
                    localized: "The Nola Server identity changed. Remove it and pair again before continuing.",
                    table: "ServerCenter"
                )
            )
        }
        let displayName = [
            identity["display_name"] as? String,
            identity["name"] as? String,
            hostID,
        ]
        .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
        .first(where: { !$0.isEmpty }) ?? hostID
        let messaging = settings["messaging"] as? [String: Any]
        let worker = messaging?["worker"] as? [String: Any]
        let channels = messaging?["channels"] as? [[String: Any]] ?? []
        let managementURL = (messaging?["management_path"] as? String).flatMap { path in
            macRemoteHostControlPlaneURL(baseURL: configuration.baseURL, path: path)
        }
        return MacRemoteHostConnectionReport(
            hostID: hostID,
            displayName: displayName,
            origin: configuration.baseURL.host ?? configuration.baseURL.absoluteString,
            checkedAt: .now,
            messagingWorkerEnabled: worker?["enabled"] as? Bool ?? false,
            messagingChannelCount: channels.count,
            messagingConnectedCount: channels.filter {
                ($0["enabled"] as? Bool ?? false) && ($0["state"] as? String) == "connected"
            }.count,
            messagingManagementURL: managementURL
        )
    }

    func fetchExtensions() async throws -> [MacExtensionItem] {
        guard let configuration = configurationStore.load() else {
            throw MacRemoteHostError.notConfigured
        }
        var request = URLRequest(url: mobileURL(baseURL: configuration.baseURL, path: "agent/settings"))
        request.httpMethod = "GET"
        request.timeoutInterval = 20
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw MacRemoteHostError.invalidResponse }
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, String(decoding: data.prefix(1_000), as: UTF8.self))
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let object = root["type"] as? String == "encrypted"
            ? try decrypt(root, token: configuration.token)
            : root
        var items: [MacExtensionItem] = []

        func appendItems(_ key: String, kind: MacExtensionItem.Kind, fallback: String) {
            guard let values = object[key] as? [[String: Any]] else { return }
            for (index, value) in values.enumerated() {
                let title = value["display_name"] as? String
                    ?? value["name"] as? String
                    ?? value["title"] as? String
                    ?? value["server_id"] as? String
                    ?? value["id"] as? String
                    ?? "\(fallback) \(index + 1)"
                let detail = value["description"] as? String
                    ?? value["endpoint"] as? String
                    ?? value["url"] as? String
                    ?? "Managed by Connected Host"
                let mcpSummary: MacMCPCapabilitySummary? = kind == .mcp
                    ? MacMCPCapabilitySummary(
                        state: {
                            switch (value["availability"] as? String)?.lowercased() {
                            case "available": return .connected
                            case "authorization_required": return .authorizationRequired
                            case "unavailable": return .unavailable
                            default: return value["protocol_version"] == nil ? .configured : .connected
                            }
                        }(),
                        protocolVersion: value["protocol_version"] as? String,
                        tools: (value["tools"] as? [String: Any])?.keys.sorted() ?? [],
                        resources: (value["resources"] as? [String: Any])?.keys.sorted() ?? [],
                        resourceTemplates: (value["resource_templates"] as? [String: Any])?.keys.sorted() ?? [],
                        prompts: (value["prompts"] as? [String: Any])?.keys.sorted() ?? []
                    )
                    : nil
                items.append(MacExtensionItem(
                    id: "host:\(kind.rawValue):\(value["id"] as? String ?? title):\(index)",
                    kind: kind,
                    source: .connectedHost,
                    title: title,
                    detail: detail,
                    path: nil,
                    isEnabled: value["enabled"] as? Bool ?? true,
                    mcpSummary: mcpSummary
                ))
            }
        }

        appendItems("uploaded_skills", kind: .skill, fallback: "Skill")
        appendItems("uploaded_documents", kind: .document, fallback: "Document")
        appendItems("mcp_servers", kind: .mcp, fallback: "MCP Server")

        for (key, title, symbolDetail) in [
            ("computer_use", "Computer Use", "Host computer interaction"),
            ("native_browser", "Native Browser", "Host browser bridge"),
        ] {
            guard let capability = object[key] as? [String: Any] else { continue }
            let enabled = capability["connected"] as? Bool
                ?? capability["feature_enabled"] as? Bool
                ?? false
            items.append(MacExtensionItem(
                id: "host:capability:\(key)",
                kind: .capability,
                source: .connectedHost,
                title: title,
                detail: symbolDetail,
                path: nil,
                isEnabled: enabled
            ))
        }
        return items
    }

    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend] {
        guard let configuration = configurationStore.load() else {
            throw MacRemoteHostError.notConfigured
        }
        let object = try await fetchSettingsObject(configuration: configuration)
        guard let values = object["execution_backends"] as? [[String: Any]] else {
            throw MacRemoteHostError.invalidResponse
        }
        return try JSONDecoder().decode([MacRemoteExecutionBackend].self,
            from: JSONSerialization.data(withJSONObject: values))
    }

    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot {
        guard let configuration = configurationStore.load() else {
            throw MacRemoteHostError.notConfigured
        }
        return try await fetchRemoteModelCatalog(configuration: configuration)
    }

    func fetchRemoteModelCatalog(configuration: MacRemoteHostConfiguration) async throws -> MacRemoteModelCatalogSnapshot {
        try MacRemoteModelCatalog.decodeSnapshot(
            settings: await fetchSettingsObject(configuration: configuration),
            expectedHostID: configuration.expectedHostID
        )
    }

    private func fetchSettingsObject(
        configuration: MacRemoteHostConfiguration,
        timeoutInterval: TimeInterval = 20
    ) async throws -> [String: Any] {
        var request = URLRequest(
            url: mobileURL(baseURL: configuration.baseURL, path: "agent/settings")
        )
        request.httpMethod = "GET"
        request.timeoutInterval = timeoutInterval
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, String(decoding: data.prefix(1_000), as: UTF8.self))
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        return root["type"] as? String == "encrypted"
            ? try decrypt(root, token: configuration.token)
            : root
    }

    /// Catch up only to this committed notification. Subsequent source output
    /// cannot move the read window or become an implicit completion condition.
    private func synchronizeAssistantJournal(
        through: MacAssistantJournalCursor, conversationID: String, configuration: MacRemoteHostConfiguration,
        application: MacApplicationPort
    ) async throws -> MacAssistantJournalCursor {
        guard let sourceHostID = configuration.expectedHostID,
              through.identity.conversation_id == conversationID,
              let limit = UInt64(through.sequence) else { throw MacRemoteHostError.invalidResponse }
        var state = try application.assistantJournalReceptionState(sourceHostID: sourceHostID, through: through)
        while !state.complete {
            try Task.checkCancellation()
            let result: MacAssistantJournalPage = try await journalRequest(.records(after: state.after, through: through), configuration: configuration)
            guard case .records(let page) = result else { throw MacAssistantJournalCatchUpError.snapshotRequired }
            guard page.after == state.after, page.through == through,
                  !page.records.isEmpty, page.records.count <= 32,
                  page.next == page.records.last?.cursor,
                  page.complete == (page.next == through),
                  let next = UInt64(page.next.sequence), next <= limit,
                  let start = UInt64(state.after.sequence), next > start,
                  page.records.allSatisfy({ record in
                      guard record.header.identity == through.identity, let sequence = UInt64(record.header.sequence) else { return false }
                      return sequence > start && sequence <= limit
                  }) else {
                throw MacRemoteHostError.invalidResponse
            }
            for record in page.records {
                _ = try await ingestAssistantJournalRecord(record, sourceHostID: sourceHostID,
                    configuration: configuration, application: application)
            }
            let previous = state.after
            state = try application.assistantJournalReceptionState(sourceHostID: sourceHostID, through: through)
            guard state.complete || state.after != previous else { throw MacRemoteHostError.invalidResponse }
        }
        return state.after
    }

    /// One committed record at a time. No operation parsing, body concatenation,
    /// in-memory recovery cursor or permission inference belongs in this adapter.
    private func ingestAssistantJournalRecord(
        _ record: MacAssistantJournalRecord, sourceHostID: String,
        configuration: MacRemoteHostConfiguration, application: MacApplicationPort
    ) async throws -> MacAssistantJournalReceipt {
        // Freeze this target for the whole transfer, including cleanup. A settings
        // change while awaiting a page must not redirect an existing source lease.
        var need = try application.nextAssistantJournalContent(sourceHostID: sourceHostID, record: record)
        var snapshot: MacAssistantJournalSnapshot?
        do {
            if need != nil {
                let opened: MacAssistantJournalSnapshot = try await journalRequest(.open(record.cursor), configuration: configuration)
                guard opened.at == record.cursor else { throw MacRemoteHostError.invalidResponse }
                snapshot = opened
            }
            while let current = need {
                try Task.checkCancellation()
                guard let snapshot else { throw MacRemoteHostError.invalidResponse }
                let chunk: MacAssistantJournalChunk = try await journalRequest(.chunk(snapshot, current), configuration: configuration)
                try application.stageAssistantJournalContent(sourceHostID: sourceHostID, record: record, need: current, chunk: chunk)
                need = try application.nextAssistantJournalContent(sourceHostID: sourceHostID, record: record)
                guard need != current else { throw MacRemoteHostError.invalidResponse }
            }
            try Task.checkCancellation()
            let receipt = try application.receiveAssistantJournalRecord(sourceHostID: sourceHostID, record: record)
            guard receipt.cursor == record.cursor else { throw MacRemoteHostError.invalidResponse }
            if let snapshot {
                let _: MacCanonicalJSONValue? = try? await journalRequest(.release(snapshot), configuration: configuration)
            }
            return receipt
        } catch {
            if let snapshot {
                let _: MacCanonicalJSONValue? = try? await journalRequest(.release(snapshot), configuration: configuration)
            }
            throw error
        }
    }

    private func journalRequest<Result: Decodable & Sendable>(
        _ input: MacAssistantJournalReadRequest, configuration: MacRemoteHostConfiguration
    ) async throws -> Result {
        let inputData = try JSONEncoder().encode(input)
        guard inputData.count <= 64 * 1024,
              let payload = try JSONSerialization.jsonObject(with: inputData) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        return try await authenticatedRead(payload, path: "chat/agent/v2/journal", configuration: configuration)
    }

    private func authenticatedRead<Result: Decodable & Sendable>(_ payload: [String: Any], path: String,
        configuration: MacRemoteHostConfiguration, responseByteLimit: Int = 512 * 1024, requestByteLimit: Int = 64 * 1024) async throws -> Result {
        guard try JSONSerialization.data(withJSONObject: payload).count <= requestByteLimit else { throw MacRemoteHostError.invalidResponse }
        guard let expectedHostID = configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        var request = URLRequest(url: mobileURL(baseURL: configuration.baseURL, path: path))
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.httpBody = try JSONSerialization.data(withJSONObject: encrypt(payload, token: configuration.token))
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (bytes, response) = try await NolaTailcatTransport.session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else { throw MacRemoteHostError.invalidResponse }
        var data = Data()
        for try await byte in bytes {
            guard data.count < responseByteLimit else { throw MacRemoteHostError.invalidResponse }
            data.append(byte)
        }
        guard let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              envelope["type"] as? String == "encrypted" else { throw MacRemoteHostError.invalidResponse }
        let object = try decrypt(envelope, token: configuration.token)
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, object["error"] as? String ?? "Journal request failed")
        }
        guard object["source_host_id"] as? String == expectedHostID, let result = object["result"] else {
            throw MacRemoteHostError.invalidResponse
        }
        return try JSONDecoder().decode(Result.self, from: JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]))
    }

    func queuedUIInput(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacUIQueueState {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        guard let epoch = target.expectedSourceStoreEpoch, !epoch.isEmpty else { throw MacRemoteHostError.invalidResponse }
        let state: MacUIQueueState = try await authenticatedRead(["expected_host_id": host, "expected_source_store_epoch": epoch, "conversation_id": conversationID,
            "client_id": target.clientID], path: "chat/agent/v2/ui/queued-input", configuration: target.configuration)
        try state.validate(hostID: host, clientID: target.clientID, conversationID: conversationID)
        return state
    }

    func cancelQueuedUIInput(conversationID: String, commandID: String, requestDigest: String, target: MacRemoteSubmissionTarget) async throws {
        nonisolated struct Result: Decodable, Sendable { let cancelled: Bool; let command_id: String }
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        guard let epoch = target.expectedSourceStoreEpoch, !epoch.isEmpty else { throw MacRemoteHostError.invalidResponse }
        let result: Result = try await authenticatedRead(["expected_host_id": host, "expected_source_store_epoch": epoch, "conversation_id": conversationID,
            "client_id": target.clientID, "command_id": commandID, "request_digest": requestDigest],
            path: "chat/agent/v2/ui/queued-input/cancel", configuration: target.configuration)
        guard result.cancelled, result.command_id == commandID else { throw MacRemoteHostError.invalidResponse }
    }

    func listUISurfaces(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> [AppleUISnapshot<MacCanonicalJSONValue>] {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        guard let epoch = target.expectedSourceStoreEpoch, !epoch.isEmpty else { throw MacRemoteHostError.invalidResponse }
        let snapshots: [AppleUISnapshot<MacCanonicalJSONValue>] = try await authenticatedRead(
            ["expected_host_id": host, "expected_source_store_epoch": epoch, "conversation_id": conversationID], path: "chat/agent/v2/ui/surfaces",
            configuration: target.configuration, responseByteLimit: 16 * 1024 * 1024)
        guard snapshots.count <= 8, snapshots.allSatisfy({ $0.identity.owner.conversation_id == conversationID && $0.identity.owner.authority_id == epoch }) else { throw MacRemoteHostError.invalidResponse }
        return snapshots
    }

    func previewUISurface(_ input: AppleUIPreviewInput<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async throws -> AppleUIPreviewResult<MacCanonicalJSONValue> {
        guard let host = target.configuration.expectedHostID,
              var payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as? [String: Any] else { throw MacRemoteHostError.invalidResponse }
        payload["expected_host_id"] = host
        return try await authenticatedRead(payload, path: "chat/agent/v2/ui/preview",
            configuration: target.configuration, responseByteLimit: 2 * 1024 * 1024)
    }

    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?, target: MacRemoteSubmissionTarget) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue> {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        let owner = try JSONSerialization.jsonObject(with: JSONEncoder().encode(identity))
        return try await authenticatedRead(["expected_host_id": host, "identity": owner,
            "base_revision": baseRevision.map { NSNumber(value: $0) as Any } ?? NSNull()],
            path: "chat/agent/v2/ui/changes", configuration: target.configuration, responseByteLimit: 2 * 1024 * 1024)
    }

    func readUISurfaceRevision(_ reference: AppleUISurfaceReference, target: MacRemoteSubmissionTarget) async throws -> AppleUISnapshot<MacCanonicalJSONValue>? {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        let identity = try JSONSerialization.jsonObject(with: JSONEncoder().encode(reference.identity))
        let snapshot: AppleUISnapshot<MacCanonicalJSONValue>? = try await authenticatedRead(
            ["expected_host_id": host, "identity": identity, "revision": reference.revision], path: "chat/agent/v2/ui/revision",
            configuration: target.configuration, responseByteLimit: 2 * 1024 * 1024)
        guard snapshot == nil || (snapshot?.identity == reference.identity && snapshot?.revision == reference.revision
            && (snapshot?.phase == .readOnly || snapshot?.phase == .deleted)) else { throw MacRemoteHostError.invalidResponse }
        return snapshot
    }

    func submitUIInteraction(_ input: AppleUISubmission<MacCanonicalJSONValue>, target: MacRemoteSubmissionTarget) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue> {
        struct Response: Decodable, Sendable {
            let status: String
            let receipt: AppleUIInteractionReceipt<MacCanonicalJSONValue>?
            let message: String?
        }
        guard let host = target.configuration.expectedHostID else { return .rejected("请重新连接会话的 Host。") }
        do {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input))
            let response: Response = try await authenticatedRead(["expected_host_id": host, "input": object],
                path: "chat/agent/v2/ui/interactions", configuration: target.configuration,
                responseByteLimit: 2 * 1024 * 1024, requestByteLimit: 128 * 1024)
            switch response.status {
            case "committed":
                guard let receipt = response.receipt, receipt.snapshot.identity == input.interaction.identity,
                      receipt.snapshot.revision > input.interaction.expected_surface_revision,
                      receipt.snapshot.revision - input.interaction.expected_surface_revision == 1,
                      receipt.queued_submission_id == nil || receipt.queued_submission_id == input.submission_id else {
                    return .unknown("表单提交结果归属或版本不匹配。")
                }
                return .committed(receipt)
            case "rejected", "conflict": return .rejected(response.message ?? "表单已更新，请重新确认。")
            default: return .unknown(response.message ?? "表单提交结果尚未确认。")
            }
        } catch { return .unknown(error.localizedDescription) }
    }

    func permissionConfiguration(identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
                                 expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?) async throws -> MacRemotePermissionConfiguration {
        guard identity.source_host_id == target.configuration.expectedHostID,
              identity.client_id == target.clientID,
              expectedRevision.map({ $0 >= 0 && $0 < Int64.max }) ?? true,
              (expectedRevision == nil) == (profile == nil) else { throw MacRemoteHostError.invalidResponse }
        var payload: [String: Any] = ["operation": profile == nil ? "read" : "patch",
            "expected_host_id": identity.source_host_id, "client_id": identity.client_id,
            "conversation_id": identity.conversation_id, "expected_run_id": identity.submission_id,
            "assistant_message_id": identity.assistant_message_id]
        if let expectedRevision, let profile {
            payload["expected_revision"] = expectedRevision
            payload["permission_profile"] = profile.rawValue
        }
        if let minimumPolicyRevision { payload["minimum_policy_revision"] = minimumPolicyRevision }
        let result: MacRemotePermissionConfiguration = try await authenticatedRead(payload,
            path: "chat/agent/v2/configuration", configuration: target.configuration)
        try result.validate(identity: identity)
        if let expectedRevision, let profile {
            guard result.configuration_revision == expectedRevision + 1,
                  result.permission_profile == profile else { throw MacRemoteHostError.invalidResponse }
        }
        return result
    }

    func observeRun(conversationID: String, expectedRunID: String?, assistantMessageID: String?,
                    target: MacRemoteSubmissionTarget) async throws -> MacRemoteRunObservationBatch {
        let configuration = target.configuration
        guard let hostID = configuration.expectedHostID,
              (expectedRunID == nil) == (assistantMessageID == nil) else { throw MacRemoteHostError.invalidResponse }
        var payload: [String: Any] = ["operation": expectedRunID == nil ? "discover" : "observe",
            "conversation_id": conversationID, "expected_host_id": hostID, "client_id": target.clientID]
        if let expectedRunID, let assistantMessageID {
            payload["expected_run_id"] = expectedRunID
            payload["assistant_message_id"] = assistantMessageID
        }
        let observation: MacRemoteRunObservation? = try await authenticatedRead(payload,
            path: "chat/agent/v2/observe", configuration: configuration)
        try Task.checkCancellation()
        guard let observation else {
            guard expectedRunID == nil else { throw MacRemoteHostError.invalidResponse }
            return MacRemoteRunObservationBatch(observation: nil, events: [])
        }
        try observation.validate(conversationID: conversationID, clientID: target.clientID,
            expectedRunID: expectedRunID, assistantMessageID: assistantMessageID)
        guard let receipt = observation.submission_receipt else { throw MacRemoteHostError.invalidResponse }
        do {
            guard receipt.identity.source_host_id == hostID, receipt.identity.client_id == target.clientID,
                  receipt.identity.conversation_id == conversationID, receipt.identity.submission_id == observation.run_id,
                  receipt.identity.assistant_message_id == observation.assistant_message_id,
                  receipt.phase == .admitted, receipt.admission?.identity == observation.admission?.identity else {
                throw MacRemoteHostError.invalidResponse
            }
            let application = configurationStore.applicationPort
            if try application.remoteSubmissionIntent(conversationID: conversationID) != nil {
                _ = try application.acceptRemoteSubmissionReceipt(receipt)
            } else if try application.canonicalRunTerminal(conversationID: conversationID, runID: observation.run_id) == nil {
                try application.adoptObservedRemoteRun(receipt)
            }
        }
        try await synchronizeRunMessageAdmissions(conversationID: conversationID, runID: observation.run_id,
            through: observation.admission, configuration: configuration)
        if let usage = observation.context_usage {
            try await receiveContextUsage(usage, conversationID: conversationID, runID: observation.run_id,
                configuration: configuration)
        }
        let cursors = try await synchronizeRunAssistantJournals(conversationID: conversationID,
            runID: observation.run_id, configuration: configuration)
        if let seal = observation.terminal_seal {
            guard let terminal = try configurationStore.applicationPort.canonicalRunTerminal(
                conversationID: conversationID, runID: observation.run_id),
                  terminal.assistantMessageID == observation.assistant_message_id,
                  terminal.status == .sealed, terminal.result == seal.result,
                  terminal.resultDigest == seal.result_digest else { throw MacRemoteHostError.invalidResponse }
            if let owner = activeToolPlanOwners[conversationID], owner.runID == observation.run_id {
                retireToolPlan(conversationID: conversationID, owner: owner)
            }
        }
        var events = cursors.map { cursor in
            MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                assistantJournalCursor: cursor).bindingCanonicalRun(observation.run_id)
        }
        if observation.context_usage != nil {
            events.append(MacLocalAgentEvent(kind: .status, text: "", title: nil,
                detail: "context_usage_committed", isError: false).bindingCanonicalRun(observation.run_id))
        }
        for permission in observation.permissions {
            let encoded = try JSONEncoder().encode(permission)
            guard let object = try JSONSerialization.jsonObject(with: encoded) as? [String: Any],
                  object["type"] as? String == "permission_request",
                  object["host_id"] as? String == hostID,
                  object["client_id"] as? String == target.clientID,
                  object["session_id"] as? String == conversationID,
                  object["canonical_run_id"] as? String == observation.run_id,
                  object["request_id"] as? String == object["id"] as? String,
                  let request = permissionRequest(from: object),
                  request.callID?.isEmpty == false,
                  request.protocolVersion == MacRemotePermissionRequest.supportedProtocolVersion,
                  let expires = request.expiresAt, expires > Int64(Date().timeIntervalSince1970),
                  !request.nonce.isEmpty else { throw MacRemoteHostError.invalidResponse }
            events.append(MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                permissionRequest: request).bindingCanonicalRun(observation.run_id))
        }
        try Task.checkCancellation()
        observedRunTargets[conversationID] = (observation.run_id, target)
        observedPermissions = observedPermissions.filter { $0.value.request.sessionID != conversationID }
        for event in events {
            if let request = event.permissionRequest {
                observedPermissions[request.nonce] = (request, target, observation.run_id, observation.assistant_message_id)
            }
        }
        return MacRemoteRunObservationBatch(observation: observation, events: events)
    }

    private func submissionEvent(_ intent: MacRemoteSubmissionIntent) -> MacLocalAgentEvent {
        if intent.phase == .cancelled || intent.phase == .rejected,
           let owner = activeToolPlanOwners[intent.identity.conversation_id], owner.runID == intent.identity.submission_id {
            retireToolPlan(conversationID: intent.identity.conversation_id, owner: owner)
        }
        return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: "submission_receipt", isError: false)
            .bindingCanonicalRun(intent.phase == .admitted ? intent.identity.submission_id : nil)
            .bindingSubmissionIntent(intent)
    }

    func refreshSubmission(conversationID: String, target: MacRemoteSubmissionTarget, cancel: Bool, expectedIdentity: MacRemoteSubmissionIdentity?) async throws -> MacRemoteSubmissionIntent? {
        let application = configurationStore.applicationPort
        guard let intent = try application.remoteSubmissionIntent(conversationID: conversationID) else { return nil }
        guard expectedIdentity.map({ $0 == intent.identity }) ?? !cancel else { throw MacRemoteHostError.invalidResponse }
        guard intent.identity.source_host_id == target.configuration.expectedHostID,
              intent.identity.client_id == target.clientID else { throw MacRemoteHostError.invalidResponse }
        guard let digest = intent.request_digest else {
            return cancel ? try application.abandonPreparedRemoteSubmission(intent.identity) : intent
        }
        let identity = try JSONSerialization.jsonObject(with: JSONEncoder().encode(intent.identity))
        let receipt: MacSourceSubmissionReceipt? = try await authenticatedRead(
            ["operation":cancel ? "cancel" : "observe", "identity":identity, "request_digest":digest],
            path: "chat/agent/v2/submission", configuration: target.configuration)
        try Task.checkCancellation()
        guard let receipt else { return intent }
        guard receipt.identity == intent.identity, receipt.request_digest == digest else { throw MacRemoteHostError.invalidResponse }
        let accepted = try application.acceptRemoteSubmissionReceipt(receipt)
        if accepted.phase == .cancelled || accepted.phase == .rejected { _ = submissionEvent(accepted) }
        if accepted.phase == .admitted {
            _ = try await synchronizeRunAssistantJournals(conversationID: conversationID,
                runID: intent.identity.submission_id, configuration: target.configuration)
            if cancel { try await cancelRun(identity: intent.identity, requestDigest: digest, configuration: target.configuration) }
        }
        return accepted
    }

    func run(
        conversationID: String,
        submissionID: String,
        userMessageID: String,
        assistantMessageID: String,
        prompt: String,
        contextComponents: [MacCoreContextComponent] = [],
        modelID: String,
        modelProviderID: String?,
        modelIntent: MacRemoteModelIntent?,
        memoryEnabled: Bool,
        executionBackend: String,
        externalSessionID: String?,
        submission: MacRemoteSubmissionControl
    ) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    try await self.start(
                        conversationID: conversationID,
                        submissionID: submissionID,
                        userMessageID: userMessageID,
                        assistantMessageID: assistantMessageID,
                        prompt: prompt,
                        contextComponents: contextComponents,
                        modelID: modelID,
                        modelProviderID: modelProviderID,
                        modelIntent: modelIntent,
                        memoryEnabled: memoryEnabled,
                        executionBackend: executionBackend,
                        externalSessionID: externalSessionID,
                        submission: submission,
                        continuation: continuation
                    )
                } catch {
                    if submission.ticket.phase == .preparing { submission.ticket.rejectBeforeTransport() }
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }


    func synchronizeRunAssistantJournals(conversationID: String, runID: String,
                                                configuration: MacRemoteHostConfiguration) async throws -> [MacAssistantJournalCursor] {
        try await synchronizeRunMessageAdmissions(conversationID: conversationID, runID: runID,
                                                  configuration: configuration)
        var after: String?
        // Terminal recovery must include the immutable completion frontier,
        // not merely whichever heads happen to be current on successive reads.
        // Application admits the terminal only after its required content is
        // durable; neither the HTTP notification nor this query unlocks the UI.
        let seal: MacAssistantRunTerminalSeal? = try await journalRequest(
            .terminalSeal(conversationID: conversationID, runID: runID), configuration: configuration)
        var recovered: [MacAssistantJournalCursor] = []
        if let seal {
            recovered = try await synchronizeSealedAssistantFrontier(seal, conversationID: conversationID,
                runID: runID, configuration: configuration)
        }
        for _ in 0..<64 {
            try Task.checkCancellation()
            let heads: [MacAssistantJournalCursor] = try await journalRequest(
                .heads(conversationID: conversationID, runID: runID, afterMessageID: after), configuration: configuration)
            guard heads.count <= 64 else { throw MacRemoteHostError.invalidResponse }
            for head in heads {
                let messageID = head.identity.assistant_message_id
                guard head.identity.conversation_id == conversationID, head.identity.run_id == runID,
                      !messageID.isEmpty,
                      after.map({ Array($0.utf8).lexicographicallyPrecedes(Array(messageID.utf8)) }) ?? true else {
                    throw MacRemoteHostError.invalidResponse
                }
                _ = try await synchronizeAssistantJournal(through: head, conversationID: conversationID,
                    configuration: configuration, application: configurationStore.applicationPort)
                after = messageID
                recovered.append(head)
            }
            if heads.count < 64 { return recovered }
        }
        throw MacRemoteHostError.invalidResponse
    }

    private func admittedContextRoot(conversationID: String, runID: String) throws -> String? {
        let application = configurationStore.applicationPort
        if let active = try application.activeRunIdentity(conversationID: conversationID), active.runID == runID {
            return active.assistantMessageID
        }
        return try application.canonicalRunTerminal(conversationID: conversationID, runID: runID)?.assistantMessageID
    }

    private func receiveContextUsage(_ record: MacCommittedRunContextUsage, conversationID: String,
        runID: String, configuration: MacRemoteHostConfiguration) async throws {
        let application = configurationStore.applicationPort
        guard let hostID = configuration.expectedHostID,
              record.identity.conversation_id == conversationID, record.identity.run_id == runID,
              record.admission.identity == record.identity,
              try admittedContextRoot(conversationID: conversationID, runID: runID) == record.identity.assistant_message_id else {
            throw MacRemoteHostError.invalidResponse
        }
        try await synchronizeRunMessageAdmissions(conversationID: conversationID, runID: runID,
            through: record.admission, configuration: configuration)
        try Task.checkCancellation()
        _ = try application.acceptRemoteRunContextUsage(sourceHostID: hostID, record: record)
    }

    private func synchronizeRunMessageAdmissions(conversationID: String, runID: String,
        through required: MacRunMessageAdmissionCursor? = nil,
        configuration: MacRemoteHostConfiguration) async throws {
        let through: MacRunMessageAdmissionCursor
        if let required { through = required }
        else {
            through = try await journalRequest(.admissionHead(conversationID: conversationID, runID: runID),
                                               configuration: configuration)
        }
        guard through.identity.conversation_id == conversationID, through.identity.run_id == runID,
              let limit = UInt64(through.sequence), String(limit) == through.sequence,
              let sourceHostID = configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        let application = configurationStore.applicationPort
        var after = try application.runMessageAdmissionReceptionState(sourceHostID: sourceHostID, through: through)
        guard after.identity == through.identity, let start = UInt64(after.sequence) else {
            throw MacRemoteHostError.invalidResponse
        }
        if start >= limit { return } // Core verified the requested historical frontier.
        while after != through {
            try Task.checkCancellation()
            let page: MacRunMessageAdmissionPage = try await journalRequest(
                .admissions(after: after, through: through), configuration: configuration)
            guard page.after == after, page.through == through, page.next.identity == through.identity,
                  let next = UInt64(page.next.sequence), let previous = UInt64(after.sequence),
                  next > previous, next <= limit, !page.records.isEmpty, page.records.count <= 64,
                  UInt64(page.records.count) == next - previous,
                  page.complete == (page.next == through) else { throw MacRemoteHostError.invalidResponse }
            let received = try application.receiveRunMessageAdmissions(sourceHostID: sourceHostID, page: page)
            guard received == page.next else { throw MacRemoteHostError.invalidResponse }
            after = received
        }
    }

    private func synchronizeSealedAssistantFrontier(_ seal: MacAssistantRunTerminalSeal,
        conversationID: String, runID: String,
        configuration: MacRemoteHostConfiguration) async throws -> [MacAssistantJournalCursor] {
        func number(_ value: String) -> UInt64? {
            guard let decoded = UInt64(value), String(decoded) == value else { return nil }
            return decoded
        }
        guard seal.protocol_version == 2,
              seal.identity.conversation_id == conversationID, seal.identity.run_id == runID,
              !seal.core_session_id.isEmpty, !seal.transition_id.isEmpty,
              seal.result.run_id == runID, seal.result.conversation_id == conversationID,
              let total = number(seal.entry_count), let heads = number(seal.heads_count),
              let children = number(seal.child_receipt_count), heads > 0,
              !heads.addingReportingOverflow(children).overflow,
              heads + children == total else { throw MacRemoteHostError.invalidResponse }
        guard let sourceHostID = configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        try await synchronizeRunMessageAdmissions(conversationID: conversationID, runID: runID,
                                                  through: seal.admission, configuration: configuration)
        let application = configurationStore.applicationPort
        var reception = try application.beginReceiveAssistantRunTerminal(sourceHostID: sourceHostID, seal: seal)
        guard reception.seal == seal, let resumedAt = number(reception.next_entry), resumedAt <= total,
              reception.manifest_complete == (resumedAt == total) else { throw MacRemoteHostError.invalidResponse }
        // Stage the complete immutable owner set before receiving its journals.
        // This admits additional assistant owners without inventing message/Run
        // bindings in the client, and survives a disconnect between pages.
        while !reception.manifest_complete {
            try Task.checkCancellation()
            guard let start = number(reception.next_entry) else { throw MacRemoteHostError.invalidResponse }
            let page: MacAssistantRunTerminalManifestPage = try await journalRequest(
                .terminalManifest(seal, afterEntry: reception.next_entry), configuration: configuration)
            guard page.seal == seal, page.after_entry == reception.next_entry,
                  let next = number(page.next_entry), next > start, next <= total,
                  page.entries.count <= 64, UInt64(page.entries.count) == next - start,
                  page.complete == (next == total) else { throw MacRemoteHostError.invalidResponse }
            reception = try application.stageAssistantRunTerminalManifest(sourceHostID: sourceHostID, page: page)
            guard reception.seal == seal, reception.next_entry == page.next_entry,
                  reception.manifest_complete == page.complete else { throw MacRemoteHostError.invalidResponse }
        }
        var after: UInt64 = 0
        var headCount: UInt64 = 0
        var childCount: UInt64 = 0
        var recovered: [MacAssistantJournalCursor] = []
        while after < total {
            try Task.checkCancellation()
            let page: MacAssistantRunTerminalManifestPage = try await journalRequest(
                .terminalManifest(seal, afterEntry: String(after)), configuration: configuration)
            guard page.seal == seal, page.after_entry == String(after),
                  let next = number(page.next_entry), next > after, next <= total,
                  page.entries.count <= 64, UInt64(page.entries.count) == next - after,
                  page.complete == (next == total) else { throw MacRemoteHostError.invalidResponse }
            for entry in page.entries {
                switch entry {
                case .assistant(let through, _, _, _, _):
                    guard through.identity.conversation_id == conversationID,
                          through.identity.run_id == runID,
                          through.identity.source_store_epoch == seal.identity.source_store_epoch else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    _ = try await synchronizeAssistantJournal(through: through, conversationID: conversationID,
                        configuration: configuration, application: configurationStore.applicationPort)
                    recovered.append(through)
                    headCount += 1
                case .childReceipt:
                    // Presentation never decides whether a child is required,
                    // nor synthesizes a local task receipt from remote metadata.
                    childCount += 1
                }
            }
            after = next
        }
        guard headCount == heads, childCount == children else { throw MacRemoteHostError.invalidResponse }
        let committed = try application.commitReceivedAssistantRunTerminal(sourceHostID: sourceHostID, seal: seal)
        guard committed.seal == seal, committed.next_entry == seal.entry_count,
              committed.manifest_complete, committed.committed else { throw MacRemoteHostError.invalidResponse }
        return recovered
    }

    private func recoverTerminalEvent(conversationID: String, runID: String,
        configuration: MacRemoteHostConfiguration, expectedDigest: String? = nil) async throws -> MacLocalAgentEvent? {
        try Task.checkCancellation()
        let seal: MacAssistantRunTerminalSeal? = try await journalRequest(
            .terminalSeal(conversationID: conversationID, runID: runID), configuration: configuration)
        guard let seal else { return nil }
        guard expectedDigest == nil || seal.seal_digest == expectedDigest else { throw MacRemoteHostError.invalidResponse }
        _ = try await synchronizeSealedAssistantFrontier(seal, conversationID: conversationID,
            runID: runID, configuration: configuration)
        let usage: MacCommittedRunContextUsage? = try await journalRequest(.contextUsage(seal.identity), configuration: configuration)
        if let usage {
            try await receiveContextUsage(usage, conversationID: conversationID, runID: runID, configuration: configuration)
        }
        return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
            assistantRunTerminalSealDigest: seal.seal_digest).bindingCanonicalRun(runID)
    }

    private func submissionFrontier(conversationID: String,
        target: MacRemoteSubmissionTarget) async throws -> AppleSubmissionFrontier {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        // Capture the displayed canonical parent before the source read. A newer
        // source parent is a reload requirement, not permission to silently rebase.
        let window = try configurationStore.applicationPort.loadConversationWindow(
            conversationID, limit: 1, contentByteLimit: 1)
        let parent = window.projection.messages.last?.messageID
        let identity = window.assistantJournalHeads.last?.identity
        guard parent == nil || identity?.assistant_message_id == parent else {
            throw MacRemoteHostError.invalidResponse
        }
        let admittedAssistant: String?
        if let identity {
            // A continuation journal can be the visible tail; source authority
            // remains the original admitted assistant, not that continuation.
            guard let terminal = try configurationStore.applicationPort.canonicalRunTerminal(
                conversationID: conversationID, runID: identity.run_id) else {
                throw MacRemoteHostError.invalidResponse
            }
            admittedAssistant = terminal.assistantMessageID
        } else { admittedAssistant = nil }
        let frontier: AppleSubmissionFrontier = try await authenticatedRead([
            "expected_host_id": host, "client_id": target.clientID, "conversation_id": conversationID,
            "expected_run_id": identity?.run_id as Any? ?? NSNull(),
            "assistant_message_id": admittedAssistant as Any? ?? NSNull()
        ], path: "chat/agent/v2/submission-frontier", configuration: target.configuration)
        try frontier.validate(host: host, client: target.clientID, conversation: conversationID, observedParent: parent)
        return frontier
    }

    func prepareRoomRequest(snapshot: MacRoomRemoteRequestPreparation, modelIntent: MacRemoteModelIntent,
        modelID: String, modelProviderID: String?, memoryEnabled: Bool) async throws -> MacRemoteSubmissionIntent {
        let target = snapshot.target
        let configuration = target.configuration
        let application = configurationStore.applicationPort
        guard let intent = try application.remoteSubmissionIntent(conversationID: snapshot.conversationID),
              intent.identity.submission_id == snapshot.submissionID,
              intent.identity.source_host_id == configuration.expectedHostID,
              intent.identity.client_id == target.clientID else { throw MacRemoteHostError.invalidResponse }
        if intent.request_digest != nil { return intent }
        let frontier = try await submissionFrontier(conversationID: snapshot.conversationID, target: target)
        let settings = try await fetchSettingsObject(configuration: configuration)
        let plan: MacRemoteHostToolPlan = try await authenticatedRead(
            ["expected_host_id": configuration.expectedHostID ?? "", "client_id": target.clientID,
             "session_id": snapshot.conversationID], path: "agent/tool-plan/prepare", configuration: configuration)
        try plan.validate(hostID: configuration.expectedHostID ?? "", clientID: target.clientID,
            sessionID: snapshot.conversationID)
        var payload: [String: Any] = [
            "messages": [["role": "user", "content": snapshot.prompt]],
            "context_components": try MacRemoteContextPayload.encode(snapshot.contextComponents),
            "memory_enabled": memoryEnabled, "execution_backend": "nola",
            "conversationId": intent.identity.conversation_id, "submission_id": intent.identity.submission_id,
            "user_message_id": intent.identity.user_message_id, "assistant_message_id": intent.identity.assistant_message_id,
            "stream": true, "client_id": target.clientID, "expected_host_id": intent.identity.source_host_id,
            "tool_plan_ref": plan.referenceObject,
            "expected_configuration_revision": frontier.configuration_revision as Any? ?? NSNull(),
            "expected_parent_message_id": frontier.parent_message_id as Any? ?? NSNull(),
        ]
        try MacRemoteModelPayload.apply(to: &payload, intent: modelIntent, settings: settings,
            expectedHostID: configuration.expectedHostID, executionBackend: "nola", modelID: modelID,
            modelProviderID: modelProviderID)
        let request = try JSONDecoder().decode(MacCanonicalJSONValue.self,
            from: JSONSerialization.data(withJSONObject: payload))
        return try application.bindRemoteSubmissionRequest(identity: intent.identity, request: request)
    }

    func runBoundRoomRequest(identity: MacRemoteSubmissionIdentity, digest: String,
        target: MacRemoteSubmissionTarget) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard identity.source_host_id == target.configuration.expectedHostID,
                          identity.client_id == target.clientID else { throw MacRemoteHostError.invalidResponse }
                    let original = try configurationStore.applicationPort.remoteSubmissionRequest(identity: identity, digest: digest)
                    guard var payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any] else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    payload["request_digest"] = digest
                    runTargets[identity.conversation_id] = target
                    activeToolPlanOwners[identity.conversation_id] = ToolPlanOwner(runID: identity.submission_id)
                    try await streamAgentPayload(payload, path: "chat/agent/v2/stream",
                        conversationID: identity.conversation_id, configuration: target.configuration, continuation: continuation)
                } catch {
                    // Cold Room replay has no new submission ticket. Observe
                    // the captured owner after transport failure, rather than
                    // relying on the ordinary submission-only recovery hook.
                    if !Task.isCancelled,
                       let receipt = try? await refreshSubmission(conversationID: identity.conversation_id,
                           target: target, cancel: false, expectedIdentity: identity),
                       receipt.phase == .rejected || receipt.phase == .cancelled {
                        continuation.yield(submissionEvent(receipt))
                        continuation.finish()
                        return
                    }
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func start(
        conversationID: String,
        submissionID: String,
        userMessageID: String,
        assistantMessageID: String,
        prompt: String,
        contextComponents: [MacCoreContextComponent],
        modelID: String,
        modelProviderID: String?,
        modelIntent: MacRemoteModelIntent?,
        memoryEnabled: Bool,
        executionBackend: String,
        externalSessionID: String?,
        submission: MacRemoteSubmissionControl,
        continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation
    ) async throws {
        guard submission.ticket.identity.conversationID == conversationID,
              submission.ticket.identity.submissionID == submissionID else {
            throw MacRunSubmissionTicket.SubmissionError.identityMismatch
        }
        try submission.ticket.checkPreparation()
        let configuration = submission.target.configuration
        let clientID = submission.target.clientID
        let frontier = try await submissionFrontier(conversationID: conversationID, target: submission.target)
        let settings = try await fetchSettingsObject(configuration: configuration)
        try submission.ticket.checkPreparation()
        let toolPlan: MacRemoteHostToolPlan = try await authenticatedRead(
            ["expected_host_id": configuration.expectedHostID ?? "", "client_id": clientID, "session_id": conversationID],
            path: "agent/tool-plan/prepare", configuration: configuration)
        try toolPlan.validate(hostID: configuration.expectedHostID ?? "", clientID: clientID,
                              sessionID: conversationID)
        try submission.ticket.checkPreparation()
        let contextPayload = try MacRemoteContextPayload.encode(contextComponents)
        var payload: [String: Any] = [
            "messages": [["role": "user", "content": prompt]],
            "context_components": contextPayload,
            "memory_enabled": memoryEnabled,
            "execution_backend": executionBackend,
            "conversationId": conversationID,
            "submission_id": submissionID,
            "user_message_id": userMessageID,
            "assistant_message_id": assistantMessageID,
            "stream": true,
            "client_id": clientID,
            "tool_plan_ref": toolPlan.referenceObject,
            "expected_configuration_revision": frontier.configuration_revision as Any? ?? NSNull(),
            "expected_parent_message_id": frontier.parent_message_id as Any? ?? NSNull(),
        ]
        if let command = submission.queuedUserCommandID {
            guard !command.isEmpty, command.utf8.count <= 128,
                  command == command.trimmingCharacters(in: .whitespacesAndNewlines),
                  !command.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
                  command == submissionID, userMessageID == "ui:\(command):user",
                  assistantMessageID == "\(command):assistant", executionBackend == "nola" else {
                throw MacRemoteHostError.invalidResponse
            }
            payload["queued_user_command_id"] = command
        }
        if let expectedHostID = configuration.expectedHostID {
            payload["expected_host_id"] = expectedHostID
        }
        try MacRemoteModelPayload.apply(
            to: &payload, intent: modelIntent, settings: settings,
            expectedHostID: configuration.expectedHostID, executionBackend: executionBackend,
            modelID: modelID, modelProviderID: modelProviderID
        )
        if let externalSessionID,
           !externalSessionID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            payload["external_session_id"] = externalSessionID
        }
        try await streamAgentPayload(payload, path: "chat/agent/v2/stream", conversationID: conversationID,
                                     configuration: configuration, continuation: continuation,
                                     submission: submission, preparedToolPlan: toolPlan)
    }

    func resume(conversationID: String, expectedRunID: String) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let observedTarget = observedRunTargets[conversationID].flatMap {
                        $0.runID == expectedRunID ? $0.target : nil
                    }
                    guard let configuration = observedTarget?.configuration ?? configuration(for: conversationID),
                          let hostID = configuration.expectedHostID else {
                        throw MacRemoteHostError.notConfigured
                    }
                    let application = configurationStore.applicationPort
                    guard let active = try application.activeRunIdentity(conversationID: conversationID),
                          active.runID == expectedRunID, let assistantID = active.assistantMessageID else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    let inspection = try application.inspectRunRecovery(conversationID: conversationID,
                        expectedRunID: expectedRunID, assistantMessageID: assistantID)
                    try inspection.requireActiveRemoteSource(hostID)
                    let clientID = try observedTarget?.clientID ?? runTargets[conversationID]?.clientID ?? configurationStore.clientID()
                    let payload = try MacRemoteResumeRequest(conversationID: conversationID,
                        expectedRunID: expectedRunID, expectedHostID: hostID, clientID: clientID).payload()
                    let observedOwner = activeToolPlanOwners[conversationID]
                    let preparedPlan: MacRemoteHostToolPlan
                    if let existingPlan = activeToolPlans[conversationID], observedOwner?.runID == expectedRunID {
                        preparedPlan = existingPlan
                    } else {
                        preparedPlan = try await readRunToolPlan(configuration: configuration, clientID: clientID,
                            conversationID: conversationID, runID: expectedRunID, assistantID: assistantID)
                    }
                    try Task.checkCancellation()
                    // Original Run catalog fetch suspends this actor. A newer submission or
                    // resume may have taken ownership while it was in flight.
                    guard activeToolPlanOwners[conversationID] == observedOwner,
                          try application.activeRunIdentity(conversationID: conversationID)?.runID == expectedRunID else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    // Adopt connection and plan together only after preparation
                    // still belongs to this Run. No await splits this adoption.
                    runTargets[conversationID] = MacRemoteSubmissionTarget(
                        configuration: configuration, clientID: clientID)
                    activeToolPlans[conversationID] = preparedPlan
                    activeToolPlanOwners[conversationID] = ToolPlanOwner(runID: expectedRunID)
                    try await streamAgentPayload(payload, path: "chat/agent/v2/resume",
                        conversationID: conversationID, configuration: configuration, continuation: continuation)
                } catch { continuation.finish(throwing: error) }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func streamAgentPayload(_ payload: [String: Any], path: String, conversationID: String,
                                    configuration: MacRemoteHostConfiguration,
                                    continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation,
                                    submission: MacRemoteSubmissionControl? = nil,
                                    preparedToolPlan: MacRemoteHostToolPlan? = nil) async throws {
        var planOwner = activeToolPlanOwners[conversationID]
        let url = mobileURL(baseURL: configuration.baseURL, path: path)
        var wirePayload = payload
        var submissionIdentity: MacRemoteSubmissionIdentity?
        if let submission {
            guard let preparedToolPlan else { throw MacRemoteHostError.invalidResponse }
            try submission.ticket.claimDispatch()
            do { try await submission.beforeDispatch() }
            catch {
                submission.ticket.rejectBeforeTransport()
                throw error
            }
            guard let intent = try configurationStore.applicationPort.remoteSubmissionIntent(conversationID: conversationID),
                  intent.identity.submission_id == submission.ticket.identity.submissionID,
                  intent.identity.source_host_id == configuration.expectedHostID,
                  intent.identity.client_id == submission.target.clientID else { throw MacRemoteHostError.invalidResponse }
            let semanticRequest = try JSONDecoder().decode(MacCanonicalJSONValue.self,
                from: JSONSerialization.data(withJSONObject: payload))
            let bound = try configurationStore.applicationPort.bindRemoteSubmissionRequest(
                identity: intent.identity, request: semanticRequest)
            submissionIdentity = intent.identity
            guard bound.phase == .bound, let digest = bound.request_digest else { throw MacRemoteHostError.invalidResponse }
            wirePayload["request_digest"] = digest
            let owner = ToolPlanOwner(runID: submission.ticket.identity.submissionID)
            activeToolPlans[conversationID] = preparedToolPlan
            activeToolPlanOwners[conversationID] = owner
            planOwner = owner
            runTargets[conversationID] = submission.target
        }
        let envelope = try encrypt(wirePayload, token: configuration.token)
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: envelope)
        request.timeoutInterval = 300
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (bytes, response) = try await NolaTailcatTransport.session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            var errorBody = ""
            for try await line in bytes.lines {
                errorBody += line
                if errorBody.count >= 1_000 { break }
            }
            throw MacRemoteHostError.http(http.statusCode, errorBody)
        }

        var decoder = MacSSEFrameDecoder()
        var receivedTerminalEvent = false
        do {
        for try await byte in bytes {
            try Task.checkCancellation()
            if let frame = decoder.append(byte) {
                if let event = try await parseFrame(
                    frame,
                    configuration: configuration,
                    expectedConversationID: conversationID,
                    expectedRunID: planOwner?.runID
                ) {
                    if event.canonicalRunID == submission?.ticket.identity.submissionID {
                        submission?.ticket.markAccepted()
                    }
                    if terminalRetiresToolPlan(event, conversationID: conversationID,
                                              owner: planOwner, hostID: configuration.expectedHostID) {
                        // This execution stream ends at its durable, exact-Run
                        // terminal receipt. Later journal updates belong to
                        // read-side observation, not a reopened execution or
                        // permission lifecycle on this already retired stream.
                        continuation.yield(event)
                        continuation.finish()
                        return
                    }
                    continuation.yield(event)
                }
            }
        }
        if let frame = decoder.finish(), let event = try await parseFrame(
            frame,
            configuration: configuration,
            expectedConversationID: conversationID,
            expectedRunID: planOwner?.runID
        ) {
            if event.canonicalRunID == submission?.ticket.identity.submissionID {
                submission?.ticket.markAccepted()
            }
            if terminalRetiresToolPlan(event, conversationID: conversationID,
                                      owner: planOwner, hostID: configuration.expectedHostID) {
                receivedTerminalEvent = true
            }
            continuation.yield(event)
        }
        } catch {
            // A lost final notification must not strand a completed Run. This
            // is read/replica recovery, never a repeat of the model submission.
            if !Task.isCancelled, let submission,
               let refreshed = try? await refreshSubmission(conversationID: conversationID, target: submission.target,
                    cancel: false, expectedIdentity: submissionIdentity) {
                continuation.yield(submissionEvent(refreshed))
                if refreshed.phase == .admitted { submission.ticket.markAccepted() }
                if refreshed.phase == .rejected || refreshed.phase == .cancelled {
                    continuation.finish(); return
                }
            }
            if !Task.isCancelled, let owner = planOwner,
               let event = try? await recoverTerminalEvent(conversationID: conversationID,
                    runID: owner.runID, configuration: configuration),
               terminalRetiresToolPlan(event, conversationID: conversationID,
                    owner: owner, hostID: configuration.expectedHostID) {
                continuation.yield(event)
                continuation.finish()
                return
            }
            throw error
        }
        if !receivedTerminalEvent, let submission,
           let refreshed = try? await refreshSubmission(conversationID: conversationID, target: submission.target,
                cancel: false, expectedIdentity: submissionIdentity) {
            continuation.yield(submissionEvent(refreshed))
            if refreshed.phase == .admitted { submission.ticket.markAccepted() }
            if refreshed.phase == .rejected || refreshed.phase == .cancelled {
                continuation.finish(); return
            }
        }
        if !receivedTerminalEvent, let owner = planOwner,
           let event = try await recoverTerminalEvent(conversationID: conversationID,
                runID: owner.runID, configuration: configuration),
           terminalRetiresToolPlan(event, conversationID: conversationID,
                owner: owner, hostID: configuration.expectedHostID) {
            continuation.yield(event)
            receivedTerminalEvent = true
        }
        guard receivedTerminalEvent else {
            throw MacRemoteHostError.transportInterrupted
        }
        continuation.finish()
    }

    func resolvePermission(
        request permissionRequest: MacRemotePermissionRequest,
        allow: Bool,
        scope: MacPermissionGrantScope = .once
    ) async throws {
        try await sendPermissionDecision(request: permissionRequest, decision: permissionRequest.decision(allow: allow, scope: scope))
    }

    func providePermissionInstruction(request: MacRemotePermissionRequest, instruction: String) async throws {
        try await sendPermissionDecision(request: request, decision: request.instructionDecision(instruction))
    }

    private func readRunToolPlan(configuration: MacRemoteHostConfiguration, clientID: String,
                                 conversationID: String, runID: String, assistantID: String) async throws -> MacRemoteHostToolPlan {
        guard let hostID = configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        let plan: MacRemoteHostToolPlan = try await authenticatedRead(
            ["expected_host_id": hostID, "client_id": clientID, "session_id": conversationID,
             "expected_run_id": runID, "assistant_message_id": assistantID],
            path: "agent/tool-plan/read", configuration: configuration)
        try plan.validate(hostID: hostID, clientID: clientID, sessionID: conversationID)
        return plan
    }

    private func sendPermissionDecision(request permissionRequest: MacRemotePermissionRequest, decision: MacRemotePermissionDecision) async throws {
        let observed = observedPermissions[permissionRequest.nonce]
        guard observed == nil || observed?.request == permissionRequest else { throw MacRemoteHostError.invalidResponse }
        guard let configuration = observed?.target.configuration ?? configuration(for: permissionRequest.sessionID) else {
            throw MacRemoteHostError.notConfigured
        }
        let runID: String
        let assistantID: String
        if let observed {
            runID = observed.runID
            assistantID = observed.assistantID
        } else {
            guard let active = try configurationStore.applicationPort.activeRunIdentity(conversationID: permissionRequest.sessionID),
                  let messageID = active.assistantMessageID,
                  activeToolPlanOwners[permissionRequest.sessionID]?.runID == active.runID else {
                throw MacRemoteHostError.invalidResponse
            }
            runID = active.runID
            assistantID = messageID
        }
        let toolPlan = try await readRunToolPlan(configuration: configuration, clientID: permissionRequest.clientID,
            conversationID: permissionRequest.sessionID, runID: runID, assistantID: assistantID)
        guard toolPlan.reference.host_id == permissionRequest.hostID,
              permissionRequest.toolCatalogEvidence?.toolID == permissionRequest.toolName,
              observed == nil || observedPermissions[permissionRequest.nonce]?.request == permissionRequest else {
            throw MacRemoteHostError.invalidResponse
        }
        if observed == nil {
            guard activeToolPlanOwners[permissionRequest.sessionID]?.runID == runID,
                  try configurationStore.applicationPort.activeRunIdentity(conversationID: permissionRequest.sessionID)?.runID == runID else {
                throw MacRemoteHostError.invalidResponse
            }
        }
        // The original authenticated Run catalog describes availability, never consent.
        // Core validates this exact nonce/call/resource and the current user decision.
        let url = mobileURL(
            baseURL: configuration.baseURL,
            path: "chat/agent/v2/permission"
        )
        let envelope = try encrypt(
            decision.payload,
            token: configuration.token
        )
        let body = try JSONSerialization.data(withJSONObject: envelope)
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(
                http.statusCode,
                String(data: data.prefix(1_000), encoding: .utf8) ?? ""
            )
        }
    }

    func sendTaskGuidance(
        conversationID: String,
        parentRunID: String,
        taskID: String,
        message: String,
        target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskGuidanceReceipt {
        guard let hostID = target.configuration.expectedHostID, !parentRunID.isEmpty else {
            throw AppleTaskGuidanceOutbox.Failure.invalidOwner
        }
        let configuration = target.configuration
        return try await AppleTaskGuidanceOutbox.shared.submit(hostId: hostID,
            conversationId: conversationID, parentRunId: parentRunID, taskId: taskID, message: message) { path, data in
                try await self.postTaskPayload(configuration: configuration, path: path, data: data)
            }
    }

    func requestTaskCancellation(
        conversationID: String,
        parentRunID: String,
        taskID: String,
        target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskCancellationReceipt {
        let configuration = target.configuration
        guard let hostID = configuration.expectedHostID else { throw MacRemoteHostError.notConfigured }
        let intent = AppleTaskCancellationIntent(hostId: hostID, conversationId: conversationID,
            parentRunId: parentRunID, taskId: taskID)
        return try await AppleTaskCancellationTransport.request(intent) { path, data in
            try await self.postTaskPayload(configuration: configuration, path: path, data: data)
        }
    }

    func cancelRun(identity: MacRemoteSubmissionIdentity, requestDigest: String) async throws {
        // Cancellation is a control request, not evidence that the Run ended.
        // Keep its permission receipt on failure and on acceptance; a delayed
        // HTTP completion must also never retire a newer Run's receipt.
        guard let configuration = observedRunTargets[identity.conversation_id].flatMap({ $0.runID == identity.submission_id ? $0.target.configuration : nil })
                ?? configuration(for: identity.conversation_id) else {
            throw MacRemoteHostError.notConfigured
        }
        try await cancelRun(identity: identity, requestDigest: requestDigest, configuration: configuration)
    }

    func cancelRun(
        identity: MacRemoteSubmissionIdentity,
        requestDigest: String,
        configuration: MacRemoteHostConfiguration
    ) async throws {
        guard identity.source_host_id == configuration.expectedHostID,
              try configurationStore.applicationPort.activeRunIdentity(conversationID: identity.conversation_id)?.runID == identity.submission_id else {
            throw MacRemoteHostError.invalidResponse
        }
        let envelope = try encrypt(
            try AppleRemoteCancellationRequest(identity: identity, requestDigest: requestDigest).object(),
            token: configuration.token
        )
        let body = try JSONSerialization.data(withJSONObject: envelope)
        var request = URLRequest(
            url: mobileURL(baseURL: configuration.baseURL, path: "chat/agent/v2/cancel")
        )
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = 20
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, String(decoding: data.prefix(1_000), as: UTF8.self))
        }
    }

    func taskEvents(
        conversationID: String,
        afterCursor: Int64
    ) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    try await self.startTaskEvents(
                        conversationID: conversationID,
                        afterCursor: afterCursor,
                        continuation: continuation
                    )
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    private func startTaskEvents(
        conversationID: String,
        afterCursor: Int64,
        continuation: AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error>.Continuation
    ) async throws {
        guard let configuration = configuration(for: conversationID) else {
            throw MacRemoteHostError.notConfigured
        }
        guard case .remote(let sourceHost, _) = try configurationStore.applicationPort
            .conversationSourceOwner(conversationID: conversationID),
              configuration.expectedHostID == sourceHost else {
            throw MacRemoteHostError.invalidResponse
        }
        if let observed = observedRunTargets[conversationID],
           let hostID = observed.target.configuration.expectedHostID {
            let targetConfiguration = observed.target.configuration
            try await AppleTaskGuidanceOutbox.shared.recover(hostId: hostID,
                conversationId: conversationID, parentRunId: observed.runID) { path, data in
                    try await self.postTaskPayload(configuration: targetConfiguration, path: path, data: data)
                }
        }
        let envelope = try encrypt(
            [
                "conversationId": conversationID,
                "afterCursor": max(0, afterCursor),
                "timeoutMs": 25_000,
            ],
            token: configuration.token
        )
        let body = try JSONSerialization.data(withJSONObject: envelope)
        var request = URLRequest(
            url: mobileURL(
                baseURL: configuration.baseURL,
                path: "chat/agent/v2/task/events"
            )
        )
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = 35
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (bytes, response) = try await NolaTailcatTransport.session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            var errorBody = ""
            for try await line in bytes.lines {
                errorBody += line
                if errorBody.count >= 1_000 { break }
            }
            throw MacRemoteHostError.http(http.statusCode, errorBody)
        }
        var decoder = MacSSEFrameDecoder()
        for try await byte in bytes {
            try Task.checkCancellation()
            if let frame = decoder.append(byte) {
                if let event = try parseTaskFrame(
                    frame,
                    token: configuration.token,
                    expectedConversationID: conversationID
                ) {
                    continuation.yield(event)
                }
            }
        }
        if let frame = decoder.finish(), let event = try parseTaskFrame(
            frame,
            token: configuration.token,
            expectedConversationID: conversationID
        ) {
            continuation.yield(event)
        }
        continuation.finish()
    }

    private func parseTaskFrame(
        _ frame: String,
        token: String,
        expectedConversationID: String
    ) throws -> MacRemoteTaskProjectionEvent? {
        let raw = frame.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty, raw != "[DONE]" else { return nil }
        guard let data = raw.data(using: .utf8),
              let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let object = parsed["type"] as? String == "encrypted"
            ? try decrypt(parsed, token: token)
            : parsed
        guard let conversationID = object["conversationId"] as? String,
              conversationID == expectedConversationID,
              let cursor = (object["cursor"] as? NSNumber)?.int64Value else {
            throw MacRemoteHostError.invalidResponse
        }
        let projection = try taskProjection(from: object)
        return MacRemoteTaskProjectionEvent(
            conversationID: conversationID,
            cursor: cursor,
            projection: projection
        )
    }

    private func taskProjection(from response: [String: Any]) throws -> MacCoreTaskProjection? {
        try Self.decodeTaskProjection(from: response)
    }

    nonisolated private static func decodeTaskProjection(from response: [String: Any]) throws -> MacCoreTaskProjection? {
        guard let value = response["taskProjection"], !(value is NSNull) else { return nil }
        guard JSONSerialization.isValidJSONObject(value) else {
            throw MacRemoteHostError.invalidResponse
        }
        let data = try JSONSerialization.data(withJSONObject: value)
        return try JSONDecoder().decode(MacCoreTaskProjection.self, from: data)
    }

    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws {
        guard let host = target.configuration.expectedHostID, !host.isEmpty,
              profile.map({ $0.hostID == host }) ?? true else { throw MacRemoteHostError.invalidResponse }
        let selection: [String: Any] = profile.map { ["kind":"model", "profile_id":$0.profileID, "profile_revision":$0.revision] }
            ?? ["kind":"inherit"]
        func exchange(_ action: [String: Any]) async throws -> [String: Any] {
            let body = try JSONSerialization.data(withJSONObject: ["expected_host_id":host,
                "conversation_id":conversationID, "action":action])
            let data = try await postTaskPayload(configuration: target.configuration,
                path: "chat/agent/v2/execution-selection", data: body)
            guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  value["source_host_id"] as? String == host, value["conversation_id"] as? String == conversationID,
                  let result = value["result"] as? [String: Any] else { throw MacRemoteHostError.invalidResponse }
            return result
        }
        let current = try await exchange(["type":"read"])
        let result: [String: Any]
        if current["status"] as? String == "not_found" {
            guard allowCreate else { throw MacRemoteHostError.invalidResponse }
            result = try await exchange(["type":"create", "title":title, "selection":selection])
        } else {
            guard current["status"] as? String == "read",
                  let revision = current["configuration_revision"] as? NSNumber, revision.int64Value > 0 else {
                throw MacRemoteHostError.invalidResponse
            }
            result = try await exchange(["type":"select", "expected_revision":revision, "selection":selection])
        }
        guard ["created", "committed"].contains(result["status"] as? String ?? ""),
              ((result["configuration_revision"] as? NSNumber)?.int64Value ?? 0) > 0 else {
            throw MacRemoteHostError.invalidResponse
        }
        if let expected = profile.map({ "configured:" + $0.profileID }) {
            guard result["model_selection_id"] as? String == expected else { throw MacRemoteHostError.invalidResponse }
        } else {
            guard result["model_selection_id"] is NSNull else { throw MacRemoteHostError.invalidResponse }
        }
    }

    func conversationProgress(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacRemoteConversationProgress {
        guard let host = target.configuration.expectedHostID else { throw MacRemoteHostError.invalidResponse }
        let data = try await postTaskPayload(configuration: target.configuration, path: "chat/agent/v2/progress",
            data: JSONSerialization.data(withJSONObject: ["expected_host_id": host, "conversation_id": conversationID]))
        let value = try JSONDecoder().decode(MacRemoteConversationProgress.self, from: data)
        guard value.source_host_id == host, value.conversation_id == conversationID,
              value.plan.map({ $0.conversationID == conversationID }) ?? true else {
            throw MacRemoteHostError.invalidResponse
        }
        return value
    }

    func goalControl(_ intent: AppleGoalControlIntent, target: MacRemoteSubmissionTarget) async throws -> AppleGoalControlResponse {
        guard target.configuration.expectedHostID == intent.expected_host_id else { throw MacRemoteHostError.invalidResponse }
        let data = try await postTaskPayload(configuration: target.configuration,
            path: "chat/agent/v2/goal", data: JSONEncoder().encode(intent), acceptedStatuses: [200, 202, 409])
        let response = try JSONDecoder().decode(AppleGoalControlResponse.self, from: data)
        guard response.source_host_id == intent.expected_host_id else { throw MacRemoteHostError.invalidResponse }
        return response
    }

    private func postTaskPayload(configuration: MacRemoteHostConfiguration, path: String, data: Data, acceptedStatuses: Set<Int>? = nil) async throws -> Data {
        guard let payload = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let envelope = try encrypt(payload, token: configuration.token)
        let body = try JSONSerialization.data(withJSONObject: envelope)
        var request = URLRequest(url: mobileURL(baseURL: configuration.baseURL, path: path))
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyAuthentication(to: &request, token: configuration.token)
        let (data, response) = try await NolaTailcatTransport.session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MacRemoteHostError.invalidResponse
        }
        guard acceptedStatuses?.contains(http.statusCode) ?? (200...299).contains(http.statusCode) else {
            throw MacRemoteHostError.http(http.statusCode, String(decoding: data.prefix(1_000), as: UTF8.self))
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        let decoded = root["type"] as? String == "encrypted"
            ? try decrypt(root, token: configuration.token) : root
        return try JSONSerialization.data(withJSONObject: decoded)
    }

    func parseFrame(
        _ frame: String,
        configuration: MacRemoteHostConfiguration,
        expectedConversationID: String,
        expectedRunID: String?
    ) async throws -> MacLocalAgentEvent? {
        guard let sourceHostID = configuration.expectedHostID,
              !expectedConversationID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw MacRemoteHostError.invalidResponse
        }
        let raw = frame.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        if raw == "[DONE]" {
            // Transport termination is not an Agent event. The terminal Core Trace
            // must arrive before this marker.
            return nil
        }
        guard let data = raw.data(using: .utf8),
              let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacRemoteHostError.invalidResponse
        }
        var object: [String: Any]
        if parsed["type"] as? String == "encrypted" {
            object = try decrypt(parsed, token: configuration.token)
        } else {
            object = parsed
        }
        if object["type"] as? String == "submission_receipt" {
            guard let rawReceipt = object["receipt"], JSONSerialization.isValidJSONObject(rawReceipt) else {
                throw MacRemoteHostError.invalidResponse
            }
            let receipt = try JSONDecoder().decode(MacSourceSubmissionReceipt.self,
                from: JSONSerialization.data(withJSONObject: rawReceipt))
            guard receipt.identity.source_host_id == sourceHostID,
                  receipt.identity.conversation_id == expectedConversationID,
                  receipt.identity.submission_id == expectedRunID else { throw MacRemoteHostError.invalidResponse }
            let accepted = try configurationStore.applicationPort.acceptRemoteSubmissionReceipt(receipt)
            return submissionEvent(accepted)
        }
        if object["type"] as? String == "context_usage" {
            // Notification values are untrusted hints. Resolve only the frozen
            // execution's admitted root and fetch its committed source record.
            guard let runID = expectedRunID,
                  object["session_id"] as? String == expectedConversationID,
                  let root = try admittedContextRoot(
                    conversationID: expectedConversationID, runID: runID) else {
                throw MacRemoteHostError.invalidResponse
            }
            let admission: MacRunMessageAdmissionCursor = try await journalRequest(
                .admissionHead(conversationID: expectedConversationID, runID: runID), configuration: configuration)
            guard admission.identity.conversation_id == expectedConversationID,
                  admission.identity.run_id == runID,
                  admission.identity.assistant_message_id == root else {
                throw MacRemoteHostError.invalidResponse
            }
            let usage: MacCommittedRunContextUsage? = try await journalRequest(
                .contextUsage(admission.identity), configuration: configuration)
            guard let usage else { return nil }
            guard usage.identity == admission.identity else { throw MacRemoteHostError.invalidResponse }
            try await receiveContextUsage(usage, conversationID: expectedConversationID, runID: runID,
                configuration: configuration)
            return MacLocalAgentEvent(kind: .status, text: "", title: nil,
                detail: "context_usage_committed", isError: false).bindingCanonicalRun(runID)
        }
        if ["assistant_journal_committed_v1", "assistant_run_terminal_committed_v1"].contains(object["type"] as? String ?? "") {
            guard object["source_host_id"] as? String == sourceHostID else {
                throw MacRemoteHostError.invalidResponse
            }
        }
        if object["type"] as? String == "assistant_run_terminal_committed_v1" {
            guard let expectedRunID, !expectedRunID.isEmpty,
                  object["run_id"] as? String == expectedRunID,
                  object["session_id"] as? String == expectedConversationID,
                  let digest = object["seal_digest"] as? String else { throw MacRemoteHostError.invalidResponse }
            guard let event = try await recoverTerminalEvent(conversationID: expectedConversationID,
                runID: expectedRunID, configuration: configuration, expectedDigest: digest) else {
                throw MacRemoteHostError.invalidResponse
            }
            return event
        }
        if object["type"] as? String == "assistant_journal_committed_v1" {
            let notification = try JSONDecoder().decode(MacAssistantJournalNotification.self,
                from: JSONSerialization.data(withJSONObject: object))
            guard notification.source_host_id == sourceHostID,
                  notification.session_id == expectedConversationID,
                  expectedRunID == nil || notification.through.identity.run_id == expectedRunID,
                  notification.session_id == notification.through.identity.conversation_id else {
                throw MacRemoteHostError.invalidResponse
            }
            let existing = try? configurationStore.applicationPort.assistantJournalReceptionState(
                sourceHostID: sourceHostID, through: notification.through)
            // Known owners already have durable admission. Only a new owner
            // needs a metadata fetch; ordinary text deltas stay incremental.
            if existing?.after.sequence == "0" || existing == nil {
                try await synchronizeRunMessageAdmissions(conversationID: expectedConversationID,
                    runID: notification.through.identity.run_id, configuration: configuration)
            }
            _ = try await synchronizeAssistantJournal(through: notification.through,
                conversationID: expectedConversationID, configuration: configuration,
                application: configurationStore.applicationPort)
            // Content is durable locally before the cursor reaches the view.
            // This marker is neither an activity row nor a permission decision.
            return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                assistantJournalCursor: notification.through).bindingCanonicalRun(notification.through.identity.run_id)
        }
        object["source_host_id"] = sourceHostID
        if object["type"] as? String == "permission_request" {
            guard object["host_id"] as? String == sourceHostID,
                  object["client_id"] as? String == (try configurationStore.clientID()),
                  let expectedRunID, object["canonical_run_id"] as? String == expectedRunID,
                  object["session_id"] as? String == expectedConversationID,
                  permissionRequest(from: object) != nil else {
                throw MacRemoteHostError.invalidResponse
            }
            // Continue through the shared event decoder so its trace/request
            // identity checks remain in force as well as the remote evidence.
        }
        let trace = MacAgentTraceEnvelope.decode(object["trace"])
        let conversationID = trace?.conversationID ?? object["session_id"] as? String
        guard let conversationID,
              conversationID == expectedConversationID,
              JSONSerialization.isValidJSONObject(object),
              let canonicalData = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
              let canonicalLine = String(data: canonicalData, encoding: .utf8),
              let event = MacAgentHostRuntime.decode(
                line: canonicalLine,
                conversationID: conversationID,
                controlSessionID: conversationID
              ) else {
            throw MacRemoteHostError.invalidResponse
        }
        return event
    }

    private func permissionRequest(from object: [String: Any]) -> MacRemotePermissionRequest? {
        guard let instructions = MacPermissionInstruction.decode(object["permission_instructions"]) else { return nil }
        guard let id = object["id"] as? String,
              let allowConversation = object["allow_conversation"] as? Bool,
              let protocolVersion = (object["protocol_version"] as? NSNumber)?.intValue,
              protocolVersion == MacRemotePermissionRequest.supportedProtocolVersion,
              let nonce = object["nonce"] as? String,
              let hostID = object["host_id"] as? String,
              let clientID = object["client_id"] as? String,
              let sessionID = object["session_id"] as? String,
              let toolName = object["tool_name"] as? String,
              let toolCatalogEvidence = MacRemotePermissionToolEvidence.decode(object["tool_catalog_evidence"], toolName: toolName) else {
            return nil
        }
        let arguments: String = {
            guard let value = object["args"], JSONSerialization.isValidJSONObject(value),
                  let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]),
                  let text = String(data: data, encoding: .utf8) else { return "" }
            return text
        }()
        return MacRemotePermissionRequest(
            id: id,
            allowConversation: allowConversation,
            protocolVersion: protocolVersion,
            nonce: nonce,
            hostID: hostID,
            clientID: clientID,
            sessionID: sessionID,
            toolName: toolName,
            arguments: arguments,
            permissionInstructions: instructions,
            callID: object["call_id"] as? String,
            reason: [object["reason"] as? String, object["safe_review_text"] as? String]
                .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n"),
            message: object["message"] as? String ?? "Allow this host tool once?",
            expiresAt: (object["expires_at"] as? NSNumber)?.int64Value,
            policyDigest: object["policy_digest"] as? String,
            manifestDigest: object["manifest_digest"] as? String,
            manifestValidUntilMs: (object["manifest_valid_until_ms"] as? NSNumber)?.int64Value,
            toolCatalogEvidence: toolCatalogEvidence
        )
    }

    private func mobileURL(baseURL: URL, path: String) -> URL {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        var parts = (components?.path ?? baseURL.path).split(separator: "/").map(String.init)
        if let apiIndex = parts.lastIndex(of: "api") {
            parts = Array(parts.prefix(apiIndex))
        } else if parts.last == "v1" {
            parts.removeLast()
        }
        components?.path = parts.isEmpty ? "" : "/" + parts.joined(separator: "/")
        var url = (components?.url ?? baseURL).appendingPathComponent("api").appendingPathComponent("mobile")
        for component in path.split(separator: "/") {
            url.appendPathComponent(String(component))
        }
        return url
    }

    private func encrypt(_ payload: [String: Any], token: String) throws -> [String: Any] {
        let plaintext = try JSONSerialization.data(withJSONObject: payload)
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(token.utf8))))
        let nonce = AES.GCM.Nonce()
        let sealed = try AES.GCM.seal(plaintext, using: key, nonce: nonce, authenticating: cryptoAAD)
        return [
            "v": 1,
            "alg": "AES-256-GCM",
            "nonce": base64URL(Data(nonce)),
            "ciphertext": base64URL(sealed.ciphertext),
            "tag": base64URL(sealed.tag),
        ]
    }

    private func decrypt(_ envelope: [String: Any], token: String) throws -> [String: Any] {
        guard (envelope["v"] as? Int) == 1,
              envelope["alg"] as? String == "AES-256-GCM",
              let nonceText = envelope["nonce"] as? String,
              let ciphertextText = envelope["ciphertext"] as? String,
              let tagText = envelope["tag"] as? String else {
            throw MacRemoteHostError.invalidEnvelope
        }
        let nonce = try AES.GCM.Nonce(data: try base64URLDecode(nonceText))
        let box = try AES.GCM.SealedBox(
            nonce: nonce,
            ciphertext: try base64URLDecode(ciphertextText),
            tag: try base64URLDecode(tagText)
        )
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(token.utf8))))
        let plaintext = try AES.GCM.open(box, using: key, authenticating: cryptoAAD)
        guard let object = try JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
            throw MacRemoteHostError.invalidEnvelope
        }
        return object
    }

    private func applyAuthentication(to request: inout URLRequest, token: String) {
        guard let url = request.url else { return }
        let timestamp = UInt64(Date().timeIntervalSince1970)
        let nonce = UUID().uuidString.lowercased()
        let path = url.path + (url.query.map { "?\($0)" } ?? "")
        let bodyDigest = SHA256.hash(data: request.httpBody ?? Data())
            .map { String(format: "%02x", $0) }
            .joined()
        let method = request.httpMethod ?? "GET"
        let message = "v1\n\(method)\n\(path)\n\(timestamp)\n\(nonce)\n\(bodyDigest)"
        let signature = HMAC<SHA256>.authenticationCode(
            for: Data(message.utf8),
            using: SymmetricKey(data: Data(token.utf8))
        )
        request.setValue(String(timestamp), forHTTPHeaderField: "X-Nola-Mobile-Timestamp")
        request.setValue(nonce, forHTTPHeaderField: "X-Nola-Mobile-Nonce")
        request.setValue(bodyDigest, forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256")
        request.setValue(base64URL(Data(signature)), forHTTPHeaderField: "X-Nola-Mobile-Auth")
    }

    private func base64URL(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private func base64URLDecode(_ text: String) throws -> Data {
        var normalized = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        normalized += String(repeating: "=", count: (4 - normalized.count % 4) % 4)
        guard let data = Data(base64Encoded: normalized) else { throw MacRemoteHostError.invalidEnvelope }
        return data
    }
}


extension MacRemoteAgentRunning {
    func refreshSubmission(conversationID: String, target: MacRemoteSubmissionTarget, cancel: Bool, expectedIdentity: MacRemoteSubmissionIdentity?) async throws -> MacRemoteSubmissionIntent? {
        throw MacRemoteHostError.invalidResponse
    }
    func observeRun(conversationID: String, expectedRunID: String?, assistantMessageID: String?,
                    target: MacRemoteSubmissionTarget) async throws -> MacRemoteRunObservationBatch {
        throw MacRemoteHostError.invalidResponse
    }
    func resume(conversationID: String, expectedRunID: String) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: MacRemoteHostError.invalidResponse) }
    }
    func providePermissionInstruction(request: MacRemotePermissionRequest, instruction: String) async throws {
        throw MacPermissionInstructionError.unsupported
    }
}
