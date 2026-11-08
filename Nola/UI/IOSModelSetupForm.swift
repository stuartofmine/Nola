// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// One model setup editor; Core owns enabled models and default selection.
struct IOSModelSetupForm: View {
    let service: IOSModelConnectionService
    let onSaved: (ModelInfo) -> Void
    private enum Field: Hashable { case endpoint, key, model, name }
    @FocusState private var focusedField: Field?
    @State private var endpoint = ""
    @State private var secret = ""
    @State private var modelID = ""
    @State private var name = ""
    @State private var wire = "chat_completions"
    @State private var showsAdvanced = false
    @State private var error: String?
    @State private var busy = false
    @State private var savedModel: ModelInfo?
    @State private var discoveredModels: [AppleModelCatalogDiscovery.Model] = []
    @State private var discoveryTask: Task<Void, Never>?
    @State private var discovering = false
    @State private var discoveryAttempt: UUID?
    private var discoveryRoute: [String] { [endpoint, secret, wire, name] }

    private var saveDisabled: Bool {
        busy || endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || modelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            field(NSLocalizedString("model.connections.endpoint", comment: "API address")) {
                TextField(text: $endpoint, prompt: Text(verbatim: "https://api.example.com/v1")) {
                    Text(NSLocalizedString("model.connections.endpoint", comment: "API endpoint"))
                }
                    .keyboardType(.URL).focused($focusedField, equals: .endpoint)
                    .submitLabel(.next).onSubmit { focusedField = .key }
                    .accessibilityIdentifier("model.setup.endpoint")
            }
            field(NSLocalizedString("model.setup.api_key", value: "API key", comment: "API key")) {
                SecureField(NSLocalizedString("model.setup.key_hint", value: "As required by your service", comment: "API key hint"), text: $secret)
                    .focused($focusedField, equals: .key)
                    .submitLabel(.next).onSubmit { focusedField = .model }
                    .accessibilityIdentifier("model.setup.key")
            }
            field(NSLocalizedString("model.setup.model_id", value: "Default model ID", comment: "Default model ID")) {
                TextField(NSLocalizedString("model.setup.model_hint", value: "Model ID from your service", comment: "Model ID hint"), text: $modelID)
                    .focused($focusedField, equals: .model)
                    .submitLabel(.done).onSubmit { focusedField = nil }
                    .accessibilityIdentifier("model.setup.modelID")
            }
            HStack {
                Button(action: discover) {
                    Label(NSLocalizedString("model.setup.discover", value: "Discover models", comment: "Discover models"), systemImage: "magnifyingglass")
                        .frame(minHeight: 44)
                }.accessibilityIdentifier("model.setup.discover")
                .disabled(discovering || endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy || savedModel != nil)
                if discovering {
                    ProgressView()
                    Button(NSLocalizedString("model.connections.cancel", comment: "Cancel discovery")) { cancelDiscovery() }
                        .frame(minHeight: 44)
                }
            }
            if !discoveredModels.isEmpty {
                Picker(NSLocalizedString("model.setup.discovered", value: "Choose a discovered model", comment: "Choose a discovered model"), selection: $modelID) {
                    Text(NSLocalizedString("model.setup.choose", value: "Choose a model", comment: "Choose a model")).tag("")
                    if !modelID.isEmpty && !discoveredModels.contains(where: { $0.id == modelID }) { Text(modelID).tag(modelID) }
                    ForEach(discoveredModels, id: \.id) { Text($0.displayName).tag($0.id) }
                }.accessibilityIdentifier("model.setup.discovered")
            }
            DisclosureGroup(isExpanded: $showsAdvanced) {
                VStack(alignment: .leading, spacing: 12) {
                    Picker(NSLocalizedString("model.connections.protocol", comment: "Protocol"), selection: $wire) {
                        Text("Chat Completions").tag("chat_completions")
                        Text("Responses").tag("responses")
                        Text("Anthropic Messages").tag("anthropic")
                    }
                    field(NSLocalizedString("model.connections.name", comment: "Connection name")) {
                        TextField(NSLocalizedString("model.setup.name_hint", value: "Optional connection name", comment: "Optional connection name"), text: $name)
                            .focused($focusedField, equals: .name)
                            .submitLabel(.done).onSubmit { focusedField = nil }
                    }
                }.padding(.top, 8)
            } label: {
                Text(NSLocalizedString("model.setup.advanced", value: "Advanced settings", comment: "Advanced settings"))
                    .frame(minHeight: 44)
            }
            Text(NSLocalizedString("model.setup.summary", value: "Save to enable this model and use it by default. No model discovery or test request is required.", comment: "Manual model configuration summary"))
                .font(.footnote).foregroundStyle(.secondary)
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
            content().disabled(busy || savedModel != nil).padding(12).frame(minHeight: 44)
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
        let address = endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
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
            if let savedModel { model = savedModel }
            else {
                let connection = try service.create(name: name, endpoint: endpoint, wireProtocol: wire, secret: secret, modelID: modelID)
                guard let profile = try service.application.listModelProfiles().first(where: {
                    guard $0.enabled, $0.deletedAtMs == nil,
                          let source = try? JSONSerialization.jsonObject(with: Data($0.sourceJSON.utf8)) as? [String: Any] else { return false }
                    return source["connectionID"] as? String == connection.id && $0.modelName == modelID.trimmingCharacters(in: .whitespacesAndNewlines) && source["wireProtocol"] as? String == wire
                }), let configured = ModelSelectionManager.modelInfo(from: profile) else { throw IOSApplicationRuntimeError.invalidResponse }
                model = configured; savedModel = configured; secret = ""
            }
            try ModelSelectionManager.commitSelectedModel(model, using: service.application)
            savedModel = nil; endpoint = ""; modelID = ""; name = ""; secret = ""
            onSaved(model)
        } catch { self.error = error.localizedDescription }
    }
}
