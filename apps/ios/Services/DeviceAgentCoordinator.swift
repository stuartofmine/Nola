// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Thin iOS Host driver for the shared Rust Agent Core v2 reducer.
// - Boundary: Executes HostRequest values and renders only normalized Core events.

import Foundation

nonisolated public enum DeviceAgentCoordinatorEvent {
    case snapshot(AgentCoreSnapshot)
    case assistantJournalCommitted(AgentCoreJSONValue)
    case coreTurnTerminal(AgentCoreTurnTerminalEnvelope)
    case taskProjectionUpdated(AgentCoreTaskProjection)
    case skillRuntimeProjectionUpdated(AgentCoreSkillRuntimeProjection)
    case capabilitiesActivated(AgentCoreCapabilityActivationEnvelope)
    case trace(AgentTraceEvent)
    case permissionRequested(IOSAgentPermissionRequest)
    case permissionResolved(requestId: String, toolName: String, decision: AgentCorePermissionDecision)
    case completed(IOSRunResult)
}

nonisolated public enum DeviceAgentCoordinatorError: LocalizedError {
    case alreadyRunning
    case invalidMode
    case missingTransition
    case invalidTransition
    case modelEndedWithoutCompletion
    case permissionNotPending
    case invalidPermissionEvent
    case invalidToolPlan
    case unsupportedHostRequest(String)
    case cancellationOutcomeUnconfirmed

    public var errorDescription: String? {
        switch self {
        case .alreadyRunning: return "A device agent turn is already running"
        case .invalidMode: return "DeviceAgentCoordinator requires device-local mode"
        case .missingTransition: return "Nola Core v2 returned no transition"
        case .invalidTransition: return "Nola Core v2 returned an invalid transition"
        case .modelEndedWithoutCompletion: return "The model stream ended without a completion"
        case .permissionNotPending: return "The agent is not waiting for permission"
        case .invalidPermissionEvent: return "Nola Core returned an invalid permission request"
        case .invalidToolPlan: return "The Core tool plan is stale or does not match the requested handler"
        case .unsupportedHostRequest(let type): return "The iOS Host cannot execute \(type)"
        case .cancellationOutcomeUnconfirmed: return "Execution stopped locally, but the physical operation's stop is unconfirmed. Its cancellation remains recoverable."
        }
    }
}

enum IOSChildPermissionEvent: Sendable {
    case requested(IOSAgentPermissionRequest, DeviceAgentCoordinator)
    case removed(IOSChildPermissionOwner, String, DeviceAgentCoordinator)
}

public actor DeviceAgentCoordinator {
    private typealias StreamContinuation = AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error>.Continuation

    private let expertTargetConfiguration: (@Sendable (AppleExpertPeerMaterial<AgentCoreJSONValue>) async throws -> AgentCoreJSONValue)?
    private let expertTaskTarget: AppleExpertTaskTargetV1?
    private let delegationSettings: AgentCoreJSONValue?
    private var localTaskHost: IOSLocalTaskHost?
    private let childOnly: Bool
    private let childPermissionOwner: IOSChildPermissionOwner?
    private let childPermissionEvents: (@Sendable (IOSChildPermissionEvent) async -> Void)?
    private let childTaskStateChanged: (@Sendable () async -> Void)?
    private var childPermissionWaiters: [String: CheckedContinuation<Void, Error>] = [:]
    private var childResponseSinks: [String: @Sendable (AgentCoreJSONValue) async throws -> Void] = [:]
    private var childOwner: (conversation: String, run: String)?
    private let modelClient: AgentModelClient
    private let freshResendTarget: String?
    private let preparedHostInvocation: IOSPreparedHostInvocation?
    private let recoveryPreparation: (@Sendable (IOSExactRunInvocation) async throws -> Void)?
    private var recoveryInputOrigin: AgentCoreInputOrigin = .interactiveUser
    private let hostDependencyIdentity: AgentCoreJSONValue
    private let hostExecutor: IOSAgentHostExecutor
    private var toolPlan: AgentCorePreparedToolPlan
    private let toolPlanSession: IOSRunToolPlanSession
    private let modelSupportsToolCalls: Bool
    private let canonicalApplication: IOSCanonicalAgentApplication
    private var continuation: StreamContinuation?
    private var lifecycleID: UUID?
    private var executionPermit: IOSRunExecutionPermit?
    private var activeConfig: AgentCoreRuntimeConfig?
    private var activeSessionId: String?
    private var recoveryTraceRunID: String?
    private var recoveryTraceSequence: UInt64 = 0
    private var activeTurnId: String?
    private var pendingPermissions: [String: IOSAgentPermissionRequest] = [:]
    private var pendingPermissionEnvelopes: [String: AgentCoreJSONValue] = [:]
    private var queuedHostRequests: [AgentCoreJSONValue] = []
    private var responseSequences: [String: Int] = [:]
    private var responseAttemptIDs: [String: String] = [:]
    private var responseDeliveries: [String: AgentCoreJSONValue] = [:]
    private var leaseHeartbeatTasks: [String: Task<Void, Never>] = [:]
    private var taskProjection = AgentCoreTaskProjectionReducer()
    private var skillRuntimeProjection = AgentCoreSkillRuntimeProjectionReducer()
    private var capabilityActivationCursor: UInt64 = 0
    private var modelIteration = 0
    private var drivingOwner: IOSRunExecutionPermit?
    private var driverTask: Task<Void, Error>?
    private var activeStreamID: UUID?
    private var cancellingOwner: IOSRunExecutionPermit?
    private var cancellationTask: Task<Void, Never>?
    private var cancellationError: Error?
    private var unconfirmedPhysicalEffects: Set<String> = []
    private struct ChildModelStop {
        let id = UUID()
        let conversationID: String
        let runID: String
        let coreSessionID: String
        let stop: @Sendable () async throws -> Void
    }
    private var childModelStops: [String: ChildModelStop] = [:]
    private var childModelStopTasks: [String: (id: UUID, task: Task<Void, Error>)] = [:]

    // Populated only by this owner's actual returned operation, never by a
    // restored queue or absence from the uncertainty set.
    private var locallyQuiescentEffects: Set<String> = []

    func refreshPermissionPolicy() async throws {
        guard let continuation, executionPermit != nil, cancellingOwner == nil,
              let transition = try canonicalApplication.applyPendingPermissionPolicy() else { return }
        try await drive(transition, continuation: continuation)
    }

    init(
        modelClient: AgentModelClient,
        childOnly: Bool = false,
        childPermissionOwner: IOSChildPermissionOwner? = nil,
        childPermissionEvents: (@Sendable (IOSChildPermissionEvent) async -> Void)? = nil,
        childTaskStateChanged: (@Sendable () async -> Void)? = nil,
        delegationSettings: AgentCoreJSONValue? = nil,
        hostExecutor: IOSAgentHostExecutor = .init(),
        toolPlan: AgentCorePreparedToolPlan,
        modelSupportsToolCalls: Bool = true,
        canonicalApplication: IOSCanonicalAgentApplication,
        toolPlanSession: IOSRunToolPlanSession? = nil,
        preparedHostInvocation: IOSPreparedHostInvocation? = nil,
        recoveryPreparation: (@Sendable (IOSExactRunInvocation) async throws -> Void)? = nil,
        hostDependencyIdentity: AgentCoreJSONValue = .array([]),
        freshResendTarget: String? = nil,
        expertTargetConfiguration: (@Sendable (AppleExpertPeerMaterial<AgentCoreJSONValue>) async throws -> AgentCoreJSONValue)? = nil,
        expertTaskTarget: AppleExpertTaskTargetV1? = nil
    ) {
        self.expertTargetConfiguration = expertTargetConfiguration
        self.expertTaskTarget = expertTaskTarget
        self.freshResendTarget = freshResendTarget
        self.delegationSettings = delegationSettings
        self.childOnly = childOnly
        self.childPermissionOwner = childPermissionOwner
        self.childPermissionEvents = childPermissionEvents
        self.childTaskStateChanged = childTaskStateChanged
        self.modelClient = modelClient
        self.preparedHostInvocation = preparedHostInvocation
        self.recoveryPreparation = recoveryPreparation
        self.hostDependencyIdentity = hostDependencyIdentity
        self.hostExecutor = hostExecutor
        self.toolPlan = toolPlan
        self.toolPlanSession = toolPlanSession ?? IOSRunToolPlanSession(plan: toolPlan)
        self.modelSupportsToolCalls = modelSupportsToolCalls
        self.canonicalApplication = canonicalApplication
    }

    func start(
        sessionId: String?,
        input: String,
        history: [AgentCoreMessage],
        inputOrigin: AgentCoreInputOrigin = .interactiveUser,
        contextComponents: [AgentCoreContextComponent] = [],
        turnIdentity: IOSCanonicalAgentTurnIdentity? = nil,
        config: AgentCoreRuntimeConfig
    ) -> AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error> {
        let streamID = UUID()
        return AsyncThrowingStream { continuation in
            guard !childOnly, self.continuation == nil else {
                continuation.finish(throwing: DeviceAgentCoordinatorError.alreadyRunning)
                return
            }
            self.continuation = continuation
            activeStreamID = streamID
            cancellationError = nil
            continuation.onTermination = { [weak self] _ in
                Task { await self?.detachStream(streamID) }
            }
            Task {
                await self.begin(
                    sessionId: sessionId,
                    input: input,
                    history: history,
                    inputOrigin: inputOrigin,
                    contextComponents: contextComponents,
                    turnIdentity: turnIdentity,
                    config: config,
                    streamID: streamID,
                    continuation: continuation
                )
            }
        }
    }

    /// Restores an Application-owned permission wait without creating a new
    /// user submission or replaying any executable Host effect.
    func recoverPendingPermissionWait(
        sessionId: String,
        config: AgentCoreRuntimeConfig
    ) -> AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error> {
        recoverExactRun(sessionId: sessionId, config: config, permissionOnly: true)
    }

    func recoverExactRun(
        sessionId: String,
        config: AgentCoreRuntimeConfig,
        permissionOnly: Bool = false,
        inputOrigin: AgentCoreInputOrigin = .interactiveUser
    ) -> AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error> {
        let streamID = UUID()
        return AsyncThrowingStream { continuation in
            guard self.continuation == nil else {
                continuation.finish(throwing: DeviceAgentCoordinatorError.alreadyRunning)
                return
            }
            self.continuation = continuation
            activeStreamID = streamID
            cancellationError = nil
            continuation.onTermination = { [weak self] _ in
                Task { await self?.detachStream(streamID) }
            }
            driverTask = Task {
                await self.recoverPendingPermissionWait(
                    sessionId: sessionId,
                    config: config,
                    permissionOnly: permissionOnly,
                    inputOrigin: inputOrigin,
                    streamID: streamID,
                    continuation: continuation
                )
            }
        }
    }

    private func recoverPendingPermissionWait(
        sessionId: String,
        config: AgentCoreRuntimeConfig,
        permissionOnly: Bool,
        inputOrigin: AgentCoreInputOrigin,
        streamID: UUID,
        continuation: StreamContinuation
    ) async {
        guard activeStreamID == streamID else { return }
        guard config.executionMode == .deviceLocal else {
            finish(throwing: DeviceAgentCoordinatorError.invalidMode)
            return
        }
        let generation = UUID()
        lifecycleID = generation
        executionPermit = IOSRunExecutionPermit()
        self.continuation = continuation
        guard let refreshed = try? await toolPlanSession.currentPlan(),
              await AgentCoreToolCatalog.isCurrent(refreshed), lifecycleID == generation else {
            if lifecycleID == generation { finish(throwing: DeviceAgentCoordinatorError.invalidToolPlan) }
            return
        }
        toolPlan = refreshed
        self.continuation = continuation
        activeSessionId = sessionId
        activeConfig = config
        recoveryInputOrigin = inputOrigin
        do {
            let recovery = try await canonicalApplication.recoverExactRun(permissionOnly: permissionOnly,
                prepareRecovery: { frozen in try await self.prepareRecoveredInvocation(frozen) })
            guard lifecycleID == generation else { return }
            switch recovery {
            case .transition(let transition): try await drive(transition, continuation: continuation)
            case .terminal(let seal):
                continuation.yield(.completed(seal.result))
                finish()
            }
        } catch {
            if lifecycleID == generation { finish(throwing: error) }
        }
    }

    static func permissionInstructionPayload(requestId: String, instruction: String) throws -> AgentCoreJSONValue {
        guard case .provideInstruction(let text)? = HostAgentPermissionDecision.validatedInstruction(instruction) else {
            throw DeviceAgentCoordinatorError.invalidPermissionEvent
        }
        return .object([
            "type": .string("permission_resolved"), "request_id": .string(requestId),
            "resolution": .object(["type": .string("intervention"), "intervention": .object([
                "type": .string("provide_instruction"), "instruction": .string(text),
            ])]),
        ])
    }

    public func providePermissionInstruction(requestId: String, instruction: String) async throws {
        guard let pending = pendingPermissions[requestId],
              HostAgentPermissionDecision.validatedInstruction(instruction, requestID: requestId, existing: pending.permissionInstructions) != nil,
              let envelope = pendingPermissionEnvelopes[requestId], (continuation != nil || childOnly),
              let owner = executionPermit else {
            throw DeviceAgentCoordinatorError.permissionNotPending
        }
        try owner.checkAdmission()
        let payload = try Self.permissionInstructionPayload(requestId: requestId, instruction: instruction)
        if pending.route == "core" {
            _ = try await submitHostResponse(envelope, payload: .object(["type": .string("execution_started")]))
        }
        let transition = try await submitHostResponse(envelope, payload: payload)
        pendingPermissions.removeValue(forKey: requestId)
        pendingPermissionEnvelopes.removeValue(forKey: requestId)
        // drive restores the successor ask from the canonical projection. No model submission is created here.
        if childOnly { await finishChildPermission(requestId: requestId) }
        else if let continuation { try await drive(transition, continuation: continuation) }
    }

    public func currentPermissionRequest(requestId: String) async throws -> IOSAgentPermissionRequest {
        guard let generation = lifecycleID, let original = pendingPermissions[requestId],
              let envelope = pendingPermissionEnvelopes[requestId] else {
            throw DeviceAgentCoordinatorError.permissionNotPending
        }
        if original.route == "core" {
            guard let refreshed = try corePermissionPresentation(envelope), refreshed == original,
                  lifecycleID == generation else { throw DeviceAgentCoordinatorError.invalidPermissionEvent }
            return refreshed
        }
        let plan = try await toolPlanSession.currentPlan()
        guard lifecycleID == generation, (continuation != nil || childOnly),
              pendingPermissionEnvelopes[requestId] == envelope,
              pendingPermissions[requestId] == original,
              let tool = plan.resolvedTool(id: original.toolId, route: original.route,
                hostId: plan.expectedIdentity.hostId, clientId: plan.expectedIdentity.clientId,
                sessionId: plan.expectedIdentity.sessionId),
              let refreshed = IOSAgentPermissionRequest(requestId: original.requestId, allowConversation: original.allowConversation, callId: original.callId,
                resolvedTool: tool, plan: plan, rationale: original.rationale, scope: original.scope,
                permissionInstructions: original.permissionInstructions, childOwner: original.childOwner) else {
            throw DeviceAgentCoordinatorError.invalidToolPlan
        }
        toolPlan = plan
        pendingPermissions[requestId] = refreshed
        return refreshed
    }

    public func resolvePermission(
        requestId: String,
        decision: AgentCorePermissionDecision,
        grant: AgentCoreJSONValue? = nil
    ) async throws {
        if decision != .deny { _ = try await currentPermissionRequest(requestId: requestId) }
        guard let pendingPermission = pendingPermissions[requestId],
              let envelope = pendingPermissionEnvelopes[requestId],
              (continuation != nil || childOnly), let owner = executionPermit else {
            throw DeviceAgentCoordinatorError.permissionNotPending
        }
        try owner.checkAdmission()
        continuation?.yield(.permissionResolved(
            requestId: requestId,
            toolName: pendingPermission.toolName,
            decision: decision
        ))
        do {
            if pendingPermission.route == "core" {
                _ = try await submitHostResponse(envelope, payload: .object(["type": .string("execution_started")]))
            }
            let transition = try await submitHostResponse(envelope, payload: .object([
                "type": .string("permission_resolved"),
                "request_id": .string(requestId),
                "resolution": .object([
                    "type": .string("authorization"),
                    "decision": .string(decision.rawValue),
                    "grant": grant ?? .null,
                ]),
            ]))
            pendingPermissions.removeValue(forKey: requestId)
            pendingPermissionEnvelopes.removeValue(forKey: requestId)
            if childOnly { await finishChildPermission(requestId: requestId) }
            else if let continuation { try await drive(transition, continuation: continuation) }
        } catch {
            if !childOnly, executionPermit === owner, cancellingOwner !== owner { finish(throwing: error) }
            throw error
        }
    }

    private func detachStream(_ streamID: UUID) {
        guard activeStreamID == streamID, cancellingOwner == nil else { return }
        // Losing an observer is not user Stop. End this process's delivery work;
        // Core retains the original Run/effects for fenced recovery after lease expiry.
        driverTask?.cancel()
        finish(throwing: CancellationError())
    }

    public func cancel(reason: String = "cancelled_by_user") async {
        guard cancellingOwner == nil else { return }
        let cancelledOwner = executionPermit
        cancelledOwner?.revoke()
        lifecycleID = nil
        guard let cancelledOwner, let sessionID = activeSessionId, let continuation else {
            finish()
            return
        }
        cancellingOwner = cancelledOwner
        let stoppedDriver = driverTask
        stoppedDriver?.cancel()
        do {
            let transition = try canonicalApplication.submitCancellation(reason: reason)
            // Observation ends immediately, including an uncooperative OS
            // authorization await. Retain the exact owner and deliveries until
            // its driver returns; stream EOF is not a Core terminal fact.
            continuation.finish()
            leaseHeartbeatTasks.values.forEach { $0.cancel() }
            leaseHeartbeatTasks.removeAll()
            if case .object(let fields) = transition,
               case .array(let requests)? = fields["host_requests"] {
                for request in requests { startLeaseHeartbeat(for: request) }
            }
            cancellationTask = Task {
                _ = try? await stoppedDriver?.value
                guard self.executionPermit === cancelledOwner else { return }
                do {
                    try await self.drainCancellation(transition, owner: cancelledOwner, sessionID: sessionID)
                    if self.executionPermit === cancelledOwner { self.finish() }
                } catch {
                    if self.executionPermit === cancelledOwner {
                        self.cancellationError = error
                        NSLog("[DeviceAgentCancellation] cancellation_control_unconfirmed")
                        self.finish(throwing: error)
                    }
                }
            }
        } catch {
            if executionPermit === cancelledOwner {
                cancellationError = error
                NSLog("[DeviceAgentCancellation] cancellation_submission_unconfirmed")
                finish(throwing: error)
            }
        }
    }

    /// Joins only an already requested cancellation; never starts or retries work.
    func waitForCancellation() async throws {
        await cancellationTask?.value
        if let cancellationError { throw cancellationError }
    }

    /// Only control requests from this Run's cancellation transition may run
    /// after revocation. No queued model/tool request crosses this boundary.
    private func drainCancellation(_ initial: AgentCoreJSONValue, owner: IOSRunExecutionPermit,
                                   sessionID: String) async throws {
        var transition = initial
        while true {
            guard executionPermit === owner, cancellingOwner === owner,
                  activeSessionId == sessionID,
                  case .object(let fields) = transition else { throw CancellationError() }
            if let control = try Self.canonicalControlProjection(fields) {
                let snapshot = try clientSnapshot(from: control)
                guard snapshot.sessionId == sessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
            }
            guard case .array(let requests)? = fields["host_requests"] else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
            if requests.isEmpty {
                guard try canonicalApplication.committedTerminalSeal(coreSessionID: sessionID) != nil else {
                    throw DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed
                }
                return
            }
            var successors: [AgentCoreJSONValue] = []
            for request in requests {
                guard executionPermit === owner,
                      case .object(let envelope) = request,
                      envelope.string("session_id") == sessionID,
                      case .object(let payload)? = envelope["payload"],
                      let type = payload.string("type") else { throw DeviceAgentCoordinatorError.invalidTransition }
                guard envelope.string("turn_id") == activeTurnId
                    || (type == "run_lifecycle_hook" && envelope.string("turn_id") == nil) else {
                    throw DeviceAgentCoordinatorError.invalidTransition
                }
                var response: AgentCoreJSONValue
                switch type {
                case "cancel":
                    let target = try confirmCancellationTarget(envelope)
                    response = .object(["type": .string("cancellation_confirmed"), "target_effect_id": .string(target)])
                case "cancel_task":
                    // Core requires ExecutionStarted before persisting intent.
                    response = .null
                case "run_lifecycle_hook":
                    response = .object(["type": .string("completed"),
                        "output": .object(["lifecycle": .string("session_end")]),
                        "usage": .null, "full_output_ref": .null, "artifacts": .array([])])
                default: throw DeviceAgentCoordinatorError.unsupportedHostRequest(type)
                }
                let started = try await submitHostResponse(request, payload: .object(["type": .string("execution_started")]))
                if case .object(let startedFields) = started, case .array(let next)? = startedFields["host_requests"] {
                    successors.append(contentsOf: next)
                }
                if type == "cancel_task" { response = try cancelTaskResponse(request) }
                let settled = try await submitHostResponse(request, payload: response)
                if case .object(let settledFields) = settled, case .array(let next)? = settledFields["host_requests"] {
                    successors.append(contentsOf: next)
                }
            }
            transition = .object(["host_requests": .array(successors)])
        }
    }

    private func retainChildModelStop(effectID: String, request: AgentModelRequest) throws {
        guard childOnly else { return }
        guard let owner = childPermissionOwner, owner.runID == request.turnId,
              owner.coreSessionID == request.sessionId else { throw DeviceAgentCoordinatorError.invalidTransition }
        guard let cancellable = modelClient as? AgentModelCancellationClient else { return }
        childModelStops[effectID] = .init(conversationID: owner.childConversationID, runID: owner.runID,
            coreSessionID: owner.coreSessionID, stop: { try await cancellable.cancelAgentTurn(request: request) })
    }

    /// A consumer may have ended on an unconfirmed mailbox ACK while its
    /// independent provider is still live. Join that exact producer, not just
    /// an already-finished operation Task, before issuing a stop witness.
    func stopChildPhysicalModel(effectID: String, conversationID: String, runID: String, coreSessionID: String) async throws {
        guard childOnly, let owner = childPermissionOwner,
              owner.childConversationID == conversationID, owner.runID == runID,
              owner.coreSessionID == coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
        if locallyQuiescentEffects.contains(effectID), !unconfirmedPhysicalEffects.contains(effectID) { return }
        guard let operation = childModelStops[effectID] else { return }
        guard operation.conversationID == conversationID, operation.runID == runID,
              operation.coreSessionID == coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
        let stop: Task<Void, Error>
        if let existing = childModelStopTasks[effectID] {
            guard existing.id == operation.id else { throw DeviceAgentCoordinatorError.invalidTransition }
            stop = existing.task
        } else {
            stop = Task { try await operation.stop() }
            childModelStopTasks[effectID] = (operation.id, stop)
        }
        defer {
            if childModelStopTasks[effectID]?.id == operation.id { childModelStopTasks.removeValue(forKey: effectID) }
        }
        try await stop.value
        if childModelStops[effectID]?.id == operation.id {
            childModelStops.removeValue(forKey: effectID)
            unconfirmedPhysicalEffects.remove(effectID)
            locallyQuiescentEffects.insert(effectID)
        } else {
            guard childModelStops[effectID] == nil, locallyQuiescentEffects.contains(effectID),
                  !unconfirmedPhysicalEffects.contains(effectID) else { throw DeviceAgentCoordinatorError.invalidTransition }
        }
    }

    func stopChildPhysicalModels(conversationID: String, runID: String, coreSessionID: String) async throws {
        var firstFailure: Error?
        for effect in Array(childModelStops.keys) {
            do { try await stopChildPhysicalModel(effectID: effect, conversationID: conversationID,
                runID: runID, coreSessionID: coreSessionID) }
            catch { if firstFailure == nil { firstFailure = error } }
        }
        if let firstFailure { throw firstFailure }
    }

    private func confirmCancellationTarget(_ envelope: [String: AgentCoreJSONValue]) throws -> String {
        let (target, admitted) = try IOSCancellationAdmissionContract.target(envelope: envelope,
            conversationID: canonicalApplication.conversationID)
        guard !unconfirmedPhysicalEffects.contains(target) else {
            throw DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed
        }
        if !admitted && !locallyQuiescentEffects.contains(target) {
            let confirmed = try canonicalApplication.observeTerminatedCancellationTarget(control: .object(envelope))
            guard confirmed == target else { throw DeviceAgentCoordinatorError.invalidTransition }
            locallyQuiescentEffects.insert(target)
        }
        return target
    }

    private func prepareRecoveredInvocation(_ frozen: IOSExactRunInvocation) async throws {
        recoveryInputOrigin = try canonicalApplication.loadFrozenInputOrigin(frozen)
        guard let recoveryPreparation, preparedHostInvocation != nil, let owner = executionPermit,
              let generation = lifecycleID, let config = activeConfig,
              frozen.coreSessionID == activeSessionId else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        try owner.checkAdmission()
        try await recoveryPreparation(frozen)
        try owner.checkAdmission()
        let refreshed = try await toolPlanSession.currentPlan()
        guard await AgentCoreToolCatalog.isCurrent(refreshed), executionPermit === owner,
              lifecycleID == generation else {
            throw DeviceAgentCoordinatorError.invalidToolPlan
        }
        try owner.checkAdmission()
        toolPlan = refreshed
        let candidate = try makeRuntimeConfig(sessionId: activeSessionId, history: [],
            inputOrigin: recoveryInputOrigin, config: config, captureDelegation: false)
        try canonicalApplication.validateRecoveryAuthority(frozen, candidate: candidate)
        if (delegationSettings != nil || expertTargetConfiguration != nil), localTaskHost == nil,
           let invocation = preparedHostInvocation, let store = canonicalApplication.embeddedStore {
            localTaskHost = IOSLocalTaskHost(store: store, modelClient: modelClient,
                invocation: invocation, toolPlan: toolPlan, hostExecutor: hostExecutor,
                config: config, childPermissionEvents: childPermissionEvents, stateChanged: childTaskStateChanged, dependencies: hostDependencyIdentity, expertTargetConfiguration: expertTargetConfiguration)
        }
        // The old lease can keep recovery outside drive(). Preserve Core's
        // validated turn owner now so a Stop during that wait validates the
        // cancellation envelopes against the recovered Run, not a nil turn.
        guard activeTurnId == nil || activeTurnId == frozen.runID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        activeTurnId = frozen.runID
        try await localTaskHost?.recoverOwnedTasks(parentConversationID: canonicalApplication.conversationID)
        guard recoveryTraceRunID == nil || recoveryTraceRunID == frozen.runID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        recoveryTraceRunID = frozen.runID
        for trace in try canonicalApplication.recoveryTraceEvents(afterSequence: Int64(recoveryTraceSequence)) {
            continuation?.yield(.trace(trace))
            recoveryTraceSequence = trace.sequence
        }
    }

    public func cancelTask(_ taskId: String) async throws {
        guard activeSessionId != nil, let continuation,
              let task = taskProjection.projection.tasks[taskId],
              !task.state.isTerminal,
              task.ownerSessionId == activeSessionId else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let transition = try await submitCoreSubmission(.object([
            "type": .string("request_task_cancellation"),
            "task_id": .string(taskId),
        ]))
        try await drive(transition, continuation: continuation)
    }

    public func sendTaskGuidance(_ taskId: String, content: String) async throws {
        let content = content.trimmingCharacters(in: .whitespacesAndNewlines)
        guard activeSessionId != nil, continuation != nil,
              let hostId = activeConfig?.hostId, !hostId.isEmpty,
              !content.isEmpty,
              let task = taskProjection.projection.tasks[taskId],
              task.kind == .agent, !task.state.isTerminal, !task.cancelRequested,
              task.ownerSessionId == activeSessionId,
              task.ownerTurnId == activeTurnId else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let application = canonicalApplication
        _ = try await AppleTaskGuidanceOutbox.shared.submit(
            hostId: hostId, conversationId: task.ownerSessionId,
            parentRunId: application.runID, taskId: taskId, message: content
        ) { path, payload in
            try application.taskGuidanceTransport(path: path, payload: payload, parentTurnID: task.ownerTurnId)
        }
    }

    /// Executes one physical request for Core's independent child pump. This
    /// instance never admits a foreground turn or runs the Swift drain loop.
    func executeChildHostRequest(
        _ request: AgentCoreJSONValue,
        conversationID: String,
        runID: String,
        coreSessionID: String,
        config: AgentCoreRuntimeConfig,
        stopTarget: @escaping @Sendable (String) async throws -> Void,
        committedResponse: @escaping @Sendable (AgentCoreJSONValue) async throws -> Void
    ) async throws {
        guard childOnly, activeStreamID == nil,
              let envelope = request.objectValue,
              envelope.string("turn_id") == runID,
              envelope.string("session_id") == coreSessionID,
              childPermissionOwner?.childConversationID == conversationID,
              childPermissionOwner?.runID == runID,
              childPermissionOwner?.coreSessionID == coreSessionID,
              let effectID = envelope.string("effect_id"),
              let payload = envelope["payload"]?.objectValue,
              let kind = payload.string("type"), childResponseSinks[effectID] == nil else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        if let childOwner {
            guard childOwner.conversation == conversationID, childOwner.run == runID else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
        } else {
            childOwner = (conversationID, runID)
            activeSessionId = coreSessionID
            activeTurnId = runID
            activeConfig = config
            lifecycleID = UUID()
            executionPermit = IOSRunExecutionPermit()
        }
        childResponseSinks[effectID] = committedResponse
        responseSequences[effectID] = envelope.int("resume_after_sequence") ?? 0
        responseAttemptIDs.removeValue(forKey: effectID)
        defer { childResponseSinks.removeValue(forKey: effectID) }
        if let target = expertTaskTarget {
            try target.validate(taskID: childPermissionOwner?.taskID ?? "", contextID: conversationID, runID: runID)

        }
        // Showing a Core question has no Host side effect. Device permission
        // prompts still require the execution barrier before contacting the OS.
        if try kind != "request_permission" || corePermissionPresentation(request) == nil {
            _ = try await submitHostResponse(request, payload: .object(["type": .string("execution_started")]))
        }
        locallyQuiescentEffects.remove(effectID)
        let (_, observations) = AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error>.makeStream()
        defer { observations.finish() }
        switch kind {
        case "request_permission":
            guard childPermissionEvents != nil, let requestID = payload.string("permission_request_id") else {
                throw DeviceAgentCoordinatorError.invalidPermissionEvent
            }
            pendingPermissionEnvelopes[requestID] = request
            // Like foreground approval, no tool operation has started while
            // this UI question is pending; cancellation joins its receiver.
            locallyQuiescentEffects.insert(effectID)
            try await registerPermissionRequest(payload)
            guard let pending = pendingPermissions[requestID] else { throw DeviceAgentCoordinatorError.invalidPermissionEvent }
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { waiter in
                    childPermissionWaiters[requestID] = waiter
                    Task { await childPermissionEvents?(.requested(pending, self)) }
                }
            } onCancel: { Task { await self.cancelChildPermission(requestID: requestID) } }
        case "cancel_task":
            throw DeviceAgentCoordinatorError.unsupportedHostRequest(kind)
        case "model_stream": _ = try await runModel(request, continuation: observations)
        case "execute_tool": _ = try await executeTool(request)
        case "compact": _ = try await compact(request, continuation: observations)
        case "fetch_context":
            _ = try await submitHostResponse(request, payload: .object([
                "type": .string("context_fetched"), "components": .array([]),
            ]))
        case "run_lifecycle_hook":
            _ = try await submitHostResponse(request, payload: .object([
                "type": .string("completed"), "output": .object(["lifecycle": .string("session_end")]),
                "usage": .null, "full_output_ref": .null, "artifacts": .array([]),
            ]))
        case "cancel":
            let (requestedTarget, _) = try IOSCancellationAdmissionContract.target(
                envelope: envelope, conversationID: conversationID)
            try await stopTarget(requestedTarget)
            let target = try confirmCancellationTarget(envelope)
            _ = try await submitHostResponse(request, payload: .object([
                "type": .string("cancellation_confirmed"), "target_effect_id": .string(target),
            ]))
        default:
            // Unsupported physical requests stay unconfirmed; transport closure
            // cannot manufacture a permission decision or cancellation proof.
            throw DeviceAgentCoordinatorError.unsupportedHostRequest(kind)
        }
    }

    private func finishChildPermission(requestId: String) async {
        let waiter = childPermissionWaiters.removeValue(forKey: requestId)
        if let owner = childPermissionOwner { await childPermissionEvents?(.removed(owner, requestId, self)) }
        waiter?.resume()
    }

    private func cancelChildPermission(requestID: String) async {
        pendingPermissions.removeValue(forKey: requestID)
        pendingPermissionEnvelopes.removeValue(forKey: requestID)
        let waiter = childPermissionWaiters.removeValue(forKey: requestID)
        if let owner = childPermissionOwner { await childPermissionEvents?(.removed(owner, requestID, self)) }
        waiter?.resume(throwing: CancellationError())
    }

    private func begin(
        sessionId: String?,
        input: String,
        history: [AgentCoreMessage],
        inputOrigin: AgentCoreInputOrigin,
        contextComponents: [AgentCoreContextComponent],
        turnIdentity: IOSCanonicalAgentTurnIdentity?,
        config: AgentCoreRuntimeConfig,
        streamID: UUID,
        continuation: StreamContinuation
    ) async {
        guard activeStreamID == streamID else { return }
        guard config.executionMode == .deviceLocal else {
            finish(throwing: DeviceAgentCoordinatorError.invalidMode)
            return
        }
        let generation = UUID()
        lifecycleID = generation
        executionPermit = IOSRunExecutionPermit()
        self.continuation = continuation
        guard let refreshed = try? await toolPlanSession.currentPlan(),
              await AgentCoreToolCatalog.isCurrent(refreshed), lifecycleID == generation else {
            if lifecycleID == generation { finish(throwing: DeviceAgentCoordinatorError.invalidToolPlan) }
            return
        }
        toolPlan = refreshed
        self.continuation = continuation
        activeConfig = config
        do {
            let stableSessionId = sessionId?.trimmingCharacters(in: .whitespacesAndNewlines)
                .nonEmpty ?? UUID().uuidString.lowercased()
            activeSessionId = stableSessionId
            var runtimeConfig = try makeRuntimeConfig(
                sessionId: stableSessionId,
                history: history,
                inputOrigin: inputOrigin,
                config: config
            )
            if (delegationSettings != nil || expertTargetConfiguration != nil), let invocation = preparedHostInvocation,
               let store = canonicalApplication.embeddedStore {
                localTaskHost = IOSLocalTaskHost(store: store, modelClient: modelClient,
                    invocation: invocation, toolPlan: toolPlan, hostExecutor: hostExecutor,
                    config: config, childPermissionEvents: childPermissionEvents, stateChanged: childTaskStateChanged, dependencies: hostDependencyIdentity, expertTargetConfiguration: expertTargetConfiguration)
            }
            let submission = try AgentCoreInputSubmission.make(
                input: input,
                origin: inputOrigin,
                contextComponents: contextComponents,
                messageId: turnIdentity?.userMessageID
                    ?? "ios-user:\(UUID().uuidString.lowercased())",
                submissionId: turnIdentity?.submissionID
                    ?? UUID().uuidString.lowercased()
            )
            if let target = freshResendTarget {
                guard history.isEmpty, stableSessionId == target, toolPlan.scopedGrants.isEmpty,
                      let store = canonicalApplication.embeddedStore,
                      case .object(var finalConfig) = runtimeConfig,
                      case .object(var identity)? = finalConfig["turn_identity"] else {
                    throw IOSApplicationRuntimeError.rejected("Edited resend requires a fresh target tool plan")
                }
                // Attest this exact, fully constructed user submission. The same
                // value enters Begin; no renderer input or local hash is trusted.
                let witness = try store.attestEditedResendHostPolicy(conversationID: target,
                    coreConfig: runtimeConfig, submission: submission)
                identity["fresh_resend_policy"] = witness
                finalConfig["turn_identity"] = .object(identity)
                runtimeConfig = .object(finalConfig)
            }
            let transition = try canonicalApplication.begin(
                coreConfig: runtimeConfig,
                submission: submission,
                createdAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
            try await drive(transition, continuation: continuation)
        } catch {
            if lifecycleID == generation { finish(throwing: error) }
        }
    }

    private func drive(
        _ initialTransition: AgentCoreJSONValue,
        continuation: StreamContinuation
    ) async throws {
        guard let owner = executionPermit else { throw CancellationError() }
        guard cancellingOwner !== owner else { throw CancellationError() }
        if drivingOwner === owner {
            try await acceptTransition(initialTransition, continuation: continuation)
            return
        }
        drivingOwner = owner
        let task = Task {
            try await self.acceptTransition(initialTransition, continuation: continuation)
            try await self.drainHostRequests(owner: owner, continuation: continuation)
        }
        driverTask = task
        defer {
            if drivingOwner === owner { drivingOwner = nil; driverTask = nil }
        }
        try await task.value
    }

    private func drainHostRequests(owner: IOSRunExecutionPermit, continuation: StreamContinuation) async throws {
        while !Task.isCancelled {
            guard executionPermit === owner else { throw CancellationError() }
            if let policy = try canonicalApplication.applyPendingPermissionPolicy() {
                try await acceptTransition(policy, continuation: continuation)
            }
            guard !queuedHostRequests.isEmpty else {
                if !pendingPermissions.isEmpty {
                    return
                }
                if let sessionID = activeSessionId,
                   let seal = try canonicalApplication.committedTerminalSeal(coreSessionID: sessionID) {
                    // A sealed Run is a completed execution, not necessarily a
                    // completed task. Preserve its business result for consumers.
                    continuation.yield(.completed(seal.result))
                    finish()
                    return
                }
                throw DeviceAgentCoordinatorError.missingTransition
            }
            let request = queuedHostRequests.removeFirst()
            guard case .object(let envelope) = request,
                  case .object(let payload)? = envelope["payload"],
                  let requestType = payload.string("type") else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
            let executableRequestTypes: Set<String> = [
                "model_stream", "execute_tool", "compact",
                "fetch_context", "cancel", "cancel_task", "run_lifecycle_hook",
                "request_permission",
                "resolve_permission_resources",
            ]
            guard executableRequestTypes.contains(requestType) else {
                throw DeviceAgentCoordinatorError.unsupportedHostRequest(requestType)
            }
            if requestType == "cancel" {
                _ = try confirmCancellationTarget(envelope)
            }
            // A redelivery is not the previous completed invocation. Revoke
            // its old stop credit before the new dispatch can start/await.
            if let effectID = envelope.string("effect_id") { locallyQuiescentEffects.remove(effectID) }
            if try requestType != "request_permission" || corePermissionPresentation(request) == nil {
                let started = try await submitHostResponse(
                    request, payload: .object(["type": .string("execution_started")]))
                guard executionPermit === owner else { throw CancellationError() }
                try await acceptTransition(started, continuation: continuation)
                guard executionPermit === owner else { throw CancellationError() }
            }
            startLeaseHeartbeat(for: request)
            if requestType == "request_permission" {
                guard let permissionID = payload.string("permission_request_id") else {
                    throw DeviceAgentCoordinatorError.invalidPermissionEvent
                }
                pendingPermissionEnvelopes[permissionID] = request
                if let effectID = envelope.string("effect_id") { locallyQuiescentEffects.insert(effectID) }
                try await registerPermissionRequest(payload)
                return
            }
            let next: AgentCoreJSONValue?
            switch requestType {
            case "model_stream":
                next = try await runModel(request, continuation: continuation)
            case "execute_tool":
                next = try await executeTool(request)
            case "resolve_permission_resources":
                next = try await submitHostResponse(request, payload: .object([
                    "type": .string("failed"),
                    "error": .object([
                        "code": .string("permission_resource_resolution_unsupported"),
                        "message": .string("This iOS Host cannot resolve the declared resource plan. The operation was not dispatched."),
                        "retryable": .bool(false),
                    ]),
                ]))
            case "compact":
                next = try await compact(request, continuation: continuation)
            case "fetch_context":
                next = try await submitHostResponse(
                    request,
                    payload: .object([
                        "type": .string("context_fetched"),
                        "components": .array([]),
                    ])
                )
            case "cancel":
                let targetEffectId = try confirmCancellationTarget(envelope)
                next = try await submitHostResponse(
                    request,
                    payload: .object([
                        "type": .string("cancellation_confirmed"),
                        "target_effect_id": .string(targetEffectId),
                    ])
                )
            case "cancel_task":
                next = try await submitHostResponse(request, payload: cancelTaskResponse(request))
            case "run_lifecycle_hook":
                next = try await submitHostResponse(
                    request,
                    payload: .object([
                        "type": .string("completed"),
                        "output": .object(["lifecycle": .string("session_end")]),
                        "usage": .null,
                        "full_output_ref": .null,
                        "artifacts": .array([]),
                    ])
                )
            default:
                preconditionFailure("validated Host request type missing from execution switch")
            }
            if let next {
                guard executionPermit === owner else { throw CancellationError() }
                try await acceptTransition(next, continuation: continuation)
            }
        }
    }

    private func acceptTransition(
        _ transition: AgentCoreJSONValue,
        continuation: StreamContinuation
    ) async throws {
        try Task.checkCancellation()
        if childOnly {
            // Child ACKs carry no projection. The shared driver publishes the
            // canonical journal; this physical executor has no second reducer.
            guard transition == .null else { throw DeviceAgentCoordinatorError.invalidTransition }
            return
        }
        guard let owner = executionPermit else { throw CancellationError() }
        guard case .object(let object) = transition else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let control = try Self.canonicalControlProjection(object)
        let decoded = try control.map { try clientSnapshot(from: $0) }
        if let taskValue = object["task_projection"] {
            guard let sessionID = activeSessionId, decoded?.sessionId == sessionID else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
            let restored = try JSONDecoder().decode(AgentCoreTaskProjection.self, from: JSONEncoder().encode(taskValue))
            guard taskProjection.restore(restored, expectedSessionId: sessionID) == .applied else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
        }
        if case .array(let publications)? = object["assistant_publications"] {
            for cursor in publications { continuation.yield(.assistantJournalCommitted(cursor)) }
        }
        if let control, let decoded {
            activeTurnId = decoded.turnId ?? activeTurnId
            continuation.yield(.snapshot(decoded))
            try await restorePermissionProjection(from: control, continuation: continuation)
            guard executionPermit === owner else { throw CancellationError() }
        }
        if object["task_projection"] != nil {
            continuation.yield(.taskProjectionUpdated(taskProjection.projection))
        }
        if case .array(let envelopes)? = object["event_envelopes"] {
            for value in envelopes {
                if let activation = AgentCoreCapabilityActivationEnvelope.decode(from: value),
                   activation.sessionID == activeSessionId,
                   activeTurnId == nil || activation.turnID == activeTurnId,
                   activation.cursor > capabilityActivationCursor {
                    capabilityActivationCursor = activation.cursor
                    continuation.yield(.capabilitiesActivated(activation))
                }
                if let skillEvent = AgentCoreSkillRuntimeUpdateEnvelope.decode(from: value),
                   let sessionId = activeSessionId,
                   let turnId = activeTurnId,
                   skillRuntimeProjection.apply(
                        skillEvent,
                        expectedSessionID: sessionId,
                        expectedTurnID: turnId
                   ) == .applied {
                    continuation.yield(.skillRuntimeProjectionUpdated(skillRuntimeProjection.projection))
                }
                if let taskEvent = AgentCoreTaskEventEnvelope.decodeCore(from: value),
                   let sessionId = activeSessionId,
                   let turnId = activeTurnId,
                   taskProjection.apply(
                        taskEvent,
                        expectedSessionId: sessionId,
                        expectedTurnId: turnId
                   ) == .applied {
                    continuation.yield(.taskProjectionUpdated(taskProjection.projection))
                }
                if let terminal = AgentCoreTurnTerminalEnvelope.decodeCore(from: value) {
                    continuation.yield(.coreTurnTerminal(terminal))
                    continue
                }
            }
        }
        if case .array(let traces)? = object["trace_events"] {
            let encoder = JSONEncoder()
            let decoder = JSONDecoder()
            for value in traces {
                let trace = try decoder.decode(
                    AgentTraceEvent.self,
                    from: encoder.encode(value)
                )
                // Recovery has already replayed committed diagnostics through
                // its pinned frontier; an overlapping transition is a replay.
                if trace.runId == recoveryTraceRunID, trace.sequence <= recoveryTraceSequence { continue }
                continuation.yield(.trace(trace))
                if trace.runId == recoveryTraceRunID { recoveryTraceSequence = trace.sequence }
            }
        }
        if case .array(let requests)? = object["host_requests"] {
            queuedHostRequests.append(contentsOf: requests)
        }
    }

    private func runModel(
        _ request: AgentCoreJSONValue,
        continuation: StreamContinuation
    ) async throws -> AgentCoreJSONValue {
        guard case .object(let envelope) = request,
              case .object(let payload)? = envelope["payload"],
              let sessionId = envelope.string("session_id"),
              let turnId = envelope.string("turn_id"),
              let initialTextPartId = payload.string("text_part_id"),
              let initialReasoningPartId = payload.string("reasoning_part_id") else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        modelIteration += 1
        let messages = modelMessages(from: payload.array("messages") ?? [])
        let tools = modelSupportsToolCalls ? (payload.array("tools") ?? []) : []
        let settings = payload.object("settings") ?? [:]
        let contextWindow = settings.int("context_window_tokens")
            ?? activeConfig?.contextWindowTokens
            ?? 128_000
        let promptLimit = settings.int("prompt_token_limit")
            ?? max(1, contextWindow - (activeConfig?.reservedOutputTokens ?? 4_096))
        // Only Core's complete model-prefix identity is valid here. Falling
        // back to the system-only digest would silently reuse a cache across
        // different Step tool projections or model invocation policies.
        let stablePrefixSHA256 = settings.string("model_prefix_digest")
        let invocation = settings.object("model_invocation_plan") ?? [:]
        guard let admittedOutput = invocation.int("effective_max_output_tokens"),
              (64...1_000_000).contains(admittedOutput) else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let modelRequest = AgentModelRequest(
            sessionId: sessionId,
            turnId: turnId,
            iteration: modelIteration,
            messages: messages,
            tools: tools,
            reservedOutputTokens: max(256, contextWindow - promptLimit),
            stablePrefixSHA256: stablePrefixSHA256,
            invocationPlan: .init(
                maxOutputTokens: admittedOutput,
                nativeToolCalls: invocation.bool("native_tool_calls") ?? false,
                parallelToolCalls: invocation.bool("parallel_tool_calls") ?? false,
                reasoning: invocation.bool("reasoning") ?? false,
                promptCache: invocation.bool("prompt_cache") ?? false,
                streamText: invocation.bool("stream_text") ?? false,
                streamReasoning: invocation.bool("stream_reasoning") ?? false,
                streamToolCalls: invocation.bool("stream_tool_calls") ?? false
            ),
            providerContinuation: invocation["provider_continuation"]
        )
        var textPartId = initialTextPartId
        var reasoningPartId = initialReasoningPartId
        var completion: AgentModelCompletion?
        var streamedText = ""
        var streamedReasoning = ""
        var emittedToolCall = false
        var attempt = 1
        while completion == nil {
            do {
                let attemptID: String
                let replacingAttempt = attempt > 1 || envelope.bool("attempt_started") == true
                if !replacingAttempt {
                    guard let preparedAttemptID = envelope.string("attempt_id") else {
                        throw DeviceAgentCoordinatorError.invalidTransition
                    }
                    attemptID = preparedAttemptID
                } else {
                    attemptID = UUID().uuidString.lowercased()
                    textPartId = "\(attemptID):text"
                    reasoningPartId = "\(attemptID):reasoning"
                }
                let admitted = try await submitHostResponse(
                    request,
                    payload: .object([
                        "type": .string("model_attempt_started"),
                        "provider": .string(activeConfig?.executionMode.rawValue ?? "device_local"),
                        "model": .string(activeConfig?.model ?? "unknown"),
                        "reason": .string(replacingAttempt ? "retry_or_fallback" : "initial"),
                    ]),
                    attemptID: attemptID
                )
                try await acceptTransition(admitted, continuation: continuation)
                try Task.checkCancellation()
                if let effectID = envelope.string("effect_id") {
                    unconfirmedPhysicalEffects.insert(effectID)
                    try retainChildModelStop(effectID: effectID, request: modelRequest)
                }
                let modelEvents = await modelClient.generateAgentTurn(request: modelRequest)
                for try await event in modelEvents {
                    try Task.checkCancellation()
                    switch event {
                    case .textDelta(let delta):
                        streamedText += delta
                        let transition = try await submitHostResponse(
                            request,
                            payload: .object([
                                "type": .string("model_delta"),
                                "channel": .string("text"),
                                "part_id": .string(textPartId),
                                "delta": .string(delta),
                            ])
                        )
                        try await acceptTransition(transition, continuation: continuation)
                    case .reasoningDelta(let delta):
                        streamedReasoning += delta
                        let transition = try await submitHostResponse(
                            request,
                            payload: .object([
                                "type": .string("model_delta"),
                                "channel": .string("reasoning"),
                                "part_id": .string(reasoningPartId),
                                "delta": .string(delta),
                            ])
                        )
                        try await acceptTransition(transition, continuation: continuation)
                    case .toolCallDelta(_, let callId, let capabilityId, let argumentsDelta):
                        emittedToolCall = true
                        let transition = try await submitHostResponse(
                            request,
                            payload: .object([
                                "type": .string("model_tool_call_delta"),
                                // Provider call IDs are only scoped to one
                                // response. The committed attempt owns Parts.
                                "part_id": .string("\(attemptID):tool:\(callId)"),
                                "call_id": .string(callId),
                                "capability_id": .string(capabilityId),
                                "arguments_delta": .string(argumentsDelta),
                            ])
                        )
                        try await acceptTransition(transition, continuation: continuation)
                    case .completed(let value):
                        completion = value
                        if let effectID = envelope.string("effect_id") {
                            childModelStops.removeValue(forKey: effectID)
                            unconfirmedPhysicalEffects.remove(effectID)
                            locallyQuiescentEffects.insert(effectID)
                        }
                    }
                }
                if completion == nil {
                    throw DeviceAgentCoordinatorError.modelEndedWithoutCompletion
                }
            } catch {
                if error is CancellationError || Task.isCancelled {
                    if childOnly, let effect = envelope.string("effect_id"), let owner = childPermissionOwner {
                        try await stopChildPhysicalModel(effectID: effect, conversationID: owner.childConversationID,
                            runID: owner.runID, coreSessionID: owner.coreSessionID)
                    } else if let cancellable = modelClient as? AgentModelCancellationClient {
                        try await cancellable.cancelAgentTurn(request: modelRequest)
                        if let effectID = envelope.string("effect_id") {
                            childModelStops.removeValue(forKey: effectID)
                            unconfirmedPhysicalEffects.remove(effectID)
                            locallyQuiescentEffects.insert(effectID)
                        }
                    }
                    throw CancellationError()
                }
                let retryable = Self.isRetryableModelFailure(error)
                let canRetry = retryable && attempt < 3 && streamedText.isEmpty &&
                    streamedReasoning.isEmpty && !emittedToolCall
                guard canRetry else {
                    return try await failedHostResponse(request, error: error)
                }
                let delayMilliseconds = 2_000 * (1 << (attempt - 1))
                let transition = try await submitHostResponse(
                    request,
                    payload: .object([
                        "type": .string("tool_progress"),
                        "title": .string("model_retry"),
                        "detail": .string("attempt=\(attempt) delay_ms=\(delayMilliseconds)"),
                        "metadata": .object([
                            "attempt": .number(Double(attempt)),
                            "delay_ms": .number(Double(delayMilliseconds)),
                        ]),
                    ])
                )
                try await acceptTransition(transition, continuation: continuation)
                try await Task.sleep(nanoseconds: UInt64(delayMilliseconds) * 1_000_000)
                attempt += 1
            }
        }
        guard let completion else {
            return try await failedHostResponse(
                request,
                error: DeviceAgentCoordinatorError.modelEndedWithoutCompletion
            )
        }
        if let providerContinuation = completion.providerContinuation {
            let transition = try await submitHostResponse(
                request,
                payload: .object([
                    "type": .string("model_continuation"),
                    "continuation": providerContinuation,
                ])
            )
            try await acceptTransition(transition, continuation: continuation)
        }
        let finalText = completion.content.isEmpty ? streamedText : completion.content
        let finalReasoning = completion.reasoning.isEmpty
            ? streamedReasoning
            : completion.reasoning
        var items: [AgentCoreJSONValue] = [
            .object([
                "type": .string("message"),
                "id": .string(textPartId),
                "role": .string("assistant"),
                "content": .string(finalText),
            ]),
        ]
        if !finalReasoning.isEmpty {
            items.append(.object([
                "type": .string("reasoning"),
                "id": .string(reasoningPartId),
                "summary": .string(""),
                "content": .string(finalReasoning),
            ]))
        }
        let toolCalls = completion.toolCalls.enumerated().map { index, call in
            AgentCoreJSONValue.object([
                "call_id": .string(
                    call.id.isEmpty
                        ? "\(modelRequest.turnId)-\(modelRequest.iteration)-tool-call-\(index)"
                        : call.id
                ),
                "name": .string(call.name),
                "arguments": call.arguments,
                "permission_scope": .null,
                "permission_rationale": .null,
            ])
        }
        let responseType = completion.toolCalls.isEmpty
            && isModelOutputLimitFinishReason(completion.finishReason)
            ? "model_output_limited"
            : "model_completed"
        return try await submitHostResponse(
            request,
            payload: .object([
                "type": .string(responseType),
                "items": .array(items),
                "tool_calls": .array(toolCalls),
                "usage": Self.v2Usage(completion.usage),
            ])
        )
    }

    private func refreshActiveToolPlan() async throws -> AgentCorePreparedToolPlan {
        guard let generation = lifecycleID, (continuation != nil || childOnly), activeSessionId != nil else {
            throw CancellationError()
        }
        let plan = try await toolPlanSession.currentPlan()
        guard lifecycleID == generation, (continuation != nil || childOnly), activeSessionId != nil else {
            throw CancellationError()
        }
        return plan
    }

    nonisolated static func expertPreparationFailureResponse(callID: String, error: Error) throws -> AgentCoreJSONValue {
        if case IOSApplicationRuntimeError.outcomePending = error { throw error }
        if error is CancellationError { throw error }
        if let preparation = error as? IOSExpertTaskPreparationError, preparation.taskMayExist { throw error }
        var output: [String: AgentCoreJSONValue] = ["error": .string(error.localizedDescription)]
        if let preparation = error as? IOSExpertTaskPreparationError {
            output["exchange_id"] = .string(preparation.exchangeID)
            output["preparation_aborted"] = .bool(preparation.preparationAborted)
            output["execution_started"] = .bool(false)
            output["recovery_actions"] = .array(preparation.preparationAborted ? [.string("new_request")] : [.string("retry_request"), .string("cancel")])
        }
        return .object(["type": .string("tool_completed"), "call_id": .string(callID),
            "output": Self.toolResult(.object(output), state: "not_started"), "outcome": .string("failed"),
            "full_output_ref": .null, "artifacts": .array([])])
    }

    private func cancelTaskResponse(_ request: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard let effectID = request.objectValue?.string("effect_id") else { throw DeviceAgentCoordinatorError.invalidTransition }
        return try canonicalApplication.taskCancellationResponse(request: request,
            delivery: responseDeliveries[effectID], nowMs: Int64(Date().timeIntervalSince1970 * 1_000))
    }

    private func executeTool(_ request: AgentCoreJSONValue) async throws -> AgentCoreJSONValue {
        if childOnly {
            guard toolPlan.expectedIdentity.sessionId == childOwner?.conversation else {
                throw DeviceAgentCoordinatorError.invalidToolPlan
            }
        }
        let observedEffectID = request.objectValue?.string("effect_id")
        var enteredExecutor = false
        defer {
            // This invocation's own preflight returned without entering the
            // native handler. Unlike a cold empty registry, this is positive
            // local evidence; cancellation drains only after driver return.
            if !enteredExecutor, let observedEffectID { locallyQuiescentEffects.insert(observedEffectID) }
        }
        if let envelope = request.objectValue, let payload = envelope["payload"]?.objectValue,
           ["delegate_expert", "expert_exchange"].contains(payload.string("capability_id") ?? "") {
            if childOnly {
                // Recovery only: no new leaf advertises the alias. Core checks
                // the original started effect and existing frozen Task before promotion.
                guard payload.string("capability_id") == "expert_exchange", let target = expertTaskTarget,
                      !target.source.isMainAgent, let invocation = preparedHostInvocation,
                      let store = canonicalApplication.embeddedStore, let config = activeConfig else {
                    throw DeviceAgentCoordinatorError.unsupportedHostRequest("delegate_expert")
                }
                if self.localTaskHost == nil {
                    self.localTaskHost = IOSLocalTaskHost(store: store, modelClient: modelClient, invocation: invocation,
                        toolPlan: toolPlan, hostExecutor: hostExecutor, config: config, childPermissionEvents: childPermissionEvents,
                        stateChanged: childTaskStateChanged, dependencies: hostDependencyIdentity, expertTargetConfiguration: expertTargetConfiguration)
                }
            }
            guard let localTaskHost, let runID = activeTurnId else {
                throw DeviceAgentCoordinatorError.unsupportedHostRequest("delegate_expert")
            }
            var current = envelope
            if let effect = envelope.string("effect_id"), let delivery = responseDeliveries[effect] { current["delivery"] = delivery }
            let boundRequest = AgentCoreJSONValue.object(current)
            do {
                let bound = try IOSExpertPeerHostRequest(request: boundRequest, expectedContextID: canonicalApplication.conversationID, expectedRunID: runID)
                enteredExecutor = true
                let response: AgentCoreJSONValue
                var preparedTask: AgentCoreJSONValue?
                if bound.arguments["action"] == .string("request") {
                    let preparation = try await localTaskHost.prepareExpert(boundRequest, contextID: canonicalApplication.conversationID, runID: runID)
                    guard let fields = preparation.objectValue, let prepared = fields["prepared"], let promoted = fields["response"] else {
                        throw DeviceAgentCoordinatorError.invalidTransition
                    }
                    preparedTask = prepared; response = promoted
                } else { response = try await localTaskHost.expertAction(boundRequest, contextID: canonicalApplication.conversationID, runID: runID) }
                let accepted = try await submitHostResponse(request, payload: response)
                if let effect = envelope.string("effect_id") { locallyQuiescentEffects.insert(effect) }
                if let preparedTask {
                    do { try await localTaskHost.wake(preparedTask) }
                    catch {
                        // Promotion already committed. Recovery owns the same
                        // queued Task; never submit a second source response.
                        await childTaskStateChanged?()
                    }
                }
                return accepted
            } catch IOSApplicationRuntimeError.outcomePending(let id) { throw IOSApplicationRuntimeError.outcomePending(id) }
            catch {
                if let effect = envelope.string("effect_id") { locallyQuiescentEffects.insert(effect) }
                return try await submitHostResponse(request, payload: Self.expertPreparationFailureResponse(
                    callID: payload.string("call_id") ?? "", error: error))
            }
        }
        if let payload = request.objectValue?["payload"]?.objectValue,
           let tool = payload.string("capability_id"), ["read_task", "wait_task"].contains(tool) {
            guard !childOnly, let localTaskHost else {
                throw DeviceAgentCoordinatorError.unsupportedHostRequest(tool)
            }
            enteredExecutor = true
            // Unwinding this read-only wait stops the local observation, not
            // its independently owned child. It leaves no executor running.
            defer { if let observedEffectID { locallyQuiescentEffects.insert(observedEffectID) } }
            let response = try await localTaskHost.observe(request)
            return try await submitHostResponse(request, payload: response)
        }
        if case .object(let envelope) = request,
           case .object(let payload)? = envelope["payload"], payload.string("capability_id") == "send_agent_message" {
            guard !childOnly, let localTaskHost else {
                throw DeviceAgentCoordinatorError.unsupportedHostRequest("send_agent_message")
            }
            enteredExecutor = true
            let response = try await localTaskHost.guide(request)
            if let observedEffectID { locallyQuiescentEffects.insert(observedEffectID) }
            return try await submitHostResponse(request, payload: response)
        }
        if case .object(let envelope) = request,
           case .object(let payload)? = envelope["payload"],
           payload.string("capability_id") == "spawn_agent" {
            guard !childOnly, let localTaskHost else {
                throw DeviceAgentCoordinatorError.unsupportedHostRequest("spawn_agent")
            }
            enteredExecutor = true
            let prepared = try await localTaskHost.prepare(request)
            guard let value = prepared.objectValue, let task = value["prepared"],
                  let promotion = value["response"], promotion != .null else {
                throw IOSApplicationRuntimeError.rejected("Delegation response requires exact source reconciliation")
            }
            let accepted = try await submitHostResponse(request, payload: promotion)
            if let effectID = envelope.string("effect_id") { locallyQuiescentEffects.insert(effectID) }
            try await localTaskHost.wake(task)
            return accepted
        }
        if case .object(let envelope) = request,
           case .object(let payload)? = envelope["payload"],
           payload.string("capability_id") == "render_ui" {
            guard let effectID = envelope.string("effect_id"),
                  let callID = payload.string("call_id"),
                  let delivery = responseDeliveries[effectID] ?? envelope["delivery"] else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
            try Task.checkCancellation()
            // Application validates the committed request, active delivery
            // fence and canonical Tool Part before touching any Surface.
            enteredExecutor = true
            let payload = try Self.uiPublicationToolResponse(callID: callID) {
                try canonicalApplication.publishUISurface(request: request, delivery: delivery)
            }
            locallyQuiescentEffects.insert(effectID)
            return try await submitHostResponse(request, payload: payload)
        }
        if let envelope = request.objectValue, let payload = envelope["payload"]?.objectValue,
           let tool = payload.string("capability_id"), let session = envelope.string("session_id"),
           let host = envelope.string("host_id"), let store = canonicalApplication.embeddedStore {
            let owner: [String: AgentCoreJSONValue] = ["host_id": .string(host), "session_id": .string(session)]
            toolPlan = try await refreshActiveToolPlan()
            let policy = try toolPlan.originalIntent.map {try JSONDecoder().decode(AgentCoreJSONValue.self,from: JSONEncoder().encode($0))} ?? .null
            var prepare = owner; prepare["type"] = .string("prepare"); prepare["policy"] = policy
            let catalog = try store.mobileMcpHost(.object(prepare))
            if catalog.objectValue?["capabilities"]?.objectValue?[tool] != nil {
                try executionPermit?.checkAdmission()
                var current = envelope
                if let effect = envelope.string("effect_id"), let delivery = responseDeliveries[effect] { current["delivery"] = delivery }
                var command = owner; command["type"] = .string("execute"); command["request"] = .object(current); command["policy"] = policy
                var cancel = owner; cancel["type"] = .string("cancel"); cancel["call_id"] = payload["call_id"]
                let execution = AgentCoreJSONValue.object(command), cancellation = AgentCoreJSONValue.object(cancel)
                enteredExecutor = true
                if let effect = envelope.string("effect_id") { unconfirmedPhysicalEffects.insert(effect) }
                let result = try await withTaskCancellationHandler(operation: {
                    try await Task.detached { try store.mobileMcpHost(execution) }.value
                }, onCancel: { _ = try? store.mobileMcpHost(cancellation) })
                let state = result.objectValue?["output"]?.objectValue?["execution"]?.objectValue?["state"]
                if state == .string("completed") || state == .string("not_started"), let effect = envelope.string("effect_id") {
                    unconfirmedPhysicalEffects.remove(effect); locallyQuiescentEffects.insert(effect)
                }
                return try await submitHostResponse(request, payload: result)
            }
        }
        let generation = lifecycleID
        let refreshed = try await refreshActiveToolPlan()
        guard let generation, lifecycleID == generation else { throw CancellationError() }
        toolPlan = refreshed
        guard let permissionProfile = request.objectValue?["payload"]?.objectValue?.string("permission_profile"),
              IOSPermissionMode(rawValue: permissionProfile) != nil else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        guard case .object(let envelope) = request,
              case .object(let payload)? = envelope["payload"],
              let callId = payload.string("call_id"),
              let toolName = payload.string("capability_id"),
              let resolvedTool = toolPlan.resolvedTool(
                id: toolName,
                route: "device",
                hostId: toolPlan.expectedIdentity.hostId,
                clientId: toolPlan.expectedIdentity.clientId,
                sessionId: toolPlan.expectedIdentity.sessionId
              ) else {
            return try await submitHostResponse(
                request,
                payload: .object([
                    "type": .string("tool_completed"),
                    "call_id": .string(
                        request.objectValue?["payload"]?.objectValue?.string("call_id") ?? ""
                    ),
                    "output": Self.toolResult(.object(["error": .string("Tool is not available in the active iOS plan.")]), state: "not_started"),
                    "outcome": .string("failed"),
                    "full_output_ref": .null,
                    "artifacts": .array([]),
                ])
            )
        }
        try Task.checkCancellation()
        enteredExecutor = true
        if let effectID = envelope.string("effect_id") { unconfirmedPhysicalEffects.insert(effectID) }
        let result = await hostExecutor.executeBridge(
            callId: callId,
            arguments: payload["input"] ?? .object([:]),
            grant: payload["grant"],
            resolvedTool: resolvedTool,
            plan: toolPlan,
            executionPermit: executionPermit
        )
        if !result.isError, let effectID = envelope.string("effect_id") {
            unconfirmedPhysicalEffects.remove(effectID)
            locallyQuiescentEffects.insert(effectID)
        }
        return try await submitHostResponse(
            request,
            payload: .object([
                "type": .string("tool_completed"),
                "call_id": .string(result.callId),
                "output": Self.toolResult(result.output, state: result.isError ? "unknown" : "completed"),
                "outcome": .string(result.isError ? "failed" : "succeeded"),
                "full_output_ref": .null,
                "artifacts": .array(result.artifacts),
            ])
        )
    }

    private func compact(
        _ request: AgentCoreJSONValue,
        continuation: StreamContinuation
    ) async throws -> AgentCoreJSONValue {
        try Task.checkCancellation()
        guard case .object(let envelope) = request,
              case .object(let payload)? = envelope["payload"],
              let sessionId = envelope.string("session_id") else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let targetTokens = payload.int("target_tokens") ?? 2_000
        guard let instructions = payload.string("instructions"),
              !instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        guard let window = payload.int("context_window_tokens"), window > 0 else { throw DeviceAgentCoordinatorError.invalidTransition }
        let outputTokens = activeConfig?.reservedOutputTokens ?? max(512, targetTokens)
        var stage = try canonicalApplication.stepCompactionSummary([
            "type": .string("start"), "messages": .array(payload.array("messages") ?? []),
            "context_window_tokens": .signedInteger(Int64(window)), "output_tokens": .signedInteger(Int64(outputTokens)),
            "target_tokens": .signedInteger(Int64(targetTokens)), "instructions": .string(instructions)
        ])
        func stageMessages(_ stage: [String: AgentCoreJSONValue]) throws -> [AgentCoreMessage] {
            guard let source = stage.array("messages") else { throw DeviceAgentCoordinatorError.invalidTransition }
            return try source.map { value in
                guard case .object(let message) = value, let role = message.string("role"), let content = message.string("content") else {
                    throw DeviceAgentCoordinatorError.invalidTransition
                }
                return AgentCoreMessage(role: role, content: content)
            }
        }
        var segmentMessages = try stageMessages(stage)
        var stageBudget = stage.int("target_tokens") ?? targetTokens
        var physicalAttempts = 0
        var maintenanceRequest = request
        var maintenanceMessages = segmentMessages
        var maintenanceInstructions = instructions
        var providerContinuation: AgentCoreJSONValue?
        var partialSummary = ""
        var continuationCount = 0
        var rewriteCount = 0
        guard let preparedAttemptID = envelope.string("attempt_id") else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let replacingAttempt = envelope.bool("attempt_started") == true
        let attemptID = replacingAttempt ? UUID().uuidString.lowercased() : preparedAttemptID
        let admitted = try await submitHostResponse(
            request,
            payload: .object([
                "type": .string("model_attempt_started"),
                "provider": .string(activeConfig?.executionMode.rawValue ?? "device_local"),
                "model": .string(activeConfig?.model ?? "unknown"),
                "reason": .string(replacingAttempt ? "retry_or_fallback" : "initial"),
            ]),
            attemptID: attemptID
        )
        try await acceptTransition(admitted, continuation: continuation)
        while true {
        try Task.checkCancellation()
        if physicalAttempts > 0 {
            guard case .object(var nextEnvelope) = maintenanceRequest else { throw DeviceAgentCoordinatorError.invalidTransition }
            let nextAttempt = UUID().uuidString.lowercased()
            nextEnvelope["attempt_id"] = .string(nextAttempt)
            maintenanceRequest = .object(nextEnvelope)
            let admitted = try await submitHostResponse(maintenanceRequest, payload: .object([
                "type": .string("model_attempt_started"), "provider": .string(activeConfig?.executionMode.rawValue ?? "device_local"),
                "model": .string(activeConfig?.model ?? "unknown"), "reason": .string("compaction_resampling")
            ]), attemptID: nextAttempt)
            try await acceptTransition(admitted, continuation: continuation)
        }
        physicalAttempts += 1
        modelIteration += 1
        let summaryRequest = AgentModelRequest(
            sessionId: sessionId, turnId: envelope.string("turn_id") ?? "compaction",
            iteration: modelIteration, messages: maintenanceMessages, tools: [],
            reservedOutputTokens: outputTokens,
            policyFragments: [AgentCorePromptFragment(id: "core-context-compaction", layer: .executionPolicy,
                trust: .hostInstruction, cacheable: false, content: maintenanceInstructions)],
            providerContinuation: providerContinuation
        )
        var completion: AgentModelCompletion?
        do {
            try Task.checkCancellation()
            if let effectID = envelope.string("effect_id") {
                unconfirmedPhysicalEffects.insert(effectID)
                try retainChildModelStop(effectID: effectID, request: summaryRequest)
            }
            let stream = await modelClient.generateAgentTurn(request: summaryRequest)
            for try await event in stream {
                try Task.checkCancellation()
                if case .completed(let value) = event {
                    completion = value
                    if let effectID = envelope.string("effect_id") {
                        childModelStops.removeValue(forKey: effectID)
                        unconfirmedPhysicalEffects.remove(effectID)
                        locallyQuiescentEffects.insert(effectID)
                    }
                }
            }
        } catch {
            if error is CancellationError || Task.isCancelled {
                if childOnly, let effect = envelope.string("effect_id"), let owner = childPermissionOwner {
                    try await stopChildPhysicalModel(effectID: effect, conversationID: owner.childConversationID,
                        runID: owner.runID, coreSessionID: owner.coreSessionID)
                } else if let cancellable = modelClient as? AgentModelCancellationClient {
                    try await cancellable.cancelAgentTurn(request: summaryRequest)
                    if let effectID = envelope.string("effect_id") {
                        childModelStops.removeValue(forKey: effectID)
                        unconfirmedPhysicalEffects.remove(effectID)
                        locallyQuiescentEffects.insert(effectID)
                    }
                }
                throw CancellationError()
            }
            if case NolaError.llm(.providerFailure(let code, _, _)) = error, code == "context_length_exceeded", let planState = stage["state"] {
                do {
                    stage = try canonicalApplication.stepCompactionSummary(["type": .string("split"), "state": planState])
                    segmentMessages = try stageMessages(stage); maintenanceMessages = segmentMessages
                    stageBudget = stage.int("target_tokens") ?? targetTokens
                    maintenanceInstructions = instructions; partialSummary = ""; providerContinuation = nil; continuationCount = 0; rewriteCount = 0
                    continue
                } catch { return try await failedHostResponse(maintenanceRequest, error: error) }
            }
            return try await failedHostResponse(maintenanceRequest, error: error)
        }
        guard let completion else {
            return try await failedHostResponse(
                maintenanceRequest,
                error: DeviceAgentCoordinatorError.modelEndedWithoutCompletion
            )
        }
        let summary = partialSummary + completion.content
        let decision = try canonicalApplication.evaluateCompactionMaintenance([
            "summary": .string(summary), "target_tokens": .signedInteger(Int64(stageBudget)),
            "output_limited": .bool(isModelOutputLimitFinishReason(completion.finishReason)),
            "has_tool_calls": .bool(!completion.toolCalls.isEmpty),
            "completed": .bool(["stop", "end_turn", "completed"].contains(completion.finishReason)),
            "cancelled": .bool(Task.isCancelled),
            "continuation_count": .signedInteger(Int64(continuationCount)),
            "rewrite_count": .signedInteger(Int64(rewriteCount)),
            "can_continue": .bool(completion.providerContinuation != nil), "can_rewrite": .bool(true)
        ])
        switch decision.string("action") {
        case "continue":
            guard let instruction = decision.string("instructions") else { throw DeviceAgentCoordinatorError.invalidTransition }
            maintenanceInstructions = instructions + "\n\n" + instruction
            partialSummary = summary
            providerContinuation = completion.providerContinuation
            maintenanceMessages = segmentMessages + [AgentCoreMessage(role: "assistant", content: summary)]
            continuationCount += 1
        case "rewrite":
            guard let rewrite = decision.string("instructions") else { throw DeviceAgentCoordinatorError.invalidTransition }
            maintenanceInstructions = instructions + "\n\n" + rewrite
            maintenanceMessages = segmentMessages + [AgentCoreMessage(role: "assistant", content: summary)]
            providerContinuation = nil
            partialSummary = ""
            rewriteCount += 1
        case "accept":
            guard let planState = stage["state"] else { throw DeviceAgentCoordinatorError.invalidTransition }
            stage = try canonicalApplication.stepCompactionSummary([
                "type": .string("accept"), "state": planState, "summary": .string(summary)
            ])
            if stage.string("action") == "request" {
                segmentMessages = try stageMessages(stage)
                maintenanceMessages = segmentMessages
                stageBudget = stage.int("target_tokens") ?? targetTokens
                maintenanceInstructions = instructions
                partialSummary = ""; providerContinuation = nil; continuationCount = 0; rewriteCount = 0
                continue
            }
            guard stage.string("action") == "complete", stage.string("summary") == summary else { throw DeviceAgentCoordinatorError.invalidTransition }
            return try await submitHostResponse(maintenanceRequest, payload: .object([
                "type": .string("compaction_completed"), "summary": .string(summary),
                "usage": Self.v2Usage(completion.usage, isCompaction: true)
            ]))
        case "reject":
            return try await submitHostResponse(maintenanceRequest, payload: .object([
                "type": .string("failed"), "error": .object([
                    "code": .string(decision.string("code") ?? "invalid_compaction_summary"),
                    "message": .string("Context maintenance could not produce an admitted checkpoint; original context retained"),
                    "retryable": .bool(false)
                ])
            ]))
        default: throw DeviceAgentCoordinatorError.invalidTransition
        }
        }
    }

    private func corePermissionPresentation(_ request: AgentCoreJSONValue) throws -> IOSAgentPermissionRequest? {
        guard let envelope = request.objectValue,
              let payload = envelope["payload"]?.objectValue,
              let permissionID = payload.string("permission_request_id") else { return nil }
        var inspected = envelope
        if !childOnly, let effectID = envelope.string("effect_id"), let renewed = responseDeliveries[effectID] {
            inspected["delivery"] = renewed
        }
        let result = try canonicalApplication.inspectMobilePermissionRequest(.object(inspected))
        guard let proof = result.objectValue,
              proof["conversation_id"] == .string(canonicalApplication.conversationID),
              proof["run_id"] == .string(canonicalApplication.runID),
              proof["core_session_id"] == envelope["session_id"],
              proof["effect_id"] == envelope["effect_id"],
              proof["request_id"] == .string(permissionID), proof["payload"] == envelope["payload"],
              proof["capability"]?.objectValue != nil, proof["permission_domain"] != nil else {
            throw DeviceAgentCoordinatorError.invalidPermissionEvent
        }
        let origin = proof["capability"]?.objectValue?["input_normalization"]?.objectValue?["origin"]?.objectValue?["kind"]
        guard proof["permission_domain"] == .null || origin == .string("mcp") else { return nil }
        guard let capability = payload.string("capability_id"),
              case .bool(let allow)? = payload["allow_conversation"],
              let instructions = AgentPermissionInstruction.decodeCore(payload["permission_instructions"]) else {
            throw DeviceAgentCoordinatorError.invalidPermissionEvent
        }
        return IOSAgentPermissionRequest(coreRequestID: permissionID, callID: payload.string("call_id") ?? "",
            capabilityID: capability, allowConversation: allow, rationale: origin == .string("mcp")
                ? (proof.string("display_title") ?? capability) + "\n" + (payload.string("reason") ?? "")
                : payload.string("reason") ?? "",
            scope: payload.string("resource"), originKind: origin == .string("mcp") ? "mcp" : nil, instructions: instructions, childOwner: childPermissionOwner)
    }

    private func registerPermissionRequest(
        _ payload: [String: AgentCoreJSONValue]
    ) async throws {
        if let permissionID = payload.string("permission_request_id"),
           let envelope = pendingPermissionEnvelopes[permissionID],
           let request = try corePermissionPresentation(envelope) {
            guard pendingPermissions[permissionID] == nil else { return }
            pendingPermissions[permissionID] = request
            continuation?.yield(.permissionRequested(request))
            return
        }
        let generation = lifecycleID
        let refreshed = try await refreshActiveToolPlan()
        guard let generation, lifecycleID == generation else { throw CancellationError() }
        toolPlan = refreshed
        guard let instructions = AgentPermissionInstruction.decodeCore(payload["permission_instructions"]),
              case .bool(let allowConversation)? = payload["allow_conversation"],
              let requestId = payload.string("permission_request_id"),
              let toolName = payload.string("capability_id"),
              let resolvedTool = toolPlan.resolvedTool(
                id: toolName,
                route: "device",
                hostId: toolPlan.expectedIdentity.hostId,
                clientId: toolPlan.expectedIdentity.clientId,
                sessionId: toolPlan.expectedIdentity.sessionId
              ),
              let request = IOSAgentPermissionRequest(
                requestId: requestId,
                allowConversation: allowConversation,
                callId: payload.string("call_id") ?? "",
                resolvedTool: resolvedTool,
                plan: toolPlan,
                rationale: payload.string("reason") ?? "",
                scope: payload.string("resource"),
                permissionInstructions: instructions,
                childOwner: childPermissionOwner
              ) else {
            throw DeviceAgentCoordinatorError.invalidPermissionEvent
        }
        guard pendingPermissions[request.requestId] == nil else { return }
        pendingPermissions[request.requestId] = request
        continuation?.yield(.permissionRequested(request))
    }

    private func registerProjectedPermission(_ projection: [String: AgentCoreJSONValue],
        continuation: StreamContinuation) async throws {
        guard let requestID = projection.string("request_id"),
              let envelope = pendingPermissionEnvelopes[requestID],
              let payload = envelope.objectValue?["payload"]?.objectValue else { return }
        try await registerPermissionRequest(payload)
    }

    private func restorePermissionProjection(
        from value: AgentCoreJSONValue,
        continuation: StreamContinuation
    ) async throws {
        guard case .object(let snapshot) = value,
              case .object(let projection)? = snapshot["permissions"],
              case .array(let pending)? = projection["pending"] else { return }
        let activeIds = Set(pending.compactMap { value -> String? in
            guard case .object(let request) = value else { return nil }
            return request.string("request_id")
        })
        pendingPermissions = pendingPermissions.filter { activeIds.contains($0.key) }
        pendingPermissionEnvelopes = pendingPermissionEnvelopes.filter { activeIds.contains($0.key) }
        for value in pending {
            guard case .object(let request) = value else { continue }
            try await registerProjectedPermission(request, continuation: continuation)
        }
    }

    static func uiPublicationToolResponse(callID: String, publish: () throws -> AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        let output: AgentCoreJSONValue
        let outcome: String
        do {
            output = toolResult(try publish(), state: "completed")
            outcome = "succeeded"
        } catch IOSApplicationRuntimeError.rejected(let message) {
            // Only an explicit committed rejection proves publication did not run.
            // Pending receipts, transport errors and cancellation remain unresolved.
            output = toolResult(.object([
                "code": .string("ui_publication_rejected"),
                "error": .string(message),
            ]), state: "not_started")
            outcome = "failed"
        }
        return .object([
            "type": .string("tool_completed"), "call_id": .string(callID),
            "output": output, "outcome": .string(outcome),
            "full_output_ref": .null, "artifacts": .array([]),
        ])
    }

    private static func toolResult(_ data: AgentCoreJSONValue, state: String) -> AgentCoreJSONValue {
        .object([
            "contract": .string("nola.tool_result.v1"),
            "data": data,
            "presentation": .object(["text": .string(""), "summary": .string("")]),
            "execution": .object(["state": .string(state), "local_stop": .null, "effects": .array([]), "checks": .array([])]),
            "full_output_ref": .null,
            "memory_usage": .null,
        ])
    }

    private func submitHostResponse(
        _ request: AgentCoreJSONValue,
        payload: AgentCoreJSONValue,
        attemptID: String? = nil
    ) async throws -> AgentCoreJSONValue {
        try Task.checkCancellation()
        guard case .object(let envelope) = request,
              let effectId = envelope.string("effect_id") else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let resumedSequence = envelope.int("resume_after_sequence") ?? 0
        let sequence = (responseSequences[effectId] ?? resumedSequence) + 1
        if let attemptID {
            responseAttemptIDs[effectId] = attemptID
        }
        let response = try Self.hostResponseEnvelope(
            request: request,
            payload: payload,
            sequence: sequence,
            attemptID: attemptID ?? responseAttemptIDs[effectId],
            delivery: responseDeliveries[effectId]
        )
        let transition: AgentCoreJSONValue
        if childOnly {
            guard let commit = childResponseSinks[effectId] else { throw DeviceAgentCoordinatorError.invalidTransition }
            try await commit(response)
            transition = .null
        } else {
            transition = try canonicalApplication.acceptHostResponse(
                response,
                receivedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
        }
        responseSequences[effectId] = sequence
        if Self.isTerminalHostResponse(payload) {
            leaseHeartbeatTasks.removeValue(forKey: effectId)?.cancel()
            responseDeliveries.removeValue(forKey: effectId)
            responseAttemptIDs.removeValue(forKey: effectId)
        }
        return transition
    }

    private func startLeaseHeartbeat(for request: AgentCoreJSONValue) {
        guard case .object(let envelope) = request,
              let effectID = envelope.string("effect_id"),
              let delivery = envelope["delivery"] else { return }
        responseDeliveries[effectID] = delivery
        leaseHeartbeatTasks.removeValue(forKey: effectID)?.cancel()
        leaseHeartbeatTasks[effectID] = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    try await Task.sleep(nanoseconds: 120 * 1_000_000_000)
                } catch {
                    return
                }
                guard !Task.isCancelled else { return }
                await self?.renewLeaseHeartbeat(effectID: effectID)
            }
        }
    }

    private func renewLeaseHeartbeat(effectID: String) {
        guard let delivery = responseDeliveries[effectID] else { return }
        do {
            responseDeliveries[effectID] = try canonicalApplication.renewHostDelivery(delivery)
        } catch {
            leaseHeartbeatTasks.removeValue(forKey: effectID)?.cancel()
            finish(throwing: error)
        }
    }

    nonisolated static func isTerminalHostResponse(
        _ payload: AgentCoreJSONValue
    ) -> Bool {
        guard case .object(let object) = payload,
              case .string(let type)? = object["type"] else { return false }
        // Mirror HostResponse::is_terminal, not the presence of a snapshot.
        // Receipts/progress keep the delivery lease and attempt alive.
        switch type {
        case "task_updated":
            guard case .string(let state)? = object["state"] else { return false }
            return ["completed", "failed", "cancelled"].contains(state)
        case "delegated_agent_event":
            guard case .object(let event)? = object["event"],
                  case .string(let kind)? = event["type"] else { return false }
            return ["completed", "failed", "cancellation_confirmed"].contains(kind)
        case "task_promoted", "delegated_permission_transport_written", "execution_rejected",
             "permission_resolved", "context_fetched", "capability_catalog_refreshed",
             "model_completed", "model_output_limited", "tool_completed", "compaction_completed",
             "completed", "failed", "cancellation_confirmed":
            return true
        default:
            return false
        }
    }

    nonisolated static func hostResponseEnvelope(
        request: AgentCoreJSONValue,
        payload: AgentCoreJSONValue,
        sequence: Int,
        attemptID: String? = nil,
        delivery: AgentCoreJSONValue? = nil
    ) throws -> AgentCoreJSONValue {
        guard case .object(let envelope) = request,
              let effectId = envelope.string("effect_id"),
              case .object? = envelope["delivery"] else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        return .object([
            "attempt_id": attemptID.map(AgentCoreJSONValue.string)
                ?? envelope["attempt_id"]
                ?? .null,
            "delegated_invocation_id": envelope["delegated_invocation_id"] ?? .null,
            "request_id": envelope["request_id"] ?? .string(effectId),
            "effect_id": envelope["effect_id"] ?? .string(effectId),
            "host_id": envelope["host_id"] ?? .string("ios-device"),
            "lease_id": envelope["lease_id"] ?? .null,
            "delivery": delivery ?? envelope["delivery"] ?? .null,
            "session_id": envelope["session_id"] ?? .string(""),
            "turn_id": envelope["turn_id"] ?? .null,
            "catalog_revision": envelope["catalog_revision"] ?? .number(1),
            "availability_epoch": envelope["availability_epoch"] ?? .number(1),
            "sequence": .number(Double(sequence)),
            "payload": payload,
        ])
    }

    private func submitCoreSubmission(
        _ submission: AgentCoreJSONValue
    ) async throws -> AgentCoreJSONValue {
        try Task.checkCancellation()
        guard let owner = executionPermit else { throw CancellationError() }
        try owner.checkAdmission()
        return try canonicalApplication.submitActive(submission)
    }

    private func failedHostResponse(
        _ request: AgentCoreJSONValue,
        error: Error
    ) async throws -> AgentCoreJSONValue {
        let failure = Self.modelFailureMetadata(error)
        return try await submitHostResponse(
            request,
            payload: .object([
                "type": .string("failed"),
                "error": .object([
                    "code": .string(failure.code),
                    "message": .string(error.localizedDescription),
                    "retryable": .bool(failure.retryable),
                ]),
            ])
        )
    }

    private static func isRetryableModelFailure(_ error: Error) -> Bool {
        modelFailureMetadata(error).retryable
    }

    static func modelFailureMetadata(_ error: Error) -> (code: String, retryable: Bool) {
        if let error = error as? NolaError {
            switch error {
            case .networking(let nested): return modelFailureMetadata(nested)
            case .llm(let nested): return modelFailureMetadata(nested)
            case .unknown(let nested): return modelFailureMetadata(nested)
            case .storage, .dataSource: return ("ios_host_failure", false)
            }
        }
        if let error = error as? LLMError {
            switch error {
            case .providerFailure(let code, _, let retryable): return (code, retryable)
            case .timeout: return ("model_timeout", true)
            case .rateLimited: return ("model_rate_limited", true)
            case .network(let nested): return modelFailureMetadata(nested)
            default: return ("ios_model_failure", false)
            }
        }
        if let error = error as? NetworkingError {
            switch error {
            case .notConnected: return ("model_network_unavailable", true)
            case .timeout: return ("model_timeout", true)
            case .rateLimited: return ("model_rate_limited", true)
            case .serverError(let status) where status >= 500:
                return ("model_server_unavailable", true)
            default: return ("ios_network_failure", false)
            }
        }
        if let error = error as? URLError {
            switch error.code {
            case .timedOut: return ("model_timeout", true)
            case .cannotFindHost, .cannotConnectToHost, .networkConnectionLost, .notConnectedToInternet:
                return ("model_network_unavailable", true)
            default: return ("ios_network_failure", false)
            }
        }
        return ("ios_host_failure", false)
    }

    /// Prepare the same Host configuration used by ordinary turns. Core owns
    /// Goal admission; this does not start a Run, execute tools or create a worker.
    func prepareHostConfiguration(sessionID: String, config: AgentCoreRuntimeConfig) throws -> AgentCoreJSONValue {
        try makeRuntimeConfig(sessionId: sessionID, history: [], inputOrigin: .interactiveUser, config: config)
    }

    private func makeRuntimeConfig(
        sessionId: String?,
        history: [AgentCoreMessage],
        inputOrigin: AgentCoreInputOrigin,
        config: AgentCoreRuntimeConfig,
        captureDelegation: Bool = true
    ) throws -> AgentCoreJSONValue {
        let hostId = config.hostId ?? toolPlan.expectedIdentity.hostId ?? "ios-device"
        var capabilities = builtInHostCapabilities()
        let uiContract = try canonicalApplication.uiCapabilityContract()
        guard let fields = uiContract.objectValue,
              let uiDescriptor = fields["descriptor"] else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        for tool in toolPlan.output.enabledTools where tool.enabled {
            capabilities[tool.toolId] = tool.toolId == "render_ui"
                ? uiDescriptor : capabilityDescriptor(for: tool)
        }
        var mcpTools: [AgentCoreJSONValue] = []
        var mcpRules: [AgentCoreJSONValue] = []
        if let sessionId, let store = canonicalApplication.embeddedStore {
            let mcp = try store.mobileMcpHost(.object(["type": .string("prepare"),
                "host_id": .string(hostId), "session_id": .string(sessionId),
                "policy": try toolPlan.originalIntent.map {try JSONDecoder().decode(AgentCoreJSONValue.self,from: JSONEncoder().encode($0))} ?? .null]))
            if case .object(let additions)? = mcp.objectValue?["capabilities"] {
                for (id, value) in additions { capabilities[id] = value }
            }
            if case .array(let tools)? = mcp.objectValue?["tools"] { mcpTools = tools }
            if case .array(let rules)? = mcp.objectValue?["permission_rules"] { mcpRules = rules }
        }
        var runtimeConfig: [String: AgentCoreJSONValue] = [
            "session_id": sessionId.map(AgentCoreJSONValue.string) ?? .null,
            "model": config.model.map(AgentCoreJSONValue.string) ?? .null,
            "initial_items": .array(
                initialItems(from: history, sessionId: sessionId ?? "ios-session")
            ),
            "tools": .array(modelSupportsToolCalls ? toolPlan.schemas + mcpTools : []),
            "context_window": .object([
                "context_window_tokens": .number(Double(config.contextWindowTokens ?? 128_000)),
                "reserved_output_tokens": .number(Double(config.reservedOutputTokens)),
            ]),
            "host_catalog": .object([
                "host_id": .string(hostId),
                "revision": .number(3),
                "availability_epoch": .number(1),
                "capabilities": .object(capabilities),
            ]),
            "permission_rules": Self.permissionRules(
                enabledTools: toolPlan.output.enabledTools,
                scopedGrants: freshResendTarget == nil ? toolPlan.scopedGrants : [],
                permissionMode: config.permissionProfile,
                isAutomated: inputOrigin.isAutomated
            ),
        ]
        if case .object(var policy)? = runtimeConfig["permission_rules"], case .array(let rules)? = policy["rules"] {
            policy["rules"] = .array(rules + mcpRules); runtimeConfig["permission_rules"] = .object(policy)
        }
        if let turnIdentity = Self.promptTurnIdentity(
            stablePromptDigest: config.stablePromptDigest,
            promptFragmentIds: config.promptFragmentIds,
            stablePromptFragmentDigests: config.stablePromptFragmentDigests ?? [:],
            modelCapability: modelClient.coreModelCapabilityProfile
        ) {
            runtimeConfig["turn_identity"] = turnIdentity
        }
        if let preparedHostInvocation {
            let policy: AgentCoreJSONValue = .object([
                "host_catalog": runtimeConfig["host_catalog"]!, "tools": runtimeConfig["tools"]!,
                "permission_rules": runtimeConfig["permission_rules"]!, "context_window": runtimeConfig["context_window"]!,
                "model_capability": modelClient.coreModelCapabilityProfile ?? .null,
                "host_dependencies": hostDependencyIdentity,
            ])
            var identity: [String: AgentCoreJSONValue] = [:]
            if case .object(let existing)? = runtimeConfig["turn_identity"] { identity = existing }
            identity["host_invocation"] = try preparedHostInvocation.binding(executionPolicy: policy)
            identity["host_dependencies"] = hostDependencyIdentity
            runtimeConfig["turn_identity"] = .object(identity)
        }
        var configured: AgentCoreJSONValue = .object(runtimeConfig)
        if captureDelegation, !childOnly, let delegationSettings, let store = canonicalApplication.embeddedStore {
            configured = try store.executeAgentCommand(.object([
                "type": .string("capture_mobile_delegation_configuration"),
                "config": configured, "settings": delegationSettings,
            ]))
        }
        return configured
    }

    nonisolated static func promptTurnIdentity(
        stablePromptDigest: String?,
        promptFragmentIds: [String],
        stablePromptFragmentDigests: [String: String],
        modelCapability: AgentCoreJSONValue? = nil
    ) -> AgentCoreJSONValue? {
        let digest = stablePromptDigest?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .nonEmpty
        let fragmentIds = promptFragmentIds
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        let fragmentDigests = stablePromptFragmentDigests.reduce(into: [String: AgentCoreJSONValue]()) { result, entry in
            let id = entry.key.trimmingCharacters(in: .whitespacesAndNewlines)
            let value = entry.value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, !value.isEmpty else { return }
            result[id] = .string(value)
        }
        var identity: [String: AgentCoreJSONValue] = [:]
        if digest != nil || !fragmentIds.isEmpty || !fragmentDigests.isEmpty {
            identity["prompt"] = .object([
                "stable_prefix_digest": digest.map(AgentCoreJSONValue.string) ?? .string(""),
                "fragment_ids": .array(fragmentIds.map(AgentCoreJSONValue.string)),
                "stable_fragment_digests": .object(fragmentDigests),
            ])
        }
        if case .object(let capability)? = modelCapability {
            identity["model_capability"] = modelCapability
            identity["model_capability_id"] = capability["profile_id"] ?? .string("")
            identity["model_capability_revision"] = capability["revision"] ?? .number(0)
        }
        return identity.isEmpty ? nil : .object(identity)
    }

    nonisolated static func permissionRules(
        enabledTools: [AgentCoreResolvedTool],
        scopedGrants: [AgentCoreScopedGrant],
        permissionMode: IOSPermissionMode = .cautious,
        isAutomated: Bool = false
    ) -> AgentCoreJSONValue {
        var rules: [AgentCoreJSONValue] = []
        switch permissionMode {
        case .cautious:
            rules.append(.object([
                "capability_pattern": .string("*"),
                "resource_pattern": .string("*"),
                "decision": .string("allow"),
                "source": .string("product_default"),
            ]))
        case .allowAll:
            rules.append(.object([
                "capability_pattern": .string("*"),
                "resource_pattern": .string("*"),
                "decision": .string("allow"),
                "source": .string("invocation_policy"),
            ]))
        }
        rules.append(contentsOf: enabledTools
            .filter { $0.permission != "none" && ["ask", "granted"].contains($0.authorization) }
            .map { tool in
                AgentCoreJSONValue.object([
                    "capability_pattern": .string(tool.capability),
                    "resource_pattern": .string("*"),
                    "decision": .string(tool.authorization == "granted" ? "allow" : "ask"),
                    "source": .string("host"),
                ])
            })
        rules.append(contentsOf: scopedGrants.compactMap { grant in
            guard let scope = grant.scope?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !scope.isEmpty else { return nil }
            return .object([
                "capability_pattern": .string(grant.capability),
                "resource_pattern": .string(scope),
                "decision": .string("allow"),
                "source": .string("host"),
            ])
        })
        return .object(["rules": .array(rules)])
    }

    private func builtInHostCapabilities() -> [String: AgentCoreJSONValue] {
        [
            "model.stream": hostCapability(
                id: "model.stream",
                title: "Model",
                route: "model",
                exposure: "internal",
                originKind: "internal",
                supportsStreaming: true,
                replaySafety: "recoverable"
            ),
            "permission.ask": hostCapability(
                id: "permission.ask",
                title: "Permission",
                route: "permission",
                exposure: "internal",
                originKind: "internal",
                replaySafety: "requires_recovery_decision"
            ),
            "context.compact": hostCapability(
                id: "context.compact",
                title: "Context compaction",
                route: "compaction",
                exposure: "internal",
                originKind: "internal",
                replaySafety: "recoverable"
            ),
            "host.cancel": hostCapability(
                id: "host.cancel",
                title: "Cancel",
                route: "tool",
                exposure: "hidden",
                originKind: "internal",
                replaySafety: "idempotent"
            ),
        ]
    }

    private func capabilityDescriptor(
        for tool: AgentCoreResolvedTool
    ) -> AgentCoreJSONValue {
        let effect = engineeringEffect(for: tool.schema)
        let readOnly = effect == "investigate"
        // This registered dispatcher journals the original source call and
        // replays canonical Task receipts; it does not repeat an external write.
        let expertDispatcher = tool.toolId == "delegate_expert"
        return hostCapability(
            id: tool.toolId,
            title: tool.name,
            route: "tool",
            permissionDomain: tool.permission == "none" ? nil : tool.capability,
            inputSchema: modelInputSchema(from: tool.schema),
            exposure: "visible",
            originKind: expertDispatcher ? "internal" : "device",
            effect: effect,
            parallelism: readOnly ? "concurrent_safe" : "serial",
            cancellation: "best_effort",
            replaySafety: expertDispatcher ? "recoverable" : (readOnly ? "idempotent" : "requires_recovery_decision"),
            permissionContract: {
                guard case .object(let schema) = tool.schema,
                      case .object(let metadata)? = schema["x_nola"] else { return nil }
                return metadata["permission_contract"]
            }()
        )
    }

    /// HostCapabilityDescriptor.input_schema is the function argument schema,
    /// not the complete OpenAI tool definition. Passing the outer definition
    /// here would make Core emit `function.parameters.function`, which remote
    /// providers correctly reject as an invalid JSON Schema.
    private func modelInputSchema(from toolSchema: AgentCoreJSONValue) -> AgentCoreJSONValue {
        guard case .object(let root) = toolSchema else {
            return .object(["type": .string("object")])
        }
        if case .object(let function)? = root["function"],
           let parameters = function["parameters"] {
            return parameters
        }
        if let parameters = root["parameters"] {
            return parameters
        }
        return .object(["type": .string("object")])
    }

    private func hostCapability(
        id: String,
        title: String,
        route: String,
        permissionDomain: String? = nil,
        inputSchema: AgentCoreJSONValue = .object([:]),
        exposure: String,
        originKind: String,
        effect: String = "execute",
        supportsStreaming: Bool = false,
        parallelism: String = "serial",
        cancellation: String = "unsupported",
        replaySafety: String,
        permissionContract: AgentCoreJSONValue? = nil
    ) -> AgentCoreJSONValue {
        .object([
            "id": .string(id),
            "title": .string(title),
            "route": .string(route),
            "availability": .string("available"),
            "exposure": .string(exposure),
            "permission_domain": permissionDomain.map(AgentCoreJSONValue.string) ?? .null,
            "input_schema": inputSchema,
            "input_normalization": .object([
                "origin": .object([
                    "kind": .string(originKind),
                    "provider_id": .string("ios-device"),
                    "provider_title": .string("iOS"),
                    "namespace": .string(originKind == "internal" ? "internal" : "device"),
                ]),
                "effect": .object([
                    "schema_version": .number(1),
                    "fixed": .string(effect),
                    "variants": .object([:]),
                ]),
                "permission_resource": Self.permissionResourceDeclaration(for: id),
                "permission_contract": permissionContract ?? .object(["risk": .string("unknown")]),
            ]),
            "output_policy": .object([
                "preview_char_limit": .number(8_000),
                "persist_full_output": .bool(false),
                "allow_artifacts": .bool(true),
            ]),
            "supports_streaming": .bool(supportsStreaming),
            "supports_argument_streaming": .bool(false),
            "parallelism": .string(parallelism),
            "cancellation": .string(cancellation),
            "replay_safety": .string(replaySafety),
        ])
    }

    private static func permissionResourceDeclaration(for capabilityID: String) -> AgentCoreJSONValue {
        switch capabilityID {
        case "device_read_file":
            return .object([
                "kind": .string("first_of"),
                "keys": .array(["path", "url", "uri"].map(AgentCoreJSONValue.string)),
            ])
        case "device_write_file":
            return .object([
                "kind": .string("first_of"),
                "keys": .array(["destination", "path", "uri"].map(AgentCoreJSONValue.string)),
            ])
        case "device_clipboard_read", "device_clipboard_write":
            return .object([
                "kind": .string("constant"),
                "value": .string("clipboard"),
            ])
        case "device_get_location":
            return .object([
                "kind": .string("constant"),
                "value": .string("current_location"),
            ])
        case "device_search_calendar_events":
            return .object([
                "kind": .string("constant"),
                "value": .string("calendar_events"),
            ])
        default:
            return .object(["kind": .string("full_input")])
        }
    }

    private func engineeringEffect(for schema: AgentCoreJSONValue) -> String {
        guard case .object(let root) = schema,
              case .object(let metadata)? = root["x_nola"],
              case .string(let declared)? = metadata["effect"] else {
            return "execute"
        }
        switch declared {
        case "read", "inspect": return "investigate"
        case "write", "external_write", "mutate": return "mutate"
        case "validate": return "validate"
        case "mutate_and_validate": return "mutate_and_validate"
        default: return "execute"
        }
    }

    private func initialItems(
        from history: [AgentCoreMessage],
        sessionId: String
    ) -> [AgentCoreJSONValue] {
        var items: [AgentCoreJSONValue] = []
        for (messageIndex, message) in history.enumerated() {
            let identityPrefix = String(
                format: "%@:%@:%08d",
                sessionId,
                "history",
                messageIndex
            )
            if message.role == "tool", let callId = message.toolCallId {
                items.append(.object([
                    "type": .string("tool_result"),
                    "id": .string("\(identityPrefix):tool-result"),
                    "call_id": .string(callId),
                    "name": .string(message.name ?? "tool"),
                    "output": .string(message.content),
                    "is_error": .bool(false),
                ]))
                continue
            }
            if !message.content.isEmpty {
                items.append(.object([
                    "type": .string("message"),
                    "id": .string("\(identityPrefix):message"),
                    "role": .string(normalizedRole(message.role)),
                    "content": .string(message.content),
                ]))
            }
            for (callIndex, call) in message.toolCalls.enumerated() {
                items.append(.object([
                    "type": .string("tool_call"),
                    "id": .string("\(identityPrefix):tool-call:\(callIndex)"),
                    "call_id": .string(call.id),
                    "name": .string(call.name),
                    "arguments": call.arguments,
                ]))
            }
        }
        return items
    }

    private func modelMessages(from items: [AgentCoreJSONValue]) -> [AgentCoreMessage] {
        var messages: [AgentCoreMessage] = []
        for value in items {
            guard case .object(let item) = value else { continue }
            switch item.string("type") {
            case "message":
                messages.append(.init(
                    role: item.string("role") ?? "user",
                    content: item.string("content") ?? ""
                ))
            case "tool_call":
                let call = AgentCoreToolCall(
                    id: item.string("call_id") ?? UUID().uuidString.lowercased(),
                    name: item.string("name") ?? "tool",
                    arguments: item["arguments"] ?? .object([:])
                )
                if let last = messages.indices.last,
                   messages[last].role == "assistant",
                   messages[last].toolCallId == nil {
                    messages[last].toolCalls.append(call)
                } else {
                    messages.append(.init(
                        role: "assistant",
                        content: "",
                        toolCalls: [call]
                    ))
                }
            case "tool_result":
                messages.append(.init(
                    role: "tool",
                    content: rendered(item["output"] ?? .null),
                    name: item.string("name"),
                    toolCallId: item.string("call_id")
                ))
            case "compaction":
                messages.append(.init(
                    role: "system",
                    content: item.string("summary") ?? ""
                ))
            default:
                break
            }
        }
        return messages
    }

    nonisolated static func canonicalControlProjection(
        _ transition: [String: AgentCoreJSONValue]
    ) throws -> AgentCoreJSONValue? {
        guard transition["snapshot"] == nil, transition["checkpoint"] == nil else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        guard let value = transition["control"] else {
            guard transition["permission_projection"] == nil else {
                throw DeviceAgentCoordinatorError.invalidTransition
            }
            return nil
        }
        guard case .object(var control) = value,
              let revision = control["permission_projection_revision"], revision != .null,
              case .object(let permissions)? = transition["permission_projection"],
              permissions["revision"] == revision,
              case .array? = permissions["pending"] else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        control["permissions"] = .object(permissions)
        return .object(control)
    }

    private func clientSnapshot(from value: AgentCoreJSONValue) throws -> AgentCoreSnapshot {
        guard case .object(let snapshot) = value else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let status = snapshot.string("turn_status") ?? "idle"
        let config = activeConfig
        let adapted = AgentCoreJSONValue.object([
            "platform": .string(config?.platform.rawValue ?? "ios"),
            "execution_mode": .string(config?.executionMode.rawValue ?? "device_local"),
            "host_id": config?.hostId.map(AgentCoreJSONValue.string) ?? .null,
            "phase": .string(status),
            "session_id": snapshot["session_id"] ?? .null,
            "turn_id": snapshot["active_turn_id"] ?? .null,
            "iteration": .number(Double(modelIteration)),
            "message_count": snapshot["history_item_count"] ?? .number(0),
            "estimated_context_tokens": snapshot["context_used_tokens"] ?? .number(0),
            "context_window_tokens": snapshot["context_window_tokens"] ?? .number(128_000),
            "context_usage_percent": snapshot["context_usage_percent"] ?? .number(0),
            "pending_tool_count": snapshot["active_tool_count"] ?? .number(0),
            "session_closing": snapshot["session_closing"] ?? .bool(false),
            "session_closed": snapshot["session_closed"] ?? .bool(false),
            "awaiting_permission": .bool(status == "awaiting_permission"),
            "awaiting_host_call": .bool(status == "awaiting_tools"),
            // Context occupancy is not provider billing usage.
            "usage": .object([:]),
            "prompt_cache_diagnostic": snapshot["prompt_cache_diagnostic"] ?? .null,
            "permissions": snapshot["permissions"] ?? .null,
        ])
        let data = try JSONEncoder().encode(adapted)
        return try JSONDecoder().decode(AgentCoreSnapshot.self, from: data)
    }

    static func v2Usage(_ usage: AgentCoreTokenUsage, isCompaction: Bool = false) -> AgentCoreJSONValue {
        // AgentCoreTokenUsage.promptTokens is the inclusive prompt total supplied
        // by the Chat/Responses adapter, not current conversation occupancy.
        let rate: AgentCoreJSONValue
        if let input = usage.promptTokens, input > 0, let read = usage.cachedInputTokens, read >= 0, read <= input {
            let product = UInt64(read).multipliedFullWidth(by: 10_000)
            rate = .number(Double(UInt64(input).dividingFullWidth(product).quotient))
        } else { rate = .null }
        let value: [String: AgentCoreJSONValue] = [
            "cache_usage": .object([
                "input_tokens": usage.promptTokens.map { .number(Double($0)) } ?? .null,
                "read_tokens": usage.cachedInputTokens.map { .number(Double($0)) } ?? .null,
                "write_tokens": usage.cacheWriteInputTokens.map { .number(Double($0)) } ?? .null,
                "hit_rate_bps": rate,
                "is_compaction": .bool(isCompaction),
            ]),
            "input_tokens": usage.promptTokens.map { .number(Double($0)) } ?? .null,
            "cached_input_tokens": usage.cachedInputTokens.map { .number(Double($0)) } ?? .null,
            "cache_write_input_tokens": usage.cacheWriteInputTokens.map { .number(Double($0)) } ?? .null,
            "output_tokens": usage.completionTokens.map { .number(Double($0)) } ?? .null,
            "reasoning_tokens": usage.reasoningTokens.map { .number(Double($0)) } ?? .null,
            "total_tokens": usage.totalTokens.map { .number(Double($0)) } ?? .null,
            "request_count": .number(1),
        ]
        return .object(value)
    }

    private func missingSuffix(full: String, current: String) -> String {
        guard full != current else { return "" }
        guard full.hasPrefix(current) else {
            return current.isEmpty ? full : ""
        }
        return String(full.dropFirst(current.count))
    }

    private func normalizedRole(_ role: String) -> String {
        switch role {
        case "system", "user", "assistant", "developer": return role
        default: return "user"
        }
    }

    private func rendered(_ value: AgentCoreJSONValue) -> String {
        if case .string(let text) = value { return text }
        guard let data = try? JSONEncoder().encode(value) else { return "" }
        return String(decoding: data, as: UTF8.self)
    }

    private func finish(throwing error: Error? = nil) {
        executionPermit?.revoke()
        executionPermit = nil
        drivingOwner = nil
        driverTask = nil
        activeStreamID = nil
        cancellingOwner = nil
        cancellationTask = nil
        unconfirmedPhysicalEffects.removeAll()
        locallyQuiescentEffects.removeAll()
        lifecycleID = nil
        if let error {
            continuation?.finish(throwing: error)
        } else {
            continuation?.finish()
        }
        continuation = nil
        activeConfig = nil
        activeSessionId = nil
        activeTurnId = nil
        pendingPermissions.removeAll()
        pendingPermissionEnvelopes.removeAll()
        queuedHostRequests.removeAll()
        responseSequences.removeAll()
        responseAttemptIDs.removeAll()
        responseDeliveries.removeAll()
        leaseHeartbeatTasks.values.forEach { $0.cancel() }
        leaseHeartbeatTasks.removeAll()
        taskProjection = AgentCoreTaskProjectionReducer()
        modelIteration = 0
    }

}

private extension String {
    nonisolated var nonEmpty: String? { isEmpty ? nil : self }
}

/// The nullable proof is mandatory in the current protocol. A null value
/// conveys no stop authority; only Application's exact admitted fact does.
nonisolated enum IOSCancellationAdmissionContract {
    static func confirmableTarget(envelope: [String: AgentCoreJSONValue], conversationID: String,
                                  locallyQuiescent: Set<String>, unconfirmed: Set<String>) throws -> String {
        let (target, admitted) = try self.target(envelope: envelope, conversationID: conversationID)
        guard !unconfirmed.contains(target), admitted || locallyQuiescent.contains(target) else {
            throw DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed
        }
        return target
    }

    static func target(envelope: [String: AgentCoreJSONValue], conversationID: String) throws -> (String, Bool) {
        guard case .object(let payload)? = envelope["payload"],
              Set(payload.keys) == ["type", "target_effect_id", "target_not_started"],
              payload.string("type") == "cancel",
              let target = payload.string("target_effect_id"), validID(target),
              let value = payload["target_not_started"] else { throw DeviceAgentCoordinatorError.invalidTransition }
        if value == .null { return (target, false) }
        guard case .object(let proof) = value,
              Set(proof.keys) == ["conversation_id", "run_id", "core_session_id", "host_id",
                "cancellation_effect_id", "cancellation_request_id", "target_effect_id",
                "target_request_id", "target_payload_identity", "checkpoint_revision", "revoked_attempts"],
              proof.string("conversation_id") == conversationID,
              proof.string("run_id") == envelope.string("turn_id"),
              proof.string("core_session_id") == envelope.string("session_id"),
              proof.string("host_id") == envelope.string("host_id"),
              proof.string("cancellation_effect_id") == envelope.string("effect_id"),
              proof.string("cancellation_request_id") == envelope.string("request_id"),
              proof.string("target_effect_id") == target,
              proof.string("cancellation_effect_id") != target,
              let revision = proof.int("checkpoint_revision"), revision > 0,
              let digest = proof.string("target_payload_identity"), digest.utf8.count == 64,
              digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              let revoked = proof.array("revoked_attempts"), revoked.count <= 256 else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        for key in ["conversation_id", "run_id", "core_session_id", "host_id",
                    "cancellation_effect_id", "cancellation_request_id", "target_effect_id", "target_request_id"] {
            guard let id = proof.string(key), validID(id) else { throw DeviceAgentCoordinatorError.invalidTransition }
        }
        var seen: Set<String> = []
        for item in revoked {
            guard case .object(let attempt) = item, Set(attempt.keys) == ["attempt_id", "revision"],
                  let id = attempt.string("attempt_id"), validID(id), seen.insert(id).inserted,
                  let revision = attempt.int("revision"), revision > 0 else { throw DeviceAgentCoordinatorError.invalidTransition }
        }
        return (target, true)
    }

    private static func validID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 1024
            && value == value.trimmingCharacters(in: .whitespacesAndNewlines)
            && !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
    }
}

private extension AgentCoreJSONValue {
    nonisolated var objectValue: [String: AgentCoreJSONValue]? {
        guard case .object(let value) = self else { return nil }
        return value
    }
}

private extension Dictionary where Key == String, Value == AgentCoreJSONValue {
    nonisolated func string(_ key: String) -> String? {
        guard case .string(let value)? = self[key] else { return nil }
        return value
    }

    nonisolated func bool(_ key: String) -> Bool? {
        guard case .bool(let value)? = self[key] else { return nil }
        return value
    }

    nonisolated func int(_ key: String) -> Int? {
        switch self[key] {
        case .number(let value): return Int(exactly: value)
        case .signedInteger(let value): return Int(exactly: value)
        case .unsignedInteger(let value): return Int(exactly: value)
        default: return nil
        }
    }

    nonisolated func array(_ key: String) -> [AgentCoreJSONValue]? {
        guard case .array(let value)? = self[key] else { return nil }
        return value
    }

    nonisolated func object(_ key: String) -> [String: AgentCoreJSONValue]? {
        guard case .object(let value)? = self[key] else { return nil }
        return value
    }
}
