// SPDX-License-Identifier: Apache-2.0
// Public presentation facts only. References are resolved through the owning
// committed Part; neither permission display nor a URI authorizes execution.
import Foundation

/// Unchanged content is reusable across message cursors, but never across
/// databases, owners, Parts or content generations.
nonisolated struct MacJournalFieldReadKey: Hashable {
    let databaseURL: URL
    let identity: MacAssistantJournalIdentity
    let partID: String
    let content: MacAssistantJournalContentReference?
}

nonisolated enum MacJournalArtifactDestination: Equatable {
    case web(URL), localFile(URL)

    static func resolve(_ uri: String, localJournal: Bool) -> Self? {
        guard let url = URL(string: uri), let scheme = url.scheme?.lowercased() else { return nil }
        if ["https", "http"].contains(scheme), let host = url.host, !host.isEmpty {
            return .web(url)
        }
        if localJournal, url.isFileURL, url.path.hasPrefix("/"),
           url.host == nil || url.host == "" || url.host == "localhost" {
            return .localFile(url)
        }
        return nil
    }
}

nonisolated enum MacJournalBodyPresentation: Hashable, Sendable {
    case markdown, plain, structured(String)

    static func toolField(_ field: String) -> Self {
        switch field {
        case "tool_input", "tool_output": return .structured("value")
        case "tool_artifacts": return .structured("artifacts")
        case "tool_resources": return .structured("resources")
        default: return .plain
        }
    }

    func display(_ text: String, complete: Bool) throws -> String {
        guard case .structured(let key) = self else { return text }
        guard complete else { return "" }
        let decoded = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(text.utf8))
        guard case .object(let envelope) = decoded,
              Set(envelope.keys) == Set(["protocol_version", key]),
              envelope["protocol_version"] == .number(1), let value = envelope[key] else {
            throw MacRemoteHostError.invalidResponse
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return String(decoding: try encoder.encode(value), as: UTF8.self)
    }
}

/// View-local handover evidence, never execution or permission authority.
nonisolated struct MacJournalActivityCoverage: Equatable {
    private var identity: MacAssistantJournalIdentity?
    private var partIDs = Set<String>()
    private var taskIDs = Set<String>()

    mutating func adopt(identity: MacAssistantJournalIdentity, partIDs: Set<String>, taskIDs: Set<String> = []) {
        if self.identity != identity {
            self.identity = identity
            self.partIDs = partIDs
            self.taskIDs = taskIDs
        } else {
            // A removed Part must not return through an older trace snapshot.
            self.partIDs.formUnion(partIDs)
            self.taskIDs.formUnion(taskIDs)
        }
    }

    func contains(identity: MacAssistantJournalIdentity, partID: String) -> Bool {
        self.identity == identity && partIDs.contains(partID)
    }

    func containsTask(identity: MacAssistantJournalIdentity, taskID: String) -> Bool {
        self.identity == identity && taskIDs.contains(taskID)
    }
}

/// Source and citation IDs are independent namespaces, not trace Part IDs.
nonisolated struct MacJournalReferenceCoverage: Equatable {
    private var identity: MacAssistantJournalIdentity?
    private var sources = Set<String>()
    private var citations = Set<String>()
    private var attachments = Set<String>()

    mutating func adopt(identity: MacAssistantJournalIdentity, sourceIDs: Set<String>, citationIDs: Set<String>, attachmentIDs: Set<String> = []) {
        if self.identity != identity {
            self.identity = identity
            sources = sourceIDs
            citations = citationIDs
            attachments = attachmentIDs
        } else {
            sources.formUnion(sourceIDs)
            citations.formUnion(citationIDs)
            attachments.formUnion(attachmentIDs)
        }
    }

    func contains(identity: MacAssistantJournalIdentity, sourceID: String, citationID: String?) -> Bool {
        guard self.identity == identity else { return false }
        if let citationID { return citations.contains(citationID) }
        return sources.contains(sourceID)
    }
    func containsAttachment(identity: MacAssistantJournalIdentity, attachmentID: String) -> Bool {
        self.identity == identity && attachments.contains(attachmentID)
    }
}

nonisolated enum MacJournalText: Decodable, Sendable {
    case inline(String)
    case field(String)
    private enum Keys: String, CodingKey { case kind, value }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        let value = try values.decode(String.self, forKey: .value)
        switch try values.decode(String.self, forKey: .kind) {
        case "inline": self = .inline(value)
        case "field": self = .field(value)
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown journal text representation")
        }
    }
}

nonisolated enum MacJournalResources: Decodable, Sendable {
    struct Resource: Decodable, Sendable {
        let resource_type: String
        let uri: String
        let operation: String
    }
    case inline([Resource])
    case field(String)
    private enum Keys: String, CodingKey { case kind, value }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "inline": self = .inline(try values.decode([Resource].self, forKey: .value))
        case "field": self = .field(try values.decode(String.self, forKey: .value))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown journal resource representation")
        }
    }
}

nonisolated enum MacAssistantJournalMetadata: Decodable, Sendable {
    struct Text: Decodable, Sendable {
        let is_final: Bool
        let visibility: String
        let review: String?
    }
    struct Activity: Decodable, Sendable {
        let title: MacJournalText
        let state: String
        var isRunning: Bool { state == "running" }
    }
    struct Origin: Decodable, Sendable {
        let kind: String
        let provider_id: MacJournalText?
        let provider_title: MacJournalText?
        let namespace: MacJournalText?
    }
    struct Permission: Decodable, Sendable {
        let request_id: String
        let permission_domain: MacJournalText
        let rationale: MacJournalText
        let decision: MacJournalText?
    }
    struct FullOutput: Decodable, Sendable {
        let message_id: String
        let part_id: String
        let tool_call_id: String
    }
    struct Tool: Decodable, Sendable {
        let execution_source: MacCanonicalConversationProjectionV1.ToolExecutionSource
        let tool_call_id: String
        let capability_id: String
        let title: MacJournalText
        let summary: MacJournalText?
        let origin: Origin
        let permission: Permission?
        let state: String
        let display_key: String?
        let affected_resources: MacJournalResources
        let artifacts: String
        let full_output: FullOutput?
        let started_at_ms: String?
        let finished_at_ms: String?
    }
    struct Source: Decodable, Sendable {
        let source_id: String
        let title: MacJournalText
        let uri: MacJournalText
        let mime_type: MacJournalText?
        let evidence: String?
    }
    struct Location: Decodable, Sendable {
        let page: String?
        let line_start: String?
        let line_end: String?
        let section: MacJournalText?
    }
    struct Range: Decodable, Sendable {
        let start: String
        let end: String
    }
    struct Citation: Decodable, Sendable {
        let citation_id: String
        let source_id: String
        let title: MacJournalText
        let uri: MacJournalText
        let mime_type: MacJournalText?
        let origin_tool_call_id: String
        let location: Location?
        let byte_range: Range?
    }
    struct Attachment: Decodable, Sendable {
        let attachment_id: String
        let name: MacJournalText
        let mime_type: MacJournalText?
        let uri: MacJournalText
        let operation: MacJournalText?
    }
    struct Progress: Decodable, Sendable {
        let fraction_bits: String
        var fraction: Double? {
            guard let bits = UInt64(fraction_bits) else { return nil }
            let result = Double(bitPattern: bits)
            return result.isFinite && (0...1).contains(result) ? result : nil
        }
    }
    struct Task: Decodable, Sendable {
        let task_id: String
        let display_key: String
        let state: String
        let progress: Progress?
        let cancel_requested: Bool
        var supportsGuidance = false
        var supportsCancellation = false
        private enum CodingKeys: String, CodingKey {
            case task_id, display_key, state, progress, cancel_requested
        }
        var isTerminal: Bool { ["completed", "sealed", "failed", "cancelled"].contains(state) }
        // Unknown states are displayable but cannot enable interaction. This
        // is a presentation gate, not execution/permission authorization.
        var acceptsInteraction: Bool {
            !cancel_requested && ["pending", "running", "waiting_for_permission", "waiting_for_input"].contains(state)
        }
    }
    struct TaskControl: Decodable, Sendable {
        var task: Task
        let supports_guidance: Bool
        let supports_cancellation: Bool
    }
    struct Skill: Decodable, Sendable {
        let run_id: String
        let skill_id: String
        let display_name: MacJournalText
        let mode: String
        let state: String
        let model_turns: String
        let tool_calls: String
        let elapsed_millis: String
    }
    struct ModelAttempt: Decodable, Sendable {
        let attempt_id: String
        let state: String
        let provider: String?
        let model: String?
        let failure_code: String
        let retryable: Bool
        let http_status: String?
        let fallback_attempt_id: String?
        let fallback_provider: String?
        let fallback_model: String?
    }
    struct Failure: Decodable, Sendable {
        let code: String
        let display_key: String
        let recoverable: Bool
    }
    struct Historical: Decodable, Sendable {
        let schema: String
        let canonical_kind: String
        let canonical_revision: String
    }

    struct UISurface: Decodable, Sendable {
        let surface: AppleUISurfaceReference
        let summary: String
    }
    case uiSurface(UISurface)
    case text(Text), reasoning(Activity), step(Activity), tool(Tool)
    case source(Source), citation(Citation), attachment(Attachment)
    case task(Task), skill(Skill), modelAttempt(ModelAttempt)
    case error(Failure), historical(Historical)

    private enum Keys: String, CodingKey { case kind, data }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "ui_surface": self = .uiSurface(try values.decode(UISurface.self, forKey: .data))
        case "text": self = .text(try values.decode(Text.self, forKey: .data))
        case "reasoning": self = .reasoning(try values.decode(Activity.self, forKey: .data))
        case "step": self = .step(try values.decode(Activity.self, forKey: .data))
        case "tool": self = .tool(try values.decode(Tool.self, forKey: .data))
        case "source": self = .source(try values.decode(Source.self, forKey: .data))
        case "citation": self = .citation(try values.decode(Citation.self, forKey: .data))
        case "attachment": self = .attachment(try values.decode(Attachment.self, forKey: .data))
        case "task": self = .task(try values.decode(Task.self, forKey: .data))
        case "task_control":
            let control = try values.decode(TaskControl.self, forKey: .data)
            var task = control.task
            task.supportsGuidance = control.supports_guidance
            task.supportsCancellation = control.supports_cancellation
            self = .task(task)
        case "skill": self = .skill(try values.decode(Skill.self, forKey: .data))
        case "model_attempt": self = .modelAttempt(try values.decode(ModelAttempt.self, forKey: .data))
        case "error": self = .error(try values.decode(Failure.self, forKey: .data))
        case "historical": self = .historical(try values.decode(Historical.self, forKey: .data))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown journal Part kind")
        }
    }
}
