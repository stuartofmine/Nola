// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated secondary-page catalog coverage, localization, layout, visual window rendering, and navigation suite for macOS.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

final class MacSecondaryPageCatalogCoverageTests: XCTestCase {
    private let requiredLocales = ["de", "es", "fr", "ja", "ko", "ru", "zh-Hans", "zh-Hant"]

    func testSecondaryTablesAndSharedSurfaceCatalogCoverage() throws {
        try assertCatalogCoverage(
            catalogPath: "apps/macos/AutomationCenter.xcstrings",
            sourcePaths: ["apps/macos/UI/MacAutomationCenterView.swift"],
            patterns: [#"String\(localized: \"([^\"]+)\", table: \"AutomationCenter\""#]
        )
        try assertCatalogCoverage(
            catalogPath: "apps/macos/ModelCenter.xcstrings",
            sourcePaths: [
                "apps/macos/UI/MacModelCenterView.swift",
                "apps/macos/Services/MacModelCatalogService.swift",
                "apps/macos/UI/MacSiwcSignInView.swift",
                "apps/macos/UI/MacSiwcAccountManagementView.swift",
                "apps/macos/Services/MacSiwcAccountBridge.swift"
            ],
            patterns: [#"String\(localized: \"([^\"]+)\", table: \"ModelCenter\""#]
        )

        let mainStrings = try loadStrings("apps/macos/Localizable.xcstrings")
        try assertCatalogCoverage(
            strings: mainStrings,
            catalogPath: "apps/macos/Localizable.xcstrings",
            sourcePaths: [
                "apps/macos/UI/MacDataCenterView.swift",
                "apps/macos/UI/MacCapabilityCenterView.swift",
                "apps/macos/Services/MacCapabilityService.swift"
            ],
            patterns: [
                #"macDataCenterText\(\"([^\"]+)\""#,
                #"macCapabilityText\(\"([^\"]+)\""#,
                #"text\(\"(mac\.capabilities\.[^\"]+)\""#,
                #"NSLocalizedString\(\"(mac\.capabilities\.[^\"]+)\""#
            ]
        )
        try assertCatalogCoverage(
            strings: mainStrings,
            catalogPath: "apps/macos/Localizable.xcstrings",
            sourcePaths: ["shared/apple/GenerativeUI/AppleUISurfaceView.swift"],
            patterns: [#"String\(localized: \"(a2ui\.[^\"]+)\""#]
        )
        try assertCatalogCoverage(
            strings: mainStrings,
            catalogPath: "apps/macos/Localizable.xcstrings",
            sourcePaths: ["shared/apple/Presentation/AppleSoftwareUpdateSection.swift"],
            patterns: [#"(?:Text|Button)\(\"(softwareUpdate\.[^\"]+)\""#],
            excludingInterpolatedLiterals: true
        )
        for status in ["disabled", "not_checked", "up_to_date", "ahead", "unavailable", "available", "incompatible_data"] {
            for locale in ["en"] + requiredLocales {
                let value = try localizedValue(strings: mainStrings, key: "softwareUpdate.status." + status, locale: locale)
                XCTAssertFalse(value.isEmpty)
                XCTAssertFalse(value.hasPrefix("softwareUpdate."))
            }
        }

        let expectations: [(String, String, String)] = [
            ("apps/macos/AutomationCenter.xcstrings", "New Automation", "新建自动化"),
            ("apps/macos/AutomationCenter.xcstrings", "Run History", "运行历史"),
            ("apps/macos/ModelCenter.xcstrings", "Load Models", "加载模型"),
            ("apps/macos/Localizable.xcstrings", "mac.capabilities.title", "系统访问"),
            ("apps/macos/Localizable.xcstrings", "mac.data_center.manage_sources", "管理网页来源"),
            ("apps/macos/Localizable.xcstrings", "Data Center", "数据中心"),
            ("apps/macos/Localizable.xcstrings", "Thinking", "思考中"),
            ("apps/macos/Localizable.xcstrings", "Thinking Process", "思考过程")
        ]
        var cachedTables: [String: [String: Any]] = ["apps/macos/Localizable.xcstrings": mainStrings]
        for (path, key, expected) in expectations {
            let strings: [String: Any]
            if let existing = cachedTables[path] {
                strings = existing
            } else {
                let loaded = try loadStrings(path)
                cachedTables[path] = loaded
                strings = loaded
            }
            let value = try localizedValue(strings: strings, key: key, locale: "zh-Hans")
            XCTAssertEqual(value, expected, "Unexpected zh-Hans localization for \(key)")
        }
    }

    func testMainCatalogCoversStaticAndDynamicSecondaryPageSources() throws {
        let repositoryRoot = repositoryRootURL()
        let mainStrings = try loadStrings("apps/macos/Localizable.xcstrings")
        let uiDirectory = repositoryRoot.appendingPathComponent("apps/macos/UI", isDirectory: true)
        let uiSourcePaths = try FileManager.default.contentsOfDirectory(
            at: uiDirectory,
            includingPropertiesForKeys: nil
        )
        .filter { $0.pathExtension == "swift" }
        .map { "apps/macos/UI/\($0.lastPathComponent)" }

        try assertCatalogCoverage(
            strings: mainStrings,
            catalogPath: "apps/macos/Localizable.xcstrings",
            sourcePaths: uiSourcePaths,
            patterns: [
                #"(?:Text|Label|Button|Section|Toggle|Picker|TextField|SecureField|MacPageHeader|MacEmptyState|MacInspectorSection|MacInspectorRow|ContentUnavailableView)\(\"([^\"\\]*(?:\\.[^\"\\]*)*)\"(?!\s*,\s*tableName:)"#,
                #"(?:navigationTitle|alert|confirmationDialog)\(\"([^\"\\]*(?:\\.[^\"\\]*)*)\""#
            ],
            excludingInterpolatedLiterals: true
        )

        let allMacSourcePaths = ["apps/macos/UI", "apps/macos/Services"].flatMap { directory in
            let url = repositoryRoot.appendingPathComponent(directory, isDirectory: true)
            let resolvedDirectoryPath = url.resolvingSymlinksInPath().path + "/"
            guard let enumerator = FileManager.default.enumerator(
                at: url,
                includingPropertiesForKeys: nil
            ) else { return [String]() }
            return enumerator.compactMap { item -> String? in
                guard let fileURL = item as? URL, fileURL.pathExtension == "swift" else { return nil }
                let relativePath = fileURL.resolvingSymlinksInPath().path
                    .replacingOccurrences(of: resolvedDirectoryPath, with: "")
                return "\(directory)/\(relativePath)"
            }
        }

        try assertCatalogCoverage(
            strings: mainStrings,
            catalogPath: "apps/macos/Localizable.xcstrings",
            sourcePaths: allMacSourcePaths,
            patterns: [
                #"mac[A-Z][A-Za-z]+Text\(\"([^\"]+)\""#,
                #"String\(localized: \"([^\"]+)\"(?!\s*,\s*table:)\)"#
            ]
        )
    }

    private func assertCatalogCoverage(
        strings: [String: Any]? = nil,
        catalogPath: String,
        sourcePaths: [String],
        patterns: [String],
        excludingInterpolatedLiterals: Bool = false
    ) throws {
        let resolvedStrings = try strings ?? loadStrings(catalogPath)
        var referencedKeys = Set<String>()

        for sourcePath in sourcePaths {
            let source = try String(contentsOf: fixtureURL(for: sourcePath), encoding: .utf8)
            for pattern in patterns {
                let expression = try NSRegularExpression(pattern: pattern)
                let range = NSRange(source.startIndex..<source.endIndex, in: source)
                for match in expression.matches(in: source, range: range) {
                    guard let captureRange = Range(match.range(at: 1), in: source) else { continue }
                    let key = String(source[captureRange])
                    if key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        continue
                    }
                    if excludingInterpolatedLiterals, key.contains(#"\("#) {
                        continue
                    }
                    referencedKeys.insert(key)
                }
            }
        }

        XCTAssertFalse(referencedKeys.isEmpty, "No localization references were discovered")
        var missing: [String] = []
        for key in referencedKeys.sorted() {
            for locale in requiredLocales {
                let entry = resolvedStrings[key] as? [String: Any]
                let localizations = entry?["localizations"] as? [String: Any]
                let localization = localizations?[locale] as? [String: Any]
                let unit = localization?["stringUnit"] as? [String: Any]
                let value = unit?["value"] as? String
                if value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false {
                    missing.append("\(locale): \(key)")
                }
            }
        }
        XCTAssertTrue(missing.isEmpty, "Incomplete catalog \(catalogPath):\n" + missing.joined(separator: "\n"))
    }

    private func loadStrings(_ path: String) throws -> [String: Any] {
        let handle = try FileHandle(forReadingFrom: fixtureURL(for: path))
        defer { try? handle.close() }
        let data = try XCTUnwrap(handle.readToEnd(), "Empty catalog at \(path)")
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try XCTUnwrap(object["strings"] as? [String: Any])
    }

    private func fixtureURL(for repositoryPath: String) throws -> URL {
        let url = repositoryRootURL().appendingPathComponent(repositoryPath)
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path), "Missing repository file: \(repositoryPath)")
        return url
    }

    private func repositoryRootURL() -> URL {
        Bundle(for: MacSecondaryPageCatalogCoverageTests.self)
            .resourceURL!
            .appendingPathComponent("RepositoryFixtures", isDirectory: true)
    }

    private func localizedValue(strings: [String: Any], key: String, locale: String) throws -> String {
        let entry = try XCTUnwrap(strings[key] as? [String: Any], "Missing catalog entry for \(key)")
        let localizations = try XCTUnwrap(entry["localizations"] as? [String: Any], "Missing localizations for \(key)")
        let localization = try XCTUnwrap(localizations[locale] as? [String: Any], "Missing \(locale) localization for \(key)")
        let unit = try XCTUnwrap(localization["stringUnit"] as? [String: Any], "Missing string unit for \(key) [\(locale)]")
        return try XCTUnwrap(unit["value"] as? String, "Missing value for \(key) [\(locale)]")
    }
}

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

    func testSecondaryPageTranslationsPlaceholdersAndConditionalCopy() throws {
        let catalog = try loadCatalog(named: "Localizable")
        for page in localizableKeysByPage.keys.sorted() {
            for key in localizableKeysByPage[page] ?? [] {
                try assertTranslated(key, page: page, catalog: catalog)
            }
        }

        let placeholderKeys = localizableKeysByPage.values.flatMap { $0 }.filter { $0.contains("%") }
        for key in placeholderKeys {
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

        let modelCatalog = try loadCatalog(named: "ModelCenter")
        for key in modelCenterKeys {
            try assertTranslated(key, page: "Model Center", catalog: modelCatalog)
        }

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

    func testMacRuntimeResolvesBundledAgentHostAndLocalizedBoundaryStrings() throws {
        let executableURL = try XCTUnwrap(MacAgentHostExecutableLocator.resolve())
        XCTAssertEqual(executableURL.lastPathComponent, "nola-agent-host")
        XCTAssertNotEqual(executableURL.lastPathComponent, "nola")
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: executableURL.path))

        let catalogURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "Localizable.xcstrings",
                withExtension: "txt"
            )
        )
        let data = try Data(contentsOf: catalogURL)
        let catalog = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let strings = try XCTUnwrap(catalog["strings"] as? [String: Any])
        let locales = ["de", "es", "fr", "ja", "ko", "ru", "zh-Hans", "zh-Hant"]
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
            for locale in locales {
                XCTAssertNotNil(localizations[locale], "Missing \(locale) localization for \(key)")
            }
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
    func testOnboardingAndStartupGateStatesRenderInBothAppearances() async throws {
        try await assertOnboardingWelcome()
        try await assertStartupGateStates()
    }

    func testWorkspaceCentersRenderInsideRealWindowsAtRegularAndCompactWidths() async throws {
        for section in ["memory", "sources", "usage"] {
            try await assertRenderedPage(
                MacDataCenterView(initialSection: section),
                name: "mac-data-center-\(section)-regular",
                size: CGSize(width: 1_040, height: 760)
            )
            try await assertRenderedPage(
                MacDataCenterView(initialSection: section),
                name: "mac-data-center-\(section)-compact",
                size: CGSize(width: 520, height: 720)
            )
        }

        try await assertRenderedPage(MacCapabilityCenterView(), name: "capabilities-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacCapabilityCenterView(), name: "capabilities-compact", size: CGSize(width: 680, height: 720))

        try await assertRenderedPage(MacDiagnosticsCenterView(), name: "diagnostics-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacDiagnosticsCenterView(), name: "diagnostics-compact", size: CGSize(width: 520, height: 720))

        try await assertRenderedPage(MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(), kinds: [.skill, .mcp]), name: "extensions-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(), kinds: [.skill, .mcp]), name: "extensions-compact", size: CGSize(width: 520, height: 720))

        try await assertRenderedPage(MacAutomationCenterView(), name: "mac-automation-center-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacAutomationCenterView(), name: "mac-automation-center-compact", size: CGSize(width: 520, height: 720))
        try await assertRenderedPage(
            MacAutomationLiveRunStatus(detail: "Reading the selected data source and preparing the next tool call")
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(MacDesignColor.canvas),
            name: "mac-automation-live-status-regular",
            size: CGSize(width: 620, height: 200)
        )
        try await assertRenderedPage(
            MacAutomationLiveRunStatus(detail: "Reading the selected data source and preparing the next tool call")
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(MacDesignColor.canvas),
            name: "mac-automation-live-status-compact",
            size: CGSize(width: 340, height: 220)
        )

        try await assertRenderedPage(MacActivityCenterView(), name: "activity-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacActivityCenterView(), name: "activity-compact", size: CGSize(width: 520, height: 720))
        try await assertRenderedPage(MacWebSourcesView(), name: "web-sources-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacWebSourcesView(), name: "web-sources-compact", size: CGSize(width: 680, height: 720))
        try await assertRenderedPage(MacDataManagementView(), name: "data-management-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacDataManagementView(), name: "data-management-compact", size: CGSize(width: 520, height: 720))
        try await assertRenderedPage(MacPromptLibraryView(), name: "prompt-library-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacPromptLibraryView(), name: "prompt-library-compact", size: CGSize(width: 680, height: 720))
    }

    func testModelAndEndpointManagersRenderAtRegularAndCompactWidths() async throws {
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost()), name: "model-center-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost()), name: "model-center-compact", size: CGSize(width: 680, height: 720))

        let apiNavigation = MacModelCenterNavigationState()
        apiNavigation.showRemoteModels()
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: apiNavigation), name: "model-center-api-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: apiNavigation), name: "model-center-api-compact", size: CGSize(width: 680, height: 720))

        let localNavigation = MacModelCenterNavigationState()
        localNavigation.showLocalModels()
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: localNavigation), name: "local-model-manager-regular", size: CGSize(width: 1_040, height: 760))
        try await assertRenderedPage(MacModelCenterView(remoteHost: PersistenceTestRemoteHost(), navigation: localNavigation), name: "local-model-manager-compact", size: CGSize(width: 680, height: 720))

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("mac-model-connections-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let connectionService = MacModelConnectionService(application: port)
        let created = try connectionService.create(
            name: "Disabled Provider",
            endpoint: "https://disabled.example.test/v1",
            wireProtocol: "responses",
            secret: "sk-disabled-fixture"
        )
        _ = try connectionService.disconnect(created)
        for (label, size) in [("regular", CGSize(width: 1_040, height: 760)), ("compact", CGSize(width: 680, height: 720))] {
            try await assertRenderedPage(
                MacModelConnectionsView(service: connectionService),
                name: "model-connections-disabled-\(label)",
                size: size
            )
        }

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
            MacRemoteModelManagerView(endpoints: endpoints).background(MacDesignColor.canvas),
            name: "remote-model-manager-regular",
            size: CGSize(width: 1_040, height: 760)
        )
        try await assertRenderedPage(
            MacRemoteModelManagerView(endpoints: endpoints).background(MacDesignColor.canvas),
            name: "remote-model-manager-compact",
            size: CGSize(width: 680, height: 720)
        )

        let parameterRoute = MacModelParametersRoute(modelName: "fixture-remote-model", isLocal: false, displayName: "Fixture Remote Model")
        try await assertRenderedPage(
            MacModelParametersView(route: parameterRoute).background(MacDesignColor.canvas),
            name: "model-parameters-regular",
            size: CGSize(width: 760, height: 720)
        )
        try await assertRenderedPage(
            MacModelParametersView(route: parameterRoute).background(MacDesignColor.canvas),
            name: "model-parameters-compact",
            size: CGSize(width: 620, height: 640)
        )
    }

    func testSettingsSectionsAndUtilityPagesRenderInBothAppearances() async throws {
        for section in ["general", "providers", "agent", "connections", "notifications", "data", "diagnostics"] {
            for width: CGFloat in [520, 1_040] {
                try await assertRenderedPage(
                    MacSettingsView(snapshotHostState: .notConfigured, initialCategory: section),
                    name: "settings-\(section)-\(Int(width))",
                    size: CGSize(width: width, height: 760)
                )
            }
        }
        let emptyRepo = FileManager.default.temporaryDirectory.appendingPathComponent("mac-review-empty-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: emptyRepo, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: emptyRepo) }
        for width: CGFloat in [680, 1_040] {
            try await assertRenderedPage(
                MacWorkspaceReviewView(route: MacWorkspaceReviewRoute(path: emptyRepo.path)),
                name: "workspace-review-\(Int(width))",
                size: CGSize(width: width, height: 760)
            )
            try await assertRenderedPage(MacMedicationSearchView(), name: "medication-\(Int(width))", size: CGSize(width: width, height: 760))
        }
        for width: CGFloat in [520, 1_040] {
            try await assertRenderedPage(MacMemoryCenterPage(), name: "memory-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacHealthSummaryView(), name: "health-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacNotificationCenterView(), name: "notifications-\(Int(width))", size: CGSize(width: width, height: 760))
            try await assertRenderedPage(MacModelTestView(), name: "model-test-\(Int(width))", size: CGSize(width: width, height: 760))
        }
    }

    private func assertOnboardingWelcome() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let label = appearance == .darkAqua ? "dark" : "light"
            for (widthLabel, size) in [("regular", CGSize(width: 980, height: 740)), ("compact", CGSize(width: 680, height: 720))] {
                try await MacVisualPixelAssertions.assertView(
                    MacOnboardingView(onComplete: {}).environment(\.accessibilityEnabled, true),
                    name: "onboarding-welcome-\(widthLabel)-\(label)",
                    size: size,
                    appearance: appearance,
                    beforeCapture: { _, host in
                        XCTAssertNotNil(self.findAccessibleElement(identifier: "onboarding.connect", in: host))
                        XCTAssertNotNil(self.findAccessibleElement(identifier: "onboarding.dismiss", in: host))
                    },
                    assertWindow: { window, host in
                        XCTAssertTrue(window.styleMask.contains(.fullSizeContentView))
                        XCTAssertEqual(window.titleVisibility, .hidden)
                        let target = window.contentView?.superview ?? host
                        let image = try MacVisualPixelAssertions.captureCGImage(of: target)
                        let button = try XCTUnwrap(self.findAccessibleElement(identifier: "onboarding.connect", in: host))
                        let frame = try XCTUnwrap(button.accessibilityFrame?())
                        let bounds = target.convert(window.convertFromScreen(frame), from: nil).insetBy(dx: 6, dy: 4)
                        let scale = CGFloat(image.width) / target.bounds.width
                        let top = target.isFlipped ? bounds.minY : target.bounds.maxY - bounds.maxY
                        let rect = CGRect(x: bounds.minX * scale, y: top * scale,
                            width: bounds.width * scale, height: bounds.height * scale)
                        let crop = try XCTUnwrap(image.cropping(to: rect))
                        let contrast = try MacVisualPixelAssertions.analyzePixels(cgImage: crop,
                            contentRect: CGRect(x: 0, y: 0, width: crop.width, height: crop.height),
                            isDark: appearance == .darkAqua)
                        XCTAssertGreaterThan(max(contrast.headerContrastDelta, contrast.bodyContrastDelta), 100,
                            "The model action must remain legible when the window is inactive.")
                        let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
                        let screenshot = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                        screenshot.name = "welcome-\(widthLabel)-\(label)"
                        screenshot.lifetime = .keepAlways
                        self.add(screenshot)
                    }
                )
            }
        }
    }

    private func assertStartupGateStates() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("mac-startup-gate-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let incompatibleDB = root.appendingPathComponent("legacy.db")
        let sqlite = Process()
        sqlite.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        sqlite.arguments = [
            incompatibleDB.path,
            "CREATE TABLE legacy(value TEXT); PRAGMA application_id=1313819713; PRAGMA user_version=11;",
        ]
        try sqlite.run()
        sqlite.waitUntilExit()
        XCTAssertEqual(sqlite.terminationStatus, 0)

        let blockerFile = root.appendingPathComponent("not-a-directory")
        try Data("invalid".utf8).write(to: blockerFile)
        let failingDB = blockerFile.appendingPathComponent("store.db")

        let incompatiblePort = MacApplicationPort(deferredDatabaseURL: incompatibleDB)
        let failingPort = MacApplicationPort(deferredDatabaseURL: failingDB)

        for (label, port) in [("incompatible", incompatiblePort), ("launch-failed", failingPort)] {
            for size in [CGSize(width: 920, height: 640), CGSize(width: 620, height: 520)] {
                try await assertRenderedPage(
                    MacApplicationStartupGate(applicationPort: port) {
                        Text("Ready")
                    },
                    name: "startup-gate-\(label)-\(Int(size.width))",
                    size: size
                )
            }
        }
    }

    private func findAccessibleElement(identifier: String, in root: AnyObject) -> AnyObject? {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var index = 0
        while index < queue.count, visited.count < 4000 {
            let object = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            if (object.accessibilityIdentifier?() ?? nil) == identifier {
                return object
            }
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView {
                queue.append(contentsOf: view.subviews)
            }
        }
        return nil
    }

    private func assertRenderedPage<Page: View>(
        _ page: Page,
        name: String,
        size: CGSize
    ) async throws {
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let appearanceName = name + (appearance == .darkAqua ? "-dark" : "-light")
            try await MacVisualPixelAssertions.assertView(
                page,
                name: appearanceName,
                size: size,
                appearance: appearance
            )
        }
    }
}

@MainActor
final class MacSecondaryPageNavigationTests: XCTestCase {
    func testSecondaryPageInitialLaunchAndNavigationStateMachines() {
        for section in MacWorkspaceSection.allCases {
            XCTAssertEqual(
                MacWorkspaceSection.initialSection(environment: ["NOLA_MAC_INITIAL_SECTION": section.rawValue]),
                section
            )
        }
        XCTAssertEqual(MacWorkspaceSection.initialSection(environment: [:]), .chat)

        let dataNavigation = MacDataCenterNavigationState()
        XCTAssertEqual(dataNavigation.route, .overview)
        dataNavigation.showWebSources()
        XCTAssertEqual(dataNavigation.route, .webSources)
        dataNavigation.showHealth()
        XCTAssertEqual(dataNavigation.route, .health)
        dataNavigation.showOverview()
        XCTAssertEqual(dataNavigation.route, .overview)

        let automationNavigation = MacAutomationCenterNavigationState()
        XCTAssertFalse(automationNavigation.showsRunHistory)
        automationNavigation.showRunHistory()
        XCTAssertTrue(automationNavigation.showsRunHistory)
        automationNavigation.showAutomations()
        XCTAssertFalse(automationNavigation.showsRunHistory)

        let modelNavigation = MacModelCenterNavigationState()
        XCTAssertTrue(modelNavigation.showsLocalModels)
        XCTAssertFalse(modelNavigation.showsRemoteModels)
        XCTAssertFalse(modelNavigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        XCTAssertFalse(modelNavigation.shouldRefreshProviderCatalog(baseURL: "", catalogIsEmpty: true))
        XCTAssertFalse(modelNavigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: false))
        modelNavigation.showLocalModels()
        XCTAssertTrue(modelNavigation.showsLocalModels)
        XCTAssertFalse(modelNavigation.showsRemoteModels)
        XCTAssertFalse(modelNavigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        modelNavigation.showRemoteModels()
        XCTAssertFalse(modelNavigation.showsLocalModels)
        XCTAssertTrue(modelNavigation.showsRemoteModels)
        XCTAssertTrue(modelNavigation.shouldRefreshProviderCatalog(baseURL: "https://example.com/v1", catalogIsEmpty: true))
        modelNavigation.showModelCenter()
        XCTAssertTrue(modelNavigation.showsLocalModels)
        XCTAssertFalse(modelNavigation.showsRemoteModels)
    }

    func testDataAndAutomationHeadersRestoreNavigationAtCompactAndRegularWidths() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("page-navigation-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let preferencesName = "page-navigation-\(UUID().uuidString)"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: preferencesName))
        defer { preferences.removePersistentDomain(forName: preferencesName) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let service = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false, preferences: preferences)
        let activity = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let environment = MacAppEnvironment(applicationSupportRoot: root.appendingPathComponent("Support"), cachesRoot: root.appendingPathComponent("Caches"))
        let automation = try MacAutomationStore(baseDirectory: root.appendingPathComponent("Automations"),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"), synchronizeSystemJobs: false,
            appEnvironment: environment, activityStore: activity,
            completionNotifier: { _, _ in }, dataSourceRegistry: registry, webSourceStore: web)
        for width: CGFloat in [680, 1040] {
            var dataNavigations = 0
            try await pressNavigation(in: MacDataCenterView(navigationAction: { dataNavigations += 1 },
                service: service, registry: registry, webSources: web), width: width)
            XCTAssertEqual(dataNavigations, 1)
            var automationNavigations = 0
            try await pressNavigation(in: MacAutomationCenterView(navigationAction: { automationNavigations += 1 },
                store: automation, activityStore: activity), width: width)
            XCTAssertEqual(automationNavigations, 1)
        }
    }

    func testEverySettingsCategoryCanOpenAndReturnInBothThemes() async throws {
        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let preferencesName = "settings-navigation-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: preferencesName))
        defer { preferences.removePersistentDomain(forName: preferencesName) }
        for dark in [false, true] {
            for width: CGFloat in [820, 1040] {
                let host = NSHostingView(rootView: MacSettingsView(snapshotHostState: .notConfigured)
                    .defaultAppStorage(preferences)
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .environment(\.accessibilityEnabled, true)
                    .frame(width: width, height: 760))
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 760),
                    styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false
                window.contentView = host
                window.orderFrontRegardless()
                defer { window.close() }
                for section in ["general", "providers", "agent", "connections", "data", "diagnostics"] {
                    try await pressIdentifier("settings.category." + section, in: host)
                }
                window.close()
            }
        }
    }

    private func pressIdentifier(_ identifier: String, in host: NSView) async throws {
        let deadline = Date().addingTimeInterval(4)
        repeat {
            host.layoutSubtreeIfNeeded()
            if let button = accessibilityObjects(host).first(where: { ($0.accessibilityIdentifier?() ?? nil) == identifier }) {
                XCTAssertTrue(button.accessibilityPerformPress?() ?? false, identifier)
                try await Task.sleep(for: .milliseconds(60))
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        } while Date() < deadline
        XCTFail("Mounted settings page has no accessible action: \(identifier)")
    }

    private func pressNavigation<Content: View>(in content: Content, width: CGFloat) async throws {
        let host = NSHostingView(rootView: content.frame(width: width, height: 640)
            .environment(\.accessibilityEnabled, true))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }
        let label = String(localized: "Toggle Sidebar")
        let deadline = Date().addingTimeInterval(3)
        repeat {
            host.layoutSubtreeIfNeeded()
            if let button = accessibilityObjects(host).first(where: { ($0.accessibilityLabel?() ?? nil) == label }) {
                XCTAssertTrue(button.accessibilityPerformPress?() ?? false)
                await Task.yield()
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        } while Date() < deadline
        XCTFail("Mounted page has no accessible navigation button at width \(width)")
    }

    private func accessibilityObjects(_ root: AnyObject) -> [AnyObject] {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}
