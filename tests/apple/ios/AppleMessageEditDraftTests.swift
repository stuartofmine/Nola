// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class AppleMessageEditDraftTests: XCTestCase {
    func testEditRetainsOriginalOwnerAndStableRetryIdentity() throws {
        var draft = AppleMessageEditDraft(sourceConversationID: "original", sourceMessageID: "user",
            expectedSourceRevision: 4, expectedMessageRevision: 7, text: "Original")
        draft.text = "Edited"
        let first = try XCTUnwrap(draft.preparation(title: "New"))
        let retry = try XCTUnwrap(draft.preparation(title: "New"))
        XCTAssertEqual(draft.originalText, "Original")
        XCTAssertEqual(first.source_conversation_id, "original")
        XCTAssertEqual(first.expected_source_revision, 4)
        XCTAssertEqual(first.expected_source_message_revision, 7)
        XCTAssertEqual(first.edited_text, "Edited")
        XCTAssertEqual(first.created_at_ms, retry.created_at_ms)
        XCTAssertEqual(first.operation_id, retry.operation_id)
        XCTAssertEqual(first.target_conversation_id, retry.target_conversation_id)
        XCTAssertNotEqual(first.target_conversation_id, first.source_conversation_id)
    }
    func testChangedInputUsesANewIntentWithoutChangingSourceWitness() {
        var draft = AppleMessageEditDraft(sourceConversationID: "source", sourceMessageID: "user",
            expectedSourceRevision: 4, expectedMessageRevision: 7, text: "First")
        let first = draft.operationID
        let target = draft.targetConversationID
        draft.text = "Second"
        XCTAssertNotEqual(draft.operationID, first)
        XCTAssertNotEqual(draft.targetConversationID, target)
        XCTAssertEqual(draft.sourceConversationID, "source")
        XCTAssertEqual(draft.expectedSourceRevision, 4)
        let second = draft.operationID
        draft.text = "Second"
        XCTAssertEqual(draft.operationID, second)
    }

    func testMissingSourceRevisionCannotBecomePreparedInput() {
        let draft = AppleMessageEditDraft(sourceConversationID: "original", sourceMessageID: "user",
            expectedSourceRevision: nil, expectedMessageRevision: 7, text: "Edited")
        XCTAssertFalse(draft.hasSourceWitness)
        XCTAssertNil(draft.preparation(title: "New"))
    }
}
