// SPDX-License-Identifier: Apache-2.0
// Thin iOS adapter for the canonical Application command boundary.

import Foundation
import CryptoKit
#if canImport(NolaCore)
import NolaCore
#endif

enum IOSApplicationRuntimeError: Error {
    case applicationUnavailable
    case incompatibleABI(UInt32)
    case incompatibleServiceProtocol(expected: UInt32, actual: UInt32)
    case openFailed(String)
    case outcomePending(String)
    case invalidResponse
    case rejected(String)
    case revisionConflict(scope: String, expected: UInt64, actual: UInt64)
}

extension IOSApplicationRuntimeError: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .applicationUnavailable:
            return "The Nola Application runtime is unavailable."
        case .incompatibleABI(let version):
            return "The Nola Application runtime ABI is incompatible (received \(version))."
        case .incompatibleServiceProtocol(let expected, let actual):
            return "The Nola Application service protocol is incompatible (expected \(expected), received \(actual))."
        case .openFailed(let detail):
            return "The Nola Application store could not be opened: \(detail)"
        case .outcomePending(let requestID):
            return String(format: String(localized: "Request %@ was accepted, but its result is not confirmed. Refresh before submitting it again."), requestID)
        case .invalidResponse:
            return "The Nola Application runtime returned an invalid response."
        case .rejected(let detail):
            return AppleMainExpertDispatchRejection(rawValue: detail)?.localizedDescription ?? "The Nola Application command was rejected: \(detail)"
        case .revisionConflict(let scope, let expected, let actual):
            return String(format: String(localized: "The operation conflicted with a newer revision of %@ (%llu expected, %llu current)."), scope, expected, actual)
        }
    }
}

nonisolated enum IOSCanonicalDataSourceKind: String, Codable {
    case http
    case localFile = "local_file"
    case directory
    case platform
    case web
    case database
    case mcp
    case custom
}

nonisolated enum IOSCanonicalDataSourceLocationKind: String, Codable {
    case httpURL = "http_url"
    case localPath = "local_path"
    case platformResource = "platform_resource"
    case databaseReference = "database_reference"
    case mcpCapability = "mcp_capability"
    case opaqueReference = "opaque_reference"
}

nonisolated struct IOSCanonicalDataSourceConfiguration: Codable, Equatable {
    let connectorID: String
    let locationKind: IOSCanonicalDataSourceLocationKind
    let location: String
    let parameters: [String: AgentCoreDataSourceJSON]
    let secretReferences: [String]

    enum CodingKeys: String, CodingKey {
        case location, parameters
        case connectorID = "connector_id"
        case locationKind = "location_kind"
        case secretReferences = "secret_references"
    }
}

nonisolated enum IOSCanonicalDataSourceAuthorizationState: String, Codable {
    case notRequired = "not_required"
    case notDetermined = "not_determined"
    case authorized
    case denied
    case restricted
    case unavailable
}

nonisolated struct IOSCanonicalDataSourceAuthorization: Codable, Equatable {
    let state: IOSCanonicalDataSourceAuthorizationState
    let capability: String?
    let observedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case state, capability
        case observedAtMs = "observed_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSourceFreshnessPolicy: Codable, Equatable {
    let maximumAgeMinutes: UInt64
    let allowStaleFallback: Bool

    enum CodingKeys: String, CodingKey {
        case maximumAgeMinutes = "maximum_age_minutes"
        case allowStaleFallback = "allow_stale_fallback"
    }
}

nonisolated enum IOSCanonicalDataSourceLifecycle: String, Codable {
    case draft
    case ready
    case failed
}

nonisolated enum IOSCanonicalDataSourceReadStatus: String, Codable {
    case neverRead = "never_read"
    case pending
    case ready
    case empty
    case denied
    case unavailable
    case failed
}

nonisolated struct IOSCanonicalDataSourceContentReference: Codable, Equatable {
    let uri: String
    let mimeType: String
    let byteCount: UInt64
    let contentVersion: String

    enum CodingKeys: String, CodingKey {
        case uri
        case mimeType = "mime_type"
        case byteCount = "byte_count"
        case contentVersion = "content_version"
    }
}

nonisolated struct IOSCanonicalDataSourceReadState: Codable, Equatable {
    let status: IOSCanonicalDataSourceReadStatus
    let readToken: String?
    let contentReference: IOSCanonicalDataSourceContentReference?
    let observedAtMs: Int64?
    let schemaVersion: UInt32?
    let recordCount: UInt64?
    let diagnostic: String?
    let requestedAtMs: Int64?
    let completedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case status, diagnostic
        case readToken = "read_token"
        case contentReference = "content_reference"
        case observedAtMs = "observed_at_ms"
        case schemaVersion = "schema_version"
        case recordCount = "record_count"
        case requestedAtMs = "requested_at_ms"
        case completedAtMs = "completed_at_ms"
    }
}

nonisolated enum IOSCanonicalDataSourceValidationStatus: String, Codable {
    case notValidated = "not_validated"
    case pending
    case verified
    case verifiedEmpty = "verified_empty"
    case failed
}

nonisolated struct IOSCanonicalDataSourceValidation: Codable, Equatable {
    let status: IOSCanonicalDataSourceValidationStatus
    let validationToken: String?
    let contract: [String: AgentCoreDataSourceJSON]?
    let diagnostic: String?
    let attemptedAtMs: Int64?
    let completedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case status, contract, diagnostic
        case validationToken = "validation_token"
        case attemptedAtMs = "attempted_at_ms"
        case completedAtMs = "completed_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSource: Codable, Equatable {
    let id: String
    let name: String
    let kind: IOSCanonicalDataSourceKind
    let configuration: IOSCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: IOSCanonicalDataSourceAuthorization
    let validation: IOSCanonicalDataSourceValidation
    let readState: IOSCanonicalDataSourceReadState
    let freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy
    let defaultEnabled: Bool
    let lifecycle: IOSCanonicalDataSourceLifecycle
    let capabilities: [String]
    let privacy: String
    let revision: Int64
    let lastSuccessfulReadAtMs: Int64?
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, name, kind, configuration, authorization, validation, lifecycle
        case capabilities, privacy, revision
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case defaultEnabled = "default_enabled"
        case readState = "read_state"
        case lastSuccessfulReadAtMs = "last_successful_read_at_ms"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSourceDraft: Codable {
    let id: String
    let name: String
    let kind: IOSCanonicalDataSourceKind
    let configuration: IOSCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: IOSCanonicalDataSourceAuthorization
    let freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy
    let capabilities: [String]
    let privacy: String
    let createdAtMs: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, name, kind, configuration, authorization, capabilities, privacy
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSourceConfigurationUpdate: Codable {
    let name: String
    let kind: IOSCanonicalDataSourceKind
    let configuration: IOSCanonicalDataSourceConfiguration
    let accessPolicy: String
    let authorization: IOSCanonicalDataSourceAuthorization
    let freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy
    let capabilities: [String]
    let privacy: String
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case name, kind, configuration, authorization, capabilities, privacy
        case accessPolicy = "access_policy"
        case freshnessPolicy = "freshness_policy"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSourceSubmission {
    let draft: IOSCanonicalDataSourceDraft
    let validationContract: [String: AgentCoreDataSourceJSON]?
    let enabled: Bool
}

nonisolated struct IOSCanonicalProductSetting: Codable, Equatable {
    let key: String
    let valueJSON: String
    let revision: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case key, revision
        case valueJSON = "value_json"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct IOSCanonicalModelProfile: Codable, Equatable {
    let id: String
    let displayName: String
    let provider: String
    let modelName: String
    let runtime: String
    let sourceJSON: String
    let parametersJSON: String
    let selected: Bool
    let enabled: Bool
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, provider, runtime, selected, enabled, revision
        case displayName = "display_name"
        case modelName = "model_name"
        case sourceJSON = "source_json"
        case parametersJSON = "parameters_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalSyncChange: Codable, Equatable {
    let operationID: String
    let entityType: String
    let entityID: String
    let revision: Int64
    let deleted: Bool
    let payloadJSON: String
    let createdAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case revision, deleted
        case operationID = "operation_id"
        case entityType = "entity_type"
        case entityID = "entity_id"
        case payloadJSON = "payload_json"
        case createdAtMs = "created_at_ms"
    }
}

nonisolated struct IOSCanonicalSyncCursor: Codable, Equatable {
    let replicaID: String
    let cursor: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case cursor
        case replicaID = "replica_id"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct IOSCanonicalRemoteSyncChange: Codable, Equatable {
    let cursor: Int64
    let operationID: String
    let entityType: String
    let entityID: String
    let revision: Int64
    let deleted: Bool
    let payloadJSON: String
    let createdAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case cursor, revision, deleted
        case operationID = "operation_id"
        case entityType = "entity_type"
        case entityID = "entity_id"
        case payloadJSON = "payload_json"
        case createdAtMs = "created_at_ms"
    }
}

nonisolated struct IOSCanonicalRemoteSyncPageResult: Codable, Equatable {
    let applied: Int
    let skipped: Int
    let cursor: Int64
}

nonisolated struct IOSCanonicalAutomationDefinition: Codable, Equatable {
    let id: String
    let name: String
    let scheduleFormat: String
    let enabled: Bool
    let revision: Int64
    let executionSpec: IOSAutomationDefinitionSpec
    let metadataJSON: String
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case id, name, enabled, revision
        case executionSpec = "execution_spec"
        case scheduleFormat = "schedule_format"
        case metadataJSON = "metadata_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalAutomationRun: Codable, Equatable {
    let id: String
    let automationID: String
    let definitionRevision: Int64
    let occurrenceKey: String
    let trigger: String
    let status: String
    let attempt: Int64
    let workerID: String?
    let leaseToken: String?
    let leaseUntilMs: Int64?
    let scheduledForMs: Int64
    let startedAtMs: Int64?
    let finishedAtMs: Int64?
    let nextRetryAtMs: Int64?
    let resultJSON: String?
    let errorJSON: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, trigger, status, attempt
        case automationID = "automation_id"
        case definitionRevision = "definition_revision"
        case occurrenceKey = "occurrence_key"
        case workerID = "worker_id"
        case leaseToken = "lease_token"
        case leaseUntilMs = "lease_until_ms"
        case scheduledForMs = "scheduled_for_ms"
        case startedAtMs = "started_at_ms"
        case finishedAtMs = "finished_at_ms"
        case nextRetryAtMs = "next_retry_at_ms"
        case resultJSON = "result_json"
        case errorJSON = "error_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

nonisolated struct IOSCanonicalDataSourceUsage: Codable, Equatable {
    let id: String
    let sourceID: String
    let sourceTitle: String
    let consumer: String
    let purpose: String
    let conversationID: String?
    let disclosedToCloud: Bool?
    let metadata: [String: AgentCoreDataSourceJSON]
    let usedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, consumer, purpose
        case sourceID = "source_id"
        case sourceTitle = "source_title"
        case conversationID = "conversation_id"
        case disclosedToCloud = "disclosed_to_cloud"
        case metadata
        case usedAtMs = "used_at_ms"
    }
}

nonisolated struct IOSCanonicalMemoryRecord: Codable, Equatable {
    let record: IOSCanonicalSemanticMemoryRecord
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let lastUsedAtMs: Int64?
    let deletedAtMs: Int64?

    var id: String { record.id }

    enum CodingKeys: String, CodingKey {
        case record
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case lastUsedAtMs = "last_used_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalMemoryNamespace: Codable, Equatable {
    let kind: String
    let name: String?
}

nonisolated struct IOSCanonicalMemoryRetention: Codable, Equatable {
    let mode: String
    let expiresAt: String?

    enum CodingKeys: String, CodingKey {
        case mode
        case expiresAt = "expires_at"
    }
}

nonisolated struct IOSCanonicalMemoryProvenance: Codable, Equatable {
    let kind: String
    let source: String
    let sessionID: String?
    let turnID: String?
    let itemIDs: [String]
    let observedAt: String?

    enum CodingKeys: String, CodingKey {
        case kind, source
        case sessionID = "session_id"
        case turnID = "turn_id"
        case itemIDs = "item_ids"
        case observedAt = "observed_at"
    }
}

nonisolated struct IOSCanonicalSemanticMemoryRecord: Codable, Equatable {
    let id: String
    let owner: CoreMemoryOwner
    let namespace: IOSCanonicalMemoryNamespace
    let scope: String
    let kind: String
    let content: String
    let summary: String?
    let tags: [String]
    let confidence: Double
    let importance: Double
    let sensitivity: String?
    let retention: IOSCanonicalMemoryRetention
    let supersedes: [String]
    let provenance: IOSCanonicalMemoryProvenance
}

nonisolated struct IOSCanonicalMemoryCandidate: Codable, Equatable {
    let record: IOSCanonicalSemanticMemoryRecord
    let quality: IOSCanonicalMemoryQuality?
    let proposedAtMs: Int64

    var id: String { record.id }

    enum CodingKeys: String, CodingKey {
        case record, quality
        case proposedAtMs = "proposed_at_ms"
    }
}

nonisolated struct IOSCanonicalMemoryQuality: Codable, Equatable {
    let applicable: Bool
    let durable: Bool
    let legible: Bool
    let rationale: String
}

nonisolated struct IOSCanonicalSkill: Codable, Equatable {
    let id: String
    let source: String
    let version: String
    let displayName: String
    let summary: String
    let category: String
    let instructionsRef: String
    let instructionsDigest: String
    let instructionsText: String
    let requiredTools: [AgentCoreSkillToolReference]
    let defaultMode: AgentCoreSkillExecutionMode?
    let enabled: Bool
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, source, version, summary, category, enabled
        case displayName = "display_name"
        case instructionsRef = "instructions_ref"
        case instructionsDigest = "instructions_digest"
        case instructionsText = "instructions_text"
        case requiredTools = "required_tools"
        case defaultMode = "default_mode"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalSkillEditor: Codable, Equatable {
    let record: IOSCanonicalSkill
    let revision: Int64
}

nonisolated struct IOSCanonicalMCPServer: Codable, Equatable {
    let id: String
    let displayName: String
    let enabled: Bool
    let configurationJSON: String
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, enabled, revision
        case displayName = "display_name"
        case configurationJSON = "configuration_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }
}

nonisolated struct IOSCanonicalConversationPart: Codable, Equatable {
    let id: String
    let kind: String
    let ordinal: Int64
    let state: String
    let contentText: String
    let contentJSON: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, kind, ordinal, state
        case contentText = "content_text"
        case contentJSON = "content_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

extension IOSCanonicalConversationPart {
    nonisolated init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        kind = try values.decode(String.self, forKey: .kind)
        ordinal = try values.decode(Int64.self, forKey: .ordinal)
        state = try values.decode(String.self, forKey: .state)
        contentText = try values.decode(String.self, forKey: .contentText)
        contentJSON = try values.decodeIfPresent(String.self, forKey: .contentJSON)
        createdAtMs = try values.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        updatedAtMs = try values.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? createdAtMs
    }
}

nonisolated struct IOSCanonicalConversationMessage: Codable, Equatable {
    let id: String
    let runID: String?
    let role: String
    let status: String
    let ordinal: Int64
    let createdAtMs: Int64
    let completedAtMs: Int64?
    let parts: [IOSCanonicalConversationPart]

    enum CodingKeys: String, CodingKey {
        case id, role, status, ordinal, parts
        case runID = "run_id"
        case createdAtMs = "created_at_ms"
        case completedAtMs = "completed_at_ms"
    }
}

nonisolated struct IOSCanonicalConversation: Codable, Equatable {
    let id: String
    let title: String
    let revision: Int64
    var configurationRevision: Int64? = nil
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let settingsJSON: String?
    let messages: [IOSCanonicalConversationMessage]

    enum CodingKeys: String, CodingKey {
        case id, title, revision, messages
        case configurationRevision = "configuration_revision"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case settingsJSON = "settings_json"
    }
}

nonisolated struct IOSContextCompactionSource: Codable, Equatable {
    let conversation_id: String
    let message_count: Int
    let source_fingerprint: String
    let context_fingerprint: String
}

nonisolated struct IOSConfigurationCommitResult: Decodable {
    enum Status: String, Decodable { case committed, conflict, notFound = "not_found" }
    let status: Status
    let snapshot: [String: AgentCoreJSONValue]?
    let expected_revision: Int64?
    let actual_revision: Int64?
    let conversation_id: String?
    func requireCommitted() throws {
        guard status == .committed, snapshot != nil else {
            throw IOSApplicationRuntimeError.rejected(status == .conflict
                ? "Conversation configuration changed; your edit is retained. Reload before saving."
                : "Conversation is unavailable.")
        }
    }
}

nonisolated struct IOSCanonicalConversationSummary: Codable, Equatable {
    let id: String
    let title: String
    let revision: Int64
    let createdAtMs: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, title, revision
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

private nonisolated struct IOSCanonicalCreateConversationInput: Encodable {
    let id: String
    let title: String
    let settingsJSON: String?
    let createdAtMs: Int64
    let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, title
        case settingsJSON = "settings_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

private nonisolated struct IOSCanonicalImportConversationTranscriptInput: Encodable {
    let importID: String
    let source: String
    let conversationID: String
    let expectedMessageCount: Int
    let messages: [IOSCanonicalConversationMessage]
    let importedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case source, messages
        case importID = "import_id"
        case conversationID = "conversation_id"
        case expectedMessageCount = "expected_message_count"
        case importedAtMs = "imported_at_ms"
    }
}

private nonisolated struct IOSCanonicalCreateConversationResult: Decodable {
    let created: Bool
}

private nonisolated struct IOSCanonicalMessageWriteResult: Decodable {
    let appendedMessageCount: Int
    let messageCount: Int
    let conversationRevision: Int64
    enum CodingKeys: String, CodingKey {
        case appendedMessageCount = "appended_message_count"
        case messageCount = "message_count"
        case conversationRevision = "conversation_revision"
    }
}

nonisolated final class IOSApplicationRuntime: @unchecked Sendable {
    static let expectedABI: UInt32 = 1
    static let expectedServiceProtocol: UInt32 = 1
    static var isAvailable: Bool {
        #if canImport(NolaCore)
        nola_core_application_abi_version() == expectedABI
            && nola_core_application_service_protocol_version() == expectedServiceProtocol
        #else
        false
        #endif
    }
    static let defaultDatabaseURL: URL = {
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Nola", isDirectory: true)
        return root.appendingPathComponent("nola-product.db", isDirectory: false)
    }()
    private static let sharedLock = NSLock()
    nonisolated(unsafe) private static var installedShared: IOSApplicationRuntime?
    nonisolated(unsafe) private static var defaultSharedResult: Result<IOSApplicationRuntime, Error>?
    static var shared: IOSApplicationRuntime? {
        try? requireShared()
    }

    /// Preserve the original failure. Retry is explicit and never replaces or
    /// converts the product database when its schema is incompatible.
    static func requireShared(retry: Bool = false) throws -> IOSApplicationRuntime {
        sharedLock.lock()
        defer { sharedLock.unlock() }
        if let installedShared { return installedShared }
        if retry, case .failure? = defaultSharedResult { defaultSharedResult = nil }
        if let result = defaultSharedResult { return try result.get() }
        let result = Result { try IOSApplicationRuntime(databaseURL: defaultDatabaseURL) }
        defaultSharedResult = result
        return try result.get()
    }

    static func installShared(_ runtime: IOSApplicationRuntime?) {
        sharedLock.lock()
        installedShared = runtime
        sharedLock.unlock()
    }

    let databaseURL: URL
    let dataSourceContentDirectory: URL
    private let lock = NSLock()
    private let pendingCommandJournal: ApplePendingCommandJournal
    private let expertHostJournal: ApplePendingCommandJournal
    #if canImport(NolaCore)
    private var handle: OpaquePointer?
    #endif

    private let expertHostAcknowledgementObserver: (@Sendable (Data, Data) throws -> Void)?
    private let childPumpObserver: (@Sendable (AppleChildPump) -> Void)?

    init(databaseURL: URL, expertHostAcknowledgementObserver: (@Sendable (Data, Data) throws -> Void)? = nil,
         childPumpObserver: (@Sendable (AppleChildPump) -> Void)? = nil) throws {
        self.expertHostAcknowledgementObserver = expertHostAcknowledgementObserver
        self.childPumpObserver = childPumpObserver
        self.databaseURL = databaseURL.standardizedFileURL
        pendingCommandJournal = ApplePendingCommandJournal(databaseURL: databaseURL, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        expertHostJournal = ApplePendingCommandJournal(databaseURL: databaseURL, namespace: .expertHost, validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
        dataSourceContentDirectory = databaseURL.deletingLastPathComponent()
            .appendingPathComponent("DataSourceContent", isDirectory: true)
        #if canImport(NolaCore)
        let abi = nola_core_application_abi_version()
        guard abi == Self.expectedABI else { throw IOSApplicationRuntimeError.incompatibleABI(abi) }
        let serviceProtocol = nola_core_application_service_protocol_version()
        guard serviceProtocol == Self.expectedServiceProtocol else {
            throw IOSApplicationRuntimeError.incompatibleServiceProtocol(
                expected: Self.expectedServiceProtocol, actual: serviceProtocol
            )
        }
        try FileManager.default.createDirectory(
            at: databaseURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let path = Data(databaseURL.path.utf8)
        handle = path.withUnsafeBytes { bytes in
            nola_core_application_open(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard handle != nil else {
            let detail = nola_core_application_last_open_error().map { String(cString: $0) } ?? "unknown Core error"
            throw IOSApplicationRuntimeError.openFailed(
                "\(databaseURL.path): \(detail)"
            )
        }
        do {
            try recoverPendingCommands()
        } catch {
            if let handle { nola_core_application_free(handle) }
            handle = nil
            throw error
        }
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    deinit {
        #if canImport(NolaCore)
        if let handle { nola_core_application_free(handle) }
        #endif
    }

    func saveDataSource(
        _ submission: IOSCanonicalDataSourceSubmission,
        expectedRevision: Int64? = nil
    ) throws -> IOSCanonicalDataSource {
        let staged: IOSCanonicalDataSource
        if let expectedRevision {
            staged = try execute(DataSourceCommandEnvelope(command: UpdateDataSourceCommand(
                type: "update_configuration",
                id: submission.draft.id,
                expectedRevision: expectedRevision,
                input: IOSCanonicalDataSourceConfigurationUpdate(
                    name: submission.draft.name,
                    kind: submission.draft.kind,
                    configuration: submission.draft.configuration,
                    accessPolicy: submission.draft.accessPolicy,
                    authorization: submission.draft.authorization,
                    freshnessPolicy: submission.draft.freshnessPolicy,
                    capabilities: submission.draft.capabilities,
                    privacy: submission.draft.privacy,
                    updatedAtMs: submission.draft.updatedAtMs
                )
            )))
        } else {
            staged = try execute(DataSourceCommandEnvelope(command: DataSourceInputCommand(
                type: "create_draft",
                input: submission.draft
            )))
        }
        guard let contract = submission.validationContract else { return staged }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let pending: IOSCanonicalDataSource = try execute(
            DataSourceCommandEnvelope(command: RequestDataSourceValidationCommand(
                type: "request_validation",
                id: staged.id,
                expectedRevision: staged.revision,
                requestedAtMs: now
            ))
        )
        guard let validationToken = pending.validation.validationToken else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let validated: IOSCanonicalDataSource = try execute(
            DataSourceCommandEnvelope(command: RecordDataSourceValidationCommand(
                type: "record_validation",
                id: staged.id,
                validationToken: validationToken,
                result: DataSourceValidationSuccess(status: "succeeded", contract: contract),
                recordedAtMs: now
            ))
        )
        return try execute(DataSourceCommandEnvelope(command: SetDataSourceEnabledCommand(
            type: "set_default_enabled",
            id: validated.id,
            expectedRevision: validated.revision,
            enabled: submission.enabled,
            updatedAtMs: now
        )))
    }

    func listDataSources() throws -> [IOSCanonicalDataSource] {
        try execute(DataSourceCommandEnvelope(command: TypeCommand(type: "list")))
    }

    func putProductSetting(key: String, valueJSON: String, updatedAtMs: Int64) throws -> IOSCanonicalProductSetting {
        try executeMutation(ProductSettingWriteCommand(
            type: "put_product_setting",
            key: key,
            valueJSON: valueJSON,
            updatedAtMs: updatedAtMs
        ))
    }

    func productSetting(key: String) throws -> IOSCanonicalProductSetting? {
        try executeOptional(ProductSettingKeyCommand(type: "get_product_setting", key: key))
    }

    func putProductSettingChecked(key: String, valueJSON: String, expectedRevision: Int64?,
                                  updatedAtMs: Int64) throws -> IOSCanonicalProductSetting {
        struct Checked: Encodable {
            let type = "put_product_setting_checked"
            let key: String
            let valueJSON: String
            let expectedRevision: Int64?
            let updatedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type, key
                case valueJSON = "value_json"
                case expectedRevision = "expected_revision"
                case updatedAtMs = "updated_at_ms"
            }
        }
        return try executeMutation(Checked(key: key, valueJSON: valueJSON,
            expectedRevision: expectedRevision, updatedAtMs: updatedAtMs))
    }

    /// Core owns strict version/tag validation; no client-side legacy fallback.
    func validateDefaultChatModel(valueJSON: String) throws {
        struct Resolve: Encodable {
            let type = "resolve_chat_model"
            let intent = ["kind": "inherit"]
            let defaultValueJSON: String
            let candidates: [String] = []
            enum CodingKeys: String, CodingKey {
                case type, intent, candidates
                case defaultValueJSON = "default_value_json"
            }
        }
        struct Resolution: Decodable { let kind: String }
        let result: Resolution = try execute(Resolve(defaultValueJSON: valueJSON))
        guard result.kind != "invalid_configuration" else {
            throw IOSApplicationRuntimeError.rejected("Invalid default chat model configuration.")
        }
    }

    @discardableResult
    func deleteProductSetting(key: String) throws -> Bool {
        try executeMutation(ProductSettingKeyCommand(type: "delete_product_setting", key: key))
    }

    func upsertModelProfile(_ input: IOSCanonicalModelProfile) throws {
        let _: Bool? = try executeOptional(ModelCommandEnvelope(command: ModelInputCommand(type: "upsert_profile", input: input)))
    }

    /// Publishes a complete profile against its observed revision (zero means absent).
    /// The caller preserves fields it is not editing. Only a confirmed conflict or
    /// rejection proves that newly staged credentials were not published; transport
    /// failures, pending outcomes and invalid receipts must retain staged credentials.
    @discardableResult
    func upsertModelProfileCAS(_ input: IOSCanonicalModelProfile, expectedRevision: UInt64,
                               requestID: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let receipt = try submitReceipt(
            ModelCommandEnvelope(command: ModelInputCommand(type: "upsert_profile", input: input)),
            expectedRevision: expectedRevision, requestID: requestID)
        guard let revision = receipt["committed_revision"] as? NSNumber,
              let committedRevision = UInt64(revision.stringValue), committedRevision > 0 else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return committedRevision
    }

    func resetModelParameters(profileIDs: [String], parametersJSON: String, updatedAtMs: Int64) throws {
        struct Reset: Encodable {
            let type = "reset_parameters"
            let profileIDs: [String]
            let parametersJSON: String
            let updatedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type
                case profileIDs = "profile_ids"
                case parametersJSON = "parameters_json"
                case updatedAtMs = "updated_at_ms"
            }
        }
        let _: Bool? = try executeOptional(ModelCommandEnvelope(command: Reset(
            profileIDs: profileIDs, parametersJSON: parametersJSON, updatedAtMs: updatedAtMs
        )))
    }

    /// Returns the revision from this write's durable receipt, not a later read.
    func configureModelParameters(_ input: IOSCanonicalModelProfile, expectedRevision: UInt64,
                                  requestID: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let receipt = try submitReceipt(ModelCommandEnvelope(command: ModelInputCommand(type: "configure_parameters", input: input)), expectedRevision: expectedRevision, requestID: requestID)
        guard let revision = receipt["committed_revision"] as? NSNumber else { throw IOSApplicationRuntimeError.invalidResponse }
        return revision.uint64Value
    }

    /// A fixed, pure Application query. It neither creates a command receipt nor
    /// stages a pending mutation file. Route/preference validation belongs to Core.
    func admitHostOutput(routes: [AgentCoreJSONValue], preference: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        #if canImport(NolaCore)
        struct Arguments: Encodable {
            let routes: [AgentCoreJSONValue]
            let preference: AgentCoreJSONValue
        }
        let arguments = try JSONSerialization.jsonObject(with:
            JSONEncoder().encode(Arguments(routes: routes, preference: preference)))
        let requestID = UUID().uuidString.lowercased()
        let input: [String: Any] = [
            "protocol_version": nola_core_application_service_protocol_version(),
            "client_id": "ios-application", "request_id": requestID,
            "command": ["type": "admit_host_output", "arguments": arguments],
        ]
        lock.lock()
        defer { lock.unlock() }
        let receipt = try transmitEnvelope(input)
        switch receipt["status"] as? String {
        case "committed":
            guard let result = receipt["result"] else { throw IOSApplicationRuntimeError.invalidResponse }
            return try JSONDecoder().decode(AgentCoreJSONValue.self,
                from: JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]))
        case "rejected":
            throw IOSApplicationRuntimeError.rejected(
                (receipt["error"] as? [String: Any])?["message"] as? String ?? "Output admission rejected")
        default:
            throw IOSApplicationRuntimeError.outcomePending(requestID)
        }
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    private func executeMutation<Command: Encodable, Result: Decodable>(_ command: Command,
                          requestID: String = UUID().uuidString.lowercased()) throws -> Result {
        let receipt = try submitReceipt(command, requestID: requestID)
        guard let result = receipt["result"] else { throw IOSApplicationRuntimeError.invalidResponse }
        return try JSONDecoder().decode(Result.self, from: JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]))
    }

    private func submitReceipt<Command: Encodable>(_ command: Command, expectedRevision: UInt64? = nil, requestID: String) throws -> [String: Any] {
        #if canImport(NolaCore)
        let clientID = "ios-application"
        let encoded = try JSONEncoder().encode(command)
        guard var arguments = try JSONSerialization.jsonObject(with: encoded) as? [String: Any], let type = arguments.removeValue(forKey: "type") as? String else { throw IOSApplicationRuntimeError.invalidResponse }
        let version = nola_core_application_service_protocol_version()
        var envelopeInput: [String: Any] = [
            "protocol_version": version, "client_id": clientID, "request_id": requestID,
            "command": ["type": type, "arguments": arguments],
        ]
        if let expectedRevision { envelopeInput["expected_revision"] = expectedRevision }
        if expectedRevision == nil, Self.isReadOnlyCommand(type: type, arguments: arguments) {
            // Observation never opens/changes the mutation journal, including
            // when another command's durable outcome is still uncertain.
            lock.lock()
            defer { lock.unlock() }
            let receipt = try transmitEnvelope(envelopeInput)
            switch receipt["status"] as? String {
            case "committed": return receipt
            case "rejected":
                throw IOSApplicationRuntimeError.rejected((receipt["error"] as? [String: Any])?["message"] as? String ?? "Query rejected")
            default: throw IOSApplicationRuntimeError.outcomePending(requestID)
            }
        }
        lock.lock()
        defer { lock.unlock() }
        let journal = try pendingCommandJournal.beginTransaction()
        defer { journal.close() }
        var pending = try journal.load()
        // Retain the exact original envelope while its outcome is uncertain.
        // A repeated user operation after a terminal receipt receives a new ID.
        let fingerprint = try commandFingerprint(envelopeInput)
        if let retained = pending[requestID], try commandFingerprint(retained) != fingerprint {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        var recoveringOriginal = false
        if let existing = try pending.first(where: { try commandFingerprint($0.value) == fingerprint }) {
            envelopeInput = existing.value
            recoveringOriginal = true
        } else if let unresolved = try IOSPendingCommandOwner.blockingRequest(
            for: envelopeInput, pending: pending, resolve: resolvePendingOwner) {
            // A dependent write must never overtake an uncertain predecessor.
            throw IOSApplicationRuntimeError.outcomePending(unresolved)
        }
        let requestID = envelopeInput["request_id"] as! String
        pending[requestID] = envelopeInput
        try journal.save(pending)
        let receipt: [String: Any]
        if recoveringOriginal { receipt = try recoverEnvelope(envelopeInput) }
        else { receipt = try transmitEnvelope(envelopeInput) }
        try validateUnknownEffectReceipt(receipt, envelope: envelopeInput)
        if ["committed", "conflict", "rejected"].contains(receipt["status"] as? String ?? "") {
            pending.removeValue(forKey: requestID)
            try journal.save(pending)
        }
        switch receipt["status"] as? String {
        case "committed":
            return receipt
        case "accepted":
            throw IOSApplicationRuntimeError.outcomePending(requestID)
        case "conflict":
            guard let conflict = receipt["conflict"] as? [String: Any],
                  let scope = conflict["scope_id"] as? String,
                  let expected = conflict["expected_revision"] as? NSNumber,
                  let actual = conflict["actual_revision"] as? NSNumber else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            throw IOSApplicationRuntimeError.revisionConflict(scope: scope, expected: expected.uint64Value, actual: actual.uint64Value)
        case "rejected":
            throw IOSApplicationRuntimeError.rejected((receipt["error"] as? [String: Any])?["message"] as? String ?? "Command rejected")
        default:
            throw IOSApplicationRuntimeError.invalidResponse
        }
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    private func commandFingerprint(_ envelope: [String: Any]) throws -> Data {
        var identity = envelope
        identity.removeValue(forKey: "request_id")
        return try JSONSerialization.data(withJSONObject: identity, options: [.sortedKeys])
    }

    /// Called only with the runtime lock held. This read bypasses mutation
    /// journaling and cannot recursively acquire submitReceipt's NSLock.
    private func resolvePendingOwner(_ owner: IOSPendingCommandOwner) throws -> IOSPendingCommandOwner {
        guard case .run(let runID) = owner else { return owner }
        let query: [String: Any] = ["protocol_version": Self.expectedServiceProtocol,
            "client_id": "ios-application", "request_id": UUID().uuidString.lowercased(),
            "command": ["type": "load_run_identity", "arguments": ["run_id": runID]]]
        let receipt = try transmitEnvelope(query)
        guard receipt["status"] as? String == "committed", let result = receipt["result"] as? [String: Any],
              result["run_id"] as? String == runID, let context = result["conversation_id"] as? String,
              !context.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        return .conversation(context)
    }

    /// Replays retained transport envelopes after interrupted delivery. Core
    /// returns the original receipt for writes already committed before a crash.
    @discardableResult
    func recoverPendingCommands() throws -> [[String: Any]] {
        lock.lock()
        defer { lock.unlock() }
        let journal = try pendingCommandJournal.beginTransaction()
        defer { journal.close() }
        var pending = try journal.load()
        var receipts: [[String: Any]] = []
        for id in pending.keys.sorted() {
            if let envelope = pending[id], envelope["expected_revision"] == nil,
               let command = envelope["command"] as? [String: Any],
               let type = command["type"] as? String,
               let arguments = command["arguments"] as? [String: Any],
               Self.isReadOnlyCommand(type: type, arguments: arguments) {
                // A current-protocol observation has no uncertain write outcome.
                // Its owner will query afresh; never retain it as a mutation fence.
                pending.removeValue(forKey: id)
                try journal.save(pending)
                continue
            }
            // A08 needs its owning ChatManager to consume the reply and either
            // inspect the seal or resume the exact Run. Startup must not discard
            // that decision receipt before its owner can observe it.
            let receipt: [String: Any]
            do {
                if try Self.unknownEffectIntent(pending[id]!) != nil { continue }
                receipt = try recoverEnvelope(pending[id]!)
                try validateUnknownEffectReceipt(receipt, envelope: pending[id]!)
            } catch {
                // This owner's unconfirmed envelope stays byte-for-byte intact.
                // Other owners and read-only observations remain available.
                continue
            }
            receipts.append(receipt)
            if ["committed", "conflict", "rejected"].contains(receipt["status"] as? String ?? "") {
                pending.removeValue(forKey: id)
                // Storage durability failures remain fatal, unlike a single
                // command's recoverable/uncertain transport outcome.
                try journal.save(pending)
            }
        }
        return receipts
    }

    func inspectUnknownEffectRecovery(conversationID: String, runID: String,
                                      assistantMessageID: String) throws -> IOSUnknownEffectRecoveryInspection? {
        guard [conversationID, runID, assistantMessageID].allSatisfy(IOSUnknownEffectRecoveryDecision.validID) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let result: IOSUnknownEffectRecoveryInspection? = try executeOptional(AgentCoreJSONValue.object([
            "type": .string("inspect_unknown_effect_recovery"), "conversation_id": .string(conversationID),
            "expected_run_id": .string(runID), "assistant_message_id": .string(assistantMessageID),
        ]))
        if let result {
            guard result.conversation_id == conversationID, result.run_id == runID,
                  result.assistant_message_id == assistantMessageID, result.targets.count <= 256 else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
        return result
    }

    /// Fixed once at explicit confirmation. A pending write must be recovered,
    /// never rebuilt with a new timestamp or resolution ID.
    func resolveUnknownEffectRecovery(_ decision: IOSUnknownEffectRecoveryDecision,
                                      requestID: String) throws -> IOSUnknownEffectRecoveryReply {
        try decision.validate()
        guard IOSUnknownEffectRecoveryDecision.validID(requestID) else { throw IOSApplicationRuntimeError.invalidResponse }
        let reply: IOSUnknownEffectRecoveryReply = try executeMutation(
            InputCommand(type: "resolve_unknown_effect_recovery", input: decision), requestID: requestID)
        try reply.validate(for: decision)
        return reply
    }

    func inspectOwnedTaskRecovery(_ owner: AppleOwnedTaskRecoveryOwner) throws -> IOSUnknownEffectRecoveryInspection? {
        struct Query: Encodable { let type = "inspect_owned_task_recovery"; let owner: AppleOwnedTaskRecoveryOwner }
        return try executeOptional(Query(owner: owner))
    }

    func resolveOwnedTaskRecovery(_ owner: AppleOwnedTaskRecoveryOwner,
                                  decision: IOSUnknownEffectRecoveryDecision) throws -> IOSUnknownEffectRecoveryReply {
        try decision.validate()
        guard decision.choice.kind == .keepUnknownBlocked else { throw IOSApplicationRuntimeError.invalidResponse }
        struct Command: Encodable {
            let type = "resolve_owned_task_recovery"
            let owner: AppleOwnedTaskRecoveryOwner
            let input: IOSUnknownEffectRecoveryDecision
        }
        let reply: IOSUnknownEffectRecoveryReply = try executeMutation(Command(owner: owner, input: decision),
            requestID: decision.resolution_id)
        try reply.validate(for: decision)
        return reply
    }

    func pendingOwnedTaskRecovery(_ owner: AppleOwnedTaskRecoveryOwner) throws -> IOSPendingUnknownEffectRecovery? {
        lock.lock(); defer { lock.unlock() }
        let journal = try pendingCommandJournal.beginTransaction()
        defer { journal.close() }
        return try journal.load().values.compactMap { envelope in
            guard let command = envelope["command"] as? [String: Any],
                  command["type"] as? String == "resolve_owned_task_recovery",
                  let arguments = command["arguments"] as? [String: Any], let rawOwner = arguments["owner"],
                  try JSONDecoder().decode(AppleOwnedTaskRecoveryOwner.self,
                      from: JSONSerialization.data(withJSONObject: rawOwner)) == owner else { return nil }
            return try Self.unknownEffectIntent(envelope)
        }.first
    }

    func pendingUnknownEffectRecovery(conversationID: String) throws -> IOSPendingUnknownEffectRecovery? {
        lock.lock()
        defer { lock.unlock() }
        let journal = try pendingCommandJournal.beginTransaction()
        defer { journal.close() }
        let pending = try journal.load()
        let matches = try pending.values.filter { try IOSPendingCommandOwner.decode($0) == .conversation(conversationID) }
            .compactMap { try Self.unknownEffectIntent($0) }
            .filter { $0.decision.conversation_id == conversationID }
        guard matches.count <= 1 else { throw IOSApplicationRuntimeError.invalidResponse }
        return matches.first
    }

    /// Sends only the original durable envelope. No fresh permission decision,
    /// command identity, timestamp, or inspection is manufactured here.
    func recoverPendingUnknownEffectRecovery(_ expected: IOSPendingUnknownEffectRecovery) throws -> IOSUnknownEffectRecoveryReply {
        lock.lock()
        defer { lock.unlock() }
        let journal = try pendingCommandJournal.beginTransaction()
        defer { journal.close() }
        var pending = try journal.load()
        guard let envelope = pending[expected.requestID],
              try Self.unknownEffectIntent(envelope) == expected else { throw IOSApplicationRuntimeError.invalidResponse }
        let receipt = try recoverEnvelope(envelope)
        try validateUnknownEffectReceipt(receipt, envelope: envelope)
        guard receipt["status"] as? String == "committed", let result = receipt["result"] else {
            if ["conflict", "rejected"].contains(receipt["status"] as? String ?? "") {
                pending.removeValue(forKey: expected.requestID)
                try journal.save(pending)
                throw IOSApplicationRuntimeError.rejected("Recovery command rejected; inspect the exact Run again")
            }
            throw IOSApplicationRuntimeError.outcomePending(expected.requestID)
        }
        let reply = try JSONDecoder().decode(IOSUnknownEffectRecoveryReply.self,
            from: JSONSerialization.data(withJSONObject: result))
        pending.removeValue(forKey: expected.requestID)
        try journal.save(pending)
        return reply
    }

    static func unknownEffectIntent(_ envelope: [String: Any]) throws -> IOSPendingUnknownEffectRecovery? {
        guard let command = envelope["command"] as? [String: Any],
              ["resolve_unknown_effect_recovery", "resolve_owned_task_recovery"].contains(command["type"] as? String ?? "") else { return nil }
        guard let requestID = envelope["request_id"] as? String,
              IOSUnknownEffectRecoveryDecision.validID(requestID),
              envelope["client_id"] as? String == "ios-application",
              (envelope["protocol_version"] as? NSNumber)?.isEqual(to: NSNumber(value: Self.expectedServiceProtocol)) == true,
              let arguments = command["arguments"] as? [String: Any], let input = arguments["input"] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let decision = try JSONDecoder().decode(IOSUnknownEffectRecoveryDecision.self,
            from: JSONSerialization.data(withJSONObject: input))
        try decision.validate()
        return .init(requestID: requestID, decision: decision)
    }

    private func validateUnknownEffectReceipt(_ receipt: [String: Any], envelope: [String: Any]) throws {
        guard let intent = try Self.unknownEffectIntent(envelope), receipt["status"] as? String == "committed" else { return }
        guard let result = receipt["result"] else { throw IOSApplicationRuntimeError.invalidResponse }
        let reply = try JSONDecoder().decode(IOSUnknownEffectRecoveryReply.self,
            from: JSONSerialization.data(withJSONObject: result))
        try reply.validate(for: intent.decision)
    }

    /// Query the original command's receipt first. Only a confirmed absence may
    /// replay the exact stored request. Query failure is not absence evidence.
    private func recoverEnvelope(_ input: [String: Any]) throws -> [String: Any] {
        try Self.recoverApplicationEnvelope(input, transmit: transmitEnvelope)
    }

    static func recoverApplicationEnvelope(_ input: [String: Any],
        transmit: ([String: Any]) throws -> [String: Any]) throws -> [String: Any] {
        guard let requestID = input["request_id"] as? String else { throw IOSApplicationRuntimeError.invalidResponse }
        let query: [String: Any] = ["protocol_version": Self.expectedServiceProtocol,
            "client_id": "ios-application", "request_id": UUID().uuidString.lowercased(),
            "command": ["type": "command_receipt", "arguments": ["request_id": requestID]]]
        let lookup = try transmit(query)
        try Self.validateApplicationReceipt(lookup, input: query)
        guard lookup["status"] as? String == "committed", let result = lookup["result"] else {
            throw IOSApplicationRuntimeError.outcomePending(requestID)
        }
        if result is NSNull {
            let replay = try transmit(input)
            try Self.validateApplicationReceipt(replay, input: input)
            return replay
        }
        guard let receipt = result as? [String: Any] else { throw IOSApplicationRuntimeError.outcomePending(requestID) }
        try Self.validateApplicationReceipt(receipt, input: input)
        return receipt
    }

    private func transmitEnvelope(_ input: [String: Any]) throws -> [String: Any] {
        #if canImport(NolaCore)
        let requestID = input["request_id"] as? String ?? ""
        guard let handle else { throw IOSApplicationRuntimeError.applicationUnavailable }
        let data = try JSONSerialization.data(withJSONObject: input, options: [.sortedKeys])
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_application_submit(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { throw IOSApplicationRuntimeError.outcomePending(requestID) }
        defer { nola_core_free_c_string(pointer) }
        let response = Data(String(cString: pointer).utf8)
        do { return try Self.decodeApplicationResponse(response, input: input) }
        catch {
            // Shape only: never log command arguments, credential-bearing results,
            // provider errors or user text. The original intent remains pending.
            NSLog("[IOSApplicationReceipt] %@", Self.receiptFailureDiagnostic(response, input: input))
            throw error
        }
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    /// Private native factory only. Its journal is physically separate from
    /// renderer/Application intents; restoring ordinary JSON cannot elevate it.
    func expertHostOperation<Result: Decodable>(_ operation: AppleExpertHostOperation<AgentCoreJSONValue>) throws -> Result {
        guard let original = try JSONSerialization.jsonObject(with: JSONEncoder().encode(operation)) as? [String: Any] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return try executeExpertHostOperation(original, isReadOnly: operation.isReadOnly)
    }

    /// A scoped manifest may prove commit before its transport acknowledgement
    /// was retained. Replay only the exact private-namespace operation to retire
    /// that uncertainty; never rebuild the old context from today's settings.
    func reconcilePendingExpertAdmission(_ input: AppleAdmitExpertConversationMessage) throws -> AppleExpertMessageManifest<AgentCoreJSONValue>? {
        let transaction = try expertHostJournal.beginTransaction()
        let pending: [String: [String: Any]]
        do { pending = try transaction.load(); transaction.close() }
        catch { transaction.close(); throw error }
        let expectedInput = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input))
        let currentAuthority = try JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertTemplateAuthority.local()))
        for (id, envelope) in pending {
            guard envelope["request_id"] as? String == id, envelope["client_id"] as? String == "native-expert-host",
                  (envelope["protocol_version"] as? NSNumber)?.isEqual(to: NSNumber(value: Self.expectedServiceProtocol)) == true else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            let operation = try AppleExpertHostIntentScope.operation(in: envelope)
            guard operation["kind"] as? String == "admit", let saved = operation["input"] as? [String: Any],
                  NSDictionary(dictionary: saved).isEqual(expectedInput),
                  NSDictionary(dictionary: try AppleExpertHostIntentScope.authority(operation)).isEqual(currentAuthority) else { continue }
            return try executeExpertHostOperation(operation, isReadOnly: false)
        }
        return nil
    }

    /// Reconcile the original durable admission intent even when Core already
    /// exposes its material. Existence alone is never a commit acknowledgement.
    func reconcilePendingExpertPeerAdmission(_ input: AppleExpertPeerInput, authority: AppleExpertTemplateAuthority) throws -> AppleExpertPeerMaterial<AgentCoreJSONValue>? {
        let transaction = try expertHostJournal.beginTransaction()
        let pending: [String: [String: Any]]
        do { pending = try transaction.load(); transaction.close() }
        catch { transaction.close(); throw error }
        let expectedAuthority = try JSONSerialization.jsonObject(with: JSONEncoder().encode(authority))
        for (id, envelope) in pending {
            guard envelope["request_id"] as? String == id,
                  envelope["client_id"] as? String == "native-expert-host",
                  (envelope["protocol_version"] as? NSNumber)?.isEqual(to: NSNumber(value: Self.expectedServiceProtocol)) == true else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            let operation = try AppleExpertHostIntentScope.operation(in: envelope)
            guard operation["kind"] as? String == "admit_peer" else { continue }
            let saved = try JSONDecoder().decode(AppleExpertPeerInput.self,
                from: JSONSerialization.data(withJSONObject: operation["input"] ?? NSNull()))
            guard saved == input,
                  NSDictionary(dictionary: try AppleExpertHostIntentScope.authority(operation)).isEqual(expectedAuthority) else { continue }
            return try executeExpertHostOperation(operation, isReadOnly: false)
        }
        return nil
    }

    private func executeExpertHostOperation<Result: Decodable>(_ original: [String: Any], isReadOnly: Bool) throws -> Result {
        #if canImport(NolaCore)
        lock.lock()
        defer { lock.unlock() }
        guard let handle else { throw IOSApplicationRuntimeError.applicationUnavailable }
        let transaction = isReadOnly ? nil : try expertHostJournal.beginTransaction()
        defer { transaction?.close() }
        var pending = try transaction?.load() ?? [:]
        var requestID = UUID().uuidString.lowercased()
        var actual = original
        let authority = try AppleExpertHostIntentScope.authority(original)
        let currentAuthority = try JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertTemplateAuthority.local()))
        guard NSDictionary(dictionary: authority).isEqual(currentAuthority) else {
            throw IOSApplicationRuntimeError.rejected("Expert template actor changed")
        }
        if !isReadOnly {
            let expected = try AppleExpertHostIntentScope.stableIdentity(original)
            for (id, envelope) in pending {
                guard envelope["request_id"] as? String == id, envelope["client_id"] as? String == "native-expert-host",
                      (envelope["protocol_version"] as? NSNumber)?.isEqual(to: NSNumber(value: Self.expectedServiceProtocol)) == true else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                let retained = try AppleExpertHostIntentScope.operation(in: envelope)
                if try AppleExpertHostIntentScope.stableIdentity(retained) == expected {
                    requestID = id; actual = retained
                } else if AppleExpertHostIntentScope.intersects(original, retained) {
                    throw IOSApplicationRuntimeError.outcomePending(id)
                }
            }
            pending[requestID] = ["request_id": requestID, "client_id": "native-expert-host",
                "protocol_version": Self.expectedServiceProtocol,
                "command": ["type": "host_expert_template_operation", "arguments": ["operation": actual]]]
            try transaction?.save(pending)
        }
        var confirmed = false
        do {
            let bytes = try JSONSerialization.data(withJSONObject: ["request_id": requestID, "operation": actual], options: [.sortedKeys])
            let pointer = bytes.withUnsafeBytes {
                nola_core_application_expert_host_operation(handle, $0.bindMemory(to: UInt8.self).baseAddress, $0.count)
            }
            guard let pointer else { throw IOSApplicationRuntimeError.outcomePending(requestID) }
            defer { nola_core_free_c_string(pointer) }
            guard let reply = try JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as? [String: Any],
                  let ok = reply["ok"] as? Bool else { throw IOSApplicationRuntimeError.invalidResponse }
            let failure = reply["error"] as? [String: Any]
            guard (reply["request_id"] as? String ?? failure?["request_id"] as? String) == requestID else {
                throw IOSApplicationRuntimeError.outcomePending(requestID)
            }
            if !ok, failure?["outcome_pending"] as? Bool == true { throw IOSApplicationRuntimeError.outcomePending(requestID) }
            if !ok, failure?["outcome_pending"] as? Bool != false { throw IOSApplicationRuntimeError.outcomePending(requestID) }
            let decoded: Result? = ok ? try JSONDecoder().decode(Result.self,
                from: JSONSerialization.data(withJSONObject: reply["result"] ?? NSNull(), options: [.fragmentsAllowed])) : nil
            if ok, let expertHostAcknowledgementObserver {
                try expertHostAcknowledgementObserver(
                    JSONSerialization.data(withJSONObject: actual, options: [.sortedKeys]),
                    JSONSerialization.data(withJSONObject: reply, options: [.sortedKeys]))
            }
            if ok, !isReadOnly {
                let resultData = try JSONSerialization.data(withJSONObject: reply["result"] ?? NSNull())
                switch actual["kind"] as? String {
                case "admit":
                    guard let input = actual["input"] else { throw IOSApplicationRuntimeError.invalidResponse }
                    let inputData = try JSONSerialization.data(withJSONObject: input)
                    let expected = try JSONDecoder().decode(AppleAdmitExpertConversationMessage.self, from: inputData)
                    let manifest = try JSONDecoder().decode(AppleExpertMessageManifest<AgentCoreJSONValue>.self, from: resultData)
                    guard manifest.version == 1, manifest.input == expected,
                          manifest.entries.map(\.participant) == expected.selected_participants,
                          Set(manifest.entries.map(\.child_submission_id)).count == manifest.entries.count else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                case "prepare_edited_resend":
                    guard let input = actual["input"] else { throw IOSApplicationRuntimeError.invalidResponse }
                    let inputData = try JSONSerialization.data(withJSONObject: input)
                    let expected = try JSONDecoder().decode(ApplePrepareEditedResend.self, from: inputData)
                    let prepared = try JSONDecoder().decode(ApplePreparedExpertResend<AgentCoreJSONValue>.self, from: resultData)
                    guard prepared.input == expected, !prepared.entries.isEmpty,
                          Set(prepared.entries.map { $0.prepared.run_id }).count == prepared.entries.count else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                default: try IOSExpertHostMutationValidation.validate(resultData, operation: actual)
                }
            }
            if transaction != nil {
                pending.removeValue(forKey: requestID)
                try transaction?.save(pending)
            }
            confirmed = true
            guard ok else { throw IOSApplicationRuntimeError.rejected(failure?["message"] as? String ?? "Expert template authorization failed") }
            guard let decoded else { throw IOSApplicationRuntimeError.invalidResponse }
            return decoded
        } catch {
            if !isReadOnly && !confirmed { throw IOSApplicationRuntimeError.outcomePending(requestID) }
            throw error
        }
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    /// Trusted native Host factory entry. Never expose this through renderer command forwarding.
    func attestEditedResendHostPolicy(conversationID: String, coreConfig: AgentCoreJSONValue, submission: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        #if canImport(NolaCore)
        let target = Data(conversationID.utf8)
        let config = try JSONEncoder().encode(AgentCoreJSONValue.object([
            "core_config": coreConfig, "submission": submission,
        ]))
        lock.lock()
        defer { lock.unlock() }
        guard let handle else { throw IOSApplicationRuntimeError.applicationUnavailable }
        let pointer = target.withUnsafeBytes { targetBytes in
            config.withUnsafeBytes { configBytes in
                nola_core_application_attest_resend_host_policy(handle,
                    targetBytes.bindMemory(to: UInt8.self).baseAddress, targetBytes.count,
                    configBytes.bindMemory(to: UInt8.self).baseAddress, configBytes.count)
            }
        }
        guard let pointer else { throw IOSApplicationRuntimeError.invalidResponse }
        defer { nola_core_free_c_string(pointer) }
        struct Reply: Decodable { let ok: Bool; let result: AgentCoreJSONValue? }
        let reply = try JSONDecoder().decode(Reply.self, from: Data(String(cString: pointer).utf8))
        guard reply.ok, let result = reply.result else {
            throw IOSApplicationRuntimeError.rejected("The fresh resend Host configuration could not be verified")
        }
        return result
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    func modelProfile(id: String) throws -> IOSCanonicalModelProfile? {
        try executeOptional(ModelCommandEnvelope(command: ModelProfileCommand(type: "get_profile", id: id)))
    }

    func listModelProfiles() throws -> [IOSCanonicalModelProfile] {
        try execute(ModelCommandEnvelope(command: TypeCommand(type: "list_profiles")))
    }

    @discardableResult
    func deleteModelProfile(id: String) throws -> Bool {
        try execute(ModelCommandEnvelope(command: DeleteCommand(
            type: "delete_profile",
            id: id,
            deletedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )))
    }

    func pendingSyncChanges(limit: Int = 100) throws -> [IOSCanonicalSyncChange] {
        try execute(LimitCommand(type: "pending_sync_changes", limit: max(1, limit)))
    }

    @discardableResult
    func resolveSyncChange(
        operationID: String,
        status: String,
        remoteCursor: Int64?,
        resolvedAtMs: Int64
    ) throws -> Bool {
        try execute(ResolveSyncChangeCommand(
            type: "resolve_sync_change",
            operationID: operationID,
            status: status,
            remoteCursor: remoteCursor,
            resolvedAtMs: resolvedAtMs
        ))
    }

    func syncCursor(replicaID: String) throws -> IOSCanonicalSyncCursor? {
        try executeOptional(SyncCursorCommand(type: "get_sync_cursor", replicaID: replicaID))
    }

    func saveSyncCursor(replicaID: String, cursor: Int64, updatedAtMs: Int64) throws -> IOSCanonicalSyncCursor {
        try execute(SaveSyncCursorCommand(
            type: "save_sync_cursor",
            replicaID: replicaID,
            cursor: cursor,
            updatedAtMs: updatedAtMs
        ))
    }

    func applyRemoteSyncPage(
        replicaID: String,
        changes: [IOSCanonicalRemoteSyncChange],
        nextCursor: Int64,
        appliedAtMs: Int64
    ) throws -> IOSCanonicalRemoteSyncPageResult {
        try execute(ApplyRemoteSyncPageCommand(
            type: "apply_remote_sync_page",
            input: .init(
                replicaID: replicaID,
                changes: changes,
                nextCursor: nextCursor,
                appliedAtMs: appliedAtMs
            )
        ))
    }

    @discardableResult
    func deleteDataSource(id: String, expectedRevision: Int64, deletedAtMs: Int64) throws -> Bool {
        return try execute(DataSourceCommandEnvelope(command: DeleteDataSourceCommand(
            type: "delete",
            id: id,
            expectedRevision: expectedRevision,
            deletedAtMs: deletedAtMs
        )))
    }

    func appendDataSourceUsage(_ input: IOSCanonicalDataSourceUsage) throws {
        let _: Bool = try execute(DataSourceCommandEnvelope(command: DataSourceInputCommand(type: "record_usage", input: input)))
    }

    func listDataSourceUsage(limit: Int = 200) throws -> [IOSCanonicalDataSourceUsage] {
        try execute(DataSourceCommandEnvelope(command: ListUsageCommand(type: "list_usage", limit: limit)))
    }

    func recallProfileMemories(query: String, limit: Int) throws -> [IOSCanonicalMemoryRecord] {
        struct Binding: Encodable {
            let profile_id = "local"
        }
        struct Query: Encodable {
            let binding = Binding()
            let namespace: IOSCanonicalMemoryNamespace
            let scope = "global"
            let query: String
            let kind: String? = nil
            let now_iso: String
            let limit: Int
        }
        let namespaces = [IOSCanonicalMemoryNamespace(kind: "personal", name: nil)] +
            [ChatMemoryNamespace.longTerm, .userPreferences].map { IOSCanonicalMemoryNamespace(kind: "custom", name: $0.rawValue) }
        return try namespaces.flatMap { namespace in
            let input = Query(namespace: namespace, query: query, now_iso: ISO8601DateFormatter().string(from: Date()), limit: limit)
            let records: [IOSCanonicalMemoryRecord] = try execute(MemoryCommandEnvelope(command: MemoryInputCommand(type: "recall", input: input)))
            return records
        }
    }

    func upsertMemory(_ input: IOSCanonicalMemoryCandidate) throws -> IOSCanonicalMemoryRecord {
        try executeMutation(MemoryCommandEnvelope(command: MemoryCommitCommand(
            type: "commit", input: input, intent: "explicit_user"
        )))
    }

    func listMemories(limit: Int = 1_000) throws -> [IOSCanonicalMemoryRecord] {
        try execute(MemoryCommandEnvelope(command: LimitCommand(type: "list", limit: limit)))
    }

    func replaceMemories(expected: [IOSCanonicalMemoryRecord], candidates: [IOSCanonicalMemoryCandidate],
                         deletedAtMs: Int64) throws -> [IOSCanonicalMemoryRecord] {
        struct Replacement: Encodable {
            let type = "replace"
            let intent = "explicit_user"
            let expected: [IOSCanonicalMemoryRecord]
            let candidates: [IOSCanonicalMemoryCandidate]
            let deletedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case type, intent, expected, candidates
                case deletedAtMs = "deleted_at_ms"
            }
        }
        return try executeMutation(MemoryCommandEnvelope(command: Replacement(
            expected: expected, candidates: candidates, deletedAtMs: deletedAtMs
        )))
    }

    func upsertAutomationDefinition(
        _ input: IOSCanonicalAutomationDefinition
    ) throws -> IOSCanonicalAutomationDefinition {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "definition", command: InputCommand(type: "upsert", input: input)
        )))
    }

    func listAutomationDefinitions() throws -> [IOSCanonicalAutomationDefinition] {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "definition", command: TypeCommand(type: "list")
        )))
    }

    @discardableResult
    func deleteAutomationDefinition(id: String, deletedAtMs: Int64) throws -> Bool {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "definition",
            command: DeleteCommand(type: "delete", id: id, deletedAtMs: deletedAtMs)
        )))
    }

    func enqueueAutomationRun(_ input: IOSCanonicalAutomationRun) throws -> IOSCanonicalAutomationRun {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: InputCommand(type: "enqueue", input: input)
        )))
    }

    func claimAutomationRun(id: String, workerID: String, leaseToken: String, nowMs: Int64, leaseUntilMs: Int64) throws -> Bool {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: AutomationLeaseCommand(
            type: "claim", id: id, workerID: workerID,
            leaseToken: leaseToken, nowMs: nowMs, leaseUntilMs: leaseUntilMs
        ))))
    }

    func heartbeatAutomationRun(id: String, leaseToken: String, nowMs: Int64, leaseUntilMs: Int64) throws -> Bool {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: AutomationHeartbeatCommand(
            type: "heartbeat", id: id, leaseToken: leaseToken,
            nowMs: nowMs, leaseUntilMs: leaseUntilMs
        ))))
    }

    func retryAutomationRun(id: String, leaseToken: String, nextRetryAtMs: Int64, errorJSON: String, updatedAtMs: Int64) throws -> Bool {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: AutomationRetryCommand(
            type: "retry", id: id, leaseToken: leaseToken,
            nextRetryAtMs: nextRetryAtMs, errorJSON: errorJSON, updatedAtMs: updatedAtMs
        ))))
    }

    func observeAutomationExecution(occurrenceID: String) throws -> IOSAutomationExecutionObservation {
        try execute(IOSAutomationExecutionCommand(type: "observe_automation_execution", occurrenceID: occurrenceID))
    }
    func reconcileAutomationExecution(occurrenceID: String, reconciledAtMs: Int64) throws -> IOSAutomationExecutionReceipt? {
        try executeOptional(IOSAutomationExecutionCommand(type: "reconcile_automation_execution", occurrenceID: occurrenceID, reconciledAtMs: reconciledAtMs))
    }
    func observeAutomationWorker(lease: IOSAutomationExecutionLease, observation: IOSAutomationWorkerObservation, observedAtMs: Int64) throws -> Bool {
        try execute(IOSAutomationExecutionCommand(type: "observe_automation_worker", lease: lease, observation: observation, observedAtMs: observedAtMs))
    }
    func ackAutomationNotification(occurrenceID: String, notificationID: String, deliveredAtMs: Int64) throws -> Bool {
        try execute(IOSAutomationExecutionCommand(type: "ack_automation_notification", occurrenceID: occurrenceID, notificationID: notificationID, deliveredAtMs: deliveredAtMs))
    }
    func listPendingAutomationExecutions(afterOccurrenceID: String?, limit: Int) throws -> [String] {
        try execute(IOSAutomationExecutionCommand(type: "list_pending_automation_executions", afterOccurrenceID: afterOccurrenceID, limit: limit))
    }

    func loadAutomationRun(id: String) throws -> IOSCanonicalAutomationRun? {
        try executeOptional(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: AutomationIDCommand(type: "load", id: id)
        )))
    }

    func listAutomationRuns(automationID: String? = nil, limit: Int = 500) throws -> [IOSCanonicalAutomationRun] {
        try execute(AutomationCommandEnvelope(command: AutomationObjectCommand(
            type: "run", command: ListAutomationRunsCommand(
                type: "list", automationID: automationID, limit: limit
            )
        )))
    }

    @discardableResult
    func deleteMemory(id: String, deletedAtMs: Int64) throws -> Bool {
        try executeMutation(MemoryCommandEnvelope(command: DeleteCommand(type: "delete", id: id, deletedAtMs: deletedAtMs)))
    }

    func upsertSkill(_ input: IOSCanonicalSkill) throws -> IOSCanonicalSkill {
        try executeMutation(SkillCommandEnvelope(command: SkillInputCommand(type: "upsert", input: input)))
    }

    func skillEditor(id: String) throws -> IOSCanonicalSkillEditor? {
        try execute(SkillCommandEnvelope(command: SkillIdentityCommand(type: "get_editor", id: id)))
    }

    func createUserSkill(_ input: IOSCanonicalSkill) throws -> IOSCanonicalSkillEditor {
        try executeMutation(SkillCommandEnvelope(command: SkillInputCommand(type: "create_user", input: input)))
    }

    func configureUserSkill(_ input: IOSCanonicalSkill, expected: IOSCanonicalSkillEditor) throws -> IOSCanonicalSkillEditor {
        guard input.id == expected.record.id else {
            throw IOSApplicationRuntimeError.rejected("Skill editor identity cannot change")
        }
        return try executeMutation(SkillCommandEnvelope(command: SkillRevisionInputCommand(
            type: "configure_user", input: input, expectedRevision: expected.revision
        )))
    }

    func setSkillEnabled(_ enabled: Bool, expected: IOSCanonicalSkillEditor, updatedAtMs: Int64) throws -> IOSCanonicalSkillEditor {
        try executeMutation(SkillCommandEnvelope(command: SkillToggleCommand(
            type: "set_enabled_checked", id: expected.record.id, enabled: enabled,
            expectedRevision: expected.revision, updatedAtMs: updatedAtMs
        )))
    }

    @discardableResult
    func deleteUserSkill(expected: IOSCanonicalSkillEditor, deletedAtMs: Int64) throws -> Bool {
        try executeMutation(SkillCommandEnvelope(command: RevisionDeleteCommand(
            type: "delete_user_checked", id: expected.record.id,
            expectedRevision: expected.revision, deletedAtMs: deletedAtMs
        )))
    }

    func listSkills() throws -> [IOSCanonicalSkill] {
        try execute(SkillCommandEnvelope(command: TypeCommand(type: "list")))
    }

    @discardableResult
    func deleteUserSkill(id: String, deletedAtMs: Int64) throws -> Bool {
        try executeMutation(SkillCommandEnvelope(command: DeleteCommand(type: "delete_user", id: id, deletedAtMs: deletedAtMs)))
    }

    func upsertMCPServer(_ input: IOSCanonicalMCPServer) throws -> IOSCanonicalMCPServer {
        try executeMutation(MCPCommandEnvelope(command: MCPInputCommand(type: "upsert", input: input)))
    }

    func createMCPServer(_ input: IOSCanonicalMCPServer) throws -> IOSCanonicalMCPServer {
        try executeMutation(MCPCommandEnvelope(command: MCPWriteCommand(
            type: "create", id: input.id, displayName: input.displayName, enabled: input.enabled,
            configurationJSON: input.configurationJSON, expectedRevision: nil,
            createdAtMs: input.createdAtMs, updatedAtMs: nil
        )))
    }

    func configureMCPServer(_ input: IOSCanonicalMCPServer, expected: IOSCanonicalMCPServer) throws -> IOSCanonicalMCPServer {
        guard input.id == expected.id else {
            throw IOSApplicationRuntimeError.rejected("MCP editor identity cannot change")
        }
        return try executeMutation(MCPCommandEnvelope(command: MCPWriteCommand(
            type: "configure", id: input.id, displayName: input.displayName, enabled: input.enabled,
            configurationJSON: input.configurationJSON, expectedRevision: expected.revision,
            createdAtMs: nil, updatedAtMs: input.updatedAtMs
        )))
    }

    @discardableResult
    func deleteMCPServer(expected: IOSCanonicalMCPServer, deletedAtMs: Int64) throws -> Bool {
        try executeMutation(MCPCommandEnvelope(command: RevisionDeleteCommand(
            type: "delete_checked", id: expected.id, expectedRevision: expected.revision, deletedAtMs: deletedAtMs
        )))
    }

    func listMCPServers() throws -> [IOSCanonicalMCPServer] {
        try execute(MCPCommandEnvelope(command: TypeCommand(type: "list")))
    }

    func createConversation(
        id: String,
        title: String,
        settingsJSON: String?,
        createdAtMs: Int64,
        updatedAtMs: Int64
    ) throws {
        let receipt: IOSCanonicalCreateConversationResult = try execute(InputCommand(
            type: "create_conversation",
            input: IOSCanonicalCreateConversationInput(
                id: id,
                title: title,
                settingsJSON: settingsJSON,
                createdAtMs: createdAtMs,
                updatedAtMs: updatedAtMs
            )
        ))
        // This adapter creates a new owner; an existing ID is not successful
        // initialization of the caller's configuration. Core leaves it intact.
        guard receipt.created else {
            throw IOSApplicationRuntimeError.rejected("Conversation already exists; its configuration was not changed.")
        }
    }

    func importConversationTranscript(
        importID: String,
        source: String,
        conversationID: String,
        expectedMessageCount: Int,
        messages: [IOSCanonicalConversationMessage],
        importedAtMs: Int64
    ) throws {
        let _: IOSCanonicalMessageWriteResult = try execute(InputCommand(
            type: "import_conversation_transcript",
            input: IOSCanonicalImportConversationTranscriptInput(
                importID: importID,
                source: source,
                conversationID: conversationID,
                expectedMessageCount: expectedMessageCount,
                messages: messages,
                importedAtMs: importedAtMs
            )
        ))
    }

    func loadConversation(id: String) throws -> IOSCanonicalConversation? {
        try executeOptional(ConversationIDCommand(type: "load_conversation", conversationID: id))
    }

    func conversationSourceOwner(conversationID: String) throws -> AppleConversationSourceOwner {
        try execute(ConversationIDCommand(type: "load_conversation_source_owner", conversationID: conversationID))
    }

    func loadConversationProjectionV1(id: String) throws -> IOSConversationProjectionV1 {
        try execute(ConversationIDCommand(type: "load_conversation_projection_v1", conversationID: id))
    }

    func loadConversationProjectionWindowV1(
        id: String,
        beforeOrdinal: Int64?,
        limit: Int,
        contentByteLimit: Int
    ) throws -> IOSConversationProjectionWindowV1 {
        try execute(ConversationProjectionWindowCommand(
            type: "load_conversation_projection_window_v1",
            conversationID: id,
            beforeOrdinal: beforeOrdinal,
            limit: limit,
            contentByteLimit: contentByteLimit
        ))
    }

    func listConversations() throws -> [IOSCanonicalConversationSummary] {
        try listConversationDirectory(commandType: "list_conversations")
    }

    func listRootConversations() throws -> [IOSCanonicalConversationSummary] {
        try listConversationDirectory(commandType: "list_root_conversations")
    }

    private func listConversationDirectory(commandType: String) throws -> [IOSCanonicalConversationSummary] {
        struct Query: Encodable {
            let project_root: String? = nil
            let non_empty = false
            let limit = 1024
            let offset: Int
        }
        struct Command: Encodable {
            let type: String
            let query: Query
        }
        var result: [IOSCanonicalConversationSummary] = []
        while true {
            let page: [IOSCanonicalConversationSummary] = try execute(Command(type: commandType, query: Query(offset: result.count)))
            result.append(contentsOf: page)
            if page.count < 1024 { return result }
        }
    }

    func renameConversation(id: String, title: String, updatedAtMs: Int64) throws {
        let _: Bool = try execute(RenameConversationCommand(
            type: "rename_conversation",
            conversationID: id,
            title: title,
            updatedAtMs: updatedAtMs
        ))
    }

    func prepareContextMaintenance(source: IOSContextCompactionSource, input: AgentCoreContextPrepareInput) throws -> IOSContextMaintenancePlan {
        struct Prepare: Encodable { let type = "prepare_maintenance"; let source: IOSContextCompactionSource; let input: AgentCoreContextPrepareInput }
        struct Command: Encodable { let type = "context"; let command: Prepare }
        return try execute(Command(command: Prepare(source: source, input: input)))
    }

    func commitContextMaintenance(plan: IOSContextMaintenancePlan, validatedSource: IOSContextCompactionSource,
                                  currentInput: AgentCoreContextPrepareInput, summary: String) throws {
        struct Input: Encodable {
            let source: IOSContextCompactionSource; let validated_source: IOSContextCompactionSource
            let original_input: AgentCoreContextPrepareInput; let current_input: AgentCoreContextPrepareInput
            let summary: String; let committed_at_ms: Int64
        }
        struct Commit: Encodable { let type = "commit_maintenance"; let input: Input }
        struct Command: Encodable { let type = "context"; let command: Commit }
        let _: AgentCoreContextPrepareResponse = try execute(Command(command: Commit(input: Input(
            source: plan.source, validated_source: validatedSource, original_input: plan.input,
            current_input: currentInput, summary: summary, committed_at_ms: Int64(Date().timeIntervalSince1970 * 1000)))))
    }

    func prepareContextCompaction(conversationID: String) throws -> IOSContextCompactionSource {
        struct Command: Encodable {
            let type = "context"
            let command: Prepare
        }
        struct Prepare: Encodable {
            let type = "prepare_compaction"
            let conversation_id: String
        }
        return try execute(Command(command: Prepare(conversation_id: conversationID)))
    }

    func replaceContextSummary(source: IOSContextCompactionSource, summary: String) throws {
        struct Replace: Encodable {
            let type = "replace_summary"
            let source: IOSContextCompactionSource
            let summary: String
            let committed_at_ms: Int64
        }
        struct Command: Encodable { let type = "context"; let command: Replace }
        let _: Bool = try execute(Command(command: Replace(source: source, summary: summary,
            committed_at_ms: Int64(Date().timeIntervalSince1970 * 1000))))
    }


    func selectConversationExecution(_ command: SelectConversationExecutionCommand<AgentCoreJSONValue>) throws -> IOSConfigurationCommitResult {
        try execute(command)
    }

    func setConversationPermission(_ command: SetConversationPermissionCommand) throws -> IOSConfigurationCommitResult {
        try execute(command)
    }

    func patchConversationConfiguration(id: String, expectedRevision: Int64, patchJSON: String,
                                       updatedAtMs: Int64) throws -> IOSConfigurationCommitResult {
        struct Input: Encodable {
            let conversation_id: String
            let expected_revision: Int64
            let patch_json: String
            let updated_at_ms: Int64
        }
        return try execute(InputCommand(type: "patch_conversation_configuration", input: Input(
            conversation_id: id, expected_revision: expectedRevision, patch_json: patchJSON, updated_at_ms: updatedAtMs)))
    }

    @discardableResult
    func deleteConversation(id: String) throws -> Bool {
        try execute(ConversationIDCommand(type: "delete_conversation", conversationID: id))
    }

    @discardableResult
    func deleteMCPServer(id: String, deletedAtMs: Int64) throws -> Bool {
        try executeMutation(MCPCommandEnvelope(command: DeleteCommand(type: "delete", id: id, deletedAtMs: deletedAtMs)))
    }

    /// Internal typed adapters build the command; UI and feature services do
    /// not receive access to Store primitives through this method.
    func readHistoricalPart(
        owner: IOSHistoricalPartRequest.Owner, revision: Int64
    ) throws -> IOSConversationProjectionV1.HistoricalProjection? {
        try executeOptional(IOSHistoricalPartQuery(owner: owner, expectedConversationRevision: revision))
    }

    private struct IOSHistoricalPartQuery: Encodable {
        let type = "read_historical_part"
        let owner: IOSHistoricalPartRequest.Owner
        let expectedConversationRevision: Int64
        let maximumOutputBytes = 32768
        enum CodingKeys: String, CodingKey {
            case type, owner
            case expectedConversationRevision = "expected_conversation_revision"
            case maximumOutputBytes = "maximum_output_bytes"
        }
    }

    func readHistoricalOriginal(_ query: IOSHistoricalOriginalQuery) throws -> IOSHistoricalOriginalPage {
        let page: IOSHistoricalOriginalPage = try execute(query)
        guard page.matches(query) else {
            throw IOSApplicationRuntimeError.rejected(String(localized: "Historical content changed. Reopen its details."))
        }
        return page
    }

    func listUISurfaces(conversationID: String) throws -> [AppleUISnapshot<AgentCoreJSONValue>] {
        try execute(AppleUIListCommand(conversation_id: conversationID))
    }

    func previewUISurface(_ input: AppleUIPreviewInput<AgentCoreJSONValue>) throws -> AppleUIPreviewResult<AgentCoreJSONValue> {
        try execute(AppleUIPreviewCommand(input: input))
    }

    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?) throws -> AppleUISurfaceUpdate<AgentCoreJSONValue> {
        try execute(AppleUIReadChangesCommand(identity: identity, base_revision: baseRevision))
    }

    func readUISurfaceRevision(identity: AppleUISurfaceIdentity, revision: UInt64) throws -> AppleUISnapshot<AgentCoreJSONValue>? {
        try execute(AppleUIReadRevisionCommand(identity: identity, revision: revision))
    }

    func submitUIInteraction(_ input: AppleUISubmission<AgentCoreJSONValue>) throws -> AppleUIInteractionReceipt<AgentCoreJSONValue> {
        try executeMutation(AppleUISubmitCommand(input: input),
            requestID: "ui-interaction:\(input.interaction.identity.owner.conversation_id):\(input.submission_id)")
    }

    /// Starts only an already admitted Task Run; Core validates the complete
    /// task attempt, frozen invocation, worker identity and assistant owner.
    func startChildPump(_ input: AgentCoreJSONValue) throws -> AppleChildPump {
        #if canImport(NolaCore)
        let data = try JSONEncoder().encode(input)
        let transport: AppleChildPump = try {
            lock.lock()
            defer { lock.unlock() }
            guard let handle else { throw IOSApplicationRuntimeError.applicationUnavailable }
            var error: UnsafeMutablePointer<CChar>?
            let pump = data.withUnsafeBytes { bytes in
                nola_core_child_pump_start(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &error)
            }
            defer { if let error { nola_core_free_c_string(error) } }
            guard let pump else {
                throw IOSApplicationRuntimeError.rejected(error.map { String(cString: $0) } ?? "Child admission is unconfirmed")
            }
            return AppleChildPump(admittedHandle: pump)
        }()
        childPumpObserver?(transport)
        return transport
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    func mobileMcpHost(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        #if canImport(NolaCore)
        let isCancellation: Bool
        if case .object(let fields) = command { isCancellation = fields["type"] == .string("cancel") }
        else { isCancellation = false }
        lock.lock()
        let retained = isCancellation ? nil : handle.flatMap { nola_core_mobile_mcp_retain($0) }
        lock.unlock()
        guard isCancellation || retained != nil else { throw IOSApplicationRuntimeError.applicationUnavailable }
        defer { if let retained { nola_core_application_free(retained) } }
        let data = try JSONEncoder().encode(command)
        let response = data.withUnsafeBytes { bytes in
            nola_core_mobile_mcp_host(retained,bytes.bindMemory(to: UInt8.self).baseAddress,bytes.count)
        }
        guard let response else { throw IOSApplicationRuntimeError.invalidResponse }
        defer { nola_core_free_c_string(response) }
        let value = try JSONDecoder().decode(AgentCoreJSONValue.self,from: Data(String(cString: response).utf8))
        guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        guard fields["ok"] == .bool(true), let result = fields["result"] else {
            var message = "MCP Host rejected request"
            if case .object(let error)? = fields["error"], case .string(let detail)? = error["message"] { message = detail }
            throw IOSApplicationRuntimeError.rejected(message)
        }
        return result
        #else
        throw IOSApplicationRuntimeError.applicationUnavailable
        #endif
    }

    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        if case .object(let fields) = command,
           fields["type"] == .string("admit_conversation_user_message"),
           case .object(let input)? = fields["input"],
           case .string(let conversationID)? = input["conversation_id"],
           case .string(let messageID)? = input["message_id"] {
            return try executeMutation(command, requestID: "expert-admission:\(conversationID):\(messageID)")
        }
        if case .object(let fields) = command,
           fields["type"] == .string("begin_queued_user_turn"),
           case .object(let input)? = fields["input"],
           case .string(let commandID)? = input["command_id"],
           case .signedInteger(let attempt)? = input["delivery_attempt"] {
            return try executeMutation(command, requestID: "queued-admission:\(commandID):\(attempt)")
        }
        // Replaying a queued submission must reconcile the original receipt before
        // entering the reducer again (including after the Run has completed).
        if case .object(let fields) = command,
           case .string("reduce_and_commit_active_submission")? = fields["type"],
           case .object(let input)? = fields["input"],
           case .string(let transitionID)? = input["transition_id"] {
            return try executeMutation(command, requestID: "active-submission:\(transitionID)")
        }
        if case .object(let fields) = command, fields["type"] == .string("step_goal"),
           let operation = fields["control"]?.objectFields?["operation_id"]?.stringScalar,
           let owner = fields["conversation_id"]?.stringScalar {
            return try executeMutation(command, requestID: "embedded-goal:\(owner):\(operation)")
        }
        return try execute(command)
    }

    struct AuxiliaryInvocationReceipt: Decodable {
        struct Operation: Decodable {
            let id: String
            let revision: Int64
            let state: String
            let attemptsStarted: Int
            enum CodingKeys: String, CodingKey {
                case id, revision, state
                case attemptsStarted = "attempts_started"
            }
        }
        struct Attempt: Decodable {
            let id: String
            let revision: Int64
            let state: String
            let usage: AgentCoreJSONValue
        }
        let operation: Operation
        let attempt: Attempt?
        let dispatchPermitted: Bool
        enum CodingKeys: String, CodingKey {
            case operation, attempt
            case dispatchPermitted = "dispatch_permitted"
        }
    }

    func executeAuxiliaryInvocation(_ command: [String: AgentCoreJSONValue]) throws -> AuxiliaryInvocationReceipt {
        try execute(AgentCoreJSONValue.object([
            "type": .string("auxiliary_invocation"), "command": .object(command)
        ]))
    }

    private func execute<Command: Encodable, Result: Decodable>(_ command: Command) throws -> Result {
        try executeMutation(command)
    }

    private func executeOptional<Command: Encodable, Result: Decodable>(_ command: Command) throws -> Result? {
        let receipt = try submitReceipt(command, requestID: UUID().uuidString.lowercased())
        guard let result = receipt["result"], !(result is NSNull) else { return nil }
        return try JSONDecoder().decode(Result.self, from: JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]))
    }

    nonisolated private struct InputCommand<Input: Encodable>: Encodable { let type: String; let input: Input }
    nonisolated private struct TypeCommand: Encodable { let type: String }
    nonisolated private struct SkillCommandEnvelope<Command: Encodable>: Encodable {
        let type = "skill"
        let command: Command
    }
    nonisolated private struct SkillInputCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }
    nonisolated private struct SkillIdentityCommand: Encodable { let type: String; let id: String }
    nonisolated private struct SkillRevisionInputCommand: Encodable {
        let type: String
        let input: IOSCanonicalSkill
        let expectedRevision: Int64
        enum CodingKeys: String, CodingKey { case type, input; case expectedRevision = "expected_revision" }
    }
    nonisolated private struct SkillToggleCommand: Encodable {
        let type: String
        let id: String
        let enabled: Bool
        let expectedRevision: Int64
        let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id, enabled
            case expectedRevision = "expected_revision"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct RevisionDeleteCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let deletedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id
            case expectedRevision = "expected_revision"
            case deletedAtMs = "deleted_at_ms"
        }
    }
    nonisolated private struct MCPCommandEnvelope<Command: Encodable>: Encodable {
        let type = "mcp"
        let command: Command
    }
    nonisolated private struct MCPInputCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }
    nonisolated private struct MCPWriteCommand: Encodable {
        let type: String
        let id: String
        let displayName: String
        let enabled: Bool
        let configurationJSON: String
        let expectedRevision: Int64?
        let createdAtMs: Int64?
        let updatedAtMs: Int64?
        enum CodingKeys: String, CodingKey {
            case type, id, enabled
            case displayName = "display_name"
            case configurationJSON = "configuration_json"
            case expectedRevision = "expected_revision"
            case createdAtMs = "created_at_ms"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct DataSourceCommandEnvelope<Command: Encodable>: Encodable {
        let type = "data_source"
        let command: Command
    }
    nonisolated private struct DataSourceInputCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }
    nonisolated private struct DataSourceIDCommand: Encodable {
        let type: String
        let id: String
    }
    nonisolated private struct UpdateDataSourceCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let input: IOSCanonicalDataSourceConfigurationUpdate

        enum CodingKeys: String, CodingKey {
            case type, id, input
            case expectedRevision = "expected_revision"
        }
    }
    nonisolated private struct RequestDataSourceValidationCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let requestedAtMs: Int64

        enum CodingKeys: String, CodingKey {
            case type, id
            case expectedRevision = "expected_revision"
            case requestedAtMs = "requested_at_ms"
        }
    }
    nonisolated private struct DataSourceValidationSuccess: Encodable {
        let status: String
        let contract: [String: AgentCoreDataSourceJSON]
    }
    nonisolated private struct RecordDataSourceValidationCommand: Encodable {
        let type: String
        let id: String
        let validationToken: String
        let result: DataSourceValidationSuccess
        let recordedAtMs: Int64

        enum CodingKeys: String, CodingKey {
            case type, id, result
            case validationToken = "validation_token"
            case recordedAtMs = "recorded_at_ms"
        }
    }
    nonisolated private struct SetDataSourceEnabledCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let enabled: Bool
        let updatedAtMs: Int64

        enum CodingKeys: String, CodingKey {
            case type, id, enabled
            case expectedRevision = "expected_revision"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct DeleteDataSourceCommand: Encodable {
        let type: String
        let id: String
        let expectedRevision: Int64
        let deletedAtMs: Int64

        enum CodingKeys: String, CodingKey {
            case type, id
            case expectedRevision = "expected_revision"
            case deletedAtMs = "deleted_at_ms"
        }
    }
    nonisolated private struct MemoryCommandEnvelope<Command: Encodable>: Encodable {
        let type = "memory"
        let command: Command
    }
    nonisolated private struct MemoryInputCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }
    nonisolated private struct MemoryCommitCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
        let intent: String
    }
    nonisolated private struct ModelCommandEnvelope<Command: Encodable>: Encodable {
        let type = "model"
        let command: Command
    }
    nonisolated private struct ModelInputCommand<Input: Encodable>: Encodable {
        let type: String
        let input: Input
    }
    nonisolated private struct ProductSettingWriteCommand: Encodable {
        let type: String
        let key: String
        let valueJSON: String
        let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, key
            case valueJSON = "value_json"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct ProductSettingKeyCommand: Encodable { let type: String; let key: String }
    nonisolated private struct ModelProfileCommand: Encodable { let type: String; let id: String }
    nonisolated private struct ResolveSyncChangeCommand: Encodable {
        let type: String
        let operationID: String
        let status: String
        let remoteCursor: Int64?
        let resolvedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, status
            case operationID = "operation_id"
            case remoteCursor = "remote_cursor"
            case resolvedAtMs = "resolved_at_ms"
        }
    }
    nonisolated private struct SyncCursorCommand: Encodable {
        let type: String
        let replicaID: String
        enum CodingKeys: String, CodingKey { case type; case replicaID = "replica_id" }
    }
    nonisolated private struct SaveSyncCursorCommand: Encodable {
        let type: String
        let replicaID: String
        let cursor: Int64
        let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, cursor
            case replicaID = "replica_id"
            case updatedAtMs = "updated_at_ms"
        }
    }

    nonisolated private struct ApplyRemoteSyncPageCommand: Encodable {
        struct Input: Encodable {
            let replicaID: String
            let changes: [IOSCanonicalRemoteSyncChange]
            let nextCursor: Int64
            let appliedAtMs: Int64
            enum CodingKeys: String, CodingKey {
                case changes
                case replicaID = "replica_id"
                case nextCursor = "next_cursor"
                case appliedAtMs = "applied_at_ms"
            }
        }
        let type: String
        let input: Input
    }
    nonisolated private struct DeleteCommand: Encodable {
        let type: String
        let id: String
        let deletedAtMs: Int64
        enum CodingKeys: String, CodingKey { case type, id; case deletedAtMs = "deleted_at_ms" }
    }
    nonisolated private struct ListUsageCommand: Encodable { let type: String; let limit: Int }
    nonisolated private struct LimitCommand: Encodable { let type: String; let limit: Int }
    nonisolated private struct AutomationIDCommand: Encodable { let type: String; let id: String }
    nonisolated private struct AutomationCommandEnvelope<Command: Encodable>: Encodable {
        let type = "automation"
        let command: Command
    }
    nonisolated private struct AutomationObjectCommand<Command: Encodable>: Encodable {
        let type: String
        let command: Command
    }
    nonisolated private struct ListAutomationRunsCommand: Encodable {
        let type: String
        let automationID: String?
        let limit: Int
        enum CodingKeys: String, CodingKey { case type, limit; case automationID = "automation_id" }
    }
    nonisolated private struct AutomationLeaseCommand: Encodable {
        let type: String; let id: String; let workerID: String; let leaseToken: String
        let nowMs: Int64; let leaseUntilMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id
            case workerID = "worker_id"; case leaseToken = "lease_token"
            case nowMs = "now_ms"; case leaseUntilMs = "lease_until_ms"
        }
    }
    nonisolated private struct AutomationHeartbeatCommand: Encodable {
        let type: String; let id: String; let leaseToken: String; let nowMs: Int64; let leaseUntilMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id; case leaseToken = "lease_token"
            case nowMs = "now_ms"; case leaseUntilMs = "lease_until_ms"
        }
    }
    nonisolated private struct AutomationRetryCommand: Encodable {
        let type: String; let id: String; let leaseToken: String; let nextRetryAtMs: Int64
        let errorJSON: String; let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id; case leaseToken = "lease_token"
            case nextRetryAtMs = "next_retry_at_ms"; case errorJSON = "error_json"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct ConversationIDCommand: Encodable {
        let type: String
        let conversationID: String
        enum CodingKeys: String, CodingKey { case type; case conversationID = "conversation_id" }
    }
    nonisolated private struct ConversationProjectionWindowCommand: Encodable {
        let type: String
        let conversationID: String
        let beforeOrdinal: Int64?
        let limit: Int
        let contentByteLimit: Int
        enum CodingKeys: String, CodingKey {
            case type, limit
            case conversationID = "conversation_id"
            case beforeOrdinal = "before_ordinal"
            case contentByteLimit = "content_byte_limit"
        }
    }
    nonisolated private struct RenameConversationCommand: Encodable {
        let type: String
        let conversationID: String
        let title: String
        let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, title
            case conversationID = "conversation_id"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct Envelope<Result: Decodable>: Decodable {
        let ok: Bool
        let result: Result?
        let error: Failure?
    }
    nonisolated private struct Failure: Decodable { let message: String }
}

nonisolated extension UserUploadedContextItem {
    var canonicalSkill: IOSCanonicalSkill {
        let digest = SHA256.hash(data: Data(content.utf8)).map { String(format: "%02x", $0) }.joined()
        let tools = (requiredTools ?? []).map {
            AgentCoreSkillToolReference.builtin(toolName: $0)
        }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return IOSCanonicalSkill(
            id: id.uuidString.lowercased(), source: "user",
            version: "\(version ?? "user")-\(digest.prefix(16))",
            displayName: name, summary: description, category: "user",
            instructionsRef: "nola://ios/skills/\(id.uuidString.lowercased())",
            instructionsDigest: digest, instructionsText: content,
            requiredTools: tools, defaultMode: nil, enabled: enabled,
            createdAtMs: now, updatedAtMs: now, deletedAtMs: nil
        )
    }
}

nonisolated extension IOSCanonicalSkill {
    var uploadedContextItem: UserUploadedContextItem? {
        guard source == "user", let uuid = UUID(uuidString: id) else { return nil }
        let tools = requiredTools.compactMap { reference -> String? in
            guard case .builtin(let toolName) = reference else { return nil }
            return toolName
        }
        return UserUploadedContextItem(
            id: uuid, name: displayName, content: instructionsText, description: summary,
            enabled: enabled, version: version, requiredTools: tools.isEmpty ? nil : tools
        )
    }
}

nonisolated extension UserMCPServer {
    var canonicalMCPServer: IOSCanonicalMCPServer? {
        guard let data = try? JSONEncoder().encode(self),
              let configuration = String(data: data, encoding: .utf8) else { return nil }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return IOSCanonicalMCPServer(
            id: id.uuidString.lowercased(), displayName: name, enabled: enabled,
            configurationJSON: configuration, revision: 0,
            createdAtMs: now, updatedAtMs: now, deletedAtMs: nil
        )
    }
}

nonisolated extension IOSCanonicalMCPServer {
    var userMCPServer: UserMCPServer? {
        guard let canonicalID = UUID(uuidString: id),
              let data = configurationJSON.data(using: .utf8),
              var server = try? JSONDecoder().decode(UserMCPServer.self, from: data) else { return nil }
        server.id = canonicalID
        server.name = displayName
        server.enabled = enabled
        return server
    }
}

nonisolated extension ChatMemoryRecord {
    var canonicalRecord: IOSCanonicalMemoryCandidate? {
        let formatter = ISO8601DateFormatter()
        let timestamp = Int64((observedAt ?? Date()).timeIntervalSince1970 * 1_000)
        return IOSCanonicalMemoryCandidate(
            record: IOSCanonicalSemanticMemoryRecord(
                id: id,
                owner: owner,
                namespace: IOSCanonicalMemoryNamespace(kind: "custom", name: namespace.rawValue),
                scope: "global",
                kind: kind == .semantic ? "reference" : "other",
                content: text,
                summary: nil,
                tags: [],
                confidence: confidence ?? 0.6,
                importance: confidence ?? 0.6,
                sensitivity: sensitivity,
                retention: IOSCanonicalMemoryRetention(
                    mode: retention == .durable ? "permanent" : "session",
                    expiresAt: nil
                ),
                supersedes: supersedes,
                provenance: IOSCanonicalMemoryProvenance(
                    kind: "explicit_user",
                    source: provenance.source,
                    sessionID: provenance.conversationID?.uuidString,
                    turnID: provenance.messageID?.uuidString,
                    itemIDs: [provenance.sourceID, provenance.messageID?.uuidString].compactMap { $0 },
                    observedAt: observedAt.map(formatter.string(from:))
                )
            ),
            quality: nil,
            proposedAtMs: timestamp
        )
    }
}

nonisolated extension IOSCanonicalMemoryRecord {
    var chatMemoryRecord: ChatMemoryRecord? {
        guard record.owner == .localProfile, record.scope == "global",
              record.namespace.kind == "custom",
              let namespaceName = record.namespace.name,
              let namespace = ChatMemoryNamespace(rawValue: namespaceName) else { return nil }
        let kind: ChatMemoryRecordKind = record.kind == "reference" ? .semantic : .procedural
        let retention: ChatMemoryRetention = record.retention.mode == "permanent" ? .durable : .session
        let formatter = ISO8601DateFormatter()
        return ChatMemoryRecord(
            id: record.id,
            owner: record.owner,
            namespace: namespace,
            kind: kind,
            text: record.content,
            confidence: record.confidence,
            provenance: ChatMemoryProvenance(
                source: record.provenance.source,
                sourceID: record.provenance.itemIDs.first,
                conversationID: record.provenance.sessionID.flatMap(UUID.init(uuidString:)),
                messageID: record.provenance.turnID.flatMap(UUID.init(uuidString:))
            ),
            observedAt: record.provenance.observedAt.flatMap(formatter.date(from:)),
            retention: retention,
            supersedes: record.supersedes,
            sensitivity: record.sensitivity
        )
    }
}

nonisolated private struct IOSURLDataSourceMetadata: Codable {
    let owner: String
    let source: URLDataSource
}

extension URLDataSource {
    var canonicalSubmission: IOSCanonicalDataSourceSubmission? {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        guard let metadataData = try? encoder.encode(
            IOSURLDataSourceMetadata(owner: "ios.url_fetcher", source: self)
        ),
        let metadataJSON = String(data: metadataData, encoding: .utf8) else {
            return nil
        }
        let canonicalKind: IOSCanonicalDataSourceKind
        let locationKind: IOSCanonicalDataSourceLocationKind
        switch kind {
        case .file, .localReminder:
            canonicalKind = .localFile
            locationKind = .localPath
        case .folder:
            canonicalKind = .directory
            locationKind = .localPath
        case .system:
            canonicalKind = .platform
            locationKind = .platformResource
        case .website, .webSearch, .api, .feed, .calendarFeed:
            canonicalKind = .http
            locationKind = .httpURL
        case .custom:
            canonicalKind = .custom
            locationKind = .opaqueReference
        }
        let timestamp = Int64(updatedAt.timeIntervalSince1970 * 1_000)
        return IOSCanonicalDataSourceSubmission(
            draft: IOSCanonicalDataSourceDraft(
                id: id.uuidString,
                name: name,
                kind: canonicalKind,
                configuration: IOSCanonicalDataSourceConfiguration(
                    connectorID: validationContract?.connectorID ?? "ios.url-fetcher",
                    locationKind: locationKind,
                    location: url,
                    parameters: ["metadataJSON": .string(metadataJSON)],
                    secretReferences: []
                ),
                accessPolicy: accessPolicy.rawValue,
                authorization: IOSCanonicalDataSourceAuthorization(
                    state: locationKind == .httpURL ? .notRequired
                        : (validationContract == nil ? .notDetermined : .authorized),
                    capability: locationKind == .httpURL ? nil : id.uuidString,
                    observedAtMs: timestamp
                ),
                freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: UInt64(max(1, validationContract?.maximumAgeMinutes ?? 60)),
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "local-first",
                createdAtMs: timestamp,
                updatedAtMs: timestamp
            ),
            validationContract: validationContract?.coreContractJSON,
            enabled: enabled
        )
    }
}

private nonisolated struct IOSCanonicalValidationProjection: Decodable {
    struct Recipe: Decodable {
        let method: String
        let operationId: String
        let configurationKeys: [String]
        let outputSchema: AgentCoreDataSourceJSON
    }

    let sourceId: UUID
    let connectorId: String
    let connectorVersion: Int
    let readerDescription: String
    let readRecipe: Recipe
    let schemaFields: [String]
    let redactedSample: String
    let recordCount: Int
    let maximumAgeMinutes: Int
    let validationState: URLDataSourceValidationState
    let validatedAt: Date
    let observedAt: Date?
    let validationDigest: String

    static func decode(_ contract: [String: AgentCoreDataSourceJSON]) throws -> URLDataSourceValidationContract {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let value = try container.decode(String.self)
            let formatter = ISO8601DateFormatter()
            formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            if let date = formatter.date(from: value) { return date }
            formatter.formatOptions = [.withInternetDateTime]
            if let date = formatter.date(from: value) { return date }
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Invalid canonical date")
        }
        let wire = try decoder.decode(Self.self, from: JSONEncoder().encode(contract))
        guard let method = URLDataSourceReadMethod(rawValue: wire.readRecipe.method == "platformApi" ? "platformAPI" : wire.readRecipe.method) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let schema = try encoder.encode(wire.readRecipe.outputSchema)
        return URLDataSourceValidationContract(
            sourceID: wire.sourceId,
            connectorID: wire.connectorId,
            connectorVersion: wire.connectorVersion,
            readerDescription: wire.readerDescription,
            readRecipe: URLDataSourceReadRecipe(
                method: method,
                operationID: wire.readRecipe.operationId,
                configurationKeys: wire.readRecipe.configurationKeys,
                outputSchemaJSON: String(decoding: schema, as: UTF8.self)
            ),
            schemaFields: wire.schemaFields,
            redactedSample: wire.redactedSample,
            recordCount: wire.recordCount,
            maximumAgeMinutes: wire.maximumAgeMinutes,
            validationState: wire.validationState,
            validatedAt: wire.validatedAt,
            observedAt: wire.observedAt,
            validationDigest: wire.validationDigest
        )
    }
}

extension IOSCanonicalDataSource {
    var iosURLSource: URLDataSource? {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard case .string(let metadataJSON)? = configuration.parameters["metadataJSON"],
              let data = metadataJSON.data(using: .utf8),
              let metadata = try? decoder.decode(IOSURLDataSourceMetadata.self, from: data),
              metadata.owner == "ios.url_fetcher" else {
            return nil
        }
        var source = metadata.source
        source.enabled = defaultEnabled
        source.canonicalRevision = revision
        source.isRefreshing = readState.status == .pending
        source.lastFetchDate = lastSuccessfulReadAtMs.map { Date(timeIntervalSince1970: Double($0) / 1_000) }
        switch readState.status {
        case .failed, .denied, .unavailable:
            source.lastError = readState.diagnostic
        default:
            source.lastError = nil
        }
        // Embedded metadata is presentation/configuration data, not validation authority.
        // Its ISO8601 dates may also have lower precision than the canonical contract.
        source.validationContract = nil
        if validation.status == .verified || validation.status == .verifiedEmpty {
            guard let contract = validation.contract,
                  let decoded = try? IOSCanonicalValidationProjection.decode(contract),
                  decoded.sourceID.uuidString == id else { return nil }
            source.validationContract = decoded
        }
        return source
    }
}

extension URLDataSourceUsageRecord {
    var canonicalRecord: IOSCanonicalDataSourceUsage {
        IOSCanonicalDataSourceUsage(
            id: id.uuidString,
            sourceID: sourceID.uuidString,
            sourceTitle: sourceTitle,
            consumer: consumer,
            purpose: purpose,
            conversationID: conversationID?.uuidString,
            disclosedToCloud: disclosedToCloud,
            metadata: [:],
            usedAtMs: Int64(usedAt.timeIntervalSince1970 * 1_000)
        )
    }
}

extension IOSCanonicalDataSourceUsage {
    var iosUsageRecord: URLDataSourceUsageRecord? {
        guard let id = UUID(uuidString: id),
              let sourceID = UUID(uuidString: sourceID) else {
            return nil
        }
        return URLDataSourceUsageRecord(
            id: id,
            sourceID: sourceID,
            sourceTitle: sourceTitle,
            consumer: consumer,
            purpose: purpose,
            conversationID: conversationID.flatMap(UUID.init(uuidString:)),
            usedAt: Date(timeIntervalSince1970: Double(usedAtMs) / 1_000),
            disclosedToCloud: disclosedToCloud
        )
    }
}

/// Host observations are submitted through Application, never written into source metadata.
nonisolated enum IOSCanonicalDataSourceReadResult: Encodable {
    case succeeded(contentReference: IOSCanonicalDataSourceContentReference, observedAtMs: Int64, schemaVersion: UInt32, recordCount: UInt64)
    case empty(observedAtMs: Int64, schemaVersion: UInt32)
    case denied(authorization: IOSCanonicalDataSourceAuthorization, diagnostic: String)
    case unavailable(diagnostic: String)
    case failed(code: String, diagnostic: String)

    private enum CodingKeys: String, CodingKey {
        case status, authorization, diagnostic, code
        case contentReference = "content_reference"
        case observedAtMs = "observed_at_ms"
        case schemaVersion = "schema_version"
        case recordCount = "record_count"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .succeeded(reference, observedAt, schema, count):
            try container.encode("succeeded", forKey: .status)
            try container.encode(reference, forKey: .contentReference)
            try container.encode(observedAt, forKey: .observedAtMs)
            try container.encode(schema, forKey: .schemaVersion)
            try container.encode(count, forKey: .recordCount)
        case let .empty(observedAt, schema):
            try container.encode("empty", forKey: .status)
            try container.encode(observedAt, forKey: .observedAtMs)
            try container.encode(schema, forKey: .schemaVersion)
        case let .denied(authorization, diagnostic):
            try container.encode("denied", forKey: .status)
            try container.encode(authorization, forKey: .authorization)
            try container.encode(diagnostic, forKey: .diagnostic)
        case let .unavailable(diagnostic):
            try container.encode("unavailable", forKey: .status)
            try container.encode(diagnostic, forKey: .diagnostic)
        case let .failed(code, diagnostic):
            try container.encode("failed", forKey: .status)
            try container.encode(code, forKey: .code)
            try container.encode(diagnostic, forKey: .diagnostic)
        }
    }
}

extension IOSApplicationRuntime {
    func requestDataSourceRefresh(id: String, expectedRevision: Int64, reason: String, requestedAtMs: Int64) throws -> IOSCanonicalDataSource {
        try execute(DataSourceCommandEnvelope(command: RequestSourceRefresh(
            id: id, expectedRevision: expectedRevision, reason: reason, requestedAtMs: requestedAtMs
        )))
    }

    func recordDataSourceRead(id: String, readToken: String, result: IOSCanonicalDataSourceReadResult, recordedAtMs: Int64) throws -> IOSCanonicalDataSource {
        try execute(DataSourceCommandEnvelope(command: RecordSourceRead(
            id: id, readToken: readToken, result: result, recordedAtMs: recordedAtMs
        )))
    }

    nonisolated private struct RequestSourceRefresh: Encodable {
        let type = "request_refresh"
        let id: String
        let expectedRevision: Int64
        let reason: String
        let requestedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id, reason
            case expectedRevision = "expected_revision"
            case requestedAtMs = "requested_at_ms"
        }
    }

    nonisolated private struct RecordSourceRead: Encodable {
        let type = "record_read"
        let id: String
        let readToken: String
        let result: IOSCanonicalDataSourceReadResult
        let recordedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, id, result
            case readToken = "read_token"
            case recordedAtMs = "recorded_at_ms"
        }
    }
}

nonisolated enum IOSCanonicalDataSourceOverride: String, Codable {
    case inherit, enabled, disabled
}

nonisolated enum IOSCanonicalDataSourceSelectionReason: String, Codable {
    case ready
    case readyEmpty = "ready_empty"
    case disabledByDefault = "disabled_by_default"
    case disabledForConversation = "disabled_for_conversation"
    case lifecycleNotReady = "lifecycle_not_ready"
    case authorizationUnavailable = "authorization_unavailable"
    case validationUnavailable = "validation_unavailable"
    case refreshRequired = "refresh_required"
    case readPending = "read_pending"
    case readUnavailable = "read_unavailable"
}

nonisolated struct IOSCanonicalDataSourceContextSelection: Decodable, Equatable {
    let sourceID: String
    let sourceRevision: Int64
    let enabled: Bool
    let usable: Bool
    let shouldRefresh: Bool
    let reason: IOSCanonicalDataSourceSelectionReason
    let contentReference: IOSCanonicalDataSourceContentReference?
    let observedAtMs: Int64?
    let schemaVersion: UInt32?
    enum CodingKeys: String, CodingKey {
        case enabled, usable, reason
        case sourceID = "source_id"
        case sourceRevision = "source_revision"
        case shouldRefresh = "should_refresh"
        case contentReference = "content_reference"
        case observedAtMs = "observed_at_ms"
        case schemaVersion = "schema_version"
    }
}

extension IOSApplicationRuntime {
    nonisolated func resolveDataSources(conversationID: String, nowMs: Int64) throws -> [IOSCanonicalDataSourceContextSelection] {
        try execute(DataSourceCommandEnvelope(command: ResolveSources(conversationID: conversationID, nowMs: nowMs)))
    }

    @discardableResult
    nonisolated func setDataSourceOverride(conversationID: String, sourceID: String, value: IOSCanonicalDataSourceOverride, updatedAtMs: Int64) throws -> Bool {
        try execute(DataSourceCommandEnvelope(command: SetSourceOverride(conversationID: conversationID, sourceID: sourceID, value: value, updatedAtMs: updatedAtMs)))
    }

    nonisolated func dataSourceOverride(conversationID: String, sourceID: String) throws -> IOSCanonicalDataSourceOverride {
        try execute(DataSourceCommandEnvelope(command: GetSourceOverride(conversationID: conversationID, sourceID: sourceID)))
    }

    nonisolated private struct ResolveSources: Encodable {
        let type = "resolve_for_conversation"
        let conversationID: String
        let nowMs: Int64
        enum CodingKeys: String, CodingKey {
            case type
            case conversationID = "conversation_id"
            case nowMs = "now_ms"
        }
    }
    nonisolated private struct SetSourceOverride: Encodable {
        let type = "set_conversation_override"
        let conversationID: String
        let sourceID: String
        let value: IOSCanonicalDataSourceOverride
        let updatedAtMs: Int64
        enum CodingKeys: String, CodingKey {
            case type, value
            case conversationID = "conversation_id"
            case sourceID = "source_id"
            case updatedAtMs = "updated_at_ms"
        }
    }
    nonisolated private struct GetSourceOverride: Encodable {
        let type = "get_conversation_override"
        let conversationID: String
        let sourceID: String
        enum CodingKeys: String, CodingKey {
            case type
            case conversationID = "conversation_id"
            case sourceID = "source_id"
        }
    }
}

nonisolated struct IOSContextMaintenancePlan: Codable, Sendable {
    let source: IOSContextCompactionSource
    let input: AgentCoreContextPrepareInput
    let output: AgentCoreContextPrepareResponse
    let instructions: String?
}
