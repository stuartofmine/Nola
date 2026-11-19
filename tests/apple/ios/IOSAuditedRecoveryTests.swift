// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSAuditedRecoveryTests: XCTestCase {
    private let key = "assistant.uploaded_documents"

    private func database() throws -> (URL, IOSApplicationRuntime) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-recovery-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("test.sqlite")
        return (database, try IOSApplicationRuntime(databaseURL: database))
    }

    func testUnrecognizedDocumentsSurviveMemorySaveAndRejectedDocumentEdit() throws {
        let (_, runtime) = try database()
        let original = try runtime.putProductSetting(key: key, valueJSON: #"{"future":["important"]}"#, updatedAtMs: 1)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let profile = store.loadProfile()
        XCTAssertFalse(profile.uploadedDocumentsReadable)
        let saved = try store.saveMemoryText(longTermMemory: "Keep me", userPreferences: "", expected: profile)
        XCTAssertThrowsError(try store.saveUploadedDocuments([], expected: saved))
        XCTAssertEqual(try runtime.productSetting(key: key), original)
    }

    func testUnknownDocumentFieldsAreRetainedForNewerClients() throws {
        let (_, runtime) = try database()
        let item = UserUploadedContextItem(name: "future", content: "text")
        var encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(item)) as? [String: Any])
        encoded["future_format"] = ["important": true]
        let original = try runtime.putProductSetting(key: key, valueJSON: String(decoding: try JSONSerialization.data(withJSONObject: [encoded]), as: UTF8.self), updatedAtMs: 1)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        XCTAssertFalse(store.loadProfile().uploadedDocumentsReadable)
        XCTAssertThrowsError(try store.saveUploadedDocuments([item], expected: store.loadProfile()))
        XCTAssertEqual(try runtime.productSetting(key: key), original)
    }

    func testDocumentEditorsRejectConflictsAndPreserveWhitespaceMetadataAndOtherFields() throws {
        let (db, runtime) = try database()
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let other = CanonicalChatMemoryProfileStore(applicationRuntime: try IOSApplicationRuntime(databaseURL: db))
        let first = store.loadProfile(), second = other.loadProfile()
        let item = UserUploadedContextItem(name: "source.py", content: "    return True\n\n", version: "2")
        let saved = try store.saveUploadedDocuments([item], expected: first)
        XCTAssertThrowsError(try other.saveUploadedDocuments([], expected: second))
        _ = try other.saveMemoryText(longTermMemory: "Concurrent memory", userPreferences: "", expected: second)
        let reopened = other.loadProfile()
        XCTAssertEqual(reopened.uploadedDocuments, [item])
        XCTAssertEqual(reopened.uploadedDocumentRevision, saved.uploadedDocumentRevision)
        XCTAssertEqual(reopened.longTermMemory, "Concurrent memory")
    }

    func testOverflowRetainsFirstThirtyNamesAndContentAfterReopen() throws {
        let (db, runtime) = try database()
        let root = db.deletingLastPathComponent()
        let existing = (1...29).map { UserUploadedContextItem(name: "\($0)", content: "prior") }
        let files = ["last.txt", "no-room.txt", "also-full.txt"].map { root.appendingPathComponent($0) }
        try "  end\n".write(to: files[0], atomically: true, encoding: .utf8)
        let imported = IOSContextFileImportPolicy.importFiles(files, existing: existing)
        XCTAssertEqual(imported.items.count, 30)
        XCTAssertEqual(Array(imported.items.prefix(29)), existing)
        XCTAssertEqual(imported.items.last?.content, "  end\n")
        XCTAssertTrue(imported.problems.joined().contains("no-room.txt"))
        XCTAssertTrue(imported.problems.joined().contains("also-full.txt"))
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        _ = try store.saveUploadedDocuments(imported.items, expected: store.loadProfile())
        let cold = CanonicalChatMemoryProfileStore(applicationRuntime: try IOSApplicationRuntime(databaseURL: db))
        XCTAssertEqual(cold.loadProfile().uploadedDocuments, imported.items)
    }

    func testPendingEditSurvivesReopenAndStatusCheckNeverSends() throws {
        let (db, runtime) = try database()
        let owner = UUID()
        let store = IOSPendingMessageEdit(runtime: runtime, conversationID: owner)
        func edit() -> AppleMessageEditSubmission {
            AppleMessageEditSubmission(operationID: UUID(), targetConversationID: UUID(), createdAtMs: 10, sourceConversationID: owner.uuidString,
                sourceMessageID: "user", expectedSourceRevision: 1, expectedMessageRevision: 1, text: "Edited")
        }
        let original = edit()
        try store.retain(original)
        let cold = IOSPendingMessageEdit(runtime: try IOSApplicationRuntime(databaseURL: db), conversationID: owner)
        let restored = try XCTUnwrap(cold.load())
        XCTAssertEqual(original, restored)
        XCTAssertEqual(try cold.runtime.editReceipt(restored), .absent)
        XCTAssertNil(try runtime.loadConversation(id: owner.uuidString))
        XCTAssertThrowsError(try cold.retain(edit()))
        try cold.clear(edit())
        XCTAssertEqual(try store.load(), original)
        try cold.clear(original)
        XCTAssertNil(try store.load())
    }
}
