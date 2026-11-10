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

/// The Chat tab's root list. It owns search/scroll state while a conversation is pushed.
struct ChatConversationsList: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage(AppleAppearancePreference.storageKey) private var appearance = "system"
    let onSelect: (UUID) -> Void
    @State private var conversations: [Conversation] = []
    @State private var pageWidth: CGFloat = 390
    @State private var previews: [UUID: String] = [:]
    @State private var expertNames: [UUID: [String]] = [:]
    @State private var previewDates: [UUID: Date] = [:]
    @Binding var search: String

    init(search: Binding<String>, onSelect: @escaping (UUID) -> Void) {
        self._search = search
        self.onSelect = onSelect
    }
    @FocusState private var searchFocused: Bool
    @State private var pendingDelete: Conversation?
    @State private var loadError: String?
    @State private var previewErrors: [UUID: String] = [:]
    @State private var previewRefreshRevision = 0
    @State private var draftTexts: [UUID: String] = [:]
    @State private var needsResponseOnly = false

    private struct PreviewLoadIdentity: Hashable {
        let lastActiveAt: Date
        let refreshRevision: Int
    }

    private var visibleConversations: [Conversation] {
        let candidates = conversations.filter { !needsResponseOnly || waiting($0.id) }
        guard !search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return candidates }
        return candidates.filter {
            title($0).localizedStandardContains(search) || (previews[$0.id] ?? "").localizedStandardContains(search) || (draftTexts[$0.id] ?? "").localizedStandardContains(search)
                || (expertNames[$0.id] ?? []).contains { $0.localizedStandardContains(search) }
        }
    }

    var body: some View {
        List {
            listHeader
                .listRowInsets(EdgeInsets())
                .listRowSeparator(.hidden)
                .listRowBackground(DesignTokens.surfaceColor)
            if let loadError {
                Section {
                    Text(loadError).foregroundStyle(.secondary)
                    Button(String(localized: "common.retry")) { refresh() }
                }.accessibilityIdentifier("chat.history.error")
            }
            if conversations.isEmpty && loadError == nil {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "chat.history.empty.title")).font(.headline)
                    Text(String(localized: "chat.history.empty.message")).foregroundStyle(.secondary)
                }.padding(.vertical, 20).listRowSeparator(.hidden).listRowBackground(DesignTokens.surfaceColor)
            }
            if !conversations.isEmpty && visibleConversations.isEmpty {
                Text(String(localized: "chat.history.no_results")).foregroundStyle(.secondary).padding(.vertical, 20).listRowBackground(DesignTokens.surfaceColor)
            }
            Section {
                        ForEach(visibleConversations, id: \.id) { conversation in
                            Button { dismissSearchKeyboard(); onSelect(conversation.id) } label: {
                                HStack(alignment: .top, spacing: 0) {
                                    VStack(alignment: .leading, spacing: 5) {
                                        HStack(alignment: .firstTextBaseline) {
                                            Text(title(conversation)).font(.body.weight(.semibold))
                                                .foregroundStyle(.primary).lineLimit(1)
                                                .layoutPriority(1)
                                            Spacer(minLength: 8)
                                            Text(conversation.lastActiveAt, format: Calendar.current.isDateInToday(conversation.lastActiveAt) ? .dateTime.hour().minute() : .dateTime.month().day())
                                                .font(.caption).foregroundStyle(.secondary)
                                                .fixedSize(horizontal: true, vertical: false)
                                        }
                                        if let draft = draftTexts[conversation.id], !draft.isEmpty {
                                            (Text(String(localized: "chat.history.draft")).foregroundColor(DesignTokens.warningColor) + Text(verbatim: " · " + draft).foregroundColor(.secondary))
                                                .font(.subheadline).lineLimit(2)
                                        } else if let previewError = previewErrors[conversation.id] {
                                            Text(previewError).font(.caption).foregroundStyle(.secondary)
                                                .lineLimit(3)
                                                .accessibilityIdentifier("chat.conversation.previewError.\(conversation.id.uuidString.lowercased())")
                                        } else if let execution = container.chatManager.executionForConversation(conversation.id) {
                                            ChatConversationLivePreview(execution: execution, fallback: previews[conversation.id] ?? "")
                                        } else {
                                            Text(previews[conversation.id].flatMap { $0.isEmpty ? nil : $0 } ?? String(localized: "chat.history.no_messages"))
                                                .font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                                        }

                                    }
                                }.frame(maxWidth: .infinity, minHeight: 55, alignment: .leading)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .listRowSeparator(.visible, edges: .bottom)
                            .listRowBackground(DesignTokens.surfaceColor)
                            .listRowInsets(EdgeInsets(top: 12, leading: pageWidth < 359 ? 16 : 20, bottom: 12, trailing: pageWidth < 359 ? 16 : 20))
                            .accessibilityIdentifier("chat.conversation.\(conversation.id.uuidString.lowercased())")
                            .task(id: PreviewLoadIdentity(lastActiveAt: conversation.lastActiveAt, refreshRevision: previewRefreshRevision)) { loadPreview(conversation) }
                            .contextMenu {
                                let isRunning = container.chatManager.executionForConversation(conversation.id)?.processingState.blocksInput == true
                                Button(isRunning ? String(localized: "chat.history.stop_before_delete") : L10n.chatClearAction,
                                       systemImage: "trash", role: .destructive) { pendingDelete = conversation }
                                    .disabled(isRunning)
                            }
                        }
            }

        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(DesignTokens.surfaceColor)
        .background(IOSWorkspaceWidthObserver(width: $pageWidth))
        .accessibilityIdentifier("chat.conversations.list")
        .navigationTitle("")
        .toolbar(.hidden, for: .navigationBar)
        .scrollDismissesKeyboard(.interactively)
        .onAppear { refresh() }
        .refreshable { refresh() }
        .confirmationDialog(String(localized: "chat.history.delete.title"), isPresented: Binding(
            get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }
        ), presenting: pendingDelete) { conversation in
            Button(L10n.chatClearAction, role: .destructive) {
                pendingDelete = nil
                Task { await container.chatManager.clearConversation(conversation.id); refresh() }
            }
            Button(L10n.cancel, role: .cancel) { pendingDelete = nil }
        }
    }

    private var listHeader: some View {
        VStack(alignment: .leading, spacing: 16) {
            ZStack {
                Text(L10n.chatTitle).font(.headline)
                HStack {
                    Spacer()
                    Button {
                        dismissSearchKeyboard()
                        if let id = container.chatManager.startConversation(title: nil, language: nil) {
                            refresh(); onSelect(id)
                        } else { loadError = String(localized: "chat.history.create_failed") }
                    } label: {
                        Image("nola-edit").renderingMode(.template).resizable().scaledToFit()
                            .frame(width: 23, height: 23).frame(width: 44, height: 44)
                    }.accessibilityLabel(Text(String(localized: "chat.history.empty.action")))
                        .accessibilityIdentifier("chat.conversations.new")
                }
            }.frame(height: 50)
            HStack(spacing: 10) {
                Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 19, height: 19).foregroundStyle(.secondary)
                TextField("", text: $search, prompt: Text(String(localized: "chat.history.search")).foregroundColor(.secondary))
                    .font(.subheadline).focused($searchFocused)
                    .submitLabel(.search).autocorrectionDisabled()
                    .accessibilityIdentifier("chat.conversations.search")
            }
            .padding(.horizontal, 13).frame(minHeight: 46)
            .background(DesignTokens.elevatedColor, in: RoundedRectangle(cornerRadius: 12))
            HStack(spacing: 18) {
                filterButton("activity.all", selected: !needsResponseOnly) { needsResponseOnly = false }
                filterButton("workspace.today.needsResponse", selected: needsResponseOnly) { needsResponseOnly = true }
                Spacer()
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, pageWidth < 359 ? 16 : 20)
        .padding(.top, 4).padding(.bottom, 16)
        .background(DesignTokens.surfaceColor)
    }

    private func waiting(_ id: UUID) -> Bool {
        guard let execution = container.chatManager.executionForConversation(id) else { return false }
        return execution.pendingHostPermissionRequest != nil || execution.localAgentPermissionRequest != nil
    }
    private func filterButton(_ key: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(LocalizedStringKey(key)).font(.subheadline.weight(selected ? .semibold : .regular))
                .foregroundStyle(selected ? DesignTokens.actionColor : .secondary)
                .frame(minHeight: 44)
                .overlay(alignment: .bottom) { if selected { Capsule().fill(DesignTokens.actionColor).frame(height: 3) } }
        }
    }

    private func dismissSearchKeyboard() {
        searchFocused = false
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }

    private func title(_ conversation: Conversation) -> String {
        IOSConversationTitlePresentationPolicy.displayTitle(conversation.title,
            conversationID: conversation.id.uuidString, fallback: L10n.chatConversationDefaultTitle)
    }

    private func loadPreview(_ conversation: Conversation) {
        do { draftTexts[conversation.id] = try FileConversationDraftRepository.application().textPreview(.conversation(conversation.id)) }
        catch { previewErrors[conversation.id] = error.localizedDescription; return }
        guard previewDates[conversation.id] != conversation.lastActiveAt else { return }
        // Read the existing association only; opening history must never create a room.
        do {
            let association = try IOSAgentRoomService(runtime: container.applicationRuntime)
                .forConversation(conversation.id.uuidString)
            expertNames[conversation.id] = association?.room.room.participants
                .filter(\.active).map(\.displayName) ?? []
        } catch { expertNames.removeValue(forKey: conversation.id) }
        do {
            let window = try container.applicationRuntime.loadConversationProjectionWindowV1(
                id: conversation.id.uuidString, beforeOrdinal: nil, limit: 2, contentByteLimit: 4096)
            let source = IOSConversationProjectionPresenter.messages(from: window.projection)
                .last(where: { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })?.content ?? ""
            previews[conversation.id] = source.components(separatedBy: .newlines).map { line in
                (try? AttributedString(markdown: line)).map { String($0.characters) } ?? line
            }.joined(separator: " ")
            previewDates[conversation.id] = conversation.lastActiveAt
            previewErrors.removeValue(forKey: conversation.id)
        } catch { previewErrors[conversation.id] = error.localizedDescription }
    }

    private func refresh() {
        do {
            let loaded = try container.conversationStore?.getAllConversations() ?? []
            previewDates.removeAll()
            previewErrors.removeAll()
            expertNames.removeAll()
            previewRefreshRevision += 1
            conversations = loaded.filter(\.appearsInOrdinaryHistory).sorted { $0.lastActiveAt > $1.lastActiveAt }
            loadError = nil
        } catch { loadError = error.localizedDescription }
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
