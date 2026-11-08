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
