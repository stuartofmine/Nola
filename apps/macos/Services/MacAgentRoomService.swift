// SPDX-License-Identifier: Apache-2.0
// Typed desktop adapter for canonical shared rooms. No client-owned member database.
import Foundation

nonisolated struct MacRoomParticipant: Codable, Identifiable, Equatable, Sendable {
    let id: String
    let agentID: String
    let agentRevision: Int64
    let displayName: String
    let description: String
    let contextConversationID: String
    let active: Bool

    enum CodingKeys: String, CodingKey {
        case id, active, description
        case agentID = "agent_id"
        case agentRevision = "agent_revision"
        case displayName = "display_name"
        case contextConversationID = "context_conversation_id"
    }
}

nonisolated struct MacAgentRoom: Codable, Identifiable, Equatable, Sendable {
    let id: String
    let title: String
    let participants: [MacRoomParticipant]
}

nonisolated struct MacAgentRoomSnapshot: Codable, Equatable, Sendable {
    let revision: Int64
    let room: MacAgentRoom
}

/// Durable association with an ordinary conversation; reading never creates or executes.
nonisolated struct MacConversationRoomAssociation: Codable, Equatable, Sendable {
    let conversationID: String
    let roomID: String
    let revision: Int64
    let room: MacAgentRoomSnapshot
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", roomID = "room_id", revision, room
    }
}

nonisolated struct MacPersistentAgent: Codable, Identifiable, Sendable {
    let id: String
    let name: String
    let description: String
    let revision: Int64
    let availability: String
    let defaults: MacCanonicalJSONValue
}

nonisolated struct MacAgentDirectory: Codable, Sendable {
    let revision: Int64?
    let agents: [MacPersistentAgent]
}

nonisolated struct MacRoomSubmissionInput: Codable, Equatable, Sendable {
    let roomID: String
    let submissionID: String
    let expectedRoomRevision: Int64
    let text: String
    let participantID: String?
    let atMs: Int64
    var parentMessageID: String? = nil
    var conversationID: String? = nil
    enum CodingKeys: String, CodingKey {
        case roomID = "room_id", submissionID = "submission_id"
        case expectedRoomRevision = "expected_room_revision", text
        case participantID = "participant_id", atMs = "at_ms"
        case parentMessageID = "parent_message_id", conversationID = "conversation_id"
    }
}

nonisolated struct MacRoomSubmission: Codable, Identifiable, Sendable {
    let revision: Int64
    let input: MacRoomSubmissionInput
    let state: String
    let selectedParticipantID: String?
    let failureCode: String?
    let executionID: String?
    let terminal: MacCanonicalJSONValue?
    let contextConversationID: String?
    var id: String { input.submissionID }
    enum CodingKeys: String, CodingKey {
        case revision, input, state, contextConversationID = "context_conversation_id"
        case selectedParticipantID = "selected_participant_id"
        case failureCode = "failure_code"
        case executionID = "execution_id"
        case terminal
    }
}

/// The closure reaches the same receipt-admitted Application runtime used by
/// ordinary conversation commands, preserving atomic creation and replay.
nonisolated final class MacAgentRoomService: Sendable {
    struct ExecutionIntent: Codable, Equatable, Sendable {
        let execution_id: String
        let envelope: Envelope
        enum Envelope: Codable, Equatable, Sendable {
            struct Native: Codable, Equatable, Sendable {
                let kind: String
                let platform: String
                let conversation_configuration_revision: Int64
                let model_profile_id: String
                let model_profile_revision: Int64
                let expected_parent_message_id: String?
                let text: String
            }
            struct RemoteHost: Codable, Equatable, Sendable {
                let kind: String
                let identity: MacRemoteSubmissionIdentity
                let request_digest: String
            }
            case native(Native)
            case remoteHost(RemoteHost)
            var native: Native? { if case .native(let value) = self { value } else { nil } }
            init(kind: String, platform: String, conversation_configuration_revision: Int64,
                 model_profile_id: String, model_profile_revision: Int64, expected_parent_message_id: String?, text: String) {
                self = .native(.init(kind: kind, platform: platform,
                    conversation_configuration_revision: conversation_configuration_revision,
                    model_profile_id: model_profile_id, model_profile_revision: model_profile_revision,
                    expected_parent_message_id: expected_parent_message_id, text: text))
            }
            private enum CodingKeys: String, CodingKey { case kind }
            init(from decoder: Decoder) throws {
                let container = try decoder.container(keyedBy: CodingKeys.self)
                switch try container.decode(String.self, forKey: .kind) {
                case "native": self = .native(try Native(from: decoder))
                case "remote_host": self = .remoteHost(try RemoteHost(from: decoder))
                default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container,
                    debugDescription: "Unsupported Room execution envelope")
                }
            }
            func encode(to encoder: Encoder) throws {
                switch self {
                case .native(let value): try value.encode(to: encoder)
                case .remoteHost(let value): try value.encode(to: encoder)
                }
            }
        }
    }
    func executionIntent(_ submission: MacRoomSubmission) throws -> ExecutionIntent? {
        try command(["type": .string("execution_intent"), "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id)])
    }
    func prepareExecution(_ submission: MacRoomSubmission, intent: ExecutionIntent) throws -> MacRoomSubmission {
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(intent))
        return try command(["type": .string("prepare_execution"), "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id), "expected_revision": .signedInteger(submission.revision),
            "intent": value, "at_ms": timestamp])
    }
    func executionExists(contextID: String, executionID: String) throws -> Bool {
        let result = try execute(.object(["type": .string("load_exact_run_invocation"),
            "conversation_id": .string(contextID), "expected_run_id": .string(executionID),
            "assistant_message_id": .string(executionID + ":assistant")]))
        guard result != .null else { return false }
        guard case .object(let fields) = result,
              fields["conversation_id"] == .string(contextID), fields["run_id"] == .string(executionID),
              fields["assistant_message_id"] == .string(executionID + ":assistant") else {
            throw MacApplicationPortError.rejected("Execution ownership mismatch")
        }
        return true
    }
    struct ExecutionClaim: Decodable, Sendable {
        let submission: MacRoomSubmission
        let dispatchPermitted: Bool
        enum CodingKeys: String, CodingKey {
            case submission
            case dispatchPermitted = "dispatch_permitted"
        }
    }

    func claimExecution(_ submission: MacRoomSubmission) throws -> ExecutionClaim {
        try command(["type": .string("claim_execution"),
            "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id),
            "expected_revision": .signedInteger(submission.revision),
            "at_ms": timestamp])
    }

    func observeExecution(_ submission: MacRoomSubmission) throws -> MacRoomSubmission {
        guard submission.executionID != nil else {
            throw MacApplicationPortError.rejected("Missing execution identity")
        }
        return try command(["type": .string("reconcile_execution"),
            "room_id": .string(submission.input.roomID), "submission_id": .string(submission.id),
            "at_ms": timestamp])
    }

    func reconcileRouting(_ submission: MacRoomSubmission, sourceHostID: String? = nil) throws -> MacRoomSubmission? {
        let value=try execute(.object(["type": .string("reconcile_room_routing"),
            "room_id": .string(submission.input.roomID), "submission_id": .string(submission.id),
            "source_host_id": sourceHostID.map(MacCanonicalJSONValue.string) ?? .null, "at_ms": timestamp]))
        if value == .null {return nil}
        return try JSONDecoder().decode(MacRoomSubmission.self,from:JSONEncoder().encode(value))
    }

    func selectRecipient(_ submission: MacRoomSubmission, participantID: String) throws -> MacRoomSubmission {
        try command(["type": .string("select_recipient"), "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id), "expected_revision": .signedInteger(submission.revision),
            "participant_id": .string(participantID), "at_ms": timestamp])
    }

    func failRouting(_ submission: MacRoomSubmission) throws -> MacRoomSubmission {
        try command(["type": .string("fail_submission"), "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id), "expected_revision": .signedInteger(submission.revision),
            "code": .string("routing_failed"), "at_ms": timestamp])
    }

    func cancelPending(_ submission: MacRoomSubmission) throws -> MacRoomSubmission {
        try command(["type": .string("cancel_pending"), "room_id": .string(submission.input.roomID),
            "submission_id": .string(submission.id), "expected_revision": .signedInteger(submission.revision),
            "at_ms": timestamp])
    }
    private let execute: @Sendable (MacCanonicalJSONValue) throws -> MacCanonicalJSONValue
    private let resumeExpertAdmission: @Sendable (AppleAdmitExpertConversationMessage) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue>?
    private let expertHost: @Sendable (AppleExpertHostOperation<MacCanonicalJSONValue>) throws -> MacCanonicalJSONValue

    init(execute: @escaping @Sendable (MacCanonicalJSONValue) throws -> MacCanonicalJSONValue,
         expertHost: @escaping @Sendable (AppleExpertHostOperation<MacCanonicalJSONValue>) throws -> MacCanonicalJSONValue = { _ in throw MacApplicationPortError.serviceUnavailable },
         resumeExpertAdmission: @escaping @Sendable (AppleAdmitExpertConversationMessage) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue>? = { _ in nil }) {
        self.execute = execute
        self.expertHost = expertHost
        self.resumeExpertAdmission = resumeExpertAdmission
    }

    convenience init(runtime: MacApplicationPort) {
        self.init(execute: { try runtime.executeAgentRoomEnvelope($0) }, expertHost: { try runtime.executeExpertHost($0) }, resumeExpertAdmission: { try runtime.resumePendingExpertAdmission($0) })
    }

    private func command<Result: Decodable>(_ value: [String: MacCanonicalJSONValue]) throws -> Result {
        let result = try execute(.object(["type": .string("agent_room"), "command": .object(value)]))
        return try JSONDecoder().decode(Result.self, from: JSONEncoder().encode(result))
    }

    func childConversations(conversationID: String, limit: Int = 32, offset: Int = 0) throws -> AppleExpertChildConversations {
        let page: AppleExpertChildConversations = try command(["type": .string("child_conversations"),
            "conversation_id": .string(conversationID), "limit": .signedInteger(Int64(limit)), "offset": .signedInteger(Int64(offset))])
        try page.validate(parentID: conversationID, offset: offset)
        return page
    }

    func childHistory(parentID: String, childID: String, before: Int64? = nil) throws -> AppleExpertChildHistory<MacCanonicalConversationWindow> {
        let page: AppleExpertChildHistory<MacCanonicalConversationWindow> = try command(["type": .string("child_history"),
            "conversation_id": .string(parentID), "child_conversation_id": .string(childID),
            "before_ordinal": before.map(MacCanonicalJSONValue.signedInteger) ?? .null,
            "limit": .signedInteger(80), "content_byte_limit": .signedInteger(512_000)])
        try page.validate(parentID: parentID, childID: childID)
        guard page.window.projection.conversationID == childID,
              !page.window.hasMore || (page.window.beforeOrdinal != nil && !page.window.projection.messages.isEmpty),
              before == nil || !page.window.hasMore || page.window.beforeOrdinal! < before! else { throw MacApplicationPortError.invalidResponse }
        return page
    }

    struct UserMessageAdmission: Codable, Equatable, Sendable {
        let conversationID: String
        let messageID: String
        let text: String
        let createdAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case conversationID = "conversation_id", messageID = "message_id", text
            case createdAtMs = "created_at_ms"
        }
    }

    func expertMessageSource(conversationID: String, messageID: String) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue>? {
        let value = try expertHost(.loadManifest(conversationID: conversationID, messageID: messageID, authority: .local()))
        if value == .null { return nil }
        return try JSONDecoder().decode(AppleExpertMessageManifest<MacCanonicalJSONValue>.self, from: JSONEncoder().encode(value))
    }

    func expertAdmissionInput(_ input: UserMessageAdmission, agentIDs: [String], association: MacConversationRoomAssociation)
        throws -> AppleAdmitExpertConversationMessage {
        let request: AppleAdmitExpertConversationMessage
        if let previous = try expertMessageSource(conversationID: input.conversationID, messageID: input.messageID) {
            guard previous.input.text == input.text,
                  Set(previous.input.selected_participants.map(\.agent_id)) == Set(agentIDs) else {
                throw MacApplicationPortError.rejected("The complete pending expert selection changed")
            }
            request = previous.input
        } else {
            struct Revision: Decodable { let revision: Int64 }
            let value = try execute(.object(["type": .string("load_conversation"), "conversation_id": .string(input.conversationID)]))
            let revision = try JSONDecoder().decode(Revision.self, from: JSONEncoder().encode(value)).revision
            let refs = try agentIDs.sorted().map { id -> AppleExpertParticipantReference in
                let matches = association.room.room.participants.filter { $0.active && $0.agentID == id }
                guard matches.count == 1, let participant = matches.first else {
                    throw MacApplicationPortError.rejected("Selected expert context is unavailable")
                }
                return .init(participant_id: participant.id, agent_id: participant.agentID,
                    agent_revision: participant.agentRevision, context_conversation_id: participant.contextConversationID)
            }
            request = .init(conversation_id: input.conversationID, expected_revision: revision,
                room_id: association.roomID, expected_room_revision: association.room.revision,
                submission_id: input.messageID, user_message_id: input.messageID, text: input.text,
                selected_participants: refs, created_at_ms: input.createdAtMs)
        }
        return request
    }

    func resumePendingExpertAdmission(_ input: AppleAdmitExpertConversationMessage) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue>? {
        try resumeExpertAdmission(input)
    }

    func expertChildInstructions(conversationID: String, expectedRunID: String) throws -> [AppleExpertPromptDefinition<MacCanonicalJSONValue>]? {
        let value = try expertHost(.loadChildInstructions(conversationID: conversationID, expectedRunID: expectedRunID, authority: .local()))
        if value == .null { return nil }
        return try JSONDecoder().decode([AppleExpertPromptDefinition<MacCanonicalJSONValue>].self, from: JSONEncoder().encode(value))
    }

    func previewExpertCapture(_ input: AppleAdmitExpertConversationMessage) throws -> AppleExpertCapturePreview<MacCanonicalJSONValue> {
        let value = try expertHost(.preview(input: input, authority: .local()))
        return try JSONDecoder().decode(AppleExpertCapturePreview<MacCanonicalJSONValue>.self, from: JSONEncoder().encode(value))
    }

    func admitExperts(_ request: AppleAdmitExpertConversationMessage, context: AppleExpertCaptureContext<MacCanonicalJSONValue>) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue> {
        let result = try expertHost(.admit(input: request, context: context))
        let manifest = try JSONDecoder().decode(AppleExpertMessageManifest<MacCanonicalJSONValue>.self, from: JSONEncoder().encode(result))
        guard manifest.version == 1, manifest.input == request,
              manifest.entries.map(\.participant) == request.selected_participants else {
            throw MacApplicationPortError.rejected("Complete expert admission receipt ownership mismatch")
        }
        return manifest
    }

    func loadPreparedExpertResend(_ target: String) throws -> ApplePreparedExpertResend<MacCanonicalJSONValue>? {
        let value = try expertHost(.loadPreparedResend(conversationID: target, authority: .local()))
        if value == .null { return nil }
        return try JSONDecoder().decode(ApplePreparedExpertResend<MacCanonicalJSONValue>.self, from: JSONEncoder().encode(value))
    }
    func preparedEditedChild(_ target: String) throws -> ApplePreparedEditedResend<MacCanonicalJSONValue>? {
        let value = try execute(.object(["type": .string("load_prepared_edited_resend"), "conversation_id": .string(target)]))
        if value == .null { return nil }
        return try JSONDecoder().decode(ApplePreparedEditedResend<MacCanonicalJSONValue>.self, from: JSONEncoder().encode(value))
    }

    func forConversation(_ conversationID: String) throws -> MacConversationRoomAssociation? {
        let association: MacConversationRoomAssociation? = try command([
            "type": .string("for_conversation"), "conversation_id": .string(conversationID)])
        if let association { try validate(association, conversationID: conversationID) }
        return association
    }

    func ensureForConversation(_ conversationID: String) throws -> MacConversationRoomAssociation {
        let association: MacConversationRoomAssociation = try command([
            "type": .string("ensure_for_conversation"), "conversation_id": .string(conversationID),
            "at_ms": timestamp])
        try validate(association, conversationID: conversationID)
        return association
    }

    private func validate(_ association: MacConversationRoomAssociation, conversationID: String) throws {
        guard association.conversationID == conversationID,
              association.roomID == association.room.room.id else {
            throw MacApplicationPortError.rejected("Expert conversation association ownership mismatch")
        }
    }

    /// Only the stored participant matching both receipt identities may label a reply.
    static func sender(for receipt: MacRoomSubmission,
                       association: MacConversationRoomAssociation) -> MacRoomParticipant? {
        guard receipt.input.roomID == association.roomID,
              receipt.input.conversationID == association.conversationID,
              receipt.input.parentMessageID?.isEmpty == false,
              let participantID = receipt.selectedParticipantID,
              let contextID = receipt.contextConversationID else { return nil }
        return association.room.room.participants.first {
            $0.id == participantID && $0.contextConversationID == contextID
        }
    }

    func roleTemplates() throws -> AppleExpertRoleTemplateCatalog {
        try command(["type": .string("role_templates")])
    }

    func directory() throws -> MacAgentDirectory {
        try command(["type": .string("directory")])
    }

    func submit(_ input: MacRoomSubmissionInput) throws -> MacRoomSubmission {
        guard (input.parentMessageID == nil) == (input.conversationID == nil),
              input.parentMessageID.map({ !$0.isEmpty }) ?? true,
              input.conversationID.map({ !$0.isEmpty }) ?? true else {
            throw MacApplicationPortError.rejected("Expert submission requires both conversation and parent message identities")
        }
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(input))
        return try command(["type": .string("submit"), "input": value])
    }

    func submission(roomID: String, submissionID: String) throws -> MacRoomSubmission? {
        try command(["type": .string("submission"), "room_id": .string(roomID), "submission_id": .string(submissionID)])
    }

    func submissions(roomID: String, limit: Int = 100, offset: Int = 0) throws -> [MacRoomSubmission] {
        try command(["type": .string("submissions"), "room_id": .string(roomID),
                     "limit": .signedInteger(Int64(limit)), "offset": .signedInteger(Int64(offset))])
    }

    struct ExpertAdmissionDraft: Codable {
        let conversationID: String
        let input: UserMessageAdmission?
        let expertIDs: [String]
        var expertRevisions: [String: Int64]? = nil
        var completeInput: AppleAdmitExpertConversationMessage? = nil
    }

    func loadExpertAdmission(conversationID: String) throws -> (ExpertAdmissionDraft, Int64)? {
        let value = try execute(.object(["type": .string("get_product_setting"),
            "key": .string("apple.chat.expert-admission/" + conversationID)]))
        if value == .null { return nil }
        let setting = try JSONDecoder().decode(MacCanonicalProductSetting.self, from: JSONEncoder().encode(value))
        let draft = try JSONDecoder().decode(ExpertAdmissionDraft.self, from: Data(setting.valueJSON.utf8))
        guard draft.conversationID == conversationID,
              draft.input.map({ $0.conversationID == conversationID }) ?? true else {
            throw MacApplicationPortError.rejected("Expert admission draft ownership mismatch")
        }
        return (draft, setting.revision)
    }

    func saveExpertAdmission(_ draft: ExpertAdmissionDraft, expectedRevision: Int64?) throws -> Int64 {
        let data = try JSONEncoder().encode(draft)
        let result = try execute(.object(["type": .string("put_product_setting_checked"),
            "key": .string("apple.chat.expert-admission/" + draft.conversationID),
            "value_json": .string(String(decoding: data, as: UTF8.self)),
            "expected_revision": expectedRevision.map { .signedInteger($0) } ?? .null,
            "updated_at_ms": timestamp]))
        return try JSONDecoder().decode(MacCanonicalProductSetting.self, from: JSONEncoder().encode(result)).revision
    }

    private struct StoredDraft: Codable {
        let format: Int
        let roomID: String
        let draft: MacRoomDraft
        let pending: [MacAgentRoomWorkspace.Pending]
    }

    func loadDraft(roomID: String) throws -> (draft: MacRoomDraft, pending: [MacAgentRoomWorkspace.Pending], revision: Int64)? {
        let value = try execute(.object(["type": .string("get_product_setting"),
            "key": .string("apple.room.draft/" + roomID)]))
        if value == .null { return nil }
        let setting = try JSONDecoder().decode(MacCanonicalProductSetting.self, from: JSONEncoder().encode(value))
        let stored = try JSONDecoder().decode(StoredDraft.self, from: Data(setting.valueJSON.utf8))
        guard stored.format == 1, stored.roomID == roomID else {
            throw MacApplicationPortError.rejected("Unsupported room draft format or owner")
        }
        guard stored.pending.allSatisfy({ $0.input.roomID == roomID }) else {
            throw MacApplicationPortError.rejected("Room submission draft ownership mismatch")
        }
        return (stored.draft, stored.pending, setting.revision)
    }

    func saveDraft(_ draft: MacRoomDraft, pending: [MacAgentRoomWorkspace.Pending], roomID: String, expectedRevision: Int64?) throws -> Int64 {
        let data = try JSONEncoder().encode(StoredDraft(format: 1, roomID: roomID, draft: draft, pending: pending))
        let result = try execute(.object(["type": .string("put_product_setting_checked"),
            "key": .string("apple.room.draft/" + roomID),
            "value_json": .string(String(decoding: data, as: UTF8.self)),
            "expected_revision": expectedRevision.map { .signedInteger($0) } ?? .null,
            "updated_at_ms": timestamp]))
        return try JSONDecoder().decode(MacCanonicalProductSetting.self, from: JSONEncoder().encode(result)).revision
    }

    func list(limit: Int = 50, offset: Int = 0) throws -> [MacAgentRoomSnapshot] {
        try command(["type": .string("list"), "limit": .signedInteger(Int64(limit)), "offset": .signedInteger(Int64(offset))])
    }

    func load(id: String) throws -> MacAgentRoomSnapshot? {
        try command(["type": .string("read"), "room_id": .string(id)])
    }

    func create(title: String) throws -> MacAgentRoomSnapshot {
        try command(["type": .string("create"), "title": .string(title), "at_ms": timestamp])
    }

    func rejoin(agent: MacPersistentAgent, room: MacAgentRoomSnapshot) throws -> MacAgentRoomSnapshot {
        try command(["type": .string("rejoin"), "input": .object([
            "room_id": .string(room.room.id), "expected_room_revision": .signedInteger(room.revision),
            "agent_id": .string(agent.id), "expected_agent_revision": .signedInteger(agent.revision), "at_ms": timestamp
        ])])
    }

    func join(agent: MacPersistentAgent, room: MacAgentRoomSnapshot) throws -> MacAgentRoomSnapshot {
        try command(["type": .string("join"), "input": .object([
            "room_id": .string(room.room.id), "expected_room_revision": .signedInteger(room.revision),
            "agent_id": .string(agent.id), "expected_agent_revision": .signedInteger(agent.revision),
            "at_ms": timestamp
        ])])
    }

    func deactivate(participantID: String, room: MacAgentRoomSnapshot) throws -> MacAgentRoomSnapshot {
        try command(["type": .string("deactivate"), "room_id": .string(room.room.id),
            "participant_id": .string(participantID), "expected_revision": .signedInteger(room.revision),
            "at_ms": timestamp])
    }

    func createAgent(name: String, description: String, defaults: MacCanonicalJSONValue, directory: MacAgentDirectory,
                     id: String = UUID().uuidString.lowercased()) throws -> MacAgentDirectory {
        try command(["type": .string("configure_agent"), "input": .object([
            "id": .string(id), "name": .string(name), "description": .string(description),
            "expected_directory_revision": directory.revision.map { .signedInteger($0) } ?? .null,
            "expected_agent_revision": .null, "defaults": defaults,
            "availability": .string("active"), "updated_at_ms": timestamp
        ])])
    }

    func updateAgent(_ agent: MacPersistentAgent, name: String, description: String,
                     availability: String, directory: MacAgentDirectory,
                     defaults: MacCanonicalJSONValue? = nil) throws -> MacAgentDirectory {
        try command(["type": .string("configure_agent"), "input": .object([
            "id": .string(agent.id), "name": .string(name), "description": .string(description),
            "expected_directory_revision": directory.revision.map { .signedInteger($0) } ?? .null,
            "expected_agent_revision": .signedInteger(agent.revision), "defaults": defaults ?? agent.defaults,
            "availability": .string(availability), "updated_at_ms": timestamp
        ])])
    }

    private var timestamp: MacCanonicalJSONValue { .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)) }
}
