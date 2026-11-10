// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PromptTemplateService logic in the iOS service layer.
// - Interfaces:
//   - PromptTemplate
//   - PromptVariable
//   - PromptTemplateService
// - APIs:
//   - none
//
//
//  PromptTemplateService.swift
//  Nola
//
//  Custom prompt template management for event analysis.
//

import Foundation
import Combine
import CryptoKit

/// Prompt template with variable substitution
nonisolated public struct PromptTemplate: Identifiable, Codable, Hashable, Sendable {
    public var id: UUID
    public var name: String
    /// BCP-47 language tag, e.g. "en", "zh-Hans". Nil means language-agnostic.
    public var language: String?
    public var content: String
    public var description: String
    public var isBuiltIn: Bool
    public var createdAt: Date
    public var updatedAt: Date
    /// Stable resource owner and versioned grants.
    public var accessOwnerID: String?
    public var accessRevision: UInt64?
    public var accessGrants: [AgentCoreResourceGrant]?
    private var presentOptionalFields = Set<String>()
    
    public init(
        id: UUID = UUID(),
        name: String,
        language: String? = nil,
        content: String,
        description: String,
        isBuiltIn: Bool = false,
        createdAt: Date = Date(),
        updatedAt: Date = Date(),
        accessOwnerID: String? = nil,
        accessRevision: UInt64? = nil,
        accessGrants: [AgentCoreResourceGrant]? = nil
    ) {
        self.id = id
        self.name = name
        self.language = language
        self.content = content
        self.description = description
        self.isBuiltIn = isBuiltIn
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.accessOwnerID = accessOwnerID
        self.accessRevision = accessRevision
        self.accessGrants = accessGrants
    }
    private enum CodingKeys: String, CodingKey, CaseIterable {
        case id, name, language, content, description, isBuiltIn, createdAt, updatedAt
        case accessOwnerID, accessRevision, accessGrants
    }
    public init(from decoder: Decoder) throws {
        let present = try AgentCoreResourceStrictDecoding.fields(decoder, allowed: Set(CodingKeys.allCases.map(\.rawValue)))
        let values = try decoder.container(keyedBy: CodingKeys.self)
        self.init(id: try values.decode(UUID.self, forKey: .id), name: try values.decode(String.self, forKey: .name),
            language: try values.decodeIfPresent(String.self, forKey: .language), content: try values.decode(String.self, forKey: .content),
            description: try values.decode(String.self, forKey: .description), isBuiltIn: try values.decode(Bool.self, forKey: .isBuiltIn),
            createdAt: try values.decode(Date.self, forKey: .createdAt), updatedAt: try values.decode(Date.self, forKey: .updatedAt),
            accessOwnerID: try values.decodeIfPresent(String.self, forKey: .accessOwnerID),
            accessRevision: try values.decodeIfPresent(UInt64.self, forKey: .accessRevision),
            accessGrants: try values.decodeIfPresent([AgentCoreResourceGrant].self, forKey: .accessGrants))
        presentOptionalFields = present.intersection(["language", "accessOwnerID", "accessRevision", "accessGrants"])
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(id, forKey: .id); try values.encode(name, forKey: .name)
        try values.encode(content, forKey: .content); try values.encode(description, forKey: .description)
        try values.encode(isBuiltIn, forKey: .isBuiltIn); try values.encode(createdAt, forKey: .createdAt)
        try values.encode(updatedAt, forKey: .updatedAt)
        try AgentCoreResourceStrictDecoding.optional(language, .language, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(accessOwnerID, .accessOwnerID, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(accessRevision, .accessRevision, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(accessGrants, .accessGrants, into: &values, present: presentOptionalFields)
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.id == rhs.id && lhs.name == rhs.name && lhs.language == rhs.language && lhs.content == rhs.content &&
        lhs.description == rhs.description && lhs.isBuiltIn == rhs.isBuiltIn && lhs.createdAt == rhs.createdAt &&
        lhs.updatedAt == rhs.updatedAt && lhs.accessOwnerID == rhs.accessOwnerID &&
        lhs.accessRevision == rhs.accessRevision && lhs.accessGrants == rhs.accessGrants
    }
    public func hash(into hasher: inout Hasher) {
        hasher.combine(id); hasher.combine(name); hasher.combine(language); hasher.combine(content); hasher.combine(description)
        hasher.combine(isBuiltIn); hasher.combine(createdAt); hasher.combine(updatedAt); hasher.combine(accessOwnerID)
        hasher.combine(accessRevision); hasher.combine(accessGrants)
    }

}

/// Prompt template variables
public enum PromptVariable: String, CaseIterable, Sendable {
    case dateRange = "{{date_range}}"
    case events = "{{events}}"
    case topN = "{{top_n}}"
    case userPrefs = "{{user_prefs}}"
    case context = "{{context}}"
    
    public var description: String {
        switch self {
        case .dateRange: return NSLocalizedString("autogen.PromptTemplateService.bee8164c", comment: "AUTOGEN")
        case .events: return NSLocalizedString("autogen.PromptTemplateService.f29b6953", comment: "AUTOGEN")
        case .topN: return NSLocalizedString("autogen.PromptTemplateService.7864544e", comment: "AUTOGEN")
        case .userPrefs: return NSLocalizedString("autogen.PromptTemplateService.ccea14b5", comment: "AUTOGEN")
        case .context: return NSLocalizedString("autogen.PromptTemplateService.ce9829e6", comment: "AUTOGEN")
        }
    }
}

nonisolated public struct PromptTemplateSnapshot: Codable, Equatable, Sendable {
    public var customTemplates: [PromptTemplate]
    public var selectedTemplateID: UUID?
    private var presentOptionalFields = Set<String>()

    public init(customTemplates: [PromptTemplate], selectedTemplateID: UUID?) {
        self.customTemplates = customTemplates
        self.selectedTemplateID = selectedTemplateID
    }
    private enum CodingKeys: String, CodingKey { case customTemplates, selectedTemplateID }
    public init(from decoder: Decoder) throws {
        let fields = try AgentCoreResourceStrictDecoding.fields(decoder, allowed: ["customTemplates", "selectedTemplateID"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        self.init(customTemplates: try values.decode([PromptTemplate].self, forKey: .customTemplates),
            selectedTemplateID: try values.decodeIfPresent(UUID.self, forKey: .selectedTemplateID))
        presentOptionalFields = fields.intersection(["selectedTemplateID"])
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(customTemplates, forKey: .customTemplates)
        try AgentCoreResourceStrictDecoding.optional(selectedTemplateID, .selectedTemplateID, into: &values, present: presentOptionalFields)
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.customTemplates == rhs.customTemplates && lhs.selectedTemplateID == rhs.selectedTemplateID
    }

}

nonisolated public enum PromptTemplateReadError: Error, Equatable, LocalizedError {
    case missing(UUID)
    case denied(UUID)
    case invalidIdentity(UUID)
    case invalidDefinition(UUID)
    case ambiguousCatalog
    case unavailable

    public var errorDescription: String? {
        switch self {
        case .missing: NSLocalizedString("prompt.templates.error.missing", value: "The selected template is unavailable. Choose an available template.", comment: "Missing template")
        case .denied: NSLocalizedString("prompt.templates.error.denied", value: "You do not have permission to read this template.", comment: "Template access denied")
        case .invalidIdentity: NSLocalizedString("prompt.templates.error.identity", value: "This template has no valid access identity.", comment: "Invalid template ownership")
        case .invalidDefinition: NSLocalizedString("prompt.templates.error.definition", value: "This template cannot be used as an instruction.", comment: "Invalid template definition")
        case .ambiguousCatalog: NSLocalizedString("prompt.templates.error.catalog", value: "The template catalog contains conflicting identities.", comment: "Invalid template catalog")
        case .unavailable: NSLocalizedString("prompt.templates.error.unavailable", value: "Prompt templates could not be loaded. No template was substituted.", comment: "Template persistence unavailable")
        }
    }
}

nonisolated public struct PromptTemplateMetadata: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let name: String
    public let language: String?
    public let description: String
    public let isBuiltIn: Bool
}

public protocol PromptTemplatePersistence: Sendable {
    func load() throws -> PromptTemplateSnapshot?
    func save(_ snapshot: PromptTemplateSnapshot) throws
}

/// Shared selection/access policy for UI reads and trusted native providers.
/// It chooses an ID; Core still rechecks current resource access at capture.
nonisolated enum PromptTemplateResolution {
    static func allowed(_ template: PromptTemplate, permission: AgentCoreResourcePermission,
                        builtins: [PromptTemplate]) -> Bool {
        if let builtin = builtins.first(where: { $0.id == template.id }) {
            guard template.isBuiltIn, template.content == builtin.content, template.language == builtin.language else { return false }
            return permission == .readContent || permission == .readMetadata
        }
        guard !template.isBuiltIn, let ownerID = template.accessOwnerID,
              !ownerID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, ownerID.utf8.count <= 512,
              !ownerID.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
              let revision = template.accessRevision, revision > 0 else { return false }
        let digest = SHA256.hash(data: Data(template.content.utf8)).map { String(format: "%02x", $0) }.joined()
        return AgentCoreResourceAccess.evaluate(resource: .init(kind: .promptTemplate,
            resourceID: template.id.uuidString.lowercased(), owner: .init(kind: "local_user", id: ownerID),
            revision: revision, contentDigest: "sha256:\(digest)"), permission: permission,
            grants: template.accessGrants ?? [], actor: AgentCoreResourceAccess.localPrincipal())?.allowed == true
    }
    static func defaultTemplate(language: String, catalog: [PromptTemplate], builtins: [PromptTemplate]) throws -> PromptTemplate? {
        guard Set(catalog.map(\.id)).count == catalog.count else { throw PromptTemplateReadError.ambiguousCatalog }
        func tag(_ value: String) -> String { value.replacingOccurrences(of: "_", with: "-").lowercased() }
        let language = tag(language)
        if let exact = catalog.first(where: { $0.language.map(tag) == language && allowed($0, permission: .readContent, builtins: builtins) }) {
            return exact
        }
        if let prefix = catalog.first(where: { $0.language.map { tag($0).hasPrefix(language + "-") } == true && allowed($0, permission: .readContent, builtins: builtins) }) {
            return prefix
        }
        return builtins.first(where: { $0.language == "en" })
    }
}

/// The current catalog is read without normalizing existing ownership facts.
public final class PromptTemplateService: ObservableObject {
    @Published public private(set) var templates: [PromptTemplate] = []
    @Published public private(set) var selectedTemplateID: UUID?
    @Published public private(set) var catalogLoadError: String?
    @Published public private(set) var builtInLanguage = "zh-hans"
    private var catalogFailure: PromptTemplateReadError?
    private var persistedSnapshot: PromptTemplateSnapshot?
    private let logger: Logger
    private let persistence: (any PromptTemplatePersistence)?
    private static let builtIns = BuiltInPromptTemplates.templates()

    public init(logger: Logger, persistence: (any PromptTemplatePersistence)? = nil) {
        self.logger = logger
        self.persistence = persistence
        templates = Self.builtIns
        do { try reloadFromPersistence() }
        catch { logger.log(.error, "PromptTemplate", "Canonical template catalog could not be loaded") }
    }

    /// Only a view filter. All 18 definitions remain available to exact-ID providers.
    public var displayedTemplates: [PromptTemplate] {
        templates.filter {
            !$0.isBuiltIn || $0.id == selectedTemplateID ||
                $0.language.map(Self.normalizedLanguageTag) == builtInLanguage
        }
    }

    private func commit(_ next: [PromptTemplate], selectedID: UUID?) throws {
        if let catalogFailure { throw catalogFailure }
        guard Set(next.map(\.id)).count == next.count else { throw PromptTemplateReadError.ambiguousCatalog }
        var snapshot = persistedSnapshot ?? PromptTemplateSnapshot(customTemplates: [], selectedTemplateID: nil)
        snapshot.customTemplates = next.filter { !$0.isBuiltIn }
        snapshot.selectedTemplateID = selectedID
        try persistence?.save(snapshot)
        persistedSnapshot = snapshot
        templates = next
        selectedTemplateID = selectedID
    }

    /// Explicit creation/copy is the only operation that assigns a new owner.
    private func newlyOwnedTemplate(_ template: PromptTemplate) -> PromptTemplate {
        var value = template
        value.accessOwnerID = AgentCoreResourceAccess.localPrincipal().id
        value.accessRevision = 1
        value.accessGrants = []
        return value
    }

    public func addTemplate(_ template: PromptTemplate) throws {
        if persistence != nil { try reloadFromPersistence() }
        guard !template.isBuiltIn, !Self.builtIns.contains(where: { $0.id == template.id }),
              !templates.contains(where: { $0.id == template.id }) else { throw PromptTemplateReadError.ambiguousCatalog }
        try commit(templates + [newlyOwnedTemplate(template)], selectedID: selectedTemplateID)
    }

    public func updateTemplate(_ template: PromptTemplate) throws {
        if persistence != nil { try reloadFromPersistence() }
        guard let index = templates.firstIndex(where: { $0.id == template.id }) else { throw PromptTemplateReadError.missing(template.id) }
        let previous = templates[index]
        guard !Self.builtIns.contains(where: { $0.id == previous.id }), !previous.isBuiltIn,
              canAccess(previous, permission: .editContent) else { throw PromptTemplateReadError.denied(template.id) }
        guard let owner = previous.accessOwnerID, let revision = previous.accessRevision,
              revision > 0, revision < UInt64.max else { throw PromptTemplateReadError.invalidIdentity(template.id) }
        var updated = previous
        updated.name = template.name
        updated.language = template.language
        updated.content = template.content
        updated.description = template.description
        updated.isBuiltIn = false
        updated.updatedAt = Date()
        updated.accessOwnerID = owner
        updated.accessRevision = revision + 1
        updated.accessGrants = previous.accessGrants
        var next = templates; next[index] = updated
        try commit(next, selectedID: selectedTemplateID)
    }

    public func deleteTemplate(id: UUID) throws {
        if persistence != nil { try reloadFromPersistence() }
        guard let template = templates.first(where: { $0.id == id }) else { throw PromptTemplateReadError.missing(id) }
        guard !Self.builtIns.contains(where: { $0.id == id }), !template.isBuiltIn,
              canAccess(template, permission: .delete) else { throw PromptTemplateReadError.denied(id) }
        // An explicit selection remains explicit; deleting its resource does not
        // silently choose a different instruction for subsequent execution.
        try commit(templates.filter { $0.id != id }, selectedID: selectedTemplateID)
    }

    public func duplicateTemplate(id: UUID) throws -> PromptTemplate {
        if persistence != nil { try reloadFromPersistence() }
        let original = try requireReadableTemplate(id: id)
        let duplicate = newlyOwnedTemplate(PromptTemplate(name: "\(original.name) (副本)", language: original.language,
            content: original.content, description: original.description))
        try commit(templates + [duplicate], selectedID: selectedTemplateID)
        return duplicate
    }

    private func currentCatalog() throws -> PromptTemplateSnapshot {
        guard let persistence else {
            if let catalogFailure { throw catalogFailure }
            return .init(customTemplates: templates.filter { !$0.isBuiltIn }, selectedTemplateID: selectedTemplateID)
        }
        do {
            let snapshot = try persistence.load() ?? .init(customTemplates: [], selectedTemplateID: nil)
            try validateCatalog(snapshot)
            return snapshot
        } catch let error as PromptTemplateReadError { throw error }
        catch { throw PromptTemplateReadError.unavailable }
    }

    private func validateCatalog(_ snapshot: PromptTemplateSnapshot) throws {
        let all = Self.builtIns + snapshot.customTemplates
        guard snapshot.customTemplates.allSatisfy({ !$0.isBuiltIn }), Set(all.map(\.id)).count == all.count else {
            throw PromptTemplateReadError.ambiguousCatalog
        }
    }

    private func readTemplate(id: UUID, in catalog: [PromptTemplate]) throws -> PromptTemplate {
        let matches = catalog.filter { $0.id == id }
        guard matches.count == 1 else {
            if matches.isEmpty { throw PromptTemplateReadError.missing(id) }
            throw PromptTemplateReadError.ambiguousCatalog
        }
        let template = matches[0]
        guard canAccess(template, permission: .readContent, catalog: catalog) else { throw PromptTemplateReadError.denied(id) }
        return template
    }

    public func requireReadableTemplate(id: UUID) throws -> PromptTemplate {
        let catalog = try currentCatalog()
        return try readTemplate(id: id, in: Self.builtIns + catalog.customTemplates)
    }

    private func checkedInstruction(_ template: PromptTemplate) throws -> PromptTemplate {
        guard !template.content.isEmpty, template.content.utf8.count <= 262_144,
              !template.content.contains("\0") else { throw PromptTemplateReadError.invalidDefinition(template.id) }
        return template
    }

    /// Fresh body-bearing read for execution. Explicit IDs never fall back.
    public func instructionTemplate(id: UUID) throws -> PromptTemplate {
        try checkedInstruction(requireReadableTemplate(id: id))
    }

    public func getCurrentTemplate() throws -> PromptTemplate? {
        let snapshot = try currentCatalog(), catalog = Self.builtIns + snapshot.customTemplates
        if let selected = snapshot.selectedTemplateID { return try checkedInstruction(readTemplate(id: selected, in: catalog)) }
        return try defaultTemplate(for: builtInLanguage, catalog: catalog)
    }

    public func templateMetadata(id: UUID) -> PromptTemplateMetadata? {
        guard let template = templates.first(where: { $0.id == id }), canAccess(template, permission: .readMetadata) else { return nil }
        return .init(id: template.id, name: template.name, language: template.language,
            description: template.description, isBuiltIn: template.isBuiltIn)
    }

    /// Optional display probe. Execution callers use the throwing instruction resolver.
    public func readableTemplate(id: UUID) -> PromptTemplate? { try? requireReadableTemplate(id: id) }

    public func getTemplate(for language: String?) throws -> PromptTemplate? {
        let snapshot = try currentCatalog(), catalog = Self.builtIns + snapshot.customTemplates
        if let selected = snapshot.selectedTemplateID { return try checkedInstruction(readTemplate(id: selected, in: catalog)) }
        return try defaultTemplate(for: language ?? builtInLanguage, catalog: catalog)
    }

    private func defaultTemplate(for language: String, catalog: [PromptTemplate]) throws -> PromptTemplate? {
        guard let selected = try PromptTemplateResolution.defaultTemplate(language: language, catalog: catalog, builtins: Self.builtIns) else { return nil }
        return try checkedInstruction(readTemplate(id: selected.id, in: catalog))
    }

    public func selectTemplate(id: UUID) throws {
        if persistence != nil { try reloadFromPersistence() }
        _ = try requireReadableTemplate(id: id)
        try commit(templates, selectedID: id)
    }

    // MARK: - Prompt Generation
    
    /// Build prompt from template with variable substitution
    #if os(iOS)
    public func buildPrompt(
        dateRange: DateInterval,
        events: [ProcessedEvent],
        topN: Int = 5,
        userPrefs: [String: Any] = [:],
        context: String? = nil,
        language: String? = nil
    ) throws -> String {
        guard let template = try getTemplate(for: language) else { throw PromptTemplateReadError.unavailable }
        
        var prompt = template.content
        
        // Substitute variables
        let dateFormatter = DateFormatter()
        dateFormatter.dateStyle = .short
        let dateRangeStr = "\(dateFormatter.string(from: dateRange.start)) 到 \(dateFormatter.string(from: dateRange.end))"
        prompt = prompt.replacingOccurrences(of: PromptVariable.dateRange.rawValue, with: dateRangeStr)
        
        // Convert events to JSON
        let eventsJSON = serializeEvents(events)
        prompt = prompt.replacingOccurrences(of: PromptVariable.events.rawValue, with: eventsJSON)
        
        // Top N
        prompt = prompt.replacingOccurrences(of: PromptVariable.topN.rawValue, with: "\(topN)")
        
        // User prefs (simplified)
        let prefsJSON = (try? JSONSerialization.data(withJSONObject: userPrefs, options: .prettyPrinted))
            .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        prompt = prompt.replacingOccurrences(of: PromptVariable.userPrefs.rawValue, with: prefsJSON)
        
        // Context
        prompt = prompt.replacingOccurrences(of: PromptVariable.context.rawValue, with: context ?? "无")
        
        logger.log(.debug, "PromptTemplate", NSLocalizedString("autogen.PromptTemplateService.ccb412b9", comment: "AUTOGEN"), metadata: ["template": template.name])
        
        return prompt
    }
    #endif
    
    /// Validate template syntax
    public func validateTemplate(_ template: PromptTemplate) -> [String] {
        var warnings: [String] = []
        
        // Check for unknown variables
        let pattern = "\\{\\{[^}]+\\}\\}"
        if let regex = try? NSRegularExpression(pattern: pattern) {
            let matches = regex.matches(in: template.content, range: NSRange(template.content.startIndex..., in: template.content))
            
            for match in matches {
                if let range = Range(match.range, in: template.content) {
                    let variable = String(template.content[range])
                    let isValid = PromptVariable.allCases.contains { $0.rawValue == variable }
                    if !isValid {
                        warnings.append("未知变量：\(variable)")
                    }
                }
            }
        }
        
        // Check for basic structure
        if template.content.isEmpty {
            warnings.append(NSLocalizedString("autogen.PromptTemplateService.db4dd68f", comment: "AUTOGEN"))
        }
        
        if template.content.count > 10000 {
            warnings.append(NSLocalizedString("autogen.PromptTemplateService.5a6d1c7f", comment: "AUTOGEN"))
        }
        
        return warnings
    }
    
    /// Estimate token count (approximate)
    public func estimateTokenCount(_ text: String) -> Int {
        let length = text.count
        let chineseCharCount = text.filter { "\u{4E00}"..."\u{9FFF}" ~= $0 }.count
        
        let chineseEstimate = Double(chineseCharCount) * 1.0
        
        let nonChineseEstimate = Double(length - chineseCharCount) / 4.7
        
        let total = Int(chineseEstimate + nonChineseEstimate) + 1
        return max(1, total)
    }
    
    /// Export template to JSON
    public func exportTemplate(_ template: PromptTemplate) throws -> Data {
        let template = try requireReadableTemplate(id: template.id)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(template)
    }
    
    /// Import template from JSON
    public func importTemplate(from data: Data) throws -> PromptTemplate {
        let decoder = JSONDecoder()
        var template = try decoder.decode(PromptTemplate.self, from: data)
        template.id = UUID() // Generate new ID
        template.isBuiltIn = false
        template.createdAt = Date()
        template.updatedAt = Date()
        template.accessOwnerID = nil
        template.accessRevision = nil
        template.accessGrants = nil
        return template
    }

    /// Explicit replacement preserves every supplied authority field. It is
    /// never used to claim ownership while loading an existing catalog.
    public func replaceCustomTemplates(_ customTemplates: [PromptTemplate], selectedID: UUID? = nil) throws {
        if persistence != nil { try reloadFromPersistence() }
        guard customTemplates.allSatisfy({ !$0.isBuiltIn }),
              templates.filter({ !$0.isBuiltIn }).allSatisfy({ canAccess($0, permission: .delete) }) else {
            throw PromptTemplateReadError.ambiguousCatalog
        }
        try commit(Self.builtIns + customTemplates, selectedID: selectedID)
    }

    public func reloadFromPersistence() throws {
        do {
            let snapshot = try persistence?.load()
            let custom = snapshot?.customTemplates ?? []
            let next = Self.builtIns + custom
            guard custom.allSatisfy({ !$0.isBuiltIn }), Set(next.map(\.id)).count == next.count else {
                throw PromptTemplateReadError.ambiguousCatalog
            }
            templates = next
            persistedSnapshot = snapshot
            selectedTemplateID = snapshot?.selectedTemplateID
            catalogFailure = nil
            catalogLoadError = nil
        } catch {
            let failure = (error as? PromptTemplateReadError) ?? .unavailable
            catalogFailure = failure
            catalogLoadError = failure.localizedDescription
            throw failure
        }
    }

    public func selectBuiltInLanguage(preferredLanguages: [String]) {
        let available = Array(Set(Self.builtIns.compactMap { $0.language.map(Self.normalizedLanguageTag) })).sorted()
        let preferred = preferredLanguages.map(Self.normalizedLanguageTag)
        builtInLanguage = preferred.lazy.compactMap { language in
            if available.contains(language) { return language }
            if let match = available.first(where: { language.hasPrefix($0 + "-") || $0.hasPrefix(language + "-") }) { return match }
            if ["zh-cn", "zh-sg"].contains(language), available.contains("zh-hans") { return "zh-hans" }
            let base = language.split(separator: "-").first.map(String.init) ?? language
            return available.first(where: { $0 == base })
        }.first ?? "en"
    }

    nonisolated private static func normalizedLanguageTag(_ value: String) -> String {
        value.replacingOccurrences(of: "_", with: "-").lowercased()
    }

    public func canReadTemplate(_ template: PromptTemplate) -> Bool { canAccess(template, permission: .readContent) }
    public func canEditTemplate(_ template: PromptTemplate) -> Bool {
        !Self.builtIns.contains(where: { $0.id == template.id }) && canAccess(template, permission: .editContent)
    }
    public func canDeleteTemplate(_ template: PromptTemplate) -> Bool {
        !Self.builtIns.contains(where: { $0.id == template.id }) && canAccess(template, permission: .delete)
    }

    private func canAccess(_ supplied: PromptTemplate, permission: AgentCoreResourcePermission,
        catalog: [PromptTemplate]? = nil) -> Bool {
        if catalog == nil, catalogFailure != nil { return false }
        let matching = (catalog ?? templates).filter { $0.id == supplied.id }
        guard matching.count == 1 else { return false }
        return PromptTemplateResolution.allowed(matching[0], permission: permission, builtins: Self.builtIns)
    }

    #if os(iOS)
    private func serializeEvents(_ events: [ProcessedEvent]) -> String {
        let eventDicts = events.prefix(20).map { event -> [String: Any] in
            [
                "type": event.source.rawValue,
                "title": event.title,
                "date": event.date?.timeIntervalSince1970 ?? 0,
                "priority": event.priority,
                "metadata": event.metadata
            ]
        }
        
        if let jsonData = try? JSONSerialization.data(withJSONObject: eventDicts, options: .prettyPrinted),
           let jsonString = String(data: jsonData, encoding: .utf8) {
            return jsonString
        }
        
        return "[]"
    }
    #endif
}
