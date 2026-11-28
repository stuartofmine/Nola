// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Edits per-model inference parameters with native macOS controls.

import SwiftUI

/// Text and parsed values travel together with the connection draft. Invalid
/// input remains visible and prevents a save rather than saving the old value.
struct MacModelParameterDraft {
    var parameters: ModelParameters
    var text: [String: String] = [:]
    var invalidFields: Set<String> = []
    private let baseline: ModelParameters
    var isEdited: Bool { parameters != baseline || !invalidFields.isEmpty }

    init(parameters: ModelParameters) {
        self.parameters = parameters
        self.baseline = parameters
    }

    mutating func update(_ input: String, field: String, integer keyPath: WritableKeyPath<ModelParameters, Int>, range: ClosedRange<Int>) {
        text[field] = input
        guard let value = ModelParameterNumberParser.integer(input), range.contains(value) else {
            invalidFields.insert(field)
            return
        }
        parameters[keyPath: keyPath] = value
        invalidFields.remove(field)
    }

    mutating func update(_ input: String, field: String, decimal keyPath: WritableKeyPath<ModelParameters, Double>, range: ClosedRange<Double>) {
        text[field] = input
        guard let value = ModelParameterNumberParser.decimal(input), range.contains(value) else {
            invalidFields.insert(field)
            return
        }
        parameters[keyPath: keyPath] = value
        invalidFields.remove(field)
    }
}

struct MacModelParameterDraftForm: View {
    @Binding var draft: MacModelParameterDraft
    var advanced = false
    var samplingAvailable = true
    var samplingRequiresNoReasoning = false

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 12) {
            if advanced {
                Group {
                    decimal(String(localized: "model.parameters.presence_penalty"), key: "presence", path: \.presencePenalty, range: -2...2)
                    decimal(String(localized: "model.parameters.frequency_penalty"), key: "frequency", path: \.frequencyPenalty, range: -2...2)
                }.disabled(!samplingAvailable)
                GridRow {
                    Text(String(localized: "mac.model_parameters.seed"))
                    TextField(String(localized: "mac.model_parameters.optional"), text: Binding(
                        get: { draft.text["seed"] ?? draft.parameters.seed.map(String.init) ?? "" },
                        set: { input in
                            draft.text["seed"] = input
                            let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
                            if trimmed.isEmpty || Int(trimmed) != nil {
                                draft.parameters.seed = Int(trimmed)
                                draft.invalidFields.remove("seed")
                            } else { draft.invalidFields.insert("seed") }
                        }))
                    .foregroundStyle(draft.invalidFields.contains("seed") ? Color.red : Color.primary)
                    .frame(width: 140)
                }
                Text(String(localized: "mac.model_parameters.custom_prompt")).font(.headline).gridCellColumns(2)
                TextEditor(text: value(\.customSystemPrompt)).frame(minHeight: 80).gridCellColumns(2)
                Text(String(localized: "mac.model_parameters.stop_sequences")).font(.headline).gridCellColumns(2)
                TextEditor(text: value(\.stopSequences)).frame(minHeight: 60).gridCellColumns(2)
            } else {
                Text(String(localized: "Generation parameters", table: "ModelCenter")).font(.headline).gridCellColumns(2)
                GridRow {
                Text(String(localized: "Parameter Profile", table: "ModelCenter"))
                Picker(String(localized: "Parameter Profile", table: "ModelCenter"), selection: Binding(
                    get: { MacModelParameterPreset.matching(draft.parameters) },
                    set: { preset in
                        guard preset != .custom else { return }
                        preset.apply(to: &draft.parameters)
                        for key in ["temperature", "topP"] {
                            draft.text.removeValue(forKey: key)
                            draft.invalidFields.remove(key)
                        }
                    })) {
                    ForEach(MacModelParameterPreset.allCases) { Text($0.title).tag($0) }
                }
                .labelsHidden()
                .disabled(!samplingAvailable)
                }
                integer(String(localized: "model.parameters.context"), key: "context", path: \.maxContextLength,
                        range: ModelParameterLimits.remoteContextTokens)
                integer(String(localized: "model.parameters.max_output"), key: "output", path: \.maxOutputTokens,
                        range: ModelParameterLimits.outputTokens)
                Group {
                    decimal(String(localized: "model.parameters.temperature"), key: "temperature", path: \.temperature, range: 0...2)
                    decimal(String(localized: "model.parameters.top_p"), key: "topP", path: \.topP, range: 0...1)
                }.disabled(!samplingAvailable)
                GridRow {
                    Text(String(localized: "mac.model_parameters.stream"))
                    HStack {
                        Spacer(minLength: 0)
                        Toggle(String(localized: "mac.model_parameters.stream"), isOn: value(\.streamOutput))
                            .labelsHidden()
                            .toggleStyle(.macSwitchControl)
                            .accessibilityLabel(String(localized: "mac.model_parameters.stream"))
                    }
                }
                GridRow {
                    Text(String(localized: "mac.model_parameters.thinking"))
                    HStack {
                        Spacer(minLength: 0)
                        Toggle(String(localized: "mac.model_parameters.thinking"), isOn: value(\.showThinking))
                            .labelsHidden()
                            .toggleStyle(.macSwitchControl)
                            .accessibilityLabel(String(localized: "mac.model_parameters.thinking"))
                    }
                }
            }
            if !samplingAvailable {
                Text(String(localized: "Sampling controls are unavailable for this model.", table: "ModelCenter"))
                    .font(.caption).foregroundStyle(.secondary).gridCellColumns(2)
            } else if samplingRequiresNoReasoning {
                Text(String(localized: "Sampling only with reasoning off", table: "ModelCenter"))
                    .font(.caption).foregroundStyle(.secondary).gridCellColumns(2)
            }
        }
        .textFieldStyle(.roundedBorder)
    }

    private func value<T>(_ path: WritableKeyPath<ModelParameters, T>) -> Binding<T> {
        Binding(get: { draft.parameters[keyPath: path] }, set: {
            draft.parameters[keyPath: path] = $0
        })
    }

    private func integer(_ title: String, key: String, path: WritableKeyPath<ModelParameters, Int>, range: ClosedRange<Int>) -> some View {
        GridRow {
            Text(title)
            TextField(title, text: Binding(
                get: { draft.text[key] ?? String(draft.parameters[keyPath: path]) },
                set: { draft.update($0, field: key, integer: path, range: range) }))
            .foregroundStyle(draft.invalidFields.contains(key) ? Color.red : Color.primary)
            .frame(width: 140).monospacedDigit()
            .accessibilityIdentifier("model-parameter-" + key)
            .help("\(range.lowerBound)–\(range.upperBound)")
        }
    }

    private func decimal(_ title: String, key: String, path: WritableKeyPath<ModelParameters, Double>, range: ClosedRange<Double>) -> some View {
        GridRow {
            Text(title)
            TextField(title, text: Binding(
                get: { draft.text[key] ?? String(draft.parameters[keyPath: path]) },
                set: { draft.update($0, field: key, decimal: path, range: range) }))
            .foregroundStyle(draft.invalidFields.contains(key) ? Color.red : Color.primary)
            .frame(width: 140).monospacedDigit()
            .accessibilityIdentifier("model-parameter-" + key)
            .help("\(range.lowerBound)–\(range.upperBound)")
        }
    }
}

nonisolated enum MacModelParameterPreset: String, CaseIterable, Identifiable, Sendable {
    case precise
    case balanced
    case creative
    case custom

    var id: String { rawValue }

    var title: String {
        switch self {
        case .precise: return String(localized: "Precise", table: "ModelCenter")
        case .balanced: return String(localized: "Balanced preset", table: "ModelCenter")
        case .creative: return String(localized: "Creative", table: "ModelCenter")
        case .custom: return String(localized: "Custom", table: "ModelCenter")
        }
    }

    var detail: String {
        switch self {
        case .precise:
            return String(localized: "Stable answers with less variation.", table: "ModelCenter")
        case .balanced:
            return String(localized: "A practical default for most conversations.", table: "ModelCenter")
        case .creative:
            return String(localized: "More variety for drafting and exploration.", table: "ModelCenter")
        case .custom:
            return String(localized: "Keep the values configured below.", table: "ModelCenter")
        }
    }

    func apply(to parameters: inout ModelParameters) {
        switch self {
        case .precise:
            parameters.temperature = 0.2
            parameters.topP = 0.8
        case .balanced:
            parameters.temperature = 0.7
            parameters.topP = 0.9
        case .creative:
            parameters.temperature = 1.0
            parameters.topP = 0.95
        case .custom:
            break
        }
    }

    static func matching(_ parameters: ModelParameters) -> Self {
        let pairs: [(Self, Double, Double)] = [
            (.precise, 0.2, 0.8),
            (.balanced, 0.7, 0.9),
            (.creative, 1.0, 0.95),
        ]
        return pairs.first {
            abs(parameters.temperature - $0.1) < 0.001 && abs(parameters.topP - $0.2) < 0.001
        }?.0 ?? .custom
    }
}

nonisolated struct MacModelParametersRoute: Codable, Hashable {
    /// Canonical profile identity used for loading and saving parameters.
    let modelName: String
    let isLocal: Bool
    /// Human-readable label is presentation-only; it must not select a profile.
    let displayName: String?

    init(modelName: String, isLocal: Bool, displayName: String? = nil) {
        self.modelName = modelName
        self.isLocal = isLocal
        self.displayName = displayName
    }

    var displayTitle: String {
        let label = displayName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return label.isEmpty ? modelName : label
    }

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.modelName == rhs.modelName && lhs.isLocal == rhs.isLocal
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(modelName)
        hasher.combine(isLocal)
    }
}

enum MacModelParameterValidation {
    static func message(
        parameters: ModelParameters,
        seedText: String,
        isLocal: Bool,
        contextRange: ClosedRange<Int>
    ) -> String? {
        if !contextRange.contains(parameters.maxContextLength) {
            return rangeMessage(
                field: String(localized: "model.parameters.context"),
                lower: contextRange.lowerBound,
                upper: contextRange.upperBound
            )
        }
        if !ModelParameterLimits.outputTokens.contains(parameters.maxOutputTokens) {
            return rangeMessage(
                field: String(localized: "model.parameters.max_output"),
                lower: 64,
                upper: ModelParameterLimits.outputTokens.upperBound
            )
        }
        if let topK = parameters.topK, topK < 1 {
            return String(
                format: String(localized: "%@ must be at least %@.", table: "ModelCenter"),
                String(localized: "model.parameters.top_k"),
                1.formatted(.number)
            )
        }
        for (field, value, range) in [
            (String(localized: "model.parameters.temperature"), parameters.temperature, -0.0...2.0),
            (String(localized: "model.parameters.top_p"), parameters.topP, 0.0...1.0),
            (String(localized: "model.parameters.presence_penalty"), parameters.presencePenalty, -2.0...2.0),
            (String(localized: "model.parameters.frequency_penalty"), parameters.frequencyPenalty, -2.0...2.0),
        ] where !range.contains(value) {
            return decimalRangeMessage(field: field, range: range)
        }
        if let penalty = parameters.repetitionPenalty, !(0.8...1.4).contains(penalty) {
            return decimalRangeMessage(
                field: String(localized: "model.parameters.repetition_penalty"),
                range: 0.8...1.4
            )
        }
        let normalizedSeed = seedText.trimmingCharacters(in: .whitespacesAndNewlines)
        if !normalizedSeed.isEmpty, Int(normalizedSeed) == nil {
            return String(localized: "Seed must be a whole number.", table: "ModelCenter")
        }
        return nil
    }

    private static func rangeMessage(field: String, lower: Int, upper: Int) -> String {
        String(
            format: String(localized: "%@ must be between %@ and %@.", table: "ModelCenter"),
            field,
            lower.formatted(.number),
            upper.formatted(.number)
        )
    }

    private static func decimalRangeMessage(field: String, range: ClosedRange<Double>) -> String {
        String(
            format: String(localized: "%@ must be between %@ and %@.", table: "ModelCenter"),
            field,
            range.lowerBound.formatted(.number),
            range.upperBound.formatted(.number)
        )
    }
}

struct MacModelParametersView: View {
    private nonisolated struct RemoteParameterSnapshot: Sendable {
        let parameters: ModelParameters
        let profile: MacCanonicalModelProfile
        let revision: UInt64
    }

    private nonisolated enum LoadedSnapshot: Sendable {
        case local(MacLocalModelConfigurationStore.ParameterSnapshot)
        case remote(RemoteParameterSnapshot)
    }

    private nonisolated static func loadSnapshotOffMainActor(route: MacModelParametersRoute) throws -> LoadedSnapshot {
        if route.isLocal, let profile = MacLocalModelProfile.resolve(route.modelName) {
            return .local(try MacLocalModelConfigurationStore.loadParameterSnapshot(for: profile))
        }
        let port = MacApplicationPort.shared
        let profile: MacCanonicalModelProfile?
        if let exact = try port.modelProfile(route.modelName) {
            profile = exact
        } else {
            let matches = try port.listModelProfiles().filter { $0.modelName == route.modelName && $0.deletedAtMs == nil }
            guard matches.count == 1 else { throw MacApplicationServiceError.invalidResponse }
            profile = matches.first
        }
        guard let profile, profile.deletedAtMs == nil, profile.revision >= 0,
              let data = profile.parametersJSON.data(using: .utf8) else {
            throw MacApplicationServiceError.invalidResponse
        }
        return .remote(RemoteParameterSnapshot(
            parameters: try JSONDecoder().decode(ModelParameters.self, from: data),
            profile: profile,
            revision: UInt64(profile.revision)
        ))
    }

    private nonisolated static func saveRemoteParametersOffMainActor(
        _ params: ModelParameters,
        snapshot: RemoteParameterSnapshot
    ) throws -> RemoteParameterSnapshot {
        let profile = snapshot.profile
        let input = MacCanonicalModelProfile(
            id: profile.id,
            displayName: profile.displayName,
            provider: profile.provider,
            modelName: profile.modelName,
            runtime: profile.runtime,
            sourceJSON: profile.sourceJSON,
            parametersJSON: String(decoding: try JSONEncoder().encode(params), as: UTF8.self),
            selected: false,
            enabled: true,
            revision: 0,
            createdAtMs: profile.createdAtMs,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000),
            deletedAtMs: nil
        )
        let revision = try MacApplicationPort.shared.configureModelParameters(input, expectedRevision: snapshot.revision)
        return RemoteParameterSnapshot(parameters: params, profile: input, revision: revision)
    }

    let route: MacModelParametersRoute

    @Environment(\.dismiss) private var dismiss
    @State private var parameters: ModelParameters
    @State private var seedText: String
    @State private var savedParameters: ModelParameters
    @State private var savedSeedText: String
    @State private var saved = false
    @State private var isLoading = true
    @State private var isSaving = false
    @State private var loadGeneration = 0
    @State private var feedbackID = UUID()
    @State private var showsDiscardConfirmation = false
    @State private var reloadAfterDiscard = false
    @State private var allowsDiscardClose = false
    @State private var saveError: String?
    @State private var localRevision: UInt64?
    @State private var remoteSnapshot: RemoteParameterSnapshot?
    @State private var selectedPreset: MacModelParameterPreset
    @State private var numericText: [String: String] = [:]
    @State private var invalidNumericFields: Set<String> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(route: MacModelParametersRoute) {
        self.route = route
        let defaults = ModelParameters.defaults(isLocal: route.isLocal)
        _parameters = State(initialValue: defaults)
        _seedText = State(initialValue: defaults.seed.map(String.init) ?? "")
        _savedParameters = State(initialValue: defaults)
        _savedSeedText = State(initialValue: defaults.seed.map(String.init) ?? "")
        _selectedPreset = State(initialValue: MacModelParameterPreset.matching(defaults))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.55)

            if let saveError {
                MacInlineNotice(
                    title: String(localized: "Could Not Save Parameters", table: "ModelCenter"),
                    message: saveError,
                    kind: .failure,
                    actionTitle: String(localized: "Reload"),
                    action: requestReload
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.top, MacDesignTokens.Spacing.md)
                .transition(.opacity.combined(with: .move(edge: .top)))
            } else if let validationMessage {
                MacInlineNotice(
                    title: String(localized: "Review Parameters", table: "ModelCenter"),
                    message: validationMessage,
                    kind: .warning
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.top, MacDesignTokens.Spacing.md)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    presetCard
                    limitsCard
                    samplingCard
                    behaviorCard
                    outputCard
                }
                .frame(maxWidth: 720)
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.xl)
                .frame(maxWidth: .infinity)
            }
            .disabled(isLoading || isSaving || (localRevision == nil && remoteSnapshot == nil))
        }
        .background(MacPageBackground())
        .task(id: route) { await loadParameters() }
        .background(MacEditorWindowCloseGuard(
            shouldPreventClose: { hasUnsavedChanges && !allowsDiscardClose },
            onCloseAttempt: requestDismiss
        ).frame(width: 0, height: 0))
        .confirmationDialog(
            String(localized: "Discard unsaved changes?", table: "ModelCenter"),
            isPresented: $showsDiscardConfirmation
        ) {
            Button(String(localized: "Discard Changes", table: "ModelCenter"), role: .destructive) {
                if reloadAfterDiscard { reloadParameters() } else {
                    allowsDiscardClose = true
                    dismiss()
                }
                reloadAfterDiscard = false
            }
            Button(String(localized: "Keep Editing", table: "ModelCenter"), role: .cancel) { reloadAfterDiscard = false }
        } message: {
            Text(String(localized: "Your model parameters have not been saved.", table: "ModelCenter"))
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: saveError)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: validationMessage)
    }

    private var header: some View {
        HStack(spacing: 13) {
            MacIconTile(
                symbol: route.isLocal ? "cpu.fill" : "cloud.fill",
                tint: route.isLocal ? Color.green : MacDesignColor.brand,
                size: MacDesignTokens.Control.featureIcon
            )
            VStack(alignment: .leading, spacing: 3) {
                Text(route.displayTitle)
                    .font(.system(size: 21, weight: .semibold, design: .rounded))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(route.displayTitle)
                Text(route.isLocal
                    ? String(localized: "Local MLX inference parameters", table: "ModelCenter")
                    : String(localized: "API request parameters", table: "ModelCenter"))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            Spacer()
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { headerActions }
                VStack(alignment: .trailing, spacing: 7) { headerActions }
            }
        }
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .padding(.vertical, MacDesignTokens.Spacing.lg)
        .background(MacDesignColor.chrome.opacity(0.96))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: saved)
    }

    @ViewBuilder
    private var headerActions: some View {
        parameterStatus
        Button(String(localized: "mac.model_parameters.reset")) { reset() }
            .disabled(isLoading || isSaving || (localRevision == nil && remoteSnapshot == nil))
        Button(String(localized: "mac.model_parameters.save")) { save() }
            .buttonStyle(.nolaProminent)
            .keyboardShortcut("s", modifiers: .command)
            .disabled(isLoading || isSaving || !hasUnsavedChanges || validationMessage != nil || (localRevision == nil && remoteSnapshot == nil))
        Button(String(localized: "mac.model_parameters.done")) { requestDismiss() }
            .keyboardShortcut(.cancelAction)
    }

    private var parameterStatus: some View {
        ZStack(alignment: .trailing) {
            Label(String(localized: "mac.model_parameters.saved"), systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .opacity(saved ? 1 : 0)
            Label(String(localized: "mac.model_parameters.unsaved"), systemImage: "circle.fill")
                .foregroundStyle(.orange)
                .opacity(hasUnsavedChanges && !saved ? 1 : 0)
        }
        .font(.caption.weight(.medium))
        .fixedSize(horizontal: true, vertical: false)
        .accessibilityHidden(!saved && !hasUnsavedChanges)
    }

    private var limitsCard: some View {
        parameterCard(title: String(localized: "model.parameters.limits"), symbol: "text.alignleft") {
            integerRow(
                String(localized: "model.parameters.context"),
                value: $parameters.maxContextLength,
                minimum: contextRange.lowerBound,
                maximum: contextRange.upperBound,
                unit: String(localized: "mac.model_parameters.unit.tokens")
            )
            Divider().opacity(0.45)
            integerRow(
                String(localized: "model.parameters.max_output"),
                value: $parameters.maxOutputTokens,
                minimum: 64,
                maximum: ModelParameterLimits.outputTokens.upperBound,
                unit: String(localized: "mac.model_parameters.unit.tokens")
            )
        }
    }

    private var presetCard: some View {
        parameterCard(
            title: String(localized: "Parameter Profile", table: "ModelCenter"),
            symbol: "dial.medium"
        ) {
            Picker(
                String(localized: "Parameter Profile", table: "ModelCenter"),
                selection: Binding(get: { selectedPreset }, set: { preset in
                    selectedPreset = preset
                    preset.apply(to: &parameters)
                    if preset != .custom {
                        for title in [String(localized: "model.parameters.temperature"), String(localized: "model.parameters.top_p")] {
                            numericText.removeValue(forKey: title)
                            invalidNumericFields.remove(title)
                        }
                    }
                    saved = false
                })
            ) {
                ForEach(MacModelParameterPreset.allCases) { preset in
                    Text(preset.title).tag(preset)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            Text(selectedPreset.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var samplingCard: some View {
        parameterCard(title: String(localized: "model.parameters.sampling"), symbol: "slider.horizontal.3") {
            decimalRow(
                String(localized: "model.parameters.temperature"),
                value: presetTrackingBinding($parameters.temperature),
                range: 0...2
            )
            Divider().opacity(0.45)
            decimalRow(
                String(localized: "model.parameters.top_p"),
                value: presetTrackingBinding($parameters.topP),
                range: 0...1
            )
            if route.isLocal {
                Divider().opacity(0.45)
                integerRow(String(localized: "model.parameters.top_k"), value: Binding(get: { LocalSamplingSettings(parameters: parameters).topK }, set: { parameters.topK = $0 }), minimum: 1, unit: String(localized: "mac.model_parameters.unit.candidates"))
            }
        }
    }

    private var behaviorCard: some View {
        parameterCard(title: String(localized: "model.parameters.behavior"), symbol: "dial.low") {
            decimalRow(String(localized: "model.parameters.presence_penalty"), value: $parameters.presencePenalty, range: -2...2)
            Divider().opacity(0.45)
            decimalRow(String(localized: "model.parameters.frequency_penalty"), value: $parameters.frequencyPenalty, range: -2...2)
            if route.isLocal {
                Divider().opacity(0.45)
                decimalRow(String(localized: "model.parameters.repetition_penalty"), value: Binding(get: { LocalSamplingSettings(parameters: parameters).repetitionPenalty }, set: { parameters.repetitionPenalty = $0 }), range: 0.8...1.4)
            }
            Divider().opacity(0.45)
            responsiveParameterRow(
                title: String(localized: "mac.model_parameters.seed"),
                detail: String(localized: "mac.model_parameters.seed_hint")
            ) {
                TextField(String(localized: "mac.model_parameters.optional"), text: $seedText)
                    .textFieldStyle(.roundedBorder)
                    .multilineTextAlignment(.trailing)
            }
        }
    }

    private var outputCard: some View {
        parameterCard(title: String(localized: "model.parameters.output"), symbol: "text.bubble") {
            if route.isLocal {
                Label(String(localized: "Parameters apply the next time the model starts.", table: "ModelCenter"), systemImage: "power")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Divider().opacity(0.45)
            }
            Toggle(String(localized: "mac.model_parameters.stream"), isOn: $parameters.streamOutput)
                .toggleStyle(.macSwitch)
            Toggle(String(localized: "mac.model_parameters.thinking"), isOn: $parameters.showThinking)
                .toggleStyle(.macSwitch)
            Divider().opacity(0.45)
            VStack(alignment: .leading, spacing: 7) {
                Text(String(localized: "mac.model_parameters.custom_prompt"))
                    .font(.subheadline.weight(.medium))
                TextEditor(text: $parameters.customSystemPrompt)
                    .font(.body)
                    .frame(minHeight: 92)
                    .padding(7)
                    .scrollContentBackground(.hidden)
                    .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 9))
                    .overlay { RoundedRectangle(cornerRadius: 9).stroke(Color.primary.opacity(0.08), lineWidth: 0.5) }
            }
            Divider().opacity(0.45)
            VStack(alignment: .leading, spacing: 7) {
                Text(String(localized: "mac.model_parameters.stop_sequences"))
                    .font(.subheadline.weight(.medium))
                Text(String(localized: "mac.model_parameters.stop_hint"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: $parameters.stopSequences)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 70)
                    .padding(7)
                    .scrollContentBackground(.hidden)
                    .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 9))
                    .overlay { RoundedRectangle(cornerRadius: 9).stroke(Color.primary.opacity(0.08), lineWidth: 0.5) }
            }
        }
    }

    private func parameterCard<Content: View>(
        title: String,
        symbol: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: symbol)
                .font(.headline)
            content()
        }
        .padding(16)
        .macPanel()
    }

    private func integerRow(
        _ title: String,
        value: Binding<Int>,
        minimum: Int,
        maximum: Int? = nil,
        unit: String
    ) -> some View {
        responsiveParameterRow(
            title: title,
            detail: maximum.map { "\(minimum.formatted(.number))–\($0.formatted(.number)) \(unit)" }
                ?? String(
                    format: String(localized: "mac.model_parameters.minimum_format"),
                    minimum,
                    unit
                )
        ) {
            TextField(String(localized: "mac.model_parameters.value"), text: Binding(get: { numericText[title] ?? String(value.wrappedValue) }, set: { text in
                numericText[title] = text
                if let parsed = ModelParameterNumberParser.integer(text), parsed >= minimum, maximum.map({ parsed <= $0 }) ?? true {
                    value.wrappedValue = parsed
                    invalidNumericFields.remove(title)
                } else { invalidNumericFields.insert(title) }
            }))
                .foregroundStyle(invalidNumericFields.contains(title) ? Color.red : Color.primary)
                .textFieldStyle(.roundedBorder)
                .multilineTextAlignment(.trailing)
                .monospacedDigit()
        }
    }

    private func decimalRow(
        _ title: String,
        value: Binding<Double>,
        range: ClosedRange<Double>
    ) -> some View {
        responsiveParameterRow(
            title: title,
            detail: "\(range.lowerBound.formatted())–\(range.upperBound.formatted())"
        ) {
            TextField(String(localized: "mac.model_parameters.value"), text: Binding(get: { numericText[title] ?? String(value.wrappedValue) }, set: { text in
                numericText[title] = text
                if let parsed = ModelParameterNumberParser.decimal(text), range.contains(parsed) {
                    value.wrappedValue = parsed
                    invalidNumericFields.remove(title)
                } else { invalidNumericFields.insert(title) }
            }))
                .foregroundStyle(invalidNumericFields.contains(title) ? Color.red : Color.primary)
                .textFieldStyle(.roundedBorder)
                .multilineTextAlignment(.trailing)
                .monospacedDigit()
        }
    }

    private func presetTrackingBinding(_ value: Binding<Double>) -> Binding<Double> {
        Binding(
            get: { value.wrappedValue },
            set: { newValue in
                value.wrappedValue = newValue
                selectedPreset = MacModelParameterPreset.matching(parameters)
            }
        )
    }

    private func responsiveParameterRow<Control: View>(
        title: String,
        detail: String,
        @ViewBuilder control: () -> Control
    ) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 20) {
                parameterDescription(title: title, detail: detail)
                Spacer(minLength: 12)
                control()
                    .frame(minWidth: 110, idealWidth: 150, maxWidth: 170)
            }
            VStack(alignment: .leading, spacing: 10) {
                parameterDescription(title: title, detail: detail)
                control()
                    .frame(maxWidth: .infinity)
            }
        }
    }

    private func parameterDescription(title: String, detail: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.subheadline.weight(.medium))
            Text(detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func save() {
        guard !isSaving, !isLoading, validationMessage == nil else { return }
        var normalized = parameters
        normalized.seed = Int(seedText.trimmingCharacters(in: .whitespacesAndNewlines))
        normalized.customSystemPrompt = normalized.customSystemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let capturedSeedText = seedText
        let route = self.route
        let expectedLocalRevision = localRevision
        let capturedRemoteSnapshot = remoteSnapshot
        isSaving = true
        saveError = nil
        Task { @MainActor in
            defer { isSaving = false }
            do {
                if route.isLocal, let profile = MacLocalModelProfile.resolve(route.modelName) {
                    guard let expectedLocalRevision else {
                        throw MacApplicationServiceError.invalidResponse
                    }
                    let nextRevision = try await Task.detached(priority: .userInitiated) {
                        try MacLocalModelConfigurationStore.persistParameters(
                            normalized,
                            for: profile,
                            expectedRevision: expectedLocalRevision
                        )
                    }.value
                    self.localRevision = nextRevision
                } else {
                    guard let capturedRemoteSnapshot else {
                        throw MacApplicationServiceError.invalidResponse
                    }
                    let nextSnapshot = try await Task.detached(priority: .userInitiated) {
                        try Self.saveRemoteParametersOffMainActor(normalized, snapshot: capturedRemoteSnapshot)
                    }.value
                    self.remoteSnapshot = nextSnapshot
                }
                parameters = normalized
                savedParameters = normalized
                numericText = [:]
                invalidNumericFields = []
                seedText = capturedSeedText
                savedSeedText = capturedSeedText
                NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
                saved = true
                feedbackID = UUID()
                let feedback = feedbackID
                Task {
                    do { try await Task.sleep(for: .seconds(1.4)) } catch { return }
                    guard feedbackID == feedback else { return }
                    saved = false
                }
            } catch {
                saveError = error.localizedDescription
            }
        }
    }

    private func reset() {
        numericText = [:]
        invalidNumericFields = []
        parameters = ModelParameters.defaults(isLocal: route.isLocal)
        seedText = ""
        saved = false
        selectedPreset = MacModelParameterPreset.matching(parameters)
    }

    private var hasUnsavedChanges: Bool {
        !numericText.isEmpty || parameters.maxContextLength != savedParameters.maxContextLength
            || parameters.maxOutputTokens != savedParameters.maxOutputTokens
            || parameters.temperature != savedParameters.temperature
            || parameters.topP != savedParameters.topP
            || parameters.topK != savedParameters.topK
            || parameters.presencePenalty != savedParameters.presencePenalty
            || parameters.frequencyPenalty != savedParameters.frequencyPenalty
            || parameters.repetitionPenalty != savedParameters.repetitionPenalty
            || parameters.streamOutput != savedParameters.streamOutput
            || parameters.showThinking != savedParameters.showThinking
            || parameters.customSystemPrompt != savedParameters.customSystemPrompt
            || parameters.stopSequences != savedParameters.stopSequences
            || seedText != savedSeedText
    }

    private var contextRange: ClosedRange<Int> {
        if route.isLocal, let profile = MacLocalModelProfile.resolve(route.modelName) { return profile.configurableContext }
        return route.isLocal ? 65_536...2_000_000 : 512...ModelParameterLimits.remoteContextTokens.upperBound
    }

    private var validationMessage: String? {
        if !invalidNumericFields.isEmpty { return NSLocalizedString("model.parameters.invalid_values", value: "Check the displayed parameter ranges. Seed must be a whole number.", comment: "Invalid model parameters") }
        return MacModelParameterValidation.message(
            parameters: parameters,
            seedText: seedText,
            isLocal: route.isLocal,
            contextRange: contextRange
        )
    }

    private func requestDismiss() {
        reloadAfterDiscard = false
        if hasUnsavedChanges {
            showsDiscardConfirmation = true
        } else {
            dismiss()
        }
    }

    private func requestReload() {
        if hasUnsavedChanges {
            reloadAfterDiscard = true
            showsDiscardConfirmation = true
        } else { reloadParameters() }
    }

    private func reloadParameters() {
        Task { @MainActor in
            await loadParameters()
        }
    }

    private func loadParameters() async {
        loadGeneration &+= 1
        let generation = loadGeneration
        let route = self.route
        isLoading = true
        defer {
            if loadGeneration == generation {
                isLoading = false
            }
        }
        do {
            let loaded = try await Task.detached(priority: .userInitiated) {
                try Self.loadSnapshotOffMainActor(route: route)
            }.value
            guard !Task.isCancelled, loadGeneration == generation else { return }
            let refreshed: ModelParameters
            switch loaded {
            case .local(let snapshot):
                refreshed = snapshot.parameters
                localRevision = snapshot.revision
                remoteSnapshot = nil
            case .remote(let snapshot):
                refreshed = snapshot.parameters
                remoteSnapshot = snapshot
                localRevision = nil
            }
            parameters = refreshed
            savedParameters = refreshed
            seedText = refreshed.seed.map(String.init) ?? ""
            savedSeedText = seedText
            numericText = [:]
            invalidNumericFields = []
            selectedPreset = MacModelParameterPreset.matching(refreshed)
            saveError = nil
            saved = false
        } catch {
            guard !Task.isCancelled, loadGeneration == generation else { return }
            // Keep the entire draft and original revision when a read fails.
            saveError = error.localizedDescription
        }
    }
}
