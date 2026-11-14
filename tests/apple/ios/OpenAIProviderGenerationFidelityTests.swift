import Foundation
import XCTest
@testable import Nola

final class OpenAIProviderGenerationFidelityTests: XCTestCase {
    @MainActor
    func testToolReasoningRejectionDoesNotSilentlyDisableReasoningOrSwitchProtocol() async throws {
        URLProtocol.registerClass(ToolReasoningFixture.self)
        ToolReasoningFixture.requests = []
        defer { URLProtocol.unregisterClass(ToolReasoningFixture.self) }
        var config = configuration()
        config.baseURL = URL(string: "https://protocol-compatibility.invalid/v1")!
        config.executionCapabilities = .init(toolCalls: true, reasoning: true)
        config.reasoningSelection = .object(["kind": .string("effort"), "value": .string("high")])
        var input = try IOSModelInferenceTestRoute.cloudInput(configuration: config, parameters: .webAlignedCloudDefaults, message: "hello")
        do {
            for try await _ in OpenAIProvider(config: config).generateSummaryStream(input: input) { }
            XCTFail("The tool/reasoning compatibility error must reach the model check")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("reasoning_effort"))
        }
        XCTAssertEqual(ToolReasoningFixture.requests.count, 1, "Never silently retry a changed model request")
        config.wireProtocol = .responses
        input = try IOSModelInferenceTestRoute.cloudInput(configuration: config, parameters: .webAlignedCloudDefaults, message: "hello")
        var answer = ""
        for try await chunk in OpenAIProvider(config: config).generateSummaryStream(input: input) { answer += chunk }
        XCTAssertEqual(answer, "fixture-ok")
        XCTAssertEqual(ToolReasoningFixture.requests.count, 2)
        let last = try XCTUnwrap(ToolReasoningFixture.requests.last)
        XCTAssertEqual(last.0, "/v1/responses")
        XCTAssertEqual((last.1["reasoning"] as? [String: Any])?["effort"] as? String, "high")
        XCTAssertEqual(last.1["tool_choice"] as? String, "none")
    }

    @MainActor
    func testConnectionCheckAdvertisesInertToolsWithTheExactReasoningSelection() throws {
        for effort: String? in [nil, "none", "high"] {
            var config = configuration()
            config.wireProtocol = .responses
            config.executionCapabilities = .init(toolCalls: true, reasoning: true)
            config.reasoningSelection = effort.map { .object(["kind": .string("effort"), "value": .string($0)]) }
            let input = try IOSModelInferenceTestRoute.cloudInput(configuration: config,
                parameters: .webAlignedCloudDefaults, message: "hello")
            let request = try OpenAIProvider(config: config).responsesRequest(input: input)
            XCTAssertEqual(request.url?.path, "/v1/responses")
            let body = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
            XCTAssertEqual((body["reasoning"] as? [String: Any])?["effort"] as? String, effort)
            XCTAssertEqual(body["tool_choice"] as? String, "none")
            XCTAssertEqual((body["tools"] as? [[String: Any]])?.first?["name"] as? String, "nola_connection_check")
        }
    }

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

private final class ToolReasoningFixture: URLProtocol, @unchecked Sendable {
    @MainActor static var requests: [(String, [String: Any])] = []
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "protocol-compatibility.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() { }
    override func startLoading() {
        let request = request
        Task { @MainActor in
            do {
                var data = request.httpBody ?? Data()
                if data.isEmpty, let stream = request.httpBodyStream {
                    stream.open(); defer { stream.close() }
                    var bytes = [UInt8](repeating: 0, count: 4096)
                    while stream.hasBytesAvailable {
                        let count = stream.read(&bytes, maxLength: bytes.count)
                        guard count > 0 else { break }
                        data.append(contentsOf: bytes.prefix(count))
                    }
                }
                let body = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                let url = try XCTUnwrap(request.url)
                Self.requests.append((url.path, body))
                let rejected = url.path.hasSuffix("/chat/completions")
                    && !(body["tools"] as? [Any] ?? []).isEmpty && body["reasoning_effort"] as? String != "none"
                let payload: Data
                if rejected {
                    payload = try JSONSerialization.data(withJSONObject: ["error": ["type": "invalid_request_error", "param": "reasoning_effort", "code": "unsupported_parameter",
                        "message": "Function tools with reasoning_effort are not supported in /v1/chat/completions. Use /v1/responses or explicitly set reasoning_effort to none."]])
                } else {
                    let events: [[String: Any]] = [
                        ["type": "response.output_text.delta", "delta": "fixture-ok"],
                        ["type": "response.completed", "response": ["id": "fixture-response", "status": "completed", "output": []]],
                    ]
                    payload = try events.reduce(into: Data()) { output, event in
                        output.append(Data("data: ".utf8)); output.append(try JSONSerialization.data(withJSONObject: event)); output.append(Data("\n\n".utf8))
                    }
                }
                let response = HTTPURLResponse(url: url, statusCode: rejected ? 400 : 200, httpVersion: nil,
                    headerFields: ["Content-Type": rejected ? "application/json" : "text/event-stream"])!
                client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                client?.urlProtocol(self, didLoad: payload)
                client?.urlProtocolDidFinishLoading(self)
            } catch { client?.urlProtocol(self, didFailWithError: error) }
        }
    }
}
