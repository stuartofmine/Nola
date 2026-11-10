// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ContextManager logic in the iOS service layer.
// - Interfaces:
//   - PromptSegment
//   - UserMCPServer
//   - UserUploadedContextItem
//   - ChatMemoryProfile
//   - ChatMemoryProfileStoreError
//   - ChatMemoryProfileStore
//   - CanonicalChatMemoryProfileStore
//   - ConversationContextUsage
// - APIs:
//   - none
//
//
//  ContextManager.swift
//  Nola
//

import Foundation

nonisolated public struct PromptSegment {
    public let role: String
    public let content: String
}

nonisolated public struct UserMCPServer: Codable, Equatable, Identifiable {
    public var id: UUID
    public var name: String
    public var endpoint: String
    public var description: String
    public var enabled: Bool
    public var serverID: String?
    public var availability: String?
    public var protocolVersion: String?
    public var tools: [String]?
    public var resources: [String]?
    public var resourceTemplates: [String]?
    public var prompts: [String]?

    public init(
        id: UUID = UUID(),
        name: String,
        endpoint: String,
        description: String = "",
        enabled: Bool = true,
        serverID: String? = nil,
        availability: String? = nil,
        protocolVersion: String? = nil,
        tools: [String]? = nil,
        resources: [String]? = nil,
        resourceTemplates: [String]? = nil,
        prompts: [String]? = nil
    ) {
        self.id = id
        self.name = name
        self.endpoint = endpoint
        self.description = description
        self.enabled = enabled
        self.serverID = serverID
        self.availability = availability
        self.protocolVersion = protocolVersion
        self.tools = tools
        self.resources = resources
        self.resourceTemplates = resourceTemplates
        self.prompts = prompts
    }
}

nonisolated public struct UserUploadedContextItem: Codable, Equatable, Identifiable {
    public var id: UUID
    public var name: String
    public var content: String
    public var description: String
    public var enabled: Bool
    public var version: String?
    public var requiredTools: [String]?
    public var validationSteps: [String]?

    public init(
        id: UUID = UUID(),
        name: String,
        content: String,
        description: String = "",
        enabled: Bool = true,
        version: String? = nil,
        requiredTools: [String]? = nil,
        validationSteps: [String]? = nil
    ) {
        self.id = id
        self.name = name
        self.content = content
        self.description = description
        self.enabled = enabled
        self.version = version
        self.requiredTools = requiredTools
        self.validationSteps = validationSteps
    }
}

nonisolated public enum ChatMemoryNamespace: String, Codable, Equatable {
    case longTerm = "long_term"
    case userPreferences = "user_preferences"
}

nonisolated public enum ChatMemoryRecordKind: String, Codable, Equatable {
    case semantic
    case procedural
}

nonisolated public enum ChatMemoryRetention: String, Codable, Equatable {
    case durable
    case session
    case ephemeral
}

nonisolated public struct ChatMemoryProvenance: Codable, Equatable {
    public var source: String
    public var sourceID: String?
    public var conversationID: UUID?
    public var messageID: UUID?
    public var model: String?
    public var route: String?

    public init(
        source: String,
        sourceID: String? = nil,
        conversationID: UUID? = nil,
        messageID: UUID? = nil,
        model: String? = nil,
        route: String? = nil
    ) {
        self.source = source
        self.sourceID = sourceID
        self.conversationID = conversationID
        self.messageID = messageID
        self.model = model
        self.route = route
    }
}

nonisolated public struct ChatMemoryRecord: Codable, Equatable, Identifiable {
    public var id: String
    public var owner: CoreMemoryOwner
    public var namespace: ChatMemoryNamespace
    public var kind: ChatMemoryRecordKind
    public var text: String
    public var confidence: Double?
    public var provenance: ChatMemoryProvenance
    public var observedAt: Date?
    public var retention: ChatMemoryRetention
    public var supersedes: [String]
    public var sensitivity: String?

    public init(
        id: String,
        owner: CoreMemoryOwner = .localProfile,
        namespace: ChatMemoryNamespace,
        kind: ChatMemoryRecordKind,
        text: String,
        confidence: Double? = nil,
        provenance: ChatMemoryProvenance,
        observedAt: Date? = nil,
        retention: ChatMemoryRetention = .durable,
        supersedes: [String] = [],
        sensitivity: String? = nil
    ) {
        self.id = id
        self.owner = owner
        self.namespace = namespace
        self.kind = kind
        self.text = text
        self.confidence = confidence
        self.provenance = provenance
        self.observedAt = observedAt
        self.retention = retention
        self.supersedes = supersedes
        self.sensitivity = sensitivity
    }
}

nonisolated public struct ChatMemoryProfile: Codable, Equatable {
    // Edit precondition only: not serialized, synchronized or used as canonical state.
    var canonicalMemorySnapshot: [IOSCanonicalMemoryRecord]? = []
    public var longTermMemory: String
    public var userPreferences: String
    public var memoryRecords: [ChatMemoryRecord]
    public var mcpServers: [UserMCPServer]
    public var uploadedSkills: [UserUploadedContextItem]
    public var uploadedDocuments: [UserUploadedContextItem]

    public init(
        longTermMemory: String = "",
        userPreferences: String = "",
        memoryRecords: [ChatMemoryRecord] = [],
        mcpServers: [UserMCPServer] = [],
        uploadedSkills: [UserUploadedContextItem] = [],
        uploadedDocuments: [UserUploadedContextItem] = []
    ) {
        self.longTermMemory = longTermMemory
        self.userPreferences = userPreferences
        self.memoryRecords = Self.normalizedRecords(memoryRecords)
        self.mcpServers = mcpServers
        self.uploadedSkills = uploadedSkills
        self.uploadedDocuments = uploadedDocuments
    }

    public static let empty = ChatMemoryProfile()

    private enum CodingKeys: String, CodingKey {
        case longTermMemory, userPreferences, memoryRecords, mcpServers, uploadedSkills, uploadedDocuments
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.longTermMemory = try container.decode(String.self, forKey: .longTermMemory)
        self.userPreferences = try container.decode(String.self, forKey: .userPreferences)
        self.memoryRecords = Self.normalizedRecords(
            try container.decode([ChatMemoryRecord].self, forKey: .memoryRecords)
        )
        self.mcpServers = try container.decode([UserMCPServer].self, forKey: .mcpServers)
        self.uploadedSkills = try container.decode([UserUploadedContextItem].self, forKey: .uploadedSkills)
        self.uploadedDocuments = try container.decode([UserUploadedContextItem].self, forKey: .uploadedDocuments)
    }

    public var activeMemoryRecords: [ChatMemoryRecord] {
        let superseded = Set(memoryRecords.flatMap(\.supersedes))
        return memoryRecords.filter { !superseded.contains($0.id) }
    }

    public func addingMemory(
        longTerm additions: [String],
        preferences preferenceAdditions: [String],
        provenance: ChatMemoryProvenance,
        observedAt: Date
    ) -> ChatMemoryProfile {
        var records = memoryRecords
        var recordKeys = Set(records.map(Self.logicalKey))

        func append(_ values: [String], namespace: ChatMemoryNamespace, kind: ChatMemoryRecordKind, confidence: Double) {
            for value in values {
                let text = value.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { continue }
                let key = Self.logicalKey(namespace: namespace, kind: kind, text: text)
                guard recordKeys.insert(key).inserted else { continue }
                records.append(ChatMemoryRecord(
                    id: "memory:\(UUID().uuidString)",
                    namespace: namespace,
                    kind: kind,
                    text: text,
                    confidence: confidence,
                    provenance: provenance,
                    observedAt: observedAt,
                    retention: .durable
                ))
            }
        }

        append(additions, namespace: .longTerm, kind: .semantic, confidence: 0.76)
        append(preferenceAdditions, namespace: .userPreferences, kind: .procedural, confidence: 0.8)
        var result = ChatMemoryProfile(
            longTermMemory: Self.mergedMemoryText(existing: longTermMemory, additions: additions),
            userPreferences: Self.mergedMemoryText(existing: userPreferences, additions: preferenceAdditions),
            memoryRecords: records,
            mcpServers: mcpServers,
            uploadedSkills: uploadedSkills,
            uploadedDocuments: uploadedDocuments
        )
        result.canonicalMemorySnapshot = canonicalMemorySnapshot
        return result
    }

    public func replacingMemoryText(
        longTermMemory newLongTermMemory: String,
        userPreferences newUserPreferences: String,
        mcpServers: [UserMCPServer],
        uploadedSkills: [UserUploadedContextItem],
        uploadedDocuments: [UserUploadedContextItem],
        provenance: ChatMemoryProvenance,
        observedAt: Date
    ) -> ChatMemoryProfile {
        let existingByKey = Dictionary(
            memoryRecords.map { (Self.logicalKey($0), $0) },
            uniquingKeysWith: { first, _ in first }
        )
        var reconciled: [ChatMemoryRecord] = []

        func append(_ value: String, namespace: ChatMemoryNamespace, kind: ChatMemoryRecordKind) {
            for text in Self.memoryLines(value) {
                let key = Self.logicalKey(namespace: namespace, kind: kind, text: text)
                if let existing = existingByKey[key] {
                    reconciled.append(existing)
                } else {
                    reconciled.append(ChatMemoryRecord(
                        id: "memory:\(UUID().uuidString)",
                        namespace: namespace,
                        kind: kind,
                        text: text,
                        confidence: nil,
                        provenance: provenance,
                        observedAt: observedAt,
                        retention: .durable
                    ))
                }
            }
        }

        append(newLongTermMemory, namespace: .longTerm, kind: .semantic)
        append(newUserPreferences, namespace: .userPreferences, kind: .procedural)
        var result = ChatMemoryProfile(
            longTermMemory: newLongTermMemory,
            userPreferences: newUserPreferences,
            memoryRecords: reconciled,
            mcpServers: mcpServers,
            uploadedSkills: uploadedSkills,
            uploadedDocuments: uploadedDocuments
        )
        result.canonicalMemorySnapshot = canonicalMemorySnapshot
        return result
    }

    private static func normalizedRecords(_ existing: [ChatMemoryRecord]) -> [ChatMemoryRecord] {
        var records: [ChatMemoryRecord] = []
        var keys = Set<String>()
        for record in existing {
            let text = record.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !record.id.isEmpty, !text.isEmpty else { continue }
            let key = logicalKey(namespace: record.namespace, kind: record.kind, text: text)
            guard keys.insert(key).inserted else { continue }
            var normalized = record
            normalized.text = text
            if normalized.confidence?.isFinite != true { normalized.confidence = nil }
            normalized.confidence = normalized.confidence.map { min(1, max(0, $0)) }
            normalized.supersedes = Array(Set(normalized.supersedes.filter { !$0.isEmpty })).sorted()
            records.append(normalized)
        }

        return records
    }

    private static func memoryLines(_ value: String) -> [String] {
        value.split(separator: "\n", omittingEmptySubsequences: true)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private static func logicalKey(_ record: ChatMemoryRecord) -> String {
        logicalKey(namespace: record.namespace, kind: record.kind, text: record.text)
    }

    private static func logicalKey(
        namespace: ChatMemoryNamespace,
        kind: ChatMemoryRecordKind,
        text: String
    ) -> String {
        "\(namespace.rawValue)|\(kind.rawValue)|\(text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())"
    }

    private static func mergedMemoryText(existing: String, additions: [String]) -> String {
        var lines = memoryLines(existing)
        var seen = Set(lines.map { $0.lowercased() })
        for addition in additions {
            let normalized = addition.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !normalized.isEmpty, seen.insert(normalized.lowercased()).inserted else { continue }
            lines.append(normalized)
        }
        return lines.joined(separator: "\n")
    }
}

nonisolated public enum ChatMemoryProfileStoreError: LocalizedError {
    case textTooLong(field: String, maxCharacters: Int)

    public var errorDescription: String? {
        switch self {
        case .textTooLong(let field, let maxCharacters):
            return "\(field) 不能超过 \(maxCharacters) 个字符。"
        }
    }
}

nonisolated public protocol ChatMemoryProfileStore: AnyObject {
    func loadProfile() -> ChatMemoryProfile
    @discardableResult
    func saveMemoryText(longTermMemory: String, userPreferences: String, expected: ChatMemoryProfile) throws -> ChatMemoryProfile
}

nonisolated protocol ChatSkillEditingStore: AnyObject {
    func loadSkillEditor(id: UUID) throws -> IOSCanonicalSkillEditor
    func saveSkill(_ item: UserUploadedContextItem, expected: IOSCanonicalSkillEditor?) throws -> IOSCanonicalSkillEditor
    func setSkillEnabled(_ enabled: Bool, expected: IOSCanonicalSkillEditor) throws -> IOSCanonicalSkillEditor
    func deleteSkill(expected: IOSCanonicalSkillEditor) throws -> Bool
}

nonisolated struct ChatCapabilityLibrarySnapshot {
    let uploadedSkills: [UserUploadedContextItem]
    let mcpServers: [UserMCPServer]
    static let empty = ChatCapabilityLibrarySnapshot(uploadedSkills: [], mcpServers: [])
}

nonisolated protocol ChatCapabilityLibraryStore: AnyObject {
    func loadCapabilityLibrary() throws -> ChatCapabilityLibrarySnapshot
}

nonisolated protocol ChatMCPEditingStore: AnyObject {
    func loadMCPEditor(id: UUID) throws -> IOSCanonicalMCPServer
    func saveMCP(_ item: UserMCPServer, expected: IOSCanonicalMCPServer?) throws -> IOSCanonicalMCPServer
    func setMCPEnabled(_ enabled: Bool, expected: IOSCanonicalMCPServer) throws -> IOSCanonicalMCPServer
    func deleteMCP(expected: IOSCanonicalMCPServer) throws -> Bool
}

nonisolated public final class CanonicalChatMemoryProfileStore: ChatMemoryProfileStore, ChatSkillEditingStore, ChatMCPEditingStore, ChatCapabilityLibraryStore {
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private let queue: DispatchQueue
    private let uploadedDocumentsKey = "assistant.uploaded_documents"
    private let maxCharacters = 6_000
    private let applicationRuntime: IOSApplicationRuntime

    public convenience init() {
        guard let store = IOSApplicationRuntime.shared else {
            preconditionFailure("Nola Application runtime is unavailable")
        }
        self.init(applicationRuntime: store)
    }

    init(applicationRuntime: IOSApplicationRuntime) {
        self.encoder = JSONEncoder()
        self.decoder = JSONDecoder()
        self.queue = DispatchQueue(label: "com.nola.chat.memoryProfile")
        self.applicationRuntime = applicationRuntime
    }

    public func loadProfile() -> ChatMemoryProfile {
        queue.sync {
            canonicalProjection()
        }
    }

    func loadCapabilityLibrary() throws -> ChatCapabilityLibrarySnapshot {
        try queue.sync {
            let skills = try applicationRuntime.listSkills().filter { $0.source != "built_in" }.map { record in
                guard let item = record.uploadedContextItem else {
                    throw IOSApplicationRuntimeError.rejected("Cannot decode Skill: \(record.id)")
                }
                return item
            }
            let servers = try applicationRuntime.listMCPServers().map { record in
                guard let item = record.userMCPServer else {
                    throw IOSApplicationRuntimeError.rejected("Cannot decode MCP configuration: \(record.id)")
                }
                return item
            }
            return ChatCapabilityLibrarySnapshot(uploadedSkills: skills, mcpServers: servers)
        }
    }

    func loadMCPEditor(id: UUID) throws -> IOSCanonicalMCPServer {
        try queue.sync {
            guard let record = try applicationRuntime.listMCPServers().first(where: { $0.id == id.uuidString.lowercased() }) else {
                throw IOSApplicationRuntimeError.rejected("MCP server is missing or deleted")
            }
            return record
        }
    }

    @discardableResult
    func saveMCP(_ item: UserMCPServer, expected: IOSCanonicalMCPServer?) throws -> IOSCanonicalMCPServer {
        try queue.sync {
            guard !item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let url = URL(string: item.endpoint), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
                  url.host != nil, let proposed = item.canonicalMCPServer else {
                throw IOSApplicationRuntimeError.rejected("Invalid MCP server configuration")
            }
            guard let expected else { return try applicationRuntime.createMCPServer(proposed) }
            guard var configuration = try JSONSerialization.jsonObject(with: Data(expected.configurationJSON.utf8)) as? [String: Any] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            configuration["name"] = item.name
            configuration["endpoint"] = item.endpoint
            configuration["description"] = item.description
            configuration["enabled"] = expected.enabled
            let data = try JSONSerialization.data(withJSONObject: configuration, options: [.sortedKeys])
            let input = IOSCanonicalMCPServer(id: proposed.id, displayName: item.name, enabled: expected.enabled,
                configurationJSON: String(decoding: data, as: UTF8.self), revision: expected.revision,
                createdAtMs: expected.createdAtMs, updatedAtMs: proposed.updatedAtMs, deletedAtMs: nil)
            return try applicationRuntime.configureMCPServer(input, expected: expected)
        }
    }

    @discardableResult
    func setMCPEnabled(_ enabled: Bool, expected: IOSCanonicalMCPServer) throws -> IOSCanonicalMCPServer {
        try queue.sync {
            let input = IOSCanonicalMCPServer(id: expected.id, displayName: expected.displayName, enabled: enabled,
                configurationJSON: expected.configurationJSON, revision: expected.revision,
                createdAtMs: expected.createdAtMs, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000), deletedAtMs: nil)
            return try applicationRuntime.configureMCPServer(input, expected: expected)
        }
    }

    @discardableResult
    func deleteMCP(expected: IOSCanonicalMCPServer) throws -> Bool {
        try queue.sync {
            try applicationRuntime.deleteMCPServer(expected: expected, deletedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        }
    }

    func loadSkillEditor(id: UUID) throws -> IOSCanonicalSkillEditor {
        try queue.sync {
            guard let snapshot = try applicationRuntime.skillEditor(id: id.uuidString.lowercased()),
                  snapshot.record.deletedAtMs == nil else {
                throw IOSApplicationRuntimeError.rejected("Skill is missing or deleted")
            }
            return snapshot
        }
    }

    @discardableResult
    func saveSkill(_ item: UserUploadedContextItem, expected: IOSCanonicalSkillEditor?) throws -> IOSCanonicalSkillEditor {
        try queue.sync {
            guard !item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  !item.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw IOSApplicationRuntimeError.rejected("Skill name and instructions are required")
            }
            guard item.content.count <= maxCharacters * 8 else {
                throw ChatMemoryProfileStoreError.textTooLong(field: "Skill", maxCharacters: maxCharacters * 8)
            }
            let proposed = item.canonicalSkill
            guard let expected else { return try applicationRuntime.createUserSkill(proposed) }
            let current = expected.record
            let input = IOSCanonicalSkill(
                id: proposed.id, source: current.source,
                version: proposed.instructionsDigest == current.instructionsDigest ? current.version : UUID().uuidString,
                displayName: proposed.displayName, summary: proposed.summary, category: current.category,
                instructionsRef: current.instructionsRef, instructionsDigest: proposed.instructionsDigest,
                instructionsText: proposed.instructionsText, requiredTools: current.requiredTools,
                defaultMode: current.defaultMode, enabled: current.enabled,
                createdAtMs: current.createdAtMs, updatedAtMs: proposed.updatedAtMs, deletedAtMs: nil
            )
            return try applicationRuntime.configureUserSkill(input, expected: expected)
        }
    }

    @discardableResult
    func setSkillEnabled(_ enabled: Bool, expected: IOSCanonicalSkillEditor) throws -> IOSCanonicalSkillEditor {
        try queue.sync {
            try applicationRuntime.setSkillEnabled(enabled, expected: expected,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        }
    }

    @discardableResult
    func deleteSkill(expected: IOSCanonicalSkillEditor) throws -> Bool {
        try queue.sync {
            try applicationRuntime.deleteUserSkill(expected: expected,
                deletedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        }
    }

    @discardableResult
    public func saveMemoryText(longTermMemory: String, userPreferences: String, expected profile: ChatMemoryProfile) throws -> ChatMemoryProfile {
        try queue.sync {
            guard let expected = profile.canonicalMemorySnapshot else {
                throw IOSApplicationRuntimeError.rejected("Reload the memory profile before saving")
            }
            // A memory editor has no authority to replace capability or document collections.
            let memoryOnly = ChatMemoryProfile(
                longTermMemory: profile.longTermMemory,
                userPreferences: profile.userPreferences,
                memoryRecords: profile.memoryRecords
            ).replacingMemoryText(
                longTermMemory: longTermMemory,
                userPreferences: userPreferences,
                mcpServers: [],
                uploadedSkills: [],
                uploadedDocuments: [],
                provenance: ChatMemoryProvenance(source: "manual_profile_edit"),
                observedAt: Date()
            )
            let normalized = try Self.validated(memoryOnly, maxCharacters: maxCharacters)
            var desired = try normalized.memoryRecords.map { record in
                guard let candidate = record.canonicalRecord else {
                    throw IOSApplicationRuntimeError.rejected("Invalid memory record: \(record.id)")
                }
                return candidate
            }
            // This editor cannot delete namespaces it does not project.
            desired += expected.filter { $0.chatMemoryRecord == nil }.map {
                IOSCanonicalMemoryCandidate(record: $0.record, quality: nil, proposedAtMs: $0.updatedAtMs)
            }
            let committed = try applicationRuntime.replaceMemories(expected: expected, candidates: desired,
                deletedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
            // A memory receipt is not a refresh of unrelated capability collections.
            var result = profile
            result.memoryRecords = committed.compactMap(\.chatMemoryRecord)
            result.longTermMemory = result.memoryRecords.filter { $0.namespace == .longTerm }.map(\.text).joined(separator: "\n")
            result.userPreferences = result.memoryRecords.filter { $0.namespace == .userPreferences }.map(\.text).joined(separator: "\n")
            result.canonicalMemorySnapshot = committed
            return result
        }
    }

    private func canonicalProjection() -> ChatMemoryProfile {
        let canonicalMemories = try? applicationRuntime.listMemories()
        let records = canonicalMemories?.compactMap(\.chatMemoryRecord) ?? []
        let longTerm = records.filter { $0.namespace == .longTerm }.map(\.text).joined(separator: "\n")
        let preferences = records.filter { $0.namespace == .userPreferences }.map(\.text).joined(separator: "\n")
        let skills = (try? applicationRuntime.listSkills().compactMap(\.uploadedContextItem)) ?? []
        let servers = (try? applicationRuntime.listMCPServers().compactMap(\.userMCPServer)) ?? []
        let documents: [UserUploadedContextItem]
        if let setting = try? applicationRuntime.productSetting(key: uploadedDocumentsKey),
           let data = setting.valueJSON.data(using: .utf8),
           let decoded = try? decoder.decode([UserUploadedContextItem].self, from: data) {
            documents = decoded
        } else {
            documents = []
        }
        var result = ChatMemoryProfile(
            longTermMemory: longTerm, userPreferences: preferences, memoryRecords: records,
            mcpServers: servers, uploadedSkills: skills,
            uploadedDocuments: documents
        )
        result.canonicalMemorySnapshot = canonicalMemories
        return result
    }

    private static func normalized(_ profile: ChatMemoryProfile) -> ChatMemoryProfile {
        ChatMemoryProfile(
            longTermMemory: profile.longTermMemory.trimmingCharacters(in: .whitespacesAndNewlines),
            userPreferences: profile.userPreferences.trimmingCharacters(in: .whitespacesAndNewlines),
            memoryRecords: profile.memoryRecords,
            mcpServers: profile.mcpServers
                .filter { !$0.endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                .map {
                    UserMCPServer(
                        id: $0.id,
                        name: $0.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? "MCP Server"
                            : $0.name.trimmingCharacters(in: .whitespacesAndNewlines),
                        endpoint: $0.endpoint.trimmingCharacters(in: .whitespacesAndNewlines),
                        description: $0.description.trimmingCharacters(in: .whitespacesAndNewlines),
                        enabled: $0.enabled
                    )
                },
            uploadedSkills: normalizeUploadedItems(profile.uploadedSkills),
            uploadedDocuments: normalizeUploadedItems(profile.uploadedDocuments)
        )
    }

    private static func validated(_ profile: ChatMemoryProfile, maxCharacters: Int) throws -> ChatMemoryProfile {
        let normalized = normalized(profile)
        if normalized.longTermMemory.count > maxCharacters {
            throw ChatMemoryProfileStoreError.textTooLong(field: "长期记忆", maxCharacters: maxCharacters)
        }
        if normalized.userPreferences.count > maxCharacters {
            throw ChatMemoryProfileStoreError.textTooLong(field: "用户偏好", maxCharacters: maxCharacters)
        }
        for record in normalized.memoryRecords where record.text.count > maxCharacters {
            throw ChatMemoryProfileStoreError.textTooLong(field: "Memory record", maxCharacters: maxCharacters)
        }
        for item in normalized.uploadedSkills where item.content.count > maxCharacters * 8 {
            throw ChatMemoryProfileStoreError.textTooLong(field: "Skill: \(item.name)", maxCharacters: maxCharacters * 8)
        }
        for item in normalized.uploadedDocuments where item.content.count > maxCharacters * 8 {
            throw ChatMemoryProfileStoreError.textTooLong(field: "文档: \(item.name)", maxCharacters: maxCharacters * 8)
        }
        return normalized
    }

    private static func normalizeUploadedItems(_ items: [UserUploadedContextItem]) -> [UserUploadedContextItem] {
        items
            .filter { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map {
                UserUploadedContextItem(
                    id: $0.id,
                    name: $0.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        ? "Uploaded Context"
                        : $0.name.trimmingCharacters(in: .whitespacesAndNewlines),
                    content: $0.content.trimmingCharacters(in: .whitespacesAndNewlines),
                    description: $0.description.trimmingCharacters(in: .whitespacesAndNewlines),
                    enabled: $0.enabled
                )
            }
    }
}

nonisolated public struct ConversationContextUsage {
    nonisolated public struct ContextSuggestion: Identifiable, Equatable {
        nonisolated public enum Severity: String, Codable {
            case high
            case medium
            case low
            case info

            var priority: Int {
                switch self {
                case .high:
                    return 3
                case .medium:
                    return 2
                case .low:
                    return 1
                case .info:
                    return 0
                }
            }
        }

        public var id: String {
            "\(severity.rawValue)-\(title)-\(detail)"
        }

        public let severity: Severity
        public let title: String
        public let detail: String
        public let estimatedSavingsTokens: Int
    }

    public let estimatedTokenLimit: Int
    public let totalTokens: Int
    public let percentage: Int
    public let activeMessageTokens: Int
    public let sessionMemoryTokens: Int
    public let longTermMemoryTokens: Int
    public let userPreferencesTokens: Int
    public let mcpServersTokens: Int
    public let uploadedSkillsTokens: Int
    public let uploadedDocumentsTokens: Int
    public let supplementalTokens: Int
    public let activeMessageCount: Int
    public let archivedMessageCount: Int
    public let sessionMemory: String
    public let longTermMemory: String
    public let userPreferences: String
    public let mcpServers: [UserMCPServer]
    public let uploadedSkills: [UserUploadedContextItem]
    public let uploadedDocuments: [UserUploadedContextItem]
    public let lastCompactedAt: Date?
    public let suggestionDetails: [ContextSuggestion]
    public let suggestions: [String]
    public let coreAction: String
    public let coreArchiveMessageIDs: [UUID]
    public var cachedInputTokens: Int? = nil
    public var cacheWriteInputTokens: Int? = nil
    public var promptCacheStatus: String? = nil
    public var cacheHitRateBps: UInt16? = nil
    public var cacheReportedRequests: UInt64? = nil
    public var cacheUnreportedRequests: UInt64? = nil
    public var hasCommittedTokenTotals = false
    public var hasEstimatedBreakdown = true
    public var contextIsEstimated = false
    public var measuredRequestID: String? = nil
    public var cacheDiagnosticRequestID: String? = nil
    public var cacheDiagnosticAttemptID: String? = nil

    public static func withoutBreakdown() -> Self {
        var value = Self(estimatedTokenLimit: 0, totalTokens: 0, percentage: 0,
            activeMessageTokens: 0, sessionMemoryTokens: 0, longTermMemoryTokens: 0,
            userPreferencesTokens: 0, mcpServersTokens: 0, uploadedSkillsTokens: 0,
            uploadedDocumentsTokens: 0, supplementalTokens: 0, activeMessageCount: 0,
            archivedMessageCount: 0, sessionMemory: "", longTermMemory: "", userPreferences: "",
            mcpServers: [], uploadedSkills: [], uploadedDocuments: [], lastCompactedAt: nil,
            suggestionDetails: [], suggestions: [], coreAction: "none", coreArchiveMessageIDs: [])
        value.hasEstimatedBreakdown = false
        return value
    }

    public func replacingTokenTotals(
        usedTokens: Int,
        limitTokens: Int,
        cachedInputTokens: Int? = nil,
        cacheWriteInputTokens: Int? = nil,
        promptCacheStatus: String? = nil,
        cacheHitRateBps: UInt16? = nil,
        cacheReportedRequests: UInt64? = nil,
        cacheUnreportedRequests: UInt64? = nil,
        hasCommittedTokenTotals: Bool = true,
        contextIsEstimated: Bool = false,
        measuredRequestID: String? = nil,
        cacheDiagnosticRequestID: String? = nil,
        cacheDiagnosticAttemptID: String? = nil
    ) -> ConversationContextUsage {
        let safeUsed = max(0, usedTokens)
        let safeLimit = max(0, limitTokens)
        var result = ConversationContextUsage(
            estimatedTokenLimit: safeLimit,
            totalTokens: safeUsed,
            percentage: safeLimit > 0 ? Int(min(100, (Double(safeUsed) / Double(safeLimit) * 100).rounded())) : 0,
            activeMessageTokens: activeMessageTokens,
            sessionMemoryTokens: sessionMemoryTokens,
            longTermMemoryTokens: longTermMemoryTokens,
            userPreferencesTokens: userPreferencesTokens,
            mcpServersTokens: mcpServersTokens,
            uploadedSkillsTokens: uploadedSkillsTokens,
            uploadedDocumentsTokens: uploadedDocumentsTokens,
            supplementalTokens: supplementalTokens,
            activeMessageCount: activeMessageCount,
            archivedMessageCount: archivedMessageCount,
            sessionMemory: sessionMemory,
            longTermMemory: longTermMemory,
            userPreferences: userPreferences,
            mcpServers: mcpServers,
            uploadedSkills: uploadedSkills,
            uploadedDocuments: uploadedDocuments,
            lastCompactedAt: lastCompactedAt,
            suggestionDetails: suggestionDetails,
            suggestions: suggestions,
            coreAction: coreAction,
            coreArchiveMessageIDs: coreArchiveMessageIDs
        )
        result.cachedInputTokens = cachedInputTokens
        result.cacheWriteInputTokens = cacheWriteInputTokens
        result.promptCacheStatus = promptCacheStatus
        result.contextIsEstimated = contextIsEstimated
        result.measuredRequestID = measuredRequestID
        result.cacheDiagnosticRequestID = cacheDiagnosticRequestID
        result.cacheDiagnosticAttemptID = cacheDiagnosticAttemptID
        result.cacheHitRateBps = cacheHitRateBps
        result.cacheReportedRequests = cacheReportedRequests
        result.cacheUnreportedRequests = cacheUnreportedRequests
        result.hasCommittedTokenTotals = hasCommittedTokenTotals && safeLimit > 0
        result.hasEstimatedBreakdown = hasEstimatedBreakdown
        return result
    }
}

nonisolated public struct ContextMemorySearchHit: Identifiable, Equatable {
    nonisolated public enum Source: String {
        case sessionMemory = "session_memory"
        case compactSnapshot = "compact_snapshot"
        case archivedMessage = "archived_message"
    }

    public var id: String {
        "\(source.rawValue)-\(referenceID)-\(score)-\(createdAt?.timeIntervalSince1970 ?? 0)"
    }

    public let source: Source
    public let sourceLabel: String
    public let score: Int
    public let preview: String
    public let referenceID: String
    public let createdAt: Date?
}

public typealias ContextManager = DefaultContextManager

public final class DefaultContextManager {
    private let store: ConversationStore
    private let profileStore: ChatMemoryProfileStore
    // Optional LLM client; set by ServiceContainer after LLM is initialized
    public var llmClient: LLMClient?
    // Optional URL fetcher for conversation-scoped data sources (injected by ServiceContainer)
    public var urlFetcher: URLFetcherService?
    // Optional snapshot of other data sources (injected by ServiceContainer)
    public var dataSources: [DataSource]?
    // Optional local storage for scheduled-action run outputs (injected by ServiceContainer)
    public var localJSONStorage: LocalJSONStorage?

    init(store: ConversationStore, profileStore: ChatMemoryProfileStore) {
        self.store = store
        self.profileStore = profileStore
    }

    func recordPreparedDataSourceUsage(
        payload: CorePreparedOutboundPayload,
        conversationId: UUID,
        disclosedToCloud: Bool
    ) async {
        guard let fetcher = urlFetcher else { return }
        for sourceID in payload.usedDataSourceIDs {
            let title = fetcher.sources.first(where: { $0.id == sourceID })?.name ?? sourceID.uuidString
            fetcher.recordUsage(URLDataSourceUsageRecord(
                sourceID: sourceID,
                sourceTitle: title,
                consumer: "chat",
                purpose: "conversation_context",
                conversationID: conversationId,
                disclosedToCloud: disclosedToCloud
            ))
        }
    }
    
    private func prepareContextWithCore(
        activeMessages: [ConversationMessage],
        components: [AgentCoreContextComponent],
        tools: [AgentCoreJSONValue],
        maxTokens: Int,
        preserveRecentMessages: Int,
        mode: AgentCoreContextPrepareMode,
        existingSummary: String?,
        summaryResult: AgentCoreContextSummaryResult? = nil
    ) -> AgentCoreContextPrepareResponse? {
        let messages = activeMessages.map { message in
            AgentCoreContextPrepareMessage(
                id: message.id.uuidString,
                role: message.role.rawValue,
                content: message.text,
                ephemeral: message.meta?["ephemeral"] == "true",
                nolaEphemeral: message.meta?["_nola_ephemeral"] == "true"
            )
        }
        return AgentCoreContextEngine.contextPrepare(
            AgentCoreContextPrepareInput(
                messages: messages,
                tools: tools,
                platform: .ios,
                components: components,
                budget: AgentCoreContextPrepareBudget(
                    contextWindowTokens: maxTokens,
                    reservedOutputTokens: 4_096,
                    keepRecentMessages: preserveRecentMessages
                ),
                mode: mode,
                existingSummary: existingSummary,
                summaryResult: summaryResult
            )
        )
    }

    private func archivedMessageIDs(in conversation: Conversation) -> Set<UUID> {
        Set(conversation.compactedMemoryEntries.flatMap(\.archivedMessageIDs))
    }


    private func normalizedDataSourceText(_ text: String?, maxLength: Int) -> String? {
        guard let text else { return nil }
        let compacted = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\t", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !compacted.isEmpty else { return nil }
        if compacted.count <= maxLength {
            return compacted
        }
        return String(compacted.prefix(maxLength)) + "…"
    }

    private func readableItemsFromFetchedEvents(
        _ events: [RawEvent],
        source: URLDataSource,
        limit: Int
    ) -> [ReadableDataSourceItem] {
        events
            .prefix(max(0, limit))
            .enumerated()
            .map { (idx, event) in
                let dateText = event.date?.formatted(date: .abbreviated, time: .shortened)
                return ReadableDataSourceItem(
                    id: "fetched:\(source.id.uuidString):\(idx):\(event.id)",
                    title: event.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? source.name : event.title,
                    summary: dateText,
                    content: event.notes,
                    sourceURL: source.url,
                    metadata: [
                        "sourceId": source.id.uuidString,
                        "sourceName": source.name,
                        "kind": source.kind.rawValue,
                        "format": source.format.rawValue,
                        "origin": "live_fetch"
                    ]
                )
            }
    }

    private func scheduledRunContextItems(for source: URLDataSource, limit: Int) -> [ReadableDataSourceItem] {
        guard source.isScheduledAutomation, let storage = localJSONStorage else { return [] }
        let runs = (try? storage.getScheduledActionRuns(limit: max(limit * 4, 12), actionId: source.id.uuidString)) ?? []
        guard !runs.isEmpty else { return [] }

        var items: [ReadableDataSourceItem] = []
        for run in runs {
            var contentParts: [String] = []
            if let journalId = run.resultJournalId,
               !journalId.isEmpty,
               let journal = try? storage.getJournal(id: journalId),
               let journalContent = normalizedDataSourceText(journal.content, maxLength: 1_600) {
                contentParts.append(journalContent)
            }
            if let details = normalizedDataSourceText(run.details, maxLength: 500) {
                contentParts.append("Details: \(details)")
            }
            if let failureReason = normalizedDataSourceText(run.failureReason, maxLength: 300) {
                contentParts.append("Failure: \(failureReason)")
            } else if let errorMessage = normalizedDataSourceText(run.errorMessage, maxLength: 300) {
                contentParts.append("Error: \(errorMessage)")
            }

            let summary = [
                "status=\(run.status.rawValue)",
                run.reason.flatMap { normalizedDataSourceText($0, maxLength: 120) }.map { "reason=\($0)" },
                "started=\(run.startedAt.formatted(date: .abbreviated, time: .shortened))"
            ]
            .compactMap { $0 }
            .joined(separator: ", ")

            let titleSuffix = run.startedAt.formatted(date: .numeric, time: .shortened)
            let itemTitle = "\(source.name) scheduled run (\(titleSuffix))"
            items.append(
                ReadableDataSourceItem(
                    id: "scheduled_run:\(run.id)",
                    title: itemTitle,
                    summary: summary,
                    content: contentParts.isEmpty ? nil : contentParts.joined(separator: "\n"),
                    sourceURL: source.url,
                    metadata: [
                        "sourceId": source.id.uuidString,
                        "sourceName": source.name,
                        "kind": source.kind.rawValue,
                        "format": source.format.rawValue,
                        "origin": "scheduled_run_output",
                        "runId": run.id
                    ]
                )
            )

            if items.count >= limit {
                break
            }
        }

        return items
    }

    private func contextItems(
        for source: URLDataSource,
        fetcher: URLFetcherService,
        allowLiveFetch: Bool,
        itemLimit: Int
    ) async -> [ReadableDataSourceItem] {
        var aggregated: [ReadableDataSourceItem] = []
        var seen = Set<String>()

        for item in scheduledRunContextItems(for: source, limit: max(1, itemLimit)) {
            if seen.insert(item.id).inserted {
                aggregated.append(item)
            }
        }

        if let historyItems = try? await source.readContextItems(limit: max(1, itemLimit)) {
            for item in historyItems.reversed() {
                if seen.insert(item.id).inserted {
                    aggregated.append(item)
                }
                if aggregated.count >= itemLimit { break }
            }
        }

        if aggregated.isEmpty, allowLiveFetch {
            if let events = try? await fetcher.fetchEvents(from: source), !events.isEmpty {
                for item in readableItemsFromFetchedEvents(events, source: source, limit: max(1, itemLimit)) {
                    if seen.insert(item.id).inserted {
                        aggregated.append(item)
                    }
                    if aggregated.count >= itemLimit { break }
                }
            }
        }

        return aggregated
    }

    private func dataSourceComponents(
        for conversation: Conversation,
        allowLiveFetch: Bool
    ) async -> [AgentCoreContextComponent] {
        guard let fetcher = urlFetcher else { return [] }
        let readable: [IOSCanonicalReadableSource]
        do {
            readable = try await fetcher.canonicalContextSources(
                conversationID: conversation.id.uuidString, allowLiveFetch: allowLiveFetch
            )
        } catch {
            Logger().log(.warn, "ContextManager", "Canonical data source selection unavailable", metadata: ["error": error.localizedDescription])
            return []
        }
        return readable.compactMap { entry in
            let source = entry.source
            let items = readableItemsFromFetchedEvents(entry.events, source: source, limit: 16)
            guard !items.isEmpty else { return nil }
            var lines = ["[DataSource: \(source.name)] Source: \(source.contextDisplaySubtitle)"]
            for (index, item) in items.enumerated() {
                lines.append("\(index + 1). \(item.title)")
                if let summary = item.summary, !summary.isEmpty { lines.append("   Summary: \(summary)") }
                if let content = item.content, !content.isEmpty { lines.append("   Content: \(content)") }
            }
            return AgentCoreContextComponent(
                kind: .platformData, id: "data_source:\(source.id.uuidString)",
                content: .string(lines.joined(separator: "\n")),
                source: .init(
                    platform: .ios, uri: source.url, title: source.name,
                    observedAt: entry.selection.observedAtMs.map {
                        ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: Double($0) / 1_000))
                    },
                    authorization: entry.authorization.state.rawValue,
                    maximumAgeMinutes: UInt64(source.validationContract?.maximumAgeMinutes ?? 60)
                )
            )
        }
    }

    private func systemDataSourceComponents(for conversation: Conversation) async -> [AgentCoreContextComponent] {
        let policyStore = SystemDataSourceAccessPolicyStore.shared
        let permissionManager = DataSourcePermissionManager.shared
        let now = Date()
        let future = Calendar.current.date(byAdding: .day, value: 30, to: now) ?? now
        let recent = Calendar.current.date(byAdding: .day, value: -7, to: now) ?? now
        var components: [AgentCoreContextComponent] = []

        for type in policyStore.configurableTypes {
            let overrideKey = policyStore.conversationOverrideKey(for: type)
            guard policyStore.isEnabledForConversation(
                type: type,
                explicitOverride: conversation.dataSourceOverrides[overrideKey]
            ) else { continue }
            guard permissionManager.getPermissionStatus(for: type) == .authorized else { continue }

            let source: DataSource?
            switch type {
            case .contacts:
                source = ContactsDataSource()
            case .calendars:
                source = CalendarsDataSource()
            case .reminders:
                source = RemindersDataSource()
            case .health:
                source = dataSources?.first(where: { $0 is HealthDataSource })
            case .url:
                source = nil
            }
            guard let source else { continue }

            let range = type == .health
                ? DateInterval(start: recent, end: now)
                : DateInterval(start: now, end: future)
            guard let events = try? await source.fetchEvents(in: range), !events.isEmpty else { continue }

            var lines = ["[DataSource: \(type.displayName)]"]
            for (index, event) in events.prefix(24).enumerated() {
                var line = "\(index + 1). \(normalizedDataSourceText(event.title, maxLength: 180) ?? event.title)"
                if let date = event.date {
                    line += " · \(date.formatted(date: .abbreviated, time: .shortened))"
                }
                lines.append(line)
                if let notes = normalizedDataSourceText(event.notes, maxLength: 280) {
                    lines.append("   \(notes)")
                }
            }
            components.append(
                AgentCoreContextComponent(
                    kind: .platformData,
                    id: overrideKey,
                    content: .string(lines.joined(separator: "\n")),
                    source: .init(
                        platform: .ios,
                        uri: "nola://system-data/\(type.rawValue)",
                        title: type.displayName,
                        authorization: "authorized",
                        maximumAgeMinutes: type == .health ? 60 : 15
                    )
                )
            )
        }

        return components
    }

    private func uploadedItemsSummary(_ items: [UserUploadedContextItem]) -> String {
        items
            .filter { $0.enabled && !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map { item in
                "## \(item.name)\n\(item.content.trimmingCharacters(in: .whitespacesAndNewlines))"
            }
            .joined(separator: "\n\n")
    }

    private func summarizeMaintenance(plan: IOSContextMaintenancePlan, conversation: Conversation,
                                      store: LocalConversationStore) async throws -> String {
        guard let client = llmClient, let archive = plan.output.archiveRequest,
              let instructions = plan.instructions, let window = plan.input.budget.contextWindowTokens else {
            throw ContextOutboundPreparationError.coreUnavailable
        }
        let captured=(client as? ModelService)?.captureAuxiliaryProvider()
        var provider:LLMClient = captured?.provider ?? client
        if let remote=provider as? WebAgentProvider {provider=try await remote.freezeAuxiliaryModel()}
        let profile:IOSCanonicalModelProfile? = try conversation.selectedModelInfo.flatMap { selected in
            let value=try store.executeRemoteJournalCommand(.object(["type":.string("model"),
                "command":.object(["type":.string("get_profile"),"id":.string(selected.id)])]))
            return try JSONDecoder().decode(IOSCanonicalModelProfile?.self,from:JSONEncoder().encode(value))
        }
        if let physical=provider as? OpenAIProvider,let profile {
            guard profile.modelName==physical.auxiliaryModelIdentity else {throw ContextOutboundPreparationError.coreUnavailable}
        }
        let frozenRevision=profile.map {UInt64(max(0,$0.revision))} ?? captured?.revision ?? 0
        func field(_ object: [String: AgentCoreJSONValue], _ key: String) -> String? {
            if case .string(let value)? = object[key] { return value }; return nil
        }
        func step(_ value: [String: AgentCoreJSONValue]) throws -> [String: AgentCoreJSONValue] {
            guard case .object(let result) = try store.executeRemoteJournalCommand(.object([
                "type": .string("step_compaction_summary"), "step": .object(value)
            ])) else { throw ContextOutboundPreparationError.coreUnavailable }
            return result
        }
        var source = try JSONDecoder().decode([AgentCoreJSONValue].self, from: JSONEncoder().encode(archive.messages))
        if let previous = archive.existingSummary, !previous.isEmpty {
            source.insert(.object(["role": .string("assistant"), "content": .string(previous)]), at: 0)
        }
        var stage: [String: AgentCoreJSONValue]
        do {
            stage = try step(["type": .string("start"), "messages": .array(source),
                "context_window_tokens": .signedInteger(Int64(window)),
                "output_tokens": .signedInteger(Int64(plan.input.budget.reservedOutputTokens)),
                "target_tokens": .signedInteger(Int64(archive.targetSummaryTokens)), "instructions": .string(instructions)])
        } catch IOSApplicationRuntimeError.rejected {
            // A pure plan can refuse an insufficient summary window. Nothing
            // was sent to a model or committed; retain the original context.
            throw ContextOutboundPreparationError.cannotFit
        }
        var correction = ""
        var rewrites = 0
        while true {
            try Task.checkCancellation()
            if provider is LocalProvider, let service=client as? ModelService,
               service.captureAuxiliaryProvider().revision != captured?.revision {throw ContextOutboundPreparationError.coreUnavailable}
            guard field(stage, "action") == "request", let state = stage["state"],
                  case .array(let messages)? = stage["messages"],
                  case .object(let system)? = messages.first,
                  case .object(let user)? = messages.last,
                  let systemText = field(system, "content"), let sourceText = field(user, "content"),
                  let target = stage["target_tokens"] else { throw ContextOutboundPreparationError.coreUnavailable }
            let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: [
                "purpose": "context_maintenance", "custom_system_prompt": systemText + correction,
                "message": sourceText, "enable_thinking": "false"
            ])
            let invocation = LLMSummaryInvocation(ownerID: conversation.id.uuidString, configurationRevision:frozenRevision, provider: provider,
                scope: .conversation(conversation.id), scenario: .contextSummary)
            let receipt: LLMSummaryPhysicalReceipt
            do {
                receipt = try await store.bindAuxiliaryInvocation(invocation).generateReceipt(input: input, timeout: 30)
            } catch {
                let failure = (error as? LLMSummaryPhysicalFailure)?.underlying ?? error
                if case NolaError.llm(.providerFailure(let code, _, _)) = failure, code == "context_length_exceeded" {
                    stage = try step(["type": .string("split"), "state": state]); correction = ""; rewrites = 0; continue
                }
                throw error
            }
            try Task.checkCancellation()
            let summary = receipt.output.explain ?? receipt.output.items.map { $0.text }.joined(separator: "\n")
            let reason = receipt.finishReason ?? "unknown"
            guard case .object(let decision) = try store.executeRemoteJournalCommand(.object([
                "type": .string("evaluate_compaction_maintenance"), "facts": .object([
                    "summary": .string(summary), "target_tokens": target,
                    "output_limited": .bool(isModelOutputLimitFinishReason(reason)), "has_tool_calls": .bool(receipt.hasToolCalls ?? true),
                    "completed": .bool(["stop", "end_turn", "completed"].contains(reason)), "cancelled": .bool(false),
                    "continuation_count": .signedInteger(0), "rewrite_count": .signedInteger(Int64(rewrites)),
                    "can_continue": .bool(false), "can_rewrite": .bool(true)
                ])
            ])) else { throw ContextOutboundPreparationError.coreUnavailable }
            switch field(decision, "action") {
            case "rewrite":
                guard let instruction = field(decision, "instructions") else { throw ContextOutboundPreparationError.coreUnavailable }
                correction = "\n\n" + instruction; rewrites += 1
            case "accept":
                stage = try step(["type": .string("accept"), "state": state, "summary": .string(summary)])
                if field(stage, "action") == "complete", let result = field(stage, "summary") { return result }
                correction = ""; rewrites = 0
            default:
                throw NolaError.invalidInput(reason: field(decision, "code") ?? "Context maintenance response was not confirmed complete")
            }
        }
    }

    /// Durable memory remains evidence, not a user instruction component.
    static func durableMemoryComponent(id: String, content: String,
        source: AgentCoreContextComponentSource? = nil) -> AgentCoreContextComponent {
        AgentCoreContextComponent(kind: .memory, id: id, content: .string(content), source: source)
    }

    private func coreComponents(
        conversation: Conversation,
        profile: ChatMemoryProfile,
        allowLiveFetch: Bool
    ) async throws -> [AgentCoreContextComponent] {
        guard PersonalIntelligenceCore.isMemorySelectionAvailable else {
            throw ContextOutboundPreparationError.coreUnavailable
        }
        var components: [AgentCoreContextComponent] = []

        func append(_ kind: AgentCoreContextComponentKind, _ id: String, _ value: String) {
            let content = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !content.isEmpty else { return }
            components.append(
                AgentCoreContextComponent(
                    kind: kind,
                    id: id,
                    content: .string(content),
                    source: .init(platform: .ios)
                )
            )
        }

        if PersonalIntelligenceCore.isMemorySelectionAvailable {
            let formatter = ISO8601DateFormatter()
            let records = profile.activeMemoryRecords.map { memory in
                let provenanceKind: CoreMemoryProvenanceKind = switch memory.provenance.source {
                case "manual_profile_edit": .explicitUser
                case "assistant_memory_tool": .agentInference
                default: .other
                }
                let retention: CoreMemoryRetention = switch memory.retention {
                case .durable: .permanent
                case .session, .ephemeral: .session
                }
                return CoreMemoryRecord(
                    id: memory.id,
                    owner: memory.owner,
                    namespace: .personal,
                    scope: .global,
                    kind: memory.kind == .procedural ? .preference : .reference,
                    content: memory.text,
                    tags: [memory.namespace.rawValue, memory.kind.rawValue],
                    confidence: memory.confidence ?? 0.6,
                    importance: 0.5,
                    sensitivity: memory.sensitivity,
                    retention: retention,
                    supersedes: memory.supersedes,
                    provenance: CoreMemoryProvenance(
                        kind: provenanceKind,
                        source: memory.provenance.sourceID ?? "nola://memory/\(memory.id)",
                        sessionID: memory.provenance.conversationID?.uuidString,
                        turnID: memory.provenance.messageID?.uuidString,
                        observedAt: memory.observedAt.map { formatter.string(from: $0) }
                    )
                )
            }
            guard let plan = PersonalIntelligenceCore.selectMemories(
                CoreMemorySelectionInput(
                    policy: CoreMemoryPolicy(
                        longTermEnabled: true,
                        automaticExtractionEnabled: true,
                        searchLimit: 20,
                        minimumConfidence: 0.5,
                        maximumAgeDays: 365
                    ),
                    records: records,
                    mode: .residentProfile,
                    allowedNamespaces: [.personal],
                    includeSessionRetention: false,
                    now: formatter.string(from: Date()),
                    maximumInputTokens: 1_024,
                    tokenAccounting: .coreEstimate
                )
            ) else { throw ContextOutboundPreparationError.coreUnavailable }
            let selected = plan.selected
            for memory in selected {
                let content = memory.requiresCurrentStateVerification
                    ? memory.record.content + "\n\n[Verify mutable current state before relying on this memory.]"
                    : memory.record.content
                components.append(
                    Self.durableMemoryComponent(
                        id: "profile:memory:\(memory.record.id)",
                        content: content,
                        source: .init(
                            platform: .ios,
                            uri: memory.record.provenance.source,
                            title: "Long-term memory",
                            observedAt: memory.record.provenance.observedAt,
                            authorization: "internal"
                        )
                    )
                )
            }
        }
        append(.sessionMemory, "conversation:\(conversation.id.uuidString):session_memory", conversation.sessionMemory)
        for item in profile.uploadedSkills where item.enabled {
            let declaredSummary = item.description.trimmingCharacters(in: .whitespacesAndNewlines)
            let summary = declaredSummary.isEmpty
                ? "User-provided instructions for \(item.name). Load this Skill before answering requests that refer to it."
                : declaredSummary
            let metadata: [String: Any] = [
                "schema_version": 1,
                "type": "available_skill",
                "name": item.name,
                "description": String(summary.prefix(512)),
                "invocation": [
                    "tool": "skill_invoke",
                    "arguments": ["skill": item.name]
                ],
                "usage_contract": "This entry is metadata, not the Skill instructions. If this Skill is selected, call invocation.tool with invocation.arguments before answering. Use the returned invoked_skill instructions for the task, and do not claim the Skill was used unless that call succeeds."
            ]
            if let data = try? JSONSerialization.data(withJSONObject: metadata, options: [.sortedKeys]),
               let encoded = String(data: data, encoding: .utf8) {
                append(.skill, "skill:\(item.id)", encoded)
            }
        }
        for item in profile.uploadedDocuments where item.enabled {
            append(.document, "document:\(item.id)", item.content)
        }
        components.append(contentsOf: await dataSourceComponents(
            for: conversation,
            allowLiveFetch: allowLiveFetch
        ))
        components.append(contentsOf: await systemDataSourceComponents(for: conversation))
        if let language = conversation.language {
            append(.system, "conversation:\(conversation.id.uuidString):language", "Conversation language: \(language)")
        }
        return components
    }

    private func buildOutboundPayload(
        for conversationId: UUID,
        maxTokens: Int,
        preserveRecentMessages: Int,
        additionalComponents: [AgentCoreContextComponent],
        tools: [AgentCoreJSONValue]
    ) async throws -> CorePreparedOutboundPayload {
        let messages = try store.getMessages(conversationId: conversationId, limit: 0)
        guard let conversation = try store.getConversation(conversationId: conversationId) else {
            throw ContextOutboundPreparationError.cannotFit
        }
        let profile = profileStore.loadProfile()
        let components = try await coreComponents(
            conversation: conversation,
            profile: profile,
            allowLiveFetch: true
        ) + additionalComponents
        let archivedMessageIDs = archivedMessageIDs(in: conversation)
        let activeMessages = messages.filter { !archivedMessageIDs.contains($0.id) }
        guard let prepared = prepareContextWithCore(
            activeMessages: activeMessages,
            components: components,
            tools: tools,
            maxTokens: maxTokens,
            preserveRecentMessages: preserveRecentMessages,
            mode: .normal,
            existingSummary: conversation.sessionMemory
        ) else {
            throw ContextOutboundPreparationError.coreUnavailable
        }
        return try CorePreparedOutboundPayload(
            response: prepared,
            contextComponents: components
        )
    }

    public func prepareOutboundContext(
        for conversationId: UUID,
        maxTokens: Int,
        preserveRecentMessages: Int = 8
    ) async throws -> [PromptSegment] {
        try await prepareOutboundPayload(
            for: conversationId,
            maxTokens: maxTokens,
            preserveRecentMessages: preserveRecentMessages,
            additionalComponents: [],
            tools: []
        ).segments
    }

    public func prepareOutboundPayload(
        for conversationId: UUID,
        maxTokens: Int,
        preserveRecentMessages: Int = 8,
        additionalComponents: [AgentCoreContextComponent],
        tools: [AgentCoreJSONValue]
    ) async throws -> CorePreparedOutboundPayload {
        do {
            return try await buildOutboundPayload(
                for: conversationId,
                maxTokens: maxTokens,
                preserveRecentMessages: preserveRecentMessages,
                additionalComponents: additionalComponents,
                tools: tools
            )
        } catch ContextOutboundPreparationError.summaryStillRequired {
            _ = try await compactConversationCore(
                for: conversationId,
                maxTokens: maxTokens,
                preserveRecentMessages: preserveRecentMessages,
                force: false,
                additionalComponents: additionalComponents,
                tools: tools
            )
            return try await buildOutboundPayload(
                for: conversationId,
                maxTokens: maxTokens,
                preserveRecentMessages: preserveRecentMessages,
                additionalComponents: additionalComponents,
                tools: tools
            )
        }
    }

    static func outboundPromptSegments(
        from prepared: AgentCoreContextPrepareResponse
    ) throws -> [PromptSegment] {
        switch prepared.action {
        case "ready":
            return prepared.messages.map {
                PromptSegment(role: $0.role, content: $0.content)
            }
        case "summary_required":
            throw ContextOutboundPreparationError.summaryStillRequired
        case "cannot_fit":
            throw ContextOutboundPreparationError.cannotFit
        default:
            throw ContextOutboundPreparationError.invalidCoreAction(prepared.action)
        }
    }

    public func buildContext(for conversationId: UUID, maxTokens: Int) async throws -> [PromptSegment] {
        try await buildOutboundPayload(
            for: conversationId,
            maxTokens: maxTokens,
            preserveRecentMessages: 8,
            additionalComponents: [],
            tools: []
        ).segments
    }

    private func buildContextSuggestionDetails(
        percentage: Int,
        totalTokens: Int,
        tokenLimit: Int,
        activeMessageTokens: Int,
        activeMessageCount: Int,
        sessionMemoryTokens: Int,
        longTermMemoryTokens: Int,
        userPreferencesTokens: Int,
        mcpServersTokens: Int,
        uploadedSkillsTokens: Int,
        uploadedDocumentsTokens: Int,
        supplementalTokens: Int,
        archivedMessageCount: Int
    ) -> [ConversationContextUsage.ContextSuggestion] {
        let safeTotalTokens = max(1, totalTokens)
        let uploadedContextTokens = uploadedSkillsTokens + uploadedDocumentsTokens
        let activeShare = Int(round((Double(activeMessageTokens) / Double(safeTotalTokens)) * 100.0))
        let supplementalShare = Int(round((Double(supplementalTokens) / Double(safeTotalTokens)) * 100.0))

        var suggestions: [ConversationContextUsage.ContextSuggestion] = []
        var seenTitles = Set<String>()

        func addSuggestion(
            _ severity: ConversationContextUsage.ContextSuggestion.Severity,
            _ title: String,
            _ detail: String,
            _ estimatedSavingsTokens: Int
        ) {
            guard !seenTitles.contains(title) else { return }
            seenTitles.insert(title)
            suggestions.append(
                ConversationContextUsage.ContextSuggestion(
                    severity: severity,
                    title: title,
                    detail: detail,
                    estimatedSavingsTokens: max(0, estimatedSavingsTokens)
                )
            )
        }

        if percentage >= 90 {
            addSuggestion(
                .high,
                "上下文即将触顶",
                "当前约 \(totalTokens)/\(tokenLimit) tokens。建议先 compact，再继续长链路推理。",
                max(1_500, Int(Double(activeMessageTokens) * 0.35))
            )
        } else if percentage >= 75 {
            addSuggestion(
                .medium,
                "上下文已进入高水位",
                "当前约 \(totalTokens)/\(tokenLimit) tokens。建议提前 compact，避免后续请求触发截断。",
                max(1_000, Int(Double(activeMessageTokens) * 0.28))
            )
        }

        if activeMessageTokens >= 4_000 && activeShare >= 45 {
            addSuggestion(
                .medium,
                "对话消息占用偏高",
                "Active messages 占上下文约 \(activeShare)%（\(activeMessageCount) 条），可折叠较早轮次。",
                max(900, Int(Double(activeMessageTokens) * 0.25))
            )
        }

        if sessionMemoryTokens >= 5_000 {
            addSuggestion(
                .medium,
                "Session memory 过长",
                "建议删除已完成阶段和重复约束，保留事实、决策和未完成事项。",
                max(500, Int(Double(sessionMemoryTokens) * 0.2))
            )
        }

        if uploadedContextTokens >= 4_000 {
            addSuggestion(
                .medium,
                "上传的 Skill/文档占用较高",
                "建议只启用当前任务需要的条目，其他内容先停用。",
                max(600, Int(Double(uploadedContextTokens) * 0.35))
            )
        }

        if longTermMemoryTokens >= 2_000 {
            addSuggestion(
                .low,
                "长期记忆可以进一步收敛",
                "仅保留跨会话稳定事实，避免把一次性目标写入长期记忆。",
                max(200, Int(Double(longTermMemoryTokens) * 0.15))
            )
        }

        if userPreferencesTokens >= 1_500 {
            addSuggestion(
                .low,
                "用户偏好建议精简",
                "保留少量稳定输出风格规则，临时偏好放入当前对话即可。",
                max(150, Int(Double(userPreferencesTokens) * 0.2))
            )
        }

        if mcpServersTokens >= 2_500 {
            addSuggestion(
                .low,
                "MCP 描述信息偏长",
                "可缩短 endpoint 描述，只保留调用所需关键信息。",
                max(200, Int(Double(mcpServersTokens) * 0.2))
            )
        }

        if supplementalTokens >= 3_000 && supplementalShare >= 20 {
            addSuggestion(
                .info,
                "系统附加上下文占比偏高",
                "附加上下文约占 \(supplementalShare)%，可按需减少数据源注入范围。",
                0
            )
        }

        if archivedMessageCount > 0 {
            addSuggestion(
                .info,
                "历史消息已折叠",
                "已有 \(archivedMessageCount) 条较早消息从模型上下文移除，但仍保留在本地历史。",
                0
            )
        }

        if suggestions.isEmpty && percentage <= 55 {
            addSuggestion(
                .info,
                "当前上下文状态良好",
                "暂不需要 compact，可继续当前工作流。",
                0
            )
        }

        return suggestions.sorted {
            if $0.severity.priority != $1.severity.priority {
                return $0.severity.priority > $1.severity.priority
            }
            return $0.estimatedSavingsTokens > $1.estimatedSavingsTokens
        }
    }

    private func memoryQueryTerms(_ query: String) -> [String] {
        let normalized = query
            .lowercased()
            .split(whereSeparator: { $0.isWhitespace })
            .map { String($0) }
            .filter { !$0.isEmpty }
        return Array(Set(normalized)).sorted { $0.count > $1.count }
    }

    private func memoryMatchScore(text: String, query: String, terms: [String]) -> Int {
        let candidate = text.lowercased()
        guard !candidate.isEmpty else { return 0 }
        var score = 0
        if !query.isEmpty, candidate.contains(query) {
            score += 8
        }
        for term in terms where term.count >= 2 {
            let count = candidate.components(separatedBy: term).count - 1
            if count > 0 {
                score += min(6, count * 2)
            }
        }
        return max(0, score)
    }

    private func memoryPreview(text: String, query: String, terms: [String], maxChars: Int = 220) -> String {
        let source = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\t", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !source.isEmpty else { return "" }
        let lower = source.lowercased()
        var pivot = -1
        if !query.isEmpty {
            pivot = lower.range(of: query)?.lowerBound.utf16Offset(in: lower) ?? -1
        }
        if pivot < 0 {
            for term in terms where term.count >= 2 {
                if let range = lower.range(of: term) {
                    pivot = range.lowerBound.utf16Offset(in: lower)
                    break
                }
            }
        }
        if pivot < 0 {
            return source.count > maxChars ? String(source.prefix(maxChars)) + "…" : source
        }
        let half = max(40, maxChars / 2)
        let start = max(0, pivot - half)
        let end = min(source.count, start + maxChars)
        let startIndex = source.index(source.startIndex, offsetBy: start)
        let endIndex = source.index(source.startIndex, offsetBy: end)
        var snippet = String(source[startIndex..<endIndex])
        if start > 0 {
            snippet = "…" + snippet
        }
        if end < source.count {
            snippet += "…"
        }
        return snippet
    }

    public func searchRawMemory(
        for conversationId: UUID,
        query: String,
        limit: Int = 12
    ) async throws -> [ContextMemorySearchHit] {
        guard let conversation = try store.getConversation(conversationId: conversationId) else {
            return []
        }
        let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let terms = memoryQueryTerms(normalized)
        let hasQuery = !normalized.isEmpty
        let safeLimit = max(1, min(limit, 50))

        let messages = try store.getMessages(conversationId: conversationId, limit: 0)
        var hits: [ContextMemorySearchHit] = []

        func appendHit(
            source: ContextMemorySearchHit.Source,
            sourceLabel: String,
            text: String,
            createdAt: Date?,
            score: Int,
            referenceID: String
        ) {
            let preview = memoryPreview(text: text, query: normalized, terms: terms)
            guard !preview.isEmpty else { return }
            hits.append(
                ContextMemorySearchHit(
                    source: source,
                    sourceLabel: sourceLabel,
                    score: max(0, score),
                    preview: preview,
                    referenceID: referenceID,
                    createdAt: createdAt
                )
            )
        }

        let sessionText = conversation.sessionMemory.trimmingCharacters(in: .whitespacesAndNewlines)
        let sessionScore = hasQuery ? memoryMatchScore(text: sessionText, query: normalized, terms: terms) : 0
        if !sessionText.isEmpty && (sessionScore > 0 || !hasQuery) {
            appendHit(
                source: .sessionMemory,
                sourceLabel: "Session memory",
                text: sessionText,
                createdAt: conversation.lastCompactedAt ?? conversation.lastActiveAt,
                score: sessionScore,
                referenceID: conversation.id.uuidString
            )
        }

        for entry in conversation.compactedMemoryEntries.reversed() {
            let combinedText = [entry.summary, entry.rawExcerpt]
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
                .joined(separator: "\n")
            guard !combinedText.isEmpty else { continue }
            let score = hasQuery ? memoryMatchScore(text: combinedText, query: normalized, terms: terms) : 0
            if score > 0 || !hasQuery {
                appendHit(
                    source: .compactSnapshot,
                    sourceLabel: "Compact 原始快照",
                    text: combinedText,
                    createdAt: entry.createdAt,
                    score: score,
                    referenceID: entry.id.uuidString
                )
            }
        }

        let archivedMessageIDs = archivedMessageIDs(in: conversation)
        for msg in messages where archivedMessageIDs.contains(msg.id) {
            let content = msg.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !content.isEmpty else { continue }
            let score = hasQuery ? memoryMatchScore(text: content, query: normalized, terms: terms) : 0
            if score > 0 {
                appendHit(
                    source: .archivedMessage,
                    sourceLabel: "已折叠消息",
                    text: content,
                    createdAt: msg.createdAt,
                    score: score,
                    referenceID: msg.id.uuidString
                )
            }
        }

        return hits
            .sorted {
                if $0.score != $1.score {
                    return $0.score > $1.score
                }
                return ($0.createdAt ?? .distantPast) > ($1.createdAt ?? .distantPast)
            }
            .prefix(safeLimit)
            .map { $0 }
    }

    public func estimateContextUsage(for conversationId: UUID, maxTokens: Int) async throws -> ConversationContextUsage {
        let messages = try store.getMessages(conversationId: conversationId, limit: 0)
        guard let conversation = try store.getConversation(conversationId: conversationId) else {
            return ConversationContextUsage(
                estimatedTokenLimit: maxTokens,
                totalTokens: 0,
                percentage: 0,
                activeMessageTokens: 0,
                sessionMemoryTokens: 0,
                longTermMemoryTokens: 0,
                userPreferencesTokens: 0,
                mcpServersTokens: 0,
                uploadedSkillsTokens: 0,
                uploadedDocumentsTokens: 0,
                supplementalTokens: 0,
                activeMessageCount: 0,
                archivedMessageCount: 0,
                sessionMemory: "",
                longTermMemory: "",
                userPreferences: "",
                mcpServers: [],
                uploadedSkills: [],
                uploadedDocuments: [],
                lastCompactedAt: nil,
                suggestionDetails: [],
                suggestions: [],
                coreAction: "ready",
                coreArchiveMessageIDs: []
            )
        }

        let profile = profileStore.loadProfile()
        let archivedMessageIDs = archivedMessageIDs(in: conversation)
        let activeMessages = messages.filter { !archivedMessageIDs.contains($0.id) }
        let archivedMessages = messages.filter { archivedMessageIDs.contains($0.id) }
        let usageComponents = try await coreComponents(
            conversation: conversation,
            profile: profile,
            allowLiveFetch: false
        )
        let prepared = prepareContextWithCore(
            activeMessages: activeMessages,
            components: usageComponents,
            tools: [],
            maxTokens: maxTokens,
            preserveRecentMessages: 8,
            mode: .normal,
            existingSummary: conversation.sessionMemory
        )
        guard let prepared else {
            throw ContextOutboundPreparationError.coreUnavailable
        }
        func componentTokens(_ kinds: Set<AgentCoreContextComponentKind>) -> Int {
            prepared.componentMetrics
                .filter { kinds.contains($0.kind) }
                .reduce(0) { $0 + $1.usedTokens }
        }
        let usedComponentTokens = prepared.componentMetrics.reduce(0) { $0 + $1.usedTokens }
        let activeMessageTokens = max(0, prepared.metrics.messageTokens - usedComponentTokens)
        let sessionMemoryTokens = componentTokens([.sessionMemory])
        let longTermMemoryTokens = componentTokens([.memory])
        let userPreferencesTokens = componentTokens([.userPreference])
        let mcpServersTokens = componentTokens([.mcp])
        let uploadedSkillsTokens = componentTokens([.skill])
        let uploadedDocumentsTokens = componentTokens([.document])
        let supplementalTokens = componentTokens([
            .system,
            .promptTemplate,
            .personalIntelligence,
            .attachment,
            .webRetrieval,
            .platformData
        ])
        let totalTokens = prepared.metrics.totalPromptTokens
        let tokenLimit = prepared.metrics.promptTokenLimit
        let percentage = tokenLimit > 0 ? min(100, Int((Double(totalTokens) / Double(tokenLimit)) * 100.0)) : 0
        let coreArchiveMessageIDs = prepared.archiveRequest?.messageIds.compactMap(UUID.init(uuidString:)) ?? []

        let suggestionDetails = buildContextSuggestionDetails(
            percentage: percentage,
            totalTokens: totalTokens,
            tokenLimit: tokenLimit,
            activeMessageTokens: activeMessageTokens,
            activeMessageCount: activeMessages.count,
            sessionMemoryTokens: sessionMemoryTokens,
            longTermMemoryTokens: longTermMemoryTokens,
            userPreferencesTokens: userPreferencesTokens,
            mcpServersTokens: mcpServersTokens,
            uploadedSkillsTokens: uploadedSkillsTokens,
            uploadedDocumentsTokens: uploadedDocumentsTokens,
            supplementalTokens: supplementalTokens,
            archivedMessageCount: archivedMessages.count
        )
        let suggestions = suggestionDetails.map { detail in
            let title = detail.title.trimmingCharacters(in: .whitespacesAndNewlines)
            let body = detail.detail.trimmingCharacters(in: .whitespacesAndNewlines)
            if title.isEmpty {
                return body
            }
            if body.isEmpty {
                return title
            }
            return "\(title)：\(body)"
        }

        return ConversationContextUsage(
            estimatedTokenLimit: tokenLimit,
            totalTokens: totalTokens,
            percentage: percentage,
            activeMessageTokens: activeMessageTokens,
            sessionMemoryTokens: sessionMemoryTokens,
            longTermMemoryTokens: longTermMemoryTokens,
            userPreferencesTokens: userPreferencesTokens,
            mcpServersTokens: mcpServersTokens,
            uploadedSkillsTokens: uploadedSkillsTokens,
            uploadedDocumentsTokens: uploadedDocumentsTokens,
            supplementalTokens: supplementalTokens,
            activeMessageCount: activeMessages.count,
            archivedMessageCount: archivedMessages.count,
            sessionMemory: conversation.sessionMemory,
            longTermMemory: profile.longTermMemory,
            userPreferences: profile.userPreferences,
            mcpServers: profile.mcpServers,
            uploadedSkills: profile.uploadedSkills,
            uploadedDocuments: profile.uploadedDocuments,
            lastCompactedAt: conversation.lastCompactedAt,
            suggestionDetails: suggestionDetails,
            suggestions: suggestions,
            coreAction: prepared.action,
            coreArchiveMessageIDs: coreArchiveMessageIDs
        )
    }

    public func updateSessionMemory(for conversationId: UUID, sessionMemory: String) async throws {
        try Task.checkCancellation()
        guard let contextStore = store as? ConversationContextStore else { throw ContextOutboundPreparationError.coreUnavailable }
        let source = try contextStore.prepareContextCompaction(conversationId: conversationId)
        try contextStore.replaceContextSummary(source: source, summary: sessionMemory)
    }

    public func compactConversation(for conversationId: UUID, maxTokens: Int, preserveRecentMessages: Int = 8, force: Bool = true) async throws -> ConversationContextUsage {
        try await compactConversationCore(
            for: conversationId,
            maxTokens: maxTokens,
            preserveRecentMessages: preserveRecentMessages,
            force: force,
            additionalComponents: [],
            tools: []
        )
    }

    private func compactConversationCore(
        for conversationId: UUID,
        maxTokens: Int,
        preserveRecentMessages: Int,
        force: Bool,
        additionalComponents: [AgentCoreContextComponent],
        tools: [AgentCoreJSONValue]
    ) async throws -> ConversationContextUsage {
        try Task.checkCancellation()
        guard let contextStore = store as? LocalConversationStore else { throw ContextOutboundPreparationError.coreUnavailable }
        let source = try contextStore.prepareContextCompaction(conversationId: conversationId)
        guard let conversation = try store.getConversation(conversationId: conversationId) else {
            return try await estimateContextUsage(for: conversationId, maxTokens: maxTokens)
        }

        let allMessages = try store.getMessages(conversationId: conversationId, limit: 0)
        let archivedMessageIDs = archivedMessageIDs(in: conversation)
        let activeMessages = allMessages.filter { !archivedMessageIDs.contains($0.id) }
        let profile = profileStore.loadProfile()
        let components = try await coreComponents(
            conversation: conversation,
            profile: profile,
            allowLiveFetch: false
        ) + additionalComponents
        func maintenanceInput(_ messages: [ConversationMessage], summary: String?) -> AgentCoreContextPrepareInput {
            AgentCoreContextPrepareInput(messages: messages.map {
                AgentCoreContextPrepareMessage(id: $0.id.uuidString, role: $0.role.rawValue, content: $0.text,
                    ephemeral: $0.meta?["ephemeral"] == "true", nolaEphemeral: $0.meta?["_nola_ephemeral"] == "true")
            }, tools: tools, platform: .ios, components: components,
                budget: AgentCoreContextPrepareBudget(contextWindowTokens: maxTokens, reservedOutputTokens: 4_096,
                    keepRecentMessages: preserveRecentMessages), mode: force ? .forceSummary : .normal, existingSummary: summary)
        }
        let plan = try contextStore.prepareContextMaintenance(source: source,
            input: maintenanceInput(activeMessages, summary: conversation.sessionMemory))
        let prepared = plan.output
        if prepared.action == "ready" {
            return try await estimateContextUsage(for: conversationId, maxTokens: maxTokens)
        }
        if prepared.action == "cannot_fit" {
            throw ContextOutboundPreparationError.cannotFit
        }
        guard prepared.action == "summary_required", let request = prepared.archiveRequest else {
            throw ContextOutboundPreparationError.invalidCoreAction(prepared.action)
        }

        let activeByID = Dictionary(uniqueKeysWithValues: activeMessages.map { ($0.id.uuidString, $0) })
        let messagesToArchive = request.messageIds.compactMap { activeByID[$0] }
        guard messagesToArchive.count == request.messageIds.count,
              messagesToArchive.map({ $0.id.uuidString }) == request.messageIds,
              Set(request.messageIds).count == request.messageIds.count else {
            throw ContextOutboundPreparationError.cannotFit
        }

        let summary = try await summarizeMaintenance(plan: plan, conversation: conversation, store: contextStore)
        try Task.checkCancellation()
        let validatedSource = try contextStore.prepareContextCompaction(conversationId: conversationId)
        guard let current = try store.getConversation(conversationId: conversationId) else {
            throw ContextOutboundPreparationError.cannotFit
        }
        let existingArchivedIDs = self.archivedMessageIDs(in: current)
        let currentMessages = try store.getMessages(conversationId: conversationId, limit: 0)
            .filter { !existingArchivedIDs.contains($0.id) }
        try contextStore.commitContextMaintenance(plan: plan, validatedSource: validatedSource,
            currentInput: maintenanceInput(currentMessages, summary: current.sessionMemory), summary: summary)

        return try await estimateContextUsage(for: conversationId, maxTokens: maxTokens)
    }
}
