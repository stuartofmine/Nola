// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Creates and manages scheduled local CLI agent runs.

import AppKit
import Combine
import SwiftUI

final class MacAutomationCenterNavigationState: ObservableObject {
    @Published var showsRunHistory = false
    @Published var searchText = ""
    @Published var selectedFilter: MacAutomationFilter = .all

    func showRunHistory() { showsRunHistory = true }
    func showAutomations() { showsRunHistory = false }
}

struct MacAutomationCenterView: View {
    @State private var store: MacAutomationStore
    @State private var activityStore: MacActivityStore
    @State private var editingAutomation: MacAutomation?
    @State private var pendingDeletion: MacAutomation?
    @State private var presentsEditor = false
    private var searchText: String {
        get { navigation.searchText }
        nonmutating set { navigation.searchText = newValue }
    }
    private var selectedFilter: MacAutomationFilter { navigation.selectedFilter }
    @StateObject private var navigation: MacAutomationCenterNavigationState
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigationAction: (() -> Void)?

    init(
        navigationAction: (() -> Void)? = nil,
        navigation: MacAutomationCenterNavigationState = MacAutomationCenterNavigationState(),
        store: MacAutomationStore? = nil,
        activityStore: MacActivityStore? = nil
    ) {
        self.navigationAction = navigationAction
        _store = State(initialValue: store ?? .shared)
        _activityStore = State(initialValue: activityStore ?? .shared)
        _navigation = StateObject(wrappedValue: navigation)
    }

    var body: some View {
        VStack(spacing: 0) {
            if navigation.showsRunHistory {
                MacActivityCenterView(
                    navigationAction: navigation.showAutomations,
                    automationOnly: true
                )
            } else {
                VStack(spacing: 0) {
                    MacPageHeader(
                        LocalizedStringKey(String(localized: "Automations", table: "AutomationCenter")),
                        symbol: "clock.arrow.circlepath",
                        navigationAction: navigationAction
                    ) {
                        Text(String(localized: "Recurring work, explicit workspaces, and visible run history", table: "AutomationCenter"))
                    } actions: {
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 8) {
                                runHistoryButton(compact: false)
                                if !store.automations.isEmpty {
                                    newAutomationButton(compact: false)
                                }
                            }

                            HStack(spacing: 6) {
                                runHistoryButton(compact: true)
                                if !store.automations.isEmpty {
                                    newAutomationButton(compact: true)
                                }
                            }
                        }
                    }

                    if let errorMessage = store.errorMessage {
                        MacInlineNotice(
                            title: String(localized: "Automation Error", table: "AutomationCenter"),
                            message: errorMessage,
                            kind: .failure,
                            dismiss: { store.errorMessage = nil }
                        )
                        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                        .padding(.vertical, MacDesignTokens.Spacing.sm)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    if store.automations.isEmpty {
                        automationEmptyState
                    } else {
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 16) {
                                MacAutomationFilterBar(
                                    searchText: $navigation.searchText,
                                    selectedFilter: $navigation.selectedFilter,
                                    totalCount: store.automations.count,
                                    enabledCount: store.automations.filter(\.isEnabled).count,
                                    runningCount: store.runningAutomationIDs.count,
                                    visibleCount: visibleAutomations.count
                                )

                                if visibleAutomations.isEmpty {
                                    MacEmptyState(
                                        LocalizedStringKey(String(localized: "No Matching Automations", table: "AutomationCenter")),
                                        description: LocalizedStringKey(String(localized: "Try another name, prompt, workspace, or status filter.", table: "AutomationCenter")),
                                        symbol: "line.3.horizontal.decrease.circle"
                                    ) {
                                        Button(String(localized: "Reset Filters", table: "AutomationCenter")) {
                                            searchText = ""
                                            navigation.selectedFilter = .all
                                        }
                                        .buttonStyle(.bordered)
                                    }
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 28)
                                } else {
                                    ForEach(visibleAutomations) { automation in
                                        automationRow(automation)
                                    }
                                }
                            }
                            .padding(.horizontal, MacDesignTokens.Spacing.xxl)
                            .padding(.vertical, MacDesignTokens.Spacing.xl)
                            .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                            .frame(maxWidth: .infinity)
                        }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(MacPageBackground())
        .sheet(isPresented: $presentsEditor) {
            MacAutomationEditor(automation: editingAutomation, saveError: store.errorMessage) { automation in
                if store.upsert(automation) {
                    presentsEditor = false
                }
            } cancel: {
                presentsEditor = false
            }
        }
        .confirmationDialog(
            String(localized: "Delete Automation?", table: "AutomationCenter"),
            isPresented: Binding(
                get: { pendingDeletion != nil },
                set: { if !$0 { pendingDeletion = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(String(localized: "Delete", table: "AutomationCenter"), role: .destructive) {
                if let pendingDeletion {
                    store.delete(pendingDeletion)
                }
                pendingDeletion = nil
            }
            Button(String(localized: "Cancel", table: "AutomationCenter"), role: .cancel) { pendingDeletion = nil }
        } message: {
            Text(String(localized: "This removes the schedule and its future runs. Existing run history is kept.", table: "AutomationCenter"))
        }
        .task {
            await MacPlatformDataService.shared.refresh()
            await MacWebSourceStore.shared.refreshDueSources()
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: store.errorMessage)
    }

    private var visibleAutomations: [MacAutomation] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return store.automations.filter { automation in
            let matchesQuery = query.isEmpty
                || automation.name.localizedCaseInsensitiveContains(query)
                || automation.prompt.localizedCaseInsensitiveContains(query)
                || automation.workspacePath.localizedCaseInsensitiveContains(query)
            let matchesFilter: Bool
            switch selectedFilter {
            case .all:
                matchesFilter = true
            case .enabled:
                matchesFilter = automation.isEnabled
            case .paused:
                matchesFilter = !automation.isEnabled
            case .running:
                matchesFilter = store.runningAutomationIDs.contains(automation.id)
            }
            return matchesQuery && matchesFilter
        }
    }

    private var automationEmptyState: some View {
        MacEmptyState(
            LocalizedStringKey(String(localized: "No Automations", table: "AutomationCenter")),
            description: LocalizedStringKey(String(localized: "Schedule a prompt to run daily or weekly in a selected workspace.", table: "AutomationCenter")),
            symbol: "clock.badge.checkmark"
        ) {
            Button {
                editingAutomation = nil
                store.errorMessage = nil
                presentsEditor = true
            } label: {
                Label(String(localized: "New Automation", table: "AutomationCenter"), systemImage: "plus")
            }
            .buttonStyle(.borderedProminent)
        }
    }

    private func runHistoryButton(compact: Bool) -> some View {
        Button(action: navigation.showRunHistory) {
            if compact {
                Image(systemName: "clock.arrow.circlepath")
                    .accessibilityLabel(String(localized: "Run History", table: "AutomationCenter"))
            } else {
                Label(String(localized: "Run History", table: "AutomationCenter"), systemImage: "clock.arrow.circlepath")
            }
        }
        .help(String(localized: "Run History", table: "AutomationCenter"))
    }

    private func newAutomationButton(compact: Bool) -> some View {
        Button {
            editingAutomation = nil
            store.errorMessage = nil
            presentsEditor = true
        } label: {
            if compact {
                Image(systemName: "plus")
                    .accessibilityLabel(String(localized: "New Automation", table: "AutomationCenter"))
            } else {
                Label(String(localized: "New Automation", table: "AutomationCenter"), systemImage: "plus")
            }
        }
        .buttonStyle(.borderedProminent)
        .help(String(localized: "New Automation", table: "AutomationCenter"))
    }

    private func automationRow(_ automation: MacAutomation) -> some View {
        let latestRun = activityStore.entries.first {
            $0.kind == .automation && $0.automationIdentity?.definitionID == automation.id
        }
        let isRunning = store.runningAutomationIDs.contains(automation.id)
        let runningDetail = store.runningAutomationDetails[automation.id]
        return VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                MacIconTile(
                    symbol: automation.frequency == .hourly
                        ? "clock.arrow.circlepath"
                        : (automation.frequency == .daily ? "sun.max" : "calendar.badge.clock"),
                    tint: automation.isEnabled ? MacDesignColor.brand : Color.secondary
                )

                VStack(alignment: .leading, spacing: 4) {
                    Text(automation.name)
                        .font(.system(size: 14, weight: .semibold))
                        .lineLimit(2)
                    Label(automation.scheduleDescription, systemImage: "calendar.badge.clock")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(automation.isEnabled ? MacDesignColor.brand : Color.secondary)
                }

                Spacer(minLength: 12)

                Toggle(isOn: Binding(
                    get: { automation.isEnabled },
                    set: { store.setEnabled($0, for: automation) }
                )) {
                    Text(
                        automation.isEnabled
                            ? String(localized: "Enabled", table: "AutomationCenter")
                            : String(localized: "Paused", table: "AutomationCenter")
                    )
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(automation.isEnabled ? Color.green : Color.secondary)
                }
                .toggleStyle(.switch)
                .controlSize(.small)
                .fixedSize()
                .accessibilityLabel(String(localized: "Enabled", table: "AutomationCenter"))
            }

            Text(automation.prompt)
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
                .lineSpacing(2)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)

            automationMetadata(automation)

            if isRunning {
                MacAutomationLiveRunStatus(detail: runningDetail)
                HStack {
                    Spacer()
                    automationManagementActions(automation, isRunning: true)
                }
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 12) {
                        automationRunState(
                            automation,
                            latestStatusTitle: latestRun?.status.title,
                            latestRunCompleted: latestRun?.status.isSuccessful == true,
                            latestFinishedAt: latestRun?.finishedAt,
                            isRunning: false,
                            runningDetail: nil
                        )
                        Spacer(minLength: 8)
                        automationManagementActions(automation, isRunning: false)
                        runNowButton(automation, isRunning: false)
                    }
                    VStack(alignment: .leading, spacing: 9) {
                        automationRunState(
                            automation,
                            latestStatusTitle: latestRun?.status.title,
                            latestRunCompleted: latestRun?.status.isSuccessful == true,
                            latestFinishedAt: latestRun?.finishedAt,
                            isRunning: false,
                            runningDetail: nil
                        )
                        HStack {
                            Spacer()
                            automationManagementActions(automation, isRunning: false)
                            runNowButton(automation, isRunning: false)
                        }
                    }
                }
            }
        }
        .padding(16)
        .macPanel()
        .accessibilityIdentifier("mac.automation.card.\(automation.id.uuidString)")
    }

    private func metadataLabel(_ title: String, symbol: String) -> some View {
        Label(title, systemImage: symbol)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(Color.primary.opacity(0.035), in: Capsule())
    }

    private func automationMetadata(_ automation: MacAutomation) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) {
                if let skillID = automation.skillID {
                    metadataLabel(skillID.localizedTitle, symbol: "sparkles")
                }
                if let dependencies = automation.dataDependencyDescription {
                    metadataLabel(dependencies, symbol: "square.stack.3d.up")
                }
                if automation.effectiveRunWhenDataChanges {
                    metadataLabel(String(localized: "On data change", table: "AutomationCenter"), symbol: "bolt.fill")
                }
                metadataLabel(automation.workspacePath, symbol: "folder")
                    .truncationMode(.middle)
                    .help(automation.workspacePath)
            }

            FlowLayout(spacing: 6) {
                if let skillID = automation.skillID {
                    metadataLabel(skillID.localizedTitle, symbol: "sparkles")
                }
                if let dependencies = automation.dataDependencyDescription {
                    metadataLabel(dependencies, symbol: "square.stack.3d.up")
                }
                if automation.effectiveRunWhenDataChanges {
                    metadataLabel(String(localized: "On data change", table: "AutomationCenter"), symbol: "bolt.fill")
                }
                metadataLabel(automation.workspacePath, symbol: "folder")
                    .truncationMode(.middle)
                    .help(automation.workspacePath)
            }
        }
        .font(.system(size: 10.5, weight: .medium))
        .foregroundStyle(.secondary)
    }

    private func automationManagementActions(_ automation: MacAutomation, isRunning: Bool) -> some View {
        HStack(spacing: 4) {
            Button {
                editingAutomation = automation
                store.errorMessage = nil
                presentsEditor = true
            } label: {
                Image(systemName: "square.and.pencil")
                    .font(.system(size: 12, weight: .medium))
            }
            .macToolbarIcon()
            .disabled(isRunning)
            .help(String(localized: "Edit", table: "AutomationCenter"))
            .accessibilityLabel(String(localized: "Edit", table: "AutomationCenter"))

            Menu {
                Button(String(localized: "View Run History", table: "AutomationCenter"), action: navigation.showRunHistory)
                if let logURL = store.revealLog(automation) {
                    Button(String(localized: "Reveal Log in Finder", table: "AutomationCenter")) {
                        NSWorkspace.shared.activateFileViewerSelecting([logURL])
                    }
                }
                Divider()
                Button(String(localized: "Delete", table: "AutomationCenter"), role: .destructive) {
                    pendingDeletion = automation
                }
                .disabled(isRunning)
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .macToolbarIcon()
            .accessibilityLabel(String(localized: "More", table: "AutomationCenter"))
            .help(String(localized: "View Run History", table: "AutomationCenter") + " · " + String(localized: "Delete", table: "AutomationCenter"))
        }
    }

    @ViewBuilder
    private func automationRunState(
        _ automation: MacAutomation,
        latestStatusTitle: String?,
        latestRunCompleted: Bool,
        latestFinishedAt: Date?,
        isRunning: Bool,
        runningDetail: String?
    ) -> some View {
        HStack(spacing: 9) {
            runStatus(
                latestStatusTitle: latestStatusTitle,
                latestRunCompleted: latestRunCompleted,
                latestFinishedAt: latestFinishedAt,
                isRunning: isRunning,
                runningDetail: runningDetail
            )
            nextRun(automation, isRunning: isRunning)
        }
    }

    @ViewBuilder
    private func runStatus(
        latestStatusTitle: String?,
        latestRunCompleted: Bool,
        latestFinishedAt: Date?,
        isRunning: Bool,
        runningDetail: String?
    ) -> some View {
        if isRunning {
            MacAutomationLiveRunStatus(detail: runningDetail)
        } else if let latestStatusTitle {
            Label(
                latestStatusTitle,
                systemImage: latestRunCompleted
                    ? "checkmark.circle.fill"
                    : "exclamationmark.triangle.fill"
            )
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(latestRunCompleted ? Color.green : Color.orange)
            if let latestFinishedAt {
                Text(latestFinishedAt, style: .relative)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func nextRun(_ automation: MacAutomation, isRunning: Bool) -> some View {
        if let nextRun = automation.nextRun, automation.isEnabled, !isRunning {
            Label {
                Text(nextRun, style: .relative)
            } icon: {
                Image(systemName: "clock")
            }
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(.secondary)
        }
    }

    private func runNowButton(_ automation: MacAutomation, isRunning: Bool) -> some View {
        Button { store.runNow(automation) } label: {
            Label(String(localized: "Run Now", table: "AutomationCenter"), systemImage: "play.fill")
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.regular)
        .disabled(isRunning)
    }
}

struct MacAutomationLiveRunStatus: View {
    let detail: String?
    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            MacBreathingStatusLight(diameter: 22, dotScale: 0.85)

            VStack(alignment: .leading, spacing: 2) {
                Text(String(localized: "Running", table: "AutomationCenter"))
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(MacDesignColor.brand)
                if let detail, !detail.isEmpty {
                    Text(detail)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                        .transaction { transaction in
                            transaction.animation = nil
                        }
                }
            }
            Spacer(minLength: 8)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(MacDesignColor.surfaceRaised, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(MacDesignColor.brand.opacity(0.12), lineWidth: 0.5)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct FlowLayout: Layout {
    let spacing: CGFloat

    func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) -> CGSize {
        arrange(proposal: proposal, subviews: subviews).size
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        let result = arrange(proposal: ProposedViewSize(width: bounds.width, height: proposal.height), subviews: subviews)
        for (index, point) in result.points.enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y), proposal: .unspecified)
        }
    }

    private func arrange(proposal: ProposedViewSize, subviews: Subviews) -> (size: CGSize, points: [CGPoint]) {
        let maximumWidth = proposal.width ?? .infinity
        var points: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var contentWidth: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > maximumWidth {
                contentWidth = max(contentWidth, x - spacing)
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            points.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        contentWidth = max(contentWidth, max(0, x - spacing))
        return (CGSize(width: min(maximumWidth, contentWidth), height: y + rowHeight), points)
    }
}

enum MacAutomationEditorValidation {
    static func message(
        name: String,
        prompt: String,
        workspacePath: String,
        validatesPersonalDataAge: Bool,
        maximumDataAge: String,
        timeoutMinutes: String,
        retryCount: String
    ) -> String? {
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Enter an automation name to continue.", table: "AutomationCenter")
        }
        if prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Enter an agent prompt to continue.", table: "AutomationCenter")
        }
        if workspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Choose a workspace to continue.", table: "AutomationCenter")
        }
        if validatesPersonalDataAge {
            guard let maximumAge = Int(maximumDataAge), (5...10_080).contains(maximumAge) else {
                return String(localized: "Maximum data age must be between 5 and 10,080 minutes.", table: "AutomationCenter")
            }
        }
        guard let timeout = Int(timeoutMinutes), (1...720).contains(timeout) else {
            return String(localized: "Timeout must be between 1 and 720 minutes.", table: "AutomationCenter")
        }
        guard let retries = Int(retryCount), (0...3).contains(retries) else {
            return String(localized: "Retries must be between 0 and 3.", table: "AutomationCenter")
        }
        return nil
    }
}

private struct MacAutomationEditor: View {
    let existing: MacAutomation?
    let saveError: String?
    let save: (MacAutomation) -> Void
    let cancel: () -> Void

    @State private var name: String
    @State private var draftID: UUID
    @State private var prompt: String
    @State private var frequency: MacAutomation.Frequency
    @State private var weekday: Int
    @State private var time: Date
    @State private var workspacePath: String
    @State private var webSourceStore = MacWebSourceStore.shared
    @State private var contextSources: Set<MacAutomation.ContextSource>
    @State private var selectedWebSourceIDs: Set<UUID>
    @State private var maximumDataAge: String
    @State private var timeoutMinutes: String
    @State private var retryCount: String
    @State private var runWhenDataChanges: Bool
    @State private var selectedSkillID: MacBuiltInSkillID?
    @State private var activeSkills: [MacBuiltInSkillDescriptor] = []
    @State private var skillLoadError: String?
    @State private var initialEditorValues: [String]?
    @State private var showsDiscardConfirmation = false
    @FocusState private var nameFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        automation: MacAutomation?,
        saveError: String? = nil,
        save: @escaping (MacAutomation) -> Void,
        cancel: @escaping () -> Void
    ) {
        existing = automation
        self.saveError = saveError
        self.save = save
        self.cancel = cancel
        _draftID = State(initialValue: automation?.id ?? UUID())
        let calendar = Calendar.current
        let components = DateComponents(hour: automation?.hour ?? 9, minute: automation?.minute ?? 0)
        _name = State(initialValue: automation?.name ?? "")
        _prompt = State(initialValue: automation?.prompt ?? "")
        _frequency = State(initialValue: automation?.frequency ?? .daily)
        _weekday = State(initialValue: automation?.weekday ?? 1)
        _time = State(initialValue: calendar.date(from: components) ?? Date())
        _workspacePath = State(
            initialValue: automation?.workspacePath
                ?? MacSecurityScopedBookmarkStore.shared.restoreWorkspace()?.path
                ?? MacAppEnvironment.shared.scratchWorkspaceURL.path
        )
        _contextSources = State(initialValue: automation?.contextSources ?? [.personalData, .webSources])
        _selectedWebSourceIDs = State(initialValue: automation?.webSourceIDs
            ?? Set(MacWebSourceStore.shared.sources.filter(\.isEnabled).map(\.id)))
        _maximumDataAge = State(initialValue: String(automation?.effectiveMaximumDataAgeMinutes ?? 60))
        _timeoutMinutes = State(initialValue: String(automation?.effectiveTimeoutMinutes ?? 30))
        _retryCount = State(initialValue: String(automation?.effectiveRetryCount ?? 0))
        _runWhenDataChanges = State(initialValue: automation?.effectiveRunWhenDataChanges ?? false)
        _selectedSkillID = State(initialValue: automation?.skillID)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                MacIconTile(symbol: existing == nil ? "clock.badge.plus" : "clock.arrow.circlepath", tint: MacDesignColor.brand)
                VStack(alignment: .leading, spacing: 3) {
                    Text(existing == nil
                        ? String(localized: "New Automation", table: "AutomationCenter")
                        : String(localized: "Edit Automation", table: "AutomationCenter"))
                        .font(.system(size: 19, weight: .semibold))
                    Text(String(localized: "Define what runs, when it runs, and which data it may use.", table: "AutomationCenter"))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
            }

            Form {
                TextField(String(localized: "Name", table: "AutomationCenter"), text: $name)
                    .focused($nameFocused)
                TextField(String(localized: "Agent prompt", table: "AutomationCenter"), text: $prompt, axis: .vertical)
                    .lineLimit(4...10)
                Picker(String(localized: "Skill", table: "AutomationCenter"), selection: $selectedSkillID) {
                    Text(String(localized: "Automatic", table: "AutomationCenter")).tag(Optional<MacBuiltInSkillID>.none)
                    ForEach(activeSkills) { descriptor in
                        Text(descriptor.title).tag(Optional(descriptor.id))
                    }
                }
                if let skillLoadError {
                    Label(skillLoadError, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
                Picker(String(localized: "Frequency", table: "AutomationCenter"), selection: $frequency) {
                    ForEach(MacAutomation.Frequency.allCases) { frequency in
                        Text(frequency.title).tag(frequency)
                    }
                }
                if frequency == .weekly {
                    Picker(String(localized: "Weekday", table: "AutomationCenter"), selection: $weekday) {
                        ForEach(Array(MacAutomation.weekdayNames.enumerated()), id: \.offset) { index, title in
                            Text(title).tag(index)
                        }
                    }
                }
                DatePicker(String(localized: "Time", table: "AutomationCenter"), selection: $time, displayedComponents: .hourAndMinute)
                Toggle(String(localized: "Run when selected data changes", table: "AutomationCenter"), isOn: $runWhenDataChanges)
                HStack {
                    TextField(String(localized: "Workspace", table: "AutomationCenter"), text: $workspacePath)
                    Button {
                        Task {
                            if let url = await MacSecurityScopedBookmarkStore.shared.chooseWorkspace() {
                                workspacePath = url.path
                            }
                        }
                    } label: {
                        Label(String(localized: "Choose…", table: "AutomationCenter"), systemImage: "folder")
                    }
                }

                Section(String(localized: "Data", table: "AutomationCenter")) {
                    ForEach(MacAutomation.ContextSource.allCases) { source in
                        Toggle(isOn: Binding(
                            get: { contextSources.contains(source) },
                            set: { enabled in
                                if enabled {
                                    contextSources.insert(source)
                                } else {
                                    contextSources.remove(source)
                                }
                            }
                        )) {
                            Label(source.title, systemImage: source.symbol)
                        }
                    }

                    if contextSources.contains(.personalData) {
                        LabeledContent(String(localized: "Maximum data age", table: "AutomationCenter")) {
                            HStack(spacing: 6) {
                                TextField("60", text: $maximumDataAge)
                                    .textFieldStyle(.roundedBorder)
                                    .frame(width: 72)
                                Text(String(localized: "minutes", table: "AutomationCenter"))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }

                    if contextSources.contains(.webSources) {
                        if webSourceStore.sources.filter(\.isEnabled).isEmpty {
                            Text(String(localized: "No web sources", table: "AutomationCenter"))
                                .foregroundStyle(.secondary)
                        } else {
                            ForEach(webSourceStore.sources.filter(\.isEnabled)) { source in
                                Toggle(source.name, isOn: Binding(
                                    get: { selectedWebSourceIDs.contains(source.id) },
                                    set: { enabled in
                                        if enabled {
                                            selectedWebSourceIDs.insert(source.id)
                                        } else {
                                            selectedWebSourceIDs.remove(source.id)
                                        }
                                    }
                                ))
                            }
                        }
                    }
                }

                Section(String(localized: "Reliability", table: "AutomationCenter")) {
                    LabeledContent(String(localized: "Timeout", table: "AutomationCenter")) {
                        HStack(spacing: 6) {
                            TextField("30", text: $timeoutMinutes)
                                .textFieldStyle(.roundedBorder)
                                .frame(width: 72)
                            Text(String(localized: "minutes", table: "AutomationCenter"))
                                .foregroundStyle(.secondary)
                        }
                    }
                    LabeledContent(String(localized: "Retries after failure", table: "AutomationCenter")) {
                        TextField("0", text: $retryCount)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 72)
                    }
                }
            }
            .formStyle(.grouped)

            if let saveError {
                MacInlineNotice(
                    title: String(localized: "Automation Error", table: "AutomationCenter"),
                    message: saveError,
                    kind: .failure
                )
                .accessibilityIdentifier("mac-automation-save-error")
            }
            HStack {
                if let validationMessage {
                    Label(validationMessage, systemImage: "exclamationmark.triangle.fill")
                        .font(.system(size: 10.5))
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 320, alignment: .leading)
                }
                Spacer()
                Button(String(localized: "Cancel", table: "AutomationCenter")) {
                    if hasUnsavedChanges { showsDiscardConfirmation = true }
                    else { cancel() }
                }
                    .keyboardShortcut(.cancelAction)
                Button(String(localized: "Save", table: "AutomationCenter")) {
                    let parts = Calendar.current.dateComponents([.hour, .minute], from: time)
                    save(MacAutomation(
                        id: draftID,
                        name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                        prompt: prompt.trimmingCharacters(in: .whitespacesAndNewlines),
                        frequency: frequency,
                        weekday: weekday,
                        hour: parts.hour ?? 9,
                        minute: parts.minute ?? 0,
                        workspacePath: workspacePath,
                        isEnabled: existing?.isEnabled ?? true,
                        createdAt: existing?.createdAt ?? .now,
                        contextSources: contextSources,
                        webSourceIDs: selectedWebSourceIDs,
                        maximumDataAgeMinutes: Int(maximumDataAge) ?? 60,
                        timeoutMinutes: Int(timeoutMinutes) ?? 30,
                        retryCount: Int(retryCount) ?? 0,
                        runWhenDataChanges: runWhenDataChanges,
                        skillID: selectedSkillID
                    ))
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .disabled(!canSave)
            }
        }
        .padding(22)
        .frame(
            minWidth: 520,
            idealWidth: 620,
            maxWidth: 720,
            minHeight: 560,
            idealHeight: 720,
            maxHeight: 820
        )
        .task {
            if initialEditorValues == nil { initialEditorValues = editorValues }
            do {
                activeSkills = try MacBuiltInSkillRegistry.activeDescriptors()
                skillLoadError = nil
            } catch {
                skillLoadError = error.localizedDescription
            }
            guard existing == nil else { return }
            await Task.yield()
            nameFocused = true
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: validationMessage)
        .interactiveDismissDisabled(hasUnsavedChanges)
        .confirmationDialog(String(localized: "Discard unsaved changes?"), isPresented: $showsDiscardConfirmation, titleVisibility: .visible) {
            Button(String(localized: "Discard Changes"), role: .destructive, action: cancel)
            Button(String(localized: "Continue Editing"), role: .cancel) {}
        }
    }

    private var editorValues: [String] {
        [name, prompt, String(describing: frequency), String(weekday),
         String(Calendar.current.component(.hour, from: time)),
         String(Calendar.current.component(.minute, from: time)), workspacePath,
         contextSources.map { String(describing: $0) }.sorted().joined(separator: ","),
         selectedWebSourceIDs.map(\.uuidString).sorted().joined(separator: ","),
         maximumDataAge, timeoutMinutes, retryCount, String(runWhenDataChanges),
         selectedSkillID.map { String(describing: $0) } ?? ""]
    }

    private var hasUnsavedChanges: Bool {
        initialEditorValues.map { $0 != editorValues } ?? false
    }

    private var canSave: Bool {
        validationMessage == nil
    }

    private var validationMessage: String? {
        MacAutomationEditorValidation.message(
            name: name,
            prompt: prompt,
            workspacePath: workspacePath,
            validatesPersonalDataAge: contextSources.contains(.personalData),
            maximumDataAge: maximumDataAge,
            timeoutMinutes: timeoutMinutes,
            retryCount: retryCount
        )
    }
}

enum MacAutomationFilter: String, CaseIterable, Identifiable {
    case all
    case enabled
    case paused
    case running

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: return String(localized: "All", table: "AutomationCenter")
        case .enabled: return String(localized: "Enabled", table: "AutomationCenter")
        case .paused: return String(localized: "Paused", table: "AutomationCenter")
        case .running: return String(localized: "Running", table: "AutomationCenter")
        }
    }
}

private struct MacAutomationFilterBar: View {
    @Binding var searchText: String
    @Binding var selectedFilter: MacAutomationFilter
    let totalCount: Int
    let enabledCount: Int
    let runningCount: Int
    let visibleCount: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) {
                    searchField.frame(maxWidth: 360)
                    filterPicker.frame(maxWidth: 440)
                }
                VStack(alignment: .leading, spacing: 10) {
                    searchField
                    filterPicker
                }
            }
            HStack(spacing: 6) {
                Text(
                    String(
                        format: String(localized: "%d of %d automations", table: "AutomationCenter"),
                        visibleCount,
                        totalCount
                    )
                )
                if runningCount > 0 {
                    Text("·")
                    Label(
                        String(localized: "Running", table: "AutomationCenter"),
                        systemImage: "waveform.path"
                    )
                    .foregroundStyle(.orange)
                }
            }
            .font(.system(size: 9.5, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 2)
        }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.tertiary)
            TextField(String(localized: "Search automations", table: "AutomationCenter"), text: $searchText)
                .textFieldStyle(.plain)
            if !searchText.isEmpty {
                Button {
                    searchText = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                .accessibilityLabel(String(localized: "Clear", table: "AutomationCenter"))
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 34)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
    }

    private var filterPicker: some View {
        ViewThatFits(in: .horizontal) {
            Picker(String(localized: "Status", table: "AutomationCenter"), selection: $selectedFilter) {
                ForEach(MacAutomationFilter.allCases) { filter in
                    Text("\(filter.title) \(count(for: filter))").tag(filter)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)

            Picker(String(localized: "Status", table: "AutomationCenter"), selection: $selectedFilter) {
                ForEach(MacAutomationFilter.allCases) { filter in
                    Text("\(filter.title) · \(count(for: filter))").tag(filter)
                }
            }
            .pickerStyle(.menu)
        }
    }

    private func count(for filter: MacAutomationFilter) -> Int {
        switch filter {
        case .all: return totalCount
        case .enabled: return enabledCount
        case .paused: return totalCount - enabledCount
        case .running: return runningCount
        }
    }
}
