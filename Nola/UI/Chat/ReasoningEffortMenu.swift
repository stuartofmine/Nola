// SPDX-License-Identifier: Apache-2.0
import SwiftUI

nonisolated enum ReasoningEffortPresentation {
    static var title: String { label("title") }
    static func label(_ value: String) -> String {
        NSLocalizedString("reasoning.effort." + value, tableName: "Reasoning", value: value, comment: "Model reasoning effort")
    }
}

struct ReasoningEffortMenu: View {
    let efforts: [String]
    let preference: String
    var disabled = false
    let select: (String) -> Void

    var body: some View {
        if !efforts.isEmpty {
            Menu {
                Picker(ReasoningEffortPresentation.title, selection: Binding(get: { preference }, set: select)) {
                    Text(ReasoningEffortPresentation.label("automatic")).tag("automatic")
                    ForEach(efforts, id: \.self) { effort in
                        Text(ReasoningEffortPresentation.label(effort)).tag(effort)
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "brain").font(.caption)
                    Text(ReasoningEffortPresentation.label(preference)).lineLimit(1)
                    Image(systemName: "chevron.down").font(.system(size: 8, weight: .semibold))
                }
                .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                .padding(.horizontal, 6).frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
            }
            .buttonStyle(.plain).disabled(disabled)
            .accessibilityLabel(ReasoningEffortPresentation.title)
            .accessibilityValue(ReasoningEffortPresentation.label(preference))
            .accessibilityIdentifier("chat.composer.reasoning")
        }
    }
}
