import XCTest
@testable import Nola

@MainActor
final class IOSCloudCredentialOwnerTests: XCTestCase {
    func testConnectionAccountPublishesOnlyOpaqueSecretAndDisablesExecutionOnDisconnect() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("account.db"))
        let vault = Vault()
        let service = IOSModelConnectionService(application: runtime, credentials: vault.owner)
        let snapshot = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1",
            wireProtocol: "responses", secret: "private-account-key", modelID: "fixture-model")
        let accounts = try service.accounts()
        XCTAssertEqual(accounts.count, 1)
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(accounts), as: UTF8.self).contains("private-account-key"))
        XCTAssertEqual(try service.credential(connectionID: snapshot.id, endpoint: "https://fixture.invalid/v1",
            wireProtocol: "responses").secret, "private-account-key")
        _ = try service.disconnect(snapshot)
        XCTAssertThrowsError(try service.credential(connectionID: snapshot.id, endpoint: "https://fixture.invalid/v1",
            wireProtocol: "responses"))
        XCTAssertEqual(vault.values.count, 1, "Disconnect must not erase credentials or user data")
    }
    func testAccountSecretIsBoundToConnectionEndpointAndProtocol() throws {
        let vault = Vault()
        let owner = vault.owner
        let reference = try owner.storeAccountSecret("account-secret", connectionID: "connection",
            endpoint: "https://example.test/v1", protocols: ["responses"])
        XCTAssertFalse(reference.contains("account-secret"))
        XCTAssertEqual(try owner.resolveAccountSecret(reference, connectionID: "connection",
            endpoint: "https://example.test/v1", wireProtocol: "responses"), "account-secret")
        XCTAssertThrowsError(try owner.resolveAccountSecret(reference, connectionID: "other",
            endpoint: "https://example.test/v1", wireProtocol: "responses"))
        XCTAssertThrowsError(try owner.resolveAccountSecret(reference, connectionID: "connection",
            endpoint: "https://evil.test/v1", wireProtocol: "responses"))
        XCTAssertThrowsError(try owner.resolveAccountSecret(reference, connectionID: "connection",
            endpoint: "https://example.test/v1", wireProtocol: "chat_completions"))
    }
    func testCASConflictRemovesOnlyNewVersionAndPreservesPublishedCredential() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("cloud.db"))
        let vault = Vault()
        let owner = vault.owner
        let original = model(secret: "original")
        _ = try ModelSelectionManager.publishCloudModel(original, existing: nil, selected: true, using: store, credentials: owner)
        let first = try XCTUnwrap(store.modelProfile(id: original.id))
        let resolved = try ModelSelectionManager.resolvedCloudProfileForRun(original, using: store, credentials: owner)
        XCTAssertEqual(resolved.model.apiKey, "original")
        XCTAssertEqual(resolved.revision, first.revision)
        XCTAssertFalse(first.sourceJSON.contains("original"))

        _ = try ModelSelectionManager.publishCloudModel(model(secret: "replacement"), existing: first,
            selected: true, using: store, credentials: owner)
        let committed = try XCTUnwrap(store.modelProfile(id: original.id))
        XCTAssertThrowsError(try ModelSelectionManager.publishCloudModel(model(secret: "stale"),
            existing: first, selected: false, using: store, credentials: owner))
        XCTAssertEqual(try store.modelProfile(id: original.id), committed)
        XCTAssertEqual(vault.values.count, 2)
        XCTAssertEqual(vault.deleted.count, 1)
        XCTAssertEqual(try owner.resolve(model: resolved.model), "original")
        XCTAssertEqual(try ModelSelectionManager.resolvedCloudProfileForRun(original, using: store,
            credentials: owner).model.apiKey, "replacement")
        vault.values.removeAll()
        XCTAssertThrowsError(try ModelSelectionManager.resolvedCloudProfileForRun(original,
            using: store, credentials: owner), "An ephemeral selected.apiKey must not replace a missing saved reference")
    }

    func testStorageFailureDoesNotPublishAndUncertainPublicationDoesNotDelete() throws {
        let vault = Vault()
        vault.failSave = true
        var committed = false
        XCTAssertThrowsError(try vault.owner.publish(model: model(secret: nil), secret: "secret") { _ in
            committed = true
            return .committed(revision: 1)
        })
        XCTAssertFalse(committed)
        vault.failSave = false
        XCTAssertThrowsError(try vault.owner.publish(model: model(secret: nil), secret: "uncertain") { _ in
            .unconfirmed
        })
        XCTAssertEqual(vault.values.count, 1)
        XCTAssertTrue(vault.deleted.isEmpty)
    }

    func testBoundReferenceCannotBeReusedForDifferentProfileRouteProtocolOrModel() throws {
        let vault = Vault()
        let original = model(secret: nil)
        let (binding, _) = try vault.owner.publish(model: original, secret: "bound") { _ in .committed(revision: 1) }
        XCTAssertEqual(try vault.owner.resolve(model: original.withCloudCredential(binding, apiKey: nil)), "bound")
        for altered in [
            model(id: "another-profile", secret: nil),
            model(endpoint: "https://attacker.invalid/v1", secret: nil),
            model(upstream: "another-model", secret: nil),
            model(protocol: .responses, secret: nil),
        ] {
            XCTAssertThrowsError(try vault.owner.resolve(model: altered.withCloudCredential(binding, apiKey: nil)))
        }
        XCTAssertThrowsError(try vault.owner.resolve(model: original.withCloudCredential(.keychain(account: "profile"), apiKey: "must-not-fallback")))
        vault.values.removeAll()
        XCTAssertThrowsError(try vault.owner.resolve(model: original.withCloudCredential(binding, apiKey: "must-not-fallback")))
    }

    func testAnonymousIsExplicitAndRedactionDoesNotPersistCredential() throws {
        let vault = Vault()
        let original = model(secret: "ephemeral")
        let redacted = original.redactedForPersistence
        XCTAssertNil(redacted.apiKey)
        XCTAssertNil(redacted.cloudCredential)
        XCTAssertThrowsError(try vault.owner.resolve(model: redacted))
        let (binding, revision) = try vault.owner.publish(model: original, secret: "") { value in
            XCTAssertEqual(value, .anonymous)
            return .committed(revision: 7)
        }
        XCTAssertEqual(revision, 7)
        XCTAssertNil(try vault.owner.resolve(model: original.withCloudCredential(binding, apiKey: nil)))
        XCTAssertTrue(vault.values.isEmpty)
    }

    func testCredentialChangeCannotEnableDisabledOrReviveDeletedProfile() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("cloud.db"))
        let vault = Vault()
        let original = model(secret: "first")
        _ = try ModelSelectionManager.publishCloudModel(original, existing: nil, selected: false,
            using: store, credentials: vault.owner)
        let initial = try XCTUnwrap(store.modelProfile(id: original.id))
        try store.upsertModelProfile(IOSCanonicalModelProfile(id: initial.id, displayName: initial.displayName,
            provider: initial.provider, modelName: initial.modelName, runtime: initial.runtime,
            sourceJSON: initial.sourceJSON, parametersJSON: initial.parametersJSON,
            selected: false, enabled: false, revision: initial.revision,
            createdAtMs: initial.createdAtMs, updatedAtMs: initial.updatedAtMs + 1, deletedAtMs: nil))
        let disabled = try XCTUnwrap(store.modelProfile(id: original.id))
        _ = try ModelSelectionManager.publishCloudModel(model(secret: "second"), existing: disabled,
            selected: true, using: store, credentials: vault.owner)
        let beforeDelete = try XCTUnwrap(store.modelProfile(id: original.id))
        XCTAssertFalse(beforeDelete.enabled)
        XCTAssertThrowsError(try ModelSelectionManager.resolvedCloudProfileForRun(original,
            using: store, credentials: vault.owner))
        _ = try store.deleteModelProfile(id: original.id)
        XCTAssertNil(try store.modelProfile(id: original.id), "Public queries hide deleted profiles")
        let accounts = vault.values
        let deletedCount = vault.deleted.count
        // This is the real configuration entrypoint's read result after deletion.
        // expectedRevision=0 must still conflict with the durable tombstone.
        XCTAssertThrowsError(try ModelSelectionManager.publishCloudModel(model(secret: "recreate-same-id"),
            existing: try store.modelProfile(id: original.id), selected: true, using: store, credentials: vault.owner))
        XCTAssertEqual(vault.values, accounts)
        XCTAssertEqual(vault.deleted.count, deletedCount + 1, "Only the rejected new version is removed")
        XCTAssertNil(try store.modelProfile(id: original.id))

        // Separately exercise the explicit tombstone value guard. This value
        // is constructed, not evidence that public reads expose tombstones.
        let tombstone = IOSCanonicalModelProfile(id: beforeDelete.id, displayName: beforeDelete.displayName,
            provider: beforeDelete.provider, modelName: beforeDelete.modelName, runtime: beforeDelete.runtime,
            sourceJSON: beforeDelete.sourceJSON, parametersJSON: beforeDelete.parametersJSON,
            selected: false, enabled: false, revision: beforeDelete.revision,
            createdAtMs: beforeDelete.createdAtMs, updatedAtMs: beforeDelete.updatedAtMs,
            deletedAtMs: beforeDelete.updatedAtMs)
        XCTAssertThrowsError(try ModelSelectionManager.publishCloudModel(model(secret: "must-not-stage"),
            existing: tombstone, selected: true, using: store, credentials: vault.owner))
        XCTAssertEqual(vault.values, accounts)
        XCTAssertEqual(vault.deleted.count, deletedCount + 1)
        XCTAssertNil(try store.modelProfile(id: original.id))
    }

    private func model(id: String = "profile", endpoint: String = "https://provider.invalid/v1",
        upstream: String = "model", protocol wireProtocol: ModelWireProtocol = .chatCompletions,
        secret: String?) -> ModelInfo {
        ModelInfo(id: id, name: "Fixture", source: "Fixture", isLocal: false,
            baseURL: endpoint, apiKey: secret, upstreamModelId: upstream, wireProtocol: wireProtocol)
    }
}

nonisolated private final class Vault: @unchecked Sendable {
    var values: [String: Data] = [:]
    var deleted: [String] = []
    var failSave = false
    // Tests use this fake synchronously on MainActor; no production Keychain access.
    var owner: IOSCloudCredentialOwner {
        .init(save: { [self] account, value in
            guard !failSave, values[account] == nil else { return false }
            values[account] = value
            return true
        }, read: { [self] in values[$0] }, delete: { [self] in
            deleted.append($0)
            values.removeValue(forKey: $0)
        })
    }
}
