// SPDX-License-Identifier: Apache-2.0
// Core owns role contents and capability/tool admission; clients only present this versioned catalog.
import Foundation

nonisolated struct AppleExpertRoleTemplateCatalog: Codable, Equatable, Sendable {
    struct Features: Codable, Equatable, Sendable {
        let reasoningEffort: Bool
        let explicitGenerationReasoning: Bool
        let rejoin: Bool
        let mainAgentDelegation: Bool
        init(reasoningEffort: Bool = false, explicitGenerationReasoning: Bool = false, rejoin: Bool = false, mainAgentDelegation: Bool = false) {
            self.reasoningEffort = reasoningEffort
            self.explicitGenerationReasoning = explicitGenerationReasoning
            self.rejoin = rejoin
            self.mainAgentDelegation = mainAgentDelegation
        }
        enum CodingKeys: String, CodingKey { case reasoningEffort = "reasoning_effort", explicitGenerationReasoning = "explicit_generation_reasoning", rejoin; case mainAgentDelegation = "main_agent_delegation" }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            reasoningEffort = try values.decodeIfPresent(Bool.self, forKey: .reasoningEffort) ?? false
            explicitGenerationReasoning = try values.decodeIfPresent(Bool.self, forKey: .explicitGenerationReasoning) ?? false
            rejoin = try values.decodeIfPresent(Bool.self, forKey: .rejoin) ?? false
            mainAgentDelegation = try values.decodeIfPresent(Bool.self, forKey: .mainAgentDelegation) ?? false
        }
    }
    let version: Int
    let features: Features
    let templates: [AppleExpertRoleTemplate]

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        version = try values.decode(Int.self, forKey: .version)
        features = try values.decodeIfPresent(Features.self, forKey: .features) ?? Features()
        templates = try values.decode([AppleExpertRoleTemplate].self, forKey: .templates)
        guard version == 1, Set(templates.compactMap { UUID(uuidString: $0.id) }).count == templates.count else {
            throw DecodingError.dataCorruptedError(forKey: .version, in: values,
                debugDescription: "Unsupported or ambiguous expert role catalog")
        }
    }
}

nonisolated struct AppleExpertRoleTemplate: Codable, Equatable, Identifiable, Sendable {
    struct ToolPack: Codable, Equatable, Sendable {
        let id: String
        let tools: [String]
    }
    struct Skill: Codable, Equatable, Identifiable, Sendable {
        let id: String
        let name: [String: String]
        let instructions: String
        func localizedName(locale: Locale = .current) -> String { AppleExpertRoleTemplate.localized(name, locale: locale) }
    }
    let id: String
    let roleID: String
    let name: [String: String]
    let description: [String: String]
    let prompt: String
    let toolPack: ToolPack
    let skills: [Skill]
    enum CodingKeys: String, CodingKey {
        case id, roleID = "role_id", name, description, prompt, toolPack = "tool_pack", skills
    }
    func localizedName(locale: Locale = .current) -> String { Self.localized(name, locale: locale) }
    func localizedDescription(locale: Locale = .current) -> String { Self.localized(description, locale: locale) }

    fileprivate static func localized(_ values: [String: String], locale: Locale) -> String {
        let preferred = locale.identifier.replacingOccurrences(of: "_", with: "-")
        if let exact = values[preferred] { return exact }
        let language = preferred.split(separator: "-").first.map(String.init) ?? preferred
        if let direct = values[language] { return direct }
        if let key = values.keys.sorted().first(where: { $0.split(separator: "-").first.map(String.init) == language }) {
            return values[key] ?? ""
        }
        return values["en"] ?? values.sorted(by: { $0.key < $1.key }).first?.value ?? ""
    }
}
