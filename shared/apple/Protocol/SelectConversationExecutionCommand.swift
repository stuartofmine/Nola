// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Clients select an execution owner; Core owns mutually exclusive settings fields.
nonisolated struct SelectConversationExecutionCommand<JSON: Codable & Sendable>: Encodable, Sendable {
    enum Selection: Encodable, Sendable {
        case inherit
        case location(executionHost: String)
        case model(id: String, parameters: [String: JSON], presentation: [String: JSON])
        case host(binding: [String: JSON], parameters: [String: JSON], presentation: [String: JSON])
        private enum CodingKeys: String, CodingKey { case kind, model_id, binding, parameters, presentation, execution_host }
        func encode(to encoder: Encoder) throws {
            var values = encoder.container(keyedBy: CodingKeys.self)
            switch self {
            case .inherit:
                try values.encode("inherit", forKey: .kind)
            case .location(let executionHost):
                try values.encode("location", forKey: .kind)
                try values.encode(executionHost, forKey: .execution_host)
            case .model(let id, let parameters, let presentation):
                try values.encode("model", forKey: .kind)
                try values.encode(id, forKey: .model_id)
                try values.encode(parameters, forKey: .parameters)
                try values.encode(presentation, forKey: .presentation)
            case .host(let binding, let parameters, let presentation):
                try values.encode("host", forKey: .kind)
                try values.encode(binding, forKey: .binding)
                try values.encode(parameters, forKey: .parameters)
                try values.encode(presentation, forKey: .presentation)
            }
        }
    }
    struct Input: Encodable, Sendable {
        let conversation_id: String
        let expected_revision: Int64
        let selection: Selection
        let updated_at_ms: Int64
        var execution_host: String? = nil
    }
    let type = "select_conversation_execution"
    let input: Input
}
