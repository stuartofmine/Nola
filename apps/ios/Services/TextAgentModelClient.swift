// SPDX-License-Identifier: Apache-2.0

import Foundation

nonisolated struct AgentNativeToolCallAccumulator {
    struct StreamDelta: Equatable {
        let callId: String
        let capabilityId: String
        let argumentsDelta: String
    }

    private struct Buffer {
        var callId: String
        var capabilityId: String
        var arguments: String
        var emittedArgumentCount: Int
    }

    private var order: [Int] = []
    private var buffers: [Int: Buffer] = [:]
    private let fallbackCallIdPrefix: String

    init(fallbackCallIdPrefix: String = "tool-call") {
        self.fallbackCallIdPrefix = fallbackCallIdPrefix
    }

    var hasCalls: Bool { !order.isEmpty }

    mutating func append(
        index: Int,
        callId: String,
        capabilityId: String,
        argumentsDelta: String
    ) -> StreamDelta? {
        if buffers[index] == nil {
            order.append(index)
            buffers[index] = Buffer(
                callId: callId.isEmpty ? "\(fallbackCallIdPrefix)-\(index)" : callId,
                capabilityId: capabilityId,
                arguments: "",
                emittedArgumentCount: 0
            )
        }
        guard var buffer = buffers[index] else { return nil }
        if !capabilityId.isEmpty {
            buffer.capabilityId = capabilityId
        }
        buffer.arguments += argumentsDelta
        guard !buffer.capabilityId.isEmpty else {
            buffers[index] = buffer
            return nil
        }
        let pendingArguments = String(buffer.arguments.dropFirst(buffer.emittedArgumentCount))
        buffer.emittedArgumentCount = buffer.arguments.count
        buffers[index] = buffer
        return StreamDelta(
            callId: buffer.callId,
            capabilityId: buffer.capabilityId,
            argumentsDelta: pendingArguments
        )
    }

    // Kept for focused accumulator tests and non-streaming adapters. Native
    // provider streams must use their protocol index so an initially-empty
    // call id cannot split one logical call into two buffers.
    mutating func append(
        callId: String,
        capabilityId: String,
        argumentsDelta: String
    ) {
        let existingIndex = order.first { buffers[$0]?.callId == callId }
        let index = existingIndex ?? ((order.max() ?? -1) + 1)
        _ = append(
            index: index,
            callId: callId,
            capabilityId: capabilityId,
            argumentsDelta: argumentsDelta
        )
    }

    func completedCalls() throws -> [AgentCoreToolCall] {
        try order.map { index in
            guard let buffer = buffers[index], !buffer.capabilityId.isEmpty else {
                throw AgentCoreRuntimeError.missingResult
            }
            let arguments: AgentCoreJSONValue
            if buffer.arguments.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                arguments = .object([:])
            } else {
                arguments = try JSONDecoder().decode(
                    AgentCoreJSONValue.self,
                    from: Data(buffer.arguments.utf8)
                )
            }
            return AgentCoreToolCall(
                id: buffer.callId,
                name: buffer.capabilityId,
                arguments: arguments
            )
        }
    }
}

public final class TextAgentModelClient: AgentModelCancellationClient, AgentIndependentModelClientFactory, @unchecked Sendable {
    private let client: LLMClient
    private let enableThinking: Bool
    private let admittedCapabilities: LLMAgentCapabilities
    private let frozenCapabilityProfile: AgentCoreJSONValue?
    public nonisolated let coreModelCapabilityProfile: AgentCoreJSONValue?
    private var controlledRequest: (session: String, turn: String, iteration: Int, producer: LLMStreamProducerControl)?

    public func makeIndependentAgentModelClient() -> any AgentModelClient {
        TextAgentModelClient(client: client, enableThinking: enableThinking, frozen: frozenCapabilityProfile.map { (admittedCapabilities, $0) })
    }

    public func cancelAgentTurn(request: AgentModelRequest) async throws {
        guard let active = controlledRequest,
              active.session == request.sessionId, active.turn == request.turnId,
              active.iteration == request.iteration else {
            throw DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed
        }
        await active.producer.cancelAndWait()
        if controlledRequest?.producer === active.producer { controlledRequest = nil }
    }

    public convenience init(client: LLMClient, enableThinking: Bool = true) {
        self.init(client: client, enableThinking: enableThinking, frozen: nil)
    }
    public convenience init(client: LLMClient, enableThinking: Bool = true, frozenModelCapability: AgentCoreJSONValue) throws {
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let facts = try decoder.decode(AppleFrozenModelCapabilityFacts.self, from: JSONEncoder().encode(frozenModelCapability))
        guard !facts.profileId.isEmpty, facts.revision > 0, facts.directToolCapabilityIds.isEmpty,
              facts.contextWindowTokens.map({ $0 > 0 }) ?? true,
              facts.maxOutputTokens.map({ $0 > 0 }) ?? true else { throw IOSApplicationRuntimeError.invalidResponse }
        let capabilities = LLMAgentCapabilities(visionInput:facts.visionInput,maxOutputTokens:facts.maxOutputTokens,
            nativeToolCalls:facts.nativeToolCalls,parallelToolCalls:facts.parallelToolCalls,
            reasoning:facts.reasoning,reasoningContinuity:facts.reasoningContinuity,promptCache:facts.promptCache,
            streamText:facts.streamText,streamReasoning:facts.streamReasoning,streamToolCalls:facts.streamToolCalls,
            requestContinuation:facts.requestContinuation,maxRequestContinuations:facts.maxRequestContinuations)
        self.init(client:client,enableThinking:enableThinking,frozen:(capabilities,frozenModelCapability))
    }
    private init(client: LLMClient, enableThinking: Bool, frozen: (LLMAgentCapabilities, AgentCoreJSONValue)?) {
        frozenCapabilityProfile = frozen?.1
        // Bind execution and its capability declaration to the same provider.
        // Retaining the mutable selector would allow later settings changes to
        // switch providers underneath an already admitted Agent turn.
        let executionClient = (client as? ModelService)?.currentProvider() ?? client
        self.client = executionClient
        self.enableThinking = enableThinking
        let supportsNativeEvents = (executionClient as? LLMEventStreamingClient)?.supportsNativeEventStream == true
        let declared = (executionClient as? LLMAgentCapabilityProviding)?.agentCapabilities
        let capabilities = LLMAgentCapabilities(
            visionInput: declared?.visionInput ?? false,
            maxOutputTokens: declared?.maxOutputTokens,
            nativeToolCalls: supportsNativeEvents && (declared?.nativeToolCalls ?? false),
            parallelToolCalls: declared?.parallelToolCalls ?? false,
            reasoning: enableThinking && (declared?.reasoning ?? false),
            reasoningContinuity: declared?.reasoningContinuity ?? false,
            promptCache: (declared?.promptCache ?? false) && (supportsNativeEvents || executionClient is AgentPromptCacheIdentityClient),
            streamText: supportsNativeEvents && (declared?.streamText ?? true),
            streamReasoning: supportsNativeEvents && (declared?.streamReasoning ?? false),
            streamToolCalls: supportsNativeEvents && (declared?.streamToolCalls ?? false),
            requestContinuation: declared?.requestContinuation ?? false,
            maxRequestContinuations: declared?.maxRequestContinuations ?? 0)
        self.admittedCapabilities = frozen?.0 ?? capabilities
        if let frozen { self.coreModelCapabilityProfile = frozen.1; return }
        // Core requires an explicit Host capability declaration. Model names
        // never substitute for this adapter's actual transport capabilities.
        let bits = [capabilities.nativeToolCalls, capabilities.parallelToolCalls, capabilities.reasoning,
            capabilities.reasoningContinuity, capabilities.promptCache, capabilities.streamText,
            capabilities.streamReasoning, capabilities.streamToolCalls, capabilities.requestContinuation,
            capabilities.visionInput].map { $0 ? "1" : "0" }.joined()
        let outputIdentity = capabilities.maxOutputTokens.map(String.init) ?? "unknown"
        self.coreModelCapabilityProfile = .object([
            "profile_id": .string("nola.ios.execution.v2:\(bits):\(outputIdentity):\(capabilities.maxRequestContinuations)"),
            "revision": .number(2),
            "instruction_profile": .string("generic"),
            "native_tool_calls": .bool(capabilities.nativeToolCalls),
            "parallel_tool_calls": .bool(capabilities.parallelToolCalls),
            "reasoning": .bool(capabilities.reasoning),
            "reasoning_continuity": .bool(capabilities.reasoningContinuity),
            "prompt_cache": .bool(capabilities.promptCache),
            "vision_input": .bool(capabilities.visionInput),
            "context_window_tokens": .null,
            "max_output_tokens": capabilities.maxOutputTokens.map { .signedInteger(Int64($0)) } ?? .null,
            "request_continuation": .bool(capabilities.requestContinuation),
            "max_request_continuations": .number(Double(capabilities.maxRequestContinuations)),
            "stream_text": .bool(capabilities.streamText),
            "stream_reasoning": .bool(capabilities.streamReasoning),
            "stream_tool_calls": .bool(capabilities.streamToolCalls),
        ])
    }

    public func generateAgentTurn(
        request: AgentModelRequest
    ) -> AsyncThrowingStream<AgentModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                try Task.checkCancellation()
                guard let prompt = AgentModelInterop.localPrompt(for: request) else {
                    throw AgentCoreRuntimeError.missingResult
                }
                let invocation = request.invocationPlan
                try validateInvocation(invocation)
                let hasNativeTools = invocation.nativeToolCalls && !request.tools.isEmpty
                if let requiredToolName = request.requiredToolName,
                   (!hasNativeTools || !prompt.allowedToolNames.contains(requiredToolName)) {
                    throw NolaError.llm(.invalidInput(reason: "The required tool is not available in this model request."))
                }
                let canStreamNativeTools = !hasNativeTools || invocation.streamToolCalls
                let hasStreamingChannel = invocation.streamText
                    || invocation.streamReasoning
                    || invocation.streamToolCalls
                    || request.requiredToolName != nil
                if let eventClient = client as? LLMEventStreamingClient,
                   eventClient.supportsNativeEventStream,
                   canStreamNativeTools,
                   hasStreamingChannel {
                    let encoder = JSONEncoder()
                    let messages = try JSONSerialization.jsonObject(with: encoder.encode(request.messages))
                    let nativeTools = invocation.nativeToolCalls ? request.tools : []
                    let tools = try JSONSerialization.jsonObject(with: encoder.encode(nativeTools))
                    var envelope: [String: Any] = [
                        "system_prompt": prompt.systemPrompt,
                        "messages": messages,
                        "tools": tools,
                        "parallel_tool_calls": invocation.parallelToolCalls,
                    ]
                    if let requiredToolName = request.requiredToolName {
                        envelope["tool_choice"] = [
                            "type": "function",
                            "function": ["name": requiredToolName],
                        ]
                    }
                    let payload = try JSONSerialization.data(withJSONObject: envelope)
                    guard let payloadJSON = String(data: payload, encoding: .utf8) else {
                        throw AgentCoreRuntimeError.missingResult
                    }
                    let formatter = ISO8601DateFormatter()
                    let now = Date()
                    var preferences = [
                        "prepared_core_payload": "true",
                        "prepared_core_payload_json": payloadJSON,
                        "enable_thinking": enableThinking && invocation.reasoning ? "true" : "false",
                    ]
                    if invocation.promptCache, let digest = request.stablePrefixSHA256 {
                        preferences["stable_prompt_digest"] = digest
                    }
                    let input = ModelInput(
                        dateRange: .init(
                            start: formatter.string(from: now.addingTimeInterval(-86_400)),
                            end: formatter.string(from: now)
                        ),
                        candidates: [],
                        userPrefs: preferences,
                        providerContinuationJSON: try request.providerContinuation.map {
                            String(decoding: try encoder.encode($0), as: UTF8.self)
                        }
                    )
                    var content = ""
                    var reasoning = ""
                    var finishReason = "stop"
                    var sawExplicitCompletion = false
                    var usage = AgentCoreTokenUsage()
                    var providerContinuation: AgentCoreJSONValue?
                    var nativeToolCalls = AgentNativeToolCallAccumulator(
                        fallbackCallIdPrefix: "\(request.turnId)-\(request.iteration)-tool-call"
                    )
                    func applyEvent(_ event: LLMStreamEvent, includeTextChannels: Bool) throws -> Bool {
                        switch event {
                        case .textDelta(let delta):
                            guard includeTextChannels else { return false }
                            content += delta
                            if invocation.streamText { continuation.yield(.textDelta(delta)) }
                            return false
                        case .reasoningDelta(let delta):
                            guard includeTextChannels else { return false }
                            reasoning += delta
                            if invocation.streamReasoning { continuation.yield(.reasoningDelta(delta)) }
                            return false
                        case .toolCallDelta(let delta):
                            guard hasNativeTools else {
                                throw NolaError.llm(.invalidResponse(reason: "The provider emitted a native tool call outside the admitted invocation."))
                            }
                            guard let normalized = nativeToolCalls.append(
                                index: delta.index,
                                callId: delta.callId,
                                capabilityId: delta.capabilityId,
                                argumentsDelta: delta.argumentsDelta
                            ) else {
                                return false
                            }
                            if invocation.streamToolCalls {
                                continuation.yield(.toolCallDelta(
                                    partId: "tool:\(normalized.callId)",
                                    callId: normalized.callId,
                                    capabilityId: normalized.capabilityId,
                                    argumentsDelta: normalized.argumentsDelta
                                ))
                            }
                            return false
                        case .usage(let value):
                            usage = .init(
                                promptTokens: value.promptTokens,
                                cachedInputTokens: value.cachedInputTokens,
                                cacheWriteInputTokens: value.cacheWriteInputTokens,
                                completionTokens: value.completionTokens,
                                totalTokens: value.totalTokens,
                                reasoningTokens: value.reasoningTokens
                            )
                            return false
                        case .continuation(let value):
                            providerContinuation = try JSONDecoder().decode(
                                AgentCoreJSONValue.self,
                                from: Data(value.utf8)
                            )
                            return false
                        case .completed(let reason):
                            finishReason = reason
                            sawExplicitCompletion = true
                            return true
                        }
                    }
                    let events: AsyncThrowingStream<LLMStreamEvent, Error>
                    if let controlled = eventClient as? LLMControlledEventStreamingClient {
                        let operation = controlled.generateControlledEventStream(input: input)
                        controlledRequest = (request.sessionId, request.turnId, request.iteration, operation.producer)
                        events = operation.events
                    } else {
                        controlledRequest = nil
                        events = eventClient.generateEventStream(input: input)
                    }
                    for try await event in events {
                        try Task.checkCancellation()
                        if try applyEvent(event, includeTextChannels: true) {
                            break
                        }
                    }
                    guard sawExplicitCompletion else {
                        throw URLError(
                            .networkConnectionLost,
                            userInfo: [NSLocalizedDescriptionKey: "The model event stream ended before completion."]
                        )
                    }
                    if nativeToolCalls.hasCalls {
                        let completedCalls = try nativeToolCalls.completedCalls()
                        if let requiredToolName = request.requiredToolName,
                           !completedCalls.contains(where: { $0.name == requiredToolName }) {
                            throw NolaError.llm(.invalidResponse(reason: "The model did not call the required tool."))
                        }
                        continuation.yield(.completed(.init(
                            content: content,
                            reasoning: reasoning,
                            toolCalls: completedCalls,
                            finishReason: finishReason,
                            usage: usage,
                            providerContinuation: providerContinuation
                        )))
                    } else {
                        guard let parsed = AgentModelInterop.parse(
                            content: content,
                            allowedToolNames: prompt.allowedToolNames
                        ) else { throw AgentCoreRuntimeError.missingResult }
                        continuation.yield(.completed(.init(
                            content: parsed.content,
                            reasoning: parsed.reasoning.isEmpty ? reasoning : parsed.reasoning,
                            toolCalls: parsed.toolCalls,
                            finishReason: finishReason,
                            usage: usage,
                            providerContinuation: providerContinuation
                        )))
                    }
                    continuation.finish()
                    return
                }
                if request.requiredToolName != nil {
                    throw NolaError.llm(.invalidResponse(reason: "This model provider cannot enforce a required native tool call."))
                }
                let output: String
                    if invocation.promptCache,
                       let digest = prompt.stablePrefixSHA256,
                       let cacheClient = client as? AgentPromptCacheIdentityClient {
                        output = try await cacheClient.chatAgentTurn(
                            message: prompt.prompt,
                            systemPrompt: prompt.systemPrompt,
                            enableThinking: enableThinking && invocation.reasoning,
                            stablePrefixSHA256: digest
                        )
                    } else {
                        output = try await client.chat(
                            message: prompt.prompt,
                            systemPrompt: prompt.systemPrompt,
                            enableThinking: enableThinking && invocation.reasoning,
                            timeout: nil
                        )
                    }
                    guard let parsed = AgentModelInterop.parse(
                        content: output,
                        allowedToolNames: prompt.allowedToolNames
                    ) else {
                        throw AgentCoreRuntimeError.missingResult
                    }
                    continuation.yield(.completed(.init(
                        content: parsed.content,
                        reasoning: parsed.reasoning,
                        toolCalls: parsed.toolCalls,
                        finishReason: parsed.finishReason
                    )))
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }

    private func validateInvocation(_ plan: AgentModelInvocationPlan) throws {
        let capabilities = admittedCapabilities
        if let expected = plan.maxOutputTokens, let actual = capabilities.maxOutputTokens,
           expected != actual {
            throw NolaError.llm(.invalidInput(reason: "The model output limit differs from its admitted invocation."))
        }
        guard (!plan.nativeToolCalls || capabilities.nativeToolCalls),
              (!plan.parallelToolCalls || capabilities.parallelToolCalls),
              (!plan.reasoning || capabilities.reasoning),
              (!plan.promptCache || capabilities.promptCache),
              (!plan.streamText || capabilities.streamText),
              (!plan.streamReasoning || capabilities.streamReasoning),
              (!plan.streamToolCalls || capabilities.streamToolCalls) else {
            throw NolaError.llm(.invalidInput(reason: "The model invocation exceeds its admitted execution capabilities."))
        }
    }
}
