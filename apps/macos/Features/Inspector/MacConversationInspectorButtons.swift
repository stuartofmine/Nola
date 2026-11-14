// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacConversationInspectorButtons: View {
    let selection: MacChatInspector.InspectorTab
    let isVisible: Bool
    let select: (MacChatInspector.InspectorTab) -> Void

    var body: some View {
        HStack(spacing: 4) {
            ForEach(MacChatInspector.InspectorArea.allCases) { group in
                let active = isVisible && selection.area == group
                Button {
                    select(selection.area == group ? selection : group.sections[0])
                } label: {
                    Image(group.icon).renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 18, height: 18).frame(width: 36, height: 36)
                }
                .macToolbarIcon(isActive: active)
                .help(group.title)
                .accessibilityLabel(group.title)
                .accessibilityIdentifier("mac-inspector-group-\(group.id)")
            }
        }.fixedSize(horizontal: true, vertical: false)
    }
}
