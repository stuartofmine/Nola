// SPDX-License-Identifier: Apache-2.0
// Exact scheduled occurrence transport; process observation is not task completion.
import Foundation

nonisolated struct IOSAutomationExecutionCommand: Encodable {
    let type: String
    var occurrenceID: String? = nil
    var afterOccurrenceID: String? = nil
    var limit: Int? = nil
    var reconciledAtMs: Int64? = nil
    var notificationID: String? = nil
    var deliveredAtMs: Int64? = nil
    var lease: IOSAutomationExecutionLease? = nil
    var observation: IOSAutomationWorkerObservation? = nil
    var observedAtMs: Int64? = nil
    enum CodingKeys: String, CodingKey {
        case type, limit, lease, observation
        case occurrenceID = "occurrence_id", afterOccurrenceID = "after_occurrence_id"
        case reconciledAtMs = "reconciled_at_ms", notificationID = "notification_id"
        case deliveredAtMs = "delivered_at_ms", observedAtMs = "observed_at_ms"
    }
}

nonisolated struct IOSUserTaskSpec: Codable, Equatable, Sendable {
    struct Recurrence: Codable, Equatable, Sendable {
        let frequency: String
        let interval: UInt32
        let daysOfWeek: [UInt32]?
        let daysOfMonth: [UInt32]?
        let monthsOfYear: [UInt32]?
        let endsAtMs: Int64?
        enum CodingKeys: String, CodingKey {
            case frequency, interval, daysOfWeek = "days_of_week", daysOfMonth = "days_of_month"
            case monthsOfYear = "months_of_year", endsAtMs = "ends_at_ms"
        }
    }
    let taskType: String
    let description: String?
    let targetAtMs: Int64?
    let recurrence: Recurrence?
    let relatedPerson: String?
    let amountDecimal: String?
    let currency: String?
    let dueAtMs: Int64?
    let completed: Bool
    let notificationIDs: [String]
    enum CodingKeys: String, CodingKey {
        case taskType = "task_type", description, targetAtMs = "target_at_ms", recurrence
        case relatedPerson = "related_person", amountDecimal = "amount_decimal", currency
        case dueAtMs = "due_at_ms", completed, notificationIDs = "notification_ids"
    }

    init(task: UserTask) throws {
        func time(_ value: Date?) -> Int64? { value.map { Int64($0.timeIntervalSince1970 * 1_000) } }
        func values(_ list: [Int]?) throws -> [UInt32]? {
            try list.map { try $0.map { value in
                guard let value = UInt32(exactly: value) else { throw CocoaError(.coderInvalidValue) }; return value
            } }
        }
        taskType = task.type.rawValue
        description = task.description
        targetAtMs = time(task.targetDate)
        if task.isRecurring {
            guard let rule = task.recurrenceRule, let interval = UInt32(exactly: rule.interval), interval > 0 else { throw CocoaError(.coderInvalidValue) }
            recurrence = try .init(frequency: rule.frequency.rawValue, interval: interval, daysOfWeek: values(rule.daysOfWeek), daysOfMonth: values(rule.daysOfMonth), monthsOfYear: values(rule.monthsOfYear), endsAtMs: time(rule.endDate))
        } else { recurrence = nil }
        relatedPerson = task.relatedPerson
        amountDecimal = task.amount.map { NSDecimalNumber(decimal: $0).stringValue }
        currency = task.currency
        dueAtMs = time(task.dueDate)
        completed = task.isCompleted
        notificationIDs = task.notificationIds
    }

    func task(definition: IOSCanonicalAutomationDefinition) throws -> UserTask {
        func date(_ value: Int64?) -> Date? { value.map { Date(timeIntervalSince1970: Double($0) / 1_000) } }
        guard let id = UUID(uuidString: definition.id), let type = TaskType(rawValue: taskType) else { throw CocoaError(.coderInvalidValue) }
        let rule: RecurrenceRule?
        if let recurrence {
            guard let frequency = RecurrenceFrequency(rawValue: recurrence.frequency), recurrence.interval > 0 else { throw CocoaError(.coderInvalidValue) }
            rule = .init(frequency: frequency, interval: Int(recurrence.interval), daysOfWeek: recurrence.daysOfWeek?.map(Int.init), daysOfMonth: recurrence.daysOfMonth?.map(Int.init), monthsOfYear: recurrence.monthsOfYear?.map(Int.init), endDate: date(recurrence.endsAtMs))
        } else { rule = nil }
        let amount = amountDecimal.flatMap { Decimal(string: $0, locale: Locale(identifier: "en_US_POSIX")) }
        guard amountDecimal == nil || amount != nil else { throw CocoaError(.coderInvalidValue) }
        return UserTask(id: id, type: type, title: definition.name, description: description, targetDate: date(targetAtMs), isRecurring: rule != nil, recurrenceRule: rule, relatedPerson: relatedPerson, amount: amount, currency: currency, dueDate: date(dueAtMs), isCompleted: completed, createdAt: date(definition.createdAtMs)!, updatedAt: date(definition.updatedAtMs)!, notificationIds: notificationIDs)
    }
}

nonisolated struct IOSAutomationExecutionSpec: Codable, Equatable, Sendable {
    struct ContextRef: Codable, Equatable, Sendable {
        let kind: String
        let id: String
    }
    let prompt: String
    let executionScope: IOSAutomationExecutionScope
    let permissionProfile: String
    let modelSelection: String
    let contextRefs: [ContextRef]
    let skillRefs: [String]
    enum CodingKeys: String, CodingKey {
        case prompt, executionScope = "execution_scope", permissionProfile = "permission_profile"
        case modelSelection = "model_selection", contextRefs = "context_refs", skillRefs = "skill_refs"
    }
}


nonisolated enum IOSAutomationExecutionScope: Codable, Equatable, Sendable {
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


nonisolated enum IOSAutomationDefinitionSpec: Codable, Equatable, Sendable {
    case agent(IOSAutomationExecutionSpec)
    case userTask(IOSUserTaskSpec)
    enum CodingKeys: String, CodingKey { case kind, spec, task }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        switch try fields.decode(String.self, forKey: .kind) {
        case "agent": self = .agent(try fields.decode(IOSAutomationExecutionSpec.self, forKey: .spec))
        case "user_task": self = .userTask(try fields.decode(IOSUserTaskSpec.self, forKey: .task))
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


nonisolated struct IOSAutomationExecutionLease: Codable, Equatable, Sendable {
    let occurrenceID: String
    let definitionRevision: Int64
    let workerID: String
    let leaseToken: String
    enum CodingKeys: String, CodingKey {
        case occurrenceID = "occurrence_id", definitionRevision = "definition_revision"
        case workerID = "worker_id", leaseToken = "lease_token"
    }
}


nonisolated struct IOSAutomationExecutionIdentity: Codable, Equatable, Sendable {
    let conversationID: String
    let submissionID: String
    let userMessageID: String
    let assistantMessageID: String
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", submissionID = "submission_id"
        case userMessageID = "user_message_id", assistantMessageID = "assistant_message_id"
    }
}


nonisolated struct IOSAutomationExecutionBinding: Codable, Equatable, Sendable {
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


nonisolated struct IOSAutomationExecutionObservation: Codable, Equatable, Sendable {
    let identity: IOSAutomationExecutionIdentity
    let binding: IOSAutomationExecutionBinding?
    let runStatus: String?
    let terminal: IOSAssistantRunTerminalSeal?
    let receipt: IOSAutomationExecutionReceipt?
    let notificationDelivered: Bool
    enum CodingKeys: String, CodingKey {
        case identity, binding, terminal, receipt
        case runStatus = "run_status"
        case notificationDelivered = "notification_delivered"
    }
}


nonisolated struct IOSAutomationExecutionReceipt: Codable, Equatable, Sendable {
    let binding: IOSAutomationExecutionBinding
    let terminal: IOSAssistantRunTerminalSeal
    let reconciledAtMs: Int64
    let notificationID: String
    enum CodingKeys: String, CodingKey {
        case binding, terminal
        case reconciledAtMs = "reconciled_at_ms", notificationID = "notification_id"
    }
}

/// Worker facts never grant a business outcome.

nonisolated enum IOSAutomationWorkerObservation: Encodable, Sendable {
    case exited(exitCode: Int32)
    case disconnected(detail: String)
    case launchFailed(detail: String)
    enum CodingKeys: String, CodingKey { case kind, exitCode = "exit_code", detail }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
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
