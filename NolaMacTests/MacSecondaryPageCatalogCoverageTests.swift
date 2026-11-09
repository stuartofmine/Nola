// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Keeps secondary-page source strings connected to complete macOS string catalogs.

import Foundation
import XCTest

final class MacSecondaryPageCatalogCoverageTests: XCTestCase {
    private let requiredLocales = ["de", "es", "fr", "ja", "ko", "ru", "zh-Hans", "zh-Hant"]

    func testAutomationCenterReferencesCompleteAutomationCatalog() throws {
        try assertCatalogCoverage(
            catalogPath: "NolaMac/AutomationCenter.xcstrings",
            sourcePaths: ["NolaMac/UI/MacAutomationCenterView.swift"],
            patterns: [#"String\(localized: \"([^\"]+)\", table: \"AutomationCenter\""#]
        )
    }

    func testModelCenterReferencesCompleteModelCatalog() throws {
        try assertCatalogCoverage(
            catalogPath: "NolaMac/ModelCenter.xcstrings",
            sourcePaths: [
                "NolaMac/UI/MacModelCenterView.swift",
                "NolaMac/Services/MacModelCatalogService.swift",
                "NolaMac/UI/MacSiwcSignInView.swift",
                "NolaMac/UI/MacSiwcAccountManagementView.swift",
                "NolaMac/Services/MacSiwcAccountBridge.swift"
            ],
            patterns: [#"String\(localized: \"([^\"]+)\", table: \"ModelCenter\""#]
        )
    }

    func testDynamicDataAndCapabilityKeysExistInMainCatalog() throws {
        try assertCatalogCoverage(
            catalogPath: "NolaMac/Localizable.xcstrings",
            sourcePaths: [
                "NolaMac/UI/MacDataCenterView.swift",
                "NolaMac/UI/MacCapabilityCenterView.swift",
                "NolaMac/Services/MacCapabilityService.swift"
            ],
            patterns: [
                #"macDataCenterText\(\"([^\"]+)\""#,
                #"macCapabilityText\(\"([^\"]+)\""#,
                #"text\(\"(mac\.capabilities\.[^\"]+)\""#,
                #"NSLocalizedString\(\"(mac\.capabilities\.[^\"]+)\""#
            ]
        )
    }

    func testStaticSecondaryPageLabelsHaveCompleteMainCatalogCoverage() throws {
        let repositoryRoot = repositoryRootURL()
        let uiDirectory = repositoryRoot.appendingPathComponent("NolaMac/UI", isDirectory: true)
        let sourcePaths = try FileManager.default.contentsOfDirectory(
            at: uiDirectory,
            includingPropertiesForKeys: nil
        )
        .filter { $0.pathExtension == "swift" }
        .map { "NolaMac/UI/\($0.lastPathComponent)" }

        try assertCatalogCoverage(
            catalogPath: "NolaMac/Localizable.xcstrings",
            sourcePaths: sourcePaths,
            patterns: [
                #"(?:Text|Label|Button|Section|Toggle|Picker|TextField|SecureField|MacPageHeader|MacEmptyState|MacInspectorSection|MacInspectorRow|ContentUnavailableView)\(\"([^\"\\]*(?:\\.[^\"\\]*)*)\"(?!\s*,\s*tableName:)"#,
                #"(?:navigationTitle|alert|confirmationDialog)\(\"([^\"\\]*(?:\\.[^\"\\]*)*)\""#
            ],
            excludingInterpolatedLiterals: true
        )
    }

    func testDynamicMacLocalizationHelpersHaveCompleteMainCatalogCoverage() throws {
        let repositoryRoot = repositoryRootURL()
        let sourcePaths = ["NolaMac/UI", "NolaMac/Services"].flatMap { directory in
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
            catalogPath: "NolaMac/Localizable.xcstrings",
            sourcePaths: sourcePaths,
            patterns: [
                #"mac[A-Z][A-Za-z]+Text\(\"([^\"]+)\""#,
                #"String\(localized: \"([^\"]+)\"(?!\s*,\s*table:)\)"#
            ]
        )
    }

    func testSharedSoftwareUpdateLabelsAndDynamicStatusesHaveTranslations() throws {
        try assertCatalogCoverage(catalogPath: "NolaMac/Localizable.xcstrings",
            sourcePaths: ["Shared/Presentation/AppleSoftwareUpdateSection.swift"],
            patterns: [#"(?:Text|Button)\(\"(softwareUpdate\.[^\"]+)\""#], excludingInterpolatedLiterals: true)
        let strings = try loadStrings("NolaMac/Localizable.xcstrings")
        for status in ["disabled", "not_checked", "up_to_date", "ahead", "unavailable", "available", "incompatible_data"] {
            for locale in ["en"] + requiredLocales {
                let value = try localizedValue(strings: strings, key: "softwareUpdate.status." + status, locale: locale)
                XCTAssertFalse(value.isEmpty)
                XCTAssertFalse(value.hasPrefix("softwareUpdate."))
            }
        }
    }

    func testChineseSecondaryPageHeadingsDoNotFallBackToEnglish() throws {
        let expectations: [(String, String, String)] = [
            ("NolaMac/AutomationCenter.xcstrings", "New Automation", "新建自动化"),
            ("NolaMac/AutomationCenter.xcstrings", "Run History", "运行历史"),
            ("NolaMac/ModelCenter.xcstrings", "Load Models", "加载模型"),
            ("NolaMac/Localizable.xcstrings", "mac.capabilities.title", "系统访问"),
            ("NolaMac/Localizable.xcstrings", "mac.data_center.manage_sources", "管理网页来源"),
            ("NolaMac/Localizable.xcstrings", "Data Center", "数据中心"),
            ("NolaMac/Localizable.xcstrings", "Thinking", "思考中"),
            ("NolaMac/Localizable.xcstrings", "Thinking Process", "思考过程")
        ]

        for (path, key, expected) in expectations {
            let strings = try loadStrings(path)
            let value = try localizedValue(strings: strings, key: key, locale: "zh-Hans")
            XCTAssertEqual(value, expected, "Unexpected zh-Hans localization for \(key)")
        }
    }

    private func assertCatalogCoverage(
        catalogPath: String,
        sourcePaths: [String],
        patterns: [String],
        excludingInterpolatedLiterals: Bool = false
    ) throws {
        let strings = try loadStrings(catalogPath)
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
                let entry = strings[key] as? [String: Any]
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
