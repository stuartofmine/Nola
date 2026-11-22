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
/// UTF-16 positions stay inside the native editor. The wire uses ordered text segments.
nonisolated public struct AppleExpertMentions: Codable, Equatable, Hashable, Sendable {
    public struct Segment: Codable, Equatable, Hashable, Sendable {
        public let text: String
        public let agentID: String?
        enum CodingKeys: String, CodingKey { case text; case agentID = "agent_id" }
    }
    public let version: Int
    public let segments: [Segment]
    public var selectedExperts: [AppleMainExpertSelectionEntry]? = nil
    enum CodingKeys: String, CodingKey { case version, segments; case selectedExperts = "selected_experts" }

    func runs(in text: String) -> [AppleDraftMention]? {
        guard version == 1, segments.map(\.text).joined() == text, let selectedExperts,
              !selectedExperts.isEmpty,
              Set(segments.compactMap(\.agentID)) == Set(selectedExperts.map(\.agentID)),
              (try? AppleMainExpertSelectionContext(selectedExperts: selectedExperts).validate()) != nil else { return nil }
        var runs: [AppleDraftMention] = []
        var offset = 0
        for segment in segments {
            if let id = segment.agentID {
                guard segment.text.hasPrefix("@"), let reference = selectedExperts.first(where: { $0.agentID == id }) else { return nil }
                runs.append(.init(location: offset, label: segment.text, expert: reference))
            }
            offset += segment.text.utf16.count
        }
        return runs
    }
}

nonisolated struct AppleDraftMention: Codable, Equatable, Sendable {
    var location: Int
    let label: String
    let expert: AppleMainExpertSelectionEntry
    var sourceID: String?
    var sourceEpoch: String?
    var range: NSRange { NSRange(location: location, length: label.utf16.count) }

    static func segments(_ mentions: [Self], in text: String) -> AppleExpertMentions? {
        guard !mentions.isEmpty else { return nil }
        let value = text as NSString
        var result: [AppleExpertMentions.Segment] = []
        var end = 0
        for mention in mentions.sorted(by: { $0.location < $1.location }) {
            let range = mention.range
            guard range.location >= end, range.location <= value.length,
                  range.length <= value.length - range.location,
                  mention.label.hasPrefix("@"), value.substring(with: range) == mention.label else { return nil }
            if end < range.location {
                result.append(.init(text: value.substring(with: NSRange(location: end, length: range.location - end)), agentID: nil))
            }
            result.append(.init(text: mention.label, agentID: mention.expert.agentID))
            end = NSMaxRange(range)
        }
        if end < value.length { result.append(.init(text: value.substring(from: end), agentID: nil)) }
        return AppleExpertMentions(version: 1, segments: result)
    }

    /// Preserve only untouched runs. Replacing a name with the same literal text removes its identity.
    static func replacing(_ mentions: [Self], range: NSRange, with replacement: String) -> [Self] {
        mentions.compactMap { mention in
            if NSMaxRange(mention.range) <= range.location { return mention }
            if mention.location >= NSMaxRange(range) {
                var next = mention
                next.location += replacement.utf16.count - range.length
                return next
            }
            return nil
        }
    }

    static func insertion(text: String, range: NSRange, name: String, expert: AppleMainExpertSelectionEntry,
                          mentions: [Self], sourceID: String?, epoch: String?) -> (text: String, mentions: [Self], caret: Int) {
        let label = "@" + name
        let replacement = label + " "
        let updated = (text as NSString).replacingCharacters(in: range, with: replacement)
        var runs = replacing(mentions, range: range, with: replacement)
        runs.append(.init(location: range.location, label: label, expert: expert, sourceID: sourceID, sourceEpoch: epoch))
        return (updated, runs.sorted { $0.location < $1.location }, range.location + replacement.utf16.count)
    }

    static func uniqueEntries(_ mentions: [Self]) -> [AppleMainExpertSelectionEntry]? {
        guard !mentions.isEmpty else { return nil }
        var seen = Set<String>()
        return mentions.compactMap { seen.insert($0.expert.agentID).inserted ? $0.expert : nil }
    }

    static func selected(_ mentions: [Self], sourceID: String?, epoch: String?) throws -> [AppleMainExpertSelectionEntry] {
        var result: [AppleMainExpertSelectionEntry] = []
        for mention in mentions {
            guard mention.sourceID == sourceID, mention.sourceEpoch == epoch else { throw CocoaError(.coderInvalidValue) }
            if let previous = result.first(where: { $0.agentID == mention.expert.agentID }) {
                guard previous == mention.expert else { throw CocoaError(.coderReadCorrupt) }
            } else { result.append(mention.expert) }
        }
        return result
    }
}

nonisolated public struct AppleMainExpertSelectionContext: Codable, Equatable, Hashable, Sendable {
    public let version: Int
    public let selectedExperts: [AppleMainExpertSelectionEntry]
    public let expertMentions: AppleExpertMentions?
    public let expectedSourceStoreEpoch: String?
    public init(selectedExperts: [AppleMainExpertSelectionEntry], expectedSourceStoreEpoch: String? = nil, expertMentions: AppleExpertMentions? = nil) {
        self.expertMentions = expertMentions
        version = 1; self.selectedExperts = selectedExperts; self.expectedSourceStoreEpoch = expectedSourceStoreEpoch
    }
    enum CodingKeys: String, CodingKey { case version; case expertMentions = "expert_mentions"; case selectedExperts = "selected_experts"; case expectedSourceStoreEpoch = "expected_source_store_epoch" }
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
