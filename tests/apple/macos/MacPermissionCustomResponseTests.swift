// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacPermissionCustomResponseTests: XCTestCase {
    private func request(_ id: String = "pending", records: [MacPermissionInstruction] = []) -> MacRemotePermissionRequest {
        MacRemotePermissionRequest(
            id: id, allowConversation: true, protocolVersion: 1, nonce: "nonce-\(id)", hostID: "host", clientID: "client",
            sessionID: "conversation", toolName: "write_file", arguments: #"{"path":"/original","content":"same"}"#,
            permissionInstructions: records, reason: "Confirm", message: "Confirm", expiresAt: nil)
    }

    func testActualDecisionPayloadIsTypedInterventionWithoutAuthorizationOrChangedArguments() throws {
        let original = request()
        let decision = try original.instructionDecision("  Preview first.  ")
        let data = try JSONSerialization.data(withJSONObject: decision.payload)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let resolution = try XCTUnwrap(payload["resolution"] as? [String: Any])
        XCTAssertEqual(payload["request_id"] as? String, original.id)
        XCTAssertEqual(payload["nonce"] as? String, original.nonce)
        XCTAssertEqual(payload["session_id"] as? String, original.sessionID)
        XCTAssertEqual(resolution["type"] as? String, "intervention")
        XCTAssertEqual(resolution["intervention"] as? [String: String], ["type": "provide_instruction", "instruction": "Preview first."])
        XCTAssertNil(resolution["decision"])
        XCTAssertNil(resolution["grant"])
        XCTAssertNil(payload["input"])
        XCTAssertFalse(decision.resolution.isAllowed)
        XCTAssertEqual(original.arguments, #"{"path":"/original","content":"same"}"#)
        XCTAssertEqual(original.decision(allow: false).payload["resolution"] as? [String: String], ["type": "authorization", "decision": "deny"])
    }

    func testBudgetMatchesUTF8CountAndHistoryLimitWithoutDisablingExplicitDecisions() throws {
        XCTAssertThrowsError(try request().instructionDecision(" \n "))
        XCTAssertNoThrow(try request().instructionDecision(String(repeating: "界", count: 1365)))
        XCTAssertThrowsError(try request().instructionDecision(String(repeating: "界", count: 1366)))
        let record = MacPermissionInstruction(permissionRequestID: "old", instruction: "prior")
        XCTAssertNoThrow(try request(records: Array(repeating: record, count: 15)).instructionDecision("last"))
        let full = request(records: Array(repeating: record, count: 16))
        XCTAssertThrowsError(try full.instructionDecision("too many"))
        XCTAssertTrue(full.decision(allow: true).resolution.isAllowed)
        XCTAssertFalse(full.decision(allow: false).resolution.isAllowed)
        let long = MacPermissionInstruction(permissionRequestID: "prior", instruction: String(repeating: "x", count: 4096))
        XCTAssertThrowsError(try request(records: Array(repeating: long, count: 3)).instructionDecision(String(repeating: "x", count: 4096)))
    }

    func testComposerKeepsDraftOnFailureAndRetriesOnlyTheSameTypedGuidance() async {
        let composer = MacPermissionInstructionComposer()
        composer.text = "Preview first."
        var delivered: [String] = []
        await composer.submit(request: request()) { text in
            delivered.append(text)
            throw MacPermissionInstructionError.unsupported
        }
        XCTAssertEqual(composer.text, "Preview first.")
        XCTAssertNotNil(composer.error)
        XCTAssertNil(composer.submittedRequestID)
        XCTAssertFalse(composer.isSending)
        await composer.submit(request: request()) { delivered.append($0) }
        XCTAssertEqual(delivered, ["Preview first.", "Preview first."])
        XCTAssertEqual(composer.submittedRequestID, "pending")
        XCTAssertEqual(composer.text, "")
        XCTAssertNil(composer.error)
        await composer.submit(request: request()) { _ in XCTFail("Answered interaction cannot submit twice") }
    }

    func testInFlightComposerCannotSubmitTwiceOrOverwriteSuccessorCard() async {
        let old = MacPermissionInstructionComposer()
        old.text = "old guidance"
        var response: CheckedContinuation<Void, Never>?
        let sending = Task { await old.submit(request: request("old")) { _ in
            await withCheckedContinuation { response = $0 }
        } }
        while response == nil { await Task.yield() }
        XCTAssertTrue(old.isSending)
        await old.submit(request: request("old")) { _ in XCTFail("Duplicate guidance") }
        // The actual card is keyed by request.id; a successor owns fresh state.
        let successor = MacPermissionInstructionComposer()
        successor.text = "new draft"
        response?.resume()
        await sending.value
        XCTAssertEqual(old.submittedRequestID, "old")
        XCTAssertEqual(successor.text, "new draft")
        XCTAssertNil(successor.submittedRequestID)
        XCTAssertTrue(successor.canSubmit(request: request("new")))
    }
}
