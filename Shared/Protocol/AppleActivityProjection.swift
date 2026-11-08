// SPDX-License-Identifier: Apache-2.0
// Core history metadata shared by both Apple clients; no execution authority.
import Foundation
nonisolated struct AppleActivityCursor: Codable, Sendable {
    let created_at_ms: Int64
    let run_id: String
}
nonisolated struct AppleActivityQuery: Encodable, Sendable {
    let source: String?
    let needs_response: Bool
    let query: String
    let cursor: AppleActivityCursor?
    let limit: Int
}
nonisolated struct AppleActivityPage: Decodable, Sendable {
    let entries: [AppleActivityEntry]
    let next_cursor: AppleActivityCursor?
}
nonisolated struct AppleActivityEntry: Decodable, Identifiable, Hashable, Sendable {
    var id: String { run_id }
    let run_id: String
    let conversation_id: String
    let assistant_message_id: String
    let source: String
    let title: String
    let created_at_ms: Int64
    let completed_at_ms: Int64?
    let status: String
    let needs_response: Bool
    let is_current: Bool
    var date: Date { Date(timeIntervalSince1970: Double(created_at_ms) / 1000) }
}
nonisolated struct AppleActivityDetail: Decodable, Sendable {
    let entry: AppleActivityEntry
    let result_text: String?
    let result_has_more: Bool
    let explanation: String?
    let source_conversation_id: String?
    let review_in_source: Bool
    let execution_host_id: String?
    let context_scope: String
}
