// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements AgentModels logic in the shared model layer.
// - Interfaces:
//   - LLMActionType
//   - LLMActionPriority
//   - RawItem
//   - CandidateEvent
//   - Action
//   - LLMActionResponse
//   - NolaNotification
//   - FetchPolicyType
// - APIs:
//   - none
//
import Foundation

// Core data models used by the Agent flow.
// Codable-friendly and intended to match Doc/flows/core_data_models.md

nonisolated public enum LLMActionType: String, Codable {
    case notify
    case store
    case ignore
    case create_task
    case reschedule
}

nonisolated public enum LLMActionPriority: String, Codable {
    case high
    case medium
    case low
}

nonisolated public struct RawItem: Codable {
    public let id: String?
    public let sourceType: String // "contacts" | "calendar" | "reminder" | "url" | "health"
    public let sourceId: String?
    public let title: String?
    public let body: String?
    public let startDate: Date?
    public let endDate: Date?
    public let url: String?
    public let metadata: [String: String]?

    public init(id: String? = nil,
                sourceType: String,
                sourceId: String? = nil,
                title: String? = nil,
                body: String? = nil,
                startDate: Date? = nil,
                endDate: Date? = nil,
                url: String? = nil,
                metadata: [String: String]? = nil) {
        self.id = id
        self.sourceType = sourceType
        self.sourceId = sourceId
        self.title = title
        self.body = body
        self.startDate = startDate
        self.endDate = endDate
        self.url = url
        self.metadata = metadata
    }
}

nonisolated public struct CandidateEvent: Codable {
    public let candidateId: String
    public let rawItem: RawItem
    public let baseScore: Double
    public let tags: [String]
    public let extractedAt: Date

    public init(candidateId: String,
                rawItem: RawItem,
                baseScore: Double,
                tags: [String] = [],
                extractedAt: Date = Date()) {
        self.candidateId = candidateId
        self.rawItem = rawItem
        self.baseScore = baseScore
        self.tags = tags
        self.extractedAt = extractedAt
    }
}

nonisolated public struct Action: Codable {
    public let type: LLMActionType
    public let priority: LLMActionPriority
    public let dedupeKey: String
    public let scheduledAt: Date?
    public let contentSummary: String?
    public let reason: String
    public let confidence: Double?
    public let extra: [String: String]?

    public init(type: LLMActionType,
                priority: LLMActionPriority,
                dedupeKey: String,
                scheduledAt: Date? = nil,
                contentSummary: String? = nil,
                reason: String,
                confidence: Double? = nil,
                extra: [String: String]? = nil) {
        self.type = type
        self.priority = priority
        self.dedupeKey = dedupeKey
        self.scheduledAt = scheduledAt
        self.contentSummary = contentSummary
        self.reason = reason
        self.confidence = confidence
        self.extra = extra
    }
}

nonisolated public struct LLMActionResponse: Codable {
    public let actions: [Action]
    public let summary: String?
    public let tags: [String]?

    public init(actions: [Action], summary: String? = nil, tags: [String]? = nil) {
        self.actions = actions
        self.summary = summary
        self.tags = tags
    }
}

nonisolated public struct NolaNotification: Codable {
    public let id: String
    public let title: String
    public let body: String
    public let priority: LLMActionPriority
    public let category: String
    public let dedupeKey: String
    public let scheduledDate: Date
    public let actionButtons: [String]?
    public let sourceReference: [String: String]?

    public init(id: String = UUID().uuidString,
                title: String,
                body: String,
                priority: LLMActionPriority,
                category: String,
                dedupeKey: String,
                scheduledDate: Date = Date(),
                actionButtons: [String]? = nil,
                sourceReference: [String: String]? = nil) {
        self.id = id
        self.title = title
        self.body = body
        self.priority = priority
        self.category = category
        self.dedupeKey = dedupeKey
        self.scheduledDate = scheduledDate
        self.actionButtons = actionButtons
        self.sourceReference = sourceReference
    }
}

nonisolated public enum FetchPolicyType: String, Codable {
    case interval
    case cron
    case onDemand
}

nonisolated public struct FetchPolicy: Codable {
    public let type: FetchPolicyType
    public let value: String? // seconds as string for interval or cron expression

    public init(type: FetchPolicyType, value: String? = nil) {
        self.type = type
        self.value = value
    }
}

nonisolated public struct URLSourceConfig: Codable {
    public let id: String
    public let url: String
    public let fetchPolicy: FetchPolicy
    public let maxBytes: Int?
    public let useETag: Bool
    public let modelOverride: String?
    public let lastFetchAt: Date?

    public init(id: String,
                url: String,
                fetchPolicy: FetchPolicy,
                maxBytes: Int? = nil,
                useETag: Bool = true,
                modelOverride: String? = nil,
                lastFetchAt: Date? = nil) {
        self.id = id
        self.url = url
        self.fetchPolicy = fetchPolicy
        self.maxBytes = maxBytes
        self.useETag = useETag
        self.modelOverride = modelOverride
        self.lastFetchAt = lastFetchAt
    }
}

nonisolated public struct LLMCallContext: Codable {
    public let modelId: String
    public let inputText: String
    public let inputSummary: String?
    public let metadata: [String: String]?
    public let requireExplicitConsent: Bool?

    public init(modelId: String,
                inputText: String,
                inputSummary: String? = nil,
                metadata: [String: String]? = nil,
                requireExplicitConsent: Bool? = nil) {
        self.modelId = modelId
        self.inputText = inputText
        self.inputSummary = inputSummary
        self.metadata = metadata
        self.requireExplicitConsent = requireExplicitConsent
    }
}

// Helpers: JSON encoder/decoder with ISO8601 date strategy
public extension JSONDecoder {
    nonisolated static func nolaDecoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }
}

public extension JSONEncoder {
    nonisolated static func nolaEncoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        return e
    }
}
