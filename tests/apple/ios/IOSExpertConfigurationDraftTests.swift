import XCTest
@testable import Nola

final class IOSExpertConfigurationDraftTests: XCTestCase {
    func testMetadataOnlySavePreservesNullAbsentAndOpaqueDefaultsExactly() throws {
        let originals: [AgentCoreJSONValue] = [.object([:]), .object([
            "modelSelectionID": .null, "selectedPromptTemplateID": .null,
            "modelOverrides": .object(["configured:prior": .object(["temperature": .number(0.27)])]),
            "language": .string("zh"), "unknownCoreOwnedField": .array([.null, .bool(true)])])]
        for original in originals {
            let draft = try AppleExpertConfigurationDraft(defaults: original)
            XCTAssertEqual(try draft.applying(to: original), original)
        }
    }

    func testExplicitSelectorsOnlyChangeTheirOwnKeys() throws {
        let original: AgentCoreJSONValue = .object([
            "modelSelectionID": .string("configured:old"), "selectedPromptTemplateID": .string("original-template"),
            "modelOverrides": .object(["configured:old": .object(["temperature": .number(0.25)])]),
            "language": .string("de")])
        var draft = try AppleExpertConfigurationDraft(defaults: original)
        draft.modelID = "configured:new"
        guard case .object(var expected) = original else { return XCTFail("Expected object") }
        expected["modelSelectionID"] = .string("configured:new")
        XCTAssertEqual(try draft.applying(to: original), .object(expected))
        draft.modelID = ""
        draft.templateID = ""
        expected.removeValue(forKey: "modelSelectionID"); expected.removeValue(forKey: "selectedPromptTemplateID")
        XCTAssertEqual(try draft.applying(to: original), .object(expected))
    }

    func testMalformedSelectorIsRejectedRatherThanReplaced() {
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .array([])))
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["modelSelectionID": .bool(false)])))
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["selectedPromptTemplateID": .signedInteger(7)])))
    }
}
