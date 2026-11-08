// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class AppleGoalControlOutboxTests: XCTestCase {
    private func directory() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent("goal-intent-\(UUID().uuidString)") }
    private func intent() -> AppleGoalControlIntent {
        AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .pause, expectedGoalId: "goal", expectedControlToken: "token"))
    }
    private static func response(_ intent: AppleGoalControlIntent, pending: Bool = false,
        host: String = "host", rejection: String? = nil) throws -> AppleGoalControlResponse {
        let operation: [String: Any] = ["command_sha256": String(repeating: "a", count: 64),
            "goal_revision": 2, "rejection": rejection as Any? ?? NSNull()]
        return try JSONDecoder().decode(AppleGoalControlResponse.self, from: JSONSerialization.data(withJSONObject: [
            "source_host_id": host, "conversationId": intent.conversation_id, "operationId": intent.control.operationId,
            "controlPending": pending, "authoritative": !pending,
            "goalProjection": ["applied_operations": pending ? [:] : [intent.control.operationId: operation]]
        ]))
    }
    func testLostResponseSurvivesColdOutboxAndResendsIdenticalOperation() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let original = intent()
        let first = AppleGoalControlOutbox(directory: directory)
        do {
            _ = try await first.submit(original) { saved in
                XCTAssertEqual(saved, original)
                throw URLError(.networkConnectionLost)
            }
            XCTFail("transport loss must remain unresolved")
        } catch is URLError {}
        let reopened = AppleGoalControlOutbox(directory: directory)
        let pending = try await reopened.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
        let response = try Self.response(original)
        let confirmed = try await reopened.submit(try XCTUnwrap(pending)) { saved in
            XCTAssertEqual(saved, original)
            return response
        }
        XCTAssertTrue(confirmed)
        let cleared = try await reopened.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNil(cleared)
    }
    func testProvisionalReplyCannotSettleOrReplaceSavedIntent() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let provisional = try Self.response(original, pending: true)
        let confirmed = try await outbox.submit(original) { _ in provisional }
        XCTAssertFalse(confirmed)
        do {
            _ = try await outbox.submit(intent()) { _ in XCTFail("replacement cannot reach transport"); return provisional }
            XCTFail("must preserve original operation")
        } catch AppleGoalControlOutbox.Failure.unresolvedPrevious {}
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
    }
    func testForeignHostReplyPreservesIntentAndExactProgressLedgerSettlesIt() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let foreign = try Self.response(original, host: "foreign")
        do { _ = try await outbox.submit(original) { _ in foreign }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.invalidResponse {}
        let response = try Self.response(original)
        let settled = try await outbox.reconcile(hostID: "host", conversationID: "conversation",
            appliedOperations: try XCTUnwrap(response.goalProjection).applied_operations)
        XCTAssertTrue(settled)
    }
    func testCanonicalRejectionSettlesOriginalIntentWithoutClaimingSuccess() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let rejected = try Self.response(original, rejection: "Goal state changed")
        do { _ = try await outbox.submit(original) { _ in rejected }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.rejected(let reason) { XCTAssertEqual(reason, "Goal state changed") }
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNil(pending)
    }
    func testGoalWireContainsFixedIntentOnlyAndValidatesActionShape() throws {
        let start = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Finish review", constraints: []))
        try start.control.validate()
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(start)) as? [String: Any])
        XCTAssertEqual(Set(body.keys), ["expected_host_id", "conversation_id", "control"])
        let control = try XCTUnwrap(body["control"] as? [String: Any])
        XCTAssertEqual(Set(control.keys), ["operationId", "action", "objective", "constraints"])
        XCTAssertNil(control["nowEpochMs"])
        XCTAssertThrowsError(try AppleGoalControl(action: .resume).validate())
        XCTAssertThrowsError(try AppleGoalControl(action: .schedule, expectedGoalId: "goal", expectedControlToken: "token").validate())
    }
    @MainActor func testPairedGoalUsesEncryptedOwnerAndTreatsHTTP202AsPending() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let control = AppleGoalControl(action: .pause, expectedGoalId: "goal", expectedControlToken: "token")
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!,
            token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/goal")
            XCTAssertEqual(request.value(forHTTPHeaderField: "X-Nola-Mobile-Crypto"), "aes-gcm-v1")
            let encrypted = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
            let body = try WebAgentProvider.decryptMobileJSON(encrypted, token: "fixture-token")
            let requestControl = try XCTUnwrap(body["control"] as? [String: Any])
            XCTAssertEqual(body["expected_host_id"] as? String, "host")
            XCTAssertEqual(body["conversation_id"] as? String, "conversation")
            XCTAssertEqual(requestControl["operationId"] as? String, control.operationId)
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": "host", "conversationId": "conversation", "operationId": control.operationId,
                "authoritative": true, "goalProjection": ["applied_operations": [control.operationId: [
                    "command_sha256": String(repeating: "a", count: 64), "goal_revision": 2, "rejection": NSNull()
                ]]]
            ], token: "fixture-token")
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 202, httpVersion: nil, headerFields: nil)!)
        }
        let confirmed = try await provider.controlConversationGoal(conversationID: "conversation", control: control, outbox: outbox)
        XCTAssertFalse(confirmed)
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending?.control, control)
    }

    func testErrorWithoutExactOperationLedgerRemainsUnconfirmed() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent()
        let response = try JSONDecoder().decode(AppleGoalControlResponse.self,
            from: Data(#"{"source_host_id":"host","error":"Configuration unavailable"}"#.utf8))
        do { _ = try await outbox.submit(original) { _ in response }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.unconfirmed(let reason) { XCTAssertEqual(reason, "Configuration unavailable") }
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
    }

    func testStartDraftSurvivesPendingAndRejectionAndClearsOnlyAfterApplied() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let original = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Retain this objective", constraints: ["Keep files"]))
        let outbox = AppleGoalControlOutbox(directory: directory)
        let provisional = try Self.response(original, pending: true)
        _ = try await outbox.submit(original) { _ in provisional }
        let cold = AppleGoalControlOutbox(directory: directory)
        let pendingDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pendingDraft?.objective, "Retain this objective")
        let rejection = try Self.response(original, rejection: "Model unavailable")
        do { _ = try await cold.submit(original) { _ in rejection }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.rejected {}
        let rejectedDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(rejectedDraft, pendingDraft)
        let retry = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Retain this objective", constraints: ["Keep files"]))
        let applied = try Self.response(retry)
        _ = try await cold.submit(retry) { _ in applied }
        let clearedDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertNil(clearedDraft)
    }

}
