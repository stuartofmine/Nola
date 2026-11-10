// SPDX-License-Identifier: Apache-2.0
// Current tool-source archive claims are history, never execution authority.
import CryptoKit
import Foundation

private nonisolated struct ToolSourceKey: CodingKey {
    let stringValue: String
    var intValue: Int? { nil }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { return nil }
}

private nonisolated func requireToolSourceKeys(_ decoder: Decoder, _ expected: Set<String>) throws {
    let values = try decoder.container(keyedBy: ToolSourceKey.self)
    guard Set(values.allKeys.map(\.stringValue)) == expected else { throw MacApplicationServiceError.invalidResponse }
}

nonisolated struct MacToolSourceBodyReference: Codable, Equatable, Sendable {
    let contentID: String
    let conversationID: String
    let messageID: String
    let partID: String
    let field: MacExportBodyField
    let totalUTF8Bytes: UInt64
    let chunkCount: UInt32
    enum CodingKeys: String, CodingKey {
        case contentID = "content_id", conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
        case field, totalUTF8Bytes = "total_utf8_bytes", chunkCount = "chunk_count"
    }
    init(from decoder: Decoder) throws {
        try requireToolSourceKeys(decoder, ["content_id", "conversation_id", "message_id", "part_id", "field", "total_utf8_bytes", "chunk_count"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        contentID = try values.decode(String.self, forKey: .contentID)
        conversationID = try values.decode(String.self, forKey: .conversationID)
        messageID = try values.decode(String.self, forKey: .messageID)
        partID = try values.decode(String.self, forKey: .partID)
        field = try values.decode(MacExportBodyField.self, forKey: .field)
        totalUTF8Bytes = try values.decode(UInt64.self, forKey: .totalUTF8Bytes)
        chunkCount = try values.decode(UInt32.self, forKey: .chunkCount)
    }
}

nonisolated struct MacToolSourceOriginalOwner: Codable, Equatable, Sendable {
    let partOwner: MacCanonicalPartOwner
    let runID: String
    let turnID: String
    enum CodingKeys: String, CodingKey { case partOwner = "part_owner", runID = "run_id", turnID = "turn_id" }
    init(from decoder: Decoder) throws {
        try requireToolSourceKeys(decoder, ["part_owner", "run_id", "turn_id"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        partOwner = try values.decode(MacCanonicalPartOwner.self, forKey: .partOwner)
        runID = try values.decode(String.self, forKey: .runID)
        turnID = try values.decode(String.self, forKey: .turnID)
    }
}

nonisolated struct MacToolSourceDerivationClaim: Codable, Equatable, Sendable {
    let operationID: String
    let sourceOwner: MacCanonicalPartOwner
    let sourceBindingSHA256: String
    enum CodingKeys: String, CodingKey {
        case operationID = "operation_id", sourceOwner = "source_owner", sourceBindingSHA256 = "source_binding_sha256"
    }
    init(from decoder: Decoder) throws {
        try requireToolSourceKeys(decoder, ["operation_id", "source_owner", "source_binding_sha256"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        operationID = try values.decode(String.self, forKey: .operationID)
        sourceOwner = try values.decode(MacCanonicalPartOwner.self, forKey: .sourceOwner)
        sourceBindingSHA256 = try values.decode(String.self, forKey: .sourceBindingSHA256)
    }
}

nonisolated struct MacToolSourceClaim: Codable, Equatable, Sendable {
    enum Kind: String, Codable, Sendable { case toolCall = "tool_call", toolResult = "tool_result" }
    let schemaVersion: UInt32
    let sourceItemID: String
    let kind: Kind
    let callID: String
    let name: String
    let owner: MacCanonicalPartOwner
    let originalOwner: MacToolSourceOriginalOwner
    let body: MacToolSourceBodyReference
    let bodySHA256: String
    let derivation: MacToolSourceDerivationClaim?
    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", sourceItemID = "source_item_id", kind, callID = "call_id", name, owner
        case originalOwner = "original_owner", body, bodySHA256 = "body_sha256", derivation
    }
    init(from decoder: Decoder) throws {
        try requireToolSourceKeys(decoder, ["schema_version", "source_item_id", "kind", "call_id", "name", "owner", "original_owner", "body", "body_sha256", "derivation"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try values.decode(UInt32.self, forKey: .schemaVersion)
        sourceItemID = try values.decode(String.self, forKey: .sourceItemID)
        kind = try values.decode(Kind.self, forKey: .kind)
        callID = try values.decode(String.self, forKey: .callID)
        name = try values.decode(String.self, forKey: .name)
        owner = try values.decode(MacCanonicalPartOwner.self, forKey: .owner)
        originalOwner = try values.decode(MacToolSourceOriginalOwner.self, forKey: .originalOwner)
        body = try values.decode(MacToolSourceBodyReference.self, forKey: .body)
        bodySHA256 = try values.decode(String.self, forKey: .bodySHA256)
        // Explicit null is valid; an absent field is not the current format.
        derivation = try values.decode(Optional<MacToolSourceDerivationClaim>.self, forKey: .derivation)
        try validate(owner: owner)
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(schemaVersion, forKey: .schemaVersion)
        try values.encode(sourceItemID, forKey: .sourceItemID)
        try values.encode(kind, forKey: .kind)
        try values.encode(callID, forKey: .callID)
        try values.encode(name, forKey: .name)
        try values.encode(owner, forKey: .owner)
        try values.encode(originalOwner, forKey: .originalOwner)
        try values.encode(body, forKey: .body)
        try values.encode(bodySHA256, forKey: .bodySHA256)
        try values.encode(derivation, forKey: .derivation)
    }

    func validate(owner expected: MacCanonicalPartOwner) throws {
        func validID(_ value: String, maximum: Int = 128) -> Bool {
            !value.isEmpty && value.utf8.count <= maximum && value == value.trimmingCharacters(in: .whitespacesAndNewlines)
                && !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
        }
        func validOwner(_ value: MacCanonicalPartOwner) -> Bool {
            [value.conversationID, value.messageID, value.partID].allSatisfy { validID($0) }
        }
        guard schemaVersion == 1, owner == expected, validOwner(owner), validOwner(originalOwner.partOwner),
              validID(sourceItemID), validID(callID, maximum: 512), validID(name, maximum: 512),
              validID(originalOwner.runID), validID(originalOwner.turnID), validID(body.contentID),
              body.conversationID == owner.conversationID, body.messageID == owner.messageID, body.partID == owner.partID,
              body.field == .contentJSON, body.chunkCount > 0, body.totalUTF8Bytes >= UInt64(body.chunkCount),
              body.totalUTF8Bytes <= UInt64(body.chunkCount) * 32768, body.totalUTF8Bytes <= 256 * 1024 * 1024,
              Self.validDigest(bodySHA256) else { throw MacApplicationServiceError.invalidResponse }
        if let derivation {
            guard validID(derivation.operationID), validOwner(derivation.sourceOwner), Self.validDigest(derivation.sourceBindingSHA256) else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
    }
    static func validDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    static func validate(_ claims: [Self], owner: MacCanonicalPartOwner) throws {
        guard claims.count <= 2, Set(claims.map(\.sourceItemID)).count == claims.count,
              Set(claims.map { $0.kind.rawValue }).count == claims.count else { throw MacApplicationServiceError.invalidResponse }
        for claim in claims { try claim.validate(owner: owner) }
        if let first = claims.first {
            guard claims.allSatisfy({ $0.callID == first.callID && $0.name == first.name && $0.originalOwner == first.originalOwner }) else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
    }
}

nonisolated struct MacToolSourceExportRange: Codable, Sendable {
    let owner: MacCanonicalPartOwner
    let sourceItemID: String
    let bodySHA256: String
    let revision: Int64
    let offsetBytes: UInt64
    let nextOffsetBytes: UInt64
    let totalUTF8Bytes: UInt64
    let text: String
    let endOfContent: Bool
    enum CodingKeys: String, CodingKey {
        case owner, sourceItemID = "source_item_id", bodySHA256 = "body_sha256", revision, text
        case offsetBytes = "offset_bytes", nextOffsetBytes = "next_offset_bytes", totalUTF8Bytes = "total_utf8_bytes", endOfContent = "end_of_content"
    }
}

nonisolated struct MacArchivedToolSource: Codable, Sendable {
    let claim: MacToolSourceClaim
    let bodyJSON: String
    enum CodingKeys: String, CodingKey { case claim, bodyJSON = "body_json" }
    func validate() throws {
        try claim.validate(owner: claim.owner)
        guard UInt64(bodyJSON.utf8.count) == claim.body.totalUTF8Bytes else { throw MacApplicationServiceError.invalidResponse }
        var digest = SHA256()
        for chunk in MacTranscriptTextChunks(text: bodyJSON) { digest.update(data: Data(chunk.text.utf8)) }
        guard digest.finalize().map({ String(format: "%02x", $0) }).joined() == claim.bodySHA256 else {
            throw MacApplicationServiceError.invalidResponse
        }
    }
}
