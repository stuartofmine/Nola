// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents the Chat tab conversation list and per-owner activity.
// - Interfaces:
//   - ChatConversationsList
// - APIs:
//   - none
//
import SwiftUI
import UIKit

private struct ConversationFilterBounds: PreferenceKey {
    static var defaultValue: [Bool: Anchor<CGRect>] { [:] }

    static func reduce(value: inout [Bool: Anchor<CGRect>], nextValue: () -> [Bool: Anchor<CGRect>]) {
        value.merge(nextValue(), uniquingKeysWith: { _, latest in latest })
    }
}

nonisolated enum IOSConversationTitlePresentationPolicy {
    static func displayTitle(
        _ title: String?,
        conversationID: String?,
        fallback: String
    ) -> String {
        let normalized = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !isRuntimePlaceholder(normalized, conversationID: conversationID) else {
            return fallback
        }
        return normalized
    }

    static func isRuntimePlaceholder(_ title: String, conversationID: String?) -> Bool {
        let normalized = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return true }
        if ["New conversation", "New Conversation", "新对话"].contains(normalized) {
            return true
        }
        if UUID(uuidString: normalized) != nil || isLegacySessionPlaceholder(normalized) {
            return true
        }
        guard let conversationID else { return false }
        let canonicalID = conversationID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !canonicalID.isEmpty else { return false }
        let runtimeFallback = String(
            format: String(localized: "Conversation %@"),
            String(canonicalID.prefix(8))
        )
        let canonicalFallback = "Conversation \(canonicalID.prefix(8))"
        return normalized == canonicalID
            || normalized == canonicalFallback
            || normalized == runtimeFallback
    }

    private static func isLegacySessionPlaceholder(_ value: String) -> Bool {
        guard value.hasPrefix("Session ") else { return false }
        let suffix = value.dropFirst("Session ".count)
        return suffix.count == 8 && suffix.allSatisfy(\.isHexDigit)
    }
}

nonisolated struct IOSConversationListPreview: Sendable {
    let lastActiveAt: Date
    let revision: Int64
    let checkedOnVisit: Int
    var hasCurrentMessage = false
    var draft: String?
    var message = ""
    var hasRoom = false
    var experts: [String] = []
    var error: String?

    func matches(_ query: String) -> Bool {
        message.localizedStandardContains(query) || draft?.localizedStandardContains(query) == true
            || experts.contains { $0.localizedStandardContains(query) }
    }

    // Search covers the whole directory even when its rows have never appeared.
    static func missing(in entries: [IOSConversationDirectoryEntry],
                        cache: [UUID: Self], visit: Int) -> [IOSConversationDirectoryEntry] {
        entries.filter { cache[$0.id]?.checkedOnVisit != visit }
    }

    static func read(_ entry: IOSConversationDirectoryEntry, runtime: IOSApplicationRuntime,
                     previous: Self?, visit: Int) async -> Self {
        var result = Self(lastActiveAt: entry.lastActiveAt, revision: entry.revision, checkedOnVisit: visit)
        do {
            if entry.canonicalID == entry.id.uuidString {
                result.draft = try FileConversationDraftRepository.scoped(to: runtime.databaseURL).textPreview(.conversation(entry.id))
            }
        } catch {
            result.draft = previous?.draft
            result.error = error.localizedDescription
        }
        do {
            let association = try IOSAgentRoomService(runtime: runtime).forConversation(entry.canonicalID)
            result.hasRoom = association != nil
            result.experts = association?.room.room.participants.filter(\.active).map(\.displayName) ?? []
        } catch {
            // A transient association read must not remove an existing disclosure.
            result.hasRoom = previous?.hasRoom ?? false
            result.experts = previous?.experts ?? []
        }
        // Drafts and room membership have independent lifetimes. The message
        // projection can be reused only after a successful read of this revision.
        if let previous, previous.lastActiveAt == entry.lastActiveAt,
           previous.revision == entry.revision, previous.hasCurrentMessage {
            result.message = previous.message
            result.hasCurrentMessage = true
            return result
        }
        do {
            let window = try runtime.loadConversationProjectionWindowV1(id: entry.canonicalID,
                beforeOrdinal: nil, limit: 2, contentByteLimit: 4096)
            let text = await MainActor.run {
                IOSConversationProjectionPresenter.messages(from: window.projection)
                    .last { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }?.content ?? ""
            }
            result.message = text.components(separatedBy: .newlines).map { line in
                (try? AttributedString(markdown: line)).map { String($0.characters) } ?? line
            }.joined(separator: " ")
            result.hasCurrentMessage = true
        } catch {
            result.message = previous?.message ?? ""
            result.error = error.localizedDescription
        }
        return result
    }
}

/// The Chat tab's root list. It owns search/scroll state while a conversation is pushed.
struct ChatConversationsList: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @AppStorage(AppleAppearancePreference.storageKey) private var appearance = "system"
    let onSelect: (UUID) -> Void
    var onContinueExpert: (IOSExpertHistoryContinuation) -> Void = { _ in }
    @State private var expandedParents: Set<UUID> = []
    @State private var conversations: [IOSConversationDirectoryEntry] = []
    private var pageInset: CGFloat { horizontalSizeClass == .regular ? 24 : 20 }
    @State private var previews: [UUID: IOSConversationListPreview] = [:]
    @State private var directorySnapshot: [IOSCanonicalConversationSummary]?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Binding var search: String

    init(search: Binding<String>, onSelect: @escaping (UUID) -> Void,
         onContinueExpert: @escaping (IOSExpertHistoryContinuation) -> Void = { _ in }) {
        self._search = search
        self.onSelect = onSelect
        self.onContinueExpert = onContinueExpert
    }
    @FocusState private var searchFocused: Bool
    @State private var pendingDelete: IOSConversationDirectoryEntry?
    @State private var renameTarget: (id: UUID, baseline: String)?
    @State private var renameDraft = ""
    @State private var directoryRequest: UUID?
    @State private var loadError: String?
    @State private var previewRefreshRevision = 0
    @State private var needsResponseOnly = false
    @State private var persistedWaitingIDs: Set<UUID> = []

    private struct PreviewLoadIdentity: Hashable {
        let lastActiveAt: Date
        let refreshRevision: Int
        let searching: Bool
    }

    private var query: String { search.trimmingCharacters(in: .whitespacesAndNewlines) }

    private struct SearchIndexIdentity: Hashable {
        let active: Bool
        let revision: Int
    }

    private var visibleConversations: [IOSConversationDirectoryEntry] {
        let candidates = conversations.filter { !needsResponseOnly || waiting($0.id) }
        let term = query
        guard !term.isEmpty else { return candidates }
        return candidates.filter {
            title($0).localizedStandardContains(term) || previews[$0.id]?.matches(term) == true
        }
    }

    var body: some View {
        List {
            listHeader
                .listRowInsets(EdgeInsets())
                .listRowSeparator(.hidden)
                .listRowBackground(DesignTokens.backgroundColor)
            if let loadError {
                Section {
                    Text(loadError).foregroundStyle(.secondary)
                    Button(String(localized: "common.retry")) { Task { await refresh() } }
                }.accessibilityIdentifier("chat.history.error")
            }
            if directorySnapshot == nil && loadError == nil {
                ProgressView().frame(maxWidth: .infinity).listRowSeparator(.hidden)
            } else if conversations.isEmpty && query.isEmpty && !needsResponseOnly && loadError == nil {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "chat.history.empty.title")).font(.headline)
                    Text(String(localized: "chat.history.empty.message")).foregroundStyle(.secondary)
                }.padding(.vertical, 20).listRowSeparator(.hidden).listRowBackground(DesignTokens.backgroundColor)
            } else if loadError == nil && visibleConversations.isEmpty {
                if !query.isEmpty && !IOSConversationListPreview.missing(in: conversations, cache: previews, visit: previewRefreshRevision).isEmpty {
                    ProgressView().frame(maxWidth: .infinity).listRowSeparator(.hidden)
                } else {
                    Text(String(localized: "chat.history.no_results")).foregroundStyle(.secondary).padding(.vertical, 20).listRowBackground(DesignTokens.backgroundColor)
                }
            }
            Section {
                ForEach(visibleConversations, id: \.id) { conversation in
                    conversationRow(conversation)
                }
            }

        }
        .listStyle(.plain)
        .modifier(IOSWorkspaceTopEdge())
        .scrollContentBackground(.hidden)
        .background(DesignTokens.backgroundColor)
        .accessibilityIdentifier("chat.conversations.list")
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(horizontalSizeClass == .compact ? .hidden : .visible, for: .navigationBar)
        .toolbarBackground(.hidden, for: .navigationBar)
        .scrollDismissesKeyboard(.interactively)
        .task { await refresh() }
        .refreshable { await refresh(forcePreviews: true) }
        .task(id: SearchIndexIdentity(active: !query.isEmpty, revision: previewRefreshRevision)) {
            guard !query.isEmpty else { return }
            for entry in IOSConversationListPreview.missing(in: conversations, cache: previews, visit: previewRefreshRevision) {
                guard !Task.isCancelled else { return }
                await loadPreview(entry)
            }
        }
        .confirmationDialog(String(localized: "chat.history.delete.title"), isPresented: Binding(
            get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }
        ), presenting: pendingDelete) { conversation in
            Button(L10n.chatClearAction, role: .destructive) {
                pendingDelete = nil
                Task { await container.chatManager.clearConversation(conversation.id); await refresh() }
            }
            Button(L10n.cancel, role: .cancel) { pendingDelete = nil }
        }
        .alert(String(localized: "chat.conversations.rename", defaultValue: "Rename Conversation"), isPresented: Binding(
            get: { renameTarget != nil }, set: { if !$0 { renameTarget = nil } }
        )) {
            TextField(String(localized: "chat.conversations.title_field", defaultValue: "Title"), text: $renameDraft)
            Button(L10n.cancel, role: .cancel) { renameTarget = nil }
            Button(L10n.done) {
                guard let target = renameTarget else { return }
                renameTarget = nil
                let trimmed = renameDraft.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty, trimmed != target.baseline else { return }
                do {
                    try container.conversationStore?.updateConversationTitle(conversationId: target.id, title: trimmed)
                    if container.chatManager.currentConversationId == target.id {
                        NotificationCenter.default.post(name: .nolaConversationRenamed, object: target.id, userInfo: ["title": trimmed])
                    }
                    Task { await refresh() }
                } catch { loadError = error.localizedDescription }
            }
        }
    }

    private func conversationRow(_ conversation: IOSConversationDirectoryEntry) -> some View {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(alignment: .center, spacing: 4) {
                            Button {
                                dismissSearchKeyboard()
                                guard conversation.canonicalID == conversation.id.uuidString else {
                                    NotificationCenter.default.post(name: .nolaOpenChatTab, object: conversation.canonicalID)
                                    return
                                }
                                onSelect(conversation.id)
                            } label: {
                                HStack(alignment: .top, spacing: 0) {
                                    VStack(alignment: .leading, spacing: 6) {
                                        HStack(alignment: .firstTextBaseline) {
                                            Text(title(conversation)).font(.body.weight(.medium))
                                                .foregroundStyle(DesignTokens.contentColor).lineLimit(1)
                                                .layoutPriority(1)
                                            Spacer(minLength: 8)
                                            Text(conversation.lastActiveAt, format: Calendar.current.isDateInToday(conversation.lastActiveAt) ? .dateTime.hour().minute() : .dateTime.month().day())
                                                .font(.caption).foregroundStyle(.secondary)
                                                .fixedSize(horizontal: true, vertical: false)
                                        }
                                        if let draft = previews[conversation.id]?.draft, !draft.isEmpty {
                                            (Text(String(localized: "chat.history.draft")).foregroundColor(DesignTokens.warningColor) + Text(verbatim: " · " + draft).foregroundColor(.secondary))
                                                .font(.subheadline).lineLimit(1)
                                        } else if let previewError = previews[conversation.id]?.error {
                                            Text(previewError).font(.caption).foregroundStyle(.secondary)
                                                .lineLimit(3)
                                                .accessibilityIdentifier("chat.conversation.previewError.\(conversation.id.uuidString.lowercased())")
                                        } else if let execution = container.chatManager.executionForConversation(conversation.id) {
                                            ChatConversationLivePreview(execution: execution, fallback: previews[conversation.id]?.message ?? "")
                                        } else if let preview = previews[conversation.id] {
                                            Text(preview.message.isEmpty ? String(localized: "chat.history.no_messages") : preview.message)
                                                .font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                                        }

                                    }
                                }.frame(maxWidth: .infinity, minHeight: 55, alignment: .leading)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(NolaPressableButtonStyle())
                            .accessibilityIdentifier("chat.conversation.\(conversation.id.uuidString.lowercased())")
                            .task(id: PreviewLoadIdentity(lastActiveAt: conversation.lastActiveAt, refreshRevision: previewRefreshRevision, searching: !query.isEmpty)) {
                                if query.isEmpty { await loadPreview(conversation) }
                            }
                            .contextMenu {
                                if conversation.canonicalID == conversation.id.uuidString {
                                    Button(String(localized: "chat.conversations.rename", defaultValue: "Rename"),
                                           systemImage: "pencil") {
                                        let current = title(conversation)
                                        renameDraft = current
                                        renameTarget = (conversation.id, current)
                                    }
                                    let isRunning = container.chatManager.executionForConversation(conversation.id)?.processingState.blocksInput == true
                                    Button(isRunning ? String(localized: "chat.history.stop_before_delete") : L10n.chatClearAction,
                                           systemImage: "trash", role: .destructive) { pendingDelete = conversation }
                                        .disabled(isRunning)
                                }
                            }
                            if previews[conversation.id]?.hasRoom == true { Button {
                                dismissSearchKeyboard()
                                if !expandedParents.insert(conversation.id).inserted { expandedParents.remove(conversation.id) }
                            } label: {
                                Image(systemName: expandedParents.contains(conversation.id) ? "chevron.down" : "chevron.right")
                                    .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                                    .frame(width: 44, height: 44).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                            .accessibilityLabel(Text("expert.history.title"))
                            .accessibilityValue(expandedParents.contains(conversation.id) ? Text("expert.history.expanded") : Text("expert.history.collapsed"))
                            .accessibilityIdentifier("chat.children.toggle." + conversation.id.uuidString.lowercased())
                            }
                        }
                        if expandedParents.contains(conversation.id) {
                            IOSExpertChildrenBranch(parentID: conversation.canonicalID, runtime: container.applicationRuntime,
                                refreshRevision: previewRefreshRevision, onContinue: onContinueExpert)
                                .padding(.leading, 20)
                        }
                    }
                    .listRowSeparator(.visible, edges: .bottom)
                    .listRowBackground(DesignTokens.backgroundColor)
                    .listRowInsets(EdgeInsets(top: 12, leading: pageInset, bottom: 12, trailing: pageInset))
    }

    private var listHeader: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                IOSWorkspacePageHeading(title: L10n.chatTitle)
                HStack {
                    Button {
                        dismissSearchKeyboard()
                        if let id = container.chatManager.startConversation(title: nil, language: nil) {
                            onSelect(id)
                        } else { loadError = String(localized: "chat.history.create_failed") }
                    } label: {
                        Image("nola-edit").renderingMode(.template).resizable().scaledToFit()
                            .frame(width: 20, height: 20)
                            .foregroundStyle(DesignTokens.actionColor)
                            .frame(width: 38, height: 38)
                            .background(DesignTokens.accentSubtleColor, in: Circle())
                            .frame(width: 44, height: 44).contentShape(Rectangle())
                    }.accessibilityLabel(Text(String(localized: "chat.history.empty.action")))
                        .accessibilityIdentifier("chat.conversations.new")
                }
            }.frame(minHeight: 64)
            HStack(spacing: 10) {
                Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 19, height: 19).foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                TextField("", text: $search, prompt: Text(String(localized: "chat.history.search")).foregroundColor(.secondary))
                    .font(.subheadline).focused($searchFocused)
                    .submitLabel(.search).autocorrectionDisabled()
                    .onSubmit { dismissSearchKeyboard() }
                    .accessibilityIdentifier("chat.conversations.search")
                if !search.isEmpty {
                    Button {
                        search = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.subheadline).foregroundStyle(.secondary)
                            .frame(width: 44, height: 44).contentShape(Rectangle())
                    }
                    .accessibilityLabel(Text("chat.history.search.clear"))
                    .accessibilityIdentifier("chat.conversations.search.clear")
                }
            }
            .padding(.leading, 13).padding(.trailing, search.isEmpty ? 13 : 0)
            .frame(minHeight: 46)
            .background(DesignTokens.elevatedColor, in: RoundedRectangle(cornerRadius: 12))
            .contentShape(Rectangle())
            .onTapGesture { searchFocused = true }
            HStack(spacing: 2) {
                filterButton("activity.all", waitingOnly: false)
                filterButton("workspace.today.needsResponse", waitingOnly: true)
                Spacer(minLength: 0)
            }
            .backgroundPreferenceValue(ConversationFilterBounds.self) { anchors in
                GeometryReader { proxy in
                    if let anchor = anchors[needsResponseOnly] {
                        let target = proxy[anchor]
                        Capsule()
                            .fill(DesignTokens.accentSubtleColor)
                            .frame(width: target.width, height: target.height)
                            .position(x: target.midX, y: target.midY)
                            .animation(reduceMotion ? nil : DesignTokens.disclosureAnimation, value: needsResponseOnly)
                    }
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, pageInset)
        .padding(.top, 4).padding(.bottom, 16)
        .background(DesignTokens.backgroundColor)
    }

    private func waiting(_ id: UUID) -> Bool {
        if persistedWaitingIDs.contains(id) { return true }
        guard let execution = container.chatManager.executionForConversation(id) else { return false }
        return execution.pendingHostPermissionRequest != nil
            || execution.localAgentPermissionRequest != nil
            || execution.remoteRunNeedsResume
            || execution.unknownEffectRecovery != nil
    }
    private func filterButton(_ key: String, waitingOnly: Bool) -> some View {
        let selected = needsResponseOnly == waitingOnly
        return Button { needsResponseOnly = waitingOnly } label: {
            Text(LocalizedStringKey(key)).font(.subheadline.weight(selected ? .semibold : .regular))
                .foregroundStyle(selected ? DesignTokens.actionColor : .secondary)
                .padding(.horizontal, 14).padding(.vertical, 6)
                .anchorPreference(key: ConversationFilterBounds.self, value: .bounds) { [waitingOnly: $0] }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func dismissSearchKeyboard() {
        searchFocused = false
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }

    private func title(_ conversation: IOSConversationDirectoryEntry) -> String {
        IOSConversationTitlePresentationPolicy.displayTitle(conversation.title,
            conversationID: conversation.id.uuidString, fallback: L10n.chatConversationDefaultTitle)
    }

    private func loadPreview(_ entry: IOSConversationDirectoryEntry) async {
        let previous = previews[entry.id]
        guard previous?.checkedOnVisit != previewRefreshRevision else { return }
        let runtime = container.applicationRuntime
        let revision = previewRefreshRevision
        let preview = await Task.detached(priority: .utility) {
            await IOSConversationListPreview.read(entry, runtime: runtime, previous: previous, visit: revision)
        }.value
        guard !Task.isCancelled, previewRefreshRevision == revision else { return }
        previews[entry.id] = preview
    }

    private func refresh(forcePreviews: Bool = false) async {
        let runtime = container.applicationRuntime, request = UUID()
        directoryRequest = request
        do {
            let previous = directorySnapshot
            let (snapshot, directory, waitingIDs) = try await Task.detached {
                let snapshot = try runtime.listRootConversations()
                let changed = forcePreviews || snapshot != previous
                let directory = changed ? try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime, summaries: snapshot) : nil
                let waitingIDs: Set<UUID>? = (try? IOSWorkspaceReadFacts.snapshot(runtime: runtime)).map { facts in
                    Set(facts.conversations.filter(\.needs_response).compactMap { UUID(uuidString: $0.conversation_id) })
                }
                return (snapshot, directory, waitingIDs)
            }.value
            guard !Task.isCancelled, directoryRequest == request else { return }
            if let waitingIDs {
                persistedWaitingIDs = waitingIDs
            }
            if let directory {
                let ids = Set(directory.map(\.id))
                previews = previews.filter { ids.contains($0.key) }
                if forcePreviews {
                    for id in previews.keys { previews[id]?.hasCurrentMessage = false }
                }
                expandedParents.formIntersection(ids)
                conversations = directory
                directorySnapshot = snapshot
            }
            previewRefreshRevision += 1
            loadError = nil
        } catch {
            if !Task.isCancelled && directoryRequest == request { loadError = error.localizedDescription }
        }
    }
}

private struct ChatConversationLivePreview: View {
    @ObservedObject var execution: ChatManager
    let fallback: String
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            if execution.pendingHostPermissionRequest != nil || execution.localAgentPermissionRequest != nil {
                Label(String(localized: "chat.history.awaiting_permission"), systemImage: "hand.raised")
                    .foregroundStyle(DesignTokens.primaryColor)
            } else if execution.processingState.blocksInput {
                Label(String(localized: "chat.history.running"), systemImage: "ellipsis.circle")
                    .foregroundStyle(DesignTokens.primaryColor)
            }
            let preview = execution.currentResponse.isEmpty ? fallback : execution.currentResponse
            if !preview.isEmpty {
                Text(preview).foregroundStyle(.secondary).lineLimit(2)
            } else if !execution.processingState.blocksInput {
                Text(String(localized: "chat.history.no_messages"))
                    .foregroundStyle(.secondary).lineLimit(2)
            }
        }.font(.subheadline)
    }
}
