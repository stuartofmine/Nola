// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated struct MacModelConnectionSnapshot: Codable, Identifiable, Equatable, Sendable {
    let revision: Int64
    var document: Document
    var id: String { document.connection.id }
    struct Document: Codable, Equatable, Sendable {
        let format_version: Int; var connection: Connection; var catalog: Catalog?
        var capability_assessments: [CapabilityAssessment]? = nil
    }
    struct CapabilityAssessment: Codable, Equatable, Sendable {
        let model_id: String
        let `protocol`: String
        let evidence: Evidence?
        let user_override: MacCanonicalJSONValue?
        struct Evidence: Codable, Equatable, Sendable {
            let text_generation: Bool
            let account_revision: Int64?
            let credential_binding: String
            let observed_at_ms: Int64
        }
    }
    struct Connection: Codable, Equatable, Sendable {
        let id: String
        var display_name: String
        var base_url: String
        var protocols: [String]
        var account_id: String?
        let source: String
        let source_id: String?
        var enabled: Bool
    }
    struct Catalog: Codable, Equatable, Sendable { let observed_at_ms: Int64; let source: String; let models: [Model] }
    struct Model: Codable, Identifiable, Equatable, Sendable {
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
    func connections() throws -> [MacModelConnectionSnapshot] {
        let value = try application.executeAgentRoomEnvelope(.object([
            "type": .string("model"),
            "command": .object(["type": .string("list_connections")]),
        ]))
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        if case .array = value {
            return try decoder.decode([MacModelConnectionSnapshot].self, from: encoder.encode(value))
        }
        if case .object(let envelope) = value,
           case .array(let items) = envelope["connections"] ?? envelope["items"] ?? .array([]) {
            return items.compactMap { item -> MacModelConnectionSnapshot? in
                guard Self.isConnectionRecord(item),
                      let data = try? encoder.encode(item) else { return nil }
                return try? decoder.decode(MacModelConnectionSnapshot.self, from: data)
            }
        }
        return try decoder.decode([MacModelConnectionSnapshot].self, from: encoder.encode(value))
    }

    func accounts() throws -> [MacModelAccountSnapshot] {
        let value = try application.executeAgentRoomEnvelope(.object([
            "type": .string("model"),
            "command": .object(["type": .string("list_accounts")]),
        ]))
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        if case .array = value {
            return try decoder.decode([MacModelAccountSnapshot].self, from: encoder.encode(value))
        }
        if case .object(let envelope) = value,
           case .array(let items) = envelope["accounts"] ?? envelope["items"] ?? .array([]) {
            return items.compactMap { item -> MacModelAccountSnapshot? in
                guard Self.isAccountRecord(item),
                      let data = try? encoder.encode(item) else { return nil }
                return try? decoder.decode(MacModelAccountSnapshot.self, from: data)
            }
        }
        return try decoder.decode([MacModelAccountSnapshot].self, from: encoder.encode(value))
    }

    private static func isConnectionRecord(_ value: MacCanonicalJSONValue) -> Bool {
        guard case .object(let dict) = value else { return false }
        if dict["document"] != nil || dict["connection"] != nil { return true }
        return dict["base_url"] != nil || dict["endpoint"] != nil || dict["protocols"] != nil
    }

    private static func isAccountRecord(_ value: MacCanonicalJSONValue) -> Bool {
        guard case .object(let dict) = value else { return false }
        if dict["account"] != nil { return true }
        if dict["base_url"] != nil || dict["endpoint"] != nil || dict["protocols"] != nil || dict["document"] != nil {
            return false
        }
        return dict["state"] != nil || dict["credential_ref"] != nil || dict["account_id"] != nil
    }
    func enabledModels(connectionID: String) throws -> Set<String> {
        try enabledModelsByConnection()[connectionID] ?? []
    }

    func enabledModelsByConnection() throws -> [String: Set<String>] {
        var result: [String: Set<String>] = [:]
        for profile in try application.listModelProfiles() {
            guard profile.enabled, profile.deletedAtMs == nil,
                  let source = try JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
                  let connectionID = source["connectionID"] as? String,
                  let catalogKey = source["catalogKey"] as? String else { continue }
            result[connectionID, default: []].insert(catalogKey)
        }
        return result
    }

    struct InventorySnapshot: Sendable {
        let connections: [MacModelConnectionSnapshot]
        let accounts: [MacModelAccountSnapshot]
        let enabledByConnection: [String: Set<String>]
    }

    func loadInventory() throws -> InventorySnapshot {
        let currentConnections = try connections()
        let currentAccounts = try accounts()
        let enabledMap = try enabledModelsByConnection()
        let normalizedEnabled = Dictionary(uniqueKeysWithValues: currentConnections.map {
            ($0.id, enabledMap[$0.id] ?? [])
        })
        return InventorySnapshot(
            connections: currentConnections,
            accounts: currentAccounts,
            enabledByConnection: normalizedEnabled
        )
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
        let created: MacModelConnectionSnapshot = try command(["type": .string("save_connection"), "input": .object([
            "id": .string(id),
                "display_name": .string(name), "base_url": .string(endpoint), "protocols": .array([.string(wireProtocol)]),
                "account_id": accountID.map { .string($0) } ?? .null, "source": .string("manual"),
                "source_id": .null, "enabled": .bool(true)]),
            "expected_revision": .null, "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return created
    }

    func update(
        _ snapshot: MacModelConnectionSnapshot,
        name: String,
        endpoint: String,
        wireProtocol: String,
        secret: String? = nil,
        clearSecret: Bool = false
    ) throws -> MacModelConnectionSnapshot {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedEndpoint = endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URLComponents(string: trimmedEndpoint),
              let scheme = url.scheme?.lowercased(), ["https", "http"].contains(scheme),
              url.host != nil, url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              !trimmedName.isEmpty else {
            throw MacApplicationPortError.rejected("Enter a name and an HTTP(S) endpoint without credentials or query")
        }
        let old = snapshot.document.connection
        let routeChanged = (trimmedEndpoint != old.base_url) || (old.protocols != [wireProtocol])
        let credentialStore = MacAppleChatConfigurationStore(applicationPort: application)
        var nextAccountID = old.account_id
        var oldRefToDelete: String?

        let existingAccount: MacModelAccountSnapshot?
        if let accountID = old.account_id {
            existingAccount = try command(["type": .string("get_account"), "id": .string(accountID)])
        } else {
            existingAccount = nil
        }

        if clearSecret {
            if let oldRef = existingAccount?.account.credential_ref, !oldRef.isEmpty {
                oldRefToDelete = oldRef
            }
            if let existingAccount {
                let _: MacModelAccountSnapshot = try command([
                    "type": .string("save_account"),
                    "input": .object([
                        "id": .string(existingAccount.account.id),
                        "display_name": .string(trimmedName),
                        "source": .string(existingAccount.account.source),
                        "credential_ref": .null,
                        "state": .string("disconnected"),
                    ]),
                    "expected_revision": .signedInteger(existingAccount.revision),
                    "updated_at_ms": now,
                ])
            }
            nextAccountID = nil
        } else if let newSecret = secret, !newSecret.isEmpty {
            let newReference = try credentialStore.storeAccountSecret(
                newSecret,
                connectionID: old.id,
                endpoint: trimmedEndpoint,
                protocols: [wireProtocol]
            )
            if let existingAccount {
                if let oldRef = existingAccount.account.credential_ref, !oldRef.isEmpty, oldRef != newReference {
                    oldRefToDelete = oldRef
                }
                let _: MacModelAccountSnapshot = try command([
                    "type": .string("save_account"),
                    "input": .object([
                        "id": .string(existingAccount.account.id),
                        "display_name": .string(trimmedName),
                        "source": .string(existingAccount.account.source),
                        "credential_ref": .string(newReference),
                        "state": .string("ready"),
                    ]),
                    "expected_revision": .signedInteger(existingAccount.revision),
                    "updated_at_ms": now,
                ])
                nextAccountID = existingAccount.account.id
            } else {
                let accountID = UUID().uuidString.lowercased()
                let _: MacModelAccountSnapshot = try command([
                    "type": .string("save_account"),
                    "input": .object([
                        "id": .string(accountID),
                        "display_name": .string(trimmedName),
                        "source": .string(old.source),
                        "credential_ref": .string(newReference),
                        "state": .string("ready"),
                    ]),
                    "expected_revision": .null,
                    "updated_at_ms": now,
                ])
                nextAccountID = accountID
            }
        } else if routeChanged, let existingAccount,
                  let oldRef = existingAccount.account.credential_ref, !oldRef.isEmpty,
                  let oldWire = old.protocols.first {
            // Re-bind the existing Keychain secret to the updated endpoint/protocol so discovery and chat continue working.
            let existingSecret = try credentialStore.resolveAccountSecret(
                oldRef,
                connectionID: old.id,
                endpoint: old.base_url,
                wireProtocol: oldWire
            )
            let reboundRef = try credentialStore.storeAccountSecret(
                existingSecret,
                connectionID: old.id,
                endpoint: trimmedEndpoint,
                protocols: [wireProtocol]
            )
            oldRefToDelete = oldRef
            let _: MacModelAccountSnapshot = try command([
                "type": .string("save_account"),
                "input": .object([
                    "id": .string(existingAccount.account.id),
                    "display_name": .string(trimmedName),
                    "source": .string(existingAccount.account.source),
                    "credential_ref": .string(reboundRef),
                    "state": .string("ready"),
                ]),
                "expected_revision": .signedInteger(existingAccount.revision),
                "updated_at_ms": now,
            ])
        } else if let existingAccount, trimmedName != existingAccount.account.display_name {
            let _: MacModelAccountSnapshot = try command([
                "type": .string("save_account"),
                "input": .object([
                    "id": .string(existingAccount.account.id),
                    "display_name": .string(trimmedName),
                    "source": .string(existingAccount.account.source),
                    "credential_ref": existingAccount.account.credential_ref.map { .string($0) } ?? .null,
                    "state": .string(existingAccount.account.state),
                ]),
                "expected_revision": .signedInteger(existingAccount.revision),
                "updated_at_ms": now,
            ])
        }

        var updatedConnection = old
        updatedConnection.display_name = trimmedName
        updatedConnection.base_url = trimmedEndpoint
        updatedConnection.protocols = [wireProtocol]
        updatedConnection.account_id = nextAccountID
        let input = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(updatedConnection))
        let updated: MacModelConnectionSnapshot = try command([
            "type": .string("save_connection"),
            "input": input,
            "clear_catalog": .bool(routeChanged),
            "expected_revision": .signedInteger(snapshot.revision),
            "updated_at_ms": now,
        ])
        if let oldRefToDelete {
            credentialStore.deleteAccountSecret(oldRefToDelete)
        }
        if routeChanged {
            try enable(updated, modelIDs: [])
        }
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return updated
    }

    func delete(_ snapshot: MacModelConnectionSnapshot, deleteAccount: Bool = true) throws {
        struct DeleteResult: Decodable {
            let ok: Bool?
            let deleted: Bool?
            let deleted_account_id: String?
        }
        let credentialStore = MacAppleChatConfigurationStore(applicationPort: application)
        var credentialRefToDelete: String?
        if deleteAccount, let accountID = snapshot.document.connection.account_id {
            if let acct: MacModelAccountSnapshot = try? command(["type": .string("get_account"), "id": .string(accountID)]),
               let ref = acct.account.credential_ref, !ref.isEmpty {
                credentialRefToDelete = ref
            }
        }
        let result: DeleteResult = try command([
            "type": .string("delete_connection"),
            "id": .string(snapshot.id),
            "delete_account": .bool(deleteAccount),
            "deleted_at_ms": now,
        ])
        if result.deleted_account_id != nil, let ref = credentialRefToDelete {
            credentialStore.deleteAccountSecret(ref)
        }
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    func disconnect(_ snapshot: MacModelConnectionSnapshot) throws -> MacModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = false
        let input = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(document.connection))
        let updated: MacModelConnectionSnapshot = try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return updated
    }
    func reconnect(_ snapshot: MacModelConnectionSnapshot) throws -> MacModelConnectionSnapshot {
        var document = snapshot.document; document.connection.enabled = true
        let input = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(document.connection))
        let updated: MacModelConnectionSnapshot = try command(["type": .string("save_connection"), "input": input,
            "expected_revision": .signedInteger(snapshot.revision), "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return updated
    }
    func discover(_ snapshot: MacModelConnectionSnapshot) async throws -> MacModelConnectionSnapshot {
        try Task.checkCancellation()
        let connection = snapshot.document.connection
        guard let wire = connection.protocols.first else {
            throw MacApplicationPortError.rejected("Invalid model discovery route")
        }
        do {
            let credential = try await Task.detached(priority: .userInitiated) { [self] in
                try self.credential(connectionID: connection.id, endpoint: connection.base_url, wireProtocol: wire)
            }.value
            let models = try await AppleModelCatalogDiscovery.discover(
                endpoint: connection.base_url, wireProtocol: wire, secret: credential.secret)
            let entries: [MacCanonicalJSONValue] = try models.map {
                try JSONDecoder().decode(MacCanonicalJSONValue.self, from: $0.catalogData(wireProtocol: wire))
            }
            try Task.checkCancellation()
            let updated: MacModelConnectionSnapshot = try await Task.detached(priority: .userInitiated) { [self] in
                try self.command(["type": .string("refresh_connection_catalog"), "connection_id": .string(connection.id),
                    "expected_revision": .signedInteger(snapshot.revision), "models": .array(entries), "observed_at_ms": self.now])
            }.value
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            return updated
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            let nsErr = error as NSError
            let host = URL(string: connection.base_url)?.host ?? "unknown"
            await Logger.shared.log(
                .error,
                "MacModelConnectionService",
                "Model catalog discovery failed",
                metadata: [
                    "connectionID": connection.id,
                    "host": host,
                    "protocol": wire,
                    "errorDomain": nsErr.domain,
                    "errorCode": String(nsErr.code),
                    "error": error.localizedDescription,
                ]
            )
            throw error
        }
    }
    func enable(_ snapshot: MacModelConnectionSnapshot, modelIDs: Set<String>) throws {
        let _: MacCanonicalJSONValue = try command(["type": .string("configure_connection_models"),
            "connection_id": .string(snapshot.id), "expected_revision": .signedInteger(snapshot.revision),
            "enabled_model_ids": .array(modelIDs.sorted().map { .string($0) }), "selected_model_id": .null,
            "updated_at_ms": now])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    func clearAllConnectionsAndAccounts() async throws {
        let credentialStore = MacAppleChatConfigurationStore(applicationPort: application)
        let currentConnections = try connections()
        for connection in currentConnections {
            try enable(connection, modelIDs: [])
            let latest: MacModelConnectionSnapshot? = try command([
                "type": .string("get_connection"),
                "id": .string(connection.id),
            ])
            if let latest, latest.document.connection.enabled {
                _ = try disconnect(latest)
            }
        }
        let currentAccounts = try accounts()
        for account in currentAccounts {
            if account.account.source == "nola_chatgpt" {
                if let bridge = try? MacSiwcAccountBridge(application: application) {
                    defer { bridge.close() }
                    if let status = try? await bridge.manage("account_status", account: account, sessionRevision: nil),
                       let session = status.session, session.signed_in {
                        _ = try? await bridge.manage(
                            "sign_out",
                            account: status.account,
                            sessionRevision: session.session_revision,
                            acknowledgeSignOut: true
                        )
                    }
                }
                let latest: MacModelAccountSnapshot? = try command([
                    "type": .string("get_account"),
                    "id": .string(account.id),
                ])
                if let latest,
                   latest.account.state != "disconnected" || latest.account.credential_ref != nil {
                    let _: MacModelAccountSnapshot = try command([
                        "type": .string("save_account"),
                        "input": .object([
                            "id": .string(latest.account.id),
                            "display_name": .string(latest.account.display_name),
                            "source": .string(latest.account.source),
                            "credential_ref": .null,
                            "state": .string("disconnected"),
                        ]),
                        "expected_revision": .signedInteger(latest.revision),
                        "updated_at_ms": now,
                    ])
                }
            } else {
                if let reference = account.account.credential_ref, !reference.isEmpty {
                    credentialStore.deleteAccountSecret(reference)
                }
                if account.account.state != "disconnected" || account.account.credential_ref != nil {
                    let _: MacModelAccountSnapshot = try command([
                        "type": .string("save_account"),
                        "input": .object([
                            "id": .string(account.account.id),
                            "display_name": .string(account.account.display_name),
                            "source": .string(account.account.source),
                            "credential_ref": .null,
                            "state": .string("disconnected"),
                        ]),
                        "expected_revision": .signedInteger(account.revision),
                        "updated_at_ms": now,
                    ])
                }
            }
        }
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }
}

nonisolated extension MacModelConnectionSnapshot {
    private enum EnvelopeKeys: String, CodingKey {
        case revision
        case document
        case connection
        case catalog
        case capability_assessments
        case format_version
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: EnvelopeKeys.self)
        let revision = try container.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
        if let document = try container.decodeIfPresent(Document.self, forKey: .document) {
            self.revision = revision
            self.document = document
            return
        }
        if let connection = try container.decodeIfPresent(Connection.self, forKey: .connection) {
            let formatVersion = try container.decodeIfPresent(Int.self, forKey: .format_version) ?? 1
            let catalog = try container.decodeIfPresent(Catalog.self, forKey: .catalog)
            let assessments = try container.decodeIfPresent([CapabilityAssessment].self, forKey: .capability_assessments)
            self.revision = revision
            self.document = Document(
                format_version: formatVersion,
                connection: connection,
                catalog: catalog,
                capability_assessments: assessments
            )
            return
        }
        let connection = try Connection(from: decoder)
        self.revision = revision
        self.document = Document(
            format_version: 1,
            connection: connection,
            catalog: nil,
            capability_assessments: nil
        )
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: EnvelopeKeys.self)
        try container.encode(revision, forKey: .revision)
        try container.encode(document, forKey: .document)
    }
}

nonisolated extension MacModelConnectionSnapshot.Connection {
    private enum Keys: String, CodingKey {
        case id
        case display_name
        case base_url
        case endpoint
        case protocols
        case `protocol`
        case account_id
        case source
        case source_id
        case enabled
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Keys.self)
        let id = try container.decode(String.self, forKey: .id)
        guard let baseURL = try container.decodeIfPresent(String.self, forKey: .base_url)
            ?? container.decodeIfPresent(String.self, forKey: .endpoint) else {
            throw DecodingError.keyNotFound(
                Keys.base_url,
                DecodingError.Context(
                    codingPath: decoder.codingPath,
                    debugDescription: "Missing base_url or endpoint on model connection record."
                )
            )
        }
        let protocols: [String]
        if let explicit = try container.decodeIfPresent([String].self, forKey: .protocols) {
            protocols = explicit
        } else if let single = try container.decodeIfPresent(String.self, forKey: .protocol), !single.isEmpty {
            protocols = [single]
        } else {
            protocols = ["chat_completions"]
        }
        self.id = id
        self.display_name = try container.decodeIfPresent(String.self, forKey: .display_name) ?? id
        self.base_url = baseURL
        self.protocols = protocols
        self.account_id = try container.decodeIfPresent(String.self, forKey: .account_id)
        self.source = try container.decodeIfPresent(String.self, forKey: .source) ?? "manual"
        self.source_id = try container.decodeIfPresent(String.self, forKey: .source_id)
        self.enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Keys.self)
        try container.encode(id, forKey: .id)
        try container.encode(display_name, forKey: .display_name)
        try container.encode(base_url, forKey: .base_url)
        try container.encode(protocols, forKey: .protocols)
        try container.encode(account_id, forKey: .account_id)
        try container.encode(source, forKey: .source)
        try container.encode(source_id, forKey: .source_id)
        try container.encode(enabled, forKey: .enabled)
    }
}

nonisolated extension MacModelAccountSnapshot {
    private enum EnvelopeKeys: String, CodingKey {
        case revision
        case account
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: EnvelopeKeys.self)
        let revision = try container.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
        if let account = try container.decodeIfPresent(Account.self, forKey: .account) {
            self.revision = revision
            self.account = account
            return
        }
        self.revision = revision
        self.account = try Account(from: decoder)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: EnvelopeKeys.self)
        try container.encode(revision, forKey: .revision)
        try container.encode(account, forKey: .account)
    }
}
