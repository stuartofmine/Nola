// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelParametersView logic in the iOS UI layer.
// - Interfaces:
//   - ModelParametersView
// - APIs:
//   - none
//
import SwiftUI

/// Compare fields structurally: prompts and stop sequences may contain any delimiter.
/// A raw number equal to its displayed value is not a change, but an unfinished
/// draft such as "-" must remain dirty even though the parsed value is unchanged.
nonisolated struct ModelParameterDraftFingerprint: Equatable {
    private let values: [String]
    private let displayedNumbers: [String: String]

    init(values: [String], numericInput: [String: String], displayedNumbers: [String: String]) {
        self.values = values
        self.displayedNumbers = displayedNumbers.merging(numericInput) { _, draft in draft }
    }
}

struct ModelParametersView: View {
    private enum NumericField: String, CaseIterable {
        case context, output, topK, topP, temperature, presence, frequency, repetition
        var integer: Bool { self == .context || self == .output }
    }
    let modelID: String
    let displayName: String
    let isLocal: Bool

    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    @State private var maxContextLength: Int = 65_536
    @State private var maxOutputTokens: Int = 16_384
    @State private var topK: Double = 40
    @State private var topP: Double = 0.9
    @State private var temperature: Double = 0.7
    @State private var presencePenalty: Double = 0.0
    @State private var frequencyPenalty: Double = 0.0
    @State private var repetitionPenalty: Double = 1.0
    @State private var seedText: String = ""
    @State private var stopSequences: String = ""
    @State private var reasoningPreference: String = "automatic"
    @State private var reasoningEfforts: [String] = []
    @State private var showThinking: Bool = true
    @State private var streamOutput: Bool = true
    @State private var responseStyle: PromptResponseStyle = .casual
    @State private var customSystemPrompt: String = ""
    @State private var initialSignature: ModelParameterDraftFingerprint?
    @State private var showDiscardConfirmation = false
    @State private var saveError: String?
    @State private var editingSnapshot: ModelParametersStore.ParameterSnapshot?
    @State private var loaded = false
    @State private var numericText: [NumericField: String] = [:]
    @State private var reloadAfterDiscard = false
    private enum InputField: Hashable {
        case systemPrompt, seed, stopSequences
        case numeric(NumericField)
    }
    @FocusState private var inputField: InputField?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.spacing_lg) {
                headerCard
                if let saveError {
                    Text(saveError)
                        .font(.callout)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(NSLocalizedString("settings.logs.reload", value: "Reload", comment: "Reload model parameters")) {
                        if hasUnsavedChanges {
                            reloadAfterDiscard = true
                            showDiscardConfirmation = true
                        } else { reloadParameters() }
                    }
                    .accessibilityIdentifier("model-parameters-reload")
                }
                if editingSnapshot != nil && !hasValidParameters {
                    Text(NSLocalizedString("model.parameters.invalid_values", value: "Check the displayed parameter ranges. Seed must be a whole number.", comment: "Invalid model parameters"))
                        .font(.callout)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("model-parameter-validation")
                }
                Group {
                    limitsCard
                    samplingCard
                    behaviorCard
                    outputCard
                }.disabled(editingSnapshot == nil)
            }
            .padding(.horizontal, DesignTokens.spacing_lg)
            .padding(.top, DesignTokens.spacing_lg)
            .padding(.bottom, DesignTokens.spacing_md)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(DesignTokens.backgroundColor.ignoresSafeArea())
        .safeAreaInset(edge: .bottom, spacing: 0) {
            actionCard
        }
        .navigationTitle(L10n.format("model.parameters.navigation_title_format", displayName))
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(true)
        .interactiveDismissDisabled(hasUnsavedChanges)
        .confirmationDialog(
            NSLocalizedString("model.parameters.discard.title", value: "Discard parameter changes?", comment: "Discard model parameter changes title"),
            isPresented: $showDiscardConfirmation
        ) {
            Button(NSLocalizedString("model.parameters.discard.action", value: "Discard Changes", comment: "Discard model parameter changes action"), role: .destructive) {
                if reloadAfterDiscard { reloadParameters() } else { dismiss() }
                reloadAfterDiscard = false
            }
            Button(L10n.cancel, role: .cancel) { reloadAfterDiscard = false }
        } message: {
            Text(NSLocalizedString("model.parameters.discard.message", value: "Your model parameter changes have not been saved.", comment: "Discard model parameter changes message"))
        }
        .toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button(L10n.done) { inputField = nil }
            }
            ToolbarItem(placement: .cancellationAction) {
                Button(L10n.done) {
                    reloadAfterDiscard = false
                    if hasUnsavedChanges {
                        showDiscardConfirmation = true
                    } else {
                        dismiss()
                    }
                }
            }
        }
        .onAppear {
            guard !loaded else { return }
            loaded = true
            reloadParameters()
        }
    }

    private var headerCard: some View {
        NolaCard {
            NolaSectionTitle(
                icon: isLocal ? "iphone" : "cloud.fill",
                title: isLocal
                    ? NSLocalizedString("model.parameters.local_title", value: "本地模型参数", comment: "Local model params title")
                    : NSLocalizedString("model.parameters.cloud_title", value: "云端模型参数", comment: "Cloud model params title")
            )
            Text(isLocal
                 ? NSLocalizedString("model.parameters.local_hint", value: "这些参数会直接用于本地 MLX 推理；未被本地引擎支持的高级项会保留为配置。", comment: "Local model params hint")
                 : NSLocalizedString("model.parameters.cloud_hint", value: "这些参数会写入 OpenAI 兼容请求，适合按任务调整稳定性、创造性和输出长度。", comment: "Cloud model params hint"))
                .font(.subheadline)
                .foregroundColor(.secondary)
            Text(NSLocalizedString("chat.more_settings.model_output.footer", value: "Advanced defaults are saved for the selected model and apply to every conversation using it.", comment: "Section footer for model and output settings"))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var limitsCard: some View {
        NolaCard {
            NolaSectionTitle(icon: "text.alignleft", title: NSLocalizedString("model.parameters.limits", value: "上下文与输出", comment: "Limits section"))
            integerFieldRow(
                field: .context,
                title: L10n.modelParametersMaxLength,
                value: $maxContextLength,
                range: isLocal ? 65_536...262_144 : ModelParameterLimits.remoteContextTokens,
                unit: "tokens"
            )
            if isLocal {
                Label(
                    NSLocalizedString(
                        "model.parameters.local_context_memory_hint",
                        value: "64K is the recommended context target. Memory use also depends on model architecture and KV-cache precision; choose 0.8B for constrained devices and 4B for higher-memory devices.",
                        comment: "Local context memory guidance"
                    ),
                    systemImage: "memorychip"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            integerFieldRow(field: .output, title: NSLocalizedString("model.parameters.max_output", value: "最大输出", comment: "Max output tokens"), value: $maxOutputTokens, range: ModelParameterLimits.outputTokens, unit: "tokens")
        }
    }

    private var samplingCard: some View {
        NolaCard {
            NolaSectionTitle(icon: "slider.horizontal.3", title: L10n.modelParametersSampling)
            decimalFieldRow(field: .temperature, title: L10n.modelParametersTemperature, value: $temperature, range: 0.0...2.0, decimals: 2, hint: NSLocalizedString("chat.more_settings.temperature.hint", value: "Higher values are more varied; lower values are more stable.", comment: "Temperature hint"))
            decimalFieldRow(field: .topP, title: L10n.modelParametersTopP, value: $topP, range: 0.0...1.0, decimals: 2, hint: NSLocalizedString("chat.more_settings.top_p.hint", value: "Controls the cumulative probability mass of candidate tokens.", comment: "Top P hint"))
            if isLocal {
                decimalFieldRow(field: .topK, title: L10n.modelParametersTopK, value: $topK, range: 1...100, decimals: 0, hint: NSLocalizedString("model.parameters.top_k.hint", value: "Limits sampling to the most likely candidate tokens.", comment: "Top K hint"))
            }
        }
    }

    private var behaviorCard: some View {
        NolaCard {
            NolaSectionTitle(icon: "dial.low", title: NSLocalizedString("model.parameters.behavior", value: "重复与惩罚", comment: "Behavior section"))
            decimalFieldRow(field: .presence, title: NSLocalizedString("model.parameters.presence_penalty", value: "Presence Penalty", comment: "Presence penalty"), value: $presencePenalty, range: -2.0...2.0, decimals: 2, hint: NSLocalizedString("model.parameters.presence_penalty.hint", value: "Encourages the model to introduce new topics.", comment: "Presence penalty hint"))
            decimalFieldRow(field: .frequency, title: NSLocalizedString("model.parameters.frequency_penalty", value: "Frequency Penalty", comment: "Frequency penalty"), value: $frequencyPenalty, range: -2.0...2.0, decimals: 2, hint: NSLocalizedString("model.parameters.frequency_penalty.hint", value: "Reduces repeated words and phrases.", comment: "Frequency penalty hint"))
            if isLocal {
                decimalFieldRow(field: .repetition, title: NSLocalizedString("model.parameters.repetition_penalty", value: "Repetition Penalty", comment: "Repetition penalty"), value: $repetitionPenalty, range: 0.8...1.4, decimals: 2, hint: NSLocalizedString("model.parameters.repetition_penalty.hint", value: "Controls repetition for local inference.", comment: "Repetition penalty hint"))
            }
        }
    }

    private var outputCard: some View {
        NolaCard {
            NolaSectionTitle(icon: "terminal", title: NSLocalizedString("model.parameters.output", value: "输出控制", comment: "Output controls"))
            Picker(NSLocalizedString("model.parameters.response_style", value: "回复风格", comment: "Response style picker"), selection: $responseStyle) {
                ForEach(PromptResponseStyle.allCases) { style in
                    Text(style.displayName).tag(style)
                }
            }
            .pickerStyle(.segmented)
            Text(NSLocalizedString("model.parameters.response_style.hint", value: "系统提示只会注入当前选择的语言风格 prompt。新对话中也可临时切换。", comment: "Response style hint"))
                .font(.caption)
                .foregroundColor(.secondary)
            TextEditor(text: $customSystemPrompt)
                .focused($inputField, equals: .systemPrompt)
                .frame(minHeight: 100)
                .font(.body)
                .padding(DesignTokens.spacing_sm)
                .scrollContentBackground(.hidden)
                .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(Color.primary.opacity(0.08), lineWidth: 0.5)
                }
            Text(
                NSLocalizedString(
                    "model.parameters.custom_system_prompt.hint",
                    value: "可选：追加到内置 system prompt 后。建议写稳定的长期偏好（例如输出格式、语气、优先级）。",
                    comment: "Custom system prompt hint"
                )
            )
                .font(.caption)
                .foregroundColor(.secondary)
            Toggle(L10n.modelParametersStreamOutput, isOn: $streamOutput)
            if !isLocal && !reasoningEfforts.isEmpty {
                HStack {
                    Text(ReasoningEffortPresentation.title)
                    Spacer(minLength: 12)
                    Picker(ReasoningEffortPresentation.title, selection: $reasoningPreference) {
                        Text(ReasoningEffortPresentation.label("automatic")).tag("automatic")
                        ForEach(reasoningEfforts, id: \.self) { effort in
                            Text(ReasoningEffortPresentation.label(effort)).tag(effort)
                        }
                    }
                    .labelsHidden()
                    .accessibilityIdentifier("model-parameters-reasoning")
                }.frame(minHeight: 44)
            }
            Toggle(L10n.modelParametersShowThinking, isOn: $showThinking)
            Text(NSLocalizedString("model.parameters.show_thinking.hint", value: "Shows available reasoning summaries. Turning this off does not change the selected reasoning effort.", comment: "Thinking toggle hint"))
                .font(.caption)
                .foregroundColor(.secondary)
            TextField(NSLocalizedString("model.parameters.seed.optional", value: "Seed（可选）", comment: "Optional seed text field"), text: $seedText)
                .keyboardType(.numbersAndPunctuation)
                .focused($inputField, equals: .seed)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("model-parameter-seed")
            TextField(NSLocalizedString("model.parameters.stop_sequences", value: "停止词，每行一个", comment: "Stop sequences text field"), text: $stopSequences, axis: .vertical)
                .focused($inputField, equals: .stopSequences)
                .lineLimit(2...5)
                .textFieldStyle(.roundedBorder)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            Text(NSLocalizedString("model.parameters.stop_sequences.hint", value: "停止词最多会取前 8 个；Seed 留空表示不固定随机种子。", comment: "Stop sequences and seed hint"))
                .font(.caption)
                .foregroundColor(.secondary)
        }
    }

    private var actionCard: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: DesignTokens.spacing_sm) {
                parameterActionButtons
            }
            VStack(spacing: DesignTokens.spacing_sm) {
                parameterActionButtons
            }
        }
        .padding(.horizontal, DesignTokens.spacing_lg)
        .padding(.vertical, DesignTokens.spacing_sm)
        .background(.regularMaterial)
        .overlay(alignment: .top) {
            Divider().opacity(0.55)
        }
    }

    @ViewBuilder
    private var parameterActionButtons: some View {
        Button(action: saveParameters) {
            Text(L10n.modelParametersSave)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .disabled(!hasUnsavedChanges || !hasValidParameters || editingSnapshot == nil)
        .accessibilityIdentifier("model-parameters-save")

        Button(action: resetToDefaults) {
            Text(L10n.modelParametersResetDefaults)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.bordered)
        .disabled(editingSnapshot == nil)
    }

    private func decimalFieldRow(
        field: NumericField,
        title: String,
        value: Binding<Double>,
        range: ClosedRange<Double>,
        decimals: Int,
        hint: String
    ) -> some View {
        let text = numericText[field] ?? String(value.wrappedValue)
        let parsed = ModelParameterNumberParser.decimal(text)
        let valid = parsed.map { range.contains($0) && (decimals != 0 || $0.rounded() == $0) } ?? false
        return VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
            parameterRowLayout {
                Text(title)
                    .frame(maxWidth: .infinity, alignment: .leading)
                TextField(title, text: Binding(
                    get: { numericText[field] ?? String(value.wrappedValue) },
                    set: { text in
                        numericText[field] = text
                        if let parsed = ModelParameterNumberParser.decimal(text) { value.wrappedValue = parsed }
                    }
                ))
                    .keyboardType(range.lowerBound < 0 ? .numbersAndPunctuation : .decimalPad)
                    .focused($inputField, equals: .numeric(field))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .multilineTextAlignment(.trailing)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 104)
                    .monospacedDigit()
                    .accessibilityIdentifier("model-parameter-\(field.rawValue)")
            }
            Text(hint)
                .font(.caption)
                .foregroundColor(.secondary)
            Text("\(range.lowerBound.formatted()) ... \(range.upperBound.formatted())")
                .font(.caption.monospacedDigit())
                .foregroundStyle(valid ? Color.secondary : Color.red)
        }
    }

    private func integerFieldRow(
        field: NumericField,
        title: String,
        value: Binding<Int>,
        range: ClosedRange<Int>,
        unit: String
    ) -> some View {
        let text = numericText[field] ?? String(value.wrappedValue)
        let valid = ModelParameterNumberParser.integer(text).map(range.contains) ?? false
        return VStack(alignment: .leading, spacing: DesignTokens.spacing_xs) {
            parameterRowLayout {
                Text(title)
                    .frame(maxWidth: .infinity, alignment: .leading)
                TextField(title, text: Binding(
                    get: { numericText[field] ?? String(value.wrappedValue) },
                    set: { text in
                        numericText[field] = text
                        if let parsed = ModelParameterNumberParser.integer(text) { value.wrappedValue = parsed }
                    }
                ))
                    .keyboardType(.numberPad)
                    .focused($inputField, equals: .numeric(field))
                    .multilineTextAlignment(.trailing)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 112)
                    .monospacedDigit()
                    .accessibilityIdentifier("model-parameter-\(field.rawValue)")
            }
            Text("\(range.lowerBound) ... \(range.upperBound) \(unit)")
                .font(.caption.monospacedDigit())
                .foregroundStyle(valid ? Color.secondary : Color.red)
        }
    }

    private var hasValidParameters: Bool {
        let contextRange = isLocal ? 65_536...262_144 : ModelParameterLimits.remoteContextTokens
        let normalizedSeed = seedText.trimmingCharacters(in: .whitespacesAndNewlines)
        return !hasUnparsedNumericInput && contextRange.contains(maxContextLength)
            && ModelParameterLimits.outputTokens.contains(maxOutputTokens)
            && (0...2).contains(temperature) && (0...1).contains(topP)
            && (-2...2).contains(presencePenalty) && (-2...2).contains(frequencyPenalty)
            && (!isLocal || ((1...100).contains(topK) && topK.rounded() == topK && (0.8...1.4).contains(repetitionPenalty)))
            && (normalizedSeed.isEmpty || Int(normalizedSeed) != nil)
    }

    private var parameterRowLayout: AnyLayout {
        if dynamicTypeSize.isAccessibilitySize {
            AnyLayout(VStackLayout(alignment: .leading, spacing: DesignTokens.spacing_sm))
        } else {
            AnyLayout(HStackLayout(spacing: DesignTokens.spacing_md))
        }
    }

    private func saveParameters() {
        guard hasValidParameters else {
            saveError = NSLocalizedString("model.parameters.invalid_values", value: "Check the displayed parameter ranges. Seed must be a whole number.", comment: "Invalid model parameters")
            return
        }
        var params = editingSnapshot?.parameters ?? ModelParameters.defaults(isLocal: isLocal)
        params.maxContextLength = maxContextLength
        params.maxOutputTokens = maxOutputTokens
        if isLocal { params.topK = Int(topK) }
        params.topP = topP
        params.temperature = temperature
        params.presencePenalty = presencePenalty
        params.frequencyPenalty = frequencyPenalty
        if isLocal { params.repetitionPenalty = repetitionPenalty }
        params.seed = Int(seedText.trimmingCharacters(in: .whitespacesAndNewlines))
        params.stopSequences = stopSequences
        if reasoningPreference != (editingSnapshot?.parameters.reasoningPreference ?? "automatic") {
            params.reasoningPreference = reasoningPreference
            params.reasoningTier = nil
        }
        params.showThinking = showThinking
        params.streamOutput = streamOutput
        params.responseStyle = responseStyle
        params.customSystemPrompt = customSystemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        saveError = nil
        do {
            guard let editingSnapshot else { throw IOSApplicationRuntimeError.invalidResponse }
            try ModelParametersStore.shared.saveParameters(params, snapshot: editingSnapshot)
            dismiss()
        } catch {
            saveError = error.localizedDescription
        }
    }

    private func resetToDefaults() {
        apply(ModelParameters.defaults(isLocal: isLocal))
    }

    private func reloadParameters() {
        do {
            let snapshot = try ModelParametersStore.shared.parameterSnapshot(for: modelID)
            let values = try ModelParametersStore.shared.reasoningEfforts(for: snapshot)
            editingSnapshot = snapshot
            reasoningEfforts = values
            apply(snapshot.parameters)
            initialSignature = parameterSignature
            saveError = nil
        } catch {
            saveError = error.localizedDescription
        }
    }

    private func apply(_ params: ModelParameters) {
        numericText = [:]
        maxContextLength = params.maxContextLength
        maxOutputTokens = params.maxOutputTokens
        topK = Double(LocalSamplingSettings(parameters: params).topK)
        topP = params.topP
        temperature = params.temperature
        presencePenalty = params.presencePenalty
        frequencyPenalty = params.frequencyPenalty
        repetitionPenalty = LocalSamplingSettings(parameters: params).repetitionPenalty
        seedText = params.seed.map(String.init) ?? ""
        stopSequences = params.stopSequences
        reasoningPreference = params.reasoningPreference ?? "automatic"
        showThinking = params.showThinking
        streamOutput = params.streamOutput
        responseStyle = params.responseStyle
        customSystemPrompt = params.customSystemPrompt
    }

    private var hasUnsavedChanges: Bool {
        initialSignature.map { parameterSignature != $0 } ?? false
    }

    private var parameterSignature: ModelParameterDraftFingerprint {
        ModelParameterDraftFingerprint(values: [
            String(maxContextLength),
            String(maxOutputTokens),
            String(topK),
            String(topP),
            String(temperature),
            String(presencePenalty),
            String(frequencyPenalty),
            String(repetitionPenalty),
            seedText,
            stopSequences,
            reasoningPreference, String(showThinking),
            String(streamOutput),
            String(describing: responseStyle),
            customSystemPrompt,
        ], numericInput: Dictionary(uniqueKeysWithValues: numericText.map { ($0.key.rawValue, $0.value) }), displayedNumbers: [
            NumericField.context.rawValue: String(maxContextLength),
            NumericField.output.rawValue: String(maxOutputTokens),
            NumericField.topK.rawValue: String(topK),
            NumericField.topP.rawValue: String(topP),
            NumericField.temperature.rawValue: String(temperature),
            NumericField.presence.rawValue: String(presencePenalty),
            NumericField.frequency.rawValue: String(frequencyPenalty),
            NumericField.repetition.rawValue: String(repetitionPenalty),
        ])
    }

    private var hasUnparsedNumericInput: Bool {
        numericText.contains { field, text in
            field.integer ? ModelParameterNumberParser.integer(text) == nil : ModelParameterNumberParser.decimal(text) == nil
        }
    }
}

#Preview {
    NavigationStack {
        ModelParametersView(modelID: "qwen-4b-mlx", displayName: "Qwen 4B MLX", isLocal: true)
    }
}
