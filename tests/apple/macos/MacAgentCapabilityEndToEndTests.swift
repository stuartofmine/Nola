// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises Mac chat capability routing and durable memory through the real Agent Host process.

import AppKit
import Darwin
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacAgentCapabilityEndToEndTests: XCTestCase {
    private var committedTextByRoot: [String: String] = [:]
    private var committedEmptyTerminalTurns: Set<String> = []
    override func tearDown() {
        MacChromeMessagingService.shared.stop()
        super.tearDown()
    }

    func testWholeAppColdPermissionRecoveryRebuildsOriginalRunAndCommitsItsSeal() async throws {
        try await assertColdPermissionRecovery(inputOrigin: "interactive_user")
    }

    func testScheduledOriginColdPermissionRecoveryPreservesOriginalInvocation() async throws {
        try await assertColdPermissionRecovery(inputOrigin: "scheduled_action")
    }

    private func assertColdPermissionRecovery(inputOrigin: String) async throws {
        let workspace = try makeWorkspace("cold-owned-run")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let target = workspace.appendingPathComponent("cold-written.txt")
        let server = try MacAgentCapabilityMockServer(workspace: workspace, steps: [
            ["message_contains": ["COLD_PERMISSION_WRITE"], "required_tools": ["write_file"], "tool_name": "write_file",
             "tool_args": ["path": target.path, "content": "COLD_RUN_CONTINUED"]],
            ["message_contains": ["cold-written.txt"], "required_tools": ["read_file"], "tool_name": "read_file",
             "tool_args": ["path": target.path, "operation_kind": "validate"]],
            ["message_contains": ["COLD_RUN_CONTINUED"], "content": "COLD_RECOVERY_DONE"],
        ])
        defer { server.stop() }
        let environment = MacAppEnvironment(applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport"),
            cachesRoot: workspace.appendingPathComponent("Caches"))
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "cold-endpoint",
            "endpointName": "Cold fixture", "baseURL": server.endpoint, "endpointEnabled": true,
            "endpointOrder": 0, "modelEnabled": true, "modelOrder": 0, "priority": 100,
            "capabilities": ["chat"], "wireProtocol": "chat_completions", "credentialAccount": "cold-key"]
        try port.upsertModelProfile(MacCanonicalModelProfile(id: "cold-profile", displayName: "Cold fixture",
            provider: "openai-compatible", modelName: "nola-capability-e2e", runtime: "remote",
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let keys = MacLocalAgentCredentialGateTests.Credentials()
        keys.set("cold-key", "local-e2e-key")
        let credentials = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { keys.read($0) })
        let original = MacAgentHostRuntime(credentialStore: credentials)
        var preparedRequest = try MacLocalAgentRunRequest(conversationID: "cold-owner-\(UUID())",
            prompt: "COLD_PERMISSION_WRITE: create cold-written.txt and validate it", projectURL: workspace,
            attachmentURLs: [], authorizedWorkspaceURLs: [workspace], modelID: "nola-capability-e2e",
            modelProfileID: "cold-profile", endpoint: server.endpoint, apiKey: "local-e2e-key",
            contextWindowTokens: 32768, inputOrigin: inputOrigin, permissionMode: .cautious,
            databaseURLOverride: environment.databaseURL, appEnvironment: environment, applicationPort: port)
        if inputOrigin == "scheduled_action" {
            // Match scheduled launch's workspace-only scope. Origin does not
            // grant an automation lease or bypass the concrete permission wait.
            preparedRequest.allowedDirectoryArguments = ["--allow-dir", workspace.standardizedFileURL.path]
        }
        let request = preparedRequest
        let watchdog = Task<Void, Never> {
            do { try await Task.sleep(for: .seconds(30)) }
            catch { return }
            if !Task.isCancelled, let pid = await original.ownedProcessIdentifier(conversationID: request.conversationID,
                expectedRunID: request.submissionID) { Darwin.kill(pid, SIGKILL) }
        }
        var originalPermissionID: String?
        var killedOwnedHost = false
        do {
            for try await event in await original.run(request) {
                if let permission = event.permissionRequest {
                    originalPermissionID = permission.id
                    let ownedPID = await original.ownedProcessIdentifier(conversationID: request.conversationID,
                        expectedRunID: request.submissionID)
                    let pid = try XCTUnwrap(ownedPID)
                    XCTAssertEqual(Darwin.kill(pid, SIGKILL), 0)
                    killedOwnedHost = true
                }
            }
        } catch {
            watchdog.cancel()
            await watchdog.value
            // Only the explicitly injected process death is expected. Model
            // discovery/admission failures must not masquerade as cold loss.
            guard killedOwnedHost else { throw error }
        }
        watchdog.cancel()
        await watchdog.value
        guard originalPermissionID != nil, killedOwnedHost else {
            return XCTFail("Fixture did not reach and kill the permission-owning Host")
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: target.path))
        let reopened = try MacApplicationPort(databaseURL: environment.databaseURL)
        XCTAssertEqual(try reopened.activeRunIdentity(conversationID: request.conversationID)?.runID, request.submissionID)
        let before = try XCTUnwrap(reopened.loadExactRunInvocation(conversationID: request.conversationID,
            expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID))
        XCTAssertEqual(try MacFrozenLaunchDescriptor.load(before).input_origin, inputOrigin)
        let coldCredentials = MacAppleChatConfigurationStore(applicationPort: reopened, credentialReader: { keys.read($0) })
        let cold = MacAgentHostRuntime(credentialStore: coldCredentials)
        let authorization = MacLocalAgentResumeAuthorization(projectURL: workspace,
            authorizedWorkspaceURLs: [workspace], permissionMode: .cautious)
        for fault in ["owner", "workspace", "credential"] {
            if fault == "credential" { keys.set("cold-key", nil) }
            do {
                try await cold.prepareColdResume(conversationID: request.conversationID,
                    expectedRunID: fault == "owner" ? "foreign-run" : request.submissionID,
                    assistantMessageID: request.assistantMessageID, applicationPort: reopened,
                    authorization: fault == "workspace" ? .init(projectURL: workspace.appendingPathComponent("foreign"),
                        authorizedWorkspaceURLs: [workspace], permissionMode: .cautious) : authorization,
                    appEnvironment: environment)
                XCTFail("Cold recovery accepted \(fault)")
            } catch { }
            keys.set("cold-key", "local-e2e-key")
            let unchanged = try XCTUnwrap(reopened.loadExactRunInvocation(conversationID: request.conversationID,
                expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID))
            XCTAssertEqual(unchanged.checkpointSHA256, before.checkpointSHA256)
            XCTAssertEqual(unchanged.invocationConfigDigest, before.invocationConfigDigest)
        }
        try await cold.prepareColdResume(conversationID: request.conversationID, expectedRunID: request.submissionID,
            assistantMessageID: request.assistantMessageID, applicationPort: reopened,
            authorization: authorization, appEnvironment: environment)
        let coldWatchdog = Task<Void, Never> {
            do { try await Task.sleep(for: .seconds(45)) }
            catch { return }
            if !Task.isCancelled, let pid = await cold.ownedProcessIdentifier(conversationID: request.conversationID,
                expectedRunID: request.submissionID) { Darwin.kill(pid, SIGKILL) }
        }
        var firstRecoveredPermission: String?
        do {
            for try await event in await cold.resume(conversationID: request.conversationID,
                expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID,
                revalidate: { authorization }, transportReady: nil) {
                if let permission = event.permissionRequest {
                    if firstRecoveredPermission == nil { firstRecoveredPermission = permission.id }
                    try await cold.resolvePermission(conversationID: request.conversationID,
                        request: permission, allow: true, scope: .once)
                }
            }
        } catch {
            coldWatchdog.cancel()
            await coldWatchdog.value
            throw error
        }
        coldWatchdog.cancel()
        await coldWatchdog.value
        XCTAssertEqual(firstRecoveredPermission, originalPermissionID)
        XCTAssertEqual(try String(contentsOf: target, encoding: .utf8), "COLD_RUN_CONTINUED")
        let terminal = try XCTUnwrap(reopened.canonicalRunTerminal(conversationID: request.conversationID, runID: request.submissionID))
        XCTAssertEqual(terminal.assistantMessageID, request.assistantMessageID)
        // This mock returns plain answer text, not an explicit requirement
        // assessment/propose_result. Core must not invent ActionCompleted from
        // observed tool effects. Assert the effects independently below/above.
        XCTAssertEqual(terminal.result.value.kind, .answered)
        let after = try XCTUnwrap(reopened.loadExactRunInvocation(conversationID: request.conversationID,
            expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID))
        XCTAssertEqual(after.invocationConfigDigest, before.invocationConfigDigest)
        XCTAssertEqual(try MacFrozenLaunchDescriptor.load(after).input_origin, inputOrigin)
        XCTAssertNil(try reopened.activeRunIdentity(conversationID: request.conversationID))
        try assertCompleteReport(server, expectedStageCount: 3)
    }

    // Canonical answer evidence is independent of transport text deltas: the
    // model may supply its answer through a legitimate structured tool call.
    func testRealConfiguredModelProducesCanonicalAnswerThroughMacAgentHostWhenExplicitlyEnabled() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac Agent Host request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let workspace = try makeWorkspace("real-provider")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "mac-real-provider-\(UUID().uuidString)"
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let applicationPort = try MacApplicationPort(databaseURL: databaseURL)
        try seedMixedOwnershipDataSources(in: applicationPort, includeMalformedOwnedSource: false)

        let events = try await runAgentHost(
            prompt: "Return only this exact ASCII token: NOLA_MAC_REAL_OK",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey
        )

        XCTAssertEqual(
            assistantText(events).trimmingCharacters(in: .whitespacesAndNewlines),
            "NOLA_MAC_REAL_OK"
        )
        XCTAssertTrue(events.contains { $0.kind == .finished })
        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(
            events.allSatisfy { event in
                event.trace != nil
                    || event.assistantJournalCursor != nil
                    || event.coreTurnTerminal != nil
                    || event.coreTaskUpdate != nil
                    || event.coreSkillRuntimeUpdate != nil
                    || event.coreCapabilityActivation != nil
                    || event.corePermissionProjection != nil
                    || event.assistantRunTerminalSealDigest != nil
                    || event.contextWindowTokens != nil
            },
            "Every non-Core-projection event must use the versioned Agent Trace contract"
        )
        XCTAssertEqual(
            Set(try applicationPort.listDataSources().map { "\($0.id):\($0.kind)" }),
            ["platform:health:platform", "web:native-fixture:web"]
        )
    }

    func testAgentHostIgnoresForeignCanonicalDataSourceKindsWithoutMutatingThem() async throws {
        let workspace = try makeWorkspace("foreign-data-sources")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let applicationPort = try MacApplicationPort(databaseURL: databaseURL)
        try seedMixedOwnershipDataSources(in: applicationPort, includeMalformedOwnedSource: true)
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["FOREIGN_DATA_SOURCE_E2E"],
                    "content": "FOREIGN_DATA_SOURCE_E2E_OK",
                ],
                [
                    "message_contains": ["FOREIGN_DATA_SOURCE_COLD_RESTART"],
                    "content": "FOREIGN_DATA_SOURCE_COLD_RESTART_OK",
                ],
            ]
        )
        defer { server.stop() }
        let conversationID = "foreign-data-source-\(UUID().uuidString)"

        let events = try await runAgentHost(
            prompt: "Return FOREIGN_DATA_SOURCE_E2E_OK for FOREIGN_DATA_SOURCE_E2E.",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false
        )

        XCTAssertTrue(events.contains { $0.kind == .finished })
        XCTAssertTrue(assistantText(events).contains("FOREIGN_DATA_SOURCE_E2E_OK"))
        let firstProjection = try XCTUnwrap(applicationPort.loadConversation(conversationID))
        XCTAssertEqual(firstProjection.messages.map(\.role), ["user", "assistant"])
        XCTAssertEqual(
            firstProjection.messages.last?.parts.first(where: { $0.kind == "text" })?.contentText,
            "FOREIGN_DATA_SOURCE_E2E_OK"
        )
        XCTAssertEqual(
            Set(try applicationPort.listDataSources().map { "\($0.id):\($0.kind)" }),
            [
                "local:malformed:localFile",
                "platform:health:platform",
                "web:native-fixture:web",
            ]
        )

        // Exercise the production cold-start boundary. The next Host must
        // reconnect to a newly launched single writer and append to the typed
        // transcript rather than start a competing service or replay text.
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        let reopenedStore = try MacApplicationPort(databaseURL: databaseURL)
        let resumedEvents = try await runAgentHost(
            prompt: "Return FOREIGN_DATA_SOURCE_COLD_RESTART_OK for FOREIGN_DATA_SOURCE_COLD_RESTART.",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false
        )
        XCTAssertTrue(resumedEvents.contains { $0.kind == .finished })
        XCTAssertTrue(assistantText(resumedEvents).contains("FOREIGN_DATA_SOURCE_COLD_RESTART_OK"))

        let restored = try XCTUnwrap(reopenedStore.loadConversation(conversationID))
        XCTAssertEqual(restored.messages.map(\.role), ["user", "assistant", "user", "assistant"])
        XCTAssertEqual(restored.messages.map(\.ordinal), [1, 2, 3, 4])
        XCTAssertEqual(
            restored.messages
                .filter { $0.role == "assistant" }
                .compactMap { message in
                    message.parts.first(where: { $0.kind == "text" })?.contentText
                },
            ["FOREIGN_DATA_SOURCE_E2E_OK", "FOREIGN_DATA_SOURCE_COLD_RESTART_OK"]
        )
        XCTAssertEqual(
            Set(try reopenedStore.listDataSources().map { "\($0.id):\($0.kind)" }),
            [
                "local:malformed:localFile",
                "platform:health:platform",
                "web:native-fixture:web",
            ]
        )
        try assertCompleteReport(server, expectedStageCount: 2)
    }

    func testImageAttachmentReachesModelAndCanonicalTranscript() async throws {
        let workspace = try makeWorkspace("native-image-attachment")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let imageURL = workspace.appendingPathComponent("pixel.png")
        let imageData = try XCTUnwrap(Data(base64Encoded:
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        ))
        try imageData.write(to: imageURL, options: .atomic)
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let conversationID = "native-image-\(UUID().uuidString)"
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [[
                "message_contains": ["Describe the attached image."],
                "requires_image": true,
                "content": "NATIVE_IMAGE_ATTACHMENT_COMPLETE",
            ]]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "Describe the attached image.",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            attachmentURLs: [imageURL],
            modelExecutionCapabilities: MacModelExecutionCapabilities(toolCalls: true, imageInput: true)
        )

        XCTAssertTrue(assistantText(events).contains("NATIVE_IMAGE_ATTACHMENT_COMPLETE"))
        let projection = try XCTUnwrap(
            MacApplicationPort(databaseURL: databaseURL).loadConversation(conversationID)
        )
        let user = try XCTUnwrap(projection.messages.first { $0.role == "user" })
        XCTAssertTrue(
            user.parts.contains { $0.kind == "attachment" },
            "Canonical user Parts: \(user.parts.map { "\($0.kind):\($0.id):\($0.contentText)" })"
        )
        try assertCompleteReport(server, expectedStageCount: 1)
    }

    func testRealConfiguredModelRendersProductionChatBubbleWithToolAndMarkdown() async throws {
        let environment = ProcessInfo.processInfo.environment
        let fixtureURL = URL(fileURLWithPath: "/tmp/nola-real-llm-e2e.json")
        let fixture = (try? Data(contentsOf: fixtureURL))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] }
        let configuredEndpoint = environment["NOLA_REAL_LLM_ENDPOINT"] ?? fixture?["endpoint"]
        let configuredAPIKey = environment["NOLA_REAL_LLM_API_KEY"] ?? fixture?["apiKey"]
        let configuredModelID = environment["NOLA_REAL_LLM_MODEL"] ?? fixture?["model"]
        let configuredDatabasePath = environment["NOLA_DB_PATH"] ?? fixture?["databasePath"]
        let hasCompleteConfiguration = [
            configuredEndpoint,
            configuredAPIKey,
            configuredModelID,
            configuredDatabasePath
        ]
            .allSatisfy { $0?.isEmpty == false }
        guard (environment["NOLA_REAL_LLM_E2E"] == "1" || fixture != nil),
              hasCompleteConfiguration else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real rendered Mac chat request.")
        }
        let endpoint = try XCTUnwrap(configuredEndpoint)
        let apiKey = try XCTUnwrap(configuredAPIKey)
        let modelID = try XCTUnwrap(configuredModelID)
        let workspace = try makeWorkspace("real-rendered-chat").resolvingSymlinksInPath()
        defer { try? FileManager.default.removeItem(at: workspace) }
        let marker = "MAC-RENDERED-TOOL-\(UUID().uuidString.uppercased())"
        try "# Source\n\nMarker: \(marker)\n".write(
            to: workspace.appendingPathComponent("report.md"),
            atomically: true,
            encoding: .utf8
        )

        let configurationStore = MacAppleChatConfigurationStore.shared
        let previousConfiguration = configurationStore.endpoints()
        try configurationStore.save(
            baseURL: endpoint,
            apiKey: apiKey,
            model: modelID,
            executionCapabilities: .init(toolCalls: true, samplingParameters: true)
        )
        defer {
            try? configurationStore.replaceEndpoints(previousConfiguration)
        }

        var conversationID = "mac-real-rendered-\(UUID().uuidString)"
        let databaseURL = URL(
            fileURLWithPath: try XCTUnwrap(configuredDatabasePath)
        )
        let previousDatabasePath = environment["NOLA_DB_PATH"]
        setenv("NOLA_DB_PATH", databaseURL.path, 1)
        defer {
            if let previousDatabasePath {
                setenv("NOLA_DB_PATH", previousDatabasePath, 1)
            } else {
                unsetenv("NOLA_DB_PATH")
            }
        }
        let archiveStore = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: databaseURL)
        )
        let bookmarkSuite = "nola-real-rendered-bookmarks-\(UUID().uuidString)"
        let bookmarkDefaults = try XCTUnwrap(UserDefaults(suiteName: bookmarkSuite))
        defer { bookmarkDefaults.removePersistentDomain(forName: bookmarkSuite) }
        let bookmarkStore = MacSecurityScopedBookmarkStore(defaults: bookmarkDefaults)
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            bookmarkStore: bookmarkStore
        )
        // Selecting an unknown ID is not conversation creation. Exercise the
        // same explicit admission path as the workspace's new-chat action.
        model.newConversation()
        conversationID = try XCTUnwrap(model.selectedConversationID)
        await model.loadSelectedConversationIfNeeded()
        model.executionLocation = .thisMac
        model.restoreDefaultModelInheritance()
        try bookmarkStore.confirmFolderAccess(workspace, conversationID: conversationID)
        model.workspaceURL = workspace
        model.draft = "Use the read_file tool to read report.md. Then answer with a Markdown level-two heading named Verified, followed by the exact marker from the file."
        model.send()
        XCTAssertTrue(model.isRunning, "The real rendered chat must be admitted before waiting: \(model.activity)")

        let deadline = ContinuousClock.now + .seconds(90)
        var timeline: [[String: Any]] = []
        var lastTimelineSignature: String?
        while model.isRunning, ContinuousClock.now < deadline {
            let visibleAssistant = model.selectedMessages.last(where: { $0.role == .assistant })
            let assistantCharacters = visibleAssistant?.text.count ?? 0
            let renderedTextNodes = visibleAssistant?.presentation?.renderedTextNodes.count ?? 0
            let traceItems = visibleAssistant?.presentation?.traceItems.count ?? 0
            let signature = "true|\(visibleAssistant != nil)|\(assistantCharacters)|\(renderedTextNodes)|\(traceItems)"
            if signature != lastTimelineSignature {
                timeline.append([
                    "stage": "real-rendered-chat",
                    "running": true,
                    "assistantPresent": visibleAssistant != nil,
                    "assistantCharacters": assistantCharacters,
                    "assistantText": visibleAssistant?.text ?? "",
                    "renderedTextNodes": renderedTextNodes,
                    "traceItems": traceItems,
                ])
                lastTimelineSignature = signature
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertFalse(model.isRunning, "The production Mac chat run did not finish")

        let assistant = try XCTUnwrap(
            model.messagesByConversation[conversationID]?.last(where: { $0.role == .assistant }),
            "No durable assistant after admission; activity=\(model.activity), recovery=\(String(describing: model.localRunRecovery))"
        )
        XCTAssertTrue(assistant.text.contains(marker), "The rendered response lost the tool result")
        XCTAssertTrue(assistant.text.contains("## Verified"), "The real response did not preserve Markdown")
        let presentation = try XCTUnwrap(assistant.presentation)
        XCTAssertTrue(
            presentation.traceItems.contains {
                $0.kind == .tool
                    && MacToolPresentationPolicy.displayTitle(for: $0.title)
                        == String(localized: "Read file")
            },
            "The production assistant bubble did not retain the real read_file trace"
        )
        XCTAssertFalse(presentation.textNodes.isEmpty)

        let root = MacMessageView(
            message: assistant,
            transcriptWidth: 900,
            isStreaming: false,
            liveActivities: [],
            isSearchMatch: false,
            onSendTaskGuidance: { _, _ in },
            onRequestTaskCancellation: { _ in },
            showsAnimatedStatusLight: false
        )
        .frame(width: 900, alignment: .leading)
        .padding(24)
        .frame(width: 948, height: 720, alignment: .topLeading)
        .background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 948, height: 720)
        hosting.wantsLayer = true
        for _ in 0..<4 {
            hosting.layoutSubtreeIfNeeded()
            hosting.displayIfNeeded()
            await Task.yield()
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertGreaterThan(hosting.fittingSize.height, 90)
        let representation = try XCTUnwrap(
            hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds)
        )
        hosting.cacheDisplay(in: hosting.bounds, to: representation)
        let png = try XCTUnwrap(
            representation.representation(using: .png, properties: [:])
        )
        XCTAssertGreaterThan(png.count, 10_000)

        timeline.append([
            "stage": "real-rendered-chat",
            "running": false,
            "assistantPresent": true,
            "assistantCharacters": assistant.text.count,
            "assistantText": assistant.text,
            "renderedTextNodes": presentation.renderedTextNodes.count,
            "traceItems": presentation.traceItems.count,
        ])
        let screenshotURL = try realUIArtifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let accessibilityURL = try realUIArtifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY")
        let timelineURL = try realUIArtifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE")
        let transcriptURL = try realUIArtifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT")
        try png.write(to: screenshotURL, options: .atomic)
        let accessibilityTree = realAccessibilityTree(hosting)
        XCTAssertTrue(
            accessibilityTree.contains(marker),
            "The production assistant answer is missing from the AppKit accessibility tree"
        )
        try accessibilityTree.write(
            to: accessibilityURL,
            atomically: true,
            encoding: .utf8
        )
        try JSONSerialization.data(
            withJSONObject: timeline,
            options: [.prettyPrinted, .sortedKeys]
        ).write(to: timelineURL, options: .atomic)
        try MacCanonicalTranscriptEvidence.persist(model.selectedMessages, to: transcriptURL)
    }

    func testRealConfiguredRoutePersistsContextBudgetBlockerWithoutModelCall() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 for real-route budget admission.")
        }
        let workspace = try makeWorkspace("real-budget-blocker")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "mac-real-budget-\(UUID().uuidString)"
        let databaseURL = URL(fileURLWithPath: environment["NOLA_DB_PATH"] ?? workspace.appendingPathComponent("state/nola.db").path)
        let events = try await runAgentHost(
            prompt: "Reply only ACK.", conversationID: conversationID, workspace: workspace,
            endpoint: try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"]), memoryEnabled: false,
            contextWindowTokens: 4096, databaseURL: databaseURL, deleteSessionAfterRun: false,
            modelID: try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"]),
            apiKey: try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        )
        let ended = try XCTUnwrap(events.compactMap(\.coreTurnTerminal).last)
        let terminal = try XCTUnwrap(MacApplicationPort(databaseURL: databaseURL)
            .canonicalRunTerminal(conversationID: conversationID, runID: ended.turnID))
        XCTAssertEqual(terminal.result.value.kind, .blocked)
        XCTAssertEqual(assistantText(events), "")
        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
    }

    func testRealConfiguredModelCompactsAndRecallsMacAgentHostConversation() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac compaction request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let workspace = try makeWorkspace("real-provider-compaction")
        // Keep the small-window recovery fixture as the default. An explicit
        // larger window separately verifies the cache-preserving hot path.
        let compactionWindow = Int(environment["NOLA_REAL_COMPACTION_WINDOW"] ?? "") ?? 16_000
        XCTAssertGreaterThan(compactionWindow, 0)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "mac-real-compaction-\(UUID().uuidString)"
        let databaseURL = URL(fileURLWithPath: environment["NOLA_DB_PATH"] ?? workspace.appendingPathComponent("state/nola.db").path)
        let fact = "MAC-COMPACTION-FACT-\(UUID().uuidString.uppercased())"
        let originalPrompt = "Remember this exact conversation fact: \(fact). Reply only ACK-0."
        try isolatedConversationRepository(workspace: workspace).restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Real provider compaction",
                    preview: originalPrompt,
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [MacChatMessage(role: .user, text: originalPrompt, createdAt: .now)],
                knowledge: MacConversationKnowledge(
                    keyPointsMarkdown: "",
                    wikiMarkdown: "",
                    updatedAt: .now
                )
            )
        ])
        var allEvents: [MacLocalAgentEvent] = []

        let firstEvents = try await runAgentHost(
            prompt: originalPrompt,
            conversationID: conversationID,
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            contextWindowTokens: compactionWindow,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey
        )
        _ = try XCTUnwrap(firstEvents.first { $0.kind == .finished }, "Initial compaction fixture turn failed; do not amplify the failed archive")
        _ = try XCTUnwrap(
            assistantText(firstEvents).trimmingCharacters(in: .whitespacesAndNewlines) == "ACK-0" ? true : nil,
            "Initial compaction fixture did not acknowledge the fact; stop before adding history"
        )
        allEvents.append(contentsOf: firstEvents)

        for index in 1...4 {
            let filler = String(
                repeating: "bounded-compaction-segment-\(index) ",
                count: 520
            )
            let turnEvents = try await runAgentHost(
                prompt: "Conversation continuation \(index). Reply only ACK-\(index).\n\(filler)",
                conversationID: conversationID,
                workspace: workspace,
                endpoint: endpoint,
                memoryEnabled: false,
                contextWindowTokens: compactionWindow,
                databaseURL: databaseURL,
                deleteSessionAfterRun: false,
                modelID: model,
                apiKey: apiKey
            )
            _ = try XCTUnwrap(turnEvents.first { $0.kind == .finished }, "Compaction fixture turn \(index) failed; stop before appending more history")
            _ = try XCTUnwrap(
                assistantText(turnEvents).trimmingCharacters(in: .whitespacesAndNewlines) == "ACK-\(index)" ? true : nil,
                "Compaction fixture turn \(index) did not acknowledge the continuation; stop before adding history"
            )
            allEvents.append(contentsOf: turnEvents)
        }

        let compactionEvents = allEvents.filter { $0.trace?.kind == "context.compacted" }
        XCTAssertFalse(
            compactionEvents.isEmpty,
            "The configured real model path never emitted context.compacted"
        )
        XCTAssertTrue(
            try isolatedConversationRepository(workspace: workspace).loadAll()
                .first(where: { $0.conversation.id == conversationID })?
                .messages.contains(where: { $0.text.contains(fact) }) == true,
            "Compaction replaced or lost the original conversation transcript"
        )

        let recallEvents = try await runAgentHost(
            prompt: "Use conversation_recall to inspect the original first user message, not just the compaction summary. Search for MAC-COMPACTION-FACT. Then return the exact fact token from that original message. Do not guess the UUID.",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            contextWindowTokens: compactionWindow,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey
        )
        XCTAssertTrue(
            assistantText(recallEvents).contains(fact),
            "The configured real model did not recover the early fact after compaction"
        )
        XCTAssertTrue(
            recallEvents.contains { $0.kind == .toolStarted && $0.title == "conversation_recall" },
            "Answering from the summary alone does not verify canonical history retrieval"
        )
        let recallResults = recallEvents.filter {
            $0.kind == .toolFinished && $0.title == "conversation_recall" && !$0.isError
        }
        XCTAssertFalse(recallResults.isEmpty, "Canonical history retrieval did not succeed")
        // UI trace results intentionally contain only previews. Verify the
        // exact successful calls in durable canonical results, not UI prose.
        let canonicalResults = try readCanonicalToolResults(
            databaseURL: databaseURL,
            conversationID: conversationID,
            callIDs: recallResults.compactMap { $0.toolCallID ?? $0.trace?.payload.callID }
        )
        let recalledContent = canonicalResults.map(\.displayString).joined(separator: "\n")
        XCTAssertTrue(
            recalledContent.contains(fact),
            "The successful history tool result did not expose the original random fact"
        )
        XCTAssertTrue(
            canonicalResults.contains { result in
                return containsCanonicalRecallEvidence(result, fact: fact)
            },
            "History results must identify the canonical source message"
        )
        XCTAssertFalse(
            allEvents.contains { $0.kind == .permissionRequested }
                || recallEvents.contains { $0.kind == .permissionRequested }
        )
    }

    func testRealConfiguredModelWaitsForUserInputThenResumesSameRun() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 for real user-input acceptance.")
        }
        let workspace = try makeWorkspace("real-user-input")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "mac-real-question-\(UUID().uuidString)"
        let databaseURL = URL(fileURLWithPath: try XCTUnwrap(environment["NOLA_DB_PATH"]))
        let appEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport"),
            cachesRoot: workspace.appendingPathComponent("Caches"))
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "wait-endpoint",
            "endpointName": "Wait acceptance", "baseURL": endpoint, "endpointEnabled": true,
            "endpointOrder": 0, "modelEnabled": true, "modelOrder": 0, "priority": 100,
            "capabilities": ["chat"], "wireProtocol": "chat_completions", "credentialAccount": "wait-key"]
        try port.upsertModelProfile(MacCanonicalModelProfile(id: "wait-profile", displayName: "Wait acceptance",
            provider: "openai-compatible", modelName: model, runtime: "remote",
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let credentials = MacAppleChatConfigurationStore(applicationPort: port,
            credentialReader: { account in account == "wait-key" ? apiKey : nil })
        let runtime = MacAgentHostRuntime(credentialStore: credentials)
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: "Use request_user_input to ask exactly one free-text question: what exact token should I echo? You do not know the token yet. Wait for my answer, then reply with only that exact token. Do not use any other tool.",
            projectURL: workspace, attachmentURLs: [],
            modelID: model, modelProfileID: "wait-profile", endpoint: endpoint, apiKey: apiKey,
            memoryEnabled: false, databaseURLOverride: databaseURL,
            appEnvironment: appEnvironment, applicationPort: try MacApplicationPort(databaseURL: databaseURL))
        var waitingEvents: [MacLocalAgentEvent] = []
        do {
            for try await event in await runtime.run(request) {
                waitingEvents.append(event)
            }
        } catch let suspension as MacLocalAgentWaitSuspension {
            XCTAssertEqual(suspension.conversationID, conversationID)
            XCTAssertEqual(suspension.runID, request.submissionID)
            XCTAssertFalse(suspension.readyToResume)
        }
        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let waits = try reopened.openUserInputWaits(conversationID: conversationID)
        XCTAssertEqual(waits.count, 1)
        let wait = try XCTUnwrap(waits.first)
        XCTAssertEqual(wait.runID, request.submissionID)
        XCTAssertFalse(waitingEvents.contains { $0.coreTurnTerminal != nil })
        XCTAssertNil(try reopened.canonicalRunTerminal(conversationID: conversationID, runID: request.submissionID))
        let question = try XCTUnwrap(wait.questions.first)
        XCTAssertEqual(wait.questions.count, 1)
        XCTAssertTrue(question.allowsFreeform)
        let marker = "MAC-WAIT-ANSWER-\(UUID().uuidString)"
        let receipt = try reopened.respondToUserInputWait(wait, commandID: UUID().uuidString,
            answers: [.init(questionID: question.id, optionID: nil, text: marker)], decline: false)
        XCTAssertEqual(receipt.disposition, "applied")
        XCTAssertEqual(receipt.resumePolicy, "conversation_run")
        let authorization = MacLocalAgentResumeAuthorization(projectURL: workspace,
            authorizedWorkspaceURLs: request.authorizedWorkspaceURLs, permissionMode: request.permissionMode)
        // Reconstruct ownership from durable invocation, not the original
        // runtime's in-memory launch snapshot.
        let coldRuntime = MacAgentHostRuntime(credentialStore: credentials)
        try await coldRuntime.prepareColdResume(conversationID: conversationID,
            expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID,
            applicationPort: reopened, authorization: authorization, appEnvironment: appEnvironment)
        var resumedEvents: [MacLocalAgentEvent] = []
        for try await event in await coldRuntime.resume(conversationID: conversationID,
            expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID,
            revalidate: { authorization }, transportReady: nil) {
            resumedEvents.append(event)
        }
        let answer = try await readCommittedAssistantText(resumedEvents, databaseURL: databaseURL)
        XCTAssertEqual(answer.trimmingCharacters(in: .whitespacesAndNewlines), marker)
        let terminal = try XCTUnwrap(reopened.canonicalRunTerminal(
            conversationID: conversationID, runID: request.submissionID))
        XCTAssertEqual(terminal.result.value.kind, .answered)
        XCTAssertTrue(try reopened.openUserInputWaits(conversationID: conversationID).isEmpty)
    }

    func testRealConfiguredModelCancelsRunningMacToolAndRecoversConversation() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac cancellation request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let workspace = try makeWorkspace("real-provider-cancellation")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "mac-real-cancellation-\(UUID().uuidString)"
        let databaseURL = URL(fileURLWithPath: environment["NOLA_DB_PATH"] ?? workspace.appendingPathComponent("state/nola.db").path)
        let appEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: "Use the bash tool to run `sleep 30` in the foreground. Wait for it before answering.",
            projectURL: workspace,
            attachmentURLs: [],
            modelID: model,
            endpoint: endpoint,
            apiKey: apiKey,
            memoryEnabled: false,
            databaseURLOverride: databaseURL,
            appEnvironment: appEnvironment,
            applicationPort: try MacApplicationPort(databaseURL: databaseURL)
        )
        let events = try await withThrowingTaskGroup(of: [MacLocalAgentEvent].self) { group in
            group.addTask {
                var captured: [MacLocalAgentEvent] = []
                var cancellationStartedAt: Date?
                for try await event in await MacAgentHostRuntime.shared.run(request) {
                    captured.append(event)
                    if event.kind == .permissionRequested, let permission = event.permissionRequest {
                        try await MacAgentHostRuntime.shared.resolvePermission(
                            conversationID: conversationID, request: permission, allow: true, scope: .once
                        )
                    }
                    if event.kind == .toolStarted, event.title == "bash" {
                        cancellationStartedAt = Date()
                        await MacAgentHostRuntime.shared.cancel(conversationID: conversationID)
                    }
                }
                if let cancellationStartedAt {
                    // Bound cancellation latency, not provider cold-prefill time.
                    XCTAssertLessThan(Date().timeIntervalSince(cancellationStartedAt), 10)
                }
                return captured
            }
            group.addTask {
                try await Task.sleep(for: .seconds(45))
                await MacAgentHostRuntime.shared.cancel(conversationID: conversationID)
                throw NSError(
                    domain: "NolaMacTests.RealCancellation",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Timed out waiting for run.cancelled"]
                )
            }
            let first = try await group.next() ?? []
            group.cancelAll()
            return first
        }
        let terminalKinds = events.compactMap { $0.coreTurnTerminal?.status }
        XCTAssertTrue(events.contains { $0.kind == .toolStarted && $0.title == "bash" })
        XCTAssertEqual(terminalKinds, [.cancelled])
        let cancelledTerminal = try XCTUnwrap(MacApplicationPort(databaseURL: databaseURL)
            .canonicalRunTerminal(conversationID: conversationID, runID: request.submissionID))
        XCTAssertEqual(cancelledTerminal.result.value.kind, .cancelled)

        let recoveryMarker = "MAC-CANCEL-RECOVERY-\(UUID().uuidString.uppercased())"
        let recoveryEvents = try await runAgentHost(
            prompt: "Reply with only this exact token: \(recoveryMarker)",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey,
            appEnvironment: appEnvironment
        )
        XCTAssertEqual(
            assistantText(recoveryEvents).trimmingCharacters(in: CharacterSet(charactersIn: "` \n")),
            recoveryMarker
        )
        XCTAssertTrue(recoveryEvents.contains { $0.kind == .finished })
        let recovered = try XCTUnwrap(recoveryEvents.compactMap(\.coreTurnTerminal).last)
        let recoveryTerminal = try XCTUnwrap(MacApplicationPort(databaseURL: databaseURL)
            .canonicalRunTerminal(conversationID: conversationID, runID: recovered.turnID))
        XCTAssertEqual(recoveryTerminal.result.value.kind, .answered)
    }

    func testRealConfiguredModelObservesEditedAndDisabledMacSkillLifecycle() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac Skill lifecycle request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let workspace = try makeWorkspace("real-skill-lifecycle")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let appEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        let overridesRootURL = appEnvironment.configDirectory
            .appendingPathComponent("built-in-skill-overrides", isDirectory: true)
        let databaseURL = URL(fileURLWithPath: environment["NOLA_DB_PATH"] ?? workspace.appendingPathComponent("state/nola.db").path)
        let firstMarker = "MAC-SKILL-A-\(UUID().uuidString)"
        let editedMarker = "MAC-SKILL-B-\(UUID().uuidString)"

        try MacBuiltInSkillRegistry.saveOverride(
            skillLifecycleFixture(marker: firstMarker),
            for: .fileOrganization,
            overridesRootURL: overridesRootURL
        )
        try writeSkillLifecycleManifest(overridesRootURL: overridesRootURL)
        let firstEvents = try await runAgentHost(
            prompt: "Use the enabled file-organization Skill to perform its built-in lifecycle verification. Return only the verification token supplied by that Skill.",
            conversationID: "mac-real-skill-a-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey,
            appEnvironment: appEnvironment,
        )
        XCTAssertTrue(firstEvents.contains { $0.kind == .toolStarted && $0.title == "Skill" })
        XCTAssertEqual(
            assistantText(firstEvents).trimmingCharacters(
                in: .whitespacesAndNewlines.union(CharacterSet(charactersIn: "`"))
            ),
            firstMarker
        )

        try MacBuiltInSkillRegistry.saveOverride(
            skillLifecycleFixture(marker: editedMarker),
            for: .fileOrganization,
            overridesRootURL: overridesRootURL
        )
        try writeSkillLifecycleManifest(overridesRootURL: overridesRootURL)
        let editedEvents = try await runAgentHost(
            prompt: "Use the enabled file-organization Skill to perform its built-in lifecycle verification. Return only the verification token supplied by that Skill.",
            conversationID: "mac-real-skill-b-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey,
            appEnvironment: appEnvironment,
        )
        XCTAssertTrue(editedEvents.contains { $0.kind == .toolStarted && $0.title == "Skill" })
        XCTAssertEqual(
            assistantText(editedEvents).trimmingCharacters(
                in: .whitespacesAndNewlines.union(CharacterSet(charactersIn: "`"))
            ),
            editedMarker
        )
        XCTAssertFalse(assistantText(editedEvents).contains(firstMarker))

        try MacBuiltInSkillRegistry.setEnabled(
            false,
            for: .fileOrganization,
            overridesRootURL: overridesRootURL,
            store: try MacApplicationPort(databaseURL: databaseURL)
        )
        let disabledEvents = try await runAgentHost(
            prompt: "Use the enabled file-organization Skill to perform its built-in lifecycle verification. If that Skill is unavailable, state that it is unavailable without inventing a verification token.",
            conversationID: "mac-real-skill-disabled-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey,
            appEnvironment: appEnvironment,
        )
        XCTAssertFalse(assistantText(disabledEvents).contains(editedMarker))
        XCTAssertFalse(
            disabledEvents.contains {
                $0.kind == .toolFinished && $0.title == "Skill" && !$0.isError
            },
            "A disabled Skill must not load successfully through the real Agent Host."
        )
    }

    func testRealConfiguredModelPersistsAndRecallsMemoryAcrossMacAgentHostConversations() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac Agent Host request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let workspace = try makeWorkspace("real-memory")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = URL(fileURLWithPath: environment["NOLA_DB_PATH"] ?? workspace.appendingPathComponent("state/nola.db").path)
        let memoryCode = "MAC-WORKSPACE-\(UUID().uuidString)"

        let writeEvents = try await runAgentHost(
            prompt: "Remember this across future conversations: my preferred Mac workspace label is \(memoryCode). Store that exact non-sensitive labeled preference in long-term memory. Do not use files. After memory is actually saved, return only MEMORY_WRITE_OK.",
            conversationID: "mac-real-memory-write-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey
        )
        XCTAssertEqual(
            writeEvents.filter { $0.kind == .toolStarted && $0.title == "memory_save" }.count,
            1
        )
        XCTAssertFalse(writeEvents.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(assistantText(writeEvents).contains("MEMORY_WRITE_OK"))

        let recallEvents = try await runAgentHost(
            prompt: "This is a new conversation. Recall my preferred Mac workspace label from long-term memory. Do not infer or guess. Return only the exact saved label.",
            conversationID: "mac-real-memory-recall-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false,
            modelID: model,
            apiKey: apiKey
        )
        let searchCount = recallEvents.filter {
            $0.kind == .toolStarted && $0.title == "memory_search"
        }.count
        XCTAssertTrue((1...3).contains(searchCount))
        XCTAssertFalse(recallEvents.contains { $0.kind == .permissionRequested })
        let recalledText = assistantText(recallEvents)
        XCTAssertTrue(
            recalledText.contains(memoryCode),
            "Expected the recalled response to contain the exact saved code, got: \(recalledText)"
        )
    }

    func testLongTermMemoryPersistsAcrossAgentHostConversations() async throws {
        let workspace = try makeWorkspace("memory")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["我叫老白"],
                    "required_tools": ["memory_save"],
                    "tool_name": "memory_save",
                    "tool_args": [
                        "scope": "global",
                        "kind": "user",
                        "content": "用户的名字是老白",
                        "summary": "用户叫老白",
                        "source": "explicit_user_statement",
                    ],
                ],
                [
                    "message_contains": ["memory_saved:", "用户的名字是老白"],
                    "content": "MEMORY_SAVE_COMPLETE",
                ],
                [
                    "message_contains": ["我叫什么"],
                    "required_tools": ["memory_search"],
                    "tool_name": "memory_search",
                    "tool_args": [
                        "scope": "global",
                        "query": "名字",
                        "kind": "user",
                        "limit": 10,
                    ],
                ],
                [
                    "message_contains": ["用户的名字是老白"],
                    "content": "MEMORY_RECALL_COMPLETE 老白",
                ],
            ]
        )
        defer { server.stop() }

        let saveEvents = try await runAgentHost(
            prompt: "我叫老白",
            conversationID: "memory-save-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL
        )
        XCTAssertTrue(saveEvents.contains { $0.kind == .toolStarted && $0.title == "memory_save" })
        XCTAssertFalse(
            saveEvents.contains { $0.kind == .permissionRequested },
            "Internal long-term memory writes must not request user permission"
        )
        XCTAssertTrue(assistantText(saveEvents).contains("MEMORY_SAVE_COMPLETE"))

        let recallEvents = try await runAgentHost(
            prompt: "我叫什么",
            conversationID: "memory-recall-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL
        )
        XCTAssertTrue(recallEvents.contains { $0.kind == .toolStarted && $0.title == "memory_search" })
        XCTAssertTrue(assistantText(recallEvents).contains("MEMORY_RECALL_COMPLETE 老白"))
        try assertCompleteReport(server, expectedStageCount: 4)
    }

    func testDisabledLongTermMemoryIsNotExposedToAgentHost() async throws {
        let workspace = try makeWorkspace("memory-disabled")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["我叫老白"],
                    "forbidden_tools": ["memory_save", "memory_search"],
                    "content": "MEMORY_DISABLED_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "我叫老白",
            conversationID: "memory-disabled-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db")
        )
        XCTAssertFalse(events.contains { $0.title == "memory_save" || $0.title == "memory_search" })
        XCTAssertTrue(assistantText(events).contains("MEMORY_DISABLED_COMPLETE"))
        try assertCompleteReport(server, expectedStageCount: 1)
    }

    func testReadFileExecutesWithoutPermissionAndReturnsRealContent() async throws {
        let workspace = try makeWorkspace("read-file")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let sourceURL = workspace.appendingPathComponent("source.txt")
        try "READ_FILE_E2E_SENTINEL".write(to: sourceURL, atomically: true, encoding: .utf8)
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["读取 source.txt"],
                    "required_tools": ["read_file"],
                    "tool_name": "read_file",
                    "tool_args": ["path": sourceURL.path],
                ],
                [
                    "message_contains": ["READ_FILE_E2E_SENTINEL"],
                    "content": "READ_FILE_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "读取 source.txt",
            conversationID: "read-file-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db")
        )

        XCTAssertTrue(events.contains { $0.kind == .toolStarted && $0.title == "read_file" })
        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(assistantText(events).contains("READ_FILE_COMPLETE"))
        try assertCompleteReport(server, expectedStageCount: 2)
    }

    func testRecommendedProfileWritesFileWithoutApproval() async throws {
        let workspace = try makeWorkspace("write-file-approved")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let destinationURL = workspace.appendingPathComponent("approved.txt")
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["创建 approved.txt"],
                    "required_tools": ["write_file"],
                    "tool_name": "write_file",
                    "tool_args": [
                        "path": destinationURL.path,
                        "content": "WRITE_FILE_E2E_SENTINEL",
                    ],
                ],
                [
                    "message_contains": ["approved.txt"],
                    "required_tools": ["read_file"],
                    "tool_name": "read_file",
                    "tool_args": [
                        "path": destinationURL.path,
                        "operation_kind": "validate",
                    ],
                ],
                [
                    "message_contains": ["approved.txt"],
                    "content": "WRITE_FILE_APPROVED_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "创建 approved.txt",
            conversationID: "write-approved-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db")
        )

        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
        XCTAssertEqual(
            try String(contentsOf: destinationURL, encoding: .utf8),
            "WRITE_FILE_E2E_SENTINEL"
        )
        XCTAssertTrue(assistantText(events).contains("WRITE_FILE_APPROVED_COMPLETE"))
        try assertCompleteReport(server, expectedStageCount: 3)
    }

    func testDeniedDeleteFileLeavesFileUntouched() async throws {
        let workspace = try makeWorkspace("delete-file-denied")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let destinationURL = workspace.appendingPathComponent("protected.txt")
        try "MUST_NOT_BE_DELETED".write(
            to: destinationURL,
            atomically: true,
            encoding: .utf8
        )
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["删除 protected.txt"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm protected.txt",
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["Permission denied"],
                    "content": "DELETE_FILE_DENIED_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "删除 protected.txt",
            conversationID: "delete-denied-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db"),
            allowPermissions: false
        )

        XCTAssertTrue(events.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(FileManager.default.fileExists(atPath: destinationURL.path))
        XCTAssertEqual(
            try String(contentsOf: destinationURL, encoding: .utf8),
            "MUST_NOT_BE_DELETED"
        )
        XCTAssertTrue(
            assistantText(events).contains("DELETE_FILE_DENIED_COMPLETE"),
            "Permission denial did not return a final assistant answer. Events:\n\(allEventText(events))"
        )
        try assertCompleteReport(server, expectedStageCount: 2)
    }

    func testConversationPermissionDecisionSerializesConversationScope() throws {
        let request = MacRemotePermissionRequest(
            id: "permission-request",
            allowConversation: true,
            protocolVersion: 1,
            nonce: "permission-nonce",
            hostID: "local",
            clientID: "mac",
            sessionID: "permission-session",
            toolName: "bash",
            arguments: #"{"command":"make test","operation_kind":"execute"}"#,
            reason: "Bash requires approval",
            message: "Allow Bash?",
            expiresAt: 4_102_444_800
        )

        let allowed = request.decision(allow: true, scope: .conversation).payload
        let allowedResolution = allowed["resolution"] as? [String: Any]
        XCTAssertEqual(allowedResolution?["type"] as? String, "authorization")
        XCTAssertEqual(allowedResolution?["decision"] as? String, "allow_conversation")
        XCTAssertNil(allowed["decision"])
        XCTAssertEqual(allowed["session_id"] as? String, "permission-session")
        XCTAssertEqual(allowed["tool_name"] as? String, "bash")

        let denied = request.decision(allow: false, scope: .conversation).payload
        let deniedResolution = denied["resolution"] as? [String: Any]
        XCTAssertEqual(deniedResolution?["type"] as? String, "authorization")
        XCTAssertEqual(deniedResolution?["decision"] as? String, "deny")
        XCTAssertNil(denied["decision"])
    }

    func testOpaqueProcessRequiresFreshOnceApprovalAcrossAgentHostRuns() async throws {
        let workspace = try makeWorkspace("permission-conversation")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let conversationID = "permission-conversation-\(UUID().uuidString)"
        let firstURL = workspace.appendingPathComponent("first.txt")
        let secondURL = workspace.appendingPathComponent("second.txt")
        let otherURL = workspace.appendingPathComponent("other.txt")
        for url in [firstURL, secondURL, otherURL] {
            try "delete-me".write(to: url, atomically: true, encoding: .utf8)
        }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["运行第一条会话命令"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm first.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["first.txt"],
                    "content": "CONVERSATION_PERMISSION_FIRST_RUN_COMPLETE",
                ],
                [
                    "message_contains": ["运行第二条会话命令"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm second.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["second.txt"],
                    "content": "CONVERSATION_PERMISSION_SECOND_RUN_COMPLETE",
                ],
                [
                    "message_contains": ["运行不同脚本"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "unlink other.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["other.txt"],
                    "content": "CONVERSATION_PERMISSION_DIFFERENT_SCOPE_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let firstRun = try await runAgentHost(
            prompt: "运行第一条会话命令",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once,
            deleteSessionAfterRun: false
        )
        XCTAssertEqual(
            firstRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "The first opaque command must request once permission; "
                + firstRun
                    .filter { $0.kind == .permissionRequested }
                    .map {
                        "\($0.permissionRequest?.toolName ?? $0.title ?? "unknown"): "
                            + ($0.permissionRequest?.arguments ?? $0.text)
                    }
                    .joined(separator: " | ")
        )
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: firstURL.path),
            "Approved command did not execute. Events: \(allEventText(firstRun))"
        )
        XCTAssertTrue(firstRun.compactMap(\.permissionRequest).allSatisfy { !$0.allowConversation })
        XCTAssertTrue(FileManager.default.fileExists(atPath: secondURL.path))
        XCTAssertTrue(
            assistantText(firstRun).contains("CONVERSATION_PERMISSION_FIRST_RUN_COMPLETE")
        )

        let secondRun = try await runAgentHost(
            prompt: "运行第二条会话命令",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false
        )
        XCTAssertEqual(secondRun.filter { $0.kind == .permissionRequested }.count, 1,
            "A prior opaque process approval cannot authorize a new invocation")
        XCTAssertTrue(secondRun.compactMap(\.permissionRequest).allSatisfy { !$0.allowConversation })
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: secondURL.path),
            "Reused command did not execute. Events: \(allEventText(secondRun))"
        )
        XCTAssertTrue(
            assistantText(secondRun).contains("CONVERSATION_PERMISSION_SECOND_RUN_COMPLETE")
        )

        let differentScopeRun = try await runAgentHost(
            prompt: "运行不同脚本",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once
        )
        XCTAssertEqual(
            differentScopeRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "A conversation grant for one script must not authorize a different script"
        )
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: otherURL.path),
            "Different-scope command did not execute. Events: \(allEventText(differentScopeRun))"
        )
        XCTAssertTrue(
            assistantText(differentScopeRun).contains(
                "CONVERSATION_PERMISSION_DIFFERENT_SCOPE_COMPLETE"
            ),
            "Expected final assistant output. Events: "
                + differentScopeRun.map {
                    "\($0.kind) title=\($0.title ?? "") text=\($0.text)"
                }.joined(separator: " | ")
        )
        try assertCompleteReport(server, expectedStageCount: 6)
    }

    func testCautiousConversationGrantReusesOnlyOriginalResourceAndOwner() async throws {
        let workspace = try makeWorkspace("permission-reuse-contract")
        defer { try? FileManager.default.removeItem(at: workspace) }
        var steps: [[String: Any]] = []
        for (index, name) in ["approved.txt", "approved.txt", "outside.txt", "approved.txt"].enumerated() {
            steps.append([
                "message_contains": ["PERMISSION_REUSE_\(index)"],
                "required_tools": ["delete_file"],
                "tool_name": "delete_file",
                "tool_args": ["path": name, "dry_run": false],
            ])
            steps.append([
                "message_contains": [index < 2 ? "Deleted" : "Permission denied"],
                "content": "PERMISSION_REUSE_\(index)_DONE",
            ])
        }
        let server = try MacAgentCapabilityMockServer(workspace: workspace, steps: steps)
        defer { server.stop() }
        try await assertCautiousConversationGrantReuse(workspace: workspace, endpoint: server.endpoint)
        try assertCompleteReport(server, expectedStageCount: 8)
    }

    func testRealConfiguredModelCautiousGrantReusePreservesResourceAndConversationBoundary() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Real model permission reuse requires explicit opt-in.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        // The runner may supply a loopback recording proxy for the LAN model.
        let workspace = try makeWorkspace("real-permission-reuse")
        defer { try? FileManager.default.removeItem(at: workspace) }
        try await assertCautiousConversationGrantReuse(
            workspace: workspace, endpoint: endpoint,
            modelID: try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"]),
            apiKey: try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        )
    }

    private func assertCautiousConversationGrantReuse(
        workspace: URL, endpoint: String,
        modelID: String = "nola-capability-e2e", apiKey: String = "local-e2e-key"
    ) async throws {
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let owner = "permission-reuse-\(UUID().uuidString)"
        let otherOwner = "permission-isolation-\(UUID().uuidString)"
        let approved = workspace.appendingPathComponent("approved.txt")
        let outside = workspace.appendingPathComponent("outside.txt")
        let original = "SYNTHETIC_PERMISSION_FIXTURE_\(UUID().uuidString)\n"
        try original.write(to: approved, atomically: true, encoding: .utf8)
        try original.write(to: outside, atomically: true, encoding: .utf8)
        for index in 0..<4 {
            if index == 1 || index == 3 {
                // The test, not the assistant, recreates exactly the disposable
                // resource. This is not a new permission or a production edit.
                try original.write(to: approved, atomically: true, encoding: .utf8)
            }
            let target = index == 2 ? outside : approved
            let events = try await runAgentHost(
                prompt: "PERMISSION_REUSE_\(index): Delete only \(target.lastPathComponent) from this disposable workspace using the file deletion capability. Discover it if necessary. Do not use shell, network, alternate tools or touch other files. If denied, stop and report denial; do not retry. This is an actual deletion, not a preview.",
                conversationID: index == 3 ? otherOwner : owner,
                workspace: workspace, endpoint: endpoint, memoryEnabled: false,
                databaseURL: databaseURL, allowPermissions: index < 2,
                permissionScope: .conversation, deleteSessionAfterRun: false,
                modelID: modelID, apiKey: apiKey, maxModelIterations: 8
            )
            let requests = events.compactMap(\.permissionRequest)
            XCTAssertEqual(requests.count, index == 1 ? 0 : 1,
                "Round \(index) must distinguish original grant, reuse, resource change and owner change. \(allEventText(events))")
            if index == 0 {
                XCTAssertTrue(requests.allSatisfy(\.allowConversation), "The grant must actually permit conversation lifetime")
            }
            if index < 2 {
                XCTAssertFalse(FileManager.default.fileExists(atPath: target.path),
                    "A final answer without the independently observed deletion is not success")
            } else {
                XCTAssertEqual(try String(contentsOf: target, encoding: .utf8), original,
                    "Denied or cross-session resource was changed")
            }
            XCTAssertEqual(try String(contentsOf: outside, encoding: .utf8), original)
        }
        try await MacAgentHostRuntime.shared.deleteSession(id: owner)
        try await MacAgentHostRuntime.shared.deleteSession(id: otherOwner)
    }

    func testRecommendedProfileAllowsRepeatedFileWritesAcrossAgentHostRuns() async throws {
        let workspace = try makeWorkspace("permission-file-write-conversation")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let conversationID = "permission-file-write-\(UUID().uuidString)"
        let firstURL = workspace.appendingPathComponent("first.txt")
        let secondURL = workspace.appendingPathComponent("second.txt")
        let thirdURL = workspace.appendingPathComponent("third.txt")
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["写入前两个文件"],
                    "required_tools": ["write_file"],
                    "tool_name": "write_file",
                    "tool_args": ["path": "first.txt", "content": "first"],
                ],
                [
                    "message_contains": ["first.txt"],
                    "required_tools": ["write_file"],
                    "tool_name": "write_file",
                    "tool_args": ["path": "second.txt", "content": "second"],
                ],
                [
                    "message_contains": ["second.txt"],
                    "required_tools": ["read_file"],
                    "tool_name": "read_file",
                    "tool_args": [
                        "path": "first.txt",
                        "operation_kind": "validate",
                    ],
                ],
                [
                    "message_contains": ["first.txt"],
                    "required_tools": ["read_file"],
                    "tool_name": "read_file",
                    "tool_args": [
                        "path": "second.txt",
                        "operation_kind": "validate",
                    ],
                ],
                [
                    "message_contains": ["second.txt"],
                    "content": "FILE_WRITE_PERMISSION_FIRST_RUN_COMPLETE",
                ],
                [
                    "message_contains": ["写入第三个文件"],
                    "required_tools": ["write_file"],
                    "tool_name": "write_file",
                    "tool_args": ["path": "third.txt", "content": "third"],
                ],
                [
                    "message_contains": ["third.txt"],
                    "required_tools": ["read_file"],
                    "tool_name": "read_file",
                    "tool_args": [
                        "path": "third.txt",
                        "operation_kind": "validate",
                    ],
                ],
                [
                    "message_contains": ["third.txt"],
                    "content": "FILE_WRITE_PERMISSION_SECOND_RUN_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let firstRun = try await runAgentHost(
            prompt: "写入前两个文件",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .conversation,
            deleteSessionAfterRun: false
        )
        XCTAssertEqual(
            firstRun.filter { $0.kind == .permissionRequested }.count,
            0,
            "Reversible writes must not request permission in the recommended profile"
        )
        XCTAssertEqual(try String(contentsOf: firstURL, encoding: .utf8), "first")
        XCTAssertEqual(try String(contentsOf: secondURL, encoding: .utf8), "second")

        let secondRun = try await runAgentHost(
            prompt: "写入第三个文件",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            deleteSessionAfterRun: false
        )
        XCTAssertFalse(
            secondRun.contains { $0.kind == .permissionRequested },
            "A restarted Host must keep reversible writes non-interactive"
        )
        XCTAssertEqual(try String(contentsOf: thirdURL, encoding: .utf8), "third")
        XCTAssertTrue(
            assistantText(secondRun).contains("FILE_WRITE_PERMISSION_SECOND_RUN_COMPLETE")
        )
        try assertCompleteReport(server, expectedStageCount: 8)
    }

    func testOncePermissionDoesNotPersistAcrossAgentHostRuns() async throws {
        let workspace = try makeWorkspace("permission-once")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let conversationID = "permission-once-\(UUID().uuidString)"
        for name in ["first-once.txt", "second-once.txt", "third-once.txt"] {
            try "delete-me".write(
                to: workspace.appendingPathComponent(name),
                atomically: true,
                encoding: .utf8
            )
        }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["运行第一条单次授权命令"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm first-once.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["first-once.txt"],
                    "content": "ONCE_PERMISSION_FIRST_RUN_COMPLETE",
                ],
                [
                    "message_contains": ["运行第二条单次授权命令"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm second-once.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["second-once.txt"],
                    "content": "ONCE_PERMISSION_SECOND_RUN_COMPLETE",
                ],
                [
                    "message_contains": ["运行第三条单次授权命令"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm third-once.txt",
                        "cwd": workspace.path,
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["third-once.txt"],
                    "content": "ONCE_PERMISSION_THIRD_RUN_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let firstRun = try await runAgentHost(
            prompt: "运行第一条单次授权命令",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once,
            deleteSessionAfterRun: false
        )
        XCTAssertEqual(
            firstRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "The first run must request permission once"
        )
        XCTAssertFalse(
            FileManager.default.fileExists(
                atPath: workspace.appendingPathComponent("first-once.txt").path
            ),
            "First allow-once command did not execute. Events: \(allEventText(firstRun))"
        )

        let secondRun = try await runAgentHost(
            prompt: "运行第二条单次授权命令",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once,
            deleteSessionAfterRun: false
        )
        XCTAssertEqual(
            secondRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "Allow once must not survive an Agent Host restart"
        )
        XCTAssertFalse(
            FileManager.default.fileExists(
                atPath: workspace.appendingPathComponent("second-once.txt").path
            ),
            "Second allow-once command did not execute. Events: \(allEventText(secondRun))"
        )

        let thirdRun = try await runAgentHost(
            prompt: "运行第三条单次授权命令",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once
        )
        XCTAssertEqual(
            thirdRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "Allow once must remain non-durable across repeated Host restarts"
        )
        XCTAssertFalse(
            FileManager.default.fileExists(
                atPath: workspace.appendingPathComponent("third-once.txt").path
            ),
            "Third allow-once command did not execute. Events: \(allEventText(thirdRun))"
        )
        try assertCompleteReport(server, expectedStageCount: 6)
    }

    func testOnceOpaqueProcessApprovalDoesNotLeakIntoAnotherConversation() async throws {
        let workspace = try makeWorkspace("permission-isolation")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        for name in ["first-session.txt", "second-session.txt"] {
            try "delete-me".write(
                to: workspace.appendingPathComponent(name),
                atomically: true,
                encoding: .utf8
            )
        }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["授权第一段对话"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm first-session.txt",
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["first-session.txt"],
                    "content": "FIRST_CONVERSATION_COMPLETE",
                ],
                [
                    "message_contains": ["运行第二段对话"],
                    "required_tools": ["bash"],
                    "tool_name": "bash",
                    "tool_args": [
                        "command": "rm second-session.txt",
                        "operation_kind": "execute",
                    ],
                ],
                [
                    "message_contains": ["second-session.txt"],
                    "content": "SECOND_CONVERSATION_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let firstRun = try await runAgentHost(
            prompt: "执行命令并授权第一段对话",
            conversationID: "permission-isolation-a-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once
        )
        XCTAssertEqual(firstRun.filter { $0.kind == .permissionRequested }.count, 1)
        XCTAssertTrue(firstRun.compactMap(\.permissionRequest).allSatisfy { !$0.allowConversation })
        XCTAssertFalse(FileManager.default.fileExists(atPath: workspace.appendingPathComponent("first-session.txt").path))

        let secondRun = try await runAgentHost(
            prompt: "运行第二段对话命令",
            conversationID: "permission-isolation-b-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL,
            permissionScope: .once
        )
        XCTAssertEqual(
            secondRun.filter { $0.kind == .permissionRequested }.count,
            1,
            "A once-only opaque process approval cannot authorize another conversation"
        )
        XCTAssertTrue(secondRun.compactMap(\.permissionRequest).allSatisfy { !$0.allowConversation })
        XCTAssertFalse(FileManager.default.fileExists(atPath: workspace.appendingPathComponent("second-session.txt").path))
        try assertCompleteReport(server, expectedStageCount: 4)
    }

    func testDesktopAndBrowserToolsEnterRealExecutionPath() async throws {
        let workspace = try makeWorkspace("native-execution")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let screenshotURL = workspace.appendingPathComponent("screen.png")
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["执行截图工具"],
                    "required_tools": ["tool_search"],
                    "tool_name": "tool_search",
                    "tool_args": ["query": "select:computer_screenshot"],
                ],
                [
                    "required_tools": ["computer_screenshot"],
                    "tool_name": "computer_screenshot",
                    "tool_args": ["path": screenshotURL.path],
                ],
                ["content": "SCREENSHOT_EXECUTION_COMPLETE"],
                [
                    "message_contains": ["查找不存在的辅助功能元素"],
                    "required_tools": ["tool_search"],
                    "tool_name": "tool_search",
                    "tool_args": ["query": "select:computer_click_element"],
                ],
                [
                    "required_tools": ["computer_click_element"],
                    "tool_name": "computer_click_element",
                    "tool_args": ["query": "__NOLA_E2E_ELEMENT_THAT_DOES_NOT_EXIST__"],
                ],
                ["content": "ACCESSIBILITY_EXECUTION_COMPLETE"],
                [
                    "message_contains": ["打开无效测试地址"],
                    "required_tools": ["browser_navigate"],
                    "forbidden_tools": ["browser_open_page"],
                    "tool_name": "browser_navigate",
                    "tool_args": [
                        "target": "attached",
                        "url": "https://example.invalid/nola-e2e",
                    ],
                ],
                ["content": "BROWSER_EXECUTION_COMPLETE"],
            ]
        )
        defer { server.stop() }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")

        let screenshotEvents = try await runAgentHost(
            prompt: "执行截图工具",
            conversationID: "screenshot-execution-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL
        )
        XCTAssertTrue(
            screenshotEvents.contains {
                $0.kind == .toolStarted && $0.title == "computer_screenshot"
            }
        )
        let screenshotFinished = try XCTUnwrap(
            screenshotEvents.first {
                $0.kind == .toolFinished && $0.title == "computer_screenshot"
            },
            "Screenshot execution never reached tool.finished"
        )
        if screenshotFinished.isError {
            XCTAssertTrue(
                screenshotFinished.artifactURLs.isEmpty,
                "A failed screenshot must not expose a nonexistent artifact"
            )
            XCTAssertFalse(
                FileManager.default.fileExists(atPath: screenshotURL.path),
                "A failed screenshot unexpectedly left an artifact behind"
            )
        } else {
            XCTAssertTrue(
                screenshotFinished.artifactURLs.contains(screenshotURL.standardizedFileURL),
                "Successful screenshot result did not expose its structured local artifact"
            )
            XCTAssertTrue(
                FileManager.default.fileExists(atPath: screenshotURL.path),
                "Screenshot tool reported completion without creating its artifact"
            )
        }
        XCTAssertTrue(assistantText(screenshotEvents).contains("SCREENSHOT_EXECUTION_COMPLETE"))

        let accessibilityEvents = try await runAgentHost(
            prompt: "查找不存在的辅助功能元素",
            conversationID: "accessibility-execution-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL
        )
        XCTAssertTrue(
            accessibilityEvents.contains {
                $0.kind == .toolStarted && $0.title == "computer_click_element"
            },
            "Accessibility execution never reached tool.started: "
                + accessibilityEvents.map {
                    "\($0.trace?.kind ?? "missing-trace"):\($0.title ?? "")"
                }.joined(separator: ", ")
        )
        XCTAssertTrue(
            assistantText(accessibilityEvents).contains("ACCESSIBILITY_EXECUTION_COMPLETE")
        )

        let browserEvents = try await runAgentHost(
            prompt: "打开无效测试地址",
            conversationID: "browser-execution-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL
        )
        XCTAssertTrue(
            browserEvents.contains {
                $0.kind == .toolStarted && $0.title == "browser_navigate"
            },
            "Browser request never entered the unified attached-Chrome execution path"
        )
        XCTAssertFalse(
            browserEvents.contains {
                $0.kind == .toolStarted && $0.title == "browser_open_page"
            },
            "Browser request incorrectly fell back to the obsolete browser host"
        )
        let executionText = allEventText(screenshotEvents + accessibilityEvents + browserEvents)
        XCTAssertFalse(executionText.localizedCaseInsensitiveContains("playwright"))
        XCTAssertFalse(executionText.localizedCaseInsensitiveContains("pip install"))
        XCTAssertTrue(assistantText(browserEvents).contains("BROWSER_EXECUTION_COMPLETE"))
        try assertCompleteReport(server, expectedStageCount: 8)
    }

    func testComputerStatusRoundTripsThroughNativeMacBridge() async throws {
        let workspace = try makeWorkspace("native-computer-bridge")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["computer_status", "检查原生辅助功能执行器"],
                    "required_tools": ["tool_search"],
                    "tool_name": "tool_search",
                    "tool_args": ["query": "select:computer_status"],
                ],
                [
                    "required_tools": ["computer_status"],
                    "tool_name": "computer_status",
                    "tool_args": [:],
                ],
                [
                    "message_contains": [
                        "nola_mac_app",
                        "accessibility_permission",
                        "screen_recording_permission",
                        "permission_subject",
                    ],
                    "content": "NATIVE_COMPUTER_BRIDGE_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "调用 computer_status 检查原生辅助功能执行器",
            conversationID: "native-computer-bridge-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db")
        )

        XCTAssertTrue(
            events.contains { $0.kind == .toolStarted && $0.title == "computer_status" },
            "Agent Host never requested the native Mac computer status tool"
        )
        XCTAssertTrue(
            events.contains {
                $0.kind == .toolFinished
                    && $0.title == "computer_status"
                    && !$0.isError
            },
            "Native Mac computer status did not complete successfully"
        )
        XCTAssertFalse(
            events.contains { $0.kind == .permissionRequested },
            "Reading computer capability status must not request execution permission"
        )
        XCTAssertTrue(
            assistantText(events).contains("NATIVE_COMPUTER_BRIDGE_COMPLETE"),
            "The mock LLM never received the native Mac bridge result"
        )
        try assertCompleteReport(server, expectedStageCount: 3)
    }

    func testConversationRecallRoundTripsOriginalTranscriptAfterPersistedCompactionCheckpoints() async throws {
        let workspace = try makeWorkspace("conversation-recall")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "conversation-recall-\(UUID().uuidString)"
        _ = isolatedConversationRepository(workspace: workspace)
        let originalMessage = MacChatMessage(
            role: .user,
            text: "Project Aurora must keep the Tokyo launch date of October 14.",
            createdAt: Date(timeIntervalSince1970: 100)
        )
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let applicationPort = try MacApplicationPort(databaseURL: databaseURL)
        let canonicalRepository = MacCanonicalConversationRepository(
            store: applicationPort,
        )
        _ = try canonicalRepository.restoreBackupSnapshot([MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Aurora",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [originalMessage],
                knowledge: MacConversationKnowledge(
                    keyPointsMarkdown: "- Aurora launches in Tokyo",
                    wikiMarkdown: "# Aurora\n\nLaunch owner: Mina",
                    updatedAt: .now
                )
            )]
        )
        for index in 1...3 {
            _ = try commitCanonicalTestContextCheckpoint(
                store: applicationPort,
                id: "aurora-summary-\(index)",
                conversationID: conversationID,
                summary: "Compaction \(index) retained Aurora planning state.",
                estimatedTokens: 12,
                payloadDigest: String(repeating: "a", count: 64),
                createdAtMs: Int64(100 + index) * 1_000
            )
        }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["回忆 Aurora 的东京发布日期"],
                    "required_tools": ["conversation_recall"],
                    "tool_name": "conversation_recall",
                    "tool_args": ["query": "Tokyo launch date", "limit": 8],
                ],
                [
                    "message_contains": [
                        "October 14",
                        "message_id",
                        "part_id",
                        "originalMessage",
                    ],
                    "content": "CONVERSATION_RECALL_COMPLETE October 14",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "回忆 Aurora 的东京发布日期",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: databaseURL
        )

        XCTAssertTrue(events.contains { $0.kind == .toolStarted && $0.title == "conversation_recall" })
        XCTAssertTrue(
            events.contains {
                $0.kind == .toolFinished
                    && $0.title == "conversation_recall"
                    && !$0.isError
            }
        )
        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(assistantText(events).contains("CONVERSATION_RECALL_COMPLETE October 14"))
        try assertCompleteReport(server, expectedStageCount: 2)
    }

    func testRealAgentHostRepeatedCompactionPreservesOriginalConversationRecall() async throws {
        let workspace = try makeWorkspace("real-repeated-compaction")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let conversationID = "real-repeated-compaction-\(UUID().uuidString)"
        let databaseURL = workspace.appendingPathComponent("state/nola.db")
        let applicationPort = try MacApplicationPort(databaseURL: databaseURL)
        let canonicalRepository = MacCanonicalConversationRepository(
            store: applicationPort,
        )
        let originalText = "Project Aurora immutable original: Tokyo launch date is October 14; owner is Mina."
        _ = try canonicalRepository.restoreBackupSnapshot([MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Aurora compaction E2E",
                    preview: originalText,
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [MacChatMessage(role: .user, text: originalText, createdAt: .now)],
                knowledge: MacConversationKnowledge(
                    keyPointsMarkdown: "- Aurora launches in Tokyo on October 14",
                    wikiMarkdown: "# Aurora\n\nLaunch owner: Mina",
                    updatedAt: .now
                )
            )]
        )

        let turnCount = 24
        var steps: [[String: Any]] = (0..<turnCount).map { index in
            [
                "message_contains": ["REAL_COMPACTION_TURN_\(index)"],
                "content": "REAL_COMPACTION_ACK_\(index)",
            ]
        }
        steps.append([
            "message_contains": ["回忆 Aurora 最初的发布日期和负责人"],
            "required_tools": ["conversation_recall"],
            "tool_name": "conversation_recall",
            "tool_args": ["query": "Project Aurora immutable original", "limit": 12],
        ])
        steps.append([
            "message_contains": [
                "Project Aurora immutable original",
                "October 14",
                "owner is Mina",
                "message_id",
                "part_id",
            ],
            "content": "REAL_COMPACTION_RECALL_COMPLETE October 14 Mina",
        ])
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: steps,
            contextWindow: 16_000,
            compactionSummary: "# Continuation\n\nAurora work remains active. Preserve exact original transcript through conversation recall."
        )
        defer { server.stop() }

        var allTurnEvents: [MacLocalAgentEvent] = []
        for index in 0..<turnCount {
            let filler = String(repeating: "bounded-context-payload-\(index)-", count: 220)
            let events = try await runAgentHost(
                prompt: "REAL_COMPACTION_TURN_\(index) \(filler)",
                conversationID: conversationID,
                workspace: workspace,
                endpoint: server.endpoint,
                memoryEnabled: false,
                contextWindowTokens: 16_000,
                databaseURL: databaseURL,
                deleteSessionAfterRun: false
            )
            XCTAssertTrue(assistantText(events).contains("REAL_COMPACTION_ACK_\(index)"))
            allTurnEvents.append(contentsOf: events)
        }

        let contextUsage = allTurnEvents.compactMap { event -> (used: Int, window: Int)? in
            guard let used = event.contextUsedTokens,
                  let window = event.contextWindowTokens else { return nil }
            return (used, window)
        }
        let observedWindows = Set(contextUsage.map(\.window))
        let maximumObservedUsage = contextUsage.map(\.used).max() ?? 0
        XCTAssertFalse(contextUsage.isEmpty, "The real Agent Host emitted no context usage events")
        XCTAssertEqual(
            observedWindows,
            [16_000],
            "The Mac-selected context window was replaced before reaching Core: \(observedWindows)"
        )
        let preRecallReport = try server.report()
        let compactionEvents = allTurnEvents.filter { $0.trace?.kind == "context.compacted" }
        XCTAssertGreaterThanOrEqual(
            compactionEvents.count,
            3,
            "The real Core/Agent Host path did not perform repeated automatic compaction; "
                + "provider compactions=\(preRecallReport.compactionCount), "
                + "max usage=\(maximumObservedUsage), windows=\(observedWindows)"
        )
        let persistedCheckpointCount = try await canonicalRepository.contextCheckpointCountAsync(
            conversationID: conversationID
        )
        XCTAssertGreaterThanOrEqual(
            persistedCheckpointCount,
            3,
            "Mac production runtime did not persist repeated context.compacted traces"
        )
        let persistedConversation = try canonicalRepository.loadAll()
            .first(where: { $0.conversation.id == conversationID })
        XCTAssertTrue(
            persistedConversation?.messages.contains(where: { $0.text == originalText }) == true,
            "Compaction must never replace the original conversation transcript"
        )

        let recallEvents = try await runAgentHost(
            prompt: "回忆 Aurora 最初的发布日期和负责人",
            conversationID: conversationID,
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            contextWindowTokens: 16_000,
            databaseURL: databaseURL
        )
        XCTAssertTrue(recallEvents.contains { $0.kind == .toolStarted && $0.title == "conversation_recall" })
        XCTAssertFalse(recallEvents.contains { $0.kind == .permissionRequested })
        XCTAssertTrue(
            assistantText(recallEvents).contains("REAL_COMPACTION_RECALL_COMPLETE October 14 Mina")
        )

        let report = try server.report()
        XCTAssertGreaterThanOrEqual(report.compactionCount, 3)
        try assertCompleteReport(server, expectedStageCount: turnCount + 2)
    }

    func testAgentHostContinuesTruncatedMarkdownInsteadOfCompletingHalfAResponse() async throws {
        let workspace = try makeWorkspace("truncated-response")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "content": "BEGIN_UNIQUE\n```cpp\nstd::vector<std",
                    "finish_reason": "length",
                ],
                [
                    "message_contains": [
                        "The preceding assistant response reached the provider output limit"
                    ],
                    "content": "::string> values;\n```\nEND_UNIQUE",
                    "finish_reason": "stop",
                ],
            ]
        )
        defer { server.stop() }

        let events = try await runAgentHost(
            prompt: "Return a long fenced code response",
            conversationID: "truncated-response-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: false,
            databaseURL: workspace.appendingPathComponent("state/nola.db")
        )
        let answer = assistantText(events)
        XCTAssertTrue(answer.contains("BEGIN_UNIQUE"))
        XCTAssertTrue(answer.contains("std::vector<std::string> values;"))
        XCTAssertTrue(answer.contains("END_UNIQUE"))
        XCTAssertEqual(answer.components(separatedBy: "BEGIN_UNIQUE").count - 1, 1)
        XCTAssertEqual(answer.components(separatedBy: "END_UNIQUE").count - 1, 1)
        try assertCompleteReport(server, expectedStageCount: 2)
    }

    func testDesktopAndBrowserIntentsReachAgentHostToolCatalog() async throws {
        let workspace = try makeWorkspace("capability-routing")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let server = try MacAgentCapabilityMockServer(
            workspace: workspace,
            steps: [
                [
                    "message_contains": ["截个图"],
                    "required_tools": ["tool_search"],
                    "forbidden_tools": ["browser_open_page"],
                    "tool_name": "tool_search",
                    "tool_args": ["query": "select:computer_screenshot"],
                ],
                [
                    "required_tools": ["computer_screenshot"],
                    "content": "SCREENSHOT_ROUTE_COMPLETE",
                ],
                [
                    "message_contains": ["用辅助功能点击桌面上的窗口"],
                    "required_tools": ["tool_search"],
                    "tool_name": "tool_search",
                    "tool_args": ["query": "select:computer_click"],
                ],
                [
                    "required_tools": ["computer_click"],
                    "content": "ACCESSIBILITY_ROUTE_COMPLETE",
                ],
                [
                    "message_contains": ["打开 GitHub 网页"],
                    "required_tools": ["browser_navigate"],
                    "forbidden_tools": ["browser_open_page"],
                    "content": "BROWSER_ROUTE_COMPLETE",
                ],
            ]
        )
        defer { server.stop() }
        let databaseURL = workspace.appendingPathComponent("state/nola.db")

        let screenshotEvents = try await runAgentHost(
            prompt: "截个图",
            conversationID: "screenshot-route-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL
        )
        XCTAssertTrue(assistantText(screenshotEvents).contains("SCREENSHOT_ROUTE_COMPLETE"))

        let accessibilityEvents = try await runAgentHost(
            prompt: "用辅助功能点击桌面上的窗口",
            conversationID: "accessibility-route-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL
        )
        XCTAssertTrue(assistantText(accessibilityEvents).contains("ACCESSIBILITY_ROUTE_COMPLETE"))

        let browserEvents = try await runAgentHost(
            prompt: "打开 GitHub 网页",
            conversationID: "browser-route-\(UUID().uuidString)",
            workspace: workspace,
            endpoint: server.endpoint,
            memoryEnabled: true,
            databaseURL: databaseURL
        )
        let browserText = assistantText(browserEvents)
        server.attachTimingReport(to: self)
        XCTAssertTrue(browserText.contains("BROWSER_ROUTE_COMPLETE"))
        XCTAssertFalse(browserText.localizedCaseInsensitiveContains("playwright"))
        XCTAssertFalse(browserText.localizedCaseInsensitiveContains("pip install"))
        try assertCompleteReport(server, expectedStageCount: 5)
    }

    func testMockProviderStartupDeadlineDoesNotBlockOnSilentProcess() throws {
        let process = Process()
        let output = Pipe()
        let errors = Pipe()
        process.executableURL = URL(fileURLWithPath: "/bin/sleep")
        process.arguments = ["2"]
        process.standardOutput = output
        process.standardError = errors
        try process.run()
        defer { if process.isRunning { process.terminate() } }
        let started = ContinuousClock.now
        XCTAssertThrowsError(try MacAgentCapabilityMockServer.readStartupLine(
            output: output.fileHandleForReading,
            errors: errors.fileHandleForReading,
            process: process,
            timeout: 0.2
        )) { error in
            XCTAssertEqual((error as NSError).code, 3)
        }
        XCTAssertLessThan(started.duration(to: .now), .seconds(1))
    }

    private func runAgentHost(
        prompt: String,
        conversationID: String,
        workspace: URL,
        endpoint: String,
        memoryEnabled: Bool,
        contextWindowTokens: Int? = nil,
        databaseURL: URL,
        allowPermissions: Bool = true,
        permissionScope: MacPermissionGrantScope = .once,
        deleteSessionAfterRun: Bool = true,
        modelID: String = "nola-capability-e2e",
        apiKey: String = "local-e2e-key",
        appEnvironment: MacAppEnvironment? = nil,
        attachmentURLs: [URL] = [],
        modelExecutionCapabilities: MacModelExecutionCapabilities? = nil,
        maxModelIterations: Int = 0
    ) async throws -> [MacLocalAgentEvent] {
        let isolatedEnvironment = appEnvironment ?? MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        _ = isolatedConversationRepository(workspace: workspace)
        let realProtocol = ProcessInfo.processInfo.environment["NOLA_REAL_LLM_E2E"] == "1"
            ? ProcessInfo.processInfo.environment["NOLA_REAL_LLM_WIRE_PROTOCOL"] : nil
        let wireProtocol = try realProtocol.map { raw in
            try XCTUnwrap(MacModelWireProtocol(rawValue: raw), "Invalid real-provider wire protocol")
        }
        // Isolated truncation acceptance must spend its injected small allowance
        // on checkpoint text, not hidden reasoning. Apply the same declared
        // setting to every main request so hot forks still preserve the prefix.
        let testReasoningNone = ProcessInfo.processInfo.environment["NOLA_REAL_LLM_E2E"] == "1"
            && ProcessInfo.processInfo.environment["NOLA_REAL_COMPACTION_REASONING_NONE"] == "1"
        if testReasoningNone { XCTAssertEqual(wireProtocol, .responses) }
        let applicationPort = try MacApplicationPort(databaseURL: databaseURL)
        // nil is an explicit "conversation does not exist" CAS frontier, not
        // permission to skip admission checks. Every follow-up needs the current
        // persisted configuration and visible parent from this fixture's store.
        let metadata = try applicationPort.loadConversationMetadata(conversationID)
        let parentMessageID: String?
        if metadata != nil {
            parentMessageID = try applicationPort.loadConversationProjectionSnapshot(conversationID)
                .projection.messages.last?.messageID
        } else {
            parentMessageID = nil
        }
        guard try applicationPort.loadConversationMetadata(conversationID) == metadata else {
            throw MacApplicationPortError.rejected("Fixture admission frontier changed while preparing the request")
        }
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            expectedConfigurationRevision: metadata?.configurationRevision,
            expectedParentMessageID: parentMessageID,
            prompt: prompt,
            projectURL: workspace,
            attachmentURLs: attachmentURLs,
            modelID: modelID,
            endpoint: endpoint,
            apiKey: apiKey,
            contextWindowTokens: contextWindowTokens,
            maxModelIterations: maxModelIterations,
            reasoningConfiguration: testReasoningNone
                ? MacReasoningConfiguration(control: .effort, responses: true, efforts: ["none"]) : nil,
            reasoningPreference: testReasoningNone ? "none" : "automatic",
            modelExecutionCapabilities: testReasoningNone
                ? MacModelExecutionCapabilities(toolCalls: true, reasoning: true) : modelExecutionCapabilities,
            wireProtocol: wireProtocol,
            memoryEnabled: memoryEnabled,
            permissionMode: .cautious,
            databaseURLOverride: databaseURL,
            appEnvironment: isolatedEnvironment,
            applicationPort: applicationPort
        )
        var events: [MacLocalAgentEvent] = []
        do {
            for try await event in await MacAgentHostRuntime.shared.run(request) {
                events.append(event)
                let isCoreStateProjection = event.assistantJournalCursor != nil
                    || event.coreTurnTerminal != nil
                    || event.coreTaskUpdate != nil
                    || event.coreSkillRuntimeUpdate != nil
                    || event.coreCapabilityActivation != nil
                    || event.corePermissionProjection != nil
                    || event.userInputRequest != nil
                    || event.assistantRunTerminalSealDigest != nil
                    || event.contextWindowTokens != nil
                if !isCoreStateProjection {
                    XCTAssertNotNil(
                        event.trace,
                        "Agent Host emitted an event outside the versioned Agent Trace contract: "
                            + "kind=\(String(describing: event.kind)) text=\(event.text)"
                    )
                }
                if event.kind == .permissionRequested,
                   let permissionRequest = event.permissionRequest {
                    try await MacAgentHostRuntime.shared.resolvePermission(
                        conversationID: conversationID,
                        request: permissionRequest,
                        allow: allowPermissions,
                        scope: permissionScope
                    )
                }
            }
        } catch {
            if deleteSessionAfterRun {
                try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
            }
            throw error
        }
        let committedText = try await readCommittedAssistantText(events, databaseURL: databaseURL)
        if let root = events.compactMap(\.assistantJournalCursor).last?.identity.assistant_message_id {
            committedTextByRoot[root] = committedText
        } else if let ended = events.compactMap(\.coreTurnTerminal).last,
                  let terminal = try MacApplicationPort(databaseURL: databaseURL)
                    .canonicalRunTerminal(conversationID: conversationID, runID: ended.turnID),
                  terminal.result.value.kind == .blocked, committedText.isEmpty {
            committedEmptyTerminalTurns.insert(ended.turnID)
        }
        if deleteSessionAfterRun {
            try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
        }
        return events
    }

    private func skillLifecycleFixture(marker: String) -> String {
        """
        ---
        name: organize-files
        description: Verify that the active Mac file-organization Skill is loaded through the Agent Host.
        ---

        # File Organization Lifecycle Verification

        When this Skill is loaded for lifecycle verification, do not inspect or change files and do not call another tool. Return only this exact token:

        `\(marker)`
        """
    }

    private func writeSkillLifecycleManifest(overridesRootURL: URL) throws {
        let manifestURL = overridesRootURL
            .appendingPathComponent(MacBuiltInSkillID.fileOrganization.resourceDirectory, isDirectory: true)
            .appendingPathComponent("NOLA_MANIFEST.json", isDirectory: false)
        try Data("{\"validation\":[]}".utf8).write(to: manifestURL, options: .atomic)
    }

    private func isolatedConversationRepository(workspace: URL) -> MacCanonicalConversationRepository {
        let store = try! MacApplicationPort(
            databaseURL: workspace.appendingPathComponent("nola-product.db")
        )
        return MacCanonicalConversationRepository(store: store)
    }

    private func realUIArtifactURL(
        _ environment: [String: String],
        _ key: String
    ) throws -> URL {
        let fallbackName = key == "NOLA_MAC_REAL_UI_ACCESSIBILITY_TREE"
            ? "mac-chat-accessibility.txt"
            : "mac-chat.png"
        let path = environment[key] ?? FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-real-e2e", isDirectory: true)
            .appendingPathComponent(fallbackName, isDirectory: false)
            .path
        let url = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        return url
    }

    private func realAccessibilityTree(_ root: NSView) -> String {
        var lines: [String] = []
        var visited: Set<ObjectIdentifier> = []
        func walkAccessibility(_ seed: any NSAccessibilityProtocol, depth seedDepth: Int) {
            var queue: [(any NSAccessibilityProtocol, Int)] = [(seed, seedDepth)]
            var cursor = 0
            while cursor < queue.count, visited.count < 4_000 {
                let (element, depth) = queue[cursor]
                cursor += 1
                let identity = ObjectIdentifier(element as AnyObject)
                guard visited.insert(identity).inserted else { continue }
                let role = element.accessibilityRole()?.rawValue
                    ?? String(describing: type(of: element))
                let identifier = element.accessibilityIdentifier() ?? ""
                let label = element.accessibilityLabel() ?? ""
                let value = element.accessibilityValue().map(String.init(describing:)) ?? ""
                lines.append(
                    "\(String(repeating: "  ", count: depth))\(role) id=\(identifier) label=\(label) value=\(value)"
                )
                guard depth < 24 else { continue }
                for child in NSAccessibility.unignoredChildren(
                    from: element.accessibilityChildren() ?? []
                ) {
                    if let accessible = child as? any NSAccessibilityProtocol {
                        queue.append((accessible, depth + 1))
                    }
                }
            }
        }
        walkAccessibility(root, depth: 0)
        func walkViews(_ view: NSView, depth: Int) {
            walkAccessibility(view, depth: depth)
            let role = view.accessibilityRole()?.rawValue ?? String(describing: type(of: view))
            let identifier = view.accessibilityIdentifier()
            let label = view.accessibilityLabel() ?? ""
            let value = view.accessibilityValue().map(String.init(describing:)) ?? ""
            lines.append(
                "\(String(repeating: "  ", count: depth))view=\(role) id=\(identifier) label=\(label) value=\(value)"
            )
            for child in view.subviews {
                walkViews(child, depth: depth + 1)
            }
        }
        walkViews(root, depth: 0)
        return lines.joined(separator: "\n")
    }

    private func assistantText(_ events: [MacLocalAgentEvent]) -> String {
        if let ended = events.compactMap(\.coreTurnTerminal).last,
           committedEmptyTerminalTurns.contains(ended.turnID) { return "" }
        guard let root = events.compactMap(\.assistantJournalCursor).last?.identity.assistant_message_id,
              let text = committedTextByRoot[root] else {
            XCTFail("Missing real source journal capture for this Run")
            return ""
        }
        return text
    }

    private func readCanonicalToolResults(
        databaseURL: URL, conversationID: String, callIDs: [String]
    ) throws -> [MacAgentTraceJSONValue] {
        guard !callIDs.isEmpty else { return [] }
        func literal(_ value: String) -> String {
            "'" + value.replacingOccurrences(of: "'", with: "''") + "'"
        }
        let sql = """
        SELECT json_extract(item.value, '$.output')
        FROM core_checkpoint_bodies AS body,
             json_each(body.checkpoint_json, '$.state.conversation.transcript') AS item
        WHERE body.conversation_id = \(literal(conversationID))
          AND json_extract(item.value, '$.type') = 'tool_result'
          AND json_extract(item.value, '$.name') = 'conversation_recall'
          AND json_extract(item.value, '$.is_error') = 0
          AND json_extract(item.value, '$.call_id') IN (\(callIDs.map(literal).joined(separator: ",")));
        """
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        process.arguments = ["-readonly", databaseURL.path, sql]
        process.standardOutput = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0, "Canonical tool result query failed")
        return try String(decoding: data, as: UTF8.self).split(separator: "\n").map {
            try JSONDecoder().decode(MacAgentTraceJSONValue.self, from: Data($0.utf8))
        }
    }

    private func containsCanonicalRecallEvidence(
        _ value: MacAgentTraceJSONValue,
        fact: String,
        depth: Int = 0
    ) -> Bool {
        guard depth < 10 else { return false }
        switch value {
        case .object(let fields):
            if fields["source"] == .string("originalMessage"),
               case .string(let messageID) = fields["message_id"], !messageID.isEmpty,
               case .string(let partID) = fields["part_id"], !partID.isEmpty,
               case .string(let snippet) = fields["snippet"], snippet.contains(fact) {
                return true
            }
            return fields.values.contains { containsCanonicalRecallEvidence($0, fact: fact, depth: depth + 1) }
        case .array(let values):
            return values.contains { containsCanonicalRecallEvidence($0, fact: fact, depth: depth + 1) }
        case .string(let text):
            guard let nested = try? JSONDecoder().decode(MacAgentTraceJSONValue.self, from: Data(text.utf8)) else { return false }
            return containsCanonicalRecallEvidence(nested, fact: fact, depth: depth + 1)
        default:
            return false
        }
    }

    private func allEventText(_ events: [MacLocalAgentEvent]) -> String {
        events
            .map { "\($0.title ?? "")\n\($0.text)" }
            .joined(separator: "\n")
    }

    private func makeWorkspace(_ name: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-agent-e2e-\(name)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func seedMixedOwnershipDataSources(
        in store: MacApplicationPort,
        includeMalformedOwnedSource: Bool
    ) throws {
        let now = Int64((Date().timeIntervalSince1970 * 1_000).rounded())
        var sources = [
            MacCanonicalDataSourceDraft(
                id: "platform:health",
                name: "Health",
                kind: .platform,
                configuration: MacCanonicalDataSourceConfiguration(
                    connectorID: "mac.platform.health",
                    locationKind: .platformResource,
                    location: "platform:health",
                    parameters: [:],
                    secretReferences: []
                ),
                accessPolicy: MacDataSourceAccessPolicy.conversationOptIn.rawValue,
                authorization: MacCanonicalDataSourceAuthorization(
                    state: .notDetermined,
                    capability: "health",
                    observedAtMs: now
                ),
                freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: 60,
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "device_private",
                createdAtMs: now,
                updatedAtMs: now
            ),
            MacCanonicalDataSourceDraft(
                id: "web:native-fixture",
                name: "Native Web Fixture",
                kind: .web,
                configuration: MacCanonicalDataSourceConfiguration(
                    connectorID: "mac.web.http",
                    locationKind: .httpURL,
                    location: "https://example.test/native-fixture",
                    parameters: [:],
                    secretReferences: []
                ),
                accessPolicy: MacDataSourceAccessPolicy.conversationOptIn.rawValue,
                authorization: MacCanonicalDataSourceAuthorization(
                    state: .notRequired,
                    capability: nil,
                    observedAtMs: now
                ),
                freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: 60,
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "cloud_reference",
                createdAtMs: now,
                updatedAtMs: now
            ),
        ]
        if includeMalformedOwnedSource {
            sources.append(MacCanonicalDataSourceDraft(
                id: "local:malformed",
                name: "Malformed local source",
                kind: .localFile,
                configuration: MacCanonicalDataSourceConfiguration(
                    connectorID: "mac.local-file",
                    locationKind: .localPath,
                    location: "/tmp/nola-malformed-source",
                    parameters: [:],
                    secretReferences: []
                ),
                accessPolicy: MacDataSourceAccessPolicy.defaultEnabled.rawValue,
                authorization: MacCanonicalDataSourceAuthorization(
                    state: .authorized,
                    capability: "filesystem.read",
                    observedAtMs: now
                ),
                freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: 60,
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "device_private",
                createdAtMs: now,
                updatedAtMs: now
            ))
        }
        for source in sources {
            _ = try store.createDataSourceDraft(source)
        }
    }

    private func assertCompleteReport(
        _ server: MacAgentCapabilityMockServer,
        expectedStageCount: Int
    ) throws {
        let report = try server.report()
        XCTAssertEqual(report.stages, Array(0..<expectedStageCount))
        XCTAssertTrue(
            report.failures.isEmpty,
            "Agent Host E2E contract failures: \(report.failures.joined(separator: "; ")); "
                + "tools by stage: \(report.toolNamesByStage)"
        )
    }
}

private struct MacAgentCapabilityMockReport {
    var stages: [Int] = []
    var failures: [String] = []
    var toolNamesByStage: [Int: [String]] = [:]
    var compactionCount = 0
}

private final class MacAgentCapabilityMockServer: @unchecked Sendable {
    private let process: Process
    private let outputPipe = Pipe()
    private let errorPipe = Pipe()
    private let reportURL: URL
    private let configurationURL: URL
    private(set) var endpoint = ""

    init(
        workspace: URL,
        steps: [[String: Any]],
        contextWindow: Int = 128_000,
        compactionSummary: String = "# Continuation\n\nPreserve the durable conversation state."
    ) throws {
        let root = workspace.appendingPathComponent(".agent-capability-e2e", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        configurationURL = root.appendingPathComponent("configuration.json")
        reportURL = root.appendingPathComponent("report.jsonl")
        let data = try JSONSerialization.data(
            withJSONObject: [
                "steps": steps,
                "report_path": reportURL.path,
                "context_window": contextWindow,
                "compaction_summary": compactionSummary,
            ],
            options: [.sortedKeys]
        )
        try data.write(to: configurationURL, options: .atomic)

        process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-u", "-c", Self.pythonServer, configurationURL.path]
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        process.standardInput = FileHandle.nullDevice
        try process.run()

        let line: String
        do {
            line = try Self.readStartupLine(
                output: outputPipe.fileHandleForReading,
                errors: errorPipe.fileHandleForReading,
                process: process
            )
        } catch {
            stop()
            throw error
        }
        guard line.hasPrefix("PORT "), let port = UInt16(line.dropFirst(5)) else {
            stop()
            throw NSError(
                domain: "NolaMacTests.MacAgentCapabilityMockServer",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Mock LLM did not publish a port: \(line)"]
            )
        }
        endpoint = "http://127.0.0.1:\(port)/v1"
    }

    deinit {
        stop()
    }

    func stop() {
        let processID = process.processIdentifier
        guard processID > 0 else { return }
        if process.isRunning {
            process.terminate()
        }

        // `Process.waitUntilExit()` can remain parked in Foundation after the
        // Python fixture is already a zombie when the complete macOS bundle is
        // running. Reap the child directly and keep fixture teardown bounded;
        // otherwise a successful product test appears to hang forever.
        var status: Int32 = 0
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline {
            let result = Darwin.waitpid(processID, &status, WNOHANG)
            if result == processID || (result < 0 && errno == ECHILD) {
                return
            }
            if result < 0, errno != EINTR {
                return
            }
            usleep(10_000)
        }

        _ = Darwin.kill(processID, SIGKILL)
        while Darwin.waitpid(processID, &status, 0) < 0, errno == EINTR {}
    }

    func attachTimingReport(to testCase: XCTestCase) {
        guard let data = try? Data(contentsOf: reportURL) else { return }
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.plain-text")
        attachment.name = "Mock provider request timeline"
        attachment.lifetime = .keepAlways
        testCase.add(attachment)
    }

    func report() throws -> MacAgentCapabilityMockReport {
        guard FileManager.default.fileExists(atPath: reportURL.path) else {
            return MacAgentCapabilityMockReport(failures: ["mock provider produced no report"])
        }
        let content = try String(contentsOf: reportURL, encoding: .utf8)
        var report = MacAgentCapabilityMockReport()
        for line in content.split(separator: "\n") {
            guard let data = line.data(using: .utf8),
                  let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }
            if object["kind"] as? String == "compaction" {
                report.compactionCount += 1
                continue
            }
            guard let stage = object["stage"] as? Int else { continue }
            report.stages.append(stage)
            report.toolNamesByStage[stage] = object["tool_names"] as? [String] ?? []
            let checks = object["checks"] as? [String: Bool] ?? [:]
            for (name, passed) in checks where !passed {
                report.failures.append("stage \(stage): \(name)")
            }
        }
        return report
    }

    fileprivate static func readStartupLine(
        output: FileHandle,
        errors: FileHandle,
        process: Process,
        timeout: TimeInterval = 8
    ) throws -> String {
        var data = Data()
        let deadline = ContinuousClock.now.advanced(by: .seconds(timeout))
        while ContinuousClock.now < deadline {
            var descriptor = pollfd(fd: output.fileDescriptor, events: Int16(POLLIN), revents: 0)
            let readiness = Darwin.poll(&descriptor, 1, 20)
            if readiness < 0 {
                if errno == EINTR { continue }
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
            guard readiness > 0 else { continue }
            let chunk = try readReadyChunk(output)
            if chunk.isEmpty {
                var errorDescriptor = pollfd(fd: errors.fileDescriptor, events: Int16(POLLIN), revents: 0)
                let errorData = Darwin.poll(&errorDescriptor, 1, 0) > 0
                    ? try readReadyChunk(errors) : Data()
                let errorText = String(decoding: errorData, as: UTF8.self)
                throw NSError(
                    domain: "NolaMacTests.MacAgentCapabilityMockServer",
                    code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Mock LLM exited before startup: \(errorText)"]
                )
            }
            data.append(chunk)
            if let newline = data.firstIndex(of: 0x0A) {
                return String(decoding: data[..<newline], as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            }
            if data.count >= 65_536 { break }
            if !process.isRunning { break }
        }
        throw NSError(
            domain: "NolaMacTests.MacAgentCapabilityMockServer",
            code: 3,
            userInfo: [NSLocalizedDescriptionKey: "Timed out waiting for the mock LLM port"]
        )
    }

    private static func readReadyChunk(_ handle: FileHandle) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: 4096)
        let count = bytes.withUnsafeMutableBytes { buffer in
            Darwin.read(handle.fileDescriptor, buffer.baseAddress, buffer.count)
        }
        guard count >= 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        return Data(bytes.prefix(count))
    }

    private static let pythonServer = #"""
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    CONFIG = json.load(handle)

LOCK = threading.Lock()
STAGE = 0
COMPACTION_COUNT = 0
LAST_PROVISIONAL_FINAL = None

def append_timing(phase, path):
    # Record transport boundaries only, never request bodies or credentials.
    with open(CONFIG["report_path"], "a", encoding="utf-8") as handle:
        handle.write(json.dumps({
            "kind": "transport_timing", "phase": phase, "path": path,
            "wall_time_ns": time.time_ns(), "monotonic_ns": time.monotonic_ns()
        }, sort_keys=True) + "\n")

def append_report(stage, checks, tool_names=None):
    with open(CONFIG["report_path"], "a", encoding="utf-8") as handle:
        handle.write(json.dumps(
            {"stage": stage, "checks": checks, "tool_names": tool_names or []},
            ensure_ascii=False,
            sort_keys=True
        ) + "\n")
        handle.flush()

def append_compaction_report(index):
    with open(CONFIG["report_path"], "a", encoding="utf-8") as handle:
        handle.write(json.dumps(
            {"kind": "compaction", "index": index},
            ensure_ascii=False,
            sort_keys=True
        ) + "\n")
        handle.flush()

def tool_response(name, arguments, call_id):
    return {
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": json.dumps(
                            arguments,
                            ensure_ascii=False,
                            separators=(",", ":")
                        )
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }

def tool_batch_response(calls, call_id_prefix):
    return {
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": index,
                    "id": call_id_prefix + "_" + str(index),
                    "type": "function",
                    "function": {
                        "name": call["name"],
                        "arguments": json.dumps(
                            call.get("arguments", {}),
                            ensure_ascii=False,
                            separators=(",", ":")
                        )
                    }
                } for index, call in enumerate(calls)]
            },
            "finish_reason": "tool_calls"
        }]
    }

def final_response(content, finish_reason="stop"):
    return {
        "choices": [{
            "index": 0,
            "delta": {"content": content},
            "finish_reason": finish_reason
        }]
    }

def is_synthesis_tool_surface(tool_names):
    internal_evidence_tools = {
        "tool_output_find",
        "tool_output_list",
        "tool_output_read",
        "tool_output_search",
    }
    return all(name in internal_evidence_tools for name in tool_names)

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        return

    def do_GET(self):
        append_timing("request_received", self.path)
        self.send_json({
            "object": "list",
            "data": [{
                "id": "nola-capability-e2e",
                "object": "model",
                "context_window": CONFIG.get("context_window", 128000),
                "max_model_len": CONFIG.get("context_window", 128000)
            }]
        })

    def do_POST(self):
        global STAGE, COMPACTION_COUNT, LAST_PROVISIONAL_FINAL
        append_timing("request_received", self.path)
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        append_timing("body_read", self.path)
        body = json.loads(raw.decode("utf-8"))
        request_messages = json.dumps(
            body.get("messages", []),
            ensure_ascii=False,
            sort_keys=True
        )
        request_tool_names = []
        memory_worker_kind = None
        for item in body.get("tools", []):
            function = item.get("function", {})
            name = function.get("name") or item.get("name")
            if name:
                request_tool_names.append(name)
            if name == "submit_memory_worker_output":
                memory_worker_kind = (
                    function.get("parameters", {})
                    .get("properties", {})
                    .get("type", {})
                    .get("const", "extraction")
                )
        if memory_worker_kind is not None:
            self.send_payload(tool_response(
                "submit_memory_worker_output",
                {
                    "type": memory_worker_kind,
                    "summary": "No additional durable memory candidates.",
                    "candidates": []
                },
                "call_memory_worker"
            ), body)
            return
        if "NOLA_SCOPE_QUALIFICATION_V1" in request_messages:
            self.send_payload(final_response('{"scope":"specific"}'), body)
            return
        if "Create a loss-minimizing continuation checkpoint" in request_messages:
            with LOCK:
                compaction_index = COMPACTION_COUNT
                COMPACTION_COUNT += 1
            append_compaction_report(compaction_index)
            self.send_payload(final_response(CONFIG.get(
                "compaction_summary",
                "# Continuation\n\nPreserve the durable conversation state."
            )), body)
            return
        # Business steps assert tools and evidence. Model the private audit
        # protocol separately without disabling production completion policy.
        system_text = "\n\n".join(
            item.get("content", "") for item in body.get("messages", [])
            if item.get("role") == "system" and isinstance(item.get("content"), str)
        )
        candidate_start = "<untrusted_answer_candidate>\n"
        candidate_end = "\n</untrusted_answer_candidate>"
        if candidate_start in system_text and candidate_end in system_text:
            candidate = system_text.split(candidate_start, 1)[1].split(candidate_end, 1)[0]
            if "This is an independent constraint-audit boundary." in system_text:
                self.send_payload(final_response(json.dumps({
                    "decision": "accept", "constraints": ["scripted capability scenario"],
                    "items": [{"label": candidate, "eligibility": "eligible",
                               "evidence": "Business steps assert the fixture's tool evidence"}],
                    "violations": []
                })), body)
                return
            if "This is the final presentation boundary." in system_text:
                with LOCK:
                    LAST_PROVISIONAL_FINAL = None
                self.send_payload(final_response(candidate), body)
                return
        steps = CONFIG.get("steps", [])
        synthesis_content = None
        # Scope provisional presentation to the exact user-input lineage of
        # this provider conversation. A new user occurrence changes the list
        # even when its text is identical; no last-answer cache spans Runs.
        user_context = json.dumps([
            message for message in body.get("messages", [])
            if isinstance(message, dict) and message.get("role") == "user"
        ], ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        with LOCK:
            if LAST_PROVISIONAL_FINAL is not None and LAST_PROVISIONAL_FINAL[0] != user_context:
                LAST_PROVISIONAL_FINAL = None
            if is_synthesis_tool_surface(request_tool_names) and LAST_PROVISIONAL_FINAL is not None:
                synthesis_content = LAST_PROVISIONAL_FINAL[1]
                LAST_PROVISIONAL_FINAL = None
                stage = STAGE
            else:
                stage = STAGE
                STAGE += 1

        if synthesis_content is not None:
            self.send_payload(final_response(synthesis_content), body)
            return

        if stage >= len(steps):
            append_report(stage, {"unexpected_extra_request": False})
            self.send_payload(final_response("UNEXPECTED_EXTRA_REQUEST"), body)
            return

        step = steps[stage]
        messages_text = json.dumps(
            body.get("messages", []),
            ensure_ascii=False,
            sort_keys=True
        )
        tool_names = []
        for item in body.get("tools", []):
            function = item.get("function", {})
            name = function.get("name") or item.get("name")
            if name:
                tool_names.append(name)

        missing_tools = [
            name for name in step.get("required_tools", [])
            if name not in tool_names
        ]
        if (
            missing_tools
            and "tool_search" in tool_names
            and step.get("tool_name") != "tool_search"
        ):
            # Deferred tools are intentionally absent from the initial model
            # surface. Exercise the same discovery round trip as a real model
            # without consuming the semantic fixture stage.
            with LOCK:
                if STAGE == stage + 1:
                    STAGE = stage
            self.send_payload(tool_response(
                "tool_search",
                {"query": "select:" + missing_tools[0]},
                "call_discover_" + str(stage) + "_" + missing_tools[0]
            ), body)
            return

        checks = {}
        for value in step.get("message_contains", []):
            checks["message:" + value] = value in messages_text
        for name in step.get("required_tools", []):
            checks["required_tool:" + name] = name in tool_names
        for name in step.get("forbidden_tools", []):
            checks["forbidden_tool:" + name] = name not in tool_names
        if step.get("requires_image"):
            checks["native_image"] = any(
                isinstance(content, list)
                and any(
                    isinstance(part, dict)
                    and (
                        part.get("type") in ("image_url", "input_image", "image")
                        or "image_url" in part
                    )
                    for part in content
                )
                for message in body.get("messages", [])
                if isinstance(message, dict)
                for content in [message.get("content")]
            )
        append_report(stage, checks, tool_names)

        if step.get("tool_calls"):
            payload = tool_batch_response(
                step["tool_calls"],
                "call_e2e_batch_" + str(stage)
            )
        elif step.get("tool_name"):
            payload = tool_response(
                step["tool_name"],
                step.get("tool_args", {}),
                "call_e2e_" + str(stage)
            )
        else:
            payload = final_response(
                step.get("content", "E2E_COMPLETE"),
                step.get("finish_reason", "stop")
            )
            if not is_synthesis_tool_surface(request_tool_names) and any(
                isinstance(message, dict)
                and (
                    message.get("role") == "tool"
                    or message.get("type") == "function_call_output"
                )
                for message in body.get("messages", [])
            ):
                with LOCK:
                    LAST_PROVISIONAL_FINAL = (user_context, step.get("content", "E2E_COMPLETE"))
        self.send_payload(payload, body)

    def send_payload(self, payload, request_body):
        if request_body.get("stream", True):
            encoded = (
                "data: " +
                json.dumps(payload, ensure_ascii=False, separators=(",", ":")) +
                "\n\ndata: [DONE]\n\n"
            ).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
            self.wfile.flush()
            append_timing("response_flushed", self.path)
            self.close_connection = True
        else:
            choice = payload["choices"][0]
            delta = choice.get("delta", {})
            self.send_json({
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", **delta},
                    "finish_reason": choice.get("finish_reason", "stop")
                }]
            })

    def send_json(self, payload, status=200):
        encoded = json.dumps(
            payload,
            ensure_ascii=False,
            separators=(",", ":")
        ).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Connection", "close")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
        self.wfile.flush()
        append_timing("response_flushed", self.path)
        self.close_connection = True

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print("PORT " + str(server.server_address[1]), flush=True)
server.serve_forever()
"""#
}
