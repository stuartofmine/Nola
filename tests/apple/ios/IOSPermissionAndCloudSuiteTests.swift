// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS host permission presentation, permission configuration owner, permission modes, cloud credential owner, container cloud configuration, and provider usage presence suite.

import Foundation
import XCTest
@testable import Nola

final class HostPermissionPresentationTests: XCTestCase {
    @MainActor
    func testImmediateRetryFromFailurePublicationOrCallbackCannotBeLost() async throws {
        for retryFromState in [true, false] {
            let session = IOSHostPermissionSession()
            let request = HostAgentPermissionRequest(id: "retry", allowConversation: true, toolName: "read", match: "fixture", nonce: "same-nonce")
            let submitted = expectation(description: "Explicit retry delivered")
            var sends = 0
            var failures = 0
            session.onStateChange = { state in
                guard let pending = state.request else { return }
                if state.submissionError == nil || retryFromState {
                    session.resolve(requestID: pending.id, decision: .deny)
                }
            }
            let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { payload in
                sends += 1
                XCTAssertEqual(payload["nonce"] as? String, "same-nonce")
                if sends == 1 { throw URLError(.notConnectedToInternet) }
            }, onSubmitted: { _ in submitted.fulfill() }, onSubmissionFailure: { request, _ in
                failures += 1
                if !retryFromState { session.resolve(requestID: request.id, decision: .deny) }
            })
            await fulfillment(of: [submitted], timeout: 1)
            // Also bound failure cleanup if the regression loses the retry.
            session.cancel()
            await task.value
            XCTAssertEqual(sends, 2)
            XCTAssertEqual(failures, 1)
        }
    }

    @MainActor
    func testImmediateDecisionWhenCardIsPublishedCannotBeLost() async throws {
        let session = IOSHostPermissionSession()
        let request = HostAgentPermissionRequest(id: "immediate", allowConversation: true, toolName: "read", match: "fixture", nonce: "nonce")
        let submitted = expectation(description: "Immediate visible-card decision delivered")
        session.onStateChange = { state in
            if let pending = state.request {
                session.resolve(requestID: pending.id, decision: .deny)
            }
        }
        let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { payload in
            XCTAssertEqual(payload["nonce"] as? String, "nonce")
            submitted.fulfill()
        }, onSubmitted: { _ in }, onSubmissionFailure: { _, error in
            XCTFail("Unexpected permission failure: \(error)")
        })
        defer { task.cancel(); session.cancel() }
        await fulfillment(of: [submitted], timeout: 1)
        session.cancel()
        await task.value
    }
    @MainActor
    func testRenewedNonceReplacesSameRequestWithoutSendingStaleDecision() async throws {
        let session = IOSHostPermissionSession()
        let conversation = UUID()
        let original = HostAgentPermissionRequest(id: "request", allowConversation: true, toolName: "read", nonce: "old", expiresAt: 100)
        let renewed = HostAgentPermissionRequest(id: "request", allowConversation: true, toolName: "read", nonce: "new", expiresAt: 400)
        let ready = expectation(description: "Original is visible")
        var visibleNonces: [String] = []
        var sentNonces: [String] = []
        session.onStateChange = { state in
            guard let request = state.request, !state.isSubmitting else { return }
            visibleNonces.append(request.nonce ?? "")
            if request.nonce == "old" { ready.fulfill() }
            else { session.resolve(requestID: request.id, decision: .allowOnce) }
        }
        let originalTask = session.start(original, conversationID: conversation, isCurrent: { true },
            send: { payload in sentNonces.append(payload["nonce"] as! String) },
            onSubmitted: { _ in XCTFail("Replaced challenge must not submit") },
            onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await fulfillment(of: [ready], timeout: 1)
        // Exact replay does not publish a second card or lose its receiver.
        session.start(original, conversationID: conversation, isCurrent: { true }, send: { _ in
            XCTFail("Replay must keep the original interaction")
        }, onSubmitted: { _ in }, onSubmissionFailure: { _, _ in })
        let successor = session.start(renewed, conversationID: conversation, isCurrent: { true },
            send: { payload in sentNonces.append(payload["nonce"] as! String) },
            onSubmitted: { _ in }, onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await originalTask.value
        await successor.value
        XCTAssertEqual(visibleNonces, ["old", "new"])
        XCTAssertEqual(sentNonces, ["new"])
    }

    @MainActor
    func testDecisionPublishesBusyAndIgnoresRepeatedTapUntilReply() async throws {
        let session = IOSHostPermissionSession()
        let request = HostAgentPermissionRequest(id: "busy", allowConversation: true, toolName: "read", match: "fixture", nonce: "nonce")
        var states: [Bool] = []
        var sends = 0
        session.onStateChange = { state in
            if state.request != nil {
                states.append(state.isSubmitting)
                // A second tap during publication must not resume twice.
                session.resolve(requestID: request.id, decision: .allowOnce)
            }
        }
        let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { _ in
            sends += 1
            XCTAssertEqual(states, [false, true])
            session.resolve(requestID: request.id, decision: .deny)
            try await Task.sleep(nanoseconds: 10_000_000)
        }, onSubmitted: { decision in
            XCTAssertEqual(decision, .allowOnce)
        }, onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await task.value
        XCTAssertEqual(sends, 1)
        XCTAssertEqual(states, [false, true])
    }

    func testUsesExplicitPermissionOwnerEvenWhenAnotherToolIsWaiting() {
        XCTAssertEqual(
            resolveActiveHostPermissionEventID(
                requestID: "permission-42",
                waitingEventIDs: ["call-1", "call-2"],
                permissionOwners: [(requestID: "permission-42", toolCallID: "call-1")]
            ),
            "call-1"
        )
    }

    func testDoesNotGuessFromWaitingOrderOrCoincidentIdentity() {
        XCTAssertNil(
            resolveActiveHostPermissionEventID(
                requestID: "call-2",
                waitingEventIDs: ["call-1", "call-2"],
                permissionOwners: []
            )
        )
    }

    func testDoesNotAttachRequestWithoutWaitingTool() {
        XCTAssertNil(
            resolveActiveHostPermissionEventID(
                requestID: "permission-42",
                waitingEventIDs: [],
                permissionOwners: [(requestID: "permission-42", toolCallID: "call-1")]
            )
        )
    }

    func testDoesNotAttachAmbiguousOrForeignPermissionOwner() {
        for owners in [
            [(requestID: "permission", toolCallID: "call-1"), (requestID: "permission", toolCallID: "call-2")],
            [(requestID: "older-permission", toolCallID: "call-1")],
            [(requestID: "permission", toolCallID: "foreign-call")],
        ] {
            XCTAssertNil(resolveActiveHostPermissionEventID(
                requestID: "permission", waitingEventIDs: ["call-1", "call-2"], permissionOwners: owners))
        }
    }
}

final class IOSPermissionConfigurationOwnerTests: XCTestCase {
    func testFailedHydrationCanRetryButOldAttemptCannotCompleteNewAttempt() throws {
        let identity = IOSRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        var hydration = IOSPermissionHydrationState()
        let first = try XCTUnwrap(hydration.begin(identity))
        XCTAssertNil(hydration.begin(identity), "Only one read may be in flight")
        XCTAssertTrue(hydration.finish(identity, request: first, succeeded: false))
        let retry = try XCTUnwrap(hydration.begin(identity))
        XCTAssertFalse(hydration.finish(identity, request: first, succeeded: true))
        XCTAssertTrue(hydration.finish(identity, request: retry, succeeded: true))
        XCTAssertNil(hydration.begin(identity), "A successful cold read is not repeated per snapshot")
        hydration.confirm(identity)
        XCTAssertNil(hydration.begin(identity), "Source commit alone does not restart ACK polling")
        hydration.invalidate(forgetSuccess: true)
        XCTAssertNotNil(hydration.begin(identity), "Explicit reconnect can recheck the same owner")
    }

    func testPermissionCompletionFenceRejectsAToBToAStaleReplyAndCleanup() {
        let a = UUID(), b = UUID(), first = UUID(), second = UUID()
        func current(_ owner: UUID?, _ token: UUID?) -> Bool {
            IOSPermissionChangeFence.matches(owner: a, changeID: first, selectedOwner: owner, currentChangeID: token)
        }
        XCTAssertTrue(current(a, first))
        XCTAssertFalse(current(b, first))
        XCTAssertFalse(current(a, nil))
        XCTAssertFalse(current(a, second), "Old success/failure must leave the new edit saving")
        XCTAssertTrue(IOSPermissionChangeFence.matches(owner: a, changeID: second, selectedOwner: a, currentChangeID: second))
    }

    private func value(_ input: Any) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: input, options: .fragmentsAllowed))
    }

    func testOwnerUsesOneCoreSnapshotAndPreservesUnresolvedObservation() throws {
        var calls = 0
        let owner = try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: "host", clientID: "client",
            unresolvedRunID: "run", sourceUnconfirmed: true) { command in
            calls += 1
            guard case .object(let input) = command else { throw IOSApplicationRuntimeError.invalidResponse }
            XCTAssertEqual(input["type"], .string("read_permission_configuration"))
            XCTAssertEqual(input["unresolved_run_ids"], .array([.string("run")]))
            XCTAssertEqual(input["source_unconfirmed"], .bool(true))
            return try self.value(["conversation_id":"conversation", "owner":["kind":"remote",
                "identity":["source_host_id":"host", "client_id":"client", "conversation_id":"conversation",
                    "submission_id":"run", "user_message_id":"user", "assistant_message_id":"assistant"]]])
        }
        XCTAssertEqual(calls, 1)
        guard case .remote(let identity) = owner else { return XCTFail("Expected source owner") }
        XCTAssertEqual(identity.submission_id, "run")
    }

    func testFailedSnapshotCannotBeDisplayedAsInactive() {
        XCTAssertThrowsError(try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: nil, clientID: nil,
            unresolvedRunID: nil, sourceUnconfirmed: false) { _ in
                throw IOSApplicationRuntimeError.rejected("owner changed")
            })
        XCTAssertThrowsError(try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: nil, clientID: nil,
            unresolvedRunID: nil, sourceUnconfirmed: false) { _ in
                try self.value(["conversation_id":"foreign", "owner":["kind":"inactive"]])
            })
    }
}

@MainActor
final class IOSCloudCredentialOwnerTests: XCTestCase {
    func testExplicitProtocolChangeRebindsCredentialAndPreservesOriginalRoute() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("route.db"))
        let vault = Vault()
        let service = IOSModelConnectionService(application: runtime, credentials: vault.owner)
        let original = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1",
            wireProtocol: "chat_completions", secret: "test-route-secret", modelID: "arbitrary-model")
        let profile = try XCTUnwrap(try runtime.listModelProfiles().first)
        let model = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
        let configured = try service.configureProtocol(for: model, wireProtocol: .responses)
        XCTAssertEqual(configured.wireProtocol, .responses)
        XCTAssertEqual(configured.effectiveUpstreamModelId, "arbitrary-model")
        let retained = try XCTUnwrap(try runtime.modelProfile(id: model.id))
        XCTAssertEqual(retained.sourceJSON, profile.sourceJSON)
        XCTAssertEqual(retained.parametersJSON, profile.parametersJSON)
        XCTAssertTrue(retained.enabled)
        let sources = try runtime.listModelProfiles()
        XCTAssertEqual(sources.count, 2)
        XCTAssertFalse(sources.contains { $0.sourceJSON.contains("test-route-secret") })
        let updated = try XCTUnwrap(try runtime.modelProfile(id: configured.id))
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(updated.sourceJSON.utf8)) as? [String: Any])
        let connectionID = try XCTUnwrap(source["connectionID"] as? String)
        XCTAssertNotEqual(connectionID, original.id)
        XCTAssertEqual(try service.credential(connectionID: connectionID, endpoint: "https://fixture.invalid/v1", wireProtocol: "responses").secret, "test-route-secret")
        XCTAssertThrowsError(try service.credential(connectionID: original.id, endpoint: "https://fixture.invalid/v1", wireProtocol: "responses"))
        XCTAssertEqual(try service.configureProtocol(for: model, wireProtocol: .responses).id, configured.id)
        XCTAssertEqual(try runtime.listModelProfiles().count, 2)
    }

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
            baseURL: endpoint, apiKey: secret, upstreamModelId: upstream, wireProtocol: wireProtocol,
            executionCapabilities: .init(samplingParameters: true))
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

@MainActor
final class IOSContainerCloudConfigurationTests: XCTestCase {
    func testCanonicalProfileAndParametersWinOverStaleSelectedValues() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "current-fixture-key")
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            let observed = try XCTUnwrap(store.modelProfile(id: model.id))
            let published = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(observed.sourceJSON.utf8)) as? [String: Any])
            XCTAssertEqual(published["capabilities"] as? [String], ["chat"])
            var parameters = ModelParameters.webAlignedCloudDefaults
            parameters.temperature = 0.37
            parameters.maxOutputTokens = 2_048
            let updated = IOSCanonicalModelProfile(id: observed.id, displayName: observed.displayName,
                provider: observed.provider, modelName: observed.modelName, runtime: observed.runtime,
                sourceJSON: observed.sourceJSON,
                parametersJSON: String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self),
                selected: observed.selected, enabled: observed.enabled, revision: observed.revision,
                createdAtMs: observed.createdAtMs, updatedAtMs: observed.updatedAtMs + 1, deletedAtMs: nil)
            _ = try store.upsertModelProfileCAS(updated, expectedRevision: UInt64(observed.revision))
            let stale = cloudModel(endpoint: "https://stale.invalid/v1", secret: "stale-key", wireProtocol: .chatCompletions)
            let config = try prepare(stale, store: store, vault: vault)
            XCTAssertEqual(config.baseURL.absoluteString, model.baseURL)
            XCTAssertEqual(config.apiKey, "current-fixture-key")
            XCTAssertEqual(config.wireProtocol, .responses)
            XCTAssertEqual(config.temperature, 0.37)
            XCTAssertEqual(config.maxCompletionTokens, 2_048)
            XCTAssertEqual(config.model, model.effectiveUpstreamModelId)
        }
    }

    func testEveryPreparationRevalidatesCredentialBeforeAnyDedupe() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "saved-key")
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            _ = try prepare(model, store: store, vault: vault)
            vault.removeAll()
            XCTAssertThrowsError(try prepare(model, store: store, vault: vault),
                "The selected ModelInfo's old key must not replace a revoked credential")
        }
    }

    func testExplicitAnonymousIsAcceptedButUnpublishedOrMissingSelectionIsRejected() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "")
            XCTAssertThrowsError(try prepare(model, store: store, vault: vault))
            XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: nil) { _ in
                XCTFail("A missing selection must not invoke a fallback resolver")
                throw IOSCloudCredentialError.unavailable
            })
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            XCTAssertEqual(try prepare(model, store: store, vault: vault).apiKey, "")
        }
    }

    func testImportedConnectionProfilesAdmitAnonymousAndAccountCredentials() throws {
        for secret in ["", "fixture-account-key"] {
            try withStore { store, vault in
                let service = IOSModelConnectionService(application: store, credentials: vault.owner)
                let connection = try service.create(name: "WSL fixture", endpoint: "http://127.0.0.1:18080/v1",
                    wireProtocol: "chat_completions", secret: secret, modelID: "qwen3.8-flash-next-iq3_s")
                let catalog: IOSModelConnectionSnapshot = try service.command([
                    "type": .string("refresh_connection_catalog"), "connection_id": .string(connection.id),
                    "expected_revision": .signedInteger(connection.revision), "observed_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000) + 1),
                    "models": .array([.object([
                        "model_id": .string("qwen3.8-flash-next-iq3_s"), "display_name": .string("Qwen fixture"),
                        "protocol": .string("chat_completions"), "context_window": .null, "output_limit": .null,
                        "modalities": .array([]), "reasoning_levels": .array([]), "execution_capabilities": .null
                    ])])])
                try service.enable(catalog, modelIDs: ["chat_completions:qwen3.8-flash-next-iq3_s"])
                let profile = try XCTUnwrap(store.listModelProfiles().first)
                XCTAssertFalse(profile.sourceJSON.contains("cloudCredential"), "Connections use account admission, not a per-profile credential field")
                let selected = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
                let config = try prepare(selected, store: store, vault: vault)
                XCTAssertEqual(config.apiKey, secret)
                XCTAssertEqual(config.model, "qwen3.8-flash-next-iq3_s")
                XCTAssertEqual(config.maxCompletionTokens, 16_384)
                let resolved = try ModelSelectionManager.resolvedCloudProfileForRun(selected, using: store, credentials: vault.owner)
                if secret.isEmpty {
                    XCTAssertEqual(resolved.model.cloudCredential, .anonymous)
                } else {
                    vault.removeAll()
                    XCTAssertThrowsError(try prepare(selected, store: store, vault: vault)) {
                        XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .credential)
                    }
                }
                _ = try service.disconnect(try XCTUnwrap(service.connections().first))
                XCTAssertThrowsError(try prepare(selected, store: store, vault: vault))
            }
        }
    }

    func testPreparationDiagnosticsIdentifyStageWithoutIncludingUnderlyingSecrets() throws {
        let selected = cloudModel(secret: "fixture-secret")
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { _ in
            throw NSError(domain: "private-token", code: 1)
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .profile) }
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { _ in
            throw IOSCloudRunCredentialResolutionError(underlyingError: NSError(domain: "private-token", code: 1))
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .credential) }
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { model in
            IOSResolvedCloudRunProfile(model: model, profileID: model.id, revision: 1, parametersJSON: "[]")
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .parameters) }
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 3
        let json = String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self)
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { model in
            IOSResolvedCloudRunProfile(model: model, profileID: model.id, revision: 1, parametersJSON: json)
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .providerValidation) }
    }

    func testExplicitWorkspaceSelectionSurvivesModeChangesAndDeferredRefreshes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("owner.db"))
        let vault = ContainerCredentialVault()
        let defaultModel = ModelInfo(id: "default-profile", name: "Old default", source: "Fixture", isLocal: false,
            baseURL: "https://default.invalid/v1", apiKey: "", upstreamModelId: "old-default", wireProtocol: .chatCompletions, executionCapabilities: .init(samplingParameters: true))
        let chosen = ModelInfo(id: "conversation-profile", name: "Qwen", source: "Fixture", isLocal: false,
            baseURL: "https://conversation.invalid/v1", apiKey: "", upstreamModelId: "qwen-current", wireProtocol: .chatCompletions, executionCapabilities: .init(samplingParameters: true))
        _ = try ModelSelectionManager.publishCloudModel(defaultModel, existing: nil, selected: true, using: store, credentials: vault.owner)
        _ = try ModelSelectionManager.publishCloudModel(chosen, existing: nil, selected: false, using: store, credentials: vault.owner)
        _ = try store.putProductSettingChecked(key: "chat.default_model",
            valueJSON: "{\"version\":1,\"selection\":{\"kind\":\"profile\",\"profile_id\":\"default-profile\"}}",
            expectedRevision: nil, updatedAtMs: 1)
        let container = ServiceContainer(applicationRuntime: store)
        func assertChosen() throws {
            let service = try XCTUnwrap(container.llmClient as? ModelService)
            let provider = try XCTUnwrap(service.currentProvider() as? OpenAIProvider)
            XCTAssertEqual(provider.auxiliaryModelIdentity, "qwen-current")
            XCTAssertEqual(container.configurationModel?.id, chosen.id)
        }
        container.useLocalModel = false
        container.reconfigureIfPossible(selectedModel: chosen)
        container.useLocalModel = false // @Published emits even when the value is unchanged.
        await Task.yield()
        container.reconfigureIfPossible()
        try assertChosen()
        XCTAssertEqual(try ModelSelectionManager.lastSelectedModel(using: store)?.id, defaultModel.id)

        let local = ModelInfo(id: "fixture-local", name: "Local fixture", source: "Local", isLocal: true)
        container.useLocalModel = true
        container.reconfigureIfPossible(selectedModel: local)
        await Task.yield()
        XCTAssertEqual(container.configurationModel?.id, local.id)
        XCTAssertFalse((container.llmClient as? ModelService)?.currentProvider() is OpenAIProvider)
        container.useLocalModel = false
        container.reconfigureIfPossible(selectedModel: chosen)
        await Task.yield()
        container.reconfigureIfPossible()
        try assertChosen()

        _ = try store.deleteModelProfile(id: chosen.id)
        container.reconfigureIfPossible()
        XCTAssertFalse((container.llmClient as? ModelService)?.currentProvider() is OpenAIProvider,
            "Retaining an explicit owner must not bypass profile revocation")
    }

    private func prepare(_ selected: ModelInfo, store: IOSApplicationRuntime,
        vault: ContainerCredentialVault) throws -> OpenAIProvider.Config {
        try IOSContainerCloudConfiguration.prepare(selected: selected) {
            try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: vault.owner)
        }
    }

    private func cloudModel(endpoint: String = "https://provider.invalid/v1", secret: String,
        wireProtocol: ModelWireProtocol = .responses) -> ModelInfo {
        ModelInfo(id: "container-profile", name: "Fixture", source: "Fixture", isLocal: false,
            baseURL: endpoint, apiKey: secret, upstreamModelId: "fixture-model", wireProtocol: wireProtocol,
            executionCapabilities: .init(samplingParameters: true))
    }

    private func withStore(_ body: (IOSApplicationRuntime, ContainerCredentialVault) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db")), ContainerCredentialVault())
    }
}

private nonisolated final class ContainerCredentialVault: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Data] = [:]
    func removeAll() { lock.withLock { values.removeAll() } }
    var owner: IOSCloudCredentialOwner {
        .init(save: { [self] account, value in lock.withLock {
            guard values[account] == nil else { return false }
            values[account] = value
            return true
        } }, read: { [self] account in lock.withLock { values[account] } },
        delete: { [self] account in lock.withLock { _ = values.removeValue(forKey: account) } })
    }
}

final class IOSPermissionModeAndProviderUsageTests: XCTestCase {
    func testPermissionModeWireValuesPersistenceInvalidRejectionAndRuntimeDefault() throws {
        XCTAssertEqual(IOSPermissionMode.allCases.map(\.rawValue), ["cautious", "allow_all"])
        XCTAssertTrue(IOSPermissionMode.cautious.autoApprove)
        XCTAssertEqual(IOSPermissionMode.cautious.networkPolicy, "ask")
        XCTAssertTrue(IOSPermissionMode.allowAll.autoApprove)
        XCTAssertEqual(IOSPermissionMode.allowAll.networkPolicy, "allow")
        XCTAssertEqual(Conversation().permissionMode, .cautious)
        XCTAssertEqual(AgentCoreRuntimeConfig().permissionProfile, .cautious)

        var conversation = Conversation(sessionMemory: "keep this memory")
        conversation.permissionMode = .allowAll
        let encoded = try JSONEncoder().encode(conversation)
        XCTAssertEqual(try JSONDecoder().decode(Conversation.self, from: encoded).permissionMode, .allowAll)

        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        object.removeValue(forKey: "permissionMode")
        XCTAssertEqual(try JSONDecoder().decode(Conversation.self, from: JSONSerialization.data(withJSONObject: object)).permissionMode, .cautious)

        for invalid in ["safe", "recommended", "moderate", "allow-all", " AllowAll ", "unknown", "", true, NSNull()] as [Any] {
            object["permissionMode"] = invalid
            XCTAssertThrowsError(try JSONDecoder().decode(Conversation.self, from: JSONSerialization.data(withJSONObject: object)))
            XCTAssertEqual(object["sessionMemory"] as? String, "keep this memory")
        }
    }

    @MainActor
    func testProviderTokenUsagePresenceAndPartialZeroPreservation() throws {
        for json in ["{}", #"{"prompt_tokens":null,"completion_tokens":null,"total_tokens":null,"reasoning_tokens":null}"#] {
            let usage = try JSONDecoder().decode(AgentCoreTokenUsage.self, from: Data(json.utf8))
            XCTAssertNil(usage.promptTokens); XCTAssertNil(usage.completionTokens); XCTAssertNil(usage.totalTokens)
            guard case .object(let wire) = DeviceAgentCoordinator.v2Usage(usage) else { return XCTFail("Missing payload") }
            for key in ["input_tokens", "output_tokens", "total_tokens", "reasoning_tokens", "cached_input_tokens", "cache_write_input_tokens"] {
                XCTAssertEqual(wire[key], .null, key)
            }
            XCTAssertEqual(wire["request_count"], .number(1))
            XCTAssertNil(wire["cached_input_tokens_reported"]); XCTAssertNil(wire["cache_write_input_tokens_reported"])
        }
        let partial = AgentCoreTokenUsage(promptTokens: 12, cachedInputTokens: 0, completionTokens: 0)
        guard case .object(let partialWire) = DeviceAgentCoordinator.v2Usage(partial) else { return XCTFail("Missing payload") }
        XCTAssertEqual(partialWire["input_tokens"], .number(12))
        XCTAssertEqual(partialWire["cached_input_tokens"], .number(0))
        XCTAssertEqual(partialWire["output_tokens"], .number(0))
        XCTAssertEqual(partialWire["total_tokens"], .null)
        XCTAssertEqual(partialWire["reasoning_tokens"], .null)
        let known = AgentCoreTokenUsage(promptTokens: 0, completionTokens: 0, totalTokens: 0, reasoningTokens: 0)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreTokenUsage.self, from: JSONEncoder().encode(known)), known)
        XCTAssertNil(LLMStreamUsage().promptTokens); XCTAssertNil(LLMStreamUsage().totalTokens)
    }
}
