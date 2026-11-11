// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatBubble logic in the iOS UI layer.
// - Interfaces:
//   - ChatBubble
//   - ChatMarkdownDisplayNormalizer
//   - ChatMarkdownView
//   - ChatMarkdownInlineText
//   - ChatMarkdownCodeBlock
//   - ChatMarkdownTableData
//   - ChatMarkdownTableBlock
//   - ChatMarkdownBlock
// - APIs:
//   - none
//
import SwiftUI
import UIKit

/// Match the prototype's 17/28 reading rhythm while retaining Dynamic Type.
private enum IOSChatProseMetrics {
    static var lineSpacing: CGFloat {
        max(0, UIFontMetrics(forTextStyle: .body).scaledValue(for: 28)
            - UIFont.preferredFont(forTextStyle: .body).lineHeight)
    }
}
import Combine
import SwiftMath
import BeautifulMermaid
import QuickLook

extension Notification.Name {
    static let chatTaskResultRequested = Notification.Name("Nola.Chat.TaskResultRequested")
    static let chatCompactionSourceRequested = Notification.Name(
        "Nola.Chat.CompactionSourceRequested"
    )
}

struct IOSCompactionSourceRequest: Equatable, Sendable {
    let conversationID: UUID
    let compactionPartID: String
    let sourceItemIDs: [String]
    let sourceFingerprint: String
}

private struct IOSCompactionSourceLink: Identifiable {
    let id: String
    let summary: String
    let sourceItemIDs: [String]
    let sourceFingerprint: String
}

struct IOSModelAttemptDisclosure: Identifiable {
    let id: String
    let presentation: AppleModelAttemptPresentation
    let failure: String
    init(_ value: IOSConversationProjectionV1.ModelAttemptPart) {
        id = value.partID
        presentation = .init(state: value.state.rawValue, provider: value.provider, model: value.model,
            successorAttemptID: value.fallbackAttemptID, successorProvider: value.fallbackProvider,
            successorModel: value.fallbackModel)
        failure = value.failure.message
    }
}

private struct IOSMessageEditorSource: Identifiable {
    let conversationID: UUID?
    let message: ChatMessage
    let sourceRevision: Int64?
    var id: String {
        let owner = conversationID?.uuidString ?? "none"
        let messageID = message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString
        let revision = message.canonicalMessageRevision.map { String($0) } ?? "none"
        return "\(owner):\(messageID):\(revision)"
    }
}

private struct IOSExpertReplyWidthKey: EnvironmentKey {
    static let defaultValue: CGFloat? = nil
}
private struct IOSCompactExpertReplyKey: EnvironmentKey {
    static let defaultValue = false
}
extension EnvironmentValues {
    var iosExpertReplyAvailableWidth: CGFloat? {
        get { self[IOSExpertReplyWidthKey.self] }
        set { self[IOSExpertReplyWidthKey.self] = newValue }
    }
    var iosCompactExpertReply: Bool {
        get { self[IOSCompactExpertReplyKey.self] }
        set { self[IOSCompactExpertReplyKey.self] = newValue }
    }
}

struct ChatBubble: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.iosExpertReplyAvailableWidth) private var expertReplyAvailableWidth
    @Environment(\.iosCompactExpertReply) private var compactExpertReply
    let conversationID: UUID?
    let message: ChatMessage
    let isLatestAssistant: Bool
    let isStreaming: Bool
    let isRetrieving: Bool
    let pendingHostPermissionRequest: HostAgentPermissionRequest?
    let onHostPermissionDecision: (String, HostAgentPermissionDecision) -> Void
    var assistantBubbleFill: Color? = nil
    var ownerIsRunning: Bool = false
    var surfaceInteractionsEnabled = true
    var onEditAndResend: ((AppleMessageEditSubmission) async throws -> Void)? = nil
    @State private var messageEditorSource: IOSMessageEditorSource?
    @State private var showManualCopy = false
    @State private var showCopiedHint: Bool = false
    @State private var copyFeedbackTask: Task<Void, Never>?
    @State private var showAnswerContextDetails: Bool = false
    @State private var showPersonalIntelligence: Bool = false
    @State private var previewAttachment: ChatAttachmentPreviewItem?
    @State private var historicalDetail: IOSHistoricalPartRequest?
    @State private var liveSourceReferences: [ChatSourceReference] = []
    @State private var processReading: [IOSMessagePartReadingKey: Bool] = [:]
    @State private var toolReading: [IOSMessagePartReadingKey: Bool] = [:]
    @State private var toolTechnicalReading: [IOSMessagePartReadingKey: Bool] = [:]
    @State private var selectedToolReader: IOSMessageToolReaderSelection?
    @State private var toolReaderOwner: IOSMessagePartReadingKey?
    @State private var toolReaderActivation = UUID()
    @State private var toolReaderPartIDs: Set<String> = []

    private var expertHorizontalPadding: CGFloat { compactExpertReply ? 12 : 13 }
    private func expertTextWidth(_ text: String) -> CGFloat? {
        guard assistantBubbleFill != nil, let available = expertReplyAvailableWidth else { return nil }
        let maximum = max(24, available - expertHorizontalPadding * 2)
        // Keep rich/multiline content at the available width. A short paragraph
        // receives a bounded intrinsic proposal, so the tint hugs its text.
        guard text.utf8.count <= 1024, !text.contains("\n") else { return maximum }
        let blocks = ChatMarkdownBlockParser.parse(text)
        guard blocks.count == 1, let block = blocks.first, case .paragraph = block else { return maximum }
        let measured = (text as NSString).size(withAttributes: [.font: UIFont.preferredFont(forTextStyle: .body)]).width
        return min(maximum, max(32, ceil(measured * 1.12) + 4))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if message.role == .user {
                userMessage
            } else {
                HStack(alignment: .top, spacing: 7) {
                    if assistantBubbleFill == nil {
                        VStack(spacing: 4) {
                            Text(verbatim: "n").font(.system(size: 14, design: .serif)).italic()
                                .frame(width: 30, height: 30).background(DesignTokens.surfaceColor, in: Circle())
                            Text(verbatim: "Nola").font(.system(size: 10))
                        }.foregroundStyle(.secondary).frame(width: 32).accessibilityHidden(true)
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        assistantMessage
                        if message.canonicalParts.isEmpty && !displayedSourceReferences.isEmpty {
                            assistantSourceReferences.transition(.opacity)
                        }
                    }
                }

                .animation(
                    reduceMotion ? nil : DesignTokens.streamRevealAnimation,
                    value: displayedSourceReferences.map(\.id)
                )
            }

        }
        // Modal residency belongs to the message, outside every process/Part
        // identity. Regrouping a segment must not dismiss an open reader.
        .fullScreenCover(item: toolReaderPresentation) { selection in
            HostToolTextReader(snapshot: selection.snapshot, onClose: {
                if selectedToolReader?.id == selection.id { selectedToolReader = nil }
            })
        }
        .onChange(of: readingKey(""), initial: true) { _, owner in
            toolReaderOwner = owner
            toolReaderActivation = UUID()
            selectedToolReader = nil
        }
        .onChange(of: message.canonicalParts.map(\.partID), initial: true) { _, ids in
            toolReaderPartIDs = Set(ids)
            if let selectedToolReader, !toolReaderPartIDs.contains(selectedToolReader.key.part) {
                self.selectedToolReader = nil
            }
        }
        .sheet(item: $messageEditorSource) { source in
            IOSMessageEditSheet(conversationID: source.conversationID, message: source.message, sourceRevision: source.sourceRevision,
                ownerIsRunning: ownerIsRunning, submit: onEditAndResend)
        }
        .onChange(of: conversationID) { _, _ in messageEditorSource = nil }
        .sheet(isPresented: $showManualCopy) {
            NavigationStack {
                ScrollView { Text(copyableText).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(20) }
                    .navigationTitle(String(localized: "chat.message.copy"))
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button(L10n.close) { showManualCopy = false } } }
            }
        }
        .sheet(item: $historicalDetail) { request in
            IOSHistoricalPartDetail(request: request).id(request.id)
        }
        .task(id: historicalDetailIsValid) {
            guard !historicalDetailIsValid else { return }
            await Task.yield()
            guard !Task.isCancelled else { return }
            historicalDetail = nil
        }
        .sheet(isPresented: $showPersonalIntelligence) {
            NavigationStack {
                PersonalIntelligenceDashboardView(showsCloseButton: true)
            }
        }
        .sheet(item: $previewAttachment) { item in
            ChatAttachmentQuickLook(url: item.url)
        }
        .onDisappear {
            copyFeedbackTask?.cancel()
            showCopiedHint = false
        }
        .onChange(of: message.id) { _, _ in
            copyFeedbackTask?.cancel()
            showCopiedHint = false
        }
        .onReceive(NotificationCenter.default.publisher(for: .chatAgentSourceReferencesDidChange)) { notification in
            guard isLatestAssistant,
                  let references = notification.object as? [ChatSourceReference],
                  references != liveSourceReferences else { return }
            liveSourceReferences = references
        }
    }

    private var historicalDetailIsValid: Bool {
        historicalDetail.map { historicalRequests.contains($0) } ?? true
    }

    private var historicalRequests: [IOSHistoricalPartRequest] {
        message.canonicalParts.compactMap { part in
            guard case .historical(_, _, let projection) = part,
                  projection.needsHistoricalDetail else { return nil }
            return IOSHistoricalPartRequest(projection)
        }
    }

    private var displayedSourceReferences: [ChatSourceReference] {
        let persisted = ChatSourceReference.decodeFromConversationMeta(message.metadata)
        if message.canonicalEventSequence != nil || !message.sourceReferences.isEmpty { return message.sourceReferences }
        return isLatestAssistant && !liveSourceReferences.isEmpty ? liveSourceReferences : persisted
    }

    private var assistantSourceReferences: some View { sourceReferenceCards(displayedSourceReferences) }

    private func sourceReferenceCards(_ references: [ChatSourceReference]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(NSLocalizedString("chat.sources.title", value: "Sources", comment: "Assistant response sources title"))
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            ForEach(references) { reference in
                Button {
                    if let fileURL = reference.fileURL {
                        previewAttachment = ChatAttachmentPreviewItem(url: fileURL)
                    } else if let webURL = reference.webURL {
                        UIApplication.shared.open(webURL, options: [:])
                    }
                } label: {
                    HStack(alignment: .top, spacing: 9) {
                        Image(systemName: reference.fileURL == nil ? "safari" : "doc.text")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(DesignTokens.primaryColor)
                            .frame(width: 22, height: 22)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(reference.displayTitle)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            let details = [reference.displayDomain, reference.location?.displayText, reference.mimeType,
                                           reference.rangeStart.map { "\($0)–\(reference.rangeEnd ?? $0) B" }]
                                .compactMap { $0 }
                                .filter { !$0.isEmpty }
                            if !details.isEmpty {
                                Text(details.joined(separator: " · "))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                            if let memoryUsage = reference.memoryUsage,
                               !memoryUsage.displayMetadata.isEmpty {
                                Text(memoryUsage.displayMetadata.joined(separator: " · "))
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                    .lineLimit(2)
                            }
                        }
                        Spacer(minLength: 6)
                        Image(systemName: reference.canOpen ? "arrow.up.right.square" : "exclamationmark.circle")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .frame(maxWidth: 520, alignment: .leading)
                    .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 11, style: .continuous)
                            .stroke(Color.primary.opacity(0.08), lineWidth: 0.5)
                    }
                }
                .buttonStyle(NolaActionButtonStyle())
                .disabled(!reference.canOpen)
                .accessibilityLabel(String(
                    format: NSLocalizedString("chat.source.open_format", value: "Open source %@", comment: "Open an assistant response source"),
                    reference.displayTitle
                ))
            }
        }
        .padding(.leading, 2)
    }

    private var userMessage: some View {
        HStack {
            Spacer(minLength: 56)
            VStack(alignment: .trailing, spacing: 5) {
                if !message.includeInContext {
                    Text(NSLocalizedString("chat.bubble.archived_to_session", value: "已折叠到 session memory", comment: "Message archived to session memory"))
                        .font(.caption2.weight(.semibold))
                        .foregroundColor(DesignTokens.warningColor)
                }
                if !message.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(message.content)
                        .font(.body)
                        .lineSpacing(IOSChatProseMetrics.lineSpacing)
                        .foregroundColor(.primary)
                        .multilineTextAlignment(.leading)
                        .textSelection(.enabled)
                        .frame(maxWidth: 640, alignment: .leading)
                        .padding(.horizontal, 13)
                        .padding(.vertical, 9)
                        .background(
                            UnevenRoundedRectangle(topLeadingRadius: 14, bottomLeadingRadius: 14,
                                bottomTrailingRadius: 14, topTrailingRadius: 5)
                                .fill(DesignTokens.accentSubtleColor)
                        )
                        .opacity(message.includeInContext ? 1 : 0.82)
                        .contentShape(RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous))
                        .contextMenu {
                            copyButton
                        }
                }

                if !userAttachments.isEmpty {
                    VStack(alignment: .trailing, spacing: 6) {
                        ForEach(userAttachments) { attachment in
                            Button {
                                guard let url = attachment.url else { return }
                                previewAttachment = ChatAttachmentPreviewItem(url: url)
                            } label: {
                                HStack(spacing: 9) {
                                    Image(systemName: attachment.iconName)
                                        .font(.system(size: 15, weight: .medium))
                                        .foregroundStyle(DesignTokens.primaryColor)
                                        .frame(width: 24, height: 24)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(attachment.name)
                                            .font(.caption.weight(.semibold))
                                            .foregroundStyle(.primary)
                                            .lineLimit(1)
                                        if let detailLabel = attachment.detailLabel {
                                            Text(detailLabel)
                                                .font(.caption2)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                    Image(systemName: attachment.url == nil ? "exclamationmark.circle" : "arrow.up.right.square")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                .padding(.horizontal, 11)
                                .padding(.vertical, 8)
                                .frame(maxWidth: 360, alignment: .leading)
                                .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                .overlay {
                                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        .stroke(Color.primary.opacity(0.09), lineWidth: 0.5)
                                }
                            }
                            .buttonStyle(NolaActionButtonStyle())
                            .disabled(attachment.url == nil)
                            .accessibilityLabel(String(
                                format: NSLocalizedString("chat.attachment.open_format", value: "Open attachment %@", comment: "Open a chat attachment"),
                                attachment.name
                            ))
                        }
                    }
                }
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 4) {
                        copyButton.fixedSize(horizontal: true, vertical: false)
                        editMessageButton.fixedSize(horizontal: true, vertical: false)
                    }
                    HStack(spacing: 4) {
                        copyButton.labelStyle(.iconOnly)
                        editMessageButton.fixedSize(horizontal: true, vertical: false)
                    }
                    VStack(alignment: .trailing, spacing: 0) {
                        copyButton
                        editMessageButton
                    }
                }
                .font(.caption).buttonStyle(.borderless).controlSize(.regular)
                .frame(minHeight: 44, alignment: .trailing)
                if ownerIsRunning {
                    Text("chat.message.editWait").font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
        .overlay(alignment: .topTrailing) {
            copiedHint
        }
    }

    private var editMessageButton: some View {
        Button {
            let revision = conversationID.flatMap { try? container.applicationRuntime.loadConversation(id: $0.uuidString)?.revision }
            messageEditorSource = IOSMessageEditorSource(conversationID: conversationID, message: message, sourceRevision: revision)
        } label: {
            Label {
                Text(String(localized: "chat.message.edit"))
            } icon: {
                Image("nola-edit").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 14, height: 14)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Rectangle())
        }
        .disabled(ownerIsRunning)
        .accessibilityIdentifier("chat.message.edit.\(message.id.uuidString.lowercased())")
    }

    private var userAttachments: [ChatMessageAttachment] {
        ChatAttachmentMessagePayload.decode(from: message.metadata).enumerated().map { index, item in
            ChatMessageAttachment(
                id: "\(index):\(item.name)",
                name: item.name,
                url: item.url.flatMap(URL.init(string:)),
                sizeBytes: item.sizeBytes
            )
        }
    }

    private func readingKey(_ partID: String) -> IOSMessagePartReadingKey {
        .init(source: message.canonicalJournalCursor?.identity.source_store_epoch ?? container.applicationRuntime.databaseURL.path,
            conversation: conversationID?.uuidString ?? "", message: message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString,
            part: partID)
    }

    private func processExpansion(_ parts: [IOSConversationProjectionV1.Part], live: Bool) -> Binding<Bool> {
        let keys = parts.map { readingKey($0.partID) }
        return Binding(get: {
            ChatExecutionProcessPolicy.segmentExpanded(choices: keys.compactMap { processReading[$0] },
                liveReasoning: !ChatExecutionProcessPolicy.liveReasoning(parts, live: live).isEmpty)
        }, set: { value in for key in keys { processReading[key] = value } })
    }

    private func toolExpansion(_ partID: String, technical: Bool) -> Binding<Bool> {
        let key = readingKey(partID)
        return Binding(get: { technical ? toolTechnicalReading[key] ?? false : toolReading[key] ?? false }, set: { value in
            if technical { toolTechnicalReading[key] = value } else { toolReading[key] = value }
            if value { processReading[key] = true }
        })
    }

    private var toolReaderPresentation: Binding<IOSMessageToolReaderSelection?> {
        let presentedID = selectedToolReader?.id
        return Binding(get: {
            guard let selected = selectedToolReader, selected.key == readingKey(selected.key.part),
                  message.canonicalParts.contains(where: { $0.partID == selected.key.part }) else { return nil }
            return selected
        }, set: { value in
            // A delayed dismissal from an earlier cover cannot clear a newer
            // snapshot, even when the same message was left and reopened.
            if value == nil, selectedToolReader?.id == presentedID { selectedToolReader = nil }
        })
    }

    private func toolReader(_ partID: String) -> Binding<HostToolTextSnapshot?> {
        let key = readingKey(partID), owner = readingKey(""), activation = toolReaderActivation
        return Binding(get: { selectedToolReader?.key == key ? selectedToolReader?.snapshot : nil }, set: { value in
            guard toolReaderOwner == owner, toolReaderActivation == activation,
                  toolReaderPartIDs.contains(partID) else { return }
            if let value {
                selectedToolReader = IOSMessageToolReaderSelection(key: key, snapshot: value)
                processReading[key] = true
            } else if selectedToolReader?.key == key { selectedToolReader = nil }
        })
    }

    @ViewBuilder private func modelAttemptCard(_ attempt: IOSModelAttemptDisclosure) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: attempt.presentation.symbolName)
                .font(.caption.weight(.semibold))
                .foregroundStyle(attempt.presentation.isFailure ? Color.red : Color.secondary)
            VStack(alignment: .leading, spacing: 3) {
                if attempt.presentation.kind == .cancelled && ChatResponseStatusPolicy.terminalState(message.metadata) == .cancelled {
                    DisclosureGroup("chat.request.diagnostic_details") {
                        Text(attempt.presentation.modelName).textSelection(.enabled)
                        if !attempt.failure.isEmpty { Text(verbatim: attempt.failure).textSelection(.enabled) }
                    }.font(.caption2)
                } else {
                    Text(attempt.presentation.title()).font(.caption.weight(.semibold))
                }
                if !(attempt.presentation.kind == .cancelled && ChatResponseStatusPolicy.terminalState(message.metadata) == .cancelled) {
                    Text(attempt.presentation.modelName)
                        .font(.caption2).foregroundStyle(.secondary).textSelection(.enabled)
                }
                if attempt.presentation.kind != .cancelled && !attempt.failure.isEmpty {
                    if attempt.presentation.hasSuccessor {
                        DisclosureGroup("chat.request.diagnostic_details") {
                            Text(verbatim: attempt.failure).textSelection(.enabled)
                        }
                        .font(.caption2).foregroundStyle(.secondary)
                    } else {
                        Text(verbatim: attempt.failure).font(.caption2).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                if let nextModel = attempt.presentation.successorModelName {
                    Text(verbatim: nextModel).font(.caption2.weight(.medium)).foregroundStyle(.secondary)
                }
            }
        }
        .padding(9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background((attempt.presentation.isFailure ? Color.red : Color.secondary).opacity(0.06), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 10, style: .continuous).stroke((attempt.presentation.isFailure ? Color.red : Color.secondary).opacity(0.18), lineWidth: 0.5) }
    }

    @ViewBuilder private func compactionSource(_ link: IOSCompactionSourceLink) -> some View {
        Button {
            guard let conversationID else { return }
            NotificationCenter.default.post(
                name: .chatCompactionSourceRequested,
                object: IOSCompactionSourceRequest(
                    conversationID: conversationID,
                    compactionPartID: link.id,
                    sourceItemIDs: link.sourceItemIDs,
                    sourceFingerprint: link.sourceFingerprint
                )
            )
        } label: {
            VStack(alignment: .leading, spacing: 3) {
                Label(
                    String(
                        format: NSLocalizedString(
                            "chat.compaction.show_sources_format",
                            value: "Show original context (%d items)",
                            comment: "Jump from a compaction summary to its canonical source items"
                        ),
                        link.sourceItemIDs.count
                    ),
                    systemImage: "arrow.up.backward.and.arrow.down.forward"
                )
                Text(link.summary)
                    .lineLimit(2)
                Text("SHA-256 · \(link.sourceFingerprint.prefix(12))")
                    .font(.caption2.monospaced())
                    .foregroundStyle(.tertiary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(.bordered)
        .font(.caption)
        .accessibilityIdentifier("compaction-sources-" + link.id)
    }

    private func taskProjection(_ taskID: String?) -> AgentCoreTaskProjection? {
        guard let projection = message.taskProjection else { return nil }
        guard let taskID else { return projection }
        guard let task = projection.tasks[taskID] else { return nil }
        return .init(revision: projection.revision, order: [taskID], tasks: [taskID: task],
            guidance: (projection.guidance?[taskID]).map { [taskID: $0] })
    }

    @ViewBuilder private func taskPresentation(_ taskID: String?) -> some View {
        if let projection = taskProjection(taskID), !projection.tasks.isEmpty {
            let execution = conversationID.flatMap { container.chatManager.executionForConversation($0) }
            ChatCoreTaskProjectionView(
                projection: projection,
                parentConversationID: conversationID?.uuidString,
                showsSummary: taskID.map { ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: $0, parts: message.canonicalParts) } ?? true,
                controls: message.canonicalParts.compactMap {
                    if case .taskControl(let control) = $0 { return control }
                    return nil
                },
                onOpenResult: { destination in
                    NotificationCenter.default.post(name: .chatTaskResultRequested, object: destination)
                },
                onCancel: { taskID in
                    guard let execution, let task = projection.tasks[taskID],
                          execution.agentTaskProjection.tasks[taskID] == task else { return }
                    guard let control = message.canonicalParts.compactMap({ part -> IOSConversationProjectionV1.TaskControlPart? in
                        if case .taskControl(let value) = part { return value }; return nil
                    }).first(where: { $0.taskID == taskID && $0.supportsCancellation }),
                          let parentRunID = control.parentRunID else { return }
                    execution.cancelAgentTask(taskID, parentRunID: parentRunID)
                },
                onSendGuidance: { taskID, message in
                    guard let execution, let task = projection.tasks[taskID],
                          execution.agentTaskProjection.tasks[taskID] == task else { return }
                    guard let control = self.message.canonicalParts.compactMap({ part -> IOSConversationProjectionV1.TaskControlPart? in
                        if case .taskControl(let value) = part { return value }; return nil
                    }).first(where: { $0.taskID == taskID && $0.supportsGuidance }),
                          let parentRunID = control.parentRunID else { return }
                    execution.sendAgentTaskGuidance(taskID, parentRunID: parentRunID, message: message)
                }
            )
            .padding(.bottom, 4)
        }


    }

    @ViewBuilder private func canonicalContent(_ part: IOSConversationProjectionV1.Part) -> some View {
        switch part {
        case .text(let text):
            if text.isDisclosed, !text.markdown.isEmpty {
                ChatSmoothStreamingMarkdownView(content: text.markdown, isStreaming: isStreaming && !text.isFinal)
                    .textSelection(.enabled)
                    .frame(width: expertTextWidth(text.markdown), alignment: .leading)
                    .padding(.horizontal, expertHorizontalPadding)
                    .padding(.top, 10)
                    .padding(.bottom, 9)
                    .background {
                        UnevenRoundedRectangle(topLeadingRadius: 5, bottomLeadingRadius: 14,
                                bottomTrailingRadius: 14, topTrailingRadius: 14).fill(assistantBubbleFill ?? DesignTokens.surfaceColor)
                    }
                    .accessibilityIdentifier("chat.assistant.answer")
            }
        case .uiSurface:
            IOSCanonicalMessageSurface(conversationID: conversationID,
                messageID: message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString, part: part,
                allowsInteraction: surfaceInteractionsEnabled).id(part.partID)
        case .historical(_, _, let history):
            if history.uiSurfaceReference != nil {
                IOSCanonicalMessageSurface(conversationID: conversationID,
                    messageID: message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString, part: part,
                allowsInteraction: surfaceInteractionsEnabled).id(part.partID)
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    if let preview = history.timelineText, !preview.isEmpty {
                        Text(preview).font(.callout).textSelection(.enabled)
                    }
                    if history.needsHistoricalDetail, let request = IOSHistoricalPartRequest(history) {
                        Button { historicalDetail = request } label: {
                            Label(IOSHistoricalDetailPresentation.detailTitle(for: request.kind), systemImage: "doc.text.magnifyingglass")
                        }.font(.caption).accessibilityIdentifier("history-detail-" + request.owner.partID)
                    } else if let reason = history.status.reason, !reason.isEmpty {
                        Text(reason).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        case .modelAttempt(let value): modelAttemptCard(IOSModelAttemptDisclosure(value))
        case .journalDetail(let id, _, let kind, let title, let content):
            VStack(alignment: .leading, spacing: 4) {
                Text(title.isEmpty ? (kind == "historical" ? String(localized: "Historical details") : String(localized: "Progress")) : title)
                    .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                if !content.isEmpty { Text(content).font(.callout).textSelection(.enabled) }
            }.accessibilityIdentifier("journal-detail-" + id)
        case .source(let value):
            sourceReferenceCards([ChatSourceReference(id: value.sourceID, sourceID: value.sourceID,
                title: value.title, uri: value.uri, mimeType: value.mimeType, memoryUsage: value.evidence?.usage)])
        case .citation(let value):
            sourceReferenceCards(IOSConversationProjectionPresenter.sourceReferences(from: message.canonicalParts).filter { $0.citationID == value.citationID })
        case .taskControl(let value):
            if taskProjection(value.taskID) != nil { taskPresentation(value.taskID) }
            else { Label(String(localized: "chat.task.details_unavailable"), systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.secondary) }
        case .task(_, _, _, let title, let state, let progress, let step, let result, _):
            VStack(alignment: .leading, spacing: 4) {
                Label(ChatExecutionProcessPolicy.displayTitle(title, fallbackKey: "chat.part.task"), systemImage: "checklist")
                Text(ChatExecutionProcessPolicy.activityTitle(state)).font(.caption).foregroundStyle(.secondary)
                if !ChatExecutionProcessPolicy.isTerminal(state) {
                    if let step, !step.isEmpty { Text(step).font(.caption) }
                    if let progress { ProgressView(value: progress) }
                }
                if let result, !result.isEmpty {
                    DisclosureGroup("chat.request.diagnostic_details") {
                        LabeledContent("chat.part.diagnostic.result_reference", value: result)
                    }.font(.caption2).textSelection(.enabled)
                }
            }
        case .skill(_, _, _, _, let name, let mode, let state, let turns, let tools, let millis, let result, let failure):
            VStack(alignment: .leading, spacing: 4) {
                Label(name, systemImage: "wand.and.stars")
                Text(ChatExecutionProcessPolicy.activityTitle(state)).font(.caption).foregroundStyle(.secondary)
                DisclosureGroup("chat.request.diagnostic_details") {
                    LabeledContent("chat.part.diagnostic.mode", value: mode)
                    LabeledContent("chat.part.diagnostic.model_turns", value: String(turns))
                    LabeledContent("chat.part.diagnostic.tool_calls", value: String(tools))
                    LabeledContent("chat.part.diagnostic.elapsed_ms", value: String(millis))
                    if let result, !result.isEmpty { LabeledContent("chat.part.diagnostic.result_reference", value: result) }
                    if let failure, !failure.isEmpty { LabeledContent("chat.part.diagnostic.failure", value: failure) }
                }.font(.caption2).textSelection(.enabled)
            }
        case .error(_, _, let code, let displayKey, let recoverable):
            Label(ChatExecutionProcessPolicy.errorTitle(displayKey: displayKey), systemImage: "exclamationmark.circle")
                .foregroundStyle(DesignTokens.dangerColor)
            DisclosureGroup("chat.request.diagnostic_details") {
                LabeledContent("chat.part.diagnostic.code", value: code)
                LabeledContent("chat.part.diagnostic.display_key", value: displayKey)
                LabeledContent("chat.part.diagnostic.recoverable", value: recoverable ? L10n.yes : L10n.no)
            }.font(.caption2).textSelection(.enabled)
        case .compaction(let id, _, let summary, let items, let fingerprint, _, _):
            if !items.isEmpty && !fingerprint.isEmpty {
                compactionSource(.init(id: id, summary: summary, sourceItemIDs: items, sourceFingerprint: fingerprint))
            } else { Text(summary).font(.caption).textSelection(.enabled) }
        case .uiAction(let action):
            Label(action.summary, systemImage: "hand.tap").font(.caption).foregroundStyle(.secondary)
        case .attachment(let attachment):
            VStack(alignment: .leading, spacing: 2) {
                Label(attachment.name, systemImage: "doc")
                let facts = [attachment.operation, attachment.mimeType].compactMap { $0 }
                if !facts.isEmpty { Text(facts.joined(separator: " · ")).foregroundStyle(.secondary) }
                Text(attachment.uri).font(.caption2.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
            }.font(.caption)
        case .reasoning, .tool: EmptyView() // Shared flow sends these through the process renderer.
        }
    }

    private var assistantMessage: some View {
        let runtime = ChatAssistantRuntimeState(
            message: message,
            isActive: isStreaming,
            isRetrieving: isRetrieving
        )
        return HStack(alignment: .top, spacing: 0) {
            VStack(alignment: .leading, spacing: 5) {
                if !message.includeInContext {
                    Text(NSLocalizedString("chat.bubble.archived_to_session", value: "已折叠到 session memory", comment: "Message archived to session memory"))
                        .font(.caption2.weight(.semibold))
                        .foregroundColor(DesignTokens.warningColor)
                }
                if isLatestAssistant, runtime.showsPhase {
                    HStack(spacing: 9) {
                        ChatThinkingStatusIcon(isFinished: false)
                        Text(ChatTracePhasePresentation.displayName(for: message.tracePhase))
                            .font(.callout.weight(.medium))
                            .foregroundStyle(.secondary)
                            .contentTransition(.identity)
                    }
                    .padding(.vertical, 4)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("chat.assistant.phase")
                }
                let thinkingText = message.thinking.trimmingCharacters(in: .whitespacesAndNewlines)
                if !message.canonicalParts.isEmpty {
                    ForEach(ChatExecutionProcessPolicy.segments(message.canonicalParts)) { segment in
                        if segment.kind == .process {
                            ChatCanonicalExecutionProcess(parts: segment.parts,
                                events: message.hostToolEvents, activities: message.traceActivities,
                                isLive: runtime.isActive,
                                expanded: processExpansion(segment.parts, live: runtime.isActive),
                                toolExpanded: { toolExpansion($0, technical: false) },
                                technicalExpanded: { toolExpansion($0, technical: true) },
                                reader: toolReader)
                        } else {
                            ForEach(segment.parts, id: \.partID) { part in
                                canonicalContent(part)
                            }
                        }
                    }
                }
                if message.canonicalParts.contains(where: { if case .text(let text) = $0 { text.isDisclosed && !text.markdown.isEmpty } else { false } }) {
                    HStack(spacing: 4) { copyButton.disabled(isStreaming) }
                        .font(.caption).buttonStyle(.borderless).frame(minHeight: 44)
                    if !isStreaming { answerContextFooter }
                }
                if message.canonicalParts.isEmpty && runtime.showsThinking {
                    ChatThinkingProcessView(
                        text: thinkingText,
                        isFinished: !runtime.isActive || message.thinkingFinished,
                        isRetrieving: runtime.isRetrieving
                    )
                    .padding(.bottom, 4)
                }

                if message.canonicalParts.isEmpty && runtime.hasTools {
                    HostToolEventsView(
                        events: message.liveHostToolEvents,
                        activities: message.traceActivities,
                        canonicalTools: message.canonicalTools,
                        pendingPermissionRequest: pendingHostPermissionRequest,
                        onPermissionDecision: onHostPermissionDecision,
                        expandedByDefault: false
                    )
                    .padding(.bottom, 4)
                }

                if (!message.canonicalParts.isEmpty || !runtime.hasTools), let pendingHostPermissionRequest {
                    HostPermissionDecisionCard(request: pendingHostPermissionRequest, onPermissionDecision: onHostPermissionDecision)
                }

                let historicalTools = isStreaming ? message.failedHistoricalToolEvents : message.hostToolEvents
                if message.canonicalParts.isEmpty && !historicalTools.isEmpty {
                    HostToolEventsView(
                        events: historicalTools,
                        activities: message.traceActivities,
                        canonicalTools: message.canonicalTools,
                        pendingPermissionRequest: nil,
                        onPermissionDecision: onHostPermissionDecision,
                        expandedByDefault: false
                    )
                    .padding(.bottom, 4)
                }

                if message.canonicalParts.isEmpty { taskPresentation(nil) }

                if runtime.showsWaiting {
                    HStack(spacing: 9) {
                        ChatThinkingStatusIcon(isFinished: false)
                        Text(NSLocalizedString(
                            "history.processing_info.title",
                            value: "Processing",
                            comment: "Immediate assistant placeholder shown before response events arrive"
                        ))
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 4)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("chat.assistant.processing")
                }

                if let terminalState = ChatResponseStatusPolicy.terminalState(message.metadata) {
                    Label(
                        terminalState == .failed
                            ? NSLocalizedString("chat.response.failed", value: "Response failed. Open the process for details.", comment: "Visible terminal failure")
                            : NSLocalizedString("chat.response.cancelled", value: "Response stopped", comment: "Visible cancellation"),
                        systemImage: terminalState == .failed ? "exclamationmark.circle" : "stop.circle"
                    )
                    .font(.callout)
                    .foregroundStyle(terminalState == .failed ? DesignTokens.dangerColor : Color.secondary)
                    .accessibilityIdentifier("chat.assistant.terminal")
                }

                let answer = message.canonicalParts.contains(where: { if case .text = $0 { true } else { false } }) ? "" : message.content
                if !answer.isEmpty {
                    ChatSmoothStreamingMarkdownView(
                        content: answer,
                        isStreaming: isStreaming
                    )
                        .textSelection(.enabled)
                        .frame(width: expertTextWidth(answer), alignment: .leading)
                        .padding(.horizontal, expertHorizontalPadding)
                        .padding(.top, 10)
                        .padding(.bottom, 9)
                        .background {
                            UnevenRoundedRectangle(topLeadingRadius: 5, bottomLeadingRadius: 14,
                                    bottomTrailingRadius: 14, topTrailingRadius: 14).fill(assistantBubbleFill ?? DesignTokens.surfaceColor)
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("chat.assistant.answer")

                    HStack(spacing: 4) { copyButton.disabled(isStreaming) }
                        .font(.caption).buttonStyle(.borderless).frame(minHeight: 44)
                    if !isStreaming { answerContextFooter }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .contextMenu {
                if !isStreaming {
                    copyButton
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.assistant.bubble")
        .accessibilityValue(message.metadata[ChatConversationSurfaceMeta.stateKey] ?? (isStreaming ? "streaming" : "completed"))
        .overlay(alignment: .topLeading) {
            copiedHint
                .padding(.leading, 4)
        }
    }

    private var copyButton: some View {
        Button {
            copyMessageToPasteboard()
        } label: {
            Label {
                Text(NSLocalizedString("chat.message.copy", value: "复制消息", comment: "Copy chat message"))
            } icon: {
                if showCopiedHint {
                    Image(systemName: "checkmark").frame(width: 14, height: 14)
                } else {
                    Image("nola-copy").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 14, height: 14)
                }
            }
            .font(.caption).foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("chat.message.copy.\(message.id.uuidString.lowercased())")
        .accessibilityLabel(showCopiedHint
            ? NSLocalizedString("chat.message.copied", value: "已复制", comment: "Copied chat message hint")
            : NSLocalizedString("chat.message.copy", value: "复制消息", comment: "Copy chat message"))
    }

    @ViewBuilder
    private var answerContextFooter: some View {
        let basis = answerBasisIdentifiers
        let learnedCount = Int(message.metadata["nola.learnedCount"] ?? "") ?? 0
        if !basis.isEmpty || learnedCount > 0 {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    if !basis.isEmpty {
                        Button {
                            withAnimation(reduceMotion ? nil : DesignTokens.disclosureAnimation) {
                                showAnswerContextDetails.toggle()
                            }
                        } label: {
                            HStack(spacing: 5) {
                                Image(systemName: "link")
                                Text(basis.map(answerSourceLabel).joined(separator: " · "))
                                    .lineLimit(1)
                                Image(systemName: showAnswerContextDetails ? "chevron.up" : "chevron.down")
                                    .font(.system(size: 8, weight: .bold))
                            }
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .background(Color.primary.opacity(0.045), in: Capsule())
                        }
                        .buttonStyle(NolaActionButtonStyle())
                    }

                    if learnedCount > 0 {
                        Button {
                            showPersonalIntelligence = true
                        } label: {
                            Label(
                                String(
                                    format: NSLocalizedString("chat.answer.memory_updates", value: "%d memory updates", comment: "Count of memory updates learned from an answer"),
                                    learnedCount
                                ),
                                systemImage: "brain.head.profile"
                            )
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(DesignTokens.primaryColor)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .background(DesignTokens.primaryColor.opacity(0.08), in: Capsule())
                        }
                        .buttonStyle(NolaActionButtonStyle())
                    }
                }

                if showAnswerContextDetails {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(NSLocalizedString("chat.answer.basis_detail", value: "Only sources that were included in this answer are shown.", comment: "Answer basis detail"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        ForEach(basis, id: \.self) { source in
                            Label(answerSourceLabel(source), systemImage: answerSourceIcon(source))
                                .font(.caption.weight(.medium))
                                .foregroundStyle(.primary)
                        }
                        if let preview = message.metadata["nola.learnedPreview"], !preview.isEmpty {
                            Text("“\(preview)”")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(3)
                        }
                    }
                    .padding(11)
                    .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .nolaDisclosureTransition(reduceMotion: reduceMotion)
                }
            }
            .padding(.top, 6)
        }
    }

    private var answerBasisIdentifiers: [String] {
        let raw = message.metadata["nola.answerBasis"] ?? ""
        var seen = Set<String>()
        return raw
            .split(separator: ",")
            .map(String.init)
            .filter {
                !$0.isEmpty
                    && !($0 == "tools" && !message.hostToolEvents.isEmpty)
                    && seen.insert($0).inserted
            }
    }

    private func answerSourceLabel(_ source: String) -> String {
        switch source {
        case "file": return NSLocalizedString("chat.answer.source.file", value: "File", comment: "Answer source: uploaded file")
        case "memory": return NSLocalizedString("chat.answer.source.memory", value: "Personal memory", comment: "Answer source: personal memory")
        case "knowledge": return NSLocalizedString("chat.answer.source.knowledge", value: "Knowledge", comment: "Answer source: connected knowledge")
        case "skills": return NSLocalizedString("chat.answer.source.skills", value: "Skills", comment: "Answer source: enabled skills")
        case "tools": return NSLocalizedString("chat.answer.source.tools", value: "Tool results", comment: "Answer source: tool results")
        default: return source.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    private func answerSourceIcon(_ source: String) -> String {
        switch source {
        case "file": return "doc.text"
        case "memory": return "brain.head.profile"
        case "knowledge": return "books.vertical"
        case "skills": return "shippingbox"
        case "tools": return "wrench.and.screwdriver"
        default: return "link"
        }
    }

    @ViewBuilder
    private var copiedHint: some View {
        if showCopiedHint {
            Label(
                NSLocalizedString("chat.message.copied", value: "已复制", comment: "Copied chat message hint"),
                systemImage: "checkmark"
            )
                .font(.caption.weight(.semibold))
                .foregroundStyle(.primary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(.regularMaterial, in: Capsule())
                .transition(reduceMotion ? .opacity : .opacity.combined(with: .scale(scale: 0.96)))
        }
    }

    private func copyMessageToPasteboard() {
        let text = copyableText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        UIPasteboard.general.string = text
        guard UIPasteboard.general.string == text else { showManualCopy = true; return }
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        copyFeedbackTask?.cancel()
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.12)) {
            showCopiedHint = true
        }
        copyFeedbackTask = Task { @MainActor in
            do { try await Task.sleep(for: .seconds(1.8)) }
            catch { return }
            withAnimation(reduceMotion ? nil : .easeIn(duration: 0.16)) {
                showCopiedHint = false
            }
        }
    }

    private var copyableText: String {
        let content = message.content.trimmingCharacters(in: .whitespacesAndNewlines)
        if !content.isEmpty {
            return content
        }
        let thinking = message.thinking.trimmingCharacters(in: .whitespacesAndNewlines)
        if !thinking.isEmpty {
            return thinking
        }
        return message.hostToolEvents
            .map { event in
                [event.name, event.display, event.text]
                    .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                    .filter { !$0.isEmpty }
                    .joined(separator: "\n")
            }
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }

}

private struct ChatMessageAttachment: Identifiable {
    let id: String
    let name: String
    let url: URL?
    let sizeBytes: Int64?

    var iconName: String {
        let ext = URL(fileURLWithPath: name).pathExtension.lowercased()
        return ["png", "jpg", "jpeg", "gif", "heic", "webp"].contains(ext) ? "photo" : "doc.text"
    }

    var sizeLabel: String? {
        guard let sizeBytes else { return nil }
        return ByteCountFormatter.string(fromByteCount: sizeBytes, countStyle: .file)
    }

    var detailLabel: String? {
        let fileType = URL(fileURLWithPath: name).pathExtension.uppercased()
        return [fileType.isEmpty ? nil : fileType, sizeLabel]
            .compactMap { $0 }
            .joined(separator: " · ")
            .nilIfEmpty
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}

private struct ChatAttachmentPreviewItem: Identifiable {
    let id = UUID()
    let url: URL
}

private struct ChatAttachmentQuickLook: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator {
        Coordinator(url: url)
    }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {
        context.coordinator.url = url
        controller.reloadData()
    }

    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        var url: URL

        init(url: URL) {
            self.url = url
        }

        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }

        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
            url as NSURL
        }
    }
}

enum ChatMarkdownDisplayNormalizer {
    static func preservingSoftLineBreaks(in markdown: String) -> String {
        let lines = markdown.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard lines.count > 1 else { return markdown }

        var output = ""
        var inFence = false

        for index in lines.indices {
            let line = lines[index]
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let isFence = trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~")

            output += line

            guard index < lines.index(before: lines.endIndex) else { continue }

            if isFence {
                inFence.toggle()
                output += "\n"
                continue
            }

            let nextLine = lines[lines.index(after: index)]
            let nextTrimmed = nextLine.trimmingCharacters(in: .whitespaces)
            let alreadyHardBreak = line.hasSuffix("  ") || line.hasSuffix("\\")
            let shouldPreserveAsHardBreak = !inFence && !trimmed.isEmpty && !nextTrimmed.isEmpty && !alreadyHardBreak

            output += shouldPreserveAsHardBreak ? "  \n" : "\n"
        }

        return output
    }
}

private struct ChatMarkdownInputBridge: UIViewRepresentable {
    let input: AppleMarkdownRenderSnapshot
    let scheduler: AppleMarkdownPresentationScheduler
    func makeUIView(context: Context) -> UIView { UIView(frame: .zero) }
    func updateUIView(_ uiView: UIView, context: Context) { scheduler.submit(input) }
}

struct ChatSmoothStreamingMarkdownView: View {
    let content: String
    let isStreaming: Bool
    @StateObject private var presentation: AppleMarkdownPresentationScheduler

    init(content: String, isStreaming: Bool) {
        self.content = content
        self.isStreaming = isStreaming
        _presentation = StateObject(wrappedValue: AppleMarkdownPresentationScheduler(
            .init(content: content, isStreaming: isStreaming)))
    }

    var body: some View {
        let current = presentation.snapshot
        let blocks = presentation.buffer.renderBlocks(settleTail: !current.isStreaming)
        VStack(alignment: .leading, spacing: 6) {
            ForEach(blocks) { block in
                ChatStableStreamingMarkdownBlockView(block: block, isStreaming: current.isStreaming)
                    .equatable()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier(block.isMutableTail
                        ? "chat.assistant.markdown.tail"
                        : "chat.assistant.markdown.committed.\(block.ordinal)")
            }
            ZStack(alignment: .leading) {
                if current.isStreaming { ChatStreamingActivityIndicator() }
            }
            .frame(height: 8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(ChatMarkdownInputBridge(
            input: .init(content: content, isStreaming: isStreaming), scheduler: presentation)
            .frame(width: 0, height: 0))
        .onDisappear { presentation.cancel() }
    }
}

private struct ChatStableStreamingMarkdownBlockView: View, Equatable {
    let block: AppleStreamingMarkdownRenderBlock
    let isStreaming: Bool

    static func == (
        lhs: ChatStableStreamingMarkdownBlockView,
        rhs: ChatStableStreamingMarkdownBlockView
    ) -> Bool {
        lhs.block.hasSameRenderedContent(as: rhs.block)
            && lhs.isStreaming == rhs.isStreaming
    }

    var body: some View {
        ChatMarkdownView(
            content: block.isMutableTail
                ? AppleStreamingMarkdownTailNormalizer.stabilize(block.markdown)
                : block.markdown
        )
        .environment(\.appleStreamingRevealActive, isStreaming)
    }
}

extension AppleStreamingMarkdownRenderBlock {
    func hasSameRenderedContent(as other: Self) -> Bool {
        ordinal == other.ordinal && markdown == other.markdown
    }
}

private struct ChatStreamingActivityIndicator: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 0.24, paused: reduceMotion)) { context in
            let activeDot = Int(context.date.timeIntervalSinceReferenceDate * 4) % 3
            HStack(spacing: 4) {
                ForEach(0..<3, id: \.self) { index in
                    Circle()
                        .fill(Color.secondary.opacity(reduceMotion ? 0.42 : (index == activeDot ? 0.72 : 0.20)))
                        .frame(width: 4, height: 4)
                }
            }
            .frame(height: 8)
            .accessibilityLabel(NSLocalizedString("chat.streaming.active", value: "正在生成", comment: "Streaming response activity"))
        }
    }
}

struct ChatMarkdownView: View {
    let content: String

    private var blocks: [ChatMarkdownRenderedBlock] {
        var occurrence: [String: Int] = [:]
        return ChatMarkdownBlockParser.parse(content).map { block in
            let kind = block.runtimeKind
            let index = occurrence[kind, default: 0]
            occurrence[kind] = index + 1
            return ChatMarkdownRenderedBlock(id: "\(kind)-\(index)", block: block)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(blocks) { rendered in
                blockView(rendered.block)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func blockView(_ block: ChatMarkdownBlock) -> some View {
        switch block {
        case .heading(let level, let text):
            ChatMarkdownInlineText(text)
                .font(headingFont(level))
                .foregroundColor(.primary)
                .padding(.top, level <= 2 ? 4 : 2)
                .padding(.bottom, 1)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Text(text))
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("chat.assistant.markdown.heading.\(level)")
        case .paragraph(let text):
            ChatMarkdownInlineText(text)
                .font(.body)
                .foregroundColor(.primary)
                .lineSpacing(IOSChatProseMetrics.lineSpacing)
        case .unorderedList(let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(items.indices, id: \.self) { index in
                    ChatMarkdownListItemView(item: items[index])
                }
            }
            .accessibilityIdentifier("chat.assistant.markdown.unordered-list")
        case .orderedList(let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(items.indices, id: \.self) { index in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("\(items[index].number).")
                            .font(.body.weight(.semibold))
                            .foregroundColor(.secondary)
                            .monospacedDigit()
                        ChatMarkdownInlineText(items[index].text)
                            .font(.body)
                            .foregroundColor(.primary)
                            .lineSpacing(IOSChatProseMetrics.lineSpacing)
                    }
                }
            }
            .accessibilityIdentifier("chat.assistant.markdown.ordered-list")
        case .quote(let text):
            HStack(alignment: .top, spacing: 10) {
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(DesignTokens.primaryColor.opacity(0.55))
                    .frame(width: 3)
                ChatMarkdownInlineText(text)
                    .font(.callout)
                    .foregroundColor(.secondary)
                    .lineSpacing(3)
            }
            .padding(.vertical, 2)
        case .code(let language, let text):
            ChatMarkdownCodeBlock(language: language, text: text)
        case .math(let text):
            ChatMarkdownMathBlock(text: text)
        case .mermaid(let text):
            ChatMarkdownMermaidBlock(source: text)
        case .table(let table):
            ChatMarkdownTableBlock(table: table)
        case .rule:
            NolaDivider()
                .padding(.vertical, 4)
        }
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1:
            return .title3.weight(.bold)
        case 2:
            return .headline.weight(.bold)
        case 3:
            return .subheadline.weight(.bold)
        default:
            return .callout.weight(.semibold)
        }
    }
}

private struct ChatMarkdownRenderedBlock: Identifiable {
    let id: String
    let block: ChatMarkdownBlock
}

private struct ChatMarkdownInlineText: View {
    let text: String
    private let tokens: [ChatMarkdownInlineToken]

    init(_ text: String) {
        self.text = text
        self.tokens = ChatMarkdownInlineMathParser.parse(text)
    }

    var body: some View {
        if tokens.allSatisfy(\.isPlainText) {
            AppleStreamingAttributedText(value: attributedText(for: text))
        } else {
            ChatInlineFlowLayout(horizontalSpacing: 4, verticalSpacing: 3) {
                ForEach(tokens.indices, id: \.self) { index in
                    switch tokens[index] {
                    case .text(let value):
                        if !value.isEmpty {
                            AppleStreamingAttributedText(value: attributedText(for: value))
                        }
                    case .math(let value):
                        ChatMathLabel(
                            equation: value,
                            mode: .text,
                            fontSize: 17,
                            alignment: .center
                        )
                        .frame(minWidth: 18, minHeight: 22)
                        .padding(.horizontal, 1)
                    }
                }
            }
        }
    }

    private func attributedText(for value: String) -> AttributedString {
        let normalized = ChatMarkdownDisplayNormalizer.preservingSoftLineBreaks(in: value)
        return (try? AttributedString(
            markdown: normalized,
            options: AttributedString.MarkdownParsingOptions(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )) ?? AttributedString(value)
    }
}

private struct ChatMarkdownListItem {
    let text: String
    let taskState: TaskState?

    enum TaskState {
        case checked
        case unchecked
    }
}

private struct ChatMarkdownListItemView: View {
    let item: ChatMarkdownListItem

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let taskState = item.taskState {
                Image(systemName: taskState == .checked ? "checkmark.square.fill" : "square")
                    .font(.body.weight(.semibold))
                    .foregroundColor(taskState == .checked ? DesignTokens.primaryColor : .secondary)
                    .accessibilityHidden(true)
            } else {
                Text("•")
                    .font(.body.weight(.semibold))
                    .foregroundColor(.secondary)
            }
            ChatMarkdownInlineText(item.text)
                .font(.body)
                .foregroundColor(.primary)
                .lineSpacing(IOSChatProseMetrics.lineSpacing)
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(item.taskState.map { state in
            NSLocalizedString(state == .checked ? "chat.checklist.checked" : "chat.checklist.unchecked", comment: "Markdown checklist state")
        } ?? "")
    }
}

private enum ChatMarkdownInlineToken {
    case text(String)
    case math(String)

    var isPlainText: Bool {
        if case .text = self { return true }
        return false
    }
}

private enum ChatMarkdownInlineMathParser {
    static func parse(_ text: String) -> [ChatMarkdownInlineToken] {
        guard text.contains("$") || text.contains("\\(") else {
            return [.text(text)]
        }

        var tokens: [ChatMarkdownInlineToken] = []
        var scan = text.startIndex
        var textStart = text.startIndex

        func appendText(upTo index: String.Index) {
            guard textStart < index else { return }
            tokens.append(.text(String(text[textStart..<index])))
        }

        while scan < text.endIndex {
            if text[scan] == "$", !isEscaped(scan, in: text), !text[text.index(after: scan)...].starts(with: "$") {
                let start = text.index(after: scan)
                if let close = closingSingleDollar(in: text, from: start) {
                    appendText(upTo: scan)
                    tokens.append(.math(String(text[start..<close]).trimmingCharacters(in: .whitespacesAndNewlines)))
                    scan = text.index(after: close)
                    textStart = scan
                    continue
                }
            }

            if text[scan...].starts(with: "\\(") {
                let start = text.index(scan, offsetBy: 2)
                if let close = text[start...].range(of: "\\)")?.lowerBound {
                    appendText(upTo: scan)
                    tokens.append(.math(String(text[start..<close]).trimmingCharacters(in: .whitespacesAndNewlines)))
                    scan = text.index(close, offsetBy: 2)
                    textStart = scan
                    continue
                }
            }

            scan = text.index(after: scan)
        }

        if textStart < text.endIndex {
            tokens.append(.text(String(text[textStart...])))
        }

        return tokens.isEmpty ? [.text(text)] : splitPlainTextTokens(tokens)
    }

    private static func splitPlainTextTokens(_ tokens: [ChatMarkdownInlineToken]) -> [ChatMarkdownInlineToken] {
        tokens.flatMap { token -> [ChatMarkdownInlineToken] in
            guard case .text(let value) = token else { return [token] }
            var parts: [ChatMarkdownInlineToken] = []
            var current = ""
            for character in value {
                current.append(character)
                if character.isWhitespace {
                    parts.append(.text(current))
                    current = ""
                }
            }
            if !current.isEmpty {
                parts.append(.text(current))
            }
            return parts
        }
    }

    private static func closingSingleDollar(in text: String, from start: String.Index) -> String.Index? {
        var index = start
        while index < text.endIndex {
            if text[index] == "\n" { return nil }
            if text[index] == "$", !isEscaped(index, in: text) {
                return index
            }
            index = text.index(after: index)
        }
        return nil
    }

    private static func isEscaped(_ index: String.Index, in text: String) -> Bool {
        guard index > text.startIndex else { return false }
        var slashCount = 0
        var current = text.index(before: index)
        while true {
            if text[current] == "\\" {
                slashCount += 1
            } else {
                break
            }
            if current == text.startIndex { break }
            current = text.index(before: current)
        }
        return slashCount % 2 == 1
    }
}

private struct ChatInlineFlowLayout: Layout {
    let horizontalSpacing: CGFloat
    let verticalSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320
        let rows = rows(for: subviews, maxWidth: width)
        let height = rows.reduce(CGFloat.zero) { partial, row in
            partial + row.height
        } + CGFloat(max(0, rows.count - 1)) * verticalSpacing
        let widest = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? widest, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let rows = rows(for: subviews, maxWidth: bounds.width)
        var y = bounds.minY
        for row in rows {
            var x = bounds.minX
            for item in row.items {
                let size = item.size
                item.subview.place(
                    at: CGPoint(x: x, y: y + (row.height - size.height) / 2),
                    proposal: ProposedViewSize(size)
                )
                x += size.width + horizontalSpacing
            }
            y += row.height + verticalSpacing
        }
    }

    private func rows(for subviews: Subviews, maxWidth: CGFloat) -> [Row] {
        var rows: [Row] = []
        var current = Row()
        let widthLimit = max(1, maxWidth)

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            let nextWidth = current.items.isEmpty
                ? size.width
                : current.width + horizontalSpacing + size.width
            if !current.items.isEmpty, nextWidth > widthLimit {
                rows.append(current)
                current = Row()
            }
            current.append(subview: subview, size: size, spacing: horizontalSpacing)
        }

        if !current.items.isEmpty {
            rows.append(current)
        }
        return rows
    }

    private struct Row {
        var items: [Item] = []
        var width: CGFloat = 0
        var height: CGFloat = 0

        mutating func append(subview: LayoutSubview, size: CGSize, spacing: CGFloat) {
            if !items.isEmpty {
                width += spacing
            }
            items.append(Item(subview: subview, size: size))
            width += size.width
            height = max(height, size.height)
        }
    }

    private struct Item {
        let subview: LayoutSubview
        let size: CGSize
    }
}

private struct ChatMarkdownCodeBlock: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let language: String?
    let text: String
    @State private var showCopiedHint = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text(codeLabel)
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.secondary)
                Spacer(minLength: 8)
                Button {
                    copyCode()
                } label: {
                    Label(
                        showCopiedHint
                            ? NSLocalizedString("chat.markdown.code.copied", value: "已复制", comment: "Code copied")
                            : NSLocalizedString("chat.markdown.code.copy", value: "复制", comment: "Copy code"),
                        systemImage: showCopiedHint ? "checkmark" : "doc.on.doc"
                    )
                    .font(.caption2.weight(.semibold))
                    .labelStyle(.titleAndIcon)
                }
                .buttonStyle(NolaActionButtonStyle())
                .foregroundColor(showCopiedHint ? DesignTokens.successColor : DesignTokens.primaryColor)
                .disabled(text.isEmpty)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Color.primary.opacity(0.035))

            ScrollView(.horizontal, showsIndicators: false) {
                AppleStreamingAttributedText(value: AttributedString(text))
                    .font(.system(.callout, design: .monospaced))
                    .foregroundColor(.primary)
                    .textSelection(.enabled)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(Color.primary.opacity(0.055))
        )
        .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }

    private func copyCode() {
        guard !text.isEmpty else { return }
        UIPasteboard.general.string = text
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.12)) {
            showCopiedHint = true
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.1) {
            withAnimation(reduceMotion ? nil : .easeIn(duration: 0.16)) {
                showCopiedHint = false
            }
        }
    }

    private var codeLabel: String {
        let normalized = language?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let normalized, !normalized.isEmpty else {
            return NSLocalizedString("chat.markdown.code", value: "代码", comment: "Code block label")
        }
        return normalized.lowercased()
    }
}

private struct ChatMarkdownTableData {
    let headers: [String]
    let alignments: [ChatMarkdownTableAlignment]
    let rows: [[String]]
}

private enum ChatMarkdownTableAlignment {
    case leading
    case center
    case trailing

    var frameAlignment: Alignment {
        switch self {
        case .leading: return .leading
        case .center: return .center
        case .trailing: return .trailing
        }
    }

    var textAlignment: SwiftUI.TextAlignment {
        switch self {
        case .leading: return .leading
        case .center: return .center
        case .trailing: return .trailing
        }
    }
}

private struct ChatMarkdownTableBlock: View {
    let table: ChatMarkdownTableData

    private var allRows: [[String]] {
        [table.headers] + table.rows
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(allRows.indices, id: \.self) { rowIndex in
                    HStack(spacing: 0) {
                        let row = allRows[rowIndex]
                        ForEach(row.indices, id: \.self) { columnIndex in
                            let alignment = table.alignments.indices.contains(columnIndex)
                                ? table.alignments[columnIndex]
                                : .leading
                            ChatMarkdownInlineText(row[columnIndex])
                                .font(.subheadline)
                                .foregroundColor(.primary)
                                .lineSpacing(2)
                                .multilineTextAlignment(alignment.textAlignment)
                                .frame(minWidth: 120, maxWidth: 220, alignment: alignment.frameAlignment)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
                                .background(
                                    rowIndex == 0
                                        ? Color.primary.opacity(0.07)
                                        : Color.clear
                                )
                                .overlay(alignment: .topLeading) {
                                    Rectangle()
                                        .fill(DesignTokens.borderColor)
                                        .frame(width: 1)
                                        .opacity(columnIndex == 0 ? 0 : 1)
                                }
                        }
                    }
                    .overlay(alignment: .top) {
                        Rectangle()
                            .fill(DesignTokens.borderColor)
                            .frame(height: 1)
                            .opacity(rowIndex == 0 ? 0 : 1)
                    }
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                    .stroke(DesignTokens.borderColor, lineWidth: 1)
            )
            .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
        }
    }
}

private struct ChatMarkdownMathBlock: View {
    let text: String

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            ChatMathLabel(
                equation: text.trimmingCharacters(in: .whitespacesAndNewlines),
                mode: .display,
                fontSize: 22,
                alignment: .center
            )
            .frame(minWidth: 260, minHeight: 52)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
        }
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.primaryColor.opacity(0.06))
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.primaryColor.opacity(0.18), lineWidth: 1)
        )
    }
}

private struct ChatMarkdownMermaidBlock: View {
    let source: String
    @Environment(\.colorScheme) private var colorScheme
    @State private var parseError: Error?
    @State private var diagramBounds: CGRect = .zero

    var body: some View {
        Group {
            if parseError == nil {
                ScrollView([.horizontal, .vertical], showsIndicators: true) {
                    MermaidDiagramView(
                        source: source,
                        theme: mermaidTheme,
                        parseError: $parseError,
                        diagramBounds: $diagramBounds
                    )
                    .frame(width: diagramSize.width, height: diagramSize.height)
                    .padding(12)
                }
                .frame(height: containerHeight)
            } else {
                ChatMarkdownCodeBlock(language: "mermaid", text: source)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.primaryColor.opacity(0.04))
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
        .onChange(of: source) { _, _ in
            parseError = nil
            diagramBounds = .zero
        }
        .onChange(of: colorScheme) { _, _ in
            diagramBounds = .zero
        }
    }

    private var mermaidTheme: DiagramTheme {
        let theme = colorScheme == .dark ? DiagramTheme.zincDark : DiagramTheme.zincLight
        return theme.withTransparent(true)
    }

    private var diagramSize: CGSize {
        guard diagramBounds.width > 0, diagramBounds.height > 0 else {
            return CGSize(width: 300, height: 180)
        }
        return CGSize(
            width: max(300, ceil(diagramBounds.width)),
            height: max(180, ceil(diagramBounds.height))
        )
    }

    private var containerHeight: CGFloat {
        min(640, max(180, diagramSize.height + 24))
    }
}

private struct ChatMathLabel: UIViewRepresentable {
    let equation: String
    let mode: MTMathUILabelMode
    let fontSize: CGFloat
    let alignment: MTTextAlignment

    func makeUIView(context: Context) -> MTMathUILabel {
        let label = MTMathUILabel()
        label.contentInsets = UIEdgeInsets(top: 2, left: 2, bottom: 2, right: 2)
        label.displayErrorInline = true
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        label.setContentHuggingPriority(.defaultHigh, for: .vertical)
        return label
    }

    func updateUIView(_ label: MTMathUILabel, context: Context) {
        label.latex = equation
        label.labelMode = mode
        label.textAlignment = alignment
        label.fontSize = fontSize
        label.textColor = UIColor.label
        label.backgroundColor = .clear
    }
}

private enum ChatMarkdownBlock {
    case heading(level: Int, text: String)
    case paragraph(String)
    case unorderedList([ChatMarkdownListItem])
    case orderedList([OrderedItem])
    case quote(String)
    case code(language: String?, text: String)
    case math(String)
    case mermaid(String)
    case table(ChatMarkdownTableData)
    case rule
}

private extension ChatMarkdownBlock {
    var runtimeKind: String {
        switch self {
        case .heading: return "heading"
        case .paragraph: return "paragraph"
        case .unorderedList: return "unordered-list"
        case .orderedList: return "ordered-list"
        case .quote: return "quote"
        case .code: return "code"
        case .math: return "math"
        case .mermaid: return "mermaid"
        case .table: return "table"
        case .rule: return "rule"
        }
    }
}

enum ChatMarkdownRuntimeInspector {
    static func blockKinds(in markdown: String) -> [String] {
        ChatMarkdownBlockParser.parse(markdown).map(\.runtimeKind)
    }
}

private struct OrderedItem {
    let number: Int
    let text: String
}

private enum ChatMarkdownBlockParser {
    static func parse(_ markdown: String) -> [ChatMarkdownBlock] {
        let lines = markdown.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard !lines.isEmpty else { return [] }

        var blocks: [ChatMarkdownBlock] = []
        var paragraphLines: [String] = []
        var unorderedItems: [ChatMarkdownListItem] = []
        var orderedItems: [OrderedItem] = []
        var quoteLines: [String] = []
        var codeLines: [String] = []
        var tableHeader: [String]? = nil
        var tableAlignments: [ChatMarkdownTableAlignment] = []
        var tableRows: [[String]] = []
        var skipTableSeparatorIndex: Int? = nil
        var inFence = false
        var fenceMarker: String?
        var currentFenceLanguage: String?
        var mathLines: [String] = []
        var inMathBlock = false
        var mathCloseMarker: String?

        func flushParagraph() {
            let text = paragraphLines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty { blocks.append(.paragraph(text)) }
            paragraphLines.removeAll()
        }

        func flushUnorderedList() {
            if !unorderedItems.isEmpty { blocks.append(.unorderedList(unorderedItems)) }
            unorderedItems.removeAll()
        }

        func flushOrderedList() {
            if !orderedItems.isEmpty { blocks.append(.orderedList(orderedItems)) }
            orderedItems.removeAll()
        }

        func flushQuote() {
            let text = quoteLines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty { blocks.append(.quote(text)) }
            quoteLines.removeAll()
        }

        func flushTable() {
            guard let header = tableHeader else { return }
            let columnCount = header.count
            let normalizedRows = tableRows.map { normalizeTableRow($0, columnCount: columnCount) }
            let normalizedAlignments = normalizeTableAlignments(tableAlignments, columnCount: columnCount)
            blocks.append(.table(ChatMarkdownTableData(headers: header, alignments: normalizedAlignments, rows: normalizedRows)))
            tableHeader = nil
            tableAlignments.removeAll()
            tableRows.removeAll()
        }

        func flushTextBlocks() {
            flushTable()
            flushParagraph()
            flushUnorderedList()
            flushOrderedList()
            flushQuote()
        }

        for index in lines.indices {
            if skipTableSeparatorIndex == index {
                skipTableSeparatorIndex = nil
                continue
            }

            let line = lines[index]
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if inMathBlock {
                if isMathClose(trimmed, marker: mathCloseMarker) {
                    blocks.append(.math(mathLines.joined(separator: "\n")))
                    mathLines.removeAll()
                    inMathBlock = false
                    mathCloseMarker = nil
                } else {
                    mathLines.append(line)
                }
                continue
            }

            if inFence {
                if isClosingFence(trimmed, marker: fenceMarker) {
                    let codeText = codeLines.joined(separator: "\n").trimmingCharacters(in: .newlines)
                    if currentFenceLanguage?.lowercased() == "mermaid" {
                        blocks.append(.mermaid(codeText))
                    } else {
                        blocks.append(.code(language: currentFenceLanguage, text: codeText))
                    }
                    codeLines.removeAll()
                    inFence = false
                    fenceMarker = nil
                    currentFenceLanguage = nil
                } else {
                    codeLines.append(line)
                }
                continue
            }

            if tableHeader != nil {
                if let rowCells = tableCells(in: line), !isTableSeparator(trimmed) {
                    tableRows.append(rowCells)
                    continue
                }
                flushTable()
            }

            if let marker = openingFenceMarker(in: trimmed) {
                flushTextBlocks()
                inFence = true
                fenceMarker = marker
                currentFenceLanguage = fenceLanguage(in: trimmed, marker: marker)
                codeLines.removeAll()
                continue
            }

            if let marker = openingMathMarker(in: trimmed) {
                flushTextBlocks()
                let rest = mathOpeningRemainder(in: trimmed, marker: marker)
                if let inlineMath = closedSingleLineMath(rest, marker: marker) {
                    blocks.append(.math(inlineMath))
                } else {
                    inMathBlock = true
                    mathCloseMarker = marker == "$$" ? "$$" : "\\]"
                    mathLines.removeAll()
                    if !rest.isEmpty {
                        mathLines.append(rest)
                    }
                }
                continue
            }

            if trimmed.isEmpty {
                flushTextBlocks()
                continue
            }

            if let headerCells = tableCells(in: line) {
                let nextIndex = index + 1
                if nextIndex < lines.count,
                   let alignments = tableSeparatorAlignments(lines[nextIndex].trimmingCharacters(in: .whitespaces)) {
                    flushTextBlocks()
                    tableHeader = headerCells
                    tableAlignments = alignments
                    tableRows.removeAll()
                    skipTableSeparatorIndex = nextIndex
                    continue
                }
            }

            if isHorizontalRule(trimmed) {
                flushTextBlocks()
                blocks.append(.rule)
                continue
            }

            if let heading = heading(in: trimmed) {
                flushTextBlocks()
                blocks.append(.heading(level: heading.level, text: heading.text))
                continue
            }

            if let item = unorderedItem(in: line) {
                flushParagraph()
                flushOrderedList()
                flushQuote()
                unorderedItems.append(item)
                continue
            }

            if let item = orderedItem(in: line) {
                flushParagraph()
                flushUnorderedList()
                flushQuote()
                orderedItems.append(item)
                continue
            }

            if let quote = quoteText(in: line) {
                flushParagraph()
                flushUnorderedList()
                flushOrderedList()
                quoteLines.append(quote)
                continue
            }

            flushUnorderedList()
            flushOrderedList()
            flushQuote()
            paragraphLines.append(line)
        }

        if inFence {
            let codeText = codeLines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            blocks.append(.code(language: currentFenceLanguage, text: codeText))
        }
        if inMathBlock {
            blocks.append(.math(mathLines.joined(separator: "\n")))
        }
        flushTextBlocks()

        return blocks
    }

    private static func heading(in trimmedLine: String) -> (level: Int, text: String)? {
        var level = 0
        for character in trimmedLine {
            if character == "#" {
                level += 1
            } else {
                break
            }
        }
        guard (1...6).contains(level), trimmedLine.dropFirst(level).first == " " else { return nil }
        let text = String(trimmedLine.dropFirst(level + 1)).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : (level, text)
    }

    private static func unorderedItem(in line: String) -> ChatMarkdownListItem? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard trimmed.count >= 3 else { return nil }
        let marker = trimmed.first
        guard marker == "-" || marker == "*" || marker == "+", trimmed.dropFirst().first == " " else { return nil }
        let text = String(trimmed.dropFirst(2)).trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return nil }
        if let task = taskListItem(from: text) {
            return task
        }
        return ChatMarkdownListItem(text: text, taskState: nil)
    }

    private static func orderedItem(in line: String) -> OrderedItem? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard let dotIndex = trimmed.firstIndex(of: ".") else { return nil }
        let numberText = String(trimmed[..<dotIndex])
        guard let number = Int(numberText), number > 0 else { return nil }
        let afterDot = trimmed.index(after: dotIndex)
        guard afterDot < trimmed.endIndex, trimmed[afterDot] == " " else { return nil }
        let textStart = trimmed.index(after: afterDot)
        guard textStart < trimmed.endIndex else { return nil }
        let text = String(trimmed[textStart...]).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : OrderedItem(number: number, text: text)
    }

    private static func quoteText(in line: String) -> String? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard trimmed.first == ">" else { return nil }
        let text = String(trimmed.dropFirst()).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : text
    }

    private static func openingFenceMarker(in trimmedLine: String) -> String? {
        if trimmedLine.hasPrefix("```") { return "```" }
        if trimmedLine.hasPrefix("~~~") { return "~~~" }
        return nil
    }

    private static func fenceLanguage(in trimmedLine: String, marker: String) -> String? {
        let info = String(trimmedLine.dropFirst(marker.count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !info.isEmpty else { return nil }
        let language = info.split(whereSeparator: { $0.isWhitespace }).first.map(String.init) ?? info
        return language.isEmpty ? nil : language
    }

    private static func taskListItem(from text: String) -> ChatMarkdownListItem? {
        let prefixes: [(String, ChatMarkdownListItem.TaskState)] = [
            ("[x] ", .checked),
            ("[X] ", .checked),
            ("[ ] ", .unchecked),
        ]
        for (prefix, state) in prefixes where text.hasPrefix(prefix) {
            let taskText = String(text.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces)
            return taskText.isEmpty ? nil : ChatMarkdownListItem(text: taskText, taskState: state)
        }
        return nil
    }

    private static func openingMathMarker(in trimmedLine: String) -> String? {
        if trimmedLine.hasPrefix("$$") { return "$$" }
        if trimmedLine.hasPrefix("\\[") { return "\\[" }
        return nil
    }

    private static func mathOpeningRemainder(in trimmedLine: String, marker: String) -> String {
        String(trimmedLine.dropFirst(marker.count)).trimmingCharacters(in: .whitespaces)
    }

    private static func closedSingleLineMath(_ text: String, marker: String) -> String? {
        guard !text.isEmpty else { return nil }
        if marker == "$$", text.hasSuffix("$$") {
            return String(text.dropLast(2)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if marker == "\\[", text.hasSuffix("\\]") {
            return String(text.dropLast(2)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return nil
    }

    private static func isMathClose(_ trimmedLine: String, marker: String?) -> Bool {
        guard let marker else { return false }
        return trimmedLine == marker
    }

    private static func isClosingFence(_ trimmedLine: String, marker: String?) -> Bool {
        guard let marker else { return false }
        return trimmedLine.hasPrefix(marker)
    }

    private static func isHorizontalRule(_ trimmedLine: String) -> Bool {
        let compact = trimmedLine.filter { !$0.isWhitespace }
        guard compact.count >= 3 else { return false }
        guard let first = compact.first, first == "-" || first == "*" || first == "_" else { return false }
        return compact.allSatisfy { $0 == first }
    }

    private static func tableCells(in line: String) -> [String]? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard trimmed.contains("|") else { return nil }

        let withoutLeadingPipe = trimmed.hasPrefix("|") ? String(trimmed.dropFirst()) : trimmed
        let withoutEdgePipes = withoutLeadingPipe.hasSuffix("|")
            ? String(withoutLeadingPipe.dropLast())
            : withoutLeadingPipe
        let cells = splitTableCells(withoutEdgePipes)
        guard cells.count >= 2 else { return nil }
        return cells
    }

    private static func tableSeparatorAlignments(_ trimmedLine: String) -> [ChatMarkdownTableAlignment]? {
        guard let cells = tableCells(in: trimmedLine), cells.count >= 2 else { return nil }
        var alignments: [ChatMarkdownTableAlignment] = []
        for cell in cells {
            let token = cell.trimmingCharacters(in: .whitespaces)
            guard token.count >= 3, token.contains("-") else { return nil }
            guard token.allSatisfy({ $0 == "-" || $0 == ":" }) else { return nil }
            if token.hasPrefix(":"), token.hasSuffix(":") {
                alignments.append(.center)
            } else if token.hasSuffix(":") {
                alignments.append(.trailing)
            } else {
                alignments.append(.leading)
            }
        }
        return alignments
    }

    private static func isTableSeparator(_ trimmedLine: String) -> Bool {
        tableSeparatorAlignments(trimmedLine) != nil
    }

    private static func splitTableCells(_ row: String) -> [String] {
        var cells: [String] = []
        var current = ""
        var escaping = false
        for character in row {
            if escaping {
                current.append(character)
                escaping = false
            } else if character == "\\" {
                escaping = true
            } else if character == "|" {
                cells.append(current.trimmingCharacters(in: .whitespaces))
                current = ""
            } else {
                current.append(character)
            }
        }
        if escaping { current.append("\\") }
        cells.append(current.trimmingCharacters(in: .whitespaces))
        return cells
    }

    private static func normalizeTableRow(_ cells: [String], columnCount: Int) -> [String] {
        if cells.count == columnCount {
            return cells
        }
        if cells.count > columnCount {
            return Array(cells.prefix(columnCount))
        }
        return cells + Array(repeating: "", count: columnCount - cells.count)
    }

    private static func normalizeTableAlignments(_ alignments: [ChatMarkdownTableAlignment], columnCount: Int) -> [ChatMarkdownTableAlignment] {
        if alignments.count == columnCount {
            return alignments
        }
        if alignments.count > columnCount {
            return Array(alignments.prefix(columnCount))
        }
        return alignments + Array(repeating: .leading, count: columnCount - alignments.count)
    }
}

private struct ChatCoreTaskProjectionView: View {

    let projection: AgentCoreTaskProjection
    let parentConversationID: String?
    var showsSummary = true
    let controls: [IOSConversationProjectionV1.TaskControlPart]
    let onOpenResult: (AppleTaskChildDestination) -> Void
    let onCancel: (String) -> Void
    let onSendGuidance: (String, String) -> Void
    @State private var guidanceDrafts: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(projection.orderedTasks) { task in
                VStack(alignment: .leading, spacing: 7) {
                    if showsSummary {
                    HStack(spacing: 8) {
                        Image(systemName: icon(for: task))
                            .foregroundStyle(color(for: task))
                        Text(title(for: task))
                            .font(.subheadline.weight(.semibold))
                        Spacer(minLength: 8)
                        Text(status(for: task.state))
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                    }
                    if let step = task.currentStep, !task.state.isTerminal {
                        Text(step)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let progress = task.progress, !task.state.isTerminal {
                        ProgressView(value: progress)
                            .tint(color(for: task))
                    }
                    }
                    if let pending = projection.guidanceQueue(for: task.id)?.pending.last {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(NSLocalizedString(
                                "chat.task.guidance.pending",
                                value: "Guidance pending",
                                comment: "Queued task guidance status"
                            ))
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.tint)
                            Text(pending.content)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(3)
                        }
                    }
                    if controls.contains(where: { $0.taskID == task.id && $0.supportsGuidance && $0.parentRunID?.isEmpty == false }),
                       task.kind == .agent,
                       !task.state.isTerminal,
                       !task.cancelRequested {
                        HStack(spacing: 8) {
                            TextField(
                                NSLocalizedString(
                                    "chat.task.guidance.placeholder",
                                    value: "Guide this task",
                                    comment: "Task guidance input placeholder"
                                ),
                                text: guidanceBinding(for: task.id)
                            )
                            .font(.caption)
                            .textFieldStyle(.roundedBorder)
                            .submitLabel(.send)
                            .onSubmit { sendGuidance(task.id) }

                            Button {
                                sendGuidance(task.id)
                            } label: {
                                Image(systemName: "arrow.up.circle.fill")
                                    .frame(minWidth: 44, minHeight: 44)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.borderless)
                            .disabled(guidanceText(for: task.id).isEmpty)
                            .accessibilityLabel(NSLocalizedString(
                                "chat.task.guidance.send",
                                value: "Send guidance",
                                comment: "Send task guidance"
                            ))
                        }
                    }
                    HStack(spacing: 12) {
                        if task.kind == .agent, let child = task.lineage?.childSessionID,
                           !child.isEmpty, let parentConversationID,
                           let parentRunID = controls.first(where: { $0.taskID == task.id })?.parentRunID,
                           !parentRunID.isEmpty {
                            Button {
                                onOpenResult(AppleTaskChildDestination(parentConversationID: parentConversationID,
                                    parentRunID: parentRunID, taskID: task.id, childConversationID: child))
                            } label: {
                                Label(
                                    NSLocalizedString("chat.task.open_result", value: "Open result", comment: "Open Core task result"),
                                    systemImage: "arrow.up.right.square"
                                )
                                .frame(minHeight: 44)
                                .contentShape(Rectangle())
                            }
                            .font(.caption.weight(.medium))
                            .accessibilityIdentifier("chat.task.openResult.\(task.id)")
                        }
                        Spacer(minLength: 0)
                        if controls.contains(where: { $0.taskID == task.id && $0.supportsCancellation && $0.parentRunID?.isEmpty == false }),
                           !task.state.isTerminal && !task.cancelRequested {
                            Button {
                                onCancel(task.id)
                            } label: {
                                Text(NSLocalizedString("chat.task.cancel", value: "Cancel", comment: "Cancel Core task"))
                                    .frame(minWidth: 44, minHeight: 44)
                                    .contentShape(Rectangle())
                            }
                            .font(.caption.weight(.medium))
                            .buttonStyle(.borderless)
                        }
                    }
                }
                .padding(11)
                .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("chat.task.\(task.id)")
            }
        }
    }

    private func title(for task: AgentCoreTaskHandle) -> String {
        switch task.kind {
        case .agent: return NSLocalizedString("chat.task.agent", value: "Background agent", comment: "Core task kind")
        case .tool: return NSLocalizedString("chat.task.tool", value: "Background tool", comment: "Core task kind")
        case .skill: return NSLocalizedString("chat.task.skill", value: "Skill task", comment: "Core task kind")
        case .dataRefresh: return NSLocalizedString("chat.task.data_refresh", value: "Data refresh", comment: "Core task kind")
        }
    }

    private func status(for state: AgentCoreTaskState) -> String {
        if state == .sealed { return NSLocalizedString("chat.task.state.sealed", value: "Execution sealed", comment: "Execution ended, not business acceptance") }
        return NSLocalizedString("chat.task.state.\(state.rawValue)", value: state.rawValue.capitalized, comment: "Core task state")
    }

    private func icon(for task: AgentCoreTaskHandle) -> String {
        if task.state == .sealed { return "archivebox" }
        if task.state == .completed { return "checkmark.circle.fill" }
        if task.state == .failed || task.state == .cancelled { return "xmark.circle.fill" }
        switch task.kind {
        case .agent: return "person.crop.circle.badge.clock"
        case .tool: return "gearshape.2"
        case .skill: return "wand.and.stars"
        case .dataRefresh: return "arrow.clockwise"
        }
    }

    private func color(for task: AgentCoreTaskHandle) -> Color {
        switch task.state {
        case .sealed: return .secondary
        case .completed: return .green
        case .failed: return .red
        case .cancelled: return .secondary
        case .suspended, .retryWaiting: return .orange
        case .queued, .running, .waiting: return .accentColor
        }
    }

    private func guidanceBinding(for taskId: String) -> Binding<String> {
        Binding(
            get: { guidanceDrafts[taskId] ?? "" },
            set: { guidanceDrafts[taskId] = $0 }
        )
    }

    private func guidanceText(for taskId: String) -> String {
        (guidanceDrafts[taskId] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func sendGuidance(_ taskId: String) {
        let content = guidanceText(for: taskId)
        guard !content.isEmpty, controls.contains(where: { $0.taskID == taskId && $0.supportsGuidance }),
              let task = projection.tasks[taskId], !task.state.isTerminal, !task.cancelRequested else { return }
        guidanceDrafts[taskId] = ""
        onSendGuidance(taskId, content)
    }
}

nonisolated func resolveActiveHostPermissionEventID(
    requestID: String?,
    waitingEventIDs: [String],
    permissionOwners: [(requestID: String, toolCallID: String)]
) -> String? {
    guard let requestID else { return nil }
    let owners = permissionOwners.filter { $0.requestID == requestID }
    guard owners.count == 1, let owner = owners.first,
          waitingEventIDs.contains(owner.toolCallID) else { return nil }
    return owner.toolCallID
}

/// One stable process owner preserves the journal's reasoning/tool order.
/// Approval controls deliberately remain outside this collapsible surface.
nonisolated struct IOSMessagePartReadingKey: Hashable {
    let source: String
    let conversation: String
    let message: String
    let part: String
}

enum ChatExecutionProcessPolicy {
    static func isTerminal(_ state: IOSConversationProjectionV1.ActivityState) -> Bool {
        switch state { case .completed, .sealed, .cancelled, .failed: true; default: false }
    }

    static func activityTitle(_ state: IOSConversationProjectionV1.ActivityState) -> String {
        let key: String
        switch state {
        case .pending: key = "chat.host_tool.status.pending"
        case .running: key = "chat.host_tool.status.running"
        case .waitingForPermission: key = "chat.host_tool.status.permission_required"
        case .waitingForInput: key = "chat.host_tool.status.waiting_input"
        case .completed: key = "chat.host_tool.status.succeeded"
        case .sealed: key = "chat.task.state.sealed"
        case .cancelled: key = "chat.host_tool.status.cancelled"
        case .failed: key = "chat.step.failed"
        }
        return NSLocalizedString(key, comment: "Canonical activity status")
    }

    static func displayTitle(_ displayKey: String, fallbackKey: String) -> String {
        let missing = "__nola_missing_activity_translation__"
        let value = Bundle.main.localizedString(forKey: displayKey, value: missing, table: nil)
        return value == missing ? NSLocalizedString(fallbackKey, comment: "Canonical activity") : value
    }

    static func errorTitle(displayKey: String, localize: (String) -> String? = { key in
        let missing = "__nola_missing_step_error_translation__"
        let value = Bundle.main.localizedString(forKey: key, value: missing, table: nil)
        return value == missing ? nil : value
    }) -> String {
        localize(displayKey) ?? localize("chat.step.failed") ?? "This step failed"
    }

    static func taskControlShowsSummary(taskID: String, parts: [IOSConversationProjectionV1.Part]) -> Bool {
        !parts.contains { part in
            if case .task(_, _, let existingID, _, _, _, _, _, _) = part { return existingID == taskID }
            return false
        }
    }

    static func segments(_ parts: [IOSConversationProjectionV1.Part]) -> [AppleMessagePartFlowSegment<IOSConversationProjectionV1.Part>] {
        AppleMessagePartFlow.segments(parts, id: { $0.partID }, kind: {
            switch $0 { case .reasoning: .reasoning; case .tool: .tool; default: .content }
        })
    }

    static func segmentExpanded(choices: [Bool], liveReasoning: Bool) -> Bool {
        AppleMessagePartFlow.isExpanded(choices: choices, defaultExpanded: liveReasoning)
    }

    static func liveReasoning(_ parts: [IOSConversationProjectionV1.Part], live: Bool) -> [IOSConversationProjectionV1.Part] {
        guard live else { return [] }
        // Only the currently running reasoning defaults open. Explicit reader
        // choices remain owned by the message even when segments regroup.
        return parts.filter {
            guard case .reasoning(let reasoning) = $0 else { return false }
            return reasoning.state == .running && !(reasoning.summary ?? "").isEmpty
        }
    }


}

private struct ChatCanonicalExecutionProcess: View {
    let parts: [IOSConversationProjectionV1.Part]
    let events: [HostAgentToolEvent]
    let activities: [AgentTraceActivity]
    let isLive: Bool
    @Binding var expanded: Bool
    let toolExpanded: (String) -> Binding<Bool>
    let technicalExpanded: (String) -> Binding<Bool>
    let reader: (String) -> Binding<HostToolTextSnapshot?>

    private var containsTools: Bool {
        parts.contains { if case .tool = $0 { true } else { false } }
    }

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(parts, id: \.partID) { part in
                    switch part {
                    case .reasoning(let reasoning):
                        if !(reasoning.summary ?? "").isEmpty {
                            ChatSmoothStreamingMarkdownView(content: reasoning.summary ?? "",
                                isStreaming: isLive && reasoning.state == .running)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled).transaction { $0.animation = nil }
                                .accessibilityIdentifier("chat.assistant.thinking")
                        }
                    case .tool(let tool):
                        if let event = events.first(where: { $0.id == tool.toolCallID })
                            ?? IOSConversationProjectionPresenter.toolEvent(part) {
                            HostToolEventCard(event: event,
                                activity: activities.first { $0.spanId == tool.toolCallID }, canonicalTool: tool,
                                readingExpansion: toolExpanded(part.partID),
                                technicalReadingExpansion: technicalExpanded(part.partID),
                                readingSelection: reader(part.partID))
                        }
                    default: EmptyView()
                    }
                }
            }.padding(.top, 8)
        } label: {
            Label(containsTools ? NSLocalizedString("chat.execution_process", value: "Execution process", comment: "Ordered reasoning and tools") : L10n.chatThinkingProcess,
                systemImage: containsTools ? "list.bullet" : "sparkles")
                .font(.callout.weight(.medium)).foregroundStyle(.secondary)
                .frame(minHeight: 44, alignment: .leading)
                .accessibilityIdentifier("chat.execution.process.toggle")
        }
        .disclosureGroupStyle(NolaRowDisclosureStyle())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.execution.process")
    }
}

private struct HostToolEventsView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let events: [HostAgentToolEvent]
    let activities: [AgentTraceActivity]
    let canonicalTools: [IOSConversationProjectionV1.ToolPart]
    let pendingPermissionRequest: HostAgentPermissionRequest?
    let onPermissionDecision: (String, HostAgentPermissionDecision) -> Void
    @State private var expanded: Bool

    init(
        events: [HostAgentToolEvent],
        activities: [AgentTraceActivity],
        canonicalTools: [IOSConversationProjectionV1.ToolPart] = [],
        pendingPermissionRequest: HostAgentPermissionRequest?,
        onPermissionDecision: @escaping (String, HostAgentPermissionDecision) -> Void,
        expandedByDefault: Bool
    ) {
        self.events = events
        self.activities = activities
        self.canonicalTools = canonicalTools
        self.pendingPermissionRequest = pendingPermissionRequest
        self.onPermissionDecision = onPermissionDecision
        _expanded = State(initialValue: expandedByDefault)
    }

    private var visibleCanonicalTools: [IOSConversationProjectionV1.ToolPart] {
        canonicalTools.filter { tool in events.contains { $0.id == tool.toolCallID } }
    }

    private var activityStatus: String {
        let permissionRequired = events.filter { $0.status == .permissionRequired }.count
        let running = events.filter { event in
            event.status == .running && canonicalTools.first(where: { $0.toolCallID == event.id }).map { $0.state == .running } != false
        }.count
        let failed = events.filter { $0.status == .failed }.count
        let cancelled = events.filter { $0.status == .cancelled }.count
        if permissionRequired > 0 {
            return NSLocalizedString(
                "chat.host_tool.status.permission_required",
                value: "Confirmation required",
                comment: "Tool permission confirmation status"
            )
        }
        if visibleCanonicalTools.contains(where: { $0.state == .waitingForInput }) {
            return NSLocalizedString("chat.host_tool.status.waiting_input", value: "Input required", comment: "Tool waits for user input")
        }
        if running > 0 {
            return String(format: NSLocalizedString("chat.tool_activity.running_format", value: "%d running", comment: "Running tool count"), running)
        }
        if visibleCanonicalTools.contains(where: { $0.state == .pending }) {
            return NSLocalizedString("chat.host_tool.status.pending", value: "Pending", comment: "Tool is queued")
        }
        if failed > 0 {
            return String(format: NSLocalizedString("chat.tool_activity.failed_format", value: "%d failed", comment: "Failed tool count"), failed)
        }
        if cancelled > 0 {
            return String(format: NSLocalizedString("chat.tool_activity.cancelled_format", value: "%d cancelled", comment: "Cancelled tool count"), cancelled)
        }
        let sealed = visibleCanonicalTools.filter { $0.state == .sealed }.count
        if sealed == events.count && sealed > 0 {
            return NSLocalizedString("chat.host_tool.status.sealed", value: "Sealed", comment: "Activity is sealed, not necessarily successful")
        }
        return String(format: NSLocalizedString("chat.tool_activity.completed_format", value: "%d completed", comment: "Completed tool count"), events.count - sealed)
    }

    private var activityTint: Color {
        if events.contains(where: { $0.status == .permissionRequired }) { return .orange }
        if events.contains(where: { $0.status == .running }) { return DesignTokens.primaryColor }
        if events.contains(where: { $0.status == .failed }) { return DesignTokens.dangerColor }
        if events.contains(where: { $0.status == .cancelled }) { return .secondary }
        return DesignTokens.successColor
    }

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(events) { event in
                    HostToolEventCard(
                        event: event,
                        activity: activities.first(where: { $0.spanId == event.id }),
                        canonicalTool: canonicalTools.first { $0.toolCallID == event.id }
                    )
                    .nolaDisclosureTransition(reduceMotion: reduceMotion)
                }
            }
            .padding(.top, 8)
        } label: {
            HStack(spacing: 8) {
                    Image(systemName: "wrench.and.screwdriver")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(DesignTokens.primaryColor)
                    Text(NSLocalizedString("chat.tool_activity", value: "Tool activity", comment: "Tool activity group title"))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.primary)
                    Spacer(minLength: 0)
                    Text(activityStatus)
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(activityTint)
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 6)
            .accessibilityValue(activityStatus)
            .accessibilityIdentifier("chat.assistant.tools.toggle")
        }
        .disclosureGroupStyle(NolaRowDisclosureStyle())
        .animation(reduceMotion ? nil : DesignTokens.disclosureAnimation, value: expanded)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.assistant.tools")

        // A current challenge stays actionable even when the tool group is
        // collapsed. Historical tool metadata is never approval authority.
        if let pendingPermissionRequest {
            HostPermissionDecisionCard(request: pendingPermissionRequest, onPermissionDecision: onPermissionDecision)
        }
    }
}

private struct HostToolScreenshotView: View {
    let screenshotURL: URL
    @State private var imageData: Data?
    @State private var loadFailed = false

    var body: some View {
        Link(destination: screenshotURL) {
            content
                .frame(maxHeight: 220)
                .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
        }
        .task(id: screenshotURL) {
            await loadImageIfNeeded()
        }
    }

    @ViewBuilder
    private var content: some View {
        if let imageData, let image = UIImage(data: imageData) {
            Image(uiImage: image)
                .resizable()
                .scaledToFit()
        } else if loadFailed {
            Label(NSLocalizedString("chat.host_tool.open_screenshot", value: "打开截图", comment: "Open screenshot label"), systemImage: "photo")
                .font(.caption)
        } else {
            HStack(spacing: 8) {
                ProgressView().scaleEffect(0.65)
                Text(NSLocalizedString("chat.host_tool.loading_screenshot", value: "加载截图…", comment: "Loading screenshot text"))
            }
            .font(.caption)
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard imageData == nil else { return }
        do {
            let selected = ModelSelectionManager.getLastSelectedModel()
            let pin = selected?.isWebBackend == true ? selected?.tlsCertificateSHA256 : nil
            imageData = try await WebAgentProvider.fetchHostResource(
                from: screenshotURL,
                pinnedCertificateSHA256: pin
            )
            loadFailed = false
        } catch {
            loadFailed = true
        }
    }
}

private struct HostToolEventCard: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let event: HostAgentToolEvent
    let activity: AgentTraceActivity?
    var canonicalTool: IOSConversationProjectionV1.ToolPart? = nil
    var readingExpansion: Binding<Bool>? = nil
    var technicalReadingExpansion: Binding<Bool>? = nil
    var readingSelection: Binding<HostToolTextSnapshot?>? = nil
    // Keep the action and summary readable without opening raw diagnostics.
    @State private var expanded: Bool = false
    @State private var technicalExpanded: Bool = false
    @State private var readerSelection: HostToolTextSnapshot?

    private var displayName: String {
        if let title = canonicalTool?.title { return title }
        return localizedHostToolDisplayName(event.name)
    }

    private var details: ChatToolDetailProjection {
        ChatToolDetailProjection(event: event, activity: activity, canonicalTool: canonicalTool)
    }

    private var iconName: String {
        if event.isBrowserTool { return "globe.badge.chevron.backward" }
        if event.isComputerTool { return "display.and.arrow.down" }
        return "wrench.and.screwdriver"
    }

    private var tint: Color {
        if canonicalTool?.state == .sealed || canonicalTool?.state == .pending { return .secondary }
        if canonicalTool?.state == .waitingForInput { return DesignTokens.warningColor }
        switch event.status {
        case .running: return DesignTokens.primaryColor
        case .succeeded: return DesignTokens.successColor
        case .failed: return DesignTokens.dangerColor
        case .cancelled: return .secondary
        case .permissionRequired: return DesignTokens.warningColor
        }
    }

    private var statusText: String {
        switch canonicalTool?.state {
        case .pending:
            return NSLocalizedString("chat.host_tool.status.pending", value: "Pending", comment: "Tool is queued")
        case .waitingForInput:
            return NSLocalizedString("chat.host_tool.status.waiting_input", value: "Input required", comment: "Tool waits for user input")
        case .sealed:
            return NSLocalizedString("chat.host_tool.status.sealed", value: "Sealed", comment: "Activity is sealed, not necessarily successful")
        default: break
        }
        switch event.status {
        case .running: return NSLocalizedString("chat.host_tool.status.running", value: "运行中", comment: "Host tool running status")
        case .succeeded: return NSLocalizedString("chat.host_tool.status.succeeded", value: "完成", comment: "Host tool succeeded status")
        case .failed:
            let reason = failureReason
            let prefix = NSLocalizedString("chat.host_tool.status.failed_prefix", value: "失败：", comment: "Host tool failed prefix")
            return reason.isEmpty
                ? NSLocalizedString("chat.host_tool.status.failed_no_reason", value: "失败：未返回原因", comment: "Host tool failed without reason")
                : "\(prefix)\(reason)"
        case .cancelled:
            return NSLocalizedString("chat.host_tool.status.cancelled", value: "Cancelled", comment: "Host tool cancelled status")
        case .permissionRequired: return NSLocalizedString("chat.host_tool.status.permission_required", value: "需要确认", comment: "Host tool permission required status")
        }
    }

    private var failureReason: String {
        guard event.status == .failed else { return "" }
        let candidates = [event.redactedDisplay, event.redactedText]
        for candidate in candidates {
            let normalized = candidate
                .replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard !normalized.isEmpty else { continue }
            let withoutPrefix = normalized
                .replacingOccurrences(of: "^(\\u5931\\u8d25|Failed)[：:]", with: "", options: .regularExpression)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return String(withoutPrefix.prefix(90))
        }
        return ""
    }

    private var summary: String {
        if let summary = canonicalTool?.summary { return summary }
        if isFilePayloadEvent {
            return resourceSummary.isEmpty ? displayName : resourceSummary
        }
        if !event.redactedDisplay.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return preview(event.redactedDisplay, limit: 180)
        }
        if let title = event.pageTitle, !title.isEmpty {
            return title
        }
        if let filename = event.downloadFilename, !filename.isEmpty {
            return filename
        }
        if !event.redactedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(event.redactedText.trimmingCharacters(in: .whitespacesAndNewlines).prefix(180))
        }
        if !event.redactedArgsPreview.isEmpty {
            return event.redactedArgsPreview
        }
        return displayName
    }

    private var isFilePayloadEvent: Bool {
        if !details.resources.isEmpty || !fileOperationSummary.isEmpty {
            return true
        }
        let normalized = event.name
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
        return [
            "read_file", "write_file", "edit_file", "apply_patch",
            "create_file", "delete_file", "move_file", "copy_file",
            "notebook", "document", "spreadsheet", "presentation",
        ].contains { normalized.contains($0) }
    }

    private var resourceSummary: String {
        let projected = details.resources
            .prefix(6)
            .map { "\($0.operationLabel) · \($0.displayName)" }
            .joined(separator: "\n")
        return projected.isEmpty ? fileOperationSummary : projected
    }

    private var fileOperationSummary: String {
        let source = event.redactedArgsPreview
        guard !source.isEmpty else { return "" }
        let pattern = #"(?i)(?:\"|\b)path(?:\"|\b)\s*[:=]\s*[\"']?([^,}\n\"']+)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return "" }
        let range = NSRange(source.startIndex..<source.endIndex, in: source)
        let paths = regex.matches(in: source, range: range).compactMap { match -> String? in
            guard match.numberOfRanges > 1,
                  let pathRange = Range(match.range(at: 1), in: source) else { return nil }
            return String(source[pathRange]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let uniquePaths = Array(NSOrderedSet(array: paths).compactMap { $0 as? String }).prefix(6)
        guard !uniquePaths.isEmpty else { return "" }
        return uniquePaths.joined(separator: "\n")
    }

    private var downloadLabel: String {
        if let filename = event.downloadFilename?.trimmingCharacters(in: .whitespacesAndNewlines),
           !filename.isEmpty {
            return filename
        }
        return NSLocalizedString("chat.host_tool.open_download", value: "打开下载文件", comment: "Open download label")
    }

    @ViewBuilder private var canonicalDetails: some View {
        if let tool = canonicalTool, !event.containsSensitiveData {
            let origin = tool.origin
            Text([origin.kind, origin.providerTitle, origin.providerId, origin.namespace].compactMap { $0 }.joined(separator: " · "))
                .font(.caption2).foregroundStyle(.secondary)
            if let permission = tool.permission {
                Text([permission.permissionDomain, permission.decision, permission.rationale].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption2).foregroundStyle(.secondary)
            }
            if let started = tool.startedAtMs {
                Text("\(started) ms" + (tool.finishedAtMs.map { " → \($0) ms" } ?? ""))
                    .font(.caption2).foregroundStyle(.secondary)
            }
            // fullOutput is an owner descriptor only. No path, read or permission action.
        }
    }

    var body: some View {
        DisclosureGroup(isExpanded: readingExpansion ?? $expanded) {
            VStack(alignment: .leading, spacing: 12) {
                if let summary = canonicalTool?.summary, !event.containsSensitiveData {
                    Text(summary).font(.caption).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let tool = canonicalTool, !event.containsSensitiveData {
                    ForEach(Array(tool.artifacts.enumerated()), id: \.offset) { _, artifact in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(artifact.name).font(.caption2.weight(.semibold))
                            Text([artifact.operation, artifact.mimeType].compactMap { $0 }.joined(separator: " · "))
                                .font(.caption2).foregroundStyle(.secondary)
                            // Descriptors are not authority to open a host path.
                            Text(artifact.uri).font(.caption2.monospaced()).textSelection(.enabled)
                        }
                    }
                }
                if event.containsSensitiveData {
                    Label(NSLocalizedString("chat.host_tool.sensitive_result_hidden", value: "结果可能包含账号、OAuth、Token 或 Secret，敏感内容和截图已在 iOS 端隐藏。", comment: "Sensitive result hidden warning"), systemImage: "lock.shield")
                        .font(.caption2)
                        .foregroundColor(DesignTokens.warningColor)
                } else if let screenshotURL = event.screenshotURL {
                    if event.isComputerTool {
                        Label(NSLocalizedString("chat.host_tool.computer_screenshot_hint", value: "这是宿主机桌面的截图，不是当前 iPhone 屏幕。", comment: "Computer-use screenshot hint"), systemImage: "display")
                            .font(.caption2)
                            .foregroundColor(.secondary)
                    }
                    HostToolScreenshotView(screenshotURL: screenshotURL)
                }

                let screenshotWarning = event.redactedScreenshotWarning.trimmingCharacters(in: .whitespacesAndNewlines)
                if !screenshotWarning.isEmpty {
                    Label(
                        String(format: NSLocalizedString("chat.host_tool.screenshot_warning", value: "截图保存提示：%@", comment: "Screenshot warning"), screenshotWarning),
                        systemImage: "exclamationmark.triangle"
                    )
                    .font(.caption2)
                    .foregroundColor(DesignTokens.warningColor)
                }

                if !event.containsSensitiveData, let finalURL = event.finalURL {
                    Link(destination: finalURL) {
                        Label(finalURL.absoluteString, systemImage: "link")
                            .font(.caption)
                            .lineLimit(2)
                    }
                }

                if !event.containsSensitiveData, let downloadURL = event.downloadURL {
                    Link(destination: downloadURL) {
                        Label(downloadLabel, systemImage: "arrow.down.circle")
                            .font(.caption)
                            .lineLimit(2)
                    }
                }

                if !event.containsSensitiveData, !details.resources.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(NSLocalizedString("chat.host_tool.files", value: "Files", comment: "Files affected by a tool call"))
                            .font(.caption2.weight(.semibold))
                            .foregroundColor(.secondary)
                        ForEach(details.resources) { resource in
                            HStack(alignment: .top, spacing: 7) {
                                Image(systemName: resource.iconName)
                                    .font(.caption2)
                                    .foregroundStyle(tint)
                                    .frame(width: 14)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(resource.displayName)
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(.primary)
                                    Text(resource.operationLabel)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                    Text(resource.uri)
                                        .font(.caption2.monospaced())
                                        .foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                }
                            }
                        }
                    }
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous)
                            .fill(DesignTokens.rowFillColor)
                    )
                    .accessibilityIdentifier("chat.assistant.tool.resources.\(event.id)")
                } else if !event.containsSensitiveData, !fileOperationSummary.isEmpty {
                    toolTextBlock(
                        title: NSLocalizedString("chat.host_tool.files", value: "Files", comment: "Files affected by a tool call"),
                        text: fileOperationSummary,
                        identifier: "chat.assistant.tool.resources.\(event.id)"
                    )
                }

                if !event.containsSensitiveData {
                    if let tool = canonicalTool, let preview = tool.outputPreview,
                       tool.output != nil, preview != details.result {
                        toolTextBlock(
                            title: NSLocalizedString("chat.host_tool.preview", value: "Preview", comment: "Independent tool preview"),
                            text: preview,
                            identifier: "chat.assistant.tool.preview.\(event.id)"
                        )
                    }
                    if !details.result.isEmpty {
                        toolTextBlock(
                            title: NSLocalizedString("chat.host_tool.result", value: "结果", comment: "Host tool result title"),
                            text: details.result,
                            identifier: "chat.assistant.tool.result.\(event.id)"
                        )
                    }
                }

                DisclosureGroup(isExpanded: technicalReadingExpansion ?? $technicalExpanded) {
                    if technicalReadingExpansion?.wrappedValue ?? technicalExpanded {
                        canonicalDetails
                        if !event.containsSensitiveData, !details.arguments.isEmpty {
                            toolTextBlock(
                                title: NSLocalizedString("chat.host_tool.args", value: "参数", comment: "Host tool args title"),
                                text: details.arguments,
                                identifier: "chat.assistant.tool.arguments.\(event.id)"
                            )
                        }
                    }
                } label: {
                    Text(NSLocalizedString("scheduled.run_detail.technical.title", value: "Technical details", comment: "Technical details"))
                        .font(.caption).foregroundStyle(.secondary)
                        .accessibilityIdentifier("chat.assistant.tool.technical.\(event.id)")
                }
                .disclosureGroupStyle(NolaRowDisclosureStyle())
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 12)
        } label: {
            HStack(alignment: .top, spacing: 8) {
                if event.status == .running && (canonicalTool == nil || canonicalTool?.state == .running) && !reduceMotion {
                    ProgressView()
                        .scaleEffect(0.58)
                        .frame(width: 16, height: 16)
                } else {
                    Image(systemName: event.status == .permissionRequired ? "exclamationmark.shield" : iconName)
                        .font(.caption.weight(.semibold))
                        .foregroundColor(tint)
                        .frame(width: 16, height: 16)
                }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .top, spacing: 6) {
                        Text(displayName)
                            .font(.caption.weight(.semibold))
                        Text(statusText)
                            .font(.caption2.weight(.semibold))
                            .foregroundColor(tint)
                    }
                    Text(summary)
                        .font(.caption2)
                        .foregroundColor(.secondary)
                        .lineLimit((readingExpansion?.wrappedValue ?? expanded) ? 4 : 2)
                }
                Spacer(minLength: 0)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("chat.assistant.tool.\(event.id)")
        }
        .disclosureGroupStyle(NolaRowDisclosureStyle())
        .padding(.leading, 14)
        .padding(.trailing, 4)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.rowFillColor.opacity(0.46))
        )
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 1, style: .continuous)
                .fill(tint.opacity(0.78))
                .frame(width: 2)
                .padding(.vertical, 8)
        }
        .animation(reduceMotion ? nil : DesignTokens.disclosureAnimation, value: readingExpansion?.wrappedValue ?? expanded)
        .onChange(of: event.id) { _, _ in readerSelection = nil }
        .fullScreenCover(item: readingSelection == nil ? $readerSelection : .constant(nil)) { selection in
            HostToolTextReader(snapshot: selection, onClose: { readerSelection = nil })
        }
    }

    private func toolTextBlock(
        title: String,
        text: String,
        identifier: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.caption2.weight(.semibold))
                .foregroundColor(.secondary)
            if ChatToolTextPresentation.needsReader(text) {
                Button(NSLocalizedString("chat.tool_reader.open", comment: "Open text reader")) {
                    if let readingSelection { readingSelection.wrappedValue = HostToolTextSnapshot(title: title, text: text) }
                    else { readerSelection = HostToolTextSnapshot(title: title, text: text) }
                }
                .font(.caption)
                .frame(minHeight: 44)
                .accessibilityIdentifier(identifier + ".reader")
            }
            Text(ChatToolTextPresentation.preview(text))
                .font(.caption.monospaced())
                .foregroundColor(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous)
                .fill(DesignTokens.rowFillColor)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(identifier)
    }

    private func preview(_ text: String, limit: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > limit else { return trimmed }
        return String(trimmed.prefix(limit)) + "\n…"
    }
}

private struct IOSMessageToolReaderSelection: Identifiable {
    let key: IOSMessagePartReadingKey
    let snapshot: HostToolTextSnapshot
    var id: UUID { snapshot.id }
}

private struct HostToolTextSnapshot: Identifiable {
    let id = UUID()
    let title: String
    let text: String
}

private struct HostToolTextReader: View {
    let snapshot: HostToolTextSnapshot
    let onClose: () -> Void

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text(NSLocalizedString("chat.tool_reader.snapshot", comment: "Text reader snapshot notice"))
                        .font(.caption).foregroundStyle(.secondary)
                    Text(verbatim: snapshot.text)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding()
            }
            .navigationTitle(snapshot.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(NSLocalizedString("common.close", comment: "Close"), action: onClose)
                }
                ToolbarItem(placement: .primaryAction) {
                    Button(NSLocalizedString("common.copy", comment: "Copy")) { UIPasteboard.general.string = snapshot.text }
                }
            }
        }
        .accessibilityIdentifier("chat.assistant.tool.reader")
    }
}

enum ChatToolTextPresentation {
    static let previewLimit = 1200
    static let previewLineLimit = 16

    static func needsReader(_ text: String) -> Bool {
        text.count > previewLimit || text.split(separator: "\n", omittingEmptySubsequences: false).count > previewLineLimit
    }

    static func preview(_ text: String) -> String {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        let excerpt = String(lines.prefix(previewLineLimit).joined(separator: "\n").prefix(previewLimit))
        return needsReader(text) ? excerpt + "\n…" : text
    }
}

private struct SubmittingHostPermissionRequestKey: EnvironmentKey {
    static let defaultValue: String? = nil
}
extension EnvironmentValues {
    var submittingHostPermissionRequestID: String? {
        get { self[SubmittingHostPermissionRequestKey.self] }
        set { self[SubmittingHostPermissionRequestKey.self] = newValue }
    }
}

struct HostPermissionDecisionCard: View {
    @Environment(\.submittingHostPermissionRequestID) private var submittingRequestID
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let request: HostAgentPermissionRequest
    let onPermissionDecision: (String, HostAgentPermissionDecision) -> Void
    @State private var permissionInstruction = ""

    var body: some View {
        permissionDecisionCard(request)
            .disabled(submittingRequestID == request.id)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("chat.permission.tool.\(request.toolName)")
            .onChange(of: request) { _, _ in permissionInstruction = "" }
    }

    private func permissionDecisionCard(_ request: HostAgentPermissionRequest) -> some View {
        let isMcp = request.originKind == "mcp"
        let isHost = request.hostId?.isEmpty == false || request.nonce?.isEmpty == false
        let tint = isHost ? DesignTokens.warningColor : DesignTokens.successColor
        return VStack(alignment: .leading, spacing: 10) {
            Label(
                isMcp ? String(localized: "mcp.permission.http_server") : request.deviceBridge == true
                    ? NSLocalizedString("chat.permission.hybrid.title", value: "宿主机请求此设备", comment: "Hybrid device permission title")
                    : isHost
                    ? NSLocalizedString("chat.permission.host.title", value: "在宿主机上执行", comment: "Host permission title")
                    : NSLocalizedString("chat.permission.device.title", value: "在此设备上执行", comment: "Device permission title"),
                systemImage: request.deviceBridge == true ? "arrow.triangle.2.circlepath" : (isHost ? "desktopcomputer" : "iphone")
            )
                .font(.caption.weight(.semibold))
                .foregroundColor(tint)
            Text(isMcp ? String(localized: "mcp.mobile.http_execution") : request.deviceBridge == true
                ? NSLocalizedString("chat.permission.hybrid.hint", value: "操作在此设备执行，结果将返回当前宿主机会话。", comment: "Hybrid permission hint")
                : isHost
                ? NSLocalizedString("chat.permission.host.hint", value: "工具将在已连接的宿主机上运行，结果返回当前对话。", comment: "Host permission hint")
                : NSLocalizedString("chat.permission.device.hint", value: "工具只访问这台设备上由系统允许的数据。", comment: "Device permission hint"))
                .font(.caption2)
                .foregroundColor(.secondary)
            VStack(spacing: 6) {
                permissionDetail(
                    icon: "location",
                    title: NSLocalizedString("chat.permission.location", value: "执行位置", comment: "Permission execution location"),
                    value: isMcp ? String(localized: "mcp.permission.http_server") : request.deviceBridge == true
                        ? NSLocalizedString("chat.permission.this_device", value: "此设备", comment: "This device permission value")
                        : (isHost ? shortHostIdentity(request.hostId) : NSLocalizedString("chat.permission.this_device", value: "此设备", comment: "This device permission value"))
                )
                permissionDetail(
                    icon: "arrow.left.arrow.right",
                    title: NSLocalizedString("chat.permission.data_boundary", value: "数据边界", comment: "Permission data boundary"),
                    value: isMcp ? String(localized: "mcp.permission.server_and_model") : request.deviceBridge == true
                        ? NSLocalizedString("chat.permission.current_session", value: "仅当前宿主机会话", comment: "Hybrid session boundary")
                        : isHost
                        ? NSLocalizedString("chat.permission.current_session", value: "仅当前会话", comment: "Current session permission boundary")
                        : NSLocalizedString("chat.permission.device_only", value: "仅此设备", comment: "Device only permission boundary")
                )
                permissionDetail(
                    icon: "1.circle",
                    title: NSLocalizedString("chat.permission.validity", value: "有效期", comment: "Permission validity"),
                    value: request.deviceBridge == true
                        ? NSLocalizedString("chat.permission.once", value: "仅本次操作", comment: "Allow once permission validity")
                        : NSLocalizedString("chat.permission.validity.choice", value: "可选择本次或本对话", comment: "Permission validity choice")
                )
            }
            Text(permissionMessage(for: request))
                .font(.caption2)
                .foregroundColor(.secondary)
                .textSelection(.enabled)
            if !request.argsPreview.isEmpty || !request.match.isEmpty {
                DisclosureGroup {
                    if !request.match.isEmpty {
                        Text(HostAgentToolEvent.redactSensitiveContent(request.match))
                            .font(.caption.monospaced()).textSelection(.enabled)
                    }
                    Text(HostAgentToolEvent.redactSensitiveContent(request.argsPreview))
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                } label: {
                    Text(NSLocalizedString("chat.host_tool.args", value: "参数", comment: "Host tool args title"))
                        .font(.caption)
                        .frame(minHeight: 44)
                }
                .disclosureGroupStyle(NolaRowDisclosureStyle())
                .id(request.id)
            }
            Button {
                NolaHaptics.primaryAction()
                onPermissionDecision(request.id, .allowOnce)
            } label: {
                HStack {
                    if submittingRequestID == request.id { ProgressView() }
                    Text(permissionAllowTitle(for: request))
                }
                .foregroundStyle(DesignTokens.onAccentColor)
                .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.borderedProminent)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("chat.permission.allow_once")

            Group {
                if dynamicTypeSize.isAccessibilitySize {
                    VStack(spacing: 8) {
                        secondaryPermissionActions(request)
                    }
                } else {
                    HStack(spacing: 8) {
                        secondaryPermissionActions(request)
                    }
                }
            }
            .font(.caption.weight(.semibold))
            permissionGuidanceEditor(request)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous)
                .fill(tint.opacity(0.08))
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous)
                .stroke(tint.opacity(0.24), lineWidth: 1)
        )
    }

    private func permissionDetail(icon: String, title: String, value: String) -> some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 3) {
                    Label(title, systemImage: icon)
                        .foregroundColor(.secondary)
                    Text(value)
                        .fontWeight(.medium)
                }
            } else {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: icon)
                        .frame(width: 14)
                        .foregroundColor(.secondary)
                    Text(title)
                        .foregroundColor(.secondary)
                    Spacer(minLength: 8)
                    Text(value)
                        .fontWeight(.medium)
                        .multilineTextAlignment(.trailing)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .font(.caption2)
    }

    @ViewBuilder
    private func secondaryPermissionActions(_ request: HostAgentPermissionRequest) -> some View {
        Button(role: .cancel) {
            NolaHaptics.warning()
            onPermissionDecision(request.id, .deny)
        } label: {
            Text(NSLocalizedString("chat.host_tool.permission.reject", value: "拒绝", comment: "Reject host tool permission"))
                .frame(maxWidth: .infinity, minHeight: 44)
        }
        .buttonStyle(.bordered)
        .frame(maxWidth: .infinity)
        .accessibilityIdentifier("chat.permission.deny")
        if request.deviceBridge != true && request.allowConversation {
            Button {
                NolaHaptics.success()
                onPermissionDecision(request.id, .allowConversation)
            } label: {
                Text(NSLocalizedString("chat.host_tool.permission.allow_conversation", value: "本对话中允许", comment: "Allow matching operations in this conversation"))
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.bordered)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("chat.permission.allow_conversation")
        }
    }

    @ViewBuilder
    private func permissionGuidanceEditor(_ request: HostAgentPermissionRequest) -> some View {
        if request.deviceBridge != true {
            VStack(alignment: .leading, spacing: 6) {
                TextField(NSLocalizedString("chat.permission.instruction.placeholder", value: "Additional guidance", comment: "Permission guidance input"), text: $permissionInstruction, axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("chat.permission.instruction.input")
                Text(NSLocalizedString("chat.permission.instruction.notice", value: "Guidance does not authorize or change tool arguments; the original call still needs explicit approval. Limit: 4 KiB per note, 16 notes and 16 KiB total. You can still approve or deny after reaching the limit.", comment: "Instruction does not authorize"))
                    .font(.caption2).foregroundColor(.secondary)
                Button(NSLocalizedString("chat.permission.instruction.submit", value: "Submit guidance and keep waiting", comment: "Submit permission guidance")) {
                    guard let response = HostAgentPermissionDecision.validatedInstruction(permissionInstruction, requestID: request.id, existing: request.permissionInstructions ?? []) else { return }
                    onPermissionDecision(request.id, response)
                }
                .disabled(HostAgentPermissionDecision.validatedInstruction(permissionInstruction, requestID: request.id, existing: request.permissionInstructions ?? []) == nil)
                .accessibilityIdentifier("chat.permission.instruction.submit")
            }

        }
    }

    private func shortHostIdentity(_ hostId: String?) -> String {
        guard let hostId, !hostId.isEmpty else {
            return NSLocalizedString("chat.permission.connected_host", value: "已连接宿主机", comment: "Connected host permission value")
        }
        return hostId.count > 18 ? String(hostId.prefix(18)) + "…" : hostId
    }

    private func permissionAllowTitle(for request: HostAgentPermissionRequest) -> String {
        switch request.toolName {
        case "device_read_file":
            return NSLocalizedString("chat.permission.action.read_file", value: "选择并读取文件", comment: "Read selected file permission action")
        case "device_write_file":
            return NSLocalizedString("chat.permission.action.write_file", value: "选择并写入文件", comment: "Write selected file permission action")
        case "device_clipboard_read":
            return NSLocalizedString("chat.permission.action.read_clipboard", value: "读取剪贴板", comment: "Read clipboard permission action")
        case "device_clipboard_write":
            return NSLocalizedString("chat.permission.action.write_clipboard", value: "更新剪贴板", comment: "Write clipboard permission action")
        case "device_get_location":
            return NSLocalizedString("chat.permission.action.location", value: "共享位置", comment: "Share location permission action")
        case "device_search_calendar_events":
            return NSLocalizedString("chat.permission.action.read_calendar", value: "共享日历事件", comment: "Read calendar permission action")
        case "device_create_calendar_event":
            return NSLocalizedString("chat.permission.action.create_calendar", value: "创建日程", comment: "Create calendar event permission action")
        default:
            return NSLocalizedString("chat.host_tool.permission.allow_once", value: "仅允许本次", comment: "Allow host tool permission once")
        }
    }

    private func permissionMessage(for request: HostAgentPermissionRequest) -> String {
        [
            HostAgentToolEvent.redactSensitiveContent(request.message.isEmpty ? request.reason : request.message),
            (request.permissionInstructions ?? []).isEmpty ? nil :
                NSLocalizedString("chat.permission.guidance", value: "Guidance recorded. Tool arguments are unchanged; explicit approval is still required.", comment: "Permission guidance is not authorization") + "\n" + (request.permissionInstructions ?? []).map(\.instruction).joined(separator: "\n"),
        ]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }

}

private func localizedHostToolDisplayName(_ rawName: String) -> String {
    let normalized = rawName
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .lowercased()
        .replacingOccurrences(of: "-", with: "_")
    let key: String
    let fallback: String
    switch normalized {
    case "bash", "shell", "run_command", "execute_command", "bash_command_review",
         "bash_background_status", "bash_background_wait", "bash_background_stop", "terminal_input":
        (key, fallback) = ("chat.host_tool.name.command", "Run command")
    case "read_file", "read_text_file", "read_many_files", "notebook_read", "file_outline", "read_symbol", "device_read_file":
        (key, fallback) = ("chat.host_tool.name.read_file", "Read file")
    case "write_file", "create_file", "device_write_file":
        (key, fallback) = ("chat.host_tool.name.write_file", "Write file")
    case "edit_file", "edit_file_lines", "multi_edit_file", "notebook_edit", "apply_patch", "replace_text",
         "replace_symbol_body", "replace_in_files", "rename_symbol", "format_files", "move_file":
        (key, fallback) = ("chat.host_tool.name.edit_file", "Edit file")
    case "delete_file", "remove_file":
        (key, fallback) = ("chat.host_tool.name.delete_file", "Delete file")
    case "list_dir", "list_directory", "file_tree", "find_files", "find_up":
        (key, fallback) = ("chat.host_tool.name.list_folder", "List folder")
    case "project_overview", "repo_map":
        (key, fallback) = ("chat.host_tool.name.project_overview", "Inspect project overview")
    case "codebase_search", "codebase_text_search", "codebase_trace", "codebase_read_symbol":
        (key, fallback) = ("chat.host_tool.name.search_files", "Search files")
    case "codebase_index", "codebase_index_status", "codebase_projects", "codebase_architecture",
         "codebase_query", "codebase_schema", "codebase_check_coverage", "codebase_changed_impact",
         "codebase_delete_index", "codebase_adr", "codebase_ingest_traces":
        (key, fallback) = ("chat.host_tool.name.workspace", "Inspect workspace")
    case "grep", "glob", "search_files", "ripgrep", "code_search", "workspace_symbols", "symbol_search",
         "implementation_search", "code_navigation", "language_service_query", "language_service_list", "call_graph", "failure_context", "failure_triage",
         "file_dependencies", "related_files", "document_search":
        (key, fallback) = ("chat.host_tool.name.search_files", "Search files")
    case "workspace_status", "code_diagnostics", "coding_conventions", "instruction_context",
         "agent_context", "command_context", "agent_config_context", "ci_context", "context_pack", "focused_context",
         "impact_analysis", "validation_plan", "test_context", "format_plan",
         "verification_review", "handoff_context", "workspace_checkpoint":
        (key, fallback) = ("chat.host_tool.name.workspace", "Inspect workspace")
    case "skill", "skill_context":
        (key, fallback) = ("chat.host_tool.name.skill", "Use skill")
    case "git_diff", "change_review", "diff_review", "changed_symbols", "git_history", "git_blame", "git_show":
        (key, fallback) = ("chat.host_tool.name.changes", "Inspect changes")
    case "web_search":
        (key, fallback) = ("chat.host_tool.name.web_search", "Search web")
    case "web_fetch", "fetch_url":
        (key, fallback) = ("chat.host_tool.name.web_read", "Read webpage")
    case "tool_search":
        (key, fallback) = ("chat.host_tool.name.search_tools", "Search tools")
    case "tool_output_list", "tool_output_read", "tool_output_search", "tool_output_find":
        (key, fallback) = ("chat.host_tool.name.read_output", "Read tool output")
    case "memory_save", "memory_search":
        (key, fallback) = ("chat.host_tool.name.memory", "Manage memory")
    case "spawn_agent", "nola_orchestrate":
        (key, fallback) = ("chat.host_tool.name.subtask", "Run subtask")
    case "list_tasks", "read_task", "wait_task", "stop_task":
        (key, fallback) = ("chat.host_tool.name.background", "Background task")
    case "propose_agent_plan", "update_agent_todos", "agent_plan_file":
        (key, fallback) = ("chat.host_tool.name.plan", "Update plan")
    case "system_time":
        (key, fallback) = ("chat.host_tool.name.time", "Read system time")
    case "device_clipboard_read", "device_clipboard_write":
        (key, fallback) = ("chat.host_tool.name.clipboard", "Use clipboard")
    case "device_search_calendar_events", "search_calendar_events":
        (key, fallback) = ("chat.host_tool.name.search_calendar", "Search calendar")
    case "device_create_calendar_event", "create_calendar_event":
        (key, fallback) = ("chat.host_tool.name.create_calendar_event", "Create calendar event")
    case "search_reminders":
        (key, fallback) = ("chat.host_tool.name.search_reminders", "Search reminders")
    case "create_reminder":
        (key, fallback) = ("chat.host_tool.name.create_reminder", "Create reminder")
    case "update_reminder":
        (key, fallback) = ("chat.host_tool.name.update_reminder", "Update reminder")
    case "device_get_location":
        (key, fallback) = ("chat.host_tool.name.location", "Read location")
    default:
        if normalized.hasPrefix("chrome_") || normalized.hasPrefix("browser_") {
            (key, fallback) = ("chat.host_tool.name.browser", "Control browser")
        } else if normalized.hasPrefix("computer_") {
            (key, fallback) = ("chat.host_tool.name.computer", "Control computer")
        } else if normalized.hasPrefix("skill_") {
            (key, fallback) = ("chat.host_tool.name.skill", "Use skill")
        } else if normalized.hasPrefix("mcp_") {
            (key, fallback) = ("chat.host_tool.name.mcp", "Call MCP")
        } else if normalized.hasPrefix("background_") {
            (key, fallback) = ("chat.host_tool.name.background", "Background task")
        } else if normalized.isEmpty {
            return NSLocalizedString("chat.host_tool.name.generic", value: "Tool", comment: "Generic tool display name")
        } else {
            return String(
                format: NSLocalizedString("chat.host_tool.name.unknown_format", value: "Tool · %@", comment: "Unknown tool display name"),
                rawName.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        }
    }
    return NSLocalizedString(key, value: fallback, comment: "Localized tool display name")
}

struct ChatToolDetailProjection: Equatable {
    struct Resource: Identifiable, Equatable {
        let uri: String
        let operation: AgentTraceResourceOperation
        let displayName: String

        var id: String { "\(operation.rawValue):\(uri)" }

        var iconName: String {
            switch operation {
            case .unknown: return "questionmark.circle"
            case .read: return "doc.text.magnifyingglass"
            case .create: return "doc.badge.plus"
            case .update: return "pencil"
            case .delete: return "trash"
            case .execute: return "terminal"
            case .open: return "arrow.up.right.square"
            case .search: return "magnifyingglass"
            }
        }

        var operationLabel: String {
            let key = "chat.host_tool.resource.operation.\(operation.rawValue)"
            return NSLocalizedString(
                key,
                value: operation.rawValue.capitalized,
                comment: "Operation performed on a tool resource"
            )
        }
    }

    let arguments: String
    let resources: [Resource]
    let result: String

    init(event: HostAgentToolEvent, activity: AgentTraceActivity?,
         canonicalTool: IOSConversationProjectionV1.ToolPart? = nil) {
        if let canonicalTool {
            arguments = HostAgentToolEvent.redactSensitiveContent(canonicalTool.input.displayText)
            result = HostAgentToolEvent.redactSensitiveContent(
                canonicalTool.output?.displayText ?? canonicalTool.outputPreview ?? ""
            )
            resources = canonicalTool.affectedResources.map { resource in
                Resource(uri: resource.uri,
                    operation: AgentTraceResourceOperation(rawValue: resource.operation) ?? .unknown,
                    displayName: URL(string: resource.uri)?.lastPathComponent.nonEmpty ?? resource.uri)
            }
            return
        }
        arguments = Self.displayText(activity?.arguments)
            .nonEmpty ?? event.redactedArgsPreview
        resources = (activity?.resources ?? []).map { resource in
            Resource(
                uri: resource.uri,
                operation: resource.operation,
                displayName: resource.displayName?.nonEmpty
                    ?? URL(string: resource.uri)?.lastPathComponent.nonEmpty
                    ?? resource.uri
            )
        }
        result = Self.displayText(activity?.result)
            .nonEmpty ?? event.redactedText
    }

    private static func displayText(_ value: AgentCoreJSONValue?) -> String {
        guard let value else { return "" }
        switch value {
        case .string(let text):
            return text
        case .number(let number):
            return String(number)
        case .signedInteger(let number):
            return String(number)
        case .unsignedInteger(let number):
            return String(number)
        case .bool(let bool):
            return bool ? "true" : "false"
        case .array(let values):
            return values.map { displayText($0) }.joined(separator: "\n")
        case .object(let values):
            if case .string(let preview)? = values["args_preview"] {
                return preview
            }
            if case .string(let text)? = values["text"] {
                return text
            }
            return values.keys.sorted().map { key in
                "\(key): \(displayText(values[key]))"
            }.joined(separator: "\n")
        case .null:
            return ""
        }
    }
}

private extension String {
    var nonEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

enum ChatResponseStatusPolicy {
    static func terminalState(_ metadata: [String: String]) -> IOSConversationProjectionV1.MessageState? {
        guard let raw = metadata[ChatConversationSurfaceMeta.stateKey],
              let state = IOSConversationProjectionV1.MessageState(rawValue: raw),
              state == .failed || state == .cancelled else { return nil }
        return state
    }
}

enum ChatThinkingDisclosurePolicy {
    static func initialExpanded(isFinished: Bool) -> Bool { !isFinished }

    static func expandedAfterStatusChange(current: Bool, isFinished: Bool, userChangedExpansion: Bool = false) -> Bool {
        // A new active round is visible by default, but never overrides an
        // explicit collapse. Completion preserves the reader's current choice.
        !isFinished && !userChangedExpansion ? true : current
    }
}

private struct ChatThinkingProcessView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let text: String
    let isFinished: Bool
    let isRetrieving: Bool
    @State private var isExpanded: Bool
    @State private var userChangedExpansion = false

    init(text: String, isFinished: Bool, isRetrieving: Bool) {
        self.text = text
        self.isFinished = isFinished
        self.isRetrieving = isRetrieving
        _isExpanded = State(initialValue: ChatThinkingDisclosurePolicy.initialExpanded(isFinished: isFinished))
    }

    private var title: String {
        if isFinished {
            return L10n.chatThinkingProcess
        }
        return L10n.chatThinkingShort
    }

    var body: some View {
        DisclosureGroup(isExpanded: Binding(
            get: { isExpanded },
            set: { isExpanded = $0; userChangedExpansion = true }
        )) {
            if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                HStack(spacing: 8) {
                    ProgressView()
                        .scaleEffect(0.65)
                    Text(L10n.chatAIThinking)
                        .font(.footnote)
                        .foregroundColor(.secondary)
                }
                .padding(.top, 6)
            } else {
                Text(text)
                    .font(.footnote)
                    .foregroundColor(.secondary)
                    .textSelection(.enabled)
                    .padding(.top, 6)
                    .transaction { transaction in
                        // Streaming reasoning should grow in place. It must not
                        // inherit the timeline's insertion/scroll animations.
                        transaction.animation = nil
                    }
            }
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 7) {
                    ChatThinkingStatusIcon(isFinished: isFinished)
                    Text(title)
                        .font(.footnote.weight(.semibold))
                        .foregroundColor(.secondary)
                    Spacer(minLength: 0)
                }


            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .disclosureGroupStyle(NolaRowDisclosureStyle())
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(isExpanded ? DesignTokens.rowFillColor : .clear)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(isExpanded ? DesignTokens.borderColor : .clear, lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.assistant.thinking")
        .animation(reduceMotion ? nil : DesignTokens.disclosureAnimation, value: isExpanded)
        .onChange(of: isFinished) { _, finished in
            // Completion updates the status, not the user's reading position.
            // Previously finished messages still initialize collapsed.
            isExpanded = ChatThinkingDisclosurePolicy.expandedAfterStatusChange(
                current: isExpanded, isFinished: finished, userChangedExpansion: userChangedExpansion)
        }
    }
}

private struct ChatThinkingStatusIcon: View {
    let isFinished: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if isFinished {
            // Finished reasoning is not evidence that the task succeeded.
            Image(systemName: "text.bubble")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 16, height: 16)
        } else {
            TimelineView(.animation(minimumInterval: 1.0 / 20, paused: reduceMotion)) { context in
                let phase = reduceMotion ? 1.0 : (sin(context.date.timeIntervalSinceReferenceDate * .pi / 1.2) + 1) / 2
                Circle()
                    .fill(DesignTokens.primaryColor.opacity(0.65 + phase * 0.35))
                    .frame(width: 8, height: 8)
                    .frame(width: 16, height: 16)
            }
        }
    }
}
