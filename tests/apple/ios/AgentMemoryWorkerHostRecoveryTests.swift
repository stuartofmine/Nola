// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies Core -> Application Store -> canonical iOS Memory Worker recovery.

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
