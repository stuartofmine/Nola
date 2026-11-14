// SPDX-License-Identifier: Apache-2.0
// Native dependencies for Core-admitted embedded children. No task state machine.
import Foundation
import UIKit

actor IOSLocalTaskHost {
    private let store: IOSApplicationRuntime
    private let modelClient: AgentModelClient
    private let invocation: IOSPreparedHostInvocation
    private let toolPlan: AgentCorePreparedToolPlan
    private let hostExecutor: IOSAgentHostExecutor
    private let config: AgentCoreRuntimeConfig
    private let childPermissionEvents: (@Sendable (IOSChildPermissionEvent) async -> Void)?
    private let expertTargetConfiguration: (@Sendable (AppleExpertPeerMaterial<AgentCoreJSONValue>) async throws -> AgentCoreJSONValue)?
    private let dependencies: AgentCoreJSONValue
    private let stateChanged: (@Sendable () async -> Void)?
    private var workers: [String: Task<Void, Error>] = [:]

    init(store: IOSApplicationRuntime, modelClient: AgentModelClient,
         invocation: IOSPreparedHostInvocation, toolPlan: AgentCorePreparedToolPlan,
         hostExecutor: IOSAgentHostExecutor, config: AgentCoreRuntimeConfig,
         childPermissionEvents: (@Sendable (IOSChildPermissionEvent) async -> Void)? = nil,
         stateChanged: (@Sendable () async -> Void)? = nil,
         dependencies: AgentCoreJSONValue,
        expertTargetConfiguration: (@Sendable (AppleExpertPeerMaterial<AgentCoreJSONValue>) async throws -> AgentCoreJSONValue)? = nil) {
        self.expertTargetConfiguration = expertTargetConfiguration
        self.store = store; self.modelClient = modelClient; self.invocation = invocation
        self.toolPlan = toolPlan; self.hostExecutor = hostExecutor; self.config = config
        self.stateChanged = stateChanged
        self.childPermissionEvents = childPermissionEvents
        self.dependencies = dependencies
    }

    func prepare(_ request: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        try store.executeAgentCommand(.object(["type": .string("prepare_mobile_delegated_task"),
            "input": .object(["request": request, "now_ms": .signedInteger(Self.now())])]))
    }

    func prepareExpert(_ request: AgentCoreJSONValue, contextID: String, runID: String) async throws -> AgentCoreJSONValue {
        try await Self.prepareExpert(request, contextID: contextID, runID: runID, store: store,
            expertTargetConfiguration: expertTargetConfiguration)
    }

    static func prepareExpert(_ request: AgentCoreJSONValue, contextID: String, runID: String,
        store: IOSApplicationRuntime,
        expertTargetConfiguration: (@Sendable (AppleExpertPeerMaterial<AgentCoreJSONValue>) async throws -> AgentCoreJSONValue)?) async throws -> AgentCoreJSONValue {
        let bound = try IOSExpertPeerHostRequest(request: request, expectedContextID: contextID, expectedRunID: runID)
        guard bound.arguments["action"] == .string("request"),
              let target = bound.arguments["target_participant_id"]?.stringScalar,
              let text = bound.arguments["text"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
        let authority: AppleExpertTemplateAuthority = try store.expertHostOperation(.derivePeerAuthority(request: request, authority: .local()))
        let prior: AppleExpertPeerMaterial<AgentCoreJSONValue>? = try store.expertHostOperation(.loadPeerForEffect(request: request, authority: authority))
        let input = AppleExpertPeerInput(sourceContextID: contextID, sourceRunID: runID, callID: bound.callID,
            targetParticipantID: target, kind: "request", text: text, inReplyTo: bound.arguments["in_reply_to"]?.stringScalar)
        let reconciled = try store.reconcilePendingExpertPeerAdmission(input, authority: authority)
        if let prior, let reconciled {
            guard prior.view.exchangeID == reconciled.view.exchangeID,
                  prior.view.source == reconciled.view.source, prior.view.target == reconciled.view.target else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
        let material: AppleExpertPeerMaterial<AgentCoreJSONValue>
        if let reconciled { material = reconciled }
        else if let prior { material = prior }
        else {
            guard !bound.isLegacyRecovery else { throw IOSApplicationRuntimeError.rejected("Legacy expert dispatch can only recover an existing Task") }
            material = try store.captureExpertPeer(.init(sourceContextID: contextID, sourceRunID: runID, callID: bound.callID,
            targetParticipantID: target, kind: "request", text: text, inReplyTo: bound.arguments["in_reply_to"]?.stringScalar), authority: authority) }
        do {
        try Task.checkCancellation()
        let retained = try store.executeAgentCommand(.object(["type": .string("runtime_task"),
            "command": .object(["type": .string("get"), "id": .string(material.view.exchangeID)])]))
        let metadata = try retained.objectFields?["metadata_json"]?.stringScalar.map {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data($0.utf8))
        }
        let configuration: AgentCoreJSONValue
        if retained != .null {
            guard let frozen = metadata?.objectFields?["expert_target_configuration"] else { throw IOSApplicationRuntimeError.invalidResponse }
            configuration = frozen
        } else {
            guard !bound.isLegacyRecovery else { throw IOSApplicationRuntimeError.rejected("Legacy expert dispatch can only recover an existing Task") }
            guard let expertTargetConfiguration else { throw IOSApplicationRuntimeError.rejected("This Host cannot capture a new expert configuration") }
            configuration = try await expertTargetConfiguration(material)
        }
        guard let projectRoot = configuration.objectFields?["turn_identity"]?.objectFields?["delegation"]?.objectFields?["project_root"]?.stringScalar,
              !projectRoot.isEmpty else { throw IOSApplicationRuntimeError.rejected("The target expert has no supported frozen Task configuration") }
        try Task.checkCancellation()
        let result: AgentCoreJSONValue = try store.expertHostOperation(.preparePeerTask(request: request, exchangeID: material.view.exchangeID,
            targetConfiguration: configuration, targetInvocation: nil, projectRoot: projectRoot, authority: authority))
        guard let prepared = result.objectFields?["prepared"]?.objectFields,
              prepared["task"]?.objectFields?["id"] == .string(material.view.exchangeID),
              prepared["task"]?.objectFields?["owner_session_id"] == .string(contextID),
              prepared["task"]?.objectFields?["child_session_id"] == .string(material.view.target.context_conversation_id),
              let response = result.objectFields?["response"]?.objectFields,
              let kind = response["type"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
        if prepared["source_admitted"] == .bool(true) {
            guard prepared["replayed"] == .bool(true), kind == "tool_completed",
                  response["call_id"] == .string(bound.callID),
                  response["output"]?.objectFields?["task_id"] == .string(material.view.exchangeID),
                  response["output"]?.objectFields?["exchange_id"] == .string(material.view.exchangeID),
                  response["output"]?.objectFields?["replayed"] == .bool(true) else { throw IOSApplicationRuntimeError.invalidResponse }
        } else {
            guard prepared["source_admitted"] == .bool(false),
                  ["task_promoted", "task_updated"].contains(kind) else { throw IOSApplicationRuntimeError.invalidResponse }
        }
        return result
        } catch IOSApplicationRuntimeError.outcomePending(let id) { throw IOSApplicationRuntimeError.outcomePending(id) }
        catch {
            let cause = error
            let abort: AgentCoreJSONValue
            do { abort = try store.expertHostOperation(.abortPeerTaskPreparation(request: request,
                exchangeID: material.view.exchangeID, authority: authority)) }
            catch IOSApplicationRuntimeError.outcomePending(let id) { throw IOSApplicationRuntimeError.outcomePending(id) }
            catch { throw IOSExpertTaskPreparationError(exchangeID: material.view.exchangeID, reason: cause.localizedDescription, taskMayExist: true) }
            if cause is CancellationError { throw cause }
            let aborted = abort.objectFields?["preparation_aborted"] == .bool(true)
                && abort.objectFields?["execution_started"] == .bool(false)
            let mayExist = abort.objectFields?["task_may_exist"] == .bool(true)
            guard aborted || mayExist else {
                throw IOSExpertTaskPreparationError(exchangeID: material.view.exchangeID, reason: cause.localizedDescription, taskMayExist: true)
            }
            throw IOSExpertTaskPreparationError(exchangeID: material.view.exchangeID, reason: cause.localizedDescription,
                taskMayExist: mayExist, preparationAborted: aborted)
        }
    }

    func expertAction(_ request: AgentCoreJSONValue, contextID: String, runID: String) async throws -> AgentCoreJSONValue {
        _ = try IOSExpertPeerHostRequest(request: request, expectedContextID: contextID, expectedRunID: runID)
        let authority: AppleExpertTemplateAuthority = try store.expertHostOperation(.derivePeerAuthority(request: request, authority: .local()))
        let clock = ContinuousClock(); var deadline: ContinuousClock.Instant?
        while true {
            try Task.checkCancellation()
            let value: AgentCoreJSONValue = try store.expertHostOperation(.peerTaskAction(request: request, authority: authority))
            guard let fields = value.objectFields, let response = fields["response"], response != .null,
                  case .bool(let terminal)? = fields["terminal"] else {
                throw IOSExpertTaskPreparationError(exchangeID: request.objectFields?["payload"]?.objectFields?["input"]?.objectFields?["exchange_id"]?.stringScalar ?? "",
                    reason: "The expert Task action outcome could not be confirmed", taskMayExist: true)
            }
            let wait: Int64
            switch fields["wait_ms"] {
            case .number(let value): guard let count = Int64(exactly: value) else { throw IOSApplicationRuntimeError.invalidResponse }; wait = count
            case .signedInteger(let value): wait = value
            case .unsignedInteger(let value): guard let count = Int64(exactly: value) else { throw IOSApplicationRuntimeError.invalidResponse }; wait = count
            default: throw IOSApplicationRuntimeError.invalidResponse
            }
            guard (0...30_000).contains(wait) else { throw IOSApplicationRuntimeError.invalidResponse }
            if deadline == nil { deadline = clock.now.advanced(by: .milliseconds(wait)) }
            if terminal || wait == 0 || clock.now >= deadline! { return response }
            try await clock.sleep(until: min(deadline!, clock.now.advanced(by: .milliseconds(250))))
        }
    }

    func guide(_ request: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        try store.executeAgentCommand(.object(["type": .string("prepare_mobile_task_guidance"),
            "input": .object(["request": request, "now_ms": .signedInteger(Self.now())])]))
    }

    func observe(_ request: AgentCoreJSONValue) async throws -> AgentCoreJSONValue {
        let clock = ContinuousClock()
        var deadline: ContinuousClock.Instant?
        while true {
            try Task.checkCancellation()
            let value = try store.executeAgentCommand(.object(["type": .string("observe_mobile_task"),
                "input": .object(["request": request, "now_ms": .signedInteger(Self.now())])]))
            guard let fields = value.objectFields, let response = fields["response"],
                  case .bool(let terminal)? = fields["terminal"],
                  case .number(let wait)? = fields["wait_ms"] else { throw IOSApplicationRuntimeError.invalidResponse }
            if deadline == nil { deadline = clock.now.advanced(by: .milliseconds(Int64(wait))) }
            if terminal || clock.now >= deadline! { return response }
            try await clock.sleep(until: min(deadline!, clock.now.advanced(by: .milliseconds(250))))
        }
    }

    /// Discovery is not an execution grant. Each candidate still passes the
    /// shared exact source/lease/config admission before a pump is opened.
    func recoverOwnedTasks(parentConversationID: String) throws {
        let value = try store.executeAgentCommand(.object([
            "type": .string("runtime_task"), "command": .object(["type": .string("list_all")]),
        ]))
        guard case .array(let tasks) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        for task in tasks {
            guard let fields = task.objectFields,
                  fields["kind"]?.stringScalar == "agent",
                  fields["owner_session_id"]?.stringScalar == parentConversationID,
                  let status = fields["status"]?.stringScalar,
                  ["queued", "running"].contains(status) else { continue }
            try wake(.object(["task": task]))
        }
    }

    /// Called only after the parent's TaskPromoted response commit ACK.
    /// Duplicate wakeups share one retained native owner; Core still owns claim.
    func wake(_ prepared: AgentCoreJSONValue) throws {
        guard let task = prepared.objectFields?["task"]?.objectFields,
              let taskID = task["id"]?.stringScalar,
              let parent = task["owner_session_id"]?.stringScalar,
              let child = task["child_session_id"]?.stringScalar else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        guard workers[taskID] == nil else { return }
        workers[taskID] = Task { [self] in
            defer { workers.removeValue(forKey: taskID) }
            do { try await execute(taskID: taskID, parent: parent, child: child) }
            catch {
                let reason = error.localizedDescription
                await MainActor.run {
                    Logger.shared.log(.warn, "IOSLocalTaskHost", "Embedded child execution remains unconfirmed",
                        metadata: ["taskId": taskID, "error": reason])
                }
                throw error
            }
        }
    }

    private func execute(taskID: String, parent: String, child: String) async throws {
        let task = try store.executeAgentCommand(.object(["type": .string("runtime_task"),
            "command": .object(["type": .string("get"), "id": .string(taskID)])]))
        guard task.objectFields?["owner_session_id"] == .string(parent), task.objectFields?["child_session_id"] == .string(child) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let candidate = try store.executeAgentCommand(.object([
            "type": .string("load_mobile_task_child_configuration"), "task_id": .string(taskID),
        ]))
        guard let fields = candidate.objectFields,
              let identity = fields["turn_identity"]?.objectFields,
              let binding = identity["host_invocation"]?.objectFields,
              let targetDependencies = identity["host_dependencies"] else {
            throw IOSApplicationRuntimeError.rejected("Child requires its own frozen model executor")
        }
        let metadata = try task.objectFields?["metadata_json"]?.stringScalar.map {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data($0.utf8))
        }
        // This is a preparation hint only. Execution authority comes from the
        // later Core admission's typed expert_target proof.
        let expertCandidate = metadata?.objectFields?["expert_target_configuration"] == candidate
            && metadata?.objectFields?["expert_target"] != nil
        guard expertCandidate || targetDependencies == dependencies else {
            throw IOSApplicationRuntimeError.rejected("Child Skill dependencies differ from its captured owner")
        }
        guard let frozenCoreSession = fields["session_id"]?.stringScalar, !frozenCoreSession.isEmpty else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        guard let route = binding["primary"], let generation = binding["generation"],
              let output = binding["output_admission"], let budget = fields["context_window"],
              let capability = identity["model_capability"] else { throw IOSApplicationRuntimeError.invalidResponse }
        let childInvocation = IOSPreparedHostInvocation(route: route, generation: generation, outputAdmission: output)
        let independentClient: any AgentModelClient
        if !expertCandidate, route == invocation.route, generation == invocation.generation, output == invocation.outputAdmission,
           let factory = modelClient as? any AgentIndependentModelClientFactory {
            independentClient = await factory.makeIndependentAgentModelClient()
        } else {
            independentClient = try await MainActor.run {
                let prepared = try IOSPreparedRunModel.recover(hostInvocation: .object(binding), contextBudget: budget, using: store)
                return try TextAgentModelClient(client: prepared.client, frozenModelCapability: capability)
            }
        }
        let childRuntimeConfig: AgentCoreRuntimeConfig
        if expertCandidate {
            struct Budget: Decodable { let context_window_tokens: Int; let reserved_output_tokens: Int }
            let values = try JSONDecoder().decode(Budget.self, from: JSONEncoder().encode(budget))
            guard let model = fields["model"]?.stringScalar, !model.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
            childRuntimeConfig = try await MainActor.run {
                guard let id = UUID(uuidString: child), id.uuidString == child,
                      let conversation = try LocalConversationStore(applicationRuntime: store).getConversation(conversationId: id) else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                return AgentCoreRuntimeConfig(platform: .ios, executionMode: .deviceLocal, hostId: config.hostId, model: model,
                    maxIterations: 0, contextWindowTokens: values.context_window_tokens, reservedOutputTokens: values.reserved_output_tokens,
                    grantedCapabilities: [], permissionProfile: conversation.permissionMode)
            }
        } else { childRuntimeConfig = config }
        let plan = try await childToolPlan(child: frozenCoreSession, configuration: candidate, expertCandidate: expertCandidate)
        // This exact typed worker is reused by both admission and the pump.
        let worker = IOSHostProcessObservation.currentWorkerIdentity()
        let now = Self.now()
        let admitted = try store.executeAgentCommand(.object([
            "type": .string("admit_mobile_task_child"), "input": .object([
                "task_id": .string(taskID), "parent_conversation_id": .string(parent),
                "worker": worker, "now_ms": .signedInteger(now),
                "lease_until_ms": .signedInteger(now + 60_000), "candidate_config": candidate,
                "current_tools": .array(plan.schemas),
            ]),
        ]))
        if admitted == .null { return }
        defer { Task { await stateChanged?() } }
        await stateChanged?()
        guard let admission = admitted.objectFields,
              admission["task_id"]?.stringScalar == taskID,
              admission["parent_conversation_id"]?.stringScalar == parent,
              let attempt = admission["attempt"], let current = admission["current"],
              let cancellationOnly = admission["cancellation_only"],
              case .bool = cancellationOnly,
              let exact = admission["invocation"]?.objectFields,
              exact["conversation_id"]?.stringScalar == child,
              let runID = exact["run_id"]?.stringScalar,
              let coreSessionID = exact["core_session_id"]?.stringScalar, coreSessionID == frozenCoreSession,
              let assistant = exact["assistant_message_id"]?.stringScalar else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let expertTarget: AppleExpertTaskTargetV1?
        if let proof = admission["expert_target"], proof != .null {
            let decoded = try JSONDecoder().decode(AppleExpertTaskTargetV1.self, from: JSONEncoder().encode(proof))
            try decoded.validate(taskID: taskID, contextID: child, runID: runID)
            guard expertCandidate, decoded.source.context_conversation_id == parent else { throw IOSApplicationRuntimeError.invalidResponse }
            expertTarget = decoded
        } else {
            guard !expertCandidate else { throw IOSApplicationRuntimeError.rejected("The common Task admission did not authorize this expert target") }
            expertTarget = nil
        }
        let canonical = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: child, conversationTitle: "", identity: try .init(
                recoveringSubmissionID: runID, assistantMessageID: assistant),
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: runID))
        let skills = try IOSPreparedHostInvocation.decodeSkillIdentity(targetDependencies)
        await hostExecutor.acquireAgentSession(sessionId: coreSessionID)
        defer { Task { await hostExecutor.releaseAgentSession(sessionId: coreSessionID) } }
        await hostExecutor.installSkillSnapshot(skills, sessionId: coreSessionID)
        let executor = DeviceAgentCoordinator(modelClient: independentClient, childOnly: true,
            childPermissionOwner: .init(taskID: taskID, parentConversationID: parent,
                childConversationID: child, runID: runID, coreSessionID: coreSessionID),
            childPermissionEvents: childPermissionEvents, childTaskStateChanged: stateChanged,
            hostExecutor: hostExecutor, toolPlan: plan,
            canonicalApplication: canonical, preparedHostInvocation: childInvocation,
            hostDependencyIdentity: targetDependencies, expertTargetConfiguration: expertTargetConfiguration, expertTaskTarget: expertTarget)
        let pump = try store.startChildPump(.object([
            "task_id": .string(taskID), "parent_conversation_id": .string(parent),
            "conversation_id": .string(child), "run_id": .string(runID),
            "assistant_message_id": .string(assistant), "attempt": attempt,
            "worker": worker, "current": current, "cancellation_only": cancellationOnly,
            "mailbox_capacity": .number(16),
        ]))
        try await IOSChildRunHost(pump: pump, executor: executor,
            conversationID: child, runID: runID, coreSessionID: coreSessionID, config: childRuntimeConfig).run()
    }

    private func childToolPlan(child: String, configuration: AgentCoreJSONValue, expertCandidate: Bool) async throws -> AgentCorePreparedToolPlan {
        guard let original = toolPlan.originalIntent,
              case .array(let tools)? = configuration.objectFields?["tools"] else {
            throw IOSApplicationRuntimeError.rejected("Frozen child tool policy is unavailable")
        }
        let allowed = Set(tools.compactMap { $0.objectFields?["function"]?.objectFields?["name"]?.stringScalar })
        let observation = await MainActor.run { () -> (Bool, [String: String]) in
            let foreground = UIApplication.shared.applicationState == .active
            return (foreground, AgentCoreToolCatalog.authorizationSnapshot(isForeground: foreground))
        }
        if expertCandidate {
            return try await MainActor.run {
                try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
                    expectedIdentity: .init(hostId: original.expectedIdentity?.hostId,
                        clientId: original.expectedIdentity?.clientId, sessionId: child),
                    isForeground: observation.0, allowedTools: allowed, scopedGrants: [], applicationRuntime: store, includeExpertTasks: false)
            }
        }
        let now = Self.now(), manifest = original.runtime
        let identity = AgentCoreExpectedIdentity(hostId: original.expectedIdentity?.hostId,
            clientId: original.expectedIdentity?.clientId, sessionId: child)
        let input = AgentCoreToolPolicyInput(platform: original.platform, nowMs: now,
            expectedIdentity: identity, runtime: .init(
                manifestId: "nola.ios.child-handlers.\(UUID().uuidString.lowercased())",
                manifestVersion: manifest.manifestVersion, issuedAtMs: now, expiresAtMs: now + 60_000,
                hostId: identity.hostId, clientId: identity.clientId, sessionId: child,
                routes: manifest.routes, capabilities: manifest.capabilities,
                authorizations: observation.1, resultEgressReceipts: [],
                tools: manifest.tools.filter { allowed.contains($0.toolId) }),
            conversation: .init(allowedTools: allowed.sorted()), userPolicy: original.userPolicy,
            scopedGrants: [], resultEgressGrants: [])
        let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(input)
        return .init(output: output, expectedIdentity: identity,
            authorizationSnapshot: observation.1, isForeground: observation.0,
            scopedGrants: [], resultEgressGrants: [], originalIntent: input)
    }

    private static func now() -> Int64 { Int64(Date().timeIntervalSince1970 * 1_000) }
}
