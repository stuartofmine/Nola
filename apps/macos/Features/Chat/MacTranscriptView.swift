// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Renders the macOS transcript window and owns scroll-follow/history navigation behavior.

import AppKit
import SwiftUI

struct MacUISurfaceContext {
    let readChanges: (AppleUISurfaceIdentity, UInt64?) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue>
    let preview: (AppleUIPreviewInput<MacCanonicalJSONValue>) async throws -> AppleUIPreviewResult<MacCanonicalJSONValue>
    let readRevision: (AppleUISurfaceReference) async throws -> AppleUISnapshot<MacCanonicalJSONValue>?
    let pendingSubmission: (AppleUISurfaceIdentity) throws -> AppleUISubmission<MacCanonicalJSONValue>?
    let submit: (AppleUISubmission<MacCanonicalJSONValue>) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue>
    var permitsFollowupActions = true
    var readLocalSource: () throws -> Bool = { false }
}

enum MacTranscriptMessageMotionPolicy {
    static func appendedIDs(
        previousConversationID: String?,
        conversationID: String,
        previous: [UUID]?,
        current: [UUID]
    ) -> Set<UUID> {
        guard previousConversationID == conversationID,
              let previous, current.count > previous.count,
              current.prefix(previous.count).elementsEqual(previous) else { return [] }
        return Set(current.dropFirst(previous.count))
    }
}
private struct MacUISurfaceContextKey: EnvironmentKey {
    static let defaultValue: MacUISurfaceContext? = nil
}
extension EnvironmentValues {
    var uiSurfaceContext: MacUISurfaceContext? {
        get { self[MacUISurfaceContextKey.self] }
        set { self[MacUISurfaceContextKey.self] = newValue }
    }
}

struct MacTranscriptView: View {
    let conversationID: String
    let messages: [MacChatMessage]
    let isRunning: Bool
    let liveActivities: [MacAgentActivity]
    let searchText: String
    let bottomClearance: CGFloat
    var viewportStore: MacTranscriptViewportStore? = nil
    var terminalRevision: UInt64 = 0
    let onSendTaskGuidance: (String, String, String) -> Void
    let onRequestTaskCancellation: (String, String) -> Void
    var hasEarlierStoredMessages = false
    var isViewingHistory = false
    var isLoadingHistoryPage = false
    var onLoadEarlierHistory: () -> Void = {}
    var onReturnToLatestHistory: () -> Void = {}
    var onLocateCompactionSource: (MacCompactionSourceRequest) async -> MacCompactionSourceNavigator.Outcome = { _ in .end }
    var previewUI: (AppleUIPreviewInput<MacCanonicalJSONValue>) async throws -> AppleUIPreviewResult<MacCanonicalJSONValue> = { _ in throw MacRemoteHostError.invalidResponse }
    var readUIChanges: (AppleUISurfaceIdentity, UInt64?) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue> = { _, _ in throw MacRemoteHostError.invalidResponse }
    var readUIRevision: (AppleUISurfaceReference) async throws -> AppleUISnapshot<MacCanonicalJSONValue>? = { _ in nil }
    var pendingUI: (AppleUISurfaceIdentity) throws -> AppleUISubmission<MacCanonicalJSONValue>? = { _ in nil }
    var submitUI: (AppleUISubmission<MacCanonicalJSONValue>) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue> = { _ in .rejected("交互入口不可用。") }
    var readLocalSource: () throws -> Bool = { false }
    @State private var scrollController = MacTranscriptScrollController()
    @State private var journalResidency = MacJournalPresentationResidency()
    @State private var trackedConversationID = ""
    @State private var visibleMessageLimit = 20
    @State private var compactionSourceNotice: String?
    @State private var compactionNavigationID: UUID?
    @State private var knownMessageIDs: [UUID]?
    @State private var knownMessageConversationID: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            if let compactionSourceNotice {
                MacInlineNotice(
                    title: String(localized: "Original context", table: "Chat"),
                    message: compactionSourceNotice,
                    kind: .failure,
                    dismiss: { self.compactionSourceNotice = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.chatTranscriptHorizontalPadding)
                .padding(.top, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            GeometryReader { availableSpace in
            let transcriptWidth = MacChatBubbleLayout.transcriptWidth(
                availableWidth: availableSpace.size.width
            )
            let transcriptWindow = MacTranscriptWindowPolicy.project(
                messages,
                isRunning: isRunning,
                visibleMessageLimit: visibleMessageLimit
            )
            let appendedMessageIDs = MacTranscriptMessageMotionPolicy.appendedIDs(
                previousConversationID: knownMessageConversationID,
                conversationID: conversationID,
                previous: knownMessageIDs,
                current: messages.map(\.id)
            )
            let enteringMessageIDs = isRunning ? appendedMessageIDs : Set<UUID>()

            ScrollViewReader { proxy in
                ScrollView {
                // The window policy already bounds resident messages. Keep one
                // measured container: lazy height estimates feeding native follow
                // can oscillate, and moving the previous tail to another parent
                // destroys its disclosure/editor identity on every new message.
                VStack(spacing: 18) {
                    if isViewingHistory {
                        Button(action: onReturnToLatestHistory) {
                            Label(String(localized: "Go to Latest Message"), systemImage: "arrow.down.circle")
                        }
                        .buttonStyle(.plain)
                        .font(.system(size: 12, weight: .medium))
                        .disabled(isLoadingHistoryPage)
                        .accessibilityIdentifier("chat.history.returnToLatest")
                    }
                    if transcriptWindow.hasEarlierVisibleMessages || hasEarlierStoredMessages {
                        Button {
                            if transcriptWindow.hasEarlierVisibleMessages {
                                visibleMessageLimit += 20
                            } else { onLoadEarlierHistory() }
                        } label: {
                            Label(
                                String(localized: "Load earlier messages"),
                                systemImage: "arrow.up.circle"
                            )
                        }
                        .buttonStyle(.plain)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                        .padding(.vertical, 6)
                        .disabled(isLoadingHistoryPage)
                        .accessibilityIdentifier("chat.history.loadEarlier")
                    }
                    ForEach(transcriptWindow.messages) { message in
                        messageRow(message, transcriptWidth: transcriptWidth)
                            .transition(enteringMessageIDs.contains(message.id) && !reduceMotion
                                ? .opacity.combined(with: .offset(y: 8))
                                : .identity)
                            .transaction { transaction in
                                if enteringMessageIDs.contains(message.id) {
                                    transaction.animation = reduceMotion ? nil : .easeOut(duration: 0.19)
                                }
                            }
                    }
                    Color.clear
                        .frame(height: max(0, bottomClearance - 22))
                        .id("latest")
                        .background {
                            MacTranscriptLatestAnchor(controller: scrollController)
                        }
                }
                .frame(width: transcriptWidth)
                .padding(.horizontal, MacDesignTokens.Layout.chatTranscriptHorizontalPadding)
                .padding(.top, 24)
                .frame(maxWidth: .infinity)
                .onGeometryChange(for: CGSize.self) { geometry in
                    geometry.size
                } action: { _ in
                    scrollController.contentChanged()
                }
                .background {
                    MacTranscriptScrollObserver(controller: scrollController)
                    .frame(width: 0, height: 0)
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: .macCompactionSourceRequested)) {
                notification in
                guard let request = notification.object as? MacCompactionSourceRequest,
                      !request.sourceItemIDs.isEmpty,
                      !request.sourceFingerprint.isEmpty,
                      request.matches(conversationID: conversationID, messages: messages) else { return }
                guard compactionNavigationID == nil else { return }
                let requestID = UUID()
                compactionNavigationID = requestID
                let navigationGeneration = scrollController.beginNavigation()
                Task { @MainActor in
                    let outcome = await onLocateCompactionSource(request)
                    // A retired request cannot clear a newer session's pending
                    // navigation, even when the old async read finishes last.
                    guard compactionNavigationID == requestID else { return }
                    compactionNavigationID = nil
                    guard scrollController.acceptsNavigation(navigationGeneration) else { return }
                    switch outcome {
                    case .found(let target, let page):
                        // A canonical page may contain more than the default
                        // tail slice. Make the exact matched item renderable
                        // before asking ScrollViewReader to locate it.
                        visibleMessageLimit = max(visibleMessageLimit, page.messages.count)
                        await Task.yield()
                        guard scrollController.acceptsNavigation(navigationGeneration) else { return }
                        proxy.scrollTo(target, anchor: .center)
                    case .end:
                        compactionSourceNotice = String(localized: "mac.chat.compaction_source.not_found")
                    case .stalled:
                        compactionSourceNotice = String(localized: "mac.chat.compaction_source.cursor_stalled")
                    case .snapshotChanged:
                        compactionSourceNotice = String(localized: "mac.chat.compaction_source.snapshot_changed")
                    case .readError:
                        compactionSourceNotice = String(localized: "mac.chat.compaction_source.read_failed")
                    case .cancelled:
                        break
                    }
                }
            }
            .task(id: conversationID) {
                if trackedConversationID != conversationID {
                    trackedConversationID = conversationID
                    compactionNavigationID = nil
                    compactionSourceNotice = nil
                    visibleMessageLimit = viewportStore?.snapshot(for: conversationID)?.visibleMessageLimit ?? 20
                    scrollController.reset(conversationID: conversationID, viewportStore: viewportStore)
                }
            }
            .onChange(of: transcriptWindow.messages.compactMap { $0.journalReference?.ownerKey }, initial: true) { _, keys in
                journalResidency.retain(Set(keys))
            }
            .onChange(of: visibleMessageLimit) { _, limit in
                viewportStore?.setVisibleLimit(limit, for: conversationID)
            }
            .onDisappear {
                scrollController.saveViewport()
                compactionNavigationID = nil
                journalResidency.removeAll()
            }
            .onChange(of: terminalRevision) { _, _ in
                scrollController.contentChanged()
            }
            .onChange(of: messages.map(\.id), initial: true) { _, ids in
                knownMessageIDs = ids
                knownMessageConversationID = conversationID
            }
            .onChange(of: conversationID, initial: true) { _, id in
                // New history is a replacement, never a live append animation.
                knownMessageIDs = nil
                knownMessageConversationID = id
            }
            .onChange(of: isRunning) { _, _ in
                scrollController.contentChanged()
            }
            .overlay(alignment: .bottomTrailing) {
                MacTranscriptFollowControl(controller: scrollController, bottomClearance: bottomClearance)
            }
                // Follow is viewport intent, not an animation of the document.
                // Animating this ancestor also animates simultaneous text reflow.
            }
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: compactionSourceNotice)
    }

    private func messageRow(_ message: MacChatMessage, transcriptWidth: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 12) {
        MacMessageSurfaceContent(scope: .init(conversationID: conversationID,
                messageID: message.canonicalMessageID ?? message.journalReference?.cursor.identity.assistant_message_id ?? "",
                runID: message.journalReference?.cursor.identity.run_id),
            context: .init(readChanges: readUIChanges, preview: previewUI,
                readRevision: readUIRevision, pendingSubmission: pendingUI, submit: submitUI, readLocalSource: readLocalSource),
            isCurrent: { trackedConversationID == conversationID }) {
        MacMessageView(
            conversationID: conversationID,
            message: message,
            transcriptWidth: transcriptWidth,
            isStreaming: isRunning && message.role == .assistant && message.id == messages.last?.id,
            liveActivities: isRunning && message.role == .assistant && message.id == messages.last?.id
                ? liveActivities : [],
            isSearchMatch: !searchText.isEmpty && message.text.localizedCaseInsensitiveContains(searchText),
            onSendTaskGuidance: { taskID, text in
                guard let runID = message.journalReference?.cursor.identity.run_id else { return }
                onSendTaskGuidance(taskID, runID, text)
            },
            onRequestTaskCancellation: { taskID in
                guard let runID = message.journalReference?.cursor.identity.run_id else { return }
                onRequestTaskCancellation(taskID, runID)
            },
            journalPresentationOwner: message.journalReference.map { journalResidency.owner(for: $0) }
        )
        }
        }
        .id(message.id)
        .background {
            MacTranscriptMessageAnchor(messageID: message.id, controller: scrollController)
        }
    }

}

/// Observe viewport intent here, not in the body that projects all message
/// rows. Beginning a scroll only changes this control's visibility.
struct MacTranscriptFollowControl: View {
    let controller: MacTranscriptScrollController
    let bottomClearance: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            if !controller.followsLatest {
                Button { controller.returnToLatest() } label: {
                    Image(systemName: "arrow.down")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 28, height: 28)
                        .background(.regularMaterial, in: Circle())
                        .overlay { Circle().stroke(Color.primary.opacity(0.08), lineWidth: 0.5) }
                }
                .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init()))
                .accessibilityIdentifier("chat.transcript.followLatest")
                .padding(.trailing, 18)
                .padding(.bottom, bottomClearance + 8)
                .help("Go to Latest Message")
                .accessibilityLabel(String(localized: "Go to Latest Message"))
                .transition(reduceMotion ? .opacity : .scale(scale: 0.94).combined(with: .opacity))
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: controller.followsLatest)
    }
}

nonisolated func macChatMessageIsVisible(_ message: MacChatMessage, isActive: Bool) -> Bool {
    if isActive || message.role != .assistant || message.journalReference != nil
        || !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return true
    }
    return message.presentation?.hasVisibleContent == true
}

struct MacTranscriptLatestAnchor: NSViewRepresentable {
    let controller: MacTranscriptScrollController

    func makeNSView(context: Context) -> AnchorView {
        let view = AnchorView()
        view.controller = controller
        controller.registerLatestAnchor(view)
        return view
    }

    func updateNSView(_ view: AnchorView, context: Context) {
        controller.registerLatestAnchor(view)
    }

    static func dismantleNSView(_ view: AnchorView, coordinator: ()) {
        view.controller?.unregisterLatestAnchor(view)
        view.controller = nil
    }

    final class AnchorView: NSView {
        weak var controller: MacTranscriptScrollController?

        override func setFrameOrigin(_ newOrigin: NSPoint) {
            guard frame.origin != newOrigin else { return }
            super.setFrameOrigin(newOrigin)
            controller?.contentChanged()
        }

        override func setFrameSize(_ newSize: NSSize) {
            guard frame.size != newSize else { return }
            super.setFrameSize(newSize)
            controller?.contentChanged()
        }

        override func viewDidMoveToSuperview() {
            super.viewDidMoveToSuperview()
            controller?.contentChanged()
        }
    }
}

struct MacTranscriptMessageAnchor: NSViewRepresentable {
    let messageID: UUID
    let controller: MacTranscriptScrollController
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        controller.register(messageID: messageID, view: view)
        return view
    }
    func updateNSView(_ view: NSView, context: Context) {
        controller.register(messageID: messageID, view: view)
    }
    func makeCoordinator() -> Coordinator { Coordinator(messageID: messageID, controller: controller) }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        coordinator.controller.unregister(messageID: coordinator.messageID, view: view)
    }
    @MainActor final class Coordinator {
        let messageID: UUID
        let controller: MacTranscriptScrollController
        init(messageID: UUID, controller: MacTranscriptScrollController) {
            self.messageID = messageID
            self.controller = controller
        }
    }
}

struct MacTranscriptScrollObserver: NSViewRepresentable {
    let controller: MacTranscriptScrollController

    func makeCoordinator() -> Coordinator { Coordinator(controller: controller) }
    func makeNSView(context: Context) -> NSView {
        let view = AttachmentView(frame: .zero)
        view.didAttach = { [weak coordinator = context.coordinator] view in
            coordinator?.attachWhenReady(to: view)
        }
        context.coordinator.attachWhenReady(to: view)
        return view
    }
    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.attachWhenReady(to: view)
    }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        (view as? AttachmentView)?.didAttach = nil
        coordinator.detach()
    }

    private final class AttachmentView: NSView {
        var didAttach: ((NSView) -> Void)?
        override func viewDidMoveToSuperview() {
            super.viewDidMoveToSuperview()
            didAttach?(self)
        }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            didAttach?(self)
        }
    }

    @MainActor final class Coordinator {
        let controller: MacTranscriptScrollController
        private weak var scrollView: NSScrollView?
        private var observations: [NSObjectProtocol] = []
        private var generation: UInt64 = 0
        private var pendingAttachment = false
        private var wheelMonitor: Any?

        init(controller: MacTranscriptScrollController) { self.controller = controller }

        func attachWhenReady(to view: NSView) {
            guard !pendingAttachment else { return }
            if let scrollView, view.enclosingScrollView === scrollView { return }
            pendingAttachment = true
            let generation = generation
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self, self.generation == generation else { return }
                self.pendingAttachment = false
                guard let scroll = view?.enclosingScrollView else { return }
                self.attach(scroll)
            }
        }

        func detach() {
            generation &+= 1
            pendingAttachment = false
            observations.forEach(NotificationCenter.default.removeObserver)
            observations.removeAll()
            if let wheelMonitor { NSEvent.removeMonitor(wheelMonitor) }
            wheelMonitor = nil
            scrollView = nil
            controller.detach()
        }

        private func observe(_ name: Notification.Name, object: AnyObject,
                             action: @escaping @MainActor () -> Void) {
            observations.append(NotificationCenter.default.addObserver(
                forName: name, object: object, queue: .main
            ) { _ in MainActor.assumeIsolated { action() } })
        }

        func attach(_ scroll: NSScrollView) {
            guard scrollView !== scroll else { return }
            detach()
            scrollView = scroll
            controller.attach(scroll)
            // Discrete mouse wheels have no live-scroll phases. Attribute only events
            // aimed at this scroll view, never a nested code/output scroll region.
            wheelMonitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                MainActor.assumeIsolated {
                    guard let self, let scroll = self.scrollView,
                          event.phase.isEmpty, event.momentumPhase.isEmpty,
                          event.window === scroll.window,
                          let hit = scroll.window?.contentView?.hitTest(event.locationInWindow),
                          (hit as? NSScrollView ?? hit.enclosingScrollView) === scroll else { return }
                    self.controller.userBeganScrolling()
                    let generation = self.controller.generation
                    DispatchQueue.main.async { [weak self] in
                        guard let self, self.controller.generation == generation else { return }
                        self.controller.userEndedScrolling(distanceFromBottom: self.distanceFromBottom())
                    }
                }
                return event
            }
            scroll.contentView.postsBoundsChangedNotifications = true
            observe(NSView.boundsDidChangeNotification, object: scroll.contentView) { [weak self] in
                // Our own scroll changes the origin; feeding it back into
                // contentChanged produces an extra layout pass for every frame.
                self?.controller.viewportChanged()
            }
            if let document = scroll.documentView {
                document.postsFrameChangedNotifications = true
                observe(NSView.frameDidChangeNotification, object: document) { [weak self] in
                    self?.controller.contentChanged()
                }
            }
            observe(NSScrollView.willStartLiveScrollNotification, object: scroll) { [weak self] in
                self?.controller.userBeganScrolling()
            }
            observe(NSScrollView.didLiveScrollNotification, object: scroll) { [weak self] in
                guard let self else { return }
                self.controller.userScrolled(distanceFromBottom: self.distanceFromBottom())
            }
            observe(NSScrollView.didEndLiveScrollNotification, object: scroll) { [weak self] in
                guard let self else { return }
                self.controller.userEndedScrolling(distanceFromBottom: self.distanceFromBottom())
            }
        }

        private func distanceFromBottom() -> CGFloat {
            controller.distanceFromLatest()
        }
    }
}
