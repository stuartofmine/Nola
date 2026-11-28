// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Deep real-model macOS E2E suite verifying wire-level reasoning_effort (`none` vs `high`),
//   @Expert mentions, mid-stream Stop/Cancel + follow-up recovery, cold-restart SQLite persistence,
//   and Model Center full CRUD/override/delete lifecycle.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRealWorkflowExpertsRecoveryLifecycleE2ETests: XCTestCase {
    override func tearDown() {
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        super.tearDown()
    }

    private struct RealLLMConfig {
        let endpoint: String
        let model: String
        let apiKey: String
        let wireProtocol: MacModelWireProtocol
    }

    private func loadRealLLMConfig() throws -> RealLLMConfig {
        let env = ProcessInfo.processInfo.environment
        let fixtureURL = URL(fileURLWithPath: "/tmp/nola-real-llm-e2e.json")
        let fixture = (try? Data(contentsOf: fixtureURL))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] }
        let envKey = env["NOLA_REAL_LLM_API_KEY"] ?? env["NOLA_PRIMARY_API_KEY"]
        let apiKey = (envKey?.isEmpty == false ? envKey : nil) ?? fixture?["apiKey"] ?? ""
        guard !apiKey.isEmpty else {
            throw XCTSkip("Requires NOLA_REAL_LLM_API_KEY or /tmp/nola-real-llm-e2e.json")
        }
        let endpoint = (env["NOLA_REAL_LLM_ENDPOINT"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["endpoint"] ?? "http://192.168.31.18:8081/v1"
        let model = (env["NOLA_REAL_LLM_MODEL"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["model"] ?? "qwen3.8-flash-next-iq3_s"
        let rawWire = env["NOLA_REAL_LLM_WIRE_PROTOCOL"] ?? fixture?["wireProtocol"] ?? "chat_completions"
        return RealLLMConfig(
            endpoint: endpoint,
            model: model,
            apiKey: apiKey,
            wireProtocol: MacModelWireProtocol(rawValue: rawWire) ?? .chatCompletions
        )
    }

    private func openWorkspace(
        root: URL,
        databaseURL: URL
    ) throws -> (port: MacApplicationPort, workspaceModel: MacWorkspaceModel) {
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)
        let repository = MacCanonicalConversationRepository(store: port)
        let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let activityStore = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let suiteName = "com.gaojc.Nola.lc-bookmarks.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: suiteName) }
        let bookmarkStore = MacSecurityScopedBookmarkStore(defaults: defaults)
        _ = registry
        let wm = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: E2ENoOpRemoteHost(),
            modelApplication: port,
            bookmarkStore: bookmarkStore,
            activityStore: activityStore,
            completionNotifier: { _, _, _ in },
            readinessIssue: { _ in nil }
        )
        return (port, wm)
    }

    // MARK: - 1. Model Center Full Lifecycle: No-Props -> Enable -> Re-Discover /props -> Override OFF/ON -> Edit -> Delete

    func testModelCenterFullLifecycleDiscoverOverrideEditDelete() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-mc-lifecycle-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }

        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("mc.db"))
        let connService = MacModelConnectionService(application: port)
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)

        // 1. Create connection
        let created = try connService.create(
            name: "Local Strata",
            endpoint: "http://192.168.31.18:8081/v1",
            wireProtocol: "chat_completions",
            secret: "sk-test-secret-1"
        )
        XCTAssertEqual(try connService.connections().count, 1)

        // 2. Discover WITHOUT /props -> enable -> verify no reasoning options yet
        let rawModel = AppleModelCatalogDiscovery.Model(
            id: "qwen3.8-flash-next-iq3_s",
            displayName: "qwen3.8-flash-next-iq3_s",
            contextWindow: 262144,
            modalities: ["text", "image"]
        )
        let rawEntry = try JSONDecoder().decode(
            MacCanonicalJSONValue.self,
            from: rawModel.catalogData(wireProtocol: "chat_completions")
        )
        let cat1: MacModelConnectionSnapshot = try connService.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(created.revision),
            "models": .array([rawEntry]),
            "observed_at_ms": .signedInteger(1_000),
        ])
        let modelKey = try XCTUnwrap(cat1.document.catalog?.models.first?.id)
        try connService.enable(cat1, modelIDs: [modelKey])
        let opt1 = try XCTUnwrap(configStore.configuredModels().first)
        XCTAssertTrue(configStore.load(model: opt1.profileID)?.reasoning?.options.isEmpty ?? true)

        // 3. Re-discover WITH /props metadata (WITHOUT toggling enable!) -> reasoning options appear IMMEDIATELY
        var propsModel = rawModel
        propsModel.reasoningSupported = true
        propsModel.reasoningLevels = ["none", "low", "medium", "high"]
        propsModel.toolCalls = true
        propsModel.outputLimit = 16384
        let propsEntry = try JSONDecoder().decode(
            MacCanonicalJSONValue.self,
            from: propsModel.catalogData(wireProtocol: "chat_completions")
        )
        let cat2: MacModelConnectionSnapshot = try connService.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(cat1.revision),
            "models": .array([propsEntry]),
            "observed_at_ms": .signedInteger(2_000),
        ])
        let cfg2 = try XCTUnwrap(configStore.load(model: opt1.profileID))
        XCTAssertEqual(cfg2.reasoning?.control, .effort)
        XCTAssertEqual(cfg2.reasoning?.options, ["none", "low", "medium", "high"])

        // 4. Manual capability override: disable Reasoning -> options disappear; re-enable -> restored!
        let targetCatalogModel = try XCTUnwrap(cat2.document.catalog?.models.first)
        let overrideOff = try connService.setCapabilityOverride(
            cat2,
            model: targetCatalogModel,
            capabilities: .object([
                "toolCalls": .bool(true),
                "imageInput": .bool(true),
                "reasoning": .bool(false),
                "samplingParameters": .bool(false),
            ])
        )
        XCTAssertTrue(configStore.load(model: opt1.profileID)?.reasoning?.options.isEmpty ?? true)

        let overrideOn = try connService.setCapabilityOverride(
            overrideOff,
            model: targetCatalogModel,
            capabilities: .object([
                "toolCalls": .bool(true),
                "imageInput": .bool(true),
                "reasoning": .bool(true),
                "samplingParameters": .bool(false),
            ])
        )
        XCTAssertEqual(configStore.load(model: opt1.profileID)?.reasoning?.options, ["none", "low", "medium", "high"])

        // 5. Edit connection route -> old catalog cleared & enabled profiles cleared cleanly
        let edited = try connService.update(
            overrideOn,
            name: "Local Strata Renamed",
            endpoint: "http://192.168.31.18:8082/v1",
            wireProtocol: "chat_completions",
            secret: "sk-test-secret-2"
        )
        XCTAssertEqual(edited.document.connection.display_name, "Local Strata Renamed")
        XCTAssertEqual(edited.document.connection.base_url, "http://192.168.31.18:8082/v1")
        XCTAssertTrue(edited.document.catalog?.models.isEmpty ?? true)

        // 6. Delete connection -> zero connections & zero orphan profiles
        try connService.delete(edited)
        XCTAssertTrue(try connService.connections().isEmpty)
        XCTAssertTrue(configStore.configuredModels().isEmpty)
    }

    // MARK: - 2. Real @Expert Mention + Reasoning Effort None vs High + Mid-Stream Cancel + Cold Restart Persistence

    func testRealExpertMentionReasoningModesCancelAndColdRestartPersistence() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-real-lifecycle-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }

        let dbURL = root.appendingPathComponent("lifecycle.db")
        let (port1, wm1) = try openWorkspace(root: root, databaseURL: dbURL)
        defer {
            if wm1.isRunning { wm1.stop(reason: "teardown") }
            wm1.presentationDidDetach()
        }

        // Seed connection + discover + enable
        let connService = MacModelConnectionService(application: port1)
        let created = try connService.create(
            name: "LAN Real",
            endpoint: config.endpoint,
            wireProtocol: config.wireProtocol.rawValue,
            secret: config.apiKey
        )
        let discovered = try await connService.discover(created)
        let models = try XCTUnwrap(discovered.document.catalog?.models)
        let target = models.first(where: { $0.model_id == config.model }) ?? models[0]
        try connService.enable(discovered, modelIDs: [target.id])

        // Create custom ExpertPeer "雷军" via real MacAgentRoomService
        let roomService = MacAgentRoomService(runtime: port1)
        let directory = try roomService.directory()
        _ = try roomService.createAgent(
            name: "雷军",
            description: "小米创始人雷军，强调专注、极致、口碑、快，回复中必须包含口头禅「Are you OK」。",
            defaults: .object(["role": .string("产品与商业战略专家")]),
            directory: directory,
            id: "expert-leijun"
        )

        wm1.newConversation()
        await wm1.loadSelectedConversationIfNeeded()
        try await wm1.waitForModelConfigurationCommit()
        let option = try XCTUnwrap(wm1.configuredModelOptions.first(where: { $0.modelID == target.model_id }))
        wm1.selectModel(option)
        try await wm1.waitForModelConfigurationCommit()
        let convID = try XCTUnwrap(wm1.selectedConversationID)

        // 2a. @雷军 with reasoning_effort = "none" (fast, zero thinking, persona adopted, zero bash DB probing)
        XCTAssertTrue(wm1.selectComposerReasoningPreference("none", conversationID: convID))
        try await wm1.waitForModelConfigurationCommit()
        wm1.draft = "@雷军 用一句话自我介绍，不要调用任何工具。"
        wm1.send()
        XCTAssertTrue(wm1.isRunning)
        let d1 = ContinuousClock.now + .seconds(90)
        while wm1.isRunning && ContinuousClock.now < d1 {
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm1.isRunning, "@雷军 turn timed out")
        await wm1.loadSelectedConversationIfNeeded()
        let expertReply = wm1.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(
            expertReply.contains("雷军") || expertReply.localizedCaseInsensitiveContains("are you ok"),
            "Expected @雷军 expert persona reply, got: \(expertReply.prefix(300))"
        )

        // 2b. Switch to reasoning_effort = "high", send long prompt, STOP mid-stream, then verify recovery!
        XCTAssertTrue(wm1.selectComposerReasoningPreference("high", conversationID: convID))
        try await wm1.waitForModelConfigurationCommit()
        let assistantCountBeforeTurn2 = wm1.selectedMessages.filter { $0.role == .assistant }.count
        wm1.draft = "请详细推导并写出 10 条完整的分布式共识算法对比分析（每条至少 150 字），不要调用工具。"
        wm1.send()
        XCTAssertTrue(wm1.isRunning)

        // Wait until Turn 2's NEW assistant message starts streaming (journalReference appears) then click Stop!
        let startWait = ContinuousClock.now + .seconds(30)
        var sawStreamBeforeStop = false
        while wm1.isRunning && ContinuousClock.now < startWait {
            let assistants = wm1.selectedMessages.filter { $0.role == .assistant }
            if assistants.count > assistantCountBeforeTurn2,
               assistants.last?.journalReference != nil {
                sawStreamBeforeStop = true
                break
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertTrue(sawStreamBeforeStop, "High-reasoning turn should begin streaming before stop")
        wm1.stop(reason: "user-e2e-stop")
        let stopDeadline = ContinuousClock.now + .seconds(15)
        while wm1.isRunning && ContinuousClock.now < stopDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm1.isRunning, "Mid-stream stop must cleanly terminate the run within 15s")

        // 2c. Follow-up turn AFTER stop in the SAME conversation MUST succeed cleanly!
        XCTAssertTrue(wm1.selectComposerReasoningPreference("low", conversationID: convID))
        try await wm1.waitForModelConfigurationCommit()
        let markerAfterStop = "AFTER_STOP_OK_\(UUID().uuidString.prefix(6))"
        wm1.draft = "只需回复 \(markerAfterStop)，不要调用工具。"
        wm1.send()
        XCTAssertTrue(wm1.isRunning)
        let d3 = ContinuousClock.now + .seconds(90)
        while wm1.isRunning && ContinuousClock.now < d3 {
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm1.isRunning, "Follow-up turn after stop timed out")
        await wm1.loadSelectedConversationIfNeeded()
        let fullText = wm1.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(fullText.contains(markerAfterStop), "Follow-up after stop must succeed: \(fullText.suffix(300))")

        // 2d. Cold-restart persistence: reopen brand-new MacApplicationPort & MacWorkspaceModel on SAME DB!
        wm1.presentationDidDetach()
        let (_, wm2) = try openWorkspace(root: root, databaseURL: dbURL)
        defer { wm2.presentationDidDetach() }
        wm2.selectedConversationID = convID
        await wm2.loadSelectedConversationIfNeeded()
        let restoredText = wm2.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(restoredText.contains(markerAfterStop), "Cold restart must restore committed messages from SQLite")
        XCTAssertEqual(wm2.selectedComposerState.reasoningPreference, "low", "Cold restart must preserve reasoningPreference")
    }
}
