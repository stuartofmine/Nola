// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelDownloadView logic in the iOS UI layer.
// - Interfaces:
//   - ModelDownloadView
//   - ModelRow
// - APIs:
//   - none
//
//
//  ModelDownloadView.swift
//  Nola
//
//  UI for downloading and managing local ML models
//

import SwiftUI
import Combine

struct ModelDownloadView: View {
    var embedded = false
    var onSelectModel: ((ModelInfo) -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var container: ServiceContainer
    @State private var showingDeleteAlert = false
    @State private var modelToDelete: ModelDownloader.ModelType?
    @State private var refreshTrigger = false
    @State private var operationError: String?
    @State private var selectedModelID: String? = nil
    
    private var downloader: ModelDownloader {
        container.modelDownloader
    }
    
    var body: some View {
        List {
            Group {
            if let operationError {
                Section { Label(operationError, systemImage: "exclamationmark.triangle").foregroundStyle(DesignTokens.dangerColor) }
            }
            Section {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        NolaIconBadge(icon: "cpu", tint: DesignTokens.primaryColor, size: DesignTokens.iconBadgeSizeLarge)
                        
                        VStack(alignment: .leading) {
                            Text(L10n.modelManagementLocalTitle)
                                .font(.headline)
                            Text(L10n.modelManagementLocalOfflineNote)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    
                    statusCard

                    HStack(spacing: 10) {
                        Label(
                            NSLocalizedString("model.download.source.label", value: "Download Source", comment: "Local model download source label"),
                            systemImage: "globe"
                        )
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                        Spacer(minLength: 8)
                        Picker(
                            NSLocalizedString("model.download.source.label", value: "Download Source", comment: "Local model download source label"),
                            selection: Binding(
                                get: { downloader.preferredSource },
                                set: { downloader.preferredSource = $0 }
                            )
                        ) {
                            ForEach(ModelDownloader.ModelSource.allCases) { source in
                                Text(source.displayName).tag(source)
                            }
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .disabled(downloader.isDownloading)
                        .accessibilityIdentifier("model.download.source.picker")
                    }
                    .padding(.top, 2)
                }
            }
            
            Section(header: Text(L10n.modelDownloadSectionAvailable)) {
                ForEach(ModelDownloader.ModelType.allCases, id: \.self) { model in
                    ModelRow(
                        model: model,
                        downloader: downloader,
                        isSelected: selectedModelID == model.rawValue,
                        onSelect: {
                            selectLocalModel(model)
                        },
                        onDelete: {
                            modelToDelete = model
                            showingDeleteAlert = true
                        }
                    )
                }
            }
            
            // Comment in English.
            if downloader.isModelDownloaded(.qwen3_5_4b) {
                localModelTestSection
            }
            
            diskSpaceSection
            
            notesSection
            }
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
        .navigationTitle(embedded ? L10n.modelsTitle : L10n.modelManagementTitle)
        .toolbar {
            if !embedded {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.close) { dismiss() }
                }
            }
        }
        .onAppear {
            let current = container.chatManager.getCurrentModel() ?? ModelSelectionManager.getLastSelectedModel()
            selectedModelID = current?.isLocal == true ? current?.id : nil
        }
        .onReceive(container.chatManager.$selectedModelInfo) { info in
            selectedModelID = info?.isLocal == true ? info?.id : nil
        }
        .onReceive(container.modelDownloader.objectWillChange) { _ in
            refreshTrigger.toggle()
        }
        .alert(L10n.modelDeleteTitle, isPresented: $showingDeleteAlert) {
            Button(L10n.modelDeleteCancel, role: .cancel) {
                modelToDelete = nil
            }
            Button(L10n.modelDeleteConfirm, role: .destructive) {
                if let model = modelToDelete {
                    deleteModel(model)
                } else {
                    deleteAllModels()
                }
            }
        } message: {
            if let model = modelToDelete {
                Text(String(format: L10n.modelDeleteConfirmSingle, model.displayName))
            } else {
                Text(L10n.modelDeleteConfirmAll)
            }
        }
    }
    
    @ViewBuilder
    private var statusCard: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                switch downloader.state {
                case .notDownloaded:
                    Label(L10n.modelStatusNotDownloaded, systemImage: "arrow.down.circle")
                        .foregroundColor(.orange)
                case .downloading(let progress):
                    VStack(alignment: .leading, spacing: 8) {
                        Label(
                            downloader.activeModel.map { "\(L10n.modelStatusDownloading) · \($0.displayName)" } ?? L10n.modelStatusDownloading,
                            systemImage: "arrow.down.circle.fill"
                        )
                        .foregroundColor(DesignTokens.primaryColor)
                        
                        ProgressView(value: progress)
                            .tint(DesignTokens.primaryColor)
                        
                        HStack {
                            Text(String(format: L10n.modelDownloadProgressFormat, Int(progress * 100)))
                                .font(.caption)
                                .fontWeight(.medium)
                                .monospacedDigit()
                            
                            Spacer()
                            
                            if downloader.totalBytes > 0 {
                                Text(String(format: L10n.modelDownloadByteProgressFormat, DisplayFormatters.gigabyteSize(downloader.downloadedBytes), DisplayFormatters.gigabyteSize(downloader.totalBytes)))
                                    .font(.caption2)
                                    .foregroundColor(.secondary)
                                    .monospacedDigit()
                            }
                        }
                        
                        if downloader.downloadSpeed > 0 {
                            Text(String(format: L10n.modelDownloadSpeedFormat, DisplayFormatters.transferRate(downloader.downloadSpeed)))
                                .font(.caption2)
                                .foregroundColor(.secondary)
                                .monospacedDigit()
                        }
                    }
                case .downloaded:
                    Label(String(format: L10n.modelDownloadedCount.localized(), downloader.downloadedModels.count), systemImage: "checkmark.circle.fill")
                        .foregroundColor(.green)
                case .failed(let error):
                    VStack(alignment: .leading) {
                        Label(L10n.modelStatusDownloadFailed, systemImage: "exclamationmark.triangle")
                            .foregroundColor(.red)
                        Text(error)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                case .paused:
                    Label(L10n.modelStatusPaused, systemImage: "pause.circle")
                        .foregroundColor(.gray)
                }
            }
            
            Spacer()
            
            // Control buttons
            if case .downloading = downloader.state {
                HStack {
                    Button {
                        downloader.pauseDownload()
                    } label: {
                        Image(systemName: "pause.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.pause", value: "Pause download", comment: "Local model action"))
                            .font(.title2)
                    }
                    .buttonStyle(.borderless)
                    
                    Button {
                        downloader.cancelDownload()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.cancel", value: "Cancel download", comment: "Local model action"))
                            .font(.title2)
                            .foregroundColor(.red)
                    }
                    .buttonStyle(.borderless)
                }
            } else if case .paused = downloader.state {
                Button {
                    downloader.resumeDownload()
                } label: {
                    Image(systemName: "play.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.resume", value: "Resume download", comment: "Local model action"))
                        .font(.title2)
                        .foregroundColor(DesignTokens.primaryColor)
                }
                .buttonStyle(.borderless)
            }
        }
        .padding()
        .background(DesignTokens.secondaryBackgroundColor)
        .cornerRadius(10)
    }
    
    private func selectLocalModel(_ model: ModelDownloader.ModelType) {
        operationError = nil
        let info = model.canonicalModelInfo
        ModelSelectionManager.addModel(info)
        if let onSelectModel {
            onSelectModel(info)
            selectedModelID = info.id
            return
        }
        do {
            try ModelSelectionManager.commitSelectedModel(info, using: container.applicationRuntime)
            container.chatManager.selectModel(info, persistAsDefault: false)
            selectedModelID = info.id
        } catch {
            operationError = error.localizedDescription
        }
    }

    private func deleteModel(_ model: ModelDownloader.ModelType) {
        operationError = nil
        modelToDelete = nil
        Task { @MainActor in
            do {
                try await downloader.deleteModelAsync(model)
                reconcileSelectionAfterDeleting([model.rawValue])
            } catch {
                operationError = NSLocalizedString("model.delete.failed", value: "The model could not be deleted. Try again.", comment: "Local model deletion failed") + " " + error.localizedDescription
                // Keep the failed model visible and report the operation inline.
                Logger.shared.log(.warn, "ModelDownloadView", NSLocalizedString("autogen.ModelDownloadView.238c0cc1", comment: "AUTOGEN: localized text"), metadata: [
                    "error": String(describing: error)
                ])
            }
        }
    }
    
    private func deleteAllModels() {
        operationError = nil
        let targets = Array(downloader.downloadedModels)
        Task { @MainActor in
            var failed: [String] = []
            var deletedIDs: Set<String> = []
            for model in targets {
                do {
                    try await downloader.deleteModelAsync(model)
                    deletedIDs.insert(model.rawValue)
                } catch {
                    failed.append(model.displayName)
                }
            }
            if !deletedIDs.isEmpty {
                reconcileSelectionAfterDeleting(deletedIDs)
            }
            if !failed.isEmpty {
                operationError = NSLocalizedString("model.delete.failed", value: "The model could not be deleted. Try again.", comment: "Local model deletion failed") + " " + failed.joined(separator: ", ")
            }
        }
    }

    @MainActor
    private func reconcileSelectionAfterDeleting(_ deletedIDs: Set<String>) {
        if let currentID = selectedModelID, deletedIDs.contains(currentID) {
            selectedModelID = nil
        }
        if let selected = container.chatManager.selectedModelInfo,
           selected.isLocal,
           deletedIDs.contains(selected.id) {
            let remaining = ModelSelectionManager.getAvailableModels()
            if let fallback = remaining.first {
                container.chatManager.selectModel(fallback)
                selectedModelID = fallback.isLocal ? fallback.id : nil
            } else {
                container.chatManager.selectedModelInfo = nil
                selectedModelID = nil
            }
        }
    }
    
    @ViewBuilder
    private var localModelTestSection: some View {
        Section(header: Text(L10n.modelLocalTestTitle)) {
            NavigationLink {
                MLXTestView()
                    .environmentObject(container)
            } label: {
                HStack {
                    Image(systemName: "play.circle.fill")
                        .foregroundColor(.green)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(L10n.modelLocalTestTitle)
                            .fontWeight(.medium)
                        Text(L10n.modelLocalTestDesc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                    Spacer()
                }
            }
            
            NavigationLink {
                IOSModelInferenceTestView(model: ModelInfo(
                    id: ModelDownloader.ModelType.qwen3_5_4b.rawValue,
                    name: ModelDownloader.ModelType.qwen3_5_4b.displayName,
                    source: "local",
                    isLocal: true
                ))
                    .environmentObject(container)
            } label: {
                HStack {
                    Image(systemName: "gearshape.2.fill")
                        .foregroundColor(DesignTokens.primaryColor)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(L10n.modelAdvancedTestTitle)
                            .fontWeight(.medium)
                        Text(L10n.modelAdvancedTestDesc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                    Spacer()
                }
            }
        }
    }
    
    @ViewBuilder
    private var diskSpaceSection: some View {
        Section(header: Text(L10n.modelDiskSpaceTitle)) {
            HStack {
                Text(L10n.modelDiskDownloadedSize)
                Spacer()
                if downloader.isDownloading {
                    Text(DisplayFormatters.gigabyteSize(downloader.downloadedBytes))
                        .foregroundColor(DesignTokens.primaryColor)
                        .monospacedDigit()
                } else {
                    Text(DisplayFormatters.gigabyteSize(downloader.diskUsage))
                        .foregroundColor(.secondary)
                        .monospacedDigit()
                }
            }
            
            if !downloader.downloadedModels.isEmpty {
                Button(role: .destructive) {
                    showingDeleteAlert = true
                } label: {
                    Label(L10n.modelDeleteAllModels, systemImage: "trash")
                }
            }
        }
    }
    
    @ViewBuilder
    private var notesSection: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text(L10n.modelNotesTitle)
                    .font(.caption)
                    .fontWeight(.semibold)
                
                Label(L10n.modelNotesDownloadWifi, systemImage: "wifi")
                    .font(.caption)
                    .foregroundColor(.secondary)
                
                Label(L10n.modelNotesSaved, systemImage: "externaldrive")
                    .font(.caption)
                    .foregroundColor(.secondary)
                
                Label(L10n.modelNotesDeleteSpace, systemImage: "trash")
                    .font(.caption)
                    .foregroundColor(.secondary)
                
                Label(L10n.modelNotesSourceSuggestion, systemImage: "network")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
    }
}

// MARK: - Model Row

struct ModelRow: View {
    let model: ModelDownloader.ModelType
    @ObservedObject var downloader: ModelDownloader
    let isSelected: Bool
    let onSelect: () -> Void
    let onDelete: () -> Void
    
    private var hasIncompleteDirectory: Bool {
        downloader.hasIncompleteDownload(model)
    }
    
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text(model.displayName)
                        .font(.headline)
                    Text(model.description)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)

                    Text(String(format: L10n.modelDownloadSizeFormat, DisplayFormatters.gigabyteSize(model.totalSize)))
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .monospacedDigit()
                }
                
                Spacer()

                if downloader.isTransferring(model) {
                    HStack(spacing: 8) {
                        Button {
                            downloader.pauseDownload()
                        } label: {
                            Image(systemName: "pause.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.pause", value: "Pause download", comment: "Local model action"))
                                .font(.title3)
                        }
                        .buttonStyle(.borderless)

                        Button {
                            downloader.cancelDownload()
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.cancel", value: "Cancel download", comment: "Local model action"))
                                .foregroundColor(.red)
                        }
                        .buttonStyle(.borderless)
                    }
                } else if downloader.isPaused(model) {
                    Button {
                        downloader.resumeDownload()
                    } label: {
                        Image(systemName: "play.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.resume", value: "Resume download", comment: "Local model action"))
                            .foregroundColor(DesignTokens.primaryColor)
                    }
                    .buttonStyle(.borderless)
                } else if downloader.isModelDownloaded(model) {
                    HStack(spacing: 8) {
                        if isSelected {
                            Label(
                                NSLocalizedString("model.center.selected", value: "Selected", comment: "Selected model badge"),
                                systemImage: "checkmark.circle.fill"
                            )
                            .font(.caption.weight(.medium))
                            .foregroundStyle(DesignTokens.successColor)
                        } else {
                            Button {
                                onSelect()
                            } label: {
                                Text(L10n.modelActionUse)
                            }
                            .buttonStyle(.borderedProminent)
                            .tint(DesignTokens.actionColor)
                            .foregroundStyle(DesignTokens.onActionColor)
                            .controlSize(.small)
                            .accessibilityIdentifier("model.download.use.\(model.rawValue)")
                        }

                        Button(role: .destructive) {
                            onDelete()
                        } label: {
                            Image(systemName: "trash")
                                .frame(minWidth: 44, minHeight: 44)
                                .accessibilityLabel(NSLocalizedString("model.delete.confirm", value: "Delete model", comment: "Local model action"))
                                .foregroundColor(.red)
                        }
                        .buttonStyle(.borderless)
                    }
                } else if hasIncompleteDirectory {
                    // Comment in English.
                    HStack(spacing: 8) {
                        Button(role: .destructive) {
                            onDelete()
                        } label: {
                            Image(systemName: "trash")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.delete.confirm", value: "Delete model", comment: "Local model action"))
                                .foregroundColor(.red)
                        }
                        .buttonStyle(.borderless)
                        
                        Button {
                            downloadModel()
                        } label: {
                            Label(L10n.modelActionDownload, systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(.bordered)
                    }
                    } else {
                    // Comment in English.
                    Button {
                        downloadModel()
                    } label: {
                        Label(L10n.modelActionDownload, systemImage: "arrow.down.circle")
                    }
                    .buttonStyle(.bordered)
                    .disabled(downloader.activeModel != nil)
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Label(
                    String(
                        format: NSLocalizedString("model.download.context_format", value: "%@ context", comment: "Local model context window"),
                        model.contextWindowTokens.formatted(.number.notation(.compactName))
                    ),
                    systemImage: "text.line.first.and.arrowtriangle.forward"
                )
                Label(
                    String(format: NSLocalizedString("model.download.runtime_memory_format", value: "~%@ at 64K", comment: "Estimated local model runtime memory at 64K context"), DisplayFormatters.gigabyteSize(model.estimatedRuntimeBytesAt64K)),
                    systemImage: "memorychip"
                )
                .monospacedDigit()
                if model.isRecommendedForCurrentDevice {
                    Label(
                        NSLocalizedString("model.download.recommended", value: "Recommended", comment: "Local model recommendation badge"),
                        systemImage: "checkmark.seal.fill"
                    )
                    .foregroundStyle(DesignTokens.successColor)
                }
            }
            .labelStyle(.titleAndIcon)
            .font(.caption2.weight(.medium))
            .foregroundStyle(.secondary)
            
            if downloader.isTransferring(model), case .downloading(let progress) = downloader.state {
                Label(
                    String(format: L10n.modelDownloadProgressFormat, Int(progress * 100)),
                    systemImage: "arrow.down.circle.fill"
                )
                .font(.caption.weight(.medium))
                .foregroundStyle(DesignTokens.primaryColor)
                .monospacedDigit()
            } else if downloader.isModelDownloaded(model) {
                Label(L10n.modelDownloadAvailableOffline, systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundColor(.green)
            } else if hasIncompleteDirectory {
                Label(L10n.modelDownloadIncomplete, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundColor(.orange)
            }
        }
        .padding(.vertical, 4)
        .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }
    }
    
    private func downloadModel() {
        Task {
            do {
                try await downloader.downloadModel(model)
            } catch {
                // Error is handled in ModelDownloader
                Logger.shared.log(.warn, "ModelDownloadView", NSLocalizedString("autogen.ModelDownloadView.e0dab22b", comment: "AUTOGEN: localized text"), metadata: [
                    "error": String(describing: error)
                ])
            }
        }
    }
    
}

#Preview {
    NavigationStack {
        ModelDownloadView()
    }
}
