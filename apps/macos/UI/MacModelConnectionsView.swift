// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Native macOS Master-Detail workspace for managing remote API model connections, catalogs, credentials, and imports.

import SwiftUI
import AppKit

struct MacModelConnectionsView: View {
    private enum ProviderPreset: String, CaseIterable, Identifiable {
        case openai
        case anthropic
        case deepseek
        case openrouter
        case ollama
        case custom

        var id: String { rawValue }

        var title: String {
            switch self {
            case .openai: "OpenAI"
            case .anthropic: "Anthropic"
            case .deepseek: "DeepSeek"
            case .openrouter: "OpenRouter"
            case .ollama: "Ollama"
            case .custom: String(localized: "Custom", table: "ModelCenter")
            }
        }

        var defaultEndpoint: String {
            switch self {
            case .openai: "https://api.openai.com/v1"
            case .anthropic: "https://api.anthropic.com"
            case .deepseek: "https://api.deepseek.com/v1"
            case .openrouter: "https://openrouter.ai/api/v1"
            case .ollama: "http://127.0.0.1:11434/v1"
            case .custom: ""
            }
        }

        var defaultWire: String {
            switch self {
            case .openai: "responses"
            case .anthropic: "anthropic"
            case .deepseek, .openrouter, .ollama, .custom: "chat_completions"
            }
        }
    }

    private enum ModelFilter: String, CaseIterable, Identifiable {
        case all
        case enabled

        var id: String { rawValue }

        var title: String {
            switch self {
            case .all: String(localized: "All", table: "ModelCenter")
            case .enabled: String(localized: "Enabled", table: "ModelCenter")
            }
        }
    }

    private enum SheetTarget: Hashable, Identifiable {
        case api
        case edit(String)
        case imports
        case accounts
        case host

        var id: String {
            switch self {
            case .api: "api"
            case .edit(let connectionID): "edit:\(connectionID)"
            case .imports: "imports"
            case .accounts: "accounts"
            case .host: "host"
            }
        }
    }

    let service: MacModelConnectionService
    let focusedRemoteProfileID: String?
    let focusRevision: Int
    let pendingAction: MacModelCenterNavigationState.RemoteAction?
    let pendingActionRevision: Int
    private var refreshOnAppear = true

    init(
        service: MacModelConnectionService,
        focusedRemoteProfileID: String? = nil,
        focusRevision: Int = 0,
        pendingAction: MacModelCenterNavigationState.RemoteAction? = nil,
        pendingActionRevision: Int = 0
    ) {
        self.service = service
        self.focusedRemoteProfileID = focusedRemoteProfileID
        self.focusRevision = focusRevision
        self.pendingAction = pendingAction
        self.pendingActionRevision = pendingActionRevision
    }

    @State private var connections: [MacModelConnectionSnapshot] = []
    @State private var accounts: [MacModelAccountSnapshot] = []
    @State private var enabled: [String: Set<String>] = [:]
    @State private var savedEnabled: [String: Set<String>] = [:]
    @State private var selectedConnectionID: String?
    @State private var showsCompactDetail = false
    @State private var hoveredConnectionID: String?
    @State private var search = ""
    @State private var modelSearch = ""
    @State private var modelFilter: ModelFilter = .all
    @State private var expandedModelID: String?
    @State private var copiedEndpointID: String?

    // Add API sheet form state
    @State private var selectedPreset: ProviderPreset = .openai
    @State private var showsAdvancedProtocol = false
    @State private var name = "OpenAI"
    @State private var endpoint = "https://api.openai.com/v1"
    @State private var secret = ""
    @State private var wire = "responses"

    // Edit API sheet form state
    @State private var editName = ""
    @State private var editEndpoint = ""
    @State private var editWire = "chat_completions"
    @State private var editSecret = ""
    @State private var editClearSecret = false

    // Delete & Discard confirmation state
    @State private var deletionTarget: MacModelConnectionSnapshot?
    @State private var confirmsDiscard = false
    @State private var pendingSelectionID: String?

    // Async execution state
    @State private var busy = false
    @State private var busyAccountIDs: Set<String> = []
    @State private var discovering = false
    @State private var operationTask: Task<Void, Never>?
    @State private var reloadTask: Task<Void, Never>?
    @State private var reloadGeneration = 0
    @State private var error: String?
    @State private var status: String?

    // Import & Probe & ChatGPT state
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
    @State private var probeRunning = false
    @State private var managementHost: MacRemoteHostConfiguration?
    @State private var showingSiwc = false
    @State private var siwcAccount: MacModelAccountSnapshot?
    @State private var activeSheet: SheetTarget?
    @State private var handledFocusRevision: Int?
    @State private var handledPendingActionRevision: Int?

    @Environment(\.openURL) private var openURL

    private var selectedConnection: MacModelConnectionSnapshot? {
        if let selectedConnectionID,
           let match = connections.first(where: { $0.id == selectedConnectionID }) {
            return match
        }
        return connections.first
    }

    private var filteredConnections: [MacModelConnectionSnapshot] {
        connections.filter {
            search.isEmpty
                || $0.document.connection.display_name.localizedCaseInsensitiveContains(search)
                || $0.document.connection.base_url.localizedCaseInsensitiveContains(search)
        }
    }

    private var hasModelChanges: Bool {
        guard let id = selectedConnection?.id else { return false }
        return enabled[id, default: []] != savedEnabled[id, default: []]
    }

    private var isEditorBusy: Bool { busy || probeRunning || !busyAccountIDs.isEmpty }

    var body: some View {
        GeometryReader { geometry in
            if connections.isEmpty && !busy {
                emptyWorkspace
            } else if geometry.size.width >= 760 {
                HStack(spacing: 0) {
                    connectionSidebar
                        .frame(width: min(320, max(268, geometry.size.width * 0.34)))
                    Divider()
                    connectionDetailPane
                }
            } else if showsCompactDetail, selectedConnection != nil {
                VStack(spacing: 0) {
                    HStack {
                        Button {
                            showsCompactDetail = false
                        } label: {
                            Label(String(localized: "API Models", table: "ModelCenter"), systemImage: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("api-model-back")
                        Spacer()
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    Divider()
                    connectionDetailPane
                }
            } else {
                connectionSidebar
            }
        }
        .background(MacDesignColor.canvas)
        .tint(MacDesignColor.brand)
        .task {
            if refreshOnAppear {
                await reloadAsync()
                managementHost = MacRemoteHostConfigurationStore.shared.load()
            }
            focusRemoteProfileIfNeeded()
            applyPendingActionIfNeeded()
        }
        .onChange(of: focusRevision) { _, _ in
            if refreshOnAppear && connections.isEmpty { reload() }
            focusRemoteProfileIfNeeded()
        }
        .onChange(of: pendingActionRevision) { _, _ in
            applyPendingActionIfNeeded()
        }
        .onDisappear {
            operationTask?.cancel()
            reloadTask?.cancel()
            cancelImport()
            cancelProbe()
        }
        .confirmationDialog(
            String(localized: "Delete this API connection?", table: "ModelCenter"),
            isPresented: Binding(
                get: { deletionTarget != nil },
                set: { if !$0 { deletionTarget = nil } }
            ),
            presenting: deletionTarget
        ) { target in
            Button(String(localized: "Delete Connection", table: "ModelCenter"), role: .destructive) {
                deleteConnection(target)
            }
            .accessibilityIdentifier("model-connection-confirm-delete")
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) {
                deletionTarget = nil
            }
        } message: { target in
            Text(String(
                format: String(localized: "“%@” and its saved API key will be removed from this Mac, and its enabled models will be removed from the chat picker.", table: "ModelCenter"),
                target.document.connection.display_name
            ))
        }
        .confirmationDialog(
            String(localized: "Discard model selection changes?", table: "ModelCenter"),
            isPresented: $confirmsDiscard
        ) {
            Button(String(localized: "Discard changes", table: "ModelCenter"), role: .destructive) {
                if let id = selectedConnection?.id {
                    enabled[id] = savedEnabled[id, default: []]
                }
                if let next = pendingSelectionID {
                    selectConnectionImmediately(next)
                    pendingSelectionID = nil
                }
            }
            Button(String(localized: "Continue editing", table: "ModelCenter"), role: .cancel) {
                pendingSelectionID = nil
            }
        }
        .sheet(item: $activeSheet) { target in
            sheetContainer(target)
        }
        .sheet(item: $probeConfirmation, onDismiss: { if !probeRunning { cancelProbe() } }) { confirmation in
            probeSheet(confirmation)
        }
        .sheet(isPresented: $showingSiwc, onDismiss: reload) {
            MacSiwcSignInView(
                application: service.application,
                account: siwcAccount,
                showAccounts: {
                    showingSiwc = false
                    activeSheet = .accounts
                    reload()
                },
                saved: { connectionID in
                    showingSiwc = false
                    activeSheet = nil
                    reload()
                    selectedConnectionID = connectionID
                    showsCompactDetail = true
                }
            )
        }
    }

    // MARK: - Empty State

    private var emptyWorkspace: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                bannerStack
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 12) {
                        Image(systemName: "network")
                            .font(.system(size: 22, weight: .semibold))
                            .foregroundStyle(MacDesignColor.brand)
                            .frame(width: 44, height: 44)
                            .background(MacDesignColor.brand.opacity(0.10), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                        VStack(alignment: .leading, spacing: 4) {
                            Text(String(localized: "Connect Remote API Models", table: "ModelCenter"))
                                .font(.title3.weight(.semibold))
                            Text(String(localized: "Add OpenAI, Anthropic, DeepSeek, OpenRouter, Ollama, or any OpenAI-compatible LAN server.", table: "ModelCenter"))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                    }

                    HStack(spacing: 10) {
                        Button {
                            openSheet(.api)
                        } label: {
                            Label(String(localized: "Connect an API account", table: "ModelCenter"), systemImage: "plus")
                        }
                        .buttonStyle(.nolaProminent)
                        .accessibilityIdentifier("model.add.api")

                        Button {
                            openSheet(.imports)
                        } label: {
                            Label(String(localized: "Import from this Mac", table: "ModelCenter"), systemImage: "square.and.arrow.down")
                        }
                        .accessibilityIdentifier("model.add.import")

                        Button {
                            openSheet(.accounts)
                        } label: {
                            Label(String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter"), systemImage: "person.crop.circle")
                        }
                        .accessibilityIdentifier("model.add.chatgpt")
                    }
                }
                .padding(24)
                .frame(maxWidth: 720, alignment: .leading)
                .macPanel()
            }
            .padding(28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - Left Sidebar

    private var connectionSidebar: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 12.5))
                        .foregroundStyle(.secondary)
                    TextField(String(localized: "Search connections", table: "ModelCenter"), text: $search)
                        .textFieldStyle(.plain)
                    if !search.isEmpty {
                        Button { search = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 9)
                .padding(.vertical, 6)
                .background(
                    MacDesignColor.subtleFill,
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                )

                Button { reload() } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 12.5, weight: .medium))
                }
                .buttonStyle(.borderless)
                .disabled(busy)
                .help(String(localized: "Refresh connections", table: "ModelCenter"))
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
            .padding(.bottom, 10)

            VStack(spacing: 6) {
                Button {
                    openSheet(.api)
                } label: {
                    Label(String(localized: "Connect an API account", table: "ModelCenter"), systemImage: "plus.circle.fill")
                        .font(.callout.weight(.medium))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 7)
                        .background(
                            MacDesignColor.brand.opacity(0.11),
                            in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                        )
                        .foregroundStyle(MacDesignColor.brand)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("model.add.api")

                HStack(spacing: 6) {
                    Button {
                        openSheet(.imports)
                    } label: {
                        Label(String(localized: "Import", table: "ModelCenter"), systemImage: "square.and.arrow.down")
                            .font(.caption.weight(.medium))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 5)
                            .background(
                                MacDesignColor.subtleFill,
                                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                            )
                    }
                    .buttonStyle(.plain)
                    .help(String(localized: "Import from this Mac", table: "ModelCenter"))
                    .accessibilityIdentifier("model.add.import")

                    Button {
                        openSheet(.accounts)
                    } label: {
                        Label(String(localized: "ChatGPT", table: "ModelCenter"), systemImage: "person.crop.circle")
                            .font(.caption.weight(.medium))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 5)
                            .background(
                                MacDesignColor.subtleFill,
                                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                            )
                    }
                    .buttonStyle(.plain)
                    .help(String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter"))
                    .accessibilityIdentifier("model.add.chatgpt")
                }
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 10)

            Divider()

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    if filteredConnections.isEmpty {
                        Text(String(localized: "No matching connections", table: "ModelCenter"))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .center)
                            .padding(.vertical, 28)
                    } else {
                        ForEach(filteredConnections) { connection in
                            sidebarConnectionRow(connection)
                        }
                    }
                }
                .padding(10)
            }
        }
        .background(MacDesignColor.sidebar.opacity(0.45))
    }

    private func sidebarConnectionRow(_ snapshot: MacModelConnectionSnapshot) -> some View {
        let connection = snapshot.document.connection
        let isSelected = selectedConnection?.id == snapshot.id
        let isHovered = hoveredConnectionID == snapshot.id
        let totalModels = snapshot.document.catalog?.models.count ?? 0
        let enabledCount = savedEnabled[snapshot.id]?.count ?? 0

        return Button {
            requestSelectConnection(snapshot.id)
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Image(systemName: connection.source == "nola_chatgpt" ? "person.crop.circle.fill" : "network")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(connection.enabled ? MacDesignColor.brand : .secondary)
                        .frame(width: 24, height: 24)
                        .background(
                            (connection.enabled ? MacDesignColor.brand : Color.secondary).opacity(0.10),
                            in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                        )
                    Text(connection.display_name)
                        .font(.body.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Circle()
                        .fill(connection.enabled ? MacDesignColor.success : Color.secondary.opacity(0.45))
                        .frame(width: 7, height: 7)
                        .help(connection.enabled ? String(localized: "Enabled") : String(localized: "Disabled"))
                }

                Text(Self.displayHost(connection.base_url))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)

                HStack(spacing: 6) {
                    if let proto = connection.protocols.first {
                        Text(Self.shortProtocolTitle(proto))
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(MacDesignColor.subtleFill, in: Capsule())
                    }
                    Spacer(minLength: 4)
                    if totalModels > 0 {
                        Text("\(enabledCount)/\(totalModels) " + String(localized: "Models", table: "ModelCenter"))
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(enabledCount > 0 ? MacDesignColor.brand : .secondary)
                    } else {
                        Text(String(localized: "Not discovered", table: "ModelCenter"))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                isSelected
                    ? MacDesignColor.brand.opacity(0.10)
                    : (isHovered ? MacDesignColor.hover : Color.clear),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
            )
            .overlay {
                if isSelected {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(MacDesignColor.brand.opacity(0.32), lineWidth: 1)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hoveredConnectionID = $0 ? snapshot.id : nil }
        .accessibilityAddTraits(isSelected ? [.isSelected] : [])
        .accessibilityIdentifier("model-connection-" + snapshot.id)
        .contextMenu {
            if connection.enabled && connection.source != "nola_chatgpt" {
                Button {
                    selectedConnectionID = snapshot.id
                    discoverModels(for: snapshot)
                } label: {
                    Label(String(localized: "Discover models", table: "ModelCenter"), systemImage: "arrow.triangle.2.circlepath")
                }
            }
            if connection.source != "nola_chatgpt" {
                Button {
                    openEditSheet(for: snapshot)
                } label: {
                    Label(String(localized: "Edit connection…", table: "ModelCenter"), systemImage: "pencil")
                }
            }
            if connection.enabled {
                Button {
                    toggleEnabled(for: snapshot)
                } label: {
                    Label(String(localized: "Disconnect", table: "ModelCenter"), systemImage: "pause.circle")
                }
            } else {
                Button {
                    toggleEnabled(for: snapshot)
                } label: {
                    Label(String(localized: "Reconnect", table: "ModelCenter"), systemImage: "play.circle")
                }
            }
            Divider()
            Button(role: .destructive) {
                deletionTarget = snapshot
            } label: {
                Label(String(localized: "Delete Connection…", table: "ModelCenter"), systemImage: "trash")
            }
        }
    }

    // MARK: - Right Detail Pane

    @ViewBuilder
    private var connectionDetailPane: some View {
        if let connection = selectedConnection {
            VStack(spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        bannerStack
                        connectionHeaderCard(connection)

                        if connection.document.connection.enabled {
                            if connection.document.connection.source == "nola_chatgpt" {
                                if let account = accounts.first(where: { $0.id == connection.document.connection.account_id }) {
                                    MacSiwcAccountManagementView(
                                        application: service.application,
                                        account: account,
                                        changed: reload,
                                        modelsDiscovered: { id in selectedConnectionID = id },
                                        reauthorize: { siwcAccount = account; showingSiwc = true },
                                        activityChanged: { active in
                                            if active { busyAccountIDs.insert(account.id) }
                                            else { busyAccountIDs.remove(account.id) }
                                        }
                                    )
                                    .padding(16)
                                    .macPanel()
                                }
                            }
                            catalogSection(connection)
                        } else {
                            disabledConnectionBanner(connection)
                        }
                    }
                    .padding(20)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                if connection.document.connection.enabled && hasModelChanges {
                    unsavedSelectionFooter(connection)
                }
            }
        } else {
            VStack(spacing: 12) {
                Image(systemName: "network")
                    .font(.system(size: 28))
                    .foregroundStyle(.secondary)
                Text(String(localized: "Select a connection to manage its models.", table: "ModelCenter"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private var bannerStack: some View {
        if let error {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(MacDesignColor.danger)
                    .padding(.top, 1)
                VStack(alignment: .leading, spacing: 6) {
                    Text(error)
                        .font(.callout)
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    if error.localizedCaseInsensitiveContains("local network") {
                        Button(String(localized: "Open Local Network Settings", table: "ModelCenter")) {
                            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork") {
                                openURL(url)
                            }
                        }
                        .controlSize(.small)
                    }
                }
                Spacer(minLength: 8)
                Button { self.error = nil } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
            .padding(12)
            .background(
                MacDesignColor.danger.opacity(0.10),
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                    .strokeBorder(MacDesignColor.danger.opacity(0.28), lineWidth: 1)
            }
        }
        if let status {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(MacDesignColor.success)
                    .padding(.top, 1)
                Text(status)
                    .font(.callout)
                    .foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                Button { self.status = nil } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
            .padding(12)
            .background(
                MacDesignColor.success.opacity(0.09),
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                    .strokeBorder(MacDesignColor.success.opacity(0.25), lineWidth: 1)
            }
        }
    }

    private func connectionHeaderCard(_ snapshot: MacModelConnectionSnapshot) -> some View {
        let connection = snapshot.document.connection
        let account = connection.account_id.flatMap { id in accounts.first(where: { $0.id == id }) }
        let wireLabel = connection.protocols.first.map(Self.protocolTitle) ?? "Chat Completions"

        return VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: connection.source == "nola_chatgpt" ? "person.crop.circle.fill" : "network")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(connection.enabled ? MacDesignColor.brand : .secondary)
                    .frame(width: 40, height: 40)
                    .background(
                        (connection.enabled ? MacDesignColor.brand : Color.secondary).opacity(0.11),
                        in: RoundedRectangle(cornerRadius: 11, style: .continuous)
                    )

                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 8) {
                        Text(connection.display_name)
                            .font(.title3.weight(.semibold))
                            .lineLimit(1)
                        statusPill(enabled: connection.enabled)
                        Text(wireLabel)
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(MacDesignColor.subtleFill, in: Capsule())
                    }

                    HStack(spacing: 6) {
                        Text(connection.base_url)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help(connection.base_url)
                        Button {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(connection.base_url, forType: .string)
                            copiedEndpointID = snapshot.id
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                                if copiedEndpointID == snapshot.id { copiedEndpointID = nil }
                            }
                        } label: {
                            Image(systemName: copiedEndpointID == snapshot.id ? "checkmark" : "doc.on.doc")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .help(String(localized: "Copy endpoint URL", table: "ModelCenter"))
                    }

                    HStack(spacing: 12) {
                        if let account {
                            Label(
                                account.account.source == "nola_chatgpt"
                                    ? account.account.display_name
                                    : String(localized: "API key stored in Keychain", table: "ModelCenter"),
                                systemImage: account.account.source == "nola_chatgpt" ? "person.crop.circle" : "key.fill"
                            )
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        } else {
                            Label(String(localized: "Anonymous endpoint (no API key)", table: "ModelCenter"), systemImage: "lock.open")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        if let observedAt = snapshot.document.catalog?.observed_at_ms, observedAt > 0 {
                            Label(
                                Date(timeIntervalSince1970: Double(observedAt) / 1000).formatted(date: .abbreviated, time: .shortened),
                                systemImage: "clock"
                            )
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        }
                    }
                }
                Spacer(minLength: 0)
            }

            Divider().opacity(0.6)

            HStack(spacing: 8) {
                if connection.enabled && connection.source != "nola_chatgpt" {
                    if discovering {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text(String(localized: "Discovering models…", table: "ModelCenter"))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                            Button(String(localized: "Cancel", table: "ModelCenter")) {
                                operationTask?.cancel()
                            }
                            .controlSize(.small)
                        }
                    } else {
                        Button {
                            discoverModels(for: snapshot)
                        } label: {
                            Label(String(localized: "Discover models", table: "ModelCenter"), systemImage: "arrow.triangle.2.circlepath")
                        }
                        .buttonStyle(.nolaProminent)
                        .disabled(busy)
                        .accessibilityIdentifier("model-connection-discover")
                    }
                }

                if connection.source != "nola_chatgpt" {
                    Button {
                        openEditSheet(for: snapshot)
                    } label: {
                        Label(String(localized: "Edit", table: "ModelCenter"), systemImage: "pencil")
                    }
                    .disabled(busy)
                    .accessibilityIdentifier("model-connection-edit")
                }

                Button {
                    toggleEnabled(for: snapshot)
                } label: {
                    Label(
                        connection.enabled
                            ? String(localized: "Disconnect", table: "ModelCenter")
                            : String(localized: "Reconnect", table: "ModelCenter"),
                        systemImage: connection.enabled ? "pause.circle" : "play.circle"
                    )
                }
                .disabled(busy)
                .accessibilityIdentifier("model-connection-toggle-enabled")

                Spacer(minLength: 0)

                Button(role: .destructive) {
                    deletionTarget = snapshot
                } label: {
                    Label(String(localized: "Delete", table: "ModelCenter"), systemImage: "trash")
                        .foregroundStyle(MacDesignColor.danger)
                }
                .disabled(busy)
                .accessibilityIdentifier("model-connection-delete")
            }
        }
        .padding(18)
        .macPanel()
    }

    private func statusPill(enabled: Bool) -> some View {
        HStack(spacing: 5) {
            Circle()
                .fill(enabled ? MacDesignColor.success : Color.secondary)
                .frame(width: 6, height: 6)
            Text(enabled ? String(localized: "Enabled") : String(localized: "Disabled"))
                .font(.caption.weight(.medium))
                .foregroundStyle(enabled ? MacDesignColor.success : .secondary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(
            (enabled ? MacDesignColor.success : Color.secondary).opacity(0.10),
            in: Capsule()
        )
    }

    private func disabledConnectionBanner(_ snapshot: MacModelConnectionSnapshot) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "pause.circle.fill")
                .font(.title3)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 3) {
                Text(String(localized: "Connection is paused", table: "ModelCenter"))
                    .font(.body.weight(.semibold))
                Text(String(localized: "This connection is disabled. Reconnect it to discover or use its models in chat.", table: "ModelCenter"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Button(String(localized: "Reconnect", table: "ModelCenter")) {
                toggleEnabled(for: snapshot)
            }
            .buttonStyle(.nolaProminent)
            .disabled(busy)
        }
        .padding(16)
        .macPanel()
    }

    private func catalogSection(_ connection: MacModelConnectionSnapshot) -> some View {
        let allModels = connection.document.catalog?.models ?? []
        let enabledSet = enabled[connection.id, default: []]
        let filteredModels = allModels.filter { model in
            if modelFilter == .enabled && !enabledSet.contains(model.id) {
                return false
            }
            if modelSearch.isEmpty { return true }
            return model.display_name.localizedCaseInsensitiveContains(modelSearch)
                || model.model_id.localizedCaseInsensitiveContains(modelSearch)
        }

        return VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                Text(String(localized: "Models", table: "ModelCenter"))
                    .font(.headline)
                if !allModels.isEmpty {
                    Text("\(enabledSet.count) / \(allModels.count)")
                        .font(.caption.monospacedDigit().weight(.medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(MacDesignColor.subtleFill, in: Capsule())
                }
                Spacer(minLength: 8)
                if !allModels.isEmpty {
                    Picker(String(localized: "Filter", table: "ModelCenter"), selection: $modelFilter) {
                        ForEach(ModelFilter.allCases) { option in
                            Text(option.title).tag(option)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(width: 148)
                }
            }

            if connection.document.connection.source == "nola_chatgpt",
               allModels.contains(where: { $0.reasoning_levels.contains("ultra") }) {
                Text(String(localized: "Ultra is a ChatGPT multi-agent mode, not a model reasoning effort. Nola uses the single-model reasoning levels available for each model.", table: "ModelCenter"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if allModels.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Text(String(localized: "No models discovered yet", table: "ModelCenter"))
                        .font(.body.weight(.medium))
                    Text(String(localized: "Click “Discover models” above to query the endpoint’s model list without running inference, then toggle the models you want available in chat.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .macPanel()
            } else {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                    TextField(String(localized: "Search models", table: "ModelCenter"), text: $modelSearch)
                        .textFieldStyle(.plain)
                    if !modelSearch.isEmpty {
                        Button { modelSearch = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                    if filteredModels.count != allModels.count {
                        Text("\(filteredModels.count)")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(
                    MacDesignColor.subtleFill,
                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                )

                if filteredModels.isEmpty {
                    Text(modelFilter == .enabled
                        ? String(localized: "No enabled models match your filter.", table: "ModelCenter")
                        : String(localized: "No matching models", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.vertical, 24)
                } else {
                    // LazyVStack keeps 477+ OpenRouter models silky smooth at 60fps.
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(filteredModels) { model in
                            let isOn = enabledSet.contains(model.id)
                            let isExpanded = expandedModelID == model.id
                            let hasEvidence = connection.document.capability_assessments?.contains(where: {
                                $0.model_id == model.model_id && $0.protocol == model.protocol && $0.evidence != nil
                            }) ?? false
                            ConnectionModelRow(
                                model: model,
                                isEnabled: isOn,
                                isExpanded: isExpanded,
                                hasVerifiedEvidence: hasEvidence,
                                onToggle: { value in
                                    if value { enabled[connection.id, default: []].insert(model.id) }
                                    else { enabled[connection.id]?.remove(model.id) }
                                },
                                onToggleExpand: {
                                    expandedModelID = isExpanded ? nil : model.id
                                },
                                expandedContent: {
                                    if connection.document.connection.source == "nola_chatgpt" {
                                        MacModelCapabilityOverrideEditor(
                                            connection: connection,
                                            model: model,
                                            service: service,
                                            onSave: reload,
                                            onError: { error = $0 }
                                        )
                                    } else {
                                        modelActions(connection, model: model)
                                    }
                                }
                            )
                        }
                    }
                }
            }
        }
    }

    private func unsavedSelectionFooter(_ connection: MacModelConnectionSnapshot) -> some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) {
                let count = enabled[connection.id, default: []].count
                Label(
                    String(
                        format: String(localized: "Unsaved model changes (%d selected)", table: "ModelCenter"),
                        count
                    ),
                    systemImage: "circle.badge.exclamationmark"
                )
                .font(.callout.weight(.medium))
                .foregroundStyle(.primary)
                Spacer()
                Button(String(localized: "Revert", table: "ModelCenter")) {
                    enabled[connection.id] = savedEnabled[connection.id, default: []]
                }
                .disabled(busy)
                Button(String(localized: "Apply enabled models", table: "ModelCenter")) {
                    let chosen = enabled[connection.id, default: []]
                    perform {
                        try await Task.detached(priority: .userInitiated) { [service] in
                            try service.enable(connection, modelIDs: chosen)
                        }.value
                        savedEnabled[connection.id] = chosen
                        status = String(localized: "Enabled models are available in the chat model picker.", table: "ModelCenter")
                    }
                }
                .buttonStyle(.nolaProminent)
                .disabled(busy)
                .accessibilityIdentifier("model-connection-apply")
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .background(.regularMaterial)
        }
    }

    // MARK: - Modal Sheets (Add / Edit / Import / Accounts / Host)

    private func sheetContainer(_ target: SheetTarget) -> some View {
        VStack(spacing: 0) {
            HStack {
                Text(sheetTitle(target)).font(.headline)
                Spacer()
                Button(String(localized: "Close", table: "ModelCenter")) {
                    if !isEditorBusy { activeSheet = nil }
                }
                .keyboardShortcut(.cancelAction)
                .disabled(isEditorBusy)
                .accessibilityIdentifier("model-editor-close")
            }
            .padding(20)
            Divider()
            Form {
                if let error {
                    Text(error)
                        .foregroundStyle(MacDesignColor.danger)
                        .textSelection(.enabled)
                }
                if let status {
                    Text(status)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                switch target {
                case .api:
                    apiConnectionSection
                case .edit(let connectionID):
                    if let snapshot = connections.first(where: { $0.id == connectionID }) {
                        editConnectionSection(snapshot)
                    } else {
                        Text(String(localized: "Connection is no longer available.", table: "ModelCenter"))
                            .foregroundStyle(.secondary)
                    }
                case .imports:
                    sourceImportSection
                case .accounts:
                    accountsSection
                case .host:
                    hostManagementSection
                }
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
            .background(MacDesignColor.canvas)
            .disabled(busy)
            .overlay(alignment: .topTrailing) {
                if busy {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        if importBridge != nil {
                            Button(String(localized: "Cancel import", table: "ModelCenter")) { cancelImport() }
                        }
                    }
                    .padding(10)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding(12)
                }
            }
        }
        .frame(minWidth: 480, idealWidth: 560, minHeight: 420, idealHeight: 580)
        .interactiveDismissDisabled(isEditorBusy)
    }

    private func sheetTitle(_ target: SheetTarget) -> String {
        switch target {
        case .api: String(localized: "Connect an API account", table: "ModelCenter")
        case .edit: String(localized: "Edit API Connection", table: "ModelCenter")
        case .imports: String(localized: "Import from this Mac", table: "ModelCenter")
        case .accounts: String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter")
        case .host: String(localized: "Source login on a Nola Host", table: "ModelCenter")
        }
    }

    private var apiConnectionSection: some View {
        Group {
            Section(String(localized: "Provider preset", table: "ModelCenter")) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 108), spacing: 8)], alignment: .leading, spacing: 8) {
                    ForEach(ProviderPreset.allCases) { preset in
                        let isSelected = selectedPreset == preset
                        Button {
                            applyPreset(preset)
                        } label: {
                            Text(preset.title)
                                .font(.system(size: 12.5, weight: isSelected ? .semibold : .medium))
                                .foregroundStyle(isSelected ? MacDesignColor.brand : .primary)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 7)
                                .padding(.horizontal, 10)
                                .background(
                                    isSelected ? MacDesignColor.brand.opacity(0.12) : MacDesignColor.subtleFill,
                                    in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                                )
                                .overlay {
                                    RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                                        .strokeBorder(
                                            isSelected ? MacDesignColor.brand : MacDesignColor.borderSubtle,
                                            lineWidth: isSelected ? 1.25 : 0.75
                                        )
                                }
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(isSelected ? .isSelected : [])
                        .accessibilityIdentifier("model.preset." + preset.rawValue)
                    }
                }
                .padding(.vertical, 2)
            }

            Section {
                TextField(String(localized: "Connection name", table: "ModelCenter"), text: $name)
                TextField(
                    String(localized: "API endpoint", table: "ModelCenter"),
                    text: $endpoint,
                    prompt: Text("https://api.example.com/v1")
                )
                .autocorrectionDisabled()
                SecureField(
                    String(localized: "API key · optional for anonymous endpoints", table: "ModelCenter"),
                    text: $secret
                )
                DisclosureGroup(String(localized: "Advanced", table: "ModelCenter"), isExpanded: $showsAdvancedProtocol) {
                    Picker(String(localized: "Protocol", table: "ModelCenter"), selection: $wire) {
                        Text(verbatim: "Responses").tag("responses")
                        Text(verbatim: "Chat Completions").tag("chat_completions")
                        Text(verbatim: "Anthropic Messages").tag("anthropic")
                    }
                    .padding(.top, 4)
                }
                if !endpoint.isEmpty && !Self.isValidEndpoint(endpoint) {
                    Text(String(localized: "Enter an HTTP or HTTPS address without embedded credentials.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(MacDesignColor.danger)
                }
                HStack {
                    Spacer()
                    Button(String(localized: "Connect account", table: "ModelCenter")) {
                        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
                        let trimmedEndpoint = endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
                        let selectedWire = wire
                        let enteredSecret = secret
                        perform {
                            let created = try await Task.detached(priority: .userInitiated) { [service] in
                                try service.create(
                                    name: trimmedName,
                                    endpoint: trimmedEndpoint,
                                    wireProtocol: selectedWire,
                                    secret: enteredSecret
                                )
                            }.value
                            secret = ""
                            applyPreset(.openai)
                            selectedConnectionID = created.id
                            showsCompactDetail = true
                            activeSheet = nil
                            status = String(localized: "Connection saved. Discover and enable models below.", table: "ModelCenter")
                        }
                    }
                    .buttonStyle(.nolaProminent)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !Self.isValidEndpoint(endpoint))
                }
            }
        }
    }

    private func editConnectionSection(_ snapshot: MacModelConnectionSnapshot) -> some View {
        let old = snapshot.document.connection
        let routeWillChange = editEndpoint.trimmingCharacters(in: .whitespacesAndNewlines) != old.base_url
            || old.protocols != [editWire]
        let hasExistingAccount = old.account_id != nil

        return Group {
            Section(String(localized: "Connection details", table: "ModelCenter")) {
                TextField(String(localized: "Connection name", table: "ModelCenter"), text: $editName)
                TextField(
                    String(localized: "API endpoint", table: "ModelCenter"),
                    text: $editEndpoint,
                    prompt: Text("https://api.example.com/v1")
                )
                .autocorrectionDisabled()
                Picker(String(localized: "Protocol", table: "ModelCenter"), selection: $editWire) {
                    Text(verbatim: "Responses").tag("responses")
                    Text(verbatim: "Chat Completions").tag("chat_completions")
                    Text(verbatim: "Anthropic Messages").tag("anthropic")
                }
                if !editEndpoint.isEmpty && !Self.isValidEndpoint(editEndpoint) {
                    Text(String(localized: "Enter an HTTP or HTTPS address without embedded credentials.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(MacDesignColor.danger)
                }
                if routeWillChange {
                    Label(
                        String(localized: "Changing the endpoint or protocol resets discovered models so you can re-discover them for the new route.", table: "ModelCenter"),
                        systemImage: "info.circle"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }

            Section(String(localized: "API Key", table: "ModelCenter")) {
                SecureField(
                    hasExistingAccount
                        ? String(localized: "Leave blank to keep existing API key", table: "ModelCenter")
                        : String(localized: "Enter API key (optional)", table: "ModelCenter"),
                    text: $editSecret
                )
                .disabled(editClearSecret)
                if hasExistingAccount {
                    Toggle(String(localized: "Remove saved API key (switch to anonymous)", table: "ModelCenter"), isOn: $editClearSecret)
                        .toggleStyle(.macSwitch)
                        .onChange(of: editClearSecret) { _, cleared in
                            if cleared { editSecret = "" }
                        }
                }
            }

            Section {
                HStack {
                    Button(role: .destructive) {
                        activeSheet = nil
                        deletionTarget = snapshot
                    } label: {
                        Label(String(localized: "Delete Connection…", table: "ModelCenter"), systemImage: "trash")
                    }
                    Spacer()
                    Button(String(localized: "Save changes", table: "ModelCenter")) {
                        let newName = editName.trimmingCharacters(in: .whitespacesAndNewlines)
                        let newEndpoint = editEndpoint.trimmingCharacters(in: .whitespacesAndNewlines)
                        let newWire = editWire
                        let newSecret = editSecret
                        let clearKey = editClearSecret
                        perform {
                            let updated = try await Task.detached(priority: .userInitiated) { [service] in
                                try service.update(
                                    snapshot,
                                    name: newName,
                                    endpoint: newEndpoint,
                                    wireProtocol: newWire,
                                    secret: newSecret.isEmpty ? nil : newSecret,
                                    clearSecret: clearKey
                                )
                            }.value
                            editSecret = ""
                            editClearSecret = false
                            selectedConnectionID = updated.id
                            activeSheet = nil
                            status = routeWillChange
                                ? String(localized: "Connection updated. Click “Discover models” to refresh the model catalog.", table: "ModelCenter")
                                : String(localized: "Connection updated.", table: "ModelCenter")
                        }
                    }
                    .buttonStyle(.nolaProminent)
                    .disabled(editName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !Self.isValidEndpoint(editEndpoint))
                }
            }
        }
    }

    private var accountsSection: some View {
        Section(String(localized: "ChatGPT authorization on this Mac", table: "ModelCenter")) {
            Text(String(localized: "Connect your ChatGPT account, then choose the models to use on this Mac.", table: "ModelCenter"))
                .font(.callout)
                .foregroundStyle(.secondary)
            Button(String(localized: "Sign in with ChatGPT", table: "ModelCenter")) {
                siwcAccount = nil
                showingSiwc = true
            }
            .buttonStyle(.nolaProminent)
            ForEach(accounts.filter { $0.account.source == "nola_chatgpt" }) { account in
                MacSiwcAccountManagementView(
                    application: service.application,
                    account: account,
                    changed: reload,
                    modelsDiscovered: { id in
                        selectedConnectionID = id
                        activeSheet = nil
                    },
                    reauthorize: { siwcAccount = account; showingSiwc = true },
                    activityChanged: { active in
                        if active { busyAccountIDs.insert(account.id) }
                        else { busyAccountIDs.remove(account.id) }
                    }
                )
            }
        }
    }

    // MARK: - Actions

    private func openSheet(_ target: SheetTarget) {
        error = nil
        status = nil
        activeSheet = target
    }

    private func openEditSheet(for snapshot: MacModelConnectionSnapshot) {
        let conn = snapshot.document.connection
        editName = conn.display_name
        editEndpoint = conn.base_url
        editWire = conn.protocols.first ?? "chat_completions"
        editSecret = ""
        editClearSecret = false
        openSheet(.edit(snapshot.id))
    }

    private func requestSelectConnection(_ id: String) {
        if id == selectedConnection?.id {
            showsCompactDetail = true
            return
        }
        if hasModelChanges {
            pendingSelectionID = id
            confirmsDiscard = true
        } else {
            selectConnectionImmediately(id)
        }
    }

    private func selectConnectionImmediately(_ id: String) {
        selectedConnectionID = id
        showsCompactDetail = true
        modelSearch = ""
        modelFilter = .all
        expandedModelID = nil
        error = nil
        status = nil
    }

    private func discoverModels(for connection: MacModelConnectionSnapshot) {
        perform {
            discovering = true
            defer { discovering = false }
            let updated = try await service.discover(connection)
            let count = updated.document.catalog?.models.count ?? 0
            status = String(
                format: String(localized: "Discovered %d models. Toggle the ones you want and click Apply.", table: "ModelCenter"),
                count
            )
        }
    }

    private func toggleEnabled(for connection: MacModelConnectionSnapshot) {
        let wasEnabled = connection.document.connection.enabled
        perform {
            _ = try await Task.detached(priority: .userInitiated) { [service] in
                if wasEnabled {
                    return try service.disconnect(connection)
                } else {
                    return try service.reconnect(connection)
                }
            }.value
            status = wasEnabled
                ? String(localized: "Connection disabled. Credentials were not deleted.", table: "ModelCenter")
                : String(localized: "Connection enabled. Discover or enable models below.", table: "ModelCenter")
        }
    }

    private func deleteConnection(_ snapshot: MacModelConnectionSnapshot) {
        deletionTarget = nil
        let removedID = snapshot.id
        let removedName = snapshot.document.connection.display_name
        perform {
            try await Task.detached(priority: .userInitiated) { [service] in
                try service.delete(snapshot, deleteAccount: true)
            }.value
            enabled[removedID] = nil
            savedEnabled[removedID] = nil
            if selectedConnectionID == removedID {
                selectedConnectionID = nil
                showsCompactDetail = false
            }
            status = String(
                format: String(localized: "Deleted “%@”.", table: "ModelCenter"),
                removedName
            )
        }
    }

    private func applyPreset(_ preset: ProviderPreset) {
        selectedPreset = preset
        if preset == .custom {
            name = ""
            endpoint = ""
            wire = preset.defaultWire
            showsAdvancedProtocol = true
        } else {
            name = preset.title
            endpoint = preset.defaultEndpoint
            wire = preset.defaultWire
            showsAdvancedProtocol = false
        }
    }

    private func applyPendingActionIfNeeded() {
        guard let pendingAction, handledPendingActionRevision != pendingActionRevision else { return }
        handledPendingActionRevision = pendingActionRevision
        switch pendingAction {
        case .api:
            openSheet(.api)
        case .accounts:
            openSheet(.accounts)
        }
    }

    nonisolated static func protocolTitle(_ value: String) -> String {
        switch value {
        case "responses": "Responses"
        case "chat_completions": "Chat Completions"
        case "anthropic": "Anthropic Messages"
        default: value
        }
    }

    nonisolated static func shortProtocolTitle(_ value: String) -> String {
        switch value {
        case "responses": "Responses"
        case "chat_completions": "Chat"
        case "anthropic": "Anthropic"
        case "siwc_responses": "ChatGPT"
        default: value
        }
    }

    nonisolated static func displayHost(_ endpoint: String) -> String {
        guard let url = URL(string: endpoint), let host = url.host else { return endpoint }
        if let port = url.port {
            return "\(host):\(port)\(url.path)"
        }
        return "\(host)\(url.path)"
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
                Button(Self.isLocalManagementHost(host.baseURL)
                    ? String(localized: "Open Host source and API management", table: "ModelCenter")
                    : String(localized: "Open remote Host API management", table: "ModelCenter")) {
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
        VStack(alignment: .leading, spacing: 10) {
            if let evidence = connection.document.capability_assessments?.first(where: {
                $0.model_id == model.model_id && $0.protocol == model.protocol
            })?.evidence {
                HStack(spacing: 6) {
                    Label(String(localized: "A previous text test succeeded", table: "ModelCenter"), systemImage: "checkmark.seal.fill")
                        .font(.callout)
                        .foregroundStyle(MacDesignColor.success)
                    Text(Date(timeIntervalSince1970: Double(evidence.observed_at_ms) / 1000), style: .date)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Button(String(localized: "Test text response…", table: "ModelCenter")) {
                perform {
                    let bridge = try service.makeImportBridge()
                    probeBridge = bridge
                    do {
                        probeConfirmation = try await bridge.prepareProbe(snapshot: connection, model: model)
                    } catch { bridge.cancel(); probeBridge = nil; throw error }
                }
            }
            MacModelCapabilityOverrideEditor(
                connection: connection,
                model: model,
                service: service,
                onSave: { reload() },
                onError: { error = $0 }
            )
        }
    }

    private func probeSheet(_ confirmation: MacModelProbeConfirmation) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(String(localized: "Test this model?", table: "ModelCenter")).font(.title2).bold()
            LabeledContent(String(localized: "Model", table: "ModelCenter"), value: confirmation.model_id)
            LabeledContent(String(localized: "Protocol", table: "ModelCenter"), value: confirmation.protocol)
            LabeledContent(
                String(localized: "Account", table: "ModelCenter"),
                value: accounts.first(where: { $0.id == confirmation.account_id })?.account.display_name
                    ?? confirmation.account_id
                    ?? String(localized: "Anonymous endpoint", table: "ModelCenter")
            )
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
                    .buttonStyle(.nolaProminent).disabled(probeRunning)
            }
        }
        .padding(24).frame(width: 440)
        .background(MacDesignColor.canvas)
        .interactiveDismissDisabled(probeRunning)
    }

    private func cancelProbe() {
        probeBridge?.cancel(); probeBridge = nil; probeConfirmation = nil
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
                probeRunning = false
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
        return view.connectionDetailPane.tint(MacDesignColor.brand)
    }

    static func visualFixture(service: MacModelConnectionService,
                              connections: [MacModelConnectionSnapshot] = [],
                              busy: Bool = false,
                              error: String? = nil) -> Self {
        var view = Self(service: service)
        view.refreshOnAppear = false
        view._connections = State(initialValue: connections)
        view._selectedConnectionID = State(initialValue: connections.first?.id)
        view._busy = State(initialValue: busy)
        view._error = State(initialValue: error)
        return view
    }
    #endif

    // MARK: - Source Import

    private var sourceImportSection: some View {
        Section {
            Text(String(localized: "Select a source file, preview its connections, then confirm. Imported API keys stay in this Mac’s Keychain. Source files are never changed.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            Picker(String(localized: "Import source", table: "ModelCenter"), selection: $importSource) {
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
                .toggleStyle(.macSwitch)
                .onChange(of: includeCredentials) { _, enabled in
                    invalidateImport()
                    if !enabled { credentialURL = nil; allowEnvironment = false }
                }
            if includeCredentials {
                Button(String(localized: "Choose a separate credentials file…", table: "ModelCenter")) { chooseSourceFile(credential: true) }
                if let credentialURL {
                    HStack {
                        Text(credentialURL.lastPathComponent)
                        Button(String(localized: "Remove selection", table: "ModelCenter")) {
                            self.credentialURL = nil; invalidateImport()
                        }
                    }
                }
                Toggle(String(localized: "Allow explicitly named environment variables", table: "ModelCenter"), isOn: $allowEnvironment)
                    .toggleStyle(.macSwitch)
                    .onChange(of: allowEnvironment) { _, _ in invalidateImport() }
            }
            Text(String(localized: "To use a ChatGPT account, sign in in your browser. Use file import for API connections.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary)
            HStack(spacing: 8) {
                Button(String(localized: "Sign in with ChatGPT", table: "ModelCenter")) {
                    cancelImport(); siwcAccount = nil; showingSiwc = true
                }
                Button(String(localized: "Source login on a Nola Host", table: "ModelCenter")) {
                    cancelImport(); activeSheet = .host
                }
            }
            Button(String(localized: "Preview import", table: "ModelCenter")) { previewImport() }
                .disabled(sourceURL == nil)
                .accessibilityIdentifier("model.import.preview")
            if busy {
                ProgressView().controlSize(.small)
                    .accessibilityLabel(String(localized: "Import in progress", table: "ModelCenter"))
            }
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
                        }))
                        .toggleStyle(.checkbox)
                        .disabled(candidate.issue != nil)
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
                            .toggleStyle(.checkbox)
                        }
                    }.padding(.vertical, 4)
                }
                Toggle(String(localized: "Enable imported models for chat", table: "ModelCenter"), isOn: $enableImportedModels)
                    .toggleStyle(.macSwitch)
                HStack(spacing: 8) {
                    Button(String(localized: "Confirm import", table: "ModelCenter")) { confirmImport(plan) }
                        .buttonStyle(.nolaProminent)
                        .disabled(importSelection.isEmpty || plan.candidates.contains { importSelection.contains($0.id) && $0.existing_revision != nil && !importUpdates.contains($0.id) })
                        .accessibilityIdentifier("model.import.confirm")
                    Button(String(localized: "Discard preview", table: "ModelCenter")) { invalidateImport() }
                }
            }
            ForEach(importResults) { result in
                Label(
                    result.status == "imported"
                        ? String(localized: "Connection imported", table: "ModelCenter")
                        : String(localized: "Connection not imported", table: "ModelCenter") + ": " + (result.error ?? String(localized: "Refresh the preview and try again", table: "ModelCenter")),
                    systemImage: result.status == "imported" ? "checkmark.circle" : "exclamationmark.triangle"
                )
            }
        }
    }

    private func chooseSourceFile(credential: Bool) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false; panel.canChooseFiles = true; panel.allowsMultipleSelection = false
        panel.message = credential
            ? String(localized: "Choose the credentials file you authorize Nola to read.", table: "ModelCenter")
            : String(localized: "Choose the configuration file for the selected source.", table: "ModelCenter")
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
            importPlan = try await bridge.preview(
                source: importSource,
                path: sourceURL.path,
                credentialPath: includeCredentials ? credentialURL?.path : nil,
                includeCredentials: includeCredentials,
                allowEnvironment: includeCredentials && allowEnvironment
            )
        }
    }

    private func confirmImport(_ plan: MacModelImportPlan) {
        guard let bridge = importBridge else { return }
        perform {
            defer { bridge.cancel(); importBridge = nil; importPlan = nil }
            importResults = try await bridge.commit(
                planID: plan.plan_id,
                selected: importSelection,
                updateExisting: importUpdates,
                enableModels: enableImportedModels
            )
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            if let firstImported = importResults.first(where: { $0.status == "imported" }) {
                selectedConnectionID = firstImported.connection_id
                showsCompactDetail = true
            }
            status = String(localized: "Import finished. Review the per-connection results below.", table: "ModelCenter")
        }
    }

    // MARK: - Reload & Execution

    private func reload() {
        reloadTask?.cancel()
        reloadTask = Task {
            await reloadAsync()
        }
    }

    private func reloadAsync() async {
        reloadGeneration &+= 1
        let generation = reloadGeneration
        do {
            let inventory = try await Task.detached(priority: .userInitiated) { [service] in
                try service.loadInventory()
            }.value
            guard !Task.isCancelled, generation == reloadGeneration else { return }
            connections = inventory.connections
            accounts = inventory.accounts
            enabled = Self.refreshedSelection(draft: enabled, saved: savedEnabled, fresh: inventory.enabledByConnection)
            savedEnabled = inventory.enabledByConnection
            if selectedConnectionID == nil || !connections.contains(where: { $0.id == selectedConnectionID }) {
                selectedConnectionID = connections.first?.id
            }
            error = nil
            focusRemoteProfileIfNeeded()
        } catch is CancellationError {
            return
        } catch {
            guard generation == reloadGeneration else { return }
            self.error = error.localizedDescription
            Logger.shared.log(
                .error,
                "MacModelConnectionsView",
                "Failed to load model connections",
                metadata: [
                    "error": error.localizedDescription,
                    "detail": String(describing: error),
                ]
            )
        }
    }

    private func focusRemoteProfileIfNeeded() {
        guard let profileID = focusedRemoteProfileID, !profileID.isEmpty,
              handledFocusRevision != focusRevision else { return }
        guard let profile = (try? service.application.listModelProfiles())?.first(where: {
            $0.id == profileID && $0.deletedAtMs == nil
        }),
              let source = (try? JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8))) as? [String: Any],
              let connectionID = source["connectionID"] as? String,
              let connection = connections.first(where: { $0.id == connectionID }) else {
            return
        }
        handledFocusRevision = focusRevision
        selectedConnectionID = connection.id
        showsCompactDetail = true
        if let catalogKey = source["catalogKey"] as? String,
           connection.document.catalog?.models.contains(where: { $0.id == catalogKey }) == true {
            expandedModelID = catalogKey
        }
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
            do {
                try await action()
                await reloadAsync()
            } catch {
                if Task.isCancelled {
                    await reloadAsync()
                    if status == nil {
                        status = String(localized: "Operation cancelled. Saved state was refreshed for review.", table: "ModelCenter")
                    }
                } else {
                    self.error = error.localizedDescription
                    Logger.shared.log(
                        .error,
                        "MacModelConnectionsView",
                        "Model connection operation failed",
                        metadata: ["error": error.localizedDescription]
                    )
                }
            }
        }
    }
}

// MARK: - Lightweight Lazy Row for 400+ Model Catalogs

private struct ConnectionModelRow<ExpandedContent: View>: View {
    let model: MacModelConnectionSnapshot.Model
    let isEnabled: Bool
    let isExpanded: Bool
    let hasVerifiedEvidence: Bool
    let onToggle: (Bool) -> Void
    let onToggleExpand: () -> Void
    @ViewBuilder let expandedContent: () -> ExpandedContent

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                Toggle(isOn: Binding(get: { isEnabled }, set: onToggle)) {
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(model.display_name)
                                .font(.body.weight(isEnabled ? .semibold : .regular))
                                .lineLimit(1)
                            if hasVerifiedEvidence {
                                Image(systemName: "checkmark.seal.fill")
                                    .font(.caption)
                                    .foregroundStyle(MacDesignColor.success)
                                    .help(String(localized: "Text response verified", table: "ModelCenter"))
                            }
                        }
                        HStack(spacing: 6) {
                            if model.display_name != model.model_id {
                                Text(model.model_id)
                                    .font(.system(.caption2, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            if let ctx = model.context_window, ctx > 0 {
                                capabilityBadge(Self.formatContext(ctx))
                            }
                            if model.modalities.contains("image") {
                                capabilityBadge("Vision")
                            }
                            if !model.reasoning_levels.isEmpty {
                                capabilityBadge("Reasoning")
                            }
                        }
                    }
                }
                .toggleStyle(.macSwitch)

                Spacer(minLength: 8)

                Button(action: onToggleExpand) {
                    Label(String(localized: "Model options", table: "ModelCenter"), systemImage: "slider.horizontal.3")
                }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .foregroundStyle(isExpanded ? MacDesignColor.brand : .secondary)
                .help(String(localized: "Model options", table: "ModelCenter"))
                .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            }

            if isExpanded {
                Divider().opacity(0.5)
                expandedContent()
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(
            isEnabled ? MacDesignColor.brand.opacity(0.05) : MacDesignColor.panel,
            in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                .strokeBorder(
                    isEnabled ? MacDesignColor.brand.opacity(0.24) : MacDesignColor.borderSubtle.opacity(0.75),
                    lineWidth: 0.75
                )
        }
    }

    private func capabilityBadge(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6)
            .padding(.vertical, 1.5)
            .background(MacDesignColor.subtleFill, in: Capsule())
    }

    private static func formatContext(_ tokens: UInt64) -> String {
        if tokens >= 1_000_000 {
            let m = Double(tokens) / 1_000_000.0
            return m.truncatingRemainder(dividingBy: 1) == 0 ? "\(Int(m))M" : String(format: "%.1fM", m)
        }
        if tokens >= 1_000 {
            return "\(tokens / 1_000)K"
        }
        return "\(tokens)"
    }
}

// MARK: - Explicit Capability Override Editor

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
                    .toggleStyle(.macSwitch)
                Toggle(String(localized: "Image input", table: "ModelCenter"), isOn: $values.imageInput)
                    .toggleStyle(.macSwitch)
                Toggle(String(localized: "Reasoning", table: "ModelCenter"), isOn: $values.reasoning)
                    .toggleStyle(.macSwitch)
                Toggle(String(localized: "Sampling parameters", table: "ModelCenter"), isOn: $values.samplingParameters)
                    .toggleStyle(.macSwitch)
                Toggle(String(localized: "Sampling requires reasoning disabled", table: "ModelCenter"), isOn: $values.samplingRequiresNoReasoning)
                    .toggleStyle(.macSwitch)
                    .disabled(!values.samplingParameters)
                DisclosureGroup(String(localized: "Advanced Overrides", table: "ModelCenter")) {
                    VStack(alignment: .leading, spacing: 8) {
                        Picker(String(localized: "Output length parameter", table: "ModelCenter"), selection: $values.chatOutputTokenField) {
                            Text(String(localized: "Automatic (protocol default)", table: "ModelCenter")).tag("auto")
                            Text(String(localized: "Legacy limit parameter (max_tokens)", table: "ModelCenter")).tag("max_tokens")
                            Text(String(localized: "Standard completion limit (max_completion_tokens)", table: "ModelCenter")).tag("max_completion_tokens")
                        }
                    }
                    .padding(.top, 6)
                }
                .disclosureGroupStyle(MacRowDisclosureStyle(isPlain: true))
                HStack {
                    Button(String(localized: "Save explicit settings", table: "ModelCenter")) { save(clear: false) }.disabled(declarationError)
                    Button(String(localized: "Clear settings", table: "ModelCenter")) { save(clear: true) }.disabled(saved == nil)
                }
            }.padding(.vertical, 8)
        }
        .disclosureGroupStyle(MacRowDisclosureStyle(isPlain: true))
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
