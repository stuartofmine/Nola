// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class IOSAgentRoomHistoryTests: XCTestCase {
    func testCanonicalOwnerSurvivesReopenWithoutLeakingIntoOrdinaryHistoryOrClientEncoding() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-room-history-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("nola.sqlite")
        var roomID = ""
        var contextID = UUID()
        var ordinaryID = UUID()
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            ordinaryID = try store.createConversation(title: "Ordinary", language: nil).id
            let service = IOSAgentRoomService(runtime: runtime)
            let directory = try service.createAgent(name: "Research", description: "", defaults: .object([:]), directory: service.directory())
            let room = try service.join(agent: XCTUnwrap(directory.agents.first), room: service.create(title: "Discussion"))
            roomID = room.room.id
            contextID = try XCTUnwrap(UUID(uuidString: XCTUnwrap(room.room.participants.first).contextConversationID))
            let all = try store.getAllConversations()
            XCTAssertEqual(Set(all.map(\.id)), Set([ordinaryID, contextID]))
            XCTAssertEqual(all.filter(\.appearsInOrdinaryHistory).map(\.id), [ordinaryID])
            XCTAssertEqual(ChatManager.coldStartConversation(from: all,
                fallbackModel: nil, activeRun: { _ in nil })?.id, ordinaryID)
            let context = try XCTUnwrap(all.first { $0.id == contextID })
            XCTAssertEqual(context.agentRoomOwner?.roomID, roomID)
            XCTAssertEqual(context.agentRoomOwner?.agentID, directory.agents.first?.id)
            let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(context)) as? [String: Any])
            XCTAssertNil(encoded["agentRoomOwner"])
        }
        let reopened = try LocalConversationStore(applicationRuntime: IOSApplicationRuntime(databaseURL: database))
        let all = try reopened.getAllConversations()
        XCTAssertEqual(all.first { $0.id == contextID }?.agentRoomOwner?.roomID, roomID)
        XCTAssertEqual(all.filter(\.appearsInOrdinaryHistory).map(\.id), [ordinaryID])
    }

    func testMalformedOwnershipIsRejectedRatherThanExposedAsOrdinaryConversation() {
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read(NSNull()))
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read(["room_id": "invalid"]))
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read([
            "room_id": UUID().uuidString, "participant_id": UUID().uuidString,
            "agent_id": "agent", "old_alias": "unsupported"
        ]))
    }
}

extension IOSAgentRoomHistoryTests {
    func testChildDirectoryRetainsRejoinedHistoryAndDoesNotCreateUnopenedRoom() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-child-directory-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let service = IOSAgentRoomService(runtime: runtime)
        let parent = try store.createConversation(title: "Main conversation", language: nil)
        let untouched = try service.childConversations(conversationID: parent.id.uuidString)
        XCTAssertTrue(untouched.children.isEmpty); XCTAssertNil(untouched.roomID)
        XCTAssertNil(try service.forConversation(parent.id.uuidString), "Reading an unopened branch must not create a room")
        let agents = try service.createAgent(name: "Expert", description: "Original", defaults: .object([:]), directory: service.directory())
        let originalAgent = try XCTUnwrap(agents.agents.first)
        let oldRoom = try service.joinMention(agentID: originalAgent.id, expectedAgentRevision: originalAgent.revision,
            conversationID: parent.id.uuidString)
        let oldMember = try XCTUnwrap(oldRoom.room.room.participants.first)
        let updated = try service.updateAgent(originalAgent, name: "Expert", description: "Updated",
            availability: "active", directory: service.directory())
        let newAgent = try XCTUnwrap(updated.agents.first)
        let newRoom = try service.rejoinMention(agentID: newAgent.id, expectedAgentRevision: newAgent.revision,
            conversationID: parent.id.uuidString)
        let newMember = try XCTUnwrap(newRoom.room.room.participants.first)
        let first = try service.childConversations(conversationID: parent.id.uuidString, limit: 1)
        XCTAssertEqual(first.children.count, 1); XCTAssertTrue(first.hasMore)
        let second = try service.childConversations(conversationID: parent.id.uuidString, limit: 1, offset: try XCTUnwrap(first.nextOffset))
        let all = first.children + second.children
        XCTAssertEqual(Set(all.map(\.conversationID)), [oldMember.contextConversationID, newMember.contextConversationID])
        let old = try XCTUnwrap(all.first { $0.conversationID == oldMember.contextConversationID })
        let current = try XCTUnwrap(all.first { $0.conversationID == newMember.contextConversationID })
        XCTAssertFalse(old.isCurrent); XCTAssertTrue(current.isCurrent)
        XCTAssertEqual(current.agentRevision, newMember.agentRevision)
        XCTAssertEqual(try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime).map(\.id), [parent.id], "Neither old nor current expert context is top-level")
        XCTAssertEqual(try service.forConversation(parent.id.uuidString)?.revision, newRoom.revision, "Read-only child enumeration cannot mutate the association")
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: old.conversationID).activeRun)
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: current.conversationID).activeRun)
        let oldHistory = try service.childHistory(conversationID: parent.id.uuidString, childID: old.id)
        let currentHistory = try service.childHistory(conversationID: parent.id.uuidString, childID: current.id)
        XCTAssertEqual(oldHistory.window.projection.conversationID, old.id)
        XCTAssertEqual(currentHistory.sourceOwner, .local)
        XCTAssertTrue(currentHistory.expertExchanges.isEmpty)
        XCTAssertThrowsError(try service.childHistory(conversationID: UUID().uuidString, childID: current.id))
        let container = ServiceContainer(applicationRuntime: runtime)
        let currentID = try XCTUnwrap(UUID(uuidString: current.conversationID))
        XCTAssertNil(container.chatManager.executionForConversation(currentID))
        let selectedBeforeRead = container.chatManager.currentConversationId
        let readOnly = IOSAgentRoomWorkspace()
        let snapshot = IOSAgentRoomSnapshot(revision: newRoom.room.revision,
            room: .init(id: newRoom.roomID, title: "Current child", participants: [newMember]))
        try readOnly.installReadOnlyHistory(snapshot, window: currentHistory.window)
        XCTAssertNil(container.chatManager.executionForConversation(currentID), "Reading child history must not allocate an execution owner")
        XCTAssertEqual(container.chatManager.currentConversationId, selectedBeforeRead)
        readOnly.close()
    }
}
