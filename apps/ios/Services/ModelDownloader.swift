// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelDownloader logic in the iOS service layer.
// - Interfaces:
//   - ModelDownloader
// - APIs:
//   - none
//
//
//  ModelDownloader.swift
//  Nola
//
//  Manages downloading and caching of local ML models
//

import Foundation
import Combine
import CryptoKit

public final class ModelDownloader: ObservableObject {
    
    public enum DownloadState: Equatable {
        case notDownloaded
        case downloading(progress: Double)
        case downloaded
        case failed(error: String)
        case paused
    }
    
    public enum ModelSource {
        case huggingface
        case modelscope
        
        static var preferred: ModelSource {
            return .huggingface
        }
        
        var displayName: String {
            switch self {
            case .huggingface: return "Hugging Face"
            case .modelscope: return NSLocalizedString("autogen.ModelDownloader.71608408", comment: "AUTOGEN")
            }
        }
    }
    
    public enum ModelType: String, CaseIterable {
        case qwen3_5_0_8b = "mlx-community/Qwen3.5-0.8B-MLX-4bit"
        case qwen3_5_4b = "mlx-community/Qwen3.5-4B-MLX-4bit"
        
        var displayName: String {
            switch self {
            case .qwen3_5_0_8b: return "Qwen3.5 0.8B (4-bit)"
            case .qwen3_5_4b: return "Qwen3.5 4B (4-bit)"
            }
        }
        
        var description: String {
            switch self {
            case .qwen3_5_0_8b:
                return NSLocalizedString("Fast, compact, and suitable for lightweight conversations.", comment: "Local model description")
            case .qwen3_5_4b:
                return NSLocalizedString("Recommended for higher-quality writing, reasoning, and tool use.", comment: "Local model description")
            }
        }
        
        // Estimated size used for disk checks. Keep consistent with `totalSize` (sum of requiredFiles).
        var estimatedSize: Int64 {
            return totalSize
        }

        var contextWindowTokens: Int { 65_536 }
        var nativeContextWindowTokens: Int { 262_144 }

        var estimatedRuntimeBytesAt64K: Int64 {
            switch self {
            case .qwen3_5_0_8b: return 3_000_000_000
            case .qwen3_5_4b: return 7_000_000_000
            }
        }

        var recommendedUnifiedMemoryBytes: UInt64 {
            switch self {
            case .qwen3_5_0_8b: return 8_000_000_000
            case .qwen3_5_4b: return 16_000_000_000
            }
        }

        var isRecommendedForCurrentDevice: Bool {
            ProcessInfo.processInfo.physicalMemory >= recommendedUnifiedMemoryBytes
        }
        
        var huggingfaceURL: String {
            switch self {
            case .qwen3_5_0_8b: return "https://huggingface.co/mlx-community/Qwen3.5-0.8B-MLX-4bit"
            case .qwen3_5_4b: return "https://huggingface.co/mlx-community/Qwen3.5-4B-MLX-4bit"
            }
        }
        
        var modelscopeURL: String {
            switch self {
            case .qwen3_5_0_8b: return "https://modelscope.cn/models/lmstudio-community/Qwen3.5-0.8B-MLX-4bit"
            case .qwen3_5_4b: return "https://modelscope.cn/models/lmstudio-community/Qwen3.5-4B-MLX-4bit"
            }
        }
        
        var huggingfaceRepo: String {
            switch self {
            case .qwen3_5_0_8b: return "mlx-community/Qwen3.5-0.8B-MLX-4bit"
            case .qwen3_5_4b: return "mlx-community/Qwen3.5-4B-MLX-4bit"
            }
        }

        // Map downloader model types to MLX/LMModel names used by MLXService.availableModels
        // This enables precise binding between a downloaded model and the local LMModel identity.
        var mlxModelName: String? {
            switch self {
            case .qwen3_5_0_8b: return rawValue
            case .qwen3_5_4b: return rawValue
            }
        }
        
        var modelscopeRepo: String {
            switch self {
            case .qwen3_5_0_8b: return "lmstudio-community/Qwen3.5-0.8B-MLX-4bit"
            case .qwen3_5_4b: return "lmstudio-community/Qwen3.5-4B-MLX-4bit"
            }
        }
        
        var requiredFiles: [(name: String, size: Int64, sha256: String)] {
            switch self {
            case .qwen3_5_0_8b:
                return Self.qwen35Files(weightBytes: 625_229_487)
            case .qwen3_5_4b:
                return Self.qwen35Files(weightBytes: 3_030_000_000)
            }
        }

        private static func qwen35Files(weightBytes: Int64) -> [(name: String, size: Int64, sha256: String)] {
            [
                ("config.json", 4_000, ""),
                ("model.safetensors", weightBytes, ""),
                ("model.safetensors.index.json", 72_000, ""),
                ("chat_template.jinja", 8_000, ""),
                ("tokenizer.json", 20_000_000, ""),
                ("tokenizer_config.json", 12_000, ""),
                ("vocab.json", 6_720_000, ""),
                ("preprocessor_config.json", 2_000, ""),
                ("processor_config.json", 2_000, ""),
                ("video_preprocessor_config.json", 2_000, "")
            ]
        }
        
        var totalSize: Int64 {
            return requiredFiles.reduce(0) { $0 + $1.size }
        }

        func validatesDownloadedFiles(at directory: URL) -> Bool {
            let fileManager = FileManager.default
            for file in requiredFiles {
                let url = directory.appendingPathComponent(file.name)
                guard let attributes = try? fileManager.attributesOfItem(atPath: url.path),
                      let size = attributes[.size] as? NSNumber,
                      size.int64Value > 0 else {
                    return false
                }
                if file.name == "model.safetensors", size.int64Value != file.size {
                    return false
                }
                if file.name.hasSuffix(".json") {
                    guard let data = try? Data(contentsOf: url),
                          (try? JSONSerialization.jsonObject(with: data)) != nil else {
                        return false
                    }
                }
            }

            let indexURL = directory.appendingPathComponent("model.safetensors.index.json")
            guard let indexData = try? Data(contentsOf: indexURL),
                  let index = try? JSONSerialization.jsonObject(with: indexData) as? [String: Any],
                  let weightMap = index["weight_map"] as? [String: String],
                  !weightMap.isEmpty else {
                return false
            }
            return Set(weightMap.values).allSatisfy {
                fileManager.fileExists(atPath: directory.appendingPathComponent($0).path)
            }
        }
        
        func pageURL(from source: ModelSource) -> String {
            switch source {
            case .huggingface:
                return huggingfaceURL
            case .modelscope:
                return modelscopeURL
            }
        }
        
        func fileURL(fileName: String, source: ModelSource) -> URL? {
            switch source {
            case .huggingface:
                return URL(string: "https://huggingface.co/\(huggingfaceRepo)/resolve/main/\(fileName)")
            case .modelscope:
                return URL(string: "https://modelscope.cn/models/\(modelscopeRepo)/resolve/master/\(fileName)")
            }
        }
    }
    
    @Published public private(set) var state: DownloadState = .notDownloaded
    @Published public private(set) var downloadedModels: Set<ModelType> = []
    @Published public var preferredSource: ModelSource = .preferred
    
    @Published public private(set) var downloadSpeed: Double = 0
    @Published public private(set) var downloadedBytes: Int64 = 0
    @Published public private(set) var totalBytes: Int64 = 0
    @Published public private(set) var diskUsage: Int64 = 0

    // Convenience flag for UI
    public var isDownloading: Bool {
        if case .downloading = state { return true }
        return false
    }

    public var activeModel: ModelType? {
        currentModel
    }

    public func isTransferring(_ model: ModelType) -> Bool {
        activeModel == model && isDownloading
    }

    public func isPaused(_ model: ModelType) -> Bool {
        guard activeModel == model else { return false }
        if case .paused = state { return true }
        return false
    }
    
    private let logger: Logger
    private var activeDownloadTask: URLSessionDownloadTask?
    private var progressObservation: NSKeyValueObservation?
    // resume data is stored per-model + per-file on disk to avoid conflicts
    private var currentDownloadingFile: String? // filename currently downloading
    private var currentModel: ModelType?
    private var isPaused: Bool = false
    private enum ControlError: Error {
        case paused
    }
    private var lastProgressUpdate = Date()
    private var lastDownloadedBytes: Int64 = 0
    private var isCancelled = false
    private var diskUsageUpdateTask: Task<Void, Never>?
    // No Rust: Swift-only download implementation
    
    public init(logger: Logger) {
        self.logger = logger
        refreshDownloadedModelsAsync()
        scheduleDiskUsageUpdate(reason: "init")
    }
    
    // MARK: - Public Methods
    
    /// Check which models are already downloaded
    public func checkDownloadedModels() {
        if case .downloading = state {
            return
        }
        let downloaded = computeDownloadedModels()
        applyDownloadedModels(downloaded)
    }

    /// Compute downloaded models without mutating published state (safe for background use)
    public func computeDownloadedModels() -> Set<ModelType> {
        if case .downloading = state {
            return downloadedModels
        }
        var downloaded: Set<ModelType> = []
        for modelType in ModelType.allCases {
            if isModelDownloaded(modelType) {
                downloaded.insert(modelType)
            }
        }
        return downloaded
    }

    /// Apply downloaded model snapshot to published state
    public func applyDownloadedModels(_ downloaded: Set<ModelType>) {
        downloadedModels = downloaded
        state = downloaded.isEmpty ? .notDownloaded : .downloaded
    }

    /// Refresh downloaded models asynchronously to avoid blocking the main thread
    public func refreshDownloadedModelsAsync() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            let downloaded = self.computeDownloadedModels()
            self.applyDownloadedModels(downloaded)
        }
    }
    
    /// Check if a specific model is downloaded (all required files exist and verified)
    public func isModelDownloaded(_ model: ModelType) -> Bool {
        let modelPath = getModelPath(for: model)
        guard model.validatesDownloadedFiles(at: modelPath) else {
            return false
        }
        
        for fileInfo in model.requiredFiles {
            let filePath = modelPath.appendingPathComponent(fileInfo.name)
            
            if !FileManager.default.fileExists(atPath: filePath.path) {
                return false
            }
            
            if !fileInfo.sha256.isEmpty {
                do {
                    let isValid = try verifyFileIntegrity(at: filePath, expectedHash: fileInfo.sha256)
                    if !isValid {
                        logger.log(.warn, "ModelDownloader", "File hash mismatch", metadata: [
                            "model": model.rawValue,
                            "file": fileInfo.name
                        ])
                        return false
                    }
                } catch {
                    logger.log(.error, "ModelDownloader", "Failed to verify file", metadata: [
                        "model": model.rawValue,
                        "file": fileInfo.name,
                        "error": error.localizedDescription
                    ])
                    return false
                }
            }
        }
        
        return true
    }
    
    /// Get the local path for a model
    public func getModelPath(for model: ModelType) -> URL {
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        return cacheDir.appendingPathComponent("models/\(model.rawValue)")
    }
    
    /// Download a model
    public func downloadModel(_ model: ModelType) async throws {
        guard !isDownloading else {
            logger.log(.warn, "ModelDownloader", "Download already in progress")
            return
        }

        // mark current model for resume semantics
        currentModel = model
        
        isCancelled = false
        
        // Check if already downloaded
        if isModelDownloaded(model) {
            logger.log(.info, "ModelDownloader", "Model already downloaded")
            await MainActor.run {
                state = .downloaded
            }
            return
        }
        
        // Check available disk space
        guard hasEnoughDiskSpace(for: model) else {
            let error = "磁盘空间不足，需要至少 \(formatBytes(model.estimatedSize * 2))"
            logger.log(.error, "ModelDownloader", error)
            await MainActor.run {
                state = .failed(error: error)
            }
            return
        }
        
        await MainActor.run {
            state = .downloading(progress: 0)
            totalBytes = model.totalSize
            downloadedBytes = 0
            downloadSpeed = 0
            lastDownloadedBytes = 0
            lastProgressUpdate = Date()
        }
        
        logger.log(.info, "ModelDownloader", "Starting download", metadata: [
            "model": model.rawValue,
            "source": preferredSource.displayName,
            "files": "\(model.requiredFiles.count)",
            "totalSize": formatBytes(model.totalSize)
        ])
        
        do {
            let modelPath = getModelPath(for: model)
            try FileManager.default.createDirectory(at: modelPath, withIntermediateDirectories: true)
            
            var accumulatedBytes: Int64 = 0
            for (index, fileInfo) in model.requiredFiles.enumerated() {
                guard let fileURL = model.fileURL(fileName: fileInfo.name, source: preferredSource) else {
                    throw NSError(domain: "ModelDownloader", code: -2, userInfo: [
                        NSLocalizedDescriptionKey: "Invalid download URL for \(fileInfo.name)"
                    ])
                }
                let destinationURL = modelPath.appendingPathComponent(fileInfo.name)
                
                logger.log(.info, "ModelDownloader", "Downloading file", metadata: [
                    "file": fileInfo.name,
                    "size": formatBytes(fileInfo.size),
                    "progress": "\(index + 1)/\(model.requiredFiles.count)"
                ])
                
                let maxRetries = 3
                var attemptCount = 0
                var downloadSucceeded = false
                
                while attemptCount < maxRetries && !downloadSucceeded {
                    attemptCount += 1
                    
                    if isCancelled {
                        throw NSError(domain: NSURLErrorDomain, code: NSURLErrorCancelled, userInfo: [
                            NSLocalizedDescriptionKey: L10n.string("model_downloader.error.download_cancelled")
                        ])
                    }
                    
                        do {
                            if attemptCount > 1 {
                                try? FileManager.default.removeItem(at: destinationURL)
                                logger.log(.info, "ModelDownloader", "Retrying download", metadata: [
                                    "file": fileInfo.name,
                                    "attempt": "\(attemptCount)/\(maxRetries)"
                                ])
                            }
                            try await downloadSingleFile(
                                from: fileURL,
                                fileName: fileInfo.name,
                                to: destinationURL,
                                fileSize: fileInfo.size,
                                previousBytes: accumulatedBytes,
                                totalBytes: model.totalSize
                            )
                        logger.log(.info, "ModelDownloader", "Verifying file integrity", metadata: ["file": fileInfo.name])
                        let isValid = try verifyFileIntegrity(at: destinationURL, expectedHash: fileInfo.sha256)
                        
                        if isValid {
                            logger.log(.info, "ModelDownloader", "File verified", metadata: ["file": fileInfo.name])
                            downloadSucceeded = true
                        } else {
                            logger.log(.warn, "ModelDownloader", "Hash verification failed", metadata: [
                                "file": fileInfo.name,
                                "attempt": "\(attemptCount)/\(maxRetries)"
                            ])
                            
                            if attemptCount >= maxRetries {
                                throw NSError(domain: "ModelDownloader", code: -1, userInfo: [
                                    NSLocalizedDescriptionKey: L10n.format("model_downloader.error.file_download_failed_after_retries_format", fileInfo.name, maxRetries)
                                ])
                            }
                        }
                        
                        } catch {
                        if isCancelled || (error as NSError).code == NSURLErrorCancelled {
                            throw error
                        }
                        
                        logger.log(.error, "ModelDownloader", "Download failed", metadata: [
                            "file": fileInfo.name,
                            "attempt": "\(attemptCount)/\(maxRetries)",
                            "error": error.localizedDescription
                        ])
                        
                        if attemptCount >= maxRetries {
                            throw error
                        }
                        
                        try await Task.sleep(nanoseconds: 1_000_000_000)
                    }
                }
                
                accumulatedBytes += fileInfo.size
            }
            
            await MainActor.run {
                downloadedModels.insert(model)
                state = .downloaded
                scheduleDiskUsageUpdate(reason: "downloadCompleted")
            }

            // clear resume state on success: remove any resume data persisted for this model
            if let model = currentModel {
                try? FileManager.default.removeItem(at: resumeDataDirectory(for: model))
            }
            isPaused = false
            currentDownloadingFile = nil
            currentModel = nil
            
            logger.log(.info, "ModelDownloader", "Download completed successfully", metadata: [
                "model": model.rawValue,
                "files": "\(model.requiredFiles.count)"
            ])
        } catch {
            // If paused, keep partial files and don't treat as failure
            if let controlError = error as? ControlError, controlError == .paused {
                logger.log(.info, "ModelDownloader", "Download paused by user")
                await MainActor.run {
                    state = .paused
                }
                return
            }

            logger.log(.error, "ModelDownloader", "Download failed", metadata: ["error": error.localizedDescription])
            
            let isCancellationError = (error as NSError).code == NSURLErrorCancelled
            // Pause returns above. A failure or explicit cancellation must not
            // leave an invalid model directory behind as apparent local
            // storage or contaminate the next clean retry.
            try? FileManager.default.removeItem(at: getModelPath(for: model))
            
            await MainActor.run {
                state = .failed(error: error.localizedDescription)
                downloadedBytes = 0
                totalBytes = 0
                downloadSpeed = 0
                lastDownloadedBytes = 0
                scheduleDiskUsageUpdate(reason: isCancellationError ? "downloadCancelled" : "downloadFailed")
            }
            throw error
        }
    }
    
    /// Delete a downloaded model
    public func deleteModel(_ model: ModelType) throws {
        let modelPath = getModelPath(for: model)
        
        guard FileManager.default.fileExists(atPath: modelPath.path) else {
            logger.log(.warn, "ModelDownloader", "Model not found for deletion")
            return
        }
        
        try FileManager.default.removeItem(at: modelPath)
        downloadedModels.remove(model)
        
        if downloadedModels.isEmpty {
            state = .notDownloaded
        }
        
        scheduleDiskUsageUpdate(reason: "removeModel")
        
        logger.log(.info, "ModelDownloader", "Model deleted", metadata: ["model": model.rawValue])
    }
    
    /// Cancel ongoing download
    public func cancelDownload() {
        let cancelledModel = currentModel
        isCancelled = true
        // Cancel means discard. Pause is the explicit resumable operation.
        progressObservation?.invalidate()
        progressObservation = nil
        activeDownloadTask?.cancel()
        activeDownloadTask = nil
        isPaused = false
        
        downloadedBytes = 0
        totalBytes = 0
        downloadSpeed = 0
        lastDownloadedBytes = 0
        
        state = .notDownloaded
        currentDownloadingFile = nil
        currentModel = nil

        if let cancelledModel {
            try? FileManager.default.removeItem(at: getModelPath(for: cancelledModel))
        }

        scheduleDiskUsageUpdate(reason: "clearAll")
        
        logger.log(.info, "ModelDownloader", "Download cancelled")
        // No Rust cleanup required for Swift-only downloads
    }
    
    /// Pause download
    public func pauseDownload() {
        guard let task = activeDownloadTask else { return }
        isPaused = true

        let resumeHandler: (Data?) -> Void = { data in
            if let d = data, let model = self.currentModel, let fileName = self.currentDownloadingFile {
                do {
                    try self.saveResumeDataToDisk(d, for: model, fileName: fileName)
                    self.logger.log(.info, "ModelDownloader", "Saved resumeData to disk", metadata: ["model": model.rawValue, "file": fileName])
                } catch {
                    self.logger.log(.warn, "ModelDownloader", "Failed to save resumeData", metadata: ["error": error.localizedDescription])
                }
            } else {
                self.logger.log(.warn, "ModelDownloader", "No resumeData produced when pausing or missing context")
            }
            self.progressObservation?.invalidate()
            self.progressObservation = nil
            self.activeDownloadTask = nil
        }

        task.cancel(byProducingResumeData: resumeHandler)

        state = .paused
        logger.log(.info, "ModelDownloader", "Download paused")
    }
    
    /// Resume download
    public func resumeDownload() {
        // If we have resume data and a current model, restart the download flow
        guard isPaused else { return }
        isPaused = false
        logger.log(.info, "ModelDownloader", "Resuming download")
        if let model = currentModel {
            Task {
                do {
                    try await downloadModel(model)
                } catch {
                    self.logger.log(.error, "ModelDownloader", "Resume failed", metadata: ["error": error.localizedDescription])
                }
            }
        } else {
            logger.log(.warn, "ModelDownloader", "No model to resume")
        }
    }
    
    /// Get total size of downloaded models (scans actual files on disk)
    public func getTotalDownloadedSize() -> Int64 {
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        let modelsDir = cacheDir.appendingPathComponent("models")
        
        guard FileManager.default.fileExists(atPath: modelsDir.path) else {
            return 0
        }
        
        var totalSize: Int64 = 0
        
        guard let enumerator = FileManager.default.enumerator(
            at: modelsDir,
            includingPropertiesForKeys: [.fileSizeKey, .isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else {
            return 0
        }
        
        for case let fileURL as URL in enumerator {
            guard let resourceValues = try? fileURL.resourceValues(forKeys: [.fileSizeKey, .isDirectoryKey]),
                  let isDirectory = resourceValues.isDirectory,
                  !isDirectory,  // Count file size only for regular files.
                  let fileSize = resourceValues.fileSize else {
                continue
            }
            totalSize += Int64(fileSize)
        }
        
        return totalSize
    }
    
    /// Update cached disk usage (call after file operations)
    ///
    /// Important: This must not run on the main thread because it can enumerate large
    /// directories and block UI/startup.
    private func scheduleDiskUsageUpdate(reason: String) {
        diskUsageUpdateTask?.cancel()
        diskUsageUpdateTask = Task.detached(priority: .utility) { [weak self] in
            guard let self else { return }
            let size = Self.computeTotalDownloadedSize()
            guard !Task.isCancelled else { return }
            await MainActor.run {
                self.diskUsage = size
            }
        }
    }

    private nonisolated static func computeTotalDownloadedSize() -> Int64 {
        let fm = FileManager.default
        let cacheDir = fm.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        let modelsDir = cacheDir.appendingPathComponent("models")
        return Self.computeDirectorySize(at: modelsDir)
    }

    private nonisolated static func computeDirectorySize(at dir: URL) -> Int64 {
        var totalSize: Int64 = 0

        guard let enumerator = FileManager.default.enumerator(
            at: dir,
            includingPropertiesForKeys: [.fileSizeKey, .isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else {
            return 0
        }

        for case let fileURL as URL in enumerator {
            guard let resourceValues = try? fileURL.resourceValues(forKeys: [.fileSizeKey, .isDirectoryKey]),
                  let isDirectory = resourceValues.isDirectory,
                  !isDirectory,
                  let fileSize = resourceValues.fileSize else {
                continue
            }
            totalSize += Int64(fileSize)
        }

        return totalSize
    }
    
    // MARK: - Private Methods

    /// Download a single file with native URLSession progress and resume support.
    private func downloadSingleFile(from url: URL, fileName: String, to destinationURL: URL, fileSize: Int64, previousBytes: Int64, totalBytes: Int64) async throws {
        return try await withCheckedThrowingContinuation { continuation in
            if isCancelled {
                continuation.resume(throwing: NSError(domain: NSURLErrorDomain, code: NSURLErrorCancelled, userInfo: [NSLocalizedDescriptionKey: L10n.string("model_downloader.error.download_cancelled")]))
                return
            }

            // Track current downloading file
            self.currentDownloadingFile = fileName

            // If we have resume data for the current download file on disk, try resuming
            var usedResume: Data? = nil
            if let model = self.currentModel {
                usedResume = try? self.loadResumeDataFromDisk(for: model, fileName: fileName)
            }

            let completion: @Sendable (URL?, URLResponse?, Error?) -> Void = { temporaryURL, response, error in
                if let error {
                    Task { @MainActor in
                        self.progressObservation?.invalidate()
                        self.progressObservation = nil
                        self.activeDownloadTask = nil
                        continuation.resume(throwing: self.isPaused ? ControlError.paused : error)
                    }
                    return
                }
                if let http = response as? HTTPURLResponse,
                   !(200...299).contains(http.statusCode) {
                    Task { @MainActor in
                        self.progressObservation?.invalidate()
                        self.progressObservation = nil
                        self.activeDownloadTask = nil
                        continuation.resume(throwing: NSError(
                            domain: NSURLErrorDomain,
                            code: NSURLErrorBadServerResponse,
                            userInfo: [NSLocalizedDescriptionKey: "HTTP \(http.statusCode)"]
                        ))
                    }
                    return
                }
                guard let temporaryURL else {
                    Task { @MainActor in
                        self.progressObservation?.invalidate()
                        self.progressObservation = nil
                        self.activeDownloadTask = nil
                        continuation.resume(throwing: NSError(
                            domain: "ModelDownloader",
                            code: -1,
                            userInfo: [NSLocalizedDescriptionKey: L10n.string("model_downloader.error.no_downloaded_file_found")]
                        ))
                    }
                    return
                }
                do {
                    // URLSession owns the temporary file only while this
                    // completion callback is running. Moving it after an actor
                    // hop lets CFNetwork delete it first.
                    try FileManager.default.createDirectory(
                        at: destinationURL.deletingLastPathComponent(),
                        withIntermediateDirectories: true
                    )
                    if FileManager.default.fileExists(atPath: destinationURL.path) {
                        try FileManager.default.removeItem(at: destinationURL)
                    }
                    try FileManager.default.moveItem(at: temporaryURL, to: destinationURL)
                    Task { @MainActor in
                        self.progressObservation?.invalidate()
                        self.progressObservation = nil
                        self.activeDownloadTask = nil
                        if let model = self.currentModel {
                            try? self.removeResumeDataFromDisk(for: model, fileName: fileName)
                        }
                        continuation.resume(returning: ())
                    }
                } catch {
                    Task { @MainActor in
                        self.progressObservation?.invalidate()
                        self.progressObservation = nil
                        self.activeDownloadTask = nil
                        continuation.resume(throwing: error)
                    }
                }
            }

            let task: URLSessionDownloadTask
            if let resume = usedResume {
                task = URLSession.shared.downloadTask(withResumeData: resume, completionHandler: completion)
            } else {
                task = URLSession.shared.downloadTask(with: url, completionHandler: completion)
            }
            self.activeDownloadTask = task
            self.progressObservation = task.progress.observe(\.fractionCompleted, options: [.new]) { progress, _ in
                let currentFileBytes = Int64(Double(fileSize) * progress.fractionCompleted)
                let totalDownloaded = previousBytes + currentFileBytes
                Task { @MainActor in
                    self.downloadedBytes = totalDownloaded
                    let overallProgress = Double(totalDownloaded) / Double(totalBytes)
                    let now = Date()
                    let timeElapsed = now.timeIntervalSince(self.lastProgressUpdate)
                    if timeElapsed >= 1 {
                        let bytesDownloaded = totalDownloaded - self.lastDownloadedBytes
                        self.downloadSpeed = Double(bytesDownloaded) / timeElapsed
                        self.lastDownloadedBytes = totalDownloaded
                        self.lastProgressUpdate = now
                        if fileSize > 10_000_000 {
                            self.logger.log(.debug, "ModelDownloader", "Progress update", metadata: [
                                "progress": String(format: "%.1f%%", overallProgress * 100),
                                "downloaded": self.formatBytes(totalDownloaded),
                                "total": self.formatBytes(totalBytes),
                                "speed": self.formatBytes(Int64(self.downloadSpeed)) + "/s"
                            ])
                        }
                    }
                    let clampedProgress = max(0.0, min(0.99, overallProgress))
                    self.state = .downloading(progress: clampedProgress)
                }
            }
            task.resume()
        }
    }
    
    private func hasEnoughDiskSpace(for model: ModelType) -> Bool {
        guard let attributes = try? FileManager.default.attributesOfFileSystem(forPath: NSHomeDirectory()),
              let freeSpace = attributes[.systemFreeSize] as? Int64 else {
            return false
        }
        
        // Need at least 2x the model size (for download + extraction)
        return freeSpace > model.estimatedSize * 2
    }

    // MARK: - Resume data persistence (per model/file)

    private func resumeDataDirectory(for model: ModelType) -> URL {
        let modelPath = getModelPath(for: model)
        let resumeDir = modelPath.appendingPathComponent("resume", isDirectory: true)
        return resumeDir
    }

    private func resumeDataFileURL(for model: ModelType, fileName: String) -> URL {
        let sanitized = fileName.replacingOccurrences(of: "/", with: "_")
        return resumeDataDirectory(for: model).appendingPathComponent("\(sanitized).resume")
    }

    private func saveResumeDataToDisk(_ data: Data, for model: ModelType, fileName: String) throws {
        let dir = resumeDataDirectory(for: model)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = resumeDataFileURL(for: model, fileName: fileName)
        try data.write(to: url, options: .atomic)
    }

    private func loadResumeDataFromDisk(for model: ModelType, fileName: String) throws -> Data? {
        let url = resumeDataFileURL(for: model, fileName: fileName)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try Data(contentsOf: url)
    }

    private func removeResumeDataFromDisk(for model: ModelType, fileName: String) throws {
        let url = resumeDataFileURL(for: model, fileName: fileName)
        if FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
    }
    
    private func getModelSize(_ model: ModelType) -> Int64? {
        let modelPath = getModelPath(for: model)
        
        var totalSize: Int64 = 0
        
        guard let enumerator = FileManager.default.enumerator(at: modelPath, includingPropertiesForKeys: [.fileSizeKey], options: [.skipsHiddenFiles]) else {
            return nil
        }
        
        for case let fileURL as URL in enumerator {
            guard let attributes = try? fileURL.resourceValues(forKeys: [.fileSizeKey]),
                  let fileSize = attributes.fileSize else {
                continue
            }
            totalSize += Int64(fileSize)
        }
        
        return totalSize > 0 ? totalSize : nil
    }
    
    private func formatBytes(_ bytes: Int64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: bytes)
    }
    
    /// Calculate SHA256 hash of a file
    private func calculateSHA256(for fileURL: URL) throws -> String {
        let bufferSize = 1024 * 1024 // 1 MB buffer
        
        guard let fileHandle = try? FileHandle(forReadingFrom: fileURL) else {
            throw NSError(domain: "ModelDownloader", code: -1, userInfo: [NSLocalizedDescriptionKey: L10n.string("model_downloader.error.unable_to_open_file")])
        }
        
        defer {
            try? fileHandle.close()
        }
        
        var hasher = SHA256()
        
        while autoreleasepool(invoking: {
            let data = fileHandle.readData(ofLength: bufferSize)
            if data.count > 0 {
                hasher.update(data: data)
                return true
            } else {
                return false
            }
        }) {}
        
        let digest = hasher.finalize()
        return digest.map { String(format: "%02x", $0) }.joined()
    }
    
    /// Verify file integrity by comparing SHA256 hash
    private func verifyFileIntegrity(at fileURL: URL, expectedHash: String) throws -> Bool {
        guard !expectedHash.isEmpty else {
            logger.log(.warn, "ModelDownloader", "Skipping hash verification (no hash provided)", metadata: ["file": fileURL.lastPathComponent])
            return true
        }

        let actualHash = try calculateSHA256(for: fileURL)
        let isValid = actualHash.lowercased() == expectedHash.lowercased()

        if !isValid {
            logger.log(.error, "ModelDownloader", "Hash mismatch", metadata: [
                "file": fileURL.lastPathComponent,
                "expected": expectedHash,
                "actual": actualHash
            ])
        }

        return isValid
    }

}
