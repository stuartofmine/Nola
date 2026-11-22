// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// The model center keeps this draft alive when its pager releases an offscreen form.
struct IOSModelSetupDraft {
    var endpoint = ""
    var secret = ""
    var modelID = ""
    var name = ""
    var wire = "chat_completions"
    var savedModel: ModelInfo?

    var hasChanges: Bool {
        !endpoint.isEmpty || !secret.isEmpty || !modelID.isEmpty || !name.isEmpty
            || wire != "chat_completions" || savedModel != nil
    }
}

struct IOSModelSetupForm: View {
    let service: IOSModelConnectionService
    @Binding var draft: IOSModelSetupDraft
    let onSaved: (ModelInfo) -> Void
    private enum Field: Hashable { case endpoint, key, model, name }
    @FocusState private var focusedField: Field?
    @State private var showsAdvanced = false
    @State private var error: String?
    @State private var busy = false
    @State private var discoveredModels: [AppleModelCatalogDiscovery.Model] = []
    @State private var discoveryTask: Task<Void, Never>?
    @State private var discovering = false
    @State private var discoveryAttempt: UUID?
    private var discoveryRoute: [String] { [draft.endpoint, draft.secret, draft.wire, draft.name] }

    private var saveDisabled: Bool {
        busy || draft.endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || draft.modelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private struct ProviderPreset: Identifiable {
        let id: String
        let title: String
        let endpoint: String
        let wire: String
        let defaultName: String
    }

    private var presets: [ProviderPreset] {
        [
            .init(id: "openai", title: "OpenAI", endpoint: "https://api.openai.com/v1", wire: "responses", defaultName: "OpenAI"),
            .init(id: "anthropic", title: "Anthropic", endpoint: "https://api.anthropic.com", wire: "anthropic", defaultName: "Anthropic"),
            .init(id: "deepseek", title: "DeepSeek", endpoint: "https://api.deepseek.com/v1", wire: "chat_completions", defaultName: "DeepSeek"),
            .init(id: "openrouter", title: "OpenRouter", endpoint: "https://openrouter.ai/api/v1", wire: "chat_completions", defaultName: "OpenRouter"),
            .init(id: "ollama", title: "Ollama", endpoint: "http://127.0.0.1:11434/v1", wire: "chat_completions", defaultName: "Ollama"),
            .init(id: "custom", title: NSLocalizedString("model.setup.preset.custom", value: "Custom", comment: "Custom provider preset"), endpoint: "", wire: "chat_completions", defaultName: "")
        ]
    }

    private func isPresetSelected(_ preset: ProviderPreset) -> Bool {
        let trimmed = draft.endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
        if preset.id == "custom" {
            return !trimmed.isEmpty && !presets.dropLast().contains(where: { $0.endpoint.caseInsensitiveCompare(trimmed) == .orderedSame })
        }
        return preset.endpoint.caseInsensitiveCompare(trimmed) == .orderedSame
    }

    private func applyPreset(_ preset: ProviderPreset) {
        error = nil
        if preset.id == "custom" {
            if presets.dropLast().contains(where: { $0.endpoint.caseInsensitiveCompare(draft.endpoint.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame }) {
                draft.endpoint = ""
                draft.name = ""
            }
            draft.wire = preset.wire
            focusedField = .endpoint
        } else {
            draft.endpoint = preset.endpoint
            draft.wire = preset.wire
            if draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                || presets.dropLast().contains(where: { $0.defaultName == draft.name }) {
                draft.name = preset.defaultName
            }
            focusedField = preset.id == "ollama" ? .model : .key
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 8) {
                Text(NSLocalizedString("model.setup.presets", value: "Provider presets", comment: "Provider presets heading"))
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.primary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(presets) { preset in
                            let selected = isPresetSelected(preset)
                            Button {
                                applyPreset(preset)
                            } label: {
                                Text(preset.title)
                                    .font(.footnote.weight(selected ? .semibold : .medium))
                                    .foregroundStyle(selected ? DesignTokens.actionColor : DesignTokens.contentColor)
                                    .padding(.horizontal, 14)
                                    .frame(minHeight: 36)
                                    .background(
                                        selected ? DesignTokens.accentSubtleColor : DesignTokens.surfaceColor,
                                        in: Capsule()
                                    )
                                    .overlay(
                                        Capsule().stroke(
                                            selected ? DesignTokens.actionColor : DesignTokens.borderColor,
                                            lineWidth: selected ? 1.25 : 1
                                        )
                                    )
                                    .frame(minHeight: 44)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .disabled(busy || draft.savedModel != nil)
                            .accessibilityIdentifier("model.setup.preset.\(preset.id)")
                            .accessibilityAddTraits(selected ? .isSelected : [])
                        }
                    }
                    .padding(.vertical, 2)
                }
            }

            field(NSLocalizedString("model.connections.endpoint", comment: "API address")) {
                TextField(text: $draft.endpoint, prompt: Text(verbatim: "https://api.example.com/v1")) {
                    Text(NSLocalizedString("model.connections.endpoint", comment: "API endpoint"))
                }
                    .keyboardType(.URL).focused($focusedField, equals: .endpoint)
                    .submitLabel(.next).onSubmit { focusedField = .key }
                    .accessibilityIdentifier("model.setup.endpoint")
            }
            field(NSLocalizedString("model.setup.api_key", value: "API key", comment: "API key")) {
                SecureField(NSLocalizedString("model.setup.key_hint", value: "As required by your service", comment: "API key hint"), text: $draft.secret)
                    .focused($focusedField, equals: .key)
                    .submitLabel(.next).onSubmit { focusedField = .model }
                    .accessibilityIdentifier("model.setup.key")
            }
            field(NSLocalizedString("model.setup.model_id", value: "Default model ID", comment: "Default model ID")) {
                TextField(NSLocalizedString("model.setup.model_hint", value: "Model ID from your service", comment: "Model ID hint"), text: $draft.modelID)
                    .focused($focusedField, equals: .model)
                    .submitLabel(.done).onSubmit { focusedField = nil }
                    .accessibilityIdentifier("model.setup.modelID")
            }
            HStack {
                Button(action: discover) {
                    Label(NSLocalizedString("model.setup.discover", value: "Discover models", comment: "Discover models"), systemImage: "magnifyingglass")
                        .frame(minHeight: 44)
                }.accessibilityIdentifier("model.setup.discover")
                .disabled(discovering || draft.endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy || draft.savedModel != nil)
                if discovering {
                    ProgressView()
                    Button(NSLocalizedString("model.connections.cancel", comment: "Cancel discovery")) { cancelDiscovery() }
                        .frame(minHeight: 44)
                }
            }
            if !discoveredModels.isEmpty {
                Picker(NSLocalizedString("model.setup.discovered", value: "Choose a discovered model", comment: "Choose a discovered model"), selection: $draft.modelID) {
                    Text(NSLocalizedString("model.setup.choose", value: "Choose a model", comment: "Choose a model")).tag("")
                    if !draft.modelID.isEmpty && !discoveredModels.contains(where: { $0.id == draft.modelID }) { Text(draft.modelID).tag(draft.modelID) }
                    ForEach(discoveredModels, id: \.id) { Text($0.displayName).tag($0.id) }
                }.accessibilityIdentifier("model.setup.discovered")
            }
            DisclosureGroup(isExpanded: $showsAdvanced) {
                VStack(alignment: .leading, spacing: 14) {
                    field(NSLocalizedString("model.connections.name", comment: "Connection name")) {
                        TextField(NSLocalizedString("model.setup.name_hint", value: "Optional connection name", comment: "Optional connection name"), text: $draft.name)
                            .focused($focusedField, equals: .name)
                            .submitLabel(.done).onSubmit { focusedField = nil }
                    }
                    field(NSLocalizedString("model.connections.protocol", comment: "Protocol")) {
                        Picker(NSLocalizedString("model.connections.protocol", comment: "Protocol"), selection: $draft.wire) {
                            Text(verbatim: "Chat Completions").tag("chat_completions")
                            Text(verbatim: "Responses").tag("responses")
                            Text(verbatim: "Anthropic Messages").tag("anthropic")
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        .accessibilityIdentifier("model.setup.protocol")
                    }
                    Text(NSLocalizedString("model.protocol.guidance", value: "For tools with reasoning, choose Responses if your service supports it. Some Chat Completions models require reasoning to be disabled explicitly.", comment: "API protocol compatibility guidance"))
                        .font(.footnote).foregroundStyle(.secondary)
                }.padding(.top, 8)
            } label: {
                Text(NSLocalizedString("model.setup.advanced", value: "Advanced settings", comment: "Advanced settings"))
                    .frame(minHeight: 44)
            }
            .disclosureGroupStyle(NolaRowDisclosureStyle())
            if let error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled).accessibilityIdentifier("model.setup.error") }
            Button(action: save) {
                HStack {
                    if busy { ProgressView().tint(DesignTokens.onActionColor) }
                    Text(NSLocalizedString("model.setup.save", value: "Save and use", comment: "Save and use model"))
                }.foregroundStyle(saveDisabled ? Color.secondary : DesignTokens.onActionColor)
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.borderedProminent).tint(DesignTokens.actionColor)
            .disabled(saveDisabled)
            .accessibilityIdentifier("model.setup.save")
        }
        .textInputAutocapitalization(.never).autocorrectionDisabled()
        .onChange(of: discoveryRoute) { _, _ in cancelDiscovery(); discoveredModels = [] }
        .onDisappear { cancelDiscovery() }
    }

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.weight(.medium)).foregroundStyle(.primary)
            content().disabled(busy || draft.savedModel != nil).padding(12).frame(minHeight: 44)
                .background(NolaSurfaceBackground(fill: DesignTokens.surfaceColor,
                    border: DesignTokens.strongBorderColor, cornerRadius: 12, raised: false))
        }
    }

    private func cancelDiscovery() {
        discoveryAttempt = nil
        discoveryTask?.cancel(); discoveryTask = nil; discovering = false
    }

    private func discover() {
        focusedField = nil
        guard !discovering, !busy else { return }
        let address = draft.endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URLComponents(string: address), let scheme = url.scheme,
              ["http", "https"].contains(scheme), url.host != nil, url.user == nil,
              url.password == nil, url.query == nil, url.fragment == nil else {
            error = IOSModelConnectionInputError.invalidEndpoint.localizedDescription
            return
        }
        let route = discoveryRoute, token = UUID()
        error = nil; discovering = true; discoveryAttempt = token
        discoveryTask = Task { @MainActor in
            defer {
                if discoveryAttempt == token { discovering = false; discoveryTask = nil; discoveryAttempt = nil }
            }
            do {
                let result = try await service.discover(endpoint: address, wireProtocol: route[2], secret: route[1], name: route[3])
                guard !Task.isCancelled, discoveryAttempt == token, discoveryRoute == route else { return }
                discoveredModels = result
                if result.isEmpty { error = NSLocalizedString("model.setup.discovery_empty", value: "No models were returned. You can enter a model ID manually.", comment: "Empty discovery") }
            } catch {
                guard !Task.isCancelled, discoveryAttempt == token, discoveryRoute == route else { return }
                self.error = error.localizedDescription + " " + NSLocalizedString("model.setup.discovery_manual", value: "You can still enter the model ID manually and save.", comment: "Manual configuration after discovery failure")
            }
        }
    }

    private func save() {
        guard !busy else { return }
        focusedField = nil
        busy = true; error = nil
        defer { busy = false }
        do {
            // If setting the default was rejected, retry that exact saved model instead
            // of creating a second connection with the same entered credentials.
            let model: ModelInfo
            if let savedModel = draft.savedModel { model = savedModel }
            else {
                let connection = try service.create(name: draft.name, endpoint: draft.endpoint, wireProtocol: draft.wire, secret: draft.secret, modelID: draft.modelID,
                    discoveredModel: discoveredModels.first { $0.id == draft.modelID.trimmingCharacters(in: .whitespacesAndNewlines) })
                guard let profile = try service.application.listModelProfiles().first(where: {
                    guard $0.enabled, $0.deletedAtMs == nil,
                          let source = try? JSONSerialization.jsonObject(with: Data($0.sourceJSON.utf8)) as? [String: Any] else { return false }
                    return source["connectionID"] as? String == connection.id && $0.modelName == draft.modelID.trimmingCharacters(in: .whitespacesAndNewlines) && source["wireProtocol"] as? String == draft.wire
                }), let configured = ModelSelectionManager.modelInfo(from: profile) else { throw IOSApplicationRuntimeError.invalidResponse }
                model = configured; draft.savedModel = configured; draft.secret = ""
            }
            try ModelSelectionManager.commitSelectedModel(model, using: service.application)
            draft = IOSModelSetupDraft()
            onSaved(model)
        } catch { self.error = error.localizedDescription }
    }
}

struct IOSModelProtocolEditor: View {
    let model: ModelInfo
    let service: IOSModelConnectionService
    let onSaved: (ModelInfo) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var wire: ModelWireProtocol
    @State private var error: String?
    @State private var saving = false

    init(model: ModelInfo, service: IOSModelConnectionService, onSaved: @escaping (ModelInfo) -> Void) {
        self.model = model; self.service = service; self.onSaved = onSaved
        _wire = State(initialValue: model.wireProtocol ?? .chatCompletions)
    }

    var body: some View {
        Form {
            Section {
                Text(model.effectiveUpstreamModelId).textSelection(.enabled)
                Text(model.baseURL ?? "").font(.footnote).foregroundStyle(.secondary).textSelection(.enabled)
                Picker(NSLocalizedString("model.connections.protocol", comment: "Protocol"), selection: $wire) {
                    Text(verbatim: "Responses").tag(ModelWireProtocol.responses)
                    Text(verbatim: "Chat Completions").tag(ModelWireProtocol.chatCompletions)
                    Text(verbatim: "Anthropic Messages").tag(ModelWireProtocol.anthropic)
                }
                .pickerStyle(.menu)
                .accessibilityIdentifier("model.protocol.selection")
            } footer: {
                Text(NSLocalizedString("model.protocol.guidance", value: "For tools with reasoning, choose Responses if your service supports it. Some Chat Completions models require reasoning to be disabled explicitly.", comment: "API protocol compatibility guidance"))
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            Button(NSLocalizedString("model.setup.save", value: "Save and use", comment: "Save and use model")) {
                saving = true; error = nil
                defer { saving = false }
                do {
                    let configured = try service.configureProtocol(for: model, wireProtocol: wire)
                    try ModelSelectionManager.commitSelectedModel(configured, using: service.application)
                    onSaved(configured)
                    dismiss()
                } catch { self.error = error.localizedDescription }
            }.disabled(saving || wire == model.wireProtocol)
                .frame(minHeight: 44).accessibilityIdentifier("model.protocol.save")
        }
        .nolaGroupedSurface()
        .navigationTitle(NSLocalizedString("model.connections.protocol", comment: "Protocol"))
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button(L10n.close) { dismiss() } } }
    }
}
