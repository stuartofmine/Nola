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

/// App-wide destinations have a single home; contained features remain searchable.
struct MacWorkspaceResourceNavigation: View {
    @Binding var search: String
    let selection: MacWorkspaceSection
    let open: (MacWorkspaceSection) -> Void
    private var destinations: [MacWorkspaceSection] {
        let candidates = search.isEmpty ? MacWorkspaceSection.resourceNavigationSections
            : MacWorkspaceSection.resourceNavigationSections + [.memory, .automations]
        return candidates.filter { search.isEmpty || String(localized: String.LocalizationValue($0.titleKey))
            .localizedCaseInsensitiveContains(search) }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            TextField(String(localized: "Search"), text: $search).textFieldStyle(.roundedBorder)
                .padding(.bottom, 8)
            ForEach(destinations) { section in
                Button { open(section) } label: {
                    Label(section.title, systemImage: section.symbol)
                        .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
                        .padding(.horizontal, 8).contentShape(Rectangle())
                }
                .buttonStyle(MacSidebarNavigationButtonStyle(isSelected: selection == section || selection.resourceNavigationOwner == section))
                .accessibilityIdentifier("workspace.resource." + section.rawValue)
            }
            Spacer()
            Divider()
            MacWorkspaceSettingsRow(isSelected: selection == .settings) { open(.settings) }
        }.padding(12)
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
    let isSelected: Bool
    let loadParticipantNames: (String) async throws -> [String]
    @State private var participantNames: [String] = []
    @State private var isHovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(MacConversationTitlePolicy.displayTitle(
                        conversation.title,
                        conversationID: conversation.id
                    ))
                        .font(.system(size: 14, weight: .medium))
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .help(MacConversationTitlePolicy.displayTitle(conversation.title, conversationID: conversation.id))
                    Text(conversation.updatedAt, format: Calendar.current.isDateInToday(conversation.updatedAt)
                         ? .dateTime.hour().minute() : .dateTime.month().day())
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize()
                }
                if !conversation.preview.isEmpty && conversation.preview != conversation.title {
                    Text(conversation.preview)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if !participantNames.isEmpty {
                    Label(participantNames.joined(separator: " · "), systemImage: "person.2")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .help(participantNames.joined(separator: " · "))
                }
            }
            Spacer(minLength: 4)

        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(minHeight: 40)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(!isSelected && isHovering ? MacDesignColor.hover : .clear,
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control))
        .macSelectionHighlight(isSelected: isSelected)
        .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .onHover { isHovering = $0 }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.17), value: isHovering)
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
