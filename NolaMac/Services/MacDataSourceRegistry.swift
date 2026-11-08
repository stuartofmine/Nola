// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Persists macOS data-source policies, validated reader contracts, conversation overrides, and provenance.

import Foundation
import Observation

nonisolated enum MacDataSourceAccessPolicy: String, CaseIterable, Codable, Identifiable, Sendable {
    case defaultEnabled
    case conversationOptIn
    case askWhenRelevant
    case blocked

    var id: String { rawValue }

    var title: String {
        switch self {
        case .defaultEnabled: return String(localized: "Every Conversation")
        case .conversationOptIn: return String(localized: "Per Conversation")
        case .askWhenRelevant: return String(localized: "Ask When Relevant")
        case .blocked: return String(localized: "Blocked")
        }
    }
}

nonisolated enum MacDataSourceValidationState: String, Codable, Sendable {
    case verified
    case verifiedEmpty
    case failed

    var title: String {
        switch self {
        case .verified: return String(localized: "Verified")
        case .verifiedEmpty: return String(localized: "Verified · No matching records")
        case .failed: return String(localized: "Validation failed")
        }
    }
}

nonisolated enum MacDataSourceReadMethod: String, Codable, Hashable, Sendable {
    case httpGet
    case localFileRead
    case directoryList
    case platformAPI = "platformApi"
    case databaseQuery
    case mcpTool
    case hostTool
    case custom

    /// Preserve the historical persisted value while emitting Core's serde
    /// camelCase spelling at the FFI contract boundary.
    nonisolated var coreRawValue: String {
        rawValue
    }
}

nonisolated struct MacDataSourceReadRecipe: Codable, Hashable, Sendable {
    var method: MacDataSourceReadMethod
    var operationID: String
    var configurationKeys: [String]
    var outputSchemaJSON: String

    var isUsable: Bool {
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

    private enum CodingKeys: String, CodingKey {
        case method
        case operationID = "operationId"
        case configurationKeys
        case outputSchema
    }

    init(
        method: MacDataSourceReadMethod,
        operationID: String,
        configurationKeys: [String],
        outputSchemaJSON: String
    ) {
        self.method = method
        self.operationID = operationID
        self.configurationKeys = configurationKeys
        self.outputSchemaJSON = outputSchemaJSON
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        method = try container.decode(MacDataSourceReadMethod.self, forKey: .method)
        operationID = try container.decode(String.self, forKey: .operationID)
        configurationKeys = try container.decode([String].self, forKey: .configurationKeys)
        let schema = try container.decode(MacCanonicalJSONValue.self, forKey: .outputSchema)
        let data = try JSONEncoder().encode(schema)
        guard let value = String(data: data, encoding: .utf8) else {
            throw DecodingError.dataCorruptedError(
                forKey: .outputSchema,
                in: container,
                debugDescription: "Data-source output schema is not UTF-8 JSON"
            )
        }
        outputSchemaJSON = value
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(method, forKey: .method)
        try container.encode(operationID, forKey: .operationID)
        try container.encode(configurationKeys, forKey: .configurationKeys)
        guard let data = outputSchemaJSON.data(using: .utf8) else {
            throw EncodingError.invalidValue(
                outputSchemaJSON,
                .init(codingPath: encoder.codingPath, debugDescription: "Invalid UTF-8 output schema")
            )
        }
        try container.encode(
            JSONDecoder().decode(MacCanonicalJSONValue.self, from: data),
            forKey: .outputSchema
        )
    }
}

nonisolated struct MacDataSourceContract: Identifiable, Codable, Hashable, Sendable {
    var sourceID: String
    var connectorID: String
    var connectorVersion: Int
    var readerDescription: String
    var readRecipe: MacDataSourceReadRecipe?
    var schemaFields: [String]
    var redactedSample: String
    var recordCount: Int
    var maximumAgeMinutes: Int
    var validationState: MacDataSourceValidationState
    var validatedAt: Date
    var observedAt: Date?
    var errorMessage: String?
    var validationDigest: String?

    var id: String { sourceID }

    private enum CodingKeys: String, CodingKey {
        case sourceID = "sourceId"
        case connectorID = "connectorId"
        case connectorVersion, readerDescription, readRecipe, schemaFields, redactedSample
        case recordCount, maximumAgeMinutes, validationState, validatedAt, observedAt
        case errorMessage, validationDigest
    }
    var isUsable: Bool {
        MacCoreDataSourceResolver.resolve(
            MacCoreDataSourceResolutionInput(
                sourceID: sourceID,
                policy: MacDataSourceAccessPolicy.defaultEnabled.rawValue,
                conversationOverride: nil,
                contract: coreContractJSON,
                now: Self.coreDateString(.now)
            )
        )?.usable == true
    }

    var expectedValidationDigest: String? {
        guard let sealed = MacCoreDataSourceResolver.sealContract(coreContractJSON),
              case .string(let digest)? = sealed["validationDigest"] else { return nil }
        return digest
    }

    mutating func sealValidation() {
        validationDigest = expectedValidationDigest
    }

    nonisolated var coreContractJSON: [String: MacCoreDataSourceJSON] {
        let outputSchema = readRecipe
            .flatMap { $0.outputSchemaJSON.data(using: .utf8) }
            .flatMap { try? JSONDecoder().decode(MacCoreDataSourceJSON.self, from: $0) }
            ?? .object(["type": .string("object")])
        let recipe: MacCoreDataSourceJSON = readRecipe.map {
            .object([
                "method": .string($0.method.coreRawValue),
                "operationId": .string($0.operationID),
                "configurationKeys": .array($0.configurationKeys.map(MacCoreDataSourceJSON.string)),
                "outputSchema": outputSchema
            ])
        } ?? .object([:])
        return [
            "sourceId": .string(sourceID),
            "connectorId": .string(connectorID),
            "connectorVersion": .number(Double(connectorVersion)),
            "readerDescription": .string(readerDescription),
            "readRecipe": recipe,
            "schemaFields": .array(schemaFields.map(MacCoreDataSourceJSON.string)),
            "redactedSample": .string(redactedSample),
            "recordCount": .number(Double(recordCount)),
            "maximumAgeMinutes": .number(Double(maximumAgeMinutes)),
            "validationState": .string(validationState.rawValue),
            "validatedAt": .string(Self.coreDateString(validatedAt)),
            "observedAt": observedAt.map { .string(Self.coreDateString($0)) } ?? .null,
            "validationDigest": .string(validationDigest ?? "")
        ]
    }

    nonisolated fileprivate static func coreDateString(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }
}

struct MacDataSourceUsageRecord: Identifiable, Codable, Hashable, Sendable {
    var id: UUID
    var sourceID: String
    var sourceTitle: String
    var consumer: String
    var purpose: String
    var conversationID: String?
    var usedAt: Date
    var disclosedToCloud: Bool
}

struct MacDataSourceAccessRecord: Codable, Hashable, Sendable {
    var ownerID: String
    var revision: UInt64
}

struct MacCanonicalDataSourceMetadata: Codable, Sendable {
    var version: Int = 1
    var accessRecord: MacDataSourceAccessRecord?
    var webSource: MacWebSource?
}

@MainActor
@Observable
final class MacDataSourceRegistry {
    static let shared = MacDataSourceRegistry()

    private(set) var contracts: [String: MacDataSourceContract] = [:]
    private(set) var policies: [String: MacDataSourceAccessPolicy] = [:]
    private(set) var conversationOverrides: [String: [String: Bool]] = [:]
    private(set) var usageRecords: [MacDataSourceUsageRecord] = []
    private(set) var accessRecords: [String: MacDataSourceAccessRecord] = [:]
    private(set) var persistenceError: String?
    private(set) var overrideDisplayRevision: UInt64 = 0

    private let dataSourceClient: (any MacDataSourceApplicationClient)?

    init(
        storeURL: URL? = nil,
        applicationPort: (any MacDataSourceApplicationClient)? = nil
    ) {
        if let storeURL {
            let databaseURL = storeURL.deletingLastPathComponent().appendingPathComponent(
                MacAppEnvironment.canonicalStoreFileName
            )
            self.dataSourceClient = applicationPort
                ?? MacDataSourceApplicationComposition.client(databaseURL: databaseURL)
        } else {
            self.dataSourceClient = applicationPort
                ?? MacDataSourceApplicationComposition.sharedClient
        }
        load()
    }

    static func platformSourceID(_ rawKind: String) -> String {
        "platform:\(rawKind)"
    }

    static func webSourceID(_ id: UUID) -> String {
        "web:\(id.uuidString.lowercased())"
    }

    func policy(
        for sourceID: String,
        fallback: MacDataSourceAccessPolicy = .conversationOptIn
    ) -> MacDataSourceAccessPolicy {
        policies[sourceID] ?? fallback
    }

    func setPolicy(_ policy: MacDataSourceAccessPolicy, for sourceID: String) {
        policies[sourceID] = policy
        persistSourceSafely(sourceID)
    }

    func conversationOverride(sourceID: String, conversationID: String) -> Bool? {
        if let persisted = try? dataSourceClient?.dataSourceOverride(
            conversationID: conversationID,
            sourceID: sourceID
        ) {
            switch persisted {
            case .inherit: return nil
            case .enabled: return true
            case .disabled: return false
            }
        }
        return conversationOverrides[conversationID]?[sourceID]
    }

    func setConversationOverride(
        _ enabled: Bool?,
        sourceID: String,
        conversationID: String
    ) {
        defer { overrideDisplayRevision &+= 1 }
        var values = conversationOverrides[conversationID] ?? [:]
        values[sourceID] = enabled
        if values.isEmpty {
            conversationOverrides[conversationID] = nil
        } else {
            conversationOverrides[conversationID] = values
        }
        do {
            _ = try dataSourceClient?.setDataSourceOverride(
                conversationID: conversationID,
                sourceID: sourceID,
                value: enabled == true ? .enabled : (enabled == false ? .disabled : .inherit),
                updatedAtMs: Self.timestamp(.now)
            )
            persistenceError = nil
        } catch {
            persistenceError = error.localizedDescription
        }
    }

    func isEnabled(
        sourceID: String,
        conversationID: String?,
        fallback: MacDataSourceAccessPolicy = .conversationOptIn
    ) -> Bool {
        resolution(
            sourceID: sourceID,
            conversationID: conversationID,
            fallback: fallback
        )?.enabled == true
    }

    func resolution(
        sourceID: String,
        conversationID: String?,
        fallback: MacDataSourceAccessPolicy = .conversationOptIn,
        now: Date = .now
    ) -> MacCoreDataSourceResolution? {
        displayResolution(sourceID: sourceID,
            override: conversationID.flatMap { conversationOverride(sourceID: sourceID, conversationID: $0) },
            fallback: fallback, now: now)
    }

    /// Display-only snapshot. Runtime callers continue using resolution(),
    /// which reads the current canonical override at the execution boundary.
    func loadOverridesForDisplay(conversationID: String?, sourceIDs: [String]) async throws -> [String: Bool] {
        guard let conversationID else { return [:] }
        let client = dataSourceClient
        let fallback = conversationOverrides[conversationID] ?? [:]
        return try await MacConversationReadTask.run {
            guard let client else { return fallback }
            var result: [String: Bool] = [:]
            for id in Set(sourceIDs) {
                try Task.checkCancellation()
                switch try client.dataSourceOverride(conversationID: conversationID, sourceID: id) {
                case .inherit: break
                case .enabled: result[id] = true
                case .disabled: result[id] = false
                }
            }
            return result
        }
    }

    func displayResolution(sourceID: String, override: Bool?,
                           fallback: MacDataSourceAccessPolicy = .conversationOptIn,
                           now: Date = .now) -> MacCoreDataSourceResolution? {
        MacCoreDataSourceResolver.resolve(
            MacCoreDataSourceResolutionInput(
                sourceID: sourceID,
                policy: policy(for: sourceID, fallback: fallback).rawValue,
                conversationOverride: override,
                contract: contracts[sourceID]?.coreContractJSON,
                now: MacDataSourceContract.coreDateString(now)
            )
        )
    }

    func contract(for sourceID: String) -> MacDataSourceContract? {
        contracts[sourceID]
    }

    @discardableResult
    func ensureOwnedResource(_ sourceID: String) -> MacDataSourceAccessRecord {
        if let existing = accessRecords[sourceID] { return existing }
        let record = MacDataSourceAccessRecord(
            ownerID: MacCoreResourceAccess.localPrincipal(),
            revision: 1
        )
        accessRecords[sourceID] = record
        persistSourceSafely(sourceID)
        return record
    }

    func canAccess(_ sourceID: String, permission: MacCoreResourcePermission) -> Bool {
        let record = accessRecords[sourceID] ?? ensureOwnedResource(sourceID)
        if let decision = MacCoreResourceAccess.evaluate(
            resourceID: sourceID,
            kind: .dataSource,
            ownerID: record.ownerID,
            revision: record.revision,
            permission: permission
        ) {
            return decision.allowed
        }
        return record.ownerID == MacCoreResourceAccess.localPrincipal()
    }

    @discardableResult
    func recordEdit(_ sourceID: String) -> Bool {
        guard canAccess(sourceID, permission: .editContent) else { return false }
        var record = accessRecords[sourceID] ?? ensureOwnedResource(sourceID)
        record.revision = record.revision == UInt64.max ? UInt64.max : max(1, record.revision) + 1
        accessRecords[sourceID] = record
        persistSourceSafely(sourceID)
        return true
    }

    func recordValidation(
        sourceID: String,
        connectorID: String,
        readerDescription: String,
        schemaFields: [String],
        redactedSample: String,
        recordCount: Int,
        maximumAgeMinutes: Int,
        observedAt: Date?,
        defaultPolicy: MacDataSourceAccessPolicy = .conversationOptIn
    ) {
        ensureOwnedResource(sourceID)
        let method: MacDataSourceReadMethod = connectorID == "mac.web.http"
            ? .httpGet
            : .platformAPI
        let configurationKeys = method == .httpGet
            ? ["url", "maximumBytes"]
            : ["sourceID"]
        var contract = MacDataSourceContract(
            sourceID: sourceID,
            connectorID: connectorID,
            connectorVersion: 1,
            readerDescription: readerDescription,
            readRecipe: MacDataSourceReadRecipe(
                method: method,
                operationID: "\(connectorID).read",
                configurationKeys: configurationKeys,
                outputSchemaJSON: Self.outputSchemaJSON(schemaFields: schemaFields)
            ),
            schemaFields: schemaFields,
            redactedSample: redactedSample,
            recordCount: max(0, recordCount),
            maximumAgeMinutes: max(1, maximumAgeMinutes),
            validationState: recordCount == 0 ? .verifiedEmpty : .verified,
            validatedAt: .now,
            observedAt: observedAt,
            errorMessage: nil,
            validationDigest: nil
        )
        contract.sealValidation()
        contracts[sourceID] = contract
        if policies[sourceID] == nil {
            policies[sourceID] = defaultPolicy
        }
        persistSourceSafely(sourceID)
    }

    func recordValidationFailure(
        sourceID: String,
        connectorID: String,
        readerDescription: String,
        error: String,
        defaultPolicy: MacDataSourceAccessPolicy = .conversationOptIn
    ) {
        ensureOwnedResource(sourceID)
        let previous = contracts[sourceID]
        contracts[sourceID] = MacDataSourceContract(
            sourceID: sourceID,
            connectorID: connectorID,
            connectorVersion: previous?.connectorVersion ?? 1,
            readerDescription: readerDescription,
            readRecipe: previous?.readRecipe,
            schemaFields: previous?.schemaFields ?? [],
            redactedSample: previous?.redactedSample ?? "",
            recordCount: previous?.recordCount ?? 0,
            maximumAgeMinutes: previous?.maximumAgeMinutes ?? 60,
            validationState: .failed,
            validatedAt: .now,
            observedAt: previous?.observedAt,
            errorMessage: error,
            validationDigest: previous?.validationDigest
        )
        if policies[sourceID] == nil {
            policies[sourceID] = defaultPolicy
        }
        persistSourceSafely(sourceID)
    }

    @discardableResult
    func removeSource(_ sourceID: String) -> Bool {
        guard canAccess(sourceID, permission: .delete) else { return false }
        defer { overrideDisplayRevision &+= 1 }
        contracts[sourceID] = nil
        policies[sourceID] = nil
        for conversationID in conversationOverrides.keys {
            conversationOverrides[conversationID]?[sourceID] = nil
            if conversationOverrides[conversationID]?.isEmpty == true {
                conversationOverrides[conversationID] = nil
            }
        }
        usageRecords.removeAll { $0.sourceID == sourceID }
        accessRecords[sourceID] = nil
        do {
            guard let existing = try dataSourceClient?.dataSource(id: sourceID) else {
                persistenceError = nil
                return true
            }
            _ = try dataSourceClient?.deleteDataSource(
                sourceID,
                expectedRevision: existing.revision,
                deletedAtMs: Self.timestamp(.now)
            )
            persistenceError = nil
            return true
        } catch {
            persistenceError = error.localizedDescription
            return false
        }
    }

    private static func outputSchemaJSON(schemaFields: [String]) -> String {
        let properties = Dictionary(uniqueKeysWithValues: schemaFields.sorted().map {
            ($0, ["type": "string"])
        })
        let schema: [String: Any] = [
            "type": "array",
            "items": [
                "type": "object",
                "properties": properties
            ]
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: schema, options: [.sortedKeys]),
              let value = String(data: data, encoding: .utf8) else {
            return "{\"type\":\"array\",\"items\":{\"type\":\"object\"}}"
        }
        return value
    }

    func recordUsage(
        sourceID: String,
        sourceTitle: String,
        consumer: String,
        purpose: String,
        conversationID: String?,
        disclosedToCloud: Bool
    ) {
        if let latest = usageRecords.first,
           latest.sourceID == sourceID,
           latest.consumer == consumer,
           latest.purpose == purpose,
           latest.conversationID == conversationID,
           Date().timeIntervalSince(latest.usedAt) < 30 {
            return
        }
        usageRecords.insert(
            MacDataSourceUsageRecord(
                id: UUID(),
                sourceID: sourceID,
                sourceTitle: sourceTitle,
                consumer: consumer,
                purpose: purpose,
                conversationID: conversationID,
                usedAt: .now,
                disclosedToCloud: disclosedToCloud
            ),
            at: 0
        )
        usageRecords = Array(usageRecords.prefix(500))
        guard let latest = usageRecords.first else { return }
        do {
            _ = try dataSourceClient?.appendDataSourceUsage(latest.canonicalInput)
            persistenceError = nil
        } catch {
            persistenceError = error.localizedDescription
        }
    }

    private func load() {
        loadCanonical()
    }

    private func persistSourceSafely(_ sourceID: String) {
        do {
            try persistSource(sourceID)
            persistenceError = nil
        } catch {
            persistenceError = error.localizedDescription
        }
    }

    func canonicalWebSources() -> [MacWebSource] {
        ((try? dataSourceClient?.listDataSources()) ?? []).compactMap { record in
            guard record.kind == .web else { return nil }
            return metadata(from: record)?.webSource
        }
    }

    func replaceWebSources(_ sources: [MacWebSource]) {
        do {
            let existing = try dataSourceClient?.listDataSources() ?? []
            let desiredIDs = Set(sources.map(\.dataSourceID))
            for record in existing where record.kind == .web && !desiredIDs.contains(record.id) {
                _ = try dataSourceClient?.deleteDataSource(
                    record.id,
                    expectedRevision: record.revision,
                    deletedAtMs: Self.timestamp(.now)
                )
            }
            for source in sources { try persistWebSource(source) }
            persistenceError = nil
        } catch {
            persistenceError = error.localizedDescription
        }
    }

    private func loadCanonical() {
        guard let dataSourceClient else { return }
        do {
            let records = try dataSourceClient.listDataSources()
            for record in records {
                if let contract = record.validation.contract {
                    contracts[record.id] = contract
                }
                policies[record.id] = MacDataSourceAccessPolicy(rawValue: record.accessPolicy)
                if let access = metadata(from: record)?.accessRecord { accessRecords[record.id] = access }
            }
            usageRecords = try dataSourceClient.listDataSourceUsage(limit: 500).compactMap(\.macRecord)
            persistenceError = nil
        } catch {
            persistenceError = error.localizedDescription
        }
    }

    private func persistSource(_ sourceID: String) throws {
        guard let dataSourceClient else { throw MacApplicationPortError.serviceUnavailable }
        let existing = try dataSourceClient.dataSource(id: sourceID)
        var envelope = existing.flatMap(metadata(from:)) ?? MacCanonicalDataSourceMetadata()
        envelope.accessRecord = accessRecords[sourceID]
        let kind = Self.canonicalSourceKind(
            sourceID: sourceID,
            existingKind: existing?.kind
        )
        try persistDefinition(
            id: sourceID,
            name: existing?.name ?? sourceID,
            kind: kind,
            location: existing?.configuration.location ?? sourceID,
            locationKind: Self.locationKind(for: kind),
            connectorID: contracts[sourceID]?.connectorID
                ?? existing?.configuration.connectorID
                ?? "mac.platform",
            policy: policies[sourceID] ?? .conversationOptIn,
            contract: contracts[sourceID],
            metadata: envelope,
            existing: existing
        )
    }

    private func persistWebSource(_ source: MacWebSource) throws {
        guard let dataSourceClient else { throw MacApplicationPortError.serviceUnavailable }
        let existing = try dataSourceClient.dataSource(id: source.dataSourceID)
        var envelope = existing.flatMap(metadata(from:)) ?? MacCanonicalDataSourceMetadata()
        envelope.webSource = source
        envelope.accessRecord = accessRecords[source.dataSourceID] ?? envelope.accessRecord
        try persistDefinition(
            id: source.dataSourceID,
            name: source.name,
            kind: .web,
            location: source.url,
            locationKind: .httpURL,
            connectorID: "mac.web.http",
            policy: policies[source.dataSourceID]
                ?? (source.isEnabled ? .defaultEnabled : .conversationOptIn),
            contract: contracts[source.dataSourceID] ?? existing?.validation.contract,
            metadata: envelope,
            existing: existing
        )
    }

    private func metadata(from record: MacCanonicalDataSource) -> MacCanonicalDataSourceMetadata? {
        guard case .string(let metadataJSON)? = record.configuration.parameters["metadataJSON"],
              let data = metadataJSON.data(using: .utf8) else { return nil }
        return try? Self.decoder.decode(MacCanonicalDataSourceMetadata.self, from: data)
    }

    private func persistDefinition(
        id: String,
        name: String,
        kind: MacCanonicalDataSourceKind,
        location: String,
        locationKind: MacCanonicalDataSourceLocationKind,
        connectorID: String,
        policy: MacDataSourceAccessPolicy,
        contract: MacDataSourceContract?,
        metadata: MacCanonicalDataSourceMetadata,
        existing: MacCanonicalDataSource?
    ) throws {
        guard let dataSourceClient else { throw MacApplicationPortError.serviceUnavailable }
        let now = Self.timestamp(.now)
        let configuration = MacCanonicalDataSourceConfiguration(
            connectorID: connectorID,
            locationKind: locationKind,
            location: location,
            parameters: ["metadataJSON": .string(try Self.encoder.encodeString(metadata))],
            secretReferences: []
        )
        let authorization = MacCanonicalDataSourceAuthorization(
            state: kind == .platform
                ? (contract == nil ? .notDetermined : .authorized)
                : .notRequired,
            capability: kind == .platform ? id : nil,
            observedAtMs: now
        )
        let freshness = MacCanonicalDataSourceFreshnessPolicy(
            maximumAgeMinutes: UInt64(max(1, contract?.maximumAgeMinutes ?? 60)),
            allowStaleFallback: false
        )
        let staged: MacCanonicalDataSource
        if let existing {
            staged = try dataSourceClient.updateDataSourceConfiguration(
                id: id,
                expectedRevision: existing.revision,
                input: MacCanonicalDataSourceConfigurationUpdate(
                    name: name,
                    kind: kind,
                    configuration: configuration,
                    accessPolicy: policy.rawValue,
                    authorization: authorization,
                    freshnessPolicy: freshness,
                    capabilities: ["read", "context"],
                    privacy: kind == .web ? "cloud_reference" : "local",
                    updatedAtMs: now
                )
            )
        } else {
            staged = try dataSourceClient.createDataSourceDraft(MacCanonicalDataSourceDraft(
                id: id,
                name: name,
                kind: kind,
                configuration: configuration,
                accessPolicy: policy.rawValue,
                authorization: authorization,
                freshnessPolicy: freshness,
                capabilities: ["read", "context"],
                privacy: kind == .web ? "cloud_reference" : "local",
                createdAtMs: now,
                updatedAtMs: now
            ))
        }
        guard let contract else { return }
        let pending = try dataSourceClient.requestDataSourceValidation(
            id: id,
            expectedRevision: staged.revision,
            requestedAtMs: now
        )
        guard let validationToken = pending.validation.validationToken else {
            throw MacApplicationPortError.invalidResponse
        }
        let validated: MacCanonicalDataSource
        if contract.validationState == .failed {
            validated = try dataSourceClient.recordDataSourceValidationFailure(
                id: id,
                validationToken: validationToken,
                code: "host_validation_failed",
                diagnostic: contract.errorMessage ?? String(localized: "Validation failed"),
                recordedAtMs: now
            )
        } else {
            validated = try dataSourceClient.recordDataSourceValidationSuccess(
                id: id,
                validationToken: validationToken,
                contract: contract,
                recordedAtMs: now
            )
        }
        if validated.lifecycle == .ready {
            _ = try dataSourceClient.setDataSourceDefaultEnabled(
                id: id,
                expectedRevision: validated.revision,
                enabled: policy != .blocked,
                updatedAtMs: now
            )
        }
    }

    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()

    private static func canonicalSourceKind(
        sourceID: String,
        existingKind: MacCanonicalDataSourceKind?
    ) -> MacCanonicalDataSourceKind {
        if sourceID.hasPrefix("platform:") {
            return .platform
        }
        if let existingKind {
            switch existingKind {
            case .http, .localFile, .directory, .web:
                return existingKind
            default:
                break
            }
        }
        return .localFile
    }

    private static func locationKind(
        for kind: MacCanonicalDataSourceKind
    ) -> MacCanonicalDataSourceLocationKind {
        switch kind {
        case .http, .web: return .httpURL
        case .localFile, .directory: return .localPath
        case .platform: return .platformResource
        case .database: return .databaseReference
        case .mcp: return .mcpCapability
        case .custom: return .opaqueReference
        }
    }

    private static func timestamp(_ date: Date) -> Int64 {
        Int64(date.timeIntervalSince1970 * 1_000)
    }
}

private extension JSONEncoder {
    func encodeString<Value: Encodable>(_ value: Value) throws -> String {
        guard let value = String(data: try encode(value), encoding: .utf8) else {
            throw MacApplicationPortError.invalidResponse
        }
        return value
    }
}

private extension MacDataSourceUsageRecord {
    var canonicalInput: MacCanonicalDataSourceUsageInput {
        MacCanonicalDataSourceUsageInput(
            id: id.uuidString,
            sourceID: sourceID,
            sourceTitle: sourceTitle,
            consumer: consumer,
            purpose: purpose,
            conversationID: conversationID,
            disclosedToCloud: disclosedToCloud,
            metadata: [:],
            usedAtMs: Int64(usedAt.timeIntervalSince1970 * 1_000)
        )
    }
}

private extension MacCanonicalDataSourceUsageInput {
    var macRecord: MacDataSourceUsageRecord? {
        guard let id = UUID(uuidString: id) else { return nil }
        return MacDataSourceUsageRecord(
            id: id,
            sourceID: sourceID,
            sourceTitle: sourceTitle,
            consumer: consumer,
            purpose: purpose,
            conversationID: conversationID,
            usedAt: Date(timeIntervalSince1970: Double(usedAtMs) / 1_000),
            disclosedToCloud: disclosedToCloud
        )
    }
}
