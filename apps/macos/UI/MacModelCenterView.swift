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
    let store: MacDefaultChatModelStore

    init(store: MacDefaultChatModelStore = .init()) {
        self.store = store
        reload()
    }

    func reload() {
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
        do {
            let currentCatalog = try MacChatModelCatalog.load(application: store.application)
            self.snapshot = try store.set(selection, expectedRevision: snapshot?.revision ?? repairRevision,
                                          candidates: currentCatalog.candidates)
            reload()
        } catch { self.error = error.localizedDescription }
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

    @Published var selectedTab: Tab = .local
    @Published private(set) var focusedRemoteProfileID: String?
    @Published private(set) var focusRevision = 0

    init(selectedTab: Tab = .local) { self.selectedTab = selectedTab }

    var showsLocalModels: Bool { selectedTab == .local }
    var showsRemoteModels: Bool { selectedTab == .api }

    func showLocalModels() {
        focusedRemoteProfileID = nil
        selectedTab = .local
    }
    func showRemoteModels() {
        focusedRemoteProfileID = nil
        selectedTab = .api
    }
    func showRemoteModel(profileID: String) {
        focusedRemoteProfileID = profileID
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
    @StateObject private var remoteDraft: MacRemoteModelEditingState
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
        self.navigationAction = navigationAction
        _navigation = StateObject(wrappedValue: navigation)
        _visitedSources = State(initialValue: [navigation.selectedTab])
        _remoteDraft = StateObject(wrappedValue: remoteDraft ?? MacRemoteModelEditingState())
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
            HStack {
                defaultModelControl
                    .frame(maxWidth: 520, alignment: .leading)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.vertical, MacDesignTokens.Spacing.sm)
            if let error = preference.error {
                HStack {
                    Text(error).font(.callout).foregroundStyle(.red)
                    Button(String(localized: "Reload")) { preference.reload() }
                }.padding(.horizontal, 24).padding(.bottom, 12)
            }

            HStack {
                Picker(String(localized: "Model source", table: "ModelCenter"), selection: $navigation.selectedTab) {
                    Label(String(localized: "Local Models", table: "ModelCenter"), systemImage: "cpu.fill")
                        .tag(MacModelCenterNavigationState.Tab.local)
                    Label(String(localized: "API Models", table: "ModelCenter"), systemImage: "network")
                        .tag(MacModelCenterNavigationState.Tab.api)
                }
                .labelsHidden()
                .pickerStyle(.segmented)
                .frame(maxWidth: 300, alignment: .leading)
                .accessibilityIdentifier("model-center-source")
                Spacer(minLength: 0)
            }
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.bottom, MacDesignTokens.Spacing.md)

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
                    MacModelConnectionsView(service: MacModelConnectionService(application: preference.store.application))
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
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in preference.reload() }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in preference.reload() }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacLocalModelRuntimeDidChange)) { _ in preference.reload() }
    }

    private var diagnostics: some View {
        Menu {
            Button(String(localized: "Test Model…", table: "ModelCenter")) { openWindow(id: "model-test") }
        } label: {
            Label(String(localized: "Diagnostics", table: "ModelCenter"), systemImage: "ellipsis")
        }.labelStyle(.iconOnly).menuStyle(.borderlessButton).fixedSize()
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
                .menuStyle(.borderlessButton)
                .help(String(localized: "Used by conversations that inherit the default. Running requests keep their current model.", table: "ModelCenter"))
                .accessibilityIdentifier("default-chat-model-picker")
                .accessibilityLabel(String(localized: "Default chat model", table: "ModelCenter"))
                .accessibilityValue(preference.title)
    }
}
