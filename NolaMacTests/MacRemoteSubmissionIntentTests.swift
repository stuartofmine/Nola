// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacRemoteSubmissionIntentTests: XCTestCase {
    func testPreparedIntentHasNoRunOrMessagesAndCanBeAbandonedWithoutSourceReceipt() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("receiver.db"))
        let prepared = try port.prepareRemoteSubmission(input())
        XCTAssertEqual(prepared.phase, .prepared)
        XCTAssertNil(prepared.request_digest)
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertTrue(try port.loadConversationWindow("conversation").projection.messages.isEmpty)
        let cancelled = try port.abandonPreparedRemoteSubmission(prepared.identity)
        XCTAssertEqual(cancelled.phase, .cancelled)
        XCTAssertNil(try port.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
        XCTAssertEqual(try port.abandonPreparedRemoteSubmission(prepared.identity).phase, .cancelled)
    }

    func testBoundRequestNeedsExactSourceAdmissionAndSurvivesColdReload() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("receiver.db")
        let port = try MacApplicationPort(databaseURL: url)
        let bound = try port.prepareFixtureRemoteSubmission(input())
        XCTAssertEqual(bound.phase, .bound)
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertTrue(try port.loadConversationWindow("conversation").projection.messages.isEmpty)
        XCTAssertThrowsError(try port.abandonPreparedRemoteSubmission(bound.identity))
        let reopened = try MacApplicationPort(databaseURL: url)
        let restored = try XCTUnwrap(reopened.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertEqual(restored.identity, bound.identity)
        XCTAssertEqual(restored.request_digest, bound.request_digest)
        let frozen = try reopened.remoteSubmissionRequest(identity: bound.identity,
            digest: XCTUnwrap(bound.request_digest))
        let semantic = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(frozen)) as? [String: Any])
        let source = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: "run",
            assistantMessageID: "run:assistant", userText: "Original request", userMessageID: "run:user",
            clientID: "fixture-client", semanticRequest: semantic)
        let receipt = try source.submissionReceipt()
        XCTAssertEqual(receipt.request_digest, bound.request_digest)
        let wrong = MacSourceSubmissionReceipt(protocol_version: receipt.protocol_version,
            identity: .init(source_host_id: "foreign-host", client_id: receipt.identity.client_id,
                conversation_id: receipt.identity.conversation_id, submission_id: receipt.identity.submission_id,
                user_message_id: receipt.identity.user_message_id, assistant_message_id: receipt.identity.assistant_message_id),
            request_digest: receipt.request_digest, phase: receipt.phase, admission: receipt.admission,
            rejection_code: receipt.rejection_code)
        XCTAssertThrowsError(try reopened.acceptRemoteSubmissionReceipt(wrong))
        XCTAssertNil(try reopened.activeRunIdentity(conversationID: "conversation"))
        XCTAssertEqual(try reopened.acceptRemoteSubmissionReceipt(receipt).phase, .admitted)
        XCTAssertEqual(try reopened.acceptRemoteSubmissionReceipt(receipt).phase, .admitted)
        XCTAssertEqual(try reopened.activeRunIdentity(conversationID: "conversation")?.runID, "run")
        let wrongDigest = MacSourceSubmissionReceipt(protocol_version: receipt.protocol_version,
            identity: receipt.identity, request_digest: String(repeating: "f", count: 64),
            phase: receipt.phase, admission: receipt.admission, rejection_code: receipt.rejection_code)
        XCTAssertThrowsError(try reopened.adoptObservedRemoteRun(wrongDigest),
            "Observation must validate the originating intent's exact bound request")
        XCTAssertThrowsError(try reopened.adoptObservedRemoteRun(wrong))
        XCTAssertNoThrow(try reopened.adoptObservedRemoteRun(receipt),
            "Exact observation reuses bound acceptance idempotently, including after cold reload")
        XCTAssertEqual(try reopened.remoteSubmissionIntent(conversationID: "conversation")?.request_digest,
            bound.request_digest)
        try source.ingestAdmissions(into: reopened)
        _ = try source.finish("Actual source completion")
        _ = try source.commitTerminal(into: reopened)
        XCTAssertNotNil(try reopened.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
        XCTAssertNil(try reopened.remoteSubmissionIntent(conversationID: "conversation"))
    }

    private func input() -> MacPrepareRemoteSubmissionInput {
        .init(conversationID: "conversation", conversationTitle: "Intent", submissionID: "run",
            sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "run:user",
            userPartID: "run:user:text", userText: "Original request", userContentJSON: nil,
            assistantMessageID: "run:assistant", assistantPartID: "run:assistant:text",
            contextCheckpointID: nil, createdAtMs: 1)
    }
}
