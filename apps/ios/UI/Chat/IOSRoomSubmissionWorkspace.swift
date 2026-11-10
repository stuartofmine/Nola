// SPDX-License-Identifier: Apache-2.0
// Room presentation owns drafts and receipt feedback; Core owns dispatch.
import Foundation
import Combine

nonisolated struct IOSRoomDraft: Codable, Equatable, Sendable {
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

@MainActor final class IOSRoomSubmissionWorkspace: ObservableObject {
    nonisolated struct Pending: Codable, Equatable, Identifiable, Sendable {
        let input: IOSRoomSubmissionInput
        let draftRevision: UInt64
        var failure: String?
        var sending = true
        var id: String { input.submissionID }
    }
    nonisolated struct TimelineEntry: Identifiable {
        let input: IOSRoomSubmissionInput
        let state: String
        let selectedParticipantID: String?
        let pending: Pending?
        var id: String { input.submissionID }
    }

    nonisolated static func timeline(receipts: [IOSRoomSubmission], pending: [Pending]) -> [TimelineEntry] {
        let acceptedIDs = Set(receipts.map(\.id))
        let received = receipts.map { TimelineEntry(input: $0.input, state: $0.state,
            selectedParticipantID: $0.selectedParticipantID, pending: nil) }
        let local = pending.filter { !acceptedIDs.contains($0.id) }.map {
            TimelineEntry(input: $0.input, state: $0.sending ? "sending" : "send_failed",
                selectedParticipantID: $0.input.participantID, pending: $0)
        }
        return (received + local).sorted {
            $0.input.atMs == $1.input.atMs ? $0.id < $1.id : $0.input.atMs < $1.input.atMs
        }
    }
    let service: IOSAgentRoomService
    @Published var rooms: [IOSAgentRoomSnapshot] = []
    @Published var directory: [IOSPersistentAgent] = []
    @Published private(set) var selectedRoom: IOSAgentRoomSnapshot?
    @Published private(set) var drafts: [String: IOSRoomDraft] = [:]
    @Published private(set) var submissions: [String: [IOSRoomSubmission]] = [:]
    @Published private(set) var pending: [String: [Pending]] = [:]
    @Published private(set) var errors: [String: String] = [:]
    @Published var directoryError: String?
    @Published private(set) var conversationAssociation: IOSConversationRoomAssociation?
    @Published private(set) var admissionError: String?
    @Published private(set) var isAdmitting = false
    private var pendingAdmission: IOSAgentRoomService.UserMessageAdmission?
    private var pendingDraftTicket: ConversationComposerDraftStore.TextSubmissionTicket?
    private var pendingExpertPlan: AppleAdmitExpertConversationMessage?
    @Published private(set) var recoverableExpertManifests: [String: AppleExpertMessageManifest<AgentCoreJSONValue>] = [:]
    @Published private(set) var resumingExpertMessages = Set<String>()
    @Published private(set) var expertResumeError: String?
    @Published private(set) var pendingExpertIDs: [String] = []
    private var restoreEpoch: UInt64 = 0
    private var visibleExpertMessageIDs = Set<String>()
    private var readingExpertRecovery = Set<String>()
    private var expertRecoveryNeedsRefresh = Set<String>()
    private var expertRecoveryRefreshTasks: [String: Task<Void, Never>] = [:]
    var pendingAdmissionText: String? { pendingAdmission?.text }
    private var admissionStorageRevision: Int64?
    private var admissionOwner: String?

    /// Reading a conversation restores presentation and retry state, never starts an expert.
    func restoreConversation(_ conversationID: String) async {
        guard !isAdmitting else { return }
        restoreEpoch &+= 1
        expertRecoveryRefreshTasks.values.forEach { $0.cancel() }
        expertRecoveryRefreshTasks.removeAll()
        visibleExpertMessageIDs.removeAll()
        readingExpertRecovery.removeAll()
        expertRecoveryNeedsRefresh.removeAll()
        let epoch = restoreEpoch
        conversationAssociation = nil
        recoverableExpertManifests = [:]
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
            // A cold-restored business intent does not own a newly mounted
            // composer revision. Never infer its draft identity from text.
            pendingDraftTicket = nil
            pendingExpertPlan = restored.1?.0.plan
            pendingExpertIDs = restored.1?.0.expertIDs ?? []
            admissionError = pendingAdmission == nil ? nil : "An expert message is pending. Retry sends the original message."
        } catch { admissionError = error.localizedDescription }
    }


    /// Freeze the entire user message before any suspension; retry never substitutes edited text.
    struct ExpertAdmissionAcknowledgement {
        let text: String
        let draftTicket: ConversationComposerDraftStore.TextSubmissionTicket?
    }
    func sendExperts(conversationID: String, text: String, agentIDs: Set<String>,
                     draftTicket: ConversationComposerDraftStore.TextSubmissionTicket?) async -> ExpertAdmissionAcknowledgement? {
        guard !isAdmitting, !agentIDs.isEmpty else { return nil }
        if admissionOwner != conversationID {
            await restoreConversation(conversationID)
            guard admissionOwner == conversationID, !isAdmitting else { return nil }
        }
        if let pendingAdmission {
            guard pendingAdmission.conversationID == conversationID else {
                admissionError = "Retry the pending expert message in its original conversation."
                return nil
            }
        } else {
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            pendingAdmission = .init(conversationID: conversationID,
                messageID: UUID().uuidString.lowercased(), text: text,
                createdAtMs: Int64(Date().timeIntervalSince1970 * 1000))
            pendingExpertIDs = agentIDs.sorted()
            pendingDraftTicket = draftTicket
        }
        guard let input = pendingAdmission else { return nil }
        isAdmitting = true
        defer { isAdmitting = false }
        do {
            let service = service
            let frozen = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID,
                input: input, expertIDs: pendingExpertIDs, plan: pendingExpertPlan)
            let previousRevision = admissionStorageRevision
            admissionStorageRevision = try await Task.detached {
                try service.saveExpertAdmission(frozen, expectedRevision: previousRevision)
            }.value
            var association = try await Task.detached { try service.ensureForConversation(input.conversationID) }.value
            let available = try await Task.detached { try service.directory().agents }.value
            for id in pendingExpertIDs {
                if association.room.room.participants.contains(where: { $0.agentID == id && $0.active }) { continue }
                guard let agent = available.first(where: { $0.id == id && $0.availability == "active" }) else {
                    throw IOSApplicationRuntimeError.rejected("Selected expert is unavailable")
                }
                let room = association.room
                _ = try await Task.detached { try service.join(agent: agent, room: room) }.value
                association = try await Task.detached { try service.ensureForConversation(input.conversationID) }.value
            }
            await select(association.room, resumeExecutions: false)
            guard selectedRoom?.room.id == association.roomID,
                  selectedRoom?.revision == association.room.revision else {
                throw IOSApplicationRuntimeError.rejected("Expert conversation could not be loaded")
            }
            if pendingExpertPlan == nil {
                let selected = Set(pendingExpertIDs)
                let participants = association.room.room.participants.filter { selected.contains($0.agentID) && $0.active }
                guard Set(participants.map(\.agentID)) == selected,
                      let conversation = try service.canonicalConversation(input.conversationID) else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                pendingExpertPlan = .init(conversation_id: input.conversationID,
                    expected_revision: conversation.revision, room_id: association.roomID,
                    expected_room_revision: association.room.revision, submission_id: input.messageID,
                    user_message_id: input.messageID, text: input.text,
                    selected_participants: participants.map { .init(participant_id: $0.id,
                        agent_id: $0.agentID, agent_revision: $0.agentRevision,
                        context_conversation_id: $0.contextConversationID) }, created_at_ms: input.createdAtMs)
                let frozen = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID,
                    input: input, expertIDs: pendingExpertIDs, plan: pendingExpertPlan)
                let revision = admissionStorageRevision
                admissionStorageRevision = try await Task.detached {
                    try service.saveExpertAdmission(frozen, expectedRevision: revision)
                }.value
            }
            guard let plan = pendingExpertPlan else { throw IOSApplicationRuntimeError.invalidResponse }
            let manifest = try await Task.detached { try service.admitExperts(plan) }.value
            conversationAssociation = association
            // Core admitted the entire selected set atomically. Read its receipts;
            // never independently construct or submit a subset after this point.
            for entry in manifest.entries {
                let child = entry.child_submission_id
                guard let receipt = try await Task.detached(operation: {
                    try service.submission(roomID: plan.room_id, submissionID: child)
                }).value, receipt.id == child,
                      receipt.input.parentMessageID == plan.user_message_id,
                      receipt.input.conversationID == plan.conversation_id,
                      receipt.contextConversationID == entry.participant.context_conversation_id else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                applyReceipt(receipt)
                if deliveryTasks[receipt.id] == nil {
                    deliveryTasks[receipt.id] = Task {
                        defer { finishExpertDelivery(receipt) }
                        await startSelected(receipt)
                    }
                }
            }
            let cleared = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: conversationID, input: nil, expertIDs: [])
            let clearRevision = admissionStorageRevision
            admissionStorageRevision = try await Task.detached {
                try service.saveExpertAdmission(cleared, expectedRevision: clearRevision)
            }.value
            let acknowledgement = ExpertAdmissionAcknowledgement(text: input.text, draftTicket: pendingDraftTicket)
            pendingAdmission = nil
            pendingDraftTicket = nil
            pendingExpertPlan = nil
            pendingExpertIDs = []
            admissionError = nil
            return acknowledgement
        } catch { admissionError = error.localizedDescription; return nil }
    }

    private var selectionEpoch: UInt64 = 0
    private var draftStorageRevisions: [String: Int64] = [:]
    private var loadedDrafts = Set<String>()
    private var savingDrafts: [String: Task<Void, Never>] = [:]
    private var durablePendingIDs: [String: Set<String>] = [:]

    private let executeOwner: (@MainActor (IOSRoomSubmission, IOSAgentRoomService.ExecutionIntent) async throws -> Void)?
    var prepareOwner: (@MainActor (IOSRoomSubmission) async throws -> IOSAgentRoomService.ExecutionIntent)?
    var resumePreparedExpertOwner: (@MainActor (String) async throws -> Bool)?
    var recoverOwner: (@MainActor (IOSRoomSubmission, IOSAgentRoomService.ExecutionIntent) async throws -> Void)?
    var cancelPreparedOwner: (@MainActor (IOSRoomSubmission) async throws -> Void)?
    private var startingExecutions = Set<String>()
    private var deliveryTasks: [String: Task<Void, Never>] = [:]
    var routeSubmission: (@MainActor (IOSRoomSubmission, IOSAgentRoomSnapshot) async throws -> IOSRoomSubmission)?
    private var executionObservers: [String: Task<Void, Never>] = [:]
    init(service: IOSAgentRoomService,
         executeOwner: (@MainActor (IOSRoomSubmission, IOSAgentRoomService.ExecutionIntent) async throws -> Void)? = nil) {
        self.service = service
        self.executeOwner = executeOwner
    }

    func startSelected(_ receipt: IOSRoomSubmission) async {
        guard ["selected", "executing"].contains(receipt.state), let executeOwner,
              let prepareOwner, startingExecutions.insert(receipt.id).inserted else { return }
        defer {
            startingExecutions.remove(receipt.id)
            if let parent = receipt.input.parentMessageID,
               receipt.input.conversationID == conversationAssociation?.conversationID,
               receipt.input.roomID == conversationAssociation?.roomID { scheduleExpertRecoveryRefresh(parent) }
        }
        let service = service
        do {
            let persisted = try await Task.detached { try service.executionIntent(receipt) }.value
            let intent: IOSAgentRoomService.ExecutionIntent
            if let persisted { intent = persisted } else { intent = try await prepareOwner(receipt) }
            var prepared = receipt
            if persisted == nil {
                prepared = try await Task.detached { try service.prepareExecution(receipt, intent: intent) }.value
            }
            let toClaim = prepared
            let claim = try await Task.detached { try service.claimExecution(toClaim) }.value
            guard claim.submission.input == receipt.input else {
                throw IOSApplicationRuntimeError.rejected("Room claim ownership mismatch")
            }
            applyReceipt(claim.submission)
            guard claim.submission.state == "executing" else {
                if claim.submission.state == "cancelled" { try await cancelPreparedOwner?(claim.submission) }
                return
            }
            guard let context = claim.submission.contextConversationID,
                  claim.submission.executionID == intent.execution_id else {
                throw IOSApplicationRuntimeError.rejected("Room execution intent identity mismatch")
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
        } catch {
            errors[receipt.input.roomID] = error.localizedDescription
            if let latest = try? service.submission(roomID: receipt.input.roomID, submissionID: receipt.id),
               latest.input == receipt.input, latest.state == "cancelled" {
                applyReceipt(latest)
                do { try await cancelPreparedOwner?(latest) }
                catch { errors[receipt.input.roomID] = error.localizedDescription }
            }
        }
    }

    /// A late observation may update only its original owner and may never
    /// roll a newer cancellation/terminal receipt back to an earlier revision.
    func applyReceipt(_ update: IOSRoomSubmission) {
        let roomID = update.input.roomID
        if let index = submissions[roomID]?.firstIndex(where: { $0.id == update.id }) {
            guard let current = submissions[roomID]?[index], current.input == update.input,
                  update.revision > current.revision else { return }
            submissions[roomID]?[index] = update
        } else { submissions[roomID, default: []].append(update) }
        if let parent = update.input.parentMessageID,
           update.input.conversationID == conversationAssociation?.conversationID,
           update.input.roomID == conversationAssociation?.roomID {
            scheduleExpertRecoveryRefresh(parent)
        }
    }

    private func finishExpertDelivery(_ receipt: IOSRoomSubmission) {
        deliveryTasks[receipt.id] = nil
        guard let parent = receipt.input.parentMessageID,
              receipt.input.conversationID == conversationAssociation?.conversationID,
              receipt.input.roomID == conversationAssociation?.roomID else { return }
        scheduleExpertRecoveryRefresh(parent)
    }

    /// Coalesce receipt/admission-boundary observations, never layout frames.
    /// The full Core manifest is reread: one admitted peer cannot hide another.
    private func scheduleExpertRecoveryRefresh(_ messageID: String) {
        guard visibleExpertMessageIDs.contains(messageID), let association = conversationAssociation else { return }
        if readingExpertRecovery.contains(messageID) || expertRecoveryRefreshTasks[messageID] != nil {
            expertRecoveryNeedsRefresh.insert(messageID)
            return
        }
        let epoch = restoreEpoch
        expertRecoveryRefreshTasks[messageID] = Task { [weak self] in
            guard let self else { return }
            defer { if self.restoreEpoch == epoch { self.expertRecoveryRefreshTasks[messageID] = nil } }
            repeat {
                guard !Task.isCancelled, self.restoreEpoch == epoch,
                      self.conversationAssociation?.conversationID == association.conversationID,
                      self.conversationAssociation?.roomID == association.roomID else { return }
                self.expertRecoveryNeedsRefresh.remove(messageID)
                await self.refreshVisibleExpertReceipts(conversationID: association.conversationID, messageIDs: [messageID])
            } while self.expertRecoveryNeedsRefresh.contains(messageID)
        }
    }

    /// Disclosed parent history uses exact Core manifests/receipts, never a
    /// first-page Room list interpreted as the complete conversation history.
    func refreshVisibleExpertReceipts(conversationID: String, messageIDs: [String]) async {
        guard let association = conversationAssociation, association.conversationID == conversationID else { return }
        visibleExpertMessageIDs.formUnion(messageIDs)
        let epoch = restoreEpoch, service = service
        func current() -> Bool {
            !Task.isCancelled && epoch == restoreEpoch && conversationAssociation?.conversationID == conversationID
                && conversationAssociation?.roomID == association.roomID
        }
        do {
            for messageID in Set(messageIDs).sorted() {
                guard current() else { return }
                guard readingExpertRecovery.insert(messageID).inserted else { continue }
                defer {
                    if epoch == restoreEpoch {
                        readingExpertRecovery.remove(messageID)
                        if expertRecoveryNeedsRefresh.contains(messageID) { scheduleExpertRecoveryRefresh(messageID) }
                    }
                }
                guard let manifest = try await Task.detached(operation: {
                    try service.expertManifest(conversationID: conversationID, messageID: messageID)
                }).value else { continue }
                guard manifest.input.conversation_id == conversationID, manifest.input.room_id == association.roomID,
                      manifest.input.user_message_id == messageID else { throw IOSApplicationRuntimeError.invalidResponse }
                var needsExplicitResume = false
                for entry in manifest.entries {
                    guard current() else { return }
                    let child = entry.child_submission_id
                    let receipt = try await Task.detached { try service.submission(roomID: association.roomID, submissionID: child) }.value
                    guard current() else { return }
                    guard let receipt, receipt.id == child, receipt.input.parentMessageID == messageID,
                          receipt.input.conversationID == conversationID,
                          receipt.contextConversationID == entry.participant.context_conversation_id,
                          receipt.selectedParticipantID == entry.participant.participant_id else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                    applyReceipt(receipt)
                    observeExecution(receipt)
                    if ["selected", "executing"].contains(receipt.state) {
                        let context = entry.participant.context_conversation_id
                        let exists = try await Task.detached { try service.executionExists(contextID: context, executionID: child) }.value
                        guard current() else { return }
                        if !exists, !startingExecutions.contains(child), deliveryTasks[child] == nil {
                            needsExplicitResume = true
                        }
                    }
                }
                if needsExplicitResume { recoverableExpertManifests[messageID] = manifest }
                else { recoverableExpertManifests[messageID] = nil }
            }
        } catch { if current() { errors[association.roomID] = error.localizedDescription } }
    }

    /// Explicit user action reuses the entire original plan. Read/selection never
    /// starts work; already admitted child Runs are observed, never submitted.
    func resumeCompleteExpertMessage(_ messageID: String) async {
        guard !isAdmitting, !resumingExpertMessages.contains(messageID),
              let original = recoverableExpertManifests[messageID],
              let association = conversationAssociation,
              original.input.conversation_id == association.conversationID,
              original.input.room_id == association.roomID else { return }
        let epoch = restoreEpoch, service = service
        func current() -> Bool {
            !Task.isCancelled && epoch == restoreEpoch && conversationAssociation?.conversationID == original.input.conversation_id
                && conversationAssociation?.roomID == original.input.room_id
        }
        resumingExpertMessages.insert(messageID)
        expertResumeError = nil
        defer { resumingExpertMessages.remove(messageID) }
        do {
            if let resumePreparedExpertOwner,
               try await resumePreparedExpertOwner(original.input.conversation_id) {
                guard current() else { return }
                await refreshVisibleExpertReceipts(conversationID: original.input.conversation_id, messageIDs: [messageID])
                return
            }
            let manifest = try await Task.detached { try service.admitExperts(original.input) }.value
            guard manifest.input == original.input, manifest.entries.map(\.child_submission_id) == original.entries.map(\.child_submission_id) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            var receipts: [IOSRoomSubmission] = []
            for entry in manifest.entries {
                guard current() else { return }
                let child = entry.child_submission_id
                guard let receipt = try await Task.detached(operation: {
                    try service.submission(roomID: original.input.room_id, submissionID: child)
                }).value, receipt.id == child, receipt.input.parentMessageID == messageID,
                      receipt.input.conversationID == original.input.conversation_id,
                      receipt.contextConversationID == entry.participant.context_conversation_id,
                      receipt.selectedParticipantID == entry.participant.participant_id else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                receipts.append(receipt)
            }
            guard current() else { return }
            // Validate the complete collection before dispatching any missing peer.
            for receipt in receipts {
                applyReceipt(receipt)
                guard let context = receipt.contextConversationID else { throw IOSApplicationRuntimeError.invalidResponse }
                let exists = try await Task.detached { try service.executionExists(contextID: context, executionID: receipt.id) }.value
                guard current() else { return }
                if exists {
                    observeExecution(receipt)
                    continue
                }
                if ["selected", "executing"].contains(receipt.state), deliveryTasks[receipt.id] == nil {
                    deliveryTasks[receipt.id] = Task {
                        defer { finishExpertDelivery(receipt) }
                        await startSelected(receipt)
                        await refreshVisibleExpertReceipts(conversationID: original.input.conversation_id, messageIDs: [messageID])
                    }
                }
            }
            guard current() else { return }
            recoverableExpertManifests[messageID] = nil
        } catch { if current() { expertResumeError = error.localizedDescription } }
    }

    private func observeExecution(_ receipt: IOSRoomSubmission) {
        guard receipt.state == "executing", executionObservers[receipt.id] == nil else { return }
        executionObservers[receipt.id] = Task { [weak self, service] in
            defer { self?.executionObservers[receipt.id] = nil }
            while !Task.isCancelled {
                // Core checks the exact canonical seal; missing/unfinished runs
                // remain unconfirmed, never converted to a client success.
                if let update = try? await Task.detached(operation: { try service.observeExecution(receipt) }).value {
                    guard let self, update.input == receipt.input else { return }
                    self.applyReceipt(update)
                    if let parent = update.input.parentMessageID,
                       self.recoverableExpertManifests[parent] != nil,
                       update.input.conversationID == self.conversationAssociation?.conversationID,
                       update.input.roomID == self.conversationAssociation?.roomID {
                        // A claim may remain at the same Room revision across
                        // actual Begin. Recheck only a displayed recovery card,
                        // using this existing Core observer, until reconciled.
                        self.scheduleExpertRecoveryRefresh(parent)
                    }
                    if update.state != "executing" { return }
                }
                try? await Task.sleep(for: .seconds(2))
                guard self != nil else { return }
            }
        }
    }
    var draft: IOSRoomDraft { selectedRoom.flatMap { drafts[$0.room.id] } ?? IOSRoomDraft() }
    var canSend: Bool {
        guard let room = selectedRoom,
              loadedDrafts.contains(room.room.id),
              room.room.participants.contains(where: \.active),
              !draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return !(pending[room.room.id] ?? []).contains { $0.draftRevision == draft.revision }
    }
    var candidates: [IOSRoomParticipant] {
        guard let query = draft.mentionQuery else { return [] }
        return (selectedRoom?.room.participants ?? []).filter {
            $0.active && (query.isEmpty || $0.displayName.localizedCaseInsensitiveContains(query))
        }
    }
    func edit(_ text: String) {
        guard let id = selectedRoom?.room.id else { return }
        var value = drafts[id] ?? IOSRoomDraft()
        value.text = text; value.revision &+= 1
        drafts[id] = value
        persistDraft(id)
    }
    func mention(_ participant: IOSRoomParticipant) {
        guard let room = selectedRoom, room.room.participants.contains(participant), participant.active else { return }
        var value = draft; value.mention(participant.id); drafts[room.room.id] = value
        persistDraft(room.room.id)
    }
    func removeMention() {
        guard let id = selectedRoom?.room.id else { return }
        var value = draft; value.participantID = nil; value.revision &+= 1; drafts[id] = value
        persistDraft(id)
    }
    func refreshDirectory() async {
        let service = service
        do {
            let result = try await Task.detached { (try service.list(), try service.directory()) }.value
            rooms = result.0; directory = result.1.agents; directoryError = nil
        } catch { directoryError = error.localizedDescription }
    }
    func select(_ room: IOSAgentRoomSnapshot, resumeExecutions: Bool = true) async {
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
                throw IOSApplicationRuntimeError.rejected("Room response ownership mismatch")
            }
            selectedRoom = snapshot
            if loadDraft {
                loadedDrafts.insert(room.room.id)
                draftStorageRevisions[room.room.id] = result.2?.revision
                let accepted = Set(result.1.map(\.id))
                pending[room.room.id] = (result.2?.pending ?? []).filter { !accepted.contains($0.id) }.map {
                    var item = $0
                    item.sending = false
                    item.failure = String(localized: "Submission status not confirmed. Retry checks the same request.")
                    return item
                }
                durablePendingIDs[room.room.id] = Set(result.2?.pending.map(\.id) ?? [])
                if drafts[room.room.id]?.revision == beforeDraft {
                    drafts[room.room.id] = result.2?.draft ?? IOSRoomDraft()
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
    func join(_ agent: IOSPersistentAgent) async {
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
            errors[room.room.id] = String(localized: "The mentioned assistant is unavailable. Remove or replace the mention.")
            return
        }
        // Freeze identity before suspension; retries reuse the same receipt key.
        let item = Pending(input: IOSRoomSubmissionInput(roomID: room.room.id,
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
                pending[owner]?[current].failure = errors[owner] ?? String(localized: "Save the draft before sending.")
            }
            return
        }
        let service = service
        do {
            var receipt = try await Task.detached { try service.submit(item.input) }.value
            guard receipt.input == item.input else { throw IOSApplicationRuntimeError.rejected("Room receipt ownership mismatch") }
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
