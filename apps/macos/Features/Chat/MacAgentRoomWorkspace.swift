// SPDX-License-Identifier: Apache-2.0
// Room presentation owns drafts and receipt feedback; Core owns dispatch.
import Foundation
import Combine

/// Composer chips are selections, not a projection of the asynchronously loaded directory.
nonisolated struct MacExpertSelectionChip: Identifiable, Equatable {
    let id: String
    let name: String

    static func rows(agentIDs: Set<String>, observedRevisions: [String: Int64],
                     directory: [MacPersistentAgent], joined: [MacRoomParticipant],
                     fallbackName: String = String(localized: "Expert")) -> [Self] {
        agentIDs.sorted().map { id in
            let revision = observedRevisions[id]
            let member = joined.first { $0.active && $0.agentID == id
                && (revision == nil || $0.agentRevision == revision) }
            let agent = directory.first { $0.id == id && (revision == nil || $0.revision == revision) }
            let name = [member?.displayName, agent?.name].compactMap { $0 }
                .first { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } ?? fallbackName
            return Self(id: id, name: name)
        }
    }
}

nonisolated struct MacRoomDraft: Codable, Equatable, Sendable {
    var text = ""
    var participantID: String?
    var revision: UInt64 = 0

    var mentionQuery: String? { ExpertMentionQuery.atEnd(of: text)?.query }

    mutating func mention(_ id: String) {
        if mentionQuery != nil, let at = text.lastIndex(of: "@") { text = String(text[..<at]) }
        participantID = id
        revision &+= 1
    }

    mutating func acknowledge(revision submitted: UInt64) {
        guard revision == submitted else { return }
        text = ""
        participantID = nil
        revision &+= 1
    }
}

nonisolated enum MacExpertMentionPreparationError: Error { case reintroductionRequired }

@MainActor final class MacAgentRoomWorkspace: ObservableObject {
    nonisolated struct Pending: Codable, Equatable, Identifiable, Sendable {
        let input: MacRoomSubmissionInput
        let draftRevision: UInt64
        var failure: String?
        var sending = true
        var id: String { input.submissionID }
    }
    let service: MacAgentRoomService
    var executionService: MacRoomExecutionService?
    var agentReasoningEfforts: (@MainActor (String) throws -> [String])?
    var agentModelOptions: (@MainActor () throws -> [MacChatModelOption])?
    var requiresMainSource = false
    @Published private(set) var mainSourceReady = false
    var mainCommand: (@MainActor (String?, MacCanonicalJSONValue) async throws -> MacCanonicalJSONValue)? {
        didSet { mainSourceReady = mainCommand != nil }
    }
    var mainIsRemote: (@MainActor () -> Bool)?
    var mainSourceEpoch: (@MainActor (String) -> String?)?
    var mainSourceIdentity: (@MainActor (String) -> String?)?
    private var mainConversationID: String?
    private var selectedSourceEpochs: [String: [String: String]] = [:]
    private var selectedSourceIDs: [String: [String: String]] = [:]

    private func mainRead<T: Decodable>(_ command: [String: MacCanonicalJSONValue], owner: String?) async throws -> T {
        guard let mainCommand else { throw MacApplicationPortError.serviceUnavailable }
        let value = try await mainCommand(owner, .object(command))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }

    func capturedMainSourceEpoch(owner: String, agentIDs: Set<String>) throws -> String? {
        guard mainIsRemote?() == true else {
            guard agentIDs.allSatisfy({ selectedSourceIDs[owner]?[$0] == nil }) else {
                throw MacApplicationPortError.rejected("The expert source changed. Select the experts again before sending.")
            }
            return nil
        }
        let epochs = Set(agentIDs.compactMap { selectedSourceEpochs[owner]?[$0] })
        let identities = Set(agentIDs.compactMap { selectedSourceIDs[owner]?[$0] })
        guard epochs.count == 1, identities.count == 1, let epoch = epochs.first,
              let identity = identities.first, mainSourceIdentity?(owner) == identity,
              mainSourceEpoch?(owner) == epoch,
              agentIDs.allSatisfy({ selectedSourceEpochs[owner]?[$0] == epoch && selectedSourceIDs[owner]?[$0] == identity }) else {
            throw MacApplicationPortError.rejected("The expert Host changed. Select the experts again before sending.")
        }
        return epoch
    }

    @Published var rooms: [MacAgentRoomSnapshot] = []
    @Published var directory: [MacPersistentAgent] = []
    private var mentionedExpertRevisions: [String: [String: Int64]] = [:]
    @Published private(set) var selectedRoom: MacAgentRoomSnapshot?
    @Published private(set) var drafts: [String: MacRoomDraft] = [:]
    @Published private(set) var submissions: [String: [MacRoomSubmission]] = [:]
    @Published private(set) var pending: [String: [Pending]] = [:]
    @Published private(set) var errors: [String: String] = [:]
    @Published var directoryError: String?
    @Published private(set) var conversationAssociation: MacConversationRoomAssociation?
    @Published private(set) var admissionError: String?
    @Published private(set) var isAdmitting = false
    private var pendingAdmission: MacAgentRoomService.UserMessageAdmission?
    @Published private(set) var resumableExpertMessages: [String: AppleExpertMessageManifest<MacCanonicalJSONValue>] = [:]
    @Published private(set) var resumingExpertMessage: String?
    @Published private(set) var expertResumeError: String?
    private var pendingCompleteExpertInput: AppleAdmitExpertConversationMessage?
    private var pendingExpertRevisions: [String: Int64] = [:]
    @Published private(set) var pendingExpertIDs: [String] = []
    private var restoreEpoch: UInt64 = 0
    var pendingAdmissionText: String? { pendingAdmission?.text }
    private var admissionStorageRevision: Int64?
    private var admissionOwner: String?

    /// One complete native Host capture/admission; retries retain the original business intent.
    private func admitCompleteExpertIntent(_ input: AppleAdmitExpertConversationMessage) async throws -> AppleExpertMessageManifest<MacCanonicalJSONValue> {
        let service = service
        if let existing = try await Task.detached(operation: { try service.expertMessageSource(conversationID: input.conversation_id, messageID: input.user_message_id) }).value {
            guard existing.input == input else { throw MacApplicationPortError.rejected("The complete expert intent changed") }
            if let reconciled: AppleExpertMessageManifest<MacCanonicalJSONValue> = try await Task.detached(operation: { try service.resumePendingExpertAdmission(input) }).value { return reconciled }
            return existing
        }
        let preview = try await Task.detached { try service.previewExpertCapture(input) }.value
        guard preview.participants.map(\.participant) == input.selected_participants else { throw MacApplicationPortError.invalidResponse }
        let context = try MacExpertInstructionProducer.capture(preview)
        return try await Task.detached { try service.admitExperts(input, context: context) }.value
    }

    /// Continue the current joined snapshot, never silently select a newer directory revision.
    func selectCurrentChild(_ child: AppleExpertChildConversation, parentID: String) async throws {
        guard child.isCurrent, child.active, let revision = child.agentRevision else {
            throw MacApplicationPortError.rejected(String(localized: "This expert history is read only"))
        }
        let service = service
        let association = try await MacConversationReadTask.run { try service.forConversation(parentID) }
        guard let association, association.conversationID == parentID,
              association.room.room.participants.contains(where: {
                  $0.active && $0.id == child.participantID && $0.agentID == child.agentID
                  && $0.agentRevision == revision && $0.contextConversationID == child.conversationID
              }) else { throw MacApplicationPortError.rejected(String(localized: "This expert history is read only")) }
        mentionedExpertRevisions[parentID, default: [:]][child.agentID] = revision
    }

    func restoreConversation(_ conversationID: String) async {
        mainConversationID = conversationID
        restoreEpoch &+= 1
        let mainReadEpoch = restoreEpoch
        guard !isAdmitting else { return }
        if mainIsRemote?() == true, mainCommand != nil {
            do {
                let association: MacConversationRoomAssociation? = try await mainRead(["type": .string("for_conversation"), "conversation_id": .string(conversationID)], owner: conversationID)
                guard mainConversationID == conversationID, restoreEpoch == mainReadEpoch, mainIsRemote?() == true else { return }
                conversationAssociation = association; directoryError = nil
            } catch { if mainConversationID == conversationID { conversationAssociation = nil; directoryError = error.localizedDescription } }
            return
        }
        restoreEpoch &+= 1
        let epoch = restoreEpoch
        conversationAssociation = nil
        resumableExpertMessages = [:]
        resumingExpertMessage = nil
        expertResumeError = nil
        let service = service
        do {
            let restored = try await Task.detached {
                (try service.forConversation(conversationID), try service.loadExpertAdmission(conversationID: conversationID))
            }.value
            guard epoch == restoreEpoch, !Task.isCancelled, !isAdmitting else { return }
            if let association = restored.0 {
                await select(association.room, resumeExecutions: false)
                guard epoch == restoreEpoch, !Task.isCancelled, !isAdmitting else { return }
                conversationAssociation = association
            }
            admissionOwner = conversationID
            admissionStorageRevision = restored.1?.1
            pendingAdmission = restored.1?.0.input
            pendingCompleteExpertInput = restored.1?.0.completeInput
            pendingExpertIDs = restored.1?.0.expertIDs ?? []
            pendingExpertRevisions = restored.1?.0.expertRevisions ?? [:]
            admissionError = pendingAdmission == nil ? nil : "An expert message is pending. Retry sends the original message."
        } catch { admissionError = error.localizedDescription }
    }


    /// Freeze the entire user message before any suspension; retry never substitutes edited text.
    func recordMainExpertSelection(_ agent: MacPersistentAgent, conversationID: String) {
        mentionedExpertRevisions[conversationID, default: [:]][agent.id] = agent.revision
        if let epoch = mainSourceEpoch?(conversationID), let identity = mainSourceIdentity?(conversationID) {
            selectedSourceEpochs[conversationID, default: [:]][agent.id] = epoch
            selectedSourceIDs[conversationID, default: [:]][agent.id] = identity
        } else {
            selectedSourceEpochs[conversationID]?[agent.id] = nil
            selectedSourceIDs[conversationID]?[agent.id] = nil
        }
    }

    func selectionChips(conversationID: String?, agentIDs: Set<String>) -> [MacExpertSelectionChip] {
        let association = conversationAssociation?.conversationID == conversationID ? conversationAssociation : nil
        return MacExpertSelectionChip.rows(agentIDs: agentIDs,
            observedRevisions: conversationID.flatMap { mentionedExpertRevisions[$0] } ?? [:],
            directory: directory, joined: association?.room.room.participants ?? [])
    }

    func mainExpertSelection(conversationID: String, agentIDs: Set<String>) async throws -> [AppleMainExpertSelectionEntry] {
        guard !requiresMainSource || mainSourceReady else { throw MacApplicationPortError.serviceUnavailable }
        let service = service
        _ = try capturedMainSourceEpoch(owner: conversationID, agentIDs: agentIDs)
        let room: MacConversationRoomAssociation?
        if mainCommand != nil { room = try await mainRead(["type": .string("for_conversation"), "conversation_id": .string(conversationID)], owner: conversationID) }
        else { room = try await MacConversationReadTask.run { try service.forConversation(conversationID) } }
        let joined = (room?.room.room.participants ?? []).filter(\.active).map {
            AppleExpertParticipantReference(participant_id: $0.id, agent_id: $0.agentID,
                agent_revision: $0.agentRevision, context_conversation_id: $0.contextConversationID)
        }
        return try AppleMainExpertSelectionContext.capture(agentIDs: agentIDs,
            observedRevisions: mentionedExpertRevisions[conversationID] ?? [:], joined: joined).selectedExperts
    }

    func sendExperts(conversationID: String, text: String, agentIDs: Set<String>, messageID: String? = nil, createdAtMs: Int64? = nil) async -> String? {
        guard !isAdmitting, !agentIDs.isEmpty else { return nil }
        if admissionOwner != conversationID {
            await restoreConversation(conversationID)
            guard admissionOwner == conversationID, !isAdmitting else { return nil }
        }
        if let pendingAdmission {
            guard pendingAdmission.conversationID == conversationID,
                  messageID == nil || (pendingAdmission.messageID == messageID && pendingAdmission.text == text
                    && pendingAdmission.createdAtMs == createdAtMs) else {
                admissionError = "Retry the pending expert message in its original conversation."
                return nil
            }
        } else {
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            pendingAdmission = .init(conversationID: conversationID,
                messageID: messageID ?? UUID().uuidString.lowercased(), text: text,
                createdAtMs: createdAtMs ?? Int64(Date().timeIntervalSince1970 * 1000))
            pendingExpertIDs = agentIDs.sorted()
            pendingExpertRevisions = (mentionedExpertRevisions[conversationID] ?? [:]).filter { agentIDs.contains($0.key) }
        }
        guard let input = pendingAdmission else { return nil }
        // A new explicit @ selection may repair an unadmitted stale selection.
        // Never change an already captured complete Core admission.
        if pendingCompleteExpertInput == nil {
            for (id, revision) in mentionedExpertRevisions[conversationID] ?? [:] where pendingExpertIDs.contains(id) {
                pendingExpertRevisions[id] = revision
            }
        }
        isAdmitting = true
        defer { isAdmitting = false }
        do {
            let service = service
            let frozen = MacAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID,
                input: input, expertIDs: pendingExpertIDs, expertRevisions: pendingExpertRevisions, completeInput: pendingCompleteExpertInput)
            let previousRevision = admissionStorageRevision
            admissionStorageRevision = try await Task.detached {
                try service.saveExpertAdmission(frozen, expectedRevision: previousRevision)
            }.value
            var association = try await Task.detached { try service.ensureForConversation(input.conversationID) }.value
            let available = try await Task.detached { try service.directory().agents }.value
            for id in pendingExpertIDs {
                let member = association.room.room.participants.first { $0.agentID == id && $0.active }
                if pendingCompleteExpertInput == nil, let observed = pendingExpertRevisions[id] {
                    // Joined contexts own an immutable expert snapshot. Only an
                    // unjoined selection must still match the fresh directory.
                    let revision = member?.agentRevision
                        ?? available.first(where: { $0.id == id && $0.availability == "active" })?.revision
                    guard revision == observed else {
                        throw MacApplicationPortError.rejected(String(localized: "The selected expert changed. Select it again before sending."))
                    }
                }
                if member != nil { continue }
                guard let agent = available.first(where: { $0.id == id && $0.availability == "active" }) else {
                    throw MacApplicationPortError.rejected("Selected expert is unavailable")
                }
                let room = association.room
                _ = try await Task.detached { try service.join(agent: agent, room: room) }.value
                association = try await Task.detached { try service.ensureForConversation(input.conversationID) }.value
            }
            await select(association.room, resumeExecutions: false)
            guard selectedRoom?.room.id == association.roomID,
                  selectedRoom?.revision == association.room.revision else {
                throw MacApplicationPortError.rejected("Expert conversation could not be loaded")
            }
            let selectedIDs = pendingExpertIDs
            let frozenAssociation = association
            let complete: AppleAdmitExpertConversationMessage
            if let pendingCompleteExpertInput { complete = pendingCompleteExpertInput }
            else { complete = try await Task.detached { try service.expertAdmissionInput(input, agentIDs: selectedIDs, association: frozenAssociation) }.value }
            pendingCompleteExpertInput = complete
            let saved = MacAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID, input: input,
                expertIDs: selectedIDs, expertRevisions: pendingExpertRevisions, completeInput: complete)
            let completeRevision = admissionStorageRevision
            admissionStorageRevision = try await Task.detached { try service.saveExpertAdmission(saved, expectedRevision: completeRevision) }.value
            let manifest = try await admitCompleteExpertIntent(complete)
            conversationAssociation = association
            for entry in manifest.entries {
                let exact = try await Task.detached { try service.submission(roomID: frozenAssociation.roomID, submissionID: entry.child_submission_id) }.value
                guard let receipt = exact,
                      receipt.contextConversationID == entry.participant.context_conversation_id,
                      receipt.input.parentMessageID == manifest.input.user_message_id,
                      receipt.selectedParticipantID == entry.participant.participant_id else {
                    throw MacApplicationPortError.rejected("Complete expert child receipt is unavailable")
                }
                applyReceipt(receipt)
                if ["selected", "executing"].contains(receipt.state) { Task { await self.startSelected(receipt) } }
            }
            let cleared = MacAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID, input: nil, expertIDs: [])
            let clearRevision = admissionStorageRevision
            admissionStorageRevision = try await Task.detached {
                try service.saveExpertAdmission(cleared, expectedRevision: clearRevision)
            }.value
            pendingAdmission = nil
            pendingCompleteExpertInput = nil
            pendingExpertIDs = []
            pendingExpertRevisions = [:]
            admissionError = nil
            return input.text
        } catch { admissionError = error.localizedDescription; return nil }
    }

    func deliverPreparedExpertResend(_ plan: ApplePreparedExpertResend<MacCanonicalJSONValue>) async throws {
        let service = service
        let association = try await Task.detached { try service.forConversation(plan.input.target_conversation_id) }.value
        guard !plan.entries.isEmpty, let association,
              association.roomID == plan.room_id else { throw MacEditedResendError.sourceUnavailable }
        for entry in plan.entries {
            let exact = try await Task.detached { try service.submission(roomID: plan.room_id, submissionID: entry.prepared.run_id) }.value
            guard let receipt = exact,
                  receipt.contextConversationID == entry.participant.context_conversation_id,
                  receipt.selectedParticipantID == entry.participant.participant_id,
                  receipt.input.parentMessageID == plan.parent_message_id else { throw MacEditedResendError.sourceUnavailable }
            applyReceipt(receipt)
            if entry.prepared.admitted_run_id == entry.prepared.run_id {
                executionService?.observe(contextID: entry.participant.context_conversation_id)
            } else if ["selected", "executing"].contains(receipt.state) {
                Task { await self.startSelected(receipt) }
            }
        }
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            let refreshed = try await Task.detached { try service.loadPreparedExpertResend(plan.input.target_conversation_id) }.value
            guard let observed = refreshed,
                  observed.input == plan.input, observed.entries.count == plan.entries.count else { throw MacEditedResendError.sourceUnavailable }
            let confirmed = observed.entries.filter { $0.prepared.admitted_run_id == $0.prepared.run_id }.count
            if confirmed == observed.entries.count {
                await select(association.room, resumeExecutions: false)
                conversationAssociation = association
                admissionError = nil; return
            }
            let status = "Expert delivery confirmed: \(confirmed)/\(observed.entries.count). Remaining experts retain this same edit intent."
            if admissionError != status { admissionError = status }
            try await Task.sleep(for: .milliseconds(500))
        }
        throw MacApplicationPortError.rejected(admissionError ?? MacEditedResendError.admissionUnconfirmed.localizedDescription)
    }

    func refreshResumableExpertMessages(conversationID: String, messageIDs: [String]) async {
        guard let association = conversationAssociation, association.conversationID == conversationID else { return }
        let epoch = restoreEpoch, service = service
        func current() -> Bool { !Task.isCancelled && epoch == restoreEpoch && conversationAssociation?.conversationID == conversationID }
        do {
            for parent in Set(messageIDs).sorted() {
                guard current() else { return }
                guard let manifest = try await Task.detached(operation: {
                    try service.expertMessageSource(conversationID: conversationID, messageID: parent)
                }).value else { continue }
                guard manifest.input.conversation_id == conversationID, manifest.input.user_message_id == parent,
                      manifest.input.room_id == association.roomID, manifest.entries.map(\.participant) == manifest.input.selected_participants else {
                    throw MacApplicationPortError.invalidResponse
                }
                var missing = false
                for entry in manifest.entries {
                    let child = entry.child_submission_id
                    guard let receipt = try await Task.detached(operation: {
                        try service.submission(roomID: association.roomID, submissionID: child)
                    }).value, receipt.contextConversationID == entry.participant.context_conversation_id,
                          receipt.selectedParticipantID == entry.participant.participant_id, receipt.input.parentMessageID == parent else {
                        throw MacApplicationPortError.invalidResponse
                    }
                    guard current() else { return }
                    applyReceipt(receipt)
                    if ["selected", "executing"].contains(receipt.state) {
                        let context = entry.participant.context_conversation_id
                        let exists = try await Task.detached { try service.executionExists(contextID: context, executionID: child) }.value
                        guard current() else { return }
                        if exists { executionService?.observe(contextID: context); observeExecution(receipt) }
                        else { missing = true }
                    }
                }
                if missing { resumableExpertMessages[parent] = manifest } else { resumableExpertMessages[parent] = nil }
            }
        } catch { if current() { expertResumeError = error.localizedDescription } }
    }

    func resumeCompleteExpertMessage(_ messageID: String) async {
        guard !isAdmitting, resumingExpertMessage == nil, let original = resumableExpertMessages[messageID],
              conversationAssociation?.conversationID == original.input.conversation_id else { return }
        let epoch = restoreEpoch, service = service
        func current() -> Bool { !Task.isCancelled && epoch == restoreEpoch && conversationAssociation?.conversationID == original.input.conversation_id }
        resumingExpertMessage = messageID; expertResumeError = nil
        defer { if current() { resumingExpertMessage = nil } }
        do {
            if let prepared = try await Task.detached(operation: { try service.loadPreparedExpertResend(original.input.conversation_id) }).value {
                guard current() else { return }
                try await deliverPreparedExpertResend(prepared)
                await refreshResumableExpertMessages(conversationID: original.input.conversation_id, messageIDs: [messageID])
                return
            }
            let plan = try await admitCompleteExpertIntent(original.input)
            guard plan.input == original.input, plan.entries.map(\.child_submission_id) == original.entries.map(\.child_submission_id) else { throw MacApplicationPortError.invalidResponse }
            var receipts: [MacRoomSubmission] = []
            for entry in plan.entries {
                guard current() else { return }
                let child = entry.child_submission_id
                guard let receipt = try await Task.detached(operation: { try service.submission(roomID: plan.input.room_id, submissionID: child) }).value,
                      receipt.contextConversationID == entry.participant.context_conversation_id,
                      receipt.selectedParticipantID == entry.participant.participant_id, receipt.input.parentMessageID == messageID else { throw MacApplicationPortError.invalidResponse }
                receipts.append(receipt)
            }
            guard current() else { return }
            for receipt in receipts {
                applyReceipt(receipt)
                guard let context = receipt.contextConversationID else { throw MacApplicationPortError.invalidResponse }
                let exists = try await Task.detached { try service.executionExists(contextID: context, executionID: receipt.id) }.value
                guard current() else { return }
                if exists { executionService?.observe(contextID: context); observeExecution(receipt) }
                else if ["selected", "executing"].contains(receipt.state) { Task {
                    await startSelected(receipt)
                    await refreshResumableExpertMessages(conversationID: original.input.conversation_id, messageIDs: [messageID])
                } }
            }
            resumableExpertMessages[messageID] = nil
        } catch { if current() { expertResumeError = error.localizedDescription } }
    }

    private var selectionEpoch: UInt64 = 0
    private var draftStorageRevisions: [String: Int64] = [:]
    private var loadedDrafts = Set<String>()
    private var savingDrafts: [String: Task<Void, Never>] = [:]
    private var durablePendingIDs: [String: Set<String>] = [:]

    private let executeOwner: (@MainActor (MacRoomSubmission, MacAgentRoomService.ExecutionIntent) async throws -> Void)?
    var prepareOwner: (@MainActor (MacRoomSubmission) async throws -> MacAgentRoomService.ExecutionIntent)?
    var recoverOwner: (@MainActor (MacRoomSubmission, MacAgentRoomService.ExecutionIntent) async throws -> Void)?
    var cancelPreparedOwner: (@MainActor (MacRoomSubmission) async throws -> Void)?
    @Published private var startingExecutions = Set<String>()
    private var deliveryTasks: [String: Task<Void, Never>] = [:]
    var routeSubmission: (@MainActor (MacRoomSubmission, MacAgentRoomSnapshot) async throws -> MacRoomSubmission)?
    private var executionObservers: [String: Task<Void, Never>] = [:]
    func hasUnsettledExpertWork(conversationID: String?) -> Bool {
        guard let association = conversationAssociation, association.conversationID == conversationID else { return false }
        return isAdmitting || (submissions[association.roomID] ?? []).contains {
            $0.input.conversationID == conversationID && ["accepted", "selected", "executing"].contains($0.state)
        }
    }

    func isDeliveringExpertMessage(_ manifest: AppleExpertMessageManifest<MacCanonicalJSONValue>) -> Bool {
        isAdmitting || manifest.entries.contains {
            startingExecutions.contains($0.child_submission_id)
                || executionService?.owners[$0.participant.context_conversation_id]?.isRunning == true
        }
    }

    init(service: MacAgentRoomService,
         executeOwner: (@MainActor (MacRoomSubmission, MacAgentRoomService.ExecutionIntent) async throws -> Void)? = nil) {
        self.service = service
        self.executeOwner = executeOwner
    }

    func startSelected(_ receipt: MacRoomSubmission) async {
        guard ["selected", "executing"].contains(receipt.state), let executeOwner,
              let prepareOwner, startingExecutions.insert(receipt.id).inserted else { return }
        defer { startingExecutions.remove(receipt.id) }
        let service = service
        do {
            let persisted = try await Task.detached { try service.executionIntent(receipt) }.value
            let intent: MacAgentRoomService.ExecutionIntent
            if let persisted { intent = persisted } else { intent = try await prepareOwner(receipt) }
            var prepared = receipt
            if persisted == nil {
                prepared = try await Task.detached { try service.prepareExecution(receipt, intent: intent) }.value
            }
            let toClaim = prepared
            let claim = try await Task.detached { try service.claimExecution(toClaim) }.value
            guard claim.submission.input == receipt.input else {
                throw MacApplicationPortError.rejected("Room claim ownership mismatch")
            }
            applyReceipt(claim.submission)
            guard claim.submission.state == "executing" else {
                if claim.submission.state == "cancelled" { try await cancelPreparedOwner?(claim.submission) }
                return
            }
            guard let context = claim.submission.contextConversationID,
                  claim.submission.executionID == intent.execution_id else {
                throw MacApplicationPortError.rejected("Room execution intent identity mismatch")
            }
            observeExecution(claim.submission)
            // Query before replaying the original admission. Existing Runs are
            // observed, never submitted again; absent Runs retain the exact
            // durable intent/ID and must pass revision checks in their owner.
            let exists: Bool
            if case .remoteHost = intent.envelope {
                // Remote admission is proven by the paired source receipt,
                // not by a device-local invocation row.
                exists = false
            } else {
                exists = try await Task.detached {
                    try service.executionExists(contextID: context, executionID: intent.execution_id)
                }.value
            }
            if exists {
                try await recoverOwner?(claim.submission, intent)
                return
            }
            try await executeOwner(claim.submission, intent)
            if case .remoteHost = intent.envelope {
                errors[receipt.input.roomID] = nil
            } else if try await Task.detached(operation: { try service.executionExists(contextID: context, executionID: intent.execution_id) }).value {
                errors[receipt.input.roomID] = nil
            }
            if let owner = receipt.input.conversationID, let parent = receipt.input.parentMessageID {
                await refreshResumableExpertMessages(conversationID: owner, messageIDs: [parent])
            }
        } catch {
            errors[receipt.input.roomID] = error.localizedDescription
            if let latest = try? service.submissions(roomID: receipt.input.roomID).first(where: { $0.id == receipt.id }),
               latest.input == receipt.input, latest.state == "cancelled" {
                applyReceipt(latest)
                do { try await cancelPreparedOwner?(latest) }
                catch { errors[receipt.input.roomID] = error.localizedDescription }
            }
        }
    }

    /// A late observation may update only its original owner and may never
    /// roll a newer cancellation/terminal receipt back to an earlier revision.
    func applyReceipt(_ update: MacRoomSubmission) {
        let roomID = update.input.roomID
        if let index = submissions[roomID]?.firstIndex(where: { $0.id == update.id }) {
            guard let current = submissions[roomID]?[index], current.input == update.input,
                  update.revision > current.revision else { return }
            submissions[roomID]?[index] = update
        } else { submissions[roomID, default: []].append(update) }
        if let parent = update.input.parentMessageID, let manifest = resumableExpertMessages[parent] {
            let terminalStates = Set(["completed", "incomplete", "blocked", "failed", "cancelled"])
            let receipts = submissions[roomID] ?? []
            if manifest.entries.allSatisfy({ entry in receipts.contains {
                $0.id == entry.child_submission_id && terminalStates.contains($0.state)
            } }) { resumableExpertMessages[parent] = nil }
        }
    }

    private func observeExecution(_ receipt: MacRoomSubmission) {
        guard receipt.state == "executing", executionObservers[receipt.id] == nil else { return }
        executionObservers[receipt.id] = Task { [weak self, service] in
            defer { self?.executionObservers[receipt.id] = nil }
            while !Task.isCancelled {
                // Core checks the exact canonical seal; missing/unfinished runs
                // remain unconfirmed, never converted to a client success.
                if let update = try? await Task.detached(operation: { try service.observeExecution(receipt) }).value {
                    guard let self, update.input == receipt.input else { return }
                    self.applyReceipt(update)
                    if let parent = update.input.parentMessageID, let owner = update.input.conversationID,
                       self.resumableExpertMessages[parent] != nil {
                        await self.refreshResumableExpertMessages(conversationID: owner, messageIDs: [parent])
                    }
                    if update.state != "executing" { return }
                }
                try? await Task.sleep(for: .seconds(2))
                guard self != nil else { return }
            }
        }
    }
    var draft: MacRoomDraft { selectedRoom.flatMap { drafts[$0.room.id] } ?? MacRoomDraft() }
    var canSend: Bool {
        guard let room = selectedRoom,
              loadedDrafts.contains(room.room.id),
              room.room.participants.contains(where: \.active),
              !draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return !(pending[room.room.id] ?? []).contains { $0.draftRevision == draft.revision }
    }
    var candidates: [MacRoomParticipant] {
        guard let query = draft.mentionQuery else { return [] }
        return (selectedRoom?.room.participants ?? []).filter {
            $0.active && (query.isEmpty || $0.displayName.localizedCaseInsensitiveContains(query))
        }
    }
    func edit(_ text: String) {
        guard let id = selectedRoom?.room.id else { return }
        var value = drafts[id] ?? MacRoomDraft()
        value.text = text; value.revision &+= 1
        drafts[id] = value
        persistDraft(id)
    }
    func mention(_ participant: MacRoomParticipant) {
        guard let room = selectedRoom, room.room.participants.contains(participant), participant.active else { return }
        var value = draft; value.mention(participant.id); drafts[room.room.id] = value
        persistDraft(room.room.id)
    }
    func removeMention() {
        guard let id = selectedRoom?.room.id else { return }
        var value = draft; value.participantID = nil; value.revision &+= 1; drafts[id] = value
        persistDraft(id)
    }
    /// Explicit ID selection can include experts not yet joined. Blank unsaved
    /// composers validate directory identity only; send admission creates their room.
    func prepareExpertMention(agentID: String, expectedRevision: Int64, conversationID: String,
                              isPersisted: Bool, allowReintroduction: Bool = false) async throws {
        guard !requiresMainSource || mainSourceReady else { throw MacApplicationPortError.serviceUnavailable }
        if mainCommand != nil {
            let fresh: MacAgentDirectory = try await mainRead(["type": .string("directory")], owner: conversationID)
            guard let agent = fresh.agents.first(where: { $0.id == agentID && $0.availability == "active" }), agent.revision == expectedRevision else {
                throw MacApplicationPortError.rejected("The selected expert changed. Select it again.")
            }
            var association: MacConversationRoomAssociation? = isPersisted
                ? try await mainRead(["type": .string("for_conversation"), "conversation_id": .string(conversationID)], owner: conversationID) : nil
            if let current = association?.room.room.participants.first(where: { $0.agentID == agentID && $0.active }), current.agentRevision != expectedRevision {
                guard allowReintroduction, let room = association?.room else { throw MacExpertMentionPreparationError.reintroductionRequired }
                let _: MacAgentRoomSnapshot = try await mainRead(["type": .string("rejoin"), "input": .object([
                    "room_id": .string(room.room.id), "expected_room_revision": .signedInteger(room.revision),
                    "agent_id": .string(agentID), "expected_agent_revision": .signedInteger(expectedRevision),
                    "at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000))])], owner: conversationID)
                association = try await mainRead(["type": .string("for_conversation"), "conversation_id": .string(conversationID)], owner: conversationID)
            }
            recordMainExpertSelection(agent, conversationID: conversationID)
            if mainConversationID == conversationID { conversationAssociation = association }
            return
        }
        let service = service
        let fresh = try await Task.detached { try service.directory().agents }.value
        guard let agent = fresh.first(where: { $0.id == agentID && $0.availability == "active" }),
              agent.revision == expectedRevision else {
            throw MacApplicationPortError.rejected(String(localized: "The mentioned assistant is unavailable. Remove or replace the mention.", table: "Chat"))
        }
        if !isPersisted {
            mentionedExpertRevisions[conversationID, default: [:]][agentID] = expectedRevision
            return
        }
        var association = try await Task.detached { try service.ensureForConversation(conversationID) }.value
        if let previous = association.room.room.participants.first(where: { $0.agentID == agentID && $0.active }),
           previous.agentRevision != agent.revision {
            let catalog = try await Task.detached { try service.roleTemplates() }.value
            guard catalog.features.rejoin else {
                throw MacApplicationPortError.rejected(String(localized: "This Host does not support expert reintroduction."))
            }
            guard allowReintroduction else { throw MacExpertMentionPreparationError.reintroductionRequired }
            let room = association.room
            _ = try await Task.detached { try service.rejoin(agent: agent, room: room) }.value
            association = try await Task.detached { try service.ensureForConversation(conversationID) }.value
        } else if !association.room.room.participants.contains(where: { $0.agentID == agentID && $0.active }) {
            let room = association.room
            _ = try await Task.detached { try service.join(agent: agent, room: room) }.value
            association = try await Task.detached { try service.ensureForConversation(conversationID) }.value
        }
        guard association.room.room.participants.contains(where: { $0.agentID == agentID && $0.active }) else {
            throw MacApplicationPortError.invalidResponse
        }
        mentionedExpertRevisions[conversationID, default: [:]][agentID] = expectedRevision
        if admissionOwner == conversationID, !isAdmitting { conversationAssociation = association }
    }

    func refreshExpertDirectory() async {
        guard !requiresMainSource || mainSourceReady else { return }
        let owner = mainConversationID
        let service = service
        do {
            let fresh: MacAgentDirectory
            if mainCommand != nil { fresh = try await mainRead(["type": .string("directory")], owner: owner) }
            else { fresh = try await Task.detached { try service.directory() }.value }
            guard owner == mainConversationID else { return }
            directory = fresh.agents; directoryError = nil
        } catch { if owner == mainConversationID { directoryError = error.localizedDescription } }
    }

    func refreshDirectory() async {
        let service = service
        do {
            let result = try await Task.detached { (try service.list(), try service.directory()) }.value
            rooms = result.0; directory = result.1.agents; directoryError = nil
        } catch { directoryError = error.localizedDescription }
    }
    func select(_ room: MacAgentRoomSnapshot, resumeExecutions: Bool = true) async {
        selectionEpoch &+= 1
        let epoch = selectionEpoch
        selectedRoom = room
        let service = service
        do {
            let loadDraft = !loadedDrafts.contains(room.room.id)
            let beforeDraft = drafts[room.room.id]?.revision
            let result = try await Task.detached {
                (try service.load(id: room.room.id), try service.submissions(roomID: room.room.id),
                 loadDraft ? try service.loadDraft(roomID: room.room.id) : nil)
            }.value
            guard epoch == selectionEpoch else { return }
            guard let snapshot = result.0, snapshot.room.id == room.room.id,
                  result.1.allSatisfy({ $0.input.roomID == room.room.id }) else {
                throw MacApplicationPortError.rejected("Room response ownership mismatch")
            }
            selectedRoom = snapshot
            for participant in snapshot.room.participants {
                executionService?.observe(contextID: participant.contextConversationID)
            }
            if loadDraft {
                loadedDrafts.insert(room.room.id)
                draftStorageRevisions[room.room.id] = result.2?.revision
                let accepted = Set(result.1.map(\.id))
                pending[room.room.id] = (result.2?.pending ?? []).filter { !accepted.contains($0.id) }.map {
                    var item = $0
                    item.sending = false
                    item.failure = String(localized: "Submission status not confirmed. Retry checks the same request.", table: "Chat")
                    return item
                }
                durablePendingIDs[room.room.id] = Set(result.2?.pending.map(\.id) ?? [])
                if drafts[room.room.id]?.revision == beforeDraft {
                    drafts[room.room.id] = result.2?.draft ?? MacRoomDraft()
                } else { persistDraft(room.room.id) }
            }
            for receipt in result.1 { applyReceipt(receipt) }
            for receipt in result.1 {
                observeExecution(receipt)
                if resumeExecutions { Task {
                    if receipt.state == "cancelled" {
                        do { try await cancelPreparedOwner?(receipt) }
                        catch { errors[receipt.input.roomID] = error.localizedDescription }
                    } else { await startSelected(receipt) }
                } }
            }
            errors[room.room.id] = nil
        } catch {
            guard epoch == selectionEpoch else { return }
            errors[room.room.id] = error.localizedDescription
        }
    }
    func create(_ title: String) async {
        let service = service
        do {
            let room = try await Task.detached { try service.create(title: title) }.value
            rooms.insert(room, at: 0)
            await select(room)
        } catch { directoryError = error.localizedDescription }
    }
    @discardableResult
    func createAgent(name: String, description: String, defaults: MacCanonicalJSONValue) async -> String? {
        let service = service
        let id = UUID().uuidString.lowercased()
        do {
            let updated = try await Task.detached {
                try service.createAgent(name: name, description: description, defaults: defaults,
                    directory: service.directory(), id: id)
            }.value
            directory = updated.agents; directoryError = nil
            return id
        } catch { directoryError = error.localizedDescription; return nil }
    }
    func updateAgent(_ agent: MacPersistentAgent, name: String, description: String, isEnabled: Bool,
                     defaults: MacCanonicalJSONValue? = nil) async {
        let service = service
        do {
            let updated = try await Task.detached {
                try service.updateAgent(agent, name: name, description: description,
                    availability: isEnabled ? "active" : "archived", directory: service.directory(), defaults: defaults)
            }.value
            directory = updated.agents; directoryError = nil
        } catch { directoryError = error.localizedDescription }
    }
    func join(_ agent: MacPersistentAgent) async {
        guard let room = selectedRoom else { return }
        let service = service
        do {
            let result = try await Task.detached { try service.join(agent: agent, room: room) }.value
            if let index = rooms.firstIndex(where: { $0.room.id == result.room.id }) { rooms[index] = result }
            if selectedRoom?.room.id == room.room.id { await select(result) }
        } catch { errors[room.room.id] = error.localizedDescription }
    }
    func send() {
        guard canSend, let room = selectedRoom else { return }
        let captured = draft
        guard !captured.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if let id = captured.participantID,
           !room.room.participants.contains(where: { $0.id == id && $0.active }) {
            errors[room.room.id] = String(localized: "The mentioned assistant is unavailable. Remove or replace the mention.", table: "Chat")
            return
        }
        // Freeze identity before suspension; retries reuse the same receipt key.
        let item = Pending(input: MacRoomSubmissionInput(roomID: room.room.id,
            submissionID: UUID().uuidString.lowercased(), expectedRoomRevision: room.revision,
            text: captured.text, participantID: captured.participantID,
            atMs: Int64(Date().timeIntervalSince1970 * 1000)), draftRevision: captured.revision)
        pending[room.room.id, default: []].append(item)
        deliveryTasks[item.id] = Task {
            defer { deliveryTasks[item.id] = nil }
            await deliver(item)
        }
    }
    func retry(_ item: Pending) {
        guard let index = pending[item.input.roomID]?.firstIndex(where: { $0.id == item.id }),
              pending[item.input.roomID]?[index].sending == false else { return }
        pending[item.input.roomID]?[index].sending = true
        deliveryTasks[item.id] = Task {
            defer { deliveryTasks[item.id] = nil }
            await deliver(item)
        }
    }
    func cancelPending(_ id: String) {
        guard let roomID = selectedRoom?.room.id,
              let receipt = submissions[roomID]?.first(where: { $0.id == id }),
              ["accepted", "selected"].contains(receipt.state) else { return }
        let service = service
        Task {
            do {
                let update = try await Task.detached { try service.cancelPending(receipt) }.value
                guard update.input == receipt.input else { return }
                applyReceipt(update)
                deliveryTasks[id]?.cancel()
                if update.state == "cancelled" { try await cancelPreparedOwner?(update) }
            } catch { errors[roomID] = error.localizedDescription }
        }
    }
    private func deliver(_ item: Pending) async {
        let owner = item.input.roomID
        guard let index = pending[owner]?.firstIndex(where: { $0.id == item.id }) else { return }
        pending[owner]?[index].sending = true
        pending[owner]?[index].failure = nil
        await persistDraft(owner)?.value
        guard durablePendingIDs[owner]?.contains(item.id) == true else {
            if let current = pending[owner]?.firstIndex(where: { $0.id == item.id }) {
                pending[owner]?[current].sending = false
                pending[owner]?[current].failure = errors[owner] ?? String(localized: "Save the draft before sending.", table: "Chat")
            }
            return
        }
        let service = service
        do {
            var receipt = try await Task.detached { try service.submit(item.input) }.value
            guard receipt.input == item.input else { throw MacApplicationPortError.rejected("Room receipt ownership mismatch") }
            if !(submissions[owner] ?? []).contains(where: { $0.id == receipt.id }) {
                submissions[owner, default: []].append(receipt)
            }
            pending[owner]?.removeAll { $0.id == item.id }
            drafts[owner]?.acknowledge(revision: item.draftRevision)
            persistDraft(owner)
            if receipt.state == "accepted", let routeSubmission,
               let room = try await Task.detached(operation: { try service.load(id: owner) }).value {
                do {
                    let routed = try await routeSubmission(receipt, room)
                    guard routed.input == item.input else { throw CocoaError(.coderInvalidValue) }
                    receipt = routed
                    applyReceipt(receipt)
                } catch {
                    errors[owner] = error.localizedDescription
                    let failedReceipt = receipt
                    if let failed = try? await Task.detached(operation: { try service.failRouting(failedReceipt) }).value,
                       failed.input == item.input { applyReceipt(failed) }
                    return
                }
            }
            await startSelected(receipt)
        } catch {
            guard let current = pending[owner]?.firstIndex(where: { $0.id == item.id }) else { return }
            pending[owner]?[current].sending = false
            pending[owner]?[current].failure = error.localizedDescription
        }
    }

    @discardableResult
    private func persistDraft(_ owner: String) -> Task<Void, Never>? {
        guard loadedDrafts.contains(owner) else { return nil }
        if let existing = savingDrafts[owner] { return existing }
        let task = Task {
            defer { savingDrafts[owner] = nil }
            // Coalesce ordinary typing, then serialize writes per owner.
            try? await Task.sleep(for: .milliseconds(200))
            while let captured = drafts[owner] {
                let capturedPending = pending[owner] ?? []
                let revision = draftStorageRevisions[owner]
                let service = service
                do {
                    let stored = try await Task.detached {
                        try service.saveDraft(captured, pending: capturedPending, roomID: owner, expectedRevision: revision)
                    }.value
                    draftStorageRevisions[owner] = stored
                    durablePendingIDs[owner] = Set(capturedPending.map(\.id))
                    if drafts[owner] == captured && (pending[owner] ?? []) == capturedPending { return }
                } catch {
                    errors[owner] = error.localizedDescription
                    return
                }
            }
        }
        savingDrafts[owner] = task
        return task
    }
}
