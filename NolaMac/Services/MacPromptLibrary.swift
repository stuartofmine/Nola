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
        guard let setting = try MacApplicationPort.shared.productSetting(key: MacProductSettingKeys.defaultPromptTemplate) else { return nil }
        guard let id = UUID(uuidString: setting.valueJSON) else { throw PromptTemplateReadError.unavailable }
        _ = try service.requireReadableTemplate(id: id)
        return id
    }

    static func setDefaultTemplateID(_ id: UUID?) throws {
        if let id {
            _ = try service.requireReadableTemplate(id: id)
            _ = try MacApplicationPort.shared.putProductSetting(key: MacProductSettingKeys.defaultPromptTemplate,
                valueJSON: id.uuidString, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
        } else { try MacApplicationPort.shared.deleteProductSetting(key: MacProductSettingKeys.defaultPromptTemplate) }
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
