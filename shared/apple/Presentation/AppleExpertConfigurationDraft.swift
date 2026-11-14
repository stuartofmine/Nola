// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Selector editing only. Core owns defaults validation and configuration
/// capture; platform adapters retain the exact original JSON representation.
nonisolated struct AppleExpertConfigurationDraft: Equatable, Sendable {
    var modelID: String
    var templateID: String
    var reasoningEffort: String

    init(modelID: String? = nil, templateID: String? = nil, reasoningEffort: String? = nil) {
        self.modelID = modelID ?? ""
        self.templateID = templateID ?? ""
        self.reasoningEffort = reasoningEffort ?? "automatic"
    }

    func applying<JSON>(to original: [String: JSON], originalSelection: Self,
                        makeString: (String) -> JSON) -> [String: JSON] {
        var fields = original
        for (key, selected, previous) in [
            ("modelSelectionID", modelID, originalSelection.modelID),
            ("selectedPromptTemplateID", templateID, originalSelection.templateID)
        ] where selected != previous {
            if selected.isEmpty { fields.removeValue(forKey: key) }
            else { fields[key] = makeString(selected) }
        }
        if reasoningEffort != originalSelection.reasoningEffort {
            if reasoningEffort == "automatic" { fields.removeValue(forKey: "expertReasoningEffort") }
            else { fields["expertReasoningEffort"] = makeString(reasoningEffort) }
        }
        return fields
    }
}
