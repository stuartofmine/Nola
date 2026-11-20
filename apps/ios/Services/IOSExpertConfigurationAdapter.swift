// SPDX-License-Identifier: Apache-2.0
import Foundation

extension AppleExpertConfigurationDraft {
    nonisolated init(defaults: AgentCoreJSONValue) throws {
        guard case .object(let fields) = defaults else { throw IOSApplicationRuntimeError.invalidResponse }
        let model: String?, template: String?, effort: String?
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
        switch fields["expertReasoningEffort"] {
        case .string(let value): effort = value
        case .none, .null: effort = nil
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
        self.init(modelID: model, templateID: template, reasoningEffort: effort)
    }

    nonisolated func hasSelectedModelOverride(in defaults: AgentCoreJSONValue) -> Bool {
        guard !modelID.isEmpty, case .object(let fields) = defaults,
              case .object(let models) = fields["modelOverrides"],
              case .object(let overrides) = models[modelID] else { return false }
        return !overrides.isEmpty
    }

    nonisolated func applying(to defaults: AgentCoreJSONValue, removingModelOverrides: Set<String> = []) throws -> AgentCoreJSONValue {
        guard case .object(let fields) = defaults else { throw IOSApplicationRuntimeError.invalidResponse }
        var updated = applying(to: fields, originalSelection: try Self(defaults: defaults), makeString: { .string($0) })
        if !removingModelOverrides.isEmpty, case .object(let models) = updated["modelOverrides"] {
            let remaining = models.filter { !removingModelOverrides.contains($0.key) }
            updated["modelOverrides"] = remaining.isEmpty ? nil : .object(remaining)
        }
        return .object(updated)
    }
}
