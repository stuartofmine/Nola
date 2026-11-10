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
    let select: (IOSConversationWorkDestination) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(conversationTitle).font(.headline).fixedSize(horizontal: false, vertical: true)
                        Text(modelLabel).font(.subheadline).foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 4)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("chat.work_tools.summary")
                }
                Section {
                    ForEach(IOSConversationWorkDestination.allCases) { item in
                        Button { select(item) } label: {
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
                        .disabled(!hasOwner)
                        .accessibilityIdentifier("chat.work_tools." + item.rawValue)
                    }
                }
            }
            .navigationTitle(String(localized: "work.tools.title"))
            .toolbar { ToolbarItem(placement: .confirmationAction) {
                Button(L10n.close) { dismiss() }.accessibilityIdentifier("chat.work_tools.close")
            } }
        }
    }
}
