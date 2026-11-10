// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

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
        let digest = String(repeating: "a", count: 64)
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/cancel")
            let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: request.httpBody!) as? [String: Any])
            let input = try WebAgentProvider.decryptMobileJSON(wire, token: "fixture")
            XCTAssertEqual(Set(input.keys), ["identity", "request_digest"])
            XCTAssertEqual(input["request_digest"] as? String, digest)
            let sent = try JSONDecoder().decode(IOSRemoteSubmissionIdentity.self,
                from: JSONSerialization.data(withJSONObject: input["identity"]!))
            XCTAssertEqual(sent, identity)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { $0.finish() },
                response: HTTPURLResponse(url: request.url!, statusCode: 202, httpVersion: nil, headerFields: nil)!)
        }
        try await provider.cancelHostGeneration(identity: identity, requestDigest: digest)
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
