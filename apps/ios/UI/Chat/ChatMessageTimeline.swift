// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatMessageTimeline logic in the iOS UI layer.
// - Interfaces:
//   - ChatMessageTimeline
// - APIs:
//   - none
//
import SwiftUI
import Combine

private struct ChatUserScrollObserver: ViewModifier {
    let began: () -> Void
    let ended: () -> Void

    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content.onScrollPhaseChange { _, phase in
                switch phase {
                case .tracking, .interacting: began()
                case .idle: ended()
                default: break // Keep the gesture active through deceleration.
                }
            }
        } else {
            // Older systems cannot report deceleration. A drag pauses follow;
            // the explicit latest button remains the unambiguous resume action.
            content.simultaneousGesture(DragGesture(minimumDistance: 4)
                .onChanged { _ in began() })
        }
    }
}

private struct ChatTimelineFollowSignal: Equatable {
    let contentPhrase: Int
    let thinkingPhrase: Int
    let toolEventCount: Int
    let toolContentPhrase: Int
}

private nonisolated struct ChatTimelineMessageIdentity: Hashable, Sendable {
    let conversationID: UUID?
    let messageID: UUID
}

enum ChatTimelineFollowPolicy {
    static func trailingPasses(reduceMotion: Bool) -> Int {
        // Follow committed geometry at display cadence, including a short
        // trailing layout window. Ink animation never delays accepted text.
        // Reduced Motion only needs the two post-layout frames.
        reduceMotion ? DesignTokens.reducedMotionFollowPasses : DesignTokens.followPasses
    }

    static func intervalNanoseconds(reduceMotion: Bool) -> UInt64 {
        reduceMotion
            ? DesignTokens.reducedMotionFollowIntervalNanoseconds
            : DesignTokens.followIntervalNanoseconds
    }

    static func acceptsAsyncResult(
        startedConversationID: UUID?,
        currentConversationID: UUID?,
        isCancelled: Bool
    ) -> Bool {
        !isCancelled && startedConversationID == currentConversationID
    }
}

enum ChatTimelineMessageMotionPolicy {
    static func appendedIDs(
        previousConversationID: UUID?,
        conversationID: UUID?,
        previous: [UUID]?,
        current: [UUID]
    ) -> Set<UUID> {
        guard previousConversationID == conversationID,
              let previous, current.count > previous.count,
              current.prefix(previous.count).elementsEqual(previous) else { return [] }
        return Set(current.dropFirst(previous.count))
    }
}

enum ChatCompactionSourceNavigator {
    static func belongsToSnapshot(
        _ window: IOSConversationProjectionWindowV1,
        revision: UInt64,
        eventSequence: UInt64
    ) -> Bool {
        window.projection.revision == revision && window.eventSequence == eventSequence
    }

    static func accepts(
        _ request: IOSCompactionSourceRequest,
        conversationID: UUID?,
        messages: [ChatMessage]
    ) -> Bool {
        guard request.conversationID == conversationID,
              !request.sourceItemIDs.isEmpty,
              !request.sourceFingerprint.isEmpty else { return false }
        return messages.contains { message in
            message.canonicalParts.contains { part in
                guard case let .compaction(
                    partID, _, _, sourceItemIDs, sourceFingerprint, _, _
                ) = part else { return false }
                return partID == request.compactionPartID
                    && sourceItemIDs == request.sourceItemIDs
                    && sourceFingerprint == request.sourceFingerprint
            }
        }
    }

    static func targetMessageID(
        sourceItemIDs: [String],
        messages: [ChatMessage]
    ) -> UUID? {
        let sourceIDs = Set(sourceItemIDs)
        return messages.first { message in
            if let canonicalMessageID = message.metadata["nola.canonicalMessageID"],
               sourceIDs.contains(canonicalMessageID) {
                return true
            }
            return message.canonicalParts.contains { sourceIDs.contains($0.partID) }
        }?.id
    }

    static func mergingOlder(_ older: [ChatMessage], into loaded: [ChatMessage]) -> [ChatMessage] {
        mergingFetchedOlderPages([older], into: loaded)
    }

    /// Lookup requests fetch pages from the current window toward older history.
    /// Commit them oldest-page first so locating a source several pages back does
    /// not invert canonical transcript order. Merge with the latest live suffix
    /// at commit time so streaming updates received during lookup are retained.
    static func mergingFetchedOlderPages(
        _ pagesNewestToOldest: [[ChatMessage]],
        into loaded: [ChatMessage]
    ) -> [ChatMessage] {
        var seen = Set<String>()
        return (pagesNewestToOldest.reversed().flatMap { $0 } + loaded).filter { message in
            seen.insert(canonicalIdentity(of: message)).inserted
        }
    }

    private static func canonicalIdentity(of message: ChatMessage) -> String {
        message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString
    }

    static func canContinue(
        previousCursor: Int64,
        nextCursor: Int64?,
        hasMore: Bool
    ) -> Bool {
        guard hasMore, let nextCursor else { return false }
        return nextCursor < previousCursor
    }
}

enum ChatCompactionSourceLookupResult: Equatable {
    case found(UUID)
    case exhausted
    case failed
}

/// Geometry preferences arrive during layout. Publish only the last viewport
/// result after a display interval, rather than feeding scrollTo back into the
/// same layout frame. A main-queue hop alone can still run within that frame.
@MainActor
final class ChatTimelineGeometryUpdates: ObservableObject {
    private var pending: (Bool, (Bool) -> Void)?
    private var scheduled = false
    private var generation: UInt64 = 0

    func submit(atLatest: Bool, apply: @escaping (Bool) -> Void) {
        pending = (atLatest, apply)
        guard !scheduled else { return }
        scheduled = true
        let token = generation
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(16)) { [weak self] in
            guard let self, self.generation == token else { return }
            self.scheduled = false
            let update = self.pending
            self.pending = nil
            if let update { update.1(update.0) }
        }
    }

    func cancel() {
        generation &+= 1
        scheduled = false
        pending = nil
    }
}

private struct ChatTimelineGeometryPreference: PreferenceKey {
    static var defaultValue: [String: CGFloat] { [:] }
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

private struct ChatTimelineContentGeometry: ViewModifier {
    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content
        } else {
            content.background(GeometryReader { geometry in
                Color.clear.preference(key: ChatTimelineGeometryPreference.self,
                    value: ["bottom": geometry.frame(in: .named("chat-timeline-viewport")).maxY])
            })
        }
    }
}

private struct ChatTimelineViewportGeometry: ViewModifier {
    let changed: (Bool) -> Void
    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content.onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentSize.height - geometry.visibleRect.maxY <= 36
            } action: { _, atLatest in
                changed(atLatest)
            }
        } else {
            content.background(GeometryReader { geometry in
                Color.clear.preference(key: ChatTimelineGeometryPreference.self,
                    value: ["viewportHeight": geometry.size.height])
            })
            .onPreferenceChange(ChatTimelineGeometryPreference.self) { geometry in
                guard let bottom = geometry["bottom"], let height = geometry["viewportHeight"], height > 0 else { return }
                changed(bottom <= height + 36)
            }
        }
    }
}

// Navigation-only state: never owns transcript or execution state.
@MainActor
final class ChatReadingPositions {
    final class Viewport {
        var offset: CGFloat = 0
        var visibleID: UUID?
    }
    struct Bookmark: Equatable {
        var firstLoadedID: UUID?
        var visibleID: UUID?
        var offset: CGFloat
        var followsLatest: Bool
    }
    var conversations: [UUID: Bookmark] = [:]
}

private struct ChatReadingPositionModifier: ViewModifier {
    let conversationID: UUID?
    let messageIDs: [UUID]
    let restore: ChatReadingPositions.Bookmark?
    let viewport: ChatReadingPositions.Viewport
    @State private var visibleID: ChatTimelineMessageIdentity?

    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content.modifier(ChatReadingPosition18(conversationID: conversationID,
                messageIDs: messageIDs, restore: restore, viewport: viewport))
        } else {
            content.scrollPosition(id: $visibleID, anchor: .top)
                .onChange(of: visibleID) { _, id in viewport.visibleID = id?.messageID }
                .onChange(of: restore, initial: true) { _, bookmark in
                    guard let id = bookmark?.visibleID else { return }
                    visibleID = .init(conversationID: conversationID, messageID: id)
                }
        }
    }
}

@available(iOS 18.0, *)
private struct ChatReadingPosition18: ViewModifier {
    let conversationID: UUID?
    let messageIDs: [UUID]
    let restore: ChatReadingPositions.Bookmark?
    let viewport: ChatReadingPositions.Viewport
    @State private var position = ScrollPosition(idType: ChatTimelineMessageIdentity.self)

    func body(content: Content) -> some View {
        content.scrollPosition($position)
            .onScrollGeometryChange(for: CGFloat.self) { geometry in
                max(0, geometry.contentOffset.y + geometry.contentInsets.top)
            } action: { _, offset in viewport.offset = offset }
            .onScrollTargetVisibilityChange(idType: ChatTimelineMessageIdentity.self, threshold: 0.01) { ids in
                let visible = Set(ids.map(\.messageID))
                viewport.visibleID = messageIDs.first { visible.contains($0) }
            }
            .onChange(of: restore, initial: true) { _, bookmark in
                guard let bookmark else { return }
                if bookmark.firstLoadedID == messageIDs.first {
                    position.scrollTo(y: bookmark.offset)
                } else if let id = bookmark.visibleID {
                    position.scrollTo(id: ChatTimelineMessageIdentity(conversationID: conversationID, messageID: id), anchor: .top)
                }
            }
    }
}

struct ChatMessageTimeline: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let bottomAnchorID = "chat-message-timeline-bottom-anchor"
    @StateObject private var geometryUpdates = ChatTimelineGeometryUpdates()
    @State private var followsLatest = true
    @State private var readingRestore: ChatReadingPositions.Bookmark?
    @State private var readingViewport = ChatReadingPositions.Viewport()
    @State private var restoredReadingPosition = false
    @State private var learningMessage: ChatMessage?
    @State private var followTask: Task<Void, Never>?
    @State private var followRequested = false
    @State private var followGeneration: UInt64 = 0
    @State private var userScrolling = false
    @State private var isAtLatest = true
    @State private var sourceLookupMessage: String?
    @State private var sourceLookupInProgress = false
    @State private var sourceLookupTask: Task<Void, Never>?
    @State private var sourceLookupID: UUID?
    @State private var knownMessageIDs: [UUID]?
    @State private var knownMessageConversationID: UUID?

    let conversationID: UUID?
    let messages: [ChatMessage]
    let activeAssistantMessageID: UUID?
    let retrievalInProgress: Bool
    let hasModels: Bool
    let selectedModel: ModelInfo?
    let hybridDeviceToolsEnabled: Bool
    let hostConnectionPhase: ChatHostConnectionPhase
    let pendingHostPermissionRequest: HostAgentPermissionRequest?
    let onLoadCompactionSource: (IOSCompactionSourceRequest) async -> ChatCompactionSourceLookupResult
    let onSuggestion: (String) -> Void
    let onPrimaryAction: () -> Void
    let onHostPermissionDecision: (String, HostAgentPermissionDecision) -> Void
    var readingPositions: ChatReadingPositions? = nil
    var ownerIsRunning: Bool = false
    var onEditAndResend: ((AppleMessageEditSubmission) async throws -> Void)? = nil

    private var followSignal: ChatTimelineFollowSignal {
        let metrics = messages.last?.timelineFollowMetrics
        return ChatTimelineFollowSignal(
            // The renderer presents semantic phrases rather than provider tokens.
            // Quantize follow updates to the same rhythm so invisible raw deltas do
            // not continuously restart the scroll animation.
            contentPhrase: metrics?.contentPhrase ?? 0,
            thinkingPhrase: metrics?.thinkingPhrase ?? 0,
            toolEventCount: metrics?.toolEventCount ?? 0,
            toolContentPhrase: metrics?.toolContentPhrase ?? 0
        )
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                timelineContent.modifier(ChatTimelineContentGeometry())
            }
            // The chat uses its own header rather than a system navigation bar.
            // Clip to the explicit viewport so bounced/scrolled text cannot paint
            // over that header or the status bar's safe area.
            .modifier(ChatReadingPositionModifier(conversationID: conversationID,
                messageIDs: messages.map(\.id), restore: readingRestore, viewport: readingViewport))
            .clipped()
            .accessibilityIdentifier("chat.timeline.viewport")
            .scrollDismissesKeyboard(.immediately)
            .coordinateSpace(name: "chat-timeline-viewport")
            .onChange(of: messages.map(\.id), initial: true) { _, ids in
                knownMessageIDs = ids
                knownMessageConversationID = conversationID
                guard !ids.isEmpty, !restoredReadingPosition else { return }
                restoredReadingPosition = true
                if let conversationID, let bookmark = readingPositions?.conversations[conversationID] {
                    followsLatest = bookmark.followsLatest
                    if !bookmark.followsLatest {
                        cancelFollow()
                        readingRestore = bookmark
                    }
                }
            }
            .modifier(ChatTimelineViewportGeometry { atLatest in
                geometryUpdates.submit(atLatest: atLatest) { latest in
                    if isAtLatest != latest { isAtLatest = latest }
                    if followsLatest && !userScrolling && !latest {
                        scheduleSmoothFollow(with: proxy)
                    }
                }
            })
            .modifier(ChatUserScrollObserver(began: {
                    userScrolling = true
                    followsLatest = false
                    cancelFollow()
                }, ended: {
                    if userScrolling {
                        userScrolling = false
                        followsLatest = isAtLatest
                        if followsLatest { scheduleSmoothFollow(with: proxy) }
                    }
                }))
            .onChange(of: messages.count) { _, _ in
                guard followsLatest else { return }
                scheduleSmoothFollow(with: proxy)
            }
            .onChange(of: followSignal) { _, _ in
                scheduleSmoothFollow(with: proxy)
            }
            .onChange(of: activeAssistantMessageID) { _, _ in
                scheduleSmoothFollow(with: proxy)
            }
            .onChange(of: pendingHostPermissionRequest?.id) { _, _ in
                guard followsLatest else { return }
                scheduleSmoothFollow(with: proxy)
            }
            .onReceive(NotificationCenter.default.publisher(for: .chatCompactionSourceRequested)) {
                notification in
                guard let request = notification.object as? IOSCompactionSourceRequest,
                      ChatCompactionSourceNavigator.accepts(
                        request,
                        conversationID: conversationID,
                        messages: messages
                      ),
                      !sourceLookupInProgress else { return }
                let sourceItemIDs = request.sourceItemIDs
                if let messageID = ChatCompactionSourceNavigator.targetMessageID(
                        sourceItemIDs: sourceItemIDs,
                        messages: messages
                      ) {
                    scrollToCompactionSource(messageID, with: proxy)
                    return
                }
                sourceLookupInProgress = true
                cancelFollow()
                followsLatest = false
                userScrolling = false
                let navigationGeneration = followGeneration
                let startedConversationID = conversationID
                sourceLookupTask?.cancel()
                let lookupID = UUID()
                sourceLookupID = lookupID
                sourceLookupTask = Task { @MainActor in
                    defer {
                        if sourceLookupID == lookupID {
                            sourceLookupInProgress = false
                            sourceLookupTask = nil
                            sourceLookupID = nil
                        }
                    }
                    let result = await onLoadCompactionSource(request)
                    guard sourceLookupID == lookupID, ChatTimelineFollowPolicy.acceptsAsyncResult(
                        startedConversationID: startedConversationID,
                        currentConversationID: conversationID,
                        isCancelled: Task.isCancelled
                    ) else { return }
                    guard followGeneration == navigationGeneration else { return }
                    switch result {
                    case .found(let messageID):
                        // Wait for the prepended page to enter the LazyVStack.
                        await Task.yield()
                        guard !Task.isCancelled, followGeneration == navigationGeneration else { return }
                        scrollToCompactionSource(messageID, with: proxy)
                    case .exhausted:
                        sourceLookupMessage = NSLocalizedString(
                            "chat.compaction.source_unavailable",
                            value: "The original context is no longer available in this conversation.",
                            comment: "Shown after all history pages were searched for compacted source content"
                        )
                    case .failed:
                        sourceLookupMessage = NSLocalizedString(
                            "chat.compaction.source_load_failed",
                            value: "Original context could not be loaded.",
                            comment: "Shown when loading compacted source history fails"
                        )
                    }
                }
            }
            .onChange(of: conversationID) { _, _ in
                // Do not treat the first hydrated history for a newly selected
                // conversation as a live append from the previous transcript.
                knownMessageIDs = nil
                knownMessageConversationID = conversationID
                geometryUpdates.cancel()
                sourceLookupTask?.cancel()
                sourceLookupTask = nil
                sourceLookupID = nil
                sourceLookupInProgress = false
                sourceLookupMessage = nil
                cancelFollow()
                userScrolling = false
                restoredReadingPosition = false
                followsLatest = conversationID.flatMap { readingPositions?.conversations[$0]?.followsLatest } ?? true
                scheduleSmoothFollow(with: proxy)
            }
            .overlay(alignment: .bottomTrailing) {
                latestMessageButton(with: proxy)
            }
            .onDisappear {
                if let conversationID, restoredReadingPosition {
                    readingPositions?.conversations[conversationID] = .init(
                        firstLoadedID: messages.first?.id, visibleID: readingViewport.visibleID,
                        offset: readingViewport.offset, followsLatest: followsLatest)
                }
                geometryUpdates.cancel()
                sourceLookupTask?.cancel()
                sourceLookupTask = nil
                sourceLookupID = nil
                sourceLookupInProgress = false
                cancelFollow()
            }
        }
        .sheet(item: $learningMessage) { message in
            ChatLearningFeedbackSheet(message: message) { statement, appliesToFuture in
                saveLearningFeedback(statement, appliesToFuture: appliesToFuture, message: message)
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
        .alert(
            NSLocalizedString("chat.compaction.source_title", value: "Original context", comment: "Compaction source dialog title"),
            isPresented: Binding(
                get: { sourceLookupMessage != nil },
                set: { if !$0 { sourceLookupMessage = nil } }
            )
        ) {
            Button(L10n.ok, role: .cancel) { sourceLookupMessage = nil }
        } message: {
            Text(sourceLookupMessage ?? "")
        }
    }

    private func scrollToCompactionSource(_ messageID: UUID, with proxy: ScrollViewProxy) {
        cancelFollow()
        userScrolling = false
        followsLatest = false
        withAnimation(reduceMotion ? nil : DesignTokens.responsiveAnimation) {
            proxy.scrollTo(messageID, anchor: .center)
        }
    }

    @ViewBuilder
    private var timelineContent: some View {
        let appendedMessageIDs = ChatTimelineMessageMotionPolicy.appendedIDs(
            previousConversationID: knownMessageConversationID,
            conversationID: conversationID,
            previous: knownMessageIDs,
            current: messages.map(\.id)
        )
        let enteringIDs = activeAssistantMessageID == nil ? Set<UUID>() : appendedMessageIDs
        if messages.isEmpty {
            ChatEmptyStateCard(
                hasModels: hasModels,
                selectedModel: selectedModel,
                hybridDeviceToolsEnabled: hybridDeviceToolsEnabled,
                hostConnectionPhase: hostConnectionPhase,
                onSuggestion: onSuggestion,
                onPrimaryAction: onPrimaryAction
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.top, 40)
            .padding(.horizontal, 16)
            .transition(.opacity.combined(with: .move(edge: .bottom)))
        } else {
            LazyVStack(alignment: .leading, spacing: 20) {
                ForEach(messages) { message in
                    let isLatestAssistant = (messages.last?.id == message.id) && message.role == .assistant
                    let isActiveAssistant = message.role == .assistant
                        && (activeAssistantMessageID == message.id || message.isCanonicalLive)
                    let isRetrieving = isActiveAssistant && retrievalInProgress
                    VStack(alignment: .leading, spacing: 2) {
                        ChatBubble(
                            conversationID: conversationID,
                            message: message,
                            isLatestAssistant: isLatestAssistant,
                            isStreaming: isActiveAssistant,
                            isRetrieving: isRetrieving,
                            pendingHostPermissionRequest: isActiveAssistant ? pendingHostPermissionRequest : nil,
                            onHostPermissionDecision: onHostPermissionDecision,
                            ownerIsRunning: ownerIsRunning,
                            onEditAndResend: onEditAndResend
                        )


                    }
                    .id(ChatTimelineMessageIdentity(conversationID: conversationID, messageID: message.id))
                    .transition(enteringIDs.contains(message.id) && !reduceMotion
                        ? .opacity.combined(with: .offset(y: 8))
                        : .identity)
                    .transaction { transaction in
                        if enteringIDs.contains(message.id) {
                            transaction.animation = reduceMotion ? nil : .easeOut(duration: 0.19)
                        }
                    }
                }

                Color.clear.frame(height: 8)
                    .id(bottomAnchorID)
            }
            .scrollTargetLayout()
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.horizontal, 16)
            .padding(.top, 16)
        }
    }

    @ViewBuilder
    private func latestMessageButton(with proxy: ScrollViewProxy) -> some View {
        if (!followsLatest || !isAtLatest), !messages.isEmpty {
            Button {
                cancelFollow()
                userScrolling = false
                followsLatest = true
                isAtLatest = true
                scrollToLatestMessage(with: proxy, duration: 0.24)
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 10)
            .padding(.bottom, 6)
            .accessibilityLabel(NSLocalizedString("chat.scroll.latest", value: "Jump to latest message", comment: "Jump to latest chat message"))
            .accessibilityIdentifier("chat.scroll.latest")
            .transition(.opacity.combined(with: .move(edge: .bottom)))
        }
    }

    private func saveLearningFeedback(
        _ statement: String,
        appliesToFuture: Bool,
        message: ChatMessage
    ) -> Bool {
        let eventID = UUID().uuidString
        let event = IntelligenceEvent(
            id: eventID,
            source: "user_feedback",
            kind: "response_correction",
            subject: message.id.uuidString,
            summary: statement,
            importance: 0.9,
            confidence: 1.0,
            tags: ["user_confirmed", appliesToFuture ? "future_preference" : "answer_feedback"],
            dedupeKey: appliesToFuture ? "user-preference-\(statement.lowercased().hashValue)" : nil
        )
        let candidates = appliesToFuture ? [
            IntelligenceMemoryCandidate(
                semanticKey: "user-preference-\(statement.lowercased().hashValue)",
                kind: .semantic,
                statement: statement,
                confidence: 1.0,
                sourceEventIDs: [eventID],
                tags: ["user_confirmed", "response_preference"]
            )
        ] : []
        let outcome = IntelligenceTaskOutcome(
            taskType: "conversation_response",
            route: "chat",
            model: selectedModel?.name ?? "unknown",
            success: true,
            userCorrected: true
        )
        let response = PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(
                events: [event],
                memoryCandidates: candidates,
                outcomes: [outcome]
            )
        )
        return ChatLearningFeedbackDraft.didPersist(response)
    }

    private func scrollToLatestMessage(with proxy: ScrollViewProxy, duration: TimeInterval?) {
        guard !messages.isEmpty else { return }
        guard duration != nil else {
            proxy.scrollTo(bottomAnchorID, anchor: .bottom)
            return
        }
        withAnimation(reduceMotion ? nil : DesignTokens.responsiveAnimation) {
            proxy.scrollTo(bottomAnchorID, anchor: .bottom)
        }
    }

    private func scheduleSmoothFollow(with proxy: ScrollViewProxy) {
        guard followsLatest, !userScrolling else { return }
        followRequested = true
        guard followTask == nil else { return }
        let generation = followGeneration
        followTask = Task { @MainActor in
            defer { if followGeneration == generation { followTask = nil } }
            // Markdown reveal and expanded tool details keep changing the row's
            // measured height after the underlying message value changes. Keep a
            // short trailing follow window so the bottom anchor tracks those
            // layout commits instead of stopping after the first text delta.
            var trailingPasses = 0
            while followRequested || trailingPasses > 0 {
                if followRequested {
                    followRequested = false
                    // The Markdown renderer continues committing measured
                    // blocks after the canonical message changes. Track that
                    // visual reveal long enough for a terminal multi-block
                    // answer instead of stopping while its tail is offscreen.
                    trailingPasses = ChatTimelineFollowPolicy.trailingPasses(
                        reduceMotion: reduceMotion
                    )
                }
                try? await Task.sleep(
                    nanoseconds: ChatTimelineFollowPolicy.intervalNanoseconds(
                        reduceMotion: reduceMotion
                    )
                )
                guard !Task.isCancelled, followGeneration == generation, followsLatest, !userScrolling else { return }
                if !isAtLatest {
                    var transaction = Transaction(animation: nil)
                    transaction.disablesAnimations = true
                    withTransaction(transaction) {
                        proxy.scrollTo(bottomAnchorID, anchor: .bottom)
                    }
                }
                trailingPasses -= 1
            }
        }
    }

    private func cancelFollow() {
        followGeneration &+= 1
        followRequested = false
        followTask?.cancel()
        followTask = nil
    }
}

enum ChatLearningFocus: String, CaseIterable, Identifiable {
    case concise
    case detailed
    case tone
    case context
    case correction

    var id: String { rawValue }

    var title: String {
        switch self {
        case .concise: return NSLocalizedString("chat.learning.focus.concise", value: fallbackTitle, comment: "Response correction focus")
        case .detailed: return NSLocalizedString("chat.learning.focus.detailed", value: fallbackTitle, comment: "Response correction focus")
        case .tone: return NSLocalizedString("chat.learning.focus.tone", value: fallbackTitle, comment: "Response correction focus")
        case .context: return NSLocalizedString("chat.learning.focus.context", value: fallbackTitle, comment: "Response correction focus")
        case .correction: return NSLocalizedString("chat.learning.focus.correction", value: fallbackTitle, comment: "Response correction focus")
        }
    }

    var statement: String {
        switch self {
        case .concise: return NSLocalizedString("chat.learning.statement.concise", value: fallbackStatement, comment: "Default learned preference")
        case .detailed: return NSLocalizedString("chat.learning.statement.detailed", value: fallbackStatement, comment: "Default learned preference")
        case .tone: return NSLocalizedString("chat.learning.statement.tone", value: fallbackStatement, comment: "Default learned preference")
        case .context: return NSLocalizedString("chat.learning.statement.context", value: fallbackStatement, comment: "Default learned preference")
        case .correction: return NSLocalizedString("chat.learning.statement.correction", value: fallbackStatement, comment: "Default learned preference")
        }
    }

    private var fallbackTitle: String {
        switch self {
        case .concise: return "More concise"
        case .detailed: return "More depth"
        case .tone: return "Adjust tone"
        case .context: return "Use more context"
        case .correction: return "Correct a fact"
        }
    }

    private var fallbackStatement: String {
        switch self {
        case .concise: return "Lead with the conclusion and keep future answers concise."
        case .detailed: return "Provide deeper analysis and the necessary supporting detail."
        case .tone: return "Adapt the tone of future answers to my communication preference."
        case .context: return "Use the available conversation and personal context more fully before answering."
        case .correction: return "Remember my correction when answering similar factual questions."
        }
    }
}

struct ChatLearningFeedbackDraft {
    var focus: ChatLearningFocus = .concise
    var note = ""
    var appliesToFuture = false
    private(set) var saveFailed = false

    var isDirty: Bool { focus != .concise || !note.isEmpty || appliesToFuture || saveFailed }

    var statement: String {
        let detail = note.trimmingCharacters(in: .whitespacesAndNewlines)
        return detail.isEmpty ? focus.statement : "\(focus.statement) \(detail)"
    }

    mutating func save(using persist: (String, Bool) -> Bool) -> Bool {
        let saved = persist(statement, appliesToFuture)
        saveFailed = !saved
        return saved
    }

    static func didPersist(_ response: [String: AgentCoreJSONValue]?) -> Bool {
        guard case .bool(true)? = response?["ok"] else { return false }
        return true
    }
}

private struct ChatLearningFeedbackSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let message: ChatMessage
    let onSave: (String, Bool) -> Bool

    @State private var draft = ChatLearningFeedbackDraft()
    @State private var showDiscardConfirmation = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(NSLocalizedString("chat.learning.title", value: "Teach Nola to understand you", comment: "Learning sheet title"))
                            .font(.title3.weight(.semibold))
                        Text(NSLocalizedString("chat.learning.subtitle", value: "Save a correction in local personal intelligence records, linked to this answer. Saving does not regenerate the answer.", comment: "Learning sheet explanation"))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text(NSLocalizedString("chat.learning.preview", value: "This answer", comment: "Answer preview label"))
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(message.content)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(12)
                            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    }

                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(ChatLearningFocus.allCases) { item in
                                Button {
                                    withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) { draft.focus = item }
                                } label: {
                                    Text(item.title)
                                        .font(.subheadline.weight(.medium))
                                        .foregroundStyle(draft.focus == item ? Color.white : Color.primary)
                                        .padding(.horizontal, 13)
                                        .frame(minHeight: 44)
                                        .background(
                                            Capsule().fill(draft.focus == item ? Color.accentColor : Color(.secondarySystemBackground))
                                        )
                                }
                                .buttonStyle(.plain)
                                .accessibilityAddTraits(draft.focus == item ? .isSelected : [])
                            }
                        }
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text(NSLocalizedString("chat.learning.note", value: "What should change?", comment: "Correction detail label"))
                            .font(.subheadline.weight(.semibold))
                        TextEditor(text: $draft.note)
                            .accessibilityLabel(NSLocalizedString("chat.learning.note", value: "What should change?", comment: "Correction detail label"))
                            .font(.body)
                            .frame(minHeight: 86)
                            .padding(8)
                            .scrollContentBackground(.hidden)
                            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .overlay(alignment: .topLeading) {
                                if draft.note.isEmpty {
                                    Text(NSLocalizedString("chat.learning.note.placeholder", value: "Tell Nola exactly how this answer should improve", comment: "Correction detail placeholder"))
                                        .font(.body)
                                        .foregroundStyle(.tertiary)
                                        .padding(.horizontal, 13)
                                        .padding(.vertical, 16)
                                        .allowsHitTesting(false)
                                }
                            }
                    }

                    VStack(alignment: .leading, spacing: 4) {
                        Toggle(
                            NSLocalizedString("chat.learning.future", value: "Use in future answers", comment: "Persist correction toggle"),
                            isOn: $draft.appliesToFuture
                        )
                        .font(.body.weight(.medium))
                        Text(NSLocalizedString("chat.learning.future.detail", value: "When enabled, also save a lasting preference for future answers. When off, save only the correction record, not a lasting preference.", comment: "Persist correction explanation"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.bottom, 12)

                    if draft.saveFailed {
                        Label(NSLocalizedString("chat.learning.save_failed", value: "Could not save the correction. Your draft is still here; try again.", comment: "Feedback save error"), systemImage: "exclamationmark.triangle")
                            .font(.callout)
                            .foregroundStyle(.red)
                            .accessibilityIdentifier("chat.learning.save_error")
                    }
                }
                .padding(20)
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(NSLocalizedString("chat.learning.cancel", value: "Cancel", comment: "Cancel learning feedback")) {
                        if draft.isDirty { showDiscardConfirmation = true } else { dismiss() }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(NSLocalizedString("chat.learning.save", value: "Save correction", comment: "Save learning feedback")) {
                        if draft.save(using: onSave) { dismiss() }
                    }
                    .fontWeight(.semibold)
                }
            }
        }
        .interactiveDismissDisabled(draft.isDirty)
        .confirmationDialog(NSLocalizedString("common.discard_changes", value: "Discard Changes?", comment: "Discard changes title"), isPresented: $showDiscardConfirmation, titleVisibility: .visible) {
            Button(NSLocalizedString("common.discard", value: "Discard", comment: "Discard changes"), role: .destructive) { dismiss() }
            Button(L10n.cancel, role: .cancel) { }
        }
    }
}
