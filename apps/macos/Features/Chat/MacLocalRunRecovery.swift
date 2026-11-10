import Foundation

/// A lost local transport is not a terminal Run. The frozen submission identity
/// is retained even when the Host never acknowledged creating a durable Run.
struct MacLocalRunRecovery: Equatable, Sendable {
    let conversationID: String
    let expectedRunID: String
    let canonicalAssistantMessageID: String
    let assistantMessageID: UUID
    let generationID: UInt64
    var restoredFromStorage = false

    var title: String {
        restoredFromStorage
            ? String(localized: "An earlier task is unfinished. It has not been restarted.", table: "Chat")
            : String(localized: "Local execution connection interrupted. The result is not yet confirmed.", table: "Chat")
    }
}
