// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

final class MacRemoteHostConfigurationCanonicalEndToEndTests: XCTestCase {
    func testBlankHostDraftCannotReuseCredentialAtAnotherDestination() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("host-draft-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("host.sqlite"))
        let store = MacRemoteHostConfigurationStore(applicationPort: application)
        try store.save(url: "https://old.example:5001/v1", token: "old-token", hostID: "old-host")
        let original = try XCTUnwrap(URL(string: "https://old.example:5001/v1"))
        XCTAssertEqual(store.tokenForDraft(url: original, enteredToken: ""), "old-token")
        for address in ["https://new.example:5001/v1", "http://old.example:5001/v1",
                        "https://old.example:5002/v1", "https://old.example:5001/other"] {
            let destination = try XCTUnwrap(URL(string: address))
            XCTAssertNil(store.tokenForDraft(url: destination, enteredToken: ""), address)
            XCTAssertEqual(store.tokenForDraft(url: destination, enteredToken: " new-token "), "new-token")
        }
        XCTAssertEqual(store.load()?.baseURL, original)
        XCTAssertEqual(store.load()?.token, "old-token")
    }

    func testFailedCredentialWriteNeverCommitsConfiguration() {
        var configurationWritten = false
        XCTAssertThrowsError(try MacHostConfigurationPersistence.commit(token: "new", previousToken: "old", writeToken: { _ in
            throw NSError(domain: "fixture-keychain", code: 1)
        }, writeConfiguration: { configurationWritten = true }))
        XCTAssertFalse(configurationWritten)
    }

    func testFailedConfigurationWriteRestoresPreviousCredential() {
        var tokens: [String] = []
        XCTAssertThrowsError(try MacHostConfigurationPersistence.commit(token: "new", previousToken: "old", writeToken: { tokens.append($0) }, writeConfiguration: {
            throw NSError(domain: "fixture-database", code: 1)
        }))
        XCTAssertEqual(tokens, ["new", "old"])
    }

    func testSharedAddressUsesPrivateLanOrBonjourInsteadOfMachineHostname() throws {
        let bound = try XCTUnwrap(URL(string: "http://0.0.0.0:5001"))
        XCTAssertEqual(MacHostAdvertisedAddress.origin(bound, addresses: ["127.0.0.1", "192.168.1.8"], localHostName: "studio")?.absoluteString, "http://192.168.1.8:5001")
        XCTAssertEqual(MacHostAdvertisedAddress.origin(bound, addresses: [], localHostName: "studio")?.host, "studio.local")
        XCTAssertNil(MacHostAdvertisedAddress.origin(bound, addresses: [], localHostName: nil))
    }

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
