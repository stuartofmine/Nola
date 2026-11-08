import Foundation
import XCTest
@testable import Nola

// Wire fixture only: production digest/source verification belongs to Store.
func macRunResultFixture(_ kind: String, conversation: String = "conversation", run: String = "run") -> [String: Any] {
    let value: [String: Any]
    switch kind {
    case "answered": value = ["kind": kind, "answer_part_refs": ["answer"]]
    case "action_completed": value = ["kind": kind, "requirement_acceptance_refs": ["assessment"]]
    case "incomplete": value = ["kind": kind, "reason": ["code": "unfinished", "detail": NSNull()], "remaining_requirement_refs": ["requirement"], "obligation_refs": []]
    case "blocked": value = ["kind": kind, "reason": ["code": "unavailable", "detail": NSNull()], "blocking_fact_refs": ["fact"], "resolution_condition": "Resource restored"]
    case "cancelled": value = ["kind": kind, "cancellation_id": "cancel", "observed_effect_refs": [], "unknown_effect_refs": []]
    case "failed": value = ["kind": kind, "error_ref": "error", "observed_effect_refs": [], "unknown_effect_refs": []]
    default: value = ["kind": kind]
    }
    return ["result_id": "result", "run_id": run, "conversation_id": conversation,
     "originating_submission_id": "submission", "input": ["watermark": "1", "applications_digest": String(repeating: "0", count: 64)],
     "decided_at_revision": "2", "decision_event_cursor": "3", "requirement_revision": "0", "evidence_frontier": "0",
     "value": value]
}

final class MacRunCancellationTests: XCTestCase {
    func testResultStructureRejectsContradictoryEvidenceAndDurableCounterOverflow() throws {
        let base: [String: Any] = ["result_id": "result", "run_id": "run", "conversation_id": "conversation",
            "originating_submission_id": "submission",
            "input": ["watermark": "1", "applications_digest": String(repeating: "a", count: 64)],
            "decided_at_revision": "2", "decision_event_cursor": "3", "requirement_revision": "0", "evidence_frontier": "0",
            "value": ["kind": "answered", "answer_part_refs": ["answer"]]]
        func decode(_ object: [String: Any]) throws -> MacRunResult {
            try JSONDecoder().decode(MacRunResult.self, from: JSONSerialization.data(withJSONObject: object))
        }
        _ = try decode(base)
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
    private func request() throws -> MacRunCancellationRequest {
        try .init(conversationID: "conversation", expectedRunID: "run",
            assistantMessageID: "assistant", cancellationID: "cancellation", requestedAtMs: 123)
    }
    private func receipt(_ disposition: String = "cancellation_committed", status: Any = NSNull()) -> [String: Any] {
        let result: Any
        let digest: Any
        if let kind = status as? String {
            result = macRunResultFixture(kind)
            digest = String(repeating: "0", count: 64)
        } else {
            result = NSNull()
            digest = NSNull()
        }
        return ["conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant",
         "cancellation_id": "cancellation", "disposition": disposition,
         "terminal_result": result, "result_digest": digest]
    }
    private func decode(_ object: [String: Any]) throws -> MacRunCancellationReceipt {
        try JSONDecoder().decode(MacRunCancellationReceipt.self,
            from: JSONSerialization.data(withJSONObject: object))
    }

    func testRequestEncodingHasOnlyExactCancellationFieldsAndExplicitNullReason() throws {
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request())) as? [String: Any])
        XCTAssertEqual(Set(object.keys), ["conversation_id", "expected_run_id", "assistant_message_id",
            "cancellation_id", "reason", "requested_at_ms"])
        XCTAssertTrue(object["reason"] is NSNull)
        XCTAssertEqual(object["requested_at_ms"] as? Int64, 123)
        XCTAssertEqual(object["expected_run_id"] as? String, "run")
    }

    func testRequestBoundsMatchSharedUtf8AndTimestampContract() throws {
        for identity in ["", " \n", String(repeating: "界", count: 342)] {
            XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: identity,
                expectedRunID: "run", assistantMessageID: "assistant", cancellationID: "id", requestedAtMs: 1))
        }
        for time: Int64 in [0, -1] {
            XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: "c",
                expectedRunID: "r", assistantMessageID: "a", cancellationID: "i", requestedAtMs: time))
        }
        XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: "c", expectedRunID: "r",
            assistantMessageID: "a", cancellationID: "i", reason: String(repeating: "界", count: 1366), requestedAtMs: 1))
        let exact = try MacRunCancellationRequest(conversationID: " c ", expectedRunID: "r",
            assistantMessageID: "a", cancellationID: String(repeating: "x", count: 1024),
            reason: String(repeating: "x", count: 4096), requestedAtMs: Int64.max)
        XCTAssertEqual(exact.conversationID, " c ", "do not silently normalize a canonical identity")
    }

    func testReceiptDispositionsPreserveTerminalHintWithoutInventingOne() throws {
        XCTAssertNil(try decode(receipt()).terminalResult)
        XCTAssertNil(try decode(receipt("already_cancelling")).terminalResult)
        for status in ["answered", "action_completed", "incomplete", "blocked", "failed", "cancelled"] {
            XCTAssertEqual(try decode(receipt("already_terminal", status: status)).terminalResult?.value.kind.rawValue, status)
            // Interrupt may finish immediately; committed does not mean only
            // pending, nor is its status a substitute for a canonical query.
            XCTAssertEqual(try decode(receipt(status: status)).terminalResult?.value.kind.rawValue, status)
        }
        XCTAssertThrowsError(try decode(receipt("already_terminal")))
        XCTAssertThrowsError(try decode(receipt("already_cancelling", status: "cancelled")))
    }

    func testReceiptRejectsMalformedShapeAndUnknownVariants() throws {
        var missing = receipt(); missing.removeValue(forKey: "terminal_result")
        XCTAssertThrowsError(try decode(missing))
        var extra = receipt(); extra["grant_workspace"] = true
        XCTAssertThrowsError(try decode(extra))
        var empty = receipt(); empty["run_id"] = " "
        XCTAssertThrowsError(try decode(empty))
        XCTAssertThrowsError(try decode(receipt("accepted")))
        XCTAssertThrowsError(try decode(receipt("already_terminal", status: "interrupted")))
        var wrongOwner = receipt("already_terminal", status: "answered")
        wrongOwner["terminal_result"] = macRunResultFixture("answered", run: "other")
        XCTAssertThrowsError(try decode(wrongOwner))
        var missingDigest = receipt("already_terminal", status: "answered")
        missingDigest["result_digest"] = NSNull()
        XCTAssertThrowsError(try decode(missingDigest))
    }

    func testReceiptRequiresAllFourOriginalIdentities() throws {
        let original = try request()
        XCTAssertNoThrow(try decode(receipt()).validated(for: original))
        for key in ["conversation_id", "run_id", "assistant_message_id", "cancellation_id"] {
            var other = receipt(); other[key] = "replacement"
            XCTAssertThrowsError(try decode(other).validated(for: original)) { error in
                XCTAssertEqual(error as? MacRunCancellationContractError, .identityMismatch)
            }
        }
    }
}
