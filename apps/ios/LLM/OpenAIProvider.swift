// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements OpenAIProvider logic in the iOS LLM integration layer.
// - Interfaces:
//   - OpenAIProvider
// - APIs:
//   - none
//
//
//  OpenAIProvider.swift
//  Nola
//
//  OpenAI-compatible chat completions provider (cloud).
//  Uses direct HTTP requests plus a small SSE parser so third-party providers
//  behave predictably without depending on SDK-specific decoding quirks.
//

import Foundation
import Combine
import CryptoKit

/// Protocol identity is data, not a model-name or endpoint heuristic.
nonisolated public enum ModelWireProtocol: String, Codable, Hashable, Sendable {
    case chatCompletions = "chat_completions"
    case responses
    case anthropic
}

nonisolated public enum ChatOutputTokenField: String, Codable, CaseIterable, Sendable {
    case auto
    case maxTokens = "max_tokens"
    case maxCompletionTokens = "max_completion_tokens"

    func requestKey(baseURL: URL) -> String {
        switch self {
        case .maxTokens: "max_tokens"
        case .maxCompletionTokens: "max_completion_tokens"
        case .auto: baseURL.host?.lowercased() == "api.openai.com" ? "max_completion_tokens" : "max_tokens"
        }
    }
}

/// Missing model execution metadata never grants optional capabilities.
nonisolated public struct ModelExecutionCapabilities: Codable, Hashable, Sendable {
    public var toolCalls: Bool = false
    public var imageInput: Bool = false
    public var reasoning: Bool = false
    public var samplingParameters: Bool = false
    public var samplingRequiresNoReasoning: Bool = false
    public var chatOutputTokenField: ChatOutputTokenField? = nil

    private enum CodingKeys: String, CodingKey {
        case toolCalls, imageInput, reasoning, samplingParameters, samplingRequiresNoReasoning, chatOutputTokenField
    }

    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        toolCalls = try fields.decode(Bool.self, forKey: .toolCalls)
        imageInput = try fields.decode(Bool.self, forKey: .imageInput)
        reasoning = try fields.decode(Bool.self, forKey: .reasoning)
        samplingParameters = try fields.decode(Bool.self, forKey: .samplingParameters)
        samplingRequiresNoReasoning = fields.contains(.samplingRequiresNoReasoning)
            ? try fields.decode(Bool.self, forKey: .samplingRequiresNoReasoning) : false
        chatOutputTokenField = fields.contains(.chatOutputTokenField)
            ? try fields.decode(ChatOutputTokenField.self, forKey: .chatOutputTokenField) : nil
    }

    public init(
        toolCalls: Bool = false,
        imageInput: Bool = false,
        reasoning: Bool = false,
        samplingParameters: Bool = false,
        samplingRequiresNoReasoning: Bool = false,
        chatOutputTokenField: ChatOutputTokenField? = nil
    ) {
        self.toolCalls = toolCalls
        self.imageInput = imageInput
        self.reasoning = reasoning
        self.samplingParameters = samplingParameters
        self.samplingRequiresNoReasoning = samplingRequiresNoReasoning
        self.chatOutputTokenField = chatOutputTokenField
    }
}

public final class OpenAIProvider: LLMClient, AgentPromptCacheIdentityClient, LLMAgentCapabilityProviding {
    public struct Config: Codable, Equatable {
        public var baseURL: URL
        public var apiKey: String
        public var model: String
        public var wireProtocol: ModelWireProtocol
        public var executionCapabilities: ModelExecutionCapabilities
        public var temperature: Double
        public var topP: Double
        public var maxCompletionTokens: Int
        public var presencePenalty: Double
        public var frequencyPenalty: Double
        public var seed: Int?
        public var stopSequences: [String]
        public var promptCacheEnabled: Bool?
        public var promptCacheKey: String?
        public var promptCacheKeyPrefix: String?
        var reasoningSelection: AgentCoreJSONValue? = nil
        var reasoningTier: String = "balanced"
        public var reasoningSummarySupported: Bool?

        private enum CodingKeys: String, CodingKey {
            case baseURL
            case apiKey
            case model
            case wireProtocol
            case executionCapabilities
            case temperature
            case topP
            case maxCompletionTokens
            case presencePenalty
            case frequencyPenalty
            case seed
            case stopSequences
            case promptCacheEnabled
            case promptCacheKey
            case promptCacheKeyPrefix
            case reasoningSelection, reasoningTier
            case reasoningSummarySupported
        }

        public init(
            baseURL: URL,
            apiKey: String,
            model: String,
            wireProtocol: ModelWireProtocol = .chatCompletions,
            executionCapabilities: ModelExecutionCapabilities = .init(),
            temperature: Double = ModelParameters.webAlignedCloudDefaults.temperature,
            topP: Double = ModelParameters.webAlignedCloudDefaults.topP,
            maxCompletionTokens: Int = ModelParameters.webAlignedCloudDefaults.maxOutputTokens,
            presencePenalty: Double = ModelParameters.webAlignedCloudDefaults.presencePenalty,
            frequencyPenalty: Double = ModelParameters.webAlignedCloudDefaults.frequencyPenalty,
            seed: Int? = nil,
            stopSequences: [String] = [],
            promptCacheEnabled: Bool? = nil,
            promptCacheKey: String? = nil,
            promptCacheKeyPrefix: String? = nil,
            reasoningSummarySupported: Bool? = nil
        ) {
            self.baseURL = baseURL
            self.apiKey = apiKey
            self.model = model
            self.wireProtocol = wireProtocol
            self.executionCapabilities = executionCapabilities
            self.temperature = temperature
            self.topP = topP
            self.maxCompletionTokens = maxCompletionTokens
            self.presencePenalty = presencePenalty
            self.frequencyPenalty = frequencyPenalty
            self.seed = seed
            self.stopSequences = stopSequences
            self.promptCacheEnabled = promptCacheEnabled
            self.promptCacheKey = promptCacheKey
            self.promptCacheKeyPrefix = promptCacheKeyPrefix
            self.reasoningSummarySupported = reasoningSummarySupported
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            baseURL = try c.decode(URL.self, forKey: .baseURL)
            apiKey = try c.decode(String.self, forKey: .apiKey)
            model = try c.decode(String.self, forKey: .model)
            wireProtocol = try c.decodeIfPresent(ModelWireProtocol.self, forKey: .wireProtocol) ?? .chatCompletions
            executionCapabilities = try c.decodeIfPresent(ModelExecutionCapabilities.self, forKey: .executionCapabilities) ?? .init()
            temperature = try c.decodeIfPresent(Double.self, forKey: .temperature) ?? ModelParameters.webAlignedCloudDefaults.temperature
            topP = try c.decodeIfPresent(Double.self, forKey: .topP) ?? ModelParameters.webAlignedCloudDefaults.topP
            maxCompletionTokens = try c.decodeIfPresent(Int.self, forKey: .maxCompletionTokens) ?? ModelParameters.webAlignedCloudDefaults.maxOutputTokens
            presencePenalty = try c.decodeIfPresent(Double.self, forKey: .presencePenalty) ?? ModelParameters.webAlignedCloudDefaults.presencePenalty
            frequencyPenalty = try c.decodeIfPresent(Double.self, forKey: .frequencyPenalty) ?? ModelParameters.webAlignedCloudDefaults.frequencyPenalty
            seed = try c.decodeIfPresent(Int.self, forKey: .seed)
            stopSequences = try c.decodeIfPresent([String].self, forKey: .stopSequences) ?? []
            promptCacheEnabled = try c.decodeIfPresent(Bool.self, forKey: .promptCacheEnabled)
            promptCacheKey = try c.decodeIfPresent(String.self, forKey: .promptCacheKey)
            promptCacheKeyPrefix = try c.decodeIfPresent(String.self, forKey: .promptCacheKeyPrefix)
            reasoningSelection = try c.decodeIfPresent(AgentCoreJSONValue.self, forKey: .reasoningSelection)
            reasoningTier = try c.decodeIfPresent(String.self, forKey: .reasoningTier) ?? "balanced"
            reasoningSummarySupported = try c.decodeIfPresent(Bool.self, forKey: .reasoningSummarySupported)
        }
    }

    private let config: Config
    public var auxiliaryModelIdentity: String? { effectiveModel }
    public var auxiliaryConfigurationIdentity: Data {
        // Only a SHA256 of these bytes enters the canonical operation. The
        // credential and endpoint query never enter receipts or diagnostics.
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return (try? encoder.encode(config)) ?? Data()
    }
    public var agentCapabilities: LLMAgentCapabilities {
        guard config.wireProtocol == .chatCompletions || config.wireProtocol == .responses else {
            return .init()
        }
        return .init(visionInput: config.executionCapabilities.imageInput,
            maxOutputTokens: config.maxCompletionTokens, nativeToolCalls: config.executionCapabilities.toolCalls,
            reasoning: config.executionCapabilities.reasoning,
            promptCache: effectivePromptCacheEnabled, streamText: true,
            streamReasoning: true,
            streamToolCalls: config.executionCapabilities.toolCalls)
    }

    private var effectivePromptCacheEnabled: Bool {
        config.promptCacheEnabled ?? (config.baseURL.host?.lowercased() == "api.openai.com")
    }
    private let logger = Logger.shared
    private let streamSession: URLSession

    private static let clientUUIDKey = "com.nola.client_uuid"
    private static let defaultTimeout: TimeInterval = 60
    public static let officialModelPlaceholder = "nola-default"

    public init(config: Config, streamSession: URLSession = .shared) {
        self.config = config
        self.streamSession = streamSession
    }

    // Only immutable configuration and shared service references are retained;
    // there is no actor-bound teardown. Explicit nonisolated destruction avoids
    // the isolated-deinit back-deployment runtime path on older iOS simulators.
    nonisolated deinit {}

    static func effectiveModel(for config: Config) -> String {
        // The configured upstream identity is authoritative. Endpoint-specific
        // aliases would make the admitted model differ from the actual request.
        return config.model
    }

    private var effectiveModel: String {
        Self.effectiveModel(for: config)
    }

    private var formattedClientUUID: String? {
        guard let uuid = KeychainHelper.shared.read(Self.clientUUIDKey) else { return nil }
        if uuid.count == 32 && !uuid.contains("-") {
            let p1 = uuid.prefix(8)
            let p2 = uuid.dropFirst(8).prefix(4)
            let p3 = uuid.dropFirst(12).prefix(4)
            let p4 = uuid.dropFirst(16).prefix(4)
            let p5 = uuid.dropFirst(20)
            return "\(p1)-\(p2)-\(p3)-\(p4)-\(p5)"
        }
        return uuid
    }

    private var shouldSendDashScopeThinkingControl: Bool {
        guard let host = config.baseURL.host?.lowercased() else { return false }
        let model = effectiveModel.lowercased()
        return host.contains("dashscope.aliyuncs.com")
            && (model.contains("qwen3") || model.contains("qwen-3") || model.contains("qwq"))
    }

    public func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        try await generateSummaryReceipt(input: input, timeout: timeout).output
    }

    func generateRoomRouting(inputJSON: String, system: String) async throws -> (AgentCoreJSONValue, AgentCoreJSONValue) {
        var usage: AgentCoreJSONValue = .object(["coverage": .string("unknown"), "provider_raw": .null, "normalized": .null])
        do {
            let content: String
            if config.wireProtocol == .responses {
                let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [],
                    userPrefs: ["message": inputJSON, "custom_system_prompt": system])
                var collected = ""
                for try await event in generateResponsesEventStream(input: input) {
                    if case .textDelta(let delta) = event { collected += delta }
                    if case .usage(let value) = event, let raw = value.providerRawUsage { usage = Self.auxiliaryUsage(raw) }
                }
                content = collected
            } else {
                let requestID = Self.makeRequestId(prefix: "room-routing")
                let response = try await sendNonStreamingRequest(system: system, userContent: inputJSON,
                    timeout: 60, requestId: requestID, enableThinking: false)
                if let raw = response["usage"] as? [String: Any] { usage = Self.auxiliaryUsage(Self.usageJSON(raw)) }
                content = try extractAssistantContent(from: response, requestId: requestID, requireContent: true)
            }
            return (try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(content.utf8)), usage)
        } catch { throw LLMSummaryPhysicalFailure(underlying: error, usage: usage) }
    }

    public func generateSummaryReceipt(input: ModelInput, timeout: TimeInterval?) async throws -> LLMSummaryPhysicalReceipt {
        var usage: AgentCoreJSONValue = .object(["coverage": .string("unknown"), "provider_raw": .null, "normalized": .null])
        let maintenance = input.userPrefs["purpose"] == "context_maintenance"
        var finishReason: String?
        var hasToolCalls = false
        do {
            if config.wireProtocol == .responses {
                var content = ""
                for try await event in generateResponsesEventStream(input: input) {
                    if case .toolCallDelta = event { hasToolCalls = true }
                    if case .completed(let reason) = event { finishReason = reason }
                    if case .textDelta(let delta) = event { content += delta }
                    if case .usage(let value) = event, let raw = value.providerRawUsage { usage = Self.auxiliaryUsage(raw) }
                }
                if maintenance { return LLMSummaryPhysicalReceipt(output: ModelOutput(items: [], explain: content), finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage) }
                if let data = content.data(using: .utf8), let output = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                    return LLMSummaryPhysicalReceipt(output: output, finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage)
                }
                if let substring = LLMResponseValidator.extractJSONSubstring(from: content),
                   let data = substring.data(using: .utf8), let output = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                    return LLMSummaryPhysicalReceipt(output: output, finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage)
                }
                throw NolaError.llm(.invalidResponse(reason: "Responses summary is not a valid ModelOutput"))
            }
            let requestId = Self.makeRequestId(prefix: "summarize")
            let prompt: (system: String, userContent: String, attachments: [ModelInput.Attachment])
            if maintenance {
                guard let instructions = input.userPrefs["custom_system_prompt"], let source = input.userPrefs["message"] else {
                    throw NolaError.invalidInput(reason: "Context maintenance requires Core instructions and source")
                }
                prompt = (instructions, source, [])
            } else { prompt = try buildPrompt(for: input) }
            let responseObject = try await sendNonStreamingRequest(
                system: prompt.system,
                userContent: prompt.userContent,
                timeout: timeout,
                requestId: requestId,
                enableThinking: !maintenance
            )
            if let raw = responseObject["usage"] as? [String: Any] { usage = Self.auxiliaryUsage(Self.usageJSON(raw)) }
            let summaryChoice = (responseObject["choices"] as? [[String: Any]])?.first
            let summaryMessage = summaryChoice?["message"] as? [String: Any]
            hasToolCalls = !(summaryMessage?["tool_calls"] as? [Any] ?? []).isEmpty
            finishReason = ((responseObject["choices"] as? [[String: Any]])?.first?["finish_reason"] as? String)
            let content = try extractAssistantContent(from: responseObject, requestId: requestId, requireContent: true)
            if maintenance { return LLMSummaryPhysicalReceipt(output: ModelOutput(items: [], explain: content), finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage) }

            if let data = content.data(using: .utf8),
               let output = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                return LLMSummaryPhysicalReceipt(output: output, finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage)
            }

            if let jsonSubstring = LLMResponseValidator.extractJSONSubstring(from: content),
               let data = jsonSubstring.data(using: .utf8),
               let output = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                return LLMSummaryPhysicalReceipt(output: output, finishReason: finishReason, hasToolCalls: hasToolCalls, usage: usage)
            }

            let sample = Self.sampleTextForLogging(content)
            logger.log(.warn, "OpenAIProvider", "LLM response content is not decodable as ModelOutput", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
                "model": effectiveModel,
                "contentLen": "\(content.count)",
                "contentSample": sample
            ])))
            throw NolaError.llm(.invalidResponse(reason: "LLM 返回内容无法解码为预期 JSON（示例前缀：\(sample)）"))
        } catch {
            throw LLMSummaryPhysicalFailure(underlying: error, usage: usage)
        }
    }

    private static func usageJSON(_ usage: [String: Any]) -> AgentCoreJSONValue {
        guard let data = try? JSONSerialization.data(withJSONObject: usage),
              let value = try? JSONDecoder().decode(AgentCoreJSONValue.self, from: data) else { return .null }
        return value
    }

    private static func auxiliaryUsage(_ raw: AgentCoreJSONValue) -> AgentCoreJSONValue {
        func counter(_ value: AgentCoreJSONValue?) -> UInt64? {
            switch value {
            case .unsignedInteger(let value)?: return value
            case .signedInteger(let value)? where value >= 0: return UInt64(value)
            case .number(let value)? where value.isFinite && value >= 0 && value.rounded(.down) == value && value <= 9_007_199_254_740_991:
                return UInt64(value)
            default: return nil
            }
        }
        guard case .object(let values) = raw,
              let input = counter(values["prompt_tokens"] ?? values["input_tokens"]),
              let output = counter(values["completion_tokens"] ?? values["output_tokens"]) else {
            return .object(["coverage": .string("unknown"), "provider_raw": raw, "normalized": .null])
        }
        let details = values["prompt_tokens_details"] ?? values["input_tokens_details"]
        var cached: AgentCoreJSONValue = .null
        if case .object(let details)? = details, let count = counter(details["cached_tokens"]), count <= input { cached = .unsignedInteger(count) }
        if cached == .null, let count = counter(values["cache_read_input_tokens"]), count <= input { cached = .unsignedInteger(count) }
        let cacheWrite = counter(values["cache_creation_input_tokens"]).flatMap { $0 <= input ? AgentCoreJSONValue.unsignedInteger($0) : nil } ?? .null
        return .object(["coverage": .string("known"), "provider_raw": raw, "normalized": .object([
            "input_tokens": .unsignedInteger(input), "output_tokens": .unsignedInteger(output), "cached_input_tokens": cached,
            "cache_write_input_tokens": cacheWrite
        ])])
    }

    public func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        if config.wireProtocol == .responses {
            var content = ""
            for try await event in generateResponsesEventStream(input: try responsesChatInput(
                message: message, systemPrompt: systemPrompt ?? PromptTemplates.chatSystem
            )) {
                if case .textDelta(let delta) = event { content += delta }
            }
            return enableThinking ? content : Self.strippingContentThinking(from: content).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let requestId = Self.makeRequestId(prefix: "chat")
        let system: String
        if let systemPrompt {
            system = systemPrompt
        } else {
            system = try PromptTemplates.chatSystem
        }
        let responseObject = try await sendNonStreamingRequest(
            system: system,
            userContent: message,
            timeout: timeout,
            requestId: requestId,
            enableThinking: enableThinking
        )

        let content = try extractAssistantContent(from: responseObject, requestId: requestId, requireContent: false)
        if !enableThinking {
            return Self.strippingContentThinking(from: content).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return content
    }

    public func chatAgentTurn(
        message: String,
        systemPrompt: String,
        enableThinking: Bool,
        stablePrefixSHA256: String
    ) async throws -> String {
        let requestId = Self.makeRequestId(prefix: "agent")
        let responseObject = try await sendNonStreamingRequest(
            system: systemPrompt,
            userContent: message,
            timeout: nil,
            requestId: requestId,
            enableThinking: enableThinking,
            promptCacheKey: derivedPromptCacheKey(stablePrefixSHA256: stablePrefixSHA256)
        )
        let content = try extractAssistantContent(
            from: responseObject,
            requestId: requestId,
            requireContent: false
        )
        return enableThinking
            ? content
            : Self.strippingContentThinking(from: content).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Streams an ordinary chat reply using the same request and validation
    /// path as direct-cloud chat. Model tests use this without a summary JSON
    /// contract or a shared mutable provider.
    public func chatStream(
        message: String,
        systemPrompt: String?,
        enableThinking: Bool
    ) -> AsyncThrowingStream<String, Error> {
        if config.wireProtocol == .responses {
            return AsyncThrowingStream { continuation in
                let task = Task {
                    do {
                        let input = try responsesChatInput(message: message, systemPrompt: systemPrompt ?? PromptTemplates.chatSystem)
                        for try await event in generateResponsesEventStream(input: input) {
                            if case .textDelta(let delta) = event { continuation.yield(delta) }
                        }
                        continuation.finish()
                    } catch { continuation.finish(throwing: error) }
                }
                continuation.onTermination = { @Sendable _ in task.cancel() }
            }
        }
        let requestId = Self.makeRequestId(prefix: "chat-test")
        let systemResult = Result { try systemPrompt ?? PromptTemplates.chatSystem }
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    try await streamRequest(
                        system: systemResult.get(),
                        userContent: message,
                        attachments: [],
                        requestId: requestId,
                        enableThinking: enableThinking,
                        continuation: continuation
                    )
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: mapError(error))
                }
            }
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }

    public func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        if config.wireProtocol == .responses {
            return AsyncThrowingStream { continuation in
                let task = Task {
                    do {
                        for try await event in generateResponsesEventStream(input: input) {
                            if case .textDelta(let delta) = event { continuation.yield(delta) }
                        }
                        continuation.finish()
                    } catch { continuation.finish(throwing: error) }
                }
                continuation.onTermination = { @Sendable _ in task.cancel() }
            }
        }
        let requestId = Self.makeRequestId(prefix: "stream")
        let promptResult = Result { try buildPrompt(for: input) }
        let enableThinking = input.userPrefs["enable_thinking"] != "false"

        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let prompt = try promptResult.get()
                    try await streamRequest(
                        system: prompt.system,
                        userContent: prompt.userContent,
                        attachments: prompt.attachments,
                        requestId: requestId,
                        enableThinking: enableThinking,
                        continuation: continuation
                    )
                    continuation.finish()
                } catch {
                    if let cancellation = error as? CancellationError {
                        continuation.finish(throwing: cancellation)
                        return
                    }
                    logLLMError(stage: "streamRequest", requestId: requestId, error: error, extra: nil)
                    continuation.finish(throwing: mapError(error))
                }
            }

            continuation.onTermination = { @Sendable _ in
                task.cancel()
            }
        }
    }

    func buildPrompt(for input: ModelInput) throws -> (system: String, userContent: String, attachments: [ModelInput.Attachment]) {
        // A Core-prepared payload is already a complete model request envelope. It
        // must not depend on the general chat `message` preference, otherwise a
        // valid Agent turn is silently reinterpreted as a summarization request.
        if input.userPrefs["prepared_core_payload"] == "true" {
            guard let payload = input.userPrefs["prepared_core_payload_json"], !payload.isEmpty else {
                throw NolaError.llm(.invalidInput(reason: "Core prepared an empty model payload"))
            }
            return ("__NOLA_CORE_CONTEXT_V1__", payload, input.attachments)
        }

        let isChat = input.candidates.isEmpty && input.userPrefs["message"] != nil
        if isChat {
            let style = PromptResponseStyle(rawValue: input.userPrefs["response_style"] ?? "") ?? .casual
            let customSystemPrompt = input.userPrefs["custom_system_prompt"]
            let system = try PromptTemplates.chatSystem(responseStyle: style, customSystemPrompt: customSystemPrompt)
            let userContent = input.userPrefs["message"] ?? ""
            return (system, userContent, input.attachments)
        }

        let userJSON = String(data: (try? JSONEncoder().encode(input)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}"
        let userContent = PromptTemplates.summarizationUserPrefix + userJSON
        return (PromptTemplates.summarizationSystem, userContent, input.attachments)
    }

    /// Converts Core's provider-neutral flat tool-call record into the OpenAI
    /// chat-completions wire shape. Core intentionally stores `{id,name,arguments}`
    /// so checkpoints are not coupled to one provider protocol.
    static func normalizeCoreToolCallsForOpenAI(_ value: Any) -> [[String: Any]] {
        guard let calls = value as? [Any] else { return [] }
        return calls.compactMap { raw in
            guard let call = raw as? [String: Any] else { return nil }
            let id = call["id"] as? String ?? UUID().uuidString
            if let rawFunction = call["function"] as? [String: Any] {
                guard let name = rawFunction["name"] as? String, !name.isEmpty else { return nil }
                return [
                    "id": id,
                    "type": "function",
                    "function": [
                        "name": name,
                        "arguments": openAIArgumentsString(rawFunction["arguments"]),
                    ],
                ]
            }
            guard let name = call["name"] as? String, !name.isEmpty else { return nil }
            return [
                "id": id,
                "type": "function",
                "function": [
                    "name": name,
                    "arguments": openAIArgumentsString(call["arguments"]),
                ],
            ]
        }
    }

    private static func openAIArgumentsString(_ value: Any?) -> String {
        if let string = value as? String { return string }
        guard let value, JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]),
              let string = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return string
    }

    private func sendNonStreamingRequest(
        system: String,
        userContent: String,
        timeout: TimeInterval?,
        requestId: String,
        enableThinking: Bool,
        promptCacheKey: String? = nil
    ) async throws -> [String: Any] {
        let request = try makeRequest(
            system: system,
            userContent: userContent,
            stream: false,
            timeout: timeout,
            requestId: requestId,
            enableThinking: enableThinking,
            promptCacheKey: promptCacheKey
        )

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw mapError(error)
        }

        try validateHTTP(response: response, data: data, requestId: requestId)

        guard let object = try? JSONSerialization.jsonObject(with: data, options: []),
              let dict = object as? [String: Any] else {
            let sample = String(data: data.prefix(800), encoding: .utf8) ?? "(binary)"
            logger.log(.warn, "OpenAIProvider", "Non-stream response is not JSON", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
                "model": effectiveModel,
                "sample": SensitiveDataMasker.sanitizeMessage(sample)
            ])))
            throw NolaError.llm(.invalidResponse(reason: "上游返回了无法解析的非 JSON 响应"))
        }

        if let errorEnvelope = Self.providerErrorEnvelope(from: dict) {
            throw makeEnvelopeError(from: errorEnvelope, statusCode: (response as? HTTPURLResponse)?.statusCode, requestId: requestId)
        }

        return dict
    }

    private func streamRequest(
        system: String,
        userContent: String,
        attachments: [ModelInput.Attachment],
        requestId: String,
        enableThinking: Bool,
        continuation: AsyncThrowingStream<String, Error>.Continuation
    ) async throws {
        let request = try makeRequest(
            system: system,
            userContent: userContent,
            attachments: attachments,
            stream: true,
            timeout: nil,
            requestId: requestId,
            enableThinking: enableThinking
        )

        let bytes: URLSession.AsyncBytes
        let response: URLResponse
        do {
            (bytes, response) = try await URLSession.shared.bytes(for: request)
        } catch {
            throw mapError(error)
        }

        if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
            var body = Data()
            for try await byte in bytes {
                body.append(byte)
            }
            try validateHTTP(response: http, data: body, requestId: requestId)
        }

        var state = StreamState()
        var sawExplicitTerminal = false
        for try await line in bytes.lines {
            if Task.isCancelled {
                throw CancellationError()
            }

            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty || trimmed.hasPrefix(":") {
                continue
            }
            guard trimmed.hasPrefix("data:") else { continue }

            let payloadStart = trimmed.index(trimmed.startIndex, offsetBy: 5)
            let payload = trimmed[payloadStart...].trimmingCharacters(in: .whitespaces)
            if payload.isEmpty {
                continue
            }
            if payload == "[DONE]" {
                sawExplicitTerminal = true
                break
            }

            if Self.streamPayloadHasTerminalFinishReason(payload) {
                sawExplicitTerminal = true
            }

            try processStreamEvent(payload: payload, state: &state, requestId: requestId, enableThinking: enableThinking, continuation: continuation)
        }

        guard sawExplicitTerminal else {
            throw URLError(
                .networkConnectionLost,
                userInfo: [NSLocalizedDescriptionKey: "The model stream ended before an explicit completion event."]
            )
        }

        if state.isThinking {
            continuation.yield("</think>\n")
        }
        if !enableThinking {
            let remainder = Self.flushContentThinkingRemainder(state: &state)
            if !remainder.isEmpty {
                continuation.yield(remainder)
            }
        }
    }

    private func processStreamEvent(
        payload: String,
        state: inout StreamState,
        requestId: String,
        enableThinking: Bool,
        continuation: AsyncThrowingStream<String, Error>.Continuation
    ) throws {
        guard let data = payload.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data, options: []),
              let dict = object as? [String: Any] else {
            logger.log(.debug, "OpenAIProvider", "Ignoring non-JSON stream payload", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
                "preview": SensitiveDataMasker.sanitizeMessage(String(payload.prefix(200)))
            ])))
            return
        }

        if let errorEnvelope = Self.providerErrorEnvelope(from: dict) {
            throw makeEnvelopeError(from: errorEnvelope, statusCode: nil, requestId: requestId)
        }

        if let totalTokens = Self.extractUsageTotalTokens(from: dict) {
            logger.log(
                .debug,
                "OpenAIProvider",
                "Stream usage received",
                metadata: sanitizedMetadata(baseMetadata(
                    requestId: requestId,
                    extra: ["totalTokens": String(totalTokens)]
                ))
            )
        }

        guard let choices = dict["choices"] as? [[String: Any]], !choices.isEmpty else {
            return
        }

        for choice in choices {
            if let delta = choice["delta"] as? [String: Any] {
                let reasoning = Self.extractText(from: delta["reasoning_content"]) ?? Self.extractText(from: delta["reasoning"])
                if enableThinking, let reasoning, !reasoning.isEmpty {
                    if !state.isThinking {
                        continuation.yield("<think>")
                        state.isThinking = true
                    }
                    continuation.yield(reasoning)
                }

                let content = Self.extractText(from: delta["content"])
                if let content {
                    state.deltaCount += 1
                    let visibleContent = enableThinking ? content : Self.removingContentThinking(from: content, state: &state)
                    if content.isEmpty {
                        logger.log(.debug, "OpenAIProvider", "Delta content (empty)", metadata: ["deltaIndex": String(state.deltaCount)])
                    } else {
                        logger.log(.debug, "OpenAIProvider", "Delta content", metadata: [
                            "preview": String(visibleContent.prefix(80)),
                            "bytes": String(visibleContent.count),
                            "deltaIndex": String(state.deltaCount)
                        ])
                    }

                    if !visibleContent.isEmpty {
                        if state.isThinking {
                            continuation.yield("</think>\n")
                            state.isThinking = false
                        }
                        if !state.firstContentLogged {
                            state.firstContentLogged = true
                            logger.log(.info, "OpenAIProvider", "First delta content", metadata: [
                                "preview": String(visibleContent.prefix(80)),
                                "bytes": String(visibleContent.count)
                            ])
                        }
                        continuation.yield(visibleContent)
                    }
                }
            } else if let message = choice["message"] as? [String: Any],
                      let content = Self.extractText(from: message["content"]),
                      !content.isEmpty {
                let visibleContent = enableThinking ? content : Self.strippingContentThinking(from: content)
                guard !visibleContent.isEmpty else { continue }
                if state.isThinking {
                    continuation.yield("</think>\n")
                    state.isThinking = false
                }
                continuation.yield(visibleContent)
            }
        }
    }

    private static func streamPayloadHasTerminalFinishReason(_ payload: String) -> Bool {
        guard let data = payload.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data),
              let envelope = object as? [String: Any],
              let choices = envelope["choices"] as? [[String: Any]] else {
            return false
        }
        return choices.contains { choice in
            (choice["finish_reason"] as? String)?.isEmpty == false
        }
    }

    private func makeRequest(
        system: String,
        userContent: String,
        attachments: [ModelInput.Attachment] = [],
        stream: Bool,
        timeout: TimeInterval?,
        requestId: String,
        enableThinking: Bool,
        promptCacheKey: String? = nil
    ) throws -> URLRequest {
        guard config.wireProtocol == .chatCompletions else {
            throw NolaError.llm(.invalidInput(reason: "The selected model requires a different wire-protocol adapter."))
        }
        let trimmedModel = effectiveModel.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmedModel.isEmpty {
            throw NolaError.llm(.notConfigured)
        }
        let requestURL = try Self.chatCompletionsURL(for: config.baseURL)
        let rootURL = requestURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let timeoutInterval = timeout ?? Self.defaultTimeout

        var customHeaders: [String: String] = [:]
        if let uuid = formattedClientUUID {
            customHeaders["X-Client-UUID"] = uuid
        }
        customHeaders["X-Request-ID"] = requestId

        logLLMRequestConfiguration(
            requestId: requestId,
            timeout: timeoutInterval,
            host: rootURL.host ?? "",
            scheme: rootURL.scheme ?? "https",
            port: rootURL.port ?? ((rootURL.scheme ?? "https") == "https" ? 443 : 80),
            basePath: rootURL.path.isEmpty ? "/" : rootURL.path,
            customHeaders: customHeaders
        )

        let body = try buildRequestBody(
            system: system,
            userContent: userContent,
            attachments: attachments,
            stream: stream,
            enableThinking: enableThinking,
            promptCacheKey: promptCacheKey
        )

        var request = URLRequest(url: requestURL)
        request.httpMethod = "POST"
        request.timeoutInterval = timeoutInterval
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(stream ? "text/event-stream" : "application/json", forHTTPHeaderField: "Accept")
        if !config.apiKey.isEmpty {
            request.setValue("Bearer \(config.apiKey)", forHTTPHeaderField: "Authorization")
        }
        for (name, value) in customHeaders {
            request.setValue(value, forHTTPHeaderField: name)
        }
        return request
    }

    static func chatCompletionsURL(for configuredURL: URL) throws -> URL {
        guard var components = URLComponents(url: configuredURL, resolvingAgainstBaseURL: false) else {
            throw NolaError.llm(.invalidInput(reason: "无效的 API Base URL"))
        }
        let trimmedPath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let normalizedPath = trimmedPath.isEmpty ? "" : "/\(trimmedPath)"
        let lowercasePath = normalizedPath.lowercased()
        if lowercasePath.hasSuffix("/chat/completions") {
            components.path = normalizedPath
        } else if lowercasePath.hasSuffix("/v1") {
            components.path = "\(normalizedPath)/chat/completions"
        } else {
            components.path = "\(normalizedPath)/v1/chat/completions"
        }
        guard let url = components.url else {
            throw NolaError.llm(.invalidInput(reason: "无效的 API Base URL"))
        }
        return url
    }

    /// Match the shared generation schema without rewriting admitted values.
    static func validateGenerationConfiguration(_ config: Config) throws {
        guard config.wireProtocol == .chatCompletions || config.wireProtocol == .responses else {
            throw NolaError.llm(.invalidInput(reason: "This provider does not implement the selected wire protocol"))
        }
        guard ModelParameterLimits.outputTokens.contains(config.maxCompletionTokens),
              config.temperature.isFinite, (0...2).contains(config.temperature),
              config.topP.isFinite, (0...1).contains(config.topP),
              config.presencePenalty.isFinite, (-2...2).contains(config.presencePenalty),
              config.frequencyPenalty.isFinite, (-2...2).contains(config.frequencyPenalty),
              config.stopSequences.count <= 16,
              config.stopSequences.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 2048 }) else {
            throw NolaError.llm(.invalidInput(reason: "Invalid model generation parameters"))
        }
        let execution = config.executionCapabilities
        guard !execution.samplingRequiresNoReasoning || config.reasoningSelection?.objectFields?["value"] == .string("none") else {
            throw NolaError.llm(.invalidInput(reason: "Conditional sampling requires explicit reasoning effort none"))
        }
        if !execution.samplingParameters && (
            config.temperature != 0.7 || config.topP != 0.9 ||
            config.presencePenalty != 0 || config.frequencyPenalty != 0 ||
            config.seed != nil || !config.stopSequences.isEmpty
        ) {
            throw NolaError.llm(.invalidInput(reason: "This model does not declare sampling parameter support"))
        }
        if config.wireProtocol == .responses && (
            config.presencePenalty != 0 || config.frequencyPenalty != 0 ||
            config.seed != nil || !config.stopSequences.isEmpty
        ) {
            throw NolaError.llm(.invalidInput(reason: "Unsupported Responses model parameters"))
        }
    }

    func buildRequestBody(
        system: String,
        userContent: String,
        attachments: [ModelInput.Attachment],
        stream: Bool,
        enableThinking: Bool,
        promptCacheKey: String? = nil
    ) throws -> Data {
        guard config.wireProtocol == .chatCompletions else {
            throw NolaError.llm(.invalidInput(reason: "The selected model requires a different wire-protocol adapter."))
        }
        try Self.validateGenerationConfiguration(config)
        var requestMessages: [[String: Any]]
        var coreTools: [Any] = []
        var parallelToolCalls = false
        var coreToolChoice: Any?
        if system == "__NOLA_CORE_CONTEXT_V1__",
           let data = userContent.data(using: .utf8),
           let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
           let typedSystemPrompt = envelope["system_prompt"] as? String,
           let rawMessages = envelope["messages"] as? [[String: Any]] {
            requestMessages = [["role": "system", "content": typedSystemPrompt]]
            requestMessages.append(contentsOf: rawMessages.compactMap { message in
                guard let role = message["role"] as? String else { return nil }
                guard role != "system" else { return nil }
                var value: [String: Any] = [
                    "role": role,
                    "content": message["content"] ?? ""
                ]
                if let name = message["name"] { value["name"] = name }
                if let toolCallID = message["tool_call_id"] { value["tool_call_id"] = toolCallID }
                if role == "assistant", let toolCalls = message["tool_calls"] {
                    let normalized = Self.normalizeCoreToolCallsForOpenAI(toolCalls)
                    if !normalized.isEmpty { value["tool_calls"] = normalized }
                }
                return value
            })
            coreTools = envelope["tools"] as? [Any] ?? []
            parallelToolCalls = envelope["parallel_tool_calls"] as? Bool ?? false
            coreToolChoice = envelope["tool_choice"]
        } else {
            requestMessages = [
                ["role": "system", "content": system],
                ["role": "user", "content": userContent]
            ]
        }

        if !attachments.isEmpty,
           let lastUserIndex = requestMessages.lastIndex(where: { ($0["role"] as? String) == "user" }) {
            let text = requestMessages[lastUserIndex]["content"] as? String ?? ""
            var parts: [[String: Any]] = [["type": "text", "text": text]]
            for attachment in attachments {
                parts.append([
                    "type": "image_url",
                    "image_url": [
                        "url": "data:\(attachment.mimeType);base64,\(attachment.base64Data)"
                    ]
                ])
            }
            requestMessages[lastUserIndex]["content"] = parts
        }

        guard !requestMessages.isEmpty else {
            throw NolaError.llm(.invalidInput(reason: "Core prepared an empty model payload"))
        }

        let execution = config.executionCapabilities
        if !execution.toolCalls && (!coreTools.isEmpty || requestMessages.contains(where: {
            ($0["role"] as? String) == "tool" || $0["tool_calls"] != nil
        })) {
            throw NolaError.llm(.invalidInput(reason: "This model does not declare native tool-call support"))
        }
        if !execution.imageInput && !attachments.isEmpty {
            throw NolaError.llm(.invalidInput(reason: "This model does not declare image-input support"))
        }
        var body: [String: Any] = [
            "model": effectiveModel,
            "messages": requestMessages,
            "stream": stream
        ]
        body[(execution.chatOutputTokenField ?? .auto).requestKey(baseURL: config.baseURL)] = config.maxCompletionTokens
        if let selection = config.reasoningSelection, selection != .null {
            guard selection.objectFields?["kind"] == .string("effort"),
                  let effort = selection.objectFields?["value"]?.stringScalar else {
                throw NolaError.llm(.invalidInput(reason: "This wire adapter requires a Core-admitted effort selection"))
            }
            body["reasoning_effort"] = effort
        }
        if execution.samplingParameters {
            body["temperature"] = config.temperature
            body["top_p"] = config.topP
        }
        if !coreTools.isEmpty {
            body["tools"] = coreTools
            body["parallel_tool_calls"] = parallelToolCalls
            if let coreToolChoice {
                body["tool_choice"] = coreToolChoice
            }
        }
        if let promptCacheKey {
            body["prompt_cache_key"] = promptCacheKey
        }
        if config.presencePenalty != 0 {
            body["presence_penalty"] = config.presencePenalty
        }
        if config.frequencyPenalty != 0 {
            body["frequency_penalty"] = config.frequencyPenalty
        }
        if let seed = config.seed {
            body["seed"] = seed
        }
        if !config.stopSequences.isEmpty {
            body["stop"] = config.stopSequences
        }
        if stream {
            body["stream_options"] = ["include_usage": true]
        }
        if !enableThinking && shouldSendDashScopeThinkingControl {
            body["enable_thinking"] = false
        }
        return try JSONSerialization.data(withJSONObject: body, options: [])
    }

    private func derivedPromptCacheKey(stablePrefixSHA256: String) -> String? {
        let digest = stablePrefixSHA256.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard digest.count == 64,
              digest.unicodeScalars.allSatisfy({ CharacterSet(charactersIn: "0123456789abcdef").contains($0) }) else {
            return nil
        }
        let host = config.baseURL.host?.lowercased() ?? ""
        guard effectivePromptCacheEnabled else { return nil }
        if let explicit = config.promptCacheKey?.trimmingCharacters(in: .whitespacesAndNewlines),
           !explicit.isEmpty {
            return explicit
        }
        let configuredPrefix = config.promptCacheKeyPrefix?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let prefix = configuredPrefix.isEmpty ? "nola" : configuredPrefix
        let namespace = "\(prefix)|\(host)|\(effectiveModel)|\(digest)"
        return SHA256.hash(data: Data(namespace.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func extractAssistantContent(
        from responseObject: [String: Any],
        requestId: String,
        requireContent: Bool
    ) throws -> String {
        guard let choices = responseObject["choices"] as? [[String: Any]], let firstChoice = choices.first else {
            logger.log(.warn, "OpenAIProvider", "Chat completion returned empty choices", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
                "model": effectiveModel,
                "reason": "empty_choices"
            ])))
            throw NolaError.llm(.invalidResponse(reason: "Chat completion 返回空 choices"))
        }

        if let message = firstChoice["message"] as? [String: Any] {
            let content = Self.extractText(from: message["content"]) ?? ""
            if !content.isEmpty || !requireContent {
                return content
            }

            if let toolCalls = message["tool_calls"] as? [Any], !toolCalls.isEmpty {
                logger.log(.warn, "OpenAIProvider", "Chat completion returned tool_calls without content", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
                    "model": effectiveModel,
                    "reason": "tool_calls"
                ])))
                throw NolaError.llm(.invalidResponse(reason: "Chat completion 返回 tool_calls，但当前流程不支持"))
            }
        }

        logger.log(.warn, "OpenAIProvider", "Chat completion missing message.content", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
            "model": effectiveModel,
            "reason": "missing_content"
        ])))

        if requireContent {
            throw NolaError.llm(.invalidResponse(reason: "Chat completion 的首个 choice 缺少 message.content"))
        }
        return ""
    }

    private func validateHTTP(response: URLResponse, data: Data, requestId: String) throws {
        guard let http = response as? HTTPURLResponse else {
            throw NolaError.llm(.network(URLError(.badServerResponse)))
        }

        if (200...299).contains(http.statusCode) {
            return
        }

        let error = makeHTTPError(statusCode: http.statusCode, data: data, requestId: requestId, url: http.url)
        throw error
    }

    private func makeHTTPError(statusCode: Int, data: Data, requestId: String, url: URL?) -> Error {
        let host = url?.host ?? config.baseURL.host ?? ""
        let path = url?.path ?? config.baseURL.path

        var message = ""
        if let object = try? JSONSerialization.jsonObject(with: data, options: []),
           let dict = object as? [String: Any] {
            message = Self.extractErrorMessage(from: dict) ?? ""
            let sample = Self.sampleJSONForLogging(dict)
            logger.log(.warn, "OpenAIProvider", "Upstream returned non-2xx response", metadata: sanitizedMetadata([
                "requestId": requestId,
                "host": host,
                "path": path,
                "status": String(statusCode),
                "sample": sample
            ]))
        } else if let text = String(data: data.prefix(800), encoding: .utf8) {
            message = text
            logger.log(.warn, "OpenAIProvider", "Upstream returned non-JSON error response", metadata: sanitizedMetadata([
                "requestId": requestId,
                "host": host,
                "path": path,
                "status": String(statusCode),
                "sample": SensitiveDataMasker.sanitizeMessage(text)
            ]))
        }

        if let code = Self.canonicalContextOverflowCode(fromHTTPBody: data) {
            return NolaError.llm(.providerFailure(code: code, message: message, retryable: false))
        }

        switch statusCode {
        case 400:
            return NolaError.llm(.invalidInput(reason: message.isEmpty ? "请求格式错误" : message))
        case 401:
            return NolaError.llm(.invalidInput(reason: message.isEmpty ? L10n.errorNetworkUnauthorized : message))
        case 403:
            return NolaError.llm(.invalidInput(reason: message.isEmpty ? L10n.errorNetworkUnauthorized : message))
        case 408:
            return NolaError.llm(.timeout)
        case 429:
            return NolaError.llm(.rateLimited)
        default:
            return NolaError.llm(.network(URLError(.badServerResponse)))
        }
    }

    private func makeEnvelopeError(from errorEnvelope: Any, statusCode: Int?, requestId: String) -> Error {
        var message = "上游返回错误"
        if let dict = errorEnvelope as? [String: Any] {
            message = Self.extractErrorMessage(from: dict) ?? message
        } else if let array = errorEnvelope as? [Any], let first = array.first as? [String: Any] {
            message = Self.extractErrorMessage(from: first) ?? message
        } else if let string = errorEnvelope as? String, !string.isEmpty {
            message = string
        }

        logger.log(.warn, "OpenAIProvider", "Upstream returned error envelope", metadata: sanitizedMetadata(baseMetadata(requestId: requestId, extra: [
            "status": statusCode.map(String.init) ?? "(nil)",
            "message": message
        ])))

        if let code = Self.canonicalContextOverflowCode(fromErrorEnvelope: errorEnvelope) {
            return NolaError.llm(.providerFailure(code: code, message: message, retryable: false))
        }

        switch statusCode {
        case 401:
            return NolaError.llm(.invalidInput(reason: message.isEmpty ? L10n.errorNetworkUnauthorized : message))
        case 403:
            return NolaError.llm(.invalidInput(reason: message))
        case 429:
            return NolaError.llm(.rateLimited)
        default:
            return NolaError.llm(.invalidResponse(reason: message))
        }
    }

    nonisolated static func canonicalContextOverflowCode(fromHTTPBody data: Data) -> String? {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let envelope = providerErrorEnvelope(from: root) else {
            return nil
        }
        return canonicalContextOverflowCode(fromErrorEnvelope: envelope)
    }

    private nonisolated static func providerErrorEnvelope(from root: [String: Any]) -> Any? {
        let envelopes = [root["error"], root["errors"]].compactMap { $0 }
        if envelopes.count == 1 { return envelopes[0] }
        return envelopes.isEmpty ? nil : envelopes
    }

    nonisolated static func canonicalContextOverflowCode(fromErrorEnvelope envelope: Any) -> String? {
        var objects: [[String: Any]] = []
        func collectObjects(from value: Any) {
            if let dictionary = value as? [String: Any] {
                objects.append(dictionary)
            } else if let array = value as? [Any] {
                array.forEach(collectObjects)
            }
        }
        collectObjects(from: envelope)

        for object in objects {
            for key in ["code", "type", "error_code"] {
                guard let providerCode = object[key] as? String else { continue }
                switch providerCode.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
                case "context_length_exceeded", "context_window_exceeded", "context_length_error",
                     "prompt_too_long", "input_too_long", "request_too_large":
                    return "context_length_exceeded"
                default:
                    continue
                }
            }
        }
        return nil
    }

    private func mapError(_ error: Error) -> Error {
        if let nola = error as? NolaError {
            return nola
        }
        if error is CancellationError {
            return error
        }
        if let urlError = error as? URLError {
            switch urlError.code {
            case .timedOut:
                return NolaError.llm(.timeout)
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost:
                return NolaError.llm(.network(urlError))
            default:
                return NolaError.llm(.network(urlError))
            }
        }
        return NolaError.llm(.network(error))
    }

    private static func extractUsageTotalTokens(from dict: [String: Any]) -> Int? {
        if let usage = dict["usage"] as? [String: Any] {
            if let total = usage["total_tokens"] as? Int {
                return total
            }
            if let total = usage["totalTokens"] as? Int {
                return total
            }
            if let total = usage["total_tokens"] as? NSNumber {
                return total.intValue
            }
        }
        return nil
    }

    private static func extractText(from any: Any?) -> String? {
        switch any {
        case let text as String:
            return text
        case let parts as [Any]:
            let joined = parts.compactMap { item -> String? in
                if let dict = item as? [String: Any] {
                    if let text = dict["text"] as? String {
                        return text
                    }
                    if let nested = dict["content"] as? String {
                        return nested
                    }
                    if let inner = dict["text"] as? [String: Any], let value = inner["value"] as? String {
                        return value
                    }
                }
                return nil
            }.joined()
            return joined
        default:
            return nil
        }
    }

    private static func removingContentThinking(from content: String, state: inout StreamState) -> String {
        var output = ""
        state.contentThinkingBuffer += content

        while !state.contentThinkingBuffer.isEmpty {
            if state.isSuppressingContentThinking {
                guard let end = state.contentThinkingBuffer.range(of: "</think>", options: [.caseInsensitive]) else {
                    state.contentThinkingBuffer = String(state.contentThinkingBuffer.suffix(max(0, "</think>".count - 1)))
                    return output
                }
                state.contentThinkingBuffer = String(state.contentThinkingBuffer[end.upperBound...])
                state.isSuppressingContentThinking = false
                continue
            }

            guard let start = state.contentThinkingBuffer.range(of: "<think>", options: [.caseInsensitive]) else {
                let keepCount = Self.openThinkingTagPrefixSuffixLength(in: state.contentThinkingBuffer)
                let flushCount = state.contentThinkingBuffer.count - keepCount
                if flushCount > 0 {
                    let flushEnd = state.contentThinkingBuffer.index(state.contentThinkingBuffer.startIndex, offsetBy: flushCount)
                    output += String(state.contentThinkingBuffer[..<flushEnd])
                    state.contentThinkingBuffer = String(state.contentThinkingBuffer[flushEnd...])
                }
                break
            }

            output += String(state.contentThinkingBuffer[..<start.lowerBound])
            state.contentThinkingBuffer = String(state.contentThinkingBuffer[start.upperBound...])
            state.isSuppressingContentThinking = true
        }

        return output
    }

    private static func strippingContentThinking(from content: String) -> String {
        var state = StreamState()
        return removingContentThinking(from: content, state: &state) + flushContentThinkingRemainder(state: &state)
    }

    private static func flushContentThinkingRemainder(state: inout StreamState) -> String {
        guard !state.isSuppressingContentThinking else {
            state.contentThinkingBuffer = ""
            return ""
        }
        let remainder = state.contentThinkingBuffer
        state.contentThinkingBuffer = ""
        return remainder
    }

    private static func openThinkingTagPrefixSuffixLength(in text: String) -> Int {
        let lowercased = text.lowercased()
        let openTag = "<think>"
        let maxLength = min(openTag.count - 1, lowercased.count)
        guard maxLength > 0 else { return 0 }
        for length in stride(from: maxLength, through: 1, by: -1) {
            let suffix = String(lowercased.suffix(length))
            if openTag.hasPrefix(suffix) {
                return length
            }
        }
        return 0
    }

    private static func extractErrorMessage(from dict: [String: Any]) -> String? {
        if let message = dict["message"] as? String, !message.isEmpty {
            return message
        }
        if let error = dict["error"] as? [String: Any],
           let nested = extractErrorMessage(from: error) {
            return nested
        }
        if let errors = dict["errors"] as? [[String: Any]],
           let first = errors.first,
           let nested = extractErrorMessage(from: first) {
            return nested
        }
        return nil
    }

    private func logLLMRequestConfiguration(
        requestId: String?,
        timeout: TimeInterval?,
        host: String,
        scheme: String,
        port: Int,
        basePath: String,
        customHeaders: [String: String]
    ) {
        let rid = (requestId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)

        var stage = "unknown"
        if rid.hasPrefix("llm-") {
            let remainder = String(rid.dropFirst(4))
            if let first = remainder.split(separator: "-").first, !first.isEmpty {
                stage = String(first)
            }
        }

        let isDefaultPort = (scheme == "https" && port == 443) || (scheme == "http" && port == 80)
        let portPart = isDefaultPort ? "" : ":\(port)"
        let finalBaseURL = "\(scheme)://\(host)\(portPart)\(basePath)"

        let hasClientUUID = customHeaders["X-Client-UUID"] != nil
        let meta: [String: String] = sanitizedMetadata([
            "requestId": rid,
            "stage": stage,
            "provider": "openai",
            "finalBaseURL": finalBaseURL,
            "host": host,
            "scheme": scheme,
            "port": "\(port)",
            "basePath": basePath,
            "configuredModel": config.model,
            "effectiveModel": effectiveModel,
            "hasXClientUUID": hasClientUUID ? "true" : "false",
            "timeoutSec": "\(Int(timeout ?? Self.defaultTimeout))",
        ])

        logger.log(.info, "OpenAIProvider", "LLM request config summary", metadata: [
            "finalBaseURL": finalBaseURL,
            "hasXClientUUID": hasClientUUID ? "true" : "false"
        ])
        logger.log(.debug, "OpenAIProvider", "LLM request config", metadata: meta)
    }

    private static func makeRequestId(prefix: String) -> String {
        "llm-\(prefix)-\(UUID().uuidString)"
    }

    private func baseMetadata(requestId: String, extra: [String: String]?) -> [String: String] {
        let host = config.baseURL.host ?? ""
        let scheme = config.baseURL.scheme ?? ""
        let path = config.baseURL.path

        var meta: [String: String] = [
            "requestId": requestId,
            "provider": "openai",
            "host": host,
            "scheme": scheme,
            "basePath": path,
        ]
        if let extra {
            for (k, v) in extra {
                meta[k] = v
            }
        }
        return meta
    }

    private func sanitizedMetadata(_ metadata: [String: String]) -> [String: String] {
        SensitiveDataMasker.sanitizeMetadata(metadata)
    }

    private static func sampleTextForLogging(_ text: String, limit: Int = 800) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let prefix = String(trimmed.prefix(limit))
        return SensitiveDataMasker.sanitizeMessage(prefix)
    }

    private static func sampleJSONForLogging(_ dict: [String: Any], limit: Int = 800) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: dict, options: []),
              let s = String(data: data, encoding: .utf8) else {
            return "(unserializable)"
        }
        let prefix = String(s.prefix(limit))
        return SensitiveDataMasker.sanitizeMessage(prefix)
    }

    private func logLLMError(stage: String, requestId: String, error: Error, extra: [String: String]?) {
        var meta = baseMetadata(requestId: requestId, extra: extra)
        meta["stage"] = stage
        meta["error"] = error.localizedDescription
        meta["model"] = effectiveModel
        logger.log(.error, "OpenAIProvider", "LLM request failed", metadata: sanitizedMetadata(meta))
    }

    private struct StreamState {
        var isThinking = false
        var isSuppressingContentThinking = false
        var contentThinkingBuffer = ""
        var firstContentLogged = false
        var deltaCount = 0
    }

    struct CreatedFixupMiddleware {
        static func normalizeChatCompletionResponse(
            response: URLResponse?,
            request: URLRequest,
            data: Data,
            now: () -> Int = { Int(Date().timeIntervalSince1970) }
        ) -> (response: URLResponse?, data: Data?) {
            guard let firstNonWhitespaceByte = data.first(where: { byte in
                byte != 32 && byte != 9 && byte != 10 && byte != 13
            }), firstNonWhitespaceByte == UInt8(ascii: "{") else {
                return (response, data)
            }

            guard let object = try? JSONSerialization.jsonObject(with: data, options: []),
                  var dict = object as? [String: Any] else {
                return (response, data)
            }

            let requestId = request.value(forHTTPHeaderField: "X-Request-ID")
            let statusCode = (response as? HTTPURLResponse)?.statusCode
            let host = request.url?.host ?? ""
            let path = request.url?.path ?? ""

            if let statusCode, statusCode >= 400 {
                let sample = Self.sampleJSONForLogging(dict)
                Logger.shared.log(.warn, "OpenAIProvider", "Upstream returned non-2xx response", metadata: SensitiveDataMasker.sanitizeMetadata([
                    "requestId": requestId ?? "(nil)",
                    "host": host,
                    "path": path,
                    "status": String(statusCode),
                    "sample": sample
                ]))
                return (response, data)
            }

            if let choices = dict["choices"] as? [Any],
               choices.isEmpty,
               dict["error"] == nil,
               dict["errors"] == nil {
                let sample = Self.sampleJSONForLogging(dict)
                Logger.shared.log(.warn, "OpenAIProvider", "Upstream returned empty choices", metadata: SensitiveDataMasker.sanitizeMetadata([
                    "requestId": requestId ?? "(nil)",
                    "host": host,
                    "path": path,
                    "status": statusCode.map(String.init) ?? "(nil)",
                    "sample": sample
                ]))
            }

            if dict["error"] != nil || dict["errors"] != nil {
                if let http = response as? HTTPURLResponse {
                    let fixedStatus = http.statusCode == 200 ? 500 : http.statusCode
                    let fixed = HTTPURLResponse(
                        url: http.url ?? requestURLFallback(from: http, original: request.url),
                        statusCode: fixedStatus,
                        httpVersion: nil,
                        headerFields: http.allHeaderFields as? [String: String]
                    )
                    let sample = Self.sampleJSONForLogging(dict)
                    Logger.shared.log(.warn, "OpenAIProvider", "Upstream returned error envelope", metadata: SensitiveDataMasker.sanitizeMetadata([
                        "requestId": requestId ?? "(nil)",
                        "host": host,
                        "path": path,
                        "status": statusCode.map(String.init) ?? "(nil)",
                        "sample": sample
                    ]))
                    return (fixed ?? response, data)
                }
                return (response, data)
            }

            var mutated = false
            var injectedCreated = false
            var injectedChoices = false

            if dict["created"] == nil || dict["created"] is NSNull {
                dict["created"] = now()
                mutated = true
                injectedCreated = true
            }

            if dict["choices"] == nil || dict["choices"] is NSNull {
                dict["choices"] = []
                mutated = true
                injectedChoices = true
            }

            if mutated {
                let sample = Self.sampleJSONForLogging(dict)
                Logger.shared.log(.warn, "OpenAIProvider", "Normalized upstream response for SDK decoding", metadata: SensitiveDataMasker.sanitizeMetadata([
                    "requestId": requestId ?? "(nil)",
                    "host": host,
                    "path": path,
                    "status": statusCode.map(String.init) ?? "(nil)",
                    "injectedCreated": injectedCreated ? "true" : "false",
                    "injectedChoices": injectedChoices ? "true" : "false",
                    "sample": sample
                ]))
            }

            if mutated, let fixed = try? JSONSerialization.data(withJSONObject: dict, options: []) {
                return (response, fixed)
            }

            return (response, data)
        }

        private static func requestURLFallback(from response: HTTPURLResponse, original: URL?) -> URL {
            response.url ?? original ?? URL(fileURLWithPath: "/")
        }

        private static func sampleJSONForLogging(_ dict: [String: Any], limit: Int = 800) -> String {
            guard let data = try? JSONSerialization.data(withJSONObject: dict, options: []),
                  let s = String(data: data, encoding: .utf8) else {
                return "(unserializable)"
            }
            let prefix = String(s.prefix(limit))
            return SensitiveDataMasker.sanitizeMessage(prefix)
        }

        static func injectCreatedIfMissingJSON(data: Data, now: () -> Int = { Int(Date().timeIntervalSince1970) }) -> Data {
            guard let firstNonWhitespaceByte = data.first(where: { byte in
                byte != 32 && byte != 9 && byte != 10 && byte != 13
            }), firstNonWhitespaceByte == UInt8(ascii: "{") else {
                return data
            }

            guard let object = try? JSONSerialization.jsonObject(with: data, options: []),
                  var dict = object as? [String: Any] else {
                return data
            }

            if dict["created"] == nil || dict["created"] is NSNull {
                dict["created"] = now()
                if let fixed = try? JSONSerialization.data(withJSONObject: dict, options: []) {
                    return fixed
                }
            }

            return data
        }
    }
}

private struct OpenAINativeToolStreamState {
    var callId: String
    var capabilityId = ""
    var pendingArguments = ""
    var didEmitArguments = false
}

extension OpenAIProvider: LLMControlledEventStreamingClient {
    public var supportsNativeEventStream: Bool { true }

    public func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        generateEventStream(input: input, producer: nil)
    }

    public func generateControlledEventStream(input: ModelInput) -> LLMControlledEventStream {
        let producer = LLMStreamProducerControl()
        return LLMControlledEventStream(events: generateEventStream(input: input, producer: producer), producer: producer)
    }

    private func generateEventStream(input: ModelInput, producer: LLMStreamProducerControl?) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        if config.wireProtocol == .responses {
            return generateResponsesEventStream(input: input, producer: producer)
        }
        let requestId = Self.makeRequestId(prefix: "agent-stream")
        let promptResult = Result { try buildPrompt(for: input) }
        let enableThinking = input.userPrefs["enable_thinking"] != "false"
        let promptCacheKey = input.userPrefs["stable_prompt_digest"].flatMap {
            derivedPromptCacheKey(stablePrefixSHA256: $0)
        }

        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let prompt = try promptResult.get()
                    let request = try makeRequest(
                        system: prompt.system,
                        userContent: prompt.userContent,
                        attachments: prompt.attachments,
                        stream: true,
                        timeout: nil,
                        requestId: requestId,
                        enableThinking: enableThinking,
                        promptCacheKey: promptCacheKey
                    )
                    let (bytes, response) = try await streamSession.bytes(for: request)
                    if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
                        var body = Data()
                        for try await byte in bytes { body.append(byte) }
                        try validateHTTP(response: http, data: body, requestId: requestId)
                    }

                    var toolStates: [Int: OpenAINativeToolStreamState] = [:]
                    var finishReason = "stop"
                    var sawExplicitTerminal = false
                    for try await line in bytes.lines {
                        if Task.isCancelled { throw CancellationError() }
                        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard trimmed.hasPrefix("data:") else { continue }
                        let payload = trimmed.dropFirst(5).trimmingCharacters(in: .whitespaces)
                        guard !payload.isEmpty, payload != "[DONE]" else {
                            if payload == "[DONE]" {
                                sawExplicitTerminal = true
                                break
                            }
                            continue
                        }
                        guard let data = payload.data(using: .utf8),
                              let object = try? JSONSerialization.jsonObject(with: data),
                              let envelope = object as? [String: Any] else { continue }
                        if let errorEnvelope = Self.providerErrorEnvelope(from: envelope) {
                            throw makeEnvelopeError(from: errorEnvelope, statusCode: nil, requestId: requestId)
                        }
                        if let usage = envelope["usage"] as? [String: Any] {
                            let promptTokens = (usage["prompt_tokens"] as? NSNumber)?.intValue
                            let promptTokenDetails = usage["prompt_tokens_details"] as? [String: Any]
                            let cachedInputTokens = (promptTokenDetails?["cached_tokens"] as? NSNumber)?.intValue
                                ?? (usage["cache_read_input_tokens"] as? NSNumber)?.intValue
                            let cacheWriteInputTokens = (usage["cache_creation_input_tokens"] as? NSNumber)?.intValue
                            let completionTokens = (usage["completion_tokens"] as? NSNumber)?.intValue
                            let totalTokens = (usage["total_tokens"] as? NSNumber)?.intValue
                            continuation.yield(.usage(.init(
                                promptTokens: promptTokens,
                                cachedInputTokens: cachedInputTokens,
                                cacheWriteInputTokens: cacheWriteInputTokens,
                                completionTokens: completionTokens,
                                totalTokens: totalTokens,
                                reasoningTokens: ((usage["completion_tokens_details"] as? [String: Any])?["reasoning_tokens"] as? NSNumber)?.intValue,
                                providerRawUsage: Self.usageJSON(usage)
                            )))
                        }
                        guard let choices = envelope["choices"] as? [[String: Any]] else { continue }
                        for choice in choices {
                            if let reason = choice["finish_reason"] as? String, !reason.isEmpty {
                                finishReason = reason
                                sawExplicitTerminal = true
                            }
                            guard let delta = choice["delta"] as? [String: Any] else { continue }
                            // Receiving public provider output is independent
                            // of support for requesting controlled reasoning.
                            if let reasoning = Self.extractText(from: delta["reasoning_content"])
                                ?? Self.extractText(from: delta["reasoning"]),
                               !reasoning.isEmpty {
                                continuation.yield(.reasoningDelta(reasoning))
                            }
                            if let content = Self.extractText(from: delta["content"]), !content.isEmpty {
                                continuation.yield(.textDelta(content))
                            }
                            guard let calls = delta["tool_calls"] as? [[String: Any]] else { continue }
                            for call in calls {
                                let index = (call["index"] as? NSNumber)?.intValue ?? 0
                                let upstreamId = (call["id"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
                                var state = toolStates[index] ?? OpenAINativeToolStreamState(
                                    callId: upstreamId?.isEmpty == false ? upstreamId! : "\(requestId)-tool-\(index)"
                                )
                                if let function = call["function"] as? [String: Any] {
                                    if let name = function["name"] as? String, !name.isEmpty {
                                        if name.hasPrefix(state.capabilityId) {
                                            state.capabilityId = name
                                        } else if !state.capabilityId.hasSuffix(name) {
                                            state.capabilityId += name
                                        }
                                    }
                                    if let arguments = function["arguments"] as? String, !arguments.isEmpty {
                                        state.pendingArguments += arguments
                                    }
                                }
                                if !state.capabilityId.isEmpty, !state.pendingArguments.isEmpty {
                                    continuation.yield(.toolCallDelta(.init(
                                        index: index,
                                        callId: state.callId,
                                        capabilityId: state.capabilityId,
                                        argumentsDelta: state.pendingArguments
                                    )))
                                    state.pendingArguments = ""
                                    state.didEmitArguments = true
                                }
                                toolStates[index] = state
                            }
                        }
                    }
                    guard sawExplicitTerminal else {
                        throw URLError(
                            .networkConnectionLost,
                            userInfo: [NSLocalizedDescriptionKey: "The model stream ended before an explicit completion event."]
                        )
                    }
                    for (index, state) in toolStates.sorted(by: { $0.key < $1.key }) {
                        guard !state.capabilityId.isEmpty else {
                            throw NolaError.llm(.invalidResponse(reason: "A streamed tool call had no capability name."))
                        }
                        if !state.pendingArguments.isEmpty || !state.didEmitArguments {
                            continuation.yield(.toolCallDelta(.init(
                                index: index,
                                callId: state.callId,
                                capabilityId: state.capabilityId,
                                argumentsDelta: state.pendingArguments.isEmpty ? "{}" : state.pendingArguments
                            )))
                        }
                    }
                    continuation.yield(.completed(finishReason: finishReason))
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: mapError(error))
                }
            }
            producer?.install(task)
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }
}

extension OpenAIProvider {
    func responsesChatInput(message: String, systemPrompt: String) throws -> ModelInput {
        let envelope: [String: Any] = [
            "system_prompt": systemPrompt,
            "messages": [["role": "user", "content": message]],
            "tools": [],
            "parallel_tool_calls": false,
        ]
        let data = try JSONSerialization.data(withJSONObject: envelope)
        return ModelInput(
            dateRange: .init(start: "", end: ""),
            candidates: [],
            userPrefs: [
                "prepared_core_payload": "true",
                "prepared_core_payload_json": String(decoding: data, as: UTF8.self),
            ]
        )
    }

    var responsesRouteIdentity: String {
        let route = "\(config.baseURL.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/")))\n\(effectiveModel)\n\(config.apiKey)"
        return SHA256.hash(data: Data(route.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    func responsesURL() throws -> URL {
        guard var components = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false) else {
            throw NolaError.llm(.invalidInput(reason: "Invalid Responses endpoint"))
        }
        let path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let lower = path.lowercased()
        if lower == "chat/completions" || lower.hasSuffix("/chat/completions") {
            let prefix = lower == "chat/completions" ? "" : String(path.dropLast("/chat/completions".count))
            components.path = prefix.isEmpty ? "/responses" : "/\(prefix)/responses"
        } else if lower == "responses" || lower.hasSuffix("/responses") {
            components.path = "/\(path)"
        } else if lower == "v1" || lower.hasSuffix("/v1") {
            components.path = "/\(path)/responses"
        } else {
            components.path = path.isEmpty ? "/v1/responses" : "/\(path)/v1/responses"
        }
        guard let url = components.url else {
            throw NolaError.llm(.invalidInput(reason: "Invalid Responses endpoint"))
        }
        return url
    }

    func responsesRequest(input: ModelInput) throws -> URLRequest {
        try Self.validateGenerationConfiguration(config)
        let prompt = try buildPrompt(for: input)
        var messages: [[String: Any]]
        var tools: [[String: Any]] = []
        var parallelToolCalls = false
        var toolChoice: Any?
        if prompt.system == "__NOLA_CORE_CONTEXT_V1__" {
            guard let data = prompt.userContent.data(using: .utf8),
                  let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let system = envelope["system_prompt"] as? String,
                  let prepared = envelope["messages"] as? [[String: Any]] else {
                throw NolaError.llm(.invalidInput(reason: "Core prepared an invalid model payload"))
            }
            messages = [["role": "system", "content": system]] + prepared
            tools = envelope["tools"] as? [[String: Any]] ?? []
            parallelToolCalls = envelope["parallel_tool_calls"] as? Bool ?? false
            toolChoice = envelope["tool_choice"]
        } else {
            messages = [["role": "system", "content": prompt.system], ["role": "user", "content": prompt.userContent]]
        }
        if !prompt.attachments.isEmpty, let lastUser = messages.lastIndex(where: { ($0["role"] as? String) == "user" }) {
            var blocks: [[String: Any]] = []
            if let text = messages[lastUser]["content"] as? String, !text.isEmpty {
                blocks.append(["type": "text", "text": text])
            }
            blocks += prompt.attachments.map { attachment in
                ["type": "image_url", "image_url": ["url": "data:\(attachment.mimeType);base64,\(attachment.base64Data)"]]
            }
            messages[lastUser]["content"] = blocks
        }
        let capabilities = config.executionCapabilities
        guard capabilities.imageInput || prompt.attachments.isEmpty,
              capabilities.toolCalls || (tools.isEmpty && !messages.contains(where: {
                  ($0["role"] as? String) == "tool" || $0["tool_calls"] != nil
              })) else {
            throw NolaError.llm(.invalidInput(reason: "Unsupported Responses model parameters or capabilities"))
        }
        var replayItems: [[String: Any]] = []
        if let encoded = input.providerContinuationJSON {
            guard let data = encoded.data(using: .utf8),
                  let continuation = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  continuation["provider_namespace"] as? String == "openai.responses",
                  continuation["model_id"] as? String == effectiveModel,
                  let state = continuation["state"] as? [String: Any],
                  state["route_identity"] as? String == responsesRouteIdentity,
                  let items = state["response_items"] as? [[String: Any]] else {
                throw NolaError.llm(.invalidInput(reason: "Responses continuation belongs to another route"))
            }
            replayItems = items
        }
        let replayIndex = replayItems.isEmpty ? nil : messages.lastIndex(where: { ($0["role"] as? String) == "assistant" })
        guard replayItems.isEmpty || replayIndex != nil else {
            throw NolaError.llm(.invalidInput(reason: "Responses continuation has no assistant message"))
        }
        let providerCallIDs = Set(replayItems.compactMap { item in
            item["type"] as? String == "function_call" ? item["call_id"] as? String : nil
        })
        var converted: [[String: Any]] = []
        for (index, message) in messages.enumerated() {
            if index == replayIndex {
                converted += replayItems.map { item in
                    var replay = item
                    if replay["type"] as? String == "reasoning",
                       let content = replay["content"] as? [[String: Any]],
                       content.contains(where: { $0["type"] as? String == "reasoning_text" && !($0["text"] as? String ?? "").isEmpty }) {
                        replay.removeValue(forKey: "encrypted_content")
                        replay.removeValue(forKey: "summary")
                    }
                    return replay
                }
                continue
            }
            guard let role = message["role"] as? String else {
                throw NolaError.llm(.invalidInput(reason: "Model message has no role"))
            }
            if role == "tool" {
                guard let scoped = message["tool_call_id"] as? String else {
                    throw NolaError.llm(.invalidInput(reason: "Tool result has no call ID"))
                }
                let providerID = providerCallIDs.first(where: { scoped == $0 || scoped.hasSuffix("::\($0)") }) ?? scoped
                converted.append(["type": "function_call_output", "call_id": providerID, "output": message["content"] ?? ""])
                continue
            }
            guard ["system", "developer", "user", "assistant"].contains(role) else {
                throw NolaError.llm(.invalidInput(reason: "Unsupported Responses message role"))
            }
            var content: Any = message["content"] ?? ""
            if let blocks = content as? [[String: Any]] {
                content = try blocks.map { block -> [String: Any] in
                    switch block["type"] as? String {
                    case "text": return ["type": role == "assistant" ? "output_text" : "input_text", "text": block["text"] ?? ""]
                    case "image_url":
                        guard let image = block["image_url"] as? [String: Any], let url = image["url"] else {
                            throw NolaError.llm(.invalidInput(reason: "Image URL missing"))
                        }
                        return ["type": "input_image", "image_url": url]
                    default: throw NolaError.llm(.invalidInput(reason: "Unsupported Responses content block"))
                    }
                }
            }
            converted.append(["role": role, "content": content])
            if let calls = message["tool_calls"] as? [[String: Any]] {
                for call in calls {
                    let function = call["function"] as? [String: Any] ?? call
                    guard let id = (call["id"] ?? call["call_id"]) as? String,
                          let name = function["name"] as? String else {
                        throw NolaError.llm(.invalidInput(reason: "Responses tool call lacks identity"))
                    }
                    let rawArguments = function["arguments"] ?? [:]
                    let arguments = rawArguments as? String ?? Self.openAIArgumentsString(rawArguments)
                    converted.append(["type": "function_call", "call_id": id, "name": name, "arguments": arguments])
                }
            }
        }
        var body: [String: Any] = [
            "model": effectiveModel, "input": converted, "stream": true, "store": false,
            "include": ["reasoning.encrypted_content"], "max_output_tokens": config.maxCompletionTokens,
        ]
        var reasoning: [String: Any] = [:]
        if let selection = config.reasoningSelection, selection != .null {
            guard selection.objectFields?["kind"] == .string("effort"),
                  let effort = selection.objectFields?["value"]?.stringScalar else {
                throw NolaError.llm(.invalidInput(reason: "This wire adapter requires a Core-admitted effort selection"))
            }
            reasoning["effort"] = effort
        }
        if capabilities.reasoning && input.userPrefs["enable_thinking"] != "false"
            && config.reasoningSummarySupported != false && reasoning["effort"] as? String != "none" {
            reasoning["summary"] = "auto"
        }
        if !reasoning.isEmpty { body["reasoning"] = reasoning }
        if capabilities.samplingParameters {
            body["temperature"] = config.temperature
            body["top_p"] = config.topP
        }
        if !tools.isEmpty {
            body["tools"] = try tools.map { tool -> [String: Any] in
                var function = tool["function"] as? [String: Any] ?? tool
                guard function["name"] is String else {
                    throw NolaError.llm(.invalidInput(reason: "Responses tool has no name"))
                }
                function["type"] = "function"
                if function["strict"] == nil { function["strict"] = false }
                return function
            }
            body["parallel_tool_calls"] = parallelToolCalls
            if let choice = toolChoice as? [String: Any],
               choice["type"] as? String == "function",
               let function = choice["function"] as? [String: Any],
               let name = function["name"] as? String {
                body["tool_choice"] = ["type": "function", "name": name]
            } else if let toolChoice {
                body["tool_choice"] = toolChoice
            }
        }
        if let digest = input.userPrefs["stable_prompt_digest"],
           let key = derivedPromptCacheKey(stablePrefixSHA256: digest) {
            body["prompt_cache_key"] = key
        }
        let data = try JSONSerialization.data(withJSONObject: body)
        var request = URLRequest(url: try responsesURL())
        request.httpMethod = "POST"
        request.httpBody = data
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        if !config.apiKey.isEmpty {
            request.setValue("Bearer \(config.apiKey)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    func generateResponsesEventStream(input: ModelInput, producer: LLMStreamProducerControl? = nil) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let request = try responsesRequest(input: input)
                    let (bytes, response) = try await streamSession.bytes(for: request)
                    if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
                        throw NolaError.llm(.invalidResponse(reason: "Responses provider returned HTTP \(http.statusCode)"))
                    }
                    var completed = false
                    var streamedText = false
                    var reasoning = ResponsesReasoningProgress()
                    for try await line in bytes.lines {
                        if Task.isCancelled { throw CancellationError() }
                        guard line.hasPrefix("data:") else { continue }
                        let payload = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
                        if payload == "[DONE]" { break }
                        guard let data = payload.data(using: .utf8),
                              let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                              let type = event["type"] as? String else { continue }
                        switch type {
                        case "response.output_text.delta", "response.refusal.delta":
                            if let delta = event["delta"] as? String, !delta.isEmpty {
                                streamedText = true
                                continuation.yield(.textDelta(delta))
                            }
                        case "response.reasoning_summary_text.delta", "response.reasoning_text.delta":
                            if let delta = try reasoning.acceptDelta(event) {
                                continuation.yield(.reasoningDelta(delta))
                            }
                        case "response.failed", "error":
                            throw NolaError.llm(.invalidResponse(reason: "Responses provider reported failure"))
                        case "response.completed", "response.incomplete":
                            guard !completed, let result = event["response"] as? [String: Any] else {
                                throw NolaError.llm(.invalidResponse(reason: "Invalid Responses terminal event"))
                            }
                            let output = result["output"] as? [[String: Any]] ?? []
                            for delta in try reasoning.finish(output) {
                                continuation.yield(.reasoningDelta(delta))
                            }
                            if !streamedText {
                                for item in output where item["type"] as? String == "message" {
                                    for part in item["content"] as? [[String: Any]] ?? [] {
                                        if let text = part["text"] as? String, !text.isEmpty { continuation.yield(.textDelta(text)) }
                                    }
                                }
                            }
                            for (index, item) in output.enumerated() where item["type"] as? String == "function_call" {
                                guard let id = item["call_id"] as? String, let name = item["name"] as? String else {
                                    throw NolaError.llm(.invalidResponse(reason: "Responses function call lacks identity"))
                                }
                                continuation.yield(.toolCallDelta(.init(
                                    index: index, callId: id, capabilityId: name,
                                    argumentsDelta: (item["arguments"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "{}"
                                )))
                            }
                            if let usage = result["usage"] as? [String: Any] {
                                let prompt = (usage["input_tokens"] as? NSNumber)?.intValue
                                let generated = (usage["output_tokens"] as? NSNumber)?.intValue
                                let details = usage["input_tokens_details"] as? [String: Any]
                                continuation.yield(.usage(.init(
                                    promptTokens: prompt,
                                    cachedInputTokens: (details?["cached_tokens"] as? NSNumber)?.intValue,
                                    completionTokens: generated,
                                    totalTokens: (usage["total_tokens"] as? NSNumber)?.intValue,
                                    reasoningTokens: ((usage["output_tokens_details"] as? [String: Any])?["reasoning_tokens"] as? NSNumber)?.intValue,
                                    providerRawUsage: Self.usageJSON(usage)
                                )))
                            }
                            var continuationState: [String: Any] = [
                                "provider_namespace": "openai.responses",
                                "model_id": effectiveModel,
                                "state": [
                                    "response_items": output,
                                    "route_identity": responsesRouteIdentity,
                                ],
                                "integrity_kind": "encrypted",
                                "compaction_policy": "clear_on_compaction",
                            ]
                            if let responseID = result["id"] as? String {
                                continuationState["response_id"] = responseID
                            }
                            let continuationData = try JSONSerialization.data(withJSONObject: continuationState)
                            continuation.yield(.continuation(String(decoding: continuationData, as: UTF8.self)))
                            let finish = type == "response.incomplete"
                                ? ((result["incomplete_details"] as? [String: Any])?["reason"] as? String ?? "incomplete")
                                : "stop"
                            continuation.yield(.completed(finishReason: finish))
                            completed = true
                        default: break
                        }
                    }
                    guard completed else {
                        throw NolaError.llm(.invalidResponse(reason: "Responses stream ended before completion"))
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            producer?.install(task)
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }
}

// Transport progress only: public reasoning may be delivered twice, as deltas
// and as a final snapshot. Encrypted continuation is never display content.
struct ResponsesReasoningProgress {
    private struct Progress {
        let channel: String
        var text: String
    }
    private var items: [String: Progress] = [:]
    private var aliases: [String: String] = [:]

    mutating func acceptDelta(_ event: [String: Any]) throws -> String? {
        let channel = event["type"] as? String == "response.reasoning_text.delta" ? "plain" : "summary"
        let idKey = (event["item_id"] as? String).map { "id:" + $0 }
        let indexKey = (event["output_index"] as? Int).map { "index:\($0)" }
        if let idOwner = idKey.flatMap({ aliases[$0] }),
           let indexOwner = indexKey.flatMap({ aliases[$0] }), idOwner != indexOwner {
            throw NolaError.llm(.invalidResponse(reason: "Responses reasoning item identity contradicts its streamed output"))
        }
        let key = idKey.flatMap { aliases[$0] } ?? indexKey.flatMap { aliases[$0] }
            ?? idKey ?? indexKey ?? "unscoped"
        if let idKey { aliases[idKey] = key }
        if let indexKey { aliases[indexKey] = key }
        var progress = items[key] ?? Progress(channel: channel, text: "")
        guard progress.channel == channel, let delta = event["delta"] as? String, !delta.isEmpty else { return nil }
        progress.text += delta
        items[key] = progress
        return delta
    }

    func finish(_ output: [[String: Any]]) throws -> [String] {
        var deltas: [String] = []
        let singleReasoningItem = output.filter { $0["type"] as? String == "reasoning" }.count == 1
        if items["unscoped"] != nil && !singleReasoningItem {
            throw NolaError.llm(.invalidResponse(reason: "Responses reasoning output has ambiguous item identity"))
        }
        for (index, item) in output.enumerated() where item["type"] as? String == "reasoning" {
            let idKey = (item["id"] as? String).map { "id:" + $0 }
            let indexKey = "index:\(index)"
            let key = idKey.flatMap { aliases[$0] } ?? aliases[indexKey] ?? idKey ?? indexKey
            let progress = items[key] ?? (singleReasoningItem ? items["unscoped"] : nil)
            func visible(_ channel: String) -> String {
                let field = channel == "summary" ? "summary" : "content"
                let type = channel == "summary" ? "summary_text" : "reasoning_text"
                return (item[field] as? [[String: Any]] ?? [])
                    .filter { $0["type"] as? String == type }
                    .compactMap { $0["text"] as? String }.joined()
            }
            let channel = progress?.channel ?? (visible("summary").isEmpty ? "plain" : "summary")
            let snapshot = visible(channel)
            let prior = progress?.text ?? ""
            guard !snapshot.isEmpty else { continue }
            guard snapshot.hasPrefix(prior) else {
                throw NolaError.llm(.invalidResponse(reason: "Responses reasoning snapshot contradicts its streamed output"))
            }
            let suffix = String(snapshot.dropFirst(prior.count))
            if !suffix.isEmpty { deltas.append(suffix) }
        }
        return deltas
    }
}
