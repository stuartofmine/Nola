// SPDX-License-Identifier: Apache-2.0
// A single Core snapshot supplies Today; no client-side permission discovery.
import Foundation

nonisolated struct IOSConversationDirectoryEntry: Identifiable, Sendable {
    let id: UUID
    let title: String
    let lastActiveAt: Date
    let knowledgePoints: String
    var sourceID: String? = nil
    var canonicalID: String { sourceID ?? id.uuidString }
}

nonisolated struct IOSWorkspaceQueuedPlan: Identifiable, Sendable {
    let id: String
    let title: String
    let date: Date
}
nonisolated enum IOSWorkspaceReadFacts {
    /// Directory surfaces never need message bodies, model credentials or
    /// per-source permission reads from the full ConversationStore projection.
    static func conversationDirectory(runtime: IOSApplicationRuntime) throws -> [IOSConversationDirectoryEntry] {
        var result: [IOSConversationDirectoryEntry] = []
        for summary in try runtime.listRootConversations() {
            try Task.checkCancellation()
            guard let id = UUID(uuidString: summary.id) else { continue }
            let metadata = try runtime.executeAgentCommand(.object([
                "type": .string("load_conversation_metadata"), "conversation_id": .string(summary.id)]))
            guard metadata != .null, case .object(let fields) = metadata else { continue }
            let settings = try IOSConversationSettingsObject.decode(fields["settings_json"]?.stringScalar)
            if let owner = settings["agentRoomOwner"] {
                _ = try ConversationAgentRoomOwner.read(owner)
                continue
            }
            result.append(.init(id: id, title: summary.title,
                lastActiveAt: Date(timeIntervalSince1970: Double(summary.updatedAtMs) / 1000),
                knowledgePoints: (settings["knowledge"] as? [String: Any])?["keyPointsMarkdown"] as? String ?? "", sourceID: summary.id))
        }
        return result.sorted { $0.lastActiveAt > $1.lastActiveAt }
    }

    private static func read<T: Decodable>(_ runtime: IOSApplicationRuntime,
        command: [String: AgentCoreJSONValue], as: T.Type) throws -> T {
        let value = try runtime.executeAgentCommand(.object(command))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    static func activity(runtime: IOSApplicationRuntime, query: AppleActivityQuery) throws -> AppleActivityPage {
        let input = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(query))
        return try read(runtime, command: ["type": .string("list_activity"), "input": input], as: AppleActivityPage.self)
    }
    static func activityDetail(runtime: IOSApplicationRuntime, entry: AppleActivityEntry) throws -> AppleActivityDetail? {
        try read(runtime, command: ["type": .string("load_activity_detail"),
            "conversation_id": .string(entry.conversation_id), "run_id": .string(entry.run_id),
            "assistant_message_id": .string(entry.assistant_message_id)], as: AppleActivityDetail?.self)
    }
    static func snapshot(runtime: IOSApplicationRuntime) throws -> AppleTodayFactsSnapshot<AgentCoreJSONValue> {
        try Task.checkCancellation()
        let value = try runtime.executeAgentCommand(.object([
            "type": .string("load_today_facts"),
            "input": .object([
                "conversation_offset": .signedInteger(0),
                "conversation_limit": .signedInteger(200),
                "queued_limit": .signedInteger(5),
            ]),
        ]))
        try Task.checkCancellation()
        return try JSONDecoder().decode(AppleTodayFactsSnapshot<AgentCoreJSONValue>.self,
            from: JSONEncoder().encode(value))
    }
}
