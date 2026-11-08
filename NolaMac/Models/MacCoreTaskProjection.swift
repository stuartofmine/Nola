// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Mirrors Core TaskHandle projections and reduces authoritative task updates for macOS UI.

import Foundation

nonisolated enum MacCoreTaskKind: String, Codable, Hashable, Sendable {
    case agent
    case tool
    case skill
    case dataRefresh = "data_refresh"
}

nonisolated enum MacCoreTaskState: String, Codable, Hashable, Sendable {
    case queued, running, waiting, suspended, completed, sealed, failed, cancelled
    case retryWaiting = "retry_waiting"

    var isActive: Bool {
        self == .queued || self == .running || self == .waiting
            || self == .suspended || self == .retryWaiting
    }
    var isTerminal: Bool { !isActive }
}

nonisolated struct MacCoreTaskDurability: Codable, Hashable, Sendable {
    let runResult: MacCanonicalJSONValue?
    let resultDigest: String?
    let terminalReceiptID: String?
    let workerID: String?
    let ownershipToken: String?
    let leaseUntilEpochMS: Int64?
    let lastHeartbeatEpochMS: Int64?
    let retryCount: UInt32?
    let retryAtEpochMS: Int64?
    let lastError: String?
    let inputWatermark: Int64?
    let completionWatermark: Int64?

    enum CodingKeys: String, CodingKey {
        case runResult = "run_result"
        case resultDigest = "result_digest"
        case terminalReceiptID = "terminal_receipt_id"
        case workerID = "worker_id"
        case ownershipToken = "ownership_token"
        case leaseUntilEpochMS = "lease_until_epoch_ms"
        case lastHeartbeatEpochMS = "last_heartbeat_epoch_ms"
        case retryCount = "retry_count"
        case retryAtEpochMS = "retry_at_epoch_ms"
        case lastError = "last_error"
        case inputWatermark = "input_watermark"
        case completionWatermark = "completion_watermark"
    }
}

nonisolated struct MacCoreTaskLineage: Codable, Hashable, Sendable {
    let parentTaskID: String?
    let childSessionID: String?
    let parentCallID: String?
    let permissionScopeID: String?
    let cancellationScopeID: String?

    enum CodingKeys: String, CodingKey {
        case parentTaskID = "parent_task_id"
        case childSessionID = "child_session_id"
        case parentCallID = "parent_call_id"
        case permissionScopeID = "permission_scope_id"
        case cancellationScopeID = "cancellation_scope_id"
    }
}

nonisolated struct MacCoreTaskHandle: Codable, Hashable, Identifiable, Sendable {
    let id: String
    let ownerSessionID: String
    let ownerTurnID: String
    let kind: MacCoreTaskKind
    let state: MacCoreTaskState
    let progress: Double?
    let currentStep: String?
    let resultRef: String?
    let cancelRequested: Bool
    let lineage: MacCoreTaskLineage?
    let durability: MacCoreTaskDurability?

    enum CodingKeys: String, CodingKey {
        case id, kind, state, progress, lineage, durability
        case ownerSessionID = "owner_session_id"
        case ownerTurnID = "owner_turn_id"
        case currentStep = "current_step"
        case resultRef = "result_ref"
        case cancelRequested = "cancel_requested"
    }

    // A structural transport check, never a replacement for Store receipt verification.
    var hasValidResultShape: Bool {
        if state != .sealed {
            return !(kind == .agent && state == .completed)
                && durability?.runResult == nil && durability?.resultDigest == nil
        }
        guard kind == .agent, let raw = durability?.runResult,
              let bytes = try? JSONEncoder().encode(raw),
              let result = try? JSONDecoder().decode(MacRunResult.self, from: bytes),
              result.value.hasRequiredEvidenceFields,
              result.conversation_id == lineage?.childSessionID,
              let digest = durability?.resultDigest,
              digest.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              let receipt = durability?.terminalReceiptID,
              !receipt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return true
    }
}

nonisolated struct MacCoreTaskProjection: Codable, Hashable, Sendable {
    var revision: UInt64 = 0
    var order: [String] = []
    var tasks: [String: MacCoreTaskHandle] = [:]
    var guidance: [String: MacCoreTaskGuidanceQueue]?

    init(revision: UInt64 = 0, order: [String] = [], tasks: [String: MacCoreTaskHandle] = [:],
         guidance: [String: MacCoreTaskGuidanceQueue]? = nil) {
        self.revision = revision; self.order = order; self.tasks = tasks; self.guidance = guidance
    }

    private enum CodingKeys: String, CodingKey { case revision, order, tasks, guidance }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        revision = try values.decode(UInt64.self, forKey: .revision)
        order = try values.decode([String].self, forKey: .order)
        tasks = try values.decode([String: MacCoreTaskHandle].self, forKey: .tasks)
        guidance = try values.decodeIfPresent([String: MacCoreTaskGuidanceQueue].self, forKey: .guidance)
        guard tasks.values.allSatisfy(\.hasValidResultShape) else {
            throw DecodingError.dataCorruptedError(forKey: .tasks, in: values,
                debugDescription: "Task snapshot lacks its exact sealed result receipt")
        }
    }

    var orderedTasks: [MacCoreTaskHandle] { order.compactMap { tasks[$0] } }
    func guidanceQueue(for taskID: String) -> MacCoreTaskGuidanceQueue? { guidance?[taskID] }
}

nonisolated struct MacCoreTaskGuidanceMessage: Codable, Hashable, Identifiable, Sendable {
    let sequence: UInt64
    let messageID: String
    let parentSessionID: String
    let parentTurnID: String
    let content: String
    let enqueuedAtEpochMS: Int64

    var id: String { messageID }

    enum CodingKeys: String, CodingKey {
        case sequence, content
        case messageID = "message_id"
        case parentSessionID = "parent_session_id"
        case parentTurnID = "parent_turn_id"
        case enqueuedAtEpochMS = "enqueued_at_epoch_ms"
    }
}

nonisolated struct MacCoreTaskGuidanceQueue: Codable, Hashable, Sendable {
    let lastSequence: UInt64
    let acknowledgedSequence: UInt64
    let pending: [MacCoreTaskGuidanceMessage]
    let acknowledgedMessageIDs: [String]

    enum CodingKeys: String, CodingKey {
        case pending
        case lastSequence = "last_sequence"
        case acknowledgedSequence = "acknowledged_sequence"
        case acknowledgedMessageIDs = "acknowledged_message_ids"
    }
}

nonisolated struct MacCoreTaskUpdateEnvelope: Codable, Hashable, Sendable {
    let protocolVersion: UInt32
    let type: String
    let sessionID: String
    let turnID: String
    let cursor: UInt64
    let revision: UInt64
    let taskProjectionRevision: UInt64
    let task: MacCoreTaskHandle
    let guidance: MacCoreTaskGuidanceQueue?

    enum CodingKeys: String, CodingKey {
        case type, cursor, revision, task, guidance
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case taskProjectionRevision = "task_projection_revision"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        protocolVersion = try values.decode(UInt32.self, forKey: .protocolVersion)
        type = try values.decode(String.self, forKey: .type)
        sessionID = try values.decode(String.self, forKey: .sessionID)
        turnID = try values.decode(String.self, forKey: .turnID)
        cursor = try values.decode(UInt64.self, forKey: .cursor)
        revision = try values.decode(UInt64.self, forKey: .revision)
        taskProjectionRevision = try values.decode(UInt64.self, forKey: .taskProjectionRevision)
        task = try values.decode(MacCoreTaskHandle.self, forKey: .task)
        guidance = try values.decodeIfPresent(MacCoreTaskGuidanceQueue.self, forKey: .guidance)

        let progressIsValid = task.progress.map { $0.isFinite && (0 ... 1).contains($0) } ?? true
        guard protocolVersion > 0,
              type == "core_task_updated",
              !sessionID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !turnID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              cursor > 0,
              revision > 0,
              taskProjectionRevision > 0,
              !task.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              task.ownerSessionID == sessionID,
              task.ownerTurnID == turnID,
              progressIsValid, task.hasValidResultShape else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: values.codingPath,
                debugDescription: "Task update identity does not match its Core session/turn envelope."
            ))
        }

        if let guidance {
            guard guidance.acknowledgedSequence <= guidance.lastSequence,
                  guidance.pending.allSatisfy({
                      $0.parentSessionID == sessionID
                          && $0.parentTurnID == turnID
                          && $0.sequence > guidance.acknowledgedSequence
                          && $0.sequence <= guidance.lastSequence
                  }) else {
                throw DecodingError.dataCorrupted(.init(
                    codingPath: values.codingPath,
                    debugDescription: "Task guidance does not match its Core session/turn envelope."
                ))
            }
        }
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(protocolVersion, forKey: .protocolVersion)
        try values.encode(type, forKey: .type)
        try values.encode(sessionID, forKey: .sessionID)
        try values.encode(turnID, forKey: .turnID)
        try values.encode(cursor, forKey: .cursor)
        try values.encode(revision, forKey: .revision)
        try values.encode(taskProjectionRevision, forKey: .taskProjectionRevision)
        try values.encode(task, forKey: .task)
        try values.encodeIfPresent(guidance, forKey: .guidance)
    }
}

nonisolated struct MacCoreTaskUIModel: Hashable, Sendable {
    let task: MacCoreTaskHandle
    var progress: Double? { task.progress.map { min(1, max(0, $0)) } }
    var isActive: Bool { task.state.isActive }
    var canCancel: Bool { task.state.isActive && !task.cancelRequested }
    var canResume: Bool { task.state == .waiting || task.state == .suspended }
    var terminalResult: MacRunResult? {
        guard task.state == .sealed, task.hasValidResultShape,
              let raw = task.durability?.runResult,
              let bytes = try? JSONEncoder().encode(raw) else { return nil }
        return try? JSONDecoder().decode(MacRunResult.self, from: bytes)
    }
    var outcomeLabel: String? {
        guard let result = terminalResult else { return nil }
        switch result.value.kind {
        case .answered: return String(localized: "Answered")
        case .actionCompleted: return String(localized: "Completed")
        case .delegationAccepted: return String(localized: "Delegated")
        case .incomplete: return String(localized: "Incomplete")
        case .blocked: return String(localized: "Blocked")
        case .cancelled: return String(localized: "Cancelled")
        case .failed: return String(localized: "Failed")
        }
    }
    var outcomeDetails: [String] {
        guard let result = terminalResult, case .object(let fields) = result.value.payload else { return [] }
        var details: [String] = []
        if case .object(let reason)? = fields["reason"] {
            if case .string(let detail)? = reason["detail"], !detail.isEmpty { details.append(detail) }
            else if case .string(let code)? = reason["code"] { details.append(code) }
        }
        if case .string(let condition)? = fields["resolution_condition"], !condition.isEmpty { details.append(condition) }
        for key in ["remaining_requirement_refs", "obligation_refs", "unknown_effect_refs"] {
            if case .array(let refs)? = fields[key] {
                details.append(contentsOf: refs.compactMap { if case .string(let text) = $0 { return text }; return nil })
            }
        }
        if result.value.kind == .failed {
            if let error = task.durability?.lastError, !error.isEmpty { details.append(error) }
            if case .string(let errorRef)? = fields["error_ref"] { details.append(errorRef) }
        }
        return details
    }
    var detail: String {
        if let first = outcomeDetails.first { return first }
        let step = task.currentStep?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !step.isEmpty { return step }
        return task.durability?.lastError?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }
}
