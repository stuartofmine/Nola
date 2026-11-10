// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class IOSCanonicalMessageIdentityTests: XCTestCase {
    func testRecoveryUsesAdmittedOpaqueIdentityWithoutNewUserMessage() throws {
        let identity = try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: "ui-action:opaque",
            assistantMessageID: "admitted-assistant")
        XCTAssertEqual(identity.submissionID, "ui-action:opaque")
        XCTAssertEqual(identity.assistantMessageID, "admitted-assistant")
        XCTAssertEqual(identity.userMessageID, "")
        XCTAssertEqual(identity.visibleAssistantMessageID,
            IOSCanonicalMessageIdentity.stableUUID(for: "admitted-assistant"))
        for invalid in ["", " run", "run\n", String(repeating: "a", count: 513)] {
            XCTAssertThrowsError(try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: invalid,
                assistantMessageID: "assistant"))
        }
    }

    func testAssistantIdentityIsDerivedFromSubmission() throws {
        let userID = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
        XCTAssertEqual(identity.submissionID, userID.uuidString.lowercased())
        XCTAssertEqual(identity.assistantMessageID, "\(userID.uuidString.lowercased()):assistant")
        XCTAssertEqual(
            identity.visibleAssistantMessageID,
            IOSCanonicalMessageIdentity.stableUUID(for: identity.assistantMessageID)
        )
    }

    func testRandomVisibleAssistantIdentityIsRejected() {
        XCTAssertThrowsError(try IOSCanonicalAgentTurnIdentity(
            userMessageID: UUID(),
            validatingVisibleAssistantMessageID: UUID()
        ))
    }
}
