// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed macOS client for the Agent Host's authoritative memory control plane.

import Foundation

nonisolated struct MacAgentHostMemoryProvenance: Codable, Equatable, Sendable {
    let kind: String?
    let source: String?
    let observedAt: String?

    enum CodingKeys: String, CodingKey {
        case kind, source
        case observedAt = "observed_at"
    }
}

nonisolated struct MacAgentHostMemoryRecord: Codable, Equatable, Identifiable, Sendable {
    let id: String
    let scope: String
    let kind: String
    let content: String
    let summary: String?
    let tags: [String]
    let source: String?
    let confidence: Double
    let importance: Double
    let sensitivity: String?
    let expiresAt: String?
    let supersedes: [String]
    let provenance: MacAgentHostMemoryProvenance
    let createdAt: String
    let updatedAt: String

    enum CodingKeys: String, CodingKey {
        case id, scope, kind, content, summary, tags, source, confidence, importance, sensitivity
        case expiresAt = "expires_at"
        case supersedes, provenance
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

nonisolated struct MacAgentHostMemoryDraft: Codable, Equatable, Sendable {
    let scope: String
    let kind: String
    let content: String
    let summary: String?
    let tags: [String]
    let confidence: Double
    let importance: Double
    let sensitivity: String?
    let expiresAt: String?
    var source: String? = nil
    var provenanceKind: String? = nil

    enum CodingKeys: String, CodingKey {
        case scope, kind, content, summary, tags, confidence, importance, sensitivity
        case expiresAt = "expires_at"
        case source
        case provenanceKind = "provenance_kind"
    }
}

nonisolated private struct MacMemoryListEnvelope: Decodable {
    let records: [MacAgentHostMemoryRecord]
}

nonisolated private struct MacMemoryMutationEnvelope: Decodable {
    let record: MacAgentHostMemoryRecord?
}

extension MacAgentHostRuntime {
    func listMemoryRecords() async throws -> [MacAgentHostMemoryRecord] {
        let data = try await runCapture(arguments: ["memory", "list", "--json"])
        return try JSONDecoder().decode(MacMemoryListEnvelope.self, from: data).records
    }

    func createMemory(_ draft: MacAgentHostMemoryDraft) async throws -> MacAgentHostMemoryRecord {
        try await mutateMemory(arguments: ["memory", "create", "--json"], draft: draft)
    }

    func reviseMemory(
        id: String,
        draft: MacAgentHostMemoryDraft
    ) async throws -> MacAgentHostMemoryRecord {
        try await mutateMemory(arguments: ["memory", "revise", id, "--json"], draft: draft)
    }

    func deleteMemory(id: String) async throws {
        _ = try await runCapture(arguments: ["memory", "delete", id, "--json"])
    }

    private func mutateMemory(
        arguments: [String],
        draft: MacAgentHostMemoryDraft
    ) async throws -> MacAgentHostMemoryRecord {
        var data = try JSONEncoder().encode(draft)
        data.append(0x0A)
        let output = try await runCapture(arguments: arguments, input: data)
        let envelope = try JSONDecoder().decode(MacMemoryMutationEnvelope.self, from: output)
        guard let record = envelope.record else {
            throw NSError(
                domain: "NolaMac.Memory",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: String(localized: "The memory store returned no record.")]
            )
        }
        return record
    }
}
