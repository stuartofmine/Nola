// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines persisted conversation state, browser snapshots, and model/host selections.

import Foundation

nonisolated struct MacPromptCacheUsage: Hashable, Codable, Sendable {
    let input_tokens: UInt64?
    let read_tokens: UInt64?
    let write_tokens: UInt64?
    let hit_rate_bps: UInt16?
    let is_compaction: Bool

    var rateText: String {
        guard let bps = hit_rate_bps, bps <= 10_000 else {
            return String(localized: "Cache rate unknown", table: "Chat")
        }
        return (Double(bps) / 10_000).formatted(.percent.precision(.fractionLength(1)))
    }
}

nonisolated struct MacConversationCacheUsage: Hashable, Codable, Sendable {
    let input_tokens: UInt64?
    let read_tokens: UInt64?
    let write_tokens: UInt64?
    let hit_rate_bps: UInt16?
    let reported_requests: UInt64
    let unreported_requests: UInt64

    var rateText: String {
        MacPromptCacheUsage(input_tokens: input_tokens, read_tokens: read_tokens,
            write_tokens: write_tokens, hit_rate_bps: hit_rate_bps, is_compaction: false).rateText
    }
}

nonisolated struct MacContextMeasurement: Hashable, Codable, Sendable {
    let requestID: String
    let attemptID: String?
    let contextGeneration: UInt64?

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case attemptID = "attempt_id"
        case contextGeneration = "context_generation"
    }
}

struct MacConversationContextSnapshot: Hashable, Codable, Sendable {
    let usedTokens: Int
    let contextWindowTokens: Int
    let promptTokenLimit: Int
    let isEstimated: Bool
    let cachedInputTokens: Int?
    let cacheWriteInputTokens: Int?
    let promptCacheDiagnostic: MacPromptCacheDiagnostic?
    let cacheUsage: MacPromptCacheUsage?
    let conversationCacheUsage: MacConversationCacheUsage?
    let measurement: MacContextMeasurement?
    let promptCacheMeasurement: MacContextMeasurement?

    init(
        usedTokens: Int,
        contextWindowTokens: Int,
        promptTokenLimit: Int,
        isEstimated: Bool,
        cachedInputTokens: Int? = nil,
        cacheWriteInputTokens: Int? = nil,
        promptCacheDiagnostic: MacPromptCacheDiagnostic? = nil,
        cacheUsage: MacPromptCacheUsage? = nil,
        conversationCacheUsage: MacConversationCacheUsage? = nil,
        measurement: MacContextMeasurement? = nil,
        promptCacheMeasurement: MacContextMeasurement? = nil
    ) {
        self.usedTokens = usedTokens
        self.contextWindowTokens = contextWindowTokens
        self.promptTokenLimit = promptTokenLimit
        self.isEstimated = isEstimated
        self.cachedInputTokens = cachedInputTokens
        self.cacheWriteInputTokens = cacheWriteInputTokens
        self.promptCacheDiagnostic = promptCacheDiagnostic
        self.cacheUsage = cacheUsage
        self.conversationCacheUsage = conversationCacheUsage
        self.measurement = measurement
        self.promptCacheMeasurement = promptCacheMeasurement
    }

    init(canonical usage: MacCanonicalConversationProjectionV1.ContextUsage) {
        self.init(usedTokens: Int(clamping: usage.usedTokens),
            contextWindowTokens: Int(clamping: usage.windowTokens),
            promptTokenLimit: Int(clamping: usage.promptTokenLimit), isEstimated: usage.estimated,
            promptCacheDiagnostic: usage.promptCacheDiagnostic, cacheUsage: usage.cacheUsage,
            conversationCacheUsage: usage.conversationCacheUsage, measurement: usage.measurement,
            promptCacheMeasurement: usage.promptCacheMeasurement)
    }
}

nonisolated struct MacConversationBrowserPage: Identifiable, Hashable, Codable, Sendable {
    let index: Int
    var title: String
    var url: String
    var isActive: Bool

    var id: Int { index }

    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        return URL(string: url)?.host ?? url
    }
}

nonisolated struct MacConversationBrowserSnapshot: Hashable, Codable, Sendable {
    var target: String
    var sessionID: String?
    var pages: [MacConversationBrowserPage]
    var currentTitle: String?
    var currentURL: String?
    var isConnected: Bool
    var updatedAt: Date
    var previewURL: String?

    var activePage: MacConversationBrowserPage? {
        pages.first(where: \.isActive)
            ?? pages.first(where: { $0.url == currentURL })
            ?? pages.first
    }

    static func reducingCoreTool(
        toolName: String,
        state: String?,
        arguments: MacAgentTraceJSONValue?,
        structuredOutput: MacAgentTraceJSONValue?,
        outputPreview: String?,
        previous: Self?
    ) -> Self? {
        guard toolName.hasPrefix("browser_") || toolName.hasPrefix("chrome_native_") else {
            return previous
        }
        let normalizedState = state?.lowercased()
        guard normalizedState == "completed" || normalizedState == "failed" else {
            return previous
        }
        let result = structuredOutput
            ?? outputPreview.map(MacAgentTraceJSONValue.string)
        return reducing(
            toolName: toolName,
            arguments: arguments,
            result: result,
            succeeded: normalizedState == "completed",
            previous: previous
        )
    }

    static func reducing(
        toolName: String,
        arguments: MacAgentTraceJSONValue?,
        result: MacAgentTraceJSONValue?,
        succeeded: Bool,
        previous: Self?
    ) -> Self? {
        if toolName == "browser_stop", succeeded { return nil }
        guard succeeded else { return previous }

        let target = string(named: "target", in: arguments)
            ?? string(named: "target", in: result)
            ?? previous?.target
            ?? (toolName.hasPrefix("chrome_native_") ? "attached" : "managed")
        var snapshot = previous ?? Self(
            target: target,
            sessionID: nil,
            pages: [],
            currentTitle: nil,
            currentURL: nil,
            isConnected: false,
            updatedAt: .now,
            previewURL: nil
        )
        snapshot.target = target
        snapshot.sessionID = string(named: "session_id", in: result) ?? snapshot.sessionID
        snapshot.previewURL = string(named: "preview_url", in: result) ?? snapshot.previewURL

        if let decodedPages = pages(in: result) {
            snapshot.pages = decodedPages
            if let active = decodedPages.first(where: \.isActive) ?? decodedPages.first {
                snapshot.currentTitle = active.title
                snapshot.currentURL = active.url
            } else {
                snapshot.currentTitle = nil
                snapshot.currentURL = nil
            }
        } else if let url = string(named: "final_url", in: result)
            ?? string(named: "url", in: result)
            ?? string(named: "url", in: arguments),
                  !url.isEmpty {
            let title = string(named: "title", in: result) ?? snapshot.currentTitle ?? ""
            snapshot.currentURL = url
            snapshot.currentTitle = title
            if let activeIndex = snapshot.pages.firstIndex(where: \.isActive) {
                snapshot.pages[activeIndex].url = url
                snapshot.pages[activeIndex].title = title
            } else if snapshot.pages.isEmpty {
                snapshot.pages = [MacConversationBrowserPage(
                    index: 0,
                    title: title,
                    url: url,
                    isActive: true
                )]
            }
        }
        snapshot.isConnected = boolean(named: "running", in: result) ?? true
        snapshot.updatedAt = .now
        return snapshot
    }

    private static func pages(in value: MacAgentTraceJSONValue?) -> [MacConversationBrowserPage]? {
        guard let pageValues = array(named: "pages", in: value) else { return nil }
        return pageValues.enumerated().compactMap { offset, value in
            guard case .object(let object) = normalized(value),
                  let url = scalarString(object["url"]),
                  !url.isEmpty else { return nil }
            return MacConversationBrowserPage(
                index: scalarInt(object["index"]) ?? offset,
                title: scalarString(object["title"]) ?? "",
                url: url,
                isActive: scalarBool(object["active"]) ?? false
            )
        }
    }

    private static func string(
        named key: String,
        in value: MacAgentTraceJSONValue?
    ) -> String? {
        first(named: key, in: value).flatMap(scalarString)
    }

    private static func boolean(
        named key: String,
        in value: MacAgentTraceJSONValue?
    ) -> Bool? {
        first(named: key, in: value).flatMap(scalarBool)
    }

    private static func array(
        named key: String,
        in value: MacAgentTraceJSONValue?
    ) -> [MacAgentTraceJSONValue]? {
        guard let found = first(named: key, in: value), case .array(let values) = found else {
            return nil
        }
        return values
    }

    private static func first(
        named key: String,
        in value: MacAgentTraceJSONValue?,
        depth: Int = 0
    ) -> MacAgentTraceJSONValue? {
        guard depth < 10, let value else { return nil }
        switch normalized(value) {
        case .object(let object):
            if let direct = object[key] { return direct }
            for child in object.values {
                if let found = first(named: key, in: child, depth: depth + 1) { return found }
            }
        case .array(let values):
            for child in values {
                if let found = first(named: key, in: child, depth: depth + 1) { return found }
            }
        default:
            break
        }
        return nil
    }

    private static func normalized(_ value: MacAgentTraceJSONValue) -> MacAgentTraceJSONValue {
        guard case .string(let string) = value,
              let data = string.data(using: .utf8),
              let decoded = try? JSONDecoder().decode(MacAgentTraceJSONValue.self, from: data) else {
            return value
        }
        return decoded
    }

    private static func scalarString(_ value: MacAgentTraceJSONValue?) -> String? {
        guard let value else { return nil }
        if case .string(let string) = value { return string }
        return nil
    }

    private static func scalarBool(_ value: MacAgentTraceJSONValue?) -> Bool? {
        guard let value else { return nil }
        if case .boolean(let boolean) = value { return boolean }
        return nil
    }

    private static func scalarInt(_ value: MacAgentTraceJSONValue?) -> Int? {
        guard let value else { return nil }
        if case .number(let number) = value { return Int(number) }
        return nil
    }
}

struct MacConversation: Identifiable, Hashable, Codable, Sendable {
    let id: String
    var title: String
    var preview: String
    var updatedAt: Date
    var isPinned: Bool
    var isPersisted: Bool
    /// Read receipt, not a configuration field or an inferred current version.
    var configurationRevision: Int64? = nil
    var promptTemplateID: UUID?
    var promptTemplateSelectionIsExplicit: Bool?
    var modelSelectionID: String?
    var executionHostRawValue: String?
    var responseStyleRawValue: String?
    var modelOverrides: [String: MacConversationModelOverride]? = nil
    var contextLimitOverride: Int?
    var memoryEnabledOverride: Bool?
    var retrievalEnabledOverride: Bool?
    var executionBackendRawValue: String?
    var permissionModeRawValue: String?
    var externalExecutionSessionIDs: [String: String]?
    var externalExecutionModelIDs: [String: String]?
    var externalExecutionModelProviderIDs: [String: String]?
    var browserSnapshot: MacConversationBrowserSnapshot?

    init(
        id: String = UUID().uuidString,
        title: String,
        preview: String,
        updatedAt: Date,
        isPinned: Bool,
        isPersisted: Bool = false,
        promptTemplateID: UUID? = nil,
        promptTemplateSelectionIsExplicit: Bool? = nil,
        modelSelectionID: String? = nil,
        executionHostRawValue: String? = nil,
        responseStyleRawValue: String? = nil,
        contextLimitOverride: Int? = nil,
        memoryEnabledOverride: Bool? = nil,
        retrievalEnabledOverride: Bool? = nil,
        executionBackendRawValue: String? = nil,
        permissionModeRawValue: String? = nil,
        externalExecutionSessionIDs: [String: String]? = nil,
        externalExecutionModelIDs: [String: String]? = nil,
        externalExecutionModelProviderIDs: [String: String]? = nil,
        browserSnapshot: MacConversationBrowserSnapshot? = nil
    ) {
        self.id = id
        self.title = title
        self.preview = preview
        self.updatedAt = updatedAt
        self.isPinned = isPinned
        self.isPersisted = isPersisted
        self.promptTemplateID = promptTemplateID
        self.promptTemplateSelectionIsExplicit = promptTemplateSelectionIsExplicit
        self.modelSelectionID = modelSelectionID
        self.executionHostRawValue = executionHostRawValue
        self.responseStyleRawValue = responseStyleRawValue
        self.contextLimitOverride = contextLimitOverride
        self.memoryEnabledOverride = memoryEnabledOverride
        self.retrievalEnabledOverride = retrievalEnabledOverride
        self.executionBackendRawValue = executionBackendRawValue
        self.permissionModeRawValue = permissionModeRawValue
        self.externalExecutionSessionIDs = externalExecutionSessionIDs
        self.externalExecutionModelIDs = externalExecutionModelIDs
        self.externalExecutionModelProviderIDs = externalExecutionModelProviderIDs
        self.browserSnapshot = browserSnapshot
    }
}

struct MacChatModelOption: Identifiable, Hashable {
    enum Source: String, Hashable {
        case configured
        case local
        case connectedHost
    }

    let source: Source
    let modelID: String
    let displayName: String
    let configurationID: String?
    let hostID: String?

    init(source: Source, modelID: String, displayName: String, configurationID: String? = nil, hostID: String? = nil) {
        self.source = source
        self.modelID = modelID
        self.displayName = displayName
        self.configurationID = configurationID
        self.hostID = hostID
    }

    var id: String {
        if source == .connectedHost {
            return "connectedHost:\(Self.identityComponent(hostID ?? "")):\(Self.identityComponent(configurationID ?? modelID))"
        }
        return "\(source.rawValue):\(configurationID ?? modelID)"
    }
    var isLocal: Bool { source == .local }
    var symbol: String { isLocal ? "internaldrive" : "cloud" }

    private static func identityComponent(_ value: String) -> String {
        Data(value.utf8).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private static func decodeIdentityComponent(_ value: Substring) -> String? {
        let encoded = String(value)
        let padded = encoded.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
            + String(repeating: "=", count: (4 - encoded.count % 4) % 4)
        guard let data = Data(base64Encoded: padded),
              let decoded = String(data: data, encoding: .utf8), !decoded.isEmpty,
              identityComponent(decoded) == encoded else { return nil }
        return decoded
    }

    static func parse(_ value: String?) -> MacChatModelOption? {
        guard let value,
              let separator = value.firstIndex(of: ":"),
              let source = Source(rawValue: String(value[..<separator])) else { return nil }
        let modelID = String(value[value.index(after: separator)...])
        guard !modelID.isEmpty else { return nil }
        if source == .connectedHost {
            let components = modelID.split(separator: ":", omittingEmptySubsequences: false)
            guard components.count == 2,
                  let hostID = decodeIdentityComponent(components[0]),
                  let profileID = decodeIdentityComponent(components[1]) else { return nil }
            return MacChatModelOption(source: .connectedHost, modelID: profileID,
                displayName: profileID, configurationID: profileID, hostID: hostID)
        }
        return MacChatModelOption(
            source: source,
            modelID: modelID,
            displayName: modelID,
            configurationID: source == .configured ? modelID : nil
        )
    }
}

enum MacExecutionHost: String, CaseIterable, Identifiable, Sendable {
    case local
    case remote

    var id: String { rawValue }

    var title: String {
        switch self {
        case .local: return String(localized: "This Mac")
        case .remote: return String(localized: "Connected Host")
        }
    }
}
