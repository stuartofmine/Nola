// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements URLFetcherService logic in the iOS service layer.
// - Interfaces:
//   - URLDataFormat
//   - URLDataSource
//   - URLFetcherService
//   - URLDataResponse
//   - JSONEvent
//   - JSONEventMetadata
//   - ValidationResult
//   - URLFetcherError
// - APIs:
//   - none
//
//
//  URLFetcherService.swift
//  Nola
//
//  Service for fetching and parsing events from remote URLs.
//

import Foundation
import CryptoKit
import Combine

/// Supported URL data formats
public enum URLDataFormat: String, Codable {
    case json           // JSON with standard schema
    case icalendar      // iCalendar (.ics) format
    case rss            // RSS/Atom feed
    case html           // HTML pages (attempt JSON-LD or metadata extraction)
    case search         // Web/search result source, usually backed by a configured search URL or API
}

public enum URLDataSourceAccessPolicy: String, CaseIterable, Codable, Hashable, Identifiable {
    case defaultEnabled
    case conversationOptIn
    case askWhenRelevant
    case blocked

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .defaultEnabled:
            return NSLocalizedString("data_source.policy.default_enabled", value: "Every conversation", comment: "Data source access policy")
        case .conversationOptIn:
            return NSLocalizedString("data_source.policy.conversation_opt_in", value: "Per conversation", comment: "Data source access policy")
        case .askWhenRelevant:
            return NSLocalizedString("data_source.policy.ask_when_relevant", value: "Ask when relevant", comment: "Data source access policy")
        case .blocked:
            return NSLocalizedString("data_source.policy.blocked", value: "Blocked", comment: "Data source access policy")
        }
    }
}

public enum URLDataSourceValidationState: String, Codable, Hashable {
    case verified
    case verifiedEmpty
    case failed
}

public enum URLDataSourceReadMethod: String, Codable, Hashable {
    case httpGet
    case localFileRead
    case directoryList
    case platformAPI
    case databaseQuery
    case mcpTool
    case hostTool
    case custom
}

public struct URLDataSourceReadRecipe: Codable, Hashable {
    public var method: URLDataSourceReadMethod
    public var operationID: String
    public var configurationKeys: [String]
    public var outputSchemaJSON: String

    public var isUsable: Bool {
        guard !operationID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              Set(configurationKeys).count == configurationKeys.count,
              !configurationKeys.contains(where: {
                  $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
              }),
              let data = outputSchemaJSON.data(using: .utf8),
              let schema = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              schema["type"] as? String != nil else {
            return false
        }
        return true
    }
}

public struct URLDataSourceValidationContract: Codable, Hashable {
    public var sourceID: UUID
    public var connectorID: String
    public var connectorVersion: Int
    public var readerDescription: String
    public var readRecipe: URLDataSourceReadRecipe? = nil
    public var schemaFields: [String]
    public var redactedSample: String
    public var recordCount: Int
    public var maximumAgeMinutes: Int
    public var validationState: URLDataSourceValidationState
    public var validatedAt: Date
    public var observedAt: Date?
    public var validationDigest: String? = nil

    public var isUsable: Bool {
        validationState != .failed
            && connectorVersion > 0
            && !connectorID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !readerDescription.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && readRecipe?.isUsable == true
            && !schemaFields.isEmpty
            && validationDigest == expectedValidationDigest
    }

    public var expectedValidationDigest: String? {
        guard let sealed = AgentCoreDataSourceResolver.sealContract(coreContractJSON),
              case .string(let digest)? = sealed["validationDigest"] else { return nil }
        return digest
    }

    var coreContractJSON: [String: AgentCoreDataSourceJSON] {
        let outputSchema = readRecipe
            .flatMap { $0.outputSchemaJSON.data(using: .utf8) }
            .flatMap { try? JSONDecoder().decode(AgentCoreDataSourceJSON.self, from: $0) }
            ?? .object(["type": .string("object")])
        let recipe: AgentCoreDataSourceJSON = readRecipe.map {
            let method = $0.method == .platformAPI ? "platformApi" : $0.method.rawValue
            return .object([
                "method": .string(method),
                "operationId": .string($0.operationID),
                "configurationKeys": .array(
                    $0.configurationKeys.map(AgentCoreDataSourceJSON.string)
                ),
                "outputSchema": outputSchema
            ])
        } ?? .object([:])
        return [
            "sourceId": .string(sourceID.uuidString),
            "connectorId": .string(connectorID),
            "connectorVersion": .number(Double(connectorVersion)),
            "readerDescription": .string(readerDescription),
            "readRecipe": recipe,
            "schemaFields": .array(schemaFields.map(AgentCoreDataSourceJSON.string)),
            "redactedSample": .string(redactedSample),
            "recordCount": .number(Double(recordCount)),
            "maximumAgeMinutes": .number(Double(maximumAgeMinutes)),
            "validationState": .string(validationState.rawValue),
            "validatedAt": .string(Self.coreDateString(validatedAt)),
            "observedAt": observedAt.map { .string(Self.coreDateString($0)) } ?? .null,
            "validationDigest": .string(validationDigest ?? "")
        ]
    }

    private static func coreDateString(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }

    public mutating func configureReadRecipe(
        method: URLDataSourceReadMethod,
        operationID: String,
        configurationKeys: [String]
    ) {
        let properties = Dictionary(uniqueKeysWithValues: schemaFields.sorted().map {
            ($0, ["type": "string"])
        })
        let schema: [String: Any] = [
            "type": "array",
            "items": ["type": "object", "properties": properties]
        ]
        let schemaData = try? JSONSerialization.data(withJSONObject: schema, options: [.sortedKeys])
        readRecipe = URLDataSourceReadRecipe(
            method: method,
            operationID: operationID,
            configurationKeys: configurationKeys,
            outputSchemaJSON: schemaData.flatMap { String(data: $0, encoding: .utf8) }
                ?? "{\"type\":\"array\",\"items\":{\"type\":\"object\"}}"
        )
        validationDigest = expectedValidationDigest
    }
}

public struct URLDataSourceUsageRecord: Identifiable, Codable, Hashable {
    public var id: UUID
    public var sourceID: UUID
    public var sourceTitle: String
    public var consumer: String
    public var purpose: String
    public var conversationID: UUID?
    public var usedAt: Date
    /// `nil` means the context builder did not yet know which model route would consume it.
    public var disclosedToCloud: Bool?

    public init(
        id: UUID = UUID(),
        sourceID: UUID,
        sourceTitle: String,
        consumer: String,
        purpose: String,
        conversationID: UUID?,
        usedAt: Date = Date(),
        disclosedToCloud: Bool?
    ) {
        self.id = id
        self.sourceID = sourceID
        self.sourceTitle = sourceTitle
        self.consumer = consumer
        self.purpose = purpose
        self.conversationID = conversationID
        self.usedAt = usedAt
        self.disclosedToCloud = disclosedToCloud
    }
}

/// URL data source configuration
public struct URLDataSource: Identifiable, Codable, Hashable {
    public let id: UUID
    public var url: String
    public var format: URLDataFormat
    public var kind: DataSourceKind
    public var name: String
    public var enabled: Bool
    public var lastFetchDate: Date?
    public var lastError: String?
    public var consecutiveFailures: Int = 0  // Number of consecutive failures
    public var isRefreshing: Bool = false     // Whether a refresh is currently running
    /// History of notifications/tests triggered for this source
    nonisolated public struct HistoryEntry: Codable, Hashable, Identifiable, Sendable {
        public let id: UUID
        public let timestamp: Date
        public let dedupeKey: String
        public let title: String
        public let details: String?
        public let eventId: String?
        public let priority: Double?
        // LLM processing metadata
        public let tokensUsed: Int?           // Number of tokens used
        public let processingDuration: Double? // Processing duration in seconds
        public let modelUsed: String?          // Model name used
        public let rawResponse: String?        // Raw LLM response

        public init(
            id: UUID = UUID(), 
            timestamp: Date = Date(), 
            dedupeKey: String, 
            title: String, 
            details: String? = nil, 
            eventId: String? = nil, 
            priority: Double? = nil,
            tokensUsed: Int? = nil,
            processingDuration: Double? = nil,
            modelUsed: String? = nil,
            rawResponse: String? = nil
        ) {
            self.id = id
            self.timestamp = timestamp
            self.dedupeKey = dedupeKey
            self.title = title
            self.details = details
            self.eventId = eventId
            self.priority = priority
            self.tokensUsed = tokensUsed
            self.processingDuration = processingDuration
            self.modelUsed = modelUsed
            self.rawResponse = rawResponse
        }
    }
    public var history: [HistoryEntry]
    // Optional metadata: human-friendly description and preferred model for processing
    public var sourceDescription: String?
    /// Security-scoped bookmark for local file/folder sources picked through Files.
    public var securityBookmarkData: Data?
    public var preferredModel: String?
    // User-provided prompt to guide LLM when explaining/deciding about this data source
    public var userPrompt: String?
    // Optional link to a `PromptTemplate`.
    public var promptTemplateID: UUID?
    // Scheduling
    public var scheduledTimeISO: String? // ISO8601 string for time component
    public var scheduleFrequency: String? // none/daily/weekly
    /// For weekly schedules, which weekday to run on (Calendar weekday 1...7).
    public var scheduledWeekday: Int?
    /// Exact execution date for a one-off automation.
    public var scheduledDate: Date?
    /// Last modification time — used for server-side conflict resolution during sync.
    public var updatedAt: Date
    /// Default access behavior. A conversation may persist an explicit override.
    public var accessPolicy: URLDataSourceAccessPolicy
    /// Persisted result of the first successful real read.
    public var validationContract: URLDataSourceValidationContract?
    public var accessOwnerID: String?
    public var accessRevision: UInt64
    public var accessGrants: [AgentCoreResourceGrant]

    /// Application projection revision. Never encoded into source metadata.
    public var canonicalRevision: Int64? = nil

    private enum CodingKeys: String, CodingKey {
        case id
        case url
        case format
        case kind
        case name
        case enabled
        case lastFetchDate
        case lastError
        case consecutiveFailures
        case isRefreshing
        case history
        case sourceDescription
        case securityBookmarkData
        case preferredModel
        case userPrompt
        case promptTemplateID
        case scheduledTimeISO
        case scheduleFrequency
        case scheduledWeekday
        case scheduledDate
        case updatedAt
        case accessPolicy
        case validationContract
        case accessOwnerID
        case accessRevision
        case accessGrants
    }

    public init(
        id: UUID = UUID(),
        url: String,
        format: URLDataFormat,
        kind: DataSourceKind? = nil,
        name: String,
        enabled: Bool = true,
        lastFetchDate: Date? = nil,
        lastError: String? = nil
    ) {
        self.id = id
        self.url = url
        self.format = format
        self.kind = kind ?? Self.inferKind(format: format, url: url)
        self.name = name
        self.enabled = enabled
        self.lastFetchDate = lastFetchDate
        self.lastError = lastError
        self.sourceDescription = nil
        self.securityBookmarkData = nil
        self.preferredModel = nil
        self.scheduledTimeISO = nil
        self.scheduleFrequency = nil
        self.scheduledWeekday = nil
        self.scheduledDate = nil
        self.userPrompt = nil
        self.promptTemplateID = nil
        self.history = []
        self.updatedAt = Date()
        self.accessPolicy = .conversationOptIn
        self.validationContract = nil
        self.accessOwnerID = nil
        self.accessRevision = 1
        self.accessGrants = []
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try container.decode(UUID.self, forKey: .id)
        self.url = try container.decode(String.self, forKey: .url)
        self.format = try container.decode(URLDataFormat.self, forKey: .format)
        self.kind = try container.decode(DataSourceKind.self, forKey: .kind)
        self.name = try container.decode(String.self, forKey: .name)
        self.enabled = try container.decode(Bool.self, forKey: .enabled)
        self.lastFetchDate = try container.decodeIfPresent(Date.self, forKey: .lastFetchDate)
        self.lastError = try container.decodeIfPresent(String.self, forKey: .lastError)
        self.consecutiveFailures = try container.decode(Int.self, forKey: .consecutiveFailures)
        self.isRefreshing = try container.decode(Bool.self, forKey: .isRefreshing)
        self.history = try container.decode([HistoryEntry].self, forKey: .history)
        self.sourceDescription = try container.decodeIfPresent(String.self, forKey: .sourceDescription)
        self.securityBookmarkData = try container.decodeIfPresent(Data.self, forKey: .securityBookmarkData)
        self.preferredModel = try container.decodeIfPresent(String.self, forKey: .preferredModel)
        self.userPrompt = try container.decodeIfPresent(String.self, forKey: .userPrompt)
        self.promptTemplateID = try container.decodeIfPresent(UUID.self, forKey: .promptTemplateID)
        self.scheduledTimeISO = try container.decodeIfPresent(String.self, forKey: .scheduledTimeISO)
        self.scheduleFrequency = try container.decodeIfPresent(String.self, forKey: .scheduleFrequency)
        self.scheduledWeekday = try container.decodeIfPresent(Int.self, forKey: .scheduledWeekday)
        self.scheduledDate = try container.decodeIfPresent(Date.self, forKey: .scheduledDate)
        self.updatedAt = try container.decode(Date.self, forKey: .updatedAt)
        self.accessPolicy = try container.decode(URLDataSourceAccessPolicy.self, forKey: .accessPolicy)
        self.validationContract = try container.decodeIfPresent(URLDataSourceValidationContract.self, forKey: .validationContract)
        self.accessOwnerID = try container.decodeIfPresent(String.self, forKey: .accessOwnerID)
        self.accessRevision = try container.decode(UInt64.self, forKey: .accessRevision)
        self.accessGrants = try container.decode([AgentCoreResourceGrant].self, forKey: .accessGrants)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(url, forKey: .url)
        try container.encode(format, forKey: .format)
        try container.encode(kind, forKey: .kind)
        try container.encode(name, forKey: .name)
        try container.encode(enabled, forKey: .enabled)
        try container.encodeIfPresent(lastFetchDate, forKey: .lastFetchDate)
        try container.encodeIfPresent(lastError, forKey: .lastError)
        try container.encode(consecutiveFailures, forKey: .consecutiveFailures)
        try container.encode(isRefreshing, forKey: .isRefreshing)
        try container.encode(history, forKey: .history)
        try container.encodeIfPresent(sourceDescription, forKey: .sourceDescription)
        try container.encodeIfPresent(securityBookmarkData, forKey: .securityBookmarkData)
        try container.encodeIfPresent(preferredModel, forKey: .preferredModel)
        try container.encodeIfPresent(userPrompt, forKey: .userPrompt)
        try container.encodeIfPresent(promptTemplateID, forKey: .promptTemplateID)
        try container.encodeIfPresent(scheduledTimeISO, forKey: .scheduledTimeISO)
        try container.encodeIfPresent(scheduleFrequency, forKey: .scheduleFrequency)
        try container.encodeIfPresent(scheduledWeekday, forKey: .scheduledWeekday)
        try container.encodeIfPresent(scheduledDate, forKey: .scheduledDate)
        try container.encode(updatedAt, forKey: .updatedAt)
        try container.encode(accessPolicy, forKey: .accessPolicy)
        try container.encodeIfPresent(validationContract, forKey: .validationContract)
        try container.encodeIfPresent(accessOwnerID, forKey: .accessOwnerID)
        try container.encode(accessRevision, forKey: .accessRevision)
        try container.encode(accessGrants, forKey: .accessGrants)
    }

    public static func inferKind(format: URLDataFormat, url: String) -> DataSourceKind {
        if url.hasPrefix("file://") {
            return url.hasSuffix("/") ? .folder : .file
        }
        switch format {
        case .json:
            return .api
        case .icalendar:
            return .calendarFeed
        case .rss:
            return .feed
        case .html:
            return .website
        case .search:
            return .webSearch
        }
    }
}

public extension URLDataSource {
    func isEnabledForConversation(explicitOverride: Bool?) -> Bool {
        if accessPolicy == .blocked { return false }
        if let explicitOverride { return explicitOverride }
        switch accessPolicy {
        case .defaultEnabled:
            return true
        case .conversationOptIn, .askWhenRelevant:
            return false
        case .blocked:
            return false
        }
    }

    var hasScheduleConfiguration: Bool {
        guard let freq = scheduleFrequency, !freq.isEmpty, freq != "none" else { return false }
        return scheduledTimeISO != nil
    }

    var isScheduledAutomation: Bool {
        hasScheduleConfiguration
    }

    var isLocalReadableSource: Bool {
        kind == .file || kind == .folder || url.hasPrefix("file://")
    }

    func resolvedLocalURL() throws -> URL {
        if let securityBookmarkData {
            var stale = false
            let resolved = try URL(
                resolvingBookmarkData: securityBookmarkData,
                options: [],
                relativeTo: nil,
                bookmarkDataIsStale: &stale
            )
            return resolved
        }
        guard let localURL = URL(string: url), localURL.isFileURL else {
            throw URLFetcherError.invalidURL
        }
        return localURL
    }

    var contextDisplaySubtitle: String {
        if kind == .webSearch {
            return sourceDescription ?? url
        }
        return url
    }

    var capabilities: [DataSourceCapability] {
        var result: [DataSourceCapability] = [.chatContext, .localOnly]
        if isScheduledAutomation {
            result.append(.scheduledOperation)
            result.append(.notification)
        }
        return result
    }
}

extension URLDataSource: ReadableDataSource {
    public var readableSourceID: UUID { id }
    public var readableKind: DataSourceKind { kind }
    public var readableCapabilities: [DataSourceCapability] { capabilities }

    public func readContextItems(limit: Int) async throws -> [ReadableDataSourceItem] {
        let boundedLimit = max(0, limit)
        return history.suffix(boundedLimit).map { entry in
            ReadableDataSourceItem(
                id: entry.id.uuidString,
                title: entry.title,
                summary: entry.details,
                content: entry.rawResponse,
                sourceURL: url,
                metadata: [
                    "sourceId": id.uuidString,
                    "sourceName": name,
                    "kind": kind.rawValue,
                    "format": format.rawValue
                ]
            )
        }
    }
}

extension URLFetcherService {
    /// Attempt to fetch the URL and detect its data format and produce a preview of parsed events.
    /// - Parameters:
    ///   - urlString: URL to fetch
    ///   - preferredFormat: User-specified format (optional). If provided, only try this format. If nil, auto-detect.
    public func detectFormatAndPreview(urlString: String, preferredFormat: URLDataFormat? = nil) async throws -> (format: URLDataFormat, preview: [RawEvent], description: String?) {
        guard let url = URL(string: urlString) else { throw URLFetcherError.invalidURL }

        if url.isFileURL {
            let source = URLDataSource(
                url: urlString,
                format: preferredFormat ?? .html,
                kind: url.hasDirectoryPath ? .folder : .file,
                name: url.lastPathComponent.isEmpty ? "Local Data Source" : url.lastPathComponent
            )
            let preview = try localFileEvents(from: source, limit: 5)
            return (
                preferredFormat ?? .html,
                preview,
                url.hasDirectoryPath
                    ? "已读取本地文件夹中的 \(preview.count) 个文本条目"
                    : "已读取本地文件：\(url.lastPathComponent)"
            )
        }

        var request = URLRequest(url: url, timeoutInterval: timeout)
        configureRequest(&request)
        let (data, response) = try await IOSBoundedDataSourceBody.load(request, using: urlSession)
        guard let httpResponse = response as? HTTPURLResponse, (200...299).contains(httpResponse.statusCode) else {
            throw URLFetcherError.httpError(statusCode: (response as? HTTPURLResponse)?.statusCode ?? -1)
        }

        if let format = preferredFormat {
            let preview: [RawEvent]
            let description: String
            
            switch format {
            case .json:
                preview = try parseJSON(data)
                description = L10n.format("url_fetcher.format.json_found", preview.count)
            case .icalendar:
                preview = try parseICalendar(data)
                description = L10n.format("url_fetcher.format.ical_found", preview.count)
            case .rss:
                preview = try parseRSS(data)
                description = L10n.format("url_fetcher.format.rss_found", preview.count)
            case .html:
                preview = try parseHTML(data, baseURL: url)
                description = L10n.format("url_fetcher.format.html_found", preview.count)
            case .search:
                throw URLFetcherError.parseError(message: L10n.format("url_fetcher.error.parse_failed", format.rawValue.uppercased()))
            }
            
            if preview.isEmpty {
                throw URLFetcherError.parseError(message: L10n.format("url_fetcher.error.parse_failed", format.rawValue.uppercased()))
            }
            
            return (format, preview, description)
        }

        // Try JSON
        if let jsonPreview = try? parseJSON(data) {
            return (.json, jsonPreview, L10n.format("url_fetcher.json.description", jsonPreview.count))
        }

        // Try iCalendar
        if let icsPreview = try? parseICalendar(data), !icsPreview.isEmpty {
            return (.icalendar, icsPreview, L10n.format("url_fetcher.ical.description", icsPreview.count))
        }

        // Try RSS
        if let rssPreview = try? parseRSS(data), !rssPreview.isEmpty {
            return (.rss, rssPreview, L10n.format("url_fetcher.rss.description", rssPreview.count))
        }

        // Fallback to HTML parser
        if let htmlPreview = try? parseHTML(data, baseURL: url), !htmlPreview.isEmpty {
            return (.html, htmlPreview, L10n.string("url_fetcher.html.description"))
        }

        // If nothing matched, throw parse error
        throw URLFetcherError.parseError(message: L10n.string("url_fetcher.error.unknown_format"))
    }
}

/// URL fetcher service
public final class URLFetcherService: ObservableObject {
    // Persistence owns its queued work independently; teardown only releases
    // values, without publishing UI or running actor-bound cleanup. Do not
    // synthesize an isolated deinit on affected older Swift runtimes (#88036).
    nonisolated deinit {}
    
    private let logger: Logger
    private let maxURLs: Int
    private let timeout: TimeInterval
    private let persistenceQueue = DispatchQueue(label: "com.nola.urlfetcher.persistence", qos: .utility)
    // Seeded once during initialization; subsequently owned by persistenceQueue.
    // Advance only from this instance's successful commits, never by silently rebasing.
    private var sourceRevisions: [String: Int64] = [:]
    private let maxHistoryEntries: Int
    private let userDefaults: UserDefaults
    private let applicationRuntime: IOSApplicationRuntime?
    
    /// Stored URL sources
    @Published public var sources: [URLDataSource] = []
    @Published public private(set) var usageRecords: [URLDataSourceUsageRecord] = []

    // MARK: - Schedule validation

    private func validatedScheduleFields(for source: URLDataSource) -> (normalized: URLDataSource, didChange: Bool) {
        var updated = source
        var changed = false

        let freqNorm = ScheduledTimeParser.normalizeFrequency(updated.scheduleFrequency)
        if freqNorm == nil {
            if updated.scheduleFrequency != nil || updated.scheduledTimeISO != nil || updated.scheduledWeekday != nil {
                changed = true
            }
            updated.scheduleFrequency = nil
            updated.scheduledTimeISO = nil
            updated.scheduledWeekday = nil
            return (updated, changed)
        }

        // Frequency is enabled; require a valid time.
        guard let rawTime = updated.scheduledTimeISO,
              let timeNorm = ScheduledTimeParser.normalizeToHHmm(rawTime)
        else {
            if updated.scheduleFrequency != nil || updated.scheduledTimeISO != nil {
                logger.log(.warn, "URLFetcher", "Invalid scheduledTimeISO; clearing schedule", metadata: [
                    "name": updated.name,
                    "id": updated.id.uuidString,
                    "freq": updated.scheduleFrequency ?? "nil",
                    "timeISO": updated.scheduledTimeISO ?? "nil"
                ])
                changed = true
            }
            updated.scheduleFrequency = nil
            updated.scheduledTimeISO = nil
            updated.scheduledWeekday = nil
            return (updated, changed)
        }

        if updated.scheduleFrequency != freqNorm {
            updated.scheduleFrequency = freqNorm
            changed = true
        }
        if updated.scheduledTimeISO != timeNorm {
            updated.scheduledTimeISO = timeNorm
            changed = true
        }

        // Weekly schedules must specify a weekday.
        if freqNorm == "weekly" {
            let wd = updated.scheduledWeekday
            if wd == nil || wd! < 1 || wd! > 7 {
                updated.scheduledWeekday = Calendar.current.component(.weekday, from: Date())
                changed = true
            }
        } else {
            if updated.scheduledWeekday != nil {
                updated.scheduledWeekday = nil
                changed = true
            }
        }

        return (updated, changed)
    }

    /// Normalize URL for comparison (remove trailing slash, fragment, etc.)
    private func normalizeURL(_ urlString: String) -> String {
        guard let url = URL(string: urlString) else { return urlString }
        
        var components = URLComponents(url: url, resolvingAgainstBaseURL: true)
        // Remove fragment
        components?.fragment = nil
        // Remove trailing slash from path
        if let path = components?.path, path.hasSuffix("/"), path.count > 1 {
            components?.path = String(path.dropLast())
        }
        
        return components?.url?.absoluteString ?? urlString
    }
    
    /// Custom URLSession with browser-like configuration
    private lazy var urlSession: URLSession = {
        let config = URLSessionConfiguration.default
        config.httpShouldSetCookies = true
        config.httpCookieAcceptPolicy = .always
        config.httpMaximumConnectionsPerHost = 4
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: config)
    }()
    
    /// Get Accept-Language header value from system preferences
    private var acceptLanguage: String {
        let preferredLanguages = Locale.preferredLanguages
        if preferredLanguages.isEmpty {
            return "en-US,en;q=0.9"
        }
        
        // Format: "zh-Hans-CN,zh-Hans;q=0.9,en-US;q=0.8,en;q=0.7"
        var result: [String] = []
        for (index, lang) in preferredLanguages.prefix(4).enumerated() {
            let quality = 1.0 - (Double(index) * 0.1)
            if index == 0 {
                result.append(lang)
            } else {
                result.append("\(lang);q=\(String(format: "%.1f", quality))")
            }
        }
        return result.joined(separator: ",")
    }
    
    /// Configure request with browser-like headers
    private func configureRequest(_ request: inout URLRequest, referer: String? = nil) {
        // Special-case: Bing Search API expects JSON and an API key header.
        // We keep this opt-in by URL: only apply when the host matches the Bing endpoint.
        if let host = request.url?.host?.lowercased(), host == "api.bing.microsoft.com" {
            request.setValue("application/json", forHTTPHeaderField: "Accept")
            // Avoid brotli for API calls; some stacks may not transparently decode it.
            request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")

            let subscriptionHeader = "Ocp-Apim-Subscription-Key"
            if request.value(forHTTPHeaderField: subscriptionHeader) == nil {
                if let data = KeychainService.read(service: "nola.bing", account: "apiKey"),
                   let key = String(data: data, encoding: .utf8),
                   !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    request.setValue(key, forHTTPHeaderField: subscriptionHeader)
                } else {
                    logger.log(.warn, "URLFetcher", "Bing API key missing (KeychainService: service=nola.bing account=apiKey)")
                }
            }
            return
        }

        request.setValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36", forHTTPHeaderField: "User-Agent")
        request.setValue("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8", forHTTPHeaderField: "Accept")
        request.setValue(acceptLanguage, forHTTPHeaderField: "Accept-Language")
        request.setValue("gzip, deflate, br", forHTTPHeaderField: "Accept-Encoding")
        request.setValue("1", forHTTPHeaderField: "DNT")
        request.setValue("keep-alive", forHTTPHeaderField: "Connection")
        request.setValue("?1", forHTTPHeaderField: "Sec-Fetch-Site")
        request.setValue("navigate", forHTTPHeaderField: "Sec-Fetch-Mode")
        request.setValue("document", forHTTPHeaderField: "Sec-Fetch-Dest")
        request.setValue("Mozilla/5.0", forHTTPHeaderField: "Sec-CH-UA-Mobile")
        request.setValue("\"Chromium\";v=\"120\", \"Google Chrome\";v=\"120\", \"Not-A.Brand\";v=\"99\"", forHTTPHeaderField: "Sec-CH-UA")
        request.setValue("\"macOS\"", forHTTPHeaderField: "Sec-CH-UA-Platform")
        request.setValue("same-origin", forHTTPHeaderField: "Sec-Fetch-User")
        request.setValue("1", forHTTPHeaderField: "Upgrade-Insecure-Requests")
        
        if let referer = referer {
            request.setValue(referer, forHTTPHeaderField: "Referer")
        }
    }
    
    public convenience init(
        logger: Logger,
        maxURLs: Int = 10,
        timeout: TimeInterval = 30,
        maxHistoryEntries: Int = 90,
        userDefaults: UserDefaults = .standard
    ) {
        self.init(
            logger: logger,
            maxURLs: maxURLs,
            timeout: timeout,
            maxHistoryEntries: maxHistoryEntries,
            userDefaults: userDefaults,
            applicationRuntime: IOSApplicationRuntime.shared
        )
    }

    init(
        logger: Logger,
        maxURLs: Int = 10,
        timeout: TimeInterval = 30,
        maxHistoryEntries: Int = 90,
        userDefaults: UserDefaults = .standard,
        applicationRuntime: IOSApplicationRuntime?
    ) {
        self.logger = logger
        self.maxURLs = maxURLs
        self.timeout = timeout
        self.maxHistoryEntries = maxHistoryEntries
        self.userDefaults = userDefaults
        self.applicationRuntime = applicationRuntime
        loadSources()
        loadUsageRecords()
    }

    /// Experimental: when true, call into the Rust core to process RawEvents into ProcessedEvent with dedupeKey and priority.
    public var useRustProcessing: Bool = false
    
    // MARK: - Source Management

    private func sourceOwnedByLocalPrincipal(_ source: URLDataSource) -> URLDataSource {
        var value = source
        if value.accessOwnerID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false {
            value.accessOwnerID = AgentCoreResourceAccess.localPrincipal().id
        }
        value.accessRevision = max(1, value.accessRevision)
        return value
    }

    private func nextAccessRevision(_ revision: UInt64) -> UInt64 {
        revision == UInt64.max ? UInt64.max : revision + 1
    }

    private func canAccess(
        _ source: URLDataSource,
        permission: AgentCoreResourcePermission
    ) -> Bool {
        let actor = AgentCoreResourceAccess.localPrincipal()
        let ownerID = source.accessOwnerID ?? actor.id
        let resource = AgentCoreManagedResourceIdentity(
            kind: .dataSource,
            resourceID: source.id.uuidString.lowercased(),
            owner: AgentCoreResourcePrincipal(kind: "local_user", id: ownerID),
            revision: max(1, source.accessRevision),
            contentDigest: nil
        )
        if let decision = AgentCoreResourceAccess.evaluate(
            resource: resource,
            permission: permission,
            grants: source.accessGrants,
            actor: actor
        ) {
            return decision.allowed
        }
        return ownerID == actor.id
    }
    
    /// Add a new URL source
    @MainActor
    public func addSource(_ source: URLDataSource) throws {
        // NOTE: URLFetcherService is initialized with an explicit maxURLs. Keep this
        // service free of global config singletons so it stays testable/deterministic.
        let effectiveLimit = max(1, maxURLs)
        guard sources.count < effectiveLimit else {
            throw URLFetcherError.tooManySources(max: effectiveLimit)
        }
        
        // Validate URL
        guard let url = URL(string: source.url) else {
            throw URLFetcherError.invalidURL
        }
        
        // Prefer HTTPS
        if url.scheme != "https" {
            logger.log(.warn, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.5a5f0ecf", comment: "AUTOGEN"), metadata: ["url": source.url])
        }
        
        var (normalized, didChange) = validatedScheduleFields(for: source)
        normalized = sourceOwnedByLocalPrincipal(normalized)
        if didChange {
            logger.log(.info, "URLFetcher", "Normalized schedule fields on add", metadata: [
                "name": normalized.name,
                "id": normalized.id.uuidString,
                "freq": normalized.scheduleFrequency ?? "nil",
                "timeISO": normalized.scheduledTimeISO ?? "nil"
            ])
        }

        sources.append(normalized)
        persistSource(normalized)
        notifySourcesUpdated()
        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.371f0e30", comment: "AUTOGEN"), metadata: ["name": source.name, "url": source.url])
    }
    
    /// Remove a source
    /// Commit an editor submission before publishing it or reporting success.
    @MainActor
    public func saveSource(_ source: URLDataSource) async throws -> URLDataSource {
        guard URL(string: source.url) != nil else { throw URLFetcherError.invalidURL }
        var (prepared, _) = validatedScheduleFields(for: source)
        if let previous = sources.first(where: { $0.id == source.id }) {
            guard source.canonicalRevision != nil else {
                throw URLFetcherError.sourceRevisionUnavailable
            }
            guard canAccess(previous, permission: .editContent) else {
                throw CocoaError(.fileWriteNoPermission)
            }
            prepared.accessOwnerID = previous.accessOwnerID ?? AgentCoreResourceAccess.localPrincipal().id
            prepared.accessRevision = nextAccessRevision(max(1, previous.accessRevision))
            prepared.accessGrants = previous.accessGrants
            prepared.updatedAt = Date()
        } else {
            guard sources.count < max(1, maxURLs) else {
                throw URLFetcherError.tooManySources(max: max(1, maxURLs))
            }
            prepared = sourceOwnedByLocalPrincipal(prepared)
        }
        let submission = prepared
        let committed: URLDataSource = try await withCheckedThrowingContinuation { continuation in
            persistenceQueue.async { [self] in
                do {
                    let record = try commitSource(submission, expectedRevision: submission.canonicalRevision)
                    guard let projection = record.iosURLSource else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                    continuation.resume(returning: projection)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
        if let index = sources.firstIndex(where: { $0.id == committed.id }) {
            sources[index] = committed
        } else {
            sources.append(committed)
        }
        notifySourcesUpdated()
        return committed
    }

    /// Compatibility entry point. New UI callers await removeSourceConfirmed
    /// so rejected deletes can be presented without hiding the source.
    @MainActor
    public func removeSource(id: UUID) {
        do {
            try enqueueSourceDeletion(id: id, expectedRevision: nil) { [weak self] result in
                Task { @MainActor in
                    guard let self else { return }
                    switch result {
                    case .success: self.publishSourceDeletion(id: id)
                    case .failure(let error):
                        self.logger.log(.error, "URLFetcher", "Data source deletion failed", metadata: ["id": id.uuidString, "error": error.localizedDescription])
                    }
                }
            }
        } catch {
            logger.log(.error, "URLFetcher", "Data source deletion denied", metadata: ["id": id.uuidString, "error": error.localizedDescription])
        }
    }

    /// Persistence and revision checks complete before the local projection changes.
    @MainActor
    public func removeSourceConfirmed(id: UUID, expectedRevision: Int64? = nil) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            do {
                try enqueueSourceDeletion(id: id, expectedRevision: expectedRevision) { result in
                    continuation.resume(with: result)
                }
            } catch { continuation.resume(throwing: error) }
        }
        publishSourceDeletion(id: id)
    }

    @MainActor
    private func publishSourceDeletion(id: UUID) {
        sources.removeAll { $0.id == id }
        notifySourcesUpdated()
        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.c40ac45d", comment: "AUTOGEN"), metadata: ["id": id.uuidString])
    }

    /// Update a source
    @MainActor
    public func updateSource(_ source: URLDataSource) {
        var didUpdate = false
        let (normalized, didChange) = validatedScheduleFields(for: source)
        if let index = sources.firstIndex(where: { $0.id == source.id }) {
            let previous = sources[index]
            guard canAccess(previous, permission: .editContent) else {
                logger.log(.warn, "URLFetcher", "Data source edit denied", metadata: ["id": source.id.uuidString])
                return
            }
            var updated = normalized
            updated.accessOwnerID = previous.accessOwnerID ?? AgentCoreResourceAccess.localPrincipal().id
            updated.accessRevision = nextAccessRevision(max(1, previous.accessRevision))
            updated.accessGrants = previous.accessGrants
            updated.updatedAt = Date()
            sources[index] = updated
            persistSource(updated)
            didUpdate = true
        }
        guard didUpdate else { return }
        if didChange {
            logger.log(.info, "URLFetcher", "Normalized schedule fields on update", metadata: [
                "name": normalized.name,
                "id": normalized.id.uuidString,
                "freq": normalized.scheduleFrequency ?? "nil",
                "timeISO": normalized.scheduledTimeISO ?? "nil"
            ])
        }
        notifySourcesUpdated()
        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.5067150c", comment: "AUTOGEN"), metadata: ["name": source.name])
    }
    
    /// Toggle source enabled state
    @MainActor
    public func toggleSource(id: UUID) {
        var didToggle = false
        if let index = sources.firstIndex(where: { $0.id == id }) {
            guard canAccess(sources[index], permission: .editContent) else {
                logger.log(.warn, "URLFetcher", "Data source edit denied", metadata: ["id": id.uuidString])
                return
            }
            sources[index].enabled.toggle()
            sources[index].accessRevision = nextAccessRevision(max(1, sources[index].accessRevision))
            sources[index].updatedAt = Date()
            persistSource(sources[index])
            didToggle = true
        }
        if didToggle {
            notifySourcesUpdated()
        }
    }

    @MainActor
    public func recordUsage(_ record: URLDataSourceUsageRecord) {
        usageRecords.insert(record, at: 0)
        if usageRecords.count > 200 {
            usageRecords.removeLast(usageRecords.count - 200)
        }
        persistenceQueue.async { [applicationRuntime] in
            try? applicationRuntime?.appendDataSourceUsage(record.canonicalRecord)
        }
    }

    /// Append a history entry for the source that matches the provided URL.
    ///
    /// - Important: `sources` is `@Published` and must be mutated on the main actor.
    @MainActor
    public func appendHistoryEntry(forURL url: String, entry: URLDataSource.HistoryEntry) {
        guard !url.isEmpty else {
            logger.log(.debug, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.760bf4a9", comment: "AUTOGEN"))
            return
        }
        
        guard let index = findSourceIndex(byURL: url) else {
            let normalizedURL = normalizeURL(url)
            logger.log(.warn, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.18ba2c58", comment: "AUTOGEN"), metadata: [
                "url": url,
                "normalizedURL": normalizedURL,
                "availableSources": sources.map { $0.url }.joined(separator: ", ")
            ])
            return
        }

        appendHistoryEntry(at: index, originalURL: url, entry: entry)
    }

    /// Append a history entry for the source that matches sourceId first, then URL as fallback.
    ///
    /// This is preferred for server push results where backend URL can differ from local URL text.
    @MainActor
    public func appendHistoryEntry(forSourceId sourceId: String, fallbackURL: String?, entry: URLDataSource.HistoryEntry) {
        let trimmedSourceId = sourceId.trimmingCharacters(in: .whitespacesAndNewlines)

        if !trimmedSourceId.isEmpty,
           let sid = UUID(uuidString: trimmedSourceId),
           let index = sources.firstIndex(where: { $0.id == sid }) {
            appendHistoryEntry(at: index, originalURL: sources[index].url, entry: entry)
            return
        }

        if let fallbackURL,
           let index = findSourceIndex(byURL: fallbackURL) {
            appendHistoryEntry(at: index, originalURL: fallbackURL, entry: entry)
            return
        }

        logger.log(.warn, "URLFetcher", "Failed to append history: sourceId/url unmatched", metadata: [
            "sourceId": trimmedSourceId,
            "fallbackURL": fallbackURL ?? "nil",
            "availableSourceIds": sources.map { $0.id.uuidString }.joined(separator: ",")
        ])
    }

    @MainActor
    private func findSourceIndex(byURL url: String) -> Int? {
        let normalizedURL = normalizeURL(url)
        return sources.firstIndex(where: {
            $0.url == url || normalizeURL($0.url) == normalizedURL
        })
    }

    @MainActor
    private func appendHistoryEntry(at index: Int, originalURL: String, entry: URLDataSource.HistoryEntry) {

        var s = sources[index]

        // Deduplicate by dedupeKey to prevent duplicate writes on retries/replays.
        if s.history.contains(where: { $0.dedupeKey == entry.dedupeKey }) {
            logger.log(.debug, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.d1814124", comment: "AUTOGEN"), metadata: [
                "name": s.name,
                "dedupeKey": entry.dedupeKey
            ])
            return
        }

        s.history.append(entry)

        if s.history.count > maxHistoryEntries {
            s.history = Array(s.history.suffix(maxHistoryEntries))
            logger.log(.debug, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.16c8f46b", comment: "AUTOGEN"), metadata: [
                "name": s.name,
                "maxEntries": "\(maxHistoryEntries)"
            ])
        }

        sources[index] = s
        persistSource(s)

        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.fc2a6707", comment: "AUTOGEN"), metadata: [
            "name": s.name,
            "entryId": entry.id.uuidString,
            "url": originalURL,
            "historyCount": "\(s.history.count)"
        ])

        // Encode on the caller thread to avoid capturing non-Sendable `entry` inside a @Sendable GCD closure.
        let entryData: Data?
        do {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            entryData = try encoder.encode(entry)
        } catch {
            logger.log(.error, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.0bc136b4", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            entryData = nil
        }

        let sourceId = s.id.uuidString
        let sourceName = s.name
        persistenceQueue.async { [logger] in
            guard let data = entryData else { return }
            let success = URLHistoryStore.shared.append(sourceID: sourceId, entryData: data)
            if !success {
                logger.log(.warn, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.7990486a", comment: "AUTOGEN"), metadata: ["source": sourceName])
            }
        }
    }
    
    // MARK: - Fetching
    
    /// Fetch events from a single source
    public func fetchEvents(from source: URLDataSource) async throws -> [RawEvent] {
        guard source.enabled else { return [] }
        return try await refreshCanonicalSource(from: source)
    }

    private func refreshCanonicalSource(from source: URLDataSource) async throws -> [RawEvent] {
        guard let revision = source.canonicalRevision else {
            throw URLFetcherError.sourceRevisionUnavailable
        }
        try Task.checkCancellation()
        let pending = try await commitReadCommand { runtime in
            return try runtime.requestDataSourceRefresh(
                id: source.id.uuidString, expectedRevision: revision,
                reason: "source_refresh", requestedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
        }
        publishReadProjection(pending)
        guard let token = pending.readState.readToken, var readable = pending.iosURLSource else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        // Admission was established by the default-enabled entry point or the
        // Application's conversation selection, not by this Host I/O adapter.
        readable.enabled = true
        var contentURL: URL?
        do {
            try Task.checkCancellation()
            let events = try await fetchEventsNetworkAndParse(from: readable)
            try Task.checkCancellation()
            let observedAt = Int64(Date().timeIntervalSince1970 * 1_000)
            let result: IOSCanonicalDataSourceReadResult
            if events.isEmpty {
                result = .empty(observedAtMs: observedAt, schemaVersion: 1)
            } else {
                guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
                let encoder = JSONEncoder()
                // RawEvent Date values must survive the local content round trip
                // exactly; ISO8601's default encoder drops fractional seconds.
                encoder.dateEncodingStrategy = .deferredToDate
                let content = try encoder.encode(events)
                guard content.count <= 8 * 1_048_576 else { throw URLError(.dataLengthExceedsMaximum) }
                let directory = applicationRuntime.dataSourceContentDirectory
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                let version = UUID().uuidString
                let url = directory.appendingPathComponent(version).appendingPathExtension("json")
                contentURL = url
                try content.write(to: url, options: .atomic)
                try Task.checkCancellation()
                result = .succeeded(
                    contentReference: .init(uri: url.absoluteString, mimeType: "application/json", byteCount: UInt64(content.count), contentVersion: version),
                    observedAtMs: observedAt, schemaVersion: 1, recordCount: UInt64(events.count)
                )
            }
            let completed = try await commitReadCommand { runtime in
                try runtime.recordDataSourceRead(id: pending.id, readToken: token, result: result, recordedAtMs: observedAt)
            }
            publishReadProjection(completed)
            return events
        } catch {
            // This settlement deliberately runs even when the caller is cancelled.
            // The Application token, not UI state, decides whether the result is still current.
            let cancelled = error is CancellationError || (error as? URLError)?.code == .cancelled
            do {
                let failed = try await commitReadCommand { runtime in
                    try runtime.recordDataSourceRead(
                        id: pending.id, readToken: token,
                        result: .failed(code: cancelled ? "cancelled" : "read_failed", diagnostic: error.localizedDescription),
                        recordedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
                    )
                }
                publishReadProjection(failed)
            } catch {
                logger.log(.warn, "URLFetcher", "Read settlement rejected or unavailable", metadata: ["sourceId": pending.id, "error": error.localizedDescription])
            }
            // A command can commit before its response is decoded. Never remove a
            // content file unless a fresh Application read proves it is unreferenced.
            if let contentURL, let runtime = applicationRuntime {
                do {
                    let current = try runtime.listDataSources().first { $0.id == pending.id }
                    if current?.readState.contentReference?.uri != contentURL.absoluteString {
                        try FileManager.default.removeItem(at: contentURL)
                    }
                    if let current { publishReadProjection(current) }
                } catch {
                    logger.log(.warn, "URLFetcher", "Deferred uncommitted source content cleanup", metadata: ["sourceId": pending.id])
                }
            }
            throw error
        }
    }

    private func commitReadCommand(
        _ command: @escaping @Sendable (IOSApplicationRuntime) throws -> IOSCanonicalDataSource
    ) async throws -> IOSCanonicalDataSource {
        try await withCheckedThrowingContinuation { continuation in
            persistenceQueue.async { [self] in
                do {
                    guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
                    let record = try command(applicationRuntime)
                    sourceRevisions[record.id] = record.revision
                    continuation.resume(returning: record)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    @MainActor
    private func publishReadProjection(_ record: IOSCanonicalDataSource) {
        guard let projection = record.iosURLSource,
              let index = sources.firstIndex(where: { $0.id == projection.id }),
              (sources[index].canonicalRevision ?? 0) <= record.revision else { return }
        sources[index] = projection
        notifySourcesUpdated()
    }

    /// Host-only probe for a draft that has not yet acquired a validated contract.
    /// Normal reads must use fetchEvents so Application owns their lifecycle.
    func readSourceForValidation(from source: URLDataSource) async throws -> [RawEvent] {
        try await fetchEventsNetworkAndParse(from: source)
    }

    private func fetchEventsNetworkAndParse(from source: URLDataSource) async throws -> [RawEvent] {
        guard source.enabled else {
            logger.log(.debug, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.256d5e02", comment: "AUTOGEN"), metadata: ["name": source.name])
            return []
        }

        if source.isLocalReadableSource {
            return try localFileEvents(from: source)
        }

        guard let url = URL(string: source.url) else {
            throw URLFetcherError.invalidURL
        }

        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.7782eb43", comment: "AUTOGEN"), metadata: ["name": source.name, "format": source.format.rawValue])

        // Fetch data with timeout and proper headers to avoid 403 errors
        var request = URLRequest(url: url, timeoutInterval: timeout)
        configureRequest(&request, referer: url.scheme.map { "\($0)://\(url.host ?? "")" })
        let (data, response) = try await IOSBoundedDataSourceBody.load(request, using: urlSession)

        // Check response
        guard let httpResponse = response as? HTTPURLResponse else {
            throw URLFetcherError.invalidResponse
        }

        guard (200...299).contains(httpResponse.statusCode) else {
            throw URLFetcherError.httpError(statusCode: httpResponse.statusCode)
        }

        // Parse based on format
        var events: [RawEvent]
        switch source.format {
        case .json:
            events = try parseJSON(data)
        case .icalendar:
            events = try parseICalendar(data)
        case .rss:
            events = try parseRSS(data)
        case .html:
            events = try parseHTML(data, baseURL: url)
        case .search:
            events = []
        }

        events = events.map { event in
            var enrichedMetadata = event.metadata
            enrichedMetadata["sourceId"] = source.id.uuidString
            enrichedMetadata["sourceName"] = source.name
            return RawEvent(
                id: event.id,
                source: event.source,
                title: event.title,
                date: event.date,
                notes: event.notes,
                metadata: enrichedMetadata
            )
        }

        logger.log(.info, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.91f68cfb", comment: "AUTOGEN"), metadata: ["name": source.name, "count": "\(events.count)"])
        return events
    }

    private func localFileEvents(from source: URLDataSource, limit: Int = 20) throws -> [RawEvent] {
        let url = try source.resolvedLocalURL()

        let didAccess = url.startAccessingSecurityScopedResource()
        defer {
            if didAccess {
                url.stopAccessingSecurityScopedResource()
            }
        }

        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else {
            throw URLFetcherError.parseError(message: "本地数据源不存在：\(url.lastPathComponent)")
        }

        if isDirectory.boolValue {
            return try localFolderEvents(from: url, source: source, limit: limit)
        }

        return [try localFileEvent(from: url, source: source, relativePath: url.lastPathComponent)]
    }

    private func localFolderEvents(from folderURL: URL, source: URLDataSource, limit: Int) throws -> [RawEvent] {
        let keys: [URLResourceKey] = [.isDirectoryKey, .isRegularFileKey, .contentModificationDateKey, .fileSizeKey]
        guard let enumerator = FileManager.default.enumerator(
            at: folderURL,
            includingPropertiesForKeys: keys,
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else {
            throw URLFetcherError.parseError(message: "无法读取文件夹：\(folderURL.lastPathComponent)")
        }

        var events: [RawEvent] = []
        for case let fileURL as URL in enumerator {
            guard events.count < limit else { break }
            let values = try? fileURL.resourceValues(forKeys: Set(keys))
            guard values?.isRegularFile == true else { continue }
            guard let event = try? localFileEvent(
                from: fileURL,
                source: source,
                relativePath: localRelativePath(fileURL, base: folderURL)
            ) else {
                continue
            }
            events.append(event)
        }

        if events.isEmpty {
            return [
                RawEvent(
                    id: "\(source.id.uuidString):\(folderURL.path)",
                    source: .url,
                    title: source.name,
                    date: nil,
                    notes: "文件夹中暂未发现可读取的文本文件。",
                    metadata: localMetadata(for: folderURL, source: source, relativePath: folderURL.lastPathComponent)
                )
            ]
        }

        return events
    }

    private func localFileEvent(from fileURL: URL, source: URLDataSource, relativePath: String) throws -> RawEvent {
        let preview = try localTextPreview(from: fileURL)
        let values = try? fileURL.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey])
        var metadata = localMetadata(for: fileURL, source: source, relativePath: relativePath)
        if let size = values?.fileSize {
            metadata["fileSize"] = "\(size)"
        }

        return RawEvent(
            id: "\(source.id.uuidString):\(relativePath)",
            source: .url,
            title: source.kind == .folder ? relativePath : (source.name.isEmpty ? fileURL.lastPathComponent : source.name),
            date: values?.contentModificationDate,
            notes: preview,
            metadata: metadata
        )
    }

    private func localTextPreview(from fileURL: URL, maxBytes: Int = 64_000) throws -> String {
        let handle = try FileHandle(forReadingFrom: fileURL)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: maxBytes) ?? Data()
        guard let text = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else {
            throw URLFetcherError.parseError(message: "文件不是可读取的 UTF-8 文本：\(fileURL.lastPathComponent)")
        }
        return String(text.prefix(12_000))
    }

    private func localMetadata(for url: URL, source: URLDataSource, relativePath: String) -> [String: String] {
        [
            "sourceId": source.id.uuidString,
            "sourceName": source.name,
            "sourceKind": source.kind.rawValue,
            "sourceURL": source.url,
            "fileName": url.lastPathComponent,
            "relativePath": relativePath
        ]
    }

    private func localRelativePath(_ fileURL: URL, base: URL) -> String {
        let basePath = base.standardizedFileURL.path
        let filePath = fileURL.standardizedFileURL.path
        if filePath.hasPrefix(basePath) {
            let suffix = filePath.dropFirst(basePath.count).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            if !suffix.isEmpty { return suffix }
        }
        return fileURL.lastPathComponent
    }

    /// Fetch events and preprocess them through Core.
    public func fetchAndProcessEvents(from source: URLDataSource) async throws -> [ProcessedEvent] {
        let events = try await fetchEvents(from: source)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(events)
        guard let processed = RustInterop.processRawEvents(data) else {
            throw NSError(
                domain: "URLFetcherService",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Core event processing is unavailable."]
            )
        }
        return processed
    }
    
    /// Check if a source should be fetched now based on its schedule
    public func shouldFetchNow(_ source: URLDataSource) -> Bool {
        guard source.enabled else { return false }
        guard let freq = source.scheduleFrequency, freq != "none" else { return false }
        guard let timeISO = source.scheduledTimeISO else { return false }
        
        let now = Date()
        let calendar = Calendar.current
        
        // Parse scheduled time (prefer "HH:mm" but tolerate common variants)
        guard let hm = ScheduledTimeParser.parseHourMinute(timeISO) else { return false }
        let scheduledHour = hm.hour
        let scheduledMinute = hm.minute

        // Allow running slightly before the target (e.g. OS wakes us a few minutes early).
        let toleranceSec: TimeInterval = 5 * 60

        if freq == "once", let targetDate = source.scheduledDate {
            guard now >= targetDate.addingTimeInterval(-toleranceSec) else { return false }
            guard let lastFetch = source.lastFetchDate else { return true }
            return lastFetch < targetDate.addingTimeInterval(-toleranceSec)
        }

        func mostRecentDailyOccurrence() -> Date? {
            var comps = calendar.dateComponents([.year, .month, .day], from: now)
            comps.hour = scheduledHour
            comps.minute = scheduledMinute
            comps.second = 0
            guard let today = calendar.date(from: comps) else { return nil }
            if now >= today.addingTimeInterval(-toleranceSec) { return today }
            return calendar.date(byAdding: .day, value: -1, to: today) ?? today
        }

        func mostRecentWeeklyOccurrence(targetWeekday: Int) -> Date? {
            let nowWeekday = calendar.component(.weekday, from: now)
            let daysBack = (nowWeekday - targetWeekday + 7) % 7
            let baseDay = calendar.date(byAdding: .day, value: -daysBack, to: calendar.startOfDay(for: now)) ?? calendar.startOfDay(for: now)
            var comps = calendar.dateComponents([.year, .month, .day], from: baseDay)
            comps.hour = scheduledHour
            comps.minute = scheduledMinute
            comps.second = 0
            guard let candidate = calendar.date(from: comps) else { return nil }
            if now >= candidate.addingTimeInterval(-toleranceSec) { return candidate }
            return calendar.date(byAdding: .day, value: -7, to: candidate) ?? candidate
        }

        let scheduledMostRecent: Date? = {
            switch freq {
            case "daily":
                return mostRecentDailyOccurrence()
            case "weekly":
                let wd = source.scheduledWeekday ?? calendar.component(.weekday, from: now)
                return mostRecentWeeklyOccurrence(targetWeekday: wd)
            default:
                return nil
            }
        }()

        guard let scheduledMostRecent else { return false }
        guard now >= scheduledMostRecent.addingTimeInterval(-toleranceSec) else { return false }

        // One execution per scheduled occurrence; allow catch-up if we missed it.
        guard let lastFetch = source.lastFetchDate else { return true }
        return lastFetch < scheduledMostRecent.addingTimeInterval(-toleranceSec)
    }
    
    /// Get sources that should be fetched now based on their schedule
    public func getScheduledSources() -> [URLDataSource] {
        return sources.filter { $0.isScheduledAutomation && shouldFetchNow($0) }
    }

    /// Get sources that are configured to run on a schedule (regardless of whether they are due now).
    ///
    /// - Note: This is used for planning the next background task run time.
    public func getConfiguredScheduledSources() -> [URLDataSource] {
        return sources.filter { $0.enabled && $0.isScheduledAutomation }
    }
    
    /// Fetch events from all enabled sources
    public func fetchAllEvents() async throws -> [RawEvent] {
        var allEvents: [RawEvent] = []

        let snapshot = await MainActor.run { sources }
        for source in snapshot where source.enabled {
            do {
                let events = try await fetchEvents(from: source)
                allEvents.append(contentsOf: events)
            } catch {
                logger.log(.error, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.6292a3a8", comment: "AUTOGEN"), metadata: [
                    "name": source.name,
                    "error": error.localizedDescription
                ])
                
                // Update source with error
                var updatedSource = source
                updatedSource.lastError = error.localizedDescription
                await MainActor.run {
                    updateSource(updatedSource)
                }
            }
        }
        
        return allEvents
    }

    /// Fetch and process all enabled sources using Rust processing (if enabled)
    public func fetchAllProcessedEvents() async throws -> [ProcessedEvent] {
        var processedAll: [ProcessedEvent] = []
        let snapshot = await MainActor.run { sources }
        for source in snapshot where source.enabled {
            do {
                let processed = try await fetchAndProcessEvents(from: source)
                processedAll.append(contentsOf: processed)
            } catch {
                logger.log(.error, "URLFetcher", "fetch and process failed for source", metadata: ["name": source.name, "error": error.localizedDescription])
            }
        }
        return processedAll
    }
    
    /// Validate a URL source by attempting to fetch and parse
    public func validateSource(_ source: URLDataSource) async -> ValidationResult {
        do {
            // Validation should not affect scheduling state (lastFetchDate), otherwise
            // a user testing/validating a source can unintentionally suppress the next scheduled run.
            let events = try await readSourceForValidation(from: source)
            let schemaFields = validationSchemaFields(events: events)
            let connectorID = source.isLocalReadableSource ? "ios.local-readable" : "ios.url-session"
            let readerDescription = "\(connectorID) -> \(source.format.rawValue) -> RawEvent"
            let sample = events.first.map { event in
                let presentFields = [
                    event.title.isEmpty ? nil : "title",
                    event.date == nil ? nil : "date",
                    event.notes?.isEmpty == false ? "notes" : nil
                ].compactMap { $0 }
                return "<redacted \(source.format.rawValue) record: \(presentFields.joined(separator: ", "))>"
            } ?? "<validated empty \(source.format.rawValue) source>"
            var contract = URLDataSourceValidationContract(
                sourceID: source.id,
                connectorID: connectorID,
                connectorVersion: 1,
                readerDescription: readerDescription,
                schemaFields: schemaFields,
                redactedSample: sample,
                recordCount: events.count,
                maximumAgeMinutes: 60,
                validationState: events.isEmpty ? .verifiedEmpty : .verified,
                validatedAt: Date(),
                observedAt: Date()
            )
            contract.configureReadRecipe(
                method: source.isLocalReadableSource ? .localFileRead : .httpGet,
                operationID: source.isLocalReadableSource
                    ? "ios.local-readable.read"
                    : "ios.url-session.get",
                configurationKeys: source.isLocalReadableSource
                    ? ["bookmark", "maximumBytes"]
                    : ["url", "maximumBytes"]
            )
            return .success(eventCount: events.count, contract: contract)
        } catch let error as URLFetcherError {
            return .failure(message: error.localizedDescription)
        } catch {
            return .failure(message: error.localizedDescription)
        }
    }

    private func validationSchemaFields(events: [RawEvent]) -> [String] {
        var fields: Set<String> = ["id", "source", "title"]
        for event in events.prefix(20) {
            if event.date != nil { fields.insert("date") }
            if event.notes?.isEmpty == false { fields.insert("notes") }
            fields.formUnion(event.metadata.keys.map { "metadata.\($0)" })
        }
        return fields.sorted()
    }
    
    // MARK: - Parsing
    
    private func parseJSON(_ data: Data) throws -> [RawEvent] {
        let decoder = JSONDecoder()
        // Be lenient: accept RFC3339, full-date (YYYY-MM-DD), and numeric timestamps.
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()

            if let seconds = try? container.decode(Double.self) {
                // Heuristic: treat large numbers as milliseconds.
                if seconds > 4_000_000_000 {
                    return Date(timeIntervalSince1970: seconds / 1000.0)
                }
                return Date(timeIntervalSince1970: seconds)
            }

            let raw = try container.decode(String.self).trimmingCharacters(in: .whitespacesAndNewlines)
            if raw.isEmpty {
                throw DecodingError.dataCorruptedError(in: container, debugDescription: "Empty date string")
            }

            // RFC3339 / ISO8601 date-time
            let iso = ISO8601DateFormatter()
            iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            if let d = iso.date(from: raw) { return d }
            iso.formatOptions = [.withInternetDateTime]
            if let d = iso.date(from: raw) { return d }
            // Full-date
            let fullDate = ISO8601DateFormatter()
            fullDate.formatOptions = [.withFullDate]
            if let d = fullDate.date(from: raw) { return d }

            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Unrecognized date format: \(raw)")
        }
        
        // 1) Primary schema: Nola URLDataResponse (version/events[])
        if let response = try? decoder.decode(URLDataResponse.self, from: data) {
            // Validate version (optional)
            if let version = response.version, version != "1.0" {
                logger.log(.warn, "URLFetcher", NSLocalizedString("autogen.URLFetcherService.cd260bd3", comment: "AUTOGEN"), metadata: ["version": version])
            }

            return response.events.map { jsonEvent in
                RawEvent(
                    id: jsonEvent.id,
                    // Events fetched from URL sources should be typed as `.url` to keep
                    // downstream grouping/dedupe/UX consistent.
                    source: .url,
                    title: jsonEvent.title,
                    date: jsonEvent.date,
                    notes: jsonEvent.description,
                    metadata: [
                        "type": jsonEvent.type,
                        "priority": "\(jsonEvent.priority ?? 0.5)",
                        "tags": jsonEvent.metadata?.tags?.joined(separator: ",") ?? "",
                        "location": jsonEvent.metadata?.location ?? ""
                    ]
                )
            }
        }

        throw URLFetcherError.parseError(message: NSLocalizedString("autogen.URLFetcherService.b506ae69", comment: "AUTOGEN"))
    }
    
    private func parseICalendar(_ data: Data) throws -> [RawEvent] {
        // Basic iCalendar parser (simplified)
        // In production, use a proper iCalendar library
        guard let content = String(data: data, encoding: .utf8) else {
            throw URLFetcherError.parseError(message: NSLocalizedString("autogen.URLFetcherService.a51bab9e", comment: "AUTOGEN"))
        }
        
        var events: [RawEvent] = []
        var currentEvent: [String: String] = [:]
        var inEvent = false
        
        for line in content.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            
            if trimmed.hasPrefix("BEGIN:VEVENT") {
                inEvent = true
                currentEvent = [:]
            } else if trimmed.hasPrefix("END:VEVENT") {
                inEvent = false
                if let event = parseICalendarEvent(currentEvent) {
                    events.append(event)
                }
            } else if inEvent {
                if let colonIndex = trimmed.firstIndex(of: ":") {
                    let key = String(trimmed[..<colonIndex])
                    let value = String(trimmed[trimmed.index(after: colonIndex)...])
                    currentEvent[key] = value
                }
            }
        }
        
        return events
    }
    
    private func parseICalendarEvent(_ data: [String: String]) -> RawEvent? {
        guard let summary = data["SUMMARY"] else { return nil }
        
        let dateFormatter = ISO8601DateFormatter()
        let date = data["DTSTART"].flatMap { dateFormatter.date(from: $0) }
        
        return RawEvent(
            id: data["UID"] ?? UUID().uuidString,
            source: .url,
            title: summary,
            date: date,
            notes: data["DESCRIPTION"],
            metadata: [
                "location": data["LOCATION"] ?? "",
                "type": "icalendar"
            ]
        )
    }
    
    private func parseRSS(_ data: Data) throws -> [RawEvent] {
        // Basic RSS/Atom parser using XMLParser (supports <item> and <entry>)
        class RSSDelegate: NSObject, XMLParserDelegate {
            var events: [RawEvent] = []
            private var currentElement: String = ""
            private var currentTitle: String = ""
            private var currentDesc: String = ""
            private var currentLink: String = ""
            private var currentPubDate: String = ""
            private var currentGUID: String = ""
            private var inItem = false

            func parser(_ parser: XMLParser, didStartElement elementName: String, namespaceURI: String?, qualifiedName qName: String?, attributes attributeDict: [String : String] = [:]) {
                currentElement = elementName.lowercased()
                if currentElement == "item" || currentElement == "entry" {
                    inItem = true
                    currentTitle = ""
                    currentDesc = ""
                    currentLink = ""
                    currentPubDate = ""
                    currentGUID = ""
                }
            }

            func parser(_ parser: XMLParser, foundCharacters string: String) {
                guard inItem else { return }
                switch currentElement {
                case "title": currentTitle += string
                case "description", "summary", "content": currentDesc += string
                case "link": currentLink += string
                case "pubdate", "published", "updated": currentPubDate += string
                case "guid", "id": currentGUID += string
                default: break
                }
            }

            func parser(_ parser: XMLParser, didEndElement elementName: String, namespaceURI: String?, qualifiedName qName: String?) {
                let el = elementName.lowercased()
                if el == "item" || el == "entry" {
                    // finalize event
                    let id = currentGUID.isEmpty ? (currentLink.isEmpty ? UUID().uuidString : currentLink) : currentGUID
                    // parse date
                    var date: Date? = nil
                    if !currentPubDate.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        date = RSSDelegate.parseDateString(currentPubDate)
                    }
                    let link = currentLink.trimmingCharacters(in: .whitespacesAndNewlines)
                    let event = RawEvent(
                        id: id,
                        source: .url,
                        title: currentTitle.trimmingCharacters(in: .whitespacesAndNewlines),
                        date: date,
                        notes: currentDesc.trimmingCharacters(in: .whitespacesAndNewlines),
                        metadata: [
                            "link": link,
                            // Keep a stable key for downstream UI actions (NotificationDetailView expects `url`).
                            "url": link,
                            "type": "rss"
                        ]
                    )
                    events.append(event)
                    inItem = false
                }
                currentElement = ""
            }

            static func parseDateString(_ s: String) -> Date? {
                let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines)
                // Try RFC822 variants
                let rfc1 = DateFormatter()
                rfc1.locale = Locale(identifier: "en_US_POSIX")
                rfc1.dateFormat = "EEE, dd MMM yyyy HH:mm:ss Z"
                if let d = rfc1.date(from: trimmed) { return d }
                let rfc2 = DateFormatter()
                rfc2.locale = Locale(identifier: "en_US_POSIX")
                rfc2.dateFormat = "EEE, dd MMM yyyy HH:mm Z"
                if let d = rfc2.date(from: trimmed) { return d }
                // Try ISO8601
                let iso = ISO8601DateFormatter()
                iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
                if let d = iso.date(from: trimmed) { return d }
                iso.formatOptions = [.withInternetDateTime]
                if let d = iso.date(from: trimmed) { return d }
                return nil
            }
        }

        let parser = XMLParser(data: data)
        let delegate = RSSDelegate()
        parser.delegate = delegate
        if parser.parse() {
            return delegate.events
        } else {
            throw URLFetcherError.parseError(message: "RSS 解析失败: \(parser.parserError?.localizedDescription ?? NSLocalizedString("autogen.URLFetcherService.5f76edc5", comment: "AUTOGEN"))")
        }
    }

    private func parseHTML(_ data: Data, baseURL: URL) throws -> [RawEvent] {
        // Try to extract JSON-LD (<script type="application/ld+json">) first
        guard let html = String(data: data, encoding: .utf8) else {
            throw URLFetcherError.parseError(message: NSLocalizedString("autogen.URLFetcherService.68591f54", comment: "AUTOGEN"))
        }

        // Find JSON-LD scripts
        let pattern = "<script[^>]+type=[\\\"']application/ld\\+json[\\\"'][^>]*>([\\s\\S]*?)</script>"
        if let regex = try? NSRegularExpression(pattern: pattern, options: .caseInsensitive) {
            let matches = regex.matches(in: html, options: [], range: NSRange(html.startIndex..., in: html))
            for m in matches {
                if let range = Range(m.range(at: 1), in: html) {
                    let jsonText = String(html[range])
                    if let jsonData = jsonText.data(using: .utf8) {
                        // Try decode as URLDataResponse or array of events
                        if let decoded = try? JSONDecoder().decode(URLDataResponse.self, from: jsonData) {
                            return decoded.events.map { je in
                                RawEvent(
                                    id: je.id,
                                    source: .url,
                                    title: je.title,
                                    date: je.date,
                                    notes: je.description,
                                    metadata: [
                                        "source": baseURL.absoluteString,
                                        "url": baseURL.absoluteString,
                                        "type": "html_jsonld"
                                    ]
                                )
                            }
                        }
                        // Try decode array of generic objects
                        if let arr = try? JSONSerialization.jsonObject(with: jsonData, options: []) as? [[String: Any]] {
                            var out: [RawEvent] = []
                            for obj in arr {
                                let id = obj["id"] as? String ?? UUID().uuidString
                                let title = obj["title"] as? String ?? (obj["name"] as? String ?? "")
                                let desc = obj["description"] as? String
                                var date: Date? = nil
                                if let dateStr = obj["datePublished"] as? String {
                                    date = ISO8601DateFormatter().date(from: dateStr)
                                }
                                out.append(
                                    RawEvent(
                                        id: id,
                                        source: .url,
                                        title: title,
                                        date: date,
                                        notes: desc,
                                        metadata: [
                                            "source": baseURL.absoluteString,
                                            "url": baseURL.absoluteString,
                                            "type": "html"
                                        ]
                                    )
                                )
                            }
                            if !out.isEmpty { return out }
                        }
                    }
                }
            }
        }

        // Fallback: extract page title and meta description as a single RawEvent
        var pageTitle: String = ""
        if let tRange = html.range(of: "<title[^>]*>([\\s\\S]*?)</title>", options: .regularExpression) {
            pageTitle = String(html[tRange])
            // Strip tags
            pageTitle = pageTitle.replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
        }

        var pageDesc: String? = nil
        if let mRange = html.range(of: "<meta[^>]*name=[\\\"']description[\\\"'][^>]*content=[\\\"']([^\\\"]*)[\\\"'][^>]*>", options: .regularExpression) {
            let meta = String(html[mRange])
            if let cRange = meta.range(of: "content=[\\\"']([^\\\"]*)[\\\"']", options: .regularExpression) {
                let content = String(meta[cRange])
                // extract after = and strip quotes
                if let eq = content.firstIndex(of: "=") {
                    var val = String(content[content.index(after: eq)...])
                    val = val.trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
                    pageDesc = val
                }
            }
        }

        // Extract a visible-text snippet (best-effort) so LLM can perform entity extraction from HTML pages.
        // Keep it short to avoid excessive token usage.
        let visibleTextSnippet: String? = {
            var t = html
            // Drop scripts/styles
            t = t.replacingOccurrences(of: "<script[\\s\\S]*?</script>", with: " ", options: [.regularExpression, .caseInsensitive])
            t = t.replacingOccurrences(of: "<style[\\s\\S]*?</style>", with: " ", options: [.regularExpression, .caseInsensitive])
            // Drop tags
            t = t.replacingOccurrences(of: "<[^>]+>", with: " ", options: .regularExpression)
            // Basic entity cleanup (minimal)
            t = t.replacingOccurrences(of: "&nbsp;", with: " ")
            t = t.replacingOccurrences(of: "&amp;", with: "&")
            t = t.replacingOccurrences(of: "&lt;", with: "<")
            t = t.replacingOccurrences(of: "&gt;", with: ">")
            t = t.replacingOccurrences(of: "&quot;", with: "\"")
            // Normalize whitespace
            t = t.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            t = t.trimmingCharacters(in: .whitespacesAndNewlines)
            guard t.count >= 80 else { return nil }
            return String(t.prefix(5000))
        }()

        let combinedNotes: String? = {
            let desc = pageDesc?.trimmingCharacters(in: .whitespacesAndNewlines)
            let snippet = visibleTextSnippet
            if let desc, !desc.isEmpty, let snippet, !snippet.isEmpty {
                return String((desc + "\n\n" + snippet).prefix(5500))
            }
            if let snippet, !snippet.isEmpty { return snippet }
            if let desc, !desc.isEmpty { return desc }
            return nil
        }()

        let title = pageTitle.isEmpty ? baseURL.absoluteString : pageTitle
        let event = RawEvent(
            id: baseURL.absoluteString,
            source: .url,
            title: title,
            date: nil,
            notes: combinedNotes,
            metadata: ["url": baseURL.absoluteString]
        )
        return [event]
    }
    
    // MARK: - Persistence

    @MainActor
    private func enqueueSourceDeletion(
        id: UUID, expectedRevision: Int64?,
        completion: @escaping (Result<Void, Error>) -> Void
    ) throws {
        guard let source = sources.first(where: { $0.id == id }) else { throw CocoaError(.fileNoSuchFile) }
        guard canAccess(source, permission: .delete) else { throw CocoaError(.fileWriteNoPermission) }
        persistenceQueue.async { [self] in
            do {
                guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
                guard let revision = expectedRevision ?? sourceRevisions[id.uuidString] else {
                    throw URLFetcherError.sourceRevisionUnavailable
                }
                guard try applicationRuntime.deleteDataSource(
                    id: id.uuidString, expectedRevision: revision,
                    deletedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
                ) else { throw IOSApplicationRuntimeError.invalidResponse }
                sourceRevisions.removeValue(forKey: id.uuidString)
                completion(.success(()))
            } catch { completion(.failure(error)) }
        }
    }

    private func persistSource(_ source: URLDataSource) {
        persistenceQueue.async { [self, logger] in
            do {
                let saved = try commitSource(source, expectedRevision: sourceRevisions[source.id.uuidString])
                guard let projection = saved.iosURLSource else { throw IOSApplicationRuntimeError.invalidResponse }
                Task { @MainActor [weak self] in
                    guard let self,
                          let index = self.sources.firstIndex(where: { $0.id == source.id }),
                          self.sources[index].updatedAt == source.updatedAt,
                          self.sources[index].accessRevision == source.accessRevision else { return }
                    self.sources[index] = projection
                    self.notifySourcesUpdated()
                }
            } catch {
                logger.log(.error, "URLFetcher", "Data source persistence failed", metadata: [
                    "id": source.id.uuidString,
                    "error": error.localizedDescription
                ])
            }
        }
    }

    // Called only on persistenceQueue; version advances follow successful commits.
    private func commitSource(_ source: URLDataSource, expectedRevision: Int64?) throws -> IOSCanonicalDataSource {
        guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
        guard let submission = source.canonicalSubmission else { throw IOSApplicationRuntimeError.invalidResponse }
        let saved = try applicationRuntime.saveDataSource(
            submission,
            expectedRevision: expectedRevision
        )
        sourceRevisions[saved.id] = saved.revision
        return saved
    }
    
    internal func waitForPersistence() {
        persistenceQueue.sync { }
    }

    private func loadSources() {
        let records = (try? applicationRuntime?.listDataSources()) ?? []
        sourceRevisions = Dictionary(uniqueKeysWithValues: records.map { ($0.id, $0.revision) })
        sources = records.compactMap(\.iosURLSource)
    }

    private func loadUsageRecords() {
        usageRecords = ((try? applicationRuntime?.listDataSourceUsage(limit: 200)) ?? [])
            .compactMap(\.iosUsageRecord)
    }

    private func notifySourcesUpdated() {
        NotificationCenter.default.post(name: .nolaCustomSourcesUpdated, object: nil)
    }
}

// MARK: - Supporting Types

/// JSON data response schema
private struct URLDataResponse: Codable {
    let events: [JSONEvent]
    let version: String?
}

private struct JSONEvent: Codable {
    let id: String
    let type: String
    let title: String
    let date: Date
    let description: String?
    let priority: Double?
    let metadata: JSONEventMetadata?
}

private struct JSONEventMetadata: Codable {
    let tags: [String]?
    let location: String?
}

/// Validation result
public enum ValidationResult {
    case success(eventCount: Int, contract: URLDataSourceValidationContract)
    case failure(message: String)
    
    public var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}

// MARK: - Errors

public enum URLFetcherError: LocalizedError {
    case sourceRevisionUnavailable
    case tooManySources(max: Int)
    case invalidURL
    case invalidResponse
    case httpError(statusCode: Int)
    case parseError(message: String)
    
    public var errorDescription: String? {
        switch self {
        case .sourceRevisionUnavailable:
            return NSLocalizedString("editor.reopen_required", tableName: "CanonicalDataSourceEditor", value: "Reopen this source before saving so its latest version can be checked.", comment: "Missing editor revision")
        case .tooManySources(let max):
            return "URL 数量超过限制（最多 \(max) 个）"
        case .invalidURL:
            return NSLocalizedString("autogen.URLFetcherService.87ad1b2c", comment: "AUTOGEN")
        case .invalidResponse:
            return NSLocalizedString("autogen.URLFetcherService.0d43d6d8", comment: "AUTOGEN")
        case .httpError(let code):
            return "HTTP 错误：\(code)"
        case .parseError(let message):
            return message
        }
    }
}

/// Bounds the decoded HTTP body before parsers or previews can retain it.
/// Content-Length is only an early rejection hint; streamed bytes remain authoritative.
nonisolated enum IOSBoundedDataSourceBody {
    static let maximumBytes = 1_048_576

    static func load(_ request: URLRequest, using session: URLSession) async throws -> (Data, URLResponse) {
        let (bytes, response) = try await session.bytes(for: request)
        defer { bytes.task.cancel() }
        let data = try await collect(bytes, advertisedLength: response.expectedContentLength)
        return (data, response)
    }

    static func collect<Body: AsyncSequence>(
        _ bytes: Body,
        advertisedLength: Int64 = -1,
        limit: Int = maximumBytes
    ) async throws -> Data where Body.Element == UInt8, Body.AsyncIterator: SendableMetatype {
        guard limit >= 0, advertisedLength <= Int64(limit) else {
            throw URLError(.dataLengthExceedsMaximum)
        }
        try Task.checkCancellation()
        var body = Data()
        body.reserveCapacity(min(limit, 65_536))
        var iterator = bytes.makeAsyncIterator()
        while let byte = try await IOSHTTPByteIterator.next(&iterator) {
            guard body.count < limit else {
                throw URLError(.dataLengthExceedsMaximum)
            }
            if body.count % 65_536 == 0 { try Task.checkCancellation() }
            body.append(byte)
        }
        try Task.checkCancellation()
        return body
    }
}

nonisolated struct IOSCanonicalReadableSource {
    let source: URLDataSource
    let authorization: IOSCanonicalDataSourceAuthorization
    let selection: IOSCanonicalDataSourceContextSelection
    let events: [RawEvent]
}

extension URLFetcherService {
    func canonicalContextSources(conversationID: String, allowLiveFetch: Bool) async throws -> [IOSCanonicalReadableSource] {
        let initial = try await canonicalContextSnapshot(conversationID: conversationID)
        if allowLiveFetch {
            for selection in initial.selections where selection.enabled && selection.shouldRefresh {
                guard let record = initial.records.first(where: { $0.id == selection.sourceID && $0.revision == selection.sourceRevision }),
                      let source = record.iosURLSource else { continue }
                do { _ = try await refreshCanonicalSource(from: source) }
                catch {
                    if Task.isCancelled { throw CancellationError() }
                    logger.log(.warn, "URLFetcher", "Context source refresh failed", metadata: ["sourceId": selection.sourceID, "error": error.localizedDescription])
                }
            }
        }
        let current = try await canonicalContextSnapshot(conversationID: conversationID)
        var candidates: [IOSCanonicalReadableSource] = []
        for selection in current.selections where selection.enabled && selection.usable {
            try Task.checkCancellation()
            guard let reference = selection.contentReference, selection.schemaVersion == 1,
                  let record = current.records.first(where: { $0.id == selection.sourceID && $0.revision == selection.sourceRevision }),
                  let source = record.iosURLSource else { continue }
            do {
                let events = try await readCanonicalContextContent(reference)
                candidates.append(.init(source: source, authorization: record.authorization, selection: selection, events: events))
            } catch {
                logger.log(.warn, "URLFetcher", "Context source content unavailable", metadata: ["sourceId": selection.sourceID, "error": error.localizedDescription])
            }
        }
        // Source configuration, permissions or conversation switches may change
        // while Host I/O is suspended. A prior selection never authorizes injection.
        let final = try await canonicalContextSnapshot(conversationID: conversationID)
        return candidates.filter { candidate in
            final.selections.contains {
                $0 == candidate.selection && $0.enabled && $0.usable
            }
        }
    }

    private func canonicalContextSnapshot(conversationID: String) async throws -> (records: [IOSCanonicalDataSource], selections: [IOSCanonicalDataSourceContextSelection]) {
        try await withCheckedThrowingContinuation { continuation in
            persistenceQueue.async { [self] in
                do {
                    guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
                    let records = try applicationRuntime.listDataSources()
                    let selections = try applicationRuntime.resolveDataSources(conversationID: conversationID, nowMs: Int64(Date().timeIntervalSince1970 * 1_000))
                    continuation.resume(returning: (records, selections))
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    private func readCanonicalContextContent(_ reference: IOSCanonicalDataSourceContentReference) async throws -> [RawEvent] {
        try await withCheckedThrowingContinuation { continuation in
            persistenceQueue.async { [self] in
                do {
                    guard let applicationRuntime,
                          reference.mimeType == "application/json",
                          reference.byteCount <= 8 * 1_048_576,
                          UUID(uuidString: reference.contentVersion) != nil,
                          let url = URL(string: reference.uri), url.isFileURL,
                          url.lastPathComponent == reference.contentVersion + ".json",
                          url.resolvingSymlinksInPath().deletingLastPathComponent() == applicationRuntime.dataSourceContentDirectory.resolvingSymlinksInPath() else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                    let handle = try FileHandle(forReadingFrom: url)
                    defer { try? handle.close() }
                    var data = Data()
                    while let chunk = try handle.read(upToCount: 65_536), !chunk.isEmpty {
                        guard data.count + chunk.count <= 8 * 1_048_576 else { throw URLError(.dataLengthExceedsMaximum) }
                        data.append(chunk)
                    }
                    guard UInt64(data.count) == reference.byteCount else { throw IOSApplicationRuntimeError.invalidResponse }
                    let decoder = JSONDecoder()
                    decoder.dateDecodingStrategy = .deferredToDate
                    let events = try decoder.decode([RawEvent].self, from: data)
                    // Model context is a bounded projection, not the full source artifact.
                    let bounded = events.prefix(16).map { event in
                        RawEvent(id: String(event.id.prefix(512)), source: event.source,
                                 title: String(event.title.prefix(256)), date: event.date,
                                 notes: event.notes.map { String($0.prefix(4_096)) }, metadata: [:])
                    }
                    continuation.resume(returning: bounded)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
}
