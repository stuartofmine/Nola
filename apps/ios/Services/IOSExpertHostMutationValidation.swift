// SPDX-License-Identifier: Apache-2.0
// Post-commit acknowledgement checks never replace Core authorization.
import Foundation

nonisolated enum IOSExpertHostMutationValidation {
    private struct ActionReceipt: Decodable {
        struct Response: Decodable {
            struct Output: Decodable {
                struct TaskIdentity: Decodable { let id: String }
                let exchange: AppleExpertPeerView
                let task: TaskIdentity?
            }
            let type: String
            let call_id: String
            let output: Output
        }
        let terminal: Bool
        let wait_ms: UInt64
        let response: Response
    }
    private struct AbortReceipt: Decodable {
        let exchange_id: String
        let preparation_aborted: Bool
        let execution_started: Bool?
        let task_may_exist: Bool?
    }
    private static func invalid() -> Error { IOSApplicationRuntimeError.invalidResponse }
    private static func decode<T: Decodable>(_ type: T.Type, _ value: Any?) throws -> T {
        try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: value ?? NSNull(), options: [.fragmentsAllowed]))
    }
    private static func request(_ operation: [String: Any]) throws -> (String, String, String, [String: Any]) {
        guard let request = operation["request"] as? [String: Any], let context = request["session_id"] as? String, !context.isEmpty,
              let run = request["turn_id"] as? String, !run.isEmpty,
              let payload = request["payload"] as? [String: Any], let call = payload["call_id"] as? String, !call.isEmpty,
              let args = payload["input"] as? [String: Any] else { throw invalid() }
        return (context, run, call, args)
    }
    private static func matches(_ view: AppleExpertPeerView, context: String, run: String, call: String) -> Bool {
        view.version == 1 && view.source.isValid && view.source.context_conversation_id == context
            && view.sourceRunID == run && view.sourceCallID == call && !view.exchangeID.isEmpty
    }
    static func validate(_ data: Data, operation: [String: Any]) throws {
        switch operation["kind"] as? String {
        case "admit_peer":
            let input = try decode(AppleExpertPeerInput.self, operation["input"])
            let material = try JSONDecoder().decode(AppleExpertPeerMaterial<AgentCoreJSONValue>.self, from: data)
            let view = material.view, entry = material.entry
            guard matches(view, context: input.sourceContextID, run: input.sourceRunID, call: input.callID),
                  view.kind == input.kind, view.text == input.text, view.inReplyTo == input.inReplyTo,
                  view.target.participant_id == input.targetParticipantID, entry.participant == view.target,
                  entry.child_submission_id == view.exchangeID,
                  entry.child_user_message_id == view.exchangeID + ":user",
                  entry.child_assistant_message_id == view.exchangeID + ":assistant" else { throw invalid() }
        case "prepare_peer_task":
            let (context, run, call, args) = try request(operation)
            guard let id = operation["exchange_id"] as? String,
                  let result = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let prepared = result["prepared"] as? [String: Any], let task = prepared["task"] as? [String: Any],
                  let snapshot = prepared["snapshot"] as? [String: Any],
                  let response = result["response"] as? [String: Any] else { throw invalid() }
            let view = try decode(AppleExpertPeerView.self, result["view"])
            guard matches(view, context: context, run: run, call: call), view.exchangeID == id,
                  view.target.participant_id == args["target_participant_id"] as? String,
                  task["id"] as? String == id, task["owner_session_id"] as? String == context,
                  task["child_session_id"] as? String == view.target.context_conversation_id,
                  snapshot["owner_session_id"] as? String == context else { throw invalid() }
            if prepared["source_admitted"] as? Bool == true {
                guard prepared["replayed"] as? Bool == true,
                      response["type"] as? String == "tool_completed", response["call_id"] as? String == call,
                      let output = response["output"] as? [String: Any],
                      output["exchange_id"] as? String == id, output["task_id"] as? String == id,
                      output["replayed"] as? Bool == true else { throw invalid() }
            } else {
                guard prepared["source_admitted"] as? Bool == false,
                      prepared["replayed"] is Bool,
                      response["task_id"] as? String == id,
                      ["task_promoted", "task_updated"].contains(response["type"] as? String ?? "") else { throw invalid() }
            }
        case "peer_task_action":
            let (context, _, call, args) = try request(operation)
            let receipt = try JSONDecoder().decode(ActionReceipt.self, from: data)
            let exchange = receipt.response.output.exchange
            guard let id = args["exchange_id"] as? String,
                  args["action"] as? String == "cancel", receipt.wait_ms == 0,
                  receipt.response.type == "tool_completed", receipt.response.call_id == call,
                  exchange.version == 1, exchange.exchangeID == id, exchange.source.isValid,
                  exchange.source.context_conversation_id == context,
                  receipt.response.output.task.map({ $0.id == id }) ?? true else { throw invalid() }
        case "abort_peer_task_preparation":
            _ = try request(operation)
            let receipt = try JSONDecoder().decode(AbortReceipt.self, from: data)
            guard receipt.exchange_id == operation["exchange_id"] as? String,
                  receipt.preparation_aborted
                    ? receipt.execution_started == false && receipt.task_may_exist != true
                    : receipt.task_may_exist == true else { throw invalid() }
        case "clarify_peer":
            // Historical committed replies may be replayed; new clarification
            // mutations are rejected by Core under the Main-only contract.
            let input = try decode(AppleExpertPeerClarificationInput.self, operation["input"])
            let view = try JSONDecoder().decode(AppleExpertPeerView.self, from: data)
            guard view.version == 1, view.exchangeID == input.exchangeID,
                  view.clarifications.contains(where: { $0.source.context_conversation_id == input.sourceContextID && $0.text == input.text }) else { throw invalid() }
        default: throw invalid()
        }
    }
}
