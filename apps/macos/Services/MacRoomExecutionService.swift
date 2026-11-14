// SPDX-License-Identifier: Apache-2.0
import Foundation
import Observation

/// Each private participant context has its own execution/presentation owner.
/// This service never touches the ordinary window's selected conversation.
@Observable @MainActor final class MacRoomExecutionService {
    private let repository: MacCanonicalConversationRepository
    private let localRuntime: any MacLocalAgentRuntime
    private let remoteHost: any MacRemoteAgentRunning
    private(set) var owners: [String: MacWorkspaceModel] = [:]

    init(repository: MacCanonicalConversationRepository,
         localRuntime: any MacLocalAgentRuntime, remoteHost: any MacRemoteAgentRunning) {
        self.repository = repository
        self.localRuntime = localRuntime
        self.remoteHost = remoteHost
    }

    func prepare(_ receipt: MacRoomSubmission) async throws -> MacAgentRoomService.ExecutionIntent {
        guard let context = receipt.contextConversationID else {
            throw MacApplicationPortError.rejected("Missing Room execution context")
        }
        let owner = observe(contextID: context)
        let service = repository.makeAgentRoomService()
        if let prepared = try service.preparedEditedChild(context) {
            guard prepared.run_id == receipt.id else { throw MacEditedResendError.sourceUnavailable }
            return .init(execution_id: prepared.run_id, envelope: try await owner.prepareEditedRoomExecution(prepared))
        }
        if let parent = receipt.input.parentMessageID, let parentOwner = receipt.input.conversationID {
            guard let manifest = try service.expertMessageSource(conversationID: parentOwner, messageID: parent),
                  let entry = manifest.entries.first(where: { $0.child_submission_id == receipt.id }),
                  entry.participant.context_conversation_id == context else { throw MacEditedResendError.sourceUnavailable }
            let envelope = try owner.capturedExpertRoomEnvelope(contextID: context, text: receipt.input.text, entry: entry)
            if owner.executionHost == .remote {
                guard let definitions = try service.expertChildInstructions(conversationID: context, expectedRunID: receipt.id) else {
                    throw MacEditedResendError.sourceUnavailable
                }
                let bound = try await owner.prepareRemoteRoomExecution(contextID: context, executionID: receipt.id,
                    text: receipt.input.text, capturedExpertDefinitions: definitions)
                guard let digest = bound.request_digest else { throw MacRemoteHostError.invalidResponse }
                return .init(execution_id: bound.identity.submission_id, envelope: .remoteHost(.init(kind: "remote_host", identity: bound.identity, request_digest: digest)))
            }
            return .init(execution_id: receipt.id, envelope: envelope)
        }
        if owner.executionHost == .remote {
            let bound = try await owner.prepareRemoteRoomExecution(contextID: context,
                executionID: receipt.id, text: receipt.input.text)
            guard let digest = bound.request_digest else { throw MacRemoteHostError.invalidResponse }
            return .init(execution_id: bound.identity.submission_id,
                envelope: .remoteHost(.init(kind: "remote_host", identity: bound.identity, request_digest: digest)))
        }
        return .init(execution_id: receipt.id,
            envelope: try owner.roomExecutionEnvelope(contextID: context, text: receipt.input.text))
    }
    func execute(_ receipt: MacRoomSubmission, intent: MacAgentRoomService.ExecutionIntent) async throws {
        guard receipt.state == "executing", let contextID = receipt.contextConversationID,
              let executionID = receipt.executionID else {
            throw MacApplicationPortError.rejected("Room execution has no canonical owner")
        }
        let owner = observe(contextID: contextID)
        if let prepared = try repository.makeAgentRoomService().preparedEditedChild(contextID) {
            guard prepared.run_id == executionID && prepared.input.edited_text == receipt.input.text else { throw MacEditedResendError.sourceUnavailable }
            try await owner.admitPreparedEditedResend(prepared, configurationPrepared: true)
            return
        }
        guard executionID == intent.execution_id else { throw MacApplicationPortError.rejected("Room execution ID changed") }
        if case .remoteHost(let remote) = intent.envelope {
            guard remote.identity.conversation_id == contextID, remote.identity.submission_id == executionID else {
                throw MacRemoteHostError.invalidResponse
            }
            try await owner.executeRemoteRoomRequest(remote, text: receipt.input.text)
        } else {
            var expert: AppleExpertMessageEntry<MacCanonicalJSONValue>?
            if let parent = receipt.input.parentMessageID, let parentOwner = receipt.input.conversationID {
                guard let manifest = try repository.makeAgentRoomService().expertMessageSource(conversationID: parentOwner, messageID: parent),
                      let entry = manifest.entries.first(where: { $0.child_submission_id == receipt.id }),
                      entry.participant.context_conversation_id == contextID else { throw MacEditedResendError.sourceUnavailable }
                expert = entry
            }
            try await owner.sendRoomMessage(contextID: contextID, intent: intent, expert: expert)
        }
    }

    var historyDatabaseURL: URL { repository.databaseURL }

    private func historyTarget(childID: String, source: AppleConversationSourceOwner) throws -> MacRemoteSubmissionTarget? {
        guard try repository.conversationSourceOwner(conversationID: childID) == source else { throw MacApplicationPortError.invalidResponse }
        switch source {
        case .local: return nil
        case .unavailable: throw MacApplicationPortError.invalidResponse
        case .remote(let hostID, let epoch):
            let target = try remoteHost.captureSubmissionTarget()
            guard target.configuration.expectedHostID == hostID else { throw MacApplicationPortError.invalidResponse }
            return MacRemoteSubmissionTarget(configuration: target.configuration, clientID: target.clientID, expectedSourceStoreEpoch: epoch)
        }
    }
    func readHistoryUIChanges(childID: String, source: AppleConversationSourceOwner,
        identity: AppleUISurfaceIdentity, baseRevision: UInt64?) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue> {
        guard identity.owner.conversation_id == childID else { throw MacApplicationPortError.invalidResponse }
        if let target = try historyTarget(childID: childID, source: source) {
            return try await remoteHost.readUISurfaceChanges(identity: identity, baseRevision: baseRevision, target: target)
        }
        let database = repository.databaseURL
        return try await Task.detached { try MacApplicationPort(databaseURL: database).readUISurfaceChanges(identity: identity, baseRevision: baseRevision) }.value
    }
    func readHistoryUIRevision(childID: String, source: AppleConversationSourceOwner,
        reference: AppleUISurfaceReference) async throws -> AppleUISnapshot<MacCanonicalJSONValue>? {
        guard reference.identity.owner.conversation_id == childID else { throw MacApplicationPortError.invalidResponse }
        if let target = try historyTarget(childID: childID, source: source) {
            return try await remoteHost.readUISurfaceRevision(reference, target: target)
        }
        let database = repository.databaseURL
        return try await Task.detached { try MacApplicationPort(databaseURL: database).readUISurfaceRevision(identity: reference.identity, revision: reference.revision) }.value
    }

    @discardableResult func observe(contextID: String) -> MacWorkspaceModel {
        if let existing = owners[contextID] { return existing }
        let owner = repository.makeRoomExecutionOwner(contextID: contextID,
            localRuntime: localRuntime, remoteHost: remoteHost)
        owners[contextID] = owner
        owner.presentationDidAttach()
        return owner
    }

    func cancelPrepared(_ receipt: MacRoomSubmission) async throws {
        guard receipt.state == "cancelled", let context = receipt.contextConversationID,
              let pending = try repository.remoteSubmissionIntent(conversationID: context),
              pending.identity.submission_id == (receipt.executionID ?? receipt.id),
              pending.phase == .prepared || pending.phase == .bound else { return }
        let target = try remoteHost.captureSubmissionTarget()
        _ = try await remoteHost.refreshSubmission(conversationID: context, target: target,
            cancel: true, expectedIdentity: pending.identity)
    }
}
