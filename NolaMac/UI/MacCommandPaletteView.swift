// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides keyboard-first navigation across conversations and desktop commands.

import AppKit
import SwiftUI

struct MacCommandPaletteView: View {
    @Bindable var model: MacWorkspaceModel
    let newConversation: () -> Void
    let selectConversation: (String) -> Void
    let focusComposer: () -> Void
    let toggleSidebar: () -> Void
    let toggleInspector: () -> Void
    let dismiss: () -> Void

    @State private var query = ""
    @State private var selectedIndex = 0
    @State private var shouldScrollToSelection = false
    @FocusState private var searchFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.openWindow) private var openWindow

    private var normalizedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(MacDesignColor.brand)
                TextField(String(localized: "Search conversations and commands"), text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 15))
                    .focused($searchFocused)
                    .accessibilityIdentifier("command-palette.search")
                    .onSubmit(performSelected)
                    .onKeyPress(.upArrow) {
                        moveSelection(by: -1)
                        return .handled
                    }
                    .onKeyPress(.downArrow) {
                        moveSelection(by: 1)
                        return .handled
                    }
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.tertiary)
                            .frame(width: 32, height: 32)
                    }
                    .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                    .accessibilityIdentifier("command-palette.clear")
                    .accessibilityLabel(NSLocalizedString(
                        "mac.command_palette.clear_search",
                        value: "Clear Search",
                        comment: "Clear command palette search"
                    ))
                }
                HStack(spacing: 5) {
                    keyHint("↑↓")
                    keyHint("↩")
                    keyHint("esc")
                }
                .accessibilityHidden(true)
            }
            .padding(.horizontal, 16)
            .frame(height: 52)

            Divider().opacity(0.55)

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 3) {
                        if !displayedConversations.isEmpty {
                            sectionLabel(String(localized: "Conversations"))
                            ForEach(Array(displayedConversations.enumerated()), id: \.element.id) { index, conversation in
                                paletteRow(
                                    index: index,
                                    symbol: "bubble.left",
                                    title: MacConversationTitlePolicy.displayTitle(
                                        conversation.title,
                                        conversationID: conversation.id
                                    ),
                                    detail: conversation.preview,
                                    shortcut: nil
                                ) {
                                    selectConversation(conversation.id)
                                }
                            }
                        }

                        if !matchingCommands.isEmpty {
                            ForEach(Array(matchingCommands.enumerated()), id: \.element.id) { offset, command in
                                if offset == 0
                                    || commandSectionID(command)
                                        != commandSectionID(matchingCommands[offset - 1]) {
                                    sectionLabel(commandSectionTitle(command))
                                }
                                paletteRow(
                                    index: displayedConversations.count + offset,
                                    symbol: command.symbol,
                                    title: command.title,
                                    detail: command.detail,
                                    shortcut: command.shortcut,
                                    action: command.action
                                )
                            }
                        }

                        if visibleItemCount == 0 {
                            VStack(spacing: 10) {
                                Image(systemName: "magnifyingglass")
                                    .font(.system(size: 20, weight: .light))
                                    .foregroundStyle(.tertiary)
                                Text(String(localized: "No Results"))
                                    .font(.system(size: 13, weight: .medium))
                                Text(String(localized: "No matching conversations or commands"))
                                    .font(.system(size: 11.5))
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 44)
                        }
                    }
                    .padding(8)
                }
                .onChange(of: selectedIndex) { _, index in
                    guard shouldScrollToSelection else { return }
                    shouldScrollToSelection = false
                    if reduceMotion {
                        proxy.scrollTo(index, anchor: .center)
                    } else {
                        withAnimation(.easeOut(duration: MacDesignTokens.Motion.selection)) {
                            proxy.scrollTo(index, anchor: .center)
                        }
                    }
                }
            }
        }
        .frame(width: paletteWidth, height: paletteHeight)
        .background(MacDesignColor.chrome)
        .clipShape(RoundedRectangle(cornerRadius: MacDesignTokens.Radius.floating, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.floating, style: .continuous)
                .stroke(MacDesignColor.borderSubtle, lineWidth: 0.5)
        }
        .shadow(color: Color.black.opacity(0.16), radius: 24, y: 10)
        .onAppear {
            searchFocused = true
            resetSelection()
        }
        .onChange(of: query) { _, _ in
            shouldScrollToSelection = true
            resetSelection()
        }
        .onChange(of: visibleItemCount) { _, count in
            selectedIndex = min(selectedIndex, max(count - 1, 0))
        }
        .onExitCommand(perform: dismiss)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(String(localized: "Command Palette"))
    }

    private var matchingConversations: [MacConversation] {
        guard !normalizedQuery.isEmpty else { return Array(model.conversations.prefix(5)) }
        return model.conversations.filter {
            $0.title.localizedCaseInsensitiveContains(normalizedQuery)
                || $0.preview.localizedCaseInsensitiveContains(normalizedQuery)
        }
    }

    private var displayedConversations: [MacConversation] {
        Array(matchingConversations.prefix(8))
    }

    private var visibleItemCount: Int {
        displayedConversations.count + matchingCommands.count
    }

    private var matchingCommands: [PaletteCommand] {
        let commands = allCommands
        guard !normalizedQuery.isEmpty else { return commands }
        return commands.filter {
            $0.title.localizedCaseInsensitiveContains(normalizedQuery)
                || $0.detail.localizedCaseInsensitiveContains(normalizedQuery)
        }
    }

    private var allCommands: [PaletteCommand] {
        [
            PaletteCommand(
                id: "new-conversation",
                title: String(localized: "New Conversation"),
                detail: String(localized: "Start a clean conversation"),
                symbol: "square.and.pencil",
                shortcut: "⌘N"
            ) {
                newConversation()
            },
            PaletteCommand(
                id: "focus-composer",
                title: String(localized: "Focus Composer"),
                detail: String(localized: "Move keyboard focus to the message field"),
                symbol: "text.cursor",
                shortcut: "⌘L"
            ) {
                dismiss()
                focusComposer()
            },
            PaletteCommand(
                id: "open-workspace",
                title: String(localized: "Open Workspace…"),
                detail: String(localized: "Choose the folder used by local tools"),
                symbol: "folder",
                shortcut: "⇧⌘O"
            ) {
                dismiss()
                model.chooseWorkspace()
            },
            PaletteCommand(
                id: "execution-local",
                title: String(localized: "This Mac"),
                detail: String(localized: "Run tools with Nola's local agent runtime."),
                symbol: "desktopcomputer",
                shortcut: nil
            ) {
                model.selectExecutionHost(.local)
                dismiss()
            },
            PaletteCommand(
                id: "execution-remote",
                title: String(localized: "Connected Host"),
                detail: String(localized: "Run tools through the selected connected host."),
                symbol: "network",
                shortcut: nil
            ) {
                model.selectExecutionHost(.remote)
                dismiss()
            },
            PaletteCommand(
                id: "toggle-sidebar",
                title: String(localized: "Toggle Sidebar"),
                detail: String(localized: "Show or hide conversations"),
                symbol: "sidebar.left",
                shortcut: "⌃⌘S"
            ) {
                toggleSidebar()
                dismiss()
            },
            PaletteCommand(
                id: "toggle-inspector",
                title: String(localized: "Toggle Inspector"),
                detail: String(localized: "Show or hide run context and tools"),
                symbol: "sidebar.right",
                shortcut: "⌥⌘I"
            ) {
                toggleInspector()
                dismiss()
            },
            PaletteCommand(
                id: "open-data-center",
                title: String(localized: "Data Center"),
                detail: String(localized: "Manage sources, memory, freshness, and provenance"),
                symbol: "square.stack.3d.up",
                shortcut: "⇧⌘D"
            ) {
                dismiss()
                openWindow(id: "data-center")
            },
            PaletteCommand(
                id: "open-automation-center",
                title: String(localized: "Automation Center"),
                detail: String(localized: "Manage schedules, triggers, and run history"),
                symbol: "clock.arrow.trianglehead.counterclockwise.rotate.90",
                shortcut: "⇧⌘A"
            ) {
                dismiss()
                openWindow(id: "automation-center")
            },
            PaletteCommand(
                id: "open-model-center",
                title: String(localized: "Model Center"),
                detail: String(localized: "Configure API and local models"),
                symbol: "cpu",
                shortcut: nil
            ) {
                dismiss()
                openWindow(id: "model-center")
            },
            PaletteCommand(
                id: "open-extensions-center",
                title: String(localized: "Skills & MCP"),
                detail: String(localized: "Browse and configure available capabilities"),
                symbol: "shippingbox",
                shortcut: "⇧⌘E"
            ) {
                dismiss()
                openWindow(id: "extensions-center")
            },
            PaletteCommand(
                id: "open-settings",
                title: String(localized: "Open Settings"),
                detail: String(localized: "Models, hosts, agent, and application defaults"),
                symbol: "gearshape",
                shortcut: "⌘,"
            ) {
                dismiss()
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            },
        ]
    }

    private func commandSectionID(_ command: PaletteCommand) -> String {
        switch command.id {
        case "new-conversation", "focus-composer", "open-workspace":
            return "conversation"
        case "execution-local", "execution-remote":
            return "execution"
        case "toggle-sidebar", "toggle-inspector":
            return "view"
        case "open-data-center", "open-automation-center", "open-model-center", "open-extensions-center":
            return "destinations"
        default:
            return "application"
        }
    }

    private func commandSectionTitle(_ command: PaletteCommand) -> String {
        switch commandSectionID(command) {
        case "conversation": return String(localized: "mac.command_palette.section.conversation")
        case "execution": return String(localized: "mac.command_palette.section.execution")
        case "view": return String(localized: "mac.command_palette.section.view")
        case "destinations": return String(localized: "mac.command_palette.section.destinations")
        default: return String(localized: "mac.command_palette.section.application")
        }
    }

    private func sectionLabel(_ title: String) -> some View {
        Text(String(localized: String.LocalizationValue(title)))
            .font(.system(size: 10.5, weight: .semibold))
            .foregroundStyle(.tertiary)
            .padding(.horizontal, 10)
            .padding(.top, 9)
            .padding(.bottom, 3)
    }

    private func paletteRow(
        index: Int,
        symbol: String,
        title: String,
        detail: String,
        shortcut: String?,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 11) {
                Image(systemName: symbol)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(selectedIndex == index ? MacDesignColor.brand : Color.secondary)
                    .frame(width: 22, height: 22)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.system(size: 12.5, weight: .medium))
                        .lineLimit(1)
                    if !detail.isEmpty {
                        Text(detail)
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                if let shortcut {
                    Text(shortcut)
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 10)
            .frame(height: 46)
            .macSelectionHighlight(isSelected: selectedIndex == index)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .id(index)
        .onHover { hovering in
            if hovering {
                shouldScrollToSelection = false
                if reduceMotion {
                    selectedIndex = index
                } else {
                    withAnimation(.easeOut(duration: MacDesignTokens.Motion.hover)) {
                        selectedIndex = index
                    }
                }
            }
        }
        .animation(
            reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.selection),
            value: selectedIndex == index
        )
        .accessibilityAddTraits(selectedIndex == index ? .isSelected : [])
    }

    private func resetSelection() {
        selectedIndex = 0
    }

    private func moveSelection(by delta: Int) {
        guard visibleItemCount > 0 else { return }
        shouldScrollToSelection = true
        selectedIndex = (selectedIndex + delta + visibleItemCount) % visibleItemCount
    }

    private func performSelected() {
        guard selectedIndex >= 0, selectedIndex < visibleItemCount else { return }
        if selectedIndex < displayedConversations.count {
            selectConversation(displayedConversations[selectedIndex].id)
            return
        }
        let commandIndex = selectedIndex - displayedConversations.count
        guard matchingCommands.indices.contains(commandIndex) else { return }
        matchingCommands[commandIndex].action()
    }

    private func keyHint(_ value: String) -> some View {
        Text(value)
            .font(.system(size: 9, weight: .semibold, design: .rounded))
            .foregroundStyle(.tertiary)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 5))
    }

    private var paletteWidth: CGFloat {
        let available = NSScreen.main?.visibleFrame.width ?? 1_280
        return min(680, max(480, available * 0.48))
    }

    private var paletteHeight: CGFloat {
        let available = NSScreen.main?.visibleFrame.height ?? 800
        return min(600, max(380, available * 0.58))
    }
}

private struct PaletteCommand: Identifiable {
    let id: String
    let title: String
    let detail: String
    let symbol: String
    let shortcut: String?
    let action: () -> Void
}
