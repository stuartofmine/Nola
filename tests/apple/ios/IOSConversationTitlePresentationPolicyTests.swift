// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class IOSConversationTitlePresentationPolicyTests: XCTestCase {
    func testRuntimeIdentifiersUseStableNewConversationLabel() {
        let id = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        XCTAssertEqual(
            IOSConversationTitlePresentationPolicy.displayTitle(
                id,
                conversationID: id,
                fallback: "New conversation"
            ),
            "New conversation"
        )
        XCTAssertEqual(
            IOSConversationTitlePresentationPolicy.displayTitle(
                "Session 01DAF2BB",
                conversationID: id,
                fallback: "New conversation"
            ),
            "New conversation"
        )
        let generated = String(
            format: String(localized: "Conversation %@"),
            String(id.prefix(8))
        )
        XCTAssertEqual(
            IOSConversationTitlePresentationPolicy.displayTitle(
                generated,
                conversationID: id,
                fallback: "New conversation"
            ),
            "New conversation"
        )
    }

    func testUserTitlesWithSimilarWordsRemainVisible() {
        let id = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        XCTAssertEqual(
            IOSConversationTitlePresentationPolicy.displayTitle(
                "Session planning",
                conversationID: id,
                fallback: "New conversation"
            ),
            "Session planning"
        )
        XCTAssertEqual(
            IOSConversationTitlePresentationPolicy.displayTitle(
                "Conversation design review",
                conversationID: id,
                fallback: "New conversation"
            ),
            "Conversation design review"
        )
    }
}
