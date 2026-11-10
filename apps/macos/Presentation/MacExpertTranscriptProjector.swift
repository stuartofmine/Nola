// SPDX-License-Identifier: Apache-2.0
// Presentation only: Core parent order and exact Room receipt ownership.
import Foundation

nonisolated struct MacRoomTranscriptRow: Identifiable {
    enum ID: Hashable {
        case ordinary(conversationID: String, messageID: UUID)
        case expert(roomID: String, contextID: String, messageID: String)
    }
    let id: ID
    let message: MacChatMessage
    let context: String
    let name: String?
    let agentID: String?
}

nonisolated enum MacExpertTranscriptProjector {
    static func rows(conversationID: String, parentMessages: [MacChatMessage],
        association: MacConversationRoomAssociation?, receipts: [MacRoomSubmission],
        contexts: [String: [MacChatMessage]]) -> [MacRoomTranscriptRow] {
        func ordinary(_ message: MacChatMessage) -> MacRoomTranscriptRow {
            .init(id: .ordinary(conversationID: conversationID, messageID: message.id),
                message: message, context: conversationID, name: nil, agentID: nil)
        }
        guard !conversationID.isEmpty, let association,
              association.conversationID == conversationID,
              association.roomID == association.room.room.id else { return parentMessages.map(ordinary) }
        let participants = association.room.room.participants
        guard Set(participants.map(\.id)).count == participants.count,
              Set(participants.map(\.contextConversationID)).count == participants.count else {
            return parentMessages.map(ordinary)
        }
        let parentCounts = Dictionary(grouping: parentMessages.filter { $0.role == .user }
            .compactMap(\.canonicalMessageID), by: { $0 }).mapValues(\.count)
        struct Reply {
            let parentID: String
            let peerIndex: Int
            let receiptID: String
            let row: MacRoomTranscriptRow
        }
        let assistantsByContext = contexts.mapValues { messages in
            Dictionary(grouping: messages.filter { $0.role == .assistant && $0.canonicalMessageID != nil },
                by: { $0.canonicalMessageID! })
        }
        var candidates: [Reply] = []
        for receipt in receipts {
            guard let parentID = receipt.input.parentMessageID, parentCounts[parentID] == 1,
                  let peer = MacAgentRoomService.sender(for: receipt, association: association),
                  receipt.input.participantID == nil || receipt.input.participantID == peer.id,
                  let peerIndex = participants.firstIndex(where: { $0.id == peer.id }),
                  let executionID = receipt.executionID, !executionID.isEmpty else { continue }
            let canonicalAssistantID = executionID + ":assistant"
            let matching = assistantsByContext[peer.contextConversationID]?[canonicalAssistantID] ?? []
            guard matching.count == 1, let message = matching.first,
                  message.journalReference == nil || (message.journalReference?.cursor.identity.assistant_message_id == canonicalAssistantID &&
                    message.journalReference?.cursor.identity.conversation_id == peer.contextConversationID &&
                    message.journalReference?.cursor.identity.run_id == executionID) else { continue }
            candidates.append(.init(parentID: parentID, peerIndex: peerIndex, receiptID: receipt.id,
                row: .init(id: .expert(roomID: association.roomID, contextID: peer.contextConversationID,
                    messageID: canonicalAssistantID), message: message, context: peer.contextConversationID,
                    name: peer.displayName, agentID: peer.agentID)))
        }
        // Two conflicting receipts cannot assign one child to two parents.
        let childCounts = Dictionary(grouping: candidates, by: { $0.row.id }).mapValues(\.count)
        let byParent = Dictionary(grouping: candidates.filter { childCounts[$0.row.id] == 1 }, by: \.parentID)
        return parentMessages.flatMap { parent -> [MacRoomTranscriptRow] in
            var result = [ordinary(parent)]
            guard parent.role == .user, let canonicalID = parent.canonicalMessageID else { return result }
            result += (byParent[canonicalID] ?? []).sorted {
                $0.peerIndex == $1.peerIndex ? $0.receiptID < $1.receiptID : $0.peerIndex < $1.peerIndex
            }.map(\.row)
            return result
        }
    }
}
