// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the macOS web-source lifecycle through a real local HTTP connection.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacWebSourceE2ETests: XCTestCase {
    func testCancelledValidationDoesNotCreateSourceOrFailureContract() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-cancelled-source-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let store = MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry)
        let source = makeSource(name: "Cancelled draft", url: "https://invalid.example/source.json")
        let task = Task { try await store.validateAndUpsert(source) }
        task.cancel()
        do {
            try await task.value
            XCTFail("Cancelled validation must not succeed")
        } catch is CancellationError {
            // Cancellation is not a connection failure and must leave no data.
        }
        XCTAssertTrue(store.sources.isEmpty)
        XCTAssertNil(store.errorMessage)
        XCTAssertNil(registry.contract(for: source.dataSourceID))
    }

    func testRealConfiguredModelAnswersFromValidatedWebSourceThroughMacAgentHost() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac data-source request.")
        }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-real-web-source-agent-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let marker = "MAC-SOURCE-\(UUID().uuidString)"
        let payload = "{\"headline\":\"\(marker)\",\"priority\":\"critical\"}"
        try Data(payload.utf8).write(to: root.appendingPathComponent("source.json"))
        let (server, port) = try startHTTPServer(directory: root)
        defer {
            server.terminate()
            server.waitUntilExit()
        }

        let registry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("source-registry.json")
        )
        let store = MacWebSourceStore(
            storeURL: root.appendingPathComponent("web-sources.json"),
            registry: registry
        )
        let source = makeSource(
            name: "Validated local JSON",
            url: "http://127.0.0.1:\(port)/source.json"
        )
        try await store.validateAndUpsert(source)
        let conversationID = "real-web-source-\(UUID().uuidString)"
        let sourceContext = await store.promptWithSources(
            "",
            conversationID: conversationID
        )
        XCTAssertTrue(sourceContext.contains(marker))
        XCTAssertTrue(try XCTUnwrap(registry.contract(for: source.dataSourceID)).isUsable)

        let prompt = "Return only the exact headline value from the verified Web Source Context as plain text, without Markdown, quotes, or backticks."
        let contextComponents = [
            MacCoreContextComponent(
                kind: "web_retrieval",
                id: source.dataSourceID,
                content: sourceContext,
                source: .init(
                    uri: source.url,
                    title: source.name,
                    mimeType: "application/json"
                )
            ),
        ]
        let contextMessages = MacContextCore.hostContextMessages(
            userPrompt: prompt,
            components: contextComponents,
            contextWindowTokens: 64_000
        )
        XCTAssertFalse(contextMessages.isEmpty)
        XCTAssertTrue(contextMessages.contains { $0.content.contains(marker) })

        let appEnvironment = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: root.appendingPathComponent("Caches", isDirectory: true)
        )
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: prompt,
            contextComponents: contextComponents,
            projectURL: root,
            attachmentURLs: [],
            modelID: model,
            endpoint: endpoint,
            apiKey: apiKey,
            contextWindowTokens: 64_000,
            memoryEnabled: false,
            databaseURLOverride: URL(
                fileURLWithPath: environment["NOLA_DB_PATH"]
                    ?? root.appendingPathComponent("state/nola.db").path
            ),
            appEnvironment: appEnvironment,
            applicationPort: try MacApplicationPort(
                databaseURL: URL(
                    fileURLWithPath: environment["NOLA_DB_PATH"]
                        ?? root.appendingPathComponent("state/nola.db").path
                )
            ),
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
                        "Agent Host emitted an event outside the versioned Agent Trace contract"
                    )
                }
            }
        } catch {
            try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
            throw error
        }
        let answer = try await readCommittedAssistantText(events, databaseURL: try XCTUnwrap(request.applicationPort).databaseURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if environment["NOLA_DB_PATH"] == nil {
            try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
        }

        XCTAssertEqual(
            answer.trimmingCharacters(in: CharacterSet(charactersIn: "`\"'")),
            marker
        )
        XCTAssertEqual(answer.components(separatedBy: marker).count - 1, 1)
        XCTAssertFalse(events.contains { $0.kind == .permissionRequested })
        XCTAssertFalse(events.contains { $0.kind == .toolStarted })
        XCTAssertEqual(events.filter { $0.kind == .finished }.count, 1)
        XCTAssertEqual(registry.usageRecords.last?.conversationID, conversationID)
        XCTAssertTrue(registry.usageRecords.last?.disclosedToCloud == true)
    }

    func testRealReadGatesPersistenceAndConversationInjection() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-web-source-e2e-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let marker = "NOLA_SOURCE_MARKER_\(UUID().uuidString)"
        let payload = "{\"headline\":\"\(marker)\",\"priority\":\"high\"}"
        try Data(payload.utf8).write(to: root.appendingPathComponent("source.json"))
        let (server, port) = try startHTTPServer(directory: root)
        defer {
            server.terminate()
            server.waitUntilExit()
        }

        let registry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("source-registry.json")
        )
        let store = MacWebSourceStore(
            storeURL: root.appendingPathComponent("web-sources.json"),
            registry: registry
        )
        let source = makeSource(
            name: "Local JSON",
            url: "http://127.0.0.1:\(port)/source.json"
        )

        try await store.validateAndUpsert(source)

        XCTAssertEqual(store.sources.count, 1)
        XCTAssertTrue(store.sources[0].cachedText.contains(marker))
        let contract = try XCTUnwrap(registry.contract(for: source.dataSourceID))
        XCTAssertTrue(contract.isUsable)
        XCTAssertEqual(contract.connectorID, "mac.web.http")
        XCTAssertEqual(contract.recordCount, 2)
        XCTAssertTrue(contract.schemaFields.contains("JSON"))
        XCTAssertFalse(contract.redactedSample.contains(marker))

        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        for id in ["conversation-default", "conversation-opt-in"] {
            XCTAssertTrue(try canonical.createConversation(.init(id: id, title: "Source consumer",
                settingsJSON: nil, createdAtMs: now, updatedAtMs: now)).created)
        }
        let defaultPrompt = await store.promptWithSources(
            "Summarize the source.",
            conversationID: "conversation-default"
        )
        XCTAssertTrue(defaultPrompt.contains(marker))
        XCTAssertEqual(registry.usageRecords.first?.sourceID, source.dataSourceID)
        XCTAssertEqual(registry.usageRecords.first?.conversationID, "conversation-default")

        registry.setPolicy(.conversationOptIn, for: source.dataSourceID)
        let excludedPrompt = await store.promptWithSources(
            "Summarize the source.",
            conversationID: "conversation-opt-in"
        )
        XCTAssertFalse(excludedPrompt.contains(marker))

        registry.setConversationOverride(
            true,
            sourceID: source.dataSourceID,
            conversationID: "conversation-opt-in"
        )
        XCTAssertNil(registry.persistenceError)
        let includedPrompt = await store.promptWithSources(
            "Summarize the source.",
            conversationID: "conversation-opt-in"
        )
        XCTAssertTrue(includedPrompt.contains(marker))

        let missing = makeSource(
            name: "Missing",
            url: "http://127.0.0.1:\(port)/missing.json"
        )
        do {
            try await store.validateAndUpsert(missing)
            XCTFail("A failed first read must not persist the source")
        } catch {
            XCTAssertEqual(store.sources.count, 1)
            XCTAssertEqual(
                registry.contract(for: missing.dataSourceID)?.validationState,
                .failed
            )
        }
    }

    func testAutonomousOrchestrationDoesNotCreateTaskWhenFirstReadFails() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-source-orchestration-failure-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let (server, port) = try startHTTPServer(directory: root)
        defer {
            server.terminate()
            server.waitUntilExit()
        }

        let registry = MacDataSourceRegistry(
            storeURL: root.appendingPathComponent("source-registry.json")
        )
        let webStore = MacWebSourceStore(
            storeURL: root.appendingPathComponent("web-sources.json"),
            registry: registry
        )
        let environment = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: root.appendingPathComponent("Caches", isDirectory: true)
        )
        let automationStore = try MacAutomationStore(
            baseDirectory: root.appendingPathComponent("Automations", isDirectory: true),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents", isDirectory: true),
            synchronizeSystemJobs: false,
            appEnvironment: environment,
            authorizationResolver: { _ in true },
            completionNotifier: { _, _ in },
            dataSourceRegistry: registry,
            webSourceStore: webStore
        )
        let sourceID = UUID().uuidString.lowercased()
        let request = MacAgentOrchestrationRequest(
            name: "Invalid source task",
            prompt: "Analyze the selected source.",
            skillID: MacBuiltInSkillID.dataAnalysis.rawValue,
            frequency: MacAutomation.Frequency.daily.rawValue,
            weekday: 1,
            hour: 9,
            minute: 0,
            enabled: false,
            runWhenDataChanges: false,
            contextSources: ["web_sources"],
            webSources: [.init(
                id: sourceID,
                name: "Missing source",
                url: "http://127.0.0.1:\(port)/missing.json",
                format: "json",
                refreshFrequency: "daily"
            )],
            maximumDataAgeMinutes: 60,
            timeoutMinutes: 2,
            retryCount: 0,
            executionSpec: .agent(.init(prompt: "Analyze the selected source.", executionScope: .localWorkspace(root: root.path),
                permissionProfile: "cautious", modelSelection: "use_current_default_at_admission",
                contextRefs: [.init(kind: "web_source", id: sourceID)], skillRefs: [MacBuiltInSkillID.dataAnalysis.rawValue]))
        )

        do {
            _ = try await MacAgentOrchestrationService.apply(
                request,
                workspaceURL: root,
                automationStore: automationStore,
                workspaceIsAuthorized: { _ in true },
                webSourceStore: webStore
            )
            XCTFail("An unreadable source must abort autonomous orchestration")
        } catch {
            XCTAssertTrue(automationStore.automations.isEmpty)
            XCTAssertTrue(webStore.sources.isEmpty)
            XCTAssertEqual(registry.contracts.count, 1)
            XCTAssertEqual(registry.contracts.values.first?.validationState, .failed)
        }
    }

    private func makeSource(name: String, url: String) -> MacWebSource {
        MacWebSource(
            id: UUID(),
            name: name,
            url: url,
            format: .json,
            isEnabled: true,
            cachedText: "",
            lastFetchedAt: nil,
            lastError: nil,
            createdAt: .now
        )
    }

    private func startHTTPServer(directory: URL) throws -> (Process, Int) {
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [
            "-c",
            """
            import http.server, os, socketserver
            os.chdir(os.environ["NOLA_TEST_ROOT"])
            server = socketserver.TCPServer(("127.0.0.1", 0), http.server.SimpleHTTPRequestHandler)
            print(server.server_address[1], flush=True)
            server.serve_forever()
            """,
        ]
        process.environment = ProcessInfo.processInfo.environment.merging([
            "NOLA_TEST_ROOT": directory.path,
        ]) { _, new in new }
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        try process.run()

        var bytes = Data()
        while !bytes.contains(0x0A) {
            let chunk = output.fileHandleForReading.availableData
            guard !chunk.isEmpty else {
                process.terminate()
                throw NSError(
                    domain: "MacWebSourceE2ETests",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Local HTTP server exited before reporting its port."]
                )
            }
            bytes.append(chunk)
        }
        let line = String(decoding: bytes, as: UTF8.self)
            .split(separator: "\n", maxSplits: 1)
            .first
        guard let line, let port = Int(line) else {
            process.terminate()
            throw NSError(
                domain: "MacWebSourceE2ETests",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "Local HTTP server returned an invalid port."]
            )
        }
        return (process, port)
    }
}
