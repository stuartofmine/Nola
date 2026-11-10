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
}
nonisolated struct IOSModelAccountSnapshot: Codable, Identifiable, Sendable {
    let revision: Int64
    let account: Account
    var id: String { account.id }
    struct Account: Codable, Sendable {
        let id: String; let display_name: String; let source: String
        let credential_ref: String?; let state: String
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
    func connections() throws -> [IOSModelConnectionSnapshot] { try command(["type": .string("list_connections")]) }
    func accounts() throws -> [IOSModelAccountSnapshot] { try command(["type": .string("list_accounts")]) }
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
    func create(name: String, endpoint: String, wireProtocol: String, secret: String, modelID: String) throws -> IOSModelConnectionSnapshot {
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
        let sameCredential = try previous.map {
            try credential(connectionID: $0.id, endpoint: endpoint, wireProtocol: wireProtocol).secret == secret
        } ?? false
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
        if !(previous?.document.catalog?.models ?? []).contains(where: { $0.model_id == modelID && $0.protocol == wireProtocol }) {
            entries.append(.object(["model_id": .string(modelID), "display_name": .string(modelID),
                "protocol": .string(wireProtocol), "context_window": .null, "output_limit": .null,
                "modalities": .array([]), "reasoning_levels": .array([]), "execution_capabilities": .null]))
        }
        // Core commits the account, manual model catalog, and enabled profile together.
        // A hand-entered model ID never requires a /models request or a probe.
        return try command(["type": .string("import_connection"), "input": .object([
            "id": .string(id), "display_name": .string(displayName), "base_url": .string(endpoint),
            "protocols": .array([.string(wireProtocol)]), "account_id": accountID.map { .string($0) } ?? .null,
            "source": .string("manual"), "source_id": .null, "enabled": .bool(true)]),
            "expected_revision": previous.map { .signedInteger($0.revision) } ?? .null,
            "account": account, "expected_account_revision": .null,
            "models": .array(entries),
            "enable_models": .bool(true), "updated_at_ms": now])
    }
    func disconnect(_ snapshot: IOSModelConnectionSnapshot) throws -> IOSModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = false
        let input = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(document.connection))
        return try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
    }
    func discover(endpoint: String, wireProtocol: String, secret: String, name: String) async throws -> [AppleModelCatalogDiscovery.Model] {
        var key = secret
        let host = URLComponents(string: endpoint)?.host ?? ""
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if key.isEmpty, let connection = try connections().first(where: {
            $0.document.connection.enabled && $0.document.connection.source == "manual" &&
            $0.document.connection.display_name.caseInsensitiveCompare(label.isEmpty ? host : label) == .orderedSame &&
            $0.document.connection.base_url == endpoint && $0.document.connection.protocols == [wireProtocol]
        }) {
            key = try credential(connectionID: connection.id, endpoint: endpoint, wireProtocol: wireProtocol).secret
        }
        return try await AppleModelCatalogDiscovery.discover(endpoint: endpoint, wireProtocol: wireProtocol, secret: key)
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
        let entries: [AgentCoreJSONValue] = models.map {
            .object(["model_id": .string($0.id), "display_name": .string($0.displayName),
                "protocol": .string(wire), "context_window": .null, "output_limit": .null,
                "modalities": .array([]), "reasoning_levels": .array([]), "execution_capabilities": .null])
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
