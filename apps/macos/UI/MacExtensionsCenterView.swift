// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents built-in, workspace, and connected-host Skills, MCP, agents, commands, and plugins.

import AppKit
import Combine
import SwiftUI

private func macExtensionsText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    if keyed != key {
        return keyed
    }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

private func macMCPStateText(_ state: MacMCPCapabilitySummary.State) -> String {
    switch state {
    case .configured:
        return macExtensionsText("mac.extensions.mcp.configured", "Configured")
    case .connected:
        return macExtensionsText("mac.extensions.mcp.connected", "Connected")
    case .authorizationRequired:
        return macExtensionsText("mac.extensions.mcp.authorization_required", "Authorization Required")
    case .unavailable:
        return String(localized: "Unavailable")
    }
}

private func macMCPSummaryText(_ summary: MacMCPCapabilitySummary) -> String {
    let counts = "\(summary.tools.count) \(String(localized: "Tools")) · "
        + "\(summary.resources.count) \(macExtensionsText("mac.extensions.mcp.resources", "Resources")) · "
        + "\(summary.resourceTemplates.count) \(macExtensionsText("mac.extensions.mcp.templates", "Templates")) · "
        + "\(summary.prompts.count) \(macExtensionsText("mac.extensions.mcp.prompts", "Prompts"))"
    guard summary.state == .connected else { return macMCPStateText(summary.state) }
    return "\(macMCPStateText(summary.state)) · \(counts)"
}

final class MacExtensionsNavigationState: ObservableObject {
    @Published var selectedID: String?
    @Published var sourceFilter: MacExtensionsCenterView.SourceFilter = .thisMac
    @Published var query = ""
    // The active editor owns its draft and decides whether leaving is safe.
    // Navigation forwards intent; it never mirrors dirty or saving state.
    var leaveHandler: ((@escaping () -> Void) -> Void)?

    func requestLeave(_ action: @escaping () -> Void) {
        if let leaveHandler { leaveHandler(action) } else { action() }
    }
}

@MainActor
final class MacExtensionEditorExitState: ObservableObject {
    @Published private(set) var needsConfirmation = false
    private var pendingAction: (() -> Void)?

    func request(hasChanges: Bool, isSaving: Bool, action: @escaping () -> Void) {
        guard !isSaving, pendingAction == nil else { return }
        guard hasChanges else { action(); return }
        pendingAction = action
        needsConfirmation = true
    }

    func cancel() {
        pendingAction = nil
        needsConfirmation = false
    }

    func discard(isSaving: Bool, discardDraft: () -> Void) {
        guard !isSaving, let action = pendingAction else { return }
        cancel()
        discardDraft()
        action()
    }
}

struct MacExtensionsCenterView: View {
    enum SourceFilter: String, CaseIterable, Identifiable {
        case all
        case thisMac
        case connectedHost

        var id: String { rawValue }
        var title: String {
            switch self {
            case .all: return macExtensionsText("mac.extensions.source.all", "All")
            case .thisMac: return macExtensionsText("mac.extensions.source.this_mac", "This Mac")
            case .connectedHost: return macExtensionsText("mac.extensions.source.connected_host", "Connected Host")
            }
        }
    }

    @StateObject private var catalog: MacExtensionsCatalogLoader
    private var items: [MacExtensionItem] { catalog.items }
    @StateObject private var navigation: MacExtensionsNavigationState
    private var selectedID: String? {
        get { navigation.selectedID }
        nonmutating set { navigation.selectedID = newValue }
    }
    private var sourceFilter: SourceFilter { navigation.sourceFilter }
    private var query: String {
        get { navigation.query }
        nonmutating set { navigation.query = newValue }
    }
    private var isLoading: Bool { catalog.isLoading }
    private var errorMessage: String? {
        get { catalog.errorMessage }
        nonmutating set { catalog.errorMessage = newValue }
    }
    @State private var pendingRemoval: MacExtensionItem?
    private var hasRemovedBuiltInSkills: Bool { catalog.hasRemovedBuiltInSkills }
    @State private var workspaceURL = MacSecurityScopedBookmarkStore.shared.restoreWorkspace()
    @State private var chromeSnapshot = MacChromeMessagingService.shared.snapshot()
    @State private var isChromeManagerPresented = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    private let visibleKinds: [MacExtensionItem.Kind]
    let navigationAction: (() -> Void)?

    init(
        remoteHost: any MacRemoteHostManaging,
        kinds: [MacExtensionItem.Kind] = MacExtensionItem.Kind.allCases,
        navigationAction: (() -> Void)? = nil,
        navigation: MacExtensionsNavigationState = MacExtensionsNavigationState(),
        catalog: MacExtensionsCatalogLoader? = nil
    ) {
        _catalog = StateObject(wrappedValue: catalog ?? MacExtensionsCatalogLoader(
            local: { try await MacExtensionsService.shared.localExtensions(workspaceURL: $0) },
            host: {
                guard await remoteHost.isConfigured() else { return [] }
                return try await remoteHost.fetchExtensions()
            },
            removedSkills: { try MacBuiltInSkillRegistry.hasRemovedSkills() }
        ))
        visibleKinds = kinds
        self.navigationAction = navigationAction
        _navigation = StateObject(wrappedValue: navigation)
    }

    var body: some View {
        Group {
            if let selectedItem {
                MacExtensionDetail(
                    item: selectedItem,
                    navigation: navigation,
                    externalErrorMessage: $catalog.errorMessage,
                    onClose: { selectedID = nil },
                    onEnabledChange: { setEnabled($0, for: selectedItem) },
                    onRemove: { pendingRemoval = selectedItem }
                )
                .id(selectedItem.id)
            } else {
                catalogContent
            }
        }
        .toolbar(removing: .sidebarToggle)
        .task { await load(includeHost: sourceFilter != .thisMac) }
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeBridgeStatusDidChange)
                .receive(on: RunLoop.main)
        ) { _ in
            chromeSnapshot = MacChromeMessagingService.shared.snapshot()
        }
        .onChange(of: sourceFilter) { _, source in
            Task { await load(includeHost: source != .thisMac) }
        }
        .confirmationDialog(
            macExtensionsText("mac.extensions.remove.title", "Remove extension?"),
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { if !$0 { pendingRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingRemoval
        ) { item in
            Button(macExtensionsText("mac.extensions.action.cancel", "Cancel"), role: .cancel) {
                pendingRemoval = nil
            }
            Button(macExtensionsText("mac.extensions.action.remove", "Remove"), role: .destructive) {
                remove(item)
            }
        } message: { item in
            Text(String(format: macExtensionsText("mac.extensions.remove.message", "Remove %1$@ from this Mac? You can restore built-in Skills later."), item.title))
        }
        .sheet(isPresented: $isChromeManagerPresented) {
            MacChromeExtensionManagerView()
                .frame(minWidth: 560, minHeight: 430)
        }
        .onDisappear {
            catalog.invalidate()
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: errorMessage)
    }

    private var catalogContent: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                LocalizedStringKey(navigationTitle),
                symbol: "shippingbox",
                navigationAction: navigationAction
            ) {
                Text(macExtensionsText("mac.extensions.subtitle", "Browse, configure, and edit tools available to this Mac."))
            } actions: {
                if showsBrowserIntegration {
                    Button {
                        isChromeManagerPresented = true
                    } label: {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: "globe")
                            Circle()
                                .fill(chromeSnapshot.isExtensionConnected ? Color.green : Color.orange)
                                .frame(width: 7, height: 7)
                                .offset(x: 4, y: -3)
                        }
                    }
                    .macToolbarIcon()
                    .help(chromeConnectionDescription)
                    .accessibilityLabel(macExtensionsText("mac.extensions.chrome.title", "Chrome Extension"))
                    .accessibilityValue(chromeConnectionDescription)
                }
                Button(action: refresh) {
                    Image(systemName: "arrow.clockwise")
                }
                .macToolbarIcon()
                .disabled(isLoading)
                .help(macExtensionsText("mac.extensions.action.refresh", "Refresh Extensions"))
                .accessibilityLabel(macExtensionsText("mac.extensions.action.refresh", "Refresh Extensions"))
            }

            if let errorMessage {
                MacInlineNotice(
                    title: macExtensionsText("mac.extensions.error.title", "Extensions Error"),
                    message: errorMessage,
                    kind: .failure,
                    dismiss: { self.errorMessage = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            if isLoading && items.isEmpty {
                MacLoadingState(
                    macExtensionsText("mac.extensions.loading", "Discovering extensions…")
                )
            } else {
                catalogToolbar
                Divider().opacity(0.55)
                MacExtensionGallery(
                    items: filteredItems,
                    emptyDescription: emptyDescription,
                    onSelect: { selectedID = $0.id },
                    onEnabledChange: { item, enabled in
                        setEnabled(enabled, for: item)
                    },
                    onRemove: { pendingRemoval = $0 }
                )
            }
        }
        .background(MacPageBackground())
    }

    private var catalogToolbar: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                sourcePicker
                scopeMenu
                Spacer(minLength: 8)
                searchField.frame(width: 248)
            }

            VStack(spacing: 10) {
                HStack(spacing: 10) {
                    sourcePicker
                    Spacer(minLength: 6)
                    scopeMenu
                }
                searchField.frame(maxWidth: .infinity)
            }
        }
        .padding(.horizontal, MacDesignTokens.Spacing.xl)
        .padding(.vertical, MacDesignTokens.Spacing.md)
        .background {
            Rectangle()
                .fill(reduceTransparency ? AnyShapeStyle(MacDesignColor.chrome) : AnyShapeStyle(.ultraThinMaterial))
                .overlay(MacDesignColor.chrome.opacity(0.72))
        }
    }

    @ViewBuilder
    private var scopeMenu: some View {
        Menu {
            Button {
                Task {
                    if let selected = await MacSecurityScopedBookmarkStore.shared.chooseWorkspace() {
                        workspaceURL = selected
                        refresh()
                    }
                }
            } label: {
                Label(
                    macExtensionsText("mac.extensions.action.choose_workspace", "Choose Workspace"),
                    systemImage: "folder.badge.plus"
                )
            }
            if hasRemovedBuiltInSkills {
                Divider()
                Button {
                    do {
                        try MacBuiltInSkillRegistry.restoreBuiltInSkills()
                        refresh()
                    } catch {
                        errorMessage = error.localizedDescription
                    }
                } label: {
                    Label(
                        macExtensionsText("mac.extensions.action.reset", "Reset"),
                        systemImage: "arrow.uturn.backward.circle"
                    )
                }
            }
        } label: {
            Label(
                workspaceURL?.lastPathComponent
                    ?? macExtensionsText("mac.extensions.action.choose_workspace", "Choose Workspace"),
                systemImage: "folder"
            )
            .font(.system(size: 12, weight: .medium))
            .lineLimit(1)
        }
        .menuStyle(.borderlessButton)
        .foregroundStyle(workspaceURL == nil ? Color.secondary : MacDesignColor.brand)
        .help(macExtensionsText("mac.extensions.action.choose_workspace", "Choose Workspace"))
        .accessibilityLabel(macExtensionsText("mac.extensions.action.choose_workspace", "Choose Workspace"))
    }

    private var sourcePicker: some View {
        ViewThatFits(in: .horizontal) {
            Picker(macExtensionsText("mac.extensions.source.label", "Source"), selection: $navigation.sourceFilter) {
                ForEach(SourceFilter.allCases) { filter in
                    Text(filter.title).tag(filter)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .frame(width: 228)

            Picker(macExtensionsText("mac.extensions.source.label", "Source"), selection: $navigation.sourceFilter) {
                ForEach(SourceFilter.allCases) { filter in
                    Text(filter.title).tag(filter)
                }
            }
            .labelsHidden()
            .pickerStyle(.menu)
        }
    }

    private var searchField: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
            TextField(macExtensionsText("mac.extensions.search", "Search Skills and MCP"), text: $navigation.query)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
                .accessibilityLabel(macExtensionsText("mac.extensions.search.clear", "Clear search"))
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, query.isEmpty ? 12 : 4)
        .frame(height: 36)
        .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10))
    }

    private var chromeConnectionDescription: String {
        if chromeSnapshot.isExtensionConnected {
            return macExtensionsText("mac.extensions.chrome.connected", "Connected to Chrome")
        }
        if !chromeSnapshot.isExtensionPrepared {
            return macExtensionsText("mac.extensions.chrome.unavailable", "Extension files are unavailable")
        }
        return macExtensionsText("mac.extensions.chrome.ready", "Ready to connect")
    }

    private var filteredItems: [MacExtensionItem] {
        let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return catalogItems.filter { item in
            let sourceMatches: Bool
            switch sourceFilter {
            case .all: sourceMatches = true
            case .thisMac: sourceMatches = item.source != .connectedHost
            case .connectedHost: sourceMatches = item.source == .connectedHost
            }
            guard sourceMatches else { return false }
            return normalized.isEmpty
                || item.title.localizedCaseInsensitiveContains(normalized)
                || item.detail.localizedCaseInsensitiveContains(normalized)
        }
    }

    private var catalogItems: [MacExtensionItem] {
        items.filter { visibleKinds.contains($0.kind) }
    }

    private var navigationTitle: String {
        Set(visibleKinds) == Set([.skill, .mcp])
            ? macExtensionsText("mac.extensions.title.skills_mcp", "Skills and MCP")
            : macExtensionsText("mac.extensions.title.all", "Extensions")
    }

    private var showsBrowserIntegration: Bool {
        Set(visibleKinds) != Set([.skill, .mcp])
    }

    private var selectedItem: MacExtensionItem? {
        guard let selectedID else { return nil }
        return items.first { $0.id == selectedID }
    }

    private var emptyDescription: String {
        if sourceFilter == .connectedHost {
            return macExtensionsText("mac.extensions.empty.host", "Configure Connected Host, then refresh to load its managed extensions.")
        }
        return macExtensionsText("mac.extensions.empty.local", "Built-in Skills and extensions from the selected workspace appear here.")
    }

    private func refresh() {
        Task { await load(includeHost: sourceFilter != .thisMac) }
    }

    private func load(includeHost: Bool) async {
        await catalog.load(includeHost: includeHost, workspaceURL: workspaceURL)
        guard !Task.isCancelled, !catalog.isLoading, catalog.errorMessage == nil else { return }
        if let selectedID, !filteredItems.contains(where: { $0.id == selectedID }) {
            self.selectedID = nil
        }
    }

    @ViewBuilder
    private func contextMenu(for item: MacExtensionItem) -> some View {
        if item.source == .builtIn {
            Text(macExtensionsText("mac.extensions.context.included", "Included with Nola"))
            Toggle(
                macExtensionsText("mac.extensions.metric.enabled", "Enabled"),
                isOn: Binding(
                    get: { item.isEnabled },
                    set: { setEnabled($0, for: item) }
                )
            )
            Divider()
            Button(macExtensionsText("mac.extensions.action.remove", "Remove"), role: .destructive) {
                pendingRemoval = item
            }
        } else if let path = item.path {
            let url = URL(fileURLWithPath: path)
            if item.kind == .skill || item.kind == .mcp {
                Toggle(
                    macExtensionsText("mac.extensions.metric.enabled", "Enabled"),
                    isOn: Binding(
                        get: { item.isEnabled },
                        set: { setEnabled($0, for: item) }
                    )
                )
                Divider()
            }
            Button(macExtensionsText("mac.extensions.action.open", "Open")) { NSWorkspace.shared.open(url) }
            Button(macExtensionsText("mac.extensions.action.reveal", "Reveal in Finder")) { NSWorkspace.shared.activateFileViewerSelecting([url]) }
            Divider()
            Button(macExtensionsText("mac.extensions.action.copy_path", "Copy Path")) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(path, forType: .string)
            }
        } else {
            Text(macExtensionsText("mac.extensions.context.managed_host", "Managed by Connected Host"))
        }
    }

    private func setEnabled(_ enabled: Bool, for item: MacExtensionItem) {
        if let id = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) {
            do {
                try MacBuiltInSkillRegistry.setEnabled(enabled, for: id)
            } catch {
                errorMessage = error.localizedDescription
                return
            }
        } else if item.kind == .skill, item.source != .connectedHost {
            do {
                try MacLocalSkillActivationStore.setEnabled(enabled, itemID: item.id)
            } catch {
                errorMessage = error.localizedDescription
                return
            }
        } else if item.kind == .mcp, item.source != .connectedHost {
            do {
                try MacLocalMCPActivationStore.setEnabled(enabled, itemID: item.id)
            } catch {
                errorMessage = error.localizedDescription
                return
            }
        } else {
            return
        }
        refresh()
    }

    private func remove(_ item: MacExtensionItem) {
        pendingRemoval = nil
        guard let id = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) else { return }
        do {
            try MacBuiltInSkillRegistry.remove(id)
        } catch {
            errorMessage = error.localizedDescription
            return
        }
        refresh()
    }
}

struct MacExtensionCatalogSection: Identifiable, Equatable {
    let kind: MacExtensionItem.Kind
    let functionalCategory: MacSkillFunctionalCategory?
    let items: [MacExtensionItem]

    var id: String { functionalCategory?.rawValue ?? kind.rawValue }
    var title: String { functionalCategory?.localizedTitle ?? kind.title }
    var symbol: String { functionalCategory?.symbol ?? kind.symbol }
}

enum MacExtensionCatalog {
    static func sections(for items: [MacExtensionItem]) -> [MacExtensionCatalogSection] {
        var sections = MacSkillFunctionalCategory.allCases.compactMap { category in
            let matching = sorted(items.filter { item in
                guard item.kind == .skill else { return false }
                guard let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) else {
                    return category == .custom
                }
                return skillID.functionalCategory == category
            })
            return matching.isEmpty ? nil : MacExtensionCatalogSection(
                kind: .skill,
                functionalCategory: category,
                items: matching
            )
        }
        sections.append(contentsOf: MacExtensionItem.Kind.allCases.compactMap { kind in
            guard kind != .skill else { return nil }
            let matching = sorted(items.filter { $0.kind == kind })
            return matching.isEmpty ? nil : MacExtensionCatalogSection(
                kind: kind,
                functionalCategory: nil,
                items: matching
            )
        })
        return sections
    }

    private static func sorted(_ items: [MacExtensionItem]) -> [MacExtensionItem] {
        items.sorted { lhs, rhs in
            if lhs.source != rhs.source {
                if lhs.source == .builtIn { return true }
                if rhs.source == .builtIn { return false }
                return lhs.source.rawValue < rhs.source.rawValue
            }
            return lhs.title.localizedCaseInsensitiveCompare(rhs.title) == .orderedAscending
        }
    }
}

struct MacExtensionGallery: View {
    let items: [MacExtensionItem]
    let emptyDescription: String
    let onSelect: (MacExtensionItem) -> Void
    let onEnabledChange: (MacExtensionItem, Bool) -> Void
    let onRemove: (MacExtensionItem) -> Void

    private var sections: [MacExtensionCatalogSection] {
        MacExtensionCatalog.sections(for: items)
    }

    var body: some View {
        if sections.isEmpty {
            MacEmptyState(
                LocalizedStringKey(macExtensionsText("mac.extensions.empty.title", "No Extensions")),
                description: LocalizedStringKey(emptyDescription),
                symbol: "shippingbox"
            ) {}
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 32) {
                    ForEach(sections) { section in
                        VStack(alignment: .leading, spacing: 14) {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Image(systemName: section.symbol)
                                    .font(.system(size: 13, weight: .semibold))
                                    .foregroundStyle(MacDesignColor.brand)
                                    .accessibilityHidden(true)
                                Text(section.title)
                                    .font(.system(size: 16, weight: .semibold))
                                Text("\(section.items.count)")
                                    .font(.system(size: 11, weight: .medium, design: .rounded))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 7)
                                    .frame(height: 20)
                                    .background(
                                        Color.primary.opacity(0.045),
                                        in: Capsule(style: .continuous)
                                    )
                            }

                            LazyVGrid(
                                columns: [GridItem(.adaptive(minimum: 300, maximum: 420), spacing: 14)],
                                alignment: .leading,
                                spacing: 14
                            ) {
                                ForEach(section.items) { item in
                                    MacExtensionGalleryCard(
                                        item: item,
                                        onEnabledChange: item.source == .builtIn
                                            ? { onEnabledChange(item, $0) }
                                            : nil,
                                        action: { onSelect(item) }
                                    )
                                    .contextMenu {
                                        if item.source == .builtIn {
                                            Button {
                                                onEnabledChange(item, !item.isEnabled)
                                            } label: {
                                                Label(
                                                    item.isEnabled
                                                        ? macExtensionsText("mac.extensions.status.disabled", "Disabled")
                                                        : macExtensionsText("mac.extensions.status.enabled", "Enabled"),
                                                    systemImage: item.isEnabled ? "pause.circle" : "play.circle"
                                                )
                                            }
                                            Divider()
                                            Button(role: .destructive) {
                                                onRemove(item)
                                            } label: {
                                                Label(macExtensionsText("mac.extensions.action.remove", "Remove"), systemImage: "trash")
                                            }
                                        } else if let path = item.path {
                                            let url = URL(fileURLWithPath: path)
                                            Button {
                                                NSWorkspace.shared.open(url)
                                            } label: {
                                                Label(macExtensionsText("mac.extensions.action.open", "Open"), systemImage: "arrow.up.forward.app")
                                            }
                                            Button {
                                                NSWorkspace.shared.activateFileViewerSelecting([url])
                                            } label: {
                                                Label(macExtensionsText("mac.extensions.action.reveal", "Reveal in Finder"), systemImage: "folder")
                                            }
                                            Button {
                                                NSPasteboard.general.clearContents()
                                                NSPasteboard.general.setString(path, forType: .string)
                                            } label: {
                                                Label(macExtensionsText("mac.extensions.action.copy_path", "Copy Path"), systemImage: "doc.on.doc")
                                            }
                                        } else {
                                            Text(macExtensionsText("mac.extensions.context.managed_host", "Managed by Connected Host"))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(MacDesignTokens.Spacing.xxl)
                .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                .frame(maxWidth: .infinity)
            }
        }
    }

}

private struct MacExtensionGalleryCard: View {
    let item: MacExtensionItem
    let onEnabledChange: ((Bool) -> Void)?
    let action: () -> Void
    @State private var isHovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            Button(action: action) {
                VStack(alignment: .leading, spacing: 16) {
                    HStack(alignment: .top, spacing: 12) {
                        Image(systemName: cardSymbol)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(cardTint)
                            .frame(width: 40, height: 40)
                            .background(
                                MacDesignColor.surfaceRaised,
                                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                            )
                        VStack(alignment: .leading, spacing: 4) {
                            Text(item.title)
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            Text(categoryTitle)
                                .font(.system(size: 12, weight: .medium))
                                .foregroundStyle(cardTint)
                                .lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Text(item.detail.isEmpty ? item.kind.title : item.detail)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .frame(maxWidth: .infinity, minHeight: 46, alignment: .topLeading)

                    if let summary = item.mcpSummary {
                        Text(macMCPSummaryText(summary))
                            .font(.system(size: 12, weight: .medium))
                            .foregroundStyle(summary.state == .connected ? Color.green : Color.secondary)
                            .lineLimit(2)
                    }

                }
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, 13)
                .frame(maxWidth: .infinity, minHeight: 140, alignment: .topLeading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("mac.extensions.card.\(item.id)")

            HStack(spacing: 8) {
                Label(item.source.title, systemImage: sourceSymbol)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if let onEnabledChange {
                    Toggle(
                        macExtensionsText("mac.extensions.metric.enabled", "Enabled"),
                        isOn: Binding(
                            get: { item.isEnabled },
                            set: onEnabledChange
                        )
                    )
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                    .help(
                        item.isEnabled
                            ? macExtensionsText("mac.extensions.status.disabled", "Disabled")
                            : macExtensionsText("mac.extensions.status.enabled", "Enabled")
                    )
                    .accessibilityLabel(macExtensionsText("mac.extensions.metric.enabled", "Enabled"))
                    .accessibilityValue(
                        item.isEnabled
                            ? macExtensionsText("mac.extensions.status.enabled", "Enabled")
                            : macExtensionsText("mac.extensions.status.disabled", "Disabled")
                    )
                    .accessibilityIdentifier("mac.extensions.toggle.\(item.id)")
                } else {
                    MacStatusBadge(
                        title: item.isEnabled
                            ? macExtensionsText("mac.extensions.status.enabled", "Enabled")
                            : macExtensionsText("mac.extensions.status.disabled", "Disabled"),
                        tint: item.isEnabled ? Color.green : Color.secondary,
                        symbol: item.isEnabled ? "checkmark" : "power"
                    )
                }
                Button(action: action) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(isHovering ? MacDesignColor.brand : Color.secondary)
                        .frame(width: 22, height: 22)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(macExtensionsText("mac.extensions.action.open_details", "Open details"))
            }
            .padding(.horizontal, 14)
            .frame(height: 42)
        }
        .frame(maxWidth: .infinity, minHeight: 184, alignment: .topLeading)
        .background(
            isHovering ? MacDesignColor.brand.opacity(0.065) : Color.primary.opacity(0.03),
            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(
                    isHovering ? MacDesignColor.brand.opacity(0.32) : Color.primary.opacity(0.075),
                    lineWidth: 0.75
                )
        }
        .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .onHover { hovering in
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.16)) {
                isHovering = hovering
            }
        }
        .accessibilityLabel("\(item.title), \(item.source.title)")
        .accessibilityHint(macExtensionsText("mac.extensions.action.open_details", "Open details"))
    }

    private var skillCategory: MacSkillFunctionalCategory? {
        MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id)?.functionalCategory
    }

    private var cardSymbol: String {
        skillCategory?.symbol ?? item.kind.symbol
    }

    private var categoryTitle: String {
        skillCategory?.localizedTitle ?? item.kind.title
    }

    private var cardTint: Color {
        if item.source == .connectedHost { return .orange }
        switch skillCategory {
        case .documents: return .blue
        case .dataAnalysis: return .green
        case .designPresentations: return .pink
        case .communication: return .cyan
        case .planningAutomation: return .orange
        case .knowledgeResearch: return .indigo
        case .workspace: return .teal
        case .custom, .none: return MacDesignColor.brand
        }
    }

    private var sourceSymbol: String {
        switch item.source {
        case .builtIn: return "shippingbox.fill"
        case .workspace: return "folder"
        case .global: return "desktopcomputer"
        case .connectedHost: return "network"
        }
    }
}

private struct MacChromeExtensionManagerView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var snapshot = MacChromeMessagingService.shared.snapshot()
    @State private var errorMessage: String?
    @State private var checkMessage: String?
    @State private var isPerformingAction = false
    private let playwright = MacPlaywrightRuntimeService.shared.snapshot()

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "network")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 42, height: 42)
                    .background(
                        Color.accentColor.opacity(0.10),
                        in: RoundedRectangle(cornerRadius: 12)
                    )
                VStack(alignment: .leading, spacing: 3) {
                    Text(macExtensionsText("mac.extensions.chrome.title", "Chrome Extension"))
                        .font(.system(size: 18, weight: .semibold))
                    Text(macExtensionsText("mac.extensions.chrome.subtitle", "Chrome connects directly to Nola for Mac."))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button(macExtensionsText("mac.extensions.action.done", "Done")) { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(22)

            Divider()

            if let errorMessage {
                MacInlineNotice(
                    title: macExtensionsText("mac.extensions.error.title", "Extensions Error"),
                    message: errorMessage,
                    kind: .failure,
                    dismiss: { self.errorMessage = nil }
                )
                .padding(.horizontal, 22)
                .padding(.top, 14)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    HStack(spacing: 14) {
                        ZStack {
                            Circle().fill(Color.blue.opacity(0.12))
                            Image(systemName: "safari")
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(Color.blue)
                        }
                        .frame(width: 44, height: 44)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(macExtensionsText("mac.extensions.browser.managed.title", "Nola Managed Browser"))
                                .font(.system(size: 14, weight: .semibold))
                            Text(playwright.isAvailable
                                 ? macExtensionsText("mac.extensions.browser.managed.ready", "Playwright is built in. Nola opens an isolated Chrome session when a browser task needs it.")
                                 : macExtensionsText("mac.extensions.browser.managed.unavailable", "The Playwright runtime is missing from this build."))
                                .font(.system(size: 11.5))
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer()
                        Label(
                            playwright.isAvailable
                                ? macExtensionsText("mac.extensions.browser.managed.available", "Available")
                                : macExtensionsText("mac.extensions.browser.managed.missing", "Missing"),
                            systemImage: playwright.isAvailable ? "checkmark.circle.fill" : "exclamationmark.triangle.fill"
                        )
                        .font(.system(size: 10.5, weight: .semibold))
                        .foregroundStyle(playwright.isAvailable ? Color.green : Color.orange)
                    }
                    .padding(17)
                    .background(Color.blue.opacity(0.045), in: RoundedRectangle(cornerRadius: 14))

                    HStack(alignment: .firstTextBaseline, spacing: 9) {
                        Image(systemName: "arrow.triangle.branch")
                            .foregroundStyle(.secondary)
                        Text(macExtensionsText("mac.extensions.browser.paths", "Use the managed browser for reliable automation. Use the Chrome extension when Nola must work with your current tabs or signed-in Chrome session."))
                            .font(.system(size: 11.5))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 4)

                    Divider()

                    HStack(spacing: 14) {
                        ZStack {
                            Circle().fill(phaseColor.opacity(0.12))
                            Image(systemName: phaseSymbol)
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(phaseColor)
                        }
                        .frame(width: 44, height: 44)

                        VStack(alignment: .leading, spacing: 4) {
                            Text(phaseTitle)
                                .font(.system(size: 14, weight: .semibold))
                            Text(phaseDetail)
                                .font(.system(size: 11.5))
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 20)
                        Button(action: performPrimaryAction) {
                            HStack(spacing: 7) {
                                if isPerformingAction {
                                    ProgressView().controlSize(.small)
                                }
                                Text(primaryActionTitle)
                            }
                            .frame(minWidth: 112)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(isPerformingAction)
                    }
                    .padding(17)
                    .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))

                    HStack(spacing: 0) {
                        connectionStage(
                            title: macExtensionsText("mac.extensions.chrome.mac_service", "Mac Service"),
                            isComplete: snapshot.isServiceRunning
                        )
                        stageConnector(isComplete: snapshot.isServiceRunning)
                        connectionStage(
                            title: macExtensionsText("mac.extensions.chrome.native_connector", "Native Connector"),
                            isComplete: snapshot.isNativeHostInstalled && snapshot.isExtensionPrepared
                        )
                        stageConnector(isComplete: snapshot.isExtensionConnected)
                        connectionStage(
                            title: macExtensionsText("mac.extensions.chrome.browser", "Chrome"),
                            isComplete: snapshot.isExtensionConnected
                        )
                    }
                    .padding(.horizontal, 8)

                    if !snapshot.isExtensionConnected {
                        VStack(alignment: .leading, spacing: 10) {
                        Text(macExtensionsText("mac.extensions.chrome.connect", "Connect Chrome"))
                            .font(.system(size: 13, weight: .semibold))
                        instruction(
                            number: 1,
                            text: macExtensionsText("mac.extensions.chrome.step.open", "Open Chrome Extensions and enable Developer mode.")
                        )
                        instruction(
                            number: 2,
                            text: macExtensionsText("mac.extensions.chrome.step.load", "Choose Load unpacked and select Nola's extension folder.")
                        )
                        instruction(
                            number: 3,
                            text: macExtensionsText("mac.extensions.chrome.step.keep_running", "Keep Nola for Mac running while using browser tools.")
                        )
                        }
                        .padding(16)
                        .background(Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 12))
                    } else if !snapshot.capabilities.isEmpty {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(macExtensionsText("mac.extensions.chrome.available_actions", "Available actions"))
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.secondary)
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 6) {
                                    ForEach(snapshot.capabilities, id: \.self) { capability in
                                        Text(capability.replacingOccurrences(of: "_", with: " "))
                                            .font(.system(size: 9.5, weight: .medium))
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 5)
                                            .background(Color.green.opacity(0.08), in: Capsule())
                                    }
                                }
                            }
                        }
                    }

                    DisclosureGroup(macExtensionsText("mac.extensions.chrome.advanced", "Setup and diagnostics")) {
                        HStack(spacing: 10) {
                        Button {
                            if !MacChromeMessagingService.shared.openChromeExtensions() {
                                errorMessage = macExtensionsText("mac.extensions.chrome.chrome_missing", "Google Chrome is not installed.")
                            }
                        } label: {
                            Label(macExtensionsText("mac.extensions.chrome.open_extensions", "Open Chrome Extensions"), systemImage: "arrow.up.forward.app")
                        }
                        .buttonStyle(.borderedProminent)

                        Button {
                            MacChromeMessagingService.shared.revealExtension()
                        } label: {
                            Label(macExtensionsText("mac.extensions.chrome.show_folder", "Show Extension Folder"), systemImage: "folder")
                        }
                        .buttonStyle(.bordered)

                        Spacer()

                        Button {
                            do {
                                try MacChromeMessagingService.shared.reinstall()
                                refresh()
                            } catch {
                                errorMessage = error.localizedDescription
                            }
                        } label: {
                            Label(macExtensionsText("mac.extensions.chrome.repair", "Repair"), systemImage: "wrench.and.screwdriver")
                        }
                        .buttonStyle(.borderless)
                        }
                        .padding(.top, 10)
                    }
                    .disclosureGroupStyle(MacRowDisclosureStyle())
                    .font(.system(size: 11.5, weight: .medium))

                    if let checkMessage {
                        Label(checkMessage, systemImage: snapshot.isExtensionConnected ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                            .font(.system(size: 10.5))
                            .foregroundStyle(snapshot.isExtensionConnected ? Color.green : Color.orange)
                            .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    if let lastSeen = snapshot.lastSeen {
                        Text(String(
                            format: macExtensionsText("mac.extensions.chrome.last_connected", "Last connected %@"),
                            lastSeen.formatted(date: .abbreviated, time: .standard)
                        ))
                            .font(.system(size: 10.5))
                            .foregroundStyle(.tertiary)
                    }
                }
                .padding(22)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeBridgeStatusDidChange)
                .receive(on: RunLoop.main)
        ) { _ in
            refresh()
        }
        .task { refresh() }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: errorMessage)
    }

    private var phaseTitle: String {
        switch snapshot.phase {
        case .connected: macExtensionsText("mac.extensions.chrome.phase.connected", "Chrome is ready")
        case .waitingForChrome: macExtensionsText("mac.extensions.chrome.phase.waiting", "Waiting for Chrome")
        case .installationRequired: macExtensionsText("mac.extensions.chrome.phase.install", "Connection setup required")
        case .serviceStopped: macExtensionsText("mac.extensions.chrome.phase.stopped", "Browser service is stopped")
        case .chromeUnavailable: macExtensionsText("mac.extensions.chrome.phase.unavailable", "Chrome is unavailable")
        case .failed: macExtensionsText("mac.extensions.chrome.phase.failed", "Browser connection needs attention")
        }
    }

    private var phaseDetail: String {
        if let error = snapshot.errorMessage, !error.isEmpty { return error }
        switch snapshot.phase {
        case .connected: return macExtensionsText("mac.extensions.chrome.detail.connected", "Nola can inspect and operate your active Chrome tabs.")
        case .waitingForChrome: return macExtensionsText("mac.extensions.chrome.detail.waiting", "Open Chrome and enable the Nola extension to finish connecting.")
        case .installationRequired: return macExtensionsText("mac.extensions.chrome.detail.install", "Install the local connector before enabling the extension in Chrome.")
        case .serviceStopped: return macExtensionsText("mac.extensions.chrome.detail.stopped", "Start the local browser service. It only listens on this Mac.")
        case .chromeUnavailable: return macExtensionsText("mac.extensions.chrome.detail.unavailable", "Install Google Chrome before configuring browser control.")
        case .failed: return macExtensionsText("mac.extensions.chrome.detail.failed", "Repair the connection and run the test again.")
        }
    }

    private var phaseColor: Color {
        switch snapshot.phase {
        case .connected: .green
        case .waitingForChrome, .installationRequired, .serviceStopped: .orange
        case .chromeUnavailable, .failed: .red
        }
    }

    private var phaseSymbol: String {
        switch snapshot.phase {
        case .connected: "checkmark.circle.fill"
        case .waitingForChrome: "network"
        case .installationRequired: "puzzlepiece.extension"
        case .serviceStopped: "power"
        case .chromeUnavailable: "exclamationmark.triangle.fill"
        case .failed: "wrench.and.screwdriver.fill"
        }
    }

    private var primaryActionTitle: String {
        switch snapshot.phase {
        case .connected: macExtensionsText("mac.extensions.chrome.test", "Test connection")
        case .waitingForChrome: macExtensionsText("mac.extensions.chrome.open_extensions", "Open Chrome Extensions")
        case .chromeUnavailable: macExtensionsText("mac.extensions.chrome.open_download", "Open Chrome website")
        default: macExtensionsText("mac.extensions.chrome.start", "Set up connection")
        }
    }

    private func connectionStage(title: String, isComplete: Bool) -> some View {
        VStack(spacing: 6) {
            Image(systemName: isComplete ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(isComplete ? Color.green : Color.secondary.opacity(0.55))
            Text(title)
                .font(.system(size: 9.5, weight: .medium))
                .foregroundStyle(isComplete ? .primary : .secondary)
        }
        .frame(width: 112)
    }

    private func stageConnector(isComplete: Bool) -> some View {
        Rectangle()
            .fill(isComplete ? Color.green.opacity(0.5) : Color.secondary.opacity(0.18))
            .frame(height: 1)
            .frame(maxWidth: .infinity)
            .offset(y: -9)
    }

    private func performPrimaryAction() {
        guard !isPerformingAction else { return }
        switch snapshot.phase {
        case .chromeUnavailable:
            NSWorkspace.shared.open(URL(string: "https://www.google.com/chrome/")!)
        case .connected:
            isPerformingAction = true
            Task {
                let result = await Task.detached {
                    MacChromeMessagingService.shared.checkConnection()
                }.value
                checkMessage = result.message
                isPerformingAction = false
                refresh()
            }
        case .waitingForChrome, .installationRequired, .serviceStopped, .failed:
            isPerformingAction = true
            Task {
                let outcome = await Task.detached { () -> Result<String, Error> in
                    do {
                        return .success(try MacChromeMessagingService.shared.beginExtensionInstallation())
                    } catch {
                        return .failure(error)
                    }
                }.value
                switch outcome {
                case .success(let path):
                    checkMessage = String(
                        format: macExtensionsText(
                            "mac.extensions.chrome.setup_opened",
                            "Chrome Extensions is open. Turn on Developer mode, choose Load unpacked, and select %@. The path is already copied."
                        ),
                        path
                    )
                case .failure(let error):
                    errorMessage = error.localizedDescription
                }
                isPerformingAction = false
                refresh()
            }
        }
    }

    private func instruction(number: Int, text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 9) {
            Text(String(number))
                .font(.system(size: 9.5, weight: .bold, design: .rounded))
                .foregroundStyle(Color.accentColor)
                .frame(width: 20, height: 20)
                .background(Color.accentColor.opacity(0.10), in: Circle())
            Text(text)
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
        }
    }

    private func refresh() {
        snapshot = MacChromeMessagingService.shared.snapshot()
    }
}

private struct MacExtensionRow: View {
    let item: MacExtensionItem

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: item.kind.symbol)
                .foregroundStyle(item.source == .connectedHost ? Color.orange : Color.accentColor)
                .frame(width: 17)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title)
                    .font(.system(size: 11.5, weight: .medium))
                    .lineLimit(1)
                Text(item.detail.isEmpty ? item.source.title : item.detail)
                    .font(.system(size: 9.5))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: 5)
            Circle()
                .fill(item.isEnabled ? Color.green : Color.secondary.opacity(0.35))
                .frame(width: 6, height: 6)
        }
        .padding(.vertical, 2)
    }
}

private struct MacExtensionDetail: View {
    let item: MacExtensionItem
    let navigation: MacExtensionsNavigationState
    @Binding var externalErrorMessage: String?
    let onClose: () -> Void
    let onEnabledChange: (Bool) -> Void
    let onRemove: () -> Void

    @State private var document: MacExtensionDocument?
    @State private var draft = ""
    @State private var savedContent = ""
    @State private var isLoading = false
    @State private var isSaving = false
    @State private var errorMessage: String?
    @State private var hasBuiltInOverride = false
    @StateObject private var exitState = MacExtensionEditorExitState()
    @State private var showsSavedConfirmation = false
    @State private var feedbackID = UUID()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var hasUnsavedChanges: Bool {
        document?.isEditable == true && draft != savedContent
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Button(action: requestClose) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 12, weight: .semibold))
                }
                .macToolbarIcon()
                .disabled(isSaving)
                .help(macExtensionsText("mac.extensions.action.back", "Back"))
                .accessibilityLabel(macExtensionsText("mac.extensions.action.back", "Back"))

                Image(systemName: detailSymbol)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(detailTint)
                    .frame(width: 38, height: 38)
                    .background(detailTint.opacity(0.10), in: RoundedRectangle(cornerRadius: 10))
                VStack(alignment: .leading, spacing: 3) {
                    Text(item.title)
                        .font(.system(size: 17, weight: .semibold))
                    Text("\(item.kind.title) · \(item.source.title)")
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if let document {
                    MacStatusBadge(
                        title: document.isEditable
                            ? macExtensionsText("mac.extensions.detail.editable", "Editable")
                            : macExtensionsText("mac.extensions.detail.read_only", "Read Only"),
                        tint: document.isEditable ? detailTint : Color.secondary,
                        symbol: document.isEditable ? "pencil.line" : "lock.fill"
                    )
                }
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 15)

            Divider()

            if let visibleErrorMessage {
                MacInlineNotice(
                    title: macExtensionsText("mac.extensions.error.title", "Extensions Error"),
                    message: visibleErrorMessage,
                    kind: .failure,
                    dismiss: {
                        errorMessage = nil
                        externalErrorMessage = nil
                    }
                )
                .padding(.horizontal, 22)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            HStack(spacing: 14) {
                Text(item.detail.isEmpty ? macExtensionsText("mac.extensions.detail.no_description", "No description provided.") : item.detail)
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .textSelection(.enabled)
                Spacer()
                if item.source == .builtIn || (item.source != .connectedHost && (item.kind == .skill || item.kind == .mcp)) {
                    Toggle(
                        isOn: Binding(
                            get: { item.isEnabled },
                            set: onEnabledChange
                        )
                    ) {
                        Text(macExtensionsText("mac.extensions.status.enabled", "Enabled"))
                            .font(.system(size: 11.5, weight: .medium))
                    }
                    .toggleStyle(.switch)
                }
                if item.source == .builtIn {
                    Menu {
                        Button(macExtensionsText("mac.extensions.action.remove", "Remove"), role: .destructive) {
                            requestExit(onRemove)
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .menuStyle(.borderlessButton)
                    .macToolbarIcon()
                    .disabled(isSaving)
                    .help(macExtensionsText("mac.extensions.action.remove", "Remove"))
                    .accessibilityLabel(macExtensionsText("mac.extensions.action.remove", "Remove"))
                }
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 11)

            Divider()

            contentPanel
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(MacPageBackground())
        .background(MacEditorWindowCloseGuard(
            shouldPreventClose: { hasUnsavedChanges || isSaving },
            onCloseAttempt: {},
            onCloseAttemptWithWindow: { window in
                requestExit { window.performClose(nil) }
            }
        ).frame(width: 0, height: 0))
        .onAppear { navigation.leaveHandler = requestExit }
        .onDisappear {
            navigation.leaveHandler = nil
            exitState.cancel()
        }
        .task {
            if !(item.source == .connectedHost && item.mcpSummary != nil) {
                await loadDocument()
            }
        }
        .confirmationDialog(
            macExtensionsText("mac.extensions.detail.discard_title", "Discard unsaved changes?"),
            isPresented: Binding(
                get: { exitState.needsConfirmation },
                set: { if !$0 { exitState.cancel() } }
            ),
            titleVisibility: .visible
        ) {
            Button(macExtensionsText("mac.extensions.detail.discard", "Discard Changes"), role: .destructive) {
                exitState.discard(isSaving: isSaving) { draft = savedContent }
            }
            Button(macExtensionsText("mac.extensions.action.cancel", "Cancel"), role: .cancel) {
                exitState.cancel()
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: visibleErrorMessage)
    }

    private var visibleErrorMessage: String? {
        errorMessage ?? externalErrorMessage
    }

    private var skillCategory: MacSkillFunctionalCategory? {
        MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id)?.functionalCategory
    }

    private var detailSymbol: String {
        skillCategory?.symbol ?? item.kind.symbol
    }

    private var detailTint: Color {
        if item.source == .connectedHost { return .orange }
        switch skillCategory {
        case .documents: return .blue
        case .dataAnalysis: return .green
        case .designPresentations: return .pink
        case .communication: return .cyan
        case .planningAutomation: return .orange
        case .knowledgeResearch: return .indigo
        case .workspace: return .teal
        case .custom, .none: return MacDesignColor.brand
        }
    }

    @ViewBuilder
    private var contentPanel: some View {
        if item.source == .connectedHost, let summary = item.mcpSummary {
            mcpCatalogPanel(summary)
        } else if isLoading {
            MacLoadingState(macExtensionsText("mac.extensions.detail.loading", "Loading…"))
        } else if let document {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Text(macExtensionsText("mac.extensions.detail.content", "Content"))
                        .font(.system(size: 11.5, weight: .semibold))
                    Text(document.format.rawValue)
                        .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 3)
                        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 5))
                    Group {
                        if hasUnsavedChanges {
                            Label(macExtensionsText("mac.extensions.detail.unsaved", "Unsaved Changes"), systemImage: "circle.fill")
                                .foregroundStyle(.orange)
                        } else if showsSavedConfirmation {
                            Label(macExtensionsText("mac.extensions.detail.saved", "Saved"), systemImage: "checkmark.circle.fill")
                                .foregroundStyle(.green)
                        }
                    }
                    .font(.system(size: 9.5, weight: .medium))
                    .frame(width: 116, alignment: .leading)
                    Spacer()
                    Button(macExtensionsText("mac.extensions.action.copy_path", "Copy Path")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(document.url.path, forType: .string)
                    }
                    .buttonStyle(.borderless)
                    Button {
                        NSWorkspace.shared.open(document.url)
                    } label: {
                        Label(macExtensionsText("mac.extensions.detail.open_externally", "Open Externally"), systemImage: "arrow.up.forward.app")
                    }
                    .buttonStyle(.borderless)
                    if document.isEditable {
                        if item.source == .builtIn && hasBuiltInOverride {
                            Button(macExtensionsText("mac.extensions.detail.restore_builtin", "Restore Built-in")) {
                                requestExit { Task { await restoreBuiltInDocument() } }
                            }
                            .buttonStyle(.borderless)
                            .disabled(isSaving)
                        }
                        Button(macExtensionsText("mac.extensions.detail.revert", "Revert")) {
                            draft = savedContent
                        }
                        .disabled(!hasUnsavedChanges || isSaving)
                        Button {
                            Task { await saveDocument() }
                        } label: {
                            if isSaving {
                                ProgressView()
                                    .controlSize(.small)
                            } else {
                                Text(macExtensionsText("mac.extensions.detail.save", "Save"))
                            }
                        }
                        .keyboardShortcut("s", modifiers: .command)
                        .disabled(!hasUnsavedChanges || isSaving)
                    }
                }
                .padding(.horizontal, 16)
                .frame(height: 44)

                Divider()

                if document.isEditable {
                    TextEditor(text: $draft)
                        .font(.system(size: 12, design: .monospaced))
                        .scrollContentBackground(.hidden)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(Color(nsColor: .textBackgroundColor).opacity(0.42))
                } else {
                    ScrollView([.horizontal, .vertical]) {
                        Text(draft.isEmpty ? macExtensionsText("mac.extensions.detail.no_content", "No content available.") : draft)
                            .font(.system(size: 12, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .topLeading)
                            .padding(16)
                    }
                    .background(Color(nsColor: .textBackgroundColor).opacity(0.28))
                }

                HStack {
                    Text(document.url.path)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer()
                    Text("\(draft.count.formatted()) \(macExtensionsText("mac.extensions.detail.characters", "characters"))")
                }
                .font(.system(size: 9.5, design: .monospaced))
                .foregroundStyle(.tertiary)
                .padding(.horizontal, 16)
                .frame(height: 30)
            }
        } else {
            MacEmptyState(
                LocalizedStringKey(macExtensionsText("mac.extensions.detail.unavailable", "Content Unavailable")),
                description: LocalizedStringKey(
                    item.source == .connectedHost
                        ? macExtensionsText("mac.extensions.detail.host_unavailable", "Connected Host did not provide editable content.")
                        : macExtensionsText("mac.extensions.detail.item_unavailable", "No content is available for this item.")
                ),
                symbol: item.source == .connectedHost ? "network.slash" : "doc.questionmark"
            ) {}
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func mcpCatalogPanel(_ summary: MacMCPCapabilitySummary) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: summary.state == .connected ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(summary.state == .connected ? Color.green : Color.orange)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(macExtensionsText("mac.extensions.mcp.catalog", "Capability Catalog"))
                            .font(.system(size: 14, weight: .semibold))
                        Text(macMCPSummaryText(summary))
                            .font(.system(size: 11.5))
                            .foregroundStyle(.secondary)
                        if let protocolVersion = summary.protocolVersion, !protocolVersion.isEmpty {
                            Text("MCP \(protocolVersion)")
                                .font(.system(size: 10.5, design: .monospaced))
                                .foregroundStyle(.tertiary)
                        }
                    }
                }

                mcpNamesSection(String(localized: "Tools"), symbol: "hammer", names: summary.tools)
                mcpNamesSection(macExtensionsText("mac.extensions.mcp.resources", "Resources"), symbol: "doc.text", names: summary.resources)
                mcpNamesSection(macExtensionsText("mac.extensions.mcp.templates", "Templates"), symbol: "square.stack.3d.up", names: summary.resourceTemplates)
                mcpNamesSection(macExtensionsText("mac.extensions.mcp.prompts", "Prompts"), symbol: "text.bubble", names: summary.prompts)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(22)
        }
    }

    private func mcpNamesSection(_ title: String, symbol: String, names: [String]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("\(title) · \(names.count)", systemImage: symbol)
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(.secondary)
            if names.isEmpty {
                Text("—")
                    .foregroundStyle(.tertiary)
            } else {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(names.prefix(24), id: \.self) { name in
                        Text(name)
                            .font(.system(size: 11, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 7)
                            .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 7))
                    }
                }
            }
        }
    }

    private func loadDocument() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let loaded = try await MacExtensionsService.shared.document(for: item)
            document = loaded
            draft = loaded.content
            savedContent = loaded.content
            if let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) {
                hasBuiltInOverride = MacBuiltInSkillRegistry.hasOverride(for: skillID)
            }
        } catch MacExtensionsServiceError.contentUnavailable {
            document = nil
        } catch {
            document = nil
            errorMessage = error.localizedDescription
        }
    }

    private func saveDocument() async {
        guard hasUnsavedChanges, !isSaving else { return }
        isSaving = true
        defer { isSaving = false }
        do {
            let saved = try await MacExtensionsService.shared.save(draft, for: item)
            document = saved
            savedContent = saved.content
            if item.source == .builtIn {
                hasBuiltInOverride = true
            }
            showsSavedConfirmation = true
            feedbackID = UUID()
            let feedback = feedbackID
            Task { @MainActor in
                do { try await Task.sleep(for: .seconds(1.8)) } catch { return }
                guard feedbackID == feedback else { return }
                showsSavedConfirmation = false
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func requestClose() {
        requestExit(onClose)
    }

    private func requestExit(_ action: @escaping () -> Void) {
        exitState.request(hasChanges: hasUnsavedChanges, isSaving: isSaving, action: action)
    }

    private func restoreBuiltInDocument() async {
        guard let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) else {
            return
        }
        isSaving = true
        defer { isSaving = false }
        do {
            try MacBuiltInSkillRegistry.resetOverride(for: skillID)
            try MacBuiltInSkillRegistry.synchronizeCatalog()
            try MacBuiltInSkillRegistry.installTrustedRouter()
            hasBuiltInOverride = false
            await loadDocument()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
