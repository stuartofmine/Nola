// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class PromptTemplateAuthorizationTests: XCTestCase {
    private final class Persistence: PromptTemplatePersistence, @unchecked Sendable {
        var value: PromptTemplateSnapshot?
        var writes = 0
        init(_ value: PromptTemplateSnapshot?) { self.value = value }
        func load() throws -> PromptTemplateSnapshot? { value }
        func save(_ snapshot: PromptTemplateSnapshot) throws { writes += 1; value = snapshot }
    }

    func testLoadingUnownedTemplateDoesNotAdoptOrRewriteIt() throws {
        let original = PromptTemplate(name: "Unowned", content: "private", description: "")
        let persistence = Persistence(.init(customTemplates: [original], selectedTemplateID: original.id))
        let service = PromptTemplateService(logger: Logger(), persistence: persistence)
        XCTAssertEqual(persistence.writes, 0)
        XCTAssertNil(service.templates.first(where: { $0.id == original.id })?.accessOwnerID)
        XCTAssertThrowsError(try service.instructionTemplate(id: original.id))
        XCTAssertThrowsError(try service.getTemplate(for: "en"))
        XCTAssertEqual(persistence.value?.customTemplates, [original])
        XCTAssertEqual(persistence.value?.selectedTemplateID, original.id)
        XCTAssertEqual(persistence.writes, 0)
    }

    func testMissingExplicitSelectionNeverFallsBackToBuiltin() throws {
        let missing = UUID()
        let persistence = Persistence(.init(customTemplates: [], selectedTemplateID: missing))
        let service = PromptTemplateService(logger: Logger(), persistence: persistence)
        service.selectBuiltInLanguage(preferredLanguages: ["en"])
        XCTAssertThrowsError(try service.getCurrentTemplate()) { XCTAssertEqual($0 as? PromptTemplateReadError, .missing(missing)) }
        XCTAssertEqual(service.selectedTemplateID, missing)
        XCTAssertEqual(persistence.writes, 0)
    }

    func testBuiltinIdentityIsStableAndLanguageFilterDoesNotRemoveDefinitions() throws {
        let first = BuiltInPromptTemplates.templates()
        let second = BuiltInPromptTemplates.templates()
        XCTAssertEqual(first.count, 18)
        XCTAssertEqual(first.map(\.id), second.map(\.id))
        XCTAssertEqual(Set(first.map(\.id)).count, 18)
        let service = PromptTemplateService(logger: Logger())
        service.selectBuiltInLanguage(preferredLanguages: ["en"])
        XCTAssertEqual(Set(service.templates.map(\.id)), Set(first.map(\.id)))
        for definition in first { XCTAssertEqual(try service.instructionTemplate(id: definition.id).content, definition.content) }
    }

    func testStrictRecordRejectsUnknownAuthorityAndMissingRequiredDates() throws {
        let original = PromptTemplate(name: "Current", content: "body", description: "")
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any])
        value["host_id"] = "not-a-supported-template-field"
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplate.self, from: JSONSerialization.data(withJSONObject: value)))
        value.removeValue(forKey: "host_id")
        value.removeValue(forKey: "createdAt")
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplate.self, from: JSONSerialization.data(withJSONObject: value)))
    }

    func testNullAndAbsentAuthorityFieldsRoundTripWithoutNormalization() throws {
        let original = PromptTemplate(name: "Current", content: "body", description: "")
        var record = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any])
        record["accessOwnerID"] = NSNull()
        record["accessGrants"] = NSNull()
        let root: [String: Any] = ["customTemplates": [record], "selectedTemplateID": NSNull()]
        let decoded = try JSONDecoder().decode(PromptTemplateSnapshot.self, from: JSONSerialization.data(withJSONObject: root))
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(decoded)) as? NSDictionary)
        XCTAssertEqual(encoded, root as NSDictionary)
        var unknown = root
        unknown["hostId"] = "wrong"
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplateSnapshot.self, from: JSONSerialization.data(withJSONObject: unknown)))
    }

    func testExpertGuidanceUsesActualParameterProducerAndKeepsExactBytes() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.responseStyle = .casual
        parameters.customSystemPrompt = "  Keep the precise source wording.\n第二行  "
        let captured = IOSExpertInstructionProducer.definitions(parameters: parameters)
        XCTAssertEqual(captured.map(\.component_id), ["ios:user-response-style", "ios:user-custom-instructions"])
        XCTAssertEqual(captured[0].content, PromptTemplates.agentResponseStyle(parameters.responseStyle))
        XCTAssertEqual(captured[1].content, "Keep the precise source wording.\n第二行")
        parameters.customSystemPrompt = "later configuration"
        XCTAssertEqual(captured[1].content, "Keep the precise source wording.\n第二行", "A captured value does not re-read later settings")
        XCTAssertNotEqual(captured[1].content_sha256, IOSExpertInstructionProducer.definitions(parameters: parameters)[1].content_sha256)
    }

    func testPreviewPresentationFeedsSameNativeDecoderWithoutChangingModelGeneration() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.25
        let updated = try ConversationParameterSnapshot.applyingPresentation(.object([
            "responseStyle": .string(PromptResponseStyle.casual.rawValue), "customSystemPrompt": .string("exact custom instruction"),
            "showThinking": .bool(true), "streamOutput": .bool(true),
        ]), to: parameters)
        XCTAssertEqual(updated.temperature, 0.25)
        XCTAssertEqual(updated.maxOutputTokens, parameters.maxOutputTokens)
        XCTAssertEqual(IOSExpertInstructionProducer.definitions(parameters: updated)[1].content, "exact custom instruction")
        XCTAssertThrowsError(try ConversationParameterSnapshot.applyingPresentation(.object(["customSystemPrompt": .bool(false)]), to: parameters))
    }

    func testPrivateHostDefiniteRejectionRetiresOnlyItsOwnPendingIntent() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let input = ApplePrepareEditedResend(operation_id: UUID().uuidString.lowercased(),
            source_conversation_id: "missing-source", source_message_id: "missing-message", expected_source_revision: 1,
            expected_source_message_revision: 1, edited_text: "new text", target_conversation_id: UUID().uuidString,
            title: "New", created_at_ms: 1)
        // This is a real Core rejection against an isolated Store. No model,
        // transport mock or fabricated success receipt participates.
        for _ in 0..<2 {
            XCTAssertThrowsError(try runtime.expertHostOperation(.prepareResend(input: input, authority: .local())) as ApplePreparedExpertResend<AgentCoreJSONValue>) {
                guard case IOSApplicationRuntimeError.rejected = $0 else { return XCTFail("Expected a definite Core rejection: \($0)") }
            }
            let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
                validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
            let transaction = try journal.beginTransaction()
            defer { transaction.close() }
            XCTAssertTrue(try transaction.load().isEmpty)
        }
    }
}
