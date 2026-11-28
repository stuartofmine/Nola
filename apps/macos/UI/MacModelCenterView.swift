// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Hosts source-scoped model libraries with a consistent list/detail workspace.

import Combine
import SwiftUI

@MainActor
final class MacDefaultChatModelPresentation: ObservableObject {
    @Published private(set) var snapshot: MacDefaultChatModelSnapshot?
    @Published private(set) var catalog = MacChatModelCatalog(entries: [])
    @Published private(set) var resolution: MacChatModelResolution = .noAutomaticCandidate
    @Published private(set) var error: String?
    private var repairRevision: Int64?
    private var canRepair = false
    private var coalescedReloadTask: Task<Void, Never>?
    private var selectTask: Task<Void, Never>?
    private var reloadGeneration: UInt64 = 0
    let store: MacDefaultChatModelStore

    init(store: MacDefaultChatModelStore = .init()) {
        self.store = store
        scheduleReload()
    }

    func scheduleReload() {
        coalescedReloadTask?.cancel()
        reloadGeneration &+= 1
        let generation = reloadGeneration
        let store = self.store
        coalescedReloadTask = Task { @MainActor [weak self] in
            await Task.yield()
            guard let self, !Task.isCancelled, self.reloadGeneration == generation else { return }
            enum FetchedDefaultModel: Sendable {
                case success([MacCanonicalModelProfile], MacDefaultChatModelSnapshot)
                case failure(String, MacDefaultChatModelRawSnapshot?)
            }
            let fetched = await Task.detached(priority: .userInitiated) { () -> FetchedDefaultModel in
                do {
                    let profiles = try MacChatModelCatalog.fetchProfiles(application: store.application)
                    let snapshot = try store.snapshot()
                    return .success(profiles, snapshot)
                } catch {
                    let raw = try? store.rawSnapshot()
                    return .failure(error.localizedDescription, raw)
                }
            }.value
            guard !Task.isCancelled, self.reloadGeneration == generation else { return }
            switch fetched {
            case .failure(let message, let raw):
                self.error = message
                self.snapshot = nil
                self.resolution = .invalidConfiguration
                if let raw {
                    self.repairRevision = raw.revision
                    self.canRepair = true
                } else {
                    self.canRepair = false
                }
            case .success(let profiles, let snapshot):
                let builtCatalog: MacChatModelCatalog
                do {
                    builtCatalog = try MacChatModelCatalog.build(from: profiles, application: store.application)
                } catch {
                    self.error = error.localizedDescription
                    self.snapshot = nil
                    self.resolution = .invalidConfiguration
                    self.repairRevision = snapshot.revision
                    self.canRepair = true
                    return
                }
                let candidates = builtCatalog.candidates
                let valueJSON = snapshot.valueJSON
                let resolved = await Task.detached(priority: .userInitiated) {
                    () -> Result<MacChatModelResolution, Error> in
                    Result {
                        try store.application.resolveChatModel(
                            intent: .inherit,
                            defaultValueJSON: valueJSON,
                            candidates: candidates
                        )
                    }
                }.value
                guard !Task.isCancelled, self.reloadGeneration == generation else { return }
                self.catalog = builtCatalog
                switch resolved {
                case .success(let resolution):
                    self.snapshot = snapshot
                    self.resolution = resolution
                    self.canRepair = true
                    self.repairRevision = snapshot.revision
                    self.error = nil
                case .failure(let err):
                    self.error = err.localizedDescription
                    self.snapshot = nil
                    self.resolution = .invalidConfiguration
                    self.repairRevision = snapshot.revision
                    self.canRepair = true
                }
            }
        }
    }

    func reload() {
        coalescedReloadTask?.cancel()
        coalescedReloadTask = nil
        reloadGeneration &+= 1
        do {
            let catalog = try MacChatModelCatalog.load(application: store.application)
            self.catalog = catalog
            let snapshot = try store.snapshot()
            let resolution = try store.resolve(candidates: catalog.candidates)
            self.snapshot = snapshot
            self.resolution = resolution
            canRepair = true
            repairRevision = snapshot.revision
            error = nil
        } catch {
            self.error = error.localizedDescription
            snapshot = nil
            resolution = .invalidConfiguration
            if let raw = try? store.rawSnapshot() {
                repairRevision = raw.revision
                canRepair = true
            } else { canRepair = false }
        }
    }

    func isDefault(_ profileID: String) -> Bool { snapshot?.selection == .profile(profileID) }

    func canSelect(_ profileID: String) -> Bool {
        canRepair && catalog.candidates.contains {
            $0.profileID == profileID && $0.allowedForChat && $0.availability == .available
        }
    }

    func select(_ selection: MacDefaultChatModelSelection) {
        guard canRepair else { return }
        let store = self.store
        let expectedRevision = snapshot?.revision ?? repairRevision
        selectTask?.cancel()
        selectTask = Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let profiles = try await Task.detached(priority: .userInitiated) {
                    try MacChatModelCatalog.fetchProfiles(application: store.application)
                }.value
                guard !Task.isCancelled else { return }
                let currentCatalog = try MacChatModelCatalog.build(from: profiles, application: store.application)
                let candidates = currentCatalog.candidates
                let saved = try await Task.detached(priority: .userInitiated) {
                    try store.set(selection, expectedRevision: expectedRevision, candidates: candidates)
                }.value
                guard !Task.isCancelled else { return }
                self.catalog = currentCatalog
                self.snapshot = saved
                self.scheduleReload()
            } catch {
                guard !Task.isCancelled else { return }
                self.error = error.localizedDescription
            }
        }
    }

    var title: String {
        if error != nil { return String(localized: "Could Not Load Default Model", table: "ModelCenter") }
        switch resolution {
        case .unavailable: return String(localized: "Default model unavailable — choose a model", table: "ModelCenter")
        case .invalidConfiguration: return String(localized: "Could Not Load Default Model", table: "ModelCenter")
        case .noAutomaticCandidate: return String(localized: "Automatic — no available model", table: "ModelCenter")
        case let .resolved(id, origin):
            let name = catalog.entries.first { $0.candidate.profileID == id }?.option.displayName ?? id
            return origin == .automatic ? String(localized: "Automatic", table: "ModelCenter") + " · " + name : name
        }
    }
}

struct MacDefaultChatModelButton: View {
    @ObservedObject var preference: MacDefaultChatModelPresentation
    let profileID: String

    var body: some View {
        Button {
            preference.select(.profile(profileID))
        } label: {
            Label(preference.isDefault(profileID)
                  ? String(localized: "Default for chat", table: "ModelCenter")
                  : String(localized: "Set as default chat model", table: "ModelCenter"),
                  systemImage: preference.isDefault(profileID) ? "checkmark.circle.fill" : "checkmark.circle")
        }
        .disabled(preference.isDefault(profileID) || !preference.canSelect(profileID))
        .accessibilityIdentifier("set-default-chat-model-" + profileID)
    }
}

final class MacModelCenterNavigationState: ObservableObject {
    static let shared = MacModelCenterNavigationState()

    enum Tab: String, CaseIterable, Identifiable {
        case local
        case api

        var id: String { rawValue }
    }

    enum RemoteAction: Equatable {
        case api
        case accounts
    }

    @Published var selectedTab: Tab = .local
    @Published private(set) var focusedRemoteProfileID: String?
    @Published private(set) var focusRevision = 0
    @Published private(set) var pendingRemoteAction: RemoteAction?
    @Published private(set) var remoteActionRevision = 0

    init(selectedTab: Tab = .local) { self.selectedTab = selectedTab }

    var showsLocalModels: Bool { selectedTab == .local }
    var showsRemoteModels: Bool { selectedTab == .api }

    func showLocalModels() {
        focusedRemoteProfileID = nil
        pendingRemoteAction = nil
        selectedTab = .local
    }
    func showRemoteModels(action: RemoteAction? = nil) {
        focusedRemoteProfileID = nil
        pendingRemoteAction = action
        if action != nil { remoteActionRevision &+= 1 }
        selectedTab = .api
    }
    func showRemoteModel(profileID: String) {
        focusedRemoteProfileID = profileID
        pendingRemoteAction = nil
        focusRevision &+= 1
        selectedTab = .api
    }
    func showSelectedModel(remoteProfileID: String?) {
        if let remoteProfileID { showRemoteModel(profileID: remoteProfileID) }
        else { showLocalModels() }
    }
    func showModelCenter() { selectedTab = .local }

    func shouldRefreshProviderCatalog(baseURL: String, catalogIsEmpty: Bool) -> Bool {
        selectedTab == .api && !baseURL.isEmpty && catalogIsEmpty
    }
}

struct MacModelCenterView: View {
    @State private var visitedSources: Set<MacModelCenterNavigationState.Tab>
    @StateObject private var navigation: MacModelCenterNavigationState
    @StateObject private var localEditing: MacLocalModelEditingState
    @StateObject private var preference: MacDefaultChatModelPresentation
    @StateObject private var downloader = ModelDownloader(logger: Logger.shared)
    @Environment(\.openWindow) private var openWindow
    let navigationAction: (() -> Void)?

    init(
        remoteHost: any MacRemoteHostManaging,
        navigationAction: (() -> Void)? = nil,
        navigation: MacModelCenterNavigationState = MacModelCenterNavigationState(),
        remoteDraft: MacRemoteModelEditingState? = nil,
        localEditing: MacLocalModelEditingState? = nil,
        preference: MacDefaultChatModelPresentation? = nil
    ) {
        _preference = StateObject(wrappedValue: preference ?? MacDefaultChatModelPresentation())
        _ = remoteHost
        _ = remoteDraft
        self.navigationAction = navigationAction
        _navigation = StateObject(wrappedValue: navigation)
        _visitedSources = State(initialValue: [navigation.selectedTab])
        _localEditing = StateObject(wrappedValue: localEditing ?? MacLocalModelEditingState())
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                LocalizedStringKey(String(localized: "Models", table: "ModelCenter")),
                symbol: "cpu",
                navigationAction: navigationAction
            ) {
                Text(String(localized: "Manage local and API models in one place.", table: "ModelCenter"))
            } actions: {
                diagnostics
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) {
                    sourceSegmentedControl
                    Spacer(minLength: 12)
                    defaultModelControl
                        .frame(maxWidth: 420, alignment: .trailing)
                }
                VStack(alignment: .leading, spacing: 10) {
                    sourceSegmentedControl
                    defaultModelControl
                }
            }
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.top, MacDesignTokens.Spacing.xs)
            .padding(.bottom, MacDesignTokens.Spacing.md)
            if let error = preference.error {
                HStack(spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(MacDesignColor.danger)
                    Text(error).font(.callout).foregroundStyle(.red)
                    Spacer(minLength: 0)
                    Button(String(localized: "Reload")) { preference.scheduleReload() }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(MacDesignColor.danger.opacity(0.08), in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous))
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.bottom, MacDesignTokens.Spacing.sm)
            }

            Divider().opacity(0.55)

            // Keep visited sources mounted: switching sources must preserve search,
            // connection drafts and selection, while hidden sources cannot receive input.
            ZStack {
                if visitedSources.contains(.local) {
                    MacLocalModelManagerView(showsHeader: false, downloader: downloader, preference: preference,
                                             editingState: localEditing)
                        .opacity(navigation.selectedTab == .local ? 1 : 0)
                        .allowsHitTesting(navigation.selectedTab == .local)
                        .accessibilityHidden(navigation.selectedTab != .local)
                        .disabled(navigation.selectedTab != .local)
                }
                if visitedSources.contains(.api) {
                    MacModelConnectionsView(
                        service: MacModelConnectionService(application: preference.store.application),
                        focusedRemoteProfileID: navigation.focusedRemoteProfileID,
                        focusRevision: navigation.focusRevision,
                        pendingAction: navigation.pendingRemoteAction,
                        pendingActionRevision: navigation.remoteActionRevision
                    )
                        .opacity(navigation.selectedTab == .api ? 1 : 0)
                        .allowsHitTesting(navigation.selectedTab == .api)
                        .accessibilityHidden(navigation.selectedTab != .api)
                        .disabled(navigation.selectedTab != .api)
                }
            }
            .onChange(of: navigation.selectedTab) { _, source in visitedSources.insert(source) }
        }
        .background(MacDesignColor.canvas)
        .tint(MacDesignColor.brand)
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in preference.scheduleReload() }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in preference.scheduleReload() }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacLocalModelRuntimeDidChange)) { _ in preference.scheduleReload() }
    }

    private var sourceSegmentedControl: some View {
        Picker(String(localized: "Model source", table: "ModelCenter"), selection: $navigation.selectedTab) {
            Label(String(localized: "Local Models", table: "ModelCenter"), systemImage: "cpu.fill")
                .tag(MacModelCenterNavigationState.Tab.local)
            Label(String(localized: "API Models", table: "ModelCenter"), systemImage: "network")
                .tag(MacModelCenterNavigationState.Tab.api)
        }
        .labelsHidden()
        .pickerStyle(.segmented)
        .frame(width: 260)
        .accessibilityIdentifier("model-center-source")
    }

    private var diagnostics: some View {
        Menu {
            Button(String(localized: "Test Model…", table: "ModelCenter")) { openWindow(id: "model-test") }
        } label: {
            Label(String(localized: "Diagnostics", table: "ModelCenter"), systemImage: "ellipsis")
        }
        .menuStyle(MacToolbarMenuStyle(size: 30))
    }

    private var defaultModelControl: some View {
                Menu {
                    Button(String(localized: "Automatic", table: "ModelCenter")) { preference.select(.automatic) }
                    ForEach([MacChatModelOption.Source.configured, .local], id: \.self) { source in
                        Section(source == .local ? String(localized: "This Mac", table: "ModelCenter") : String(localized: "API Models", table: "ModelCenter")) {
                            ForEach(preference.catalog.entries.filter { $0.option.source == source }, id: \.candidate.profileID) { entry in
                                Button { preference.select(.profile(entry.candidate.profileID)) } label: {
                                    if preference.isDefault(entry.candidate.profileID) {
                                        Label(entry.option.displayName, systemImage: "checkmark")
                                    } else { Text(entry.option.displayName) }
                                }
                                    .disabled(!preference.canSelect(entry.candidate.profileID))
                            }
                        }
                    }
                } label: {
                    Text(String(localized: "Default chat model", table: "ModelCenter") + " · " + preference.title)
                        .font(.callout).lineLimit(1).truncationMode(.middle)
                }
                .menuStyle(MacPillMenuStyle(height: 28))
                .help(String(localized: "Used by conversations that inherit the default. Running requests keep their current model.", table: "ModelCenter"))
                .accessibilityIdentifier("default-chat-model-picker")
                .accessibilityLabel(String(localized: "Default chat model", table: "ModelCenter"))
                .accessibilityValue(preference.title)
    }
}
