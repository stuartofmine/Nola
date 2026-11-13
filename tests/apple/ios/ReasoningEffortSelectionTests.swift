// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class ReasoningEffortSelectionTests: XCTestCase {
    func testEveryDeclaredEffortSurvivesProfileAndConversationRoundTrips() throws {
        for effort in ["none", "minimal", "low", "medium", "high", "xhigh", "max", "automatic"] {
            var parameters = ModelParameters.webAlignedCloudDefaults
            parameters.reasoningPreference = effort
            parameters.temperature = 0.25
            let profile = String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self)
            XCTAssertEqual(try ModelParameters.decodeProfile(profile).reasoningPreference, effort)
            let settings = try ConversationParameterSnapshot.updating("{\"unrelated\":true}", modelKey: "configured:fixture", parameters: parameters)
            let restored = try ConversationParameterSnapshot.read(settings, modelKey: "configured:fixture")
            XCTAssertEqual(restored.reasoningPreference, effort)
            XCTAssertEqual(restored.temperature, 0.25)
            XCTAssertEqual(try IOSConversationSettingsObject.decode(settings)["unrelated"] as? Bool, true)
            XCTAssertEqual(try ConversationParameterSnapshot.applyingOverrides(settings, modelKey: "configured:fixture", to: .webAlignedCloudDefaults).reasoningPreference, effort)
        }
    }
}
