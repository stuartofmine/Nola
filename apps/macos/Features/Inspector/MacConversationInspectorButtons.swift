// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacConversationInspectorButtons: View {
    let selection: MacChatInspector.InspectorTab
    let isVisible: Bool
    var showsLabels: Bool = true
    let select: (MacChatInspector.InspectorTab) -> Void
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var trackFill: Color {
        colorScheme == .dark ? Color.white.opacity(0.045) : MacDesignColor.subtleFill.opacity(0.75)
    }

    private var trackBorder: Color {
        colorScheme == .dark ? Color.white.opacity(0.08) : MacDesignColor.borderSubtle
    }

    private var activeFill: Color {
        colorScheme == .dark ? Color.white.opacity(0.13) : Color.white
    }

    private var activeBorder: Color {
        colorScheme == .dark ? Color.white.opacity(0.16) : MacDesignColor.borderSubtle
    }

    @State private var hoveredGroup: MacChatInspector.InspectorArea?

    @ViewBuilder
    private func tabButton(for group: MacChatInspector.InspectorArea) -> some View {
        let active = isVisible && selection.area == group
        let hovered = !active && hoveredGroup == group
        let pillShape = RoundedRectangle(cornerRadius: 7, style: .continuous)
        Button {
            withAnimation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.selection)) {
                select(selection.area == group ? selection : group.sections[0])
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: group.symbol)
                    .font(.system(size: 12, weight: active ? .semibold : .medium))
                if showsLabels {
                    Text(group.title)
                        .font(.system(size: 12, weight: active ? .semibold : .medium))
                        .lineLimit(1)
                }
            }
            .foregroundStyle(active || hovered ? Color.primary : Color.secondary)
            .padding(.horizontal, showsLabels ? 9 : 0)
            .frame(minWidth: showsLabels ? nil : 28, minHeight: 28)
            .background {
                if active {
                    pillShape
                        .fill(activeFill)
                        .overlay(pillShape.strokeBorder(activeBorder, lineWidth: 0.75))
                } else if hovered {
                    pillShape
                        .fill(Color.primary.opacity(0.055))
                }
            }
            .contentShape(pillShape)
        }
        .buttonStyle(.plain)
        .onHover { isHovering in
            hoveredGroup = isHovering ? group : (hoveredGroup == group ? nil : hoveredGroup)
        }
        .help(group.title)
        .accessibilityLabel(group.title)
        .accessibilityIdentifier("mac-inspector-group-\(group.id)")
    }

    var body: some View {
        let trackShape = RoundedRectangle(cornerRadius: 9, style: .continuous)
        HStack(spacing: 2) {
            ForEach(MacChatInspector.InspectorArea.allCases) { group in
                tabButton(for: group)
            }
        }
        .padding(2)
        .background(
            trackShape
                .fill(trackFill)
                .overlay(trackShape.strokeBorder(trackBorder, lineWidth: 0.75))
        )
        .fixedSize(horizontal: true, vertical: false)
    }
}
