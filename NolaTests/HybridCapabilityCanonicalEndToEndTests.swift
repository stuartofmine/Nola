// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class HybridCapabilityCanonicalEndToEndTests: XCTestCase {
    @MainActor
    func testCapabilitiesColdReopenAndHostRebindUseOnlyCanonicalStore() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-hybrid-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        UserDefaults.standard.set(true, forKey: "chat.hybrid.device_tools_enabled")
        UserDefaults.standard.set(true, forKey: "chat.hybrid.files_enabled")
        defer {
            UserDefaults.standard.removeObject(forKey: "chat.hybrid.device_tools_enabled")
            UserDefaults.standard.removeObject(forKey: "chat.hybrid.files_enabled")
        }

        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            XCTAssertEqual(try IOSHybridCapabilitySettings.load(store: store), .init())
            try IOSHybridCapabilitySettings(
                boundHostURL: "https://host-a.example/v1",
                deviceToolsEnabled: true,
                filesEnabled: true,
                clipboardEnabled: true,
                locationEnabled: false,
                calendarEnabled: true
            ).save(store: store)
        }

        do {
            let reopened = try IOSApplicationRuntime(databaseURL: database)
            let persisted = try IOSHybridCapabilitySettings.load(store: reopened)
            XCTAssertTrue(persisted.deviceToolsEnabled)
            XCTAssertTrue(persisted.filesEnabled)
            XCTAssertTrue(persisted.clipboardEnabled)
            XCTAssertFalse(persisted.locationEnabled)
            XCTAssertTrue(persisted.calendarEnabled)

            try IOSHybridCapabilitySettings.bindHost("https://host-b.example/v1", store: reopened)
            let rebound = try IOSHybridCapabilitySettings.load(store: reopened)
            XCTAssertEqual(rebound.boundHostURL, "https://host-b.example/v1")
            XCTAssertFalse(rebound.deviceToolsEnabled)
            XCTAssertFalse(rebound.filesEnabled)
            XCTAssertFalse(rebound.clipboardEnabled)
            XCTAssertFalse(rebound.locationEnabled)
            XCTAssertFalse(rebound.calendarEnabled)
        }
    }

    @MainActor
    func testCurrentSchemaCorruptionFailsInsteadOfFallingBackToDefaults() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-hybrid-invalid-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        let invalidPayloads = [
            #"{"schemaVersion":0,"boundHostURL":"","deviceToolsEnabled":false,"filesEnabled":false,"clipboardEnabled":false,"locationEnabled":false,"calendarEnabled":false}"#,
            #"{"schemaVersion":1,"boundHostURL":"","deviceToolsEnabled":false,"filesEnabled":false,"clipboardEnabled":false,"locationEnabled":false,"calendarEnabled":false,"legacyFlag":true}"#
        ]

        for payload in invalidPayloads {
            let store = try IOSApplicationRuntime(databaseURL: database)
            _ = try store.putProductSetting(
                key: IOSHybridCapabilitySettings.productSettingKey,
                valueJSON: payload,
                updatedAtMs: 1
            )
            XCTAssertThrowsError(try IOSHybridCapabilitySettings.load(store: store))
        }

        let store = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertThrowsError(try store.putProductSetting(
            key: IOSHybridCapabilitySettings.productSettingKey,
            valueJSON: "not-json",
            updatedAtMs: 2
        ))
    }
}
