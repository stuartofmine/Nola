// SPDX-License-Identifier: Apache-2.0
// Complete Core-owned expert plans. Definitions are data, never permission grants.
import Foundation

nonisolated struct AppleExpertParticipantReference: Codable, Equatable, Sendable {
    let participant_id: String
    let agent_id: String
    let agent_revision: Int64
    let context_conversation_id: String
}
nonisolated struct AppleAdmitExpertConversationMessage: Codable, Equatable, Sendable {
    let conversation_id: String
    let expected_revision: Int64
    let room_id: String
    let expected_room_revision: Int64
    let submission_id: String
    let user_message_id: String
    let text: String
    let selected_participants: [AppleExpertParticipantReference]
    let created_at_ms: Int64
}
nonisolated struct AppleExpertModelDeclaration<JSON: JournalJSONValue>: Codable, Sendable {
    let profile_id: String
    let profile_revision: Int64
    let provider: String
    let model_name: String
    let runtime: String
    let parameters: JSON
}
nonisolated struct AppleExpertPromptDefinition<JSON: JournalJSONValue>: Codable, Sendable {
    let component_id: String
    let content: String
    let content_sha256: String
    let definition_revision: UInt64?
    let platform: JSON
}
nonisolated struct AppleExpertInstructionWitness<JSON: JournalJSONValue>: Decodable, Sendable {
    let checkpoint_sha256: String
    let component_payload_digest: String
    let prompt_definitions: [AppleExpertPromptDefinition<JSON>]
}
nonisolated struct AppleExpertMessageEntry<JSON: JournalJSONValue>: Decodable, Sendable {
    let participant: AppleExpertParticipantReference
    let display_name: String
    let description: String
    let defaults: JSON
    let model_declaration: AppleExpertModelDeclaration<JSON>
    let context_configuration_revision: Int64
    let child_submission_id: String
    let child_user_message_id: String
    let child_assistant_message_id: String
    let expected_prompt_definition: AppleExpertPromptDefinition<JSON>?
    let model_witness: AppleMessageSendWitness<JSON>?
    let instruction_witness: AppleExpertInstructionWitness<JSON>?
}
nonisolated struct AppleExpertMessageManifest<JSON: JournalJSONValue>: Decodable, Sendable {
    let version: UInt32
    let input: AppleAdmitExpertConversationMessage
    let entries: [AppleExpertMessageEntry<JSON>]
}
nonisolated struct ApplePreparedExpertResendEntry<JSON: JournalJSONValue>: Decodable, Sendable {
    let original: AppleExpertMessageEntry<JSON>
    let participant: AppleExpertParticipantReference
    let prepared: ApplePreparedEditedResend<JSON>
}
nonisolated struct ApplePreparedExpertResend<JSON: JournalJSONValue>: Decodable, Sendable {
    let input: ApplePrepareEditedResend
    let source_submission_id: String
    let room_id: String
    let parent_message_id: String
    let entries: [ApplePreparedExpertResendEntry<JSON>]
}
