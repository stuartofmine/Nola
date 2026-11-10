// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacRemoteSessionProjectionEventTests: XCTestCase {
    func testTerminalHintRejectsInvalidOrConflictingIdentity() throws {
        let valid: [String: Any] = ["type": "assistant_run_terminal_committed_v1",
            "session_id": "conversation", "source_host_id": "host", "run_id": "run",
            "seal_digest": String(repeating: "a", count: 64)]
        let invalidFields: [(String, Any)] = [
            ("session_id", "foreign"), ("source_host_id", ""), ("run_id", " "),
            ("seal_digest", String(repeating: "A", count: 64)), ("seal_digest", true),
            ("canonical_run_id", "another-run"),
        ]
        for (key, value) in invalidFields {
            var object = valid
            object[key] = value
            let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
            let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: line, conversationID: "conversation"))
            XCTAssertTrue(event.isError, key)
            XCTAssertNil(event.assistantRunTerminalSealDigest, key)
            XCTAssertNil(event.coreTurnTerminal, key)
        }
    }

    func testRemovedAggregateProtocolsCannotWritePresentationOrTerminalState() throws {
        for type in ["core_session_projection_updated", "conversation_projection_committed_v1",
                     "remote_assistant_projection_committed_v1"] {
            let data = try JSONSerialization.data(withJSONObject: ["type": type, "text": "untrusted"])
            let event = try XCTUnwrap(MacAgentHostRuntime.decode(
                line: String(decoding: data, as: UTF8.self), conversationID: "conversation"))
            XCTAssertTrue(event.isError)
            XCTAssertNil(event.assistantJournalCursor)
            XCTAssertNil(event.assistantRunTerminalSealDigest)
            XCTAssertNil(event.coreTurnTerminal)
            XCTAssertNil(event.assistantJournalCursor)
        }
    }
    func testHostModelProgressPreservesSummaryAsPresentationTitle() throws {
        let line = #"{"type":"tool_progress","trace":{"schema_version":1,"trace_id":"trace-1","run_id":"run-1","conversation_id":"conversation-1","message_id":"message-1","turn_id":"turn-1","event_id":"event-1","sequence":1,"emitted_at":"2026-09-13T00:00:00Z","kind":"tool.progress","span_id":"effect-1","parent_span_id":null,"status":"running","visibility":"summary","payload":{"phase":"working","summary":"model_retry","detail":"primary/model → fallback/model · quota exhausted","metadata":{"attempt":2,"target_exhausted":true,"failure_code":"quota_exhausted","failure_message":"The provider quota is exhausted."}}}}"#

        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(line: line, conversationID: "conversation-1")
        )

        XCTAssertEqual(event.kind, .status)
        XCTAssertEqual(event.title, "model_retry")
        XCTAssertEqual(event.detail, "primary/model → fallback/model · quota exhausted")
        XCTAssertEqual(event.trace?.payload.metadata?.objectValue?["attempt"]?.integerValue, 2)
        XCTAssertEqual(event.trace?.payload.metadata?.objectValue?["target_exhausted"]?.booleanValue, true)
        XCTAssertEqual(
            event.trace?.payload.metadata?.objectValue?["failure_message"]?.stringValue,
            "The provider quota is exhausted."
        )
    }

    func testCanonicalImportInvalidationDecodesWithConversationIdentity() throws {
        let wire = #"{"scope":{"type":"conversation","conversation_id":"history"},"cursor":{"scope":{"type":"conversation","conversation_id":"history"},"sequence":1},"identity":{"event_id":"import-event","conversation_id":"history"},"mutation":{"type":"conversation","mutation":{"type":"invalidate_projection","revision":2}},"occurred_at":"2026-09-08T00:00:00Z"}"#
        let event = try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(wire.utf8))
        XCTAssertEqual(event.mutation, .invalidateProjection(revision: 2))
        XCTAssertEqual(event.sequence, 1)
        XCTAssertEqual(event.conversationID, "history")
        let invalid = wire.replacingOccurrences(of: #""event_id":"import-event","conversation_id":"history""#,
            with: #""event_id":"import-event","conversation_id":"foreign""#)
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(invalid.utf8)))
    }

    private func userInputWaitLine(conversationID: String = "conversation-1", state: String = "open", revision: Int = 1) throws -> String {
        let object: [String: Any] = ["type": "user_input_wait_changed", "record": [
            "conversation_id": conversationID, "run_id": "run-1", "wait": [
                "wait_id": "input-1", "core_session_id": "core-session", "revision": revision,
                "response_schema_digest": String(repeating: "a", count: 64), "state": ["type": state],
                "question_set": ["questions": [["id": "workspace", "prompt": "Which workspace should I use?",
                    "options": [["id": "current", "label": "Current", "description": "Use the open workspace"],
                                ["id": "choose", "label": "Choose", "description": "Select another folder"]],
                    "allow_freeform": true, "required": true, "max_text_chars": 4096]]]]]]
        return String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }

    func testUserInputRequestDecodesAsADurableRunWait() throws {
        let line = try userInputWaitLine()

        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(line: line, conversationID: "conversation-1", coreSessionID: "core-session")
        )

        XCTAssertEqual(event.kind, .userInputRequested)
        XCTAssertFalse(event.isError)
        XCTAssertEqual(event.userInputRequest?.id, "input-1")
        XCTAssertEqual(event.userInputRequest?.runID, "run-1")
        XCTAssertEqual(event.userInputRequest?.questions.first?.id, "workspace")
        XCTAssertEqual(event.userInputRequest?.questions.first?.options.map(\.id), ["current", "choose"])
        XCTAssertEqual(event.userInputRequest?.questions.first?.options.last?.detail, "Select another folder")
        XCTAssertEqual(event.userInputRequest?.questions.first?.allowsFreeform, true)
        XCTAssertEqual(event.userInputRequest?.isOpen, true)
        XCTAssertNil(event.trace)
    }

    func testUserInputRequestRejectsAnotherConversation() throws {
        let line = try userInputWaitLine(conversationID: "conversation-2")

        let event = MacAgentHostRuntime.decode(line: line, conversationID: "conversation-1", coreSessionID: "core-session")

        XCTAssertEqual(event?.kind, .failed)
        XCTAssertTrue(event?.isError == true)
        XCTAssertNil(event?.userInputRequest)
    }

    func testClosedUserWaitIsDeliveredToClearPresentation() throws {
        let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: userInputWaitLine(state: "resolved", revision: 2),
            conversationID: "conversation-1", coreSessionID: "core-session"))
        XCTAssertEqual(event.userInputRequest?.isOpen, false)
        XCTAssertEqual(event.userInputRequest?.revision, 2)
        XCTAssertEqual(event.eventID, "user-input:input-1:2")
    }

    func testWaitSubmissionRetryMatchesTheOriginalTypedAnswer() throws {
        let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: userInputWaitLine(),
            conversationID: "conversation-1", coreSessionID: "core-session"))
        let wait = try XCTUnwrap(event.userInputRequest)
        let answer = MacUserInputAnswer(questionID: "workspace", optionID: "current", text: nil)
        let intent = MacUserInputSubmissionIntent(commandID: "stable-answer-command", request: wait,
            answers: [answer], decline: false)
        XCTAssertTrue(intent.matches(request: wait, answers: [answer], decline: false))
        XCTAssertFalse(intent.matches(request: wait, answers: [answer], decline: true))
        XCTAssertFalse(intent.matches(request: wait, answers: [
            .init(questionID: "workspace", optionID: "choose", text: nil)], decline: false))
        let encoded = try XCTUnwrap(answer.object["value"] as? [String: String])
        XCTAssertEqual(encoded, ["type": "option", "option_id": "current"])
    }

    func testUserWaitActivityHasItsOwnCanonicalState() throws {
        let state = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ActivityState.self,
            from: Data(#""waiting_for_input""#.utf8))
        XCTAssertEqual(state, .waitingForInput)
        XCTAssertNotEqual(state, .waitingForPermission)
    }

    func testCanonicalDynamicJSONDecoderRejectsExcessiveNestingWithoutCrashing() {
        var nested = "null"
        for _ in 0..<(MacCanonicalJSONValue.maximumNestingDepth + 8) {
            nested = "[\(nested)]"
        }

        XCTAssertThrowsError(
            try JSONDecoder().decode(
                MacCanonicalJSONValue.self,
                from: Data(nested.utf8)
            )
        )
    }


}
