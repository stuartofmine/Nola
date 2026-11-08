// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated struct MacModelConnectionSnapshot: Codable, Identifiable, Sendable {
    let revision: Int64
    var document: Document
    var id: String { document.connection.id }
    struct Document: Codable, Sendable {
        let format_version: Int; var connection: Connection; var catalog: Catalog?
        var capability_assessments: [CapabilityAssessment]? = nil
    }
    struct CapabilityAssessment: Codable, Sendable {
        let model_id: String
        let `protocol`: String
        let evidence: Evidence?
        let user_override: MacCanonicalJSONValue?
        struct Evidence: Codable, Sendable {
            let text_generation: Bool
            let account_revision: Int64?
            let credential_binding: String
            let observed_at_ms: Int64
        }
    }
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
        let execution_capabilities: MacCanonicalJSONValue?
        var id: String { `protocol` + ":" + model_id }
    }
}
nonisolated struct MacModelAccountSnapshot: Codable, Identifiable, Sendable {
    let revision: Int64
    let account: Account
    var id: String { account.id }
    struct Account: Codable, Sendable {
        let id: String; let display_name: String; let source: String
        let credential_ref: String?; let state: String
    }
}

nonisolated final class MacModelConnectionService: Sendable {
    let application: MacApplicationPort
    init(application: MacApplicationPort) { self.application = application }
    func makeImportBridge() throws -> MacModelImportBridge { try MacModelImportBridge(application: application) }
    func setCapabilityOverride(_ snapshot: MacModelConnectionSnapshot, model: MacModelConnectionSnapshot.Model,
        capabilities: MacCanonicalJSONValue?) throws -> MacModelConnectionSnapshot {
        let updated: MacModelConnectionSnapshot = try command(["type": .string("set_connection_capability_override"), "connection_id": .string(snapshot.id),
            "expected_revision": .signedInteger(snapshot.revision), "model_id": .string(model.model_id),
            "protocol": .string(model.protocol), "capabilities": capabilities ?? .null, "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return updated
    }
    private var now: MacCanonicalJSONValue { .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)) }
    func command<T: Decodable>(_ command: [String: MacCanonicalJSONValue]) throws -> T {
        let value = try application.executeAgentRoomEnvelope(.object(["type": .string("model"), "command": .object(command)]))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    func connections() throws -> [MacModelConnectionSnapshot] { try command(["type": .string("list_connections")]) }
    func accounts() throws -> [MacModelAccountSnapshot] { try command(["type": .string("list_accounts")]) }
    func enabledModels(connectionID: String) throws -> Set<String> {
        Set(try application.listModelProfiles().compactMap { profile in
            guard profile.enabled, profile.deletedAtMs == nil,
                  let source = try JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
                  source["connectionID"] as? String == connectionID else { return nil }
            return source["catalogKey"] as? String
        })
    }
    func credential(connectionID: String, endpoint: String, wireProtocol: String) throws -> (reference: String, secret: String) {
        guard wireProtocol != "siwc_responses" else {
            throw MacApplicationPortError.rejected("ChatGPT authorization is resolved only by the local Host, never as an API key")
        }
        let snapshot: MacModelConnectionSnapshot? = try command(["type": .string("get_connection"), "id": .string(connectionID)])
        guard let connection = snapshot?.document.connection, connection.enabled,
              connection.base_url == endpoint, connection.protocols.contains(wireProtocol) else {
            throw MacApplicationPortError.rejected("Model connection no longer matches its execution route")
        }
        guard let accountID = connection.account_id else { return ("", "") }
        let account: MacModelAccountSnapshot? = try command(["type": .string("get_account"), "id": .string(accountID)])
        guard let account, account.account.state == "ready", let reference = account.account.credential_ref else {
            throw MacApplicationPortError.rejected("Model account is unavailable or needs reconnection")
        }
        let secret = try MacAppleChatConfigurationStore(applicationPort: application).resolveAccountSecret(reference,
            connectionID: connectionID, endpoint: endpoint, wireProtocol: wireProtocol)
        return (reference, secret)
    }
    func create(name: String, endpoint: String, wireProtocol: String, secret: String) throws -> MacModelConnectionSnapshot {
        guard let url = URLComponents(string: endpoint), let scheme = url.scheme, ["https", "http"].contains(scheme),
              url.host != nil, url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw MacApplicationPortError.rejected("Enter a name and an HTTP(S) endpoint without credentials or query")
        }
        let id = UUID().uuidString.lowercased()
        var accountID: String?
        if !secret.isEmpty {
            let reference = try MacAppleChatConfigurationStore(applicationPort: application).storeAccountSecret(secret, connectionID: id,
                endpoint: endpoint, protocols: [wireProtocol])
            let account = UUID().uuidString.lowercased()
            let _: MacModelAccountSnapshot = try command(["type": .string("save_account"),
                "input": .object(["id": .string(account), "display_name": .string(name), "source": .string("manual"),
                    "credential_ref": .string(reference), "state": .string("ready")]),
                "expected_revision": .null, "updated_at_ms": now])
            accountID = account
        }
        return try command(["type": .string("save_connection"), "input": .object([
            "id": .string(id),
                "display_name": .string(name), "base_url": .string(endpoint), "protocols": .array([.string(wireProtocol)]),
                "account_id": accountID.map { .string($0) } ?? .null, "source": .string("manual"),
                "source_id": .null, "enabled": .bool(true)]),
            "expected_revision": .null, "updated_at_ms": now])
    }
    func disconnect(_ snapshot: MacModelConnectionSnapshot) throws -> MacModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = false
        let input = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(document.connection))
        return try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
    }
    func discover(_ snapshot: MacModelConnectionSnapshot) async throws -> MacModelConnectionSnapshot {
        try Task.checkCancellation()
        let connection = snapshot.document.connection
        guard let wire = connection.protocols.first else {
            throw MacApplicationPortError.rejected("Invalid model discovery route")
        }
        let credential = try credential(connectionID: connection.id, endpoint: connection.base_url, wireProtocol: wire)
        let models = try await AppleModelCatalogDiscovery.discover(
            endpoint: connection.base_url, wireProtocol: wire, secret: credential.secret)
        let entries: [MacCanonicalJSONValue] = models.map {
            .object(["model_id": .string($0.id), "display_name": .string($0.displayName),
                "protocol": .string(wire), "context_window": .null, "output_limit": .null,
                "modalities": .array([]), "reasoning_levels": .array([]), "execution_capabilities": .null])
        }
        try Task.checkCancellation()
        return try command(["type": .string("refresh_connection_catalog"), "connection_id": .string(connection.id),
            "expected_revision": .signedInteger(snapshot.revision), "models": .array(entries), "observed_at_ms": now])
    }
    func enable(_ snapshot: MacModelConnectionSnapshot, modelIDs: Set<String>) throws {
        let _: MacCanonicalJSONValue = try command(["type": .string("configure_connection_models"),
            "connection_id": .string(snapshot.id), "expected_revision": .signedInteger(snapshot.revision),
            "enabled_model_ids": .array(modelIDs.sorted().map { .string($0) }), "selected_model_id": .null,
            "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }
}
