// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Captures deterministic Debug UI snapshots without Screen Recording permission.

#if DEBUG
import AppKit
import SwiftUI

@MainActor
enum MacVisualSnapshotHarness {
    enum Page: String, CaseIterable {
        case chat
        case modelCenter = "model-center"
        case localModels = "local-models"
        case settings
        case dataCenter = "data-center"
        case capabilities
        case automations
        case activity
        case extensions
        case diagnostics
        case dataManagement = "data-management"
        case promptLibrary = "prompt-library"
        case webSources = "web-sources"
        case health
        case notifications
        case medication
        case modelTest = "model-test"
    }

    static func captureIfRequested() async {
        var environment = ProcessInfo.processInfo.environment
        for key in [
            "NOLA_CAPTURE_UI",
            "NOLA_CAPTURE_APPEARANCE",
            "NOLA_CAPTURE_WIDTH",
            "NOLA_CAPTURE_HEIGHT",
            "NOLA_CAPTURE_PAGE",
            "NOLA_CAPTURE_SECTION",
            "NOLA_CAPTURE_HOST_STATE",
            "NOLA_CAPTURE_INSPECTOR_TAB",
            "NOLA_CAPTURE_ATTACHMENT",
            "NOLA_CAPTURE_BROWSER_PREVIEW_URL",
            "NOLA_CAPTURE_BROWSER_PAGE_URL",
            "NOLA_CAPTURE_EXIT",
        ] {
            if let value = currentEnvironmentValue(for: key) {
                environment[key] = value
            } else {
                environment.removeValue(forKey: key)
            }
        }
        guard let path = environment["NOLA_CAPTURE_UI"], !path.isEmpty else { return }

        let colorScheme: ColorScheme = environment["NOLA_CAPTURE_APPEARANCE"] == "dark" ? .dark : .light
        let appearance = NSAppearance(
            named: colorScheme == .dark ? .darkAqua : .aqua
        )
        NSApplication.shared.appearance = appearance
        let visibleFrame = NSScreen.main?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1_280, height: 820)
        let requestedWidth = environment["NOLA_CAPTURE_WIDTH"].flatMap(Double.init)
        let requestedHeight = environment["NOLA_CAPTURE_HEIGHT"].flatMap(Double.init)
        let size = CGSize(
            width: max(620, min(requestedWidth ?? 1_280, visibleFrame.width - 48)),
            height: max(520, min(requestedHeight ?? 820, visibleFrame.height - 48))
        )
        let page = Page(rawValue: environment["NOLA_CAPTURE_PAGE"] ?? "chat") ?? .chat
        let settingsCategoryKey = "nola.mac.settings.category"
        let previousSettingsCategory = UserDefaults.standard.object(forKey: settingsCategoryKey)
        if page == .settings,
           let section = environment["NOLA_CAPTURE_SECTION"],
           !section.isEmpty {
            UserDefaults.standard.set(section, forKey: settingsCategoryKey)
        }
        let root = content(
            for: page,
            inspectorCaptureTab: environment["NOLA_CAPTURE_INSPECTOR_TAB"],
            hostState: snapshotHostState(environment["NOLA_CAPTURE_HOST_STATE"])
        )
            .frame(width: size.width, height: size.height)
            .environment(\.colorScheme, colorScheme)

        let hostingView = NSHostingView(rootView: root)
        hostingView.appearance = appearance
        hostingView.frame = CGRect(origin: .zero, size: size)
        let window = NSWindow(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = windowTitle(for: page)
        window.appearance = appearance
        window.isReleasedWhenClosed = false
        window.titlebarAppearsTransparent = true
        window.contentView = hostingView
        window.center()
        NSApplication.shared.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        window.makeMain()
        window.makeKey()
        window.orderFrontRegardless()

        if MacProcessRuntimePolicy.isRunningTests {
            // XCTest owns activation, so waiting for the fixture window to become the
            // process key window adds the full timeout to every page without improving
            // the offscreen NSHostingView capture. Give SwiftUI two update turns and a
            // short async settle window instead.
            await Task.yield()
            hostingView.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(160))
            await Task.yield()
        } else {
            let activationDeadline = ContinuousClock.now + .seconds(2)
            while (!NSApplication.shared.isActive || !window.isKeyWindow),
                  ContinuousClock.now < activationDeadline {
                try? await Task.sleep(for: .milliseconds(40))
            }
            try? await Task.sleep(for: .milliseconds(900))
        }
        hostingView.layoutSubtreeIfNeeded()
        let representation = hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds)
            ?? NSBitmapImageRep(
                bitmapDataPlanes: nil,
                pixelsWide: Int(size.width * window.backingScaleFactor),
                pixelsHigh: Int(size.height * window.backingScaleFactor),
                bitsPerSample: 8,
                samplesPerPixel: 4,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: .deviceRGB,
                bytesPerRow: 0,
                bitsPerPixel: 0
            )
        representation?.size = hostingView.bounds.size
        if let representation {
            hostingView.cacheDisplay(in: hostingView.bounds, to: representation)
        }
        if let data = representation?.representation(using: .png, properties: [:]) {
            try? data.write(to: URL(fileURLWithPath: path), options: .atomic)
        }
        if page == .settings {
            if let previousSettingsCategory {
                UserDefaults.standard.set(previousSettingsCategory, forKey: settingsCategoryKey)
            } else {
                UserDefaults.standard.removeObject(forKey: settingsCategoryKey)
            }
        }
        await finish(window: window, environment: environment)
    }

    private static func currentEnvironmentValue(for key: String) -> String? {
        key.withCString { pointer in
            getenv(pointer).map { String(cString: $0) }
        }
    }

    private static func content(
        for page: Page,
        inspectorCaptureTab: String? = nil,
        hostState: MacRemoteHostConnectionState? = nil
    ) -> AnyView {
        switch page {
        case .chat:
            return AnyView(MacChatWorkspaceSnapshotContent(
                inspectorTab: inspectorCaptureTab,
                canonicalRepository: MacCanonicalConversationRepository(store: MacApplicationPort.shared)
            ))
        case .modelCenter:
            return AnyView(MacModelCenterView(remoteHost: MacRemoteHostService.shared))
        case .localModels:
            let navigation = MacModelCenterNavigationState()
            navigation.showLocalModels()
            return AnyView(MacModelCenterView(remoteHost: MacRemoteHostService.shared, navigation: navigation))
        case .settings:
            return AnyView(MacSettingsView(snapshotHostState: hostState))
        case .dataCenter:
            let directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("nola-data-center-visual-snapshot-\(UUID().uuidString)", isDirectory: true)
            let registry = MacDataSourceRegistry(
                storeURL: directory.appendingPathComponent("source-registry.json")
            )
            if ProcessInfo.processInfo.environment["NOLA_CAPTURE_SECTION"] == "usage" {
                registry.recordUsage(
                    sourceID: MacDataSourceRegistry.platformSourceID("contacts"),
                    sourceTitle: String(localized: "Contacts"),
                    consumer: "chat",
                    purpose: "answer",
                    conversationID: "visual-snapshot-conversation",
                    disclosedToCloud: true
                )
                registry.recordUsage(
                    sourceID: MacDataSourceRegistry.platformSourceID("health"),
                    sourceTitle: String(localized: "Health"),
                    consumer: "automation",
                    purpose: "scheduled summary",
                    conversationID: nil,
                    disclosedToCloud: false
                )
            }
            let webSources = MacWebSourceStore(
                storeURL: directory.appendingPathComponent("web-sources.json"),
                registry: registry
            )
            let service = MacPlatformDataService(registry: registry)
            return AnyView(MacDataCenterView(
                service: service,
                registry: registry,
                webSources: webSources,
                initialSection: ProcessInfo.processInfo.environment["NOLA_CAPTURE_SECTION"]
            ))
        case .capabilities:
            return AnyView(MacCapabilityCenterView())
        case .automations:
            return AnyView(MacAutomationCenterView())
        case .activity:
            return AnyView(MacActivityCenterView())
        case .extensions:
            return AnyView(MacExtensionsCenterView(remoteHost: MacRemoteHostService.shared))
        case .diagnostics:
            return AnyView(MacDiagnosticsCenterView())
        case .dataManagement:
            return AnyView(MacDataManagementView())
        case .promptLibrary:
            return AnyView(MacPromptLibraryView())
        case .webSources:
            return AnyView(MacWebSourcesView())
        case .health:
            return AnyView(MacHealthSummaryView())
        case .notifications:
            return AnyView(MacNotificationCenterView())
        case .medication:
            return AnyView(MacMedicationSearchView())
        case .modelTest:
            return AnyView(MacModelTestView())
        }
    }

    private static func snapshotHostState(_ rawValue: String?) -> MacRemoteHostConnectionState? {
        switch rawValue {
        case "not-configured":
            return .notConfigured
        case "unverified":
            return .unverified
        case "checking":
            return .checking
        case "connected":
            return .connected(MacRemoteHostConnectionReport(
                hostID: "studio-host",
                displayName: "Studio Mac",
                origin: "studio-mac.local",
                checkedAt: .now,
                messagingWorkerEnabled: true,
                messagingChannelCount: 3,
                messagingConnectedCount: 2,
                messagingManagementURL: URL(string: "https://studio-mac.local:5001/settings#channels")
            ))
        case "failed":
            return .failed(String(localized: "mac.host.error.unreachable"))
        default:
            return nil
        }
    }

    private static func windowTitle(for page: Page) -> String {
        switch page {
        case .chat: return "Nola"
        case .modelCenter, .localModels: return String(localized: "Model Center")
        case .settings: return String(localized: "Settings")
        case .dataCenter: return String(localized: "Data Center")
        case .capabilities: return String(localized: "System Access")
        case .automations: return String(localized: "Automations")
        case .activity: return String(localized: "Activity")
        case .extensions: return String(localized: "Extensions")
        case .diagnostics: return String(localized: "Runtime Diagnostics")
        case .dataManagement: return String(localized: "Data Management")
        case .promptLibrary: return String(localized: "Prompt Library")
        case .webSources: return String(localized: "Web Sources")
        case .health: return String(localized: "iPhone Health")
        case .notifications: return String(localized: "Notifications")
        case .medication: return String(localized: "Medication Lookup")
        case .modelTest: return String(localized: "Model Test")
        }
    }

    private static func finish(window: NSWindow, environment: [String: String]) async {
        window.orderOut(nil)
        window.close()
        await Task.yield()
        if environment["NOLA_CAPTURE_EXIT"] == "1" {
            NSApplication.shared.terminate(nil)
        }
    }
}
#endif
