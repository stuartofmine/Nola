// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides sidebar interaction styles, status, and conversation-row presentation.

import SwiftUI

struct MacSidebarNavigationButtonStyle: ButtonStyle {
    let isSelected: Bool

    func makeBody(configuration: Configuration) -> some View {
        MacSidebarNavigationButtonBody(
            label: configuration.label,
            isSelected: isSelected,
            isPressed: configuration.isPressed
        )
    }
}

struct MacSidebarNavigationButtonBody<Label: View>: View {
    let label: Label
    let isSelected: Bool
    let isPressed: Bool
    @State private var isHovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .background(
                isEnabled && !isSelected ? (isPressed ? MacDesignColor.pressed : isHovering ? MacDesignColor.hover : .clear) : .clear,
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
            )
            .macSelectionHighlight(isSelected: isSelected)
            .opacity(isEnabled ? 1 : 0.45)
            .contentShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
            .onHover { isHovering = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovering = false } }
            .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: isSelected)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovering)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.10), value: isPressed)
    }
}

struct MacSidebarIconButtonStyle: ButtonStyle {
    var cornerRadius: CGFloat = 8

    func makeBody(configuration: Configuration) -> some View {
        MacSidebarIconButtonBody(
            label: configuration.label,
            isPressed: configuration.isPressed,
            cornerRadius: cornerRadius
        )
    }
}

struct MacSidebarIconButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let cornerRadius: CGFloat
    @State private var isHovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .foregroundStyle(isHovering ? Color.primary : Color.secondary)
            .background(
                isEnabled && isPressed ? MacDesignColor.pressed : isEnabled && isHovering ? MacDesignColor.hover : Color.clear,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(Color.primary.opacity(isHovering ? 0.085 : 0), lineWidth: 0.5)
            }
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .onHover { isHovering = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovering = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: isHovering)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.09), value: isPressed)
    }
}

struct MacWorkspaceSettingsRow: View {
    let isSelected: Bool
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Label(String(localized: "Settings"), systemImage: "gearshape")
                .padding(.horizontal, 8)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(MacSidebarNavigationButtonStyle(isSelected: isSelected))
        .accessibilityIdentifier("workspace.settings")
    }
}

struct MacConversationRow: View {
    let conversation: MacConversation
    let loadParticipantNames: (String) async throws -> [String]
    @State private var participantNames: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(MacConversationTitlePolicy.displayTitle(conversation.title, conversationID: conversation.id))
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Color("NolaTextPrimary"))
                .lineLimit(1)
                .help(MacConversationTitlePolicy.displayTitle(conversation.title, conversationID: conversation.id))
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if !conversation.preview.isEmpty && conversation.preview != conversation.title {
                    Text(conversation.preview)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                }
                Text(conversation.updatedAt, format: Calendar.current.isDateInToday(conversation.updatedAt)
                     ? .dateTime.hour().minute() : .dateTime.month().day())
                    .font(.caption.monospacedDigit())
                    .fixedSize()
            }
            .font(.system(size: 12))
            .foregroundStyle(Color("NolaTextSecondary"))
            if !participantNames.isEmpty {
                Label(participantNames.joined(separator: " · "), systemImage: "person.2")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help(participantNames.joined(separator: " · "))
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .frame(minHeight: 44)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(MacConversationTitlePolicy.displayTitle(
            conversation.title,
            conversationID: conversation.id
        ))
        .accessibilityValue([conversation.preview, participantNames.joined(separator: ", "),
                             conversation.updatedAt.formatted(date: .abbreviated, time: .shortened)]
            .filter { !$0.isEmpty }.joined(separator: ", "))
        .task(id: conversation.updatedAt) {
            // Lazy rows read canonical membership only; this never creates a room or starts a run.
            do {
                let names = try await loadParticipantNames(conversation.id)
                guard !Task.isCancelled else { return }
                participantNames = names
            } catch {
                guard !Task.isCancelled else { return }
                participantNames = []
            }
        }
    }
}
