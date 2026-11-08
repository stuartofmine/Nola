// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacPermissionPresentationIdentityTests: XCTestCase {
    private func request(
        id: String = "permission", host: String = "host", client: String = "client",
        session: String = "session", nonce: String = "challenge", call: String? = "call",
        instructions: [MacPermissionInstruction] = [], reason: String = "Review",
        expiresAt: Int64? = 100
    ) -> MacRemotePermissionRequest {
        MacRemotePermissionRequest(id: id, allowConversation: true, protocolVersion: 4, nonce: nonce,
            hostID: host, clientID: client, sessionID: session, toolName: "write_file",
            arguments: #"{"path":"/workspace/example"}"#,
            permissionInstructions: instructions, callID: call, reason: reason,
            message: "Permission needed", expiresAt: expiresAt)
    }

    func testGuidanceAndPresentationMetadataDoNotReplaceInteractionIdentity() {
        let original = request()
        let updated = request(instructions: [
            MacPermissionInstruction(permissionRequestID: "permission", instruction: "Preview first")
        ], reason: "Updated explanation", expiresAt: 200)
        XCTAssertNotEqual(original, updated, "Authorization still compares complete envelopes.")
        XCTAssertEqual(original.presentationIdentity, updated.presentationIdentity)
    }

    func testReusedRequestIDCannotRetainStateAcrossOwnerOrChallengeChanges() {
        let identity = request().presentationIdentity
        let successors = [
            request(id: "next"), request(host: "other-host"),
            request(client: "other-client"), request(session: "other-session"),
            request(nonce: "new-challenge"), request(call: "other-call"), request(call: nil)
        ]
        for successor in successors {
            XCTAssertNotEqual(identity, successor.presentationIdentity)
        }
        XCTAssertEqual(Set(successors.map(\.presentationIdentity)).count, successors.count)
    }

    func testPresentationIdentityDoesNotChangeDecisionEnvelope() {
        let original = request()
        _ = original.presentationIdentity
        let decision = original.decision(allow: true)
        XCTAssertEqual(decision.requestID, original.id)
        XCTAssertEqual(decision.nonce, original.nonce)
        XCTAssertEqual(decision.hostID, original.hostID)
        XCTAssertEqual(decision.clientID, original.clientID)
        XCTAssertEqual(decision.sessionID, original.sessionID)
        XCTAssertTrue(decision.resolution.isAllowed)
    }
}
