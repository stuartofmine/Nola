// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Persists the MLX model selected for native on-device macOS chat.

import Foundation

@MainActor
final class MacLocalModelConfigurationStore {
    static let shared = MacLocalModelConfigurationStore()
    private let modelClient: any MacLocalModelApplicationClient
    /// File validation is event-invalidated by downloads/configuration changes.
    /// A time-based cache still performed Date/locking work during every SwiftUI
    /// recomputation and made the model picker visibly sluggish.
    private var readinessByProfileID: [String: Bool] = [:]

    init(
        applicationPort: any MacLocalModelApplicationClient = MacApplicationPort.shared
    ) {
        self.modelClient = applicationPort
    }

    var selectedModel: ModelDownloader.ModelType {
        if let model = ModelDownloader.ModelType(rawValue: selectedModelID) {
            return model
        }
        return ModelDownloader.ModelType.allCases[0]
    }

    var selectedModelID: String {
        if let profiles = try? modelClient.listModelProfiles(),
           let selected = profiles.first(where: {
               $0.provider == "local" && $0.runtime == "mlx" && $0.selected && $0.enabled
           }) {
            return selected.id
        }
        return ModelDownloader.ModelType.allCases[0].rawValue
    }

    var selectedProfile: MacLocalModelProfile {
        MacLocalModelProfile.resolve(selectedModelID) ?? .qwen35_08B
    }

    var selectedModelDirectory: URL {
        selectedProfile.installedDirectory ?? modelDirectory(for: selectedModel)
    }

    var isSelectedModelReady: Bool {
        isReady(selectedProfile)
    }

    var isSelectedModelRunning: Bool {
        MacMLXOpenAIService.shared.isRunning(selectedProfile)
    }

    func select(_ model: ModelDownloader.ModelType) throws {
        guard let profile = MacLocalModelProfile.resolve(model.rawValue) else {
            throw ConfigurationError.unknownModel(model.rawValue)
        }
        try select(profile)
    }

    func select(_ profile: MacLocalModelProfile) throws {
        try persist(profile: profile, selected: true)
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    func parameters(for profile: MacLocalModelProfile) -> ModelParameters {
        if let stored = try? modelClient.modelProfile(profile.id),
           let data = stored.parametersJSON.data(using: .utf8),
           let parameters = try? JSONDecoder().decode(ModelParameters.self, from: data) {
            return parameters
        }
        return ModelParameters.localDefaults
    }

    struct ParameterSnapshot {
        let parameters: ModelParameters
        let revision: UInt64
    }

    func parameterSnapshot(for profile: MacLocalModelProfile) throws -> ParameterSnapshot {
        guard let stored = try modelClient.modelProfile(profile.id) else {
            return ParameterSnapshot(parameters: .localDefaults, revision: 0)
        }
        guard let revision = UInt64(exactly: stored.revision),
              let data = stored.parametersJSON.data(using: .utf8) else {
            throw MacApplicationServiceError.invalidResponse
        }
        return ParameterSnapshot(parameters: try JSONDecoder().decode(ModelParameters.self, from: data), revision: revision)
    }

    @discardableResult
    func saveParameters(_ parameters: ModelParameters, for profile: MacLocalModelProfile,
                        expectedRevision: UInt64? = nil) throws -> UInt64 {
        let expected: UInt64
        if let expectedRevision {
            expected = expectedRevision
        } else {
            // A complete replacement need not decode the previous payload.
            // Editors pass the revision captured alongside their displayed data.
            let stored = try modelClient.modelProfile(profile.id)
            guard let revision = UInt64(exactly: stored?.revision ?? 0) else {
                throw MacApplicationServiceError.invalidResponse
            }
            expected = revision
        }
        let revision = try modelClient.configureModelParameters(modelRecord(
            profile: profile, selected: false, parameters: parameters,
            createdAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        ), expectedRevision: expected)
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        return revision
    }

    func persist(
        profile: MacLocalModelProfile,
        selected: Bool,
        parameters: ModelParameters? = nil
    ) throws {
        let store = modelClient
        let existing = try store.modelProfile(profile.id)
        let effectiveParameters = parameters ?? self.parameters(for: profile)
        try store.upsertModelProfile(modelRecord(
            profile: profile, selected: selected, parameters: effectiveParameters,
            createdAtMs: existing?.createdAtMs ?? Int64(Date().timeIntervalSince1970 * 1_000)
        ))
    }

    private func modelRecord(
        profile: MacLocalModelProfile,
        selected: Bool,
        parameters: ModelParameters,
        createdAtMs: Int64
    ) throws -> MacCanonicalModelProfile {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        var source: [String: String] = ["modelID": profile.id, "format": profile.format]
        source["installedDirectory"] = profile.installedDirectory?.path
        let parameterData = try JSONEncoder().encode(parameters)
        let sourceData = try JSONSerialization.data(withJSONObject: source)
        guard let parameterJSON = String(data: parameterData, encoding: .utf8),
              let sourceJSON = String(data: sourceData, encoding: .utf8) else {
            throw ConfigurationError.invalidEncodedState
        }
        return MacCanonicalModelProfile(
            id: profile.id,
            displayName: profile.displayName,
            provider: "local",
            modelName: profile.id,
            runtime: "mlx",
            sourceJSON: sourceJSON,
            parametersJSON: parameterJSON,
            selected: selected,
            enabled: true,
            revision: 0,
            createdAtMs: createdAtMs,
            updatedAtMs: now,
            deletedAtMs: nil
        )
    }

    private enum ConfigurationError: LocalizedError {
        case invalidEncodedState
        case unknownModel(String)

        var errorDescription: String? {
            switch self {
            case .invalidEncodedState:
                return String(localized: "The local model configuration could not be encoded.", table: "ModelCenter")
            case .unknownModel(let id):
                return String(format: String(localized: "Unknown local model: %@", table: "ModelCenter"), id)
            }
        }
    }

    func isReady(_ profile: MacLocalModelProfile) -> Bool {
        if let cached = readinessByProfileID[profile.id] {
            return cached
        }
        let ready: Bool
        if let directory = profile.installedDirectory {
            ready = FileManager.default.fileExists(
                atPath: directory.appendingPathComponent("config.json").path
            )
        } else if let model = ModelDownloader.ModelType(rawValue: profile.id) {
            ready = model.validatesDownloadedFiles(at: modelDirectory(for: model))
        } else {
            ready = false
        }
        readinessByProfileID[profile.id] = ready
        return ready
    }

    func isReady(_ model: ModelDownloader.ModelType) -> Bool {
        guard let profile = MacLocalModelProfile.resolve(model.rawValue) else {
            return model.validatesDownloadedFiles(at: modelDirectory(for: model))
        }
        return isReady(profile)
    }

    func invalidateReadinessCache() {
        readinessByProfileID.removeAll(keepingCapacity: true)
    }

    func modelDirectory(for model: ModelDownloader.ModelType) -> URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(model.rawValue, isDirectory: true)
    }
}
