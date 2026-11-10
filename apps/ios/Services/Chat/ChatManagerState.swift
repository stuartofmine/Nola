// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable public error and processing-state values for iOS chat.

import Foundation

/// A foreground draft is not an execution record. Hidden conversations keep
/// their durable Run, but their delayed UI callbacks cannot mutate this canvas.
enum ChatDraftPresentationOwnership {
    static func accepts(
        assistantID: UUID,
        activeAssistantID: UUID?,
        ownerConversationID: UUID?,
        selectedConversationID: UUID?
    ) -> Bool {
        guard let ownerConversationID else { return false }
        return assistantID == activeAssistantID && ownerConversationID == selectedConversationID
    }
}

public enum ChatManagerError: LocalizedError {
    case noModelSelected
    case failedToLoadLocalModel(String)
    case failedToProcessMessage(String)
    case failedToCreateConversation(String)
    case failedToAppendMessage(String)

    public var errorDescription: String? {
        switch self {
        case .noModelSelected:
            return L10n.chatErrorNoModelSelected
        case .failedToLoadLocalModel(let message):
            return L10n.format("chat.error.failed_to_load_local_model_format", message)
        case .failedToProcessMessage(let message):
            return L10n.format("chat.error.failed_to_process_message_format", message)
        case .failedToCreateConversation(let message):
            return L10n.format("chat.error.failed_to_create_conversation_format", message)
        case .failedToAppendMessage(let message):
            return L10n.format("chat.error.failed_to_append_message_format", message)
        }
    }
}

public enum ChatProcessingState: Equatable {
    case idle
    case loadingLocalModel
    case processingWithLocal
    case processingWithCloud
    case streaming(progress: Double)
    case error(String)
    case completed

    public var isIdle: Bool {
        if case .idle = self { return true }
        return false
    }

    public var blocksInput: Bool {
        switch self {
        case .loadingLocalModel, .processingWithLocal, .processingWithCloud, .streaming:
            return true
        case .idle, .error, .completed:
            return false
        }
    }

    public var showsInlineStatus: Bool {
        switch self {
        case .loadingLocalModel, .processingWithLocal, .processingWithCloud, .streaming:
            return true
        case .idle, .error, .completed:
            return false
        }
    }
}
