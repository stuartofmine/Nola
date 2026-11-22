// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents and edits conversation-scoped knowledge, wiki, and recall projections.

import Observation
import SwiftUI

enum MacInspectorKnowledgePane: String, CaseIterable, Identifiable {
    case points
    case wiki
    case recall

    var id: String { rawValue }

    var title: String {
        switch self {
        case .points: return String(localized: "Knowledge Points", table: "Chat")
        case .wiki: return String(localized: "Conversation Wiki", table: "Chat")
        case .recall: return String(localized: "Conversation Recall", table: "Chat")
        }
    }

    var compactTitle: String {
        switch self {
        case .points: return String(localized: "Points", table: "Chat")
        case .wiki: return String(localized: "Wiki", table: "Chat")
        case .recall: return String(localized: "Recall", table: "Chat")
        }
    }

    var symbol: String {
        switch self {
        case .points: return "list.bullet.rectangle"
        case .wiki: return "book.pages"
        case .recall: return "text.magnifyingglass"
        }
    }
}

struct MacInspectorKnowledgeView: View {
    @Bindable var model: MacWorkspaceModel
    @ObservedObject var navigation: MacInspectorNavigationState
    @State private var showsDiscardConfirmation = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Text(String(localized: "Conversation Knowledge", table: "Chat"))
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .semibold))
                Spacer()
                Text(
                    String(
                        format: String(localized: "%lld original messages", table: "Chat"),
                        Int64(model.selectedMessages.count)
                    )
                    + " · "
                    + (model.selectedContextArchiveCount.map { count in
                        String(format: String(localized: "%lld summaries", table: "Chat"), Int64(count))
                    } ?? (model.selectedContextArchiveCountFailed ? "--" : "…"))
                )
                .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .task(id: "\(model.selectedConversationID ?? ""):archive:\(model.isSelectedConversationVisiblyRunning)") {
                    await model.refreshContextArchiveCount()
                }
            }

            MacSegmentedTabBar(
                options: MacInspectorKnowledgePane.allCases,
                selection: $navigation.knowledgePane,
                title: { $0.compactTitle }
            )

            Group {
                switch navigation.knowledgePane {
                case .points:
                    VStack(alignment: .leading, spacing: 8) {
                        MacInspectorDocumentEditor(
                            text: Binding(
                                get: { model.selectedKnowledgePointsText },
                                set: { model.selectedKnowledgePointsText = $0 }
                            ),
                            placeholder: String(localized: "Add one knowledge point per line…", table: "Chat"),
                            minHeight: 150
                        )
                        knowledgeSaveStatus
                    }
                    .id("knowledge-points")
                case .wiki:
                    VStack(alignment: .leading, spacing: 8) {
                        MacInspectorDocumentEditor(
                            text: Binding(
                                get: { model.selectedWikiText },
                                set: { model.selectedWikiText = $0 }
                            ),
                            placeholder: String(localized: "Write the conversation Wiki in Markdown…", table: "Chat"),
                            minHeight: 180
                        )
                        knowledgeSaveStatus
                    }
                    .id("knowledge-wiki")
                case .recall:
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 7) {
                            Image(systemName: "text.magnifyingglass")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                            TextField(
                                String(localized: "Search original conversation", table: "Chat"),
                                text: $model.conversationRecallQuery
                            )
                            .textFieldStyle(.plain)
                            if !model.conversationRecallQuery.isEmpty {
                                Button {
                                    model.conversationRecallQuery = ""
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .foregroundStyle(.tertiary)
                                        .frame(width: 24, height: 24)
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init()))
                                .accessibilityLabel(String(localized: "Clear"))
                                .help(String(localized: "Clear"))
                                .accessibilityIdentifier("inspector-recall-clear")
                            }
                        }
                        .padding(.horizontal, 10)
                        .frame(height: 32)
                        .background(
                            colorScheme == .dark ? Color.white.opacity(0.045) : MacDesignColor.hover,
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )
                        .overlay {
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .strokeBorder(colorScheme == .dark ? Color.white.opacity(0.09) : MacDesignColor.borderSubtle, lineWidth: 0.75)
                        }

                        if !model.conversationRecallQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            if model.isConversationRecallSearching {
                                ProgressView()
                                    .controlSize(.small)
                                    .frame(maxWidth: .infinity, minHeight: 100)
                                    .accessibilityLabel(String(localized: "Search"))
                            } else if model.conversationRecallFailed {
                                VStack(spacing: 10) {
                                    Text(String(localized: "Conversation search failed. Please try again.", table: "Chat"))
                                        .foregroundStyle(.secondary)
                                        .multilineTextAlignment(.center)
                                    Button(String(localized: "Retry search", table: "Chat")) {
                                        model.retryConversationRecallSearch()
                                    }
                                }
                                .frame(maxWidth: .infinity, minHeight: 100)
                                .accessibilityIdentifier("conversation-recall-error")
                            } else if model.conversationRecallResults.isEmpty {
                                ContentUnavailableView(
                                    String(localized: "No matching conversation memory", table: "Chat"),
                                    systemImage: "text.magnifyingglass"
                                )
                                .frame(maxWidth: .infinity, minHeight: 100)
                            } else {
                                LazyVStack(spacing: 7) {
                                    ForEach(model.conversationRecallResults) { result in
                                        MacConversationRecallRow(result: result)
                                    }
                                }
                            }
                        }
                    }
                    .id("knowledge-recall")
                }
            }
        }
    }

    @ViewBuilder
    private var knowledgeSaveStatus: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                if model.selectedKnowledgeSaveFailed {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(String(localized: "Knowledge save was not confirmed. Your draft is retained.", table: "Chat"))
                        .fixedSize(horizontal: false, vertical: true)
                } else if model.selectedKnowledgeIsSaving {
                    ProgressView()
                        .controlSize(.mini)
                    Text(String(localized: "Saving…", table: "Chat"))
                } else if let updatedAt = model.selectedKnowledgeUpdatedAt {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Text(
                        String(
                            format: String(localized: "Saved %@", table: "Chat"),
                            updatedAt.formatted(date: .abbreviated, time: .shortened)
                        )
                    )
                }
            }
            if model.selectedKnowledgeSaveFailed {
                HStack(spacing: 8) {
                    Button(String(localized: "Retry Save", table: "Chat")) {
                        model.retrySelectedKnowledgeSave()
                    }
                    .buttonStyle(MacActionButtonStyle())
                    Button(String(localized: "Discard Unsaved Edits", table: "Chat"), role: .destructive) {
                        showsDiscardConfirmation = true
                    }
                    .buttonStyle(.bordered)
                }
            }
        }
        .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .medium))
        .foregroundStyle(.secondary)
        .frame(minHeight: 16, alignment: .leading)
        .animation(reduceMotion ? nil : .smooth(duration: 0.18), value: model.selectedKnowledgeIsSaving)
        .confirmationDialog(
            String(localized: "Discard unsaved changes?"),
            isPresented: $showsDiscardConfirmation,
            titleVisibility: .visible
        ) {
            Button(String(localized: "Discard Unsaved Edits", table: "Chat"), role: .destructive) {
                model.discardSelectedKnowledgeEdits()
            }
            Button(String(localized: "Keep Editing"), role: .cancel) {}
        }
    }
}
