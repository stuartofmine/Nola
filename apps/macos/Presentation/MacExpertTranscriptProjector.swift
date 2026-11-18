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
        contexts: [String: [MacChatMessage]],
        peerExchanges: [AppleExpertPeerView] = []) -> [MacRoomTranscriptRow] {
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
        let assistantRunCounts = Dictionary(grouping: parentMessages.filter { $0.role == .assistant }
            .compactMap(runID(for:)), by: { $0 }).mapValues(\.count)
        struct Reply {
            let parentID: String
            let peerIndex: Int
            let receiptID: String
            let row: MacRoomTranscriptRow
        }
        struct PeerExchangeReply {
            let sourceRunID: String
            let createdAtMs: Int64
            let peerIndex: Int
            let exchangeID: String
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
        let validReceipts = candidates.filter { childCounts[$0.row.id] == 1 }
        let receiptRowIDs = Set(validReceipts.map(\.row.id))
        let byParent = Dictionary(grouping: validReceipts, by: \.parentID)

        var exchangeCandidates: [PeerExchangeReply] = []
        for exchange in peerExchanges {
            guard exchange.version == 1,
                  exchange.parentConversationID == conversationID,
                  exchange.roomID == association.roomID,
                  exchange.source.isMainAgent,
                  exchange.source.context_conversation_id == conversationID,
                  assistantRunCounts[exchange.sourceRunID] == 1,
                  let peerIndex = participants.firstIndex(where: {
                      $0.id == exchange.target.participant_id
                          && $0.contextConversationID == exchange.target.context_conversation_id
                  }) else { continue }
            let peer = participants[peerIndex]
            let canonicalAssistantID = exchange.exchangeID + ":assistant"
            let rowID = MacRoomTranscriptRow.ID.expert(
                roomID: association.roomID,
                contextID: peer.contextConversationID,
                messageID: canonicalAssistantID
            )
            guard !receiptRowIDs.contains(rowID) else { continue }
            let matching = assistantsByContext[peer.contextConversationID]?[canonicalAssistantID] ?? []
            guard matching.count == 1, let message = matching.first,
                  message.journalReference == nil || (message.journalReference?.cursor.identity.assistant_message_id == canonicalAssistantID &&
                    message.journalReference?.cursor.identity.conversation_id == peer.contextConversationID &&
                    message.journalReference?.cursor.identity.run_id == exchange.exchangeID) else { continue }
            let senderName = exchange.targetDisplayName?.trimmingCharacters(in: .whitespacesAndNewlines)
            let resolvedName = (senderName?.isEmpty == false ? senderName : nil) ?? peer.displayName
            exchangeCandidates.append(.init(
                sourceRunID: exchange.sourceRunID,
                createdAtMs: exchange.createdAtMs,
                peerIndex: peerIndex,
                exchangeID: exchange.exchangeID,
                row: .init(
                    id: rowID,
                    message: message,
                    context: peer.contextConversationID,
                    name: resolvedName,
                    agentID: peer.agentID
                )
            ))
        }
        let exchangeChildCounts = Dictionary(grouping: exchangeCandidates, by: { $0.row.id }).mapValues(\.count)
        let exchangesBySourceRun = Dictionary(
            grouping: exchangeCandidates.filter { exchangeChildCounts[$0.row.id] == 1 },
            by: \.sourceRunID
        )

        return parentMessages.flatMap { parent -> [MacRoomTranscriptRow] in
            var result = [ordinary(parent)]
            if parent.role == .user, let canonicalID = parent.canonicalMessageID {
                result += (byParent[canonicalID] ?? []).sorted {
                    $0.peerIndex == $1.peerIndex ? $0.receiptID < $1.receiptID : $0.peerIndex < $1.peerIndex
                }.map(\.row)
            } else if parent.role == .assistant, let runID = runID(for: parent) {
                result += (exchangesBySourceRun[runID] ?? []).sorted {
                    if $0.createdAtMs != $1.createdAtMs { return $0.createdAtMs < $1.createdAtMs }
                    if $0.peerIndex != $1.peerIndex { return $0.peerIndex < $1.peerIndex }
                    return $0.exchangeID < $1.exchangeID
                }.map(\.row)
            }
            return result
        }
    }

    private static func runID(for message: MacChatMessage) -> String? {
        if let runID = message.journalReference?.cursor.identity.run_id, !runID.isEmpty {
            return runID
        }
        guard let canonicalID = message.canonicalMessageID,
              canonicalID.hasSuffix(":assistant") else { return nil }
        let stem = String(canonicalID.dropLast(":assistant".count))
        return stem.isEmpty ? nil : stem
    }
}
