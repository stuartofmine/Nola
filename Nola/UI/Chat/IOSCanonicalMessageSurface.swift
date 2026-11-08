// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// A canonical Part anchor shared by ordinary and expert messages. The owning
/// manager resolves the real source Host; this view never borrows a parent owner.
struct IOSCanonicalMessageSurface: View {
    @EnvironmentObject private var container: ServiceContainer
    let conversationID: UUID?
    let messageID: String
    let part: IOSConversationProjectionV1.Part
    var allowsInteraction = true
    private var manager: ChatManager? { container.chatManager }
    private var reference: AppleUISurfaceReference? {
        switch part {
        case .uiSurface(let value): value.surface
        case .historical(_, _, let value): value.uiSurfaceReference
        default: nil
        }
    }
    private func accepts(_ identity: AppleUISurfaceIdentity) -> Bool {
        identity == reference?.identity && identity.owner.conversation_id == conversationID?.uuidString
            && identity.owner.message_id == messageID
    }
    @MainActor private func pendingUI(_ identity: AppleUISurfaceIdentity) throws -> AppleUISubmission<AgentCoreJSONValue>? {
        guard accepts(identity), let manager else { throw IOSApplicationRuntimeError.invalidResponse }
        return try manager.pendingUIInteraction(identity: identity)
    }

    @MainActor private func previewUI(_ input: AppleUIPreviewInput<AgentCoreJSONValue>) async throws -> AppleUIPreviewResult<AgentCoreJSONValue> {
        guard accepts(input.identity), let manager else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await manager.previewUISurface(input)
    }

    @MainActor private func readUIChanges(_ identity: AppleUISurfaceIdentity, _ revision: UInt64?) async throws -> AppleUISurfaceUpdate<AgentCoreJSONValue> {
        guard accepts(identity), let manager else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await manager.readUISurfaceChanges(identity: identity, baseRevision: revision)
    }

    @MainActor private func readUIRevision(_ reference: AppleUISurfaceReference) async throws -> AppleUISnapshot<AgentCoreJSONValue>? {
        guard accepts(reference.identity), let manager else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await manager.readUISurfaceRevision(reference)
    }

    @MainActor private func submitUI(_ input: AppleUISubmission<AgentCoreJSONValue>) async -> AppleUISubmissionOutcome<AgentCoreJSONValue> {
        guard accepts(input.interaction.identity), let manager else { return .rejected("会话来源已改变，请返回原会话操作。") }
        return await manager.submitUIInteraction(input)
    }


    private func belongs(_ reference: AppleUISurfaceReference) -> Bool {
        reference.identity.owner.conversation_id == conversationID?.uuidString
            && reference.identity.owner.message_id == messageID
    }
    var body: some View {
        Group {
            switch part {
            case .uiSurface(let value):
                if belongs(value.surface), !allowsInteraction {
                    AppleUIReadOnlyReferencedSurfaceView<AgentCoreJSONValue>(reference: value.surface, summary: value.summary,
                        notice: String(localized: "chat.expert.ui.read_only"), readRevision: readUIRevision)
                        .id(value.surface.identity)
                } else if belongs(value.surface) {
                    AppleUIReferencedSurfaceView<AgentCoreJSONValue>(reference: value.surface, summary: value.summary,
                        readChanges: readUIChanges, preview: previewUI, readRevision: readUIRevision,
                        pendingSubmission: pendingUI, submit: submitUI)
                        .id(value.surface.identity)
                }
            case .historical(_, _, let history):
                if let reference = history.uiSurfaceReference, belongs(reference) {
                    AppleUIHistoryView<AgentCoreJSONValue>(reference: reference, load: readUIRevision)
                }
            default: EmptyView()
            }
        }
    }
}
