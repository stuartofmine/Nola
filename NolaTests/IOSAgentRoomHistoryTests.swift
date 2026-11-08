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
