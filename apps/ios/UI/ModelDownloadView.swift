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
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var container: ServiceContainer
    @State private var showingDeleteAlert = false
    @State private var modelToDelete: ModelDownloader.ModelType?
    @State private var refreshTrigger = false
    @State private var operationError: String?
    
    private var downloader: ModelDownloader {
        container.modelDownloader
    }
    
    var body: some View {
        List {
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
                }
            }
            
            Section(header: Text(L10n.modelDownloadSectionAvailable)) {
                ForEach(ModelDownloader.ModelType.allCases, id: \.self) { model in
                    ModelRow(
                        model: model,
                        downloader: downloader,
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
        .navigationTitle(embedded ? L10n.modelsTitle : L10n.modelManagementTitle)
        .toolbar {
            if !embedded {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.close) { dismiss() }
                }
            }
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
                        Label(L10n.modelStatusDownloading, systemImage: "arrow.down.circle.fill")
                            .foregroundColor(DesignTokens.primaryColor)
                        
                        ProgressView(value: progress)
                            .tint(DesignTokens.primaryColor)
                        
                            HStack {
                            Text(String(format: L10n.modelDownloadProgressFormat, Int(progress * 100)))
                                .font(.caption)
                                .fontWeight(.medium)
                            
                            Spacer()
                            
                            // Comment in English.
                            if downloader.totalBytes > 0 {
                                Text(String(format: L10n.modelDownloadByteProgressFormat, DisplayFormatters.byteSize(downloader.downloadedBytes), DisplayFormatters.byteSize(downloader.totalBytes)))
                                    .font(.caption2)
                                    .foregroundColor(.secondary)
                            }
                        }
                        
                        // Comment in English.
                        if downloader.downloadSpeed > 0 {
                            Text(String(format: L10n.modelDownloadSpeedFormat, DisplayFormatters.transferRate(downloader.downloadSpeed)))
                                .font(.caption2)
                                .foregroundColor(.secondary)
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
                    
                    Button {
                        downloader.cancelDownload()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.cancel", value: "Cancel download", comment: "Local model action"))
                            .font(.title2)
                            .foregroundColor(.red)
                    }
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
            }
        }
        .padding()
        .background(Color(.systemGray6))
        .cornerRadius(10)
    }
    
    private func deleteModel(_ model: ModelDownloader.ModelType) {
        operationError = nil
        do {
            try downloader.deleteModel(model)
        } catch {
            operationError = NSLocalizedString("model.delete.failed", value: "The model could not be deleted. Try again.", comment: "Local model deletion failed") + " " + error.localizedDescription
            // Keep the failed model visible and report the operation inline.
            Logger.shared.log(.warn, "ModelDownloadView", NSLocalizedString("autogen.ModelDownloadView.238c0cc1", comment: "AUTOGEN: localized text"), metadata: [
                "error": String(describing: error)
            ])
        }
        modelToDelete = nil
    }
    
    private func deleteAllModels() {
        operationError = nil
        var failed: [String] = []
        for model in downloader.downloadedModels {
            do { try downloader.deleteModel(model) }
            catch { failed.append(model.displayName) }
        }
        if !failed.isEmpty {
            operationError = NSLocalizedString("model.delete.failed", value: "The model could not be deleted. Try again.", comment: "Local model deletion failed") + " " + failed.joined(separator: ", ")
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
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.action.resume", value: "Resume download", comment: "Local model action"))
                        .foregroundColor(.green)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(L10n.modelLocalTestTitle)
                            .fontWeight(.medium)
                        Text(L10n.modelLocalTestDesc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundColor(.secondary)
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
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundColor(.secondary)
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
                    Text(DisplayFormatters.byteSize(downloader.downloadedBytes))
                        .foregroundColor(DesignTokens.primaryColor)
                } else {
                    Text(DisplayFormatters.byteSize(downloader.diskUsage))
                        .foregroundColor(.secondary)
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
    let onDelete: () -> Void
    
    // Comment in English.
    private var modelDirectoryExists: Bool {
        let modelPath = downloader.getModelPath(for: model)
        return FileManager.default.fileExists(atPath: modelPath.path)
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

                    Text(String(format: L10n.modelDownloadSizeFormat, DisplayFormatters.byteSize(model.totalSize)))
                        .font(.caption)
                        .foregroundColor(.secondary)
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
                    // Comment in English.
                    Button(role: .destructive) {
                        onDelete()
                    } label: {
                        Image(systemName: "trash")
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityLabel(NSLocalizedString("model.delete.confirm", value: "Delete model", comment: "Local model action"))
                            .foregroundColor(.red)
                    }
                    .buttonStyle(.borderless)
                } else if modelDirectoryExists {
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
                    String(format: NSLocalizedString("model.download.runtime_memory_format", value: "~%@ at 64K", comment: "Estimated local model runtime memory at 64K context"), DisplayFormatters.byteSize(model.estimatedRuntimeBytesAt64K)),
                    systemImage: "memorychip"
                )
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
            
            // Comment in English.
            if downloader.isTransferring(model), case .downloading(let progress) = downloader.state {
                VStack(alignment: .leading, spacing: 6) {
                    ProgressView(value: progress)
                        .tint(DesignTokens.primaryColor)
                    
                    HStack {
                        Text(String(format: L10n.modelDownloadProgressFormat, Int(progress * 100)))
                            .font(.caption)
                            .fontWeight(.medium)
                        
                        Spacer()
                        
                        if downloader.totalBytes > 0 {
                                Text(String(format: L10n.modelDownloadByteProgressFormat, DisplayFormatters.byteSize(downloader.downloadedBytes), DisplayFormatters.byteSize(downloader.totalBytes)))
                                .font(.caption2)
                                .foregroundColor(.secondary)
                        }
                    }
                    
                    if downloader.downloadSpeed > 0 {
                        Text(String(format: L10n.modelDownloadSpeedFormat, DisplayFormatters.transferRate(downloader.downloadSpeed)))
                            .font(.caption2)
                            .foregroundColor(.secondary)
                    }
                }
                .padding(.top, 4)
            } else if downloader.isModelDownloaded(model) {
                Label(L10n.modelDownloadAvailableOffline, systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundColor(.green)
            } else if modelDirectoryExists {
                // Comment in English.
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
