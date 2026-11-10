// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSourceDetailView logic in the iOS UI layer.
// - Interfaces:
//   - DataSourceDetailView
//   - StatusCard
//   - StatView
//   - PermissionRow
//   - InfoRow
// - APIs:
//   - none
//
//
//  DataSourceDetailView.swift
//  Nola
//
// Comment in English.
//

import SwiftUI
import Contacts
import EventKit
import HealthKit

// Comment in English.

struct DataSourceDetailView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let sourceType: DataSourceType
    @Environment(\.dismiss) var dismiss
    
    @State private var isEnabled = false
    @State private var permissionStatus: AuthorizationStatus = .notDetermined
    @State private var isCheckingPermission = false
    @State private var showPermissionAlert = false
    @State private var eventCount: Int = 0
    @State private var lastFetchTime: Date?
    @State private var isRefreshing = false
    @State private var errorMessage: String?
    @State private var showError = false
    @State private var accessPolicy: URLDataSourceAccessPolicy
    @StateObject private var healthDashboard = HealthDashboardStore.shared

    init(sourceType: DataSourceType, healthDashboard: HealthDashboardStore? = nil) {
        self.sourceType = sourceType
        _accessPolicy = State(initialValue: SystemDataSourceAccessPolicyStore.shared.policy(for: sourceType))
        _healthDashboard = StateObject(wrappedValue: healthDashboard ?? HealthDashboardStore.shared)
    }
    
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                DataSourceDetailHero(sourceType: sourceType, state: runtimeState)

                Button {
                    performPrimaryAction()
                } label: {
                    HStack(spacing: 9) {
                        if isRefreshing || isCheckingPermission {
                            ProgressView().tint(.white)
                        } else {
                            Image(systemName: primaryActionIcon)
                                .contentTransition(.symbolEffect(.replace))
                        }
                        Text(primaryActionTitle)
                            .fontWeight(.semibold)
                            .contentTransition(.opacity)
                        Spacer()
                        if runtimeState.phase != .syncing && runtimeState.phase != .checking {
                            Image(systemName: "arrow.right")
                                .font(.caption.weight(.bold))
                                .opacity(0.72)
                        }
                    }
                    .animation(reduceMotion ? nil : DesignTokens.responsiveAnimation, value: runtimeState.phase)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 13)
                }
                .buttonStyle(.plain)
                .foregroundStyle(.white)
                .background(sourceType.color, in: RoundedRectangle(cornerRadius: 15, style: .continuous))
                .disabled(!primaryActionEnabled)
                .opacity(primaryActionEnabled ? 1 : 0.48)

                if sourceType == .health, runtimeState.phase == .ready || healthDashboard.snapshot != nil {
                    DataSourceDetailCard(
                        title: L10n.dataSourceDetailSectionHealthOverview,
                        icon: "heart.text.square.fill",
                        tint: .red
                    ) {
                        HealthDashboardView(store: healthDashboard)
                    }
                }

                DataSourceDetailCard(
                    title: NSLocalizedString("data_sources.runtime.detail.access_title", value: "Access", comment: "Data source access card title"),
                    icon: "hand.raised.fill",
                    tint: sourceType.color
                ) {
                    InfoRow(label: L10n.dataSourceDetailStatPermissionStatus, value: permissionStatus.displayText)
                    Divider()
                    InfoRow(
                        label: NSLocalizedString("data_sources.runtime.detail.last_success", value: "Last successful read", comment: "Last successful read label"),
                        value: lastFetchTime.map(timeAgoString) ?? NSLocalizedString("data_sources.runtime.never_read", value: "No successful read yet", comment: "No successful read")
                    )
                    Divider()
                    InfoRow(label: L10n.dataSourceDetailInfoEventCount, value: lastFetchTime == nil ? "—" : "\(eventCount)")
                }

                if sourceType != .url {
                    DataSourceDetailCard(
                        title: NSLocalizedString("data_source.policy.section", value: "Context access", comment: "Data source policy section title"),
                        icon: "switch.2",
                        tint: .teal
                    ) {
                        HStack(spacing: 12) {
                            Label(
                                NSLocalizedString("data_source.policy.title", value: "Default access", comment: "Data source policy picker title"),
                                systemImage: accessPolicy.systemImageName
                            )
                            .font(.subheadline.weight(.semibold))
                            Spacer(minLength: 8)
                            Picker(NSLocalizedString("data_source.policy.title", value: "Default access", comment: "Data source policy picker title"), selection: $accessPolicy) {
                                ForEach(URLDataSourceAccessPolicy.allCases) { policy in
                                    Text(policy.displayName).tag(policy)
                                }
                            }
                            .labelsHidden()
                            .pickerStyle(.menu)
                            .accessibilityIdentifier("data.source.access.policy")
                        }
                        .onChange(of: accessPolicy) { _, newValue in
                            SystemDataSourceAccessPolicyStore.shared.setPolicy(newValue, for: sourceType)
                        }

                        Text(accessPolicy.summaryText)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("data.source.policy.summary")

                        Label(
                            NSLocalizedString("data_source.policy.saved_automatically", value: "Changes are saved automatically for future conversations.", comment: "Data source policy automatic save note"),
                            systemImage: "checkmark.circle"
                        )
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)
                    }
                }

                DataSourceDetailCard(
                    title: NSLocalizedString("data_sources.runtime.detail.contributes_title", value: "What Nola can use", comment: "Data source contribution card title"),
                    icon: "text.viewfinder",
                    tint: .blue
                ) {
                    Text(sourceType.supportedEvents)
                        .font(.subheadline.weight(.medium))
                    Text(sourceType.usageDescription)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if let error = errorMessage {
                    DataSourceDetailCard(
                        title: L10n.dataSourceDetailSectionError,
                        icon: "exclamationmark.triangle.fill",
                        tint: .red
                    ) {
                        Text(error)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 28)
        }
        .background(Color(.systemGroupedBackground))
        .accessibilityIdentifier("data.source.detail.\(sourceType.rawValue)")
        .navigationTitle(sourceType.displayName)
        .navigationBarTitleDisplayMode(.inline)
        .task {
            await checkInitialState()
        }
        .onChange(of: scenePhase) { _, newPhase in
            guard newPhase == .active else { return }
            Task {
                if sourceType == .health {
                    await container.healthDataSource?.invalidateSyncCache()
                }
                await checkInitialState()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaHealthDataDidChange)) { _ in
            guard sourceType == .health else { return }
            Task { await fetchHealthDashboard() }
        }
        .refreshable {
            await refreshData()
        }
        .alert(Text(L10n.dataSourceDetailPermissionAlertTitle), isPresented: $showPermissionAlert) {
            Button(L10n.cancel, role: .cancel) {}
            Button(L10n.permissionGoToSettings) {
                openSettings()
            }
        } message: {
            Text(L10n.format("data_source_detail.permission_alert.message_format", sourceType.displayName))
        }
        .alert(Text(L10n.dataSourceDetailErrorAlertTitle), isPresented: $showError) {
            Button(L10n.ok, role: .cancel) {}
        } message: {
            Text(errorMessage ?? L10n.dataSourceDetailUnknownError)
        }
    }

    private var runtimeState: DataSourceRuntimeState {
        if isRefreshing || isCheckingPermission {
            return DataSourceRuntimeState(
                phase: .syncing,
                permission: permissionStatus,
                itemCount: lastFetchTime == nil ? nil : eventCount,
                lastReadAt: lastFetchTime,
                errorMessage: nil
            )
        }
        if let errorMessage {
            return DataSourceRuntimeState(
                phase: .error,
                permission: permissionStatus,
                itemCount: lastFetchTime == nil ? nil : eventCount,
                lastReadAt: lastFetchTime,
                errorMessage: errorMessage
            )
        }
        if sourceType == .health, !isEnabled, permissionStatus == .authorized {
            return DataSourceRuntimeState(phase: .disabled, permission: permissionStatus, itemCount: nil, lastReadAt: lastFetchTime, errorMessage: nil)
        }
        switch permissionStatus {
        case .notDetermined, .denied, .insufficient:
            return DataSourceRuntimeState(phase: .needsPermission, permission: permissionStatus, itemCount: nil, lastReadAt: lastFetchTime, errorMessage: nil)
        case .restricted, .unavailable:
            return DataSourceRuntimeState(phase: .unavailable, permission: permissionStatus, itemCount: nil, lastReadAt: lastFetchTime, errorMessage: nil)
        case .authorized:
            let phase: DataSourceRuntimePhase
            if let lastFetchTime, Date().timeIntervalSince(lastFetchTime) > 21_600 {
                phase = .stale
            } else {
                phase = .ready
            }
            return DataSourceRuntimeState(
                phase: phase,
                permission: permissionStatus,
                itemCount: lastFetchTime == nil ? nil : eventCount,
                lastReadAt: lastFetchTime,
                errorMessage: nil
            )
        }
    }

    private var primaryActionTitle: String {
        switch runtimeState.phase {
        case .needsPermission:
            return permissionStatus == .notDetermined
                ? NSLocalizedString("data_sources.runtime.action.allow", value: "Allow access", comment: "Allow data source access action")
                : L10n.dataSourceDetailOpenSystemSettings
        case .error:
            return NSLocalizedString("data_sources.runtime.action.retry", value: "Try again", comment: "Retry data source action")
        case .checking, .syncing:
            return L10n.dataSourceDetailRefreshing
        case .disabled:
            return NSLocalizedString("data_sources.runtime.action.turn_on", value: "Turn on and refresh", comment: "Enable data source action")
        case .ready, .stale:
            return L10n.dataSourceDetailRefreshNow
        case .unavailable:
            return runtimeState.statusText
        }
    }

    private var primaryActionIcon: String {
        switch runtimeState.phase {
        case .needsPermission: return permissionStatus == .notDetermined ? "hand.raised.fill" : "gearshape.fill"
        case .error: return "arrow.clockwise"
        case .disabled: return "power"
        case .ready, .stale: return "arrow.clockwise"
        case .checking, .syncing: return "arrow.triangle.2.circlepath"
        case .unavailable: return "slash.circle"
        }
    }

    private var primaryActionEnabled: Bool {
        !isRefreshing && !isCheckingPermission && runtimeState.phase != .unavailable
    }

    private func performPrimaryAction() {
        Task {
            switch runtimeState.phase {
            case .needsPermission:
                if permissionStatus == .notDetermined {
                    await requestPermission()
                } else {
                    openSettings()
                }
            case .disabled:
                isEnabled = true
                updateDataSourceState(true)
                await refreshData()
            case .ready, .stale, .error:
                await refreshData()
            case .checking, .syncing, .unavailable:
                break
            }
        }
    }
    
    // MARK: - Methods
    
    private func checkInitialState() async {
        guard !isCheckingPermission else { return }
        isCheckingPermission = true
        
        // Check permission status
        permissionStatus = await checkPermissionStatus()
        
        // Check if the source is enabled
        switch sourceType {
        case .health:
            isEnabled = container.healthDataSourceEnabled
        default:
            isEnabled = true // Other source types are enabled by default
        }
        
        // HealthKit read authorization is opaque. Direct reads are the source
        // of truth and must not be blocked by a synthetic permission state.
        if sourceType == .health, permissionStatus != .unavailable {
            await fetchHealthDashboard()
        } else if permissionStatus == .authorized {
            await fetchEventCount()
        }
        
        isCheckingPermission = false
    }
    
    private func checkPermissionStatus() async -> AuthorizationStatus {
        switch sourceType {
        case .contacts:
            let status = CNContactStore.authorizationStatus(for: .contacts)
            return AuthorizationStatus(from: status)
            
        case .calendars:
            let status = EKEventStore.authorizationStatus(for: .event)
            return AuthorizationStatus(from: status)
            
        case .reminders:
            let status = EKEventStore.authorizationStatus(for: .reminder)
            return AuthorizationStatus(from: status)
            
        case .health:
            let healthSource = container.healthDataSource ?? HealthDataSource()
            switch await healthSource.currentReadAuthorizationState() {
            case .authorized:
                return .authorized
            case .denied:
                return .denied
            case .unavailable:
                return .unavailable
            case .notDetermined:
                return .notDetermined
            }
            
        case .url:
            // Comment in English.
            return .authorized
        }
    }
    
    private func requestPermission() async {
        isCheckingPermission = true
        errorMessage = nil
        
        do {
            switch sourceType {
            case .contacts:
                let store = CNContactStore()
                let granted = try await store.requestAccess(for: .contacts)
                permissionStatus = granted ? .authorized : .denied
                
            case .calendars:
                let store = EKEventStore()
                let granted: Bool
                if #available(iOS 17.0, *) {
                    granted = try await store.requestFullAccessToEvents()
                } else {
                    granted = try await store.requestAccess(to: .event)
                }
                permissionStatus = granted ? .authorized : .denied
                
            case .reminders:
                let store = EKEventStore()
                let granted: Bool
                if #available(iOS 17.0, *) {
                    granted = try await store.requestFullAccessToReminders()
                } else {
                    granted = try await store.requestAccess(to: .reminder)
                }
                permissionStatus = granted ? .authorized : .denied
                
            case .health:
                let granted = await container.requestHealthAuthorization()
                permissionStatus = granted ? .authorized : .denied
                if granted {
                    container.healthDataSourceEnabled = true
                    isEnabled = true
                }
                
            case .url:
                permissionStatus = .authorized
            }
            
            if permissionStatus == .authorized {
                if sourceType == .health {
                    await fetchHealthDashboard()
                } else {
                    await fetchEventCount()
                }
            } else if permissionStatus == .denied {
                showPermissionAlert = true
            }
            
        } catch {
            errorMessage = error.localizedDescription
            showError = true
        }
        
        isCheckingPermission = false
    }
    
    private func fetchEventCount() async {
        let now = Date()
        let range = sourceType == .health
            ? DateInterval(start: Calendar.current.date(byAdding: .day, value: -7, to: now) ?? now, end: now)
            : DateInterval(start: now, end: Calendar.current.date(byAdding: .day, value: 30, to: now) ?? now)
        
        do {
            let events: [RawEvent]
            
            switch sourceType {
            case .contacts:
                let source = ContactsDataSource()
                events = try await source.fetchEvents(in: range)
                
            case .calendars:
                let source = CalendarsDataSource()
                events = try await source.fetchEvents(in: range)
                
            case .reminders:
                let source = RemindersDataSource()
                events = try await source.fetchEvents(in: range)
                
            case .health:
                if let healthSource = container.healthDataSource {
                    events = try await healthSource.fetchEvents(in: range)
                } else {
                    events = []
                }
                
            case .url:
                if let fetcher = container.urlFetcher {
                    let sourcesSnapshot = fetcher.sources.filter { $0.enabled }
                    let all: [RawEvent] = await withTaskGroup(of: [RawEvent].self) { group in
                        for src in sourcesSnapshot {
                            group.addTask {
                                (try? await fetcher.fetchEvents(from: src)) ?? []
                            }
                        }

                        var collected: [RawEvent] = []
                        for await evs in group {
                            collected.append(contentsOf: evs)
                        }
                        return collected
                    }
                    events = all
                } else {
                    events = []
                }
            }
            
            await MainActor.run {
                eventCount = events.count
                lastFetchTime = Date()
            }
            
        } catch {
            errorMessage = L10n.format("data_source_detail.error.fetch_failed_format", error.localizedDescription)
        }
    }
    
    private func fetchHealthDashboard() async {
        await healthDashboard.load()
        eventCount = healthDashboard.snapshot?.availableMetricCount ?? 0
        lastFetchTime = healthDashboard.snapshot?.generatedAt
        errorMessage = healthDashboard.errorMessage
        if healthDashboard.snapshot != nil, healthDashboard.errorMessage == nil {
            permissionStatus = .authorized
            isEnabled = true
            container.healthDataSourceEnabled = true
        }
    }
    
    private func refreshData() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        errorMessage = nil

        if sourceType == .health {
            await container.healthDataSource?.invalidateSyncCache()
            await fetchHealthDashboard()
        } else {
            await fetchEventCount()
        }
        
        isRefreshing = false
    }
    
    private func updateDataSourceState(_ enabled: Bool) {
        switch sourceType {
        case .health:
            container.healthDataSourceEnabled = enabled
        default:
            break
        }
    }
    
    private func openSettings() {
        if let url = URL(string: UIApplication.openSettingsURLString) {
            UIApplication.shared.open(url)
        }
    }
    
    private func timeAgoString(from date: Date) -> String {
        let interval = Date().timeIntervalSince(date)
        
        if interval < 60 {
            return L10n.relativeJustNow
        } else if interval < 3600 {
            return L10n.format("relative.minutes_ago", Int(interval / 60))
        } else if interval < 86400 {
            return L10n.format("relative.hours_ago", Int(interval / 3600))
        } else {
            return L10n.format("relative.days_ago", Int(interval / 86400))
        }
    }
}

private struct DataSourceDetailHero: View {
    let sourceType: DataSourceType
    let state: DataSourceRuntimeState

    var body: some View {
        VStack(alignment: .leading, spacing: 15) {
            HStack(alignment: .top, spacing: 13) {
                Image(systemName: sourceType.iconSystemName)
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(sourceType.color)
                    .frame(width: 46, height: 46)
                    .background(sourceType.color.opacity(0.10), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                VStack(alignment: .leading, spacing: 5) {
                    Text(sourceType.displayName)
                        .font(.system(.title3, design: .rounded, weight: .bold))
                    DataSourceStatusPill(state: state)
                }
                Spacer(minLength: 0)
            }

            Text(state.detailText)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(17)
        .background(DesignTokens.elevatedColor, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        }
    }
}

private struct DataSourceDetailCard<Content: View>: View {
    let title: String
    let icon: String
    let tint: Color
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 13) {
            Label(title, systemImage: icon)
                .font(.headline)
                .foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 11) {
                content
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(DesignTokens.elevatedColor, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        }
    }
}

// MARK: - Supporting Views

struct StatusCard: View {
    let isEnabled: Bool
    let permissionStatus: AuthorizationStatus
    let eventCount: Int
    let lastFetchTime: Date?
    
    var body: some View {
        VStack(spacing: 16) {
            // Comment in English.
            HStack(spacing: 12) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 12, height: 12)
                
                Text(statusText)
                    .font(.headline)
                    .foregroundColor(statusColor)
                
                Spacer()
            }
            
            // Comment in English.
            HStack(spacing: 24) {
                StatView(label: L10n.dataSourceDetailStatPermissionStatus, value: permissionStatus.displayText, color: permissionStatus.uiColor)
                StatView(label: L10n.dataSourceDetailStatSource, value: isEnabled ? L10n.dataSourceDetailStatEnabledValue : L10n.dataSourceDetailStatDisabledValue, color: isEnabled ? .green : .gray)
                if eventCount > 0 {
                    StatView(label: L10n.dataSourceDetailStatEventCount, value: "\(eventCount)", color: .blue)
                }
            }
            
            if let time = lastFetchTime {
                HStack {
                    Image(systemName: "clock.fill")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                    Text(L10n.format("data_source_detail.last_updated_prefix_format", timeAgoString(from: time)))
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
        }
        .padding()
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color(.secondarySystemBackground))
        )
    }
    
    private var statusColor: Color {
        if !isEnabled {
            return .gray
        } else if permissionStatus == .authorized {
            return .green
        } else if permissionStatus == .insufficient {
            return .orange
        } else if permissionStatus == .denied {
            return .red
        } else {
            return .orange
        }
    }
    
    private var statusText: String {
        if !isEnabled {
            return L10n.dataSourceDetailStatusDisabled
        } else if permissionStatus == .authorized {
            return L10n.dataSourceDetailStatusOK
        } else if permissionStatus == .insufficient {
            return L10n.dataSourceDetailStatusInsufficient
        } else if permissionStatus == .denied {
            return L10n.dataSourceDetailStatusDenied
        } else {
            return L10n.dataSourceDetailStatusNeedsAuth
        }
    }
    
    private func timeAgoString(from date: Date) -> String {
        let interval = Date().timeIntervalSince(date)
        if interval < 60 { return L10n.relativeJustNow }
        else if interval < 3600 { return L10n.format("relative.minutes_ago", Int(interval / 60)) }
        else if interval < 86400 { return L10n.format("relative.hours_ago", Int(interval / 3600)) }
        else { return L10n.format("relative.days_ago", Int(interval / 86400)) }
    }
}

struct StatView: View {
    let label: String
    let value: String
    let color: Color
    
    var body: some View {
        VStack(spacing: 4) {
            Text(value)
                .font(.headline)
                .foregroundColor(color)
            Text(label)
                .font(.caption2)
                .foregroundColor(.secondary)
        }
    }
}

struct PermissionRow: View {
    let status: AuthorizationStatus
    let onRequestPermission: () -> Void
    
    var body: some View {
        HStack {
            Image(systemName: status.systemImage)
                .foregroundColor(status.uiColor)
            
            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.dataSourceDetailPermissionAccessTitle)
                    .font(.body)
                Text(status.displayText)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            
            Spacer()
            
            if status == .notDetermined {
                Button(L10n.dataSourceDetailPermissionRequestButton) {
                    onRequestPermission()
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }
        }
    }
}

struct InfoRow: View {
    let label: String
    let value: String
    
    var body: some View {
        HStack {
            Text(label)
                .foregroundColor(.secondary)
            Spacer()
            Text(value)
                .foregroundColor(.primary)
        }
    }
}

#Preview {
    NavigationStack {
        DataSourceDetailView(sourceType: .health)
            .environmentObject(ServiceContainer())
    }
}
