// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Renders one macOS chat message and its deferred/history content presentation.

import AppKit
import SwiftUI

enum MacAssistantFailurePresentationPolicy {
    static func hasPartialToolResults(
        deliveryState: MacChatMessage.DeliveryState?, traces: [MacAssistantTrace]
    ) -> Bool {
        guard deliveryState == .failed || deliveryState == .cancelled else { return false }
        return traces.contains { $0.kind == .tool && $0.state == .completed && !($0.result ?? "").isEmpty }
    }

    static func highlightsResponse(deliveryState: MacChatMessage.DeliveryState?) -> Bool {
        // Attempt failures remain visible in their own history, but successful
        // recovery must not make the completed answer look like a failed Run.
        deliveryState == .failed
    }

    static func showsFallbackCard(
        deliveryState: MacChatMessage.DeliveryState?,
        traces: [MacAssistantTrace]
    ) -> Bool {
        guard deliveryState == .failed || deliveryState == .cancelled else { return false }
        // Internal traces are collapsed, so the terminal result must remain
        // visible even when a detailed failed step is available.
        return true
    }
}

@MainActor
private struct MacDeferredTextPreview: View {
    let content: MacDeferredMessageContent
    @State private var isPresented = false

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "doc.text.magnifyingglass")
                .foregroundStyle(.secondary)
            Text("history.preview.label", tableName: "Chat")
                .font(.caption)
                .foregroundStyle(.secondary)
            Button { isPresented = true } label: {
                Text("history.reader.open", tableName: "Chat")
            }
            .buttonStyle(.link)
        }
        .accessibilityIdentifier("chat.deferredText.open")
        .sheet(isPresented: $isPresented) {
            MacDeferredTextReader(content: content).id(content)
        }
    }
}

struct MacMessageView: View {
    var conversationID = ""
    let message: MacChatMessage
    let transcriptWidth: CGFloat
    let isStreaming: Bool
    let liveActivities: [MacAgentActivity]
    let isSearchMatch: Bool
    let onSendTaskGuidance: (String, String) -> Void
    let onRequestTaskCancellation: (String) -> Void
    var userDisplayName: String? = nil
    var showsAnimatedStatusLight = true
    var showsRoleAvatar = true
    var assistantBubbleFill: Color? = nil
    var journalPresentationOwner: MacJournalPresentationOwner?
    @State private var copied = false
    @State private var copyTask: Task<Void, Never>?
    @State private var copyFeedbackTask: Task<Void, Never>?
    @State private var copyError: String?
    @State private var selectableResponse: MacMessageTextSelection?
    @State private var isHovering = false
    @State private var localJournalActivityCoverage = MacJournalActivityCoverage()
    @State private var localJournalReferenceCoverage = MacJournalReferenceCoverage()
    private var journalActivityCoverage: MacJournalActivityCoverage {
        get { journalPresentationOwner?.activityCoverage ?? localJournalActivityCoverage }
        nonmutating set {
            if let journalPresentationOwner { journalPresentationOwner.activityCoverage = newValue }
            else { localJournalActivityCoverage = newValue }
        }
    }
    private var journalReferenceCoverage: MacJournalReferenceCoverage {
        get { journalPresentationOwner?.referenceCoverage ?? localJournalReferenceCoverage }
        nonmutating set {
            if let journalPresentationOwner { journalPresentationOwner.referenceCoverage = newValue }
            else { localJournalReferenceCoverage = newValue }
        }
    }
    @State private var attachmentOpenError: String?
    @Environment(\.macMessageEditAction) private var editMessage
    @Environment(\.macTranscriptReadOnly) private var transcriptReadOnly
    @Environment(\.openURL) private var openURL
    @Environment(\.uiSurfaceContext) private var messageSourceContext
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var showsInlinePendingIndicator: Bool {
        // Keep text-only streaming status in the fixed avatar footprint.
        // Moving it above the first phrase inserts a row which disappears at
        // completion and pulls the entire answer upward.
        message.role == .assistant
            && isStreaming
            && (message.presentation?.traceItems.isEmpty ?? true)
    }

    private var showsMessageMetadata: Bool {
        guard message.role == .assistant else { return true }
        return assistantOutputIsComplete && hasAssistantResponse
    }

    private var assistantOutputIsComplete: Bool {
        guard !isStreaming else { return false }
        guard message.deliveryState != .running else { return false }
        guard message.deliveryState != .failed, message.deliveryState != .cancelled else { return false }
        return message.presentation?.isComplete ?? true
    }

    private var assistantFailureTitle: String? {
        switch message.deliveryState {
        case .failed: String(localized: "Response failed", table: "Chat")
        case .cancelled: String(localized: "Response cancelled", table: "Chat")
        default: nil
        }
    }

    private var assistantFailureColor: Color {
        message.deliveryState == .failed ? .red : .secondary
    }

    private var assistantFailureDetail: String? {
        message.presentation?.traceItems.last(where: { $0.state == .failed })?.detail
    }

    private var hasAssistantResponse: Bool {
        if message.journalReference != nil { return true }
        if let canonical = message.canonicalPresentation, !canonical.message.parts.isEmpty { return true }
        if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return true
        }
        if let presentation = message.presentation {
            return !presentation.renderedTextNodes.isEmpty
                || !presentation.renderedSourceReferences.isEmpty
                || !presentation.renderedCompactionReferences.isEmpty
        }
        return false
    }

    private var showsAssistantFailureFallback: Bool {
        MacAssistantFailurePresentationPolicy.showsFallbackCard(
            deliveryState: message.deliveryState,
            traces: message.presentation?.traceItems ?? []
        )
    }

    @ViewBuilder private var assistantResponseBody: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let reference = message.journalReference {
                MacJournalMessageTextView(reference: reference,
                    deliveryState: message.deliveryState,
                    onActivitiesLoaded: { identity, ids, taskIDs in
                        // A shared owner already published this handover.
                        guard journalPresentationOwner == nil else { return }
                        journalActivityCoverage.adopt(identity: identity, partIDs: ids, taskIDs: taskIDs)
                    }, onSendTaskGuidance: onSendTaskGuidance,
                    onRequestTaskCancellation: onRequestTaskCancellation,
                    taskGuidance: message.presentation?.renderedTaskGuidance ?? [:],
                    onReferencesLoaded: { identity, sourceIDs, citationIDs, attachmentIDs in
                        guard journalPresentationOwner == nil else { return }
                        journalReferenceCoverage.adopt(identity: identity,
                            sourceIDs: sourceIDs, citationIDs: citationIDs, attachmentIDs: attachmentIDs)
                    }, presentationOwner: journalPresentationOwner)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if let canonical = message.canonicalPresentation {
                MacCanonicalPartFlowView(conversationID: conversationID, canonical: canonical.message,
                    historicalDetails: message.historicalDetails ?? [],
                    isStreaming: isStreaming,
                    guidance: message.presentation?.renderedTaskGuidance ?? [:],
                    onSendGuidance: onSendTaskGuidance, onRequestCancellation: onRequestTaskCancellation,
                    onOpenAttachment: { openAttachments([$0]) })
                    .id(MacCanonicalReadingOwner(conversationID: conversationID, messageID: canonical.message.messageID, source: canonical.source))
            } else if assistantOutputIsComplete,
               !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                // Semantic phrase nodes keep streaming layout stable, but
                // Markdown constructs can span several nodes. Once the
                // response is durable, render the canonical text as one
                // document so headings, lists and tables regain context.
                MacMarkdownView(markdown: message.text, isStreaming: false, animatesAppends: false)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if let presentation = message.presentation {
                MacAssistantPresentationView(
                    nodes: presentation.renderedTextNodes,
                    // Transport can finish before queued semantic
                    // phrases have finished arriving at the view.
                    animateNewNodes: !presentation.isComplete,
                    contentIsFinal: presentation.isComplete,
                    mutableNodeID: presentation.mutableTextNode?.id
                )
                .frame(maxWidth: .infinity, alignment: .leading)
            } else if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                // Noncanonical optimistic/model-test messages
                // still render their explicit plain text. An existing
                // presentation owns reveal timing, including an
                // empty committed prefix while prose is buffered.
                MacMarkdownView(markdown: message.text, isStreaming: isStreaming, animatesAppends: isStreaming)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            let sources = (message.presentation?.renderedSourceReferences ?? []).filter { source in
                guard let identity = message.journalReference?.cursor.identity else { return true }
                return !journalReferenceCoverage.contains(identity: identity,
                    sourceID: source.sourceID, citationID: source.citationID)
            }
            if message.canonicalPresentation == nil && message.journalReference == nil && !sources.isEmpty {
                MacAssistantSourcesView(sources: sources)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if message.canonicalPresentation == nil, let references = message.presentation?.renderedCompactionReferences,
               !references.isEmpty {
                MacAssistantCompactionReferencesView(
                    conversationID: conversationID,
                    messageID: message.id,
                    references: references
                )
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 2)
        .padding(.vertical, 3)
        .accessibilityIdentifier("mac-assistant-response-bubble")
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var uncoveredTraceItems: [MacAssistantTrace] {
        (message.presentation?.traceItems ?? []).filter { trace in
            guard let identity = message.journalReference?.cursor.identity,
                  let partID = trace.externalID else { return true }
            return !journalActivityCoverage.contains(identity: identity, partID: partID)
        }
    }
    private var uncoveredTaskHandles: [MacCoreTaskHandle] {
        (message.presentation?.renderedTaskHandles ?? []).filter { task in
            guard let identity = message.journalReference?.cursor.identity else { return true }
            return !journalActivityCoverage.containsTask(identity: identity, taskID: task.id)
        }
    }
    private var accessibleAuthor: String {
        switch message.role {
        case .user: userDisplayName ?? String(localized: "You")
        case .assistant: "Nola"
        case .system: String(localized: "System")
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            if message.role == .user {
                Spacer(minLength: min(96, max(48, transcriptWidth * 0.14)))
            } else if showsRoleAvatar && message.role != .assistant {
                Group {
                    if showsInlinePendingIndicator {
                        MacBreathingStatusLight(diameter: 18)
                    } else {
                        Image(systemName: message.role == .assistant ? "sparkles" : "info.circle")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(MacDesignColor.brand.opacity(0.82))
                    }
                }
                .frame(width: 24, height: 28)
            }

            VStack(alignment: message.role == .user ? .trailing : .leading, spacing: 7) {
                if message.role == .assistant {
                    VStack(alignment: .leading, spacing: 10) {
                        if showsRoleAvatar && assistantBubbleFill == nil { AppleConversationSignature() }
                        // Retire only exact Parts successfully adopted by the
                        // committed reader, not all traces of a broad kind.
                        let traces = uncoveredTraceItems
                        let tasks = uncoveredTaskHandles
                        if isStreaming, !hasAssistantResponse, traces.isEmpty {
                            MacFlowingStatusText(text: Text("Nola is processing"))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .frame(height: 28, alignment: .center)
                                .accessibilityIdentifier("chat.assistant.processing")
                        }
                        if message.journalReference == nil && message.canonicalPresentation == nil && !tasks.isEmpty {
                            MacAssistantTaskList(
                                tasks: tasks,
                                guidance: message.presentation?.renderedTaskGuidance ?? [:],
                                onSendGuidance: onSendTaskGuidance,
                                onRequestCancellation: onRequestTaskCancellation
                            )
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if message.journalReference == nil && message.canonicalPresentation == nil && !traces.isEmpty {
                            MacAssistantTraceView(
                                traces: traces,
                                isStreaming: isStreaming
                            )
                            .frame(maxWidth: .infinity, alignment: .leading)
                        } else if message.journalReference == nil && message.canonicalPresentation == nil && (isStreaming || !liveActivities.isEmpty)
                                    && !showsInlinePendingIndicator {
                            MacAssistantPendingProcessView(
                                activities: liveActivities,
                                showsAnimatedStatusLight: showsAnimatedStatusLight
                            )
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if hasAssistantResponse {
                            assistantResponseBody
                        }
                        if MacAssistantFailurePresentationPolicy.hasPartialToolResults(
                            deliveryState: message.deliveryState,
                            traces: message.presentation?.traceItems ?? []
                        ) {
                            Text("The reply did not finish. Completed tool results remain available in the activity details.", tableName: "Chat")
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .accessibilityIdentifier("chat.partialToolResults")
                        }
                        if showsAssistantFailureFallback, let assistantFailureTitle {
                            HStack(alignment: .top, spacing: 8) {
                                Image(systemName: "exclamationmark.circle.fill")
                                    .foregroundStyle(assistantFailureColor)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(assistantFailureTitle)
                                        .font(.system(size: 12.5, weight: .semibold))
                                    if let assistantFailureDetail, !assistantFailureDetail.isEmpty {
                                        Text(assistantFailureDetail)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                            .textSelection(.enabled)
                                    }
                                }
                            }
                                .padding(.horizontal, 12)
                                .padding(.vertical, 9)
                                .background(
                                    assistantFailureColor.opacity(0.08),
                                    in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                                )
                                .overlay {
                                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                                        .stroke(
                                            assistantFailureColor.opacity(0.2),
                                            lineWidth: 0.5
                                        )
                                }
                                .accessibilityIdentifier("mac-assistant-failure")
                        }
                    }
                    // The journal/text reader uses an infinite maximum; give expert
                    // prose a measured proposal so a short reply remains a short bubble.
                    .frame(width: assistantBubbleFill == nil ? nil : MacExpertBubbleLayout.contentWidth(
                        text: message.text, maximumWidth: transcriptWidth,
                        requiresFullWidth: !message.uiSurfaceParts.isEmpty || !message.uiHistoryReferences.isEmpty
                            || !(message.presentation?.traceItems.isEmpty ?? true)
                            || !(message.presentation?.renderedTaskHandles.isEmpty ?? true)
                    ), alignment: .leading)
                    // Expert tint belongs to the response, never the identity or actions.
                    .padding(assistantBubbleFill == nil ? 0 : 18)
                    .background(assistantBubbleFill ?? .clear,
                        in: UnevenRoundedRectangle(topLeadingRadius: 5, bottomLeadingRadius: 16,
                            bottomTrailingRadius: 16, topTrailingRadius: 16))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(String(localized: "Response"))
                    .accessibilityValue(message.canonicalPresentation == nil && message.journalReference == nil ? message.text : "")
                } else {
                    if !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Text(message.text)
                            .font(.system(size: MacMarkdownTypography.bodySize))
                            .textSelection(.enabled)
                            .lineSpacing(MacMarkdownTypography.lineSpacing)
                            .padding(.horizontal, message.role == .user ? 16 : 0)
                            .padding(.vertical, message.role == .user ? 12 : 0)
                            .background {
                                if message.role == .user {
                                    UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16,
                                        bottomTrailingRadius: 16, topTrailingRadius: 5)
                                        .fill(MacDesignColor.surfaceRaised)
                                }
                            }
                            .frame(
                                maxWidth: message.role == .user
                                    ? MacChatBubbleLayout.userBubbleWidth(transcriptWidth: transcriptWidth)
                                    : .infinity,
                                alignment: message.role == .user ? .trailing : .leading
                            )
                    }
                }

                if message.canonicalPresentation == nil, let details = message.historicalDetails, !details.isEmpty {
                    MacHistoricalPartDetailsButton(selections: details)
                }

                if let deferred = message.deferredContent {
                    MacDeferredTextPreview(content: deferred)
                }

                if (message.canonicalPresentation == nil || message.role != .assistant) && !uncoveredAttachments.isEmpty {
                    VStack(
                        alignment: message.role == .user ? .trailing : .leading,
                        spacing: 6
                    ) {
                        ForEach(uncoveredAttachments, id: \.offset) { index, name in
                            Button {
                                guard index < message.attachmentURLs.count else { return }
                                openAttachments([message.attachmentURLs[index]])
                            } label: {
                                HStack(spacing: 9) {
                                    Image(systemName: attachmentIconName(at: index))
                                        .font(.system(size: 14, weight: .medium))
                                        .foregroundStyle(MacDesignColor.brand)
                                        .frame(width: 22, height: 22)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(name)
                                            .font(.system(size: 12, weight: .semibold))
                                            .foregroundStyle(.primary)
                                            .lineLimit(1)
                                        Text(attachmentTypeLabel(at: index, fallbackName: name))
                                            .font(.system(size: 12, weight: .medium))
                                            .foregroundStyle(.secondary)
                                    }
                                    Spacer(minLength: 12)
                                }
                                .padding(.horizontal, 11)
                                .padding(.vertical, 8)
                                .frame(width: 280, alignment: .leading)
                                .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                                .overlay {
                                    RoundedRectangle(cornerRadius: 11, style: .continuous)
                                        .stroke(Color.primary.opacity(0.09), lineWidth: 0.5)
                                }
                            }
                            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                            .disabled(index >= message.attachmentURLs.count)
                            .help(String(
                                format: NSLocalizedString(
                                    "Open attachment %@",
                                    comment: "Open a chat attachment"
                                ),
                                name
                            ))
                        }
                    }
                    .frame(
                        maxWidth: .infinity,
                        alignment: message.role == .user ? .trailing : .leading
                    )
                }

                if message.role == .assistant || showsMessageMetadata {
                    HStack(spacing: 8) {
                        Text(message.createdAt, style: .time)
                            .font(.system(size: 12))
                            .foregroundStyle(Color("NolaTextSecondary"))

                        HStack(spacing: 5) {
                            Button {
                                copyMessage()
                            } label: {
                                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                                    .frame(width: 32, height: 32)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(MacMessageActionButtonStyle())
                            .help(copied ? String(localized: "Copied") : String(localized: "Copy"))
                            .accessibilityLabel(copied ? String(localized: "Copied") : String(localized: "Copy"))
                            if message.role == .user && !transcriptReadOnly {
                                Button { editMessage.open(conversationID, message) } label: {
                                    Image(systemName: "square.and.pencil").frame(width: 32, height: 32).contentShape(Rectangle())
                                }.buttonStyle(MacMessageActionButtonStyle()).disabled(!editMessage.enabled)
                                    .help(editMessage.enabled ? String(localized: "Edit and resend") : String(localized: "Wait for the current response or stop it before editing."))
                                    .accessibilityLabel(String(localized: "Edit and resend"))
                                    .accessibilityIdentifier("mac-message-edit-" + message.id.uuidString)
                            }

                        }
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        // Keep the completed-message action reachable while
                        // the pointer crosses the gap below the bubble. Hover
                        // raises emphasis; it no longer controls hit testing.
                        .opacity(1)
                    }
                    .frame(minHeight: 32)
                    .frame(maxWidth: .infinity, alignment: message.role == .assistant ? .leading : .trailing)
                    .accessibilityElement(children: .contain)
                    .opacity(showsMessageMetadata ? 1 : 0)
                    .allowsHitTesting(showsMessageMetadata)
                    .accessibilityHidden(!showsMessageMetadata)
                }
            }

            if message.role != .user {
                Spacer(minLength: 32)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(
            isSearchMatch
                ? Color.yellow.opacity(0.10)
                : Color.clear,
            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
        )
        .onHover { hovering in
            guard isHovering != hovering else { return }
            if reduceMotion {
                isHovering = hovering
            } else {
                withAnimation(.easeOut(duration: 0.14)) {
                    isHovering = hovering
                }
            }
        }
        .accessibilityElement(children: .contain)
        .onDisappear {
            copyTask?.cancel()
            copyFeedbackTask?.cancel()
            copied = false
        }
        .onChange(of: message.id) { _, _ in
            copyTask?.cancel()
            copyFeedbackTask?.cancel()
            copied = false
            copyError = nil
        }
        .alert("Unable to copy message", isPresented: Binding(
            get: { copyError != nil }, set: { if !$0 { copyError = nil } }
        )) {
            Button("OK", role: .cancel) { copyError = nil }
        } message: { Text(verbatim: copyError ?? "") }
        .accessibilityLabel(accessibleAuthor)
        .accessibilityValue(message.text)
        .accessibilityAction(named: "Copy") {
            if message.role != .assistant || assistantOutputIsComplete { copyMessage() }
        }
        .accessibilityAction(named: "Quick Look") {
            openAttachments(uncoveredLocalAttachmentURLs)
        }
        .contextMenu {
            Button("Select Text…") {
                selectableResponse = .init(reference: message.journalReference, fallback: message.text)
            }
            .disabled(message.role == .assistant && !assistantOutputIsComplete)
            Button("Copy") {
                copyMessage()
            }
            .disabled(message.role == .assistant && !assistantOutputIsComplete)
            Button("Copy as Markdown") {
                copyMessage(asMarkdown: true)
            }
            .disabled(message.role == .assistant && !assistantOutputIsComplete)
            if !uncoveredLocalAttachmentURLs.isEmpty {
                Divider()
                Button("Quick Look") {
                    openAttachments(uncoveredLocalAttachmentURLs)
                }
                Button("Reveal in Finder") {
                    openAttachments(uncoveredLocalAttachmentURLs, reveal: true)
                }
            }
        }
        .alert("Unable to open attachment", isPresented: Binding(
            get: { attachmentOpenError != nil },
            set: { if !$0 { attachmentOpenError = nil } }
        )) {
            Button("OK", role: .cancel) { attachmentOpenError = nil }
        } message: {
            Text(verbatim: attachmentOpenError ?? "")
        }
        .sheet(item: $selectableResponse) { selection in
            MacMessageTextSelectionView(selection: selection, onCopy: copyText,
                onDone: { selectableResponse = nil })
        }
    }

    private func copyMessage(asMarkdown: Bool = false) {
        copyTask?.cancel()
        copyError = nil
        let reference = message.journalReference
        let fallback = message.text
        copyTask = Task { @MainActor in
            do {
                let text: String
                if let reference {
                    text = try await MacJournalTextRead.markdown(reference, historicalFallback: fallback)
                } else { text = fallback }
                try Task.checkCancellation()
                copyText(asMarkdown || message.role != .assistant ? text : MacMarkdownTextParser.plainText(text))
            } catch {
                if !Task.isCancelled { copyError = error.localizedDescription }
            }
        }
    }

    private func copyText(_ text: String) {
        NSPasteboard.general.clearContents()
        guard NSPasteboard.general.setString(text, forType: .string) else {
            Logger.shared.log(
                "Pasteboard rejected message copy",
                level: .warn,
                category: "MacChatClipboard",
                metadata: ["message": message.id.uuidString]
            )
            selectableResponse = .init(reference: nil, fallback: text)
            return
        }
        copyFeedbackTask?.cancel()
        copied = true
        Logger.shared.log(
            "Message copied",
            level: .debug,
            category: "MacChatClipboard",
            metadata: ["message": message.id.uuidString, "chars": String(text.count)]
        )
        copyFeedbackTask = Task { @MainActor in
            do { try await Task.sleep(for: .seconds(1.8)) }
            catch { return }
            copied = false
        }
    }

    private func openAttachments(_ urls: [URL], reveal: Bool = false) {
        guard !urls.isEmpty else { return }
        let reference = message.journalReference
        Task { @MainActor in
            do {
                let local: Bool
                if urls.contains(where: \.isFileURL), let reference {
                    local = try await MacJournalTextRead.isLocal(reference)
                } else if urls.contains(where: \.isFileURL), message.canonicalMessageID != nil {
                    guard let messageSourceContext else {
                        throw MacApplicationPortError.rejected(String(localized: "The attachment source is not available."))
                    }
                    local = try messageSourceContext.readLocalSource()
                    guard local else {
                        throw MacApplicationPortError.rejected(String(localized: "The attachment belongs to another Host. Open it from its source."))
                    }
                } else {
                    local = reference == nil
                }
                try Task.checkCancellation()
                var files: [URL] = []
                for url in urls {
                    switch MacJournalArtifactDestination.resolve(url.absoluteString, localJournal: local) {
                    case .localFile(let file): files.append(file)
                    case .web(let address): if !reveal { openURL(address) }
                    case nil: throw MacRemoteHostError.invalidResponse
                    }
                }
                if !files.isEmpty {
                    if reveal { NSWorkspace.shared.activateFileViewerSelecting(files) }
                    else { MacQuickLookController.shared.preview(files) }
                }
            } catch {
                if !Task.isCancelled { attachmentOpenError = error.localizedDescription }
            }
        }
    }

    private var uncoveredLocalAttachmentURLs: [URL] {
        uncoveredAttachments.compactMap { entry in
            guard entry.offset < message.attachmentURLs.count else { return nil }
            let url = message.attachmentURLs[entry.offset]
            return url.isFileURL ? url : nil
        }
    }

    private var uncoveredAttachments: [(offset: Int, element: String)] {
        Array(message.attachmentNames.enumerated()).filter { entry in
            guard let identity = message.journalReference?.cursor.identity,
                  let ids = message.attachmentPartIDs, entry.offset < ids.count else { return true }
            return !journalReferenceCoverage.containsAttachment(identity: identity, attachmentID: ids[entry.offset])
        }
    }

    private func attachmentIconName(at index: Int) -> String {
        guard index < message.attachmentURLs.count else { return "doc.text" }
        switch message.attachmentURLs[index].pathExtension.lowercased() {
        case "png", "jpg", "jpeg", "gif", "heic", "webp":
            return "photo"
        case "pdf":
            return "doc.richtext"
        case "doc", "docx":
            return "doc.text"
        case "xls", "xlsx", "csv":
            return "tablecells"
        case "ppt", "pptx":
            return "rectangle.on.rectangle"
        default:
            return "doc"
        }
    }

    private func attachmentTypeLabel(at index: Int, fallbackName: String) -> String {
        let name = index < message.attachmentURLs.count
            ? message.attachmentURLs[index].lastPathComponent
            : fallbackName
        let fileExtension = URL(fileURLWithPath: name).pathExtension.uppercased()
        let fileType = fileExtension.isEmpty ? String(localized: "File") : fileExtension
        let metadata = index < message.attachmentURLs.count
            ? message.attachmentMetadata?.first { $0.uri == message.attachmentURLs[index].absoluteString }
            : nil
        return [fileType, metadata?.mimeType, metadata?.operation].compactMap { $0 }.joined(separator: " · ")
    }
}

private struct MacMessageTextSelection: Identifiable {
    let id = UUID()
    let reference: MacJournalMessageReference?
    let fallback: String
}

@MainActor
private struct MacMessageTextSelectionView: View {
    let selection: MacMessageTextSelection
    let onCopy: (String) -> Void
    let onDone: () -> Void
    @State private var text: String?
    @State private var error: String?
    @State private var retry = 0

    var body: some View {
        VStack(alignment: .trailing, spacing: 12) {
            Group {
                if let text {
                    MacSelectableResponseText(text: text)
                } else if let error {
                    VStack(spacing: 12) {
                        Text(verbatim: error).textSelection(.enabled)
                        Button("Retry") { retry += 1 }
                    }
                } else {
                    ProgressView()
                }
            }.frame(minWidth: 560, minHeight: 380)
            HStack {
                Button("Copy") { if let text { onCopy(text) } }.disabled(text == nil)
                Spacer()
                Button("Done", action: onDone).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .task(id: retry) {
            text = nil
            error = nil
            do {
                let value: String
                if let reference = selection.reference {
                    value = try await MacJournalTextRead.markdown(reference, historicalFallback: selection.fallback)
                } else { value = selection.fallback }
                try Task.checkCancellation()
                text = value
            } catch {
                if !Task.isCancelled { self.error = error.localizedDescription }
            }
        }
    }
}

private struct MacAssistantTaskList: View {
    let tasks: [MacCoreTaskHandle]
    let guidance: [String: MacCoreTaskGuidanceQueue]
    let onSendGuidance: (String, String) -> Void
    let onRequestCancellation: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(tasks) { task in
                MacAssistantTaskCard(
                    task: task,
                    guidance: guidance[task.id],
                    onSendGuidance: { onSendGuidance(task.id, $0) },
                    onRequestCancellation: { onRequestCancellation(task.id) }
                )
            }
        }
    }
}

private struct MacAssistantTaskCard: View {
    let task: MacCoreTaskHandle
    let guidance: MacCoreTaskGuidanceQueue?
    let onSendGuidance: (String) -> Void
    let onRequestCancellation: () -> Void
    @Environment(\.macTranscriptReadOnly) private var readOnly
    @State private var isExpanded = false
    @State private var guidanceDraft = ""
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var model: MacCoreTaskUIModel { .init(task: task) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 10) {
                    taskIcon
                    VStack(alignment: .leading, spacing: 3) {
                        Text(title)
                            .font(.system(size: 12.5, weight: .semibold))
                            .foregroundStyle(.primary)
                        if !model.detail.isEmpty {
                            Text(model.detail)
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .transaction { $0.animation = nil }
                        }
                    }
                    Spacer(minLength: 8)
                    if let progress = model.progress, model.isActive {
                        Text(progress.formatted(.percent.precision(.fractionLength(0))))
                            .font(.system(size: 12, weight: .medium, design: .rounded))
                            .foregroundStyle(.secondary)
                    }
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isExpanded ? 180 : 0))
                }
                .contentShape(Rectangle())
                .padding(12)
            }
            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))

            if let progress = model.progress, model.isActive {
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
                    .tint(.accentColor)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 10)
            }

            if isExpanded {
                VStack(alignment: .leading, spacing: 9) {
                    taskDetail(String(localized: "State"), stateLabel)
                    ForEach(Array(model.outcomeDetails.enumerated()), id: \.offset) { _, detail in
                        Text(detail)
                            .font(.body)
                            .foregroundStyle(.primary)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                    taskDetail(String(localized: "Task ID"), task.id)
                    if let parentTaskID = task.lineage?.parentTaskID, !parentTaskID.isEmpty {
                        taskDetail(String(localized: "Continued from"), parentTaskID)
                    }
                    if let result = task.resultRef, !result.isEmpty {
                        taskDetail(String(localized: "Result"), result)
                    }
                    if let lastGuidance = guidance?.pending.last {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(String(localized: "Guidance pending", table: "Chat"))
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(MacDesignColor.brand)
                            Text(lastGuidance.content)
                                .font(.system(size: 11))
                                .foregroundStyle(.secondary)
                                .lineLimit(3)
                                .textSelection(.enabled)
                        }
                    }
                    if !readOnly, model.isActive, task.kind == .agent {
                        HStack(spacing: 7) {
                            TextField(
                                String(localized: "Guide this task", table: "Chat"),
                                text: $guidanceDraft
                            )
                            .textFieldStyle(.plain)
                            .font(.system(size: 12))
                            .onSubmit(sendGuidance)

                            Button(action: sendGuidance) {
                                Image(systemName: "arrow.up.circle.fill")
                                    .font(.system(size: 16, weight: .semibold))
                                    .foregroundStyle(canSendGuidance ? MacDesignColor.brand : Color.secondary)
                            }
                            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                            .disabled(!canSendGuidance)
                            .help(String(localized: "Send guidance", table: "Chat"))
                        }
                        .padding(.horizontal, 9)
                        .frame(height: 30)
                        .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                    if !readOnly, model.isActive {
                        Button(role: .destructive, action: onRequestCancellation) {
                            Label(String(localized: "Cancel task", table: "Chat"), systemImage: "stop.circle")
                                .font(.system(size: 11, weight: .medium))
                        }
                        .buttonStyle(MacActionButtonStyle())
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.primary.opacity(0.07), lineWidth: 1)
        }
    }

    @ViewBuilder private var taskIcon: some View {
        if model.isActive {
            MacBreathingStatusLight(diameter: 20)
                .frame(width: 22, height: 22)
        } else {
            Image(systemName: terminalIcon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.secondary)
                .frame(width: 22, height: 22)
                .accessibilityLabel(stateLabel)
        }
    }

    private var title: String {
        switch task.kind {
        case .agent: return String(localized: "Background agent")
        case .tool: return String(localized: "Background tool")
        case .skill: return String(localized: "Background skill")
        case .dataRefresh: return String(localized: "Data refresh")
        }
    }

    private var stateLabel: String {
        if let outcome = model.outcomeLabel { return outcome }
        switch task.state {
        case .queued: return String(localized: "Queued")
        case .running: return String(localized: "Running")
        case .waiting: return String(localized: "Waiting")
        case .suspended: return String(localized: "Suspended")
        case .retryWaiting: return String(localized: "Waiting to retry")
        case .completed: return String(localized: "Completed")
        case .sealed: return String(localized: "Execution sealed")
        case .failed: return String(localized: "Failed")
        case .cancelled: return String(localized: "Cancelled")
        }
    }

    private var terminalIcon: String {
        if let result = model.terminalResult {
            switch result.value.kind {
            case .answered, .actionCompleted: return "checkmark.circle.fill"
            case .delegationAccepted: return "arrow.triangle.branch"
            case .incomplete: return "circle.lefthalf.filled"
            case .blocked: return "pause.circle.fill"
            case .cancelled: return "xmark.circle.fill"
            case .failed: return "exclamationmark.circle.fill"
            }
        }
        switch task.state {
        case .completed: return "checkmark.circle.fill"
        case .cancelled: return "xmark.circle.fill"
        case .failed: return "exclamationmark.circle.fill"
        default: return "archivebox"
        }
    }

    private func taskDetail(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label).foregroundStyle(Color("NolaTextSecondary")).frame(width: 58, alignment: .leading)
            Text(value).foregroundStyle(.secondary).textSelection(.enabled)
        }
        .font(.system(size: 12))
    }

    private var canSendGuidance: Bool {
        !guidanceDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func sendGuidance() {
        let message = guidanceDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else { return }
        guidanceDraft = ""
        onSendGuidance(message)
    }
}

private struct MacAssistantCompactionReferencesView: View {
    let conversationID: String
    let messageID: UUID
    let references: [MacAssistantCompactionReference]

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            ForEach(references) { reference in
                VStack(alignment: .leading, spacing: 9) {
                    HStack(alignment: .center, spacing: 9) {
                        Image(systemName: "arrow.triangle.2.circlepath")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(MacDesignColor.brand)
                            .frame(width: 24, height: 24)
                            .background(MacDesignColor.brand.opacity(0.11), in: Circle())
                        VStack(alignment: .leading, spacing: 1) {
                            Text(String(localized: "Context compressed automatically", table: "Chat"))
                                .font(.system(size: 12, weight: .semibold))
                            Text(
                                String(
                                    format: String(localized: "%lld earlier items were summarized so the conversation could continue.", table: "Chat"),
                                    Int64(reference.sourceItemIDs.count)
                                )
                            )
                            .font(.system(size: 12))
                            .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 8)
                        Text(String(localized: "Checkpoint", table: "Chat"))
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(MacDesignColor.brand)
                            .padding(.horizontal, 7)
                            .frame(height: 20)
                            .background(MacDesignColor.brand.opacity(0.09), in: Capsule())
                    }

                    Text(reference.summary)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(3)

                    HStack(spacing: 8) {
                        Button {
                            NotificationCenter.default.post(
                                name: .macCompactionSourceRequested,
                                object: MacCompactionSourceRequest(
                                    conversationID: conversationID,
                                    messageID: messageID,
                                    compactionPartID: reference.id,
                                    sourceItemIDs: reference.sourceItemIDs,
                                    sourceFingerprint: reference.sourceFingerprint
                                )
                            )
                        } label: {
                            Label(
                                String(localized: "Review original context", table: "Chat"),
                                systemImage: "clock.arrow.circlepath"
                            )
                        }
                        .buttonStyle(.link)
                        .disabled(reference.sourceItemIDs.isEmpty || reference.sourceFingerprint.isEmpty)

                        Spacer()
                        Text("SHA-256 · \(reference.sourceFingerprint.prefix(12))")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(Color("NolaTextSecondary"))
                    }
                }
                .padding(11)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(MacDesignColor.brand.opacity(0.045), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 11, style: .continuous)
                        .stroke(MacDesignColor.brand.opacity(0.14), lineWidth: 0.5)
                }
                .accessibilityIdentifier("mac-compaction-sources-\(reference.id)")
            }
        }
    }
}

private struct MacAssistantSourcesView: View {
    let sources: [MacAssistantSourceReference]
    @State private var expanded = false
    @Environment(\.openURL) private var openURL
    @Environment(\.openWindow) private var openWindow
    @Environment(\.macOpenFileReference) private var openFileReference

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(
                sources.allSatisfy(\.isMemoryReference)
                    ? String(localized: "Context")
                    : String(localized: "Sources", table: "Chat")
            )
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
            ForEach(expanded ? sources : Array(sources.prefix(3))) { source in
                Button { open(source) } label: {
                    HStack(alignment: .top, spacing: 7) {
                        Image(systemName: source.isMemoryReference ? "brain.head.profile" : (source.domain == nil ? "doc.text" : "globe"))
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(MacDesignColor.brand)
                            .frame(width: 14)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 7) {
                                Text(source.displayTitle)
                                    .font(.system(size: 12, weight: .medium))
                                    .foregroundStyle(.primary)
                                    .lineLimit(1)
                                if let domain = source.domain {
                                    Text(domain)
                                        .font(.system(size: 12))
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                                if let location = locationText(source) {
                                    Text(location)
                                        .font(.system(size: 12, design: .monospaced))
                                        .foregroundStyle(Color("NolaTextSecondary"))
                                        .lineLimit(1)
                                }
                            }
                            if let metadata = source.memoryUsage?.displayMetadata, !metadata.isEmpty {
                                Text(metadata.joined(separator: " · "))
                                    .font(.system(size: 12))
                                    .foregroundStyle(Color("NolaTextSecondary"))
                                    .lineLimit(2)
                            }
                        }
                        Spacer(minLength: 0)
                        if source.destination != nil {
                            Image(systemName: "arrow.up.right")
                                .font(.system(size: 8.5, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(MacActionButtonStyle())
                .disabled(source.destination == nil)
                .help(source.uri)
            }
            if sources.count > 3 {
                Button { expanded.toggle() } label: {
                    Text(verbatim: expanded ? "−" : "…").frame(minWidth: 28, minHeight: 28)
                }
                .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                .accessibilityLabel(Text("Sources", tableName: "Chat"))
                .help(String(localized: "Sources", table: "Chat"))
                .accessibilityValue(Text(expanded ? "Expanded" : "Collapsed", tableName: "Chat"))
            }
        }
        .padding(.top, 2)
    }

    private func open(_ source: MacAssistantSourceReference) {
        switch source.destination {
        case .web(let url):
            openURL(url)
        case .file(let url, let line):
            openFileReference(MacCodeReference(url: url, line: line, column: nil))
        case .memory(_):
            openWindow(id: "memory-center")
        case nil:
            break
        }
    }

    private func locationText(_ source: MacAssistantSourceReference) -> String? {
        var parts: [String] = []
        if let page = source.page { parts.append("p.\(page)") }
        if let start = source.lineStart {
            if let end = source.lineEnd, end != start {
                parts.append("L\(start)–\(end)")
            } else {
                parts.append("L\(start)")
            }
        }
        if let section = source.section?.trimmingCharacters(in: .whitespacesAndNewlines),
           !section.isEmpty {
            parts.append("§ \(section)")
        }
        if let start = source.rangeStart, let end = source.rangeEnd { parts.append("\(start)–\(end) B") }
        if let mimeType = source.mimeType, !mimeType.isEmpty { parts.append(mimeType) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

private struct MacAssistantTraceView: View {
    let traces: [MacAssistantTrace]
    let isStreaming: Bool

    private var hasVisibleContent: Bool {
        if isStreaming { return true }
        return !visibleTraces.isEmpty
    }

    private var visibleTraces: [MacAssistantTrace] {
        MacAssistantArtifactPresentationPolicy.visibleProcessTraces(traces)
    }

    var body: some View {
        Group {
            if hasVisibleContent {
                Group {
                    if isStreaming, visibleTraces.isEmpty {
                        MacBreathingStatusLight(diameter: 16)
                            .frame(width: 22, height: 22, alignment: .leading)
                            .padding(.vertical, 3)
                            .accessibilityLabel(String(localized: "Nola is processing"))
                    } else {
                        MacThinkingProcessRow(
                            traces: visibleTraces,
                            isStreaming: isStreaming,
                            initiallyExpanded: false
                        )
                        .transaction { transaction in
                            // Trace replacements must never inherit transcript
                            // animation. Disclosure and the breathing light own
                            // their animations independently.
                            if !transaction.disablesAnimations {
                                transaction.animation = nil
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

}

/// The non-journal canonical path preserves the same typed Part order as the
/// journal path. It never reconstructs order from aggregated text or traces.
private struct MacCanonicalPartFlowView: View {
    let conversationID: String
    let canonical: MacCanonicalConversationProjectionV1.Message
    let historicalDetails: [MacHistoricalPartSelection]
    let isStreaming: Bool
    let guidance: [String: MacCoreTaskGuidanceQueue]
    let onSendGuidance: (String, String) -> Void
    let onRequestCancellation: (String) -> Void
    let onOpenAttachment: (URL) -> Void
    @Environment(\.uiSurfaceContext) private var uiContext
    @State private var presentationOwner = MacJournalPresentationOwner()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(MacCanonicalPartFlow.segments(canonical.parts)) { segment in
                if segment.kind == .process {
                    let states = MacCanonicalPartFlow.states(segment.parts)
                    MacJournalProcessView(partIDs: segment.parts.map(\.partID),
                        delivery: MacCanonicalPartFlow.delivery(states),
                        requiresAttention: states.contains(.waitingForPermission) || states.contains(.waitingForInput),
                        toolCount: segment.parts.filter { if case .tool = $0 { true } else { false } }.count,
                        attentionState: states.contains(.waitingForPermission) ? "waiting_for_permission"
                            : states.contains(.waitingForInput) ? "waiting_for_input" : nil,
                        reasoningOnly: !segment.parts.contains { if case .tool = $0 { true } else { false } },
                        currentActivityTitle: isStreaming ? MacCanonicalPartFlow.currentActivityTitle(segment.parts) : nil) {
                            ForEach(segment.parts, id: \.partID) { part in partView(part) }
                        }
                } else {
                    ForEach(segment.parts, id: \.partID) { part in partView(part) }
                }
            }
        }
        .environment(\.journalPresentationOwner, presentationOwner)
        .modifier(MacMessageReaderPresenter(owner: presentationOwner))
        .onDisappear { presentationOwner.cancel() }
    }

    private func presentation(_ part: MacCanonicalConversationProjectionV1.Part) -> MacChatMessage? {
        try? MacCanonicalConversationRepository.projectMessage(.init(messageID: canonical.messageID,
            role: canonical.role, state: canonical.state, revision: canonical.revision,
            createdAtMs: canonical.createdAtMs, parts: [part]))
    }

    @ViewBuilder private func partView(_ part: MacCanonicalConversationProjectionV1.Part) -> some View {
        switch part {
        case .text(let value):
            if value.isDisclosed {
                MacMarkdownView(markdown: value.markdown, isStreaming: isStreaming && !value.isFinal, animatesAppends: false)
                    .accessibilityIdentifier("chat.canonical.text." + value.partID)
            }
        case .reasoning(let value):
            if let summary = value.summary, !summary.isEmpty {
                Text(verbatim: summary).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                    .accessibilityIdentifier("chat.canonical.reasoning." + value.partID)
            }
        case .uiSurface(let value):
            if value.surface.identity.owner.conversation_id == conversationID,
               value.surface.identity.owner.message_id == canonical.messageID, uiContext != nil {
                MacMessageReferencedSurface(reference: value.surface, summary: value.summary).id(value.surface.identity)
            }
        case .uiAction(let value):
            Text(verbatim: value.summary).textSelection(.enabled)
        case .historical(let value):
            if let surface = value.projection.uiSurfaceReference,
               surface.identity.owner.conversation_id == conversationID,
               surface.identity.owner.message_id == canonical.messageID, let uiContext {
                AppleUIHistoryView<MacCanonicalJSONValue>(reference: surface, load: uiContext.readRevision).id(surface.identity)
            } else if let text = value.projection.visibleText {
                MacMarkdownView(markdown: text, isStreaming: false, animatesAppends: false)
            } else {
                if let projected = presentation(part) { traceViews(projected) }
                let selections = historicalDetails.filter {
                    $0.owner.conversationID == conversationID && $0.owner.messageID == canonical.messageID
                        && $0.owner.partID == value.partID && $0.canonicalRevision == value.projection.canonicalRevision
                }
                if !selections.isEmpty { MacHistoricalPartDetailsButton(selections: selections) }
            }
        case .attachment(let value):
            Button {
                if let url = URL(string: value.uri) { onOpenAttachment(url) }
            } label: {
                Label(value.name, systemImage: "doc")
                    .font(.body).frame(maxWidth: .infinity, alignment: .leading)
            }.buttonStyle(MacActionButtonStyle())
                .help(value.uri)
        default:
            if let projected = presentation(part) {
                if let tasks = projected.presentation?.renderedTaskHandles, !tasks.isEmpty {
                    MacAssistantTaskList(tasks: tasks, guidance: guidance,
                        onSendGuidance: onSendGuidance, onRequestCancellation: onRequestCancellation)
                }
                if let sources = projected.presentation?.renderedSourceReferences, !sources.isEmpty {
                    MacAssistantSourcesView(sources: sources)
                }
                if let references = projected.presentation?.renderedCompactionReferences, !references.isEmpty {
                    MacAssistantCompactionReferencesView(conversationID: conversationID,
                        messageID: projected.id, references: references)
                }
                traceViews(projected)
            } else {
                Text("history.details.failure", tableName: "Chat").foregroundStyle(.secondary)
            }
        }
    }
    @ViewBuilder private func traceViews(_ projected: MacChatMessage) -> some View {
        ForEach(projected.presentation?.traceItems ?? []) { trace in
            if trace.kind == .thinking {
                Text(verbatim: trace.detail).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
            } else {
                MacThinkingPhaseRow(trace: trace, summary: trace.title)
            }
        }
    }
}

nonisolated enum MacCanonicalPartFlow {
    static func segments(_ parts: [MacCanonicalConversationProjectionV1.Part]) -> [AppleMessagePartFlowSegment<MacCanonicalConversationProjectionV1.Part>] {
        AppleMessagePartFlow.segments(parts, id: \.partID) { part in
            switch part { case .reasoning: .reasoning; case .tool: .tool; default: .content }
        }
    }
    static func states(_ parts: [MacCanonicalConversationProjectionV1.Part]) -> [MacCanonicalConversationProjectionV1.ActivityState] {
        parts.compactMap { part in switch part { case .reasoning(let value): value.state; case .tool(let value): value.state; default: nil } }
    }
    static func currentActivityTitle(_ parts: [MacCanonicalConversationProjectionV1.Part]) -> String? {
        let current = parts.last { part in
            switch part {
            case .tool(let tool): [.running, .pending].contains(tool.state)
            case .reasoning(let reasoning): reasoning.state == .running
            default: false
            }
        }
        switch current {
        case .tool(let tool) where [.running, .pending].contains(tool.state):
            return tool.title.isEmpty ? String(localized: "Activity") : tool.title
        case .reasoning(let reasoning) where reasoning.state == .running:
            return String(localized: "Thinking")
        default: return nil
        }
    }
    static func delivery(_ states: [MacCanonicalConversationProjectionV1.ActivityState]) -> MacChatMessage.DeliveryState? {
        if states.contains(where: { ![.completed,.sealed,.failed,.cancelled].contains($0) }) { return .running }
        return nil
    }
}
