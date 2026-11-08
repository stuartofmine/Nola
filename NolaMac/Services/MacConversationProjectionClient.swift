// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Decodes revisioned Application Service conversation projection events.

import Foundation

nonisolated struct MacCanonicalConversationProjectionEvent: Decodable, Equatable, Sendable {
    let conversationID: String
    let runID: String?
    let attemptID: String?
    let delegatedInvocationID: String?
    let eventID: String
    let sequence: UInt64
    let mutation: Mutation
    let occurredAt: String

    enum Mutation: Decodable, Equatable, Sendable {
        case invalidateProjection(revision: UInt64)
        case setTitle(title: String, revision: UInt64)
        case upsertMessage(MacCanonicalConversationProjectionV1.Message)
        case removeMessage(messageID: String, revision: UInt64)
        case appendTextPreview(messageID: String, partID: String, revision: UInt64, delta: String)
        case completeTextPart(messageID: String, partID: String, revision: UInt64, markdown: String)
        case setActiveRun(MacCanonicalConversationProjectionV1.RunActivity?)
        case replaceSurface(MacCanonicalConversationProjectionV1.Surface)
        case setContextUsage(MacCanonicalConversationProjectionV1.ContextUsage)

        private enum Kind: String, Decodable {
            case invalidateProjection = "invalidate_projection"
            case setTitle = "set_title"
            case upsertMessage = "upsert_message"
            case removeMessage = "remove_message"
            case appendTextPreview = "append_text_preview"
            case completeTextPart = "complete_text_part"
            case setActiveRun = "set_active_run"
            case replaceSurface = "replace_surface"
            case setContextUsage = "set_context_usage"
        }

        private enum CodingKeys: String, CodingKey {
            case type, title, revision, message, delta, markdown, surface
            case messageID = "message_id"
            case partID = "part_id"
            case activeRun = "active_run"
            case contextUsage = "context_usage"
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            switch try values.decode(Kind.self, forKey: .type) {
            case .invalidateProjection:
                self = .invalidateProjection(revision: try values.decode(UInt64.self, forKey: .revision))
            case .setTitle:
                self = .setTitle(
                    title: try values.decode(String.self, forKey: .title),
                    revision: try values.decode(UInt64.self, forKey: .revision)
                )
            case .upsertMessage:
                self = .upsertMessage(
                    try values.decode(
                        MacCanonicalConversationProjectionV1.Message.self,
                        forKey: .message
                    )
                )
            case .removeMessage:
                self = .removeMessage(
                    messageID: try values.decode(String.self, forKey: .messageID),
                    revision: try values.decode(UInt64.self, forKey: .revision)
                )
            case .appendTextPreview:
                self = .appendTextPreview(
                    messageID: try values.decode(String.self, forKey: .messageID),
                    partID: try values.decode(String.self, forKey: .partID),
                    revision: try values.decode(UInt64.self, forKey: .revision),
                    delta: try values.decode(String.self, forKey: .delta)
                )
            case .completeTextPart:
                self = .completeTextPart(
                    messageID: try values.decode(String.self, forKey: .messageID),
                    partID: try values.decode(String.self, forKey: .partID),
                    revision: try values.decode(UInt64.self, forKey: .revision),
                    markdown: try values.decode(String.self, forKey: .markdown)
                )
            case .setActiveRun:
                self = .setActiveRun(
                    try values.decodeIfPresent(
                        MacCanonicalConversationProjectionV1.RunActivity.self,
                        forKey: .activeRun
                    )
                )
            case .replaceSurface:
                self = .replaceSurface(
                    try values.decode(
                        MacCanonicalConversationProjectionV1.Surface.self,
                        forKey: .surface
                    )
                )
            case .setContextUsage:
                self = .setContextUsage(
                    try values.decode(
                        MacCanonicalConversationProjectionV1.ContextUsage.self,
                        forKey: .contextUsage
                    )
                )
            }
        }
    }

    private struct Scope: Decodable, Equatable {
        let type: String
        let conversationID: String?

        private enum CodingKeys: String, CodingKey {
            case type
            case conversationID = "conversation_id"
        }
    }

    private struct Cursor: Decodable {
        let scope: Scope
        let sequence: UInt64
    }

    private struct Identity: Decodable {
        let eventID: String
        let conversationID: String?
        let runID: String?
        let attemptID: String?
        let delegatedInvocationID: String?
        let messageID: String?
        let partID: String?

        private enum CodingKeys: String, CodingKey {
            case eventID = "event_id"
            case conversationID = "conversation_id"
            case runID = "run_id"
            case attemptID = "attempt_id"
            case delegatedInvocationID = "delegated_invocation_id"
            case messageID = "message_id"
            case partID = "part_id"
        }
    }

    private struct MutationEnvelope: Decodable {
        let type: String
        let mutation: Mutation
    }

    private enum CodingKeys: String, CodingKey {
        case scope, cursor, identity, mutation
        case occurredAt = "occurred_at"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let scope = try values.decode(Scope.self, forKey: .scope)
        let cursor = try values.decode(Cursor.self, forKey: .cursor)
        let identity = try values.decode(Identity.self, forKey: .identity)
        let envelope = try values.decode(MutationEnvelope.self, forKey: .mutation)
        let targetIdentity: (messageID: String?, partID: String?) = switch envelope.mutation {
        case .invalidateProjection(_), .setTitle(_, _), .setActiveRun(_), .replaceSurface(_), .setContextUsage(_):
            (nil, nil)
        case let .upsertMessage(message):
            (message.messageID, nil)
        case let .removeMessage(messageID, _):
            (messageID, nil)
        case let .appendTextPreview(messageID, partID, _, _),
             let .completeTextPart(messageID, partID, _, _):
            (messageID, partID)
        }
        guard scope.type == "conversation",
              cursor.scope == scope,
              let scopedConversationID = scope.conversationID,
              identity.conversationID == scopedConversationID,
              identity.messageID == targetIdentity.messageID,
              identity.partID == targetIdentity.partID,
              envelope.type == "conversation",
              cursor.sequence > 0,
              !(identity.attemptID != nil && identity.delegatedInvocationID != nil),
              (identity.attemptID == nil && identity.delegatedInvocationID == nil) || identity.runID != nil,
              !identity.eventID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: values.codingPath,
                debugDescription: "Projection event identity does not match its conversation scope."
            ))
        }
        conversationID = scopedConversationID
        runID = identity.runID
        attemptID = identity.attemptID
        delegatedInvocationID = identity.delegatedInvocationID
        eventID = identity.eventID
        sequence = cursor.sequence
        mutation = envelope.mutation
        occurredAt = try values.decode(String.self, forKey: .occurredAt)
    }
}

nonisolated struct MacCanonicalConversationProjectionEventBatch: Equatable, Sendable {
    let events: [MacCanonicalConversationProjectionEvent]
    let hasMore: Bool
    let afterSequence: UInt64
    let nextSequence: UInt64
}
