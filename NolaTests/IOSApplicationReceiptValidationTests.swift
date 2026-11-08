// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSApplicationReceiptValidationTests: XCTestCase {
    func testReceiptFailureDiagnosticContainsShapeNotPayloadOrIdentity() throws {
        let secret = "private-user-text-and-credential"
        let response: [String: Any] = ["ok": false, "error": ["message": secret],
            "result": ["request_id": secret, "client_id": secret, "status": secret,
                       "result": ["content": secret]]]
        let input: [String: Any] = ["request_id": "original", "client_id": "ios-application",
            "command": ["type": "reduce_and_commit_active_submission", "arguments": ["text": secret]]]
        let diagnostic = IOSApplicationRuntime.receiptFailureDiagnostic(
            try JSONSerialization.data(withJSONObject: response), input: input)
        XCTAssertTrue(diagnostic.contains("command=reduce_and_commit_active_submission"))
        XCTAssertTrue(diagnostic.contains("outer_ok=false"))
        XCTAssertTrue(diagnostic.contains("request_matches=false"))
        XCTAssertFalse(diagnostic.contains(secret))
        XCTAssertFalse(diagnostic.contains("original"))
    }

    private let input: [String: Any] = ["protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
        "client_id": "ios-application", "request_id": "original"]
    private func receipt(_ status: String) -> [String: Any] {
        var value: [String: Any] = ["protocol_version": IOSApplicationRuntime.expectedServiceProtocol, "client_id": "ios-application",
            "request_id": "original", "received_at": "2026-10-12T00:00:00Z", "status": status]
        if status != "accepted" { value["completed_at"] = "2026-10-12T00:00:01Z" }
        return value
    }
    private func decode(_ value: [String: Any]) throws -> [String: Any] {
        try IOSApplicationRuntime.decodeApplicationResponse(
            JSONSerialization.data(withJSONObject: value), input: input)
    }
    func testOuterNonrecoverableFailureIsNotRejectedReceipt() {
        XCTAssertThrowsError(try decode(["ok": false, "error": ["recoverable": false,
            "code": "internal", "message": "failure"]])) { error in
            guard case IOSApplicationRuntimeError.outcomePending("original") = error else {
                return XCTFail("Outer error retired the intent")
            }
        }
    }
    func testObservationFailurePreservesCoreErrorWithoutInventingPendingMutation() throws {
        var query = input
        query["command"] = ["type": "load_conversation_projection_window_v1", "arguments": [:]]
        let message = "decode canonical Core checkpoint failed: missing field control_generation"
        let bytes = try JSONSerialization.data(withJSONObject: ["ok": false,
            "error": ["code": "application_rejected", "message": message, "recoverable": false]])
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(bytes, input: query)) { error in
            guard case IOSApplicationRuntimeError.rejected(let detail) = error else {
                return XCTFail("Read failure was presented as a pending mutation")
            }
            XCTAssertEqual(detail, message)
        }
        let accepted = try JSONSerialization.data(withJSONObject: ["ok": true, "result": receipt("accepted")])
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(accepted, input: query)) { error in
            guard case IOSApplicationRuntimeError.invalidResponse = error else {
                return XCTFail("Read-only query cannot have durable accepted status")
            }
        }
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(Data("{}".utf8), input: query)) { error in
            guard case IOSApplicationRuntimeError.invalidResponse = error else {
                return XCTFail("Malformed read response was presented as accepted")
            }
        }
    }

    func testOnlyCompleteCanonicalOutcomesValidate() throws {
        var committed = receipt("committed")
        committed["result"] = NSNull()
        XCTAssertEqual(try decode(["ok": true, "result": committed])["status"] as? String, "committed")
        var rejected = receipt("rejected")
        rejected["error"] = ["code": "denied", "message": "denied", "recoverable": false]
        XCTAssertNoThrow(try decode(["ok": true, "result": rejected]))
        var conflict = receipt("conflict")
        conflict["conflict"] = ["scope_id": "run", "expected_revision": 1, "actual_revision": 2]
        XCTAssertNoThrow(try decode(["ok": true, "result": conflict]))
        XCTAssertNoThrow(try decode(["ok": true, "result": receipt("accepted")]))
    }
    func testMalformedTerminalReceiptsRemainPending() {
        for status in ["committed", "rejected", "conflict", "unknown"] {
            XCTAssertThrowsError(try decode(["ok": true, "result": receipt(status)]))
        }
        var rejected = receipt("rejected")
        rejected["error"] = ["message": "not enough"]
        XCTAssertThrowsError(try decode(["ok": true, "result": rejected]))
        var conflict = receipt("conflict")
        conflict["conflict"] = ["scope_id": "run", "expected_revision": -1, "actual_revision": 2]
        XCTAssertThrowsError(try decode(["ok": true, "result": conflict]))
        conflict["conflict"] = ["scope_id": "run", "expected_revision": true, "actual_revision": 2]
        XCTAssertThrowsError(try decode(["ok": true, "result": conflict]))
    }
    func testWrongIdentityMissingTimestampAndZeroRevisionRemainPending() {
        var good = receipt("committed")
        good["result"] = [:] as [String: String]
        for key in ["request_id", "client_id", "received_at", "completed_at"] {
            var missing = good
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(["ok": true, "result": missing]))
        }
        var wrong = good
        wrong["request_id"] = "other"
        XCTAssertThrowsError(try decode(["ok": true, "result": wrong]))
        wrong = good
        wrong["committed_revision"] = 0
        XCTAssertThrowsError(try decode(["ok": true, "result": wrong]))
    }

    func testRecoveryQueriesOriginalReceiptBeforeAnyReplay() throws {
        var calls = 0
        var stored = receipt("committed")
        stored["result"] = NSNull()
        let recovered = try IOSApplicationRuntime.recoverApplicationEnvelope(input) { query in
            calls += 1
            XCTAssertEqual((query["command"] as? [String: Any])?["type"] as? String, "command_receipt")
            var response = self.receipt("committed")
            response["request_id"] = query["request_id"]
            response["result"] = stored
            return response
        }
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(recovered["request_id"] as? String, "original")
    }

    func testOnlyExplicitMissingReceiptPermitsExactReplay() throws {
        var calls: [[String: Any]] = []
        _ = try IOSApplicationRuntime.recoverApplicationEnvelope(input) { request in
            calls.append(request)
            var response = self.receipt("committed")
            response["request_id"] = request["request_id"]
            response["result"] = NSNull()
            return response
        }
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[1] as NSDictionary, input as NSDictionary)
        calls = []
        XCTAssertThrowsError(try IOSApplicationRuntime.recoverApplicationEnvelope(input) { request in
            calls.append(request)
            throw IOSApplicationRuntimeError.outcomePending("lookup")
        })
        XCTAssertEqual(calls.count, 1)
    }
}
