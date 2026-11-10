// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacPromptTemplateCanonicalEndToEndTests: XCTestCase {
    func testPromptTemplateSnapshotSurvivesRealCoreReopenWithoutLegacyDefaults() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-prompts-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        UserDefaults.standard.set(Data("legacy".utf8), forKey: "promptTemplates")
        UserDefaults.standard.set(UUID().uuidString, forKey: "nola.mac.prompt.defaultTemplateID")
        defer {
            UserDefaults.standard.removeObject(forKey: "promptTemplates")
            UserDefaults.standard.removeObject(forKey: "nola.mac.prompt.defaultTemplateID")
        }

        let template = PromptTemplate(name: "Canonical", content: "body", description: "test")
        let snapshot = PromptTemplateSnapshot(customTemplates: [template], selectedTemplateID: template.id)
        do {
            let store = try MacApplicationPort(databaseURL: database)
            let data = try JSONEncoder().encode(snapshot)
            _ = try store.putProductSetting(
                key: MacProductSettingKeys.promptTemplates,
                valueJSON: String(decoding: data, as: UTF8.self),
                updatedAtMs: 1
            )
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let setting = try XCTUnwrap(reopened.productSetting(key: MacProductSettingKeys.promptTemplates))
        let restored = try JSONDecoder().decode(
            PromptTemplateSnapshot.self,
            from: try XCTUnwrap(setting.valueJSON.data(using: .utf8))
        )
        XCTAssertEqual(restored, snapshot)
    }
}
