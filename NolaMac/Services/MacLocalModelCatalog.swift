// SPDX-License-Identifier: MIT
// Device-aware catalog for native MLX models offered by Nola on macOS.

import Foundation

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

    /// The catalog advertises the native ceiling. The effective runtime window is
    /// frozen by `MacMLXOpenAIService.start` from the user's model parameters.
    var contextTokens: Int { nativeContextTokens }

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
        let root = modelsRoot
        let catalogURL = root.appendingPathComponent("catalog.json")
        guard let data = try? Data(contentsOf: catalogURL),
              let document = try? JSONDecoder().decode(InstalledCatalog.self, from: data) else { return [] }
        return document.models.compactMap { record in
            guard record.enabled != false,
                  record.state == "configured",
                  record.runtime == "mlx",
                  record.format == "mlx",
                  record.contextWindowTokens >= minimumContextTokens,
                  let snapshotPath = record.snapshotPath else { return nil }
            let directory = URL(fileURLWithPath: snapshotPath).standardizedFileURL
            guard directory.path.hasPrefix(root.standardizedFileURL.path + "/"),
                  validateSnapshot(directory) else { return nil }
            return MacLocalModelProfile(
                id: record.id,
                displayName: record.name,
                summary: String(localized: "Installed from a verified model snapshot."),
                parameterCount: record.quantization ?? String(localized: "Custom"),
                downloadBytes: record.artifactBytes,
                estimatedRuntimeBytesAt64K: record.estimatedPeakMemoryBytes ?? max(record.artifactBytes * 2, record.artifactBytes),
                recommendedUnifiedMemoryBytes: UInt64(record.recommendedMemoryBytes ?? record.estimatedPeakMemoryBytes ?? max(record.artifactBytes * 2, 8_000_000_000)),
                nativeContextTokens: record.contextWindowTokens,
                installedDirectory: directory,
                runtime: record.runtime,
                format: record.format
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
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("models", isDirectory: true)
    }

    static func removeInstalled(_ profile: MacLocalModelProfile) throws {
        guard let snapshot = profile.installedDirectory else { return }
        let root = modelsRoot.standardizedFileURL
        let modelDirectory = snapshot.deletingLastPathComponent().deletingLastPathComponent().standardizedFileURL
        guard modelDirectory.deletingLastPathComponent() == root else {
            throw CocoaError(.fileReadNoPermission)
        }
        let catalogURL = root.appendingPathComponent("catalog.json")
        guard let data = try? Data(contentsOf: catalogURL),
              var document = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let models = document["models"] as? [[String: Any]] else {
            throw CocoaError(.fileReadCorruptFile)
        }
        try FileManager.default.removeItem(at: modelDirectory)
        document["models"] = models.filter { ($0["id"] as? String) != profile.id }
        let encoded = try JSONSerialization.data(withJSONObject: document, options: [.prettyPrinted, .sortedKeys])
        try encoded.write(to: catalogURL, options: .atomic)
    }

    private static func validateSnapshot(_ directory: URL) -> Bool {
        let manifestURL = directory.appendingPathComponent("snapshot.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONDecoder().decode(SnapshotManifest.self, from: data),
              !manifest.artifacts.isEmpty else { return false }
        let prefix = directory.standardizedFileURL.path + "/"
        var total: Int64 = 0
        for artifact in manifest.artifacts {
            let file = directory.appendingPathComponent(artifact.path).standardizedFileURL
            guard file.path.hasPrefix(prefix),
                  let values = try? file.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
                  values.isRegularFile == true,
                  Int64(values.fileSize ?? -1) == artifact.bytes else { return false }
            total += artifact.bytes
        }
        return total == manifest.totalBytes
            && FileManager.default.fileExists(atPath: directory.appendingPathComponent("config.json").path)
    }

    private struct InstalledCatalog: Codable {
        var models: [InstalledRecord]
    }

    private struct InstalledRecord: Codable {
        let id: String
        let name: String
        let runtime: String
        let format: String
        let quantization: String?
        let contextWindowTokens: Int
        let snapshotPath: String?
        let artifactBytes: Int64
        let estimatedPeakMemoryBytes: Int64?
        let recommendedMemoryBytes: Int64?
        let state: String
        let enabled: Bool?
    }

    private struct SnapshotManifest: Decodable {
        let totalBytes: Int64
        let artifacts: [SnapshotArtifact]
    }

    private struct SnapshotArtifact: Decodable {
        let path: String
        let bytes: Int64
    }
}
