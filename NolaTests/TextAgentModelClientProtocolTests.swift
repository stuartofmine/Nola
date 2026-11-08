import XCTest
@testable import Nola

@MainActor
final class TextAgentModelClientProtocolTests: XCTestCase {
    func testPublicReasoningStreamsWithoutControlledReasoningRequestCapability() async throws {
        let provider = PublicReasoningEventClient()
        let client = TextAgentModelClient(client: provider, enableThinking: true)
        var deltas: [String] = []
        var completion: AgentModelCompletion?
        for try await event in client.generateAgentTurn(request: capabilityRequest(
            .init(reasoning: false, streamText: true, streamReasoning: true))) {
            switch event {
            case .reasoningDelta(let value): deltas.append(value)
            case .completed(let value): completion = value
            default: break
            }
        }
        XCTAssertEqual(provider.requestThinking, "false", "Receiving a public stream must not enable a request control")
        XCTAssertEqual(deltas, ["Public ", "reasoning"])
        XCTAssertEqual(completion?.reasoning, "Public reasoning")
        XCTAssertEqual(completion?.providerContinuation, provider.outgoing)
        XCTAssertFalse((completion?.reasoning ?? "").contains("private-opaque"))
    }

    func testDisabledReasoningProjectionBuffersPublicTerminalAndKeepsContinuationOpaque() async throws {
        let provider = PublicReasoningEventClient()
        let client = TextAgentModelClient(client: provider, enableThinking: true)
        var deltas: [String] = []
        var completion: AgentModelCompletion?
        for try await event in client.generateAgentTurn(request: capabilityRequest(
            .init(reasoning: false, streamText: true, streamReasoning: false))) {
            switch event {
            case .reasoningDelta(let value): deltas.append(value)
            case .completed(let value): completion = value
            default: break
            }
        }
        XCTAssertEqual(provider.requestThinking, "false")
        XCTAssertTrue(deltas.isEmpty)
        XCTAssertEqual(completion?.reasoning, "Public reasoning", "A disabled incremental projection does not erase received terminal facts")
        XCTAssertEqual(completion?.providerContinuation, provider.outgoing)
        XCTAssertFalse((completion?.content ?? "").contains("private-opaque"))
        XCTAssertFalse((completion?.reasoning ?? "").contains("private-opaque"))
    }
    func testEndingAgentObservationCancelsItsProviderProducer() async throws {
        let started = expectation(description: "provider dispatched")
        let stopped = expectation(description: "provider cancellation propagated")
        let client = TextAgentModelClient(client: ControlledCancellationClient(started: started, stopped: stopped))
        let request = capabilityRequest(.init(streamText: true))
        let consumer = Task {
            do { for try await _ in client.generateAgentTurn(request: request) {} }
            catch {}
        }
        await fulfillment(of: [started], timeout: 3)
        consumer.cancel()
        await fulfillment(of: [stopped], timeout: 3)
        await consumer.value
    }

    func testCancellationJoinsExactProviderProducerAndRejectsAnotherOwner() async throws {
        let started = expectation(description: "provider dispatched")
        let stopped = expectation(description: "provider physically returned")
        let provider = ControlledCancellationClient(started: started, stopped: stopped)
        let client = TextAgentModelClient(client: provider)
        let request = capabilityRequest(.init(streamText: true))
        let consumer = Task {
            do { for try await _ in client.generateAgentTurn(request: request) {} }
            catch {}
        }
        await fulfillment(of: [started], timeout: 3)
        let foreign = AgentModelRequest(sessionId: "other", turnId: request.turnId,
            iteration: request.iteration, messages: request.messages, tools: [],
            reservedOutputTokens: 128)
        do {
            try await client.cancelAgentTurn(request: foreign)
            XCTFail("Another conversation cannot stop this producer")
        } catch DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed {}
        consumer.cancel()
        try await client.cancelAgentTurn(request: request)
        await fulfillment(of: [stopped], timeout: 3)
        await consumer.value
    }

    func testAgentKeepsAdmittedProviderWhenModelSelectionChanges() async throws {
        let admitted = LegacyThinkingClient()
        let replacement = LegacyThinkingClient()
        let selector = ModelService(provider: admitted)
        let running = TextAgentModelClient(client: selector)
        selector.installTestingProvider(replacement)
        for try await _ in running.generateAgentTurn(request: capabilityRequest(.init(reasoning: true))) {}
        XCTAssertEqual(admitted.receivedThinking, true)
        XCTAssertNil(replacement.receivedThinking, "Selection changes must not redirect an admitted turn")
        let successor = TextAgentModelClient(client: selector)
        for try await _ in successor.generateAgentTurn(request: capabilityRequest(.init(reasoning: true))) {}
        XCTAssertEqual(replacement.receivedThinking, true, "A new admission uses the new provider")
    }

    func testUndeclaredEventTransportRejectsNativePlanBeforeProviderCall() async throws {
        let provider = UndeclaredToolDeltaClient()
        let client = TextAgentModelClient(client: provider)
        var rejected = false
        do {
            for try await _ in client.generateAgentTurn(request: capabilityRequest(
                .init(nativeToolCalls: true, streamText: true, streamToolCalls: true))) {}
        } catch { rejected = true }
        XCTAssertTrue(rejected)
        XCTAssertEqual(provider.calls, 0)
    }

    func testUnexpectedToolDeltaWithoutNativeInvocationCannotBecomeCompletion() async throws {
        let provider = UndeclaredToolDeltaClient()
        var rejected = false
        var completed = false
        do {
            for try await event in TextAgentModelClient(client: provider).generateAgentTurn(
                request: capabilityRequest(.init(streamText: true))) {
                if case .completed = event { completed = true }
            }
        } catch { rejected = true }
        XCTAssertEqual(provider.calls, 1)
        XCTAssertTrue(rejected)
        XCTAssertFalse(completed)
    }

    func testLegacyThinkingRequiresBothUserPreferenceAndCoreInvocation() async throws {
        for (userEnabled, coreEnabled) in [(true, false), (false, true), (true, true)] {
            let provider = LegacyThinkingClient()
            let client = TextAgentModelClient(client: provider, enableThinking: userEnabled)
            if !userEnabled && coreEnabled {
                var rejected = false
                do {
                    for try await _ in client.generateAgentTurn(request: capabilityRequest(.init(reasoning: coreEnabled))) {}
                } catch { rejected = true }
                XCTAssertTrue(rejected, "A Core plan cannot re-enable an explicitly disabled reasoning capability")
                XCTAssertNil(provider.receivedThinking, "Reject before calling legacy chat")
                continue
            }
            for try await _ in client.generateAgentTurn(request: capabilityRequest(.init(reasoning: coreEnabled))) {}
            XCTAssertEqual(provider.receivedThinking, userEnabled && coreEnabled)
        }
    }

    func testMismatchedCoreOutputLimitIsRejectedBeforeProviderCall() async throws {
        let provider = LegacyThinkingClient()
        let client = TextAgentModelClient(client: provider)
        var rejected = false
        do {
            for try await _ in client.generateAgentTurn(request: capabilityRequest(.init(maxOutputTokens: 128))) {}
        } catch { rejected = true }
        XCTAssertTrue(rejected)
        XCTAssertNil(provider.receivedThinking)
    }

    private func capabilityRequest(_ invocation: AgentModelInvocationPlan) -> AgentModelRequest {
        .init(sessionId: "session", turnId: "turn", iteration: 1,
            messages: [.init(role: "user", content: "继续处理")],
            tools: [.object(["type": .string("function"), "function": .object(["name": .string("known_tool")])])],
            reservedOutputTokens: 128, invocationPlan: invocation)
    }

    func testOutputLimitFinishReasonsAreClassifiedWithoutPromptHeuristics() {
        XCTAssertTrue(isModelOutputLimitFinishReason("length"))
        XCTAssertTrue(isModelOutputLimitFinishReason(" MAX_OUTPUT_TOKENS "))
        XCTAssertFalse(isModelOutputLimitFinishReason("stop"))
        XCTAssertFalse(isModelOutputLimitFinishReason("tool_calls"))
    }

    func testProtocolCompletionDoesNotWaitForTransportClosureOrAppendLateData() async throws {
        let provider = TerminalThenDelayedEventClient()
        let client = TextAgentModelClient(client: provider)
        let request = AgentModelRequest(
            sessionId: "session",
            turnId: "turn",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "Respond briefly")],
            tools: [],
            reservedOutputTokens: 128,
            invocationPlan: AgentModelInvocationPlan(streamText: true)
        )

        let startedAt = Date()
        var deltas = ""
        var completion: AgentModelCompletion?
        for try await event in client.generateAgentTurn(request: request) {
            switch event {
            case .textDelta(let delta):
                deltas += delta
            case .completed(let value):
                completion = value
            default:
                break
            }
        }

        XCTAssertLessThan(Date().timeIntervalSince(startedAt), 0.8)
        XCTAssertEqual(deltas, "hello")
        XCTAssertEqual(completion?.content, "hello")
        XCTAssertEqual(completion?.finishReason, "stop")
    }

    func testUnknownNativeToolIdentityReachesCoreUnchanged() async throws {
        let provider = UnknownToolEventClient()
        let client = TextAgentModelClient(client: provider)
        let request = AgentModelRequest(
            sessionId: "session",
            turnId: "turn",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "继续处理；search 和 current 只是普通文字")],
            tools: [
                .object([
                    "type": .string("function"),
                    "function": .object(["name": .string("known_tool")]),
                ]),
            ],
            reservedOutputTokens: 128,
            invocationPlan: .init(nativeToolCalls: true, streamToolCalls: true)
        )

        var completion: AgentModelCompletion?
        for try await event in client.generateAgentTurn(request: request) {
            if case .completed(let value) = event { completion = value }
        }

        let call = try XCTUnwrap(completion?.toolCalls.first)
        XCTAssertEqual(call.id, "provider-call-7")
        XCTAssertEqual(call.name, "catalog_tool_v2")
        XCTAssertEqual(call.arguments, .object(["路径": .string("当前")]))
    }

    func testResponsesContinuationPassesThroughModelClient() async throws {
        let incoming: AgentCoreJSONValue = .object([
            "provider_namespace": .string("openai.responses"), "model_id": .string("fixture"),
        ])
        let outgoing: AgentCoreJSONValue = .object([
            "provider_namespace": .string("openai.responses"), "model_id": .string("fixture"),
            "response_id": .string("resp_next"),
        ])
        let provider = ContinuationEventClient(outgoing: outgoing)
        let client = TextAgentModelClient(client: provider)
        let request = AgentModelRequest(
            sessionId: "session", turnId: "turn", iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "继续")],
            tools: [.object(["type": .string("function"), "function": .object(["name": .string("read")])])],
            reservedOutputTokens: 128,
            invocationPlan: .init(nativeToolCalls: true, streamText: true, streamToolCalls: true),
            providerContinuation: incoming
        )
        var completion: AgentModelCompletion?
        for try await event in client.generateAgentTurn(request: request) {
            if case .completed(let value) = event { completion = value }
        }
        XCTAssertEqual(provider.received, incoming)
        XCTAssertEqual(completion?.providerContinuation, outgoing)
    }
}

private final class PublicReasoningEventClient: LLMEventStreamingClient, LLMAgentCapabilityProviding, @unchecked Sendable {
    var requestThinking: String?
    let outgoing: AgentCoreJSONValue = .object(["protocol": .string("responses"), "output": .array([
        .object(["type": .string("reasoning"), "encrypted_content": .string("private-opaque")]),
    ])])
    var agentCapabilities: LLMAgentCapabilities {
        .init(reasoning: false, streamText: true, streamReasoning: true, requestContinuation: true, maxRequestContinuations: 1)
    }
    var supportsNativeEventStream: Bool { true }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        requestThinking = input.userPrefs["enable_thinking"]
        return AsyncThrowingStream { continuation in
            continuation.yield(.reasoningDelta("Public "))
            continuation.yield(.reasoningDelta("reasoning"))
            continuation.yield(.textDelta("answer"))
            continuation.yield(.continuation(String(decoding: try! JSONEncoder().encode(outgoing), as: UTF8.self)))
            continuation.yield(.completed(finishReason: "stop"))
            continuation.finish()
        }
    }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { $0.finish() } }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String { "" }
}

private final class ControlledCancellationClient: LLMControlledEventStreamingClient, @unchecked Sendable {
    let started: XCTestExpectation
    let stopped: XCTestExpectation
    init(started: XCTestExpectation, stopped: XCTestExpectation) {
        self.started = started
        self.stopped = stopped
    }
    var supportsNativeEventStream: Bool { true }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        generateControlledEventStream(input: input).events
    }
    func generateControlledEventStream(input: ModelInput) -> LLMControlledEventStream {
        let control = LLMStreamProducerControl()
        let events = AsyncThrowingStream<LLMStreamEvent, Error> { continuation in
            let task = Task {
                started.fulfill()
                do { try await Task.sleep(nanoseconds: 60_000_000_000) }
                catch {}
                stopped.fulfill()
                continuation.finish()
            }
            control.install(task)
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
        return .init(events: events, producer: control)
    }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { $0.finish() } }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String { "" }
}

private final class UndeclaredToolDeltaClient: LLMEventStreamingClient, @unchecked Sendable {
    var calls = 0
    var supportsNativeEventStream: Bool { true }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        calls += 1
        return AsyncThrowingStream {
            $0.yield(.toolCallDelta(.init(index: 0, callId: "unadmitted", capabilityId: "known_tool", argumentsDelta: "{}")))
            $0.yield(.completed(finishReason: "tool_calls"))
            $0.finish()
        }
    }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { $0.finish() } }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String { "done" }
}

private final class LegacyThinkingClient: LLMAgentCapabilityProviding, @unchecked Sendable {
    var receivedThinking: Bool?
    var agentCapabilities: LLMAgentCapabilities { .init(maxOutputTokens: 64, reasoning: true) }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { $0.finish() } }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        receivedThinking = enableThinking
        return "done"
    }
}

private final class ContinuationEventClient: LLMEventStreamingClient, LLMAgentCapabilityProviding, @unchecked Sendable {
    var agentCapabilities: LLMAgentCapabilities {
        .init(nativeToolCalls: true, streamText: true, streamToolCalls: true,
              requestContinuation: true, maxRequestContinuations: 2)
    }
    let outgoing: AgentCoreJSONValue
    var received: AgentCoreJSONValue?
    var supportsNativeEventStream: Bool { true }
    init(outgoing: AgentCoreJSONValue) { self.outgoing = outgoing }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        received = input.providerContinuationJSON.flatMap { try? JSONDecoder().decode(AgentCoreJSONValue.self, from: Data($0.utf8)) }
        return AsyncThrowingStream { continuation in
            continuation.yield(.textDelta("done"))
            continuation.yield(.toolCallDelta(.init(index: 0, callId: "call_1", capabilityId: "read", argumentsDelta: "{}")))
            let outgoingJSON = try! JSONEncoder().encode(outgoing)
            continuation.yield(.continuation(String(decoding: outgoingJSON, as: UTF8.self)))
            continuation.yield(.completed(finishReason: "stop"))
            continuation.finish()
        }
    }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { $0.finish() } }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String { "" }
}

private final class UnknownToolEventClient: LLMEventStreamingClient, LLMAgentCapabilityProviding, @unchecked Sendable {
    var agentCapabilities: LLMAgentCapabilities {
        .init(nativeToolCalls: true, streamText: true, streamToolCalls: true)
    }
    var supportsNativeEventStream: Bool { true }

    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            continuation.yield(.toolCallDelta(.init(
                index: 0,
                callId: "provider-call-7",
                capabilityId: "catalog_tool_v2",
                argumentsDelta: #"{"路径":"当前"}"#
            )))
            continuation.yield(.completed(finishReason: "tool_calls"))
            continuation.finish()
        }
    }

    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { $0.finish() }
    }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String { "" }
}

private final class TerminalThenDelayedEventClient: LLMEventStreamingClient, @unchecked Sendable {
    var supportsNativeEventStream: Bool { true }

    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            continuation.yield(.textDelta("hello"))
            continuation.yield(.completed(finishReason: "stop"))
            Task {
                try? await Task.sleep(nanoseconds: 1_500_000_000)
                continuation.yield(.textDelta("late"))
                continuation.finish()
            }
        }
    }

    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        throw URLError(.unsupportedURL)
    }

    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            continuation.finish(throwing: URLError(.unsupportedURL))
        }
    }

    func chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Bool,
        timeout: TimeInterval?
    ) async throws -> String {
        throw URLError(.unsupportedURL)
    }
}
