// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the single global storage contract for macOS assistant preferences.

import Foundation

struct MacStructuredMemoryRecord: Codable, Equatable, Identifiable {
    let id: String
    var namespace: String
    var scope: String
    var kind: String
    var content: String
    var confidence: Double
    var provenanceKind: String
    var sourceReference: String?
    var observedAt: Date?
    var createdAt: Date
    var updatedAt: Date
    var retention: String
    var expiresAt: Date?
    var supersedes: [String]
    var tags: [String]
    var contentHash: String?
    var previousVersionID: String?
}

nonisolated enum MacProductSettingKeys {
    static let memoryEnabled = "assistant.memory.enabled"
    static let userPreferences = "assistant.user_preferences"
    static let retrievalEnabled = "assistant.retrieval.enabled"
    static let contextLimit = "assistant.context_limit"
    static let hiddenBuiltInSkills = "skill.builtin.hidden"
    static let promptTemplates = "prompt.templates"
    static let defaultPromptTemplate = "prompt.default_template"
    static let connectedHostConfiguration = "connected_host.configuration"
    static let backedUpKeys = [
        memoryEnabled,
        userPreferences,
        retrievalEnabled,
        contextLimit,
        hiddenBuiltInSkills,
        promptTemplates,
        defaultPromptTemplate,
        connectedHostConfiguration,
    ]
}

enum MacAssistantPreferences {
    nonisolated private struct Setting<Value: Codable>: Codable {
        let schemaVersion: Int
        let value: Value
    }

    static var userPreferences: String {
        userPreferences(in: MacApplicationPort.shared)
    }

    static var retrievalEnabled: Bool {
        retrievalEnabled(in: MacApplicationPort.shared)
    }

    static var memoryEnabled: Bool {
        memoryEnabled(in: MacApplicationPort.shared)
    }

    nonisolated static func memoryEnabled(in store: MacApplicationPort) -> Bool {
        value(for: MacProductSettingKeys.memoryEnabled, default: false, in: store)
    }

    static func setMemoryEnabled(_ enabled: Bool, in store: MacApplicationPort = MacApplicationPort.shared) {
        set(enabled, for: MacProductSettingKeys.memoryEnabled, in: store)
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    static var contextLimit: Int {
        contextLimit(in: MacApplicationPort.shared)
    }

    static func userPreferences(in store: MacApplicationPort) -> String {
        value(for: MacProductSettingKeys.userPreferences, default: "", in: store)
    }

    static func setUserPreferences(_ value: String, in store: MacApplicationPort = MacApplicationPort.shared) {
        set(value, for: MacProductSettingKeys.userPreferences, in: store)
    }

    nonisolated static func retrievalEnabled(in store: MacApplicationPort) -> Bool {
        value(for: MacProductSettingKeys.retrievalEnabled, default: true, in: store)
    }

    static func setRetrievalEnabled(_ enabled: Bool, in store: MacApplicationPort = MacApplicationPort.shared) {
        set(enabled, for: MacProductSettingKeys.retrievalEnabled, in: store)
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
    }

    static func contextLimit(in store: MacApplicationPort) -> Int {
        max(16_384, value(for: MacProductSettingKeys.contextLimit, default: 128_000, in: store))
    }

    static func setContextLimit(_ limit: Int, in store: MacApplicationPort = MacApplicationPort.shared) {
        set(min(max(limit, 16_384), 2_000_000), for: MacProductSettingKeys.contextLimit, in: store)
    }

    nonisolated private static func value<Value: Codable>(
        for key: String,
        default defaultValue: Value,
        in store: MacApplicationPort
    ) -> Value {
        guard let setting = try? store.productSetting(key: key),
              let data = setting.valueJSON.data(using: .utf8),
              let decoded = try? JSONDecoder().decode(Setting<Value>.self, from: data),
              decoded.schemaVersion == 1 else { return defaultValue }
        return decoded.value
    }

    private static func set<Value: Codable>(
        _ value: Value,
        for key: String,
        in store: MacApplicationPort
    ) {
        guard let data = try? JSONEncoder().encode(Setting(schemaVersion: 1, value: value)),
              let valueJSON = String(data: data, encoding: .utf8) else { return }
        _ = try? store.putProductSetting(
            key: key,
            valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }
}
