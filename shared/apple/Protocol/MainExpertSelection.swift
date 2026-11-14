// SPDX-License-Identifier: Apache-2.0
// A user selection guides one main Agent turn; it never starts expert execution.
import Foundation

nonisolated public struct AppleMainExpertSelectionEntry: Codable, Equatable, Hashable, Sendable {
    public let agentID: String
    public let agentRevision: Int64
    public let participantID: String?
    public let contextConversationID: String?
    public init(agentID: String, agentRevision: Int64, participantID: String? = nil, contextConversationID: String? = nil) {
        self.agentID = agentID; self.agentRevision = agentRevision
        self.participantID = participantID; self.contextConversationID = contextConversationID
    }
    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id", agentRevision = "agent_revision", participantID = "participant_id", contextConversationID = "context_conversation_id"
    }
}
nonisolated public struct AppleMainExpertSelectionContext: Codable, Equatable, Hashable, Sendable {
    public let version: Int
    public let selectedExperts: [AppleMainExpertSelectionEntry]
    public let expectedSourceStoreEpoch: String?
    public init(selectedExperts: [AppleMainExpertSelectionEntry], expectedSourceStoreEpoch: String? = nil) {
        version = 1; self.selectedExperts = selectedExperts; self.expectedSourceStoreEpoch = expectedSourceStoreEpoch
    }
    enum CodingKeys: String, CodingKey { case version; case selectedExperts = "selected_experts"; case expectedSourceStoreEpoch = "expected_source_store_epoch" }
    static func capture(agentIDs: Set<String>, observedRevisions: [String: Int64], joined: [AppleExpertParticipantReference], expectedEpoch: String? = nil) throws -> Self {
        guard Set(joined.map(\.agent_id)).count == joined.count else { throw CocoaError(.coderReadCorrupt) }
        let entries = try agentIDs.sorted().map { id in
            let member = joined.first { $0.agent_id == id }
            guard let revision = observedRevisions[id] ?? member?.agent_revision,
                  member == nil || member?.agent_revision == revision else { throw CocoaError(.coderReadCorrupt) }
            return AppleMainExpertSelectionEntry(agentID: id, agentRevision: revision,
                participantID: member?.participant_id, contextConversationID: member?.context_conversation_id)
        }
        let value = Self(selectedExperts: entries, expectedSourceStoreEpoch: expectedEpoch)
        try value.validate(); return value
    }

    public func validate() throws {
        guard version == 1, expectedSourceStoreEpoch?.isEmpty != true, !selectedExperts.isEmpty,
              Set(selectedExperts.map { $0.agentID.lowercased() }).count == selectedExperts.count,
              selectedExperts.allSatisfy({ !$0.agentID.isEmpty && $0.agentRevision >= 0
                  && (($0.participantID == nil && $0.contextConversationID == nil)
                      || ($0.participantID?.isEmpty == false && $0.contextConversationID?.isEmpty == false)) }) else {
            throw CocoaError(.coderReadCorrupt)
        }
    }
}
