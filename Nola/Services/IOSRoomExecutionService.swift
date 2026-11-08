// SPDX-License-Identifier: Apache-2.0
import Foundation

@MainActor final class IOSRoomExecutionService {
    let runtime: IOSApplicationRuntime
    let manager: ChatManager
    init(runtime: IOSApplicationRuntime, manager: ChatManager) { self.runtime = runtime; self.manager = manager }

    func prepare(_ receipt: IOSRoomSubmission) async throws -> IOSAgentRoomService.ExecutionIntent {
        if let entry = try expertEntry(receipt) {
            let prepared = try IOSPreparedRunModel.prepareExpert(entry, using: runtime)
            _ = prepared // Resolve the exact declared profile/credential before claiming.
            return .init(execution_id: entry.child_submission_id, envelope: try expertEnvelope(receipt, entry: entry))
        }
        if let context = receipt.contextConversationID, let id = UUID(uuidString: context),
           let owner = manager.ensureExecutionForConversation(id), owner.getCurrentModel()?.isWebBackend == true {
            guard let executionID = UUID(uuidString: receipt.id) else { throw IOSApplicationRuntimeError.invalidResponse }
            let bound = try await owner.prepareRemoteRoomExecution(text: receipt.input.text, executionID: executionID)
            guard let digest = bound.request_digest else { throw IOSApplicationRuntimeError.invalidResponse }
            return .init(execution_id: bound.identity.submission_id,
                envelope: .remoteHost(.init(kind: "remote_host", identity: bound.identity, request_digest: digest)))
        }
        return .init(execution_id: receipt.id, envelope: try envelope(receipt))
    }

    private func expertEntry(_ receipt: IOSRoomSubmission) throws -> AppleExpertMessageEntry<AgentCoreJSONValue>? {
        guard let parent = receipt.input.parentMessageID, let owner = receipt.input.conversationID else { return nil }
        guard let manifest = try IOSAgentRoomService(runtime: runtime).expertManifest(conversationID: owner, messageID: parent),
              manifest.input.room_id == receipt.input.roomID, manifest.input.text == receipt.input.text,
              let entry = manifest.entries.first(where: { $0.child_submission_id == receipt.id }),
              entry.participant.context_conversation_id == receipt.contextConversationID,
              entry.participant.participant_id == receipt.selectedParticipantID else {
            throw IOSApplicationRuntimeError.rejected("The complete original expert declaration is unavailable")
        }
        return entry
    }

    private func expertEnvelope(_ receipt: IOSRoomSubmission, entry: AppleExpertMessageEntry<AgentCoreJSONValue>) throws -> IOSAgentRoomService.ExecutionIntent.Envelope {
        guard let context = receipt.contextConversationID, let conversation = try runtime.loadConversation(id: context),
              conversation.configurationRevision == entry.context_configuration_revision else {
            throw IOSApplicationRuntimeError.rejected("The expert context configuration changed after admission")
        }
        return .init(kind: "native", platform: "ios", conversation_configuration_revision: entry.context_configuration_revision,
            model_profile_id: entry.model_declaration.profile_id, model_profile_revision: entry.model_declaration.profile_revision,
            expected_parent_message_id: conversation.messages.last?.id, text: receipt.input.text)
    }

    private func envelope(_ receipt: IOSRoomSubmission) throws -> IOSAgentRoomService.ExecutionIntent.Envelope {
        guard let context = receipt.contextConversationID, let id = UUID(uuidString: context),
              let owner = manager.ensureExecutionForConversation(id),
              let model = owner.getCurrentModel(),
              let profile = try runtime.modelProfile(id: model.id), profile.enabled, profile.deletedAtMs == nil,
              let conversation = try runtime.loadConversation(id: context),
              let revision = conversation.configurationRevision else {
            throw IOSApplicationRuntimeError.rejected("Room execution model or configuration is unavailable")
        }
        return .init(kind: "native", platform: "ios", conversation_configuration_revision: revision,
            model_profile_id: profile.id, model_profile_revision: profile.revision,
            expected_parent_message_id: conversation.messages.last?.id, text: receipt.input.text)
    }

    func execute(_ receipt: IOSRoomSubmission, intent: IOSAgentRoomService.ExecutionIntent) async throws {
        if case .remoteHost(let remote) = intent.envelope {
            guard receipt.state == "executing", receipt.executionID == intent.execution_id,
                  receipt.contextConversationID == remote.identity.conversation_id,
                  intent.execution_id == remote.identity.submission_id,
                  let id = UUID(uuidString: remote.identity.conversation_id),
                  let owner = manager.ensureExecutionForConversation(id) else {
                throw IOSApplicationRuntimeError.rejected("Remote Room execution owner changed")
            }
            try await owner.executeRemoteRoomRequest(remote, text: receipt.input.text)
            return
        }
        guard let native = intent.envelope.native,
              receipt.state == "executing", receipt.executionID == intent.execution_id,
              let context = receipt.contextConversationID, let id = UUID(uuidString: context),
              let executionID = UUID(uuidString: intent.execution_id),
              let owner = manager.ensureExecutionForConversation(id) else {
            throw IOSApplicationRuntimeError.rejected("The captured Room execution configuration changed; no new request was sent")
        }
        if let entry = try expertEntry(receipt) {
            // Existing durable intent remains the captured target. Resolve only
            // its original declared profile, never today's inherited selection.
            guard intent.execution_id == entry.child_submission_id,
                  native.model_profile_id == entry.model_declaration.profile_id,
                  native.model_profile_revision == entry.model_declaration.profile_revision,
                  native.conversation_configuration_revision == entry.context_configuration_revision,
                  native.text == receipt.input.text, native.platform == "ios",
                  let current = try runtime.loadConversation(id: context),
                  current.configurationRevision == entry.context_configuration_revision else {
                throw IOSApplicationRuntimeError.rejected("The original expert execution intent changed")
            }
            try await owner.processExpertMessage(native.text, submissionID: receipt.id, entry: entry)
        } else {
            guard try envelope(receipt) == intent.envelope else { throw IOSApplicationRuntimeError.invalidResponse }
            _ = try await owner.processMessage(native.text, userMessageID: executionID)
        }
    }

    func recover(_ receipt: IOSRoomSubmission, intent: IOSAgentRoomService.ExecutionIntent) async throws {
        guard receipt.executionID == intent.execution_id, let context = receipt.contextConversationID,
              let id = UUID(uuidString: context), let owner = manager.ensureExecutionForConversation(id) else {
            throw IOSApplicationRuntimeError.rejected("Room recovery owner changed")
        }
        try await owner.recoverRoomExecution(executionID: intent.execution_id)
    }

    func cancelPrepared(_ receipt: IOSRoomSubmission) async throws {
        guard receipt.state == "cancelled", let context = receipt.contextConversationID,
              let id = UUID(uuidString: context), let owner = manager.ensureExecutionForConversation(id) else { return }
        try await owner.cancelPreparedRoomRequest(executionID: receipt.executionID ?? receipt.id)
    }
}
