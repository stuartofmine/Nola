// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacConversationInspectorButtons: View {
    let selection: MacChatInspector.InspectorTab
    let isVisible: Bool
    let select: (MacChatInspector.InspectorTab) -> Void

    private let groups: [(id: String, title: String, icon: String, tabs: [MacChatInspector.InspectorTab])] = [
        ("task", String(localized: "Task"), "nola-progress", [.overview, .progress, .goal]),
        ("artifacts", String(localized: "Artifacts"), "nola-artifacts", [.review, .artifacts]),
        ("context", String(localized: "Context"), "nola-file", [.context, .execution, .participants])
    ]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(groups, id: \.id) { group in
                let active = isVisible && group.tabs.contains(selection)
                Button {
                    select(group.tabs.contains(selection) ? selection : group.tabs[0])
                } label: {
                    Image(group.icon).renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 18, height: 18).frame(width: 36, height: 36)
                }
                .macToolbarIcon(isActive: active)
                .help(group.title + " · " + group.tabs.map(\.title).joined(separator: " · "))
                .accessibilityLabel(group.title)
                .accessibilityIdentifier("mac-inspector-group-\(group.id)")
            }
        }.fixedSize(horizontal: true, vertical: false)
    }
}
