// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Real macOS production UI E2E suite driving the real LAN model and capturing PNG evidence across all core macOS views.

import AppKit
import Foundation
import SQLite3
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRealModelE2ETests: XCTestCase {
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
            throw XCTSkip("Requires NOLA_REAL_LLM_API_KEY, NOLA_PRIMARY_API_KEY, or /tmp/nola-real-llm-e2e.json")
        }

        let endpoint = (env["NOLA_REAL_LLM_ENDPOINT"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["endpoint"]
            ?? "http://192.168.31.18:8081/v1"
        let model = (env["NOLA_REAL_LLM_MODEL"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["model"]
            ?? "qwen3.8-flash-next-iq3_s"
        let rawWire = env["NOLA_REAL_LLM_WIRE_PROTOCOL"] ?? fixture?["wireProtocol"] ?? "chat_completions"
        let wireProtocol = MacModelWireProtocol(rawValue: rawWire) ?? .chatCompletions
        return RealLLMConfig(endpoint: endpoint, model: model, apiKey: apiKey, wireProtocol: wireProtocol)
    }

    private func evidenceDirectory() throws -> URL {
        let env = ProcessInfo.processInfo.environment
        let base = env["NOLA_E2E_EVIDENCE_DIR"].flatMap { $0.isEmpty ? nil : $0 }
            ?? "/tmp/nola-real-e2e-evidence"
        let dir = URL(fileURLWithPath: base, isDirectory: true).appendingPathComponent("macos", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testRealModelChatAndProductionViewsScreenshots() async throws {
        let config = try loadRealLLMConfig()
        let evidenceDir = try evidenceDirectory()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-real-e2e-\(UUID().uuidString)", isDirectory: true)
        let workspaceDir = root.appendingPathComponent("workspace", isDirectory: true)
        try FileManager.default.createDirectory(at: workspaceDir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }

        let reportMarker = "NOLA_MAC_E2E_REPORT_\(UUID().uuidString.prefix(8))"
        try "# Project Report\n\nUnique marker: \(reportMarker)\n".write(
            to: workspaceDir.appendingPathComponent("report.md"),
            atomically: true,
            encoding: .utf8
        )

        let databaseURL = root.appendingPathComponent("nola-mac-e2e.db")
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)

        // Real user API connection flow: create -> discover (/v1/models) -> enable
        let connectionService = MacModelConnectionService(application: port)
        let createdConnection = try connectionService.create(
            name: "LAN Real Model",
            endpoint: config.endpoint,
            wireProtocol: config.wireProtocol.rawValue,
            secret: config.apiKey
        )
        let discoveredConnection = try await connectionService.discover(createdConnection)
        let discoveredModels = try XCTUnwrap(
            discoveredConnection.document.catalog?.models,
            "Real LAN server discovery should populate connection catalog"
        )
        XCTAssertFalse(discoveredModels.isEmpty, "Discovered catalog from real LAN server must be non-empty")
        let selectedCatalogModel = discoveredModels.first(where: { $0.model_id == config.model }) ?? discoveredModels[0]
        let connectionForEnable: MacModelConnectionSnapshot
        if selectedCatalogModel.execution_capabilities == nil {
            let capabilities: MacCanonicalJSONValue = .object([
                "toolCalls": .bool(true),
                "imageInput": .bool(false),
                "reasoning": .bool(true),
                "samplingParameters": .bool(false),
            ])
            connectionForEnable = try connectionService.setCapabilityOverride(
                discoveredConnection,
                model: selectedCatalogModel,
                capabilities: capabilities
            )
        } else {
            connectionForEnable = discoveredConnection
        }
        let catalogKey = selectedCatalogModel.id
        try connectionService.enable(connectionForEnable, modelIDs: [catalogKey])

        let loadedCatalog = try MacChatModelCatalog.load(application: port)
        XCTAssertTrue(
            loadedCatalog.entries.contains(where: {
                $0.option.source == .configured && $0.option.modelID == selectedCatalogModel.model_id
            }),
            "MacChatModelCatalog.load should expose newly enabled API model \(selectedCatalogModel.model_id)"
        )

        let repository = MacCanonicalConversationRepository(store: port)
        let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
        let remoteHost = E2ENoOpRemoteHost()
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let webStore = MacWebSourceStore(storeURL: root.appendingPathComponent("web-sources.json"), registry: registry)
        let activityStore = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let appEnv = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("Support"),
            cachesRoot: root.appendingPathComponent("Caches")
        )
        let automationStore = try MacAutomationStore(
            baseDirectory: root.appendingPathComponent("Automations"),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"),
            synchronizeSystemJobs: false,
            appEnvironment: appEnv,
            authorizationResolver: { _ in true },
            activityStore: activityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: registry,
            webSourceStore: webStore,
            automationClient: port,
            skillCatalogClient: port
        )
        let sampleSpec = MacAutomationExecutionSpec(
            prompt: "Review report.md and summarize key milestones.",
            executionScope: .localWorkspace(root: workspaceDir.path),
            permissionProfile: "cautious",
            modelSelection: "use_current_default_at_admission",
            contextRefs: [],
            skillRefs: []
        )
        _ = try automationStore.upsertWithVerification(
            MacAutomation(
                id: UUID(),
                name: "Morning Workspace Digest",
                prompt: sampleSpec.prompt,
                frequency: .daily,
                weekday: 1,
                hour: 9,
                minute: 0,
                workspacePath: workspaceDir.path,
                isEnabled: true,
                createdAt: .now,
                contextSources: [],
                executionSpec: sampleSpec
            )
        )

        let bookmarkSuiteName = "com.gaojc.Nola.real-e2e-bookmarks.\(UUID().uuidString)"
        let bookmarkDefaults = try XCTUnwrap(UserDefaults(suiteName: bookmarkSuiteName))
        addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: bookmarkSuiteName) }
        let bookmarkStore = MacSecurityScopedBookmarkStore(defaults: bookmarkDefaults)

        let workspaceModel = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: remoteHost,
            modelApplication: port,
            bookmarkStore: bookmarkStore,
            activityStore: activityStore,
            completionNotifier: { _, _, _ in },
            readinessIssue: { _ in nil }
        )
        let inspectorDependencies = MacInspectorDependencies(
            dataRegistry: registry,
            platformData: MacPlatformDataService.shared,
            webSources: webStore,
            chromeMessaging: MacChromeMessagingService.shared
        )

        // 1. Real MacChatWorkspaceView: send real prompt to real LAN model & capture reply
        let chatView = MacChatWorkspaceView(
            forcesInspectorVisible: false,
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: remoteHost,
            inspectorDependencies: inspectorDependencies,
            model: workspaceModel
        )
        let (chatWindow, chatHosting) = makeHostWindow(chatView)
        defer {
            if workspaceModel.isRunning { workspaceModel.stop(reason: "real-e2e-cleanup") }
            workspaceModel.presentationDidDetach()
            chatWindow.orderOut(nil)
            chatWindow.contentView = nil
            chatWindow.close()
        }

        await settle(chatHosting)
        workspaceModel.newConversation()
        await workspaceModel.loadSelectedConversationIfNeeded()
        try await workspaceModel.waitForModelConfigurationCommit()
        let configuredOption = try XCTUnwrap(
            workspaceModel.configuredModelOptions.first(where: { $0.modelID == selectedCatalogModel.model_id })
                ?? workspaceModel.configuredModelOptions.first,
            "workspaceModel.configuredModelOptions should include newly enabled API model \(selectedCatalogModel.model_id)"
        )
        XCTAssertEqual(configuredOption.modelID, selectedCatalogModel.model_id)
        XCTAssertEqual(MacPermissionMode.allCases, [.cautious, .allowAll])
        workspaceModel.selectModel(configuredOption)
        try await workspaceModel.waitForModelConfigurationCommit()
        await settle(chatHosting)
        XCTAssertFalse(
            workspaceModel.reasoningConfiguration.options.isEmpty,
            "Composer should expose reasoning effort options for reasoning-capable model \(selectedCatalogModel.model_id)"
        )

        // Verify reasoning effort switching is optimistic with ZERO composer jump (no submissionBlockReason flash)
        let convIDForSwitch = try XCTUnwrap(workspaceModel.selectedConversationID)
        XCTAssertTrue(workspaceModel.selectComposerReasoningPreference("high", conversationID: convIDForSwitch))
        XCTAssertEqual(workspaceModel.selectedComposerState.reasoningPreference, "high")
        XCTAssertNil(
            workspaceModel.selectedComposerState.submissionBlockReason,
            "Switching reasoning effort must NOT flash a submissionBlockReason banner below the composer"
        )
        try await workspaceModel.waitForModelConfigurationCommit()
        await settle(chatHosting)

        let expectedMarker = "MAC_REAL_MODEL_OK"
        workspaceModel.draft = "用简短的中文回复，用 Markdown 列出 2 条 Nola 本地优先架构的优点，并包含一个行内公式 $p = n_s + n_t$ 和一个独立公式块：$$S_s = \\lambda S_t + UV^\\top,\\quad \\lambda = D_s/D_t$$，结尾必须包含 \(expectedMarker)。不要调用任何工具。"
        workspaceModel.send()
        XCTAssertTrue(workspaceModel.isRunning, "Real model send did not start: activity=\(workspaceModel.activity)")

        var observedMidStreamJournal = false
        var capturedMidStreamScreenshot = false
        let deadline = ContinuousClock.now + .seconds(120)
        while workspaceModel.isRunning && ContinuousClock.now < deadline {
            if let lastAssistant = workspaceModel.selectedMessages.last(where: { $0.role == .assistant }),
               lastAssistant.journalReference != nil {
                observedMidStreamJournal = true
                if !capturedMidStreamScreenshot {
                    capturedMidStreamScreenshot = true
                    await settle(chatHosting)
                    try? capturePNG(chatHosting, to: evidenceDir.appendingPathComponent("mac-01a-mid-stream-thinking.png"))
                }
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(workspaceModel.isRunning, "Real LAN model turn timed out after 120s")
        XCTAssertTrue(
            observedMidStreamJournal,
            "Live reasoning/text journal publications MUST stream to the UI while the turn is running, not only at turn completion"
        )
        await workspaceModel.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .seconds(AppleStreamingRevealTiming.duration + 0.2))
        await settle(chatHosting)

        let assistantReplies = workspaceModel.selectedMessages.filter { $0.role == .assistant }
        let replyText = assistantReplies.map(\.text).joined(separator: "\n")
        var repoDiag = "no-id"
        if let cid = workspaceModel.selectedConversationID {
            do {
                let loaded = try repository.load(conversationID: cid)
                let summary = loaded?.messages.map { "\($0.role.rawValue):len=\($0.text.count):j=\($0.journalReference != nil)" } ?? ["nil"]
                repoDiag = "loaded=\(summary)"
            } catch {
                repoDiag = "loadErr=\(error)"
            }
        }
        var dbDiag: [String] = []
        var dbPtr: OpaquePointer?
        if sqlite3_open_v2(databaseURL.path, &dbPtr, SQLITE_OPEN_READONLY, nil) == SQLITE_OK {
            defer { sqlite3_close(dbPtr) }
            var stmt: OpaquePointer?
            if sqlite3_prepare_v2(dbPtr, "SELECT key, substr(value_json, 1, 300) FROM kv WHERE key LIKE 'run_terminal:%' OR key LIKE 'journal_head:%'", -1, &stmt, nil) == SQLITE_OK {
                defer { sqlite3_finalize(stmt) }
                while sqlite3_step(stmt) == SQLITE_ROW {
                    let k = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
                    let v = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
                    dbDiag.append("\(k)=\(v)")
                }
            }
            var evStmt: OpaquePointer?
            if sqlite3_prepare_v2(dbPtr, "SELECT kind, substr(payload_json, 1, 160) FROM events ORDER BY seq", -1, &evStmt, nil) == SQLITE_OK {
                defer { sqlite3_finalize(evStmt) }
                while sqlite3_step(evStmt) == SQLITE_ROW {
                    let k = sqlite3_column_text(evStmt, 0).map { String(cString: $0) } ?? ""
                    let v = sqlite3_column_text(evStmt, 1).map { String(cString: $0) } ?? ""
                    dbDiag.append("ev:\(k)=\(v)")
                }
            }
        }
        let diagnostics = "msgs=\(workspaceModel.selectedMessages.count) \(repoDiag) kv=\(dbDiag.joined(separator: " | ")) activity=\(workspaceModel.activity.map { "\($0.title):\($0.detail):\($0.state)" }) subFail=\(String(describing: workspaceModel.localSubmissionFailure)) rec=\(String(describing: workspaceModel.localRunRecovery?.title)) recErr=\(String(describing: workspaceModel.localRecoveryError))"
        XCTAssertFalse(replyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, "Real model returned an empty reply (\(diagnostics))")
        XCTAssertTrue(replyText.contains(expectedMarker), "Expected real reply to contain \(expectedMarker), got: \(replyText.prefix(200)) (\(diagnostics))")

        try capturePNG(chatHosting, to: evidenceDir.appendingPathComponent("mac-01-chat-real-model.png"))

        // 1b. Real Permission Upgrade (Cautious -> AllowAll on resume) + Tool-Calling Turn:
        // Turn 1 ran under Cautious; switching to AllowAll MUST allow bash execution with ZERO approval prompts!
        let boundWorkspace = workspaceModel.switchWorkspace(to: workspaceDir)
        XCTAssertNotNil(boundWorkspace, "switchWorkspace(to:) should bind workspaceDir for real tool execution")
        workspaceModel.permissionMode = .allowAll
        try await workspaceModel.waitForModelConfigurationCommit()
        await settle(chatHosting)

        let assistantCountBeforeToolTurn = workspaceModel.selectedMessages.filter { $0.role == .assistant }.count
        workspaceModel.draft = "请使用 bash 工具执行 `cat report.md`，并原样输出其中的 Unique marker 值。不要猜测，必须通过 bash 命令读取。"
        workspaceModel.send()
        XCTAssertTrue(workspaceModel.isRunning, "Real tool-calling send did not start: activity=\(workspaceModel.activity)")

        var unexpectedPermissionPromptCount = 0
        let toolDeadline = ContinuousClock.now + .seconds(150)
        while workspaceModel.isRunning && ContinuousClock.now < toolDeadline {
            if let pending = workspaceModel.pendingRemotePermission, !workspaceModel.isResolvingPermission {
                unexpectedPermissionPromptCount += 1
                workspaceModel.resolvePermission(
                    allow: true,
                    scope: .conversation,
                    requestID: pending.id,
                    expectedRequest: pending
                )
            }
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(workspaceModel.isRunning, "Real tool-calling turn timed out after 150s")
        XCTAssertEqual(
            unexpectedPermissionPromptCount,
            0,
            "Switching conversation from Cautious to AllowAll must take effect immediately on resume with ZERO permission prompts"
        )
        await workspaceModel.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .seconds(AppleStreamingRevealTiming.duration + 0.2))
        await settle(chatHosting)

        let allAssistantAfterToolTurn = workspaceModel.selectedMessages.filter { $0.role == .assistant }
        let toolTurnReplies = Array(allAssistantAfterToolTurn.dropFirst(assistantCountBeforeToolTurn))
        let toolReplyText = (toolTurnReplies.isEmpty ? allAssistantAfterToolTurn : toolTurnReplies)
            .map(\.text)
            .joined(separator: "\n")
        XCTAssertTrue(
            toolReplyText.contains(reportMarker),
            "Expected real tool-calling reply to contain \(reportMarker) read from report.md, got: \(toolReplyText.prefix(400))"
        )
        try capturePNG(chatHosting, to: evidenceDir.appendingPathComponent("mac-09-chat-real-tool.png"))

        // 2. Real MacChatWorkspaceView with Inspector expanded
        let inspectorView = MacChatWorkspaceView(
            forcesInspectorVisible: true,
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: remoteHost,
            inspectorDependencies: inspectorDependencies,
            model: workspaceModel
        )
        try await renderAndCapture(inspectorView, to: evidenceDir.appendingPathComponent("mac-02-inspector.png"))

        // 3. End-to-end Codex import bridge (preview -> commit with Keychain handshake) + MacModelCenterView
        let importDir = root.appendingPathComponent("codex-import", isDirectory: true)
        try FileManager.default.createDirectory(at: importDir, withIntermediateDirectories: true)
        let codexConfigURL = importDir.appendingPathComponent("config.toml")
        let codexAuthURL = importDir.appendingPathComponent("auth.json")
        try """
        model = "gpt-5.3-codex"
        model_provider = "e2e_codex"

        [model_providers.e2e_codex]
        name = "Imported Codex Gateway"
        base_url = "https://codex-gateway.example.invalid/v1"
        wire_api = "responses"
        env_key = "OPENAI_API_KEY"
        """.write(to: codexConfigURL, atomically: true, encoding: .utf8)
        try #"{"OPENAI_API_KEY": "sk-codex-e2e-test-key"}"#.write(
            to: codexAuthURL,
            atomically: true,
            encoding: .utf8
        )
        let importBridge = try connectionService.makeImportBridge()
        defer { importBridge.cancel() }
        let importPlan = try await importBridge.preview(
            source: "codex",
            path: codexConfigURL.path,
            credentialPath: codexAuthURL.path,
            includeCredentials: true,
            allowEnvironment: false
        )
        let importCandidate = try XCTUnwrap(
            importPlan.candidates.first(where: { $0.issue == nil }),
            "Expected a valid Codex import candidate from preview"
        )
        XCTAssertEqual(importCandidate.credential_status, "available")
        let importResults = try await importBridge.commit(
            planID: importPlan.plan_id,
            selected: [importCandidate.id],
            updateExisting: [],
            enableModels: false
        )
        XCTAssertTrue(
            importResults.contains(where: { $0.candidate_id == importCandidate.id && $0.error == nil }),
            "Expected Codex import commit to succeed: \(importResults)"
        )
        let connectionsAfterImport = try connectionService.connections()
        XCTAssertTrue(
            connectionsAfterImport.contains(where: { $0.id == importCandidate.connection.id }),
            "Imported Codex connection should appear in connectionService.connections()"
        )

        // 3b. Real Capability Probe (prepareProbe -> confirmProbe) against the real LAN server
        let connectionBeforeProbe = try XCTUnwrap(
            connectionsAfterImport.first(where: { $0.id == createdConnection.id }),
            "LAN connection should exist before capability probe"
        )
        let probeBridge = try MacModelImportBridge(application: port)
        defer { probeBridge.cancel() }
        let probeConfirmation = try await probeBridge.prepareProbe(
            snapshot: connectionBeforeProbe,
            model: selectedCatalogModel
        )
        XCTAssertEqual(probeConfirmation.connection_id, connectionBeforeProbe.id)
        XCTAssertEqual(probeConfirmation.model_id, selectedCatalogModel.model_id)
        XCTAssertFalse(probeConfirmation.acknowledge_possible_charges)

        let probedSnapshot = try await probeBridge.confirmProbe(
            requestID: probeConfirmation.request_id,
            acknowledgePossibleCharges: true
        )
        XCTAssertGreaterThan(
            probedSnapshot.revision,
            connectionBeforeProbe.revision,
            "confirmProbe should increment connection revision"
        )

        let connectionsAfterProbe = try connectionService.connections()
        let reloadedProbedConnection = try XCTUnwrap(
            connectionsAfterProbe.first(where: { $0.id == createdConnection.id }),
            "Reloaded connections should contain probed LAN connection"
        )
        XCTAssertGreaterThan(
            reloadedProbedConnection.revision,
            connectionBeforeProbe.revision,
            "Reloaded connection revision should reflect persisted capability probe"
        )
        let matchingAssessment = reloadedProbedConnection.document.capability_assessments?.first(where: {
            $0.model_id == selectedCatalogModel.model_id && $0.protocol == selectedCatalogModel.protocol
        })
        XCTAssertEqual(
            matchingAssessment?.evidence?.text_generation,
            true,
            "Persisted capability_assessments should record evidence.text_generation == true for \(selectedCatalogModel.model_id)"
        )

        let modelPreference = MacDefaultChatModelPresentation(store: .init(application: port))
        modelPreference.reload()
        let modelNav = MacModelCenterNavigationState(selectedTab: .api)
        try await renderAndCapture(
            MacModelCenterView(remoteHost: remoteHost, navigation: modelNav, preference: modelPreference),
            extraWait: .milliseconds(600),
            to: evidenceDir.appendingPathComponent("mac-03-models.png")
        )

        // 4. Real MacExtensionsCenterView (Skills & MCP)
        try await renderAndCapture(
            MacExtensionsCenterView(
                remoteHost: remoteHost,
                kinds: [.skill, .mcp],
                activeWorkspaceURL: workspaceDir,
                store: port
            ),
            extraWait: .milliseconds(900),
            to: evidenceDir.appendingPathComponent("mac-04-extensions.png")
        )

        // 5. Real MacAutomationCenterView
        try await renderAndCapture(
            MacAutomationCenterView(store: automationStore, activityStore: activityStore),
            to: evidenceDir.appendingPathComponent("mac-05-automations.png")
        )

        // 6. Real MacDataCenterView (Memory & Sources)
        try await renderAndCapture(
            MacDataCenterView(registry: registry, webSources: webStore, initialSection: "sources"),
            to: evidenceDir.appendingPathComponent("mac-06-memory-sources.png")
        )

        // 7 & 8. Real MacSettingsView in Light (Aqua) and Dark (DarkAqua) appearance
        try await renderAndCapture(
            MacSettingsView(snapshotHostState: .notConfigured),
            appearance: .aqua,
            to: evidenceDir.appendingPathComponent("mac-07-settings-light.png")
        )
        try await renderAndCapture(
            MacSettingsView(snapshotHostState: .notConfigured),
            appearance: .darkAqua,
            to: evidenceDir.appendingPathComponent("mac-08-settings-dark.png")
        )
    }

    // MARK: - Window rendering & PNG capture helpers

    private func makeHostWindow<Content: View>(
        _ view: Content,
        size: NSSize = NSSize(width: 1280, height: 820),
        appearance: NSAppearance.Name? = nil
    ) -> (NSWindow, NSHostingView<AnyView>) {
        let hosting = NSHostingView(rootView: AnyView(view.frame(width: size.width, height: size.height)))
        hosting.frame = NSRect(origin: .zero, size: size)
        let window = NSWindow(
            contentRect: hosting.bounds,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        if let appearance {
            let resolved = NSAppearance(named: appearance)
            window.appearance = resolved
            hosting.appearance = resolved
        }
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        return (window, hosting)
    }

    private func renderAndCapture<Content: View>(
        _ view: Content,
        size: NSSize = NSSize(width: 1280, height: 820),
        appearance: NSAppearance.Name? = nil,
        extraWait: Duration = .zero,
        to url: URL
    ) async throws {
        let (window, hosting) = makeHostWindow(view, size: size, appearance: appearance)
        defer {
            window.orderOut(nil)
            window.contentView = nil
            window.close()
        }
        if extraWait > .zero {
            try? await Task.sleep(for: extraWait)
        }
        await settle(hosting)
        try capturePNG(hosting, to: url)
    }

    private func settle(_ hosting: NSView) async {
        for _ in 0..<5 {
            hosting.layoutSubtreeIfNeeded()
            hosting.displayIfNeeded()
            try? await Task.sleep(for: .milliseconds(40))
            await Task.yield()
        }
    }

    private func capturePNG(_ hosting: NSView, to url: URL) throws {
        hosting.layoutSubtreeIfNeeded()
        hosting.displayIfNeeded()
        let rep = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
        hosting.cacheDisplay(in: hosting.bounds, to: rep)
        let data = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        let decoded = try XCTUnwrap(NSBitmapImageRep(data: data))
        XCTAssertGreaterThan(decoded.pixelsWide, 0)
        XCTAssertGreaterThan(decoded.pixelsHigh, 0)
        try data.write(to: url, options: .atomic)
    }
}

// MARK: - Minimal offline remote-host stub for local-mode E2E views

@MainActor
final class E2ENoOpRemoteHost: MacRemoteAgentRunning {
    private nonisolated static func unsupported(_ name: String) -> Error {
        NSError(domain: "MacRealModelE2ETests.RemoteHost", code: 1, userInfo: [NSLocalizedDescriptionKey: "Unexpected remote call: \(name)"])
    }
    func isConfigured() async -> Bool { false }
    func testSavedConnection() async throws -> MacRemoteHostConnectionReport { throw Self.unsupported("testSavedConnection") }
    func requestWebManagementURL(target: String) async throws -> URL { throw Self.unsupported("requestWebManagementURL") }
    func fetchExtensions() async throws -> [MacExtensionItem] { [] }
    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend] { [] }
    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot { throw Self.unsupported("fetchRemoteModelCatalog") }
    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget { throw Self.unsupported("captureSubmissionTarget") }
    func permissionConfiguration(
        identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
        expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?
    ) async throws -> MacRemotePermissionConfiguration { throw Self.unsupported("permissionConfiguration") }
    func saveConversationSelection(
        conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget
    ) async throws {}
    func run(
        conversationID: String, submissionID: String, userMessageID: String,
        assistantMessageID: String, prompt: String, contextComponents: [MacCoreContextComponent],
        modelID: String, modelProviderID: String?, modelIntent: MacRemoteModelIntent?,
        memoryEnabled: Bool, executionBackend: String, externalSessionID: String?,
        submission: MacRemoteSubmissionControl
    ) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: Self.unsupported("run")) }
    }
    func resolvePermission(request: MacRemotePermissionRequest, allow: Bool, scope: MacPermissionGrantScope) async throws {
        throw Self.unsupported("resolvePermission")
    }
    func sendTaskGuidance(
        conversationID: String, parentRunID: String, taskID: String,
        message: String, target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskGuidanceReceipt { throw Self.unsupported("sendTaskGuidance") }
    func requestTaskCancellation(
        conversationID: String, parentRunID: String, taskID: String,
        target: MacRemoteSubmissionTarget
    ) async throws -> AppleTaskCancellationReceipt { throw Self.unsupported("requestTaskCancellation") }
    func cancelRun(identity: MacRemoteSubmissionIdentity) async throws {}
    func taskEvents(conversationID: String, afterCursor: Int64) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: Self.unsupported("taskEvents")) }
    }
}
