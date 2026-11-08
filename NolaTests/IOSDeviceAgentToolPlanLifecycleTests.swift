import Foundation
import XCTest
@testable import Nola

final class IOSDeviceAgentToolPlanLifecycleTests: XCTestCase {
    func testChildPermissionPresentationIdentityIncludesExactOwner() {
        func request(child: String, run: String) -> IOSAgentPermissionRequest {
            .init(coreRequestID: "same-core-request", callID: "call", capabilityID: "internal.capability",
                allowConversation: false, rationale: "Review", scope: nil, instructions: [],
                childOwner: .init(taskID: "task", parentConversationID: "parent",
                    childConversationID: child, runID: run, coreSessionID: "core-" + child))
        }
        let first = request(child: "child-a", run: "run-a")
        XCTAssertEqual(first.requestId, request(child: "child-b", run: "run-b").requestId)
        XCTAssertNotEqual(first.presentationID, request(child: "child-b", run: "run-b").presentationID)
        XCTAssertNotEqual(first.presentationID, request(child: "child-a", run: "run-new").presentationID)
        XCTAssertEqual(first.presentationID, request(child: "child-a", run: "run-a").presentationID)
        XCTAssertTrue(first.presentationRationale.contains("task"))
    }

    @MainActor
    func testCorePermissionWithoutOSDomainStillRequiresExplicitUserDecision() async {
        let request = IOSAgentPermissionRequest(coreRequestID: "request", callID: "call",
            capabilityID: "internal.capability", allowConversation: false, rationale: "Review",
            scope: nil, instructions: [], childOwner: nil)
        let broker = IOSAgentPermissionBroker()
        let denied = await broker.resolve(request, userApproved: false)
        let approved = await broker.resolve(request, userApproved: true)
        XCTAssertEqual(denied, .decision(requestId: "request", decision: .deny))
        XCTAssertEqual(approved, .decision(requestId: "request", decision: .allowOnce))
    }

    @MainActor
    func testIndependentChildWaitsForCommittedStartBeforeInvokingModel() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("child-barrier.db"))
        let identity = AgentCoreExpectedIdentity(hostId: "ios-local", clientId: "test", sessionId: "child")
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, allowedTools: [])
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "child", conversationTitle: "", identity: try .init(
                recoveringSubmissionID: "run", assistantMessageID: "assistant"),
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: "run"))
        let model = ToolPlanLifecycleModel(toolName: "device_read_file")
        let executor = DeviceAgentCoordinator(modelClient: model, childOnly: true,
            childPermissionOwner: .init(taskID: "task", parentConversationID: "parent", childConversationID: "child", runID: "run", coreSessionID: "child"),
            toolPlan: plan, canonicalApplication: application)
        // An intentionally unadmitted transport input must never reach a model
        // when the Application rejects its execution-start barrier.
        let request: AgentCoreJSONValue = .object([
            "effect_id": .string("effect"), "turn_id": .string("run"),
            "session_id": .string("child"), "delivery": .object([:]),
            "payload": .object(["type": .string("model_stream")]),
        ])
        for foreign in [("other-child", "run", "child"), ("child", "other-run", "child"), ("child", "run", "other-core-session")] {
            do {
                try await executor.executeChildHostRequest(request, conversationID: foreign.0, runID: foreign.1, coreSessionID: foreign.2,
                    config: .init(model: "fixture"), stopTarget: { _ in XCTFail("Foreign owner cannot stop anything") }) { _ in
                        XCTFail("Foreign owner must fail before execution-start or permission response")
                    }
                XCTFail("Foreign child ownership must be rejected")
            } catch DeviceAgentCoordinatorError.invalidTransition { }
        }
        do {
            try await executor.executeChildHostRequest(request, conversationID: "child", runID: "run", coreSessionID: "child",
                config: .init(model: "fixture"), stopTarget: { _ in XCTFail("No cancellation was admitted") }) { response in
                    XCTAssertEqual(response.objectFields?["payload"]?.objectFields?["type"], .string("execution_started"))
                    throw IOSApplicationRuntimeError.rejected("Unadmitted request")
                }
            XCTFail("A rejected start must remain rejected")
        } catch IOSApplicationRuntimeError.rejected(let message) { XCTAssertEqual(message, "Unadmitted request") }
        XCTAssertEqual(model.callCount, 0)
    }

    func testUIPublicationRejectionIsRecoverableButUnknownOutcomePropagates() throws {
        let guidance = #"{"code":"revision_conflict","expected":1,"current":2}"#
        let response = try DeviceAgentCoordinator.uiPublicationToolResponse(callID: "call") {
            throw IOSApplicationRuntimeError.rejected(guidance)
        }
        guard case .object(let fields) = response,
              case .object(let output)? = fields["output"],
              case .object(let execution)? = output["execution"],
              case .object(let data)? = output["data"] else { return XCTFail("Expected typed tool response") }
        XCTAssertEqual(fields["outcome"], .string("failed"))
        XCTAssertEqual(execution["state"], .string("not_started"))
        XCTAssertEqual(data["error"], .string(guidance))
        for error: Error in [IOSApplicationRuntimeError.outcomePending("receipt"), IOSApplicationRuntimeError.invalidResponse, CancellationError(), IOSApplicationRuntimeError.revisionConflict(scope: "store", expected: 1, actual: 2)] {
            XCTAssertThrowsError(try DeviceAgentCoordinator.uiPublicationToolResponse(callID: "call") { throw error })
        }
    }

    func testCancellationAdmissionIsRequiredExactAndDoesNotTurnColdAbsenceIntoStop() throws {
        let proof: [String: AgentCoreJSONValue] = [
            "conversation_id": .string("conversation"), "run_id": .string("run"),
            "core_session_id": .string("core"), "host_id": .string("ios-local"),
            "cancellation_effect_id": .string("cancel"), "cancellation_request_id": .string("request"),
            "target_effect_id": .string("target"), "target_request_id": .string("target-request"),
            "target_payload_identity": .string(String(repeating: "a", count: 64)),
            "checkpoint_revision": .number(3), "revoked_attempts": .array([
                .object(["attempt_id": .string("revoked"), "revision": .number(2)])]),
        ]
        var envelope: [String: AgentCoreJSONValue] = [
            "session_id": .string("core"), "turn_id": .string("run"), "host_id": .string("ios-local"),
            "effect_id": .string("cancel"), "request_id": .string("request"),
            "payload": .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                  "target_not_started": .object(proof)]),
        ]
        func confirm(_ value: [String: AgentCoreJSONValue], local: Set<String> = [], unknown: Set<String> = []) throws -> String {
            try IOSCancellationAdmissionContract.confirmableTarget(envelope: value, conversationID: "conversation",
                locallyQuiescent: local, unconfirmed: unknown)
        }
        XCTAssertEqual(try confirm(envelope), "target")
        XCTAssertThrowsError(try confirm(envelope, unknown: ["target"]), "a contradictory physical uncertainty cannot be overridden")
        for key in ["conversation_id", "run_id", "core_session_id", "host_id", "cancellation_effect_id",
                    "cancellation_request_id", "target_effect_id", "target_payload_identity"] {
            var foreign = proof
            foreign[key] = .string("foreign")
            var request = envelope
            request["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                          "target_not_started": .object(foreign)])
            XCTAssertThrowsError(try confirm(request), key)
        }
        envelope["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target"),
                                        "target_not_started": .null])
        XCTAssertThrowsError(try confirm(envelope), "a cold empty registry is not proof")
        XCTAssertEqual(try confirm(envelope, local: ["target"]), "target")
        XCTAssertThrowsError(try confirm(envelope, local: ["other"]), "another effect cannot stop this target")
        envelope["payload"] = .object(["type": .string("cancel"), "target_effect_id": .string("target")])
        XCTAssertThrowsError(try confirm(envelope, local: ["target"]), "missing required nullable field is rejected")
    }

    @MainActor
    func testCancellationDuringInitialObservationDoesNotBeginCanonicalRun() async throws {
        try await exerciseCancellation(suspendObservation: 1)
    }

    @MainActor
    func testCancellationDuringPermissionObservationDoesNotReviveFilePermission() async throws {
        try await exerciseCancellation(suspendObservation: 2, permissionRequired: true)
    }

    @MainActor
    func testCancellationDuringOrdinaryReadObservationDoesNotExecuteUnscopedFile() async throws {
        try await exerciseCancellation(suspendObservation: 2)
    }

    @MainActor
    func testRejectedSecondStreamDoesNotCancelOriginalOwner() async throws {
        try await exerciseCancellation(suspendObservation: 2, permissionRequired: true, rejectSecondStream: true)
    }

    @MainActor
    func testCancellingUnconfirmedPhysicalModelDoesNotInventTerminalSealOrReplay() async throws {
        try await exercisePhysicalStreamTermination(userStop: true)
    }

    @MainActor
    func testConsumerDetachDoesNotPersistBusinessCancellationOrReplay() async throws {
        try await exercisePhysicalStreamTermination(userStop: false)
    }

    @MainActor
    func testNativeStopPersistsExactInterruptBeforeRetiringUIObservation() async throws {
        try await exercisePhysicalStreamTermination(userStop: true, retireObserverAfterInterrupt: true)
    }

    @MainActor
    func testDelayedNativeStopNeverRetiresReplacementUIOwner() async throws {
        try await exercisePhysicalStreamTermination(userStop: true, retireObserverAfterInterrupt: true, changeOwnerDuringCancel: true)
    }

    @MainActor
    private func exercisePhysicalStreamTermination(userStop: Bool, retireObserverAfterInterrupt: Bool = false,
                                                   changeOwnerDuringCancel: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sessionID = UUID().uuidString
        let expectedIdentity = AgentCoreExpectedIdentity(hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: sessionID)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: expectedIdentity, isForeground: true, allowedTools: [],
            resultEgressAuthorization: .explicitUserApproval(identity: expectedIdentity, userApproved: true))
        let toolSession = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000),
                isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("physical.db"))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let application = IOSCanonicalAgentApplication(store: runtime, descriptor: .init(
            conversationID: sessionID, conversationTitle: "Unconfirmed physical stop", identity: identity,
            userText: "Hold the physical model", userContentJSON: nil, contextCheckpointID: nil))
        let dispatched = expectation(description: "Physical model stream was dispatched")
        let drained = expectation(description: "Cancelled model observation ended")
        let detached = userStop ? nil : expectation(description: "Physical stream observation detached")
        let model = ToolPlanLifecycleModel(toolName: "device_read_file", heldModelStarted: dispatched,
            heldModelDetached: detached)
        defer { model.releaseHeldModel() }
        let coordinator = DeviceAgentCoordinator(modelClient: model, toolPlan: plan,
            canonicalApplication: application, toolPlanSession: toolSession,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: runtime, model: "fixture",
                outputTokens: 1_024, capabilities: model.coreModelCapabilityProfile))
        let stream = await coordinator.start(sessionId: sessionID, input: "Hold the physical model", history: [],
            turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                permissionProfile: .cautious))
        let consumer = Task {
            do {
                for try await event in stream {
                    if case .completed = event { XCTFail("Unconfirmed stop cannot publish a completed Run") }
                }
            } catch is CancellationError {} catch { XCTFail("Unexpected observation error: \(error)") }
            drained.fulfill()
        }
        await fulfillment(of: [dispatched], timeout: 5)
        if retireObserverAfterInterrupt {
            let ignored = await ChatManager.cancelNativeBeforeRetiringObservation(coordinator,
                isCurrent: { false }, retireObservation: { XCTFail("A stale owner must not retire current observation") })
            XCTAssertFalse(ignored)
            XCTAssertEqual(model.callCount, 1)
            var ownerChecks = 0
            let retired = await ChatManager.cancelNativeBeforeRetiringObservation(coordinator,
                isCurrent: { ownerChecks += 1; return !changeOwnerDuringCancel || ownerChecks == 1 }, retireObservation: {
                    XCTAssertFalse(changeOwnerDuringCancel, "A resumed cancellation cannot clear replacement UI state")
                    do {
                        let pending = try runtime.executeAgentCommand(.object([
                            "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)]))
                        guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
                            return XCTFail("Missing canonical cancellation effects before observer retirement")
                        }
                        XCTAssertTrue(requests.contains {
                            guard case .object(let envelope) = $0, case .object(let payload)? = envelope["payload"] else { return false }
                            return payload["type"] == .string("cancel")
                        }, "The real Store must contain the interrupt before stream cancellation can clear its permit")
                    } catch { XCTFail("Cancellation persistence could not be verified: \(error)") }
                    consumer.cancel()
                })
            XCTAssertEqual(retired, !changeOwnerDuringCancel)
        } else if userStop { await coordinator.cancel() } else { consumer.cancel() }
        await fulfillment(of: [drained], timeout: 5)
        if userStop {
            do {
                try await coordinator.waitForCancellation()
                XCTFail("A stopped consumer is not proof that the physical provider stopped")
            } catch DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed {}
        } else {
            await fulfillment(of: [try XCTUnwrap(detached)], timeout: 5)
            try await coordinator.waitForCancellation()
        }
        await consumer.value
        XCTAssertNil(try application.committedTerminalSeal(coreSessionID: sessionID))
        XCTAssertThrowsError(try ChatManager.validateLocalRunAdmission(runtime: runtime, conversationID: sessionID)) { error in
            XCTAssertTrue(error is IOSLocalRunAdmissionError)
        }
        let pending = try runtime.executeAgentCommand(.object([
            "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)
        ]))
        guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
            return XCTFail("Missing durable cancellation control")
        }
        XCTAssertEqual(requests.contains { request in
            guard case .object(let envelope) = request, case .object(let payload)? = envelope["payload"] else { return false }
            return payload["type"] == .string("cancel")
        }, userStop, "Only explicit user Stop may persist a cancellation control")
        XCTAssertEqual(model.callCount, 1, "Cancellation cleanup cannot replay the physical model")
    }

    @MainActor
    private func exerciseCancellation(suspendObservation: Int, permissionRequired: Bool = false,
                                      rejectSecondStream: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sessionID = UUID().uuidString
        let expectedIdentity = AgentCoreExpectedIdentity(hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: sessionID)
        // A plain read no longer asks for Nola confirmation because OS scope
        // is absent. The real write contract exercises permission observation.
        let toolName = permissionRequired ? "device_write_file" : "device_read_file"
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: expectedIdentity, isForeground: true, allowedTools: [toolName],
            resultEgressAuthorization: .explicitUserApproval(identity: expectedIdentity, userApproved: true))
        // Egress consent permits advertising the tool; no file scope is granted.
        let fileTool = try XCTUnwrap(plan.output.enabledTools.first { $0.toolId == toolName },
            "Disabled tools: \(plan.output.disabledTools)")
        XCTAssertEqual(fileTool.authorization, "ask")
        let boundTool = try XCTUnwrap(plan.resolvedTool(id: toolName, route: "device",
            hostId: expectedIdentity.hostId, clientId: expectedIdentity.clientId, sessionId: sessionID))
        XCTAssertEqual(boundTool, fileTool)
        let restoredOutput = try JSONDecoder().decode(AgentCoreToolPolicyOutput.self,
            from: JSONEncoder().encode(plan.output))
        let restoredTool = try XCTUnwrap(restoredOutput.enabledTools.first { $0.toolId == toolName })
        XCTAssertEqual(restoredTool, boundTool, "The resolved bound contract must survive its current wire roundtrip")
        guard case .object(let schema) = restoredTool.schema,
              case .object(let metadata)? = schema["x_nola"] else {
            return XCTFail("The bound native manifest lost its trusted permission contract")
        }
        XCTAssertEqual(Set(metadata.keys), Set(["permission_contract"]), "Client display metadata is not execution authority")
        XCTAssertEqual(metadata["permission_contract"], .object([
            "risk": .string(permissionRequired ? "unknown" : "ordinary_read")
        ]))
        let suspended = expectation(description: "Authorization observation suspended")
        let consumerFinished = expectation(description: "Cancelled stream drained")
        let originalPermission = rejectSecondStream ? expectation(description: "Original owner still publishes its permission") : nil
        let forbiddenHTTP = expectation(description: "Cancelled tool must not send HTTP")
        forbiddenHTTP.isInverted = true
        let forbiddenBegin = suspendObservation == 1
            ? expectation(description: "Cancelled preparation must not create a Run") : nil
        forbiddenBegin?.isInverted = true
        let gate = ToolPlanObservationGate(at: suspendObservation, suspended: suspended,
            observation: .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000),
                isForeground: true, authorizations: plan.authorizationSnapshot))
        let toolSession = IOSRunToolPlanSession(plan: plan, observe: { await gate.observe() })
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
        let store = ToolPlanLifecycleStore(runtime: runtime, forbiddenBegin: forbiddenBegin)
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let application = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: sessionID, conversationTitle: "Cancellation isolation", identity: identity,
            userText: "Read the fixture", userContentJSON: nil, contextCheckpointID: nil))
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ToolPlanBlockedURLProtocol.self]
        ToolPlanBlockedURLProtocol.install(forbiddenHTTP)
        defer { ToolPlanBlockedURLProtocol.install(nil) }
        let urlSession = URLSession(configuration: configuration)
        defer { urlSession.invalidateAndCancel() }
        let executor = IOSAgentHostExecutor(session: urlSession)
        await executor.acquireAgentSession(sessionId: sessionID)
        let model = ToolPlanLifecycleModel(toolName: toolName)
        let coordinator = DeviceAgentCoordinator(modelClient: model, hostExecutor: executor,
            toolPlan: plan, canonicalApplication: application, toolPlanSession: toolSession,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: runtime, model: "fixture",
                outputTokens: 1_024, capabilities: model.coreModelCapabilityProfile))
        let stream = await coordinator.start(sessionId: sessionID, input: "Read the fixture", history: [],
            turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                permissionProfile: .cautious))
        let consumer = Task { () -> Int in
            var permissions = 0
            do {
                for try await event in stream {
                    if case .permissionRequested = event {
                        permissions += 1
                        originalPermission?.fulfill()
                    }
                }
            } catch is CancellationError {} catch {
                XCTFail("Unexpected lifecycle error: \(error)")
            }
            await gate.markConsumerFinished()
            consumerFinished.fulfill()
            return permissions
        }
        await fulfillment(of: [suspended], timeout: 5)
        let reachedObservation = await gate.didSuspend
        if rejectSecondStream {
            let rejected = await coordinator.start(sessionId: sessionID, input: "Must not replace owner", history: [],
                turnIdentity: identity, config: .init(platform: .ios, executionMode: .deviceLocal,
                    model: "fixture", maxIterations: 2, contextWindowTokens: 64_000, reservedOutputTokens: 1_024,
                    permissionProfile: .cautious))
            do {
                for try await _ in rejected {}
                XCTFail("A second stream must reject without changing the first owner")
            } catch DeviceAgentCoordinatorError.alreadyRunning {}
            await gate.release()
            if let originalPermission { await fulfillment(of: [originalPermission], timeout: 5) }
        }
        await coordinator.cancel()
        if suspendObservation != 1 && !rejectSecondStream {
            XCTAssertNil(try application.committedTerminalSeal(coreSessionID: sessionID),
                "An authorization await has not returned: closing observation must not manufacture cancellation acknowledgement")
        }
        await gate.release()
        // Always consume every created expectation, including failure cleanup.
        await fulfillment(of: [consumerFinished], timeout: 5)
        try await coordinator.waitForCancellation()
        await fulfillment(of: [forbiddenHTTP] + (forbiddenBegin.map { [$0] } ?? []), timeout: 0.2)
        guard reachedObservation else {
            consumer.cancel()
            await executor.releaseAgentSession(sessionId: sessionID)
            return // The failed expectation must not create a new suspended observation.
        }
        // Join the shared refresh itself; negative expectations cover late actor work
        // after that observation completes, without granting a replacement lifecycle.
        _ = try await toolSession.currentPlan()
        guard await gate.consumerFinished else {
            consumer.cancel()
            await executor.releaseAgentSession(sessionId: sessionID)
            return
        }
        let permissionCount = await consumer.value
        XCTAssertEqual(permissionCount, rejectSecondStream ? 1 : 0)
        for requestID in store.permissionRequestIDs {
            XCTAssertNotEqual(requestID, "fixture-call", "A permission request identity is not its model call identity")
            do {
                _ = try await coordinator.currentPermissionRequest(requestId: requestID)
                XCTFail("Cancelled lifecycle must not expose its actual pending permission")
            } catch DeviceAgentCoordinatorError.permissionNotPending {
                // Query the exact Core-issued ID, not an unrelated call ID.
            }
        }
        XCTAssertEqual(model.callCount, suspendObservation == 1 ? 0 : 1)
        if suspendObservation == 1 {
            XCTAssertEqual(store.commandCount, 0)
        } else if permissionRequired {
            XCTAssertGreaterThan(store.permissionClaims, 0,
                "The real Application must have claimed the permission effect before observation pauses")
            XCTAssertFalse(store.permissionRequestIDs.isEmpty)
            XCTAssertTrue(store.permissionClaimsHaveDeliveryWithoutModelAttempt,
                "Permission delivery identity is not a model-attempt identity")
        } else {
            XCTAssertEqual(store.permissionClaims, 0,
                "An ordinary read must not ask for Nola authority before its Host scope check")
        }
        if suspendObservation != 1 {
            let reopened = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
            let coldApplication = IOSCanonicalAgentApplication(store: reopened, descriptor: .init(
                conversationID: sessionID, conversationTitle: "", identity: identity,
                userText: "", userContentJSON: nil, contextCheckpointID: nil))
            let seal = try XCTUnwrap(coldApplication.committedTerminalSeal(coreSessionID: sessionID))
            XCTAssertEqual(seal.result.value.kind, .cancelled)
            XCTAssertNil(try reopened.loadConversationProjectionV1(id: sessionID).activeRun)
            XCTAssertNoThrow(try ChatManager.validateLocalRunAdmission(runtime: reopened, conversationID: sessionID))
            let pending = try reopened.executeAgentCommand(.object([
                "type": .string("load_committed_host_requests"), "run_id": .string(identity.submissionID)
            ]))
            guard case .object(let fields) = pending, case .array(let requests)? = fields["requests"] else {
                return XCTFail("Missing formal outbox projection")
            }
            XCTAssertTrue(requests.isEmpty, "Cancellation controls must settle rather than disappear only from the UI queue")
        }
        await executor.releaseAgentSession(sessionId: sessionID)
    }
}

private actor ToolPlanObservationGate {
    let target: Int
    let suspended: XCTestExpectation
    let observation: IOSRunToolPlanObservation
    var count = 0
    var released = false
    var didSuspend = false
    var consumerFinished = false
    var continuation: CheckedContinuation<Void, Never>?
    init(at target: Int, suspended: XCTestExpectation, observation: IOSRunToolPlanObservation) {
        self.target = target; self.suspended = suspended; self.observation = observation
    }
    func observe() async -> IOSRunToolPlanObservation {
        count += 1
        if count == target && !released {
            await withCheckedContinuation { continuation in
                self.continuation = continuation
                didSuspend = true
                suspended.fulfill()
            }
        }
        return observation
    }
    func release() { released = true; continuation?.resume(); continuation = nil }
    func markConsumerFinished() { consumerFinished = true }
}

private nonisolated final class ToolPlanLifecycleStore: IOSAgentApplicationCommandStore, @unchecked Sendable {
    let runtime: IOSApplicationRuntime
    let forbiddenBegin: XCTestExpectation?
    private let lock = NSLock()
    private var count = 0
    private var claimedPermissions = 0
    private var validPermissionIdentities = true
    private var claimedPermissionRequestIDs: Set<String> = []
    var commandCount: Int { lock.withLock { count } }
    var permissionClaims: Int { lock.withLock { claimedPermissions } }
    var permissionClaimsHaveDeliveryWithoutModelAttempt: Bool { lock.withLock { validPermissionIdentities } }
    var permissionRequestIDs: Set<String> { lock.withLock { claimedPermissionRequestIDs } }
    init(runtime: IOSApplicationRuntime, forbiddenBegin: XCTestExpectation?) {
        self.runtime = runtime; self.forbiddenBegin = forbiddenBegin
    }
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        lock.withLock { count += 1 }
        if case .object(let fields) = command, fields["type"] == .string("begin_assistant_turn") {
            forbiddenBegin?.fulfill()
        }
        let result = try runtime.executeAgentCommand(command)
        if case .object(let fields) = command, fields["type"] == .string("claim_committed_host_requests"),
           case .object(let root) = result, case .array(let requests)? = root["requests"] {
            for request in requests {
                guard case .object(let envelope) = request,
                      case .object(let payload)? = envelope["payload"],
                      payload["type"] == .string("request_permission") else { continue }
                let valid: Bool
                if case .object(let delivery)? = envelope["delivery"],
                   case .string(let deliveryID)? = delivery["attempt_id"], !deliveryID.isEmpty {
                    valid = envelope["attempt_id"] == nil || envelope["attempt_id"] == .null
                } else { valid = false }
                lock.withLock {
                    claimedPermissions += 1
                    validPermissionIdentities = validPermissionIdentities && valid
                    if case .string(let permissionID)? = payload["permission_request_id"] {
                        claimedPermissionRequestIDs.insert(permissionID)
                    }
                }
            }
        }
        return result
    }
}

private nonisolated final class ToolPlanLifecycleModel: AgentModelClient, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    let toolName: String
    private let heldModelStarted: XCTestExpectation?
    private var heldContinuation: AsyncThrowingStream<AgentModelEvent, Error>.Continuation?
    private let heldModelDetached: XCTestExpectation?
    init(toolName: String, heldModelStarted: XCTestExpectation? = nil, heldModelDetached: XCTestExpectation? = nil) {
        self.toolName = toolName
        self.heldModelStarted = heldModelStarted
        self.heldModelDetached = heldModelDetached
    }
    func releaseHeldModel() {
        let continuation = lock.withLock {
            let value = heldContinuation
            heldContinuation = nil
            return value
        }
        continuation?.finish()
    }
    var callCount: Int { lock.withLock { count } }
    let coreModelCapabilityProfile: AgentCoreJSONValue? = .object([
        "profile_id": .string("ios-native-event-test-v1"), "revision": .number(1),
        "instruction_profile": .string("generic"), "native_tool_calls": .bool(true),
        "parallel_tool_calls": .bool(false), "reasoning": .bool(false), "reasoning_continuity": .bool(false),
        "prompt_cache": .bool(false), "context_window_tokens": .null, "max_output_tokens": .null,
        "request_continuation": .bool(true), "max_request_continuations": .number(2),
        "stream_text": .bool(true), "stream_reasoning": .bool(false), "stream_tool_calls": .bool(true)
    ])
    func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
        let iteration = lock.withLock { count += 1; return count }
        if let heldModelStarted {
            return AsyncThrowingStream { continuation in
                lock.withLock { heldContinuation = continuation }
                continuation.onTermination = { [heldModelDetached] _ in heldModelDetached?.fulfill() }
                heldModelStarted.fulfill()
            }
        }
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(.init(content: iteration == 1 ? "" : "done",
                toolCalls: iteration == 1 ? [.init(id: "fixture-call", name: toolName,
                    arguments: toolName == "device_write_file"
                        ? .object(["content": .string("cancelled fixture")]) : .object([:]))] : [],
                finishReason: iteration == 1 ? "tool_calls" : "stop")))
            continuation.finish()
        }
    }
}

private nonisolated final class ToolPlanBlockedURLProtocol: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var expectation: XCTestExpectation?
    static func install(_ value: XCTestExpectation?) { lock.withLock { expectation = value } }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lock.withLock { Self.expectation?.fulfill() }
        client?.urlProtocol(self, didFailWithError: URLError(.cancelled))
    }
    override func stopLoading() {}
}
