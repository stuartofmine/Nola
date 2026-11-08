// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed iOS boundary for Core's stateless memory-selection ABI.
// - Boundary: The platform owns persistence/retrieval; Core owns policy selection.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated public enum CoreMemoryScope: String, Codable, Sendable {
    case turn, conversation, global
}

nonisolated public enum CoreMemoryRecallMode: String, Codable, Sendable {
    case residentProfile = "resident_profile"
    case onDemand = "on_demand"
}

nonisolated public enum CoreMemoryTokenAccounting: String, Codable, Sendable {
    case hostExact = "host_exact"
    case coreEstimate = "core_estimate"
}

nonisolated public enum CoreMemoryKind: String, Codable, Sendable {
    case preference, identity, decision, project, feedback, reference, other
}

nonisolated public enum CoreMemoryProvenanceKind: String, Codable, Sendable {
    case explicitUser = "explicit_user"
    case agentInference = "agent_inference"
    case toolResult = "tool_result"
    case imported, synced, other
}

nonisolated public enum CoreMemoryNamespace: Codable, Equatable, Sendable {
    case personal
    case workspace
    case conversation
    case custom(String)

    private enum CodingKeys: String, CodingKey { case kind, name }
    private enum Kind: String, Codable { case personal, workspace, conversation, custom }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(Kind.self, forKey: .kind) {
        case .personal: self = .personal
        case .workspace: self = .workspace
        case .conversation: self = .conversation
        case .custom: self = .custom(try values.decode(String.self, forKey: .name))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .personal: try values.encode(Kind.personal, forKey: .kind)
        case .workspace: try values.encode(Kind.workspace, forKey: .kind)
        case .conversation: try values.encode(Kind.conversation, forKey: .kind)
        case .custom(let name):
            try values.encode(Kind.custom, forKey: .kind)
            try values.encode(name, forKey: .name)
        }
    }
}

nonisolated public enum CoreMemoryRetention: Codable, Equatable, Sendable {
    case permanent
    case session
    case untilSuperseded
    case expiresAt(String)

    private enum CodingKeys: String, CodingKey { case mode; case expiresAt = "expires_at" }
    private enum Mode: String, Codable {
        case permanent, session
        case untilSuperseded = "until_superseded"
        case expiresAt = "expires_at"
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(Mode.self, forKey: .mode) {
        case .permanent: self = .permanent
        case .session: self = .session
        case .untilSuperseded: self = .untilSuperseded
        case .expiresAt: self = .expiresAt(try values.decode(String.self, forKey: .expiresAt))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .permanent: try values.encode(Mode.permanent, forKey: .mode)
        case .session: try values.encode(Mode.session, forKey: .mode)
        case .untilSuperseded: try values.encode(Mode.untilSuperseded, forKey: .mode)
        case .expiresAt(let timestamp):
            try values.encode(Mode.expiresAt, forKey: .mode)
            try values.encode(timestamp, forKey: .expiresAt)
        }
    }
}

nonisolated public struct CoreMemoryPolicy: Codable, Equatable, Sendable {
    public let longTermEnabled: Bool
    public let automaticExtractionEnabled: Bool
    public let searchLimit: Int
    public let minimumConfidence: Double
    public let maximumAgeDays: UInt32?
    public let allowedSensitivities: [String]

    public init(longTermEnabled: Bool, automaticExtractionEnabled: Bool, searchLimit: Int, minimumConfidence: Double, maximumAgeDays: UInt32? = nil, allowedSensitivities: [String] = []) {
        self.longTermEnabled = longTermEnabled
        self.automaticExtractionEnabled = automaticExtractionEnabled
        self.searchLimit = searchLimit
        self.minimumConfidence = minimumConfidence
        self.maximumAgeDays = maximumAgeDays
        self.allowedSensitivities = allowedSensitivities
    }

    enum CodingKeys: String, CodingKey {
        case longTermEnabled = "long_term_enabled"
        case automaticExtractionEnabled = "automatic_extraction_enabled"
        case searchLimit = "search_limit"
        case minimumConfidence = "minimum_confidence"
        case maximumAgeDays = "maximum_age_days"
        case allowedSensitivities = "allowed_sensitivities"
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        longTermEnabled = try values.decode(Bool.self, forKey: .longTermEnabled)
        automaticExtractionEnabled = try values.decode(Bool.self, forKey: .automaticExtractionEnabled)
        searchLimit = try values.decode(Int.self, forKey: .searchLimit)
        minimumConfidence = try values.decode(Double.self, forKey: .minimumConfidence)
        maximumAgeDays = try values.decodeIfPresent(UInt32.self, forKey: .maximumAgeDays)
        allowedSensitivities = try values.decodeIfPresent(
            [String].self,
            forKey: .allowedSensitivities
        ) ?? []
    }
}

nonisolated public struct CoreMemoryProvenance: Codable, Equatable, Sendable {
    public let kind: CoreMemoryProvenanceKind
    public let source: String
    public let sessionID: String?
    public let turnID: String?
    public let itemIDs: [String]
    public let observedAt: String?

    public init(kind: CoreMemoryProvenanceKind, source: String, sessionID: String? = nil, turnID: String? = nil, itemIDs: [String] = [], observedAt: String? = nil) {
        self.kind = kind
        self.source = source
        self.sessionID = sessionID
        self.turnID = turnID
        self.itemIDs = itemIDs
        self.observedAt = observedAt
    }

    enum CodingKeys: String, CodingKey {
        case kind, source
        case sessionID = "session_id"
        case turnID = "turn_id"
        case itemIDs = "item_ids"
        case observedAt = "observed_at"
    }
}

nonisolated public struct CoreMemoryOwner: Codable, Equatable, Sendable {
    public let kind: String
    public let profileID: String
    public let workspaceID: String?
    public let conversationID: String?
    public let turnID: String?

    public static let localProfile = CoreMemoryOwner(kind: "profile", profileID: "local", workspaceID: nil, conversationID: nil, turnID: nil)

    enum CodingKeys: String, CodingKey {
        case kind
        case profileID = "profile_id"
        case workspaceID = "workspace_id"
        case conversationID = "conversation_id"
        case turnID = "turn_id"
    }
}

nonisolated public struct CoreMemoryRecord: Codable, Equatable, Sendable, Identifiable {
    public let owner: CoreMemoryOwner
    public let id: String
    public let namespace: CoreMemoryNamespace
    public let scope: CoreMemoryScope
    public let kind: CoreMemoryKind
    public let content: String
    public let summary: String?
    public let tags: [String]
    public let confidence: Double
    public let importance: Double
    public let sensitivity: String?
    public let retention: CoreMemoryRetention
    public let supersedes: [String]
    public let provenance: CoreMemoryProvenance

    public init(id: String, owner: CoreMemoryOwner, namespace: CoreMemoryNamespace, scope: CoreMemoryScope, kind: CoreMemoryKind, content: String, summary: String? = nil, tags: [String] = [], confidence: Double, importance: Double, sensitivity: String? = nil, retention: CoreMemoryRetention, supersedes: [String] = [], provenance: CoreMemoryProvenance) {
        self.id = id
        self.owner = owner
        self.namespace = namespace
        self.scope = scope
        self.kind = kind
        self.content = content
        self.summary = summary
        self.tags = tags
        self.confidence = confidence
        self.importance = importance
        self.sensitivity = sensitivity
        self.retention = retention
        self.supersedes = supersedes
        self.provenance = provenance
    }
}

nonisolated public struct CoreMemorySelectionInput: Codable, Equatable, Sendable {
    public let policy: CoreMemoryPolicy
    public let records: [CoreMemoryRecord]
    public let mode: CoreMemoryRecallMode
    public let allowedNamespaces: [CoreMemoryNamespace]
    public let includeSessionRetention: Bool
    public let now: String?
    public let maximumInputTokens: UInt32?
    public let tokenAccounting: CoreMemoryTokenAccounting
    public let tokenCounts: [String: UInt32]

    public init(policy: CoreMemoryPolicy, records: [CoreMemoryRecord], mode: CoreMemoryRecallMode = .onDemand, allowedNamespaces: [CoreMemoryNamespace] = [], includeSessionRetention: Bool = false, now: String? = nil, maximumInputTokens: UInt32? = nil, tokenAccounting: CoreMemoryTokenAccounting = .hostExact, tokenCounts: [String: UInt32] = [:]) {
        self.policy = policy
        self.records = records
        self.mode = mode
        self.allowedNamespaces = allowedNamespaces
        self.includeSessionRetention = includeSessionRetention
        self.now = now
        self.maximumInputTokens = maximumInputTokens
        self.tokenAccounting = tokenAccounting
        self.tokenCounts = tokenCounts
    }

    enum CodingKeys: String, CodingKey {
        case policy, records, mode, now
        case allowedNamespaces = "allowed_namespaces"
        case includeSessionRetention = "include_session_retention"
        case maximumInputTokens = "maximum_input_tokens"
        case tokenAccounting = "token_accounting"
        case tokenCounts = "token_counts"
    }
}

nonisolated public enum CoreMemorySelectionRejectReason: String, Codable, Sendable {
    case disabled
    case invalidRecord = "invalid_record"
    case namespaceNotAllowed = "namespace_not_allowed"
    case sensitivityNotAllowed = "sensitivity_not_allowed"
    case belowConfidence = "below_confidence"
    case sessionOnly = "session_only"
    case expired
    case tooOld = "too_old"
    case invalidTimestamp = "invalid_timestamp"
    case superseded
    case notResidentProfile = "not_resident_profile"
    case missingTokenCount = "missing_token_count"
    case tokenBudgetExceeded = "token_budget_exceeded"
    case limitExceeded = "limit_exceeded"
}

nonisolated public enum CoreMemorySelectionReason: String, Codable, Sendable {
    case residentProfile = "resident_profile"
    case hostRankedOnDemand = "host_ranked_on_demand"
}

nonisolated public struct CoreSelectedMemory: Codable, Equatable, Sendable {
    public let record: CoreMemoryRecord
    public let requiresCurrentStateVerification: Bool
    public let tokenCount: UInt32?
    public let tokenCountIsEstimated: Bool
    public let selectionReason: CoreMemorySelectionReason

    enum CodingKeys: String, CodingKey {
        case record
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case tokenCount = "token_count"
        case tokenCountIsEstimated = "token_count_is_estimated"
        case selectionReason = "selection_reason"
    }
}

nonisolated public struct CoreRejectedMemory: Codable, Equatable, Sendable {
    public let memoryID: String
    public let reason: CoreMemorySelectionRejectReason

    enum CodingKeys: String, CodingKey {
        case memoryID = "memory_id"
        case reason
    }
}

nonisolated public struct CoreMemorySelectionPlan: Codable, Equatable, Sendable {
    public let mode: CoreMemoryRecallMode
    public let selected: [CoreSelectedMemory]
    public let rejected: [CoreRejectedMemory]
    public let usedInputTokens: UInt32
    public let remainingInputTokens: UInt32?
    public let tokenCountsAreEstimated: Bool
    public let planDigest: String

    enum CodingKeys: String, CodingKey {
        case mode, selected, rejected
        case usedInputTokens = "used_input_tokens"
        case remainingInputTokens = "remaining_input_tokens"
        case tokenCountsAreEstimated = "token_counts_are_estimated"
        case planDigest = "plan_digest"
    }
}

extension PersonalIntelligenceCore {
    public nonisolated static var isMemorySelectionAvailable: Bool {
#if canImport(NolaCore)
        nola_core_memory_selection_abi_version() == 4
#else
        false
#endif
    }

    public nonisolated static func selectMemories(_ input: CoreMemorySelectionInput) -> CoreMemorySelectionPlan? {
#if canImport(NolaCore)
        guard isMemorySelectionAvailable,
              let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_memory_select_json(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(CoreMemorySelectionPlan.self, from: Data(raw.utf8))
#else
        return nil
#endif
    }
}
