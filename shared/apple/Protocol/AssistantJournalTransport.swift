// SPDX-License-Identifier: Apache-2.0
// Current Apple wire contract. Core remains the authority for evidence acceptance.
import Foundation

nonisolated public protocol JournalJSONValue: Codable, Equatable, Sendable {
    var journalObject: [String: Self]? { get }
    var journalArray: [Self]? { get }
    var journalString: String? { get }
    var journalIsNull: Bool { get }
}

nonisolated struct AppleAssistantJournalIdentity: Codable, Hashable, Sendable {
    let source_store_epoch: String
    let conversation_id: String
    let run_id: String
    let assistant_message_id: String
}

nonisolated struct AppleAssistantJournalCursor: Codable, Hashable, Sendable {
    let identity: AppleAssistantJournalIdentity
    let sequence: String
    let record_digest: String

    // A cached cursor is progress evidence, never authority to keep updating a
    // historical Run after this view has attached to a different execution.
    func advances(_ previous: Self?, conversationID: String, observedRunID: String) -> Bool {
        guard identity.conversation_id == conversationID,
              identity.run_id == observedRunID,
              let sequence = UInt64(sequence) else { return false }
        guard let previous, previous.identity == identity else { return true }
        guard let priorSequence = UInt64(previous.sequence) else { return false }
        return sequence > priorSequence
    }
}

nonisolated struct AppleAssistantJournalNotification: Decodable, Sendable {
    let session_id: String
    let source_host_id: String
    let through: AppleAssistantJournalCursor
}

nonisolated struct AppleRunMessageAdmissionCursor: Codable, Equatable, Sendable {
    let identity: AppleAssistantJournalIdentity
    let sequence: String
    let record_digest: String
}

nonisolated struct AppleRunMessageAdmissionPage<JSON: JournalJSONValue>: Codable, Sendable {
    let after: AppleRunMessageAdmissionCursor
    let through: AppleRunMessageAdmissionCursor
    let next: AppleRunMessageAdmissionCursor
    let complete: Bool
    let records: [JSON]
}

nonisolated public struct AppleRunResult<JSON: JournalJSONValue>: Codable, Equatable, Sendable {
    struct Input: Codable, Equatable, Sendable {
        let watermark: String
        let applications_digest: String
    }
    struct Value: Codable, Equatable, Sendable {
        enum Kind: String, Codable, Sendable {
            case answered
            case actionCompleted = "action_completed"
            case delegationAccepted = "delegation_accepted"
            case incomplete, blocked, cancelled, failed
        }
        let kind: Kind
        let payload: JSON
        private enum CodingKeys: String, CodingKey { case kind }
        init(from decoder: Decoder) throws {
            kind = try decoder.container(keyedBy: CodingKeys.self).decode(Kind.self, forKey: .kind)
            payload = try JSON(from: decoder)
        }
        func encode(to encoder: Encoder) throws { try payload.encode(to: encoder) }
        // Presence/shape only: receipt authenticity and evidence acceptance stay in Rust.
        var hasRequiredEvidenceFields: Bool {
            guard let fields = payload.journalObject else { return false }
            func text(_ key: String) -> Bool {
                guard let value = fields[key]?.journalString else { return false }
                return !value.isEmpty && value == value.trimmingCharacters(in: .whitespacesAndNewlines)
                    && value.rangeOfCharacter(from: .controlCharacters) == nil
            }
            func refs(_ key: String, nonempty: Bool = false) -> Bool {
                guard let values = fields[key]?.journalArray, !nonempty || !values.isEmpty else { return false }
                var seen = Set<String>()
                return values.allSatisfy {
                    guard let value = $0.journalString,
                          !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines),
                          value.rangeOfCharacter(from: .controlCharacters) == nil else { return false }
                    return seen.insert(value).inserted
                }
            }
            func reason() -> Bool {
                guard let value = fields["reason"]?.journalObject,
                      let code = value["code"]?.journalString,
                      code.range(of: "^[a-z0-9_.]+$", options: .regularExpression) != nil else { return false }
                return value["detail"]?.journalIsNull == true || {
                    if value["detail"]?.journalString != nil { return true }; return false
                }()
            }
            func effects() -> Bool {
                guard refs("observed_effect_refs"), refs("unknown_effect_refs"),
                      let observed = fields["observed_effect_refs"]?.journalArray,
                      let unknown = fields["unknown_effect_refs"]?.journalArray else { return false }
                return !unknown.contains { observed.contains($0) }
            }
            switch kind {
            case .answered: return refs("answer_part_refs", nonempty: true)
            case .actionCompleted: return refs("requirement_acceptance_refs", nonempty: true)
            case .delegationAccepted: return text("task_id") && text("parent_call_id")
            case .incomplete: return reason() && refs("remaining_requirement_refs") && refs("obligation_refs")
                && (refs("remaining_requirement_refs", nonempty: true) || refs("obligation_refs", nonempty: true))
            case .blocked:
                guard let condition = fields["resolution_condition"]?.journalString else { return false }
                return reason() && refs("blocking_fact_refs", nonempty: true)
                    && !condition.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            case .cancelled: return text("cancellation_id") && effects()
            case .failed: return text("error_ref") && effects()
            }
        }
    }
    let result_id: String
    let run_id: String
    let conversation_id: String
    let originating_submission_id: String
    let input: Input
    let decided_at_revision: String
    let decision_event_cursor: String
    let requirement_revision: String
    let evidence_frontier: String
    let value: Value

    private enum CodingKeys: String, CodingKey {
        case result_id, run_id, conversation_id, originating_submission_id, input
        case decided_at_revision, decision_event_cursor, requirement_revision, evidence_frontier, value
    }
    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        result_id = try fields.decode(String.self, forKey: .result_id)
        run_id = try fields.decode(String.self, forKey: .run_id)
        conversation_id = try fields.decode(String.self, forKey: .conversation_id)
        originating_submission_id = try fields.decode(String.self, forKey: .originating_submission_id)
        input = try fields.decode(Input.self, forKey: .input)
        decided_at_revision = try fields.decode(String.self, forKey: .decided_at_revision)
        decision_event_cursor = try fields.decode(String.self, forKey: .decision_event_cursor)
        requirement_revision = try fields.decode(String.self, forKey: .requirement_revision)
        evidence_frontier = try fields.decode(String.self, forKey: .evidence_frontier)
        value = try fields.decode(Value.self, forKey: .value)
        let identities = [result_id, run_id, conversation_id, originating_submission_id]
        let counters = [input.watermark, decided_at_revision, decision_event_cursor, requirement_revision, evidence_frontier]
        guard identities.allSatisfy({ !$0.isEmpty && $0 == $0.trimmingCharacters(in: .whitespacesAndNewlines)
                && $0.rangeOfCharacter(from: .controlCharacters) == nil }),
              counters.allSatisfy({ raw in
                  guard let number = Int64(raw), number >= 0 else { return false }
                  return String(number) == raw
              }),
              input.applications_digest.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              (input.watermark == "0") == (input.applications_digest == "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"),
              value.hasRequiredEvidenceFields else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Run result has invalid identities, durable counters, or evidence fields"))
        }
    }
}

nonisolated struct AppleAssistantRunTerminalSeal<JSON: JournalJSONValue>: Codable, Equatable, Sendable {
    let protocol_version: UInt16
    let admission: AppleRunMessageAdmissionCursor
    let identity: AppleAssistantJournalIdentity
    let core_session_id: String
    let transition_id: String
    let checkpoint_revision: String
    let core_revision: String
    let event_cursor: String
    let result: AppleRunResult<JSON>
    let result_digest: String
    let completed_at_ms: Int64
    let entry_count: String
    let heads_count: String
    let child_receipt_count: String
    let manifest_digest: String
    let seal_digest: String
}

nonisolated enum AppleAssistantRunTerminalManifestEntry: Codable, Sendable {
    case assistant(through: AppleAssistantJournalCursor, ordinal: Int64, createdAtMs: Int64,
                   state: String, completedAtMs: Int64)
    case childReceipt(taskID: String, receiptID: String, parentTurnID: String,
                      childSessionID: String?, childRunID: String?, inputWatermark: Int64?,
                      completionWatermark: Int64?, status: String, receiptDigest: String, resultDigest: String)

    private enum CodingKeys: String, CodingKey {
        case kind, through, ordinal, created_at_ms, state, completed_at_ms, task_id, receipt_id, parent_turn_id
        case child_session_id, child_run_id, input_watermark, completion_watermark, status, receipt_digest, result_digest
    }
    init(from decoder: Decoder) throws {
        let value = try decoder.container(keyedBy: CodingKeys.self)
        switch try value.decode(String.self, forKey: .kind) {
        case "assistant":
            self = .assistant(through: try value.decode(AppleAssistantJournalCursor.self, forKey: .through),
                ordinal: try value.decode(Int64.self, forKey: .ordinal),
                createdAtMs: try value.decode(Int64.self, forKey: .created_at_ms),
                state: try value.decode(String.self, forKey: .state),
                completedAtMs: try value.decode(Int64.self, forKey: .completed_at_ms))
        case "child_receipt":
            self = .childReceipt(taskID: try value.decode(String.self, forKey: .task_id),
                receiptID: try value.decode(String.self, forKey: .receipt_id),
                parentTurnID: try value.decode(String.self, forKey: .parent_turn_id),
                childSessionID: try value.decodeIfPresent(String.self, forKey: .child_session_id),
                childRunID: try value.decodeIfPresent(String.self, forKey: .child_run_id),
                inputWatermark: try value.decodeIfPresent(Int64.self, forKey: .input_watermark),
                completionWatermark: try value.decodeIfPresent(Int64.self, forKey: .completion_watermark),
                status: try value.decode(String.self, forKey: .status),
                receiptDigest: try value.decode(String.self, forKey: .receipt_digest),
                resultDigest: try value.decode(String.self, forKey: .result_digest))
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: value,
                debugDescription: "Unknown terminal manifest entry")
        }
    }
    func encode(to encoder: Encoder) throws {
        var value = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .assistant(let through, let ordinal, let createdAtMs, let state, let completedAtMs):
            try value.encode("assistant", forKey: .kind)
            try value.encode(through, forKey: .through)
            try value.encode(ordinal, forKey: .ordinal)
            try value.encode(createdAtMs, forKey: .created_at_ms)
            try value.encode(state, forKey: .state)
            try value.encode(completedAtMs, forKey: .completed_at_ms)
        case .childReceipt(let taskID, let receiptID, let parentTurnID, let childSessionID,
                           let childRunID, let inputWatermark, let completionWatermark,
                           let status, let receiptDigest, let resultDigest):
            try value.encode("child_receipt", forKey: .kind)
            try value.encode(taskID, forKey: .task_id)
            try value.encode(receiptID, forKey: .receipt_id)
            try value.encode(parentTurnID, forKey: .parent_turn_id)
            try value.encode(childSessionID, forKey: .child_session_id)
            try value.encode(childRunID, forKey: .child_run_id)
            try value.encode(inputWatermark, forKey: .input_watermark)
            try value.encode(completionWatermark, forKey: .completion_watermark)
            try value.encode(status, forKey: .status)
            try value.encode(receiptDigest, forKey: .receipt_digest)
            try value.encode(resultDigest, forKey: .result_digest)
        }
    }
}

nonisolated struct AppleAssistantRunTerminalManifestPage<JSON: JournalJSONValue>: Codable, Sendable {
    let seal: AppleAssistantRunTerminalSeal<JSON>
    let after_entry: String
    let next_entry: String
    let complete: Bool
    let entries: [AppleAssistantRunTerminalManifestEntry]
}

nonisolated struct AppleAssistantRunTerminalReception<JSON: JournalJSONValue>: Decodable, Sendable {
    let seal: AppleAssistantRunTerminalSeal<JSON>
    let next_entry: String
    let manifest_complete: Bool
    let committed: Bool
}

nonisolated struct AppleAssistantJournalRecord<JSON: JournalJSONValue>: Codable, Sendable {
    struct Header: Codable, Sendable {
        let protocol_version: UInt16
        let identity: AppleAssistantJournalIdentity
        let sequence: String
        let previous_record_digest: String
        let operation_id: String
        let source_commit_revision: String
        let operation_batch_digest: String
        let record_digest: String
    }
    let header: Header
    let batch: JSON
    var cursor: AppleAssistantJournalCursor {
        .init(identity: header.identity, sequence: header.sequence, record_digest: header.record_digest)
    }
}

nonisolated struct AppleAssistantJournalContentReference: Codable, Hashable, Sendable {
    let field: String
    let content_generation: String
    let utf8_bytes: String
    let chunk_count: String
    let content_root: String
}

nonisolated struct AppleAssistantJournalContentNeed: Decodable, Equatable, Sendable {
    let part_id: String
    let content: AppleAssistantJournalContentReference
    let expected_prefix: AppleAssistantJournalContentReference
}

nonisolated struct AppleAssistantJournalChunk: Codable, Sendable {
    let digest: String
    let utf8_bytes: String
    let data_base64: String
}

nonisolated struct AppleAssistantJournalSnapshot: Codable, Sendable {
    let snapshot_id: String
    let at: AppleAssistantJournalCursor
}

nonisolated struct AppleAssistantJournalReceipt: Decodable, Sendable {
    let applied: Bool
    let cursor: AppleAssistantJournalCursor
}

nonisolated struct AppleAssistantJournalReceptionState: Decodable, Sendable {
    let after: AppleAssistantJournalCursor
    let complete: Bool
}

nonisolated enum AppleAssistantJournalCatchUpError: Error {
    case snapshotRequired
}

nonisolated enum AppleAssistantJournalPage<JSON: JournalJSONValue>: Decodable, Sendable {
    struct Records: Decodable, Sendable {
        let after: AppleAssistantJournalCursor
        let through: AppleAssistantJournalCursor
        let records: [AppleAssistantJournalRecord<JSON>]
        let next: AppleAssistantJournalCursor
        let complete: Bool
    }
    case records(Records)
    case snapshotRequired
    private enum CodingKeys: String, CodingKey { case status }
    init(from decoder: Decoder) throws {
        let value = try decoder.container(keyedBy: CodingKeys.self)
        switch try value.decode(String.self, forKey: .status) {
        case "records": self = .records(try Records(from: decoder))
        case "snapshot_required": self = .snapshotRequired
        default: throw DecodingError.dataCorruptedError(forKey: .status, in: value, debugDescription: "Unknown journal page status")
        }
    }
}


nonisolated enum AppleAssistantJournalReadRequest<JSON: JournalJSONValue>: Encodable, Sendable {
    case admissionHead(conversationID: String, runID: String)
    case contextUsage(AppleAssistantJournalIdentity)
    case admissions(after: AppleRunMessageAdmissionCursor, through: AppleRunMessageAdmissionCursor)
    case terminalSeal(conversationID: String, runID: String)
    case terminalManifest(AppleAssistantRunTerminalSeal<JSON>, afterEntry: String)
    case heads(conversationID: String, runID: String, afterMessageID: String?)
    case records(after: AppleAssistantJournalCursor, through: AppleAssistantJournalCursor)
    case open(AppleAssistantJournalCursor)
    case chunk(AppleAssistantJournalSnapshot, AppleAssistantJournalContentNeed)
    case release(AppleAssistantJournalSnapshot)

    private enum CodingKeys: String, CodingKey {
        case operation, at, snapshot, part_id, content, chunk_index, after, through, max_records, max_encoded_bytes
        case conversation_id, run_id, after_message_id, limit
        case seal, after_entry, identity
    }
    func encode(to encoder: Encoder) throws {
        var value = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .contextUsage(let identity):
            try value.encode("context_usage", forKey: .operation)
            try value.encode(identity, forKey: .identity)
        case .admissionHead(let conversationID, let runID):
            try value.encode("admission_head", forKey: .operation)
            try value.encode(conversationID, forKey: .conversation_id)
            try value.encode(runID, forKey: .run_id)
        case .admissions(let after, let through):
            try value.encode("admissions", forKey: .operation)
            try value.encode(after, forKey: .after)
            try value.encode(through, forKey: .through)
            try value.encode(64, forKey: .max_records)
            try value.encode(256 * 1024, forKey: .max_encoded_bytes)
        case .terminalSeal(let conversationID, let runID):
            try value.encode("terminal_seal", forKey: .operation)
            try value.encode(conversationID, forKey: .conversation_id)
            try value.encode(runID, forKey: .run_id)
        case .terminalManifest(let seal, let afterEntry):
            try value.encode("terminal_manifest", forKey: .operation)
            try value.encode(seal, forKey: .seal)
            try value.encode(afterEntry, forKey: .after_entry)
            try value.encode(64, forKey: .max_records)
            try value.encode(128 * 1024, forKey: .max_encoded_bytes)
        case .heads(let conversationID, let runID, let afterMessageID):
            try value.encode("heads", forKey: .operation)
            try value.encode(conversationID, forKey: .conversation_id)
            try value.encode(runID, forKey: .run_id)
            try value.encodeIfPresent(afterMessageID, forKey: .after_message_id)
            try value.encode(64, forKey: .limit)
        case .records(let after, let through):
            try value.encode("read", forKey: .operation)
            try value.encode(after, forKey: .after)
            try value.encode(through, forKey: .through)
            try value.encode(32, forKey: .max_records)
            try value.encode(256 * 1024, forKey: .max_encoded_bytes)
        case .open(let cursor):
            try value.encode("open_snapshot", forKey: .operation)
            try value.encode(cursor, forKey: .at)
        case .chunk(let snapshot, let need):
            try value.encode("record_chunk", forKey: .operation)
            try value.encode(snapshot, forKey: .snapshot)
            try value.encode(need.part_id, forKey: .part_id)
            try value.encode(need.content, forKey: .content)
            try value.encode(need.expected_prefix.chunk_count, forKey: .chunk_index)
        case .release(let snapshot):
            try value.encode("release_snapshot", forKey: .operation)
            try value.encode(snapshot, forKey: .snapshot)
        }
    }
}

