// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Prevents secondary macOS workspace pages from shipping partial localization or detached navigation.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

final class MacSecondaryPageLocalizationTests: XCTestCase {
    private let requiredLocales = ["zh-Hans", "zh-Hant", "ja", "ko", "fr", "de", "es", "ru"]

    private let localizableKeysByPage: [String: [String]] = [
        "Data Center": [
            "Data Center",
            "Control what Nola can perceive and how conversations may use it.",
            "Source Health",
            "All connected sources passed a real read test.",
            "%d sources need attention",
            "Every Conversation",
            "Per Conversation",
            "Ask When Relevant",
            "Blocked",
            "Verified",
            "Validation failed",
            "Reader",
            "Data Format",
            "Redacted Sample",
            "Test Connection",
            "Never Tested",
            "No validated contract. Test the connection before using this source.",
            "Recent Usage",
            "No source usage yet",
            "Usage appears after a conversation or automation reads a source.",
            "Web Sources",
        ],
        "Web Sources": [
            "Sources available to retrieval context",
            "Add Source",
            "No Web Sources",
            "Add JSON, RSS, Atom, or text sources for retrieval context.",
            "Select a source to review its cached content and refresh status.",
            "Web Source Error",
            "New Web Source",
            "Edit Web Source",
            "Testing connection…",
            "Connection verified",
            "The source response exceeds the 2 MB limit.",
            "The source returned no readable content.",
            "No Cached Content",
            "Refresh this source to fetch its current content.",
            "Run History",
        ],
        "iPhone Health": [
            "Synced from %@ · %@",
            "iCloud Health sync requires a signed build with the KVS capability",
            "Waiting for an aggregate Health snapshot from Nola on iPhone",
            "Sync Now",
            "Active Energy",
            "Resting Heart Rate",
            "SEVEN-DAY STEPS",
            "No Synced Health Summary",
            "iCloud Sync Unavailable",
            "Open Nola’s Health dashboard on iPhone after granting Health access. The next successful refresh publishes an aggregate snapshot through iCloud.",
            "Run a signed Nola build with the iCloud key-value store capability to receive Health summaries from iPhone.",
            "May be outdated",
            "Up to date",
        ],
        "Automations": [
            "Automations",
            "Automation Count",
            "Recurring work, explicit workspaces, and visible run history",
            "New Automation",
            "Edit Automation",
            "No Automations",
            "Schedule a prompt to run daily or weekly in a selected workspace.",
            "Run Now",
            "View Run History",
            "Reveal Log in Finder",
            "Agent prompt",
            "Automation Error",
        ],
        "Activity": [
            "Activity",
            "Agent and automation results with local run details",
            "Search activity",
            "Activity Center",
            "Agent and automation results stay available here.",
            "Open Conversation",
            "Clear History",
            "Clear Activity History?",
            "This removes the local history list. Conversation and automation logs are not deleted.",
            "Started",
            "Duration",
            "Result",
            "OUTPUT",
            "%lld sec",
            "%lld min %lld sec",
        ],
        "Capabilities": [
            "System Access",
            "System access and tools available to Nola on this Mac",
        ],
        "Host Connection": [
            "mac.settings.host.status.not_configured",
            "mac.settings.host.status.unverified",
            "mac.settings.host.status.checking",
            "mac.settings.host.status.connected",
            "mac.settings.host.status.failed",
            "mac.settings.host.detail.not_configured",
            "mac.settings.host.detail.unverified",
            "mac.settings.host.detail.checking",
            "mac.settings.host.security",
            "mac.settings.host.guide.start",
            "mac.settings.host.guide.copy",
            "mac.settings.host.remove",
            "mac.settings.host.remove.confirm.title",
            "mac.settings.host.remove.confirm.message",
            "mac.settings.host.remove.confirm.action",
            "mac.settings.host.save_verify",
            "mac.settings.host.verify_again",
            "mac.settings.host.retry",
            "mac.host.error.timeout",
            "mac.host.error.unreachable",
            "mac.host.error.unauthorized",
            "mac.host.error.http",
            "mac.host.menu.status",
            "mac.host.status.not_configured",
            "mac.host.status.checking",
            "mac.host.status.offline",
            "mac.host.status.connected",
            "mac.host.detail.not_configured",
            "mac.host.detail.unverified",
            "mac.host.detail.checking",
            "mac.host.detail.failed",
            "mac.host.action.retry",
            "mac.host.action.edit",
            "mac.host.action.configure",
        ],
        "Extensions": [
            "Browse, configure, and edit tools available to this Mac.",
            "Chrome Extension",
            "Ready to connect",
        ],
        "Data Management": [
            "Data Management",
            "Back up or reset data owned by Nola for Mac",
            "Backup and Restore",
            "Export Backup",
            "Restore Backup",
            "Clear Application Data…",
            "Data Management Error",
        ],
        "Diagnostics": [
            "Runtime Diagnostics",
            "Runtime status, logs, and support export.",
            "Detailed Logging",
            "Warnings & Errors",
            "Search logs",
            "No log entries match current filter",
            "Waiting for log entries",
            "Detailed logging is off.",
        ],
        "Prompt Library": [
            "Prompt Library",
            "Create reusable instructions without adding controls to the composer.",
            "Search templates",
            "Delete Prompt Template?",
            "This removes the custom template from this Mac. This action cannot be undone.",
            "%d tokens",
            "Discard unsaved changes?",
            "Discard Changes",
            "Keep Editing",
            "Your prompt template has not been saved.",
        ],
        "Local Models": [
            "Local Models",
            "Manage the shared Apple-client MLX model cache",
            "Delete Local Model?",
            "The downloaded model files will be removed from this Mac.",
            "Download Source",
            "Estimated download · %@",
            "SELECTED",
            "Use for Chat",
            "Local model storage",
        ],
        "Model Center": [
            "Model Center",
            "Models stay scoped to their execution environment",
            "Manage Local Models…",
            "Connected Host",
            "Chat Only",
            "Available model",
        ],
    ]

    private let modelCenterKeys = [
        "API models",
        "Choose, configure, and start local models independently.",
        "Estimated download",
        "Local agent runtime",
        "Local MLX inference parameters",
        "Local models",
        "Native MLX inference",
        "API request parameters",
        "Parameters apply the next time the model starts.",
        "Remote Agent execution",
        "Running",
        "Select",
        "Selected",
        "Selection does not start the model. Start it only when you need local inference.",
        "Start",
        "Start the selected local model before using it in chat.",
        "Stop",
        "The selected model is running locally and ready for chat.",
        "Configured",
        "Configure a host only when remote execution is needed.",
        "OpenAI-compatible responses without local or host tools",
    ]

    func testSecondaryPagesTranslateCriticalCopyInEverySupportedLocale() throws {
        let catalog = try loadCatalog(named: "Localizable")

        for page in localizableKeysByPage.keys.sorted() {
            for key in localizableKeysByPage[page] ?? [] {
                try assertTranslated(key, page: page, catalog: catalog)
            }
        }
    }

    func testModelCenterTableTranslatesExecutionSpecificCopy() throws {
        let catalog = try loadCatalog(named: "ModelCenter")

        for key in modelCenterKeys {
            try assertTranslated(key, page: "Model Center", catalog: catalog)
        }
    }

    func testLocalizedFormatPlaceholdersMatchSourceKeys() async throws {
        let catalog = try loadCatalog(named: "Localizable")
        let keys = localizableKeysByPage.values.flatMap { $0 }.filter { $0.contains("%") }

        for key in keys {
            let sourcePlaceholders = placeholders(in: key)
            for locale in requiredLocales {
                let value = try localizedValue(for: key, locale: locale, catalog: catalog)
                XCTAssertEqual(
                    placeholders(in: value),
                    sourcePlaceholders,
                    "Placeholder mismatch for \(key) [\(locale)]"
                )
            }
        }
    }

    func testSecondaryPageViewsDoNotRenderLiteralConditionalStrings() throws {
        let files = [
            "apps/macos/UI/MacDataManagementView.swift",
            "apps/macos/UI/MacDiagnosticsCenterView.swift",
            "apps/macos/UI/MacPromptLibraryView.swift",
            "apps/macos/UI/MacWebSourcesView.swift",
            "apps/macos/UI/MacHealthSummaryView.swift",
            "apps/macos/UI/MacAutomationCenterView.swift",
            "apps/macos/UI/MacActivityCenterView.swift",
        ]
        let expression = try NSRegularExpression(
            pattern: #"(?:Text|Label|ContentUnavailableView)\s*\([^,\n]*(?<!\?)\?(?!\?)\s*\""#
        )

        for relativePath in files {
            let source = try String(contentsOf: fixtureURL(for: relativePath), encoding: .utf8)
            let range = NSRange(source.startIndex..<source.endIndex, in: source)
            XCTAssertNil(
                expression.firstMatch(in: source, range: range),
                "Conditional UI copy must call String(localized:) in \(relativePath)"
            )
        }
    }

    private func loadCatalog(named name: String) throws -> [String: Any] {
        let url = try fixtureURL(for: "apps/macos/\(name).xcstrings")
        let data = try Data(contentsOf: url)
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let strings = root["strings"] as? [String: Any] else {
            XCTFail("Invalid string catalog: \(url.path)")
            return [:]
        }
        return strings
    }

    private func fixtureURL(for repositoryPath: String) throws -> URL {
        let repositoryRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let liveSource = repositoryRoot.appendingPathComponent(repositoryPath)
        if FileManager.default.fileExists(atPath: liveSource.path) {
            return liveSource
        }

        let resourceName = URL(fileURLWithPath: repositoryPath).lastPathComponent
        return try XCTUnwrap(
            Bundle(for: Self.self).url(forResource: resourceName, withExtension: "txt"),
            "Missing source for \(repositoryPath)"
        )
    }

    private func assertTranslated(
        _ key: String,
        page: String,
        catalog: [String: Any]
    ) throws {
        for locale in requiredLocales {
            let value = try localizedValue(for: key, locale: locale, catalog: catalog)
            XCTAssertFalse(
                value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                "Empty translation for \(page): \(key) [\(locale)]"
            )
        }
    }

    private func localizedValue(
        for key: String,
        locale: String,
        catalog: [String: Any]
    ) throws -> String {
        guard let entry = catalog[key] as? [String: Any] else {
            XCTFail("Missing localization key: \(key)")
            return ""
        }
        guard let localizations = entry["localizations"] as? [String: Any],
              let localization = localizations[locale] as? [String: Any],
              let stringUnit = localization["stringUnit"] as? [String: Any],
              stringUnit["state"] as? String == "translated",
              let value = stringUnit["value"] as? String else {
            XCTFail("Missing translated value for \(key) [\(locale)]")
            return ""
        }
        return value
    }

    private func placeholders(in value: String) -> [String] {
        let expression = try? NSRegularExpression(pattern: #"%(?:\d+\$)?(?:@|lld|ld|d|f)"#)
        let range = NSRange(value.startIndex..<value.endIndex, in: value)
        return expression?.matches(in: value, range: range).compactMap { match in
            guard let tokenRange = Range(match.range, in: value) else { return nil }
            return String(value[tokenRange])
        } ?? []
    }
}

@MainActor
final class MacSecondaryPageVisualEndToEndTests: XCTestCase {
    func testDataCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(
            MacDataCenterView(),
            name: "mac-data-center-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacDataCenterView(),
            name: "mac-data-center-compact",
            size: CGSize(width: 520, height: 720)
        )
        try await assertRenderedPage(
            MacDataCenterView(initialSection: "sources"),
            name: "mac-data-center-sources-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacDataCenterView(initialSection: "sources"),
            name: "mac-data-center-sources-compact",
            size: CGSize(width: 520, height: 720)
        )
    }

    func testCapabilityCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(
            MacCapabilityCenterView(),
            name: "capabilities-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacCapabilityCenterView(),
            name: "capabilities-compact",
            size: CGSize(width: 680, height: 720)
        )
    }

    func testDiagnosticsCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(
            MacDiagnosticsCenterView(),
            name: "diagnostics-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacDiagnosticsCenterView(),
            name: "diagnostics-compact",
            size: CGSize(width: 520, height: 720)
        )
    }

    func testExtensionsCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(
            MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(), kinds: [.skill, .mcp]),
            name: "extensions-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(), kinds: [.skill, .mcp]),
            name: "extensions-compact",
            size: CGSize(width: 520, height: 720)
        )
    }

    func testAutomationCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(
            MacAutomationCenterView(),
            name: "mac-automation-center-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacAutomationCenterView(),
            name: "mac-automation-center-compact",
            size: CGSize(width: 520, height: 720)
        )
        try await assertRenderedPage(
            MacAutomationLiveRunStatus(detail: "Reading the selected data source and preparing the next tool call"),
            name: "mac-automation-live-status-regular",
            size: CGSize(width: 620, height: 160)
        )
        try await assertRenderedPage(
            MacAutomationLiveRunStatus(detail: "Reading the selected data source and preparing the next tool call"),
            name: "mac-automation-live-status-compact",
            size: CGSize(width: 320, height: 180)
        )
    }

    func testActivityCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacActivityCenterView(), name: "activity-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacActivityCenterView(), name: "activity-compact", size: CGSize(width: 520, height: 720))
    }

    func testWebSourcesRenderInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacWebSourcesView(), name: "web-sources-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacWebSourcesView(), name: "web-sources-compact", size: CGSize(width: 680, height: 720))
    }

    func testDataManagementRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacDataManagementView(), name: "data-management-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacDataManagementView(), name: "data-management-compact", size: CGSize(width: 520, height: 720))
    }

    func testModelCenterRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost()), name: "model-center-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost()), name: "model-center-compact", size: CGSize(width: 680, height: 720))
        let apiNavigation = MacModelCenterNavigationState()
        apiNavigation.showRemoteModels()
        try await assertRenderedPage(
            MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: apiNavigation),
            name: "model-center-api-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: apiNavigation),
            name: "model-center-api-compact",
            size: CGSize(width: 680, height: 720)
        )
    }

    func testLocalModelManagerRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        let navigation = MacModelCenterNavigationState()
        navigation.showLocalModels()
        try await assertRenderedPage(
            MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: navigation),
            name: "local-model-manager-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: navigation),
            name: "local-model-manager-compact",
            size: CGSize(width: 680, height: 720)
        )
    }

    func testRemoteModelManagerRendersMultipleEndpointsAtRegularAndCompactWidths() async throws {
        let endpoints = [
            MacRemoteEndpointConfiguration(
                id: "private",
                name: "Private Gateway",
                baseURL: "https://private.example.test/v1",
                order: 0,
                models: [
                    MacRemoteModelConfiguration(
                        id: "private-primary",
                        modelName: "reasoning-large",
                        displayName: "Reasoning Large",
                        priority: 90,
                        capabilities: [.chat, .issueRepair, .sensitiveData]
                    ),
                    MacRemoteModelConfiguration(
                        id: "private-search",
                        modelName: "search-fast",
                        displayName: "Search Fast",
                        priority: 70,
                        order: 1,
                        capabilities: [.retrieval]
                    ),
                ]
            ),
            MacRemoteEndpointConfiguration(
                id: "cloud",
                name: "Cloud API",
                baseURL: "https://api.example.test/v1",
                order: 1,
                models: [
                    MacRemoteModelConfiguration(
                        id: "cloud-chat",
                        modelName: "chat-small",
                        displayName: "Chat Small",
                        priority: 50,
                        capabilities: [.chat]
                    ),
                ]
            ),
        ]
        try await assertRenderedPage(
            MacRemoteModelManagerView(endpoints: endpoints),
            name: "remote-model-manager-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacRemoteModelManagerView(endpoints: endpoints),
            name: "remote-model-manager-compact",
            size: CGSize(width: 680, height: 720)
        )
    }

    func testPromptLibraryRendersInsideARealWindowAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacPromptLibraryView(), name: "prompt-library-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacPromptLibraryView(), name: "prompt-library-compact", size: CGSize(width: 680, height: 720))
    }

    func testSettingsSectionsAndUtilityPagesRenderInBothAppearances() async throws {
        for section in ["general", "providers", "agent", "connections", "notifications", "data", "diagnostics"] {
            for width: CGFloat in [520, 1040] {
                try await assertRenderedPage(
                    MacSettingsView(snapshotHostState: .notConfigured, embedsInWorkspace: true, initialCategory: section),
                    name: "settings-\(section)-\(Int(width))", size: CGSize(width: width, height: 760))
            }
        }
        for width: CGFloat in [520, 1040] {
            try await assertRenderedPage(MacMemoryCenterPage(), name: "memory-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacHealthSummaryView(), name: "health-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacNotificationCenterView(), name: "notifications-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacModelTestView(), name: "model-test-\(Int(width))", size: CGSize(width: width, height: 760))
        }
    }

    private func assertRenderedPage<Page: View>(
        _ page: Page,
        name: String,
        size: CGSize
    ) async throws {
        for dark in [false, true] {
        let name = name + (dark ? "-dark" : "-light")
        let host = NSHostingView(rootView: page.frame(width: size.width, height: size.height)
            .environment(\.colorScheme, dark ? .dark : .light))
        host.frame = NSRect(origin: .zero, size: size)
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.animationBehavior = .none
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.appearance = host.appearance
        window.orderFrontRegardless()
        defer { window.close() }

        try? await Task.sleep(for: .milliseconds(350))
        host.layoutSubtreeIfNeeded()

        let representation = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 20_000, "\(name) rendered as an unexpectedly small or blank image")
        XCTAssertEqual(host.frame.size, size)

        let attachmentURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-secondary-page-\(UUID().uuidString).png")
        try png.write(to: attachmentURL, options: .atomic)
        let attachment = XCTAttachment(contentsOfFile: attachmentURL)
        attachment.name = name
        attachment.lifetime = .keepAlways
        XCTContext.runActivity(named: name) { $0.add(attachment) }
        let output = URL(fileURLWithPath: "/tmp/nola-native-page-matrix", isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        try png.write(to: output.appendingPathComponent(name + ".png"), options: .atomic)
        }
    }
}

final class MacRuntimeBoundaryTests: XCTestCase {
    func testMacRuntimeResolvesBundledAgentHostExecutable() throws {
        let executableURL = try XCTUnwrap(MacAgentHostExecutableLocator.resolve())

        XCTAssertEqual(executableURL.lastPathComponent, "nola-agent-host")
        XCTAssertNotEqual(executableURL.lastPathComponent, "nola")
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: executableURL.path))
    }

    func testRuntimeBoundaryStringsAreLocalized() throws {
        let catalogURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "Localizable.xcstrings",
                withExtension: "txt"
            )
        )
        let data = try Data(contentsOf: catalogURL)
        let catalog = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let strings = try XCTUnwrap(catalog["strings"] as? [String: Any])
        let requiredLocales = ["de", "es", "fr", "ja", "ko", "ru", "zh-Hans", "zh-Hant"]
        let requiredKeys = [
            "Choose a provider model or an on-device model for chat.",
            "Provider Model",
            "Provider Model is not configured",
            "Local Agent Runtime",
            "Local Agent Runtime is unavailable",
            "The local agent runtime is part of Nola for Mac. Rebuild the Mac target or configure NOLA_AGENT_HOST_PATH.",
            "Starting the local execution environment",
            "Run tools with Nola's local agent runtime.",
            "Run tools through the selected connected host.",
        ]

        for key in requiredKeys {
            let entry = try XCTUnwrap(strings[key] as? [String: Any], "Missing localization key: \(key)")
            let localizations = try XCTUnwrap(entry["localizations"] as? [String: Any])
            for locale in requiredLocales {
                XCTAssertNotNil(localizations[locale], "Missing \(locale) localization for \(key)")
            }
        }
    }
}

@MainActor
final class MacSecondaryPageNavigationTests: XCTestCase {
    func testDataCenterNavigationStateOwnsWebAndHealthRoutes() {
        let navigation = MacDataCenterNavigationState()
        XCTAssertEqual(navigation.route, .overview)
        navigation.showWebSources()
        XCTAssertEqual(navigation.route, .webSources)
        navigation.showHealth()
        XCTAssertEqual(navigation.route, .health)
        navigation.showOverview()
        XCTAssertEqual(navigation.route, .overview)
    }

    func testAutomationNavigationStateReturnsFromRunHistory() {
        let navigation = MacAutomationCenterNavigationState()
        XCTAssertFalse(navigation.showsRunHistory)
        navigation.showRunHistory()
        XCTAssertTrue(navigation.showsRunHistory)
        navigation.showAutomations()
        XCTAssertFalse(navigation.showsRunHistory)
    }

    func testModelCenterNavigationStateReturnsFromLocalModels() {
        let navigation = MacModelCenterNavigationState()
        XCTAssertTrue(navigation.showsLocalModels)
        XCTAssertFalse(navigation.showsRemoteModels)
        XCTAssertFalse(navigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        XCTAssertFalse(navigation.shouldRefreshProviderCatalog(baseURL: "", catalogIsEmpty: true))
        XCTAssertFalse(navigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: false))
        navigation.showLocalModels()
        XCTAssertTrue(navigation.showsLocalModels)
        XCTAssertFalse(navigation.showsRemoteModels)
        XCTAssertFalse(navigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        navigation.showRemoteModels()
        XCTAssertFalse(navigation.showsLocalModels)
        XCTAssertTrue(navigation.showsRemoteModels)
        XCTAssertTrue(navigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        navigation.showModelCenter()
        XCTAssertTrue(navigation.showsLocalModels)
        XCTAssertFalse(navigation.showsRemoteModels)
    }
}
