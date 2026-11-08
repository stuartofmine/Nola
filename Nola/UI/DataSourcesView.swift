// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSourcesView logic in the iOS UI layer.
// - Interfaces:
//   - DataSourcesView
//   - DataSourceOverviewHero
//   - DataSourceCapabilityIntro
//   - OverviewPill
//   - DataSourcesCardSection
//   - OverviewStat
//   - EnhancedDataSourceRow
//   - DataSourceRow
// - APIs:
//   - none
//
import SwiftUI
import Combine

enum DataSourceRuntimePhase: Equatable {
    case checking
    case ready
    case needsPermission
    case syncing
    case stale
    case error
    case unavailable
    case disabled
}

struct DataSourceRuntimeState {
    let phase: DataSourceRuntimePhase
    let permission: AuthorizationStatus
    let itemCount: Int?
    let lastReadAt: Date?
    let errorMessage: String?

    static let checking = DataSourceRuntimeState(
        phase: .checking,
        permission: .notDetermined,
        itemCount: nil,
        lastReadAt: nil,
        errorMessage: nil
    )

    var statusText: String {
        let key: String
        let fallback: String
        switch phase {
        case .checking, .syncing:
            key = "data_sources.runtime.status.syncing"
            fallback = "Updating"
        case .ready:
            key = "data_sources.runtime.status.ready"
            fallback = "Ready"
        case .needsPermission:
            key = "data_sources.runtime.status.needs_permission"
            fallback = "Access needed"
        case .stale:
            key = "data_sources.runtime.status.stale"
            fallback = "May be outdated"
        case .error:
            key = "data_sources.runtime.status.error"
            fallback = "Refresh failed"
        case .unavailable:
            key = "data_sources.runtime.status.unavailable"
            fallback = "Unavailable"
        case .disabled:
            key = "data_sources.runtime.status.disabled"
            fallback = "Off"
        }
        return NSLocalizedString(key, value: fallback, comment: "Data source runtime status")
    }

    var statusIcon: String {
        switch phase {
        case .checking, .syncing: return "arrow.triangle.2.circlepath"
        case .ready: return "checkmark"
        case .needsPermission: return "lock.open"
        case .stale: return "clock"
        case .error: return "exclamationmark"
        case .unavailable: return "slash.circle"
        case .disabled: return "pause"
        }
    }

    var tint: Color {
        switch phase {
        case .ready: return .green
        case .checking, .syncing: return DesignTokens.primaryColor
        case .needsPermission, .stale: return .orange
        case .error: return .red
        case .unavailable, .disabled: return .secondary
        }
    }

    var detailText: String {
        let lastRead = lastReadAt.map(relativeText)
        switch phase {
        case .checking, .syncing:
            return NSLocalizedString(
                "data_sources.runtime.detail.syncing",
                value: "Checking the latest available data...",
                comment: "Data source syncing detail"
            )
        case .ready:
            if let itemCount, let lastRead {
                let format = NSLocalizedString(
                    "data_sources.runtime.detail.ready_count_format",
                    value: "%1$d items read · %2$@",
                    comment: "Data source ready detail"
                )
                return String(format: format, locale: Locale.current, itemCount, lastRead)
            }
            return NSLocalizedString(
                "data_sources.runtime.detail.ready_empty",
                value: "Read succeeded · no recent items",
                comment: "Data source ready empty detail"
            )
        case .needsPermission:
            return permission == .notDetermined
                ? NSLocalizedString("data_sources.runtime.detail.request_access", value: "Allow access before Nola can read this source", comment: "Data source request access detail")
                : NSLocalizedString("data_sources.runtime.detail.open_settings", value: "Access is off · change it in System Settings", comment: "Data source open settings detail")
        case .stale:
            let format = NSLocalizedString(
                "data_sources.runtime.detail.stale_format",
                value: "Last successful read %1$@",
                comment: "Stale data source detail"
            )
            return String(format: format, locale: Locale.current, lastRead ?? "—")
        case .error:
            if let lastRead {
                let format = NSLocalizedString(
                    "data_sources.runtime.detail.error_cached_format",
                    value: "Refresh failed · last read %1$@",
                    comment: "Data source error with cached read detail"
                )
                return String(format: format, locale: Locale.current, lastRead)
            }
            return NSLocalizedString("data_sources.runtime.detail.error", value: "No successful read yet", comment: "Data source error detail")
        case .unavailable:
            return NSLocalizedString("data_sources.runtime.detail.unavailable", value: "This source is not available on this device", comment: "Data source unavailable detail")
        case .disabled:
            return NSLocalizedString("data_sources.runtime.detail.disabled", value: "This source is currently turned off", comment: "Data source disabled detail")
        }
    }

    private func relativeText(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

@MainActor
final class DataSourceStatusStore: ObservableObject {
    static let shared = DataSourceStatusStore()

    @Published private(set) var states: [DataSourceType: DataSourceRuntimeState] = [:]
    @Published private(set) var isRefreshing = false

    let systemTypes: [DataSourceType] = [.contacts, .calendars, .reminders, .health]

    var readyCount: Int {
        states.values.filter { $0.phase == .ready }.count
    }

    var attentionCount: Int {
        states.values.filter { [.needsPermission, .stale, .error, .unavailable].contains($0.phase) }.count
    }

    var syncingCount: Int {
        if isRefreshing {
            return states.isEmpty ? systemTypes.count : states.values.filter { [.checking, .syncing].contains($0.phase) }.count
        }
        return states.values.filter { [.checking, .syncing].contains($0.phase) }.count
    }

    var lastSuccessfulRead: Date? {
        states.values.compactMap(\.lastReadAt).max()
    }

    var errorMessages: [String] {
        states.compactMap { type, state in
            guard state.phase == .error, let message = state.errorMessage else { return nil }
            return "\(type.displayName): \(message)"
        }
    }

    func state(for type: DataSourceType) -> DataSourceRuntimeState {
        states[type] ?? cachedState(for: type) ?? .checking
    }

    func refresh(container: ServiceContainer, healthDashboard: HealthDashboardStore) async {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }

        var syncingStates = states
        for type in systemTypes {
            let previous = state(for: type)
            syncingStates[type] = DataSourceRuntimeState(
                phase: .syncing,
                permission: previous.permission,
                itemCount: previous.itemCount,
                lastReadAt: previous.lastReadAt,
                errorMessage: nil
            )
        }
        states = syncingStates

        var refreshedStates = syncingStates
        for type in systemTypes {
            refreshedStates[type] = await probe(type, container: container, healthDashboard: healthDashboard)
        }
        states = refreshedStates
    }

    private func probe(
        _ type: DataSourceType,
        container: ServiceContainer,
        healthDashboard: HealthDashboardStore
    ) async -> DataSourceRuntimeState {
        let cached = cachedState(for: type)
        let permission = await permissionStatus(for: type, container: container)

        if type == .health {
            if !container.healthDataSourceEnabled, permission == .authorized {
                return DataSourceRuntimeState(
                    phase: .disabled,
                    permission: permission,
                    itemCount: cached?.itemCount,
                    lastReadAt: cached?.lastReadAt,
                    errorMessage: nil
                )
            }

            await healthDashboard.load()
            let state = Self.resolveHealthState(
                snapshot: healthDashboard.snapshot,
                errorMessage: healthDashboard.errorMessage,
                reportedPermission: permission,
                cached: cached
            )
            if state.phase == .ready, let snapshot = healthDashboard.snapshot {
                return successfulState(
                    type: .health,
                    permission: .authorized,
                    count: snapshot.availableMetricCount,
                    date: snapshot.generatedAt
                )
            }
            return state
        }

        switch permission {
        case .notDetermined, .denied, .insufficient:
            return DataSourceRuntimeState(
                phase: .needsPermission,
                permission: permission,
                itemCount: cached?.itemCount,
                lastReadAt: cached?.lastReadAt,
                errorMessage: nil
            )
        case .restricted, .unavailable:
            return DataSourceRuntimeState(
                phase: .unavailable,
                permission: permission,
                itemCount: cached?.itemCount,
                lastReadAt: cached?.lastReadAt,
                errorMessage: nil
            )
        case .authorized:
            break
        }

        do {
            let now = Date()
            let end = Calendar.current.date(byAdding: .day, value: 30, to: now) ?? now
            let range = DateInterval(start: now, end: end)
            let events: [RawEvent]
            switch type {
            case .contacts:
                events = try await ContactsDataSource().fetchEvents(in: range)
            case .calendars:
                events = try await CalendarsDataSource().fetchEvents(in: range)
            case .reminders:
                events = try await RemindersDataSource().fetchEvents(in: range)
            case .health, .url:
                events = []
            }
            return successfulState(type: type, permission: permission, count: events.count, date: now)
        } catch {
            return failedState(permission: permission, cached: cached, message: error.localizedDescription)
        }
    }

    private func permissionStatus(for type: DataSourceType, container: ServiceContainer) async -> AuthorizationStatus {
        guard type == .health else {
            return DataSourcePermissionManager.shared.getPermissionStatus(for: type)
        }
        let source = container.healthDataSource ?? HealthDataSource()
        switch await source.currentReadAuthorizationState() {
        case .authorized: return .authorized
        case .denied: return .denied
        case .notDetermined: return .notDetermined
        case .unavailable: return .unavailable
        }
    }

    static func resolveHealthState(
        snapshot: HealthDashboardSnapshot?,
        errorMessage: String?,
        reportedPermission: AuthorizationStatus,
        cached: DataSourceRuntimeState?
    ) -> DataSourceRuntimeState {
        if let snapshot, snapshot.hasReadableData, errorMessage == nil {
            return DataSourceRuntimeState(
                phase: Date().timeIntervalSince(snapshot.generatedAt) > 21_600 ? .stale : .ready,
                permission: .authorized,
                itemCount: snapshot.availableMetricCount,
                lastReadAt: snapshot.generatedAt,
                errorMessage: nil
            )
        }

        switch reportedPermission {
        case .notDetermined, .denied, .insufficient:
            return DataSourceRuntimeState(
                phase: .needsPermission,
                permission: reportedPermission,
                itemCount: cached?.itemCount,
                lastReadAt: cached?.lastReadAt,
                errorMessage: errorMessage
            )
        case .restricted, .unavailable:
            return DataSourceRuntimeState(
                phase: .unavailable,
                permission: reportedPermission,
                itemCount: cached?.itemCount,
                lastReadAt: cached?.lastReadAt,
                errorMessage: errorMessage
            )
        case .authorized:
            return DataSourceRuntimeState(
                phase: .error,
                permission: .authorized,
                itemCount: snapshot?.availableMetricCount ?? cached?.itemCount,
                lastReadAt: snapshot?.generatedAt ?? cached?.lastReadAt,
                errorMessage: errorMessage ?? NSLocalizedString(
                    "data_sources.runtime.error.no_snapshot",
                    value: "No readable health summary was returned.",
                    comment: "Missing health snapshot error"
                )
            )
        }
    }

    private func successfulState(type: DataSourceType, permission: AuthorizationStatus, count: Int, date: Date) -> DataSourceRuntimeState {
        UserDefaults.standard.set(date.timeIntervalSince1970, forKey: lastReadKey(for: type))
        UserDefaults.standard.set(count, forKey: itemCountKey(for: type))
        return DataSourceRuntimeState(
            phase: Date().timeIntervalSince(date) > 21_600 ? .stale : .ready,
            permission: permission,
            itemCount: count,
            lastReadAt: date,
            errorMessage: nil
        )
    }

    private func failedState(permission: AuthorizationStatus, cached: DataSourceRuntimeState?, message: String) -> DataSourceRuntimeState {
        DataSourceRuntimeState(
            phase: .error,
            permission: permission,
            itemCount: cached?.itemCount,
            lastReadAt: cached?.lastReadAt,
            errorMessage: message
        )
    }

    private func cachedState(for type: DataSourceType) -> DataSourceRuntimeState? {
        let timestamp = UserDefaults.standard.double(forKey: lastReadKey(for: type))
        guard timestamp > 0 else { return nil }
        let date = Date(timeIntervalSince1970: timestamp)
        return DataSourceRuntimeState(
            phase: Date().timeIntervalSince(date) > 21_600 ? .stale : .ready,
            permission: DataSourcePermissionManager.shared.getPermissionStatus(for: type),
            itemCount: UserDefaults.standard.integer(forKey: itemCountKey(for: type)),
            lastReadAt: date,
            errorMessage: nil
        )
    }

    private func lastReadKey(for type: DataSourceType) -> String {
        "nola.dataSource.lastRead.\(type.rawValue)"
    }

    private func itemCountKey(for type: DataSourceType) -> String {
        "nola.dataSource.itemCount.\(type.rawValue)"
    }
}

struct DataSourcesView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    
    @State private var isRefreshingSources = false
    @State private var refreshSourcesError: String? = nil
    @State private var showRefreshSourcesError = false
    @State private var lastAutomaticRefresh = Date.distantPast
    @StateObject private var sourceStatusStore = DataSourceStatusStore.shared
    @StateObject private var healthDashboard = HealthDashboardStore.shared
    
    var body: some View {
        ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    DataSourceOverviewHero(store: sourceStatusStore)
                        .accessibilityIdentifier("data.center.hero")

                    DataSourcesCardSection(
                        title: NSLocalizedString("data_sources.runtime.device_section", value: "On this device", comment: "Device data sources section"),
                        titleIdentifier: "data.center.section.device"
                    ) {
                        VStack(spacing: 0) {
                            NavigationLink {
                                DataSourceDetailView(sourceType: .contacts)
                                    .environmentObject(container)
                            } label: {
                                EnhancedDataSourceRow(
                                    icon: "person.2.fill",
                                    title: L10n.settingsDataSourceContacts,
                                    subtitle: L10n.dataSourceContactsSubtitle,
                                    color: .blue,
                                    state: sourceStatusStore.state(for: .contacts),
                                    policy: SystemDataSourceAccessPolicyStore.shared.policy(for: .contacts)
                                )
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("data.center.source.contacts")

                            NolaDivider(leading: 58)

                            NavigationLink {
                                DataSourceDetailView(sourceType: .calendars)
                                    .environmentObject(container)
                            } label: {
                                EnhancedDataSourceRow(
                                    icon: "calendar",
                                    title: L10n.settingsDataSourceCalendars,
                                    subtitle: L10n.dataSourceCalendarsSubtitle,
                                    color: .orange,
                                    state: sourceStatusStore.state(for: .calendars),
                                    policy: SystemDataSourceAccessPolicyStore.shared.policy(for: .calendars)
                                )
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("data.center.source.calendars")

                            NolaDivider(leading: 58)

                            NavigationLink {
                                DataSourceDetailView(sourceType: .reminders)
                                    .environmentObject(container)
                            } label: {
                                EnhancedDataSourceRow(
                                    icon: "checklist",
                                    title: L10n.settingsDataSourceReminders,
                                    subtitle: L10n.dataSourceRemindersSubtitle,
                                    color: .green,
                                    state: sourceStatusStore.state(for: .reminders),
                                    policy: SystemDataSourceAccessPolicyStore.shared.policy(for: .reminders)
                                )
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("data.center.source.reminders")

                            NolaDivider(leading: 58)

                            NavigationLink {
                                DataSourceDetailView(sourceType: .health, healthDashboard: healthDashboard)
                                    .environmentObject(container)
                            } label: {
                                HealthDataSourceRow(
                                    dashboard: healthDashboard,
                                    runtimeState: sourceStatusStore.state(for: .health),
                                    policy: SystemDataSourceAccessPolicyStore.shared.policy(for: .health)
                                )
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("data.center.source.health")
                        }
                    }
                    DataSourcesCardSection(title: NSLocalizedString("data_sources.readable_sources.title", value: "Readable sources", comment: "Readable sources section title")) {
                        InlineURLSourcesList()
                            .environmentObject(container)
                    }

                    DataSourceAccessGuide()

                    if let fetcher = container.urlFetcher, !fetcher.usageRecords.isEmpty {
                        DataSourcesCardSection(title: NSLocalizedString("data_sources.usage.title", value: "Recent use", comment: "Data source recent usage title")) {
                            DataSourceUsageTimeline(fetcher: fetcher)
                        }
                    }

                }
                .frame(maxWidth: 760, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 24)
            }
            .accessibilityElement(children: .contain)
            .refreshable {
                await refreshDataSources(force: true, showErrors: true)
            }
            .accessibilityIdentifier("data.center.screen")
            .background(DesignTokens.backgroundColor)
            .navigationTitle(L10n.dataSourcesTitle)
            .navigationBarTitleDisplayMode(.large)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task {
                            await refreshDataSources(force: true, showErrors: true)
                        }
                    } label: {
                        Group {
                            if isRefreshingSources {
                                ProgressView()
                                    .controlSize(.small)
                            } else {
                                Image(systemName: "arrow.clockwise")
                            }
                        }
                        .frame(width: 22, height: 22)
                    }
                    .disabled(isRefreshingSources)
                    .accessibilityLabel(L10n.homeRefresh)
                    .accessibilityIdentifier("data.center.refresh")
                }
            }
            .task {
                await refreshDataSources(force: false, showErrors: false)
            }
            .onChange(of: scenePhase) { _, newPhase in
                guard newPhase == .active else { return }
                Task { await refreshDataSources(force: false, showErrors: false) }
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaCustomSourcesUpdated)) { _ in
                Task { await refreshDataSources(force: false, showErrors: false) }
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaScheduledActionRunsUpdated)) { _ in
                Task { await refreshDataSources(force: false, showErrors: false) }
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaHealthDataDidChange)) { notification in
                if let source = notification.object as? HealthDashboardStore, source === healthDashboard {
                    return
                }
                Task { await refreshDataSources(force: true, showErrors: false) }
            }
            .alert(L10n.homeDashboardPartialSourcesFailedTitle, isPresented: $showRefreshSourcesError) {
                Button(L10n.ok, role: .cancel) {}
            } message: {
                Text(refreshSourcesError ?? "")
        }
    }

    private func refreshDataSources(force: Bool, showErrors: Bool) async {
        guard !isRefreshingSources else { return }
        let now = Date()
        guard force || now.timeIntervalSince(lastAutomaticRefresh) >= 20 else { return }

        isRefreshingSources = true
        lastAutomaticRefresh = now
        await sourceStatusStore.refresh(container: container, healthDashboard: healthDashboard)
        isRefreshingSources = false
        NotificationCenter.default.post(name: .nolaHealthDataDidChange, object: healthDashboard)

        guard showErrors, !sourceStatusStore.errorMessages.isEmpty else { return }
        refreshSourcesError = sourceStatusStore.errorMessages.sorted().joined(separator: "\n")
        showRefreshSourcesError = true
    }
}

struct DataSourceStatusInlineSummary: View {
    @EnvironmentObject private var container: ServiceContainer
    @ObservedObject var store: DataSourceStatusStore
    @StateObject private var healthDashboard = HealthDashboardStore.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 8) {
                statusPill(
                    value: store.readyCount,
                    label: NSLocalizedString(
                        "data_sources.runtime.metric.ready",
                        value: "Ready",
                        comment: "Ready data source count"
                    ),
                    tint: .green
                )

                if store.syncingCount > 0 {
                    statusPill(
                        value: store.syncingCount,
                        label: NSLocalizedString(
                            "data_sources.runtime.status.syncing",
                            value: "Updating",
                            comment: "Updating data source count"
                        ),
                        tint: DesignTokens.primaryColor
                    )
                } else {
                    statusPill(
                        value: store.attentionCount,
                        label: NSLocalizedString(
                            "data_sources.runtime.metric.attention",
                            value: "Attention",
                            comment: "Data sources needing attention count"
                        ),
                        tint: store.attentionCount > 0 ? .orange : .secondary
                    )
                }
            }

            if let lastRead = store.lastSuccessfulRead {
                Text(lastReadText(lastRead))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .task {
            await store.refresh(container: container, healthDashboard: healthDashboard)
        }
    }

    private func statusPill(value: Int, label: String, tint: Color) -> some View {
        HStack(spacing: 5) {
            Circle()
                .fill(tint)
                .frame(width: 6, height: 6)
            Text("\(value) \(label)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 5)
        .background(tint.opacity(0.1), in: Capsule())
    }

    private func lastReadText(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        let relative = formatter.localizedString(for: date, relativeTo: Date())
        let format = NSLocalizedString(
            "data_sources.runtime.last_read_format",
            value: "Latest successful read %1$@",
            comment: "Latest successful data source read"
        )
        return String(format: format, locale: Locale.current, relative)
    }
}

struct DataSourceOverviewHero: View {
    @ObservedObject var store: DataSourceStatusStore
    
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center, spacing: 10) {
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(DesignTokens.primaryColor)
                    .frame(width: 24)

                VStack(alignment: .leading, spacing: 5) {
                    Text(NSLocalizedString("data_sources.runtime.hero.title", value: "Live context", comment: "Data source runtime hero title"))
                        .font(.headline.weight(.semibold))
                    Text(NSLocalizedString("data_sources.runtime.hero.subtitle", value: "A source is ready only after Nola has read it successfully.", comment: "Data source runtime hero subtitle"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { overviewPills }
                VStack(alignment: .leading, spacing: 4) { overviewPills }
            }
            .padding(.leading, 34)

            HStack(spacing: 6) {
                Image(systemName: "clock")
                Text(lastReadText)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.leading, 34)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    private var lastReadText: String {
        guard let date = store.lastSuccessfulRead else {
            return NSLocalizedString("data_sources.runtime.never_read", value: "No successful read yet", comment: "No successful data source read")
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        let relative = formatter.localizedString(for: date, relativeTo: Date())
        let format = NSLocalizedString("data_sources.runtime.last_read_format", value: "Latest successful read %1$@", comment: "Latest successful source read")
        return String(format: format, locale: Locale.current, relative)
    }

    @ViewBuilder
    private var overviewPills: some View {
        OverviewPill(icon: "checkmark", label: NSLocalizedString("data_sources.runtime.metric.ready", value: "Ready", comment: "Ready source metric"), value: "\(store.readyCount)", tint: .green)
        OverviewPill(
            icon: store.attentionCount > 0 ? "exclamationmark" : "checkmark.shield",
            label: NSLocalizedString("data_sources.runtime.metric.attention", value: "Attention", comment: "Attention source metric"),
            value: "\(store.attentionCount)",
            tint: store.attentionCount > 0 ? .orange : .secondary
        )
        if store.syncingCount > 0 {
            OverviewPill(icon: "arrow.triangle.2.circlepath", label: NSLocalizedString("data_sources.runtime.metric.updating", value: "Updating", comment: "Updating source metric"), value: "\(store.syncingCount)", tint: DesignTokens.primaryColor)
        }
    }
}

private struct DataSourceCapabilityIntro: View {
    @State private var isExpanded = false

    private let capabilities: [(String, String, Color)] = [
        ("text.badge.plus", NSLocalizedString("data_sources.capability.context", value: "Inject into chat context", comment: "Data source capability context"), DesignTokens.primaryColor),
        ("message.badge.waveform", NSLocalizedString("data_sources.capability.configure_in_chat", value: "Configurable in chat", comment: "Data source capability configure in chat"), .teal),
        ("clock.badge.checkmark", NSLocalizedString("data_sources.capability.scheduled", value: "Schedule and notify", comment: "Data source capability scheduled"), .orange),
        ("lock.shield", NSLocalizedString("data_sources.capability.local_first", value: "Local-first storage", comment: "Data source capability local first"), .green)
    ]

    private let kinds: [DataSourceKind] = [.website, .webSearch, .folder, .file, .api, .feed, .system, .custom]

    var body: some View {
        NolaDisclosure(isExpanded: $isExpanded) {
            VStack(alignment: .leading, spacing: 12) {
                Text(NSLocalizedString("data_sources.capability.section_subtitle", value: "Data sources are not just URLs. They are any source Nola can read, summarize, search, and inject into context. You can reference them in chat or let Nola create and adjust scheduled actions.", comment: "Data source capability section subtitle"))
                    .font(.subheadline)
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                LazyVGrid(columns: [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: 10) {
                    ForEach(capabilities, id: \.1) { item in
                        HStack(spacing: 8) {
                            Image(systemName: item.0)
                                .font(.caption)
                                .foregroundColor(item.2)
                            Text(item.1)
                                .font(.caption.weight(.semibold))
                                .foregroundColor(.primary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(kinds, id: \.self) { kind in
                            HStack(spacing: 6) {
                                Image(systemName: kind.iconSystemName)
                                Text(kind.displayName)
                            }
                            .font(.caption2.weight(.medium))
                            .foregroundColor(.secondary)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .background(Color.secondary.opacity(0.10))
                            .clipShape(Capsule())
                        }
                    }
                }
            }
            .padding(.top, 12)
        } label: {
            HStack(spacing: 11) {
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(.teal)
                    .frame(width: 34, height: 34)
                    .background(
                        Color.teal.opacity(0.09),
                        in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                    )
                VStack(alignment: .leading, spacing: 2) {
                    Text(NSLocalizedString("data_sources.capability.section_title", value: "Unified data source capabilities", comment: "Data source capability section title"))
                        .font(.subheadline.weight(.semibold))
                    Text(NSLocalizedString("data_sources.capability.section_subtitle", value: "Data sources are not just URLs. They are any source Nola can read, summarize, search, and inject into context. You can reference them in chat or let Nola create and adjust scheduled actions.", comment: "Data source capability section subtitle"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

struct OverviewPill: View {
    let icon: String
    let label: String
    let value: String
    let tint: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 5) {
                Image(systemName: icon)
                Text(label)
                    .lineLimit(1)
                    .minimumScaleFactor(0.72)
            }
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            Text(value)
                .font(.system(.title3, design: .rounded, weight: .bold))
                .monospacedDigit()
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.075), in: RoundedRectangle(cornerRadius: 13, style: .continuous))
    }
}

struct DataSourcesCardSection<Content: View>: View {
    let title: String
    let titleIdentifier: String?
    let content: Content

    init(
        title: String,
        titleIdentifier: String? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.titleIdentifier = titleIdentifier
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
            Group {
                if let titleIdentifier {
                    Text(title)
                        .accessibilityIdentifier(titleIdentifier)
                } else {
                    Text(title)
                }
            }
            .font(.headline)
            .foregroundColor(.primary)

            VStack(alignment: .leading, spacing: 0) { content }
        }
    }
}

struct OverviewStat: View {
    let icon: String
    let color: Color
    let label: String
    let value: String
    
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Image(systemName: icon)
                .font(.title3)
                .foregroundColor(color)
            
            Text(value)
                .font(.title2)
                .fontWeight(.bold)
            
            Text(label)
                .font(.caption2)
                .foregroundColor(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct EnhancedDataSourceRow: View {
    let icon: String
    let title: String
    let subtitle: String
    let color: Color
    let state: DataSourceRuntimeState
    let policy: URLDataSourceAccessPolicy
    
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(color)
                .frame(width: 24)
            
            VStack(alignment: .leading, spacing: 5) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(title)
                            .font(.body.weight(.semibold))
                            .lineLimit(1)
                        Spacer(minLength: 6)
                        DataSourceStatusPill(state: state)
                    }

                    VStack(alignment: .leading, spacing: 5) {
                        Text(title)
                            .font(.body.weight(.semibold))
                            .fixedSize(horizontal: false, vertical: true)
                        DataSourceStatusPill(state: state)
                    }
                }

                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)

                Label(policy.displayName, systemImage: policy.systemImageName)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(policy == .blocked ? Color.red : color)
                    .lineLimit(1)
                
                Text(state.detailText)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            
            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tertiary)
                .padding(.top, 8)
        }
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
        .accessibilityValue("\(state.statusText), \(policy.displayName), \(state.detailText)")
    }
}

struct DataSourceStatusPill: View {
    let state: DataSourceRuntimeState

    var body: some View {
        Label(state.statusText, systemImage: state.statusIcon)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(state.tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(state.tint.opacity(0.09), in: Capsule())
            .lineLimit(1)
    }
}

struct DataSourceRow: View {
    let icon: String
    let title: String
    let subtitle: String
    let isEnabled: Bool
    
    var body: some View {
        HStack {
            Image(systemName: icon)
                .foregroundColor(.blue)
                .frame(width: 30)
            
            VStack(alignment: .leading) {
                Text(title)
                    .font(.body)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            
            Spacer()
            
            if isEnabled {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.green)
            }
        }
    }
}

private struct DataSourceAccessGuide: View {
    @State private var isExpanded = false

    var body: some View {
        NolaDisclosure(isExpanded: $isExpanded) {
            VStack(alignment: .leading, spacing: 12) {
                Text(NSLocalizedString("data_source.policy.note", value: "Choose whether this source is used automatically, enabled per conversation, requested only when relevant, or blocked.", comment: "Data source policy explanation"))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                LazyVGrid(
                    columns: [
                        GridItem(.flexible(), alignment: .leading),
                        GridItem(.flexible(), alignment: .leading)
                    ],
                    spacing: 8
                ) {
                    accessPill(
                        NSLocalizedString("data_source.policy.default_enabled", value: "Every conversation", comment: "Default enabled data source policy"),
                        systemImage: "checkmark.circle.fill",
                        tint: DesignTokens.primaryColor
                    )
                    accessPill(
                        NSLocalizedString("data_source.policy.conversation_opt_in", value: "Per conversation", comment: "Conversation opt-in data source policy"),
                        systemImage: "bubble.left.and.bubble.right.fill",
                        tint: .teal
                    )
                    accessPill(
                        NSLocalizedString("data_source.policy.ask_when_relevant", value: "Ask when relevant", comment: "Ask when relevant data source policy"),
                        systemImage: "questionmark.bubble.fill",
                        tint: .orange
                    )
                    accessPill(
                        NSLocalizedString("data_source.policy.blocked", value: "Blocked", comment: "Blocked data source policy"),
                        systemImage: "nosign",
                        tint: .secondary
                    )
                }

                Divider()

                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "lock.shield.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.green)
                        .frame(width: 28, height: 28)
                        .background(Color.green.opacity(0.09), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    VStack(alignment: .leading, spacing: 3) {
                        Text(L10n.homeDataSourceLocalStorageLabel)
                            .font(.subheadline.weight(.semibold))
                        Text(L10n.homeDataSourcePrivacyDescription)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("data.center.access.privacy")
            }
            .padding(.top, 12)
        } label: {
            HStack(spacing: 11) {
                Image(systemName: "switch.2")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(DesignTokens.primaryColor)
                    .frame(width: 34, height: 34)
                    .background(
                        DesignTokens.primaryColor.opacity(0.09),
                        in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                    )
                VStack(alignment: .leading, spacing: 2) {
                    Text(NSLocalizedString("data_source.policy.section", value: "Context access", comment: "Data source policy section title"))
                        .font(.subheadline.weight(.semibold))
                    Text(NSLocalizedString("data_source.policy.note", value: "Choose whether this source is used automatically, enabled per conversation, requested only when relevant, or blocked.", comment: "Data source policy explanation"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("data.center.access.guide")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func accessPill(_ title: String, systemImage: String, tint: Color) -> some View {
        Label(title, systemImage: systemImage)
            .font(.caption.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(tint.opacity(0.09), in: Capsule())
    }
}

private struct InlineURLSourcesList: View {
    @EnvironmentObject var container: ServiceContainer
    @State private var editingSource: URLDataSource? = nil
    @State private var showingAdd: Bool = false
    @State private var saveConfirmation: String? = nil

    var body: some View {
        VStack(spacing: 0) {
            if let saveConfirmation {
                Button {
                    self.saveConfirmation = nil
                } label: {
                    HStack(spacing: 9) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(DesignTokens.successColor)
                        Text(saveConfirmation)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                        Spacer()
                        Image(systemName: "xmark")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 11)
                    .background(DesignTokens.successColor.opacity(0.09), in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                }
                .buttonStyle(.plain)
                .padding(.bottom, 10)
            }

            if let fetcher = container.urlFetcher {
                let sources = fetcher.sources
                if sources.isEmpty {
                    NolaInlineEmptyState(
                        icon: "tray.full",
                        title: NSLocalizedString("data_sources.inline.empty.title", value: "No readable sources yet", comment: "Inline data source empty title"),
                        message: NSLocalizedString("data_sources.inline.empty.message", value: "Add websites, search-result URLs, APIs, or feeds. Then you can inject them into chat context or use them in scheduled actions.", comment: "Inline data source empty message"),
                        tint: DesignTokens.primaryColor,
                        actionTitle: NSLocalizedString("data_sources.inline.empty.action", value: "Add source", comment: "Inline data source empty action")
                    ) {
                        showingAdd = true
                    }
                    .accessibilityIdentifier("data.center.empty")
                    .padding(.vertical, 4)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(sources.enumerated()), id: \.element.id) { idx, source in
                            Button {
                                editingSource = source
                            } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: source.kind.iconSystemName)
                                        .font(.system(size: 17, weight: .semibold))
                                        .foregroundStyle(DesignTokens.primaryColor)
                                        .frame(width: 24)
                                    VStack(alignment: .leading, spacing: 6) {
                                        Text(source.name)
                                            .font(.headline)
                                        Text(source.contextDisplaySubtitle)
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                            .lineLimit(1)
                                        HStack(spacing: 7) {
                                            Text(source.kind.displayName)
                                            Text("·")
                                            Text(source.accessPolicy.displayName)
                                            Text("·")
                                            Label(
                                                source.validationContract?.isUsable == true
                                                    ? NSLocalizedString("data_source.contract.verified", value: "Verified", comment: "Verified source contract")
                                                    : NSLocalizedString("data_source.contract.needs_validation", value: "Needs validation", comment: "Unverified source contract"),
                                                systemImage: source.validationContract?.isUsable == true ? "checkmark.seal.fill" : "exclamationmark.triangle.fill"
                                            )
                                            .foregroundColor(source.validationContract?.isUsable == true ? .green : .orange)
                                        }
                                        .font(.caption2.weight(.medium))
                                        .foregroundColor(.secondary)
                                        .lineLimit(1)
                                        if source.validationContract?.isUsable == true {
                                            InlineDataSourceContractSummary(source: source)
                                        }
                                    }
                                    Spacer()
                                    Image(systemName: "chevron.right")
                                        .foregroundColor(.secondary)
                                }
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityElement(children: .combine)
                            .accessibilityIdentifier("data.center.source.\(source.id.uuidString)")

                            if idx != sources.count - 1 {
                                NolaDivider(leading: 12)
                            }
                        }

                        NolaDivider(leading: 12)

                        Button {
                            showingAdd = true
                        } label: {
                            Label(
                                NSLocalizedString("data_sources.inline.empty.action", value: "Add source", comment: "Inline data source add action"),
                                systemImage: "plus.circle.fill"
                            )
                            .font(.subheadline.weight(.semibold))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 11)
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(DesignTokens.primaryColor)
                        .accessibilityIdentifier("data.center.add")
                    }
                }
            } else {
                NolaInlineEmptyState(
                    icon: "tray.full",
                    title: NSLocalizedString("data_sources.inline.unavailable.title", value: "Data source service unavailable", comment: "Inline data source unavailable title"),
                    message: NSLocalizedString("data_sources.inline.unavailable.message", value: "Cannot load custom data source configuration right now.", comment: "Inline data source unavailable message"),
                    tint: DesignTokens.primaryColor
                )
            }

        }
        .sheet(item: $editingSource, onDismiss: { }) { src in
            AddURLDataSourceSheet(existingSource: src) { saved in
                saveConfirmation = String(
                    format: NSLocalizedString("data_source.saved", value: "%@ is ready", comment: "Data source save confirmation"),
                    saved.name
                )
            }
                .environmentObject(container)
        }
        .sheet(isPresented: $showingAdd) {
            AddURLDataSourceSheet(existingSource: nil) { saved in
                saveConfirmation = String(
                    format: NSLocalizedString("data_source.saved", value: "%@ is ready", comment: "Data source save confirmation"),
                    saved.name
                )
            }
                .environmentObject(container)
        }
    }
}

private struct InlineDataSourceContractSummary: View {
    let source: URLDataSource

    private var validationTime: String {
        guard let date = source.validationContract?.validatedAt else { return "" }
        return RelativeDateTimeFormatter().localizedString(for: date, relativeTo: Date())
    }

    var body: some View {
        if let contract = source.validationContract {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 5) {
                    Text(NSLocalizedString("data_source.contract.reader", value: "Reader", comment: "Data source contract reader label"))
                        .foregroundStyle(.secondary)
                    Text(contract.readerDescription)
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                }
                HStack(spacing: 8) {
                    Text(
                        String(
                            format: NSLocalizedString("data_source.contract.format", value: "Format: %@", comment: "Data source contract format"),
                            source.format.rawValue.uppercased()
                        )
                    )
                    Text("·")
                    Text(
                        String(
                            format: NSLocalizedString("data_source.contract.records", value: "%d records", comment: "Data source contract record count"),
                            contract.recordCount
                        )
                    )
                    Text("·")
                    Text(
                        String(
                            format: NSLocalizedString("data_source.contract.validated", value: "Validated %@", comment: "Data source contract validation time"),
                            validationTime
                        )
                    )
                }
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            .font(.caption2)
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
    }
}

private struct DataSourceUsageTimeline: View {
    @ObservedObject var fetcher: URLFetcherService

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(fetcher.usageRecords.prefix(8).enumerated()), id: \.element.id) { index, record in
                HStack(alignment: .top, spacing: 11) {
                    Image(systemName: record.disclosedToCloud == true ? "cloud.fill" : "arrow.turn.down.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(record.disclosedToCloud == true ? Color.blue : Color.green)
                        .frame(width: 28, height: 28)
                        .background(
                            (record.disclosedToCloud == true ? Color.blue : Color.green).opacity(0.09),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )

                    VStack(alignment: .leading, spacing: 4) {
                        Text(record.sourceTitle)
                            .font(.subheadline.weight(.semibold))
                        HStack(spacing: 5) {
                            Text(usagePurpose(record.purpose))
                            Text("·")
                            Text(relativeTime(record.usedAt))
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        Text(disclosureText(record.disclosedToCloud))
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(record.disclosedToCloud == true ? Color.blue : Color.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 9)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("data.center.usage.\(record.sourceID.uuidString)")

                if index < min(fetcher.usageRecords.count, 8) - 1 {
                    NolaDivider(leading: 39)
                }
            }
        }
    }

    private func usagePurpose(_ purpose: String) -> String {
        switch purpose {
        case "conversation_context":
            return NSLocalizedString("data_sources.usage.purpose.conversation_context", value: "Conversation context", comment: "Data source usage purpose")
        case "scheduled_context_refresh":
            return NSLocalizedString("data_sources.usage.purpose.scheduled_refresh", value: "Scheduled refresh", comment: "Data source usage purpose")
        default:
            return NSLocalizedString("data_sources.usage.purpose.other", value: "Data access", comment: "Data source usage purpose")
        }
    }

    private func disclosureText(_ disclosed: Bool?) -> String {
        switch disclosed {
        case true:
            return NSLocalizedString("data_sources.usage.disclosure.cloud", value: "Shared with the configured cloud model", comment: "Data source cloud disclosure")
        case false:
            return NSLocalizedString("data_sources.usage.disclosure.local", value: "Stayed on this device", comment: "Data source local disclosure")
        case nil:
            return NSLocalizedString("data_sources.usage.disclosure.pending", value: "Model route recorded when the request is sent", comment: "Data source pending disclosure")
        }
    }

    private func relativeTime(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

struct HealthDataSourceRow: View {
    @ObservedObject var dashboard: HealthDashboardStore
    let runtimeState: DataSourceRuntimeState
    let policy: URLDataSourceAccessPolicy

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "heart.fill")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(.red)
                .frame(width: 24)

            VStack(alignment: .leading, spacing: 5) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(L10n.homeFeatureHealthTitle)
                            .font(.body.weight(.semibold))
                        Spacer(minLength: 6)
                        if dashboard.isLoading && dashboard.snapshot == nil {
                            ProgressView().controlSize(.small)
                        } else {
                            DataSourceStatusPill(state: runtimeState)
                        }
                    }
                    VStack(alignment: .leading, spacing: 5) {
                        Text(L10n.homeFeatureHealthTitle)
                            .font(.body.weight(.semibold))
                        if dashboard.isLoading && dashboard.snapshot == nil {
                            ProgressView().controlSize(.small)
                        } else {
                            DataSourceStatusPill(state: runtimeState)
                        }
                    }
                }
                Text(summaryText)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                Label(policy.displayName, systemImage: policy.systemImageName)
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(policy == .blocked ? Color.red : Color.red.opacity(0.88))
                    .lineLimit(1)
            }

            Image(systemName: "chevron.right")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.tertiary)
                .padding(.top, 8)
        }
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
        .accessibilityValue("\(runtimeState.statusText), \(policy.displayName), \(runtimeState.detailText)")
        .accessibilityIdentifier("data.center.health")
    }

    private var summaryText: String {
        guard [.ready, .stale].contains(runtimeState.phase),
              let snapshot = dashboard.snapshot,
              snapshot.hasReadableData else {
            return runtimeState.detailText
        }
        let format = NSLocalizedString(
            "health.dashboard.summary_format",
            value: "%1$@ steps · %2$@ kcal",
            comment: "Health source row summary"
        )
        return String(
            format: format,
            snapshot.todaySteps.formatted(.number.precision(.fractionLength(0))),
            snapshot.todayActiveEnergyKilocalories.formatted(.number.precision(.fractionLength(0)))
        )
    }

}

struct LockedFeatureRow: View {
    let title: String
    let icon: String
    let description: String
    
    var body: some View {
        HStack {
            Image(systemName: icon)
                .foregroundColor(.gray)
                .frame(width: 30)
            
            VStack(alignment: .leading) {
                HStack {
                    Text(title)
                    Image(systemName: "lock.fill")
                        .font(.caption)
                        .foregroundColor(.gray)
                }
                Text(description)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
    }
}
