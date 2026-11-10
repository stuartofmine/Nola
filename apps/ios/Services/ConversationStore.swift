// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ConversationStore logic in the iOS service layer.
// - Interfaces:
//   - ChatRole
//   - CompactedMemoryEntry
//   - Conversation
//   - ConversationMessage
//   - ConversationStore
//   - LocalConversationStore
//   - AnyEncodable
// - APIs:
//   - none
//
//
//  ConversationStore.swift
//  Nola
//

import Foundation

// MARK: - Chat Role Enum
nonisolated public enum ChatRole: String, Codable {
    case user
    case assistant
    case system
}

nonisolated public struct CompactedMemoryEntry: Identifiable, Codable, Equatable {
    public let id: UUID
    public let createdAt: Date
    public let summary: String
    public let rawExcerpt: String
    public let archivedMessageIDs: [UUID]
    public let archiveID: String?
    public let archiveDigest: String?
    public let payloadDigest: String?

    public init(
        id: UUID = UUID(),
        createdAt: Date = Date(),
        summary: String,
        rawExcerpt: String,
        archivedMessageIDs: [UUID],
        archiveID: String? = nil,
        archiveDigest: String? = nil,
        payloadDigest: String? = nil
    ) {
        self.id = id
        self.createdAt = createdAt
        self.summary = summary
        self.rawExcerpt = rawExcerpt
        self.archivedMessageIDs = archivedMessageIDs
        self.archiveID = archiveID
        self.archiveDigest = archiveDigest
        self.payloadDigest = payloadDigest
    }
}

/// Read-only ownership derived from canonical settings, never a client configuration field.
nonisolated public struct ConversationAgentRoomOwner: Equatable, Sendable {
    public let roomID: String
    public let participantID: String
    public let agentID: String

    static func read(_ value: Any) throws -> Self {
        guard let fields = value as? [String: Any],
              Set(fields.keys) == Set(["room_id", "participant_id", "agent_id"]),
              let room = fields["room_id"] as? String, UUID(uuidString: room) != nil,
              let participant = fields["participant_id"] as? String, UUID(uuidString: participant) != nil,
              let agent = fields["agent_id"] as? String, !agent.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("Invalid canonical room ownership")
        }
        return Self(roomID: room, participantID: participant, agentID: agent)
    }
}

nonisolated public struct Conversation: Identifiable, Codable {
    public let id: UUID
    public var title: String?
    public var createdAt: Date
    public var lastActiveAt: Date
    public var language: String? // "en" / "zh-Hans"
    public var sessionMemory: String
    public var knowledgePoints: String
    public var wiki: String
    public var lastCompactedAt: Date?
    // Optional conversation-scoped prompt template (user can pick a template for this conversation)
    public var selectedPromptTemplateID: UUID?
    // Explicit per-conversation decisions. Missing means the source policy decides.
    public var dataSourceOverrides: [String: Bool]
    public var selectedModelInfo: ModelInfo?
    public var modelSelectionID: String? = nil
    public var configurationRevision: Int64? = nil
    /// Derived availability; never persisted as conversation configuration.
    public var modelUnavailableReason: String? = nil
    public fileprivate(set) var agentRoomOwner: ConversationAgentRoomOwner? = nil
    public var appearsInOrdinaryHistory: Bool { agentRoomOwner == nil }
    /// Shared settings_json contract: nil inherits the selected model's defaults.
    public var promptResponseStyle: PromptResponseStyle? = nil
    /// Conversation-scoped Agent execution unit. External executors live on
    /// the connected Host; iOS only persists the selection and resume handle.
    public var executionBackend: String
    public var externalExecutionSessions: [String: String]
    public var persistenceEnabled: Bool
    public var permissionMode: IOSPermissionMode
    // Raw memory snapshots captured on compact for future lookup.
    public var compactedMemoryEntries: [CompactedMemoryEntry]
    public init(id: UUID = UUID(), title: String? = nil, createdAt: Date = Date(), lastActiveAt: Date = Date(), language: String? = nil, sessionMemory: String = "", knowledgePoints: String = "", wiki: String = "", lastCompactedAt: Date? = nil, selectedPromptTemplateID: UUID? = nil, dataSourceOverrides: [String: Bool] = [:], selectedModelInfo: ModelInfo? = nil, executionBackend: String = "nola", externalExecutionSessions: [String: String] = [:], persistenceEnabled: Bool = true, permissionMode: IOSPermissionMode = .cautious, compactedMemoryEntries: [CompactedMemoryEntry] = []) {
        self.id = id
        self.title = title
        self.createdAt = createdAt
        self.lastActiveAt = lastActiveAt
        self.language = language
        self.sessionMemory = sessionMemory
        self.knowledgePoints = knowledgePoints
        self.wiki = wiki
        self.lastCompactedAt = lastCompactedAt
        self.selectedPromptTemplateID = selectedPromptTemplateID
        self.dataSourceOverrides = dataSourceOverrides
        self.selectedModelInfo = selectedModelInfo
        self.executionBackend = executionBackend
        self.externalExecutionSessions = externalExecutionSessions
        self.persistenceEnabled = persistenceEnabled
        self.permissionMode = permissionMode
        self.compactedMemoryEntries = compactedMemoryEntries
    }

    private enum CodingKeys: String, CodingKey {
        case id, title, createdAt, lastActiveAt, language, sessionMemory
        case knowledgePoints, wiki, lastCompactedAt, selectedPromptTemplateID
        case dataSourceOverrides, modelSelectionID, promptResponseStyle, executionBackend, externalExecutionSessions
        case configurationRevision
        case persistenceEnabled, permissionMode, compactedMemoryEntries
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(UUID.self, forKey: .id)
        title = try values.decodeIfPresent(String.self, forKey: .title)
        createdAt = try values.decode(Date.self, forKey: .createdAt)
        lastActiveAt = try values.decode(Date.self, forKey: .lastActiveAt)
        language = try values.decodeIfPresent(String.self, forKey: .language)
        sessionMemory = try values.decode(String.self, forKey: .sessionMemory)
        knowledgePoints = try values.decode(String.self, forKey: .knowledgePoints)
        wiki = try values.decode(String.self, forKey: .wiki)
        lastCompactedAt = try values.decodeIfPresent(Date.self, forKey: .lastCompactedAt)
        selectedPromptTemplateID = try values.decodeIfPresent(UUID.self, forKey: .selectedPromptTemplateID)
        dataSourceOverrides = try values.decode([String: Bool].self, forKey: .dataSourceOverrides)
        selectedModelInfo = nil
        modelSelectionID = try values.decodeIfPresent(String.self, forKey: .modelSelectionID)
        configurationRevision = try values.decodeIfPresent(Int64.self, forKey: .configurationRevision)
        promptResponseStyle = try values.decodeIfPresent(PromptResponseStyle.self, forKey: .promptResponseStyle)
        executionBackend = try values.decode(String.self, forKey: .executionBackend)
        externalExecutionSessions = try values.decode([String: String].self, forKey: .externalExecutionSessions)
        persistenceEnabled = try values.decode(Bool.self, forKey: .persistenceEnabled)
        permissionMode = values.contains(.permissionMode)
            ? try values.decode(IOSPermissionMode.self, forKey: .permissionMode) : .cautious
        compactedMemoryEntries = try values.decode([CompactedMemoryEntry].self, forKey: .compactedMemoryEntries)
    }

}

nonisolated public struct ConversationMessage: Identifiable, Codable {
    public let id: UUID
    public let conversationId: UUID
    public let role: ChatRole
    public let text: String
    public let createdAt: Date
    public let meta: [String: String]?

    public init(id: UUID = UUID(), conversationId: UUID, role: ChatRole, text: String, createdAt: Date = Date(), meta: [String: String]? = nil) {
        self.id = id
        self.conversationId = conversationId
        self.role = role
        self.text = text
        self.createdAt = createdAt
        self.meta = meta
    }
}

protocol ConversationContextStore {
    func prepareContextCompaction(conversationId: UUID) throws -> IOSContextCompactionSource
    func replaceContextSummary(source: IOSContextCompactionSource, summary: String) throws
}

nonisolated public enum ConversationMessageImportSource: String, Codable {
    case backupRestore = "backup_restore"
    case remoteSync = "remote_sync"
    case externalArchive = "external_archive"
}

nonisolated protocol ConversationPermissionStore: AnyObject {
    func setConversationPermission(_ command: SetConversationPermissionCommand) throws
}

nonisolated public protocol ConversationStore: AnyObject {
    func getMessages(conversationId: UUID, limit: Int) throws -> [ConversationMessage]
    func getConversation(conversationId: UUID) throws -> Conversation?
    func createConversation(title: String?, language: String?) throws -> Conversation
    func deleteConversation(conversationId: UUID) throws
    func exportConversation(conversationId: UUID) throws -> Data
    func getAllConversations() throws -> [Conversation]
    // Update conversation metadata (e.g. title)
    func updateConversationTitle(conversationId: UUID, title: String) throws
    func setPromptResponseStyle(conversationId: UUID, style: PromptResponseStyle?) throws
    // Update conversation metadata: language, selected prompt template, and associated data sources
    func updateConversationMetadata(conversationId: UUID, language: String?, selectedPromptTemplateID: UUID?, dataSourceOverrides: [String: Bool]?) throws
    // Merge a conversation from remote source into local store.
    func upsertConversation(_ conversation: Conversation) throws
    // Append only messages not already present locally (incremental sync, dedup by remoteMessageId).
    func upsertMessages(conversationId: UUID, messages: [ConversationMessage]) throws
    // Import finalized history with explicit provenance. Interactive messages
    // must not use this path.
    func importMessages(
        conversationId: UUID,
        messages: [ConversationMessage],
        source: ConversationMessageImportSource
    ) throws
}

/// The two request limits shared with Mac's sparse `modelOverrides` contract.
/// Nil inherits the selected model profile; zero is never a valid token limit.
nonisolated struct IOSConversationGenerationOverride: Equatable, Sendable {
    var maxContextLength: Int?
    var maxOutputTokens: Int?

    static func modelKey(for model: ModelInfo) -> String {
        if model.isWebBackend { return "execution-host" }
        return "\(model.isLocal ? "local" : "configured"):\(model.id)"
    }

    func applying(to defaults: ModelParameters) -> ModelParameters {
        var result = defaults
        if let maxContextLength { result.maxContextLength = maxContextLength }
        if let maxOutputTokens { result.maxOutputTokens = maxOutputTokens }
        return result
    }

    var isEmpty: Bool { maxContextLength == nil && maxOutputTokens == nil }
}

nonisolated protocol ConversationGenerationOverrideStore: AnyObject {
    func generationOverride(conversationId: UUID, modelKey: String) throws -> IOSConversationGenerationOverride
    func setGenerationOverride(
        conversationId: UUID,
        modelKey: String,
        override: IOSConversationGenerationOverride
    ) throws
    func setGenerationOverride(conversationId: UUID, modelKey: String,
        override: IOSConversationGenerationOverride, expectedBaseline: IOSConversationGenerationOverride) throws
}

nonisolated protocol ConfiguredConversationCreationStore: AnyObject {
    func createConfiguredConversation(title: String?, language: String?, model: ModelInfo, parameters: ModelParameters) throws -> Conversation
    func selectConversationModel(conversationID: UUID, model: ModelInfo, initialParameters: ModelParameters) throws
}

nonisolated final class LocalConversationStore: ConversationPermissionStore, ConversationStore, ConversationProjectionStore, ConfiguredConversationCreationStore,
    ConversationRemoteTurnStore, ConversationGenerationOverrideStore, ConversationParameterSnapshotStore, ConversationContextStore {
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private let queue: DispatchQueue
    private let applicationRuntime: IOSApplicationRuntime

    init(
        applicationRuntime: IOSApplicationRuntime? = IOSApplicationRuntime.shared
    ) throws {
        self.encoder = JSONEncoder()
        self.encoder.dateEncodingStrategy = .iso8601
        self.decoder = JSONDecoder()
        self.decoder.dateDecodingStrategy = .iso8601
        self.queue = DispatchQueue(label: "com.nola.conversation.store")
        guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
        self.applicationRuntime = applicationRuntime
    }

    public func createConversation(title: String?, language: String?) throws -> Conversation {
        try queue.sync {
            let now = Date()
            let conversation = Conversation(
                title: title,
                createdAt: now,
                lastActiveAt: now,
                language: language,
                persistenceEnabled: true
            )
            try persistCanonical(conversation, messages: [])
            guard let created = try applicationRuntime.loadConversation(id: conversation.id.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Created conversation is unavailable")
            }
            return try domainConversation(created)
        }
    }

    public func getMessages(conversationId: UUID, limit: Int = 100) throws -> [ConversationMessage] {
        return try queue.sync {
            let sorted = try applicationRuntime.loadConversationProjectionV1(id: conversationId.uuidString)
                .messages.compactMap { domainMessage($0, conversationID: conversationId) }
            if limit > 0 && sorted.count > limit {
                return Array(sorted.suffix(limit))
            }
            return sorted
        }
    }

    func createConfiguredConversation(title: String?, language: String?, model: ModelInfo, parameters: ModelParameters) throws -> Conversation {
        try queue.sync {
            let now = Date()
            var conversation = Conversation(title: title, createdAt: now, lastActiveAt: now,
                language: language, persistenceEnabled: true)
            conversation.selectedModelInfo = model.redactedForPersistence
            let key = IOSConversationGenerationOverride.modelKey(for: model)
            var fields = try IOSConversationSettingsObject.decode(ConversationParameterSnapshot.updating(
                settingsJSON(conversation), modelKey: key, parameters: parameters))
            if model.isWebBackend {
                fields["executionHostBinding"] = try hostBinding(model)
            } else {
                try validateModel(model)
                fields["modelSelectionID"] = key
            }
            let json = String(decoding: try JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]), as: UTF8.self)
            try applicationRuntime.createConversation(id: conversation.id.uuidString, title: title ?? "",
                settingsJSON: json, createdAtMs: now.millisecondsSince1970, updatedAtMs: now.millisecondsSince1970)
            guard let created = try applicationRuntime.loadConversation(id: conversation.id.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Created conversation is unavailable")
            }
            return try domainConversation(created)
        }
    }

    func loadConversationProjection(conversationId: UUID) throws -> IOSConversationProjectionV1 {
        try queue.sync { try applicationRuntime.loadConversationProjectionV1(id: conversationId.uuidString) }
    }

    func selectConversationModel(conversationID: UUID, model: ModelInfo, initialParameters: ModelParameters) throws {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationID.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            if !model.isWebBackend { try validateModel(model) }
            let key = IOSConversationGenerationOverride.modelKey(for: model)
            var fields = try IOSConversationSettingsObject.decode(stored.settingsJSON)
            if (model.isWebBackend && fields["clientHostParameters"] != nil) ||
                (fields["modelOverrides"] as? [String: Any])?[key] != nil {
                _ = try ConversationParameterSnapshot.read(stored.settingsJSON, modelKey: key)
            } else {
                guard fields["modelOverrides"] == nil || fields["modelOverrides"] is [String: Any] else {
                    throw IOSApplicationRuntimeError.rejected("Invalid conversation parameters")
                }
                fields = try IOSConversationSettingsObject.decode(ConversationParameterSnapshot.updating(
                    stored.settingsJSON, modelKey: key, parameters: initialParameters))
            }
            guard let revision = stored.configurationRevision else {
                throw IOSApplicationRuntimeError.rejected("Missing configuration revision")
            }
            func wire(_ fields: [String: Any]) throws -> [String: AgentCoreJSONValue] {
                try JSONDecoder().decode([String: AgentCoreJSONValue].self,
                    from: JSONSerialization.data(withJSONObject: fields))
            }
            let parameters = model.isWebBackend ? fields["clientHostParameters"] as? [String: Any]
                : (fields["modelOverrides"] as? [String: Any])?[key] as? [String: Any]
            guard let parameters else { throw IOSApplicationRuntimeError.rejected("Missing selected model parameters") }
            let presentation = (fields["clientModelPresentation"] as? [String: Any])?[key] as? [String: Any] ?? [:]
            let selection: SelectConversationExecutionCommand<AgentCoreJSONValue>.Selection
            if model.isWebBackend {
                selection = .host(binding: try wire(hostBinding(model)), parameters: try wire(parameters), presentation: try wire(presentation))
            } else {
                selection = .model(id: key, parameters: try wire(parameters), presentation: try wire(presentation))
            }
            try applicationRuntime.selectConversationExecution(.init(input: .init(conversation_id: conversationID.uuidString,
                expected_revision: revision, selection: selection, updated_at_ms: Int64(Date().timeIntervalSince1970 * 1000)))).requireCommitted()

        }
    }

    func loadConversationProjectionWindow(
        conversationId: UUID,
        beforeOrdinal: Int64?,
        limit: Int,
        contentByteLimit: Int
    ) throws -> IOSConversationProjectionWindowV1 {
        try queue.sync {
            try applicationRuntime.loadConversationProjectionWindowV1(
                id: conversationId.uuidString,
                beforeOrdinal: beforeOrdinal,
                limit: limit,
                contentByteLimit: contentByteLimit
            )
        }
    }

    func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws {
        try queue.sync {
            try IOSCanonicalRemoteConversationCommands.prepare(
                store: applicationRuntime,
                descriptor: descriptor
            )
        }
    }

    func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        try queue.sync { try applicationRuntime.executeAgentCommand(command) }
    }

    @MainActor
    func bindAuxiliaryInvocation(_ invocation: LLMSummaryInvocation) -> LLMSummaryInvocation {
        invocation.usingLedger(applicationRuntime)
    }

    public func getConversation(conversationId: UUID) throws -> Conversation? {
        return try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else { return nil }
            return try domainConversation(stored)
        }
    }

    func generationOverride(conversationId: UUID, modelKey: String) throws -> IOSConversationGenerationOverride {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            return IOSConversationGenerationOverrides.read(stored.settingsJSON, modelKey: modelKey)
        }
    }

    func conversationParameters(conversationId: UUID, modelKey: String) throws -> ModelParameters {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            return try ConversationParameterSnapshot.read(stored.settingsJSON, modelKey: modelKey)
        }
    }

    func hasConversationParameters(conversationId: UUID, modelKey: String) throws -> Bool {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            let root = try IOSConversationSettingsObject.decode(stored.settingsJSON)
            if modelKey == "execution-host" {
                guard root["clientHostParameters"] != nil else { return false }
                _ = try ConversationParameterSnapshot.read(stored.settingsJSON, modelKey: modelKey)
                return true
            }
            guard let raw = root["modelOverrides"] else { return false }
            guard let models = raw as? [String: Any] else { throw IOSApplicationRuntimeError.rejected("Invalid conversation model parameters") }
            guard models[modelKey] != nil else { return false }
            _ = try ConversationParameterSnapshot.read(stored.settingsJSON, modelKey: modelKey)
            return true
        }
    }

    func saveConversationParameters(conversationId: UUID, modelKey: String, parameters: ModelParameters) throws {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            let updated = try IOSConversationSettingsObject.decode(
                ConversationParameterSnapshot.updating(stored.settingsJSON, modelKey: modelKey, parameters: parameters))
            let patch = updated.filter { ["modelOverrides", "clientModelPresentation", "clientHostParameters"].contains($0.key) }
            let patchJSON = String(decoding: try JSONSerialization.data(withJSONObject: patch, options: [.sortedKeys]), as: UTF8.self)
            try patchConfiguration(stored: stored, patchJSON: patchJSON)
        }
    }

    func setGenerationOverride(
        conversationId: UUID,
        modelKey: String,
        override value: IOSConversationGenerationOverride
    ) throws {
        try updateGenerationOverride(conversationId: conversationId, modelKey: modelKey, value: value, expectedBaseline: nil)
    }

    func setGenerationOverride(conversationId: UUID, modelKey: String,
        override value: IOSConversationGenerationOverride, expectedBaseline: IOSConversationGenerationOverride) throws {
        try updateGenerationOverride(conversationId: conversationId, modelKey: modelKey, value: value, expectedBaseline: expectedBaseline)
    }

    private func updateGenerationOverride(conversationId: UUID, modelKey: String,
        value: IOSConversationGenerationOverride, expectedBaseline: IOSConversationGenerationOverride?) throws {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found")
            }
            if let expectedBaseline,
               IOSConversationGenerationOverrides.read(stored.settingsJSON, modelKey: modelKey) != expectedBaseline {
                throw IOSApplicationRuntimeError.rejected("Conversation parameters changed while editing; reload and retry")
            }
            let updated = try IOSConversationGenerationOverrides.patching(
                stored.settingsJSON, modelKey: modelKey, override: value
            )
            let fields = try IOSConversationSettingsObject.decode(updated)
            let patch = ["modelOverrides": fields["modelOverrides"] ?? [:]]
            let patchJSON = String(decoding: try JSONSerialization.data(withJSONObject: patch), as: UTF8.self)
            try patchConfiguration(stored: stored, patchJSON: patchJSON)
        }
    }

    public func deleteConversation(conversationId: UUID) throws {
        try queue.sync { _ = try applicationRuntime.deleteConversation(id: conversationId.uuidString) }
        try FileConversationDraftRepository.scoped(to: applicationRuntime.databaseURL).remove(.conversation(conversationId))
    }

    public func updateConversationTitle(conversationId: UUID, title: String) throws {
        try queue.sync {
            guard try applicationRuntime.loadConversation(id: conversationId.uuidString) != nil else {
                throw NSError(domain: "LocalConversationStore", code: -1, userInfo: [NSLocalizedDescriptionKey: L10n.format("conversation_store.error.conversation_not_found_format", conversationId.uuidString)])
            }
            try applicationRuntime.renameConversation(
                id: conversationId.uuidString,
                title: title,
                updatedAtMs: Date().millisecondsSince1970
            )
        }
    }

    public func updateConversationMetadata(conversationId: UUID, language: String?, selectedPromptTemplateID: UUID?, dataSourceOverrides: [String: Bool]?) throws {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw NSError(domain: "LocalConversationStore", code: -1, userInfo: [NSLocalizedDescriptionKey: L10n.format("conversation_store.error.conversation_not_found_format", conversationId.uuidString)])
            }
            var conv = try domainConversation(stored)
            if let lang = language {
                conv.language = lang
            }
            if let tid = selectedPromptTemplateID {
                conv.selectedPromptTemplateID = tid
            } else if dataSourceOverrides != nil {
                conv.selectedPromptTemplateID = nil
            }
            if let dataSourceOverrides {
                // Only an explicit context edit changes canonical source overrides.
                // Ordinary conversation snapshot saves must not replay stale switches.
                let now = Date().millisecondsSince1970
                for source in try applicationRuntime.listDataSources() {
                    let desired: IOSCanonicalDataSourceOverride = dataSourceOverrides[source.id]
                        .map { $0 ? .enabled : .disabled } ?? .inherit
                    if try applicationRuntime.dataSourceOverride(conversationID: conversationId.uuidString, sourceID: source.id) != desired {
                        try applicationRuntime.setDataSourceOverride(
                            conversationID: conversationId.uuidString, sourceID: source.id,
                            value: desired, updatedAtMs: now
                        )
                    }
                }
                conv.dataSourceOverrides = dataSourceOverrides
            }
            conv.lastActiveAt = Date()
            try patchConfiguration(stored: stored, conversation: conv)
        }
    }

    func prepareContextMaintenance(source: IOSContextCompactionSource, input: AgentCoreContextPrepareInput) throws -> IOSContextMaintenancePlan {
        try queue.sync { try applicationRuntime.prepareContextMaintenance(source: source, input: input) }
    }

    func commitContextMaintenance(plan: IOSContextMaintenancePlan, validatedSource: IOSContextCompactionSource,
                                  currentInput: AgentCoreContextPrepareInput, summary: String) throws {
        try queue.sync { try applicationRuntime.commitContextMaintenance(plan: plan, validatedSource: validatedSource,
            currentInput: currentInput, summary: summary) }
    }

    func prepareContextCompaction(conversationId: UUID) throws -> IOSContextCompactionSource {
        try queue.sync { try applicationRuntime.prepareContextCompaction(conversationID: conversationId.uuidString) }
    }

    func replaceContextSummary(source: IOSContextCompactionSource, summary: String) throws {
        try queue.sync { try applicationRuntime.replaceContextSummary(source: source, summary: summary) }
    }

    func setConversationPermission(_ command: SetConversationPermissionCommand) throws {
        try queue.sync {
            try applicationRuntime.setConversationPermission(command).requireCommitted()
        }
    }

    public func upsertConversation(_ conversation: Conversation) throws {
        try queue.sync {
            if let stored = try applicationRuntime.loadConversation(id: conversation.id.uuidString) {
                guard conversation.configurationRevision == stored.configurationRevision else {
                    throw IOSApplicationRuntimeError.rejected("Conversation configuration changed; your edit is retained")
                }
                try patchConfiguration(stored: stored, conversation: conversation)
                try applicationRuntime.renameConversation(
                    id: conversation.id.uuidString,
                    title: conversation.title ?? "",
                    updatedAtMs: conversation.lastActiveAt.millisecondsSince1970
                )
                if stored.messages.isEmpty { return }
            } else {
                try persistCanonical(conversation, messages: [])
            }
        }
    }

    public func setPromptResponseStyle(conversationId: UUID, style: PromptResponseStyle?) throws {
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw IOSApplicationRuntimeError.rejected("Conversation not found: \(conversationId)")
            }
            try patchConfiguration(stored: stored, patchJSON: String(decoding: JSONSerialization.data(
                withJSONObject: ["promptResponseStyle": style.map { $0.rawValue as Any } ?? NSNull()]), as: UTF8.self))
        }
    }

    public func upsertMessages(conversationId: UUID, messages: [ConversationMessage]) throws {
        guard !messages.isEmpty else { return }
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else { return }
            let existing = stored.messages.compactMap { domainMessage($0, conversationID: conversationId) }
            let existingIDs = Set(existing.map(\.id))
            let remoteIDs = Set(existing.compactMap { $0.meta?["remoteMessageId"] })
            let additions = messages.filter { message in
                !existingIDs.contains(message.id)
                    && message.meta?["remoteMessageId"].map { !remoteIDs.contains($0) } != false
            }
            guard !additions.isEmpty else { return }
            let start = stored.messages.count + 1
            try applicationRuntime.importConversationTranscript(
                importID: "remote-sync:\(conversationId.uuidString):\(stored.messages.count)",
                source: ConversationMessageImportSource.remoteSync.rawValue,
                conversationID: conversationId.uuidString,
                expectedMessageCount: stored.messages.count,
                messages: additions.enumerated().map { canonicalMessage($0.element, ordinal: Int64(start + $0.offset)) },
                importedAtMs: additions.map(\.createdAt).max()?.millisecondsSince1970 ?? Date().millisecondsSince1970
            )
        }
    }

    public func importMessages(
        conversationId: UUID,
        messages: [ConversationMessage],
        source: ConversationMessageImportSource
    ) throws {
        guard !messages.isEmpty else { return }
        try queue.sync {
            guard let stored = try applicationRuntime.loadConversation(id: conversationId.uuidString) else {
                throw NSError(
                    domain: "LocalConversationStore",
                    code: -1,
                    userInfo: [NSLocalizedDescriptionKey: L10n.format("conversation_store.error.conversation_not_found_format", conversationId.uuidString)]
                )
            }
            let start = stored.messages.count + 1
            try applicationRuntime.importConversationTranscript(
                importID: "\(source.rawValue):\(conversationId.uuidString):\(stored.messages.count)",
                source: source.rawValue,
                conversationID: conversationId.uuidString,
                expectedMessageCount: stored.messages.count,
                messages: messages.enumerated().map {
                    canonicalMessage($0.element, ordinal: Int64(start + $0.offset))
                },
                importedAtMs: messages.map(\.createdAt).max()?.millisecondsSince1970
                    ?? Date().millisecondsSince1970
            )
        }
    }

    public func exportConversation(conversationId: UUID) throws -> Data {
        let conv = try getConversation(conversationId: conversationId)
        let messages = try getMessages(conversationId: conversationId, limit: 0)
        let payload: [String: Any] = [
            "conversation": conv as Any,
            "messages": messages
        ]
        let data = try JSONSerialization.data(withJSONObject: payload.mapValues { encodableToJson($0) }, options: [.prettyPrinted, .sortedKeys])
        return data
    }

    public func getAllConversations() throws -> [Conversation] {
        return try queue.sync {
            try applicationRuntime.listConversations().compactMap { summary in
                guard let stored = try applicationRuntime.loadConversation(id: summary.id) else { return nil }
                return try domainConversation(stored)
            }
        }
    }

    private func persistCanonical(_ conversation: Conversation, messages: [ConversationMessage]) throws {
        try applicationRuntime.createConversation(
            id: conversation.id.uuidString,
            title: conversation.title ?? "",
            settingsJSON: try settingsJSON(conversation),
            createdAtMs: conversation.createdAt.millisecondsSince1970,
            updatedAtMs: conversation.lastActiveAt.millisecondsSince1970
        )
        if !messages.isEmpty {
            try applicationRuntime.importConversationTranscript(
                importID: "external-archive:\(conversation.id.uuidString)",
                source: ConversationMessageImportSource.externalArchive.rawValue,
                conversationID: conversation.id.uuidString,
                expectedMessageCount: 0,
                messages: messages.enumerated().map {
                    canonicalMessage($0.element, ordinal: Int64($0.offset + 1))
                },
                importedAtMs: conversation.lastActiveAt.millisecondsSince1970
            )
        }
    }

    private func patchConfiguration(stored: IOSCanonicalConversation, patchJSON: String) throws {
        guard let revision = stored.configurationRevision else { throw IOSApplicationRuntimeError.rejected("Missing configuration revision") }
        try applicationRuntime.patchConversationConfiguration(id: stored.id, expectedRevision: revision,
            patchJSON: patchJSON, updatedAtMs: Date().millisecondsSince1970).requireCommitted()
    }

    private func patchConfiguration(stored: IOSCanonicalConversation, conversation: Conversation) throws {
        let current = try domainConversation(stored)
        guard current.permissionMode == conversation.permissionMode else {
            throw IOSApplicationRuntimeError.rejected("Use the owner-bound permission command to change permissions")
        }
        let canonicalSourceIDs = Set(try applicationRuntime.listDataSources().map(\.id))
        func fields(_ value: Conversation) -> [String: Any] {
            ["language": value.language as Any? ?? NSNull(),
             "selectedPromptTemplateID": value.selectedPromptTemplateID?.uuidString as Any? ?? NSNull(),
             "dataSourceOverrides": value.dataSourceOverrides.filter { !canonicalSourceIDs.contains($0.key) },
             "executionBackend": value.executionBackend,
             "externalExecutionSessions": value.externalExecutionSessions,
             "persistenceEnabled": value.persistenceEnabled,
             "promptResponseStyle": value.promptResponseStyle?.rawValue as Any? ?? NSNull(),
             "knowledge": ["keyPointsMarkdown": value.knowledgePoints, "wikiMarkdown": value.wiki]]
        }
        let before = fields(current)
        let after = fields(conversation)
        var changed = after.filter { key, value in
            !NSDictionary(dictionary: ["v": value]).isEqual(to: ["v": before[key] ?? NSNull()])
        }
        if changed["dataSourceOverrides"] != nil {
            let previous = before["dataSourceOverrides"] as? [String: Bool] ?? [:]
            let desired = after["dataSourceOverrides"] as? [String: Bool] ?? [:]
            var updates = desired.mapValues { $0 as Any }
            for key in previous.keys where desired[key] == nil { updates[key] = NSNull() }
            changed["dataSourceOverrides"] = updates
        }
        guard !changed.isEmpty else { return }
        try patchConfiguration(stored: stored, patchJSON: String(decoding:
            JSONSerialization.data(withJSONObject: changed, options: [.sortedKeys]), as: UTF8.self))
    }

    private func settingsJSON(_ conversation: Conversation, existingJSON: String? = nil) throws -> String {
        var settings = conversation
        let canonicalIDs = Set(try applicationRuntime.listDataSources().map(\.id))
        settings.dataSourceOverrides = settings.dataSourceOverrides.filter { !canonicalIDs.contains($0.key) }
        let encoded = try encoder.encode(IOSConversationSettings(settings))
        var merged = try IOSConversationSettingsObject.decode(existingJSON)
        // Only replace fields owned by the iOS settings projection. Other clients'
        // metadata must survive an unrelated conversation edit.
        for key in IOSConversationSettings.ownedKeys { merged.removeValue(forKey: key) }
        let own = try IOSConversationSettingsObject.decode(String(decoding: encoded, as: UTF8.self))
        merged.merge(own) { _, new in new }
        // Import/create may carry a canonical identity. Existing records retain
        // their stored identity; only the explicit configuration command changes it.
        if existingJSON == nil, let identity = conversation.modelSelectionID {
            merged["modelSelectionID"] = identity
        }
        merged["promptResponseStyle"] = settings.promptResponseStyle.map { $0.rawValue as Any } ?? NSNull()
        merged.removeValue(forKey: "permissionMode")
        merged["permissionModeRawValue"] = settings.permissionMode.rawValue
        merged.removeValue(forKey: "knowledgePoints")
        merged.removeValue(forKey: "wiki")
        merged["knowledge"] = ["keyPointsMarkdown": settings.knowledgePoints, "wikiMarkdown": settings.wiki, "updatedAt": NSNull()]
        return String(decoding: try JSONSerialization.data(withJSONObject: merged, options: [.sortedKeys]), as: UTF8.self)
    }

    private func domainConversation(_ stored: IOSCanonicalConversation) throws -> Conversation {
        let fields = try IOSConversationSettingsObject.decode(stored.settingsJSON)
        let configuredPermission: IOSPermissionMode?
        if let value = fields["permissionModeRawValue"] {
            guard let raw = value as? String, let mode = IOSPermissionMode(rawValue: raw) else {
                throw DecodingError.dataCorrupted(.init(codingPath: [],
                    debugDescription: "Permission mode must be cautious or allow_all."))
            }
            configuredPermission = mode
        } else {
            configuredPermission = nil
        }
        var conversation = Conversation(
            id: UUID(uuidString: stored.id) ?? UUID(),
            title: stored.title,
            createdAt: Date(millisecondsSince1970: stored.createdAtMs),
            lastActiveAt: Date(millisecondsSince1970: stored.updatedAtMs)
        )
        if let owner = fields["agentRoomOwner"] {
            conversation.agentRoomOwner = try ConversationAgentRoomOwner.read(owner)
        }
        conversation.sessionMemory = fields["sessionMemory"] as? String ?? ""
        if let date = fields["lastCompactedAt"] as? String { conversation.lastCompactedAt = ISO8601DateFormatter().date(from: date) }
        if let entries = fields["compactedMemoryEntries"] as? [Any] {
            conversation.compactedMemoryEntries = try decoder.decode([CompactedMemoryEntry].self,
                from: JSONSerialization.data(withJSONObject: entries))
        }
        if let configuredPermission { conversation.permissionMode = configuredPermission }
        // Shared configuration is readable even when another client did not
        // encode the historical iOS settings projection's required fields.
        if let language = fields["language"] as? String { conversation.language = language }
        if let template = fields["selectedPromptTemplateID"] as? String {
            conversation.selectedPromptTemplateID = UUID(uuidString: template)
        }
        if let backend = fields["executionBackend"] as? String { conversation.executionBackend = backend }
        if let sessions = fields["externalExecutionSessions"] as? [String: String] { conversation.externalExecutionSessions = sessions }
        if let enabled = fields["persistenceEnabled"] as? Bool { conversation.persistenceEnabled = enabled }
        if let overrides = fields["dataSourceOverrides"] as? [String: Bool] { conversation.dataSourceOverrides = overrides }
        conversation.selectedModelInfo = nil
        conversation.modelSelectionID = fields["modelSelectionID"] as? String
        conversation.configurationRevision = stored.configurationRevision
        if let knowledge = fields["knowledge"] as? [String: Any] {
            conversation.knowledgePoints = knowledge["keyPointsMarkdown"] as? String ?? conversation.knowledgePoints
            conversation.wiki = knowledge["wikiMarkdown"] as? String ?? conversation.wiki
        }
        if let identity = fields["modelSelectionID"] as? String {
            let parts = identity.split(separator: ":", maxSplits: 1).map(String.init)
            if parts.count == 2, ["configured", "local"].contains(parts[0]),
               let profile = try applicationRuntime.modelProfile(id: parts[1]),
               profile.runtime == (parts[0] == "local" ? "mlx" : "remote") {
                conversation.selectedModelInfo = ModelSelectionManager.modelInfo(from: profile)
            }
            if conversation.selectedModelInfo == nil {
                conversation.modelUnavailableReason = "The conversation's selected model is unavailable. Select a model to continue; history is preserved."
            }
        } else if let binding = fields["executionHostBinding"] as? [String: Any],
                  let baseURL = binding["baseURL"] as? String {
            conversation.selectedModelInfo = ModelInfo(id: "web-backend:local-host", name: "Web Host", source: "web",
                isLocal: false, baseURL: baseURL, tlsCertificateSHA256: binding["tlsCertificateSHA256"] as? String)
        }
        // Read the shared field even when settings originated on another client
        // and do not decode as the iOS-specific projection.
        if let json = stored.settingsJSON {
            let fields = try IOSConversationSettingsObject.decode(json)
            conversation.promptResponseStyle = (fields["promptResponseStyle"] as? String).flatMap(PromptResponseStyle.init(rawValue:))
        }
        for source in try applicationRuntime.listDataSources() {
            conversation.dataSourceOverrides.removeValue(forKey: source.id)
            switch try applicationRuntime.dataSourceOverride(conversationID: stored.id, sourceID: source.id) {
            case .inherit: break
            case .enabled: conversation.dataSourceOverrides[source.id] = true
            case .disabled: conversation.dataSourceOverrides[source.id] = false
            }
        }
        return conversation
    }

    private func validateModel(_ model: ModelInfo) throws {
        guard let profile = try applicationRuntime.modelProfile(id: model.id),
              profile.runtime == (model.isLocal ? "mlx" : "remote"),
              ModelSelectionManager.modelInfo(from: profile) != nil else {
            throw IOSApplicationRuntimeError.rejected("Selected model profile is unavailable")
        }
    }

    private func hostBinding(_ model: ModelInfo) throws -> [String: Any] {
        guard let baseURL = model.baseURL, let url = URLComponents(string: baseURL),
              ["http", "https"].contains(url.scheme ?? ""), url.host != nil,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else {
            throw IOSApplicationRuntimeError.rejected("Invalid execution Host connection")
        }
        var binding: [String: Any] = ["baseURL": baseURL]
        if let pin = model.tlsCertificateSHA256 { binding["tlsCertificateSHA256"] = pin }
        return binding
    }

    private func canonicalMessage(_ message: ConversationMessage, ordinal: Int64) -> IOSCanonicalConversationMessage {
        let canonicalID = message.meta?["nola.canonicalMessageID"] ?? message.id.uuidString
        return IOSCanonicalConversationMessage(
            id: canonicalID,
            runID: nil,
            role: message.role.rawValue,
            status: "completed",
            ordinal: ordinal,
            createdAtMs: message.createdAt.millisecondsSince1970,
            completedAtMs: message.createdAt.millisecondsSince1970,
            parts: [IOSCanonicalConversationPart(
                id: "\(canonicalID):text",
                kind: "text",
                ordinal: 1,
                state: "completed",
                contentText: message.text,
                contentJSON: message.meta.flatMap { try? String(decoding: JSONEncoder().encode($0), as: UTF8.self) },
                createdAtMs: message.createdAt.millisecondsSince1970,
                updatedAtMs: message.createdAt.millisecondsSince1970
            )]
        )
    }

    private func domainMessage(_ stored: IOSCanonicalConversationMessage, conversationID: UUID) -> ConversationMessage? {
        guard let role = ChatRole(rawValue: stored.role) else { return nil }
        let id = UUID(uuidString: stored.id)
            ?? IOSCanonicalMessageIdentity.stableUUID(for: stored.id)
        let part = stored.parts.first(where: { $0.kind == "text" }) ?? stored.parts.first
        var meta = part?.contentJSON.flatMap {
            try? JSONDecoder().decode([String: String].self, from: Data($0.utf8))
        } ?? [:]
        meta["nola.canonicalMessageID"] = stored.id
        return ConversationMessage(
            id: id,
            conversationId: conversationID,
            role: role,
            text: part?.contentText ?? "",
            createdAt: Date(millisecondsSince1970: stored.createdAtMs),
            meta: meta
        )
    }

    private func domainMessage(_ projected: IOSConversationProjectionV1.Message, conversationID: UUID) -> ConversationMessage? {
        let role: ChatRole
        switch projected.role { case .user: role = .user; case .assistant: role = .assistant; case .system: return nil }
        let text = projected.parts.compactMap { part -> String? in
            switch part {
            case .text(let value): return value.isDisclosed ? value.markdown : nil
            case .uiAction(let value): return value.summary
            case .historical(_, _, let projection): return projection.timelineText
            default: return nil
            }
        }.joined(separator: "\n\n")
        return ConversationMessage(id: UUID(uuidString: projected.messageID) ?? IOSCanonicalMessageIdentity.stableUUID(for: projected.messageID), conversationId: conversationID, role: role, text: text, createdAt: Date(millisecondsSince1970: projected.createdAtMs), meta: ["nola.canonicalMessageID": projected.messageID])
    }

    private func getConversationUnlocked(_ id: UUID) throws -> Conversation? {
        guard let stored = try applicationRuntime.loadConversation(id: id.uuidString) else { return nil }
        return try domainConversation(stored)
    }

    private func getMessagesUnlocked(_ id: UUID, limit: Int) throws -> [ConversationMessage] {
        let messages = try applicationRuntime.loadConversationProjectionV1(id: id.uuidString).messages
            .compactMap { domainMessage($0, conversationID: id) }
        return limit > 0 && messages.count > limit ? Array(messages.suffix(limit)) : messages
    }

    // Helpers
    private func encodableToJson(_ v: Any) -> Any {
        // JSONEncoder -> Data -> JSON object
        if let enc = v as? Encodable {
            do {
                let d = try encoder.encode(AnyEncodable(enc))
                let obj = try JSONSerialization.jsonObject(with: d, options: [])
                return obj
            } catch {
                return "<encoding_error>"
            }
        }
        return "<not_encodable>"
    }

    private func atomicWrite(_ data: Data, to url: URL) throws {
        let tmp = url.appendingPathExtension("tmp")
        try data.write(to: tmp, options: [.atomic])
        _ = try? FileManager.default.replaceItemAt(url, withItemAt: tmp)
    }
}

nonisolated enum IOSConversationSettingsObject {
    static func decode(_ json: String?) throws -> [String: Any] {
        guard let json else { return [:] }
        guard let object = try JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed]) as? [String: Any] else {
            throw IOSApplicationRuntimeError.rejected("Conversation settings must be a JSON object")
        }
        return object
    }
}

/// Patches only iOS-owned generation fields. The containing model declaration
/// may include Mac-owned protocol/reasoning fields, which must survive reset.
nonisolated enum IOSConversationGenerationOverrides {
    static func read(_ json: String?, modelKey: String) -> IOSConversationGenerationOverride {
        guard let fields = try? IOSConversationSettingsObject.decode(json),
              let models = fields["modelOverrides"] as? [String: Any],
              let model = models[modelKey] as? [String: Any] else { return .init() }
        return .init(
            maxContextLength: model["maxContextLength"] as? Int,
            maxOutputTokens: model["maxOutputTokens"] as? Int
        )
    }

    static func patching(
        _ json: String?, modelKey: String, override value: IOSConversationGenerationOverride
    ) throws -> String {
        guard !modelKey.isEmpty else { throw IOSApplicationRuntimeError.rejected("Model key is required") }
        if let limit = value.maxContextLength, !ModelParameterLimits.remoteContextTokens.contains(limit) {
            throw IOSApplicationRuntimeError.rejected("Invalid context window")
        }
        if let limit = value.maxOutputTokens, !ModelParameterLimits.outputTokens.contains(limit) {
            throw IOSApplicationRuntimeError.rejected("Invalid output token limit")
        }
        var fields = try IOSConversationSettingsObject.decode(json)
        var models = fields["modelOverrides"] as? [String: Any] ?? [:]
        var model = models[modelKey] as? [String: Any] ?? [:]
        // Absence means no change, not inheritance from mutable app defaults.
        if let limit = value.maxContextLength { model["maxContextLength"] = limit }
        if let limit = value.maxOutputTokens { model["maxOutputTokens"] = limit }
        if model.isEmpty { models.removeValue(forKey: modelKey) } else { models[modelKey] = model }
        if models.isEmpty { fields.removeValue(forKey: "modelOverrides") } else { fields["modelOverrides"] = models }
        return String(decoding: try JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]), as: UTF8.self)
    }
}

nonisolated private struct IOSConversationSettings: Encodable {
    static let ownedKeys = ["version", "language", "sessionMemory", "knowledgePoints", "wiki", "lastCompactedAt", "selectedPromptTemplateID", "dataSourceOverrides", "selectedModelInfo", "executionBackend", "externalExecutionSessions", "persistenceEnabled", "permissionMode", "compactedMemoryEntries", "promptResponseStyle"]
    let version: Int
    let language: String?
    let sessionMemory: String
    let knowledgePoints: String
    let wiki: String
    let lastCompactedAt: Date?
    let selectedPromptTemplateID: UUID?
    let dataSourceOverrides: [String: Bool]
    let promptResponseStyle: PromptResponseStyle?
    let executionBackend: String
    let externalExecutionSessions: [String: String]
    let persistenceEnabled: Bool
    let permissionMode: IOSPermissionMode?
    let compactedMemoryEntries: [CompactedMemoryEntry]

    init(_ conversation: Conversation) {
        version = 1
        language = conversation.language
        sessionMemory = conversation.sessionMemory
        knowledgePoints = conversation.knowledgePoints
        wiki = conversation.wiki
        lastCompactedAt = conversation.lastCompactedAt
        selectedPromptTemplateID = conversation.selectedPromptTemplateID
        dataSourceOverrides = conversation.dataSourceOverrides
        promptResponseStyle = conversation.promptResponseStyle
        executionBackend = conversation.executionBackend
        externalExecutionSessions = conversation.externalExecutionSessions
        persistenceEnabled = conversation.persistenceEnabled
        permissionMode = conversation.permissionMode
        compactedMemoryEntries = conversation.compactedMemoryEntries
    }

}

nonisolated private extension Date {
    var millisecondsSince1970: Int64 { Int64((timeIntervalSince1970 * 1_000).rounded()) }

    init(millisecondsSince1970: Int64) {
        self.init(timeIntervalSince1970: TimeInterval(millisecondsSince1970) / 1_000)
    }
}

// Wrapper to encode Encodable existential
nonisolated private struct AnyEncodable: Encodable {
    private let _encode: (Encoder) throws -> Void

    init(_ enc: Encodable) {
        self._encode = enc.encode
    }

    func encode(to encoder: Encoder) throws {
        try _encode(encoder)
    }
}
