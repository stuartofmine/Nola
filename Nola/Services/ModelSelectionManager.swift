// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelSelectionManager logic in the iOS service layer.
// - Interfaces:
//   - ModelInfo
//   - ModelSelectionManager
// - APIs:
//   - none
//
//
//  ModelSelectionManager.swift
//  Nola
//
//  Model selection and persistence management.
//

import Foundation

nonisolated enum RemoteModelEndpointURL {
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

nonisolated public struct ModelInfo: Codable, Identifiable, Hashable {
    public let id: String
    public let name: String
    public let source: String
    public let isLocal: Bool
    public let baseURL: String?
    public let apiKey: String?
    public let tlsCertificateSHA256: String?
    public let upstreamModelId: String?
    public let wireProtocol: ModelWireProtocol?
    public let executionCapabilities: ModelExecutionCapabilities?
    public let cloudCredential: IOSCloudCredentialBinding?

    public init(
        id: String,
        name: String,
        source: String,
        isLocal: Bool,
        baseURL: String? = nil,
        apiKey: String? = nil,
        upstreamModelId: String? = nil,
        wireProtocol: ModelWireProtocol? = nil,
        tlsCertificateSHA256: String? = nil,
        executionCapabilities: ModelExecutionCapabilities? = nil,
        cloudCredential: IOSCloudCredentialBinding? = nil
    ) {
        self.id = id
        self.name = name
        self.source = source
        self.isLocal = isLocal
        self.baseURL = baseURL
        self.apiKey = apiKey
        self.tlsCertificateSHA256 = tlsCertificateSHA256
        self.upstreamModelId = upstreamModelId
        self.wireProtocol = wireProtocol
        self.executionCapabilities = executionCapabilities
        self.cloudCredential = cloudCredential
    }

    public var effectiveUpstreamModelId: String {
        if let s = upstreamModelId?.trimmingCharacters(in: .whitespacesAndNewlines), !s.isEmpty {
            return s
        }
        return id
    }

    public var resolvedAPIKey: String? {
        guard isWebBackend, let baseURL else { return apiKey }
        return ModelSelectionManager.webHostToken(for: baseURL) ?? apiKey
    }

    public var redactedForPersistence: ModelInfo {
        guard !isLocal, apiKey != nil else { return self }
        return ModelInfo(
            id: id,
            name: name,
            source: source,
            isLocal: isLocal,
            baseURL: baseURL,
            apiKey: nil,
            upstreamModelId: upstreamModelId,
            wireProtocol: wireProtocol,
            tlsCertificateSHA256: tlsCertificateSHA256,
            executionCapabilities: executionCapabilities,
            cloudCredential: cloudCredential
        )
    }

    func withCloudCredential(_ binding: IOSCloudCredentialBinding?, apiKey: String?) -> ModelInfo {
        ModelInfo(id: id, name: name, source: source, isLocal: isLocal, baseURL: baseURL,
            apiKey: apiKey, upstreamModelId: upstreamModelId, wireProtocol: wireProtocol,
            tlsCertificateSHA256: tlsCertificateSHA256, executionCapabilities: executionCapabilities,
            cloudCredential: binding)
    }
}

nonisolated struct IOSResolvedCloudRunProfile {
    let model: ModelInfo
    let profileID: String
    let revision: Int64
    let parametersJSON: String
    var reasoningSummarySupported: Bool? = nil
    var reasoningSelection: AgentCoreJSONValue? = nil
    var reasoningTier: String = "balanced"
}

public final class ModelSelectionManager {
    nonisolated public static let webBackendIdPrefix = "web-backend:"
    public static var webBackendSource: String {
        NSLocalizedString("chat.model.web_backend", value: "Web backend", comment: "Web host model source")
    }

    nonisolated private static let pairedWebHostIdentityService = "com.nola.web-host.identity"
    nonisolated private static let webHostTokenService = "nola.web-host.token"
    nonisolated private static let sessionCredentialLock = NSLock()
    nonisolated(unsafe) private static var sessionWebHostTokens: [String: String] = [:]
    nonisolated(unsafe) private static var sessionWebHostIdentities: [String: String] = [:]

    public static func webBackendModel(
        baseURL: String? = nil,
        token: String? = nil,
        pinnedCertificateSHA256: String? = nil
    ) -> ModelInfo {
        if let baseURL {
            resetHybridCapabilitiesIfHostChanged(baseURL)
        }
        let tokenStored = baseURL.flatMap { url in
            token.map { saveWebHostToken($0, for: url) }
        } ?? false
        return ModelInfo(
            id: "\(webBackendIdPrefix)local-host",
            name: NSLocalizedString("chat.model.web_backend", value: "Web backend", comment: "Web host model name"),
            source: webBackendSource,
            isLocal: false,
            baseURL: baseURL,
            apiKey: tokenStored ? nil : token,
            upstreamModelId: "web-host",
            tlsCertificateSHA256: pinnedCertificateSHA256
        )
    }

    public static func getAvailableModels() -> [ModelInfo] {
        guard let store = IOSApplicationRuntime.shared else { return [] }
        do {
            return try availableModels(using: store)
        } catch {
            Logger.shared.log(.error, "ModelSelectionManager", "Failed to read canonical model catalog",
                              metadata: ["error": error.localizedDescription])
            return []
        }
    }

    public static func getLastSelectedModel() -> ModelInfo? {
        guard let store = IOSApplicationRuntime.shared else { return nil }
        do {
            return try lastSelectedModel(using: store)
        } catch {
            Logger.shared.log(.error, "ModelSelectionManager", "Failed to read canonical model selection",
                              metadata: ["error": error.localizedDescription])
            return nil
        }
    }

    public static func resolvedCloudModelForRun(_ selected: ModelInfo) throws -> ModelInfo {
        try resolvedCloudProfileForRun(selected).model
    }

    static func resolvedCloudProfileForRun(_ selected: ModelInfo) throws -> IOSResolvedCloudRunProfile {
        try resolvedCloudProfileForRun(selected, parameters: nil)
    }

    static func resolvedCloudProfileForRun(_ selected: ModelInfo, parameters: ModelParameters?) throws -> IOSResolvedCloudRunProfile {
        guard let store = IOSApplicationRuntime.shared else { throw IOSCloudCredentialError.unavailable }
        return try resolvedCloudProfileForRun(selected, using: store, parameters: parameters)
    }

    static func resolvedCloudModelForRun(_ selected: ModelInfo, using store: IOSApplicationRuntime) throws -> ModelInfo {
        try resolvedCloudProfileForRun(selected, using: store).model
    }

    static func resolvedCloudProfileForRun(_ selected: ModelInfo, using store: IOSApplicationRuntime,
        credentials: IOSCloudCredentialOwner = .shared, parameters: ModelParameters? = nil) throws -> IOSResolvedCloudRunProfile {
        guard let profile = try store.modelProfile(id: selected.id),
              profile.enabled, profile.deletedAtMs == nil,
              profile.runtime == "remote",
              let current = modelInfo(from: profile),
              !current.isLocal, !current.isWebBackend,
              current.id == profile.id, current.source == profile.provider,
              current.effectiveUpstreamModelId == profile.modelName else {
            throw NSError(domain: "Nola.ModelExecutionCapabilities", code: 3,
                          userInfo: [NSLocalizedDescriptionKey: "The selected cloud model profile is no longer available."])
        }
        // Core owns tier→wire selection and capability validation for all Hosts.
        var request: [String: AgentCoreJSONValue] = [:]
        if let parameters {
            request["max_output_tokens"] = .signedInteger(Int64(parameters.maxOutputTokens))
            if let preference = parameters.reasoningPreference {
                request["reasoning_selection"] = preference == "automatic" ? .null
                    : .object(["kind": .string("effort"), "value": .string(preference)])
            } else if let tier = parameters.reasoningTier { request["reasoning_tier"] = .string(tier) }
        }
        let resolution = try store.executeAgentCommand(.object(["type":.string("resolve_remote_model_profile"),
            "profile_id":.string(profile.id),"expected_revision":.signedInteger(profile.revision),"request":.object(request)]))
        let runtime = resolution.objectFields?["resolved"]?.objectFields?["runtime"]?.objectFields
        let selection = runtime?["reasoning_selection"] ?? .null
        let key: String?
        var binding = current.cloudCredential
        do {
            if let source = try JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
               let connectionID = source["connectionID"] as? String {
                guard let endpoint = current.baseURL, let wire = current.wireProtocol else {
                    throw IOSCloudCredentialError.unavailable
                }
                let credential = try IOSModelConnectionService(application: store, credentials: credentials).credential(
                    connectionID: connectionID, endpoint: endpoint, wireProtocol: wire.rawValue)
                key = credential.secret
                // Only the validated connection can establish anonymous admission.
                // A profile's missing binding or a missing account secret cannot.
                if credential.reference.isEmpty { binding = .anonymous }
            } else {
                key = try credentials.resolve(model: current)
            }
        } catch {
            throw IOSCloudRunCredentialResolutionError(underlyingError: error)
        }
        let summary = resolution.objectFields?["resolved"]?.objectFields?["runtime"]?
            .objectFields?["model_capabilities"]?.objectFields?["reasoning_summary_supported"] ?? .null
        let summarySupported = try JSONDecoder().decode(Bool?.self, from: JSONEncoder().encode(summary))
        return IOSResolvedCloudRunProfile(model: current.withCloudCredential(binding, apiKey: key),
            profileID: profile.id, revision: profile.revision, parametersJSON: profile.parametersJSON,
            reasoningSummarySupported: summarySupported, reasoningSelection: selection == .null ? nil : selection,
            reasoningTier: runtime?["reasoning_tier"]?.stringScalar ?? "balanced")
    }

    static func availableModels(using store: IOSApplicationRuntime) throws -> [ModelInfo] {
        let stored = try store.listModelProfiles().filter { $0.enabled && $0.deletedAtMs == nil }.compactMap(modelInfo(from:))
        // Catalog suggestions are not configured profiles. A query must not
        // turn them into durable models or choose one on the user's behalf.
        return stored
    }

    static func lastSelectedModel(using store: IOSApplicationRuntime) throws -> ModelInfo? {
        guard let setting = try store.productSetting(key: "chat.default_model") else { return nil }
        try store.validateDefaultChatModel(valueJSON: setting.valueJSON)
        struct Configuration: Decodable {
            struct Selection: Decodable { let kind: String; let profile_id: String? }
            let selection: Selection
        }
        let selection = try JSONDecoder().decode(Configuration.self, from: Data(setting.valueJSON.utf8)).selection
        if selection.kind == "automatic" { return nil }
        guard let id = selection.profile_id, let profile = try store.modelProfile(id: id),
              profile.enabled, profile.deletedAtMs == nil, let model = modelInfo(from: profile) else {
            throw IOSApplicationRuntimeError.rejected("The selected default chat model is unavailable.")
        }
        return model
    }

    public static func saveSelectedModel(_ model: ModelInfo) {
        guard let store = IOSApplicationRuntime.shared else { return }
        let secured = persistWebHostTokens(in: [model])[0]
        do {
            try commitSelectedModel(secured, using: store)
        } catch {
            Logger.shared.log(
                .error,
                "ModelSelectionManager",
                "Failed to commit canonical model selection",
                metadata: ["modelId": secured.id, "error": error.localizedDescription]
            )
            return
        }
        if secured.isWebBackend, let baseURL = secured.baseURL {
            resetHybridCapabilitiesIfHostChanged(baseURL)
        }
    }

    static func commitSelectedModel(_ model: ModelInfo, using store: IOSApplicationRuntime) throws {
        let defaultSnapshot = try store.productSetting(key: "chat.default_model")
        let existing = try store.modelProfile(id: model.id)
        guard existing == nil || (existing?.enabled == true && existing?.deletedAtMs == nil) else {
            throw IOSApplicationRuntimeError.rejected("The selected default chat model is unavailable.")
        }
        let managedConnection = existing.flatMap { try? JSONSerialization.jsonObject(with: Data($0.sourceJSON.utf8)) as? [String: Any] }?["connectionID"] as? String
        if managedConnection != nil {
            _ = try resolvedCloudProfileForRun(model, using: store)
        } else if !model.isLocal, !model.isWebBackend {
            _ = try publishCloudModel(model, existing: existing, selected: existing?.selected ?? false, using: store)
        } else {
            guard let expected = UInt64(exactly: existing?.revision ?? 0) else {
                throw IOSApplicationRuntimeError.rejected("Invalid model profile revision.")
            }
            _ = try store.upsertModelProfileCAS(canonicalProfile(for: model,
                selected: existing?.selected ?? false, existing: existing), expectedRevision: expected)
        }
        // Profile publication (including secure credentials) must succeed first.
        // Scope-local selected flags are not the global chat preference.
        let data = try JSONSerialization.data(withJSONObject: ["version": 1,
            "selection": ["kind": "profile", "profile_id": model.id]])
        _ = try store.putProductSettingChecked(key: "chat.default_model",
            valueJSON: String(decoding: data, as: UTF8.self), expectedRevision: defaultSnapshot?.revision,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
    }

    public static func saveAddedModels(_ models: [ModelInfo]) {
        guard let store = IOSApplicationRuntime.shared else { return }
        for model in persistWebHostTokens(in: models) {
            if !model.isLocal, !model.isWebBackend {
                do {
                    let existing = try store.modelProfile(id: model.id)
                    _ = try publishCloudModel(model, existing: existing, selected: existing?.selected ?? false, using: store)
                } catch { Logger.shared.log(.error, "ModelSelectionManager", "Cloud model publication failed", metadata: ["error": error.localizedDescription]) }
                continue
            }
            let existing = try? store.modelProfile(id: model.id)
            try? store.upsertModelProfile(canonicalProfile(
                for: model,
                selected: (existing ?? nil)?.selected ?? false,
                existing: existing ?? nil
            ))
        }
    }

    public static func loadAddedModels() -> [ModelInfo]? {
        let models = getAvailableModels()
        return models.isEmpty ? nil : models
    }

    public static func addModel(_ model: ModelInfo) {
        saveAddedModels([model])
    }

    public static func addConfiguredCloudModel(_ model: ModelInfo) throws {
        guard !model.isLocal, !model.isWebBackend,
              let store = IOSApplicationRuntime.shared else {
            throw NSError(domain: "Nola.ModelConfiguration", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "Cloud model storage is unavailable."])
        }
        let existing = try store.modelProfile(id: model.id)
        _ = try publishCloudModel(model, existing: existing, selected: existing?.selected ?? false, using: store)
    }

    @discardableResult
    static func publishCloudModel(_ model: ModelInfo, existing: IOSCanonicalModelProfile?, selected: Bool,
        using store: IOSApplicationRuntime, credentials: IOSCloudCredentialOwner = .shared) throws -> UInt64 {
        guard !model.isLocal, !model.isWebBackend,
              existing == nil || (existing?.id == model.id && existing?.deletedAtMs == nil &&
                existing?.runtime == "remote" && existing?.provider == model.source) else {
            throw IOSCloudCredentialError.invalidReference
        }
        func commit(_ secured: ModelInfo) throws -> UInt64 {
            guard let expected = UInt64(exactly: existing?.revision ?? 0) else {
                throw IOSCloudCredentialError.invalidReference
            }
            return try store.upsertModelProfileCAS(canonicalProfile(for: secured, selected: selected, existing: existing),
                expectedRevision: expected)
        }
        if model.apiKey == nil, model.cloudCredential != nil {
            _ = try credentials.resolve(model: model)
            return try commit(model.redactedForPersistence)
        }
        guard let secret = model.apiKey else { throw IOSCloudCredentialError.invalidReference }
        let (_, revision) = try credentials.publish(model: model, secret: secret) { binding in
            do {
                let revision = try commit(model.withCloudCredential(binding, apiKey: nil))
                guard let exact = Int64(exactly: revision) else { return .unconfirmed }
                return .committed(revision: exact)
            } catch IOSApplicationRuntimeError.revisionConflict { return .rejected }
              catch IOSApplicationRuntimeError.rejected { return .rejected }
        }
        return UInt64(revision)
    }

    /// Update only one configured cloud model; the Application keeps its
    /// parameter record and selected identity in the same profile mutation.
    public static func updateExecutionCapabilities(
        for model: ModelInfo,
        to capabilities: ModelExecutionCapabilities
    ) throws -> ModelInfo {
        guard let store = IOSApplicationRuntime.shared else {
            throw NSError(domain: "Nola.ModelExecutionCapabilities", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "The configured cloud model is no longer available."])
        }
        return try updateExecutionCapabilities(for: model, to: capabilities, using: store)
    }

    static func updateExecutionCapabilities(
        for model: ModelInfo,
        to capabilities: ModelExecutionCapabilities,
        using store: IOSApplicationRuntime
    ) throws -> ModelInfo {
        guard let existing = try store.modelProfile(id: model.id),
              existing.runtime == "remote", !model.isLocal, !model.isWebBackend,
              let current = modelInfo(from: existing) else {
            throw NSError(domain: "Nola.ModelExecutionCapabilities", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "The configured cloud model is no longer available."])
        }
        guard !capabilities.samplingRequiresNoReasoning else {
            throw NSError(domain: "Nola.ModelExecutionCapabilities", code: 2,
                          userInfo: [NSLocalizedDescriptionKey: "Direct Chat Completions cannot configure conditional sampling."])
        }
        let source = try JSONSerialization.jsonObject(with: Data(existing.sourceJSON.utf8)) as? [String: Any]
        if let connectionID = source?["connectionID"] as? String {
            let service = IOSModelConnectionService(application: store)
            guard let connection = try service.connections().first(where: { $0.id == connectionID }),
                  let wireProtocol = current.wireProtocol else { throw IOSCloudCredentialError.invalidReference }
            let value = try JSONDecoder().decode(AgentCoreJSONValue.self,
                from: JSONEncoder().encode(capabilities))
            let _: IOSModelConnectionSnapshot = try service.command([
                "type": .string("set_connection_capability_override"),
                "connection_id": .string(connectionID), "expected_revision": .signedInteger(connection.revision),
                "model_id": .string(current.effectiveUpstreamModelId), "protocol": .string(wireProtocol.rawValue),
                "capabilities": value, "updated_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)),
            ])
            guard let profile = try store.modelProfile(id: model.id),
                  let updated = modelInfo(from: profile) else { throw IOSApplicationRuntimeError.invalidResponse }
            return updated
        }
        let updated = ModelInfo(
            id: current.id, name: current.name, source: current.source, isLocal: false,
            baseURL: current.baseURL, apiKey: current.apiKey,
            upstreamModelId: current.upstreamModelId,
            wireProtocol: current.wireProtocol,
            tlsCertificateSHA256: current.tlsCertificateSHA256,
            executionCapabilities: capabilities,
            cloudCredential: current.cloudCredential
        )
        _ = try publishCloudModel(updated, existing: existing, selected: existing.selected, using: store)
        return updated
    }

    public static func removeModel(_ model: ModelInfo) {
        guard let store = IOSApplicationRuntime.shared else { return }
        _ = try? store.deleteModelProfile(id: model.id)
        if model.isWebBackend, let baseURL = model.baseURL {
            deleteWebHostToken(for: baseURL)
            forgetWebHostIdentity(for: URL(string: baseURL) ?? URL(fileURLWithPath: "/"))
        }
    }


    public static func hasAvailableModels() -> Bool {
        !getAvailableModels().isEmpty
    }

    public static func firstWebBackendModel() -> ModelInfo? {
        getAvailableModels().first { $0.isWebBackend }
    }

    public static func pairedWebHostIdentity(for baseURL: URL) -> String? {
        let account = webHostIdentityKey(baseURL)
        sessionCredentialLock.lock()
        let sessionIdentity = sessionWebHostIdentities[account]
        sessionCredentialLock.unlock()
        if let sessionIdentity { return sessionIdentity }
        guard let data = KeychainService.read(
            service: pairedWebHostIdentityService,
            account: account
        ) else { return nil }
        let identity = String(data: data, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .nilIfEmpty
        if let identity {
            sessionCredentialLock.lock()
            sessionWebHostIdentities[account] = identity
            sessionCredentialLock.unlock()
        }
        return identity
    }

    public static func hasPairedWebHost(for model: ModelInfo) -> Bool {
        guard model.isWebBackend,
              let baseURLText = model.baseURL?.trimmingCharacters(in: .whitespacesAndNewlines),
              let baseURL = URL(string: baseURLText) else {
            return false
        }
        return pairedWebHostIdentity(for: baseURL) != nil
    }

    @discardableResult
    public static func trustWebHostIdentity(_ hostId: String, for baseURL: URL) -> Bool {
        let normalized = hostId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (16...128).contains(normalized.count),
              normalized.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else {
            return false
        }
        let key = webHostIdentityKey(baseURL)
        if let existing = pairedWebHostIdentity(for: baseURL), !existing.isEmpty {
            return existing == normalized
        }
        let saved = KeychainService.save(
            service: pairedWebHostIdentityService,
            account: key,
            value: Data(normalized.utf8)
        )
        if saved {
            sessionCredentialLock.lock()
            sessionWebHostIdentities[key] = normalized
            sessionCredentialLock.unlock()
        }
        return saved
    }

    public static func forgetWebHostIdentity(for baseURL: URL) {
        let key = webHostIdentityKey(baseURL)
        sessionCredentialLock.lock()
        sessionWebHostIdentities.removeValue(forKey: key)
        sessionCredentialLock.unlock()
        _ = KeychainService.delete(
            service: pairedWebHostIdentityService,
            account: key
        )
    }

    #if DEBUG
    @discardableResult
    public static func installEphemeralWebHostCredential(
        token: String,
        identity hostID: String,
        for baseURL: URL
    ) -> Bool {
        let normalizedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedIdentity = hostID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedToken.isEmpty,
              (16...128).contains(normalizedIdentity.count),
              normalizedIdentity.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else {
            return false
        }
        let account = webHostIdentityKey(baseURL)
        sessionCredentialLock.lock()
        sessionWebHostTokens[account] = normalizedToken
        sessionWebHostIdentities[account] = normalizedIdentity
        sessionCredentialLock.unlock()
        return true
    }
    #endif

    nonisolated private static func webHostIdentityKey(_ baseURL: URL) -> String {
        var value = baseURL.absoluteString
        if var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) {
            let normalizedScheme = components.scheme?.lowercased()
            let normalizedHost = components.host?.lowercased()
            components.scheme = normalizedScheme
            components.host = normalizedHost
            value = components.url?.absoluteString ?? value
        }
        while value.last == "/" { value.removeLast() }
        return value
    }

    nonisolated public static func webHostToken(for baseURL: String) -> String? {
        let account = normalizedWebHostURL(baseURL)
        sessionCredentialLock.lock()
        let sessionToken = sessionWebHostTokens[account]
        sessionCredentialLock.unlock()
        if let sessionToken { return sessionToken }
        if let data = KeychainService.read(service: webHostTokenService, account: account),
           let token = String(data: data, encoding: .utf8),
           !token.isEmpty {
            sessionCredentialLock.lock()
            sessionWebHostTokens[account] = token
            sessionCredentialLock.unlock()
            return token
        }
        return nil
    }

    @discardableResult
    nonisolated public static func saveWebHostToken(_ token: String, for baseURL: String) -> Bool {
        let token = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { return false }
        let account = normalizedWebHostURL(baseURL)
        let saved = KeychainService.save(
            service: webHostTokenService,
            account: account,
            value: Data(token.utf8)
        )
        if saved {
            sessionCredentialLock.lock()
            sessionWebHostTokens[account] = token
            sessionCredentialLock.unlock()
        }
        return saved
    }

    public static func deleteWebHostToken(for baseURL: String) {
        if let url = URL(string: baseURL) { NolaTailcatTransport.unregister(baseURL: url) }
        let account = normalizedWebHostURL(baseURL)
        sessionCredentialLock.lock()
        sessionWebHostTokens.removeValue(forKey: account)
        sessionCredentialLock.unlock()
        _ = KeychainService.delete(service: webHostTokenService, account: account)
    }

    nonisolated static func normalizedWebHostURL(_ raw: String) -> String {
        guard let url = URL(string: raw) else {
            return raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        }
        return webHostIdentityKey(url)
    }

    private static func resetHybridCapabilitiesIfHostChanged(_ baseURL: String) {
        do {
            try IOSHybridCapabilitySettings.bindHost(baseURL)
        } catch {
            Logger.shared.log(
                .error,
                "ModelSelectionManager",
                "Failed to bind canonical hybrid capability settings",
                metadata: ["error": error.localizedDescription]
            )
        }
    }

nonisolated static func modelInfo(from profile: IOSCanonicalModelProfile) -> ModelInfo? {
    guard profile.enabled, profile.deletedAtMs == nil,
          ["remote", "mlx", "agent-host"].contains(profile.runtime),
          let data = profile.sourceJSON.data(using: .utf8),
          let source = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
    func decode<T: Decodable>(_ key: String, as type: T.Type) -> T? {
        guard let value = source[key], let bytes = try? JSONSerialization.data(withJSONObject: value) else { return nil }
        return try? JSONDecoder().decode(type, from: bytes)
    }
    return ModelInfo(id: profile.id, name: profile.displayName, source: profile.provider,
        isLocal: profile.runtime == "mlx", baseURL: source["baseURL"] as? String,
        upstreamModelId: profile.modelName,
        wireProtocol: (source["wireProtocol"] as? String).flatMap(ModelWireProtocol.init(rawValue:)),
        tlsCertificateSHA256: source["tlsCertificateSHA256"] as? String,
        executionCapabilities: decode("executionCapabilities", as: ModelExecutionCapabilities.self),
        cloudCredential: decode("cloudCredential", as: IOSCloudCredentialBinding.self))
}

private static func canonicalProfile(
    for model: ModelInfo,
    selected: Bool,
    existing: IOSCanonicalModelProfile?
) -> IOSCanonicalModelProfile {
    let now = Int64(Date().timeIntervalSince1970 * 1_000)
    let sourceData = try? JSONEncoder().encode(model.redactedForPersistence)
    var source = existing.flatMap { $0.sourceJSON.data(using: .utf8) }
        .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
    for key in ["id", "name", "source", "isLocal", "baseURL", "apiKey", "tlsCertificateSHA256",
                "upstreamModelId", "wireProtocol", "executionCapabilities", "cloudCredential"] {
        source.removeValue(forKey: key)
    }
    if let sourceData, let fields = try? JSONSerialization.jsonObject(with: sourceData) as? [String: Any] {
        source.merge(fields) { _, new in new }
    }
    let sourceJSON = (try? JSONSerialization.data(withJSONObject: source, options: [.sortedKeys]))
        .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    let parameterData = try? JSONEncoder().encode(ModelParameters.defaults(isLocal: model.isLocal))
    let defaultParameters = parameterData.flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    return IOSCanonicalModelProfile(
        id: model.id,
        displayName: model.name,
        provider: model.source,
        modelName: model.effectiveUpstreamModelId,
        runtime: model.isLocal ? "mlx" : (model.isWebBackend ? "agent-host" : "remote"),
        sourceJSON: sourceJSON,
        parametersJSON: existing?.parametersJSON ?? defaultParameters,
        selected: selected,
        enabled: existing?.enabled ?? true,
        revision: existing?.revision ?? 0,
        createdAtMs: existing?.createdAtMs ?? now,
        updatedAtMs: now,
        deletedAtMs: existing?.deletedAtMs
    )
}

    private static func persistWebHostTokens(in models: [ModelInfo]) -> [ModelInfo] {
        models.map { model in
            guard model.isWebBackend else { return model }
            if model.isWebBackend,
               let baseURL = model.baseURL,
               let token = model.apiKey,
               !saveWebHostToken(token, for: baseURL) {
                Logger.shared.log(
                    .error,
                    "security",
                    "Web Host token could not be saved to Keychain",
                    metadata: ["modelId": model.id]
                )
                return model
            }
            return model.redactedForPersistence
        }
    }

}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}

nonisolated public extension ModelInfo {
    var isWebBackend: Bool {
        id.hasPrefix(ModelSelectionManager.webBackendIdPrefix)
    }
}

/// Marks credential admission separately from profile validation without logging secret-bearing errors.
nonisolated struct IOSCloudRunCredentialResolutionError: Error {
    let underlyingError: Error
}
