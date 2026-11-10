// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents workspace navigation and conversation selection commands.

import Observation
import SwiftUI

struct MacConversationSidebar: View {
    @Bindable var model: MacWorkspaceModel
    @Binding var selectedSection: MacWorkspaceSection
    let selectConversation: (String) -> Void
    let loadParticipantNames: (String) async throws -> [String]
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isSearchHovering = false
    @State private var deletionTarget: MacConversation?
    @FocusState private var focusedRenameID: String?

    private var pinned: [MacConversation] {
        model.filteredConversations.filter(\.isPinned)
    }

    private var recent: [MacConversation] {
        model.filteredConversations.filter { !$0.isPinned }
    }


    var body: some View {
        VStack(spacing: 0) {
            sidebarSectionLabel(String(localized: "Conversations"))
                .padding(.horizontal, 12)
                .padding(.top, 16)
                .padding(.bottom, 10)

            HStack(spacing: 7) {
                HStack(spacing: 7) {
                    Image("nola-search").renderingMode(.template).resizable().scaledToFit().frame(width: 16, height: 16)
                        .font(.system(size: 14, weight: .medium))
                        .foregroundStyle(.secondary)
                    TextField(String(localized: "Search conversations"), text: $model.searchText)
                        .textFieldStyle(.plain)
                        .font(.system(size: 12.5))
                    if !model.searchText.isEmpty {
                        Button {
                            model.searchText = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.tertiary)
                        }
                        .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                        .help(String(localized: "Clear Search"))
                        .accessibilityLabel(String(localized: "Clear"))
                    }
                }
                .padding(.horizontal, 10)
                .frame(height: 36)
                .background(
                    Color.primary.opacity(isSearchHovering ? 0.072 : 0.055),
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous)
                        .stroke(Color.primary.opacity(isSearchHovering ? 0.085 : 0), lineWidth: 0.5)
                }
                .contentShape(RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous))
                .onHover { isSearchHovering = $0 }
                .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: isSearchHovering)


            }
            .padding(.horizontal, 10)
            .padding(.bottom, 10)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 3) {
                    if !pinned.isEmpty {
                        conversationSection(String(localized: "Pinned"), conversations: pinned)
                    }

                    conversationSection(String(localized: "Recent"), conversations: recent)
                    if pinned.isEmpty && recent.isEmpty {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(model.searchText.isEmpty
                                ? String(localized: "No conversations yet")
                                : String(localized: "No matching conversations"))
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                            if !model.searchText.isEmpty {
                                Button(String(localized: "Clear Search")) { model.searchText = "" }
                                    .buttonStyle(MacActionButtonStyle())
                                    .foregroundStyle(MacDesignColor.brand)
                            }
                        }
                        .padding(10)
                        .accessibilityIdentifier("mac-sidebar-conversation-empty")
                    }
                }
                .padding(.horizontal, 9)
                .padding(.bottom, 10)
            }
            .background(Color.clear)

            Divider()
                .opacity(0.55)

            Button {
                selectedSection = .settings
            } label: {
                HStack(spacing: 9) {
                    MacSidebarStatusIndicator(isRunning: model.isVisiblyRunning)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Nola")
                            .font(.system(size: 12.5, weight: .semibold))
                        Text(String(localized: "Settings"))
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image("nola-settings").renderingMode(.template).resizable().scaledToFit().frame(width: 18, height: 18)
                        .font(.system(size: 13.5, weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(width: 26, height: 26)
                }
                .padding(.horizontal, 8)
                .frame(height: 44)
                .contentShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(MacSidebarNavigationButtonStyle(isSelected: false))
            .padding(.horizontal, 8)
            .padding(.vertical, 7)
            .help(String(localized: "Settings"))
            .accessibilityLabel(String(localized: "Settings"))
        }
        .onChange(of: model.renamingConversationID) { _, conversationID in
            Task { @MainActor in
                await Task.yield()
                focusedRenameID = conversationID
            }
        }
        .onChange(of: focusedRenameID) { previousID, focusedID in
            guard previousID != nil,
                  focusedID == nil,
                  model.renamingConversationID == previousID else { return }
            model.commitRename()
        }
        .confirmationDialog(
            String(localized: "Delete Conversation?", table: "Chat"),
            isPresented: Binding(get: { deletionTarget != nil }, set: { if !$0 { deletionTarget = nil } }),
            titleVisibility: .visible,
            presenting: deletionTarget
        ) { conversation in
            Button(String(localized: "Delete"), role: .destructive) {
                model.deleteConversation(conversation)
                deletionTarget = nil
            }
            Button(String(localized: "Cancel"), role: .cancel) { deletionTarget = nil }
        } message: { conversation in
            Text(String(format: String(localized: "Delete the conversation “%@”? This cannot be undone. Generated workspace files are kept.", table: "Chat"), conversation.title))
        }
        .alert(String(localized: "Conversation"), isPresented: Binding(
            get: { model.conversationOperationError != nil },
            set: { if !$0 { model.conversationOperationError = nil } }
        )) {
            Button(String(localized: "OK"), role: .cancel) { model.conversationOperationError = nil }
        } message: {
            Text(model.conversationOperationError ?? "")
        }
    }

    @ViewBuilder
    private func conversationSection(_ title: String, conversations: [MacConversation]) -> some View {
        if !conversations.isEmpty {
            sidebarSectionLabel(title)
                .padding(.horizontal, 3)
                .padding(.top, 5)
                .padding(.bottom, 2)
            conversationRows(conversations)
        }
    }

    private func sidebarSectionLabel(_ title: String) -> some View {
        Text(String(localized: String.LocalizationValue(title)))
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private func conversationRows(_ conversations: [MacConversation]) -> some View {
        ForEach(conversations) { conversation in
            if model.renamingConversationID == conversation.id {
                inlineRenameRow(conversation)
                    .transition(.opacity.combined(with: .scale(scale: 0.985)))
            } else {
                Button {
                    selectConversation(conversation.id)
                } label: {
                    MacConversationRow(
                        conversation: conversation,
                        isSelected: selectedSection == .chat && model.selectedConversationID == conversation.id,
                        loadParticipantNames: loadParticipantNames
                    )
                }
                .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                .accessibilityAddTraits(selectedSection == .chat && model.selectedConversationID == conversation.id ? .isSelected : [])
                .accessibilityIdentifier("workspace.conversation." + conversation.id)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .contextMenu {
                    Button("Open in New Window") {
                        openWindow(value: conversation.id)
                    }
                    Button("Rename…") { model.beginRenaming(conversation) }
                    Button(
                        conversation.isPinned
                            ? String(localized: "Unpin")
                            : String(localized: "Pin")
                    ) {
                        model.togglePinned(conversation)
                    }
                    Divider()
                    Button("Delete", role: .destructive) {
                        deletionTarget = conversation
                    }
                    .disabled((model.isRunning && model.selectedConversationID == conversation.id)
                        || model.deletingConversationIDs.contains(conversation.id))
                }
            }
        }
    }

    private func inlineRenameRow(_ conversation: MacConversation) -> some View {
        HStack(spacing: 5) {
            TextField(String(localized: "Conversation name"), text: $model.renameDraft)
                .textFieldStyle(.plain)
                .font(.system(size: 14, weight: .medium))
                .focused($focusedRenameID, equals: conversation.id)
                .onSubmit { model.commitRename() }
                .onExitCommand { cancelRename() }

            Button {
                model.commitRename()
            } label: {
                Image(systemName: "checkmark")
                    .font(.system(size: 10.5, weight: .semibold))
            }
            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
            .disabled(model.renameDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .help(String(localized: "Rename"))
            .accessibilityLabel(String(localized: "Rename"))

            Button(action: cancelRename) {
                Image(systemName: "xmark")
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
            .help(String(localized: "Cancel"))
            .accessibilityLabel(String(localized: "Cancel"))
        }
        .padding(.horizontal, 9)
        .frame(minHeight: 38)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color.accentColor.opacity(0.10),
            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(Color.accentColor.opacity(0.28), lineWidth: 0.75)
        }
        .accessibilityElement(children: .contain)
    }

    private func cancelRename() {
        model.renamingConversationID = nil
        model.renameDraft = ""
        focusedRenameID = nil
    }
}
