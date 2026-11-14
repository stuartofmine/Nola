// SPDX-License-Identifier: Apache-2.0
// Prepared is not sent. These model-only witnesses never authorize old effects.
import Foundation

nonisolated struct ApplePrepareEditedResend: Codable, Equatable, Sendable {
    let operation_id: String
    let source_conversation_id: String
    let source_message_id: String
    let expected_source_revision: Int64
    let expected_source_message_revision: UInt64
    let edited_text: String
    let target_conversation_id: String
    let title: String
    let created_at_ms: Int64
}
nonisolated struct AppleFrozenResendModel<JSON: JournalJSONValue>: Codable, Sendable {
    let version: UInt32
    let primary: JSON
    let fallbacks: JSON
    let generation: JSON
    let output_admission: JSON
    let model_capability: JSON
}
nonisolated struct AppleMessageSendWitness<JSON: JournalJSONValue>: Decodable, Sendable {
    let conversation_id: String
    let message_id: String
    let run_id: String
    let assistant_message_id: String
    let body_sha256: String
    let invocation_config_sha256: String
    let model: AppleFrozenResendModel<JSON>
}
nonisolated struct ApplePreparedEditedResend<JSON: JournalJSONValue>: Decodable, Sendable {
    let input: ApplePrepareEditedResend
    let source: AppleMessageSendWitness<JSON>
    let user_message_id: String
    let assistant_message_id: String
    let run_id: String
    /// Non-nil proves normal Run admission; it does not prove task completion.
    let admitted_run_id: String?
}

/// Decoded Core intrinsic facts. The original JSON remains the binding source.
nonisolated struct AppleFrozenModelCapabilityFacts: Decodable, Sendable {
    let profileId: String
    let revision: UInt64
    let instructionProfile: String
    let nativeToolCalls: Bool
    let parallelToolCalls: Bool
    let reasoning: Bool
    let reasoningContinuity: Bool
    let promptCache: Bool
    let visionInput: Bool
    let nativePdfInput: Bool
    let contextWindowTokens: Int?
    let maxOutputTokens: Int?
    let requestContinuation: Bool
    let maxRequestContinuations: Int
    let streamText: Bool
    let streamReasoning: Bool
    let streamToolCalls: Bool
    let directToolCapabilityIds: [String]
}

// Additive same-conversation contract; the fresh-branch command remains supported.
nonisolated struct ApplePrepareInPlaceResend: Codable, Equatable, Sendable {
    let operation_id: String
    let conversation_id: String
    let message_id: String
    let expected_revision: Int64
    let expected_message_revision: UInt64
    let edited_text: String
    let created_at_ms: Int64
}
nonisolated struct ApplePreparedInPlaceResend: Codable, Equatable, Sendable {
    let input: ApplePrepareInPlaceResend
    let run_id: String
    let user_message_id: String
    let experts: [AppleExpertParticipantReference]
}
