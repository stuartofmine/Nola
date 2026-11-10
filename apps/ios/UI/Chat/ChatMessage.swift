// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatMessage logic in the iOS UI layer.
// - Interfaces:
//   - ChatMessage
// - APIs:
//   - none
//
import Foundation

nonisolated enum ChatAgentTraceMeta {
    static let thinkingKey = "agentThinkingText"
    static let phaseKey = "agentTracePhase"

    static func values(thinking: String, phase: String?) -> [String: String] {
        var values: [String: String] = [:]
        let thinking = thinking.trimmingCharacters(in: .whitespacesAndNewlines)
        if !thinking.isEmpty {
            values[thinkingKey] = thinking
        }
        let phase = phase?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !phase.isEmpty {
            values[phaseKey] = phase
        }
        return values
    }

    static func thinking(from meta: [String: String]?) -> String {
        meta?[thinkingKey] ?? ""
    }

    static func phase(from meta: [String: String]?) -> String? {
        let value = meta?[phaseKey]?.trimmingCharacters(in: .whitespacesAndNewlines)
        return value?.isEmpty == false ? value : nil
    }
}

nonisolated enum ChatConversationSurfaceMeta {
    static let liveKey = "nola.surface.live"
    static let placeholderKey = "nola.surface.placeholder"
    static let currentToolCallIDKey = "nola.surface.currentToolCallID"
    static let archivedActivityIDsKey = "nola.surface.archivedActivityIDs"
    static let stateKey = "nola.surface.messageState"
}

/// Immutable counters used by the timeline's auto-follow trigger. Computing
/// these while the message snapshot is assembled avoids walking a long
/// streaming answer again whenever SwiftUI reevaluates the timeline body.
struct ChatTimelineFollowMetrics: Equatable {
    let contentPhrase: Int
    let thinkingPhrase: Int
    let toolEventCount: Int
    let toolContentPhrase: Int

    init(
        content: String,
        thinking: String,
        hostToolEvents: [HostAgentToolEvent],
        currentToolCallID: String?
    ) {
        let liveEvent: HostAgentToolEvent?
        if let currentToolCallID,
           let current = hostToolEvents.first(where: { $0.id == currentToolCallID }) {
            liveEvent = current
        } else {
            liveEvent = hostToolEvents.last(where: { event in
                event.status == .running || event.status == .permissionRequired
            })
        }
        contentPhrase = content.count / 24
        thinkingPhrase = thinking.count / 24
        toolEventCount = liveEvent == nil ? 0 : 1
        toolContentPhrase = liveEvent.map {
            ($0.name.count + $0.display.count + $0.text.count) / 24
        } ?? 0
    }
}

struct ChatMessage: Identifiable {
    let id: UUID
    enum Role { case user, assistant }
    let role: Role
    let content: String
    let includeInContext: Bool
    let thinking: String
    let thinkingFinished: Bool
    let hostToolEvents: [HostAgentToolEvent]
    let traceActivities: [AgentTraceActivity]
    let tracePhase: String?
    let taskProjection: AgentCoreTaskProjection?
    let skillRuntimeProjection: AgentCoreSkillRuntimeProjection?
    let canonicalParts: [IOSConversationProjectionV1.Part]
    let canonicalJournalCursor: IOSAssistantJournalCursor?
    let sourceReferences: [ChatSourceReference]
    let canonicalEventSequence: UInt64?
    let canonicalMessageRevision: UInt64?
    let metadata: [String: String]
    let timelineFollowMetrics: ChatTimelineFollowMetrics

    init(
        id: UUID = UUID(),
        role: Role,
        content: String,
        includeInContext: Bool = true,
        thinking: String = "",
        thinkingFinished: Bool = false,
        hostToolEvents: [HostAgentToolEvent] = [],
        traceActivities: [AgentTraceActivity] = [],
        tracePhase: String? = nil,
        taskProjection: AgentCoreTaskProjection? = nil,
        skillRuntimeProjection: AgentCoreSkillRuntimeProjection? = nil,
        canonicalParts: [IOSConversationProjectionV1.Part] = [],
        canonicalJournalCursor: IOSAssistantJournalCursor? = nil,
        sourceReferences: [ChatSourceReference] = [],
        canonicalEventSequence: UInt64? = nil,
        canonicalMessageRevision: UInt64? = nil,
        metadata: [String: String] = [:]
    ) {
        self.id = id
        self.role = role
        self.content = content
        self.includeInContext = includeInContext
        self.thinking = thinking
        self.thinkingFinished = thinkingFinished
        let restoredSkillProjection = skillRuntimeProjection
            ?? AgentCoreSkillProjectionMetadata.decode(from: metadata)
        let skillHostToolEvents = restoredSkillProjection?.mergingHostToolEvents(hostToolEvents)
            ?? hostToolEvents
        let resolvedHostToolEvents = IOSConversationProjectionPresenter.overlayCanonicalTools(
            on: skillHostToolEvents, parts: canonicalParts
        )
        self.hostToolEvents = resolvedHostToolEvents
        self.traceActivities = traceActivities
        self.tracePhase = tracePhase
        self.taskProjection = taskProjection
            ?? AgentCoreTaskProjectionMeta.decode(from: metadata, assistantMessageId: id)
        self.skillRuntimeProjection = restoredSkillProjection
        self.canonicalParts = canonicalParts
        self.canonicalJournalCursor = canonicalJournalCursor
        self.sourceReferences = sourceReferences
        self.canonicalEventSequence = canonicalEventSequence
        self.canonicalMessageRevision = canonicalMessageRevision
        self.metadata = metadata
        self.timelineFollowMetrics = ChatTimelineFollowMetrics(
            content: content,
            thinking: thinking,
            hostToolEvents: resolvedHostToolEvents,
            currentToolCallID: metadata[ChatConversationSurfaceMeta.currentToolCallIDKey]
        )
    }
}

extension ChatMessage {
    var withholdsLatestCanonicalText: Bool {
        for part in canonicalParts.reversed() {
            guard case .text(let text) = part else { continue }
            return !text.isDisclosed
        }
        return false
    }

    var disclosedCanonicalText: String {
        canonicalParts.compactMap { part -> String? in
            guard case .text(let text) = part, text.isDisclosed else { return nil }
            return text.markdown
        }.joined()
    }

    var canonicalTools: [IOSConversationProjectionV1.ToolPart] {
        canonicalParts.compactMap { part in
            guard case .tool(let value) = part else { return nil }
            return value
        }
    }

    var canonicalReasoningTitles: [String] {
        canonicalParts.compactMap { part in
            guard case .reasoning(let value) = part else { return nil }
            return value.title
        }
    }

    var canonicalAttachments: [IOSConversationProjectionV1.AttachmentPart] {
        canonicalParts.compactMap { part in
            guard case .attachment(let value) = part else { return nil }
            return value
        }
    }

    var isCanonicalLive: Bool {
        metadata[ChatConversationSurfaceMeta.liveKey] == "true"
    }

    /// Live chat communicates the one operation that currently needs
    /// attention. `hostToolEvents` remains the complete persisted audit trail.
    var liveHostToolEvents: [HostAgentToolEvent] {
        if let currentID = metadata[ChatConversationSurfaceMeta.currentToolCallIDKey],
           let current = hostToolEvents.first(where: { $0.id == currentID }) {
            return [current]
        }
        guard let current = hostToolEvents.last(where: { event in
            event.status == .running || event.status == .permissionRequired
        }) else {
            return []
        }
        return [current]
    }

    /// Keep failed receipts visible while a later model/tool round is running.
    /// These are history, never another source of live permission actions.
    var failedHistoricalToolEvents: [HostAgentToolEvent] {
        let liveIDs = Set(liveHostToolEvents.map(\.id))
        return hostToolEvents.filter { $0.status == .failed && !liveIDs.contains($0.id) }
    }
}

struct ChatAssistantRuntimeState: Equatable {
    let isActive: Bool
    let isRetrieving: Bool
    let hasContent: Bool
    let hasThinking: Bool
    let hasTools: Bool
    let hasPhase: Bool

    init(message: ChatMessage, isActive: Bool, isRetrieving: Bool) {
        self.isActive = isActive
        self.isRetrieving = isActive && isRetrieving
        self.hasContent = !message.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        self.hasThinking = !message.thinking.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        self.hasTools = isActive && !message.liveHostToolEvents.isEmpty
        self.hasPhase = !(message.tracePhase ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .isEmpty
    }

    var showsWaiting: Bool {
        isActive && !isRetrieving && !hasContent && !hasThinking && !hasTools && !hasPhase
    }

    var showsThinking: Bool {
        isRetrieving || hasThinking
    }

    var showsPhase: Bool {
        isActive && hasPhase && !hasThinking && !hasTools && !hasContent
    }
}

enum ChatTracePhasePresentation {
    static func displayName(for phase: String?) -> String {
        let normalized = (phase ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
        let key: String
        let fallback: String
        if normalized == "compacting" {
            key = "chat.assistant.phase.compacting"
            fallback = "Organizing context"
        } else if normalized.contains("permission") || normalized.contains("confirm") || normalized.contains("waiting") {
            key = "chat.assistant.phase.waiting"
            fallback = "Waiting for confirmation"
        } else if normalized.contains("search") || normalized.contains("retriev") || normalized.contains("browse") {
            key = "chat.assistant.phase.searching"
            fallback = "Searching sources"
        } else if normalized.contains("tool") || normalized.contains("execut") || normalized.contains("command") {
            key = "chat.assistant.phase.tools"
            fallback = "Using tools"
        } else if normalized.contains("validat") || normalized.contains("verif") || normalized.contains("review") {
            key = "chat.assistant.phase.verifying"
            fallback = "Verifying the result"
        } else if normalized.contains("compos") || normalized.contains("generat") || normalized.contains("respond") {
            key = "chat.assistant.phase.composing"
            fallback = "Composing the response"
        } else if normalized.contains("plan") || normalized.contains("reason") || normalized.contains("think") {
            key = "chat.assistant.phase.planning"
            fallback = "Planning the next step"
        } else {
            key = "chat.assistant.processing"
            fallback = "Nola is processing"
        }
        return NSLocalizedString(key, value: fallback, comment: "User-facing assistant run phase")
    }
}

enum ChatMessageDraftReducer {
    /// Attach complete Core task handles only to their exact journal Run owner.
    /// This remains active after model streaming has ended and on cold reload.
    @MainActor static func applyTaskProjection(
        _ projection: AgentCoreTaskProjection, in messages: inout [ChatMessage], conversationID: String
    ) {
        for index in messages.indices where messages[index].role == .assistant {
            let current = messages[index]
            guard let identity = current.canonicalJournalCursor?.identity,
                  IOSCommittedAssistantProjection.sameConversation(identity.conversation_id, conversationID),
                  current.metadata["nola.canonicalMessageID"] == identity.assistant_message_id,
                  projection.revision >= (current.taskProjection?.revision ?? 0) else { continue }
            // The caller supplies the Core projection validated for this conversation.
            // Core session IDs are not Conversation IDs; the journal Run is
            // the exact message owner within that already-scoped projection.
            let tasks = projection.orderedTasks.filter {
                $0.ownerTurnId == identity.run_id && $0.isValid
            }
            guard !tasks.isEmpty else { continue }
            let scoped = AgentCoreTaskProjection(revision: projection.revision, order: tasks.map(\.id),
                tasks: Dictionary(uniqueKeysWithValues: tasks.map { ($0.id, $0) }),
                guidance: projection.guidance?.filter { key, _ in tasks.contains { $0.id == key } })
            guard scoped != current.taskProjection else { continue }
            messages[index] = ChatMessage(id: current.id, role: current.role, content: current.content,
                includeInContext: current.includeInContext, thinking: current.thinking,
                thinkingFinished: current.thinkingFinished, hostToolEvents: current.hostToolEvents,
                traceActivities: current.traceActivities, tracePhase: current.tracePhase,
                taskProjection: scoped, skillRuntimeProjection: current.skillRuntimeProjection,
                canonicalParts: current.canonicalParts, canonicalJournalCursor: current.canonicalJournalCursor,
                sourceReferences: current.sourceReferences, canonicalEventSequence: current.canonicalEventSequence,
                canonicalMessageRevision: current.canonicalMessageRevision, metadata: current.metadata)
        }
    }

    /// Journal sequence orders content only; it must never advance the Store
    /// metadata clock or allow a streaming draft to resurrect retired content.
    @MainActor @discardableResult
    static func applyJournalContent(
        _ content: IOSJournalMessageContent, in messages: inout [ChatMessage], conversationID: String
    ) -> Bool {
        let cursor = content.cursor
        guard IOSCommittedAssistantProjection.sameConversation(cursor.identity.conversation_id, conversationID),
              let sequence = UInt64(cursor.sequence),
              let index = messages.firstIndex(where: {
                  $0.role == .assistant && $0.metadata["nola.canonicalMessageID"] == cursor.identity.assistant_message_id
              }) else { return false }
        let current = messages[index]
        if let previous = current.canonicalJournalCursor {
            guard previous.identity == cursor.identity, let previousSequence = UInt64(previous.sequence),
                  sequence > previousSequence else { return false }
        }
        let reasoningFinished = !content.reasoning.isEmpty && content.parts.allSatisfy {
            if case .reasoning(let part) = $0 {
                return part.state == .completed || part.state == .sealed || part.state == .cancelled || part.state == .failed
            }
            return true
        }
        messages[index] = ChatMessage(
            id: current.id, role: current.role, content: content.text,
            includeInContext: current.includeInContext, thinking: content.reasoning,
            thinkingFinished: reasoningFinished,
            hostToolEvents: content.parts.compactMap(IOSConversationProjectionPresenter.toolEvent),
            traceActivities: current.traceActivities, tracePhase: current.tracePhase,
            taskProjection: current.taskProjection, skillRuntimeProjection: current.skillRuntimeProjection,
            canonicalParts: content.parts, canonicalJournalCursor: cursor,
            sourceReferences: IOSConversationProjectionPresenter.sourceReferences(from: content.parts),
            canonicalEventSequence: current.canonicalEventSequence,
            canonicalMessageRevision: current.canonicalMessageRevision, metadata: current.metadata)
        return true
    }

    /// Only durable Application facts may introduce successor user/assistant
    /// bubbles. Never reassign the streaming submission's root identity.
    @MainActor static func applyJournalPresentation(
        _ update: IOSCommittedJournalPresentation, in messages: inout [ChatMessage], conversationID: String
    ) {
        guard IOSCommittedAssistantProjection.sameConversation(update.conversationID, conversationID) else { return }
        for projected in update.messages {
            guard let rendered = IOSConversationProjectionPresenter.message(
                from: projected, activeRun: update.activeRun, surface: update.surface) else { continue }
            let index = messages.firstIndex { $0.id == rendered.id }
            let old = index.map { messages[$0] }
            guard old?.canonicalEventSequence.map({ update.eventSequence >= $0 }) ?? true,
                  old?.canonicalMessageRevision.map({ projected.revision >= $0 }) ?? true else { continue }
            var metadata = old?.metadata ?? [:]
            metadata.merge(rendered.metadata) { _, canonical in canonical }
            let journal = old?.canonicalJournalCursor != nil ? old : nil
            let next = ChatMessage(id: rendered.id, role: rendered.role, content: journal?.content ?? rendered.content,
                includeInContext: old?.includeInContext ?? true, thinking: journal?.thinking ?? rendered.thinking,
                thinkingFinished: journal?.thinkingFinished ?? rendered.thinkingFinished,
                hostToolEvents: journal?.hostToolEvents ?? rendered.hostToolEvents,
                traceActivities: old?.traceActivities ?? [], tracePhase: rendered.tracePhase,
                taskProjection: old?.taskProjection, skillRuntimeProjection: old?.skillRuntimeProjection,
                canonicalParts: journal?.canonicalParts ?? rendered.canonicalParts,
                canonicalJournalCursor: journal?.canonicalJournalCursor,
                sourceReferences: journal?.sourceReferences ?? rendered.sourceReferences,
                canonicalEventSequence: update.eventSequence, canonicalMessageRevision: projected.revision,
                metadata: metadata)
            if let index { messages[index] = next } else { messages.append(next) }
        }
    }

    /// Merge only a committed, exact-owner query. Rich metadata must not rewind
    /// a newer streaming body; review withholding is an explicit exception.
    @MainActor @discardableResult
    static func applyCommitted(
        _ snapshot: IOSCommittedAssistantProjection,
        in messages: inout [ChatMessage],
        conversationID: String
    ) -> Bool {
        guard snapshot.belongs(to: conversationID),
              let rendered = IOSConversationProjectionPresenter.message(
                  from: snapshot.message, activeRun: snapshot.activeRun, surface: snapshot.surface
              ),
              let index = messages.firstIndex(where: { $0.id == rendered.id && $0.role == .assistant })
        else { return false }
        let current = messages[index]
        guard current.metadata["nola.canonicalMessageID"].map({ $0 == snapshot.message.messageID }) ?? true,
              current.canonicalEventSequence.map({ snapshot.eventSequence >= $0 }) ?? true,
              current.canonicalMessageRevision.map({ snapshot.message.revision >= $0 }) ?? true
        else { return false }
        var metadata = current.metadata
        metadata.merge(rendered.metadata) { _, committed in committed }
        messages[index] = ChatMessage(
            id: current.id, role: .assistant,
            content: rendered.withholdsLatestCanonicalText || current.content.isEmpty
                ? rendered.content : current.content,
            includeInContext: current.includeInContext,
            thinking: current.thinking.isEmpty ? rendered.thinking : current.thinking,
            thinkingFinished: current.thinkingFinished || rendered.thinkingFinished,
            hostToolEvents: current.hostToolEvents, traceActivities: current.traceActivities,
            tracePhase: current.tracePhase, taskProjection: current.taskProjection,
            skillRuntimeProjection: current.skillRuntimeProjection,
            canonicalParts: current.canonicalJournalCursor == nil ? rendered.canonicalParts : current.canonicalParts,
            canonicalJournalCursor: current.canonicalJournalCursor,
            sourceReferences: current.canonicalJournalCursor == nil ? rendered.sourceReferences : current.sourceReferences,
            canonicalEventSequence: snapshot.eventSequence,
            canonicalMessageRevision: snapshot.message.revision, metadata: metadata
        )
        return true
    }

    static func upsertAssistant(
        in messages: inout [ChatMessage],
        assistantID: UUID,
        afterUserID userID: UUID,
        content: String? = nil,
        thinking: String? = nil,
        thinkingFinished: Bool? = nil,
        hostToolEvents: [HostAgentToolEvent]? = nil,
        traceActivities: [AgentTraceActivity]? = nil,
        tracePhase: String? = nil,
        taskProjection: AgentCoreTaskProjection? = nil,
        skillRuntimeProjection: AgentCoreSkillRuntimeProjection? = nil,
        metadata: [String: String]? = nil
    ) {
        if let index = messages.firstIndex(where: { $0.id == assistantID }) {
            let current = messages[index]
            var resolvedMetadata = metadata ?? current.metadata
            if current.canonicalEventSequence != nil {
                for key in ["nola.canonicalMessageID", ChatConversationSurfaceMeta.liveKey,
                    ChatConversationSurfaceMeta.placeholderKey, ChatConversationSurfaceMeta.stateKey,
                    ChatConversationSurfaceMeta.archivedActivityIDsKey, ChatConversationSurfaceMeta.currentToolCallIDKey] {
                    if let value = current.metadata[key] { resolvedMetadata[key] = value }
                }
            }
            messages[index] = ChatMessage(
                id: current.id,
                role: .assistant,
                content: current.canonicalJournalCursor != nil ? current.content :
                    (current.withholdsLatestCanonicalText ? current.disclosedCanonicalText : content ?? current.content),
                includeInContext: current.includeInContext,
                thinking: current.canonicalJournalCursor != nil ? current.thinking : thinking ?? current.thinking,
                thinkingFinished: current.canonicalJournalCursor != nil ? current.thinkingFinished : thinkingFinished ?? current.thinkingFinished,
                hostToolEvents: current.canonicalJournalCursor != nil ? current.hostToolEvents : hostToolEvents ?? current.hostToolEvents,
                traceActivities: traceActivities ?? current.traceActivities,
                tracePhase: tracePhase ?? current.tracePhase,
                taskProjection: taskProjection ?? current.taskProjection,
                skillRuntimeProjection: skillRuntimeProjection ?? current.skillRuntimeProjection,
                canonicalParts: current.canonicalParts,
                canonicalJournalCursor: current.canonicalJournalCursor,
                sourceReferences: current.sourceReferences,
                canonicalEventSequence: current.canonicalEventSequence,
                canonicalMessageRevision: current.canonicalMessageRevision,
                metadata: resolvedMetadata
            )
            return
        }

        let draft = ChatMessage(
            id: assistantID,
            role: .assistant,
            content: content ?? "",
            includeInContext: true,
            thinking: thinking ?? "",
            thinkingFinished: thinkingFinished ?? false,
            hostToolEvents: hostToolEvents ?? [],
            traceActivities: traceActivities ?? [],
            tracePhase: tracePhase,
            taskProjection: taskProjection,
            skillRuntimeProjection: skillRuntimeProjection,
            metadata: metadata ?? [:]
        )
        if let userIndex = messages.firstIndex(where: { $0.id == userID }) {
            messages.insert(draft, at: min(messages.index(after: userIndex), messages.endIndex))
        } else {
            messages.append(draft)
        }
    }
}
