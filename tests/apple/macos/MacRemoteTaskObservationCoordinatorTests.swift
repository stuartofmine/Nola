// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies remote task observation cursor, terminal, and cancellation lifecycles.

import XCTest
@testable import Nola

@MainActor
final class MacRemoteTaskObservationCoordinatorTests: XCTestCase {
    func testSealedAgentTaskRetainsAllBusinessResultsAndReceiptIdentity() throws {
        for kind in ["answered", "action_completed", "incomplete", "blocked", "cancelled", "failed"] {
            let object: [String: Any] = ["id": "task", "owner_session_id": "parent", "owner_turn_id": "turn",
                "kind": "agent", "state": "sealed", "cancel_requested": false,
                "lineage": ["child_session_id": "child"],
                "durability": ["run_result": macRunResultFixture(kind, conversation: "child", run: "child-run"),
                    "result_digest": String(repeating: "a", count: 64), "terminal_receipt_id": "receipt"]]
            let task = try JSONDecoder().decode(MacCoreTaskHandle.self, from: JSONSerialization.data(withJSONObject: object))
            XCTAssertTrue(task.hasValidResultShape)
            var foreign = object; foreign["lineage"] = ["child_session_id": "other-child"]
            XCTAssertFalse(try JSONDecoder().decode(MacCoreTaskHandle.self,
                from: JSONSerialization.data(withJSONObject: foreign)).hasValidResultShape)
            XCTAssertTrue(task.state.isTerminal)
            XCTAssertFalse(MacCoreTaskUIModel(task: task).canCancel)
            XCTAssertFalse(MacCoreTaskUIModel(task: task).canResume)
            let ui = MacCoreTaskUIModel(task: task)
            XCTAssertEqual(ui.terminalResult?.value.kind.rawValue, kind)
            XCTAssertNotNil(ui.outcomeLabel)
            XCTAssertNotEqual(ui.outcomeLabel, String(localized: "Execution sealed"))
            if ["incomplete", "blocked", "failed"].contains(kind) {
                XCTAssertFalse(ui.outcomeDetails.isEmpty, kind)
            }
            let copy = try JSONDecoder().decode(MacCoreTaskHandle.self, from: JSONEncoder().encode(task))
            XCTAssertEqual(copy, task)
            XCTAssertEqual(copy.durability?.terminalReceiptID, "receipt")
            let snapshot = MacCoreTaskProjection(revision: 2, order: [task.id], tasks: [task.id: task])
            XCTAssertEqual(try JSONDecoder().decode(MacCoreTaskProjection.self, from: JSONEncoder().encode(snapshot)), snapshot)
            for missing in ["run_result", "result_digest", "terminal_receipt_id"] {
                var broken = object
                var durability = try XCTUnwrap(broken["durability"] as? [String: Any])
                durability.removeValue(forKey: missing); broken["durability"] = durability
                let invalid = try JSONDecoder().decode(MacCoreTaskHandle.self, from: JSONSerialization.data(withJSONObject: broken))
                XCTAssertFalse(invalid.hasValidResultShape, missing)
                let corruptSnapshot = MacCoreTaskProjection(revision: 2, order: [invalid.id], tasks: [invalid.id: invalid])
                XCTAssertThrowsError(try JSONDecoder().decode(MacCoreTaskProjection.self,
                    from: JSONEncoder().encode(corruptSnapshot)), missing)
            }
        }
    }
    func testTaskCancellationReceiptRequiresExactOwnerAndDurableFact() throws {
        let intent = AppleTaskCancellationIntent(hostId: "host", conversationId: "parent", parentRunId: "original-run", taskId: "task")
        let body: [String: Any] = ["hostId":"host", "conversationId":"parent", "parentRunId":"original-run", "taskId":"task",
            "cancelRequested":true, "terminal":false, "state":"running", "applicationPending":false]
        func decode(_ value: [String: Any]) throws -> AppleTaskCancellationReceipt {
            try JSONDecoder().decode(AppleTaskCancellationReceipt.self, from: JSONSerialization.data(withJSONObject:value))
        }
        XCTAssertNoThrow(try decode(body).validate(intent))
        for key in ["hostId", "conversationId", "parentRunId", "taskId"] {
            var changed = body; changed[key] = "foreign"
            XCTAssertThrowsError(try decode(changed).validate(intent))
        }
        var pending = body; pending["applicationPending"] = true
        XCTAssertThrowsError(try decode(pending).validate(intent))
        var falseTerminal = body; falseTerminal["terminal"] = true
        XCTAssertThrowsError(try decode(falseTerminal).validate(intent))
    }

    func testReconnectsFromLatestCursorAndStopsAtTerminalProjection() async {
        var requestedCursors: [Int64] = []
        var sourceCallCount = 0
        let terminalOutput = expectation(description: "terminal projection delivered")
        var outputs: [MacRemoteTaskObservationCoordinator.Output] = []
        let coordinator = MacRemoteTaskObservationCoordinator { conversationID, cursor in
            XCTAssertEqual(conversationID, "conversation-1")
            requestedCursors.append(cursor)
            sourceCallCount += 1
            if sourceCallCount == 1 {
                return Self.stream([
                    .init(
                        conversationID: conversationID,
                        cursor: 7,
                        projection: Self.projection(state: .running)
                    ),
                ])
            }
            return Self.stream([
                .init(
                    conversationID: conversationID,
                    cursor: 11,
                    projection: Self.projection(state: .sealed)
                ),
            ])
        }

        coordinator.start(conversationID: "conversation-1", taskID: "task-1") { output in
            outputs.append(output)
            if outputs.count == 2 {
                terminalOutput.fulfill()
            }
        }

        await fulfillment(of: [terminalOutput], timeout: 1)
        XCTAssertEqual(requestedCursors, [0, 7])
        XCTAssertEqual(outputs.count, 2)
    }

    func testTimeoutResumesAtLastCursorAndWaitsForTargetTerminalEvent() async {
        let terminal = expectation(description: "observed remote terminal event")
        var cursors: [Int64] = []
        var states: [MacCoreTaskState] = []
        let coordinator = MacRemoteTaskObservationCoordinator { owner, cursor in
            cursors.append(cursor)
            return AsyncThrowingStream { continuation in
                if cursor == 0 {
                    continuation.yield(.init(conversationID: owner, cursor: 3, projection: Self.projection(state: .running)))
                    continuation.finish(throwing: URLError(.timedOut))
                } else {
                    continuation.yield(.init(conversationID: owner, cursor: 4, projection: .init()))
                    continuation.yield(.init(conversationID: owner, cursor: 5, projection: Self.projection(state: .sealed)))
                    continuation.finish()
                }
            }
        }
        coordinator.start(conversationID: "conversation-1", taskID: "task-1") { event in
            guard case .projection(_, _, let projection) = event, let state = projection.tasks["task-1"]?.state else {
                return XCTFail("Expected a status for the observed task")
            }
            states.append(state)
            if state.isTerminal { terminal.fulfill() }
        }
        await fulfillment(of: [terminal], timeout: 3)
        XCTAssertEqual(cursors, [0, 3])
        XCTAssertEqual(states, [.running, .sealed])
    }

    func testUnauthorizedHostStopsObservation() async {
        let failed = expectation(description: "host denied access")
        var calls = 0
        let coordinator = MacRemoteTaskObservationCoordinator { _, _ in
            calls += 1
            return AsyncThrowingStream { $0.finish(throwing: MacRemoteHostError.http(403, "")) }
        }
        coordinator.start(conversationID: "conversation-1", taskID: "task-1") { event in
            if case .failure = event { failed.fulfill() }
        }
        await fulfillment(of: [failed], timeout: 1)
        XCTAssertEqual(calls, 1)
    }

    func testCancelEndsTheConversationSubscription() async {
        let subscriptionStarted = expectation(description: "subscription started")
        let subscriptionCancelled = expectation(description: "subscription cancelled")
        let coordinator = MacRemoteTaskObservationCoordinator { _, _ in
            AsyncThrowingStream { continuation in
                continuation.onTermination = { termination in
                    if case .cancelled = termination {
                        subscriptionCancelled.fulfill()
                    }
                }
                subscriptionStarted.fulfill()
            }
        }

        coordinator.start(conversationID: "conversation-1", taskID: "task-1") { _ in
            XCTFail("A cancelled idle subscription must not emit output")
        }
        await fulfillment(of: [subscriptionStarted], timeout: 1)
        coordinator.cancel(conversationID: "conversation-1")
        await fulfillment(of: [subscriptionCancelled], timeout: 1)
    }

    private static func stream(
        _ events: [MacRemoteTaskProjectionEvent]
    ) -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        AsyncThrowingStream { continuation in
            events.forEach { continuation.yield($0) }
            continuation.finish()
        }
    }

    private static func projection(state: MacCoreTaskState) -> MacCoreTaskProjection {
        let lineage = try! JSONDecoder().decode(MacCoreTaskLineage.self,
            from: JSONSerialization.data(withJSONObject: ["child_session_id": "child"]))
        let durability: MacCoreTaskDurability? = state == .sealed ? try! JSONDecoder().decode(MacCoreTaskDurability.self,
            from: JSONSerialization.data(withJSONObject: ["run_result": macRunResultFixture("answered", conversation: "child", run: "child-run"),
                "result_digest": String(repeating: "a", count: 64), "terminal_receipt_id": "receipt"])) : nil
        let task = MacCoreTaskHandle(
            id: "task-1",
            ownerSessionID: "conversation-1",
            ownerTurnID: "turn-1",
            kind: .agent,
            state: state,
            progress: nil,
            currentStep: nil,
            resultRef: nil,
            cancelRequested: false,
            lineage: lineage,
            durability: durability
        )
        return MacCoreTaskProjection(revision: 1, order: [task.id], tasks: [task.id: task])
    }
}
