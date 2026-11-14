// SPDX-License-Identifier: Apache-2.0
// Core-owned peer provenance; presentation must never infer authors from text.
import Foundation

/// New dispatches originate in the main conversation. Legacy expert references
/// are decoded only to preserve already-recorded history and recovery identity.
nonisolated enum AppleExpertTaskSource: Codable, Equatable, Sendable {
    case mainAgent(contextConversationID: String)
    case legacyExpert(AppleExpertParticipantReference)

    var context_conversation_id: String {
        switch self { case .mainAgent(let id): return id; case .legacyExpert(let ref): return ref.context_conversation_id }
    }
    var isMainAgent: Bool { if case .mainAgent = self { return true }; return false }
    var isValid: Bool {
        switch self {
        case .mainAgent(let id): return !id.isEmpty
        case .legacyExpert(let ref): return !ref.participant_id.isEmpty && !ref.agent_id.isEmpty && !ref.context_conversation_id.isEmpty && ref.agent_revision >= 0
        }
    }
    enum CodingKeys: String, CodingKey { case kind; case contextConversationID = "context_conversation_id" }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        if c.contains(.kind) {
            guard try c.decode(String.self, forKey: .kind) == "main_agent" else {
                throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "Unsupported expert dispatch source")
            }
            let id = try c.decode(String.self, forKey: .contextConversationID)
            guard !id.isEmpty else { throw DecodingError.dataCorruptedError(forKey: .contextConversationID, in: c, debugDescription: "Missing main conversation") }
            self = .mainAgent(contextConversationID: id)
        } else { self = .legacyExpert(try AppleExpertParticipantReference(from: decoder)) }
    }
    func encode(to encoder: Encoder) throws {
        switch self {
        case .mainAgent(let id):
            var c = encoder.container(keyedBy: CodingKeys.self)
            try c.encode("main_agent", forKey: .kind); try c.encode(id, forKey: .contextConversationID)
        case .legacyExpert(let ref): try ref.encode(to: encoder)
        }
    }
}

nonisolated struct AppleExpertPeerClarification: Codable, Equatable, Sendable {
    let messageID: String
    let source: AppleExpertParticipantReference
    let target: AppleExpertParticipantReference
    let text: String
    let createdAtMs: Int64
    enum CodingKeys: String, CodingKey {
        case source, target, text
        case messageID = "message_id", createdAtMs = "created_at_ms"
    }
}

nonisolated struct AppleExpertPeerView: Codable, Equatable, Identifiable, Sendable {
    let version: Int
    let exchangeID: String
    let roomID: String
    let parentConversationID: String
    let source: AppleExpertTaskSource
    let target: AppleExpertParticipantReference
    let sourceRunID: String
    let sourceCallID: String
    let sourceDisplayName: String?
    let targetDisplayName: String?
    let kind: String
    let text: String
    let inReplyTo: String?
    let rootExchangeID: String
    let hop: Int
    let createdAtMs: Int64
    let clarifications: [AppleExpertPeerClarification]
    var id: String { exchangeID }
    enum CodingKeys: String, CodingKey {
        case version, source, target, kind, text, hop, clarifications
        case exchangeID = "exchange_id", roomID = "room_id", parentConversationID = "parent_conversation_id"
        case sourceRunID = "source_run_id", sourceCallID = "source_call_id"
        case sourceDisplayName = "source_display_name", targetDisplayName = "target_display_name"
        case inReplyTo = "in_reply_to", rootExchangeID = "root_exchange_id", createdAtMs = "created_at_ms"
    }
    func displayName(for participant: AppleExpertParticipantReference) -> String? {
        let name = source == .legacyExpert(participant) ? sourceDisplayName : (participant == target ? targetDisplayName : nil)
        guard let normalized = name?.trimmingCharacters(in: .whitespacesAndNewlines), !normalized.isEmpty else { return nil }
        return normalized
    }
    func displayName(for source: AppleExpertTaskSource) -> String? {
        guard source == self.source, let name = sourceDisplayName?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else { return nil }
        return name
    }

}

nonisolated struct AppleExpertPeerExchanges: Codable, Equatable, Sendable {
    let version: Int
    let conversationID: String
    let exchanges: [AppleExpertPeerView]
    let hasMore: Bool
    let nextOffset: Int?
    enum CodingKeys: String, CodingKey {
        case version, exchanges
        case conversationID = "conversation_id", hasMore = "has_more", nextOffset = "next_offset"
    }
    func validate(contextID: String, offset: Int) throws {
        func valid(_ reference: AppleExpertParticipantReference) -> Bool {
            !reference.participant_id.isEmpty && !reference.agent_id.isEmpty
                && !reference.context_conversation_id.isEmpty && reference.agent_revision >= 0
        }
        guard version == 1, conversationID == contextID, offset >= 0,
              Set(exchanges.map(\.exchangeID)).count == exchanges.count,
              exchanges.allSatisfy({ $0.version == 1 && $0.kind == "request" && !$0.exchangeID.isEmpty
                  && ($0.source.context_conversation_id == contextID || $0.target.context_conversation_id == contextID)
                  && !$0.roomID.isEmpty && !$0.parentConversationID.isEmpty && !$0.rootExchangeID.isEmpty
                  && $0.source.isValid && valid($0.target) && $0.hop >= 0 && $0.createdAtMs >= 0
                  && $0.clarifications.allSatisfy { !$0.messageID.isEmpty && valid($0.source) && valid($0.target) && $0.createdAtMs >= 0 } }),
              hasMore ? (nextOffset == offset + exchanges.count && !exchanges.isEmpty) : nextOffset == nil else {
            throw CocoaError(.coderReadCorrupt)
        }
    }
}


/// A scoped canonical message window and its peer authors from one Core read.
nonisolated struct AppleExpertChildHistory<Window: Decodable & Sendable>: Decodable, Sendable {
    let version: Int
    let conversationID: String
    let childConversationID: String
    let roomID: String
    let roomRevision: Int64
    let sourceOwner: AppleConversationSourceOwner
    let window: Window
    let expertExchanges: [AppleExpertPeerView]
    enum CodingKeys: String, CodingKey {
        case version, window
        case conversationID = "conversation_id", childConversationID = "child_conversation_id"
        case roomID = "room_id", roomRevision = "room_revision", sourceOwner = "source_owner"
        case expertExchanges = "expert_exchanges"
    }
    func validate(parentID: String, childID: String) throws {
        guard version == 1, conversationID == parentID, childConversationID == childID,
              !roomID.isEmpty, roomRevision >= 0, sourceOwner != .unavailable,
              expertExchanges.allSatisfy({ $0.version == 1 && $0.parentConversationID == parentID && $0.roomID == roomID }) else {
            throw CocoaError(.coderReadCorrupt)
        }
    }
}

extension AppleExpertChildHistory: Equatable where Window: Equatable {}

nonisolated struct AppleExpertPeerInput: Codable, Equatable, Sendable {
    let sourceContextID: String
    let sourceRunID: String
    let callID: String
    let targetParticipantID: String
    let kind: String
    let text: String
    let inReplyTo: String?
    enum CodingKeys: String, CodingKey {
        case kind, text
        case sourceContextID = "source_context_id", sourceRunID = "source_run_id", callID = "call_id"
        case targetParticipantID = "target_participant_id", inReplyTo = "in_reply_to"
    }
}

nonisolated struct AppleExpertPeerClarificationInput: Codable, Equatable, Sendable {
    let sourceContextID: String
    let sourceRunID: String
    let callID: String
    let exchangeID: String
    let text: String
    enum CodingKeys: String, CodingKey {
        case text
        case sourceContextID = "source_context_id", sourceRunID = "source_run_id", callID = "call_id", exchangeID = "exchange_id"
    }
}

nonisolated struct AppleExpertPeerMaterial<JSON: JournalJSONValue>: Decodable, Sendable {
    let view: AppleExpertPeerView
    let entry: AppleExpertMessageEntry<JSON>
    let promptDefinitions: [AppleExpertPromptDefinition<JSON>]
    enum CodingKeys: String, CodingKey { case view, entry; case promptDefinitions = "prompt_definitions" }
}

/// Only emitted by common Task admission after validating the frozen expert target.
nonisolated struct AppleExpertTaskTargetV1: Decodable, Equatable, Sendable {
    let version: Int
    let taskID: String
    let exchangeID: String
    let parentConversationID: String
    let roomID: String
    let source: AppleExpertTaskSource
    let target: AppleExpertParticipantReference
    let sourceRunID: String
    let sourceCallID: String
    enum CodingKeys: String, CodingKey {
        case version, source, target
        case taskID = "task_id", exchangeID = "exchange_id", parentConversationID = "parent_conversation_id", roomID = "room_id"
        case sourceRunID = "source_run_id", sourceCallID = "source_call_id"
    }
    func validate(taskID: String, contextID: String, runID: String) throws {
        guard version == 1, self.taskID == taskID, exchangeID == taskID, exchangeID == runID,
              target.context_conversation_id == contextID, source.isValid,
              (!source.isMainAgent || source.context_conversation_id == parentConversationID), !parentConversationID.isEmpty, !roomID.isEmpty,
              !sourceRunID.isEmpty, !sourceCallID.isEmpty else { throw CocoaError(.coderReadCorrupt) }
    }
}
