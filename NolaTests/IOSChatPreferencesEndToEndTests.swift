import Foundation
import Testing
@testable import Nola

@Suite("iOS canonical chat preferences E2E")
struct IOSChatPreferencesEndToEndTests {
    @Test
    func preferencesSurviveColdReopenAndIgnoreRemovedUserDefaultsKeys() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-chat-preferences-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let removedKey = "chat.auto_compact.trigger_percent"
        UserDefaults.standard.set(55, forKey: removedKey)
        defer { UserDefaults.standard.removeObject(forKey: removedKey) }

        let first = try IOSApplicationRuntime(databaseURL: database)
        var preferences = IOSChatPreferences()
        preferences.autoCompactTriggerPercent = 90
        try preferences.save(store: first)

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try IOSChatPreferences.load(store: reopened).autoCompactTriggerPercent == 90)
    }

    @Test
    func invalidOrObsoletePayloadsAreRejectedInsteadOfRepaired() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-invalid-chat-preferences-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let store = try IOSApplicationRuntime(databaseURL: database)
        _ = try store.putProductSetting(
            key: IOSChatPreferences.productSettingKey,
            valueJSON: #"{"schemaVersion":0,"autoCompactTriggerPercent":70}"#,
            updatedAtMs: 1
        )

        #expect(throws: IOSChatPreferencesError.unsupportedSchema(0)) {
            _ = try IOSChatPreferences.load(store: store)
        }
    }
}
