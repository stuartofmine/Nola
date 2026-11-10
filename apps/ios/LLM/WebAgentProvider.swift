// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements WebAgentProvider logic in the iOS LLM integration layer.
// - Interfaces:
//   - HostAgentToolEvent
//   - HostAgentPermissionRequest
//   - WebAgentStreamEvent
//   - WebAgentEventStreaming
//   - WebAgentProvider
// - APIs:
//   - none
//
import Foundation
import CryptoKit
import Security

/// Shared byte framing for chat and task SSE. Empty lines are protocol data,
/// and UTF-8 scalars may span arbitrary network chunks.
nonisolated struct WebAgentSSELineFramer {
    private var bytes = Data()
    mutating func append(_ byte: UInt8) -> String? {
        guard byte == 0x0A else { bytes.append(byte); return nil }
        defer { bytes.removeAll(keepingCapacity: true) }
        return String(decoding: bytes, as: UTF8.self)
    }
    mutating func finish() -> String? {
        guard !bytes.isEmpty else { return nil }
        defer { bytes.removeAll(keepingCapacity: true) }
        return String(decoding: bytes, as: UTF8.self)
    }
}

private struct LossyDecodedArray<Element: Decodable>: Decodable {
    let elements: [Element]

    init(from decoder: Decoder) throws {
        var container = try decoder.unkeyedContainer()
        var decoded: [Element] = []
        while !container.isAtEnd {
            if let element = try? container.decode(Element.self) {
                decoded.append(element)
            } else {
                _ = try? container.decode(DiscardedDecodable.self)
            }
        }
        self.elements = decoded
    }
}

private struct DiscardedDecodable: Decodable {}

private func decodeLossyArray<Element: Decodable, Key: CodingKey>(
    _ type: Element.Type,
    from container: KeyedDecodingContainer<Key>,
    forKey key: Key
) -> [Element] {
    (try? container.decode(LossyDecodedArray<Element>.self, forKey: key).elements) ?? []
}

private func decodeLossyStringArray<Key: CodingKey>(
    _ container: KeyedDecodingContainer<Key>,
    _ key: Key
) -> [String]? {
    let values = decodeLossyArray(String.self, from: container, forKey: key)
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
    return values.isEmpty ? nil : values
}

private func decodeString<Key: CodingKey>(
    _ container: KeyedDecodingContainer<Key>,
    _ key: Key
) -> String? {
    if let value = try? container.decodeIfPresent(String.self, forKey: key) {
        return value
    }
    if let value = try? container.decodeIfPresent(Int.self, forKey: key) {
        return String(value)
    }
    if let value = try? container.decodeIfPresent(Double.self, forKey: key) {
        return String(value)
    }
    if let value = try? container.decodeIfPresent(Bool.self, forKey: key) {
        return value ? "true" : "false"
    }
    return nil
}

private func decodeFirstNonBlankString<Key: CodingKey>(
    _ container: KeyedDecodingContainer<Key>,
    _ keys: Key...
) -> String? {
    for key in keys {
        guard let value = decodeString(container, key) else { continue }
        if !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return value
        }
    }
    return nil
}

private func decodeBoolish<Key: CodingKey>(
    _ container: KeyedDecodingContainer<Key>,
    _ key: Key
) -> Bool? {
    if let value = try? container.decodeIfPresent(Bool.self, forKey: key) {
        return value
    }
    if let value = try? container.decodeIfPresent(Int.self, forKey: key) {
        if value == 0 { return false }
        if value == 1 { return true }
        return nil
    }
    if let value = try? container.decodeIfPresent(Double.self, forKey: key) {
        if value == 0 { return false }
        if value == 1 { return true }
        return nil
    }
    if let value = try? container.decodeIfPresent(String.self, forKey: key) {
        switch value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "true", "1", "yes", "y", "on":
            return true
        case "false", "0", "no", "n", "off":
            return false
        default:
            return nil
        }
    }
    return nil
}

nonisolated public struct HostAgentToolEvent: Identifiable, Codable, Equatable {
    nonisolated public enum Status: String, Codable {
        case running
        case succeeded
        case failed
        case cancelled
        case permissionRequired
    }

    public let id: String
    public let name: String
    public let argsPreview: String
    public let status: Status
    public let display: String
    public let text: String
    public let screenshotURL: URL?
    public let finalURL: URL?
    public let pageTitle: String?
    public let downloadURL: URL?
    public let downloadFilename: String?
    public let screenshotWarning: String?
    public let sensitive: Bool

    public init(
        id: String,
        name: String,
        argsPreview: String = "",
        status: Status,
        display: String = "",
        text: String = "",
        screenshotURL: URL? = nil,
        finalURL: URL? = nil,
        pageTitle: String? = nil,
        downloadURL: URL? = nil,
        downloadFilename: String? = nil,
        screenshotWarning: String? = nil,
        sensitive: Bool = false
    ) {
        self.id = id
        self.name = name
        self.argsPreview = argsPreview
        self.status = status
        self.display = display
        self.text = text
        self.screenshotURL = screenshotURL
        self.finalURL = finalURL
        self.pageTitle = pageTitle
        self.downloadURL = downloadURL
        self.downloadFilename = downloadFilename
        self.screenshotWarning = screenshotWarning
        self.sensitive = sensitive
    }

    /// Keeps the richer canonical-Part presentation while allowing the
    /// canonical trace stream to settle the lifecycle for the same call.
    /// Some hosts emit the terminal trace before the final Part projection.
    public func reconcilingLifecycle(from traceEvent: HostAgentToolEvent) -> HostAgentToolEvent {
        guard id == traceEvent.id else { return self }
        return HostAgentToolEvent(
            id: id,
            name: name == "tool" ? traceEvent.name : name,
            argsPreview: argsPreview.isEmpty ? traceEvent.argsPreview : argsPreview,
            status: traceEvent.status,
            display: display.isEmpty ? traceEvent.display : display,
            text: traceEvent.text.isEmpty ? text : traceEvent.text,
            screenshotURL: screenshotURL ?? traceEvent.screenshotURL,
            finalURL: finalURL ?? traceEvent.finalURL,
            pageTitle: pageTitle ?? traceEvent.pageTitle,
            downloadURL: downloadURL ?? traceEvent.downloadURL,
            downloadFilename: downloadFilename ?? traceEvent.downloadFilename,
            screenshotWarning: screenshotWarning ?? traceEvent.screenshotWarning,
            sensitive: sensitive || traceEvent.sensitive
        )
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case name
        case argsPreview
        case status
        case display
        case text
        case screenshotURL
        case finalURL
        case pageTitle
        case downloadURL
        case downloadFilename
        case screenshotWarning
        case sensitive
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try container.decode(String.self, forKey: .id)
        self.name = try container.decode(String.self, forKey: .name)
        self.argsPreview = try container.decodeIfPresent(String.self, forKey: .argsPreview) ?? ""
        self.status = try container.decode(Status.self, forKey: .status)
        self.display = try container.decodeIfPresent(String.self, forKey: .display) ?? ""
        self.text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        self.screenshotURL = try container.decodeIfPresent(URL.self, forKey: .screenshotURL)
        self.finalURL = try container.decodeIfPresent(URL.self, forKey: .finalURL)
        self.pageTitle = try container.decodeIfPresent(String.self, forKey: .pageTitle)
        self.downloadURL = try container.decodeIfPresent(URL.self, forKey: .downloadURL)
        self.downloadFilename = try container.decodeIfPresent(String.self, forKey: .downloadFilename)
        self.screenshotWarning = try container.decodeIfPresent(String.self, forKey: .screenshotWarning)
        self.sensitive = try container.decodeIfPresent(Bool.self, forKey: .sensitive) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(name, forKey: .name)
        try container.encode(argsPreview, forKey: .argsPreview)
        try container.encode(status, forKey: .status)
        try container.encode(display, forKey: .display)
        try container.encode(text, forKey: .text)
        try container.encodeIfPresent(screenshotURL, forKey: .screenshotURL)
        try container.encodeIfPresent(finalURL, forKey: .finalURL)
        try container.encodeIfPresent(pageTitle, forKey: .pageTitle)
        try container.encodeIfPresent(downloadURL, forKey: .downloadURL)
        try container.encodeIfPresent(downloadFilename, forKey: .downloadFilename)
        try container.encodeIfPresent(screenshotWarning, forKey: .screenshotWarning)
        if sensitive {
            try container.encode(true, forKey: .sensitive)
        }
    }

    public nonisolated var isBrowserTool: Bool {
        name.hasPrefix("chrome_native_")
    }

    public nonisolated var isComputerTool: Bool {
        name.hasPrefix("computer_")
    }

    public nonisolated var containsSensitiveData: Bool {
        if sensitive { return true }
        return Self.containsSensitiveData(
            in: [
                argsPreview,
                display,
                text,
                screenshotURL?.absoluteString,
                finalURL?.absoluteString,
                downloadURL?.absoluteString,
                pageTitle,
                downloadFilename,
                screenshotWarning,
            ]
                .compactMap { $0 }
                .joined(separator: "\n")
        )
    }

    public nonisolated var redactedArgsPreview: String {
        Self.redactSensitiveContent(argsPreview)
    }

    public nonisolated var redactedDisplay: String {
        Self.redactSensitiveContent(display)
    }

    public nonisolated var redactedText: String {
        Self.redactSensitiveContent(text)
    }

    public nonisolated var redactedScreenshotWarning: String {
        Self.redactSensitiveContent(screenshotWarning ?? "")
    }

    public nonisolated static func containsSensitiveData(in text: String) -> Bool {
        let patterns = [
            #"(?i)client[_\s-]?secret"#,
            #"(?i)api[_\s-]?key"#,
            #"(?i)access[_\s-]?token"#,
            #"(?i)refresh[_\s-]?token"#,
            #"(?i)id[_\s-]?token"#,
            #"(?i)"?(?:token|secret|cookie|session(?:[_\s-]?id)?|auth)"?\s*[:=]"#,
            #"(?i)authorization"#,
            #"(?i)password"#,
            #"(?i)private[_\s-]?key"#,
            #"(?i)credential"#,
            #"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"#,
            #"sk-[A-Za-z0-9_-]{16,}"#,
            #"GOCSPX-[A-Za-z0-9_-]+"#,
            #"gh[pousr]_[A-Za-z0-9_]{20,}"#,
            #"github_pat_[A-Za-z0-9_]{20,}"#,
            #"-----BEGIN [A-Z ]*PRIVATE KEY-----"#,
        ]
        return patterns.contains { pattern in
            text.range(of: pattern, options: .regularExpression) != nil
        }
    }

    public nonisolated static func redactSensitiveContent(_ text: String) -> String {
        var redacted = text
        let replacements: [(String, String)] = [
            (#"(?is)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----"#, "[敏感密钥已隐藏]"),
            (#"(?i)("?(?:client[_\s-]?secret|api[_\s-]?key|access[_\s-]?token|refresh[_\s-]?token|id[_\s-]?token|token|secret|cookie|session(?:[_\s-]?id)?|auth|authorization|password|private[_\s-]?key|credential)"?\s*(?::|=|\bis\b)\s*"?)((?:Bearer\s+)?[^"\s,}]+)("?)"#, "$1[已隐藏]$3"),
            (#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"#, "Bearer [已隐藏]"),
            (#"sk-[A-Za-z0-9_-]{16,}"#, "sk-[已隐藏]"),
            (#"GOCSPX-[A-Za-z0-9_-]+"#, "GOCSPX-[已隐藏]"),
            (#"gh[pousr]_[A-Za-z0-9_]{20,}"#, "gh[已隐藏]"),
            (#"github_pat_[A-Za-z0-9_]{20,}"#, "github_pat_[已隐藏]"),
        ]
        for (pattern, replacement) in replacements {
            redacted = redacted.replacingOccurrences(
                of: pattern,
                with: replacement,
                options: .regularExpression
            )
        }
        return redacted
    }
}

public struct HostAgentPermissionRequest: Identifiable, Codable, Equatable {
    public let allowConversation: Bool
    public let id: String
    public let toolName: String
    public let argsPreview: String
    public let match: String
    public let reason: String
    public let message: String
    public let nonce: String?
    public let hostId: String?
    public let clientId: String?
    public let sessionId: String?
    public let canonicalRunID: String?
    public let expiresAt: Int64?
    public let protocolVersion: Int?
    public let originKind: String?
    public let deviceBridge: Bool?
    public let permissionInstructions: [AgentPermissionInstruction]?

    public init(
        id: String,
        allowConversation: Bool,
        toolName: String,
        argsPreview: String = "",
        match: String = "*",
        reason: String = "",
        message: String = "",
        nonce: String? = nil,
        hostId: String? = nil,
        clientId: String? = nil,
        sessionId: String? = nil,
        canonicalRunID: String? = nil,
        expiresAt: Int64? = nil,
        protocolVersion: Int? = nil,
        deviceBridge: Bool = false,
        originKind: String? = nil,
        permissionInstructions: [AgentPermissionInstruction] = []
    ) {
        self.id = id
        self.allowConversation = allowConversation
        self.toolName = toolName
        self.argsPreview = argsPreview
        self.match = match
        self.reason = reason
        self.message = message
        self.nonce = nonce
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
        self.canonicalRunID = canonicalRunID
        self.expiresAt = expiresAt
        self.protocolVersion = protocolVersion
        self.deviceBridge = deviceBridge
        self.originKind = originKind
        self.permissionInstructions = permissionInstructions
    }
}

public enum HostAgentPermissionDecision: Equatable, Sendable {
    case deny
    case allowOnce
    case allowConversation

    case provideInstruction(String)

    public var isAllowed: Bool {
        switch self { case .allowOnce, .allowConversation: true; case .deny, .provideInstruction: false }
    }
    public var resolutionPayload: [String: Any] {
        if case .provideInstruction(let instruction) = self {
            return ["type": "intervention", "intervention": ["type": "provide_instruction", "instruction": instruction]]
        }
        return ["type": "authorization", "decision": wireValue]
    }
    nonisolated public static func validatedInstruction(_ text: String, requestID: String = "", existing: [AgentPermissionInstruction] = []) -> HostAgentPermissionDecision? {
        let instruction = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !instruction.isEmpty, instruction.utf8.count <= 4096, existing.count < 16 else { return nil }
        var bytes = instruction.utf8.count + requestID.utf8.count
        for record in existing {
            guard record.instruction.utf8.count <= 4096 else { return nil }
            bytes += record.instruction.utf8.count + record.permissionRequestId.utf8.count
            guard bytes <= 16_384 else { return nil }
        }
        guard bytes <= 16_384 else { return nil }
        return .provideInstruction(instruction)
    }
    public var wireValue: String {
        switch self {
        case .deny: "deny"
        case .allowOnce: "allow_once"
        case .allowConversation: "allow_conversation"
        case .provideInstruction: "provide_instruction"
        }
    }
}

public struct HostDeviceToolRequest: Identifiable, Sendable, Equatable {
    public let id: String
    public let nonce: String
    public let hostId: String
    public let clientId: String
    public let sessionId: String
    public let toolName: String
    /// Informational only. Core resolves the trusted capability from toolName.
    public let capability: AgentCoreCapability?
    public let policyDigest: String?
    public let manifestDigest: String?
    public let manifestValidUntilMs: Int64?
    public let arguments: AgentCoreJSONValue
    public let argsPreview: String
    public let expiresAt: Int64
    public let protocolVersion: Int

    public init(
        id: String,
        nonce: String,
        hostId: String,
        clientId: String,
        sessionId: String,
        toolName: String,
        capability: AgentCoreCapability? = nil,
        policyDigest: String? = nil,
        manifestDigest: String? = nil,
        manifestValidUntilMs: Int64? = nil,
        arguments: AgentCoreJSONValue,
        argsPreview: String,
        expiresAt: Int64,
        protocolVersion: Int
    ) {
        self.id = id
        self.nonce = nonce
        self.hostId = hostId
        self.clientId = clientId
        self.sessionId = sessionId
        self.toolName = toolName
        self.capability = capability
        self.policyDigest = policyDigest
        self.manifestDigest = manifestDigest
        self.manifestValidUntilMs = manifestValidUntilMs
        self.arguments = arguments
        self.argsPreview = argsPreview
        self.expiresAt = expiresAt
        self.protocolVersion = protocolVersion
    }
}

public enum WebAgentStreamEvent {
    case coreTurnTerminal(AgentCoreTurnTerminalEnvelope, trace: AgentTraceEvent?)
    case coreTaskUpdated(AgentCoreTaskEventEnvelope)
    case coreSkillRuntimeUpdated(AgentCoreSkillRuntimeUpdateEnvelope)
    case coreCapabilitiesActivated(AgentCoreCapabilityActivationEnvelope)
    case corePermissionProjectionUpdated(AgentCorePermissionProjection)
    case coreMemoryRuntimeProjectionUpdated(AgentCoreJSONValue)
    case contextUsageUpdated(HostAgentContextUsageProjection)
    case assistantJournalCommitted(AgentCoreJSONValue)
    case assistantRunTerminalSealed(AgentCoreJSONValue)
    case submissionReceipt(AgentCoreJSONValue)
    case trace(AgentTraceEvent)
    case permissionRequest(HostAgentPermissionRequest, trace: AgentTraceEvent)
    case delegatedPermissionRequest(HostAgentPermissionRequest)
    case deviceToolRequest(HostDeviceToolRequest, trace: AgentTraceEvent)
    case externalSessionStarted(backendID: String, sessionID: String)
    case error(String)
}

public struct HostAgentContextUsageProjection: Codable, Sendable, Equatable {
    public let protocolVersion: Int
    public let sessionID: String
    public let sourceHostID: String
    public let usedTokens: Int
    public let contextWindowTokens: Int
    public let promptTokenLimit: Int
    public let reservedOutputTokens: Int
    public let messageTokens: Int?
    public let toolSchemaTokens: Int?
    public let componentTokens: Int?
    public let toolResultTokens: Int?
    public let conversationTokens: Int?
    public let messageCount: Int?
    public let toolCount: Int?
    public let breakdownStatus: String?
    public let usagePercent: Double

    private enum CodingKeys: String, CodingKey {
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case sourceHostID = "source_host_id"
        case usedTokens = "used_tokens"
        case contextWindowTokens = "context_window_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case reservedOutputTokens = "reserved_output_tokens"
        case messageTokens = "message_tokens"
        case toolSchemaTokens = "tool_schema_tokens"
        case componentTokens = "component_tokens"
        case toolResultTokens = "tool_result_tokens"
        case conversationTokens = "conversation_tokens"
        case messageCount = "message_count"
        case toolCount = "tool_count"
        case breakdownStatus = "breakdown_status"
        case usagePercent = "usage_percent"
    }
}

public extension HostAgentToolEvent {
    nonisolated static let conversationMetaKey = "hostToolEventsJSON"

    nonisolated static func encodeForConversationMeta(_ events: [HostAgentToolEvent]) -> String? {
        let safeEvents = events.map { $0.redactedForConversationMeta() }
        guard !events.isEmpty,
              let data = try? JSONEncoder().encode(safeEvents) else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    nonisolated static func decodeFromConversationMeta(_ meta: [String: String]?) -> [HostAgentToolEvent] {
        guard let raw = meta?[conversationMetaKey],
              let data = raw.data(using: .utf8),
              let events = try? JSONDecoder().decode([HostAgentToolEvent].self, from: data) else {
            return []
        }
        return events
    }

    nonisolated private func redactedForConversationMeta() -> HostAgentToolEvent {
        let sensitive = containsSensitiveData
        return HostAgentToolEvent(
            id: id,
            name: name,
            argsPreview: sensitive ? "" : Self.redactSensitiveContent(argsPreview),
            status: status,
            display: Self.redactSensitiveContent(display),
            text: sensitive ? "" : Self.redactSensitiveContent(text),
            screenshotURL: sensitive ? nil : screenshotURL,
            finalURL: sensitive ? nil : finalURL,
            pageTitle: sensitive ? nil : pageTitle.map(Self.redactSensitiveContent),
            downloadURL: sensitive ? nil : downloadURL,
            downloadFilename: sensitive ? nil : downloadFilename.map(Self.redactSensitiveContent),
            screenshotWarning: screenshotWarning.map(Self.redactSensitiveContent),
            sensitive: sensitive
        )
    }
}

public protocol WebAgentEventStreaming {
    func generateAgentEventStream(input: ModelInput) -> AsyncThrowingStream<WebAgentStreamEvent, Error>
}

nonisolated struct WebAgentTransportResponse: Sendable {
    let bytes: AsyncThrowingStream<UInt8, Error>
    let response: URLResponse
}

public struct WebAgentTaskProjectionEvent: Sendable {
    public let conversationId: String
    public let cursor: UInt64
    public let projection: AgentCoreTaskProjection?
}

typealias WebAgentStreamLoader = @Sendable (URLRequest) async throws -> WebAgentTransportResponse

public struct WebAgentProvider: LLMClient, LLMRemoteAuxiliaryInvocationProviding, WebAgentEventStreaming, @unchecked Sendable {
    nonisolated public static let hostPermissionProtocolVersion = 1

    private static let defaultMaxIterations = 0

    private static func localized(_ key: String, value: String) -> String {
        NSLocalizedString(key, value: value, comment: "")
    }

    private static func localizedFormat(_ key: String, value: String, _ arguments: CVarArg...) -> String {
        String(format: localized(key, value: value), locale: Locale.current, arguments: arguments)
    }

    public struct Config: Codable, Equatable {
        public var baseURL: URL
        public var token: String
        public var model: String
        public var pinnedCertificateSHA256: String?
        public var expectedHostId: String?
        public var clientId: String

        public init(baseURL: URL, token: String, model: String = "", pinnedCertificateSHA256: String? = nil, expectedHostId: String? = nil, clientId: String? = nil) {
            self.baseURL = baseURL
            self.token = token
            self.model = model
            self.pinnedCertificateSHA256 = Self.normalizedFingerprint(pinnedCertificateSHA256)
            let normalizedHostId = expectedHostId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            self.expectedHostId = normalizedHostId.isEmpty ? nil : normalizedHostId
            self.clientId = clientId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                ? clientId!.trimmingCharacters(in: .whitespacesAndNewlines)
                : Self.persistentClientId()
            NolaTailcatTransport.register(baseURL: baseURL, token: token, hostID: self.expectedHostId, certificatePin: self.pinnedCertificateSHA256)
        }

        public static func normalizedFingerprint(_ value: String?) -> String? {
            let normalized = (value ?? "")
                .filter { $0.isHexDigit }
                .uppercased()
            return normalized.count == 64 ? normalized : nil
        }

        private static func persistentClientId() -> String {
            let key = "nola.mobile.webHostClientId"
            if let existing = UserDefaults.standard.string(forKey: key), !existing.isEmpty {
                return existing
            }
            let generated = UUID().uuidString.lowercased()
            UserDefaults.standard.set(generated, forKey: key)
            return generated
        }

        public static var currentClientId: String { persistentClientId() }
    }

    public struct LoginResponse: Decodable {
        public let token: String
        public let token_type: String?
        public let expires_in: Int?
        public let local_user_id: String?
        public let display_name: String?
        public let email: String?
    }

    public struct HostUploadedSkill: Decodable, Equatable {
        public let name: String
        public let content: String
        public let description: String?
        public let enabled: Bool?

        private enum CodingKeys: String, CodingKey {
            case name
            case content
            case text
            case description
            case enabled
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let name = decodeString(container, .name)?.trimmingCharacters(in: .whitespacesAndNewlines)
            self.name = name?.isEmpty == false ? name! : "Host Skill"
            self.content = decodeFirstNonBlankString(container, .content, .text) ?? ""
            self.description = decodeString(container, .description)
            self.enabled = decodeBoolish(container, .enabled)
        }
    }

    public struct HostUploadedDocument: Decodable, Equatable {
        public let name: String
        public let content: String
        public let description: String?
        public let kind: String?
        public let enabled: Bool?

        private enum CodingKeys: String, CodingKey {
            case name
            case content
            case text
            case description
            case kind
            case enabled
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let name = decodeString(container, .name)?.trimmingCharacters(in: .whitespacesAndNewlines)
            self.name = name?.isEmpty == false ? name! : "Host Document"
            self.content = decodeFirstNonBlankString(container, .content, .text) ?? ""
            self.description = decodeString(container, .description)
            self.kind = decodeString(container, .kind)
            self.enabled = decodeBoolish(container, .enabled)
        }
    }

    public struct HostMCPServer: Decodable, Equatable {
        private struct CatalogEntry: Decodable, Equatable {}

        public let name: String
        public let serverID: String
        public let endpoint: String
        public let description: String?
        public let enabled: Bool?
        public let availability: String?
        public let protocolVersion: String?
        public let tools: [String]
        public let resources: [String]
        public let resourceTemplates: [String]
        public let prompts: [String]

        private enum CodingKeys: String, CodingKey {
            case name
            case displayName = "display_name"
            case serverID = "server_id"
            case endpoint
            case url
            case description
            case enabled
            case availability
            case protocolVersion = "protocol_version"
            case tools
            case resources
            case resourceTemplates = "resource_templates"
            case prompts
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let name = decodeFirstNonBlankString(container, .displayName, .name, .serverID)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            self.name = name?.isEmpty == false ? name! : "MCP Server"
            self.serverID = decodeString(container, .serverID) ?? self.name
            self.endpoint = decodeFirstNonBlankString(container, .endpoint, .url) ?? ""
            self.description = decodeString(container, .description)
            self.enabled = decodeBoolish(container, .enabled)
            self.availability = decodeString(container, .availability)
            self.protocolVersion = decodeString(container, .protocolVersion)
            self.tools = (try? container.decode([String: CatalogEntry].self, forKey: .tools))
                .map { $0.keys.sorted() } ?? []
            self.resources = (try? container.decode([String: CatalogEntry].self, forKey: .resources))
                .map { $0.keys.sorted() } ?? []
            self.resourceTemplates = (try? container.decode([String: CatalogEntry].self, forKey: .resourceTemplates))
                .map { $0.keys.sorted() } ?? []
            self.prompts = (try? container.decode([String: CatalogEntry].self, forKey: .prompts))
                .map { $0.keys.sorted() } ?? []
        }
    }

    public struct HostComputerUseStatus: Decodable, Equatable {
        public struct Host: Decodable, Equatable {
            public let host_id: String
            public let capabilities: [String]?
            public let last_seen_age_sec: Double?
        }

        public let feature_enabled: Bool?
        public let token_configured: Bool?
        public let connected: Bool?
        public let hosts: [Host]?
        public let tools: [String]?

        private enum CodingKeys: String, CodingKey {
            case feature_enabled
            case token_configured
            case connected
            case hosts
            case tools
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            self.feature_enabled = decodeBoolish(container, .feature_enabled)
            self.token_configured = decodeBoolish(container, .token_configured)
            self.connected = decodeBoolish(container, .connected)
            self.hosts = decodeLossyArray(Host.self, from: container, forKey: .hosts)
            self.tools = decodeLossyStringArray(container, .tools)
        }
    }

    public struct HostNativeBrowserStatus: Decodable, Equatable {
        public struct Host: Decodable, Equatable {
            public let host_id: String
            public let capabilities: [String]?
            public let last_seen_age_sec: Double?
        }

        public let token_configured: Bool?
        public let connected: Bool?
        public let structured_search: Bool?
        public let hosts: [Host]?
        public let tools: [String]?
        public let pending_commands: Int?
        public let in_flight_commands: Int?

        private enum CodingKeys: String, CodingKey {
            case token_configured
            case connected
            case structured_search
            case hosts
            case tools
            case pending_commands
            case in_flight_commands
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            self.token_configured = decodeBoolish(container, .token_configured)
            self.connected = decodeBoolish(container, .connected)
            self.structured_search = decodeBoolish(container, .structured_search)
            self.hosts = decodeLossyArray(Host.self, from: container, forKey: .hosts)
            self.tools = decodeLossyStringArray(container, .tools)
            self.pending_commands = try? container.decodeIfPresent(Int.self, forKey: .pending_commands)
            self.in_flight_commands = try? container.decodeIfPresent(Int.self, forKey: .in_flight_commands)
        }
    }

    public struct HostAgentSettings: Decodable, Equatable {
        public struct Messaging: Decodable, Equatable {
            public struct Worker: Decodable, Equatable {
                public let enabled: Bool

                public init(enabled: Bool) {
                    self.enabled = enabled
                }

                private enum CodingKeys: String, CodingKey { case enabled }

                public init(from decoder: Decoder) throws {
                    let container = try decoder.container(keyedBy: CodingKeys.self)
                    self.enabled = decodeBoolish(container, .enabled) ?? false
                }
            }

            public struct Channel: Decodable, Equatable, Identifiable {
                public var id: String { connection_id }
                public let connection_id: String
                public let kind: String
                public let display_name: String?
                public let account_id: String?
                public let enabled: Bool
                public let state: String
                public let last_received_at: String?

                private enum CodingKeys: String, CodingKey {
                    case connection_id, kind, display_name, account_id, enabled, state, last_received_at
                }

                public init(from decoder: Decoder) throws {
                    let container = try decoder.container(keyedBy: CodingKeys.self)
                    self.connection_id = try container.decode(String.self, forKey: .connection_id)
                    self.kind = try container.decode(String.self, forKey: .kind)
                    self.display_name = try? container.decodeIfPresent(String.self, forKey: .display_name)
                    self.account_id = try? container.decodeIfPresent(String.self, forKey: .account_id)
                    self.enabled = decodeBoolish(container, .enabled) ?? false
                    self.state = (try? container.decodeIfPresent(String.self, forKey: .state)) ?? "awaiting_verification"
                    self.last_received_at = try? container.decodeIfPresent(String.self, forKey: .last_received_at)
                }
            }

            public let worker: Worker
            public let channels: [Channel]
            public let error: String?
            public let management_path: String?

            private enum CodingKeys: String, CodingKey {
                case worker, channels, error, management_path
            }

            public init(from decoder: Decoder) throws {
                let container = try decoder.container(keyedBy: CodingKeys.self)
                self.worker = (try? container.decodeIfPresent(Worker.self, forKey: .worker))
                    ?? Worker(enabled: false)
                self.channels = decodeLossyArray(Channel.self, from: container, forKey: .channels)
                self.error = try? container.decodeIfPresent(String.self, forKey: .error)
                self.management_path = try? container.decodeIfPresent(String.self, forKey: .management_path)
            }
        }

        public struct ExecutionBackend: Decodable, Equatable, Identifiable {
            public enum AvailabilityState: Equatable {
                case available
                case notInstalled
                case signInRequired
                case incompatible
                case unreachable
                case unavailable
            }

            public struct Capabilities: Decodable, Equatable {
                public let streams_text: Bool
                public let streams_reasoning: Bool
                public let streams_tools: Bool
                public let supports_resume: Bool
                public let supports_cancel: Bool
                public let supports_permission_bridge: Bool
            }

            public let id: String
            public let display_name: String
            public let availability: String
            public let executable_path: String?
            public let version: String?
            public let capabilities: Capabilities

            public var availabilityState: AvailabilityState {
                switch availability {
                case "available": return .available
                case "not_installed": return .notInstalled
                case "unauthenticated": return .signInRequired
                case "incompatible": return .incompatible
                case "unreachable": return .unreachable
                default: return .unavailable
                }
            }
        }

        public struct HostIdentity: Decodable, Equatable {
            public let id: String
            public let permission_protocol_version: Int

            private enum CodingKeys: String, CodingKey { case id, permission_protocol_version }

            public init(from decoder: Decoder) throws {
                let values = try decoder.container(keyedBy: CodingKeys.self)
                id = try values.decode(String.self, forKey: .id)
                permission_protocol_version = try values.decode(Int.self, forKey: .permission_protocol_version)
                guard permission_protocol_version == WebAgentProvider.hostPermissionProtocolVersion else {
                    throw DecodingError.dataCorruptedError(forKey: .permission_protocol_version, in: values, debugDescription: "Unsupported host permission protocol version")
                }
            }
        }

        public let uploaded_skills: [HostUploadedSkill]
        public let uploaded_documents: [HostUploadedDocument]
        public let mcp_servers: [HostMCPServer]
        public let enable_local_dev_tools: Bool?
        public let computer_use: HostComputerUseStatus?
        public let native_browser: HostNativeBrowserStatus?
        public let host_identity: HostIdentity?
        public let execution_backends: [ExecutionBackend]
        public let execution_backend: String?
        public let messaging: Messaging?

        private enum CodingKeys: String, CodingKey {
            case uploaded_skills
            case uploaded_documents
            case mcp_servers
            case enable_local_dev_tools
            case computer_use
            case native_browser
            case host_identity
            case execution_backends
            case execution_backend
            case messaging
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            self.uploaded_skills = decodeLossyArray(
                HostUploadedSkill.self,
                from: container,
                forKey: .uploaded_skills
            )
            self.uploaded_documents = decodeLossyArray(
                HostUploadedDocument.self,
                from: container,
                forKey: .uploaded_documents
            )
            self.mcp_servers = decodeLossyArray(
                HostMCPServer.self,
                from: container,
                forKey: .mcp_servers
            )
            self.enable_local_dev_tools = decodeBoolish(container, .enable_local_dev_tools)
            self.computer_use = try? container.decodeIfPresent(HostComputerUseStatus.self, forKey: .computer_use)
            self.native_browser = try? container.decodeIfPresent(HostNativeBrowserStatus.self, forKey: .native_browser)
            self.host_identity = try container.decodeIfPresent(HostIdentity.self, forKey: .host_identity)
            self.execution_backends = decodeLossyArray(
                ExecutionBackend.self,
                from: container,
                forKey: .execution_backends
            )
            self.execution_backend = try? container.decodeIfPresent(String.self, forKey: .execution_backend)
            self.messaging = try? container.decodeIfPresent(Messaging.self, forKey: .messaging)
        }
    }

    private let config: Config
    private let streamLoader: WebAgentStreamLoader?

    public init(config: Config) {
        self.config = config
        self.streamLoader = nil
        NolaTailcatTransport.register(baseURL: config.baseURL, token: config.token, hostID: config.expectedHostId, certificatePin: config.pinnedCertificateSHA256)
    }

    init(config: Config, streamLoader: @escaping WebAgentStreamLoader) {
        self.config = config
        self.streamLoader = streamLoader
    }

    public static func login(baseURL: URL, email: String, password: String, pinnedCertificateSHA256: String? = nil) async throws -> LoginResponse {
        let secureTransport = baseURL.scheme?.lowercased() == "https"
            || baseURL.host.map(isLoopbackHost) == true
        guard secureTransport else {
            throw NolaError.llm(.invalidResponse(reason: "Password login requires HTTPS outside this device. Use a securely transferred access token for an HTTP LAN host."))
        }
        let url = mobileURL(baseURL: baseURL, path: "login")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 20
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "email": email,
            "password": password,
        ])

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        do {
            let (data, response) = try await session.data(for: request)
            try validateHTTP(response: response, data: data)
            return try JSONDecoder().decode(LoginResponse.self, from: data)
        } catch {
            throw friendlyHostError(error, baseURL: baseURL)
        }
    }

    public static func validateToken(baseURL: URL, token: String, pinnedCertificateSHA256: String? = nil) async throws {
        guard MobileHostTransportPolicy.allows(baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        let url = mobileURL(baseURL: baseURL, path: "me")
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.timeoutInterval = 10
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyMobileAuth(to: &request, token: token)

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        do {
            let (data, response) = try await session.data(for: request)
            try validateHTTP(response: response, data: data)
        } catch {
            throw friendlyHostError(error, baseURL: baseURL)
        }
    }

    public static func fetchHostAgentSettings(baseURL: URL, token: String, pinnedCertificateSHA256: String? = nil) async throws -> HostAgentSettings {
        NolaTailcatTransport.register(baseURL: baseURL, token: token, hostID: ModelSelectionManager.pairedWebHostIdentity(for: baseURL), certificatePin: pinnedCertificateSHA256)
        guard MobileHostTransportPolicy.allows(baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        let url = mobileURL(baseURL: baseURL, path: "agent/settings")
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        applyMobileAuth(to: &request, token: token)

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        do {
            let (data, response) = try await session.data(for: request)
            try validateHTTP(response: response, data: data)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw NolaError.llm(.invalidResponse(reason: "Invalid host agent settings response"))
            }
            let decoded: [String: Any]
            if object["type"] as? String == "encrypted" {
                decoded = try decryptMobileJSON(object, token: token)
            } else {
                decoded = object
            }
            let normalized = try JSONSerialization.data(withJSONObject: decoded)
            return try JSONDecoder().decode(HostAgentSettings.self, from: normalized)
        } catch {
            throw friendlyHostError(error, baseURL: baseURL)
        }
    }

    public static func requestWebManagementURL(
        baseURL: URL,
        token: String,
        target: String = "/settings#channels",
        pinnedCertificateSHA256: String? = nil
    ) async throws -> URL {
        guard MobileHostTransportPolicy.allows(baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        let url = mobileURL(baseURL: baseURL, path: "web-handoff")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        request.httpBody = try JSONSerialization.data(withJSONObject: encryptMobileJSON(
            ["target": target],
            token: token
        ))
        applyMobileAuth(to: &request, token: token)

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared { session.finishTasksAndInvalidate() }
        }
        do {
            let (data, response) = try await session.data(for: request)
            try validateHTTP(response: response, data: data)
            guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw NolaError.llm(.invalidResponse(reason: "Invalid Host browser handoff response"))
            }
            let object = root["type"] as? String == "encrypted"
                ? try decryptMobileJSON(root, token: token)
                : root
            guard let path = object["path"] as? String,
                  let managementURL = controlPlaneURL(baseURL: baseURL, path: path) else {
                throw NolaError.llm(.invalidResponse(reason: "Host browser handoff did not include a valid path"))
            }
            return managementURL
        } catch {
            throw friendlyHostError(error, baseURL: baseURL)
        }
    }

    public static func parseDocumentAttachment(
        baseURL: URL,
        token: String,
        filename: String,
        mimeType: String,
        base64Data: String,
        pinnedCertificateSHA256: String? = nil
    ) async throws -> String {
        guard MobileHostTransportPolicy.allows(baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        let url = mobileURL(baseURL: baseURL, path: "attachments/parse")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        // The Host emits an SSE comment heartbeat every 15 seconds. Keep a
        // wider idle allowance for transient LAN stalls without imposing a
        // total duration limit on long-running Agent turns.
        request.timeoutInterval = 120
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        let payload: [String: Any] = [
            "filename": filename,
            "mime_type": mimeType,
            "data_base64": base64Data
        ]
        request.httpBody = try JSONSerialization.data(withJSONObject: encryptMobileJSON(payload, token: token))
        applyMobileAuth(to: &request, token: token)

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        do {
            let (data, response) = try await session.data(for: request)
            try validateHTTP(response: response, data: data)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw NolaError.llm(.invalidResponse(reason: "Invalid host attachment parse response"))
            }
            let decoded: [String: Any]
            if object["type"] as? String == "encrypted" {
                decoded = try decryptMobileJSON(object, token: token)
            } else {
                decoded = object
            }
            guard let text = decoded["text"] as? String,
                  !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw NolaError.llm(.invalidResponse(reason: "Host parser returned empty text"))
            }
            return text
        } catch {
            throw friendlyHostError(error, baseURL: baseURL)
        }
    }

    private var frozenAuxiliaryProfile:(String,AgentCoreJSONValue)?
    func freezeAuxiliaryModel() async throws -> WebAgentProvider {
        guard let host=config.expectedHostId else {throw IOSApplicationRuntimeError.invalidResponse}
        let value=try await auxiliaryRequest(.object(["expected_host_id":.string(host)]),path:"prepare",timeout:30)
        guard let fields=value.objectFields,fields["source_host_id"] == .string(host),
              let profile=fields["profile_id"]?.stringScalar,let revision=fields["revision"] else {throw IOSApplicationRuntimeError.invalidResponse}
        var client=WebAgentProvider(config:config)
        client.frozenAuxiliaryProfile=(profile,revision)
        return client
    }

    public func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        try await LLMSummaryInvocation(ownerID: "local-account", provider: self).generate(input: input, timeout: timeout)
    }

    // Physical auxiliary ownership is remote, never one local wrapper row.
    public var supportsAuxiliaryPhysicalInvocation: Bool { false }
    public var remoteAuxiliarySourceHostID: String? { config.expectedHostId }

    public func inspectRemoteAuxiliary(_ observation: AgentCoreJSONValue,
        observe: @escaping @Sendable (AgentCoreJSONValue) throws -> Void) async throws -> Bool {
        guard case .object(let prior) = observation,
              case .string(let host)? = prior["source_host_id"], host == config.expectedHostId,
              case .string(let operation)? = prior["operation_id"],
              case .string(let attempt)? = prior["attempt_id"] else {
            throw IOSApplicationRuntimeError.rejected("Pending auxiliary source requires its original paired Host")
        }
        let response = try await auxiliaryRequest(.object(["expected_host_id": .string(host),
            "operation_id": .string(operation), "attempt_id": .string(attempt)]), path: "inspect", timeout: 30)
        guard case .object(let envelope) = response,
              envelope["source_host_id"] == .string(host), envelope["operation_id"] == .string(operation),
              envelope["attempt_id"] == .string(attempt), case .object(let receipt)? = envelope["receipt"],
              case .object(let op)? = receipt["operation"], op["id"] == .string(operation),
              case .object(let owner)? = op["owner"], owner["kind"] == .string("source_authority"),
              owner["source_host_id"] == .string(host), let receiptValue = envelope["receipt"] else {
            throw IOSApplicationRuntimeError.rejected("Pending auxiliary source receipt changed identity")
        }
        // Canonical ObserveSource checks frozen config, owner, Attempt and
        // revision monotonicity against the existing durable observation.
        try observe(.object(["source_host_id": .string(host), "operation_id": .string(operation),
            "attempt_id": .string(attempt), "receipt": receiptValue]))
        return op["state"] == .string("succeeded") || op["state"] == .string("failed") || op["state"] == .string("cancelled")
    }

    public func generateRemoteAuxiliary(input: ModelInput, operationID: UUID,
        scenario: LLMAuxiliaryInvocationScenario, timeout: TimeInterval?,
        existingReceipt: AgentCoreJSONValue?,
        observe: @escaping @Sendable (AgentCoreJSONValue) throws -> Void) async throws -> LLMSummaryPhysicalReceipt {
        guard input.attachments.isEmpty, input.providerContinuationJSON == nil else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary port accepts bounded text ModelInput only")
        }
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let output = try await generateRemoteAuxiliaryJSON(inputData: encoder.encode(input),
            operationID: operationID, scenario: scenario, timeout: timeout,
            existingReceipt: existingReceipt, observe: observe)
        guard case .object(let fields) = output else {
            throw IOSApplicationRuntimeError.rejected("Invalid auxiliary output receipt")
        }
        let reason: String? = if case .string(let value)? = fields["finish_reason"] { value } else { nil }
        let tools: Bool? = if case .bool(let value)? = fields["has_tool_calls"] { value } else { nil }
        return LLMSummaryPhysicalReceipt(
            output: try JSONDecoder().decode(ModelOutput.self, from: JSONEncoder().encode(output)),
            finishReason: reason, hasToolCalls: tools)

    }

    func generateRemoteAuxiliaryJSON(inputData: Data, operationID: UUID,
        scenario: LLMAuxiliaryInvocationScenario, timeout: TimeInterval?,
        existingReceipt: AgentCoreJSONValue?,
        observe: @escaping @Sendable (AgentCoreJSONValue) throws -> Void) async throws -> AgentCoreJSONValue {
        guard let host = config.expectedHostId else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary invocation requires a paired Host")
        }
        let operation = operationID.uuidString.lowercased()
        let attempt = "\(operation):physical"
        guard inputData.count <= 256 * 1024, let inputJSON = String(data: inputData, encoding: .utf8) else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary port accepts bounded text ModelInput only")
        }
        let digest = SHA256.hash(data: inputData).map { String(format: "%02x", $0) }.joined()
        let control: AgentCoreJSONValue = .object(["expected_host_id": .string(host),
            "operation_id": .string(operation), "attempt_id": .string(attempt)])

        @Sendable func integer(_ value: AgentCoreJSONValue?) -> Int64? {
            switch value {
            case .signedInteger(let value)?: return value
            case .unsignedInteger(let value)?: return Int64(exactly: value)
            case .number(let value)?: return value.isFinite ? Int64(exactly: value) : nil
            default: return nil
            }
        }
        @Sendable func fields(_ value: AgentCoreJSONValue) throws -> [String: AgentCoreJSONValue] {
            guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.rejected("Invalid auxiliary receipt") }
            return fields
        }
        @Sendable func source(_ raw: AgentCoreJSONValue) throws -> (AgentCoreJSONValue, [String: AgentCoreJSONValue]) {
            let envelope = try fields(raw)
            guard envelope["source_host_id"] == .string(host), envelope["operation_id"] == .string(operation),
                  envelope["attempt_id"] == .string(attempt), let receipt = envelope["receipt"] else {
                throw IOSApplicationRuntimeError.rejected("Auxiliary source receipt identity changed")
            }
            let receiptFields = try fields(receipt)
            let op = try fields(receiptFields["operation"] ?? .null)
            let owner = try fields(op["owner"] ?? .null)
            let configuration = try fields(op["configuration"] ?? .null)
            guard op["id"] == .string(operation), op["scenario"] == .string(scenario.rawValue),
                  owner["kind"] == .string("source_authority"), owner["source_host_id"] == .string(host),
                  configuration["input_digest"] == .string(digest), integer(configuration["max_attempts"]) == 1,
                  receiptFields["dispatch_permitted"] == .bool(false) else {
                throw IOSApplicationRuntimeError.rejected("Auxiliary receipt changed frozen operation input")
            }
            if let attemptValue = receiptFields["attempt"], attemptValue != .null {
                let row = try fields(attemptValue)
                guard row["id"] == .string(attempt), row["operation_id"] == .string(operation) else {
                    throw IOSApplicationRuntimeError.rejected("Auxiliary receipt contains a foreign physical Attempt")
                }
            }
            return (receipt, configuration)
        }
        @Sendable func record(_ raw: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            let (receipt, _) = try source(raw)
            try observe(.object(["source_host_id": .string(host), "operation_id": .string(operation),
                "attempt_id": .string(attempt), "receipt": receipt]))
            return receipt
        }
        @Sendable func result(_ receipt: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            let receipt = try fields(receipt)
            let op = try fields(receipt["operation"] ?? .null)
            if op["state"] == .string("cancelled") { throw CancellationError() }
            guard let rowValue = receipt["attempt"], rowValue != .null else {
                if op["state"] == .string("cancelled") { throw CancellationError() }
                throw LLMRemoteAuxiliaryOutcomeUnknown(sourceHostID: host, operationID: operation, attemptID: attempt)
            }
            let row = try fields(rowValue)
            if row["state"] == .string("succeeded"), let output = row["output"], output != .null {
                return output
            }
            if row["state"] == .string("cancelled") { throw CancellationError() }
            if row["state"] == .string("failed") {
                throw IOSApplicationRuntimeError.rejected("Remote auxiliary physical request failed; its usage receipt is retained")
            }
            throw LLMRemoteAuxiliaryOutcomeUnknown(sourceHostID: host, operationID: operation, attemptID: attempt)
        }

        let profile: String
        let revision: AgentCoreJSONValue
        let duration: AgentCoreJSONValue
        if let existingReceipt {
            let synthetic: AgentCoreJSONValue = .object(["source_host_id": .string(host), "operation_id": .string(operation),
                "attempt_id": .string(attempt), "receipt": existingReceipt])
            let (_, configuration) = try source(synthetic)
            guard case .string(let provider)? = configuration["provider"], provider.hasPrefix("configured:"),
                  let frozenRevision = configuration["configuration_revision"], let frozenDuration = configuration["maximum_duration_ms"] else {
                throw IOSApplicationRuntimeError.rejected("Auxiliary source has no frozen Host model profile")
            }
            profile = String(provider.dropFirst("configured:".count)); revision = frozenRevision; duration = frozenDuration
            let inspected = try record(await auxiliaryRequest(control, path: "inspect", timeout: 30))
            let op = try fields(try fields(inspected)["operation"] ?? .null)
            if op["state"] != .string("ready") { return try result(inspected) }
        } else if let frozenAuxiliaryProfile {
            profile=frozenAuxiliaryProfile.0; revision=frozenAuxiliaryProfile.1
            duration = .unsignedInteger(UInt64(max(1,min(timeout ?? 300,3600))*1000))
        } else {
            let preparation: [String: AgentCoreJSONValue] = ["expected_host_id": .string(host)]
            let prepared = try fields(await auxiliaryRequest(.object(preparation), path: "prepare", timeout: 30))
            guard prepared["source_host_id"] == .string(host), case .string(let selected)? = prepared["profile_id"],
                  let selectedRevision = prepared["revision"], (integer(selectedRevision) ?? 0) > 0 else {
                throw IOSApplicationRuntimeError.rejected("Invalid auxiliary model preparation witness")
            }
            profile = selected; revision = selectedRevision
            duration = .unsignedInteger(UInt64(max(1, min(timeout ?? 300, 3600)) * 1000))
        }
        let request: AgentCoreJSONValue = .object(["expected_host_id": .string(host), "operation_id": .string(operation),
            "attempt_id": .string(attempt), "profile_id": .string(profile), "expected_profile_revision": revision,
            "scenario": .string(scenario.rawValue), "input_json": .string(inputJSON), "maximum_duration_ms": duration])
        let admittedEnvelope = try await auxiliaryRequest(request, path: "admit", timeout: 30)
        let (_, admittedConfiguration) = try source(admittedEnvelope)
        guard admittedConfiguration["provider"] == .string("configured:\(profile)"),
              integer(admittedConfiguration["configuration_revision"]) == integer(revision),
              integer(admittedConfiguration["maximum_duration_ms"]) == integer(duration) else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary admission changed its prepared model or budget")
        }
        let admitted = try record(admittedEnvelope)
        let admittedOp = try fields(try fields(admitted)["operation"] ?? .null)
        if admittedOp["state"] != .string("ready") { return try result(admitted) }
        // The source Ready receipt is durable locally before any physical
        // request. A crash/timeout retains its original operation identity.
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            let envelope: AgentCoreJSONValue
            do {
                envelope = try await auxiliaryRequest(request, path: "invoke", timeout: Double(integer(duration) ?? 300000) / 1000 + 15)
            } catch {
                // Query once using the same identity. Never automatically
                // dispatch again after an uncertain transport outcome.
                guard let inspected = try? await auxiliaryRequest(control, path: "inspect", timeout: 30) else {
                    throw LLMRemoteAuxiliaryOutcomeUnknown(sourceHostID: host, operationID: operation, attemptID: attempt)
                }
                envelope = inspected
            }
            let receipt = try record(envelope)
            try Task.checkCancellation()
            return try result(receipt)
        } onCancel: {
            Task {
                if let response = try? await auxiliaryRequest(control, path: "cancel", timeout: 30) {
                    _ = try? record(response)
                }
            }
        }
    }

    private func auxiliaryRequest(_ input: AgentCoreJSONValue, path: String, timeout: TimeInterval) async throws -> AgentCoreJSONValue {
        guard MobileHostTransportPolicy.allows(config.baseURL), config.expectedHostId != nil else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary request requires a paired secure Host transport")
        }
        let encoded = try JSONEncoder().encode(input)
        guard encoded.count <= 512 * 1024,
              let payload = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] else {
            throw IOSApplicationRuntimeError.rejected("Invalid bounded auxiliary request")
        }
        var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: "auxiliary/\(path)"))
        request.httpMethod = "POST"; request.timeoutInterval = timeout
        request.httpBody = try JSONSerialization.data(withJSONObject: Self.encryptMobileJSON(payload, token: config.token))
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)
        let data: Data
        let response: URLResponse
        if let streamLoader {
            let loaded = try await streamLoader(request)
            var collected = Data()
            for try await byte in loaded.bytes {
                guard collected.count < 4 * 1024 * 1024 else { throw IOSApplicationRuntimeError.rejected("Auxiliary response exceeds budget") }
                collected.append(byte)
            }
            data = collected; response = loaded.response
        } else {
            let session = Self.makeURLSession(pinnedCertificateSHA256: config.pinnedCertificateSHA256)
            defer { if session !== URLSession.shared { session.finishTasksAndInvalidate() } }
            (data, response) = try await session.data(for: request)
        }
        try Self.validateHTTP(response: response, data: data)
        guard data.count <= 4 * 1024 * 1024,
              let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              envelope["type"] as? String == "encrypted" else {
            throw IOSApplicationRuntimeError.rejected("Auxiliary response requires authenticated encryption")
        }
        let decoded = try Self.decryptMobileJSON(envelope, token: config.token)
        return try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: decoded))
    }

    public func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        var userPrefs = ["message": message]
        if !enableThinking {
            userPrefs["enable_thinking"] = "false"
        }
        let input = ModelInput(
            dateRange: ModelInput.DateRange(
                start: ISO8601DateFormatter().string(from: Date().addingTimeInterval(-86400)),
                end: ISO8601DateFormatter().string(from: Date())
            ),
            candidates: [],
            userPrefs: userPrefs
        )
        var text = ""
        for try await chunk in generateSummaryStream(input: input) {
            text += chunk
        }
        return text
    }

    public func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        // String chunks cannot express replacement or retraction. This API
        // returns one final public result; chat uses journal-backed presentation.
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let prepared = try await prepareStandaloneSubmission(makeHostAgentPayload(input: input))
                    var receipt: IOSSourceSubmissionReceipt?
                    var terminalDigest: String?
                    do {
                        for try await event in generateAgentEventStream(input: input,
                            boundRequestDigest: prepared.digest, frozenPayload: prepared.request) {
                            switch event {
                            case .submissionReceipt(let raw):
                                let candidate = try JSONDecoder().decode(IOSSourceSubmissionReceipt.self,
                                    from: JSONEncoder().encode(raw))
                                try validateSummaryReceipt(candidate, identity: prepared.identity, digest: prepared.digest)
                                if let admitted = receipt?.admission, candidate.admission?.identity != admitted.identity {
                                    throw IOSApplicationRuntimeError.rejected("Summary admission changed source epoch")
                                }
                                receipt = candidate
                            case .assistantRunTerminalSealed(let raw):
                                struct Hint: Decodable { let source_host_id: String; let session_id: String; let run_id: String; let seal_digest: String }
                                let hint = try JSONDecoder().decode(Hint.self, from: JSONEncoder().encode(raw))
                                guard hint.source_host_id == prepared.identity.source_host_id,
                                      hint.session_id == prepared.identity.conversation_id,
                                      hint.run_id == prepared.identity.submission_id else {
                                    throw IOSApplicationRuntimeError.rejected("Summary terminal owner changed")
                                }
                                terminalDigest = hint.seal_digest
                            default:
                                // Trace, Part mirrors and EOF are not public text
                                // or authoritative completion evidence.
                                break
                            }
                        }
                    } catch is CancellationError { throw CancellationError() }
                    catch let error as IOSApplicationRuntimeError { throw error }
                    catch let error as DecodingError { throw error }
                    catch {
                        // A transport failure is recoverable only through the
                        // exact original receipt and sealed result below.
                    }
                    try Task.checkCancellation()
                    if receipt?.phase != "admitted" {
                        receipt = try await submissionReceipt(identity: prepared.identity, digest: prepared.digest, cancel: false)
                    }
                    guard let receipt else {
                        throw IOSApplicationRuntimeError.rejected("The Host has not confirmed the original submission. Its outcome remains recoverable; it was not resubmitted.")
                    }
                    try validateSummaryReceipt(receipt, identity: prepared.identity, digest: prepared.digest)
                    guard receipt.phase == "admitted", let admission = receipt.admission else {
                        throw IOSApplicationRuntimeError.rejected(receipt.rejection_code ?? "The original summary submission has not been admitted.")
                    }
                    let text = try await readCompletedSummary(admission: admission, expectedSealDigest: terminalDigest)
                    // Even a transport error can be recovered by an exact sealed
                    // source result. Without a seal the read throws, never success.
                    continuation.yield(text)
                    continuation.finish()
                } catch { continuation.finish(throwing: error) }
            }
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }

    private func validateSummaryReceipt(_ receipt: IOSSourceSubmissionReceipt,
        identity: IOSRemoteSubmissionIdentity, digest: String) throws {
        guard receipt.protocol_version == 1, receipt.identity == identity, receipt.request_digest == digest,
              identity.source_host_id == config.expectedHostId, identity.client_id == config.clientId else {
            throw IOSApplicationRuntimeError.rejected("Summary submission receipt changed owner")
        }
        if receipt.phase == "admitted" {
            guard let admission = receipt.admission,
                  admission.identity.conversation_id == identity.conversation_id,
                  admission.identity.run_id == identity.submission_id,
                  admission.identity.assistant_message_id == identity.assistant_message_id else {
                throw IOSApplicationRuntimeError.rejected("Summary admission does not own the requested root")
            }
        }
    }

    func readCompletedSummary(admission: IOSRunMessageAdmissionCursor, expectedSealDigest: String?) async throws -> String {
        let identity = admission.identity
        let seal: IOSAssistantRunTerminalSeal? = try await journalRequest(.terminalSeal(
            conversationID: identity.conversation_id, runID: identity.run_id))
        guard let seal else {
            throw IOSApplicationRuntimeError.rejected("The Host stream ended without a durable terminal seal. The original Run remains recoverable.")
        }
        guard seal.protocol_version == 1, seal.identity == identity,
              seal.admission.identity == identity,
              expectedSealDigest.map({ $0 == seal.seal_digest }) ?? true,
              seal.result.run_id == identity.run_id,
              seal.result.conversation_id == identity.conversation_id,
              seal.result.value.kind == .answered else {
            throw IOSApplicationRuntimeError.rejected("The original summary Run did not complete with a matching source seal.")
        }
        guard let count = UInt64(seal.entry_count), String(count) == seal.entry_count else {
            throw IOSApplicationRuntimeError.rejected("Invalid summary terminal manifest length")
        }
        var after: UInt64 = 0
        var heads: [(ordinal: Int64, cursor: IOSAssistantJournalCursor)] = []
        var seen = Set<String>()
        repeat {
            let page: IOSAssistantRunTerminalManifestPage = try await journalRequest(.terminalManifest(seal, afterEntry: String(after)))
            guard page.seal == seal, page.after_entry == String(after), page.entries.count <= 64,
                  let next = UInt64(page.next_entry), String(next) == page.next_entry,
                  next >= after, next - after == UInt64(page.entries.count), next <= count,
                  page.complete == (next == count), page.complete || next > after else {
                throw IOSApplicationRuntimeError.rejected("Invalid summary terminal manifest page")
            }
            for entry in page.entries {
                if case .assistant(let through, let ordinal, _, _, _) = entry {
                    guard through.identity.source_store_epoch == identity.source_store_epoch,
                          through.identity.conversation_id == identity.conversation_id,
                          through.identity.run_id == identity.run_id else {
                        throw IOSApplicationRuntimeError.rejected("Summary manifest contains a foreign Run")
                    }
                    guard seen.insert(through.identity.assistant_message_id).inserted else {
                        throw IOSApplicationRuntimeError.rejected("Duplicate summary assistant")
                    }
                    heads.append((ordinal, through))
                }
            }
            after = next
            if page.complete { break }
        } while true
        let ordered = try Self.summaryAssistantHeads(heads, root: identity, expectedCount: seal.heads_count)
        var text = ""
        for head in ordered {
            // Each read stays pinned to its own manifest-proven message owner.
            let reader = IOSAssistantJournalReader(transport: { command in
                try await readSummaryJournalCommand(command, owner: head.identity)
            })
            text += try await reader.read(at: head).text
        }
        return text
    }

    static func summaryAssistantHeads(_ heads: [(ordinal: Int64, cursor: IOSAssistantJournalCursor)],
        root: IOSAssistantJournalIdentity, expectedCount: String) throws -> [IOSAssistantJournalCursor] {
        guard let count = UInt64(expectedCount), String(count) == expectedCount,
              count == UInt64(heads.count),
              Set(heads.map { $0.ordinal }).count == heads.count,
              Set(heads.map { $0.cursor.identity.assistant_message_id }).count == heads.count,
              heads.allSatisfy({ $0.ordinal >= 0 && $0.cursor.identity.source_store_epoch == root.source_store_epoch
                  && $0.cursor.identity.conversation_id == root.conversation_id
                  && $0.cursor.identity.run_id == root.run_id }),
              heads.isEmpty || heads.contains(where: { $0.cursor.identity == root }) else {
            throw IOSApplicationRuntimeError.rejected("Invalid sealed summary assistant ownership")
        }
        return heads.sorted { $0.ordinal < $1.ordinal }.map(\.cursor)
    }

    private func readSummaryJournalCommand(_ command: AgentCoreJSONValue,
        owner: IOSAssistantJournalIdentity) async throws -> AgentCoreJSONValue {
        guard case .object(var fields) = command, case .string(let type)? = fields.removeValue(forKey: "type") else {
            throw IOSApplicationRuntimeError.rejected("Invalid summary journal read")
        }
        let operations = ["open_assistant_journal_snapshot":"open_snapshot",
            "read_assistant_journal_parts":"parts",
            "read_assistant_journal_part_changes":"changes", "read_assistant_journal_range":"range",
            "release_assistant_journal_snapshot":"release_snapshot"]
        guard let operation = operations[type] else { throw IOSApplicationRuntimeError.rejected("Unsupported summary journal read") }
        let at: AgentCoreJSONValue?
        if type == "open_assistant_journal_snapshot" { at = fields["at"] }
        else if case .object(let snapshot)? = fields["snapshot"] { at = snapshot["at"] }
        else { at = nil }
        guard let at, try JSONDecoder().decode(IOSAssistantJournalCursor.self,
            from: JSONEncoder().encode(at)).identity == owner else {
            throw IOSApplicationRuntimeError.rejected("Summary journal request escaped its admitted root")
        }
        fields["operation"] = .string(operation)
        return try await pairedReadRequest(AgentCoreJSONValue.object(fields), path: "chat/agent/v2/journal")
    }

    static func summaryStreamToolUseLine(for toolName: String) -> String {
        localizedFormat(
            "chat.web_agent.summary.tool_running_format",
            value: "\n\n🔧 %@ is running…\n",
            toolName
        )
    }

    static func summaryStreamToolResultLine(for tool: HostAgentToolEvent) -> String {
        let summary: String
        if tool.containsSensitiveData {
            summary = localized(
                "chat.web_agent.summary.sensitive_hidden",
                value: "Sensitive tool result hidden"
            )
        } else {
            let rawSummary = tool.display.isEmpty ? tool.text : tool.display
            let redacted = HostAgentToolEvent.redactSensitiveContent(rawSummary)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            summary = redacted.isEmpty
                ? localized("chat.web_agent.summary.done", value: "Done")
                : redacted
        }
        return localizedFormat(
            "chat.web_agent.summary.tool_result_format",
            value: "\n✅ %@: %@\n",
            tool.name,
            summary
        )
    }

    static func summaryStreamPermissionLine(toolName: String, summary: String) -> String {
        localizedFormat(
            "chat.web_agent.summary.permission_required_format",
            value: "\n⚠️ %@ requires permission: %@\n",
            toolName,
            summary
        )
    }

    static func summaryStreamErrorLine(for message: String) -> String {
        let redacted = HostAgentToolEvent.redactSensitiveContent(message)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return localizedFormat(
            "chat.web_agent.summary.error_format",
            value: "\nError: %@\n",
            redacted.isEmpty ? localized("chat.web_agent.summary.done", value: "Done") : redacted
        )
    }

    static func summaryStreamMaxIterationsNotice() -> String {
        localized(
            "chat.web_agent.summary.max_iterations_notice",
            value: "\n\nReached this turn's maximum agent iterations, returning the current result. Send another message to continue."
        )
    }

    public func generateAgentEventStream(input: ModelInput) -> AsyncThrowingStream<WebAgentStreamEvent, Error> {
        generateAgentEventStream(input: input, boundRequestDigest: nil)
    }

    func generateAgentEventStream(input: ModelInput, boundRequestDigest: String?, frozenPayload: AgentCoreJSONValue? = nil) -> AsyncThrowingStream<WebAgentStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    try await streamEvents(input: input, boundRequestDigest: boundRequestDigest, frozenPayload: frozenPayload, continuation: continuation)
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { @Sendable _ in task.cancel() }
        }
    }

    public func submitHostPermissionDecision(_ payload: [String: Any]) async throws {
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        var request = URLRequest(
            url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/permission")
        )
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.httpBody = try JSONSerialization.data(
            withJSONObject: Self.encryptMobileJSON(payload, token: config.token)
        )
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)

        let session = Self.makeURLSession(
            pinnedCertificateSHA256: config.pinnedCertificateSHA256
        )
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        let (data, response) = try await session.data(for: request)
        try Self.validateHTTP(response: response, data: data)
    }

    func cancelHostGeneration(identity: IOSRemoteSubmissionIdentity, requestDigest: String) async throws {
        guard identity.source_host_id == journalSourceHostID,
              identity.client_id == observationClientID, !requestDigest.isEmpty else {
            throw NolaError.llm(.invalidResponse(reason: "Cancellation must retain its admitted submission owner."))
        }
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        var request = URLRequest(
            url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/cancel")
        )
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.httpBody = try JSONSerialization.data(
            withJSONObject: Self.encryptMobileJSON(
                try AppleRemoteCancellationRequest(identity: identity, requestDigest: requestDigest).object(),
                token: config.token
            )
        )
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)

        if let streamLoader {
            let loaded = try await streamLoader(request)
            var data = Data()
            for try await byte in loaded.bytes {
                guard data.count < 64 * 1024 else { throw IOSApplicationRuntimeError.rejected("Cancellation response exceeds control limit") }
                data.append(byte)
            }
            try Self.validateHTTP(response: loaded.response, data: data)
            return
        }

        let session = Self.makeURLSession(
            pinnedCertificateSHA256: config.pinnedCertificateSHA256
        )
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        let (data, response) = try await session.data(for: request)
        try Self.validateHTTP(response: response, data: data)
    }

    public func cancelHostTask(
        conversationId: String,
        parentRunId: String,
        taskId: String
    ) async throws -> AppleTaskCancellationReceipt {
        guard let hostId = config.expectedHostId, !hostId.isEmpty else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let intent = AppleTaskCancellationIntent(hostId: hostId, conversationId: conversationId,
            parentRunId: parentRunId, taskId: taskId)
        return try await AppleTaskCancellationTransport.request(intent) { path, data in
            guard let payload = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return try await self.submitHostTaskPayload(path: path, payload: payload)
        }
    }

    public func submitHostTaskGuidance(
        conversationId: String,
        parentRunId: String,
        taskId: String,
        message: String
    ) async throws -> AppleTaskGuidanceReceipt {
        guard let hostId = config.expectedHostId else { throw AppleTaskGuidanceOutbox.Failure.invalidOwner }
        return try await AppleTaskGuidanceOutbox.shared.submit(hostId: hostId,
            conversationId: conversationId, parentRunId: parentRunId, taskId: taskId, message: message) { path, data in
                guard let payload = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    throw AppleTaskGuidanceOutbox.Failure.invalidReceipt
                }
                return try await self.submitHostTaskPayload(path: path, payload: payload)
            }
    }

    public func recoverHostTaskGuidance(conversationId: String, parentRunId: String) async throws {
        guard let hostId = config.expectedHostId else { throw AppleTaskGuidanceOutbox.Failure.invalidOwner }
        try await AppleTaskGuidanceOutbox.shared.recover(hostId: hostId,
            conversationId: conversationId, parentRunId: parentRunId) { path, data in
                guard let payload = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    throw AppleTaskGuidanceOutbox.Failure.invalidReceipt
                }
                return try await self.submitHostTaskPayload(path: path, payload: payload)
            }
    }

    public func taskProjectionEvents(
        conversationId: String,
        afterCursor: UInt64
    ) -> AsyncThrowingStream<WebAgentTaskProjectionEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard MobileHostTransportPolicy.allows(config.baseURL) else {
                        throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
                    }
                    var request = URLRequest(
                        url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/task/events")
                    )
                    request.httpMethod = "POST"
                    request.timeoutInterval = 35
                    request.httpBody = try JSONSerialization.data(
                        withJSONObject: Self.encryptMobileJSON(
                            [
                                "conversationId": conversationId,
                                "afterCursor": afterCursor,
                                "timeoutMs": 25_000,
                            ],
                            token: config.token
                        )
                    )
                    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
                    Self.applyMobileAuth(to: &request, token: config.token)

                    let session = Self.makeURLSession(
                        pinnedCertificateSHA256: config.pinnedCertificateSHA256
                    )
                    defer {
                        if session !== URLSession.shared {
                            session.finishTasksAndInvalidate()
                        }
                    }
                    let (bytes, response) = try await session.bytes(for: request)
                    if let http = response as? HTTPURLResponse,
                       !(200 ... 299).contains(http.statusCode) {
                        var body = Data()
                        for try await byte in bytes { body.append(byte) }
                        try Self.validateHTTP(response: http, data: body)
                    }

                    var dataLines: [String] = []
                    func flushFrame() throws {
                        guard let raw = Self.sseFramePayload(from: dataLines) else {
                            dataLines.removeAll()
                            return
                        }
                        dataLines.removeAll()
                        guard raw != "[DONE]",
                              let data = raw.data(using: .utf8),
                              let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                            return
                        }
                        let decoded = envelope["type"] as? String == "encrypted"
                            ? try Self.decryptMobileJSON(envelope, token: config.token)
                            : envelope
                        guard decoded["type"] as? String == "task_projection" else { return }
                        let eventConversationId = decoded["conversationId"] as? String
                            ?? conversationId
                        let cursor = (decoded["cursor"] as? NSNumber)?.uint64Value
                            ?? afterCursor
                        let projection: AgentCoreTaskProjection? = try {
                            guard let object = decoded["taskProjection"],
                                  !(object is NSNull),
                                  JSONSerialization.isValidJSONObject(object) else { return nil }
                            let projectionData = try JSONSerialization.data(withJSONObject: object)
                            return try JSONDecoder().decode(AgentCoreTaskProjection.self, from: projectionData)
                        }()
                        continuation.yield(
                            WebAgentTaskProjectionEvent(
                                conversationId: eventConversationId,
                                cursor: cursor,
                                projection: projection
                            )
                        )
                    }

                    // AsyncLineSequence omits empty lines; SSE uses them as
                    // frame boundaries. Preserve raw framing like consumeEvents.
                    var lineFramer = WebAgentSSELineFramer()
                    var iterator = bytes.makeAsyncIterator()
                    while let byte = try await IOSHTTPByteIterator.next(&iterator) {
                        try Task.checkCancellation()
                        if let line = lineFramer.append(byte) {
                            if line.trimmingCharacters(in: .newlines).isEmpty {
                                try flushFrame()
                            } else if let value = Self.sseDataLineValue(line) {
                                dataLines.append(value)
                            }
                        }
                    }
                    if let line = lineFramer.finish(), let value = Self.sseDataLineValue(line) {
                        dataLines.append(value)
                    }
                    try flushFrame()
                    continuation.finish()
                } catch is CancellationError {
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func submitHostTaskPayload(path: String, payload: [String: Any]) async throws -> Data {
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: path))
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.httpBody = try JSONSerialization.data(
            withJSONObject: Self.encryptMobileJSON(payload, token: config.token)
        )
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)

        let session = Self.makeURLSession(
            pinnedCertificateSHA256: config.pinnedCertificateSHA256
        )
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        let (data, response) = try await session.data(for: request)
        try Self.validateHTTP(response: response, data: data)
        guard let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NolaError.llm(.invalidResponse(reason: "Invalid Host task response."))
        }
        let decoded = envelope["type"] as? String == "encrypted"
            ? try Self.decryptMobileJSON(envelope, token: config.token)
            : envelope
        return try JSONSerialization.data(withJSONObject: decoded)
    }

    public func submitDeviceToolResult(_ payload: [String: Any]) async throws {
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        var request = URLRequest(
            url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/device-tool")
        )
        request.httpMethod = "POST"
        request.timeoutInterval = 30
        request.httpBody = try JSONSerialization.data(
            withJSONObject: Self.encryptMobileJSON(payload, token: config.token)
        )
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)

        let session = Self.makeURLSession(
            pinnedCertificateSHA256: config.pinnedCertificateSHA256
        )
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }
        let (data, response) = try await session.data(for: request)
        try Self.validateHTTP(response: response, data: data)
    }

    func matchesUIHost(baseURL: String?, certificateSHA256: String?) -> Bool {
        guard let baseURL, let url = URL(string: baseURL), config.expectedHostId?.isEmpty == false else { return false }
        return config.baseURL == url && config.pinnedCertificateSHA256 == Config.normalizedFingerprint(certificateSHA256)
    }

    var journalSourceHostID: String? { config.expectedHostId }
    var uiIntentSource: AppleUIIntentSource {
        AppleUIIntentSource(hostID: config.expectedHostId, address: config.baseURL.absoluteString,
            certificate: config.pinnedCertificateSHA256, clientID: config.clientId)
    }
    var observationClientID: String { config.clientId }

    func conversationProgress(conversationID: String) async throws -> IOSInspectorProgressSnapshot {
        guard let source = config.expectedHostId, !source.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("Progress requires a paired Host")
        }
        struct Request: Encodable { let expected_host_id: String; let conversation_id: String }
        let result: AgentCoreJSONValue = try await pairedReadRequest(
            Request(expected_host_id: source, conversation_id: conversationID), path: "chat/agent/v2/progress", envelope: .sourceProjection)
        var snapshot = try IOSInspectorProgressSnapshot.remote(result, sourceHostID: source, conversationID: conversationID)
        do {
            _ = try await AppleGoalControlOutbox.shared.reconcile(hostID: source, conversationID: conversationID,
                appliedOperations: snapshot.facts?.goal_projection?.applied_operations ?? [:])
        } catch { snapshot.goalControlNotice = error.localizedDescription }
        snapshot.supportsGoalControl = true
        snapshot.pendingGoalControl = try await AppleGoalControlOutbox.shared.pending(hostID: source, conversationID: conversationID)
        snapshot.goalDraft = try await AppleGoalControlOutbox.shared.draft(hostID: source, conversationID: conversationID)
        return snapshot
    }

    func controlConversationGoal(conversationID: String, control: AppleGoalControl?, outbox: AppleGoalControlOutbox = .shared) async throws -> Bool {
        guard let source = config.expectedHostId, !source.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        let intent: AppleGoalControlIntent
        if let control { intent = AppleGoalControlIntent(hostID: source, conversationID: conversationID, control: control) }
        else {
            guard let saved = try await outbox.pending(hostID: source, conversationID: conversationID) else {
                throw AppleGoalControlOutbox.Failure.invalidIntent
            }
            intent = saved
        }
        return try await outbox.submit(intent) { [self] saved in
            guard saved.expected_host_id == source, saved.conversation_id == conversationID else { throw IOSApplicationRuntimeError.invalidResponse }
            return try await pairedReadRequest(saved, path: "chat/agent/v2/goal", envelope: .goalControl,
                acceptedStatusCodes: [200, 202, 409])
        }
    }

    func taskChildWindow(_ query: AppleTaskChildWindowRequest) async throws -> IOSConversationProjectionWindowV1 {
        guard case .object(var fields) = try JSONDecoder().decode(AgentCoreJSONValue.self,
            from: JSONEncoder().encode(query)) else { throw IOSApplicationRuntimeError.invalidResponse }
        fields["operation"] = .string("task_child_window")
        return try await pairedReadRequest(AgentCoreJSONValue.object(fields), path: "chat/agent/v2/journal",
            responseByteLimit: 4 * 1024 * 1024)
    }

    func taskChildJournalContent(at cursor: IOSAssistantJournalCursor) async throws -> IOSJournalMessageContent {
        let reader = IOSAssistantJournalReader(transport: { command in
            try await self.readSummaryJournalCommand(command, owner: cursor.identity)
        })
        return try await reader.read(at: cursor)
    }

    func journalRequest<Result: Decodable & Sendable>(_ input: IOSAssistantJournalReadRequest) async throws -> Result {
        try await pairedReadRequest(input, path: "chat/agent/v2/journal")
    }

    func compactContext(owner: IOSConfirmedTerminalOwner) async throws -> IOSRemoteContextCompactionReceipt {
        guard let sourceHost = config.expectedHostId else {
            throw IOSApplicationRuntimeError.rejected("Context maintenance requires a paired Host")
        }
        struct Request: Encodable {
            let expected_host_id: String
            let client_id: String
            let conversation_id: String
            let expected_run_id: String
            let assistant_message_id: String
        }
        let receipt: IOSRemoteContextCompactionReceipt
        do {
            receipt = try await pairedReadRequest(Request(expected_host_id: sourceHost,
                client_id: config.clientId, conversation_id: owner.conversation_id,
                expected_run_id: owner.run_id, assistant_message_id: owner.assistant_message_id),
                path: "chat/agent/v2/context/compact", timeout: 180)
        } catch {
            // A lost reply does not prove that the Host failed to commit. Never
            // replay this maintenance command automatically or mutate a mirror.
            throw IOSApplicationRuntimeError.rejected(NSLocalizedString("chat.context.host.unconfirmed",
                value: "Host context maintenance could not be confirmed. Reload the Host state before trying again.",
                comment: "Unknown authoritative maintenance result"))
        }
        try receipt.validate(conversationID: owner.conversation_id)
        return receipt
    }

    func submitUIInteraction(_ input: AppleUISubmission<AgentCoreJSONValue>) async -> AppleUISubmissionOutcome<AgentCoreJSONValue> {
        guard let source = config.expectedHostId else { return .rejected("UI interaction requires a paired Host") }
        struct Request: Encodable {
            let expected_host_id: String
            let input: AppleUISubmission<AgentCoreJSONValue>
        }
        nonisolated struct Response: Decodable, Sendable {
            let status: String
            let receipt: AppleUIInteractionReceipt<AgentCoreJSONValue>?
            let message: String?
        }
        do {
            let result: Response = try await pairedReadRequest(Request(expected_host_id: source, input: input),
                path: "chat/agent/v2/ui/interactions", responseByteLimit: 2 * 1024 * 1024,
                requestByteLimit: 128 * 1024)
            switch result.status {
            case "committed":
                guard let receipt = result.receipt,
                      receipt.snapshot.identity == input.interaction.identity,
                      receipt.snapshot.revision > input.interaction.expected_surface_revision,
                      receipt.snapshot.revision - input.interaction.expected_surface_revision == 1,
                      receipt.queued_submission_id == nil || receipt.queued_submission_id == input.submission_id else {
                    return .unknown("UI receipt identity or revision changed")
                }
                return .committed(receipt)
            case "rejected", "conflict": return .rejected(result.message ?? "UI interaction was rejected")
            case "unknown": return .unknown(result.message ?? "UI interaction outcome is unknown")
            default: return .unknown("Invalid UI interaction outcome")
            }
        } catch { return .unknown(error.localizedDescription) }
    }

    nonisolated struct UIQueueHead: Decodable, Sendable {
        let command_id: String
        let kind: String
        let user_message_id: String?
        let lease_owner: String?
        let lease_until_ms: Int64?

        func validate() throws {
            guard !command_id.isEmpty,
                  (lease_owner == nil) == (lease_until_ms == nil),
                  ["user_input", "active_submission", "permission_policy", "task_terminal", "task_guidance"].contains(kind) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
    }

    nonisolated struct UIQueueState: Decodable, Sendable {
        let head: UIQueueHead?
        let dispatch: IOSSourceSubmissionReceipt?
        private enum CodingKeys: String, CodingKey { case head, dispatch }
        init(from decoder: Decoder) throws {
            let fields = try decoder.container(keyedBy: CodingKeys.self)
            guard fields.contains(.head), fields.contains(.dispatch) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Queue state requires head and dispatch"))
            }
            head = try fields.decodeIfPresent(UIQueueHead.self, forKey: .head)
            dispatch = try fields.decodeIfPresent(IOSSourceSubmissionReceipt.self, forKey: .dispatch)
        }
        func validate(hostID: String, clientID: String, conversationID: String) throws {
            try head?.validate()
            guard let receipt = dispatch else { return }
            guard let head, receipt.protocol_version == 1,
                  receipt.identity.source_host_id == hostID, receipt.identity.client_id == clientID,
                  receipt.identity.conversation_id == conversationID, receipt.identity.submission_id == head.command_id,
                  receipt.identity.user_message_id == head.user_message_id,
                  !receipt.request_digest.isEmpty,
                  ["prepared", "admitted", "rejected", "cancelled"].contains(receipt.phase) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
    }

    func idleUIQueueHead(conversationID: String, expectedSourceStoreEpoch: String) async throws -> UIQueueState {
        guard let source = config.expectedHostId else {
            throw IOSApplicationRuntimeError.rejected("UI queue requires a paired Host")
        }
        guard !expectedSourceStoreEpoch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        struct Query: Encodable { let expected_host_id: String; let expected_source_store_epoch: String; let conversation_id: String; let client_id: String }
        let state: UIQueueState = try await pairedReadRequest(
            Query(expected_host_id: source, expected_source_store_epoch: expectedSourceStoreEpoch, conversation_id: conversationID, client_id: config.clientId),
            path: "chat/agent/v2/ui/queued-input", responseByteLimit: 64 * 1024)
        try state.validate(hostID: source, clientID: config.clientId, conversationID: conversationID)
        return state
    }

    func cancelQueuedUIInput(conversationID: String, expectedSourceStoreEpoch: String, commandID: String, requestDigest: String) async throws {
        guard !expectedSourceStoreEpoch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        struct Query: Encodable { let expected_host_id: String; let expected_source_store_epoch: String; let conversation_id: String; let client_id: String; let command_id: String; let request_digest: String }
        nonisolated struct Result: Decodable, Sendable { let cancelled: Bool; let command_id: String }
        guard let source = config.expectedHostId else { throw IOSApplicationRuntimeError.invalidResponse }
        let result: Result = try await pairedReadRequest(Query(expected_host_id: source, expected_source_store_epoch: expectedSourceStoreEpoch, conversation_id: conversationID,
            client_id: config.clientId, command_id: commandID, request_digest: requestDigest), path: "chat/agent/v2/ui/queued-input/cancel")
        guard result.cancelled, result.command_id == commandID else { throw IOSApplicationRuntimeError.invalidResponse }
    }

    func previewUISurface(_ input: AppleUIPreviewInput<AgentCoreJSONValue>) async throws -> AppleUIPreviewResult<AgentCoreJSONValue> {
        struct Request: Encodable {
            let expected_host_id: String
            let identity: AppleUISurfaceIdentity
            let expected_surface_revision: UInt64
            let edits: [AppleUIEdit<AgentCoreJSONValue>]
            let draft_generation: UInt64
        }
        guard let source = config.expectedHostId else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await pairedReadRequest(Request(expected_host_id: source, identity: input.identity,
            expected_surface_revision: input.expected_surface_revision, edits: input.edits, draft_generation: input.draft_generation),
            path: "chat/agent/v2/ui/preview", responseByteLimit: 2 * 1024 * 1024)
    }

    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?) async throws -> AppleUISurfaceUpdate<AgentCoreJSONValue> {
        struct Request: Encodable { let expected_host_id: String; let identity: AppleUISurfaceIdentity; let base_revision: UInt64? }
        guard let source = config.expectedHostId else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await pairedReadRequest(Request(expected_host_id: source, identity: identity, base_revision: baseRevision),
            path: "chat/agent/v2/ui/changes", responseByteLimit: 2 * 1024 * 1024)
    }

    func readUISurfaceRevision(_ reference: AppleUISurfaceReference) async throws -> AppleUISnapshot<AgentCoreJSONValue>? {
        struct Request: Encodable {
            let expected_host_id: String
            let identity: AppleUISurfaceIdentity
            let revision: UInt64
        }
        guard let source = config.expectedHostId, !source.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        let snapshot: AppleUISnapshot<AgentCoreJSONValue>? = try await pairedReadRequest(
            Request(expected_host_id: source, identity: reference.identity, revision: reference.revision),
            path: "chat/agent/v2/ui/revision", responseByteLimit: 2 * 1024 * 1024)
        guard snapshot == nil || (snapshot?.identity == reference.identity && snapshot?.revision == reference.revision
            && (snapshot?.phase == .readOnly || snapshot?.phase == .deleted)) else { throw IOSApplicationRuntimeError.invalidResponse }
        return snapshot
    }

    func listUISurfaces(conversationID: String, expectedSourceStoreEpoch: String) async throws -> [AppleUISnapshot<AgentCoreJSONValue>] {
        guard let source = config.expectedHostId else {
            throw IOSApplicationRuntimeError.rejected("UI projection requires a paired Host")
        }
        guard !expectedSourceStoreEpoch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        struct Query: Encodable { let expected_host_id: String; let expected_source_store_epoch: String; let conversation_id: String }
        let surfaces: [AppleUISnapshot<AgentCoreJSONValue>] = try await pairedReadRequest(
            Query(expected_host_id: source, expected_source_store_epoch: expectedSourceStoreEpoch, conversation_id: conversationID),
            path: "chat/agent/v2/ui/surfaces", responseByteLimit: 16 * 1024 * 1024)
        guard surfaces.count <= 8,
              surfaces.allSatisfy({ $0.identity.owner.conversation_id == conversationID && $0.identity.owner.authority_id == expectedSourceStoreEpoch }) else {
            throw IOSApplicationRuntimeError.rejected("UI projection belongs to another conversation or exceeds its budget")
        }
        return surfaces
    }

    func observeRun(_ input: IOSRunObservationRequest) async throws -> IOSRunObservation? {
        let envelope: IOSRunObservationEnvelope = try await pairedReadRequest(input,
            path: "chat/agent/v2/observe", envelope: .sourceProjection)
        return try envelope.validated(for: input)
    }

    func permissionConfiguration(owner: IOSRemoteSubmissionIdentity,
        expectedRevision: Int64? = nil, profile: IOSPermissionMode? = nil, minimumPolicyRevision: Int64? = nil) async throws -> IOSRemotePermissionConfiguration {
        struct Request: Encodable {
            let operation: String
            let expected_host_id: String
            let client_id: String
            let conversation_id: String
            let expected_run_id: String
            let assistant_message_id: String
            let expected_revision: Int64?
            let permission_profile: IOSPermissionMode?
            let minimum_policy_revision: Int64?
        }
        guard owner.source_host_id == config.expectedHostId, owner.client_id == config.clientId,
              expectedRevision.map({ $0 >= 0 && $0 < Int64.max }) ?? true,
              (expectedRevision == nil) == (profile == nil) else {
            throw IOSApplicationRuntimeError.rejected("Permission configuration pairing changed")
        }
        let value: IOSRemotePermissionConfiguration = try await pairedReadRequest(Request(
            operation: profile == nil ? "read" : "patch", expected_host_id: owner.source_host_id,
            client_id: owner.client_id, conversation_id: owner.conversation_id,
            expected_run_id: owner.submission_id, assistant_message_id: owner.assistant_message_id,
            expected_revision: expectedRevision, permission_profile: profile, minimum_policy_revision: minimumPolicyRevision), path: "chat/agent/v2/configuration")
        try value.validate(conversationID: owner.conversation_id, runID: owner.submission_id)
        if value.owner.kind == .remote, value.owner.identity != owner {
            throw IOSApplicationRuntimeError.rejected("Permission configuration source identity changed")
        }
        if value.owner.kind == .local, value.owner.assistant_message_id != owner.assistant_message_id {
            throw IOSApplicationRuntimeError.rejected("Permission configuration assistant owner changed")
        }
        if let profile, let expectedRevision {
            guard value.permission_profile == profile, value.configuration_revision == expectedRevision + 1 else {
                throw IOSApplicationRuntimeError.rejected("Host permission save was not confirmed")
            }
        }
        return value
    }

    func submissionReceipt(identity: IOSRemoteSubmissionIdentity, digest: String, cancel: Bool) async throws -> IOSSourceSubmissionReceipt? {
        struct Query: Encodable { let operation: String; let identity: IOSRemoteSubmissionIdentity; let request_digest: String }
        guard identity.source_host_id == config.expectedHostId, identity.client_id == config.clientId else {
            throw IOSApplicationRuntimeError.rejected("Submission source identity changed")
        }
        let receipt: IOSSourceSubmissionReceipt? = try await pairedReadRequest(Query(operation: cancel ? "cancel" : "observe",
            identity: identity, request_digest: digest), path: "chat/agent/v2/submission")
        guard receipt == nil || (receipt?.identity == identity && receipt?.request_digest == digest) else {
            throw IOSApplicationRuntimeError.rejected("Submission receipt owner changed")
        }
        return receipt
    }

    func resumeObservedRun(_ run: IOSRunObservation) -> AsyncThrowingStream<WebAgentStreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard let host = config.expectedHostId, run.client_id == config.clientId,
                          run.recovery == "resume_required", run.worker.state == "absent",
                          MobileHostTransportPolicy.allows(config.baseURL) else {
                        throw IOSApplicationRuntimeError.rejected("Resume requires an exact recoverable paired Host Run")
                    }
                    try run.validate(conversation: run.conversation_id, client: config.clientId, previous: nil)
                    var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/resume"))
                    request.httpMethod = "POST"; request.timeoutInterval = 60
                    request.httpBody = try JSONSerialization.data(withJSONObject: Self.encryptMobileJSON([
                        "conversation_id": run.conversation_id, "expected_run_id": run.run_id,
                        "expected_host_id": host, "client_id": config.clientId,
                    ], token: config.token))
                    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
                    Self.applyMobileAuth(to: &request, token: config.token)
                    if let streamLoader {
                        let loaded = try await streamLoader(request)
                        try await consumeEvents(bytes: loaded.bytes, response: loaded.response, continuation: continuation)
                    } else {
                        let session = Self.makeURLSession(pinnedCertificateSHA256: config.pinnedCertificateSHA256)
                        defer { if session !== URLSession.shared { session.finishTasksAndInvalidate() } }
                        let (bytes, response) = try await session.bytes(for: request)
                        try await consumeEvents(bytes: bytes, response: response, continuation: continuation)
                    }
                    continuation.finish()
                } catch { continuation.finish(throwing: error) }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private enum PairedReadEnvelope { case result, sourceProjection, goalControl }

    private func pairedReadRequest<Input: Encodable, Result: Decodable & Sendable>(_ input: Input, path: String, envelope responseEnvelope: PairedReadEnvelope = .result, timeout: TimeInterval = 30, responseByteLimit: Int = 512 * 1024, requestByteLimit: Int = 64 * 1024, acceptedStatusCodes: Set<Int>? = nil) async throws -> Result {
        guard let source = config.expectedHostId, MobileHostTransportPolicy.allows(config.baseURL) else {
            throw IOSApplicationRuntimeError.rejected("Journal requires a paired authenticated Host")
        }
        let encoded = try JSONEncoder().encode(input)
        guard encoded.count <= requestByteLimit, let payload = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] else {
            throw IOSApplicationRuntimeError.rejected("Invalid journal request budget")
        }
        var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: path))
        request.httpMethod = "POST"
        request.timeoutInterval = timeout
        request.httpBody = try JSONSerialization.data(withJSONObject: Self.encryptMobileJSON(payload, token: config.token))
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        Self.applyMobileAuth(to: &request, token: config.token)
        func collect<Bytes: AsyncSequence>(_ bytes: Bytes, response: URLResponse) async throws -> Result where Bytes.Element == UInt8, Bytes.AsyncIterator: SendableMetatype {
            var data = Data()
            var iterator = bytes.makeAsyncIterator()
            while let byte = try await IOSHTTPByteIterator.next(&iterator) {
                guard data.count < responseByteLimit else { throw IOSApplicationRuntimeError.rejected("Paired response exceeds budget") }
                data.append(byte)
            }
            if let http = response as? HTTPURLResponse,
               !(acceptedStatusCodes?.contains(http.statusCode) ?? (200...299).contains(http.statusCode)) {
                throw IOSRunObservationTransportError.http(http.statusCode)
            }
            guard let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any], envelope["type"] as? String == "encrypted" else {
                throw IOSApplicationRuntimeError.rejected("Journal response is not authenticated encryption")
            }
            let object = try Self.decryptMobileJSON(envelope, token: config.token)
            guard object["source_host_id"] as? String == source else {
                throw IOSApplicationRuntimeError.rejected("Journal source Host changed")
            }
            let result: Any
            switch responseEnvelope {
            case .result:
                guard let value = object["result"] else { throw IOSApplicationRuntimeError.invalidResponse }
                result = value
            case .sourceProjection:
                result = object
            case .goalControl:
                var control = object
                if (response as? HTTPURLResponse)?.statusCode == 202 {
                    control["controlPending"] = true
                    control["authoritative"] = false
                }
                result = control
            }
            return try JSONDecoder().decode(Result.self, from: JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]))
        }
        if let streamLoader {
            let loaded = try await streamLoader(request)
            return try await collect(loaded.bytes, response: loaded.response)
        }
        let session = Self.makeURLSession(pinnedCertificateSHA256: config.pinnedCertificateSHA256)
        defer { if session !== URLSession.shared { session.finishTasksAndInvalidate() } }
        let (bytes, response) = try await session.bytes(for: request)
        return try await collect(bytes, response: response)
    }

    private func prepareStandaloneSubmission(_ original: [String: Any]) async throws
        -> (identity: IOSRemoteSubmissionIdentity, digest: String, request: AgentCoreJSONValue) {
        var payload = original
        // Standalone summary/model-test calls still own a new explicit
        // submission; these identifiers are created once before hashing.
        if payload["conversationId"] == nil { payload["conversationId"] = UUID().uuidString }
        if payload["submission_id"] == nil { payload["submission_id"] = UUID().uuidString }
        if payload["user_message_id"] == nil { payload["user_message_id"] = UUID().uuidString }
        if payload["assistant_message_id"] == nil { payload["assistant_message_id"] = "\(payload["submission_id"] as! String):assistant" }
        guard let host = config.expectedHostId, let conversation = payload["conversationId"] as? String,
              let submission = payload["submission_id"] as? String, let user = payload["user_message_id"] as? String,
              let assistant = payload["assistant_message_id"] as? String else {
            throw IOSApplicationRuntimeError.rejected("Submission requires explicit conversation and message identity")
        }
        let identity = IOSRemoteSubmissionIdentity(source_host_id: host, client_id: config.clientId,
            conversation_id: conversation, submission_id: submission, user_message_id: user, assistant_message_id: assistant)
        payload = try await prepareHostToolPlan(payload)
        struct Query: Encodable { let operation = "prepare"; let identity: IOSRemoteSubmissionIdentity; let request: AgentCoreJSONValue }
        nonisolated struct Prepared: Decodable, Sendable { let identity: IOSRemoteSubmissionIdentity; let request_digest: String }
        let prepared: Prepared = try await pairedReadRequest(Query(identity: identity,
            request: JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: payload))),
            path: "chat/agent/v2/submission")
        guard prepared.identity == identity else { throw IOSApplicationRuntimeError.rejected("Submission preparation owner changed") }
        guard prepared.request_digest.utf8.count == 64,
              prepared.request_digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw IOSApplicationRuntimeError.rejected("Invalid prepared submission digest")
        }
        let frozen = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: payload))
        return (identity, prepared.request_digest, frozen)
    }

    private func streamEvents(input: ModelInput, boundRequestDigest: String?, frozenPayload: AgentCoreJSONValue?, continuation: AsyncThrowingStream<WebAgentStreamEvent, Error>.Continuation) async throws {
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw NolaError.llm(.invalidResponse(reason: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network."))
        }
        var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: "chat/agent/v2/stream"))
        request.httpMethod = "POST"
        request.timeoutInterval = 60
        var payload: [String: Any]
        if let frozenPayload {
            guard let frozen = try JSONSerialization.jsonObject(with: JSONEncoder().encode(frozenPayload)) as? [String: Any] else {
                throw IOSApplicationRuntimeError.rejected("Invalid frozen submission request")
            }
            payload = frozen
        } else { payload = makeHostAgentPayload(input: input) }
        let digest: String
        if let boundRequestDigest { digest = boundRequestDigest }
        else {
            let prepared = try await prepareStandaloneSubmission(payload)
            payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(prepared.request)) as! [String: Any]
            digest = prepared.digest
        }
        payload["request_digest"] = digest
        let encryptedPayload = try Self.encryptMobileJSON(payload, token: config.token)
        request.httpBody = try JSONSerialization.data(withJSONObject: encryptedPayload)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        Self.applyMobileAuth(to: &request, token: config.token)
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")

        if let streamLoader {
            let loaded = try await streamLoader(request)
            try await consumeEvents(
                bytes: loaded.bytes,
                response: loaded.response,
                continuation: continuation
            )
            return
        }

        let session = Self.makeURLSession(
            pinnedCertificateSHA256: config.pinnedCertificateSHA256
        )
        let (bytes, response) = try await session.bytes(for: request)
        try await consumeEvents(
            bytes: bytes,
            response: response,
            continuation: continuation
        )
    }

    private func consumeEvents<Bytes: AsyncSequence>(
        bytes: Bytes,
        response: URLResponse,
        continuation: AsyncThrowingStream<WebAgentStreamEvent, Error>.Continuation
    ) async throws where Bytes.Element == UInt8, Bytes.AsyncIterator: SendableMetatype {
        if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
            if http.statusCode == 409 { throw IOSRunObservationTransportError.http(409) }
            var body = Data()
            var iterator = bytes.makeAsyncIterator()
            while let byte = try await IOSHTTPByteIterator.next(&iterator) {
                body.append(byte)
            }
            try Self.validateHTTP(response: http, data: body)
        }

        var dataLines: [String] = []
        func flushFrame() throws -> Bool {
            guard let raw = Self.sseFramePayload(from: dataLines) else {
                dataLines.removeAll()
                return false
            }
            dataLines.removeAll()
            if raw == "[DONE]" {
                return true
            }
            guard let event = try Self.parseSSEPayload(raw, token: config.token) else {
                return false
            }
            continuation.yield(event)
            return false
        }

        func consumeLine(_ line: String) throws -> Bool {
            if Task.isCancelled {
                throw CancellationError()
            }
            if line.trimmingCharacters(in: .newlines).isEmpty {
                if try flushFrame() {
                    return true
                }
                return false
            }
            if let value = Self.sseDataLineValue(line) {
                dataLines.append(value)
            }
            return false
        }

        var lineFramer = WebAgentSSELineFramer()
        var reachedDone = false
        var iterator = bytes.makeAsyncIterator()
        while let byte = try await IOSHTTPByteIterator.next(&iterator) {
            if let line = lineFramer.append(byte) {
                if try consumeLine(line) {
                    reachedDone = true
                    break
                }
            }
        }
        if !reachedDone, let line = lineFramer.finish() {
            reachedDone = try consumeLine(line)
        }
        if !reachedDone, !dataLines.isEmpty {
            _ = try flushFrame()
        }
    }

    static func resolveToolURL(_ url: URL?, baseURL: URL) -> URL? {
        guard let url else { return nil }
        if url.scheme == nil {
            return URL(string: url.absoluteString, relativeTo: baseURL)?.absoluteURL
        }
        guard let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https" else {
            return nil
        }
        return rewriteLoopbackToolURL(url, baseURL: baseURL) ?? url
    }

    private static func rewriteLoopbackToolURL(_ url: URL, baseURL: URL) -> URL? {
        guard let host = url.host, isLoopbackHost(host),
              let replacementHost = baseURL.host,
              !replacementHost.isEmpty,
              !isLoopbackHost(replacementHost),
              var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        components.host = replacementHost
        if components.port == nil {
            components.port = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)?.port
        }
        return components.url
    }

    private static func isLoopbackHost(_ host: String) -> Bool {
        let normalized = host
            .trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()
        if normalized == "localhost" ||
            normalized == "localhost.localdomain" ||
            normalized.hasSuffix(".localhost") ||
            normalized == "::1" ||
            normalized == "0:0:0:0:0:0:0:1" {
            return true
        }
        let parts = normalized.split(separator: ".")
        guard parts.count == 4, parts.first == "127" else { return false }
        return parts.allSatisfy { part in
            guard let value = Int(part) else { return false }
            return (0...255).contains(value)
        }
    }

    private func hostModelSettings() async throws -> [String: Any] {
        guard MobileHostTransportPolicy.allows(config.baseURL) else {
            throw IOSApplicationRuntimeError.rejected("Host transport is not permitted")
        }
        var request = URLRequest(url: Self.mobileURL(baseURL: config.baseURL, path: "agent/settings"))
        request.httpMethod = "GET"
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        Self.applyMobileAuth(to: &request, token: config.token)
        let data: Data
        let response: URLResponse
        if let streamLoader {
            let loaded = try await streamLoader(request)
            var body = Data()
            for try await byte in loaded.bytes { body.append(byte) }
            data = body
            response = loaded.response
        } else {
            let session = Self.makeURLSession(pinnedCertificateSHA256: config.pinnedCertificateSHA256)
            defer { if session !== URLSession.shared { session.finishTasksAndInvalidate() } }
            (data, response) = try await session.data(for: request)
        }
        try Self.validateHTTP(response: response, data: data)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return object["type"] as? String == "encrypted"
            ? try Self.decryptMobileJSON(object, token: config.token) : object
    }

    /// Read the same Host contract through this owner's configured transport.
    /// Capability refresh and inference must not use different connections.
    func hostAgentSettings() async throws -> HostAgentSettings {
        do {
            return try JSONDecoder().decode(HostAgentSettings.self,
                from: JSONSerialization.data(withJSONObject: try await hostModelSettings()))
        } catch {
            throw Self.friendlyHostError(error, baseURL: config.baseURL)
        }
    }

    func prepareHostToolPlan(_ original: [String: Any], sourceRunID: String? = nil,
                             sourceAssistantMessageID: String? = nil) async throws -> [String: Any] {
        if let commandID = original["queued_user_command_id"] as? String {
            guard !commandID.isEmpty, commandID.utf8.count <= 128,
                  commandID.trimmingCharacters(in: .whitespacesAndNewlines) == commandID,
                  !commandID.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
                  original["submission_id"] as? String == commandID,
                  original["user_message_id"] as? String == "ui:\(commandID):user",
                  original["assistant_message_id"] as? String == "\(commandID):assistant",
                  (original["execution_backend"] as? String ?? "nola") == "nola",
                  (original["external_session_id"] as? String ?? "").isEmpty else {
                throw IOSApplicationRuntimeError.rejected("Invalid queued UI command identity")
            }
        }
        guard let host = config.expectedHostId, let sessionID = original["conversationId"] as? String,
              !sessionID.isEmpty, original["tool_plan_ref"] == nil else {
            throw IOSApplicationRuntimeError.rejected("Tool plan requires an unbound paired conversation")
        }
        guard (sourceRunID == nil) == (sourceAssistantMessageID == nil),
              original["expected_configuration_revision"] == nil else {
            throw IOSApplicationRuntimeError.rejected("Submission frontier is already frozen or its source witness is incomplete")
        }
        struct FrontierQuery: Encodable {
            let expected_host_id: String
            let client_id: String
            let conversation_id: String
            let expected_run_id: String?
            let assistant_message_id: String?
        }
        let frontier: AppleSubmissionFrontier = try await pairedReadRequest(FrontierQuery(
            expected_host_id: host, client_id: config.clientId, conversation_id: sessionID,
            expected_run_id: sourceRunID, assistant_message_id: sourceAssistantMessageID),
            path: "chat/agent/v2/submission-frontier")
        try frontier.validate(host: host, client: config.clientId, conversation: sessionID,
            observedParent: original["expected_parent_message_id"] as? String)
        // Only fresh requests enter this path. Bound/unknown submissions replay
        // their exact durable payload and never resolve a new default model.
        let frozenModel: AppleRemoteHostModelCatalog.DefaultIntent?
        if (original["execution_backend"] as? String ?? "nola") == "nola" {
            guard original["model_intent"] == nil else {
                throw IOSApplicationRuntimeError.rejected("Model intent is already frozen")
            }
            frozenModel = try AppleRemoteHostModelCatalog.freezeDefault(
                settings: await hostModelSettings(), expectedHostID: host)
        } else { frozenModel = nil }
        struct Query: Encodable { let expected_host_id: String; let client_id: String; let session_id: String }
        let plan: IOSRemoteHostToolPlan = try await pairedReadRequest(
            Query(expected_host_id: host, client_id: config.clientId, session_id: sessionID), path: "agent/tool-plan/prepare")
        var payload = original
        payload["expected_configuration_revision"] = frontier.configuration_revision.map { $0 as Any } ?? NSNull()
        payload["expected_parent_message_id"] = frontier.parent_message_id.map { $0 as Any } ?? NSNull()
        if let frozenModel {
            try AppleRemoteHostModelCatalog.validateUnchanged(frozenModel, settings: await hostModelSettings())
            // Prompt voice is independent of the model's reasoning tier.
            payload["model_intent"] = try frozenModel.payload(tier: "balanced")
        }
        payload["tool_plan_ref"] = try plan.validatedReference(hostID: host, clientID: config.clientId, sessionID: sessionID)
        return payload
    }

    public func makeHostAgentPayload(input: ModelInput) -> [String: Any] {
        var payload = Self.makeHostAgentPayload(input: input)
        let expectedHostId = config.expectedHostId
            ?? ModelSelectionManager.pairedWebHostIdentity(for: config.baseURL)
        if let expectedHostId { payload["expected_host_id"] = expectedHostId }
        payload["client_id"] = config.clientId
        if input.userPrefs["hybrid_device_tools_enabled"] == "true",
           let expectedHostId,
           let sessionId = payload["conversationId"] as? String,
           let bridge = Self.deviceBridgeAdvertisement(
                hostId: expectedHostId,
                clientId: config.clientId,
                sessionId: sessionId,
                allowedCapabilities: Set(
                    (input.userPrefs["hybrid_device_capabilities"] ?? "")
                        .split(separator: ",")
                        .map(String.init)
                ),
                userApprovedResultEgress: input.userPrefs["hybrid_device_result_egress_enabled"] == "true",
                isForeground: input.userPrefs["app_foreground"] != "false"
           ) {
            payload["device_bridge"] = bridge
        }
        return payload
    }

    private static func deviceBridgeAdvertisement(
        hostId: String,
        clientId: String,
        sessionId: String,
        allowedCapabilities: Set<String>,
        userApprovedResultEgress: Bool,
        isForeground: Bool
    ) -> [String: Any]? {
        let identity = AgentCoreExpectedIdentity(hostId: hostId, clientId: clientId, sessionId: sessionId)
        let resultEgressAuthorization = IOSAgentResultEgressAuthorization.explicitUserApproval(
            identity: identity,
            userApproved: userApprovedResultEgress
        )
        guard let plan = try? AgentCoreToolCatalog.prepareBoundPlan(
            for: .hybrid,
            expectedIdentity: identity,
            isForeground: isForeground,
            allowedCapabilities: allowedCapabilities,
            resultEgressAuthorization: resultEgressAuthorization
        ) else {
            return nil
        }
        let tools = plan.output.enabledTools.compactMap { tool -> [String: Any]? in
            guard tool.route == "device",
                  allowedCapabilities.contains(tool.capability),
                  let data = try? JSONEncoder().encode(tool.schema),
                  let schema = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                return nil
            }
            return [
                "schema": schema,
                "capability": tool.capability,
                "allow_result_egress": !tool.requiresResultEgress || plan.resultEgressGrants.contains(where: {
                    $0.toolId == tool.toolId && $0.route == tool.route
                }),
            ]
        }
        guard !tools.isEmpty else { return nil }
        return [
            "protocol_version": 1,
            "host_id": hostId,
            "client_id": clientId,
            "session_id": sessionId,
            "allow_result_egress": !plan.resultEgressGrants.isEmpty,
            "policy_digest": plan.output.policyDigest,
            "manifest_digest": plan.output.manifestDigest,
            "manifest_valid_until_ms": plan.output.manifestValidUntilMs,
            "tools": tools,
        ]
    }

    public static func makeHostAgentPayload(input: ModelInput) -> [String: Any] {
        let message = input.userPrefs["message"] ?? ""
        let allowLocalDevTools = input.userPrefs["host_allow_local_dev_tools"] != "false"
        let requestedMaxIterations = Int(input.userPrefs["web_agent_max_iterations"] ?? "")
        let maxIterations = max(0, requestedMaxIterations ?? defaultMaxIterations)
        let hostMessages: [[String: Any]] = {
            guard let raw = input.userPrefs["host_agent_messages"],
                  let data = raw.data(using: .utf8),
                  let parsed = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
                return [["role": "user", "content": message]]
            }
            let allowedRoles: Set<String> = ["user", "assistant"]
            let sanitized = parsed.compactMap { item -> [String: Any]? in
                let role = (item["role"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                let content = (item["content"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                guard allowedRoles.contains(role), !content.isEmpty else { return nil }
                return ["role": role, "content": content]
            }
            return sanitized.isEmpty ? [["role": "user", "content": message]] : sanitized
        }()
        // iOS acts as an input surface for the Web host. Endpoint/API key/model
        // remain owned by the host-side Web settings; mobile only sends prompt
        // and execution preferences.
        var payload: [String: Any] = [
            "messages": hostMessages,
            "allow_local_dev_tools": allowLocalDevTools,
            "agent_mode": true,
            "max_iterations": maxIterations,
            "timezone": TimeZone.current.identifier,
            "language": Locale.current.identifier,
        ]
        if let conversationId = input.userPrefs["host_conversation_id"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !conversationId.isEmpty {
            payload["conversationId"] = conversationId
        }
        for identityField in ["submission_id", "user_message_id", "assistant_message_id"] {
            if let value = input.userPrefs[identityField]?.trimmingCharacters(
                in: .whitespacesAndNewlines
            ), !value.isEmpty {
                payload[identityField] = value
            }
        }
        payload["expected_parent_message_id"] = input.userPrefs["expected_parent_message_id"].map { $0 as Any } ?? NSNull()
        if let commandID = input.userPrefs["queued_user_command_id"] {
            payload["queued_user_command_id"] = commandID
        }
        if let executionBackend = input.userPrefs["execution_backend"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !executionBackend.isEmpty {
            payload["execution_backend"] = executionBackend
        }
        if let externalSessionID = input.userPrefs["external_session_id"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !externalSessionID.isEmpty {
            payload["external_session_id"] = externalSessionID
        }
        if let permissionProfile = input.userPrefs["permission_profile"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           IOSPermissionMode(rawValue: permissionProfile) != nil {
            payload["permission_profile"] = permissionProfile
        }
        if let responseStyle = input.userPrefs["response_style"], !responseStyle.isEmpty {
            payload["response_style"] = responseStyle
        }
        if let customSystemPrompt = input.userPrefs["custom_system_prompt"], !customSystemPrompt.isEmpty {
            payload["custom_system_prompt"] = customSystemPrompt
        }
        if let nativeAppHostId = input.userPrefs["native_app_host_id"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !nativeAppHostId.isEmpty {
            payload["native_app_host_id"] = nativeAppHostId
        }
        return payload
    }

    public static func parseSSEObject(_ object: [String: Any]) -> WebAgentStreamEvent? {
        if object["type"] as? String == "submission_receipt", let receipt = object["receipt"] as? [String: Any],
           let value = coreJSONValue(from: receipt) { return .submissionReceipt(value) }
        if let type = object["type"] as? String,
           ["assistant_journal_committed_v1", "assistant_run_terminal_committed_v1"].contains(type) {
            guard let value = coreJSONValue(from: object) else {
                return .error("Invalid assistant journal notification")
            }
            return type == "assistant_journal_committed_v1"
                ? .assistantJournalCommitted(value) : .assistantRunTerminalSealed(value)
        }
        if object["type"] as? String == "external_session_started" {
            let backendID = (object["backend_id"] as? String ?? object["execution_backend"] as? String ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let sessionID = (object["session_id"] as? String ?? object["external_session_id"] as? String ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard !backendID.isEmpty, !sessionID.isEmpty else {
                return .error("Agent Host protocol violation: malformed external session event")
            }
            return .externalSessionStarted(backendID: backendID, sessionID: sessionID)
        }
        if let type = object["type"] as? String,
           ["turn_completed", "turn_failed", "turn_cancelled"].contains(type),
           object["protocol_version"] as? Int == 1,
           object["event"] == nil,
           let terminal = AgentCoreTurnTerminalEnvelope.decodeHost(from: object) {
            return .coreTurnTerminal(terminal, trace: AgentTraceAdapter.decodeNestedTrace(from: object))
        }
        if let type = object["type"] as? String {
            switch type {
            case "core_task_updated":
                guard let value = coreJSONValue(from: object),
                      let envelope = AgentCoreTaskEventEnvelope.decodeHost(from: value) else {
                    return .error("Agent Host protocol violation: malformed Core task projection")
                }
                return .coreTaskUpdated(envelope)
            case "core_skill_runtime_updated":
                guard let value = coreJSONValue(from: object),
                      let envelope = AgentCoreSkillRuntimeUpdateEnvelope.decode(from: value) else {
                    return .error("Agent Host protocol violation: malformed Core Skill projection")
                }
                return .coreSkillRuntimeUpdated(envelope)
            case "core_capabilities_activated":
                guard let value = coreJSONValue(from: object),
                      let envelope = AgentCoreCapabilityActivationEnvelope.decode(from: value) else {
                    return .error("Agent Host protocol violation: malformed Core capability projection")
                }
                return .coreCapabilitiesActivated(envelope)
            case "core_permission_projection_updated":
                guard let projection = object["permission_projection"],
                      JSONSerialization.isValidJSONObject(projection),
                      let data = try? JSONSerialization.data(withJSONObject: projection),
                      let value = try? JSONDecoder().decode(AgentCorePermissionProjection.self, from: data) else {
                    return .error("Agent Host protocol violation: malformed Core permission projection")
                }
                return .corePermissionProjectionUpdated(value)
            case "core_memory_runtime_projection_updated":
                guard let projection = object["memory_runtime_projection"],
                      let value = coreJSONValue(from: projection) else {
                    return .error("Agent Host protocol violation: malformed Core memory runtime projection")
                }
                return .coreMemoryRuntimeProjectionUpdated(value)
            case "context_usage":
                guard object["protocol_version"] as? Int == 1,
                      let data = try? JSONSerialization.data(withJSONObject: object),
                      let projection = try? JSONDecoder().decode(
                        HostAgentContextUsageProjection.self,
                        from: data
                      ) else {
                    return .error("Agent Host protocol violation: malformed context usage projection")
                }
                return .contextUsageUpdated(projection)
            case "error":
                let message = (object["message"] as? String)?.trimmingCharacters(
                    in: .whitespacesAndNewlines
                )
                if let message, !message.isEmpty {
                    return .error(message)
                }
                return .error("Agent Host returned an unspecified transport error")
            default:
                break
            }
        }
        // Delegated approval is an owner-bound Host challenge, not a parent
        // Core trace. Never manufacture a trace for the child's operation.
        if object["type"] as? String == "permission_request",
           let task = object["task_id"] as? String, !task.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            guard object["protocol_version"] as? Int == hostPermissionProtocolVersion,
                  let request = permissionRequest(from: object) else {
                return .error("Invalid delegated permission challenge")
            }
            return .delegatedPermissionRequest(request)
        }
        guard let trace = AgentTraceAdapter.decodeNestedTrace(from: object) else {
            return .error("Agent Host protocol violation: event is missing Core Trace")
        }
        guard let type = object["type"] as? String else { return .trace(trace) }
        switch type {
        case "permission_request":
            guard let version = object["protocol_version"] as? Int,
                  version == hostPermissionProtocolVersion else {
                return .error("Unsupported host permission protocol version")
            }
            guard let request = permissionRequest(from: object) else {
                return .error("Invalid permission instruction metadata")
            }
            return .permissionRequest(request, trace: trace)
        case "device_tool_request":
            guard let request = deviceToolRequest(from: object) else { return nil }
            return .deviceToolRequest(request, trace: trace)
        default:
            return .trace(trace)
        }
    }

    private static func coreJSONValue(from object: [String: Any]) -> AgentCoreJSONValue? {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return try? JSONDecoder().decode(AgentCoreJSONValue.self, from: data)
    }

    private static func coreJSONValue(from object: Any) -> AgentCoreJSONValue? {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return try? JSONDecoder().decode(AgentCoreJSONValue.self, from: data)
    }

    nonisolated static func sseDataLineValue(_ line: String) -> String? {
        let normalized = line.hasSuffix("\r") ? String(line.dropLast()) : line
        guard normalized.hasPrefix("data:") else { return nil }
        var value = String(normalized.dropFirst(5))
        if value.first == " " {
            value.removeFirst()
        }
        return value
    }

    nonisolated static func sseFramePayload(from dataLines: [String]) -> String? {
        guard !dataLines.isEmpty else { return nil }
        let raw = dataLines
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return raw.isEmpty ? nil : raw
    }

    static func parseSSEPayload(_ raw: String, token: String) throws -> WebAgentStreamEvent? {
        guard raw != "[DONE]",
              let data = raw.data(using: .utf8),
              let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        let object: [String: Any]
        if parsed["type"] as? String == "encrypted" {
            object = try decryptMobileJSON(parsed, token: token)
        } else {
            object = parsed
        }
        return parseSSEObject(object)
    }

    static func isNonFatalAgentErrorMessage(_ message: String) -> Bool {
        let lowered = message.lowercased()
        return lowered.contains("reached max iterations") || lowered.contains("max iterations")
    }

    static func permissionRequest(from object: [String: Any]) -> HostAgentPermissionRequest? {
        guard let allowConversation = object["allow_conversation"] as? Bool else { return nil }
        guard let instructions = AgentPermissionInstruction.decode(object["permission_instructions"]) else { return nil }
        let id = object["id"] as? String ?? UUID().uuidString
        let toolName = (object["tool_name"] as? String) ?? (object["name"] as? String) ?? "tool"
        let argsPreview: String = {
            if let review = object["safe_review_text"] as? String { return review }
            guard let args = object["args"] else { return "" }
            guard JSONSerialization.isValidJSONObject(args),
                  let data = try? JSONSerialization.data(withJSONObject: args, options: [.sortedKeys]),
                  let text = String(data: data, encoding: .utf8) else {
                return String(describing: args)
            }
            return text
        }()
        return HostAgentPermissionRequest(
            id: id,
            allowConversation: allowConversation,
            toolName: toolName,
            argsPreview: argsPreview,
            match: object["match"] as? String ?? "*",
            reason: object["reason"] as? String ?? "",
            message: object["message"] as? String ?? "",
            nonce: object["nonce"] as? String,
            hostId: object["host_id"] as? String,
            clientId: object["client_id"] as? String,
            sessionId: object["session_id"] as? String,
            canonicalRunID: object["canonical_run_id"] as? String,
            expiresAt: (object["expires_at"] as? NSNumber)?.int64Value,
            protocolVersion: (object["protocol_version"] as? NSNumber)?.intValue,
            permissionInstructions: instructions
        )
    }

    private static func deviceToolRequest(from object: [String: Any]) -> HostDeviceToolRequest? {
        guard let id = object["id"] as? String,
              let nonce = object["nonce"] as? String,
              let hostId = object["host_id"] as? String,
              let clientId = object["client_id"] as? String,
              let sessionId = object["session_id"] as? String,
              let toolName = object["tool_name"] as? String,
              let expiresAt = (object["expires_at"] as? NSNumber)?.int64Value,
              let protocolVersion = (object["protocol_version"] as? NSNumber)?.intValue,
              let rawArguments = object["arguments"],
              JSONSerialization.isValidJSONObject(rawArguments),
              let data = try? JSONSerialization.data(withJSONObject: rawArguments),
              let arguments = try? JSONDecoder().decode(AgentCoreJSONValue.self, from: data) else {
            return nil
        }
        let argsPreview = (object["args"] as? [String: Any]).flatMap { value -> String? in
            guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else { return nil }
            return String(data: data, encoding: .utf8)
        } ?? ""
        return HostDeviceToolRequest(
            id: id,
            nonce: nonce,
            hostId: hostId,
            clientId: clientId,
            sessionId: sessionId,
            toolName: toolName,
            capability: (object["capability"] as? String).flatMap(AgentCoreCapability.init(rawValue:)),
            policyDigest: object["policy_digest"] as? String,
            manifestDigest: object["manifest_digest"] as? String,
            manifestValidUntilMs: (object["manifest_valid_until_ms"] as? NSNumber)?.int64Value,
            arguments: arguments,
            argsPreview: argsPreview,
            expiresAt: expiresAt,
            protocolVersion: protocolVersion
        )
    }

    private static func hasUnsafeURLCharacters(_ value: String) -> Bool {
        value.unicodeScalars.contains { scalar in
            scalar.value <= 0x20 || scalar.value == 0x7F
        }
    }

    private static func boolish(_ value: Any?) -> Bool? {
        if let bool = value as? Bool {
            return bool
        }
        if let number = value as? NSNumber {
            if number.doubleValue == 0 {
                return false
            }
            if number.doubleValue == 1 {
                return true
            }
            return nil
        }
        if let string = value as? String {
            switch string.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "true", "1", "yes", "y", "on":
                return true
            case "false", "0", "no", "n", "off":
                return false
            default:
                return nil
            }
        }
        return nil
    }

    public static func mobileURL(baseURL: URL, path: String) -> URL {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        var parts = (components?.path ?? baseURL.path)
            .split(separator: "/")
            .map(String.init)

        if let apiIndex = parts.lastIndex(of: "api"),
           apiIndex + 1 < parts.count,
           parts[apiIndex + 1] == "mobile" {
            parts = Array(parts.prefix(apiIndex))
        } else if parts.last == "api" {
            parts.removeLast()
        }

        components?.path = parts.isEmpty ? "" : "/" + parts.joined(separator: "/")
        let root = components?.url ?? baseURL
        var url = root
            .appendingPathComponent("api")
            .appendingPathComponent("mobile")
        for part in path.split(separator: "/") {
            url.appendPathComponent(String(part))
        }
        return url
    }

    public static func controlPlaneURL(baseURL: URL, path: String) -> URL? {
        guard var origin = URLComponents(url: baseURL, resolvingAgainstBaseURL: false),
              origin.scheme != nil,
              origin.host != nil else { return nil }
        if let absolute = URL(string: path),
           let scheme = absolute.scheme?.lowercased(),
           scheme == "http" || scheme == "https" {
            return absolute
        }
        let requested = URLComponents(string: path)
        origin.path = "/" + (requested?.path ?? path)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        origin.query = requested?.query
        origin.fragment = requested?.fragment
        return origin.url
    }

    nonisolated static func encryptMobileJSON(_ payload: [String: Any], token: String) throws -> [String: Any] {
        let plaintext = try JSONSerialization.data(withJSONObject: payload)
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(token.utf8))))
        let nonce = AES.GCM.Nonce()
        let sealed = try AES.GCM.seal(plaintext, using: key, nonce: nonce, authenticating: mobileCryptoAAD)
        return [
            "v": 1,
            "alg": "AES-256-GCM",
            "nonce": base64URLEncode(Data(nonce)),
            "ciphertext": base64URLEncode(sealed.ciphertext),
            "tag": base64URLEncode(sealed.tag),
        ]
    }

    nonisolated static func decryptMobileJSON(_ envelope: [String: Any], token: String) throws -> [String: Any] {
        guard (envelope["v"] as? Int) == 1,
              (envelope["alg"] as? String) == "AES-256-GCM",
              let nonceText = envelope["nonce"] as? String,
              let ciphertextText = envelope["ciphertext"] as? String,
              let tagText = envelope["tag"] as? String else {
            throw NolaError.llm(.invalidResponse(reason: "Invalid encrypted Web bridge event"))
        }
        let nonceData = try base64URLDecode(nonceText)
        let ciphertext = try base64URLDecode(ciphertextText)
        let tag = try base64URLDecode(tagText)
        let nonce = try AES.GCM.Nonce(data: nonceData)
        let sealed = try AES.GCM.SealedBox(nonce: nonce, ciphertext: ciphertext, tag: tag)
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(token.utf8))))
        let plaintext = try AES.GCM.open(sealed, using: key, authenticating: mobileCryptoAAD)
        guard let object = try JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
            throw NolaError.llm(.invalidResponse(reason: "Invalid encrypted Web bridge JSON"))
        }
        return object
    }

    nonisolated private static let mobileCryptoAAD = Data("nola-mobile-bridge-v1".utf8)

    static func applyMobileAuth(to request: inout URLRequest, token: String) {
        guard let url = request.url else { return }
        let timestamp = UInt64(Date().timeIntervalSince1970)
        let nonce = UUID().uuidString.lowercased()
        let method = (request.httpMethod ?? "GET").uppercased()
        let path = url.path + (url.query.map { "?\($0)" } ?? "")
        let bodyDigest = SHA256.hash(data: request.httpBody ?? Data())
            .map { String(format: "%02x", $0) }
            .joined()
        let message = "v1\n\(method)\n\(path)\n\(timestamp)\n\(nonce)\n\(bodyDigest)"
        let key = SymmetricKey(data: Data(token.utf8))
        let signature = HMAC<SHA256>.authenticationCode(for: Data(message.utf8), using: key)
        request.setValue(nil, forHTTPHeaderField: "Authorization")
        request.setValue(String(timestamp), forHTTPHeaderField: "X-Nola-Mobile-Timestamp")
        request.setValue(nonce, forHTTPHeaderField: "X-Nola-Mobile-Nonce")
        request.setValue(bodyDigest, forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256")
        request.setValue(base64URLEncode(Data(signature)), forHTTPHeaderField: "X-Nola-Mobile-Auth")
    }

    nonisolated private static func base64URLEncode(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    nonisolated private static func base64URLDecode(_ text: String) throws -> Data {
        var normalized = text
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        normalized += String(repeating: "=", count: (4 - normalized.count % 4) % 4)
        guard let data = Data(base64Encoded: normalized) else {
            throw NolaError.llm(.invalidResponse(reason: "Invalid encrypted Web bridge base64"))
        }
        return data
    }

    private static func makeURLSession(pinnedCertificateSHA256: String?) -> URLSession {
        guard let pinnedCertificateSHA256 else { return URLSession(configuration: NolaTailcatTransport.sessionConfiguration()) }
        let delegate = PinnedCertificateDelegate(expectedSHA256: pinnedCertificateSHA256)
        return URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
    }

    public static func fetchHostResource(from url: URL, pinnedCertificateSHA256: String? = nil) async throws -> Data {
        var request = URLRequest(url: url)
        request.timeoutInterval = 20
        request.setValue("*/*", forHTTPHeaderField: "Accept")

        let session = makeURLSession(pinnedCertificateSHA256: Config.normalizedFingerprint(pinnedCertificateSHA256))
        defer {
            if session !== URLSession.shared {
                session.finishTasksAndInvalidate()
            }
        }

        let (data, response) = try await session.data(for: request)
        try validateHTTP(response: response, data: data)
        return data
    }

    private final class PinnedCertificateDelegate: NSObject, URLSessionDelegate {
        let expectedSHA256: String

        init(expectedSHA256: String) {
            self.expectedSHA256 = expectedSHA256
        }

        func urlSession(
            _ session: URLSession,
            didReceive challenge: URLAuthenticationChallenge,
            completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
        ) {
            guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
                  let trust = challenge.protectionSpace.serverTrust,
                  let certificate = (SecTrustCopyCertificateChain(trust) as? [SecCertificate])?.first else {
                completionHandler(.performDefaultHandling, nil)
                return
            }
            let data = SecCertificateCopyData(certificate) as Data
            let actual = Data(SHA256.hash(data: data)).map { String(format: "%02X", $0) }.joined()
            if actual == expectedSHA256 {
                completionHandler(.useCredential, URLCredential(trust: trust))
            } else {
                completionHandler(.cancelAuthenticationChallenge, nil)
            }
        }
    }

    private static func validateHTTP(response: URLResponse, data: Data) throws {
        guard let http = response as? HTTPURLResponse else {
            throw NolaError.llm(.network(URLError(.badServerResponse)))
        }
        guard (200...299).contains(http.statusCode) else {
            let message = extractErrorMessage(from: data) ?? "HTTP \(http.statusCode)"
            if http.statusCode == 401 || http.statusCode == 403 {
                let format = NSLocalizedString(
                    "chat.web_host.auth_failed",
                    value: "Host authentication failed: %@",
                    comment: "Host authentication failure message"
                )
                throw NolaError.llm(.invalidInput(reason: String(format: format, message)))
            }
            throw NolaError.llm(.invalidResponse(reason: message))
        }
    }

    private static func extractErrorMessage(from data: Data) -> String? {
        guard !data.isEmpty else { return nil }
        if let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            if let error = object["error"] as? String { return error }
            if let detail = object["detail"] as? String { return detail }
        }
        return String(data: data.prefix(500), encoding: .utf8)
    }

    private static func friendlyHostError(_ error: Error, baseURL: URL) -> Error {
        let host = baseURL.host ?? ""
        if let urlError = error as? URLError {
            switch urlError.code {
            case .cannotConnectToHost, .cannotFindHost, .notConnectedToInternet, .networkConnectionLost:
                let target = host.isEmpty ? baseURL.absoluteString : host
                let format = NSLocalizedString(
                    "chat.web_host.connection_failed",
                    value: "Cannot connect to host %@. Open Nola on your Mac, enable device connections, and confirm both devices are on the same network.",
                    comment: "Host connection failure troubleshooting message"
                )
                return NolaError.llm(.network(NSError(
                    domain: "WebAgentProvider",
                    code: urlError.errorCode,
                    userInfo: [
                        NSLocalizedDescriptionKey: String(format: format, target)
                    ]
                )))
            case .timedOut:
                return NolaError.llm(.timeout)
            default:
                return error
            }
        }
        return error
    }
}
