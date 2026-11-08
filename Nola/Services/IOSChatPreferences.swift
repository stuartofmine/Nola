// SPDX-License-Identifier: Apache-2.0
// Canonical, current-schema chat preferences shared by the iOS UI and runtime.

import Foundation

struct IOSChatPreferences: Codable, Equatable {
    static let productSettingKey = "chat.preferences"
    static let schemaVersion = 1

    var schemaVersion = Self.schemaVersion
    var autoCompactTriggerPercent = ChatManager.autoCompactTriggerPercentDefault

    static func load(store: IOSApplicationRuntime? = .shared) throws -> Self {
        guard let store else {
            throw IOSChatPreferencesError.applicationRuntimeUnavailable
        }
        guard let setting = try store.productSetting(key: productSettingKey) else {
            return Self()
        }
        guard let data = setting.valueJSON.data(using: .utf8) else {
            throw IOSChatPreferencesError.invalidEncoding
        }
        let preferences = try JSONDecoder().decode(Self.self, from: data)
        guard preferences.schemaVersion == schemaVersion else {
            throw IOSChatPreferencesError.unsupportedSchema(preferences.schemaVersion)
        }
        guard ChatManager.autoCompactTriggerPercentRange.contains(preferences.autoCompactTriggerPercent) else {
            throw IOSChatPreferencesError.invalidAutoCompactTrigger(preferences.autoCompactTriggerPercent)
        }
        return preferences
    }

    func save(store: IOSApplicationRuntime? = .shared) throws {
        guard let store else {
            throw IOSChatPreferencesError.applicationRuntimeUnavailable
        }
        guard Self.schemaVersion == schemaVersion else {
            throw IOSChatPreferencesError.unsupportedSchema(schemaVersion)
        }
        guard ChatManager.autoCompactTriggerPercentRange.contains(autoCompactTriggerPercent) else {
            throw IOSChatPreferencesError.invalidAutoCompactTrigger(autoCompactTriggerPercent)
        }
        let data = try JSONEncoder().encode(self)
        guard let valueJSON = String(data: data, encoding: .utf8) else {
            throw IOSChatPreferencesError.invalidEncoding
        }
        _ = try store.putProductSetting(
            key: Self.productSettingKey,
            valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }
}

enum IOSChatPreferencesError: LocalizedError, Equatable {
    case applicationRuntimeUnavailable
    case invalidEncoding
    case unsupportedSchema(Int)
    case invalidAutoCompactTrigger(Int)

    var errorDescription: String? {
        switch self {
        case .applicationRuntimeUnavailable:
            return "The Application runtime is unavailable."
        case .invalidEncoding:
            return "The chat preferences payload is not valid UTF-8."
        case .unsupportedSchema(let version):
            return "Unsupported chat preferences schema version: \(version)."
        case .invalidAutoCompactTrigger(let value):
            return "Invalid auto-compact trigger percentage: \(value)."
        }
    }
}
