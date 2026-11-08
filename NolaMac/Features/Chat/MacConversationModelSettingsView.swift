// SPDX-License-Identifier: Apache-2.0
import SwiftUI

@MainActor
enum MacConversationReasoningAdmission {
    @discardableResult
    static func validate(
        declaration: MacReasoningConfiguration?, protocol wireProtocol: MacModelWireProtocol,
        outputLimit: Int, supportsReasoning: Bool, preference: String,
        port: MacApplicationPort = .shared
    ) throws -> MacCanonicalJSONValue {
        let selection: MacCanonicalJSONValue
        if preference == "automatic" { selection = .null }
        else if let declared = declaration?.selection(preference) {
            selection = try MacCanonicalJSONValue(foundationValue: declared)
        } else { throw MacApplicationPortError.rejected("The reasoning preference has no declared transport selection.") }
        return try port.resolveReasoning(declaration: declaration, wireProtocol: wireProtocol,
            outputLimit: outputLimit, supportsReasoning: supportsReasoning,
            requested: selection, tier: nil)
    }
}

struct MacConversationModelSettingsView: View {
    @Bindable var model: MacWorkspaceModel
    @State private var draft: MacConversationModelOverride
    @State private var validationError: String?
    @State private var promptTemplateID: UUID?
    @ObservedObject private var promptTemplates = MacPromptLibrary.service
    private let initialPromptTemplateID: UUID?
    @Environment(\.openWindow) private var openWindow
    @State private var numericText: [String: String] = [:]
    @State private var invalidNumericFields: Set<String> = []
    @State private var showsDiscardConfirmation = false
    private let initialDraft: MacConversationModelOverride
    @Environment(\.dismiss) private var dismiss

    init(model: MacWorkspaceModel) {
        self.model = model
        initialDraft = model.conversationModelOverride
        initialPromptTemplateID = model.selectedPromptTemplateID
        _promptTemplateID = State(initialValue: model.selectedPromptTemplateID)
        _draft = State(initialValue: model.conversationModelOverride)
    }

    private var inheritedParameters: ModelParameters {
        var parameters = model.defaultModelParameters
        if let context = model.selectedConversation?.contextLimitOverride {
            parameters.maxContextLength = context
        }
        return parameters
    }

    private var draftProtocol: MacModelWireProtocol {
        draft.wireProtocol ?? model.selectedRemoteConfiguration?.wireProtocol ?? .chatCompletions
    }

    private var draftReasoning: MacReasoningConfiguration {
        var value = model.selectedRemoteConfiguration?.reasoning ?? .init()
        value.responses = draftProtocol == .responses
        value.outputLimit = draft.maxOutputTokens ?? inheritedParameters.maxOutputTokens
        return value
    }

    private func validateSelectedReasoningPreference() throws {
        guard model.executionHost != .remote else { return }
        try MacConversationReasoningAdmission.validate(
            declaration: draftReasoning, protocol: draftProtocol,
            outputLimit: draft.maxOutputTokens ?? inheritedParameters.maxOutputTokens,
            supportsReasoning: model.selectedRemoteConfiguration?.executionCapabilities.reasoning ?? false,
            preference: draft.reasoningPreference ?? inheritedParameters.reasoningPreference ?? "automatic")
    }

    private var reasoningValidationError: String? {
        do { try validateSelectedReasoningPreference(); return nil }
        catch { return error.localizedDescription }
    }

    private var samplingAvailable: Bool {
        model.selectedRemoteConfiguration?.executionCapabilities.samplingParameters ?? true
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(String(localized: "Conversation model settings")).font(.title2.weight(.semibold))
            Text(model.activeModelName).foregroundStyle(.secondary)
            Text(String(localized: "Changes apply only to this conversation and model. Unset values inherit model defaults."))
                .font(.callout).foregroundStyle(.secondary)
            ScrollView {
              Form {
                Picker(String(localized: "Prompt"), selection: $promptTemplateID) {
                    Text(String(localized: "None")).tag(UUID?.none)
                    ForEach(promptTemplates.templates) { template in
                        Text(template.name).tag(Optional(template.id))
                    }
                }
                .accessibilityIdentifier("mac-model-prompt-template")
                Button(String(localized: "Open Prompt Library")) { openWindow(id: "prompt-library") }

                if model.executionHost != .remote {
                    Picker(String(localized: "API protocol"), selection: $draft.wireProtocol) {
                        Text(String(localized: "Use model defaults")).tag(MacModelWireProtocol?.none)
                        ForEach(MacModelWireProtocol.allCases) { value in Text(value.title).tag(Optional(value)) }
                    }
                }
                Group {
                    parameter("Temperature", value: $draft.temperature, fallback: inheritedParameters.temperature, range: 0...2)
                    parameter("Top P", value: $draft.topP, fallback: inheritedParameters.topP, range: 0...1)
                }.disabled(!samplingAvailable)
                if !samplingAvailable {
                    Text(String(localized: "Sampling controls are unavailable for this model.", table: "ModelCenter"))
                        .font(.caption).foregroundStyle(.secondary)
                } else if model.selectedRemoteConfiguration?.executionCapabilities.samplingRequiresNoReasoning == true {
                    Text(String(localized: "Sampling only with reasoning off", table: "ModelCenter"))
                        .font(.caption).foregroundStyle(.secondary)
                }
                integerParameter("Output token limit", value: $draft.maxOutputTokens, fallback: inheritedParameters.maxOutputTokens, range: ModelParameterLimits.outputTokens)
                integerParameter("Context window", value: $draft.maxContextLength, fallback: inheritedParameters.maxContextLength, range: ModelParameterLimits.remoteContextTokens)
                Group {
                    parameter("Presence penalty", value: $draft.presencePenalty, fallback: inheritedParameters.presencePenalty, range: -2...2)
                    parameter("Frequency penalty", value: $draft.frequencyPenalty, fallback: inheritedParameters.frequencyPenalty, range: -2...2)
                }.disabled(!samplingAvailable)
                if model.selectedModelOption?.isLocal == true {
                    integerParameter("Top K", value: $draft.topK, fallback: LocalSamplingSettings(parameters: inheritedParameters).topK, range: 1...1_000_000)
                    parameter("Repetition penalty", value: $draft.repetitionPenalty, fallback: LocalSamplingSettings(parameters: inheritedParameters).repetitionPenalty, range: 0.8...1.4)
                }
                Picker("Seed", selection: Binding(get: {
                    switch draft.seed {
                    case .none: return 0
                    case .clear: return 1
                    case .value: return 2
                    }
                }, set: { selection in
                    draft.seed = selection == 0 ? nil : selection == 1 ? .clear
                        : .value(Int64(inheritedParameters.seed ?? 0))
                    numericText.removeValue(forKey: "Seed")
                    invalidNumericFields.remove("Seed")
                })) {
                    Text("Use model defaults").tag(0)
                    Text("Disabled").tag(1)
                    Text("Custom").tag(2)
                }
                if case .value(let seed) = draft.seed {
                    TextField("Seed", text: Binding(get: {
                        numericText["Seed"] ?? String(seed)
                    }, set: { text in
                        numericText["Seed"] = text
                        if let value = MacSeedOverride.canonicalValue(text) {
                            draft.seed = .value(value)
                            invalidNumericFields.remove("Seed")
                        } else {
                            invalidNumericFields.insert("Seed")
                        }
                    }))
                    .foregroundStyle(invalidNumericFields.contains("Seed") ? Color.red : Color.primary)
                    if invalidNumericFields.contains("Seed") {
                        Label(String(localized: "Enter a valid whole number."), systemImage: "exclamationmark.circle")
                            .font(.caption).foregroundStyle(.red)
                    }
                }
                Toggle(String(localized: "Override stop sequences"), isOn: Binding(get: { draft.stopSequences != nil }, set: { draft.stopSequences = $0 ? inheritedParameters.stopSequences : nil }))
                if draft.stopSequences != nil {
                    TextField(String(localized: "Stop sequences, one per line"), text: Binding(get: { draft.stopSequences ?? "" }, set: { draft.stopSequences = $0 }), axis: .vertical)
                }
                if let error = reasoningValidationError {
                    Text(error)
                        .font(.caption).foregroundStyle(.red)
                }
              }
            }.frame(maxHeight: 500)
            if let validationError { Text(validationError).foregroundStyle(.red).font(.caption) }
            HStack {
                Button(String(localized: "Use model defaults")) {
                    draft = .init()
                    numericText = [:]
                    invalidNumericFields = []
                    validationError = nil
                }
                Spacer()
                Button(String(localized: "Cancel")) {
                    if hasUnsavedChanges { showsDiscardConfirmation = true }
                    else { dismiss() }
                }.keyboardShortcut(.cancelAction)
                Button(String(localized: "Save")) {
                    guard invalidNumericFields.isEmpty else { return }
                    let resolved = draft.applying(to: inheritedParameters)
                    if let error = MacModelParameterValidation.message(
                        parameters: resolved,
                        seedText: resolved.seed.map(String.init) ?? "",
                        isLocal: model.selectedModelOption?.isLocal == true,
                        contextRange: ModelParameterLimits.remoteContextTokens
                    ) {
                        validationError = error
                        return
                    }
                    do {
                        try validateSelectedReasoningPreference()
                    } catch {
                        validationError = error.localizedDescription
                        return
                    }
                    if model.executionHost == .remote { draft.wireProtocol = nil }
                    model.conversationModelOverride = draft
                    if promptTemplateID != initialPromptTemplateID { model.selectPromptTemplate(promptTemplateID) }
                    dismiss()
                }.keyboardShortcut(.defaultAction)
                    .disabled(!invalidNumericFields.isEmpty)
            }
        }.padding(24).frame(minWidth: 440, idealWidth: 520, maxWidth: 680)
            .interactiveDismissDisabled(hasUnsavedChanges)
            .confirmationDialog(String(localized: "Discard unsaved changes?"), isPresented: $showsDiscardConfirmation, titleVisibility: .visible) {
                Button(String(localized: "Discard Changes"), role: .destructive) { dismiss() }
                Button(String(localized: "Continue Editing"), role: .cancel) {}
            }
    }

    private var hasUnsavedChanges: Bool {
        draft != initialDraft || promptTemplateID != initialPromptTemplateID || !invalidNumericFields.isEmpty
    }

    private func parameter(_ title: String, value: Binding<Double?>, fallback: Double, range: ClosedRange<Double>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
          HStack {
            Toggle(isOn: Binding(get: { value.wrappedValue != nil }, set: {
                value.wrappedValue = $0 ? fallback : nil
                numericText.removeValue(forKey: title)
                invalidNumericFields.remove(title)
            })) { Text(LocalizedStringKey(title)) }
            TextField(LocalizedStringKey(title), text: Binding(get: {
                numericText[title] ?? (model.executionHost == .remote && value.wrappedValue == nil
                    ? String(localized: "Use model defaults") : String(value.wrappedValue ?? fallback))
            }, set: { text in
                numericText[title] = text
                if let parsed = ModelParameterNumberParser.decimal(text), range.contains(parsed) {
                    value.wrappedValue = parsed
                    invalidNumericFields.remove(title)
                } else {
                    invalidNumericFields.insert(title)
                }
            }))
                .foregroundStyle(invalidNumericFields.contains(title) ? Color.red : Color.primary)
                .frame(width: 110).disabled(value.wrappedValue == nil)
                .help("\(range.lowerBound)–\(range.upperBound)")
          }
          if invalidNumericFields.contains(title) {
              Label(String(format: String(localized: "Enter a number between %@ and %@."), range.lowerBound.formatted(), range.upperBound.formatted()), systemImage: "exclamationmark.circle")
                  .font(.caption).foregroundStyle(.red)
          }
        }
    }
    private func integerParameter(_ title: String, value: Binding<Int?>, fallback: Int, range: ClosedRange<Int>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
          HStack {
            Toggle(isOn: Binding(get: { value.wrappedValue != nil }, set: {
                value.wrappedValue = $0 ? fallback : nil
                numericText.removeValue(forKey: title)
                invalidNumericFields.remove(title)
            })) { Text(LocalizedStringKey(title)) }
            TextField(LocalizedStringKey(title), text: Binding(get: {
                numericText[title] ?? (model.executionHost == .remote && value.wrappedValue == nil
                    ? String(localized: "Use model defaults") : String(value.wrappedValue ?? fallback))
            }, set: { text in
                numericText[title] = text
                if let parsed = ModelParameterNumberParser.integer(text), range.contains(parsed) {
                    value.wrappedValue = parsed
                    invalidNumericFields.remove(title)
                } else {
                    invalidNumericFields.insert(title)
                }
            }))
                .foregroundStyle(invalidNumericFields.contains(title) ? Color.red : Color.primary)
                .frame(width: 110).disabled(value.wrappedValue == nil)
                .help("\(range.lowerBound)–\(range.upperBound)")
          }
          if invalidNumericFields.contains(title) {
              Label(String(format: String(localized: "Enter a whole number between %@ and %@."), range.lowerBound.formatted(), range.upperBound.formatted()), systemImage: "exclamationmark.circle")
                  .font(.caption).foregroundStyle(.red)
          }
        }
    }
}

/// Keeps unfinished edits visible while making an invalid draft unsaveable.
/// A value/format TextField otherwise leaves the previous valid budget in the
/// binding when the user clears the field or enters non-numeric text.
struct MacReasoningBudgetDraft {
    private var textByIndex: [Int: String] = [:]

    func text(at index: Int, value: Int) -> String {
        guard let text = textByIndex[index], Self.value(text) == value else { return String(value) }
        return text
    }

    mutating func update(_ text: String, at index: Int) -> Int {
        textByIndex[index] = text
        return Self.value(text)
    }

    private static func value(_ text: String) -> Int {
        ModelParameterNumberParser.integer(text) ?? -1
    }
}

struct MacReasoningConfigurationEditor: View {
    @Binding var configuration: MacReasoningConfiguration
    let wireProtocol: MacModelWireProtocol
    @State private var budgetDraft = MacReasoningBudgetDraft()
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker(String(localized: "Thinking control"), selection: Binding(
                get: { configuration.control },
                set: { configuration.control = $0 }
            )) {
                Text(String(localized: "Provider default")).tag(MacReasoningConfiguration.Control.automatic)
                if wireProtocol != .anthropic || configuration.control == .effort {
                    Text("Effort").tag(MacReasoningConfiguration.Control.effort)
                }
                if wireProtocol == .anthropic || configuration.control == .budget {
                    Text(String(localized: "Thinking budget (Anthropic)")).tag(MacReasoningConfiguration.Control.budget)
                }
            }
            if configuration.control == .effort {
                Text(String(localized: "Supported reasoning levels")).font(.caption).foregroundStyle(.secondary)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 90))], alignment: .leading) {
                    ForEach(wireProtocol == .responses ? ["none", "minimal", "low", "medium", "high", "xhigh", "max"] : ["low", "medium", "high"], id: \.self) { value in
                        Toggle(value, isOn: Binding(get: { configuration.efforts.contains(value) }, set: { enabled in
                            configuration.efforts.removeAll { $0 == value }
                            if enabled { configuration.efforts.append(value) }
                        }))
                    }
                }
            }
            if configuration.control == .budget {
                ForEach(Array(["Concise", "Balanced", "Detailed"].enumerated()), id: \.offset) { index, title in
                    HStack {
                        Text(LocalizedStringKey(title))
                        Spacer()
                        TextField(LocalizedStringKey(title), text: Binding(get: {
                            budgetDraft.text(at: index, value: configuration.budgets.indices.contains(index) ? configuration.budgets[index] : 1024)
                        }, set: { text in
                            while configuration.budgets.count < 3 { configuration.budgets.append(1024) }
                            configuration.budgets[index] = budgetDraft.update(text, at: index)
                        }))
                        .textFieldStyle(.roundedBorder)
                        .monospacedDigit()
                        .frame(width: 120)
                        .accessibilityIdentifier("thinking-budget-\(index)")
                    }
                }
            }
            if !configuration.isValid(for: wireProtocol) {
                Text(String(localized: "Invalid thinking configuration. Check the supported efforts, three ordered budgets, and output limit."))
                    .font(.caption).foregroundStyle(.red)
            }
        }
    }
}
