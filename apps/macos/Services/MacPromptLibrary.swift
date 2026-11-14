// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exposes the shared iOS/macOS prompt-template service to desktop scenes.

import Foundation

@MainActor
enum MacPromptLibrary {
    static let service: PromptTemplateService = {
        let service = PromptTemplateService(
            logger: .shared,
            persistence: MacPromptTemplatePersistence(store: MacApplicationPort.shared)
        )
        service.selectBuiltInLanguage(preferredLanguages: Locale.preferredLanguages)
        return service
    }()

    static func defaultTemplateID() throws -> UUID? {
        try defaultTemplateID(using: .shared)
    }

    static func defaultTemplateID(using store: MacApplicationPort) throws -> UUID? {
        guard let setting = try store.productSetting(key: MacProductSettingKeys.defaultPromptTemplate) else { return nil }
        guard let raw = try? JSONDecoder().decode(String.self, from: Data(setting.valueJSON.utf8)),
              let id = UUID(uuidString: raw) else { throw PromptTemplateReadError.unavailable }
        let scoped = PromptTemplateService(logger: .shared, persistence: MacPromptTemplatePersistence(store: store))
        _ = try scoped.requireReadableTemplate(id: id)
        return id
    }

    static func setDefaultTemplateID(_ id: UUID?) throws {
        try setDefaultTemplateID(id, using: .shared)
    }

    static func setDefaultTemplateID(_ id: UUID?, using store: MacApplicationPort) throws {
        if let id {
            let scoped = PromptTemplateService(logger: .shared, persistence: MacPromptTemplatePersistence(store: store))
            _ = try scoped.requireReadableTemplate(id: id)
            _ = try store.putProductSetting(key: MacProductSettingKeys.defaultPromptTemplate,
                valueJSON: String(decoding: try JSONEncoder().encode(id.uuidString), as: UTF8.self),
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
        } else { try store.deleteProductSetting(key: MacProductSettingKeys.defaultPromptTemplate) }
    }

}

private struct MacPromptTemplatePersistence: PromptTemplatePersistence {
    let store: MacApplicationPort

    func load() throws -> PromptTemplateSnapshot? {
        guard let setting = try store.productSetting(key: MacProductSettingKeys.promptTemplates) else { return nil }
        return try JSONDecoder().decode(PromptTemplateSnapshot.self, from: Data(setting.valueJSON.utf8))
    }

    func save(_ snapshot: PromptTemplateSnapshot) throws {
        let data = try JSONEncoder().encode(snapshot)
        guard let valueJSON = String(data: data, encoding: .utf8) else { throw PromptTemplateReadError.unavailable }
        _ = try store.putProductSetting(key: MacProductSettingKeys.promptTemplates, valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
    }
}
