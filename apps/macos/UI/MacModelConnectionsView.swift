// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import AppKit

struct MacModelConnectionsView: View {
    let service: MacModelConnectionService
    private var refreshOnAppear = true
    init(service: MacModelConnectionService) { self.service = service }
    @State private var connections: [MacModelConnectionSnapshot] = []
    @State private var accounts: [MacModelAccountSnapshot] = []
    @State private var enabled: [String: Set<String>] = [:]
    @State private var savedEnabled: [String: Set<String>] = [:]
    @State private var confirmsDiscard = false
    @State private var name = ""
    @State private var endpoint = ""
    @State private var secret = ""
    @State private var wire = "responses"
    @State private var busy = false
    @State private var busyAccountIDs: Set<String> = []
    @State private var discovering = false
    @State private var operationTask: Task<Void, Never>?
    @State private var error: String?
    @State private var status: String?
    @State private var importSource = "open_code"
    @State private var sourceURL: URL?
    @State private var credentialURL: URL?
    @State private var includeCredentials = false
    @State private var allowEnvironment = false
    @State private var enableImportedModels = true
    @State private var importPlan: MacModelImportPlan?
    @State private var importSelection = Set<String>()
    @State private var importUpdates = Set<String>()
    @State private var importResults: [MacModelImportResult] = []
    @State private var importBridge: MacModelImportBridge?
    @State private var probeBridge: MacModelImportBridge?
    @State private var probeConfirmation: MacModelProbeConfirmation?
    @State private var showingProbe = false
    @State private var probeRunning = false
    @State private var managementHost: MacRemoteHostConfiguration?
    @State private var showingSiwc = false
    @State private var siwcAccount: MacModelAccountSnapshot?
    @State private var hoveredConnectionID: String?
    @State private var search = ""
    @State private var modelSearch = ""
    @State private var expandedModelID: String?
    @State private var selectedConnectionID: String?
    @State private var workspace: Workspace?
    private enum Workspace: Hashable, Identifiable {
        case connections, api, imports, accounts, host
        var id: Self { self }
    }
    @Environment(\.openURL) private var openURL
    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                HStack(spacing: 8) {
                    Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 16, height: 16).foregroundStyle(.secondary)
                    TextField(String(localized: "Search connections", table: "ModelCenter"), text: $search)
                        .textFieldStyle(.plain)
                }
                .padding(9)
                .background(
                    MacDesignColor.chrome.opacity(0.72),
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous)
                )
                .frame(maxWidth: 320)
                Spacer(minLength: 0)
                if busy { ProgressView().controlSize(.small) }
                Button { reload() } label: {
                    Label(String(localized: "Refresh connections", table: "ModelCenter"), systemImage: "arrow.clockwise")
                }
                .labelStyle(.iconOnly)
                .help(String(localized: "Refresh connections", table: "ModelCenter"))
            }
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter).padding(.vertical, MacDesignTokens.Spacing.md)
            .background(MacDesignColor.chrome.opacity(0.72))
            if !connections.isEmpty {
                connectionActionRow
                    .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                    .padding(.bottom, MacDesignTokens.Spacing.md)
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled).padding(.horizontal, 24) }
            if let status { Text(status).font(.callout).foregroundStyle(.secondary).padding(.horizontal, 24) }
            if connections.isEmpty {
                VStack(alignment: .leading, spacing: 20) {
                    Text(String(localized: "Connections and accounts", table: "ModelCenter"))
                        .font(.title2.weight(.semibold))
                    connectionActionRow
                    Spacer()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(24)

            } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(filteredConnections) { connection in
                        Button {
                            openEditor(.connections)
                            selectedConnectionID = connection.id
                        } label: {
                            connectionRow(connection)
                        }
                        .buttonStyle(.plain)
                        .background(hoveredConnectionID == connection.id ? MacDesignColor.hover : Color.clear, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous))
                        .onHover { hoveredConnectionID = $0 ? connection.id : nil }
                        .accessibilityIdentifier("model-connection-" + connection.id)
                        Divider().padding(.leading, 44)
                    }
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                if filteredConnections.isEmpty {
                    ContentUnavailableView.search(text: search)
                }
            }
            }
        }
        .background(MacDesignColor.canvas)
        .tint(MacDesignColor.brand)
        .task { if refreshOnAppear { reload(); managementHost = MacRemoteHostConfigurationStore.shared.load() } }
        .sheet(item: $workspace) { target in
            VStack(spacing: 0) {
                HStack {
                    Text(editorTitle(target)).font(.headline)
                    Spacer()
                    Button(String(localized: "Close", table: "ModelCenter")) {
                        requestEditorClose()
                    }
                        .keyboardShortcut(.cancelAction)
                        .disabled(isEditorBusy)
                        .accessibilityIdentifier("model-editor-close")
                }.padding(20)
                Divider()
                editorContent(target)
                editorFooter
            }
            .frame(minWidth: 460, idealWidth: 580, minHeight: 480, idealHeight: 640)
            .interactiveDismissDisabled(hasModelChanges || isEditorBusy)
            .background(MacEditorWindowCloseGuard(
                shouldPreventClose: { hasModelChanges || isEditorBusy },
                onCloseAttempt: requestEditorClose
            ))
            .confirmationDialog(String(localized: "Discard model selection changes?", table: "ModelCenter"), isPresented: $confirmsDiscard) {
                Button(String(localized: "Discard changes", table: "ModelCenter"), role: .destructive) {
                    if let id = selectedConnectionID { enabled[id] = savedEnabled[id, default: []] }
                    workspace = nil
                }
                Button(String(localized: "Continue editing", table: "ModelCenter"), role: .cancel) {}
            }
        }
    }

    private var connectionActionRow: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { connectionActions }
            VStack(alignment: .leading, spacing: 8) { connectionActions }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var connectionActions: some View {
        Button(String(localized: "Connect an API account", table: "ModelCenter")) { openEditor(.api) }
            .accessibilityIdentifier("model.add.api")
        Button(String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter")) { openEditor(.accounts) }
            .accessibilityIdentifier("model.add.chatgpt")
        Button(String(localized: "Import from this Mac", table: "ModelCenter")) { openEditor(.imports) }
            .accessibilityIdentifier("model.add.import")
    }

    private var apiConnectionSection: some View {
        Section {
            TextField(String(localized: "Connection name", table: "ModelCenter"), text: $name)
            TextField(String(localized: "API endpoint", table: "ModelCenter"), text: $endpoint,
                prompt: Text("https://api.example.com/v1"))
                .autocorrectionDisabled()
            Picker(String(localized: "Protocol", table: "ModelCenter"), selection: $wire) {
                Text(verbatim: "Responses").tag("responses")
                Text(verbatim: "Chat Completions").tag("chat_completions")
                Text(verbatim: "Anthropic Messages").tag("anthropic")
            }
            SecureField(String(localized: "API key · optional for anonymous endpoints", table: "ModelCenter"), text: $secret)
            Button(String(localized: "Connect account", table: "ModelCenter")) {
                perform {
                    let created = try service.create(name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                        endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines),
                        wireProtocol: wire, secret: secret)
                    secret = ""; name = ""; endpoint = ""
                    selectedConnectionID = created.id
                    workspace = .connections
                    status = String(localized: "Connection saved. Discover and enable models before using it.", table: "ModelCenter")
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !Self.isValidEndpoint(endpoint))
            if !endpoint.isEmpty && !Self.isValidEndpoint(endpoint) {
                Text(String(localized: "Enter an HTTP or HTTPS address without embedded credentials.", table: "ModelCenter"))
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private var filteredConnections: [MacModelConnectionSnapshot] {
        connections.filter {
            search.isEmpty || $0.document.connection.display_name.localizedCaseInsensitiveContains(search)
                || $0.document.connection.base_url.localizedCaseInsensitiveContains(search)
        }
    }

    private var hasModelChanges: Bool {
        guard let id = selectedConnectionID else { return false }
        return enabled[id, default: []] != savedEnabled[id, default: []]
    }

    private func editorTitle(_ target: Workspace) -> String {
        switch target {
        case .connections: connections.first { $0.id == selectedConnectionID }?.document.connection.display_name
            ?? String(localized: "Connections and accounts", table: "ModelCenter")
        case .api: String(localized: "Connect an API account", table: "ModelCenter")
        case .imports: String(localized: "Import from this Mac", table: "ModelCenter")
        case .accounts: String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter")
        case .host: String(localized: "Source login on a Nola Host", table: "ModelCenter")
        }
    }

    private var isEditorBusy: Bool { busy || probeRunning || !busyAccountIDs.isEmpty }

    private func requestEditorClose() {
        guard !isEditorBusy else { return }
        if hasModelChanges { confirmsDiscard = true } else { workspace = nil }
    }

    private func openEditor(_ target: Workspace) {
        error = nil
        status = nil
        modelSearch = ""
        workspace = target
        selectedConnectionID = nil
    }

    private func connectionRow(_ snapshot: MacModelConnectionSnapshot) -> some View {
        let connection = snapshot.document.connection
        return HStack(spacing: 14) {
            Image(connection.source == "nola_chatgpt" ? "nola-person" : "nola-models")
                .renderingMode(.template).resizable().scaledToFit().frame(width: 17, height: 17)
                .foregroundStyle(MacDesignColor.brand)
                .frame(width: 30, height: 30)
                .background(MacDesignColor.brand.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 5) {
                Text(connection.display_name).font(.body.weight(.semibold)).lineLimit(1)
                Text(connection.base_url)
                    .font(.callout).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 5) {
                Text(connection.enabled ? String(localized: "Enabled")
                     : String(localized: "Disabled"))
                    .font(.callout)
                    .foregroundStyle(connection.enabled ? Color.primary : Color.secondary)
                Text(String(localized: "Models", table: "ModelCenter") + " · " + String(snapshot.document.catalog?.models.count ?? 0))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Image("nola-chevron").renderingMode(.template).resizable().scaledToFit()
                .frame(width: 12, height: 12).foregroundStyle(.tertiary)
        }
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        .help(connection.base_url)
    }

    @ViewBuilder private var editorFooter: some View {
        if workspace == .connections,
           let connection = connections.first(where: { $0.id == selectedConnectionID }),
           connection.document.connection.enabled {
            Divider()
            HStack {
                Menu {
                    Button(String(localized: "Disconnect", table: "ModelCenter"), role: .destructive) {
                        perform {
                            _ = try service.disconnect(connection)
                            status = String(localized: "Connection disabled. Credentials were not deleted.", table: "ModelCenter")
                        }
                    }
                } label: {
                    Label(String(localized: "Model options", table: "ModelCenter"), systemImage: "ellipsis")
                }.labelStyle(.iconOnly).menuStyle(.borderlessButton).fixedSize()
                .help(String(localized: "Model options", table: "ModelCenter"))
                Spacer()
                if connection.document.catalog != nil {
                    Button(String(localized: "Apply enabled models", table: "ModelCenter")) {
                        perform {
                            try service.enable(connection, modelIDs: enabled[connection.id, default: []])
                            savedEnabled[connection.id] = enabled[connection.id, default: []]
                            status = String(localized: "Enabled models are available in the chat model picker.", table: "ModelCenter")
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    // Applying also adopts newly discovered capability metadata for enabled models.
                    .disabled(busy)
                }
            }.padding(.horizontal, 24).padding(.vertical, 12).disabled(busy)
        }
    }

    private var connectionDetail: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
                if let status { Text(status).font(.callout).foregroundStyle(.secondary) }
            if connections.isEmpty {
                ContentUnavailableView {
                    Label(String(localized: "Connections and accounts", table: "ModelCenter"), systemImage: "network")
                } actions: {
                    Button(String(localized: "Connect an API account", table: "ModelCenter")) { workspace = .api }
                        .buttonStyle(.borderedProminent)
                    Button(String(localized: "Import from this Mac", table: "ModelCenter")) { workspace = .imports }
                }
            }
            ForEach(connections.filter { $0.id == selectedConnectionID }) { connection in
                VStack(alignment: .leading, spacing: 10) {
                    VStack(alignment: .leading, spacing: 8) {
                    Text(connection.document.connection.base_url).font(.caption).textSelection(.enabled)
                        .lineLimit(2).truncationMode(.middle).help(connection.document.connection.base_url)
                    Text(connection.document.connection.enabled ? String(localized: "Enabled") : String(localized: "Disabled"))
                        .foregroundStyle(.secondary)
                    if let id = connection.document.connection.account_id,
                       let account = accounts.first(where: { $0.id == id }) {
                        Label(account.account.display_name, systemImage: "person.crop.circle")
                    }
                    }
                    if connection.document.connection.enabled {
                        if connection.document.connection.source != "nola_chatgpt" {
                        Button(String(localized: "Discover models · no inference", table: "ModelCenter")) {
                            perform {
                                discovering = true
                                defer { discovering = false }
                                _ = try await service.discover(connection)
                                status = String(localized: "Model discovery finished. Select the models to enable.", table: "ModelCenter")
                            }
                        }
                        } else {
                            if let account = accounts.first(where: { $0.id == connection.document.connection.account_id }) {
                                MacSiwcAccountManagementView(application: service.application, account: account, changed: reload)
                            }
                        }
                        if let catalog = connection.document.catalog, !catalog.models.isEmpty {
                            if connection.document.connection.source == "nola_chatgpt",
                               catalog.models.contains(where: { $0.reasoning_levels.contains("ultra") }) {
                                Text(String(localized: "Ultra is a ChatGPT multi-agent mode, not a model reasoning effort. Nola uses the single-model reasoning levels available for each model.", table: "ModelCenter"))
                                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                            }
                            TextField(String(localized: "Search models", table: "ModelCenter"), text: $modelSearch)
                                .textFieldStyle(.roundedBorder)
                        } else {
                            Text(String(localized: "Discover models to choose which ones appear in chat.", table: "ModelCenter"))
                                .font(.callout).foregroundStyle(.secondary)
                        }
                        ForEach((connection.document.catalog?.models ?? []).filter {
                            modelSearch.isEmpty || $0.display_name.localizedCaseInsensitiveContains(modelSearch)
                                || $0.model_id.localizedCaseInsensitiveContains(modelSearch)
                        }) { model in
                            VStack(alignment: .leading, spacing: 8) {
                            HStack(spacing: 12) {
                            Toggle(isOn: Binding(get: { enabled[connection.id, default: []].contains(model.id) }, set: { value in
                                if value { enabled[connection.id, default: []].insert(model.id) }
                                else { enabled[connection.id]?.remove(model.id) }
                            })) {
                                VStack(alignment: .leading) {
                                    Text(model.display_name)
                                    Text(Self.protocolTitle(model.protocol)).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            Spacer(minLength: 8)
                                Button {
                                    expandedModelID = expandedModelID == model.id ? nil : model.id
                                } label: {
                                    Label(String(localized: "Model options", table: "ModelCenter"), systemImage: "slider.horizontal.3")
                                }
                                .labelStyle(.iconOnly).buttonStyle(.borderless)
                                .help(String(localized: "Model options", table: "ModelCenter"))
                                .accessibilityValue(expandedModelID == model.id ? String(localized: "Expanded") : String(localized: "Collapsed"))
                            }
                            if expandedModelID == model.id {
                                if connection.document.connection.source == "nola_chatgpt" {
                                    MacModelCapabilityOverrideEditor(connection: connection, model: model,
                                        service: service, onSave: reload, onError: { error = $0 })
                                } else { modelActions(connection, model: model) }
                            }
                            }.padding(.vertical, 6)
                            Divider()
                        }

                    }
                }
            }
            
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func editorContent(_ target: Workspace) -> some View {
        Group {
        if target == .connections { connectionDetail }
        else { Form {
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if let status { Text(status).font(.callout).foregroundStyle(.secondary) }
            if target == .imports { sourceImportSection }
            if target == .accounts {
            Section(String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter")) {
                Text(String(localized: "Connect your ChatGPT account, then choose the models to use on this Mac.", table: "ModelCenter"))
                    .font(.callout).foregroundStyle(.secondary)
                Button(String(localized: "Sign in with ChatGPT", table: "ModelCenter")) { siwcAccount = nil; showingSiwc = true }
                ForEach(accounts.filter { $0.account.source == "nola_chatgpt" }) { account in
                    MacSiwcAccountManagementView(application: service.application, account: account, changed: reload,
                        modelsDiscovered: { id in selectedConnectionID = id; workspace = .connections },
                        reauthorize: { siwcAccount = account; showingSiwc = true },
                        activityChanged: { active in
                            if active { busyAccountIDs.insert(account.id) }
                            else { busyAccountIDs.remove(account.id) }
                        })
                }
            }
            }
            if target == .host { hostManagementSection }
            if target == .api { apiConnectionSection }
        }
        .formStyle(.grouped)
        }
        }
        .scrollContentBackground(.hidden)
        .background(MacDesignColor.canvas)
        .disabled(busy)
        .overlay(alignment: .topTrailing) {
            if busy {
                HStack {
                    ProgressView().controlSize(.small)
                    if discovering { Button(String(localized: "Cancel discovery", table: "ModelCenter")) { operationTask?.cancel() } }
                    if importBridge != nil { Button(String(localized: "Cancel import", table: "ModelCenter")) { cancelImport() } }
                    if probeBridge != nil { Button(String(localized: "Cancel test", table: "ModelCenter")) { cancelProbe() } }
                }.padding().background(.regularMaterial)
            }
        }
        .task { if refreshOnAppear { reload(); managementHost = MacRemoteHostConfigurationStore.shared.load() } }
        .navigationTitle(String(localized: "Connections and accounts", table: "ModelCenter"))
        .sheet(isPresented: $showingProbe, onDismiss: { if !probeRunning { cancelProbe() } }) {
            probeSheet
        }
        .sheet(isPresented: $showingSiwc, onDismiss: reload) {
            MacSiwcSignInView(application: service.application, account: siwcAccount, showAccounts: {
                showingSiwc = false; workspace = .accounts; reload()
            }, saved: { connectionID in
                showingSiwc = false
                reload()
                selectedConnectionID = connectionID
                workspace = .connections
            })
        }
        .onDisappear { operationTask?.cancel(); cancelImport(); cancelProbe() }
    }
    nonisolated static func protocolTitle(_ value: String) -> String {
        switch value {
        case "responses": "Responses"
        case "chat_completions": "Chat Completions"
        case "anthropic": "Anthropic Messages"
        default: value
        }
    }

    nonisolated static func isValidEndpoint(_ value: String) -> Bool {
        guard let url = URL(string: value.trimmingCharacters(in: .whitespacesAndNewlines)),
              ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              let host = url.host, !host.isEmpty else { return false }
        return url.user == nil && url.password == nil && url.query == nil && url.fragment == nil
    }

    private var hostManagementSection: some View {
        Section(String(localized: "Source login on a Nola Host", table: "ModelCenter")) {
            Text(String(localized: "API import above belongs to this Mac. Codex and Kimi Code login management belongs to a separately running Nola Host, not this Mac’s native account store.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            Text(String(localized: "Logging in does not enable subscription inference. Tokens remain with the source application. OpenCode login is available only when the Host supports its provider-specific flow.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            if let host = managementHost, let owner = host.expectedHostID {
                LabeledContent(String(localized: "Host owner", table: "ModelCenter"), value: owner)
                Text(host.baseURL.absoluteString).font(.caption).textSelection(.enabled)
                if Self.isLocalManagementHost(host.baseURL) {
                    Text(String(localized: "This loopback Host can manage source login in its browser page. Confirm the source owner there before logging in, refreshing or cancelling.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                } else {
                    Text(String(localized: "Remote access supports API import and capability checks. Source login is local-only: open /models?tab=api in a browser on that Host computer. This Mac cannot start or cancel its source login remotely.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                Button(Self.isLocalManagementHost(host.baseURL) ? String(localized: "Open Host source and API management", table: "ModelCenter") : String(localized: "Open remote Host API management", table: "ModelCenter")) {
                    perform {
                        let url = try await MacRemoteHostService.shared.requestModelManagementURL(configuration: host)
                        try Task.checkCancellation()
                        openURL(url)
                        status = String(localized: "Continue on the selected Host. Its accounts are not copied into this Mac.", table: "ModelCenter")
                    }
                }
            } else {
                Text(String(localized: "Pair a Nola Host in Server settings first. For source login on this computer, pair an explicitly configured loopback Host. Nola will not launch Codex or Kimi automatically.", table: "ModelCenter"))
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }
    nonisolated static func isLocalManagementHost(_ url: URL) -> Bool {
        ["http", "https"].contains(url.scheme?.lowercased() ?? "")
            && url.user == nil && url.password == nil
            && ["127.0.0.1", "[::1]", "::1"].contains(url.host?.lowercased() ?? "")
    }
    private func modelActions(_ connection: MacModelConnectionSnapshot,
        model: MacModelConnectionSnapshot.Model) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let evidence = connection.document.capability_assessments?.first(where: {
                $0.model_id == model.model_id && $0.protocol == model.protocol
            })?.evidence {
                Label(String(localized: "A previous text test succeeded", table: "ModelCenter"), systemImage: "clock")
                    .font(.callout).foregroundStyle(.secondary)
                Text(Date(timeIntervalSince1970: Double(evidence.observed_at_ms) / 1000), style: .date)
                    .font(.caption).foregroundStyle(.secondary)
            }
            Button(String(localized: "Test text response…", table: "ModelCenter")) {
                perform {
                    let bridge = try service.makeImportBridge()
                    probeBridge = bridge
                    do {
                        probeConfirmation = try await bridge.prepareProbe(snapshot: connection, model: model)
                        showingProbe = true
                    } catch { bridge.cancel(); probeBridge = nil; throw error }
                }
            }
            MacModelCapabilityOverrideEditor(connection: connection, model: model, service: service,
                onSave: { reload() }, onError: { error = $0 })
        }
    }
    private var probeSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(String(localized: "Test this model?", table: "ModelCenter")).font(.title2).bold()
            if let confirmation = probeConfirmation {
                LabeledContent(String(localized: "Model", table: "ModelCenter"), value: confirmation.model_id)
                LabeledContent(String(localized: "Protocol", table: "ModelCenter"), value: confirmation.protocol)
                LabeledContent(String(localized: "Account", table: "ModelCenter"), value: accounts.first(where: { $0.id == confirmation.account_id })?.account.display_name ?? confirmation.account_id ?? String(localized: "Anonymous endpoint", table: "ModelCenter"))
                if let connection = connections.first(where: { $0.id == confirmation.connection_id }) {
                    Text(connection.document.connection.base_url)
                        .font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text(String(localized: "This sends one short text request and may incur provider charges. It does not test tools, images, or reasoning capabilities.", table: "ModelCenter"))
                    .fixedSize(horizontal: false, vertical: true)
                if probeRunning {
                    ProgressView(String(localized: "Testing text response…", table: "ModelCenter"))
                    Text(String(localized: "Stopping cannot undo a request already sent. An uncertain result is not retried automatically.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                HStack {
                    Spacer()
                    Button(probeRunning ? String(localized: "Stop test", table: "ModelCenter") : String(localized: "Cancel", table: "ModelCenter")) { cancelProbe() }
                        .keyboardShortcut(.cancelAction)
                    Button(String(localized: "Send one test request", table: "ModelCenter")) { confirmProbe(confirmation) }
                        .buttonStyle(.borderedProminent).disabled(probeRunning)
                }
            }
        }
        .padding(24).frame(width: 440)
        .interactiveDismissDisabled(probeRunning)
    }
    private func cancelProbe() {
        probeBridge?.cancel(); probeBridge = nil; probeConfirmation = nil
        showingProbe = false
        if probeRunning {
            operationTask?.cancel()
            status = String(localized: "Test stopped. A request already sent may still be charged; its result may be unknown.", table: "ModelCenter")
        }
    }
    private func confirmProbe(_ confirmation: MacModelProbeConfirmation) {
        guard let bridge = probeBridge, !probeRunning else { return }
        probeRunning = true
        perform {
            defer {
                bridge.cancel(); probeBridge = nil; probeConfirmation = nil
                showingProbe = false; probeRunning = false
            }
            _ = try await bridge.confirmProbe(requestID: confirmation.request_id, acknowledgePossibleCharges: true)
            status = String(localized: "Text response verified. Other capabilities were not tested.", table: "ModelCenter")
        }
    }
    #if DEBUG
    static func visualDetailFixture(service: MacModelConnectionService,
        connection: MacModelConnectionSnapshot) -> some View {
        var view = Self(service: service)
        view.refreshOnAppear = false
        view._connections = State(initialValue: [connection])
        view._selectedConnectionID = State(initialValue: connection.id)
        return view.editorContent(.connections).tint(MacDesignColor.brand)
    }

    static func visualFixture(service: MacModelConnectionService,
        connections: [MacModelConnectionSnapshot] = [], busy: Bool = false, error: String? = nil) -> Self {
        var view = Self(service: service)
        view.refreshOnAppear = false
        view._connections = State(initialValue: connections)
        view._busy = State(initialValue: busy)
        view._error = State(initialValue: error)
        return view
    }
    #endif
    private var sourceImportSection: some View {
        Section {
            Text(String(localized: "Select a source file, preview its connections, then confirm. Imported API keys stay in this Mac’s Keychain. Source files are never changed.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            Picker(String(localized: "Source", table: "ModelCenter"), selection: $importSource) {
                Text(verbatim: "OpenCode").tag("open_code")
                Text(verbatim: "Codex").tag("codex")
                Text(verbatim: "Kimi Code CLI").tag("kimi_code")
            }.onChange(of: importSource) { _, _ in
                invalidateImport(); sourceURL = nil; credentialURL = nil
            }
            Button(String(localized: "Choose configuration file…", table: "ModelCenter")) { chooseSourceFile(credential: false) }
                .accessibilityIdentifier("model.import.choose-source")
            if let sourceURL { Text(sourceURL.lastPathComponent).font(.caption).textSelection(.enabled).help(sourceURL.path) }
            Toggle(String(localized: "Import API keys from the selected source", table: "ModelCenter"), isOn: $includeCredentials)
                .onChange(of: includeCredentials) { _, enabled in
                    invalidateImport()
                    if !enabled { credentialURL = nil; allowEnvironment = false }
                }
            if includeCredentials {
                Button(String(localized: "Choose a separate credentials file…", table: "ModelCenter")) { chooseSourceFile(credential: true) }
                if let credentialURL {
                    HStack { Text(credentialURL.lastPathComponent); Button(String(localized: "Remove selection", table: "ModelCenter")) { self.credentialURL = nil; invalidateImport() } }
                }
                Toggle(String(localized: "Allow explicitly named environment variables", table: "ModelCenter"), isOn: $allowEnvironment)
                    .onChange(of: allowEnvironment) { _, _ in invalidateImport() }
            }
            Text(String(localized: "To use a ChatGPT account, sign in in your browser. Use file import for API connections.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            Button(String(localized: "Sign in with ChatGPT", table: "ModelCenter")) {
                cancelImport(); siwcAccount = nil; showingSiwc = true
            }
            Button(String(localized: "Source login on a Nola Host", table: "ModelCenter")) {
                cancelImport(); workspace = .host
            }
            Button(String(localized: "Preview import", table: "ModelCenter")) { previewImport() }.disabled(sourceURL == nil)
                .accessibilityIdentifier("model.import.preview")
            if busy { ProgressView().controlSize(.small).accessibilityLabel(String(localized: "Import in progress", table: "ModelCenter")) }
            if let plan = importPlan {
                if plan.candidates.isEmpty {
                    Text(String(localized: "No connections were found in this file. Check the selected source and configuration file.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                Text(String(localized: "Choose connections to import", table: "ModelCenter")).font(.headline)
                ForEach(plan.candidates) { candidate in
                    VStack(alignment: .leading, spacing: 6) {
                        Toggle(candidate.connection.display_name, isOn: Binding(get: { importSelection.contains(candidate.id) }, set: { value in
                            if value { importSelection.insert(candidate.id) } else { importSelection.remove(candidate.id) }
                        })).disabled(candidate.issue != nil)
                        Text(candidate.connection.base_url).font(.caption).textSelection(.enabled)
                        Text(candidate.connection.protocols.joined(separator: " · ") + " · " + String(localized: "Models", table: "ModelCenter") + " \(candidate.models.count)")
                            .font(.caption).foregroundStyle(.secondary)
                        Text(importCredentialLabel(candidate.credential_status))
                            .font(.caption).foregroundStyle(.secondary)
                        if let issue = candidate.issue {
                            Text(issue == "api_credential_required"
                                ? String(localized: "API Key", table: "ModelCenter") + " · " + String(localized: "Not configured", table: "ModelCenter")
                                : issue == "subscription_credential_reuse_not_supported"
                                ? String(localized: "Codex and Kimi subscription logins are not API keys. Authorize ChatGPT separately on this Mac; existing Nola ChatGPT authorizations support token refresh.", table: "ModelCenter")
                                : issue)
                                .font(.callout).foregroundStyle(.secondary)
                        }
                        if candidate.existing_revision != nil {
                            Toggle(String(localized: "Update this existing connection", table: "ModelCenter"), isOn: Binding(get: { importUpdates.contains(candidate.id) }, set: { value in
                                if value { importUpdates.insert(candidate.id) } else { importUpdates.remove(candidate.id) }
                            }))
                        }
                    }.padding(.vertical, 4)
                }
                Toggle(String(localized: "Enable imported models for chat", table: "ModelCenter"), isOn: $enableImportedModels)
                Button(String(localized: "Confirm import", table: "ModelCenter")) { confirmImport(plan) }
                    .buttonStyle(.borderedProminent)
                    .disabled(importSelection.isEmpty || plan.candidates.contains { importSelection.contains($0.id) && $0.existing_revision != nil && !importUpdates.contains($0.id) })
                    .accessibilityIdentifier("model.import.confirm")
                Button(String(localized: "Discard preview", table: "ModelCenter")) { invalidateImport() }
            }
            ForEach(importResults) { result in
                Label(result.status == "imported" ? String(localized: "Connection imported", table: "ModelCenter") : String(localized: "Connection not imported", table: "ModelCenter") + ": " + (result.error ?? String(localized: "Refresh the preview and try again", table: "ModelCenter")),
                    systemImage: result.status == "imported" ? "checkmark.circle" : "exclamationmark.triangle")
            }
        }
    }
    private func chooseSourceFile(credential: Bool) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false; panel.canChooseFiles = true; panel.allowsMultipleSelection = false
        panel.message = credential ? String(localized: "Choose the credentials file you authorize Nola to read.", table: "ModelCenter") : String(localized: "Choose the configuration file for the selected source.", table: "ModelCenter")
        panel.prompt = String(localized: "Choose", table: "ModelCenter")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        invalidateImport()
        if credential { credentialURL = url } else { sourceURL = url }
    }
    private func importCredentialLabel(_ status: String) -> String {
        switch status {
        case "available": String(localized: "API key: included", table: "ModelCenter")
        case "not_requested": String(localized: "API key: not requested", table: "ModelCenter")
        default: String(localized: "API key: not found", table: "ModelCenter")
        }
    }

    private func invalidateImport() {
        error = nil; status = nil
        importBridge?.cancel(); importBridge = nil; importPlan = nil
        importSelection = []; importUpdates = []; importResults = []
    }
    private func cancelImport() {
        importBridge?.cancel(); importBridge = nil; importPlan = nil
    }
    private func previewImport() {
        guard let sourceURL else { return }
        invalidateImport()
        perform {
            let bridge = try service.makeImportBridge(); importBridge = bridge
            importPlan = try await bridge.preview(source: importSource, path: sourceURL.path,
                credentialPath: includeCredentials ? credentialURL?.path : nil,
                includeCredentials: includeCredentials, allowEnvironment: includeCredentials && allowEnvironment)
        }
    }
    private func confirmImport(_ plan: MacModelImportPlan) {
        guard let bridge = importBridge else { return }
        perform {
            defer { bridge.cancel(); importBridge = nil; importPlan = nil }
            importResults = try await bridge.commit(planID: plan.plan_id, selected: importSelection,
                updateExisting: importUpdates, enableModels: enableImportedModels)
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            status = String(localized: "Import finished. Review the per-connection results below.", table: "ModelCenter")
        }
    }
    private func reload() {
        do {
            connections = try service.connections(); accounts = try service.accounts()
            let fresh = try Dictionary(uniqueKeysWithValues: connections.map { ($0.id, try service.enabledModels(connectionID: $0.id)) })
            enabled = Self.refreshedSelection(draft: enabled, saved: savedEnabled, fresh: fresh)
            savedEnabled = fresh
            error = nil
        } catch { self.error = error.localizedDescription }
    }
    nonisolated static func refreshedSelection(draft: [String: Set<String>], saved: [String: Set<String>],
        fresh: [String: Set<String>]) -> [String: Set<String>] {
        fresh.mapValues { $0 }.merging(draft.filter { id, value in
            fresh[id] != nil && value != saved[id, default: []]
        }) { _, edited in edited }
    }

    private func perform(_ action: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }
        busy = true; error = nil; status = nil
        operationTask = Task {
            defer { busy = false; operationTask = nil }
            do { try await action(); reload() }
            catch {
                if Task.isCancelled { reload(); if status == nil { status = String(localized: "Operation cancelled. Saved state was refreshed for review.", table: "ModelCenter") } }
                else { self.error = error.localizedDescription }
            }
        }
    }
}

private struct MacModelCapabilityOverrideEditor: View {
    let connection: MacModelConnectionSnapshot
    let model: MacModelConnectionSnapshot.Model
    let service: MacModelConnectionService
    let onSave: () -> Void
    let onError: (String) -> Void
    @State private var values = Values()
    @State private var expanded = false
    @State private var declarationError = false
    private struct Values: Codable {
        var toolCalls = true
        var imageInput = false
        var reasoning = false
        var samplingParameters = false
        var samplingRequiresNoReasoning = false
        var chatOutputTokenField = "auto"
    }
    private var saved: MacCanonicalJSONValue? {
        connection.document.capability_assessments?.first {
            $0.model_id == model.model_id && $0.protocol == model.protocol
        }?.user_override
    }
    var body: some View {
        DisclosureGroup(String(localized: "Explicit capability settings", table: "ModelCenter"), isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 8) {
                Text(String(localized: "These are your explicit settings, not test results. Saving disables unchecked capabilities. Clear settings to use the source declaration.", table: "ModelCenter"))
                    .font(.callout).foregroundStyle(.secondary)
                Toggle(String(localized: "Tool calls", table: "ModelCenter"), isOn: $values.toolCalls)
                Toggle(String(localized: "Image input", table: "ModelCenter"), isOn: $values.imageInput)
                Toggle(String(localized: "Reasoning", table: "ModelCenter"), isOn: $values.reasoning)
                Toggle(String(localized: "Sampling parameters", table: "ModelCenter"), isOn: $values.samplingParameters)
                Toggle(String(localized: "Sampling requires reasoning disabled", table: "ModelCenter"), isOn: $values.samplingRequiresNoReasoning)
                    .disabled(!values.samplingParameters)
                Picker(String(localized: "Output token field", table: "ModelCenter"), selection: $values.chatOutputTokenField) {
                    Text(String(localized: "Protocol default", table: "ModelCenter")).tag("auto")
                    Text("max_tokens").tag("max_tokens")
                    Text("max_completion_tokens").tag("max_completion_tokens")
                }
                HStack {
                    Button(String(localized: "Save explicit settings", table: "ModelCenter")) { save(clear: false) }.disabled(declarationError)
                    Button(String(localized: "Clear settings", table: "ModelCenter")) { save(clear: true) }.disabled(saved == nil)
                }
            }.padding(.vertical, 8)
        }
        .disclosureGroupStyle(MacRowDisclosureStyle())
        .task(id: connection.revision) {
            declarationError = false
            guard let declaration = saved ?? model.execution_capabilities else { values = Values(); return }
            do {
                let decoded = try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                    from: JSONEncoder().encode(declaration))
                values = Values(toolCalls: decoded.toolCalls, imageInput: decoded.imageInput,
                    reasoning: decoded.reasoning, samplingParameters: decoded.samplingParameters,
                    samplingRequiresNoReasoning: decoded.samplingRequiresNoReasoning,
                    chatOutputTokenField: decoded.chatOutputTokenField?.rawValue ?? "auto")
            } catch { declarationError = true; onError(error.localizedDescription) }
        }
        .onChange(of: values.samplingParameters) { _, enabled in
            if !enabled { values.samplingRequiresNoReasoning = false }
        }
    }
    private func save(clear: Bool) {
        do {
            let capabilities = clear ? nil : try JSONDecoder().decode(MacCanonicalJSONValue.self,
                from: JSONEncoder().encode(values))
            _ = try service.setCapabilityOverride(connection, model: model, capabilities: capabilities)
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            onSave()
        } catch { onError(error.localizedDescription) }
    }
}
