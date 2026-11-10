// SPDX-License-Identifier: Apache-2.0
// Typed iOS port for remote/API conversation turns owned by Application.

import Foundation

nonisolated struct IOSRemoteConversationTurnDescriptor: Sendable {
    let sourceHostID: String
    let clientID: String
    let conversationID: String
    let conversationTitle: String
    let identity: IOSCanonicalAgentTurnIdentity
    let userText: String
    let userContentJSON: String?
    let contextCheckpointID: String?
    let createdAtMs: Int64
}

nonisolated protocol ConversationRemoteTurnStore: AnyObject {
    func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws
    func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue

}

nonisolated enum IOSCanonicalRemoteConversationCommands {
    static func prepare(
        store: IOSApplicationRuntime,
        descriptor: IOSRemoteConversationTurnDescriptor
    ) throws {
        _ = try store.executeAgentCommand(.object([
            "type": .string("prepare_remote_submission"),
            "input": .object([
                "source_host_id": .string(descriptor.sourceHostID),
                "client_id": .string(descriptor.clientID),
                "conversation_id": .string(descriptor.conversationID),
                "conversation_title": .string(descriptor.conversationTitle),
                "submission_id": .string(descriptor.identity.submissionID),
                "user_message_id": .string(descriptor.identity.userMessageID),
                "user_part_id": .string("\(descriptor.identity.userMessageID):text"),
                "user_text": .string(descriptor.userText),
                "user_content_json": descriptor.userContentJSON.map(AgentCoreJSONValue.string) ?? .null,
                "assistant_message_id": .string(descriptor.identity.assistantMessageID),
                "assistant_part_id": .string("\(descriptor.identity.assistantMessageID):text"),
                "context_checkpoint_id": descriptor.contextCheckpointID.map(AgentCoreJSONValue.string) ?? .null,
                "created_at_ms": .number(Double(descriptor.createdAtMs)),
            ]),
        ]))
    }

}
