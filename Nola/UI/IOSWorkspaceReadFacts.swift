// SPDX-License-Identifier: Apache-2.0
// A single Core snapshot supplies Today; no client-side permission discovery.
import Foundation

nonisolated struct IOSWorkspaceQueuedPlan: Identifiable, Sendable {
    let id: String
    let title: String
    let date: Date
}
nonisolated enum IOSWorkspaceReadFacts {
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
