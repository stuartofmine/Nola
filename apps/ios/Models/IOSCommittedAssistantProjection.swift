// SPDX-License-Identifier: Apache-2.0
// Read-only, owner-bound facts loaded after Application commits a transition.
import Foundation

/// Presentation-only cache: content progress never invalidates row metadata.
@MainActor
final class IOSRemoteJournalPresentationCache {
    private var admission: IOSRunMessageAdmissionCursor?
    private var generation: UInt64?
    private var scope: IOSLocalJournalPublicationScope?
    private var identity: IOSAssistantJournalIdentity?
    private var retiredIdentities = Set<IOSAssistantJournalIdentity>()

    func loadIfNeeded(admission next: IOSRunMessageAdmissionCursor, generation nextGeneration: UInt64,
                      cursor: IOSAssistantJournalCursor?, force: Bool, isCurrent: () -> Bool,
                      readWindow: (Int64?) throws -> IOSConversationProjectionWindowV1,
                      sleep: (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) async throws -> IOSConversationProjectionWindowV1? {
        guard isCurrent() else { return nil }
        if let generation, nextGeneration < generation { return nil }
        if generation != nextGeneration { retiredIdentities.removeAll() }
        guard !retiredIdentities.contains(next.identity) else {
            throw IOSApplicationRuntimeError.rejected("Remote journal presentation owner was superseded")
        }
        if generation != nextGeneration || identity != next.identity {
            if generation == nextGeneration, let identity { retiredIdentities.insert(identity) }
            generation = nextGeneration
            identity = next.identity
            admission = nil
            scope = IOSLocalJournalPublicationScope(conversationID: next.identity.conversation_id,
                runID: next.identity.run_id)
        }
        if let cursor {
            guard cursor.identity.source_store_epoch == next.identity.source_store_epoch,
                  cursor.identity.conversation_id == next.identity.conversation_id,
                  cursor.identity.run_id == next.identity.run_id else {
                throw IOSApplicationRuntimeError.rejected("Remote journal presentation owner mismatch")
            }
        }
        let ownerNeedsAdmission = try cursor.map { try scope!.needsAdmission($0) } ?? false
        guard force || admission != next || ownerNeedsAdmission else { return nil }
        let window: IOSConversationProjectionWindowV1?
        if let cursor, ownerNeedsAdmission, !force {
            window = try await IOSLocalJournalWindowAdmission.load(cursor: cursor,
                isCurrent: isCurrent, readWindow: readWindow, sleep: sleep)
        } else {
            window = try await IOSLocalJournalWindowAdmission.read(isCurrent: isCurrent,
                operation: { try readWindow(nil) }, sleep: sleep)
        }
        guard isCurrent(), generation == nextGeneration, identity == next.identity, let window else { return nil }
        guard window.projection.conversationID == next.identity.conversation_id else {
            throw IOSApplicationRuntimeError.rejected("Remote journal window belongs to another conversation")
        }
        if let cursor, !force {
            guard window.projection.messages.contains(where: { $0.messageID == cursor.identity.assistant_message_id && $0.role == .assistant }),
                  window.assistantJournalHeads.contains(where: { $0.identity == cursor.identity }) else {
                return nil // Missing metadata remains unadmitted and retryable.
            }
            try scope!.didAdmit(cursor)
        }
        for head in window.assistantJournalHeads where
            head.identity.source_store_epoch == next.identity.source_store_epoch &&
            head.identity.conversation_id == next.identity.conversation_id &&
            head.identity.run_id == next.identity.run_id {
            try scope!.didAdmit(head)
        }
        admission = next
        return window
    }
}

/// A bounded canonical update can contain newly admitted user/assistant rows.
/// Its observation generation fences presentation, never execution ownership.
nonisolated struct IOSCommittedJournalPresentation: Equatable, Sendable {
    let conversationID: String
    let generation: UInt64
    let eventSequence: UInt64
    let messages: [IOSConversationProjectionV1.Message]
    let activeRun: IOSConversationProjectionV1.RunActivity?
    let surface: IOSConversationProjectionV1.Surface
}

nonisolated struct IOSCommittedAssistantOwner: Equatable, Sendable {
    let conversationID: String
    let assistantMessageID: String
    let generation: UInt64

    /// Journal bodies and Message lifecycle metadata have independent clocks.
    /// A matching native control wakeup may refresh metadata while it is still
    /// missing/pending; it never supplies or fabricates the new Message state.
    func needsLocalMetadataRefresh(sessionID: String?, runID: String?, expectedRunID: String,
                                   currentGeneration: UInt64, projection: IOSCommittedAssistantProjection?) -> Bool {
        guard generation == currentGeneration, let sessionID,
              IOSCommittedAssistantProjection.sameConversation(sessionID, conversationID),
              runID == expectedRunID else { return false }
        guard let projection else { return true }
        return projection.belongs(to: self) && projection.message.state == .pending
    }
}

nonisolated struct IOSRecoveryObservationFence: Sendable {
    let conversationID: UUID
    let generation: UInt64

    func accepts(conversationID: UUID?, generation: UInt64, coordinatorIsAvailable: Bool) -> Bool {
        self.conversationID == conversationID && self.generation == generation && coordinatorIsAvailable
    }
}

nonisolated struct IOSCommittedAssistantProjection: Equatable, Sendable {
    let journalCursor: IOSAssistantJournalCursor?
    let conversationID: String
    let generation: UInt64
    let eventSequence: UInt64
    let message: IOSConversationProjectionV1.Message
    let activeRun: IOSConversationProjectionV1.RunActivity?
    let surface: IOSConversationProjectionV1.Surface

    /// Core's projection revision, Message revision and Store event cursor are
    /// different clocks. Only compare the same Store cursor/Message owner here.
    func isNotOlder(than previous: Self) -> Bool {
        Self.sameConversation(conversationID, previous.conversationID)
            && message.messageID == previous.message.messageID
            && eventSequence >= previous.eventSequence
            && message.revision >= previous.message.revision
    }

    func belongs(to conversationID: String) -> Bool {
        Self.sameConversation(self.conversationID, conversationID)
    }

    func belongs(to owner: IOSCommittedAssistantOwner) -> Bool {
        belongs(to: owner.conversationID) && message.messageID == owner.assistantMessageID
            && generation == owner.generation
    }

    static func sameConversation(_ lhs: String, _ rhs: String) -> Bool {
        if let left = UUID(uuidString: lhs), let right = UUID(uuidString: rhs) {
            return left == right
        }
        return lhs == rhs
    }

    init?(window: IOSConversationProjectionWindowV1, owner: IOSCommittedAssistantOwner) {
        guard Self.sameConversation(window.projection.conversationID, owner.conversationID),
              let message = window.projection.messages.first(where: {
                  $0.messageID == owner.assistantMessageID && $0.role == .assistant
              }) else { return nil }
        conversationID = window.projection.conversationID
        generation = owner.generation
        eventSequence = window.eventSequence
        self.message = message
        journalCursor = window.assistantJournalHeads.first { $0.identity.assistant_message_id == owner.assistantMessageID }
        activeRun = window.projection.activeRun
        surface = window.projection.surface
    }

    static func load(
        from application: IOSApplicationRuntime,
        owner: IOSCommittedAssistantOwner
    ) throws -> Self? {
        // A Run can publish several assistant owners. Its root is not always
        // the latest row; locate the exact owner in bounded windows, never by
        // substituting the last message or materializing the entire history.
        var before: Int64?
        while true {
            let window = try application.loadConversationProjectionWindowV1(
                id: owner.conversationID, beforeOrdinal: before, limit: 32,
                contentByteLimit: 512_000
            )
            if let result = Self(window: window, owner: owner) { return result }
            guard window.hasMore, let next = window.beforeOrdinal,
                  before == nil || next < before! else { return nil }
            before = next
        }
    }
}
