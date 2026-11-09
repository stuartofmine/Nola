// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

/// Deterministic model execution still freezes a real v7 invocation and obtains
/// its output admission from the isolated Application, never from a fake receipt.
@MainActor
func makeIOSDeterministicHostInvocation(store: IOSApplicationRuntime, model: String,
    outputTokens: Int, capabilities: AgentCoreJSONValue? = nil) throws -> IOSPreparedHostInvocation {
    let configuration = OpenAIProvider.Config(baseURL: URL(string: "https://ios-fixture.invalid/v1")!,
        apiKey: "fixture-only", model: model, maxCompletionTokens: outputTokens)
    let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: configuration, provider: "fixture")
    let output = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: outputTokens, using: store)
    var declared: [String: AgentCoreJSONValue] = [:]
    if case .object(let values)? = capabilities { declared = values }
    let generation: AgentCoreJSONValue = .object([
        "temperature": .number(0), "top_p": .number(1), "max_output_tokens": .number(Double(outputTokens)),
        "max_retries": .number(0), "parallel_tool_calls": declared["parallel_tool_calls"] ?? .bool(false),
        "reasoning_selection": .null, "sampling_override": .bool(false), "generation_options": .object([:]),
        "reasoning_tier": .string("balanced"),
        "thinking_enabled": declared["reasoning"] ?? .bool(false), "prompt_cache": declared["prompt_cache"] ?? .bool(false),
        "prompt_cache_key_sha256": .null, "prompt_cache_key_prefix_sha256": .null, "prompt_cache_retention": .null,
    ])
    return IOSPreparedHostInvocation(route: route, generation: generation, outputAdmission: output)
}

final class IOSCanonicalAgentApplicationContractTests: XCTestCase {
    func testTaskCancellationUsesCoreCurrentFenceAndOnlyReportsRequested() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = CommandStoreStub(responses: [.object(["kind": .string("applied"), "cancel_requested": .bool(true)])])
        let app = IOSCanonicalAgentApplication(store: store, descriptor: .init(conversationID: "parent", conversationTitle: "",
            identity: identity, userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "run"))
        let current: AgentCoreJSONValue = .object(["lease_token": .string("current")])
        let request: AgentCoreJSONValue = .object(["effect_id": .string("effect"), "request_id": .string("request"),
            "delivery": .object(["lease_token": .string("old")]),
            "payload": .object(["type": .string("cancel_task"), "task_id": .string("child")])])
        let response = try app.taskCancellationResponse(request: request, delivery: current, nowMs: 42)
        XCTAssertEqual(store.commands.first?.cancellationObject?["type"], .string("execute_task_cancellation_for_effect"))
        XCTAssertEqual(store.commands.first?.cancellationObject?["request"]?.cancellationObject?["delivery"], current)
        XCTAssertEqual(response.cancellationObject?["type"], .string("completed"))
        XCTAssertEqual(response.cancellationObject?["output"]?.cancellationObject?["cancel_requested"], .bool(true))
        XCTAssertNil(response.cancellationObject?["state"])
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(response), as: UTF8.self).contains("cancelled"))
    }

    func testTaskCancellationTransferIsUnsupportedAndMalformedAppliedIsNotSuccess() throws {
        let request: AgentCoreJSONValue = .object(["effect_id": .string("effect"),
            "payload": .object(["type": .string("cancel_task"), "task_id": .string("child")])])
        for result in [AgentCoreJSONValue.object(["kind": .string("transfer"), "authorization": .object([:])]),
                       .object(["kind": .string("applied"), "cancel_requested": .string("false")])] {
            let store = CommandStoreStub(responses: [result])
            let app = IOSCanonicalAgentApplication(store: store, descriptor: .init(conversationID: "parent", conversationTitle: "",
                identity: .init(userMessageID: UUID()), userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "run"))
            if result.cancellationObject?["kind"] == .string("transfer") {
                let response = try app.taskCancellationResponse(request: request, delivery: .object([:]), nowMs: 42)
                XCTAssertEqual(response.cancellationObject?["type"], .string("failed"))
                XCTAssertEqual(response.cancellationObject?["error"]?.cancellationObject?["code"], .string("task_cancellation_transfer_unsupported"))
            } else { XCTAssertThrowsError(try app.taskCancellationResponse(request: request, delivery: .object([:]), nowMs: 42)) }
        }
    }

    func testLocalGuidanceUsesCurrentAdmissionAndExactReceiptOwner() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = CommandStoreStub(responses: [.null, .null, .null])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "parent", conversationTitle: "", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "run"))
        let intent = AppleTaskGuidanceIntent(hostId: "device", conversationId: "parent",
            parentRunId: "run", taskId: "child", messageId: "fixed-intent",
            submittedAtMs: 42, message: "Continue with the smaller scope")
        let payload = try JSONEncoder().encode(intent)
        _ = try application.taskGuidanceTransport(path: "chat/agent/v2/task/message", payload: payload, parentTurnID: "turn")
        _ = try application.taskGuidanceTransport(path: "chat/agent/v2/task/message/receipt", payload: payload, parentTurnID: "turn")
        XCTAssertEqual(store.commands, [
            .object(["type": .string("load_task_guidance_receipt"), "task_id": .string("child"),
                "message_id": .string("fixed-intent"), "parent_conversation_id": .string("parent"),
                "parent_run_id": .string("run")]),
            .object(["type": .string("submit_task_guidance"), "task_id": .string("child"),
                "message_id": .string("fixed-intent"), "parent_turn_id": .string("turn"),
                "content": .string(intent.message), "submitted_at_ms": .number(42)]),
            .object(["type": .string("load_task_guidance_receipt"), "task_id": .string("child"),
                "message_id": .string("fixed-intent"), "parent_conversation_id": .string("parent"),
                "parent_run_id": .string("run")])])
        let foreign = AppleTaskGuidanceIntent(hostId: "device", conversationId: "parent",
            parentRunId: "foreign", taskId: "child", messageId: "fixed-intent",
            submittedAtMs: 42, message: intent.message)
        XCTAssertThrowsError(try application.taskGuidanceTransport(path: "chat/agent/v2/task/message",
            payload: JSONEncoder().encode(foreign), parentTurnID: "turn"))
        XCTAssertEqual(store.commands.count, 3)
    }

    func testTaskControlRequiresExplicitCoreCapabilities() throws {
        let value = #"{"kind":"task_control","part_id":"p","revision":4,"task_id":"task","state":"running","supports_guidance":false,"supports_cancellation":true}"#
        guard case .taskControl(let control) = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self,
            from: Data(value.utf8)) else { return XCTFail("TaskControl was discarded") }
        XCTAssertFalse(control.supportsGuidance)
        XCTAssertTrue(control.supportsCancellation)
        XCTAssertThrowsError(try JSONDecoder().decode(IOSConversationProjectionV1.Part.self,
            from: Data(value.replacingOccurrences(of: #","supports_guidance":false"#, with: "").utf8)))
    }

    func testQueuedAdmissionUsesCurrentSubmissionContextAndKeepsOriginalCommand() throws {
        let identity = IOSCanonicalAgentTurnIdentity(queuedSubmissionID: "queued-command", userMessageID: "ui-user")
        let store = CommandStoreStub(responses: [
            .object(["execution": admissionExecutionFixture(), "invocation_config_digest": .string("digest"),
                "events": .array([]), "traces": .array([])]),
            .object(["requests": .array([])])
        ])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation", conversationTitle: "", identity: identity,
            userText: "Display summary", userContentJSON: nil, contextCheckpointID: nil,
            expectedConfigurationRevision: 7, expectedParentMessageID: "parent",
            queuedUserAdmission: .init(commandID: "queued-command", leaseOwner: "dispatcher", deliveryAttempt: 1)))
        let submission = try AgentCoreInputSubmission.make(input: "Display summary", messageId: "ui-user",
            submissionId: "queued-command")
        _ = try application.begin(coreConfig: .object([:]), submission: submission, createdAtMs: 123)
        guard case .object(let command)? = store.commands.first,
              case .object(let input)? = command["input"] else { return XCTFail("Missing queued admission") }
        XCTAssertEqual(command["type"], .string("begin_queued_user_turn"))
        XCTAssertEqual(input["command_id"], .string("queued-command"))
        XCTAssertEqual(input["expected_configuration_revision"], .signedInteger(7))
        XCTAssertEqual(input["turn_context_platform"], .string("ios"))
        XCTAssertEqual(input["turn_context_items"], .array([]))
        XCTAssertEqual(input["turn_context_components"], .array([]))
        XCTAssertNil(input["submission"], "Application must consume the already committed typed action")
        XCTAssertNil(input["user_text"])
    }

    func testEmptyControlQueuesNeverClaimOrReplayAgentCommands() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = CommandStoreStub(responses: [.null])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "policy-owner", conversationTitle: "", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        XCTAssertNil(try application.applyPendingPermissionPolicy())
        XCTAssertEqual(store.commands.compactMap { command -> String? in
            guard case .object(let fields) = command, case .string(let type)? = fields["type"] else { return nil }
            return type
        }, ["encode_host_worker_identity", "apply_pending_task_notice", "peek_agent_command"])
        XCTAssertEqual(store.commands.last, .object(["type": .string("peek_agent_command"), "session_id": .string("policy-owner")]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "peek_agent_command", arguments: [:]))
    }

    func testCancellationRetryFreezesCommandAndAdvancesOnlyDeliveryClock() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let claims: AgentCoreJSONValue = .object(["requests": .array([])])
        let accepted: AgentCoreJSONValue = .object(["events": .array([]), "traces": .array([])])
        let underlying = CommandStoreStub(responses: [
            .object(["execution": admissionExecutionFixture(), "invocation_config_digest": .string("digest"),
                     "events": .array([]), "traces": .array([])]), claims,
            .null, accepted, // Current source execution, then the lost cancellation ACK.
            .null, accepted, claims, // Re-observe before retrying the frozen command.
        ])
        let store = CancellationLostAcknowledgementStore(underlying)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "cancel-conversation", conversationTitle: "Cancel", identity: identity,
            userText: "hello", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "bound-run"))
        XCTAssertThrowsError(try application.prepareCancellation(reason: "before admission"))
        XCTAssertTrue(underlying.commands.isEmpty)
        _ = try application.begin(coreConfig: .object([:]), submission: .object([:]), createdAtMs: 1)
        let operation = try application.prepareCancellation(reason: "explicit user cancellation")
        XCTAssertEqual(operation.runID, "bound-run")
        XCTAssertEqual(operation.assistantMessageID, identity.assistantMessageID)
        XCTAssertThrowsError(try application.submitCancellation(reason: "ignored later reason"))
        let retryTime = operation.committedAtMs + 600_000
        _ = try application.submitCancellation(reason: "stream terminated later", observedAtMs: retryTime)
        XCTAssertEqual(try application.prepareCancellation(reason: "again"), operation)
        let cancellations = underlying.commands.filter {
            guard case .object(let command) = $0 else { return false }
            return command["type"] == .string("reduce_and_commit_active_submission")
        }
        XCTAssertEqual(cancellations, [operation.command, operation.command])
        guard let lastCommand = underlying.commands.last, case .object(let last) = lastCommand,
              case .object(let input)? = last["input"] else { return XCTFail("Missing delivery claim") }
        XCTAssertEqual(input["claimed_at_ms"], .number(Double(retryTime)))
        XCTAssertEqual(input["lease_expires_at_ms"], .number(Double(retryTime + 240_000)))
    }

    @MainActor
    func testWindowAdmissionRetriesTransientFailureWithoutLosingOwner() async throws {
        let (cursor, window) = try publicationWindowFixture(message: "assistant")
        var attempts = 0
        var sleeps = 0
        let result = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { true }, readWindow: { _ in
            attempts += 1
            if attempts == 1 { throw IOSApplicationRuntimeError.outcomePending("read-only-query") }
            return window
        }, sleep: { _ in sleeps += 1 })
        XCTAssertEqual(result, window)
        XCTAssertEqual(attempts, 2)
        XCTAssertEqual(sleeps, 1)
        attempts = 0
        let unavailable = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { true }, readWindow: { _ in
            attempts += 1
            throw IOSApplicationRuntimeError.applicationUnavailable
        }, sleep: { _ in })
        XCTAssertNil(unavailable)
        XCTAssertEqual(attempts, 3)
    }

    @MainActor
    func testWindowAdmissionSelectionSwitchNeverAdmitsOldOwner() async throws {
        let (cursor, window) = try publicationWindowFixture(message: "assistant")
        var selected = true
        var attempts = 0
        let result = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { selected }, readWindow: { _ in
            attempts += 1
            throw IOSApplicationRuntimeError.outcomePending("read-only-query")
        }, sleep: { _ in selected = false })
        XCTAssertNil(result)
        XCTAssertEqual(attempts, 1)
        var scope = IOSLocalJournalPublicationScope(conversationID: "conversation", runID: "run")
        XCTAssertTrue(try scope.needsAdmission(cursor))
        selected = true
        let retried = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { selected }, readWindow: { _ in window })
        if retried != nil { try scope.didAdmit(cursor) }
        XCTAssertFalse(try scope.needsAdmission(cursor))
    }

    @MainActor
    func testWindowAdmissionRequiresExactMessageAndRunInBoundedWindow() async throws {
        let (cursor, target) = try publicationWindowFixture(message: "assistant")
        let (_, absent) = try publicationWindowFixture(message: "other")
        let missing = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { true }, readWindow: { _ in absent })
        XCTAssertNil(missing)
        var requests: [Int64?] = []
        let newer = IOSConversationProjectionWindowV1(projection: absent.projection, eventSequence: 1,
            totalMessageCount: 81, beforeOrdinal: 1, hasMore: true, assistantJournalHeads: absent.assistantJournalHeads)
        let found = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { true }, readWindow: { before in
            requests.append(before)
            return before == nil ? newer : target
        })
        XCTAssertEqual(found, target)
        XCTAssertEqual(requests.count, 2)
        XCTAssertEqual(requests[1], 1)
        let foreign = IOSConversationProjectionWindowV1(projection: target.projection, eventSequence: 1,
            totalMessageCount: 1, beforeOrdinal: nil, hasMore: false, assistantJournalHeads: absent.assistantJournalHeads)
        var attempts = 0
        do {
            _ = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: { true }, readWindow: { _ in
                attempts += 1; return foreign
            })
            XCTFail("A foreign journal owner must be rejected")
        } catch { XCTAssertEqual(attempts, 1) }
    }

    @MainActor
    func testPublicationReadRetryHonorsCancellationAndRejectsProtocolDamage() async throws {
        var attempts = 0
        do {
            let _: Int? = try await IOSLocalJournalWindowAdmission.read(isCurrent: { true }, operation: {
                attempts += 1
                throw IOSApplicationRuntimeError.applicationUnavailable
            }, sleep: { _ in throw CancellationError() })
            XCTFail("Cancellation must stop presentation retry")
        } catch is CancellationError { XCTAssertEqual(attempts, 1) }
        attempts = 0
        do {
            let _: Int? = try await IOSLocalJournalWindowAdmission.read(isCurrent: { true }, operation: {
                attempts += 1
                throw IOSApplicationRuntimeError.invalidResponse
            }, sleep: { _ in XCTFail("Protocol damage must not be retried") })
            XCTFail("Invalid protocol must fail closed")
        } catch { XCTAssertEqual(attempts, 1) }
    }

    private func publicationWindowFixture(message: String) throws -> (IOSAssistantJournalCursor, IOSConversationProjectionWindowV1) {
        let cursor = IOSAssistantJournalCursor(identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: message), sequence: "1", record_digest: String(repeating: "a", count: 64))
        let json: [String: Any] = ["contract_version": 1, "execution_ownership": ["kind": "user"],  "conversation_id": "conversation", "revision": 1, "title": "Fixture",
            "messages": [["message_id": message, "role": "assistant", "state": "running", "revision": 1, "created_at_ms": 1, "parts": []]],
            "context_usage": ["used_tokens": 0, "context_window_tokens": 128000, "prompt_token_limit": 120000,
                "estimated": true, "measured_at_revision": 1]]
        let projection = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: JSONSerialization.data(withJSONObject: json))
        return (cursor, IOSConversationProjectionWindowV1(projection: projection, eventSequence: 1, totalMessageCount: 1,
            beforeOrdinal: nil, hasMore: false, assistantJournalHeads: [cursor]))
    }

    func testLocalPublicationScopeAdmitsEachMessageOnceAndRejectsForeignOwners() throws {
        func cursor(conversation: String = "conversation", run: String = "run", epoch: String = "epoch",
                    message: String = "assistant", sequence: String = "1") -> IOSAssistantJournalCursor {
            .init(identity: .init(source_store_epoch: epoch, conversation_id: conversation, run_id: run,
                assistant_message_id: message), sequence: sequence, record_digest: String(repeating: "a", count: 64))
        }
        var scope = IOSLocalJournalPublicationScope(conversationID: "conversation", runID: "run")
        XCTAssertTrue(try scope.needsAdmission(cursor()))
        try scope.didAdmit(cursor())
        XCTAssertFalse(try scope.needsAdmission(cursor(sequence: "2")))
        XCTAssertTrue(try scope.needsAdmission(cursor(message: "second-message")))
        try scope.didAdmit(cursor(message: "second-message"))
        XCTAssertFalse(try scope.needsAdmission(cursor(message: "second-message", sequence: "3")))
        for foreign in [cursor(conversation: "foreign"), cursor(run: "foreign"), cursor(epoch: "foreign"), cursor(epoch: "")] {
            XCTAssertThrowsError(try scope.needsAdmission(foreign))
        }
    }

    func testCoordinatorConsumesOnlyControlWithMatchingSeparatePermissions() throws {
        guard case .object(let execution) = admissionExecutionFixture() else { return XCTFail("Invalid fixture") }
        let valid: [String: AgentCoreJSONValue] = [
            "control": execution["control"]!, "permission_projection": execution["permissions"]!,
        ]
        guard case .object(let control)? = try DeviceAgentCoordinator.canonicalControlProjection(valid) else {
            return XCTFail("Missing control")
        }
        XCTAssertEqual(control["permissions"], execution["permissions"])
        XCTAssertNil(try DeviceAgentCoordinator.canonicalControlProjection(["host_requests": .array([])]))
        for key in ["snapshot", "checkpoint"] {
            var legacy = valid
            legacy[key] = .null
            XCTAssertThrowsError(try DeviceAgentCoordinator.canonicalControlProjection(legacy))
        }
        for key in ["control", "permission_projection"] {
            var missing = valid
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try DeviceAgentCoordinator.canonicalControlProjection(missing))
        }
        var mismatched = valid
        mismatched["permission_projection"] = .object(["revision": .unsignedInteger(1), "pending": .array([])])
        XCTAssertThrowsError(try DeviceAgentCoordinator.canonicalControlProjection(mismatched))
    }

    func testControlQueriesOnlyChangedPermissionRevisionAndRejectsForeignEmptyProjection() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let cursor = UInt64.max - 10
        func control(_ revision: UInt64, _ sequence: UInt64) -> AgentCoreJSONValue {
            .object(["control": .object(["session_id": .string("core"), "active_turn_id": .string("turn"),
                "last_event_cursor": .object(["sequence": .unsignedInteger(sequence)]),
                "permission_projection_revision": .unsignedInteger(revision), "turn_status": .string("awaiting_model")]),
                "events": .array([]), "traces": .array([])])
        }
        func permission(_ revision: UInt64, run: String) -> AgentCoreJSONValue {
            .object(["conversation_id": .string("control-conversation"), "run_id": .string(run),
                "session_id": .string("core"), "active_turn_id": .string("turn"),
                "event_cursor": .unsignedInteger(cursor + 2), "permission_projection_revision": .unsignedInteger(revision),
                "permissions": .object(["revision": .unsignedInteger(revision), "pending": .array([])])])
        }
        let claims: AgentCoreJSONValue = .object(["requests": .array([])])
        let store = CommandStoreStub(responses: [
            .object(["execution": admissionExecutionFixture(), "invocation_config_digest": .string("digest"), "events": .array([]), "traces": .array([])]), claims,
            control(1,cursor), permission(1,run: identity.submissionID), claims,
            control(1,cursor + 1), claims,
            control(2,cursor + 2), permission(2,run: "foreign-run"),
        ])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "control-conversation", conversationTitle: "control", identity: identity,
            userText: "hello", userContentJSON: nil, contextCheckpointID: nil))
        _ = try application.begin(coreConfig: .object([:]), submission: .object([:]), createdAtMs: 1)
        for time in 2...3 {
            guard case .object(let transition) = try application.acceptHostResponse(.object([:]), receivedAtMs: Int64(time)) else {
                return XCTFail("Missing control transition")
            }
            XCTAssertNil(transition["snapshot"])
            XCTAssertNotNil(transition["control"])
            XCTAssertNotNil(transition["permission_projection"])
        }
        XCTAssertThrowsError(try application.acceptHostResponse(.object([:]), receivedAtMs: 4))
        let queries = store.commands.compactMap { value -> [String: AgentCoreJSONValue]? in
            guard case .object(let command) = value, command["type"] == .string("load_conversation_permission_projection_v1") else { return nil }
            return command
        }
        XCTAssertEqual(queries.count,2)
        let claimsIssued = store.commands.filter { value in
            guard case .object(let command) = value else { return false }
            return command["type"] == .string("claim_committed_host_requests")
        }
        XCTAssertEqual(claimsIssued.count, 3, "Foreign permission ownership must fail before claiming successor work")
        let claimTimes = claimsIssued.compactMap { value -> AgentCoreJSONValue? in
            guard case .object(let command) = value, case .object(let input)? = command["input"] else { return nil }
            return input["claimed_at_ms"]
        }
        XCTAssertEqual(claimTimes, [.number(1), .number(2), .number(3)],
            "Successor delivery claims must use the accepted operation clock, not a fresh wall clock")
        guard case .object(let scope)? = queries.first?["scope"] else { return XCTFail("Missing exact scope") }
        XCTAssertEqual(scope["event_cursor"], .unsignedInteger(cursor))
        XCTAssertEqual(scope["active_turn_id"], .string("turn"))
    }

    func testClaimedPermissionHasDeliveryWithoutModelAttempt() throws {
        let request = PermissionRecoveryCommandStoreStub.request(sessionID: "session",
            workerGeneration: "worker", leaseOwner: "owner", leaseExpiry: 2_000,
            payloadType: "request_permission")
        try IOSCanonicalAgentApplication.validateClaimedHostRequests([request],
            workerGeneration: "worker", leaseOwner: "owner", claimedAtMs: 1_000)
        try IOSCanonicalAgentApplication.validateRecoveredPermissionRequests([request],
            runID: "run", sessionID: "session", workerGeneration: "worker",
            leaseOwner: "owner", claimedAtMs: 1_000)
    }

    func testModelAttemptAndPhysicalDeliveryIdentityRemainIndependent() throws {
        guard case .object(var request) = PermissionRecoveryCommandStoreStub.request(sessionID: "session",
            workerGeneration: "worker", leaseOwner: "owner", leaseExpiry: 2_000,
            payloadType: "model_stream") else { return XCTFail("Invalid fixture") }
        request["attempt_id"] = .string("distinct-model-attempt")
        try IOSCanonicalAgentApplication.validateClaimedHostRequests([.object(request)],
            workerGeneration: "worker", leaseOwner: "owner", claimedAtMs: 1_000)
        request["payload"] = .object(["type": .string("request_permission")])
        try IOSCanonicalAgentApplication.validateRecoveredPermissionRequests([.object(request)],
            runID: "run", sessionID: "session", workerGeneration: "worker",
            leaseOwner: "owner", claimedAtMs: 1_000)
    }

    func testClaimedDeliveryRejectsMissingForeignAndExpiredLease() throws {
        guard case .object(let valid) = PermissionRecoveryCommandStoreStub.request(sessionID: "session",
            workerGeneration: "worker", leaseOwner: "owner", leaseExpiry: 2_000,
            payloadType: "request_permission"), case .object(let lease)? = valid["delivery"] else {
            return XCTFail("Invalid fixture")
        }
        var missing = valid
        missing.removeValue(forKey: "delivery")
        var invalidRequests = [AgentCoreJSONValue.object(missing)]
        for (key, value): (String, AgentCoreJSONValue) in [
            ("attempt_id", .string("")), ("worker_generation", .string("foreign")),
            ("lease_owner", .string("foreign")), ("lease_token", .string("")),
            ("lease_expires_at_ms", .signedInteger(1_000))
        ] {
            var modifiedLease = lease
            modifiedLease[key] = value
            var envelope = valid
            envelope["delivery"] = .object(modifiedLease)
            invalidRequests.append(.object(envelope))
        }
        for request in invalidRequests {
            XCTAssertThrowsError(try IOSCanonicalAgentApplication.validateClaimedHostRequests([request],
                workerGeneration: "worker", leaseOwner: "owner", claimedAtMs: 1_000))
        }
    }

    func testRecoveredLeaseIntegerAdmissionDoesNotRoundOrTrapAtInt64Boundary() throws {
        func validate(_ expiry: AgentCoreJSONValue) throws {
            let request: AgentCoreJSONValue = .object([
                "session_id": .string("session"),
                "payload": .object(["type": .string("request_permission")]),
                "delivery": .object([
                    "attempt_id": .string("attempt"), "worker_generation": .string("worker"),
                    "lease_owner": .string("owner"), "lease_token": .string("token"),
                    "lease_expires_at_ms": expiry,
                ]),
            ])
            try IOSCanonicalAgentApplication.validateRecoveredPermissionRequests([request],
                runID: "run", sessionID: "session", workerGeneration: "worker",
                leaseOwner: "owner", claimedAtMs: 1_000)
        }
        for valid: AgentCoreJSONValue in [.number(2_000), .signedInteger(Int64.max), .unsignedInteger(UInt64(Int64.max))] {
            XCTAssertNoThrow(try validate(valid))
        }
        for invalid: AgentCoreJSONValue in [
            .number(Double(Int64.max)), .number(.infinity), .number(.nan), .number(2_000.5),
            .unsignedInteger(UInt64.max), .signedInteger(Int64.min), .string("2000"), .null,
        ] {
            XCTAssertThrowsError(try validate(invalid))
        }
    }

    func testDeltaOnlyHostResponseForwardsEventsWithoutInventingOrClearingSnapshot() throws {
        let event: AgentCoreJSONValue = .object(["sequence": .number(7), "delta": .string("hello")])
        let trace: AgentCoreJSONValue = .object(["kind": .string("model_delta")])
        let successor: AgentCoreJSONValue = .object(["attempt_id": .string("model-attempt-2")])
        let store = CommandStoreStub(responses: [
            .object(["execution": admissionExecutionFixture(), "events": .array([]), "traces": .array([]), "invocation_config_digest": .string("digest")]),
            .object(["requests": .array([])]),
            .object(["events": .array([event]), "traces": .array([trace])]),
            .object(["requests": .array([successor])]),
            .object(["control": .null, "events": .array([]), "traces": .array([])]),
            .object(["requests": .array([])]),
        ])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "delta-contract", conversationTitle: "Delta", identity: .init(userMessageID: UUID()),
            userText: "hello", userContentJSON: nil, contextCheckpointID: nil
        ))
        _ = try application.begin(coreConfig: .object([:]), submission: .object([:]), createdAtMs: 1)
        guard case .object(let delta) = try application.acceptHostResponse(.object([:]), receivedAtMs: 2) else {
            return XCTFail("Missing delta transition")
        }
        XCTAssertNil(delta["snapshot"])
        XCTAssertEqual(delta["event_envelopes"], .array([event]))
        XCTAssertEqual(delta["trace_events"], .array([trace]))
        XCTAssertEqual(delta["host_requests"], .array(store.lastClaimedRequests))
        guard case .object(let claimed)? = store.lastClaimedRequests.first else {
            return XCTFail("Missing claimed successor")
        }
        XCTAssertEqual(claimed["attempt_id"], .string("model-attempt-2"))
        guard case .object(let receipt) = try application.acceptHostResponse(.object([:]), receivedAtMs: 3) else {
            return XCTFail("Missing receipt transition")
        }
        XCTAssertNil(receipt["snapshot"])
        XCTAssertEqual(receipt["host_requests"], .array([]))
        XCTAssertEqual(store.commands.count, 7, "One current worker identity query plus six delivery commands; deltas must not fetch a replacement snapshot")
    }

    func testBeginRequiresExecutionStateBeforeClaimingHostWork() {
        let store = CommandStoreStub(responses: [
            .object(["events": .array([]), "traces": .array([]), "invocation_config_digest": .string("digest")]),
            .object(["requests": .array([])]),
        ])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "invalid-begin", conversationTitle: "Invalid", identity: .init(userMessageID: UUID()),
            userText: "hello", userContentJSON: nil, contextCheckpointID: nil
        ))
        XCTAssertThrowsError(try application.begin(coreConfig: .object([:]), submission: .object([:]), createdAtMs: 1))
        XCTAssertEqual(store.commands.count, 1, "Invalid admission must not claim Host deliveries")
    }

    func testMalformedAdmissionCannotClaimHostDeliveries() {
        guard case .object(let execution) = admissionExecutionFixture(),
              case .object(let control)? = execution["control"],
              case .object(let permissions)? = execution["permissions"] else {
            return XCTFail("Invalid fixture")
        }
        let valid: [String: AgentCoreJSONValue] = [
            "execution": .object(execution), "invocation_config_digest": .string("digest"),
            "events": .array([]), "traces": .array([]),
        ]
        var invalid: [[String: AgentCoreJSONValue]] = []
        for key in ["snapshot", "checkpoint"] {
            var reply = valid
            reply[key] = .null
            invalid.append(reply)
        }
        for key in ["events", "traces"] {
            var reply = valid
            reply.removeValue(forKey: key)
            invalid.append(reply)
        }
        for revision: AgentCoreJSONValue? in [nil, .null, .unsignedInteger(1)] {
            var changedControl = control
            var changedPermissions = permissions
            changedControl["permission_projection_revision"] = revision
            // Equal missing/null values must not masquerade as a valid witness.
            if revision != .unsignedInteger(1) { changedPermissions["revision"] = revision }
            var changedExecution = execution
            changedExecution["control"] = .object(changedControl)
            changedExecution["permissions"] = .object(changedPermissions)
            var reply = valid
            reply["execution"] = .object(changedExecution)
            invalid.append(reply)
        }
        for reply in invalid {
            let store = CommandStoreStub(responses: [.object(reply)])
            let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
                conversationID: "invalid-begin", conversationTitle: "Invalid",
                identity: .init(userMessageID: UUID()), userText: "hello",
                userContentJSON: nil, contextCheckpointID: nil))
            XCTAssertThrowsError(try application.begin(coreConfig: .object([:]), submission: .object([:]), createdAtMs: 1))
            XCTAssertEqual(store.commands.count, 1, "Invalid admission must not claim Host deliveries")
        }
    }

    func testOnlyTerminalHostFactsReleasePendingDelivery() {
        for type in ["accepted", "execution_started", "model_attempt_started", "model_delta", "model_committed_delta", "model_tool_call_delta", "model_continuation", "tool_progress", "tool_call_progress", "unknown"] {
            XCTAssertFalse(DeviceAgentCoordinator.isTerminalHostResponse(.object(["type": .string(type)])), type)
        }
        for type in ["model_completed", "tool_completed", "permission_resolved", "completed", "failed", "cancellation_confirmed", "task_promoted"] {
            XCTAssertTrue(DeviceAgentCoordinator.isTerminalHostResponse(.object(["type": .string(type)])), type)
        }
        for state in ["queued", "running", "waiting", "suspended", "retry_waiting", "completed", "failed", "cancelled"] {
            XCTAssertEqual(DeviceAgentCoordinator.isTerminalHostResponse(.object([
                "type": .string("task_updated"), "state": .string(state),
            ])), ["completed", "failed", "cancelled"].contains(state))
        }
        for event in ["text_delta", "tool_started", "tool_progress", "completed", "failed", "cancellation_confirmed"] {
            XCTAssertEqual(DeviceAgentCoordinator.isTerminalHostResponse(.object([
                "type": .string("delegated_agent_event"), "event": .object(["type": .string(event)]),
            ])), ["completed", "failed", "cancellation_confirmed"].contains(event))
        }
    }

    func testColdRecoveryCanPrepareExactCancellationBeforeOldLeaseExpires() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID,
            sessionID: "conversation-1", waitForLease: true)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverExactRun(prepareRecovery: { _ in },
                nowMilliseconds: { 1_000 }, sleep: { _ in
                    let cancellation = try application.prepareCancellation(reason: "stop during recovery")
                    XCTAssertEqual(cancellation.runID, identity.submissionID)
                    XCTAssertEqual(cancellation.assistantMessageID, identity.assistantMessageID)
                    XCTAssertEqual(cancellation.invocationConfigDigest, String(repeating: "a", count: 64))
                    throw CancellationError()
                })
            XCTFail("The interrupted recovery must not claim successor work")
        } catch is CancellationError { }
        let commandTypes = store.commands.compactMap { value -> String? in
            guard case .object(let command) = value else { return nil }
            return jsonString(command, key: "type")
        }
        let preparationIndex = try XCTUnwrap(commandTypes.firstIndex(of: "prepare_run_effect_recovery"))
        let scheduleIndex = try XCTUnwrap(commandTypes.firstIndex(of: "load_runtime_effect_work_schedule"))
        XCTAssertLessThan(preparationIndex, scheduleIndex)
        XCTAssertTrue(commandTypes.contains("inspect_run_process_owners"))
        XCTAssertTrue(commandTypes.contains("validate_run_process_observations"))
        XCTAssertFalse(commandTypes.contains("recover_observed_orphaned_run"), "An unobservable owner is never proof of death")
        XCTAssertFalse(store.commands.contains { value in
            guard case .object(let command) = value else { return false }
            return jsonString(command, key: "type") == "claim_committed_host_requests"
        })
    }

    func testUnknownPermissionWithoutWakeRequiresDecisionWithoutClaimOrReplay() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID,
            sessionID: "conversation-1", requiresRecoveryDecision: true)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverExactRun(prepareRecovery: { _ in },
                nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("Unknown outcome is a decision, not a guessed timer") })
            XCTFail("Unknown permission delivery must not be claimed as runnable work")
        } catch let error as IOSCanonicalAgentApplicationError {
            XCTAssertEqual(error, .recoveryDecisionRequired)
        }
        let types = store.commands.compactMap { value -> String? in
            guard case .object(let command) = value else { return nil }
            return jsonString(command, key: "type")
        }
        XCTAssertTrue(types.contains("inspect_unknown_effect_recovery"))
        XCTAssertFalse(types.contains("claim_committed_host_requests"))
        XCTAssertFalse(types.contains("recover_observed_orphaned_run"))
        XCTAssertFalse(types.contains("resolve_unknown_effect_recovery"))
    }

    func testColdPermissionRecoveryClaimsFreshFencedDeliveryAndRestoresDigest() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(
            userMessageID: UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        )
        let store = PermissionRecoveryCommandStoreStub(
            runID: identity.submissionID,
            sessionID: "conversation-1",
            coreSessionID: "independent-core-session"
        )
        let application = IOSCanonicalAgentApplication(
            store: store,
            descriptor: .init(
                conversationID: "conversation-1",
                conversationTitle: "Recovery",
                identity: identity,
                userText: "",
                userContentJSON: nil,
                contextCheckpointID: nil
            )
        )

        let recovered = try await application.recoverPendingPermissionWait(
            prepareRecovery: { _ in },
            nowMilliseconds: { 1_000 },
            sleep: { _ in XCTFail("A ready permission delivery must not sleep") }
        )
        guard case .object(let recoveredObject) = recovered,
              case .array(let requests)? = recoveredObject["host_requests"] else {
            return XCTFail("Recovery did not return the claimed permission request")
        }
        XCTAssertEqual(requests.count, 1)
        guard case .object(let control)? = recoveredObject["control"] else {
            return XCTFail("Recovery must restore the active Core turn before handling effects")
        }
        XCTAssertEqual(control["session_id"], .string("independent-core-session"))
        XCTAssertEqual(control["active_turn_id"], .string("recovered-turn"))
        XCTAssertNotNil(recoveredObject["permission_projection"])
        XCTAssertNotNil(recoveredObject["task_projection"])
        guard case .object(let permissionRequest)? = requests.first else { return XCTFail("Missing request") }
        XCTAssertEqual(permissionRequest["session_id"], .string("independent-core-session"))

        _ = try application.acceptHostResponse(.object([:]), receivedAtMs: 1_001)
        let reduceCommand = try XCTUnwrap(store.commands.first { value in
            guard case .object(let command) = value else { return false }
            return jsonString(command, key: "type") == "reduce_and_accept_host_response"
        })
        guard case .object(let command) = reduceCommand,
              case .object(let input)? = command["input"] else {
            return XCTFail("Recovery did not resume through the active Run command")
        }
        XCTAssertEqual(jsonString(input, key: "run_id"), identity.submissionID)
        XCTAssertEqual(jsonString(input, key: "invocation_config_digest"), String(repeating: "a", count: 64))
        XCTAssertFalse(store.commands.contains { value in
            guard case .object(let command) = value else { return false }
            return jsonString(command, key: "type") == "begin_assistant_turn"
        })
    }

    func testExactRunRecoveryUsesBoundRunRatherThanSubmissionAndClaimsExistingToolOnly() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let runID = "distinct-bound-run"
        let store = PermissionRecoveryCommandStoreStub(runID: runID, sessionID: "automation-owner",
            coreSessionID: "bound-core", payloadType: "execute_tool", assistantMessageID: identity.assistantMessageID)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "automation-owner", conversationTitle: "", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: runID))
        let transition = try await application.recoverExactRun(prepareRecovery: { _ in },
            nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("Ready work must not sleep") })
        guard case .transition(.object(let value)) = transition, case .array(let requests)? = value["host_requests"] else {
            return XCTFail("Missing claimed transition")
        }
        XCTAssertEqual(requests.count, 1)
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command, case .string(let type)? = fields["type"] else { return false }
            return type == "begin_assistant_turn" || type == "begin_automation_assistant_turn"
        })
    }

    func testPermissionOnlyRecoveryStillRefusesAnExecutableTool() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID, sessionID: "owner", payloadType: "execute_tool")
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "owner", conversationTitle: "", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverPendingPermissionWait(prepareRecovery: { _ in })
            XCTFail("Permission-only reattachment must not dispatch executable work")
        } catch { }
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command else { return false }
            return fields["type"] == .string("claim_committed_host_requests")
        })
    }

    func testPermissionRecoveryRejectsChangedCheckpointBeforeClaim() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID,
            sessionID: "conversation-1", coreSessionID: "independent-core", changesBeforeClaim: true)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverPendingPermissionWait(prepareRecovery: { _ in },
                nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("No sleep expected") })
            XCTFail("Changed recovery proof must be rejected")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("changed while waiting"))
        }
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command else { return false }
            return fields["type"] == .string("claim_committed_host_requests")
        })
    }

    func testPermissionRecoveryRejectsForeignControlBeforeClaim() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID,
            sessionID: "conversation-1", coreSessionID: "independent-core", foreignControl: true)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverPendingPermissionWait(prepareRecovery: { _ in },
                nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("No sleep expected") })
            XCTFail("Foreign control must not authorize a claim")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("invalid owner"))
        }
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command else { return false }
            return fields["type"] == .string("claim_committed_host_requests")
        })
    }

    func testPermissionRecoveryCancellationDuringPreparationNeverClaims() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID, sessionID: "conversation-1")
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        let operation = Task {
            try await application.recoverPendingPermissionWait(prepareRecovery: { _ in
                withUnsafeCurrentTask { $0?.cancel() }
            }, nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("No sleep expected") })
        }
        do {
            _ = try await operation.value
            XCTFail("Cancelled preparation must stop before claiming work")
        } catch is CancellationError {
            // Expected: reconstruction can finish after its owner was cancelled.
        }
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command else { return false }
            return fields["type"] == .string("claim_committed_host_requests")
        })
    }

    func testPermissionRecoveryRejectsStaleTaskWitnessBeforeClaim() async throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = PermissionRecoveryCommandStoreStub(runID: identity.submissionID,
            sessionID: "conversation-1", staleTasks: true)
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "conversation-1", conversationTitle: "Recovery", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil))
        do {
            _ = try await application.recoverPendingPermissionWait(prepareRecovery: { _ in },
                nowMilliseconds: { 1_000 }, sleep: { _ in XCTFail("No sleep expected") })
            XCTFail("Task state must match the same recovery witness")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("checkpoint witness"))
        }
        XCTAssertFalse(store.commands.contains { command in
            guard case .object(let fields) = command else { return false }
            return fields["type"] == .string("claim_committed_host_requests")
        })
    }

    func testRecoveredPermissionValidationRejectsExpiredOrExecutableEffect() throws {
        let request = PermissionRecoveryCommandStoreStub.request(
            sessionID: "conversation-1",
            workerGeneration: "generation",
            leaseOwner: "owner",
            leaseExpiry: 999,
            payloadType: "request_permission"
        )
        XCTAssertThrowsError(try IOSCanonicalAgentApplication.validateRecoveredPermissionRequests(
            [request],
            runID: "run",
            sessionID: "conversation-1",
            workerGeneration: "generation",
            leaseOwner: "owner",
            claimedAtMs: 1_000
        ))
        let tool = PermissionRecoveryCommandStoreStub.request(
            sessionID: "conversation-1",
            workerGeneration: "generation",
            leaseOwner: "owner",
            leaseExpiry: 2_000,
            payloadType: "execute_tool"
        )
        XCTAssertThrowsError(try IOSCanonicalAgentApplication.validateRecoveredPermissionRequests(
            [tool],
            runID: "run",
            sessionID: "conversation-1",
            workerGeneration: "generation",
            leaseOwner: "owner",
            claimedAtMs: 1_000
        ))
    }

    func testBeginUsesCanonicalRunAndMessageIdentity() throws {
        let store = CommandStoreStub(responses: [
            .object([
                "execution": admissionExecutionFixture(),
                "events": .array([]),
                "traces": .array([]),
                "invocation_config_digest": .string("digest-1"),
            ]),
            .object(["requests": .array([])]),
        ])
        let userID = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
        let application = IOSCanonicalAgentApplication(
            store: store,
            descriptor: .init(
                conversationID: "conversation-1",
                conversationTitle: "Contract",
                identity: identity,
                userText: "hello",
                userContentJSON: nil,
                contextCheckpointID: nil,
                expectedConfigurationRevision: 9_007_199_254_740_993,
                expectedParentMessageID: "previous-assistant"
            )
        )

        _ = try application.begin(
            coreConfig: .object(["session_id": .string("conversation-1")]),
            submission: .object([
                "type": .string("user_input"),
                "submission_id": .string(identity.submissionID),
                "items": .array([]),
                "submitted_at_epoch_ms": .number(1),
                "delivery": .string("current_turn"),
            ]),
            createdAtMs: 1
        )

        guard case .object(let command) = store.commands[0],
              jsonString(command, key: "type") == "begin_assistant_turn",
              case .object(let input)? = command["input"] else {
            return XCTFail("Missing typed begin command")
        }
        XCTAssertEqual(jsonString(input, key: "user_message_id"), identity.userMessageID)
        XCTAssertEqual(jsonString(input, key: "assistant_message_id"), identity.assistantMessageID)
        XCTAssertEqual(input["expected_configuration_revision"], .signedInteger(9_007_199_254_740_993))
        XCTAssertEqual(jsonString(input, key: "expected_parent_message_id"), "previous-assistant")
        guard case .object(let submission)? = input["submission"] else {
            return XCTFail("Missing Core submission")
        }
        XCTAssertEqual(jsonString(submission, key: "submission_id"), identity.submissionID)
    }

    func testActiveCommandsUseTheDigestReturnedByBegin() throws {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = CommandStoreStub(responses: [
            .object([
                "execution": admissionExecutionFixture(),
                "events": .array([]),
                "traces": .array([]),
                "invocation_config_digest": .string("run-config-digest"),
            ]),
            .object(["requests": .array([])]),
            .object([
                "control": .null,
                "events": .array([]),
                "traces": .array([]),
                "successor_requests": .array([]),
            ]),
            .object(["requests": .array([])]),
            .object([
                "control": .object([
                    "session_id": .string("conversation-1"),
                    "active_turn_id": .string(identity.submissionID),
                    "last_event_cursor": .object(["sequence": .unsignedInteger(3)]),
                    "permission_projection_revision": .unsignedInteger(1),
                    "turn_status": .string("cancelling"),
                ]),
                "events": .array([]),
                "traces": .array([]),
                "successor_requests": .array([]),
            ]),
            .object([
                "conversation_id": .string("conversation-1"),
                "run_id": .string(identity.submissionID),
                "session_id": .string("conversation-1"),
                "active_turn_id": .string(identity.submissionID),
                "event_cursor": .unsignedInteger(3),
                "permission_projection_revision": .unsignedInteger(1),
                "permissions": .object(["revision": .unsignedInteger(1), "pending": .array([])]),
            ]),
            .object(["requests": .array([])]),
        ])
        let application = IOSCanonicalAgentApplication(
            store: store,
            descriptor: .init(
                conversationID: "conversation-1",
                conversationTitle: "Contract",
                identity: identity,
                userText: "hello",
                userContentJSON: nil,
                contextCheckpointID: nil
            )
        )
        _ = try application.begin(
            coreConfig: .object(["session_id": .string("conversation-1")]),
            submission: .object([
                "type": .string("user_input"),
                "submission_id": .string(identity.submissionID),
                "items": .array([]),
                "submitted_at_epoch_ms": .number(1),
                "delivery": .string("current_turn"),
            ]),
            createdAtMs: 1
        )
        _ = try application.acceptHostResponse(.object([:]), receivedAtMs: 2)
        guard case .object(let activeReply) = try application.submitActive(.object(["type": .string("cancel_requested")])) else {
            return XCTFail("Missing active control reply")
        }
        XCTAssertNil(activeReply["snapshot"])
        XCTAssertNotNil(activeReply["control"])
        XCTAssertNotNil(activeReply["permission_projection"])

        let activeCommands = store.commands.filter { value in
            guard case .object(let command) = value else { return false }
            let type = jsonString(command, key: "type")
            return type == "reduce_and_accept_host_response"
                || type == "reduce_and_commit_active_submission"
        }
        XCTAssertEqual(activeCommands.count, 2)
        for value in activeCommands {
            guard case .object(let command) = value,
                  case .object(let input)? = command["input"] else {
                return XCTFail("Missing active Run command")
            }
            XCTAssertEqual(jsonString(input, key: "invocation_config_digest"), "run-config-digest")
            XCTAssertNil(input["core_config"])
        }
    }

    private func jsonString(
        _ object: [String: AgentCoreJSONValue],
        key: String
    ) -> String? {
        guard case .string(let value)? = object[key] else { return nil }
        return value
    }
}

private func admissionExecutionFixture() -> AgentCoreJSONValue {
    .object([
        "control": .object([
            "session_id": .string("core"), "active_turn_id": .null,
            "turn_status": .string("awaiting_model"),
            "permission_projection_revision": .unsignedInteger(0),
            "last_event_cursor": .object(["sequence": .unsignedInteger(0)]),
        ]),
        "permissions": .object(["revision": .unsignedInteger(0), "pending": .array([])]),
    ])
}

private final class CancellationLostAcknowledgementStore: IOSAgentApplicationCommandStore, @unchecked Sendable {
    let underlying: CommandStoreStub
    private let lock = NSLock()
    private var dropped = false
    init(_ underlying: CommandStoreStub) { self.underlying = underlying }
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        let result = try underlying.executeAgentCommand(command)
        if case .object(let fields) = command, fields["type"] == .string("reduce_and_commit_active_submission") {
            let loseAcknowledgement = lock.withLock {
                if dropped { return false }
                dropped = true
                return true
            }
            if loseAcknowledgement { throw IOSApplicationRuntimeError.outcomePending("original-cancel") }
        }
        return result
    }
}

private final class CommandStoreStub: IOSAgentApplicationCommandStore, @unchecked Sendable {
    private let lock = NSLock()
    private var responses: [AgentCoreJSONValue]
    private(set) var commands: [AgentCoreJSONValue] = []
    private(set) var lastClaimedRequests: [AgentCoreJSONValue] = []

    init(responses: [AgentCoreJSONValue]) {
        self.responses = responses
    }

    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        lock.lock()
        defer { lock.unlock() }
        commands.append(command)
        if case .object(let fields) = command,
           fields["type"] == .string("encode_host_worker_identity") {
            return .object(["worker_generation": .string("fixture-worker-generation"),
                            "lease_owner": .string("fixture-worker-owner")])
        }
        // This fixture has no child Task receipt source. Checking that queue
        // must not consume a response intended for a permission/control command.
        if case .object(let fields) = command, fields["type"] == .string("apply_pending_task_notice") {
            return .null
        }
        guard !responses.isEmpty else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Stub response exhausted")
        }
        let response = responses.removeFirst()
        guard case .object(let commandFields) = command,
              commandFields["type"] == .string("claim_committed_host_requests"),
              case .object(let input)? = commandFields["input"],
              case .object(var result) = response,
              case .array(let requests)? = result["requests"] else { return response }
        let claimed = requests.map { request -> AgentCoreJSONValue in
            guard case .object(var envelope) = request else { return request }
            envelope["delivery"] = .object([
                "attempt_id": .string("physical-delivery-2"),
                "worker_generation": input["worker_generation"] ?? .null,
                "lease_owner": input["lease_owner"] ?? .null,
                "lease_token": .string("fixture-lease-token"),
                "lease_expires_at_ms": input["lease_expires_at_ms"] ?? .null,
            ])
            return .object(envelope)
        }
        lastClaimedRequests = claimed
        result["requests"] = .array(claimed)
        return .object(result)
    }
}

private final class PermissionRecoveryCommandStoreStub: IOSAgentApplicationCommandStore, @unchecked Sendable {
    private let lock = NSLock()
    private let runID: String
    private let sessionID: String
    private let coreSessionID: String
    private let changesBeforeClaim: Bool
    private let foreignControl: Bool
    private let staleTasks: Bool
    private let payloadType: String
    private let waitForLease: Bool
    private let requiresRecoveryDecision: Bool
    private let assistantMessageID: String
    private var invocationReadCount = 0
    private var claimCount = 0
    private(set) var commands: [AgentCoreJSONValue] = []

    init(runID: String, sessionID: String, coreSessionID: String? = nil, changesBeforeClaim: Bool = false,
         foreignControl: Bool = false, staleTasks: Bool = false,
         payloadType: String = "request_permission", assistantMessageID: String? = nil, waitForLease: Bool = false, requiresRecoveryDecision: Bool = false) {
        self.runID = runID
        self.sessionID = sessionID
        self.coreSessionID = coreSessionID ?? sessionID
        self.changesBeforeClaim = changesBeforeClaim
        self.foreignControl = foreignControl
        self.staleTasks = staleTasks
        self.payloadType = payloadType
        self.waitForLease = waitForLease
        self.requiresRecoveryDecision = requiresRecoveryDecision
        self.assistantMessageID = assistantMessageID ?? "\(runID):assistant"
    }

    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        lock.lock()
        defer { lock.unlock() }
        commands.append(command)
        guard case .object(let object) = command,
              case .string(let type)? = object["type"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Malformed test command")
        }
        switch type {
        case "encode_host_worker_identity":
            return .object(["worker_generation": .string("fixture-worker-generation"),
                            "lease_owner": .string("fixture-worker-owner")])
        case "load_conversation_task_projection_v1":
            return .object([
                "core_session_id": .string(coreSessionID),
                "checkpoint_revision": .number(staleTasks ? 0 : 1), "checkpoint_event_cursor": .number(2),
                "task_projection": .object(["revision": .number(0), "order": .array([]), "tasks": .object([:])]),
            ])
        case "load_host_execution_state":
            return .object([
                "control": .object([
                    "session_id": .string(foreignControl ? "foreign-core" : coreSessionID),
                    "active_turn_id": .string("recovered-turn"),
                    "permission_projection_revision": .number(1),
                ]),
                "permissions": .object(["revision": .number(1), "pending": .array([])]),
            ])
        case "load_exact_run_invocation":
            invocationReadCount += 1
            return .object([
                "conversation_id": .string(sessionID), "run_id": .string(runID),
                "assistant_message_id": .string(assistantMessageID),
                "invocation_config_digest": .string(String(repeating: "a", count: 64)),
                "core_session_id": .string(coreSessionID),
                "checkpoint_revision": .number(changesBeforeClaim && invocationReadCount >= 3 ? 2 : 1),
                "checkpoint_event_cursor": .number(2), "checkpoint_sha256": .string(String(repeating: "b", count: 64)),
                "status": .string("running"), "host_invocation": .object(["version": .number(3)]),
                "remote_tool_plan": .null,
            "context_budget": .object(["context_window_tokens": .number(4096), "reserved_output_tokens": .number(64),
                    "prompt_token_limit": .null, "auto_compact_tokens": .null, "compaction_target_tokens": .null,
                    "auto_compact_percent": .number(80), "compaction_target_percent": .number(50)]),
            ])
        case "active_run":
            return .object([
                "run_id": .string(runID),
                "kind": .string("conversation"),
                "assistant_message_id": .string(assistantMessageID),
                "next_event_sequence": .number(2),
                "invocation_config_digest": .string(String(repeating: "a", count: 64)),
            ])
        case "load_committed_host_requests":
            return .object(["requests": .array([.object([
                "payload": .object(["type": .string(payloadType)]),
            ])])])
        case "prepare_run_effect_recovery":
            return .object(["rebound_count": .number(0), "traces": .array([])])
        case "load_runtime_effect_work_schedule":
            if requiresRecoveryDecision { return .object(["runs": .array([]), "next_wake_at_ms": .null]) }
            if waitForLease { return .object(["runs": .array([]), "next_wake_at_ms": .number(241_000)]) }
            return .object(["runs": .array([.object(["run_id": .string(runID), "reason": .string("pending")])])])
        case "inspect_unknown_effect_recovery":
            return .object([
                "conversation_id": .string(sessionID), "run_id": .string(runID),
                "assistant_message_id": .string(assistantMessageID),
                "invocation_config_digest": .string(String(repeating: "a", count: 64)),
                "independent_pending_effects": .number(0), "retry_allowed": .bool(true),
                "targets": .array([.object([
                    "effect_id": .string("permission-effect"), "request_id": .string("permission-request"),
                    "attempt_id": .string("permission-attempt"), "worker_generation": .string("fixture-worker"),
                    "lease_owner": .string("fixture-owner"), "lease_token": .string("fixture-token"),
                    "expected_attempt_revision": .number(3),
                ])]),
            ])
        case "inspect_run_process_owners":
            // An unobservable owner must keep the original lease wait; this
            // fixture does not invent an OS death proof to bypass it.
            return .object([
                "conversation_id": .string(sessionID), "run_id": .string(runID),
                "assistant_message_id": .string(assistantMessageID),
                "invocation_config_digest": .string(String(repeating: "a", count: 64)),
                "host_invocation": .object(["version": .number(3)]),
                "has_session_lease": .bool(false), "has_unobservable_owner": .bool(true),
                "process_owners": .array([]), "deliveries": .array([])])
        case "validate_run_process_observations":
            return .bool(false)
        case "claim_committed_host_requests":
            claimCount += 1
            guard claimCount == 1,
                  case .object(let input)? = object["input"],
                  case .string(let generation)? = input["worker_generation"],
                  case .string(let owner)? = input["lease_owner"] else {
                return .object(["requests": .array([])])
            }
            return .object(["requests": .array([Self.request(
                sessionID: coreSessionID,
                workerGeneration: generation,
                leaseOwner: owner,
                leaseExpiry: 241_000,
                payloadType: payloadType
            )])])
        case "reduce_and_accept_host_response":
            return .object([
                "control": .null,
                "events": .array([]),
                "traces": .array([]),
            ])
        default:
            throw IOSCanonicalAgentApplicationError.invalidResult("Unexpected test command \(type)")
        }
    }

    static func request(
        sessionID: String,
        workerGeneration: String,
        leaseOwner: String,
        leaseExpiry: Int64,
        payloadType: String
    ) -> AgentCoreJSONValue {
        .object([
            "request_id": .string("request-1"),
            "effect_id": .string("effect-1"),
            "session_id": .string(sessionID),
            "delivery": .object([
                "attempt_id": .string("attempt-1"),
                "worker_generation": .string(workerGeneration),
                "lease_owner": .string(leaseOwner),
                "lease_token": .string("lease-token"),
                "lease_expires_at_ms": .number(Double(leaseExpiry)),
            ]),
            "payload": .object(["type": .string(payloadType)]),
        ])
    }
}

private extension AgentCoreJSONValue {
    var cancellationObject: [String: AgentCoreJSONValue]? {
        guard case .object(let fields) = self else { return nil }
        return fields
    }
}
