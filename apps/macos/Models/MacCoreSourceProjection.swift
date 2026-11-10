// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Projects Core Source/Citation Parts into safe, stable assistant source rows.

import Foundation

nonisolated struct MacCoreSourceLocation: Codable, Hashable, Sendable {
    let page: UInt64?
    let lineStart: UInt64?
    let lineEnd: UInt64?
    let section: String?

    private enum CodingKeys: String, CodingKey {
        case page, section
        case lineStart = "line_start"
        case lineEnd = "line_end"
    }
}

nonisolated struct MacCoreCitationRange: Codable, Hashable, Sendable {
    let start: UInt64
    let end: UInt64
}

nonisolated struct MacCoreMemoryUsageEvidence: Codable, Hashable, Sendable {
    struct Namespace: Codable, Hashable, Sendable {
        let kind: String
        let name: String?
    }

    let recordID: String
    let componentID: String
    let scope: String?
    let namespace: Namespace?
    let provenanceKind: String?
    let source: String?
    let title: String?
    let observedAt: String?
    let sourceRole: String?
    let eventTime: String?
    let rangeStart: String?
    let rangeEnd: String?
    let requiresCurrentStateVerification: Bool
    let truncated: Bool
    let supersedes: [String]
    let usageCount: UInt64
    let lastUsedAtEpochMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case scope, namespace, source, title, supersedes, truncated
        case recordID = "record_id"
        case componentID = "component_id"
        case provenanceKind = "provenance_kind"
        case observedAt = "observed_at"
        case sourceRole = "source_role"
        case eventTime = "event_time"
        case rangeStart = "range_start"
        case rangeEnd = "range_end"
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case usageCount = "usage_count"
        case lastUsedAtEpochMs = "last_used_at_epoch_ms"
    }

    var displayMetadata: [String] {
        var values: [String] = []
        if let scope, !scope.isEmpty { values.append(scope.replacingOccurrences(of: "_", with: " ")) }
        if let namespace { values.append((namespace.name ?? namespace.kind).replacingOccurrences(of: "_", with: " ")) }
        if let source, !source.isEmpty { values.append(source) }
        if let observedAt, !observedAt.isEmpty { values.append(observedAt) }
        if let sourceRole, !sourceRole.isEmpty { values.append(sourceRole.replacingOccurrences(of: "_", with: " ")) }
        if let eventTime, !eventTime.isEmpty { values.append(eventTime) }
        if let rangeStart, !rangeStart.isEmpty { values.append(rangeStart + (rangeEnd.map { "–" + $0 } ?? "")) }
        if usageCount > 0 { values.append("×\(usageCount)") }
        if !supersedes.isEmpty { values.append("↺\(supersedes.count)") }
        if requiresCurrentStateVerification { values.append("⚠︎") }
        if truncated { values.append("…") }
        return values
    }
}

nonisolated struct MacCoreSourceEvidence: Codable, Hashable, Sendable {
    let kind: String
    let usage: MacCoreMemoryUsageEvidence?

    private enum CodingKeys: String, CodingKey { case kind, usage }
    private struct AnyKey: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    init(kind: String, usage: MacCoreMemoryUsageEvidence?) { self.kind = kind; self.usage = usage }
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: AnyKey.self)
        guard Set(keys.allKeys.map(\.stringValue)) == Set(["kind", "usage"]) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Source evidence contains only typed redacted usage"))
        }
        let values = try decoder.container(keyedBy: CodingKeys.self)
        kind = try values.decode(String.self, forKey: .kind)
        guard kind == "memory" else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown typed source evidence")
        }
        usage = try values.decode(MacCoreMemoryUsageEvidence.self, forKey: .usage)
    }
}

nonisolated struct MacCoreEvidencePart: Hashable, Sendable {
    let kind: String
    let id: String
    let messageID: String?
    let turnID: String?
    let sourceID: String
    let citationID: String?
    let title: String
    let uri: String
    let mimeType: String?
    let location: MacCoreSourceLocation?
    let range: MacCoreCitationRange?
    let evidence: MacCoreSourceEvidence?
    var originToolCallID: String? = nil
}

nonisolated struct MacAssistantSourceReference: Codable, Hashable, Identifiable, Sendable {
    enum Destination: Hashable, Sendable {
        case web(URL)
        case file(URL, line: Int?)
        case memory(recordID: String)
    }

    let id: String
    let sourceID: String
    let citationID: String?
    let title: String
    let uri: String
    let mimeType: String?
    let page: UInt64?
    let lineStart: UInt64?
    let lineEnd: UInt64?
    let section: String?
    let rangeStart: UInt64?
    let rangeEnd: UInt64?
    let memoryUsage: MacCoreMemoryUsageEvidence?
    var location: MacCoreSourceLocation? = nil
    var originToolCallID: String? = nil
    var fallbackTitle: String? = nil

    var domain: String? {
        guard let url = safeWebURL else { return nil }
        let host = url.host(percentEncoded: false) ?? url.host
        guard let host, !host.isEmpty else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    var displayTitle: String {
        if memoryRecordID != nil { return String(localized: "Long-term Memory") }
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        if let fallbackTitle, !fallbackTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return fallbackTitle }
        if let domain { return domain }
        if case .file(let url, _) = destination { return url.lastPathComponent }
        return uri
    }

    var destination: Destination? {
        if let memoryRecordID { return .memory(recordID: memoryRecordID) }
        if let url = safeWebURL { return .web(url) }
        guard let url = URL(string: uri), url.isFileURL else { return nil }
        let host = url.host(percentEncoded: false) ?? url.host
        guard host == nil || host?.isEmpty == true || host == "localhost" else { return nil }
        let standardized = url.standardizedFileURL
        guard standardized.path.hasPrefix("/") else { return nil }
        return .file(standardized, line: lineStart.flatMap(Int.init(exactly:)))
    }

    var isMemoryReference: Bool { memoryRecordID != nil }

    private var memoryRecordID: String? {
        guard let url = URL(string: uri),
              url.scheme?.lowercased() == "nola-memory" else { return nil }
        guard (url.host(percentEncoded: false) ?? url.host) == "record" else { return nil }
        let value = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return value.isEmpty ? nil : value
    }

    private var safeWebURL: URL? {
        guard let url = URL(string: uri),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              url.host != nil,
              url.user == nil,
              url.password == nil else {
            return nil
        }
        return url
    }
}

nonisolated enum MacCoreSourceProjection {
    static func project(
        _ parts: [MacCoreEvidencePart],
        messageID: String? = nil,
        turnID: String? = nil
    ) -> [MacAssistantSourceReference] {
        let owned = parts.filter { part in
            if let messageID, part.messageID != messageID { return false }
            if let turnID, part.turnID != turnID { return false }
            guard part.kind == "source" || part.kind == "citation" else { return false }
            // Workspace files belong to the tool step that read or changed
            // them. Promoting them to reply-level sources duplicates the same
            // file below the assistant bubble and makes an implementation
            // detail look like a user attachment.
            if isLocalFileReference(part.uri) { return false }
            return !isInternalRuntimeSource(part.uri)
        }
        var sources: [String: MacCoreEvidencePart] = [:]
        for part in owned where part.kind == "source" {
            sources[part.sourceID] = part
        }

        var result: [MacAssistantSourceReference] = []
        var citedSourceIDs: Set<String> = []
        var identities: Set<String> = []
        for citation in owned where citation.kind == "citation" {
            citedSourceIDs.insert(citation.sourceID)
            let source = sources[citation.sourceID]
            let reference = makeReference(citation, fallback: source)
            let identity = reference.citationID ?? reference.id
            if identities.insert(identity).inserted { result.append(reference) }
        }
        for part in owned where part.kind == "source" && !citedSourceIDs.contains(part.sourceID) {
            let reference = makeReference(part, fallback: nil)
            if identities.insert(reference.id).inserted { result.append(reference) }
        }
        return result
    }

    /// Files used to implement a bundled Skill are execution inputs, not
    /// evidence for the user's answer. This remains a defense-in-depth guard
    /// for non-file runtime resource URIs.
    private static func isInternalRuntimeSource(_ uri: String) -> Bool {
        guard let url = URL(string: uri), url.isFileURL else { return false }
        let path = url.standardizedFileURL.path
        guard path.contains("/NolaBuiltInSkills.bundle/") else { return false }
        let name = url.lastPathComponent.lowercased()
        return name == "nola_manifest.json"
            || name == "skill.md"
            || path.contains("/scripts/")
            || path.contains("/references/")
    }

    private static func isLocalFileReference(_ uri: String) -> Bool {
        let trimmed = uri.trimmingCharacters(in: .whitespacesAndNewlines)
        if URL(string: trimmed)?.isFileURL == true { return true }
        return NSString(string: trimmed).isAbsolutePath
    }

    private static func makeReference(
        _ part: MacCoreEvidencePart,
        fallback: MacCoreEvidencePart?
    ) -> MacAssistantSourceReference {
        MacAssistantSourceReference(
            id: part.citationID ?? part.id,
            sourceID: part.sourceID,
            citationID: part.citationID,
            title: part.title,
            uri: part.uri.isEmpty ? (fallback?.uri ?? "") : part.uri,
            mimeType: part.mimeType ?? fallback?.mimeType,
            page: part.location?.page,
            lineStart: part.location?.lineStart,
            lineEnd: part.location?.lineEnd,
            section: part.location?.section,
            rangeStart: part.range?.start,
            rangeEnd: part.range?.end,
            memoryUsage: part.evidence?.usage ?? fallback?.evidence?.usage,
            location: part.location,
            originToolCallID: part.originToolCallID,
            fallbackTitle: fallback?.title
        )
    }
}
