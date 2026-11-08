// SPDX-License-Identifier: Apache-2.0
// Versioned iOS decoder for the UI-neutral Core conversation projection.

import Foundation

nonisolated struct IOSConversationProjectionV1: Decodable, Equatable, Sendable {
    let contractVersion: UInt32
    let conversationID: String
    let executionOwnership: AppleConversationExecutionOwnership
    let revision: UInt64
    let title: String
    let messages: [Message]
    let activeRun: RunActivity?
    let contextUsage: ContextUsage
    let surface: Surface

    struct Message: Decodable, Equatable, Sendable {
        let messageID: String
        let role: Role
        let state: MessageState
        let revision: UInt64
        let createdAtMs: Int64
        let parts: [Part]

        private enum CodingKeys: String, CodingKey {
            case role, state, revision, parts
            case messageID = "message_id"
            case createdAtMs = "created_at_ms"
        }

        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            messageID = try v.decode(String.self, forKey: .messageID)
            role = try v.decode(Role.self, forKey: .role)
            state = try v.decode(MessageState.self, forKey: .state)
            revision = try v.decode(UInt64.self, forKey: .revision)
            createdAtMs = try v.decode(Int64.self, forKey: .createdAtMs)
            parts = try v.decode([Part].self, forKey: .parts)
            for case .tool(let tool) in parts {
                if let owner = tool.fullOutput, owner.messageID != messageID || owner.partID != tool.partID || owner.toolCallID != tool.toolCallID {
                    throw DecodingError.dataCorruptedError(forKey: .parts, in: v, debugDescription: "Full output belongs to a foreign message, Part or tool call")
                }
            }
        }
    }

    enum Role: String, Decodable, Equatable, Sendable { case user, assistant, system }
    enum MessageState: String, Decodable, Equatable, Sendable { case pending, running, completed, cancelled, failed }
    enum ActivityState: String, Decodable, Equatable, Sendable {
        case pending, running
        case waitingForPermission = "waiting_for_permission"
        case waitingForInput = "waiting_for_input"
        case completed, sealed, cancelled, failed
    }

    struct AffectedResource: Decodable, Equatable, Sendable {
        let resourceType: String
        let uri: String
        let operation: String
        private enum CodingKeys: String, CodingKey {
            case uri, operation
            case resourceType = "resource_type"
        }
    }

    /// The public and Core paths share one lossless integer codec. Decoding
    /// through a second Double-only tree would round Tool and history facts.
    typealias StructuredValue = AgentCoreJSONValue

    /// Read-only historical data. Never converted into a live tool/permission Part.
    struct HistoricalProjection: Decodable, Equatable, Sendable {
        let owner: StructuredValue
        let canonicalRevision: Int64
        let source: StructuredValue?
        let schema: String?
        let canonicalKind: String
        let binding: String?
        let status: Status
        let fields: [String: Field]
        let canonicalText: TextPreview?
        struct TextPreview: Decodable, Equatable, Sendable {
            let text: String
            let totalUTF8Bytes: UInt64
            private enum CodingKeys: String, CodingKey { case text; case totalUTF8Bytes = "total_utf8_bytes" }
        }
        struct Status: Decodable, Equatable, Sendable {
            enum State: String, Decodable { case supported, deferred, unsupported; case notLoaded = "not_loaded" }
            let state: State
            let code: String?
            let reason: String?
        }
        enum Field: Decodable, Equatable, Sendable {
            case complete(StructuredValue)
            case deferred(StructuredValue)
            private enum Keys: String, CodingKey { case availability, value, locator }
            init(from decoder: Decoder) throws {
                let values = try decoder.container(keyedBy: Keys.self)
                switch try values.decode(String.self, forKey: .availability) {
                case "complete": self = .complete(try values.decode(StructuredValue.self, forKey: .value))
                case "deferred": self = .deferred(try values.decode(StructuredValue.self, forKey: .locator))
                default: throw DecodingError.dataCorruptedError(forKey: .availability, in: values, debugDescription: "Unknown historical availability")
                }
            }
        }
        private enum CodingKeys: String, CodingKey {
            case owner, source, schema, binding, status, fields
            case canonicalRevision = "canonical_revision"
            case canonicalKind = "canonical_kind"
            case canonicalText = "canonical_text"
        }
        /// A historical wrapper never grants edits. Decode only a complete,
        /// canonically bound reference; partial/untrusted fields stay diagnostic.
        var uiSurfaceReference: AppleUISurfaceReference? {
            guard schema == "message_part_view", canonicalKind == "ui_surface", binding == "canonical",
                  status.state == .supported,
                  fields.values.allSatisfy({ if case .complete = $0 { return true }; return false }),
                  case .complete(.string("ui_surface"))? = fields["kind"],
                  case .complete(.string(let partID))? = fields["part_id"], !partID.isEmpty,
                  case .complete(let revisionValue)? = fields["revision"],
                  let revisionData = try? JSONEncoder().encode(revisionValue),
                  (try? JSONDecoder().decode(UInt64.self, from: revisionData)) != nil,
                  case .complete(let value)? = fields["surface"],
                  case .complete(.string)? = fields["summary"],
                  let encoded = try? JSONEncoder().encode(value) else { return nil }
            return try? JSONDecoder().decode(AppleUISurfaceReference.self, from: encoded)
        }

        var uiActionSummary: String? {
            guard schema == "message_part_view", canonicalKind == "ui_action",
                  binding == "canonical", status.state == .supported,
                  case .complete(.string(let summary))? = fields["summary"] else { return nil }
            return summary
        }

        var decodedText: String? {
            guard status.state != .unsupported, status.state != .notLoaded,
                  ["text", "reasoning"].contains(canonicalKind) else { return nil }
            let name: String
            switch (schema, canonicalKind) {
            case ("agent_part", "text"): name = "text"
            case ("message_part_view", "text"): name = "markdown"
            case ("agent_part", "reasoning"): name = "detail"
            case ("message_part_view", "reasoning"): name = "summary"
            default: return nil
            }
            if case .complete(.string(let value)) = fields[name] { return value }
            return nil
        }
        /// User content suitable for the chat timeline. Decoder status and
        /// schema diagnostics remain available to the explicit detail sheet.
        var timelineText: String? {
            guard canonicalKind != "reasoning" else { return nil }
            if canonicalKind == "ui_action" { return uiActionSummary }
            return decodedText ?? canonicalText?.text
        }
        /// Fully disclosed plain text is already visible in the timeline,
        /// whether supplied by semantic decoding or a complete canonical body.
        /// Partial previews and unknown part types retain the detail affordance.
        var needsHistoricalDetail: Bool {
            guard canonicalKind == "text" else { return true }
            // Core's canonical text is independently disclosure-checked. A
            // metadata-only JSON envelope can be unsupported while its entire
            // plain-text body is already rendered. Count bytes, not characters.
            if let preview = canonicalText, timelineText == preview.text,
               UInt64(preview.text.utf8.count) == preview.totalUTF8Bytes {
                return false
            }
            guard status.state == .supported, decodedText != nil else { return true }
            return fields.values.contains { field in
                if case .deferred = field { return true }
                return false
            }
        }
        var statusLabel: String {
            let label = String(localized: "Historical content")
            return "\(label) · \(canonicalKind) · \(status.state.rawValue)" + ((status.code ?? status.reason).map { " · \($0)" } ?? "") + (canonicalText.map { "\n" + $0.text } ?? "")
        }
    }

    struct ModelAttemptFailure: Decodable, Equatable, Sendable {
        let code: String
        let message: String
        let retryable: Bool
    }

    struct ModelAttemptPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let state: ActivityState
        let attemptID: String
        let provider: String?
        let model: String?
        let failure: ModelAttemptFailure
        let fallbackAttemptID: String?
        let fallbackProvider: String?
        let fallbackModel: String?
        private enum CodingKeys: String, CodingKey {
            case revision, state, provider, model, failure
            case partID = "part_id"; case attemptID = "attempt_id"
            case fallbackAttemptID = "fallback_attempt_id"
            case fallbackProvider = "fallback_provider"; case fallbackModel = "fallback_model"
        }
    }

    struct TaskControlPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let taskID: String
        let state: ActivityState
        /// Present only when hydrated from an exact journal owner.
        let parentRunID: String?
        let supportsGuidance: Bool
        let supportsCancellation: Bool
        enum CodingKeys: String, CodingKey {
            case partID = "part_id", revision, taskID = "task_id", state
            case parentRunID = "parent_run_id"
            case supportsGuidance = "supports_guidance"
            case supportsCancellation = "supports_cancellation"
        }
    }

    enum Part: Decodable, Equatable, Sendable {
        /// Read-only journal detail; never a live execution or permission Part.
        case journalDetail(id: String, revision: UInt64, kind: String, title: String, content: String)
        case historical(id: String, revision: UInt64, projection: HistoricalProjection)
        case text(TextPart)
        case reasoning(ReasoningPart)
        case tool(ToolPart)
        case source(SourcePart)
        case citation(CitationPart)
        case attachment(AttachmentPart)
        case uiSurface(AppleUISurfacePart)
        case uiAction(AppleUIActionPart<AgentCoreJSONValue>)
        case taskControl(TaskControlPart)
        case task(id: String, revision: UInt64, taskID: String, displayKey: String, state: ActivityState, progress: Double?, currentStep: String?, resultRef: String?, cancelRequested: Bool)
        case skill(id: String, revision: UInt64, runID: String, skillID: String, displayName: String, mode: String, state: ActivityState, modelTurns: UInt32, toolCalls: UInt32, elapsedMillis: UInt64, resultRef: String?, failure: String?)
        case modelAttempt(ModelAttemptPart)
        case compaction(id: String, revision: UInt64, summary: String, retainedItemIDs: [String], sourceFingerprint: String, originalTokenCount: UInt64, compactedTokenCount: UInt64)
        case error(id: String, revision: UInt64, code: String, displayKey: String, recoverable: Bool)

        var partID: String {
            switch self {
            case .journalDetail(let id, _, _, _, _), .historical(let id, _, _), .task(let id, _, _, _, _, _, _, _, _),
                 .skill(let id, _, _, _, _, _, _, _, _, _, _, _),
                 .compaction(let id, _, _, _, _, _, _),
                 .error(let id, _, _, _, _):
                return id
            case .taskControl(let value): return value.partID
            case .modelAttempt(let value):
                return value.partID
            case .text(let value): return value.partID
            case .reasoning(let value): return value.partID
            case .tool(let value): return value.partID
            case .source(let value): return value.partID
            case .citation(let value): return value.partID
            case .attachment(let value): return value.partID
            case .uiSurface(let value): return value.partID
            case .uiAction(let value): return value.partID
            }
        }

        private enum CodingKeys: String, CodingKey {
            case projection, kind, revision, markdown, state, summary, title, uri, snippet, name, mode, progress, code, recoverable, input, output, failure
            case partID = "part_id"; case isFinal = "is_final"; case toolCallID = "tool_call_id"; case capabilityID = "capability_id"; case displayKey = "display_key"; case affectedResources = "affected_resources"; case sourceID = "source_id"; case mimeType = "mime_type"; case originToolCallID = "origin_tool_call_id"; case attachmentID = "attachment_id"; case taskID = "task_id"; case currentStep = "current_step"; case resultRef = "result_ref"; case cancelRequested = "cancel_requested"; case runID = "run_id"; case skillID = "skill_id"; case displayName = "display_name"; case modelTurns = "model_turns"; case toolCalls = "tool_calls"; case elapsedMillis = "elapsed_millis"; case retainedItemIDs = "retained_item_ids"; case sourceFingerprint = "source_fingerprint"; case originalTokenCount = "original_token_count"; case compactedTokenCount = "compacted_token_count"
        }

        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            let kind = try v.decode(String.self, forKey: .kind)
            let id = try v.decode(String.self, forKey: .partID)
            let revision = try v.decode(UInt64.self, forKey: .revision)
            switch kind {
            case "historical": self = .historical(id: id, revision: revision, projection: try v.decode(HistoricalProjection.self, forKey: .projection))
            case "text": self = .text(try TextPart(from: decoder))
            case "reasoning": self = .reasoning(try ReasoningPart(from: decoder))
            case "tool": self = .tool(try ToolPart(from: decoder))
            case "source": self = .source(try SourcePart(from: decoder))
            case "citation": self = .citation(try CitationPart(from: decoder))
            case "attachment": self = .attachment(try AttachmentPart(from: decoder))
            case "ui_surface": self = .uiSurface(try AppleUISurfacePart(from: decoder))
            case "ui_action": self = .uiAction(try AppleUIActionPart(from: decoder))
            case "task_control": self = .taskControl(try TaskControlPart(from: decoder))
            case "task": self = .task(id: id, revision: revision, taskID: try v.decode(String.self, forKey: .taskID), displayKey: try v.decode(String.self, forKey: .displayKey), state: try v.decode(ActivityState.self, forKey: .state), progress: try v.decodeIfPresent(Double.self, forKey: .progress), currentStep: try v.decodeIfPresent(String.self, forKey: .currentStep), resultRef: try v.decodeIfPresent(String.self, forKey: .resultRef), cancelRequested: try v.decode(Bool.self, forKey: .cancelRequested))
            case "skill": self = .skill(id: id, revision: revision, runID: try v.decode(String.self, forKey: .runID), skillID: try v.decode(String.self, forKey: .skillID), displayName: try v.decode(String.self, forKey: .displayName), mode: try v.decode(String.self, forKey: .mode), state: try v.decode(ActivityState.self, forKey: .state), modelTurns: try v.decode(UInt32.self, forKey: .modelTurns), toolCalls: try v.decode(UInt32.self, forKey: .toolCalls), elapsedMillis: try v.decode(UInt64.self, forKey: .elapsedMillis), resultRef: try v.decodeIfPresent(String.self, forKey: .resultRef), failure: try v.decodeIfPresent(String.self, forKey: .failure))
            case "model_attempt": self = .modelAttempt(try ModelAttemptPart(from: decoder))
            case "compaction": self = .compaction(id: id, revision: revision, summary: try v.decode(String.self, forKey: .summary), retainedItemIDs: try v.decode([String].self, forKey: .retainedItemIDs), sourceFingerprint: try v.decodeIfPresent(String.self, forKey: .sourceFingerprint) ?? "", originalTokenCount: try v.decode(UInt64.self, forKey: .originalTokenCount), compactedTokenCount: try v.decode(UInt64.self, forKey: .compactedTokenCount))
            case "error": self = .error(id: id, revision: revision, code: try v.decode(String.self, forKey: .code), displayKey: try v.decode(String.self, forKey: .displayKey), recoverable: try v.decode(Bool.self, forKey: .recoverable))
            default: throw DecodingError.dataCorruptedError(forKey: .kind, in: v, debugDescription: "Unsupported projection Part kind: \(kind)")
            }
        }
    }

    struct RunActivity: Decodable, Equatable, Sendable {
        let runID: String; let state: ActivityState; let activePartID: String?; let displayKey: String?; let progress: Double?
        private enum CodingKeys: String, CodingKey { case state, progress; case runID = "run_id"; case activePartID = "active_part_id"; case displayKey = "display_key" }
    }
    struct CacheUsage: Codable, Equatable, Sendable {
        let input_tokens: UInt64?
        let read_tokens: UInt64?
        let write_tokens: UInt64?
        let hit_rate_bps: UInt16?
        let is_compaction: Bool
    }

    struct ContextUsage: Decodable, Equatable, Sendable {
        var measurement: IOSContextMeasurement? = nil
        var promptCacheMeasurement: IOSContextMeasurement? = nil
        var promptCacheDiagnostic: AgentCorePromptCacheDiagnostic? = nil
        var cacheUsage: CacheUsage? = nil
        var conversationCacheUsage: IOSCommittedRunContextUsage.RunCacheUsage? = nil
        let usedTokens: UInt64; let windowTokens: UInt64; let promptTokenLimit: UInt64; let estimated: Bool; let measuredAtRevision: UInt64
        private enum CodingKeys: String, CodingKey {
            case measurement
            case promptCacheMeasurement = "prompt_cache_measurement"
            case promptCacheDiagnostic = "prompt_cache_diagnostic"
            case cacheUsage = "cache_usage"; case conversationCacheUsage = "conversation_cache_usage"; case estimated
            case usedTokens = "used_tokens"; case windowTokens = "context_window_tokens"
            case promptTokenLimit = "prompt_token_limit"; case measuredAtRevision = "measured_at_revision"
        }
    }

    struct Surface: Decodable, Equatable, Sendable {
        let revision: UInt64
        let committedMessageIDs: [String]
        let liveMessageID: String?
        let currentActivityPartID: String?
        let archivedActivityPartIDs: [String]
        let showAssistantPlaceholder: Bool

        private enum CodingKeys: String, CodingKey {
            case revision
            case committedMessageIDs = "committed_message_ids"
            case liveMessageID = "live_message_id"
            case currentActivityPartID = "current_activity_part_id"
            case archivedActivityPartIDs = "archived_activity_part_ids"
            case showAssistantPlaceholder = "show_assistant_placeholder"
        }

        static func derived(
            revision: UInt64,
            messages: [Message],
            activeRun: RunActivity?
        ) -> Surface {
            let live = messages.last { message in
                message.role == .assistant && (message.state == .pending || message.state == .running)
            }
            let archived = messages.flatMap(\.parts).compactMap { part -> String? in
                guard let state = part.activityState,
                      state == .completed || state == .sealed || state == .cancelled || state == .failed
                else { return nil }
                return part.partID
            }
            let hasVisibleText = live?.parts.contains { part in
                guard case .text(let value) = part, value.isDisclosed else { return false }
                return !value.markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            } ?? false
            return Surface(
                revision: revision,
                committedMessageIDs: messages
                    .filter { $0.messageID != live?.messageID }
                    .map(\.messageID),
                liveMessageID: live?.messageID,
                currentActivityPartID: activeRun?.activePartID,
                archivedActivityPartIDs: archived,
                showAssistantPlaceholder: live != nil && !hasVisibleText
            )
        }
    }

    private enum CodingKeys: String, CodingKey { case revision, title, messages, surface; case contractVersion = "contract_version"; case conversationID = "conversation_id"; case executionOwnership = "execution_ownership"; case activeRun = "active_run"; case contextUsage = "context_usage" }
    init(from decoder: Decoder) throws {
        let v = try decoder.container(keyedBy: CodingKeys.self)
        contractVersion = try v.decode(UInt32.self, forKey: .contractVersion)
        guard contractVersion == 1 else { throw DecodingError.dataCorruptedError(forKey: .contractVersion, in: v, debugDescription: "Unsupported conversation projection contract") }
        conversationID = try v.decode(String.self, forKey: .conversationID)
        executionOwnership = try v.decode(AppleConversationExecutionOwnership.self, forKey: .executionOwnership)
        revision = try v.decode(UInt64.self, forKey: .revision)
        title = try v.decode(String.self, forKey: .title)
        messages = try v.decode([Message].self, forKey: .messages)
        activeRun = try v.decodeIfPresent(RunActivity.self, forKey: .activeRun)
        contextUsage = try v.decode(ContextUsage.self, forKey: .contextUsage)
        surface = try v.decodeIfPresent(Surface.self, forKey: .surface)
            ?? Surface.derived(revision: revision, messages: messages, activeRun: activeRun)
    }
}

private extension IOSConversationProjectionV1.Part {
    nonisolated var activityState: IOSConversationProjectionV1.ActivityState? {
        switch self {
        case .reasoning(let value): return value.state
        case .tool(let value): return value.state
        case .taskControl(let value): return value.state
        case .task(_, _, _, _, let state, _, _, _, _),
             .skill(_, _, _, _, _, _, let state, _, _, _, _, _):
            return state
        case .modelAttempt(let value):
            return value.state
        default:
            return nil
        }
    }
}

nonisolated protocol ConversationProjectionStore: AnyObject {
    func loadConversationProjection(conversationId: UUID) throws -> IOSConversationProjectionV1
    func loadConversationProjectionWindow(
        conversationId: UUID,
        beforeOrdinal: Int64?,
        limit: Int,
        contentByteLimit: Int
    ) throws -> IOSConversationProjectionWindowV1
}

nonisolated struct IOSConversationProjectionWindowV1: Decodable, Equatable, Sendable {
    let projection: IOSConversationProjectionV1
    let eventSequence: UInt64
    let totalMessageCount: UInt64
    let beforeOrdinal: Int64?
    let hasMore: Bool
    let assistantJournalHeads: [IOSAssistantJournalCursor]

    init(projection: IOSConversationProjectionV1, eventSequence: UInt64, totalMessageCount: UInt64,
         beforeOrdinal: Int64?, hasMore: Bool, assistantJournalHeads: [IOSAssistantJournalCursor] = []) {
        self.projection = projection; self.eventSequence = eventSequence
        self.totalMessageCount = totalMessageCount; self.beforeOrdinal = beforeOrdinal
        self.hasMore = hasMore; self.assistantJournalHeads = assistantJournalHeads
    }

    private enum CodingKeys: String, CodingKey {
        case projection
        case eventSequence = "event_sequence"
        case totalMessageCount = "total_message_count"
        case beforeOrdinal = "before_ordinal"
        case hasMore = "has_more"
        case assistantJournalHeads = "assistant_journal_heads"
    }
}

extension IOSConversationProjectionV1.StructuredValue {
    nonisolated var displayText: String {
        switch self {
        case .null: return "null"
        case .bool(let value): return value ? "true" : "false"
        case .signedInteger(let value): return String(value)
        case .unsignedInteger(let value): return String(value)
        case .number(let value):
            // Manual Double values keep the established small-number display,
            // without a trapping cast at the signed integer boundary.
            if let integer = Int64(exactly: value) { return String(integer) }
            return String(value)
        case .string(let value): return value
        case .array(let values): return "[" + values.map(\.displayText).joined(separator: ", ") + "]"
        case .object(let values):
            return "{" + values.keys.sorted().map { "\($0): \(values[$0]?.displayText ?? "null")" }.joined(separator: ", ") + "}"
        }
    }
}
