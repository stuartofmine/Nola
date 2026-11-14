// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the native macOS application lifecycle, windows, menus, and global settings.

import SwiftUI
import UserNotifications
import AppKit
import Darwin
import ServiceManagement

private func macSettingsText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    let serverCenter = Bundle.main.localizedString(
        forKey: key,
        value: nil,
        table: "ServerCenter"
    )
    guard serverCenter == key else { return serverCenter }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

@MainActor
private enum MacBackgroundGoalRunner {
    static func run(sessionIDs: Set<String>) async -> Bool {
        var succeeded = true
        for sessionID in sessionIDs.sorted() {
            do {
                try await run(sessionID: sessionID)
            } catch {
                succeeded = false
                Logger.shared.log(
                    .error,
                    "MacBackgroundGoal",
                    "Scheduled Goal continuation failed",
                    metadata: [
                        "conversation": String(sessionID.prefix(12)),
                        "error": error.localizedDescription,
                    ]
                )
            }
        }
        return succeeded
    }

    private static func run(sessionID: String) async throws {
        let repository = MacCanonicalConversationRepository(store: MacApplicationPort.shared)
        guard var archive = try repository.load(conversationID: sessionID) else {
            throw failure("The scheduled Goal conversation is unavailable.")
        }
        let goal = try loadGoalControl(
            sessionID: sessionID,
            repository: repository
        )
        let workspace = MacSecurityScopedBookmarkStore.shared.restoreWorkspace(
            conversationID: sessionID
        ) ?? MacAppEnvironment.shared.scratchWorkspaceURL
        // A wake supplies only Goal identity. The native Host restores the
        // committed model/policy/vault binding; current model selection is not
        // a continuation authority.
        var request = try MacLocalAgentRunRequest(
            conversationID: sessionID,
            prompt: "",
            projectURL: workspace,
            attachmentURLs: [],
            inputOrigin: "scheduled_action"
        )
        request.goalWakeID = goal.id
        var terminalFailure: String?
        var reachedSuccessfulTerminal = false
        let stream = await MacAgentHostRuntime.shared.run(request)
        for try await event in stream {
            if event.detail == "external_session_started",
               let eventBackend = event.title,
               !event.text.isEmpty {
                var updated = archive.conversation
                var sessions = updated.externalExecutionSessionIDs ?? [:]
                sessions[eventBackend] = event.text
                updated.externalExecutionSessionIDs = sessions
                archive = MacStoredConversation(
                    conversation: updated,
                    messages: archive.messages,
                    knowledge: archive.knowledge
                )
                try repository.saveMetadata(
                    conversation: updated,
                    knowledge: nil
                )
            }
            if event.kind == .finished {
                reachedSuccessfulTerminal = true
            } else if event.kind == .failed || event.kind == .cancelled {
                terminalFailure = event.detail ?? event.text
            }
        }
        if let terminalFailure {
            throw failure(terminalFailure)
        }
        guard reachedSuccessfulTerminal else {
            throw failure("Scheduled Goal continuation ended without a successful terminal event.")
        }
        Logger.shared.log(
            .info,
            "MacBackgroundGoal",
            "Scheduled Goal continuation completed",
            metadata: ["conversation": String(sessionID.prefix(12))]
        )
    }

    private static func selectedModelID(_ selectionID: String?) -> String? {
        MacChatModelOption.parse(selectionID)?.modelID
    }

    private static func loadGoalControl(
        sessionID: String,
        repository: MacCanonicalConversationRepository
    ) throws
        -> (id: String, objective: String, constraints: [String]) {
        guard let session = try repository.sessionProjection(conversationID: sessionID) else {
            throw failure("The scheduled Goal session is unavailable.")
        }
        guard let projection = try JSONSerialization.jsonObject(
            with: session.goalProjection
        ) as? [String: Any],
              let active = projection["active"] as? [String: Any],
              let id = active["id"] as? String, !id.isEmpty,
              let objective = active["objective"] as? String,
              !objective.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw failure("The scheduled Goal is not active.")
        }
        return (
            id, objective,
            active["constraints"] as? [String] ?? []
        )
    }

    private static func failure(_ message: String) -> NSError {
        NSError(
            domain: "NolaMac.BackgroundGoal",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }
}

@MainActor
final class NolaMacApplicationDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        AppleAppearancePreference.applyToApplication()
    }

    func applicationWillTerminate(_ notification: Notification) {
        MacManagedNolaServer.shared.stop()
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        MacOwnedChildProcessRegistry.shared.stopAll()
        if ProcessInfo.processInfo.environment["NOLA_CAPTURE_UI"] == nil {
            MacChromeMessagingService.shared.stop()
        }
    }
}

@main
struct NolaMacApp: App {
    @NSApplicationDelegateAdaptor(NolaMacApplicationDelegate.self)
    private var applicationDelegate
    private let conversationRepository: MacCanonicalConversationRepository
    private let localAgentRuntime: any MacLocalAgentRuntime
    private let remoteHost: MacRemoteHostService
    private let inspectorDependencies: MacInspectorDependencies

    init() {
        // Chrome launches this executable as a Native Messaging adapter, not
        // as an interactive application. Dispatch before constructing any
        // canonical repositories, agent runtimes or startup integrations.
        // Browser transport must not depend on the chat service opening its
        // Store successfully, and must not recursively warm up the bridge.
        if CommandLine.arguments.contains("--nola-chrome-native-host") {
            NSApplication.shared.setActivationPolicy(.prohibited)
            exit(MacChromeNativeHostProcess.run())
        }
        if CommandLine.arguments.contains("--nola-register-background-scheduler") {
            NSApplication.shared.setActivationPolicy(.prohibited)
            MacBackgroundSchedulerService.registerIfNeeded()
            switch MacBackgroundSchedulerService.status {
            case .enabled, .requiresApproval:
                exit(0)
            case .notRegistered, .notFound:
                exit(1)
            @unknown default:
                exit(1)
            }
        }
        if CommandLine.arguments.contains("--nola-background-scheduler-tick") {
            NSApplication.shared.setActivationPolicy(.prohibited)
            guard let hostURL = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host") else {
                exit(127)
            }
            let process = Process()
            process.executableURL = hostURL
            process.arguments = ["--background-scheduler-tick"]
            process.environment = MacAppEnvironment.shared.launchAgentEnvironment(
                projectURL: MacAppEnvironment.shared.scratchWorkspaceURL
            )
            do {
                try process.run()
                process.waitUntilExit()
                exit(process.terminationStatus)
            } catch {
                NSLog("Nola background scheduler tick failed: %@", error.localizedDescription)
                exit(126)
            }
        }
        conversationRepository = MacCanonicalConversationRepository(store: MacApplicationPort.shared)
        localAgentRuntime = MacAgentHostRuntime.shared
        remoteHost = MacRemoteHostService.shared
        inspectorDependencies = MacInspectorDependencies(
            dataRegistry: MacDataSourceRegistry.shared,
            platformData: MacPlatformDataService.shared,
            webSources: MacWebSourceStore.shared,
            chromeMessaging: MacChromeMessagingService.shared
        )
        MacDebugLoggingConfiguration.apply()
        if !MacProcessRuntimePolicy.isRunningTests,
           UserDefaults.standard.bool(forKey: MacManagedNolaServer.enabledKey),
           ProcessInfo.processInfo.environment["NOLA_CAPTURE_UI"] == nil {
            Task { @MainActor in await MacManagedNolaServer.shared.start() }
        }
        if !MacProcessRuntimePolicy.isRunningTests,
           ProcessInfo.processInfo.environment["NOLA_CAPTURE_UI"] == nil {
            Task.detached(priority: .utility) {
                do {
                    try MacChromeMessagingService.shared.ensureStarted()
                } catch {
                    await Logger.shared.log(
                        "Chrome bridge warm-up failed",
                        level: .debug,
                        category: "MacChromeBridge",
                        metadata: ["error": error.localizedDescription]
                    )
                }
            }
        }
        Logger.shared.log(
            .info,
            "MacAppLifecycle",
            "Mac application runtime started",
            metadata: [
                "debugLogging": MacDebugLoggingConfiguration.isEnabled() ? "enabled" : "disabled",
                "runtime": MacProcessRuntimePolicy.allowsPersistentBackgroundSchedulerRegistration
                    ? "installed"
                    : "development",
            ]
        )
        if CommandLine.arguments.contains("--nola-background-scheduler-run") {
            NSApplication.shared.setActivationPolicy(.prohibited)
            let automationOccurrences: [MacAutomationOccurrence] = {
                guard let marker = CommandLine.arguments.firstIndex(of: "--automation-occurrences"),
                      CommandLine.arguments.indices.contains(marker + 1) else { return [] }
                return CommandLine.arguments[marker + 1]
                    .split(separator: ",")
                    .compactMap { raw in
                        let fields = raw.split(separator: "@", maxSplits: 1)
                        guard fields.count == 2,
                              let id = UUID(uuidString: String(fields[0])),
                              let seconds = Int64(fields[1]) else { return nil }
                        return MacAutomationOccurrence(
                            automationID: id,
                            occurrenceKey: "scheduled:\(seconds)",
                            trigger: "schedule",
                            scheduledForMs: seconds * 1_000
                        )
                    }
            }()
            let refreshWebSources = CommandLine.arguments.contains("--refresh-web-sources")
            let goalSessionIDs: Set<String> = {
                guard let marker = CommandLine.arguments.firstIndex(of: "--goal-session-ids"),
                      CommandLine.arguments.indices.contains(marker + 1) else { return [] }
                return Set(CommandLine.arguments[marker + 1]
                    .split(separator: ",")
                    .map(String.init)
                    .filter { !$0.isEmpty })
            }()
            Task { @MainActor in
                var succeeded = true
                if refreshWebSources {
                    let store = MacWebSourceStore.shared
                    await store.refreshDueSources()
                    succeeded = succeeded && store.errorMessage == nil
                }
                succeeded = await MacAutomationStore.shared.runScheduledAutomations(
                    occurrences: automationOccurrences
                ) && succeeded
                succeeded = await MacBackgroundGoalRunner.run(sessionIDs: goalSessionIDs)
                    && succeeded
                exit(succeeded ? 0 : 1)
            }
            return
        }
        if let marker = CommandLine.arguments.firstIndex(of: "--nola-background-refresh-web-source"),
           CommandLine.arguments.indices.contains(marker + 1),
           let sourceID = UUID(uuidString: CommandLine.arguments[marker + 1]) {
            NSApplication.shared.setActivationPolicy(.prohibited)
            Task { @MainActor in
                let store = MacWebSourceStore.shared
                guard let source = store.sources.first(where: { $0.id == sourceID }) else {
                    exit(2)
                }
                await store.refresh(source, synchronizeSchedule: false)
                let failed = store.sources.first(where: { $0.id == sourceID })?.lastError != nil
                exit(failed ? 1 : 0)
            }
            return
        }
        if CommandLine.arguments.contains("--nola-background-refresh-data") {
            NSApplication.shared.setActivationPolicy(.prohibited)
            Task { @MainActor in
                await MacPlatformDataService.shared.refresh(synchronizeAutomations: false)
                exit(MacPlatformDataService.shared.automationContextError == nil ? 0 : 1)
            }
            return
        }
        UNUserNotificationCenter.current().delegate = MacNotificationRouter.shared
        let isVisualSnapshot = ProcessInfo.processInfo.environment["NOLA_CAPTURE_UI"] != nil
        if MacProcessRuntimePolicy.allowsStartupIntegrations,
           !isVisualSnapshot {
            Task { @MainActor in
                MacBackgroundSchedulerService.registerIfNeeded()
            }
        }
#if DEBUG
        if isVisualSnapshot {
            Task { @MainActor in await MacVisualSnapshotHarness.captureIfRequested() }
        }
        if ProcessInfo.processInfo.environment["NOLA_MAC_RUNTIME_ACCEPTANCE"] != nil {
            Task { @MainActor in await MacRuntimeAcceptanceHarness.runIfRequested() }
        }
#endif
    }

    var body: some Scene {
        let environment = ProcessInfo.processInfo.environment
        let isVisualSnapshot = environment["NOLA_CAPTURE_UI"] != nil
        let isRuntimeAcceptance = environment["NOLA_MAC_RUNTIME_ACCEPTANCE"] != nil
        let suppressProductWindow = MacProcessRuntimePolicy.isRunningTests
            && !isVisualSnapshot
            && !isRuntimeAcceptance
        WindowGroup(id: "chat") {
            if isVisualSnapshot || suppressProductWindow {
                Color.clear
                    .frame(minWidth: 620, minHeight: 520)
            } else {
                MacApplicationStartupGate {
                    MacOnboardingRootView(
                        canonicalRepository: conversationRepository,
                        localAgentRuntime: localAgentRuntime,
                        remoteHost: remoteHost,
                        inspectorDependencies: inspectorDependencies
                    )
                }
                    .frame(minWidth: 920, minHeight: 640)
            }
        }
        .defaultSize(width: 1280, height: 820)
        .windowStyle(.automatic)
        .commands {
            NolaMacCommands()
        }

        WindowGroup("mac.window.conversation", for: String.self) { $conversationID in
            MacApplicationStartupGate {
                MacChatWorkspaceView(
                    initialConversationID: conversationID,
                    canonicalRepository: conversationRepository,
                    localAgentRuntime: localAgentRuntime,
                    remoteHost: remoteHost,
                    inspectorDependencies: inspectorDependencies
                )
            }
                .frame(minWidth: 920, minHeight: 640)
        }
        .defaultSize(width: 1180, height: 780)

        Window("mac.window.data_center", id: "data-center") {
            MacApplicationStartupGate { MacDataCenterView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("Long-term Memory", id: "memory-center") {
            MacApplicationStartupGate {
                MacMemoryCenterPage()
            }
            .frame(
                minWidth: MacWindowMetrics.primaryMinimum.width,
                minHeight: MacWindowMetrics.primaryMinimum.height
            )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("System Access", id: "capability-center") {
            MacApplicationStartupGate { MacCapabilityCenterView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.automations", id: "automation-center") {
            MacApplicationStartupGate { MacAutomationCenterView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.activity", id: "activity-center") {
            MacApplicationStartupGate { MacActivityCenterView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.model_center", id: "model-center") {
            MacApplicationStartupGate {
                MacModelCenterView(remoteHost: remoteHost, navigation: .shared)
            }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        WindowGroup("mac.window.workspace_changes", for: MacWorkspaceReviewRoute.self) { $route in
            if let route {
                MacApplicationStartupGate { MacWorkspaceReviewView(route: route) }
                    .frame(minWidth: 900, minHeight: 620)
            }
        }
        .defaultSize(width: 1180, height: 760)

        Window("mac.window.extensions", id: "extensions-center") {
            MacApplicationStartupGate { MacExtensionsCenterView(remoteHost: remoteHost) }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.diagnostics", id: "diagnostics-center") {
            MacDiagnosticsCenterView()
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.data_management", id: "data-management") {
            MacApplicationStartupGate { MacDataManagementView() }
                .frame(
                    minWidth: MacWindowMetrics.utilityMinimum.width,
                    minHeight: MacWindowMetrics.utilityMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.utilityDefault)

        Window("mac.window.prompt_library", id: "prompt-library") {
            MacApplicationStartupGate { MacPromptLibraryView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.web_sources", id: "web-sources") {
            MacApplicationStartupGate { MacWebSourcesView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.iphone_health", id: "health-summary") {
            MacApplicationStartupGate { MacHealthSummaryView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.notifications", id: "notification-center") {
            MacApplicationStartupGate { MacNotificationCenterView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        Window("mac.window.medication_lookup", id: "medication-search") {
            MacApplicationStartupGate { MacMedicationSearchView() }
                .frame(
                    minWidth: MacWindowMetrics.utilityMinimum.width,
                    minHeight: MacWindowMetrics.utilityMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.utilityDefault)

        Window("mac.window.model_test", id: "model-test") {
            MacApplicationStartupGate { MacModelTestView() }
                .frame(
                    minWidth: MacWindowMetrics.primaryMinimum.width,
                    minHeight: MacWindowMetrics.primaryMinimum.height
                )
        }
        .defaultSize(MacWindowMetrics.primaryDefault)

        WindowGroup("mac.window.model_parameters", for: MacModelParametersRoute.self) { $route in
            if let route {
                MacApplicationStartupGate { MacModelParametersView(route: route) }
                    .frame(minWidth: 680, minHeight: 620)
            }
        }
        .defaultSize(width: 760, height: 760)

        Settings {
            MacSettingsView()
        }
        .defaultSize(width: 820, height: 620)
        .windowResizability(.contentMinSize)
    }
}

struct MacCommandActions {
    let newConversation: () -> Void
    let focusComposer: () -> Void
    let openWorkspace: () -> Void
    let reviewWorkspace: () -> Void
    let showCommandPalette: () -> Void
    let toggleSidebar: () -> Void
    let toggleInspector: () -> Void
    let stop: () -> Void
    let openConversationWindow: () -> Void
}

private enum MacWindowMetrics {
    static let primaryMinimum = CGSize(width: 820, height: 600)
    static let primaryDefault = CGSize(width: 980, height: 720)
    static let utilityMinimum = CGSize(width: 720, height: 560)
    static let utilityDefault = CGSize(width: 860, height: 680)
}

private struct MacCommandActionsKey: FocusedValueKey {
    typealias Value = MacCommandActions
}

extension FocusedValues {
    var nolaMacCommandActions: MacCommandActions? {
        get { self[MacCommandActionsKey.self] }
        set { self[MacCommandActionsKey.self] = newValue }
    }
}

private struct NolaMacCommands: Commands {
    @Environment(\.openWindow) private var openWindow
    @FocusedValue(\.nolaMacCommandActions) private var actions

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("mac.command.new_conversation") {
                actions?.newConversation()
            }
            .keyboardShortcut("n", modifiers: .command)
            .disabled(actions == nil)

            Button("mac.command.new_window") {
                if let actions {
                    actions.openConversationWindow()
                } else {
                    openWindow(id: "chat")
                }
            }
            .keyboardShortcut("n", modifiers: [.command, .shift])
        }

        CommandMenu("mac.command.menu.conversation") {
            Button("mac.command.command_palette") {
                actions?.showCommandPalette()
            }
            .keyboardShortcut("k", modifiers: .command)
            .disabled(actions == nil)

            Button("mac.command.open_prompt_library") {
                openWindow(id: "prompt-library")
            }
            .keyboardShortcut("p", modifiers: [.command, .shift])

            Divider()

            Button("mac.command.focus_composer") {
                actions?.focusComposer()
            }
            .keyboardShortcut("l", modifiers: .command)
            .disabled(actions == nil)

            Divider()

            Button("mac.command.open_workspace") {
                actions?.openWorkspace()
            }
            .keyboardShortcut("o", modifiers: [.command, .shift])
            .disabled(actions == nil)

            Button("mac.command.review_workspace") {
                actions?.reviewWorkspace()
            }
            .keyboardShortcut("g", modifiers: [.command, .shift])
            .disabled(actions == nil)

            Divider()

            Button("mac.command.stop_run") {
                actions?.stop()
            }
            .keyboardShortcut(".", modifiers: .command)
            .disabled(actions == nil)
        }

        CommandGroup(after: .sidebar) {
            Button("mac.command.toggle_sidebar") {
                actions?.toggleSidebar()
            }
            .keyboardShortcut("s", modifiers: [.command, .control])
            .disabled(actions == nil)

            Button("mac.command.toggle_inspector") {
                actions?.toggleInspector()
            }
            .keyboardShortcut("i", modifiers: [.command, .option])
            .disabled(actions == nil)
        }

        CommandMenu("Workspace") {
            Button("mac.command.open_data_center") {
                openWindow(id: "data-center")
            }

            Button("mac.command.automations") {
                openWindow(id: "automation-center")
            }

            Divider()

            Button("mac.window.model_center") {
                openWindow(id: "model-center")
            }

            Button("mac.command.extensions") {
                openWindow(id: "extensions-center")
            }

            Button("System Access") {
                openWindow(id: "capability-center")
            }

            Divider()

            Button("mac.command.activity_center") {
                openWindow(id: "activity-center")
            }

            Button("mac.command.notification_center") {
                openWindow(id: "notification-center")
            }

            Divider()

            Button("mac.command.runtime_diagnostics") {
                openWindow(id: "diagnostics-center")
            }

            Button("mac.window.model_test") { openWindow(id: "model-test") }
            Button("mac.window.data_management") { openWindow(id: "data-management") }
            Divider()
            Button("mac.window.iphone_health") { openWindow(id: "health-summary") }
            Button("mac.window.medication_lookup") { openWindow(id: "medication-search") }
        }

    }
}

private enum MacSettingsCategory: String, CaseIterable, Identifiable {
    case general
    case providers
    case agent
    case connections
    case notifications
    case data
    case diagnostics

    var id: String { rawValue }
    var navigationCategory: Self { self == .notifications ? .general : self }

    var title: String {
        switch self {
        case .general: return macSettingsText("mac.settings.category.general", "General")
        case .providers: return macSettingsText("mac.settings.category.providers", "Providers")
        case .agent: return macSettingsText("mac.settings.category.agent", "Agent")
        case .connections: return macSettingsText("mac.settings.category.connections", "Connections")
        case .data: return macSettingsText("mac.settings.category.data", "Data")
        case .diagnostics: return macSettingsText("mac.settings.category.diagnostics", "Diagnostics")
        case .notifications: return macSettingsText("mac.settings.category.notifications", "Notifications")
        }
    }

    var image: Image {
        switch self {
        case .general: Image("nola-settings")
        case .providers: Image("nola-models")
        case .agent: Image("nola-person")
        case .data: Image("nola-folder")
        case .connections: Image("nola-host")
        case .notifications: Image("nola-bell")
        case .diagnostics: Image("nola-execution")
        }
    }

    var symbol: String {
        switch self {
        case .general: return "gearshape"
        case .providers: return "server.rack"
        case .agent: return "brain"
        case .connections: return "network"
        case .data: return "square.stack.3d.up"
        case .diagnostics: return "stethoscope"
        case .notifications: return "bell"
        }
    }
}

private enum MacSettingsGroup: String, CaseIterable, Identifiable {
    case application
    case intelligence
    case workspace
    case system

    var id: String { rawValue }

    var title: String {
        switch self {
        case .application: return String(localized: "General")
        case .intelligence: return String(localized: "Models")
        case .workspace: return String(localized: "Workspace")
        case .system: return String(localized: "System")
        }
    }

    var categories: [MacSettingsCategory] {
        switch self {
        case .application: return [.general]
        case .intelligence: return [.providers, .agent]
        case .workspace: return [.connections, .data]
        case .system: return [.diagnostics]
        }
    }

    static let primary: [MacSettingsGroup] = [.application, .intelligence, .workspace]
}

enum MacNotificationAuthorizationPolicy {
    static func isGranted(_ status: UNAuthorizationStatus) -> Bool {
        switch status {
        case .authorized, .provisional, .ephemeral:
            true
        case .notDetermined, .denied:
            false
        @unknown default:
            false
        }
    }

    static func needsSystemSettings(_ status: UNAuthorizationStatus) -> Bool {
        status == .denied
    }
}

struct MacSettingsView: View {
    @StateObject private var defaultChatModel = MacDefaultChatModelPresentation()
    @Environment(\.openURL) private var openURL
    @AppStorage("nola.mac.settings.category") private var selectedCategory = MacSettingsCategory.general
    @State private var launchAtLogin = MacLaunchAtLoginService.isEnabled
    @State private var hostURL = ""
    @State private var hostToken = ""
    @State private var savedHostURL: URL?
    @State private var hasSavedHostToken = false
    @State private var hostExpectedID = ""
    @State private var importedHostOrigin: URL?
    @State private var hostConnectionState = MacRemoteHostConnectionState.initial
    @State private var managedServer = MacManagedNolaServer.shared
    @AppStorage(MacManagedNolaServer.enabledKey) private var managedServerEnabled = false
    @AppStorage(MacManagedNolaServer.permissionProfileKey)
    private var managedServerPermissionProfile = MacPermissionMode.cautious.permissionProfile
    @State private var hostVerificationRevision = 0
    @State private var showsRemoveHostConfirmation = false
    @State private var settingsError: String?
    @State private var serverCopyFeedback: String?
    @State private var isOpeningHostManagement = false
    @State private var isUpdatingNotificationSettings = false
    @State private var notificationAuthorizationStatus: UNAuthorizationStatus = .notDetermined
    @State private var notificationFeedback: String?
    @State private var userPreferences = MacAssistantPreferences.userPreferences
    @State private var retrievalEnabled = MacAssistantPreferences.retrievalEnabled
    @State private var memoryEnabled = MacAssistantPreferences.memoryEnabled
    @State private var contextLimit = MacAssistantPreferences.contextLimit
    @AppStorage(MacRunNotificationPreferenceKeys.agentCompletion) private var agentCompletionNotifications = false
    @AppStorage(MacRunNotificationPreferenceKeys.notificationsEnabled) private var notificationsEnabled = true
    private let skipsAutomaticHostVerification: Bool
    private let embedsInWorkspace: Bool
    private let initialCategory: String?
    @State private var showsOverview: Bool
    private let openModels: ((String?) -> Void)?

    init(snapshotHostState: MacRemoteHostConnectionState? = nil, embedsInWorkspace: Bool = false,
         initialCategory: String? = nil, openModels: ((String?) -> Void)? = nil) {
        self.embedsInWorkspace = embedsInWorkspace
        self.initialCategory = initialCategory
        _showsOverview = State(initialValue: embedsInWorkspace && initialCategory == nil)
        self.openModels = openModels
        if !MacProcessRuntimePolicy.usesEphemeralCredentials {
            let host = MacRemoteHostConfigurationStore.shared.displayValues()
            _hostURL = State(initialValue: host.url)
            _savedHostURL = State(initialValue: URL(string: host.url))
            _hasSavedHostToken = State(initialValue: !host.token.isEmpty)
            _hostExpectedID = State(initialValue: host.hostID)
        }
        skipsAutomaticHostVerification = snapshotHostState != nil
        guard let snapshotHostState else { return }
        _hostConnectionState = State(initialValue: snapshotHostState)
        switch snapshotHostState {
        case .notConfigured:
            _hostURL = State(initialValue: "")
            _hostToken = State(initialValue: "")
        default:
            _hostURL = State(initialValue: "https://studio-mac.local:5001")
            _hostToken = State(initialValue: "snapshot-token")
        }
    }
    @AppStorage(MacRunNotificationPreferenceKeys.stepGuidance) private var stepGuidanceEnabled = true
    @AppStorage(MacRunNotificationPreferenceKeys.aggregateCalendar) private var aggregateCalendarNotifications = true
    @AppStorage(MacRunNotificationPreferenceKeys.frequency) private var notificationFrequency = "daily"
    @AppStorage(MacDebugLoggingConfiguration.enabledKey) private var debugLoggingEnabled = false
    @AppStorage(MacDebugLoggingConfiguration.levelKey) private var debugLoggingLevel = "debug"
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        HStack(spacing: 0) {
            if !embedsInWorkspace {
                settingsSidebar
                Rectangle().fill(MacDesignColor.borderSubtle).frame(width: 1)
            }
            VStack(spacing: 0) {
                if embedsInWorkspace && showsOverview { settingsOverview }
                else {
                settingsHeader
                Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)

                if let settingsError {
                    MacInlineNotice(
                        title: macSettingsText("mac.settings.error.title", "Could not update settings"),
                        message: settingsError,
                        kind: .failure,
                        dismiss: { self.settingsError = nil }
                    )
                    .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                    .padding(.top, MacDesignTokens.Spacing.md)
                    .transition(.opacity.combined(with: .move(edge: .top)))
                }

                Form {
                    settingsSections(for: selectedCategory)
                }
                .id(selectedCategory)
                .transition(.opacity)
                .animation(reduceMotion ? nil : MacDesignTokens.Motion.reveal, value: selectedCategory)
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
                .frame(maxWidth: 840, maxHeight: .infinity, alignment: .topLeading)
                .frame(maxWidth: .infinity, alignment: .top)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(MacPageBackground())
        }
        .frame(
            minWidth: embedsInWorkspace ? 0 : 760,
            idealWidth: embedsInWorkspace ? nil : 820,
            minHeight: 540,
            idealHeight: 620
        )
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: settingsError)
        .onAppear { selectedCategory = selectedCategory.navigationCategory }
        .onChange(of: initialCategory, initial: true) { _, category in
            guard embedsInWorkspace else { return }
            if let category, let destination = MacSettingsCategory(rawValue: category) {
                selectedCategory = destination.navigationCategory
                showsOverview = false
            } else { showsOverview = true }
        }
        .task(id: "\(selectedCategory.rawValue)/\(showsOverview)") {
            guard !embedsInWorkspace || !showsOverview else { return }
            if selectedCategory == .providers {
                defaultChatModel.reload()
            }
            if selectedCategory == .connections, !skipsAutomaticHostVerification {
                await verifySavedHostConnection()
            }
            if selectedCategory == .notifications || selectedCategory == .general {
                await synchronizeNotificationAuthorization()
            }
        }
        .confirmationDialog(
            macSettingsText("mac.settings.host.remove.confirm.title", "Remove connected host?"),
            isPresented: $showsRemoveHostConfirmation
        ) {
            Button(
                macSettingsText("mac.settings.host.remove.confirm.action", "Remove Connection"),
                role: .destructive,
                action: removeHostConnection
            )
            Button(macSettingsText("mac.settings.action.cancel", "Cancel"), role: .cancel) {}
        } message: {
            Text(macSettingsText(
                "mac.settings.host.remove.confirm.message",
                "This removes the saved address and access token from this Mac."
            ))
        }
    }

    private var settingsSidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Image("nola-settings").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 15, height: 15)
                    .foregroundStyle(MacDesignColor.brand)
                Text(macSettingsText("mac.settings.title", "Settings"))
                    .font(.system(size: 15, weight: .semibold))
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 16)

            GeometryReader { proxy in
                ScrollViewReader { reader in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(MacSettingsGroup.primary) { group in
                                settingsSidebarGroup(group)
                                    .padding(.bottom, 12)
                            }

                            Spacer(minLength: 12)

                            settingsSidebarGroup(.system)
                        }
                        .frame(minHeight: proxy.size.height, alignment: .topLeading)
                    }
                    .scrollIndicators(.hidden)
                    .onChange(of: selectedCategory) { _, category in
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.responsive) {
                            reader.scrollTo(category.id, anchor: .center)
                        }
                    }
                }
            }
        }
        .padding(12)
        .frame(width: MacDesignTokens.Layout.sidebarWidth)
        .frame(maxHeight: .infinity, alignment: .topLeading)
        .background {
            if reduceTransparency {
                MacDesignColor.sidebar
            } else {
                Rectangle()
                    .fill(.ultraThinMaterial)
                    .overlay(MacDesignColor.sidebarGlass)
            }
        }
    }

    private func settingsSidebarGroup(_ group: MacSettingsGroup) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(group.title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.bottom, 2)

            ForEach(group.categories) { category in
                MacSettingsSidebarRow(
                    category: category,
                    isSelected: selectedCategory == category
                ) {
                    withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.responsive) {
                        selectedCategory = category
                    }
                }
                .id(category.id)
            }
        }
    }

    private var settingsOverview: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(String(localized: "Settings")).font(.system(size: 24, weight: .semibold))
                    .padding(.bottom, 28)
                ForEach(MacSettingsGroup.allCases.flatMap(\.categories)) { category in
                    Divider()
                    Button { selectedCategory = category; showsOverview = false } label: {
                        HStack(spacing: 14) {
                            category.image.renderingMode(.template).resizable().scaledToFit()
                                .frame(width: 20, height: 20).foregroundStyle(.secondary)
                            Text(category.title)
                            Spacer(minLength: 8)
                        }.frame(maxWidth: .infinity, minHeight: 64, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)))
                    .accessibilityIdentifier("settings.category.\(category.rawValue)")
                }
            }.padding(24).frame(maxWidth: 840, alignment: .topLeading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        }
    }

    private var settingsHeader: some View {
        HStack(spacing: 9) {
            if embedsInWorkspace {
                Button { showsOverview = true } label: {
                    Label { Text(String(localized: "Settings")) } icon: {
                        Image("nola-back").renderingMode(.template).resizable().scaledToFit().frame(width: 14, height: 14)
                    }
                }.buttonStyle(MacPressFeedbackButtonStyle())
                    .help(String(localized: "Settings"))
                    .accessibilityIdentifier("settings.back")
                Divider().frame(height: 16)
            }
            selectedCategory.image.renderingMode(.template).resizable().scaledToFit()
                .frame(width: 16, height: 16)
                .foregroundStyle(.secondary)
                .frame(width: 22, height: 22)
            Text(selectedCategory.title)
                .font(.system(size: 15, weight: .semibold))
            Spacer()

        }
        .padding(.horizontal, 20)
        .frame(height: MacDesignTokens.Control.pageHeader)
        .background {
            if reduceTransparency {
                MacDesignColor.chrome
            } else {
                Rectangle()
                    .fill(.ultraThinMaterial)
                    .overlay(MacDesignColor.chrome.opacity(0.72))
            }
        }
    }

    private var hostStatusTitle: String {
        switch hostConnectionState {
        case .notConfigured:
            return macSettingsText("mac.settings.host.status.not_configured", "Not configured")
        case .unverified:
            return macSettingsText("mac.settings.host.status.unverified", "Saved, not verified")
        case .checking:
            return macSettingsText("mac.settings.host.status.checking", "Verifying connection…")
        case .connected:
            return macSettingsText("mac.settings.host.status.connected", "Connected")
        case .failed:
            return macSettingsText("mac.settings.host.status.failed", "Connection failed")
        }
    }

    private var hostStatusDetail: String {
        switch hostConnectionState {
        case .notConfigured:
            return macSettingsText("mac.settings.host.detail.not_configured", "Start Nola Server, then add its address and access token.")
        case .unverified:
            return macSettingsText("mac.settings.host.detail.unverified", "Test the saved connection before selecting it in a conversation.")
        case .checking:
            return macSettingsText("mac.settings.host.detail.checking", "Checking Nola Server identity, authentication, and protocol compatibility.")
        case .connected(let report):
            return "\(report.displayName) · \(report.origin)"
        case .failed(let message):
            return message
        }
    }

    private var hostStatusSymbol: String {
        switch hostConnectionState {
        case .notConfigured: return "network.slash"
        case .unverified: return "network.badge.shield.half.filled"
        case .checking: return "network"
        case .connected: return "checkmark.circle.fill"
        case .failed: return "exclamationmark.triangle.fill"
        }
    }

    private var hostStatusColor: Color {
        switch hostConnectionState {
        case .notConfigured: return .secondary
        case .unverified: return .orange
        case .checking: return .blue
        case .connected: return .green
        case .failed: return .red
        }
    }

    private var hasHostDraft: Bool {
        !hostURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !hostToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var canVerifyHostDraft: Bool {
        guard let url = URL(string: hostURL.trimmingCharacters(in: .whitespacesAndNewlines)),
              ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              !hostToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                || (hasSavedHostToken && savedHostURL == url) else {
            return false
        }
        return true
    }

    private var hostPrimaryActionTitle: String {
        switch hostConnectionState {
        case .connected:
            return macSettingsText("mac.settings.host.verify_again", "Verify Again")
        case .failed:
            return macSettingsText("mac.settings.host.retry", "Retry Connection")
        default:
            return macSettingsText("mac.settings.host.save_verify", "Save & Verify")
        }
    }

    private var hostConnectionStatusCard: some View {
        HStack(alignment: .top, spacing: 12) {
            ZStack {
                Circle()
                    .fill(hostStatusColor.opacity(0.12))
                if case .checking = hostConnectionState {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: hostStatusSymbol)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(hostStatusColor)
                }
            }
            .frame(width: 32, height: 32)

            VStack(alignment: .leading, spacing: 3) {
                Text(hostStatusTitle)
                    .font(.callout.weight(.semibold))
                Text(hostStatusDetail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Spacer(minLength: 8)
        }
        .padding(12)
        .background(MacDesignColor.chrome.opacity(0.72), in: RoundedRectangle(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .stroke(hostStatusColor.opacity(0.18), lineWidth: 0.5)
        }
    }

    private var managedServerStatusColor: Color {
        switch managedServer.state {
        case .stopped: return .secondary
        case .starting: return .blue
        case .ready: return .green
        case .failed: return .red
        }
    }

    private var managedServerStatusTitle: String {
        switch managedServer.state {
        case .stopped:
            return macSettingsText("mac.settings.server.stopped", "Server is stopped")
        case .starting:
            return macSettingsText("mac.settings.server.starting", "Starting Nola Server…")
        case .ready:
            return macSettingsText("mac.settings.server.ready", "Nola Server is ready")
        case .failed:
            return macSettingsText("mac.settings.server.failed", "Nola Server could not start")
        }
    }

    private var managedServerControlCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Toggle(
                macSettingsText("mac.settings.server.toggle", "Share this Mac as a Nola Server"),
                isOn: $managedServerEnabled
            )
            .font(.callout.weight(.semibold))
            .onChange(of: managedServerEnabled) { _, enabled in
                Task {
                    if enabled {
                        await managedServer.start()
                    } else {
                        managedServer.stop()
                    }
                }
            }

            Picker(
                macSettingsText("mac.settings.server.permission_mode", "Server permission mode"),
                selection: $managedServerPermissionProfile
            ) {
                ForEach(MacPermissionMode.allCases) { mode in
                    Text(mode.title).tag(mode.permissionProfile)
                }
            }
            .pickerStyle(.segmented)
            .disabled(!managedServerPermissionModeIsEditable)

            Text(managedServerPermissionMode?.detail ?? String(localized: "Invalid permission configuration. Select a valid permission mode before starting."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack(alignment: .top, spacing: 12) {
                ZStack {
                    Circle().fill(managedServerStatusColor.opacity(0.12))
                    if case .starting = managedServer.state {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(
                            systemName: managedServer.state.isRunning
                                ? "checkmark.circle.fill"
                                : "server.rack"
                        )
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(managedServerStatusColor)
                    }
                }
                .frame(width: 32, height: 32)

                VStack(alignment: .leading, spacing: 4) {
                    Text(managedServerStatusTitle)
                        .font(.callout.weight(.semibold))
                    switch managedServer.state {
                    case .ready(let info):
                        Text(info.origin.absoluteString)
                            .font(.system(.caption, design: .monospaced))
                            .textSelection(.enabled)
                        HStack(spacing: 8) {
                            Button(macSettingsText("mac.settings.server.copy_address", "Copy Address")) {
                                copyServerValue(
                                    info.origin.absoluteString,
                                    feedback: macSettingsText("mac.settings.server.address_copied", "Address copied")
                                )
                            }
                            Button(macSettingsText("mac.settings.server.copy_token", "Copy Access Token")) {
                                copyServerValue(
                                    info.accessToken,
                                    feedback: macSettingsText("mac.settings.server.token_copied", "Access token copied")
                                )
                            }
                            if let setup = info.connectionSetupText {
                                Button(macSettingsText("mac.settings.server.copy_setup", "Copy Connection Setup")) {
                                    copyServerValue(
                                        setup,
                                        feedback: macSettingsText("mac.settings.server.setup_copied", "Connection setup copied")
                                    )
                                }
                            }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        if let serverCopyFeedback {
                            Label(serverCopyFeedback, systemImage: "checkmark.circle.fill")
                                .font(.caption)
                                .foregroundStyle(.green)
                                .transition(.opacity)
                        }
                    case .failed(let message):
                        Text(message)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Button(macSettingsText("mac.settings.server.retry", "Try Again")) {
                            Task { await managedServer.start() }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                    default:
                        Text(macSettingsText(
                            "mac.settings.server.detail",
                            "Other Nola clients can use the models, tools, Skills, MCP servers, and data sources configured for this Nola Server."
                        ))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
            }
            Label(
                macSettingsText(
                    "mac.settings.server.security",
                    "Use only on a trusted private network or behind HTTPS. Keep the access token private."
                ),
                systemImage: "lock.shield"
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private func copyToPasteboard(_ value: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
    }

    private func copyServerValue(_ value: String, feedback: String) {
        copyToPasteboard(value)
        withAnimation(.easeOut(duration: 0.16)) {
            serverCopyFeedback = feedback
        }
        Task {
            try? await Task.sleep(for: .seconds(1.8))
            await MainActor.run {
                withAnimation(.easeOut(duration: 0.16)) {
                    if serverCopyFeedback == feedback { serverCopyFeedback = nil }
                }
            }
        }
    }

    private var managedServerPermissionMode: MacPermissionMode? {
        MacPermissionMode(rawValue: managedServerPermissionProfile)
    }

    private var managedServerPermissionModeIsEditable: Bool {
        switch managedServer.state {
        case .stopped, .failed:
            return true
        case .starting, .ready:
            return false
        }
    }

    private func messagingChannelStatusCard(_ report: MacRemoteHostConnectionReport) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: "message.badge.waveform")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(report.messagingWorkerEnabled ? Color.green : Color.secondary)
                .frame(width: 30, height: 30)
                .background(
                    (report.messagingWorkerEnabled ? Color.green : Color.secondary).opacity(0.10),
                    in: RoundedRectangle(cornerRadius: 9)
                )
            VStack(alignment: .leading, spacing: 3) {
                Text(macSettingsText("mac.settings.messaging.title", "Messaging channels"))
                    .font(.callout.weight(.semibold))
                Text(String(format: macSettingsText(
                    "mac.settings.messaging.summary",
                    "%lld connected of %lld · worker %@"
                ), Int64(report.messagingConnectedCount), Int64(report.messagingChannelCount), report.messagingWorkerEnabled
                    ? macSettingsText("mac.settings.messaging.running", "running")
                    : macSettingsText("mac.settings.messaging.stopped", "stopped")))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(macSettingsText(
                    "mac.settings.messaging.providers",
                    "Telegram · WhatsApp · WeChat Official Account · WeCom"
                ))
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(MacDesignColor.brand)
            }
            Spacer(minLength: 12)
            if let url = report.messagingManagementURL {
                Button(macSettingsText("mac.settings.messaging.manage", "Manage on Host")) {
                    isOpeningHostManagement = true
                    let target = url.path
                        + (url.query.map { "?\($0)" } ?? "")
                        + (url.fragment.map { "#\($0)" } ?? "")
                    Task {
                        do {
                            let managementURL = try await MacRemoteHostService.shared
                                .requestWebManagementURL(target: target)
                            await MainActor.run { openURL(managementURL) }
                        } catch {
                            await MainActor.run {
                                settingsError = macRemoteHostUserFacingError(error)
                            }
                        }
                        await MainActor.run { isOpeningHostManagement = false }
                    }
                }
                .buttonStyle(.bordered)
                .disabled(isOpeningHostManagement)
                .overlay {
                    if isOpeningHostManagement {
                        ProgressView().controlSize(.small)
                    }
                }
            }
        }
        .padding(12)
        .background(MacDesignColor.chrome.opacity(0.66), in: RoundedRectangle(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .stroke(MacDesignColor.borderSubtle, lineWidth: 0.5)
        }
    }

    private func markHostConfigurationEdited() {
        hostVerificationRevision &+= 1
        let hasValues = !hostURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !hostToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        hostConnectionState = hasValues ? .unverified : .notConfigured
    }

    private func verifySavedHostConnection() async {
        hostVerificationRevision &+= 1
        let revision = hostVerificationRevision
        guard MacRemoteHostConfigurationStore.shared.load() != nil else {
            hostConnectionState = .notConfigured
            return
        }
        hostConnectionState = .checking
        do {
            let report = try await MacRemoteHostService.shared.testSavedConnection()
            guard revision == hostVerificationRevision else { return }
            hostConnectionState = .connected(report)
        } catch {
            guard revision == hostVerificationRevision else { return }
            hostConnectionState = .failed(macRemoteHostUserFacingError(error))
        }
    }

    private func expectedHostIdentity(for url: URL) -> String? {
        if let importedHostOrigin, AppleHostConnectionSetup.sameOrigin(importedHostOrigin, url) {
            return hostExpectedID
        }
        if let saved = MacRemoteHostConfigurationStore.shared.load(), let savedIdentity = saved.expectedHostID,
           saved.baseURL == url { return savedIdentity }
        return nil
    }

    private func saveAndVerifyHostConnection() async {
        let normalizedURL = hostURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: normalizedURL),
              ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              let normalizedToken = MacRemoteHostConfigurationStore.shared.tokenForDraft(
                url: url, enteredToken: hostToken) else {
            hostConnectionState = .failed(
                macSettingsText("mac.settings.host.invalid", "Enter a valid Host URL and access token.")
            )
            return
        }
        hostVerificationRevision &+= 1
        let revision = hostVerificationRevision
        hostConnectionState = .checking
        do {
            let report = try await MacRemoteHostService.shared.testConnection(
                configuration: MacRemoteHostConfiguration(
                    baseURL: url,
                    token: normalizedToken,
                    expectedHostID: expectedHostIdentity(for: url)
                )
            )
            guard revision == hostVerificationRevision else { return }
            try MacRemoteHostConfigurationStore.shared.save(
                url: normalizedURL,
                token: normalizedToken,
                hostID: report.hostID
            )
            hostURL = normalizedURL
            hostToken = ""
            savedHostURL = url
            hasSavedHostToken = true
            hostExpectedID = report.hostID
            withAnimation(.easeOut(duration: 0.18)) {
                hostConnectionState = .connected(report)
            }
        } catch {
            guard revision == hostVerificationRevision else { return }
            hostConnectionState = .failed(macRemoteHostUserFacingError(error))
        }
    }

    private func removeHostConnection() {
        hostVerificationRevision &+= 1
        do {
            try MacRemoteHostConfigurationStore.shared.save(url: "", token: "", hostID: nil)
        } catch {
            hostConnectionState = .failed(error.localizedDescription)
            return
        }
        hostURL = ""
        hostToken = ""
        savedHostURL = nil
        hasSavedHostToken = false
        hostExpectedID = ""
        importedHostOrigin = nil
        withAnimation(.easeOut(duration: 0.18)) {
            hostConnectionState = .notConfigured
        }
    }

    private func importHostConnectionSetupFromPasteboard() {
        guard let text = NSPasteboard.general.string(forType: .string),
              let setup = MacNolaServerConnectionSetup.decode(text) else {
            hostConnectionState = .failed(
                macSettingsText(
                    "mac.settings.host.invalid_setup",
                    "The clipboard does not contain a valid Nola Server connection setup."
                )
            )
            return
        }
        hostURL = setup.origin.absoluteString
        hostToken = setup.accessToken
        hostExpectedID = setup.hostID
        importedHostOrigin = setup.origin
        markHostConfigurationEdited()
    }

    @ViewBuilder
    private func settingsSections(for category: MacSettingsCategory) -> some View {
        switch category {
        case .general:
            Section { AppleAppearancePicker() }
            notificationSettingsSections
            Section { AppleSoftwareUpdateSection() }
            Section(macSettingsText("mac.settings.section.startup", "Startup")) {
                Toggle(macSettingsText("mac.settings.launch_at_login", "Launch Nola at login"), isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, enabled in
                        do {
                            try MacLaunchAtLoginService.setEnabled(enabled)
                        } catch {
                            launchAtLogin = MacLaunchAtLoginService.isEnabled
                            settingsError = error.localizedDescription
                        }
                    }
            }
        case .providers:
            Section(macSettingsText("mac.settings.section.provider", "API Providers")) {
                Text(macSettingsText(
                    "mac.settings.provider.detail",
                    "Manage multiple endpoints and models in Model Center. Priorities, order, and allowed uses determine which models are available."
                ))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                let endpoints = MacAppleChatConfigurationStore.shared.endpoints()
                let models = MacAppleChatConfigurationStore.shared.configuredModels()
                LabeledContent(macSettingsText("mac.settings.provider.endpoints", "Endpoints")) {
                    Text(endpoints.count, format: .number)
                }
                LabeledContent(macSettingsText("mac.settings.provider.chat_models", "Chat models")) {
                    Text(models.count, format: .number)
                }
                MacSettingsDefaultChatModelSummary(preference: defaultChatModel) { remoteProfileID in
                    if let openModels { openModels(remoteProfileID) }
                    else {
                        MacModelCenterNavigationState.shared.showSelectedModel(remoteProfileID: remoteProfileID)
                        openWindow(id: "model-center")
                    }
                }
                Button {
                    if let openModels { openModels(nil) }
                    else {
                        MacModelCenterNavigationState.shared.showRemoteModels()
                        openWindow(id: "model-center")
                    }
                } label: {
                    Label(
                        macSettingsText("mac.settings.provider.open_model_center", "Manage API Models"),
                        systemImage: "arrow.up.forward.app"
                    )
                }
                .buttonStyle(.borderedProminent)
            }

        case .agent:
            Section(macSettingsText("mac.settings.section.conversation_defaults", "Conversation Defaults")) {
                Text(macSettingsText("mac.settings.agent.defaults_detail", "These defaults apply globally. Individual conversations can choose whether to use memory and retrieval."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Toggle(
                    macSettingsText("mac.settings.agent.memory_default", "Use long-term memory by default"),
                    isOn: Binding(
                        get: { memoryEnabled },
                        set: { enabled in
                            memoryEnabled = enabled
                            MacAssistantPreferences.setMemoryEnabled(enabled)
                        }
                    )
                )
                Toggle(
                    macSettingsText("mac.settings.agent.retrieval_default", "Use retrieval by default"),
                    isOn: Binding(
                        get: { retrievalEnabled },
                        set: { enabled in
                            retrievalEnabled = enabled
                            MacAssistantPreferences.setRetrievalEnabled(enabled)
                        }
                    )
                )
            }
            Section(macSettingsText("mac.settings.section.context_window", "Context Window")) {
                LabeledContent(macSettingsText("mac.settings.agent.context_default", "Default context window")) {
                    TextField(macSettingsText("mac.settings.agent.tokens", "Tokens"), value: $contextLimit, format: .number)
                        .frame(minWidth: 100, idealWidth: 120, maxWidth: 160)
                        .multilineTextAlignment(.trailing)
                        .onSubmit {
                            contextLimit = min(max(contextLimit, 16_384), 2_000_000)
                            MacAssistantPreferences.setContextLimit(contextLimit)
                        }
                }
                Text(macSettingsText("mac.settings.agent.context_commit", "Press Return to save. Supported range: 16,384–2,000,000 tokens."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section(macSettingsText("mac.settings.section.long_term_memory", "Long-term Memory")) {
                VStack(alignment: .leading, spacing: 7) {
                    Text(macSettingsText("mac.settings.agent.user_preferences", "User preferences"))
                        .font(.callout.weight(.medium))
                    TextEditor(text: $userPreferences)
                        .onChange(of: userPreferences) { _, value in
                            MacAssistantPreferences.setUserPreferences(value)
                        }
                        .font(.body)
                        .frame(minHeight: 120)
                        .accessibilityLabel(macSettingsText("mac.settings.agent.user_preferences", "User preferences"))
                        .background(Color(nsColor: .textBackgroundColor))
                        .clipShape(RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field))
                        .overlay {
                            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field)
                                .stroke(MacDesignColor.borderSubtle, lineWidth: 0.5)
                        }
                }
                Button {
                    openWindow(id: "memory-center")
                } label: {
                    Label(
                        macSettingsText("mac.settings.agent.open_memory_center", "Open Memory Center"),
                        systemImage: "brain.head.profile"
                    )
                }
                .buttonStyle(.link)
                Label(
                    macSettingsText("mac.settings.agent.auto_saved", "Changes are saved automatically on this Mac."),
                    systemImage: "checkmark.circle"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

        case .connections:
            Section(macSettingsText("mac.settings.server.section", "Share This Mac")) {
                managedServerControlCard
            }
            Section(macSettingsText("mac.settings.section.connected_host", "Connect to a Nola Server")) {
                Text(macSettingsText("mac.settings.host.detail", "Configure a host only when remote execution is needed."))
                    .font(.body)
                    .foregroundStyle(.secondary)
                DisclosureGroup {
                    VStack(alignment: .leading, spacing: 9) {
                        Text("nola server --host 0.0.0.0 --port 5001")
                            .font(.system(.callout, design: .monospaced, weight: .medium))
                            .textSelection(.enabled)
                            .padding(.horizontal, 12)
                            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                            .background(
                                Color(nsColor: .textBackgroundColor),
                                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field)
                            )
                        Text(macSettingsText(
                            "mac.settings.host.guide.copy",
                            "Copy the address and startup access token below. Use a trusted private network or HTTPS."
                        ))
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.top, 8)
                } label: {
                    Label(
                        macSettingsText(
                            "mac.settings.host.guide.start",
                            "On the host device, start Nola Web Host and keep it running."
                        ),
                        systemImage: "terminal"
                    )
                }
                .disclosureGroupStyle(MacRowDisclosureStyle())
                .font(.callout)
                .foregroundStyle(.secondary)
                hostConnectionStatusCard
                if case .connected(let report) = hostConnectionState {
                    messagingChannelStatusCard(report)
                }
                Button {
                    importHostConnectionSetupFromPasteboard()
                } label: {
                    Label(
                        macSettingsText("mac.settings.host.paste_setup", "Paste Connection Setup"),
                        systemImage: "doc.on.clipboard"
                    )
                }
                .buttonStyle(.bordered)
                VStack(alignment: .leading, spacing: 7) {
                    Text(macSettingsText("mac.settings.host.url", "Connected host URL"))
                        .font(.callout.weight(.semibold))
                    TextField(macSettingsText("mac.settings.host.url", "Connected host URL"), text: Binding(get: { hostURL }, set: { value in
                        guard value != hostURL else { return }
                        hostToken = ""
                        hostExpectedID = ""
                        importedHostOrigin = nil
                        hostURL = value
                        markHostConfigurationEdited()
                    }), prompt: Text("https://studio-mac.local:5001"))
                        .labelsHidden()
                        .accessibilityLabel(macSettingsText("mac.settings.host.url", "Connected host URL"))
                        .textContentType(.URL)
                        .textFieldStyle(.roundedBorder)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: .infinity)
                        .controlSize(.large)
                }
                VStack(alignment: .leading, spacing: 7) {
                    Text(macSettingsText("mac.settings.host.token", "Access token"))
                        .font(.callout.weight(.semibold))
                    SecureField(macSettingsText("mac.settings.host.token", "Access token"), text: Binding(get: { hostToken }, set: { value in
                            hostToken = value
                            markHostConfigurationEdited()
                        }),
                        prompt: Text(hasSavedHostToken && URL(string: hostURL.trimmingCharacters(in: .whitespacesAndNewlines)) == savedHostURL
                            ? "••••••••" : macSettingsText("mac.settings.host.token", "Access token")))
                        .labelsHidden()
                        .accessibilityLabel(macSettingsText("mac.settings.host.token", "Access token"))
                        .textContentType(.password)
                        .textFieldStyle(.roundedBorder)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: .infinity)
                        .controlSize(.large)
                }
                Label(
                    macSettingsText("mac.settings.host.security", "The access token is stored in Keychain and is sent only to this host."),
                    systemImage: "lock.shield"
                )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 10) {
                    if hasHostDraft {
                        Button(role: .destructive) {
                            showsRemoveHostConfirmation = true
                        } label: {
                            Label(
                                macSettingsText("mac.settings.host.remove", "Remove Connection"),
                                systemImage: "trash"
                            )
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.regular)
                    }
                    Spacer(minLength: 12)
                    Button {
                        Task { await saveAndVerifyHostConnection() }
                    } label: {
                        if case .checking = hostConnectionState {
                            Label(
                                macSettingsText("mac.settings.host.status.checking", "Verifying connection…"),
                                systemImage: "network"
                            )
                        } else {
                            Label(hostPrimaryActionTitle, systemImage: "checkmark.shield")
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(MacDesignColor.brand)
                    .controlSize(.large)
                    .disabled({
                        if case .checking = hostConnectionState { return true }
                        return !canVerifyHostDraft
                    }())
                }
            }

        case .data:
            Section(macSettingsText("mac.settings.section.data_sources", "Data Sources")) {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 220), spacing: 12)],
                    alignment: .leading,
                    spacing: 12
                ) {
                    MacSettingsDestinationCard(
                        title: macSettingsText("mac.settings.data.open_center", "Open Data Center"),
                        symbol: "square.stack.3d.up",
                        tint: .cyan
                    ) { openWindow(id: "data-center") }
                    MacSettingsDestinationCard(
                        title: macSettingsText("mac.settings.data.management", "Data Management"),
                        symbol: "externaldrive",
                        tint: .orange
                    ) { openWindow(id: "data-management") }
                }
                .padding(.vertical, 4)
            }

        case .diagnostics:
            Section(macSettingsText("mac.settings.section.logging", "Logging")) {
                Toggle(macSettingsText("mac.settings.diagnostics.enable_logging", "Enable debug logging"), isOn: $debugLoggingEnabled)
                    .onChange(of: debugLoggingEnabled) { _, _ in
                        MacDebugLoggingConfiguration.apply()
                    }
                Picker(macSettingsText("mac.settings.diagnostics.log_detail", "Log detail"), selection: $debugLoggingLevel) {
                    Text(macSettingsText("mac.settings.diagnostics.info", "Info")).tag("info")
                    Text(macSettingsText("mac.settings.diagnostics.debug", "Debug")).tag("debug")
                    Text(macSettingsText("mac.settings.diagnostics.trace", "Trace")).tag("trace")
                }
                .disabled(!debugLoggingEnabled)
                .onChange(of: debugLoggingLevel) { _, _ in
                    MacDebugLoggingConfiguration.apply()
                }
                Text(macSettingsText("mac.settings.diagnostics.privacy", "Warnings and errors are always recorded. Detailed logs stay on this Mac, rotate automatically, and exclude message content, credentials, full URLs, and file paths."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Section(macSettingsText("mac.settings.section.diagnostic_tools", "Diagnostic Tools")) {
                Button(macSettingsText("mac.settings.diagnostics.open", "Open Diagnostics")) {
                    openWindow(id: "diagnostics-center")
                }
            }

        case .notifications:
            notificationSettingsSections
        }
    }

    @ViewBuilder
    private var notificationSettingsSections: some View {
        Section(macSettingsText("mac.settings.category.notifications", "Notifications")) {
            Text(macSettingsText("mac.settings.notifications.detail", "Nola can notify you when a long-running local or connected-host agent finishes while the app is in the background."))
                .font(.callout)
                .foregroundStyle(.secondary)
            notificationAuthorizationCard
            if let notificationFeedback {
                if MacNotificationAuthorizationPolicy.needsSystemSettings(notificationAuthorizationStatus) {
                    MacInlineNotice(
                        title: macSettingsText("mac.notifications.status.disabled", "Notifications disabled"),
                        message: notificationFeedback,
                        kind: .warning,
                        actionTitle: macSettingsText("mac.notifications.permission.settings", "Open Settings"),
                        action: openNotificationSystemSettings,
                        dismiss: { self.notificationFeedback = nil }
                    )
                    .macDisclosureTransition(reduceMotion: reduceMotion)
                } else {
                    MacInlineNotice(
                        title: macSettingsText("mac.notifications.status.disabled", "Notifications disabled"),
                        message: notificationFeedback,
                        kind: .warning,
                        dismiss: { self.notificationFeedback = nil }
                    )
                    .macDisclosureTransition(reduceMotion: reduceMotion)
                }
            }
            HStack(spacing: 10) {
                Toggle(macSettingsText("mac.settings.notifications.enable", "Enable notifications"), isOn: $notificationsEnabled)
                    .disabled(isUpdatingNotificationSettings)
                    .onChange(of: notificationsEnabled) { _, enabled in
                        guard !isUpdatingNotificationSettings else { return }
                        isUpdatingNotificationSettings = true
                        Task {
                            let actual = await MacRunNotificationService.shared.setNotificationsEnabled(enabled)
                            notificationsEnabled = actual
                            await refreshNotificationAuthorization()
                            if enabled, !actual {
                                notificationFeedback = notificationPermissionFailureMessage
                            } else if !enabled {
                                notificationFeedback = nil
                            }
                            isUpdatingNotificationSettings = false
                        }
                    }
                if isUpdatingNotificationSettings {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel(macSettingsText("mac.settings.notifications.updating", "Updating notification permission"))
                }
            }
        }
        Section(macSettingsText("mac.settings.section.events", "Events")) {
            Toggle(macSettingsText("mac.settings.notifications.agent_completion", "Agent completion notifications"), isOn: $agentCompletionNotifications)
                .onChange(of: agentCompletionNotifications) { _, enabled in
                    Task {
                        let actual = await MacRunNotificationService.shared.setAgentCompletionEnabled(enabled)
                        if actual != enabled {
                            agentCompletionNotifications = actual
                            if enabled {
                                notificationFeedback = notificationPermissionFailureMessage
                            }
                        }
                        await refreshNotificationAuthorization()
                    }
                }
                .disabled(!notificationsEnabled)
            Toggle(macSettingsText("mac.settings.notifications.step_guidance", "Step guidance"), isOn: $stepGuidanceEnabled)
                .disabled(!notificationsEnabled)
            Toggle(macSettingsText("mac.settings.notifications.calendar", "Aggregate calendar notifications"), isOn: $aggregateCalendarNotifications)
                .disabled(!notificationsEnabled)
        }
        Section(macSettingsText("mac.settings.section.delivery", "Delivery")) {
            Picker(macSettingsText("mac.settings.notifications.frequency", "Notification frequency"), selection: $notificationFrequency) {
                Text(macSettingsText("mac.settings.notifications.daily", "Daily")).tag("daily")
                Text(macSettingsText("mac.settings.notifications.weekly", "Weekly")).tag("weekly")
                Text(macSettingsText("mac.settings.notifications.immediate", "Immediate")).tag("immediate")
            }
            .disabled(!notificationsEnabled)
        }
        Section(macSettingsText("mac.settings.section.related_tools", "Related Tools")) {
            Button(macSettingsText("mac.settings.notifications.open_automations", "Open Automations")) {
                openWindow(id: "automation-center")
            }
            Button(macSettingsText("mac.settings.notifications.open_activity", "Open Activity Center")) {
                openWindow(id: "activity-center")
            }
        }
    }

    private var notificationAuthorizationCard: some View {
        HStack(spacing: 10) {
            Image(systemName: notificationAuthorizationSymbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(notificationAuthorizationTint)
                .frame(width: 28, height: 28)
                .background(
                    notificationAuthorizationTint.opacity(0.10),
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control)
                )

            VStack(alignment: .leading, spacing: 2) {
                Text(notificationAuthorizationTitle)
                    .font(.system(size: 12.5, weight: .semibold))
                Text(notificationAuthorizationDetail)
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 12)

            if isUpdatingNotificationSettings {
                ProgressView()
                    .controlSize(.small)
            } else if MacNotificationAuthorizationPolicy.needsSystemSettings(notificationAuthorizationStatus) {
                Button(
                    macSettingsText("mac.notifications.permission.settings", "Open Settings"),
                    action: openNotificationSystemSettings
                )
                .controlSize(.small)
            }
        }
        .padding(10)
        .background(
            notificationAuthorizationTint.opacity(0.055),
            in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel)
        )
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel)
                .stroke(notificationAuthorizationTint.opacity(0.14), lineWidth: 0.5)
        }
        .animation(
            reduceMotion ? nil : MacDesignTokens.Motion.responsive,
            value: notificationAuthorizationStatus.rawValue
        )
    }

    private var notificationAuthorizationTitle: String {
        switch notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral:
            macSettingsText("mac.notifications.status.enabled", "Notifications enabled")
        case .denied:
            macSettingsText("mac.notifications.status.disabled", "Notifications disabled")
        case .notDetermined:
            macSettingsText("mac.notifications.status.unconfigured", "Notifications not configured")
        @unknown default:
            macSettingsText("mac.notifications.status.unavailable", "Notification status unavailable")
        }
    }

    private var notificationAuthorizationDetail: String {
        switch notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral:
            macSettingsText("mac.settings.notifications.permission_enabled_detail", "Nola can deliver selected events while the app is in the background.")
        case .denied:
            macSettingsText("mac.settings.notifications.permission_denied_detail", "macOS is blocking delivery. Change the permission in System Settings to enable it.")
        case .notDetermined:
            macSettingsText("mac.settings.notifications.permission_not_requested_detail", "Turn on notifications below to request permission from macOS.")
        @unknown default:
            macSettingsText("mac.settings.notifications.permission_unknown_detail", "Nola could not read the current macOS notification permission.")
        }
    }

    private var notificationAuthorizationSymbol: String {
        switch notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral: "checkmark.circle.fill"
        case .denied: "bell.slash.fill"
        case .notDetermined: "bell.badge"
        @unknown default: "exclamationmark.triangle.fill"
        }
    }

    private var notificationAuthorizationTint: Color {
        switch notificationAuthorizationStatus {
        case .authorized, .provisional, .ephemeral: .green
        case .denied: .orange
        case .notDetermined: MacDesignColor.brand
        @unknown default: .secondary
        }
    }

    private var notificationPermissionFailureMessage: String {
        if MacNotificationAuthorizationPolicy.needsSystemSettings(notificationAuthorizationStatus) {
            return macSettingsText(
                "mac.settings.notifications.permission_denied_feedback",
                "macOS denied notification access. Event controls remain unavailable until permission is enabled in System Settings."
            )
        }
        return macSettingsText(
            "mac.settings.notifications.permission_failed_feedback",
            "Nola could not enable notification delivery. No notification preference was applied."
        )
    }

    private func refreshNotificationAuthorization() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        await MainActor.run {
            notificationAuthorizationStatus = settings.authorizationStatus
            if !MacNotificationAuthorizationPolicy.isGranted(settings.authorizationStatus) {
                notificationsEnabled = false
            }
        }
    }

    private func synchronizeNotificationAuthorization() async {
        isUpdatingNotificationSettings = true
        await refreshNotificationAuthorization()
        isUpdatingNotificationSettings = false
    }

    private func openNotificationSystemSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") else {
            return
        }
        NSWorkspace.shared.open(url)
    }
}

struct MacSettingsDefaultChatModelSummary: View {
    @ObservedObject var preference: MacDefaultChatModelPresentation
    let openModel: (String?) -> Void

    var body: some View {
        LabeledContent(String(localized: "Default chat model", table: "ModelCenter")) {
            Text(preference.title)
                .lineLimit(2)
                .textSelection(.enabled)
                .accessibilityIdentifier("settings.default-chat-model.summary")
        }
        Text(String(localized: "Used by conversations that inherit the default. Running requests keep their current model.", table: "ModelCenter"))
            .font(.caption)
            .foregroundStyle(.secondary)
        Button(String(localized: "Manage Default Chat Model", table: "ModelCenter")) {
            let profileID: String?
            switch preference.resolution {
            case let .resolved(id, _), let .unavailable(id, _, _): profileID = id
            case .noAutomaticCandidate, .invalidConfiguration: profileID = nil
            }
            let configured = preference.catalog.entries.first {
                $0.candidate.profileID == profileID && $0.option.source == .configured
            }
            openModel(configured?.candidate.profileID)
        }
        .accessibilityIdentifier("settings.default-chat-model.manage")
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { notification in
            guard let databaseURL = notification.object as? URL,
                  databaseURL == preference.store.application.databaseURL else { return }
            preference.reload()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in
            preference.reload()
        }
    }
}

private struct MacSettingsSidebarRow: View {
    let category: MacSettingsCategory
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                category.image.renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 15, height: 15)
                    .foregroundStyle(isSelected ? Color.accentColor : Color.secondary)
                    .frame(width: 28, height: 28)
                    .background(
                        Color.primary.opacity(isSelected ? 0.055 : 0.025),
                        in: Circle()
                    )
                Text(category.title)
                    .font(.system(size: 14, weight: .medium))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 40)
            .background(!isSelected && isHovered ? MacDesignColor.hover : .clear,
                        in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control))
            .macSelectionHighlight(isSelected: isSelected)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            if reduceMotion {
                isHovered = hovering
            } else {
                withAnimation(.easeOut(duration: MacDesignTokens.Motion.hover)) {
                    isHovered = hovering
                }
            }
        }
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: isSelected)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

private struct MacSettingsDestinationCard: View {
    let title: String
    let symbol: String
    let tint: Color
    let action: () -> Void
    @State private var isHovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button(action: action) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: symbol)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(tint)
                    .frame(width: 30, height: 30)
                    .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))

                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(12)
            .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            .background(
                isHovered ? MacDesignColor.hover : Color.primary.opacity(0.025),
                in: RoundedRectangle(cornerRadius: 11)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 11)
                    .stroke(isHovered ? tint.opacity(0.28) : MacDesignColor.borderSubtle, lineWidth: 0.5)
            }
            .contentShape(RoundedRectangle(cornerRadius: 11))
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            if reduceMotion {
                isHovered = hovering
            } else {
                withAnimation(.easeOut(duration: MacDesignTokens.Motion.hover)) {
                    isHovered = hovering
                }
            }
        }
    }
}

/// Do not construct conversation views until the canonical store is reachable:
/// a failed connection is not an empty conversation list.
struct MacApplicationStartupGate<Content: View>: View {
    @State private var ready = false
    @State private var failure: MacApplicationServiceError?
    @State private var attempt = 0
    @State private var recoveryRequested = false
    private let applicationPort: MacApplicationPort
    private let content: () -> Content

    init(
        applicationPort: MacApplicationPort = .shared,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.applicationPort = applicationPort
        self.content = content
    }

    var body: some View {
        Group {
            if ready {
                content()
            } else if let failure {
                VStack(alignment: .leading, spacing: 16) {
                    Label(
                        isIncompatible(failure)
                            ? String(localized: "mac.startup.incompatible.title", table: "Startup")
                            : String(localized: "mac.startup.failure.title"),
                        systemImage: isIncompatible(failure)
                            ? "externaldrive.badge.exclamationmark"
                            : "exclamationmark.triangle"
                    )
                        .font(.title2)
                    Text(
                        isIncompatible(failure)
                            ? String(localized: "mac.startup.incompatible.message", table: "Startup")
                            : String(localized: "mac.startup.failure.message")
                    )
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if isIncompatible(failure) {
                        Text(
                            String(
                                format: String(
                                    localized: "mac.startup.incompatible.archive_location",
                                    table: "Startup"
                                ),
                                recoveryArchiveRoot.path
                            )
                        )
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    }
                    DisclosureGroup("mac.startup.failure.details") {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 12) {
                                Text(verbatim: failure.localizedDescription)
                                Text(verbatim: applicationPort.databaseURL.path)
                                    .font(.caption.monospaced())
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                            .padding(.vertical, 8)
                        }
                        .frame(maxHeight: 160)
                    }
                    .disclosureGroupStyle(MacRowDisclosureStyle())
                    HStack(spacing: 12) {
                        if isIncompatible(failure) {
                            Button {
                                recoveryRequested = true
                                self.failure = nil
                                attempt += 1
                            } label: {
                                Text("mac.startup.incompatible.recover", tableName: "Startup")
                                    .font(.body.weight(.medium))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 18)
                                    .padding(.vertical, 9)
                                    .background(
                                        Color.accentColor,
                                        in: RoundedRectangle(cornerRadius: 8)
                                    )
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("mac.startup.recover")
                            .keyboardShortcut(.defaultAction)
                        }
                        if isIncompatible(failure) {
                            Button {
                                self.failure = nil
                                attempt += 1
                            } label: {
                                Text("mac.startup.failure.retry")
                                    .font(.body.weight(.medium))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 9)
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("mac.startup.retry")
                        } else {
                            Button {
                                self.failure = nil
                                attempt += 1
                            } label: {
                                Text("mac.startup.failure.retry")
                                    .font(.body.weight(.medium))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 18)
                                    .padding(.vertical, 9)
                                    .background(
                                        Color.accentColor,
                                        in: RoundedRectangle(cornerRadius: 8)
                                    )
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("mac.startup.retry")
                            .keyboardShortcut(.defaultAction)
                        }
                        if !isIncompatible(failure) {
                            Spacer(minLength: 0)
                        }
                    }
                }
                .padding(32)
                .frame(maxWidth: 640)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .accessibilityLabel("Nola")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .task(id: attempt) {
            let port = applicationPort
            let shouldRecover = recoveryRequested
            recoveryRequested = false
            let startupFailure = await Task.detached(priority: .userInitiated) {
                () -> MacApplicationServiceError? in
                do {
                    if shouldRecover {
                        _ = try port.archiveIncompatibleStoreAndPrepare()
                    } else {
                        try port.prepare()
                    }
                    try MacBuiltInSkillRegistry.synchronizeCatalog(store: port)
                    return nil
                } catch let error as MacApplicationServiceError {
                    return error
                } catch {
                    return .launchFailed(error.localizedDescription)
                }
            }.value
            guard !Task.isCancelled else { return }
            failure = startupFailure
            ready = startupFailure == nil
        }
    }

    private func isIncompatible(_ error: MacApplicationServiceError) -> Bool {
        if case .incompatibleStore = error { return true }
        return false
    }

    private var recoveryArchiveRoot: URL {
        applicationPort.databaseURL.deletingLastPathComponent()
            .appendingPathComponent("Recovered Stores", isDirectory: true)
    }
}
