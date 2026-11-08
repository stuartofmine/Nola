// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Native desktop management for the shared Apple-client MLX model cache.

import AppKit
import Combine
import SwiftUI

@MainActor
final class MacLocalModelEditingState: ObservableObject {
    @Published var searchText = ""
    @Published var inspectedModelID: String?
    @Published var showsCompactDetail = false
    @Published var libraryScrollID: String?
}

struct MacLocalModelManagerView: View {
    @StateObject private var downloader: ModelDownloader
    @ObservedObject private var preference: MacDefaultChatModelPresentation
    @StateObject private var editing: MacLocalModelEditingState
    @State private var deletionTarget: ModelDownloader.ModelType?
    @State private var errorMessage: String?
    @State private var selectedModelID: String
    @State private var startingModel: ModelDownloader.ModelType?
    @State private var runtimeRefreshID = UUID()
    @State private var dynamicDeletionTarget: MacLocalModelProfile?
    private var searchText: String { editing.searchText }
    private var inspectedModelID: String? {
        get { editing.inspectedModelID }
        nonmutating set { editing.inspectedModelID = newValue }
    }
    private var showsCompactDetail: Bool {
        get { editing.showsCompactDetail }
        nonmutating set { editing.showsCompactDetail = newValue }
    }
    @State private var showsDownloadSettings = false
    @State private var parameterRoute: MacModelParametersRoute?
    let navigationAction: (() -> Void)?
    let showsHeader: Bool

    init(navigationAction: (() -> Void)? = nil, showsHeader: Bool = true,
         downloader: ModelDownloader? = nil,
         preference: MacDefaultChatModelPresentation? = nil,
         editingState: MacLocalModelEditingState? = nil) {
        self.navigationAction = navigationAction
        self.showsHeader = showsHeader
        _downloader = StateObject(wrappedValue: downloader ?? ModelDownloader(logger: Logger.shared))
        self.preference = preference ?? MacDefaultChatModelPresentation()
        _editing = StateObject(wrappedValue: editingState ?? MacLocalModelEditingState())
        _selectedModelID = State(initialValue: MacLocalModelConfigurationStore.shared.selectedModelID)
    }

    var body: some View {
        // The runtime is not ObservableObject. Read the notification revision so
        // SwiftUI invalidates status rows without replacing their view identity.
        let _ = runtimeRefreshID
        VStack(spacing: 0) {
            if showsHeader {
                MacPageHeader(
                    LocalizedStringKey(String(localized: "Local Models", table: "ModelCenter")),
                    symbol: "cpu.fill",
                    navigationAction: navigationAction,
                    navigationSymbol: "chevron.left",
                    navigationLabel: LocalizedStringKey(String(localized: "Model Center", table: "ModelCenter"))
                ) {
                    Text(String(localized: "Choose, configure, and start local models independently.", table: "ModelCenter"))
                } actions: {
                    refreshButton
                }
            }
            GeometryReader { geometry in
                if geometry.size.width >= 760 {
                    HStack(spacing: 0) {
                        modelLibrary.frame(width: min(320, geometry.size.width * 0.36))
                        Divider()
                        modelDetail
                    }
                } else if showsCompactDetail {
                    VStack(spacing: 0) {
                        HStack {
                            Button {
                                showsCompactDetail = false
                            } label: {
                                Label(String(localized: "Local Models", table: "ModelCenter"), systemImage: "chevron.left")
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("local-model-back")
                            Spacer()
                        }
                        .padding()
                        modelDetail
                    }
                } else {
                    modelLibrary
                }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacLocalModelRuntimeDidChange)) { _ in
            runtimeRefreshID = UUID()
            preference.reload()
        }
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in preference.reload() }
        .onChange(of: downloader.downloadedModels) { _, _ in preference.reload() }
        .sheet(isPresented: Binding(
            get: { parameterRoute != nil },
            set: { if !$0 { parameterRoute = nil } }
        )) {
            if let parameterRoute {
                MacModelParametersView(route: parameterRoute)
                    .frame(minWidth: 540, idealWidth: 720, minHeight: 560, idealHeight: 720)
                    .interactiveDismissDisabled()
            }
        }
        .background(MacPageBackground())
        .confirmationDialog(String(localized: "Delete Local Model?", table: "ModelCenter"), isPresented: Binding(
            get: { deletionTarget != nil },
            set: { if !$0 { deletionTarget = nil } }
        ), titleVisibility: .visible) {
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) { deletionTarget = nil }
            Button(String(localized: "Delete", table: "ModelCenter"), role: .destructive) {
                guard let model = deletionTarget else { return }
                do { try downloader.deleteModel(model) }
                catch { errorMessage = error.localizedDescription }
                deletionTarget = nil
            }
        } message: {
            Text(String(localized: "The downloaded model files will be removed from this Mac.", table: "ModelCenter"))
        }
        .confirmationDialog(String(localized: "Delete Local Model?", table: "ModelCenter"), isPresented: Binding(
            get: { dynamicDeletionTarget != nil },
            set: { if !$0 { dynamicDeletionTarget = nil } }
        ), titleVisibility: .visible) {
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) { dynamicDeletionTarget = nil }
            Button(String(localized: "Delete", table: "ModelCenter"), role: .destructive) {
                guard let profile = dynamicDeletionTarget else { return }
                do {
                    try MacLocalModelProfile.removeInstalled(profile)
                    if selectedModelID == profile.id {
                        let fallback = MacLocalModelProfile.recommended
                        selectedModelID = fallback.id
                        try MacLocalModelConfigurationStore.shared.select(fallback)
                    }
                }
                catch { errorMessage = error.localizedDescription }
                dynamicDeletionTarget = nil
                runtimeRefreshID = UUID()
            }
        } message: {
            Text(String(localized: "The downloaded model files will be removed from this Mac.", table: "ModelCenter"))
        }
    }

    private var visibleBuiltInModels: [ModelDownloader.ModelType] {
        ModelDownloader.ModelType.allCases.filter {
            searchText.isEmpty || $0.displayName.localizedCaseInsensitiveContains(searchText)
                || $0.rawValue.localizedCaseInsensitiveContains(searchText)
        }
    }

    private var visibleInstalledModels: [MacLocalModelProfile] {
        MacLocalModelProfile.installed.filter {
            searchText.isEmpty || $0.displayName.localizedCaseInsensitiveContains(searchText)
                || $0.id.localizedCaseInsensitiveContains(searchText)
        }
    }

    private var modelLibrary: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField(String(localized: "Search models", table: "ModelCenter"), text: $editing.searchText)
                    .textFieldStyle(.plain)
                    .accessibilityIdentifier("local-model-search")
                refreshButton
            }
            .padding(14)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(visibleBuiltInModels, id: \.rawValue) { model in
                        localModelRow(id: model.rawValue, name: model.displayName,
                                      downloaded: downloader.isModelDownloaded(model))
                            .id(model.rawValue)
                    }
                    ForEach(visibleInstalledModels) { profile in
                        localModelRow(id: profile.id, name: profile.displayName, downloaded: true)
                            .id(profile.id)
                    }
                    if visibleBuiltInModels.isEmpty && visibleInstalledModels.isEmpty {
                        Text(String(localized: "No matching models", table: "ModelCenter"))
                            .foregroundStyle(.secondary)
                            .padding()
                    }
                }
                .scrollTargetLayout()
                .padding(10)
            }
            .scrollPosition(id: $editing.libraryScrollID)
            Divider()
            Button {
                showsDownloadSettings.toggle()
            } label: {
                Label(String(localized: "Download and storage settings", table: "ModelCenter"), systemImage: "gearshape")
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .padding(14)
            .popover(isPresented: $showsDownloadSettings) {
                VStack(spacing: 12) {
                    sourceCard
                    storageCard
                }
                .padding(16)
                .frame(width: 370)
            }
        }
    }

    private var detailModelID: String { inspectedModelID ?? selectedModelID }

    private func localModelRow(id: String, name: String, downloaded: Bool) -> some View {
        let profile = MacLocalModelProfile.resolve(id)
        let running = profile.map(MacMLXOpenAIService.shared.isRunning) ?? false
        let loading = MacMLXOpenAIService.shared.status.modelID == id
            && MacMLXOpenAIService.shared.status.isStarting
        let downloading = downloader.activeModel?.rawValue == id && downloader.isDownloading
        let status = localModelStatus(id: id, downloaded: downloaded, running: running, loading: loading)
        return Button {
            inspectedModelID = id
            showsCompactDetail = true
        } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: running ? "play.circle.fill" : downloaded ? "internaldrive" : "arrow.down.circle")
                    .foregroundStyle(running ? Color.green : Color.secondary)
                    .frame(width: 20)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 5) {
                    Text(name).font(.body.weight(.medium)).lineLimit(2)
                    Text(status).font(.caption).foregroundStyle(.secondary)
                    if selectedModelID == id {
                        Text(String(localized: "Runtime target", table: "ModelCenter"))
                            .font(.caption).foregroundStyle(MacDesignColor.brand)
                    }
                    if preference.isDefault(id) {
                        Label(String(localized: "Default for chat", table: "ModelCenter"), systemImage: "checkmark.circle.fill")
                            .font(.caption).foregroundStyle(MacDesignColor.brand)
                    }
                }
                Spacer(minLength: 0)
                if loading || downloading { ProgressView().controlSize(.small) }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(detailModelID == id ? MacDesignColor.brand.opacity(0.10) : Color.clear,
                        in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(detailModelID == id ? [.isSelected] : [])
        .accessibilityIdentifier("local-model-library-" + id)
    }

    private func localModelStatus(id: String, downloaded: Bool, running: Bool, loading: Bool) -> String {
        if running { return String(localized: "Running", table: "ModelCenter") }
        if loading { return String(localized: "Starting", table: "ModelCenter") }
        if downloader.activeModel?.rawValue == id {
            switch downloader.state {
            case .downloading: return String(localized: "Downloading model", table: "ModelCenter")
            case .paused: return String(localized: "Download paused", table: "ModelCenter")
            case .failed: return String(localized: "Download failed", table: "ModelCenter")
            case .notDownloaded, .downloaded: break
            }
        }
        return downloaded
            ? String(localized: "Downloaded · not running", table: "ModelCenter")
            : String(localized: "Available to download", table: "ModelCenter")
    }

    private var modelDetail: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                MacDefaultChatModelButton(preference: preference, profileID: detailModelID)
                Text(String(localized: "The runtime target controls Start. The default chat model is a separate preference.", table: "ModelCenter"))
                    .font(.caption).foregroundStyle(.secondary)
                if let running = runningModelProfile, running.id != detailModelID {
                    HStack {
                        Label(running.displayName, systemImage: "play.circle.fill")
                        Spacer()
                        Button(String(localized: "Stop running model", table: "ModelCenter"), action: stopLocalModel)
                    }
                    .padding(12).macPanel()
                }
                if let model = ModelDownloader.ModelType.allCases.first(where: { $0.rawValue == detailModelID }) {
                    modelCard(model)
                    if model.rawValue == MacLocalModelProfile.recommended.id {
                        Label(String(localized: "Recommended for this Mac", table: "ModelCenter"), systemImage: "sparkles")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                } else if let profile = MacLocalModelProfile.installed.first(where: { $0.id == detailModelID }) {
                    installedModelCard(profile)
                } else {
                    Text(String(localized: "Select a model to view its details", table: "ModelCenter"))
                        .foregroundStyle(.secondary)
                }
                if case .failed(let detail) = downloader.state {
                    downloadFailureCard(detail: detail)
                }
                if downloader.isDownloading || downloader.activeModel != nil
                    || MacMLXOpenAIService.shared.status.phase != .idle || errorMessage != nil {
                    statusCard
                }
            }
            .frame(maxWidth: 760, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .id(detailModelID)
        }
    }

    private var refreshButton: some View {
        Button {
            downloader.refreshDownloadedModelsAsync()
        } label: {
            Image(systemName: "arrow.clockwise")
        }
        .macToolbarIcon()
        .help(String(localized: "Refresh", table: "ModelCenter"))
        .accessibilityLabel(String(localized: "Refresh", table: "ModelCenter"))
        .disabled(downloader.isDownloading)
    }

    private var statusCard: some View {
        let runtime = MacMLXOpenAIService.shared.status
        return HStack(spacing: 12) {
            ZStack {
                Image(systemName: runtime.isStarting ? "cpu" : statusSymbol)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(runtime.isStarting ? MacDesignColor.brand : statusTint)
                if runtime.isStarting {
                    ProgressView().controlSize(.small)
                        .frame(width: 36, height: 36)
                }
            }
            .frame(width: 36, height: 36)
            .background((runtime.isStarting ? MacDesignColor.brand : statusTint).opacity(0.10), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 5) {
                Text(runtimeStatusTitle(runtime))
                    .font(.system(size: 12.5, weight: .semibold))
                if runtime.isStarting {
                    Text(runtimeStatusDetail(runtime))
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                        .contentTransition(.numericText())
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Text(runtimeElapsedText(runtime, now: context.date))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                } else if runtime.phase == .failed, let detail = runtime.errorDescription {
                    Text(detail)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.orange)
                        .textSelection(.enabled)
                }
                if case .downloading(let progress) = downloader.state {
                    ProgressView(value: progress)
                    HStack {
                        Text("\(Int(progress * 100))%")
                        Spacer()
                        Text("\(bytes(downloader.downloadedBytes)) / \(bytes(downloader.totalBytes))")
                        if downloader.downloadSpeed > 0 {
                            Text("· \(bytes(Int64(downloader.downloadSpeed)))/s")
                        }
                    }
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                }
                if let errorMessage {
                    Text(errorMessage)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.orange)
                        .textSelection(.enabled)
                }
            }
            Spacer()
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 6) { downloadControls }
                VStack(alignment: .trailing, spacing: 6) { downloadControls }
            }
        }
        .padding(15)
        .macPanel()
    }

    private var sourceCard: some View {
        ViewThatFits(in: .horizontal) {
            HStack {
                Label(String(localized: "Download Source", table: "ModelCenter"), systemImage: "network")
                    .font(.system(size: 12, weight: .medium))
                Spacer()
                downloadSourcePicker
                    .frame(width: 250)
            }
            VStack(alignment: .leading, spacing: 9) {
                Label(String(localized: "Download Source", table: "ModelCenter"), systemImage: "network")
                    .font(.system(size: 12, weight: .medium))
                downloadSourcePicker
                    .frame(maxWidth: .infinity)
            }
        }
        .padding(14)
        .macPanel()
    }

    private func downloadFailureCard(detail: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.orange)
                .frame(width: 36, height: 36)
                .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 5) {
                Text(String(localized: "Download failed", table: "ModelCenter"))
                    .font(.system(size: 12.5, weight: .semibold))
                if let model = downloader.activeModel {
                    Text(model.displayName)
                        .font(.system(size: 10.5, weight: .medium))
                        .foregroundStyle(.secondary)
                }
                Text(detail)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.orange)
                    .textSelection(.enabled)
            }
            Spacer(minLength: 12)
            if let model = downloader.activeModel {
                Button {
                    download(model)
                } label: {
                    Label(String(localized: "Retry", table: "ModelCenter"), systemImage: "arrow.clockwise")
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(15)
        .macPanel()
    }

    private func modelCard(_ model: ModelDownloader.ModelType) -> some View {
        let downloaded = downloader.isModelDownloaded(model)
        let profile = MacLocalModelProfile.resolve(model.rawValue)
        let running = profile.map(MacMLXOpenAIService.shared.isRunning) ?? false
        return VStack(alignment: .leading, spacing: 11) {
            HStack(alignment: .top, spacing: 13) {
                Image(systemName: downloaded ? "checkmark.circle.fill" : "arrow.down.circle")
                    .font(.system(size: 18, weight: .medium))
                    .foregroundStyle(downloaded ? Color.green : MacDesignColor.brand)
                    .frame(width: 20, height: 20)
                VStack(alignment: .leading, spacing: 5) {
                    Text(model.displayName)
                        .font(.system(size: 14, weight: .semibold))
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 6) {
                        Text("MLX")
                            .font(.system(size: 9, weight: .bold))
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3)
                            .background(Color.primary.opacity(0.055), in: Capsule())
                        if downloaded && selectedModelID == model.rawValue {
                            MacStatusBadge(
                                title: String(localized: "Runtime target", table: "ModelCenter"),
                                tint: .green,
                                symbol: "checkmark"
                            )
                        }
                        if running {
                            MacStatusBadge(
                                title: String(localized: "Running", table: "ModelCenter"),
                                tint: MacDesignColor.brand,
                                symbol: "play.fill"
                            )
                        }
                    }
                    Text(model.description)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 7) {
                            Label("64K", systemImage: "text.line.first.and.arrowtriangle.forward")
                            Text("·")
                            Text(String(localized: "Runtime", table: "ModelCenter") + " ~" + bytes(model.estimatedRuntimeBytesAt64K))
                            Text("·")
                            Text(String(localized: "Recommended memory", table: "ModelCenter") + " " + bytes(Int64(model.recommendedUnifiedMemoryBytes)))
                        }
                        VStack(alignment: .leading, spacing: 3) {
                            Label("64K", systemImage: "text.line.first.and.arrowtriangle.forward")
                            Text(String(localized: "Runtime", table: "ModelCenter") + " ~" + bytes(model.estimatedRuntimeBytesAt64K))
                            Text(String(localized: "Recommended memory", table: "ModelCenter") + " " + bytes(Int64(model.recommendedUnifiedMemoryBytes)))
                        }
                    }
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(model.isRecommendedForCurrentDevice ? Color.secondary : Color.orange)
                    Text(String(localized: "Estimated download", table: "ModelCenter") + " · \(bytes(model.estimatedSize))")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }

            Divider().opacity(0.45)

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 9) { modelActions(model, downloaded: downloaded, running: running) }
                VStack(alignment: .leading, spacing: 9) { modelActions(model, downloaded: downloaded, running: running) }
            }
            .controlSize(.regular)
        }
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .overlay(alignment: .bottom) {
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 0.5)
        }
    }

    private func installedModelCard(_ profile: MacLocalModelProfile) -> some View {
        let running = MacMLXOpenAIService.shared.isRunning(profile)
        let starting = MacMLXOpenAIService.shared.status.isStarting
            && MacMLXOpenAIService.shared.status.modelID == profile.id
        let selected = selectedModelID == profile.id
        return VStack(alignment: .leading, spacing: 11) {
            HStack(alignment: .top, spacing: 13) {
                Image(systemName: "checkmark.seal.fill")
                    .font(.system(size: 18, weight: .medium))
                    .foregroundStyle(.green)
                    .frame(width: 20, height: 20)
                VStack(alignment: .leading, spacing: 5) {
                    Text(profile.displayName)
                        .font(.system(size: 14, weight: .semibold))
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 6) {
                        Text("MLX").font(.system(size: 9, weight: .bold)).padding(.horizontal, 7).padding(.vertical, 3)
                            .background(Color.primary.opacity(0.055), in: Capsule())
                        if selected {
                            MacStatusBadge(
                                title: String(localized: "Runtime target", table: "ModelCenter"),
                                tint: .green,
                                symbol: "checkmark"
                            )
                        }
                        if running {
                            MacStatusBadge(
                                title: String(localized: "Running", table: "ModelCenter"),
                                tint: MacDesignColor.brand,
                                symbol: "play.fill"
                            )
                        }
                        if starting {
                            HStack(spacing: 4) {
                                ProgressView().controlSize(.mini)
                                Text(String(localized: "Starting", table: "ModelCenter").uppercased())
                            }
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(MacDesignColor.brand)
                        }
                    }
                    Text(profile.id).font(.system(size: 10.5, design: .monospaced)).foregroundStyle(.secondary).textSelection(.enabled)
                    Text("64K+ · \(profile.downloadText) · \(profile.parameterCount)")
                        .font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            Divider().opacity(0.45)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 9) { installedModelActions(profile, selected: selected, running: running, starting: starting) }
                VStack(alignment: .leading, spacing: 9) { installedModelActions(profile, selected: selected, running: running, starting: starting) }
            }
        }
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .overlay(alignment: .bottom) {
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 0.5)
        }
    }

    @ViewBuilder
    private func installedModelActions(
        _ profile: MacLocalModelProfile,
        selected: Bool,
        running: Bool,
        starting: Bool
    ) -> some View {
                if !selected {
                    Button {
                        do {
                            try MacLocalModelConfigurationStore.shared.select(profile)
                            selectedModelID = profile.id
                            errorMessage = nil
                            runtimeRefreshID = UUID()
                        } catch {
                            errorMessage = error.localizedDescription
                        }
                    } label: {
                        Label(String(localized: "Use for Start", table: "ModelCenter"), systemImage: "checkmark.circle")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(runningModelProfile != nil || MacMLXOpenAIService.shared.status.isStarting)
                }
                Button {
                    parameterRoute = MacModelParametersRoute(modelName: profile.id, isLocal: true, displayName: profile.displayName)
                } label: {
                    Label(String(localized: "Parameters…", table: "ModelCenter"), systemImage: "slider.horizontal.3")
                }
                if running {
                    Button(action: stopLocalModel) {
                        Label(String(localized: "Stop", table: "ModelCenter"), systemImage: "stop.fill")
                    }
                    .buttonStyle(.bordered)
                } else if selected {
                    Button { startLocalProfile(profile) } label: {
                        if starting {
                            HStack(spacing: 6) {
                                ProgressView().controlSize(.small)
                                Text(String(localized: "Starting", table: "ModelCenter") + "…")
                            }
                        } else {
                            Label(String(localized: "Start", table: "ModelCenter"), systemImage: "play.fill")
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(runningModelProfile != nil || MacMLXOpenAIService.shared.status.isStarting)
                }
                Menu {
                    Button(String(localized: "Reveal", table: "ModelCenter")) { if let directory = profile.installedDirectory { NSWorkspace.shared.activateFileViewerSelecting([directory]) } }
                    Divider()
                    Button(String(localized: "Delete", table: "ModelCenter"), role: .destructive) { dynamicDeletionTarget = profile }
                        .disabled(running || starting)
                } label: { Label(String(localized: "More"), systemImage: "ellipsis") }
                .menuStyle(.borderlessButton)
    }

    @ViewBuilder
    private func modelActions(_ model: ModelDownloader.ModelType, downloaded: Bool, running: Bool) -> some View {
        if downloaded {
            if selectedModelID != model.rawValue {
                Button(String(localized: "Use for Start", table: "ModelCenter")) {
                    do {
                        try MacLocalModelConfigurationStore.shared.select(model)
                        selectedModelID = model.rawValue
                        errorMessage = nil
                    } catch {
                        errorMessage = error.localizedDescription
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(runningModelProfile != nil || MacMLXOpenAIService.shared.status.isStarting)
            }
            Button {
                parameterRoute = MacModelParametersRoute(
                    modelName: model.mlxModelName ?? model.rawValue,
                    isLocal: true,
                    displayName: model.displayName
                )
            } label: {
                Label(String(localized: "Parameters…", table: "ModelCenter"), systemImage: "slider.horizontal.3")
            }
            if running {
                Button(action: stopLocalModel) {
                    Label(String(localized: "Stop", table: "ModelCenter"), systemImage: "stop.fill")
                }
                .buttonStyle(.bordered)
            } else if selectedModelID == model.rawValue {
                Button {
                    startLocalModel(model)
                } label: {
                    HStack(spacing: 6) {
                        if startingModel == model {
                            ProgressView().controlSize(.small)
                        }
                        Text(String(localized: "Start", table: "ModelCenter"))
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(startingModel != nil || runningModelProfile != nil || MacMLXOpenAIService.shared.status.isStarting)
            }
            Menu {
                Button(String(localized: "Reveal", table: "ModelCenter")) {
                    NSWorkspace.shared.activateFileViewerSelecting([downloader.getModelPath(for: model)])
                }
                Divider()
                Button(String(localized: "Delete", table: "ModelCenter"), role: .destructive) {
                    deletionTarget = model
                }
                .disabled(running || (MacMLXOpenAIService.shared.status.isStarting
                    && MacMLXOpenAIService.shared.status.modelID == model.rawValue))
            } label: {
                Label(String(localized: "More"), systemImage: "ellipsis")
            }
            .menuStyle(.borderlessButton)
        } else {
            Button {
                download(model)
            } label: {
                Label(String(localized: "Download", table: "ModelCenter"), systemImage: "arrow.down")
            }
                .buttonStyle(.borderedProminent)
                .disabled(downloader.isDownloading)
        }
    }

    private var runningModel: ModelDownloader.ModelType? {
        ModelDownloader.ModelType.allCases.first { model in
            guard let profile = MacLocalModelProfile.resolve(model.rawValue) else { return false }
            return MacMLXOpenAIService.shared.isRunning(profile)
        }
    }

    private var runningModelProfile: MacLocalModelProfile? {
        MacLocalModelProfile.available.first(where: MacMLXOpenAIService.shared.isRunning)
    }

    private var storageCard: some View {
        HStack {
            Label(String(localized: "Local model storage", table: "ModelCenter"), systemImage: "internaldrive")
                .font(.system(size: 11.5, weight: .medium))
            Spacer()
            Text(bytes(downloader.diskUsage))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
        }
        .padding(14)
        .macPanel()
    }

    private var statusTitle: String {
        switch downloader.state {
        case .notDownloaded: return String(localized: "No local model downloaded", table: "ModelCenter")
        case .downloading: return String(localized: "Downloading model", table: "ModelCenter")
        case .downloaded: return String(localized: "Downloaded · not running", table: "ModelCenter")
        case .failed: return String(localized: "Download failed", table: "ModelCenter")
        case .paused: return String(localized: "Download paused", table: "ModelCenter")
        }
    }

    private func runtimeStatusTitle(_ runtime: MacLocalModelRuntimeStatus) -> String {
        switch runtime.phase {
        case .startingService, .loadingWeights:
            return String(localized: "Starting", table: "ModelCenter") + " " + (runtime.displayName ?? "")
        case .ready:
            return (runtime.displayName ?? String(localized: "Local model ready", table: "ModelCenter"))
                + " · " + String(localized: "Running", table: "ModelCenter")
        case .failed:
            return String(localized: "Local model failed to start", table: "ModelCenter")
        case .idle:
            return statusTitle
        }
    }

    private func runtimeStatusDetail(_ runtime: MacLocalModelRuntimeStatus) -> String {
        runtime.phase == .loadingWeights
            ? String(localized: "Loading model weights", table: "ModelCenter")
            : String(localized: "Starting local model service", table: "ModelCenter")
    }

    private func runtimeElapsedText(_ runtime: MacLocalModelRuntimeStatus, now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince(runtime.startedAt ?? now)))
        return String(format: "%02d:%02d", seconds / 60, seconds % 60)
    }

    @ViewBuilder
    private var downloadControls: some View {
        if case .downloading = downloader.state {
            Button(action: downloader.pauseDownload) {
                Label(String(localized: "Pause", table: "ModelCenter"), systemImage: "pause.fill")
            }
            Button(role: .destructive, action: downloader.cancelDownload) {
                Label(String(localized: "Cancel", table: "ModelCenter"), systemImage: "xmark")
            }
        } else if case .paused = downloader.state {
            Button(action: downloader.resumeDownload) {
                Label(String(localized: "Resume", table: "ModelCenter"), systemImage: "play.fill")
            }
                .buttonStyle(.borderedProminent)
            Button(role: .destructive, action: downloader.cancelDownload) {
                Label(String(localized: "Cancel", table: "ModelCenter"), systemImage: "xmark")
            }
        }
    }

    private var downloadSourcePicker: some View {
        Picker(String(localized: "Download Source", table: "ModelCenter"), selection: $downloader.preferredSource) {
            Text("Hugging Face").tag(ModelDownloader.ModelSource.huggingface)
            Text("ModelScope").tag(ModelDownloader.ModelSource.modelscope)
        }
        .labelsHidden()
        .pickerStyle(.segmented)
        .disabled(downloader.isDownloading)
    }

    private var statusSymbol: String {
        switch downloader.state {
        case .notDownloaded: return "arrow.down.circle"
        case .downloading: return "arrow.down.circle.fill"
        case .downloaded: return "checkmark.circle.fill"
        case .failed: return "exclamationmark.triangle.fill"
        case .paused: return "pause.circle.fill"
        }
    }

    private var statusTint: Color {
        switch downloader.state {
        case .notDownloaded: return .secondary
        case .downloading: return MacDesignColor.brand
        case .downloaded: return .green
        case .failed, .paused: return .orange
        }
    }

    private func download(_ model: ModelDownloader.ModelType) {
        errorMessage = nil
        Task {
            do { try await downloader.downloadModel(model) }
            catch { /* Downloader owns the single, actionable failure state. */ }
        }
    }

    private func startLocalModel(_ model: ModelDownloader.ModelType) {
        guard let profile = MacLocalModelProfile.resolve(model.rawValue) else { return }
        errorMessage = nil
        startingModel = model
        Task {
            do {
                _ = try await MacMLXOpenAIService.shared.start(profile)
            } catch {
                errorMessage = error.localizedDescription
            }
            await MainActor.run {
                startingModel = nil
                runtimeRefreshID = UUID()
                NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            }
        }
    }

    private func startLocalProfile(_ profile: MacLocalModelProfile) {
        errorMessage = nil
        runtimeRefreshID = UUID()
        Task {
            do { _ = try await MacMLXOpenAIService.shared.start(profile) }
            catch { errorMessage = error.localizedDescription }
            await MainActor.run {
                runtimeRefreshID = UUID()
                NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            }
        }
    }

    private func stopLocalModel() {
        MacMLXOpenAIService.shared.stop()
        runtimeRefreshID = UUID()
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    private func bytes(_ value: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: value, countStyle: .file)
    }
}
