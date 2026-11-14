// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements URLDataSourceBoundaryTests logic in the project module.
// - Interfaces:
//   - URLDataSourceBoundaryTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class URLDataSourceBoundaryTests: XCTestCase {
    func testURLDataSourceClassifiesScheduledAutomation() {
        var urlAutomation = URLDataSource(
            url: "https://example.com/feed",
            format: .rss,
            name: "Daily Feed"
        )
        urlAutomation.scheduleFrequency = "daily"
        urlAutomation.scheduledTimeISO = "08:00"

        XCTAssertTrue(urlAutomation.isScheduledAutomation)
    }

    func testSystemDataSourcePolicyPersistsAndRespectsConversationOverrides() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-source-policy-\(UUID().uuidString)", isDirectory: true)
        defer {
            try? FileManager.default.removeItem(at: root)
        }
        let store = SystemDataSourceAccessPolicyStore(
            applicationRuntime: try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        )

        XCTAssertEqual(store.policy(for: .health), .conversationOptIn)
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: nil))

        store.setPolicy(.defaultEnabled, for: .health)
        XCTAssertEqual(store.policy(for: .health), .defaultEnabled)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: nil))
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: false))

        store.setPolicy(.conversationOptIn, for: .health)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: true))

        store.setPolicy(.blocked, for: .health)
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: true))
        XCTAssertEqual(store.conversationOverrideKey(for: .health), "system:health")
    }
}
