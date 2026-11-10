// SPDX-License-Identifier: Apache-2.0
import Foundation

typealias IOSRemoteSubmissionIdentity = AppleRemoteSubmissionIdentity
nonisolated struct IOSRemoteSubmissionIntent: Codable, Sendable {
    let identity: IOSRemoteSubmissionIdentity
    let request_digest: String?
    let phase: String
}
nonisolated struct IOSSourceSubmissionReceipt: Codable, Sendable {
    let protocol_version: Int
    let identity: IOSRemoteSubmissionIdentity
    let request_digest: String
    let phase: String
    let admission: IOSRunMessageAdmissionCursor?
    let rejection_code: String?
}
nonisolated struct IOSConfirmedTerminalOwner: Decodable, Sendable {
    let conversation_id: String
    let run_id: String
    let assistant_message_id: String
}

extension ConversationRemoteTurnStore {
    /// The visible message supplies only a lookup key. Core's committed terminal
    /// supplies the immutable root owner; an ephemeral submission intent does not.
    func confirmedTerminalOwner(conversationID: String,
        observedMessage: IOSCanonicalConversationMessage?) throws -> IOSConfirmedTerminalOwner? {
        guard let observedMessage else { return nil }
        guard let runID = observedMessage.runID else {
            throw IOSApplicationRuntimeError.rejected("The visible Host history has no confirmed Run owner")
        }
        let owner: IOSConfirmedTerminalOwner? = try submissionCommand("canonical_run_terminal", [
            "conversation_id": .string(conversationID), "run_id": .string(runID),
        ])
        guard let owner, owner.conversation_id == conversationID, owner.run_id == runID,
              !owner.assistant_message_id.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("The previous Host turn has no confirmed terminal owner; reconnect before continuing.")
        }
        return owner
    }

    func submissionCommand<Result: Decodable>(_ type: String, _ fields: [String: AgentCoreJSONValue]) throws -> Result {
        var fields = fields; fields["type"] = .string(type)
        return try JSONDecoder().decode(Result.self, from: JSONEncoder().encode(executeRemoteJournalCommand(.object(fields))))
    }
    func remoteSubmissionIntent(conversationID: String) throws -> IOSRemoteSubmissionIntent? {
        try submissionCommand("read_remote_submission_intent", ["conversation_id":.string(conversationID)])
    }
    func bindRemoteSubmissionRequest(identity: IOSRemoteSubmissionIdentity, request: [String: Any]) throws -> IOSRemoteSubmissionIntent {
        try submissionCommand("bind_remote_submission_request", ["identity":try submissionValue(identity),
            "request":try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: request))])
    }
    func remoteSubmissionRequest(identity: IOSRemoteSubmissionIdentity, digest: String) throws -> AgentCoreJSONValue {
        try submissionCommand("read_remote_submission_request", ["identity": try submissionValue(identity),
            "request_digest": .string(digest)])
    }
    func acceptRemoteSubmissionReceipt(_ receipt: IOSSourceSubmissionReceipt) throws -> IOSRemoteSubmissionIntent {
        try submissionCommand("accept_remote_submission_receipt", ["receipt":try submissionValue(receipt)])
    }
    func abandonPreparedSubmission(_ identity: IOSRemoteSubmissionIdentity) throws -> IOSRemoteSubmissionIntent {
        try submissionCommand("abandon_prepared_remote_submission", ["identity":try submissionValue(identity)])
    }
    func adoptObservedRemoteRun(_ receipt: IOSSourceSubmissionReceipt) throws {
        let _: AgentCoreJSONValue = try submissionCommand("adopt_observed_remote_run", ["receipt":try submissionValue(receipt)])
    }
    func hasRemoteCanonicalRun(conversationID: String, runID: String) throws -> Bool {
        let value: AgentCoreJSONValue = try submissionCommand("canonical_run_terminal", [
            "conversation_id": .string(conversationID), "run_id": .string(runID)])
        return value != .null
    }
    private func submissionValue<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
    }
}
