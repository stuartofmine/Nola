// SPDX-License-Identifier: Apache-2.0
import Foundation

extension AppleExpertConfigurationDraft {
    nonisolated init(defaults: AgentCoreJSONValue) throws {
        guard case .object(let fields) = defaults else { throw IOSApplicationRuntimeError.invalidResponse }
        let model: String?, template: String?
        switch fields["modelSelectionID"] {
        case .string(let value): model = value
        case .none, .null: model = nil
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
        switch fields["selectedPromptTemplateID"] {
        case .string(let value): template = value
        case .none, .null: template = nil
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
        self.init(modelID: model, templateID: template)
    }

    nonisolated func applying(to defaults: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard case .object(let fields) = defaults else { throw IOSApplicationRuntimeError.invalidResponse }
        return .object(applying(to: fields, originalSelection: try Self(defaults: defaults), makeString: { .string($0) }))
    }
}
