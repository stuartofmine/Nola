// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the typed macOS Application boundary for automation definitions and runs.

import Foundation

nonisolated struct MacCanonicalAutomation: Codable, Equatable, Sendable {
    let id: String
    let name: String
    let scheduleFormat: String
    let enabled: Bool
    let revision: Int64
    let executionSpec: MacAutomationDefinitionSpec
    let metadataJSON: String
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, name, enabled, revision
        case executionSpec = "execution_spec"
        case scheduleFormat = "schedule_format"
        case metadataJSON = "metadata_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct MacCanonicalAutomationInput: Codable, Equatable, Sendable {
    let id: String
    let name: String
    let scheduleFormat: String
    let enabled: Bool
    let revision: Int64
    let executionSpec: MacAutomationDefinitionSpec
    let metadataJSON: String
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?
}

nonisolated struct MacAutomationExecutionSpec: Codable, Hashable, Sendable {
    struct ContextRef: Codable, Hashable, Sendable {
        let kind: String
        let id: String
    }
    let prompt: String
    let executionScope: MacAutomationExecutionScope
    let permissionProfile: String
    let modelSelection: String
    let contextRefs: [ContextRef]
    let skillRefs: [String]
    enum CodingKeys: String, CodingKey {
        case prompt, executionScope = "execution_scope", permissionProfile = "permission_profile"
        case modelSelection = "model_selection", contextRefs = "context_refs", skillRefs = "skill_refs"
    }
}

nonisolated enum MacAutomationExecutionScope: Codable, Hashable, Sendable {
    case localWorkspace(root: String)
    case deviceLocal(hostID: String)
    enum CodingKeys: String, CodingKey { case kind, root, hostID = "host_id" }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "local_workspace": self = .localWorkspace(root: try values.decode(String.self, forKey: .root))
        case "device_local": self = .deviceLocal(hostID: try values.decode(String.self, forKey: .hostID))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown execution scope")
        }
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .localWorkspace(let root): try values.encode("local_workspace", forKey: .kind); try values.encode(root, forKey: .root)
        case .deviceLocal(let host): try values.encode("device_local", forKey: .kind); try values.encode(host, forKey: .hostID)
        }
    }
}

nonisolated enum MacAutomationDefinitionSpec: Codable, Equatable, Sendable {
    case agent(MacAutomationExecutionSpec)
    case userTask(MacCanonicalJSONValue)
    enum CodingKeys: String, CodingKey { case kind, spec, task }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        switch try fields.decode(String.self, forKey: .kind) {
        case "agent": self = .agent(try fields.decode(MacAutomationExecutionSpec.self, forKey: .spec))
        case "user_task": self = .userTask(try fields.decode(MacCanonicalJSONValue.self, forKey: .task))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: fields, debugDescription: "Unknown automation definition kind")
        }
    }
    func encode(to encoder: Encoder) throws {
        var fields = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .agent(let spec):
            try fields.encode("agent", forKey: .kind)
            try fields.encode(spec, forKey: .spec)
        case .userTask(let task):
            try fields.encode("user_task", forKey: .kind)
            try fields.encode(task, forKey: .task)
        }
    }
}

nonisolated struct MacCanonicalReconcileAutomationsInput: Codable, Equatable, Sendable {
    let inputs: [MacCanonicalAutomationInput]
    let reconciledAtMs: Int64
}

nonisolated struct MacCanonicalAutomationRun: Codable, Equatable, Sendable {
    let id: String
    let automationID: String
    let definitionRevision: Int64
    let occurrenceKey: String
    let trigger: String
    let status: String
    let attempt: Int64
    let workerID: String?
    let leaseToken: String?
    let leaseUntilMs: Int64?
    let scheduledForMs: Int64
    let startedAtMs: Int64?
    let finishedAtMs: Int64?
    let nextRetryAtMs: Int64?
    let resultJSON: String?
    let errorJSON: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id, trigger, status, attempt
        case automationID = "automation_id"
        case definitionRevision = "definition_revision"
        case occurrenceKey = "occurrence_key"
        case workerID = "worker_id"
        case leaseToken = "lease_token"
        case leaseUntilMs = "lease_until_ms"
        case scheduledForMs = "scheduled_for_ms"
        case startedAtMs = "started_at_ms"
        case finishedAtMs = "finished_at_ms"
        case nextRetryAtMs = "next_retry_at_ms"
        case resultJSON = "result_json"
        case errorJSON = "error_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

/// Typed Application use cases required by automation scheduling and presentation.
/// Implementations translate these values to the wire protocol; callers never
/// construct generic command envelopes or access the canonical Store directly.
protocol MacAutomationApplicationClient: Sendable {
    func upsertAutomation(_ input: MacCanonicalAutomationInput) throws -> MacCanonicalAutomation
    func reconcileAutomations(
        _ input: MacCanonicalReconcileAutomationsInput
    ) throws -> [MacCanonicalAutomation]
    func listAutomations() throws -> [MacCanonicalAutomation]
    func enqueueAutomationRun(
        _ input: MacCanonicalAutomationRun
    ) throws -> MacCanonicalAutomationRun
    func claimAutomationRun(
        id: String,
        workerID: String,
        leaseToken: String,
        nowMs: Int64,
        leaseUntilMs: Int64
    ) throws -> Bool
    func heartbeatAutomationRun(
        id: String,
        leaseToken: String,
        nowMs: Int64,
        leaseUntilMs: Int64
    ) throws -> Bool
    func observeAutomationExecution(occurrenceID: String) throws -> MacAutomationExecutionObservation
    func listPendingAutomationExecutions(afterOccurrenceID: String?, limit: Int) throws -> [String]
    func reconcileAutomationExecution(occurrenceID: String, reconciledAtMs: Int64) throws -> MacAutomationExecutionReceipt?
    func observeAutomationWorker(lease: MacAutomationExecutionLease, observation: MacAutomationWorkerObservation, observedAtMs: Int64) throws -> Bool
    func ackAutomationNotification(occurrenceID: String, notificationID: String, deliveredAtMs: Int64) throws -> Bool
    func loadAutomationRun(id: String) throws -> MacCanonicalAutomationRun?
    func listAutomationRuns(
        automationID: String?,
        limit: Int
    ) throws -> [MacCanonicalAutomationRun]
}

nonisolated struct MacAutomationExecutionLease: Codable, Equatable, Sendable {
    let occurrenceID: String
    let definitionRevision: Int64
    let workerID: String
    let leaseToken: String
    enum CodingKeys: String, CodingKey {
        case occurrenceID = "occurrence_id", definitionRevision = "definition_revision"
        case workerID = "worker_id", leaseToken = "lease_token"
    }
}

/// In-process admission input to the shared Host. Never a separate recovery record.
nonisolated struct MacAutomationHostAdmission: Sendable {
    let lease: MacAutomationExecutionLease
    let spec: MacAutomationExecutionSpec
    let artifacts: Data
}

nonisolated struct MacAutomationExecutionIdentity: Codable, Equatable, Sendable {
    let conversationID: String
    let submissionID: String
    let userMessageID: String
    let assistantMessageID: String
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", submissionID = "submission_id"
        case userMessageID = "user_message_id", assistantMessageID = "assistant_message_id"
    }
}

nonisolated struct MacAutomationExecutionBinding: Codable, Equatable, Sendable {
    let occurrenceID: String
    let definitionRevision: Int64
    let conversationID: String
    let submissionID: String
    let runID: String
    let invocationConfigDigest: String
    let boundAtMs: Int64
    enum CodingKeys: String, CodingKey {
        case occurrenceID = "occurrence_id", definitionRevision = "definition_revision"
        case conversationID = "conversation_id", submissionID = "submission_id", runID = "run_id"
        case invocationConfigDigest = "invocation_config_digest", boundAtMs = "bound_at_ms"
    }
}

nonisolated struct MacAutomationExecutionObservation: Codable, Equatable, Sendable {
    let phase: MacAutomationExecutionPhase
    let latestWorkerObservation: MacAutomationWorkerObservation?
    let identity: MacAutomationExecutionIdentity
    let binding: MacAutomationExecutionBinding?
    let runStatus: String?
    let terminal: MacAssistantRunTerminalSeal?
    let receipt: MacAutomationExecutionReceipt?
    let notificationDelivered: Bool
    enum CodingKeys: String, CodingKey {
        case phase, identity, binding, terminal, receipt
        case latestWorkerObservation = "latest_worker_observation"
        case runStatus = "run_status"
        case notificationDelivered = "notification_delivered"
    }
}

nonisolated enum MacAutomationExecutionPhase: String, Codable, Sendable {
    case queued, claimed, preparing, admitted, terminal
    case admissionPending = "admission_pending"
    case launchFailed = "launch_failed"
}

nonisolated struct MacAutomationExecutionReceipt: Codable, Equatable, Sendable {
    let binding: MacAutomationExecutionBinding
    let terminal: MacAssistantRunTerminalSeal
    let reconciledAtMs: Int64
    let notificationID: String
    enum CodingKeys: String, CodingKey {
        case binding, terminal
        case reconciledAtMs = "reconciled_at_ms", notificationID = "notification_id"
    }
}

/// Worker facts never grant a business outcome.
nonisolated enum MacAutomationWorkerObservation: Codable, Equatable, Sendable {
    case preparing
    case admissionRequested
    case exited(exitCode: Int32)
    case disconnected(detail: String)
    case launchFailed(detail: String)
    enum CodingKeys: String, CodingKey { case kind, exitCode = "exit_code", detail }
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "preparing": self = .preparing
        case "admission_requested": self = .admissionRequested
        case "exited": self = .exited(exitCode: try container.decode(Int32.self, forKey: .exitCode))
        case "disconnected": self = .disconnected(detail: try container.decode(String.self, forKey: .detail))
        case "launch_failed": self = .launchFailed(detail: try container.decode(String.self, forKey: .detail))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "Unknown worker observation")
        }
    }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .preparing:
            try container.encode("preparing", forKey: .kind)
        case .admissionRequested:
            try container.encode("admission_requested", forKey: .kind)
        case .exited(let code):
            try container.encode("exited", forKey: .kind)
            try container.encode(code, forKey: .exitCode)
        case .disconnected(let detail):
            try container.encode("disconnected", forKey: .kind)
            try container.encode(detail, forKey: .detail)
        case .launchFailed(let detail):
            try container.encode("launch_failed", forKey: .kind)
            try container.encode(detail, forKey: .detail)
        }
    }
}
