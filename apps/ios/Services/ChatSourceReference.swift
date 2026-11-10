// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable, persisted UI projection for Core Source/Citation parts.

import Foundation

nonisolated public struct ChatMemoryUsageEvidence: Codable, Equatable, Sendable {
    public struct Namespace: Codable, Equatable, Sendable {
        public let kind: String
        public let name: String?
    }

    public let recordID: String
    public let componentID: String
    public let scope: String?
    public let namespace: Namespace?
    public let provenanceKind: String?
    public let source: String?
    public let title: String?
    public let observedAt: String?
    public let requiresCurrentStateVerification: Bool
    public let truncated: Bool
    public let supersedes: [String]
    public let usageCount: UInt64
    public let lastUsedAtEpochMs: Int64?
    public var sourceRole: String? = nil
    public var eventTime: String? = nil
    public var rangeStart: String? = nil
    public var rangeEnd: String? = nil

    enum CodingKeys: String, CodingKey {
        case scope, namespace, source, title, supersedes, truncated
        case recordID = "record_id"
        case componentID = "component_id"
        case provenanceKind = "provenance_kind"
        case observedAt = "observed_at"
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case usageCount = "usage_count"
        case lastUsedAtEpochMs = "last_used_at_epoch_ms"
        case sourceRole = "source_role", eventTime = "event_time", rangeStart = "range_start", rangeEnd = "range_end"
    }

    public var displayMetadata: [String] {
        var values: [String] = []
        if let scope, !scope.isEmpty { values.append(scope.replacingOccurrences(of: "_", with: " ")) }
        if let namespace {
            values.append((namespace.name ?? namespace.kind).replacingOccurrences(of: "_", with: " "))
        }
        if let source, !source.isEmpty { values.append(source) }
        if let observedAt, !observedAt.isEmpty { values.append(observedAt) }
        if let sourceRole, !sourceRole.isEmpty { values.append(sourceRole) }
        if let eventTime, !eventTime.isEmpty { values.append(eventTime) }
        if let rangeStart { values.append(rangeStart + (rangeEnd.map { "–" + $0 } ?? "")) }
        if usageCount > 0 { values.append("×\(usageCount)") }
        if !supersedes.isEmpty { values.append("↺\(supersedes.count)") }
        if requiresCurrentStateVerification { values.append("⚠︎") }
        if truncated { values.append("…") }
        return values
    }
}

nonisolated public struct ChatSourceLocation: Codable, Equatable, Sendable {
    public let page: UInt64?
    public let lineStart: UInt64?
    public let lineEnd: UInt64?
    public let section: String?

    private enum CodingKeys: String, CodingKey {
        case page, section
        case lineStart = "line_start", lineEnd = "line_end"
    }

    public init(page: UInt64? = nil, lineStart: UInt64? = nil, lineEnd: UInt64? = nil, section: String? = nil) {
        self.page = page
        self.lineStart = lineStart
        self.lineEnd = lineEnd
        self.section = section
    }

    public var displayText: String? {
        // Keep the localized wording without narrowing a public u64 to C Int32.
        func numbers(_ template: String, _ values: [UInt64]) -> String {
            var result = template
            for value in values {
                guard let placeholder = result.range(of: "%d") else { break }
                result.replaceSubrange(placeholder, with: String(value))
            }
            return result
        }
        var components: [String] = []
        if let page {
            components.append(numbers(NSLocalizedString("chat.source.page_format", value: "Page %d", comment: "Source page number"), [page]))
        }
        if let lineStart {
            if let lineEnd, lineEnd != lineStart {
                components.append(numbers(NSLocalizedString("chat.source.lines_format", value: "Lines %d-%d", comment: "Source line range"), [lineStart, lineEnd]))
            } else {
                components.append(numbers(NSLocalizedString("chat.source.line_format", value: "Line %d", comment: "Source line number"), [lineStart]))
            }
        }
        if let section = section?.trimmingCharacters(in: .whitespacesAndNewlines), !section.isEmpty {
            components.append(section)
        }
        return components.isEmpty ? nil : components.joined(separator: " · ")
    }
}

nonisolated public struct ChatSourceReference: Codable, Equatable, Identifiable, Sendable {
    public static let conversationMetaKey = "nola.agentSourceReferences"

    public let id: String
    public let sourceID: String
    public let citationID: String?
    public let title: String
    public let uri: String
    public let mimeType: String?
    public let location: ChatSourceLocation?
    public let originToolCallID: String?
    public let memoryUsage: ChatMemoryUsageEvidence?
    public let rangeStart: UInt64?
    public let rangeEnd: UInt64?

    public init(
        id: String,
        sourceID: String,
        citationID: String? = nil,
        title: String,
        uri: String,
        mimeType: String? = nil,
        location: ChatSourceLocation? = nil,
        originToolCallID: String? = nil,
        memoryUsage: ChatMemoryUsageEvidence? = nil,
        rangeStart: UInt64? = nil,
        rangeEnd: UInt64? = nil
    ) {
        self.id = id
        self.sourceID = sourceID
        self.citationID = citationID
        self.title = title
        self.uri = uri
        self.mimeType = mimeType
        self.location = location
        self.originToolCallID = originToolCallID
        self.memoryUsage = memoryUsage
        self.rangeStart = rangeStart
        self.rangeEnd = rangeEnd
    }

    public var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        if let fileURL { return fileURL.lastPathComponent }
        return displayDomain ?? NSLocalizedString("chat.source.untitled", value: "Source", comment: "Untitled source")
    }

    public var displayDomain: String? {
        guard let host = webURL?.host?.lowercased(), !host.isEmpty else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    public var webURL: URL? {
        guard let url = URL(string: uri),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              url.host != nil else { return nil }
        return url
    }

    public var fileURL: URL? {
        if let url = URL(string: uri), url.isFileURL { return url.standardizedFileURL }
        guard uri.hasPrefix("/") else { return nil }
        return URL(fileURLWithPath: uri).standardizedFileURL
    }

    public var canOpen: Bool { webURL != nil || fileURL != nil }

    public static func encodeForConversationMeta(_ references: [ChatSourceReference]) -> String? {
        guard !references.isEmpty,
              let data = try? JSONEncoder().encode(references) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public static func decodeFromConversationMeta(_ meta: [String: String]?) -> [ChatSourceReference] {
        guard let encoded = meta?[conversationMetaKey],
              let data = encoded.data(using: .utf8),
              let references = try? JSONDecoder().decode([ChatSourceReference].self, from: data) else { return [] }
        return references
    }
}

extension Notification.Name {
    static let chatAgentSourceReferencesDidChange = Notification.Name("chatAgentSourceReferencesDidChange")
}
