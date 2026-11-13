// SPDX-License-Identifier: Apache-2.0
import CryptoKit
import XCTest
@testable import Nola

@MainActor final class MacExpertTemplateHostTests: XCTestCase {
    func testParentExpertReplyExcludesPrivateExecutionDetails() {
        XCTAssertTrue(MacExpertReplyPresentation.includes(.text(.init(is_final: true, visibility: "public", review: nil))))
        XCTAssertTrue(MacExpertReplyPresentation.includes(.text(.init(is_final: false, visibility: "public", review: nil))))
        XCTAssertFalse(MacExpertReplyPresentation.includes(.text(.init(is_final: true, visibility: "private", review: nil))))
        XCTAssertFalse(MacExpertReplyPresentation.includes(.reasoning(.init(title: .inline("Thinking"), state: "running"))))
        XCTAssertFalse(MacExpertReplyPresentation.includes(.step(.init(title: .inline("Executing"), state: "completed"))))
    }

    func testHostInstructionProducerPreservesStyleAndNonemptyCustomDefinitionExactly() {
        let definitions = MacExpertInstructionProducer.definitions(style: .detailed,
            customSystemPrompt: "  My precise model instructions.\n")
        XCTAssertEqual(definitions.map(\.component_id), ["macos:response-depth", "macos:user-custom-instructions"])
        XCTAssertEqual(definitions[0].content, MacResponseStyle.detailed.responseInstruction)
        XCTAssertEqual(definitions[1].content, "My precise model instructions.")
        for definition in definitions {
            XCTAssertNil(definition.definition_revision)
            XCTAssertEqual(definition.platform, .string("macos"))
            XCTAssertEqual(definition.content_sha256, SHA256.hash(data: Data(definition.content.utf8))
                .map { String(format: "%02x", $0) }.joined())
        }
    }

    func testBlankCustomDefinitionIsNotRegisteredAndDoesNotSuppressResponseStyle() {
        let definitions = MacExpertInstructionProducer.definitions(style: .balanced, customSystemPrompt: " \n\t")
        XCTAssertEqual(definitions.count, 1)
        XCTAssertEqual(definitions[0].component_id, "macos:response-depth")
        XCTAssertEqual(definitions[0].content, MacResponseStyle.balanced.responseInstruction)
    }

    func testCaptureSelectionCarriesRequiredExactModelPinAndExplicitNilTemplate() throws {
        let selection = AppleExpertCaptureSelection<MacCanonicalJSONValue>(
            context_conversation_id: "child", context_configuration_revision: 7,
            model_profile: .init(profile_id: "profile", admitted_revision: 11),
            source: .noTemplate, template_id: nil,
            host_user_instructions: MacExpertInstructionProducer.definitions(style: .concise, customSystemPrompt: "Original"))
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
        let model = try XCTUnwrap(value["model_profile"] as? [String: Any])
        XCTAssertEqual(model["profile_id"] as? String, "profile")
        XCTAssertEqual(model["admitted_revision"] as? Int, 11)
        XCTAssertTrue(value["template_id"] is NSNull)
        XCTAssertEqual((value["host_user_instructions"] as? [[String: Any]])?.count, 2)
    }
}
