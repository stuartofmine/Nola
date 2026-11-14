// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

final class IOSExpertHostMutationValidationTests: XCTestCase {
    func testCancellationOfPreviousMainTurnKeepsCurrentCallAndExactOwner() throws {
        let target = AppleExpertParticipantReference(participant_id: "target", agent_id: "agent",
            agent_revision: 1, context_conversation_id: "expert-context")
        let exchange = AppleExpertPeerView(version: 1, exchangeID: "exchange", roomID: "room",
            parentConversationID: "main", source: .mainAgent(contextConversationID: "main"),
            target: target, sourceRunID: "previous-main-run", sourceCallID: "previous-request-call",
            sourceDisplayName: "Main", targetDisplayName: "Expert", kind: "request", text: "Original",
            inReplyTo: nil, rootExchangeID: "exchange", hop: 1, createdAtMs: 1, clarifications: [])
        let view = try JSONSerialization.jsonObject(with: JSONEncoder().encode(exchange))
        let operation: [String: Any] = ["kind": "peer_task_action", "request": [
            "session_id": "main", "turn_id": "later-main-run", "payload": ["call_id": "current-cancel-call",
                "input": ["action": "cancel", "exchange_id": "exchange"]]]]
        func receipt(call: String = "current-cancel-call", task: String = "exchange", owner: String = "main") throws -> Data {
            var projected = try XCTUnwrap(view as? [String: Any])
            projected["source"] = ["kind": "main_agent", "context_conversation_id": owner]
            return try JSONSerialization.data(withJSONObject: ["terminal": false, "wait_ms": 0,
                "response": ["type": "tool_completed", "call_id": call,
                    "output": ["exchange": projected, "task": ["id": task]]]])
        }
        XCTAssertNoThrow(try IOSExpertHostMutationValidation.validate(receipt(), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(call: "foreign-call"), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(task: "foreign-task"), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(owner: "foreign-main"), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(), operation: ["kind": "unknown_mutation"]))
    }
    func testAlreadyAdmittedPreparationOnlyAcceptsExactRecoveredTaskReply() throws {
        let target = AppleExpertParticipantReference(participant_id: "target", agent_id: "agent",
            agent_revision: 1, context_conversation_id: "expert")
        let view = AppleExpertPeerView(version: 1, exchangeID: "exchange", roomID: "room",
            parentConversationID: "main", source: .mainAgent(contextConversationID: "main"), target: target,
            sourceRunID: "run", sourceCallID: "call", sourceDisplayName: "Main", targetDisplayName: "Expert",
            kind: "request", text: "Request", inReplyTo: nil, rootExchangeID: "exchange", hop: 1,
            createdAtMs: 1, clarifications: [])
        let operation: [String: Any] = ["kind": "prepare_peer_task", "exchange_id": "exchange", "request": [
            "session_id": "main", "turn_id": "run", "payload": ["call_id": "call",
                "input": ["action": "request", "target_participant_id": "target"]]]]
        func receipt(admitted: Bool = true, call: String = "call", task: String = "exchange") throws -> Data {
            try JSONSerialization.data(withJSONObject: [
                "view": JSONSerialization.jsonObject(with: JSONEncoder().encode(view)),
                "prepared": ["task": ["id": "exchange", "owner_session_id": "main", "child_session_id": "expert"],
                    "snapshot": ["owner_session_id": "main"], "replayed": true, "source_admitted": admitted],
                "response": ["type": "tool_completed", "call_id": call,
                    "output": ["exchange_id": "exchange", "task_id": task, "replayed": true]]])
        }
        XCTAssertNoThrow(try IOSExpertHostMutationValidation.validate(receipt(), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(admitted: false), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(call: "foreign"), operation: operation))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(receipt(task: "foreign"), operation: operation))
    }

}
