// SPDX-License-Identifier: Apache-2.0
// Read-only attachment to a paired Host Run. No input or execution is created.
import Foundation

nonisolated enum IOSRunObservationTransportError: LocalizedError {
    case http(Int)
    var errorDescription: String? {
        switch self {
        case .http(let status):
            return String(format: NSLocalizedString("chat.host.observation_unavailable",
                value: "Unable to confirm the existing Host task (HTTP %d). Reconnect before sending another message.",
                comment: "Unknown Host observation must not become a new submission"), status)
        }
    }
}

nonisolated struct IOSRunObservationRequest: Encodable, Sendable {
    let operation: String
    let conversation_id: String
    let expected_host_id: String
    let client_id: String
    let expected_run_id: String?
    let assistant_message_id: String?

    init(conversation: String, host: String, client: String, run: IOSRunObservation? = nil,
         expectedRunID: String? = nil, assistantID: String? = nil) {
        operation = run == nil && expectedRunID == nil ? "discover" : "observe"
        conversation_id = conversation; expected_host_id = host; client_id = client
        expected_run_id = run?.run_id ?? expectedRunID
        assistant_message_id = run?.assistant_message_id ?? assistantID
    }
}

nonisolated struct IOSRunObservationEnvelope: Decodable, Sendable {
    let source_host_id: String
    let conversation_id: String
    let client_id: String
    let result: IOSRunObservation?
    enum CodingKeys: String, CodingKey { case source_host_id, conversation_id, client_id, result }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        source_host_id = try fields.decode(String.self, forKey: .source_host_id)
        conversation_id = try fields.decode(String.self, forKey: .conversation_id)
        client_id = try fields.decode(String.self, forKey: .client_id)
        guard fields.contains(.result) else { throw IOSApplicationRuntimeError.invalidResponse }
        result = try fields.decodeIfPresent(IOSRunObservation.self, forKey: .result)
    }

    func validated(for request: IOSRunObservationRequest) throws -> IOSRunObservation? {
        guard source_host_id == request.expected_host_id, conversation_id == request.conversation_id,
              client_id == request.client_id else { throw IOSApplicationRuntimeError.invalidResponse }
        if result == nil, request.expected_run_id != nil { throw IOSApplicationRuntimeError.invalidResponse }
        return result
    }
}

/// Bounded committed control data, not a permission or execution capability.
/// Core verifies its digest and admitted source provenance before persistence.
nonisolated struct IOSContextMeasurement: Codable, Equatable, Sendable {
    let request_id: String
    let attempt_id: String?
    let context_generation: UInt64?

    var isValid: Bool {
        !request_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (attempt_id.map { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } ?? true)
    }
}

nonisolated struct IOSRemotePermissionConfiguration: Decodable, Sendable {
    let conversation_id: String
    let configuration_revision: Int64
    let permission_profile: IOSPermissionMode?
    let active_run_id: String?
    let active_policy: AgentActivePermissionPolicy?
    enum ApplicationState: String, Decodable, Sendable { case inactive, pending, active }
    struct Owner: Decodable, Sendable {
        enum Kind: String, Decodable, Sendable { case inactive, local, remote }
        let kind: Kind
        let run_id: String?
        let assistant_message_id: String?
        let identity: IOSRemoteSubmissionIdentity?
    }
    let owner: Owner
    let state: ApplicationState

    func validate(conversationID: String, runID: String) throws {
        switch owner.kind {
        case .inactive:
            guard active_run_id == nil, state == .inactive else { throw IOSApplicationRuntimeError.rejected("Permission configuration owner is invalid") }
        case .local:
            guard owner.run_id == runID, owner.run_id == active_run_id,
                  owner.assistant_message_id?.isEmpty == false else { throw IOSApplicationRuntimeError.rejected("Permission configuration owner is invalid") }
        case .remote:
            guard let source = owner.identity, source.conversation_id == conversationID,
                  source.submission_id == runID, source.submission_id == active_run_id else { throw IOSApplicationRuntimeError.rejected("Permission configuration owner is invalid") }
        }
        guard conversation_id == conversationID, configuration_revision > 0,
              active_run_id == nil || active_run_id == runID,
              active_policy.map({ $0.runID == runID && $0.runID == active_run_id && $0.configurationRevision > 0
                  && $0.configurationRevision <= configuration_revision }) ?? true else {
            throw IOSApplicationRuntimeError.rejected("Permission configuration belongs to another Host Run")
        }
    }
}

nonisolated struct IOSCommittedRunContextUsage: Codable, Equatable, Sendable {
    struct RunCacheUsage: Codable, Equatable, Sendable {
        let input_tokens: UInt64?
        let read_tokens: UInt64?
        let write_tokens: UInt64?
        let hit_rate_bps: UInt16?
        let is_compaction: Bool
        let reported_requests: UInt64
        let unreported_requests: UInt64
    }
    struct Usage: Codable, Equatable, Sendable {
        var measurement: IOSContextMeasurement? = nil
        var prompt_cache_measurement: IOSContextMeasurement? = nil
        var prompt_cache_diagnostic: AgentCorePromptCacheDiagnostic? = nil
        var cache_usage: IOSConversationProjectionV1.CacheUsage? = nil
        let used_tokens: String
        let context_window_tokens: String
        let prompt_token_limit: String
        let estimated: Bool
    }
    let protocol_version: Int
    let identity: IOSAssistantJournalIdentity
    let admission: IOSRunMessageAdmissionCursor
    let event_sequence: String
    let attempt_sequence: String
    let run_cache_usage: RunCacheUsage?
    let usage: Usage
    let record_digest: String

    func validate(owner: IOSAssistantJournalIdentity) throws {
        func decimal(_ value: String, positive: Bool = false) -> Bool {
            guard let parsed = UInt64(value), String(parsed) == value else { return false }
            return !positive || parsed > 0
        }
        guard protocol_version == 1, identity == owner, admission.identity == owner,
              usage.measurement?.isValid ?? true, usage.prompt_cache_measurement?.isValid ?? true,
              usage.prompt_cache_diagnostic == nil || usage.prompt_cache_measurement != nil,
              decimal(attempt_sequence),
              decimal(event_sequence, positive: true), decimal(admission.sequence, positive: true),
              decimal(usage.used_tokens), decimal(usage.context_window_tokens), decimal(usage.prompt_token_limit),
              record_digest.utf8.count == 64,
              record_digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw IOSApplicationRuntimeError.rejected("Committed context usage owner or values differ")
        }
    }
}

nonisolated struct IOSRunObservation: Decodable, Sendable {
    struct Worker: Decodable, Sendable { let state: String; let generation: String? }
    let protocol_version: Int
    let conversation_id: String
    let client_id: String
    let run_id: String
    let assistant_message_id: String
    let status: String
    let worker: Worker
    let recovery: String
    let admission: IOSRunMessageAdmissionCursor
    let terminal_seal: IOSAssistantRunTerminalSeal?
    let permissions: [AgentCoreJSONValue]
    let can_submit: Bool
    var submission_receipt: IOSSourceSubmissionReceipt? = nil
    var context_usage: IOSCommittedRunContextUsage? = nil

    func validate(conversation: String, client: String, previous: IOSRunObservation?) throws {
        guard protocol_version == 1, conversation_id == conversation, client_id == client,
              !run_id.isEmpty, !assistant_message_id.isEmpty,
              admission.identity.conversation_id == conversation, admission.identity.run_id == run_id,
              admission.identity.assistant_message_id == assistant_message_id,
              UInt64(admission.sequence).map({ String($0) == admission.sequence }) == true,
              worker.generation == nil || UInt64(worker.generation!).map({ String($0) == worker.generation! }) == true,
              ["running", "sealed"].contains(status),
              ["running", "starting", "absent", "unknown"].contains(worker.state),
              ["observe", "resume_required", "terminal", "inconsistent"].contains(recovery),
              previous == nil || (previous?.run_id == run_id && previous?.assistant_message_id == assistant_message_id),
              permissions.count <= 16 else {
            throw IOSApplicationRuntimeError.rejected("Run observation identity or protocol mismatch")
        }
        if let seal = terminal_seal {
            guard recovery == "terminal", seal.identity == admission.identity,
                  seal.protocol_version == 1, status == "sealed",
                  seal.result.run_id == run_id,
                  seal.result.conversation_id == conversation_id else {
                throw IOSApplicationRuntimeError.rejected("Run observation terminal identity mismatch")
            }
        } else if recovery == "terminal" || status != "running" || can_submit {
            throw IOSApplicationRuntimeError.rejected("Run observation has no terminal authority")
        }
        try context_usage?.validate(owner: admission.identity)
        if recovery == "resume_required", worker.state != "absent" {
            throw IOSApplicationRuntimeError.rejected("A live worker cannot be resumed")
        }
        if worker.state == "absent", worker.generation != nil {
            throw IOSApplicationRuntimeError.rejected("Absent worker has an execution generation")
        }
    }

    @MainActor func permissionRequests(host: String, client: String, now: Int64) throws -> [HostAgentPermissionRequest] {
        try permissions.map { raw in
            let data = try JSONEncoder().encode(raw)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  object["canonical_run_id"] as? String == run_id,
                  object["session_id"] as? String == conversation_id,
                  object["host_id"] as? String == host, object["client_id"] as? String == client,
                  let requestID = object["id"] as? String, !requestID.isEmpty,
                  let request = WebAgentProvider.permissionRequest(from: object),
                  request.protocolVersion == WebAgentProvider.hostPermissionProtocolVersion,
                  request.nonce?.isEmpty == false, let expiry = request.expiresAt, expiry >= now,
                  worker.state == "running", recovery == "observe", terminal_seal == nil else {
                throw IOSApplicationRuntimeError.rejected("Observed permission is not owned by this live Run")
            }
            return request
        }
    }
}

/// One cancellable observer per selected conversation/lifecycle generation.
/// Repeated successful reads are bounded pages; errors have a finite retry budget.
@MainActor final class IOSRemoteRunObserver {
    private var task: Task<Void, Never>?
    private var generation: UInt64 = 0

    func cancel() { generation &+= 1; task?.cancel(); task = nil }

    func start(
        initial: IOSRunObservation? = nil,
        read: @escaping @MainActor (IOSRunObservation?) async throws -> IOSRunObservation?,
        receive: @escaping @MainActor (IOSRunObservation) async throws -> Void,
        idle: @escaping @MainActor () -> Void,
        failed: @escaping @MainActor (Error) -> Void,
        sleep: @escaping @Sendable () async throws -> Void = { try await Task.sleep(for: .seconds(1)) }
    ) {
        cancel()
        let fence = generation
        task = Task { [weak self] in
            var current = initial
            var failures = 0
            while !Task.isCancelled {
                guard let self, self.generation == fence else { return }
                do {
                    let observed = try await read(current)
                    try Task.checkCancellation()
                    guard self.generation == fence else { return }
                    guard let observed else {
                        guard current == nil else { throw IOSApplicationRuntimeError.rejected("Observed Run disappeared") }
                        idle(); return
                    }
                    if observed.recovery == "inconsistent" { throw IOSApplicationRuntimeError.rejected("Host Run owner is inconsistent") }
                    try await receive(observed)
                    try Task.checkCancellation()
                    guard self.generation == fence else { return }
                    current = observed; failures = 0
                    if observed.recovery == "terminal" || observed.recovery == "resume_required" { return }
                    try await sleep()
                } catch is CancellationError { return }
                catch {
                    guard self.generation == fence, !Task.isCancelled else { return }
                    failures += 1
                    if failures >= 3 { failed(error); return }
                    do { try await sleep() } catch { return }
                }
            }
        }
    }
}

/// A maintenance receipt from the authoritative Host, never a local archive.
nonisolated struct IOSRemoteContextCompactionReceipt: Decodable, Sendable {
    struct Context: Decodable, Sendable {
        let used_tokens: UInt64
        let context_window_tokens: UInt64
        let usage_percent: Double
        let replaced_item_count: UInt64
        let remaining_item_count: UInt64
    }
    let conversation_id: String
    let compacted: Bool
    let checkpoint_revision: UInt64
    let committed_at_ms: Int64
    let context: Context

    func validate(conversationID: String) throws {
        guard conversation_id == conversationID, checkpoint_revision > 0, committed_at_ms > 0,
              compacted == (context.replaced_item_count > 0) else {
            throw IOSApplicationRuntimeError.rejected("Host context maintenance receipt changed owner or checkpoint")
        }
    }
}
