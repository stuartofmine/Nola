// SPDX-License-Identifier: Apache-2.0
// Read-only execution discovery. Completion remains an Application journal fact.
import Foundation

nonisolated struct MacRemoteRunObservation: Decodable, Sendable {
    enum Recovery: String, Decodable, Sendable { case observe, resume_required, terminal, inconsistent }
    struct Worker: Decodable, Sendable {
        enum State: String, Decodable, Sendable { case running, absent, starting, unknown }
        let state: State
        let generation: String?
    }
    let protocol_version: Int
    let conversation_id: String
    let client_id: String
    let run_id: String
    let assistant_message_id: String
    let status: String
    let worker: Worker
    let recovery: Recovery
    let admission: MacRunMessageAdmissionCursor?
    let terminal_seal: MacAssistantRunTerminalSeal?
    let permissions: [MacCanonicalJSONValue]
    let can_submit: Bool
    var submission_receipt: MacSourceSubmissionReceipt? = nil
    var context_usage: MacCommittedRunContextUsage? = nil

    func validate(conversationID: String, clientID: String,
                  expectedRunID: String?, assistantMessageID: String?) throws {
        guard protocol_version == 1, conversation_id == conversationID, client_id == clientID,
              !run_id.isEmpty, !assistant_message_id.isEmpty,
              ["running", "sealed"].contains(status),
              expectedRunID == nil || expectedRunID == run_id,
              assistantMessageID == nil || assistantMessageID == assistant_message_id,
              permissions.count <= 16,
              let admission, admission.identity.conversation_id == conversationID,
              admission.identity.run_id == run_id,
              admission.identity.assistant_message_id == assistant_message_id else {
            throw MacRemoteHostError.invalidResponse
        }
        if let seal = terminal_seal {
            guard recovery == .terminal, seal.identity == admission.identity,
                  seal.protocol_version == 1, status == "sealed",
                  seal.result.run_id == run_id,
                  seal.result.conversation_id == conversation_id else { throw MacRemoteHostError.invalidResponse }
        } else {
            guard recovery != .terminal, !can_submit else { throw MacRemoteHostError.invalidResponse }
        }
        if let usage = context_usage {
            guard usage.identity == admission.identity, usage.admission.identity == admission.identity else {
                throw MacRemoteHostError.invalidResponse
            }
        }
        switch worker.state {
        case .absent:
            guard worker.generation == nil,
                  recovery == .resume_required || recovery == .terminal else { throw MacRemoteHostError.invalidResponse }
        case .running, .starting:
            guard let generation = worker.generation, let parsed = UInt64(generation),
                  String(parsed) == generation,
                  recovery == .observe || recovery == .terminal else { throw MacRemoteHostError.invalidResponse }
        case .unknown:
            guard let generation = worker.generation, let parsed = UInt64(generation), String(parsed) == generation,
                  recovery == .inconsistent || recovery == .terminal else { throw MacRemoteHostError.invalidResponse }
        }
        guard permissions.isEmpty || (worker.state == .running && recovery == .observe) else {
            throw MacRemoteHostError.invalidResponse
        }
    }
}

nonisolated struct MacRemoteRunObservationBatch: Sendable {
    let observation: MacRemoteRunObservation?
    // Journal markers only after local durable catchup; permission events keep
    // the original live nonce. No body or trace is an authorization source.
    let events: [MacLocalAgentEvent]
}
