// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacPermissionInstructionTests: XCTestCase {
    func testOnceOnlyEligibilityIsRequiredAndSurvivesNativeEnvelopeDecoding() throws {
        var object: [String: Any] = ["type":"permission_request", "protocol_version":1,
            "allow_conversation":false,"request_id":"opaque","nonce":"nonce","host_id":"host",
            "client_id":"client","session_id":"conversation","tool_name":"opaque-executor","args":[:]]
        func decode() throws -> MacLocalAgentEvent? {
            MacAgentHostRuntime.decode(line:String(decoding:try JSONSerialization.data(withJSONObject:object),as:UTF8.self),
                conversationID:"conversation",controlSessionID:"conversation")
        }
        XCTAssertFalse(try XCTUnwrap(decode()?.permissionRequest).allowConversation)
        object.removeValue(forKey:"allow_conversation")
        XCTAssertNil(try decode()?.permissionRequest,"Missing eligibility must not become a reusable offer")
        object["allow_conversation"] = false
        object["protocol_version"] = 3
        XCTAssertNil(try decode()?.permissionRequest,"Old permission protocol must fail closed")
    }

    func testReaskedEnvelopePreservesGuidanceAndOriginalArguments() throws {
        let object: [String: Any] = [
            "type": "permission_request", "allow_conversation": true, "protocol_version": 1, "request_id": "reasked", "nonce": "new-nonce",
            "host_id": "host", "client_id": "client", "session_id": "conversation",
            "tool_name": "write_file", "args": ["path": "/original"],
            "permission_instructions": [["permission_request_id": "original", "instruction": "Use a preview <first>."]]
        ]
        let line = String(data: try JSONSerialization.data(withJSONObject: object), encoding: .utf8)!
        let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: line, conversationID: "conversation",
                                                          controlSessionID: "conversation"))
        let request = try XCTUnwrap(event.permissionRequest)
        XCTAssertEqual(request.id, "reasked")
        XCTAssertEqual(request.permissionInstructions.first?.permissionRequestID, "original")
        XCTAssertEqual(request.permissionInstructions.first?.instruction, "Use a preview <first>.")
        XCTAssertTrue(request.arguments.contains("/original"))
        XCTAssertFalse(request.arguments.contains("preview"))
        XCTAssertEqual(request.decision(allow: true).payload["resolution"] as? [String: String], ["type": "authorization", "decision": "allow_once"])
    }

    func testMissingMetadataRemainsCompatibleAndMalformedMetadataFails() {
        XCTAssertEqual(MacPermissionInstruction.decode(nil), [])
        for value: Any in [NSNull(), "text", [["permission_request_id": "old", "instruction": 1]], [["permission_request_id": "old", "instruction": " "]], [["permission_request_id": "old", "instruction": "text", "grant": true]]] {
            XCTAssertNil(MacPermissionInstruction.decode(value))
        }
    }
}
