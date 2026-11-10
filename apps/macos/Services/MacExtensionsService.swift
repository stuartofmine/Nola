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

nonisolated private func macExtensionTrustedItemID(_ itemID: String) -> Bool {
    guard let marker = itemID.range(of: "#sha256=", options: .backwards) else { return false }
    let digest = itemID[marker.upperBound...]
    return digest.count == 64 && digest.allSatisfy { $0.isHexDigit }
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

    var isLocallyEditable: Bool {
        path != nil && source != .connectedHost
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
        let digest = macExtensionBoundedData(at: url).map(macExtensionContentDigest)
            ?? "unavailable"
        return "\(source.rawValue):skill:\(url.path)#sha256=\(digest)"
    }

    static func isEnabled(
        itemID: String,
        store: MacApplicationPort? = .shared
    ) -> Bool {
        guard macExtensionTrustedItemID(itemID) else { return false }
        if let record = try? store?.listSkills().first(where: { $0.id == itemID }) {
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
        if let current = try store.skillEditor(itemID) {
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
                instructionsDigest: digest,
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
        guard let marker = itemID.range(of: ":skill:"),
              let digestMarker = itemID.range(of: "#sha256=", options: .backwards),
              digestMarker.lowerBound > marker.upperBound,
              let source = MacExtensionItem.Source(rawValue: String(itemID[..<marker.lowerBound]))
        else { return nil }
        let path = String(itemID[marker.upperBound..<digestMarker.lowerBound])
        let digest = String(itemID[digestMarker.upperBound...])
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard let data = macExtensionBoundedData(at: url),
              macExtensionContentDigest(data) == digest,
              let text = String(data: data, encoding: .utf8) else { return nil }
        return Candidate(id: itemID, source: source, url: url, digest: digest, text: text)
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
        let digest = serverConfigurations(in: url)[serverName]
            .flatMap { try? JSONSerialization.data(withJSONObject: $0, options: [.sortedKeys]) }
            .map(macExtensionContentDigest)
            ?? "unavailable"
        return "\(source.rawValue):mcp:\(url.path):\(serverName)#sha256=\(digest)"
    }

    static func isEnabled(
        itemID: String,
        store: MacApplicationPort? = .shared
    ) -> Bool {
        guard macExtensionTrustedItemID(itemID) else { return false }
        if let record = try? store?.listMCPServers().first(where: { $0.id == itemID }) {
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
        if let current = try store.listMCPServers().first(where: { $0.id == itemID }) {
            _ = try store.configureMCPServer(
                current, displayName: current.displayName, enabled: enabled,
                configurationJSON: current.configurationJSON,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
        } else {
            _ = try store.createMCPServer(candidate.input(enabled: enabled))
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
        let canonical = Dictionary(
            uniqueKeysWithValues: ((try? store?.listMCPServers()) ?? []).map { ($0.id, $0) }
        )
        for (source, root) in roots {
            for relativePath in configurationPaths {
                let url = root.appendingPathComponent(relativePath).standardizedFileURL
                for (name, rawConfiguration) in serverConfigurations(in: url).sorted(by: { $0.key < $1.key }) {
                    let id = itemID(source: source, configurationURL: url, serverName: name)
                    let enabled = canonical[id]?.enabled
                        ?? isEnabled(itemID: id, store: store)
                    guard enabled, seen.insert(id).inserted else {
                        continue
                    }
                    var configuration = rawConfiguration
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
        guard let marker = itemID.range(of: ":mcp:"),
              let digestMarker = itemID.range(of: "#sha256=", options: .backwards),
              digestMarker.lowerBound > marker.upperBound,
              let source = MacExtensionItem.Source(rawValue: String(itemID[..<marker.lowerBound]))
        else { return nil }
        let identity = String(itemID[marker.upperBound..<digestMarker.lowerBound])
        guard let separator = identity.range(of: ":", options: .backwards) else { return nil }
        let url = URL(fileURLWithPath: String(identity[..<separator.lowerBound])).standardizedFileURL
        let name = String(identity[separator.upperBound...])
        let digest = String(itemID[digestMarker.upperBound...])
        guard let configuration = serverConfigurations(in: url)[name],
              let data = try? JSONSerialization.data(withJSONObject: configuration, options: [.sortedKeys]),
              macExtensionContentDigest(data) == digest else { return nil }
        return Candidate(
            id: itemID,
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

actor MacExtensionsService {
    static let shared = MacExtensionsService()

    private let maximumDocumentBytes = 2 * 1_024 * 1_024
    private let fileManager = FileManager.default
    func localExtensions(workspaceURL: URL?) throws -> [MacExtensionItem] {
        var items = try MacBuiltInSkillRegistry.extensionItems()
        if let workspaceURL {
            items.append(contentsOf: scanSkills(root: workspaceURL, source: .workspace, limit: 120))
            items.append(contentsOf: scanKnownExtensions(root: workspaceURL, source: .workspace))
        }
        let globalSkills = MacAppEnvironment.shared.skillsDirectory
        if fileManager.fileExists(atPath: globalSkills.path) {
            items.append(contentsOf: scanSkills(root: globalSkills, source: .global, limit: 80))
        }
        items.append(contentsOf: scanKnownExtensions(
            root: fileManager.homeDirectoryForCurrentUser,
            source: .global
        ))
        return deduplicated(items).sorted {
            if $0.kind != $1.kind { return $0.kind.rawValue < $1.kind.rawValue }
            return $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
        }
    }

    func document(
        for item: MacExtensionItem,
        store: MacApplicationPort = .shared
    ) throws -> MacExtensionDocument {
        guard item.source != .connectedHost else {
            throw MacExtensionsServiceError.contentUnavailable
        }
        try authorizeManagedResource(item, permission: .readContent)
        if item.source == .builtIn {
            guard let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id),
                  let editor = try store.skillEditor(skillID.rawValue),
                  editor.record.source == .builtIn,
                  let url = URL(string: editor.record.instructionsRef), url.isFileURL else {
                throw MacExtensionsServiceError.contentUnavailable
            }
            guard editor.record.instructionsText.utf8.count <= maximumDocumentBytes else {
                throw MacExtensionsServiceError.contentTooLarge
            }
            return MacExtensionDocument(
                content: editor.record.instructionsText, url: url,
                format: .markdown, isEditable: item.isLocallyEditable,
                canonicalSkillRevision: editor.revision
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

    func save(_ content: String, for item: MacExtensionItem) throws -> MacExtensionDocument {
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
        let url: URL
        if item.source == .builtIn,
           let skillID = MacBuiltInSkillRegistry.skillID(forExtensionItemID: item.id) {
            url = try MacBuiltInSkillRegistry.saveOverride(content, for: skillID)
            try MacBuiltInSkillRegistry.synchronizeCatalog()
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

    private func scanSkills(root: URL, source: MacExtensionItem.Source, limit: Int) -> [MacExtensionItem] {
        var items: [MacExtensionItem] = []
        for url in MacLocalSkillActivationStore.instructionURLs(root: root, limit: limit) {
            let text = readPrefix(url, maximumBytes: 96_000)
            let title = skillTitle(url: url, text: text)
            let detail = skillDescription(text)
            let id = MacLocalSkillActivationStore.itemID(source: source, instructionURL: url)
            items.append(MacExtensionItem(
                id: id,
                kind: .skill,
                source: source,
                title: title,
                detail: detail,
                path: url.path,
                isEnabled: MacLocalSkillActivationStore.isEnabled(itemID: id)
            ))
        }
        return items
    }

    private func scanKnownExtensions(root: URL, source: MacExtensionItem.Source) -> [MacExtensionItem] {
        var items: [MacExtensionItem] = []
        for relativePath in MacLocalMCPActivationStore.configurationPaths {
            let url = root.appendingPathComponent(relativePath)
            guard fileManager.fileExists(atPath: url.path) else { continue }
            let mcpNames = MacLocalMCPActivationStore.serverNames(in: url)
            if !mcpNames.isEmpty {
                for name in mcpNames {
                    items.append(MacExtensionItem(
                        id: MacLocalMCPActivationStore.itemID(
                            source: source,
                            configurationURL: url,
                            serverName: name
                        ),
                        kind: .mcp,
                        source: source,
                        title: name,
                        detail: String(localized: "mac.extensions.state.discovered_configuration") + " · " + relativePath,
                        path: url.path,
                        isEnabled: MacLocalMCPActivationStore.isEnabled(itemID:
                            MacLocalMCPActivationStore.itemID(
                                source: source,
                                configurationURL: url,
                                serverName: name
                            )
                        ),
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
