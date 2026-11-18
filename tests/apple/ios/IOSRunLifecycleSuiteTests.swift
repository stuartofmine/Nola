// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS run configuration admission, execution permits, run observation, tool plan sessions/lifecycle, exact run invocation, local run configuration, and prepared run model suite.

import CryptoKit
import Foundation
import MLXLMCommon
import Testing
import XCTest
@testable import Nola

@MainActor
final class IOSRunConfigurationAdmissionTests: XCTestCase {
    func testRecoveryUsesFrozenGenerationAndRequiresLiveCredentialAuthorization() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  original  "
        try withProfile(protocol: .chatCompletions, sampling: true, parameters: parameters) { store, model, owner in
            let prepared = try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
                resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner) },
                admitOutput: { try IOSPreparedHostOutputAdmission.admit(route: $0, outputTokens: $1, using: store) })
            let original = try prepared.prepareHostInvocation()
            let frozen = IOSExactRunInvocation(conversationID: "conversation", runID: "run",
                assistantMessageID: "assistant", invocationConfigDigest: String(repeating: "a", count: 64),
                status: "running", hostInvocation: try original.binding(executionPolicy: .object([:])),
                contextBudget: .object(["context_window_tokens": .number(Double(parameters.maxContextLength)),
                    "reserved_output_tokens": .number(64)]), remoteToolPlan: .null, coreSessionID: "core",
                checkpointRevision: 1, checkpointEventCursor: 1, checkpointSHA256: String(repeating: "b", count: 64))
            let stored = try XCTUnwrap(store.modelProfile(id: model.id))
            var changed = parameters
            changed.temperature = 1.5
            changed.maxOutputTokens = 512
            changed.seed = nil
            let update = IOSCanonicalModelProfile(id: stored.id, displayName: stored.displayName,
                provider: stored.provider, modelName: stored.modelName, runtime: stored.runtime,
                sourceJSON: stored.sourceJSON, parametersJSON: String(decoding: try JSONEncoder().encode(changed), as: UTF8.self),
                selected: stored.selected, enabled: stored.enabled, revision: stored.revision,
                createdAtMs: stored.createdAtMs, updatedAtMs: stored.updatedAtMs + 1, deletedAtMs: nil)
            _ = try store.upsertModelProfileCAS(update, expectedRevision: UInt64(stored.revision))
            let restored = try IOSPreparedRunModel.recover(frozen, using: store, credentials: owner)
            XCTAssertEqual(restored.parameters.temperature, 0)
            XCTAssertEqual(restored.parameters.maxOutputTokens, 64)
            XCTAssertEqual(restored.parameters.seed, 9_007_199_254_740_993)
            XCTAssertEqual(try restored.prepareHostInvocation().generation, original.generation)
            _ = try store.deleteModelProfile(id: model.id)
            XCTAssertThrowsError(try IOSPreparedRunModel.recover(frozen, using: store, credentials: owner))
        }
    }

    func testResponsesUnsupportedOptionsFailBeforeOutputAdmissionOrRunCreation() throws {
        for edit: (inout ModelParameters) -> Void in [
            { $0.seed = 0 }, { $0.stopSequences = " END " },
            { $0.presencePenalty = 0.1 }, { $0.frequencyPenalty = -0.1 }
        ] {
            var parameters = ModelParameters.webAlignedCloudDefaults
            edit(&parameters)
            try withProfile(protocol: .responses, sampling: true, parameters: parameters) { store, model, owner in
                try assertRejectedBeforeAdmission(store: store, model: model, owner: owner, parameters: parameters)
            }
        }
    }

    func testUndeclaredSamplingNondefaultsFailDuringPreparation() throws {
        for wireProtocol: ModelWireProtocol in [.chatCompletions, .responses] {
            var parameters = ModelParameters.webAlignedCloudDefaults
            parameters.temperature = 0
            parameters.topP = 0
            try withProfile(protocol: wireProtocol, sampling: false, parameters: parameters) { store, model, owner in
                try assertRejectedBeforeAdmission(store: store, model: model, owner: owner, parameters: parameters)
            }
        }
    }

    func testDeclaredChatPreservesLargeSeedAndStopWhitespace() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  END  \n next "
        try withProfile(protocol: .chatCompletions, sampling: true, parameters: parameters) { store, model, owner in
            let config = try prepareContainer(store: store, model: model, owner: owner)
            XCTAssertEqual(config.seed, parameters.seed)
            XCTAssertEqual(config.stopSequences, ["  END  ", " next "])
            XCTAssertTrue(try store.listConversations().isEmpty)
        }
    }

    func testDeclaredResponsesPreservesExplicitZeroSampling() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0
        parameters.topP = 0
        try withProfile(protocol: .responses, sampling: true, parameters: parameters) { store, model, owner in
            let config = try prepareContainer(store: store, model: model, owner: owner)
            XCTAssertEqual(config.temperature, 0)
            XCTAssertEqual(config.topP, 0)
            XCTAssertTrue(try store.listConversations().isEmpty)
        }
    }

    private func assertRejectedBeforeAdmission(store: IOSApplicationRuntime, model: ModelInfo,
        owner: IOSCloudCredentialOwner, parameters: ModelParameters) throws {
        XCTAssertThrowsError(try prepareContainer(store: store, model: model, owner: owner))
        var admissions = 0
        XCTAssertThrowsError(try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
            resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner) },
            admitOutput: { _, _ in
                admissions += 1
                XCTFail("Invalid generation configuration reached output admission")
                throw IOSApplicationRuntimeError.invalidResponse
            }))
        XCTAssertEqual(admissions, 0)
        XCTAssertTrue(try store.listConversations().isEmpty)
    }

    private func prepareContainer(store: IOSApplicationRuntime, model: ModelInfo,
        owner: IOSCloudCredentialOwner) throws -> OpenAIProvider.Config {
        try IOSContainerCloudConfiguration.prepare(selected: model) {
            try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner)
        }
    }

    private func withProfile(protocol wireProtocol: ModelWireProtocol, sampling: Bool, parameters: ModelParameters,
        _ body: (IOSApplicationRuntime, ModelInfo, IOSCloudCredentialOwner) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        let vault = AdmissionCredentialVault()
        let owner = vault.owner
        let model = ModelInfo(id: "admission-fixture", name: "Fixture", source: "Fixture", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "fixture-only-key", upstreamModelId: "fixture-model",
            wireProtocol: wireProtocol, executionCapabilities: .init(samplingParameters: sampling))
        _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
            using: store, credentials: owner)
        let stored = try XCTUnwrap(store.modelProfile(id: model.id))
        let input = IOSCanonicalModelProfile(id: stored.id, displayName: stored.displayName, provider: stored.provider,
            modelName: stored.modelName, runtime: stored.runtime, sourceJSON: stored.sourceJSON,
            parametersJSON: String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self),
            selected: stored.selected, enabled: stored.enabled, revision: stored.revision,
            createdAtMs: stored.createdAtMs, updatedAtMs: stored.updatedAtMs + 1, deletedAtMs: nil)
        _ = try store.upsertModelProfileCAS(input, expectedRevision: UInt64(stored.revision))
        try body(store, model, owner)
    }
}

private nonisolated final class AdmissionCredentialVault: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Data] = [:]
    var owner: IOSCloudCredentialOwner {
        .init(save: { [self] account, value in lock.withLock {
            guard values[account] == nil else { return false }
            values[account] = value
            return true
        } }, read: { [self] account in lock.withLock { values[account] } },
        delete: { [self] account in lock.withLock { _ = values.removeValue(forKey: account) } })
    }
}

private actor PermissionGETAttemptCounter {
    private var count = 0
    func next() -> Int { count += 1; return count }
    func value() -> Int { count }
}

@MainActor final class IOSRunObservationTests: XCTestCase {
    func testIdleObservationRequiresExactEnvelopeOwnerBeforeAcceptingAbsence() throws {
        let request = IOSRunObservationRequest(conversation: "conversation", host: "host", client: "client")
        func decode(_ conversation: String, _ client: String) throws -> IOSRunObservationEnvelope {
            try JSONDecoder().decode(IOSRunObservationEnvelope.self, from: JSONSerialization.data(withJSONObject: [
                "source_host_id": "host", "conversation_id": conversation, "client_id": client, "result": NSNull()]))
        }
        XCTAssertNil(try decode("conversation", "client").validated(for: request))
        XCTAssertThrowsError(try decode("other", "client").validated(for: request))
        XCTAssertThrowsError(try decode("conversation", "other").validated(for: request))
        XCTAssertThrowsError(try decode("conversation", "client").validated(for: .init(
            conversation: "conversation", host: "host", client: "client", expectedRunID: "run", assistantID: "assistant")))
    }

    func testColdPermissionGETFailureReleasesHydrationAndReconnectUsesExactOwnerAgain() async throws {
        let owner = IOSRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let attempts = PermissionGETAttemptCounter()
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: "host", clientId: "client")) { request in
            let attempt = await attempts.next()
            let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: request.httpBody!) as? [String: Any])
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            XCTAssertEqual(input["operation"] as? String, "read")
            XCTAssertEqual(input["expected_run_id"] as? String, owner.submission_id)
            XCTAssertEqual(input["assistant_message_id"] as? String, owner.assistant_message_id)
            if attempt == 1 { throw URLError(.notConnectedToInternet) }
            var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": [
                "conversation_id": "conversation", "configuration_revision": 12,
                "permission_profile": "cautious", "active_run_id": "run", "active_policy": NSNull(), "owner": ["kind": "local", "run_id": "run", "assistant_message_id": "assistant"], "state": "pending",
            ]], token: "fixture")
            encrypted["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: encrypted)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { continuation in
                data.forEach { continuation.yield($0) }; continuation.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        var hydration = IOSPermissionHydrationState()
        let first = try XCTUnwrap(hydration.begin(owner))
        do { _ = try await provider.permissionConfiguration(owner: owner); XCTFail("Expected first GET to fail") }
        catch { XCTAssertTrue(hydration.finish(owner, request: first, succeeded: false)) }
        let retry = try XCTUnwrap(hydration.begin(owner))
        let current = try await provider.permissionConfiguration(owner: owner)
        XCTAssertEqual(current.active_run_id, owner.submission_id)
        XCTAssertNil(current.active_policy, "A cold configuration is not a fabricated active-policy ACK")
        XCTAssertTrue(hydration.finish(owner, request: retry, succeeded: true))
        XCTAssertNil(hydration.begin(owner))
        let totalAttempts = await attempts.value()
        XCTAssertEqual(totalAttempts, 2)
    }

    func testRemotePermissionCASUsesPairedExactRunAndCannotSubmitOtherSettings() async throws {
        let owner = IOSRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/configuration")
            let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: request.httpBody!) as? [String: Any])
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            let patch = input["operation"] as? String == "patch"
            XCTAssertEqual(Set(input.keys), Set(["operation", "expected_host_id", "client_id", "conversation_id", "expected_run_id", "assistant_message_id"] +
                (patch ? ["expected_revision", "permission_profile"] : [])))
            XCTAssertEqual(input["expected_run_id"] as? String, "run")
            XCTAssertEqual(input["assistant_message_id"] as? String, "assistant")
            if patch {
                XCTAssertEqual(input["expected_revision"] as? Int, 12)
                XCTAssertEqual(input["permission_profile"] as? String, "allow_all")
            }
            var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": [
                "conversation_id": "conversation", "configuration_revision": patch ? 13 : 12,
                "permission_profile": patch ? "allow_all" : "cautious", "active_run_id": "run", "active_policy": NSNull(), "owner": ["kind": "local", "run_id": "run", "assistant_message_id": "assistant"], "state": "pending",
            ]], token: "fixture")
            encrypted["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: encrypted)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { continuation in
                data.forEach { continuation.yield($0) }; continuation.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        let baseline = try await provider.permissionConfiguration(owner: owner)
        XCTAssertEqual(baseline.configuration_revision, 12)
        let saved = try await provider.permissionConfiguration(owner: owner, expectedRevision: 12, profile: .allowAll)
        XCTAssertEqual(saved.configuration_revision, 13)
        XCTAssertEqual(saved.permission_profile, .allowAll)
        XCTAssertNil(saved.active_policy, "Configuration commit is not an execution ACK")
    }

    func testCancellationCarriesFrozenSubmissionAndNeverOnlyConversationIdentity() async throws {
        let identity = IOSRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "original", user_message_id: "user",
            assistant_message_id: "assistant")
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/cancel")
            let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: request.httpBody!) as? [String: Any])
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            XCTAssertEqual(Set(input.keys), ["identity"])
            let sent = try JSONDecoder().decode(IOSRemoteSubmissionIdentity.self,
                from: JSONSerialization.data(withJSONObject: input["identity"]!))
            XCTAssertEqual(sent, identity)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { $0.finish() },
                response: HTTPURLResponse(url: request.url!, statusCode: 202, httpVersion: nil, headerFields: nil)!)
        }
        try await provider.cancelHostGeneration(identity: identity)
    }

    nonisolated private static func object(recovery: String = "observe", worker: String = "running") -> [String: Any] {
        ["protocol_version": 1, "conversation_id": "conversation", "client_id": "client",
         "run_id": "run", "assistant_message_id": "assistant", "status": "running",
         "worker": ["state": worker, "generation": worker == "absent" ? NSNull() : "9007199254740993"] as [String: Any], "recovery": recovery,
         "admission": ["identity": ["source_store_epoch": "epoch", "conversation_id": "conversation",
             "run_id": "run", "assistant_message_id": "assistant"], "sequence": "9007199254740993",
             "record_digest": String(repeating: "a", count: 64)],
         "terminal_seal": NSNull(), "permissions": [], "can_submit": false]
    }

    nonisolated private static func decode(_ value: [String: Any]) throws -> IOSRunObservation {
        try JSONDecoder().decode(IOSRunObservation.self, from: JSONSerialization.data(withJSONObject: value))
    }

    func testColdDiscoveryUsesAuthenticatedReadEvenWithoutAnyJournalHead() async throws {
        let config = WebAgentProvider.Config(baseURL: URL(string: "http://127.0.0.1:8123")!, token: "fixture",
            expectedHostId: "host", clientId: "client")
        let provider = WebAgentProvider(config: config) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/observe")
            let wire = try JSONSerialization.jsonObject(with: request.httpBody!) as! [String: Any]
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            XCTAssertEqual(input["operation"] as? String, "discover")
            XCTAssertEqual(input["expected_host_id"] as? String, "host")
            XCTAssertEqual(input["client_id"] as? String, "client")
            XCTAssertNil(input["messages"])
            var response = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "conversation_id": "conversation", "client_id": "client", "result": Self.object()], token: "fixture")
            response["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: response)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { continuation in
                data.forEach { continuation.yield($0) }; continuation.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        let value = try await provider.observeRun(.init(conversation: "conversation", host: "host", client: "client"))
        XCTAssertEqual(value?.admission.sequence, "9007199254740993")
        try XCTUnwrap(value).validate(conversation: "conversation", client: "client", previous: nil)
    }

    func testObservedContextRecordIsBoundToAdmissionAndPreservesDecimalSequence() throws {
        var wire = Self.object()
        let admission = try XCTUnwrap(wire["admission"] as? [String: Any])
        var usage: [String: Any] = ["protocol_version": 1, "identity": admission["identity"]!, "admission": admission,
            "attempt_sequence": "0", "run_cache_usage": NSNull(),
            "event_sequence": "9007199254740993", "record_digest": String(repeating: "b", count: 64),
            "usage": ["measurement": NSNull(), "prompt_cache_measurement": NSNull(), "prompt_cache_diagnostic": NSNull(),
                "used_tokens": "7500", "context_window_tokens": "32768", "prompt_token_limit": "28672", "estimated": false]]
        wire["context_usage"] = usage
        let observed = try Self.decode(wire)
        try observed.validate(conversation: "conversation", client: "client", previous: nil)
        XCTAssertEqual(observed.context_usage?.event_sequence, "9007199254740993")
        var owner = try XCTUnwrap(admission["identity"] as? [String: Any])
        owner["assistant_message_id"] = "foreign-root"
        usage["identity"] = owner
        wire["context_usage"] = usage
        XCTAssertThrowsError(try Self.decode(wire).validate(conversation: "conversation", client: "client", previous: nil))
    }

    func testOwnerMismatchAndUnknownTerminalAreNotIdle() throws {
        var raw = Self.object()
        raw["client_id"] = "another-client"
        XCTAssertThrowsError(try Self.decode(raw).validate(conversation: "conversation", client: "client", previous: nil))
        raw = Self.object(); raw["can_submit"] = true
        XCTAssertThrowsError(try Self.decode(raw).validate(conversation: "conversation", client: "client", previous: nil))
        raw = Self.object(); raw["status"] = "completed"
        XCTAssertThrowsError(try Self.decode(raw).validate(conversation: "conversation", client: "client", previous: nil))
        let previous = try Self.decode(Self.object())
        raw = Self.object(); raw["run_id"] = "replacement"
        XCTAssertThrowsError(try Self.decode(raw).validate(conversation: "conversation", client: "client", previous: previous))
    }

    func testColdKnownRunRequestsExactOwnerRatherThanDiscoveringOnlyActiveHostRun() throws {
        let request = IOSRunObservationRequest(conversation: "conversation", host: "host", client: "client",
            expectedRunID: "already-completed-on-host", assistantID: "durable-root")
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as! [String: Any]
        XCTAssertEqual(object["operation"] as? String, "observe")
        XCTAssertEqual(object["expected_run_id"] as? String, "already-completed-on-host")
        XCTAssertEqual(object["assistant_message_id"] as? String, "durable-root")
        XCTAssertNil(object["messages"])
    }

    func testObservedPermissionUsesOriginalNonceWithoutTraceAndRejectsForeignRun() throws {
        var raw = Self.object()
        var permission: [String: Any] = ["id": "request", "canonical_run_id": "run", "session_id": "conversation",
            "host_id": "host", "client_id": "client", "protocol_version": WebAgentProvider.hostPermissionProtocolVersion,
            "nonce": "original-nonce", "expires_at": 200, "tool_name": "shell", "permission_instructions": [], "allow_conversation": false,
            "safe_review_text": "Read project files"]
        raw["permissions"] = [permission]
        let request = try XCTUnwrap(Self.decode(raw).permissionRequests(host: "host", client: "client", now: 100).first)
        XCTAssertEqual(request.nonce, "original-nonce")
        XCTAssertEqual(request.argsPreview, "Read project files")
        permission["canonical_run_id"] = "foreign"; raw["permissions"] = [permission]
        XCTAssertThrowsError(try Self.decode(raw).permissionRequests(host: "host", client: "client", now: 100))
    }

    func testObserverHasBoundedErrorsAndNeverConverts409Or401ToIdle() async {
        for status in [409, 401, 404] {
            let observer = IOSRemoteRunObserver()
            let failed = expectation(description: "bounded failure \(status)")
            var reads = 0
            observer.start(read: { _ in reads += 1; throw IOSRunObservationTransportError.http(status) },
                receive: { _ in XCTFail("No accepted observation") }, idle: { XCTFail("HTTP is not idle") },
                failed: { _ in failed.fulfill() }, sleep: { await Task.yield() })
            await fulfillment(of: [failed], timeout: 2)
            XCTAssertEqual(reads, 3)
            observer.cancel()
        }
    }

    func testGenerationSwitchDropsDelayedOldObservation() async throws {
        let observer = IOSRemoteRunObserver()
        let entered = expectation(description: "first read")
        let idle = expectation(description: "new selection idle")
        var suspended: CheckedContinuation<IOSRunObservation?, Never>?
        observer.start(read: { _ in
            await withCheckedContinuation { continuation in suspended = continuation; entered.fulfill() }
        }, receive: { _ in XCTFail("Old generation must not render or ingest") }, idle: { XCTFail("Old idle") }, failed: { _ in XCTFail() })
        await fulfillment(of: [entered], timeout: 2)
        observer.start(read: { _ in nil }, receive: { _ in XCTFail() }, idle: { idle.fulfill() }, failed: { _ in XCTFail() })
        suspended?.resume(returning: try Self.decode(Self.object()))
        await fulfillment(of: [idle], timeout: 2)
        observer.cancel()
    }

    func testExactResumeUsesNoNewInputAnd409RemainsObservable() async throws {
        let run = try Self.decode(Self.object(recovery: "resume_required", worker: "absent"))
        let config = WebAgentProvider.Config(baseURL: URL(string: "http://127.0.0.1:8123")!, token: "fixture",
            expectedHostId: "host", clientId: "client")
        let provider = WebAgentProvider(config: config) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/resume")
            let wire = try JSONSerialization.jsonObject(with: request.httpBody!) as! [String: Any]
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            XCTAssertEqual(Set(input.keys), Set(["conversation_id", "expected_run_id", "expected_host_id", "client_id"]))
            XCTAssertEqual(input["expected_run_id"] as? String, "run")
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { $0.finish() },
                response: HTTPURLResponse(url: request.url!, statusCode: 409, httpVersion: nil, headerFields: nil)!)
        }
        do {
            for try await _ in provider.resumeObservedRun(run) { XCTFail("No new stream is owned") }
            XCTFail("Expected busy worker conflict")
        } catch IOSRunObservationTransportError.http(let status) { XCTAssertEqual(status, 409) }
    }
}

final class IOSRunToolPlanSessionTests: XCTestCase {
    @MainActor
    func testExpiredReceiptRenewsOriginalIntentWithoutExpandingSelection() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true,
            allowedTools: ["memory_search"], deniedTools: ["device_clipboard_read"], nowMs: now - 61_000)
        let original = try XCTUnwrap(plan.originalIntent)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now, isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let fresh = try await session.currentPlan()
        XCTAssertGreaterThan(fresh.output.manifestValidUntilMs, now)
        XCTAssertEqual(fresh.originalIntent, original)
        XCTAssertEqual(fresh.expectedIdentity, plan.expectedIdentity)
        XCTAssertTrue(fresh.output.enabledTools.allSatisfy { $0.toolId == "memory_search" })
        XCTAssertFalse(fresh.output.enabledTools.contains { $0.toolId == "device_clipboard_read" })
        let again = try await session.currentPlan()
        XCTAssertEqual(again, fresh)
    }

    @MainActor
    func testExpiredOriginalScopedGrantIsRemovedNotRenewed() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let grant = AgentCoreScopedGrant(capability: AgentCoreCapability.filesRead.rawValue,
            route: "device", scope: "file:///isolated", expiresAtMs: now - 1)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, scopedGrants: [grant], nowMs: now - 1000)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now, isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let refreshed = try await session.currentPlan()
        XCTAssertTrue(refreshed.scopedGrants.isEmpty)
        XCTAssertEqual(refreshed.originalIntent?.scopedGrants, [grant])
    }

    @MainActor
    func testConcurrentRefreshSharesObservationAndNextCallRechecksRevocation() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, nowMs: now - 61_000)
        var revoked = plan.authorizationSnapshot
        revoked[AgentCoreCapability.clipboard.rawValue] = "denied"
        let latest = IOSRunToolPlanObservation(nowMs: now, isForeground: true, authorizations: revoked)
        let gate = ObservationGate(latest: latest)
        let session = IOSRunToolPlanSession(plan: plan, observe: { await gate.observe() })
        let old = Task { try await session.currentPlan() }
        await gate.waitUntilSuspended()
        let second = Task { try await session.currentPlan() }
        await gate.release(latest)
        let returned = try await old.value
        let fresh = try await second.value
        XCTAssertEqual(fresh, returned)
        let subsequent = try await session.currentPlan()
        XCTAssertEqual(subsequent.authorizationSnapshot, revoked)
        XCTAssertEqual(returned.authorizationSnapshot, revoked)
        XCTAssertFalse(returned.output.enabledTools.contains { $0.toolId == "device_clipboard_read" })
    }

    @MainActor
    func testForegroundRefreshDoesNotExtendOriginalEgressConsent() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let authorization = try XCTUnwrap(IOSAgentResultEgressAuthorization.explicitUserApproval(
            identity: identity, userApproved: true, nowMs: now))
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .hybrid,
            expectedIdentity: identity, isForeground: true, resultEgressAuthorization: authorization, nowMs: now)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now + 1, isForeground: false, authorizations: plan.authorizationSnapshot)
        })
        let fresh = try await session.currentPlan()
        XCTAssertFalse(fresh.resultEgressGrants.isEmpty)
        XCTAssertEqual(Set(fresh.resultEgressGrants.map(\.expiresAtMs)), Set(plan.resultEgressGrants.map(\.expiresAtMs)))
        let expiredSession = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now + 61_000, isForeground: false, authorizations: plan.authorizationSnapshot)
        })
        let expired = try await expiredSession.currentPlan()
        XCTAssertTrue(expired.resultEgressGrants.isEmpty)
    }

    private var identity: AgentCoreExpectedIdentity {
        .init(hostId: "test-host", clientId: "test-client", sessionId: "test-session")
    }

    private actor ObservationGate {
        let latest: IOSRunToolPlanObservation
        var suspended: CheckedContinuation<IOSRunToolPlanObservation, Never>?
        var waiter: CheckedContinuation<Void, Never>?
        var started = false
        init(latest: IOSRunToolPlanObservation) { self.latest = latest }
        func observe() async -> IOSRunToolPlanObservation {
            guard !started else { return latest }
            started = true
            return await withCheckedContinuation { continuation in
                suspended = continuation
                waiter?.resume(); waiter = nil
            }
        }
        func waitUntilSuspended() async {
            if suspended != nil { return }
            await withCheckedContinuation { waiter = $0 }
        }
        func release(_ value: IOSRunToolPlanObservation) { suspended?.resume(returning: value); suspended = nil }
    }
}

final class IOSExactRunInvocationTests: XCTestCase {
    func testFrozenSkillsUseExactCheckpointInsteadOfChangedCurrentCatalog() throws {
        let fixture = Fixture()
        let original = IOSPreparedHostInvocation.skillIdentity([.init(id: "skill", name: "Original", summary: "summary",
            instructions: "frozen instructions", version: "1", requiredTools: [], validationSteps: [])])
        let current = IOSPreparedHostInvocation.skillIdentity([.init(id: "skill", name: "Changed", summary: "summary",
            instructions: "new instructions", version: "2", requiredTools: ["write_file"], validationSteps: [])])
        let body: AgentCoreJSONValue = .object(["session_id": .string("actual-core-session"),
            "state": .object(["configuration": .object(["turn_identity": .object(["host_dependencies": original])])])])
        let raw = String(decoding: try JSONEncoder().encode(body), as: UTF8.self)
        var fields = fixture.fields
        fields["checkpoint_sha256"] = .string(SHA256.hash(data: Data(raw.utf8)).map { String(format: "%02x", $0) }.joined())
        let frozen = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        let checkpoint: [String: AgentCoreJSONValue] = ["conversation_id": .string("conversation"),
            "revision": .number(12), "event_cursor": .number(34), "checkpoint_json": .string(raw)]
        let store = Store(response: .object(checkpoint))
        let recovered = try fixture.application(store).loadFrozenSkillIdentity(frozen)
        XCTAssertEqual(recovered, original)
        XCTAssertNotEqual(recovered, current)
        XCTAssertEqual(store.types, ["load_checkpoint"])
        for (key, altered): (String, AgentCoreJSONValue) in [
            ("conversation_id", .string("foreign")), ("revision", .number(13)),
            ("event_cursor", .number(35)), ("checkpoint_json", .string(raw + " ")),
        ] {
            var changed = checkpoint; changed[key] = altered
            XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadFrozenSkillIdentity(frozen))
        }
    }

    func testCoreCheckpointAdvancePreservesOnlyExactFrozenInvocation() throws {
        let fixture = Fixture()
        let original = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        var advanced = fixture.fields
        advanced["checkpoint_revision"] = .number(13)
        advanced["checkpoint_event_cursor"] = .number(40)
        advanced["checkpoint_sha256"] = .string(String(repeating: "c", count: 64))
        let resumed = try fixture.application(Store(response: .object(advanced))).loadExactRunInvocation()
        XCTAssertNotEqual(original, resumed)
        XCTAssertTrue(resumed.hasSameFrozenInvocation(as: original))
        advanced["host_invocation"] = .object(["version": .number(3), "tampered": .bool(true)])
        let changed = try fixture.application(Store(response: .object(advanced))).loadExactRunInvocation()
        XCTAssertFalse(changed.hasSameFrozenInvocation(as: original))
    }

    func testFrozenRemoteToolPlanIsRequiredAndParticipatesInAuthorityComparison() throws {
        let fixture = Fixture()
        let original = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        var changed = fixture.fields
        changed["remote_tool_plan"] = .object(["reference": .string("changed-plan")])
        let current = try fixture.application(Store(response: .object(changed))).loadExactRunInvocation()
        XCTAssertFalse(current.hasSameFrozenInvocation(as: original))
        changed.removeValue(forKey: "remote_tool_plan")
        XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadExactRunInvocation())
        changed["remote_tool_plan"] = .string("invalid-plan-shape")
        XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadExactRunInvocation())
    }

    func testRecoveryTraceReplayPinsFrontierAndFeedsNextIncrementWithoutGap() throws {
        let fixture = Fixture()
        func event(_ sequence: UInt64) -> AgentTraceEvent {
            AgentTraceEvent(traceId: "trace", runId: fixture.identity.submissionID,
                conversationId: "conversation", messageId: fixture.identity.assistantMessageID,
                turnId: fixture.identity.submissionID, sequence: sequence, kind: .toolProgress)
        }
        func page(_ sequence: UInt64) throws -> AgentCoreJSONValue {
            .object(["trace_id": .string("trace"), "through_sequence": .number(2),
                "next_sequence": .number(Double(sequence)),
                "events": .array([try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(event(sequence)))])])
        }
        let store = ReplayStore(pages: [try page(1), try page(2)])
        let replay = try fixture.application(store).recoveryTraceEvents(afterSequence: 0)
        var reducer = AgentTraceReducer()
        for trace in replay { XCTAssertEqual(reducer.apply(trace), .applied) }
        XCTAssertEqual(reducer.apply(event(3)), .applied)
        XCTAssertEqual(store.commands.count, 2)
        guard case .object(let second) = store.commands[1] else { return XCTFail("Missing replay request") }
        XCTAssertEqual(second["after_sequence"], .signedInteger(1))
        XCTAssertEqual(second["through_sequence"], .signedInteger(2))
        var foreign = event(1); foreign.runId = "foreign"
        let invalid = AgentCoreJSONValue.object(["trace_id": .string("trace"), "through_sequence": .number(1),
            "next_sequence": .number(1), "events": .array([
                try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(foreign))])])
        XCTAssertThrowsError(try fixture.application(ReplayStore(pages: [invalid])).recoveryTraceEvents(afterSequence: 0))
    }

    @MainActor
    func testRecordedColdCancellationTraceRestoresThroughActualTerminal() throws {
        let env = ProcessInfo.processInfo.environment
        guard let file = env["NOLA_RECORDED_TRACE_COPY"],
              let conversation = env["NOLA_RECORDED_TRACE_CONVERSATION"],
              let run = env["NOLA_RECORDED_TRACE_RUN"], let userID = UUID(uuidString: run) else {
            throw XCTSkip("Requires an isolated copy of a real cold-cancelled iOS Run")
        }
        let runtime = try IOSApplicationRuntime(databaseURL: FileManager.default.temporaryDirectory.appendingPathComponent(file))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
        let application = IOSCanonicalAgentApplication(store: runtime,
            descriptor: .init(conversationID: conversation, conversationTitle: "", identity: identity,
                userText: "", userContentJSON: nil, contextCheckpointID: nil))
        let events = try application.recoveryTraceEvents(afterSequence: 0)
        XCTAssertGreaterThan(events.count, 2)
        XCTAssertEqual(events.last?.kind, .runCancelled)
        var reducer = AgentTraceReducer()
        for event in events { XCTAssertEqual(reducer.apply(event), .applied) }
        XCTAssertEqual(reducer.projection.runId, run)
        XCTAssertEqual(reducer.projection.lastSequence, events.last?.sequence)
        XCTAssertEqual(reducer.apply(try XCTUnwrap(events.last)), .duplicate)
        XCTAssertTrue(try application.recoveryTraceEvents(afterSequence: Int64(reducer.projection.lastSequence)).isEmpty)
    }

    func testExactReadIsReadOnlyAndPreservesOpaqueBinding() throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        let result = try fixture.application(store).loadExactRunInvocation()
        XCTAssertEqual(result.runID, fixture.identity.submissionID)
        XCTAssertEqual(result.hostInvocation, .object(["version": .number(3)]))
        XCTAssertEqual(result.coreSessionID, "actual-core-session")
        XCTAssertEqual(result.checkpointRevision, 12)
        XCTAssertEqual(result.checkpointEventCursor, 34)
        XCTAssertEqual(result.checkpointSHA256, String(repeating: "b", count: 64))
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testRawReceiptIntegersRemainExactThroughFoundationAndOpaqueBinding() throws {
        let fixture = Fixture()
        // UInt64.max checks the generic JSON container, not valid i64 seed admission.
        let numbers: [(String, AgentCoreJSONValue)] = [
            ("9007199254740993", .signedInteger(9_007_199_254_740_993)),
            ("9223372036854775807", .signedInteger(Int64.max)),
            ("-9223372036854775808", .signedInteger(Int64.min)),
            ("18446744073709551615", .unsignedInteger(UInt64.max)),
        ]
        for (literal, expected) in numbers {
            // Mirrors transmitEnvelope -> receipt.result -> executeMutation decoding.
            // Do not manufacture a Double before decoding the raw Rust JSON integer.
            let json = """
            {"ok":true,"result":{"result":{
              "conversation_id":"conversation","run_id":"\(fixture.identity.submissionID)",
              "assistant_message_id":"\(fixture.identity.assistantMessageID)",
              "invocation_config_digest":"\(String(repeating: "a", count: 64))","status":"running",
              "core_session_id":null,"checkpoint_revision":null,"checkpoint_event_cursor":null,"checkpoint_sha256":null,
              "host_invocation":{"version":3,"generation_options":{"seed":\(literal)}},
              "remote_tool_plan":null,"context_budget":{"context_window_tokens":4096,"reserved_output_tokens":64,"prompt_token_limit":null,
              "auto_compact_tokens":null,"compaction_target_tokens":null,"auto_compact_percent":80,"compaction_target_percent":50}
            }}}
            """
            let envelope = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
            let receipt = try XCTUnwrap(envelope["result"] as? [String: Any])
            let rawResult = try XCTUnwrap(receipt["result"])
            let resultData = try JSONSerialization.data(withJSONObject: rawResult, options: [.fragmentsAllowed])
            let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: resultData)
            let frozen = try fixture.application(Store(response: response)).loadExactRunInvocation()
            guard case .object(let binding) = frozen.hostInvocation,
                  case .object(let options)? = binding["generation_options"] else {
                return XCTFail("Opaque binding structure changed")
            }
            XCTAssertEqual(options["seed"], expected)
            let encoded = try JSONEncoder().encode(frozen.hostInvocation)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: encoded), frozen.hostInvocation)
            XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains(literal))
        }
    }

    func testAdmissionWithoutCheckpointUsesFourExplicitNullWitnessFields() throws {
        let fixture = Fixture()
        var fields = fixture.fields
        for key in ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"] {
            fields[key] = .null
        }
        let result = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertNil(result.coreSessionID)
        XCTAssertNil(result.checkpointRevision)
        XCTAssertNil(result.checkpointEventCursor)
        XCTAssertNil(result.checkpointSHA256)
    }

    func testCheckpointWitnessIsStrictCompleteAndLossless() throws {
        let fixture = Fixture()
        let keys = ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"]
        for key in keys {
            var missing = fixture.fields; missing.removeValue(forKey: key)
            XCTAssertThrowsError(try fixture.application(Store(response: .object(missing))).loadExactRunInvocation())
            var partial = fixture.fields; partial[key] = .null
            XCTAssertThrowsError(try fixture.application(Store(response: .object(partial))).loadExactRunInvocation())
        }
        let invalid: [(String, AgentCoreJSONValue)] = [
            ("core_session_id", .string(" ")), ("checkpoint_sha256", .string(String(repeating: "B", count: 64))),
            ("checkpoint_revision", .number(-1)), ("checkpoint_event_cursor", .number(1.5)),
            ("checkpoint_event_cursor", .unsignedInteger(UInt64.max)), ("checkpoint_revision", .string("12")),
        ]
        for (key, value) in invalid {
            var fields = fixture.fields; fields[key] = value
            XCTAssertThrowsError(try fixture.application(Store(response: .object(fields))).loadExactRunInvocation())
        }
        var fields = fixture.fields
        fields["checkpoint_revision"] = .signedInteger(Int64.max)
        fields["checkpoint_event_cursor"] = .signedInteger(9_007_199_254_740_993)
        let exact = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertEqual(exact.checkpointRevision, Int64.max)
        XCTAssertEqual(exact.checkpointEventCursor, 9_007_199_254_740_993)
        fields["checkpoint_sha256"] = .string(String(repeating: "c", count: 64))
        let changed = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertNotEqual(changed, exact, "Recovery preparation must detect a changed checkpoint witness")
    }

    func testRejectsMissingMalformedAndWrongIdentityResponsesWithoutClaim() throws {
        let fixture = Fixture()
        var samples: [AgentCoreJSONValue] = [.null, .object([:])]
        for key in ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest", "status", "host_invocation", "context_budget"] {
            var value = fixture.fields
            value[key] = .null
            samples.append(.object(value))
        }
        for (key, replacement) in [("conversation_id", "other"), ("run_id", "other"),
            ("assistant_message_id", "other"), ("invocation_config_digest", String(repeating: "A", count: 64)),
            ("invocation_config_digest", "short")] {
            var value = fixture.fields
            value[key] = .string(replacement)
            samples.append(.object(value))
        }
        for version in [0.0, -1.0, 1.5, Double(UInt32.max) + 1, Double.infinity] {
            var value = fixture.fields
            value["host_invocation"] = .object(["version": .number(version)])
            samples.append(.object(value))
        }
        for response in samples {
            let store = Store(response: response)
            XCTAssertThrowsError(try fixture.application(store).loadExactRunInvocation())
            XCTAssertEqual(store.types, ["load_exact_run_invocation"])
        }
    }

    func testMissingPreparationCannotClaimEvenWithValidBindingEnvelope() async throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        do {
            _ = try await fixture.application(store).recoverPendingPermissionWait()
            XCTFail("Opaque binding alone must not authorize recovery")
        } catch {}
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testPreparationFailureOccursBeforeAnyClaimOrScheduleMutation() async throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        do {
            _ = try await fixture.application(store).recoverPendingPermissionWait(prepareRecovery: { frozen in
                XCTAssertEqual(frozen.runID, fixture.identity.submissionID)
                XCTAssertEqual(store.types, ["load_exact_run_invocation"])
                throw PreparationError.revoked
            })
            XCTFail("Revoked authorization must block recovery")
        } catch PreparationError.revoked {} catch { XCTFail("Unexpected error: \(error)") }
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testRecoveryAuthorityRequiresExactCoreProofWithoutClaimingRun() throws {
        let fixture = Fixture()
        let frozen = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        let candidate: AgentCoreJSONValue = .object([
            "host_catalog": .object([:]), "tools": .array([]), "permission_rules": .array([]),
            "context_window": .object([:]), "turn_identity": .object(["host_invocation": frozen.hostInvocation]),
        ])
        let proof = fixture.fields.filter { ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest"].contains($0.key) }
        let store = Store(response: .object(proof))
        try fixture.application(store).validateRecoveryAuthority(frozen, candidate: candidate)
        XCTAssertEqual(store.types, ["validate_mobile_run_recovery_authority"])
        for key in proof.keys {
            var foreign = proof; foreign[key] = .string("foreign")
            let rejected = Store(response: .object(foreign))
            XCTAssertThrowsError(try fixture.application(rejected).validateRecoveryAuthority(frozen, candidate: candidate))
            XCTAssertEqual(rejected.types, ["validate_mobile_run_recovery_authority"])
        }
        let malformed = Store(response: .object(proof))
        XCTAssertThrowsError(try fixture.application(malformed).validateRecoveryAuthority(frozen, candidate: .object([:])))
        XCTAssertTrue(malformed.types.isEmpty)
    }

    func testTerminalOwnerUsesCommittedProofAndRejectsMissingOrForeignOwner() throws {
        let message = IOSCanonicalConversationMessage(id: "visible", runID: "run", role: "assistant",
            status: "completed", ordinal: 1, createdAtMs: 1, completedAtMs: 2, parts: [])
        let valid: AgentCoreJSONValue = .object(["conversation_id": .string("conversation"),
            "run_id": .string("run"), "assistant_message_id": .string("root-assistant")])
        let store = OwnerStore(valid)
        let owner = try store.confirmedTerminalOwner(conversationID: "conversation", observedMessage: message)
        XCTAssertEqual(owner?.assistant_message_id, "root-assistant")
        XCTAssertEqual(store.command, .object(["type": .string("canonical_run_terminal"),
            "conversation_id": .string("conversation"), "run_id": .string("run")]))
        XCTAssertThrowsError(try OwnerStore(.null).confirmedTerminalOwner(conversationID: "conversation", observedMessage: message))
        guard case .object(var foreign) = valid else { return XCTFail() }
        foreign["conversation_id"] = .string("foreign")
        XCTAssertThrowsError(try OwnerStore(.object(foreign)).confirmedTerminalOwner(conversationID: "conversation", observedMessage: message))
        let empty = OwnerStore(.null)
        XCTAssertNil(try empty.confirmedTerminalOwner(conversationID: "new", observedMessage: nil))
        XCTAssertNil(empty.command)
    }

    private nonisolated final class OwnerStore: ConversationRemoteTurnStore, @unchecked Sendable {
        let result: AgentCoreJSONValue
        var command: AgentCoreJSONValue?
        init(_ result: AgentCoreJSONValue) { self.result = result }
        func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws {}
        func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            self.command = command
            return result
        }
    }

    private enum PreparationError: Error { case revoked }

    private struct Fixture: Sendable {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        var fields: [String: AgentCoreJSONValue] { [
            "conversation_id": .string("conversation"), "run_id": .string(identity.submissionID),
            "assistant_message_id": .string(identity.assistantMessageID),
            "invocation_config_digest": .string(String(repeating: "a", count: 64)),
            "core_session_id": .string("actual-core-session"), "checkpoint_revision": .number(12),
            "checkpoint_event_cursor": .number(34), "checkpoint_sha256": .string(String(repeating: "b", count: 64)),
            "status": .string("running"), "host_invocation": .object(["version": .number(3)]),
            "remote_tool_plan": .null,
            "context_budget": .object(["context_window_tokens": .number(4096), "reserved_output_tokens": .number(64),
                "prompt_token_limit": .null, "auto_compact_tokens": .null, "compaction_target_tokens": .null,
                "auto_compact_percent": .number(80), "compaction_target_percent": .number(50)]),
        ] }
        var response: AgentCoreJSONValue { .object(fields) }
        func application(_ store: any IOSAgentApplicationCommandStore) -> IOSCanonicalAgentApplication {
            IOSCanonicalAgentApplication(store: store, descriptor: .init(conversationID: "conversation",
                conversationTitle: "Test", identity: identity, userText: "", userContentJSON: nil,
                contextCheckpointID: nil))
        }
    }

    private final class ReplayStore: IOSAgentApplicationCommandStore, @unchecked Sendable {
        private var pages: [AgentCoreJSONValue]
        private(set) var commands: [AgentCoreJSONValue] = []
        init(pages: [AgentCoreJSONValue]) { self.pages = pages }
        func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            commands.append(command)
            guard !pages.isEmpty else { throw PreparationError.revoked }
            return pages.removeFirst()
        }
    }

    private final class Store: IOSAgentApplicationCommandStore, @unchecked Sendable {
        let response: AgentCoreJSONValue
        private let lock = NSLock()
        private var recorded: [String] = []
        var types: [String] { lock.lock(); defer { lock.unlock() }; return recorded }
        init(response: AgentCoreJSONValue) { self.response = response }
        func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            lock.lock(); defer { lock.unlock() }
            guard case .object(let value) = command, case .string(let type)? = value["type"] else {
                throw PreparationError.revoked
            }
            recorded.append(type)
            return response
        }
    }
}

@MainActor
struct IOSLocalRunConfigurationTests {
    private let model = LMModel(name: "fixture/local", configuration: ModelConfiguration(id: "fixture/local"), type: .llm)

    @Test
    func runOwnsSessionValuesAcrossLaterDefaultChangesAndCalls() async throws {
        var defaults = ModelParameters.localDefaults
        var session = IOSConversationGenerationOverride(maxContextLength: 4096, maxOutputTokens: 64)
            .applying(to: defaults)
        session.temperature = 0
        session.topP = 0
        session.topK = 1
        session.presencePenalty = -0.5
        session.frequencyPenalty = 0.75
        session.repetitionPenalty = 1.2
        session.seed = 0
        session.showThinking = false
        var requests: [LocalProviderGenerationRequest] = []
        let provider = try IOSConversationRunConfiguration.localProvider(
            model: identity, parameters: session, availableModels: [model], generation: { request in
                requests.append(request)
                return AsyncThrowingStream { $0.yield("answer"); $0.finish() }
            })
        _ = try await provider.chat(message: "first", systemPrompt: "system", enableThinking: true, timeout: nil)
        #expect(provider.agentCapabilities == .init(maxOutputTokens: 64, reasoning: false))
        defaults.temperature = 1.8
        defaults.maxOutputTokens = 64000
        session = defaults
        let laterProvider = try IOSConversationRunConfiguration.localProvider(
            model: identity, parameters: defaults, availableModels: [model], generation: { request in
                requests.append(request)
                return AsyncThrowingStream { $0.yield("later"); $0.finish() }
            })
        _ = try await laterProvider.chat(message: "later", systemPrompt: "system", enableThinking: true, timeout: nil)
        _ = try await provider.chat(message: "second", systemPrompt: "system", enableThinking: true, timeout: nil)
        #expect(provider.agentCapabilities.maxOutputTokens == 64)
        #expect(!provider.agentCapabilities.nativeToolCalls)
        #expect(requests.count == 3)
        for request in [requests[0], requests[2]] {
            #expect(request.model.name == model.name)
            #expect(request.parameters.maxOutputTokens == 64)
            #expect(request.parameters.maxContextLength == 4096)
            #expect(request.parameters.temperature == 0)
            #expect(request.parameters.topP == 0)
            #expect(request.parameters.topK == 1)
            #expect(request.parameters.presencePenalty == -0.5)
            #expect(request.parameters.frequencyPenalty == 0.75)
            #expect(request.parameters.repetitionPenalty == 1.2)
            #expect(request.parameters.seed == 0)
            #expect(!request.enableThinking)
        }
        #expect(requests[1].parameters.temperature == 1.8)
        #expect(requests[1].parameters.maxOutputTokens == 64000)
    }

    @Test
    func invalidOrUnsupportedValuesRejectBeforeGeneration() throws {
        let mutations: [(inout ModelParameters) -> Void] = [
            { $0.maxOutputTokens = 0 }, { $0.maxContextLength = 0 },
            { $0.temperature = .nan }, { $0.topP = 1.1 }, { $0.topK = 0 },
            { $0.presencePenalty = 3 }, { $0.frequencyPenalty = -.infinity },
            { $0.repetitionPenalty = 0 }, { $0.stopSequences = "STOP" }
        ]
        for mutate in mutations {
            var parameters = ModelParameters.localDefaults
            mutate(&parameters)
            #expect(throws: Error.self) {
                try IOSConversationRunConfiguration.localProvider(model: identity, parameters: parameters,
                    availableModels: [model], generation: { _ in
                        Issue.record("Invalid configuration reached generation")
                        return AsyncThrowingStream { $0.finish() }
                    })
            }
        }
    }

    @Test
    func unavailableModelDoesNotFallBackToAnotherLoadedModel() throws {
        #expect(throws: Error.self) {
            try IOSConversationRunConfiguration.localProvider(
                model: ModelInfo(id: "absent", name: "Absent", source: "Local", isLocal: true),
                parameters: .localDefaults, availableModels: [model])
        }
    }

    @Test
    func catalogNameAliasKeepsActualTargetAndFrozenGenerationTogether() async throws {
        let alias = ModelDownloader.ModelType.qwen3_5_0_8b
        let target = LMModel(name: try #require(alias.mlxModelName),
            configuration: ModelConfiguration(id: "fixture/actual-catalog-target"), type: .llm)
        var parameters = ModelParameters.localDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        var observed: LocalProviderGenerationRequest?
        let provider = try IOSConversationRunConfiguration.localProvider(
            model: ModelInfo(id: alias.rawValue, name: "UI alias", source: "Local", isLocal: true),
            parameters: parameters, availableModels: [model, target], generation: { request in
                observed = request
                return AsyncThrowingStream { $0.yield("fixture"); $0.finish() }
            })
        parameters.maxOutputTokens = 64000
        _ = try await provider.chat(message: "fixture", systemPrompt: "fixture", enableThinking: true, timeout: nil)
        let request = try #require(observed)
        #expect(provider.resolvedModelIdentity.id == target.id)
        #expect(provider.resolvedModelIdentity.name == target.name)
        #expect(provider.resolvedModelIdentity.id == request.model.id)
        #expect(provider.resolvedModelIdentity.name == request.model.name)
        #expect(request.parameters.maxOutputTokens == 64)
        #expect(request.parameters.temperature == 0)
    }

    private var identity: ModelInfo {
        ModelInfo(id: model.name, name: "Fixture", source: "Local", isLocal: true)
    }
}

@MainActor
struct IOSPreparedRunModelTests {
    @Test
    func frozenSkillsRoundTripWithoutCurrentCatalogFallback() throws {
        let original = IOSAgentSkillSnapshot(id: "frozen", name: "Frozen", summary: "summary",
            instructions: "original instructions", version: "1", requiredTools: ["read_file"], validationSteps: ["verify"])
        let encoded = IOSPreparedHostInvocation.skillIdentity([original])
        let decoded = try IOSPreparedHostInvocation.decodeSkillIdentity(encoded)
        #expect(IOSPreparedHostInvocation.skillIdentity(decoded) == encoded)
        #expect(try IOSPreparedHostInvocation.decodeSkillIdentity(.array([])).isEmpty)
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.null) }
        guard case .array(let values) = encoded, case .object(var fields) = values[0] else {
            Issue.record("Missing frozen skill fixture")
            return
        }
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.array(values + values)) }
        fields.removeValue(forKey: "instructions")
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.array([.object(fields)])) }
    }

    @Test
    func oneResolutionOwnsClientModelCapabilityAndParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        for (tier, effort) in [("concise", "low"), ("balanced", "low"), ("detailed", "high")] {
            let resolved = try runtime.executeAgentCommand(.object([
                "type": .string("resolve_model_reasoning"),
                "declaration": .object(["control": .string("effort"), "efforts": .array([.string("low"), .string("high")])]),
                "protocol": .string("siwc_responses"), "output_limit": .number(16384),
                "supports_reasoning": .bool(true), "requested": .null, "tier": .string(tier)
            ]))
            #expect(resolved.objectFields?["selection"] == .object(["kind": .string("effort"), "value": .string(effort)]))
        }
        let selected = ModelInfo(id: "profile", name: "Selected", source: "api", isLocal: false)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        parameters.topP = 0
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  preserve space  \nsecond"
        var resolutions = 0
        let prepared = try IOSPreparedRunModel.prepare(model: selected, parameters: parameters, resolveCloud: { input in
            resolutions += 1
            #expect(input.id == "profile")
            return IOSResolvedCloudRunProfile(model: ModelInfo(id: input.id, name: "Resolved", source: "api", isLocal: false,
                baseURL: "https://fixture.invalid/v1", apiKey: "test-only",
                upstreamModelId: "resolved-model", wireProtocol: .chatCompletions,
                executionCapabilities: .init(toolCalls: true, reasoning: true, samplingParameters: true)), profileID: input.id, revision: 7,
                parametersJSON: "{}", reasoningSummarySupported: false,
                reasoningSelection: .object(["kind": .string("effort"), "value": .string("high")]), reasoningTier: "detailed")
        }, admitOutput: { route, tokens in
            try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: tokens, using: runtime)
        })
        parameters.maxOutputTokens = 64000
        #expect(resolutions == 1)
        #expect(prepared.client is OpenAIProvider)
        #expect(prepared.modelName == "resolved-model")
        #expect(prepared.supportsToolCalls)
        #expect(prepared.parameters.maxOutputTokens == 64)
        #expect(prepared.parameters.temperature == 0)
        #expect(prepared.cloudProfile?.profileID == "profile")
        #expect(prepared.cloudProfile?.revision == 7)
        guard case .http(let configuration, let source) = prepared.executionSource else {
            Issue.record("Expected the exact admitted HTTP configuration")
            return
        }
        #expect(configuration.model == prepared.modelName)
        #expect(configuration.baseURL.absoluteString == "https://fixture.invalid/v1")
        #expect(configuration.apiKey == "test-only")
        #expect(configuration.wireProtocol == .chatCompletions)
        #expect(configuration.executionCapabilities.toolCalls == prepared.supportsToolCalls)
        #expect(configuration.maxCompletionTokens == 64)
        #expect(configuration.temperature == 0)
        #expect(configuration.topP == 0)
        #expect(configuration.reasoningSummarySupported == false)
        #expect(configuration.seed == 9_007_199_254_740_993)
        #expect(configuration.stopSequences == ["  preserve space  ", "second"])
        #expect(source.profileID == "profile")
        #expect(source.revision == 7)
        #expect(source.parametersJSON == "{}", "Profile defaults are provenance, not a replacement for frozen session parameters")
        guard case .object(let admission) = prepared.outputAdmission,
              case .object(let output)? = admission["resolved"] else {
            Issue.record("Expected shared output admission")
            return
        }
        #expect(output["tokens"] == .number(64))
        let captured = try prepared.prepareHostInvocation()
        let policy: AgentCoreJSONValue = .object(["permissions": .array([]), "host": .string("ios")])
        let binding = try captured.binding(executionPolicy: policy)
        guard case .object(let fields) = binding,
              case .object(let generation)? = fields["generation"],
              case .object(let options)? = generation["generation_options"],
              case .object(let route)? = fields["primary"] else {
            Issue.record("Missing admitted Host binding")
            return
        }
        #expect(fields["version"] == .number(1))
        #expect(route["siwc_account"] == .null)
        #expect(fields["output_admission"] == prepared.outputAdmission)
        #expect(route["model"] == .string(configuration.model))
        #expect(generation["max_output_tokens"] == .number(64))
        #expect(generation["temperature"] == .number(0))
        #expect(generation["reasoning_tier"] == .string("detailed"))
        #expect(generation["reasoning_selection"] == .object(["kind": .string("effort"), "value": .string("high")]))
        #expect(generation["reasoning_summary_supported"] == .bool(false))
        var unknownConfiguration = configuration
        unknownConfiguration.reasoningSummarySupported = nil
        let unknown = IOSPreparedRunModel(client: OpenAIProvider(config: unknownConfiguration),
            parameters: prepared.parameters, modelName: prepared.modelName, supportsToolCalls: prepared.supportsToolCalls,
            executionSource: .http(configuration: unknownConfiguration, profile: source), outputAdmission: prepared.outputAdmission)
        #expect(try unknown.prepareHostInvocation().generation.objectFields?["reasoning_summary_supported"] == .null)
        #expect(options["seed"] == .signedInteger(9_007_199_254_740_993))
        #expect(options["stop"] == .array([.string("  preserve space  "), .string("second")]))
        let encoded = String(decoding: try JSONEncoder().encode(binding), as: UTF8.self)
        #expect(!encoded.contains("test-only"), "Durable binding must not contain credentials")
        #expect(try captured.binding(executionPolicy: .object(["host": .string("ios"), "permissions": .array([])])) == binding)
        #expect(try captured.binding(executionPolicy: .object(["host": .string("foreign")])) != binding)
    }

    @Test
    func preparationRejectsAdmissionForAnotherExecution() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        let model = ModelInfo(id: "profile", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "fixture-key", wireProtocol: .chatCompletions)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        for replaceRoute in [false, true] {
            #expect(throws: Error.self) {
                try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
                    resolveCloud: { _ in .init(model: model, profileID: model.id, revision: 1, parametersJSON: "{}") },
                    admitOutput: { route, tokens in
                        var admittedRoute = route
                        if replaceRoute, case .object(var fields) = route {
                            fields["model"] = .string("another-model")
                            admittedRoute = .object(fields)
                        }
                        return try IOSPreparedHostOutputAdmission.admit(route: admittedRoute,
                            outputTokens: replaceRoute ? tokens : 128, using: runtime)
                    })
            }
        }
    }

    @Test
    func preparationRejectsForeignOrUncommittedSourceIdentity() throws {
        let model = ModelInfo(id: "profile", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "test-only", wireProtocol: .chatCompletions)
        for (id, revision) in [("other-profile", Int64(1)), ("profile", Int64(0))] {
            #expect(throws: Error.self) {
                try IOSPreparedRunModel.prepare(model: model, parameters: .webAlignedCloudDefaults,
                    resolveCloud: { _ in .init(model: model, profileID: id, revision: revision, parametersJSON: "{}") })
            }
        }
    }

    @Test
    func rejectedPreparationNeverFallsBackToSharedClient() throws {
        enum Expected: Error { case unavailable }
        let model = ModelInfo(id: "profile", name: "Selected", source: "api", isLocal: false)
        #expect(throws: Expected.self) {
            try IOSPreparedRunModel.prepare(model: model, parameters: .webAlignedCloudDefaults,
                resolveCloud: { _ in throw Expected.unavailable })
        }
        var invalid = ModelParameters.webAlignedCloudDefaults
        invalid.maxOutputTokens = 0
        #expect(throws: Error.self) {
            try IOSPreparedRunModel.prepare(model: model, parameters: invalid, resolveCloud: { _ in
                Issue.record("Invalid parameters must fail before resolving a provider")
                throw Expected.unavailable
            })
        }
    }
}

final class IOSDeviceAgentToolPlanLifecycleTests: XCTestCase {
    func testChildPermissionPresentationIdentityIncludesExactOwner() {
        func request(child: String, run: String) -> IOSAgentPermissionRequest {
            .init(coreRequestID: "same-core-request", callID: "call", capabilityID: "internal.capability",
                allowConversation: false, rationale: "Review", scope: nil, instructions: [],
                childOwner: .init(taskID: "task", parentConversationID: "parent",
                    childConversationID: child, runID: run, coreSessionID: "core-" + child))
        }
        let first = request(child: "child-a", run: "run-a")
        XCTAssertEqual(first.requestId, request(child: "child-b", run: "run-b").requestId)
        XCTAssertNotEqual(first.presentationID, request(child: "child-b", run: "run-b").presentationID)
        XCTAssertNotEqual(first.presentationID, request(child: "child-a", run: "run-new").presentationID)
        XCTAssertEqual(first.presentationID, request(child: "child-a", run: "run-a").presentationID)
        XCTAssertTrue(first.presentationRationale.contains("task"))
    }

    @MainActor
    func testCorePermissionWithoutOSDomainStillRequiresExplicitUserDecision() async {
        let request = IOSAgentPermissionRequest(coreRequestID: "request", callID: "call",
            capabilityID: "internal.capability", allowConversation: false, rationale: "Review",
            scope: nil, instructions: [], childOwner: nil)
        let broker = IOSAgentPermissionBroker()
        let denied = await broker.resolve(request, userApproved: false)
        let approved = await broker.resolve(request, userApproved: true)
        XCTAssertEqual(denied, .decision(requestId: "request", decision: .deny))
        XCTAssertEqual(approved, .decision(requestId: "request", decision: .allowOnce))
    }

    @MainActor
    func testIndependentChildWaitsForCommittedStartBeforeInvokingModel() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("child-barrier.db"))
        let identity = AgentCoreExpectedIdentity(hostId: "ios-local", clientId: "test", sessionId: "child")
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, allowedTools: [])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "child", conversationTitle: "", identity: try .init(
                recoveringSubmissionID: "run", assistantMessageID: "assistant"),
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "run"))
        let model = ToolPlanLifecycleModel(toolName: "device_read_file")
        let executor = DeviceAgentCoordinator(modelClient: model, childOnly: true,
            childPermissionOwner: .init(taskID: "task", parentConversationID: "parent", childConversationID: "child", runID: "run", coreSessionID: "child"),
            toolPlan: plan, canonicalApplication: application)
        // An intentionally unadmitted transport input must never reach a model
        // when the Application rejects its execution-start barrier.
        let request: AgentCoreJSONValue = .object([
            "effect_id": .string("effect"), "turn_id": .string("run"),
            "session_id": .string("child"), "delivery": .object([:]),
            "payload": .object(["type": .string("model_stream")]),
        ])
        for foreign in [("other-child", "run", "child"), ("child", "other-run", "child"), ("child", "run", "other-core-session")] {
            do {
                try await executor.executeChildHostRequest(request, conversationID: foreign.0, runID: foreign.1, coreSessionID: foreign.2,
                    config: .init(model: "fixture"), stopTarget: { _ in XCTFail("Foreign owner cannot stop anything") }) { _ in
                        XCTFail("Foreign owner must fail before execution-start or permission response")
                    }
                XCTFail("Foreign child ownership must be rejected")
            } catch DeviceAgentCoordinatorError.invalidTransition { }
        }
        do {
            try await executor.executeChildHostRequest(request, conversationID: "child", runID: "run", coreSessionID: "child",
                config: .init(model: "fixture"), stopTarget: { _ in XCTFail("No cancellation was admitted") }) { response in
                    XCTAssertEqual(response.objectFields?["payload"]?.objectFields?["type"], .string("execution_started"))
                    throw IOSApplicationRuntimeError.rejected("Unadmitted request")
                }
            XCTFail("A rejected start must remain rejected")
        } catch IOSApplicationRuntimeError.rejected(let message) { XCTAssertEqual(message, "Unadmitted request") }
        XCTAssertEqual(model.callCount, 0)
    }

    func testUIPublicationRejectionIsRecoverableButUnknownOutcomePropagates() throws {
        let guidance = #"{"code":"revision_conflict","expected":1,"current":2}"#
        let response = try DeviceAgentCoordinator.uiPublicationToolResponse(callID: "call") {
            throw IOSApplicationRuntimeError.rejected(guidance)
        }
        guard case .object(let fields) = response,
              case .object(let output)? = fields["output"],
              case .object(let execution)? = output["execution"],
              case .object(let data)? = output["data"] else { return XCTFail("Expected typed tool response") }
        XCTAssertEqual(fields["outcome"], .string("failed"))
        XCTAssertEqual(execution["state"], .string("not_started"))
        XCTAssertEqual(data["error"], .string(guidance))
        for error: Error in [IOSApplicationRuntimeError.outcomePending("receipt"), IOSApplicationRuntimeError.invalidResponse, CancellationError(), IOSApplicationRuntimeError.revisionConflict(scope: "store", expected: 1, actual: 2)] {
            XCTAssertThrowsError(try DeviceAgentCoordinator.uiPublicationToolResponse(callID: "call") { throw error })
        }
    }

    func testCancellationAdmissionIsRequiredExactAndDoesNotTurnColdAbsenceIntoStop() throws {
        let proof: [String: AgentCoreJSONValue] = [
            "conversation_id": .string("conversation"), "run_id": .string("run"),
            "core_session_id": .string("core"), "host_id": .string("ios-local"),
            "cancellation_effect_id": .string("cancel"), "cancellation_request_id": .string("request"),
            "target_effect_id": .string("target"), "target_request_id": .string("target-request"),
            "target_payload_identity": .string(String(repeating: "a", count: 64)),
            "checkpoint_revision": .number(3), "revoked_attempts": .array([
                .object(["attempt_id": .string("revoked"), "revision": .number(2)])]),
        ]
        var envelope: [String: AgentCoreJSONValue] = [
            "session_id": .string("core"), "turn_id": .string("run"), "host_id": .string("ios-local"),
            "effect_id": .string("cancel"), "request_id": .string("request"),
            "payload": .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                  "target_not_started": .object(proof)]),
        ]
        func confirm(_ value: [String: AgentCoreJSONValue], local: Set<String> = [], unknown: Set<String> = []) throws -> String {
            try IOSCancellationAdmissionContract.confirmableTarget(envelope: value, conversationID: "conversation",
                locallyQuiescent: local, unconfirmed: unknown)
        }
        XCTAssertEqual(try confirm(envelope), "target")
        XCTAssertThrowsError(try confirm(envelope, unknown: ["target"]), "a contradictory physical uncertainty cannot be overridden")
        for key in ["conversation_id", "run_id", "core_session_id", "host_id", "cancellation_effect_id",
                    "cancellation_request_id", "target_effect_id", "target_payload_identity"] {
            var foreign = proof
            foreign[key] = .string("foreign")
            var request = envelope
            request["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                          "target_not_started": .object(foreign)])
            XCTAssertThrowsError(try confirm(request), key)
        }
        envelope["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                        "target_not_started": .null])
        XCTAssertThrowsError(try confirm(envelope), "a cold empty registry is not proof")
        XCTAssertEqual(try confirm(envelope, local: ["target"]), "target")
        XCTAssertThrowsError(try confirm(envelope, local: ["other"]), "another effect cannot stop this target")
        envelope["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target")])
        XCTAssertThrowsError(try confirm(envelope, local: ["target"]), "missing required nullable field is rejected")
    }

    @MainActor
    func testCancellationDuringInitialObservationDoesNotBeginCanonicalRun() async throws {
        try await exerciseCancellation(suspendObservation: 1)
    }

    @MainActor
    func testCancellationDuringPermissionObservationDoesNotReviveFilePermission() async throws {
        try await exerciseCancellation(suspendObservation: 2, permissionRequired: true)
    }

    @MainActor
    func testCancellationDuringOrdinaryReadObservationDoesNotExecuteUnscopedFile() async throws {
        try await exerciseCancellation(suspendObservation: 2)
    }

    @MainActor
    func testRejectedSecondStreamDoesNotCancelOriginalOwner() async throws {
        try await exerciseCancellation(suspendObservation: 2, permissionRequired: true, rejectSecondStream: true)
    }

    @MainActor
    func testCancellingUnconfirmedPhysicalModelDoesNotInventTerminalSealOrReplay() async throws {
        try await exercisePhysicalStreamTermination(userStop: true)
    }

    @MainActor
    func testConsumerDetachDoesNotPersistBusinessCancellationOrReplay() async throws {
        try await exercisePhysicalStreamTermination(userStop: false)
    }

    @MainActor
    func testNativeStopPersistsExactInterruptBeforeRetiringUIObservation() async throws {
        try await exercisePhysicalStreamTermination(userStop: true, retireObserverAfterInterrupt: true)
    }

    @MainActor
    func testDelayedNativeStopNeverRetiresReplacementUIOwner() async throws {
        try await exercisePhysicalStreamTermination(userStop: true, retireObserverAfterInterrupt: true, changeOwnerDuringCancel: true)
    }

    @MainActor
    private func exercisePhysicalStreamTermination(userStop: Bool, retireObserverAfterInterrupt: Bool = false,
                                                   changeOwnerDuringCancel: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sessionID = UUID().uuidString
        let expectedIdentity = AgentCoreExpectedIdentity(hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: sessionID)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: expectedIdentity, isForeground: true, allowedTools: [],
            resultEgressAuthorization: .explicitUserApproval(identity: expectedIdentity, userApproved: true))
        let toolSession = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000),
                isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("physical.db"))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let application = IOSCanonicalAgentApplication(store: runtime, descriptor: .init(
            conversationID: sessionID, conversationTitle: "Unconfirmed physical stop", identity: identity,
            userText: "Hold the physical model", userContentJSON: nil, contextCheckpointID: nil))
        let dispatched = expectation(description: "Physical model stream was dispatched")
        let drained = expectation(description: "Cancelled model observation ended")
        let detached = userStop ? nil : expectation(description: "Physical stream observation detached")
        let model = ToolPlanLifecycleModel(toolName: "device_read_file", heldModelStarted: dispatched,
            heldModelDetached: detached)
        defer { model.releaseHeldModel() }
        let coordinator = DeviceAgentCoordinator(modelClient: model, toolPlan: plan,
            canonicalApplication: application, toolPlanSession: toolSession,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: runtime, model: "fixture",
                outputTokens: 1_024, capabilities: model.coreModelCapabilityProfile))
        let stream = await coordinator.start(sessionId: sessionID, input: "Hold the physical model", history: [],
            turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                permissionProfile: .cautious))
        let consumer = Task {
            do {
                for try await event in stream {
                    if case .completed = event { XCTFail("Unconfirmed stop cannot publish a completed Run") }
                }
            } catch is CancellationError {} catch { XCTFail("Unexpected observation error: \(error)") }
            drained.fulfill()
        }
        await fulfillment(of: [dispatched], timeout: 5)
        if retireObserverAfterInterrupt {
            let ignored = await ChatManager.cancelNativeBeforeRetiringObservation(coordinator,
                isCurrent: { false }, retireObservation: { XCTFail("A stale owner must not retire current observation") })
            XCTAssertFalse(ignored)
            XCTAssertEqual(model.callCount, 1)
            var ownerChecks = 0
            let retired = await ChatManager.cancelNativeBeforeRetiringObservation(coordinator,
                isCurrent: { ownerChecks += 1; return !changeOwnerDuringCancel || ownerChecks == 1 }, retireObservation: {
                    XCTAssertFalse(changeOwnerDuringCancel, "A resumed cancellation cannot clear replacement UI state")
                    do {
                        let pending = try runtime.executeAgentCommand(.object([
                            "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)]))
                        guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
                            return XCTFail("Missing canonical cancellation effects before observer retirement")
                        }
                        XCTAssertTrue(requests.contains {
                            guard case .object(let envelope) = $0, case .object(let payload)? = envelope["payload"] else { return false }
                            return payload["type"] == .string("cancel")
                        }, "The real Store must contain the interrupt before stream cancellation can clear its permit")
                    } catch { XCTFail("Cancellation persistence could not be verified: \(error)") }
                    consumer.cancel()
                })
            XCTAssertEqual(retired, !changeOwnerDuringCancel)
        } else if userStop { await coordinator.cancel() } else { consumer.cancel() }
        await fulfillment(of: [drained], timeout: 5)
        if userStop {
            do {
                try await coordinator.waitForCancellation()
                XCTFail("A stopped consumer is not proof that the physical provider stopped")
            } catch DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed {}
        } else {
            await fulfillment(of: [try XCTUnwrap(detached)], timeout: 5)
            try await coordinator.waitForCancellation()
        }
        await consumer.value
        XCTAssertNil(try application.committedTerminalSeal(coreSessionID: sessionID))
        XCTAssertThrowsError(try ChatManager.validateLocalRunAdmission(runtime: runtime, conversationID: sessionID)) { error in
            XCTAssertTrue(error is IOSLocalRunAdmissionError)
        }
        let pending = try runtime.executeAgentCommand(.object([
            "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)
        ]))
        guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
            return XCTFail("Missing durable cancellation control")
        }
        XCTAssertEqual(requests.contains { request in
            guard case .object(let envelope) = request, case .object(let payload)? = envelope["payload"] else { return false }
            return payload["type"] == .string("cancel")
        }, userStop, "Only explicit user Stop may persist a cancellation control")
        XCTAssertEqual(model.callCount, 1, "Cancellation cleanup cannot replay the physical model")
    }

    @MainActor
    private func exerciseCancellation(suspendObservation: Int, permissionRequired: Bool = false,
                                      rejectSecondStream: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sessionID = UUID().uuidString
        let expectedIdentity = AgentCoreExpectedIdentity(hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: sessionID)
        // A plain read no longer asks for Nola confirmation because OS scope
        // is absent. The real write contract exercises permission observation.
        let toolName = permissionRequired ? "device_write_file" : "device_read_file"
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: expectedIdentity, isForeground: true, allowedTools: [toolName],
            resultEgressAuthorization: .explicitUserApproval(identity: expectedIdentity, userApproved: true))
        // Egress consent permits advertising the tool; no file scope is granted.
        let fileTool = try XCTUnwrap(plan.output.enabledTools.first { $0.toolId == toolName },
            "Disabled tools: \(plan.output.disabledTools)")
        XCTAssertEqual(fileTool.authorization, "ask")
        let boundTool = try XCTUnwrap(plan.resolvedTool(id: toolName, route: "device",
            hostId: expectedIdentity.hostId, clientId: expectedIdentity.clientId, sessionId: sessionID))
        XCTAssertEqual(boundTool, fileTool)
        let restoredOutput = try JSONDecoder().decode(AgentCoreToolPolicyOutput.self,
            from: JSONEncoder().encode(plan.output))
        let restoredTool = try XCTUnwrap(restoredOutput.enabledTools.first { $0.toolId == toolName })
        XCTAssertEqual(restoredTool, boundTool, "The resolved bound contract must survive its current wire roundtrip")
        guard case .object(let schema) = restoredTool.schema,
              case .object(let metadata)? = schema["x_nola"] else {
            return XCTFail("The bound native manifest lost its trusted permission contract")
        }
        XCTAssertEqual(Set(metadata.keys), Set(["permission_contract"]), "Client display metadata is not execution authority")
        XCTAssertEqual(metadata["permission_contract"], .object([
            "risk": .string(permissionRequired ? "unknown" : "ordinary_read")
        ]))
        let suspended = expectation(description: "Authorization observation suspended")
        let consumerFinished = expectation(description: "Cancelled stream drained")
        let originalPermission = rejectSecondStream ? expectation(description: "Original owner still publishes its permission") : nil
        let forbiddenHTTP = expectation(description: "Cancelled tool must not send HTTP")
        forbiddenHTTP.isInverted = true
        let forbiddenBegin = suspendObservation == 1
            ? expectation(description: "Cancelled preparation must not create a Run") : nil
        forbiddenBegin?.isInverted = true
        let gate = ToolPlanObservationGate(at: suspendObservation, suspended: suspended,
            observation: .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000),
                isForeground: true, authorizations: plan.authorizationSnapshot))
        let toolSession = IOSRunToolPlanSession(plan: plan, observe: { await gate.observe() })
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
        let store = ToolPlanLifecycleStore(runtime: runtime, forbiddenBegin: forbiddenBegin)
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: sessionID, conversationTitle: "Cancellation isolation", identity: identity,
            userText: "Read the fixture", userContentJSON: nil, contextCheckpointID: nil))
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ToolPlanBlockedURLProtocol.self]
        ToolPlanBlockedURLProtocol.install(forbiddenHTTP)
        defer { ToolPlanBlockedURLProtocol.install(nil) }
        let urlSession = URLSession(configuration: configuration)
        defer { urlSession.invalidateAndCancel() }
        let executor = IOSAgentHostExecutor(session: urlSession)
        await executor.acquireAgentSession(sessionId: sessionID)
        let model = ToolPlanLifecycleModel(toolName: toolName)
        let coordinator = DeviceAgentCoordinator(modelClient: model, hostExecutor: executor,
            toolPlan: plan, canonicalApplication: application, toolPlanSession: toolSession,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: runtime, model: "fixture",
                outputTokens: 1_024, capabilities: model.coreModelCapabilityProfile))
        let stream = await coordinator.start(sessionId: sessionID, input: "Read the fixture", history: [],
            turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                permissionProfile: .cautious))
        let consumer = Task { () -> Int in
            var permissions = 0
            do {
                for try await event in stream {
                    if case .permissionRequested = event {
                        permissions += 1
                        originalPermission?.fulfill()
                    }
                }
            } catch is CancellationError {} catch {
                XCTFail("Unexpected lifecycle error: \(error)")
            }
            await gate.markConsumerFinished()
            consumerFinished.fulfill()
            return permissions
        }
        await fulfillment(of: [suspended], timeout: 5)
        let reachedObservation = await gate.didSuspend
        if rejectSecondStream {
            let rejected = await coordinator.start(sessionId: sessionID, input: "Must not replace owner", history: [],
                turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                    model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                    permissionProfile: .cautious))
            do {
                for try await _ in rejected {}
                XCTFail("A second stream must reject without changing the first owner")
            } catch DeviceAgentCoordinatorError.alreadyRunning {}
            await gate.release()
            if let originalPermission { await fulfillment(of: [originalPermission], timeout: 5) }
        }
        await coordinator.cancel()
        if suspendObservation != 1 && !rejectSecondStream {
            XCTAssertNil(try application.committedTerminalSeal(coreSessionID: sessionID),
                "An authorization await has not returned: closing observation must not manufacture cancellation acknowledgement")
        }
        await gate.release()
        // Always consume every created expectation, including failure cleanup.
        await fulfillment(of: [consumerFinished], timeout: 5)
        try await coordinator.waitForCancellation()
        await fulfillment(of: [forbiddenHTTP] + (forbiddenBegin.map { [$0] } ?? []), timeout: 0.2)
        guard reachedObservation else {
            consumer.cancel()
            await executor.releaseAgentSession(sessionId: sessionID)
            return // The failed expectation must not create a new suspended observation.
        }
        // Join the shared refresh itself; negative expectations cover late actor work
        // after that observation completes, without granting a replacement lifecycle.
        _ = try await toolSession.currentPlan()
        guard await gate.consumerFinished else {
            consumer.cancel()
            await executor.releaseAgentSession(sessionId: sessionID)
            return
        }
        let permissionCount = await consumer.value
        XCTAssertEqual(permissionCount, rejectSecondStream ? 1 : 0)
        for requestID in store.permissionRequestIDs {
            XCTAssertNotEqual(requestID, "fixture-call", "A permission request identity is not its model call identity")
            do {
                _ = try await coordinator.currentPermissionRequest(requestId: requestID)
                XCTFail("Cancelled lifecycle must not expose its actual pending permission")
            } catch DeviceAgentCoordinatorError.permissionNotPending {
                // Query the exact Core-issued ID, not an unrelated call ID.
            }
        }
        XCTAssertEqual(model.callCount, suspendObservation == 1 ? 0 : 1)
        if suspendObservation == 1 {
            XCTAssertEqual(store.commandCount, 0)
        } else if permissionRequired {
            XCTAssertGreaterThan(store.permissionClaims, 0,
                "The real Application must have claimed the permission effect before observation pauses")
            XCTAssertFalse(store.permissionRequestIDs.isEmpty)
            XCTAssertTrue(store.permissionClaimsHaveDeliveryWithoutModelAttempt,
                "Permission delivery identity is not a model-attempt identity")
        } else {
            XCTAssertEqual(store.permissionClaims, 0,
                "An ordinary read must not ask for Nola authority before its Host scope check")
        }
        if suspendObservation != 1 {
            let reopened = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
            let coldApplication = IOSCanonicalAgentApplication(store: reopened, descriptor: .init(
                conversationID: sessionID, conversationTitle: "", identity: identity,
                userText: "", userContentJSON: nil, contextCheckpointID: nil))
            let seal = try XCTUnwrap(coldApplication.committedTerminalSeal(coreSessionID: sessionID))
            XCTAssertEqual(seal.result.value.kind, .cancelled)
            XCTAssertNil(try reopened.loadConversationProjectionV1(id: sessionID).activeRun)
            XCTAssertNoThrow(try ChatManager.validateLocalRunAdmission(runtime: reopened, conversationID: sessionID))
            let pending = try reopened.executeAgentCommand(.object([
                "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)
            ]))
            guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
                return XCTFail("Missing formal outbox projection")
            }
            XCTAssertTrue(requests.isEmpty, "Cancellation controls must settle rather than disappear only from the UI queue")
        }
        await executor.releaseAgentSession(sessionId: sessionID)
    }
}

private actor ToolPlanObservationGate {
    let target: Int
    let suspended: XCTestExpectation
    let observation: IOSRunToolPlanObservation
    var count = 0
    var released = false
    var didSuspend = false
    var consumerFinished = false
    var continuation: CheckedContinuation<Void, Never>?
    init(at target: Int, suspended: XCTestExpectation, observation: IOSRunToolPlanObservation) {
        self.target = target; self.suspended = suspended; self.observation = observation
    }
    func observe() async -> IOSRunToolPlanObservation {
        count += 1
        if count == target && !released {
            await withCheckedContinuation { continuation in
                self.continuation = continuation
                didSuspend = true
                suspended.fulfill()
            }
        }
        return observation
    }
    func release() { released = true; continuation?.resume(); continuation = nil }
    func markConsumerFinished() { consumerFinished = true }
}

private nonisolated final class ToolPlanLifecycleStore: IOSAgentApplicationCommandStore, @unchecked Sendable {
    let runtime: IOSApplicationRuntime
    let forbiddenBegin: XCTestExpectation?
    private let lock = NSLock()
    private var count = 0
    private var claimedPermissions = 0
    private var validPermissionIdentities = true
    private var claimedPermissionRequestIDs: Set<String> = []
    var commandCount: Int { lock.withLock { count } }
    var permissionClaims: Int { lock.withLock { claimedPermissions } }
    var permissionClaimsHaveDeliveryWithoutModelAttempt: Bool { lock.withLock { validPermissionIdentities } }
    var permissionRequestIDs: Set<String> { lock.withLock { claimedPermissionRequestIDs } }
    init(runtime: IOSApplicationRuntime, forbiddenBegin: XCTestExpectation?) {
        self.runtime = runtime; self.forbiddenBegin = forbiddenBegin
    }
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        lock.withLock { count += 1 }
        if case .object(let fields) = command, fields["type"] == .string("begin_assistant_turn") {
            forbiddenBegin?.fulfill()
        }
        let result = try runtime.executeAgentCommand(command)
        if case .object(let fields) = command, fields["type"] == .string("claim_committed_host_requests"),
           case .object(let root) = result, case .array(let requests)? = root["requests"] {
            for request in requests {
                guard case .object(let envelope) = request,
                      case .object(let payload)? = envelope["payload"],
                      payload["type"] == .string("request_permission") else { continue }
                let valid: Bool
                if case .object(let delivery)? = envelope["delivery"],
                   case .string(let deliveryID)? = delivery["attempt_id"], !deliveryID.isEmpty {
                    valid = envelope["attempt_id"] == nil || envelope["attempt_id"] == .null
                } else { valid = false }
                lock.withLock {
                    claimedPermissions += 1
                    validPermissionIdentities = validPermissionIdentities && valid
                    if case .string(let permissionID)? = payload["permission_request_id"] {
                        claimedPermissionRequestIDs.insert(permissionID)
                    }
                }
            }
        }
        return result
    }
}

private nonisolated final class ToolPlanLifecycleModel: AgentModelClient, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    let toolName: String
    private let heldModelStarted: XCTestExpectation?
    private var heldContinuation: AsyncThrowingStream<AgentModelEvent, Error>.Continuation?
    private let heldModelDetached: XCTestExpectation?
    init(toolName: String, heldModelStarted: XCTestExpectation? = nil, heldModelDetached: XCTestExpectation? = nil) {
        self.toolName = toolName
        self.heldModelStarted = heldModelStarted
        self.heldModelDetached = heldModelDetached
    }
    func releaseHeldModel() {
        let continuation = lock.withLock {
            let value = heldContinuation
            heldContinuation = nil
            return value
        }
        continuation?.finish()
    }
    var callCount: Int { lock.withLock { count } }
    let coreModelCapabilityProfile: AgentCoreJSONValue? = .object([
        "profile_id": .string("ios-native-event-test-v1"), "revision": .number(1),
        "instruction_profile": .string("generic"), "native_tool_calls": .bool(true),
        "parallel_tool_calls": .bool(false), "reasoning": .bool(false), "reasoning_continuity": .bool(false),
        "prompt_cache": .bool(false), "context_window_tokens": .null, "max_output_tokens": .null,
        "request_continuation": .bool(true), "max_request_continuations": .number(2),
        "stream_text": .bool(true), "stream_reasoning": .bool(false), "stream_tool_calls": .bool(true)
    ])
    func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
        let iteration = lock.withLock { count += 1; return count }
        if let heldModelStarted {
            return AsyncThrowingStream { continuation in
                lock.withLock { heldContinuation = continuation }
                continuation.onTermination = { [heldModelDetached] _ in heldModelDetached?.fulfill() }
                heldModelStarted.fulfill()
            }
        }
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(.init(content: iteration == 1 ? "" : "done",
                toolCalls: iteration == 1 ? [.init(id: "fixture-call", name: toolName,
                    arguments: toolName == "device_write_file"
                        ? .object(["content": .string("cancelled fixture")]) : .object([:]))] : [],
                finishReason: iteration == 1 ? "tool_calls" : "stop")))
            continuation.finish()
        }
    }
}

private nonisolated final class ToolPlanBlockedURLProtocol: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var expectation: XCTestExpectation?
    static func install(_ value: XCTestExpectation?) { lock.withLock { expectation = value } }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lock.withLock { Self.expectation?.fulfill() }
        client?.urlProtocol(self, didFailWithError: URLError(.cancelled))
    }
    override func stopLoading() {}
}

final class IOSRunExecutionPermitTests: XCTestCase {
    func testExecutionPermitRevocationAndExecutorBridgeShortCircuit() async throws {
        let old = IOSRunExecutionPermit()
        let current = IOSRunExecutionPermit()
        try old.checkAdmission()
        old.revoke()
        old.revoke()
        XCTAssertThrowsError(try old.checkAdmission()) { error in
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertNoThrow(try current.checkAdmission())

        let executor = IOSAgentHostExecutor()
        let result = await executor.executeBridge(callId: "revoked", toolName: "memory_search",
            capability: .none, arguments: .object([:]), grant: nil, executionPermit: old)
        XCTAssertTrue(result.isError)
        guard case .object(let output) = result.output else { return XCTFail("Expected rejection") }
        XCTAssertEqual(output["error"], .string(CancellationError().localizedDescription))
    }
}
