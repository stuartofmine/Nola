// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated struct IOSModelConnectionSnapshot: Codable, Identifiable, Sendable {
    let revision: Int64
    var document: Document
    var id: String { document.connection.id }
    struct Document: Codable, Sendable { let format_version: Int; var connection: Connection; var catalog: Catalog? }
    struct Connection: Codable, Sendable {
        let id: String
        var display_name: String
        let base_url: String
        let protocols: [String]
        let account_id: String?
        let source: String
        let source_id: String?
        var enabled: Bool

        init(
            id: String,
            display_name: String,
            base_url: String,
            protocols: [String],
            account_id: String?,
            source: String,
            source_id: String?,
            enabled: Bool
        ) {
            self.id = id
            self.display_name = display_name
            self.base_url = base_url
            self.protocols = protocols
            self.account_id = account_id
            self.source = source
            self.source_id = source_id
            self.enabled = enabled
        }

        private enum CodingKeys: String, CodingKey {
            case id, display_name, base_url, protocols, account_id, source, source_id, enabled
        }

        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            id = try v.decode(String.self, forKey: .id)
            display_name = try v.decodeIfPresent(String.self, forKey: .display_name) ?? id
            base_url = try v.decode(String.self, forKey: .base_url)
            protocols = try v.decodeIfPresent([String].self, forKey: .protocols) ?? ["chat_completions"]
            account_id = try v.decodeIfPresent(String.self, forKey: .account_id)
            source = try v.decodeIfPresent(String.self, forKey: .source) ?? "manual"
            source_id = try v.decodeIfPresent(String.self, forKey: .source_id)
            enabled = try v.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        }
    }
    struct Catalog: Codable, Sendable { let observed_at_ms: Int64; let source: String; let models: [Model] }
    struct Model: Codable, Identifiable, Sendable {
        let model_id: String
        let display_name: String
        let `protocol`: String
        let context_window: UInt64?
        let output_limit: UInt64?
        let modalities: [String]
        let reasoning_levels: [String]
        var summary_supported: Bool? = nil
        let execution_capabilities: AgentCoreJSONValue?
        var id: String { `protocol` + ":" + model_id }
    }

    init(revision: Int64, document: Document) {
        self.revision = revision
        self.document = document
    }

    private enum CodingKeys: String, CodingKey {
        case revision, document, connection, catalog
    }

    init(from decoder: Decoder) throws {
        let v = try decoder.container(keyedBy: CodingKeys.self)
        if let document = try v.decodeIfPresent(Document.self, forKey: .document) {
            self.revision = try v.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
            self.document = document
            return
        }
        if let wrapped = try v.decodeIfPresent(Connection.self, forKey: .connection) {
            self.revision = try v.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
            let catalog = try v.decodeIfPresent(Catalog.self, forKey: .catalog)
            self.document = Document(format_version: 1, connection: wrapped, catalog: catalog)
            return
        }
        let flat = try Connection(from: decoder)
        let catalog = try v.decodeIfPresent(Catalog.self, forKey: .catalog)
        self.revision = try v.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
        self.document = Document(format_version: 1, connection: flat, catalog: catalog)
    }

    func encode(to encoder: Encoder) throws {
        var v = encoder.container(keyedBy: CodingKeys.self)
        try v.encode(revision, forKey: .revision)
        try v.encode(document, forKey: .document)
    }
}
nonisolated struct IOSModelAccountSnapshot: Codable, Identifiable, Sendable {
    let revision: Int64
    let account: Account
    var id: String { account.id }
    struct Account: Codable, Sendable {
        let id: String; let display_name: String; let source: String
        let credential_ref: String?; let state: String
    }

    init(revision: Int64, account: Account) {
        self.revision = revision
        self.account = account
    }

    private enum CodingKeys: String, CodingKey {
        case revision, account
    }

    init(from decoder: Decoder) throws {
        let v = try decoder.container(keyedBy: CodingKeys.self)
        if let account = try v.decodeIfPresent(Account.self, forKey: .account) {
            self.revision = try v.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
            self.account = account
            return
        }
        let flat = try Account(from: decoder)
        self.revision = try v.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
        self.account = flat
    }

    func encode(to encoder: Encoder) throws {
        var v = encoder.container(keyedBy: CodingKeys.self)
        try v.encode(revision, forKey: .revision)
        try v.encode(account, forKey: .account)
    }
}

nonisolated enum IOSModelConnectionInputError: LocalizedError, Equatable {
    case invalidEndpoint
    case missingModelID
    var errorDescription: String? {
        if self == .missingModelID { return NSLocalizedString("model.setup.model_required", value: "Enter the model ID supplied by your service.", comment: "Missing model ID") }
        return NSLocalizedString("model.connections.invalid_endpoint", value: "Enter an HTTP or HTTPS API address without a username, password, query, or fragment.", comment: "Invalid API endpoint")
    }
}

nonisolated final class IOSModelConnectionService: Sendable {
    let application: IOSApplicationRuntime
    let credentials: IOSCloudCredentialOwner
    init(application: IOSApplicationRuntime, credentials: IOSCloudCredentialOwner = .shared) {
        self.application = application; self.credentials = credentials
    }
    private var now: AgentCoreJSONValue { .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)) }
    func command<T: Decodable>(_ command: [String: AgentCoreJSONValue]) throws -> T {
        let value = try application.executeAgentCommand(.object(["type": .string("model"), "command": .object(command)]))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    private func decodeCollection<T: Decodable>(_ value: AgentCoreJSONValue, wrapperKeys: [String]) throws -> [T] {
        let elements: [AgentCoreJSONValue]
        switch value {
        case .array(let array):
            elements = array
        case .object(let fields):
            var matched: [AgentCoreJSONValue]? = nil
            for key in wrapperKeys {
                if case .array(let array)? = fields[key] {
                    matched = array
                    break
                }
            }
            elements = matched ?? []
        default:
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let encoder = JSONEncoder()
        let decoder = JSONDecoder()
        return elements.compactMap { item in
            guard let data = try? encoder.encode(item) else { return nil }
            return try? decoder.decode(T.self, from: data)
        }
    }

    func connections() throws -> [IOSModelConnectionSnapshot] {
        let value = try application.executeAgentCommand(.object([
            "type": .string("model"),
            "command": .object(["type": .string("list_connections")])
        ]))
        return try decodeCollection(value, wrapperKeys: ["connections", "items"])
    }

    func accounts() throws -> [IOSModelAccountSnapshot] {
        let value = try application.executeAgentCommand(.object([
            "type": .string("model"),
            "command": .object(["type": .string("list_accounts")])
        ]))
        return try decodeCollection(value, wrapperKeys: ["accounts", "items"])
    }
    func enabledModels(connectionID: String) throws -> Set<String> {
        Set(try application.listModelProfiles().compactMap { profile in
            guard profile.enabled, profile.deletedAtMs == nil,
                  let source = try JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
                  source["connectionID"] as? String == connectionID else { return nil }
            return source["catalogKey"] as? String
        })
    }
    func credential(connectionID: String, endpoint: String, wireProtocol: String) throws -> (reference: String, secret: String) {
        let snapshot: IOSModelConnectionSnapshot? = try command(["type": .string("get_connection"), "id": .string(connectionID)])
        guard let connection = snapshot?.document.connection, connection.enabled,
              connection.base_url == endpoint, connection.protocols.contains(wireProtocol) else {
            throw IOSApplicationRuntimeError.rejected("Model connection no longer matches its execution route")
        }
        guard let accountID = connection.account_id else { return ("", "") }
        let account: IOSModelAccountSnapshot? = try command(["type": .string("get_account"), "id": .string(accountID)])
        guard let account, account.account.state == "ready", let reference = account.account.credential_ref else {
            throw IOSApplicationRuntimeError.rejected("Model account is unavailable or needs reconnection")
        }
        let secret = try credentials.resolveAccountSecret(reference,
            connectionID: connectionID, endpoint: endpoint, wireProtocol: wireProtocol)
        return (reference, secret)
    }
    func create(name: String, endpoint: String, wireProtocol: String, secret: String, modelID: String, discoveredModel: AppleModelCatalogDiscovery.Model? = nil) throws -> IOSModelConnectionSnapshot {
        let endpoint = endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
        let modelID = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URLComponents(string: endpoint), let scheme = url.scheme, ["https", "http"].contains(scheme),
              let host = url.host, !host.isEmpty, url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else {
            throw IOSModelConnectionInputError.invalidEndpoint
        }
        guard !modelID.isEmpty else { throw IOSModelConnectionInputError.missingModelID }
        let displayName = name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "\(endpoint) · \(wireProtocol)" : name.trimmingCharacters(in: .whitespacesAndNewlines)
        let previous = try connections().first {
            $0.document.connection.source == "manual" &&
            $0.document.connection.display_name.caseInsensitiveCompare(displayName) == .orderedSame &&
            $0.document.connection.base_url == endpoint && $0.document.connection.protocols == [wireProtocol]
        }
        let id = previous?.id ?? UUID().uuidString.lowercased()
        var accountID = previous?.document.connection.account_id
        var account: AgentCoreJSONValue = .null
        let sameCredential: Bool
        if let previous, previous.document.connection.enabled {
            if secret.isEmpty {
                sameCredential = try credential(connectionID: previous.id, endpoint: endpoint, wireProtocol: wireProtocol).secret == secret
            } else {
                sameCredential = (try? credential(connectionID: previous.id, endpoint: endpoint, wireProtocol: wireProtocol).secret) == secret
            }
        } else {
            sameCredential = false
        }
        if !secret.isEmpty && !sameCredential {
            let reference = try credentials.storeAccountSecret(secret, connectionID: id,
                endpoint: endpoint, protocols: [wireProtocol])
            let accountIdentity = UUID().uuidString.lowercased()
            accountID = accountIdentity
            account = .object(["id": .string(accountIdentity), "display_name": .string(displayName), "source": .string("manual"),
                "credential_ref": .string(reference), "state": .string("ready")])
        }
        var entries: [AgentCoreJSONValue] = try (previous?.document.catalog?.models ?? []).map {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode($0))
        }
        if let observed = discoveredModel, observed.id == modelID {
            let entry = try JSONDecoder().decode(AgentCoreJSONValue.self, from: observed.catalogData(wireProtocol: wireProtocol))
            if let index = previous?.document.catalog?.models.firstIndex(where: { $0.model_id == modelID && $0.protocol == wireProtocol }) {
                entries[index] = entry
            } else { entries.append(entry) }
        } else if !(previous?.document.catalog?.models ?? []).contains(where: { $0.model_id == modelID && $0.protocol == wireProtocol }) {
            let manual = AppleModelCatalogDiscovery.Model(id: modelID, displayName: modelID)
            entries.append(try JSONDecoder().decode(AgentCoreJSONValue.self, from: manual.catalogData(wireProtocol: wireProtocol)))
        }
        // Core commits the account, manual model catalog, and enabled profile together.
        // A hand-entered model ID never requires a /models request or a probe.
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let snapshot: IOSModelConnectionSnapshot = try command(["type": .string("import_connection"), "input": .object([
            "id": .string(id), "display_name": .string(displayName), "base_url": .string(endpoint),
            "protocols": .array([.string(wireProtocol)]), "account_id": accountID.map { .string($0) } ?? .null,
            "source": .string("manual"), "source_id": .null, "enabled": .bool(true)]),
            "expected_revision": previous.map { .signedInteger($0.revision) } ?? .null,
            "account": account, "expected_account_revision": .null,
            "models": .array(entries),
            "enable_models": .bool(true), "updated_at_ms": .signedInteger(nowMs)])
        if case .object = account, let accountID {
            let existingAccount: IOSModelAccountSnapshot? = try? command(["type": .string("get_account"), "id": .string(accountID)])
            if existingAccount == nil {
                let _: AgentCoreJSONValue = try command([
                    "type": .string("save_account"),
                    "input": account,
                    "expected_revision": .null,
                    "updated_at_ms": .signedInteger(nowMs)
                ])
            }
        }
        let hasProfile = (try? application.listModelProfiles().contains(where: { profile in
            guard profile.enabled, profile.deletedAtMs == nil,
                  let source = try? JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any] else { return false }
            return source["connectionID"] as? String == snapshot.id
                && profile.modelName == modelID
                && source["wireProtocol"] as? String == wireProtocol
        })) ?? false
        if !hasProfile {
            var sourceObject: [String: Any] = [
                "baseURL": endpoint,
                "wireProtocol": wireProtocol,
                "connectionID": snapshot.id,
                "catalogKey": "\(wireProtocol):\(modelID)",
                "capabilities": ["chat"]
            ]
            if let discovered = discoveredModel, discovered.id == modelID,
               let catalogBytes = try? discovered.catalogData(wireProtocol: wireProtocol),
               let catalogObj = try? JSONSerialization.jsonObject(with: catalogBytes) as? [String: Any],
               let caps = catalogObj["execution_capabilities"], !(caps is NSNull) {
                sourceObject["executionCapabilities"] = caps
            }
            let sourceData = try JSONSerialization.data(withJSONObject: sourceObject, options: [.sortedKeys])
            let sourceJSON = String(decoding: sourceData, as: UTF8.self)
            let defaultParams = (try? JSONEncoder().encode(ModelParameters.defaults(isLocal: false)))
                .map { String(decoding: $0, as: UTF8.self) } ?? "{}"
            let providerName = wireProtocol == "anthropic" ? "anthropic" : "openai"
            let profileDisplayName = (discoveredModel?.id == modelID ? discoveredModel?.displayName : nil) ?? modelID
            let profileID = "conn:\(snapshot.id):\(wireProtocol):\(modelID)"
            try application.upsertModelProfile(IOSCanonicalModelProfile(
                id: profileID,
                displayName: profileDisplayName,
                provider: providerName,
                modelName: modelID,
                runtime: "remote",
                sourceJSON: sourceJSON,
                parametersJSON: defaultParams,
                selected: false,
                enabled: true,
                revision: 1,
                createdAtMs: nowMs,
                updatedAtMs: nowMs,
                deletedAtMs: nil
            ))
        }
        return snapshot
    }

    /// Explicitly configure another API route. Never replace the old route or
    /// reuse a credential at a different endpoint; the new route gets its own
    /// protocol-scoped credential reference through the ordinary create path.
    @MainActor
    func configureProtocol(for model: ModelInfo, wireProtocol: ModelWireProtocol) throws -> ModelInfo {
        guard !model.isLocal, !model.isWebBackend,
              wireProtocol == .responses || wireProtocol == .chatCompletions,
              let stored = try application.modelProfile(id: model.id), stored.enabled, stored.deletedAtMs == nil,
              let current = ModelSelectionManager.modelInfo(from: stored),
              current.baseURL == model.baseURL, current.wireProtocol == model.wireProtocol,
              current.effectiveUpstreamModelId == model.effectiveUpstreamModelId,
              let endpoint = current.baseURL, let originalWire = current.wireProtocol,
              originalWire == .responses || originalWire == .chatCompletions,
              let source = try JSONSerialization.jsonObject(with: Data(stored.sourceJSON.utf8)) as? [String: Any],
              source["sourceKind"] as? String != "nola_chatgpt" else {
            throw IOSCloudCredentialError.invalidReference
        }
        if originalWire == wireProtocol { return current }
        let secret: String
        if let connectionID = source["connectionID"] as? String {
            secret = try credential(connectionID: connectionID, endpoint: endpoint, wireProtocol: originalWire.rawValue).secret
        } else {
            secret = try credentials.resolve(model: current) ?? ""
        }
        let connection = try create(name: "", endpoint: endpoint, wireProtocol: wireProtocol.rawValue,
            secret: secret, modelID: current.effectiveUpstreamModelId)
        guard let profile = try application.listModelProfiles().first(where: { candidate in
            guard candidate.enabled, candidate.deletedAtMs == nil,
                  candidate.modelName == current.effectiveUpstreamModelId,
                  let source = try? JSONSerialization.jsonObject(with: Data(candidate.sourceJSON.utf8)) as? [String: Any] else { return false }
            return source["connectionID"] as? String == connection.id && source["wireProtocol"] as? String == wireProtocol.rawValue
        }), let configured = ModelSelectionManager.modelInfo(from: profile) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return configured
    }
    func disconnect(_ snapshot: IOSModelConnectionSnapshot) throws -> IOSModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = false
        let input = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(document.connection))
        return try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
    }
    func reconnect(_ snapshot: IOSModelConnectionSnapshot) throws -> IOSModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = true
        let input = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(document.connection))
        return try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
    }
    func discover(endpoint: String, wireProtocol: String, secret: String, name: String) async throws -> [AppleModelCatalogDiscovery.Model] {
        var key = secret
        let trimmedEndpoint = endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
        let host = URLComponents(string: trimmedEndpoint)?.host ?? ""
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let defaultDisplayName = "\(trimmedEndpoint) · \(wireProtocol)"
        if key.isEmpty, let connection = try connections().first(where: {
            $0.document.connection.enabled && $0.document.connection.source == "manual" &&
            $0.document.connection.base_url == trimmedEndpoint && $0.document.connection.protocols == [wireProtocol] &&
            (label.isEmpty
                ? ($0.document.connection.display_name.caseInsensitiveCompare(defaultDisplayName) == .orderedSame
                    || (!host.isEmpty && $0.document.connection.display_name.caseInsensitiveCompare(host) == .orderedSame))
                : $0.document.connection.display_name.caseInsensitiveCompare(label) == .orderedSame)
        }) {
            key = try credential(connectionID: connection.id, endpoint: trimmedEndpoint, wireProtocol: wireProtocol).secret
        }
        return try await AppleModelCatalogDiscovery.discover(endpoint: trimmedEndpoint, wireProtocol: wireProtocol, secret: key)
    }

    func discover(_ snapshot: IOSModelConnectionSnapshot) async throws -> IOSModelConnectionSnapshot {
        try Task.checkCancellation()
        let connection = snapshot.document.connection
        guard let wire = connection.protocols.first else {
            throw IOSApplicationRuntimeError.rejected("Invalid model discovery route")
        }
        let credential = try credential(connectionID: connection.id, endpoint: connection.base_url, wireProtocol: wire)
        let models = try await AppleModelCatalogDiscovery.discover(
            endpoint: connection.base_url, wireProtocol: wire, secret: credential.secret)
        let entries: [AgentCoreJSONValue] = try models.map {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: $0.catalogData(wireProtocol: wire))
        }
        try Task.checkCancellation()
        return try command(["type": .string("refresh_connection_catalog"), "connection_id": .string(connection.id),
            "expected_revision": .signedInteger(snapshot.revision), "models": .array(entries), "observed_at_ms": now])
    }
    func enable(_ snapshot: IOSModelConnectionSnapshot, modelIDs: Set<String>) throws {
        let _: AgentCoreJSONValue = try command(["type": .string("configure_connection_models"),
            "connection_id": .string(snapshot.id), "expected_revision": .signedInteger(snapshot.revision),
            "enabled_model_ids": .array(modelIDs.sorted().map { .string($0) }), "selected_model_id": .null,
            "updated_at_ms": now])
    }
}
