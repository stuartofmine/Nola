// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionsView logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionsView
//   - ScheduledActionsHeroCard
//   - ScheduledActionsMetricCard
//   - ScheduledActionsEmptyCard
//   - ScheduledActionSimpleRow
//   - ScheduledActionInfoBadge
// - APIs:
//   - none
//
import SwiftUI

struct ScheduledActionsView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    private let embedded: Bool

    @State private var refreshNonce: Int = 0
    @State private var latestRunsByActionId: [String: LocalJSONStorage.ScheduledActionRunRecord] = [:]
    @State private var latestSummaryByActionId: [String: String] = [:]
    @State private var latestRunsError: String? = nil
    @State private var loadRunsTask: Task<Void, Never>? = nil
    @State private var searchText = ""
    @State private var selectedFilter: ScheduledActionFilter = .all

    private var enabledCount: Int {
        scheduledActions.filter { $0.source.enabled }.count
    }

    init(embedded: Bool = false) {
        self.embedded = embedded
    }

    var body: some View {
        Group {
            if embedded {
                pageContent
            } else {
                NavigationStack {
                    pageContent
                }
            }
        }
        .onAppear {
            refreshNonce += 1
            scheduleLoadLatestRuns()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaCustomSourcesUpdated)) { _ in
            refreshNonce += 1
            scheduleLoadLatestRuns()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaScheduledActionRunsUpdated)) { _ in
            refreshNonce += 1
            scheduleLoadLatestRuns()
        }
        .onChange(of: scenePhase) { _, newPhase in
            guard newPhase == .active else { return }
            refreshNonce += 1
            scheduleLoadLatestRuns()
        }
        .onDisappear {
            loadRunsTask?.cancel()
            loadRunsTask = nil
        }
    }

    private var pageContent: some View {
        NolaPage {
                if !embedded {
                    NolaPageHeader(
                        title: NSLocalizedString("scheduled.actions.redesigned.title", value: "Automation & Scheduling", comment: "Scheduled actions redesigned page title")
                    )
                }

                if !scheduledActions.isEmpty {
                    let nextEntry = scheduledActions.first {
                        $0.source.enabled && $0.nextRun != nil
                    }
                    ScheduledActionsHeroCard(
                        enabledCount: enabledCount,
                        totalCount: scheduledActions.count,
                        nextActionName: nextEntry?.source.name,
                        nextRun: nextEntry?.nextRun,
                        onCreateInChat: openChatForAutomation
                    )
                }

                if let latestRunsError {
                    NolaCard {
                        HStack(alignment: .top, spacing: 10) {
                            Image(systemName: "exclamationmark.arrow.triangle.2.circlepath")
                                .foregroundStyle(DesignTokens.warningColor)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(NSLocalizedString("scheduled.runs.load_failed", value: "Run history is temporarily unavailable", comment: "Scheduled run history load failure"))
                                    .font(.callout.weight(.semibold))
                                Text(latestRunsError)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                            Spacer(minLength: 8)
                            Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry action")) {
                                scheduleLoadLatestRuns()
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.regular)
                            .frame(minHeight: 44)
                        }
                    }
                }

                if !scheduledActions.isEmpty {
                    ScheduledActionsFilterBar(
                        searchText: $searchText,
                        selectedFilter: $selectedFilter,
                        totalCount: scheduledActions.count,
                        enabledCount: enabledCount,
                        pausedCount: scheduledActions.count - enabledCount,
                        attentionCount: attentionCount
                    )
                }

                if scheduledActions.isEmpty {
                    ScheduledActionsEmptyCard(onCreateInChat: openChatForAutomation)
                } else if visibleScheduledActions.isEmpty {
                    NolaCard {
                        NolaInlineEmptyState(
                            icon: "line.3.horizontal.decrease.circle",
                            title: NSLocalizedString("scheduled.filter.empty.title", value: "No matching automations", comment: "No filtered automations title"),
                            message: NSLocalizedString("scheduled.filter.empty.detail", value: "Try another name or status filter.", comment: "No filtered automations detail"),
                            tint: DesignTokens.primaryColor
                        )
                        Button(NSLocalizedString("scheduled.filter.reset", value: "Reset filters", comment: "Reset automation filters")) {
                            searchText = ""
                            selectedFilter = .all
                        }
                        .buttonStyle(.bordered)
                        .frame(maxWidth: .infinity)
                    }
                } else {
                    ForEach(visibleScheduledActions) { entry in
                        ScheduledActionSimpleRow(
                            source: entry.source,
                            nextRun: entry.nextRun,
                            latestRun: entry.latestRun,
                            latestSummary: entry.latestSummary,
                            onToggleEnabled: {
                                container.urlFetcher?.toggleSource(id: entry.source.id)
                                refreshNonce += 1
                            }
                        )
                    }
                }

        }
        .navigationTitle(NSLocalizedString("scheduled.actions.redesigned.title", value: "Automation & Scheduling", comment: "Scheduled actions redesigned page title"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(embedded ? .visible : .hidden, for: .navigationBar)
        .refreshable {
            refreshNonce += 1
            await loadLatestRuns()
        }
    }

    private func openChatForAutomation() {
        NotificationCenter.default.post(name: .nolaOpenChatTab, object: nil)
    }

    // MARK: - Derived Data

    private struct ScheduledActionEntry: Identifiable {
        let source: URLDataSource
        let nextRun: Date?
        let latestRun: LocalJSONStorage.ScheduledActionRunRecord?
        let latestSummary: String?
        var id: UUID { source.id }
    }

    private var attentionCount: Int {
        scheduledActions.filter { entry in
            guard let latestRun = entry.latestRun else { return false }
            return ScheduledActionRunText.statusCategory(latestRun.status) == .issues
        }.count
    }

    private var visibleScheduledActions: [ScheduledActionEntry] {
        let normalizedQuery = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return scheduledActions.filter { entry in
            let matchesQuery = normalizedQuery.isEmpty
                || entry.source.name.localizedCaseInsensitiveContains(normalizedQuery)
                || entry.source.url.localizedCaseInsensitiveContains(normalizedQuery)
                || entry.source.sourceDescription?.localizedCaseInsensitiveContains(normalizedQuery) == true
                || entry.source.userPrompt?.localizedCaseInsensitiveContains(normalizedQuery) == true
            let matchesFilter: Bool
            switch selectedFilter {
            case .all:
                matchesFilter = true
            case .enabled:
                matchesFilter = entry.source.enabled
            case .paused:
                matchesFilter = !entry.source.enabled
            case .attention:
                matchesFilter = entry.latestRun.map { ScheduledActionRunText.statusCategory($0.status) == .issues } ?? false
            }
            return matchesQuery && matchesFilter
        }
    }

    private var scheduledActions: [ScheduledActionEntry] {
        _ = refreshNonce
        guard let fetcher = container.urlFetcher else { return [] }
        let actions = fetcher.sources
            .filter { $0.isScheduledAutomation }
        let entries = actions.map { src in
            ScheduledActionEntry(
                source: src,
                nextRun: nextExecutionDate(for: src),
                latestRun: latestRunsByActionId[src.id.uuidString],
                latestSummary: latestSummaryByActionId[src.id.uuidString]
            )
        }
        return entries.sorted { lhs, rhs in
            if lhs.source.enabled != rhs.source.enabled {
                return lhs.source.enabled
            }
            switch (lhs.nextRun, rhs.nextRun) {
            case let (left?, right?) where left != right:
                return left < right
            case (_?, nil):
                return true
            case (nil, _?):
                return false
            default:
                break
            }
            return lhs.source.name.localizedCaseInsensitiveCompare(rhs.source.name) == .orderedAscending
        }
    }

    private func nextExecutionDate(for source: URLDataSource, now: Date = Date()) -> Date? {
        guard source.enabled, let frequency = source.scheduleFrequency else { return nil }
        if frequency == "once" {
            guard let target = source.scheduledDate, target > now else { return nil }
            return target
        }
        guard frequency == "daily" || frequency == "weekly",
              let rawTime = source.scheduledTimeISO,
              let (hour, minute) = clockTime(from: rawTime) else {
            return nil
        }

        var components = DateComponents()
        components.hour = hour
        components.minute = minute
        if frequency == "weekly", let weekday = source.scheduledWeekday, (1...7).contains(weekday) {
            components.weekday = weekday
        }
        return Calendar.current.nextDate(
            after: now,
            matching: components,
            matchingPolicy: .nextTime,
            repeatedTimePolicy: .first,
            direction: .forward
        )
    }

    private func clockTime(from raw: String) -> (Int, Int)? {
        let values = raw.split(whereSeparator: { !$0.isNumber }).compactMap { Int($0) }
        guard values.count >= 2 else { return nil }
        for index in stride(from: values.count - 2, through: 0, by: -1) {
            let hour = values[index]
            let minute = values[index + 1]
            if (0...23).contains(hour), (0...59).contains(minute) {
                return (hour, minute)
            }
        }
        return nil
    }

    @MainActor
    private func loadLatestRuns() async {
        guard let storage = container.localJSONStorage else {
            latestRunsByActionId = [:]
            latestSummaryByActionId = [:]
            latestRunsError = nil
            return
        }
        do {
            let allRuns = try storage.getScheduledActionRuns(limit: 200)

            var latestByActionId: [String: LocalJSONStorage.ScheduledActionRunRecord] = [:]
            for run in allRuns {
                let key = run.actionId
                if let current = latestByActionId[key] {
                    if run.finishedAt > current.finishedAt {
                        latestByActionId[key] = run
                    }
                } else {
                    latestByActionId[key] = run
                }
            }
            latestRunsByActionId = latestByActionId

            var snippets: [String: String] = [:]
            for (aid, run) in latestByActionId {
                guard let jid = run.resultJournalId, !jid.isEmpty else { continue }
                if let journal = try storage.getJournal(id: jid) {
                    let raw = journal.content
                        .replacingOccurrences(of: "\n\n\n", with: "\n\n")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if !raw.isEmpty {
                        snippets[aid] = String(raw.prefix(240))
                    }
                }
            }
            latestSummaryByActionId = snippets
            latestRunsError = nil
        } catch {
            latestRunsError = error.localizedDescription
        }
    }

    @MainActor
    private func scheduleLoadLatestRuns() {
        loadRunsTask?.cancel()
        loadRunsTask = Task {
            await loadLatestRuns()
        }
    }

}


// MARK: - Scheduled Actions Cards

private struct ScheduledActionsHeroCard: View {
    let enabledCount: Int
    let totalCount: Int
    let nextActionName: String?
    let nextRun: Date?
    let onCreateInChat: () -> Void

    var body: some View {
        NolaCard {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 12) {
                    heroIcon
                    heroCopy
                    Spacer(minLength: 4)
                    enabledBadge
                }
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .center) {
                        heroIcon
                        Spacer(minLength: 8)
                        enabledBadge
                    }
                    heroCopy
                }
            }

            HStack {
                Spacer()
                Button(action: onCreateInChat) {
                    Label(
                        NSLocalizedString("scheduled.timeline.create", value: "Create in chat", comment: "Create automation in chat"),
                        systemImage: "plus"
                    )
                    .font(.callout.weight(.semibold))
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
                .frame(minHeight: 44)
            }
        }
    }

    private var heroIcon: some View {
        Image(systemName: "clock.arrow.circlepath")
            .font(.system(size: 18, weight: .semibold))
            .foregroundStyle(DesignTokens.primaryColor)
            .frame(width: 40, height: 40)
            .background(
                DesignTokens.primaryColor.opacity(0.11),
                in: RoundedRectangle(cornerRadius: 13, style: .continuous)
            )
    }

    private var heroCopy: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(NSLocalizedString("scheduled.timeline.next", value: "Next automation", comment: "Next automation label"))
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .tracking(0.45)
            Text(nextActionName ?? NSLocalizedString("scheduled.timeline.none", value: "Nothing scheduled yet", comment: "No next automation"))
                .font(.title3.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            if let nextRun {
                Text(nextRun.formatted(date: .abbreviated, time: .shortened))
                    .font(.callout)
                    .foregroundStyle(DesignTokens.primaryColor)
            } else {
                Text(NSLocalizedString("scheduled.timeline.create.detail", value: "Describe a routine in chat and Nola will schedule it.", comment: "Create automation explanation"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private var enabledBadge: some View {
        if totalCount > 0 {
            Text("\(enabledCount)/\(totalCount)")
                .font(.caption.weight(.bold))
                .foregroundStyle(DesignTokens.primaryColor)
                .frame(minWidth: 28, minHeight: 28)
                .background(DesignTokens.primaryColor.opacity(0.1), in: Capsule())
                .accessibilityLabel(
                    String(
                        format: NSLocalizedString("scheduled.timeline.enabled.total", value: "%d of %d enabled", comment: "Enabled automation total count"),
                        enabledCount,
                        totalCount
                    )
                )
        }
    }
}

private enum ScheduledActionFilter: String, CaseIterable, Identifiable {
    case all
    case enabled
    case paused
    case attention

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all:
            return NSLocalizedString("scheduled.filter.all", value: "All", comment: "All automations filter")
        case .enabled:
            return NSLocalizedString("scheduled.filter.enabled", value: "Enabled", comment: "Enabled automations filter")
        case .paused:
            return NSLocalizedString("scheduled.filter.paused", value: "Paused", comment: "Paused automations filter")
        case .attention:
            return NSLocalizedString("scheduled.filter.attention", value: "Needs attention", comment: "Automation attention filter")
        }
    }
}

private struct ScheduledActionsFilterBar: View {
    @Binding var searchText: String
    @Binding var selectedFilter: ScheduledActionFilter
    let totalCount: Int
    let enabledCount: Int
    let pausedCount: Int
    let attentionCount: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 9) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField(
                    NSLocalizedString("scheduled.filter.search", value: "Search automations", comment: "Search automations placeholder"),
                    text: $searchText
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                if !searchText.isEmpty {
                    Button {
                        searchText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.tertiary)
                            .frame(width: 32, height: 44)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(NSLocalizedString("common.clear", value: "Clear", comment: "Clear search"))
                }
            }
            .padding(.horizontal, 13)
            .frame(minHeight: 44)
            .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 13, style: .continuous))

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(ScheduledActionFilter.allCases) { filter in
                        let selected = selectedFilter == filter
                        Button {
                            selectedFilter = filter
                        } label: {
                            Text("\(filter.title) \(count(for: filter))")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(selected ? Color.primary : Color.secondary)
                                .padding(.horizontal, 11)
                                .padding(.vertical, 7)
                                .frame(minHeight: 40)
                                .background(
                                    selected ? DesignTokens.accentSubtleColor : Color.clear,
                                    in: Capsule()
                                )
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(selected ? .isSelected : [])
                    }
                }
            }
        }
    }

    private func count(for filter: ScheduledActionFilter) -> Int {
        switch filter {
        case .all: return totalCount
        case .enabled: return enabledCount
        case .paused: return pausedCount
        case .attention: return attentionCount
        }
    }
}

private struct ScheduledActionsEmptyCard: View {
    let onCreateInChat: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(NSLocalizedString("scheduled.actions.ai_only.empty.title", value: "No automations yet", comment: "AI-only scheduled actions empty title"))
                .font(.headline)
            Text(NSLocalizedString("scheduled.actions.ai_only.empty.body", value: "Create one in AI chat with a natural-language sentence, for example: \"Summarize this RSS feed every Monday at 8:00.\" Reminders go to local tasks instead of the automation list.", comment: "AI-only scheduled actions empty body"))
                .font(.subheadline).foregroundStyle(.secondary)
            Button(action: onCreateInChat) {
                Label(NSLocalizedString("scheduled.timeline.create", value: "Create in chat", comment: "Create automation in chat"), image: "nola-chat")
                    .font(.callout.weight(.semibold))
            }
            .buttonStyle(.plain)
            .frame(minHeight: 44)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 12)
    }
}

private struct ScheduledActionSimpleRow: View {
    @EnvironmentObject var container: ServiceContainer

    let source: URLDataSource
    let nextRun: Date?
    let latestRun: LocalJSONStorage.ScheduledActionRunRecord?
    let latestSummary: String?
    let onToggleEnabled: () -> Void

    private var scheduleText: String {
        guard let freq = source.scheduleFrequency, let time = source.scheduledTimeISO else {
            return L10n.scheduledActionsScheduleNotSet
        }
        let freqLabel: String = {
            switch freq {
            case "once":
                return NSLocalizedString("scheduled.actions.frequency.once", value: "Once", comment: "One-off scheduled action frequency")
            case "daily":
                return NSLocalizedString("common.frequency.daily", value: "Daily", comment: "Daily frequency")
            case "weekly":
                return NSLocalizedString("common.frequency.weekly", value: "Weekly", comment: "Weekly frequency")
            default:
                return freq
            }
        }()

        if freq == "weekly", let wd = source.scheduledWeekday, (1...7).contains(wd) {
            let symbols = DateFormatter().weekdaySymbols ?? []
            let dayName = (symbols.count == 7) ? symbols[wd - 1] : "\(wd)"
            return "\(freqLabel) · \(dayName) · \(time)"
        }

        if freq == "once", let targetDate = source.scheduledDate {
            return "\(freqLabel) · \(targetDate.formatted(date: .abbreviated, time: .shortened))"
        }

        return "\(freqLabel) · \(time)"
    }

    var body: some View {
        NolaCard {
            HStack(alignment: .top, spacing: DesignTokens.spacing_md) {
                Image(systemName: "clock.fill")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(source.enabled ? DesignTokens.primaryColor : Color.secondary)
                    .frame(width: 36, height: 36)
                    .background(
                        (source.enabled ? DesignTokens.primaryColor : Color.secondary).opacity(0.1),
                        in: RoundedRectangle(cornerRadius: 11, style: .continuous)
                    )

                VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                    VStack(alignment: .leading, spacing: 10) {
                        ViewThatFits(in: .horizontal) {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                actionTitle
                                actionStatusBadge
                            }
                            VStack(alignment: .leading, spacing: 6) {
                                actionTitle
                                actionStatusBadge
                            }
                        }

                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 7) {
                                metadataBadge(scheduleText, systemImage: "repeat", tint: .secondary)
                                if let nextRun {
                                    metadataBadge(
                                        "\(NSLocalizedString("scheduled.timeline.next", value: "Next automation", comment: "Next automation label")) · \(nextRun.formatted(date: .abbreviated, time: .shortened))",
                                        systemImage: "calendar.badge.clock",
                                        tint: source.enabled ? DesignTokens.primaryColor : .secondary
                                    )
                                }
                            }
                            VStack(alignment: .leading, spacing: 7) {
                                metadataBadge(scheduleText, systemImage: "repeat", tint: .secondary)
                                if let nextRun {
                                    metadataBadge(
                                        "\(NSLocalizedString("scheduled.timeline.next", value: "Next automation", comment: "Next automation label")) · \(nextRun.formatted(date: .abbreviated, time: .shortened))",
                                        systemImage: "calendar.badge.clock",
                                        tint: source.enabled ? DesignTokens.primaryColor : .secondary
                                    )
                                }
                            }
                        }

                        if let latestRun {
                            VStack(alignment: .leading, spacing: 5) {
                                HStack(spacing: 6) {
                                    Circle()
                                        .fill(statusColor(for: latestRun))
                                        .frame(width: 6, height: 6)
                                    Text(String(format: L10n.scheduledActionsOverviewLastRun, latestRun.finishedAt.formatted(date: .abbreviated, time: .shortened)))
                                        .font(.caption.weight(.medium))
                                        .foregroundColor(.secondary)
                                }
                                if let s = latestSummary?.nolaSanitizedSummary(), !s.isEmpty {
                                    Text(s)
                                        .font(.callout)
                                        .foregroundColor(.secondary)
                                        .lineLimit(2)
                                }
                            }
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.secondary.opacity(0.055), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                Toggle("", isOn: Binding(get: { source.enabled }, set: { _ in onToggleEnabled() }))
                    .labelsHidden()
                    .fixedSize()
                    .accessibilityLabel(
                        Text(
                            String(
                                format: NSLocalizedString("scheduled.actions.toggle.accessibility", value: "Enable %@", comment: "Automation toggle accessibility label"),
                                source.name
                            )
                        )
                    )
                    .accessibilityValue(
                        Text(
                            source.enabled
                                ? NSLocalizedString("scheduled.filter.enabled", value: "Enabled", comment: "Enabled automation status")
                                : NSLocalizedString("scheduled.filter.paused", value: "Paused", comment: "Paused automation status")
                        )
                    )
                    .accessibilityIdentifier("scheduled.action.toggle.\(source.id.uuidString)")
            }

            HStack {
                Spacer()
                NavigationLink {
                    ScheduledActionRunsView(
                        actionId: source.id.uuidString,
                        titleOverride: source.name
                    )
                    .environmentObject(container)
                } label: {
                    Label(
                        NSLocalizedString("scheduled.timeline.history", value: "Run history", comment: "Automation run history link"),
                        systemImage: "clock.arrow.circlepath"
                    )
                    .font(.caption.weight(.semibold))
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
                .frame(minHeight: 44)
                .accessibilityIdentifier("scheduled.action.history.\(source.id.uuidString)")
            }
        }
        .contentShape(Rectangle())
        .accessibilityIdentifier("scheduled.action.card.\(source.id.uuidString)")
    }

    private var actionTitle: some View {
        Text(source.name)
            .font(.body.weight(.semibold))
            .foregroundColor(.primary)
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private var actionStatusBadge: some View {
        if !source.enabled {
            ScheduledActionInfoBadge(
                label: NSLocalizedString("scheduled.actions.badge.paused", value: "Paused", comment: "Scheduled action paused badge"),
                tint: .secondary
            )
        } else if let latestRun, latestRun.status != .success {
            ScheduledActionInfoBadge(
                label: statusText(for: latestRun),
                tint: statusColor(for: latestRun)
            )
        }
    }

    private func metadataBadge(_ text: String, systemImage: String, tint: Color) -> some View {
        Label(text, systemImage: systemImage)
            .font(.caption.weight(.medium))
            .foregroundStyle(tint)
            .lineLimit(1)
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .background(tint.opacity(0.075), in: Capsule())
    }

    private func statusColor(for run: LocalJSONStorage.ScheduledActionRunRecord) -> Color {
        switch run.status {
        case .success: return DesignTokens.successColor
        case .partial: return DesignTokens.warningColor
        case .skipped: return .secondary
        case .failed: return DesignTokens.dangerColor
        case .actionCompleted: return DesignTokens.successColor
        case .answered, .delegationAccepted, .cancelled: return .secondary
        case .incomplete, .blocked: return DesignTokens.warningColor
        }
    }

    private func statusText(for run: LocalJSONStorage.ScheduledActionRunRecord) -> String {
        switch run.status {
        case .success: return L10n.scheduledActionsLatestRunSuccess
        case .partial: return L10n.scheduledActionsLatestRunPartial
        case .skipped: return L10n.scheduledActionsLatestRunSkipped
        case .failed: return L10n.scheduledActionsLatestRunFailed
        case .answered: return String(localized: "Answered")
        case .delegationAccepted: return String(localized: "Delegated")
        case .actionCompleted: return String(localized: "Action completed")
        case .incomplete: return String(localized: "Incomplete")
        case .blocked: return String(localized: "Blocked")
        case .cancelled: return String(localized: "Cancelled")
        }
    }
}


private struct ScheduledActionInfoBadge: View {
    let label: String
    let tint: Color

    var body: some View {
        Text(label)
            .font(.caption.weight(.semibold))
            .lineLimit(1)
            .foregroundColor(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(
                Capsule()
                    .fill(tint.opacity(DesignTokens.overlaySoftOpacity))
            )
    }
}
