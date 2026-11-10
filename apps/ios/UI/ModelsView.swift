// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelsView logic in the iOS UI layer.
// - Interfaces:
//   - ModelsView
// - APIs:
//   - none
//
import SwiftUI

struct ModelsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var sourceTab = 0
    var ownsNavigationStack = true
    var onConfigured: ((ModelInfo) -> Void)? = nil
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.openURL) private var openURL
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var models: [ModelInfo] = []
    @State private var selectedModel: ModelInfo? = nil
    @State private var detailTarget: ModelInfo? = nil
    @State private var parameterTarget: ModelInfo? = nil
    @State private var capabilityTarget: ModelInfo? = nil
    @State private var modelTestTarget: ModelInfo? = nil
    @State private var showsConfiguredModels = false
    @State private var localRuntime = MLXService.shared
    @State private var runtimeError: String?
    @State private var statusMessage: String?
    @State private var hostAgentSettings: WebAgentProvider.HostAgentSettings?
    @State private var isOpeningHostManagement = false

    private var localModels: [ModelInfo] {
        models.filter(\.isLocal).sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private var cloudModels: [ModelInfo] {
        models.filter { !$0.isLocal && !$0.isWebBackend }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private var hostModels: [ModelInfo] {
        models.filter(\.isWebBackend)
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private var runningLocalModel: LMModel? {
        MLXService.availableModels.first { localRuntime.isRunning($0) }
    }

    var body: some View {
        Group {
            if ownsNavigationStack {
                NavigationStack { content }
            } else {
                content
            }
        }
    }

    private var content: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                sourceTabButton(0, title: NSLocalizedString("model.setup.service", value: "Service address", comment: "Service address tab"))
                sourceTabButton(1, title: NSLocalizedString("model.center.local_models", value: "Local models", comment: "Local models"))
                sourceTabButton(2, title: NSLocalizedString("model.center.host_models", value: "Host", comment: "Host"))
            }
            .padding(.horizontal, 20).padding(.vertical, 8)
            TabView(selection: $sourceTab) {
                apiContent.tag(0)
                ModelDownloadView(embedded: true).environmentObject(container).tag(1)
                WebHostConfigurationView(existingModel: hostModels.first, embedded: true) { configured in
                    ModelSelectionManager.addModel(configured)
                    reloadModels()
                    onConfigured?(configured)
                }.tag(2)
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(DesignTokens.backgroundColor)
        .navigationTitle(L10n.modelsTitle)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(DesignTokens.backgroundColor, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            if ownsNavigationStack && sourceTab == 0 {
                ToolbarItem(placement: .cancellationAction) { Button(L10n.close) { dismiss() } }
            }
        }
    }

    private func sourceTabButton(_ value: Int, title: String) -> some View {
        Button { withAnimation(reduceMotion ? nil : .snappy(duration: 0.22)) { sourceTab = value } } label: {
            Text(title).font(.subheadline.weight(sourceTab == value ? .semibold : .regular))
                .foregroundStyle(sourceTab == value ? Color.primary : Color.secondary)
                .frame(maxWidth: .infinity, minHeight: 44)
                .contentShape(Rectangle())
                .overlay(alignment: .bottom) {
                    if sourceTab == value { Capsule().fill(Color.primary).frame(height: 2) }
                }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("model.setup.source.\(value)")
        .accessibilityAddTraits(sourceTab == value ? .isSelected : [])
    }

    private var apiContent: some View {
            NolaPage {
                IOSModelSetupForm(service: IOSModelConnectionService(application: container.applicationRuntime)) { model in
                    reloadModels()
                    showsConfiguredModels = true
                    statusMessage = NSLocalizedString("model.setup.saved", value: "Saved and set as default · not tested", comment: "Configured model saved")
                    onConfigured?(model)
                }
                if let statusMessage, !statusMessage.isEmpty { statusBanner(statusMessage) }
                if !models.isEmpty {
                    DisclosureGroup(isExpanded: $showsConfiguredModels) {
                modelListSection(title: NSLocalizedString("model.center.local_models", value: "本地模型", comment: "Local models section"), items: localModels)
                modelListSection(title: NSLocalizedString("model.center.cloud_models", value: "云端模型", comment: "Cloud models section"), items: cloudModels)
                modelListSection(title: NSLocalizedString("model.center.host_models", value: "宿主机", comment: "Host models section"), items: hostModels)
                    } label: {
                        Text(NSLocalizedString("model.setup.configured", value: "Configured models", comment: "Configured models"))
                            .font(.headline).frame(minHeight: 44)
                    }
                }

            }
            .navigationTitle(L10n.modelsTitle)
            .onAppear {
                reloadModels()
                Task { await refreshHostAgentSettings() }
            }
            .onReceive(container.chatManager.$selectedModelInfo) { info in
                selectedModel = info
            }
            .sheet(item: $parameterTarget) { model in
                NavigationStack {
                    ModelParametersView(modelID: model.id, displayName: model.name, isLocal: model.isLocal)
                }
            }
            .sheet(item: $capabilityTarget) { model in
                NavigationStack {
                    IOSModelExecutionCapabilitiesView(model: model) { capabilities in
                        let updated = try ModelSelectionManager.updateExecutionCapabilities(for: model, to: capabilities)
                        if isSelected(model) {
                            container.chatManager.selectModel(updated)
                            selectedModel = updated
                        }
                        reloadModels()
                    }
                }
            }
            .sheet(item: $modelTestTarget) { model in
                NavigationStack {
                    IOSModelInferenceTestView(model: model)
                        .environmentObject(container)
                }
            }
            .alert(
                NSLocalizedString("model.center.runtime_error.title", value: "Local model could not start", comment: "Local model runtime error title"),
                isPresented: Binding(
                    get: { runtimeError != nil },
                    set: { if !$0 { runtimeError = nil } }
                )
            ) {
                Button(L10n.ok, role: .cancel) { runtimeError = nil }
            } message: {
                Text(runtimeError ?? "")
            }
    }

    private func hostMessagingCard(_ messaging: WebAgentProvider.HostAgentSettings.Messaging) -> some View {
        let connected = messaging.channels.filter { $0.enabled && $0.state == "connected" }.count
        return NolaCard {
            HStack(alignment: .top, spacing: DesignTokens.spacing_sm) {
                NolaIconBadge(
                    icon: "message.badge.waveform",
                    tint: messaging.worker.enabled ? DesignTokens.successColor : .secondary,
                    size: DesignTokens.iconBadgeSizeSmall
                )
                .accessibilityIdentifier("model.host.messaging.card")
                VStack(alignment: .leading, spacing: 5) {
                    Text(NSLocalizedString("model.host.messaging.title", value: "Messaging channels", comment: "Host messaging channels title"))
                        .font(.subheadline.weight(.semibold))
                    Text(String(format: NSLocalizedString(
                        "model.host.messaging.summary",
                        value: "%lld connected of %lld · background worker %@",
                        comment: "Host messaging channels summary"
                    ), Int64(connected), Int64(messaging.channels.count), messaging.worker.enabled
                        ? NSLocalizedString("model.host.messaging.running", value: "running", comment: "Messaging worker running")
                        : NSLocalizedString("model.host.messaging.stopped", value: "stopped", comment: "Messaging worker stopped")))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(NSLocalizedString(
                        "model.host.messaging.providers",
                        value: "Telegram · WhatsApp · WeChat Official Account · WeCom",
                        comment: "Supported Host messaging providers"
                    ))
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(DesignTokens.primaryColor)
                }
                Spacer(minLength: 0)
                if let model = hostModels.first,
                   resolvedHostConnection(for: model) != nil,
                   messaging.management_path != nil {
                    Button(NSLocalizedString("model.host.messaging.manage", value: "Manage on Host", comment: "Manage messaging channels on Host")) {
                        Task { await openHostMessagingManagement(messaging, model: model) }
                    }
                    .accessibilityIdentifier("model.host.messaging.manage")
                    .buttonStyle(.bordered)
                    .disabled(isOpeningHostManagement)
                    .overlay {
                        if isOpeningHostManagement { ProgressView().controlSize(.small) }
                    }
                }
            }
        }
    }

    @MainActor
    private func openHostMessagingManagement(
        _ messaging: WebAgentProvider.HostAgentSettings.Messaging,
        model: ModelInfo
    ) async {
        guard let connection = resolvedHostConnection(for: model) else { return }
        isOpeningHostManagement = true
        defer { isOpeningHostManagement = false }
        do {
            let url = try await WebAgentProvider.requestWebManagementURL(
                baseURL: connection.baseURL,
                token: connection.token,
                target: messaging.management_path ?? "/settings#channels",
                pinnedCertificateSHA256: model.tlsCertificateSHA256
            )
            openURL(url)
        } catch {
            statusMessage = error.localizedDescription
        }
    }

    @MainActor
    private func refreshHostAgentSettings() async {
        guard let model = hostModels.first,
              let connection = resolvedHostConnection(for: model) else {
            hostAgentSettings = nil
            return
        }
        do {
            let settings = try await WebAgentProvider.fetchHostAgentSettings(
                baseURL: connection.baseURL,
                token: connection.token,
                pinnedCertificateSHA256: model.tlsCertificateSHA256
            )
            guard settings.host_identity?.id == connection.hostID else {
                hostAgentSettings = nil
                statusMessage = NSLocalizedString(
                    "model.host.identity_changed",
                    value: "The connected host identity changed. Forget the existing pairing before reconnecting.",
                    comment: "Host identity mismatch"
                )
                return
            }
            hostAgentSettings = settings
        } catch {
            hostAgentSettings = nil
        }
    }

    private func resolvedHostConnection(for model: ModelInfo) -> (baseURL: URL, token: String, hostID: String)? {
        let baseURLText = (model.baseURL ?? container.baseURLString)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let token = (model.resolvedAPIKey ?? container.apiKey)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty,
              let baseURL = URL(string: baseURLText),
              let hostID = ModelSelectionManager.pairedWebHostIdentity(for: baseURL) else {
            return nil
        }
        return (baseURL, token, hostID)
    }

    private func modelListSection(title: String, items: [ModelInfo]) -> some View {
        Group {
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                    NolaSectionTitle(icon: "list.bullet.rectangle", title: title)
                    VStack(spacing: 0) {
                        ForEach(items) { model in
                            modelRow(model)
                            if model.id != items.last?.id || model.isLocal != items.last?.isLocal {
                                NolaDivider(leading: 24)
                            }
                        }
                    }
                }
            }
        }
    }

    private func modelRow(_ model: ModelInfo) -> some View {
        DisclosureGroup(isExpanded: Binding(
            get: { detailTarget?.id == model.id },
            set: { detailTarget = $0 ? model : nil }
        )) {
            modelDetailContent(model)
        } label: {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(model.name).font(.body).foregroundStyle(.primary)
                    Text(modelDetail(model)).font(.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 8)
                if isSelected(model) { Image(systemName: "checkmark").foregroundStyle(.primary) }
            }
            .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func modelDetailContent(_ model: ModelInfo) -> some View {
        VStack(alignment: .leading, spacing: DesignTokens.spacing_xs) {
            HStack(spacing: 6) {
                if isSelected(model) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(DesignTokens.primaryColor)
                    Text(NSLocalizedString("model.center.selected", value: "Selected", comment: "Selected model badge"))
                }
                if model.isLocal {
                    Image(systemName: "arrow.down.circle.fill")
                    Text(NSLocalizedString("model.center.downloaded", value: "Downloaded", comment: "Downloaded local model badge"))
                }
                if model.isLocal,
                   let runtimeModel = resolveLocalModel(model) {
                    if localRuntime.isRunning(runtimeModel) {
                        Image(systemName: "bolt.fill").foregroundStyle(DesignTokens.successColor)
                        Text(NSLocalizedString("model.center.running", value: "Running", comment: "Running local model"))
                    } else if localRuntime.loadingModelName == runtimeModel.name {
                        ProgressView().controlSize(.small)
                        Text(NSLocalizedString("model.center.starting", value: "Starting", comment: "Starting local model"))
                    }
                }
            }
            .font(.caption2.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.leading, 28)

            LazyVGrid(columns: modelActionColumns, alignment: .leading, spacing: 4) {
                if model.isLocal,
                   let runtimeModel = resolveLocalModel(model) {
                    if localRuntime.isRunning(runtimeModel) {
                        Button { localRuntime.stop() } label: {
                            Label(NSLocalizedString("model.center.stop", value: "Stop", comment: "Stop local model"), systemImage: "stop.fill")
                                .font(.caption.weight(.semibold))
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        }
                        .buttonStyle(.bordered)
                        .tint(.secondary)
                    } else if localRuntime.loadingModelName != runtimeModel.name && isSelected(model) {
                        Button { startLocalModel(runtimeModel) } label: {
                            Label(NSLocalizedString("model.center.start", value: "Start", comment: "Start local model"), systemImage: "play.fill")
                                .font(.caption.weight(.semibold))
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(DesignTokens.actionColor)
                        .foregroundStyle(DesignTokens.onActionColor)
                        .buttonBorderShape(.roundedRectangle(radius: 12))
                        .disabled(runningLocalModel != nil)
                    }
                }

                if !isSelected(model) {
                    Button { select(model) } label: {
                        Label(NSLocalizedString("model.center.use", value: "使用", comment: "Use model button"), systemImage: "checkmark.circle")
                            .font(.caption.weight(.semibold))
                            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(DesignTokens.actionColor)
                    .foregroundStyle(DesignTokens.onActionColor)
                    .buttonBorderShape(.roundedRectangle(radius: 12))
                }

                if model.isWebBackend {
                    Button {
                        sourceTab = 2
                    } label: {
                        Label(
                            NSLocalizedString("model.center.configure_host", value: "Configure host", comment: "Configure host button"),
                            systemImage: "checkmark.shield"
                        )
                        .font(.caption.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityLabel(NSLocalizedString("model.center.configure_host", value: "配置宿主机", comment: "Configure host button"))
                } else {
                    if !model.isLocal {
                        Button {
                            capabilityTarget = model
                        } label: {
                            Label(NSLocalizedString("model.center.execution_capabilities", value: "Capabilities", comment: "Cloud model execution capabilities"), systemImage: "checkmark.shield")
                                .font(.caption.weight(.semibold))
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("model.capabilities.\(model.id)")
                    }
                    Button {
                        modelTestTarget = model
                    } label: {
                        Label(
                            NSLocalizedString("model.test.title", value: "Test model", comment: "Test this configured model"),
                            systemImage: "gauge.with.dots.needle.67percent"
                        )
                        .font(.caption.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    }
                    .buttonStyle(.bordered)
                    Button {
                        parameterTarget = model
                    } label: {
                        Label(
                            NSLocalizedString("model.center.parameters", value: "Parameters", comment: "Parameters button"),
                            systemImage: "slider.horizontal.3"
                        )
                        .font(.caption.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityLabel(NSLocalizedString("model.center.parameters", value: "参数", comment: "Parameters button"))
                }
            }
        }
        .padding(.vertical, DesignTokens.spacing_sm)
        .accessibilityElement(children: .contain)
    }

    private var modelActionColumns: [GridItem] {
        if horizontalSizeClass == .regular {
            return [GridItem(.adaptive(minimum: 132, maximum: 210), alignment: .leading)]
        }
        return [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)]
    }

    private func statusBanner(_ message: String) -> some View {
        HStack(alignment: .top, spacing: DesignTokens.spacing_sm) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(DesignTokens.successColor)
            Text(message)
                .font(.footnote)
                .foregroundStyle(.primary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button {
                statusMessage = nil
            } label: {
                Image(systemName: "xmark")
                    .frame(width: 44, height: 44)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel(NSLocalizedString("common.close", value: "Close", comment: "Close action"))
        }
        .padding(DesignTokens.spacing_md)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.successColor.opacity(DesignTokens.overlaySoftOpacity))
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.successColor.opacity(0.20), lineWidth: 1)
        )
    }

    private func reloadModels() {
        models = ModelSelectionManager.getAvailableModels()
        selectedModel = container.chatManager.getCurrentModel() ?? ModelSelectionManager.getLastSelectedModel()
    }

    private func resolveLocalModel(_ model: ModelInfo) -> LMModel? {
        if let type = ModelDownloader.ModelType(rawValue: model.id),
           let mlxName = type.mlxModelName,
           let resolved = MLXService.availableModels.first(where: { $0.name == mlxName }) {
            return resolved
        }
        return MLXService.availableModels.first { $0.id == model.id || $0.name == model.id }
    }

    private func startLocalModel(_ model: LMModel) {
        Task {
            do {
                try await localRuntime.start(model: model)
            } catch {
                runtimeError = error.localizedDescription
            }
        }
    }

    private func select(_ model: ModelInfo) {
        if model.isWebBackend,
           (model.baseURL?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false
            || model.resolvedAPIKey?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false) {
            sourceTab = 2
            return
        }
        ModelSelectionManager.saveSelectedModel(model)
        container.chatManager.selectModel(model)
        selectedModel = model
        statusMessage = model.isLocal
            ? String(
                format: NSLocalizedString(
                    "model.center.local_selected_format",
                    value: "%@ selected. Start it when you are ready to run locally.",
                    comment: "Local model selected without starting"
                ),
                model.name
            )
            : String(
                format: NSLocalizedString(
                    "model.center.remote_selected_format",
                    value: "%@ is now active.",
                    comment: "Remote model selected"
                ),
                model.name
            )
    }

    private func isSelected(_ model: ModelInfo) -> Bool {
        selectedModel?.id == model.id && selectedModel?.isLocal == model.isLocal
    }

    private func modelDetail(_ model: ModelInfo) -> String {
        if let baseURL = model.baseURL, let host = URL(string: baseURL)?.host, !host.isEmpty {
            return host
        }
        return model.source
    }
}

private struct IOSModelExecutionCapabilitiesView: View {
    @Environment(\.dismiss) private var dismiss
    let model: ModelInfo
    let onSave: (ModelExecutionCapabilities) throws -> Void
    @State private var capabilities: ModelExecutionCapabilities
    @State private var errorMessage: String?
    @State private var showDiscardConfirmation = false

    private var isDirty: Bool { capabilities != (model.executionCapabilities ?? .init(toolCalls: true)) }

    init(model: ModelInfo, onSave: @escaping (ModelExecutionCapabilities) throws -> Void) {
        self.model = model
        self.onSave = onSave
        _capabilities = State(initialValue: model.executionCapabilities ?? .init(toolCalls: true))
    }

    var body: some View {
        Form {
            Section {
                Text(NSLocalizedString("model.execution.hint", value: "Tool calls are enabled by default on import. Provider declarations and your explicit settings override this default.", comment: "Model execution capability guidance"))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Toggle(NSLocalizedString("model.execution.tools", value: "Native tool calls", comment: "Tool-call capability"), isOn: $capabilities.toolCalls)
                Toggle(NSLocalizedString("model.execution.images", value: "Image input", comment: "Image-input capability"), isOn: $capabilities.imageInput)
                Toggle(NSLocalizedString("model.execution.reasoning", value: "Reasoning summaries", comment: "Public reasoning capability"), isOn: $capabilities.reasoning)
                Toggle(NSLocalizedString("model.execution.sampling", value: "Temperature and Top P", comment: "Sampling capability"), isOn: $capabilities.samplingParameters)
                Picker(NSLocalizedString("model.execution.output_field", value: "Chat output limit field", comment: "Chat Completions token-limit key"), selection: Binding(
                    get: { capabilities.chatOutputTokenField ?? .auto },
                    set: { capabilities.chatOutputTokenField = $0 }
                )) {
                    Text(NSLocalizedString("model.execution.output_auto", value: "Automatic", comment: "Provider default output field"))
                        .tag(ChatOutputTokenField.auto)
                    Text(verbatim: "max_tokens").tag(ChatOutputTokenField.maxTokens)
                    Text(verbatim: "max_completion_tokens").tag(ChatOutputTokenField.maxCompletionTokens)
                }
            }
            if let errorMessage {
                Text(errorMessage).foregroundStyle(.red)
            }
        }
        .navigationTitle(model.name)
        .navigationBarTitleDisplayMode(.inline)
        .interactiveDismissDisabled(isDirty)
        .confirmationDialog(NSLocalizedString("common.discard_changes", value: "Discard Changes?", comment: "Discard changes confirmation"), isPresented: $showDiscardConfirmation) {
            Button(NSLocalizedString("common.discard", value: "Discard", comment: "Discard changes"), role: .destructive) { dismiss() }
            Button(L10n.cancel, role: .cancel) { }
        }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button(L10n.cancel) {
                    if isDirty { showDiscardConfirmation = true } else { dismiss() }
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(L10n.commonSave) {
                    do {
                        try onSave(capabilities)
                        dismiss()
                    } catch {
                        errorMessage = error.localizedDescription
                    }
                }
                .disabled(capabilities == (model.executionCapabilities ?? .init(toolCalls: true)))
            }
        }
    }
}


struct WebHostConfigurationView: View {
    @Environment(\.dismiss) private var dismiss

    let existingModel: ModelInfo?
    let embedded: Bool
    let onConnected: (ModelInfo) -> Void

    @State private var hostURL: String
    @State private var accessToken: String
    @State private var isConnecting = false
    @State private var importedSetup: AppleHostConnectionSetup?
    private enum ConnectionField: Hashable {
        case url
        case token
    }
    @FocusState private var connectionField: ConnectionField?
    @State private var connectionTask: Task<Void, Never>?
    @State private var errorMessage: String?

    init(existingModel: ModelInfo?, embedded: Bool = false, onConnected: @escaping (ModelInfo) -> Void) {
        self.existingModel = existingModel
        self.embedded = embedded
        self.onConnected = onConnected
        _hostURL = State(initialValue: existingModel?.baseURL ?? "")
        _accessToken = State(initialValue: existingModel?.resolvedAPIKey ?? "")
    }

    var body: some View {
        Form {
            Section {
                VStack(alignment: .leading, spacing: 12) {
                    Label {
                        Text(NSLocalizedString(
                            "model.web_host.setup.start",
                            value: "On the Mac, open Nola Settings > Host connection and enable device connections.",
                            comment: "Web Host setup first step"
                        ))
                    } icon: {
                        Image(systemName: "1.circle.fill")
                            .foregroundStyle(DesignTokens.primaryColor)
                    }

                    Label {
                        Text(NSLocalizedString(
                            "model.web_host.setup.copy",
                            value: "Copy the address and pairing token shown by Nola on the Mac. Keep both devices on a trusted private network, or use HTTPS.",
                            comment: "Web Host setup second step"
                        ))
                    } icon: {
                        Image(systemName: "2.circle.fill")
                            .foregroundStyle(DesignTokens.primaryColor)
                    }
                }
                .font(.subheadline)
            } header: {
                Text(NSLocalizedString("model.web_host.setup", value: "Before connecting", comment: "Web Host setup section"))
            }

            if isConnecting {
                Section {
                    HStack(spacing: 12) {
                        ProgressView()
                        VStack(alignment: .leading, spacing: 3) {
                            Text(NSLocalizedString("model.web_host.checking", value: "Checking host", comment: "Web Host checking title"))
                                .font(.subheadline.weight(.semibold))
                            Text(NSLocalizedString(
                                "model.web_host.checking.detail",
                                value: "Verifying the token, permission protocol, and host identity before saving.",
                                comment: "Web Host checking detail"
                            ))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                }
            }

            Section {
                Button {
                    guard let text = UIPasteboard.general.string, let setup = AppleHostConnectionSetup.decode(text) else {
                        errorMessage = NSLocalizedString("model.web_host.invalid_setup", value: "Copy Connection Setup from Nola on the host, then paste it here.", comment: "Invalid host setup")
                        return
                    }
                    importedSetup = setup
                    hostURL = setup.origin.absoluteString
                    accessToken = setup.accessToken
                    errorMessage = nil
                } label: {
                    Label(NSLocalizedString("model.web_host.paste_setup", value: "Paste Connection Setup", comment: "Paste host setup action"), systemImage: "doc.on.clipboard")
                }
                .disabled(isConnecting)
                .accessibilityIdentifier("model.web_host.paste_setup")

                TextField(
                    NSLocalizedString("model.web_host.url", value: "Host URL", comment: "Web Host URL field"),
                    text: $hostURL
                )
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                .focused($connectionField, equals: .url)
                .accessibilityIdentifier("model.web_host.url_field")
                .disabled(isConnecting)

                SecureField(
                    NSLocalizedString("model.web_host.token", value: "Access token", comment: "Web Host access token field"),
                    text: $accessToken
                )
                // A Host access token is not a website password. Nola owns its
                // verified Keychain persistence. A nil content type still lets
                // UIKit infer a login form and present Save Password after
                // pairing; mark the pairing code explicitly instead.
                .textContentType(.oneTimeCode)
                .focused($connectionField, equals: .token)
                .keyboardType(.asciiCapable)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("model.web_host.token_field")
                .disabled(isConnecting)

            } header: {
                Text(NSLocalizedString("model.web_host.connection", value: "Connection", comment: "Web Host connection section"))
            } footer: {
                if let errorMessage, !errorMessage.isEmpty {
                    Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("model.web_host.validation_error")
                } else {
                    Text(NSLocalizedString(
                        "model.web_host.connection.footer",
                        value: "Nola verifies the host before storing the token in this device's Keychain.",
                        comment: "Web Host connection footer"
                    ))
                }
            }

            Section {
                Label {
                    Text(NSLocalizedString(
                        "model.web_host.security_summary",
                        value: "Nola verifies the token, permission protocol, and host identity before pairing. Every host or device tool still follows its own approval policy.",
                        comment: "Web Host security summary"
                    ))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                } icon: {
                    Image(systemName: "checkmark.shield")
                        .foregroundStyle(DesignTokens.primaryColor)
                }
                .accessibilityElement(children: .combine)
            }

            if embedded { Section { connectButton } }

        }
        .navigationTitle(embedded ? L10n.modelsTitle : NSLocalizedString("model.web_host.title", value: "Host connection", comment: "Web Host configuration title"))
        .navigationBarTitleDisplayMode(.inline)
        .interactiveDismissDisabled(isConnecting)
        .toolbar {
            if !embedded {
                ToolbarItem(placement: .cancellationAction) {
                    Button(NSLocalizedString("common.cancel", value: "Cancel", comment: "Cancel action")) {
                        connectionTask?.cancel()
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) { connectButton }
            }
        }
        .onDisappear { connectionTask?.cancel() }
    }

    private var connectButton: some View {
        Button {
            connect()
        } label: {
            if isConnecting {
                ProgressView()
            } else {
                Text(NSLocalizedString("model.web_host.connect", value: "Connect", comment: "Connect Web Host action"))
            }
        }
        .disabled(isConnecting)
        .accessibilityIdentifier("model.web_host.connect")
        .accessibilityHint(NSLocalizedString(
            "model.web_host.connect.accessibility_hint",
            value: "Validates this host and pairs its identity before saving.",
            comment: "Connect Web Host accessibility hint"
        ))
    }

    private func connect() {
        guard !isConnecting else { return }
        errorMessage = nil
        let rawURL = hostURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let token = accessToken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: rawURL),
              let scheme = url.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              url.host?.isEmpty == false else {
            errorMessage = NSLocalizedString("model.web_host.invalid_url", value: "Enter a valid HTTP or HTTPS Host URL.", comment: "Invalid Web Host URL error")
            return
        }
        guard MobileHostTransportPolicy.allows(url) else {
            errorMessage = NSLocalizedString("model.web_host.public_http_forbidden", value: "Public Web Host addresses require HTTPS. HTTP is limited to this device or a private local network.", comment: "Public HTTP Web Host error")
            return
        }
        guard !token.isEmpty else {
            errorMessage = NSLocalizedString("model.web_host.missing_token", value: "Enter the access token for this host.", comment: "Missing Web Host token error")
            return
        }
        connectionField = nil
        isConnecting = true
        connectionTask = Task { @MainActor in
            defer { isConnecting = false }
            do {
                try await WebAgentProvider.validateToken(
                    baseURL: url,
                    token: token,
                    pinnedCertificateSHA256: nil
                )
                let settings = try await WebAgentProvider.fetchHostAgentSettings(
                    baseURL: url,
                    token: token,
                    pinnedCertificateSHA256: nil
                )
                try Task.checkCancellation()
                guard settings.host_identity?.permission_protocol_version == WebAgentProvider.hostPermissionProtocolVersion,
                      let identity = settings.host_identity?.id.trimmingCharacters(in: .whitespacesAndNewlines),
                      !identity.isEmpty else {
                    throw WebHostPairingFailure.unsupportedPermissionProtocol
                }
                if let setup = importedSetup, AppleHostConnectionSetup.sameOrigin(setup.origin, url), setup.hostID != identity {
                    throw WebHostPairingFailure.identityChanged
                }
                if let existingIdentity = ModelSelectionManager.pairedWebHostIdentity(for: url),
                   existingIdentity != identity {
                    throw WebHostPairingFailure.identityChanged
                }
                guard ModelSelectionManager.saveWebHostToken(token, for: rawURL) else {
                    throw WebHostPairingFailure.secureStorageFailed
                }
                guard ModelSelectionManager.trustWebHostIdentity(identity, for: url) else {
                    throw WebHostPairingFailure.identityStorageFailed
                }
                let model = ModelSelectionManager.webBackendModel(
                    baseURL: rawURL,
                    pinnedCertificateSHA256: nil
                )
                // The verified token is now owned by Keychain. Remove the
                // transient field value before the form disappears so UIKit
                // does not offer to save this pairing secret as a login.
                accessToken = ""
                connectionField = nil
                await Task.yield()
                try Task.checkCancellation()
                onConnected(model)
                dismiss()
            } catch {
                guard !Task.isCancelled else { return }
                errorMessage = webHostPairingErrorMessage(error)
            }
        }
    }

}

/// A model check uses the selected profile's saved parameters without
/// changing the shared chat provider or creating a user conversation.
struct IOSModelInferenceTestView: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss

    let model: ModelInfo
    @State private var prompt = NSLocalizedString("model.test.default_prompt", value: "Explain what Nola can do in three concise bullet points.", comment: "Default model test prompt")
    @State private var output = ""
    @State private var errorMessage: String?
    @State private var isRunning = false
    @State private var startedAt: Date?
    @State private var firstTokenLatency: TimeInterval?
    @State private var totalDuration: TimeInterval?
    @State private var runTask: Task<Void, Never>?
    @State private var runGeneration = 0
    @State private var showParameters = false

    var body: some View {
        NolaPage {
            NolaPageHeader(
                icon: "gauge.with.dots.needle.67percent",
                title: NSLocalizedString("model.test.title", value: "Test model", comment: "Model inference test title"),
                subtitle: model.name
            )
            NolaCard {
                NolaSectionTitle(icon: "bubble.left", title: NSLocalizedString("model.test.prompt", value: "Test prompt", comment: "Model test prompt"))
                TextEditor(text: $prompt)
                    .frame(minHeight: 140)
                    .accessibilityIdentifier("model.test.prompt")
                HStack {
                    Button(NSLocalizedString("model.center.parameters", value: "Parameters", comment: "Model parameters")) {
                        showParameters = true
                    }
                    .buttonStyle(.bordered)
                    Spacer()
                    Button(isRunning
                        ? NSLocalizedString("model.test.stop", value: "Stop", comment: "Stop model test")
                        : NSLocalizedString("model.test.run", value: "Run", comment: "Run model test")) {
                        isRunning ? stop() : run()
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!isRunning && prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            NolaCard {
                NolaSectionTitle(icon: "stopwatch", title: NSLocalizedString("model.test.metrics", value: "Metrics", comment: "Model test metrics"))
                metric(NSLocalizedString("model.test.first_token", value: "First token", comment: "First token latency"), firstTokenLatency)
                metric(NSLocalizedString("model.test.total_time", value: "Total time", comment: "Model test duration"), totalDuration)
                HStack {
                    Text(NSLocalizedString("model.test.characters", value: "Characters", comment: "Generated character count"))
                    Spacer()
                    Text(output.count.formatted()).monospacedDigit()
                }
            }
            NolaCard {
                HStack {
                    NolaSectionTitle(icon: "text.bubble", title: NSLocalizedString("model.test.output", value: "Output", comment: "Model test output"))
                    Spacer()
                    if isRunning { ProgressView().controlSize(.small) }
                }
                if let errorMessage {
                    Text(errorMessage).foregroundStyle(.red).textSelection(.enabled)
                }
                if output.isEmpty {
                    Text(NSLocalizedString("model.test.empty", value: "Run a model test to inspect its response here.", comment: "Empty model test output"))
                        .foregroundStyle(.secondary)
                } else {
                    ChatSmoothStreamingMarkdownView(content: output, isStreaming: isRunning)
                        .textSelection(.enabled)
                }
            }
        }
        .navigationTitle(model.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(NSLocalizedString("model.test.close", value: "Close", comment: "Close model test")) { dismiss() }
            }
        }
        .navigationDestination(isPresented: $showParameters) {
            ModelParametersView(modelID: model.id, displayName: model.name, isLocal: model.isLocal)
        }
        .onDisappear { stop() }
    }

    private func metric(_ label: String, _ value: TimeInterval?) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(value.map { String(format: "%.2f s", $0) } ?? "—").monospacedDigit()
        }
    }

    private func run() {
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        stop()
        runGeneration += 1
        let generation = runGeneration
        output = ""
        errorMessage = nil
        firstTokenLatency = nil
        totalDuration = nil
        let start = Date()
        startedAt = start
        isRunning = true
        runTask = Task { @MainActor in
            do {
                let parameters = ModelParametersStore.shared.loadParameters(for: model.id, isLocal: model.isLocal)
                let stream: AsyncThrowingStream<String, Error>
                if model.isLocal {
                    let downloaded = ModelDownloader.ModelType(rawValue: model.id)
                        .map { container.modelDownloader.isModelDownloaded($0) } ?? false
                    guard let local = IOSModelInferenceTestRoute.localRuntimeModel(model, downloaded: downloaded) else {
                        throw NolaError.llm(.notConfigured)
                    }
                    try await MLXService.shared.start(model: local)
                    try Task.checkCancellation()
                    stream = try await MLXService.shared.generate(
                        prompt: text, systemPrompt: parameters.customSystemPrompt.isEmpty ? nil : parameters.customSystemPrompt,
                        model: local, temperature: Float(parameters.temperature), maxTokens: parameters.maxOutputTokens,
                        topP: Float(parameters.topP), topK: parameters.topK, enableThinking: parameters.showThinking,
                        repetitionPenalty: Float(LocalSamplingSettings(parameters: parameters).repetitionPenalty), maxContextTokens: parameters.maxContextLength,
                        seed: parameters.seed
                    )
                } else {
                    let resolvedModel = try ModelSelectionManager.resolvedCloudModelForRun(model)
                    let configuration = try IOSConversationRunConfiguration.directCloudProviderConfig(model: resolvedModel, parameters: parameters)
                    stream = OpenAIProvider(config: configuration).chatStream(
                        message: text,
                        systemPrompt: parameters.customSystemPrompt.isEmpty ? nil : parameters.customSystemPrompt,
                        enableThinking: parameters.showThinking
                    )
                }
                for try await chunk in stream {
                    try Task.checkCancellation()
                    guard generation == runGeneration else { return }
                    if !chunk.isEmpty && firstTokenLatency == nil {
                        firstTokenLatency = Date().timeIntervalSince(start)
                    }
                    output += chunk
                }
            } catch is CancellationError {
                // Stop is an explicit user action, not a failed inference.
            } catch {
                if generation == runGeneration { errorMessage = error.localizedDescription }
            }
            guard generation == runGeneration else { return }
            totalDuration = Date().timeIntervalSince(start)
            isRunning = false
            runTask = nil
        }
    }

    private func stop() {
        runGeneration += 1
        runTask?.cancel()
        runTask = nil
        if isRunning, let startedAt { totalDuration = Date().timeIntervalSince(startedAt) }
        isRunning = false
    }
}

enum IOSModelInferenceTestRoute {
    static func localRuntimeModel(_ model: ModelInfo, downloaded: Bool) -> LMModel? {
        guard model.isLocal, downloaded else { return nil }
        return MLXService.availableModels.first { $0.id == model.id || $0.name == model.id }
    }
}

enum WebHostPairingFailure: Error {
    case unsupportedPermissionProtocol
    case identityChanged
    case secureStorageFailed
    case identityStorageFailed
}

func webHostPairingErrorMessage(_ error: Error) -> String {
    switch error {
    case WebHostPairingFailure.unsupportedPermissionProtocol:
        return NSLocalizedString("model.web_host.error.protocol", value: "This host uses an unsupported permission protocol. Update Nola on the host and try again.", comment: "Unsupported Web Host protocol")
    case WebHostPairingFailure.identityChanged:
        return NSLocalizedString("model.web_host.error.identity_changed", value: "This address now identifies a different host. Forget the previous pairing before reconnecting.", comment: "Changed Web Host identity")
    case WebHostPairingFailure.secureStorageFailed:
        return NSLocalizedString("model.web_host.error.secure_storage", value: "The access token could not be stored securely on this device.", comment: "Web Host secure storage failure")
    case WebHostPairingFailure.identityStorageFailed:
        return NSLocalizedString("model.web_host.error.pairing", value: "The verified host identity could not be saved. Try again.", comment: "Web Host pairing storage failure")
    default:
        break
    }

    if let urlError = error as? URLError {
        switch urlError.code {
        case .timedOut:
            return NSLocalizedString("model.web_host.error.timeout", value: "The host did not respond in time. Check that Nola Web Host is running and reachable.", comment: "Web Host timeout")
        case .cannotConnectToHost, .cannotFindHost, .dnsLookupFailed, .networkConnectionLost, .notConnectedToInternet:
            return NSLocalizedString("model.web_host.error.unreachable", value: "The host could not be reached. Check its address, network, and running status.", comment: "Web Host unreachable")
        default:
            break
        }
    }

    let description = error.localizedDescription.lowercased()
    if description.contains("401") || description.contains("403") || description.contains("unauthorized") {
        return NSLocalizedString("model.web_host.error.unauthorized", value: "The access token was rejected. Copy the current token from the host and try again.", comment: "Web Host unauthorized")
    }
    return NSLocalizedString("model.web_host.error.generic", value: "The host could not be verified. Check the connection details and try again.", comment: "Generic Web Host connection failure")
}

#Preview {
    ModelsView()
        .environmentObject(ServiceContainer())
}
