// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class IOSExpertPresentationTests: XCTestCase {
    func testExpertColorPreferencePersistsByStableAgentIdentity() throws {
        let suite = "IOSExpertPresentationTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let preferences = IOSExpertColorPreferences(defaults: defaults)
        let otherDefault = preferences.color(for: "other-agent")
        preferences.save(.pink, for: "agent-one")
        XCTAssertEqual(preferences.color(for: "agent-one"), .pink)
        XCTAssertEqual(preferences.color(for: "other-agent"), otherDefault)
        let reloaded = IOSExpertColorPreferences(defaults: defaults)
        XCTAssertEqual(reloaded.color(for: "agent-one"), .pink)
        reloaded.save(.green, for: "agent-one")
        XCTAssertEqual(IOSExpertColorPreferences(defaults: defaults).color(for: "agent-one"), .green)
    }

    func testAppleAppearanceUsesOneJSONDataContractAndBlueDefault() throws {
        let suite = "IOSExpertPresentationTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let preferences = IOSExpertColorPreferences(defaults: defaults)
        XCTAssertEqual(preferences.color(for: "agent-a"), .blue)
        XCTAssertEqual(preferences.color(for: "agent-b"), .blue)
        preferences.save(.teal, for: "agent-a")
        let data = try XCTUnwrap(defaults.data(forKey: AppleExpertAppearance.storageKey))
        XCTAssertEqual(try JSONDecoder().decode([String: String].self, from: data), ["agent-a": "teal"])
        XCTAssertEqual(AppleExpertAppearance.selected(data, agentID: "agent-a"), .teal)
        XCTAssertNil(AppleExpertAppearance.saving(.red, agentID: "", in: data))
    }

    func testWorkspaceManagementRetainsAllExistingDestinations() {
        XCTAssertEqual(Set(IOSWorkspaceDestination.allCases.map(\.rawValue)),
            Set(["sources", "knowledge", "memory", "plans", "records", "models", "settings"]))
        XCTAssertNil(IOSWorkspaceDestination(page: -1))
        XCTAssertEqual(IOSWorkspaceDestination(page: 0), .sources)
        XCTAssertEqual(IOSWorkspaceDestination(page: 3), .records)
    }
}
