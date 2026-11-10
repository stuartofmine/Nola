// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacConversationDraftRepositoryTests: XCTestCase {
    private func save(_ record: MacConversationDraftRecord, to repository: MacConversationDraftRepository) async -> Bool {
        await withCheckedContinuation { continuation in repository.save(record) { continuation.resume(returning: $0) } }
    }

    func testDraftRestoresFromSameStoreAndDoesNotCrossDatabaseScopes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("draft-repository-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let firstPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("one.sqlite"))
        let repository = MacConversationDraftRepository(store: firstPort)
        let record = MacConversationDraftRecord(conversationID: "conversation-a", text: "未提交的草稿", attachments: [])
        let saved = await save(record, to: repository)
        XCTAssertTrue(saved)
        let restored = try await MacConversationDraftRepository(store: firstPort).load("conversation-a")
        XCTAssertEqual(restored?.text, record.text)
        let otherPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("two.sqlite"))
        let unrelated = try await MacConversationDraftRepository(store: otherPort).load("conversation-a")
        XCTAssertNil(unrelated)
    }

    func testStaleWindowCannotOverwriteDraftAndDeleteIsOrdered() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("draft-conflict-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.sqlite"))
        let first = MacConversationDraftRepository(store: port), second = MacConversationDraftRepository(store: port)
        _ = try await first.load("a"); _ = try await second.load("a")
        let saved = await save(.init(conversationID: "a", text: "first", attachments: []), to: first)
        let conflicted = await save(.init(conversationID: "a", text: "stale", attachments: []), to: second)
        XCTAssertTrue(saved); XCTAssertFalse(conflicted)
        let current = try await first.load("a"); XCTAssertEqual(current?.text, "first")
        let deleted: Bool = await withCheckedContinuation { continuation in first.delete("a") { continuation.resume(returning: $0) } }
        XCTAssertTrue(deleted)
        do { _ = try await first.load("a"); XCTFail("Deleted owners must not hydrate") } catch { }
        let resurrected = await save(.init(conversationID: "a", text: "stale after delete", attachments: []), to: second)
        XCTAssertFalse(resurrected)
    }

    func testDraftReferencesDoNotAcceptRemoteURLs() {
        let record = MacConversationDraftRecord(conversationID: "a", text: "", attachments: [
            .init(url: URL(string: "https://example.com/private")!, text: "", method: "", byteCount: 0, ready: false)
        ])
        XCTAssertThrowsError(try record.validate())
    }

    func testDraftSaveAndDeleteNeverEnterCanonicalSyncOutbox() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("draft-local-only-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.sqlite"))
        let before = try port.pendingSyncChanges()
        let repository = MacConversationDraftRepository(store: port)
        let browser = MacConversationBrowserSnapshot(target: "managed", sessionID: "display-only-session",
            pages: [], currentTitle: "Local page", currentURL: "https://example.com/local-page",
            isConnected: false, updatedAt: .now, previewURL: nil)
        let saved = await save(.init(conversationID: "private", text: "private unsent content", attachments: [],
            browserSnapshot: browser), to: repository)
        XCTAssertTrue(saved)
        let reopened = try await MacConversationDraftRepository(store: port).load("private")
        XCTAssertEqual(reopened?.browserSnapshot, browser)
        XCTAssertEqual(try port.pendingSyncChanges(), before)
        XCTAssertNil(try port.loadConversationMetadata("private"))
        XCTAssertNil(try port.productSetting(key: "macos.conversation-draft." + Data("private".utf8).base64EncodedString()))
        let deleted: Bool = await withCheckedContinuation { continuation in
            repository.delete("private") { continuation.resume(returning: $0) }
        }
        XCTAssertTrue(deleted)
        XCTAssertEqual(try port.pendingSyncChanges(), before)
        let freshRepository = MacConversationDraftRepository(store: port)
        let resurrected = await save(.init(conversationID: "private", text: "late", attachments: []), to: freshRepository)
        XCTAssertFalse(resurrected)
    }
}
