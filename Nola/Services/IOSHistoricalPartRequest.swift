// SPDX-License-Identifier: Apache-2.0
// Immutable identity for one revision-pinned historical read.
import Foundation

nonisolated struct IOSHistoricalPartRequest: Identifiable, Equatable, Sendable {
    struct Owner: Codable, Equatable, Sendable {
        let conversationID: String
        let messageID: String
        let partID: String
        enum CodingKeys: String, CodingKey {
            case conversationID = "conversation_id"
            case messageID = "message_id"
            case partID = "part_id"
        }
    }
    let owner: Owner
    let revision: Int64
    let kind: String
    var id: String { [owner.conversationID, owner.messageID, owner.partID, String(revision)].map { "\($0.utf8.count):\($0)" }.joined() }
    init?(_ projection: IOSConversationProjectionV1.HistoricalProjection) {
        guard case .object(let owner) = projection.owner,
              case .string(let conversation)? = owner["conversation_id"],
              case .string(let message)? = owner["message_id"],
              case .string(let part)? = owner["part_id"] else { return nil }
        self.owner = Owner(conversationID: conversation, messageID: message, partID: part)
        revision = projection.canonicalRevision
        kind = projection.canonicalKind
    }
}

/// Existing archive contract, not a path- or locator-based authorization.
nonisolated struct IOSHistoricalOriginalQuery: Encodable, Equatable, Sendable {
    let type = "read_conversation_export_range"
    let conversationID: String
    let messageID: String
    let partID: String
    let field = "content_json"
    let expectedRevision: Int64
    let offsetBytes: UInt64
    let limitBytes = 8192
    init(_ request: IOSHistoricalPartRequest, offset: UInt64) {
        conversationID = request.owner.conversationID
        messageID = request.owner.messageID
        partID = request.owner.partID
        expectedRevision = request.revision
        offsetBytes = offset
    }
    enum CodingKeys: String, CodingKey {
        case type, field
        case conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
        case expectedRevision = "expected_revision", offsetBytes = "offset_bytes", limitBytes = "limit_bytes"
    }
}

nonisolated struct IOSHistoricalOriginalPage: Decodable, Equatable, Sendable {
    let conversationID: String
    let messageID: String
    let partID: String
    let field: String
    let revision: Int64
    let offsetBytes: UInt64
    let nextOffsetBytes: UInt64
    let totalUTF8Bytes: UInt64
    let text: String
    let endOfContent: Bool
    enum CodingKeys: String, CodingKey {
        case field, revision, text
        case conversationID = "conversation_id", messageID = "message_id", partID = "part_id"
        case offsetBytes = "offset_bytes", nextOffsetBytes = "next_offset_bytes"
        case totalUTF8Bytes = "total_utf8_bytes", endOfContent = "end_of_content"
    }
    func matches(_ query: IOSHistoricalOriginalQuery) -> Bool {
        let next = query.offsetBytes.addingReportingOverflow(UInt64(text.utf8.count))
        return conversationID == query.conversationID && messageID == query.messageID
            && partID == query.partID && field == query.field && revision == query.expectedRevision
            && offsetBytes == query.offsetBytes && !next.overflow && nextOffsetBytes == next.partialValue
            && text.utf8.count <= query.limitBytes && nextOffsetBytes <= totalUTF8Bytes
            && endOfContent == (nextOffsetBytes == totalUTF8Bytes)
            && (endOfContent || nextOffsetBytes > offsetBytes)
    }
}
