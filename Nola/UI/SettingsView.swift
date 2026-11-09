// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements SettingsView logic in the iOS UI layer.
// - Interfaces:
//   - SettingsView
//   - ShareSheet
// - APIs:
//   - none
//
import SwiftUI
import UserNotifications
import UIKit

struct SettingsView: View {
    var ownsNavigationStack = true
    @State private var showsAgentManagement = false
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var showingClearDataAlert = false
    @State private var pageWidth: CGFloat = 390
    @State private var showingExportSheet = false
    @State private var exportedFileURL: URL?
    @State private var isExporting = false
    @State private var isClearingData = false
    @State private var localDataSize = "—"
    @State private var operationMessage: String?
    @State private var operationIsError = false
    @State private var notificationAuthorizationStatus: UNAuthorizationStatus = .notDetermined
    @State private var isUpdatingNotificationAuthorization = false
    @State private var shouldEnableNotificationsAfterSettings = false

    @AppStorage("enableNotifications") private var enableNotifications: Bool = true
    @AppStorage("enableStepReminders") private var enableStepReminders: Bool = true
    @AppStorage("aggregateCalendarNotifications") private var aggregateCalendarNotifications: Bool = true
    @AppStorage("notificationFrequency") private var notificationFrequency: String = "daily"

    var body: some View {
        Group {
            if ownsNavigationStack { NavigationStack { content } }
            else { content }
        }
        .task {
            refreshLocalDataSize()
            await refreshNotificationAuthorization()
        }
        .onChange(of: enableNotifications) { _, enabled in
            Task { await updateNotificationPreference(enabled: enabled) }
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            Task { await refreshNotificationAuthorization() }
        }
    }

    private enum Category: String, CaseIterable {
        case providers, agents, agent, appearance, notifications, privacy, memory, plans, updates
        var titleKey: String {
            switch self {
            case .updates: "softwareUpdate.title"
            case .agents: "experts.manage"
            case .agent: "personal.capabilities.title"
            default: "settings.navigation." + rawValue
            }
        }
        var detailKey: String { self == .updates ? "softwareUpdate.check" : titleKey + ".detail" }
    }

    private var categoryGroups: [[Category]] {
        [[.providers, .agents, .agent], [.appearance, .notifications, .updates, .privacy], [.memory, .plans]]
    }

    private var content: some View {
        settingsPage(title: L10n.settings, showsHeading: false) {
            if ownsNavigationStack {
                Text(L10n.settings).font(.headline).frame(maxWidth: .infinity).frame(minHeight: 44)
            }
            ForEach(categoryGroups.indices, id: \.self) { group in
                VStack(spacing: 0) {
                    ForEach(categoryGroups[group], id: \.rawValue) { category in
                        Group {
                            if category == .agents {
                                Button { showsAgentManagement = true } label: { categoryLabel(category) }
                            } else {
                                NavigationLink { destination(category).toolbar(.visible, for: .navigationBar) } label: { categoryLabel(category) }
                            }
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier(category == .providers ? "settings.models" : "settings.category." + category.rawValue)
                        if category != categoryGroups[group].last {
                            Rectangle().fill(DesignTokens.borderColor).frame(height: 0.5).padding(.leading, 48).accessibilityHidden(true)
                        }
                    }
                }.background(NolaSurfaceBackground(fill: DesignTokens.surfaceColor, border: DesignTokens.borderColor, cornerRadius: 14))
            }
        }
        .sheet(isPresented: $showsAgentManagement) { IOSAgentRoomsView().environmentObject(container) }
        .navigationTitle(L10n.settings)
        .toolbar(ownsNavigationStack ? .hidden : .visible, for: .navigationBar)
    }

    private func categoryLabel(_ category: Category) -> some View {
        HStack(spacing: 12) {
            Image(categoryIcon(category)).renderingMode(.template).resizable().scaledToFit()
                .frame(width: 22, height: 22).foregroundStyle(DesignTokens.actionColor)
            VStack(alignment: .leading, spacing: 5) {
                Text(LocalizedStringKey(category.titleKey)).font(.body.weight(.medium)).foregroundStyle(.primary)
                if category != .agents {
                    Group {
                        if category == .notifications { Text(notificationStatusText) }
                        else if category == .agent { Text("personal.capabilities.subtitle") }
                        else { Text(LocalizedStringKey(category.detailKey)) }
                    }.font(.caption).foregroundStyle(.secondary)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
            Image("nola-chevron").renderingMode(.template).resizable().scaledToFit()
                .frame(width: 14, height: 14).foregroundStyle(.secondary).accessibilityHidden(true)
        }.frame(minHeight: 62).padding(.horizontal, 14).padding(.vertical, 8).contentShape(Rectangle())
    }

    private func categoryIcon(_ category: Category) -> String {
        switch category {
        case .providers: "nola-models"
        case .agents: "nola-person"
        case .agent: "nola-execution"
        case .appearance: "nola-sun"
        case .notifications: "nola-bell"
        case .privacy: "nola-review"
        case .memory: "nola-person"
        case .plans: "nola-calendar"
        case .updates: "nola-settings"
        }
    }

    @ViewBuilder private func destination(_ category: Category) -> some View {
        switch category {
        case .updates:
            settingsPage(title: NSLocalizedString(category.titleKey, comment: "Software update"), showsHeading: false) { AppleSoftwareUpdateSection() }
                .navigationTitle(LocalizedStringKey(category.titleKey)).navigationBarTitleDisplayMode(.inline)
        case .appearance:
            settingsPage(title: NSLocalizedString(category.titleKey, comment: "Appearance settings"), showsHeading: false) {
                HStack {
                    Text("settings.appearance.theme").font(.body.weight(.medium))
                    Spacer(minLength: 12)
                    AppleAppearancePicker().labelsHidden().pickerStyle(.menu)
                }.frame(minHeight: 52)
                let motionKey = reduceMotion ? "settings.appearance.motionReduced" : "settings.appearance.motionStandard"
                Text(LocalizedStringKey(motionKey))
                    .font(.footnote).foregroundStyle(.secondary)
            }.navigationTitle(LocalizedStringKey(category.titleKey)).navigationBarTitleDisplayMode(.inline)
        case .notifications:
            settingsPage(title: NSLocalizedString(category.titleKey, comment: "Notification settings"), showsHeading: false) { notificationControls }
                .navigationTitle(LocalizedStringKey(category.titleKey)).navigationBarTitleDisplayMode(.inline)
        case .providers:
            ModelsView(ownsNavigationStack: false).environmentObject(container)
        case .agents:
            IOSAgentRoomsView().environmentObject(container)
        case .agent:
            PersonalCapabilitiesLibraryView().environmentObject(container)
        case .memory:
            PersonalIntelligenceDashboardView(embedded: true).environmentObject(container)
        case .privacy:
            privacyPage
        case .plans:
            ScheduledActionsView(embedded: true).environmentObject(container)
        }
    }

    private func settingsPage<Content: View>(title: String, showsHeading: Bool = true, @ViewBuilder content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if showsHeading { IOSWorkspacePageHeading(title: title) }
                content()
            }
            .padding(.horizontal, pageWidth < 359 ? 16 : 20)
            .padding(.top, DesignTokens.spacing_lg)
            .padding(.bottom, DesignTokens.pageBottomInset)
            .frame(maxWidth: DesignTokens.pageMaxWidth)
            .frame(maxWidth: .infinity)
        }
        .background(NolaPageBackground())
        .background {
            GeometryReader { geometry in
                Color.clear.onChange(of: geometry.size.width, initial: true) { _, width in
                    if width > 0 { pageWidth = width }
                }
            }
        }
    }

    private var notificationControls: some View {
        NolaSettingsCard {
            NolaSettingsToggleRow(
                icon: "bell.badge",
                tint: .red,
                title: L10n.settingsEnableNotifications,
                detail: notificationStatusText,
                isBusy: isUpdatingNotificationAuthorization,
                isOn: $enableNotifications
            )

            if notificationAuthorizationStatus == .denied {
                NolaSettingsDivider()
                Button { openApplicationSettings() } label: {
                    NolaSettingsLinkLabel(
                        icon: "gearshape.fill",
                        tint: .red,
                        title: NSLocalizedString("permission.go_to_settings", value: "Open Settings", comment: "Open system settings"),
                        detail: NSLocalizedString("settings.notifications.status.denied", value: "Blocked in iOS Settings", comment: "Notifications denied")
                    )
                }
                .buttonStyle(.plain)
            }

            Group {
                NolaSettingsDivider()
                NolaSettingsToggleRow(
                    icon: "figure.walk",
                    tint: .green,
                    title: L10n.settingsEnableStepReminders,
                    isOn: $enableStepReminders
                )
                NolaSettingsDivider()
                NolaSettingsToggleRow(
                    icon: "calendar",
                    tint: DesignTokens.actionColor,
                    title: L10n.settingsCalendarAggregateNotificationsTitle,
                    isOn: $aggregateCalendarNotifications
                )
                NolaSettingsDivider()
                NolaSettingsPickerRow(
                    icon: "clock",
                    tint: .indigo,
                    title: NSLocalizedString("settings.notifications.frequency", value: "Notification Frequency", comment: "Notification frequency setting"),
                    selection: $notificationFrequency
                )
            }
            .disabled(!notificationsAreUsable)
            .opacity(notificationsAreUsable ? 1 : 0.64)
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: notificationsAreUsable)
    }

    private var privacyPage: some View {
        settingsPage(title: NSLocalizedString("settings.navigation.privacy", comment: "Privacy and data settings"), showsHeading: false) {
            if let operationMessage, !operationMessage.isEmpty {
                NolaSettingsStatusBanner(
                    message: operationMessage,
                    isError: operationIsError,
                    onDismiss: {
                        withAnimation(reduceMotion ? nil : DesignTokens.responsiveAnimation) {
                            self.operationMessage = nil
                        }
                    }
                )
                .transition(.move(edge: .top).combined(with: .opacity))
            }
            settingsSectionTitle(L10n.settingsSectionDataManagement)
            NolaSettingsCard {
                NolaSettingsValueRow(
                    icon: "internaldrive",
                    tint: .gray,
                    title: L10n.settingsLocalDataSize,
                    value: localDataSize
                )
                NolaSettingsDivider()
                Button { beginExport() } label: {
                    NolaSettingsLinkLabel(
                        icon: isExporting ? "arrow.triangle.2.circlepath" : "square.and.arrow.up",
                        tint: DesignTokens.actionColor,
                        title: isExporting
                            ? NSLocalizedString("settings.data.exporting", value: "Preparing export…", comment: "Export in progress")
                            : L10n.settingsExportData,
                        detail: NSLocalizedString("settings.data.export.detail", value: "Create a named, portable copy of your Nola data", comment: "Export data detail"),
                        trailingIcon: nil,
                        isBusy: isExporting
                    )
                }
                .buttonStyle(.plain)
                .disabled(isExporting || isClearingData)
            }
            settingsSectionTitle(NSLocalizedString("settings.section.privacy", comment: "Privacy section"))
            NolaSettingsCard {
                NavigationLink {
                    PrivacyDisclosureView()
                        .toolbar(.visible, for: .navigationBar)
                } label: {
                    NolaSettingsLinkLabel(
                        icon: "hand.raised.fill",
                        tint: DesignTokens.actionColor,
                        title: NSLocalizedString("privacy.disclosure.nav.title", value: "Privacy", comment: "Privacy disclosure title"),
                        detail: NSLocalizedString("settings.privacy.detail", value: "See what stays on this device and when data leaves it", comment: "Privacy settings detail")
                    )
                }
                .buttonStyle(.plain)
            }
            settingsSectionTitle(NSLocalizedString("settings.section.reset", comment: "Reset section"))
            NolaSettingsCard(tint: .red) {
                Button(role: .destructive) {
                    showingClearDataAlert = true
                } label: {
                    NolaSettingsActionLabel(
                        icon: "trash",
                        title: isClearingData
                            ? NSLocalizedString("settings.data.clearing", value: "Clearing data…", comment: "Clear data in progress")
                            : L10n.settingsClearAllData,
                        detail: NSLocalizedString("settings.data.clear.detail", value: "Remove conversations, memories, and connected data", comment: "Clear data detail")
                    )
                }
                .buttonStyle(.plain)
                .disabled(isExporting || isClearingData)
            }
        }
        .navigationTitle(LocalizedStringKey("settings.navigation.privacy"))
        .navigationBarTitleDisplayMode(.inline)
        .alert(L10n.settingsClearAllData, isPresented: $showingClearDataAlert) {
            Button(L10n.cancel, role: .cancel) {}
            Button(L10n.settingsClearAllData, role: .destructive) { beginClearAllData() }
        } message: {
            Text(L10n.settingsClearAllDataWarning)
        }
        .sheet(isPresented: $showingExportSheet, onDismiss: finishExportPresentation) {
            if let exportedFileURL {
                ShareSheet(items: [exportedFileURL])
            }
        }
    }

    private var notificationsAreUsable: Bool {
        enableNotifications && (
            notificationAuthorizationStatus == .authorized
                || notificationAuthorizationStatus == .provisional
                || notificationAuthorizationStatus == .ephemeral
        )
    }

    private var notificationStatusText: String {
        if isUpdatingNotificationAuthorization {
            return NSLocalizedString("settings.notifications.status.updating", value: "Checking permission…", comment: "Notification permission update state")
        }
        if notificationAuthorizationStatus == .denied {
            return NSLocalizedString("settings.notifications.status.denied", value: "Blocked in iOS Settings", comment: "Notifications denied")
        }
        guard enableNotifications else {
            return NSLocalizedString("settings.notifications.status.disabled", value: "Off in Nola", comment: "Notifications disabled in app")
        }
        switch notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral:
            return NSLocalizedString("settings.notifications.status.ready", value: "Allowed by iOS", comment: "Notifications authorized")
        case .denied:
            return NSLocalizedString("settings.notifications.status.denied", value: "Blocked in iOS Settings", comment: "Notifications denied")
        case .notDetermined:
            return NSLocalizedString("settings.notifications.status.not_determined", value: "Permission not requested", comment: "Notification permission not determined")
        @unknown default:
            return NSLocalizedString("settings.notifications.status.unavailable", value: "Permission unavailable", comment: "Notification permission unavailable")
        }
    }

    private func settingsSectionTitle(_ title: String) -> some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .textCase(.uppercase)
            .tracking(0.5)
            .padding(.horizontal, 4)
            .padding(.bottom, -12)
    }

    private func beginExport() {
        guard !isExporting, !isClearingData else { return }
        operationMessage = nil
        operationIsError = false
        isExporting = true
        Task { @MainActor in
            defer { isExporting = false }
            await Task.yield()
            do {
                guard let data = container.dataManager.exportData() else {
                    throw CocoaError(.fileWriteUnknown)
                }
                let directory = FileManager.default.temporaryDirectory
                let formatter = DateFormatter()
                formatter.locale = Locale(identifier: "en_US_POSIX")
                formatter.dateFormat = "yyyy-MM-dd-HHmmss"
                let url = directory.appendingPathComponent("Nola-Export-\(formatter.string(from: Date())).json")
                try data.write(to: url, options: .atomic)
                exportedFileURL = url
                showingExportSheet = true
                refreshLocalDataSize()
            } catch {
                operationIsError = true
                operationMessage = String(
                    format: NSLocalizedString("settings.data.export_failed", value: "Could not create the export: %@", comment: "Export failed"),
                    error.localizedDescription
                )
            }
        }
    }

    @MainActor
    private func refreshNotificationAuthorization() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        notificationAuthorizationStatus = settings.authorizationStatus
        let isAuthorized = settings.authorizationStatus == .authorized
            || settings.authorizationStatus == .provisional
            || settings.authorizationStatus == .ephemeral
        if shouldEnableNotificationsAfterSettings, isAuthorized {
            shouldEnableNotificationsAfterSettings = false
            enableNotifications = true
        } else if settings.authorizationStatus == .denied
                    || settings.authorizationStatus == .notDetermined {
            enableNotifications = false
        }
    }

    @MainActor
    private func updateNotificationPreference(enabled: Bool) async {
        guard enabled else { return }
        guard !isUpdatingNotificationAuthorization else { return }
        guard !notificationsAreUsable else { return }
        operationMessage = nil
        operationIsError = false

        if SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: enabled,
            authorizationStatus: notificationAuthorizationStatus
        ) {
            shouldEnableNotificationsAfterSettings = true
            enableNotifications = false
            openApplicationSettings()
            return
        }

        isUpdatingNotificationAuthorization = true
        defer { isUpdatingNotificationAuthorization = false }

        do {
            let granted = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
            await refreshNotificationAuthorization()
            if !granted {
                enableNotifications = false
                operationIsError = true
                operationMessage = NSLocalizedString(
                    "settings.notifications.permission_denied",
                    value: "Notifications are blocked by iOS. You can allow them later in Settings.",
                    comment: "Notification permission denied guidance"
                )
            }
        } catch {
            enableNotifications = false
            operationIsError = true
            operationMessage = String(
                format: NSLocalizedString("settings.notifications.permission_failed", value: "Could not update notification permission: %@", comment: "Notification permission update failed"),
                error.localizedDescription
            )
        }
    }

    private func beginClearAllData() {
        guard !isClearingData, !isExporting else { return }
        operationMessage = nil
        operationIsError = false
        isClearingData = true
        Task { @MainActor in
            defer { isClearingData = false }
            await Task.yield()
            let completelyCleared = clearAllData()
            refreshLocalDataSize()
            operationIsError = !completelyCleared
            operationMessage = completelyCleared
                ? NSLocalizedString("settings.data.clear_complete", value: "Local data was cleared.", comment: "Clear data completed")
                : NSLocalizedString("settings.data.clear_partial", value: "Most local data was cleared, but some conversation records could not be removed.", comment: "Clear data partially completed")
        }
    }

    @discardableResult
    private func clearAllData() -> Bool {
        var completelyCleared = true
        container.dataManager.clearAllData()
        if let store = container.conversationStore {
            do {
                let conversations = try store.getAllConversations()
                for conversation in conversations {
                    try store.deleteConversation(conversationId: conversation.id)
                }
            } catch {
                completelyCleared = false
                container.logger.log(.warn, "SettingsView", "Failed to clear conversations", metadata: ["error": error.localizedDescription])
            }
        }
        container.chatManager.currentConversationId = nil
        container.chatManager.reset()
        if let fetcher = container.urlFetcher {
            let sourceIDs = fetcher.sources.map(\.id)
            for sourceID in sourceIDs {
                fetcher.removeSource(id: sourceID)
            }
        }
        return completelyCleared
    }

    private func refreshLocalDataSize() {
        localDataSize = container.dataManager.getDataSize()
    }

    private func openApplicationSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    private func finishExportPresentation() {
        if let exportedFileURL {
            try? FileManager.default.removeItem(at: exportedFileURL)
            self.exportedFileURL = nil
        }
        operationIsError = false
        operationMessage = NSLocalizedString(
            "settings.data.export_complete",
            value: "Export prepared successfully.",
            comment: "Export completed"
        )
    }
}

enum SettingsNotificationPresentation {
    static func requiresSystemSettings(
        requestedEnabled: Bool,
        authorizationStatus: UNAuthorizationStatus
    ) -> Bool {
        requestedEnabled && authorizationStatus == .denied
    }
}

private struct NolaSettingsStatusBanner: View {
    let message: String
    let isError: Bool
    let onDismiss: () -> Void

    private var tint: Color {
        isError ? .red : .green
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: isError ? "exclamationmark.circle.fill" : "checkmark.circle.fill")
                .foregroundStyle(tint)
            Text(message)
                .font(.footnote)
                .foregroundStyle(.primary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .frame(width: 44, height: 44)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel(NSLocalizedString("common.close", value: "Close", comment: "Close action"))
        }
        .padding(14)
        .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(tint.opacity(0.20), lineWidth: 1)
        }
    }
}

private struct NolaSettingsCard<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    private let content: Content
    private let tint: Color?

    init(tint: Color? = nil, @ViewBuilder content: () -> Content) {
        self.tint = tint
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct NolaSettingsIconTile: View {
    let icon: String
    let tint: Color

    var body: some View {
        Image(systemName: icon)
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(tint)
            .frame(width: 34, height: 34)
            .background(DesignTokens.rowFillColor, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct NolaSettingsLinkLabel: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let icon: String
    let tint: Color
    let title: String
    let detail: String
    var trailingIcon: String? = "chevron.right"
    var isBusy = false

    var body: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 12))
            : AnyLayout(HStackLayout(spacing: 12))
        return layout {
            if isBusy {
                ProgressView()
                    .controlSize(.small)
                    .frame(width: 34, height: 34)
                    .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .accessibilityLabel(title)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.body.weight(.medium))
                    .foregroundStyle(.primary)
                    .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: 8) }
            if let trailingIcon, !dynamicTypeSize.isAccessibilitySize {
                Image(systemName: trailingIcon)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(minHeight: 64)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

struct NolaSettingsToggleRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let icon: String
    let tint: Color
    let title: String
    var detail: String? = nil
    var isBusy = false
    @Binding var isOn: Bool

    private var rowLabel: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 12))
            : AnyLayout(HStackLayout(spacing: 12))
        return layout {
            NolaSettingsIconTile(icon: icon, tint: tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.body.weight(.medium))
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: 8) }
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }

    var body: some View {
        Group {
            if isBusy {
                HStack(spacing: 12) {
                    rowLabel
                    ProgressView().controlSize(.small).frame(width: 51)
                        .accessibilityLabel(detail ?? title)
                }
            } else if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 12) {
                    Button { isOn.toggle() } label: { rowLabel }
                        .buttonStyle(NolaActionButtonStyle())
                        // The native switch below supplies the single VoiceOver
                        // toggle; this label also accepts direct touch input.
                        .accessibilityHidden(true)
                    Toggle(title, isOn: $isOn)
                        .labelsHidden()
                        .accessibilityLabel(title)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .trailing)
                }
            } else {
                Toggle(isOn: $isOn) { rowLabel }
            }
        }
        .frame(minHeight: detail == nil ? 58 : 66)
        .padding(.vertical, 8)
    }

}

private struct NolaSettingsPickerRow: View {
    let icon: String
    let tint: Color
    let title: String
    @Binding var selection: String

    var body: some View {
        Menu {
            Picker(title, selection: $selection) {
                Text(L10n.settingsNotificationDaily).tag("daily")
                Text(L10n.settingsNotificationWeekly).tag("weekly")
                Text(L10n.settingsNotificationImmediate).tag("immediate")
            }
        } label: {
            HStack(spacing: 12) {
                NolaSettingsIconTile(icon: icon, tint: tint)
                Text(title).font(.body.weight(.medium)).foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(["daily": L10n.settingsNotificationDaily, "weekly": L10n.settingsNotificationWeekly,
                      "immediate": L10n.settingsNotificationImmediate][selection] ?? selection)
                    .foregroundStyle(.secondary)
                Image(systemName: "chevron.up.chevron.down").font(.caption)
            }
            .frame(maxWidth: .infinity, minHeight: 58)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(NolaActionButtonStyle())
        .accessibilityLabel(title)
    }

}

private struct NolaSettingsValueRow: View {
    let icon: String
    let tint: Color
    let title: String
    let value: String

    var body: some View {
        HStack(spacing: 12) {
            NolaSettingsIconTile(icon: icon, tint: tint)
            Text(title)
                .font(.body.weight(.medium))
            Spacer(minLength: 8)
            Text(value)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(minHeight: 58)
        .padding(.vertical, 8)
    }
}

private struct NolaSettingsActionLabel: View {
    let icon: String
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 12) {
            NolaSettingsIconTile(icon: icon, tint: .red)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.body.weight(.medium))
                    .foregroundStyle(.red)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
        }
        .frame(minHeight: 62)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

private struct NolaSettingsDivider: View {
    var body: some View {
        Color.clear.frame(height: 8).accessibilityHidden(true)
    }
}

#Preview {
    SettingsView()
        .environmentObject(ServiceContainer())
}

struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
