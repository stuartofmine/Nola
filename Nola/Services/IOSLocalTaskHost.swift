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
    private let dependencies: AgentCoreJSONValue
    private let stateChanged: (@Sendable () async -> Void)?
    private var workers: [String: Task<Void, Error>] = [:]

    init(store: IOSApplicationRuntime, modelClient: AgentModelClient,
         invocation: IOSPreparedHostInvocation, toolPlan: AgentCorePreparedToolPlan,
         hostExecutor: IOSAgentHostExecutor, config: AgentCoreRuntimeConfig,
         childPermissionEvents: (@Sendable (IOSChildPermissionEvent) async -> Void)? = nil,
         stateChanged: (@Sendable () async -> Void)? = nil,
         dependencies: AgentCoreJSONValue) {
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
        let candidate = try store.executeAgentCommand(.object([
            "type": .string("load_mobile_task_child_configuration"), "task_id": .string(taskID),
        ]))
        guard let fields = candidate.objectFields,
              let identity = fields["turn_identity"]?.objectFields,
              let binding = identity["host_invocation"]?.objectFields,
              identity["host_dependencies"] == dependencies else {
            throw IOSApplicationRuntimeError.rejected("Child requires its own frozen model executor")
        }
        guard let frozenCoreSession = fields["session_id"]?.stringScalar, !frozenCoreSession.isEmpty else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        guard let route = binding["primary"], let generation = binding["generation"],
              let output = binding["output_admission"], let budget = fields["context_window"],
              let capability = identity["model_capability"] else { throw IOSApplicationRuntimeError.invalidResponse }
        let childInvocation = IOSPreparedHostInvocation(route: route, generation: generation, outputAdmission: output)
        let independentClient: any AgentModelClient
        if route == invocation.route, generation == invocation.generation, output == invocation.outputAdmission,
           let factory = modelClient as? any AgentIndependentModelClientFactory {
            independentClient = await factory.makeIndependentAgentModelClient()
        } else {
            independentClient = try await MainActor.run {
                let prepared = try IOSPreparedRunModel.recover(hostInvocation: .object(binding), contextBudget: budget, using: store)
                return try TextAgentModelClient(client: prepared.client, frozenModelCapability: capability)
            }
        }
        let plan = try await childToolPlan(child: frozenCoreSession, configuration: candidate)
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
        let canonical = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: child, conversationTitle: "", identity: try .init(
                recoveringSubmissionID: runID, assistantMessageID: assistant),
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: runID))
        let skills = try IOSPreparedHostInvocation.decodeSkillIdentity(dependencies)
        await hostExecutor.acquireAgentSession(sessionId: coreSessionID)
        defer { Task { await hostExecutor.releaseAgentSession(sessionId: coreSessionID) } }
        await hostExecutor.installSkillSnapshot(skills, sessionId: coreSessionID)
        let executor = DeviceAgentCoordinator(modelClient: independentClient, childOnly: true,
            childPermissionOwner: .init(taskID: taskID, parentConversationID: parent,
                childConversationID: child, runID: runID, coreSessionID: coreSessionID),
            childPermissionEvents: childPermissionEvents,
            hostExecutor: hostExecutor, toolPlan: plan,
            canonicalApplication: canonical, preparedHostInvocation: childInvocation,
            hostDependencyIdentity: dependencies)
        let pump = try store.startChildPump(.object([
            "task_id": .string(taskID), "parent_conversation_id": .string(parent),
            "conversation_id": .string(child), "run_id": .string(runID),
            "assistant_message_id": .string(assistant), "attempt": attempt,
            "worker": worker, "current": current, "cancellation_only": cancellationOnly,
            "mailbox_capacity": .number(16),
        ]))
        try await IOSChildRunHost(pump: pump, executor: executor,
            conversationID: child, runID: runID, coreSessionID: coreSessionID, config: config).run()
    }

    private func childToolPlan(child: String, configuration: AgentCoreJSONValue) async throws -> AgentCorePreparedToolPlan {
        guard let original = toolPlan.originalIntent,
              case .array(let tools)? = configuration.objectFields?["tools"] else {
            throw IOSApplicationRuntimeError.rejected("Frozen child tool policy is unavailable")
        }
        let allowed = Set(tools.compactMap { $0.objectFields?["function"]?.objectFields?["name"]?.stringScalar })
        let observation = await MainActor.run { () -> (Bool, [String: String]) in
            let foreground = UIApplication.shared.applicationState == .active
            return (foreground, AgentCoreToolCatalog.authorizationSnapshot(isForeground: foreground))
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
