// SPDX-License-Identifier: Apache-2.0
// Canonical persistence for the iOS device capabilities exposed to a paired host.

import Foundation

nonisolated struct IOSHybridCapabilitySettings: Codable, Equatable, Sendable {
    static let productSettingKey = "chat.hybrid.capabilities"
    static let schemaVersion = 1

    var schemaVersion = Self.schemaVersion
    var boundHostURL = ""
    var deviceToolsEnabled = false
    var filesEnabled = false
    var clipboardEnabled = false
    var locationEnabled = false
    var calendarEnabled = false

    @MainActor static func load(store: IOSApplicationRuntime? = .shared) throws -> Self {
        guard let store else {
            throw IOSHybridCapabilitySettingsError.applicationRuntimeUnavailable
        }
        guard let setting = try store.productSetting(key: productSettingKey) else {
            return Self()
        }
        guard let data = setting.valueJSON.data(using: .utf8) else {
            throw IOSHybridCapabilitySettingsError.invalidEncoding
        }
        let object = try JSONSerialization.jsonObject(with: data)
        guard let dictionary = object as? [String: Any] else {
            throw IOSHybridCapabilitySettingsError.invalidPayload
        }
        let allowedKeys: Set<String> = [
            "schemaVersion", "boundHostURL", "deviceToolsEnabled", "filesEnabled",
            "clipboardEnabled", "locationEnabled", "calendarEnabled"
        ]
        let unknownKeys = Set(dictionary.keys).subtracting(allowedKeys)
        guard unknownKeys.isEmpty else {
            throw IOSHybridCapabilitySettingsError.unknownFields(unknownKeys.sorted())
        }
        let settings = try JSONDecoder().decode(Self.self, from: data)
        guard settings.schemaVersion == schemaVersion else {
            throw IOSHybridCapabilitySettingsError.unsupportedSchema(settings.schemaVersion)
        }
        return settings
    }

    @MainActor func save(store: IOSApplicationRuntime? = .shared) throws {
        guard let store else {
            throw IOSHybridCapabilitySettingsError.applicationRuntimeUnavailable
        }
        guard schemaVersion == Self.schemaVersion else {
            throw IOSHybridCapabilitySettingsError.unsupportedSchema(schemaVersion)
        }
        let data = try JSONEncoder().encode(self)
        guard let valueJSON = String(data: data, encoding: .utf8) else {
            throw IOSHybridCapabilitySettingsError.invalidEncoding
        }
        _ = try store.putProductSetting(
            key: Self.productSettingKey,
            valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }

    @MainActor static func bindHost(_ rawURL: String, store: IOSApplicationRuntime? = .shared) throws {
        let normalized = ModelSelectionManager.normalizedWebHostURL(rawURL)
        var settings = try load(store: store)
        guard settings.boundHostURL != normalized else { return }
        settings = Self(boundHostURL: normalized)
        try settings.save(store: store)
    }
}

enum IOSHybridCapabilitySettingsError: LocalizedError, Equatable {
    case applicationRuntimeUnavailable
    case invalidEncoding
    case invalidPayload
    case unknownFields([String])
    case unsupportedSchema(Int)

    var errorDescription: String? {
        switch self {
        case .applicationRuntimeUnavailable:
            return "The Application runtime is unavailable."
        case .invalidEncoding:
            return "The hybrid capability payload is not valid UTF-8."
        case .invalidPayload:
            return "The hybrid capability payload is not a JSON object."
        case .unknownFields(let fields):
            return "Unknown hybrid capability fields: \(fields.joined(separator: ", "))."
        case .unsupportedSchema(let version):
            return "Unsupported hybrid capability schema version: \(version)."
        }
    }
}
