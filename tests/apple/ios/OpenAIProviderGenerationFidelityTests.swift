import Foundation
import XCTest
@testable import Nola

final class OpenAIProviderGenerationFidelityTests: XCTestCase {
    func testEndpointCannotRewriteConfiguredUpstreamModel() throws {
        for host in ["api-inference.modelscope.cn", "api-inference.modelscope.cn.fixture.invalid"] {
            for model in ["deepseek-v3", OpenAIProvider.officialModelPlaceholder] {
                var config = configuration()
                config.baseURL = try XCTUnwrap(URL(string: "https://\(host)/v1"))
                config.model = model
                XCTAssertEqual(try requestBody(config)["model"] as? String, model)
            }
        }
    }

    @MainActor
    func testRunFactoryPreservesStopWhitespaceThroughActualRequest() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.stopSequences = "  boundary  \n \nEND\r\n"
        let model = ModelInfo(id: "profile", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "test-only", upstreamModelId: "exact-model",
            wireProtocol: .chatCompletions, executionCapabilities: .init(samplingParameters: true))
        let config = try IOSConversationRunConfiguration.directCloudProviderConfig(model: model, parameters: parameters)
        XCTAssertEqual(try requestBody(config)["stop"] as? [String], ["  boundary  ", " ", "END"])
    }

    @MainActor
    func testAnonymousModelRequiresExplicitDeclarationAndOmitsAuthorization() throws {
        let model = ModelInfo(id: "anonymous", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", upstreamModelId: "exact-model", wireProtocol: .responses,
            cloudCredential: .anonymous)
        let config = try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: model, parameters: .webAlignedCloudDefaults)
        let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: [
            "prepared_core_payload": "true",
            "prepared_core_payload_json": #"{"system_prompt":"system","messages":[{"role":"user","content":"hello"}],"tools":[]}"#,
        ])
        let request = try OpenAIProvider(config: config).responsesRequest(input: input)
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
        XCTAssertThrowsError(try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: model.withCloudCredential(nil, apiKey: nil), parameters: .webAlignedCloudDefaults))
        XCTAssertThrowsError(try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: model.withCloudCredential(.keychain(account: "version:\(UUID().uuidString.lowercased())"), apiKey: nil),
            parameters: .webAlignedCloudDefaults))
    }

    func testChatSendsCompleteSharedSchemaStopListAndExactPenalties() throws {
        var config = configuration()
        config.presencePenalty = -2
        config.frequencyPenalty = 2
        config.temperature = 0
        config.topP = 0
        config.seed = 0
        config.stopSequences = ["  boundary  ", "\n", String(repeating: "界", count: 682)]
            + (3..<16).map { "stop-\($0)" }
        let body = try requestBody(config)
        XCTAssertEqual(body["stop"] as? [String], config.stopSequences)
        XCTAssertEqual(body["presence_penalty"] as? Double, -2)
        XCTAssertEqual(body["frequency_penalty"] as? Double, 2)
        XCTAssertEqual(body["temperature"] as? Double, 0)
        XCTAssertEqual(body["top_p"] as? Double, 0)
        XCTAssertEqual(body["seed"] as? Int, 0)
    }

    func testInvalidValuesRejectInsteadOfClampingTruncatingOrDropping() throws {
        let mutations: [(inout OpenAIProvider.Config) -> Void] = [
            { $0.presencePenalty = -2.01 }, { $0.frequencyPenalty = 2.01 },
            { $0.presencePenalty = .nan }, { $0.frequencyPenalty = .infinity },
            { $0.temperature = .infinity }, { $0.topP = -.infinity },
            { $0.stopSequences = Array(repeating: "stop", count: 17) },
            { $0.stopSequences = [""] },
            { $0.stopSequences = [String(repeating: "界", count: 683)] },
        ]
        for mutate in mutations {
            var config = configuration()
            mutate(&config)
            XCTAssertThrowsError(try requestBody(config))
        }
    }

    func testUndeclaredSamplingRejectsWhitespaceStopInsteadOfSilentlyDroppingIt() throws {
        var config = configuration()
        config.executionCapabilities = .init()
        config.stopSequences = [" "]
        XCTAssertThrowsError(try requestBody(config))
    }

    func testResponsesStillExplicitlyRejectsUnsupportedOptions() throws {
        let mutations: [(inout OpenAIProvider.Config) -> Void] = [
            { $0.presencePenalty = 0.5 }, { $0.frequencyPenalty = -0.5 },
            { $0.seed = 0 }, { $0.stopSequences = ["stop"] },
        ]
        let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: [
            "prepared_core_payload": "true",
            "prepared_core_payload_json": #"{"system_prompt":"system","messages":[{"role":"user","content":"hello"}],"tools":[]}"#,
        ])
        for mutate in mutations {
            var config = configuration()
            config.wireProtocol = .responses
            mutate(&config)
            XCTAssertThrowsError(try OpenAIProvider(config: config).responsesRequest(input: input))
        }
    }

    func testResponsesSummarySupportIsIndependentOfReasoningAndThinkingPreference() throws {
        for support: Bool? in [nil, false, true] {
            var config = configuration()
            config.wireProtocol = .responses
            config.executionCapabilities = .init(reasoning: true)
            config.reasoningSummarySupported = support
            let roundTrip = try JSONDecoder().decode(OpenAIProvider.Config.self, from: JSONEncoder().encode(config))
            XCTAssertEqual(roundTrip.reasoningSummarySupported, support)
            for thinking in [false, true] {
                let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: [
                    "enable_thinking": thinking ? "true" : "false", "prepared_core_payload": "true",
                    "prepared_core_payload_json": #"{"system_prompt":"system","messages":[{"role":"user","content":"fixture"}],"tools":[]}"#,
                ])
                let request = try OpenAIProvider(config: roundTrip).responsesRequest(input: input)
                let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
                let summary = (body["reasoning"] as? [String: Any])?["summary"] as? String
                XCTAssertEqual(summary, thinking && support != false ? "auto" : nil)
            }
        }
    }

    func testResponsesPreservesCoreEffortAndDoesNotRequestSummaryForNone() throws {
        for effort in ["low", "medium", "high", "none"] {
            var config = configuration()
            config.wireProtocol = .responses
            config.executionCapabilities = .init(reasoning: true)
            config.reasoningSelection = .object(["kind": .string("effort"), "value": .string(effort)])
            let roundTrip = try JSONDecoder().decode(OpenAIProvider.Config.self, from: JSONEncoder().encode(config))
            let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: [
                "prepared_core_payload": "true",
                "prepared_core_payload_json": #"{"system_prompt":"system","messages":[{"role":"user","content":"fixture"}],"tools":[]}"#,
            ])
            let request = try OpenAIProvider(config: roundTrip).responsesRequest(input: input)
            let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
            let reasoning = try XCTUnwrap(body["reasoning"] as? [String: Any])
            XCTAssertEqual(reasoning["effort"] as? String, effort)
            XCTAssertEqual(reasoning["summary"] as? String, effort == "none" ? nil : "auto")
        }
    }

    private func configuration() -> OpenAIProvider.Config {
        .init(baseURL: URL(string: "https://fixture.invalid/v1")!, apiKey: "test-only", model: "fixture",
            executionCapabilities: .init(samplingParameters: true))
    }

    private func requestBody(_ config: OpenAIProvider.Config) throws -> [String: Any] {
        let data = try OpenAIProvider(config: config).buildRequestBody(system: "system", userContent: "hello",
            attachments: [], stream: false, enableThinking: false)
        return try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }
}
