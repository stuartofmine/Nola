// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Persists multiple API endpoints and their ordered, purpose-scoped remote models.

import Foundation
import Security

nonisolated enum MacRemoteModelCapability: String, CaseIterable, Codable, Hashable, Identifiable, Sendable {
    case chat
    case retrieval
    case issueRepair = "issue_repair"
    case sensitiveData = "sensitive_data"

    var id: String { rawValue }
}

nonisolated enum MacChatOutputTokenField: String, Codable, CaseIterable, Sendable {
    case auto
    case maxTokens = "max_tokens"
    case maxCompletionTokens = "max_completion_tokens"
}

nonisolated struct MacModelExecutionCapabilities: Codable, Hashable, Sendable {
    var toolCalls = false
    var imageInput = false
    var reasoning = false
    var samplingParameters = false
    var samplingRequiresNoReasoning = false
    var chatOutputTokenField: MacChatOutputTokenField? = nil
    // Read-only route metadata. Encoded separately on the profile source, never
    // inside Domain's execution-capability override document.
    var reasoningSummarySupported: Bool? = nil

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case toolCalls, imageInput, reasoning, samplingParameters
        case samplingRequiresNoReasoning, chatOutputTokenField
    }
}

extension MacModelExecutionCapabilities {
    // Preserve the shared contract's required fields and optional defaults,
    // without turning an absent declaration or explicit JSON null into support.
    nonisolated init(from decoder: Decoder) throws {
        struct AnyKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }
        let keys = try decoder.container(keyedBy: AnyKey.self)
        let supported = Set(CodingKeys.allCases.map(\.rawValue))
        guard Set(keys.allKeys.map(\.stringValue)).isSubset(of: supported) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Unsupported model execution capability field."))
        }
        let values = try decoder.container(keyedBy: CodingKeys.self)
        toolCalls = try values.decode(Bool.self, forKey: .toolCalls)
        imageInput = try values.decode(Bool.self, forKey: .imageInput)
        reasoning = try values.decode(Bool.self, forKey: .reasoning)
        samplingParameters = try values.decode(Bool.self, forKey: .samplingParameters)
        samplingRequiresNoReasoning = values.contains(.samplingRequiresNoReasoning)
            ? try values.decode(Bool.self, forKey: .samplingRequiresNoReasoning) : false
        // The existing Mac representation omits nil on save and maps it to
        // Domain's auto spelling at the Host request boundary.
        chatOutputTokenField = values.contains(.chatOutputTokenField)
            ? try values.decode(MacChatOutputTokenField.self, forKey: .chatOutputTokenField) : nil
        guard !samplingRequiresNoReasoning || (samplingParameters && reasoning) else {
            throw DecodingError.dataCorruptedError(forKey: .samplingRequiresNoReasoning,
                in: values, debugDescription: "Sampling effort restriction requires sampling and reasoning support.")
        }
    }
}

nonisolated enum MacModelExecutionCapabilityStatus: Hashable, Sendable {
    case needsConfirmation
    case textOnly
    case toolsEnabled

    init(_ declaration: MacModelExecutionCapabilities?) {
        if let declaration {
            self = declaration.toolCalls ? .toolsEnabled : .textOnly
        } else {
            self = .needsConfirmation
        }
    }

    var label: String? {
        switch self {
        case .needsConfirmation:
            String(localized: "Confirm model capabilities", table: "ModelCenter")
        case .textOnly:
            String(localized: "Chat only · Tools off", table: "ModelCenter")
        case .toolsEnabled:
            nil
        }
    }
}

nonisolated enum MacModelCapabilityDeclarationError: LocalizedError {
    case confirmationRequired(modelName: String)

    var errorDescription: String? {
        switch self {
        case .confirmationRequired(let modelName):
            String(format: String(localized: "Confirm execution capabilities for %@ in Model Center before using this model. Disabled capabilities mean unsupported; allowed uses do not declare tool support.", table: "ModelCenter"), modelName)
        }
    }
}

nonisolated struct MacRemoteModelConfiguration: Codable, Hashable, Identifiable, Sendable {
    var id: String
    var modelName: String {
        didSet {
            if modelName.trimmingCharacters(in: .whitespacesAndNewlines)
                != oldValue.trimmingCharacters(in: .whitespacesAndNewlines) {
                hasExecutionCapabilityDeclaration = false
            }
        }
    }
    var displayName: String
    var isEnabled: Bool
    var priority: Int
    var order: Int
    var capabilities: Set<MacRemoteModelCapability>
    /// Editable values are a draft until confirmed. A missing declaration
    /// must not become an all-false execution contract during projection.
    var executionCapabilities: MacModelExecutionCapabilities
    private(set) var hasExecutionCapabilityDeclaration: Bool
    var reasoning: MacReasoningConfiguration?
    var reasoningSummarySupported: Bool?

    var declaredExecutionCapabilities: MacModelExecutionCapabilities? {
        hasExecutionCapabilityDeclaration ? executionCapabilities : nil
    }

    var executionCapabilityStatus: MacModelExecutionCapabilityStatus {
        MacModelExecutionCapabilityStatus(declaredExecutionCapabilities)
    }

    mutating func confirmExecutionCapabilities() {
        hasExecutionCapabilityDeclaration = true
    }

    mutating func invalidateExecutionCapabilityDeclaration() {
        hasExecutionCapabilityDeclaration = false
    }

    init(
        id: String = UUID().uuidString.lowercased(),
        modelName: String = "",
        displayName: String = "",
        isEnabled: Bool = true,
        priority: Int = 50,
        order: Int = 0,
        capabilities: Set<MacRemoteModelCapability> = [.chat, .retrieval, .issueRepair],
        executionCapabilities: MacModelExecutionCapabilities? = .init(toolCalls: true),
        reasoning: MacReasoningConfiguration? = nil,
        reasoningSummarySupported: Bool? = nil
    ) {
        self.id = id
        self.modelName = modelName
        self.displayName = displayName
        self.isEnabled = isEnabled
        self.priority = priority
        self.order = order
        self.capabilities = capabilities
        self.executionCapabilities = executionCapabilities ?? .init()
        self.hasExecutionCapabilityDeclaration = executionCapabilities != nil
        self.reasoning = reasoning
        self.reasoningSummarySupported = reasoningSummarySupported
    }

    private enum CodingKeys: String, CodingKey {
        case id, modelName, displayName, isEnabled, priority, order, capabilities, executionCapabilities, reasoning, reasoningSummarySupported
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        self.init(id: try values.decode(String.self, forKey: .id),
            modelName: try values.decode(String.self, forKey: .modelName),
            displayName: try values.decode(String.self, forKey: .displayName),
            isEnabled: try values.decode(Bool.self, forKey: .isEnabled),
            priority: try values.decode(Int.self, forKey: .priority),
            order: try values.decode(Int.self, forKey: .order),
            capabilities: try values.decode(Set<MacRemoteModelCapability>.self, forKey: .capabilities),
            executionCapabilities: try values.decodeIfPresent(MacModelExecutionCapabilities.self, forKey: .executionCapabilities),
            reasoning: try values.decodeIfPresent(MacReasoningConfiguration.self, forKey: .reasoning),
            reasoningSummarySupported: try values.decodeIfPresent(Bool.self, forKey: .reasoningSummarySupported))
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(id, forKey: .id)
        try values.encode(modelName, forKey: .modelName)
        try values.encode(displayName, forKey: .displayName)
        try values.encode(isEnabled, forKey: .isEnabled)
        try values.encode(priority, forKey: .priority)
        try values.encode(order, forKey: .order)
        try values.encode(capabilities, forKey: .capabilities)
        try values.encodeIfPresent(declaredExecutionCapabilities, forKey: .executionCapabilities)
        try values.encodeIfPresent(reasoning, forKey: .reasoning)
        try values.encodeIfPresent(reasoningSummarySupported, forKey: .reasoningSummarySupported)
    }
}

nonisolated enum MacRemoteModelOrdering {
    static func primaryChatModelID(in endpoints: [MacRemoteEndpointConfiguration]) -> String? {
        var selected: MacRemoteModelConfiguration?
        for endpoint in endpoints where endpoint.isEnabled {
            for model in endpoint.models where model.isEnabled && model.capabilities.contains(.chat) {
                if selected.map({ model.priority > $0.priority }) ?? true {
                    selected = model
                }
            }
        }
        return selected?.id
    }

    static func moving(
        _ models: [MacRemoteModelConfiguration],
        draggedID: String,
        onto targetID: String
    ) -> [MacRemoteModelConfiguration]? {
        guard let sourceIndex = models.firstIndex(where: { $0.id == draggedID }),
              let targetIndex = models.firstIndex(where: { $0.id == targetID })
        else { return nil }
        guard sourceIndex != targetIndex else { return reindexed(models) }
        var reordered = models
        let moved = reordered.remove(at: sourceIndex)
        reordered.insert(moved, at: min(targetIndex, reordered.count))
        return reindexed(reordered)
    }

    /// Ordering only breaks ties between equal-priority models. Reordering the
    /// presentation must never silently change the user's routing priorities.
    static func reindexed(_ models: [MacRemoteModelConfiguration]) -> [MacRemoteModelConfiguration] {
        models.enumerated().map { index, original in
            var model = original
            model.order = index
            return model
        }
    }
}

nonisolated struct MacRemoteEndpointConfiguration: Hashable, Identifiable, Sendable {
    var wireProtocol: MacModelWireProtocol = .chatCompletions {
        didSet {
            if wireProtocol != oldValue { invalidateExecutionCapabilityDeclarations() }
        }
    }
    var id: String
    var name: String
    var baseURL: String {
        didSet {
            if baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
                != oldValue.trimmingCharacters(in: .whitespacesAndNewlines) {
                invalidateExecutionCapabilityDeclarations()
            }
        }
    }
    var apiKey: String
    var isEnabled: Bool
    var order: Int
    var models: [MacRemoteModelConfiguration]

    private mutating func invalidateExecutionCapabilityDeclarations() {
        for index in models.indices {
            models[index].invalidateExecutionCapabilityDeclaration()
        }
    }

    init(
        id: String = UUID().uuidString.lowercased(),
        name: String = "",
        baseURL: String = "",
        apiKey: String = "",
        isEnabled: Bool = true,
        order: Int = 0,
        models: [MacRemoteModelConfiguration] = [],
        wireProtocol: MacModelWireProtocol = .chatCompletions
    ) {
        self.id = id
        self.name = name
        self.baseURL = baseURL
        self.apiKey = apiKey
        self.isEnabled = isEnabled
        self.order = order
        self.models = models
        self.wireProtocol = wireProtocol
    }
}

nonisolated struct MacRemoteEndpointSnapshot: Sendable {
    let endpoints: [MacRemoteEndpointConfiguration]
    let revisions: [String: Int64]
}

nonisolated enum MacRemoteEndpointURL {
    static func isValid(_ value: String) -> Bool {
        guard let components = URLComponents(string: value.trimmingCharacters(in: .whitespacesAndNewlines)),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              let host = components.host, !host.isEmpty,
              components.user == nil,
              components.password == nil,
              components.fragment == nil else { return false }
        return true
    }
}

nonisolated struct MacRemoteModelOption: Hashable, Identifiable, Sendable {
    let profileID: String
    let endpointID: String
    let endpointName: String
    let modelName: String
    let displayName: String
    let priority: Int
    let endpointOrder: Int
    let modelOrder: Int
    let capabilities: Set<MacRemoteModelCapability>
    let executionCapabilityStatus: MacModelExecutionCapabilityStatus
    let selected: Bool

    var id: String { profileID }
}

nonisolated struct MacRemoteModelFallback: Hashable, Sendable {
    let profileID: String
    let endpointName: String
    let endpoint: String
    let apiKey: String
    let model: String
}

nonisolated struct MacRemoteModelRoutePlan: Sendable {
    let primary: MacAppleChatConfiguration
    let fallbacks: [MacRemoteModelFallback]
}

nonisolated enum MacRemoteModelRoutePresentation {
    static func retryAttempt(from detail: String) -> Int {
        guard let range = detail.range(of: "attempt="),
              let attempt = Int(detail[range.upperBound...].prefix { $0.isNumber }) else { return 1 }
        return max(1, attempt)
    }

    static func retryReason(from detail: String) -> String {
        guard let range = detail.range(of: " reason=") else { return detail }
        let reason = detail[range.upperBound...]
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return reason.isEmpty ? detail : reason
    }

    static func status(
        for plan: MacRemoteModelRoutePlan,
        failedCandidateIndex: Int,
        fallsBack: Bool,
        failureMessage: String
    ) -> (title: String, detail: String) {
        let targets = [targetLabel(
            endpointName: plan.primary.endpointName,
            endpoint: plan.primary.baseURL.absoluteString,
            model: plan.primary.model
        )] + plan.fallbacks.map {
            targetLabel(endpointName: $0.endpointName, endpoint: $0.endpoint, model: $0.model)
        }
        let isFallback = fallsBack
        let failedIndex = max(0, failedCandidateIndex)
        let nextIndex = isFallback ? failedIndex + 1 : failedIndex
        let failed = targets.indices.contains(failedIndex) ? targets[failedIndex] : targets[0]
        let next = targets.indices.contains(nextIndex) ? targets[nextIndex] : nil
        let title = isFallback ? String(localized: "Model fallback") : String(localized: "Model retry")
        var lines = [
            "\(String(localized: "Failed model")): \(failed)",
            "\(String(localized: "Failure reason")): \(failureMessage)",
        ]
        if let next {
            let label = isFallback ? String(localized: "Falling back to") : String(localized: "Retrying with")
            lines.append("\(label): \(next)")
        }
        return (title, lines.joined(separator: "\n"))
    }

    static func failureStatus(
        for plan: MacRemoteModelRoutePlan,
        candidateIndex: Int,
        errorDetail: String
    ) -> (title: String, detail: String) {
        let targets = targetLabels(for: plan)
        let target = targets.indices.contains(candidateIndex)
            ? targets[candidateIndex]
            : targets[0]
        return (
            String(localized: "Model request failed"),
            [
                "\(String(localized: "Failed model")): \(target)",
                "\(String(localized: "Failure reason")): \(errorDetail)",
            ].joined(separator: "\n")
        )
    }

    private static func targetLabels(for plan: MacRemoteModelRoutePlan) -> [String] {
        [targetLabel(
            endpointName: plan.primary.endpointName,
            endpoint: plan.primary.baseURL.absoluteString,
            model: plan.primary.model
        )] + plan.fallbacks.map {
            targetLabel(endpointName: $0.endpointName, endpoint: $0.endpoint, model: $0.model)
        }
    }

    private static func targetLabel(endpointName: String?, endpoint: String, model: String) -> String {
        let configuredName = endpointName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let provider = !configuredName.isEmpty
            ? configuredName
            : (URL(string: endpoint)?.host ?? endpoint)
        return "\(provider) · \(model)"
    }
}

nonisolated enum MacRemoteModelRoutingError: LocalizedError {
    case allConfiguredModelsCoolingDown
    case selectedModelUnavailable
    case invalidDefaultConfiguration

    var errorDescription: String? {
        switch self {
        case .allConfiguredModelsCoolingDown:
            String(localized: "All configured chat models are temporarily unavailable. Check their quota status in Model Center.")
        case .selectedModelUnavailable:
            String(localized: "The selected chat model is unavailable. Choose a model or restore the default in the model menu.", table: "ModelCenter")
        case .invalidDefaultConfiguration:
            String(localized: "The default chat model configuration is invalid. Reset it in Model Center.", table: "ModelCenter")
        }
    }
}

nonisolated enum MacRemoteQuotaFailurePolicy {
    static func exhaustedQuotaResetDate(from detail: String) -> Date? {
        guard let start = detail.firstIndex(of: "{"),
              let data = String(detail[start...]).data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) else { return nil }
        let remaining = findValue(in: root, keys: [
            "X-RateLimit-Remaining", "x-ratelimit-remaining", "remaining",
        ])
        let isExhausted = (remaining as? NSNumber)?.intValue == 0
            || (remaining as? String) == "0"
        guard isExhausted,
              let reset = findValue(in: root, keys: [
                "X-RateLimit-Reset", "x-ratelimit-reset", "reset",
              ]) else { return nil }
        let milliseconds: Double?
        if let number = reset as? NSNumber { milliseconds = number.doubleValue }
        else if let string = reset as? String { milliseconds = Double(string) }
        else { milliseconds = nil }
        guard let milliseconds, milliseconds > 0 else { return nil }
        let epochSeconds = milliseconds > 10_000_000_000
            ? milliseconds / 1_000
            : milliseconds
        return Date(timeIntervalSince1970: epochSeconds)
    }

    static func failedCandidateIndex(from detail: String) -> Int {
        guard let range = detail.range(of: "attempt="),
              let attempt = Int(detail[range.upperBound...].prefix { $0.isNumber }) else { return 0 }
        return max(0, attempt - 2)
    }

    private static func findValue(in value: Any, keys: Set<String>) -> Any? {
        if let object = value as? [String: Any] {
            for key in keys {
                if let found = object[key] { return found }
            }
            for child in object.values {
                if let found = findValue(in: child, keys: keys) { return found }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let found = findValue(in: child, keys: keys) { return found }
            }
        }
        return nil
    }
}

struct MacAppleChatConfiguration: Sendable {
    var profileRevision: Int64? = nil
    var wireProtocol: MacModelWireProtocol = .chatCompletions
    var siwcProfile: MacSiwcProfileSelection? = nil
    var parameters: ModelParameters = .webAlignedCloudDefaults
    var reasoning: MacReasoningConfiguration? = nil
    var executionCapabilities: MacModelExecutionCapabilities = .init()
    let profileID: String?
    let endpointID: String?
    let endpointName: String?
    let baseURL: URL
    let apiKey: String
    let model: String
    let contextWindowTokens: Int
    let capabilities: Set<MacRemoteModelCapability>

    nonisolated init(
        profileID: String? = nil,
        endpointID: String? = nil,
        endpointName: String? = nil,
        baseURL: URL,
        apiKey: String,
        model: String,
        contextWindowTokens: Int = 128_000,
        capabilities: Set<MacRemoteModelCapability> = [.chat]
    ) {
        self.profileID = profileID
        self.endpointID = endpointID
        self.endpointName = endpointName
        self.baseURL = baseURL
        self.apiKey = apiKey
        self.model = model
        self.contextWindowTokens = min(max(contextWindowTokens, 2_000), 4_000_000)
        self.capabilities = capabilities
    }
}

/// Test hosts must never touch the signed application's Keychain. Unlike a
/// configuration-store instance, this vault follows the canonical database
/// identity so independently constructed adapters resolve the same fixture.
private nonisolated final class MacEphemeralModelCredentialVault: @unchecked Sendable {
    static let shared = MacEphemeralModelCredentialVault()
    private let lock = NSLock()
    private var values: [URL: [String: String]] = [:]

    func read(account: String, database: URL) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return values[database]?[account]
    }

    func write(_ value: String?, account: String, database: URL) {
        lock.lock()
        defer { lock.unlock() }
        values[database, default: [:]][account] = value
        if values[database]?.isEmpty == true { values[database] = nil }
    }
}

nonisolated final class MacAppleChatConfigurationStore: @unchecked Sendable {
    static let shared = MacAppleChatConfigurationStore()

    private static let provider = "openai-compatible"
    private static let runtime = "remote"
    private static let schemaVersion = 1
    private static let healthSettingKey = "mac.remote-model.health.v1"
    private let keychainService = "com.gaojc.Nola.mac.chat-model"
    private let applicationPort: MacApplicationPort
    private let ephemeralCredentialDatabase: URL
    private let credentialReader: (@Sendable (String) throws -> String?)?

    init(applicationPort: MacApplicationPort = MacApplicationPort.shared,
         credentialReader: (@Sendable (String) throws -> String?)? = nil) {
        self.applicationPort = applicationPort
        self.ephemeralCredentialDatabase = applicationPort.databaseURL.standardizedFileURL.resolvingSymlinksInPath()
        self.credentialReader = credentialReader
    }

    func captureRunCredential(profileID: String, endpoint: String, model: String,
                              apiKey: String, wireProtocol: MacModelWireProtocol) throws -> MacRunModelCredential {
        let credential = try currentRunCredential(profileID: profileID)
        guard credential.endpoint == endpoint, credential.model == model,
              credential.wireProtocol == wireProtocol,
              credential.secretDigest == MacRunModelCredential.digest(apiKey) else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        return credential
    }

    func validateRunCredential(_ frozen: MacRunModelCredential) throws {
        guard try currentRunCredential(profileID: frozen.profileID) == frozen else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
    }
    func validateSiwcProfile(_ selected: MacSiwcProfileSelection) throws {
        guard let profile = try applicationPort.modelProfile(selected.profileID),
              profile.provider == "nola-chatgpt", profile.runtime == Self.runtime,
              profile.revision == selected.revision, profile.enabled, profile.deletedAtMs == nil,
              let source = decodeSource(profile.sourceJSON), source.wireProtocol == .siwcResponses,
              source.connectionID != nil, source.accountID != nil,
              source.endpointEnabled, source.modelEnabled else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        // The Host checks the account identity, revision and private session.
        // Swift never requests a Keychain/API-key representation of this route.
    }

    /// Resolve only the original canonical profile, never the current UI model.
    /// The caller must compare the returned route and digest with the frozen Run.
    func resolveColdRunCredential(profileID: String) throws -> (MacRunModelCredential, String) {
        let credential = try currentRunCredential(profileID: profileID)
        let secret: String?
        if credential.account.hasPrefix("account:") {
            secret = try MacModelConnectionService(application: applicationPort).credential(
                connectionID: credential.endpointID, endpoint: credential.endpoint,
                wireProtocol: credential.wireProtocol.rawValue).secret
        } else { secret = credential.account.isEmpty ? "" : try readAPIKey(account: credential.account) }
        guard let secret, MacRunModelCredential.digest(secret) == credential.secretDigest else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        return (credential, secret)
    }

    private func currentRunCredential(profileID: String) throws -> MacRunModelCredential {
        try applicationPort.prepare()
        guard let profile = try applicationPort.listModelProfiles().first(where: { $0.id == profileID }),
              profile.deletedAtMs == nil, profile.enabled,
              profile.provider == Self.provider, profile.runtime == Self.runtime,
              let source = decodeSource(profile.sourceJSON), source.endpointEnabled, source.modelEnabled,
              source.wireProtocol != .siwcResponses,
              let endpoint = URL(string: source.baseURL)?.absoluteString else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        let managedCredential = try source.connectionID.map {
            try MacModelConnectionService(application: applicationPort).credential(connectionID: $0,
                endpoint: source.baseURL, wireProtocol: (source.wireProtocol ?? .chatCompletions).rawValue)
        }
        let account = managedCredential?.reference ?? source.credentialAccount ?? keychainAccount(endpointID: source.endpointID)
        let secret: String
        if let managedCredential { secret = managedCredential.secret }
        else if account.isEmpty { secret = "" }
        else {
            guard let value = try readAPIKey(account: account) else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            secret = value
        }
        return MacRunModelCredential(profileID: profileID, endpointID: source.endpointID,
            account: account, endpoint: endpoint, model: profile.modelName,
            wireProtocol: source.wireProtocol ?? .chatCompletions,
            secretDigest: MacRunModelCredential.digest(secret))
    }

    func endpoints() -> [MacRemoteEndpointConfiguration] {
        (try? endpointsThrowing()) ?? []
    }

    func endpointsThrowing() throws -> [MacRemoteEndpointConfiguration] {
        try endpointSnapshot().endpoints
    }

    /// One canonical read binds the visible catalog to the exact membership
    /// and profile revisions a subsequent replacement must compare atomically.
    func endpointSnapshot(includeCredentials: Bool = true) throws -> MacRemoteEndpointSnapshot {
        try applicationPort.prepare()
        let profiles = try applicationPort.listModelProfiles().filter {
            $0.provider == Self.provider && $0.runtime == Self.runtime && $0.deletedAtMs == nil
        }
        guard profiles.allSatisfy(\.enabled) else { throw MacApplicationServiceError.invalidResponse }
        let decoded = try profiles.map { profile -> (profile: MacCanonicalModelProfile, source: RemoteSource) in
            guard let source = decodeSource(profile.sourceJSON) else {
                throw MacApplicationServiceError.invalidResponse
            }
            return (profile, source)
        }
        let editable = decoded.filter { $0.source.connectionID == nil }
        let revisions = Dictionary(uniqueKeysWithValues: editable.map { ($0.profile.id, $0.profile.revision) })
        return MacRemoteEndpointSnapshot(endpoints: try endpoints(from: editable, includeCredentials: includeCredentials), revisions: revisions)
    }

    private func endpoints(
        from decoded: [(profile: MacCanonicalModelProfile, source: RemoteSource)],
        includeCredentials: Bool = true
    ) throws -> [MacRemoteEndpointConfiguration] {
        let grouped = Dictionary(grouping: decoded, by: { $0.source.endpointID })
        return try grouped.compactMap { endpointID, records in
            guard let first = records.first else { return nil }
            let source = first.source
            guard records.allSatisfy({ $0.source.credentialAccount == source.credentialAccount }) else {
                throw MacApplicationServiceError.invalidResponse
            }
            let apiKey = includeCredentials ? try readAPIKey(
                account: source.credentialAccount ?? keychainAccount(endpointID: endpointID)
            ) : nil
            if includeCredentials && source.credentialAccount?.isEmpty == false && apiKey == nil {
                throw ConfigurationError.credentialUnavailable
            }
            // The endpoint owns its wire protocol. A model's reasoning metadata
            // is not evidence that a gateway implements the Responses route.
            let wireProtocol = source.wireProtocol ?? .chatCompletions
            let models = records.map { record in
                var reasoning = record.source.reasoning
                reasoning?.responses = wireProtocol.usesResponsesTransport
                return MacRemoteModelConfiguration(
                    id: record.profile.id,
                    modelName: record.profile.modelName,
                    displayName: record.profile.displayName,
                    isEnabled: record.source.modelEnabled,
                    priority: record.source.priority,
                    order: record.source.modelOrder,
                    capabilities: Set(record.source.capabilities),
                    executionCapabilities: record.source.executionCapabilities,
                    reasoning: reasoning,
                    reasoningSummarySupported: record.source.reasoningSummarySupported
                )
            }.sorted {
                if $0.order != $1.order { return $0.order < $1.order }
                return $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
            }
            return MacRemoteEndpointConfiguration(
                id: endpointID,
                name: source.endpointName,
                baseURL: source.baseURL,
                apiKey: apiKey ?? "",
                isEnabled: source.endpointEnabled,
                order: source.endpointOrder,
                models: models,
                wireProtocol: wireProtocol
            )
        }.sorted {
            if $0.order != $1.order { return $0.order < $1.order }
            return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
    }

    func configuredModels(
        for capability: MacRemoteModelCapability = .chat
    ) -> [MacRemoteModelOption] {
        let records = (try? decodedProfilesThrowing()) ?? []
        return records.compactMap { record in
            let source = record.source
            let capabilities = Set(source.capabilities)
            guard source.endpointEnabled,
                  source.modelEnabled,
                  capabilities.contains(capability)
            else { return nil }
            return MacRemoteModelOption(
                profileID: record.profile.id,
                endpointID: source.endpointID,
                endpointName: source.endpointName,
                modelName: record.profile.modelName,
                displayName: record.profile.displayName,
                priority: source.priority,
                endpointOrder: source.endpointOrder,
                modelOrder: source.modelOrder,
                capabilities: capabilities,
                executionCapabilityStatus: MacModelExecutionCapabilityStatus(source.executionCapabilities),
                selected: record.profile.selected
            )
        }.sorted(by: Self.modelOptionSort)
    }

    func configuredModelNames() -> [String] {
        configuredModels().map(\.modelName)
    }

    func load(
        model requestedModel: String? = nil,
        for capability: MacRemoteModelCapability = .chat
    ) -> MacAppleChatConfiguration? {
        try? loadThrowing(model: requestedModel, for: capability)
    }

    func loadThrowing(
        model requestedModel: String? = nil,
        for capability: MacRemoteModelCapability = .chat
    ) throws -> MacAppleChatConfiguration? {
        let candidates = configuredModels(for: capability)
        let option = try resolvePrimary(requestedModel, candidates: candidates, capability: capability)
        guard let option else { return nil }
        return try configuration(for: option)
    }

    private func resolvePrimary(_ requested: String?, candidates: [MacRemoteModelOption],
                                capability: MacRemoteModelCapability) throws -> MacRemoteModelOption? {
        if let requested {
            guard let option = candidates.first(where: { $0.profileID == requested }) else {
                throw MacRemoteModelRoutingError.selectedModelUnavailable
            }
            return option
        }
        guard capability == .chat else { return candidates.first }
        let resolution = try MacDefaultChatModelStore(application: applicationPort).resolve(
            candidates: candidates.map { MacChatModelCandidate(profileID: $0.profileID) })
        switch resolution {
        case let .resolved(profileID, _): return candidates.first { $0.profileID == profileID }
        case .noAutomaticCandidate: return nil
        case .unavailable: throw MacRemoteModelRoutingError.selectedModelUnavailable
        case .invalidConfiguration: throw MacRemoteModelRoutingError.invalidDefaultConfiguration
        }
    }

    func routePlan(
        preferredModel requestedModel: String?,
        for capability: MacRemoteModelCapability = .chat,
        now: Date = .now
    ) -> MacRemoteModelRoutePlan? {
        try? routePlanThrowing(preferredModel: requestedModel, for: capability, now: now)
    }

    func routePlanThrowing(
        preferredModel requestedModel: String?,
        for capability: MacRemoteModelCapability = .chat,
        now: Date = .now
    ) throws -> MacRemoteModelRoutePlan? {
        let candidates = configuredModels(for: capability)
        guard let preferred = try resolvePrimary(requestedModel, candidates: candidates, capability: capability) else { return nil }
        let healthy = candidates.filter { unavailableUntil(profileID: $0.profileID) <= now }
        guard healthy.contains(where: { $0.profileID == preferred.profileID }) else {
            throw MacRemoteModelRoutingError.allConfiguredModelsCoolingDown
        }
        let ordered = [preferred] + healthy.filter { $0.profileID != preferred.profileID }
        guard let primary = try configuration(for: preferred) else { return nil }
        if primary.wireProtocol == .siwcResponses { return MacRemoteModelRoutePlan(primary: primary, fallbacks: []) }
        let fallbacks = ordered.dropFirst().compactMap { option -> MacRemoteModelFallback? in
            guard let configuration = try? configuration(for: option) else { return nil }
            // CLI fallbacks replace only endpoint/model/key. They must never
            // inherit a different model's protocol, capabilities or defaults.
            let encoder = JSONEncoder()
            encoder.outputFormatting = .sortedKeys
            guard let candidateParameters = try? encoder.encode(configuration.parameters),
                  let primaryParameters = try? encoder.encode(primary.parameters),
                  configuration.wireProtocol == primary.wireProtocol,
                  configuration.executionCapabilities == primary.executionCapabilities,
                  configuration.reasoning == primary.reasoning,
                  candidateParameters == primaryParameters
            else { return nil }
            return MacRemoteModelFallback(
                profileID: option.profileID,
                endpointName: configuration.endpointName
                    ?? configuration.baseURL.host
                    ?? option.endpointName,
                endpoint: configuration.baseURL.absoluteString,
                apiKey: configuration.apiKey,
                model: configuration.model
            )
        }
        return MacRemoteModelRoutePlan(primary: primary, fallbacks: fallbacks)
    }

    func markQuotaExhausted(profileID: String, until resetDate: Date) {
        guard !profileID.isEmpty, resetDate > .now else { return }
        var health = healthRecords()
        health[profileID] = Int64(resetDate.timeIntervalSince1970 * 1_000)
        guard let data = try? JSONEncoder().encode(health),
              let json = String(data: data, encoding: .utf8) else { return }
        _ = try? applicationPort.putProductSetting(
            key: Self.healthSettingKey,
            valueJSON: json,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    func unavailableUntil(profileID: String) -> Date {
        let milliseconds = healthRecords()[profileID] ?? 0
        return Date(timeIntervalSince1970: TimeInterval(milliseconds) / 1_000)
    }

    private func configuration(for option: MacRemoteModelOption) throws -> MacAppleChatConfiguration? {
        let profiles = try decodedProfilesThrowing()
        guard let record = profiles.first(where: { $0.profile.id == option.profileID }) else {
            throw ConfigurationError.invalidModel
        }
        let parameters = try ModelParameters.decodeProfile(record.profile.parametersJSON)
        if let connectionID = record.source.connectionID {
            let source = record.source
            guard let wire = source.wireProtocol, let base = URL(string: source.baseURL),
                  let capabilities = source.executionCapabilities else {
                throw ConfigurationError.invalidModel
            }
            let selection = wire == .siwcResponses ? MacSiwcProfileSelection(profileID: record.profile.id, revision: record.profile.revision) : nil
            if let selection { try validateSiwcProfile(selection) }
            let secret: String
            if selection != nil { secret = "" }
            else { secret = try MacModelConnectionService(application: applicationPort).credential(
                connectionID: connectionID, endpoint: source.baseURL, wireProtocol: wire.rawValue).secret }
            var configuration = MacAppleChatConfiguration(profileID: record.profile.id,
                endpointID: source.endpointID, endpointName: source.endpointName, baseURL: base,
                apiKey: secret, model: record.profile.modelName,
                contextWindowTokens: parameters.maxContextLength,
                capabilities: Set(source.capabilities))
            configuration.profileRevision = record.profile.revision
            configuration.wireProtocol = wire
            configuration.siwcProfile = selection
            configuration.executionCapabilities = capabilities
            configuration.executionCapabilities.reasoningSummarySupported = source.reasoningSummarySupported
            configuration.reasoning = source.reasoning
            configuration.reasoning?.responses = wire.usesResponsesTransport
            configuration.parameters = parameters
            return configuration
        }
        let records = profiles.filter { $0.source.endpointID == option.endpointID }
        guard let endpoint = try endpoints(from: records).first,
              let model = endpoint.models.first(where: { $0.id == option.profileID }),
              let baseURL = URL(string: endpoint.baseURL) else { return nil }
        guard let executionCapabilities = model.declaredExecutionCapabilities else {
            throw MacModelCapabilityDeclarationError.confirmationRequired(modelName: option.modelName)
        }
        var configuration = MacAppleChatConfiguration(
            profileID: option.profileID,
            endpointID: option.endpointID,
            endpointName: option.endpointName,
            baseURL: baseURL,
            apiKey: endpoint.apiKey,
            model: option.modelName,
            contextWindowTokens: parameters.maxContextLength,
            capabilities: option.capabilities
        )
        configuration.profileRevision = record.profile.revision
        configuration.reasoning = model.reasoning
        configuration.executionCapabilities = executionCapabilities
        configuration.executionCapabilities.reasoningSummarySupported = model.reasoningSummarySupported
        configuration.wireProtocol = endpoint.wireProtocol
        configuration.parameters = parameters
        return configuration
    }

    private func healthRecords() -> [String: Int64] {
        guard let setting = try? applicationPort.productSetting(key: Self.healthSettingKey),
              let value = setting.valueJSON.data(using: .utf8),
              let decoded = try? JSONDecoder().decode([String: Int64].self, from: value) else {
            return [:]
        }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return decoded.filter { $0.value > now }
    }

    func replaceEndpoints(
        _ input: [MacRemoteEndpointConfiguration],
        parametersByModelID: [String: ModelParameters] = [:],
        expectedRevisions: [String: Int64]? = nil
    ) throws {
        let normalized = try normalize(input)
        try persistEndpoints(normalized, parametersByModelID: parametersByModelID,
                             expectedRevisions: expectedRevisions)
    }

    /// Validate only the edited connection. Other persisted connections may
    /// contain incomplete declarations and must not block or be rewritten by it.
    func replaceConnection(
        endpointID: String,
        draft: MacRemoteEndpointConfiguration?,
        parametersByModelID: [String: ModelParameters] = [:],
        expectedRevisions: [String: Int64]
    ) throws {
        guard draft == nil || draft?.id == endpointID else {
            throw ConfigurationError.invalidEndpoint
        }
        let current = try endpointSnapshot()
        var endpoints = current.endpoints.filter { $0.id != endpointID }
        if let draft {
            var normalized = try normalize([draft])[0]
            normalized.order = current.endpoints.first(where: { $0.id == endpointID })?.order
                ?? ((current.endpoints.map(\.order).max() ?? -1) + 1)
            let otherIDs = Set(endpoints.flatMap { $0.models.map(\.id) })
            guard normalized.models.allSatisfy({ !otherIDs.contains($0.id) }) else {
                throw ConfigurationError.invalidModel
            }
            endpoints.append(normalized)
        }
        let selectedIDs = Set(draft?.models.map(\.id) ?? [])
        try persistEndpoints(endpoints.sorted { $0.order < $1.order },
            parametersByModelID: parametersByModelID.filter { selectedIDs.contains($0.key) },
            expectedRevisions: expectedRevisions,
            preservedEndpointIDs: Set(current.endpoints.map(\.id)).subtracting([endpointID]))
    }

    private func persistEndpoints(
        _ normalized: [MacRemoteEndpointConfiguration],
        parametersByModelID: [String: ModelParameters],
        expectedRevisions: [String: Int64]?,
        preservedEndpointIDs: Set<String> = []
    ) throws {
        let current = try endpointSnapshot()
        let existingRecords = try decodedProfilesThrowing()
        let replacementExpectation = expectedRevisions ?? current.revisions
        let existingByID = Dictionary(uniqueKeysWithValues: existingRecords.map { ($0.profile.id, $0.profile) })
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let currentEndpoints = Dictionary(uniqueKeysWithValues: current.endpoints.map { ($0.id, $0) })
        let existingSources = Dictionary(grouping: existingRecords, by: { $0.source.endpointID })
        var credentialAccounts: [String: String?] = [:]
        var stagedCredentials: [(account: String, value: String)] = []
        for endpoint in normalized {
            if endpoint.apiKey.isEmpty {
                credentialAccounts[endpoint.id] = ""
            } else if currentEndpoints[endpoint.id]?.apiKey == endpoint.apiKey,
                      let source = existingSources[endpoint.id]?.first?.source {
                credentialAccounts[endpoint.id] = source.credentialAccount
            } else {
                let account = "endpoint:\(endpoint.id):\(UUID().uuidString.lowercased())"
                credentialAccounts[endpoint.id] = account
                stagedCredentials.append((account, endpoint.apiKey))
            }
        }

        var records: [(profile: MacCanonicalModelProfile, source: RemoteSource)] = []
        for endpoint in normalized {
            if preservedEndpointIDs.contains(endpoint.id) {
                records.append(contentsOf: existingRecords.filter { $0.source.endpointID == endpoint.id })
                continue
            }
            for model in endpoint.models {
                let source = RemoteSource(
                    schemaVersion: Self.schemaVersion,
                    endpointID: endpoint.id,
                    endpointName: endpoint.name,
                    baseURL: endpoint.baseURL,
                    endpointEnabled: endpoint.isEnabled,
                    endpointOrder: endpoint.order,
                    modelEnabled: model.isEnabled,
                    modelOrder: model.order,
                    priority: model.priority,
                    capabilities: model.capabilities.sorted { $0.rawValue < $1.rawValue },
                    executionCapabilities: model.declaredExecutionCapabilities,
                    reasoning: model.reasoning,
                    reasoningSummarySupported: model.reasoningSummarySupported,
                    wireProtocol: endpoint.wireProtocol,
                    credentialAccount: credentialAccounts[endpoint.id] ?? nil
                )
                let previous = existingByID[model.id]
                // A full catalog write must not create revisions for untouched
                // connections merely because JSONEncoder reordered object keys.
                let sourceJSON: String
                if let previous, decodeSource(previous.sourceJSON) == source {
                    sourceJSON = previous.sourceJSON
                } else {
                    sourceJSON = try encode(source)
                }
                let parametersJSON = if let parameters = parametersByModelID[model.id] {
                    try encode(parameters)
                } else if let previous {
                    previous.parametersJSON
                } else {
                    try encode(ModelParameters.webAlignedCloudDefaults)
                }
                records.append((MacCanonicalModelProfile(
                    id: model.id,
                    displayName: model.displayName,
                    provider: Self.provider,
                    modelName: model.modelName,
                    runtime: Self.runtime,
                    sourceJSON: sourceJSON,
                    parametersJSON: parametersJSON,
                    selected: false,
                    enabled: true,
                    revision: previous?.revision ?? 0,
                    createdAtMs: previous?.createdAtMs ?? now,
                    updatedAtMs: now,
                    deletedAtMs: nil
                ), source))
            }
        }

        guard !records.isEmpty else {
            try applicationPort.replaceModelProfiles(
                provider: Self.provider,
                runtime: Self.runtime,
                profiles: [],
                updatedAtMs: now,
                expectedRevisions: replacementExpectation
            )
            cleanupObsoleteCredentials(existingRecords: existingRecords, activeAccounts: [])
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            return
        }

        let eligible = records.filter {
            $0.source.endpointEnabled && $0.source.modelEnabled && $0.source.capabilities.contains(.chat)
        }.sorted { Self.recordSort($0, $1) }
        // This selection belongs to the provider catalog, not the global chat
        // preference. Editing priority must not change either user's intent.
        let selectedID = existingRecords.first(where: { record in
            record.profile.selected && records.contains(where: { $0.profile.id == record.profile.id })
        })?.profile.id ?? eligible.first?.profile.id ?? records.first?.profile.id

        let profiles = records.map { record in
            let profile = record.profile
            if preservedEndpointIDs.contains(record.source.endpointID),
               profile.selected == (profile.id == selectedID) {
                return profile
            }
            return MacCanonicalModelProfile(
                id: profile.id,
                displayName: profile.displayName,
                provider: profile.provider,
                modelName: profile.modelName,
                runtime: profile.runtime,
                sourceJSON: profile.sourceJSON,
                parametersJSON: profile.parametersJSON,
                selected: profile.id == selectedID,
                enabled: profile.enabled,
                revision: profile.revision,
                createdAtMs: profile.createdAtMs,
                updatedAtMs: profile.updatedAtMs,
                deletedAtMs: nil
            )
        }
        var stagedAccounts: [String] = []
        do {
            for credential in stagedCredentials {
                try writeAPIKey(credential.value, account: credential.account)
                stagedAccounts.append(credential.account)
            }
            try applicationPort.replaceModelProfiles(
                provider: Self.provider,
                runtime: Self.runtime,
                profiles: profiles,
                updatedAtMs: now,
                expectedRevisions: replacementExpectation
            )
        } catch {
            for account in stagedAccounts { deleteAPIKey(account: account) }
            throw error
        }
        cleanupObsoleteCredentials(
            existingRecords: existingRecords,
            activeAccounts: Set(records.map {
                $0.source.credentialAccount ?? keychainAccount(endpointID: $0.source.endpointID)
            }.filter { !$0.isEmpty })
        )
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    /// The editor uses the same pure admission rule as persistence before it
    /// offers Save; no credentials or model profiles are changed here.
    func validateEndpoints(_ input: [MacRemoteEndpointConfiguration]) throws {
        _ = try normalize(input)
    }

    // Diagnostics read the same product preference as chat, never catalog.selected.
    func displayValues() -> (baseURL: String, apiKey: String, model: String) {
        (try? displayValuesThrowing()) ?? ("", "", "")
    }

    func displayValuesThrowing() throws -> (baseURL: String, apiKey: String, model: String) {
        let records = try decodedProfilesThrowing()
        let options = configuredModels()
        let option = try resolvePrimary(nil, candidates: options, capability: .chat)
        let endpoints = try endpoints(from: records.filter { $0.source.endpointID == option?.endpointID })
        let endpoint = option.flatMap { selected in endpoints.first(where: { $0.id == selected.endpointID }) }
        return (
            endpoint?.baseURL ?? "",
            endpoint?.apiKey ?? "",
            option?.modelName ?? ""
        )
    }

    func save(baseURL: String, apiKey: String, model: String,
              executionCapabilities: MacModelExecutionCapabilities? = nil) throws {
        let endpoint = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let names = parseModelNames(model)
        guard endpoint.isEmpty == names.isEmpty else { throw ConfigurationError.incompleteProvider }
        guard !endpoint.isEmpty else {
            try replaceEndpoints([])
            return
        }
        guard let executionCapabilities else {
            throw MacModelCapabilityDeclarationError.confirmationRequired(modelName: names.joined(separator: ", "))
        }
        let models = names.enumerated().map { index, name in
            MacRemoteModelConfiguration(
                id: "remote-default-\(Data(name.utf8).base64EncodedString())",
                modelName: name,
                displayName: name,
                priority: max(0, 100 - index),
                order: index,
                capabilities: [.chat, .retrieval, .issueRepair],
                executionCapabilities: executionCapabilities
            )
        }
        try replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "default-api-endpoint",
            name: "API Provider",
            baseURL: endpoint,
            apiKey: apiKey,
            models: models
        )])
    }

    private func decodedProfilesThrowing() throws -> [(profile: MacCanonicalModelProfile, source: RemoteSource)] {
        try applicationPort.prepare()
        return try applicationPort.listModelProfiles()
            .filter { ($0.provider == Self.provider || $0.provider == "nola-chatgpt") && $0.runtime == Self.runtime && $0.enabled && $0.deletedAtMs == nil }
            .compactMap { profile in
                guard let source = decodeSource(profile.sourceJSON) else { return nil }
                guard (profile.provider == "nola-chatgpt") == (source.wireProtocol == .siwcResponses) else { return nil }
                return (profile, source)
            }
    }

    private func normalize(_ endpoints: [MacRemoteEndpointConfiguration]) throws -> [MacRemoteEndpointConfiguration] {
        var endpointIDs = Set<String>()
        var profileIDs = Set<String>()
        return try endpoints.enumerated().map { endpointIndex, original in
            var endpoint = original
            endpoint.id = endpoint.id.trimmingCharacters(in: .whitespacesAndNewlines)
            endpoint.name = endpoint.name.trimmingCharacters(in: .whitespacesAndNewlines)
            endpoint.baseURL = endpoint.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
            endpoint.apiKey = endpoint.apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
            endpoint.order = endpointIndex
            guard !endpoint.id.isEmpty,
                  endpointIDs.insert(endpoint.id).inserted,
                  !endpoint.name.isEmpty
            else { throw ConfigurationError.invalidEndpoint }
            guard MacRemoteEndpointURL.isValid(endpoint.baseURL) else {
                throw ConfigurationError.invalidEndpointURL
            }
            var modelNames = Set<String>()
            endpoint.models = try endpoint.models.enumerated().map { modelIndex, originalModel in
                var model = originalModel
                model.id = model.id.trimmingCharacters(in: .whitespacesAndNewlines)
                model.modelName = model.modelName.trimmingCharacters(in: .whitespacesAndNewlines)
                model.displayName = model.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
                model.order = modelIndex
                if model.displayName.isEmpty { model.displayName = model.modelName }
                guard model.hasExecutionCapabilityDeclaration else {
                    throw MacModelCapabilityDeclarationError.confirmationRequired(modelName: model.modelName)
                }
                guard (0...100).contains(model.priority) else {
                    throw ConfigurationError.invalidPriority
                }
                guard !model.id.isEmpty,
                      profileIDs.insert(model.id).inserted,
                      !model.modelName.isEmpty,
                      modelNames.insert(model.modelName).inserted,
                      !model.executionCapabilities.samplingRequiresNoReasoning
                        || (model.executionCapabilities.samplingParameters
                            && model.executionCapabilities.reasoning
                            && endpoint.wireProtocol == .responses
                            && model.reasoning?.control == .effort
                            && model.reasoning?.efforts.contains("none") == true)
                else { throw ConfigurationError.invalidModel }
                if var reasoning = model.reasoning {
                    reasoning.responses = endpoint.wireProtocol.usesResponsesTransport
                    guard reasoning.isValid(for: endpoint.wireProtocol),
                          ModelParameterLimits.outputTokens.contains(reasoning.outputLimit),
                          (reasoning.control == .automatic || model.executionCapabilities.reasoning),
                          reasoning.control == .automatic || !reasoning.options.isEmpty else {
                        throw ConfigurationError.invalidReasoning
                    }
                    model.reasoning = reasoning
                }
                return model
            }
            guard !endpoint.models.isEmpty else { throw ConfigurationError.endpointNeedsModel }
            return endpoint
        }
    }

    private static func modelOptionSort(_ lhs: MacRemoteModelOption, _ rhs: MacRemoteModelOption) -> Bool {
        if lhs.priority != rhs.priority { return lhs.priority > rhs.priority }
        if lhs.endpointOrder != rhs.endpointOrder { return lhs.endpointOrder < rhs.endpointOrder }
        if lhs.modelOrder != rhs.modelOrder { return lhs.modelOrder < rhs.modelOrder }
        return lhs.displayName.localizedCaseInsensitiveCompare(rhs.displayName) == .orderedAscending
    }

    private static func recordSort(
        _ lhs: (profile: MacCanonicalModelProfile, source: RemoteSource),
        _ rhs: (profile: MacCanonicalModelProfile, source: RemoteSource)
    ) -> Bool {
        if lhs.source.priority != rhs.source.priority { return lhs.source.priority > rhs.source.priority }
        if lhs.source.endpointOrder != rhs.source.endpointOrder {
            return lhs.source.endpointOrder < rhs.source.endpointOrder
        }
        if lhs.source.modelOrder != rhs.source.modelOrder { return lhs.source.modelOrder < rhs.source.modelOrder }
        return lhs.profile.displayName.localizedCaseInsensitiveCompare(rhs.profile.displayName) == .orderedAscending
    }

    private func parseModelNames(_ value: String) -> [String] {
        let separators = CharacterSet(charactersIn: ",;\n")
        var seen = Set<String>()
        return value.components(separatedBy: separators)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && seen.insert($0).inserted }
    }

    private func decodeSource(_ value: String) -> RemoteSource? {
        guard let data = value.data(using: .utf8),
              let source = try? JSONDecoder().decode(RemoteSource.self, from: data),
              source.schemaVersion == Self.schemaVersion else { return nil }
        return source
    }

    private func encode<Value: Encodable>(_ value: Value) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(value)
        guard let result = String(data: data, encoding: .utf8) else {
            throw ConfigurationError.invalidEncodedState
        }
        return result
    }

    private enum ConfigurationError: LocalizedError {
        case invalidReasoning
        case invalidEncodedState
        case incompleteProvider
        case invalidEndpoint
        case invalidEndpointURL
        case endpointNeedsModel
        case invalidModel
        case invalidPriority
        case credentialUnavailable

        var errorDescription: String? {
            switch self {
            case .invalidReasoning: return String(localized: "Invalid thinking configuration. Check the supported efforts, three ordered budgets, and output limit.")
            case .invalidEncodedState: return "The model configuration could not be encoded."
            case .incompleteProvider: return "The endpoint and model name must either both be present or both be empty."
            case .invalidEndpoint: return "Each endpoint needs a unique ID and a name."
            case .invalidEndpointURL: return String(localized: "Enter a valid HTTP(S) base URL without a username, password or fragment.", table: "ModelCenter")
            case .endpointNeedsModel: return "Each endpoint must contain at least one model."
            case .invalidModel: return "Each model needs a unique ID and a unique model name within its endpoint."
            case .invalidPriority: return String(localized: "Priority must be 0–100.", table: "ModelCenter")
            case .credentialUnavailable: return String(localized: "The model credential could not be read or saved securely. The model settings were not changed.", table: "ModelCenter")
            }
        }
    }

    private struct RemoteSource: Codable, Equatable {
        var connectionID: String? = nil
        var accountID: String? = nil
        let schemaVersion: Int
        let endpointID: String
        let endpointName: String
        let baseURL: String
        let endpointEnabled: Bool
        let endpointOrder: Int
        let modelEnabled: Bool
        let modelOrder: Int
        let priority: Int
        let capabilities: [MacRemoteModelCapability]
        var executionCapabilities: MacModelExecutionCapabilities? = nil
        var reasoning: MacReasoningConfiguration? = nil
        var reasoningSummarySupported: Bool? = nil
        var wireProtocol: MacModelWireProtocol? = nil
        var credentialAccount: String? = nil
    }


    private func keychainAccount(endpointID: String) -> String { "endpoint:\(endpointID)" }

    private struct AccountSecret: Codable {
        let connectionID: String
        let endpoint: String
        let protocols: [String]
        let secret: String
    }

    func storeAccountSecret(_ secret: String, connectionID: String, endpoint: String, protocols: [String]) throws -> String {
        let reference = "account:" + UUID().uuidString.lowercased()
        let payload = AccountSecret(connectionID: connectionID, endpoint: endpoint, protocols: protocols, secret: secret)
        guard !secret.isEmpty else { throw ConfigurationError.credentialUnavailable }
        try writeAPIKey(String(decoding: JSONEncoder().encode(payload), as: UTF8.self), account: reference)
        return reference
    }

    func resolveAccountSecret(_ reference: String, connectionID: String, endpoint: String, wireProtocol: String) throws -> String {
        guard reference.hasPrefix("account:"), let raw = try readAPIKey(account: reference),
              let bound = try? JSONDecoder().decode(AccountSecret.self, from: Data(raw.utf8)),
              bound.connectionID == connectionID, bound.endpoint == endpoint,
              bound.protocols.contains(wireProtocol), !bound.secret.isEmpty else { throw ConfigurationError.credentialUnavailable }
        return bound.secret
    }

    private func readAPIKey(account: String) throws -> String? {
        guard !account.isEmpty else { return nil }
        if let credentialReader { return try credentialReader(account) }
        if MacProcessRuntimePolicy.usesEphemeralCredentials {
            return MacEphemeralModelCredentialVault.shared.read(account: account, database: ephemeralCredentialDatabase)
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data,
              let value = String(data: data, encoding: .utf8) else {
            throw ConfigurationError.credentialUnavailable
        }
        return value
    }

    private func writeAPIKey(_ value: String, account: String) throws {
        if MacProcessRuntimePolicy.usesEphemeralCredentials {
            MacEphemeralModelCredentialVault.shared.write(value, account: account, database: ephemeralCredentialDatabase)
            return
        }
        let item: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecValueData as String: Data(value.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlock,
        ]
        guard SecItemAdd(item as CFDictionary, nil) == errSecSuccess else {
            throw ConfigurationError.credentialUnavailable
        }
    }

    private func deleteAPIKey(account: String) {
        guard !account.isEmpty else { return }
        if MacProcessRuntimePolicy.usesEphemeralCredentials {
            MacEphemeralModelCredentialVault.shared.write(nil, account: account, database: ephemeralCredentialDatabase)
            return
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }

    private func cleanupObsoleteCredentials(
        existingRecords: [(profile: MacCanonicalModelProfile, source: RemoteSource)],
        activeAccounts: Set<String>
    ) {
        let oldAccounts = Set(existingRecords.map {
            $0.source.credentialAccount ?? keychainAccount(endpointID: $0.source.endpointID)
        })
        for account in oldAccounts.subtracting(activeAccounts) { deleteAPIKey(account: account) }
    }
}
