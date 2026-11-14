// SPDX-License-Identifier: Apache-2.0
import Foundation

extension AppleExpertConfigurationDraft {
    nonisolated init(defaults: MacCanonicalJSONValue) throws {
        guard case .object(let fields) = defaults else { throw MacApplicationPortError.invalidResponse }
        let model: String?, template: String?, effort: String?
        switch fields["modelSelectionID"] {
        case .string(let id): model = id
        case .none, .null: model = nil
        default: throw MacApplicationPortError.invalidResponse
        }
        switch fields["selectedPromptTemplateID"] {
        case .string(let id): template = id
        case .none, .null: template = nil
        default: throw MacApplicationPortError.invalidResponse
        }
        switch fields["expertReasoningEffort"] {
        case .string(let value): effort = value
        case .none, .null: effort = nil
        default: throw MacApplicationPortError.invalidResponse
        }
        self.init(modelID: model, templateID: template, reasoningEffort: effort)
    }

    nonisolated func applying(to defaults: MacCanonicalJSONValue) throws -> MacCanonicalJSONValue {
        guard case .object(let fields) = defaults else { throw MacApplicationPortError.invalidResponse }
        let original = try Self(defaults: defaults)
        return .object(applying(to: fields, originalSelection: original, makeString: { .string($0) }))
    }
}
