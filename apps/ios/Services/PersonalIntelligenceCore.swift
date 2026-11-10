// SPDX-License-Identifier: Apache-2.0
import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated public enum IntelligenceMemoryKind: String, Codable, Sendable {
    case episodic
    case semantic
    case procedural
}

nonisolated public struct IntelligenceEvent: Codable, Sendable {
    public var id: String
    public var source: String
    public var kind: String
    public var occurredAt: String
    public var observedAt: String
    public var subject: String?
    public var summary: String
    public var payload: [String: AgentCoreJSONValue]?
    public var importance: Double
    public var confidence: Double
    public var tags: [String]
    public var dedupeKey: String?

    public init(
        id: String = "",
        source: String,
        kind: String,
        occurredAt: String = "",
        observedAt: String = "",
        subject: String? = nil,
        summary: String,
        payload: [String: AgentCoreJSONValue]? = nil,
        importance: Double = 0.5,
        confidence: Double = 0.7,
        tags: [String] = [],
        dedupeKey: String? = nil
    ) {
        self.id = id
        self.source = source
        self.kind = kind
        self.occurredAt = occurredAt
        self.observedAt = observedAt
        self.subject = subject
        self.summary = summary
        self.payload = payload
        self.importance = importance
        self.confidence = confidence
        self.tags = tags
        self.dedupeKey = dedupeKey
    }

    enum CodingKeys: String, CodingKey {
        case id, source, kind, subject, summary, payload, importance, confidence, tags
        case occurredAt = "occurred_at"
        case observedAt = "observed_at"
        case dedupeKey = "dedupe_key"
    }
}

nonisolated public struct IntelligenceMemoryCandidate: Codable, Sendable {
    public var semanticKey: String?
    public var kind: IntelligenceMemoryKind
    public var statement: String
    public var confidence: Double
    public var sourceEventIDs: [String]
    public var observedAt: String
    public var tags: [String]

    public init(
        semanticKey: String? = nil,
        kind: IntelligenceMemoryKind = .semantic,
        statement: String,
        confidence: Double = 0.7,
        sourceEventIDs: [String] = [],
        observedAt: String = "",
        tags: [String] = []
    ) {
        self.semanticKey = semanticKey
        self.kind = kind
        self.statement = statement
        self.confidence = confidence
        self.sourceEventIDs = sourceEventIDs
        self.observedAt = observedAt
        self.tags = tags
    }

    enum CodingKeys: String, CodingKey {
        case kind, statement, confidence, tags
        case semanticKey = "semantic_key"
        case sourceEventIDs = "source_event_ids"
        case observedAt = "observed_at"
    }
}

nonisolated public struct IntelligenceTaskOutcome: Codable, Sendable {
    public var id: String
    public var taskType: String
    public var route: String
    public var model: String
    public var tools: [String]
    public var success: Bool
    public var userCorrected: Bool
    public var retryCount: Int
    public var latencyMS: UInt64
    public var occurredAt: String

    public init(
        id: String = "",
        taskType: String,
        route: String,
        model: String,
        tools: [String] = [],
        success: Bool,
        userCorrected: Bool = false,
        retryCount: Int = 0,
        latencyMS: UInt64 = 0,
        occurredAt: String = ""
    ) {
        self.id = id
        self.taskType = taskType
        self.route = route
        self.model = model
        self.tools = tools
        self.success = success
        self.userCorrected = userCorrected
        self.retryCount = retryCount
        self.latencyMS = latencyMS
        self.occurredAt = occurredAt
    }

    enum CodingKeys: String, CodingKey {
        case id, route, model, tools, success
        case taskType = "task_type"
        case userCorrected = "user_corrected"
        case retryCount = "retry_count"
        case latencyMS = "latency_ms"
        case occurredAt = "occurred_at"
    }
}

nonisolated public struct IntelligenceIngestRequest: Codable, Sendable {
    public var events: [IntelligenceEvent]
    public var memoryCandidates: [IntelligenceMemoryCandidate]
    public var outcomes: [IntelligenceTaskOutcome]

    public init(
        events: [IntelligenceEvent] = [],
        memoryCandidates: [IntelligenceMemoryCandidate] = [],
        outcomes: [IntelligenceTaskOutcome] = []
    ) {
        self.events = events
        self.memoryCandidates = memoryCandidates
        self.outcomes = outcomes
    }

    enum CodingKeys: String, CodingKey {
        case events, outcomes
        case memoryCandidates = "memory_candidates"
    }
}

nonisolated public struct IntelligenceRecallRequest: Codable, Sendable {
    public var query: String
    public var limit: Int
    public var kinds: [IntelligenceMemoryKind]
    public var source: String?
    public var minimumScore: Double

    public init(
        query: String,
        limit: Int = 8,
        kinds: [IntelligenceMemoryKind] = [],
        source: String? = nil,
        minimumScore: Double = 0.18
    ) {
        self.query = query
        self.limit = limit
        self.kinds = kinds
        self.source = source
        self.minimumScore = minimumScore
    }

    enum CodingKeys: String, CodingKey {
        case query, limit, kinds, source
        case minimumScore = "minimum_score"
    }
}

nonisolated public enum PersonalIntelligenceCore {
    public nonisolated static var isAvailable: Bool {
#if canImport(NolaCore)
        return nola_core_intelligence_abi_version() == 1
#else
        return false
#endif
    }

    public nonisolated static func ingest(_ request: IntelligenceIngestRequest) -> [String: AgentCoreJSONValue]? {
        call(request, function: nola_core_intelligence_ingest)
    }

    public nonisolated static func recall(_ request: IntelligenceRecallRequest) -> [String: AgentCoreJSONValue]? {
        call(request, function: nola_core_intelligence_recall)
    }

    public nonisolated static func recallContext(for query: String, limit: Int = 6) -> String {
        guard let value = recall(IntelligenceRecallRequest(query: query, limit: limit))?["context"],
              case .string(let context) = value else {
            return ""
        }
        return context.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
    }

    public nonisolated static func snapshot() -> [String: AgentCoreJSONValue]? {
#if canImport(NolaCore)
        guard isAvailable, let pointer = nola_core_intelligence_snapshot() else { return nil }
        defer { nola_core_free_c_string(pointer) }
        return decode(pointer)
#else
        return nil
#endif
    }

    @discardableResult
    public nonisolated static func clear() -> Bool {
#if canImport(NolaCore)
        return isAvailable && nola_core_intelligence_clear()
#else
        return false
#endif
    }

    private nonisolated static func call<T: Encodable>(
        _ input: T,
        function: (UnsafePointer<UInt8>?, Int) -> UnsafeMutablePointer<CChar>?
    ) -> [String: AgentCoreJSONValue]? {
#if canImport(NolaCore)
        guard isAvailable, let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            function(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        return decode(pointer)
#else
        return nil
#endif
    }

    private nonisolated static func decode(
        _ pointer: UnsafePointer<CChar>
    ) -> [String: AgentCoreJSONValue]? {
        guard let raw = String(validatingUTF8: pointer),
              let data = raw.data(using: .utf8),
              let value = try? JSONDecoder().decode(AgentCoreJSONValue.self, from: data),
              case .object(let object) = value else {
            return nil
        }
        return object
    }
}
