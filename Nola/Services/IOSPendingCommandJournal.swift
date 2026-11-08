// SPDX-License-Identifier: Apache-2.0
// Transport intent only. Execution truth and retirement authority live in Application receipts.
import Foundation
import Darwin

/// Transport ordering only, not authorization. Every path below names an exact
/// current Application command schema. Unknown/cross-domain writes stay global.
nonisolated enum IOSPendingCommandOwner: Equatable {
    case global
    case conversation(String)
    case run(String)

    private enum Command: String {
        case begin = "begin_assistant_turn", response = "reduce_and_accept_host_response"
        case beginAutomation = "begin_automation_assistant_turn"
        case beginQueued = "begin_queued_user_turn", promoteQueued = "promote_queued_assistant_turn"
        case active = "reduce_and_commit_active_submission", idle = "reduce_and_commit_idle_submission"
        case cancel = "request_run_cancellation", recover = "resolve_unknown_effect_recovery"
        case recoverOwnedTask = "resolve_owned_task_recovery"
        case respond = "respond_to_conversation_wait", metadata = "patch_conversation_metadata"
        case configuration = "patch_conversation_configuration", revert = "revert_conversation"
        case prepareRemote = "prepare_remote_submission", claim = "claim_committed_host_requests"
        case heartbeat = "heartbeat_effect_delivery", claimCancellation = "claim_cancellation_host_requests"
        case bindRemote = "bind_remote_submission_request", cancelRemote = "cancel_source_submission"
        case registerRemote = "register_source_submission", rejectRemote = "reject_source_submission"
        case abandonRemote = "abandon_prepared_remote_submission", acceptRemote = "accept_remote_submission_receipt"
        case adoptRemote = "adopt_observed_remote_run", create = "create_conversation"
        case delete = "delete_conversation", rename = "rename_conversation"
        case policy = "apply_active_permission_policy", expired = "recover_expired_run_deliveries"
        case observed = "recover_observed_orphaned_run", enqueue = "enqueue_agent_command"
        case claimAgent = "claim_next_agent_command", releaseAgent = "release_agent_command_lease"
        case acknowledgeAgent = "acknowledge_agent_command", acquireSession = "acquire_session_run_lease"
        case releaseSession = "release_session_run_lease"
        case stepGoal = "step_goal"
        case openSnapshot = "open_assistant_journal_snapshot", releaseSnapshot = "release_assistant_journal_snapshot"
        case remoteUsage = "accept_remote_run_context_usage", remoteAdmissions = "receive_run_message_admissions"
        case remoteRecord = "receive_assistant_journal_record", remoteContent = "stage_assistant_journal_content"
        case remoteTerminalBegin = "begin_receive_assistant_run_terminal"
        case remoteTerminalStage = "stage_assistant_run_terminal_manifest"
        case remoteTerminalCommit = "commit_received_assistant_run_terminal"
    }

    static func decode(_ envelope: [String: Any]) throws -> Self {
        guard let command = envelope["command"] as? [String: Any],
              let type = command["type"] as? String,
              let arguments = command["arguments"] as? [String: Any] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        guard let kind = Command(rawValue: type) else { return .global }
        let path: [String]
        switch kind {
        case .begin, .beginQueued, .promoteQueued, .beginAutomation, .response, .active, .idle, .cancel, .recover, .respond,
             .metadata, .configuration, .revert, .prepareRemote, .recoverOwnedTask:
            path = ["input", "conversation_id"]
        case .claim, .heartbeat: path = ["input", "run_id"]
        case .claimCancellation: path = ["input", "identity", "conversation_id"]
        case .bindRemote, .cancelRemote, .registerRemote, .rejectRemote, .abandonRemote:
            path = ["identity", "conversation_id"]
        case .acceptRemote, .adoptRemote: path = ["receipt", "identity", "conversation_id"]
        case .create: path = ["input", "id"]
        case .delete, .rename, .policy, .expired, .observed, .stepGoal: path = ["conversation_id"]
        case .enqueue, .acquireSession: path = ["input", "session_id"]
        case .claimAgent, .releaseAgent, .acknowledgeAgent, .releaseSession: path = ["session_id"]
        case .openSnapshot: path = ["at", "identity", "conversation_id"]
        case .releaseSnapshot: path = ["snapshot", "at", "identity", "conversation_id"]
        case .remoteUsage: path = ["record", "identity", "conversation_id"]
        case .remoteAdmissions: path = ["page", "through", "identity", "conversation_id"]
        case .remoteRecord: path = ["record", "header", "identity", "conversation_id"]
        case .remoteContent: path = ["input", "record", "header", "identity", "conversation_id"]
        case .remoteTerminalBegin, .remoteTerminalCommit: path = ["seal", "identity", "conversation_id"]
        case .remoteTerminalStage: path = ["page", "seal", "identity", "conversation_id"]
        }
        var value: Any = arguments
        for field in path {
            guard let next = (value as? [String: Any])?[field] else { throw IOSApplicationRuntimeError.invalidResponse }
            value = next
        }
        guard let id = value as? String, !id.isEmpty, id.count <= 512,
              id.trimmingCharacters(in: .whitespacesAndNewlines) == id,
              !id.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return kind == .claim || kind == .heartbeat ? .run(id) : .conversation(id)
    }

    static func blockingRequest(for envelope: [String: Any], pending: [String: [String: Any]],
                                resolve: (Self) throws -> Self) throws -> String? {
        let target = try resolve(decode(envelope))
        for id in pending.keys.sorted() {
            let owner = try resolve(decode(pending[id]!))
            if target == .global || owner == .global || target == owner { return id }
        }
        return nil
    }
    static func validateJournalOwners(_ pending: [String: [String: Any]]) throws {
        var owners: [Self] = []
        for envelope in pending.values {
            let owner = try decode(envelope)
            guard !owners.contains(owner), !(owner == .global && !owners.isEmpty),
                  !owners.contains(.global) else { throw IOSApplicationRuntimeError.invalidResponse }
            owners.append(owner)
        }
    }

}
