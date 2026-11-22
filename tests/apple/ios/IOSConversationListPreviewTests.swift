// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSConversationListPreviewTests: XCTestCase {
    func testDirectoryPreservesExactKeyAndReportsAmbiguousUUIDsWithoutChangingEitherRecord() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.db"))
        let upper = "5DE71C3B-807E-4CCF-9977-B6A5CCA55D3A"
        let lower = upper.lowercased()
        try runtime.createConversation(id: lower, title: "CLI", settingsJSON: "{}", createdAtMs: 1, updatedAtMs: 1)
        let directory = try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime)
        XCTAssertEqual(directory.map(\.canonicalID), [lower])
        let window = try runtime.loadConversationProjectionWindowV1(id: lower, beforeOrdinal: nil, limit: 40, contentByteLimit: 512_000)
        XCTAssertEqual(window.projection.conversationID, lower)
        XCTAssertEqual(window.projection.title, "CLI")
        XCTAssertNil(try runtime.loadConversation(id: upper))
        try runtime.createConversation(id: upper, title: "iOS", settingsJSON: "{}", createdAtMs: 2, updatedAtMs: 2)
        XCTAssertThrowsError(try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime))
        XCTAssertEqual(try runtime.loadConversation(id: upper)?.title, "iOS")
        XCTAssertEqual(try runtime.loadConversation(id: lower)?.title, "CLI")
    }

    func testSearchReadsDraftAndExpertOfConversationOutsideLoadedRows() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let first = try store.createConversation(title: "First", language: nil)
        let second = try store.createConversation(title: "Second", language: nil)
        let drafts = FileConversationDraftRepository.scoped(to: runtime.databaseURL)
        try drafts.save(.init(text: "会议摘要"), owner: .conversation(second.id))
        let rooms = IOSAgentRoomService(runtime: runtime)
        let directory = try rooms.createAgent(name: "Reviewer", description: "", defaults: .object([:]), directory: rooms.directory())
        let expert = try XCTUnwrap(directory.agents.first { $0.name == "Reviewer" })
        _ = try rooms.joinMention(agentID: expert.id, expectedAgentRevision: expert.revision, conversationID: second.id.uuidString)

        let entries = try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime)
        let firstEntry = try XCTUnwrap(entries.first { $0.id == first.id })
        var cache = [first.id: IOSConversationListPreview(lastActiveAt: firstEntry.lastActiveAt, revision: firstEntry.revision, checkedOnVisit: 1)]
        let missing = IOSConversationListPreview.missing(in: entries, cache: cache, visit: 1)
        XCTAssertEqual(missing.map(\.id), [second.id])
        let entry = try XCTUnwrap(missing.first)
        let preview = await IOSConversationListPreview.read(entry, runtime: runtime, previous: nil, visit: 1)
        XCTAssertNil(preview.error)
        XCTAssertTrue(preview.matches("摘要"))
        XCTAssertTrue(preview.matches("reviewer"))
        XCTAssertFalse(preview.matches("unrelated"))
        XCTAssertTrue(preview.hasRoom)
        cache[entry.id] = preview
        XCTAssertTrue(IOSConversationListPreview.missing(in: entries, cache: cache, visit: 1).isEmpty)

        // Editing a draft leaves the conversation's Core revision unchanged.
        try drafts.save(.init(text: "另一份草稿"), owner: .conversation(second.id))
        XCTAssertEqual(Set(IOSConversationListPreview.missing(in: entries, cache: cache, visit: 2).map(\.id)),
                       Set([first.id, second.id]))
        let updated = await IOSConversationListPreview.read(entry, runtime: runtime, previous: preview, visit: 2)
        XCTAssertTrue(updated.matches("另一份"))
        XCTAssertFalse(updated.matches("摘要"))
        XCTAssertTrue(updated.hasCurrentMessage)

        var failedRead = updated
        failedRead.hasCurrentMessage = false
        failedRead.error = "interrupted"
        failedRead.message = "stale message after failed read"
        let retried = await IOSConversationListPreview.read(entry, runtime: runtime, previous: failedRead, visit: 3)
        XCTAssertTrue(retried.hasCurrentMessage)
        XCTAssertNil(retried.error)
        XCTAssertNotEqual(retried.message, failedRead.message)
    }
}
