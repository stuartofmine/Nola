// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS agent memory worker recovery/scheduling, native tool-call accumulator, request model builder, trace contracts, outbound context preparation, LLM capabilities/validator, shared protocol, and FFI linkage suite.

import Foundation
import Testing
import XCTest
@testable import Nola

@MainActor
final class AgentMemoryWorkerHostRecoveryTests: XCTestCase {
    func testCompletedTurnMemoryJobRunsThroughCanonicalApplicationAfterColdReopen() async throws {
        guard IOSApplicationRuntime.isAvailable else {
            throw XCTSkip("NolaCore runtime is unavailable")
        }

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-memory-worker-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }
        let conversationID = "ios-memory-worker-recovery"
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())

        do {
            let foregroundStore = try IOSApplicationRuntime(databaseURL: database)
            let application = IOSCanonicalAgentApplication(
                store: foregroundStore,
                descriptor: .init(
                    conversationID: conversationID,
                    conversationTitle: "Memory recovery",
                    identity: identity,
                    userText: "Prefer real end-to-end evidence for routing changes.",
                    userContentJSON: nil,
                    contextCheckpointID: nil
                )
            )
            let started = try application.begin(
                coreConfig: runtimeConfig(sessionID: conversationID),
                submission: userInput(identity: identity),
                createdAtMs: nowMilliseconds()
            )
            let claimedRequests = started
            if let checkpointRequest = hostRequest(in: claimedRequests, type: "persist_checkpoint") {
                _ = try application.acceptHostResponse(
                    hostResponse(
                        for: checkpointRequest,
                        payload: .object(["type": .string("execution_started")])
                    ),
                    receivedAtMs: nowMilliseconds()
                )
                _ = try application.acceptHostResponse(
                    hostResponse(
                        for: checkpointRequest,
                        payload: .object(["type": .string("accepted")]),
                        sequenceOffset: 2
                    ),
                    receivedAtMs: nowMilliseconds()
                )
            }
            let modelRequest = try XCTUnwrap(hostRequest(in: claimedRequests, type: "model_stream"))
            _ = try application.acceptHostResponse(
                hostResponse(
                    for: modelRequest,
                    payload: .object(["type": .string("execution_started")])
                ),
                receivedAtMs: nowMilliseconds()
            )
            try await Task.sleep(nanoseconds: 5_000_000)
            guard case .object(let modelEnvelope) = modelRequest else {
                return XCTFail("Expected a model Host request envelope")
            }
            let renewedModelRequest = try replacingDelivery(
                in: modelRequest,
                with: application.renewHostDelivery(
                    try XCTUnwrap(modelEnvelope["delivery"]),
                    heartbeatAtMs: nowMilliseconds()
                )
            )
            _ = try application.acceptHostResponse(
                hostResponse(
                    for: renewedModelRequest,
                    payload: .object([
                        "type": .string("model_attempt_started"),
                        "provider": .string("fixture-provider"),
                        "model": .string("fixture-model"),
                        "reason": .string("initial"),
                    ]),
                    sequenceOffset: 2
                ),
                receivedAtMs: nowMilliseconds()
            )
            _ = try application.acceptHostResponse(
                hostResponse(
                    for: renewedModelRequest,
                    payload: modelCompletion(),
                    sequenceOffset: 3
                ),
                receivedAtMs: nowMilliseconds()
            )
        }

        let workerStore = try IOSApplicationRuntime(databaseURL: database)
        let workerService = IOSCanonicalMemoryWorkerService(store: workerStore)
        let queued = try workerStore.executeAgentCommand(memoryJobsCommand(conversationID: conversationID))
        XCTAssertEqual(arrayCount(queued), 1)
        XCTAssertTrue(try workerService.hasClaimableJobs(conversationID: conversationID))

        let modelClient = MemoryWorkerFixtureModelClient()
        let report = try await workerService.runReport(
            conversationID: conversationID,
            coreConfig: runtimeConfig(sessionID: conversationID),
            modelClient: modelClient,
            maximumJobCount: 1
        )
        XCTAssertEqual(
            report,
            IOSMemoryWorkerRunReport(attempted: 1, completed: 1, retryScheduled: 0)
        )
        XCTAssertEqual(modelClient.request?.requiredToolName, AgentMemoryWorkerModelExecutor.submissionToolName)
        XCTAssertFalse(try workerService.hasClaimableJobs(conversationID: conversationID))

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(
            arrayCount(try reopened.executeAgentCommand(memoryJobsCommand(conversationID: conversationID))),
            0
        )
    }

    private func runtimeConfig(sessionID: String) -> AgentCoreJSONValue {
        .object([
            "session_id": .string(sessionID),
            "model": .string("fixture-model"),
            "turn_identity": IOSCanonicalHostTestProfile.turnIdentity(toolIDs: []),
            "initial_items": .array([]),
            "tools": .array([]),
            "host_catalog": .object([
                "host_id": .string("ios-memory-test-host"),
                "revision": .number(1),
                "availability_epoch": .number(1),
                "capabilities": .object([
                    "model.stream": capability("model.stream", route: "model", streaming: true),
                ]),
            ]),
            "permission_rules": .object(["rules": .array([])]),
            "memory_policy": .object([
                "long_term_enabled": .bool(true),
                "automatic_extraction_enabled": .bool(true),
                "search_limit": .number(8),
                "minimum_confidence": .number(0.5),
                "maximum_age_days": .null,
            ]),
        ])
    }

    private func capability(_ id: String, route: String, streaming: Bool = false) -> AgentCoreJSONValue {
        .object([
            "id": .string(id), "title": .string(id), "route": .string(route),
            "availability": .string("available"), "exposure": .string("visible"),
            "permission_domain": .null,
            "input_schema": .object(["type": .string("object")]),
            "output_policy": .object([
                "preview_char_limit": .number(4_000),
                "persist_full_output": .bool(true),
                "allow_artifacts": .bool(true),
            ]),
            "supports_streaming": .bool(streaming),
            "supports_argument_streaming": .bool(false),
            "parallelism": .string("serial"),
            "cancellation": .string("cooperative"),
            "replay_safety": .string("recoverable"),
        ])
    }

    private func userInput(identity: IOSCanonicalAgentTurnIdentity) -> AgentCoreJSONValue {
        .object([
            "type": .string("user_input"),
            "items": .array([.object([
                "type": .string("message"),
                "id": .string(identity.userMessageID),
                "role": .string("user"),
                "content": .string("Prefer real end-to-end evidence for routing changes."),
            ])]),
            "submission_id": .string(identity.submissionID),
            "delivery": .string("current_turn"),
        ])
    }

    private func modelCompletion() -> AgentCoreJSONValue {
        .object([
            "type": .string("model_completed"),
            "items": .array([.object([
                "type": .string("message"),
                "id": .string("memory-assistant-1"),
                "role": .string("assistant"),
                "content": .string("I will preserve that preference through the internal memory workflow."),
            ])]),
            "tool_calls": .array([]),
            "usage": .object([
                "input_tokens": .number(8),
                "output_tokens": .number(3),
                "total_tokens": .number(11),
            ]),
        ])
    }

    private func memoryJobsCommand(conversationID: String) -> AgentCoreJSONValue {
        .object([
            "type": .string("query_claimable_memory_jobs"),
            "input": .object([
                "conversation_id": .string(conversationID),
                "now_epoch_ms": .number(Double(nowMilliseconds())),
            ]),
        ])
    }

    private func hostRequest(in transition: AgentCoreJSONValue, type: String) -> AgentCoreJSONValue? {
        guard case .object(let object) = transition,
              case .array(let requests)? = object["host_requests"] else { return nil }
        return requests.reversed().first { request in
            guard case .object(let envelope) = request,
                  case .object(let payload)? = envelope["payload"] else { return false }
            return payload["type"] == .string(type)
        }
    }

    private func claimHostRequests(
        store: IOSApplicationRuntime,
        runID: String,
        nowMs: Int64
    ) throws -> AgentCoreJSONValue {
        let result = try store.executeAgentCommand(.object([
            "type": .string("claim_committed_host_requests"),
            "input": .object([
                "run_id": .string(runID),
                "worker_generation": .string("ios-memory-test-generation"),
                "lease_owner": .string("ios-memory-test-worker"),
                "claimed_at_ms": .number(Double(nowMs)),
                "lease_expires_at_ms": .number(Double(nowMs + 60_000)),
                "limit": .number(64),
            ]),
        ]))
        guard case .object(let object) = result,
              case .array(let requests)? = object["requests"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Claimed Host requests are missing"
            )
        }
        for request in requests {
            guard case .object(let envelope) = request,
                  envelope["attempt_id"] != nil,
                  envelope["delivery"] != nil else {
                throw IOSCanonicalAgentApplicationError.invalidResult(
                    "Claimed Host request omitted its delivery attempt identity"
                )
            }
        }
        return .object(["host_requests": .array(requests)])
    }

    private func hostResponse(
        for request: AgentCoreJSONValue,
        payload: AgentCoreJSONValue,
        sequenceOffset: Double = 1
    ) -> AgentCoreJSONValue {
        guard case .object(let envelope) = request else { return .null }
        let resumedSequence: Double
        if case .number(let value)? = envelope["resume_after_sequence"] { resumedSequence = value }
        else { resumedSequence = 0 }
        return .object([
            "attempt_id": envelope["attempt_id"] ?? .null,
            "delegated_invocation_id": envelope["delegated_invocation_id"] ?? .null,
            "request_id": envelope["request_id"] ?? .null,
            "effect_id": envelope["effect_id"] ?? .null,
            "host_id": envelope["host_id"] ?? .null,
            "lease_id": envelope["lease_id"] ?? .null,
            "session_id": envelope["session_id"] ?? .null,
            "turn_id": envelope["turn_id"] ?? .null,
            "catalog_revision": envelope["catalog_revision"] ?? .number(1),
            "availability_epoch": envelope["availability_epoch"] ?? .number(1),
            "delivery": envelope["delivery"] ?? .null,
            "sequence": .number(resumedSequence + sequenceOffset),
            "payload": payload,
        ])
    }

    private func replacingDelivery(
        in request: AgentCoreJSONValue,
        with delivery: AgentCoreJSONValue
    ) throws -> AgentCoreJSONValue {
        guard case .object(var envelope) = request else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Host request envelope is missing"
            )
        }
        envelope["delivery"] = delivery
        return .object(envelope)
    }

    private func arrayCount(_ value: AgentCoreJSONValue) -> Int {
        guard case .array(let values) = value else { return -1 }
        return values.count
    }

    private func nowMilliseconds() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1_000)
    }
}

@MainActor
private final class MemoryWorkerFixtureModelClient: AgentModelClient {
    var request: AgentModelRequest?

    func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
        self.request = request
        let output = AgentCoreJSONValue.object([
            "type": .string("extraction"),
            "summary": .string("No durable memory was extracted."),
            "candidates": .array([]),
        ])
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(.init(
                content: "",
                toolCalls: [AgentCoreToolCall(
                    id: "memory-worker-result",
                    name: AgentMemoryWorkerModelExecutor.submissionToolName,
                    arguments: output
                )],
                finishReason: "tool_calls"
            )))
            continuation.finish()
        }
    }
}

final class AgentNativeToolCallAccumulatorTests: XCTestCase {
    func testParallelDeltasBecomeOrderedCompleteCalls() throws {
        var accumulator = AgentNativeToolCallAccumulator()
        accumulator.append(
            callId: "call-a",
            capabilityId: "read_file",
            argumentsDelta: #"{"path":""#
        )
        accumulator.append(
            callId: "call-b",
            capabilityId: "web_search",
            argumentsDelta: #"{"query":"current"#
        )
        accumulator.append(
            callId: "call-a",
            capabilityId: "read_file",
            argumentsDelta: #"/tmp/a"}"#
        )
        accumulator.append(
            callId: "call-b",
            capabilityId: "web_search",
            argumentsDelta: #" status"}"#
        )

        let calls = try accumulator.completedCalls()

        XCTAssertEqual(calls.map(\.id), ["call-a", "call-b"])
        XCTAssertEqual(calls.map(\.name), ["read_file", "web_search"])
        XCTAssertEqual(
            calls[0].arguments,
            .object(["path": .string("/tmp/a")])
        )
        XCTAssertEqual(
            calls[1].arguments,
            .object(["query": .string("current status")])
        )
    }

    func testInvalidFinalArgumentsAreRejectedInsteadOfSilentlyExecuting() {
        var accumulator = AgentNativeToolCallAccumulator()
        accumulator.append(
            callId: "call-invalid",
            capabilityId: "read_file",
            argumentsDelta: #"{"path":"unterminated""#
        )

        XCTAssertThrowsError(try accumulator.completedCalls())
    }

    func testProviderIndexKeepsLateCallIdentityAndArgumentsInOneLogicalCall() throws {
        var accumulator = AgentNativeToolCallAccumulator()

        XCTAssertNil(accumulator.append(
            index: 3,
            callId: "",
            capabilityId: "",
            argumentsDelta: "{\"path\":\""
        ))
        let activated = accumulator.append(
            index: 3,
            callId: "provider-id-arrived-late",
            capabilityId: "read_file",
            argumentsDelta: "notes.md\"}"
        )

        XCTAssertEqual(activated?.callId, "tool-call-3")
        XCTAssertEqual(activated?.capabilityId, "read_file")
        XCTAssertEqual(activated?.argumentsDelta, #"{"path":"notes.md"}"#)
        let calls = try accumulator.completedCalls()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0].id, "tool-call-3")
        XCTAssertEqual(calls[0].name, "read_file")
        XCTAssertEqual(calls[0].arguments, .object(["path": .string("notes.md")]))
    }

    func testFallbackIdentityIsScopedToTheModelIteration() throws {
        var first = AgentNativeToolCallAccumulator(fallbackCallIdPrefix: "turn-a-1-tool-call")
        var second = AgentNativeToolCallAccumulator(fallbackCallIdPrefix: "turn-a-2-tool-call")

        _ = first.append(index: 0, callId: "", capabilityId: "read_file", argumentsDelta: "{}")
        _ = second.append(index: 0, callId: "", capabilityId: "read_file", argumentsDelta: "{}")

        XCTAssertEqual(try first.completedCalls().first?.id, "turn-a-1-tool-call-0")
        XCTAssertEqual(try second.completedCalls().first?.id, "turn-a-2-tool-call-0")
    }

    @MainActor
    func testMemoryWorkerExecutorRequiresStructuredSubmissionTool() async throws {
        let expected: AgentCoreJSONValue = .object([
            "candidates": .array([]),
        ])
        let client = CapturingAgentModelClient(completion: .init(
            content: "",
            toolCalls: [AgentCoreToolCall(
                id: "memory-call",
                name: AgentMemoryWorkerModelExecutor.submissionToolName,
                arguments: expected
            )],
            finishReason: "tool_calls"
        ))
        let plan: AgentCoreJSONValue = .object([
            "job_id": .string("job-1"),
            "system_instructions": .string("Extract durable memory."),
            "model_input": .object(["transcript": .array([])]),
            "response_schema": .object([
                "type": .string("object"),
                "properties": .object([:]),
            ]),
        ])

        let output = try await AgentMemoryWorkerModelExecutor.execute(
            plan: plan,
            client: client
        )

        XCTAssertEqual(output, expected)
        XCTAssertEqual(
            client.request?.requiredToolName,
            AgentMemoryWorkerModelExecutor.submissionToolName
        )
        XCTAssertEqual(client.request?.tools.count, 1)
        XCTAssertEqual(client.request?.invocationPlan.nativeToolCalls, true)
    }

    @MainActor
    func testStructuredOutputExecutorRejectsDuplicateRequiredCalls() async {
        let arguments: AgentCoreJSONValue = .object(["value": .string("first")])
        let toolName = "submit_fixture"
        let client = CapturingAgentModelClient(completion: .init(
            content: "",
            toolCalls: [
                AgentCoreToolCall(id: "call-1", name: toolName, arguments: arguments),
                AgentCoreToolCall(id: "call-2", name: toolName, arguments: arguments),
            ],
            finishReason: "tool_calls"
        ))

        do {
            _ = try await AgentStructuredOutputExecutor.execute(
                sessionId: "fixture-session",
                turnId: "fixture-turn",
                systemInstructions: "Return structured output.",
                modelInput: .object(["input": .string("fixture")]),
                responseSchema: .object(["type": .string("object")]),
                submissionToolName: toolName,
                submissionToolDescription: "Submit the fixture.",
                client: client
            )
            XCTFail("duplicate required tool calls must be rejected")
        } catch {
            XCTAssertEqual(client.request?.requiredToolName, toolName)
        }
    }
}

@MainActor
private final class CapturingAgentModelClient: AgentModelClient {
    let completion: AgentModelCompletion
    var request: AgentModelRequest?

    init(completion: AgentModelCompletion) {
        self.completion = completion
    }

    func generateAgentTurn(
        request: AgentModelRequest
    ) -> AsyncThrowingStream<AgentModelEvent, Error> {
        self.request = request
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(completion))
            continuation.finish()
        }
    }
}

final class AgentRequestModelBuilderTests: XCTestCase {
    func testMainTurnPreservesCoreSystemWithoutHostRecomposition() throws {
        let builder = AgentRequestModelBuilder { _ in
            XCTFail("Main Agent turns must not be recomposed by the iOS Host")
            throw AgentCorePromptComposerError.invalidResponse
        }
        let query = "search is an ambiguous word, do not force a tool"
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "turn-1",
            iteration: 2,
            messages: [
                AgentCoreMessage(role: "system", content: "CORE REFERENCE"),
                AgentCoreMessage(role: "user", content: query),
            ],
            tools: [
                .object([
                    "type": .string("function"),
                    "function": .object(["name": .string("device_read_file")]),
                ]),
            ],
            reservedOutputTokens: 4_096,
            stablePrefixSHA256: "core-digest"
        )

        let prompt = try builder.build(request)

        XCTAssertEqual(prompt.systemPrompt, "CORE REFERENCE")
        XCTAssertEqual(prompt.allowedToolNames, ["device_read_file"])
        XCTAssertTrue(prompt.prompt.contains(query))
        XCTAssertEqual(prompt.stablePrefixSHA256, "core-digest")
    }

    func testBuilderDoesNotInferToolsFromNaturalLanguage() throws {
        let builder = AgentRequestModelBuilder { _ in
            XCTFail("Natural language must not trigger prompt recomposition")
            throw AgentCorePromptComposerError.invalidResponse
        }
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "turn-1",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "搜索当前文件")],
            tools: [],
            reservedOutputTokens: 1_024
        )

        let prompt = try builder.build(request)

        XCTAssertTrue(prompt.allowedToolNames.isEmpty)
        XCTAssertTrue(prompt.prompt.contains("搜索当前文件"))
        XCTAssertFalse(prompt.systemPrompt.contains("device_read_file"))
    }

    func testAuxiliaryRequestComposesOnlyExplicitPolicyFragments() throws {
        var captured: [AgentCorePromptFragment] = []
        let builder = AgentRequestModelBuilder { fragments in
            captured = fragments
            return AgentCorePromptComposition(
                stablePrefix: "",
                dynamicSuffix: "AUXILIARY POLICY",
                rendered: "AUXILIARY POLICY",
                fragmentIds: fragments.map(\.id),
                stablePrefixSha256: "auxiliary-digest",
                stableFragmentSha256: nil
            )
        }
        let policy = AgentCorePromptFragment(
            id: "core-context-compaction",
            layer: .executionPolicy,
            trust: .hostInstruction,
            cacheable: false,
            content: "Summarize faithfully."
        )
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "compaction",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "history")],
            tools: [],
            reservedOutputTokens: 1_024,
            policyFragments: [policy]
        )

        let prompt = try builder.build(request)

        XCTAssertEqual(captured.map(\.id), ["core-context-compaction"])
        XCTAssertEqual(prompt.systemPrompt, "AUXILIARY POLICY")
        XCTAssertNil(prompt.stablePrefixSHA256)
    }
}

final class AgentTraceContractTests: XCTestCase {
    func testCompactionOwnsStageUntilCompletionAndModelProgressIsNotATool() {
        var reducer = AgentTraceReducer()
        func event(_ sequence: UInt64, _ kind: AgentTraceKind, _ payload: AgentTracePayload) -> AgentTraceEvent {
            AgentTraceEvent(traceId: "trace", runId: "run", conversationId: "conversation", turnId: "turn", sequence: sequence, kind: kind, spanId: "model-effect", payload: payload)
        }
        reducer.apply(event(1, .phaseChanged, .init(phase: "compacting")))
        reducer.apply(event(2, .toolProgress, .init(summary: "model_attempt_started")))
        XCTAssertEqual(reducer.projection.phase, "compacting")
        reducer.apply(event(3, .toolProgress, .init(summary: "model_response_provenance")))
        XCTAssertTrue(reducer.projection.activities.isEmpty)
        reducer.apply(event(4, .contextCompacted, .init(phase: "compacted")))
        XCTAssertEqual(reducer.projection.phase, "processing")
        reducer.apply(event(5, .toolProgress, .init(summary: "model_retry")))
        XCTAssertEqual(reducer.projection.phase, "model_retry")
        reducer.apply(event(6, .toolProgress, .init(summary: "model_attempt_started")))
        XCTAssertEqual(reducer.projection.phase, "processing")
        XCTAssertTrue(reducer.projection.activities.isEmpty)
        reducer.apply(event(7, .phaseChanged, .init(phase: "compacting")))
        reducer.apply(event(8, .runCancelled, .init(reason: "cancelled")))
        XCTAssertNil(reducer.projection.phase)
    }

    func testTraceLifecycleSettlesCanonicalPartToolPresentation() {
        let partEvent = HostAgentToolEvent(
            id: "call-1",
            name: "read_file",
            argsPreview: #"{"path":"Doc/README.md"}"#,
            status: .running,
            display: "Read file"
        )
        let completedTrace = HostAgentToolEvent(
            id: "call-1",
            name: "read_file",
            status: .succeeded,
            text: "read 12 lines"
        )

        let reconciled = partEvent.reconcilingLifecycle(from: completedTrace)

        XCTAssertEqual(reconciled.status, .succeeded)
        XCTAssertEqual(reconciled.argsPreview, partEvent.argsPreview)
        XCTAssertEqual(reconciled.display, "Read file")
        XCTAssertEqual(reconciled.text, "read 12 lines")
    }

    func testSharedCoreLongTaskFixtureReplaysIdenticallyOnIOS() throws {
        let fixtureURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "long_task_trace_v1",
                withExtension: "json"
            )
        )
        let data = try Data(contentsOf: fixtureURL)
        let events = try JSONDecoder().decode([AgentTraceEvent].self, from: data)
        var reducer = AgentTraceReducer()
        var results: [AgentTraceApplyResult] = []

        for event in events {
            results.append(reducer.apply(event))
        }

        XCTAssertEqual(results.filter { $0 == .duplicate }.count, 1)
        XCTAssertFalse(results.contains {
            if case .gap = $0 { return true }
            if case .outOfOrder = $0 { return true }
            return false
        })
        XCTAssertEqual(reducer.projection.traceId, "trace_contract_long_task")
        XCTAssertEqual(reducer.projection.phase, "planning")
        XCTAssertEqual(reducer.projection.activities.map(\.toolName), ["web_search", "read_file"])
        XCTAssertEqual(reducer.projection.activities.map(\.status), [.succeeded, .succeeded])
        XCTAssertEqual(reducer.projection.activities.last?.resources.first?.uri, "Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md")
        XCTAssertEqual(reducer.projection.usage?.usedTokens, 8_192)
        XCTAssertEqual(reducer.projection.usage?.contextWindowTokens, 131_072)
        XCTAssertEqual(reducer.projection.status, .succeeded)
        XCTAssertEqual(reducer.projection.lastSequence, 9)
    }

    func testDecodesAuthoritativeCoreTraceFixture() throws {
        let object: [String: Any] = [
            "type": "phase",
            "trace": [
                "schema_version": 1,
                "trace_id": "trace_fixture",
                "run_id": "run_fixture",
                "conversation_id": "conversation_fixture",
                "message_id": "assistant_fixture",
                "turn_id": "turn_fixture",
                "event_id": "event_fixture",
                "sequence": 7,
                "emitted_at": "2026-07-19T08:00:00.000Z",
                "kind": "phase.changed",
                "parent_span_id": "turn_fixture",
                "status": "running",
                "visibility": "summary",
                "payload": ["phase": "processing"],
            ],
        ]

        let trace = try XCTUnwrap(AgentTraceAdapter.decodeNestedTrace(from: object))
        XCTAssertEqual(trace.schemaVersion, 1)
        XCTAssertEqual(trace.sequence, 7)
        XCTAssertEqual(trace.kind, .phaseChanged)
        XCTAssertEqual(trace.payload.phase, "processing")
    }

    func testLongTaskProjectionPreservesOrderedIntermediateState() {
        var context = AgentTraceContext(conversationId: "conversation", messageId: "assistant")
        var reducer = AgentTraceReducer()
        let callId = "call_search"
        let events = [
            context.makeEvent(kind: .runStarted, payload: .init(phase: "planning")),
            context.makeEvent(
                kind: .toolStarted,
                spanId: callId,
                payload: .init(
                    summary: "搜索最新资料",
                    toolName: "web_search",
                    callId: callId,
                    arguments: .object(["query": .string("latest statement")])
                )
            ),
            context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: callId,
                payload: .init(toolName: "web_search", callId: callId, permissionRequestId: "permission")
            ),
            context.makeEvent(
                kind: .toolPermissionResolved,
                spanId: callId,
                payload: .init(toolName: "web_search", callId: callId, permissionDecision: "allow_once")
            ),
            context.makeEvent(
                kind: .toolCompleted,
                status: .succeeded,
                spanId: callId,
                payload: .init(
                    summary: "找到 20 个结果",
                    toolName: "web_search",
                    callId: callId,
                    result: .object(["count": .number(20)])
                )
            ),
            context.makeEvent(
                kind: .usageSnapshot,
                visibility: .detail,
                payload: .init(usage: .init(
                    usedTokens: 4_096,
                    contextWindowTokens: 128_000,
                    promptTokenLimit: 120_000
                ))
            ),
            context.makeEvent(kind: .runCompleted, status: .succeeded),
        ]

        for event in events {
            XCTAssertEqual(reducer.apply(event), .applied)
        }

        XCTAssertEqual(reducer.projection.lastSequence, 7)
        XCTAssertEqual(reducer.projection.activities.count, 1)
        XCTAssertEqual(reducer.projection.activities.first?.status, .succeeded)
        XCTAssertEqual(reducer.projection.activities.first?.arguments, .object(["query": .string("latest statement")]))
        XCTAssertEqual(reducer.projection.usage?.usedTokens, 4_096)
        XCTAssertEqual(reducer.projection.status, .succeeded)
    }

    func testPermissionResumePreservesControlPhaseAndResolvesExactActivity() {
        var context = AgentTraceContext(conversationId: "conversation")
        var reducer = AgentTraceReducer()
        let initial = [
            context.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing")),
            context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: "call",
                payload: .init(toolName: "read_file", callId: "call", permissionRequestId: "request")
            ),
        ]
        initial.forEach { XCTAssertEqual(reducer.apply($0), .applied) }

        XCTAssertEqual(reducer.apply(AgentTraceAdapter.permissionResolved(
            requestId: "request",
            toolName: "read_file",
            allowed: true,
            context: &context
        )), .applied)
        XCTAssertEqual(reducer.apply(context.makeEvent(
            kind: .phaseChanged,
            payload: .init(phase: "processing")
        )), .applied)

        XCTAssertEqual(reducer.projection.phase, "processing")
        XCTAssertEqual(reducer.projection.lastSequence, 4)
        XCTAssertEqual(reducer.projection.activities.first?.status, .running)
    }

    func testSequenceAndEventIdAreStableAndIdempotent() {
        var context = AgentTraceContext(conversationId: "conversation")
        var reducer = AgentTraceReducer()
        let first = context.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing"))
        XCTAssertEqual(reducer.apply(first), .applied)
        XCTAssertEqual(reducer.apply(first), .duplicate)

        var gap = context.makeEvent(kind: .phaseChanged, payload: .init(phase: "waiting"))
        gap.sequence = 3
        XCTAssertEqual(reducer.apply(gap), .gap(expected: 2, received: 3))

        var outOfOrder = first
        outOfOrder.eventId = "new-id-old-sequence"
        XCTAssertEqual(reducer.apply(outOfOrder), .outOfOrder(last: 1, received: 1))
        XCTAssertEqual(reducer.projection.phase, "processing")
    }

    func testRemovedBodyTraceKindsCannotEnterControlProjection() throws {
        var mutable = AgentTraceContext(conversationId: "conversation")
        let control = mutable.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing"))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(control)) as? [String: Any])
        for kind in ["assistant.delta", "assistant.reset", "thinking.delta"] {
            object["kind"] = kind
            object["payload"] = ["delta": "正文只能由 canonical journal 提供"]
            XCTAssertNil(AgentTraceAdapter.decodeNestedTrace(from: ["trace": object]))
        }
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(control)) as? [String: Any])
        XCTAssertNil((encoded["payload"] as? [String: Any])?["delta"])
    }
}

final class OutboundContextPreparationTests: XCTestCase {
    func testCoreTurnKeepsLatestSubmissionSeparateFromCommittedHistory() {
        let first = "Explain blue sky. Begin SKY_REVIEW"
        let second = "Now explain red sunsets. Begin SUNSET_REVIEW"
        let history = [
            AgentCoreContextPrepareMessage(id: "system", role: "system", content: "typed context"),
            AgentCoreContextPrepareMessage(id: "u1", role: "user", content: first),
            AgentCoreContextPrepareMessage(id: "a1", role: "assistant", content: "SKY_REVIEW: Blue light scatters.")
        ]
        let turn = ConversationTurnInput(currentUserText: second, historySource: .prepared(history.map { .init(role: $0.role, content: $0.content) }))
        XCTAssertEqual(turn.input, second)
        XCTAssertEqual(turn.history.map(\.role), ["user", "assistant"])
        XCTAssertEqual(turn.history.map(\.content), [first, "SKY_REVIEW: Blue light scatters."])
    }

    func testFirstCoreTurnUsesPlainSubmittedTextRatherThanProviderEnvelope() {
        let prompt = "请解释晚霞；从 SUNSET_REVIEW 开始。"
        let turn = ConversationTurnInput(currentUserText: prompt, historySource: .prepared([]))
        XCTAssertEqual(turn.input, prompt)
        XCTAssertTrue(turn.history.isEmpty)
    }

    func testIdenticalOrUnansweredPriorUserTurnIsNotRemovedFromHistory() {
        let prompt = "再解释一下"
        let history = [AgentCoreContextPrepareMessage(id: "prior", role: "user", content: prompt)]
        let turn = ConversationTurnInput(currentUserText: prompt, historySource: .prepared(history.map { .init(role: $0.role, content: $0.content) }))
        XCTAssertEqual(turn.input, prompt)
        XCTAssertEqual(turn.history.map(\.content), [prompt], "Text equality cannot identify a new turn")
    }

    func testSessionMemoryWireKindIsSeparateFromDurableMemoryAndUserInstructions() throws {
        for kind: AgentCoreContextComponentKind in [.sessionMemory, .memory, .userPreference] {
            let component = AgentCoreContextComponent(kind: kind, id: "fixture", content: .string("same text"))
            let encoded = try JSONEncoder().encode(component)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreContextComponent.self, from: encoded), component)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
            XCTAssertEqual(object["kind"] as? String, kind.rawValue)
        }
        XCTAssertEqual(AgentCoreContextComponentKind.sessionMemory.rawValue, "session_memory")
    }

    @MainActor
    func testDurableLearnedPreferencesRemainMemoryNotUserInstructions() {
        for content in ["Prefer concise answers", "Procedure: verify the result", "普通长期记忆", "session_memory"] {
            let component = DefaultContextManager.durableMemoryComponent(id: "durable-fixture", content: content)
            XCTAssertEqual(component.kind, .memory)
            XCTAssertEqual(component.content, .string(content))
        }
    }

    func testReadyCoreMessagesAreForwardedInAuthoritativeOrderWithoutSwiftTrimming() throws {
        let prepared = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "system", role: "system", content: "policy"),
                AgentCoreContextPrepareMessage(id: "old-user", role: "user", content: "old question"),
                AgentCoreContextPrepareMessage(id: "old-assistant", role: "assistant", content: "Core-compacted answer"),
                AgentCoreContextPrepareMessage(id: "new-user", role: "user", content: "latest question")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 20,
                toolSchemaTokens: 0,
                totalPromptTokens: 20,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 20,
                beforeTokens: 80,
                afterTokens: 20
            ),
            archiveRequest: nil
        )

        let segments = try DefaultContextManager.outboundPromptSegments(from: prepared)

        XCTAssertEqual(segments.map(\.role), ["system", "user", "assistant", "user"])
        XCTAssertEqual(segments.map(\.content), ["policy", "old question", "Core-compacted answer", "latest question"])
    }

    func testLatestPersistedUserIsIncludedExactlyOnceInProviderEnvelope() throws {
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "assistant", role: "assistant", content: "previous answer"),
                AgentCoreContextPrepareMessage(id: "user", role: "user", content: "latest question")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 12,
                toolSchemaTokens: 0,
                totalPromptTokens: 12,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 12,
                beforeTokens: 12,
                afterTokens: 12
            ),
            archiveRequest: nil
        )
        let prepared = try CorePreparedOutboundPayload(response: response)
        let data = try XCTUnwrap(
            prepared.providerEnvelopeJSON(
                systemPrompt: "composed policy",
                userQuery: "latest question"
            ).data(using: .utf8)
        )
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let messages = try XCTUnwrap(envelope["messages"] as? [[String: Any]])
        let contents = messages.compactMap { $0["content"] as? String }

        XCTAssertEqual(envelope["system_prompt"] as? String, "composed policy")
        XCTAssertEqual(contents.filter { $0 == "latest question" }.count, 1)
        XCTAssertEqual(contents, ["previous answer", "latest question"])
        XCTAssertFalse(String(data: data, encoding: .utf8)?.contains("[User]:") == true)
    }

    func testPreparedPayloadReportsOnlyDataSourcesActuallyIncludedByCore() throws {
        let usedID = UUID()
        let truncatedID = UUID()
        let droppedID = UUID()
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [AgentCoreContextPrepareMessage(id: "user", role: "user", content: "question")],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 10,
                toolSchemaTokens: 0,
                totalPromptTokens: 10,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 10,
                beforeTokens: 10,
                afterTokens: 10
            ),
            componentMetrics: [
                AgentCoreContextComponentMetric(
                    kind: .platformData,
                    usedTokens: 6,
                    droppedTokens: 4,
                    truncatedTokens: 2,
                    usedIds: ["data_source:\(usedID.uuidString)"],
                    droppedIds: ["data_source:\(droppedID.uuidString)"],
                    truncatedIds: ["data_source:\(truncatedID.uuidString)"]
                ),
                AgentCoreContextComponentMetric(
                    kind: .platformData,
                    usedTokens: 2,
                    droppedTokens: 0,
                    truncatedTokens: 0,
                    usedIds: ["platform_data:health", "data_source:\(usedID.uuidString)"],
                    droppedIds: [],
                    truncatedIds: []
                )
            ]
        )

        let payload = try CorePreparedOutboundPayload(response: response)

        XCTAssertEqual(Set(payload.usedDataSourceIDs), Set([usedID, truncatedID]))
        XCTAssertFalse(payload.usedDataSourceIDs.contains(droppedID))
    }

    func testPreparedPayloadRetainsTypedComponentsForCoreHostedTurn() throws {
        let skill = AgentCoreContextComponent(
            kind: .skill,
            id: "skill:word",
            content: .string(#"{"type":"available_skill","name":"Word","invocation":{"tool":"skill_invoke","arguments":{"skill":"Word"}}}"#),
            source: .init(platform: .ios, title: "Word")
        )
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "skill:word", role: "system", content: "prepared skill"),
                AgentCoreContextPrepareMessage(id: "user", role: "user", content: "create a document")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 12,
                toolSchemaTokens: 8,
                totalPromptTokens: 20,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 20,
                beforeTokens: 20,
                afterTokens: 20
            ),
            archiveRequest: nil
        )

        let payload = try CorePreparedOutboundPayload(
            response: response,
            contextComponents: [skill]
        )

        XCTAssertEqual(payload.contextComponents, [skill])
        XCTAssertEqual(payload.contextComponents.first?.kind, .skill)
    }

    func testCannotFitBlocksOutboundInsteadOfFallingBackToLatestMessage() {
        let prepared = AgentCoreContextPrepareResponse(
            action: "cannot_fit",
            messages: [AgentCoreContextPrepareMessage(id: "latest", role: "user", content: "must not send alone")],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 120,
                toolSchemaTokens: 0,
                totalPromptTokens: 120,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 120,
                beforeTokens: 120,
                afterTokens: 120
            ),
            archiveRequest: nil
        )

        XCTAssertThrowsError(try DefaultContextManager.outboundPromptSegments(from: prepared)) { error in
            XCTAssertEqual(error as? ContextOutboundPreparationError, .cannotFit)
        }
    }

    func testSummaryRequiredCannotEscapePreflight() {
        let prepared = AgentCoreContextPrepareResponse(
            action: "summary_required",
            messages: [],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 90,
                toolSchemaTokens: 0,
                totalPromptTokens: 90,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 90,
                beforeTokens: 90,
                afterTokens: 90
            ),
            archiveRequest: nil
        )

        XCTAssertThrowsError(try DefaultContextManager.outboundPromptSegments(from: prepared)) { error in
            XCTAssertEqual(error as? ContextOutboundPreparationError, .summaryStillRequired)
        }
    }
}

final class IOSSharedProtocolTests: XCTestCase {
    func testHostSetupUsesStrictVersionAndSafeOrigin() throws {
        let valid = #"{"type":"nola.server.connection","protocol_version":1,"origin":"http://192.168.1.8:5001","host_id":"host-fixture","access_token":"fixture-token"}"#
        XCTAssertEqual(AppleHostConnectionSetup.decode(valid)?.hostID, "host-fixture")
        for invalid in [valid.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":true"#),
                        valid.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":2"#),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "user:pass@192.168.1.8:5001"),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "8.8.8.8:5001"),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "192.168.1.8:5001?token=secret")] {
            XCTAssertNil(AppleHostConnectionSetup.decode(invalid))
        }
        XCTAssertTrue(AppleHostConnectionSetup.sameOrigin(URL(string: "https://Studio.local")!, URL(string: "https://studio.local:443/api/mobile")!))
        XCTAssertFalse(AppleHostConnectionSetup.sameOrigin(URL(string: "http://one.local")!, URL(string: "http://two.local")!))
    }

    func testCancellationCarriesExactAdmissionOwnerWithoutClaimingTerminal() throws {
        let identity = AppleRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let object = try AppleRemoteCancellationRequest(identity: identity, requestDigest: "digest").object()
        XCTAssertEqual(Set(object.keys), ["identity", "request_digest"])
        XCTAssertEqual((object["identity"] as? [String: Any])?["submission_id"] as? String, "run")
        XCTAssertThrowsError(try AppleRemoteCancellationRequest(identity: identity, requestDigest: ""))
    }

    func testPermissionWriteCarriesExplicitInactiveOwnerAndRevision() throws {
        let command = SetConversationPermissionCommand(conversationID: "conversation", expectedRevision: 7,
            expectedRunID: nil, expectedSourceHostID: nil, profile: "cautious", updatedAtMs: 100)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(command)) as? [String: Any])
        XCTAssertEqual(object["type"] as? String, "set_conversation_permission")
        XCTAssertEqual(object["expected_revision"] as? Int, 7)
        XCTAssertTrue(object["expected_run_id"] is NSNull)
        XCTAssertTrue(object["expected_source_host_id"] is NSNull)
    }

    func testExecutionIntentSeparatesInheritanceFromExecutionLocation() throws {
        for selection in [SelectConversationExecutionCommand<AgentCoreJSONValue>.Selection.inherit, .location(executionHost: "remote")] {
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
            XCTAssertNil(object["modelSelectionID"])
            XCTAssertNil(object["executionHostBinding"])
            if object["kind"] as? String == "location" {
                XCTAssertEqual(object["execution_host"] as? String, "remote")
            } else { XCTAssertEqual(object["kind"] as? String, "inherit") }
        }
    }

    func testSharedResultEvidenceRejectsOverlappingCancelledEffects() throws {
        let wire = Data(#"{"kind":"cancelled","cancellation_id":"cancel-1","observed_effect_refs":["effect-1"],"unknown_effect_refs":["effect-1"]}"#.utf8)
        let value = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: wire)
        XCTAssertFalse(value.hasRequiredEvidenceFields)
    }

    func testSharedResultEvidencePreservesAnsweredVariant() throws {
        let wire = Data(#"{"kind":"answered","answer_part_refs":["part-1"]}"#.utf8)
        let value = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: wire)
        XCTAssertTrue(value.hasRequiredEvidenceFields)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value)),
                       try JSONDecoder().decode(AgentCoreJSONValue.self, from: wire))
    }

    func testSharedJournalReadUsesDurableCursorAndBoundedPage() throws {
        let identity = AppleAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        let cursor = AppleAssistantJournalCursor(identity: identity, sequence: "9007199254740993", record_digest: "digest")
        let request = AppleAssistantJournalReadRequest<AgentCoreJSONValue>.records(after: cursor, through: cursor)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(object["operation"] as? String, "read")
        XCTAssertEqual(object["max_records"] as? Int, 32)
        XCTAssertEqual(object["max_encoded_bytes"] as? Int, 256 * 1024)
        XCTAssertEqual((object["after"] as? [String: Any])?["sequence"] as? String, "9007199254740993")
    }

    func testSharedDiscoveryUsesProviderAuthenticationAndCatalogOnly() throws {
        let request = try AppleModelCatalogDiscovery.request(endpoint: "https://example.test/v1/", wireProtocol: "anthropic", secret: "test-secret")
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.httpMethod, "GET")
        XCTAssertEqual(request.value(forHTTPHeaderField: "x-api-key"), "test-secret")
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
        let url = try XCTUnwrap(request.url)
        let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
        let models = try AppleModelCatalogDiscovery.parse(data: Data(#"{"data":[{"id":"a","display_name":"A"},{"id":"b"},{"id":""}]}"#.utf8), response: response)
        XCTAssertEqual(models.map(\.id), ["a", "b"])
        XCTAssertEqual(models.map(\.displayName), ["A", "b"])
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: Data(#"{"models":[]}"#.utf8), response: response))
    }

    @MainActor func testCatalogReadConsumesBoundedChunksWithoutBlockingThePicker() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [IOSCatalogFixtureProtocol.self]
        for path in ["oversized", "declared", "redirect"] {
            let started = ContinuousClock.now
            do {
                _ = try await AppleModelCatalogTransport.fetch(
                    URLRequest(url: URL(string: "https://catalog.invalid/" + path)!), configuration: configuration)
                XCTFail("Expected catalog rejection: " + path)
            } catch let error as URLError {
                XCTAssertEqual(error.code, path == "redirect" ? .badServerResponse : .dataLengthExceedsMaximum)
            }
            XCTAssertLessThan(ContinuousClock.now - started, .seconds(5))
        }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testDiscoveredReasoningMetadataSurvivesCoreCatalogSerialization() throws {
        let url = try XCTUnwrap(URL(string: "https://models.example/v1/models"))
        let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
        for levels in [#""supported_reasoning_levels":[{"effort":"none"},{"effort":"low"},{"effort":"high"},{"effort":"max"},{"effort":"high"}]"#,
                       #""reasoning_levels":["none","low","high","max"]"#] {
            let body = "{\"data\":[{\"id\":\"fixture\",\"display_name\":\"Readable\",\"context_window\":128000,\"max_output_tokens\":32000," + levels + "}]}"
            let model = try XCTUnwrap(AppleModelCatalogDiscovery.parse(data: Data(body.utf8), response: response).first)
            XCTAssertEqual(model.reasoningLevels, ["none", "low", "high", "max"])
            let entry = try XCTUnwrap(JSONSerialization.jsonObject(with: model.catalogData(wireProtocol: "chat_completions")) as? [String: Any])
            XCTAssertEqual(entry["reasoning_levels"] as? [String], model.reasoningLevels)
            XCTAssertEqual(entry["context_window"] as? Int, 128000)
            XCTAssertEqual((entry["execution_capabilities"] as? [String: Any])?["reasoning"] as? Bool, true)
        }
        let body = #"{"data":[{"id":"gpt-looking-name"},{"id":"empty","supported_reasoning_levels":[],"reasoning_levels":["high"]}]}"#
        let models = try AppleModelCatalogDiscovery.parse(data: Data(body.utf8), response: response)
        XCTAssertTrue(models.allSatisfy { $0.reasoningLevels.isEmpty })
        XCTAssertTrue(models.allSatisfy { $0.reasoningSupported == nil })
    }

    @MainActor func testDiscoveredModelCanUpdateAnExistingManualConnectionWithoutLosingEfforts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("catalog-efforts-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("isolated.sqlite")
        let application = try IOSApplicationRuntime(databaseURL: database)
        let service = IOSModelConnectionService(application: application)
        let initial = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "fixture-model")
        let discovered = AppleModelCatalogDiscovery.Model(id: "fixture-model", displayName: "Discovered", reasoningLevels: ["low", "high"])
        let refreshed = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "fixture-model", discoveredModel: discovered)
        XCTAssertEqual(refreshed.id, initial.id)
        XCTAssertGreaterThan(refreshed.revision, initial.revision)
        XCTAssertEqual(refreshed.document.catalog?.models.first?.reasoning_levels, ["low", "high"])
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let profile = try XCTUnwrap(reopened.listModelProfiles().first { $0.modelName == "fixture-model" })
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        XCTAssertEqual((source["reasoning"] as? [String: Any])?["efforts"] as? [String], ["low", "high"])
    }

    @MainActor func testObservedAnthropicEffortsKeepExactDeclaredLevels() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("anthropic-catalog-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("isolated.sqlite"))
        let service = IOSModelConnectionService(application: application)
        let model = AppleModelCatalogDiscovery.Model(id: "adaptive-observed", displayName: "Fixture", reasoningLevels: ["low", "high"])
        let saved = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "anthropic", secret: "", modelID: model.id, discoveredModel: model)
        XCTAssertEqual(saved.document.catalog?.models.first?.reasoning_levels, ["low", "high"])
        let profile = try XCTUnwrap(application.listModelProfiles().first { $0.modelName == model.id })
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        XCTAssertEqual((source["reasoning"] as? [String: Any])?["efforts"] as? [String], ["low", "high"])
    }

}


private nonisolated final class IOSCatalogFixtureProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "catalog.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let path = request.url!.lastPathComponent
        let response = HTTPURLResponse(url: request.url!, statusCode: path == "redirect" ? 302 : 200,
            httpVersion: "HTTP/1.1", headerFields: path == "declared" ? ["Content-Length": "2097153"] : [:])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: path == "oversized" ? Data(repeating: 32, count: 2097153) : Data("{\"data\":[]}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}

}

@MainActor
struct LLMAgentCapabilitiesTests {
    @Test
    func capabilitiesHonorFrozenConfigCacheRulesAndUnsupportedAdapters() throws {
        let responsesProvider = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://fixture.invalid/v1")),
            apiKey: "fixture", model: "model", wireProtocol: .responses, maxCompletionTokens: 64))
        #expect(responsesProvider.supportsNativeEventStream)
        #expect(responsesProvider.agentCapabilities == .init(maxOutputTokens: 64, streamText: true, streamReasoning: true))

        let transport = AgentModelInvocationPlan(reasoning: false, streamReasoning: true)
        #expect(!transport.reasoning && transport.streamReasoning)
        let declared = LLMAgentCapabilities(reasoning: false, streamReasoning: true)
        #expect(!declared.reasoning && declared.streamReasoning && !declared.reasoningContinuity)

        var config = OpenAIProvider.Config(baseURL: try #require(URL(string: "https://fixture.invalid/v1")),
            apiKey: "fixture", model: "model", executionCapabilities: .init(toolCalls: true, imageInput: true, reasoning: true),
            maxCompletionTokens: 64, promptCacheEnabled: true)
        let frozenProvider = OpenAIProvider(config: config)
        config.executionCapabilities = .init(); config.promptCacheEnabled = false; config.maxCompletionTokens = 64000
        #expect(frozenProvider.agentCapabilities == .init(visionInput: true, maxOutputTokens: 64, nativeToolCalls: true, reasoning: true,
            promptCache: true, streamText: true, streamReasoning: true, streamToolCalls: true))
        #expect(!frozenProvider.agentCapabilities.parallelToolCalls && !frozenProvider.agentCapabilities.reasoningContinuity && !frozenProvider.agentCapabilities.requestContinuation)

        for (host, explicit, expected) in [
            ("api.openai.com", Optional<Bool>.none, true),
            ("fixture.invalid", nil, false),
            ("api.openai.com", false, false),
            ("fixture.invalid", true, true),
        ] {
            let p = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://\(host)/v1")),
                apiKey: "fixture", model: "model", promptCacheEnabled: explicit))
            #expect(p.agentCapabilities.promptCache == expected)
        }

        let anthropicProvider = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://fixture.invalid")),
            apiKey: "fixture", model: "model", wireProtocol: .anthropic,
            executionCapabilities: .init(toolCalls: true, reasoning: true), promptCacheEnabled: true))
        #expect(anthropicProvider.agentCapabilities == .init())
        #expect(LLMAgentCapabilities(parallelToolCalls: true, reasoningContinuity: true,
            streamReasoning: true, streamToolCalls: true, requestContinuation: true) == .init(streamReasoning: true))
    }
}

final class IOSMemoryWorkerLLMValidatorAndInteropTests: XCTestCase {
    func testMemoryWorkerSessionSchedulingBudgets() async throws {
        var visited1: [(String, Int)] = []
        let r1 = try await processClaimableMemoryWorkerSessions(conversationIDs: ["first", "second", "third"], maximumJobCount: 2) { id, rem in
            visited1.append((id, rem))
            return id == "third" ? IOSMemoryWorkerRunReport(attempted: 1, completed: 1) : IOSMemoryWorkerRunReport()
        }
        XCTAssertEqual(r1, IOSMemoryWorkerRunReport(attempted: 1, completed: 1))
        XCTAssertEqual(visited1.map(\.0), ["first", "second", "third"])
        XCTAssertEqual(visited1.map(\.1), [2, 2, 2])

        var visited2: [String] = []
        let r2 = try await processClaimableMemoryWorkerSessions(conversationIDs: ["first", "second", "third"], maximumJobCount: 2) { id, rem in
            visited2.append(id)
            return IOSMemoryWorkerRunReport(attempted: rem, completed: rem)
        }
        XCTAssertEqual(r2, IOSMemoryWorkerRunReport(attempted: 2, completed: 2))
        XCTAssertEqual(visited2, ["first"])

        var visited3: [String] = []
        let r3 = try await processClaimableMemoryWorkerSessions(conversationIDs: ["first", "second", "third"], maximumJobCount: 2) { id, _ in
            visited3.append(id)
            return IOSMemoryWorkerRunReport(attempted: 1, retryScheduled: 1)
        }
        XCTAssertEqual(r3, IOSMemoryWorkerRunReport(attempted: 2, retryScheduled: 2))
        XCTAssertEqual(visited3, ["first", "second"])
    }

    func testAgentActionModelsLLMResponseValidatorAndRustInteropDedupe() throws {
        let action1 = Action(type: LLMActionType.notify, priority: LLMActionPriority.medium,
            dedupeKey: "url:example.com:post-123:2025-11-16",
            scheduledAt: ISO8601DateFormatter().date(from: "2025-11-16T08:00:00Z"),
            contentSummary: "摘要示例", reason: "包含工作相关建议", confidence: 0.85)
        let action2 = Action(type: LLMActionType.store, priority: LLMActionPriority.low,
            dedupeKey: "store:url:example.com:post-123",
            contentSummary: "保存文章 URL", reason: "用户可能需要稍后查阅", confidence: 0.6)
        let resp = LLMActionResponse(actions: [action1, action2], summary: "建议发送一条中午提醒并将文章存档。")
        let decoded = try JSONDecoder.nolaDecoder().decode(LLMActionResponse.self, from: JSONEncoder.nolaEncoder().encode(resp))
        XCTAssertEqual(decoded.actions.count, 2)
        XCTAssertEqual(decoded.summary, resp.summary)
        XCTAssertEqual(decoded.actions[0].dedupeKey, action1.dedupeKey)

        let fenced = "Here is the result:\n```json {\n  \"actions\": [ { \"type\": \"notify\", \"priority\": \"medium\", \"dedupeKey\": \"k1\", \"reason\": \"r\" } ] } ```\nThanks."
        let extracted = LLMResponseValidator.extractJSONSubstring(from: fenced)
        XCTAssertEqual(extracted?.contains("actions"), true)

        let validJSON = "{ \"actions\": [ { \"type\": \"notify\", \"priority\": \"medium\", \"dedupeKey\": \"url:ex:1\", \"scheduledAt\": null, \"contentSummary\": \"摘要\", \"reason\": \"测试\", \"confidence\": 0.9 } ], \"summary\": \"ok\" }"
        let validated = try LLMResponseValidator.validateAndDecode("Some preface text\n\(validJSON)\nfooter")
        XCTAssertEqual(validated.actions.count, 1)
        XCTAssertEqual(validated.summary, "ok")

        XCTAssertThrowsError(try LLMResponseValidator.validateAndDecode("{ \"note\": \"nothing here\" }"))
        XCTAssertTrue(LLMResponseValidator.userFriendlyMessage(for: LLMValidationError.noJSONFound).contains("未能从模型响应中解析到"))
        LLMResponseValidator.recordAudit(candidateId: "cand-1", modelId: "local:test", inputPreview: "preview text", extractedJSON: "{...}", decodedSummary: "ok", error: nil)

        let key = RustInterop.generateDedupeKey(source: "calendar", title: "Test", dateISO: nil, sourceId: nil)
        XCTAssertNotNil(key)
        XCTAssertFalse(key?.isEmpty ?? true)
    }
}
