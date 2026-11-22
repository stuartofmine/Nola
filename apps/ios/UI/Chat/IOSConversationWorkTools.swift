// SPDX-License-Identifier: Apache-2.0
import SwiftUI

enum IOSConversationWorkDestination: String, CaseIterable, Identifiable {
    case progress, goals, resources, context, experts, execution
    var id: String { rawValue }
    var title: String {
        switch self {
        case .progress: String(localized: "inspector.progress", table: "ConversationInspector")
        case .goals: String(localized: "work.tools.goals")
        case .resources: String(localized: "inspector.resources", table: "ConversationInspector")
        case .context: String(localized: "chat.context.title")
        case .experts: String(localized: "chat.experts.title")
        case .execution: String(localized: "work.tools.execution")
        }
    }
    var iconAsset: String {
        switch self {
        case .progress: "nola-progress"
        case .goals: "nola-goal"
        case .resources: "nola-folder"
        case .context: "nola-overview"
        case .experts: "nola-person"
        case .execution: "nola-execution"
        }
    }
}

/// Navigation over existing conversation capabilities. This view does not
/// reconstruct execution, authorize actions, or manufacture review/artifact data.
struct IOSConversationWorkTools: View {
    let conversationTitle: String
    let modelLabel: String
    let hasOwner: Bool
    var onRename: (() -> Void)? = nil
    var onConfirmRename: ((String) -> Void)? = nil
    var select: ((IOSConversationWorkDestination) -> Void)? = nil
    var destination: ((IOSConversationWorkDestination) -> AnyView)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var showingRenameAlert = false
    @State private var renameDraft = ""

    var body: some View {
        NavigationStack {
            List {
                Group {
                Section {
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(conversationTitle).font(.headline).lineLimit(2)
                            Text(modelLabel).font(.subheadline).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 8)
                        if hasOwner, onConfirmRename != nil || onRename != nil {
                            Button {
                                if onConfirmRename != nil {
                                    renameDraft = conversationTitle
                                    showingRenameAlert = true
                                } else {
                                    onRename?()
                                }
                            } label: {
                                Image(systemName: "pencil")
                                    .frame(width: 44, height: 44)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(String(localized: "chat.conversations.rename", defaultValue: "Rename"))
                            .accessibilityIdentifier("chat.work_tools.rename")
                        }
                    }
                    .padding(.vertical, 4)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("chat.work_tools.summary")
                }
                Section {
                    ForEach(IOSConversationWorkDestination.allCases) { item in
                        if destination != nil {
                            NavigationLink(value: item) {
                                HStack(spacing: 12) {
                                    Image(item.iconAsset).renderingMode(.template).resizable().scaledToFit().frame(width: 22, height: 22).accessibilityHidden(true)
                                    Text(item.title).foregroundStyle(.primary)
                                    Spacer(minLength: 8)
                                }
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .accessibilityIdentifier("chat.work_tools." + item.rawValue)
                        } else {
                            Button { select?(item) } label: {
                                HStack(spacing: 12) {
                                    Image(item.iconAsset).renderingMode(.template).resizable().scaledToFit().frame(width: 22, height: 22).accessibilityHidden(true)
                                    Text(item.title).foregroundStyle(.primary)
                                    Spacer(minLength: 8)
                                    Image("nola-chevron").renderingMode(.template).resizable().scaledToFit().frame(width: 14, height: 14).foregroundStyle(.tertiary)
                                        .accessibilityHidden(true)
                                }
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .accessibilityIdentifier("chat.work_tools." + item.rawValue)
                        }
                    }
                }
                }
                .listRowBackground(DesignTokens.surfaceColor)
                .listRowSeparatorTint(DesignTokens.borderColor)
            }
            .nolaGroupedSurface()
            .navigationTitle(String(localized: "work.tools.title"))
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: IOSConversationWorkDestination.self) { item in
                if let destination {
                    destination(item)
                }
            }
            .toolbar { ToolbarItem(placement: .confirmationAction) {
                Button(L10n.close) { dismiss() }.accessibilityIdentifier("chat.work_tools.close")
            } }
            .alert(String(localized: "chat.conversations.rename", defaultValue: "Rename Conversation"), isPresented: $showingRenameAlert) {
                TextField(String(localized: "chat.conversations.title_field", defaultValue: "Title"), text: $renameDraft)
                Button(L10n.cancel, role: .cancel) {}
                Button(L10n.done) {
                    onConfirmRename?(renameDraft)
                }
            }
        }
    }
}
