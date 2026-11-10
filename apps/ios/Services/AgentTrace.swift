// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: iOS projection of the versioned Rust Core Agent Trace contract.
// - Architecture: Transport events normalize here once, then a pure reducer owns visible run state.
// - Security: Adapters keep redaction at the existing HostAgentToolEvent boundary.

import Foundation

nonisolated public let agentTraceSchemaVersion = 1

nonisolated public enum AgentTraceKind: String, Codable, Sendable, Equatable {
    case runStarted = "run.started"
    case promptComposed = "prompt.composed"
    case phaseChanged = "phase.changed"
    case toolStarted = "tool.started"
    case toolProgress = "tool.progress"
    case toolPermissionRequested = "tool.permission.requested"
    case toolPermissionResolved = "tool.permission.resolved"
    case toolCompleted = "tool.completed"
    case hookStarted = "hook.started"
    case hookCompleted = "hook.completed"
    case planUpdated = "plan.updated"
    case subagentUpdated = "subagent.updated"
    case usageSnapshot = "usage.snapshot"
    case memoryUsed = "memory.used"
    case validationUpdated = "validation.updated"
    case contextCompacted = "context.compacted"
    case runCompleted = "run.completed"
    case runFailed = "run.failed"
    case runCancelled = "run.cancelled"
}

nonisolated public enum AgentTraceStatus: String, Codable, Sendable, Equatable {
    case pending
    case running
    case waiting
    case succeeded
    case failed
    case cancelled
    case incomplete
}

nonisolated public enum AgentTraceVisibility: String, Codable, Sendable, Equatable {
    case summary
    case detail
    case `internal`
}

nonisolated public enum AgentTraceResourceOperation: String, Codable, Sendable, Equatable {
    case unknown
    case read
    case create
    case update
    case delete
    case execute
    case open
    case search
}

nonisolated public struct AgentTraceResource: Codable, Sendable, Equatable {
    public var uri: String
    public var operation: AgentTraceResourceOperation
    public var displayName: String?
    public var mimeType: String?

    public init(
        uri: String,
        operation: AgentTraceResourceOperation,
        displayName: String? = nil,
        mimeType: String? = nil
    ) {
        self.uri = uri
        self.operation = operation
        self.displayName = displayName
        self.mimeType = mimeType
    }

    enum CodingKeys: String, CodingKey {
        case uri, operation
        case displayName = "display_name"
        case mimeType = "mime_type"
    }
}

nonisolated public struct AgentTraceUsage: Codable, Sendable, Equatable {
    public var usedTokens: Int
    public var contextWindowTokens: Int
    public var promptTokenLimit: Int
    public var cachedInputTokens: Int?
    public var cachedInputTokensReported: Bool?
    public var cacheWriteInputTokens: Int?
    public var cacheWriteInputTokensReported: Bool?

    public init(
        usedTokens: Int = 0,
        contextWindowTokens: Int = 0,
        promptTokenLimit: Int = 0,
        cachedInputTokens: Int? = nil,
        cachedInputTokensReported: Bool? = nil,
        cacheWriteInputTokens: Int? = nil,
        cacheWriteInputTokensReported: Bool? = nil
    ) {
        self.usedTokens = usedTokens
        self.contextWindowTokens = contextWindowTokens
        self.promptTokenLimit = promptTokenLimit
        self.cachedInputTokens = cachedInputTokens
        self.cachedInputTokensReported = cachedInputTokensReported
        self.cacheWriteInputTokens = cacheWriteInputTokens
        self.cacheWriteInputTokensReported = cacheWriteInputTokensReported
    }

    enum CodingKeys: String, CodingKey {
        case usedTokens = "used_tokens"
        case contextWindowTokens = "context_window_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case cachedInputTokens = "cached_input_tokens"
        case cachedInputTokensReported = "cached_input_tokens_reported"
        case cacheWriteInputTokens = "cache_write_input_tokens"
        case cacheWriteInputTokensReported = "cache_write_input_tokens_reported"
    }
}

nonisolated public struct AgentTraceValidation: Codable, Sendable, Equatable {
    public var status: String
    public var level: String?
    public var summary: String?
    public var nextAction: String?

    public init(
        status: String,
        level: String? = nil,
        summary: String? = nil,
        nextAction: String? = nil
    ) {
        self.status = status
        self.level = level
        self.summary = summary
        self.nextAction = nextAction
    }

    enum CodingKeys: String, CodingKey {
        case status, level, summary
        case nextAction = "next_action"
    }
}

nonisolated public struct AgentTraceMemoryUsage: Codable, Sendable, Equatable {
    public var recordId: String
    public var componentId: String
    public var scope: AgentCoreJSONValue?
    public var namespace: AgentCoreJSONValue?
    public var provenanceKind: String?
    public var source: String?
    public var title: String?
    public var observedAt: String?
    public var requiresCurrentStateVerification: Bool
    public var truncated: Bool

    enum CodingKeys: String, CodingKey {
        case recordId = "record_id"
        case componentId = "component_id"
        case scope
        case namespace
        case provenanceKind = "provenance_kind"
        case source, title
        case observedAt = "observed_at"
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case truncated
    }
}

nonisolated public struct AgentTracePayload: Codable, Sendable, Equatable {
    public var phase: String?
    public var summary: String?
    public var detail: String?
    public var toolName: String?
    public var callId: String?
    public var arguments: AgentCoreJSONValue?
    public var result: AgentCoreJSONValue?
    public var permissionRequestId: String?
    public var permissionScope: String?
    public var permissionDecision: String?
    public var reason: String?
    public var code: String?
    public var progress: Double?
    public var usage: AgentTraceUsage?
    public var promptCacheDiagnostic: AgentCorePromptCacheDiagnostic?
    public var validation: AgentTraceValidation?
    public var resources: [AgentTraceResource]
    public var memories: [AgentTraceMemoryUsage]

    public init(
        phase: String? = nil,
        summary: String? = nil,
        detail: String? = nil,
        toolName: String? = nil,
        callId: String? = nil,
        arguments: AgentCoreJSONValue? = nil,
        result: AgentCoreJSONValue? = nil,
        permissionRequestId: String? = nil,
        permissionScope: String? = nil,
        permissionDecision: String? = nil,
        reason: String? = nil,
        code: String? = nil,
        progress: Double? = nil,
        usage: AgentTraceUsage? = nil,
        promptCacheDiagnostic: AgentCorePromptCacheDiagnostic? = nil,
        validation: AgentTraceValidation? = nil,
        resources: [AgentTraceResource] = [],
        memories: [AgentTraceMemoryUsage] = []
    ) {
        self.phase = phase
        self.summary = summary
        self.detail = detail
        self.toolName = toolName
        self.callId = callId
        self.arguments = arguments
        self.result = result
        self.permissionRequestId = permissionRequestId
        self.permissionScope = permissionScope
        self.permissionDecision = permissionDecision
        self.reason = reason
        self.code = code
        self.progress = progress
        self.usage = usage
        self.promptCacheDiagnostic = promptCacheDiagnostic
        self.validation = validation
        self.resources = resources
        self.memories = memories
    }

    enum CodingKeys: String, CodingKey {
        case phase, summary, detail, arguments, result, reason, code, progress, usage, validation, resources, memories
        case promptCacheDiagnostic = "prompt_cache_diagnostic"
        case toolName = "tool_name"
        case callId = "call_id"
        case permissionRequestId = "permission_request_id"
        case permissionScope = "permission_scope"
        case permissionDecision = "permission_decision"
    }

    public init(from decoder: Decoder) throws {
        enum RetiredKeys: String, CodingKey { case delta }
        let retired = try decoder.container(keyedBy: RetiredKeys.self)
        guard !retired.contains(.delta) else {
            throw DecodingError.dataCorruptedError(forKey: .delta, in: retired, debugDescription: "Trace payload cannot carry assistant content")
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        phase = try container.decodeIfPresent(String.self, forKey: .phase)
        summary = try container.decodeIfPresent(String.self, forKey: .summary)
        detail = try container.decodeIfPresent(String.self, forKey: .detail)
        toolName = try container.decodeIfPresent(String.self, forKey: .toolName)
        callId = try container.decodeIfPresent(String.self, forKey: .callId)
        arguments = try container.decodeIfPresent(AgentCoreJSONValue.self, forKey: .arguments)
        result = try container.decodeIfPresent(AgentCoreJSONValue.self, forKey: .result)
        permissionRequestId = try container.decodeIfPresent(String.self, forKey: .permissionRequestId)
        permissionScope = try container.decodeIfPresent(String.self, forKey: .permissionScope)
        permissionDecision = try container.decodeIfPresent(String.self, forKey: .permissionDecision)
        reason = try container.decodeIfPresent(String.self, forKey: .reason)
        code = try container.decodeIfPresent(String.self, forKey: .code)
        progress = try container.decodeIfPresent(Double.self, forKey: .progress)
        usage = try container.decodeIfPresent(AgentTraceUsage.self, forKey: .usage)
        promptCacheDiagnostic = try container.decodeIfPresent(
            AgentCorePromptCacheDiagnostic.self,
            forKey: .promptCacheDiagnostic
        )
        validation = try container.decodeIfPresent(AgentTraceValidation.self, forKey: .validation)
        resources = try container.decodeIfPresent([AgentTraceResource].self, forKey: .resources) ?? []
        memories = try container.decodeIfPresent([AgentTraceMemoryUsage].self, forKey: .memories) ?? []
    }
}

nonisolated public struct AgentTraceEvent: Codable, Sendable, Equatable, Identifiable {
    public var schemaVersion: Int
    public var traceId: String
    public var runId: String
    public var conversationId: String
    public var messageId: String?
    public var turnId: String
    public var eventId: String
    public var sequence: UInt64
    public var emittedAt: String
    public var kind: AgentTraceKind
    public var spanId: String?
    public var parentSpanId: String?
    public var status: AgentTraceStatus
    public var visibility: AgentTraceVisibility
    public var payload: AgentTracePayload

    public var id: String { eventId }

    public init(
        schemaVersion: Int = agentTraceSchemaVersion,
        traceId: String,
        runId: String,
        conversationId: String,
        messageId: String? = nil,
        turnId: String,
        eventId: String = "event_\(UUID().uuidString)",
        sequence: UInt64,
        emittedAt: String = ISO8601DateFormatter().string(from: Date()),
        kind: AgentTraceKind,
        spanId: String? = nil,
        parentSpanId: String? = nil,
        status: AgentTraceStatus = .running,
        visibility: AgentTraceVisibility = .summary,
        payload: AgentTracePayload = .init()
    ) {
        self.schemaVersion = schemaVersion
        self.traceId = traceId
        self.runId = runId
        self.conversationId = conversationId
        self.messageId = messageId
        self.turnId = turnId
        self.eventId = eventId
        self.sequence = sequence
        self.emittedAt = emittedAt
        self.kind = kind
        self.spanId = spanId
        self.parentSpanId = parentSpanId
        self.status = status
        self.visibility = visibility
        self.payload = payload
    }

    enum CodingKeys: String, CodingKey {
        case kind, sequence, status, visibility, payload
        case schemaVersion = "schema_version"
        case traceId = "trace_id"
        case runId = "run_id"
        case conversationId = "conversation_id"
        case messageId = "message_id"
        case turnId = "turn_id"
        case eventId = "event_id"
        case emittedAt = "emitted_at"
        case spanId = "span_id"
        case parentSpanId = "parent_span_id"
    }
}

nonisolated public struct AgentTraceActivity: Sendable, Equatable, Identifiable {
    public var spanId: String
    public var toolName: String?
    public var status: AgentTraceStatus
    public var summary: String?
    public var detail: String?
    public var arguments: AgentCoreJSONValue?
    public var result: AgentCoreJSONValue?
    public var resources: [AgentTraceResource]
    public var permissionRequestId: String?
    public var startedSequence: UInt64
    public var finishedSequence: UInt64?

    public var id: String { spanId }

    public init(
        spanId: String,
        toolName: String? = nil,
        status: AgentTraceStatus,
        summary: String? = nil,
        detail: String? = nil,
        arguments: AgentCoreJSONValue? = nil,
        result: AgentCoreJSONValue? = nil,
        resources: [AgentTraceResource] = [],
        permissionRequestId: String? = nil,
        startedSequence: UInt64,
        finishedSequence: UInt64? = nil
    ) {
        self.spanId = spanId
        self.toolName = toolName
        self.status = status
        self.summary = summary
        self.detail = detail
        self.arguments = arguments
        self.result = result
        self.resources = resources
        self.permissionRequestId = permissionRequestId
        self.startedSequence = startedSequence
        self.finishedSequence = finishedSequence
    }
}

nonisolated public struct AgentRunProjection: Sendable, Equatable {
    public var traceId: String
    public var runId: String
    public var conversationId: String
    public var turnId: String
    public var status: AgentTraceStatus
    public var phase: String?
    public var activities: [AgentTraceActivity]
    public var memories: [AgentTraceMemoryUsage]
    public var usage: AgentTraceUsage?
    public var promptCacheDiagnostic: AgentCorePromptCacheDiagnostic?
    public var validation: AgentTraceValidation?
    public var terminalReason: String?
    public var lastSequence: UInt64

    public init(
        traceId: String = "",
        runId: String = "",
        conversationId: String = "",
        turnId: String = "",
        status: AgentTraceStatus = .running,
        phase: String? = nil,
        activities: [AgentTraceActivity] = [],
        memories: [AgentTraceMemoryUsage] = [],
        usage: AgentTraceUsage? = nil,
        promptCacheDiagnostic: AgentCorePromptCacheDiagnostic? = nil,
        validation: AgentTraceValidation? = nil,
        terminalReason: String? = nil,
        lastSequence: UInt64 = 0
    ) {
        self.traceId = traceId
        self.runId = runId
        self.conversationId = conversationId
        self.turnId = turnId
        self.status = status
        self.phase = phase
        self.activities = activities
        self.memories = memories
        self.usage = usage
        self.promptCacheDiagnostic = promptCacheDiagnostic
        self.validation = validation
        self.terminalReason = terminalReason
        self.lastSequence = lastSequence
    }

    public var hostToolEvents: [HostAgentToolEvent] {
        activities.map { activity in
            HostAgentToolEvent(
                id: activity.spanId,
                name: activity.toolName ?? "tool",
                argsPreview: activity.arguments?.agentTraceDisplayText ?? "",
                status: activity.hostToolStatus,
                display: activity.summary ?? "",
                text: activity.result?.agentTraceDisplayText ?? activity.detail ?? ""
            )
        }
    }

    /// Sequence and transport identifiers are intentionally excluded. The UI
    /// must not publish another render pass for an idempotent progress event
    /// that leaves the user-visible run state unchanged.
    public func isVisuallyEquivalent(to other: AgentRunProjection) -> Bool {
        status == other.status
            && phase == other.phase
            && activities == other.activities
            && memories == other.memories
            && usage == other.usage
            && validation == other.validation
            && terminalReason == other.terminalReason
    }

    public var isTerminal: Bool {
        [.succeeded, .failed, .cancelled, .incomplete].contains(status)
    }

    public var hasFinishedThinking: Bool {
        isTerminal
    }
}

nonisolated public enum AgentTraceApplyResult: Sendable, Equatable {
    case applied
    case duplicate
    case gap(expected: UInt64, received: UInt64)
    case outOfOrder(last: UInt64, received: UInt64)
}

nonisolated public struct AgentTraceReducer: Sendable {
    public private(set) var projection = AgentRunProjection()
    private var seenEventIds = Set<String>()

    public init() {}

    @discardableResult
    public mutating func apply(_ event: AgentTraceEvent) -> AgentTraceApplyResult {
        if seenEventIds.contains(event.eventId) {
            return .duplicate
        }
        let expected = projection.lastSequence &+ 1
        if event.sequence > expected {
            return .gap(expected: expected, received: event.sequence)
        }
        if event.sequence < expected {
            return .outOfOrder(last: projection.lastSequence, received: event.sequence)
        }

        seenEventIds.insert(event.eventId)
        projection.lastSequence = event.sequence
        projection.traceId = event.traceId
        projection.runId = event.runId
        projection.conversationId = event.conversationId
        projection.turnId = event.turnId
        projection.status = event.status
        applyPayload(event)
        return .applied
    }

    private mutating func applyPayload(_ event: AgentTraceEvent) {
        switch event.kind {
        case .promptComposed:
            break
        case .toolProgress where event.payload.toolName == nil && event.payload.callId == nil:
            // Model/Host progress is not a tool call. A nested model request
            // during compaction cannot replace the more specific stage.
            if projection.phase != "compacting" {
                if event.payload.summary == "model_retry" {
                    projection.phase = "model_retry"
                } else if event.payload.summary == "model_attempt_started" {
                    projection.phase = "processing"
                }
            }
        case .toolStarted, .toolProgress, .toolPermissionRequested, .toolPermissionResolved, .toolCompleted:
            applyActivity(event)
        case .hookStarted, .hookCompleted:
            // Hooks remain in the durable Trace for diagnostics, but they are
            // not user work and should not flash as transient tool cards.
            break
        case .usageSnapshot:
            projection.usage = event.payload.usage
            projection.promptCacheDiagnostic = event.payload.promptCacheDiagnostic
        case .memoryUsed:
            projection.memories = event.payload.memories
        case .contextCompacted:
            if let usage = event.payload.usage {
                projection.usage = usage
            }
            projection.phase = "processing"
        case .validationUpdated:
            projection.validation = event.payload.validation
        case .runCompleted, .runFailed, .runCancelled:
            projection.terminalReason = event.payload.reason
            if ["compacting", "model_retry"].contains(projection.phase ?? "") {
                projection.phase = nil
            }
        case .planUpdated, .subagentUpdated:
            break
        case .runStarted, .phaseChanged:
            if let phase = event.payload.phase {
                projection.phase = phase
            }
        }
    }

    private mutating func applyActivity(_ event: AgentTraceEvent) {
        let spanId = event.spanId ?? event.payload.callId ?? event.eventId
        let matchingIndex = projection.activities.firstIndex(where: { activity in
            activity.spanId == spanId
                || (
                    event.kind == .toolPermissionResolved
                        && event.payload.permissionRequestId != nil
                        && activity.permissionRequestId == event.payload.permissionRequestId
                )
        })
        if let index = matchingIndex {
            var activity = projection.activities[index]
            activity.status = event.status
            activity.toolName = event.payload.toolName ?? activity.toolName
            activity.summary = event.payload.summary ?? activity.summary
            activity.detail = event.payload.detail ?? activity.detail
            activity.arguments = event.payload.arguments ?? activity.arguments
            activity.result = event.payload.result ?? activity.result
            activity.permissionRequestId = event.payload.permissionRequestId ?? activity.permissionRequestId
            if !event.payload.resources.isEmpty {
                activity.resources = event.payload.resources
            }
            if event.kind == .toolCompleted || event.kind == .hookCompleted {
                activity.finishedSequence = event.sequence
            }
            projection.activities[index] = activity
        } else {
            projection.activities.append(.init(
                spanId: spanId,
                toolName: event.payload.toolName,
                status: event.status,
                summary: event.payload.summary,
                detail: event.payload.detail,
                arguments: event.payload.arguments,
                result: event.payload.result,
                resources: event.payload.resources,
                permissionRequestId: event.payload.permissionRequestId,
                startedSequence: event.sequence,
                finishedSequence: (event.kind == .toolCompleted || event.kind == .hookCompleted)
                    ? event.sequence
                    : nil
            ))
        }
    }
}

nonisolated public struct AgentTraceContext: Sendable {
    public private(set) var traceId: String
    public private(set) var runId: String
    public private(set) var conversationId: String
    public private(set) var messageId: String?
    public private(set) var turnId: String
    private var nextSequence: UInt64

    public init(conversationId: String, messageId: String? = nil) {
        let identifier = UUID().uuidString
        traceId = "trace_\(identifier)"
        runId = "run_\(identifier)"
        self.conversationId = conversationId
        self.messageId = messageId ?? "assistant_\(identifier)"
        turnId = "turn_\(identifier)"
        nextSequence = 1
    }

    public mutating func adopt(_ event: AgentTraceEvent) {
        traceId = event.traceId
        runId = event.runId
        conversationId = event.conversationId
        messageId = event.messageId
        turnId = event.turnId
        nextSequence = max(nextSequence, event.sequence &+ 1)
    }

    public mutating func makeEvent(
        kind: AgentTraceKind,
        status: AgentTraceStatus = .running,
        visibility: AgentTraceVisibility = .summary,
        spanId: String? = nil,
        payload: AgentTracePayload = .init()
    ) -> AgentTraceEvent {
        defer { nextSequence &+= 1 }
        return AgentTraceEvent(
            traceId: traceId,
            runId: runId,
            conversationId: conversationId,
            messageId: messageId,
            turnId: turnId,
            sequence: nextSequence,
            kind: kind,
            spanId: spanId,
            parentSpanId: turnId,
            status: status,
            visibility: visibility,
            payload: payload
        )
    }
}

nonisolated public enum AgentTraceAdapter {
    public static func decodeNestedTrace(from object: [String: Any]) -> AgentTraceEvent? {
        let candidate = (object["trace"] as? [String: Any]) ?? object
        guard candidate["schema_version"] != nil,
              JSONSerialization.isValidJSONObject(candidate),
              let data = try? JSONSerialization.data(withJSONObject: candidate) else {
            return nil
        }
        return try? JSONDecoder().decode(AgentTraceEvent.self, from: data)
    }

    public static func web(
        _ event: WebAgentStreamEvent,
        context: inout AgentTraceContext
    ) -> [AgentTraceEvent] {
        switch event {
        case .coreTurnTerminal(_, let trace):
            guard let trace else { return [] }
            context.adopt(trace)
            return [trace]
        case .coreTaskUpdated, .coreSkillRuntimeUpdated, .coreCapabilitiesActivated,
                .corePermissionProjectionUpdated, .coreMemoryRuntimeProjectionUpdated,
                .contextUsageUpdated,
                .submissionReceipt, .assistantJournalCommitted, .assistantRunTerminalSealed:
            return []
        case .delegatedPermissionRequest:
            return []
        case .externalSessionStarted:
            // Session continuity is persisted by ChatManager. It is transport
            // metadata, not a user-visible reasoning or tool event.
            return []
        case .trace(let trace):
            context.adopt(trace)
            return [trace]
        case .permissionRequest(let request, _):
            let tool = HostAgentToolEvent(
                id: request.id,
                name: request.toolName,
                argsPreview: request.argsPreview,
                status: .permissionRequired,
                display: request.message.isEmpty ? request.reason : request.message
            )
            return [context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: request.id,
                payload: .init(
                    summary: tool.display,
                    toolName: request.toolName,
                    callId: request.id,
                    arguments: .object(["args_preview": .string(request.argsPreview)]),
                    permissionRequestId: request.id,
                    permissionScope: request.match
                )
            )]
        case .deviceToolRequest(let request, _):
            return [context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: request.id,
                payload: .init(
                    summary: "Device approval required",
                    toolName: request.toolName,
                    callId: request.id,
                    arguments: request.arguments,
                    permissionRequestId: request.id,
                    permissionScope: request.capability?.rawValue
                )
            )]
        case .error(let message):
            return [context.makeEvent(
                kind: .runFailed,
                status: .failed,
                payload: .init(summary: message, reason: message)
            )]
        }
    }

    public static func permissionResolved(
        requestId: String,
        toolName: String,
        allowed: Bool,
        context: inout AgentTraceContext
    ) -> AgentTraceEvent {
        return context.makeEvent(
            kind: .toolPermissionResolved,
            status: allowed ? .running : .failed,
            spanId: requestId,
            payload: .init(
                toolName: toolName,
                callId: requestId,
                permissionRequestId: requestId,
                permissionDecision: allowed ? "allow_once" : "deny"
            )
        )
    }

}

private extension AgentTraceActivity {
    nonisolated var hostToolStatus: HostAgentToolEvent.Status {
        switch status {
        case .pending, .running:
            return .running
        case .waiting:
            return .permissionRequired
        case .succeeded:
            return .succeeded
        case .failed, .incomplete:
            return .failed
        case .cancelled:
            return .cancelled
        }
    }
}

private extension AgentCoreJSONValue {
    nonisolated var agentTraceDisplayText: String {
        switch self {
        case .string(let value):
            return value
        case .number(let value):
            return String(value)
        case .signedInteger(let value):
            return String(value)
        case .unsignedInteger(let value):
            return String(value)
        case .bool(let value):
            return value ? "true" : "false"
        case .object(let values):
            if case .string(let preview)? = values["args_preview"] {
                return preview
            }
            if case .string(let text)? = values["text"] {
                return text
            }
            return values.keys.sorted().map { key in
                "\(key): \(values[key]?.agentTraceDisplayText ?? "")"
            }.joined(separator: "\n")
        case .array(let values):
            return values.map(\.agentTraceDisplayText).joined(separator: "\n")
        case .null:
            return ""
        }
    }
}
