// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements Models logic in the shared model layer.
// - Interfaces:
//   - JSONValue
//   - URLSource
//   - RawFetch
//   - RawURLItem
//   - ProcessingRequest
//   - ManualParseRequest
//   - MorningBrief
//   - ProviderInfo
// - APIs:
//   - none
//
// Models copied from Doc/Generated/Models.swift for project use
// Generated: 2025-11-15

import Foundation

public enum JSONValue: Codable, Equatable {
    case string(String)
    case number(Double)
    case object([String: JSONValue])
    case array([JSONValue])
    case bool(Bool)
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
            return
        }
        if let b = try? container.decode(Bool.self) {
            self = .bool(b); return
        }
        if let n = try? container.decode(Double.self) {
            self = .number(n); return
        }
        if let s = try? container.decode(String.self) {
            self = .string(s); return
        }
        if let arr = try? container.decode([JSONValue].self) {
            self = .array(arr); return
        }
        if let obj = try? container.decode([String: JSONValue].self) {
            self = .object(obj); return
        }
        throw DecodingError.dataCorruptedError(in: container, debugDescription: "Unsupported JSON value")
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let s): try container.encode(s)
        case .number(let n): try container.encode(n)
        case .bool(let b): try container.encode(b)
        case .array(let a): try container.encode(a)
        case .object(let o): try container.encode(o)
        case .null: try container.encodeNil()
        }
    }
}

// MARK: - URLSource
public struct URLSource: Codable, Equatable {
    public var id: String
    public var alias: String
    public var url: String
    public var enabled: Bool
    public var modelPreference: String?
    public var userDescription: String?
    public var refreshIntervalMinutes: Int?
    public var lastFetchAt: String?
    public var lastStatus: String?
}

// MARK: - RawFetch
public struct RawFetch: Codable, Equatable {
    public var id: String
    public var urlSourceId: String
    public var httpStatus: Int
    public var headers: [String: String]?
    public var rawText: String?
    public var etag: String?
    public var fetchedAt: String
}

// MARK: - RawURLItem
public struct RawURLItem: Codable, Equatable {
    public var id: String
    public var rawFetchId: String
    public var itemIndex: Int
    public var originalJson: JSONValue?
    public var originalHtmlSnippet: String?
    public var extractedText: String
}

// MARK: - ProcessingRequest
public struct ProcessingRequest: Codable, Equatable {
    public var id: String
    public var rawURLItemId: String
    public var sourceAlias: String
    public var userPrompt: String
    public var modelPreference: String?
    public var freshnessMeta: [String: JSONValue]?
}

// NOTE: ModelOutput and ProcessedEvent types are defined elsewhere in the project
// (e.g. `apps/ios/LLM/ModelTypes.swift` and `apps/ios/Services/DataSources.swift`).
// To avoid duplicate type definitions and ambiguity, those canonical types
// are used by the services. This Models.swift file keeps auxiliary types
// (URLSource, RawFetch, RawURLItem, etc.) and the JSONValue helper.

// MARK: - ManualParseRequest & ParsedDraft
public struct ManualParseRequest: Codable, Equatable {
    public var id: String
    public var rawText: String
    public var submittedAt: String
    public var autoConfirm: Bool?
    public var parsedDraft: ParsedDraft?

    public struct ParsedDraft: Codable, Equatable {
        public var title: String?
        public var start: String?
        public var due: String?
        public var confidence: Double?
    }
}

// MARK: - MorningBrief
public struct MorningBrief: Codable, Equatable {
    public var id: String
    public var date: String
    public var items: [BriefItem]
    public var explain: String?
    public var generatedBy: String?

    public struct BriefItem: Codable, Equatable {
        public var text: String
        public var priority: Double
        public var action: [String: JSONValue]?
        public var dedupeKey: String
    }
}

// MARK: - ProviderInfo (Rust <-> Swift model)
public struct ProviderInfo: Codable, Equatable {
    // Matches the Rust `ProviderInfo { provider_type, details }` (serde rename_all = "camelCase")
    public var providerType: String
    public var details: [String: String]?
}
