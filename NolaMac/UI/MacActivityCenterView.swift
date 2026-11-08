// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents a native searchable history of agent and automation runs.

import AppKit
import SwiftUI

struct MacActivityCenterView: View {
    let navigationAction: (() -> Void)?
    let automationOnly: Bool
    @Environment(\.openWindow) private var openWindow
    @State private var showsLocalSummaries = false
    init(navigationAction: (() -> Void)? = nil, automationOnly: Bool = false) {
        self.navigationAction = navigationAction; self.automationOnly = automationOnly
    }
    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader("Activity", symbol: "waveform.path.ecg", navigationAction: navigationAction) {
                EmptyView()
            } actions: {
                Button("activity.localSummaries") { showsLocalSummaries = true }
            }
            NavigationStack {
                AppleActivityView(source: automationOnly ? "automation" : "all", load: { query in
                    try await Task.detached { try MacApplicationPort.shared.listActivity(query) }.value
                }, detail: { entry in
                    try await Task.detached { try MacApplicationPort.shared.activityDetail(entry) }.value
                }, openSource: { source in openWindow(value: source) })
            }
        }
        .sheet(isPresented: $showsLocalSummaries) {
            MacLocalActivitySummariesPanel(automationOnly: automationOnly)
        }
    }
}

struct MacLocalActivitySummariesPanel: View {
    var automationOnly = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack {
            HStack { Spacer(); Button("common.close") { dismiss() } }.padding()
            MacLocalActivitySummariesView(automationOnly: automationOnly)
        }.frame(minWidth: 660, minHeight: 500)
    }
}

private struct MacLocalActivitySummariesView: View {
    private enum Filter: String, CaseIterable, Identifiable {
        case all
        case agent
        case automation
        case attention

        var id: String { rawValue }
        var title: String {
            switch self {
            case .all: return String(localized: "All")
            case .agent: return String(localized: "Agent")
            case .automation: return String(localized: "Automation")
            case .attention: return String(localized: "Attention")
            }
        }

        var emptyTitle: String {
            switch self {
            case .all: return String(localized: "No activity yet")
            case .agent: return String(localized: "No agent activity")
            case .automation: return String(localized: "No automation activity")
            case .attention: return String(localized: "Nothing needs attention")
            }
        }

        var emptyDescription: String {
            switch self {
            case .all: return String(localized: "Completed agent and automation runs will appear here.")
            case .agent: return String(localized: "Agent runs will appear after you complete a task.")
            case .automation: return String(localized: "Run an automation to create its first activity record.")
            case .attention: return String(localized: "Failed and cancelled runs will appear here for review.")
            }
        }
    }

    @State private var store = MacActivityStore.shared
    @State private var selection: UUID?
    @State private var filter = Filter.all
    @State private var query = ""
    @State private var confirmsClear = false
    @State private var isRefreshing = false
    @State private var compactDetailIsPresented = false
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigationAction: (() -> Void)?

    init(navigationAction: (() -> Void)? = nil, automationOnly: Bool = false) {
        self.navigationAction = navigationAction
        _filter = State(initialValue: automationOnly ? .automation : .all)
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                "activity.localSummaries",
                symbol: "waveform.path.ecg",
                navigationAction: navigationAction,
                navigationSymbol: "chevron.left",
                navigationLabel: "Automations"
            ) {
                Text("Agent and automation results with local run details")
            } actions: {
                Button {
                    refreshActivity()
                } label: {
                    if isRefreshing {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityLabel(String(localized: "Refreshing activity"))
                    } else {
                        Image(systemName: "arrow.clockwise")
                    }
                }
                .macToolbarIcon()
                .help(String(localized: "Refresh"))
                .accessibilityLabel(String(localized: "Refresh"))
                .disabled(isRefreshing)
                Menu {
                    Button("Clear History", role: .destructive) { confirmsClear = true }
                        .disabled(store.entries.isEmpty)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .macToolbarIcon()
                .help(String(localized: "Clear History"))
                .accessibilityLabel(String(localized: "Clear History"))
            }

            if let lastError = store.lastError {
                MacInlineNotice(
                    title: String(localized: "Activity Could Not Be Updated"),
                    message: lastError,
                    kind: .failure,
                    actionTitle: String(localized: "Refresh"),
                    action: refreshActivity,
                    dismiss: { store.lastError = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            if !store.entries.isEmpty {
                MacPageMetricsBar(metrics: activityMetrics, isWorking: isRefreshing)
                    .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                    .padding(.vertical, MacDesignTokens.Spacing.sm)
            }

            if store.entries.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(filter.emptyTitle),
                    description: LocalizedStringKey(filter.emptyDescription),
                    symbol: filter == .attention ? "checkmark.circle" : "waveform.path.ecg"
                ) {
                    Button("Automations") {
                        openWindow(id: "automation-center")
                    }
                    .buttonStyle(.borderedProminent)
                }
            } else {
                GeometryReader { proxy in
                    if proxy.size.width < 720 {
                        compactContent
                    } else {
                        regularContent
                    }
                }
            }
        }
        .background(MacPageBackground())
        .task {
            refreshActivity()
        }
        .onChange(of: filteredEntries.map(\.id)) { _, ids in
            if let selection, ids.contains(selection) { return }
            selection = ids.first
            compactDetailIsPresented = false
        }
        .confirmationDialog(
            "Clear Activity History?",
            isPresented: $confirmsClear,
            titleVisibility: .visible
        ) {
            Button("Cancel", role: .cancel) {}
            Button("Clear", role: .destructive) { store.clear() }
        } message: {
            Text("This removes the local history list. Conversation and automation logs are not deleted.")
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: store.lastError)
    }

    private var regularContent: some View {
        MacDesktopSplitView(
            sidebarIdealWidth: MacDesignTokens.Layout.sidebarWidth,
            sidebarMaxWidth: 320
        ) {
            activityList()
        } detail: {
            activityDetail(compact: false)
        }
    }

    @ViewBuilder
    private var compactContent: some View {
        if compactDetailIsPresented, let selectedEntry {
            VStack(spacing: 0) {
                HStack {
                    Button {
                        compactDetailIsPresented = false
                    } label: {
                        Label("Activity", systemImage: "chevron.left")
                    }
                    .buttonStyle(.borderless)
                    Spacer()
                }
                .padding(.horizontal, MacDesignTokens.Spacing.lg)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .background(MacDesignColor.chrome.opacity(0.72))

                MacActivityDetail(entry: selectedEntry, compact: true) {
                    openConversation(selectedEntry)
                }
            }
        } else {
            activityList(compact: true)
        }
    }

    private func activityList(compact: Bool = false) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                activityFilterControl
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)

            if filteredEntries.isEmpty {
                Group {
                    if !normalizedQuery.isEmpty {
                        ContentUnavailableView.search(text: query)
                    } else {
                        ContentUnavailableView(
                            filter.emptyTitle,
                            systemImage: filter == .attention
                                ? "checkmark.circle"
                                : "line.3.horizontal.decrease.circle",
                            description: Text(filter.emptyDescription)
                        )
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(filteredEntries) { entry in
                            Button {
                                selection = entry.id
                                if compact { compactDetailIsPresented = true }
                            } label: {
                                activityRow(entry)
                                    .contentShape(Rectangle())
                            }
                                .buttonStyle(.plain)
                                .macSelectableListRow(isSelected: selection == entry.id)
                                .accessibilityAddTraits(selection == entry.id ? .isSelected : [])
                        }
                    }
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                }
            }
        }
        .searchable(
            text: $query,
            placement: .automatic,
            prompt: Text("Search activity")
        )
        .background(MacDesignColor.chrome.opacity(0.54))
    }

    @ViewBuilder
    private func activityDetail(compact: Bool) -> some View {
        if let selectedEntry {
            MacActivityDetail(entry: selectedEntry, compact: compact) {
                openConversation(selectedEntry)
            }
        } else if !normalizedQuery.isEmpty {
            ContentUnavailableView.search(text: query)
        } else {
            MacEmptyState(
                LocalizedStringKey(filter.emptyTitle),
                description: LocalizedStringKey(filter.emptyDescription),
                symbol: filter == .attention
                    ? "checkmark.circle"
                    : "line.3.horizontal.decrease.circle"
            ) {}
        }
    }

    private func filterButton(_ option: Filter) -> some View {
        let isSelected = filter == option
        let count = filterCount(option)
        return Button {
            filter = option
        } label: {
            HStack(spacing: 5) {
                Text(option.title)
                Text(String(count))
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(isSelected ? .primary : .tertiary)
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .background(
                isSelected ? Color.accentColor.opacity(0.12) : Color.clear,
                in: Capsule()
            )
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .foregroundStyle(isSelected ? Color.accentColor : Color.secondary)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var activityFilterControl: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 5) {
                ForEach(Filter.allCases) { option in
                    filterButton(option)
                }
            }

            Picker(selection: $filter) {
                ForEach(Filter.allCases) { option in
                    Text(verbatim: "\(option.title) · \(filterCount(option))")
                        .tag(option)
                }
            } label: {
                Label {
                    Text(verbatim: filter.title)
                } icon: {
                    Image(systemName: "line.3.horizontal.decrease.circle")
                }
            }
            .pickerStyle(.menu)
        }
    }

    private func activityRow(_ entry: MacActivityEntry) -> some View {
        MacActivityRow(entry: entry)
            .tag(entry.id)
            .onTapGesture(count: 2) {
                if entry.conversationID != nil { openConversation(entry) }
            }
            .contextMenu {
                if entry.conversationID != nil {
                    Button(String(localized: "Open Conversation")) { openConversation(entry) }
                }
                if let logPath = entry.logPath {
                    Button(String(localized: "Reveal Log in Finder")) {
                        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: logPath)])
                    }
                }
                Divider()
                Button(String(localized: "Remove"), role: .destructive) { store.remove(entry) }
            }
    }

    private var filteredEntries: [MacActivityEntry] {
        let normalized = normalizedQuery
        return store.entries.filter { entry in
            let filterMatches: Bool
            switch filter {
            case .all: filterMatches = true
            case .agent: filterMatches = entry.kind != .automation
            case .automation: filterMatches = entry.kind == .automation
            case .attention: filterMatches = entry.status.needsAttention
            }
            guard filterMatches else { return false }
            return normalized.isEmpty
                || entry.title.localizedCaseInsensitiveContains(normalized)
                || entry.detail.localizedCaseInsensitiveContains(normalized)
                || entry.outputPreview.localizedCaseInsensitiveContains(normalized)
        }
    }

    private var normalizedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func filterCount(_ option: Filter) -> Int {
        store.entries.filter { entry in
            switch option {
            case .all: return true
            case .agent: return entry.kind != .automation
            case .automation: return entry.kind == .automation
            case .attention: return entry.status.needsAttention
            }
        }.count
    }

    private var selectedEntry: MacActivityEntry? {
        guard let selection else { return nil }
        return store.entries.first { $0.id == selection }
    }

    private func refreshActivity() {
        guard !isRefreshing else { return }
        Task { @MainActor in
            // NSTableView can start this view's task while its delegate is still
            // committing the initial rows. Move observable mutations beyond that
            // AppKit update cycle instead of publishing reentrantly.
            try? await Task.sleep(for: .milliseconds(40))
            guard !isRefreshing else { return }
            isRefreshing = true
            store.lastError = nil
            store.synchronizeAutomationLogs(MacAutomationStore.shared.automations)
            if selection == nil { selection = filteredEntries.first?.id }
            isRefreshing = false
        }
    }

    private var activityMetrics: [MacPageMetric] {
        [
            MacPageMetric(
                id: "activity-total",
                "Activity",
                value: "\(store.entries.count)",
                symbol: "waveform.path.ecg",
                tint: .blue
            ),
            MacPageMetric(
                id: "activity-completed",
                "Action completed",
                value: "\(store.entries.filter { $0.status == .actionCompleted }.count)",
                symbol: "checkmark.circle.fill",
                tint: .green
            ),
            MacPageMetric(
                id: "activity-attention",
                "Attention",
                value: "\(store.entries.filter { $0.status.needsAttention }.count)",
                symbol: "exclamationmark.triangle.fill",
                tint: .orange
            ),
        ]
    }

    private func openConversation(_ entry: MacActivityEntry) {
        guard let conversationID = entry.conversationID else { return }
        openWindow(value: conversationID)
    }
}

struct MacActivityRow: View {
    let entry: MacActivityEntry

    var body: some View {
        HStack(spacing: 11) {
            ZStack {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(statusColor.opacity(0.10))
                Image(systemName: entry.kind.symbol)
                    .foregroundStyle(statusColor)
            }
            .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 3) {
                Text(entry.title)
                    .font(.system(size: 12.5, weight: .semibold))
                    .lineLimit(1)
                if !entry.detail.isEmpty {
                    Text(entry.detail)
                        .font(.system(size: 9.5))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                HStack(spacing: 5) {
                    Text(entry.kind.title)
                    Text("·")
                    Label(entry.status.title, systemImage: statusSymbol)
                        .foregroundStyle(statusColor)
                }
                .font(.system(size: 9.5, weight: .medium))
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            if entry.conversationID != nil {
                Image(systemName: "arrow.up.forward")
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            Text(entry.finishedAt, style: .relative)
                .font(.system(size: 9.5))
                .foregroundStyle(.tertiary)
        }
        .lineLimit(1)
        .padding(.vertical, entry.detail.isEmpty ? 4 : 6)
        .accessibilityElement(children: .combine)
    }

    private var statusColor: Color {
        switch entry.status {
        case .answered: return .blue
        case .actionCompleted: return .green
        case .delegationAccepted: return .secondary
        case .incomplete, .blocked: return .orange
        case .executionFinished: return .secondary
        case .failed: return .red
        case .cancelled: return .orange
        }
    }

    private var statusSymbol: String {
        switch entry.status {
        case .answered: return "bubble.left.fill"
        case .actionCompleted: return "checkmark.circle.fill"
        case .delegationAccepted: return "arrow.triangle.branch"
        case .incomplete: return "circle.dotted"
        case .blocked: return "pause.circle.fill"
        case .executionFinished: return "archivebox"
        case .failed: return "xmark.circle.fill"
        case .cancelled: return "stop.circle.fill"
        }
    }
}

struct MacActivityDetail: View {
    let entry: MacActivityEntry
    var compact = false
    let openConversation: () -> Void
    @State private var copiedOutput = false
    @State private var manuallyCopyOutput = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                detailHeader

                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 176), spacing: 12)],
                    alignment: .leading,
                    spacing: 12
                ) {
                    summaryCard(
                        "Started",
                        value: entry.startedAt.formatted(date: .abbreviated, time: .standard),
                        symbol: "clock"
                    )
                    summaryCard("Duration", value: durationText, symbol: "timer")
                    summaryCard(
                        "Ended",
                        value: entry.finishedAt.formatted(date: .abbreviated, time: .standard),
                        symbol: "clock.badge.checkmark",
                        tint: statusColor
                    )
                }

                if !entry.detail.isEmpty {
                    detailSection("Details", symbol: resultSymbol) {
                        Text(entry.detail)
                            .font(.body)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }

                if !entry.outputPreview.isEmpty {
                    detailSection("Output", symbol: "terminal") {
                        VStack(alignment: .leading, spacing: 10) {
                            HStack {
                                Spacer()
                                Button {
                                    NSPasteboard.general.clearContents()
                                    guard NSPasteboard.general.setString(entry.outputPreview, forType: .string) else {
                                        manuallyCopyOutput = true
                                        return
                                    }
                                    withAnimation(.easeOut(duration: 0.16)) {
                                        copiedOutput = true
                                    }
                                    Task { @MainActor in
                                        try? await Task.sleep(for: .seconds(1.4))
                                        withAnimation(.easeOut(duration: 0.16)) {
                                            copiedOutput = false
                                        }
                                    }
                                } label: {
                                    Label(
                                        copiedOutput ? "Copied" : "Copy",
                                        systemImage: copiedOutput ? "checkmark" : "doc.on.doc"
                                    )
                                }
                                .buttonStyle(.borderless)
                                .controlSize(.small)
                                .foregroundStyle(copiedOutput ? Color.green : Color.accentColor)
                            }
                            ScrollView(.vertical) {
                                Text(entry.outputPreview)
                                    .font(.system(size: 11.5, design: .monospaced))
                                    .textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(14)
                            }
                            .frame(minHeight: 96, maxHeight: 360)
                            .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 11))
                            .overlay {
                                RoundedRectangle(cornerRadius: 11, style: .continuous)
                                    .strokeBorder(Color.primary.opacity(0.065), lineWidth: 0.5)
                            }
                        }
                    }
                }

                if let logPath = entry.logPath {
                    detailSection("Log", symbol: "doc.text.magnifyingglass") {
                        HStack(spacing: 12) {
                            Text(URL(fileURLWithPath: logPath).lastPathComponent)
                                .font(.callout.monospaced())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer()
                            Button {
                                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: logPath)])
                            } label: {
                                Label(String(localized: "Reveal Log in Finder"), systemImage: "folder")
                            }
                        }
                    }
                }
            }
            .padding(compact ? MacDesignTokens.Spacing.lg : MacDesignTokens.Spacing.xxl)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(MacPageBackground())
        .onChange(of: entry.id) { _, _ in
            copiedOutput = false
            manuallyCopyOutput = false
        }
        .sheet(isPresented: $manuallyCopyOutput) {
            VStack(alignment: .leading, spacing: 16) {
                Text("Copy").font(.headline)
                ScrollView { Text(entry.outputPreview).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                HStack { Spacer(); Button("Done") { manuallyCopyOutput = false }.keyboardShortcut(.defaultAction) }
            }.padding(24).frame(width: 560, height: 420)
        }
    }

    private var detailHeader: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 14) {
                detailIdentity
                Spacer(minLength: 20)
                conversationButton
            }
            VStack(alignment: .leading, spacing: 16) {
                detailIdentity
                conversationButton
            }
        }
    }

    private var detailIdentity: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: entry.kind.symbol)
                .font(.system(size: 22, weight: .medium))
                .foregroundStyle(statusColor)
                .frame(width: 48, height: 48)
                .background(statusColor.opacity(0.10), in: RoundedRectangle(cornerRadius: 13))
            VStack(alignment: .leading, spacing: 5) {
                Text(entry.title)
                    .font(.system(size: 19, weight: .semibold))
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Text(entry.kind.title)
                        .foregroundStyle(.secondary)
                    Label(entry.status.title, systemImage: statusSymbol)
                        .foregroundStyle(statusColor)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(statusColor.opacity(0.09), in: Capsule())
                }
                .font(.callout)
            }
        }
    }

    @ViewBuilder
    private var conversationButton: some View {
        if entry.conversationID != nil {
            Button(action: openConversation) {
                Label(String(localized: "Open Conversation"), systemImage: "arrow.up.forward")
            }
                .buttonStyle(.borderedProminent)
        }
    }

    private func summaryCard(
        _ title: LocalizedStringKey,
        value: String,
        symbol: String,
        tint: Color = .accentColor
    ) -> some View {
        HStack(spacing: 11) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(tint)
                .frame(width: 30, height: 30)
                .background(tint.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(value)
                    .font(.callout.weight(.medium))
                    .lineLimit(2)
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .macPanel(cornerRadius: MacDesignTokens.Radius.panel)
    }

    private func detailSection<Content: View>(
        _ title: LocalizedStringKey,
        symbol: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: symbol)
                .font(.headline)
            content()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }

    private var durationText: String {
        let seconds = max(0, Int(entry.duration))
        if seconds < 60 {
            return String(format: String(localized: "%lld sec"), Int64(seconds))
        }
        return String(
            format: String(localized: "%lld min %lld sec"),
            Int64(seconds / 60),
            Int64(seconds % 60)
        )
    }

    private var statusColor: Color {
        switch entry.status {
        case .answered: return .blue
        case .actionCompleted: return .green
        case .delegationAccepted: return .secondary
        case .incomplete, .blocked: return .orange
        case .executionFinished: return .secondary
        case .failed: return .red
        case .cancelled: return .orange
        }
    }

    private var statusSymbol: String {
        switch entry.status {
        case .answered: return "bubble.left.fill"
        case .actionCompleted: return "checkmark.circle.fill"
        case .delegationAccepted: return "arrow.triangle.branch"
        case .incomplete: return "circle.dotted"
        case .blocked: return "pause.circle.fill"
        case .executionFinished: return "archivebox"
        case .failed: return "xmark.circle.fill"
        case .cancelled: return "stop.circle.fill"
        }
    }

    private var resultSymbol: String {
        switch entry.status {
        case .answered: return "bubble.left"
        case .actionCompleted: return "checkmark.seal"
        case .delegationAccepted: return "arrow.triangle.branch"
        case .incomplete: return "circle.dotted"
        case .blocked: return "pause.circle"
        case .executionFinished: return "archivebox"
        case .failed: return "exclamationmark.triangle"
        case .cancelled: return "stop.circle"
        }
    }
}
