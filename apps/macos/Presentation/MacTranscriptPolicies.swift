// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Pure transcript window, follow, navigation, animation, and context publication policies.

import Foundation
import SwiftUI
import AppKit
import Observation

/// Window-local presentation state. It contains no messages, execution state or Store writes.
@MainActor
final class MacTranscriptViewportStore {
    struct Snapshot: Equatable {
        var messageID: UUID?
        var offset: CGFloat = 0
        var followsLatest = true
        var visibleMessageLimit = 20
    }
    private var snapshots: [String: Snapshot] = [:]
    func snapshot(for conversationID: String) -> Snapshot? { snapshots[conversationID] }
    func save(_ snapshot: Snapshot, for conversationID: String) {
        guard !conversationID.isEmpty else { return }
        snapshots[conversationID] = snapshot
    }
    func setVisibleLimit(_ limit: Int, for conversationID: String) {
        var snapshot = snapshots[conversationID] ?? Snapshot()
        snapshot.visibleMessageLimit = limit
        save(snapshot, for: conversationID)
    }
}

/// Owns viewport intent independently of asynchronous layout observations.
/// Geometry can satisfy an intent, but cannot manufacture a user gesture.
@MainActor @Observable
final class MacTranscriptScrollController {
    enum Intent: Equatable { case latest, reading, locating }
    private(set) var intent: Intent = .latest
    private(set) var generation: UInt64 = 0
    @ObservationIgnored private weak var scrollView: NSScrollView?
    @ObservationIgnored private var pendingLayout: DispatchWorkItem?
    @ObservationIgnored private var userInteracting = false
    @ObservationIgnored private var observedUserScroll = false
    @ObservationIgnored private var conversationID = ""
    @ObservationIgnored private var viewportStore: MacTranscriptViewportStore?
    private struct Anchor { let messageID: UUID; let offset: CGFloat }
    private final class Marker { weak var view: NSView?; init(_ view: NSView) { self.view = view } }
    @ObservationIgnored private var markers: [UUID: Marker] = [:]
    @ObservationIgnored private var readingAnchor: Anchor?
    @ObservationIgnored private weak var latestAnchor: NSView?
    @ObservationIgnored private var requiresLatestAnchor = false
    @ObservationIgnored private var viewportSize: NSSize = .zero

    var followsLatest: Bool { intent == .latest }

    func reset(conversationID: String, viewportStore: MacTranscriptViewportStore? = nil) {
        guard self.conversationID != conversationID else { return }
        saveViewport()
        self.viewportStore = viewportStore
        self.conversationID = conversationID
        generation &+= 1
        let snapshot = viewportStore?.snapshot(for: conversationID)
        intent = snapshot?.followsLatest == false ? .reading : .latest
        userInteracting = false
        readingAnchor = snapshot.flatMap { value in value.messageID.map { Anchor(messageID: $0, offset: value.offset) } }
        invalidateLayout()
        contentChanged()
    }

    func saveViewport() {
        guard let viewportStore, !conversationID.isEmpty else { return }
        if intent == .reading { captureReadingAnchor() }
        // A locating request is transient; preserve the previously saved reading intent.
        guard intent != .locating else { return }
        var snapshot = viewportStore.snapshot(for: conversationID) ?? .init()
        snapshot.messageID = readingAnchor?.messageID
        snapshot.offset = readingAnchor?.offset ?? 0
        snapshot.followsLatest = followsLatest
        viewportStore.save(snapshot, for: conversationID)
    }

    func attach(_ scrollView: NSScrollView) {
        guard self.scrollView !== scrollView else { return }
        detach()
        self.scrollView = scrollView
        viewportSize = scrollView.contentView.bounds.size
        contentChanged()
    }

    func detach() {
        saveViewport()
        generation &+= 1
        invalidateLayout()
        scrollView = nil
        userInteracting = false
    }

    func userBeganScrolling() {
        generation &+= 1
        userInteracting = true
        observedUserScroll = false
        intent = .reading
        invalidateLayout()
    }

    func userScrolled(distanceFromBottom: CGFloat) {
        guard userInteracting else { return }
        observedUserScroll = true
        intent = distanceFromBottom <= 2 ? .latest : .reading
    }

    func userEndedScrolling(distanceFromBottom: CGFloat) {
        // A session switch, explicit navigation, or jump-to-latest can retire
        // the gesture before AppKit delivers its final notification.
        guard userInteracting else { return }
        // Async hydration can change geometry after the last real scroll event.
        // Ending a gesture must not reinterpret that reflow as user intent.
        // Discrete wheels have no didLiveScroll event, so sample their endpoint.
        if !observedUserScroll { userScrolled(distanceFromBottom: distanceFromBottom) }
        if intent == .reading { captureReadingAnchor() }
        userInteracting = false
        saveViewport()
        contentChanged()
    }

    func returnToLatest() {
        generation &+= 1
        intent = .latest
        userInteracting = false
        saveViewport()
        invalidateLayout()
        contentChanged()
    }

    func beginNavigation() -> UInt64 {
        generation &+= 1
        userInteracting = false
        intent = .locating
        invalidateLayout()
        return generation
    }

    func acceptsNavigation(_ generation: UInt64) -> Bool {
        self.generation == generation && intent == .locating
    }

    func register(messageID: UUID, view: NSView) {
        guard markers[messageID]?.view !== view else { return }
        markers[messageID] = Marker(view)
        contentChanged()
    }

    func unregister(messageID: UUID, view: NSView) {
        if markers[messageID]?.view === view { markers.removeValue(forKey: messageID) }
    }

    func registerLatestAnchor(_ view: NSView) {
        guard latestAnchor !== view else { return }
        requiresLatestAnchor = true
        latestAnchor = view
        contentChanged()
    }

    func unregisterLatestAnchor(_ view: NSView) {
        if latestAnchor === view { latestAnchor = nil }
    }

    func distanceFromLatest() -> CGFloat {
        guard let scrollView, let document = scrollView.documentView else { return .infinity }
        let visible = scrollView.documentVisibleRect
        if let latestAnchor, latestAnchor.isDescendant(of: document) {
            let frame = latestAnchor.convert(latestAnchor.bounds, to: document)
            guard frame.width > 0 else { return .infinity }
            return max(0, document.isFlipped ? frame.maxY - visible.maxY : visible.minY - frame.minY)
        }
        guard !requiresLatestAnchor else { return .infinity }
        return max(0, document.isFlipped
            ? document.bounds.maxY - visible.maxY : visible.minY - document.bounds.minY)
    }

    private func captureReadingAnchor() {
        guard let scrollView, let document = scrollView.documentView else { return }
        let visible = scrollView.documentVisibleRect
        let frames = markers.compactMap { id, marker -> (UUID, NSRect)? in
            guard let view = marker.view, view.isDescendant(of: document) else { return nil }
            return (id, view.convert(view.bounds, to: document))
        }.filter { $0.1.intersects(visible) }
        let first = frames.sorted { document.isFlipped ? $0.1.minY < $1.1.minY : $0.1.maxY > $1.1.maxY }.first
        guard let (id, frame) = first else { return }
        readingAnchor = Anchor(messageID: id, offset: document.isFlipped
            ? visible.minY - frame.minY : visible.maxY - frame.maxY)
    }

    func contentChanged() {
        guard scrollView != nil, !userInteracting else { return }
        guard pendingLayout == nil else { return }
        let generation = generation
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.generation == generation else { return }
            self.pendingLayout = nil
            self.reconcileLayout()
        }
        pendingLayout = work
        // Coalesce stream/layout notifications to one reconciliation per frame.
        // Do not debounce: a continuous stream must not postpone following.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0 / 60.0, execute: work)
    }

    func viewportChanged() {
        guard let size = scrollView?.contentView.bounds.size, size != viewportSize else { return }
        viewportSize = size
        contentChanged()
    }

    /// Called after AppKit layout, also testable with a real NSScrollView.
    func reconcileLayout() {
        guard !userInteracting,
              let scrollView, let document = scrollView.documentView else { return }
        let clip = scrollView.contentView
        let minimumY = document.bounds.minY
        let maximumY = max(minimumY, document.bounds.maxY - clip.bounds.height)
        let targetY: CGFloat
        switch intent {
        case .latest:
            if let latestAnchor, latestAnchor.isDescendant(of: document) {
                // The eager footer is the actual transcript end. A lazy history
                // can leave a temporarily larger document extent after terminal
                // reflow; following that estimate would display empty space.
                let frame = latestAnchor.convert(latestAnchor.bounds, to: document)
                guard frame.width > 0, frame.minY.isFinite, frame.maxY.isFinite else { return }
                targetY = min(maximumY, max(minimumY,
                    document.isFlipped ? frame.maxY - clip.bounds.height : frame.minY))
            } else {
                // Once a transcript has a measured end, a transient remount
                // must not send the viewport to a stale document estimate.
                guard !requiresLatestAnchor else { return }
                targetY = document.isFlipped ? maximumY : minimumY
            }
        case .reading:
            guard let anchor = readingAnchor, let view = markers[anchor.messageID]?.view,
                  view.isDescendant(of: document) else { return }
            let frame = view.convert(view.bounds, to: document)
            targetY = max(minimumY, min(maximumY,
                (document.isFlipped ? frame.minY : frame.maxY - clip.bounds.height) + anchor.offset))
        case .locating:
            return
        }
        guard abs(clip.bounds.origin.y - targetY) > 0.5 else { return }
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: targetY))
        scrollView.reflectScrolledClipView(clip)
    }

    private func invalidateLayout() {
        pendingLayout?.cancel()
        pendingLayout = nil
    }
}

nonisolated func macContinuousStatusAnimationEnabled(
    isActive: Bool,
    reduceMotion: Bool,
    isVisible: Bool = true
) -> Bool {
    isActive && isVisible && !reduceMotion
}

struct MacTranscriptWindow {
    let messages: [MacChatMessage]
    let hasEarlierVisibleMessages: Bool
    let inspectedMessageCount: Int
}

enum MacTranscriptWindowPolicy {
    static func project(
        _ messages: [MacChatMessage],
        isRunning: Bool,
        visibleMessageLimit: Int
    ) -> MacTranscriptWindow {
        let limit = max(1, visibleMessageLimit)
        let byteBudget = max(160_000, limit * 8_000)
        let activeMessageID = isRunning ? messages.last?.id : nil
        var selected: [MacChatMessage] = []
        selected.reserveCapacity(min(limit, messages.count))
        var consumedBytes = 0
        var inspectedMessageCount = 0
        var hasEarlierVisibleMessages = false

        for message in messages.reversed() {
            inspectedMessageCount += 1
            guard macChatMessageIsVisible(
                message,
                isActive: message.id == activeMessageID
            ) else { continue }
            if selected.count >= limit {
                hasEarlierVisibleMessages = true
                break
            }
            let messageBytes = message.text.utf8.count
            if !selected.isEmpty, consumedBytes + messageBytes > byteBudget {
                hasEarlierVisibleMessages = true
                break
            }
            selected.append(message)
            consumedBytes += messageBytes
        }

        return MacTranscriptWindow(
            messages: selected.reversed(),
            hasEarlierVisibleMessages: hasEarlierVisibleMessages,
            inspectedMessageCount: inspectedMessageCount
        )
    }
}

struct MacCompactionSourceRequest: Equatable {
    let conversationID: String
    let messageID: UUID
    let compactionPartID: String
    let sourceItemIDs: [String]
    let sourceFingerprint: String

    func matches(conversationID: String, messages: [MacChatMessage]) -> Bool {
        guard self.conversationID == conversationID,
              let message = messages.first(where: { $0.id == messageID }) else { return false }
        return message.presentation?.renderedCompactionReferences.contains(where: { reference in
            reference.id == compactionPartID
                && reference.sourceItemIDs == sourceItemIDs
                && reference.sourceFingerprint == sourceFingerprint
        }) == true
    }
}

enum MacCompactionSourceNavigator {
    enum LoadError: Error {
        case snapshotChanged
    }

    struct Page: Equatable {
        let requestedBeforeOrdinal: Int64?
        let beforeOrdinal: Int64?
        let hasMore: Bool
        let messages: [MacChatMessage]
    }

    enum Outcome: Equatable {
        case found(messageID: UUID, page: Page)
        case end
        case stalled
        case snapshotChanged
        case readError
        case cancelled
    }

    static func targetMessageID(
        sourceItemIDs: [String],
        messages: [MacChatMessage]
    ) -> UUID? {
        let sourceIDs = Set(sourceItemIDs)
        return messages.first { message in
            guard let canonicalItemIDs = message.canonicalItemIDs else { return false }
            return !canonicalItemIDs.isDisjoint(with: sourceIDs)
        }?.id
    }

    static func locate(
        sourceItemIDs: [String],
        initialPage: Page,
        loadEarlier: (Int64) async throws -> Page
    ) async -> Outcome {
        var page = initialPage
        var requestedCursors = Set<Int64>()
        while true {
            if let target = targetMessageID(sourceItemIDs: sourceItemIDs, messages: page.messages) {
                return .found(messageID: target, page: page)
            }
            guard page.hasMore else { return .end }
            guard let cursor = page.beforeOrdinal,
                  requestedCursors.insert(cursor).inserted,
                  page.requestedBeforeOrdinal == nil || cursor < page.requestedBeforeOrdinal! else {
                return .stalled
            }
            do {
                page = try await loadEarlier(cursor)
            } catch LoadError.snapshotChanged {
                return .snapshotChanged
            } catch is CancellationError {
                return .cancelled
            } catch {
                return .readError
            }
        }
    }
}

nonisolated enum MacVisibleRunStatePolicy {
    static func isRunning(
        internalIsRunning: Bool,
        activeConversationID: String?,
        terminalObservedConversationIDs: Set<String>
    ) -> Bool {
        guard internalIsRunning, let activeConversationID else { return false }
        return !terminalObservedConversationIDs.contains(activeConversationID)
    }
}

enum MacContextSnapshotPublicationPolicy {
    static func shouldPublish(
        current: MacConversationContextSnapshot?,
        next: MacConversationContextSnapshot
    ) -> Bool {
        guard let current else { return true }
        guard current != next else { return false }
        // Context occupancy may be coalesced; provider cache evidence is an
        // independent metric and must never wait for an occupancy bucket change.
        guard current.conversationCacheUsage == next.conversationCacheUsage,
              current.cacheUsage == next.cacheUsage,
              current.cachedInputTokens == next.cachedInputTokens,
              current.cacheWriteInputTokens == next.cacheWriteInputTokens,
              current.promptCacheDiagnostic == next.promptCacheDiagnostic,
              current.measurement == next.measurement,
              current.promptCacheMeasurement == next.promptCacheMeasurement else {
            return true
        }
        guard current.contextWindowTokens == next.contextWindowTokens,
              current.promptTokenLimit == next.promptTokenLimit,
              current.isEstimated == next.isEstimated else {
            return true
        }
        return usageBucket(current) != usageBucket(next)
    }

    private static func usageBucket(_ snapshot: MacConversationContextSnapshot) -> Int {
        let window = max(1, snapshot.contextWindowTokens)
        return Int(
            (Double(max(0, snapshot.usedTokens)) / Double(window) * 1_000)
                .rounded(.down)
        )
    }
}
