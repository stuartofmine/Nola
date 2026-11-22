// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Discovers local CLI skills/configuration and merges optional connected-host extensions.

import Foundation
import CryptoKit

nonisolated private let macExtensionMaximumTrustBytes = 2 * 1_024 * 1_024

nonisolated private func macExtensionContentDigest(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

nonisolated private func macExtensionBoundedData(at url: URL) -> Data? {
    guard FileManager.default.fileExists(atPath: url.path),
          let handle = try? FileHandle(forReadingFrom: url) else { return nil }
    defer { try? handle.close() }
    guard let data = try? handle.read(upToCount: macExtensionMaximumTrustBytes + 1),
          data.count <= macExtensionMaximumTrustBytes else { return nil }
    return data
}

nonisolated private func macExtensionStrippingLegacyDigest(_ itemID: String) -> String {
    guard let marker = itemID.range(of: "#sha256=", options: .backwards) else { return itemID }
    return String(itemID[..<marker.lowerBound])
}

nonisolated private func macExtensionTrustedItemID(_ itemID: String) -> Bool {
    let base = macExtensionStrippingLegacyDigest(itemID)
    return base.contains(":skill:") || base.contains(":mcp:") || base.hasPrefix("managed-mcp:")
}

nonisolated struct MacExtensionItem: Identifiable, Hashable, Sendable {
    enum Kind: String, CaseIterable, Sendable {
        case skill
        case agent
        case command
        case mcp
        case plugin
        case document
        case capability
        case configuration

        var title: String {
            switch self {
            case .skill: return String(localized: "Skills")
            case .agent: return String(localized: "Agents")
            case .command: return String(localized: "Commands")
            case .mcp: return String(localized: "MCP Servers")
            case .plugin: return String(localized: "Plugins")
            case .document: return String(localized: "Documents")
            case .capability: return String(localized: "Host Capabilities")
            case .configuration: return String(localized: "Configuration")
            }
        }

        var symbol: String {
            switch self {
            case .skill: return "shippingbox"
            case .agent: return "person.crop.circle.badge.gearshape"
            case .command: return "terminal"
            case .mcp: return "server.rack"
            case .plugin: return "puzzlepiece.extension"
            case .document: return "doc.text"
            case .capability: return "switch.2"
            case .configuration: return "gearshape.2"
            }
        }
    }

    enum Source: String, Sendable {
        case builtIn
        case workspace
        case global
        case connectedHost

        var title: String {
            switch self {
            case .builtIn: return String(localized: "Built-in")
            case .workspace: return String(localized: "Workspace")
            case .global: return String(localized: "This Mac")
            case .connectedHost: return String(localized: "Connected Host")
            }
        }
    }

    let id: String
    let kind: Kind
    let source: Source
    let title: String
    let detail: String
    let path: String?
    let isEnabled: Bool
    let mcpSummary: MacMCPCapabilitySummary?

    init(
        id: String,
        kind: Kind,
        source: Source,
        title: String,
        detail: String,
        path: String?,
        isEnabled: Bool,
        mcpSummary: MacMCPCapabilitySummary? = nil
    ) {
        self.id = id
        self.kind = kind
        self.source = source
        self.title = title
        self.detail = detail
        self.path = path
        self.isEnabled = isEnabled
        self.mcpSummary = mcpSummary
    }

    var installedSkillID: String? {
        let prefix = "managed-skill:"
        guard kind == .skill, source == .global, id.hasPrefix(prefix) else { return nil }
        return String(id.dropFirst(prefix.count))
    }

    var isLocallyEditable: Bool {
        (path != nil || installedSkillID != nil || id.hasPrefix("managed-mcp:")) && source != .connectedHost
    }
}

nonisolated struct MacMCPCapabilitySummary: Hashable, Sendable {
    enum State: String, Hashable, Sendable {
        case configured
        case connected
        case authorizationRequired
        case unavailable
    }

    let state: State
    let protocolVersion: String?
    let tools: [String]
    let resources: [String]
    let resourceTemplates: [String]
    let prompts: [String]

    static let configured = MacMCPCapabilitySummary(
        state: .configured,
        protocolVersion: nil,
        tools: [],
        resources: [],
        resourceTemplates: [],
        prompts: []
    )

    var capabilityCount: Int {
        tools.count + resources.count + resourceTemplates.count + prompts.count
    }
}

nonisolated struct MacExtensionDocument: Equatable, Sendable {
    enum Format: String, Sendable {
        case markdown = "Markdown"
        case json = "JSON"
        case text = "Text"
    }

    let content: String
    let url: URL
    let format: Format
    let isEditable: Bool
    let canonicalSkillRevision: Int64?

    init(content: String, url: URL, format: Format, isEditable: Bool, canonicalSkillRevision: Int64? = nil) {
        self.content = content
        self.url = url
        self.format = format
        self.isEditable = isEditable
        self.canonicalSkillRevision = canonicalSkillRevision
    }
}

nonisolated enum MacExtensionsServiceError: LocalizedError {
    case contentUnavailable
    case contentTooLarge
    case invalidText
    case readOnly
    case invalidJSON
    case emptySkill
    case invalidDraftConfiguration(String)

    var errorDescription: String? {
        switch self {
        case .contentUnavailable:
            return String(localized: "Content is unavailable for this extension.")
        case .contentTooLarge:
            return String(localized: "This file is too large to edit here.")
        case .invalidText:
            return String(localized: "The file is not valid UTF-8 text.")
        case .readOnly:
            return String(localized: "This content is read only.")
        case .invalidJSON:
            return String(localized: "The MCP configuration is not valid JSON.")
        case .emptySkill:
            return String(localized: "A Skill cannot be empty.")
        case .invalidDraftConfiguration(let detail):
            return detail
        }
    }
}

/// Persists explicit activation for discovered workspace and global Skills.
/// Discovery is not authorization: a SKILL.md found on disk stays inactive
/// until the user enables that exact resource in the Extensions center.
nonisolated enum MacLocalSkillActivationStore {
    private static let skippedDirectories: Set<String> = [
        ".git", ".build", ".swiftpm", "node_modules", "DerivedData", "build", "dist", "target", "vendor",
    ]

    static func itemID(source: MacExtensionItem.Source, instructionURL: URL) -> String {
        let url = instructionURL.standardizedFileURL
        return "\(source.rawValue):skill:\(url.path)"
    }

    static func isEnabled(
        itemID: String,
        store: MacApplicationPort? = .shared
    ) -> Bool {
        guard macExtensionTrustedItemID(itemID) else { return false }
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        if let record = try? store?.listSkills().first(where: { $0.id == normalized || $0.id == itemID }) {
            return record.enabled
        }
        return false
    }

    static func setEnabled(
        _ enabled: Bool,
        itemID: String,
        store: MacApplicationPort? = .shared
    ) throws {
        guard macExtensionTrustedItemID(itemID), let candidate = candidate(for: itemID) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        guard let store else { throw CocoaError(.fileReadUnknown) }
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        if let current = try store.skillEditor(normalized) ?? store.skillEditor(itemID) {
            _ = try store.setSkillEnabled(current, enabled: enabled,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        } else {
            _ = try store.createUserSkill(candidate.input(enabled: enabled))
        }
    }

    static func instructionURLs(root: URL, limit: Int) -> [URL] {
        guard FileManager.default.fileExists(atPath: root.path),
              let enumerator = FileManager.default.enumerator(
                at: root,
                includingPropertiesForKeys: [.isDirectoryKey, .isRegularFileKey],
                options: [.skipsPackageDescendants]
              ) else { return [] }
        var urls: [URL] = []
        var scanned = 0
        while let url = enumerator.nextObject() as? URL, scanned < 20_000, urls.count < limit {
            scanned += 1
            if (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
                if skippedDirectories.contains(url.lastPathComponent) {
                    enumerator.skipDescendants()
                }
                continue
            }
            guard url.lastPathComponent == "SKILL.md" else { continue }
            urls.append(url.standardizedFileURL)
        }
        return urls
    }

    static func activeSkillDirectories(
        workspaceURL: URL,
        globalSkillsURL: URL,
        store: MacApplicationPort? = .shared
    ) -> [URL] {
        let candidates = instructionURLs(root: workspaceURL, limit: 120).map {
            (MacExtensionItem.Source.workspace, $0)
        } + instructionURLs(root: globalSkillsURL, limit: 80).map {
            (MacExtensionItem.Source.global, $0)
        }
        var seen = Set<String>()
        let canonical = Dictionary(
            uniqueKeysWithValues: ((try? store?.listSkills()) ?? []).map { ($0.id, $0) }
        )
        return candidates.compactMap { source, instructionURL in
            let id = itemID(source: source, instructionURL: instructionURL)
            let enabled = canonical[id]?.enabled
                ?? isEnabled(itemID: id, store: store)
            guard enabled else { return nil }
            let directory = instructionURL.deletingLastPathComponent().standardizedFileURL
            return seen.insert(directory.path).inserted ? directory : nil
        }
    }

    private struct Candidate {
        let id: String
        let source: MacExtensionItem.Source
        let url: URL
        let digest: String
        let text: String

        func input(enabled: Bool) -> MacCanonicalSkillInput {
            let now = Int64(Date().timeIntervalSince1970 * 1_000)
            return MacCanonicalSkillInput(
                id: id,
                source: .user,
                version: "content-\(digest.prefix(16))",
                displayName: url.deletingLastPathComponent().lastPathComponent,
                summary: "",
                category: "custom",
                instructionsRef: url.absoluteString,
                instructionsText: text,
                requiredTools: [],
                defaultMode: .inline,
                enabled: enabled,
                createdAtMs: now,
                updatedAtMs: now
            )
        }
    }

    private static func candidate(for itemID: String) -> Candidate? {
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        guard let marker = normalized.range(of: ":skill:"),
              let source = MacExtensionItem.Source(rawValue: String(normalized[..<marker.lowerBound]))
        else { return nil }
        let path = String(normalized[marker.upperBound...])
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard let data = macExtensionBoundedData(at: url),
              let text = String(data: data, encoding: .utf8) else { return nil }
        let digest = macExtensionContentDigest(data)
        return Candidate(id: normalized, source: source, url: url, digest: digest, text: text)
    }

}

/// Explicit activation and request-time snapshotting for local MCP configs.
/// Finding a config file is discovery only; no server is connected or exposed
/// to the model until the user enables that exact file/server pair.
nonisolated enum MacLocalMCPActivationStore {
    static func serverID(for itemID: String) -> String {
        "local-" + macExtensionContentDigest(Data(itemID.utf8))
    }
    static let configurationPaths = [
        ".mcp.json", ".claude/settings.json", ".claude/settings.local.json",
        ".claude/settings.jsonc", ".claude.json", "opencode.json", "opencode.jsonc",
        ".opencode.json", ".opencode.jsonc", ".opencode/opencode.json",
        ".opencode/opencode.jsonc",
    ]

    static func itemID(
        source: MacExtensionItem.Source,
        configurationURL: URL,
        serverName: String
    ) -> String {
        let url = configurationURL.standardizedFileURL
        return "\(source.rawValue):mcp:\(url.path):\(serverName)"
    }

    static func isEnabled(
        itemID: String,
        store: MacApplicationPort? = .shared
    ) -> Bool {
        guard macExtensionTrustedItemID(itemID) else { return false }
        if itemID.hasPrefix("managed-mcp:") {
            let serverID = String(itemID.dropFirst("managed-mcp:".count))
            if let record = (try? store?.listMCPServers())?.first(where: { $0.id == serverID && $0.deletedAtMs == nil }) {
                return record.enabled
            }
            return false
        }
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        if let record = try? store?.listMCPServers().first(where: {
            $0.deletedAtMs == nil && macExtensionStrippingLegacyDigest($0.id) == normalized
        }) {
            return record.enabled
        }
        return false
    }

    static func setEnabled(
        _ enabled: Bool,
        itemID: String,
        store: MacApplicationPort? = .shared
    ) throws {
        guard macExtensionTrustedItemID(itemID) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        guard let store else { throw CocoaError(.fileReadUnknown) }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        if itemID.hasPrefix("managed-mcp:") {
            let serverID = String(itemID.dropFirst("managed-mcp:".count))
            guard let current = try store.listMCPServers().first(where: { $0.id == serverID && $0.deletedAtMs == nil }) else {
                throw CocoaError(.fileNoSuchFile)
            }
            _ = try store.configureMCPServer(
                current, displayName: current.displayName, enabled: enabled,
                configurationJSON: current.configurationJSON,
                updatedAtMs: now
            )
            return
        }
        guard let candidate = candidate(for: itemID) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        let next = candidate.input(enabled: enabled)
        if let current = try store.listMCPServers().first(where: {
            $0.deletedAtMs == nil && macExtensionStrippingLegacyDigest($0.id) == normalized
        }) {
            _ = try store.configureMCPServer(
                current, displayName: next.displayName, enabled: enabled,
                configurationJSON: enabled ? next.configurationJSON : current.configurationJSON,
                updatedAtMs: now
            )
        } else {
            _ = try store.createMCPServer(next)
        }
    }

    static func serverNames(in url: URL) -> [String] {
        serverConfigurations(in: url).keys.sorted()
    }

    /// Returns a secret-bearing JSON snapshot for the authenticated local
    /// agent-host stdin request. It must never enter Core state or chat text.
    static func activeServerConfigurationData(
        workspaceURL: URL,
        store: MacApplicationPort? = .shared
    ) -> Data? {
        let roots: [(MacExtensionItem.Source, URL)] = [
            (.workspace, workspaceURL.standardizedFileURL),
            (.global, FileManager.default.homeDirectoryForCurrentUser.standardizedFileURL),
        ]
        var active: [[String: Any]] = []
        var seen = Set<String>()
        let records = (try? store?.listMCPServers())?.filter { $0.deletedAtMs == nil } ?? []
        let canonical = Dictionary(
            records.map { (macExtensionStrippingLegacyDigest($0.id), $0) },
            uniquingKeysWith: { _, latest in latest }
        )
        for (source, root) in roots {
            for relativePath in configurationPaths {
                let url = root.appendingPathComponent(relativePath).standardizedFileURL
                for (name, _) in serverConfigurations(in: url).sorted(by: { $0.key < $1.key }) {
                    let id = itemID(source: source, configurationURL: url, serverName: name)
                    guard let approvedRecord = canonical[id],
                          approvedRecord.enabled,
                          seen.insert(id).inserted,
                          let approvedData = approvedRecord.configurationJSON.data(using: .utf8),
                          var configuration = (try? JSONSerialization.jsonObject(with: approvedData)) as? [String: Any] else {
                        continue
                    }
                    configuration["name"] = name
                    configuration["server_id"] = serverID(for: id)
                    configuration["enabled"] = true
                    if configuration["cwd"] == nil {
                        configuration["cwd"] = root.path
                    }
                    if configuration["endpoint"] == nil, let url = configuration["url"] as? String {
                        configuration["endpoint"] = url
                    }
                    active.append(configuration)
                }
            }
        }
        for record in records.sorted(by: { $0.id < $1.id }) where record.enabled && !record.id.contains(":mcp:") {
            guard seen.insert("managed-mcp:\(record.id)").inserted,
                  let data = record.configurationJSON.data(using: .utf8),
                  var configuration = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
                continue
            }
            configuration["name"] = record.displayName
            if (configuration["server_id"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false {
                configuration["server_id"] = record.id
            }
            configuration["enabled"] = true
            if configuration["endpoint"] == nil, let url = configuration["url"] as? String {
                configuration["endpoint"] = url
            }
            active.append(configuration)
        }
        guard !active.isEmpty else { return nil }
        return try? JSONSerialization.data(withJSONObject: active, options: [.sortedKeys])
    }

    private struct Candidate {
        let id: String
        let name: String
        let source: MacExtensionItem.Source
        let configurationURL: URL
        let configuration: [String: Any]

        func input(enabled: Bool) -> MacCanonicalMCPServerInput {
            let now = Int64(Date().timeIntervalSince1970 * 1_000)
            var payload = configuration
            payload["name"] = name
            payload["server_id"] = MacLocalMCPActivationStore.serverID(for: id)
            payload["source"] = source.rawValue
            payload["configuration_path"] = configurationURL.path
            let data = (try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys]))
                ?? Data("{}".utf8)
            return MacCanonicalMCPServerInput(
                id: id,
                displayName: name,
                enabled: enabled,
                configurationJSON: String(decoding: data, as: UTF8.self),
                revision: 0,
                createdAtMs: now,
                updatedAtMs: now,
                deletedAtMs: nil
            )
        }
    }

    private static func candidate(for itemID: String) -> Candidate? {
        let normalized = macExtensionStrippingLegacyDigest(itemID)
        guard let marker = normalized.range(of: ":mcp:"),
              let source = MacExtensionItem.Source(rawValue: String(normalized[..<marker.lowerBound]))
        else { return nil }
        let identity = String(normalized[marker.upperBound...])
        guard let separator = identity.range(of: ":", options: .backwards) else { return nil }
        let url = URL(fileURLWithPath: String(identity[..<separator.lowerBound])).standardizedFileURL
        let name = String(identity[separator.upperBound...])
        guard let configuration = serverConfigurations(in: url)[name] else { return nil }
        return Candidate(
            id: normalized,
            name: name,
            source: source,
            configurationURL: url,
            configuration: configuration
        )
    }

    private static func serverConfigurations(in url: URL) -> [String: [String: Any]] {
        guard let data = boundedData(at: url),
              let normalized = normalizeJSONC(data),
              let root = try? JSONSerialization.jsonObject(with: normalized) as? [String: Any],
              let servers = (root["mcpServers"] ?? root["mcp"]) as? [String: Any] else {
            return [:]
        }
        return servers.reduce(into: [:]) { result, entry in
            if let configuration = entry.value as? [String: Any] {
                result[entry.key] = configuration
            }
        }
    }

    private static func boundedData(at url: URL) -> Data? {
        macExtensionBoundedData(at: url)
    }

    private static func normalizeJSONC(_ data: Data) -> Data? {
        guard let source = String(data: data, encoding: .utf8) else { return nil }
        let scalars = Array(source.unicodeScalars)
        var output: [UnicodeScalar] = []
        var index = 0
        var inString = false
        var escaped = false
        var lineComment = false
        var blockComment = false
        while index < scalars.count {
            let scalar = scalars[index]
            let next = index + 1 < scalars.count ? scalars[index + 1] : nil
            if lineComment {
                if scalar == "\n" { lineComment = false; output.append(scalar) }
            } else if blockComment {
                if scalar == "*", next == "/" { blockComment = false; index += 1 }
            } else if inString {
                output.append(scalar)
                if escaped { escaped = false }
                else if scalar == "\\" { escaped = true }
                else if scalar == "\"" { inString = false }
            } else if scalar == "\"" {
                inString = true
                output.append(scalar)
            } else if scalar == "/", next == "/" {
                lineComment = true
                index += 1
            } else if scalar == "/", next == "*" {
                blockComment = true
                index += 1
            } else if scalar == "," {
                var lookahead = index + 1
                while lookahead < scalars.count,
                      CharacterSet.whitespacesAndNewlines.contains(scalars[lookahead]) {
                    lookahead += 1
                }
                if lookahead >= scalars.count || (scalars[lookahead] != "}" && scalars[lookahead] != "]") {
                    output.append(scalar)
                }
            } else {
                output.append(scalar)
            }
            index += 1
        }
        return String(String.UnicodeScalarView(output)).data(using: .utf8)
    }
}

nonisolated struct MacManagedMCPServerDraft: Equatable, Sendable {
    enum Transport: String, CaseIterable, Identifiable, Sendable {
        case stdio
        case http

        var id: String { rawValue }
        var title: String {
            switch self {
            case .stdio: return "Stdio"
            case .http: return "HTTP / SSE"
            }
        }
    }

    private static let managedKeys: Set<String> = [
        "name", "command", "args", "env", "cwd", "url", "endpoint", "headers", "timeout_ms", "timeoutMs",
    ]

    var displayName: String = ""
    var transport: Transport = .stdio
    var stdioCommand: String = ""
    var httpEndpointURL: String = ""
    var argumentsText: String = ""
    var workingDirectory: String = ""
    var environmentText: String = ""
    var headersText: String = ""
    var timeoutMilliseconds: String = ""
    var enabled: Bool = true
    var preservedExtraJSON: String? = nil

    init(
        displayName: String = "",
        transport: Transport = .stdio,
        stdioCommand: String = "",
        httpEndpointURL: String = "",
        commandOrURL: String? = nil,
        argumentsText: String = "",
        workingDirectory: String = "",
        environmentText: String = "",
        headersText: String = "",
        timeoutMilliseconds: String = "",
        enabled: Bool = true,
        preservedExtraJSON: String? = nil
    ) {
        self.displayName = displayName
        self.transport = transport
        if let commandOrURL {
            self.stdioCommand = transport == .stdio ? commandOrURL : stdioCommand
            self.httpEndpointURL = transport == .http ? commandOrURL : httpEndpointURL
        } else {
            self.stdioCommand = stdioCommand
            self.httpEndpointURL = httpEndpointURL
        }
        self.argumentsText = argumentsText
        self.workingDirectory = workingDirectory
        self.environmentText = environmentText
        self.headersText = headersText
        self.timeoutMilliseconds = timeoutMilliseconds
        self.enabled = enabled
        self.preservedExtraJSON = preservedExtraJSON
    }

    var commandOrURL: String {
        get { transport == .stdio ? stdioCommand : httpEndpointURL }
        set {
            switch transport {
            case .stdio: stdioCommand = newValue
            case .http: httpEndpointURL = newValue
            }
        }
    }

    func validatedConfigurationJSON() throws -> (displayName: String, json: String) {
        let trimmedName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty else {
            throw MacExtensionsServiceError.invalidDraftConfiguration(
                String(localized: "MCP server name cannot be empty.")
            )
        }
        var payload: [String: Any] = [:]
        if let preservedExtraJSON,
           let extraData = preservedExtraJSON.data(using: .utf8),
           let extraObject = (try? JSONSerialization.jsonObject(with: extraData)) as? [String: Any] {
            for (key, value) in extraObject where !Self.managedKeys.contains(key) {
                payload[key] = value
            }
        }
        payload["name"] = trimmedName
        let trimmedTimeout = timeoutMilliseconds.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedTimeout.isEmpty {
            guard let timeout = Int(trimmedTimeout), timeout >= 1_000, timeout <= 21_600_000 else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Timeout must be an integer between 1000 and 21600000 ms.")
                )
            }
            payload["timeout_ms"] = timeout
        }
        switch transport {
        case .stdio:
            let trimmedCommand = stdioCommand.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmedCommand.isEmpty else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Stdio command cannot be empty.")
                )
            }
            payload["command"] = trimmedCommand
            let args = argumentsText
                .components(separatedBy: .newlines)
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
            if !args.isEmpty { payload["args"] = args }
            let trimmedCwd = workingDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmedCwd.isEmpty { payload["cwd"] = trimmedCwd }
            let env = try Self.parseKeyValueLines(environmentText, separator: "=", label: "Environment")
            if !env.isEmpty { payload["env"] = env }
        case .http:
            let trimmedEndpoint = httpEndpointURL.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let url = URL(string: trimmedEndpoint),
                  let scheme = url.scheme?.lowercased(),
                  (scheme == "http" || scheme == "https"),
                  !(url.host ?? "").isEmpty else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Endpoint URL must be a valid HTTP or HTTPS URL.")
                )
            }
            payload["url"] = trimmedEndpoint
            payload["endpoint"] = trimmedEndpoint
            let headers = try Self.parseKeyValueLines(headersText, separator: ":", label: "Headers")
            if !headers.isEmpty { payload["headers"] = headers }
        }
        let data = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
        return (trimmedName, String(decoding: data, as: UTF8.self))
    }

    static func from(server: MacCanonicalMCPServer) -> MacManagedMCPServerDraft? {
        guard let data = server.configurationJSON.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return nil
        }
        let extra = object.filter { !managedKeys.contains($0.key) }
        let extraJSON = extra.isEmpty
            ? nil
            : (try? JSONSerialization.data(withJSONObject: extra, options: [.sortedKeys]))
                .map { String(decoding: $0, as: UTF8.self) }
        let timeoutText = (object["timeout_ms"] as? NSNumber ?? object["timeoutMs"] as? NSNumber)
            .map { String($0.intValue) } ?? ""
        if let command = object["command"] as? String {
            let args = (object["args"] as? [String])?.joined(separator: "\n") ?? ""
            let env = (object["env"] as? [String: String])?
                .sorted { $0.key < $1.key }
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: "\n") ?? ""
            let cwd = (object["cwd"] as? String) ?? ""
            return MacManagedMCPServerDraft(
                displayName: server.displayName,
                transport: .stdio,
                stdioCommand: command,
                httpEndpointURL: "",
                argumentsText: args,
                workingDirectory: cwd,
                environmentText: env,
                headersText: "",
                timeoutMilliseconds: timeoutText,
                enabled: server.enabled,
                preservedExtraJSON: extraJSON
            )
        }
        if let endpoint = (object["url"] as? String) ?? (object["endpoint"] as? String) {
            let headers = (object["headers"] as? [String: String])?
                .sorted { $0.key < $1.key }
                .map { "\($0.key): \($0.value)" }
                .joined(separator: "\n") ?? ""
            return MacManagedMCPServerDraft(
                displayName: server.displayName,
                transport: .http,
                stdioCommand: "",
                httpEndpointURL: endpoint,
                argumentsText: "",
                workingDirectory: "",
                environmentText: "",
                headersText: headers,
                timeoutMilliseconds: timeoutText,
                enabled: server.enabled,
                preservedExtraJSON: extraJSON
            )
        }
        return nil
    }

    private static func parseKeyValueLines(
        _ text: String,
        separator: Character,
        label: String
    ) throws -> [String: String] {
        var result: [String: String] = [:]
        for line in text.split(whereSeparator: \.isNewline) {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }
            guard let idx = trimmed.firstIndex(of: separator) else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(
                        format: String(localized: "%1$@ line (%2$@) must use KEY%3$@VALUE format."),
                        label,
                        trimmed,
                        String(separator)
                    )
                )
            }
            let key = String(trimmed[..<idx]).trimmingCharacters(in: .whitespacesAndNewlines)
            let value = String(trimmed[trimmed.index(after: idx)...]).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !key.isEmpty else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(
                        format: String(localized: "%1$@ key cannot be empty in line (%2$@)."),
                        label,
                        trimmed
                    )
                )
            }
            result[key] = value
        }
        return result
    }
}

actor MacExtensionsService {
    static let shared = MacExtensionsService()

    private let maximumDocumentBytes = 2 * 1_024 * 1_024
    private let fileManager = FileManager.default
    func localExtensions(
        workspaceURL: URL?,
        store: MacApplicationPort = .shared
    ) throws -> [MacExtensionItem] {
        let skills = try store.listSkills()
        let mcpServers = (try? store.listMCPServers()) ?? []
        var skillEnabledByID: [String: Bool] = [:]
        for skill in skills {
            skillEnabledByID[skill.id] = skill.enabled
            let normalized = macExtensionStrippingLegacyDigest(skill.id)
            if skillEnabledByID[normalized] == nil {
                skillEnabledByID[normalized] = skill.enabled
            }
        }
        var mcpEnabledByNormalizedID: [String: Bool] = [:]
        for server in mcpServers where server.deletedAtMs == nil {
            let normalized = macExtensionStrippingLegacyDigest(server.id)
            if mcpEnabledByNormalizedID[normalized] == nil {
                mcpEnabledByNormalizedID[normalized] = server.enabled
            }
        }
        var items = try MacBuiltInSkillRegistry.extensionItems(store: store, skills: skills)
        if let workspaceURL {
            items.append(contentsOf: scanSkills(root: workspaceURL, source: .workspace, limit: 120, skillEnabledByID: skillEnabledByID))
            items.append(contentsOf: scanKnownExtensions(root: workspaceURL, source: .workspace, mcpEnabledByNormalizedID: mcpEnabledByNormalizedID))
        }
        let globalSkills = MacAppEnvironment.shared.skillsDirectory
        if fileManager.fileExists(atPath: globalSkills.path) {
            items.append(contentsOf: scanSkills(root: globalSkills, source: .global, limit: 80, skillEnabledByID: skillEnabledByID))
        }
        items.append(contentsOf: scanKnownExtensions(
            root: fileManager.homeDirectoryForCurrentUser,
            source: .global,
            mcpEnabledByNormalizedID: mcpEnabledByNormalizedID
        ))
        items.append(contentsOf: skills.compactMap { skill in
            guard let source = skill.gitHubSourceURL else { return nil }
            return MacExtensionItem(id: "managed-skill:" + skill.id, kind: .skill, source: .global,
                title: skill.displayName, detail: skill.summary.isEmpty ? source.absoluteString : skill.summary,
                path: nil, isEnabled: skill.enabled)
        })
        items.append(contentsOf: managedMCPItems(store: store))
        return deduplicated(items).sorted {
            if $0.kind != $1.kind { return $0.kind.rawValue < $1.kind.rawValue }
            return $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
        }
    }

    func managedMCPDraft(
        forItemID itemID: String,
        store: MacApplicationPort = .shared
    ) throws -> MacManagedMCPServerDraft? {
        guard let server = try managedMCPServer(forItemID: itemID, store: store) else { return nil }
        return MacManagedMCPServerDraft.from(server: server)
    }

    @discardableResult
    func createManagedMCPServer(
        _ draft: MacManagedMCPServerDraft,
        store: MacApplicationPort = .shared
    ) throws -> MacCanonicalMCPServer {
        let validated = try draft.validatedConfigurationJSON()
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return try store.createMCPServer(MacCanonicalMCPServerInput(
            id: UUID().uuidString.lowercased(),
            displayName: validated.displayName,
            enabled: draft.enabled,
            configurationJSON: validated.json,
            revision: 0,
            createdAtMs: now,
            updatedAtMs: now,
            deletedAtMs: nil
        ))
    }

    @discardableResult
    func updateManagedMCPServer(
        itemID: String,
        draft: MacManagedMCPServerDraft,
        store: MacApplicationPort = .shared
    ) throws -> MacCanonicalMCPServer {
        guard let existing = try managedMCPServer(forItemID: itemID, store: store) else {
            throw MacExtensionsServiceError.contentUnavailable
        }
        let validated = try draft.validatedConfigurationJSON()
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return try store.configureMCPServer(
            existing,
            displayName: validated.displayName,
            enabled: draft.enabled,
            configurationJSON: validated.json,
            updatedAtMs: now
        )
    }

    @discardableResult
    func deleteManagedMCPServer(
        itemID: String,
        store: MacApplicationPort = .shared
    ) throws -> Bool {
        guard let existing = try managedMCPServer(forItemID: itemID, store: store) else {
            return false
        }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return try store.deleteMCPServer(existing, deletedAtMs: now)
    }

    func document(
        for item: MacExtensionItem,
        store: MacApplicationPort = .shared
    ) throws -> MacExtensionDocument {
        guard item.source != .connectedHost else {
            throw MacExtensionsServiceError.contentUnavailable
        }
        try authorizeManagedResource(item, permission: .readContent)
        if let id = item.installedSkillID {
            guard let editor = try store.skillEditor(id), let url = editor.record.gitHubSourceURL else {
                throw MacExtensionsServiceError.contentUnavailable
            }
            return MacExtensionDocument(content: editor.record.instructionsText, url: url, format: .markdown,
                isEditable: true, canonicalSkillRevision: editor.revision)
        }
        if item.source == .builtIn {
            guard let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id),
                  let editor = try store.skillEditor(skillID.rawValue),
                  editor.record.source == .builtIn else {
                throw MacExtensionsServiceError.contentUnavailable
            }
            let url = try MacBuiltInSkillRegistry.materializedInstructionURL(for: skillID, record: editor.record)
            guard editor.record.instructionsText.utf8.count <= maximumDocumentBytes else {
                throw MacExtensionsServiceError.contentTooLarge
            }
            return MacExtensionDocument(
                content: editor.record.instructionsText, url: url,
                format: .markdown, isEditable: item.isLocallyEditable,
                canonicalSkillRevision: editor.revision
            )
        }
        if item.id.hasPrefix("managed-mcp:") {
            guard let server = try managedMCPServer(forItemID: item.id, store: store) else {
                throw MacExtensionsServiceError.contentUnavailable
            }
            let content = prettyPrintedJSON(server.configurationJSON) ?? server.configurationJSON
            guard content.utf8.count <= maximumDocumentBytes else {
                throw MacExtensionsServiceError.contentTooLarge
            }
            return MacExtensionDocument(
                content: content,
                url: URL(fileURLWithPath: "/managed-mcp/\(server.id).json"),
                format: .json,
                isEditable: true
            )
        }
        let url: URL
        if let path = item.path {
            url = URL(fileURLWithPath: path).standardizedFileURL
        } else {
            throw MacExtensionsServiceError.contentUnavailable
        }
        let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard values.isRegularFile == true else {
            throw MacExtensionsServiceError.contentUnavailable
        }
        guard (values.fileSize ?? 0) <= maximumDocumentBytes else {
            throw MacExtensionsServiceError.contentTooLarge
        }
        let data = try Data(contentsOf: url, options: [.mappedIfSafe])
        guard let content = String(data: data, encoding: .utf8) else {
            throw MacExtensionsServiceError.invalidText
        }
        return MacExtensionDocument(
            content: content,
            url: url,
            format: documentFormat(for: url, kind: item.kind),
            isEditable: item.isLocallyEditable
        )
    }

    func save(
        _ content: String,
        for item: MacExtensionItem,
        expectedRevision: Int64? = nil,
        store: MacApplicationPort = .shared
    ) throws -> MacExtensionDocument {
        guard item.isLocallyEditable else {
            throw MacExtensionsServiceError.readOnly
        }
        try authorizeManagedResource(item, permission: .editContent)
        let data = Data(content.utf8)
        guard data.count <= maximumDocumentBytes else {
            throw MacExtensionsServiceError.contentTooLarge
        }
        if item.kind == .skill,
           content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw MacExtensionsServiceError.emptySkill
        }
        if let id = item.installedSkillID {
            guard let editor = try store.skillEditor(id), let url = editor.record.gitHubSourceURL,
                  let expectedRevision, editor.revision == expectedRevision else {
                throw MacApplicationPortError.rejected(String(localized: "The Skill has changed. Reopen it before saving."))
            }
            let updated = try store.updateInstalledSkillInstructions(content, expected: editor)
            return MacExtensionDocument(content: updated.record.instructionsText, url: url,
                format: .markdown, isEditable: true, canonicalSkillRevision: updated.revision)
        }
        if item.id.hasPrefix("managed-mcp:") {
            let trimmed = content.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty,
                  let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                  !object.isEmpty else {
                throw MacExtensionsServiceError.invalidJSON
            }
            guard let server = try managedMCPServer(forItemID: item.id, store: store) else {
                throw MacExtensionsServiceError.contentUnavailable
            }
            let displayName: String
            if let rawName = object["name"] {
                guard let nameString = rawName as? String else {
                    throw MacExtensionsServiceError.invalidDraftConfiguration(
                        String(localized: "MCP server name cannot be empty.")
                    )
                }
                let trimmedName = nameString.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmedName.isEmpty else {
                    throw MacExtensionsServiceError.invalidDraftConfiguration(
                        String(localized: "MCP server name cannot be empty.")
                    )
                }
                displayName = trimmedName
            } else {
                displayName = server.displayName
            }
            if let rawTimeout = object["timeout_ms"] ?? object["timeoutMs"] {
                guard let number = rawTimeout as? NSNumber,
                      CFGetTypeID(number) != CFBooleanGetTypeID(),
                      number.doubleValue == floor(number.doubleValue),
                      number.intValue >= 1_000,
                      number.intValue <= 21_600_000 else {
                    throw MacExtensionsServiceError.invalidDraftConfiguration(
                        String(localized: "Timeout must be an integer between 1000 and 21600000 ms.")
                    )
                }
            }
            let rawCommand = (object["command"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
            let rawEndpoint = ((object["url"] as? String) ?? (object["endpoint"] as? String))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let hasValidStdio = rawCommand?.isEmpty == false
            let hasValidHTTP: Bool = {
                guard let rawEndpoint, !rawEndpoint.isEmpty,
                      let url = URL(string: rawEndpoint),
                      let scheme = url.scheme?.lowercased(),
                      (scheme == "http" || scheme == "https"),
                      !(url.host ?? "").isEmpty else {
                    return false
                }
                return true
            }()
            if object["command"] != nil, !hasValidStdio {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Stdio command cannot be empty.")
                )
            }
            if (object["url"] != nil || object["endpoint"] != nil), !hasValidHTTP {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Endpoint URL must be a valid HTTP or HTTPS URL.")
                )
            }
            guard hasValidStdio || hasValidHTTP else {
                throw MacExtensionsServiceError.invalidDraftConfiguration(
                    String(localized: "Endpoint URL must be a valid HTTP or HTTPS URL.")
                )
            }
            let now = Int64(Date().timeIntervalSince1970 * 1_000)
            let updated = try store.configureMCPServer(
                server,
                displayName: displayName,
                enabled: server.enabled,
                configurationJSON: trimmed,
                updatedAtMs: now
            )
            let formatted = prettyPrintedJSON(updated.configurationJSON) ?? updated.configurationJSON
            return MacExtensionDocument(
                content: formatted,
                url: URL(fileURLWithPath: "/managed-mcp/\(updated.id).json"),
                format: .json,
                isEditable: true
            )
        }
        let url: URL
        if item.source == .builtIn,
           let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) {
            url = try MacBuiltInSkillRegistry.saveOverride(content, for: skillID, store: store)
            try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
            try MacBuiltInSkillRegistry.installTrustedRouter()
        } else if let path = item.path {
            url = URL(fileURLWithPath: path).standardizedFileURL
        } else {
            throw MacExtensionsServiceError.readOnly
        }
        if item.kind == .mcp,
           url.pathExtension.lowercased() == "json",
           (try? JSONSerialization.jsonObject(with: data)) == nil {
            throw MacExtensionsServiceError.invalidJSON
        }
        if item.source != .builtIn {
            try data.write(to: url, options: .atomic)
        }
        return MacExtensionDocument(
            content: content,
            url: url,
            format: documentFormat(for: url, kind: item.kind),
            isEditable: true
        )
    }

    private func managedMCPItems(store: MacApplicationPort) -> [MacExtensionItem] {
        let servers = (try? store.listMCPServers()) ?? []
        return servers
            .filter { $0.deletedAtMs == nil && !$0.id.contains(":mcp:") }
            .map { server in
                MacExtensionItem(
                    id: "managed-mcp:\(server.id)",
                    kind: .mcp,
                    source: .global,
                    title: server.displayName,
                    detail: managedMCPDetail(server),
                    path: nil,
                    isEnabled: server.enabled,
                    mcpSummary: .configured
                )
            }
    }

    private func managedMCPServer(
        forItemID itemID: String,
        store: MacApplicationPort
    ) throws -> MacCanonicalMCPServer? {
        guard itemID.hasPrefix("managed-mcp:") else { return nil }
        let serverID = String(itemID.dropFirst("managed-mcp:".count))
        return try store.listMCPServers().first { $0.id == serverID && $0.deletedAtMs == nil }
    }

    private func managedMCPDetail(_ server: MacCanonicalMCPServer) -> String {
        guard let data = server.configurationJSON.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return String(localized: "Managed MCP")
        }
        if let command = object["command"] as? String, !command.isEmpty {
            let args = (object["args"] as? [String])?.joined(separator: " ") ?? ""
            return args.isEmpty ? command : "\(command) \(args)"
        }
        if let endpoint = (object["url"] as? String) ?? (object["endpoint"] as? String), !endpoint.isEmpty {
            return endpoint
        }
        return String(localized: "Managed MCP")
    }

    private func prettyPrintedJSON(_ raw: String) -> String? {
        guard let data = raw.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data),
              let pretty = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]) else {
            return nil
        }
        return String(decoding: pretty, as: UTF8.self)
    }

    private func authorizeManagedResource(
        _ item: MacExtensionItem,
        permission: MacCoreResourcePermission
    ) throws {
        let kind: MacCoreResourceKind
        switch item.kind {
        case .skill:
            kind = .skill
        case .mcp:
            kind = .mcpServer
        default:
            return
        }
        guard MacCoreResourceAccess.evaluate(
            item: item,
            kind: kind,
            permission: permission
        )?.allowed == true else {
            if permission == .editContent {
                throw MacExtensionsServiceError.readOnly
            }
            throw MacExtensionsServiceError.contentUnavailable
        }
    }

    private func scanSkills(
        root: URL,
        source: MacExtensionItem.Source,
        limit: Int,
        skillEnabledByID: [String: Bool]
    ) -> [MacExtensionItem] {
        var items: [MacExtensionItem] = []
        for url in MacLocalSkillActivationStore.instructionURLs(root: root, limit: limit) {
            let text = readPrefix(url, maximumBytes: 96_000)
            let title = skillTitle(url: url, text: text)
            let detail = skillDescription(text)
            let id = MacLocalSkillActivationStore.itemID(source: source, instructionURL: url)
            let normalized = macExtensionStrippingLegacyDigest(id)
            let isEnabled = macExtensionTrustedItemID(id)
                ? (skillEnabledByID[normalized] ?? skillEnabledByID[id] ?? false)
                : false
            items.append(MacExtensionItem(
                id: id,
                kind: .skill,
                source: source,
                title: title,
                detail: detail,
                path: url.path,
                isEnabled: isEnabled
            ))
        }
        return items
    }

    private func scanKnownExtensions(
        root: URL,
        source: MacExtensionItem.Source,
        mcpEnabledByNormalizedID: [String: Bool]
    ) -> [MacExtensionItem] {
        var items: [MacExtensionItem] = []
        for relativePath in MacLocalMCPActivationStore.configurationPaths {
            let url = root.appendingPathComponent(relativePath)
            guard fileManager.fileExists(atPath: url.path) else { continue }
            let mcpNames = MacLocalMCPActivationStore.serverNames(in: url)
            if !mcpNames.isEmpty {
                for name in mcpNames {
                    let itemID = MacLocalMCPActivationStore.itemID(
                        source: source,
                        configurationURL: url,
                        serverName: name
                    )
                    let normalized = macExtensionStrippingLegacyDigest(itemID)
                    let isEnabled = macExtensionTrustedItemID(itemID)
                        ? (mcpEnabledByNormalizedID[normalized] ?? false)
                        : false
                    items.append(MacExtensionItem(
                        id: itemID,
                        kind: .mcp,
                        source: source,
                        title: name,
                        detail: String(localized: "mac.extensions.state.discovered_configuration") + " · " + relativePath,
                        path: url.path,
                        isEnabled: isEnabled,
                        mcpSummary: .configured
                    ))
                }
            } else {
                items.append(MacExtensionItem(
                    id: "\(source.rawValue):config:\(url.path)",
                    kind: .configuration,
                    source: source,
                    title: url.lastPathComponent,
                    detail: relativePath,
                    path: url.path,
                    isEnabled: true
                ))
            }
        }

        let directories: [(String, MacExtensionItem.Kind)] = [
            (".claude/agents", .agent), (".opencode/agent", .agent), (".opencode/agents", .agent),
            ("agents", .agent), (".claude/commands", .command), (".opencode/command", .command),
            (".opencode/commands", .command), ("commands", .command),
            (".opencode/plugin", .plugin), (".opencode/plugins", .plugin), (".claude/plugins", .plugin),
        ]
        for (relativeDirectory, kind) in directories {
            let directory = root.appendingPathComponent(relativeDirectory, isDirectory: true)
            guard let enumerator = fileManager.enumerator(
                at: directory,
                includingPropertiesForKeys: [.isRegularFileKey],
                options: [.skipsHiddenFiles, .skipsPackageDescendants]
            ) else { continue }
            var count = 0
            while let url = enumerator.nextObject() as? URL, count < 100 {
                guard (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else { continue }
                count += 1
                let text = readPrefix(url, maximumBytes: 48_000)
                let title = frontmatterValue("name", in: text)
                    ?? url.deletingPathExtension().lastPathComponent
                let detail = frontmatterValue("description", in: text)
                    ?? relativeDirectory
                items.append(MacExtensionItem(
                    id: "\(source.rawValue):\(kind.rawValue):\(url.path)",
                    kind: kind,
                    source: source,
                    title: title,
                    detail: detail,
                    path: url.path,
                    isEnabled: true
                ))
            }
        }
        return items
    }

    private func skillTitle(url: URL, text: String) -> String {
        if let name = frontmatterValue("name", in: text) { return name }
        for line in text.split(separator: "\n").prefix(40) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.lowercased().hasPrefix("# skill:"), let separator = trimmed.firstIndex(of: ":") {
                let value = trimmed[trimmed.index(after: separator)...].trimmingCharacters(in: .whitespaces)
                if !value.isEmpty { return value }
            }
            if trimmed.hasPrefix("# ") { return String(trimmed.dropFirst(2)) }
        }
        return url.deletingLastPathComponent().lastPathComponent
    }

    private func skillDescription(_ text: String) -> String {
        if let value = frontmatterValue("description", in: text) { return value }
        return text.split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty && !$0.hasPrefix("#") && !$0.hasPrefix("---") } ?? "Reusable CLI workflow"
    }

    private func frontmatterValue(_ key: String, in text: String) -> String? {
        for line in text.split(separator: "\n").prefix(50) {
            let value = line.trimmingCharacters(in: .whitespaces)
            guard value.lowercased().hasPrefix("\(key.lowercased()):") else { continue }
            let result = value.dropFirst(key.count + 1)
                .trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            if !result.isEmpty { return result }
        }
        return nil
    }

    private func readPrefix(_ url: URL, maximumBytes: Int) -> String {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return "" }
        defer { try? handle.close() }
        let data = (try? handle.read(upToCount: maximumBytes)) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    private func documentFormat(
        for url: URL,
        kind: MacExtensionItem.Kind
    ) -> MacExtensionDocument.Format {
        if kind == .skill || url.pathExtension.lowercased() == "md" {
            return .markdown
        }
        if ["json", "jsonc"].contains(url.pathExtension.lowercased()) {
            return .json
        }
        return .text
    }

    private func deduplicated(_ items: [MacExtensionItem]) -> [MacExtensionItem] {
        var seen: Set<String> = []
        return items.filter { seen.insert($0.id).inserted }
    }
}
