// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class AgentCoreTaskProjectionTests: XCTestCase {
    func testResultStructureRejectsContradictoryEvidenceAndDurableCounterOverflow() throws {
        let base: [String: Any] = ["result_id": "result", "run_id": "run", "conversation_id": "conversation",
            "originating_submission_id": "submission",
            "input": ["watermark": "1", "applications_digest": String(repeating: "a", count: 64)],
            "decided_at_revision": "2", "decision_event_cursor": "3", "requirement_revision": "0", "evidence_frontier": "0",
            "value": ["kind": "answered", "answer_part_refs": ["answer"]]]
        func decode(_ object: [String: Any]) throws -> IOSRunResult {
            try JSONDecoder().decode(IOSRunResult.self, from: JSONSerialization.data(withJSONObject: object))
        }
        _ = try decode(base)
        var delegated = base
        delegated["value"] = ["kind": "delegation_accepted", "task_id": "task", "parent_call_id": "call"]
        XCTAssertEqual(try decode(delegated).value.kind, .delegationAccepted)
        for evidence in [["kind": "delegation_accepted", "task_id": "task"],
                         ["kind": "delegation_accepted", "task_id": "", "parent_call_id": "call"]] {
            delegated["value"] = evidence
            XCTAssertThrowsError(try decode(delegated))
        }
        let badValues: [[String: Any]] = [
            ["kind": "blocked", "reason": ["code": "blocked", "detail": NSNull()], "blocking_fact_refs": [], "resolution_condition": "Restore"],
            ["kind": "incomplete", "reason": ["code": "unfinished", "detail": NSNull()], "remaining_requirement_refs": [], "obligation_refs": []],
            ["kind": "failed", "error_ref": "error", "observed_effect_refs": ["effect"], "unknown_effect_refs": ["effect"]],
            ["kind": "cancelled", "cancellation_id": "cancel", "observed_effect_refs": ["effect"], "unknown_effect_refs": ["effect"]],
            ["kind": "blocked", "reason": ["code": "Localized reason", "detail": NSNull()], "blocking_fact_refs": ["fact"], "resolution_condition": "Restore"],
            ["kind": "answered", "answer_part_refs": [" answer"]],
        ]
        for value in badValues {
            var broken = base; broken["value"] = value
            XCTAssertThrowsError(try decode(broken))
        }
        for field in ["decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier"] {
            var valid = base; valid[field] = String(Int64.max); _ = try decode(valid)
            var broken = base; broken[field] = "9223372036854775808"
            XCTAssertThrowsError(try decode(broken), field)
        }
        let empty = "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
        for input in [["watermark": "0", "applications_digest": String(repeating: "a", count: 64)],
                      ["watermark": "1", "applications_digest": empty],
                      ["watermark": "9223372036854775808", "applications_digest": String(repeating: "a", count: 64)]] {
            var broken = base; broken["input"] = input
            XCTAssertThrowsError(try decode(broken))
        }
        var validEmpty = base; validEmpty["input"] = ["watermark": "0", "applications_digest": empty]
        _ = try decode(validEmpty)
    }
    func testSealedTaskRoundTripKeepsReceiptAndDoesNotBecomeBusinessCompleted() throws {
        let result: [String: Any] = ["result_id": "decision", "run_id": "child-run", "conversation_id": "child",
            "originating_submission_id": "submission", "input": ["watermark": "1", "applications_digest": String(repeating: "b", count: 64)],
            "decided_at_revision": "2", "decision_event_cursor": "4", "requirement_revision": "1", "evidence_frontier": "3",
            "value": ["kind": "answered", "answer_part_refs": ["answer"]]]
        let object: [String: Any] = ["id": "task", "owner_session_id": "parent", "owner_turn_id": "turn",
            "kind": "agent", "state": "sealed", "cancel_requested": false,
            "lineage": ["child_session_id": "child"],
            "durability": ["run_result": result, "result_digest": String(repeating: "a", count: 64), "terminal_receipt_id": "receipt"]]
        let task = try JSONDecoder().decode(AgentCoreTaskHandle.self, from: JSONSerialization.data(withJSONObject: object))
        XCTAssertTrue(task.isValid)
        XCTAssertTrue(task.state.isTerminal)
        XCTAssertNotEqual(task.state, .completed)
        var foreign = object
        foreign["lineage"] = ["child_session_id": "other-child"]
        XCTAssertFalse(try JSONDecoder().decode(AgentCoreTaskHandle.self,
            from: JSONSerialization.data(withJSONObject: foreign)).isValid)
        var missingEvidence = object
        var partialResult = result; partialResult["value"] = ["kind": "answered"]
        missingEvidence["durability"] = ["run_result": partialResult, "result_digest": String(repeating: "a", count: 64), "terminal_receipt_id": "receipt"]
        XCTAssertFalse(try JSONDecoder().decode(AgentCoreTaskHandle.self,
            from: JSONSerialization.data(withJSONObject: missingEvidence)).isValid)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreTaskHandle.self, from: JSONEncoder().encode(task)), task)
        for missing in ["run_result", "result_digest", "terminal_receipt_id"] {
            var broken = object
            var durability = try XCTUnwrap(broken["durability"] as? [String: Any])
            durability.removeValue(forKey: missing); broken["durability"] = durability
            let invalid = try JSONDecoder().decode(AgentCoreTaskHandle.self, from: JSONSerialization.data(withJSONObject: broken))
            XCTAssertFalse(invalid.isValid, missing)
        }
    }
    func testTaskProjectionRejectsStaleWrongIdentityAndWrongOwner() throws {
        var reducer = AgentCoreTaskProjectionReducer()
        let running = try XCTUnwrap(event(revision: 3, state: "running"))
        XCTAssertEqual(reducer.apply(running, expectedSessionId: session, expectedTurnId: turn), .applied)
        XCTAssertEqual(reducer.projection.orderedTasks.map(\.id), ["task-report-1"])
        XCTAssertEqual(reducer.apply(running, expectedSessionId: session, expectedTurnId: turn), .stale)
        XCTAssertEqual(reducer.apply(running, expectedSessionId: "wrong", expectedTurnId: turn), .identityMismatch)
        let wrongOwner = try XCTUnwrap(event(revision: 4, state: "waiting", ownerSession: "wrong"))
        XCTAssertEqual(reducer.apply(wrongOwner, expectedSessionId: session, expectedTurnId: turn), .invalidOwner)
    }

    func testShortToolEventDoesNotCreateBackgroundTask() {
        let shortTool: AgentCoreJSONValue = .object([
            "type": .string("tool_started"),
            "conversation_id": .string(session),
            "run_id": .string(turn),
            "name": .string("read_file"),
        ])
        XCTAssertNil(AgentCoreTaskEventEnvelope.decodeCore(from: shortTool))
        XCTAssertTrue(AgentCoreTaskProjectionReducer().projection.tasks.isEmpty)
    }

    func testTaskProjectionPersistsOnlyForOwningAssistantMessage() throws {
        var reducer = AgentCoreTaskProjectionReducer()
        _ = reducer.apply(
            try XCTUnwrap(event(revision: 3, state: "running")),
            expectedSessionId: session,
            expectedTurnId: turn
        )
        let owner = UUID()
        let encoded = try XCTUnwrap(AgentCoreTaskProjectionMeta.encode(reducer.projection, assistantMessageId: owner))
        let meta = [AgentCoreTaskProjectionMeta.conversationMetaKey: encoded]
        XCTAssertEqual(AgentCoreTaskProjectionMeta.decode(from: meta, assistantMessageId: owner), reducer.projection)
        XCTAssertNil(AgentCoreTaskProjectionMeta.decode(from: meta, assistantMessageId: UUID()))
        let restored = ChatMessage(id: owner, role: .assistant, content: "", metadata: meta)
        XCTAssertEqual(restored.taskProjection, reducer.projection)
        let shortToolOnly = ChatMessage(
            role: .assistant,
            content: "",
            hostToolEvents: [],
            metadata: [:]
        )
        XCTAssertNil(shortToolOnly.taskProjection)
    }

    private let session = "conversation-task-contract"
    private let turn = "turn-task-contract"

    private func event(
        revision: UInt64,
        state: String,
        ownerSession: String = "conversation-task-contract"
    ) -> AgentCoreTaskEventEnvelope? {
        AgentCoreTaskEventEnvelope.decodeCore(from: .object([
            "session_id": .string(session),
            "turn_id": .string(turn),
            "cursor": .object(["sequence": .number(Double(revision + 10))]),
            "revision": .number(Double(revision + 5)),
            "event": .object([
                "type": .string("task_updated"),
                "task_projection_revision": .number(Double(revision)),
                "task": .object([
                    "id": .string("task-report-1"),
                    "owner_session_id": .string(ownerSession),
                    "owner_turn_id": .string(turn),
                    "kind": .string("skill"),
                    "state": .string(state),
                    "progress": .number(state == "running" ? 0.25 : 0.5),
                    "current_step": .string("Reading source documents"),
                    "result_ref": .null,
                    "cancel_requested": .bool(false),
                ]),
            ]),
        ]))
    }
}
