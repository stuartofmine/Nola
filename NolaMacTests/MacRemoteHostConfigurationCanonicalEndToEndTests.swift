// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

final class MacRemoteHostConfigurationCanonicalEndToEndTests: XCTestCase {
    func testHostIdentitySurvivesRealServiceColdReopenAndIgnoresObsoletePreferences() throws {
        let defaults = UserDefaults.standard
        let obsoleteKeys = [
            "nola.mac.connectedHost.url",
            "nola.mac.connectedHost.id",
            "nola.mac.connectedHost.clientID",
        ]
        let previous = Dictionary(uniqueKeysWithValues: obsoleteKeys.map { ($0, defaults.object(forKey: $0)) })
        defer {
            for key in obsoleteKeys {
                if let value = previous[key] ?? nil { defaults.set(value, forKey: key) }
                else { defaults.removeObject(forKey: key) }
            }
        }
        defaults.set("https://obsolete.invalid", forKey: obsoleteKeys[0])
        defaults.set("obsolete-host", forKey: obsoleteKeys[1])
        defaults.set("obsolete-client", forKey: obsoleteKeys[2])

        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-host-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let database = workspace.appendingPathComponent("nola.db")
        let firstClientID: String
        do {
            let canonical = try MacApplicationPort(databaseURL: database)
            let configuration = MacRemoteHostConfigurationStore(applicationPort: canonical)
            XCTAssertEqual(configuration.displayValues().url, "")
            try configuration.save(url: "https://host.example/v1", token: "test-token", hostID: "host-1")
            firstClientID = try configuration.clientID()
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let configuration = MacRemoteHostConfigurationStore(applicationPort: reopened)
        XCTAssertEqual(configuration.displayValues().url, "https://host.example/v1")
        XCTAssertEqual(configuration.displayValues().hostID, "host-1")
        XCTAssertEqual(try configuration.clientID(), firstClientID)
        try configuration.save(url: "", token: "", hostID: nil)

        let finalStore = try MacApplicationPort(databaseURL: database)
        let finalConfiguration = MacRemoteHostConfigurationStore(applicationPort: finalStore)
        XCTAssertFalse(finalConfiguration.isConfigured)
        XCTAssertEqual(finalConfiguration.displayValues().url, "")
        XCTAssertNil(try finalStore.productSetting(key: MacProductSettingKeys.connectedHostConfiguration))
    }
}
