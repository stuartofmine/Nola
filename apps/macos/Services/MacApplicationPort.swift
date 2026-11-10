// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides the macOS Application port backed by the Nola service protocol.

import Foundation

#if canImport(NolaCore)
import NolaCore
#endif

nonisolated enum MacApplicationPortError: LocalizedError, Equatable {
    case serviceUnavailable
    case invalidResponse
    case rejected(String)

    var errorDescription: String? {
        switch self {
        case .serviceUnavailable:
            String(localized: "The Nola application service is unavailable.")
        case .invalidResponse:
            String(localized: "The Nola application service returned an invalid response.")
        case let .rejected(message):
            message
        }
    }
}

nonisolated struct MacCanonicalProductSetting: Codable, Equatable, Sendable {
    let key: String
    let valueJSON: String
    let revision: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case key, revision
        case valueJSON = "value_json"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacCanonicalPendingSyncChange: Codable, Equatable, Sendable {
    let operationID: String
    let entityType: String
    let entityID: String
    let revision: Int64
    let deleted: Bool
    let payloadJSON: String
    let createdAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case revision, deleted
        case operationID = "operation_id"
        case entityType = "entity_type"
        case entityID = "entity_id"
        case payloadJSON = "payload_json"
        case createdAtMs = "created_at_ms"
    }
}

nonisolated struct MacCanonicalSyncCursor: Codable, Equatable, Sendable {
    let replicaID: String
    let cursor: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case cursor
        case replicaID = "replica_id"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacCanonicalRemoteSyncChange: Codable, Equatable, Sendable {
    let cursor: Int64
    let operationID: String
    let entityType: String
    let entityID: String
    let revision: Int64
    let deleted: Bool
    let payloadJSON: String
    let createdAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case cursor, revision, deleted
        case operationID = "operation_id"
        case entityType = "entity_type"
        case entityID = "entity_id"
        case payloadJSON = "payload_json"
        case createdAtMs = "created_at_ms"
    }
}

nonisolated struct MacCanonicalRemoteSyncPageResult: Codable, Equatable, Sendable {
    let applied: Int
    let skipped: Int
    let cursor: Int64
}

nonisolated struct MacCanonicalMemoryRecord: Codable, Equatable, Sendable {
    let record: MacCanonicalSemanticMemoryRecord
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let lastUsedAtMs: Int64?
    let deletedAtMs: Int64?

    var id: String { record.id }

    private enum CodingKeys: String, CodingKey {
        case record
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case lastUsedAtMs = "last_used_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct MacCanonicalSemanticMemoryRecord: Codable, Equatable, Sendable {
    let id: String
    let namespace: MacCanonicalJSONValue
    let scope: String
    let kind: String
    let content: String
    let summary: String?
    let tags: [String]
    let confidence: Double
    let importance: Double
    let sensitivity: String?
    let retention: MacCanonicalJSONValue
    let supersedes: [String]
    let provenance: MacCanonicalJSONValue
}

nonisolated struct MacCanonicalConversationPlan: Codable, Equatable, Sendable {
    let conversationID: String
    let todos: MacCanonicalJSONValue
    let revision: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case revision
        case conversationID = "conversation_id"
        case todos
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated enum MacCanonicalDataSourceKind: String, Codable, Sendable {
    case http
    case localFile = "local_file"
    case directory
    case platform
    case web
    case database
    case mcp
    case custom
}

nonisolated enum MacCanonicalDataSourceLocationKind: String, Codable, Sendable {
    case httpURL = "http_url"
    case localPath = "local_path"
    case platformResource = "platform_resource"
    case databaseReference = "database_reference"
    case mcpCapability = "mcp_capability"
    case opaqueReference = "opaque_reference"
}

nonisolated struct MacCanonicalDataSourceConfiguration: Codable, Equatable, Sendable {
    let connectorID: String
    let locationKind: MacCanonicalDataSourceLocationKind
    let location: String
    let parameters: [String: MacCanonicalJSONValue]
    let secretReferences: [String]

    private enum CodingKeys: String, CodingKey {
        case parameters, location
        case connectorID = "connector_id"
        case locationKind = "location_kind"
        case secretReferences = "secret_references"
    }
}

nonisolated enum MacCanonicalDataSourceAuthorizationState: String, Codable, Sendable {
    case notRequired = "not_required"
    case notDetermined = "not_determined"
    case authorized
    case denied
    case restricted
    case unavailable
}

nonisolated struct MacCanonicalDataSourceAuthorization: Codable, Equatable, Sendable {
    let state: MacCanonicalDataSourceAuthorizationState
    let capability: String?
    let observedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case state, capability
        case observedAtMs = "observed_at_ms"
    }
}

nonisolated struct MacCanonicalDataSourceFreshnessPolicy: Codable, Equatable, Sendable {
    let maximumAgeMinutes: UInt64
    let allowStaleFallback: Bool

    private enum CodingKeys: String, CodingKey {
        case maximumAgeMinutes = "maximum_age_minutes"
        case allowStaleFallback = "allow_stale_fallback"
    }
}

nonisolated enum MacCanonicalDataSourceLifecycle: String, Codable, Sendable {
    case draft
    case ready
    case disabled
    case failed
}

nonisolated enum MacCanonicalDataSourceValidationStatus: String, Codable, Sendable {
    case notValidated = "not_validated"
    case pending
    case verified
    case verifiedEmpty = "verified_empty"
    case failed
}

nonisolated struct MacCanonicalDataSourceValidation: Codable, Equatable, Sendable {
    let status: MacCanonicalDataSourceValidationStatus
    let validationToken: String?
    let contract: MacDataSourceContract?
    let diagnostic: String?
    let attemptedAtMs: Int64?
    let completedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case status, contract, diagnostic
        case validationToken = "validation_token"
        case attemptedAtMs = "attempted_at_ms"
        case completedAtMs = "completed_at_ms"
    }
}

nonisolated struct MacCanonicalDataSource: Codable, Equatable, Sendable {
    let id: String
    let name: String
    let kind: MacCanonicalDataSourceKind
    let configuration: MacCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: MacCanonicalDataSourceAuthorization
    let validation: MacCanonicalDataSourceValidation
    let freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy
    let defaultEnabled: Bool
    let lifecycle: MacCanonicalDataSourceLifecycle
    let capabilities: [String]
    let privacy: String
    let revision: Int64
    let lastSuccessfulReadAtMs: Int64?
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, name, kind, configuration, authorization, validation, lifecycle
        case capabilities, privacy, revision
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case defaultEnabled = "default_enabled"
        case lastSuccessfulReadAtMs = "last_successful_read_at_ms"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct MacCanonicalDataSourceDraft: Codable, Sendable {
    let id: String
    let name: String
    let kind: MacCanonicalDataSourceKind
    let configuration: MacCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: MacCanonicalDataSourceAuthorization
    let freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy
    let capabilities: [String]
    let privacy: String
    let createdAtMs: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id, name, kind, configuration, authorization, capabilities, privacy
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacCanonicalDataSourceConfigurationUpdate: Codable, Sendable {
    let name: String
    let kind: MacCanonicalDataSourceKind
    let configuration: MacCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: MacCanonicalDataSourceAuthorization
    let freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy
    let capabilities: [String]
    let privacy: String
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case name, kind, configuration, authorization, capabilities, privacy
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated enum MacCanonicalDataSourceOverride: String, Codable, Sendable {
    case inherit
    case enabled
    case disabled
}

nonisolated struct MacCanonicalDataSourceUsageInput: Codable, Equatable, Sendable {
    let id: String
    let sourceID: String
    let sourceTitle: String
    let consumer: String
    let purpose: String
    let conversationID: String?
    let disclosedToCloud: Bool
    let metadata: [String: MacCanonicalJSONValue]
    let usedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id, consumer, purpose, metadata
        case sourceID = "source_id"
        case sourceTitle = "source_title"
        case conversationID = "conversation_id"
        case disclosedToCloud = "disclosed_to_cloud"
        case usedAtMs = "used_at_ms"
    }
}

nonisolated struct MacCanonicalOutboxEffectInput: Codable, Equatable, Sendable {
    let effectID: String
    let requestID: String
    let effectKind: String
    let payloadJSON: String
}

nonisolated struct MacCanonicalHostEventInput: Codable, Equatable, Sendable {
    let eventID: String
    let effectID: String
    let requestID: String
    let sequence: Int64
    let payloadJSON: String
    let receivedAtMs: Int64
}

nonisolated struct MacCanonicalRemoteTransitionCommit: Decodable, Equatable, Sendable {
    let applied: Bool
    let nextEventSequence: Int64

    private enum CodingKeys: String, CodingKey {
        case applied
        case nextEventSequence = "next_event_sequence"
    }
}

nonisolated struct MacCanonicalRunTerminal: Decodable, Equatable, Sendable {
    enum Status: String, Decodable, Sendable { case sealed }
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let status: Status
    let result: MacRunResult
    let resultDigest: String
    let completedAtMs: Int64
    private enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", runID = "run_id"
        case assistantMessageID = "assistant_message_id", status, completedAtMs = "completed_at_ms"
        case result, resultDigest = "result_digest"
    }
}

nonisolated enum MacExportBodyField: String, Codable, Sendable {
    case contentText = "content_text"
    case contentJSON = "content_json"
}

nonisolated struct MacCanonicalPartOwner: Codable, Hashable, Sendable {
    let conversationID: String
    let messageID: String
    let partID: String
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
    }
}

nonisolated struct MacPartFieldFingerprint: Codable, Equatable, Sendable {
    let field: MacExportBodyField
    let totalUTF8Bytes: UInt64
    let sha256: String
    enum CodingKeys: String, CodingKey {
        case field, sha256, totalUTF8Bytes = "total_utf8_bytes"
    }
}

nonisolated struct MacPartDerivationSource: Codable, Equatable, Sendable {
    let owner: MacCanonicalPartOwner
    let runID: String?
    let kind: String
    let state: String
    let fields: [MacPartFieldFingerprint?]
    enum CodingKeys: String, CodingKey {
        case owner, kind, state, fields, runID = "run_id"
    }
}

// A backup preserves this claim, never the Core-owned provenance authority.
nonisolated struct MacPartDerivation: Codable, Equatable, Sendable {
    let version: UInt32
    let operationID: String
    let target: MacPartDerivationSource
    let immediateSource: MacPartDerivationSource
    let rootSource: MacPartDerivationSource?
    enum CodingKeys: String, CodingKey {
        case version, target, operationID = "operation_id"
        case immediateSource = "immediate_source", rootSource = "root_source"
    }
}

nonisolated struct MacStoredPartDerivation: Codable, Equatable, Sendable {
    enum Provenance: String, Codable, Sendable {
        case localBranch = "local_branch", replica, archive
    }
    let derivation: MacPartDerivation
    let provenance: Provenance
}

nonisolated struct MacConversationExportPage: Codable, Sendable {
    struct Header: Codable, Equatable, Sendable {
        let id: String
        let title: String
        let revision: Int64
        let createdAtMs: Int64
        let updatedAtMs: Int64
        let settingsJSON: String?
        enum CodingKeys: String, CodingKey {
            case id, title, revision
            case createdAtMs = "created_at_ms", updatedAtMs = "updated_at_ms", settingsJSON = "settings_json"
        }
    }
    struct Part: Codable, Sendable {
        let id: String
        let kind: String
        let ordinal: Int64
        let state: String
        let contentDigest: String?
        let createdAtMs: Int64
        let updatedAtMs: Int64
        let contentTextUTF8Bytes: UInt64
        let contentJSONUTF8Bytes: UInt64?
        let hasDerivation: Bool
        enum CodingKeys: String, CodingKey {
            case id, kind, ordinal, state
            case hasDerivation = "has_derivation"
            case contentDigest = "content_digest", createdAtMs = "created_at_ms", updatedAtMs = "updated_at_ms"
            case contentTextUTF8Bytes = "content_text_utf8_bytes", contentJSONUTF8Bytes = "content_json_utf8_bytes"
        }
        init(id: String, kind: String, ordinal: Int64, state: String, contentDigest: String?,
             createdAtMs: Int64, updatedAtMs: Int64, contentTextUTF8Bytes: UInt64,
             contentJSONUTF8Bytes: UInt64?, hasDerivation: Bool = false) {
            self.id = id
            self.kind = kind
            self.ordinal = ordinal
            self.state = state
            self.contentDigest = contentDigest
            self.createdAtMs = createdAtMs
            self.updatedAtMs = updatedAtMs
            self.contentTextUTF8Bytes = contentTextUTF8Bytes
            self.contentJSONUTF8Bytes = contentJSONUTF8Bytes
            self.hasDerivation = hasDerivation
        }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            id = try values.decode(String.self, forKey: .id)
            kind = try values.decode(String.self, forKey: .kind)
            ordinal = try values.decode(Int64.self, forKey: .ordinal)
            state = try values.decode(String.self, forKey: .state)
            contentDigest = try values.decodeIfPresent(String.self, forKey: .contentDigest)
            createdAtMs = try values.decode(Int64.self, forKey: .createdAtMs)
            updatedAtMs = try values.decode(Int64.self, forKey: .updatedAtMs)
            contentTextUTF8Bytes = try values.decode(UInt64.self, forKey: .contentTextUTF8Bytes)
            contentJSONUTF8Bytes = try values.decodeIfPresent(UInt64.self, forKey: .contentJSONUTF8Bytes)
            hasDerivation = try values.decode(Bool.self, forKey: .hasDerivation)
        }
    }
    struct Message: Codable, Sendable {
        let id: String
        let runID: String?
        let role: String
        let status: String
        let ordinal: Int64
        let createdAtMs: Int64
        let completedAtMs: Int64?
        let parts: [Part]
        enum CodingKeys: String, CodingKey {
            case id, role, status, ordinal, parts
            case runID = "run_id", createdAtMs = "created_at_ms", completedAtMs = "completed_at_ms"
        }
    }
    let conversation: Header
    let messages: [Message]
    let nextOrdinal: Int64?
    let hasMore: Bool
    enum CodingKeys: String, CodingKey {
        case conversation, messages
        case nextOrdinal = "next_ordinal", hasMore = "has_more"
    }
}

nonisolated struct MacConversationExportRange: Decodable, Sendable {
    let conversationID: String
    let messageID: String
    let partID: String
    let field: MacExportBodyField
    let revision: Int64
    let offsetBytes: UInt64
    let nextOffsetBytes: UInt64
    let totalUTF8Bytes: UInt64
    let text: String
    let endOfContent: Bool
    enum CodingKeys: String, CodingKey {
        case field, revision, text
        case conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
        case offsetBytes = "offset_bytes", nextOffsetBytes = "next_offset_bytes"
        case totalUTF8Bytes = "total_utf8_bytes", endOfContent = "end_of_content"
    }
}

nonisolated struct MacCanonicalConversationPart: Codable, Equatable, Sendable {
    let id: String
    let kind: String
    let ordinal: Int64
    let state: String
    let contentText: String
    let contentJSON: String?
    let contentDigest: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let derivation: MacPartDerivation?

    init(id: String, kind: String, ordinal: Int64, state: String, contentText: String, contentJSON: String?,
         contentDigest: String? = nil, createdAtMs: Int64, updatedAtMs: Int64, derivation: MacPartDerivation? = nil) {
        self.id = id
        self.kind = kind
        self.ordinal = ordinal
        self.state = state
        self.contentText = contentText
        self.contentJSON = contentJSON
        self.contentDigest = contentDigest
        self.createdAtMs = createdAtMs
        self.updatedAtMs = updatedAtMs
        self.derivation = derivation
    }

    private enum CodingKeys: String, CodingKey {
        case id, kind, ordinal, state, derivation
        case contentText = "content_text"
        case contentJSON = "content_json"
        case contentDigest = "content_digest", createdAtMs = "created_at_ms", updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacCanonicalConversationMessage: Codable, Equatable, Sendable {
    let id: String
    let runID: String?
    let role: String
    let status: String
    let ordinal: Int64
    let createdAtMs: Int64
    let completedAtMs: Int64?
    let parts: [MacCanonicalConversationPart]

    private enum CodingKeys: String, CodingKey {
        case id, role, status, ordinal, parts
        case runID = "run_id"
        case createdAtMs = "created_at_ms"
        case completedAtMs = "completed_at_ms"
    }
}

nonisolated struct MacCanonicalConversation: Codable, Equatable, Sendable {
    let id: String
    let title: String
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let settingsJSON: String?
    let messages: [MacCanonicalConversationMessage]

    private enum CodingKeys: String, CodingKey {
        case id, title, revision, messages
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case settingsJSON = "settings_json"
    }
}

/// Versioned, UI-neutral conversation projection materialized by the Rust
/// application layer. The Mac client decodes this contract strictly and never
/// reconstructs it from Host events or diagnostic traces.
/// A display identity only. It neither exposes a Host path nor authorizes a read.
nonisolated struct MacCanonicalToolFullOutput: Codable, Hashable, Sendable {
    let messageID: String
    let partID: String
    let toolCallID: String
    private enum CodingKeys: String, CodingKey, CaseIterable {
        case messageID = "message_id", partID = "part_id", toolCallID = "tool_call_id"
    }
    private struct AnyKey: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    init?(messageID: String, partID: String, toolCallID: String) {
        guard [messageID, partID, toolCallID].allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else { return nil }
        self.messageID = messageID
        self.partID = partID
        self.toolCallID = toolCallID
    }
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: AnyKey.self)
        guard Set(keys.allKeys.map(\.stringValue)) == Set(CodingKeys.allCases.map(\.rawValue)) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Full output is an exact canonical identity, never a path or read grant"))
        }
        let values = try decoder.container(keyedBy: CodingKeys.self)
        messageID = try values.decode(String.self, forKey: .messageID)
        partID = try values.decode(String.self, forKey: .partID)
        toolCallID = try values.decode(String.self, forKey: .toolCallID)
        guard [messageID, partID, toolCallID].allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Full output requires non-empty canonical identities"))
        }
    }
}

nonisolated struct MacCanonicalConversationProjectionV1: Decodable, Equatable, Sendable {
    let contractVersion: UInt32
    let conversationID: String
    let revision: UInt64
    let title: String
    let messages: [Message]
    let activeRun: RunActivity?
    let surface: Surface
    let contextUsage: ContextUsage

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

        init(messageID: String, role: Role, state: MessageState, revision: UInt64,
             createdAtMs: Int64, parts: [Part]) {
            self.messageID = messageID
            self.role = role
            self.state = state
            self.revision = revision
            self.createdAtMs = createdAtMs
            self.parts = parts
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            messageID = try values.decode(String.self, forKey: .messageID)
            role = try values.decode(Role.self, forKey: .role)
            state = try values.decode(MessageState.self, forKey: .state)
            revision = try values.decode(UInt64.self, forKey: .revision)
            createdAtMs = try values.decode(Int64.self, forKey: .createdAtMs)
            parts = try values.decode([Part].self, forKey: .parts)
            for case .tool(let tool) in parts {
                if let fullOutput = tool.fullOutput,
                   fullOutput.messageID != messageID || fullOutput.partID != tool.partID || fullOutput.toolCallID != tool.toolCallID {
                    throw DecodingError.dataCorruptedError(forKey: .parts, in: values,
                        debugDescription: "Full output belongs to the exact message, Part and tool call")
                }
            }
        }
    }

    enum Role: String, Decodable, Equatable, Sendable {
        case user
        case assistant
        case system
    }

    enum MessageState: String, Decodable, Equatable, Sendable {
        case pending
        case running
        case completed
        case cancelled
        case failed
    }

    enum ActivityState: String, Decodable, Equatable, Sendable {
        case pending
        case running
        case waitingForPermission = "waiting_for_permission"
        case waitingForInput = "waiting_for_input"
        case completed
        case sealed
        case cancelled
        case failed
    }

    struct TextPart: Decodable, Equatable, Sendable {
        enum Review: String, Codable, Hashable, Sendable { case pending, approved, rejected, interrupted }
        let partID: String
        let revision: UInt64
        let markdown: String
        let isFinal: Bool
        let review: Review?

        var isDisclosed: Bool { review == nil || review == .approved }

        private enum CodingKeys: String, CodingKey {
            case revision, markdown, review
            case partID = "part_id"
            case isFinal = "is_final"
        }
    }

    struct ReasoningPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let state: ActivityState
        let title: String
        let summary: String?

        private enum CodingKeys: String, CodingKey {
            case revision, state, title, summary
            case partID = "part_id"
        }
    }

    nonisolated struct ToolExecutionSource: Decodable, Equatable, Sendable {
        static let model = ToolExecutionSource()
        private init() { kind = .model; cell = nil }
        enum Kind: String, Decodable, Sendable { case model, codeCell = "code_cell" }
        let kind: Kind
        let cell: CodeCellIdentity?
        private enum CodingKeys: String, CodingKey { case kind, cell }
        private struct SourceKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            kind = try values.decode(Kind.self, forKey: .kind)
            let keys = try decoder.container(keyedBy: SourceKey.self)
            let expected: Set<String> = kind == .model ? ["kind"] : ["kind", "cell"]
            guard Set(keys.allKeys.map(\.stringValue)) == expected else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Unexpected tool execution source fields"))
            }
            cell = try values.decodeIfPresent(CodeCellIdentity.self, forKey: .cell)
            guard (kind == .codeCell) == (cell != nil) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Tool execution source and cell identity disagree"))
            }
        }
    }

    nonisolated struct CodeCellIdentity: Decodable, Equatable, Sendable {
        let hostID: String
        let conversationID: String
        let turnID: String
        let parentCallID: String
        let cellID: String
        let generation: String
        private enum CodingKeys: String, CodingKey, CaseIterable {
            case hostID = "host_id", conversationID = "conversation_id", turnID = "turn_id"
            case parentCallID = "parent_call_id", cellID = "cell_id", generation
        }
        private struct SourceKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }
        init(from decoder: Decoder) throws {
            let keys = try decoder.container(keyedBy: SourceKey.self)
            guard Set(keys.allKeys.map(\.stringValue)) == Set(CodingKeys.allCases.map(\.rawValue)) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Code cell identity requires exactly six owner fields"))
            }
            let values = try decoder.container(keyedBy: CodingKeys.self)
            hostID = try values.decode(String.self, forKey: .hostID)
            conversationID = try values.decode(String.self, forKey: .conversationID)
            turnID = try values.decode(String.self, forKey: .turnID)
            parentCallID = try values.decode(String.self, forKey: .parentCallID)
            cellID = try values.decode(String.self, forKey: .cellID)
            generation = try values.decode(String.self, forKey: .generation)
            guard [hostID, conversationID, turnID, parentCallID, cellID, generation].allSatisfy({
                !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && $0.utf8.count <= 512
                    && $0.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
            }) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Invalid code cell provenance identity"))
            }
        }
    }

    struct ToolPart: Decodable, Equatable, Sendable {
        let executionSource: ToolExecutionSource
        let partID: String
        let revision: UInt64
        let toolCallID: String
        let capabilityID: String
        let title: String
        let summary: String?
        let origin: MacCoreCapabilityOrigin
        let permission: MacCoreToolPermission?
        let state: ActivityState
        let displayKey: String?
        let input: MacCanonicalJSONValue
        let output: MacCanonicalJSONValue?
        let outputPreview: String?
        let fullOutput: MacCanonicalToolFullOutput?
        let artifacts: [MacCorePartArtifact]
        let startedAtMs: UInt64?
        let finishedAtMs: UInt64?
        let affectedResources: [AffectedResource]

        private enum CodingKeys: String, CodingKey {
            case revision, state, title, summary, origin, permission, input, output, artifacts
            case partID = "part_id"
            case executionSource = "execution_source"
            case toolCallID = "tool_call_id"
            case capabilityID = "capability_id"
            case displayKey = "display_key"
            case affectedResources = "affected_resources"
            case outputPreview = "output_preview"
            case fullOutput = "full_output"
            case startedAtMs = "started_at_ms"
            case finishedAtMs = "finished_at_ms"
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            partID = try values.decode(String.self, forKey: .partID)
            revision = try values.decode(UInt64.self, forKey: .revision)
            toolCallID = try values.decode(String.self, forKey: .toolCallID)
            capabilityID = try values.decode(String.self, forKey: .capabilityID)
            executionSource = try values.decode(ToolExecutionSource.self, forKey: .executionSource)
            title = try values.decode(String.self, forKey: .title)
            summary = try values.decodeIfPresent(String.self, forKey: .summary)
            origin = try values.decode(MacCoreCapabilityOrigin.self, forKey: .origin)
            permission = try values.decodeIfPresent(MacCoreToolPermission.self, forKey: .permission)
            guard !capabilityID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DecodingError.dataCorruptedError(
                    forKey: .capabilityID,
                    in: values,
                    debugDescription: "Tool Part requires a non-empty capability_id."
                )
            }
            state = try values.decode(ActivityState.self, forKey: .state)
            displayKey = try values.decodeIfPresent(String.self, forKey: .displayKey)
            input = try values.decode(MacCanonicalJSONValue.self, forKey: .input)
            // Omitted means absent; explicit JSON null is a structured result.
            output = values.contains(.output) ? try values.decode(MacCanonicalJSONValue.self, forKey: .output) : nil
            outputPreview = try values.decodeIfPresent(String.self, forKey: .outputPreview)
            fullOutput = try values.decodeIfPresent(MacCanonicalToolFullOutput.self, forKey: .fullOutput)
            if let fullOutput, fullOutput.partID != partID || fullOutput.toolCallID != toolCallID {
                throw DecodingError.dataCorruptedError(forKey: .fullOutput, in: values,
                    debugDescription: "Full output identity does not match the Tool Part")
            }
            artifacts = try values.decode([MacCorePartArtifact].self, forKey: .artifacts)
            startedAtMs = try values.decodeIfPresent(UInt64.self, forKey: .startedAtMs)
            finishedAtMs = try values.decodeIfPresent(UInt64.self, forKey: .finishedAtMs)
            affectedResources = try values.decode([AffectedResource].self, forKey: .affectedResources)
        }
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

    struct SourcePart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let sourceID: String
        let title: String
        let uri: String
        let mimeType: String?
        let snippet: String?
        let evidence: MacCoreSourceEvidence?

        private enum CodingKeys: String, CodingKey {
            case revision, title, uri, snippet, evidence
            case partID = "part_id"
            case sourceID = "source_id"
            case mimeType = "mime_type"
        }
    }

    struct CitationPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let sourceID: String
        let citationID: String
        let title: String
        let uri: String
        let mimeType: String?
        let section: String?
        let location: MacCoreSourceLocation?
        let originToolCallID: String
        let page: UInt64?
        let lineStart: UInt64?
        let lineEnd: UInt64?
        let rangeStart: UInt64?
        let rangeEnd: UInt64?

        private enum CodingKeys: String, CodingKey {
            case revision, title, uri, page, section, location
            case partID = "part_id"
            case sourceID = "source_id"
            case citationID = "citation_id"
            case mimeType = "mime_type"
            case originToolCallID = "origin_tool_call_id"
            case lineStart = "line_start"
            case lineEnd = "line_end"
            case rangeStart = "range_start"
            case rangeEnd = "range_end"
        }

        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            partID = try v.decode(String.self, forKey: .partID)
            revision = try v.decode(UInt64.self, forKey: .revision)
            sourceID = try v.decode(String.self, forKey: .sourceID)
            citationID = try v.decode(String.self, forKey: .citationID)
            title = try v.decode(String.self, forKey: .title)
            uri = try v.decode(String.self, forKey: .uri)
            mimeType = try v.decodeIfPresent(String.self, forKey: .mimeType)
            section = try v.decodeIfPresent(String.self, forKey: .section)
            location = try v.decodeIfPresent(MacCoreSourceLocation.self, forKey: .location)
            originToolCallID = try v.decode(String.self, forKey: .originToolCallID)
            page = try v.decodeIfPresent(UInt64.self, forKey: .page)
            lineStart = try v.decodeIfPresent(UInt64.self, forKey: .lineStart)
            lineEnd = try v.decodeIfPresent(UInt64.self, forKey: .lineEnd)
            guard location?.page == page, location?.lineStart == lineStart,
                  location?.lineEnd == lineEnd, location?.section == section else {
                throw DecodingError.dataCorruptedError(forKey: .location, in: v,
                    debugDescription: "Citation convenience fields differ from its exact location")
            }
            rangeStart = try v.decodeIfPresent(UInt64.self, forKey: .rangeStart)
            rangeEnd = try v.decodeIfPresent(UInt64.self, forKey: .rangeEnd)
            guard (rangeStart == nil) == (rangeEnd == nil) else {
                throw DecodingError.dataCorruptedError(forKey: .rangeStart, in: v,
                    debugDescription: "Citation range must have both bounds or neither")
            }
        }
    }

    struct AttachmentPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let attachmentID: String
        let name: String
        let mimeType: String?
        let uri: String
        let operation: String?

        private enum CodingKeys: String, CodingKey {
            case revision, name, uri, operation
            case partID = "part_id"
            case attachmentID = "attachment_id"
            case mimeType = "mime_type"
        }
    }

    struct TaskPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let taskID: String
        let displayKey: String
        let state: ActivityState
        let progress: Double?
        let currentStep: String?
        let resultRef: String?
        let cancelRequested: Bool

        private enum CodingKeys: String, CodingKey {
            case revision, state, progress
            case partID = "part_id"
            case taskID = "task_id"
            case displayKey = "display_key"
            case currentStep = "current_step"
            case resultRef = "result_ref"
            case cancelRequested = "cancel_requested"
        }
    }

    struct SkillPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let runID: String
        let skillID: String
        let displayName: String
        let mode: String
        let state: ActivityState
        let modelTurns: UInt32
        let toolCalls: UInt32
        let elapsedMillis: UInt64
        let resultRef: String?
        let failure: String?

        private enum CodingKeys: String, CodingKey {
            case revision, mode, state, failure
            case partID = "part_id"
            case runID = "run_id"
            case skillID = "skill_id"
            case displayName = "display_name"
            case modelTurns = "model_turns"
            case toolCalls = "tool_calls"
            case elapsedMillis = "elapsed_millis"
            case resultRef = "result_ref"
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
            case partID = "part_id"
            case attemptID = "attempt_id"
            case fallbackAttemptID = "fallback_attempt_id"
            case fallbackProvider = "fallback_provider"
            case fallbackModel = "fallback_model"
        }
    }

    struct CompactionPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let summary: String
        let sourceItemIDs: [String]
        let sourceFingerprint: String
        let originalTokenCount: UInt64
        let compactedTokenCount: UInt64

        private enum CodingKeys: String, CodingKey {
            case revision, summary
            case partID = "part_id"
            case sourceItemIDs = "source_item_ids"
            case sourceFingerprint = "source_fingerprint"
            case retainedItemIDs = "retained_item_ids"
            case originalTokenCount = "original_token_count"
            case compactedTokenCount = "compacted_token_count"
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            partID = try values.decode(String.self, forKey: .partID)
            revision = try values.decode(UInt64.self, forKey: .revision)
            summary = try values.decode(String.self, forKey: .summary)
            sourceItemIDs = try values.decodeIfPresent([String].self, forKey: .sourceItemIDs)
                ?? values.decodeIfPresent([String].self, forKey: .retainedItemIDs)
                ?? []
            sourceFingerprint = try values.decodeIfPresent(String.self, forKey: .sourceFingerprint)
                ?? ""
            originalTokenCount = try values.decode(UInt64.self, forKey: .originalTokenCount)
            compactedTokenCount = try values.decode(UInt64.self, forKey: .compactedTokenCount)
        }
    }

    struct ErrorPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let code: String
        let displayKey: String
        let recoverable: Bool

        private enum CodingKeys: String, CodingKey {
            case revision, code, recoverable
            case partID = "part_id"
            case displayKey = "display_key"
        }
    }

    /// Read-only historical data. Never converted into a live tool/permission Part.
    struct HistoricalProjection: Decodable, Equatable, Sendable {
        let owner: MacCanonicalJSONValue
        let canonicalRevision: Int64
        let source: MacCanonicalJSONValue?
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
            case complete(MacCanonicalJSONValue)
            case deferred(MacCanonicalJSONValue)
            private enum Keys: String, CodingKey { case availability, value, locator }
            init(from decoder: Decoder) throws {
                let values = try decoder.container(keyedBy: Keys.self)
                switch try values.decode(String.self, forKey: .availability) {
                case "complete": self = .complete(try values.decode(MacCanonicalJSONValue.self, forKey: .value))
                case "deferred": self = .deferred(try values.decode(MacCanonicalJSONValue.self, forKey: .locator))
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
        /// Chat text remains readable when an older structured envelope cannot
        /// be interpreted. The canonical text preview is an owner-bound Store
        /// read, so the unsupported envelope can remain available in details.
        var visibleText: String? {
            if canonicalKind == "ui_action" { return uiActionSummary }
            return decodedText ?? (canonicalKind == "text" ? canonicalText?.text : nil)
        }
        var statusLabel: String {
            let label = String(localized: "Historical content", table: "Chat")
            return "\(label) · \(canonicalKind) · \(status.state.rawValue)" + ((status.code ?? status.reason).map { " · \($0)" } ?? "") + (canonicalText.map { "\n" + $0.text } ?? "")
        }
    }

    struct HistoricalPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let projection: HistoricalProjection
        private enum CodingKeys: String, CodingKey { case revision, projection; case partID = "part_id" }
    }

    enum Part: Decodable, Equatable, Sendable {
        case historical(HistoricalPart)
        case text(TextPart)
        case reasoning(ReasoningPart)
        case tool(ToolPart)
        case source(SourcePart)
        case citation(CitationPart)
        case attachment(AttachmentPart)
        case uiSurface(AppleUISurfacePart)
        case uiAction(AppleUIActionPart<MacCanonicalJSONValue>)
        case task(TaskPart)
        case skill(SkillPart)
        case modelAttempt(ModelAttemptPart)
        case compaction(CompactionPart)
        case error(ErrorPart)

        private enum Kind: String, Decodable {
            case historical, text, reasoning, tool, source, citation, attachment, task, skill, compaction, error
            case modelAttempt = "model_attempt"
            case uiSurface = "ui_surface"
            case uiAction = "ui_action"
        }

        private enum CodingKeys: String, CodingKey { case kind }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            switch try values.decode(Kind.self, forKey: .kind) {
            case .historical: self = .historical(try HistoricalPart(from: decoder))
            case .text: self = .text(try TextPart(from: decoder))
            case .reasoning: self = .reasoning(try ReasoningPart(from: decoder))
            case .tool: self = .tool(try ToolPart(from: decoder))
            case .source: self = .source(try SourcePart(from: decoder))
            case .citation: self = .citation(try CitationPart(from: decoder))
            case .attachment: self = .attachment(try AttachmentPart(from: decoder))
            case .uiSurface: self = .uiSurface(try AppleUISurfacePart(from: decoder))
            case .uiAction: self = .uiAction(try AppleUIActionPart(from: decoder))
            case .task: self = .task(try TaskPart(from: decoder))
            case .skill: self = .skill(try SkillPart(from: decoder))
            case .modelAttempt: self = .modelAttempt(try ModelAttemptPart(from: decoder))
            case .compaction: self = .compaction(try CompactionPart(from: decoder))
            case .error: self = .error(try ErrorPart(from: decoder))
            }
        }

        var partID: String {
            switch self {
            case .historical(let value): value.partID
            case .text(let value): value.partID
            case .reasoning(let value): value.partID
            case .tool(let value): value.partID
            case .source(let value): value.partID
            case .citation(let value): value.partID
            case .attachment(let value): value.partID
            case .uiSurface(let value): value.partID
            case .uiAction(let value): value.partID
            case .task(let value): value.partID
            case .skill(let value): value.partID
            case .modelAttempt(let value): value.partID
            case .compaction(let value): value.partID
            case .error(let value): value.partID
            }
        }
    }

    struct RunActivity: Decodable, Equatable, Sendable {
        let runID: String
        let state: ActivityState
        let activePartID: String?
        let displayKey: String?
        let progress: Double?

        private enum CodingKeys: String, CodingKey {
            case state, progress
            case runID = "run_id"
            case activePartID = "active_part_id"
            case displayKey = "display_key"
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
    }

    struct ContextUsage: Codable, Equatable, Sendable {
        var conversationCacheUsage: MacConversationCacheUsage? = nil
        var cacheUsage: MacPromptCacheUsage? = nil
        var measurement: MacContextMeasurement? = nil
        var promptCacheMeasurement: MacContextMeasurement? = nil
        var promptCacheDiagnostic: MacPromptCacheDiagnostic? = nil
        let usedTokens: UInt64
        let windowTokens: UInt64
        let promptTokenLimit: UInt64
        let estimated: Bool
        let measuredAtRevision: UInt64

        private enum CodingKeys: String, CodingKey {
            case cacheUsage = "cache_usage"
            case conversationCacheUsage = "conversation_cache_usage"
            case measurement
            case promptCacheMeasurement = "prompt_cache_measurement"
            case promptCacheDiagnostic = "prompt_cache_diagnostic"
            case estimated
            case usedTokens = "used_tokens"
            case windowTokens = "context_window_tokens"
            case promptTokenLimit = "prompt_token_limit"
            case measuredAtRevision = "measured_at_revision"
        }
    }

    private enum CodingKeys: String, CodingKey {
        case revision, title, messages
        case contractVersion = "contract_version"
        case conversationID = "conversation_id"
        case activeRun = "active_run"
        case surface
        case contextUsage = "context_usage"
    }
}

/// A projection and the journal position from the same canonical read.
nonisolated struct MacCanonicalConversationSnapshot: Sendable {
    let projection: MacCanonicalConversationProjectionV1
    let sequence: UInt64
}

/// History position and event position are separate. Reading a window does
/// not acknowledge events on any existing subscription.
nonisolated struct MacCanonicalConversationWindow: Decodable, Sendable {
    struct DeferredTextPart: Codable, Hashable, Sendable {
        let messageID: String
        let partID: String
        let isFinal: Bool
        let canReadOriginal: Bool
        let totalUTF8Bytes: UInt64
        let previewUTF8Bytes: UInt64
        private enum CodingKeys: String, CodingKey, CaseIterable {
            case messageID = "message_id", partID = "part_id"
            case isFinal = "is_final"
            case canReadOriginal = "can_read_original"
            case totalUTF8Bytes = "total_utf8_bytes", previewUTF8Bytes = "preview_utf8_bytes"
        }
        private struct AnyKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }

        init(from decoder: Decoder) throws {
            let keys = try decoder.container(keyedBy: AnyKey.self)
            guard Set(keys.allKeys.map(\.stringValue)) == Set(CodingKeys.allCases.map(\.rawValue)) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Deferred text requires its exact canonical metadata fields"))
            }
            let values = try decoder.container(keyedBy: CodingKeys.self)
            messageID = try values.decode(String.self, forKey: .messageID)
            partID = try values.decode(String.self, forKey: .partID)
            isFinal = try values.decode(Bool.self, forKey: .isFinal)
            canReadOriginal = try values.decode(Bool.self, forKey: .canReadOriginal)
            totalUTF8Bytes = try values.decode(UInt64.self, forKey: .totalUTF8Bytes)
            previewUTF8Bytes = try values.decode(UInt64.self, forKey: .previewUTF8Bytes)
            guard !messageID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DecodingError.dataCorruptedError(forKey: .messageID, in: values,
                    debugDescription: "Deferred text requires a canonical message identity")
            }
            guard !partID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DecodingError.dataCorruptedError(forKey: .partID, in: values,
                    debugDescription: "Deferred text requires a canonical Part identity")
            }
            guard totalUTF8Bytes > previewUTF8Bytes else {
                throw DecodingError.dataCorruptedError(forKey: .previewUTF8Bytes, in: values,
                    debugDescription: "Deferred text preview must be shorter than its original body")
            }
            guard !canReadOriginal || isFinal else {
                throw DecodingError.dataCorruptedError(forKey: .canReadOriginal, in: values,
                    debugDescription: "Unversioned original text requires a final Part")
            }
        }
    }
    struct StreamOwner: Codable, Equatable, Sendable {
        enum Execution: Codable, Equatable, Sendable {
            case modelAttempt(attemptID: String)
            case delegatedInvocation(invocationID: String)

            private enum CodingKeys: String, CodingKey {
                case type
                case attemptID = "attempt_id"
                case invocationID = "invocation_id"
            }

            init(from decoder: Decoder) throws {
                let values = try decoder.container(keyedBy: CodingKeys.self)
                switch try values.decode(String.self, forKey: .type) {
                case "model_attempt":
                    guard !values.contains(.invocationID) else {
                        throw DecodingError.dataCorruptedError(forKey: .invocationID, in: values,
                            debugDescription: "Model ownership cannot contain invocation identity")
                    }
                    self = .modelAttempt(attemptID: try values.decode(String.self, forKey: .attemptID))
                case "delegated_invocation":
                    guard !values.contains(.attemptID) else {
                        throw DecodingError.dataCorruptedError(forKey: .attemptID, in: values,
                            debugDescription: "Delegated ownership cannot contain attempt identity")
                    }
                    self = .delegatedInvocation(invocationID: try values.decode(String.self, forKey: .invocationID))
                default:
                    throw DecodingError.dataCorruptedError(forKey: .type, in: values,
                        debugDescription: "Unknown execution owner")
                }
            }

            func encode(to encoder: Encoder) throws {
                var values = encoder.container(keyedBy: CodingKeys.self)
                switch self {
                case .modelAttempt(let attemptID):
                    try values.encode("model_attempt", forKey: .type)
                    try values.encode(attemptID, forKey: .attemptID)
                case .delegatedInvocation(let invocationID):
                    try values.encode("delegated_invocation", forKey: .type)
                    try values.encode(invocationID, forKey: .invocationID)
                }
            }
        }

        let messageID: String
        let partID: String
        let runID: String
        let execution: Execution

        private enum CodingKeys: String, CodingKey {
            case messageID = "message_id"
            case partID = "part_id"
            case runID = "run_id"
            case execution
        }
    }

    let projection: MacCanonicalConversationProjectionV1
    let streamOwners: [StreamOwner]?
    let assistantJournalHeads: [MacAssistantJournalCursor]
    let eventSequence: UInt64
    let totalMessageCount: UInt64
    let beforeOrdinal: Int64?
    let hasMore: Bool
    let deferredTextParts: [DeferredTextPart]

    private enum CodingKeys: String, CodingKey {
        case projection
        case streamOwners = "stream_owners"
        case assistantJournalHeads = "assistant_journal_heads"
        case eventSequence = "event_sequence"
        case totalMessageCount = "total_message_count"
        case beforeOrdinal = "before_ordinal"
        case hasMore = "has_more"
        case deferredTextParts = "deferred_text_parts"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        projection = try values.decode(MacCanonicalConversationProjectionV1.self, forKey: .projection)
        // Null means unavailable, not an empty ownership set. A missing field
        // indicates an incompatible producer rather than a guessed fallback.
        streamOwners = try values.decode([StreamOwner]?.self, forKey: .streamOwners)
        assistantJournalHeads = try values.decode([MacAssistantJournalCursor].self, forKey: .assistantJournalHeads)
        eventSequence = try values.decode(UInt64.self, forKey: .eventSequence)
        totalMessageCount = try values.decode(UInt64.self, forKey: .totalMessageCount)
        beforeOrdinal = try values.decodeIfPresent(Int64.self, forKey: .beforeOrdinal)
        hasMore = try values.decode(Bool.self, forKey: .hasMore)
        deferredTextParts = try values.decode([DeferredTextPart].self, forKey: .deferredTextParts)
    }
}

nonisolated struct MacCanonicalTextRange: Decodable, Sendable {
    let conversationID: String
    let messageID: String
    let partID: String
    let offsetBytes: UInt64
    let nextOffsetBytes: UInt64
    let totalUTF8Bytes: UInt64
    let text: String
    let endOfContent: Bool
    private enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
        case offsetBytes = "offset_bytes", nextOffsetBytes = "next_offset_bytes"
        case totalUTF8Bytes = "total_utf8_bytes", text, endOfContent = "end_of_content"
    }
}

/// Core-owned session read model. Projection payloads remain encoded here so
/// platform presentation types do not become part of the Store ABI.
nonisolated struct MacCanonicalSessionProjection: Decodable, Sendable {
    let conversationID: String
    let source: String
    let sourceHostID: String?
    let storeRevision: Int64
    let coreRevision: UInt64
    let eventCursor: UInt64
    let capabilityCatalogRevision: UInt64
    let checkpointCreatedAtMs: UInt64
    let conversationProjection: Data
    let taskProjection: Data
    let goalProjection: Data
    let memoryProjection: Data
    let memoryRuntimeProjection: Data
    let skillRuntimeProjection: Data
    let permissionProjection: Data
    let permissionRuleSet: Data
    let contextUsage: MacCanonicalContextUsage

    private enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id"
        case source
        case sourceHostID = "source_host_id"
        case storeRevision = "store_revision"
        case coreRevision = "core_revision"
        case eventCursor = "event_cursor"
        case capabilityCatalogRevision = "capability_catalog_revision"
        case checkpointCreatedAtMs = "checkpoint_created_at_ms"
        case conversationProjection = "conversation_projection"
        case taskProjection = "task_projection"
        case goalProjection = "goal_projection"
        case memoryProjection = "memory_projection"
        case memoryRuntimeProjection = "memory_runtime_projection"
        case skillRuntimeProjection = "skill_runtime_projection"
        case permissionProjection = "permission_projection"
        case permissionRuleSet = "permission_rule_set"
        case contextUsage = "context_usage"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        conversationID = try values.decode(String.self, forKey: .conversationID)
        source = try values.decode(String.self, forKey: .source)
        sourceHostID = try values.decodeIfPresent(String.self, forKey: .sourceHostID)
        storeRevision = try values.decode(Int64.self, forKey: .storeRevision)
        coreRevision = try values.decode(UInt64.self, forKey: .coreRevision)
        eventCursor = try values.decode(UInt64.self, forKey: .eventCursor)
        capabilityCatalogRevision = try values.decode(UInt64.self, forKey: .capabilityCatalogRevision)
        checkpointCreatedAtMs = try values.decode(UInt64.self, forKey: .checkpointCreatedAtMs)
        conversationProjection = try Self.encodedJSON(values, key: .conversationProjection)
        taskProjection = try Self.encodedJSON(values, key: .taskProjection)
        goalProjection = try Self.encodedJSON(values, key: .goalProjection)
        memoryProjection = try Self.encodedJSON(values, key: .memoryProjection)
        memoryRuntimeProjection = try Self.encodedJSON(values, key: .memoryRuntimeProjection)
        skillRuntimeProjection = try Self.encodedJSON(values, key: .skillRuntimeProjection)
        permissionProjection = try Self.encodedJSON(values, key: .permissionProjection)
        permissionRuleSet = try Self.encodedJSON(values, key: .permissionRuleSet)
        contextUsage = try values.decode(MacCanonicalContextUsage.self, forKey: .contextUsage)
    }

    private static func encodedJSON(
        _ values: KeyedDecodingContainer<CodingKeys>,
        key: CodingKeys
    ) throws -> Data {
        let value = try values.decode(MacCanonicalJSONValue.self, forKey: key)
        return try JSONEncoder().encode(value)
    }

}

nonisolated struct MacCanonicalContextUsage: Codable, Equatable, Sendable {
    let usedTokens: Int
    let contextWindowTokens: Int
    let promptTokenLimit: Int
    let usagePercent: Double
    let usageIsEstimated: Bool

    private enum CodingKeys: String, CodingKey {
        case usedTokens = "used_tokens"
        case contextWindowTokens = "context_window_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case usagePercent = "usage_percent"
        case usageIsEstimated = "usage_is_estimated"
    }
}

nonisolated struct MacCanonicalSkillRuntimeProjection: Codable, Sendable {
    let sessionID: String
    let cursor: UInt64
    let revision: UInt64
    let catalogRevision: UInt64
    let order: [String]
    let runs: [String: MacCoreSkillRun]

    private enum CodingKeys: String, CodingKey {
        case cursor, revision, order, runs
        case sessionID = "session_id"
        case catalogRevision = "catalog_revision"
    }

    var presentationProjection: MacCoreSkillRuntimeProjection {
        MacCoreSkillRuntimeProjection(
            revision: revision,
            lastCursor: cursor,
            lastEventRevision: nil,
            catalogRevision: catalogRevision,
            order: order,
            runs: runs
        )
    }
}

nonisolated enum MacCanonicalJSONValue: Codable, Hashable, Sendable {
    /// Dynamic protocol payloads are data, not an unbounded recursive program.
    /// Keep their nesting below Swift's cooperative-thread stack budget so a
    /// malformed Host snapshot becomes a decoding error instead of SIGBUS.
    static let maximumNestingDepth = 64

    case object([String: MacCanonicalJSONValue])
    case array([MacCanonicalJSONValue])
    case string(String)
    case number(Double)
    // Preserve Core's i64/u64 values beyond binary64's exact integer range.
    // Ordinary small integers retain the existing numeric representation.
    case signedInteger(Int64)
    case unsignedInteger(UInt64)
    case bool(Bool)
    case null

    nonisolated enum FoundationValueError: Error {
        case unsupportedType(String)
        case excessiveNestingDepth(maximum: Int)
    }

    /// Converts an object that has already passed `JSONSerialization` into the
    /// canonical application value without decoding the same JSON document a
    /// second time. Transport envelopes use this boundary so an application
    /// payload cannot invalidate otherwise valid protocol metadata.
    init(foundationValue value: Any) throws {
        self = try Self.convertFoundationValue(value, depth: 0)
    }

    private static func convertFoundationValue(
        _ value: Any,
        depth: Int
    ) throws -> Self {
        guard depth <= maximumNestingDepth else {
            throw FoundationValueError.excessiveNestingDepth(maximum: maximumNestingDepth)
        }
        switch value {
        case is NSNull:
            return .null
        case let value as String:
            return .string(value)
        case let value as NSNumber:
            if CFGetTypeID(value) == CFBooleanGetTypeID() {
                return .bool(value.boolValue)
            } else if let integer = Int64(value.stringValue) {
                return Self.integer(integer)
            } else if let integer = UInt64(value.stringValue) {
                return Self.integer(integer)
            } else {
                return .number(value.doubleValue)
            }
        case let value as Bool:
            return .bool(value)
        case let value as [Any]:
            return .array(try value.map {
                try convertFoundationValue($0, depth: depth + 1)
            })
        case let value as [String: Any]:
            return .object(try value.mapValues {
                try convertFoundationValue($0, depth: depth + 1)
            })
        default:
            throw FoundationValueError.unsupportedType(String(reflecting: type(of: value)))
        }
    }

    var displayString: String {
        if case let .string(value) = self {
            return value
        }
        // This text participates in presentation equality (tool arguments and
        // results). Dictionary enumeration is not a content revision: equal
        // canonical values must not trigger a transcript rebuild on reread.
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(self),
              let value = String(data: data, encoding: .utf8) else {
            return ""
        }
        return value
    }

    private static func integer(_ value: Int64) -> Self {
        if (-9_007_199_254_740_991...9_007_199_254_740_991).contains(value) {
            return .number(Double(value))
        }
        return .signedInteger(value)
    }

    private static func integer(_ value: UInt64) -> Self {
        if value <= UInt64(Int64.max) { return integer(Int64(value)) }
        return .unsignedInteger(value)
    }

    init(from decoder: Decoder) throws {
        let codingPath = decoder.codingPath
        guard codingPath.count <= Self.maximumNestingDepth else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: codingPath,
                debugDescription: "Dynamic JSON exceeds the maximum nesting depth of \(Self.maximumNestingDepth)."
            ))
        }
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Int64.self) { self = Self.integer(value) }
        else if let value = try? container.decode(UInt64.self) { self = Self.integer(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([String: Self].self) { self = .object(value) }
        else { self = .array(try container.decode([Self].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .signedInteger(let value): try container.encode(value)
        case .unsignedInteger(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

nonisolated struct MacCanonicalMessagePartInput: Codable, Equatable, Sendable {
    let id: String
    let kind: String
    let ordinal: Int64
    let state: String
    let contentText: String
    let contentJSON: String?
    let contentDigest: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64
}

nonisolated struct MacCanonicalMessageInput: Codable, Equatable, Sendable {
    let id: String
    let runID: String?
    let role: String
    let status: String
    let ordinal: Int64
    let createdAtMs: Int64
    let completedAtMs: Int64?
    let parts: [MacCanonicalMessagePartInput]
}

nonisolated struct MacCanonicalCreateConversationInput: Codable, Equatable, Sendable {
    let id: String
    let title: String
    let settingsJSON: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64
}

nonisolated struct MacCanonicalConversationCreation: Codable, Equatable, Sendable {
    let created: Bool
    let initialConfigurationRevision: Int64?
    enum CodingKeys: String, CodingKey {
        case created
        case initialConfigurationRevision = "initial_configuration_revision"
    }
}

nonisolated struct MacCanonicalImportConversationTranscriptInput: Codable, Equatable, Sendable {
    let importID: String
    let source: String
    let conversationID: String
    let expectedMessageCount: Int
    let messages: [MacCanonicalMessageInput]
    let importedAtMs: Int64
}

nonisolated struct MacCanonicalTranscriptImportResult: Codable, Equatable, Sendable {
    let appendedMessageCount: Int
    let messageCount: Int
    let conversationRevision: Int64

    private enum CodingKeys: String, CodingKey {
        case appendedMessageCount = "appended_message_count"
        case messageCount = "message_count"
        case conversationRevision = "conversation_revision"
    }
}

nonisolated struct MacCanonicalContextTokenUsage: Codable, Equatable, Sendable {
    let estimated: Int64
    let measured: Int64?
    let window: Int64
    let promptLimit: Int64

    private enum CodingKeys: String, CodingKey {
        case estimated
        case measured
        case window
        case promptLimit = "prompt_limit"
    }
}

nonisolated struct MacCanonicalContextCheckpoint: Codable, Equatable, Sendable {
    let id: String
    let conversationID: String
    let runID: String?
    let modelProfileID: String
    let policyVersion: String
    let coveredThroughMessageID: String?
    let coveredThroughSequence: Int64?
    let summaryPartID: String?
    let retainedTailStartMessageID: String?
    let tokenUsage: MacCanonicalContextTokenUsage
    let payloadDigest: String
    let createdAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id
        case conversationID = "conversation_id"
        case runID = "run_id"
        case modelProfileID = "model_profile_id"
        case policyVersion = "policy_version"
        case coveredThroughMessageID = "covered_through_message_id"
        case coveredThroughSequence = "covered_through_sequence"
        case summaryPartID = "summary_part_id"
        case retainedTailStartMessageID = "retained_tail_start_message_id"
        case tokenUsage = "token_usage"
        case payloadDigest = "payload_digest"
        case createdAtMs = "created_at_ms"
    }
}

nonisolated struct MacCanonicalContextContentReference: Codable, Equatable, Sendable {
    let type: String
    let text: String?
    let truncated: Bool?
    let artifactID: String?
    let blobID: String?

    private enum CodingKeys: String, CodingKey {
        case type
        case text
        case truncated
        case artifactID = "artifact_id"
        case blobID = "blob_id"
    }
}

nonisolated struct MacCanonicalContextProvenance: Codable, Equatable, Sendable {
    let providerID: String
    let collectionMethod: String
    let sourceRevision: String?
    let disclosedToModel: Bool

    private enum CodingKeys: String, CodingKey {
        case providerID = "provider_id"
        case collectionMethod = "collection_method"
        case sourceRevision = "source_revision"
        case disclosedToModel = "disclosed_to_model"
    }
}

nonisolated struct MacCanonicalContextComponent: Codable, Equatable, Sendable {
    let id: String
    let checkpointID: String
    let kind: String
    let sourceID: String?
    let sourceURI: String?
    let title: String?
    let mimeType: String?
    let authorization: String
    let observedAtMs: Int64?
    let maximumAgeMinutes: Int64?
    let freshness: String
    let contentReference: MacCanonicalContextContentReference
    let contentDigest: String
    let estimatedTokens: Int64
    let provenance: MacCanonicalContextProvenance

    private enum CodingKeys: String, CodingKey {
        case id
        case checkpointID = "checkpoint_id"
        case kind
        case sourceID = "source_id"
        case sourceURI = "source_uri"
        case title
        case mimeType = "mime_type"
        case authorization
        case observedAtMs = "observed_at_ms"
        case maximumAgeMinutes = "maximum_age_minutes"
        case freshness
        case contentReference = "content_reference"
        case contentDigest = "content_digest"
        case estimatedTokens = "estimated_tokens"
        case provenance
    }
}

nonisolated struct MacCanonicalContextCommit: Codable, Equatable, Sendable {
    let checkpoint: MacCanonicalContextCheckpoint
    let components: [MacCanonicalContextComponent]
}

nonisolated struct MacCanonicalContextAggregate: Codable, Equatable, Sendable {
    let checkpoint: MacCanonicalContextCheckpoint
    let components: [MacCanonicalContextComponent]
}

nonisolated private struct ContextCommitCommand: Encodable {
    nonisolated struct Inner: Encodable {
        let type: String
        let input: MacCanonicalContextCommit
    }

    let type: String
    let command: Inner
}

nonisolated private struct ContextLoadCommand: Encodable {
    nonisolated struct Inner: Encodable {
        let type: String
        let conversationID: String

        private enum CodingKeys: String, CodingKey {
            case type
            case conversationID = "conversation_id"
        }
    }

    let type: String
    let command: Inner
}

nonisolated struct MacCanonicalPatchConversationMetadataInput: Codable, Equatable, Sendable {
    let conversationID: String
    let expectedRevision: Int64
    let patchJSON: String
    let updatedAtMs: Int64
}

nonisolated struct MacConfigurationCommitResult: Decodable, Sendable {
    enum Status: String, Decodable, Equatable, Sendable { case committed, conflict, notFound = "not_found" }
    let status: Status
    let snapshot: [String: MacCanonicalJSONValue]?
    let expected_revision: Int64?
    let actual_revision: Int64?
    let conversation_id: String?

    func committedSnapshot() throws -> [String: MacCanonicalJSONValue] {
        guard status == .committed, let snapshot else {
            throw MacApplicationPortError.rejected(status == .conflict
                ? "Conversation configuration changed. Your edit is retained; reload before saving."
                : "Conversation is unavailable.")
        }
        return snapshot
    }
}

nonisolated struct MacCanonicalConversationMetadata: Codable, Equatable, Sendable {
    let id: String
    let revision: Int64
    var configurationRevision: Int64? = nil
    let settingsJSON: String?

    private enum CodingKeys: String, CodingKey {
        case id, revision
        case configurationRevision = "configuration_revision"
        case settingsJSON = "settings_json"
    }
}

nonisolated struct MacCanonicalConversationSummary: Codable, Equatable, Sendable {
    let id: String
    let title: String
    let revision: Int64
    let messageCount: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id, title, revision
        case messageCount = "message_count"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated enum MacCanonicalPermissionGrantTarget: Codable, Equatable, Sendable {
    case userGlobal
    case host(id: String)
    case conversation(id: String)

    private enum CodingKeys: String, CodingKey {
        case type
        case hostID = "host_id"
        case conversationID = "conversation_id"
    }

    private enum Kind: String, Codable {
        case userGlobal = "user_global"
        case host
        case conversation
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(Kind.self, forKey: .type) {
        case .userGlobal:
            self = .userGlobal
        case .host:
            self = .host(id: try container.decode(String.self, forKey: .hostID))
        case .conversation:
            self = .conversation(id: try container.decode(String.self, forKey: .conversationID))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .userGlobal:
            try container.encode(Kind.userGlobal, forKey: .type)
        case .host(let id):
            try container.encode(Kind.host, forKey: .type)
            try container.encode(id, forKey: .hostID)
        case .conversation(let id):
            try container.encode(Kind.conversation, forKey: .type)
            try container.encode(id, forKey: .conversationID)
        }
    }
}

nonisolated enum MacCanonicalPermissionDecision: String, Codable, Sendable {
    case allow
    case deny
}

nonisolated struct MacCanonicalPermissionGrantInput: Codable, Sendable {
    let id: String
    let target: MacCanonicalPermissionGrantTarget
    let capabilityPattern: String
    let resourcePattern: String
    let decision: MacCanonicalPermissionDecision
    let createdAtMs: Int64
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case id, target, decision
        case capabilityPattern = "capability_pattern"
        case resourcePattern = "resource_pattern"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacCanonicalPermissionGrant: Codable, Sendable {
    let id: String
    let target: MacCanonicalPermissionGrantTarget
    let capabilityPattern: String
    let resourcePattern: String
    let decision: MacCanonicalPermissionDecision
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let revokedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, target, decision
        case capabilityPattern = "capability_pattern"
        case resourcePattern = "resource_pattern"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case revokedAtMs = "revoked_at_ms"
    }
}

nonisolated struct MacCanonicalPermissionRuleSet: Codable, Sendable {
    let rules: [MacCanonicalPermissionRule]
}

nonisolated struct MacCanonicalPermissionRule: Codable, Sendable {
    let capabilityPattern: String
    let resourcePattern: String
    let decision: String
    let source: String

    private enum CodingKeys: String, CodingKey {
        case decision, source
        case capabilityPattern = "capability_pattern"
        case resourcePattern = "resource_pattern"
    }
}

nonisolated struct MacCanonicalMCPServerInput: Codable, Sendable {
    let id: String
    let displayName: String
    let enabled: Bool
    let configurationJSON: String
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, enabled, revision
        case displayName = "display_name"
        case configurationJSON = "configuration_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct MacCanonicalMCPServer: Codable, Equatable, Sendable {
    let id: String
    let displayName: String
    let enabled: Bool
    let configurationJSON: String
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, enabled, revision
        case displayName = "display_name"
        case configurationJSON = "configuration_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

private nonisolated struct MacSkillCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "skill"
    let command: Payload
}

private nonisolated struct MacUpsertSkillCommand: Encodable {
    let type = "upsert"
    let input: MacCanonicalSkillInput
}

private nonisolated struct MacListSkillsCommand: Encodable {
    let type: String
}

private nonisolated struct MacMCPCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "mcp"
    let command: Payload
}

private nonisolated struct MacListMCPCommand: Encodable {
    let type: String
}

private nonisolated struct MacDataSourceCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "data_source"
    let command: Payload
}

private nonisolated struct MacListDataSourceCommand: Encodable { let type: String }

private nonisolated struct MacCreateDataSourceDraftCommand: Encodable {
    let type = "create_draft"
    let input: MacCanonicalDataSourceDraft
}

private nonisolated struct MacUpdateDataSourceConfigurationCommand: Encodable {
    let type = "update_configuration"
    let id: String
    let expectedRevision: Int64
    let input: MacCanonicalDataSourceConfigurationUpdate

    private enum CodingKeys: String, CodingKey {
        case type, id, input
        case expectedRevision = "expected_revision"
    }
}

private nonisolated struct MacRequestDataSourceValidationCommand: Encodable {
    let type = "request_validation"
    let id: String
    let expectedRevision: Int64
    let requestedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case type, id
        case expectedRevision = "expected_revision"
        case requestedAtMs = "requested_at_ms"
    }
}

private nonisolated struct MacDataSourceValidationSuccess: Encodable {
    let status = "succeeded"
    let contract: [String: MacCoreDataSourceJSON]
}

private nonisolated struct MacDataSourceValidationFailure: Encodable {
    let status = "failed"
    let code: String
    let diagnostic: String
}

private nonisolated struct MacRecordDataSourceValidationCommand<Result: Encodable>: Encodable {
    let type = "record_validation"
    let id: String
    let validationToken: String
    let result: Result
    let recordedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case type, id, result
        case validationToken = "validation_token"
        case recordedAtMs = "recorded_at_ms"
    }
}

private nonisolated struct MacSetDataSourceDefaultEnabledCommand: Encodable {
    let type = "set_default_enabled"
    let id: String
    let expectedRevision: Int64
    let enabled: Bool
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case type, id, enabled
        case expectedRevision = "expected_revision"
        case updatedAtMs = "updated_at_ms"
    }
}

private nonisolated struct MacGetDataSourceCommand: Encodable {
    let type = "get"
    let id: String
}

private nonisolated struct MacMemoryCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "memory"
    let command: Payload
}

private nonisolated struct MacModelCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "model"
    let command: Payload
}

private nonisolated struct MacConversationPlanCommandEnvelope<Payload: Encodable>: Encodable {
    let type = "conversation_plan"
    let command: Payload
}

private nonisolated struct MacEffectivePermissionGrantsCommand: Encodable {
    let type: String
    let conversationID: String?
    let hostID: String?

    private enum CodingKeys: String, CodingKey {
        case type
        case conversationID = "conversation_id"
        case hostID = "host_id"
    }
}

private nonisolated struct MacRevokePermissionGrantCommand: Encodable {
    let type: String
    let id: String
    let revokedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case type, id
        case revokedAtMs = "revoked_at_ms"
    }
}

nonisolated struct MacCanonicalModelRuntime: Codable, Equatable, Sendable {
    let hostID: String
    let profileID: String
    let phase: String
    let endpointJSON: String?
    let processID: Int64?
    let error: String?
    let startedAtMs: Int64?
    let stoppedAtMs: Int64?
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case phase, error
        case hostID = "host_id"
        case profileID = "profile_id"
        case endpointJSON = "endpoint_json"
        case processID = "process_id"
        case startedAtMs = "started_at_ms"
        case stoppedAtMs = "stopped_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct MacPrepareRemoteSubmissionInput: Encodable, Sendable {
    let conversationID: String
    let conversationTitle: String
    let submissionID: String
    let sourceHostID: String
    let clientID: String
    let userMessageID: String
    let userPartID: String
    let userText: String
    let userContentJSON: String?
    let assistantMessageID: String
    let assistantPartID: String
    let contextCheckpointID: String?
    let createdAtMs: Int64
}

/// Non-secret exact Run identity returned by one authoritative Application read.
/// Missing witness keys are a protocol error; explicit null means that the Run
/// has not persisted a checkpoint yet.
nonisolated struct MacExactRunInvocation: Decodable, Sendable {
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let invocationConfigDigest: String
    let status: String
    let hostInvocation: MacCanonicalJSONValue
    let coreSessionID: String?
    let checkpointRevision: Int64?
    let checkpointEventCursor: Int64?
    let checkpointSHA256: String?

    private enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id"
        case runID = "run_id"
        case assistantMessageID = "assistant_message_id"
        case invocationConfigDigest = "invocation_config_digest"
        case status
        case hostInvocation = "host_invocation"
        case coreSessionID = "core_session_id"
        case checkpointRevision = "checkpoint_revision"
        case checkpointEventCursor = "checkpoint_event_cursor"
        case checkpointSHA256 = "checkpoint_sha256"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        conversationID = try values.decode(String.self, forKey: .conversationID)
        runID = try values.decode(String.self, forKey: .runID)
        assistantMessageID = try values.decode(String.self, forKey: .assistantMessageID)
        invocationConfigDigest = try values.decode(String.self, forKey: .invocationConfigDigest)
        status = try values.decode(String.self, forKey: .status)
        hostInvocation = try values.decode(MacCanonicalJSONValue.self, forKey: .hostInvocation)
        coreSessionID = try values.decode(String?.self, forKey: .coreSessionID)
        checkpointRevision = try values.decode(Int64?.self, forKey: .checkpointRevision)
        checkpointEventCursor = try values.decode(Int64?.self, forKey: .checkpointEventCursor)
        checkpointSHA256 = try values.decode(String?.self, forKey: .checkpointSHA256)
        let absent = coreSessionID == nil && checkpointRevision == nil
            && checkpointEventCursor == nil && checkpointSHA256 == nil
        let complete = coreSessionID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            && checkpointRevision.map { $0 >= 0 } == true
            && checkpointEventCursor.map { $0 >= 0 } == true
            && checkpointSHA256.map(Self.validDigest) == true
        guard !conversationID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !runID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !assistantMessageID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              Self.validDigest(invocationConfigDigest), absent || complete else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Invalid exact Run invocation or Core checkpoint witness."))
        }
    }

    private static func validDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy {
            (48...57).contains($0) || (97...102).contains($0)
        }
    }
}

nonisolated final class MacApplicationPort:
    MacAutomationApplicationClient,
    MacSkillCatalogApplicationClient,
    MacLocalModelApplicationClient,
    @unchecked Sendable
{
    static let defaultDatabaseURL = MacAppEnvironment.shared.databaseURL
    static let shared = MacApplicationPort(deferredDatabaseURL: defaultDatabaseURL)

    private let lock = NSLock()
    private let serviceClient: MacApplicationServiceClient
    let databaseURL: URL

    init(deferredDatabaseURL: URL) {
        databaseURL = deferredDatabaseURL.standardizedFileURL
        serviceClient = .deferred(databaseURL: databaseURL)
    }

    func prepare() throws {
        try serviceClient.prepare()
    }

    func executeAgentRoomEnvelope(_ envelope: MacCanonicalJSONValue) throws -> MacCanonicalJSONValue {
        return try execute(envelope)
    }

    func preparedGoalNativeRoute(conversationID: String, goalID: String) throws -> Data {
        struct Query: Encodable {
            let type = "prepare_goal_execution"
            let conversationID: String
            let goalID: String
        }
        let response = try serviceClient.execute(commandData: Self.makeEncoder().encode(
            Query(conversationID: conversationID, goalID: goalID)))
        guard let binding = try JSONSerialization.jsonObject(with: response) as? [String: Any],
              let identity = binding["identity"] as? [String: Any],
              identity["conversation_id"] as? String == conversationID, identity["goal_id"] as? String == goalID,
              let template = binding["template"] as? [String: Any],
              let config = template["core_config"] as? [String: Any],
              let turn = config["turn_identity"] as? [String: Any],
              let invocation = turn["host_invocation"] as? [String: Any],
              let primary = invocation["primary"] as? [String: Any], primary["kind"] as? String == "native" else {
            throw MacApplicationPortError.invalidResponse
        }
        var route = primary
        route.removeValue(forKey: "kind")
        return try JSONSerialization.data(withJSONObject: route, options: [.sortedKeys])
    }

    struct GoalControlObservation: Decodable, Sendable {
        let goalID: String
        let controlToken: String
        let status: String
        let canSubmit: Bool
        let canPause: Bool
        let canResume: Bool
        let canCancel: Bool
        enum CodingKeys: String, CodingKey {
            case goalID = "goal_id", controlToken = "control_token", status, canSubmit = "can_submit"
            case canPause = "can_pause", canResume = "can_resume", canCancel = "can_cancel"
        }
    }
    func goalControlObservation(conversationID: String) throws -> GoalControlObservation? {
        struct Query: Encodable { let type = "load_goal_control_observation"; let conversationID: String }
        return try executeOptional(Query(conversationID: conversationID))
    }

    func goalExecutionAssistantIdentity(conversationID: String, goalID: String, runID: String) throws -> String? {
        struct Query: Encodable {
            let type = "load_goal_execution_owner"
            let conversationID: String
            let goalID: String
            let runID: String
        }
        struct Owner: Decodable {
            let goalID: String
            let runID: String
            let contractID: String
            let assistantMessageID: String
            let controlToken: String
            enum CodingKeys: String, CodingKey {
                case goalID = "goal_id", runID = "run_id", contractID = "contract_id"
                case assistantMessageID = "assistant_message_id", controlToken = "control_token"
            }
        }
        let owner: Owner? = try executeOptional(Query(conversationID: conversationID, goalID: goalID, runID: runID))
        guard let owner else { return nil }
        guard owner.goalID == goalID, owner.runID == runID,
              !owner.contractID.isEmpty, !owner.controlToken.isEmpty, !owner.assistantMessageID.isEmpty else {
            throw MacApplicationPortError.invalidResponse
        }
        return owner.assistantMessageID
    }

    func loadExactRunInvocation(
        conversationID: String, expectedRunID: String, assistantMessageID: String
    ) throws -> MacExactRunInvocation? {
        struct Query: Encodable {
            let type = "load_exact_run_invocation"
            let conversationID: String
            let expectedRunID: String
            let assistantMessageID: String
        }
        let result: MacExactRunInvocation? = try executeOptional(Query(
            conversationID: conversationID, expectedRunID: expectedRunID,
            assistantMessageID: assistantMessageID
        ))
        if let result,
           result.conversationID != conversationID || result.runID != expectedRunID
            || result.assistantMessageID != assistantMessageID {
            throw MacApplicationPortError.rejected("Exact Run invocation ownership mismatch.")
        }
        return result
    }

    func archiveIncompatibleStoreAndPrepare() throws -> URL? {
        try serviceClient.archiveIncompatibleStoreAndPrepare()
    }

    init(databaseURL: URL) throws {
        let canonicalDatabaseURL = databaseURL.standardizedFileURL
        self.databaseURL = canonicalDatabaseURL
        serviceClient = try MacApplicationServiceClient(databaseURL: canonicalDatabaseURL)
    }

    func assistantJournalReceptionState(sourceHostID: String, through: MacAssistantJournalCursor) throws -> MacAssistantJournalReceptionState {
        struct Query: Encodable {
            let type = "assistant_journal_reception_state"
            let sourceHostID: String
            let through: MacAssistantJournalCursor
        }
        return try execute(Query(sourceHostID: sourceHostID, through: through))
    }

    func runMessageAdmissionReceptionState(sourceHostID: String,
        through: MacRunMessageAdmissionCursor) throws -> MacRunMessageAdmissionCursor {
        struct Query: Encodable {
            let type = "run_message_admission_reception_state"
            let sourceHostID: String
            let through: MacRunMessageAdmissionCursor
        }
        return try execute(Query(sourceHostID: sourceHostID, through: through))
    }

    func receiveRunMessageAdmissions(sourceHostID: String,
        page: MacRunMessageAdmissionPage) throws -> MacRunMessageAdmissionCursor {
        struct Command: Encodable {
            let type = "receive_run_message_admissions"
            let sourceHostID: String
            let page: MacRunMessageAdmissionPage
        }
        return try execute(Command(sourceHostID: sourceHostID, page: page))
    }

    func acceptRemoteRunContextUsage(sourceHostID: String,
        record: MacCommittedRunContextUsage) throws -> MacCommittedRunContextUsage {
        struct Command: Encodable {
            let type = "accept_remote_run_context_usage"
            let source_host_id: String
            let record: MacCommittedRunContextUsage
        }
        // Context usage contains canonical typed payloads (including the
        // camelCase PromptCacheDiagnostic contract). Do not rename nested keys.
        let data = try JSONEncoder().encode(Command(source_host_id: sourceHostID, record: record))
        let response = try serviceClient.execute(commandData: data)
        return try Self.makeDecoder().decode(MacCommittedRunContextUsage.self, from: response)
    }

    func localRunContextUsage(identity: MacAssistantJournalIdentity) throws -> MacCommittedRunContextUsage? {
        struct Query: Encodable {
            let type = "local_run_context_usage"
            let identity: MacAssistantJournalIdentity
        }
        return try executeOptional(Query(identity: identity))
    }

    func beginReceiveAssistantRunTerminal(sourceHostID: String,
        seal: MacAssistantRunTerminalSeal) throws -> MacAssistantRunTerminalReception {
        struct Command: Encodable {
            let type = "begin_receive_assistant_run_terminal"
            let sourceHostID: String
            let seal: MacAssistantRunTerminalSeal
        }
        return try execute(Command(sourceHostID: sourceHostID, seal: seal))
    }

    func stageAssistantRunTerminalManifest(sourceHostID: String,
        page: MacAssistantRunTerminalManifestPage) throws -> MacAssistantRunTerminalReception {
        struct Command: Encodable {
            let type = "stage_assistant_run_terminal_manifest"
            let sourceHostID: String
            let page: MacAssistantRunTerminalManifestPage
        }
        return try execute(Command(sourceHostID: sourceHostID, page: page))
    }

    func commitReceivedAssistantRunTerminal(sourceHostID: String,
        seal: MacAssistantRunTerminalSeal) throws -> MacAssistantRunTerminalReception {
        struct Command: Encodable {
            let type = "commit_received_assistant_run_terminal"
            let sourceHostID: String
            let seal: MacAssistantRunTerminalSeal
        }
        return try execute(Command(sourceHostID: sourceHostID, seal: seal))
    }

    func openAssistantJournalSnapshot(at: MacAssistantJournalCursor) throws -> MacAssistantJournalSnapshot {
        struct Query: Encodable {
            let type = "open_assistant_journal_snapshot"
            let at: MacAssistantJournalCursor
        }
        return try execute(Query(at: at))
    }

    func localAssistantJournalHead(identity: MacAssistantJournalIdentity) throws -> MacAssistantJournalCursor {
        struct Query: Encodable {
            let type = "local_assistant_journal_head"
            let identity: MacAssistantJournalIdentity
        }
        return try execute(Query(identity: identity))
    }

    func assistantJournalParts(snapshot: MacAssistantJournalSnapshot, afterPartID: String? = nil) throws -> MacAssistantJournalPartPage {
        struct Query: Encodable {
            let type = "read_assistant_journal_parts"
            let snapshot: MacAssistantJournalSnapshot
            let afterPartID: String?
            let maxRecords = 64
            let maxEncodedBytes = 262144
        }
        return try execute(Query(snapshot: snapshot, afterPartID: afterPartID))
    }

    func assistantJournalPartChanges(snapshot: MacAssistantJournalSnapshot, base: MacAssistantJournalCursor,
                                     afterPartID: String? = nil) throws -> MacAssistantJournalPartChangePage {
        struct Query: Encodable {
            let type = "read_assistant_journal_part_changes"
            let snapshot: MacAssistantJournalSnapshot
            let base: MacAssistantJournalCursor
            let afterPartID: String?
            let maxRecords = 64
            let maxEncodedBytes = 262144
        }
        return try execute(Query(snapshot: snapshot, base: base, afterPartID: afterPartID))
    }

    func assistantJournalRange(snapshot: MacAssistantJournalSnapshot, partID: String,
                               content: MacAssistantJournalContentReference, offsetBytes: String) throws -> MacAssistantJournalTextRange {
        struct Query: Encodable {
            let type = "read_assistant_journal_range"
            let snapshot: MacAssistantJournalSnapshot
            let partID: String
            let content: MacAssistantJournalContentReference
            let offsetBytes: String
            let limitBytes = 32768
        }
        return try execute(Query(snapshot: snapshot, partID: partID, content: content, offsetBytes: offsetBytes))
    }

    func releaseAssistantJournalSnapshot(_ snapshot: MacAssistantJournalSnapshot) throws {
        struct Query: Encodable {
            let type = "release_assistant_journal_snapshot"
            let snapshot: MacAssistantJournalSnapshot
        }
        let _: MacCanonicalJSONValue? = try executeOptional(Query(snapshot: snapshot))
    }

    func nextAssistantJournalContent(sourceHostID: String, record: MacAssistantJournalRecord) throws -> MacAssistantJournalContentNeed? {
        struct Query: Encodable {
            let type = "next_assistant_journal_content"
            let sourceHostID: String
            let record: MacAssistantJournalRecord
        }
        return try executeOptional(Query(sourceHostID: sourceHostID, record: record))
    }

    func stageAssistantJournalContent(sourceHostID: String, record: MacAssistantJournalRecord,
                                      need: MacAssistantJournalContentNeed, chunk: MacAssistantJournalChunk) throws {
        struct Input: Encodable {
            let sourceHostID: String
            let record: MacAssistantJournalRecord
            let partID: String
            let content: MacAssistantJournalContentReference
            let expectedPrefix: MacAssistantJournalContentReference
            let chunks: [MacAssistantJournalChunk]
            let restart = false
        }
        let _: MacAssistantJournalContentReference = try execute(Command(type: "stage_assistant_journal_content",
            input: Input(sourceHostID: sourceHostID, record: record, partID: need.part_id,
                         content: need.content, expectedPrefix: need.expected_prefix, chunks: [chunk])))
    }

    func receiveAssistantJournalRecord(sourceHostID: String, record: MacAssistantJournalRecord) throws -> MacAssistantJournalReceipt {
        struct Input: Encodable {
            let type = "receive_assistant_journal_record"
            let sourceHostID: String
            let record: MacAssistantJournalRecord
            let chunks: [MacAssistantJournalChunk] = []
        }
        return try execute(Input(sourceHostID: sourceHostID, record: record))
    }

    func createConversation(
        _ input: MacCanonicalCreateConversationInput
    ) throws -> MacCanonicalConversationCreation {
        try execute(Command(type: "create_conversation", input: input))
    }

    func importConversationTranscript(
        _ input: MacCanonicalImportConversationTranscriptInput
    ) throws -> MacCanonicalTranscriptImportResult {
        try execute(Command(type: "import_conversation_transcript", input: input))
    }

    func archiveDirectoryRevision() throws -> UInt64 {
        try execute(ArchiveDirectoryRevisionCommand(type: "archive_directory_revision"))
    }

    func beginArchiveRestore(_ input: MacBeginArchiveRestore) throws -> MacArchiveRestoreProgress {
        try execute(Command(type: "begin_archive_restore", input: input))
    }

    func stageArchiveManifest(_ input: MacStageArchiveManifest) throws -> MacArchiveRestoreProgress {
        try execute(Command(type: "stage_archive_manifest", input: input))
    }

    func commitArchiveRestore(_ input: MacCommitArchiveRestore) throws -> MacArchiveRestoreResult {
        try execute(Command(type: "commit_archive_restore", input: input))
    }

    private struct ArchiveDirectoryRevisionCommand: Encodable {
        let type: String
    }

    func beginContentUpload(id: String, owner: MacContentUploadOwner, byteCount: UInt64) throws -> MacContentUploadProgress {
        try execute(Command(type: "begin_content_upload", input: ContentUploadInput(
            uploadID: id, owner: owner, expectedUTF8Bytes: byteCount
        )))
    }

    func appendContentChunk(id: String, owner: MacContentUploadOwner, offset: UInt64, text: String) throws -> MacContentUploadProgress {
        try execute(Command(type: "append_content_chunk", input: ContentUploadInput(
            uploadID: id, owner: owner, offsetBytes: offset, text: text
        )))
    }

    func contentUploadProgress(id: String, owner: MacContentUploadOwner) throws -> MacContentUploadProgress {
        try execute(Command(type: "content_upload_progress", input: ContentUploadInput(uploadID: id, owner: owner)))
    }

    func sealContentUpload(id: String, owner: MacContentUploadOwner) throws -> MacContentUploadProgress {
        try execute(Command(type: "seal_content_upload", input: ContentUploadInput(uploadID: id, owner: owner)))
    }

    func readContentUploadChunk(id: String, owner: MacContentUploadOwner, offset: UInt64) throws -> MacContentUploadChunk {
        try execute(Command(type: "read_content_upload_chunk", input: ContentUploadInput(
            uploadID: id, owner: owner, offsetBytes: offset
        )))
    }

    private struct ContentUploadInput: Encodable {
        let uploadID: String
        let owner: MacContentUploadOwner
        var expectedUTF8Bytes: UInt64? = nil
        var offsetBytes: UInt64? = nil
        var text: String? = nil
        enum CodingKeys: String, CodingKey {
            case owner, text
            case uploadID = "upload_id"
            case expectedUTF8Bytes = "expected_utf8_bytes"
            case offsetBytes = "offset_bytes"
        }
    }

    func commitContextCheckpoint(
        _ input: MacCanonicalContextCommit
    ) throws -> MacCanonicalContextAggregate {
        try execute(ContextCommitCommand(
            type: "context",
            command: ContextCommitCommand.Inner(type: "commit_checkpoint", input: input)
        ))
    }

    func countContextCheckpoints(_ conversationID: String) throws -> Int {
        try execute(ContextLoadCommand(
            type: "context",
            command: ContextLoadCommand.Inner(
                type: "count_checkpoints",
                conversationID: conversationID
            )
        ))
    }

    func loadContextCheckpoint(_ checkpointID: String) throws -> MacCanonicalContextAggregate? {
        struct Request: Encodable {
            struct Inner: Encodable {
                let type = "load_checkpoint"
                let checkpointID: String
                enum CodingKeys: String, CodingKey {
                    case type
                    case checkpointID = "checkpoint_id"
                }
            }
            let type = "context"
            let command: Inner
        }
        return try execute(Request(command: Request.Inner(checkpointID: checkpointID)))
    }

    func patchConversationMetadata(
        _ input: MacCanonicalPatchConversationMetadataInput
    ) throws -> Int64? {
        try execute(Command(type: "patch_conversation_metadata", input: input))
    }

    func selectConversationExecution(_ command: SelectConversationExecutionCommand<MacCanonicalJSONValue>) throws -> MacConfigurationCommitResult {
        try execute(command)
    }

    func setConversationPermission(_ command: SetConversationPermissionCommand) throws -> MacConfigurationCommitResult {
        try execute(command)
    }

    func patchConversationConfiguration(_ input: MacCanonicalPatchConversationMetadataInput) throws -> MacConfigurationCommitResult {
        try execute(Command(type: "patch_conversation_configuration", input: input))
    }

    func loadConversation(_ conversationID: String) throws -> MacCanonicalConversation? {
        try execute(LoadConversationCommand(
            type: "load_conversation",
            conversationID: conversationID
        ))
    }

    func loadConversationMetadata(_ conversationID: String) throws -> MacCanonicalConversationMetadata? {
        let metadata: MacCanonicalConversationMetadata? = try execute(LoadConversationCommand(
            type: "load_conversation_metadata", conversationID: conversationID
        ))
        if let metadata, metadata.id != conversationID || metadata.revision < 1 {
            throw MacApplicationPortError.rejected("Invalid conversation metadata identity or revision.")
        }
        return metadata
    }

    func loadConversationExportPage(_ conversationID: String, afterOrdinal: Int64? = nil,
                                    expectedRevision: Int64? = nil) throws -> MacConversationExportPage? {
        struct Request: Encodable {
            let type = "load_conversation_export_page"
            let conversationID: String
            let afterOrdinal: Int64?
            let expectedRevision: Int64?
            enum CodingKeys: String, CodingKey {
                case type, conversationID = "conversation_id", afterOrdinal = "after_ordinal", expectedRevision = "expected_revision"
            }
        }
        let page: MacConversationExportPage? = try execute(Request(conversationID: conversationID,
            afterOrdinal: afterOrdinal, expectedRevision: expectedRevision))
        if let page {
            guard page.conversation.id == conversationID, page.conversation.revision > 0,
                  expectedRevision == nil || page.conversation.revision == expectedRevision,
                  page.messages.count <= 1, page.nextOrdinal == page.messages.last?.ordinal,
                  !page.hasMore || !page.messages.isEmpty,
                  page.messages.allSatisfy({ $0.ordinal > (afterOrdinal ?? 0) && $0.parts.count <= 512 }) else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
        return page
    }

    /// Independent, read-only detail query. Never applies a projection event or receipt.
    func readHistoricalPart(_ owner: MacCanonicalPartOwner,
                            expectedConversationRevision: Int64) throws -> MacCanonicalConversationProjectionV1.HistoricalProjection? {
        struct Request: Encodable {
            let type = "read_historical_part"
            let owner: MacCanonicalPartOwner
            let expectedConversationRevision: Int64
            let maximumOutputBytes = 32768
            enum CodingKeys: String, CodingKey {
                case type, owner
                case expectedConversationRevision = "expected_conversation_revision"
                case maximumOutputBytes = "maximum_output_bytes"
            }
        }
        let request = Request(owner: owner, expectedConversationRevision: expectedConversationRevision)
        let response = try serviceClient.execute(commandData: Self.makeEncoder().encode(request))
        return try Self.decodeHistoricalPart(response, owner: owner, revision: expectedConversationRevision)
    }

    static func decodeHistoricalPart(_ response: Data, owner: MacCanonicalPartOwner,
                                     revision: Int64) throws -> MacCanonicalConversationProjectionV1.HistoricalProjection? {
        guard response.count <= 32768 else { throw MacApplicationServiceError.invalidResponse }
        let value = try Self.makeDecoder().decode(Optional<MacCanonicalConversationProjectionV1.HistoricalProjection>.self, from: response)
        if let value {
            guard value.matches(owner: owner, revision: revision) else { throw MacApplicationServiceError.invalidResponse }
        }
        return value
    }

    func readConversationPartDerivation(_ owner: MacCanonicalPartOwner,
                                        expectedConversationRevision: Int64) throws -> MacStoredPartDerivation? {
        struct Request: Encodable {
            let type = "read_conversation_part_derivation"
            let owner: MacCanonicalPartOwner
            let expectedConversationRevision: Int64
            enum CodingKeys: String, CodingKey {
                case type, owner, expectedConversationRevision = "expected_conversation_revision"
            }
        }
        let record: MacStoredPartDerivation? = try execute(Request(owner: owner,
            expectedConversationRevision: expectedConversationRevision))
        if let record {
            guard record.derivation.target.owner == owner,
                  try JSONEncoder().encode(record).count <= 24 * 1024 else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
        return record
    }

    func readConversationToolSources(_ owner: MacCanonicalPartOwner,
                                     expectedConversationRevision: Int64) throws -> [MacToolSourceClaim] {
        struct Request: Encodable {
            let type = "read_conversation_tool_sources"
            let owner: MacCanonicalPartOwner
            let expectedConversationRevision: Int64
        }
        let claims: [MacToolSourceClaim] = try execute(Request(owner: owner,
            expectedConversationRevision: expectedConversationRevision))
        try MacToolSourceClaim.validate(claims, owner: owner)
        return claims
    }

    func readConversationToolSourceRange(_ owner: MacCanonicalPartOwner, sourceItemID: String,
                                         expectedConversationRevision: Int64, offsetBytes: UInt64) throws -> MacToolSourceExportRange {
        struct Request: Encodable {
            let type = "read_conversation_tool_source_range"
            let owner: MacCanonicalPartOwner
            let sourceItemID: String
            let expectedConversationRevision: Int64
            let offsetBytes: UInt64
            let limitBytes = 32768
        }
        let range: MacToolSourceExportRange = try execute(Request(owner: owner, sourceItemID: sourceItemID,
            expectedConversationRevision: expectedConversationRevision, offsetBytes: offsetBytes))
        let next = offsetBytes.addingReportingOverflow(UInt64(range.text.utf8.count))
        guard range.owner == owner, range.sourceItemID == sourceItemID, MacToolSourceClaim.validDigest(range.bodySHA256),
              range.revision == expectedConversationRevision, range.offsetBytes == offsetBytes,
              !next.overflow, range.nextOffsetBytes == next.partialValue, range.text.utf8.count <= 32768,
              range.nextOffsetBytes <= range.totalUTF8Bytes, range.nextOffsetBytes > offsetBytes || range.endOfContent,
              range.endOfContent == (range.nextOffsetBytes == range.totalUTF8Bytes) else { throw MacApplicationServiceError.invalidResponse }
        return range
    }

    func readConversationExportRange(_ conversationID: String, messageID: String, partID: String,
                                     field: MacExportBodyField, expectedRevision: Int64,
                                     offsetBytes: UInt64) throws -> MacConversationExportRange {
        struct Request: Encodable {
            let type = "read_conversation_export_range"
            let conversationID: String
            let messageID: String
            let partID: String
            let field: MacExportBodyField
            let expectedRevision: Int64
            let offsetBytes: UInt64
            let limitBytes = 32 * 1024
            enum CodingKeys: String, CodingKey {
                case type, field, conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
                case expectedRevision = "expected_revision", offsetBytes = "offset_bytes", limitBytes = "limit_bytes"
            }
        }
        let range: MacConversationExportRange = try execute(Request(conversationID: conversationID,
            messageID: messageID, partID: partID, field: field, expectedRevision: expectedRevision, offsetBytes: offsetBytes))
        let next = offsetBytes.addingReportingOverflow(UInt64(range.text.utf8.count))
        guard range.conversationID == conversationID, range.messageID == messageID,
              range.partID == partID, range.field == field, range.revision == expectedRevision,
              range.offsetBytes == offsetBytes, !next.overflow, range.nextOffsetBytes == next.partialValue,
              range.text.utf8.count <= 32 * 1024, range.nextOffsetBytes <= range.totalUTF8Bytes,
              range.endOfContent == (range.nextOffsetBytes == range.totalUTF8Bytes),
              range.endOfContent || range.nextOffsetBytes > offsetBytes else {
            throw MacApplicationServiceError.invalidResponse
        }
        return range
    }

    func queryCanonicalHistory(_ input: [String: Any]) throws -> [String: Any] {
        struct Command: Encodable {
            let type = "query_canonical_history"
            let input: MacCanonicalJSONValue
        }
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self,
            from: JSONSerialization.data(withJSONObject: input))
        let result: MacCanonicalJSONValue = try execute(Command(input: value))
        guard let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as? [String: Any] else {
            throw MacApplicationPortError.invalidResponse
        }
        return object
    }

    func searchConversationText(
        _ conversationID: String,
        query: String,
        limit: Int
    ) throws -> [MacCanonicalConversationTextMatch] {
        try execute(ConversationTextSearchCommand(
            type: "search_conversation_text", conversationID: conversationID,
            query: query, limit: limit
        ))
    }

    private struct ConversationTextSearchCommand: Encodable {
        let type: String
        let conversationID: String
        let query: String
        let limit: Int
        private enum CodingKeys: String, CodingKey {
            case type, query, limit
            case conversationID = "conversation_id"
        }
    }

    func loadConversationProjectionV1(
        _ conversationID: String
    ) throws -> MacCanonicalConversationProjectionV1 {
        try execute(LoadConversationCommand(
            type: "load_conversation_projection_v1",
            conversationID: conversationID
        ))
    }

    /// Loads the canonical conversation projection through the Application
    /// Service projection boundary. Clients must not reconstruct a logical
    /// snapshot by combining independently paged table reads.
    func loadConversationProjectionSnapshot(
        _ conversationID: String
    ) throws -> MacCanonicalConversationSnapshot {
        let pageSize = 64
        var offset = 0
        var expectedRevision: UInt64?
        var firstProjection: MacCanonicalConversationProjectionV1?
        var messages: [MacCanonicalConversationProjectionV1.Message] = []
        var messageIDs = Set<String>()
        var committedMessageIDs: [String] = []
        var archivedActivityPartIDs: [String] = []
        var liveMessageID: String?
        var currentActivityPartID: String?
        var activeRun: MacCanonicalConversationProjectionV1.RunActivity?
        var showAssistantPlaceholder = false
        var finalCursorSequence: UInt64 = 0
        while true {
            let snapshot: ProjectionSnapshot = try execute(
                ProjectionSnapshotCommand(
                    type: "projection_snapshot",
                    scope: .init(type: "conversation", conversationID: conversationID),
                    page: .init(
                        offset: UInt64(offset),
                        limit: pageSize,
                        expectedRevision: expectedRevision
                    )
                )
            )
            guard let record = snapshot.records.first(where: {
                $0.entityType == "conversation_projection"
                    && $0.entityID == conversationID
            }), let page = snapshot.page else {
                throw MacApplicationServiceError.invalidResponse
            }
            if expectedRevision == nil {
                expectedRevision = snapshot.revision
                firstProjection = record.payload
            } else if snapshot.revision != expectedRevision
                || snapshot.cursor.sequence != finalCursorSequence {
                throw MacApplicationServiceError.invalidResponse
            }
            let projection = record.payload
            guard projection.conversationID == conversationID,
                  projection.revision == firstProjection?.revision,
                  projection.contractVersion == firstProjection?.contractVersion,
                  projection.surface.revision == firstProjection?.surface.revision,
                  projection.contextUsage == firstProjection?.contextUsage,
                  !page.hasMore || !projection.messages.isEmpty else {
                throw MacApplicationServiceError.invalidResponse
            }
            let pageMessageIDs = projection.messages.map(\.messageID)
            let surfaceMessageIDs = projection.surface.committedMessageIDs
                + (projection.surface.liveMessageID.map { [$0] } ?? [])
            guard Set(pageMessageIDs).count == pageMessageIDs.count,
                  Set(surfaceMessageIDs).count == surfaceMessageIDs.count,
                  Set(pageMessageIDs) == Set(surfaceMessageIDs),
                  messageIDs.isDisjoint(with: pageMessageIDs) else {
                throw MacApplicationServiceError.invalidResponse
            }
            messageIDs.formUnion(pageMessageIDs)
            committedMessageIDs.append(contentsOf: projection.surface.committedMessageIDs)
            archivedActivityPartIDs.append(contentsOf: projection.surface.archivedActivityPartIDs)
            if let pageLiveMessageID = projection.surface.liveMessageID {
                guard liveMessageID == nil else {
                    throw MacApplicationServiceError.invalidResponse
                }
                liveMessageID = pageLiveMessageID
            }
            if let pageActivityPartID = projection.surface.currentActivityPartID {
                guard currentActivityPartID == nil else {
                    throw MacApplicationServiceError.invalidResponse
                }
                currentActivityPartID = pageActivityPartID
            }
            if let pageRun = projection.activeRun {
                guard activeRun == nil || activeRun == pageRun else {
                    throw MacApplicationServiceError.invalidResponse
                }
                activeRun = pageRun
            }
            showAssistantPlaceholder = showAssistantPlaceholder
                || projection.surface.showAssistantPlaceholder
            messages.append(contentsOf: record.payload.messages)
            finalCursorSequence = snapshot.cursor.sequence
            guard page.hasMore else { break }
            offset += record.payload.messages.count
        }
        guard let firstProjection else {
            throw MacApplicationPortError.rejected(
                "Conversation projection \(conversationID) does not exist."
            )
        }
        let projection = MacCanonicalConversationProjectionV1(
            contractVersion: firstProjection.contractVersion,
            conversationID: firstProjection.conversationID,
            revision: firstProjection.revision,
            title: firstProjection.title,
            messages: messages,
            activeRun: activeRun,
            surface: .init(
                revision: firstProjection.surface.revision,
                committedMessageIDs: committedMessageIDs,
                liveMessageID: liveMessageID,
                currentActivityPartID: currentActivityPartID,
                archivedActivityPartIDs: archivedActivityPartIDs,
                showAssistantPlaceholder: showAssistantPlaceholder
            ),
            contextUsage: firstProjection.contextUsage
        )
        return MacCanonicalConversationSnapshot(projection: projection, sequence: finalCursorSequence)
    }

    func waitForConversationProjectionCursor(
        _ conversationID: String,
        afterSequence: UInt64,
        waitMilliseconds: Int,
        cancellation: MacApplicationQueryCancellation? = nil
    ) throws -> UInt64 {
        let scope = ProjectionScope(type: "conversation", conversationID: conversationID)
        let command = ProjectionCursorWaitCommand(
            type: "projection_cursor_wait",
            after: ProjectionCursor(scope: scope, sequence: afterSequence),
            waitMilliseconds: waitMilliseconds
        )
        let response = try serviceClient.execute(
            commandData: Self.makeEncoder().encode(command), queryCancellation: cancellation
        )
        let cursor = try Self.makeDecoder().decode(ProjectionCursor.self, from: response)
        guard cursor.scope.type == scope.type,
              cursor.scope.conversationID == conversationID,
              cursor.sequence >= afterSequence else {
            throw MacApplicationServiceError.invalidResponse
        }
        return cursor.sequence
    }

    func loadConversationProjectionEventBatch(
        _ conversationID: String,
        afterSequence: UInt64,
        waitMilliseconds: Int? = nil
    ) throws -> MacCanonicalConversationProjectionEventBatch {
        let currentSequence = afterSequence
        let scope = ProjectionScope(type: "conversation", conversationID: conversationID)
        let suffix: ProjectionEventSuffix = try execute(
            ProjectionEventSuffixCommand(
                type: "projection_event_suffix",
                scope: scope,
                after: ProjectionCursor(scope: scope, sequence: currentSequence),
                limit: 256,
                waitMilliseconds: waitMilliseconds
            )
        )
        var expectedSequence = currentSequence
        for event in suffix.events {
            guard expectedSequence < UInt64.max,
                  event.conversationID == conversationID,
                  event.sequence == expectedSequence + 1 else {
                throw MacApplicationServiceError.invalidResponse
            }
            expectedSequence = event.sequence
        }
        guard suffix.nextCursor.sequence == expectedSequence,
              !suffix.hasMore || !suffix.events.isEmpty else {
            throw MacApplicationServiceError.invalidResponse
        }
        return MacCanonicalConversationProjectionEventBatch(
            events: suffix.events,
            hasMore: suffix.hasMore,
            afterSequence: currentSequence,
            nextSequence: expectedSequence
        )
    }

    func loadConversationProjectionPageV1(
        _ conversationID: String,
        offset: Int,
        limit: Int
    ) throws -> MacCanonicalConversationProjectionV1 {
        try execute(LoadConversationPageCommand(
            type: "load_conversation_projection_page_v1",
            conversationID: conversationID,
            offset: offset,
            limit: limit
        ))
    }

    func readConversationTextRange(
        _ conversationID: String, messageID: String, partID: String,
        offsetBytes: UInt64, limitBytes: Int = 32 * 1024,
        cancellation: MacApplicationQueryCancellation? = nil
    ) throws -> MacCanonicalTextRange {
        guard (4...32 * 1024).contains(limitBytes) else { throw MacApplicationServiceError.invalidResponse }
        let range: MacCanonicalTextRange = try execute(ReadConversationTextRangeCommand(
            conversationID: conversationID, messageID: messageID, partID: partID,
            offsetBytes: offsetBytes, limitBytes: limitBytes
        ), cancellation: cancellation)
        let next = offsetBytes.addingReportingOverflow(UInt64(range.text.utf8.count))
        guard range.conversationID == conversationID, range.messageID == messageID,
              range.partID == partID, range.offsetBytes == offsetBytes,
              !next.overflow, range.nextOffsetBytes == next.partialValue,
              range.text.utf8.count <= limitBytes, range.nextOffsetBytes <= range.totalUTF8Bytes,
              range.endOfContent == (range.nextOffsetBytes == range.totalUTF8Bytes),
              range.endOfContent || range.nextOffsetBytes > offsetBytes else {
            throw MacApplicationServiceError.invalidResponse
        }
        return range
    }

    private struct ReadConversationTextRangeCommand: Encodable {
        let type = "read_conversation_text_range"
        let conversationID: String
        let messageID: String
        let partID: String
        let offsetBytes: UInt64
        let limitBytes: Int
        enum CodingKeys: String, CodingKey {
            case type, conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
            case offsetBytes = "offset_bytes", limitBytes = "limit_bytes"
        }
    }

    struct ActiveRunIdentity: Decodable, Sendable {
        let runID: String
        let kind: String
        let assistantMessageID: String?
        let nextEventSequence: Int64
        let invocationConfigDigest: String?

        private enum CodingKeys: String, CodingKey {
            case runID = "run_id", kind
            case assistantMessageID = "assistant_message_id"
            case nextEventSequence = "next_event_sequence"
            case invocationConfigDigest = "invocation_config_digest"
        }
    }

    func activeRunIdentity(conversationID: String) throws -> ActiveRunIdentity? {
        try executeOptional(LoadConversationCommand(type: "active_run", conversationID: conversationID))
    }

    struct PermissionConfigurationSnapshot: Decodable, Sendable {
        struct Owner: Decodable, Sendable {
            let kind: String
            let run_id: String?
            let assistant_message_id: String?
            let identity: MacRemoteSubmissionIdentity?
        }
        let conversation_id: String
        let configuration_revision: Int64
        let owner: Owner
        let state: String
    }

    func permissionConfigurationSnapshot(conversationID: String, unresolvedRunIDs: [String] = [],
                                         sourceUnconfirmed: Bool = false) throws -> PermissionConfigurationSnapshot {
        struct Query: Encodable {
            let type = "read_permission_configuration"
            let conversation_id: String
            let unresolved_run_ids: [String]
            let source_unconfirmed: Bool
        }
        let value: PermissionConfigurationSnapshot = try execute(Query(conversation_id: conversationID,
            unresolved_run_ids: unresolvedRunIDs, source_unconfirmed: sourceUnconfirmed))
        guard value.conversation_id == conversationID, value.configuration_revision > 0,
              ["inactive", "pending", "active"].contains(value.state) else {
            throw MacApplicationPortError.invalidResponse
        }
        return value
    }

    func canonicalRunTerminal(conversationID: String, runID: String) throws -> MacCanonicalRunTerminal? {
        struct Command: Encodable {
            let type = "canonical_run_terminal"
            let conversation_id: String
            let run_id: String
        }
        let value: MacCanonicalRunTerminal? = try executeOptional(Command(conversation_id: conversationID, run_id: runID))
        guard value == nil || (value?.conversationID == conversationID && value?.runID == runID) else {
            throw MacApplicationServiceError.invalidResponse
        }
        return value
    }

    func requestRunCancellation(_ input: MacRunCancellationRequest) throws -> MacRunCancellationReceipt {
        struct Command: Encodable {
            let type = "request_run_cancellation"
            let input: MacRunCancellationRequest
        }
        let receipt: MacRunCancellationReceipt = try execute(Command(input: input))
        return try receipt.validated(for: input)
    }

    func inspectRunRecovery(conversationID: String, expectedRunID: String,
                            assistantMessageID: String) throws -> MacRunRecoveryInspection {
        struct Query: Encodable {
            let type = "inspect_run_recovery"
            let conversation_id: String
            let expected_run_id: String
            let assistant_message_id: String
        }
        let value: MacRunRecoveryInspection = try execute(Query(conversation_id: conversationID,
            expected_run_id: expectedRunID, assistant_message_id: assistantMessageID))
        guard value.conversationID == conversationID, value.runID == expectedRunID,
              value.assistantMessageID == assistantMessageID else {
            throw MacApplicationServiceError.invalidResponse
        }
        return value
    }

    func loadConversationWindow(
        _ conversationID: String,
        beforeOrdinal: Int64? = nil,
        limit: Int = 64,
        contentByteLimit: Int = 512_000
    ) throws -> MacCanonicalConversationWindow {
        let window: MacCanonicalConversationWindow = try execute(
            LoadConversationWindowCommand(
                type: "load_conversation_projection_window_v1",
                conversationID: conversationID,
                beforeOrdinal: beforeOrdinal,
                limit: limit,
                contentByteLimit: contentByteLimit
            )
        )
        guard window.projection.conversationID == conversationID,
              !window.hasMore || (window.beforeOrdinal != nil && !window.projection.messages.isEmpty),
              beforeOrdinal == nil || !window.hasMore || window.beforeOrdinal! < beforeOrdinal! else {
            throw MacApplicationServiceError.invalidResponse
        }
        return window
    }

    func loadConversationWindowUpdate(
        _ conversationID: String,
        afterSequence: UInt64,
        beforeOrdinal: Int64? = nil,
        limit: Int = 64,
        contentByteLimit: Int = 512_000
    ) throws -> MacCanonicalConversationWindow? {
        let window: MacCanonicalConversationWindow? = try execute(
            LoadConversationWindowUpdateCommand(
                conversationID: conversationID, beforeOrdinal: beforeOrdinal,
                limit: limit, contentByteLimit: contentByteLimit, afterSequence: afterSequence
            )
        )
        guard let window else { return nil }
        guard window.projection.conversationID == conversationID,
              window.eventSequence > afterSequence,
              !window.hasMore || (window.beforeOrdinal != nil && !window.projection.messages.isEmpty),
              beforeOrdinal == nil || !window.hasMore || window.beforeOrdinal! < beforeOrdinal! else {
            throw MacApplicationServiceError.invalidResponse
        }
        return window
    }

    private struct LoadConversationWindowUpdateCommand: Encodable {
        let type = "load_conversation_projection_window_update_v1"
        let conversationID: String
        let beforeOrdinal: Int64?
        let limit: Int
        let contentByteLimit: Int
        let afterSequence: UInt64

        private enum CodingKeys: String, CodingKey {
            case type, limit
            case conversationID = "conversation_id"
            case beforeOrdinal = "before_ordinal"
            case contentByteLimit = "content_byte_limit"
            case afterSequence = "after_sequence"
        }
    }

    private struct LoadConversationWindowCommand: Encodable {
        let type: String
        let conversationID: String
        let beforeOrdinal: Int64?
        let limit: Int
        let contentByteLimit: Int

        private enum CodingKeys: String, CodingKey {
            case type, limit
            case conversationID = "conversation_id"
            case beforeOrdinal = "before_ordinal"
            case contentByteLimit = "content_byte_limit"
        }
    }

    func loadConversationSettings(_ conversationID: String) throws -> String? {
        try executeOptional(LoadConversationCommand(
            type: "load_conversation_settings",
            conversationID: conversationID
        ))
    }

    func loadConversationContextUsage(
        _ conversationID: String
    ) throws -> MacCanonicalConversationProjectionV1.ContextUsage {
        try execute(LoadConversationCommand(
            type: "load_conversation_context_usage_v1",
            conversationID: conversationID
        ))
    }

    /// Durable user control; a stopped parent process is not an ownership gate.
    func requestOwnedTaskCancellation(conversationID: String, parentRunID: String, taskID: String) throws {
        struct Owner: Encodable {
            let parent_conversation_id: String
            let parent_run_id: String
            let task_id: String
        }
        struct Control: Encodable {
            let type = "request_owned_cancellation"
            let input: Owner
            let now_ms: Int64
        }
        struct Command: Encodable { let type = "runtime_task"; let command: Control }
        struct Receipt: Decodable {
            let parent_conversation_id: String
            let parent_run_id: String
            let task_id: String
            let cancel_requested: Bool
            let terminal: Bool
        }
        let result: Receipt = try execute(Command(command: Control(input: Owner(
            parent_conversation_id: conversationID, parent_run_id: parentRunID, task_id: taskID),
            now_ms: Int64(Date().timeIntervalSince1970 * 1_000))))
        guard result.parent_conversation_id == conversationID, result.parent_run_id == parentRunID,
              result.task_id == taskID, result.cancel_requested || result.terminal else {
            throw MacApplicationPortError.invalidResponse
        }
    }

    func taskGuidanceTransport(path: String, payload: Data,
        conversationID: String, parentRunID: String, coreSessionID: String? = nil
    ) throws -> Data {
        let intent = try JSONDecoder().decode(AppleTaskGuidanceIntent.self, from: payload)
        struct Projection: Decodable {
            let coreSessionID: String
            let taskProjection: MacCoreTaskProjection
            enum CodingKeys: String, CodingKey {
                case coreSessionID = "core_session_id", taskProjection = "task_projection"
            }
        }
        let projection: Projection = try execute(LoadConversationCommand(
            type: "load_conversation_task_projection_v1", conversationID: conversationID))
        guard coreSessionID == nil || projection.coreSessionID == coreSessionID,
              let task = projection.taskProjection.tasks[intent.taskId],
              task.ownerSessionID == projection.coreSessionID else {
            throw AppleTaskGuidanceOutbox.Failure.invalidOwner
        }
        return try AppleTaskGuidanceCoreTransport.exchange(path: path, payload: payload,
            conversationId: conversationID, parentRunId: parentRunID, parentTurnId: task.ownerTurnID) { command in
                let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: command)
                let result: MacCanonicalJSONValue = try self.execute(value)
                return try JSONEncoder().encode(result)
            }
    }

    func loadSessionProjection(_ conversationID: String) throws -> MacCanonicalSessionProjection? {
        try execute(LoadConversationCommand(
            type: "load_session_projection",
            conversationID: conversationID
        ))
    }

    func loadPermissionProjection(_ conversationID: String, runID: String) throws -> MacCorePermissionProjection? {
        struct Scope: Encodable { let type = "current_run" }
        struct Command: Encodable {
            let type = "load_conversation_permission_projection_v1"
            let conversation_id: String
            let run_id: String
            let scope = Scope()
        }
        struct Response: Decodable {
            let conversation_id: String
            let run_id: String
            let session_id: String
            let active_turn_id: String?
            let event_cursor: UInt64
            let permission_projection_revision: UInt64
            let permissions: MacCorePermissionProjection
        }
        guard let response: Response = try executeOptional(Command(conversation_id: conversationID, run_id: runID)) else { return nil }
        guard response.conversation_id == conversationID, response.run_id == runID,
              !response.session_id.isEmpty,
              response.permission_projection_revision == response.permissions.revision else {
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Permission projection has another Run owner"))
        }
        if let policy = response.permissions.activePolicy {
            guard policy.runID == runID, response.active_turn_id == runID,
                  policy.configurationRevision > 0 else {
                throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Active permission policy has another Run owner or invalid revision"))
            }
        }
        return response.permissions
    }

    func listConversations() throws -> [MacCanonicalConversationSummary] {
        let snapshot: ConversationListProjectionSnapshot = try execute(
            ProjectionSnapshotCommand(
                type: "projection_snapshot",
                scope: .init(type: "conversations", conversationID: nil),
                page: nil
            )
        )
        return snapshot.records.compactMap { record in
            guard record.entityType == "conversation_summary" else { return nil }
            return record.payload
        }
    }

    func renameConversation(_ conversationID: String, title: String, updatedAtMs: Int64) throws -> Bool {
        try execute(RenameConversationCommand(
            type: "rename_conversation",
            conversationID: conversationID,
            title: title,
            updatedAtMs: updatedAtMs
        ))
    }

    func deleteConversation(_ conversationID: String) throws -> Bool {
        try execute(LoadConversationCommand(
            type: "delete_conversation",
            conversationID: conversationID
        ))
    }

    func upsertPermissionGrant(
        _ input: MacCanonicalPermissionGrantInput
    ) throws -> MacCanonicalPermissionGrant {
        try execute(Command(type: "upsert_permission_grant", input: input))
    }

    func effectivePermissionRules(
        conversationID: String?,
        hostID: String?
    ) throws -> MacCanonicalPermissionRuleSet {
        try execute(MacEffectivePermissionGrantsCommand(
            type: "effective_permission_grants",
            conversationID: conversationID,
            hostID: hostID
        ))
    }

    func revokePermissionGrant(_ id: String, revokedAtMs: Int64) throws -> Bool {
        try execute(MacRevokePermissionGrantCommand(
            type: "revoke_permission_grant",
            id: id,
            revokedAtMs: revokedAtMs
        ))
    }

    func upsertModelProfile(_ input: MacCanonicalModelProfile) throws {
        let _: MacCanonicalJSONValue = try execute(MacModelCommandEnvelope(command: Command(type: "upsert_profile", input: input)))
    }

    /// Application owns reasoning admission. The native client supplies frozen
    /// route facts and never substitutes a local result when the service fails.
    func resolveReasoning(
        declaration: MacReasoningConfiguration?,
        wireProtocol: MacModelWireProtocol,
        outputLimit: Int,
        supportsReasoning: Bool,
        requested: MacCanonicalJSONValue,
        tier: String?
    ) throws -> MacCanonicalJSONValue {
        struct Resolve: Encodable {
            let type = "resolve_model_reasoning"
            let declaration: MacCanonicalJSONValue
            let wireProtocol: String
            let outputLimit: Int
            let supportsReasoning: Bool
            let requested: MacCanonicalJSONValue
            let tier: String?
            enum CodingKeys: String, CodingKey {
                case type, declaration, requested, tier
                case wireProtocol = "protocol"
                case outputLimit = "output_limit"
                case supportsReasoning = "supports_reasoning"
            }
        }
        struct Resolution: Decodable {
            let selection: MacCanonicalJSONValue
        }
        let declarationValue: MacCanonicalJSONValue
        if let declaration {
            declarationValue = try JSONDecoder().decode(
                MacCanonicalJSONValue.self, from: JSONEncoder().encode(declaration)
            )
        } else {
            declarationValue = .null
        }
        let response: Resolution = try execute(Resolve(
            declaration: declarationValue,
            wireProtocol: wireProtocol.rawValue,
            outputLimit: outputLimit,
            supportsReasoning: supportsReasoning,
            requested: requested,
            tier: tier
        ))
        return response.selection
    }

    func configureModelParameters(_ input: MacCanonicalModelProfile, expectedRevision: UInt64) throws -> UInt64 {
        let data = try Self.makeEncoder().encode(MacModelCommandEnvelope(
            command: Command(type: "configure_parameters", input: input)
        ))
        var revision: UInt64?
        _ = try serviceClient.execute(commandData: data, expectedRevision: expectedRevision,
                                      onCommittedRevision: { revision = $0 })
        guard let revision else { throw MacApplicationServiceError.invalidResponse }
        return revision
    }

    func resetModelParameters(profileIDs: [String], parametersJSON: String, updatedAtMs: Int64) throws {
        struct Reset: Encodable {
            let type = "reset_parameters"
            let profileIDs: [String]
            let parametersJSON: String
            let updatedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type
                case profileIDs = "profile_ids"
                case parametersJSON = "parameters_json"
                case updatedAtMs = "updated_at_ms"
            }
        }
        let _: MacCanonicalJSONValue = try execute(MacModelCommandEnvelope(command: Reset(
            profileIDs: profileIDs, parametersJSON: parametersJSON, updatedAtMs: updatedAtMs
        )))
    }

    func replaceModelProfiles(
        provider: String,
        runtime: String,
        profiles: [MacCanonicalModelProfile],
        updatedAtMs: Int64,
        expectedRevisions: [String: Int64]
    ) throws {
        struct Replacement: Encodable {
            let type = "replace_profiles"
            let provider: String
            let runtime: String
            let profiles: [MacCanonicalModelProfile]
            let updatedAtMs: Int64
            let expectedRevisions: [String: Int64]

            enum CodingKeys: String, CodingKey {
                case type, provider, runtime, profiles
                case updatedAtMs = "updated_at_ms"
                case expectedRevisions = "expected_revisions"
            }
        }
        let _: MacCanonicalJSONValue = try execute(MacModelCommandEnvelope(command: Replacement(
            provider: provider, runtime: runtime, profiles: profiles, updatedAtMs: updatedAtMs,
            expectedRevisions: expectedRevisions
        )))
    }

    func modelProfile(_ id: String) throws -> MacCanonicalModelProfile? {
        try executeOptional(MacModelCommandEnvelope(command: ModelProfileCommand(type: "get_profile", id: id)))
    }

    func listModelProfiles() throws -> [MacCanonicalModelProfile] {
        try execute(MacModelCommandEnvelope(command: TypeOnlyCommand(type: "list_profiles")))
    }

    func deleteModelProfile(_ id: String, deletedAtMs: Int64) throws -> Bool {
        try execute(MacModelCommandEnvelope(command: DeleteModelProfileCommand(
            type: "delete_profile",
            id: id,
            deletedAtMs: deletedAtMs
        )))
    }

    func reportModelRuntime(_ input: MacCanonicalModelRuntime) throws {
        let _: MacCanonicalJSONValue = try execute(MacModelCommandEnvelope(command: Command(type: "report_runtime", input: input)))
    }

    func modelRuntime(hostID: String) throws -> MacCanonicalModelRuntime? {
        try executeOptional(MacModelCommandEnvelope(command: ModelRuntimeCommand(type: "get_runtime", hostID: hostID)))
    }

    func saveConversationPlan(
        conversationID: String,
        expectedRevision: Int64?,
        todos: MacCanonicalJSONValue,
        updatedAtMs: Int64
    ) throws -> MacCanonicalConversationPlan {
        try execute(MacConversationPlanCommandEnvelope(command: SaveConversationPlanCommand(
            type: "save",
            conversationID: conversationID,
            expectedRevision: expectedRevision,
            sop: .object([
                "runId": .string("\(conversationID):plan"),
                "sopId": .string("nola.conversation-plan"),
                "sopVersion": .number(1),
            ]),
            todos: todos,
            updatedAtMs: updatedAtMs
        )))
    }

    func conversationPlan(_ conversationID: String) throws -> MacCanonicalConversationPlan? {
        try executeOptional(MacConversationPlanCommandEnvelope(command: ConversationPlanCommand(
            type: "get",
            conversationID: conversationID
        )))
    }

    func upsertSkill(_ input: MacCanonicalSkillInput) throws -> MacCanonicalSkill {
        try execute(MacSkillCommandEnvelope(command: MacUpsertSkillCommand(input: input)))
    }

    func synchronizeBuiltInSkills(_ inputs: [MacCanonicalSkillInput]) throws -> [MacCanonicalSkillEditor] {
        struct Payload: Encodable {
            let type = "synchronize_built_ins"
            let inputs: [MacCanonicalSkillInput]
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(inputs: inputs)))
    }

    func skillEditors() throws -> [MacCanonicalSkillEditor] {
        struct Payload: Encodable {
            let type = "list_editors"
        }
        return try execute(MacSkillCommandEnvelope(command: Payload()))
    }

    func skillEditor(_ id: String) throws -> MacCanonicalSkillEditor? {
        struct Payload: Encodable {
            let type = "get_editor"
            let id: String
        }
        return try executeOptional(MacSkillCommandEnvelope(command: Payload(id: id)))
    }

    func createUserSkill(_ input: MacCanonicalSkillInput) throws -> MacCanonicalSkillEditor {
        struct Payload: Encodable {
            let type = "create_user"
            let input: MacCanonicalSkillInput
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(input: input)))
    }

    func setSkillEnabled(_ expected: MacCanonicalSkillEditor, enabled: Bool, updatedAtMs: Int64) throws -> MacCanonicalSkillEditor {
        struct Payload: Encodable {
            let type = "set_enabled_checked"
            let id: String
            let expectedRevision: Int64
            let enabled: Bool
            let updatedAtMs: Int64
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(
            id: expected.record.id, expectedRevision: expected.revision,
            enabled: enabled, updatedAtMs: updatedAtMs
        )))
    }

    func listSkills() throws -> [MacCanonicalSkill] {
        try execute(MacSkillCommandEnvelope(command: MacListSkillsCommand(type: "list")))
    }

    func restoreBuiltInSkills(
        _ expected: [MacCanonicalSkillEditor], visibilityRevision: Int64?, updatedAtMs: Int64
    ) throws -> [MacCanonicalSkillEditor] {
        struct Payload: Encodable {
            let type = "restore_built_ins"
            let expectedRevisions: [String: Int64]
            let expectedVisibilityRevision: Int64?
            let updatedAtMs: Int64
        }
        var revisions: [String: Int64] = [:]
        for snapshot in expected {
            guard revisions.updateValue(snapshot.revision, forKey: snapshot.record.id) == nil else {
                throw NSError(domain: "Nola.SkillCommand", code: 1, userInfo: [
                    NSLocalizedDescriptionKey: "Duplicate Skill identity in restore command: \(snapshot.record.id)"
                ])
            }
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(
            expectedRevisions: revisions, expectedVisibilityRevision: visibilityRevision,
            updatedAtMs: updatedAtMs
        )))
    }

    func removeBuiltInSkill(
        _ expected: MacCanonicalSkillEditor, visibilityRevision: Int64?, updatedAtMs: Int64
    ) throws -> MacCanonicalSkillEditor {
        struct Payload: Encodable {
            let type = "remove_built_in"
            let id: String
            let expectedRevision: Int64
            let expectedVisibilityRevision: Int64?
            let updatedAtMs: Int64
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(
            id: expected.record.id, expectedRevision: expected.revision,
            expectedVisibilityRevision: visibilityRevision, updatedAtMs: updatedAtMs
        )))
    }

    func setSkillsEnabled(
        _ expected: [MacCanonicalSkillEditor], enabled: Bool, updatedAtMs: Int64
    ) throws -> [MacCanonicalSkillEditor] {
        struct Change: Encodable {
            let type = "set_enabled_checked"
            let id: String
            let expectedRevision: Int64
            let enabled: Bool
            let updatedAtMs: Int64
        }
        struct Payload: Encodable {
            let type = "apply_changes"
            let commands: [Change]
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(commands: expected.map {
            Change(id: $0.record.id, expectedRevision: $0.revision,
                   enabled: enabled, updatedAtMs: updatedAtMs)
        })))
    }

    func deleteUserSkill(_ expected: MacCanonicalSkillEditor, deletedAtMs: Int64) throws -> Bool {
        struct Payload: Encodable {
            let type = "delete_user_checked"
            let id: String
            let expectedRevision: Int64
            let deletedAtMs: Int64
        }
        return try execute(MacSkillCommandEnvelope(command: Payload(
            id: expected.record.id, expectedRevision: expected.revision, deletedAtMs: deletedAtMs
        )))
    }

    func createMCPServer(_ input: MacCanonicalMCPServerInput) throws -> MacCanonicalMCPServer {
        struct Payload: Encodable {
            let type = "create"
            let id: String
            let displayName: String
            let enabled: Bool
            let configurationJSON: String
            let createdAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type, id, enabled
                case displayName = "display_name"
                case configurationJSON = "configuration_json"
                case createdAtMs = "created_at_ms"
            }
        }
        return try execute(MacMCPCommandEnvelope(command: Payload(
            id: input.id, displayName: input.displayName, enabled: input.enabled,
            configurationJSON: input.configurationJSON, createdAtMs: input.createdAtMs
        )))
    }

    func configureMCPServer(
        _ expected: MacCanonicalMCPServer,
        displayName: String,
        enabled: Bool,
        configurationJSON: String,
        updatedAtMs: Int64
    ) throws -> MacCanonicalMCPServer {
        struct Payload: Encodable {
            let type = "configure"
            let id: String
            let expectedRevision: Int64
            let displayName: String
            let enabled: Bool
            let configurationJSON: String
            let updatedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type, id, enabled
                case expectedRevision = "expected_revision"
                case displayName = "display_name"
                case configurationJSON = "configuration_json"
                case updatedAtMs = "updated_at_ms"
            }
        }
        return try execute(MacMCPCommandEnvelope(command: Payload(
            id: expected.id, expectedRevision: expected.revision, displayName: displayName,
            enabled: enabled, configurationJSON: configurationJSON, updatedAtMs: updatedAtMs
        )))
    }

    func deleteMCPServer(_ expected: MacCanonicalMCPServer, deletedAtMs: Int64) throws -> Bool {
        struct Payload: Encodable {
            let type = "delete_checked"
            let id: String
            let expectedRevision: Int64
            let deletedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type, id
                case expectedRevision = "expected_revision"
                case deletedAtMs = "deleted_at_ms"
            }
        }
        return try execute(MacMCPCommandEnvelope(command: Payload(
            id: expected.id, expectedRevision: expected.revision, deletedAtMs: deletedAtMs
        )))
    }

    func listMCPServers() throws -> [MacCanonicalMCPServer] {
        try execute(MacMCPCommandEnvelope(command: MacListMCPCommand(type: "list")))
    }

    func upsertAutomation(
        _ input: MacCanonicalAutomationInput
    ) throws -> MacCanonicalAutomation {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "definition",
            command: Command(type: "upsert", input: input)
        )))
    }

    func reconcileAutomations(
        _ input: MacCanonicalReconcileAutomationsInput
    ) throws -> [MacCanonicalAutomation] {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "definition",
            command: ReconcileAutomationsCommand(
                type: "reconcile", inputs: input.inputs, scheduleFormats: ["nola.mac.automation.v1"], reconciledAtMs: input.reconciledAtMs
            )
        )))
    }

    func listAutomations() throws -> [MacCanonicalAutomation] {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "definition", command: TypeOnlyCommand(type: "list")
        )))
    }

    func enqueueAutomationRun(_ input: MacCanonicalAutomationRun) throws -> MacCanonicalAutomationRun {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "run", command: Command(type: "enqueue", input: input)
        )))
    }

    func claimAutomationRun(
        id: String,
        workerID: String,
        leaseToken: String,
        nowMs: Int64,
        leaseUntilMs: Int64
    ) throws -> Bool {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "run", command: ClaimAutomationRunCommand(
            type: "claim",
            id: id,
            workerID: workerID,
            leaseToken: leaseToken,
            nowMs: nowMs,
            leaseUntilMs: leaseUntilMs
        ))))
    }

    func heartbeatAutomationRun(
        id: String,
        leaseToken: String,
        nowMs: Int64,
        leaseUntilMs: Int64
    ) throws -> Bool {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "run", command: HeartbeatAutomationRunCommand(
            type: "heartbeat",
            id: id,
            leaseToken: leaseToken,
            nowMs: nowMs,
            leaseUntilMs: leaseUntilMs
        ))))
    }

    func observeAutomationExecution(occurrenceID: String) throws -> MacAutomationExecutionObservation {
        try execute(ObserveAutomationExecutionCommand(occurrenceID: occurrenceID))
    }
    func listPendingAutomationExecutions(afterOccurrenceID: String?, limit: Int) throws -> [String] {
        try execute(ListPendingAutomationExecutionsCommand(afterOccurrenceID: afterOccurrenceID, limit: limit))
    }

    func reconcileAutomationExecution(occurrenceID: String, reconciledAtMs: Int64) throws -> MacAutomationExecutionReceipt? {
        try executeOptional(ReconcileAutomationExecutionCommand(occurrenceID: occurrenceID, reconciledAtMs: reconciledAtMs))
    }

    func observeAutomationWorker(lease: MacAutomationExecutionLease, observation: MacAutomationWorkerObservation, observedAtMs: Int64) throws -> Bool {
        try execute(ObserveAutomationWorkerCommand(lease: lease, observation: observation, observedAtMs: observedAtMs))
    }

    func ackAutomationNotification(occurrenceID: String, notificationID: String, deliveredAtMs: Int64) throws -> Bool {
        try execute(AckAutomationNotificationCommand(occurrenceID: occurrenceID, notificationID: notificationID, deliveredAtMs: deliveredAtMs))
    }

    func loadAutomationRun(id: String) throws -> MacCanonicalAutomationRun? {
        try executeOptional(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "run", command: LoadAutomationRunCommand(type: "load", id: id)
        )))
    }

    func listAutomationRuns(automationID: String? = nil, limit: Int = 200) throws -> [MacCanonicalAutomationRun] {
        try execute(MacAutomationCommandEnvelope(command: MacAutomationObjectCommand(
            type: "run", command: ListAutomationRunsCommand(
            type: "list",
            automationID: automationID,
            limit: max(1, limit)
        ))))
    }

    func createDataSourceDraft(_ input: MacCanonicalDataSourceDraft) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacCreateDataSourceDraftCommand(input: input)))
    }

    func updateDataSourceConfiguration(
        id: String,
        expectedRevision: Int64,
        input: MacCanonicalDataSourceConfigurationUpdate
    ) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacUpdateDataSourceConfigurationCommand(
            id: id,
            expectedRevision: expectedRevision,
            input: input
        )))
    }

    func requestDataSourceValidation(
        id: String,
        expectedRevision: Int64,
        requestedAtMs: Int64
    ) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacRequestDataSourceValidationCommand(
            id: id,
            expectedRevision: expectedRevision,
            requestedAtMs: requestedAtMs
        )))
    }

    func recordDataSourceValidationSuccess(
        id: String,
        validationToken: String,
        contract: MacDataSourceContract,
        recordedAtMs: Int64
    ) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacRecordDataSourceValidationCommand(
            id: id,
            validationToken: validationToken,
            result: MacDataSourceValidationSuccess(contract: contract.coreContractJSON),
            recordedAtMs: recordedAtMs
        )))
    }

    func recordDataSourceValidationFailure(
        id: String,
        validationToken: String,
        code: String,
        diagnostic: String,
        recordedAtMs: Int64
    ) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacRecordDataSourceValidationCommand(
            id: id,
            validationToken: validationToken,
            result: MacDataSourceValidationFailure(code: code, diagnostic: diagnostic),
            recordedAtMs: recordedAtMs
        )))
    }

    func setDataSourceDefaultEnabled(
        id: String,
        expectedRevision: Int64,
        enabled: Bool,
        updatedAtMs: Int64
    ) throws -> MacCanonicalDataSource {
        try execute(MacDataSourceCommandEnvelope(command: MacSetDataSourceDefaultEnabledCommand(
            id: id,
            expectedRevision: expectedRevision,
            enabled: enabled,
            updatedAtMs: updatedAtMs
        )))
    }

    func dataSource(id: String) throws -> MacCanonicalDataSource? {
        try executeOptional(MacDataSourceCommandEnvelope(command: MacGetDataSourceCommand(id: id)))
    }

    func listDataSources() throws -> [MacCanonicalDataSource] {
        try execute(MacDataSourceCommandEnvelope(command: MacListDataSourceCommand(type: "list")))
    }

    func deleteDataSource(_ id: String, expectedRevision: Int64, deletedAtMs: Int64) throws -> Bool {
        try execute(MacDataSourceCommandEnvelope(command: DeleteDataSourceCommand(
            type: "delete",
            id: id,
            expectedRevision: expectedRevision,
            deletedAtMs: deletedAtMs
        )))
    }

    func setDataSourceOverride(
        conversationID: String,
        sourceID: String,
        value: MacCanonicalDataSourceOverride,
        updatedAtMs: Int64
    ) throws -> Bool {
        try execute(MacDataSourceCommandEnvelope(command: SetDataSourceOverrideCommand(
            type: "set_conversation_override",
            conversationID: conversationID,
            sourceID: sourceID,
            value: value,
            updatedAtMs: updatedAtMs
        )))
    }

    func dataSourceOverride(
        conversationID: String,
        sourceID: String
    ) throws -> MacCanonicalDataSourceOverride {
        try execute(MacDataSourceCommandEnvelope(command: GetDataSourceOverrideCommand(
            type: "get_conversation_override",
            conversationID: conversationID,
            sourceID: sourceID
        )))
    }

    func appendDataSourceUsage(_ input: MacCanonicalDataSourceUsageInput) throws -> Bool {
        try execute(MacDataSourceCommandEnvelope(command: Command(type: "record_usage", input: input)))
    }

    func listDataSourceUsage(limit: Int = 500) throws -> [MacCanonicalDataSourceUsageInput] {
        try execute(MacDataSourceCommandEnvelope(command: ListDataSourceUsageCommand(type: "list_usage", limit: max(1, limit))))
    }

    func listMemories(limit: Int = 100) throws -> [MacCanonicalMemoryRecord] {
        try execute(MacMemoryCommandEnvelope(command: ListMemoriesCommand(type: "list", limit: max(1, limit))))
    }

    func putProductSetting(
        key: String,
        valueJSON: String,
        updatedAtMs: Int64
    ) throws -> MacCanonicalProductSetting {
        try execute(PutProductSettingCommand(
            type: "put_product_setting",
            key: key,
            valueJSON: valueJSON,
            updatedAtMs: updatedAtMs
        ))
    }

    func productSetting(key: String) throws -> MacCanonicalProductSetting? {
        try executeOptional(ProductSettingKeyCommand(type: "get_product_setting", key: key))
    }

    func putProductSettingChecked(
        key: String,
        valueJSON: String,
        expectedRevision: Int64?,
        updatedAtMs: Int64
    ) throws -> MacCanonicalProductSetting {
        struct Checked: Encodable {
            let type = "put_product_setting_checked"
            let key: String
            let valueJSON: String
            let expectedRevision: Int64?
            let updatedAtMs: Int64
        }
        return try execute(Checked(key: key, valueJSON: valueJSON,
                                   expectedRevision: expectedRevision, updatedAtMs: updatedAtMs))
    }

    func resolveChatModel(
        intent: MacChatModelIntent,
        defaultValueJSON: String?,
        candidates: [MacChatModelCandidate]
    ) throws -> MacChatModelResolution {
        struct Resolve: Encodable {
            let type = "resolve_chat_model"
            let intent: MacChatModelIntent
            let defaultValueJSON: String?
            let candidates: [MacChatModelCandidate]
        }
        return try execute(Resolve(intent: intent, defaultValueJSON: defaultValueJSON, candidates: candidates))
    }

    @discardableResult
    func deleteProductSetting(key: String) throws -> Bool {
        try execute(ProductSettingKeyCommand(type: "delete_product_setting", key: key))
    }

    func pendingSyncChanges(limit: Int = 100) throws -> [MacCanonicalPendingSyncChange] {
        try execute(PendingSyncChangesCommand(type: "pending_sync_changes", limit: max(1, limit)))
    }

    @discardableResult
    func resolveSyncChange(
        operationID: String,
        status: String,
        remoteCursor: Int64?,
        resolvedAtMs: Int64
    ) throws -> Bool {
        try execute(ResolveSyncChangeCommand(
            type: "resolve_sync_change",
            operationID: operationID,
            status: status,
            remoteCursor: remoteCursor,
            resolvedAtMs: resolvedAtMs
        ))
    }

    func syncCursor(replicaID: String) throws -> MacCanonicalSyncCursor? {
        try executeOptional(SyncCursorCommand(type: "get_sync_cursor", replicaID: replicaID))
    }

    func saveSyncCursor(replicaID: String, cursor: Int64, updatedAtMs: Int64) throws -> MacCanonicalSyncCursor {
        try execute(SaveSyncCursorCommand(
            type: "save_sync_cursor",
            replicaID: replicaID,
            cursor: cursor,
            updatedAtMs: updatedAtMs
        ))
    }

    func applyRemoteSyncPage(
        replicaID: String,
        changes: [MacCanonicalRemoteSyncChange],
        nextCursor: Int64,
        appliedAtMs: Int64
    ) throws -> MacCanonicalRemoteSyncPageResult {
        try execute(ApplyRemoteSyncPageCommand(
            type: "apply_remote_sync_page",
            input: .init(
                replicaID: replicaID,
                changes: changes,
                nextCursor: nextCursor,
                appliedAtMs: appliedAtMs
            )
        ))
    }

    func prepareRemoteSubmission(_ input: MacPrepareRemoteSubmissionInput) throws -> MacRemoteSubmissionIntent {
        struct Command: Encodable { let type = "prepare_remote_submission"; let input: MacPrepareRemoteSubmissionInput }
        return try execute(Command(input: input))
    }
    func bindRemoteSubmissionRequest(identity: MacRemoteSubmissionIdentity, request: MacCanonicalJSONValue) throws -> MacRemoteSubmissionIntent {
        struct Command: Encodable {
            let type = "bind_remote_submission_request"
            let identity: MacRemoteSubmissionIdentity
            let request: MacCanonicalJSONValue
        }
        return try execute(Command(identity: identity, request: request))
    }
    func conversationSourceOwner(conversationID: String) throws -> AppleConversationSourceOwner {
        struct Query: Encodable { let type = "load_conversation_source_owner"; let conversation_id: String }
        return try execute(Query(conversation_id: conversationID))
    }
    func remoteSubmissionIntent(conversationID: String) throws -> MacRemoteSubmissionIntent? {
        struct Command: Encodable { let type = "read_remote_submission_intent"; let conversation_id: String }
        return try executeOptional(Command(conversation_id: conversationID))
    }
    func remoteSubmissionRequest(identity: MacRemoteSubmissionIdentity, digest: String) throws -> MacCanonicalJSONValue {
        struct Command: Encodable { let type = "read_remote_submission_request"; let identity: MacRemoteSubmissionIdentity; let request_digest: String }
        return try execute(Command(identity: identity, request_digest: digest))
    }
    func acceptRemoteSubmissionReceipt(_ receipt: MacSourceSubmissionReceipt) throws -> MacRemoteSubmissionIntent {
        struct Command: Encodable { let type = "accept_remote_submission_receipt"; let receipt: MacSourceSubmissionReceipt }
        return try execute(Command(receipt: receipt))
    }
    func abandonPreparedRemoteSubmission(_ identity: MacRemoteSubmissionIdentity) throws -> MacRemoteSubmissionIntent {
        struct Command: Encodable { let type = "abandon_prepared_remote_submission"; let identity: MacRemoteSubmissionIdentity }
        return try execute(Command(identity: identity))
    }
    func adoptObservedRemoteRun(_ receipt: MacSourceSubmissionReceipt) throws {
        struct Command: Encodable { let type = "adopt_observed_remote_run"; let receipt: MacSourceSubmissionReceipt }
        let _: MacCanonicalJSONValue = try execute(Command(receipt: receipt))
    }

    /// Questions are canonical Run state, not a live subprocess rendezvous.
    func openUserInputWaits(conversationID: String) throws -> [MacRemoteUserInputRequest] {
        let command: [String: Any] = ["type": "list_open_user_input_waits",
                                     "conversation_id": conversationID]
        let data = try JSONSerialization.data(withJSONObject: command)
        let response = try serviceClient.execute(commandData: data)
        guard let records = try JSONSerialization.jsonObject(with: response) as? [[String: Any]] else {
            throw MacApplicationServiceError.invalidResponse
        }
        return try records.map { record in
            guard let wait = MacRemoteUserInputRequest(object: record), wait.isOpen,
                  wait.conversationID == conversationID else {
                throw MacApplicationServiceError.invalidResponse
            }
            return wait
        }
    }

    func respondToUserInputWait(_ request: MacRemoteUserInputRequest, commandID: String,
                               answers: [MacUserInputAnswer], decline: Bool) throws -> MacUserInputWaitReceipt {
        let response: [String: Any] = decline
            ? ["type": "decline", "reason": NSNull()]
            : ["type": "answers", "answers": answers.map(\.object)]
        let command: [String: Any] = [
            "type": "respond_to_conversation_wait",
            "input": ["conversation_id": request.conversationID, "run_id": request.runID,
                      "committed_at_ms": Int64(Date().timeIntervalSince1970 * 1_000),
                      "response": ["command_id": commandID, "wait_id": request.id,
                                   "expected_revision": request.revision,
                                   "response_schema_digest": request.schemaDigest,
                                   "response": response]],
        ]
        let data = try JSONSerialization.data(withJSONObject: command)
        let result = try serviceClient.execute(commandData: data)
        guard let object = try JSONSerialization.jsonObject(with: result) as? [String: Any],
              let type = object["type"] as? String, ["applied", "replay", "conflict"].contains(type),
              let record = object["wait"] as? [String: Any],
              let wait = MacRemoteUserInputRequest(object: record),
              wait.id == request.id, wait.runID == request.runID,
              wait.conversationID == request.conversationID else {
            throw MacApplicationServiceError.invalidResponse
        }
        let resumePolicy = object["resume_policy"] as? String
        guard type == "conflict" || resumePolicy == "conversation_run" || resumePolicy == "task_scheduler" else {
            throw MacApplicationServiceError.invalidResponse
        }
        guard type == "conflict" || !wait.isOpen else { throw MacApplicationServiceError.invalidResponse }
        return MacUserInputWaitReceipt(disposition: type, resumePolicy: resumePolicy, wait: wait)
    }

    /// Observation only: the Host still claims the FIFO head and validates
    /// idle admission transactionally before it can execute this input.
    func idleQueuedUserCommand(conversationID: String, nowMS: Int64) throws -> String? {
        struct ActiveQuery: Encodable { let type = "active_run"; let conversation_id: String }
        struct Active: Decodable { let run_id: String }
        let active: Active? = try execute(ActiveQuery(conversation_id: conversationID))
        guard active == nil else { return nil }
        struct HeadQuery: Encodable { let type = "peek_agent_command"; let session_id: String }
        struct Head: Decodable {
            let command_id: String
            let kind: String
            let user_message_id: String?
            let lease_until_ms: Int64?
        }
        let head: Head? = try execute(HeadQuery(session_id: conversationID))
        guard let head, head.kind == "user_input",
              head.user_message_id == "ui:\(head.command_id):user",
              head.lease_until_ms.map({ $0 <= nowMS }) ?? true else { return nil }
        return head.command_id
    }

    func listUISurfaces(conversationID: String) throws -> [AppleUISnapshot<MacCanonicalJSONValue>] {
        try execute(AppleUIListCommand(conversation_id: conversationID))
    }

    func previewUISurface(_ input: AppleUIPreviewInput<MacCanonicalJSONValue>) throws -> AppleUIPreviewResult<MacCanonicalJSONValue> {
        try execute(AppleUIPreviewCommand(input: input))
    }

    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?) throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue> {
        try execute(AppleUIReadChangesCommand(identity: identity, base_revision: baseRevision))
    }

    func readUISurfaceRevision(identity: AppleUISurfaceIdentity, revision: UInt64) throws -> AppleUISnapshot<MacCanonicalJSONValue>? {
        try execute(AppleUIReadRevisionCommand(identity: identity, revision: revision))
    }

    func submitUIInteraction(_ input: AppleUISubmission<MacCanonicalJSONValue>) throws -> AppleUIInteractionReceipt<MacCanonicalJSONValue> {
        let requestID = "ui-interaction:\(input.interaction.identity.owner.conversation_id):\(input.submission_id)"
        let response = try serviceClient.execute(
            commandData: JSONEncoder().encode(AppleUISubmitCommand(input: input)), requestID: requestID)
        return try JSONDecoder().decode(AppleUIInteractionReceipt<MacCanonicalJSONValue>.self, from: response)
    }

    func prepareEditedResend(_ input: ApplePrepareEditedResend) throws -> ApplePreparedEditedResend<MacCanonicalJSONValue> {
        struct Command: Encodable { let type = "prepare_edited_resend"; let input: ApplePrepareEditedResend }
        return try execute(Command(input: input))
    }

    func loadPreparedEditedResend(_ conversationID: String) throws -> ApplePreparedEditedResend<MacCanonicalJSONValue>? {
        struct Query: Encodable { let type = "load_prepared_edited_resend"; let conversation_id: String }
        return try executeOptional(Query(conversation_id: conversationID))
    }

    func resumePendingExpertAdmission<Result: Decodable>(_ input: AppleAdmitExpertConversationMessage) throws -> Result? {
        try serviceClient.resumePendingExpertAdmission(input)
    }

    func executeExpertHost<Result: Decodable>(_ operation: AppleExpertHostOperation<MacCanonicalJSONValue>) throws -> Result {
        try serviceClient.expertHostOperation(operation)
    }

    func expertMessageSource(conversationID: String, messageID: String) throws -> AppleExpertMessageManifest<MacCanonicalJSONValue>? {
        try executeExpertHost(.loadManifest(conversationID: conversationID, messageID: messageID, authority: .local()))
    }
    func prepareExpertResend(_ input: ApplePrepareEditedResend) throws -> ApplePreparedExpertResend<MacCanonicalJSONValue> {
        try executeExpertHost(.prepareResend(input: input, authority: .local()))
    }
    func loadPreparedExpertResend(_ conversationID: String) throws -> ApplePreparedExpertResend<MacCanonicalJSONValue>? {
        try executeExpertHost(.loadPreparedResend(conversationID: conversationID, authority: .local()))
    }
    func expertChildInstructions(conversationID: String, expectedRunID: String) throws -> [AppleExpertPromptDefinition<MacCanonicalJSONValue>]? {
        try executeExpertHost(.loadChildInstructions(conversationID: conversationID, expectedRunID: expectedRunID, authority: .local()))
    }

    func listActivity(_ query: AppleActivityQuery) throws -> AppleActivityPage {
        struct Command: Encodable { let type = "list_activity"; let input: AppleActivityQuery }
        return try execute(Command(input: query))
    }
    func activityDetail(_ entry: AppleActivityEntry) throws -> AppleActivityDetail? {
        struct Command: Encodable {
            let type = "load_activity_detail"
            let conversation_id: String; let run_id: String; let assistant_message_id: String
        }
        return try execute(Command(conversation_id: entry.conversation_id,
            run_id: entry.run_id, assistant_message_id: entry.assistant_message_id))
    }

    private func execute<CommandValue: Encodable, Result: Decodable>(
        _ command: CommandValue,
        cancellation: MacApplicationQueryCancellation? = nil
    ) throws -> Result {
        // Application Service owns command ordering and durable receipts.
        // Serializing every client request here makes a long-poll projection
        // read block send, cancel, and permission commands on the same port.
        let data = try Self.makeEncoder().encode(command)
        let response = try serviceClient.execute(commandData: data, queryCancellation: cancellation)
        return try Self.makeDecoder().decode(Result.self, from: response)
    }

    private func executeOptional<CommandValue: Encodable, Result: Decodable>(
        _ command: CommandValue
    ) throws -> Result? {
        let data = try Self.makeEncoder().encode(command)
        let response = try serviceClient.execute(commandData: data)
        return try Self.makeDecoder().decode(Optional<Result>.self, from: response)
    }

    private struct Command<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }

    private struct LoadConversationCommand: Encodable {
        let type: String
        let conversationID: String
    }

    private struct LoadConversationPageCommand: Encodable {
        let type: String
        let conversationID: String
        let offset: Int
        let limit: Int
    }

    private struct ProjectionSnapshotCommand: Encodable {
        let type: String
        let scope: ProjectionScope
        let page: ProjectionPageRequest?
    }

    private struct ProjectionPageRequest: Encodable {
        let offset: UInt64
        let limit: Int
        let expectedRevision: UInt64?

        private enum CodingKeys: String, CodingKey {
            case offset, limit
            case expectedRevision = "expected_revision"
        }
    }

    private struct ProjectionCursorWaitCommand: Encodable {
        let type: String
        let after: ProjectionCursor
        let waitMilliseconds: Int

        private enum CodingKeys: String, CodingKey {
            case type, after
            case waitMilliseconds = "wait_ms"
        }
    }

    private struct ProjectionEventSuffixCommand: Encodable {
        let type: String
        let scope: ProjectionScope
        let after: ProjectionCursor
        let limit: Int
        let waitMilliseconds: Int?

        private enum CodingKeys: String, CodingKey {
            case type, scope, after, limit
            case waitMilliseconds = "wait_ms"
        }
    }

    private struct ProjectionScope: Codable {
        let type: String
        let conversationID: String?

        private enum CodingKeys: String, CodingKey {
            case type
            case conversationID = "conversation_id"
        }
    }

    private struct ProjectionSnapshot: Decodable {
        let revision: UInt64
        let cursor: ProjectionCursor
        let records: [ProjectionRecord]
        let page: ProjectionPage?
    }

    private struct ProjectionPage: Decodable {
        let offset: UInt64
        let limit: Int
        let hasMore: Bool

        private enum CodingKeys: String, CodingKey {
            case offset, limit
            case hasMore = "has_more"
        }
    }

    private struct ProjectionCursor: Codable {
        let scope: ProjectionScope
        let sequence: UInt64
    }

    private struct ProjectionRecord: Decodable {
        let entityType: String
        let entityID: String
        let revision: UInt64
        let payload: MacCanonicalConversationProjectionV1

        private enum CodingKeys: String, CodingKey {
            case entityType = "entity_type"
            case entityID = "entity_id"
            case revision
            case payload
        }
    }

    private struct ConversationListProjectionSnapshot: Decodable {
        let cursor: ProjectionCursor
        let records: [ConversationListProjectionRecord]
    }

    private struct ConversationListProjectionRecord: Decodable {
        let entityType: String
        let entityID: String
        let revision: UInt64
        let payload: MacCanonicalConversationSummary

        private enum CodingKeys: String, CodingKey {
            case entityType = "entity_type"
            case entityID = "entity_id"
            case revision
            case payload
        }
    }

    private struct ProjectionEventSuffix: Decodable {
        let events: [MacCanonicalConversationProjectionEvent]
        let nextCursor: ProjectionCursor
        let hasMore: Bool

        private enum CodingKeys: String, CodingKey {
            case events
            case nextCursor = "next_cursor"
            case hasMore = "has_more"
        }
    }

    private struct TypeOnlyCommand: Encodable {
        let type: String
    }

    private struct PutProductSettingCommand: Encodable {
        let type: String
        let key: String
        let valueJSON: String
        let updatedAtMs: Int64
    }

    private struct ProductSettingKeyCommand: Encodable {
        let type: String
        let key: String
    }

    private struct PendingSyncChangesCommand: Encodable {
        let type: String
        let limit: Int
    }

    private struct ResolveSyncChangeCommand: Encodable {
        let type: String
        let operationID: String
        let status: String
        let remoteCursor: Int64?
        let resolvedAtMs: Int64
    }

    private struct SyncCursorCommand: Encodable {
        let type: String
        let replicaID: String
    }

    private struct SaveSyncCursorCommand: Encodable {
        let type: String
        let replicaID: String
        let cursor: Int64
        let updatedAtMs: Int64
    }

    private struct ApplyRemoteSyncPageCommand: Encodable {
        struct Input: Encodable {
            let replicaID: String
            let changes: [MacCanonicalRemoteSyncChange]
            let nextCursor: Int64
            let appliedAtMs: Int64
        }
        let type: String
        let input: Input
    }

    private struct ModelProfileCommand: Encodable {
        let type: String
        let id: String
    }

    private struct DeleteModelProfileCommand: Encodable {
        let type: String
        let id: String
        let deletedAtMs: Int64

        private enum CodingKeys: String, CodingKey {
            case type, id
            case deletedAtMs = "deleted_at_ms"
        }
    }

    private struct ModelRuntimeCommand: Encodable {
        let type: String
        let hostID: String
    }

    private struct SaveConversationPlanCommand: Encodable {
        let type: String
        let conversationID: String
        let expectedRevision: Int64?
        let sop: MacCanonicalJSONValue
        let todos: MacCanonicalJSONValue
        let updatedAtMs: Int64
    }

    private struct ConversationPlanCommand: Encodable {
        let type: String
        let conversationID: String
    }

    private struct ReconcileAutomationsCommand: Encodable {
        let type: String
        let inputs: [MacCanonicalAutomationInput]
        let scheduleFormats: [String]
        let reconciledAtMs: Int64
    }

    private struct MacAutomationCommandEnvelope<Payload: Encodable>: Encodable {
        let type = "automation"
        let command: Payload
    }

    private struct MacAutomationObjectCommand<Payload: Encodable>: Encodable {
        let type: String
        let command: Payload
    }

    private struct ClaimAutomationRunCommand: Encodable {
        let type: String
        let id: String
        let workerID: String
        let leaseToken: String
        let nowMs: Int64
        let leaseUntilMs: Int64
    }

    private struct HeartbeatAutomationRunCommand: Encodable {
        let type: String
        let id: String
        let leaseToken: String
        let nowMs: Int64
        let leaseUntilMs: Int64
    }

    private struct ObserveAutomationExecutionCommand: Encodable {
        let type = "observe_automation_execution"
        let occurrenceID: String
    }
    private struct ListPendingAutomationExecutionsCommand: Encodable {
        let type = "list_pending_automation_executions"
        let afterOccurrenceID: String?
        let limit: Int
    }
    private struct ReconcileAutomationExecutionCommand: Encodable {
        let type = "reconcile_automation_execution"
        let occurrenceID: String
        let reconciledAtMs: Int64
    }
    private struct ObserveAutomationWorkerCommand: Encodable {
        let type = "observe_automation_worker"
        let lease: MacAutomationExecutionLease
        let observation: MacAutomationWorkerObservation
        let observedAtMs: Int64
    }
    private struct AckAutomationNotificationCommand: Encodable {
        let type = "ack_automation_notification"
        let occurrenceID: String
        let notificationID: String
        let deliveredAtMs: Int64
    }

    private struct LoadAutomationRunCommand: Encodable {
        let type: String
        let id: String
    }

    private struct ListAutomationRunsCommand: Encodable {
        let type: String
        let automationID: String?
        let limit: Int
    }

    private struct ListMemoriesCommand: Encodable {
        let type: String
        let limit: Int
    }

    private struct DeleteDataSourceCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let deletedAtMs: Int64

        private enum CodingKeys: String, CodingKey {
            case type, id
            case expectedRevision = "expected_revision"
            case deletedAtMs = "deleted_at_ms"
        }
    }

    private struct SetDataSourceOverrideCommand: Encodable {
        let type: String
        let conversationID: String
        let sourceID: String
        let value: MacCanonicalDataSourceOverride
        let updatedAtMs: Int64
    }

    private struct GetDataSourceOverrideCommand: Encodable {
        let type: String
        let conversationID: String
        let sourceID: String
    }

    private struct ListDataSourceUsageCommand: Encodable {
        let type: String
        let limit: Int
    }

    private struct RenameConversationCommand: Encodable {
        let type: String
        let conversationID: String
        let title: String
        let updatedAtMs: Int64
    }

    private static func makeEncoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }

    private static func makeDecoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}
