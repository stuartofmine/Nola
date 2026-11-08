// SPDX-License-Identifier: Apache-2.0
// Core-authored read-only Today facts. Never an execution or permission command.
import Foundation

nonisolated struct AppleTodayFactsSnapshot<JSON: JournalJSONValue>: Decodable, Sendable {
    let conversations: [AppleTodayConversationFact<JSON>]
    let recent_conversations: [AppleTodayConversationFact<JSON>]
    let unrepresented_expert_contexts: [AppleTodayExpertContextFact<JSON>]
    let needs_attention: Bool
    let has_more_conversations: Bool
    let next_conversation_offset: Int?
    let queued_instances: [AppleTodayQueuedInstance]
    let has_more_queued_instances: Bool
}
nonisolated struct AppleTodayConversationFact<JSON: JournalJSONValue>: Decodable, Sendable {
    let conversation_id: String
    let title: String
    let revision: Int64
    let updated_at_ms: Int64
    let needs_response: Bool
    let active_run: AppleTodayRunOwner?
    let permission_projection: JSON?
    let open_waits: [AppleTodayStoredWait<JSON>]
    let owned_task_waits: [AppleTodayOwnedTaskWait<JSON>]
    let owned_task_permissions: [AppleTodayOwnedTaskPermission<JSON>]
    let expert_contexts: [AppleTodayExpertContextFact<JSON>]
}
nonisolated struct AppleTodayRunOwner: Decodable, Sendable {
    let run_id: String
    let assistant_message_id: String
}
nonisolated struct AppleTodayStoredWait<JSON: JournalJSONValue>: Decodable, Sendable {
    let conversation_id: String
    let run_id: String
    let wait: AppleTodayWait<JSON>
}
nonisolated struct AppleTodayWait<JSON: JournalJSONValue>: Decodable, Sendable {
    let wait_id: String
    let core_session_id: String
    let turn_id: String
    let originating_call_id: String?
    let revision: UInt64
    let created_at_revision: UInt64
    let response_schema_digest: String
    let question_set: JSON
    let state: JSON
}
nonisolated struct AppleTodayOwnedTaskWait<JSON: JournalJSONValue>: Decodable, Sendable {
    let task_id: String
    let parent_conversation_id: String
    let parent_run_id: String
    let child_conversation_id: String
    let child_run_id: String
    let admission_attempt_id: String
    let stored: AppleTodayStoredWait<JSON>
}
nonisolated struct AppleTodayOwnedTaskPermission<JSON: JournalJSONValue>: Decodable, Sendable {
    let task_id: String
    let parent_conversation_id: String
    let parent_run_id: String
    let child_conversation_id: String
    let child_run_id: String
    let admission_attempt_id: String
    let permission_projection: JSON
}
nonisolated struct AppleTodayExpertContextFact<JSON: JournalJSONValue>: Decodable, Sendable {
    let room_id: String
    let room_revision: Int64
    let association_revision: Int64?
    let canonical_conversation_id: String?
    let participant_id: String
    let agent_id: String
    let agent_revision: Int64
    let display_name: String
    let context_conversation_id: String
    let context_revision: Int64
    let fact: AppleTodayConversationFact<JSON>
}
nonisolated struct AppleTodayQueuedInstance: Decodable, Sendable {
    let occurrence_id: String
    let automation_id: String
    let definition_revision: Int64
    let title: String?
    let scheduled_for_ms: Int64
}
