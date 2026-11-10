// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements HomeView logic in the iOS UI layer.
// - Interfaces:
//   - HomeView
//   - HomeRefreshStrip
//   - HomeQuickActionsRow
//   - HomeQuickActionButton
//   - DataSourcesScheduleHubView
//   - CategorizedTodayView
//   - SummaryItemCard
//   - HistoryDetailView
// - APIs:
//   - none
//
import SwiftUI
import UIKit

private struct HomeHealthDashboardCard: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ObservedObject var store: HealthDashboardStore
    let onTapManage: () -> Void
    let onTapAuthorize: () -> Void
    let onTapOpenSettings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
            Button(action: onTapManage) {
                HStack(spacing: DesignTokens.spacing_md) {
                    Image(systemName: DataSourceType.health.iconSystemName)
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(DesignTokens.primaryColor)
                        .frame(width: 36, height: 36)
                        .background(DesignTokens.primaryColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 11, style: .continuous))

                    VStack(alignment: .leading, spacing: 2) {
                        Text(L10n.settingsHealthDataSource)
                            .font(.headline)
                            .foregroundStyle(.primary)
                        Text(store.snapshot.map { snapshot in
                            "\(activityTitle(snapshot.activityDate)) · \(freshnessLabel(snapshot.generatedAt))"
                        } ?? tr("health.dashboard.local_source", "Private, on-device source"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if store.isLoading {
                        ProgressView().controlSize(.small)
                    } else {
                        HStack(spacing: 6) {
                            Circle()
                                .fill(store.snapshot == nil ? Color.secondary : Color.green)
                                .frame(width: 7, height: 7)
                            Text(store.snapshot == nil
                                ? tr("health.dashboard.waiting", "Waiting")
                                : tr("health.dashboard.connected", "Connected"))
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Image(systemName: "chevron.right")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if let snapshot = store.snapshot {
                if let error = store.errorMessage {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(DesignTokens.dangerColor)
                }

                primaryStepsMetric(snapshot)

                HStack(spacing: 8) {
                    compactMetric(
                        title: tr("health.dashboard.distance", "Distance"),
                        value: snapshot.todayDistanceKilometers > 0 ? decimal(snapshot.todayDistanceKilometers) : "—",
                        unit: tr("health.dashboard.unit.km", "km"),
                        icon: "location.fill",
                        color: .cyan
                    )
                    compactMetric(
                        title: tr("health.dashboard.active_energy", "Active energy"),
                        value: snapshot.todayActiveEnergyKilocalories > 0 ? integer(snapshot.todayActiveEnergyKilocalories) : "—",
                        unit: tr("health.dashboard.unit.kcal", "kcal"),
                        icon: "flame.fill",
                        color: .orange
                    )
                    compactMetric(
                        title: tr("health.dashboard.exercise", "Exercise"),
                        value: snapshot.todayExerciseMinutes > 0 ? integer(snapshot.todayExerciseMinutes) : "—",
                        unit: tr("health.dashboard.unit.min", "min"),
                        icon: "figure.run",
                        color: .green
                    )
                }

                HStack(spacing: 7) {
                    Image(systemName: "lock.fill")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                    Text("\(tr("health.dashboard.on_device", "On this device")) · \(freshnessLabel(snapshot.generatedAt))")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                    Spacer()
                    Button(L10n.settings, action: onTapOpenSettings)
                        .font(.caption.weight(.semibold))
                        .buttonStyle(.plain)
                        .foregroundStyle(DesignTokens.primaryColor)
                }
            } else if store.isLoading {
                HStack(spacing: DesignTokens.spacing_sm) {
                    ProgressView().controlSize(.small)
                    Text(tr("health.dashboard.reading", "Reading the latest Health data..."))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.vertical, DesignTokens.spacing_lg)
            } else {
                VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                    if let error = store.errorMessage {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .font(.caption)
                            .foregroundStyle(DesignTokens.dangerColor)
                    } else {
                        Text(tr("health.dashboard.no_readable_data", "No readable Health data is available yet."))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Button(action: onTapAuthorize) {
                            Label(tr("health.dashboard.authorize", "Authorize Health access"), systemImage: "heart.text.square")
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(DesignTokens.actionColor)
                        .foregroundStyle(DesignTokens.onActionColor)
                        Button(L10n.settings, action: onTapOpenSettings)
                            .buttonStyle(.bordered)
                    }
                }
            }
        }
        .padding(.vertical, DesignTokens.spacing_xs)
        .animation(reduceMotion ? nil : DesignTokens.revealAnimation, value: store.isLoading)
    }

    private func primaryStepsMetric(_ snapshot: HealthDashboardSnapshot) -> some View {
        HStack(spacing: DesignTokens.spacing_md) {
            VStack(alignment: .leading, spacing: 5) {
                Label(tr("health.dashboard.steps", "Steps"), systemImage: "figure.walk")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.blue)
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(snapshot.todaySteps > 0 ? integer(snapshot.todaySteps) : "—")
                        .font(.system(size: 34, weight: .semibold, design: .rounded))
                        .tracking(-0.8)
                        .contentTransition(reduceMotion ? .identity : .numericText())
                    Text(tr("health.dashboard.unit.steps", "steps"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(activityTitle(snapshot.activityDate))
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 8)
            Image(systemName: "figure.walk.motion")
                .font(.system(size: 28, weight: .medium))
                .foregroundStyle(.blue)
                .frame(width: 58, height: 58)
                .background(.blue.opacity(0.10), in: Circle())
                .overlay {
                    Circle().stroke(.blue.opacity(0.10), lineWidth: 7)
                        .padding(-6)
                }
        }
        .padding(16)
        .padding(.vertical, 4)
    }

    private func compactMetric(title: String, value: String, unit: String, icon: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            Image(systemName: icon)
                .font(.caption.weight(.semibold))
                .foregroundStyle(color)
                .frame(width: 26, height: 26)
                .background(color.opacity(0.10), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                Text(value)
                    .font(.system(.callout, design: .rounded, weight: .semibold))
                    .contentTransition(reduceMotion ? .identity : .numericText())
                if value != "—" {
                    Text(unit)
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 13, style: .continuous))
    }

    private func freshnessLabel(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    private func integer(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(0)))
    }

    private func decimal(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(1)))
    }

    private func activityTitle(_ date: Date) -> String {
        if Calendar.autoupdatingCurrent.isDateInToday(date) {
            return tr("health.dashboard.today", "Today")
        }
        return date.formatted(date: .abbreviated, time: .omitted)
    }

    private func tr(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "")
    }
}

struct HomeView: View {
    var ownsNavigationStack = true
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase

    @AppStorage("birthdayLeadDays") private var birthdayLeadDays: Int = 3
    @AppStorage("reminderLeadDays") private var reminderLeadDays: Int = 1

    @StateObject private var healthDashboard = HealthDashboardStore.shared

    @State private var scheduleItems: [HomeScheduleItem] = []
    @State private var isLoadingSchedule: Bool = false
    @State private var scheduleError: String? = nil

    @State private var showHealthDetail: Bool = false
    @State private var showScheduleDetail: Bool = false
    @State private var showScheduledActionsView: Bool = false
    @State private var showNotificationManager: Bool = false
    @State private var selectedReminderId: String? = nil
    @State private var selectedCalendarEventId: String? = nil
    @State private var unreadNotificationCount: Int = 0

    @State private var scheduledActionEntries: [HomeScheduledActionsOverviewCard.Entry] = []
    @State private var isLoadingScheduledActions: Bool = false
    @State private var lastReloadedAt: Date? = nil

    private var isReloading: Bool {
        healthDashboard.isLoading || isLoadingSchedule || isLoadingScheduledActions
    }

    private var isRunningUITests: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing")
    }

    var body: some View {
        Group {
            if ownsNavigationStack { NavigationStack { content } }
            else { content }
        }
    }

    private var content: some View {
        NolaPage {
            NolaPageHeader(
                title: NSLocalizedString("data.title", value: "数据", comment: "Data tab page title"),
                subtitle: NSLocalizedString("data.subtitle.local_first", value: "本机数据源、定时动作与最近执行状态。", comment: "Data page subtitle")
            ) {
                Button {
                    showNotificationManager = true
                } label: {
                    Image(systemName: unreadNotificationCount > 0 ? "bell.fill" : "bell")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundColor(DesignTokens.primaryColor)
                        .frame(width: 36, height: 36)
                        .background(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                .fill(DesignTokens.primaryColor.opacity(DesignTokens.overlayMediumOpacity))
                        )
                        .overlay(alignment: .topTrailing) {
                            if unreadNotificationCount > 0 {
                                Text(unreadNotificationCount > 99 ? "99+" : String(unreadNotificationCount))
                                    .font(.caption2.weight(.bold))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 4)
                                    .frame(minWidth: 16, minHeight: 16)
                                    .background(Color.red, in: Capsule())
                                    .overlay(Capsule().stroke(Color.white, lineWidth: 1.5))
                                    .offset(x: 7, y: -6)
                            }
                        }
                }
                .accessibilityLabel(L10n.notificationsTitle)
                .accessibilityValue(String(unreadNotificationCount))
            }

            HomeRefreshStrip(
                isLoading: isReloading,
                lastReloadedAt: lastReloadedAt,
                onRefresh: { Task { await reloadAll() } }
            )
            VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                NavigationLink {
                    DataSourcesView()
                        .environmentObject(container)
                } label: {
                    HStack(spacing: DesignTokens.spacing_md) {
                        Image(systemName: "square.stack.3d.up")
                            .foregroundStyle(DesignTokens.primaryColor)
                            .frame(width: 22)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(NSLocalizedString("data_sources.runtime.hero.title", value: "Live context", comment: "Data source runtime hero title"))
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.primary)
                            Text(NSLocalizedString("data_sources.runtime.hero.subtitle", value: "A source is ready only after Nola has read it successfully.", comment: "Data source runtime hero subtitle"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.leading)
                                .lineLimit(2)
                        }
                        Spacer(minLength: DesignTokens.spacing_sm)
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                    .padding(.vertical, 5)
                }
                .buttonStyle(NolaPressableStyle())
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("data.center.open")

                HomeHealthDashboardCard(
                    store: healthDashboard,
                    onTapManage: { showHealthDetail = true },
                    onTapAuthorize: {
                        Task {
                            _ = await container.requestHealthAuthorization()
                            await reloadHealth()
                        }
                    },
                    onTapOpenSettings: { container.openAppSettings() }
                )

                HomeScheduledActionsOverviewCard(
                    entries: scheduledActionEntries,
                    onTapAll: { showScheduledActionsView = true },
                    onTapEntry: { _ in showScheduledActionsView = true }
                )

                HomePhoneScheduleCard(
                    birthdayLeadDays: birthdayLeadDays,
                    scheduleLeadDays: reminderLeadDays,
                    items: scheduleItems,
                    isLoading: isLoadingSchedule,
                    error: scheduleError,
                    onTapManage: { showScheduleDetail = true },
                    onTapItem: { item in
                        openScheduleItem(item)
                    }
                )
            }
        }
        .toolbar(ownsNavigationStack ? .hidden : .visible, for: .navigationBar)

        .onAppear {
            refreshUnreadNotificationCount()
            guard !isRunningUITests else { return }
            Task { await reloadAll() }
        }
        .onChange(of: birthdayLeadDays) { _, _ in
            Task { await reloadPhoneSchedule() }
        }
        .onChange(of: reminderLeadDays) { _, _ in
            Task { await reloadPhoneSchedule() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaCustomSourcesUpdated)) { _ in
            Task { await reloadScheduledActions() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaScheduledActionRunsUpdated)) { _ in
            Task { await reloadScheduledActions() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaNotificationsUpdated)) { _ in
            refreshUnreadNotificationCount()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaHealthDataDidChange)) { _ in
            Task { await reloadHealth() }
        }
        .onChange(of: scenePhase) { _, newPhase in
            guard newPhase == .active, !isRunningUITests else { return }
            refreshUnreadNotificationCount()
            Task {
                await reloadHealth()
                await reloadScheduledActions()
            }
        }
        .refreshable {
            await reloadAll()
        }
        .navigationDestination(isPresented: $showHealthDetail) {
            DataSourceDetailView(sourceType: .health)
                .environmentObject(container)
        }
        .navigationDestination(isPresented: $showScheduleDetail) {
            DataSourcesScheduleHubView()
                .environmentObject(container)
        }
        .navigationDestination(isPresented: $showScheduledActionsView) {
            ScheduledActionsView(embedded: true)
                .environmentObject(container)
        }
        .navigationDestination(isPresented: $showNotificationManager) {
            NotificationCenterView(ownsNavigationStack: false)
                .environmentObject(container)
        }
        .sheet(isPresented: Binding(
            get: { selectedReminderId != nil },
            set: { if !$0 { selectedReminderId = nil } }
        )) {
            if let reminderId = selectedReminderId {
                ReminderDetailSheet(reminderId: reminderId)
            }
        }
        .sheet(isPresented: Binding(
            get: { selectedCalendarEventId != nil },
            set: { if !$0 { selectedCalendarEventId = nil } }
        )) {
            if let eventId = selectedCalendarEventId {
                CalendarEventDetailSheet(eventId: eventId)
            }
        }
    }

    @MainActor
    private func refreshUnreadNotificationCount() {
        unreadNotificationCount = container.notificationManager?.unreadCount ?? 0
    }

    private func openScheduleItem(_ item: HomeScheduleItem) {
        switch item.kind {
        case .localReminder:
            return
        case .reminder:
            let schemes = [
                "x-apple-reminder://\(item.id)",
                "x-apple-reminderkit://\(item.id)"
            ]
            var opened = false
            for s in schemes {
                if let url = URL(string: s), UIApplication.shared.canOpenURL(url) {
                    openURL(url)
                    opened = true
                    break
                }
            }
            if !opened {
                selectedReminderId = item.id
            }
        case .calendar, .birthday:
            let interval = item.date.timeIntervalSinceReferenceDate
            if let url = URL(string: "calshow:\(interval)") {
                if UIApplication.shared.canOpenURL(url) {
                    openURL(url)
                } else {
                    selectedCalendarEventId = item.id
                }
            }
        }
    }

    @MainActor
    private func reloadAll() async {
        await reloadHealth()
        await reloadPhoneSchedule()
        await reloadScheduledActions()
        lastReloadedAt = Date()
    }

    @MainActor
    private func reloadHealth() async {
        await container.healthDataSource?.invalidateSyncCache()
        await healthDashboard.load()
    }

    @MainActor
    private func reloadPhoneSchedule() async {
        guard !isLoadingSchedule else { return }
        isLoadingSchedule = true
        scheduleError = nil
        defer { isLoadingSchedule = false }

        let cal = Calendar.current
        let start = cal.startOfDay(for: Date())
        let birthdayWindowDays = max(0, birthdayLeadDays) + 1
        let scheduleWindowDays = max(0, reminderLeadDays) + 1
        let birthdayEnd = cal.date(byAdding: .day, value: birthdayWindowDays, to: start) ?? start.addingTimeInterval(TimeInterval(birthdayWindowDays) * 24 * 3600)
        let scheduleEnd = cal.date(byAdding: .day, value: scheduleWindowDays, to: start) ?? start.addingTimeInterval(TimeInterval(scheduleWindowDays) * 24 * 3600)

        do {
            async let bdays = ContactsDataSource().fetchEvents(in: DateInterval(start: start, end: birthdayEnd))
            async let cals = CalendarsDataSource().fetchEvents(in: DateInterval(start: start, end: scheduleEnd))
            async let rems = RemindersDataSource().fetchEvents(in: DateInterval(start: start, end: scheduleEnd))

            let (birthdayEvents, calendarEvents, reminderEvents) = try await (bdays, cals, rems)

            let birthdayItems = birthdayEvents.compactMap { ev -> HomeScheduleItem? in
                guard let d = ev.date else { return nil }
                return HomeScheduleItem(id: ev.id, title: ev.title, date: d, kind: .birthday)
            }

            let calendarItems = calendarEvents.compactMap { ev -> HomeScheduleItem? in
                guard let d = ev.date else { return nil }
                return HomeScheduleItem(id: ev.id, title: ev.title, date: d, kind: .calendar)
            }

            let reminderItems = reminderEvents.compactMap { ev -> HomeScheduleItem? in
                guard let d = ev.date else { return nil }
                return HomeScheduleItem(id: ev.id, title: ev.title, date: d, kind: .reminder)
            }

            let localTasks: [UserTask]
            if let storage = container.localJSONStorage {
                localTasks = (try? storage.getAllUserTasks()) ?? []
            } else {
                localTasks = []
            }
            let localReminderItems = localTasks.compactMap { task -> HomeScheduleItem? in
                guard task.type == .reminder,
                      !task.isCompleted,
                      let targetDate = task.targetDate,
                      scheduleEnd > targetDate,
                      targetDate >= start
                else {
                    return nil
                }
                return HomeScheduleItem(id: task.id.uuidString, title: task.title, date: targetDate, kind: .localReminder)
            }

            scheduleItems = (birthdayItems + calendarItems + reminderItems + localReminderItems)
                .sorted { $0.date < $1.date }
        } catch {
            scheduleItems = []
            scheduleError = error.localizedDescription
        }
    }

    @MainActor
    private func reloadScheduledActions() async {
        guard !isLoadingScheduledActions else { return }
        isLoadingScheduledActions = true
        defer { isLoadingScheduledActions = false }

        guard let fetcher = container.urlFetcher else {
            scheduledActionEntries = []
            return
        }

        let scheduledSources = fetcher.sources.filter { src in
            src.isScheduledAutomation
        }

        guard !scheduledSources.isEmpty else {
            scheduledActionEntries = []
            return
        }

        guard let storage = container.localJSONStorage else {
            scheduledActionEntries = scheduledSources.map {
                HomeScheduledActionsOverviewCard.Entry(id: $0.id, name: $0.name, actionId: $0.id.uuidString, latestRun: nil, latestText: nil)
            }
            return
        }

        do {
            let allRuns = try storage.getScheduledActionRuns(limit: 200)
            var bestByActionId: [String: LocalJSONStorage.ScheduledActionRunRecord] = [:]
            var fallbackByActionId: [String: LocalJSONStorage.ScheduledActionRunRecord] = [:]
            for run in allRuns {
                let key = run.actionId
                if fallbackByActionId[key] == nil { fallbackByActionId[key] = run }
                if let jid = run.resultJournalId, !jid.isEmpty, bestByActionId[key] == nil {
                    bestByActionId[key] = run
                }
            }
            var chosen: [String: LocalJSONStorage.ScheduledActionRunRecord] = [:]
            for (key, fb) in fallbackByActionId {
                chosen[key] = bestByActionId[key] ?? fb
            }

            var textSnippets: [String: String] = [:]
            for (aid, run) in chosen {
                guard let jid = run.resultJournalId, !jid.isEmpty else { continue }
                if let journal = try? storage.getJournal(id: jid) {
                    let raw = journal.content
                        .replacingOccurrences(of: "\n\n\n", with: "\n\n")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if !raw.isEmpty { textSnippets[aid] = String(raw.prefix(200)) }
                }
            }

            let entries = scheduledSources.map { src in
                HomeScheduledActionsOverviewCard.Entry(
                    id: src.id,
                    name: src.name,
                    actionId: src.id.uuidString,
                    latestRun: chosen[src.id.uuidString],
                    latestText: textSnippets[src.id.uuidString]
                )
            }.sorted { lhs, rhs in
                let l = lhs.latestRun?.finishedAt ?? .distantPast
                let r = rhs.latestRun?.finishedAt ?? .distantPast
                if l != r { return l > r }
                return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
            }
            scheduledActionEntries = entries
        } catch {
            scheduledActionEntries = []
        }
    }

}

private struct HomeRefreshStrip: View {
    let isLoading: Bool
    let lastReloadedAt: Date?
    let onRefresh: () -> Void

    private var statusText: String {
        guard let lastReloadedAt else {
            return NSLocalizedString("home.refresh.not_yet", value: "尚未刷新", comment: "Home refresh not yet status")
        }
        return lastReloadedAt.formatted(date: .omitted, time: .shortened)
    }

    var body: some View {
        HStack(alignment: .center, spacing: DesignTokens.spacing_md) {
            VStack(alignment: .leading, spacing: 2) {
                Text(NSLocalizedString("home.refresh.title", value: "数据状态", comment: "Home refresh strip title"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.primary)
                Text(isLoading
                     ? NSLocalizedString("home.refresh.loading", value: "正在更新本地数据…", comment: "Home refresh loading status")
                     : String(format: NSLocalizedString("home.refresh.last_updated", value: "最近更新：%@", comment: "Home refresh last updated status"), statusText))
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if isLoading {
                ProgressView()
                    .controlSize(.small)
            }

            Button(NSLocalizedString("home.refresh.button", value: "刷新", comment: "Home refresh button")) {
                onRefresh()
            }
            .font(.callout.weight(.semibold))
            .buttonStyle(.bordered)
            .disabled(isLoading)
        }
        .padding(.horizontal, DesignTokens.spacing_lg)
        .padding(.vertical, DesignTokens.spacing_md)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.elevatedColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

private struct HomeQuickActionsRow: View {
    let onOpenChat: () -> Void
    let onOpenScheduledActions: () -> Void

    var body: some View {
        HStack(spacing: DesignTokens.spacing_sm) {
            Button(action: onOpenChat) {
                HomeQuickActionButton(
                    icon: "bubble.left.and.bubble.right.fill",
                    title: L10n.chatTitle,
                    subtitle: NSLocalizedString("home.quick.chat", value: "继续对话", comment: "Home quick action chat subtitle"),
                    tint: DesignTokens.primaryColor
                )
            }
            .buttonStyle(.plain)

            Button(action: onOpenScheduledActions) {
                HomeQuickActionButton(
                    icon: "clock.arrow.circlepath",
                    title: L10n.scheduledActionsTitle,
                    subtitle: NSLocalizedString("home.quick.scheduled", value: "查看任务", comment: "Home quick action scheduled subtitle"),
                    tint: .orange
                )
            }
            .buttonStyle(.plain)
        }
    }
}

private struct HomeQuickActionButton: View {
    let icon: String
    let title: String
    let subtitle: String
    let tint: Color

    var body: some View {
        HStack(spacing: DesignTokens.spacing_md) {
            ZStack {
                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                    .fill(tint.opacity(DesignTokens.overlaySoftOpacity))
                    .frame(width: 42, height: 42)
                Image(systemName: icon)
                    .foregroundColor(tint)
            }

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.primary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: DesignTokens.spacing_sm)
        }
        .padding(.horizontal, DesignTokens.spacing_md)
        .padding(.vertical, DesignTokens.spacing_md)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.backgroundColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(Color.primary.opacity(DesignTokens.overlaySoftOpacity), lineWidth: 1)
        )
    }
}

private struct DataSourcesScheduleHubView: View {
    @EnvironmentObject var container: ServiceContainer

    var body: some View {
        List {
            Section {
                NavigationLink {
                    DataSourceDetailView(sourceType: .contacts)
                        .environmentObject(container)
                } label: {
                VStack(alignment: .leading, spacing: 10) {
                    Label(L10n.settingsDataSourceContacts, systemImage: DataSourceType.contacts.iconSystemName)
                
                    DataSourceStatusInlineSummary(store: .shared)
                }}

                NavigationLink {
                    DataSourceDetailView(sourceType: .calendars)
                        .environmentObject(container)
                } label: {
                    Label(L10n.settingsDataSourceCalendars, systemImage: DataSourceType.calendars.iconSystemName)
                }

                NavigationLink {
                    DataSourceDetailView(sourceType: .reminders)
                        .environmentObject(container)
                } label: {
                    Label(L10n.settingsDataSourceReminders, systemImage: DataSourceType.reminders.iconSystemName)
                }
            }
        }
        .navigationTitle(L10n.homeSectionDataSources)
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct CategorizedTodayView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let processedEvents: [ProcessedEvent]
    @State private var expandedSections: Set<String> = ["urlHistory", "url", "calendar", "reminder", "health"]

    private func ensureReminderExpandedIfNeeded() {
        let hasReminders = processedEvents.contains { $0.sourceType == .reminder || $0.sourceType == .manual }
        if hasReminders && !expandedSections.contains("reminder") {
            expandedSections.insert("reminder")
            Logger.shared.log(.debug, "CategorizedTodayView", "Auto-expanded reminder section due to new processed event")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let fetcher = container.urlFetcher {
                let sources = fetcher.sources
                if !sources.isEmpty {
                    VStack(alignment: .leading, spacing: 12) {
                        sectionHeaderButton(
                            title: L10n.homeSectionDataSources,
                            id: "urlHistory",
                            count: sources.count
                        )
                        if isExpanded("urlHistory") {
                            VStack(alignment: .leading, spacing: 8) {
                                ForEach(sources, id: \.id) { source in
                                    urlSourceCard(source)
                                }
                            }
                            .nolaDisclosureTransition(reduceMotion: reduceMotion)
                        }
                    }
                    .padding(.horizontal)
                }
            }

            categorySection(title: L10n.homeSectionURLContent, id: "url", events: urlEvents, icon: "link.circle.fill", color: .blue)
            categorySection(title: L10n.homeSectionCalendar, id: "calendar", events: calendarEvents, icon: "calendar.circle.fill", color: .orange)
            categorySection(title: L10n.homeSectionReminders, id: "reminder", events: reminderEvents, icon: "checkmark.circle.fill", color: .green)
            categorySection(title: L10n.homeSectionHealth, id: "health", events: healthEvents, icon: "heart.circle.fill", color: .red)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 8)
        .onAppear { ensureReminderExpandedIfNeeded() }
        .onChange(of: processedEvents.count) { _, _ in ensureReminderExpandedIfNeeded() }
    }

    @ViewBuilder
    private func urlSourceCard(_ source: URLDataSource) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                HStack(spacing: 8) {
                    Image(systemName: "link.circle.fill")
                        .font(.title3)
                        .foregroundColor(.blue)
                    Text(source.name)
                        .font(.system(.body, design: .rounded))
                        .fontWeight(.semibold)
                }
                Spacer()
                Text(L10n.format("url.source.history.count_format", source.history.count))
                    .font(.caption)
                    .fontWeight(.bold)
                    .foregroundColor(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Color(.tertiarySystemFill)))
            }
            
            if source.history.isEmpty {
                Text(L10n.urlSourceHistoryEmpty)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 2)
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(source.history.sorted(by: { $0.timestamp > $1.timestamp }).prefix(3), id: \.id) { entry in
                        NavigationLink(destination: HistoryDetailView(entry: entry, source: source).environmentObject(container)) {
                            HStack(spacing: 12) {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(entry.title)
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                        .lineLimit(2)
                                        .foregroundColor(.primary)
                                    Text(entry.timestamp.formatted(date: .abbreviated, time: .shortened))
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                }
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .font(.caption2)
                                    .foregroundColor(.secondary.opacity(0.5))
                            }
                            .padding(10)
                            .background(
                                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                    .fill(DesignTokens.surfaceColor)
                            )
                            .overlay(
                                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                    .stroke(DesignTokens.borderColor, lineWidth: 1)
                            )
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.surfaceColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }

    @ViewBuilder
    private func categorySection(
        title: String,
        id: String,
        events: [ProcessedEvent],
        icon: String,
        color: Color
    ) -> some View {
        if !events.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                sectionHeaderButton(
                    title: title,
                    id: id,
                    count: events.count,
                    icon: icon,
                    color: color
                )
                if isExpanded(id) {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(events.prefix(5), id: \.id) { ev in
                            categoryEventCard(ev, icon: icon, color: color)
                        }
                    }
                    .nolaDisclosureTransition(reduceMotion: reduceMotion)
                }
            }
            .padding(.horizontal)
        }
    }

    @ViewBuilder
    private func categoryEventCard(
        _ event: ProcessedEvent,
        icon: String,
        color: Color
    ) -> some View {
        NavigationLink(value: event.dedupeKey) {
            HStack(spacing: 12) {
                ZStack {
                    Circle()
                        .fill(color.opacity(0.15))
                        .frame(width: 40, height: 40)
                    Image(systemName: icon)
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundColor(color)
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text(event.title)
                        .font(.body)
                        .fontWeight(.semibold)
                        .foregroundColor(.primary)
                        .lineLimit(2)
                    
                    HStack(spacing: 8) {
                        if let notes = event.notes, !notes.isEmpty {
                            Text(notes)
                                .font(.caption)
                                .foregroundColor(.secondary)
                                .lineLimit(1)
                        }
                        
                        if let date = event.date {
                            HStack(spacing: 3) {
                                Image(systemName: "calendar")
                                    .font(.caption2)
                                Text(date.formatted(date: .abbreviated, time: .shortened))
                                    .font(.caption)
                            }
                            .foregroundColor(.secondary)
                        }
                    }
                }
                
                Spacer()
                
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(.secondary.opacity(0.4))
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.surfaceColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }

    @ViewBuilder
    private func sectionHeaderButton(
        title: String,
        id: String,
        count: Int,
        icon: String? = nil,
        color: Color = .blue
    ) -> some View {
        Button(action: { toggle(id) }) {
            HStack(spacing: 10) {
                if let icon = icon {
                    ZStack {
                        Circle()
                            .fill(color.opacity(0.15))
                            .frame(width: 32, height: 32)
                        Image(systemName: icon)
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundColor(color)
                    }
                }
                Text(title)
                    .font(.system(.headline, design: .rounded))
                    .fontWeight(.bold)
                    .foregroundColor(.primary)
                Spacer()
                HStack(spacing: 8) {
                    Text(String(count))
                        .font(.caption)
                        .fontWeight(.bold)
                        .foregroundColor(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(
                            Capsule()
                                .fill(color.opacity(0.8))
                        )
                    Image(systemName: isExpanded(id) ? "chevron.up.circle.fill" : "chevron.down.circle.fill")
                        .font(.system(size: 18))
                        .foregroundColor(color.opacity(0.6))
                }
            }
            .contentShape(Rectangle())
            .padding(.vertical, 6)
        }
        .buttonStyle(.plain)
    }

    private func isExpanded(_ id: String) -> Bool { expandedSections.contains(id) }
    private func toggle(_ id: String) {
        withAnimation(reduceMotion ? nil : DesignTokens.disclosureAnimation) {
            if expandedSections.contains(id) {
                expandedSections.remove(id)
            } else {
                expandedSections.insert(id)
            }
        }
    }

    private var urlEvents: [ProcessedEvent] { processedEvents.filter { $0.sourceType == .url } }
    private var calendarEvents: [ProcessedEvent] { processedEvents.filter { $0.sourceType == .calendar } }
    private var reminderEvents: [ProcessedEvent] { processedEvents.filter { $0.sourceType == .reminder || $0.sourceType == .manual } }
    private var healthEvents: [ProcessedEvent] { processedEvents.filter { $0.sourceType == .health } }
}

struct SummaryItemCard: View {
    @EnvironmentObject var container: ServiceContainer
    let item: SummaryItem
    let prefersEventDetail: Bool
    @State private var isPressed = false

    var body: some View {
        Group {
            if prefersEventDetail {
                NavigationLink(value: item.dedupeKey) {
                    cardContent
                }
            } else {
                NavigationLink(destination: SummaryDetailView(item: item).environmentObject(container)) {
                    cardContent
                }
            }
        }
        .buttonStyle(.plain)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in isPressed = true }
                .onEnded { _ in isPressed = false }
        )
    }

    private var cardContent: some View {
        HStack(spacing: 14) {
                VStack(spacing: 6) {
                    ZStack {
                        Circle()
                            .fill(priorityColor.opacity(0.15))
                            .frame(width: 44, height: 44)
                        
                        Image(systemName: prioritySymbol)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundColor(priorityColor)
                    }
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(item.text)
                        .font(.body)
                        .fontWeight(.semibold)
                        .foregroundColor(.primary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                    
                    HStack(spacing: 8) {
                        HStack(spacing: 4) {
                            Image(systemName: "chart.bar.fill")
                                .font(.caption2)
                            Text(L10n.format("home.priority.percent_format", Int(item.priority * 100)))
                                .font(.caption)
                        }
                        .foregroundColor(.secondary)
                        
                        if item.action != nil {
                            HStack(spacing: 4) {
                                Image(systemName: "hand.tap.fill")
                                    .font(.caption2)
                                Text(L10n.itemActionable)
                                    .font(.caption)
                            }
                            .foregroundColor(.blue)
                        }
                    }
                }

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(.secondary.opacity(0.5))
            }
            .contentShape(Rectangle())
            .padding(16)
            .background(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .fill(DesignTokens.surfaceColor)
            )
            .overlay(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .stroke(DesignTokens.borderColor, lineWidth: 1)
            )
            .scaleEffect(isPressed ? 0.98 : 1.0)
        }
    private var priorityColor: Color {
        if item.priority >= 0.8 { return .red }
        if item.priority >= 0.5 { return .orange }
        return .green
    }

    private var priorityLabel: String {
        if item.priority >= 0.8 { return L10n.homePriorityHigh }
        if item.priority >= 0.5 { return L10n.homePriorityMedium }
        return L10n.homePriorityLow
    }

    private var prioritySymbol: String {
        if item.priority >= 0.8 { return "flame.fill" }
        if item.priority >= 0.5 { return "bolt.fill" }
        return "checkmark"
    }
}

struct HistoryDetailView: View {
    @EnvironmentObject var container: ServiceContainer
    let entry: URLDataSource.HistoryEntry
    let source: URLDataSource
    @State private var showFullResponse = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(L10n.historyDataSourceTitle)
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text(source.name)
                        .font(.title2)
                        .fontWeight(.semibold)
                    Text(source.url)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(2)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
                .background(
                    RoundedRectangle(cornerRadius: 12)
                        .fill(Color(.secondarySystemBackground))
                )

                VStack(alignment: .leading, spacing: 8) {
                    Text(L10n.historyAISummaryTitle)
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text(entry.title)
                        .font(.headline)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
                .background(
                    RoundedRectangle(cornerRadius: 12)
                        .fill(Color(.secondarySystemBackground))
                )

                if entry.tokensUsed != nil || entry.processingDuration != nil || entry.modelUsed != nil {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(L10n.historyProcessingInfoTitle)
                            .font(.caption)
                            .foregroundColor(.secondary)
                        
                        if let tokens = entry.tokensUsed {
                            HStack {
                                Image(systemName: "number.circle.fill")
                                    .foregroundColor(.blue)
                                Text(L10n.historyTokenCountLabel)
                                    .foregroundColor(.secondary)
                                Spacer()
                                Text(String(tokens))
                                    .fontWeight(.semibold)
                            }
                        }
                        
                        if let duration = entry.processingDuration {
                            HStack {
                                Image(systemName: "clock.fill")
                                    .foregroundColor(.green)
                                Text(L10n.historyProcessingDurationLabel)
                                    .foregroundColor(.secondary)
                                Spacer()
                                Text(String(format: "%.2fs", duration))
                                    .fontWeight(.semibold)
                            }
                        }
                        
                        if let model = entry.modelUsed {
                            HStack {
                                Image(systemName: "brain.head.profile")
                                    .foregroundColor(.purple)
                                Text(L10n.historyModelLabel)
                                    .foregroundColor(.secondary)
                                Spacer()
                                Text(model)
                                    .font(.caption)
                                    .fontWeight(.semibold)
                                    .lineLimit(1)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                    .background(
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color(.secondarySystemBackground))
                    )
                }

                HStack {
                    Image(systemName: "calendar")
                        .foregroundColor(.secondary)
                    Text(L10n.itemTimeLabel)
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Spacer()
                    Text(entry.timestamp.formatted(date: .numeric, time: .standard))
                        .font(.caption)
                }
                .padding()
                .background(
                    RoundedRectangle(cornerRadius: 12)
                        .fill(Color(.secondarySystemBackground))
                )

                if let details = entry.details {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(L10n.historyOriginalContentTitle)
                            .font(.caption)
                            .foregroundColor(.secondary)
                        Text(details)
                            .font(.body)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                    .background(
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color(.secondarySystemBackground))
                    )
                }

                if let response = entry.rawResponse {
                    VStack(alignment: .leading, spacing: 8) {
                        Button(action: { showFullResponse.toggle() }) {
                            HStack {
                                Text(L10n.historyRawResponseTitle)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                Spacer()
                                Image(systemName: showFullResponse ? "chevron.up" : "chevron.down")
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        }
                        
                        if showFullResponse {
                            ScrollView {
                                Text(response)
                                    .font(.system(.caption, design: .monospaced))
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .frame(maxHeight: 300)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                    .background(
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color(.secondarySystemBackground))
                    )
                }

                if entry.eventId != nil {
                    NavigationLink(destination: NotificationDetailView(dedupeKey: entry.dedupeKey).environmentObject(container)) {
                        HStack {
                            Image(systemName: "bell.fill")
                                .foregroundColor(.orange)
                            Text(L10n.notificationViewDetails)
                                .foregroundColor(.blue)
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                        .padding()
                        .background(
                            RoundedRectangle(cornerRadius: 12)
                                .fill(Color(.secondarySystemBackground))
                        )
                    }
                }
            }
            .padding()
        }
        .navigationTitle(L10n.historyDetailsTitle)
        .navigationBarTitleDisplayMode(.inline)
    }
}
