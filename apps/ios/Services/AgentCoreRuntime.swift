// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Shared Swift value contracts and stateless context transforms.
// - Interfaces: Typed command/event models used by the iOS chat host.

import Foundation

#if canImport(NolaCore)
import NolaCore
#endif

nonisolated public enum AgentCorePlatform: String, Codable, Sendable {
    case cli
    case ios
    case android
    case other
}

nonisolated public enum AgentCoreExecutionMode: String, Codable, Sendable {
    case deviceLocal = "device_local"
    case hostRemote = "host_remote"
    case hybrid
}

nonisolated public enum AgentCoreInputOrigin: String, Codable, Sendable {
    case unknown
    case interactiveUser = "interactive_user"
    case goalInteractiveUser = "goal_interactive_user"
    case goalContinuation = "goal_continuation"
    case scheduledAction = "scheduled_action"
    case replay
    case systemRecovery = "system_recovery"
    case delegatedAgent = "delegated_agent"

    public var isAutomated: Bool {
        switch self {
        case .scheduledAction, .replay, .systemRecovery, .delegatedAgent, .goalContinuation:
            return true
        case .unknown, .interactiveUser, .goalInteractiveUser:
            return false
        }
    }
}

nonisolated public enum AgentCoreInputSubmissionError: LocalizedError, Equatable {
    case unsupportedOrigin(AgentCoreInputOrigin)

    public var errorDescription: String? {
        switch self {
        case .unsupportedOrigin(let origin):
            return "Agent input origin cannot start this turn: \(origin.rawValue)"
        }
    }
}

nonisolated public enum AgentCoreInputSubmission {
    public static func make(
        input: String,
        origin: AgentCoreInputOrigin = .interactiveUser,
        contextComponents: [AgentCoreContextComponent] = [],
        messageId: String = "ios-user:\(UUID().uuidString.lowercased())",
        submissionId: String = UUID().uuidString.lowercased(),
        submittedAtEpochMilliseconds: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) throws -> AgentCoreJSONValue {
        let items: AgentCoreJSONValue = .array([
            .object([
                "type": .string("message"),
                "id": .string(messageId),
                "role": .string("user"),
                "content": .string(input),
            ]),
        ])
        let encodedComponents = try contextComponents.map { component -> AgentCoreJSONValue in
            let data = try JSONEncoder().encode(component)
            return try JSONDecoder().decode(AgentCoreJSONValue.self, from: data)
        }
        func submission(_ values: [String: AgentCoreJSONValue]) -> AgentCoreJSONValue {
            var values = values
            // Current admission contracts require an explicit platform and
            // context collections, including the empty-context case. These are
            // turn context, never replacements for a queued UiAction payload.
            values["turn_context_items"] = .array([])
            values["turn_context_platform"] = .string("ios")
            values["turn_context_components"] = .array(encodedComponents)
            return .object(values)
        }
        switch origin {
        case .interactiveUser, .goalInteractiveUser:
            return submission([
                "type": .string(origin == .goalInteractiveUser ? "goal_user_input" : "user_input"),
                "items": items,
                "submission_id": .string(submissionId),
                "submitted_at_epoch_ms": .number(Double(submittedAtEpochMilliseconds)),
                "delivery": .string("current_turn"),
            ])
        case .scheduledAction, .replay, .systemRecovery, .delegatedAgent:
            return submission([
                "type": .string("automated_input"),
                "origin": .string(origin.rawValue),
                "items": items,
                "submission_id": .string(submissionId),
                "submitted_at_epoch_ms": .number(Double(submittedAtEpochMilliseconds)),
            ])
        case .unknown, .goalContinuation:
            throw AgentCoreInputSubmissionError.unsupportedOrigin(origin)
        }
    }
}

nonisolated public enum AgentCoreCapability: String, Codable, Sendable {
    case none
    case filesRead = "files_read"
    case filesWrite = "files_write"
    case process
    case network
    case background
    case camera
    case microphone
    case photos
    case location
    case calendar
    case reminders
    case contacts
    case health
    case notifications
    case clipboard
    case computer
}

nonisolated public enum AgentCorePermissionDecision: String, Codable, Sendable {
    case allowOnce = "allow_once"
    case allowConversation = "allow_conversation"
    case deny
}

nonisolated public enum IOSPermissionMode: String, CaseIterable, Codable, Identifiable, Sendable {
    case cautious
    case allowAll = "allow_all"

    public var id: String { rawValue }
    public var autoApprove: Bool { true }
    public var networkPolicy: String { self == .allowAll ? "allow" : "ask" }


    public var title: String {
        switch self {
        case .cautious: return NSLocalizedString("permission.mode.cautious", value: "Cautious", comment: "Cautious permission mode")
        case .allowAll: return NSLocalizedString("permission.mode.allow_all", value: "Allow All", comment: "Allow all permission mode")
        }
    }

    public var detail: String {
        switch self {
        case .cautious: return NSLocalizedString("permission.mode.cautious.detail", value: "Confirm deletion, overwrites, sensitive data access, outgoing content, and new connections. Ordinary reads and new files proceed.", comment: "Cautious permission mode detail")
        case .allowAll: return NSLocalizedString("permission.mode.allow_all.detail", value: "No Nola approval prompts in this conversation. System permissions still apply.", comment: "Allow all permission mode detail")
        }
    }
}

nonisolated public enum AgentCoreAuthorization: String, Codable, Sendable {
    case notRequired = "not_required"
    case agentApproval = "agent_approval"
    case scopedResource = "scoped_resource"
    case systemPermission = "system_permission"
    case platformCapability = "platform_capability"
    case platformLifecycle = "platform_lifecycle"
    case unsupported
}


nonisolated public struct AgentCorePermissionSpec: Codable, Sendable, Equatable {
    public var capability: AgentCoreCapability
    public var rationale: String
    public var scope: String?
    public var requiresUserConsent: Bool

    public init(
        capability: AgentCoreCapability,
        rationale: String = "",
        scope: String? = nil,
        requiresUserConsent: Bool = true
    ) {
        self.capability = capability
        self.rationale = rationale
        self.scope = scope
        self.requiresUserConsent = requiresUserConsent
    }

    enum CodingKeys: String, CodingKey {
        case capability, rationale, scope
        case requiresUserConsent = "requires_user_consent"
    }
}

nonisolated public struct AgentCoreToolCall: Codable, Sendable, Equatable {
    public var id: String
    public var name: String
    public var arguments: AgentCoreJSONValue
    public var permission: AgentCorePermissionSpec?

    public init(
        id: String = "",
        name: String,
        arguments: AgentCoreJSONValue = .object([:]),
        permission: AgentCorePermissionSpec? = nil
    ) {
        self.id = id
        self.name = name
        self.arguments = arguments
        self.permission = permission
    }
}

nonisolated public struct AgentCoreMessage: Codable, Sendable, Equatable {
    public var role: String
    public var content: String
    public var name: String?
    public var toolCallId: String?
    public var toolCalls: [AgentCoreToolCall]

    public init(
        role: String,
        content: String,
        name: String? = nil,
        toolCallId: String? = nil,
        toolCalls: [AgentCoreToolCall] = []
    ) {
        self.role = role
        self.content = content
        self.name = name
        self.toolCallId = toolCallId
        self.toolCalls = toolCalls
    }

    enum CodingKeys: String, CodingKey {
        case role, content, name
        case toolCallId = "tool_call_id"
        case toolCalls = "tool_calls"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        role = try container.decode(String.self, forKey: .role)
        content = try container.decode(String.self, forKey: .content)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        toolCallId = try container.decodeIfPresent(String.self, forKey: .toolCallId)
        toolCalls = try container.decodeIfPresent([AgentCoreToolCall].self, forKey: .toolCalls) ?? []
    }
}

nonisolated public struct AgentCoreTokenUsage: Codable, Sendable, Equatable {
    public var promptTokens: Int?
    public var cachedInputTokens: Int?
    public var cacheWriteInputTokens: Int?
    public var completionTokens: Int?
    public var totalTokens: Int?
    public var reasoningTokens: Int?

    public init(
        promptTokens: Int? = nil,
        cachedInputTokens: Int? = nil,
        cacheWriteInputTokens: Int? = nil,
        completionTokens: Int? = nil,
        totalTokens: Int? = nil,
        reasoningTokens: Int? = nil
    ) {
        self.promptTokens = promptTokens
        self.cachedInputTokens = cachedInputTokens
        self.cacheWriteInputTokens = cacheWriteInputTokens
        self.completionTokens = completionTokens
        self.totalTokens = totalTokens
        self.reasoningTokens = reasoningTokens
    }

    enum CodingKeys: String, CodingKey {
        case promptTokens = "prompt_tokens"
        case cachedInputTokens = "cached_input_tokens"
        case cacheWriteInputTokens = "cache_write_input_tokens"
        case completionTokens = "completion_tokens"
        case totalTokens = "total_tokens"
        case reasoningTokens = "reasoning_tokens"
    }
}

nonisolated public struct AgentCorePromptCacheDiagnostic: Codable, Sendable, Equatable {
    var cacheUsage: IOSConversationProjectionV1.CacheUsage? = nil
    public let schemaVersion: UInt32
    public let status: String
    public let identityStable: Bool
    public let identitySHA256: String?
    public let reasons: [String]
    public let fragmentChanges: [AgentCorePromptCacheFragmentChange]?
    public let cacheReadInputTokens: Int?
    public let cacheWriteInputTokens: Int?

    enum CodingKeys: String, CodingKey {
        case cacheUsage
        case schemaVersion, status, identityStable, reasons, fragmentChanges
        case identitySHA256 = "identitySha256"
        case cacheReadInputTokens, cacheWriteInputTokens
    }
}

nonisolated public struct AgentCorePromptCacheFragmentChange: Codable, Sendable, Equatable {
    public let fragmentId: String
    public let kind: String
}

nonisolated public struct AgentCoreRuntimeConfig: Codable, Sendable {
    public var platform: AgentCorePlatform
    public var executionMode: AgentCoreExecutionMode
    public var hostId: String?
    public var model: String?
    public var stablePromptDigest: String?
    public var promptFragmentIds: [String]
    public var stablePromptFragmentDigests: [String: String]?
    public var maxIterations: Int
    public var contextWindowTokens: Int?
    public var reservedOutputTokens: Int
    public var grantedCapabilities: [AgentCoreCapability]
    public var permissionProfile: IOSPermissionMode

    public init(
        platform: AgentCorePlatform = .ios,
        executionMode: AgentCoreExecutionMode = .deviceLocal,
        hostId: String? = nil,
        model: String? = nil,
        stablePromptDigest: String? = nil,
        promptFragmentIds: [String] = [],
        stablePromptFragmentDigests: [String: String]? = nil,
        maxIterations: Int = 0,
        contextWindowTokens: Int? = nil,
        reservedOutputTokens: Int = 4_096,
        grantedCapabilities: [AgentCoreCapability] = [.network],
        permissionProfile: IOSPermissionMode = .cautious
    ) {
        self.platform = platform
        self.executionMode = executionMode
        self.hostId = hostId
        self.model = model
        self.stablePromptDigest = stablePromptDigest
        self.promptFragmentIds = promptFragmentIds
        self.stablePromptFragmentDigests = stablePromptFragmentDigests
        self.maxIterations = maxIterations
        self.contextWindowTokens = contextWindowTokens
        self.reservedOutputTokens = reservedOutputTokens
        self.grantedCapabilities = grantedCapabilities
        self.permissionProfile = permissionProfile
    }

    enum CodingKeys: String, CodingKey {
        case platform
        case executionMode = "execution_mode"
        case hostId = "host_id"
        case model
        case stablePromptDigest = "stable_prompt_digest"
        case promptFragmentIds = "prompt_fragment_ids"
        case stablePromptFragmentDigests = "stable_prompt_fragment_digests"
        case maxIterations = "max_iterations"
        case contextWindowTokens = "context_window_tokens"
        case reservedOutputTokens = "reserved_output_tokens"
        case grantedCapabilities = "granted_capabilities"
        case permissionProfile = "permission_profile"
    }
}

nonisolated public struct AgentCoreEvent: Codable, Sendable {
    public let type: String
    public let phase: String?
    public let sessionId: String?
    public let turnId: String?
    public let iteration: Int?
    public let messages: [AgentCoreMessage]?
    public let tools: [AgentCoreJSONValue]?
    public let estimatedContextTokens: Int?
    public let contextWindowTokens: Int?
    public let reservedOutputTokens: Int?
    public let requestId: String?
    public let callId: String?
    public let toolName: String?
    public let capability: AgentCoreCapability?
    public let authorization: String?
    public let rationale: String?
    public let scope: String?
    public let arguments: AgentCoreJSONValue?
    public let grant: AgentCoreJSONValue?
    public let content: String?
    public let thinking: String?
    public let reason: String?
    public let code: String?
    public let message: String?
    public let isError: Bool?
    public let decision: AgentCorePermissionDecision?

    enum CodingKeys: String, CodingKey {
        case type, phase, iteration, messages, tools, capability, authorization, rationale, scope, decision
        case arguments, grant, content, thinking, reason, code, message
        case sessionId = "session_id"
        case turnId = "turn_id"
        case estimatedContextTokens = "estimated_context_tokens"
        case contextWindowTokens = "context_window_tokens"
        case reservedOutputTokens = "reserved_output_tokens"
        case requestId = "request_id"
        case callId = "call_id"
        case toolName = "tool_name"
        case isError = "is_error"
    }
}

nonisolated public enum AgentCoreTaskKind: String, Codable, Sendable {
    case agent
    case tool
    case skill
    case dataRefresh = "data_refresh"
}

nonisolated public enum AgentCoreTaskState: String, Codable, Sendable {
    case queued, running, waiting, suspended, completed, sealed, failed, cancelled
    case retryWaiting = "retry_waiting"

    public var isTerminal: Bool {
        self == .completed || self == .sealed || self == .failed || self == .cancelled
    }
}

nonisolated public struct AgentCoreTaskDurability: Codable, Equatable, Sendable {
    public let runResult: AgentCoreJSONValue?
    public let resultDigest: String?
    public let terminalReceiptID: String?
    public let workerId: String?
    public let ownershipToken: String?
    public let leaseUntilEpochMs: Int64?
    public let lastHeartbeatEpochMs: Int64?
    public let retryCount: UInt32?
    public let retryAtEpochMs: Int64?
    public let lastError: String?
    public let inputWatermark: Int64?
    public let completionWatermark: Int64?

    enum CodingKeys: String, CodingKey {
        case runResult = "run_result"
        case resultDigest = "result_digest"
        case terminalReceiptID = "terminal_receipt_id"
        case workerId = "worker_id"
        case ownershipToken = "ownership_token"
        case leaseUntilEpochMs = "lease_until_epoch_ms"
        case lastHeartbeatEpochMs = "last_heartbeat_epoch_ms"
        case retryCount = "retry_count"
        case retryAtEpochMs = "retry_at_epoch_ms"
        case lastError = "last_error"
        case inputWatermark = "input_watermark"
        case completionWatermark = "completion_watermark"
    }
}

nonisolated public struct AgentCoreTaskLineage: Codable, Equatable, Sendable {
    public let parentTaskID: String?
    public let childSessionID: String?
    public let parentCallID: String?
    public let permissionScopeID: String?
    public let cancellationScopeID: String?

    enum CodingKeys: String, CodingKey {
        case parentTaskID = "parent_task_id"
        case childSessionID = "child_session_id"
        case parentCallID = "parent_call_id"
        case permissionScopeID = "permission_scope_id"
        case cancellationScopeID = "cancellation_scope_id"
    }
}

nonisolated public struct AgentCoreTaskHandle: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let ownerSessionId: String
    public let ownerTurnId: String
    public let kind: AgentCoreTaskKind
    public let state: AgentCoreTaskState
    public let progress: Double?
    public let currentStep: String?
    public let resultRef: String?
    public let cancelRequested: Bool
    public let lineage: AgentCoreTaskLineage?
    public let durability: AgentCoreTaskDurability?

    enum CodingKeys: String, CodingKey {
        case id, kind, state, progress, lineage, durability
        case ownerSessionId = "owner_session_id"
        case ownerTurnId = "owner_turn_id"
        case currentStep = "current_step"
        case resultRef = "result_ref"
        case cancelRequested = "cancel_requested"
    }

    var isValid: Bool {
        !id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !ownerSessionId.isEmpty
            && !ownerTurnId.isEmpty
            && progress.map { $0.isFinite && (0 ... 1).contains($0) } != false
            && currentStep.map { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } != false
            && resultRef.map { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } != false
            && hasValidResultShape
    }

    // Source and digest authenticity are verified by Core/Store, not this view projection.
    private var hasValidResultShape: Bool {
        if state != .sealed {
            return !(kind == .agent && state == .completed)
                && durability?.runResult == nil && durability?.resultDigest == nil
        }
        guard kind == .agent, let raw = durability?.runResult,
              let bytes = try? JSONEncoder().encode(raw),
              let result = try? JSONDecoder().decode(IOSRunResult.self, from: bytes),
              result.value.hasRequiredEvidenceFields,
              result.conversation_id == lineage?.childSessionID,
              let digest = durability?.resultDigest,
              digest.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              let receipt = durability?.terminalReceiptID,
              !receipt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return true
    }
}

nonisolated public struct AgentCoreTaskProjection: Codable, Equatable, Sendable {
    public var revision: UInt64
    public var order: [String]
    public var tasks: [String: AgentCoreTaskHandle]
    public var guidance: [String: AgentCoreTaskGuidanceQueue]?

    public init(
        revision: UInt64 = 0,
        order: [String] = [],
        tasks: [String: AgentCoreTaskHandle] = [:],
        guidance: [String: AgentCoreTaskGuidanceQueue]? = nil
    ) {
        self.revision = revision
        self.order = order
        self.tasks = tasks
        self.guidance = guidance
    }

    public var orderedTasks: [AgentCoreTaskHandle] { order.compactMap { tasks[$0] } }
    public func guidanceQueue(for taskID: String) -> AgentCoreTaskGuidanceQueue? {
        guidance?[taskID]
    }
}

nonisolated public struct AgentCoreTaskGuidanceMessage: Codable, Equatable, Identifiable, Sendable {
    public let sequence: UInt64
    public let messageID: String
    public let parentSessionID: String
    public let parentTurnID: String
    public let content: String
    public let enqueuedAtEpochMS: Int64

    public var id: String { messageID }

    enum CodingKeys: String, CodingKey {
        case sequence, content
        case messageID = "message_id"
        case parentSessionID = "parent_session_id"
        case parentTurnID = "parent_turn_id"
        case enqueuedAtEpochMS = "enqueued_at_epoch_ms"
    }
}

nonisolated public struct AgentCoreTaskGuidanceQueue: Codable, Equatable, Sendable {
    public let lastSequence: UInt64
    public let acknowledgedSequence: UInt64
    public let pending: [AgentCoreTaskGuidanceMessage]
    public let acknowledgedMessageIDs: [String]

    enum CodingKeys: String, CodingKey {
        case pending
        case lastSequence = "last_sequence"
        case acknowledgedSequence = "acknowledged_sequence"
        case acknowledgedMessageIDs = "acknowledged_message_ids"
    }
}

nonisolated public struct AgentCoreTaskEventEnvelope: Equatable, Sendable {
    public let sessionId: String
    public let turnId: String
    public let cursor: UInt64
    public let revision: UInt64
    public let taskProjectionRevision: UInt64
    public let task: AgentCoreTaskHandle
    public let guidance: AgentCoreTaskGuidanceQueue?

    public static func decodeCore(from value: AgentCoreJSONValue) -> Self? {
        guard case .object(let envelope) = value,
              case .string(let sessionId)? = envelope["session_id"],
              case .string(let turnId)? = envelope["turn_id"],
              case .object(let cursorObject)? = envelope["cursor"],
              let cursor = unsigned(cursorObject["sequence"]),
              let revision = unsigned(envelope["revision"]),
              case .object(let event)? = envelope["event"],
              case .string("task_updated")? = event["type"] else { return nil }
        return decodePayload(event, sessionId: sessionId, turnId: turnId, cursor: cursor, revision: revision)
    }

    public static func decodeHost(from value: AgentCoreJSONValue) -> Self? {
        guard case .object(let envelope) = value,
              envelope["event"] == nil,
              unsigned(envelope["protocol_version"]) == 1,
              case .string("core_task_updated")? = envelope["type"],
              case .string(let sessionId)? = envelope["session_id"],
              case .string(let turnId)? = envelope["turn_id"],
              let cursor = unsigned(envelope["cursor"]),
              let revision = unsigned(envelope["revision"]) else { return nil }
        return decodePayload(envelope, sessionId: sessionId, turnId: turnId, cursor: cursor, revision: revision)
    }

    private static func decodePayload(
        _ event: [String: AgentCoreJSONValue],
        sessionId: String,
        turnId: String,
        cursor: UInt64,
        revision: UInt64
    ) -> Self? {
        guard let projectionRevision = unsigned(event["task_projection_revision"]),
              let taskValue = event["task"],
              let taskData = try? JSONEncoder().encode(taskValue),
              let task = try? JSONDecoder().decode(AgentCoreTaskHandle.self, from: taskData),
              task.isValid else { return nil }
        let guidance: AgentCoreTaskGuidanceQueue? = event["guidance"].flatMap { value in
            guard let data = try? JSONEncoder().encode(value) else { return nil }
            return try? JSONDecoder().decode(AgentCoreTaskGuidanceQueue.self, from: data)
        }
        return .init(
            sessionId: sessionId,
            turnId: turnId,
            cursor: cursor,
            revision: revision,
            taskProjectionRevision: projectionRevision,
            task: task,
            guidance: guidance
        )
    }

    private static func unsigned(_ value: AgentCoreJSONValue?) -> UInt64? {
        switch value {
        case .unsignedInteger(let number):
            return number
        case .signedInteger(let number) where number >= 0:
            return UInt64(number)
        case .number(let number) where number.isFinite
            && number >= 0
            && number.rounded(.towardZero) == number:
            return UInt64(number)
        default:
            return nil
        }
    }
}

nonisolated public enum AgentCoreTaskProjectionApplyResult: Equatable, Sendable {
    case applied, stale, identityMismatch, invalidOwner, invalidTransition
}

nonisolated public struct AgentCoreTaskProjectionReducer: Sendable {
    public private(set) var projection = AgentCoreTaskProjection()

    public init() {}

    @discardableResult
    public mutating func restore(
        _ restored: AgentCoreTaskProjection,
        expectedSessionId: String,
        expectedTurnId: String? = nil
    ) -> AgentCoreTaskProjectionApplyResult {
        guard restored.tasks.values.allSatisfy({ task in
            task.isValid && task.ownerSessionId == expectedSessionId
                && (expectedTurnId == nil || task.ownerTurnId == expectedTurnId)
                && restored.order.contains(task.id)
        }), Set(restored.order).count == restored.order.count,
        Set(restored.order) == Set(restored.tasks.keys) else { return .invalidOwner }
        guard restored.revision >= projection.revision else { return .stale }
        projection = restored
        return .applied
    }

    @discardableResult
    public mutating func apply(
        _ envelope: AgentCoreTaskEventEnvelope,
        expectedSessionId: String,
        expectedTurnId: String? = nil
    ) -> AgentCoreTaskProjectionApplyResult {
        guard envelope.sessionId == expectedSessionId,
              expectedTurnId == nil || envelope.turnId == expectedTurnId else {
            return .identityMismatch
        }
        let task = envelope.task
        guard task.isValid, task.ownerSessionId == envelope.sessionId,
              task.ownerTurnId == envelope.turnId else { return .invalidOwner }
        guard envelope.taskProjectionRevision > projection.revision else { return .stale }
        if let current = projection.tasks[task.id] {
            guard current.ownerSessionId == task.ownerSessionId,
                  current.ownerTurnId == task.ownerTurnId,
                  current.kind == task.kind,
                  !current.state.isTerminal,
                  Self.canTransition(from: current.state, to: task.state),
                  current.progress.map({ old in task.progress.map { $0 >= old } ?? false }) != false,
                  !current.cancelRequested || task.cancelRequested else {
                return .invalidTransition
            }
        } else if task.state.isTerminal {
            return .invalidTransition
        }
        projection.revision = envelope.taskProjectionRevision
        if projection.tasks[task.id] == nil { projection.order.append(task.id) }
        projection.tasks[task.id] = task
        if let guidance = envelope.guidance {
            var queues = projection.guidance ?? [:]
            queues[task.id] = guidance
            projection.guidance = queues
        }
        return .applied
    }

    private static func canTransition(from current: AgentCoreTaskState, to next: AgentCoreTaskState) -> Bool {
        if next == .sealed { return !current.isTerminal }
        switch current {
        case .queued: return true
        case .running, .waiting: return next != .queued
        case .suspended:
            return next == .suspended || next == .running || next == .failed || next == .cancelled
        case .retryWaiting:
            return next == .retryWaiting || next == .running || next == .failed || next == .cancelled
        case .completed, .sealed, .failed, .cancelled: return false
        }
    }
}

nonisolated public enum AgentCoreTaskProjectionMeta {
    public static let conversationMetaKey = "agentCoreTaskProjection.v1"

    private struct Payload: Codable {
        let assistantMessageId: UUID
        let projection: AgentCoreTaskProjection
    }

    public static func encode(_ projection: AgentCoreTaskProjection, assistantMessageId: UUID) -> String? {
        guard !projection.tasks.isEmpty,
              let data = try? JSONEncoder().encode(Payload(
                assistantMessageId: assistantMessageId,
                projection: projection
              )) else { return nil }
        return data.base64EncodedString()
    }

    public static func decode(
        from meta: [String: String]?,
        assistantMessageId: UUID
    ) -> AgentCoreTaskProjection? {
        guard let encoded = meta?[conversationMetaKey],
              let data = Data(base64Encoded: encoded),
              let payload = try? JSONDecoder().decode(Payload.self, from: data),
              payload.assistantMessageId == assistantMessageId else { return nil }
        return payload.projection
    }
}

nonisolated public struct AgentCoreSnapshot: Codable, Sendable {
    public let platform: AgentCorePlatform
    public let executionMode: AgentCoreExecutionMode
    public let hostId: String?
    public let phase: String
    public let sessionId: String?
    public let turnId: String?
    public let iteration: Int
    public let messageCount: Int
    public let estimatedContextTokens: Int
    public let contextWindowTokens: Int?
    public let contextUsagePercent: Double?
    public let pendingToolCount: Int
    public let sessionClosing: Bool
    public let sessionClosed: Bool
    public let awaitingPermission: Bool
    public let awaitingHostCall: Bool
    public let usage: AgentCoreTokenUsage
    public let promptCacheDiagnostic: AgentCorePromptCacheDiagnostic?
    public let tasks: AgentCoreTaskProjection?
    public let permissions: AgentCorePermissionProjection?

    enum CodingKeys: String, CodingKey {
        case platform, phase, iteration, usage, tasks, permissions
        case promptCacheDiagnostic = "prompt_cache_diagnostic"
        case executionMode = "execution_mode"
        case hostId = "host_id"
        case sessionId = "session_id"
        case turnId = "turn_id"
        case messageCount = "message_count"
        case estimatedContextTokens = "estimated_context_tokens"
        case contextWindowTokens = "context_window_tokens"
        case contextUsagePercent = "context_usage_percent"
        case pendingToolCount = "pending_tool_count"
        case sessionClosing = "session_closing"
        case sessionClosed = "session_closed"
        case awaitingPermission = "awaiting_permission"
        case awaitingHostCall = "awaiting_host_call"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        platform = try container.decode(AgentCorePlatform.self, forKey: .platform)
        executionMode = try container.decode(AgentCoreExecutionMode.self, forKey: .executionMode)
        hostId = try container.decodeIfPresent(String.self, forKey: .hostId)
        phase = try container.decode(String.self, forKey: .phase)
        sessionId = try container.decodeIfPresent(String.self, forKey: .sessionId)
        turnId = try container.decodeIfPresent(String.self, forKey: .turnId)
        iteration = try container.decodeIfPresent(Int.self, forKey: .iteration) ?? 0
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCount) ?? 0
        estimatedContextTokens = try container.decodeIfPresent(Int.self, forKey: .estimatedContextTokens) ?? 0
        contextWindowTokens = try container.decodeIfPresent(Int.self, forKey: .contextWindowTokens)
        contextUsagePercent = try container.decodeIfPresent(Double.self, forKey: .contextUsagePercent)
        pendingToolCount = try container.decodeIfPresent(Int.self, forKey: .pendingToolCount) ?? 0
        sessionClosing = try container.decodeIfPresent(Bool.self, forKey: .sessionClosing) ?? false
        sessionClosed = try container.decodeIfPresent(Bool.self, forKey: .sessionClosed) ?? false
        awaitingPermission = try container.decodeIfPresent(Bool.self, forKey: .awaitingPermission) ?? false
        awaitingHostCall = try container.decodeIfPresent(Bool.self, forKey: .awaitingHostCall) ?? false
        usage = try container.decode(AgentCoreTokenUsage.self, forKey: .usage)
        promptCacheDiagnostic = try container.decodeIfPresent(
            AgentCorePromptCacheDiagnostic.self,
            forKey: .promptCacheDiagnostic
        )
        tasks = try container.decodeIfPresent(AgentCoreTaskProjection.self, forKey: .tasks)
        permissions = try container.decodeIfPresent(AgentCorePermissionProjection.self, forKey: .permissions)
    }
}

nonisolated public struct AgentCorePermissionProjection: Codable, Sendable {
    public let revision: UInt64
    public let pending: [AgentCorePendingPermissionRequest]
    public var activePolicy: AgentActivePermissionPolicy? = nil

    enum CodingKeys: String, CodingKey {
        case revision, pending
        case activePolicy = "active_policy"
    }
}

nonisolated public struct AgentActivePermissionPolicy: Codable, Sendable {
    public let runID: String
    public let configurationRevision: Int64
    public let profile: IOSPermissionMode

    enum CodingKeys: String, CodingKey {
        case runID = "run_id"
        case configurationRevision = "configuration_revision"
        case profile
    }
}

nonisolated public struct AgentPermissionInstruction: Codable, Equatable, Sendable {
    public let permissionRequestId: String
    public let instruction: String
    enum CodingKeys: String, CodingKey {
        case permissionRequestId = "permission_request_id"
        case instruction
    }
    public static func decodeCore(_ value: AgentCoreJSONValue?) -> [Self]? {
        guard let value else { return [] }
        guard let data = try? JSONEncoder().encode(value),
              let object = try? JSONSerialization.jsonObject(with: data) else { return nil }
        return decode(object)
    }
    public static func decode(_ value: Any?) -> [Self]? {
        guard let value else { return [] }
        guard let records = value as? [[String: Any]],
              records.allSatisfy({ Set($0.keys) == ["permission_request_id", "instruction"] }),
              let data = try? JSONSerialization.data(withJSONObject: records),
              let result = try? JSONDecoder().decode([Self].self, from: data),
              result.allSatisfy({ !$0.permissionRequestId.isEmpty && !$0.instruction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else { return nil }
        return result
    }
}

nonisolated public struct AgentCorePendingPermissionRequest: Codable, Sendable {
    public let allowConversation: Bool
    public let requestId: String
    public let turnId: String
    public let callId: String
    public let capabilityId: String
    public let resource: String
    public let arguments: AgentCoreJSONValue
    public let permissionDomain: String
    public let rationale: String
    public let evaluation: AgentCoreJSONValue
    public let permissionInstructions: [AgentPermissionInstruction]?

    enum CodingKeys: String, CodingKey {
        case allowConversation = "allow_conversation"
        case resource, arguments, rationale, evaluation
        case permissionInstructions = "permission_instructions"
        case permissionDomain = "permission_domain"
        case requestId = "request_id"
        case turnId = "turn_id"
        case callId = "call_id"
        case capabilityId = "capability_id"
    }
}

nonisolated public struct AgentCoreDispatchResponse: Codable, Sendable {
    public let ok: Bool
    public let events: [AgentCoreEvent]
    public let snapshot: AgentCoreSnapshot
    public let error: String?
}

nonisolated public struct AgentCoreContextPlanInput: Codable, Sendable {
    public var contextWindowTokens: Int?
    public var tokenThreshold: Int?
    public var promptTokenLimit: Int?
    public var reservedOutputTokens: Int
    public var estimatedTokens: Int
    public var messageCount: Int
    public var thresholdPercent: Int
    public var keepRecentMessages: Int
    public var minMessagesBeforeCompaction: Int
    public var adaptiveLongHistory: Bool

    public init(
        contextWindowTokens: Int?,
        tokenThreshold: Int? = nil,
        promptTokenLimit: Int? = nil,
        reservedOutputTokens: Int = 4_096,
        estimatedTokens: Int,
        messageCount: Int,
        thresholdPercent: Int = 60,
        keepRecentMessages: Int = 8,
        minMessagesBeforeCompaction: Int = 4,
        adaptiveLongHistory: Bool = true
    ) {
        self.contextWindowTokens = contextWindowTokens
        self.tokenThreshold = tokenThreshold
        self.promptTokenLimit = promptTokenLimit
        self.reservedOutputTokens = reservedOutputTokens
        self.estimatedTokens = estimatedTokens
        self.messageCount = messageCount
        self.thresholdPercent = thresholdPercent
        self.keepRecentMessages = keepRecentMessages
        self.minMessagesBeforeCompaction = minMessagesBeforeCompaction
        self.adaptiveLongHistory = adaptiveLongHistory
    }

    enum CodingKeys: String, CodingKey {
        case contextWindowTokens = "context_window_tokens"
        case tokenThreshold = "token_threshold"
        case promptTokenLimit = "prompt_token_limit"
        case reservedOutputTokens = "reserved_output_tokens"
        case estimatedTokens = "estimated_tokens"
        case messageCount = "message_count"
        case thresholdPercent = "threshold_percent"
        case keepRecentMessages = "keep_recent_messages"
        case minMessagesBeforeCompaction = "min_messages_before_compaction"
        case adaptiveLongHistory = "adaptive_long_history"
    }
}

nonisolated public struct AgentCoreContextPlan: Codable, Sendable {
    public let shouldCompact: Bool
    public let hardLimitReached: Bool
    public let triggerTokens: Int
    public let promptTokenLimit: Int
    public let targetTokens: Int
    public let keepRecentMessages: Int
    public let archiveMessageCount: Int
    public let usagePercent: Double?
    public let reason: String

    enum CodingKeys: String, CodingKey {
        case shouldCompact = "should_compact"
        case hardLimitReached = "hard_limit_reached"
        case triggerTokens = "trigger_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case targetTokens = "target_tokens"
        case keepRecentMessages = "keep_recent_messages"
        case archiveMessageCount = "archive_message_count"
        case usagePercent = "usage_percent"
        case reason
    }
}

nonisolated public enum AgentCoreContextPrepareMode: String, Codable, Sendable {
    case normal
    case contextLimitRecovery = "context_limit_recovery"
    case forceSummary = "force_summary"
}

nonisolated public enum AgentCoreContextPlatform: String, Codable, Sendable {
    case ios, android, macos, cli, web, host, shared, unknown
}

nonisolated public enum AgentCoreContextComponentKind: String, Codable, Sendable {
    case system
    case promptTemplate = "prompt_template"
    case memory
    case sessionMemory = "session_memory"
    case userPreference = "user_preference"
    case personalIntelligence = "personal_intelligence"
    case skill
    case document
    case mcp
    case attachment
    case webRetrieval = "web_retrieval"
    case platformData = "platform_data"
}

nonisolated public struct AgentCoreContextComponentSource: Codable, Sendable, Equatable {
    public var platform: AgentCoreContextPlatform?
    public var uri: String?
    public var title: String?
    public var mimeType: String?
    public var observedAt: String?
    public var authorization: String?
    public var maximumAgeMinutes: UInt64?

    public init(
        platform: AgentCoreContextPlatform? = nil,
        uri: String? = nil,
        title: String? = nil,
        mimeType: String? = nil,
        observedAt: String? = nil,
        authorization: String? = nil,
        maximumAgeMinutes: UInt64? = nil
    ) {
        self.platform = platform
        self.uri = uri
        self.title = title
        self.mimeType = mimeType
        self.observedAt = observedAt
        self.authorization = authorization
        self.maximumAgeMinutes = maximumAgeMinutes
    }

    enum CodingKeys: String, CodingKey {
        case platform, uri, title
        case mimeType = "mime_type"
        case observedAt = "observed_at"
        case authorization
        case maximumAgeMinutes = "maximum_age_minutes"
    }
}

nonisolated public struct AgentCoreContextComponent: Codable, Sendable, Equatable {
    public var kind: AgentCoreContextComponentKind
    public var id: String
    public var content: AgentCoreJSONValue
    public var source: AgentCoreContextComponentSource?

    public init(
        kind: AgentCoreContextComponentKind,
        id: String,
        content: AgentCoreJSONValue,
        source: AgentCoreContextComponentSource? = nil
    ) {
        self.kind = kind
        self.id = id
        self.content = content
        self.source = source
    }
}

nonisolated public struct AgentCoreContextSummaryResult: Codable, Sendable, Equatable {
    public var archiveId: String
    public var archiveDigest: String
    public var content: String

    public init(archiveId: String, archiveDigest: String, content: String) {
        self.archiveId = archiveId
        self.archiveDigest = archiveDigest
        self.content = content
    }

    enum CodingKeys: String, CodingKey {
        case archiveId = "archive_id"
        case archiveDigest = "archive_digest"
        case content
    }
}

nonisolated public struct AgentCoreContextPrepareMessage: Codable, Sendable, Equatable {
    public var id: String
    public var role: String
    public var content: String
    public var name: String?
    public var toolCallId: String?
    public var toolCalls: [AgentCoreToolCall]
    public var ephemeral: Bool
    public var nolaEphemeral: Bool

    public init(
        id: String,
        role: String,
        content: String,
        name: String? = nil,
        toolCallId: String? = nil,
        toolCalls: [AgentCoreToolCall] = [],
        ephemeral: Bool = false,
        nolaEphemeral: Bool = false
    ) {
        self.id = id
        self.role = role
        self.content = content
        self.name = name
        self.toolCallId = toolCallId
        self.toolCalls = toolCalls
        self.ephemeral = ephemeral
        self.nolaEphemeral = nolaEphemeral
    }

    enum CodingKeys: String, CodingKey {
        case id, role, content, name, ephemeral
        case toolCallId = "tool_call_id"
        case toolCalls = "tool_calls"
        case nolaEphemeral = "_nola_ephemeral"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeIfPresent(String.self, forKey: .id) ?? ""
        role = try container.decode(String.self, forKey: .role)
        content = try container.decodeIfPresent(String.self, forKey: .content) ?? ""
        name = try container.decodeIfPresent(String.self, forKey: .name)
        toolCallId = try container.decodeIfPresent(String.self, forKey: .toolCallId)
        toolCalls = try container.decodeIfPresent([AgentCoreToolCall].self, forKey: .toolCalls) ?? []
        ephemeral = try container.decodeIfPresent(Bool.self, forKey: .ephemeral) ?? false
        nolaEphemeral = try container.decodeIfPresent(Bool.self, forKey: .nolaEphemeral) ?? false
    }
}

nonisolated public struct AgentCoreContextPrepareBudget: Codable, Sendable, Equatable {
    public var contextWindowTokens: Int?
    public var promptTokenLimit: Int?
    public var tokenThreshold: Int?
    public var reservedOutputTokens: Int
    public var thresholdPercent: Int?
    public var keepRecentMessages: Int
    public var minMessagesBeforeCompaction: Int?
    public var compactedToolChars: Int?
    public var compactedAssistantChars: Int?

    public init(
        contextWindowTokens: Int?,
        promptTokenLimit: Int? = nil,
        tokenThreshold: Int? = nil,
        reservedOutputTokens: Int = 4_096,
        thresholdPercent: Int? = nil,
        keepRecentMessages: Int = 8,
        minMessagesBeforeCompaction: Int? = nil,
        compactedToolChars: Int? = nil,
        compactedAssistantChars: Int? = nil
    ) {
        self.contextWindowTokens = contextWindowTokens
        self.promptTokenLimit = promptTokenLimit
        self.tokenThreshold = tokenThreshold
        self.reservedOutputTokens = reservedOutputTokens
        self.thresholdPercent = thresholdPercent
        self.keepRecentMessages = keepRecentMessages
        self.minMessagesBeforeCompaction = minMessagesBeforeCompaction
        self.compactedToolChars = compactedToolChars
        self.compactedAssistantChars = compactedAssistantChars
    }

    enum CodingKeys: String, CodingKey {
        case contextWindowTokens = "context_window_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case tokenThreshold = "token_threshold"
        case reservedOutputTokens = "reserved_output_tokens"
        case thresholdPercent = "threshold_percent"
        case keepRecentMessages = "keep_recent_messages"
        case minMessagesBeforeCompaction = "min_messages_before_compaction"
        case compactedToolChars = "compacted_tool_chars"
        case compactedAssistantChars = "compacted_assistant_chars"
    }
}

nonisolated public struct AgentCoreContextPrepareInput: Codable, Sendable, Equatable {
    public var schemaVersion: Int
    public var messages: [AgentCoreContextPrepareMessage]
    public var tools: [AgentCoreJSONValue]
    public var platform: AgentCoreContextPlatform
    public var components: [AgentCoreContextComponent]
    public var budget: AgentCoreContextPrepareBudget
    public var mode: AgentCoreContextPrepareMode
    public var existingSummary: String?
    public var summaryResult: AgentCoreContextSummaryResult?

    public init(
        schemaVersion: Int = 1,
        messages: [AgentCoreContextPrepareMessage],
        tools: [AgentCoreJSONValue] = [],
        platform: AgentCoreContextPlatform = .unknown,
        components: [AgentCoreContextComponent] = [],
        budget: AgentCoreContextPrepareBudget,
        mode: AgentCoreContextPrepareMode = .normal,
        existingSummary: String? = nil,
        summaryResult: AgentCoreContextSummaryResult? = nil
    ) {
        self.schemaVersion = schemaVersion
        self.messages = messages
        self.tools = tools
        self.platform = platform
        self.components = components
        self.budget = budget
        self.mode = mode
        self.existingSummary = existingSummary
        self.summaryResult = summaryResult
    }

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case messages, tools, platform, components, budget, mode
        case existingSummary = "existing_summary"
        case summaryResult = "summary_result"
    }
}

nonisolated public struct AgentCoreContextPrepareMetrics: Codable, Sendable, Equatable {
    public let messageTokens: Int
    public let toolSchemaTokens: Int
    public let totalPromptTokens: Int
    public let promptTokenLimit: Int
    public let reservedOutputTokens: Int
    public let usagePercent: Double
    public let beforeTokens: Int
    public let afterTokens: Int
    public let triggerTokens: Int
    public let targetTokens: Int
    public let archiveMessageCount: Int
    public let transactionCount: Int

    public init(
        messageTokens: Int,
        toolSchemaTokens: Int,
        totalPromptTokens: Int,
        promptTokenLimit: Int,
        reservedOutputTokens: Int,
        usagePercent: Double,
        beforeTokens: Int,
        afterTokens: Int,
        triggerTokens: Int = 0,
        targetTokens: Int = 0,
        archiveMessageCount: Int = 0,
        transactionCount: Int = 0
    ) {
        self.messageTokens = messageTokens
        self.toolSchemaTokens = toolSchemaTokens
        self.totalPromptTokens = totalPromptTokens
        self.promptTokenLimit = promptTokenLimit
        self.reservedOutputTokens = reservedOutputTokens
        self.usagePercent = usagePercent
        self.beforeTokens = beforeTokens
        self.afterTokens = afterTokens
        self.triggerTokens = triggerTokens
        self.targetTokens = targetTokens
        self.archiveMessageCount = archiveMessageCount
        self.transactionCount = transactionCount
    }

    enum CodingKeys: String, CodingKey {
        case messageTokens = "message_tokens"
        case toolSchemaTokens = "tool_schema_tokens"
        case totalPromptTokens = "total_prompt_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case reservedOutputTokens = "reserved_output_tokens"
        case usagePercent = "usage_percent"
        case beforeTokens = "before_tokens"
        case afterTokens = "after_tokens"
        case triggerTokens = "trigger_tokens"
        case targetTokens = "target_tokens"
        case archiveMessageCount = "archive_message_count"
        case transactionCount = "transaction_count"
    }
}

nonisolated public struct AgentCoreContextComponentMetric: Codable, Sendable, Equatable {
    public let kind: AgentCoreContextComponentKind
    public let usedTokens: Int
    public let droppedTokens: Int
    public let truncatedTokens: Int
    public let usedIds: [String]
    public let droppedIds: [String]
    public let truncatedIds: [String]

    enum CodingKeys: String, CodingKey {
        case kind
        case usedTokens = "used_tokens"
        case droppedTokens = "dropped_tokens"
        case truncatedTokens = "truncated_tokens"
        case usedIds = "used_ids"
        case droppedIds = "dropped_ids"
        case truncatedIds = "truncated_ids"
    }
}

nonisolated public struct AgentCoreContextArchiveRequest: Codable, Sendable, Equatable {
    public let archiveId: String
    public let archiveDigest: String
    public let messageIndices: [Int]
    public let messageIds: [String]
    public let messages: [AgentCoreContextPrepareMessage]
    public let existingSummary: String?
    public let targetSummaryTokens: Int

    enum CodingKeys: String, CodingKey {
        case archiveId = "archive_id"
        case archiveDigest = "archive_digest"
        case messageIndices = "message_indices"
        case messageIds = "message_ids"
        case messages
        case existingSummary = "existing_summary"
        case targetSummaryTokens = "target_summary_tokens"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        archiveId = try container.decode(String.self, forKey: .archiveId)
        archiveDigest = try container.decode(String.self, forKey: .archiveDigest)
        messageIndices = try container.decodeIfPresent([Int].self, forKey: .messageIndices) ?? []
        messageIds = try container.decodeIfPresent([String].self, forKey: .messageIds) ?? []
        messages = try container.decodeIfPresent([AgentCoreContextPrepareMessage].self, forKey: .messages) ?? []
        existingSummary = try container.decodeIfPresent(String.self, forKey: .existingSummary)
        targetSummaryTokens = try container.decodeIfPresent(Int.self, forKey: .targetSummaryTokens) ?? 0
    }

    public init(
        archiveId: String,
        archiveDigest: String,
        messageIndices: [Int],
        messageIds: [String],
        messages: [AgentCoreContextPrepareMessage] = [],
        existingSummary: String? = nil,
        targetSummaryTokens: Int = 0
    ) {
        self.archiveId = archiveId
        self.archiveDigest = archiveDigest
        self.messageIndices = messageIndices
        self.messageIds = messageIds
        self.messages = messages
        self.existingSummary = existingSummary
        self.targetSummaryTokens = targetSummaryTokens
    }
}

nonisolated public struct AgentCoreContextPrepareResponse: Codable, Sendable, Equatable {
    public let schemaVersion: Int
    public let action: String
    public let messages: [AgentCoreContextPrepareMessage]
    public let tools: [AgentCoreJSONValue]
    public let payloadDigest: String
    public let changed: Bool
    public let reason: String
    public let metrics: AgentCoreContextPrepareMetrics
    public let componentMetrics: [AgentCoreContextComponentMetric]
    public let archiveRequest: AgentCoreContextArchiveRequest?

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case action, messages, tools
        case payloadDigest = "payload_digest"
        case changed, reason, metrics
        case componentMetrics = "component_metrics"
        case archiveRequest = "archive_request"
    }

    public init(
        schemaVersion: Int = 1,
        action: String,
        messages: [AgentCoreContextPrepareMessage],
        tools: [AgentCoreJSONValue] = [],
        payloadDigest: String = "",
        changed: Bool = false,
        reason: String = "",
        metrics: AgentCoreContextPrepareMetrics,
        componentMetrics: [AgentCoreContextComponentMetric] = [],
        archiveRequest: AgentCoreContextArchiveRequest? = nil
    ) {
        self.schemaVersion = schemaVersion
        self.action = action
        self.messages = messages
        self.tools = tools
        self.payloadDigest = payloadDigest
        self.changed = changed
        self.reason = reason
        self.metrics = metrics
        self.componentMetrics = componentMetrics
        self.archiveRequest = archiveRequest
    }
}

nonisolated public enum AgentCoreRuntimeError: LocalizedError {
    case libraryUnavailable
    case incompatibleABI(UInt32)
    case runtimeCreationFailed
    case missingResult
    case invalidUTF8
    case core(String)
    case coreProtocol(message: String, code: String, category: String)

    public var errorDescription: String? {
        switch self {
        case .libraryUnavailable: return "NolaCore static library is unavailable"
        case .incompatibleABI(let version): return "Unsupported NolaCore ABI version: \(version)"
        case .runtimeCreationFailed: return "Failed to create NolaCore agent runtime"
        case .missingResult: return "NolaCore returned no result"
        case .invalidUTF8: return "NolaCore returned invalid UTF-8"
        case .core(let message): return message
        case .coreProtocol(let message, _, _): return message
        }
    }
}

nonisolated private struct AgentCoreCommandEnvelope: Encodable {
    let type: String
    var sessionId: String?
    var input: String?
    var history: [AgentCoreMessage]?
    var tools: [AgentCoreJSONValue]?
    var turnId: String?
    var content: String?
    var thinking: String?
    var toolCalls: [AgentCoreToolCall]?
    var usage: AgentCoreTokenUsage?
    var error: String?
    var capability: AgentCoreCapability?
    var authorization: AgentCoreAuthorization?
    var rationale: String?
    var scope: String?
    var arguments: AgentCoreJSONValue?
    var requestId: String?
    var decision: AgentCorePermissionDecision?
    var callId: String?
    var toolName: String?
    var output: AgentCoreJSONValue?
    var grant: AgentCoreJSONValue?
    var isError: Bool?
    var reason: String?

    enum CodingKeys: String, CodingKey {
        case type, input, history, tools, content, thinking, usage, error, capability, authorization
        case rationale, scope, arguments, decision, output, grant, reason
        case sessionId = "session_id"
        case turnId = "turn_id"
        case toolCalls = "tool_calls"
        case requestId = "request_id"
        case callId = "call_id"
        case toolName = "tool_name"
        case isError = "is_error"
    }
}

/// Pure context transforms. No execution handle, submission or checkpoint writer.
nonisolated public enum AgentCoreContextEngine {
    public static func contextPlan(
        _ input: AgentCoreContextPlanInput
    ) -> AgentCoreContextPlan? {
#if canImport(NolaCore)
        guard nola_core_context_engine_abi_version() == 1 else { return nil }
        guard let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_context_plan(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let json = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(
            AgentCoreContextPlan.self,
            from: Data(json.utf8)
        )
#else
        return nil
#endif
    }

    public static func contextPrepare(
        _ input: AgentCoreContextPrepareInput
    ) -> AgentCoreContextPrepareResponse? {
#if canImport(NolaCore)
        guard nola_core_context_engine_abi_version() == 1 else { return nil }
        guard let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_context_prepare(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let json = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(
            AgentCoreContextPrepareResponse.self,
            from: Data(json.utf8)
        )
#else
        return nil
#endif
    }
}
