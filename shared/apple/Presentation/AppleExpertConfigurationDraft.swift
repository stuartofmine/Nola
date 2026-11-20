// SPDX-License-Identifier: Apache-2.0
import SwiftUI

nonisolated struct AppleExpertExecutor: Hashable, Codable, Sendable {
    let backend: String
    var model: String? = nil
    var model_provider: String? = nil

    static func backendKey(_ id: String) -> String { id == "opencode" ? "open_code" : id }
}

nonisolated struct AppleExpertExecutorModel: Decodable, Hashable, Sendable {
    let id: String
    let display_name: String
    let provider_id: String?
}

nonisolated struct AppleExpertExecutorOption: Identifiable, Sendable {
    let id: String
    let name: String
    let available: Bool
    let models: [AppleExpertExecutorModel]

    var key: String { AppleExpertExecutor.backendKey(id) }
    func label(_ model: AppleExpertExecutorModel) -> String {
        guard key == "codex", let provider = model.provider_id else { return model.display_name }
        return model.display_name + " · " + provider
    }

    func selection(_ model: AppleExpertExecutorModel) -> AppleExpertExecutor {
        .init(backend: key, model: model.id, model_provider: key == "codex" ? model.provider_id : nil)
    }
}

struct AppleExpertExecutorControls: View {
    @Binding var selected: AppleExpertExecutor?
    let catalog: [AppleExpertExecutorOption]
    let executorLabel: LocalizedStringKey
    let modelLabel: LocalizedStringKey
    let defaultLabel: LocalizedStringKey

    var body: some View {
        if !catalog.isEmpty || selected != nil {
            Picker(executorLabel, selection: Binding(get: { selected?.backend ?? "" }, set: { id in
                if selected?.backend != id { selected = id.isEmpty ? nil : .init(backend: id) }
            })) {
                Text("Nola").tag("")
                ForEach(catalog) { item in Text(item.name).tag(item.key).disabled(!item.available) }
                if let selected, !catalog.contains(where: { $0.key == selected.backend }) {
                    Text(verbatim: selected.backend).tag(selected.backend).disabled(true)
                }
            }.accessibilityIdentifier("experts.executor")
            if let binding = selected {
                let entry = catalog.first { $0.key == binding.backend }
                let choices = entry?.models ?? []
                Picker(modelLabel, selection: $selected) {
                    Text(defaultLabel).tag(AppleExpertExecutor?.some(.init(backend: binding.backend)))
                    if let entry {
                        ForEach(choices, id: \.self) { model in
                            Text(entry.label(model))
                                .tag(Optional(entry.selection(model)))
                        }
                    }
                    if binding != .init(backend: binding.backend), !choices.contains(where: { entry?.selection($0) == binding }) {
                        Text(verbatim: [binding.model, binding.model_provider].compactMap { $0 }.joined(separator: " · "))
                            .tag(Optional(binding)).disabled(true)
                    }
                }.accessibilityIdentifier("experts.executor.model")
            }
        }
    }
}

/// Selector editing only. Core owns defaults validation and configuration
/// capture; platform adapters retain the exact original JSON representation.
nonisolated struct AppleExpertConfigurationDraft: Equatable, Sendable {
    var executor: AppleExpertExecutor?
    var modelID: String
    var templateID: String
    var reasoningEffort: String

    init(modelID: String? = nil, templateID: String? = nil, reasoningEffort: String? = nil) {
        self.modelID = modelID ?? ""
        self.templateID = templateID ?? ""
        self.reasoningEffort = reasoningEffort ?? "automatic"
    }

    func applying<JSON: Codable>(to original: [String: JSON], originalSelection: Self,
                        makeString: (String) -> JSON) throws -> [String: JSON] {
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
        if executor != nil { fields.removeValue(forKey: "expertReasoningEffort") }
        if executor != originalSelection.executor {
            fields["expertExecutor"] = try executor.map { try JSONDecoder().decode(JSON.self, from: JSONEncoder().encode($0)) }
        }
        return fields
    }
}
