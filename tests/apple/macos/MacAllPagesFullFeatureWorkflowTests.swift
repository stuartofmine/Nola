// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exhaustive real-state, all-pages macOS functional & UI workflow test suite covering
//   Capabilities (Skills, MCP, Extensions/Plugins), Automation Center & Activity, Memory & Data Sources
//   & Knowledge, Experts & Agent Rooms, Conversation Management & Inspector Tabs, and Settings /
//   Local Models / Backup & Restore / Onboarding against strictly isolated temp SQLite stores.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacAllPagesFullFeatureWorkflowTests: XCTestCase {
    override func tearDown() {
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        super.tearDown()
    }

    // MARK: - Isolated Test Helpers

    private func makeTempDirectory(_ prefix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-allpages-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    private func makeIsolatedWorkspaceModel(
        root: URL,
        port: MacApplicationPort,
        activityStore: MacActivityStore? = nil
    ) throws -> MacWorkspaceModel {
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)
        let repository = MacCanonicalConversationRepository(store: port)
        let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
        let resolvedActivity = activityStore ?? MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let suiteName = "com.gaojc.Nola.allpages-bookmarks.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: suiteName) }
        let bookmarkStore = MacSecurityScopedBookmarkStore(defaults: defaults)
        return MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: E2ENoOpRemoteHost(),
            modelApplication: port,
            bookmarkStore: bookmarkStore,
            activityStore: resolvedActivity,
            completionNotifier: { _, _, _ in },
            readinessIssue: { _ in nil }
        )
    }

    @discardableResult
    private func renderHostedView<V: View>(
        _ view: V,
        label: String,
        size: NSSize = NSSize(width: 1120, height: 760),
        colorScheme: ColorScheme = .dark
    ) throws -> Data {
        let wrapped = AnyView(
            view
                .frame(width: size.width, height: size.height)
                .environment(\.colorScheme, colorScheme)
        )
        let hosting = NSHostingView(rootView: wrapped)
        hosting.frame = NSRect(origin: .zero, size: size)
        let window = NSWindow(
            contentRect: hosting.frame,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
        window.contentView = hosting
        window.displayIfNeeded()
        hosting.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(hosting.bounds.width, 0, "\(label): rendered width must be > 0")
        XCTAssertGreaterThan(hosting.bounds.height, 0, "\(label): rendered height must be > 0")
        let rep = try XCTUnwrap(
            hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds),
            "\(label): bitmapImageRepForCachingDisplay returned nil"
        )
        hosting.cacheDisplay(in: hosting.bounds, to: rep)
        let png = try XCTUnwrap(
            rep.representation(using: .png, properties: [:]),
            "\(label): PNG representation failed"
        )
        XCTAssertGreaterThan(png.count, 100, "\(label): rendered PNG must be non-empty")
        window.contentView = nil
        window.close()
        return png
    }

    // MARK: - 1. Capabilities (Skills, MCP, Extensions/Plugins)

    func testCapabilitiesSkillsMCPAndExtensionsFullLifecycleAndUI() throws {
        let root = try makeTempDirectory("capabilities")
        let dbURL = root.appendingPathComponent("capabilities.db")
        let port = try MacApplicationPort(databaseURL: dbURL)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // 1a. Built-in skills discovery & canonical synchronization
        let overridesRoot = root.appendingPathComponent("builtin-overrides", isDirectory: true)
        let builtInDescriptors = MacBuiltInSkillRegistry.descriptors(overridesRootURL: overridesRoot)
        XCTAssertFalse(builtInDescriptors.isEmpty, "Built-in skill catalog should contain bundled skills")

        let builtInInputs: [MacCanonicalSkillInput] = builtInDescriptors.prefix(4).map { desc in
            MacCanonicalSkillInput(
                id: desc.id.rawValue,
                source: .builtIn,
                version: "2.0.0",
                displayName: desc.title,
                summary: desc.summary,
                category: desc.id.functionalCategory.rawValue,
                instructionsRef: desc.instructionURL.absoluteString,
                instructionsText: (try? String(contentsOf: desc.instructionURL, encoding: .utf8)) ?? "# \(desc.title)",
                requiredTools: [],
                defaultMode: .inline,
                enabled: true,
                createdAtMs: nowMs,
                updatedAtMs: nowMs
            )
        }
        let syncedBuiltIns = try port.synchronizeBuiltInSkills(builtInInputs)
        XCTAssertGreaterThanOrEqual(syncedBuiltIns.count, builtInInputs.count)

        // 1b. Create, edit, toggle, persist across reopen, and delete a custom user skill
        let skillID = "custom-allpages-skill-\(UUID().uuidString.lowercased())"
        let createdSkill = try port.createUserSkill(
            MacCanonicalSkillInput(
                id: skillID,
                source: .user,
                version: "1.0.0",
                displayName: "Release Audit Skill",
                summary: "Validates macOS release checklists",
                category: "custom",
                instructionsRef: "nola://github-skills/example/skills/blob/main/release-audit/SKILL.md",
                instructionsText: "# Release Audit v1\nVerify tests and signatures.",
                requiredTools: [],
                defaultMode: .inline,
                enabled: true,
                createdAtMs: nowMs,
                updatedAtMs: nowMs
            )
        )
        XCTAssertEqual(createdSkill.record.id, skillID)
        XCTAssertTrue(createdSkill.record.enabled)
        XCTAssertGreaterThan(createdSkill.revision, 0)

        // Edit instructions with CAS revision check
        let editedSkill = try port.updateInstalledSkillInstructions(
            "# Release Audit v2\nVerify tests, signatures, and SQLite v2 migrations.",
            expected: createdSkill
        )
        XCTAssertGreaterThan(editedSkill.revision, createdSkill.revision)
        XCTAssertTrue(editedSkill.record.instructionsText.contains("SQLite v2 migrations"))
        // Stale revision edit must be rejected
        XCTAssertThrowsError(
            try port.updateInstalledSkillInstructions("# Stale edit", expected: createdSkill),
            "Editing a skill with a stale revision must fail CAS check"
        )

        // Toggle disabled -> verify -> toggle enabled -> verify
        let disabledSkill = try port.setSkillEnabled(editedSkill, enabled: false, updatedAtMs: nowMs + 20)
        XCTAssertFalse(disabledSkill.record.enabled)
        XCTAssertGreaterThan(disabledSkill.revision, editedSkill.revision)

        // Cold reopen verifies persistence of both built-in and edited custom skills
        let reopenedPort1 = try MacApplicationPort(databaseURL: dbURL)
        let listedAfterReopen = try reopenedPort1.listSkills()
        XCTAssertTrue(listedAfterReopen.contains(where: { $0.id == builtInInputs[0].id }))
        let persistedSkill = try XCTUnwrap(try reopenedPort1.skillEditor(skillID))
        XCTAssertFalse(persistedSkill.record.enabled)
        XCTAssertTrue(persistedSkill.record.instructionsText.contains("SQLite v2 migrations"))

        let reenabledSkill = try reopenedPort1.setSkillEnabled(persistedSkill, enabled: true, updatedAtMs: nowMs + 30)
        XCTAssertTrue(reenabledSkill.record.enabled)

        // 1c. MCP Servers: add BOTH stdio and HTTP servers, list, toggle, persist across reopen, delete
        let stdioID = "mcp-stdio-\(UUID().uuidString.lowercased())"
        let httpID = "mcp-http-\(UUID().uuidString.lowercased())"
        let stdioConfig = #"{"name":"Local Stdio Inspector","transport":"stdio","command":"/usr/bin/env","args":["echo","hello"]}"#
        let httpConfig = #"{"name":"Remote HTTP Docs","transport":"http","url":"https://mcp.example.invalid/mcp"}"#

        let createdStdio = try reopenedPort1.createMCPServer(
            MacCanonicalMCPServerInput(
                id: stdioID,
                displayName: "Local Stdio Inspector",
                enabled: true,
                configurationJSON: stdioConfig,
                revision: 0,
                createdAtMs: nowMs,
                updatedAtMs: nowMs,
                deletedAtMs: nil
            )
        )
        let createdHTTP = try reopenedPort1.createMCPServer(
            MacCanonicalMCPServerInput(
                id: httpID,
                displayName: "Remote HTTP Docs",
                enabled: true,
                configurationJSON: httpConfig,
                revision: 0,
                createdAtMs: nowMs + 5,
                updatedAtMs: nowMs + 5,
                deletedAtMs: nil
            )
        )
        XCTAssertTrue(createdStdio.enabled)
        XCTAssertTrue(createdHTTP.enabled)

        let listedMCPs = try reopenedPort1.listMCPServers()
        XCTAssertTrue(listedMCPs.contains(where: { $0.id == stdioID }))
        XCTAssertTrue(listedMCPs.contains(where: { $0.id == httpID }))

        let updatedStdio = try reopenedPort1.configureMCPServer(
            createdStdio,
            displayName: "Local Stdio Inspector (Paused)",
            enabled: false,
            configurationJSON: stdioConfig,
            updatedAtMs: nowMs + 40
        )
        XCTAssertFalse(updatedStdio.enabled)
        XCTAssertEqual(updatedStdio.displayName, "Local Stdio Inspector (Paused)")

        let updatedHTTPConfig = #"{"name":"Remote HTTP Docs v2","transport":"http","url":"https://mcp.example.invalid/v2/mcp"}"#
        let updatedHTTP = try reopenedPort1.configureMCPServer(
            createdHTTP,
            displayName: "Remote HTTP Docs v2",
            enabled: true,
            configurationJSON: updatedHTTPConfig,
            updatedAtMs: nowMs + 45
        )
        XCTAssertEqual(updatedHTTP.displayName, "Remote HTTP Docs v2")
        XCTAssertTrue(updatedHTTP.configurationJSON.contains("/v2/mcp"))

        // Cold reopen verifies MCP persistence
        let reopenedPort2 = try MacApplicationPort(databaseURL: dbURL)
        let reloadedStdio = try XCTUnwrap(try reopenedPort2.listMCPServers().first(where: { $0.id == stdioID }))
        let reloadedHTTP = try XCTUnwrap(try reopenedPort2.listMCPServers().first(where: { $0.id == httpID }))
        XCTAssertFalse(reloadedStdio.enabled)
        XCTAssertEqual(reloadedStdio.displayName, "Local Stdio Inspector (Paused)")
        XCTAssertTrue(reloadedHTTP.enabled)
        XCTAssertEqual(reloadedHTTP.displayName, "Remote HTTP Docs v2")

        // 1d. Render REAL Capabilities, Extensions (Skills & MCP), and Experts+Skills+Plugins Center UI views
        let roomService = MacAgentRoomService(runtime: reopenedPort2)
        let roomWorkspace = MacAgentRoomWorkspace(service: roomService)
        let skillsNav = MacExtensionsNavigationState()
        let remoteHost = E2ENoOpRemoteHost()

        try renderHostedView(MacCapabilityCenterView(), label: "MacCapabilityCenterView", colorScheme: .dark)
        try renderHostedView(
            MacExtensionsCenterView(remoteHost: remoteHost, kinds: [.skill], store: reopenedPort2),
            label: "MacExtensionsCenterView.skill",
            colorScheme: .dark
        )
        try renderHostedView(
            MacExtensionsCenterView(remoteHost: remoteHost, kinds: [.mcp], store: reopenedPort2),
            label: "MacExtensionsCenterView.mcp",
            colorScheme: .light
        )
        for tab in MacExpertsAndSkillsCenterView.Tab.allCases {
            skillsNav.centerTab = tab
            try renderHostedView(
                MacExpertsAndSkillsCenterView(
                    roomWorkspace: roomWorkspace,
                    remoteHost: remoteHost,
                    skillsNavigation: skillsNav
                ),
                label: "MacExpertsAndSkillsCenterView.\(tab.rawValue)",
                colorScheme: .dark
            )
        }

        // Delete custom skill and both MCP servers & verify persistence across cold reopen
        XCTAssertTrue(try reopenedPort2.deleteUserSkill(reenabledSkill, deletedAtMs: nowMs + 60))
        XCTAssertTrue(try reopenedPort2.deleteMCPServer(reloadedStdio, deletedAtMs: nowMs + 65))
        XCTAssertTrue(try reopenedPort2.deleteMCPServer(reloadedHTTP, deletedAtMs: nowMs + 70))

        let postDeletePort = try MacApplicationPort(databaseURL: dbURL)
        XCTAssertFalse(try postDeletePort.listSkills().contains(where: { $0.id == skillID && $0.deletedAtMs == nil }))
        XCTAssertFalse(try postDeletePort.listMCPServers().contains(where: { $0.id == stdioID && $0.deletedAtMs == nil }))
        XCTAssertFalse(try postDeletePort.listMCPServers().contains(where: { $0.id == httpID && $0.deletedAtMs == nil }))
    }

    // MARK: - 2. Automation Center & Activity Store

    func testAutomationCenterSchedulesVerificationActivityAndUI() throws {
        let root = try makeTempDirectory("automations")
        let dbURL = root.appendingPathComponent("automations.db")
        let port = try MacApplicationPort(databaseURL: dbURL)
        let workspaceDir = root.appendingPathComponent("workspace", isDirectory: true)
        try FileManager.default.createDirectory(at: workspaceDir, withIntermediateDirectories: true)

        let env = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("Support"),
            cachesRoot: root.appendingPathComponent("Caches")
        )
        let registry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("registry.json"),
            applicationPort: port
        )
        let webStore = MacWebSourceStore(
            storeURL: root.appendingPathComponent("web.json"),
            registry: registry
        )
        let activityDir = root.appendingPathComponent("Activity", isDirectory: true)
        let activityStore = MacActivityStore(baseDirectory: activityDir, applicationPort: port)
        let automationStore = try MacAutomationStore(
            baseDirectory: root.appendingPathComponent("Automations"),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"),
            synchronizeSystemJobs: false,
            appEnvironment: env,
            authorizationResolver: { _ in true },
            activityStore: activityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: registry,
            webSourceStore: webStore,
            automationClient: port,
            skillCatalogClient: port
        )

        func makeAutomation(name: String, prompt: String, frequency: MacAutomation.Frequency, weekday: Int, hour: Int, minute: Int, enabled: Bool) -> MacAutomation {
            let spec = MacAutomationExecutionSpec(
                prompt: prompt,
                executionScope: .localWorkspace(root: workspaceDir.path),
                permissionProfile: "cautious",
                modelSelection: "use_current_default_at_admission",
                contextRefs: [],
                skillRefs: []
            )
            return MacAutomation(
                id: UUID(),
                name: name,
                prompt: prompt,
                frequency: frequency,
                weekday: weekday,
                hour: hour,
                minute: minute,
                workspacePath: workspaceDir.path,
                isEnabled: enabled,
                createdAt: .now,
                contextSources: [],
                executionSpec: spec
            )
        }

        // 2a. Create Daily, Weekly, and Hourly (interval) automations with verified canonical readback
        let dailyAuto = makeAutomation(
            name: "Daily Workspace Digest",
            prompt: "Summarize git changes from the last 24 hours.",
            frequency: .daily,
            weekday: 1,
            hour: 8,
            minute: 30,
            enabled: true
        )
        let weeklyAuto = makeAutomation(
            name: "Weekly Architecture Review",
            prompt: "Audit Swift/Rust FFI boundaries every Monday morning.",
            frequency: .weekly,
            weekday: 1,
            hour: 10,
            minute: 0,
            enabled: true
        )
        let hourlyAuto = makeAutomation(
            name: "Hourly Build Health Pulse",
            prompt: "Check workspace log status every hour at minute 15.",
            frequency: .hourly,
            weekday: 0,
            hour: 0,
            minute: 15,
            enabled: true
        )

        for auto in [dailyAuto, weeklyAuto, hourlyAuto] {
            let verified = try automationStore.upsertWithVerification(auto)
            XCTAssertNil(verified.verificationError, "upsertWithVerification failed for \(auto.name): \(verified.verificationError ?? "")")
            XCTAssertFalse(verified.submittedExecutionSpecDigest.isEmpty)
        }
        XCTAssertEqual(automationStore.automations.count, 3)
        XCTAssertEqual(try port.listAutomations().count, 3)

        // 2b. Toggle enable/disable and verify canonical SQLite reflection
        automationStore.setEnabled(false, for: weeklyAuto)
        let weeklyDisabled = try XCTUnwrap(try port.listAutomations().first(where: { $0.id == weeklyAuto.id.uuidString.lowercased() }))
        XCTAssertFalse(weeklyDisabled.enabled)

        automationStore.setEnabled(true, for: weeklyAuto)
        let weeklyReenabled = try XCTUnwrap(try port.listAutomations().first(where: { $0.id == weeklyAuto.id.uuidString.lowercased() }))
        XCTAssertTrue(weeklyReenabled.enabled)

        // 2c. Query scheduled occurrences across all 3 frequencies
        let now = Date()
        for auto in [dailyAuto, weeklyAuto, hourlyAuto] {
            let nextRun = try XCTUnwrap(
                auto.computeNextRun(using: port),
                "computeNextRun(using:) returned nil for \(auto.frequency.rawValue)"
            )
            XCTAssertGreaterThan(nextRun, now, "Next occurrence for \(auto.name) must be in the future")
        }

        // Verify multi-occurrence window query for hourly interval
        let multiQueryInput: MacCanonicalJSONValue = .object([
            "automation_id": .string(hourlyAuto.id.uuidString),
            "timezone": .string(TimeZone.current.identifier),
            "frequency": .string("hourly"),
            "time": .string("00:15"),
            "weekday": .signedInteger(1),
            "once_at_ms": .null,
            "after_ms": .signedInteger(Int64(now.timeIntervalSince1970 * 1_000)),
            "through_ms": .signedInteger(Int64(now.addingTimeInterval(6 * 3_600).timeIntervalSince1970 * 1_000)),
            "limit": .signedInteger(5)
        ])
        let occValue = try port.executeAgentRoomEnvelope(
            .object(["type": .string("query_automation_occurrences"), "input": multiQueryInput])
        )
        struct OccurrencesPage: Decodable {
            struct Item: Decodable { let scheduled_for_ms: Int64 }
            let occurrences: [Item]
        }
        let occPage = try JSONDecoder().decode(OccurrencesPage.self, from: JSONEncoder().encode(occValue))
        XCTAssertGreaterThanOrEqual(occPage.occurrences.count, 3, "Hourly query over 6h should yield >= 3 occurrences")
        for idx in 1..<occPage.occurrences.count {
            XCTAssertGreaterThan(occPage.occurrences[idx].scheduled_for_ms, occPage.occurrences[idx - 1].scheduled_for_ms)
        }

        // 2d. Verify MacActivityStore recording, persistence, and filtering
        activityStore.recordConversationRun(
            title: "Architecture Audit",
            conversationID: "conv-activity-1",
            execution: .thisMac,
            status: .answered,
            startedAt: Date().addingTimeInterval(-12),
            output: "Completed audit with zero regressions."
        )
        activityStore.recordConversationRun(
            title: "Blocked Deploy Check",
            conversationID: "conv-activity-2",
            execution: .localModel,
            status: .failed,
            startedAt: Date().addingTimeInterval(-5),
            output: "Failed check due to missing token."
        )
        XCTAssertEqual(activityStore.entries.count, 2)

        let reopenedActivity = MacActivityStore(baseDirectory: activityDir, applicationPort: port)
        XCTAssertEqual(reopenedActivity.entries.count, 2)
        XCTAssertTrue(reopenedActivity.entries.contains(where: { $0.conversationID == "conv-activity-1" && $0.status.isSuccessful }))
        XCTAssertTrue(reopenedActivity.entries.contains(where: { $0.conversationID == "conv-activity-2" && $0.status.needsAttention }))

        // 2e. Render REAL MacAutomationCenterView (list mode & run history mode) + MacActivityCenterView
        let autoNav = MacAutomationCenterNavigationState()
        try renderHostedView(
            MacAutomationCenterView(navigation: autoNav, store: automationStore, activityStore: activityStore)
                .environment(\.macActivityApplicationClient, port),
            label: "MacAutomationCenterView.list",
            colorScheme: .dark
        )
        autoNav.showRunHistory()
        try renderHostedView(
            MacAutomationCenterView(navigation: autoNav, store: automationStore, activityStore: activityStore)
                .environment(\.macActivityApplicationClient, port),
            label: "MacAutomationCenterView.runHistory",
            colorScheme: .light
        )
        try renderHostedView(
            MacActivityCenterView(automationOnly: false)
                .environment(\.macActivityApplicationClient, port),
            label: "MacActivityCenterView.all",
            colorScheme: .dark
        )

        // 2f. Delete automation & verify removal persists across cold reopen
        automationStore.delete(hourlyAuto)
        XCTAssertEqual(automationStore.automations.count, 2)
        let reopenedPort = try MacApplicationPort(databaseURL: dbURL)
        let remainingAutos = try reopenedPort.listAutomations()
        XCTAssertEqual(remainingAutos.count, 2)
        XCTAssertFalse(remainingAutos.contains(where: { $0.id == hourlyAuto.id.uuidString.lowercased() }))
    }

    // MARK: - 3. Memory, Data Sources, Web Sources, Overrides & Conversation Knowledge

    func testMemoryDataSourcesWebSourcesOverridesAndKnowledgeFullWorkflowAndUI() async throws {
        let root = try makeTempDirectory("memory-datasources-knowledge")
        let dbURL = root.appendingPathComponent("knowledge.db")
        let port = try MacApplicationPort(databaseURL: dbURL)
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)
        let runtime = MacAgentHostRuntime(credentialStore: configStore)

        // 3a. Memory full CRUD (create -> search/list -> revise -> delete)
        let prefMem = try await runtime.createMemory(
            MacAgentHostMemoryDraft(
                scope: "global",
                kind: "preference",
                content: "Always use isolated temporary SQLite v2 databases in automated macOS tests.",
                summary: "SQLite isolation preference",
                tags: ["testing", "sqlite"],
                confidence: 0.91,
                importance: 0.82,
                sensitivity: "normal",
                expiresAt: nil,
                source: "explicit_user_memory_management",
                provenanceKind: "user_stated"
            ),
            databaseURL: dbURL
        )
        let projectMem = try await runtime.createMemory(
            MacAgentHostMemoryDraft(
                scope: "project",
                kind: "project",
                content: "Nola 2.0 unified 8-crate workspace targets user_version = 2.",
                summary: "Nola 2.0 baseline",
                tags: ["nola", "architecture"],
                confidence: 0.95,
                importance: 0.90,
                sensitivity: "normal",
                expiresAt: nil,
                source: "explicit_user_memory_management",
                provenanceKind: "user_stated"
            ),
            databaseURL: dbURL
        )
        XCTAssertEqual(prefMem.kind, "preference")
        XCTAssertEqual(projectMem.scope, "project")

        let allMemories = try await runtime.listMemoryRecords(databaseURL: dbURL)
        XCTAssertEqual(allMemories.filter { $0.content.localizedCaseInsensitiveContains("SQLite") }.count, 1)
        XCTAssertEqual(allMemories.filter { $0.content.localizedCaseInsensitiveContains("8-crate") }.count, 1)

        // Revise preference memory & verify supersedes chain
        let revisedPref = try await runtime.reviseMemory(
            id: prefMem.id,
            draft: MacAgentHostMemoryDraft(
                scope: "global",
                kind: "preference",
                content: "Always use isolated temporary SQLite v2 databases and tear them down after macOS tests.",
                summary: "Updated SQLite isolation preference",
                tags: ["testing", "sqlite", "cleanup"],
                confidence: 0.97,
                importance: 0.88,
                sensitivity: "normal",
                expiresAt: nil,
                source: "explicit_user_memory_management",
                provenanceKind: "user_stated"
            ),
            databaseURL: dbURL
        )
        XCTAssertTrue(revisedPref.supersedes.contains(prefMem.id))
        XCTAssertEqual(revisedPref.confidence, 0.97, accuracy: 0.001)

        // Delete project memory & verify gone
        try await runtime.deleteMemory(id: projectMem.id, databaseURL: dbURL)
        let afterDelete = try await runtime.listMemoryRecords(databaseURL: dbURL)
        XCTAssertFalse(afterDelete.contains(where: { $0.id == projectMem.id }))
        XCTAssertTrue(afterDelete.contains(where: { $0.id == revisedPref.id }))

        // Render REAL MacMemoryCenterView & MacMemoryCenterPage
        try renderHostedView(MacMemoryCenterView(), label: "MacMemoryCenterView", colorScheme: .dark)
        try renderHostedView(MacMemoryCenterPage(), label: "MacMemoryCenterPage", colorScheme: .light)

        // 3b. Web Sources CRUD (MacWebSourceStore)
        let registry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("registry.json"),
            applicationPort: port
        )
        let webStoreURL = root.appendingPathComponent("web-sources.json")
        let webStore = MacWebSourceStore(storeURL: webStoreURL, registry: registry)

        let feedID = UUID()
        let jsonID = UUID()
        let feedSource = MacWebSource(
            id: feedID,
            name: "Alpha Release Feed",
            url: "https://releases.example.invalid/feed.xml",
            format: .rss,
            isEnabled: true,
            cachedText: "<rss>Release 2.0.0</rss>",
            lastFetchedAt: .now,
            lastError: nil,
            createdAt: .now,
            scheduleFrequency: .daily,
            scheduledHour: 9,
            scheduledMinute: 0
        )
        let jsonSource = MacWebSource(
            id: jsonID,
            name: "Beta Status JSON",
            url: "https://status.example.invalid/api/v1/health.json",
            format: .json,
            isEnabled: true,
            cachedText: #"{"status":"ok"}"#,
            lastFetchedAt: .now,
            lastError: nil,
            createdAt: .now,
            scheduleFrequency: .weekly,
            scheduledWeekday: 2,
            scheduledHour: 11,
            scheduledMinute: 30
        )
        webStore.upsert(feedSource)
        webStore.upsert(jsonSource)
        XCTAssertEqual(webStore.sources.count, 2)
        XCTAssertEqual(webStore.sources.map(\.name), ["Alpha Release Feed", "Beta Status JSON"])

        // Toggle & edit web source
        webStore.setEnabled(false, for: jsonSource)
        XCTAssertEqual(webStore.sources.first(where: { $0.id == jsonID })?.isEnabled, false)
        var editedFeed = feedSource
        editedFeed.name = "Alpha Release Feed v2"
        webStore.upsert(editedFeed)

        // Reopen MacWebSourceStore against same SQLite-backed registry
        let reopenedRegistry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("registry.json"),
            applicationPort: try MacApplicationPort(databaseURL: dbURL)
        )
        let reopenedWebStore = MacWebSourceStore(storeURL: webStoreURL, registry: reopenedRegistry)
        XCTAssertEqual(reopenedWebStore.sources.count, 2)
        XCTAssertEqual(reopenedWebStore.sources.first(where: { $0.id == feedID })?.name, "Alpha Release Feed v2")
        XCTAssertEqual(reopenedWebStore.sources.first(where: { $0.id == jsonID })?.isEnabled, false)

        // Delete web source & verify removal
        let targetToDelete = try XCTUnwrap(reopenedWebStore.sources.first(where: { $0.id == jsonID }))
        reopenedWebStore.delete(targetToDelete)
        XCTAssertEqual(reopenedWebStore.sources.count, 1)
        XCTAssertNil(reopenedWebStore.sources.first(where: { $0.id == jsonID }))

        // 3c. MacDataSourceRegistry validation freshness deduplication (NO write storms!)
        let observed = Date()
        registry.recordValidation(
            sourceID: "platform:calendar",
            connectorID: "eventkit.calendar",
            readerDescription: "EKEventStore.events(matching:)",
            schemaFields: ["eventIdentifier", "title", "startDate"],
            redactedSample: #"{"count":4}"#,
            recordCount: 4,
            maximumAgeMinutes: 30,
            observedAt: observed
        )
        let initialRev = try XCTUnwrap(try port.dataSource(id: "platform:calendar")?.revision)
        for _ in 0..<10 {
            registry.recordValidation(
                sourceID: "platform:calendar",
                connectorID: "eventkit.calendar",
                readerDescription: "EKEventStore.events(matching:)",
                schemaFields: ["eventIdentifier", "title", "startDate"],
                redactedSample: #"{"count":4}"#,
                recordCount: 4,
                maximumAgeMinutes: 30,
                observedAt: observed
            )
        }
        let afterRepeatedRev = try XCTUnwrap(try port.dataSource(id: "platform:calendar")?.revision)
        XCTAssertEqual(
            initialRev,
            afterRepeatedRev,
            "10 repeated identical recordValidation calls within freshness window must NOT increment SQLite revision"
        )

        // A genuinely changed recordCount MUST advance revision
        registry.recordValidation(
            sourceID: "platform:calendar",
            connectorID: "eventkit.calendar",
            readerDescription: "EKEventStore.events(matching:)",
            schemaFields: ["eventIdentifier", "title", "startDate"],
            redactedSample: #"{"count":7}"#,
            recordCount: 7,
            maximumAgeMinutes: 30,
            observedAt: observed
        )
        let afterChangeRev = try XCTUnwrap(try port.dataSource(id: "platform:calendar")?.revision)
        XCTAssertGreaterThan(afterChangeRev, afterRepeatedRev, "Changed recordCount must persist a new revision")

        // 3d. Per-conversation data source overrides (BOTH unsaved draft AND persisted conversation)
        let draftConvID = "draft-\(UUID().uuidString.lowercased())"
        registry.setConversationOverride(true, sourceID: "platform:calendar", conversationID: draftConvID)
        XCTAssertNil(registry.persistenceError)
        XCTAssertEqual(registry.conversationOverride(sourceID: "platform:calendar", conversationID: draftConvID), true)
        XCTAssertEqual(try port.dataSourceOverride(conversationID: draftConvID, sourceID: "platform:calendar"), .enabled)

        registry.setConversationOverride(false, sourceID: "platform:calendar", conversationID: draftConvID)
        XCTAssertEqual(try port.dataSourceOverride(conversationID: draftConvID, sourceID: "platform:calendar"), .disabled)

        let repo = MacCanonicalConversationRepository(store: port)
        let persistedConvID = "conv-ds-\(UUID().uuidString.lowercased())"
        try repo.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: persistedConvID,
                    title: "Data Source Override Thread",
                    preview: "Testing overrides",
                    updatedAt: .now,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Check my calendar events.", createdAt: .now)
                ]
            )
        ])
        registry.setConversationOverride(true, sourceID: "platform:calendar", conversationID: persistedConvID)
        XCTAssertEqual(registry.conversationOverride(sourceID: "platform:calendar", conversationID: persistedConvID), true)
        let displayOverrides = try await registry.loadOverridesForDisplay(
            conversationID: persistedConvID,
            sourceIDs: ["platform:calendar"]
        )
        XCTAssertEqual(displayOverrides["platform:calendar"], true)

        registry.setConversationOverride(nil, sourceID: "platform:calendar", conversationID: persistedConvID)
        XCTAssertNil(registry.conversationOverride(sourceID: "platform:calendar", conversationID: persistedConvID))
        XCTAssertEqual(try port.dataSourceOverride(conversationID: persistedConvID, sourceID: "platform:calendar"), .inherit)

        // 3e. Conversation Knowledge (key points & wiki read & save) via real MacWorkspaceModel
        let wm = try makeIsolatedWorkspaceModel(root: root, port: port)
        wm.selectedConversationID = persistedConvID
        wm.selectedKnowledgePointsText = "- Baseline: Nola 2.0.0\n- SQLite user_version = 2"
        wm.selectedWikiText = "## Architecture Notes\nUnified 8-crate Rust core with native macOS SwiftUI client."

        let knowledgeDeadline = ContinuousClock.now + .seconds(5)
        while wm.selectedKnowledgeHasPendingChanges && ContinuousClock.now < knowledgeDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertFalse(wm.selectedKnowledgeSaveFailed, "Knowledge save should succeed without error")
        XCTAssertFalse(wm.selectedKnowledgeHasPendingChanges, "Knowledge save should flush to canonical SQLite")

        let coldRepo = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: dbURL))
        let reloadedSidebar = try coldRepo.loadSidebar(eagerConversationID: persistedConvID)
        let reloadedKnowledge = try XCTUnwrap(reloadedSidebar.first(where: { $0.conversation.id == persistedConvID })?.knowledge)
        XCTAssertEqual(reloadedKnowledge.keyPointsMarkdown, "- Baseline: Nola 2.0.0\n- SQLite user_version = 2")
        XCTAssertEqual(reloadedKnowledge.wikiMarkdown, "## Architecture Notes\nUnified 8-crate Rust core with native macOS SwiftUI client.")

        // 3f. Render REAL Data Center & Web Sources UI views
        let platformData = MacPlatformDataService(registry: reopenedRegistry, startsPeriodicRefresh: false)
        let dcNav = MacDataCenterNavigationState()
        for section in [MacDataCenterSection.sources, .memory, .usage] {
            dcNav.section = section
            try renderHostedView(
                MacDataCenterView(navigation: dcNav, service: platformData, registry: reopenedRegistry, webSources: reopenedWebStore),
                label: "MacDataCenterView.\(section.rawValue)",
                colorScheme: .dark
            )
        }
        try renderHostedView(
            MacWebSourcesView(store: reopenedWebStore),
            label: "MacWebSourcesView",
            colorScheme: .light
        )
    }

    // MARK: - 4. Experts & Agent Rooms

    func testExpertsAndAgentRoomsFullLifecycleAndUI() throws {
        let root = try makeTempDirectory("experts-rooms")
        let dbURL = root.appendingPathComponent("rooms.db")
        let port = try MacApplicationPort(databaseURL: dbURL)
        let roomService = MacAgentRoomService(runtime: port)

        // 4a. Create 2 custom ExpertPeer personas, update & list them
        let dir0 = try roomService.directory()
        let dir1 = try roomService.createAgent(
            name: "Systems Architect",
            description: "Reviews Rust crate boundaries and FFI safety.",
            defaults: .object(["promptResponseStyle": .string("balanced")]),
            directory: dir0
        )
        let architect = try XCTUnwrap(dir1.agents.first(where: { $0.name == "Systems Architect" }))

        let dir2 = try roomService.createAgent(
            name: "Security Auditor",
            description: "Audits permissions, SSRF guards, and Keychain boundaries.",
            defaults: .object(["promptResponseStyle": .string("concise")]),
            directory: dir1
        )
        let auditor = try XCTUnwrap(dir2.agents.first(where: { $0.name == "Security Auditor" }))

        let dir3 = try roomService.updateAgent(
            architect,
            name: "Principal Systems Architect",
            description: "Reviews Rust 8-crate architecture and FFI safety.",
            availability: "active",
            directory: dir2
        )
        let updatedArchitect = try XCTUnwrap(dir3.agents.first(where: { $0.id == architect.id }))
        XCTAssertEqual(updatedArchitect.name, "Principal Systems Architect")
        XCTAssertGreaterThan(updatedArchitect.revision, architect.revision)

        let dir4 = try roomService.updateAgent(
            auditor,
            name: "Staff Security Auditor",
            description: "Audits permissions, SSRF, sandbox, and Keychain boundaries.",
            availability: "active",
            directory: dir3
        )
        let updatedAuditor = try XCTUnwrap(dir4.agents.first(where: { $0.id == auditor.id }))
        XCTAssertEqual(updatedAuditor.name, "Staff Security Auditor")
        XCTAssertGreaterThanOrEqual(dir4.agents.count, 2)

        // 4b. Verify readableTemplates & built-in roleTemplates
        struct ReadableTemplatesResponse: Decodable {
            let revision: Int64
            let templates: [MacCanonicalJSONValue]
        }
        let readable: ReadableTemplatesResponse = try port.executeExpertHost(.readableTemplates(authority: .local()))
        XCTAssertGreaterThanOrEqual(readable.revision, 1)
        let roleCatalog = try roomService.roleTemplates()
        XCTAssertFalse(roleCatalog.templates.isEmpty, "Built-in expert role templates should be available")

        // 4c. Create an AgentRoom, add/remove participants, post messages, load transcript
        let room0 = try roomService.create(title: "Nola 2.0 Release Room")
        XCTAssertEqual(room0.room.title, "Nola 2.0 Release Room")

        let room1 = try roomService.join(agent: updatedArchitect, room: room0)
        let room2 = try roomService.join(agent: updatedAuditor, room: room1)
        let activeParticipants = room2.room.participants.filter(\.active)
        XCTAssertEqual(activeParticipants.count, 2)
        let architectPart = try XCTUnwrap(activeParticipants.first(where: { $0.agentID == updatedArchitect.id }))
        let auditorPart = try XCTUnwrap(activeParticipants.first(where: { $0.agentID == updatedAuditor.id }))

        // Post messages directed to participants & read back transcript
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)
        let sub1 = try roomService.submit(
            MacRoomSubmissionInput(
                roomID: room2.room.id,
                submissionID: "sub-arch-\(UUID().uuidString.lowercased())",
                expectedRoomRevision: room2.revision,
                text: "Verify FFI ABI surface stability across all 8 crates.",
                participantID: architectPart.id,
                atMs: nowMs
            )
        )
        let sub2 = try roomService.submit(
            MacRoomSubmissionInput(
                roomID: room2.room.id,
                submissionID: "sub-sec-\(UUID().uuidString.lowercased())",
                expectedRoomRevision: room2.revision,
                text: "Audit MCP OAuth PKCE flow and SSRF blocklist.",
                participantID: auditorPart.id,
                atMs: nowMs + 10
            )
        )
        XCTAssertEqual(sub1.state, "selected")
        XCTAssertEqual(sub1.selectedParticipantID, architectPart.id)
        XCTAssertEqual(sub2.state, "selected")
        XCTAssertEqual(sub2.selectedParticipantID, auditorPart.id)

        let transcript = try roomService.submissions(roomID: room2.room.id)
        XCTAssertEqual(transcript.count, 2)
        XCTAssertTrue(transcript.contains(where: { $0.id == sub1.id && $0.input.text.contains("FFI ABI") }))
        XCTAssertTrue(transcript.contains(where: { $0.id == sub2.id && $0.input.text.contains("OAuth PKCE") }))
        let loadedSingle = try XCTUnwrap(try roomService.submission(roomID: room2.room.id, submissionID: sub1.id))
        XCTAssertEqual(loadedSingle.input.text, sub1.input.text)

        // Remove (deactivate) one participant
        let room3 = try roomService.deactivate(participantID: auditorPart.id, room: room2)
        let remainingActive = room3.room.participants.filter(\.active)
        XCTAssertEqual(remainingActive.count, 1)
        XCTAssertEqual(remainingActive.first?.id, architectPart.id)

        // 4d. Render REAL Expert & Room UI views
        let roomWorkspace = MacAgentRoomWorkspace(service: roomService)
        roomWorkspace.directory = dir4.agents
        roomWorkspace.rooms = [room3]
        let workspaceModel = try makeIsolatedWorkspaceModel(root: root, port: port)

        try renderHostedView(
            MacExpertDirectoryManagementView(workspace: roomWorkspace, showsCloseButton: false),
            label: "MacExpertDirectoryManagementView",
            colorScheme: .dark
        )
        try renderHostedView(
            MacChatExpertPicker(agents: dir4.agents, selection: .constant(Set([updatedArchitect.id]))),
            label: "MacChatExpertPicker",
            size: NSSize(width: 480, height: 520),
            colorScheme: .light
        )
        try renderHostedView(
            MacUnifiedExpertTranscript(model: workspaceModel, workspace: roomWorkspace, width: 760),
            label: "MacUnifiedExpertTranscript",
            colorScheme: .dark
        )

        // 4e. Delete the room & verify removal persists across cold reopen
        let deleteResult = try port.executeAgentRoomEnvelope(
            .object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("delete_room"),
                    "room_id": .string(room3.room.id)
                ])
            ])
        )
        if case .object(let fields) = deleteResult {
            XCTAssertEqual(fields["deleted"], .bool(true))
        } else {
            XCTFail("Expected JSON object from delete_room, got \(deleteResult)")
        }

        let reopenedRooms = MacAgentRoomService(runtime: try MacApplicationPort(databaseURL: dbURL))
        XCTAssertFalse(try reopenedRooms.list().contains(where: { $0.room.id == room3.room.id }))
        XCTAssertEqual(try reopenedRooms.directory().agents.count, dir4.agents.count)
    }

    // MARK: - 5. Conversation Management, Edit-and-Resend, Context Snapshot & Inspector Tabs

    func testConversationManagementEditResendContextAndInspectorTabsUI() async throws {
        let root = try makeTempDirectory("conversations-inspector")
        let dbURL = root.appendingPathComponent("conversations.db")
        let port = try MacApplicationPort(databaseURL: dbURL)
        let repo = MacCanonicalConversationRepository(store: port)
        let now = Date()

        // 5a. Seed 5 distinct persisted conversations with messages
        let specs: [(id: String, title: String, userMsg: String, assistantMsg: String, offsetSec: TimeInterval)] = [
            ("conv-01", "Quarterly Roadmap", "Draft Q4 roadmap milestones for Nola.", "Q4 milestones: SwiftUI polish, Rust core stability.", -500),
            ("conv-02", "SQLite Schema Audit", "Verify user_version = 2 WAL checkpoints.", "WAL checkpoints and FTS5 indices verified.", -400),
            ("conv-03", "MLX Quantization Notes", "Compare Qwen3.5 4B 4-bit memory footprint.", "Peak unified memory is ~7 GB at 64K context.", -300),
            ("conv-04", "MCP OAuth Flow", "Trace PKCE token exchange for HTTP MCP servers.", "PKCE S256 verifier validated cleanly.", -200),
            ("conv-05", "Release Checklist", "Confirm macOS notarization and export_public.py.", "All public export checks passed.", -100),
        ]
        try repo.restoreBackupSnapshot(
            specs.map { spec in
                MacStoredConversation(
                    conversation: MacConversation(
                        id: spec.id,
                        title: spec.title,
                        preview: spec.assistantMsg,
                        updatedAt: now.addingTimeInterval(spec.offsetSec),
                        isPinned: false,
                        isPersisted: true
                    ),
                    messages: [
                        MacChatMessage(role: .user, text: spec.userMsg, createdAt: now.addingTimeInterval(spec.offsetSec)),
                        MacChatMessage(role: .assistant, text: spec.assistantMsg, createdAt: now.addingTimeInterval(spec.offsetSec + 1)),
                    ]
                )
            }
        )

        let wm = try makeIsolatedWorkspaceModel(root: root, port: port)
        XCTAssertEqual(wm.conversations.count, 5)

        // 5b. Rename conversation & verify persisted in SQLite
        wm.renameConversation(id: "conv-02", to: "SQLite v2 Schema Audit (Verified)")
        XCTAssertEqual(wm.conversations.first(where: { $0.id == "conv-02" })?.title, "SQLite v2 Schema Audit (Verified)")
        let sqlSummary = try XCTUnwrap(try port.listConversations().first(where: { $0.id == "conv-02" }))
        XCTAssertEqual(sqlSummary.title, "SQLite v2 Schema Audit (Verified)")

        // 5c. Pin & unpin conversations and verify sort ordering
        let conv1 = try XCTUnwrap(wm.conversations.first(where: { $0.id == "conv-01" }))
        let conv3 = try XCTUnwrap(wm.conversations.first(where: { $0.id == "conv-03" }))
        wm.togglePinned(conv1)
        wm.togglePinned(conv3)
        XCTAssertTrue(wm.conversations[0].isPinned && wm.conversations[1].isPinned)
        XCTAssertEqual(Set(wm.conversations.prefix(2).map(\.id)), Set(["conv-01", "conv-03"]))

        // Unpin conv-01 -> only conv-03 stays pinned at index 0
        let pinnedConv1 = try XCTUnwrap(wm.conversations.first(where: { $0.id == "conv-01" }))
        wm.togglePinned(pinnedConv1)
        XCTAssertEqual(wm.conversations.first?.id, "conv-03")
        XCTAssertFalse(wm.conversations.first(where: { $0.id == "conv-01" })?.isPinned ?? true)

        // 5d. Search across conversations (sidebar filter + canonical FTS search)
        wm.searchText = "OAuth"
        XCTAssertEqual(wm.filteredConversations.map(\.id), ["conv-04"])
        wm.searchText = "SQLite v2"
        XCTAssertEqual(wm.filteredConversations.map(\.id), ["conv-02"])
        wm.searchText = ""
        XCTAssertEqual(wm.filteredConversations.count, 5)

        do {
            let ftsMatches = try port.searchConversationText("conv-03", query: "Qwen3.5", limit: 12)
            XCTAssertFalse(ftsMatches.isEmpty, "Canonical conversation recall search should match 'Qwen3.5'")
        } catch {
            XCTFail("STEP 5d searchConversationText failed: \(error)")
        }

        // 5e. Switch active conversation & verify selection + messages
        wm.selectedConversationID = "conv-04"
        XCTAssertEqual(wm.selectedConversation?.id, "conv-04")
        wm.selectedConversationID = "conv-05"
        XCTAssertEqual(wm.selectedConversation?.id, "conv-05")

        // 5f. Delete a conversation & verify removed from memory + SQLite
        let convToDelete = try XCTUnwrap(wm.conversations.first(where: { $0.id == "conv-01" }))
        wm.deleteConversation(convToDelete)
        let delDeadline = ContinuousClock.now + .seconds(5)
        while wm.conversations.contains(where: { $0.id == "conv-01" }) && ContinuousClock.now < delDeadline {
            try await Task.sleep(for: .milliseconds(40))
        }
        XCTAssertEqual(wm.conversations.count, 4)
        XCTAssertFalse(try port.listConversations().contains(where: { $0.id == "conv-01" }))

        // 5g. Edit-and-Resend preparation & readback (survives cold reopen)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)
        let opID = "edit-resend-\(UUID().uuidString.lowercased())"
        let branchConvID = "conv-branch-\(UUID().uuidString.lowercased())"
        do {
            let prepared = try port.prepareEditedResend(
                ApplePrepareEditedResend(
                    operation_id: opID,
                    source_conversation_id: "conv-05",
                    source_message_id: "msg-orig-1",
                    expected_source_revision: 1,
                    expected_source_message_revision: 1,
                    edited_text: "Confirm macOS notarization, export_public.py, and SHA-256 checksums.",
                    target_conversation_id: branchConvID,
                    title: "Release Checklist (Edited Branch)",
                    created_at_ms: nowMs
                )
            )
            XCTAssertEqual(prepared.run_id, opID)
            XCTAssertEqual(prepared.input.target_conversation_id, branchConvID)
            XCTAssertFalse(prepared.user_message_id.isEmpty)
            XCTAssertFalse(prepared.assistant_message_id.isEmpty)

            let reopenedPort = try MacApplicationPort(databaseURL: dbURL)
            let loadedPrepared = try XCTUnwrap(try reopenedPort.loadPreparedEditedResend(branchConvID))
            XCTAssertEqual(loadedPrepared.run_id, opID)
            XCTAssertEqual(loadedPrepared.input.edited_text, "Confirm macOS notarization, export_public.py, and SHA-256 checksums.")
        } catch {
            XCTFail("STEP 5g prepareEditedResend failed: \(error)")
        }

        // 5h. Context snapshot inspection & ALL Inspector tabs rendered cleanly
        await wm.refreshContextArchiveCount()
        do {
            let contextUsage = try port.loadConversationContextUsage("conv-05")
            XCTAssertNotNil(contextUsage)
        } catch {
            XCTFail("STEP 5h loadConversationContextUsage failed: \(error)")
        }

        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"), applicationPort: port)
        let platformData = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false)
        let webSources = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let inspectorDeps = MacInspectorDependencies(
            dataRegistry: registry,
            platformData: platformData,
            webSources: webSources,
            chromeMessaging: MacChromeMessagingService.shared
        )

        for tab in MacChatInspector.InspectorTab.allCases {
            let nav = MacInspectorNavigationState(selectedTab: tab)
            try renderHostedView(
                MacChatInspector(
                    model: wm,
                    hostDisplayName: "This Mac",
                    dependencies: inspectorDeps,
                    navigation: nav,
                    showsHeader: true,
                    showsHeaderNavigation: true
                ),
                label: "MacChatInspector.\(tab.rawValue)",
                size: NSSize(width: 380, height: 780),
                colorScheme: .dark
            )
        }
        // Also render Context tab with Knowledge Points & Wiki expanded
        for pane in [MacInspectorKnowledgePane.points, .wiki] {
            let nav = MacInspectorNavigationState(selectedTab: .context, revealedKnowledge: pane)
            try renderHostedView(
                MacChatInspector(
                    model: wm,
                    hostDisplayName: "This Mac",
                    dependencies: inspectorDeps,
                    navigation: nav,
                    showsHeader: true
                ),
                label: "MacChatInspector.context.\(pane.rawValue)",
                size: NSSize(width: 380, height: 780),
                colorScheme: .light
            )
        }
    }

    // MARK: - 6. Settings, Local Models, Backup/Restore & Onboarding

    func testSettingsLocalModelsBackupRestoreAndOnboardingUI() async throws {
        let root = try makeTempDirectory("settings-models-backup")
        let sourceDB = root.appendingPathComponent("source.db")
        let sourcePort = try MacApplicationPort(databaseURL: sourceDB)
        let sourceRepo = MacCanonicalConversationRepository(store: sourcePort)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // 6a. Populate full source state: conversations, automations, activity, settings, productSettings, webSources, notifications
        let convA = "backup-conv-a-\(UUID().uuidString.lowercased())"
        let convB = "backup-conv-b-\(UUID().uuidString.lowercased())"
        try sourceRepo.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: convA,
                    title: "Backup Conversation A",
                    preview: "Restored assistant answer A",
                    updatedAt: .now,
                    isPinned: true,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Prompt A for archive roundtrip", createdAt: .now),
                    MacChatMessage(role: .assistant, text: "Restored assistant answer A", createdAt: .now),
                ]
            ),
            MacStoredConversation(
                conversation: MacConversation(
                    id: convB,
                    title: "Backup Conversation B",
                    preview: "Restored assistant answer B",
                    updatedAt: .now,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Prompt B with Unicode: 架构验证 🚀", createdAt: .now),
                    MacChatMessage(role: .assistant, text: "Restored assistant answer B: 完整保留 ✅", createdAt: .now),
                ]
            ),
        ])

        // Product settings (assistant preferences + custom theme/locale/notifications/privacy redaction)
        MacAssistantPreferences.setMemoryEnabled(true, in: sourcePort)
        MacAssistantPreferences.setRetrievalEnabled(false, in: sourcePort)
        MacAssistantPreferences.setUserPreferences("Prefer terse Rust diffs and explicit SQLite assertions.", in: sourcePort)
        MacAssistantPreferences.setContextLimit(131_072, in: sourcePort)

        let customSettings: [(String, String)] = [
            ("ui.theme", #"{"mode":"dark"}"#),
            ("ui.locale", #"{"identifier":"zh-Hans"}"#),
            ("notifications.enabled", #"{"enabled":true,"frequency":"daily"}"#),
            ("privacy.redaction.enabled", #"{"enabled":true,"redactSecrets":true}"#),
        ]
        for (key, json) in customSettings {
            let stored = try sourcePort.putProductSetting(key: key, valueJSON: json, updatedAtMs: nowMs)
            XCTAssertEqual(stored.key, key)
            XCTAssertGreaterThan(stored.revision, 0)
        }
        // Verify CAS revision check on putProductSettingChecked
        let themeSetting = try XCTUnwrap(try sourcePort.productSetting(key: "ui.theme"))
        let updatedTheme = try sourcePort.putProductSettingChecked(
            key: "ui.theme",
            valueJSON: #"{"mode":"system"}"#,
            expectedRevision: themeSetting.revision,
            updatedAtMs: nowMs + 10
        )
        XCTAssertGreaterThan(updatedTheme.revision, themeSetting.revision)
        XCTAssertThrowsError(
            try sourcePort.putProductSettingChecked(
                key: "ui.theme",
                valueJSON: #"{"mode":"light"}"#,
                expectedRevision: themeSetting.revision,
                updatedAtMs: nowMs + 20
            ),
            "Stale product setting expectedRevision must be rejected"
        )

        let workspaceDir = root.appendingPathComponent("auto-ws", isDirectory: true)
        try FileManager.default.createDirectory(at: workspaceDir, withIntermediateDirectories: true)
        let backupAuto = MacAutomation(
            id: UUID(),
            name: "Backup Verified Daily Check",
            prompt: "Verify release artifacts daily.",
            frequency: .daily,
            weekday: 1,
            hour: 7,
            minute: 45,
            workspacePath: workspaceDir.path,
            isEnabled: true,
            createdAt: .now,
            contextSources: [],
            executionSpec: MacAutomationExecutionSpec(
                prompt: "Verify release artifacts daily.",
                executionScope: .localWorkspace(root: workspaceDir.path),
                permissionProfile: "cautious",
                modelSelection: "use_current_default_at_admission",
                contextRefs: [],
                skillRefs: []
            )
        )
        let backupWebSource = MacWebSource(
            id: UUID(),
            name: "Backup Status Feed",
            url: "https://backup-status.example.invalid/feed.json",
            format: .json,
            isEnabled: true,
            cachedText: #"{"healthy":true}"#,
            lastFetchedAt: .now,
            lastError: nil,
            createdAt: .now
        )
        let backupActivity = MacActivityEntry(
            id: UUID(),
            sourceID: convA,
            automationIdentity: nil,
            kind: .localAgent,
            status: .answered,
            title: "Backup Conversation A",
            detail: "Answered",
            conversationID: convA,
            startedAt: Date().addingTimeInterval(-30),
            finishedAt: .now,
            outputPreview: "Restored assistant answer A",
            logPath: nil
        )
        let backupNotification = MacNotificationInboxItem(
            identifier: "notif-backup-1",
            title: "Run Complete",
            body: "Daily check finished.",
            date: .now,
            deliveryState: .history,
            conversationID: convA,
            isRead: false
        )

        let canonicalA = try XCTUnwrap(try sourcePort.loadConversation(convA))
        let canonicalB = try XCTUnwrap(try sourcePort.loadConversation(convB))
        let backedUpProductSettings = try MacProductSettingKeys.backedUpKeys.compactMap {
            try sourcePort.productSetting(key: $0)
        }
        let backup = MacDataBackup(
            format: MacDataBackupCodec.format,
            version: MacDataBackupCodec.version,
            exportedAt: .now,
            conversations: [MacBackupConversation(canonicalA), MacBackupConversation(canonicalB)],
            activity: [backupActivity],
            automations: [backupAuto],
            settings: [
                MacBackupSetting(key: MacRunNotificationPreferenceKeys.agentCompletion, type: .boolean, value: "true")
            ],
            productSettings: backedUpProductSettings,
            webSources: [backupWebSource],
            notifications: [backupNotification]
        )

        // Encode -> Decode + invalid format/version rejection checks
        let encoded = try MacDataBackupCodec.encode(backup)
        let decoded = try MacDataBackupCodec.decode(encoded)
        XCTAssertEqual(decoded.conversations.count, 2)
        XCTAssertEqual(decoded.automations.count, 1)
        XCTAssertEqual(decoded.activity.count, 1)
        XCTAssertEqual(decoded.webSources.count, 1)
        XCTAssertEqual(decoded.settings.count, 1)
        XCTAssertEqual(decoded.notifications.count, 1)
        XCTAssertEqual(decoded.productSettings.count, backedUpProductSettings.count)

        let badFormatBackup = MacDataBackup(
            format: "com.invalid.other",
            version: 1,
            exportedAt: .now,
            conversations: [],
            activity: [],
            automations: [],
            settings: [],
            productSettings: [],
            webSources: [],
            notifications: []
        )
        XCTAssertThrowsError(try MacDataBackupCodec.decode(try MacDataBackupCodec.encode(badFormatBackup)))

        let badVersionBackup = MacDataBackup(
            format: MacDataBackupCodec.format,
            version: 99,
            exportedAt: .now,
            conversations: [],
            activity: [],
            automations: [],
            settings: [],
            productSettings: [],
            webSources: [],
            notifications: []
        )
        XCTAssertThrowsError(try MacDataBackupCodec.decode(try MacDataBackupCodec.encode(badVersionBackup)))

        // 6b. Restore EVERYTHING into a brand-new isolated SQLite DB & isolated stores
        let restoredRoot = root.appendingPathComponent("restored", isDirectory: true)
        try FileManager.default.createDirectory(at: restoredRoot, withIntermediateDirectories: true)
        let restoredDB = restoredRoot.appendingPathComponent("restored.db")
        let restoredPort = try MacApplicationPort(databaseURL: restoredDB)

        try MacArchiveRestoreWriter.restore(decoded.conversations, port: restoredPort)
        for setting in decoded.productSettings {
            _ = try restoredPort.putProductSetting(key: setting.key, valueJSON: setting.valueJSON, updatedAtMs: nowMs + 50)
        }
        let restoredRegistry = MacDataSourceRegistry(
            storeURL: restoredRoot.appendingPathComponent("registry.json"),
            applicationPort: restoredPort
        )
        let restoredWebStore = MacWebSourceStore(
            storeURL: restoredRoot.appendingPathComponent("web.json"),
            registry: restoredRegistry
        )
        try restoredWebStore.replaceAllThrowing(decoded.webSources)

        let restoredActivityStore = MacActivityStore(
            baseDirectory: restoredRoot.appendingPathComponent("Activity"),
            applicationPort: restoredPort
        )
        try restoredActivityStore.restoreThrowing(decoded.activity)

        let restoredEnv = MacAppEnvironment(
            applicationSupportRoot: restoredRoot.appendingPathComponent("Support"),
            cachesRoot: restoredRoot.appendingPathComponent("Caches")
        )
        let restoredAutomationStore = try MacAutomationStore(
            baseDirectory: restoredRoot.appendingPathComponent("Automations"),
            launchAgentDirectory: restoredRoot.appendingPathComponent("LaunchAgents"),
            synchronizeSystemJobs: false,
            appEnvironment: restoredEnv,
            authorizationResolver: { _ in true },
            activityStore: restoredActivityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: restoredRegistry,
            webSourceStore: restoredWebStore,
            automationClient: restoredPort,
            skillCatalogClient: restoredPort
        )
        try restoredAutomationStore.replaceAllThrowing(decoded.automations)

        let restoredNotifRepo = MacNotificationHistoryRepository(supportRoot: restoredRoot.appendingPathComponent("Support"))
        for notif in decoded.notifications {
            try await restoredNotifRepo.recordCommitted(notif)
        }

        // Cold reopen restored.db & verify 100% survival of ALL entities
        let verifyPort = try MacApplicationPort(databaseURL: restoredDB)
        let verifyRepo = MacCanonicalConversationRepository(store: verifyPort)
        let loadedA = try XCTUnwrap(try verifyRepo.load(conversationID: convA))
        let loadedB = try XCTUnwrap(try verifyRepo.load(conversationID: convB))
        XCTAssertEqual(loadedA.conversation.title, "Backup Conversation A")
        XCTAssertEqual(loadedA.messages.map(\.text), ["Prompt A for archive roundtrip", "Restored assistant answer A"])
        XCTAssertEqual(loadedB.conversation.title, "Backup Conversation B")
        XCTAssertEqual(loadedB.messages.map(\.text), ["Prompt B with Unicode: 架构验证 🚀", "Restored assistant answer B: 完整保留 ✅"])

        XCTAssertEqual(try verifyPort.listAutomations().count, 1)
        XCTAssertEqual(try verifyPort.listAutomations().first?.name, "Backup Verified Daily Check")
        XCTAssertEqual(MacAssistantPreferences.memoryEnabled(in: verifyPort), true)
        XCTAssertEqual(MacAssistantPreferences.retrievalEnabled(in: verifyPort), false)
        XCTAssertEqual(MacAssistantPreferences.userPreferences(in: verifyPort), "Prefer terse Rust diffs and explicit SQLite assertions.")
        XCTAssertEqual(MacAssistantPreferences.contextLimit(in: verifyPort), 131_072)

        let verifyRegistry = MacDataSourceRegistry(
            storeURL: restoredRoot.appendingPathComponent("registry.json"),
            applicationPort: verifyPort
        )
        let verifyWebStore = MacWebSourceStore(
            storeURL: restoredRoot.appendingPathComponent("web.json"),
            registry: verifyRegistry
        )
        XCTAssertEqual(verifyWebStore.sources.count, 1)
        XCTAssertEqual(verifyWebStore.sources.first?.name, "Backup Status Feed")

        let verifyActivity = MacActivityStore(
            baseDirectory: restoredRoot.appendingPathComponent("Activity"),
            applicationPort: verifyPort
        )
        XCTAssertEqual(verifyActivity.entries.count, 1)
        XCTAssertEqual(verifyActivity.entries.first?.conversationID, convA)
        let verifyNotifs = await restoredNotifRepo.items()
        XCTAssertEqual(verifyNotifs.count, 1)
        XCTAssertEqual(verifyNotifs.first?.identifier, "notif-backup-1")

        // 6c. Local Model Catalog discovery & parameter editing with CAS revision conflict detection
        XCTAssertEqual(MacLocalModelProfile.all.count, 2)
        XCTAssertTrue(MacLocalModelProfile.all.contains(where: { $0.id == MacLocalModelProfile.qwen35_08B.id }))
        XCTAssertTrue(MacLocalModelProfile.all.contains(where: { $0.id == MacLocalModelProfile.qwen35_4B.id }))

        // Verify custom installed MLX snapshot discovery in an isolated temp models root
        let modelsRoot = root.appendingPathComponent("models-root", isDirectory: true)
        let snapDir = modelsRoot.appendingPathComponent("custom-qwen-snap", isDirectory: true)
        try FileManager.default.createDirectory(at: snapDir, withIntermediateDirectories: true)
        let cfgBytes = Data(#"{"model_type":"qwen3"}"#.utf8)
        let weightsBytes = Data(repeating: 0x42, count: 128)
        try cfgBytes.write(to: snapDir.appendingPathComponent("config.json"))
        try weightsBytes.write(to: snapDir.appendingPathComponent("model.safetensors"))
        let manifestJSON = """
        {
          "schemaVersion": 1,
          "totalBytes": \(cfgBytes.count + weightsBytes.count),
          "artifacts": [
            {"path": "config.json", "bytes": \(cfgBytes.count)},
            {"path": "model.safetensors", "bytes": \(weightsBytes.count)}
          ]
        }
        """
        try Data(manifestJSON.utf8).write(to: snapDir.appendingPathComponent("snapshot.json"))
        let catalogJSON = """
        {
          "schemaVersion": 2,
          "models": [
            {
              "id": "local/custom-qwen-test",
              "name": "Custom Qwen Test",
              "runtime": "mlx",
              "format": "mlx",
              "state": "configured",
              "enabled": true,
              "contextWindowTokens": 131072,
              "artifactBytes": \(cfgBytes.count + weightsBytes.count),
              "snapshotPath": "\(snapDir.path)"
            }
          ]
        }
        """
        try Data(catalogJSON.utf8).write(to: modelsRoot.appendingPathComponent("catalog.json"))
        let discoveredInstalled = MacLocalModelProfile.installed(in: modelsRoot)
        XCTAssertEqual(discoveredInstalled.count, 1)
        XCTAssertEqual(discoveredInstalled.first?.id, "local/custom-qwen-test")
        XCTAssertEqual(discoveredInstalled.first?.nativeContextTokens, 131_072)

        // Parameter editing with CAS revision conflict detection
        let targetProfile = MacLocalModelProfile.qwen35_4B
        let snap0 = try MacLocalModelConfigurationStore.loadParameterSnapshot(for: targetProfile, applicationPort: verifyPort)
        XCTAssertEqual(snap0.revision, 0)

        var editedParams = snap0.parameters
        editedParams.temperature = 0.45
        editedParams.maxOutputTokens = 4_096
        editedParams.customSystemPrompt = "Be precise and cite file paths."
        let rev1 = try MacLocalModelConfigurationStore.persistParameters(
            editedParams,
            for: targetProfile,
            expectedRevision: snap0.revision,
            applicationPort: verifyPort
        )
        XCTAssertGreaterThan(rev1, snap0.revision)

        // Stale revision conflict MUST throw!
        XCTAssertThrowsError(
            try MacLocalModelConfigurationStore.persistParameters(
                editedParams,
                for: targetProfile,
                expectedRevision: snap0.revision,
                applicationPort: verifyPort
            ),
            "configureModelParameters with stale expectedRevision must throw a conflict error"
        )

        // Cold reopen verifies edited local model parameters persisted
        let coldVerifyPort = try MacApplicationPort(databaseURL: restoredDB)
        let snap1 = try MacLocalModelConfigurationStore.loadParameterSnapshot(for: targetProfile, applicationPort: coldVerifyPort)
        XCTAssertEqual(snap1.revision, rev1)
        XCTAssertEqual(snap1.parameters.temperature, 0.45, accuracy: 0.001)
        XCTAssertEqual(snap1.parameters.maxOutputTokens, 4_096)
        XCTAssertEqual(snap1.parameters.customSystemPrompt, "Be precise and cite file paths.")

        // 6d. Render REAL MacSettingsView, MacModelCenterView, and MacOnboardingView
        for category in ["general", "providers", "agent", "connections", "data", "diagnostics"] {
            try renderHostedView(
                MacSettingsView(snapshotHostState: .notConfigured, initialCategory: category),
                label: "MacSettingsView.\(category)",
                size: NSSize(width: 860, height: 640),
                colorScheme: category == "general" ? .light : .dark
            )
        }

        let modelCenterNav = MacModelCenterNavigationState(selectedTab: .local)
        let modelPref = MacDefaultChatModelPresentation(store: .init(application: coldVerifyPort))
        try renderHostedView(
            MacModelCenterView(
                remoteHost: E2ENoOpRemoteHost(),
                navigation: modelCenterNav,
                preference: modelPref
            ),
            label: "MacModelCenterView.local",
            colorScheme: .dark
        )
        modelCenterNav.selectedTab = .api
        try renderHostedView(
            MacModelCenterView(
                remoteHost: E2ENoOpRemoteHost(),
                navigation: modelCenterNav,
                preference: modelPref
            ),
            label: "MacModelCenterView.api",
            colorScheme: .light
        )

        try renderHostedView(
            MacOnboardingView(isHelpWindow: false, onComplete: {}),
            label: "MacOnboardingView.dark",
            size: NSSize(width: 680, height: 680),
            colorScheme: .dark
        )
        try renderHostedView(
            MacOnboardingView(isHelpWindow: true, onComplete: {}),
            label: "MacOnboardingView.light",
            size: NSSize(width: 680, height: 680),
            colorScheme: .light
        )
    }
}
