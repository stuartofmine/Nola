// SPDX-License-Identifier: Apache-2.0
// Typed iOS adapter for the canonical Application Run boundary.

import Foundation
import CryptoKit
import Darwin

private extension Dictionary where Key == String, Value == AgentCoreJSONValue {
    nonisolated func iosString(_ key: String) -> String? {
        guard case .string(let value)? = self[key] else { return nil }
        return value
    }

    nonisolated func iosArray(_ key: String) -> [AgentCoreJSONValue] {
        guard case .array(let value)? = self[key] else { return [] }
        return value
    }

    nonisolated func iosInt64(_ key: String) -> Int64? {
        switch self[key] {
        case .signedInteger(let value): return value
        case .unsignedInteger(let value): return Int64(exactly: value)
        case .number(let value): return Int64(exactly: value)
        default: return nil
        }
    }
    nonisolated func iosUInt64(_ key: String) -> UInt64? {
        switch self[key] {
        case .unsignedInteger(let value): return value
        case .signedInteger(let value): return UInt64(exactly: value)
        case .number(let value): return UInt64(exactly: value)
        default: return nil
        }
    }
}

nonisolated protocol IOSAgentApplicationCommandStore: Sendable {
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue
}

extension IOSApplicationRuntime: IOSAgentApplicationCommandStore {}

nonisolated enum IOSCanonicalAgentApplicationError: LocalizedError, Equatable {
    case invalidResult(String)
    case recoveryDecisionRequired

    var errorDescription: String? {
        switch self {
        case .invalidResult(let detail):
            return "Canonical Agent Application returned an invalid result: \(detail)"
        case .recoveryDecisionRequired:
            return "The previous operation has an unknown outcome. Review its recovery decision before continuing."
        }
    }
}

/// Transport envelope only. An opaque validated binding is not a reconstructed provider.
nonisolated struct IOSExactRunInvocation: Sendable, Equatable {
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let invocationConfigDigest: String
    let status: String
    let hostInvocation: AgentCoreJSONValue
    let contextBudget: AgentCoreJSONValue
    let remoteToolPlan: AgentCoreJSONValue
    let coreSessionID: String?
    let checkpointRevision: Int64?
    let checkpointEventCursor: Int64?
    let checkpointSHA256: String?

    // Core ResumeHostEffect may advance only the checkpoint witness while the
    // admitted invocation stays frozen. Live authority is validated by Core.
    func hasSameFrozenInvocation(as other: Self) -> Bool {
        status == other.status && hasSameInvocationAuthority(as: other)
    }

    func hasSameInvocationAuthority(as other: Self) -> Bool {
        conversationID == other.conversationID && runID == other.runID &&
        assistantMessageID == other.assistantMessageID &&
        invocationConfigDigest == other.invocationConfigDigest &&
        hostInvocation == other.hostInvocation && contextBudget == other.contextBudget &&
        remoteToolPlan == other.remoteToolPlan &&
        coreSessionID == other.coreSessionID
    }
}

nonisolated enum IOSCanonicalRunRecovery: Sendable {
    case transition(AgentCoreJSONValue)
    case terminal(IOSAssistantRunTerminalSeal)
}

nonisolated struct IOSQueuedUserAdmission: Sendable {
    let commandID: String
    let leaseOwner: String
    let deliveryAttempt: Int64
}

nonisolated struct IOSCanonicalAgentRunDescriptor: Sendable {
    let conversationID: String
    let conversationTitle: String
    let identity: IOSCanonicalAgentTurnIdentity
    let userText: String
    let userContentJSON: String?
    let contextCheckpointID: String?
    var automationLease: IOSAutomationExecutionLease? = nil
    var automationSpec: IOSAutomationExecutionSpec? = nil
    var boundRunID: String? = nil
    var expectedConfigurationRevision: Int64? = nil
    var expectedParentMessageID: String? = nil
    var queuedUserAdmission: IOSQueuedUserAdmission? = nil
    var runID: String { boundRunID ?? identity.submissionID }
}

/// One user cancellation is one immutable command, including its operation
/// clock. A transport retry must recover that command, not create a new intent.
nonisolated struct IOSCanonicalCancellationOperation: Sendable, Equatable {
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let invocationConfigDigest: String
    let cancellationID: String
    let transitionID: String
    let committedAtMs: Int64
    let reason: String

    var command: AgentCoreJSONValue {
        .object(["type": .string("reduce_and_commit_active_submission"), "input": .object([
            "conversation_id": .string(conversationID), "run_id": .string(runID),
            "transition_id": .string(transitionID),
            "invocation_config_digest": .string(invocationConfigDigest),
            "committed_at_ms": .signedInteger(committedAtMs),
            "submission": .object(["type": .string("interrupt"),
                "cancellation_id": .string(cancellationID), "reason": .string(reason)]),
        ])])
    }
}

/// Owns one canonical interactive Run. It translates the platform-neutral
/// Application result into the transition shape consumed by the Host driver;
/// it does not reduce Core state or persist projections itself.
nonisolated final class IOSCanonicalAgentApplication: @unchecked Sendable {
    private let store: any IOSAgentApplicationCommandStore
    private let descriptor: IOSCanonicalAgentRunDescriptor
    private let submissionCommitted: (@Sendable () -> Void)?
    private let stateLock = NSLock()
    private var invocationConfigDigest: String?
    private var runActivated = false
    private var permissionControlOwner: [String: AgentCoreJSONValue]?
    private var currentPermissionProjection: AgentCoreJSONValue?
    private var permissionReadCursor: UInt64 = 0
    private var cancellationOperation: IOSCanonicalCancellationOperation?
    private var workerIdentity: (generation: String, owner: String)?
    // Core caps one delivery lease extension at five minutes. Stay below the
    // boundary so clock skew cannot turn a valid claim into a rejected one.
    private static let effectLeaseMilliseconds: Int64 = 4 * 60 * 1_000

    var embeddedStore: IOSApplicationRuntime? { store as? IOSApplicationRuntime }
    var conversationID: String { descriptor.conversationID }
    var runID: String { descriptor.runID }

    init(
        store: any IOSAgentApplicationCommandStore,
        descriptor: IOSCanonicalAgentRunDescriptor,
        submissionCommitted: (@Sendable () -> Void)? = nil
    ) {
        self.store = store
        self.descriptor = descriptor
        self.submissionCommitted = submissionCommitted
    }

    /// Reuses the shared immutable guidance outbox; Core alone decides whether
    /// this exact task has a live child binding and can accept the instruction.
    func taskGuidanceTransport(path: String, payload: Data, parentTurnID: String) throws -> Data {
        try AppleTaskGuidanceCoreTransport.exchange(path: path, payload: payload,
            conversationId: descriptor.conversationID, parentRunId: descriptor.runID,
            parentTurnId: parentTurnID) { command in
                let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: command)
                return try JSONEncoder().encode(store.executeAgentCommand(value))
            }
    }

    /// Persists only cancellation intent under Core's exact started-delivery
    /// fence. It never claims that a child worker has stopped.
    func taskCancellationResponse(request: AgentCoreJSONValue, delivery: AgentCoreJSONValue?, nowMs: Int64) throws -> AgentCoreJSONValue {
        guard case .object(var envelope) = request,
              case .object(let payload)? = envelope["payload"], payload["type"] == .string("cancel_task"),
              let taskID = payload.iosString("task_id") else { throw IOSApplicationRuntimeError.invalidResponse }
        if let delivery { envelope["delivery"] = delivery }
        let result = try store.executeAgentCommand(.object([
            "type": .string("execute_task_cancellation_for_effect"), "request": .object(envelope),
            "now_ms": .signedInteger(nowMs),
        ]))
        guard case .object(let value) = result else { throw IOSApplicationRuntimeError.invalidResponse }
        switch value.iosString("kind") {
        case "applied":
            guard Set(value.keys) == ["kind", "cancel_requested"], case .bool(let requested)? = value["cancel_requested"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return .object(["type": .string("completed"),
                "output": .object(["task_id": .string(taskID), "cancel_requested": .bool(requested)]),
                "usage": .null, "full_output_ref": .null, "artifacts": .array([])])
        case "transfer":
            return .object(["type": .string("failed"), "error": .object([
                "code": .string("task_cancellation_transfer_unsupported"),
                "message": .string("This mobile Host cannot transfer task cancellation to a different execution Store."),
                "retryable": .bool(false),
            ])])
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
    }

    private func encodedWorkerIdentity() throws -> (String, String) {
        stateLock.lock()
        defer { stateLock.unlock() }
        if let workerIdentity { return (workerIdentity.generation, workerIdentity.owner) }
        let result = try store.executeAgentCommand(.object([
            "type": .string("encode_host_worker_identity"),
            "identity": IOSHostProcessObservation.currentWorkerIdentity(),
        ]))
        guard case .object(let value) = result,
              Set(value.keys) == Set(["worker_generation", "lease_owner"]),
              let generation = value.iosString("worker_generation"), !generation.isEmpty,
              let owner = value.iosString("lease_owner"), !owner.isEmpty else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Core worker identity is unavailable")
        }
        workerIdentity = (generation, owner)
        return (generation, owner)
    }

    func observeTerminatedCancellationTarget(control: AgentCoreJSONValue) throws -> String {
        let plan = try store.executeAgentCommand(.object([
            "type": .string("inspect_cancellation_process_owners"),
            "identity": .object(["conversation_id": .string(descriptor.conversationID),
                "expected_run_id": .string(descriptor.runID),
                "assistant_message_id": .string(descriptor.identity.assistantMessageID)]),
            "control": control,
        ]))
        guard case .object(let fields) = plan, case .array(let owners)? = fields["target_owners"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Cancellation process plan is invalid")
        }
        let observations = try owners.map { owner -> AgentCoreJSONValue in
            guard case .object(let fields) = owner, let pid = fields.iosInt64("pid"),
                  let pid = Int32(exactly: pid), pid > 0 else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Cancellation process owner is invalid")
            }
            return .object(["owner": owner, "observation": IOSHostProcessObservation.observe(pid: pid)])
        }
        let target = try store.executeAgentCommand(.object([
            "type": .string("validate_cancellation_process_observations"), "plan": plan,
            "current_process": IOSHostProcessObservation.observe(pid: getpid()),
            "observations": .array(observations),
        ]))
        guard case .object(let request) = target, let effect = request.iosString("effect_id"),
              case .object(let original) = control, case .object(let payload)? = original["payload"],
              payload["target_effect_id"] == .string(effect),
              request["session_id"] == original["session_id"], request["turn_id"] == original["turn_id"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Core cancellation target changed")
        }
        return effect
    }

    func stepCompactionSummary(_ step: [String: AgentCoreJSONValue]) throws -> [String: AgentCoreJSONValue] {
        let value = try store.executeAgentCommand(.object(["type": .string("step_compaction_summary"), "step": .object(step)]))
        guard case .object(let result) = value else { throw IOSCanonicalAgentApplicationError.invalidResult("Invalid compaction plan step") }
        return result
    }

    func evaluateCompactionMaintenance(_ facts: [String: AgentCoreJSONValue]) throws -> [String: AgentCoreJSONValue] {
        let value = try store.executeAgentCommand(.object([
            "type": .string("evaluate_compaction_maintenance"), "facts": .object(facts)
        ]))
        guard case .object(let decision) = value else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Invalid maintenance decision")
        }
        return decision
    }

    func begin(
        coreConfig: AgentCoreJSONValue,
        submission: AgentCoreJSONValue,
        createdAtMs: Int64
    ) throws -> AgentCoreJSONValue {
        let mayBegin = runIsInactive()
        guard mayBegin else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Canonical Agent Run is already active")
        }
        var actualConfig = coreConfig
        if let spec = descriptor.automationSpec {
            guard descriptor.automationLease != nil, case .object(var config) = actualConfig,
                  case .deviceLocal(let hostID) = spec.executionScope,
                  case .object(let catalog)? = config["host_catalog"], catalog["host_id"] == .string(hostID),
                  descriptor.userText == spec.prompt else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Automation Host scope does not match actual execution")
            }
            config["automation_execution_spec"] = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(spec))
            actualConfig = .object(config)
        }
        var command: [String: AgentCoreJSONValue] = [
            "type": .string(descriptor.automationLease == nil ? "begin_assistant_turn" : "begin_automation_assistant_turn"),
            "input": .object([
                "conversation_id": .string(descriptor.conversationID),
                "conversation_title": .string(descriptor.conversationTitle),
                "user_message_id": .string(descriptor.identity.userMessageID),
                "user_part_id": .string("\(descriptor.identity.userMessageID):text"),
                "user_text": .string(descriptor.userText),
                "user_content_json": descriptor.userContentJSON.map(AgentCoreJSONValue.string) ?? .null,
                "assistant_message_id": .string(descriptor.identity.assistantMessageID),
                "context_checkpoint_id": descriptor.contextCheckpointID.map(AgentCoreJSONValue.string) ?? .null,
                "expected_configuration_revision": descriptor.expectedConfigurationRevision.map(AgentCoreJSONValue.signedInteger) ?? .null,
                "expected_parent_message_id": descriptor.expectedParentMessageID.map(AgentCoreJSONValue.string) ?? .null,
                "core_config": actualConfig,
                "submission": submission,
                "created_at_ms": .number(Double(createdAtMs)),
            ]),
        ]
        if let lease = descriptor.automationLease {
            guard descriptor.automationSpec != nil else { throw IOSCanonicalAgentApplicationError.invalidResult("Automation definition spec is missing") }
            command["lease"] = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(lease))
        }
        if let queued = descriptor.queuedUserAdmission {
            guard descriptor.automationLease == nil, descriptor.automationSpec == nil,
                  queued.commandID == descriptor.identity.submissionID,
                  let revision = descriptor.expectedConfigurationRevision,
                  case .object(let fields) = submission,
                  fields["type"] == .string("user_input"),
                  let contextItems = fields["turn_context_items"],
                  let platform = fields["turn_context_platform"],
                  let components = fields["turn_context_components"] else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Invalid queued user admission")
            }
            command = ["type": .string("begin_queued_user_turn"), "input": .object([
                "conversation_id": .string(descriptor.conversationID),
                "command_id": .string(queued.commandID), "lease_owner": .string(queued.leaseOwner),
                "delivery_attempt": .signedInteger(queued.deliveryAttempt),
                "expected_configuration_revision": .signedInteger(revision),
                "expected_parent_message_id": descriptor.expectedParentMessageID.map(AgentCoreJSONValue.string) ?? .null,
                "core_config": actualConfig, "now_ms": .signedInteger(createdAtMs),
                "turn_context_items": contextItems, "turn_context_platform": platform,
                "turn_context_components": components,
            ])]
        }
        let result = try store.executeAgentCommand(.object(command))
        guard case .object(let resultObject) = result,
              case .string(let digest)? = resultObject["invocation_config_digest"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Run invocation config digest is missing"
            )
        }
        guard case .object(let execution)? = resultObject["execution"],
              case .object(let control)? = execution["control"],
              case .object(let permissions)? = execution["permissions"],
              let revision = control["permission_projection_revision"], revision != .null,
              revision == permissions["revision"],
              case .array? = permissions["pending"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Admission execution state is missing or inconsistent")
        }
        var normalized = resultObject
        normalized["control"] = .object(control)
        guard case .object(var translated) = try Self.transition(from: .object(normalized), hostRequests: []) else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Invalid admission control transition")
        }
        // Validate the admission contract before claiming any committed work.
        activateRun(invocationConfigDigest: digest)
        submissionCommitted?()
        translated["host_requests"] = .array(try claimCommittedHostRequests(claimedAtMs: createdAtMs))
        translated["permission_projection"] = .object(permissions)
        return .object(translated)
    }

    static func recoveryIdentity(store: any IOSAgentApplicationCommandStore,
                                 conversationID: String, expectedRunID: String) throws -> IOSCanonicalAgentTurnIdentity {
        let result = try store.executeAgentCommand(.object([
            "type": .string("active_run"), "conversation_id": .string(conversationID)]))
        guard case .object(let active) = result,
              active.iosString("run_id") == expectedRunID,
              let assistantID = active.iosString("assistant_message_id") else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Active recovery Run changed")
        }
        return try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: expectedRunID,
            assistantMessageID: assistantID)
    }

    private func loadFrozenState(_ frozen: IOSExactRunInvocation) throws -> [String: AgentCoreJSONValue] {
        guard frozen.conversationID == descriptor.conversationID, frozen.runID == descriptor.runID,
              frozen.assistantMessageID == descriptor.identity.assistantMessageID,
              let expectedRevision = frozen.checkpointRevision, let expectedCursor = frozen.checkpointEventCursor,
              let expectedHash = frozen.checkpointSHA256, let expectedSession = frozen.coreSessionID else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Frozen Skill snapshot lacks an exact Run witness")
        }
        let result = try store.executeAgentCommand(.object([
            "type": .string("load_checkpoint"), "conversation_id": .string(frozen.conversationID),
        ]))
        guard case .object(let checkpoint) = result,
              checkpoint.iosString("conversation_id") == frozen.conversationID,
              checkpoint.iosInt64("revision") == expectedRevision,
              checkpoint.iosInt64("event_cursor") == expectedCursor,
              let raw = checkpoint.iosString("checkpoint_json"),
              SHA256.hash(data: Data(raw.utf8)).map({ String(format: "%02x", $0) }).joined() == expectedHash,
              case .object(let decoded) = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(raw.utf8)),
              decoded.iosString("session_id") == expectedSession,
              case .object(let state)? = decoded["state"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Frozen Skill snapshot checkpoint changed or is incomplete")
        }
        return state
    }

    func loadFrozenSkillIdentity(_ frozen: IOSExactRunInvocation) throws -> AgentCoreJSONValue {
        let state = try loadFrozenState(frozen)
        guard let dependencies = state["configuration"]?.objectFields?["turn_identity"]?.objectFields?["host_dependencies"] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        _ = try IOSPreparedHostInvocation.decodeSkillIdentity(dependencies)
        return dependencies
    }

    func loadFrozenInputOrigin(_ frozen: IOSExactRunInvocation) throws -> AgentCoreInputOrigin {
        let state = try loadFrozenState(frozen)
        guard let turn = state["execution"]?.objectFields?["active_turn"]?.objectFields,
              let raw = turn["context"]?.objectFields?["input_origin"]?.stringScalar,
              let origin = AgentCoreInputOrigin(rawValue: raw) else { throw IOSApplicationRuntimeError.invalidResponse }
        return origin
    }

    func validateRecoveryAuthority(_ frozen: IOSExactRunInvocation, candidate: AgentCoreJSONValue) throws {
        guard frozen.conversationID == descriptor.conversationID, frozen.runID == descriptor.runID,
              frozen.assistantMessageID == descriptor.identity.assistantMessageID,
              case .object(let fields) = candidate,
              case .object(let identity)? = fields["turn_identity"],
              let invocation = identity["host_invocation"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Recovery authority owner or candidate is invalid")
        }
        var current: [String: AgentCoreJSONValue] = ["host_invocation": invocation]
        for key in ["host_catalog", "tools", "permission_rules", "context_window"] {
            guard let value = fields[key] else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Recovery authority is missing \(key)")
            }
            current[key] = value
        }
        let result = try store.executeAgentCommand(.object([
            "type": .string("validate_mobile_run_recovery_authority"),
            "conversation_id": .string(frozen.conversationID), "expected_run_id": .string(frozen.runID),
            "assistant_message_id": .string(frozen.assistantMessageID),
            "expected_invocation_config_digest": .string(frozen.invocationConfigDigest),
            "current": .object(current),
        ]))
        guard case .object(let proof) = result,
              Set(proof.keys) == Set(["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest"]),
              proof["conversation_id"] == .string(frozen.conversationID),
              proof["run_id"] == .string(frozen.runID),
              proof["assistant_message_id"] == .string(frozen.assistantMessageID),
              proof["invocation_config_digest"] == .string(frozen.invocationConfigDigest) else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Core returned a different recovery authority")
        }
    }

    func inspectMobilePermissionRequest(_ request: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        try store.executeAgentCommand(.object(["type": .string("inspect_mobile_permission_request"),
            "request": request, "now_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1_000))]))
    }

    func loadExactRunInvocation() throws -> IOSExactRunInvocation {
        let result = try store.executeAgentCommand(.object([
            "type": .string("load_exact_run_invocation"),
            "conversation_id": .string(descriptor.conversationID),
            "expected_run_id": .string(descriptor.runID),
            "assistant_message_id": .string(descriptor.identity.assistantMessageID),
        ]))
        return try Self.decodeExactRunInvocation(result, conversationID: descriptor.conversationID,
            runID: descriptor.runID, assistantMessageID: descriptor.identity.assistantMessageID)
    }

    static func decodeExactRunInvocation(_ result: AgentCoreJSONValue, conversationID: String,
        runID: String, assistantMessageID: String) throws -> IOSExactRunInvocation {
        let keys: Set<String> = ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest", "status", "host_invocation", "context_budget",
            "core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256", "remote_tool_plan"]
        guard case .object(let value) = result, Set(value.keys) == keys,
              value.iosString("conversation_id") == conversationID,
              value.iosString("run_id") == runID,
              value.iosString("assistant_message_id") == assistantMessageID,
              let digest = value.iosString("invocation_config_digest"), digest.utf8.count == 64,
              digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              let remoteToolPlan = value["remote_tool_plan"],
              remoteToolPlan == .null || { if case .object = remoteToolPlan { return true }; return false }(),
              let status = value.iosString("status"), !status.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              case .object(let binding)? = value["host_invocation"], !binding.isEmpty,
              case .object(let context)? = value["context_budget"],
              Set(context.keys) == Set(["context_window_tokens", "reserved_output_tokens", "prompt_token_limit", "auto_compact_tokens", "compaction_target_tokens", "auto_compact_percent", "compaction_target_percent"]),
              case .number(let version)? = binding["version"], version.isFinite,
              version >= 1, version <= Double(UInt32.max), version.rounded() == version else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Frozen Run invocation is missing or has invalid identity/shape")
        }
        let proofKeys = ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"]
        let hasCheckpoint = !proofKeys.allSatisfy { value[$0] == .null }
        let coreSessionID = value.iosString("core_session_id")
        let checkpointRevision = value.iosInt64("checkpoint_revision")
        let checkpointEventCursor = value.iosInt64("checkpoint_event_cursor")
        let checkpointSHA256 = value.iosString("checkpoint_sha256")
        if hasCheckpoint {
            guard let coreSessionID, !coreSessionID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let checkpointRevision, checkpointRevision >= 0,
                  let checkpointEventCursor, checkpointEventCursor >= 0,
                  let checkpointSHA256, checkpointSHA256.utf8.count == 64,
                  checkpointSHA256.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Exact Run checkpoint witness is partial or malformed")
            }
        }
        return IOSExactRunInvocation(conversationID: conversationID,
            runID: runID, assistantMessageID: assistantMessageID,
            invocationConfigDigest: digest, status: status, hostInvocation: .object(binding), contextBudget: .object(context), remoteToolPlan: remoteToolPlan,
            coreSessionID: coreSessionID, checkpointRevision: checkpointRevision,
            checkpointEventCursor: checkpointEventCursor, checkpointSHA256: checkpointSHA256)
    }

    /// Replays only this exact Run's committed diagnostics to a fixed frontier.
    /// Text/reasoning remain owned by the separate canonical assistant journal.
    func recoveryTraceEvents(afterSequence: Int64) throws -> [AgentTraceEvent] {
        var cursor = afterSequence
        var through: Int64?
        var traceID: String?
        var events: [AgentTraceEvent] = []
        repeat {
            let result = try store.executeAgentCommand(.object([
                "type": .string("load_exact_run_trace_replay"),
                "conversation_id": .string(descriptor.conversationID),
                "run_id": .string(descriptor.runID),
                "assistant_message_id": .string(descriptor.identity.assistantMessageID),
                "after_sequence": .signedInteger(cursor),
                "through_sequence": through.map(AgentCoreJSONValue.signedInteger) ?? .null,
                "limit": .number(256),
            ]))
            guard case .object(let page) = result,
                  Set(page.keys) == ["trace_id", "through_sequence", "next_sequence", "events"],
                  let frontier = page.iosInt64("through_sequence"), frontier >= cursor,
                  let next = page.iosInt64("next_sequence"), next >= cursor, next <= frontier,
                  case .array(let values)? = page["events"], values.count <= 256,
                  next - cursor == Int64(values.count),
                  through == nil || through == frontier,
                  traceID == nil || traceID == page.iosString("trace_id") else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Exact trace replay page is invalid")
            }
            through = frontier
            traceID = page.iosString("trace_id")
            for value in values {
                let event = try JSONDecoder().decode(AgentTraceEvent.self, from: JSONEncoder().encode(value))
                guard event.conversationId == descriptor.conversationID,
                      event.runId == descriptor.runID, event.messageId == descriptor.identity.assistantMessageID,
                      event.traceId == traceID, event.sequence == UInt64(cursor + 1) else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Exact trace replay event owner or cursor changed")
                }
                events.append(event)
                cursor += 1
            }
            guard cursor == next, cursor == frontier || !values.isEmpty else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Exact trace replay did not advance")
            }
        } while cursor < (through ?? cursor)
        return events
    }

    /// The source Store's exact immutable Run seal is completion authority.
    /// Trace events and stream EOF are not evidence that all work committed.
    func committedTerminalSeal(coreSessionID: String) throws -> IOSAssistantRunTerminalSeal? {
        let result = try store.executeAgentCommand(.object([
            "type": .string("local_assistant_run_terminal_seal"),
            "conversation_id": .string(descriptor.conversationID),
            "run_id": .string(descriptor.runID),
        ]))
        if result == .null { return nil }
        let seal = try JSONDecoder().decode(IOSAssistantRunTerminalSeal.self, from: JSONEncoder().encode(result))
        guard seal.protocol_version == 2,
              seal.identity.conversation_id == descriptor.conversationID,
              seal.identity.run_id == descriptor.runID,
              seal.identity.assistant_message_id == descriptor.identity.assistantMessageID,
              seal.core_session_id == coreSessionID,
              seal.result.run_id == descriptor.runID,
              seal.result.conversation_id == descriptor.conversationID,
              seal.seal_digest.utf8.count == 64,
              seal.seal_digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Terminal seal does not match the exact local Run owner")
        }
        return seal
    }

    func recoverPendingPermissionWait(
        prepareRecovery: (@Sendable (IOSExactRunInvocation) async throws -> Void)? = nil,
        nowMilliseconds: @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970 * 1_000) },
        sleep: @Sendable (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) async throws -> AgentCoreJSONValue {
        switch try await recoverExactRun(permissionOnly: true, prepareRecovery: prepareRecovery,
            nowMilliseconds: nowMilliseconds, sleep: sleep) {
        case .transition(let transition): return transition
        case .terminal:
            throw IOSCanonicalAgentApplicationError.invalidResult("The exact permission wait already completed")
        }
    }

    /// Reclaims the existing exact Run through the shared Application work schedule.
    /// Frozen invocation validation and physical delivery claims remain mandatory;
    /// missing/ambiguous work is never replaced by a fresh submission.
    func recoverExactRun(
        permissionOnly: Bool = false,
        prepareRecovery: (@Sendable (IOSExactRunInvocation) async throws -> Void)? = nil,
        nowMilliseconds: @Sendable () -> Int64 = {
            Int64(Date().timeIntervalSince1970 * 1_000)
        },
        sleep: @Sendable (UInt64) async throws -> Void = { nanoseconds in
            try await Task.sleep(nanoseconds: nanoseconds)
        }
    ) async throws -> IOSCanonicalRunRecovery {
        try Task.checkCancellation()
        let mayRecover = runIsInactive()
        guard mayRecover else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Canonical Agent Run is already active")
        }
        var frozen = try loadExactRunInvocation()
        guard let coreSessionID = frozen.coreSessionID, !coreSessionID.isEmpty else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Permission recovery has no committed Core identity")
        }
        guard let prepareRecovery else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Frozen provider reconstruction and current authorization validation are unavailable")
        }
        // This gate must reconstruct dependencies from frozen values and revalidate
        // live authority. Merely decoding a binding version does not authorize claim.
        try await prepareRecovery(frozen)
        try Task.checkCancellation()
        guard try loadExactRunInvocation() == frozen else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run invocation changed while preparing recovery")
        }
        let active = try store.executeAgentCommand(.object([
            "type": .string("active_run"),
            "conversation_id": .string(descriptor.conversationID),
        ]))
        guard case .object(let activeRun) = active,
              activeRun.iosString("run_id") == descriptor.runID,
              activeRun.iosString("kind") == "conversation",
              activeRun.iosString("assistant_message_id") == descriptor.identity.assistantMessageID,
              let digest = activeRun.iosString("invocation_config_digest"),
              digest == frozen.invocationConfigDigest else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Active Agent Run identity or invocation config digest does not match recovery"
            )
        }
        // The exact persisted owner is already verified. Cancellation must be
        // available while an old delivery lease is still draining; claiming its
        // successor is not a prerequisite for asking Core to stop that owner.
        rememberRecoveryCancellationIdentity(digest)
        let committed = try store.executeAgentCommand(.object([
            "type": .string("load_committed_host_requests"),
            "run_id": .string(descriptor.runID),
        ]))
        guard case .object(let committedObject) = committed,
              case .array(let committedRequests)? = committedObject["requests"],
              !committedRequests.isEmpty,
              (!permissionOnly || committedRequests.allSatisfy(Self.isPermissionRequest)) else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Process recovery refuses to claim a non-permission Host effect"
            )
        }

        while true {
            try Task.checkCancellation()
            let now = nowMilliseconds()
            let prepared = try store.executeAgentCommand(.object([
                "type": .string("prepare_run_effect_recovery"),
                "input": .object([
                    "conversation_id": .string(descriptor.conversationID),
                    "run_id": .string(descriptor.runID),
                    "invocation_config_digest": .string(frozen.invocationConfigDigest),
                    "now_ms": .signedInteger(now),
                ]),
            ]))
            guard case .object(let preparation) = prepared,
                  let rebound = preparation.iosInt64("rebound_count"), rebound >= 0 else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Invalid Core recovery preparation result")
            }
            if rebound > 0 {
                let refreshed = try loadExactRunInvocation()
                guard refreshed.hasSameFrozenInvocation(as: frozen) else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Frozen Run changed during effect recovery")
                }
                try await prepareRecovery(refreshed)
                frozen = refreshed
                let committed = try store.executeAgentCommand(.object([
                    "type": .string("load_committed_host_requests"),
                    "run_id": .string(descriptor.runID),
                ]))
                guard case .object(let fields) = committed,
                      case .array(let requests)? = fields["requests"], !requests.isEmpty,
                      (!permissionOnly || requests.allSatisfy(Self.isPermissionRequest)) else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Resumed Run has no owned recoverable requests")
                }
            }
            let schedule = try store.executeAgentCommand(.object([
                "type": .string("load_runtime_effect_work_schedule"),
                "input": .object([
                    "conversation_ids": .array([.string(descriptor.conversationID)]),
                    "now_ms": .number(Double(now)),
                    "limit": .number(1),
                ]),
            ]))
            guard case .object(let scheduleObject) = schedule else {
                throw IOSCanonicalAgentApplicationError.invalidResult(
                    "Permission wait has no effect-work schedule"
                )
            }
            let ready = scheduleObject.iosArray("runs").first { value in
                guard case .object(let run) = value else { return false }
                return run.iosString("run_id") == descriptor.runID
            }
            if case .object(let work)? = ready {
                guard let reason = work.iosString("reason"), ["pending", "retry", "resume", "recover_expired_lease"].contains(reason) else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Unknown recovery work reason")
                }
                if reason != "recover_expired_lease" { break }
                // Schedule readiness does not change a leased delivery into a
                // claimable successor. Reconcile the exact frozen Run first.
                try await prepareRecovery(frozen)
                try Task.checkCancellation()
                guard try loadExactRunInvocation() == frozen else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Run changed before expired delivery reconciliation")
                }
                let interruptedAt = nowMilliseconds()
                _ = try store.executeAgentCommand(.object([
                    "type": .string("recover_expired_run_deliveries"),
                    "conversation_id": .string(descriptor.conversationID),
                    "expected_run_id": .string(descriptor.runID),
                    "host_invocation": frozen.hostInvocation,
                    "interrupted_at": .string(ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: Double(interruptedAt) / 1_000))),
                    "interrupted_at_ms": .signedInteger(interruptedAt),
                ]))
                guard try loadExactRunInvocation() == frozen else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Run changed during expired delivery reconciliation")
                }
                continue // Re-observe: an intervening heartbeat may have renewed the lease.
            }
            guard let wakeAt = scheduleObject.iosInt64("next_wake_at_ms") else {
                // Unknown physical outcomes are deliberately absent from the
                // runnable schedule. Ask Core for the exact decision owner;
                // never invent a wake time or turn this into replayable work.
                let value = try store.executeAgentCommand(.object([
                    "type": .string("inspect_unknown_effect_recovery"),
                    "conversation_id": .string(descriptor.conversationID),
                    "expected_run_id": .string(descriptor.runID),
                    "assistant_message_id": .string(descriptor.identity.assistantMessageID),
                ]))
                if value != .null {
                    let inspection = try JSONDecoder().decode(IOSUnknownEffectRecoveryInspection.self,
                        from: JSONEncoder().encode(value))
                    guard inspection.conversation_id == descriptor.conversationID,
                          inspection.run_id == descriptor.runID,
                          inspection.assistant_message_id == descriptor.identity.assistantMessageID,
                          inspection.invocation_config_digest == frozen.invocationConfigDigest,
                          !inspection.targets.isEmpty, inspection.targets.count <= 256 else {
                        throw IOSCanonicalAgentApplicationError.invalidResult("Unknown recovery decision changed owner")
                    }
                    throw IOSCanonicalAgentApplicationError.recoveryDecisionRequired
                }
                throw IOSCanonicalAgentApplicationError.invalidResult(
                    "Permission wait has no recoverable wake time"
                )
            }
            switch try reconcileObservedOrphanedDelivery(frozen: frozen, nowMilliseconds: nowMilliseconds) {
            case .reconciled: continue
            case .terminal(let seal): return .terminal(seal)
            case .waiting: break
            }
            let delayMilliseconds = max(1, wakeAt - now)
            try await sleep(UInt64(delayMilliseconds) * 1_000_000)
        }

        guard try loadExactRunInvocation() == frozen else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run invocation changed while waiting for permission recovery")
        }
        try await prepareRecovery(frozen)
        try Task.checkCancellation()
        // Rehydrate only execution controls, never the conversation/checkpoint.
        // The proof reread below fences this read against a concurrent Run change.
        let execution = try store.executeAgentCommand(.object([
            "type": .string("load_host_execution_state"),
            "conversation_id": .string(descriptor.conversationID),
        ]))
        guard case .object(let state) = execution,
              case .object(let control)? = state["control"],
              control.iosString("session_id") == coreSessionID,
              let turnID = control.iosString("active_turn_id"), !turnID.isEmpty,
              case .object(let permissions)? = state["permissions"],
              let revision = control["permission_projection_revision"], revision != .null,
              permissions["revision"] == revision,
              case .array? = permissions["pending"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Recovered execution control has an invalid owner or permission revision")
        }
        let taskRead = try store.executeAgentCommand(.object([
            "type": .string("load_conversation_task_projection_v1"),
            "conversation_id": .string(descriptor.conversationID),
        ]))
        guard case .object(let taskFields) = taskRead,
              taskFields.iosString("core_session_id") == coreSessionID,
              taskFields.iosInt64("checkpoint_revision") == frozen.checkpointRevision,
              taskFields.iosInt64("checkpoint_event_cursor") == frozen.checkpointEventCursor,
              let taskValue = taskFields["task_projection"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Recovered Tasks do not match the exact checkpoint witness")
        }
        let tasks = try JSONDecoder().decode(AgentCoreTaskProjection.self, from: JSONEncoder().encode(taskValue))
        var taskValidation = AgentCoreTaskProjectionReducer()
        guard taskValidation.restore(tasks, expectedSessionId: coreSessionID) == .applied else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Recovered Tasks have an invalid Core owner")
        }
        guard try loadExactRunInvocation() == frozen else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run invocation changed during final recovery authorization")
        }
        try Task.checkCancellation()
        let (workerGeneration, leaseOwner) = try encodedWorkerIdentity()
        let claimedAt = nowMilliseconds()
        let requests = try claimCommittedHostRequests(claimedAtMs: claimedAt)
        if permissionOnly {
            try Self.validateRecoveredPermissionRequests(requests, runID: descriptor.runID,
                sessionID: coreSessionID, workerGeneration: workerGeneration,
                leaseOwner: leaseOwner, claimedAtMs: claimedAt)
        } else {
            try Self.validateClaimedHostRequests(requests, workerGeneration: workerGeneration,
                leaseOwner: leaseOwner, claimedAtMs: claimedAt)
            guard !requests.isEmpty, requests.allSatisfy({ value in
                guard case .object(let request) = value else { return false }
                return request.iosString("session_id") == coreSessionID
            }) else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Exact Run has no claimable, owned Host work")
            }
        }
        activateRun(invocationConfigDigest: digest)
        return .transition(.object([
            "control": .object(control), "permission_projection": .object(permissions),
            "task_projection": taskValue,
            "host_requests": .array(requests),
        ]))
    }

    /// The Host observes only the exact owners selected by Core. Unknown or
    /// live processes retain the normal lease-expiry path; no PID is guessed.
    private enum ProcessRecoveryObservation {
        case waiting, reconciled, terminal(IOSAssistantRunTerminalSeal)
    }

    private func reconcileObservedOrphanedDelivery(frozen: IOSExactRunInvocation,
        nowMilliseconds: () -> Int64) throws -> ProcessRecoveryObservation {
        do {
            let plan = try store.executeAgentCommand(.object([
                "type": .string("inspect_run_process_owners"),
                "conversation_id": .string(descriptor.conversationID),
                "expected_run_id": .string(descriptor.runID)]))
            guard case .object(let fields) = plan,
                  fields.iosString("conversation_id") == descriptor.conversationID,
                  fields.iosString("run_id") == descriptor.runID,
                  fields.iosString("assistant_message_id") == descriptor.identity.assistantMessageID,
                  fields.iosString("invocation_config_digest") == frozen.invocationConfigDigest,
                  fields["host_invocation"] == frozen.hostInvocation,
                  case .bool(let hasSessionLease)? = fields["has_session_lease"],
                  case .array(let owners)? = fields["process_owners"],
                  case .array(let deliveries)? = fields["deliveries"] else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Run process observation plan changed owner")
            }
            // Embedded iOS does not assert death for a separate session owner.
            guard !hasSessionLease else { return .waiting }
            let observations = try owners.map { owner -> AgentCoreJSONValue in
                guard case .object(let identity) = owner, let value = identity.iosInt64("pid"),
                      let pid = Int32(exactly: value), pid > 0 else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Run process owner has an invalid PID")
                }
                return .object(["owner": owner, "observation": IOSHostProcessObservation.observe(pid: pid)])
            }
            let validated = try store.executeAgentCommand(.object([
                "type": .string("validate_run_process_observations"), "plan": plan,
                "observations": .array(observations)]))
            guard case .bool(let terminated) = validated else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Invalid Run process observation result")
            }
            guard terminated else { return .waiting }
            let interruptedAt = nowMilliseconds()
            _ = try store.executeAgentCommand(.object([
                "type": .string("recover_observed_orphaned_run"),
                "conversation_id": .string(descriptor.conversationID), "expected_run_id": .string(descriptor.runID),
                "host_invocation": frozen.hostInvocation, "observed_session_lease": .null,
                "observed_deliveries": .array(deliveries),
                "interrupted_at": .string(ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: Double(interruptedAt) / 1_000))),
                "interrupted_at_ms": .signedInteger(interruptedAt)]))
            guard try loadExactRunInvocation() == frozen else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Frozen Run changed during observed process recovery")
            }
            return .reconciled
        } catch {
            // A legitimate pump may seal this exact owner between inspect and
            // validation. Only its immutable invocation and durable seal can
            // convert that race into observed completion.
            if let current = try? loadExactRunInvocation(), current.status == "sealed",
               current.hasSameInvocationAuthority(as: frozen),
               let session = current.coreSessionID,
               let seal = try? committedTerminalSeal(coreSessionID: session) {
                return .terminal(seal)
            }
            throw error
        }
    }

    private func runIsInactive() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return !runActivated
    }

    private func rememberRecoveryCancellationIdentity(_ digest: String) {
        stateLock.lock()
        invocationConfigDigest = digest
        stateLock.unlock()
    }

    private func activateRun(invocationConfigDigest digest: String) {
        stateLock.lock()
        invocationConfigDigest = digest
        runActivated = true
        stateLock.unlock()
    }

    func acceptHostResponse(
        _ response: AgentCoreJSONValue,
        receivedAtMs: Int64
    ) throws -> AgentCoreJSONValue {
        stateLock.lock()
        let currentDigest = invocationConfigDigest
        stateLock.unlock()
        guard let currentDigest else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run has not begun")
        }
        let result = try store.executeAgentCommand(.object([
            "type": .string("reduce_and_accept_host_response"),
            "input": .object([
                "conversation_id": .string(descriptor.conversationID),
                "run_id": .string(descriptor.runID),
                "invocation_config_digest": .string(currentDigest),
                "response": response,
                "received_at_ms": .number(Double(receivedAtMs)),
            ]),
        ]))
        return try committedControlTransition(from: result, committedAtMs: receivedAtMs)
    }

    private func committedControlTransition(from result: AgentCoreJSONValue,
                                            committedAtMs: Int64) throws -> AgentCoreJSONValue {
        guard case .object(var transition) = try Self.transition(from: result, hostRequests: []) else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Invalid control transition")
        }
        guard case .object(let fields) = result, case .object(let control)? = fields["control"] else {
            transition["host_requests"] = .array(try claimCommittedHostRequests(claimedAtMs: committedAtMs))
            return .object(transition)
        }
        guard let session = control.iosString("session_id"), !session.isEmpty,
              let turn = control["active_turn_id"],
              let revision = control["permission_projection_revision"],
              case .object(let cursor)? = control["last_event_cursor"], let sequence = cursor["sequence"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Control permission witness is incomplete")
        }
        let owner: [String: AgentCoreJSONValue] = ["session_id": .string(session), "active_turn_id": turn,
            "permission_projection_revision": revision]
        stateLock.lock()
        let needsQuery = permissionControlOwner != owner || currentPermissionProjection == nil
        stateLock.unlock()
        if needsQuery {
            var scope = owner
            scope["type"] = .string("control")
            scope["event_cursor"] = sequence
            let read = try store.executeAgentCommand(.object([
                "type": .string("load_conversation_permission_projection_v1"),
                "conversation_id": .string(descriptor.conversationID), "run_id": .string(descriptor.runID),
                "scope": .object(scope),
            ]))
            guard case .object(let response) = read,
                  response["conversation_id"] == .string(descriptor.conversationID),
                  response["run_id"] == .string(descriptor.runID),
                  owner.allSatisfy({ response[$0.key] == $0.value }),
                  let actualCursor = response.iosUInt64("event_cursor"), let expectedCursor = cursor.iosUInt64("sequence"),
                  actualCursor >= expectedCursor,
                  case .object(let permissions)? = response["permissions"], permissions["revision"] == revision,
                  case .array? = permissions["pending"],
                  Set(permissions.keys).isSubset(of: ["revision", "pending", "active_policy"]) else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Permission projection does not match the exact control owner")
            }
            if let policyValue = permissions["active_policy"], policyValue != .null {
                guard case .object(let policy) = policyValue,
                      Set(policy.keys) == ["run_id", "configuration_revision", "profile"],
                      policy["run_id"] == turn,
                      let policyRevision = policy.iosUInt64("configuration_revision"), policyRevision > 0,
                      let profile = policy.iosString("profile"), ["cautious", "allow_all"].contains(profile) else {
                    throw IOSCanonicalAgentApplicationError.invalidResult("Active permission policy has another Run owner or invalid configuration")
                }
            }
            stateLock.lock()
            guard actualCursor >= permissionReadCursor else {
                stateLock.unlock()
                throw IOSCanonicalAgentApplicationError.invalidResult("Permission read was superseded by a newer control")
            }
            permissionControlOwner = owner
            currentPermissionProjection = .object(permissions)
            permissionReadCursor = actualCursor
            stateLock.unlock()
        }
        stateLock.lock()
        let permissions = currentPermissionProjection
        let stillOwned = permissionControlOwner == owner
        stateLock.unlock()
        guard stillOwned else { throw IOSCanonicalAgentApplicationError.invalidResult("Permission control owner changed during read") }
        transition["permission_projection"] = permissions
        // Successor leases belong to this same operation's clock. Sampling a
        // second wall clock here can predate the accepted response/recovery.
        transition["host_requests"] = .array(try claimCommittedHostRequests(claimedAtMs: committedAtMs))
        return .object(transition)
    }

    func prepareCancellation(reason: String) throws -> IOSCanonicalCancellationOperation {
        stateLock.lock()
        defer { stateLock.unlock() }
        guard let digest = invocationConfigDigest else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run has not begun")
        }
        if let cancellationOperation {
            guard cancellationOperation.conversationID == descriptor.conversationID,
                  cancellationOperation.runID == descriptor.runID,
                  cancellationOperation.assistantMessageID == descriptor.identity.assistantMessageID,
                  cancellationOperation.invocationConfigDigest == digest else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Cancellation belongs to a different Run owner")
            }
            return cancellationOperation
        }
        let operation = IOSCanonicalCancellationOperation(conversationID: descriptor.conversationID,
            runID: descriptor.runID, assistantMessageID: descriptor.identity.assistantMessageID,
            invocationConfigDigest: digest, cancellationID: UUID().uuidString.lowercased(),
            transitionID: UUID().uuidString.lowercased(),
            committedAtMs: Int64(Date().timeIntervalSince1970 * 1_000), reason: reason)
        cancellationOperation = operation
        return operation
    }

    func submitCancellation(reason: String,
                            observedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)) throws -> AgentCoreJSONValue {
        let execution = try store.executeAgentCommand(.object(["type": .string("load_host_execution_state"),
            "conversation_id": .string(descriptor.conversationID)]))
        if let control = execution.objectFields?["control"]?.objectFields,
           control["active_turn_id"]?.stringScalar == descriptor.runID,
           ["cancelling", "cancelled"].contains(control["turn_status"]?.stringScalar ?? "") {
            let exact = try loadExactRunInvocation()
            guard exact.coreSessionID == control["session_id"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
            // A guarded Goal control may already have committed the exact
            // interruption before its native coordinator attached. Join it.
            return try committedControlTransition(from: .object(["events": .array([]), "traces": .array([]),
                "control": .object(control)]), committedAtMs: observedAtMs)
        }
        let operation = try prepareCancellation(reason: reason)
        // The runtime journals this exact command before send. Keep the frozen
        // operation on failure so a repeated request reuses that pending envelope.
        let result = try store.executeAgentCommand(operation.command)
        // Delivery leases belong to this observation, not the original intent
        // clock: a recovered ACK may arrive minutes after the persisted command.
        return try committedControlTransition(from: result, committedAtMs: max(observedAtMs, operation.committedAtMs))
    }

    func submitActive(
        _ submission: AgentCoreJSONValue,
        transitionID: String = UUID().uuidString.lowercased(),
        committedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) throws -> AgentCoreJSONValue {
        stateLock.lock()
        let currentDigest = invocationConfigDigest
        stateLock.unlock()
        guard let currentDigest else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Run has not begun")
        }
        let result = try store.executeAgentCommand(.object([
            "type": .string("reduce_and_commit_active_submission"),
            "input": .object([
                "conversation_id": .string(descriptor.conversationID),
            "run_id": .string(descriptor.runID),
                "transition_id": .string(transitionID),
                "invocation_config_digest": .string(currentDigest),
                "submission": submission,
                "committed_at_ms": .number(Double(committedAtMs)),
            ]),
        ]))
        return try committedControlTransition(from: result, committedAtMs: committedAtMs)
    }

    func uiCapabilityContract() throws -> AgentCoreJSONValue {
        try store.executeAgentCommand(.object(["type": .string("read_ui_capability")]))
    }

    /// Publication consumes the durable effect payload in Core. No renderer or
    /// model supplied owner is forwarded to the store.
    func publishUISurface(request: AgentCoreJSONValue, delivery: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard case .object(let envelope) = request,
              let effectID = envelope.iosString("effect_id"),
              let requestID = envelope.iosString("request_id") else {
            throw IOSCanonicalAgentApplicationError.invalidResult("UI publication lacks its effect identity")
        }
        return try store.executeAgentCommand(.object([
            "type": .string("execute_ui_tool"),
            "input": .object([
                "effect_id": .string(effectID), "request_id": .string(requestID),
                "delivery": delivery,
                "now_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)),
            ]),
        ]))
    }

    /// Consume a durable configuration notice through Application. This method
    /// cannot manufacture policy from a picker or revive a resolved challenge.
    func applyPendingPermissionPolicy() throws -> AgentCoreJSONValue? {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let (_, leaseOwner) = try encodedWorkerIdentity()
        let terminal = try store.executeAgentCommand(.object([
            "type": .string("apply_pending_task_notice"),
            "conversation_id": .string(descriptor.conversationID),
            "consumer_id": .string(leaseOwner), "now_ms": .signedInteger(now),
        ]))
        if terminal != .null {
            return try committedControlTransition(from: terminal, committedAtMs: now)
        }
        let head = try store.executeAgentCommand(.object([
            "type": .string("peek_agent_command"), "session_id": .string(descriptor.conversationID),
        ]))
        guard head != .null else { return nil }
        let claimed = try store.executeAgentCommand(.object([
            "type": .string("claim_next_agent_command"), "session_id": .string(descriptor.conversationID),
            "lease_owner": .string(leaseOwner), "now_ms": .signedInteger(now),
            "lease_until_ms": .signedInteger(now + Self.effectLeaseMilliseconds),
        ]))
        guard claimed != .null else { return nil }
        guard case .object(let lease) = claimed, lease.iosString("session_id") == descriptor.conversationID,
              lease.iosString("lease_owner") == leaseOwner,
              lease.iosInt64("lease_until_ms").map({ $0 > now }) == true,
              lease.iosInt64("delivery_attempt").map({ $0 > 0 }) == true,
              let commandID = lease.iosString("command_id") else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Policy notice has another lease owner")
        }
        func settle(_ type: String) throws {
            _ = try store.executeAgentCommand(.object(["type": .string(type),
                "session_id": .string(descriptor.conversationID), "command_id": .string(commandID),
                "lease_owner": .string(leaseOwner)]))
        }
        do {
            guard let payloadJSON = lease.iosString("payload_json"), let data = payloadJSON.data(using: .utf8),
                  case .object(let payload) = try JSONDecoder().decode(AgentCoreJSONValue.self, from: data) else {
                try settle("release_agent_command_lease")
                return nil
            }
            if payload.iosString("type") == "submission",
               case .object(let submission)? = payload["submission"],
               submission.iosString("type") == "user_input",
               submission.iosString("submission_id") == commandID,
               let submittedAt = submission.iosInt64("submitted_at_epoch_ms") {
                let transition = try submitActive(.object(submission),
                    transitionID: "queued-input:\(commandID)", committedAtMs: submittedAt)
                try settle("acknowledge_agent_command")
                return transition
            }
            guard payload.iosString("type") == "permission_policy_available",
                  case .object(let policy)? = payload["policy"],
                  let run = policy.iosString("run_id"), let revision = policy.iosInt64("configuration_revision"), revision > 0,
                  commandID == "permission-policy:\(run):\(revision)" else {
                try settle("release_agent_command_lease")
                return nil
            }
            if run != descriptor.runID {
                let active = try store.executeAgentCommand(.object([
                    "type": .string("active_run"), "conversation_id": .string(descriptor.conversationID),
                ]))
                if case .object(let current) = active, current.iosString("run_id") == run {
                    try settle("release_agent_command_lease")
                    return nil
                }
            }
            let accepted = try store.executeAgentCommand(.object([
                "type": .string("apply_active_permission_policy"),
                "conversation_id": .string(descriptor.conversationID), "policy": .object(policy),
                "now_ms": .signedInteger(now),
            ]))
            guard run == descriptor.runID || accepted == .null else {
                throw IOSCanonicalAgentApplicationError.invalidResult("Another Run accepted this owner's policy notice")
            }
            let transition = try accepted == .null ? nil : committedControlTransition(from: accepted, committedAtMs: now)
            try settle("acknowledge_agent_command")
            return transition
        } catch {
            try? settle("release_agent_command_lease")
            throw error
        }
    }

    func renewHostDelivery(
        _ delivery: AgentCoreJSONValue,
        heartbeatAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) throws -> AgentCoreJSONValue {
        guard case .object(let lease) = delivery,
              case .string(let attemptID)? = lease["attempt_id"],
              case .string(let generation)? = lease["worker_generation"],
              case .string(let owner)? = lease["lease_owner"],
              case .string(let token)? = lease["lease_token"],
              case .number(let expiry)? = lease["lease_expires_at_ms"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Host-effect delivery lease is incomplete"
            )
        }
        let (workerGeneration, leaseOwner) = try encodedWorkerIdentity()
        guard generation == workerGeneration, owner == leaseOwner else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Host-effect delivery lease belongs to another iOS worker"
            )
        }
        return try store.executeAgentCommand(.object([
            "type": .string("heartbeat_effect_delivery"),
            "input": .object([
            "run_id": .string(descriptor.runID),
                "attempt_id": .string(attemptID),
                "worker_generation": .string(generation),
                "lease_owner": .string(owner),
                "lease_token": .string(token),
                "current_lease_expires_at_ms": .number(expiry),
                "heartbeat_at_ms": .number(Double(heartbeatAtMs)),
                "requested_lease_expires_at_ms": .number(
                    Double(heartbeatAtMs + Self.effectLeaseMilliseconds)
                ),
            ]),
        ]))
    }

    private func claimCommittedHostRequests(claimedAtMs claimedAt: Int64) throws -> [AgentCoreJSONValue] {
        let (workerGeneration, leaseOwner) = try encodedWorkerIdentity()
        let result = try store.executeAgentCommand(.object([
            "type": .string("claim_committed_host_requests"),
            "input": .object([
            "run_id": .string(descriptor.runID),
                "worker_generation": .string(workerGeneration),
                "lease_owner": .string(leaseOwner),
                "claimed_at_ms": .number(Double(claimedAt)),
                "lease_expires_at_ms": .number(Double(claimedAt + Self.effectLeaseMilliseconds)),
                "limit": .number(64),
            ]),
        ]))
        guard case .object(let object) = result,
              case .array(let requests)? = object["requests"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Claimed Host requests are missing"
            )
        }
        try Self.validateClaimedHostRequests(requests, workerGeneration: workerGeneration,
            leaseOwner: leaseOwner, claimedAtMs: claimedAt)
        return requests
    }

    /// Model attempt identity is optional and independent of the physical delivery lease.
    /// Permission and cancellation effects need a delivery identity, not a model attempt.
    static func validateClaimedHostRequests(
        _ requests: [AgentCoreJSONValue],
        workerGeneration: String,
        leaseOwner: String,
        claimedAtMs: Int64
    ) throws {
        guard !workerGeneration.isEmpty, !leaseOwner.isEmpty else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Host delivery owner is missing")
        }
        for value in requests {
            guard case .object(let request) = value,
                  case .object(let delivery)? = request["delivery"],
                  let attempt = delivery.iosString("attempt_id"), !attempt.isEmpty,
                  delivery.iosString("worker_generation") == workerGeneration,
                  delivery.iosString("lease_owner") == leaseOwner,
                  let token = delivery.iosString("lease_token"), !token.isEmpty,
                  let expiry = delivery.iosInt64("lease_expires_at_ms"), expiry > claimedAtMs else {
                throw IOSCanonicalAgentApplicationError.invalidResult(
                    "Claimed Host request has invalid effect-delivery identity or lease"
                )
            }
        }
    }

    private static func isPermissionRequest(_ value: AgentCoreJSONValue) -> Bool {
        guard case .object(let request) = value,
              case .object(let payload)? = request["payload"] else { return false }
        return payload.iosString("type") == "request_permission"
    }

    static func validateRecoveredPermissionRequests(
        _ requests: [AgentCoreJSONValue],
        runID: String,
        sessionID: String,
        workerGeneration: String,
        leaseOwner: String,
        claimedAtMs: Int64
    ) throws {
        guard !runID.isEmpty, !sessionID.isEmpty, !workerGeneration.isEmpty,
              !leaseOwner.isEmpty, !requests.isEmpty else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Permission recovery has no claimed Host request"
            )
        }
        try validateClaimedHostRequests(requests, workerGeneration: workerGeneration,
            leaseOwner: leaseOwner, claimedAtMs: claimedAtMs)
        for value in requests {
            guard case .object(let request) = value,
                  request.iosString("session_id") == sessionID,
                  Self.isPermissionRequest(value) else {
                throw IOSCanonicalAgentApplicationError.invalidResult(
                    "Recovered permission request has invalid identity, lease, or effect type"
                )
            }
        }
    }

    private static func transition(
        from result: AgentCoreJSONValue,
        hostRequests: [AgentCoreJSONValue]
    ) throws -> AgentCoreJSONValue {
        guard case .object(let object) = result,
              case .array(let events)? = object["events"],
              case .array(let traces)? = object["traces"] else {
            throw IOSCanonicalAgentApplicationError.invalidResult(
                "Events or traces are missing"
            )
        }
        guard object["snapshot"] == nil && object["checkpoint"] == nil else {
            throw IOSCanonicalAgentApplicationError.invalidResult("Execution replies cannot carry cumulative state")
        }
        let control: AgentCoreJSONValue?
        switch object["control"] {
        case .object(let value): control = .object(value)
        case nil, .null:
            control = nil
        default:
            throw IOSCanonicalAgentApplicationError.invalidResult("Invalid execution control state")
        }
        var transition: [String: AgentCoreJSONValue] = [
            "event_envelopes": .array(events),
            "trace_events": .array(traces),
            "host_requests": .array(hostRequests),
        ]
        // Begin returns publication cursors directly; accepted transitions
        // return them in the commit receipt. Neither contains assistant bodies.
        if case .object(let commit)? = object["commit"], case .array(let publications)? = commit["assistant_publications"] {
            transition["assistant_publications"] = .array(publications)
        } else if case .array(let publications)? = object["assistant_publications"] {
            transition["assistant_publications"] = .array(publications)
        }
        if let control { transition["control"] = control }
        return .object(transition)
    }
}

/// Runs Core-authored long-term-memory work without exposing checkpoints or
/// reducer configuration to the platform Host.
nonisolated struct IOSMemoryWorkerRunReport: Equatable, Sendable {
    var attempted = 0
    var completed = 0
    var retryScheduled = 0

    static func + (lhs: Self, rhs: Self) -> Self {
        Self(
            attempted: lhs.attempted + rhs.attempted,
            completed: lhs.completed + rhs.completed,
            retryScheduled: lhs.retryScheduled + rhs.retryScheduled
        )
    }
}

nonisolated final class IOSCanonicalMemoryWorkerService: @unchecked Sendable {
    private let store: any IOSAgentApplicationCommandStore

    init(store: any IOSAgentApplicationCommandStore) {
        self.store = store
    }

    func hasClaimableJobs(conversationID: String) throws -> Bool {
        guard !conversationID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw AgentCoreRuntimeError.core("A conversation ID is required for Memory Workers.")
        }
        let query = try store.executeAgentCommand(.object([
            "type": .string("query_claimable_memory_jobs"),
            "input": .object([
                "conversation_id": .string(conversationID),
                "now_epoch_ms": .number(Double(Self.nowMilliseconds())),
            ]),
        ]))
        guard case .array(let jobs) = query else {
            throw AgentCoreRuntimeError.core("Application returned an invalid Memory Worker queue.")
        }
        return !jobs.isEmpty
    }

    func run(
        conversationID: String,
        coreConfig: AgentCoreJSONValue,
        modelClient: any AgentModelClient,
        maximumJobCount: Int = 2
    ) async throws -> Int {
        try await runReport(
            conversationID: conversationID,
            coreConfig: coreConfig,
            modelClient: modelClient,
            maximumJobCount: maximumJobCount
        ).completed
    }

    func runReport(
        conversationID: String,
        coreConfig: AgentCoreJSONValue,
        modelClient: any AgentModelClient,
        maximumJobCount: Int = 2
    ) async throws -> IOSMemoryWorkerRunReport {
        guard !conversationID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw AgentCoreRuntimeError.core("A conversation ID is required for Memory Workers.")
        }
        guard maximumJobCount > 0 else { return IOSMemoryWorkerRunReport() }
        let query = try store.executeAgentCommand(.object([
            "type": .string("query_claimable_memory_jobs"),
            "input": .object([
                "conversation_id": .string(conversationID),
                "now_epoch_ms": .number(Double(Self.nowMilliseconds())),
            ]),
        ]))
        guard case .array(let jobs) = query else {
            throw AgentCoreRuntimeError.core("Application returned an invalid Memory Worker queue.")
        }
        let workerID = "ios-memory-worker-\(UUID().uuidString.lowercased())"
        var report = IOSMemoryWorkerRunReport()
        for value in jobs.prefix(maximumJobCount) {
            guard case .object(let job) = value,
                  case .string(let jobID)? = job["id"],
                  !jobID.isEmpty else {
                throw AgentCoreRuntimeError.core("Core returned a Memory job without an id.")
            }
            let ownershipToken = UUID().uuidString.lowercased()
            let claimedAt = Self.nowMilliseconds()
            report.attempted += 1
            try commit(conversationID: conversationID, coreConfig: coreConfig, submission: .object([
                "type": .string("claim_memory_worker"),
                "operation_id": .string("ios-memory-claim-\(UUID().uuidString.lowercased())"),
                "job_id": .string(jobID),
                "worker_id": .string(workerID),
                "ownership_token": .string(ownershipToken),
                "now_epoch_ms": .number(Double(claimedAt)),
                "lease_until_epoch_ms": .number(Double(claimedAt + Self.leaseMilliseconds)),
            ]))
            do {
                let plan = try store.executeAgentCommand(.object([
                    "type": .string("build_memory_worker_plan"),
                    "input": .object([
                        "conversation_id": .string(conversationID),
                        "job_id": .string(jobID),
                    ]),
                ]))
                guard case .object(let planObject) = plan,
                      case .string(let watermark)? = planObject["input_watermark"] else {
                    throw AgentCoreRuntimeError.core("Core returned an invalid Memory Worker plan.")
                }
                let output = try await executePlan(
                    conversationID: conversationID,
                    coreConfig: coreConfig,
                    jobID: jobID,
                    ownershipToken: ownershipToken,
                    plan: plan,
                    modelClient: modelClient
                )
                try commit(conversationID: conversationID, coreConfig: coreConfig, submission: .object([
                    "type": .string("memory_worker_completed"),
                    "operation_id": .string("ios-memory-complete-\(UUID().uuidString.lowercased())"),
                    "job_id": .string(jobID),
                    "ownership_token": .string(ownershipToken),
                    "now_epoch_ms": .number(Double(Self.nowMilliseconds())),
                    "completion_watermark": .string(watermark),
                    "output": output,
                ]))
                report.completed += 1
            } catch {
                if error is CancellationError { throw error }
                try commit(conversationID: conversationID, coreConfig: coreConfig, submission: .object([
                    "type": .string("memory_worker_failed"),
                    "operation_id": .string("ios-memory-fail-\(UUID().uuidString.lowercased())"),
                    "job_id": .string(jobID),
                    "ownership_token": .string(ownershipToken),
                    "now_epoch_ms": .number(Double(Self.nowMilliseconds())),
                    "retry_after_ms": .number(Double(Self.retryMilliseconds)),
                    "max_retry_count": .number(Double(Self.maximumRetryCount)),
                    "error": .string(error.localizedDescription),
                ]))
                report.retryScheduled += 1
            }
        }
        return report
    }

    private func executePlan(
        conversationID: String,
        coreConfig: AgentCoreJSONValue,
        jobID: String,
        ownershipToken: String,
        plan: AgentCoreJSONValue,
        modelClient: any AgentModelClient
    ) async throws -> AgentCoreJSONValue {
        try await withThrowingTaskGroup(of: AgentCoreJSONValue?.self) { group in
            group.addTask {
                try await AgentMemoryWorkerModelExecutor.execute(plan: plan, client: modelClient)
            }
            group.addTask { [self] in
                while !Task.isCancelled {
                    try await Task.sleep(nanoseconds: Self.heartbeatNanoseconds)
                    try Task.checkCancellation()
                    let now = Self.nowMilliseconds()
                    try commit(conversationID: conversationID, coreConfig: coreConfig, submission: .object([
                        "type": .string("heartbeat_memory_worker"),
                        "operation_id": .string("ios-memory-heartbeat-\(UUID().uuidString.lowercased())"),
                        "job_id": .string(jobID),
                        "ownership_token": .string(ownershipToken),
                        "now_epoch_ms": .number(Double(now)),
                        "lease_until_epoch_ms": .number(Double(now + Self.leaseMilliseconds)),
                    ]))
                }
                return nil
            }
            guard let first = try await group.next(), let output = first else {
                group.cancelAll()
                throw AgentCoreRuntimeError.core("Memory Worker returned no structured output.")
            }
            group.cancelAll()
            return output
        }
    }

    private func commit(
        conversationID: String,
        coreConfig: AgentCoreJSONValue,
        submission: AgentCoreJSONValue
    ) throws {
        _ = try store.executeAgentCommand(.object([
            "type": .string("reduce_and_commit_idle_submission"),
            "input": .object([
                "conversation_id": .string(conversationID),
                "core_config": coreConfig,
                "submission": submission,
                "committed_at_ms": .number(Double(Self.nowMilliseconds())),
            ]),
        ]))
    }

    private static func nowMilliseconds() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1_000)
    }

    private static let leaseMilliseconds: Int64 = 15 * 60 * 1_000
    private static let heartbeatNanoseconds: UInt64 = 5 * 60 * 1_000_000_000
    private static let retryMilliseconds: Int64 = 60 * 1_000
    private static let maximumRetryCount: Int64 = 3
}
