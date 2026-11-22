// SPDX-License-Identifier: Apache-2.0
// Canonical parent-owned expert histories. Reading this projection never joins or executes.
import Foundation

nonisolated struct AppleExpertChildConversations: Codable, Equatable, Sendable {
    let version: Int
    let conversationID: String
    let roomID: String?
    let roomRevision: Int64?
    let children: [AppleExpertChildConversation]
    let hasMore: Bool
    let nextOffset: Int?

    enum CodingKeys: String, CodingKey {
        case version, children
        case conversationID = "conversation_id", roomID = "room_id", roomRevision = "room_revision"
        case hasMore = "has_more", nextOffset = "next_offset"
    }

    /// Find the exact frozen participant in pages from one Room revision.
    @MainActor static func child(for expert: AppleMainExpertSelectionEntry, parentID: String,
                      load: (Int) async throws -> Self) async throws -> (String, AppleExpertChildConversation) {
        var offset = 0
        var first: Self?
        while true {
            let page = try await load(offset)
            try page.validate(parentID: parentID, offset: offset)
            guard let room = page.roomID, first == nil || (first?.roomID == room && first?.roomRevision == page.roomRevision) else { throw CocoaError(.coderReadCorrupt) }
            if let child = page.children.first(where: { $0.conversationID == expert.contextConversationID && $0.participantID == expert.participantID
                    && $0.agentID == expert.agentID && ($0.agentRevision == expert.agentRevision || (!$0.isCurrent && $0.agentRevision == nil)) }) { return (room, child) }
            guard let next = page.nextOffset else { throw CocoaError(.fileNoSuchFile) }
            first = first ?? page; offset = next
        }
    }

    func validate(parentID: String, offset: Int) throws {
        func validID(_ value: String) -> Bool {
            !value.isEmpty && value.utf8.count <= 512 && value == value.trimmingCharacters(in: .whitespacesAndNewlines)
                && !value.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
        }
        guard version == 1, conversationID == parentID, validID(parentID),
              (roomID == nil) == (roomRevision == nil),
              roomID.map(validID) ?? true, roomRevision.map { $0 >= 0 } ?? true,
              children.isEmpty || roomID != nil,
              offset >= 0, Set(children.map(\.conversationID)).count == children.count,
              children.allSatisfy({ validID($0.conversationID) && $0.conversationID != parentID
                  && validID($0.participantID) && validID($0.agentID) && $0.revision >= 0
                  && ($0.agentRevision.map { $0 >= 0 } ?? true) && $0.createdAtMs >= 0
                  && $0.updatedAtMs >= 0 && $0.messageCount >= 0 }),
              (hasMore ? (nextOffset.map { $0 > offset } == true && !children.isEmpty) : nextOffset == nil) else {
            throw CocoaError(.coderReadCorrupt)
        }
    }
}

nonisolated struct AppleExpertChildConversation: Codable, Equatable, Identifiable, Sendable {
    let conversationID: String
    let revision: Int64
    let title: String
    let participantID: String
    let agentID: String
    let agentRevision: Int64?
    let isCurrent: Bool
    let active: Bool
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let messageCount: Int
    var id: String { conversationID }

    enum CodingKeys: String, CodingKey {
        case revision, title, active
        case conversationID = "conversation_id", participantID = "participant_id", agentID = "agent_id"
        case agentRevision = "agent_revision", isCurrent = "is_current"
        case createdAtMs = "created_at_ms", updatedAtMs = "updated_at_ms", messageCount = "message_count"
    }
}
