// SPDX-License-Identifier: Apache-2.0
import Foundation
import Combine

/// Presentation owner for a shared room. Model input remains in each participant's canonical conversation.
@MainActor final class IOSAgentRoomWorkspace: ObservableObject {
    struct Row: Identifiable {
        let participant: IOSRoomParticipant
        let projected: IOSConversationProjectionV1.Message
        let message: ChatMessage
        var id: String { participant.contextConversationID + "/" + projected.messageID }
    }
    @Published private(set) var room: IOSAgentRoomSnapshot?
    @Published private(set) var rows: [Row] = []
    @Published private(set) var hasOlderMessages = false
    private var historyCursors: [String: Int64] = [:]
    private var messages: [String: [String: IOSConversationProjectionV1.Message]] = [:]
    private var messageOrder: [String: [String]] = [:]
    private var journalContent: [String: [String: IOSJournalMessageContent]] = [:]
    private var runs: [String: IOSConversationProjectionV1.RunActivity] = [:]
    var hasActiveRuns: Bool { !runs.isEmpty }
    private var surfaces: [String: IOSConversationProjectionV1.Surface] = [:]
    private var cursors: [String: UInt64] = [:]
    private var journalSequences: [String: UInt64] = [:]
    private var subscriptions: [String: Set<AnyCancellable>] = [:]

    func open(_ snapshot: IOSAgentRoomSnapshot, container: ServiceContainer, observesExecution: Bool = true) throws {
        if let current = room, current.room.id == snapshot.room.id, current.revision > snapshot.revision { return }
        guard let store = container.conversationStore as? ConversationProjectionStore else {
            throw IOSApplicationRuntimeError.rejected("Canonical conversation store is unavailable")
        }
        var loaded: [String: IOSConversationProjectionV1] = [:]
        var loadedCursors: [String: UInt64] = [:]
        var older: [String: Int64] = [:]
        for participant in snapshot.room.participants {
            guard let id = UUID(uuidString: participant.contextConversationID) else {
                throw IOSApplicationRuntimeError.rejected("Invalid participant context identity")
            }
            let window = try store.loadConversationProjectionWindow(
                conversationId: id, beforeOrdinal: nil, limit: 80, contentByteLimit: 512_000)
            loaded[participant.contextConversationID] = window.projection
            loadedCursors[participant.contextConversationID] = window.eventSequence
            if window.hasMore, let cursor = window.beforeOrdinal { older[participant.contextConversationID] = cursor }
        }
        try install(snapshot, projections: loaded, eventSequences: loadedCursors)
        historyCursors = older
        hasOlderMessages = !older.isEmpty
        if observesExecution {
            for participant in snapshot.room.participants { observe(participant, container: container) }
        }
    }

    /// A scoped Core history response already owns its provenance and message
    /// window. Installing it never creates a ChatManager execution owner.
    func installReadOnlyHistory(_ snapshot: IOSAgentRoomSnapshot, window: IOSConversationProjectionWindowV1) throws {
        guard snapshot.room.participants.count == 1,
              let owner = snapshot.room.participants.first?.contextConversationID,
              window.projection.conversationID == owner else { throw IOSApplicationRuntimeError.invalidResponse }
        try install(snapshot, projections: [owner: window.projection], eventSequences: [owner: window.eventSequence])
        historyCursors[owner] = window.hasMore ? window.beforeOrdinal : nil
        hasOlderMessages = !historyCursors.isEmpty
    }

    func appendReadOnlyHistory(_ window: IOSConversationProjectionWindowV1) throws {
        let owner = window.projection.conversationID
        guard room?.room.participants.count == 1,
              room?.room.participants.first?.contextConversationID == owner,
              messages[owner] != nil else { throw IOSApplicationRuntimeError.invalidResponse }
        let existing = Set(messageOrder[owner, default: []])
        messageOrder[owner] = window.projection.messages.map(\.messageID).filter { !existing.contains($0) }
            + messageOrder[owner, default: []]
        for message in window.projection.messages {
            if let current = messages[owner]?[message.messageID], current.revision >= message.revision { continue }
            messages[owner]?[message.messageID] = message
        }
        historyCursors[owner] = window.hasMore ? window.beforeOrdinal : nil
        hasOlderMessages = !historyCursors.isEmpty
        rebuild()
    }

    /// Installs an atomically captured presentation without selecting or starting any execution.
    /// Kept independent of ServiceContainer so ownership and stale-update rules can be tested.
    func install(_ snapshot: IOSAgentRoomSnapshot, projections: [String: IOSConversationProjectionV1],
                 eventSequences: [String: UInt64]) throws {
        let owners = Set(snapshot.room.participants.map(\.contextConversationID))
        guard owners.count == snapshot.room.participants.count,
              owners == Set(projections.keys), owners == Set(eventSequences.keys),
              projections.allSatisfy({ owner, projection in projection.conversationID == owner }) else {
            throw IOSApplicationRuntimeError.rejected("Room projection ownership mismatch")
        }
        if room?.room.id == snapshot.room.id, let current = room, current.revision > snapshot.revision { return }
        subscriptions.removeAll()
        messages.removeAll(); journalContent.removeAll(); runs.removeAll(); surfaces.removeAll(); cursors.removeAll()
        messageOrder.removeAll()
        historyCursors.removeAll(); hasOlderMessages = false
        room = snapshot
        cursors = eventSequences
        journalSequences = eventSequences
        for (owner, projection) in projections {
            messages[owner] = Dictionary(uniqueKeysWithValues: projection.messages.map { ($0.messageID, $0) })
            messageOrder[owner] = projection.messages.map(\.messageID)
            runs[owner] = projection.activeRun
            surfaces[owner] = projection.surface
        }
        rebuild()
    }

    func close() {
        subscriptions.removeAll()
        room = nil; rows = []
        journalSequences.removeAll()
        messages.removeAll(); journalContent.removeAll(); runs.removeAll(); surfaces.removeAll(); cursors.removeAll()
        messageOrder.removeAll()
        historyCursors.removeAll(); hasOlderMessages = false
    }

    func loadOlder(container: ServiceContainer) throws {
        guard let store = container.conversationStore as? ConversationProjectionStore else { return }
        for (owner, cursor) in historyCursors {
            guard let id = UUID(uuidString: owner) else { continue }
            let window = try store.loadConversationProjectionWindow(conversationId: id,
                beforeOrdinal: cursor, limit: 80, contentByteLimit: 512_000)
            guard window.projection.conversationID == owner else {
                throw IOSApplicationRuntimeError.rejected("Room projection ownership mismatch")
            }
            let existing = Set(messageOrder[owner, default: []])
            messageOrder[owner] = window.projection.messages.map(\.messageID).filter { !existing.contains($0) }
                + messageOrder[owner, default: []]
            for message in window.projection.messages {
                if let current = messages[owner]?[message.messageID], current.revision >= message.revision { continue }
                messages[owner]?[message.messageID] = message
            }
            historyCursors[owner] = window.hasMore ? window.beforeOrdinal : nil
        }
        hasOlderMessages = !historyCursors.isEmpty
        rebuild()
    }

    private func observe(_ participant: IOSRoomParticipant, container: ServiceContainer) {
        let owner = participant.contextConversationID
        guard subscriptions[owner] == nil, let id = UUID(uuidString: owner),
              let execution = container.chatManager.ensureExecutionForConversation(id) else { return }
        var tokens = Set<AnyCancellable>()
        execution.$committedJournalPresentation.compactMap { $0 }.sink { [weak self] update in
            guard UUID(uuidString: update.conversationID) == id else { return }
            self?.apply(owner: owner, sequence: update.eventSequence, updates: update.messages,
                run: update.activeRun, surface: update.surface, orderedWindow: true)
        }.store(in: &tokens)
        execution.$committedAssistantProjection.compactMap { $0 }.sink { [weak self] update in
            guard UUID(uuidString: update.conversationID) == id else { return }
            self?.apply(owner: owner, sequence: update.eventSequence, updates: [update.message],
                run: update.activeRun, surface: update.surface)
        }.store(in: &tokens)
        execution.$committedAssistantJournalContent.compactMap { $0 }.sink { [weak self] content in
            self?.applyJournalContent(content, owner: owner)
        }.store(in: &tokens)
        subscriptions[owner] = tokens
    }

    func apply(owner: String, sequence: UInt64, updates: [IOSConversationProjectionV1.Message],
                       run: IOSConversationProjectionV1.RunActivity?, surface: IOSConversationProjectionV1.Surface,
                       orderedWindow: Bool = false) {
        guard messages[owner] != nil else { return }
        if orderedWindow {
            guard sequence >= (journalSequences[owner] ?? 0) else { return }
            journalSequences[owner] = sequence
            let orderedIDs = updates.map(\.messageID)
            let included = Set(orderedIDs)
            let previousOrder = messageOrder[owner, default: []]
            let insertion = previousOrder.firstIndex(where: { included.contains($0) }) ?? previousOrder.count
            let prefix = previousOrder.prefix(insertion).filter { !included.contains($0) }
            let suffix = previousOrder.dropFirst(insertion).filter { !included.contains($0) }
            messageOrder[owner] = prefix + orderedIDs + suffix
        } else {
            guard sequence >= (cursors[owner] ?? 0) else { return }
        }
        for message in updates {
            if !orderedWindow, messages[owner]?[message.messageID] == nil {
                messageOrder[owner, default: []].append(message.messageID)
            }
            if let previous = messages[owner]?[message.messageID], previous.revision > message.revision { continue }
            messages[owner]?[message.messageID] = message
        }
        if sequence >= (cursors[owner] ?? 0) {
            cursors[owner] = sequence
            runs[owner] = run
            surfaces[owner] = surface
        }
        rebuild()
    }

    func applyJournalContent(_ content: IOSJournalMessageContent, owner: String) {
        guard messages[owner] != nil,
              IOSCommittedAssistantProjection.sameConversation(content.cursor.identity.conversation_id, owner),
              let sequence = UInt64(content.cursor.sequence) else { return }
        let id = content.cursor.identity.assistant_message_id
        if let previous = journalContent[owner]?[id] {
            guard previous.cursor.identity == content.cursor.identity,
                  sequence > (UInt64(previous.cursor.sequence) ?? 0) else { return }
        }
        journalContent[owner, default: [:]][id] = content
        rebuild()
    }

    private func rebuild() {
        var streams: [[Row]] = []
        for participant in room?.room.participants ?? [] {
            let owner = participant.contextConversationID
            var stream: [Row] = []
            for messageID in messageOrder[owner, default: []] {
                guard let projected = messages[owner]?[messageID] else { continue }
                guard let message = IOSConversationProjectionPresenter.message(from: projected,
                    activeRun: runs[owner], surface: surfaces[owner]) else { continue }
                var rendered = [message]
                if let content = journalContent[owner]?[projected.messageID] {
                    ChatMessageDraftReducer.applyJournalContent(content, in: &rendered, conversationID: owner)
                }
                stream.append(Row(participant: participant, projected: projected, message: rendered[0]))
            }
            streams.append(stream)
        }
        // Merge stream heads, never sort away canonical order within a Context.
        var offsets = Array(repeating: 0, count: streams.count)
        var result: [Row] = []
        while let next = streams.indices.filter({ offsets[$0] < streams[$0].count }).min(by: {
            let lhs = streams[$0][offsets[$0]], rhs = streams[$1][offsets[$1]]
            return lhs.projected.createdAtMs == rhs.projected.createdAtMs
                ? $0 < $1 : lhs.projected.createdAtMs < rhs.projected.createdAtMs
        }) {
            result.append(streams[next][offsets[next]])
            offsets[next] += 1
        }
        rows = result
    }
}
