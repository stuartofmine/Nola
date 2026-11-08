// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the typed macOS Application boundary for the canonical Skill catalog.

import Foundation

nonisolated enum MacCanonicalSkillSourceKind: String, Codable, Equatable, Sendable {
    case builtIn = "built_in"
    case user
}

nonisolated struct MacCanonicalSkillInput: Codable, Sendable {
    let id: String
    let source: MacCanonicalSkillSourceKind
    let version: String
    let displayName: String
    let summary: String
    let category: String
    let instructionsRef: String
    let instructionsDigest: String
    let instructionsText: String
    let requiredTools: [MacCoreCapabilityToolReference]
    let defaultMode: MacCoreSkillExecutionMode?
    let enabled: Bool
    let createdAtMs: Int64
    let updatedAtMs: Int64
}

nonisolated struct MacCanonicalSkill: Codable, Equatable, Sendable {
    let id: String
    let source: MacCanonicalSkillSourceKind
    let version: String
    let displayName: String
    let summary: String
    let category: String
    let instructionsRef: String
    let instructionsDigest: String
    let instructionsText: String
    let requiredTools: [MacCoreCapabilityToolReference]
    let defaultMode: MacCoreSkillExecutionMode?
    let enabled: Bool
    let createdAtMs: Int64
    let updatedAtMs: Int64
    let deletedAtMs: Int64?

    private enum CodingKeys: String, CodingKey {
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

nonisolated struct MacCanonicalSkillEditor: Codable, Equatable, Sendable {
    let record: MacCanonicalSkill
    let revision: Int64
}

/// Typed Application use cases required to project and edit the built-in Skill catalog.
/// Implementations preserve the canonical revision checks and wire schema.
nonisolated protocol MacSkillCatalogApplicationClient: Sendable {
    func synchronizeBuiltInSkills(
        _ inputs: [MacCanonicalSkillInput]
    ) throws -> [MacCanonicalSkillEditor]
    func skillEditor(_ id: String) throws -> MacCanonicalSkillEditor?
    func setSkillEnabled(
        _ expected: MacCanonicalSkillEditor,
        enabled: Bool,
        updatedAtMs: Int64
    ) throws -> MacCanonicalSkillEditor
    func listSkills() throws -> [MacCanonicalSkill]
    func restoreBuiltInSkills(
        _ expected: [MacCanonicalSkillEditor],
        visibilityRevision: Int64?,
        updatedAtMs: Int64
    ) throws -> [MacCanonicalSkillEditor]
    func removeBuiltInSkill(
        _ expected: MacCanonicalSkillEditor,
        visibilityRevision: Int64?,
        updatedAtMs: Int64
    ) throws -> MacCanonicalSkillEditor
    func productSetting(key: String) throws -> MacCanonicalProductSetting?
}
