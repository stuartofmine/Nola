import XCTest
@testable import Nola

@MainActor
final class MacUnknownEffectRecoveryControlTests: XCTestCase {
    private func fixture(_ run: String = "run") -> MacUnknownEffectRecoveryDecision {
        MacUnknownEffectRecoveryDecision(conversationID: "conversation-\(run)", runID: run,
            invocationConfigDigest: "digest", resolutionID: "resolution-\(run)",
            targets: [MacUnknownEffectRecoveryTarget(effectID: "effect", requestID: "request", attemptID: "attempt",
                workerGeneration: "generation", leaseOwner: "owner", leaseToken: "fence", expectedAttemptRevision: 4)],
            choice: .init(kind: .authorizeRiskRetry, reason: "Explicitly accept duplicate-effect risk"), resolvedAtMs: 1234)
    }
    private func database() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        return directory.appendingPathComponent("fixture.db")
    }
    private func reply(_ decision: MacUnknownEffectRecoveryDecision) -> [String: Any] {
        ["outcome": "retry_authorized", "resolution_id": decision.resolutionID, "run_id": decision.runID]
    }

    func testOwnedTaskKeepsUnknownAndRecoversExactReceiptAfterReopen() throws {
        let url = try database()
        let owner = AppleOwnedTaskRecoveryOwner(parent_conversation_id: "parent", parent_run_id: "parent-run", task_id: "task")
        let risky = fixture()
        let decision = MacUnknownEffectRecoveryDecision(conversationID: risky.conversationID, runID: risky.runID,
            invocationConfigDigest: risky.invocationConfigDigest, resolutionID: risky.resolutionID,
            targets: risky.targets, choice: .init(kind: .keepUnknownBlocked, reason: "Keep the physical outcome unknown"),
            resolvedAtMs: risky.resolvedAtMs)
        var submitted = 0
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, id in
            submitted += 1
            let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(command["type"] as? String, "resolve_owned_task_recovery")
            let sentOwner = try XCTUnwrap(command["owner"] as? [String: String])
            XCTAssertEqual(sentOwner["parent_run_id"], owner.parent_run_id)
            XCTAssertEqual(id, "fixed-owned")
            throw MacApplicationServiceError.connectionFailed("lost reply")
        }
        XCTAssertThrowsError(try first.submitConfirmed(risky, assistantMessageID: "assistant", taskOwner: owner))
        XCTAssertEqual(submitted, 0, "Parent recovery must not authorize a retry")
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "fixed-owned", taskOwner: owner))
        XCTAssertEqual(try first.pendingOwnedDecision(owner), decision)
        XCTAssertNil(try first.pendingOwnedDecision(.init(parent_conversation_id: "parent", parent_run_id: "another-run", task_id: "task")))
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            XCTAssertEqual(query["request_id"] as? String, "fixed-owned")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "fixed-owned", "status": "committed",
                "result": ["outcome": "unknown_preserved_blocked", "resolution_id": decision.resolutionID, "run_id": decision.runID]])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: decision.runID, taskOwner: owner)?.outcome, .unknownPreservedBlocked)
        XCTAssertNil(try reopened.pendingOwnedDecision(owner))
    }

    func testLostReplyReopensAndQueriesOriginalReceiptWithoutResending() throws {
        let url = try database(), decision = fixture()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, id in
            XCTAssertEqual(id, "fixed-request")
            throw MacApplicationServiceError.connectionFailed("lost reply")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "fixed-request"))
        XCTAssertEqual(try first.pendingDecision(runID: "run"), decision)
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            XCTAssertEqual(query["request_id"] as? String, "fixed-request")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "fixed-request", "status": "committed", "result": self.reply(decision)])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run")?.outcome, .retryAuthorized)
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
    }

    func testLocalRejectedErrorIsNotAuthorityToRetireAndOtherRunsRemainIndependent() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.rejected("local protocol mismatch, no receipt")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "original"))
        XCTAssertThrowsError(try first.submitConfirmed(fixture("other"), assistantMessageID: "other-assistant", requestID: "other-request"))
        XCTAssertEqual(try first.pendingDecision(runID: "other"), fixture("other"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["request_id"] as? String, "original")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "original", "status": "rejected", "error": ["message": "canonical rejection"]])
        }
        XCTAssertThrowsError(try reopened.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "replacement"))
        XCTAssertEqual(calls, 0)
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
        XCTAssertNotNil(try reopened.pendingDecision(runID: "other"))
    }

    func testMissingReceiptReplaysIdenticalPayloadAndRequestNotNewAuthorization() throws {
        let url = try database(), decision = fixture()
        var original: Data?
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            original = data
            throw MacApplicationServiceError.outcomePending(requestID: "original")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, id in
            calls += 1
            if calls == 1 { return Data("null".utf8) }
            XCTAssertEqual(id, "original")
            XCTAssertEqual(data, original)
            return try JSONSerialization.data(withJSONObject: self.reply(decision))
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run")?.outcome, .retryAuthorized)
        XCTAssertEqual(calls, 2)
    }

    func testTerminalReconciliationOnlyQueriesReceiptAndPreservesMissingIntent() throws {
        let url = try database(), decision = fixture()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("reply lost")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(command["type"] as? String, "command_receipt")
            XCTAssertEqual(command["request_id"] as? String, "original")
            return Data("null".utf8)
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run", replayIfMissing: false))
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(try reopened.pendingDecision(runID: "run"), decision)
    }

    func testTerminalBlockedReceiptRetiresOriginalDecisionWithoutReplaying() throws {
        let url = try database(), base = fixture()
        let decision = MacUnknownEffectRecoveryDecision(conversationID: base.conversationID,
            runID: base.runID, invocationConfigDigest: base.invocationConfigDigest,
            resolutionID: base.resolutionID, targets: base.targets,
            choice: .init(kind: .keepUnknownBlocked, reason: "Preserve the unknown result"), resolvedAtMs: base.resolvedAtMs)
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("reply lost after commit")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            return try JSONSerialization.data(withJSONObject: [
                "protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "original", "status": "committed",
                "result": ["outcome": "unknown_preserved_blocked", "resolution_id": decision.resolutionID,
                           "run_id": decision.runID],
            ])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run", replayIfMissing: false)?.outcome, .unknownPreservedBlocked)
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
    }

    func testWrongClientAndPersistenceFailureNeverTransmit() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "owner") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let wrong = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "foreign") { _, _ in
            XCTFail("wrong client must not transmit"); return Data()
        }
        XCTAssertThrowsError(try wrong.recoverPending(runID: "run"))
        XCTAssertNotNil(try first.pendingDecision(runID: "run"))
        let brokenURL = url.deletingLastPathComponent().appendingPathComponent("broken.db")
        try Data("not a directory".utf8).write(to: brokenURL.appendingPathExtension("pending-recovery-decisions"))
        let broken = MacUnknownEffectRecoveryControl(databaseURL: brokenURL, clientID: "client") { _, _ in
            XCTFail("persistence must precede transmission"); return Data()
        }
        XCTAssertThrowsError(try broken.submitConfirmed(fixture(), assistantMessageID: "assistant"))
    }

    func testProtocolMaximumTargetsFitTheBoundedJournal() throws {
        let url = try database(), base = fixture()
        let identity = String(repeating: "\\", count: 512)
        let target = MacUnknownEffectRecoveryTarget(effectID: identity, requestID: identity, attemptID: identity,
            workerGeneration: identity, leaseOwner: identity, leaseToken: identity, expectedAttemptRevision: 4)
        let decision = MacUnknownEffectRecoveryDecision(conversationID: base.conversationID, runID: base.runID,
            invocationConfigDigest: base.invocationConfigDigest, resolutionID: base.resolutionID,
            targets: Array(repeating: target, count: 256), choice: .init(kind: .keepUnknownBlocked, reason: String(repeating: "r", count: 4096)), resolvedAtMs: 1234)
        var transmitted = false
        let service = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            transmitted = true; throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try service.submitConfirmed(decision, assistantMessageID: "assistant"))
        XCTAssertTrue(transmitted)
        XCTAssertEqual(try service.pendingDecision(runID: "run"), decision)
    }

    func testAcceptedOrWrongOwnerReceiptKeepsTheFrozenIntent() throws {
        for wrongOwner in [false, true] {
            let url = try database(), decision = fixture()
            let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
                throw MacApplicationServiceError.connectionFailed("offline")
            }
            XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
            let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
                try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                    "client_id": wrongOwner ? "foreign" : "client", "request_id": "original", "status": "accepted"])
            }
            XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
            XCTAssertEqual(try reopened.pendingDecision(runID: "run"), decision)
        }
    }

    func testCorruptIntentFailsClosedWithoutDeletingOrSending() throws {
        let url = try database()
        let service = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try service.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let files = try FileManager.default.contentsOfDirectory(at: url.appendingPathExtension("pending-recovery-decisions"), includingPropertiesForKeys: nil)
        let journal = try XCTUnwrap(files.first { $0.pathExtension == "json" })
        let corrupt = Data("{broken".utf8)
        try corrupt.write(to: journal)
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            XCTFail("corrupt intent cannot authorize a transmission"); return Data()
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertEqual(try Data(contentsOf: journal), corrupt)
    }

    func testOversizedIntentBlocksRecoveryAndReplacementWithoutTransmission() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let files = try FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("pending-recovery-decisions"), includingPropertiesForKeys: nil)
        let journal = try XCTUnwrap(files.first { $0.pathExtension == "json" })
        let size = 12 * 1024 * 1024 + 1
        try Data(repeating: 0x20, count: size).write(to: journal)
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            XCTFail("oversized intent must not permit any transmission"); return Data()
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertThrowsError(try reopened.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "replacement"))
        let attributes = try FileManager.default.attributesOfItem(atPath: journal.path)
        XCTAssertEqual((attributes[.size] as? NSNumber)?.intValue, size)
    }
}
