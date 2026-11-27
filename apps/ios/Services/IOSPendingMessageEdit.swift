// SPDX-License-Identifier: Apache-2.0
import Foundation

enum IOSEditReceipt { case absent, prepared, admitted }

extension IOSApplicationRuntime {
    /// Looking up the frozen intent must never dispatch a turn, including when the owner is busy.
    func editReceipt(_ request: AppleMessageEditSubmission) throws -> IOSEditReceipt {
        let input = request.inPlacePreparation()
        let prepared = try executeAgentCommand(.object(["type": .string("load_prepared_in_place_resend"),
                                                        "conversation_id": .string(input.conversation_id),
                                                        "operation_id": .string(input.operation_id)]))
        if prepared == .null { return .absent }
        let saved = try JSONDecoder().decode(ApplePreparedInPlaceResend.self, from: JSONEncoder().encode(prepared))
        guard saved.input == input, saved.run_id == input.operation_id else { throw IOSApplicationRuntimeError.invalidResponse }
        do {
            let run = try executeAgentCommand(.object(["type": .string("load_run_identity"), "run_id": .string(saved.run_id)]))
            if run == .null { return .prepared }
            guard case .object(let identity) = run, identity["conversation_id"] == .string(input.conversation_id) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return .admitted
        } catch IOSApplicationRuntimeError.rejected(let detail) where detail.contains("run_not_found") {
            return .prepared
        }
    }
}

struct IOSPendingMessageEdit {
    let runtime: IOSApplicationRuntime
    let conversationID: UUID

    private var drafts: FileConversationDraftRepository { .scoped(to: runtime.databaseURL) }

    func load() throws -> AppleMessageEditSubmission? { try drafts.pendingEdit(conversationID) }

    func retain(_ input: AppleMessageEditSubmission) throws {
        try drafts.retainEdit(input, for: conversationID)
        NotificationCenter.default.post(name: .nolaPendingMessageEditChanged, object: conversationID)
    }

    func clear(_ input: AppleMessageEditSubmission) throws {
        try drafts.clearEdit(input, for: conversationID)
        NotificationCenter.default.post(name: .nolaPendingMessageEditChanged, object: conversationID)
    }
}

extension Notification.Name {
    static let nolaPendingMessageEditChanged = Notification.Name("nolaPendingMessageEditChanged")
}
