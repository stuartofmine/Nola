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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 5) {
                    Label(
                        String(
                            format: String(localized: "%lld original messages", table: "Chat"),
                            Int64(model.selectedMessages.count)
                        ),
                        systemImage: "text.bubble"
                    )
                    Spacer()
                    Label(
                        model.selectedContextArchiveCount.map { count in String(
                            format: String(localized: "%lld summaries", table: "Chat"),
                            Int64(count)
                        ) } ?? (model.selectedContextArchiveCountFailed ? "--" : "..."),
                        systemImage: "archivebox"
                    )
                    .task(id: "\(model.selectedConversationID ?? ""):archive:\(model.isSelectedConversationVisiblyRunning)") {
                        await model.refreshContextArchiveCount()
                    }
                }
                .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .medium))
                .foregroundStyle(Color.secondary.opacity(0.84))
            }
            .padding(5)
            .background(
                Color.primary.opacity(0.025),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
            )

            DisclosureGroup(String(localized: "Knowledge Points", table: "Chat"), isExpanded: Binding(
                get: { navigation.expandedKnowledge.contains(.points) },
                set: { expanded in
                    if expanded { navigation.expandedKnowledge.insert(.points) }
                    else { navigation.expandedKnowledge.remove(.points) }
                }
            )) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "Keep stable facts, decisions, constraints, and open questions for this conversation.", table: "Chat"))
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    MacInspectorDocumentEditor(
                        text: Binding(
                            get: { model.selectedKnowledgePointsText },
                            set: { model.selectedKnowledgePointsText = $0 }
                        ),
                        placeholder: String(localized: "Add one knowledge point per line…", table: "Chat"),
                        minHeight: 230
                    )
                    knowledgeSaveStatus
                }
            }
            .id("knowledge-points")
            DisclosureGroup(String(localized: "Conversation Wiki", table: "Chat"), isExpanded: Binding(
                get: { navigation.expandedKnowledge.contains(.wiki) },
                set: { expanded in
                    if expanded { navigation.expandedKnowledge.insert(.wiki) }
                    else { navigation.expandedKnowledge.remove(.wiki) }
                }
            )) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "Maintain a living Markdown reference that belongs only to this conversation.", table: "Chat"))
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    MacInspectorDocumentEditor(
                        text: Binding(
                            get: { model.selectedWikiText },
                            set: { model.selectedWikiText = $0 }
                        ),
                        placeholder: String(localized: "Write the conversation Wiki in Markdown…", table: "Chat"),
                        minHeight: 300
                    )
                    knowledgeSaveStatus
                }
            }
            .id("knowledge-wiki")
            DisclosureGroup(String(localized: "Conversation Recall", table: "Chat"), isExpanded: Binding(
                get: { navigation.expandedKnowledge.contains(.recall) },
                set: { expanded in
                    if expanded { navigation.expandedKnowledge.insert(.recall) }
                    else { navigation.expandedKnowledge.remove(.recall) }
                }
            )) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "Search original messages. Compaction summaries are not included.", table: "Chat"))
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)

                    HStack(spacing: 7) {
                        Image(systemName: "text.magnifyingglass")
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
                            }
                            .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init(top: 2, leading: 2, bottom: 2, trailing: 2)))
                            .accessibilityLabel(String(localized: "Clear"))
                        }
                    }
                    .padding(.horizontal, 10)
                    .frame(height: 34)
                    .background(
                        MacDesignColor.hover,
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                    )

                    if !model.conversationRecallQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        if model.isConversationRecallSearching {
                            ProgressView()
                                .controlSize(.small)
                                .frame(maxWidth: .infinity, minHeight: 160)
                                .accessibilityLabel(String(localized: "Search"))
                        } else if model.conversationRecallFailed {
                            VStack(spacing: 12) {
                                Text(String(localized: "Conversation search failed. Please try again.", table: "Chat"))
                                    .foregroundStyle(.secondary)
                                    .multilineTextAlignment(.center)
                                Button(String(localized: "Retry search", table: "Chat")) {
                                    model.retryConversationRecallSearch()
                                }
                            }
                            .frame(maxWidth: .infinity, minHeight: 160)
                            .accessibilityIdentifier("conversation-recall-error")
                        } else if model.conversationRecallResults.isEmpty {
                            ContentUnavailableView(
                                String(localized: "No matching conversation memory", table: "Chat"),
                                systemImage: "text.magnifyingglass"
                            )
                            .frame(maxWidth: .infinity, minHeight: 160)
                        } else {
                            LazyVStack(spacing: 7) {
                                ForEach(model.conversationRecallResults) { result in
                                    MacConversationRecallRow(result: result)
                                }
                            }
                        }
                    }
                }
            }
            .id("knowledge-recall")
        }
        .disclosureGroupStyle(MacRowDisclosureStyle())
    }

    @ViewBuilder
    private var knowledgeSaveStatus: some View {
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
        .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .medium))
        .foregroundStyle(.secondary)
        .frame(minHeight: 16, alignment: .leading)
        .animation(reduceMotion ? nil : .smooth(duration: 0.18), value: model.selectedKnowledgeIsSaving)
    }
}
