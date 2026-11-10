import Foundation

/// Immutable user intent captured on the sending actor before preparation
/// yields. Resource authorization is revalidated, not granted by this value.
struct MacRunSubmissionSnapshot {
    let conversation: MacConversation
    let submissionID: String
    let canonicalAssistantMessageID: String
    let userMessage: MacChatMessage
    let assistantMessage: MacChatMessage
    let prompt: String
    let draft: String
    let attachments: [MacAttachment]
    let preparedAttachments: [MacPreparedAttachment]
    let contextComponents: [MacCoreContextComponent]
    let platformSelection: MacPlatformContextSelection
    let webSelection: MacWebSourceContextSelection?
    let workspaceSelection: MacSecurityScopedBookmarkStore.SubmissionSelection
    let externalSessionID: String?
    let remoteTarget: MacRemoteSubmissionTarget?
    let expectedParentMessageID: String?
    var queuedUserCommandID: String? = nil
    var canonicalUserMessageID: String? = nil
    var ownsComposerDraft = true
}
