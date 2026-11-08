import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacRunRecoveryInspectionTests: XCTestCase {
    func testRestoredRunIsNotPresentedAsNewConnectionFailure() {
        let live = MacLocalRunRecovery(conversationID: "c", expectedRunID: "r",
            canonicalAssistantMessageID: "r:assistant", assistantMessageID: UUID(), generationID: 1)
        var restored = live
        restored.restoredFromStorage = true
        XCTAssertEqual(restored.title,
            String(localized: "An earlier task is unfinished. It has not been restarted.", table: "Chat"))
        XCTAssertNotEqual(restored.title, live.title)
        XCTAssertEqual(restored.expectedRunID, live.expectedRunID)
    }

    func testRealApplicationQueryPreservesMissingAndBoundRemoteIdentityWithoutMutation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("store.db"))
        let missing = try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
                                                  assistantMessageID: "run:assistant")
        XCTAssertEqual(missing.observation, .missing)
        XCTAssertThrowsError(try missing.requireActiveRemoteSource("fixture-host"))
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Inspection",
            submissionID: "run", sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "user", userPartID: "user:text",
            userText: "Original", userContentJSON: nil, assistantMessageID: "run:assistant",
            assistantPartID: "run:assistant:text", contextCheckpointID: nil, createdAtMs: 1))
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: "conversation",
            runID: "run", assistantMessageID: "run:assistant")
        withExtendedLifetime(source) {}
        let before = try port.loadConversationWindow("conversation").projection.revision
        let inspection = try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
                                                     assistantMessageID: "run:assistant")
        guard case .active(let facts) = inspection.observation else { return XCTFail("Expected persisted active facts") }
        XCTAssertEqual(facts.sourceHostID, "fixture-host")
        XCTAssertNoThrow(try inspection.requireActiveRemoteSource("fixture-host"))
        XCTAssertThrowsError(try inspection.requireActiveRemoteSource("different-host"))
        XCTAssertThrowsError(try inspection.requireActiveRemoteSource(""))
        XCTAssertEqual(facts.pendingEffectCount, 0)
        XCTAssertTrue(facts.deliveries.isEmpty)
        XCTAssertEqual(try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
            assistantMessageID: "other-assistant").observation, .identityMismatch)
        XCTAssertEqual(try port.inspectRunRecovery(conversationID: "other-conversation", expectedRunID: "run",
            assistantMessageID: "run:assistant").observation, .identityMismatch)
        XCTAssertEqual(try port.loadConversationWindow("conversation").projection.revision, before)
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
    }

    func testDecoderRejectsUnknownOutcomeInsteadOfTreatingItAsRecoveryAuthority() throws {
        func decode(_ observation: String) throws -> MacRunRecoveryInspection {
            try JSONDecoder().decode(MacRunRecoveryInspection.self, from: Data(
                "{\"conversation_id\":\"c\",\"run_id\":\"r\",\"assistant_message_id\":\"a\",\"observation\":\(observation)}".utf8))
        }
        XCTAssertThrowsError(try decode(#"{"state":"safe_to_retry"}"#))
        XCTAssertThrowsError(try decode(#"{"state":"terminal","status":"unknown","completed_at_ms":1}"#))
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let object: [String: Any] = ["state": "terminal", "result": macRunResultFixture(status.rawValue, conversation: "c", run: "r"),
                "result_digest": String(repeating: "0", count: 64), "completed_at_ms": 42]
            let value = try decode(String(decoding: JSONSerialization.data(withJSONObject: object), as: UTF8.self))
            guard case .terminal(let result, _, let timestamp) = value.observation else { return XCTFail("Missing result") }
            XCTAssertEqual(result.value.kind, status)
            XCTAssertEqual(timestamp, 42)
            XCTAssertThrowsError(try value.requireActiveRemoteSource("fixture-host"))
        }
    }

    func testRecoveryExplanationDoesNotClaimMissingRunWasNeverSubmitted() {
        XCTAssertEqual(MacRunRecoveryPresentation.detail(.missing),
            String(localized: "No committed Run record was found. Submission remains unconfirmed.", table: "Chat"))
        let facts = MacRunRecoveryInspection.Active(runStatus: "running", checkpointPresent: true,
            pendingEffectCount: 1, sourceHostID: nil, hasSessionLease: false,
            deliveries: [.init(state: "outcome_unknown", count: 1)])
        let local = MacRunRecoveryInspection(conversationID: "c", runID: "r",
            assistantMessageID: "a", observation: .active(facts))
        XCTAssertThrowsError(try local.requireActiveRemoteSource("fixture-host"),
            "A local checkpoint cannot authorize recovery on the current remote Host")
        XCTAssertEqual(MacRunRecoveryPresentation.detail(.active(facts)),
            String(localized: "An execution result is unknown. No automatic retry was attempted.", table: "Chat"))
    }
}
