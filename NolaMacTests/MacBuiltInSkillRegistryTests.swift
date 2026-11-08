import XCTest
@testable import Nola

@MainActor
final class MacBuiltInSkillRegistryTests: XCTestCase {
    func testMacCorePolicyExposesOnlyFormalSkillCapability() throws {
        let plan = try MacCoreToolPolicy.localPlan(
            executableURL: URL(fileURLWithPath: "/bin/echo"),
            projectURL: URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true),
            sessionId: "skill-policy-contract"
        )

        let skill = try XCTUnwrap(plan.output.enabledTools.first { $0.toolId == "skill" })
        XCTAssertEqual(skill.toolId, "skill")
        XCTAssertEqual(skill.name, "skill")
        XCTAssertEqual(skill.authorization, "granted")
        XCTAssertNil(plan.output.enabledTools.first { $0.toolId == "skill_context" })
    }

    private var defaults: UserDefaults!
    private var defaultsSuiteName: String!

    override func setUp() {
        super.setUp()
        defaultsSuiteName = "MacBuiltInSkillRegistryTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: defaultsSuiteName)
        defaults.removePersistentDomain(forName: defaultsSuiteName)
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
    }

    override func tearDown() {
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
        defaults.removePersistentDomain(forName: defaultsSuiteName)
        defaults = nil
        defaultsSuiteName = nil
        super.tearDown()
    }

    func testRegistersEveryBuiltInSkill() {
        let descriptors = MacBuiltInSkillRegistry.descriptors

        XCTAssertEqual(descriptors.map(\.id), MacBuiltInSkillID.allCases)
        XCTAssertEqual(Set(descriptors.map(\.instructionURL.lastPathComponent)), Set(["SKILL.md"]))
        XCTAssertTrue(descriptors.allSatisfy { FileManager.default.fileExists(atPath: $0.instructionURL.path) })
    }

    func testBuiltInEditorReadsCommittedContentAndRevisionInsteadOfMutableFile() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-editor-snapshot-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        let url = try MacBuiltInSkillRegistry.saveOverride(
            "Committed instructions", for: .word, overridesRootURL: overrides
        )
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let item = try XCTUnwrap(MacBuiltInSkillRegistry.extensionItems(store: store).first {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) == .word
        })
        let editor = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue))
        try Data("Uncommitted external changes".utf8).write(to: url)
        let document = try await MacExtensionsService.shared.document(for: item, store: store)
        XCTAssertEqual(document.content, "Committed instructions")
        XCTAssertEqual(document.canonicalSkillRevision, editor.revision)
        XCTAssertEqual(document.url, url)
        XCTAssertEqual(try store.skillEditor(editor.record.id)?.revision, editor.revision)
    }

    func testMissingBundleFailsStartupWithoutReplacingInstalledCatalog() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-incomplete-bundle-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            store: store, resourceRootURL: nil
        ))
        XCTAssertTrue(try store.skillEditors().isEmpty)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let baseline = try store.skillEditors()
        let partialBundle = root.appendingPathComponent("partial-bundle", isDirectory: true)
        let word = partialBundle.appendingPathComponent(MacBuiltInSkillID.word.resourceDirectory, isDirectory: true)
        try FileManager.default.createDirectory(at: word, withIntermediateDirectories: true)
        try Data("Changed instructions in incomplete package".utf8).write(to: word.appendingPathComponent("SKILL.md"))
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: root.appendingPathComponent("overrides"),
            store: store, resourceRootURL: partialBundle
        ))
        let afterFailure = try store.skillEditors()
        XCTAssertEqual(afterFailure.map { $0.record.id }, baseline.map { $0.record.id })
        XCTAssertEqual(afterFailure.map(\.revision), baseline.map(\.revision))
        XCTAssertEqual(afterFailure.map { $0.record.instructionsText }, baseline.map { $0.record.instructionsText })
    }

    func testRelocatedBundleRegistersANewImmutableVersionWithTheSameContent() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-relocated-skills-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let relocated = root.appendingPathComponent("relocated-bundle", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: source, to: relocated)
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let before = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue)).record
        try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store, resourceRootURL: relocated
        )
        let after = try XCTUnwrap(store.skillEditor(before.id)).record
        XCTAssertEqual(after.instructionsDigest, before.instructionsDigest)
        XCTAssertEqual(after.instructionsText, before.instructionsText)
        XCTAssertNotEqual(after.instructionsRef, before.instructionsRef)
        XCTAssertNotEqual(after.version, before.version)
        let revision = try store.skillEditor(before.id)?.revision
        try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store, resourceRootURL: relocated
        )
        XCTAssertEqual(try store.skillEditor(before.id)?.revision, revision)
    }

    func testCatalogMetadataRefreshIsDecidedByApplicationRatherThanTextDigest() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-catalog-metadata-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let original = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue)).record
        let stale = try XCTUnwrap(store.synchronizeBuiltInSkills([MacCanonicalSkillInput(
            id: original.id, source: .builtIn, version: original.version,
            displayName: original.displayName, summary: "Outdated package description",
            category: "outdated-category", instructionsRef: original.instructionsRef,
            instructionsDigest: original.instructionsDigest, instructionsText: original.instructionsText,
            requiredTools: original.requiredTools, defaultMode: original.defaultMode,
            enabled: original.enabled, createdAtMs: original.createdAtMs, updatedAtMs: original.updatedAtMs
        )]).first)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let refreshed = try XCTUnwrap(store.skillEditor(original.id))
        XCTAssertEqual(refreshed.record.summary, original.summary)
        XCTAssertEqual(refreshed.record.category, original.category)
        XCTAssertEqual(refreshed.record.instructionsDigest, original.instructionsDigest)
        XCTAssertGreaterThan(refreshed.revision, stale.revision)
        let revisions = try store.skillEditors().map(\.revision)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        XCTAssertEqual(try store.skillEditors().map(\.revision), revisions)
    }

    func testUnchangedBuiltInCatalogDoesNotWriteAnotherDurableCatalogReceipt() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-stable-skill-catalog-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state/nola.db")
        let store = try MacApplicationPort(databaseURL: database)

        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let initialReceipts = try builtInSynchronizationReceiptCount(database: database)
        XCTAssertEqual(initialReceipts, 1)

        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        XCTAssertEqual(
            try builtInSynchronizationReceiptCount(database: database),
            initialReceipts
        )
    }

    private func builtInSynchronizationReceiptCount(database: URL) throws -> Int {
        let sqlite = Process()
        let output = Pipe()
        sqlite.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        sqlite.arguments = [
            database.path,
            "SELECT count(*) FROM command_receipts "
                + "WHERE command_type='skill' "
                + "AND json_extract(command_json, '$.command.type')='synchronize_built_ins';",
        ]
        sqlite.standardOutput = output
        sqlite.standardError = output
        try sqlite.run()
        sqlite.waitUntilExit()
        let text = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard sqlite.terminationStatus == 0, let count = Int(text) else {
            throw NSError(
                domain: "MacBuiltInSkillRegistryTests",
                code: Int(sqlite.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: text]
            )
        }
        return count
    }

    func testFrozenSkillSnapshotKeepsDirectoryAuthorityConsistentAfterToggle() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-frozen-skills-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let frozen = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        let word = try XCTUnwrap(frozen.first { $0.id == .word })
        let directory = word.instructionURL.deletingLastPathComponent().standardizedFileURL.path
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word, store: store)
        let revisions = try store.skillEditors().map(\.revision)
        XCTAssertTrue(MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: root, activeSkills: frozen
        ).contains(directory))
        XCTAssertTrue(MacBuiltInSkillRegistry.skillDirectoryArguments(activeSkills: frozen).contains(directory))
        XCTAssertEqual(try store.skillEditors().map(\.revision), revisions)
        let next = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        XCTAssertFalse(MacBuiltInSkillRegistry.skillDirectoryArguments(activeSkills: next).contains(directory))
        XCTAssertFalse(MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: root, activeSkills: next
        ).contains(directory))
    }

    func testInvalidLastPackageCannotPartiallyUpdateTheInstalledCatalog() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-catalog-preparation-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        XCTAssertTrue(try MacBuiltInSkillRegistry.extensionItems(store: store).isEmpty)
        XCTAssertTrue(try store.skillEditors().isEmpty)
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let installedGallery = try MacBuiltInSkillRegistry.extensionItems(store: store)
        let installedActive = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        let baseline = try store.skillEditors()
        let ids = MacBuiltInSkillID.allCases
        let first = try XCTUnwrap(ids.first)
        let last = try XCTUnwrap(ids.last)
        XCTAssertNotEqual(first, last)
        for (id, bytes) in [(first, Data("Changed package instructions".utf8)), (last, Data([0xFF]))] {
            let directory = overrides.appendingPathComponent(id.resourceDirectory, isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try bytes.write(to: directory.appendingPathComponent("SKILL.md"))
        }
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store
        ))
        XCTAssertEqual(try MacBuiltInSkillRegistry.extensionItems(store: store), installedGallery)
        XCTAssertEqual(try MacBuiltInSkillRegistry.activeDescriptors(store: store), installedActive)
        let afterFailure = try store.skillEditors()
        XCTAssertEqual(afterFailure.map { $0.record.id }, baseline.map { $0.record.id })
        XCTAssertEqual(afterFailure.map(\.revision), baseline.map(\.revision))
        XCTAssertEqual(afterFailure.map { $0.record.instructionsText }, baseline.map { $0.record.instructionsText })
    }

    func testExtensionsDiscoveryIncludesEveryBuiltInSkillWithoutAWorkspace() async throws {
        let items = try await MacExtensionsService.shared.localExtensions(workspaceURL: nil)
        let builtInSkillIDs = items
            .filter { $0.kind == .skill && $0.source == .builtIn }
            .compactMap { MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) }

        XCTAssertEqual(Set(builtInSkillIDs), Set(MacBuiltInSkillID.allCases))
    }

    func testExtensionCatalogStartsAsGalleryAndGroupsSkillsBeforeOpeningDetail() {
        let skill = MacExtensionItem(
            id: "builtin-skill:\(MacBuiltInSkillID.word.rawValue)",
            kind: .skill,
            source: .builtIn,
            title: "Word",
            detail: "Create documents",
            path: "/tmp/word/SKILL.md",
            isEnabled: true
        )
        let mcp = MacExtensionItem(
            id: "mcp:example",
            kind: .mcp,
            source: .workspace,
            title: "Example MCP",
            detail: "Workspace server",
            path: "/tmp/mcp.json",
            isEnabled: true
        )

        let sections = MacExtensionCatalog.sections(for: [mcp, skill])

        XCTAssertEqual(sections.map(\.kind), [.skill, .mcp])
        XCTAssertEqual(sections[0].functionalCategory, .documents)
        XCTAssertEqual(sections[0].title, String(localized: "Documents"))
        XCTAssertEqual(sections[0].items, [skill])
        XCTAssertEqual(sections[1].items, [mcp])
    }

    func testBuiltInSkillsAreGroupedByFunctionRatherThanPublisherOrSuite() throws {
        let sections = MacExtensionCatalog.sections(
            for: try MacBuiltInSkillRegistry.extensionItems()
        )
        let skillSections = sections.filter { $0.kind == .skill }

        XCTAssertEqual(
            skillSections.compactMap(\.functionalCategory),
            MacSkillFunctionalCategory.allCases.filter { $0 != .custom }
        )
        XCTAssertTrue(skillSections.allSatisfy { !$0.items.isEmpty })
        XCTAssertFalse(skillSections.map(\.title).contains("Nola Productivity"))
        XCTAssertFalse(skillSections.map(\.title).contains("Research Workbench"))
    }

    func testOfficeSkillResourcesUseExplicitDesignAndValidationRules() throws {
        let officeIDs: Set<MacBuiltInSkillID> = [.word, .excel, .powerpoint]
        for descriptor in MacBuiltInSkillRegistry.descriptors where officeIDs.contains(descriptor.id) {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            let expectedExtension = descriptor.id == .word ? ".docx" : descriptor.id == .excel ? ".xlsx" : ".pptx"
            XCTAssertTrue(content.hasPrefix("---\nname:"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains("\ndescription:"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains(expectedExtension), descriptor.id.rawValue)
            for requiredSection in [
                "## Activation boundary",
                "## Default design system",
                "## Workflow",
                "## Validation gate",
                "## Completion contract",
                "## Safety and quality rules",
            ] {
                XCTAssertTrue(content.contains(requiredSection), "\(descriptor.id.rawValue): \(requiredSection)")
            }
            XCTAssertTrue(content.localizedCaseInsensitiveContains("validate"), descriptor.id.rawValue)
            XCTAssertTrue(content.localizedCaseInsensitiveContains("never overwrite"), descriptor.id.rawValue)
            XCTAssertTrue(content.localizedCaseInsensitiveContains("absolute file path"), descriptor.id.rawValue)
            for sharedColor in ["#18212B", "#1769E0", "#EEF4FA"] {
                XCTAssertTrue(content.contains(sharedColor), "\(descriptor.id.rawValue): \(sharedColor)")
            }

            let orderedSections = [
                "## Activation boundary",
                "## Default design system",
                "## Workflow",
                "## Validation gate",
                "## Completion contract",
                "## Safety and quality rules",
            ]
            let sectionOffsets = try orderedSections.map { section in
                try XCTUnwrap(content.range(of: section)?.lowerBound, "\(descriptor.id.rawValue): \(section)")
            }
            XCTAssertEqual(sectionOffsets, sectionOffsets.sorted(), descriptor.id.rawValue)
        }
    }

    func testWordSkillRequiresNativeOOXMLAndRejectsLossyTextutilConversion() throws {
        let descriptor = try XCTUnwrap(
            MacBuiltInSkillRegistry.descriptors.first { $0.id == .word }
        )
        let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)

        XCTAssertTrue(content.contains("scripts/create_document.py"))
        XCTAssertTrue(content.contains("native OOXML"))
        XCTAssertTrue(content.contains("Never create a DOCX with `textutil`"))
        XCTAssertFalse(content.contains("textutil -convert docx"))
    }

    func testEveryGeneralSkillUsesTheSharedSafetyContract() throws {
        let officeIDs: Set<MacBuiltInSkillID> = [.word, .excel, .powerpoint]
        let generalSkills = MacBuiltInSkillRegistry.descriptors.filter { !officeIDs.contains($0.id) }
        let sideEffectingSkillIDs = Set(generalSkills.map(\.id)).subtracting([.deepResearch])

        XCTAssertEqual(generalSkills.count, 21)
        for descriptor in generalSkills {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            XCTAssertTrue(content.hasPrefix("---\nname: \(descriptor.id.rawValue)\n"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains("\ndescription:"), descriptor.id.rawValue)
            for section in ["## Readiness", "## Workflow", "## User control", "## Validation", "## Completion"] {
                XCTAssertTrue(content.contains(section), "\(descriptor.id.rawValue): \(section)")
            }
            if sideEffectingSkillIDs.contains(descriptor.id) {
                XCTAssertTrue(content.localizedCaseInsensitiveContains("confirm"), descriptor.id.rawValue)
            }
            XCTAssertFalse(content.contains("TODO"), descriptor.id.rawValue)
        }
    }

    func testEveryBuiltInSkillRetainsDomainSpecificQualityGates() throws {
        let requiredTerms: [MacBuiltInSkillID: [String]] = [
            .word: ["tracked changes", "accessibility", "render"],
            .excel: ["recalculation", "external links", "row counts"],
            .powerpoint: ["speaker notes", "overflow", "render every slide"],
            .pdf: ["underlying content", "page manifest", "encryption state"],
            .figma: ["advisory mode", "Auto Layout", "before snapshot"],
            .dataAnalysis: ["analysis contract", "causal inference", "row counts"],
            .deepResearch: ["source ledger", "up to 20 ranked results", "search snippets"],
            .documentReview: ["finding categories", "tracked changes", "page or section anchor"],
            .imageOCR: ["reading order", "OCR confidence", "uncertain"],
            .meetings: ["explicit decisions", "action item", "speaker attribution"],
            .scheduleTasks: ["preview table", "persistent identifier", "timezone"],
            .knowledgeMemory: ["explicit consent", "provenance", "expiry"],
            .fileOrganization: ["dry-run manifest", "hash equality", "symlinks"],
            .automations: ["idempotency", "retry cap", "dry-run"],
            .email: ["reply all", "exact draft", "provider message identifier"],
            .academicReader: ["source format", "source anchors", "figure"],
            .scientificWriting: ["paper_type", "evidence", "output format"],
            .academicPolishing: ["paper_type", "section", "journal"],
            .manuscriptReviewer: ["3 reviewer reports", "claim_pointer", "evidence_pointer"],
            .reviewerResponse: ["point-by-point", "revision", "evidence"],
            .statisticsAudit: ["experimental unit", "effect sizes", "AUTHOR_INPUT_NEEDED"],
            .referenceVerifier: ["多源", "DOI", "Zotero"],
            .paperToPresentation: ["paper_type", "audit_pptx_quality.py", "source"],
            .openSourceModel: ["hardware profile", "NVIDIA CUDA", "AMD ROCm", "execution_mode: background", "bash_background_status", "Content-Range", "ETag", "cross-process lock", "task.json", "catalog.json", "64K", "peak memory", "SHA-256", "smoke inference"],
        ]

        XCTAssertEqual(Set(requiredTerms.keys), Set(MacBuiltInSkillID.allCases))
        for descriptor in MacBuiltInSkillRegistry.descriptors {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            for term in try XCTUnwrap(requiredTerms[descriptor.id]) {
                XCTAssertTrue(
                    content.localizedCaseInsensitiveContains(term),
                    "\(descriptor.id.rawValue): \(term)"
                )
            }
        }
    }

    func testResearchSkillsAreBundledWithRuntimeContractsAndSharedReferences() throws {
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let researchSkills = MacBuiltInSkillID.allCases.filter(\.usesSharedResearchReferences)

        XCTAssertEqual(researchSkills.count, 8)
        XCTAssertTrue(FileManager.default.fileExists(
            atPath: root.appendingPathComponent("nature-shared/SKILL.md").path
        ))
        for id in researchSkills {
            let directory = root.appendingPathComponent(id.resourceDirectory, isDirectory: true)
            XCTAssertTrue(FileManager.default.fileExists(
                atPath: directory.appendingPathComponent("NOLA_MANIFEST.json").path
            ), id.rawValue)
        }
    }

    func testEveryUserFacingBuiltInSkillShipsAMachineReadableRuntimeContract() throws {
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)

        for id in MacBuiltInSkillID.allCases {
            let manifestURL = root
                .appendingPathComponent(id.resourceDirectory, isDirectory: true)
                .appendingPathComponent("NOLA_MANIFEST.json")
            let data = try Data(contentsOf: manifestURL)
            let manifest = try XCTUnwrap(
                JSONSerialization.jsonObject(with: data) as? [String: Any],
                id.rawValue
            )
            XCTAssertEqual(manifest["schema_version"] as? Int, 1, id.rawValue)
            XCTAssertEqual(manifest["id"] as? String, id.rawValue, id.rawValue)
            XCTAssertEqual(manifest["delivery"] as? String, "built-in", id.rawValue)
            XCTAssertNotNil(manifest["category"] as? [String: Any], id.rawValue)
            XCTAssertFalse((manifest["required_tools"] as? [String] ?? []).isEmpty, id.rawValue)
            XCTAssertFalse((manifest["validation"] as? [String] ?? []).isEmpty, id.rawValue)
        }
    }

    func testAgentHostSkillSnapshotRemovesHistoricalGeneratedRouter() throws {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("nola-skill-router-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let agentsURL = directory.appendingPathComponent("AGENTS.md")
        try """
        # User-owned guidance

        <!-- NOLA_MAC_BUILTIN_SKILLS_START -->
        stale generated Skill routes
        <!-- NOLA_MAC_BUILTIN_SKILLS_END -->
        """.write(to: agentsURL, atomically: true, encoding: .utf8)

        try MacBuiltInSkillRegistry.installTrustedRouter(
            configurationDirectory: directory,
            activeSkills: MacBuiltInSkillRegistry.descriptors
        )
        let remaining = try String(contentsOf: agentsURL, encoding: .utf8)
        XCTAssertTrue(remaining.contains("User-owned guidance"))
        XCTAssertFalse(remaining.contains("NOLA_MAC_BUILTIN_SKILLS"))
        XCTAssertFalse(remaining.contains("stale generated Skill routes"))
    }

    func testAllowedDirectoriesKeepProjectAndGrantOnlyEnabledSkillDirectories() throws {
        let project = URL(fileURLWithPath: "/tmp/nola-project", isDirectory: true)
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word)

        let arguments = try MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: project
        )
        let allowedPaths = stride(from: 1, to: arguments.count, by: 2).map { arguments[$0] }
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)

        XCTAssertEqual(arguments.first, "--allow-dir")
        XCTAssertTrue(allowedPaths.contains(project.path))
        XCTAssertFalse(allowedPaths.contains(root.path))
        let descriptorPaths = Dictionary(
            uniqueKeysWithValues: MacBuiltInSkillRegistry.descriptors.map {
                ($0.id, $0.instructionURL.deletingLastPathComponent().standardizedFileURL.path)
            }
        )
        XCTAssertFalse(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.word])))
        XCTAssertTrue(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.excel])))
        XCTAssertTrue(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.powerpoint])))
    }

    func testEnableStatePersistsAndUpdatesExtensionItems() throws {
        try MacBuiltInSkillRegistry.setEnabled(false, for: .excel)

        let items = try MacBuiltInSkillRegistry.extensionItems()
        let excel = items.first {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) == .excel
        }

        XCTAssertEqual(items.count, MacBuiltInSkillID.allCases.count)
        XCTAssertEqual(excel?.isEnabled, false)
        XCTAssertFalse(try MacBuiltInSkillRegistry.activeDescriptors().contains {
            $0.id == .excel
        })
    }

    func testEnableStateUsesTheExplicitApplicationPort() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-store-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let isolatedStore = try MacApplicationPort(
            databaseURL: directory.appendingPathComponent("state/nola.db")
        )

        try MacBuiltInSkillRegistry.setEnabled(
            false,
            for: .fileOrganization,
            store: isolatedStore
        )

        XCTAssertFalse(try MacBuiltInSkillRegistry.activeDescriptors(store: isolatedStore).contains {
            $0.id == .fileOrganization
        })
        XCTAssertTrue(try MacBuiltInSkillRegistry.activeDescriptors().contains {
            $0.id == .fileOrganization
        })
    }

    func testDiscoveredWorkspaceSkillRequiresExplicitActivationAndEntersHostSnapshot() async throws {
        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-workspace-skill-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let skillDirectory = workspace.appendingPathComponent(".nola/skills/example", isDirectory: true)
        try FileManager.default.createDirectory(at: skillDirectory, withIntermediateDirectories: true)
        try """
        ---
        name: example-workspace-skill
        description: Test workspace activation
        ---
        # Example
        """.write(
            to: skillDirectory.appendingPathComponent("SKILL.md"),
            atomically: true,
            encoding: .utf8
        )

        let discovered = try await MacExtensionsService.shared.localExtensions(workspaceURL: workspace)
        let item = try XCTUnwrap(discovered.first {
            $0.source == .workspace && $0.title == "example-workspace-skill"
        })
        XCTAssertFalse(item.isEnabled)

        let applicationPort = try MacApplicationPort(
            databaseURL: workspace.appendingPathComponent("nola-product.db")
        )
        try MacLocalSkillActivationStore.setEnabled(true, itemID: item.id, store: applicationPort)
        let applicationSupport = workspace.appendingPathComponent("ApplicationSupport", isDirectory: true)
        let request = try MacLocalAgentRunRequest(
            conversationID: "workspace-skill-contract",
            prompt: "Use the enabled workspace Skill.",
            projectURL: workspace,
            attachmentURLs: [],
            appEnvironment: MacAppEnvironment(
                applicationSupportRoot: applicationSupport,
                cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
            ),
            applicationPort: applicationPort,
        )

        XCTAssertTrue(request.activeExternalSkillDirectories.contains(skillDirectory.standardizedFileURL))
        XCTAssertTrue(request.skillDirectoryArguments.contains(skillDirectory.path))
        XCTAssertTrue(request.allowedDirectoryArguments.contains(skillDirectory.path))
        XCTAssertFalse(request.skillDirectoryArguments.contains("--no-skills"))
    }

    func testRemoveHidesSkillAndRestoreReturnsAllSkillsEnabled() throws {
        try MacBuiltInSkillRegistry.setEnabled(false, for: .powerpoint)
        try MacBuiltInSkillRegistry.remove(.word)

        XCTAssertTrue(try MacBuiltInSkillRegistry.hasRemovedSkills())
        let visibleIDs = try MacBuiltInSkillRegistry.extensionItems().compactMap {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id)
        }
        XCTAssertEqual(visibleIDs, MacBuiltInSkillID.allCases.filter { $0 != .word })

        try MacBuiltInSkillRegistry.restoreBuiltInSkills()

        XCTAssertFalse(try MacBuiltInSkillRegistry.hasRemovedSkills())
        XCTAssertEqual(
            try MacBuiltInSkillRegistry.extensionItems().count,
            MacBuiltInSkillID.allCases.count
        )
        XCTAssertTrue(try MacBuiltInSkillRegistry.extensionItems().allSatisfy(\.isEnabled))
    }

    func testInvalidCanonicalHiddenSkillSettingsFailClosed() throws {
        let store = try XCTUnwrap(MacApplicationPort.shared)
        let key = MacProductSettingKeys.hiddenBuiltInSkills
        let previous = try store.productSetting(key: key)
        defer {
            if let previous {
                _ = try? store.putProductSetting(
                    key: key,
                    valueJSON: previous.valueJSON,
                    updatedAtMs: previous.updatedAtMs
                )
            } else {
                _ = try? store.deleteProductSetting(key: key)
            }
        }
        let invalidPayloads = [
            #"{"schemaVersion":0,"ids":[]}"#,
            #"{"schemaVersion":1,"ids":[],"legacyIDs":[]}"#,
            #"{"schemaVersion":1,"ids":["removed-skill-that-does-not-exist"]}"#,
        ]
        for (index, payload) in invalidPayloads.enumerated() {
            _ = try store.putProductSetting(
                key: key,
                valueJSON: payload,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000) + Int64(index)
            )
            XCTAssertThrowsError(try MacBuiltInSkillRegistry.extensionItems())
        }
    }

    func testBuiltInSkillOverrideIsEditableAndUsedByDescriptorUntilReset() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-overrides-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let custom = """
        ---
        name: word
        description: Customized Word workflow
        ---
        # Custom Word workflow
        """

        let savedURL = try MacBuiltInSkillRegistry.saveOverride(
            custom,
            for: .word,
            overridesRootURL: root
        )

        XCTAssertTrue(MacBuiltInSkillRegistry.hasOverride(for: .word, overridesRootURL: root))
        XCTAssertEqual(
            MacBuiltInSkillRegistry.instructionURL(for: .word, overridesRootURL: root),
            savedURL
        )
        XCTAssertEqual(try String(contentsOf: savedURL, encoding: .utf8), custom)

        try MacBuiltInSkillRegistry.resetOverride(for: .word, overridesRootURL: root)

        XCTAssertFalse(MacBuiltInSkillRegistry.hasOverride(for: .word, overridesRootURL: root))
        let restoredURL = try XCTUnwrap(
            MacBuiltInSkillRegistry.instructionURL(for: .word, overridesRootURL: root)
        )
        XCTAssertNotEqual(restoredURL, savedURL)
        XCTAssertTrue(restoredURL.path.contains("NolaBuiltInSkills.bundle"))
    }
}
