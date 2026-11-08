// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Registers Mac-only built-in Skills and exposes them to the local Agent Host safely.

import Foundation
import CryptoKit

nonisolated enum MacSkillFunctionalCategory: String, CaseIterable, Sendable {
    case documents
    case dataAnalysis = "data-analysis"
    case designPresentations = "design-presentations"
    case communication
    case planningAutomation = "planning-automation"
    case knowledgeResearch = "knowledge-research"
    case workspace
    case custom

    var localizedTitle: String {
        switch self {
        case .documents: return String(localized: "Documents")
        case .dataAnalysis: return String(localized: "Data & Analysis")
        case .designPresentations: return String(localized: "Design & Presentations")
        case .communication: return String(localized: "Communication")
        case .planningAutomation: return String(localized: "Planning & Automation")
        case .knowledgeResearch: return String(localized: "Knowledge & Research")
        case .workspace: return String(localized: "Workspace")
        case .custom: return String(localized: "Custom Skills")
        }
    }

    var symbol: String {
        switch self {
        case .documents: return "doc.text"
        case .dataAnalysis: return "chart.xyaxis.line"
        case .designPresentations: return "paintbrush"
        case .communication: return "bubble.left.and.bubble.right"
        case .planningAutomation: return "calendar.badge.clock"
        case .knowledgeResearch: return "books.vertical"
        case .workspace: return "folder"
        case .custom: return "shippingbox"
        }
    }
}

nonisolated enum MacBuiltInSkillID: String, CaseIterable, Codable, Sendable {
    case word = "word-document-authoring"
    case excel = "excel-workbook-authoring"
    case powerpoint = "powerpoint-presentation-authoring"
    case pdf = "work-with-pdf"
    case figma = "collaborate-in-figma"
    case dataAnalysis = "analyze-data"
    case deepResearch = "conduct-deep-research"
    case documentReview = "review-documents"
    case imageOCR = "extract-image-text"
    case meetings = "assist-meetings"
    case scheduleTasks = "plan-schedule-tasks"
    case knowledgeMemory = "curate-knowledge-memory"
    case fileOrganization = "organize-files"
    case automations = "design-automations"
    case email = "assist-email"
    case academicReader = "nature-reader"
    case scientificWriting = "nature-writing"
    case academicPolishing = "nature-polishing"
    case manuscriptReviewer = "nature-reviewer"
    case reviewerResponse = "nature-response"
    case statisticsAudit = "nature-statistics"
    case referenceVerifier = "nature-ref-verifier"
    case paperToPresentation = "nature-paper2ppt"
    case openSourceModel = "install-open-source-model"

    var resourceDirectory: String {
        switch self {
        case .word: return "word"
        case .excel: return "excel"
        case .powerpoint: return "powerpoint"
        case .pdf, .figma, .dataAnalysis, .deepResearch, .documentReview, .imageOCR,
             .meetings, .scheduleTasks, .knowledgeMemory, .fileOrganization,
             .automations, .email, .academicReader, .scientificWriting,
             .academicPolishing, .manuscriptReviewer, .reviewerResponse,
             .statisticsAudit, .referenceVerifier, .paperToPresentation:
            return rawValue
        case .openSourceModel:
            return rawValue
        }
    }

    var localizedTitle: String {
        switch self {
        case .word: return String(localized: "Create Word Document")
        case .excel: return String(localized: "Create Excel Workbook")
        case .powerpoint: return String(localized: "Create PowerPoint Presentation")
        case .pdf: return String(localized: "Work with PDF")
        case .figma: return String(localized: "Collaborate in Figma")
        case .dataAnalysis: return String(localized: "Analyze Data")
        case .deepResearch: return String(localized: "Conduct Deep Research")
        case .documentReview: return String(localized: "Review Documents")
        case .imageOCR: return String(localized: "Extract Text from Images")
        case .meetings: return String(localized: "Assist with Meetings")
        case .scheduleTasks: return String(localized: "Plan Schedule and Tasks")
        case .knowledgeMemory: return String(localized: "Curate Knowledge and Memory")
        case .fileOrganization: return String(localized: "Organize Files")
        case .automations: return String(localized: "Design Automations")
        case .email: return String(localized: "Assist with Email")
        case .academicReader: return String(localized: "Read Scientific Papers")
        case .scientificWriting: return String(localized: "Write Scientific Manuscripts")
        case .academicPolishing: return String(localized: "Polish Academic Writing")
        case .manuscriptReviewer: return String(localized: "Review Scientific Manuscripts")
        case .reviewerResponse: return String(localized: "Respond to Peer Review")
        case .statisticsAudit: return String(localized: "Audit Statistical Reporting")
        case .referenceVerifier: return String(localized: "Verify Academic References")
        case .paperToPresentation: return String(localized: "Turn Papers into Presentations")
        case .openSourceModel: return String(localized: "Install Open-Source Model")
        }
    }

    var localizedSummary: String {
        switch self {
        case .word:
            return String(localized: "Create polished, editable Word documents with structured headings, tables, and verified output.")
        case .excel:
            return String(localized: "Build editable Excel workbooks with formulas, formatting, charts, and validation.")
        case .powerpoint:
            return String(localized: "Design editable PowerPoint presentations with clear narrative, consistent layouts, and verified output.")
        case .pdf:
            return String(localized: "Read, transform, OCR, redact, and validate PDF files without compromising the original.")
        case .figma:
            return String(localized: "Inspect and improve Figma designs while preserving components, variables, and layout behavior.")
        case .dataAnalysis:
            return String(localized: "Clean, analyze, visualize, and explain datasets with reproducible methods and validation.")
        case .deepResearch:
            return String(localized: "Research complex questions with current primary sources, cross-checking, and precise citations.")
        case .documentReview:
            return String(localized: "Review and compare important documents with traceable findings, risks, and revisions.")
        case .imageOCR:
            return String(localized: "Extract and verify structured text, tables, and fields from images and scans.")
        case .meetings:
            return String(localized: "Turn meeting material into reliable summaries, decisions, actions, and follow-ups.")
        case .scheduleTasks:
            return String(localized: "Build conflict-aware plans and safely manage authorized calendars, tasks, and reminders.")
        case .knowledgeMemory:
            return String(localized: "Curate durable personal knowledge with consent, provenance, correction, and expiry controls.")
        case .fileOrganization:
            return String(localized: "Classify and organize workspace files through reversible plans and verified operations.")
        case .automations:
            return String(localized: "Design observable, permission-aware automations with safe testing and failure handling.")
        case .email:
            return String(localized: "Summarize threads and prepare grounded email drafts, sending only after confirmation.")
        case .academicReader:
            return String(localized: "Read complete scientific papers with bilingual, figure-aware, source-grounded output.")
        case .scientificWriting:
            return String(localized: "Plan and draft evidence-bounded scientific manuscripts and submission materials.")
        case .academicPolishing:
            return String(localized: "Polish academic prose without changing claims, evidence strength, or author intent.")
        case .manuscriptReviewer:
            return String(localized: "Run a traceable pre-submission review across novelty, rigor, evidence, and readability.")
        case .reviewerResponse:
            return String(localized: "Build point-by-point reviewer responses with action tracking and evidence boundaries.")
        case .statisticsAudit:
            return String(localized: "Audit statistical design and reporting for transparent, reproducible manuscript claims.")
        case .referenceVerifier:
            return String(localized: "Cross-check academic references and report field-level conflicts without inventing metadata.")
        case .paperToPresentation:
            return String(localized: "Transform papers into editable, evidence-linked presentations with slide-level QA.")
        case .openSourceModel:
            return String(localized: "Find a hardware-compatible model variant, then download, configure, and validate it safely.")
        }
    }

    var routingDescription: String {
        switch self {
        case .word: return "Create or edit Word documents (`.docx`)"
        case .excel: return "Create or edit Excel workbooks (`.xlsx`)"
        case .powerpoint: return "Create or edit PowerPoint presentations (`.pptx`)"
        case .pdf: return "Read, create, transform, OCR, redact, or validate PDF files"
        case .figma: return "Inspect, audit, prototype, or edit Figma designs and design systems"
        case .dataAnalysis: return "Clean, analyze, visualize, or explain structured datasets"
        case .deepResearch: return "Conduct current, cited, cross-source web or academic research"
        case .documentReview: return "Review, compare, critique, or redline important documents"
        case .imageOCR: return "Extract structured text, tables, or fields from images and scans"
        case .meetings: return "Prepare meetings or turn meeting material into decisions and actions"
        case .scheduleTasks: return "Plan or change calendars, tasks, reminders, and focus time"
        case .knowledgeMemory: return "Store, merge, correct, retrieve, or forget durable personal knowledge"
        case .fileOrganization: return "Classify, rename, move, deduplicate, or archive workspace files"
        case .automations: return "Design, test, enable, or explain scheduled and event-driven automations"
        case .email: return "Search, summarize, draft, reply to, or send authorized email"
        case .academicReader: return "Read or translate complete scientific papers with figures and source anchors"
        case .scientificWriting: return "Plan, draft, or restructure scientific manuscripts and submission materials"
        case .academicPolishing: return "Polish or translate academic prose without overstating the evidence"
        case .manuscriptReviewer: return "Simulate a rigorous, traceable pre-submission scientific review"
        case .reviewerResponse: return "Draft or audit point-by-point reviewer and editor responses"
        case .statisticsAudit: return "Audit manuscript statistics, reporting, sample units, and uncertainty"
        case .referenceVerifier: return "Verify academic references, DOI metadata, and citation consistency"
        case .paperToPresentation: return "Turn a scientific paper into an editable, source-grounded presentation"
        case .openSourceModel: return "Find, download, configure, and validate a hardware-compatible open-source model variant"
        }
    }

    var usesSharedResearchReferences: Bool {
        switch self {
        case .academicReader, .scientificWriting, .academicPolishing,
             .manuscriptReviewer, .reviewerResponse, .statisticsAudit,
             .referenceVerifier, .paperToPresentation:
            return true
        default:
            return false
        }
    }

    var functionalCategory: MacSkillFunctionalCategory {
        switch self {
        case .word, .pdf, .documentReview, .imageOCR:
            return .documents
        case .excel, .dataAnalysis, .statisticsAudit:
            return .dataAnalysis
        case .powerpoint, .figma, .paperToPresentation:
            return .designPresentations
        case .email, .meetings:
            return .communication
        case .scheduleTasks, .automations:
            return .planningAutomation
        case .fileOrganization:
            return .workspace
        case .openSourceModel:
            return .workspace
        case .deepResearch, .knowledgeMemory, .academicReader, .scientificWriting,
             .academicPolishing, .manuscriptReviewer, .reviewerResponse,
             .referenceVerifier:
            return .knowledgeResearch
        }
    }
}

nonisolated struct MacBuiltInSkillDescriptor: Identifiable, Hashable, Sendable {
    let id: MacBuiltInSkillID
    let title: String
    let summary: String
    let instructionURL: URL
}

nonisolated enum MacBuiltInSkillRegistry {
    private static let managedStart = "<!-- NOLA_MAC_BUILTIN_SKILLS_START -->"
    private static let managedEnd = "<!-- NOLA_MAC_BUILTIN_SKILLS_END -->"
    private struct HiddenSkillSetting: Codable {
        let schemaVersion: Int
        let ids: [String]
    }
    private static let installationLock = NSLock()

    static var builtInSkillOverridesRootURL: URL {
        MacAppEnvironment.shared.configDirectory
            .appendingPathComponent("built-in-skill-overrides", isDirectory: true)
    }

    static var resourceRootURL: URL? {
        if let bundled = Bundle.main.url(
            forResource: "NolaBuiltInSkills",
            withExtension: "bundle"
        ) {
            return bundled
        }

        let sourceRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let developmentBundle = sourceRoot
            .appendingPathComponent("Resources", isDirectory: true)
            .appendingPathComponent("NolaBuiltInSkills.bundle", isDirectory: true)
        return FileManager.default.fileExists(atPath: developmentBundle.path)
            ? developmentBundle
            : nil
    }

    static var descriptors: [MacBuiltInSkillDescriptor] {
        descriptors(overridesRootURL: builtInSkillOverridesRootURL)
    }

    static func descriptors(
        overridesRootURL: URL,
        resourceRootURL: URL? = MacBuiltInSkillRegistry.resourceRootURL
    ) -> [MacBuiltInSkillDescriptor] {
        guard let root = resourceRootURL else { return [] }
        return MacBuiltInSkillID.allCases.compactMap { id in
            let bundledURL = root
                .appendingPathComponent(id.resourceDirectory, isDirectory: true)
                .appendingPathComponent("SKILL.md", isDirectory: false)
            guard FileManager.default.fileExists(atPath: bundledURL.path) else { return nil }
            let overrideURL = overridesRootURL
                .appendingPathComponent(id.resourceDirectory, isDirectory: true)
                .appendingPathComponent("SKILL.md", isDirectory: false)
            let url = FileManager.default.fileExists(atPath: overrideURL.path)
                ? overrideURL
                : bundledURL
            return MacBuiltInSkillDescriptor(
                id: id,
                title: id.localizedTitle,
                summary: id.localizedSummary,
                instructionURL: url
            )
        }
    }

    static func instructionURL(
        for id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL
    ) -> URL? {
        descriptors(overridesRootURL: overridesRootURL)
            .first(where: { $0.id == id })?
            .instructionURL
    }

    static func hasOverride(
        for id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL
    ) -> Bool {
        let url = overridesRootURL
            .appendingPathComponent(id.resourceDirectory, isDirectory: true)
            .appendingPathComponent("SKILL.md", isDirectory: false)
        return FileManager.default.fileExists(atPath: url.path)
    }

    @discardableResult
    static func saveOverride(
        _ content: String,
        for id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL
    ) throws -> URL {
        let directory = overridesRootURL
            .appendingPathComponent(id.resourceDirectory, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        if let bundledDirectory = resourceRootURL?
            .appendingPathComponent(id.resourceDirectory, isDirectory: true),
           FileManager.default.fileExists(atPath: bundledDirectory.path) {
            let companionURLs = try FileManager.default.contentsOfDirectory(
                at: bundledDirectory,
                includingPropertiesForKeys: nil,
                options: [.skipsHiddenFiles]
            ).filter { $0.lastPathComponent != "SKILL.md" }
            for sourceURL in companionURLs {
                let destinationURL = directory.appendingPathComponent(
                    sourceURL.lastPathComponent,
                    isDirectory: sourceURL.hasDirectoryPath
                )
                if FileManager.default.fileExists(atPath: destinationURL.path) {
                    try FileManager.default.removeItem(at: destinationURL)
                }
                try FileManager.default.copyItem(at: sourceURL, to: destinationURL)
            }
        }
        let url = directory.appendingPathComponent("SKILL.md", isDirectory: false)
        try content.write(to: url, atomically: true, encoding: .utf8)
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: url.path
        )
        return url
    }

    static func resetOverride(
        for id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL
    ) throws {
        let directory = overridesRootURL
            .appendingPathComponent(id.resourceDirectory, isDirectory: true)
        if FileManager.default.fileExists(atPath: directory.path) {
            try FileManager.default.removeItem(at: directory)
        }
    }

    static func synchronizeCatalog(
        overridesRootURL: URL = builtInSkillOverridesRootURL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared,
        resourceRootURL: URL? = MacBuiltInSkillRegistry.resourceRootURL
    ) throws {
        _ = try canonicalRecords(
            overridesRootURL: overridesRootURL, store: store, resourceRootURL: resourceRootURL
        )
    }

    static func extensionItems(
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> [MacExtensionItem] {
        let records = Dictionary(uniqueKeysWithValues: try store.listSkills()
            .filter { $0.source == .builtIn }.map { ($0.id, $0) })
        let removed = try hiddenSkillIDs(store: store)
        return try MacBuiltInSkillID.allCases.compactMap { id in
            guard !removed.contains(id), let record = records[id.rawValue] else { return nil }
            guard let url = URL(string: record.instructionsRef), url.isFileURL else {
                throw CocoaError(.fileReadUnsupportedScheme)
            }
            return MacExtensionItem(
                id: "builtin-skill:\(id.rawValue)",
                kind: .skill,
                source: .builtIn,
                title: id.localizedTitle,
                detail: id.localizedSummary,
                path: url.path,
                isEnabled: record.enabled
            )
        }
    }

    static func skillID(forExtensionItemID itemID: String) -> MacBuiltInSkillID? {
        let prefix = "builtin-skill:"
        guard itemID.hasPrefix(prefix) else { return nil }
        return MacBuiltInSkillID(rawValue: String(itemID.dropFirst(prefix.count)))
    }

    static func setEnabled(
        _ enabled: Bool,
        for id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws {
        _ = try canonicalRecords(overridesRootURL: overridesRootURL, store: store)
        guard let expected = try store.skillEditor(id.rawValue) else {
            throw CocoaError(.fileReadNoSuchFile)
        }
        _ = try store.setSkillEnabled(
            expected,
            enabled: enabled,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }

    static func remove(
        _ id: MacBuiltInSkillID,
        overridesRootURL: URL = builtInSkillOverridesRootURL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws {
        _ = try canonicalRecords(overridesRootURL: overridesRootURL, store: store)
        guard let expected = try store.skillEditor(id.rawValue) else {
            throw CocoaError(.fileReadNoSuchFile)
        }
        let visibility = try store.productSetting(key: MacProductSettingKeys.hiddenBuiltInSkills)
        _ = try store.removeBuiltInSkill(
            expected, visibilityRevision: visibility?.revision,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }

    static func restoreBuiltInSkills(
        overridesRootURL: URL = builtInSkillOverridesRootURL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws {
        let records = try canonicalRecords(overridesRootURL: overridesRootURL, store: store)
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let visibility = try store.productSetting(key: MacProductSettingKeys.hiddenBuiltInSkills)
        let snapshots = try records.keys.sorted().map { id in
            guard let expected = try store.skillEditor(id) else {
                throw CocoaError(.fileReadNoSuchFile)
            }
            return expected
        }
        _ = try store.restoreBuiltInSkills(
            snapshots, visibilityRevision: visibility?.revision, updatedAtMs: now
        )
    }

    static func hasRemovedSkills(
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> Bool {
        try !hiddenSkillIDs(store: store).isEmpty
    }

    static func activeDescriptors(
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> [MacBuiltInSkillDescriptor] {
        try extensionItems(store: store).compactMap { item in
            guard item.isEnabled,
                  let id = skillID(forExtensionItemID: item.id),
                  let path = item.path else { return nil }
            return MacBuiltInSkillDescriptor(
                id: id, title: item.title, summary: item.detail,
                instructionURL: URL(fileURLWithPath: path)
            )
        }
    }

    static func allowedDirectoryArguments(
        projectURL: URL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> [String] {
        let activeSkills = try activeDescriptors(
            store: store
        )
        return allowedDirectoryArguments(projectURL: projectURL, activeSkills: activeSkills)
    }

    static func allowedDirectoryArguments(
        projectURL: URL,
        activeSkills: [MacBuiltInSkillDescriptor]
    ) -> [String] {
        var directories = [projectURL.standardizedFileURL]
        directories.append(contentsOf: activeSkills.map {
            $0.instructionURL.deletingLastPathComponent().standardizedFileURL
        })
        if activeSkills.contains(where: { $0.id.usesSharedResearchReferences }),
           let root = resourceRootURL {
            directories.append(
                root.appendingPathComponent("nature-shared", isDirectory: true).standardizedFileURL
            )
        }
        var seen = Set<String>()
        return directories
            .filter { seen.insert($0.path).inserted }
            .flatMap { ["--allow-dir", $0.path] }
    }

    /// Returns a capability-specific whitelist for the Agent Host. This must
    /// remain separate from readable directories: removing a Skill should
    /// revoke Skill loading even when its bundle remains filesystem-readable.
    static func skillDirectoryArguments(
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> [String] {
        let activeSkills = try activeDescriptors(
            store: store
        )
        return skillDirectoryArguments(activeSkills: activeSkills)
    }

    static func skillDirectoryArguments(activeSkills: [MacBuiltInSkillDescriptor]) -> [String] {
        var seen = Set<String>()
        let directories = activeSkills
            .map { $0.instructionURL.deletingLastPathComponent().standardizedFileURL }
            .filter { seen.insert($0.path).inserted }
        guard !directories.isEmpty else { return ["--no-skills"] }
        return directories.flatMap { ["--skill-dir", $0.path] }
    }

    static func installTrustedRouter() throws {
        try installTrustedRouter(
            configurationDirectory: MacAppEnvironment.shared.configDirectory,
            activeSkills: try activeDescriptors()
        )
    }

    static func installTrustedRouter(
        configurationDirectory: URL,
        activeSkills _: [MacBuiltInSkillDescriptor]
    ) throws {
        guard resourceRootURL != nil else { return }
        installationLock.lock()
        defer { installationLock.unlock() }

        try FileManager.default.createDirectory(
            at: configurationDirectory,
            withIntermediateDirectories: true
        )
        let targetURL = configurationDirectory.appendingPathComponent("AGENTS.md")
        let existing = (try? String(contentsOf: targetURL, encoding: .utf8)) ?? ""
        // Agent Host receives the frozen Skill roots directly through
        // `--skill-dir` and generates the model-visible metadata catalog from
        // that same snapshot. Remove the historical duplicate router block so
        // AGENTS.md remains solely user/project-owned guidance.
        let updated = removingManagedBlock(from: existing)
        guard updated != existing else { return }
        if updated.isEmpty {
            try FileManager.default.removeItem(at: targetURL)
        } else {
            try updated.write(to: targetURL, atomically: true, encoding: .utf8)
        }
    }

    /// Prepares every changed package before submitting one Application transaction.
    /// Query/startup separation is still required; no per-package writes belong here.
    private static func canonicalRecords(
        overridesRootURL: URL = builtInSkillOverridesRootURL,
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared,
        resourceRootURL: URL? = MacBuiltInSkillRegistry.resourceRootURL
    ) throws -> [String: MacCanonicalSkill] {
        _ = try hiddenSkillIDs(store: store)
        let packages = descriptors(overridesRootURL: overridesRootURL, resourceRootURL: resourceRootURL)
        let present = Set(packages.map(\.id))
        let missing = MacBuiltInSkillID.allCases.filter { !present.contains($0) }
        guard missing.isEmpty else {
            // A partial/missing bundle is an installation failure, not a new
            // catalog that implicitly removes resources or completes startup.
            throw CocoaError(.fileReadNoSuchFile, userInfo: [
                NSFilePathErrorKey: missing.map { id in
                    "\(resourceRootURL?.path ?? "NolaBuiltInSkills.bundle")/\(id.resourceDirectory)/SKILL.md"
                }.joined(separator: "\n")
            ])
        }
        var updates: [MacCanonicalSkillInput] = []
        for descriptor in packages {
            let data = try Data(contentsOf: descriptor.instructionURL)
            guard let text = String(data: data, encoding: .utf8) else {
                throw MacBuiltInSkillRegistryError.invalidInstructionEncoding(descriptor.id.rawValue)
            }
            let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            let instructionRef = descriptor.instructionURL.standardizedFileURL.absoluteString
            // A version also freezes its source reference. Relocating the app
            // registers a new version rather than rewriting an immutable one.
            let version = SHA256.hash(data: Data((digest + "\n" + instructionRef).utf8))
                .map { String(format: "%02x", $0) }.joined()
            let timestamp = Int64((try descriptor.instructionURL.resourceValues(
                forKeys: [.contentModificationDateKey]
            ).contentModificationDate?.timeIntervalSince1970 ?? 0) * 1_000)
            updates.append(MacCanonicalSkillInput(
                id: descriptor.id.rawValue,
                source: .builtIn,
                version: version,
                displayName: descriptor.id.rawValue,
                summary: descriptor.id.routingDescription,
                category: descriptor.id.functionalCategory.rawValue,
                instructionsRef: instructionRef,
                instructionsDigest: digest,
                instructionsText: text,
                requiredTools: [],
                defaultMode: .inline,
                // Activation and original creation time are Application-owned.
                // Never copy an earlier query over a concurrent user change.
                enabled: true,
                createdAtMs: timestamp,
                updatedAtMs: timestamp
            ))
        }
        let storedBuiltIns = try store.listSkills().filter { $0.source == .builtIn }
        let catalogIsCurrent = catalog(updates, matches: storedBuiltIns)
        if !updates.isEmpty, !catalogIsCurrent {
            _ = try store.synchronizeBuiltInSkills(updates)
        }
        let records = catalogIsCurrent
            ? storedBuiltIns
            : try store.listSkills().filter { $0.source == .builtIn }
        return Dictionary(
            uniqueKeysWithValues: records.map { ($0.id, $0) }
        )
    }

    /// Built-in package discovery runs at startup. Avoid submitting the same
    /// complete catalog as another durable editor command when Application
    /// already owns the exact package revisions. Enabled state and timestamps
    /// remain Application-owned and do not make an unchanged package stale.
    private static func catalog(
        _ inputs: [MacCanonicalSkillInput],
        matches records: [MacCanonicalSkill]
    ) -> Bool {
        guard inputs.count == records.count else { return false }
        let recordsByID = Dictionary(uniqueKeysWithValues: records.map { ($0.id, $0) })
        return inputs.allSatisfy { input in
            guard let record = recordsByID[input.id] else { return false }
            return record.source == input.source
                && record.version == input.version
                && record.displayName == input.displayName
                && record.summary == input.summary
                && record.category == input.category
                && record.instructionsRef == input.instructionsRef
                && record.instructionsDigest == input.instructionsDigest
                && record.instructionsText == input.instructionsText
                && record.requiredTools == input.requiredTools
                && record.defaultMode == input.defaultMode
        }
    }

    private static func hiddenSkillIDs(
        store: any MacSkillCatalogApplicationClient = MacApplicationPort.shared
    ) throws -> Set<MacBuiltInSkillID> {
        guard let setting = try store.productSetting(key: MacProductSettingKeys.hiddenBuiltInSkills) else {
            return []
        }
        guard let data = setting.valueJSON.data(using: .utf8) else {
            throw MacBuiltInSkillRegistryError.invalidSettingEncoding
        }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacBuiltInSkillRegistryError.invalidSettingPayload
        }
        let unknownFields = Set(object.keys).subtracting(["schemaVersion", "ids"])
        guard unknownFields.isEmpty else {
            throw MacBuiltInSkillRegistryError.unknownSettingFields(unknownFields.sorted())
        }
        let decoded = try JSONDecoder().decode(HiddenSkillSetting.self, from: data)
        guard decoded.schemaVersion == 1 else {
            throw MacBuiltInSkillRegistryError.unsupportedSettingSchema(decoded.schemaVersion)
        }
        let unknownIDs = decoded.ids.filter { MacBuiltInSkillID(rawValue: $0) == nil }
        guard unknownIDs.isEmpty else {
            throw MacBuiltInSkillRegistryError.unknownSkillIDs(unknownIDs)
        }
        return Set(decoded.ids.compactMap(MacBuiltInSkillID.init(rawValue:)))
    }

    private static func removingManagedBlock(from existing: String) -> String {
        guard let start = existing.range(of: managedStart),
              let end = existing.range(of: managedEnd, range: start.upperBound..<existing.endIndex) else {
            return existing
        }
        var output = existing
        output.removeSubrange(start.lowerBound..<end.upperBound)
        let trimmed = output.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "" : trimmed + "\n"
    }
}

nonisolated enum MacBuiltInSkillRegistryError: LocalizedError, Equatable {
    case invalidInstructionEncoding(String)
    case invalidSettingEncoding
    case invalidSettingPayload
    case unknownSettingFields([String])
    case unsupportedSettingSchema(Int)
    case unknownSkillIDs([String])

    var errorDescription: String? {
        switch self {
        case .invalidInstructionEncoding(let id): return "Built-in Skill instructions are not valid UTF-8: \(id)."
        case .invalidSettingEncoding: return "Built-in Skill settings are not valid UTF-8."
        case .invalidSettingPayload: return "Built-in Skill settings are not a JSON object."
        case .unknownSettingFields(let fields): return "Unknown built-in Skill setting fields: \(fields.joined(separator: ", "))."
        case .unsupportedSettingSchema(let version): return "Unsupported built-in Skill setting schema: \(version)."
        case .unknownSkillIDs(let ids): return "Unknown built-in Skill identifiers: \(ids.joined(separator: ", "))."
        }
    }
}
