// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class PromptTemplateCanonicalEndToEndTests: XCTestCase {
    func testPromptTemplatesColdReopenIgnoreLegacyDefaults() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-prompts-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        let legacy = PromptTemplate(name: "Legacy", content: "legacy", description: "legacy")
        UserDefaults.standard.set(try JSONEncoder().encode([legacy]), forKey: "promptTemplates")
        UserDefaults.standard.set(legacy.id.uuidString, forKey: "selectedPromptTemplateID")
        defer {
            UserDefaults.standard.removeObject(forKey: "promptTemplates")
            UserDefaults.standard.removeObject(forKey: "selectedPromptTemplateID")
        }

        let canonical = PromptTemplate(name: "Canonical", content: "canonical", description: "canonical")
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            let snapshot = PromptTemplateSnapshot(
                customTemplates: [canonical],
                selectedTemplateID: canonical.id
            )
            let data = try JSONEncoder().encode(snapshot)
            _ = try store.putProductSetting(
                key: "prompt.templates",
                valueJSON: String(decoding: data, as: UTF8.self),
                updatedAtMs: 1
            )
        }

        do {
            let reopened = try IOSApplicationRuntime(databaseURL: database)
            let setting = try XCTUnwrap(reopened.productSetting(key: "prompt.templates"))
            let restored = try JSONDecoder().decode(
                PromptTemplateSnapshot.self,
                from: try XCTUnwrap(setting.valueJSON.data(using: .utf8))
            )
            XCTAssertEqual(restored.customTemplates.map(\.id), [canonical.id])
            XCTAssertFalse(restored.customTemplates.contains(where: { $0.id == legacy.id }))
            XCTAssertEqual(restored.selectedTemplateID, canonical.id)
        }
    }
}
