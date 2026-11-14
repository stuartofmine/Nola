// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSCanonicalRunFeedbackTests: XCTestCase {
    func testReopenedBlockedResultUsesExactCanonicalOwnerAndOnlyReadCommand() throws {
        let cursor = IOSAssistantJournalCursor(identity: .init(source_store_epoch: "store", conversation_id: "parent",
            run_id: "run", assistant_message_id: "assistant"), sequence: "2", record_digest: String(repeating: "a", count: 64))
        let result: [String: Any] = ["result_id": "result", "run_id": "run", "conversation_id": "parent",
            "originating_submission_id": "submission", "input": ["watermark": "0", "applications_digest": "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"],
            "decided_at_revision": "1", "decision_event_cursor": "2", "requirement_revision": "0", "evidence_frontier": "0",
            "value": ["kind": "blocked", "reason": ["code": "context.input_budget_exceeded", "detail": NSNull()],
                "resolution_condition": "opaque budget", "blocking_fact_refs": ["budget"]]]
        for assistant in ["assistant", "other-assistant"] {
            let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: [
                "conversation_id": "parent", "run_id": "run", "assistant_message_id": assistant, "status": "sealed", "result": result]))
            let store = FeedbackStore(response: response)
            if assistant == "assistant" { XCTAssertNotNil(try IOSCanonicalRunFeedback.read(cursor: cursor, store: store)) }
            else { XCTAssertThrowsError(try IOSCanonicalRunFeedback.read(cursor: cursor, store: store)) }
        }
        XCTAssertNil(try IOSCanonicalRunFeedback.read(cursor: cursor, store: FeedbackStore(response: .null)))
    }
}

private nonisolated struct FeedbackStore: IOSAgentApplicationCommandStore {
    let response: AgentCoreJSONValue
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        XCTAssertEqual(command.objectFields?["type"], .string("canonical_run_terminal"))
        XCTAssertEqual(command.objectFields?["conversation_id"], .string("parent"))
        XCTAssertEqual(command.objectFields?["run_id"], .string("run"))
        return response
    }
}
