// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed macOS boundary for Core's stateless memory-selection ABI.
// - Boundary: The platform owns persistence/retrieval; Core owns policy selection.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated enum MacCoreMemoryScope: String, Codable, Sendable { case turn, conversation, global }
nonisolated enum MacCoreMemoryRecallMode: String, Codable, Sendable {
    case residentProfile = "resident_profile"
    case onDemand = "on_demand"
}
nonisolated enum MacCoreMemoryTokenAccounting: String, Codable, Sendable {
    case hostExact = "host_exact"
    case coreEstimate = "core_estimate"
}
nonisolated enum MacCoreMemoryKind: String, Codable, Sendable { case preference, identity, decision, project, feedback, reference, other }
nonisolated enum MacCoreMemoryProvenanceKind: String, Codable, Sendable {
    case explicitUser = "explicit_user"
    case agentInference = "agent_inference"
    case toolResult = "tool_result"
    case imported, synced, other
}

nonisolated enum MacCoreMemoryNamespace: Codable, Equatable, Sendable {
    case personal, workspace, conversation, custom(String)
    private enum CodingKeys: String, CodingKey { case kind, name }
    private enum Kind: String, Codable { case personal, workspace, conversation, custom }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(Kind.self, forKey: .kind) {
        case .personal: self = .personal
        case .workspace: self = .workspace
        case .conversation: self = .conversation
        case .custom: self = .custom(try values.decode(String.self, forKey: .name))
        }
    }

    func encode(to encoder: Encoder) throws {
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

nonisolated enum MacCoreMemoryRetention: Codable, Equatable, Sendable {
    case permanent, session, untilSuperseded, expiresAt(String)
    private enum CodingKeys: String, CodingKey { case mode; case expiresAt = "expires_at" }
    private enum Mode: String, Codable {
        case permanent, session
        case untilSuperseded = "until_superseded"
        case expiresAt = "expires_at"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(Mode.self, forKey: .mode) {
        case .permanent: self = .permanent
        case .session: self = .session
        case .untilSuperseded: self = .untilSuperseded
        case .expiresAt: self = .expiresAt(try values.decode(String.self, forKey: .expiresAt))
        }
    }

    func encode(to encoder: Encoder) throws {
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

nonisolated struct MacCoreMemoryPolicy: Codable, Equatable, Sendable {
    let longTermEnabled: Bool
    let automaticExtractionEnabled: Bool
    let searchLimit: Int
    let minimumConfidence: Double
    let maximumAgeDays: UInt32?
    let allowedSensitivities: [String]

    init(longTermEnabled: Bool, automaticExtractionEnabled: Bool, searchLimit: Int, minimumConfidence: Double, maximumAgeDays: UInt32? = nil, allowedSensitivities: [String] = []) {
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

    init(from decoder: Decoder) throws {
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

nonisolated struct MacCoreMemoryProvenance: Codable, Equatable, Sendable {
    let kind: MacCoreMemoryProvenanceKind
    let source: String
    let sessionID: String?
    let turnID: String?
    let itemIDs: [String]
    let observedAt: String?
    enum CodingKeys: String, CodingKey {
        case kind, source
        case sessionID = "session_id"
        case turnID = "turn_id"
        case itemIDs = "item_ids"
        case observedAt = "observed_at"
    }
}

nonisolated struct MacCoreMemoryOwner: Codable, Equatable, Sendable {
    let kind: String
    let profileID: String
    let workspaceID: String?
    let conversationID: String?
    let turnID: String?

    static let localProfile = MacCoreMemoryOwner(kind: "profile", profileID: "local", workspaceID: nil, conversationID: nil, turnID: nil)

    enum CodingKeys: String, CodingKey {
        case kind
        case profileID = "profile_id"
        case workspaceID = "workspace_id"
        case conversationID = "conversation_id"
        case turnID = "turn_id"
    }
}

nonisolated struct MacCoreMemoryRecord: Codable, Equatable, Sendable, Identifiable {
    let id: String
    let owner: MacCoreMemoryOwner
    let namespace: MacCoreMemoryNamespace
    let scope: MacCoreMemoryScope
    let kind: MacCoreMemoryKind
    let content: String
    let summary: String?
    let tags: [String]
    let confidence: Double
    let importance: Double
    let sensitivity: String?
    let retention: MacCoreMemoryRetention
    let supersedes: [String]
    let provenance: MacCoreMemoryProvenance
}

nonisolated struct MacCoreMemorySelectionInput: Codable, Equatable, Sendable {
    let policy: MacCoreMemoryPolicy
    let records: [MacCoreMemoryRecord]
    let mode: MacCoreMemoryRecallMode
    let allowedNamespaces: [MacCoreMemoryNamespace]
    let includeSessionRetention: Bool
    let now: String?
    let maximumInputTokens: UInt32?
    let tokenAccounting: MacCoreMemoryTokenAccounting
    let tokenCounts: [String: UInt32]

    init(policy: MacCoreMemoryPolicy, records: [MacCoreMemoryRecord], mode: MacCoreMemoryRecallMode = .onDemand, allowedNamespaces: [MacCoreMemoryNamespace] = [], includeSessionRetention: Bool = false, now: String? = nil, maximumInputTokens: UInt32? = nil, tokenAccounting: MacCoreMemoryTokenAccounting = .hostExact, tokenCounts: [String: UInt32] = [:]) {
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

nonisolated enum MacCoreMemorySelectionRejectReason: String, Codable, Sendable {
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

nonisolated enum MacCoreMemorySelectionReason: String, Codable, Sendable {
    case residentProfile = "resident_profile"
    case hostRankedOnDemand = "host_ranked_on_demand"
}

nonisolated struct MacCoreSelectedMemory: Codable, Equatable, Sendable {
    let record: MacCoreMemoryRecord
    let requiresCurrentStateVerification: Bool
    let tokenCount: UInt32?
    let tokenCountIsEstimated: Bool
    let selectionReason: MacCoreMemorySelectionReason
    enum CodingKeys: String, CodingKey {
        case record
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case tokenCount = "token_count"
        case tokenCountIsEstimated = "token_count_is_estimated"
        case selectionReason = "selection_reason"
    }
}

nonisolated struct MacCoreRejectedMemory: Codable, Equatable, Sendable {
    let memoryID: String
    let reason: MacCoreMemorySelectionRejectReason
    enum CodingKeys: String, CodingKey { case memoryID = "memory_id"; case reason }
}

nonisolated struct MacCoreMemorySelectionPlan: Codable, Equatable, Sendable {
    let mode: MacCoreMemoryRecallMode
    let selected: [MacCoreSelectedMemory]
    let rejected: [MacCoreRejectedMemory]
    let usedInputTokens: UInt32
    let remainingInputTokens: UInt32?
    let tokenCountsAreEstimated: Bool
    let planDigest: String
    enum CodingKeys: String, CodingKey {
        case mode, selected, rejected
        case usedInputTokens = "used_input_tokens"
        case remainingInputTokens = "remaining_input_tokens"
        case tokenCountsAreEstimated = "token_counts_are_estimated"
        case planDigest = "plan_digest"
    }
}

extension MacPersonalIntelligenceCore {
    nonisolated static var isMemorySelectionAvailable: Bool {
#if canImport(NolaCore)
        nola_core_memory_selection_abi_version() == 1
#else
        false
#endif
    }

    nonisolated static func selectMemories(_ input: MacCoreMemorySelectionInput) -> MacCoreMemorySelectionPlan? {
#if canImport(NolaCore)
        guard isMemorySelectionAvailable,
              let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_memory_select_json(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(MacCoreMemorySelectionPlan.self, from: Data(raw.utf8))
#else
        return nil
#endif
    }
}
