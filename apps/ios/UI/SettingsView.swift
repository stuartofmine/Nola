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
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.colorScheme) private var colorScheme
    @State private var showingClearDataAlert = false
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
    @State private var notificationError: String?

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
            Task { await refreshNotificationAuthorization(returningToApp: true) }
        }
    }

    private enum Category: String, CaseIterable {
        case general, providers, agents, agent, privacy
        var titleKey: String {
            switch self {
            case .agents: "experts.manage"
            case .agent: "personal.capabilities.title"
            default: "settings.navigation." + rawValue
            }
        }
        var detailKey: String { titleKey + ".detail" }
    }

    private var categoryGroups: [[Category]] {
        [[.general], [.providers, .agents, .agent], [.privacy]]
    }

    private var content: some View {
        settingsPage(title: L10n.settings, showsHeading: false, topInset: ownsNavigationStack ? 4 : DesignTokens.spacing_lg) {
            if ownsNavigationStack && horizontalSizeClass != .compact {
                IOSWorkspacePageHeading(title: L10n.settings).frame(minHeight: 64)
            }
            ForEach(categoryGroups.indices, id: \.self) { group in
                VStack(alignment: .leading, spacing: 8) {
                    settingsSectionTitle(NSLocalizedString(["settings.group.preferences", "settings.group.connections", "settings.group.security"][group], comment: "Settings group heading"), compactSpacing: true)
                    NolaSettingsCard {
                        ForEach(categoryGroups[group], id: \.rawValue) { category in
                            NavigationLink { destination(category) } label: { categoryLabel(category) }
                            .buttonStyle(NolaPressableButtonStyle())
                            .accessibilityIdentifier(category == .providers ? "settings.models" : "settings.category." + category.rawValue)
                            if category != categoryGroups[group].last {
                                Rectangle().fill(DesignTokens.hairlineBorderColor(for: colorScheme)).frame(height: 0.5).padding(.leading, 48).accessibilityHidden(true)
                            }
                        }
                    }
                }
            }
            VStack(alignment: .leading, spacing: 8) {
                settingsSectionTitle(
                    NSLocalizedString("settings.group.about_diagnostics", value: "About & Diagnostics", comment: "About and diagnostics settings section"),
                    compactSpacing: true
                )
                NolaSettingsCard {
                    NolaSettingsValueRow(
                        icon: "info.circle",
                        tint: DesignTokens.secondaryContentColor,
                        title: NSLocalizedString("settings.about.version", value: "App Version", comment: "App version label"),
                        value: appVersionLabel
                    )
                    NolaSettingsDivider()
                    NavigationLink {
                        LogViewerView()
                    } label: {
                        NolaSettingsLinkLabel(
                            icon: "doc.text.magnifyingglass",
                            tint: DesignTokens.actionColor,
                            title: NSLocalizedString("settings.diagnostics.logs", value: "Diagnostic Logs", comment: "Diagnostic logs link"),
                            trailingIcon: "chevron.right"
                        )
                    }
                    .buttonStyle(NolaPressableButtonStyle())
                    .accessibilityIdentifier("settings.diagnostics.logs")
                }
            }
        }
        .navigationTitle(ownsNavigationStack ? "" : L10n.settings)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if ownsNavigationStack && horizontalSizeClass == .compact {
                IOSWorkspaceToolbarHeading(title: L10n.settings, identifier: "settings.heading")
            }
        }
        .toolbarBackground(DesignTokens.backgroundColor, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
    }

    private func categoryLabel(_ category: Category) -> some View {
        HStack(spacing: 12) {
            Image(categoryIcon(category)).renderingMode(.template).resizable().scaledToFit()
                .frame(width: 22, height: 22).foregroundStyle(DesignTokens.actionColor)
            Text(LocalizedStringKey(category.titleKey)).font(.body).foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Image(systemName: "chevron.right").font(.caption.weight(.medium)).foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, minHeight: 56)
        .padding(.vertical, 8)
        .padding(.horizontal, 14)
        .contentShape(Rectangle())
    }

    private func categoryIcon(_ category: Category) -> String {
        switch category {
        case .general: "nola-settings"
        case .providers: "nola-models"
        case .agents: "nola-person"
        case .agent: "nola-execution"
        case .privacy: "nola-review"
        }
    }

    @ViewBuilder private func destination(_ category: Category) -> some View {
        switch category {
        case .general:
            settingsPage(title: NSLocalizedString(category.titleKey, comment: "General settings"), showsHeading: false) {
                settingsSectionTitle(NSLocalizedString("settings.navigation.appearance", comment: "Appearance section"))
                NolaSettingsCard { appearanceControls }
                settingsSectionTitle(NSLocalizedString("settings.navigation.notifications", comment: "Notifications section"))
                notificationControls
                if let notificationError {
                    NolaSettingsStatusBanner(message: notificationError, isError: true) {
                        self.notificationError = nil
                    }
                }
                AppleSoftwareUpdateSection()
            }.navigationTitle(LocalizedStringKey(category.titleKey)).navigationBarTitleDisplayMode(.inline)
        case .providers:
            ModelsView(ownsNavigationStack: false).environmentObject(container)
        case .agents:
            IOSAgentRoomsView(embeddedInNavigationStack: true).environmentObject(container)
        case .agent:
            PersonalCapabilitiesLibraryView().environmentObject(container)
        case .privacy:
            privacyPage
        }
    }

    private var appearanceControls: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("settings.appearance.theme")
                .font(.body.weight(.medium))
            AppleAppearancePicker()
                .labelsHidden()
                .pickerStyle(.segmented)
            let motionKey = reduceMotion ? "settings.appearance.motionReduced" : "settings.appearance.motionStandard"
            Text(LocalizedStringKey(motionKey))
                .font(.footnote).foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    private func settingsPage<Content: View>(title: String, showsHeading: Bool = true, topInset: CGFloat = DesignTokens.spacing_lg, @ViewBuilder content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if showsHeading { IOSWorkspacePageHeading(title: title) }
                content()
            }
            .padding(.horizontal, horizontalSizeClass == .regular ? 24 : 20)
            .padding(.top, topInset)
            .padding(.bottom, DesignTokens.pageBottomInset)
            .frame(maxWidth: DesignTokens.pageMaxWidth)
            .frame(maxWidth: .infinity)
        }
        .background(NolaPageBackground())
    }

    private var notificationControls: some View {
        NolaSettingsCard {
            NolaSettingsToggleRow(
                icon: "bell.badge",
                tint: DesignTokens.actionColor,
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
                        tint: DesignTokens.dangerColor,
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
                    tint: DesignTokens.actionColor,
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
                    tint: DesignTokens.actionColor,
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
                    tint: DesignTokens.secondaryContentColor,
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
                } label: {
                    NolaSettingsLinkLabel(
                        icon: "hand.raised.fill",
                        tint: DesignTokens.actionColor,
                        title: NSLocalizedString("privacy.disclosure.nav.title", value: "Privacy", comment: "Privacy disclosure title")
                    )
                }
                .buttonStyle(.plain)
            }
            settingsSectionTitle(NSLocalizedString("settings.section.reset", comment: "Reset section"))
            NolaSettingsCard(tint: DesignTokens.dangerColor) {
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
                .accessibilityIdentifier("settings.privacy.clear")
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

    private var appVersionLabel: String {
        let short = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
        let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String
        if let build, !build.isEmpty, build != short {
            return "\(short) (\(build))"
        }
        return short
    }

    private func settingsSectionTitle(_ title: String, compactSpacing: Bool = false) -> some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(DesignTokens.secondaryContentColor)
            .textCase(.uppercase)
            .tracking(0.5)
            .padding(.horizontal, 4)
            .padding(.bottom, compactSpacing ? 0 : -12)
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
    private func refreshNotificationAuthorization(returningToApp: Bool = false) async {
        let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
        notificationAuthorizationStatus = status
        let requested = shouldEnableNotificationsAfterSettings && returningToApp
        if returningToApp { shouldEnableNotificationsAfterSettings = false }
        enableNotifications = SettingsNotificationPresentation.refreshedPreference(
            current: enableNotifications, requestedFromSettings: requested, authorizationStatus: status
        )
        if notificationsAreUsable { notificationError = nil }
    }

    @MainActor
    private func updateNotificationPreference(enabled: Bool) async {
        guard enabled else {
            UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
            return
        }
        guard !isUpdatingNotificationAuthorization else { return }
        guard !notificationsAreUsable else { return }
        notificationError = nil

        if SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: enabled,
            authorizationStatus: notificationAuthorizationStatus
        ) {
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
                notificationError = NSLocalizedString(
                    "settings.notifications.permission_denied",
                    value: "Notifications are blocked by iOS. You can allow them later in Settings.",
                    comment: "Notification permission denied guidance"
                )
            }
        } catch {
            enableNotifications = false
            notificationError = String(
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
            let completelyCleared = await clearAllData()
            refreshLocalDataSize()
            operationIsError = !completelyCleared
            operationMessage = completelyCleared
                ? NSLocalizedString("settings.data.clear_complete", value: "Local data was cleared.", comment: "Clear data completed")
                : NSLocalizedString("settings.data.clear_partial", value: "Some local data could not be removed. Review the remaining conversations and data sources.", comment: "Clear data partially completed")
        }
    }

    @discardableResult
    private func clearAllData() async -> Bool {
        await Self.clearLocalData(container: container)
    }

    @MainActor
    @discardableResult
    static func clearLocalData(container: ServiceContainer) async -> Bool {
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
            let sources = fetcher.sources
            for source in sources {
                do {
                    try await fetcher.removeSourceConfirmed(id: source.id, expectedRevision: source.canonicalRevision)
                } catch {
                    completelyCleared = false
                    container.logger.log(.warn, "SettingsView", "Failed to clear data source", metadata: ["error": error.localizedDescription])
                }
            }
            if !fetcher.sources.isEmpty {
                completelyCleared = false
            }
        }
        let clearedAtMs = Int64(Date().timeIntervalSince1970 * 1_000)
        if !container.applicationRuntime.clearIntelligence() {
            completelyCleared = false
            container.logger.log(.warn, "SettingsView", "Failed to clear PersonalIntelligenceCore store")
        }
        do {
            _ = try container.applicationRuntime.deleteProductSetting(key: "assistant.uploaded_documents")
        } catch {
            completelyCleared = false
            container.logger.log(.warn, "SettingsView", "Failed to clear uploaded documents", metadata: ["error": error.localizedDescription])
        }
        do {
            let memories = try container.applicationRuntime.listMemories(limit: 1_000)
            if !memories.isEmpty {
                _ = try container.applicationRuntime.replaceMemories(
                    expected: memories,
                    candidates: [],
                    deletedAtMs: clearedAtMs
                )
            }
        } catch {
            completelyCleared = false
            container.logger.log(.warn, "SettingsView", "Failed to clear memories", metadata: ["error": error.localizedDescription])
        }
        do {
            let userSkills = try container.applicationRuntime.listSkills().filter { $0.source != "built_in" }
            for skill in userSkills {
                _ = try container.applicationRuntime.deleteUserSkill(id: skill.id, deletedAtMs: clearedAtMs)
            }
        } catch {
            completelyCleared = false
            container.logger.log(.warn, "SettingsView", "Failed to clear user skills", metadata: ["error": error.localizedDescription])
        }
        do {
            for server in try container.applicationRuntime.listMCPServers() {
                _ = try container.applicationRuntime.deleteMCPServer(id: server.id, deletedAtMs: clearedAtMs)
            }
        } catch {
            completelyCleared = false
            container.logger.log(.warn, "SettingsView", "Failed to clear MCP servers", metadata: ["error": error.localizedDescription])
        }
        if let notificationManager = container.notificationManager {
            do {
                try notificationManager.clearAll()
            } catch {
                completelyCleared = false
                container.logger.log(.warn, "SettingsView", "Failed to clear notifications", metadata: ["error": error.localizedDescription])
            }
        }
        if let promptTemplates = container.promptTemplates {
            let customTemplates = promptTemplates.templates.filter { !$0.isBuiltIn }
            for template in customTemplates {
                do {
                    try promptTemplates.deleteTemplate(id: template.id)
                } catch {
                    completelyCleared = false
                    container.logger.log(.warn, "SettingsView", "Failed to clear custom prompt template", metadata: ["error": error.localizedDescription])
                }
            }
        }
        if let storage = container.localJSONStorage {
            do {
                let tasks = try storage.getAllUserTasks()
                for task in tasks {
                    try storage.deleteUserTask(id: task.id)
                }
            } catch {
                completelyCleared = false
                container.logger.log(.warn, "SettingsView", "Failed to clear user tasks", metadata: ["error": error.localizedDescription])
            }
        }
        for localModel in container.modelDownloader.downloadedModels {
            do {
                try container.modelDownloader.deleteModel(localModel)
            } catch {
                completelyCleared = false
                container.logger.log(.warn, "SettingsView", "Failed to delete local model", metadata: ["error": error.localizedDescription])
            }
        }
        for model in ModelSelectionManager.getAvailableModels() {
            ModelSelectionManager.removeModel(model)
        }
        container.summaryStorage.clearAllSummaries()
        return completelyCleared
    }

    private func refreshLocalDataSize() {
        let databaseURL = container.applicationRuntime.databaseURL
        Task.detached(priority: .utility) {
            let size = DataManager.computeDataSize(databaseURL: databaseURL)
            await MainActor.run {
                localDataSize = size
            }
        }
    }

    private func openApplicationSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        shouldEnableNotificationsAfterSettings = true
        UIApplication.shared.open(url) { opened in
            if !opened { shouldEnableNotificationsAfterSettings = false }
        }
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
    static func refreshedPreference(current: Bool, requestedFromSettings: Bool, authorizationStatus: UNAuthorizationStatus) -> Bool {
        switch authorizationStatus {
        case .denied, .notDetermined: false
        case .authorized, .provisional, .ephemeral: current || requestedFromSettings
        @unknown default: current
        }
    }

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
        isError ? DesignTokens.dangerColor : DesignTokens.successColor
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
        .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(tint?.opacity(0.28) ?? DesignTokens.hairlineBorderColor(for: colorScheme), lineWidth: 1)
        }
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
    var detail: String? = nil
    var trailingIcon: String? = nil
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
            } else {
                NolaSettingsIconTile(icon: icon, tint: tint)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.body.weight(.medium))
                    .foregroundStyle(.primary)
                    .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                if let detail, !detail.isEmpty {
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: 8) }
            if let trailingIcon, !dynamicTypeSize.isAccessibilitySize {
                Image(systemName: trailingIcon)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
        .padding(.horizontal, 14)
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
                        .buttonStyle(NolaPressableButtonStyle())
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
        .padding(.horizontal, 14)
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
                Image(systemName: "chevron.up.chevron.down").font(.caption).foregroundStyle(.tertiary)
            }
            .frame(maxWidth: .infinity, minHeight: 58)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(NolaPressableButtonStyle())
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
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }
}

private struct NolaSettingsActionLabel: View {
    let icon: String
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 12) {
            NolaSettingsIconTile(icon: icon, tint: DesignTokens.dangerColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.body.weight(.medium))
                    .foregroundStyle(DesignTokens.dangerColor)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
        }
        .frame(minHeight: 62)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

private struct NolaSettingsDivider: View {
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Rectangle()
            .fill(DesignTokens.hairlineBorderColor(for: colorScheme))
            .frame(height: 0.5)
            .padding(.leading, 60)
            .accessibilityHidden(true)
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
