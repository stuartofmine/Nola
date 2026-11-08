// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacAgentHostTransportGateTests: XCTestCase {
    func testGoalReplayReadsRunFromTraceWithoutUsingControlRequestIdentity() throws {
        let trace: [String: Any] = [
            "schema_version": 1, "trace_id": "goal-trace", "run_id": "goal-run-1",
            "conversation_id": "conversation-1", "message_id": "goal-run-1:assistant",
            "turn_id": "goal-run-1", "event_id": "seal-replay", "sequence": 22,
            "emitted_at": "2026-08-22T00:00:00Z", "kind": "phase.changed",
            "status": "running", "visibility": "user", "payload": ["phase": "processing"],
        ]
        let line = String(decoding: try JSONSerialization.data(withJSONObject: ["type": "phase", "trace": trace]), as: UTF8.self)
        XCTAssertEqual(MacAgentHostRuntime.goalEventRunID(line: line), "goal-run-1")
        XCTAssertNil(MacAgentHostRuntime.goalEventRunID(line: "{\"type\":\"status\"}"))
    }

    func testDuplicateEventIDIsDroppedBeforePresentation() throws {
        var gate = MacAgentHostTransportGate(
            runID: "run-1",
            assistantMessageID: "run-1:assistant"
        )
        let event = event(eventID: "event-1", sequence: 1)
        XCTAssertEqual(try gate.accept(event), .accepted)
        XCTAssertEqual(try gate.accept(event), .duplicate)
    }

    func testOutOfOrderEventFailsTheRun() throws {
        var gate = MacAgentHostTransportGate(
            runID: "run-1",
            assistantMessageID: "run-1:assistant"
        )
        XCTAssertEqual(try gate.accept(event(eventID: "event-2", sequence: 2)), .accepted)
        XCTAssertThrowsError(try gate.accept(event(eventID: "event-1", sequence: 1)))
    }

    func testForeignRunIsRejectedBeforeItCanOwnTheStream() throws {
        var gate = MacAgentHostTransportGate(
            runID: "run-expected",
            assistantMessageID: "run-expected:assistant"
        )
        XCTAssertThrowsError(
            try gate.accept(event(
                eventID: "foreign-event",
                sequence: 1,
                runID: "run-foreign",
                messageID: "run-expected:assistant"
            ))
        )
    }

    private func event(
        eventID: String,
        sequence: UInt64,
        runID: String = "run-1",
        messageID: String = "run-1:assistant"
    ) -> MacLocalAgentEvent {
        let trace = MacAgentTraceEnvelope.decode([
            "schema_version": 1,
            "trace_id": "trace-1",
            "run_id": runID,
            "conversation_id": "conversation-1",
            "message_id": messageID,
            "turn_id": runID,
            "event_id": eventID,
            "sequence": sequence,
            "emitted_at": "2026-08-22T00:00:00Z",
            "kind": "phase.changed",
            "status": "running",
            "visibility": "user",
            "payload": ["phase": "processing"],
        ])!
        return MacLocalAgentEvent(
            kind: .status,
            text: "delta",
            title: nil,
            detail: nil,
            isError: false,
            eventID: eventID,
            trace: trace
        )
    }
}
