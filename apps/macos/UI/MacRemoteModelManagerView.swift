// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Manages multiple API endpoints and ordered, purpose-scoped models on macOS.

import Combine
import SwiftUI

enum MacRemoteModelDestination: Hashable {
    case connection(String)
    case model(connectionID: String, profileID: String)
    case newConnection

    var connectionID: String? {
        switch self {
        case .connection(let id), .model(let id, _): id
        case .newConnection: nil
        }
    }
}

@MainActor
final class MacRemoteModelEditingState: ObservableObject {
    @Published var endpoints: [MacRemoteEndpointConfiguration] = []
    @Published var savedEndpoints: [MacRemoteEndpointConfiguration] = []
    @Published var savedRevisions: [String: Int64] = [:]
    @Published var pendingModelParameters: [String: ModelParameters] = [:]
    @Published var parameterDrafts: [String: MacModelParameterDraft] = [:]
    @Published var parameterErrors: [String: String] = [:]
    @Published var selectedEndpointID: String?
    @Published var selectedModelID: String?
    @Published var modelSearch = ""
    @Published var compactShowsDetail = false
    @Published var loadError: String?
    @Published var needsReload = false
    @Published var notice: MacRemoteModelManagerNotice?
    @Published var pendingDestination: MacRemoteModelDestination?
    @Published var libraryScrollID: MacRemoteModelDestination?
    var lastFocusRevision: Int?
    var initialized = false
    var showsConnectionOnLoad = false

    var hasUnsavedChanges: Bool {
        endpoints != savedEndpoints || !pendingModelParameters.isEmpty
            || parameterDrafts.values.contains { $0.isEdited }
    }

    func updateParameterDraft(_ draft: MacModelParameterDraft, for profileID: String) {
        parameterDrafts[profileID] = draft
        let isPersisted = savedEndpoints.contains { $0.models.contains { $0.id == profileID } }
        pendingModelParameters[profileID] = draft.isEdited || !isPersisted ? draft.parameters : nil
    }

    /// Returns whether crossing the connection transaction needs a decision.
    func requestNavigation(to destination: MacRemoteModelDestination) -> Bool {
        pendingDestination = destination
        return selectedEndpointID != destination.connectionID && hasUnsavedChanges
    }

    func navigate(to destination: MacRemoteModelDestination) {
        selectedEndpointID = destination.connectionID
        if case let .model(_, profileID) = destination { selectedModelID = profileID }
        else { selectedModelID = nil }
        pendingDestination = nil
        compactShowsDetail = true
    }
}

/// The whole header, including its text and empty trailing space, is the
/// disclosure target. SwiftUI's default DisclosureGroup can leave only its
/// chevron clickable in a form-like container on macOS.
private struct MacModelCenterDisclosure<Content: View>: View {
    let title: String
    let symbol: String
    @Binding var isExpanded: Bool
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { isExpanded.toggle() } label: {
                HStack(spacing: 8) {
                    Label(title, systemImage: symbol)
                        .font(.callout.weight(.medium))
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .rotationEffect(.degrees(isExpanded ? 180 : 0))
                }
                .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            if isExpanded { content }
        }
    }
}

/// A successful probe proves the concrete model/endpoint accepts a Responses
/// request. It does not prove that ignored optional parameters (including
/// reasoning effort) are implemented by the provider.
nonisolated enum MacResponsesRouteProbe {
    static func url(for baseURL: URL) -> URL {
        var value = baseURL
        if value.path.hasSuffix("/chat/completions") {
            value.deleteLastPathComponent()
            value.deleteLastPathComponent()
        } else if value.path.hasSuffix("/responses") {
            value.deleteLastPathComponent()
        }
        return value.appendingPathComponent("responses")
    }

    static func accepts(_ data: Data) -> Bool {
        guard let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
        return response["object"] as? String == "response"
            && response["output"] is [Any]
            && ["completed", "incomplete"].contains(response["status"] as? String ?? "")
    }
}

nonisolated struct MacDiscoveredModelCatalog: Sendable {
    let baseURL: String
    let apiKey: String
    let wireProtocol: MacModelWireProtocol
    let models: [String]

    init(endpoint: MacRemoteEndpointConfiguration, models: [String]) {
        baseURL = endpoint.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        apiKey = endpoint.apiKey
        wireProtocol = endpoint.wireProtocol
        self.models = models
    }

    func matches(_ endpoint: MacRemoteEndpointConfiguration) -> Bool {
        baseURL == endpoint.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
            && apiKey == endpoint.apiKey && wireProtocol == endpoint.wireProtocol
    }

    func models(for endpoint: MacRemoteEndpointConfiguration) -> [String] {
        matches(endpoint) ? models : []
    }
}

struct MacRemoteModelManagerNotice: Identifiable, Equatable {
    enum Kind: Equatable { case success, information, warning, failure }

    let id = UUID()
    let title: String
    let message: String
    let kind: Kind
    var offersReload = false

    var inlineKind: MacInlineNotice.Kind {
        switch kind {
        case .success: .success
        case .information: .information
        case .warning: .warning
        case .failure: .failure
        }
    }
}

struct MacAddRemoteModelRequest: Identifiable {
    let endpointID: String
    let discoveredModels: [String]
    let initialModelID: String

    var id: String { endpointID }
}

struct MacAddRemoteModelResult {
    let configuration: MacRemoteModelConfiguration
    let parameters: ModelParameters
}

/// Discovery is an explicit, optional action. A provider's model catalog is not
/// an authority for whether a manually configured model may be persisted:
/// compatible endpoints and Anthropic Messages need not expose `/models` at all.
enum MacRemoteModelDraftCommit {
    static func validateConnection(
        endpointID: String?,
        drafts: [MacRemoteEndpointConfiguration],
        store: MacAppleChatConfigurationStore
    ) throws {
        try store.validateEndpoints(drafts.filter { $0.id == endpointID })
    }

    static func saveConnection(
        endpointID: String,
        draft: MacRemoteEndpointConfiguration?,
        baseline: [MacRemoteEndpointConfiguration],
        parametersByModelID: [String: ModelParameters],
        expectedRevisions: [String: Int64],
        store: MacAppleChatConfigurationStore
    ) throws -> MacRemoteEndpointSnapshot {
        guard draft == nil || draft?.id == endpointID else {
            throw MacApplicationServiceError.rejected("connection draft identity mismatch")
        }
        let current = try store.endpointSnapshot()
        let originalIDs = Set(baseline.first(where: { $0.id == endpointID })?.models.map(\.id) ?? [])
        let currentIDs = Set(current.endpoints.first(where: { $0.id == endpointID })?.models.map(\.id) ?? [])
        guard originalIDs == currentIDs,
              originalIDs.allSatisfy({ expectedRevisions[$0] == current.revisions[$0] }) else {
            throw MacApplicationServiceError.rejected("revision_conflict: model catalog connection changed")
        }
        let draftIDs = Set(draft?.models.map(\.id) ?? [])
        try store.replaceConnection(endpointID: endpointID, draft: draft,
            parametersByModelID: parametersByModelID.filter { draftIDs.contains($0.key) },
            expectedRevisions: current.revisions)
        return try store.endpointSnapshot()
    }

    static func save(
        _ endpoints: [MacRemoteEndpointConfiguration],
        parametersByModelID: [String: ModelParameters],
        expectedRevisions: [String: Int64],
        store: MacAppleChatConfigurationStore
    ) throws -> MacRemoteEndpointSnapshot {
        try store.replaceEndpoints(
            endpoints,
            parametersByModelID: parametersByModelID,
            expectedRevisions: expectedRevisions
        )
        return try store.endpointSnapshot()
    }
}

enum MacRemoteModelSaveFailure {
    static func isCatalogConflict(_ error: Error) -> Bool {
        guard let error = error as? MacApplicationServiceError,
              case .rejected(let message) = error else { return false }
        return message.contains("revision_conflict: model catalog")
    }
}

enum MacAddRemoteModelValidation {
    static func identityMessage(
        modelID: String,
        capabilities: Set<MacRemoteModelCapability>
    ) -> String? {
        if modelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Enter a model ID to continue.", table: "ModelCenter")
        }
        if capabilities.subtracting([.sensitiveData]).isEmpty {
            return String(localized: "Select at least one allowed use.", table: "ModelCenter")
        }
        return nil
    }

    static func message(
        modelID: String,
        capabilities: Set<MacRemoteModelCapability>,
        parameters: ModelParameters
    ) -> String? {
        if let identityMessage = identityMessage(modelID: modelID, capabilities: capabilities) {
            return identityMessage
        }
        return MacModelParameterValidation.message(
            parameters: parameters,
            seedText: "",
            isLocal: false,
            contextRange: ModelParameterLimits.remoteContextTokens
        )
    }
}

struct MacRemoteModelManagerView: View {
    private enum DetailSection: String, CaseIterable, Identifiable {
        case basic, advanced
        var id: String { rawValue }
        var title: String {
            switch self {
            case .basic: String(localized: "Common settings", table: "ModelCenter")
            case .advanced: String(localized: "Advanced settings", table: "ModelCenter")
            }
        }
    }
    @StateObject private var editing: MacRemoteModelEditingState
    @ObservedObject private var preference: MacDefaultChatModelPresentation
    private var endpoints: [MacRemoteEndpointConfiguration] {
        get { editing.endpoints }
        nonmutating set { editing.endpoints = newValue }
    }
    private var savedEndpoints: [MacRemoteEndpointConfiguration] {
        get { editing.savedEndpoints }
        nonmutating set { editing.savedEndpoints = newValue }
    }
    private var savedRevisions: [String: Int64] {
        get { editing.savedRevisions }
        nonmutating set { editing.savedRevisions = newValue }
    }
    private var pendingModelParameters: [String: ModelParameters] {
        get { editing.pendingModelParameters }
        nonmutating set { editing.pendingModelParameters = newValue }
    }
    private var loadError: String? {
        get { editing.loadError }
        nonmutating set { editing.loadError = newValue }
    }
    private var needsReload: Bool {
        get { editing.needsReload }
        nonmutating set { editing.needsReload = newValue }
    }
    private var selectedEndpointID: String? {
        get { editing.selectedEndpointID }
        nonmutating set { editing.selectedEndpointID = newValue }
    }
    private var selectedModelID: String? {
        get { editing.selectedModelID }
        nonmutating set { editing.selectedModelID = newValue }
    }
    private var modelSearch: String { editing.modelSearch }
    private var compactShowsDetail: Bool {
        get { editing.compactShowsDetail }
        nonmutating set { editing.compactShowsDetail = newValue }
    }
    @State private var discoveredModels: [String: MacDiscoveredModelCatalog] = [:]
    @State private var discoveryFailures: [String: (catalog: MacDiscoveredModelCatalog, message: String)] = [:]
    @State private var discoverySearch: [String: String] = [:]
    @State private var loadingEndpointID: String?
    @State private var probingEndpointID: String?
    @State private var isSaving = false
    private var notice: MacRemoteModelManagerNotice? {
        get { editing.notice }
        nonmutating set { editing.notice = newValue }
    }
    @State private var deletionEndpointID: String?
    @State private var discardPending = false
    @State private var conflictReloadPending = false
    @State private var draggedModelID: String?
    @State private var expandedEndpointSettings: Set<String> = []
    @State private var expandedModelSettings: Set<String> = []
    @State private var addModelRequest: MacAddRemoteModelRequest?
    @State private var detailSection: DetailSection = .basic
    @State private var confirmsSelectionChange = false
    @State private var showsFeedback = false
    @Environment(\.openWindow) private var openWindow
    let navigationAction: (() -> Void)?
    let showsHeader: Bool
    let focusedProfileID: String?
    let focusRevision: Int
    let application: MacApplicationPort
    private let initialEndpoints: [MacRemoteEndpointConfiguration]?
    private var configurationStore: MacAppleChatConfigurationStore {
        MacAppleChatConfigurationStore(applicationPort: application)
    }

    init(
        navigationAction: (() -> Void)? = nil,
        showsHeader: Bool = true,
        focusedProfileID: String? = nil,
        focusRevision: Int = 0,
        endpoints initialEndpoints: [MacRemoteEndpointConfiguration]? = nil,
        initialCatalogs: [String: MacDiscoveredModelCatalog] = [:],
        editingState: MacRemoteModelEditingState? = nil,
        preference: MacDefaultChatModelPresentation? = nil,
        application: MacApplicationPort = .shared
    ) {
        self.navigationAction = navigationAction
        self.showsHeader = showsHeader
        self.focusedProfileID = focusedProfileID
        self.focusRevision = focusRevision
        self.application = application
        self.preference = preference ?? MacDefaultChatModelPresentation(store: .init(application: application))
        let editing = editingState ?? MacRemoteModelEditingState()
        _editing = StateObject(wrappedValue: editing)
        self.initialEndpoints = initialEndpoints
        _discoveredModels = State(initialValue: initialCatalogs)
    }

    /// Hydration happens after SwiftUI mounts the view, never while a parent
    /// body is constructing a child with an already-observed draft owner.
    private func hydrateEditingState() {
        guard !editing.initialized else { return }
        let loadedEndpoints: [MacRemoteEndpointConfiguration]
        let loadedRevisions: [String: Int64]
        let initialLoadError: String?
        if let initialEndpoints {
            loadedEndpoints = initialEndpoints
            loadedRevisions = [:]
            initialLoadError = nil
        } else {
            do {
                let snapshot = try MacAppleChatConfigurationStore(applicationPort: application).endpointSnapshot()
                loadedEndpoints = snapshot.endpoints
                loadedRevisions = snapshot.revisions
                initialLoadError = nil
            } catch {
                loadedEndpoints = []
                loadedRevisions = [:]
                initialLoadError = error.localizedDescription
            }
        }
        editing.endpoints = loadedEndpoints
        editing.savedEndpoints = loadedEndpoints
        editing.savedRevisions = loadedRevisions
        editing.loadError = initialLoadError
        editing.selectedEndpointID = loadedEndpoints.first?.id
        editing.selectedModelID = editing.showsConnectionOnLoad ? nil : loadedEndpoints.first?.models.first?.id
        editing.compactShowsDetail = editing.compactShowsDetail || !discoveredModels.isEmpty
        editing.initialized = true
        if initialEndpoints != nil {
            for model in loadedEndpoints.flatMap(\.models) {
                editing.parameterDrafts[model.id] = .init(parameters: .webAlignedCloudDefaults)
            }
        }
        expandedEndpointSettings = Set(loadedEndpoints.filter { $0.models.isEmpty }.map(\.id))
    }

    var body: some View {
        VStack(spacing: 0) {
            if showsHeader {
                MacPageHeader(
                    LocalizedStringKey(String(localized: "API Models", table: "ModelCenter")),
                    symbol: "network",
                    navigationAction: navigationAction,
                    navigationSymbol: "chevron.left",
                    navigationLabel: LocalizedStringKey(String(localized: "Model Center", table: "ModelCenter"))
                ) {
                    Text(String(localized: "Select a model to edit. Connection settings apply to its models.", table: "ModelCenter"))
                } actions: {
                    managementActions
                }
            } else {
                HStack {
                    Text(String(localized: "Manage endpoints, model order, priorities, and allowed uses.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    Spacer()
                    managementActions
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
            }

            Button { showsFeedback.toggle() } label: {
                Label(feedbackTitle ?? "", systemImage: feedbackIsError ? "exclamationmark.circle" : "info.circle")
                    .font(.caption).lineLimit(1)
                    .foregroundStyle(feedbackIsError ? Color.orange : Color.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .opacity(feedbackTitle == nil ? 0 : 1)
            .disabled(feedbackTitle == nil)
            .accessibilityHidden(feedbackTitle == nil)
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .frame(height: 24)
            .accessibilityIdentifier("model-center-feedback")
            .popover(isPresented: $showsFeedback) {
                feedbackDetails.frame(width: 520, height: 250)
            }

            GeometryReader { geometry in
                if geometry.size.width >= 760 {
                    HStack(spacing: 0) {
                        modelLibrary.frame(width: 250)
                        Divider()
                        selectedDetail
                    }
                } else if compactShowsDetail {
                    VStack(alignment: .leading, spacing: 0) {
                        Button {
                            compactShowsDetail = false
                        } label: {
                            Label(String(localized: "All API models", table: "ModelCenter"), systemImage: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .padding(16)
                        selectedDetail
                    }
                } else {
                    modelLibrary
                }
            }
            .disabled(loadError != nil)
            .task(id: focusRevision) {
                hydrateEditingState()
                _ = expandFocusedProfileIfPresent()
                loadParameterDraft()
            }
            .task(id: selectedModelID) { loadParameterDraft() }
        }
        .background(MacPageBackground())
        .background {
            Button("Save") { save() }
                .keyboardShortcut("s", modifiers: .command)
                .hidden()
                .disabled(!hasUnsavedChanges || isSaving || loadError != nil || needsReload || draftValidationMessage != nil)
        }
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in preference.reload() }
        .sheet(item: $addModelRequest) { request in
            MacAddRemoteModelSheet(request: request) { result in
                addModel(result, endpointID: request.endpointID)
            }
        }
        .confirmationDialog(
            String(localized: "Discard unsaved changes?", table: "ModelCenter"),
            isPresented: $confirmsSelectionChange
        ) {
            Button(String(localized: "Save and Continue", table: "ModelCenter")) {
                if save() { finishPendingNavigation() }
            }
            .accessibilityIdentifier("model-navigation-save")
            .disabled(draftValidationMessage != nil || needsReload || loadError != nil)
            Button(String(localized: "Discard Changes", table: "ModelCenter"), role: .destructive) {
                endpoints = savedEndpoints
                pendingModelParameters.removeAll()
                editing.parameterDrafts.removeAll()
                editing.parameterErrors.removeAll()
                needsReload = false
                notice = nil
                finishPendingNavigation()
            }
            .accessibilityIdentifier("model-navigation-discard")
            Button(String(localized: "Keep Editing", table: "ModelCenter"), role: .cancel) {
                editing.pendingDestination = nil
            }
            .accessibilityIdentifier("model-navigation-stay")
        }
        .confirmationDialog(
            String(localized: "Delete this endpoint?", table: "ModelCenter"),
            isPresented: Binding(
                get: { deletionEndpointID != nil },
                set: { if !$0 { deletionEndpointID = nil } }
            )
        ) {
            Button(String(localized: "Delete Endpoint", table: "ModelCenter"), role: .destructive) {
                guard let deletionEndpointID else { return }
                let removedModelIDs = Set(
                    endpoints.first(where: { $0.id == deletionEndpointID })?.models.map(\.id) ?? []
                )
                endpoints.removeAll { $0.id == deletionEndpointID }
                pendingModelParameters = pendingModelParameters.filter {
                    !removedModelIDs.contains($0.key)
                }
                discoveredModels[deletionEndpointID] = nil
                expandedEndpointSettings.remove(deletionEndpointID)
                expandedModelSettings.subtract(removedModelIDs)
                self.deletionEndpointID = nil
                selectedModelID = nil
            }
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) {}
        } message: {
            Text(String(localized: "Its models and saved API key will be removed when you save.", table: "ModelCenter"))
        }
        .confirmationDialog(
            String(localized: "Discard unsaved changes?", table: "ModelCenter"),
            isPresented: $discardPending
        ) {
            Button(String(localized: "Discard Changes", table: "ModelCenter"), role: .destructive) {
                if needsReload {
                    reloadEndpoints()
                    return
                }
                endpoints = savedEndpoints
                pendingModelParameters.removeAll()
                editing.parameterDrafts.removeAll()
                editing.parameterErrors.removeAll()
                addModelRequest = nil
                deletionEndpointID = nil
                draggedModelID = nil
                expandedEndpointSettings.removeAll()
                expandedModelSettings.removeAll()
                notice = nil
                loadParameterDraft()
            }
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) {}
        }
        .confirmationDialog(
            String(localized: "Reload API models and discard these edits?", table: "ModelCenter"),
            isPresented: $conflictReloadPending
        ) {
            Button(String(localized: "Reload and Discard Edits", table: "ModelCenter"), role: .destructive) {
                reloadEndpoints()
            }
            Button(String(localized: "Keep Editing", table: "ModelCenter"), role: .cancel) {}
        }
    }

    private var feedbackTitle: String? {
        if loadError != nil { return String(localized: "Could Not Load API Models", table: "ModelCenter") }
        if needsReload { return String(localized: "API Models Changed Elsewhere", table: "ModelCenter") }
        if hasUnsavedChanges, draftValidationMessage != nil {
            return String(localized: "Check API model settings", table: "ModelCenter")
        }
        return notice?.title
    }

    private var feedbackIsError: Bool {
        loadError != nil || (hasUnsavedChanges && draftValidationMessage != nil)
            || notice?.kind == .failure || notice?.kind == .warning
    }

    private var feedbackDetails: some View {
        ScrollView {
            VStack(spacing: 12) {
                if let loadError {
                    MacInlineNotice(title: String(localized: "Could Not Load API Models", table: "ModelCenter"),
                                    message: loadError, kind: .failure,
                                    actionTitle: String(localized: "Reload"), action: reloadEndpoints)
                } else if let notice {
                    MacInlineNotice(title: notice.title, message: notice.message, kind: notice.inlineKind,
                                    actionTitle: notice.offersReload ? String(localized: "Reload") : nil,
                                    action: notice.offersReload ? {
                                        showsFeedback = false
                                        conflictReloadPending = true
                                    } : nil,
                                    dismiss: { self.notice = nil; showsFeedback = false })
                }
                if hasUnsavedChanges, let draftValidationMessage {
                    MacInlineNotice(title: String(localized: "Check API model settings", table: "ModelCenter"),
                                    message: draftValidationMessage, kind: .warning)
                    if let id = invalidParameterModelID,
                       let endpoint = endpoints.first(where: { $0.models.contains(where: { $0.id == id }) }) {
                        Button(String(localized: "Review parameters", table: "ModelCenter")) {
                            showsFeedback = false
                            requestNavigation(.model(connectionID: endpoint.id, profileID: id))
                            let fields = editing.parameterDrafts[id]?.invalidFields ?? []
                            if !fields.isDisjoint(with: ["presence", "frequency", "seed"]) {
                                detailSection = .advanced
                            }
                        }
                    }
                }
                if needsReload && notice?.offersReload != true {
                    Button(String(localized: "Reload")) {
                        showsFeedback = false
                        requestParameterReload()
                    }
                }
            }
            .padding(16)
        }
    }

    private var modelLibrary: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField(String(localized: "Search models and connections", table: "ModelCenter"), text: $editing.modelSearch)
                .textFieldStyle(.roundedBorder)
                .padding(.horizontal, 16)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(endpoints) { endpoint in
                        let matchesConnection = modelSearch.isEmpty || endpoint.name.localizedCaseInsensitiveContains(modelSearch)
                        let models = endpoint.models.filter { matchesConnection || $0.modelName.localizedCaseInsensitiveContains(modelSearch) || $0.displayName.localizedCaseInsensitiveContains(modelSearch) }
                        if matchesConnection || !models.isEmpty {
                            Button { selectEndpoint(endpoint.id) } label: {
                                HStack {
                                    Image(systemName: "network")
                                    Text(endpoint.name.isEmpty ? endpoint.baseURL : endpoint.name).lineLimit(1)
                                    Spacer(minLength: 4)
                                    if !endpoint.isEnabled { Image(systemName: "pause.circle") }
                                    Image(systemName: "slider.horizontal.3")
                                }
                                .font(.callout.weight(.medium))
                                .padding(12)
                                .contentShape(Rectangle())
                                .macSelectionHighlight(isSelected: selectedEndpointID == endpoint.id && selectedModelID == nil)
                            }
                            .buttonStyle(MacActionButtonStyle(isSelected: selectedEndpointID == endpoint.id && selectedModelID == nil, contentInsets: .init()))
                            .id(MacRemoteModelDestination.connection(endpoint.id))
                            ForEach(models) { model in
                                Button {
                                    requestNavigation(.model(connectionID: endpoint.id, profileID: model.id))
                                } label: {
                                    HStack(spacing: 8) {
                                        Image(systemName: model.isEnabled && endpoint.isEnabled ? "cube" : "pause.circle")
                                        VStack(alignment: .leading, spacing: 4) {
                                            Text(model.displayName.isEmpty ? model.modelName : model.displayName).lineLimit(1)
                                            Text(model.modelName).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                        }
                                        Spacer(minLength: 0)
                                        if preference.isDefault(model.id) {
                                            Image(systemName: "checkmark.circle.fill")
                                                .foregroundStyle(MacDesignColor.brand)
                                                .accessibilityLabel(String(localized: "Default for chat", table: "ModelCenter"))
                                        }
                                        if !savedEndpoints.flatMap(\.models).contains(model)
                                            || pendingModelParameters[model.id] != nil
                                            || editing.parameterDrafts[model.id]?.isEdited == true {
                                            Image(systemName: "circle.fill").font(.system(size: 6))
                                                .accessibilityLabel(String(localized: "Unsaved changes", table: "ModelCenter"))
                                        }
                                    }
                                    .padding(12)
                                    .contentShape(Rectangle())
                                    .macSelectionHighlight(isSelected: selectedModelID == model.id)
                                }
                                .buttonStyle(MacActionButtonStyle(isSelected: selectedModelID == model.id, contentInsets: .init()))
                                .accessibilityIdentifier("model-library-" + model.id)
                                .id(MacRemoteModelDestination.model(connectionID: endpoint.id, profileID: model.id))
                                .padding(.leading, 12)
                            }
                        }
                    }
                    if endpoints.isEmpty {
                        Text(String(localized: "Add a connection to get started.", table: "ModelCenter"))
                            .foregroundStyle(.secondary).padding()
                    }
                }
                .scrollTargetLayout()
                .padding(.horizontal, 10)
            }
            .scrollPosition(id: $editing.libraryScrollID)
            .onMoveCommand { direction in
                guard direction == .up || direction == .down else { return }
                let destinations = endpoints.flatMap { endpoint -> [MacRemoteModelDestination] in
                    let matches = modelSearch.isEmpty || endpoint.name.localizedCaseInsensitiveContains(modelSearch)
                    let models = endpoint.models.filter { matches || $0.modelName.localizedCaseInsensitiveContains(modelSearch) || $0.displayName.localizedCaseInsensitiveContains(modelSearch) }
                    guard matches || !models.isEmpty else { return [] }
                    return [.connection(endpoint.id)] + models.map { .model(connectionID: endpoint.id, profileID: $0.id) }
                }
                guard !destinations.isEmpty else { return }
                let current = selectedModelID.map { MacRemoteModelDestination.model(connectionID: selectedEndpointID ?? "", profileID: $0) }
                    ?? selectedEndpointID.map(MacRemoteModelDestination.connection)
                let index = current.flatMap { destinations.firstIndex(of: $0) } ?? -1
                let target = destinations[min(max(index + (direction == .down ? 1 : -1), 0), destinations.count - 1)]
                requestNavigation(target)
                editing.libraryScrollID = target
            }
        }
        .padding(.top, 16)
    }

    private func selectEndpoint(_ id: String) {
        requestNavigation(.connection(id))
    }

    private func requestNavigation(_ destination: MacRemoteModelDestination) {
        if editing.requestNavigation(to: destination) {
            confirmsSelectionChange = true
            return
        }
        finishPendingNavigation()
    }

    private func finishPendingNavigation() {
        guard let destination = editing.pendingDestination else { return }
        if selectedEndpointID != destination.connectionID {
            needsReload = false
            notice = nil
        }
        if destination == .newConnection {
            editing.pendingDestination = nil
            createEndpoint()
        } else {
            editing.navigate(to: destination)
            if let id = destination.connectionID { expandedEndpointSettings.insert(id) }
        }
        detailSection = .basic
        loadParameterDraft()
    }

    @ViewBuilder
    private var selectedDetail: some View {
        if let index = endpoints.firstIndex(where: { $0.id == selectedEndpointID }) {
            if let modelIndex = endpoints[index].models.firstIndex(where: { $0.id == selectedModelID }) {
                modelDetail(endpointIndex: index, modelIndex: modelIndex)
                    .id(endpoints[index].models[modelIndex].id)
            } else {
                ScrollView {
                    endpointCard(index: index)
                        .padding(20)
                        .frame(maxWidth: 850)
                        .frame(maxWidth: .infinity, alignment: .top)
                }
                .id(endpoints[index].id)
            }
        } else {
            ContentUnavailableView {
                Label(String(localized: "Select a model or connection", table: "ModelCenter"), systemImage: "cube")
            } description: {
                Text(String(localized: "Connection settings are shared; each model has its own parameters and capabilities.", table: "ModelCenter"))
            }
        }
    }

    @ViewBuilder
    private var managementActions: some View {
        Button(action: addEndpoint) {
            Label(String(localized: "Add Endpoint", table: "ModelCenter"), systemImage: "plus")
        }
        .disabled(loadError != nil)
        if hasUnsavedChanges {
            Button(String(localized: "Discard Changes", table: "ModelCenter")) {
                discardPending = true
            }
            .disabled(isSaving || loadError != nil)
        }
        Button {
            save()
        } label: {
            if isSaving {
                ProgressView().controlSize(.small)
            } else {
                Label(
                    hasUnsavedChanges ? String(localized: "Save connection", table: "ModelCenter") : String(localized: "Saved", table: "ModelCenter"),
                    systemImage: hasUnsavedChanges ? "square.and.arrow.down" : "checkmark.circle.fill"
                )
            }
        }
        .buttonStyle(.borderedProminent)
        .disabled(!hasUnsavedChanges || isSaving || loadError != nil || needsReload || draftValidationMessage != nil)
    }

    private var guidanceCard: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "arrow.triangle.branch")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(MacDesignColor.brand)
                .frame(width: 28, height: 28)
                .background(MacDesignColor.brand.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 6) {
                Text(String(localized: "Routing policy", table: "ModelCenter"))
                    .font(.callout.weight(.semibold))
                Text(String(localized: "Higher priority models are preferred. Equal-priority models follow the endpoint and model order shown below. Sensitive data is opt-in and disabled by default.", table: "ModelCenter"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(14)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }

    @ViewBuilder
    private func endpointCard(index: Int) -> some View {
        let initial = endpoints[index]
        let endpoint = Binding(
            get: { endpoints.first(where: { $0.id == initial.id }) ?? initial },
            set: { value in
                guard let index = endpoints.firstIndex(where: { $0.id == initial.id }) else { return }
                endpoints[index] = value
            }
        )
        VStack(alignment: .leading, spacing: 14) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 16) {
                    endpointIdentityButton(endpoint)
                        .frame(minWidth: 220)
                    Spacer(minLength: 0)
                    endpointActions(endpoint, index: index)
                }
                VStack(alignment: .leading, spacing: 12) {
                    endpointIdentityButton(endpoint)
                    endpointCompactActions(endpoint, index: index)
                }
            }

            Divider().opacity(0.5)

            MacModelCenterDisclosure(title: String(localized: "Endpoint settings", table: "ModelCenter"), symbol: "slider.horizontal.3", isExpanded: Binding(
                get: { expandedEndpointSettings.contains(endpoint.wrappedValue.id) },
                set: { expanded in
                    if expanded { expandedEndpointSettings.insert(endpoint.wrappedValue.id) }
                    else { expandedEndpointSettings.remove(endpoint.wrappedValue.id) }
                }
            )) {
                endpointFields(endpoint)
                    .padding(.top, MacDesignTokens.Spacing.sm)
            }
            .accessibilityIdentifier("endpoint-settings-\(endpoint.wrappedValue.id)")
            endpointModels(endpoint, index: index)
        }
        .padding(16)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }

    private func endpointIdentity(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
            let endpointURL = endpoint.wrappedValue.baseURL
            let summaryURL = endpointURL.isEmpty
                ? "https://api.example.com/v1"
                : MacRemoteEndpointURL.isValid(endpointURL)
                    ? endpointURL
                    : String(localized: "Check endpoint URL", table: "ModelCenter")
            return HStack(spacing: 10) {
                Image(systemName: "network")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(MacDesignColor.brand)
                    .frame(width: 30, height: 30)
                    .background(MacDesignColor.brand.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 2) {
                    Text(endpoint.wrappedValue.name.isEmpty
                        ? String(localized: "New Endpoint", table: "ModelCenter")
                        : endpoint.wrappedValue.name)
                        .font(.headline)
                        .lineLimit(2)
                        .help(endpoint.wrappedValue.name)
                    Text(summaryURL)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(summaryURL)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
    }

    private func endpointIdentityButton(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
        Button {
            let id = endpoint.wrappedValue.id
            if !expandedEndpointSettings.insert(id).inserted {
                expandedEndpointSettings.remove(id)
            }
        } label: {
            endpointIdentity(endpoint)
                .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("endpoint-identity-\(endpoint.wrappedValue.id)")
    }

    private func endpointActions(_ endpoint: Binding<MacRemoteEndpointConfiguration>, index: Int) -> some View {
            HStack(spacing: 10) {
                Text(endpoint.wrappedValue.wireProtocol.title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Toggle(String(localized: "Enable Endpoint", table: "ModelCenter"), isOn: endpoint.isEnabled)
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .tint(MacDesignColor.brand)
                Button(role: .destructive) { deletionEndpointID = endpoint.wrappedValue.id } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                .help(String(localized: "Delete Endpoint", table: "ModelCenter"))
                .accessibilityLabel(String(localized: "Delete Endpoint", table: "ModelCenter"))
            }
            .fixedSize(horizontal: true, vertical: false)
    }

    private func endpointCompactActions(_ endpoint: Binding<MacRemoteEndpointConfiguration>, index: Int) -> some View {
        HStack(spacing: 12) {
            Text(endpoint.wrappedValue.wireProtocol.title)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Toggle("", isOn: endpoint.isEnabled)
                .toggleStyle(.switch)
                .controlSize(.small)
                .tint(MacDesignColor.brand)
                .labelsHidden()
                .accessibilityLabel(String(localized: "Enable Endpoint", table: "ModelCenter"))
            Spacer(minLength: 0)
            Menu {
                Button(String(localized: "Delete Endpoint", table: "ModelCenter"), role: .destructive) {
                    deletionEndpointID = endpoint.wrappedValue.id
                }
            } label: {
                Label(String(localized: "More"), systemImage: "ellipsis.circle")
            }
        }
    }

    private func endpointFields(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
            ViewThatFits(in: .horizontal) {
                endpointFieldsGrid(endpoint)
                VStack(alignment: .leading, spacing: MacDesignTokens.Spacing.md) {
                    Picker(String(localized: "API protocol"), selection: endpoint.wireProtocol) {
                        ForEach(MacModelWireProtocol.allCases) { value in
                            Text(value.title).tag(value)
                        }
                    }
                    TextField(String(localized: "Endpoint name", table: "ModelCenter"), text: endpoint.name)
                        .textFieldStyle(.roundedBorder)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "Base URL", table: "ModelCenter"))
                            .foregroundStyle(.secondary)
                        endpointURLField(endpoint)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "API Key", table: "ModelCenter"))
                            .foregroundStyle(.secondary)
                        SecureField(String(localized: "Optional for local endpoints", table: "ModelCenter"), text: endpoint.apiKey)
                            .textFieldStyle(.roundedBorder)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.callout)
    }

    private func endpointFieldsGrid(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    Text(String(localized: "API protocol"))
                        .foregroundStyle(.secondary)
                    Picker(String(localized: "API protocol"), selection: endpoint.wireProtocol) {
                        ForEach(MacModelWireProtocol.allCases) { value in
                            Text(value.title).tag(value)
                        }
                    }.labelsHidden()
                }
                GridRow {
                    Text(String(localized: "Name", table: "ModelCenter"))
                        .foregroundStyle(.secondary)
                    TextField(String(localized: "Endpoint name", table: "ModelCenter"), text: endpoint.name)
                        .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text(String(localized: "Base URL", table: "ModelCenter"))
                        .foregroundStyle(.secondary)
                    endpointURLField(endpoint)
                }
                GridRow {
                    Text(String(localized: "API Key", table: "ModelCenter"))
                        .foregroundStyle(.secondary)
                    SecureField(String(localized: "Optional for local endpoints", table: "ModelCenter"), text: endpoint.apiKey)
                        .textFieldStyle(.roundedBorder)
                }
            }
    }

    private func endpointURLField(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            TextField("https://api.example.com/v1", text: endpoint.baseURL)
                .textFieldStyle(.roundedBorder)
            if !endpoint.wrappedValue.baseURL.isEmpty,
               !MacRemoteEndpointURL.isValid(endpoint.wrappedValue.baseURL) {
                Text(String(localized: "Enter a valid HTTP(S) base URL without a username, password or fragment.", table: "ModelCenter"))
                    .font(.caption)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private func endpointModels(_ endpoint: Binding<MacRemoteEndpointConfiguration>, index: Int) -> some View {
            ViewThatFits(in: .horizontal) {
                HStack {
                    endpointModelsTitle(endpoint)
                    Spacer(minLength: 8)
                    endpointModelsActions(endpoint, index: index)
                }
                VStack(alignment: .leading, spacing: 8) {
                    endpointModelsTitle(endpoint)
                    endpointModelsActions(endpoint, index: index)
                }
            }
            .controlSize(.small)

            discoveryResults(endpoint.wrappedValue, index: index)

            if endpoint.wrappedValue.models.isEmpty {
                Text(String(localized: "Discover models, choose one to configure, then save. You can also enter a model ID manually.", table: "ModelCenter"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                ForEach(endpoint.wrappedValue.models.map(\.id), id: \.self) { modelID in
                    if let modelIndex = endpoint.wrappedValue.models.firstIndex(where: { $0.id == modelID }) {
                                Button {
                                    selectedModelID = modelID
                                    detailSection = .basic
                                } label: {
                                    HStack {
                                        Text(endpoint.wrappedValue.models[modelIndex].displayName.isEmpty
                                            ? endpoint.wrappedValue.models[modelIndex].modelName
                                            : endpoint.wrappedValue.models[modelIndex].displayName)
                                            .lineLimit(1)
                                        Spacer()
                                    }
                                    .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                                    .padding(.horizontal, 8)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(MacPressFeedbackButtonStyle())
                            .draggable(modelID) {
                                dragPreview(endpoint.wrappedValue.models[modelIndex])
                            }
                            .dropDestination(for: String.self) { items, _ in
                                guard let draggedID = items.first else { return false }
                                return reorderModel(
                                    draggedID: draggedID,
                                    endpointIndex: index,
                                    targetModelID: modelID
                                )
                            } isTargeted: { targeted in
                                draggedModelID = targeted ? modelID : nil
                            }
                            .background(
                                draggedModelID == modelID
                                    ? MacDesignColor.brand.opacity(0.08)
                                    : Color.clear,
                                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                            )
                        if modelIndex < endpoint.wrappedValue.models.index(before: endpoint.wrappedValue.models.endIndex) {
                            Divider().opacity(0.35)
                        }
                    }
                }
                Button {
                    Task { await verifyResponsesRoute(endpointID: endpoint.wrappedValue.id) }
                } label: {
                    HStack(spacing: 6) {
                        if probingEndpointID == endpoint.wrappedValue.id {
                            ProgressView().controlSize(.small)
                        }
                        Label(String(localized: "Verify Responses route"), systemImage: "checkmark.shield")
                    }
                }
                .buttonStyle(.link)
                .disabled(probingEndpointID != nil || !endpoint.wrappedValue.models.contains(where: { $0.isEnabled })
                    || !MacRemoteEndpointURL.isValid(endpoint.wrappedValue.baseURL))
                .help(String(localized: "Tests this endpoint with one configured model. A successful route test does not establish which reasoning efforts the model supports."))
            }
    }

    private func endpointModelsTitle(_ endpoint: Binding<MacRemoteEndpointConfiguration>) -> some View {
        HStack(spacing: 6) {
            Text(String(localized: "Configured models", table: "ModelCenter"))
                .font(.headline)
            Text("\(endpoint.wrappedValue.models.count)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
        }
    }

    private func endpointModelsActions(_ endpoint: Binding<MacRemoteEndpointConfiguration>, index: Int) -> some View {
        HStack(spacing: 8) {
            Button {
                Task { await discoverModels(endpointIndex: index) }
            } label: {
                if loadingEndpointID == endpoint.wrappedValue.id {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text(String(localized: "Discovering models…", table: "ModelCenter"))
                    }
                } else {
                    Label(String(localized: "Discover", table: "ModelCenter"), systemImage: "arrow.clockwise")
                }
            }
            .disabled(loadingEndpointID != nil || !MacRemoteEndpointURL.isValid(endpoint.wrappedValue.baseURL))
            Button { addModel(endpointIndex: index) } label: {
                Label(String(localized: "Enter model ID", table: "ModelCenter"), systemImage: "plus")
            }
        }
    }

    @ViewBuilder
    private func discoveryResults(_ endpoint: MacRemoteEndpointConfiguration, index: Int) -> some View {
        if let failure = discoveryFailures[endpoint.id], failure.catalog.matches(endpoint) {
            MacInlineNotice(
                title: String(localized: "Could Not Discover Models", table: "ModelCenter"),
                message: failure.message,
                kind: .failure
            )
        } else if let catalog = discoveredModels[endpoint.id], catalog.matches(endpoint) {
            VStack(alignment: .leading, spacing: 10) {
                Label(String(localized: "Model catalog loaded", table: "ModelCenter"), systemImage: "checkmark.circle.fill")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.green)
                Text(String(localized: "Discovery confirms catalog access, not model capabilities. Configure a model before saving it.", table: "ModelCenter"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if catalog.models.isEmpty {
                    Text(String(localized: "This endpoint returned an empty catalog. You can enter a model ID manually.", table: "ModelCenter"))
                        .font(.callout)
                } else {
                    if catalog.models.count > 8 {
                        TextField(String(localized: "Filter models", table: "ModelCenter"), text: Binding(
                            get: { discoverySearch[endpoint.id] ?? "" },
                            set: { discoverySearch[endpoint.id] = $0 }
                        ))
                        .textFieldStyle(.roundedBorder)
                    }
                    let query = catalog.models.count > 8 ? (discoverySearch[endpoint.id] ?? "") : ""
                    let matches = catalog.models.filter { query.isEmpty || $0.localizedCaseInsensitiveContains(query) }
                    if matches.isEmpty {
                        Text(String(localized: "No matching models", table: "ModelCenter"))
                            .foregroundStyle(.secondary)
                    }
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 8) {
                            ForEach(matches, id: \.self) { name in
                                HStack(spacing: 12) {
                                    Text(name)
                                        .font(.system(.callout, design: .monospaced))
                                        .textSelection(.enabled)
                                        .lineLimit(2)
                                        .help(name)
                                    Spacer(minLength: 8)
                                    if endpoint.models.contains(where: { $0.modelName.caseInsensitiveCompare(name) == .orderedSame }) {
                                        Label(String(localized: "Added to draft", table: "ModelCenter"), systemImage: "checkmark")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    } else {
                                        Button(String(localized: "Configure and add", table: "ModelCenter")) {
                                            addDiscoveredModel(name, endpointIndex: index)
                                        }
                                        .controlSize(.small)
                                    }
                                }
                                .padding(10)
                                .background(.background.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
                            }
                        }
                    }
                    .frame(height: CGFloat(min(max(matches.count, 1), 5)) * 58)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(MacDesignColor.surfaceRaised, in: RoundedRectangle(cornerRadius: 12))
        }
    }

    @ViewBuilder
    private func modelDetail(endpointIndex: Int, modelIndex: Int) -> some View {
        let endpointID = endpoints[endpointIndex].id
        let initial = endpoints[endpointIndex].models[modelIndex]
        let model = Binding(
            get: { endpoints.first(where: { $0.id == endpointID })?.models.first(where: { $0.id == initial.id }) ?? initial },
            set: { value in
                guard let endpointIndex = endpoints.firstIndex(where: { $0.id == endpointID }),
                      let modelIndex = endpoints[endpointIndex].models.firstIndex(where: { $0.id == initial.id }) else { return }
                endpoints[endpointIndex].models[modelIndex] = value
            }
        )
        ScrollView {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    modelEnabledControl(model)
                    modelRowIdentity(model)
                        .frame(minWidth: 210, maxWidth: .infinity, alignment: .leading)
                    modelRowActions(model, endpointIndex: endpointIndex, modelIndex: modelIndex)
                }
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 10) {
                        modelEnabledControl(model)
                        modelRowIdentity(model)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    HStack {
                        Spacer(minLength: 0)
                        modelRowActions(model, endpointIndex: endpointIndex, modelIndex: modelIndex)
                    }
                }
            }

            HStack {
                Button(endpoints[endpointIndex].name) { selectEndpoint(endpointID) }
                    .buttonStyle(.link)
                Spacer()
                MacDefaultChatModelButton(preference: preference, profileID: model.wrappedValue.id)
            }
            if !model.wrappedValue.hasExecutionCapabilityDeclaration,
               let status = model.wrappedValue.executionCapabilityStatus.label {
                HStack(spacing: 8) {
                    Label(status, systemImage: model.wrappedValue.hasExecutionCapabilityDeclaration
                        ? "bubble.left" : "exclamationmark.circle")
                        .font(.caption)
                        .foregroundStyle(model.wrappedValue.hasExecutionCapabilityDeclaration
                            ? Color.secondary : Color.orange)
                    if !model.wrappedValue.hasExecutionCapabilityDeclaration {
                        Button(String(localized: "Review capabilities", table: "ModelCenter")) {
                            detailSection = .advanced
                        }
                        .buttonStyle(.link)
                    }
                }
            }

            let unavailableUntil = configurationStore.unavailableUntil(
                profileID: model.wrappedValue.id
            )
            if unavailableUntil > .now {
                Label {
                    Text(
                        String(
                            format: String(localized: "Paused until %@", table: "ModelCenter"),
                            unavailableUntil.formatted(date: .abbreviated, time: .shortened)
                        )
                    )
                } icon: {
                    Image(systemName: "clock.badge.exclamationmark.fill")
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(.orange)
            }

            Picker(String(localized: "Model configuration"), selection: $detailSection) {
                ForEach(DetailSection.allCases) { section in Text(section.title).tag(section) }
            }
            .pickerStyle(.segmented)
            .padding(.vertical, 12)
                VStack(alignment: .leading, spacing: MacDesignTokens.Spacing.lg) {
                    if detailSection == .basic {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                        GridRow {
                            Text(String(localized: "Model ID", table: "ModelCenter"))
                                .foregroundStyle(.secondary)
                            modelNameField(model)
                        }
                        GridRow {
                            Text(String(localized: "Display name", table: "ModelCenter"))
                                .foregroundStyle(.secondary)
                            modelDisplayNameField(model)
                        }
                    }
                    .font(.callout)
                    Text(String(localized: "Connection settings are shared; each model has its own parameters and capabilities.", table: "ModelCenter"))
                        .font(.caption).foregroundStyle(.secondary)
                    }
                    if detailSection == .advanced {
                        LabeledContent(String(localized: "Routing priority", table: "ModelCenter")) {
                            modelPriorityControl(model)
                        }
                        Text(String(localized: "Routing priority does not change the default chat model.", table: "ModelCenter"))
                            .font(.caption).foregroundStyle(.secondary)
                        capabilityControls(model)
                        Text(String(localized: "Allowed uses select models for tasks; they do not grant access to files, devices, or private data. Concrete requests are checked by the shared permission policy.", table: "ModelCenter"))
                            .font(.callout).foregroundStyle(.secondary)
                        Divider()
                        Label(String(localized: "Recommended permission defaults", table: "ModelCenter"), systemImage: "hand.raised")
                            .font(.headline)
                        Text(String(localized: "Private data, memory, device control, browser automation and commands ask before use. Existing scoped grants and explicit denials still apply. Change the permission mode in the conversation, not in model capabilities.", table: "ModelCenter"))
                            .font(.callout).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if detailSection == .advanced {
                    executionCapabilityControls(model, wireProtocol: endpoints[endpointIndex].wireProtocol)
                    Divider()
                    MacReasoningConfigurationEditor(configuration: Binding(
                        get: { model.wrappedValue.reasoning ?? .init() },
                        set: { model.wrappedValue.reasoning = $0 }
                    ), wireProtocol: endpoints[endpointIndex].wireProtocol)
                    if model.wrappedValue.reasoning?.control != .automatic
                        && model.wrappedValue.reasoning != nil
                        && !model.wrappedValue.executionCapabilities.reasoning {
                        Label(
                            String(localized: "Declare Reasoning in Model execution capabilities before saving an effort control."),
                            systemImage: "info.circle"
                        )
                        .font(.caption)
                        .foregroundStyle(.orange)
                    }
                    }
                    if let draft = editing.parameterDrafts[initial.id] {
                        MacModelParameterDraftForm(draft: Binding(
                            get: { editing.parameterDrafts[initial.id] ?? draft },
                            set: { editing.updateParameterDraft($0, for: initial.id) }), advanced: detailSection == .advanced,
                            samplingAvailable: model.wrappedValue.executionCapabilities.samplingParameters,
                            samplingRequiresNoReasoning: model.wrappedValue.executionCapabilities.samplingRequiresNoReasoning)
                    } else if let error = editing.parameterErrors[initial.id] {
                        Text(error).foregroundStyle(.red)
                        Button(String(localized: "Reload")) { requestParameterReload() }
                            .accessibilityIdentifier("model-parameter-reload")
                    }
                }
                .padding(.vertical, MacDesignTokens.Spacing.sm)

        }
        .padding(20)
        .frame(maxWidth: 820, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .top)
        }
    }

    private func modelRowIdentity(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(model.wrappedValue.displayName.isEmpty
                 ? model.wrappedValue.modelName : model.wrappedValue.displayName)
                .font(.callout.weight(.semibold))
                .lineLimit(1)
            if !model.wrappedValue.displayName.isEmpty {
                Text(model.wrappedValue.modelName)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .help(model.wrappedValue.modelName)
    }

    private func modelIdentityButton(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        Button {
            let id = model.wrappedValue.id
            if !expandedModelSettings.insert(id).inserted {
                expandedModelSettings.remove(id)
            }
        } label: {
            modelRowIdentity(model)
                .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("model-identity-\(model.wrappedValue.id)")
    }

    private func modelRowActions(
        _ model: Binding<MacRemoteModelConfiguration>,
        endpointIndex: Int,
        modelIndex: Int
    ) -> some View {
        HStack(spacing: 10) {
            modelOrderControls(endpointIndex: endpointIndex, modelIndex: modelIndex)
            Menu {
                removeModelButton(endpointIndex: endpointIndex, modelIndex: modelIndex)
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .accessibilityLabel(String(localized: "Model", table: "ModelCenter") + ": " + model.wrappedValue.displayName)
            .accessibilityIdentifier("model-actions-\(model.wrappedValue.id)")
        }
        .fixedSize(horizontal: true, vertical: false)
    }

    private func modelEnabledControl(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        Toggle("", isOn: model.isEnabled)
            .labelsHidden()
            .accessibilityLabel(String(localized: "Enable Model", table: "ModelCenter"))
            .toggleStyle(.switch)
            .controlSize(.mini)
            .tint(MacDesignColor.brand)
    }

    private func modelNameField(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        TextField(String(localized: "Model ID", table: "ModelCenter"), text: model.modelName)
            .textFieldStyle(.roundedBorder)
            .accessibilityLabel(String(localized: "Model ID", table: "ModelCenter"))
    }

    private func modelDisplayNameField(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        TextField(String(localized: "Display name", table: "ModelCenter"), text: model.displayName)
            .textFieldStyle(.roundedBorder)
            .accessibilityLabel(String(localized: "Display name", table: "ModelCenter"))
    }

    private func modelPriorityControl(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        TextField("0–100", value: Binding(
            get: { model.wrappedValue.priority },
            set: { model.wrappedValue.priority = $0 }
        ), format: .number.grouping(.never))
            .textFieldStyle(.roundedBorder)
            .multilineTextAlignment(.trailing)
            .monospacedDigit()
            .frame(width: 72)
            .accessibilityLabel(String(localized: "Priority", table: "ModelCenter"))
    }

    private func modelOrderControls(endpointIndex: Int, modelIndex: Int) -> some View {
        orderButtons(
            moveUp: { moveModel(endpointIndex: endpointIndex, modelIndex: modelIndex, offset: -1) },
            moveDown: { moveModel(endpointIndex: endpointIndex, modelIndex: modelIndex, offset: 1) },
            canMoveUp: modelIndex > 0,
            canMoveDown: modelIndex < endpoints[endpointIndex].models.count - 1
        )
    }

    private func removeModelButton(endpointIndex: Int, modelIndex: Int) -> some View {
        Button(role: .destructive) {
            let modelID = endpoints[endpointIndex].models[modelIndex].id
            pendingModelParameters[modelID] = nil
            expandedModelSettings.remove(modelID)
            endpoints[endpointIndex].models.remove(at: modelIndex)
        } label: {
            Label(String(localized: "Delete", table: "ModelCenter"), systemImage: "trash")
        }
        .buttonStyle(MacActionButtonStyle())
    }

    @ViewBuilder
    private func capabilityControls(_ model: Binding<MacRemoteModelConfiguration>) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(String(localized: "Allowed uses", table: "ModelCenter"))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    HStack(spacing: 6) {
                        ForEach([MacRemoteModelCapability.chat, .retrieval, .issueRepair]) { capability in
                            capabilityToggle(capability, model: model)
                        }
                    }
                }
            }
            VStack(alignment: .leading, spacing: 10) {
                Text(String(localized: "Allowed uses", table: "ModelCenter"))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                HStack(spacing: 6) {
                    ForEach([MacRemoteModelCapability.chat, .retrieval, .issueRepair]) { capability in
                        capabilityToggle(capability, model: model)
                    }
                }
            }
        }
    }

    private func executionCapabilityControls(_ model: Binding<MacRemoteModelConfiguration>, wireProtocol: MacModelWireProtocol) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(String(localized: "Model execution capabilities", table: "ModelCenter"))
                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            Text(String(localized: "Enable only capabilities supported by this exact endpoint and model.", table: "ModelCenter"))
                .font(.caption).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 14) {
                    Toggle(String(localized: "Tool calls", table: "ModelCenter"), isOn: model.executionCapabilities.toolCalls)
                    Toggle(String(localized: "Image input", table: "ModelCenter"), isOn: model.executionCapabilities.imageInput)
                    Toggle(String(localized: "Reasoning", table: "ModelCenter"), isOn: model.executionCapabilities.reasoning)
                }
                HStack(spacing: 14) {
                    Toggle(String(localized: "Sampling parameters", table: "ModelCenter"), isOn: model.executionCapabilities.samplingParameters)
                    Toggle(String(localized: "Sampling only with reasoning off", table: "ModelCenter"), isOn: model.executionCapabilities.samplingRequiresNoReasoning)
                        .disabled(!model.wrappedValue.executionCapabilities.samplingParameters)
                }
                if wireProtocol == .chatCompletions {
                Picker(String(localized: "Chat output limit field", table: "ModelCenter"), selection: Binding(
                    get: { model.wrappedValue.executionCapabilities.chatOutputTokenField ?? .auto },
                    set: { model.wrappedValue.executionCapabilities.chatOutputTokenField = $0 }
                )) {
                    Text(String(localized: "Automatic", table: "ModelCenter")).tag(MacChatOutputTokenField.auto)
                    Text("max_tokens").tag(MacChatOutputTokenField.maxTokens)
                    Text("max_completion_tokens").tag(MacChatOutputTokenField.maxCompletionTokens)
                }
                }
            }
            .toggleStyle(.checkbox)
            .font(.caption)
            Text(String(localized: "Unchecked capabilities are explicitly unsupported. Allowed uses do not enable tools.", table: "ModelCenter"))
                .font(.caption).foregroundStyle(.secondary)
            if !model.wrappedValue.hasExecutionCapabilityDeclaration {
                Button(String(localized: "Confirm capability settings", table: "ModelCenter")) {
                    model.wrappedValue.confirmExecutionCapabilities()
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }
        }
    }

    private func capabilityToggle(
        _ capability: MacRemoteModelCapability,
        model: Binding<MacRemoteModelConfiguration>
    ) -> some View {
        let enabled = model.wrappedValue.capabilities.contains(capability)
        return Button {
            setCapability(capability, enabled: !enabled, model: model)
        } label: {
            capabilityBadge(capability)
                .foregroundStyle(enabled ? MacDesignColor.brand : Color.secondary)
                .padding(.horizontal, 9)
                .padding(.vertical, 6)
                .background(
                    enabled ? MacDesignColor.brand.opacity(0.15) : Color.secondary.opacity(0.07),
                    in: Capsule()
                )
                .overlay(Capsule().stroke(enabled ? MacDesignColor.brand.opacity(0.55) : Color.secondary.opacity(0.18)))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(enabled ? .isSelected : [])
    }


    private func setCapability(
        _ capability: MacRemoteModelCapability,
        enabled: Bool,
        model: Binding<MacRemoteModelConfiguration>
    ) {
        if enabled { model.wrappedValue.capabilities.insert(capability) }
        else { model.wrappedValue.capabilities.remove(capability) }
    }

    private func capabilityBadge(_ capability: MacRemoteModelCapability) -> some View {
        Label(capabilityTitle(capability), systemImage: capabilitySymbol(capability))
            .font(.caption.weight(.medium))
    }

    private func capabilityTitle(_ capability: MacRemoteModelCapability) -> String {
        switch capability {
        case .chat: return String(localized: "Chat", table: "ModelCenter")
        case .retrieval: return String(localized: "Retrieval", table: "ModelCenter")
        case .issueRepair: return String(localized: "Issue Repair", table: "ModelCenter")
        case .sensitiveData: return String(localized: "Sensitive Data", table: "ModelCenter")
        }
    }

    private func capabilitySymbol(_ capability: MacRemoteModelCapability) -> String {
        switch capability {
        case .chat: return "bubble.left.and.bubble.right"
        case .retrieval: return "magnifyingglass"
        case .issueRepair: return "wrench.and.screwdriver"
        case .sensitiveData: return "lock.shield"
        }
    }

    private var dragHandle: some View {
        Image(systemName: "line.3.horizontal")
            .foregroundStyle(.tertiary)
            .frame(width: 18)
            .help(String(localized: "Drag to reorder models with the same priority.", table: "ModelCenter"))
    }

    private func dragPreview(_ model: MacRemoteModelConfiguration) -> some View {
        Label(model.displayName.isEmpty ? model.modelName : model.displayName, systemImage: "line.3.horizontal")
            .font(.callout.weight(.semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
    }

    private func orderButtons(
        moveUp: @escaping () -> Void,
        moveDown: @escaping () -> Void,
        canMoveUp: Bool,
        canMoveDown: Bool
    ) -> some View {
        HStack(spacing: 2) {
            Button(action: moveUp) { Image(systemName: "chevron.up") }
                .disabled(!canMoveUp)
                .accessibilityLabel(String(localized: "Move Up", table: "ModelCenter"))
            Button(action: moveDown) { Image(systemName: "chevron.down") }
                .disabled(!canMoveDown)
                .accessibilityLabel(String(localized: "Move Down", table: "ModelCenter"))
        }
        .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
        .controlSize(.small)
    }

    private func addEndpoint() {
        requestNavigation(.newConnection)
    }

    private func createEndpoint() {
        let endpoint = MacRemoteEndpointConfiguration(
            name: String(localized: "New Endpoint", table: "ModelCenter"),
            order: endpoints.count,
            models: []
        )
        endpoints.append(endpoint)
        expandedEndpointSettings.insert(endpoint.id)
        selectedEndpointID = endpoint.id
        selectedModelID = nil
        compactShowsDetail = true
    }

    private func addModel(endpointIndex: Int) {
        guard endpoints.indices.contains(endpointIndex) else { return }
        let endpoint = endpoints[endpointIndex]
        addModelRequest = MacAddRemoteModelRequest(
            endpointID: endpoint.id,
            discoveredModels: availableDiscoveredModels(for: endpoint),
            initialModelID: ""
        )
    }

    private func addDiscoveredModel(_ name: String, endpointIndex: Int) {
        guard endpoints.indices.contains(endpointIndex) else { return }
        guard !endpoints[endpointIndex].models.contains(where: { $0.modelName == name }) else { return }
        let endpoint = endpoints[endpointIndex]
        let discovered = availableDiscoveredModels(for: endpoint)
        guard discovered.contains(name) else { return }
        addModelRequest = MacAddRemoteModelRequest(
            endpointID: endpoint.id,
            discoveredModels: discovered,
            initialModelID: name
        )
    }

    private func addModel(_ result: MacAddRemoteModelResult, endpointID: String) {
        guard let endpointIndex = endpoints.firstIndex(where: { $0.id == endpointID }) else { return }
        guard !endpoints[endpointIndex].models.contains(where: {
            $0.modelName.caseInsensitiveCompare(result.configuration.modelName) == .orderedSame
        }) else {
            notice = MacRemoteModelManagerNotice(
                title: String(localized: "Model Already Added", table: "ModelCenter"),
                message: result.configuration.modelName,
                kind: .warning
            )
            return
        }
        var configuration = result.configuration
        configuration.order = endpoints[endpointIndex].models.count
        endpoints[endpointIndex].models.append(configuration)
        expandedModelSettings.insert(configuration.id)
        pendingModelParameters[configuration.id] = result.parameters
        selectedModelID = configuration.id
        detailSection = .basic
        notice = MacRemoteModelManagerNotice(
            title: String(localized: "Model Ready to Save", table: "ModelCenter"),
            message: String(localized: "Review its routing permissions, then save the endpoint.", table: "ModelCenter"),
            kind: .information
        )
    }

    private func parameterSummary(_ parameters: ModelParameters) -> String {
        let context = parameters.maxContextLength.formatted(.number)
        let output = parameters.maxOutputTokens.formatted(.number)
        let temperature = parameters.temperature.formatted(.number.precision(.fractionLength(0...2)))
        return String(
            format: String(localized: "%@ context · %@ max output · temperature %@", table: "ModelCenter"),
            context,
            output,
            temperature
        )
    }

    private func moveModel(endpointIndex: Int, modelIndex: Int, offset: Int) {
        let destination = modelIndex + offset
        guard endpoints.indices.contains(endpointIndex),
              endpoints[endpointIndex].models.indices.contains(modelIndex),
              endpoints[endpointIndex].models.indices.contains(destination) else { return }
        endpoints[endpointIndex].models.swapAt(modelIndex, destination)
        updateModelOrder(endpointIndex: endpointIndex)
    }

    private func reorderModel(
        draggedID: String,
        endpointIndex: Int,
        targetModelID: String
    ) -> Bool {
        guard endpoints.indices.contains(endpointIndex),
              let reordered = MacRemoteModelOrdering.moving(
                endpoints[endpointIndex].models,
                draggedID: draggedID,
                onto: targetModelID
              )
        else { return false }
        endpoints[endpointIndex].models = reordered
        draggedModelID = nil
        return true
    }

    private func updateModelOrder(endpointIndex: Int) {
        guard endpoints.indices.contains(endpointIndex) else { return }
        endpoints[endpointIndex].models = MacRemoteModelOrdering.reindexed(
            endpoints[endpointIndex].models
        )
    }

    @discardableResult
    private func save() -> Bool {
        guard loadError == nil, draftValidationMessage == nil, !needsReload else { return false }
        isSaving = true
        defer { isSaving = false }
        do {
            guard let selectedEndpointID else { return false }
            let snapshot = try MacRemoteModelDraftCommit.saveConnection(
                endpointID: selectedEndpointID,
                draft: endpoints.first(where: { $0.id == selectedEndpointID }),
                baseline: savedEndpoints,
                parametersByModelID: pendingModelParameters,
                expectedRevisions: savedRevisions,
                store: configurationStore
            )
            endpoints = snapshot.endpoints
            savedEndpoints = snapshot.endpoints
            savedRevisions = snapshot.revisions
            if !endpoints.contains(where: { $0.id == selectedEndpointID }) {
                self.selectedEndpointID = endpoints.first?.id
                selectedModelID = nil
            }
            pendingModelParameters.removeAll()
            editing.parameterDrafts.removeAll()
            editing.parameterErrors.removeAll()
            expandedEndpointSettings.removeAll()
            expandedModelSettings.removeAll()
            loadParameterDraft()
            needsReload = false
            notice = MacRemoteModelManagerNotice(
                title: String(localized: "API Models Saved", table: "ModelCenter"),
                message: String(localized: "Routing order, permissions, and request parameters are now active.", table: "ModelCenter"),
                kind: .success
            )
            preference.reload()
            return true
        } catch {
            let conflict = MacRemoteModelSaveFailure.isCatalogConflict(error)
            needsReload = conflict
            notice = MacRemoteModelManagerNotice(
                title: conflict
                    ? String(localized: "API Models Changed Elsewhere", table: "ModelCenter")
                    : String(localized: "Could Not Update API Models", table: "ModelCenter"),
                message: conflict
                    ? String(localized: "Another window changed these models. Reload to review the current configuration before saving again.", table: "ModelCenter")
                    : error.localizedDescription,
                kind: conflict ? .warning : .failure,
                offersReload: conflict
            )
            return false
        }
    }

    private func reloadEndpoints() {
        do {
            let snapshot = try configurationStore.endpointSnapshot()
            endpoints = snapshot.endpoints
            savedEndpoints = snapshot.endpoints
            savedRevisions = snapshot.revisions
            pendingModelParameters.removeAll()
            editing.parameterDrafts.removeAll()
            editing.parameterErrors.removeAll()
            discoveredModels.removeAll()
            expandedEndpointSettings.removeAll()
            expandedModelSettings.removeAll()
            _ = expandFocusedProfileIfPresent()
            notice = nil
            loadError = nil
            needsReload = false
            loadParameterDraft()
        } catch {
            loadError = error.localizedDescription
        }
    }

    @discardableResult
    private func expandFocusedProfileIfPresent() -> String? {
        guard editing.lastFocusRevision != focusRevision, let focusedProfileID,
              let endpoint = endpoints.first(where: { endpoint in
                  endpoint.models.contains(where: { $0.id == focusedProfileID })
              }) else { return nil }
        editing.lastFocusRevision = focusRevision
        expandedEndpointSettings.insert(endpoint.id)
        expandedModelSettings.insert(focusedProfileID)
        requestNavigation(.model(connectionID: endpoint.id, profileID: focusedProfileID))
        return endpoint.id
    }

    private var hasUnsavedChanges: Bool {
        editing.hasUnsavedChanges
    }

    private var invalidParameterModelID: String? {
        endpoints.first(where: { $0.id == selectedEndpointID })?.models.first {
            editing.parameterErrors[$0.id] != nil
                || !(editing.parameterDrafts[$0.id]?.invalidFields.isEmpty ?? true)
        }?.id
    }

    private var draftValidationMessage: String? {
        let modelIDs = Set(endpoints.first(where: { $0.id == selectedEndpointID })?.models.map(\.id) ?? [])
        for id in modelIDs {
            let name = endpoints.flatMap(\.models).first(where: { $0.id == id })?.displayName ?? id
            if let error = editing.parameterErrors[id] { return name + ": " + error }
            if let draft = editing.parameterDrafts[id], !draft.invalidFields.isEmpty {
                return name + ": " + String(localized: "Check the highlighted generation parameters before saving.", table: "ModelCenter")
            }
            if let parameters = pendingModelParameters[id], let message = MacModelParameterValidation.message(
                parameters: parameters, seedText: parameters.seed.map(String.init) ?? "", isLocal: false,
                contextRange: ModelParameterLimits.remoteContextTokens) { return message }
        }
        do {
            try MacRemoteModelDraftCommit.validateConnection(
                endpointID: selectedEndpointID, drafts: endpoints, store: configurationStore)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    private func loadParameterDraft() {
        guard let id = selectedModelID, editing.parameterDrafts[id] == nil else { return }
        do {
            let parameters: ModelParameters
            if let pending = pendingModelParameters[id] { parameters = pending }
            else {
                guard let profile = try application.modelProfile(id), profile.deletedAtMs == nil,
                      savedRevisions[id] == profile.revision else {
                    throw MacApplicationServiceError.rejected("revision_conflict: model catalog parameters changed")
                }
                parameters = try ModelParameters.decodeProfile(profile.parametersJSON)
            }
            editing.parameterDrafts[id] = MacModelParameterDraft(parameters: parameters)
            editing.parameterErrors[id] = nil
        } catch {
            editing.parameterErrors[id] = error.localizedDescription
            if MacRemoteModelSaveFailure.isCatalogConflict(error) { needsReload = true }
        }
    }

    private func requestParameterReload() {
        if hasUnsavedChanges { conflictReloadPending = true }
        else { reloadEndpoints() }
    }

    private var persistedModelIDs: Set<String> {
        Set(savedEndpoints.flatMap(\.models).map(\.id))
    }

    private func availableDiscoveredModels(for endpoint: MacRemoteEndpointConfiguration) -> [String] {
        let existing = Set(endpoint.models.map { $0.modelName.lowercased() })
        return (discoveredModels[endpoint.id]?.models(for: endpoint) ?? []).filter {
            !existing.contains($0.lowercased())
        }
    }

    @MainActor
    private func discoverModels(endpointIndex: Int) async {
        guard loadingEndpointID == nil, endpoints.indices.contains(endpointIndex) else { return }
        let endpoint = endpoints[endpointIndex]
        let value = endpoint.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard MacRemoteEndpointURL.isValid(value), let baseURL = URL(string: value) else {
            notice = MacRemoteModelManagerNotice(
                title: String(localized: "Could Not Discover Models", table: "ModelCenter"),
                message: String(localized: "Enter a valid endpoint URL before discovering models.", table: "ModelCenter"),
                kind: .warning
            )
            return
        }
        loadingEndpointID = endpoint.id
        discoveryFailures[endpoint.id] = nil
        discoveredModels[endpoint.id] = nil
        defer { loadingEndpointID = nil }
        do {
            let available = try await fetchModelIDs(endpoint: endpoint, baseURL: baseURL)
            guard let current = endpoints.first(where: { $0.id == endpoint.id }),
                  MacDiscoveredModelCatalog(endpoint: endpoint, models: []).matches(current)
            else { return }
            let discovered = Array(Set(available.filter { !$0.isEmpty })).sorted()
            discoveredModels[endpoint.id] = MacDiscoveredModelCatalog(endpoint: endpoint, models: discovered)
        } catch {
            guard let current = endpoints.first(where: { $0.id == endpoint.id }),
                  MacDiscoveredModelCatalog(endpoint: endpoint, models: []).matches(current) else { return }
            discoveryFailures[endpoint.id] = (
                MacDiscoveredModelCatalog(endpoint: endpoint, models: []),
                MacNetworkFailure.message(for: error)
            )
        }
    }

    private func fetchModelIDs(
        endpoint: MacRemoteEndpointConfiguration,
        baseURL: URL
    ) async throws -> [String] {
        try await MacModelCatalogClient.fetch(baseURL: baseURL, apiKey: endpoint.apiKey)
    }

    @MainActor
    private func verifyResponsesRoute(endpointID: String) async {
        guard probingEndpointID == nil,
              let endpoint = endpoints.first(where: { $0.id == endpointID }),
              let model = endpoint.models.first(where: { $0.isEnabled && !$0.modelName.isEmpty }),
              let baseURL = URL(string: endpoint.baseURL),
              MacRemoteEndpointURL.isValid(endpoint.baseURL) else { return }
        probingEndpointID = endpointID
        defer { probingEndpointID = nil }
        do {
            var request = URLRequest(url: MacResponsesRouteProbe.url(for: baseURL))
            request.httpMethod = "POST"
            request.timeoutInterval = 25
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            if !endpoint.apiKey.isEmpty {
                request.setValue("Bearer \(endpoint.apiKey)", forHTTPHeaderField: "Authorization")
            }
            request.httpBody = try JSONSerialization.data(withJSONObject: [
                "model": model.modelName,
                "input": "Reply OK.",
                "max_output_tokens": 64,
                "store": false,
            ])
            let (data, response) = try await URLSession.shared.data(for: request)
            guard selectedEndpointID == endpointID,
                  let current = endpoints.first(where: { $0.id == endpointID }),
                  MacDiscoveredModelCatalog(endpoint: endpoint, models: []).matches(current),
                  current.models.contains(where: { $0.id == model.id && $0.modelName == model.modelName }) else { return }
            guard let http = response as? HTTPURLResponse,
                  (200...299).contains(http.statusCode),
                  MacResponsesRouteProbe.accepts(data) else {
                notice = MacRemoteModelManagerNotice(
                    title: String(localized: "Responses route not verified"),
                    message: String(localized: "The endpoint did not return a valid Responses envelope for this model. The saved protocol was not changed."),
                    kind: .warning
                )
                return
            }
            guard let index = endpoints.firstIndex(where: { $0.id == endpointID }),
                  endpoints[index].baseURL == endpoint.baseURL,
                  endpoints[index].apiKey == endpoint.apiKey,
                  endpoints[index].models.contains(where: { $0.id == model.id && $0.modelName == model.modelName })
            else { return }
            guard endpoints[index].wireProtocol == endpoint.wireProtocol else { return }
            notice = MacRemoteModelManagerNotice(
                title: String(localized: "Responses route verified"),
                message: String(localized: "Responses is available for this model. Select it in connection settings if needed. This test does not change your protocol or verify reasoning effort support.", table: "ModelCenter"),
                kind: .success
            )
        } catch {
            guard selectedEndpointID == endpointID,
                  let current = endpoints.first(where: { $0.id == endpointID }),
                  MacDiscoveredModelCatalog(endpoint: endpoint, models: []).matches(current),
                  current.models.contains(where: { $0.id == model.id && $0.modelName == model.modelName }) else { return }
            notice = MacRemoteModelManagerNotice(
                title: String(localized: "Responses route not verified"),
                message: MacNetworkFailure.message(for: error),
                kind: .failure
            )
        }
    }
}

struct MacAddRemoteModelSheet: View {
    enum Step: String, CaseIterable, Identifiable {
        case identity
        case requestBehavior

        var id: String { rawValue }

        var title: String {
            switch self {
            case .identity: String(localized: "Model", table: "ModelCenter")
            case .requestBehavior: String(localized: "Request behavior", table: "ModelCenter")
            }
        }
    }

    private static let manualSelection = "__manual_model__"

    let request: MacAddRemoteModelRequest
    let onAdd: (MacAddRemoteModelResult) -> Void

    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var catalogSelection: String
    @State private var modelID: String
    @State private var displayName: String
    @State private var capabilities: Set<MacRemoteModelCapability> = [.chat, .retrieval, .issueRepair]
    @State private var executionCapabilities = MacModelExecutionCapabilities(toolCalls: true)
    @State private var preset: MacModelParameterPreset = .balanced
    @State private var numericText: [String: String] = [:]
    @State private var invalidNumericFields: Set<String> = []
    @State private var parameters = ModelParameters.webAlignedCloudDefaults
    @State private var showsAdvancedParameters = false
    @State private var step: Step
    @State private var showsDiscardConfirmation = false

    init(
        request: MacAddRemoteModelRequest,
        initialStep: Step = .identity,
        initiallyShowsAdvancedParameters: Bool = false,
        onAdd: @escaping (MacAddRemoteModelResult) -> Void
    ) {
        self.request = request
        self.onAdd = onAdd
        let initial = request.initialModelID
        _catalogSelection = State(initialValue: initial.isEmpty ? Self.manualSelection : initial)
        _modelID = State(initialValue: initial)
        _displayName = State(initialValue: initial)
        _step = State(initialValue: initialStep)
        _showsAdvancedParameters = State(initialValue: initiallyShowsAdvancedParameters)
    }

    private var normalizedModelID: String {
        modelID.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                MacIconTile(symbol: "plus", tint: MacDesignColor.brand)
                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "Add API Model", table: "ModelCenter"))
                        .font(.title3.weight(.semibold))
                    Text(String(localized: "Enter a model ID to add it. Common uses and tool calls are enabled by default; advanced settings are optional.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(MacDesignTokens.Layout.pageGutter)

            Divider().opacity(0.55)

            Picker(String(localized: "Request behavior", table: "ModelCenter"), selection: $step) {
                ForEach(Step.allCases) { value in
                    Text(value.title).tag(value)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .frame(maxWidth: 360)
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.vertical, MacDesignTokens.Spacing.md)

            Group {
                switch step {
                case .identity:
                    identityForm
                case .requestBehavior:
                    requestBehaviorForm
                }
            }
            .id(step)
            .transition(.opacity)
            .animation(reduceMotion ? nil : MacDesignTokens.Motion.pageChange, value: step)

            Divider().opacity(0.55)
            HStack(spacing: 8) {
                if let currentValidationMessage {
                    Label(currentValidationMessage, systemImage: "exclamationmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                        .transition(.opacity.combined(with: .move(edge: .bottom)))
                }
                Spacer()
                Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) {
                    if hasUnsavedChanges { showsDiscardConfirmation = true }
                    else { dismiss() }
                }
                .keyboardShortcut(.cancelAction)
                .accessibilityIdentifier("model-add.cancel")
                if step == .requestBehavior {
                Button(String(localized: "Back")) {
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.pageChange) {
                            step = .identity
                        }
                    }
                }
                Button(primaryActionTitle) {
                    performPrimaryAction()
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(currentValidationMessage != nil)
                .accessibilityIdentifier("model-add.primary")
            }
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.vertical, MacDesignTokens.Spacing.md)
        }
        .frame(minWidth: 480, idealWidth: 620, maxWidth: 760, minHeight: 440, idealHeight: 680, maxHeight: 780)
        .background(MacPageBackground())
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: currentValidationMessage)
        .interactiveDismissDisabled(hasUnsavedChanges)
        .confirmationDialog(String(localized: "Discard unsaved changes?", table: "ModelCenter"), isPresented: $showsDiscardConfirmation, titleVisibility: .visible) {
            Button(String(localized: "Discard Changes", table: "ModelCenter"), role: .destructive) { dismiss() }
            Button(String(localized: "Keep Editing", table: "ModelCenter"), role: .cancel) {}
        }
    }

    private var hasUnsavedChanges: Bool {
        modelID != request.initialModelID || displayName != request.initialModelID
            || capabilities != [.chat, .retrieval, .issueRepair]
            || executionCapabilities != MacModelExecutionCapabilities(toolCalls: true)
            || parameters != .webAlignedCloudDefaults || !invalidNumericFields.isEmpty
    }

    private var identityForm: some View {
        Form {
            Section(String(localized: "Model", table: "ModelCenter")) {
                if !request.discoveredModels.isEmpty {
                    Picker(String(localized: "Provider catalog", table: "ModelCenter"), selection: $catalogSelection) {
                        Text(String(localized: "Enter model ID manually", table: "ModelCenter"))
                            .tag(Self.manualSelection)
                        ForEach(request.discoveredModels, id: \.self) { name in
                            Text(name).tag(name)
                        }
                    }
                    .onChange(of: catalogSelection) { _, selection in
                        guard selection != Self.manualSelection else { return }
                        let previousModelID = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
                        modelID = selection
                        let normalizedDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
                        if normalizedDisplayName.isEmpty || normalizedDisplayName == previousModelID {
                            displayName = selection
                        }
                    }
                }
                LabeledContent(String(localized: "Model ID", table: "ModelCenter")) {
                    TextField("", text: $modelID, prompt: Text(verbatim: "gpt-4.1-mini"))
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 300)
                        .onChange(of: modelID) { _, value in
                            guard catalogSelection != Self.manualSelection,
                                  value.trimmingCharacters(in: .whitespacesAndNewlines) != catalogSelection else { return }
                            catalogSelection = Self.manualSelection
                        }
                }
                LabeledContent(String(localized: "Display name", table: "ModelCenter")) {
                    TextField(
                        "",
                        text: $displayName,
                        prompt: Text(String(localized: "Optional", table: "ModelCenter"))
                    )
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 300)
                }
            }

            Section(String(localized: "Allowed uses", table: "ModelCenter")) {
                capabilityToggle(.chat, title: String(localized: "Chat", table: "ModelCenter"), symbol: "bubble.left.and.bubble.right")
                capabilityToggle(.retrieval, title: String(localized: "Retrieval", table: "ModelCenter"), symbol: "magnifyingglass")
                capabilityToggle(.issueRepair, title: String(localized: "Issue Repair", table: "ModelCenter"), symbol: "wrench.and.screwdriver")
            }

            Section(String(localized: "Access and authorization", table: "ModelCenter")) {
                Text(String(localized: "Allowed uses select models for tasks; they do not grant access to files, devices, or private data. Concrete requests are checked by the shared permission policy.", table: "ModelCenter"))
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }

    private var requestBehaviorForm: some View {
        Form {
            Section(String(localized: "Model execution capabilities", table: "ModelCenter")) {
                Text(String(localized: "Enable only capabilities supported by this exact endpoint and model.", table: "ModelCenter"))
                    .font(.caption).foregroundStyle(.secondary)
                Toggle(String(localized: "Tool calls", table: "ModelCenter"), isOn: $executionCapabilities.toolCalls)
                Toggle(String(localized: "Image input", table: "ModelCenter"), isOn: $executionCapabilities.imageInput)
                Toggle(String(localized: "Reasoning", table: "ModelCenter"), isOn: $executionCapabilities.reasoning)
                Toggle(String(localized: "Sampling parameters", table: "ModelCenter"), isOn: $executionCapabilities.samplingParameters)
                Picker(String(localized: "Chat output limit field", table: "ModelCenter"), selection: Binding(
                    get: { executionCapabilities.chatOutputTokenField ?? .auto },
                    set: { executionCapabilities.chatOutputTokenField = $0 }
                )) {
                    Text(String(localized: "Automatic", table: "ModelCenter")).tag(MacChatOutputTokenField.auto)
                    Text("max_tokens").tag(MacChatOutputTokenField.maxTokens)
                    Text("max_completion_tokens").tag(MacChatOutputTokenField.maxCompletionTokens)
                }
                Text(String(localized: "Tool calls are enabled by default, not pre-verified. Actual tool execution still follows your permission settings. Disable only if this model cannot accept tools.", table: "ModelCenter"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section(String(localized: "Request behavior", table: "ModelCenter")) {
                Picker(String(localized: "Profile", table: "ModelCenter"), selection: $preset) {
                    ForEach(MacModelParameterPreset.allCases.filter { $0 != .custom }) { value in
                        Text(value.title).tag(value)
                    }
                }
                .pickerStyle(.segmented)
                .onChange(of: preset) { _, value in value.apply(to: &parameters) }
                Text(preset.detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                parameterSummary
                advancedParametersDisclosure
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }

    private var validationMessage: String? {
        if !invalidNumericFields.isEmpty {
            return NSLocalizedString("model.parameters.invalid_values", value: "Check the displayed parameter ranges. Seed must be a whole number.", comment: "Invalid model parameters")
        }
        return MacAddRemoteModelValidation.message(
            modelID: normalizedModelID,
            capabilities: capabilities,
            parameters: parameters
        )
    }

    private var currentValidationMessage: String? {
        validationMessage
    }

    private var primaryActionTitle: String {
        String(localized: "Add Model", table: "ModelCenter")
    }

    private func performPrimaryAction() {
        guard currentValidationMessage == nil else { return }
            onAdd(MacAddRemoteModelResult(
                configuration: MacRemoteModelConfiguration(
                    modelName: normalizedModelID,
                    displayName: displayName.trimmingCharacters(in: .whitespacesAndNewlines),
                    capabilities: capabilities,
                    executionCapabilities: executionCapabilities
                ),
                parameters: parameters
            ))
            dismiss()
    }

    private var advancedParametersDisclosure: some View {
        VStack(spacing: 0) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    showsAdvancedParameters.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    Label(
                        String(localized: "Advanced parameters", table: "ModelCenter"),
                        systemImage: "slider.horizontal.3"
                    )
                    .font(.callout.weight(.medium))
                    Spacer()
                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(showsAdvancedParameters ? 180 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showsAdvancedParameters {
                VStack(spacing: 10) {
                    parameterField(
                        String(localized: "Context window", table: "ModelCenter"),
                        value: $parameters.maxContextLength,
                        range: ModelParameterLimits.remoteContextTokens
                    )
                    parameterField(
                        String(localized: "Maximum output", table: "ModelCenter"),
                        value: $parameters.maxOutputTokens,
                        range: ModelParameterLimits.outputTokens
                    )
                    parameterSlider(
                        String(localized: "Temperature", table: "ModelCenter"),
                        value: presetTrackingBinding($parameters.temperature),
                        range: 0...2
                    )
                    parameterSlider(
                        String(localized: "Top P", table: "ModelCenter"),
                        value: presetTrackingBinding($parameters.topP),
                        range: 0...1
                    )
                }
                .padding(.top, 12)
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
    }

    private func capabilityToggle(
        _ capability: MacRemoteModelCapability,
        title: String,
        symbol: String
    ) -> some View {
        Toggle(isOn: capabilityBinding(capability)) {
            Label(title, systemImage: symbol)
                .foregroundStyle(capabilities.contains(capability) ? MacDesignColor.brand : Color.primary)
        }
        .tint(MacDesignColor.brand)
    }

    private var parameterSummary: some View {
        HStack(spacing: 6) {
            parameterSummaryItem(
                String(localized: "Context"),
                value: compactTokenCount(parameters.maxContextLength)
            )
            parameterSummaryItem(
                String(localized: "Output"),
                value: compactTokenCount(parameters.maxOutputTokens)
            )
            parameterSummaryItem(
                String(localized: "Temperature", table: "ModelCenter"),
                value: parameters.temperature.formatted(.number.precision(.fractionLength(1...2)))
            )
            parameterSummaryItem(
                String(localized: "Top P", table: "ModelCenter"),
                value: parameters.topP.formatted(.number.precision(.fractionLength(1...2)))
            )
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }

    private func parameterSummaryItem(_ title: String, value: String) -> some View {
        HStack(spacing: 4) {
            Text(title)
                .foregroundStyle(.secondary)
            Text(value)
                .fontWeight(.semibold)
                .monospacedDigit()
                .contentTransition(.numericText())
        }
        .font(.caption)
        .padding(.horizontal, 8)
        .frame(height: 25)
        .background(
            Color.primary.opacity(0.045),
            in: Capsule(style: .continuous)
        )
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: value)
    }

    private func compactTokenCount(_ value: Int) -> String {
        guard value >= 1_000 else { return value.formatted(.number) }
        let divisor = value >= 1_000_000 ? 1_000_000.0 : 1_000.0
        let suffix = value >= 1_000_000 ? "M" : "K"
        let scaled = Double(value) / divisor
        if scaled.rounded() == scaled {
            return scaled.formatted(.number.precision(.fractionLength(0))) + suffix
        }
        return scaled.formatted(.number.precision(.fractionLength(1))) + suffix
    }

    private func capabilityBinding(_ capability: MacRemoteModelCapability) -> Binding<Bool> {
        Binding(
            get: { capabilities.contains(capability) },
            set: { enabled in
                if enabled { capabilities.insert(capability) }
                else { capabilities.remove(capability) }
            }
        )
    }

    private func parameterField(
        _ title: String,
        value: Binding<Int>,
        range: ClosedRange<Int>
    ) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                Text("\(range.lowerBound.formatted(.number))–\(range.upperBound.formatted(.number))")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            TextField(title, text: Binding(get: { numericText[title] ?? String(value.wrappedValue) }, set: { text in
                numericText[title] = text
                if let parsed = ModelParameterNumberParser.integer(text), range.contains(parsed) {
                    value.wrappedValue = parsed
                    invalidNumericFields.remove(title)
                } else { invalidNumericFields.insert(title) }
            }))
                .foregroundStyle(invalidNumericFields.contains(title) ? Color.red : Color.primary)
                .textFieldStyle(.roundedBorder)
                .labelsHidden()
                .multilineTextAlignment(.trailing)
                .monospacedDigit()
                .frame(width: 126)
        }
    }

    private func parameterSlider(
        _ title: String,
        value: Binding<Double>,
        range: ClosedRange<Double>
    ) -> some View {
        HStack(spacing: 10) {
            Text(title)
                .frame(width: 102, alignment: .leading)
            Slider(value: value, in: range)
            Text(value.wrappedValue.formatted(.number.precision(.fractionLength(2))))
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 36, alignment: .trailing)
        }
    }

    private func presetTrackingBinding(_ value: Binding<Double>) -> Binding<Double> {
        Binding(
            get: { value.wrappedValue },
            set: { newValue in
                value.wrappedValue = newValue
                preset = MacModelParameterPreset.matching(parameters)
            }
        )
    }
}
