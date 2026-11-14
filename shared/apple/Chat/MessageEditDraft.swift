// SPDX-License-Identifier: Apache-2.0
// Window-scoped editing only. No composer/history mutation and no Run execution.
import Foundation

nonisolated struct AppleMessageEditSubmission: Sendable {
    let operationID: UUID
    let targetConversationID: UUID
    let createdAtMs: Int64
    let sourceConversationID: String
    let sourceMessageID: String
    let expectedSourceRevision: Int64
    let expectedMessageRevision: UInt64
    let text: String
    func inPlacePreparation() -> ApplePrepareInPlaceResend {
        ApplePrepareInPlaceResend(operation_id: operationID.uuidString.lowercased(), conversation_id: sourceConversationID,
            message_id: sourceMessageID, expected_revision: expectedSourceRevision,
            expected_message_revision: expectedMessageRevision, edited_text: text, created_at_ms: createdAtMs)
    }
    func preparation(title: String) -> ApplePrepareEditedResend {
        ApplePrepareEditedResend(operation_id: operationID.uuidString.lowercased(),
            source_conversation_id: sourceConversationID, source_message_id: sourceMessageID,
            expected_source_revision: expectedSourceRevision, expected_source_message_revision: expectedMessageRevision,
            edited_text: text, target_conversation_id: targetConversationID.uuidString, title: title, created_at_ms: createdAtMs)
    }
}

nonisolated struct AppleMessageEditDraft: Sendable {
    private(set) var operationID: UUID
    private(set) var targetConversationID: UUID
    private(set) var createdAtMs: Int64
    let sourceConversationID: String?
    let sourceMessageID: String
    let expectedSourceRevision: Int64?
    let expectedMessageRevision: UInt64?
    let originalText: String
    var text: String {
        didSet {
            guard text != oldValue else { return }
            // Deliberately changing input is a new intent; identical retries retain IDs.
            operationID = UUID(); targetConversationID = UUID()
            createdAtMs = Int64(Date().timeIntervalSince1970 * 1_000)
        }
    }

    init(sourceConversationID: String?, sourceMessageID: String, expectedSourceRevision: Int64?,
         expectedMessageRevision: UInt64?, text: String,
         operationID: UUID = UUID(), targetConversationID: UUID = UUID()) {
        self.createdAtMs = Int64(Date().timeIntervalSince1970 * 1_000)
        self.operationID = operationID
        self.targetConversationID = targetConversationID
        self.sourceConversationID = sourceConversationID
        self.sourceMessageID = sourceMessageID
        self.expectedSourceRevision = expectedSourceRevision
        self.expectedMessageRevision = expectedMessageRevision
        originalText = text
        self.text = text
    }
    /// A UI hint only. Core must revalidate both captured revisions at prepare.
    var hasSourceWitness: Bool {
        sourceConversationID != nil && expectedSourceRevision.map { $0 > 0 } == true
            && expectedMessageRevision.map { $0 > 0 } == true && !sourceMessageID.isEmpty
    }
    func inPlacePreparation() -> ApplePrepareInPlaceResend? {
        guard let sourceConversationID, let expectedSourceRevision, let expectedMessageRevision,
              hasSourceWitness, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return ApplePrepareInPlaceResend(operation_id: operationID.uuidString.lowercased(),
            conversation_id: sourceConversationID, message_id: sourceMessageID,
            expected_revision: expectedSourceRevision, expected_message_revision: expectedMessageRevision,
            edited_text: text, created_at_ms: createdAtMs)
    }
    func preparation(title: String) -> ApplePrepareEditedResend? {
        guard let sourceConversationID, let expectedSourceRevision, let expectedMessageRevision,
              hasSourceWitness, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return ApplePrepareEditedResend(operation_id: operationID.uuidString.lowercased(),
            source_conversation_id: sourceConversationID, source_message_id: sourceMessageID,
            expected_source_revision: expectedSourceRevision, expected_source_message_revision: expectedMessageRevision,
            edited_text: text, target_conversation_id: targetConversationID.uuidString,
            title: title, created_at_ms: createdAtMs)
    }
}
