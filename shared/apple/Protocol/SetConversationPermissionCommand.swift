// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Core atomically checks the observed execution owner and configuration revision.
nonisolated struct SetConversationPermissionCommand: Encodable, Sendable {
    let conversationID: String
    let expectedRevision: Int64
    let expectedRunID: String?
    let expectedSourceHostID: String?
    let profile: String
    let updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case type, profile
        case conversationID = "conversation_id", expectedRevision = "expected_revision"
        case expectedRunID = "expected_run_id", expectedSourceHostID = "expected_source_host_id"
        case updatedAtMs = "updated_at_ms"
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode("set_conversation_permission", forKey: .type)
        try values.encode(conversationID, forKey: .conversationID)
        try values.encode(expectedRevision, forKey: .expectedRevision)
        try values.encode(expectedRunID, forKey: .expectedRunID)
        try values.encode(expectedSourceHostID, forKey: .expectedSourceHostID)
        try values.encode(profile, forKey: .profile)
        try values.encode(updatedAtMs, forKey: .updatedAtMs)
    }
}
