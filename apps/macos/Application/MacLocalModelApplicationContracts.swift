// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the typed macOS Application boundary for local model profile configuration.

import Foundation

nonisolated struct MacCanonicalModelProfile: Codable, Equatable, Sendable {
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

    private enum CodingKeys: String, CodingKey {
        case id, provider, runtime, selected, enabled, revision
        case displayName = "display_name"
        case modelName = "model_name"
        case sourceJSON = "source_json"
        case parametersJSON = "parameters_json"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
        case deletedAtMs = "deleted_at_ms"
    }

    init(
        id: String,
        displayName: String,
        provider: String,
        modelName: String,
        runtime: String,
        sourceJSON: String,
        parametersJSON: String,
        selected: Bool,
        enabled: Bool,
        revision: Int64,
        createdAtMs: Int64,
        updatedAtMs: Int64,
        deletedAtMs: Int64?
    ) {
        self.id = id
        self.displayName = displayName
        self.provider = provider
        self.modelName = modelName
        self.runtime = runtime
        self.sourceJSON = sourceJSON
        self.parametersJSON = parametersJSON
        self.selected = selected
        self.enabled = enabled
        self.revision = revision
        self.createdAtMs = createdAtMs
        self.updatedAtMs = updatedAtMs
        self.deletedAtMs = deletedAtMs
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        modelName = try container.decodeIfPresent(String.self, forKey: .modelName) ?? id
        displayName = try container.decodeIfPresent(String.self, forKey: .displayName) ?? modelName
        provider = try container.decodeIfPresent(String.self, forKey: .provider) ?? "openai-compatible"
        runtime = try container.decodeIfPresent(String.self, forKey: .runtime) ?? "remote"
        sourceJSON = try container.decodeIfPresent(String.self, forKey: .sourceJSON) ?? "{}"
        parametersJSON = try container.decodeIfPresent(String.self, forKey: .parametersJSON) ?? "{}"
        selected = try container.decodeIfPresent(Bool.self, forKey: .selected) ?? false
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        revision = try container.decodeIfPresent(Int64.self, forKey: .revision) ?? 1
        createdAtMs = try container.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        updatedAtMs = try container.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? createdAtMs
        deletedAtMs = try container.decodeIfPresent(Int64.self, forKey: .deletedAtMs)
    }
}

/// Application use cases needed to select a local model and edit its parameters.
nonisolated protocol MacLocalModelApplicationClient: Sendable {
    func upsertModelProfile(_ input: MacCanonicalModelProfile) throws
    func configureModelParameters(
        _ input: MacCanonicalModelProfile,
        expectedRevision: UInt64
    ) throws -> UInt64
    func modelProfile(_ id: String) throws -> MacCanonicalModelProfile?
    func listModelProfiles() throws -> [MacCanonicalModelProfile]
}
