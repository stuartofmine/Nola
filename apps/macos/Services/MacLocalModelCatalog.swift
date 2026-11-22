// SPDX-License-Identifier: MIT
// Device-aware catalog for installed local models and the built-in MLX presets.

import Foundation

nonisolated enum MacLocalInferenceEngine: String, CaseIterable, Sendable {
    case mlx
    case llamaCpp = "llama.cpp"
    case vllm

    static var defaultForMac: Self {
        #if arch(arm64)
        .mlx
        #else
        .llamaCpp
        #endif
    }

    var title: String {
        switch self {
        case .mlx: "MLX"
        case .llamaCpp: "llama.cpp"
        case .vllm: "vLLM"
        }
    }

    var format: String {
        switch self {
        case .mlx: "mlx"
        case .llamaCpp: "gguf"
        case .vllm: "safetensors"
        }
    }

    var adapter: String { self == .llamaCpp ? "llama_cpp" : rawValue }
}

nonisolated struct MacLocalModelProfile: Identifiable, Hashable, Sendable {
    enum Readiness: Hashable, Sendable {
        case recommended
        case available
        case memoryConstrained
    }

    static let minimumContextTokens = 65_536

    let id: String
    let displayName: String
    let summary: String
    let parameterCount: String
    let downloadBytes: Int64
    let estimatedRuntimeBytesAt64K: Int64
    let recommendedUnifiedMemoryBytes: UInt64
    let nativeContextTokens: Int
    let installedDirectory: URL?
    let runtime: String
    let format: String
    var artifactURL: URL? = nil
    var repository: String? = nil
    var recommendedContextTokens: Int? = nil
    var recommendedDevice: String? = nil
    var engine: MacLocalInferenceEngine? { MacLocalInferenceEngine(rawValue: runtime) }

    /// The catalog advertises the native ceiling. The effective runtime window is
    /// frozen by `MacMLXOpenAIService.start` from the user's model parameters.
    var contextTokens: Int { nativeContextTokens }

    var configurableContext: ClosedRange<Int> {
        let minimum = installedDirectory == nil ? Self.minimumContextTokens : 512
        return min(minimum, nativeContextTokens)...nativeContextTokens
    }

    var defaultParameters: ModelParameters {
        var value = ModelParameters.localDefaults
        value.maxContextLength = min(max(configurableContext.lowerBound, recommendedContextTokens ?? value.maxContextLength), nativeContextTokens)
        value.maxOutputTokens = min(value.maxOutputTokens, max(1, value.maxContextLength / 4))
        return value
    }

    var readiness: Readiness {
        let memory = ProcessInfo.processInfo.physicalMemory
        if memory < recommendedUnifiedMemoryBytes {
            return .memoryConstrained
        }
        return id == Self.qwen35_4B.id ? .recommended : .available
    }

    var downloadText: String { Self.storageFormatter.string(fromByteCount: downloadBytes) }
    var runtimeText: String { Self.storageFormatter.string(fromByteCount: estimatedRuntimeBytesAt64K) }
    var recommendedMemoryText: String {
        Self.storageFormatter.string(fromByteCount: Int64(recommendedUnifiedMemoryBytes))
    }

    static let qwen35_08B = MacLocalModelProfile(
        id: "mlx-community/Qwen3.5-0.8B-MLX-4bit",
        displayName: "Qwen3.5 0.8B",
        summary: String(localized: "Fast, compact, and suitable for lightweight conversations."),
        parameterCount: "0.8B",
        downloadBytes: 652_100_000,
        estimatedRuntimeBytesAt64K: 3_000_000_000,
        recommendedUnifiedMemoryBytes: 8_000_000_000,
        nativeContextTokens: 262_144,
        installedDirectory: nil,
        runtime: "mlx",
        format: "mlx"
    )

    static let qwen35_4B = MacLocalModelProfile(
        id: "mlx-community/Qwen3.5-4B-MLX-4bit",
        displayName: "Qwen3.5 4B",
        summary: String(localized: "Recommended for higher-quality writing, reasoning, and tool use."),
        parameterCount: "4B",
        downloadBytes: 3_060_000_000,
        estimatedRuntimeBytesAt64K: 7_000_000_000,
        recommendedUnifiedMemoryBytes: 16_000_000_000,
        nativeContextTokens: 262_144,
        installedDirectory: nil,
        runtime: "mlx",
        format: "mlx"
    )

    static let all: [MacLocalModelProfile] = [.qwen35_08B, .qwen35_4B]

    static var available: [MacLocalModelProfile] {
        let builtInIDs = Set(all.map(\.id))
        return all + installed.filter { !builtInIDs.contains($0.id) }
    }

    static var installed: [MacLocalModelProfile] {
        installed(in: modelsRoot)
    }

    static func loadInstalledAsync(in root: URL = modelsRoot) async -> [MacLocalModelProfile] {
        await Task.detached(priority: .userInitiated) {
            installed(in: root)
        }.value
    }

    static func installed(in root: URL) -> [MacLocalModelProfile] {
        let root = root.standardizedFileURL.resolvingSymlinksInPath()
        let catalogURL = root.appendingPathComponent("catalog.json")
        guard let data = try? Data(contentsOf: catalogURL),
              let document = try? JSONDecoder().decode(InstalledCatalog.self, from: data) else { return [] }
        return document.models.compactMap { record in
            let context = record.contextWindowTokens ?? 4096
            guard record.enabled != false,
                  record.state == "configured",
                  let engine = MacLocalInferenceEngine(rawValue: record.runtime),
                  record.format == engine.format,
                  context > 0,
                  record.artifactBytes > 0, record.artifactBytes <= Int64.max / 2,
                  let snapshotPath = record.snapshotPath else { return nil }
            let directory = URL(fileURLWithPath: snapshotPath).standardizedFileURL.resolvingSymlinksInPath()
            guard directory.path.hasPrefix(root.standardizedFileURL.path + "/"),
                  validateSnapshot(directory, engine: engine) else { return nil }
            let artifact = record.artifactPath.map { URL(fileURLWithPath: $0).standardizedFileURL.resolvingSymlinksInPath() }
            if engine == .llamaCpp {
                guard let artifact, artifact.path.hasPrefix(directory.path + "/"),
                      artifact.pathExtension.lowercased() == "gguf",
                      (try? artifact.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
                else { return nil }
            }
            return MacLocalModelProfile(
                id: record.id,
                displayName: record.name,
                summary: String(localized: "Installed from a verified model snapshot."),
                parameterCount: record.quantization ?? String(localized: "Custom"),
                downloadBytes: record.artifactBytes,
                estimatedRuntimeBytesAt64K: record.estimatedPeakMemoryBytes ?? max(record.artifactBytes * 2, record.artifactBytes),
                recommendedUnifiedMemoryBytes: UInt64(max(0, record.recommendedMemoryBytes ?? record.estimatedPeakMemoryBytes ?? max(record.artifactBytes * 2, 8_000_000_000))),
                nativeContextTokens: context,
                installedDirectory: directory,
                runtime: record.runtime,
                format: record.format,
                artifactURL: artifact,
                repository: record.repository,
                recommendedContextTokens: record.recommendedContextTokens,
                recommendedDevice: record.recommendedDevice
            )
        }
    }

    static func resolve(_ value: String) -> MacLocalModelProfile? {
        available.first { $0.id == value }
    }

    static var recommended: MacLocalModelProfile {
        ProcessInfo.processInfo.physicalMemory >= qwen35_4B.recommendedUnifiedMemoryBytes
            ? qwen35_4B : qwen35_08B
    }

    private static let storageFormatter: ByteCountFormatter = {
        let formatter = ByteCountFormatter()
        formatter.allowedUnits = [.useGB, .useMB]
        formatter.countStyle = .file
        formatter.includesUnit = true
        formatter.isAdaptive = true
        return formatter
    }()

    static var modelsRoot: URL {
        if let override = ProcessInfo.processInfo.environment["NOLA_MODELS_ROOT"], override.hasPrefix("/") {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("models", isDirectory: true)
    }

    private static func validateSnapshot(_ directory: URL, engine: MacLocalInferenceEngine) -> Bool {
        let manifestURL = directory.appendingPathComponent("snapshot.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONDecoder().decode(SnapshotManifest.self, from: data),
              !manifest.artifacts.isEmpty else { return false }
        let prefix = directory.standardizedFileURL.path + "/"
        var total: Int64 = 0
        for artifact in manifest.artifacts {
            let file = directory.appendingPathComponent(artifact.path).standardizedFileURL.resolvingSymlinksInPath()
            guard file.path.hasPrefix(prefix),
                  let values = try? file.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
                  values.isRegularFile == true,
                  Int64(values.fileSize ?? -1) == artifact.bytes else { return false }
            guard artifact.bytes >= 0, artifact.bytes <= Int64.max - total else { return false }
            total += artifact.bytes
        }
        return total == manifest.totalBytes
            && (engine == .llamaCpp || FileManager.default.fileExists(atPath: directory.appendingPathComponent("config.json").path))
    }

    private enum SchemaKey: CodingKey { case schemaVersion }

    private static func requireSupportedVersion(_ decoder: Decoder, maximum: Int) throws {
        let header = try decoder.container(keyedBy: SchemaKey.self)
        guard header.contains(.schemaVersion) else { return }
        let version = try header.decode(Int.self, forKey: .schemaVersion)
        guard (1...maximum).contains(version) else {
            throw DecodingError.dataCorruptedError(forKey: .schemaVersion, in: header, debugDescription: "Unsupported model storage format.")
        }
    }

    private struct InstalledCatalog: Decodable {
        let models: [InstalledRecord]
        private enum CodingKeys: CodingKey { case models }

        init(from decoder: Decoder) throws {
            try MacLocalModelProfile.requireSupportedVersion(decoder, maximum: 2)
            models = try decoder.container(keyedBy: CodingKeys.self).decode([InstalledRecord].self, forKey: .models)
        }
    }

    private struct InstalledRecord: Codable {
        let id: String
        let name: String
        let repository: String?
        let runtime: String
        let format: String
        let quantization: String?
        let contextWindowTokens: Int?
        let recommendedContextTokens: Int?
        let recommendedDevice: String?
        let snapshotPath: String?
        let artifactPath: String?
        let artifactBytes: Int64
        let estimatedPeakMemoryBytes: Int64?
        let recommendedMemoryBytes: Int64?
        let state: String
        let enabled: Bool?
    }

    private struct SnapshotManifest: Decodable {
        let totalBytes: Int64
        let artifacts: [SnapshotArtifact]
        private enum CodingKeys: CodingKey { case totalBytes, artifacts }

        init(from decoder: Decoder) throws {
            try MacLocalModelProfile.requireSupportedVersion(decoder, maximum: 1)
            let values = try decoder.container(keyedBy: CodingKeys.self)
            totalBytes = try values.decode(Int64.self, forKey: .totalBytes)
            artifacts = try values.decode([SnapshotArtifact].self, forKey: .artifacts)
        }
    }

    private struct SnapshotArtifact: Decodable {
        let path: String
        let bytes: Int64
    }
}
