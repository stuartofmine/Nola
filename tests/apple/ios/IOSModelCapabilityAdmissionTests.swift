import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSModelCapabilityAdmissionTests: XCTestCase {
    func testEventTransportWithoutDeclarationCannotGrantNativeToolsOrReasoning() async throws {
        try await run(provider: CapabilityEventTransport(), expectedTools: false, expectedThinking: false)
    }

    func testExplicitToolFalseSurvivesCoreAndActualModelCall() async throws {
        try await run(provider: DeclaredCapabilityEventTransport(capabilities:
            .init(maxOutputTokens: 64, nativeToolCalls: false, streamText: true)), expectedTools: false, expectedThinking: false)
    }

    func testExplicitToolsAndReasoningReachCoreInvocationAndEnableThinkingInput() async throws {
        try await run(provider: DeclaredCapabilityEventTransport(capabilities:
            .init(maxOutputTokens: 64, nativeToolCalls: true, reasoning: true, streamText: true,
                  streamReasoning: true, streamToolCalls: true)), expectedTools: true, expectedThinking: true)
    }

    private func run(provider: CapabilityEventTransport, expectedTools: Bool, expectedThinking: Bool) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sessionID = UUID().uuidString
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: .init(hostId: "ios-local", clientId: Bundle.main.bundleIdentifier ?? "nola-ios",
                sessionId: sessionID), isForeground: true, allowedTools: ["skill_invoke"])
        XCTAssertTrue(plan.output.enabledTools.contains { $0.toolId == "skill_invoke" })
        let store = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
        let turn = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: sessionID, conversationTitle: "Capability contract", identity: turn,
            userText: "继续；search/current 都只是普通文字", userContentJSON: nil, contextCheckpointID: nil))
        let client = InvocationRecordingClient(client: TextAgentModelClient(client: provider))
        let executor = IOSAgentHostExecutor()
        await executor.acquireAgentSession(sessionId: sessionID)
        let toolSession = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000), isForeground: true,
                authorizations: plan.authorizationSnapshot)
        })
        let coordinator = DeviceAgentCoordinator(modelClient: client, hostExecutor: executor,
            toolPlan: plan, canonicalApplication: application, toolPlanSession: toolSession,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: store, model: "fixture",
                outputTokens: 1_024, capabilities: client.coreModelCapabilityProfile))
        let stream = await coordinator.start(sessionId: sessionID, input: "继续；search/current 都只是普通文字",
            history: [], turnIdentity: turn, config: .init(platform: .ios, executionMode: .deviceLocal,
                model: "fixture", maxIterations: 1, contextWindowTokens: 64_000, reservedOutputTokens: 1_024))
        do { for try await _ in stream {} }
        catch {
            await executor.releaseAgentSession(sessionId: sessionID)
            throw error
        }
        await executor.releaseAgentSession(sessionId: sessionID)
        let request = try XCTUnwrap(client.lastRequest)
        if provider is LLMAgentCapabilityProviding {
            guard case .object(let profile)? = client.coreModelCapabilityProfile else {
                return XCTFail("Declared capabilities must reach the Core profile")
            }
            XCTAssertEqual(profile["max_output_tokens"], .signedInteger(64))
            XCTAssertEqual(request.invocationPlan.maxOutputTokens, 64,
                "Actual Core invocation output limit must remain separate from context reservation")
        }
        XCTAssertEqual(request.invocationPlan.nativeToolCalls, expectedTools)
        XCTAssertEqual(request.invocationPlan.reasoning, expectedThinking)
        XCTAssertEqual(request.invocationPlan.streamToolCalls, expectedTools)
        let input = try XCTUnwrap(provider.lastInput, "The actual TextAgent adapter must call the provider")
        XCTAssertEqual(input.userPrefs["enable_thinking"], expectedThinking ? "true" : "false")
        let encoded = try XCTUnwrap(input.userPrefs["prepared_core_payload_json"])
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(encoded.utf8)) as? [String: Any])
        let tools = try XCTUnwrap(payload["tools"] as? [[String: Any]])
        XCTAssertEqual(!tools.isEmpty, expectedTools,
            "Actual provider payload must follow Core's invocation, not transport conformance")
    }
}

private nonisolated class CapabilityEventTransport: LLMEventStreamingClient, @unchecked Sendable {
    private let lock = NSLock()
    private var input: ModelInput?
    var lastInput: ModelInput? { lock.withLock { input } }
    var supportsNativeEventStream: Bool { true }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        lock.withLock { self.input = input }
        return AsyncThrowingStream {
            $0.yield(.textDelta("done"))
            $0.yield(.completed(finishReason: "stop"))
            $0.finish()
        }
    }
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput { .empty }
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { $0.finish(throwing: URLError(.unsupportedURL)) }
    }
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        throw URLError(.unsupportedURL)
    }
}

private nonisolated final class DeclaredCapabilityEventTransport: CapabilityEventTransport,
    LLMAgentCapabilityProviding, @unchecked Sendable {
    let agentCapabilities: LLMAgentCapabilities
    init(capabilities: LLMAgentCapabilities) { agentCapabilities = capabilities }
}

private nonisolated final class InvocationRecordingClient: AgentModelClient, @unchecked Sendable {
    private let client: TextAgentModelClient
    private let lock = NSLock()
    private var request: AgentModelRequest?
    var lastRequest: AgentModelRequest? { lock.withLock { request } }
    var coreModelCapabilityProfile: AgentCoreJSONValue? { client.coreModelCapabilityProfile }
    init(client: TextAgentModelClient) { self.client = client }
    @MainActor
    func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
        lock.withLock { self.request = request }
        return client.generateAgentTurn(request: request)
    }
}
