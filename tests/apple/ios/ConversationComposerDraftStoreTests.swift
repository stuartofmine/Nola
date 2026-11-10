import XCTest
@testable import Nola

final class ConversationComposerDraftStoreTests: XCTestCase {
    func testOldExpertAcknowledgementCannotClearNewIdenticalTextAfterOwnerRoundTrip() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a); store.current.text = "same message"
        let old = store.textSubmissionTicket()
        store.select(b); store.current.text = "B draft"
        store.select(a); store.current.text = ""; store.current.text = "same message"
        XCTAssertFalse(store.acknowledgeSubmittedText(old))
        XCTAssertEqual(store.current.text, "same message")
        store.select(b); XCTAssertEqual(store.current.text, "B draft")
    }

    func testAcknowledgementClearsOnlyItsExactUnchangedOwnerWhileAnotherDraftIsSelected() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a); store.current.text = "submitted A"
        let ticket = store.textSubmissionTicket()
        store.select(b); store.current.text = "unsent B"
        XCTAssertTrue(store.acknowledgeSubmittedText(ticket))
        XCTAssertEqual(store.current.text, "unsent B")
        store.select(a); XCTAssertEqual(store.current.text, "")
        XCTAssertFalse(store.acknowledgeSubmittedText(ticket), "An old acknowledgement cannot consume a later draft revision")
    }

    func testPersistenceFailureFollowsItsOwnerNotSelection() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a)
        store.current.text = "A"
        store.recordPersistenceFailure(owner: .conversation(a), detail: "A conflict")
        store.select(b)
        store.current.text = "B"
        XCTAssertNil(store.persistenceError)
        store.select(a)
        XCTAssertEqual(store.persistenceError, "A conflict")
        XCTAssertEqual(store.current.text, "A")
    }
    private func attachment(_ name: String) -> ChatAttachmentDraft {
        .init(name: name, typeLabel: "text", sizeBytes: 1, extractedContent: name)
    }

    func testPickerAndImportCompleteIntoOriginalConversationAfterSwitch() throws {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a)
        store.current.text = "A draft"
        let operation = try XCTUnwrap(store.beginImport())
        store.select(b)
        store.current.text = "B draft"
        let bOperation = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(operation, attachments: [attachment("A.txt")], notice: "A imported"))
        XCTAssertEqual(store.current.text, "B draft")
        XCTAssertTrue(store.current.attachments.isEmpty)
        XCTAssertEqual(store.current.importID, bOperation.id)
        XCTAssertNil(store.current.notice)
        store.select(a)
        XCTAssertEqual(store.current.text, "A draft")
        XCTAssertEqual(store.current.attachments.map(\.name), ["A.txt"])
        XCTAssertEqual(store.current.notice, "A imported")
        XCTAssertNil(store.current.importID)
    }

    func testDuplicateAndDeletedOwnerCannotBeResurrected() throws {
        var store = ConversationComposerDraftStore()
        let id = UUID()
        store.select(id)
        let old = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(old, attachments: [attachment("once")], notice: nil))
        XCTAssertFalse(store.finishImport(old, attachments: [attachment("twice")], notice: "stale"))
        let pending = try XCTUnwrap(store.beginImport())
        store.remove(id)
        XCTAssertFalse(store.finishImport(pending, attachments: [attachment("deleted")], notice: nil))
        XCTAssertNil(store.draft(for: .conversation(id)))
    }

    func testNewDraftSubmissionRetiresOldOperationsAndDoesNotRestoreSentText() throws {
        var store = ConversationComposerDraftStore()
        let unsent = store.selected
        store.current.text = "submitted"
        let operation = try XCTUnwrap(store.beginImport())
        let id = UUID()
        store.submitted(to: id)
        store.current.text = "next draft"
        store.select(id) // delayed selection publication must not reset it
        XCTAssertEqual(store.current.text, "next draft")
        XCTAssertNil(store.draft(for: unsent))
        XCTAssertFalse(store.finishImport(operation, attachments: [attachment("late")], notice: nil))
        store.select(nil)
        XCTAssertNotEqual(store.selected, unsent)
        XCTAssertEqual(store.current.text, "")
    }

    func testCancellationOnlyRetiresMatchingOperation() throws {
        var store = ConversationComposerDraftStore()
        let cancelled = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(cancelled, attachments: [], notice: nil))
        let replacement = try XCTUnwrap(store.beginImport())
        XCTAssertFalse(store.finishImport(cancelled, attachments: [], notice: "old error"))
        XCTAssertEqual(store.current.importID, replacement.id)
        store.cancelImports()
        XCTAssertFalse(store.accepts(replacement))
    }
}
