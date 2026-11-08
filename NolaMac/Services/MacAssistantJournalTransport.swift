// SPDX-License-Identifier: Apache-2.0
// Transport values only. Core owns record validation, content roots and recovery.
import Foundation

typealias MacAssistantJournalIdentity = AppleAssistantJournalIdentity

typealias MacAssistantJournalCursor = AppleAssistantJournalCursor

typealias MacAssistantJournalNotification = AppleAssistantJournalNotification

// This is source completion evidence, not a local completion flag. Application
// must admit its immutable manifest after all required journal heads are durable.
typealias MacRunMessageAdmissionCursor = AppleRunMessageAdmissionCursor

typealias MacRunMessageAdmissionPage = AppleRunMessageAdmissionPage<MacCanonicalJSONValue>

nonisolated struct MacCommittedRunContextUsage: Codable, Equatable, Sendable {
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
        var measurement: MacContextMeasurement? = nil
        var prompt_cache_measurement: MacContextMeasurement? = nil
        var prompt_cache_diagnostic: MacPromptCacheDiagnostic? = nil
        var cache_usage: MacPromptCacheUsage? = nil
        let used_tokens: String
        let context_window_tokens: String
        let prompt_token_limit: String
        let estimated: Bool
    }
    let protocol_version: UInt16
    let identity: MacAssistantJournalIdentity
    let admission: MacRunMessageAdmissionCursor
    let event_sequence: String
    let attempt_sequence: String
    let run_cache_usage: RunCacheUsage?
    let usage: Usage
    let record_digest: String
}

// Transport retains the complete result evidence. Rust validates source ownership,
// required variant fields and digest before accepting any terminal publication.
typealias MacRunResult = AppleRunResult<MacCanonicalJSONValue>

typealias MacAssistantRunTerminalSeal = AppleAssistantRunTerminalSeal<MacCanonicalJSONValue>

typealias MacAssistantRunTerminalManifestEntry = AppleAssistantRunTerminalManifestEntry

typealias MacAssistantRunTerminalManifestPage = AppleAssistantRunTerminalManifestPage<MacCanonicalJSONValue>

typealias MacAssistantRunTerminalReception = AppleAssistantRunTerminalReception<MacCanonicalJSONValue>

typealias MacAssistantJournalRecord = AppleAssistantJournalRecord<MacCanonicalJSONValue>

typealias MacAssistantJournalContentReference = AppleAssistantJournalContentReference

typealias MacAssistantJournalContentNeed = AppleAssistantJournalContentNeed

typealias MacAssistantJournalChunk = AppleAssistantJournalChunk

typealias MacAssistantJournalSnapshot = AppleAssistantJournalSnapshot

// The presentation reader preserves all public metadata kinds and immutable
// body references. It must not eagerly hydrate tool JSON or treat metadata as
// an execution/permission grant.
nonisolated struct MacAssistantJournalPart: Decodable, Sendable {
    struct Order: Decodable, Sendable {
        let section: String
        let position: String
    }
    let identity: MacAssistantJournalIdentity
    let part_id: String
    let generation: String
    let revision: String
    let metadata_revision: String
    let order_key: Order
    let metadata: MacAssistantJournalMetadata
    let bodies: [MacAssistantJournalContentReference]
}

nonisolated struct MacAssistantJournalPartPage: Decodable, Sendable {
    let at: MacAssistantJournalCursor
    let parts: [MacAssistantJournalPart]
    let next_part_id: String?
}

nonisolated struct MacAssistantJournalPartChangePage: Decodable, Sendable {
    struct Change: Decodable, Sendable {
        let part_id: String
        let head: MacAssistantJournalPart?
        private enum CodingKeys: String, CodingKey { case part_id, head }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            part_id = try values.decode(String.self, forKey: .part_id)
            // decode(Optional.self), unlike decodeIfPresent, requires a key.
            head = try values.decode(MacAssistantJournalPart?.self, forKey: .head)
            guard !part_id.isEmpty, head == nil || head?.part_id == part_id else {
                throw DecodingError.dataCorruptedError(forKey: .head, in: values,
                    debugDescription: "Part change requires an exact ID and an explicit head or retirement")
            }
        }
    }
    let base: MacAssistantJournalCursor
    let at: MacAssistantJournalCursor
    let changes: [Change]
    let next_part_id: String?
    private enum CodingKeys: String, CodingKey { case base, at, changes, next_part_id }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        base = try values.decode(MacAssistantJournalCursor.self, forKey: .base)
        at = try values.decode(MacAssistantJournalCursor.self, forKey: .at)
        changes = try values.decode([Change].self, forKey: .changes)
        next_part_id = try values.decode(String?.self, forKey: .next_part_id)
    }
}

nonisolated struct MacAssistantJournalTextRange: Decodable, Sendable {
    let offset_bytes: String
    let next_offset_bytes: String
    let total_utf8_bytes: String
    let text: String
    let end_of_content: Bool
}

typealias MacAssistantJournalReceipt = AppleAssistantJournalReceipt

typealias MacAssistantJournalReceptionState = AppleAssistantJournalReceptionState

typealias MacAssistantJournalCatchUpError = AppleAssistantJournalCatchUpError

typealias MacAssistantJournalPage = AppleAssistantJournalPage<MacCanonicalJSONValue>

typealias MacAssistantJournalReadRequest = AppleAssistantJournalReadRequest<MacCanonicalJSONValue>

// JSON adapters retain each platform transport; protocol validation is shared.
extension MacCanonicalJSONValue: JournalJSONValue {
    nonisolated var journalObject: [String: Self]? { if case .object(let value) = self { return value }; return nil }
    nonisolated var journalArray: [Self]? { if case .array(let value) = self { return value }; return nil }
    nonisolated var journalString: String? { if case .string(let value) = self { return value }; return nil }
    nonisolated var journalIsNull: Bool { self == .null }
}
