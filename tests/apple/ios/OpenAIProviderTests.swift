// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements OpenAIProviderTests logic in the project module.
// - Interfaces:
//   - OpenAIProviderTests
// - APIs:
//   - none
//
import XCTest
import Foundation

@testable import Nola

final class OpenAIProviderTests: XCTestCase {
    private static var retainedProviders: [OpenAIProvider] = []

    @MainActor func testChatPublicReasoningAliasesAreReceivedWithoutRequestingControlledReasoning() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ResponsesFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let provider = OpenAIProvider(config: .init(baseURL: URL(string: "https://fixture.invalid/chat-public/v1")!,
            apiKey: "fixture", model: "fixture", wireProtocol: .chatCompletions,
            executionCapabilities: .init(reasoning: false)), streamSession: session)
        Self.retainedProviders.append(provider)
        var reasoning: [String] = []
        var text = ""
        var completed = false
        for try await event in provider.generateEventStream(input: ModelInput(dateRange: .init(start: "", end: ""),
            candidates: [], userPrefs: ["message": "fixture", "enable_thinking": "false"])) {
            switch event {
            case .reasoningDelta(let delta): XCTAssertFalse(completed); reasoning.append(delta)
            case .textDelta(let delta): text += delta
            case .completed: completed = true
            default: break
            }
        }
        XCTAssertEqual(reasoning, ["Public ", "reasoning"])
        XCTAssertEqual(text, "answer")
        XCTAssertTrue(completed)
    }

    func testResponsesPublicReasoningDeltasAndTerminalSuffix() throws {
        var progress = ResponsesReasoningProgress()
        XCTAssertEqual(try progress.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "delta": "正在"]), "正在")
        XCTAssertEqual(try progress.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "delta": "分析"]), "分析")
        XCTAssertNil(try progress.acceptDelta(["type": "response.reasoning_summary_text.delta", "item_id": "r1", "delta": "duplicate representation"]))
        let output: [[String: Any]] = [["id": "r1", "type": "reasoning", "summary": [],
            "content": [["type": "reasoning_text", "text": "正在分析结果"]], "encrypted_content": "private-opaque"]]
        XCTAssertEqual(try progress.finish(output), ["结果"])
        XCTAssertEqual(try progress.finish([["id": "r1", "type": "reasoning", "content": [["type": "reasoning_text", "text": "正在分析"]]]]), [])
        XCTAssertThrowsError(try progress.finish([["id": "r1", "type": "reasoning", "content": [["type": "reasoning_text", "text": "contradictory"]]]]))
    }

    func testResponsesReasoningIdentityAliasesAndAmbiguousUnscopedOutput() throws {
        var progress = ResponsesReasoningProgress()
        XCTAssertEqual(try progress.acceptDelta(["type": "response.reasoning_text.delta", "output_index": 0, "delta": "A"]), "A")
        XCTAssertEqual(try progress.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "output_index": 0, "delta": "B"]), "B")
        XCTAssertEqual(try progress.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "delta": "C"]), "C")
        XCTAssertEqual(try progress.finish([["id": "r1", "type": "reasoning", "content": [["type": "reasoning_text", "text": "ABC!"]]]]), ["!"])
        var unscoped = ResponsesReasoningProgress()
        _ = try unscoped.acceptDelta(["type": "response.reasoning_text.delta", "delta": "A"])
        XCTAssertThrowsError(try unscoped.finish([["type": "reasoning"], ["type": "reasoning"]]))
        var conflicting = ResponsesReasoningProgress()
        _ = try conflicting.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "delta": "A"])
        _ = try conflicting.acceptDelta(["type": "response.reasoning_text.delta", "output_index": 0, "delta": "B"])
        XCTAssertThrowsError(try conflicting.acceptDelta(["type": "response.reasoning_text.delta", "item_id": "r1", "output_index": 0, "delta": "C"]))
    }

    func testResponsesTerminalReasoningUsesOnlyPublicContent() throws {
        let progress = ResponsesReasoningProgress()
        XCTAssertEqual(try progress.finish([["type": "reasoning", "summary": [], "content": [["type": "reasoning_text", "text": "公开思考"]], "encrypted_content": "opaque"]]), ["公开思考"])
        XCTAssertEqual(try progress.finish([["type": "reasoning", "summary": [["type": "summary_text", "text": "summary"]], "content": [["type": "reasoning_text", "text": "plain"]]]]), ["summary"])
        XCTAssertEqual(try progress.finish([["type": "reasoning", "encrypted_content": "opaque"]]), [])
    }

    @MainActor func testResponsesStreamEmitsPlainReasoningBeforeTerminalWithoutDuplicatingSnapshot() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ResponsesFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let provider = OpenAIProvider(config: .init(
            baseURL: URL(string: "https://fixture.invalid/reasoning/v1")!,
            apiKey: "fixture", model: "fixture", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: true)), streamSession: session)
        Self.retainedProviders.append(provider)
        var reasoning: [String] = []
        var visibleOrder: [String] = []
        var completed = false
        var continuationState = ""
        for try await event in provider.generateEventStream(input: ModelInput(
            dateRange: .init(start: "", end: ""), candidates: [], userPrefs: ["message": "fixture", "enable_thinking": "false"])) {
            switch event {
            case .reasoningDelta(let delta):
                XCTAssertFalse(completed)
                reasoning.append(delta)
                visibleOrder.append("reasoning:" + delta)
            case .textDelta(let delta): visibleOrder.append("text:" + delta)
            case .continuation(let state): continuationState = state
            case .completed: completed = true
            default: break
            }
        }
        XCTAssertEqual(reasoning, ["Live ", "thinking", " suffix"])
        XCTAssertEqual(visibleOrder, ["reasoning:Live ", "reasoning:thinking", "text:answer", "reasoning: suffix"])
        XCTAssertTrue(completed)
        XCTAssertTrue(continuationState.contains("private-opaque"))
        XCTAssertFalse(reasoning.joined().contains("private-opaque"))
    }

    @MainActor func testResponsesPartialUsagePreservesUnknownTotalAndExplicitZero() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ResponsesFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let provider = OpenAIProvider(config: .init(
            baseURL: URL(string: "https://fixture.invalid/partial/v1")!,
            apiKey: "fixture", model: "fixture", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: true)), streamSession: session)
        Self.retainedProviders.append(provider)
        var captured: LLMStreamUsage?
        for try await event in provider.generateEventStream(input: ModelInput(
            dateRange: .init(start: "", end: ""), candidates: [], userPrefs: ["message": "fixture"])) {
            if case .usage(let value) = event { captured = value }
        }
        let usage = try XCTUnwrap(captured)
        XCTAssertEqual(usage.promptTokens, 12)
        XCTAssertEqual(usage.completionTokens, 0)
        XCTAssertEqual(usage.reasoningTokens, 0)
        XCTAssertNil(usage.totalTokens)
        XCTAssertNil(usage.cachedInputTokens)
    }

    func testResponsesProfileCannotBuildChatCompletionsRequest() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://fixture.invalid/v1"))
        let provider = OpenAIProvider(config: .init(
            baseURL: baseURL, apiKey: "fixture", model: "fixture", wireProtocol: .responses))
        XCTAssertThrowsError(try provider.buildRequestBody(
            system: "system", userContent: "hello", attachments: [], stream: true, enableThinking: true))
    }

    func testResponsesRequestUsesExplicitRouteAndStatelessContinuation() throws {
        let provider = OpenAIProvider(config: .init(
            baseURL: URL(string: "https://fixture.invalid/v1")!,
            apiKey: "fixture", model: "fixture", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: true, reasoning: true)
        ))
        Self.retainedProviders.append(provider)
        let payload: [String: Any] = [
            "system_prompt": "system",
            "messages": [
                ["role": "user", "content": "read"],
                ["role": "assistant", "content": "", "tool_calls": [["id": "turn::call_1", "name": "read", "arguments": [:]]]],
                ["role": "tool", "content": "done", "tool_call_id": "turn::call_1"],
            ],
            "tools": [["type": "function", "function": ["name": "read", "parameters": ["type": "object"]]]],
        ]
        let state: [String: Any] = [
            "provider_namespace": "openai.responses", "model_id": "fixture",
            "state": [
                "route_identity": provider.responsesRouteIdentity,
                "response_items": [
                    ["type": "reasoning", "encrypted_content": "opaque"],
                    ["type": "function_call", "call_id": "call_1", "name": "read", "arguments": "{}"],
                ],
            ],
        ]
        let input = ModelInput(
            dateRange: .init(start: "", end: ""), candidates: [],
            userPrefs: [
                "prepared_core_payload": "true",
                "prepared_core_payload_json": String(decoding: try JSONSerialization.data(withJSONObject: payload), as: UTF8.self),
            ],
            providerContinuationJSON: String(decoding: try JSONSerialization.data(withJSONObject: state), as: UTF8.self)
        )
        let request = try provider.responsesRequest(input: input)
        XCTAssertEqual(request.url?.path, "/v1/responses")
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
        XCTAssertEqual(body["store"] as? Bool, false)
        XCTAssertEqual(body["include"] as? [String], ["reasoning.encrypted_content"])
        XCTAssertEqual((body["reasoning"] as? [String: String])?["summary"], "auto")
        XCTAssertNil(body["messages"])
        let items = try XCTUnwrap(body["input"] as? [[String: Any]])
        XCTAssertEqual(items.first(where: { $0["type"] as? String == "reasoning" })?["encrypted_content"] as? String, "opaque")
        XCTAssertEqual(items.last?["call_id"] as? String, "call_1")
        var wrongState = state
        wrongState["state"] = ["route_identity": "other", "response_items": []]
        let wrong = ModelInput(dateRange: .init(start: "", end: ""), candidates: [],
            userPrefs: input.userPrefs,
            providerContinuationJSON: String(decoding: try JSONSerialization.data(withJSONObject: wrongState), as: UTF8.self))
        XCTAssertThrowsError(try provider.responsesRequest(input: wrong))
    }

    @MainActor func testResponsesSSEKeepsToolCallUsageAndEncryptedState() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ResponsesFixtureURLProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let provider = OpenAIProvider(config: .init(
            baseURL: URL(string: "https://fixture.invalid/v1")!,
            apiKey: "fixture", model: "fixture", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: true)
        ), streamSession: session)
        Self.retainedProviders.append(provider)
        let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [],
            userPrefs: ["message": "read a"])
        var events: [LLMStreamEvent] = []
        for try await event in provider.generateEventStream(input: input) { events.append(event) }
        XCTAssertEqual(events.first, .textDelta("Checking."))
        let call = events.compactMap { event -> LLMToolCallDelta? in
            if case .toolCallDelta(let value) = event { return value }
            return nil
        }.first
        XCTAssertEqual(call?.callId, "call_1")
        XCTAssertEqual(call?.capabilityId, "read")
        let state = events.compactMap { event -> [String: Any]? in
            guard case .continuation(let value) = event else { return nil }
            return (try? JSONSerialization.jsonObject(with: Data(value.utf8))) as? [String: Any]
        }.first
        guard let fields = state,
              let providerState = fields["state"] as? [String: Any],
              let items = providerState["response_items"] as? [[String: Any]] else {
            return XCTFail("Responses continuation was not emitted")
        }
        XCTAssertEqual(fields["provider_namespace"] as? String, "openai.responses")
        XCTAssertEqual(items.count, 2)
        XCTAssertEqual(events.last, .completed(finishReason: "stop"))
    }

    func testExplicitLargeOutputAndZeroTemperatureReachRequestBody() throws {
        let provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: URL(string: "https://fixture.invalid/v1")!, apiKey: "fixture", model: "fixture",
            executionCapabilities: ModelExecutionCapabilities(samplingParameters: true, chatOutputTokenField: .maxCompletionTokens),
            temperature: 0, maxCompletionTokens: 64_000
        ))
        Self.retainedProviders.append(provider)
        let data = try provider.buildRequestBody(system: "system", userContent: "hello", attachments: [], stream: true, enableThinking: true)
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(body["max_completion_tokens"] as? Int, 64_000)
        XCTAssertEqual(body["temperature"] as? Double, 0)
    }

    func testUndeclaredMobileExecutionCapabilitiesFailClosed() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://fixture.invalid/v1"))
        let provider = OpenAIProvider(config: .init(baseURL: baseURL, apiKey: "fixture", model: "fixture"))
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with:
            provider.buildRequestBody(system: "system", userContent: "hello", attachments: [], stream: false, enableThinking: true)
        ) as? [String: Any])
        XCTAssertNil(body["temperature"])
        XCTAssertNil(body["top_p"])
        XCTAssertEqual(body["max_tokens"] as? Int, ModelParameters.webAlignedCloudDefaults.maxOutputTokens)
        XCTAssertNil(body["max_completion_tokens"])
        let official = OpenAIProvider(config: .init(baseURL: URL(string: "https://api.openai.com/v1")!, apiKey: "fixture", model: "fixture"))
        let officialBody = try XCTUnwrap(try JSONSerialization.jsonObject(with:
            official.buildRequestBody(system: "system", userContent: "hello", attachments: [], stream: false, enableThinking: true)
        ) as? [String: Any])
        XCTAssertNotNil(officialBody["max_completion_tokens"])
        XCTAssertNil(officialBody["max_tokens"])
        let customized = OpenAIProvider(config: .init(baseURL: baseURL, apiKey: "fixture", model: "fixture", temperature: 0.2))
        XCTAssertThrowsError(try customized.buildRequestBody(system: "system", userContent: "hello", attachments: [], stream: false, enableThinking: true))
        let customPenalty = OpenAIProvider(config: .init(baseURL: baseURL, apiKey: "fixture", model: "fixture", presencePenalty: 0.5))
        XCTAssertThrowsError(try customPenalty.buildRequestBody(system: "system", userContent: "hello", attachments: [], stream: false, enableThinking: true))
        XCTAssertThrowsError(try provider.buildRequestBody(system: "system", userContent: "image", attachments: [
            .init(name: "chart.png", mimeType: "image/png", base64Data: "AQID")
        ], stream: false, enableThinking: true))
    }

    func testStructuredContextOverflowUsesCanonicalCoreFailureWithoutHostRetry() throws {
        let body = try XCTUnwrap(
            #"{"error":{"message":"输入超过上下文限制","code":"context_window_exceeded"}}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: body),
            "context_length_exceeded"
        )
        let chainedErrors = try XCTUnwrap(
            #"{"errors":[{"code":"gateway_failure"},{"message":"入力が長すぎます","type":"prompt_too_long"}]}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: chainedErrors),
            "context_length_exceeded"
        )
        let errorCodeEnvelope = try XCTUnwrap(
            #"{"errors":[{"error_code":"gateway_failure"},{"message":"demasiado largo","error_code":"context_length_error"}]}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: errorCodeEnvelope),
            "context_length_exceeded"
        )
        let requestTooLargeEnvelope = try XCTUnwrap(
            #"{"error":{"error_code":"request_too_large","message":"リクエストが大きすぎます"}}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: requestTooLargeEnvelope),
            "context_length_exceeded"
        )
        let laterTypedField = try XCTUnwrap(
            #"{"error":{"code":"invalid_request","error_code":"context_length_error"}}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: laterTypedField),
            "context_length_exceeded"
        )
        let laterEnvelope = try XCTUnwrap(
            #"{"error":{"code":"gateway_failure"},"errors":[{"type":"invalid_request"},{"error_code":"request_too_large"}]}"#
                .data(using: .utf8)
        )
        XCTAssertEqual(
            OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: laterEnvelope),
            "context_length_exceeded"
        )
        let failure = DeviceAgentCoordinator.modelFailureMetadata(
            NolaError.llm(
                .providerFailure(
                    code: "context_length_exceeded",
                    message: "输入超过上下文限制",
                    retryable: false
                )
            )
        )
        XCTAssertEqual(failure.code, "context_length_exceeded")
        XCTAssertFalse(failure.retryable)
    }

    func testProseAndRequestEchoCannotSelectContextOverflowRecovery() throws {
        let prose = try XCTUnwrap("context_length_exceeded: 输入太长".data(using: .utf8))
        XCTAssertNil(OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: prose))

        let echoedRequest = try XCTUnwrap(
            #"{"error":{"code":"invalid_request"},"request":{"code":"context_length_exceeded"}}"#
                .data(using: .utf8)
        )
        XCTAssertNil(OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: echoedRequest))

        let naturalFollowUp = try XCTUnwrap(
            #"{"error":{"message":"Please search the current context","code":"invalid_request"}}"#
                .data(using: .utf8)
        )
        XCTAssertNil(OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: naturalFollowUp))

        let echoedAlternativeCode = try XCTUnwrap(
            #"{"error":{"error_code":"invalid_request"},"request":{"error_code":"context_length_error"}}"#
                .data(using: .utf8)
        )
        XCTAssertNil(OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: echoedAlternativeCode))

        let malformedTypedFields = try XCTUnwrap(
            #"{"error":{"code":{"value":"context_length_exceeded"},"type":["request_too_large"],"error_code":{"nested":true}}}"#
                .data(using: .utf8)
        )
        XCTAssertNil(OpenAIProvider.canonicalContextOverflowCode(fromHTTPBody: malformedTypedFields))
    }

    func testEffectiveModel_OfficialBackendHonorsExplicitModel() throws {
        // Ensure the test host is recognized as an "official backend" host.
        ClientConfig.kHardcodedOfficialAPIBase_CN = "https://official.example.com"
        ClientConfig.kHardcodedOfficialAPIBase_Global = "https://official.example.com"

        let cfg = OpenAIProvider.Config(
            baseURL: URL(string: "https://official.example.com/v1")!,
            apiKey: "official-backend-uses-x-client-uuid",
            model: "deepseek-ai/DeepSeek-V3.2"
        )
        XCTAssertEqual(OpenAIProvider.effectiveModel(for: cfg), "deepseek-ai/DeepSeek-V3.2")
    }

    func testEffectiveModel_OfficialBackendUsesPlaceholderWhenModelMissing() throws {
        ClientConfig.kHardcodedOfficialAPIBase_CN = "https://official.example.com"
        ClientConfig.kHardcodedOfficialAPIBase_Global = "https://official.example.com"

        let cfg = OpenAIProvider.Config(
            baseURL: URL(string: "https://official.example.com/v1")!,
            apiKey: "official-backend-uses-x-client-uuid",
            model: OpenAIProvider.officialModelPlaceholder
        )
        XCTAssertEqual(OpenAIProvider.effectiveModel(for: cfg), OpenAIProvider.officialModelPlaceholder)
    }

    func testEffectiveModel_NonOfficialKeepsModel() throws {
        let cfg = OpenAIProvider.Config(
            baseURL: URL(string: "https://api.openai.com/v1")!,
            apiKey: "sk-test",
            model: "gpt-4o-mini"
        )
        XCTAssertEqual(OpenAIProvider.effectiveModel(for: cfg), "gpt-4o-mini")
    }

    func testBuildRequestBody_DisablesDashScopeThinkingWhenRequested() throws {
        let provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: URL(string: "https://dashscope.aliyuncs.com/compatible-mode/v1")!,
            apiKey: "sk-test",
            model: "qwen3.5-35b-a3b"
        ))
        Self.retainedProviders.append(provider)

        let data = try provider.buildRequestBody(
            system: "system",
            userContent: "hello",
            attachments: [],
            stream: true,
            enableThinking: false
        )
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(body["enable_thinking"] as? Bool, false)
        XCTAssertNotNil(body["stream_options"])
    }

    func testBuildRequestBody_DoesNotSendProviderSpecificThinkingFlagToOpenAI() throws {
        let provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: URL(string: "https://api.openai.com/v1")!,
            apiKey: "sk-test",
            model: "gpt-4o-mini"
        ))
        Self.retainedProviders.append(provider)

        let data = try provider.buildRequestBody(
            system: "system",
            userContent: "hello",
            attachments: [],
            stream: true,
            enableThinking: false
        )
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertNil(body["enable_thinking"])
    }

    func testBuildRequestBody_DoesNotSendDashScopeThinkingFlagForNonQwenThinkingModel() throws {
        let provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: URL(string: "https://dashscope.aliyuncs.com/compatible-mode/v1")!,
            apiKey: "sk-test",
            model: "deepseek-ai/DeepSeek-V3.2"
        ))
        Self.retainedProviders.append(provider)

        let data = try provider.buildRequestBody(
            system: "system",
            userContent: "hello",
            attachments: [],
            stream: true,
            enableThinking: false
        )
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertNil(body["enable_thinking"])
    }

    func testBuildRequestBodyInjectsNativeImageIntoTheLastUserMessage() throws {
        let provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: URL(string: "https://api.openai.com/v1")!,
            apiKey: "sk-test",
            model: "vision-model",
            executionCapabilities: ModelExecutionCapabilities(imageInput: true)
        ))
        Self.retainedProviders.append(provider)

        let data = try provider.buildRequestBody(
            system: "system",
            userContent: "analyze this",
            attachments: [
                ModelInput.Attachment(name: "chart.png", mimeType: "image/png", base64Data: "AQID")
            ],
            stream: true,
            enableThinking: true
        )
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let messages = try XCTUnwrap(body["messages"] as? [[String: Any]])
        let user = try XCTUnwrap(messages.last)
        let content = try XCTUnwrap(user["content"] as? [[String: Any]])

        XCTAssertEqual(content.first?["type"] as? String, "text")
        XCTAssertEqual(content.first?["text"] as? String, "analyze this")
        XCTAssertEqual(content.last?["type"] as? String, "image_url")
        let imageURL = try XCTUnwrap(content.last?["image_url"] as? [String: String])
        XCTAssertEqual(imageURL["url"], "data:image/png;base64,AQID")
    }

    func testChatCompletionsURL_NormalizesSupportedConfigurationShapes() throws {
        let cases = [
            (
                "https://example.com",
                "https://example.com/v1/chat/completions"
            ),
            (
                "https://example.com/v1",
                "https://example.com/v1/chat/completions"
            ),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
            ),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
            ),
        ]

        for (configured, expected) in cases {
            let configuredURL = try XCTUnwrap(URL(string: configured))
            XCTAssertEqual(
                try OpenAIProvider.chatCompletionsURL(for: configuredURL).absoluteString,
                expected
            )
        }
    }

    func testCreatedFixupMiddleware_InsertsCreatedWhenMissing() throws {
        let json = """
        {
          "id": "some_id",
          "object": "chat.completion",
          "model": "gpt-4",
          "choices": []
        }
        """
        let data = try XCTUnwrap(json.data(using: .utf8))

        let fixed = OpenAIProvider.CreatedFixupMiddleware.injectCreatedIfMissingJSON(data: data, now: { 123456 })
        let obj = try XCTUnwrap(try JSONSerialization.jsonObject(with: fixed) as? [String: Any])

        XCTAssertEqual(obj["created"] as? Int, 123456)
    }

        func testCreatedFixupMiddleware_InsertsChoicesWhenMissing() throws {
                let json = """
                {
                    "id": "some_id",
                    "object": "chat.completion",
                    "model": "gpt-4",
                    "created": 1
                }
                """
                let data = try XCTUnwrap(json.data(using: .utf8))

                let url = try XCTUnwrap(URL(string: "https://example.com/v1/chat/completions"))
                let request = URLRequest(url: url)

                let (_, fixedData) = OpenAIProvider.CreatedFixupMiddleware.normalizeChatCompletionResponse(
                        response: nil,
                    request: request,
                        data: data,
                        now: { 999 }
                )
                let fixed = try XCTUnwrap(fixedData)
                let obj = try XCTUnwrap(try JSONSerialization.jsonObject(with: fixed) as? [String: Any])
                XCTAssertNotNil(obj["choices"], "choices should be inserted")
                XCTAssertTrue((obj["choices"] as? [Any])?.isEmpty ?? false)
        }

        func testCreatedFixupMiddleware_Converts200ErrorEnvelopeToNon2xx() throws {
                let json = """
                {
                    "error": {
                        "message": "boom",
                        "type": "invalid_request_error"
                    }
                }
                """
                let data = try XCTUnwrap(json.data(using: .utf8))
                let url = try XCTUnwrap(URL(string: "https://example.com/v1/chat/completions"))
                let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))

                let request = URLRequest(url: url)

                let (fixedResponse, _) = OpenAIProvider.CreatedFixupMiddleware.normalizeChatCompletionResponse(
                        response: response,
                    request: request,
                        data: data,
                        now: { 1 }
                )
                let fixedHTTP = try XCTUnwrap(fixedResponse as? HTTPURLResponse)
                XCTAssertNotEqual(fixedHTTP.statusCode, 200)
        }
}

private final class ResponsesFixtureURLProtocol: URLProtocol {
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "fixture.invalid"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let usage = request.url?.path.contains("/partial/") == true
            ? #"{"input_tokens":12,"output_tokens":0,"total_tokens":null,"output_tokens_details":{"reasoning_tokens":0}}"#
            : #"{"input_tokens":12,"output_tokens":4,"total_tokens":16}"#
        var body = """
        data: {"type":"response.output_text.delta","delta":"Checking."}

        data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","call_id":"call_1","name":"read","arguments":"{}"}],"usage":\(usage)}}

        data: [DONE]

        """
        if request.url?.path.contains("/chat-public/") == true {
            var data = request.httpBody ?? Data()
            if request.httpBody == nil, let stream = request.httpBodyStream {
                stream.open()
                defer { stream.close() }
                var buffer = [UInt8](repeating: 0, count: 4096)
                while data.count < 1_048_576 {
                    let read = stream.read(&buffer, maxLength: buffer.count)
                    if read == 0 { break }
                    guard read > 0 else { XCTFail("The actual request body stream must be readable"); break }
                    data.append(contentsOf: buffer.prefix(read))
                }
            }
            let sent = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            XCTAssertNotNil(sent, "The actual provider request must remain JSON")
            XCTAssertNil(sent?["enable_thinking"], "Unknown request controls must remain omitted on the generic endpoint")
            XCTAssertNil(sent?["reasoning_effort"])
            body = """
            data: {"choices":[{"delta":{"reasoning_content":"Public "}}]}

            data: {"choices":[{"delta":{"reasoning":"reasoning"}}]}

            data: {"choices":[{"delta":{"content":"answer"},"finish_reason":"stop"}]}

            data: [DONE]

            """
        }
        if request.url?.path.contains("/reasoning/") == true {
            body = """
            data: {"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"Live "}

            data: {"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"thinking"}

            data: {"type":"response.output_text.delta","delta":"answer"}

            data: {"type":"response.completed","response":{"id":"resp_reason","output":[{"id":"r1","type":"reasoning","summary":[],"content":[{"type":"reasoning_text","text":"Live thinking suffix"}],"encrypted_content":"private-opaque"}]}}

            data: [DONE]

            """
        }
        let response = HTTPURLResponse(url: request.url!, statusCode: 200,
            httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "text/event-stream"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() { }
}
